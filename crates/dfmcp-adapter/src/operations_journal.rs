#![forbid(unsafe_code)]

//! Synced, hash-chained operations observations and exact projection replay.
//! This is an observation archive, not the effect ledger, a game checkpoint,
//! authenticated provenance, a monotonic rollback floor, or continuous history.

use std::io::{self, Read, Seek, SeekFrom, Write};
use std::time::Instant;

use dfmcp_core::{Capability, DfmcpError, Digest32, ErrorCode, FortressId, GameTick,
    ObservationCursor, OperationContext, Result, RiskTier, StateAnchor};
use dfmcp_world::WorldSnapshot;
use crate::live_jobs::JobPublication;
use crate::live_operations::{LiveOperationsObservation, LiveOperationsState, MAX_OPERATIONS_BYTES};

#[path = "operations_journal_file.rs"]
mod file_storage;
pub use file_storage::{PrivateJournalFile, open_private_journal};

const MAGIC: &[u8; 8] = b"DFMOJ001";
const FOOTER: &[u8; 8] = b"DFMOEND1";
const RECORD: &[u8; 8] = b"DFMOREC1";
const FRAME_HEADER_BYTES: usize = 44;
const HEADER_BYTES: usize = 80;
const MAX_BODY_BYTES: usize = MAX_OPERATIONS_BYTES + 1024;
pub const MAX_JOURNAL_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_JOURNAL_RECORDS: usize = 4096;

fn corrupt(text: &str) -> DfmcpError { DfmcpError::new(ErrorCode::CorruptLedger, text) }
fn budget(text: &str) -> DfmcpError { DfmcpError::new(ErrorCode::BudgetExceeded, text) }
fn storage_error(_: io::Error) -> DfmcpError { corrupt("operations journal I/O failed; reopen for verified recovery") }

/// Injected storage must preserve seek semantics and make `sync` a real durability
/// boundary. File storage holds an exclusive nonblocking lock for its lifetime.
pub trait JournalStorage: Read + Write + Seek {
    fn sync(&mut self) -> io::Result<()>;
    fn truncate(&mut self, length: u64) -> io::Result<()>;
    fn validate_identity(&self) -> io::Result<()> { Ok(()) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JournalLimits { pub max_bytes: u64, pub max_records: usize }
impl Default for JournalLimits {
    fn default() -> Self { Self { max_bytes: 64 * 1024 * 1024, max_records: 1024 } }
}
impl JournalLimits {
    fn validate(self) -> Result<()> {
        if !(HEADER_BYTES as u64..=MAX_JOURNAL_BYTES).contains(&self.max_bytes)
            || !(1..=MAX_JOURNAL_RECORDS).contains(&self.max_records) {
            return Err(budget("operations journal limits exceed the implementation bounds"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TailRecovery { Refuse, TruncateIncomplete }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalEntry {
    pub number: u64,
    pub anchor: StateAnchor,
    pub source_digest: Digest32,
    pub record_digest: Digest32,
    pub previous_digest: Digest32,
    pub offset: u64,
    pub encoded_bytes: u32,
}

/// In-memory state advances only after the complete next frame has been synced.
/// A failed write/sync fences this object, even when all frame bytes were written.
pub struct OperationsJournal<S> {
    storage: S,
    limits: JournalLimits,
    fortress: FortressId,
    id: Digest32,
    header_digest: Digest32,
    head: Digest32,
    length: u64,
    entries: Vec<JournalEntry>,
    state: LiveOperationsState,
    fenced: bool,
    repaired_tail_bytes: u64,
}

fn check(context: &OperationContext, started: Instant) -> Result<()> {
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if started.elapsed().as_millis() > u128::from(context.budget.max_wall_millis) {
        return Err(budget("operations journal replay exceeded its wall-time budget"));
    }
    Ok(())
}

impl<S: JournalStorage> OperationsJournal<S> {
    /// `initialize_empty` is supplied only after exclusive new-file creation.
    /// Existing empty/partial headers and complete corrupt frames are never repaired.
    pub fn open(mut storage: S, context: &OperationContext, limits: JournalLimits,
        initialize_empty: bool, recovery: TailRecovery) -> Result<Self> {
        let started = Instant::now();
        check(context, started)?;
        limits.validate()?;
        storage.validate_identity().map_err(storage_error)?;
        let mut length = storage.seek(SeekFrom::End(0)).map_err(storage_error)?;
        if length > limits.max_bytes { return Err(budget("operations journal exceeds its retained byte limit")); }
        if length == 0 && initialize_empty {
            context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
            let mut identity = b"dfmcp-operations-journal-incarnation/1\0".to_vec();
            identity.extend_from_slice(&context.session_id.get().to_be_bytes());
            identity.extend_from_slice(&context.request_id.get().to_be_bytes());
            identity.extend_from_slice(context.anchor.state_hash.as_bytes());
            let id = Digest32::of_bytes(&identity);
            let mut header = MAGIC.to_vec();
            put_u64(&mut header, context.anchor.fortress_id.get());
            header.extend_from_slice(id.as_bytes());
            let digest = header_hash(&header);
            header.extend_from_slice(digest.as_bytes());
            storage.seek(SeekFrom::Start(0)).map_err(storage_error)?;
            storage.write_all(&header).map_err(storage_error)?;
            storage.sync().map_err(storage_error)?;
            length = HEADER_BYTES as u64;
        }
        if length < HEADER_BYTES as u64 { return Err(corrupt("operations journal header is incomplete; original file is unchanged")); }
        storage.seek(SeekFrom::Start(0)).map_err(storage_error)?;
        let mut header = [0; HEADER_BYTES];
        storage.read_exact(&mut header).map_err(storage_error)?;
        let (fortress, id, header_digest) = decode_header(&header)?;
        if fortress != context.anchor.fortress_id {
            return Err(DfmcpError::new(ErrorCode::StaleAnchor,"journal belongs to another fortress"));
        }
        let mut journal = Self { storage, limits, fortress, id, header_digest, head: header_digest,
            length: HEADER_BYTES as u64, entries: Vec::new(), state: LiveOperationsState::default(),
            fenced: false, repaired_tail_bytes: 0 };
        while journal.length < length {
            check(context, started)?;
            if journal.entries.len() >= limits.max_records {
                return Err(budget("operations journal exceeds its record limit"));
            }
            let remaining = length - journal.length;
            journal.storage.seek(SeekFrom::Start(journal.length)).map_err(storage_error)?;
            if remaining < FRAME_HEADER_BYTES as u64 {
                let mut prefix = vec![0; remaining as usize];
                journal.storage.read_exact(&mut prefix).map_err(storage_error)?;
                let comparable = prefix.len().min(RECORD.len());
                if prefix[..comparable] != RECORD[..comparable] {
                    return Err(corrupt("journal trailing bytes do not begin a record; no repair applied"));
                }
                break;
            }
            let mut prefix = [0; FRAME_HEADER_BYTES];
            journal.storage.read_exact(&mut prefix).map_err(storage_error)?;
            let body_length = decode_frame_header(&prefix, journal.id)?;
            let frame_length = FRAME_HEADER_BYTES + body_length + 32 + FOOTER.len();
            if remaining < frame_length as u64 { break; }
            let mut frame = vec![0; frame_length]; frame[..FRAME_HEADER_BYTES].copy_from_slice(&prefix);
            journal.storage.read_exact(&mut frame[FRAME_HEADER_BYTES..]).map_err(storage_error)?;
            let (entry, observation) = decode_frame(&frame, journal.id, journal.length)?;
            journal.accept_replayed(entry, observation, context)?;
            journal.length += frame_length as u64;
        }
        if journal.length != length {
            if recovery != TailRecovery::TruncateIncomplete {
                return Err(corrupt("incomplete operations journal tail; operator repair opt-in is required; original file is unchanged"));
            }
            context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
            check(context, started)?;
            journal.storage.validate_identity().map_err(storage_error)?;
            if journal.storage.seek(SeekFrom::End(0)).map_err(storage_error)? != length {
                return Err(corrupt("journal changed during recovery; no repair applied"));
            }
            journal.storage.truncate(journal.length).map_err(storage_error)?;
            journal.storage.sync().map_err(storage_error)?;
            journal.repaired_tail_bytes = length - journal.length;
        }
        check(context, started)?;
        journal.storage.validate_identity().map_err(storage_error)?;
        Ok(journal)
    }

    fn accept_replayed(&mut self, entry: JournalEntry, observation: LiveOperationsObservation,
        context: &OperationContext) -> Result<()> {
        if entry.number != self.entries.len() as u64 + 1 || entry.previous_digest != self.head
            || entry.anchor.fortress_id != self.fortress || observation.source_digest()? != entry.source_digest {
            return Err(corrupt("operations journal sequence, predecessor, fortress or source digest disagrees"));
        }
        check_observation(&observation, context)?;
        let outcome = self.state.publish(observation)?;
        if outcome == JobPublication::Heartbeat || self.state.snapshot().map(WorldSnapshot::anchor) != Some(entry.anchor) {
            return Err(corrupt("operations journal does not reproduce its recorded projection anchor"));
        }
        self.head = entry.record_digest;
        self.entries.push(entry);
        Ok(())
    }

    pub fn append(&mut self, observation: LiveOperationsObservation, context: &OperationContext) -> Result<JobPublication> {
        let started = Instant::now();
        check(context, started)?;
        context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        self.ensure_healthy(context)?;
        check_observation(&observation, context)?;
        if observation.jobs.fortress_id()? != self.fortress { return Err(corrupt("journal observation has another fortress")); }
        let mut candidate = self.state.clone();
        let outcome = candidate.publish(observation.clone())?;
        if outcome == JobPublication::Heartbeat { return Ok(outcome); }
        let anchor = candidate.snapshot().map(WorldSnapshot::anchor).ok_or_else(||corrupt("journal candidate snapshot missing"))?;
        let mut target_context = context.clone(); target_context.anchor = anchor;
        target_context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        if self.entries.len() >= self.limits.max_records { return Err(budget("operations journal record capacity reached; rotate explicitly")); }
        let frame = encode_frame(self.id, self.entries.len() as u64 + 1, self.head, anchor, &observation)?;
        let next_length = self.length.checked_add(frame.len() as u64).ok_or_else(||budget("journal length overflow"))?;
        if next_length > self.limits.max_bytes { return Err(budget("operations journal byte capacity reached; rotate explicitly")); }
        let (entry, _) = decode_frame(&frame, self.id, self.length)?;
        check(context, started)?;
        let write_result = (|| -> io::Result<()> {
            self.storage.validate_identity()?;
            if self.storage.seek(SeekFrom::End(0))? != self.length {
                return Err(io::Error::other("journal length changed"));
            }
            self.storage.write_all(&frame)?;
            self.storage.sync()?;
            self.storage.validate_identity()
        })();
        if write_result.is_err() {
            self.fenced = true;
            return Err(corrupt("journal append outcome is uncertain; no new in-memory anchor was published; reopen for verified recovery"));
        }
        // No fallible step after this durability boundary may hide a committed record.
        self.length = next_length; self.head = entry.record_digest;
        self.entries.push(entry); self.state = candidate;
        Ok(outcome)
    }

    fn ensure_healthy(&mut self, context: &OperationContext) -> Result<()> {
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if self.fenced { return Err(corrupt("operations journal is fenced; reopen for verified recovery")); }
        if context.anchor.fortress_id != self.fortress { return Err(corrupt("journal request names another fortress")); }
        if self.storage.validate_identity().is_err()
            || self.storage.seek(SeekFrom::End(0)).map_err(storage_error)? != self.length {
            self.fenced = true; return Err(corrupt("operations journal identity or length changed"));
        }
        Ok(())
    }

    /// Historical authority is checked at the caller's CURRENT anchor, never at
    /// the archived tick. Every replayed source and generated anchor is rechecked.
    pub fn snapshot_at(&mut self, number: u64, digest: Digest32, context: &OperationContext) -> Result<WorldSnapshot> {
        let started = Instant::now(); check(context, started)?; self.ensure_healthy(context)?;
        let index = number.checked_sub(1).and_then(|n|usize::try_from(n).ok())
            .filter(|&i|i < self.entries.len()).ok_or_else(||DfmcpError::new(ErrorCode::CursorGap,"journal record is not retained"))?;
        if self.entries[index].record_digest != digest { return Err(DfmcpError::new(ErrorCode::StaleAnchor,"journal record digest differs")); }
        self.storage.seek(SeekFrom::Start(0)).map_err(storage_error)?;
        let mut header = [0; HEADER_BYTES]; self.storage.read_exact(&mut header).map_err(storage_error)?;
        if decode_header(&header)? != (self.fortress,self.id,self.header_digest) { self.fenced=true; return Err(corrupt("journal header changed")); }
        let mut state = LiveOperationsState::default();
        let mut previous = self.header_digest;
        for i in 0..=index {
            check(context, started)?;
            let known = &self.entries[i];
            self.storage.seek(SeekFrom::Start(known.offset)).map_err(storage_error)?;
            let mut frame = vec![0; known.encoded_bytes as usize];
            self.storage.read_exact(&mut frame).map_err(storage_error)?;
            let (entry, observation) = decode_frame(&frame,self.id,known.offset)?;
            if &entry != known || entry.previous_digest != previous { self.fenced=true; return Err(corrupt("journal record changed after opening")); }
            check_observation(&observation,context)?;
            if state.publish(observation)? == JobPublication::Heartbeat
                || state.snapshot().map(WorldSnapshot::anchor) != Some(entry.anchor) {
                self.fenced=true; return Err(corrupt("historical projection anchor does not reproduce"));
            }
            previous = entry.record_digest;
        }
        check(context,started)?;
        state.snapshot().cloned().ok_or_else(||corrupt("historical snapshot absent"))
    }

    pub fn entries(&self) -> &[JournalEntry] { &self.entries }
    pub fn state(&self) -> &LiveOperationsState { &self.state }
    pub fn id(&self) -> Digest32 { self.id }
    pub fn head(&self) -> Digest32 { self.head }
    pub fn retained_bytes(&self) -> u64 { self.length }
    pub fn repaired_tail_bytes(&self) -> u64 { self.repaired_tail_bytes }
    pub fn fenced(&self) -> bool { self.fenced }
}

fn check_observation(observation: &LiveOperationsObservation, context: &OperationContext) -> Result<()> {
    if observation.jobs.jobs.len().saturating_add(observation.items.len()).saturating_add(observation.buildings.len()).saturating_add(1)
        > context.budget.max_entities as usize || observation.encode_payload()?.len() as u64 > context.budget.max_bytes {
        return Err(budget("archived operations observation exceeds the current session's acquisition limits"));
    }
    Ok(())
}
fn header_hash(bytes: &[u8]) -> Digest32 {
    let mut input = b"dfmcp-operations-journal-header/1\0".to_vec(); input.extend_from_slice(bytes); Digest32::of_bytes(&input)
}
fn frame_hash(id: Digest32, prefix: &[u8]) -> Digest32 {
    let mut bytes=b"dfmcp-operations-journal-record/1\0".to_vec(); bytes.extend_from_slice(id.as_bytes());
    bytes.extend_from_slice(prefix); Digest32::of_bytes(&bytes)
}
fn frame_header_hash(id:Digest32,prefix:&[u8])->Digest32 {
    let mut bytes=b"dfmcp-operations-journal-frame-header/1\0".to_vec();bytes.extend_from_slice(id.as_bytes());
    bytes.extend_from_slice(prefix);Digest32::of_bytes(&bytes)
}
fn decode_frame_header(prefix:&[u8],id:Digest32)->Result<usize> {
    if prefix.len()!=FRAME_HEADER_BYTES || &prefix[..8]!=RECORD {
        return Err(corrupt("journal record marker is invalid"));
    }
    let size=u32::from_be_bytes(prefix[8..12].try_into().map_err(|_|corrupt("journal record length"))?) as usize;
    if !(148..=MAX_BODY_BYTES).contains(&size) || &prefix[12..]!=frame_header_hash(id,&prefix[..12]).as_bytes() {
        return Err(corrupt("journal length-header checksum failed; no repair applied"));
    }
    Ok(size)
}
fn decode_header(bytes: &[u8; HEADER_BYTES]) -> Result<(FortressId,Digest32,Digest32)> {
    let mut r=Reader(bytes); if r.take(8)? != MAGIC { return Err(corrupt("unsupported operations journal schema")); }
    let fortress=FortressId::new(r.u64()?); let id=r.digest()?; let digest=r.digest()?;
    if fortress==FortressId::NIL || id==Digest32::ZERO || digest!=header_hash(&bytes[..48]) {
        return Err(corrupt("operations journal header checksum or identity is invalid"));
    }
    Ok((fortress,id,digest))
}
fn put_u64(out:&mut Vec<u8>,value:u64){out.extend_from_slice(&value.to_be_bytes());}
fn put_text(out:&mut Vec<u8>,value:&str)->Result<()> {
    if value.is_empty() || value.len()>128 || value.contains('\0'){return Err(corrupt("invalid journal manifest text"));}
    out.extend_from_slice(&(value.len() as u16).to_be_bytes());out.extend_from_slice(value.as_bytes());Ok(())
}
fn encode_frame(id:Digest32,number:u64,previous:Digest32,anchor:StateAnchor,observation:&LiveOperationsObservation)->Result<Vec<u8>> {
    let mut body=Vec::new();put_u64(&mut body,number);body.extend_from_slice(previous.as_bytes());
    for n in [anchor.fortress_id.get(),anchor.cursor.epoch,anchor.cursor.sequence,anchor.tick.0]{put_u64(&mut body,n);}
    body.extend_from_slice(anchor.state_hash.as_bytes());body.extend_from_slice(observation.source_digest()?.as_bytes());
    put_u64(&mut body,observation.jobs.bridge_generation);put_text(&mut body,&observation.jobs.df_version)?;
    put_text(&mut body,&observation.jobs.dfhack_version)?;
    let payload=observation.encode_payload()?;body.extend_from_slice(&(payload.len() as u32).to_be_bytes());body.extend_from_slice(&payload);
    if body.len()>MAX_BODY_BYTES{return Err(budget("journal frame exceeds its bound"));}
    let mut frame=RECORD.to_vec();frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    let header_digest=frame_header_hash(id,&frame);frame.extend_from_slice(header_digest.as_bytes());frame.extend_from_slice(&body);
    let digest=frame_hash(id,&frame);frame.extend_from_slice(digest.as_bytes());frame.extend_from_slice(FOOTER);Ok(frame)
}
fn decode_frame(frame:&[u8],id:Digest32,offset:u64)->Result<(JournalEntry,LiveOperationsObservation)> {
    let mut r=Reader(frame);let size=decode_frame_header(r.take(FRAME_HEADER_BYTES)?,id)?;
    if frame.len()!=size+FRAME_HEADER_BYTES+40 {return Err(corrupt("invalid journal frame length"));}
    let mut body=Reader(r.take(size)?);let record_digest=r.digest()?;
    if r.take(8)?!=FOOTER || record_digest!=frame_hash(id,&frame[..size+FRAME_HEADER_BYTES]) {return Err(corrupt("journal record checksum or commit footer failed; no repair applied"));}
    let number=body.u64()?;let previous_digest=body.digest()?;
    let anchor=StateAnchor {fortress_id:FortressId::new(body.u64()?),
        cursor:ObservationCursor{epoch:body.u64()?,sequence:body.u64()?},tick:GameTick(body.u64()?),state_hash:body.digest()?};
    let source_digest=body.digest()?;let generation=body.u64()?;let df=body.text()?;let dfhack=body.text()?;let n=body.u32()? as usize;
    let observation=LiveOperationsObservation::decode_payload(body.take(n)?,generation,df,dfhack)?;
    if !body.0.is_empty(){return Err(corrupt("trailing journal record data"));}
    Ok((JournalEntry{number,anchor,source_digest,record_digest,previous_digest,offset,encoded_bytes:frame.len() as u32},observation))
}
struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self,n:usize)->Result<&'a [u8]>{
        let out=self.0.get(..n).ok_or_else(||corrupt("truncated journal record"))?;self.0=&self.0[n..];Ok(out)
    }
    fn u32(&mut self)->Result<u32>{Ok(u32::from_be_bytes(self.take(4)?.try_into().map_err(|_|corrupt("journal u32"))?))}
    fn u64(&mut self)->Result<u64>{Ok(u64::from_be_bytes(self.take(8)?.try_into().map_err(|_|corrupt("journal u64"))?))}
    fn digest(&mut self)->Result<Digest32>{Ok(Digest32::from_bytes(self.take(32)?.try_into().map_err(|_|corrupt("journal digest"))?))}
    fn text(&mut self)->Result<String>{
        let n=u16::from_be_bytes(self.take(2)?.try_into().map_err(|_|corrupt("journal text"))?) as usize;
        if !(1..=128).contains(&n){return Err(corrupt("journal manifest text exceeds its bound"));}
        let text=std::str::from_utf8(self.take(n)?).map_err(|_|corrupt("journal manifest is not UTF-8"))?;
        if text.contains('\0'){return Err(corrupt("journal manifest contains NUL"));}Ok(text.to_owned())
    }
}

#[cfg(test)]
#[path = "operations_journal_tests.rs"]
mod tests;
