#![forbid(unsafe_code)]

//! Durable control/1.7 coordinator state. This journal records only pause-effect
//! coordination; it is not game-state history and it never grants mutation authority.
//! A synced `CommitStarted` record is required before bridge dispatch. Any crash or
//! ambiguous transport outcome therefore recovers as reconciliation-required rather
//! than as a retryable failure.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

use dfmcp_core::{Capability, DfmcpError, Digest32, ErrorCode, OperationContext, Result, RiskTier};

const MAGIC: &[u8; 8] = b"DFMCEJ01";
const RECORD: &[u8; 8] = b"DFMCREC1";
const FOOTER: &[u8; 8] = b"DFMCEND1";
const HEADER_BYTES: usize = 72;
const FRAME_HEADER_BYTES: usize = 44;
const MAX_KEY_BYTES: usize = 512;
const MAX_BODY_BYTES: usize = 768;
const MAX_LEDGER_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TRANSITIONS: usize = 16_384;
const MAX_EFFECTS: usize = 4_096;

fn corrupt(message: &str) -> DfmcpError { DfmcpError::new(ErrorCode::CorruptLedger, message) }
fn invalid(message: &str) -> DfmcpError { DfmcpError::new(ErrorCode::InvalidRequest, message) }
fn conflict(message: &str) -> DfmcpError { DfmcpError::new(ErrorCode::Conflict, message) }
fn exhausted(message: &str) -> DfmcpError { DfmcpError::new(ErrorCode::BudgetExceeded, message) }
fn storage_error(_: io::Error) -> DfmcpError { corrupt("control effect journal I/O failed; reopen for verified recovery") }

fn authorize(context: &OperationContext) -> Result<()> {
    context.authorize(Capability::ControlClock, RiskTier::Reversible, &[], None)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectTailRecovery { Refuse, TruncateIncomplete }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum DurablePauseState {
    Prepared = 1,
    CommitStarted = 2,
    VerifiedApplied = 3,
    VerifiedNotApplied = 4,
    Indeterminate = 5,
}
impl DurablePauseState {
    fn decode(tag: u8) -> Result<Self> {
        match tag {
            1 => Ok(Self::Prepared),
            2 => Ok(Self::CommitStarted),
            3 => Ok(Self::VerifiedApplied),
            4 => Ok(Self::VerifiedNotApplied),
            5 => Ok(Self::Indeterminate),
            _ => Err(corrupt("control effect journal contains an unknown state")),
        }
    }
    #[must_use]
    pub const fn terminal(self) -> bool {
        matches!(self, Self::VerifiedApplied | Self::VerifiedNotApplied)
    }
    #[must_use]
    pub const fn reconciliation_required(self) -> bool {
        matches!(self, Self::CommitStarted | Self::Indeterminate)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurablePauseRecord {
    pub idempotency_key: String,
    pub plan_digest: Digest32,
    pub desired_paused: bool,
    pub expected_game_tick: u64,
    pub bridge_generation: u64,
    pub prepare_token: [u8; 16],
    pub state: DurablePauseState,
    pub effect_known: bool,
    pub effect_applied: bool,
    pub observed_paused: Option<bool>,
    pub observed_game_tick: Option<u64>,
    pub receipt_digest: Option<Digest32>,
    pub revision: u64,
    pub transition_number: u64,
    pub previous_digest: Digest32,
    pub record_digest: Digest32,
}
impl DurablePauseRecord {
    #[must_use]
    pub const fn safe_to_dispatch(&self, bridge_generation: u64) -> bool {
        self.state == DurablePauseState::Prepared && self.bridge_generation == bridge_generation
    }
}

pub trait EffectJournalStorage: Read + Write + Seek {
    fn sync(&mut self) -> io::Result<()>;
    fn truncate(&mut self, length: u64) -> io::Result<()>;
    fn validate_identity(&self) -> io::Result<()> { Ok(()) }
}

pub struct ControlEffectJournal<S> {
    storage: S,
    id: Digest32,
    head: Digest32,
    length: u64,
    transitions: usize,
    records: BTreeMap<String, DurablePauseRecord>,
    fenced: bool,
    repaired_tail_bytes: u64,
}

impl<S: EffectJournalStorage> ControlEffectJournal<S> {
    pub fn open(mut storage: S, context: &OperationContext, initialize_empty: bool,
        bridge_generation: u64, recovery: EffectTailRecovery) -> Result<Self> {
        authorize(context)?;
        if bridge_generation == 0 || bridge_generation == u64::MAX {
            return Err(invalid("control bridge generation is invalid"));
        }
        storage.validate_identity().map_err(storage_error)?;
        let mut file_length = storage.seek(SeekFrom::End(0)).map_err(storage_error)?;
        if file_length > MAX_LEDGER_BYTES { return Err(exhausted("control effect journal exceeds 64 MiB")); }
        if file_length == 0 && initialize_empty {
            let mut identity = b"dfmcp-control-effect-journal-incarnation/1\0".to_vec();
            identity.extend_from_slice(&context.session_id.get().to_be_bytes());
            identity.extend_from_slice(&context.request_id.get().to_be_bytes());
            identity.extend_from_slice(&bridge_generation.to_be_bytes());
            let id = Digest32::of_bytes(&identity);
            let mut header = MAGIC.to_vec();
            header.extend_from_slice(id.as_bytes());
            let digest = header_hash(&header);
            header.extend_from_slice(digest.as_bytes());
            storage.seek(SeekFrom::Start(0)).map_err(storage_error)?;
            storage.write_all(&header).map_err(storage_error)?;
            storage.sync().map_err(storage_error)?;
            file_length = HEADER_BYTES as u64;
        }
        if file_length < HEADER_BYTES as u64 {
            return Err(corrupt("control effect journal header is incomplete; original bytes are unchanged"));
        }
        storage.seek(SeekFrom::Start(0)).map_err(storage_error)?;
        let mut header = [0u8; HEADER_BYTES];
        storage.read_exact(&mut header).map_err(storage_error)?;
        let (id, header_digest) = decode_header(&header)?;
        let mut journal = Self { storage, id, head: header_digest,
            length: HEADER_BYTES as u64, transitions: 0, records: BTreeMap::new(),
            fenced: false, repaired_tail_bytes: 0 };
        while journal.length < file_length {
            if journal.transitions >= MAX_TRANSITIONS {
                return Err(exhausted("control effect journal transition limit reached"));
            }
            let remaining = file_length - journal.length;
            journal.storage.seek(SeekFrom::Start(journal.length)).map_err(storage_error)?;
            if remaining < FRAME_HEADER_BYTES as u64 {
                let mut prefix = vec![0u8; remaining as usize];
                journal.storage.read_exact(&mut prefix).map_err(storage_error)?;
                let comparable = prefix.len().min(RECORD.len());
                if prefix[..comparable] != RECORD[..comparable] {
                    return Err(corrupt("control journal trailing bytes are not an incomplete record"));
                }
                break;
            }
            let mut prefix = [0u8; FRAME_HEADER_BYTES];
            journal.storage.read_exact(&mut prefix).map_err(storage_error)?;
            let body_length = decode_frame_header(&prefix, journal.id)?;
            let frame_length = FRAME_HEADER_BYTES + body_length + 32 + FOOTER.len();
            if remaining < frame_length as u64 { break; }
            let mut frame = vec![0u8; frame_length];
            frame[..FRAME_HEADER_BYTES].copy_from_slice(&prefix);
            journal.storage.read_exact(&mut frame[FRAME_HEADER_BYTES..]).map_err(storage_error)?;
            let record = decode_frame(&frame, journal.id)?;
            journal.accept_replay(record)?;
            journal.length += frame_length as u64;
        }
        if journal.length != file_length {
            if recovery != EffectTailRecovery::TruncateIncomplete {
                return Err(corrupt("incomplete control effect journal tail; explicit operator repair is required"));
            }
            authorize(context)?;
            journal.storage.validate_identity().map_err(storage_error)?;
            if journal.storage.seek(SeekFrom::End(0)).map_err(storage_error)? != file_length {
                return Err(corrupt("control journal changed during recovery; no repair applied"));
            }
            journal.storage.truncate(journal.length).map_err(storage_error)?;
            journal.storage.sync().map_err(storage_error)?;
            journal.repaired_tail_bytes = file_length - journal.length;
        }
        journal.storage.validate_identity().map_err(storage_error)?;
        Ok(journal)
    }

    fn accept_replay(&mut self, record: DurablePauseRecord) -> Result<()> {
        if record.record_digest == Digest32::ZERO
            || record.transition_number != self.transitions as u64 + 1
            || record.previous_digest != self.head {
            return Err(corrupt("control effect journal sequence or predecessor chain disagrees"));
        }
        let expected_revision = self.records.get(&record.idempotency_key).map_or(1, |prior| prior.revision.saturating_add(1));
        if record.revision != expected_revision { return Err(corrupt("control effect revision is not contiguous")); }
        validate_transition(self.records.get(&record.idempotency_key), &record)?;
        self.head = record.record_digest;
        self.transitions += 1;
        self.records.insert(record.idempotency_key.clone(), record);
        Ok(())
    }

    fn ensure_healthy(&mut self, context: &OperationContext) -> Result<()> {
        authorize(context)?;
        if self.fenced { return Err(corrupt("control effect journal is fenced; reopen for verified recovery")); }
        if self.storage.validate_identity().is_err()
            || self.storage.seek(SeekFrom::End(0)).map_err(storage_error)? != self.length {
            self.fenced = true;
            return Err(corrupt("control effect journal identity or length changed"));
        }
        Ok(())
    }

    fn append(&mut self, mut next: DurablePauseRecord, context: &OperationContext) -> Result<DurablePauseRecord> {
        self.ensure_healthy(context)?;
        if self.transitions >= MAX_TRANSITIONS { return Err(exhausted("control effect journal transition limit reached")); }
        if !self.records.contains_key(&next.idempotency_key) && self.records.len() >= MAX_EFFECTS {
            return Err(exhausted("control effect journal effect limit reached"));
        }
        next.revision = self.records.get(&next.idempotency_key).map_or(1, |prior| prior.revision.saturating_add(1));
        next.transition_number = self.transitions as u64 + 1;
        next.previous_digest = self.head;
        validate_transition(self.records.get(&next.idempotency_key), &next)?;
        let frame = encode_frame(self.id, &next)?;
        let next_length = self.length.checked_add(frame.len() as u64).ok_or_else(||exhausted("control journal length overflow"))?;
        if next_length > MAX_LEDGER_BYTES { return Err(exhausted("control effect journal byte limit reached")); }
        let decoded = decode_frame(&frame, self.id)?;
        let write = (|| -> io::Result<()> {
            self.storage.validate_identity()?;
            if self.storage.seek(SeekFrom::End(0))? != self.length { return Err(io::Error::other("journal length changed")); }
            self.storage.write_all(&frame)?;
            self.storage.sync()?;
            self.storage.validate_identity()
        })();
        if write.is_err() {
            self.fenced = true;
            return Err(corrupt("control journal append outcome is uncertain; reopen and reconcile before any effect retry"));
        }
        self.length = next_length;
        self.head = decoded.record_digest;
        self.transitions += 1;
        self.records.insert(decoded.idempotency_key.clone(), decoded.clone());
        Ok(decoded)
    }

    pub fn record_prepared(&mut self, key: String, plan_digest: Digest32, desired_paused: bool,
        expected_game_tick: u64, bridge_generation: u64, prepare_token: [u8; 16],
        context: &OperationContext) -> Result<DurablePauseRecord> {
        validate_key(&key)?;
        if bridge_generation == 0 || bridge_generation == u64::MAX || prepare_token == [0u8; 16] {
            return Err(invalid("invalid prepared pause effect identity"));
        }
        if let Some(existing) = self.records.get(&key) {
            if existing.plan_digest == plan_digest && existing.desired_paused == desired_paused
                && existing.expected_game_tick == expected_game_tick && existing.bridge_generation == bridge_generation
                && existing.prepare_token == prepare_token {
                return Ok(existing.clone());
            }
            return Err(conflict("idempotency key already names different durable pause-effect content"));
        }
        self.append(DurablePauseRecord { idempotency_key:key, plan_digest, desired_paused,
            expected_game_tick, bridge_generation, prepare_token, state:DurablePauseState::Prepared,
            effect_known:false, effect_applied:false, observed_paused:None, observed_game_tick:None,
            receipt_digest:None, revision:0, transition_number:0, previous_digest:Digest32::ZERO,
            record_digest:Digest32::ZERO }, context)
    }

    pub fn begin_commit(&mut self, key: &str, plan_digest: Digest32, bridge_generation: u64,
        context: &OperationContext) -> Result<DurablePauseRecord> {
        let current = self.require(key, plan_digest)?;
        if current.bridge_generation != bridge_generation {
            return Err(DfmcpError::new(ErrorCode::EffectIndeterminate,
                "prepared pause effect belongs to another bridge generation; replan with a new idempotency key"));
        }
        match current.state {
            DurablePauseState::Prepared => {
                let mut next = current; next.state = DurablePauseState::CommitStarted;
                self.append(next, context)
            }
            DurablePauseState::CommitStarted | DurablePauseState::Indeterminate => Err(DfmcpError::new(
                ErrorCode::EffectIndeterminate, "pause effect has an unresolved commit attempt; reconcile before any retry")),
            _ => Ok(current),
        }
    }

    pub fn mark_indeterminate(&mut self, key: &str, plan_digest: Digest32,
        context: &OperationContext) -> Result<DurablePauseRecord> {
        let current = self.require(key, plan_digest)?;
        if current.state == DurablePauseState::Indeterminate { return Ok(current); }
        if current.state != DurablePauseState::CommitStarted {
            return Err(conflict("only a started pause commit can become indeterminate"));
        }
        let mut next = current; next.state = DurablePauseState::Indeterminate;
        self.append(next, context)
    }

    pub fn record_reconciliation(&mut self, key: &str, plan_digest: Digest32,
        bridge_generation: u64, effect_known: bool, effect_applied: bool,
        observed_paused: bool, observed_game_tick: u64, receipt_digest: Option<Digest32>,
        context: &OperationContext) -> Result<DurablePauseRecord> {
        let current = self.require(key, plan_digest)?;
        if current.state.terminal() { return Ok(current); }
        if current.state == DurablePauseState::Prepared {
            return Err(conflict("pause effect has not begun commit and cannot be reconciled as an effect attempt"));
        }
        if bridge_generation != current.bridge_generation || !effect_known {
            return self.mark_indeterminate(key, plan_digest, context);
        }
        if effect_applied && observed_paused != current.desired_paused {
            return Err(corrupt("bridge claimed a pause effect applied but observed the opposite pause state"));
        }
        if effect_applied && receipt_digest.is_none() {
            return Err(corrupt("applied pause effect lacks a receipt digest"));
        }
        let mut next = current;
        next.state = if effect_applied { DurablePauseState::VerifiedApplied } else { DurablePauseState::VerifiedNotApplied };
        next.effect_known = true;
        next.effect_applied = effect_applied;
        next.observed_paused = Some(observed_paused);
        next.observed_game_tick = Some(observed_game_tick);
        next.receipt_digest = receipt_digest;
        self.append(next, context)
    }

    fn require(&self, key: &str, digest: Digest32) -> Result<DurablePauseRecord> {
        validate_key(key)?;
        let record = self.records.get(key).ok_or_else(||invalid("pause effect is not present in the durable control journal"))?;
        if record.plan_digest != digest { return Err(conflict("idempotency key belongs to another plan digest")); }
        Ok(record.clone())
    }

    #[must_use]
    pub fn lookup(&self, key: &str) -> Option<&DurablePauseRecord> { self.records.get(key) }
    #[must_use]
    pub fn id(&self) -> Digest32 { self.id }
    #[must_use]
    pub fn head(&self) -> Digest32 { self.head }
    #[must_use]
    pub fn retained_bytes(&self) -> u64 { self.length }
    #[must_use]
    pub fn transition_count(&self) -> usize { self.transitions }
    #[must_use]
    pub fn effect_count(&self) -> usize { self.records.len() }
    #[must_use]
    pub fn fenced(&self) -> bool { self.fenced }
    #[must_use]
    pub fn repaired_tail_bytes(&self) -> u64 { self.repaired_tail_bytes }
}

fn validate_key(key: &str) -> Result<()> {
    if key.is_empty() || key.len() > MAX_KEY_BYTES || key.chars().any(char::is_control) {
        return Err(invalid("idempotency key must contain 1..=512 non-control bytes"));
    }
    Ok(())
}

fn validate_transition(previous: Option<&DurablePauseRecord>, next: &DurablePauseRecord) -> Result<()> {
    validate_key(&next.idempotency_key)?;
    if next.bridge_generation == 0 || next.bridge_generation == u64::MAX || next.prepare_token == [0u8; 16]
        || next.transition_number == 0 || next.previous_digest == Digest32::ZERO {
        return Err(corrupt("invalid durable pause record identity"));
    }
    match previous {
        None => {
            if next.revision != 1 || next.state != DurablePauseState::Prepared || next.effect_known
                || next.effect_applied || next.observed_paused.is_some() || next.observed_game_tick.is_some()
                || next.receipt_digest.is_some() {
                return Err(corrupt("first durable pause record must be a clean prepared state"));
            }
        }
        Some(previous) => {
            if next.revision != previous.revision.checked_add(1).ok_or_else(||corrupt("pause revision overflow"))?
                || next.idempotency_key != previous.idempotency_key || next.plan_digest != previous.plan_digest
                || next.desired_paused != previous.desired_paused || next.expected_game_tick != previous.expected_game_tick
                || next.bridge_generation != previous.bridge_generation || next.prepare_token != previous.prepare_token {
                return Err(corrupt("durable pause transition changed immutable effect identity"));
            }
            let allowed = matches!((previous.state, next.state),
                (DurablePauseState::Prepared, DurablePauseState::CommitStarted)
                | (DurablePauseState::CommitStarted, DurablePauseState::Indeterminate)
                | (DurablePauseState::CommitStarted, DurablePauseState::VerifiedApplied)
                | (DurablePauseState::CommitStarted, DurablePauseState::VerifiedNotApplied)
                | (DurablePauseState::Indeterminate, DurablePauseState::VerifiedApplied)
                | (DurablePauseState::Indeterminate, DurablePauseState::VerifiedNotApplied));
            if !allowed { return Err(corrupt("invalid durable pause-effect state transition")); }
        }
    }
    if next.state.terminal() {
        if !next.effect_known || next.observed_paused.is_none() || next.observed_game_tick.is_none()
            || next.effect_applied != (next.state == DurablePauseState::VerifiedApplied) {
            return Err(corrupt("terminal pause-effect record lacks a complete reconciled outcome"));
        }
        if next.effect_applied && next.receipt_digest.is_none() {
            return Err(corrupt("verified applied pause effect lacks receipt evidence"));
        }
    } else if next.effect_known || next.effect_applied || next.observed_paused.is_some()
        || next.observed_game_tick.is_some() || next.receipt_digest.is_some() {
        return Err(corrupt("nonterminal pause-effect record carried terminal evidence"));
    }
    Ok(())
}

fn header_hash(bytes: &[u8]) -> Digest32 {
    let mut input = b"dfmcp-control-effect-journal-header/1\0".to_vec(); input.extend_from_slice(bytes); Digest32::of_bytes(&input)
}
fn frame_header_hash(id: Digest32, bytes: &[u8]) -> Digest32 {
    let mut input = b"dfmcp-control-effect-journal-frame-header/1\0".to_vec(); input.extend_from_slice(id.as_bytes()); input.extend_from_slice(bytes); Digest32::of_bytes(&input)
}
fn frame_hash(id: Digest32, bytes: &[u8]) -> Digest32 {
    let mut input = b"dfmcp-control-effect-journal-record/1\0".to_vec(); input.extend_from_slice(id.as_bytes()); input.extend_from_slice(bytes); Digest32::of_bytes(&input)
}
fn decode_header(bytes: &[u8; HEADER_BYTES]) -> Result<(Digest32, Digest32)> {
    if &bytes[..8] != MAGIC { return Err(corrupt("unsupported control effect journal schema")); }
    let id = Digest32::from_bytes(bytes[8..40].try_into().map_err(|_|corrupt("control ledger id"))?);
    let digest = Digest32::from_bytes(bytes[40..72].try_into().map_err(|_|corrupt("control header digest"))?);
    if id == Digest32::ZERO || digest != header_hash(&bytes[..40]) { return Err(corrupt("control effect journal header checksum failed")); }
    Ok((id, digest))
}
fn decode_frame_header(prefix: &[u8], id: Digest32) -> Result<usize> {
    if prefix.len() != FRAME_HEADER_BYTES || &prefix[..8] != RECORD { return Err(corrupt("invalid control record marker")); }
    let length = u32::from_be_bytes(prefix[8..12].try_into().map_err(|_|corrupt("control record length"))?) as usize;
    if !(120..=MAX_BODY_BYTES).contains(&length) || &prefix[12..44] != frame_header_hash(id, &prefix[..12]).as_bytes() {
        return Err(corrupt("control record header checksum failed"));
    }
    Ok(length)
}
fn encode_frame(id: Digest32, record: &DurablePauseRecord) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    body.extend_from_slice(&record.transition_number.to_be_bytes());
    body.extend_from_slice(record.previous_digest.as_bytes());
    body.extend_from_slice(&record.revision.to_be_bytes());
    body.extend_from_slice(&(record.idempotency_key.len() as u16).to_be_bytes());
    body.extend_from_slice(record.idempotency_key.as_bytes());
    body.extend_from_slice(record.plan_digest.as_bytes());
    body.push(u8::from(record.desired_paused));
    body.extend_from_slice(&record.expected_game_tick.to_be_bytes());
    body.extend_from_slice(&record.bridge_generation.to_be_bytes());
    body.push(record.state as u8);
    body.extend_from_slice(&record.prepare_token);
    body.push(u8::from(record.effect_known));
    body.push(u8::from(record.effect_applied));
    body.push(match record.observed_paused { None => 0, Some(false) => 1, Some(true) => 2 });
    body.extend_from_slice(&record.observed_game_tick.unwrap_or(0).to_be_bytes());
    body.extend_from_slice(record.receipt_digest.unwrap_or(Digest32::ZERO).as_bytes());
    if body.len() > MAX_BODY_BYTES { return Err(exhausted("control record body exceeds its bound")); }
    let mut frame = RECORD.to_vec();
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    let header = frame_header_hash(id, &frame); frame.extend_from_slice(header.as_bytes()); frame.extend_from_slice(&body);
    let digest = frame_hash(id, &frame); frame.extend_from_slice(digest.as_bytes()); frame.extend_from_slice(FOOTER);
    Ok(frame)
}
fn decode_frame(frame: &[u8], id: Digest32) -> Result<DurablePauseRecord> {
    let mut reader = Reader(frame);
    let body_length = decode_frame_header(reader.take(FRAME_HEADER_BYTES)?, id)?;
    if frame.len() != FRAME_HEADER_BYTES + body_length + 32 + FOOTER.len() { return Err(corrupt("control record length mismatch")); }
    let body_bytes = reader.take(body_length)?;
    let digest = reader.digest()?;
    if reader.take(FOOTER.len())? != FOOTER || digest != frame_hash(id, &frame[..FRAME_HEADER_BYTES + body_length]) {
        return Err(corrupt("control record checksum or commit footer failed"));
    }
    let mut body = Reader(body_bytes);
    let transition_number = body.u64()?;
    let previous_digest = body.digest()?;
    let revision = body.u64()?;
    let key = body.text(MAX_KEY_BYTES)?;
    let plan_digest = body.digest()?;
    let desired_paused = body.boolean()?;
    let expected_game_tick = body.u64()?;
    let bridge_generation = body.u64()?;
    let state = DurablePauseState::decode(body.byte()?)?;
    let prepare_token: [u8; 16] = body.take(16)?.try_into().map_err(|_|corrupt("prepare token length"))?;
    let effect_known = body.boolean()?;
    let effect_applied = body.boolean()?;
    let observed_paused = match body.byte()? { 0 => None, 1 => Some(false), 2 => Some(true), _ => return Err(corrupt("invalid observed pause tag")) };
    let observed_raw = body.u64()?;
    let receipt_raw = body.digest()?;
    if !body.0.is_empty() { return Err(corrupt("trailing control record bytes")); }
    Ok(DurablePauseRecord { idempotency_key:key, plan_digest, desired_paused, expected_game_tick,
        bridge_generation, prepare_token, state, effect_known, effect_applied, observed_paused,
        observed_game_tick: observed_paused.map(|_|observed_raw), receipt_digest:(receipt_raw != Digest32::ZERO).then_some(receipt_raw),
        revision, transition_number, previous_digest, record_digest:digest })
}

struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let out = self.0.get(..count).ok_or_else(||corrupt("truncated control effect record"))?;
        self.0 = &self.0[count..]; Ok(out)
    }
    fn byte(&mut self) -> Result<u8> { Ok(self.take(1)?[0]) }
    fn boolean(&mut self) -> Result<bool> { match self.byte()? { 0 => Ok(false), 1 => Ok(true), _ => Err(corrupt("noncanonical control Boolean")) } }
    fn u64(&mut self) -> Result<u64> { Ok(u64::from_be_bytes(self.take(8)?.try_into().map_err(|_|corrupt("control u64"))?)) }
    fn digest(&mut self) -> Result<Digest32> { Ok(Digest32::from_bytes(self.take(32)?.try_into().map_err(|_|corrupt("control digest"))?)) }
    fn text(&mut self, maximum: usize) -> Result<String> {
        let n = u16::from_be_bytes(self.take(2)?.try_into().map_err(|_|corrupt("control text length"))?) as usize;
        if n == 0 || n > maximum { return Err(corrupt("control text exceeds its bound")); }
        let text = std::str::from_utf8(self.take(n)?).map_err(|_|corrupt("control text is not UTF-8"))?;
        if text.chars().any(char::is_control) { return Err(corrupt("control text contains a control character")); }
        Ok(text.to_owned())
    }
}

pub struct PrivateControlJournalFile {
    file: File,
    path: PathBuf,
    #[cfg(unix)]
    identity: (u64, u64, u32, u64, u64),
}
impl Read for PrivateControlJournalFile { fn read(&mut self, out:&mut [u8])->io::Result<usize>{self.file.read(out)} }
impl Write for PrivateControlJournalFile {
    fn write(&mut self, bytes:&[u8])->io::Result<usize>{self.file.write(bytes)}
    fn flush(&mut self)->io::Result<()>{self.file.flush()}
}
impl Seek for PrivateControlJournalFile { fn seek(&mut self, from:SeekFrom)->io::Result<u64>{self.file.seek(from)} }
impl EffectJournalStorage for PrivateControlJournalFile {
    fn sync(&mut self)->io::Result<()>{self.validate_identity()?;self.file.sync_all()}
    fn truncate(&mut self,length:u64)->io::Result<()>{self.validate_identity()?;self.file.set_len(length)}
    fn validate_identity(&self)->io::Result<()> {
        #[cfg(unix)] {
            use std::os::unix::fs::MetadataExt;
            let denied=||io::Error::new(io::ErrorKind::PermissionDenied,"control journal custody changed");
            let parent=self.path.parent().ok_or_else(denied)?;
            if parent.canonicalize()?!=parent{return Err(denied());}
            let dir=fs::symlink_metadata(parent)?;let named=fs::symlink_metadata(&self.path)?;let opened=self.file.metadata()?;
            if !dir.is_dir()||dir.mode()&0o7777!=0o700||!named.is_file()||named.mode()&0o7777!=0o600||named.nlink()!=1
                ||named.uid()!=dir.uid()||(opened.dev(),opened.ino(),opened.uid(),dir.dev(),dir.ino())!=self.identity
                ||(named.dev(),named.ino())!=(opened.dev(),opened.ino()){return Err(denied());}
            Ok(())
        }
        #[cfg(not(unix))] { Err(io::Error::new(io::ErrorKind::Unsupported,"private control journals require Unix custody checks")) }
    }
}

pub fn open_private_control_journal(path:&Path,context:&OperationContext,bridge_generation:u64,
    recovery:EffectTailRecovery)->Result<ControlEffectJournal<PrivateControlJournalFile>> {
    authorize(context)?;
    #[cfg(unix)] {
        use std::os::unix::fs::{MetadataExt,OpenOptionsExt};
        let denied=||DfmcpError::new(ErrorCode::CapabilityDenied,
            "control journal requires an absolute normalized path in a private 0700 directory and a single-link 0600 regular file");
        if !path.is_absolute()||path.as_os_str().len()>4096||path.components().any(|c|!matches!(c,Component::RootDir|Component::Normal(_)))
            ||path.file_name().is_none(){return Err(denied());}
        let parent=path.parent().ok_or_else(denied)?;
        if parent.canonicalize().map_err(storage_error)?!=parent{return Err(denied());}
        let dir=fs::symlink_metadata(parent).map_err(storage_error)?;
        if !dir.is_dir()||dir.mode()&0o7777!=0o700{return Err(denied());}
        let before=match fs::symlink_metadata(path){
            Ok(meta)=>{if !meta.is_file()||meta.mode()&0o7777!=0o600||meta.nlink()!=1||meta.uid()!=dir.uid(){return Err(denied());}Some(meta)},
            Err(error) if error.kind()==io::ErrorKind::NotFound=>None,
            Err(error)=>return Err(storage_error(error)),
        };
        let created=before.is_none();let mut options=OpenOptions::new();options.read(true).write(true);
        if created{options.create_new(true).mode(0o600);}
        let file=options.open(path).map_err(storage_error)?;
        file.try_lock().map_err(|_|conflict("control effect journal already has a writer or cannot be exclusively locked"))?;
        let opened=file.metadata().map_err(storage_error)?;
        if before.as_ref().is_some_and(|meta|(meta.dev(),meta.ino())!=(opened.dev(),opened.ino())){return Err(denied());}
        let storage=PrivateControlJournalFile{identity:(opened.dev(),opened.ino(),opened.uid(),dir.dev(),dir.ino()),file,path:path.to_owned()};
        storage.validate_identity().map_err(storage_error)?;
        if created{File::open(parent).and_then(|directory|directory.sync_all()).map_err(storage_error)?;}
        ControlEffectJournal::open(storage,context,created,bridge_generation,recovery)
    }
    #[cfg(not(unix))] {
        let _=(path,bridge_generation,recovery);
        Err(DfmcpError::new(ErrorCode::CapabilityDenied,"private control effect journals are currently Unix-only"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use dfmcp_core::{CapabilityGrant,CapabilityScope,FortressId,GameTick,ObservationCursor,RequestId,SessionId,StateAnchor,WorkBudget};

    #[derive(Default)]
    struct Memory { bytes:Cursor<Vec<u8>>, sync_fails:bool }
    impl Read for Memory { fn read(&mut self,out:&mut [u8])->io::Result<usize>{self.bytes.read(out)} }
    impl Seek for Memory { fn seek(&mut self,from:SeekFrom)->io::Result<u64>{self.bytes.seek(from)} }
    impl Write for Memory { fn write(&mut self,bytes:&[u8])->io::Result<usize>{self.bytes.write(bytes)} fn flush(&mut self)->io::Result<()>{Ok(())} }
    impl EffectJournalStorage for Memory {
        fn sync(&mut self)->io::Result<()>{if self.sync_fails{Err(io::Error::other("injected sync"))}else{Ok(())}}
        fn truncate(&mut self,length:u64)->io::Result<()>{self.bytes.get_mut().truncate(length as usize);Ok(())}
    }
    fn context()->OperationContext{
        OperationContext{session_id:SessionId::new(1),request_id:RequestId::new(2),anchor:StateAnchor{fortress_id:FortressId::new(1),
            cursor:ObservationCursor::ORIGIN,tick:GameTick(10),state_hash:Digest32::ZERO},budget:WorkBudget::default(),grants:vec![CapabilityGrant{
            capability:Capability::ControlClock,scope:CapabilityScope::default(),max_risk:RiskTier::Reversible,expires_at_tick:None,remaining_uses:None}],cancellation_requested:false}
    }
    fn fresh()->Result<ControlEffectJournal<Memory>>{ControlEffectJournal::open(Memory::default(),&context(),true,7,EffectTailRecovery::Refuse)}
    #[test]
    fn prepare_commit_and_verified_replay_are_exact()->Result<()> {
        let mut j=fresh()?;let digest=Digest32::of_bytes(b"plan");let token=[3u8;16];
        j.record_prepared("k".to_owned(),digest,true,10,7,token,&context())?;
        j.begin_commit("k",digest,7,&context())?;
        let receipt=Digest32::of_bytes(b"receipt");
        j.record_reconciliation("k",digest,7,true,true,true,11,Some(receipt),&context())?;
        let expected=j.lookup("k").cloned().ok_or_else(||corrupt("test record missing"))?;
        let bytes=j.storage.bytes.into_inner();let reopened=ControlEffectJournal::open(Memory{bytes:Cursor::new(bytes),..Memory::default()},&context(),false,7,EffectTailRecovery::Refuse)?;
        assert_eq!(reopened.lookup("k"),Some(&expected));assert_eq!(expected.state,DurablePauseState::VerifiedApplied);Ok(())
    }
    #[test]
    fn commit_attempt_is_durable_before_effect_and_never_retryable_as_prepared()->Result<()> {
        let mut j=fresh()?;let digest=Digest32::of_bytes(b"plan");j.record_prepared("k".to_owned(),digest,false,10,7,[1u8;16],&context())?;
        j.begin_commit("k",digest,7,&context())?;assert!(matches!(j.begin_commit("k",digest,7,&context()),Err(e) if e.code==ErrorCode::EffectIndeterminate));
        j.mark_indeterminate("k",digest,&context())?;assert_eq!(j.lookup("k").map(|v|v.state),Some(DurablePauseState::Indeterminate));Ok(())
    }
    #[test]
    fn generation_change_or_unknown_reconciliation_stays_indeterminate()->Result<()> {
        let mut j=fresh()?;let digest=Digest32::of_bytes(b"plan");j.record_prepared("k".to_owned(),digest,true,10,7,[1u8;16],&context())?;
        j.begin_commit("k",digest,7,&context())?;j.record_reconciliation("k",digest,8,false,false,false,0,None,&context())?;
        let record=j.lookup("k").ok_or_else(||corrupt("test record missing"))?;assert_eq!(record.state,DurablePauseState::Indeterminate);assert!(!record.safe_to_dispatch(8));Ok(())
    }
    #[test]
    fn conflicting_key_content_is_rejected()->Result<()> {
        let mut j=fresh()?;let digest=Digest32::of_bytes(b"plan");j.record_prepared("k".to_owned(),digest,true,10,7,[1u8;16],&context())?;
        assert!(matches!(j.record_prepared("k".to_owned(),Digest32::of_bytes(b"other"),true,10,7,[1u8;16],&context()),Err(e) if e.code==ErrorCode::Conflict));Ok(())
    }
    #[test]
    fn failed_commit_started_sync_fences_without_dispatchable_state()->Result<()> {
        let mut j=fresh()?;let digest=Digest32::of_bytes(b"plan");j.record_prepared("k".to_owned(),digest,true,10,7,[1u8;16],&context())?;
        j.storage.sync_fails=true;assert!(j.begin_commit("k",digest,7,&context()).is_err());assert!(j.fenced());
        assert_eq!(j.lookup("k").map(|v|v.state),Some(DurablePauseState::Prepared));Ok(())
    }
    #[test]
    fn incomplete_tail_requires_explicit_repair_and_preserves_last_complete_state()->Result<()> {
        let mut j=fresh()?;let digest=Digest32::of_bytes(b"plan");j.record_prepared("k".to_owned(),digest,true,10,7,[1u8;16],&context())?;
        let length=j.length;let mut bytes=j.storage.bytes.into_inner();bytes.extend_from_slice(&RECORD[..3]);
        assert!(ControlEffectJournal::open(Memory{bytes:Cursor::new(bytes.clone()),..Memory::default()},&context(),false,7,EffectTailRecovery::Refuse).is_err());
        let repaired=ControlEffectJournal::open(Memory{bytes:Cursor::new(bytes),..Memory::default()},&context(),false,7,EffectTailRecovery::TruncateIncomplete)?;
        assert_eq!(repaired.retained_bytes(),length);assert_eq!(repaired.repaired_tail_bytes(),3);assert_eq!(repaired.lookup("k").map(|v|v.state),Some(DurablePauseState::Prepared));Ok(())
    }
    #[test]
    fn replay_rejects_validly_hashed_but_wrong_predecessor_chain()->Result<()> {
        let mut j=fresh()?;let digest=Digest32::of_bytes(b"plan");let prepared=j.record_prepared("k".to_owned(),digest,true,10,7,[1u8;16],&context())?;
        let mut next=prepared.clone();next.state=DurablePauseState::CommitStarted;next.revision=2;next.transition_number=2;next.previous_digest=Digest32::of_bytes(b"wrong");
        let forged=encode_frame(j.id,&next)?;j.storage.bytes.get_mut().extend_from_slice(&forged);
        let bytes=j.storage.bytes.into_inner();assert!(ControlEffectJournal::open(Memory{bytes:Cursor::new(bytes),..Memory::default()},&context(),false,7,EffectTailRecovery::Refuse).is_err());Ok(())
    }
}
