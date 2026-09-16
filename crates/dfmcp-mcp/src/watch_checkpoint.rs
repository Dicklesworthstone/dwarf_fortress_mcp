//! Fixed-format watch checkpoints. This is not a game-state or effect journal.
//! The caller validates every recovered payload against its observation archive.

use std::io::{self, SeekFrom};
use std::time::Instant;
use dfmcp_adapter::operations_journal::JournalStorage;
use dfmcp_core::{Capability, DfmcpError, Digest32, ErrorCode, OperationContext, Result, RiskTier};

const MAGIC: &[u8; 8] = b"DFWLOG01";
const RECORD: &[u8; 8] = b"DFWREC01";
const FOOTER: &[u8; 8] = b"DFWEND01";
const HEADER: usize = 104;
const PREFIX: usize = 84;
pub(super) const MAX_PAYLOAD: usize = 1024 * 1024;
const MAX_BYTES: u64 = 64 * 1024 * 1024;
const MAX_RECORDS: u64 = 4096;

fn corrupt(text: &str) -> DfmcpError { DfmcpError::new(ErrorCode::CorruptLedger, text) }
fn budget(text: &str) -> DfmcpError { DfmcpError::new(ErrorCode::BudgetExceeded, text) }
fn io_error(_: io::Error) -> DfmcpError { corrupt("watch journal I/O failed; reopen for verified recovery") }
fn check(context: &OperationContext, started: Instant) -> Result<()> {
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if started.elapsed().as_millis() >= u128::from(context.budget.max_wall_millis) {
        return Err(budget("watch journal work exceeded its wall-time budget"));
    }
    Ok(())
}
fn hash(domain: &[u8], parts: &[&[u8]]) -> Digest32 {
    let mut bytes = domain.to_vec();
    for part in parts { bytes.extend_from_slice(part); }
    Digest32::of_bytes(&bytes)
}
fn get_digest(bytes: &[u8]) -> Result<Digest32> {
    let value: [u8; 32] = bytes.try_into().map_err(|_| corrupt("watch journal digest length"))?;
    Ok(Digest32::from_bytes(value))
}
fn get_u64(bytes: &[u8]) -> Result<u64> {
    Ok(u64::from_be_bytes(bytes.try_into().map_err(|_| corrupt("watch journal integer length"))?))
}

pub(super) struct Pending {
    previous: Digest32,
    number: u64,
    head: Digest32,
    length: u64,
    frame: Vec<u8>,
    payload: Vec<u8>,
}
impl Pending {
    pub(super) fn head(&self) -> Digest32 { self.head }
    pub(super) fn number(&self) -> u64 { self.number }
    pub(super) fn retained_bytes(&self) -> u64 { self.length }
}

pub(super) struct Journal<S> {
    storage: S,
    id: Digest32,
    head: Digest32,
    count: u64,
    length: u64,
    latest: Vec<u8>,
    fenced: bool,
}
impl<S: JournalStorage> Journal<S> {
    /// Never initialize an existing empty file, repair a tail, or change a binding.
    pub(super) fn open<F>(mut storage: S, context: &OperationContext,
        binding: Digest32, created: bool, mut validate: F) -> Result<Self>
    where F: FnMut(&[u8]) -> Result<()> {
        let started = Instant::now();
        check(context, started)?;
        if binding == Digest32::ZERO { return Err(corrupt("watch journal has no observation binding")); }
        storage.validate_identity().map_err(io_error)?;
        let mut end = storage.seek(SeekFrom::End(0)).map_err(io_error)?;
        if end > MAX_BYTES { return Err(budget("watch journal exceeds 64 MiB")); }
        if created && end == 0 {
            let id = hash(b"dfmcp-watch-journal-incarnation/1\0", &[
                binding.as_bytes(), &context.session_id.get().to_be_bytes(),
                &context.request_id.get().to_be_bytes(), context.anchor.state_hash.as_bytes()]);
            let mut header = MAGIC.to_vec();
            header.extend_from_slice(binding.as_bytes());
            header.extend_from_slice(id.as_bytes());
            let digest = hash(b"dfmcp-watch-journal-header/1\0", &[&header]);
            header.extend_from_slice(digest.as_bytes());
            storage.seek(SeekFrom::Start(0)).map_err(io_error)?;
            storage.write_all(&header).map_err(io_error)?;
            storage.sync().map_err(io_error)?;
            end = HEADER as u64;
        }
        if end < HEADER as u64 { return Err(corrupt("watch journal header incomplete; bytes preserved")); }
        let mut header = [0; HEADER];
        storage.seek(SeekFrom::Start(0)).map_err(io_error)?;
        storage.read_exact(&mut header).map_err(io_error)?;
        let head = get_digest(&header[72..])?;
        if &header[..8] != MAGIC || get_digest(&header[8..40])? != binding
            || head != hash(b"dfmcp-watch-journal-header/1\0", &[&header[..72]]) {
            return Err(corrupt("watch journal format, observation binding or header checksum differs"));
        }
        let id = get_digest(&header[40..72])?;
        if id == Digest32::ZERO { return Err(corrupt("watch journal identity is zero")); }
        let mut journal = Self { storage, id, head, count: 0, length: HEADER as u64,
            latest: Vec::new(), fenced: false };
        while journal.length < end {
            check(context, started)?;
            if journal.count >= MAX_RECORDS { return Err(budget("watch journal exceeds 4096 checkpoints")); }
            if end - journal.length < PREFIX as u64 {
                return Err(corrupt("incomplete watch checkpoint prefix; no repair performed"));
            }
            let mut prefix = [0; PREFIX];
            journal.storage.read_exact(&mut prefix).map_err(io_error)?;
            let size = journal.validate_prefix(&prefix)?;
            let frame_bytes = PREFIX + size + 32 + FOOTER.len();
            if end - journal.length < frame_bytes as u64 {
                return Err(corrupt("incomplete watch checkpoint; no repair performed"));
            }
            let mut payload = vec![0; size];
            journal.storage.read_exact(&mut payload).map_err(io_error)?;
            let mut tail = [0; 40];
            journal.storage.read_exact(&mut tail).map_err(io_error)?;
            let digest = get_digest(&tail[..32])?;
            if &tail[32..] != FOOTER || digest != hash(b"dfmcp-watch-journal-record/1\0",
                &[journal.id.as_bytes(), &prefix, &payload]) {
                return Err(corrupt("watch checkpoint checksum or commit footer failed"));
            }
            validate(&payload)?;
            journal.latest = payload;
            journal.head = digest;
            journal.count += 1;
            journal.length += frame_bytes as u64;
        }
        check(context, started)?;
        journal.verify(context)?;
        Ok(journal)
    }

    fn validate_prefix(&self, prefix: &[u8; PREFIX]) -> Result<usize> {
        if &prefix[..8] != RECORD || get_u64(&prefix[8..16])? != self.count + 1
            || get_digest(&prefix[16..48])? != self.head
            || get_digest(&prefix[52..])? != hash(b"dfmcp-watch-journal-prefix/1\0",
                &[self.id.as_bytes(), &prefix[..52]]) {
            return Err(corrupt("watch checkpoint prefix, sequence or predecessor failed"));
        }
        let size = u32::from_be_bytes(prefix[48..52].try_into()
            .map_err(|_| corrupt("watch checkpoint length"))?) as usize;
        if !(1..=MAX_PAYLOAD).contains(&size) { return Err(budget("watch checkpoint exceeds 1 MiB")); }
        Ok(size)
    }

    pub(super) fn verify(&mut self, context: &OperationContext) -> Result<()> {
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if self.fenced { return Err(corrupt("watch journal is fenced; reopen for verified recovery")); }
        if self.storage.validate_identity().is_err()
            || !matches!(self.storage.seek(SeekFrom::End(0)), Ok(length) if length == self.length) {
            self.fenced = true;
            return Err(corrupt("watch journal identity or length changed"));
        }
        Ok(())
    }

    /// Compute the prospective receipt before rendering a response; no I/O here.
    pub(super) fn stage(&self, payload: Vec<u8>, context: &OperationContext) -> Result<Pending> {
        let started = Instant::now();
        check(context, started)?;
        if self.fenced { return Err(corrupt("watch journal is fenced")); }
        if payload.is_empty() || payload.len() > MAX_PAYLOAD || self.count >= MAX_RECORDS {
            return Err(budget("watch checkpoint or retention limit exceeded"));
        }
        let mut frame = RECORD.to_vec();
        frame.extend_from_slice(&(self.count + 1).to_be_bytes());
        frame.extend_from_slice(self.head.as_bytes());
        frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        let checksum = hash(b"dfmcp-watch-journal-prefix/1\0", &[self.id.as_bytes(), &frame]);
        frame.extend_from_slice(checksum.as_bytes());
        frame.extend_from_slice(&payload);
        let head = hash(b"dfmcp-watch-journal-record/1\0", &[self.id.as_bytes(), &frame]);
        frame.extend_from_slice(head.as_bytes());
        frame.extend_from_slice(FOOTER);
        let length = self.length.checked_add(frame.len() as u64)
            .ok_or_else(|| budget("watch journal length overflow"))?;
        if length > MAX_BYTES { return Err(budget("watch journal retention is full; no automatic pruning")); }
        check(context, started)?;
        Ok(Pending { previous: self.head, number: self.count + 1, head, length, frame, payload })
    }

    /// Sync before changing the in-memory root or acknowledging the new state.
    pub(super) fn commit(&mut self, pending: Pending, context: &OperationContext) -> Result<()> {
        self.verify(context)?;
        if pending.previous != self.head || pending.number != self.count + 1 {
            return Err(corrupt("watch checkpoint was staged against another journal head"));
        }
        let result = (|| -> io::Result<()> {
            self.storage.write_all(&pending.frame)?;
            self.storage.sync()?;
            self.storage.validate_identity()
        })();
        if result.is_err() {
            self.fenced = true;
            return Err(corrupt("watch checkpoint durability is uncertain; in-memory change not published"));
        }
        self.latest = pending.payload;
        self.head = pending.head;
        self.count = pending.number;
        self.length = pending.length;
        Ok(())
    }
    pub(super) fn latest(&self) -> &[u8] { &self.latest }
    pub(super) fn id(&self) -> Digest32 { self.id }
    pub(super) fn head(&self) -> Digest32 { self.head }
    pub(super) fn count(&self) -> u64 { self.count }
    pub(super) fn retained_bytes(&self) -> u64 { self.length }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read, Seek, Write};
    use dfmcp_core::{CapabilityGrant, CapabilityScope, FortressId, GameTick, ObservationCursor,
        RequestId, SessionId, StateAnchor, WorkBudget};
    #[derive(Default)]
    struct Memory { bytes: Cursor<Vec<u8>>, fail_sync: bool, write_limit: Option<usize> }
    impl Read for Memory { fn read(&mut self,b:&mut[u8])->io::Result<usize>{self.bytes.read(b)} }
    impl Seek for Memory { fn seek(&mut self,p:SeekFrom)->io::Result<u64>{self.bytes.seek(p)} }
    impl Write for Memory {
        fn write(&mut self,b:&[u8])->io::Result<usize>{
            if let Some(remaining)=self.write_limit {
                if remaining==0{return Err(io::Error::other("injected short write"));}
                let count=remaining.min(b.len());self.write_limit=Some(remaining-count);
                self.bytes.write(&b[..count])
            }else{self.bytes.write(b)}
        }
        fn flush(&mut self)->io::Result<()>{Ok(())}
    }
    impl JournalStorage for Memory {
        fn sync(&mut self)->io::Result<()>{if self.fail_sync{Err(io::Error::other("sync"))}else{Ok(())}}
        fn truncate(&mut self,_:u64)->io::Result<()>{Err(io::Error::other("repair forbidden"))}
    }
    fn context()->OperationContext{OperationContext{session_id:SessionId::new(7),request_id:RequestId::new(3),
        anchor:StateAnchor{fortress_id:FortressId::new(9),cursor:ObservationCursor::ORIGIN,
            tick:GameTick(10),state_hash:Digest32::of_bytes(b"world")},budget:WorkBudget{max_wall_millis:60000,..WorkBudget::default()},
        grants:vec![CapabilityGrant{capability:Capability::Query,scope:CapabilityScope::default(),
            max_risk:RiskTier::ReadOnly,expires_at_tick:None,remaining_uses:None}],cancellation_requested:false}}
    fn binding()->Digest32{Digest32::of_bytes(b"spatial/1.8 archive")}
    fn memory(bytes:Vec<u8>)->Memory{Memory{bytes:Cursor::new(bytes),..Memory::default()}}
    fn fresh()->Result<Journal<Memory>>{Journal::open(Memory::default(),&context(),binding(),true,|_|Ok(()))}
    #[test]
    fn synced_checkpoints_reopen_with_exact_head_and_payload()->Result<()>{
        let mut j=fresh()?;let first=j.stage(b"first".to_vec(),&context())?;j.commit(first,&context())?;
        let second=j.stage(b"second".to_vec(),&context())?;j.commit(second,&context())?;
        let head=j.head();let mut seen=Vec::new();let bytes=j.storage.bytes.into_inner();
        let reopened=Journal::open(memory(bytes),&context(),binding(),false,|p|{seen.push(p.to_vec());Ok(())})?;
        assert_eq!(seen,[b"first".to_vec(),b"second".to_vec()]);assert_eq!(reopened.latest(),b"second");
        assert_eq!(reopened.head(),head);assert_eq!(reopened.count(),2);Ok(())
    }
    #[test]
    fn every_incomplete_frame_prefix_is_refused_without_tail_repair()->Result<()>{
        let mut j=fresh()?;let p=j.stage(b"complete".to_vec(),&context())?;j.commit(p,&context())?;
        let bytes=j.storage.bytes.into_inner();
        for end in HEADER+1..bytes.len(){assert!(Journal::open(memory(bytes[..end].to_vec()),&context(),binding(),false,|_|Ok(())).is_err(),"cut {end}");}
        Ok(())
    }
    #[test]
    fn every_single_byte_corruption_is_refused()->Result<()>{
        let mut j=fresh()?;let p=j.stage(b"watch-state".to_vec(),&context())?;j.commit(p,&context())?;
        let bytes=j.storage.bytes.into_inner();
        for offset in 0..bytes.len(){let mut bad=bytes.clone();bad[offset]^=1;
            assert!(Journal::open(memory(bad),&context(),binding(),false,|_|Ok(())).is_err(),"byte {offset}");}Ok(())
    }
    #[test]
    fn uncertain_sync_fences_but_restart_can_recover_complete_frame()->Result<()>{
        let mut j=fresh()?;let old=j.head();let p=j.stage(b"new".to_vec(),&context())?;j.storage.fail_sync=true;
        assert!(j.commit(p,&context()).is_err());assert_eq!(j.head(),old);assert!(j.fenced);
        assert!(j.verify(&context()).is_err());
        let reopened=Journal::open(memory(j.storage.bytes.into_inner()),&context(),binding(),false,|_|Ok(()))?;
        assert_eq!(reopened.latest(),b"new");Ok(())
    }
    #[test]
    fn partial_write_never_publishes_candidate()->Result<()>{
        let mut j=fresh()?;let old=j.head();let p=j.stage(b"new".to_vec(),&context())?;j.storage.write_limit=Some(17);
        assert!(j.commit(p,&context()).is_err());assert_eq!(j.head(),old);assert!(j.fenced);
        assert!(Journal::open(memory(j.storage.bytes.into_inner()),&context(),binding(),false,|_|Ok(())).is_err());Ok(())
    }
    #[test]
    fn wrong_archive_empty_existing_file_and_duplicate_frames_fail()->Result<()>{
        assert!(Journal::open(Memory::default(),&context(),binding(),false,|_|Ok(())).is_err());
        let mut j=fresh()?;let p=j.stage(b"one".to_vec(),&context())?;j.commit(p,&context())?;
        let bytes=j.storage.bytes.into_inner();
        assert!(Journal::open(memory(bytes.clone()),&context(),Digest32::of_bytes(b"other"),false,|_|Ok(())).is_err());
        let mut duplicate=bytes.clone();duplicate.extend_from_slice(&bytes[HEADER..]);
        assert!(Journal::open(memory(duplicate),&context(),binding(),false,|_|Ok(())).is_err());Ok(())
    }
    #[test]
    fn invalid_payload_authority_and_budget_refuse_before_publication()->Result<()>{
        let mut j=fresh()?;assert!(j.stage(vec![0;MAX_PAYLOAD+1],&context()).is_err());
        let mut c=context();c.cancellation_requested=true;assert!(j.stage(b"x".to_vec(),&c).is_err());
        c.cancellation_requested=false;c.grants.clear();assert!(j.verify(&c).is_err());
        let p=j.stage(b"bad-state".to_vec(),&context())?;j.commit(p,&context())?;
        assert!(Journal::open(memory(j.storage.bytes.into_inner()),&context(),binding(),false,|_|Err(corrupt("invalid semantic state"))).is_err());Ok(())
    }
}
