//! Durable intent/cancellation, with outcomes derived from the authoritative
//! observation archive. No sample checkpoint can lag or lose an appended capture.
use super::super::Reader;
use super::{
    MAX_WATCH_KEY, MAX_WATCHES, WatchDefinition, WatchEvaluation, WatchGoal, WatchRecordRef,
    WatchSpec, error,
};
use crate::operations_journal::JournalStorage;
use dfmcp_core::{
    Capability, Digest32, ErrorCode, FortressId, GameTick, OperationContext, Result, RiskTier,
};
use std::collections::BTreeMap;
use std::io::{self, SeekFrom};
use std::time::{Duration, Instant};

#[path = "watch_file.rs"]
mod file;
use super::super::archive::{
    ArchiveMode, MAX_ARCHIVE_RECORDS, MAX_FRAME_BYTES, ProgressArchive, ProgressArchiveSummary,
};
pub use file::{PrivateWatchFile, open_watch_book};

pub const MAX_BOOK_BYTES: u64 = 128 * 1024;
pub const MAX_BOOK_EVENTS: usize = 2 * MAX_WATCHES;
pub const BOOK_OPEN_RESERVE: u64 = MAX_BOOK_BYTES
    + MAX_BOOK_EVENTS as u64 * MAX_FRAME_BYTES
    + MAX_ARCHIVE_RECORDS as u64 * (MAX_FRAME_BYTES + 512);
const MAGIC: &[u8; 8] = b"DFMPWB01";
const FRAME: &[u8; 8] = b"DFMPWR01";
const FOOTER: &[u8; 8] = b"DFMPWE01";
const HEADER_BYTES: u64 = 112;
const PREFIX: usize = 52;
const MAX_BODY: usize = 512;
const MAX_EVENT: u64 = (PREFIX + MAX_BODY + 40) as u64;
fn corrupt(text: &str) -> dfmcp_core::DfmcpError {
    error(ErrorCode::CorruptLedger, text)
}
fn exhausted() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "progress watch book exceeds its work or retention bound",
    )
}
fn storage_error(_: io::Error) -> dfmcp_core::DfmcpError {
    corrupt("progress watch custody or I/O failed; reopen without erasing intent")
}

struct Budget {
    until: Instant,
    bytes: u64,
}
impl Budget {
    fn new(c: &OperationContext) -> Result<Self> {
        c.budget.validate()?;
        Ok(Self {
            until: Instant::now()
                .checked_add(Duration::from_millis(c.budget.max_wall_millis.min(60_000)))
                .ok_or_else(exhausted)?,
            bytes: c.budget.max_bytes,
        })
    }
    fn context(&self, c: &OperationContext) -> Result<OperationContext> {
        let mut work = c.clone();
        work.budget.max_wall_millis = self
            .until
            .checked_duration_since(Instant::now())
            .and_then(|d| u64::try_from(d.as_millis()).ok())
            .filter(|n| *n > 0)
            .ok_or_else(exhausted)?;
        work.budget.max_bytes = self.bytes;
        work.budget.validate()?;
        Ok(work)
    }
    fn charge(&mut self, bytes: u64) -> Result<()> {
        if Instant::now() >= self.until {
            return Err(exhausted());
        }
        self.bytes = self.bytes.checked_sub(bytes).ok_or_else(exhausted)?;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetainedWatch {
    pub definition: WatchDefinition,
    pub cancelled_at: Option<WatchRecordRef>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchBookSummary {
    pub book_id: Digest32,
    pub head: Digest32,
    pub archive_id: Digest32,
    pub retained_bytes: u64,
    pub events: usize,
    pub read_only: bool,
    pub definitions: Vec<(String, Digest32)>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchBatch {
    pub archive_id: Digest32,
    pub archive_head: Digest32,
    pub archive_records: usize,
    pub book_head: Digest32,
    pub results: Vec<WatchEvaluation>,
}

pub struct WatchBook<S> {
    storage: S,
    fortress: FortressId,
    archive_id: Digest32,
    id: Digest32,
    head: Digest32,
    length: u64,
    events: usize,
    records: BTreeMap<String, RetainedWatch>,
    event_frontier: Option<WatchRecordRef>,
    mode: ArchiveMode,
    fenced: bool,
}
impl<S: JournalStorage> WatchBook<S> {
    pub fn open<A: JournalStorage>(
        mut storage: S,
        mode: ArchiveMode,
        initialize: bool,
        archive: &mut ProgressArchive<A>,
        c: &OperationContext,
    ) -> Result<Self> {
        let mut budget = Budget::new(c)?;
        let a = archive.summary(&budget.context(c)?)?;
        authorize(mode, &a, c, initialize || mode == ArchiveMode::Live)?;
        if initialize && mode == ArchiveMode::Offline {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "offline watches cannot initialize bytes",
            ));
        }
        storage.validate_identity().map_err(storage_error)?;
        let mut length = storage.seek(SeekFrom::End(0)).map_err(storage_error)?;
        if length > MAX_BOOK_BYTES {
            return Err(exhausted());
        }
        budget.charge(length.max(HEADER_BYTES))?;
        if length == 0 && initialize {
            let mut seed = b"dfmcp-progress-watch-book-id/1\0".to_vec();
            seed.extend_from_slice(a.archive_id.as_bytes());
            seed.extend_from_slice(&c.session_id.get().to_be_bytes());
            seed.extend_from_slice(&c.request_id.get().to_be_bytes());
            let mut header = MAGIC.to_vec();
            header.extend_from_slice(&a.fortress_id.get().to_be_bytes());
            header.extend_from_slice(a.archive_id.as_bytes());
            header.extend_from_slice(Digest32::of_bytes(&seed).as_bytes());
            let hash = header_hash(&header);
            header.extend_from_slice(hash.as_bytes());
            budget.charge(HEADER_BYTES)?;
            storage.seek(SeekFrom::Start(0)).map_err(storage_error)?;
            storage.write_all(&header).map_err(storage_error)?;
            storage.sync().map_err(storage_error)?;
            length = HEADER_BYTES;
        }
        if length < HEADER_BYTES {
            return Err(corrupt("incomplete watch book header; bytes unchanged"));
        }
        storage.seek(SeekFrom::Start(0)).map_err(storage_error)?;
        let mut header = [0; HEADER_BYTES as usize];
        storage.read_exact(&mut header).map_err(storage_error)?;
        let mut r = Reader { bytes: &header };
        if r.take(8)? != MAGIC {
            return Err(corrupt("not a progress watch book"));
        }
        let fortress = FortressId::new(r.u64()?);
        let archive_id = Digest32::from_bytes(r.array()?);
        let id = Digest32::from_bytes(r.array()?);
        let head = Digest32::from_bytes(r.array()?);
        if fortress != a.fortress_id
            || archive_id != a.archive_id
            || head != header_hash(&header[..80])
        {
            return Err(corrupt(
                "watch book header or paired archive identity differs",
            ));
        }
        let mut out = Self {
            storage,
            fortress,
            archive_id,
            id,
            head,
            length: HEADER_BYTES,
            events: 0,
            records: BTreeMap::new(),
            event_frontier: None,
            mode,
            fenced: false,
        };
        while out.length < length {
            budget.context(c)?;
            if out.events >= MAX_BOOK_EVENTS || length - out.length < PREFIX as u64 + 40 {
                return Err(corrupt("incomplete or excessive watch book events"));
            }
            out.storage
                .seek(SeekFrom::Start(out.length))
                .map_err(storage_error)?;
            let mut prefix = [0; PREFIX];
            out.storage.read_exact(&mut prefix).map_err(storage_error)?;
            let mut r = Reader { bytes: &prefix };
            if r.take(8)? != FRAME {
                return Err(corrupt("invalid watch frame"));
            }
            let size = r.u32()? as usize;
            let number = r.u64()?;
            let previous = Digest32::from_bytes(r.array()?);
            if size > MAX_BODY
                || number != out.events as u64 + 1
                || previous != out.head
                || (PREFIX + size + 40) as u64 > length - out.length
            {
                return Err(corrupt("watch frame order, size or predecessor failed"));
            }
            let mut body = vec![0; size];
            out.storage.read_exact(&mut body).map_err(storage_error)?;
            let mut trailer = [0; 40];
            out.storage
                .read_exact(&mut trailer)
                .map_err(storage_error)?;
            let mut r = Reader { bytes: &trailer };
            let digest = Digest32::from_bytes(r.array()?);
            if r.take(8)? != FOOTER || digest != frame_hash(out.id, &prefix, &body) {
                return Err(corrupt("watch checksum or footer failed"));
            }
            let event = decode_event(&body)?;
            out.accept(event, archive, &mut budget, c)?;
            out.events += 1;
            out.head = digest;
            out.length += (PREFIX + size + 40) as u64;
        }
        // Rebuild every outcome before accepting cancellation histories. A rehashed
        // cancellation after satisfaction is still corrupt, not a valid retirement.
        out.evaluate_inner(archive, &mut budget, c)?;
        out.access(archive, &budget.context(c)?)?;
        Ok(out)
    }
    fn access<A: JournalStorage>(
        &mut self,
        archive: &mut ProgressArchive<A>,
        c: &OperationContext,
    ) -> Result<ProgressArchiveSummary> {
        let a = archive.summary(c)?;
        if a.fortress_id != self.fortress || a.archive_id != self.archive_id {
            return Err(corrupt("watch book is paired with another archive"));
        }
        authorize(self.mode, &a, c, false)?;
        if self.fenced {
            return Err(corrupt(
                "watch book fenced; reopen without replacing missing evidence",
            ));
        }
        let check = (|| {
            self.storage.validate_identity().map_err(storage_error)?;
            if self.storage.seek(SeekFrom::End(0)).map_err(storage_error)? != self.length {
                return Err(corrupt("watch book extent changed outside its owner"));
            }
            Ok(())
        })();
        if check.is_err() {
            self.fenced = true;
        }
        check?;
        Ok(a)
    }
    pub fn verify_access<A: JournalStorage>(
        &mut self,
        archive: &mut ProgressArchive<A>,
        c: &OperationContext,
    ) -> Result<()> {
        self.access(archive, c).map(|_| ())
    }
    pub fn summary<A: JournalStorage>(
        &mut self,
        archive: &mut ProgressArchive<A>,
        c: &OperationContext,
    ) -> Result<WatchBookSummary> {
        let mut budget = Budget::new(c)?;
        self.access(archive, &budget.context(c)?)?;
        if self.records.len() > c.budget.max_entities as usize {
            return Err(exhausted());
        }
        budget.charge(512 * self.records.len() as u64)?;
        Ok(WatchBookSummary {
            book_id: self.id,
            head: self.head,
            archive_id: self.archive_id,
            retained_bytes: self.length,
            events: self.events,
            read_only: self.mode == ArchiveMode::Offline,
            definitions: self
                .records
                .values()
                .map(|r| (r.definition.spec().key().to_owned(), r.definition.digest()))
                .collect(),
        })
    }
    pub fn definition<A: JournalStorage>(
        &mut self,
        key: &str,
        digest: Digest32,
        archive: &mut ProgressArchive<A>,
        c: &OperationContext,
    ) -> Result<RetainedWatch> {
        self.access(archive, c)?;
        let value = self
            .records
            .get(key)
            .ok_or_else(|| error(ErrorCode::InvalidRequest, "unknown retained watch key"))?;
        if value.definition.digest() != digest {
            return Err(error(ErrorCode::Conflict, "watch key and digest disagree"));
        }
        Ok(value.clone())
    }
    pub fn register<A: JournalStorage>(
        &mut self,
        spec: WatchSpec,
        origin: WatchRecordRef,
        archive: &mut ProgressArchive<A>,
        c: &OperationContext,
    ) -> Result<RetainedWatch> {
        let mut budget = Budget::new(c)?;
        let a = self.access(archive, c)?;
        authorize(self.mode, &a, c, true)?;
        if let Some(old) = self.records.get(spec.key()) {
            if old.definition.spec() != &spec || old.definition.origin() != origin {
                return Err(error(
                    ErrorCode::Conflict,
                    "watch key already seals another intent or origin",
                ));
            }
            return Ok(old.clone()); // Never renew deadline, origin or cancellation.
        }
        if self.records.len() >= MAX_WATCHES || self.records.len() >= c.budget.max_entities as usize
        {
            return Err(exhausted());
        }
        if origin.number != a.records as u64 || origin.digest != a.head {
            return Err(error(
                ErrorCode::StaleAnchor,
                "new watches require the latest exact archive record",
            ));
        }
        let record = read_record(archive, origin, &mut budget, c)?;
        let definition = WatchDefinition::new(spec, self.archive_id, &record)?;
        if definition.spec().deadline() - definition.registered_tick() > c.budget.max_game_ticks {
            return Err(exhausted());
        }
        self.append(
            Event::Register(definition.spec().clone(), origin, definition.digest()),
            archive,
            &mut budget,
            c,
        )?;
        self.records
            .get(definition.spec().key())
            .cloned()
            .ok_or_else(|| corrupt("watch publication missing"))
    }
    pub fn cancel<A: JournalStorage>(
        &mut self,
        key: &str,
        digest: Digest32,
        expected_archive_head: Digest32,
        archive: &mut ProgressArchive<A>,
        c: &OperationContext,
    ) -> Result<RetainedWatch> {
        let mut budget = Budget::new(c)?;
        let a = self.access(archive, c)?;
        authorize(self.mode, &a, c, true)?;
        let old = self.definition(key, digest, archive, &budget.context(c)?)?;
        if old.cancelled_at.is_some() {
            return Ok(old);
        }
        if a.head != expected_archive_head {
            return Err(error(
                ErrorCode::StaleAnchor,
                "archive changed before watch cancellation",
            ));
        }
        let batch = self.evaluate_inner(archive, &mut budget, c)?;
        let mut evaluation = batch
            .results
            .into_iter()
            .find(|r| r.definition().digest() == digest)
            .ok_or_else(|| corrupt("retained watch was omitted during evaluation"))?;
        let at = WatchRecordRef {
            number: a.records as u64,
            digest: a.head,
        };
        evaluation.cancel_at(at)?;
        self.append(
            Event::Cancel(key.to_owned(), digest, at),
            archive,
            &mut budget,
            c,
        )?;
        self.records
            .get(key)
            .cloned()
            .ok_or_else(|| corrupt("cancelled watch publication missing"))
    }
    pub fn evaluate<A: JournalStorage>(
        &mut self,
        archive: &mut ProgressArchive<A>,
        c: &OperationContext,
    ) -> Result<WatchBatch> {
        self.evaluate_inner(archive, &mut Budget::new(c)?, c)
    }
    fn evaluate_inner<A: JournalStorage>(
        &mut self,
        archive: &mut ProgressArchive<A>,
        budget: &mut Budget,
        c: &OperationContext,
    ) -> Result<WatchBatch> {
        let a = self.access(archive, &budget.context(c)?)?;
        if self.records.len() > c.budget.max_entities as usize {
            return Err(exhausted());
        }
        let mut evaluations: BTreeMap<String, WatchEvaluation> = BTreeMap::new();
        let start = self
            .records
            .values()
            .map(|r| r.definition.origin().number)
            .min();
        if let Some(first) = start {
            let mut after = first - 1;
            while after < a.records as u64 {
                let work = budget.context(c)?;
                budget.charge(64 * 512)?;
                let page = archive.page(a.head, after, 64, &work)?;
                if page.entries.is_empty() {
                    return Err(corrupt("watch replay made no archive progress"));
                }
                for entry in page.entries {
                    let record = read_record(
                        archive,
                        WatchRecordRef {
                            number: entry.number,
                            digest: entry.record_digest,
                        },
                        budget,
                        c,
                    )?;
                    for (key, retained) in &self.records {
                        let origin = retained.definition.origin();
                        if origin.number == entry.number {
                            let checked = WatchDefinition::new(
                                retained.definition.spec().clone(),
                                self.archive_id,
                                &record,
                            )?;
                            if checked != retained.definition {
                                return Err(corrupt("watch origin changed after registration"));
                            }
                            evaluations.insert(key.clone(), WatchEvaluation::new(checked));
                        } else if origin.number < entry.number {
                            evaluations
                                .get_mut(key)
                                .ok_or_else(|| corrupt("watch origin was skipped"))?
                                .advance(&record)?;
                        }
                        if retained
                            .cancelled_at
                            .is_some_and(|at| at.number == entry.number)
                        {
                            let at = retained
                                .cancelled_at
                                .ok_or_else(|| corrupt("watch cancellation missing"))?;
                            if at != WatchRecordRef::of(&record) {
                                return Err(corrupt(
                                    "watch cancellation names another archive frame",
                                ));
                            }
                            evaluations
                                .get_mut(key)
                                .ok_or_else(|| corrupt("cancellation precedes registration"))?
                                .cancel_at(at)
                                .map_err(|_| {
                                    corrupt(
                                        "watch history cancels a terminal or unevaluated predicate",
                                    )
                                })?;
                        }
                    }
                    after = entry.number;
                }
            }
        }
        if evaluations.len() != self.records.len() {
            return Err(corrupt("watch origin is not retained in its archive"));
        }
        let current = self.access(archive, &budget.context(c)?)?;
        if current.head != a.head {
            return Err(error(
                ErrorCode::StaleAnchor,
                "archive changed during watch evaluation",
            ));
        }
        Ok(WatchBatch {
            archive_id: self.archive_id,
            archive_head: a.head,
            archive_records: a.records,
            book_head: self.head,
            results: evaluations.into_values().collect(),
        })
    }
    fn accept<A: JournalStorage>(
        &mut self,
        event: Event,
        archive: &mut ProgressArchive<A>,
        budget: &mut Budget,
        c: &OperationContext,
    ) -> Result<()> {
        let at = event.reference();
        if at.number == 0
            || self.event_frontier.is_some_and(|old| {
                at.number < old.number || (at.number == old.number && at.digest != old.digest)
            })
        {
            return Err(corrupt("watch events regress or fork the archive frontier"));
        }
        let record = read_record(archive, at, budget, c)?;
        match event {
            Event::Register(spec, _, digest) => {
                if self.records.contains_key(spec.key()) || self.records.len() >= MAX_WATCHES {
                    return Err(corrupt("duplicate or excessive watch registrations"));
                }
                if self.records.len() >= c.budget.max_entities as usize {
                    return Err(exhausted());
                }
                let definition = WatchDefinition::new(spec, self.archive_id, &record)?;
                if definition.digest() != digest {
                    return Err(corrupt("watch definition digest mismatch"));
                }
                self.records.insert(
                    definition.spec().key().to_owned(),
                    RetainedWatch {
                        definition,
                        cancelled_at: None,
                    },
                );
            }
            Event::Cancel(key, digest, _) => {
                let old = self
                    .records
                    .get_mut(&key)
                    .ok_or_else(|| corrupt("cancellation has no registration"))?;
                if old.definition.digest() != digest
                    || old.cancelled_at.is_some()
                    || at.number < old.definition.origin().number
                {
                    return Err(corrupt(
                        "watch cancellation changes identity or repeats retirement",
                    ));
                }
                old.cancelled_at = Some(at);
            }
        }
        self.event_frontier = Some(at);
        Ok(())
    }
    fn append<A: JournalStorage>(
        &mut self,
        event: Event,
        archive: &mut ProgressArchive<A>,
        budget: &mut Budget,
        c: &OperationContext,
    ) -> Result<()> {
        let a = self.access(archive, &budget.context(c)?)?;
        authorize(self.mode, &a, c, true)?;
        if self.events >= MAX_BOOK_EVENTS || self.length + MAX_EVENT > MAX_BOOK_BYTES {
            return Err(exhausted());
        }
        let body = encode_event(&event);
        if body.len() > MAX_BODY {
            return Err(exhausted());
        }
        // Validate and stage in-memory state BEFORE storage. Restore it until sync
        // has succeeded, so a partial/failed append cannot publish a new root.
        let previous = self.records.clone();
        let frontier = self.event_frontier;
        if let Err(cause) = self.accept(event, archive, budget, c) {
            self.records = previous;
            self.event_frontier = frontier;
            return Err(cause);
        }
        let candidate = std::mem::replace(&mut self.records, previous);
        let candidate_frontier = self.event_frontier;
        self.event_frontier = frontier;
        let mut prefix = FRAME.to_vec();
        prefix.extend_from_slice(&(body.len() as u32).to_be_bytes());
        prefix.extend_from_slice(&(self.events as u64 + 1).to_be_bytes());
        prefix.extend_from_slice(self.head.as_bytes());
        let digest = frame_hash(self.id, &prefix, &body);
        let mut bytes = prefix;
        bytes.extend_from_slice(&body);
        bytes.extend_from_slice(digest.as_bytes());
        bytes.extend_from_slice(FOOTER);
        budget.charge(bytes.len() as u64)?;
        let result = (|| {
            let a = self.access(archive, &budget.context(c)?)?;
            authorize(self.mode, &a, c, true)?;
            self.storage
                .seek(SeekFrom::Start(self.length))
                .map_err(storage_error)?;
            self.storage.write_all(&bytes).map_err(storage_error)?;
            self.storage.sync().map_err(storage_error)?;
            self.storage.validate_identity().map_err(storage_error)?;
            if self.storage.seek(SeekFrom::End(0)).map_err(storage_error)?
                != self.length + bytes.len() as u64
            {
                return Err(corrupt("watch write extent mismatch"));
            }
            budget.context(c)?;
            Ok(())
        })();
        if result.is_err() {
            self.fenced = true;
        }
        result?;
        self.records = candidate;
        self.event_frontier = candidate_frontier;
        self.length += bytes.len() as u64;
        self.events += 1;
        self.head = digest;
        Ok(())
    }
}
fn authorize(
    mode: ArchiveMode,
    a: &ProgressArchiveSummary,
    c: &OperationContext,
    write: bool,
) -> Result<()> {
    let mut current = c.clone();
    current.anchor.tick = GameTick(c.anchor.tick.get().max(a.authority_tick_floor));
    if current.anchor.fortress_id != a.fortress_id {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "watch authority names another fortress",
        ));
    }
    current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if write {
        if mode != ArchiveMode::Live || a.read_only {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "offline watch recovery cannot change intent even with an injected Observe grant",
            ));
        }
        current.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
    }
    Ok(())
}
fn read_record<A: JournalStorage>(
    archive: &mut ProgressArchive<A>,
    at: WatchRecordRef,
    budget: &mut Budget,
    c: &OperationContext,
) -> Result<super::super::archive::ArchivedProgress> {
    let work = budget.context(c)?;
    budget.charge(MAX_FRAME_BYTES)?;
    archive.record(at.number, at.digest, &work)
}
enum Event {
    Register(WatchSpec, WatchRecordRef, Digest32),
    Cancel(String, Digest32, WatchRecordRef),
}
impl Event {
    fn reference(&self) -> WatchRecordRef {
        match self {
            Self::Register(_, r, _) | Self::Cancel(_, _, r) => *r,
        }
    }
}
fn put_ref(out: &mut Vec<u8>, r: WatchRecordRef) {
    out.extend_from_slice(&r.number.to_be_bytes());
    out.extend_from_slice(r.digest.as_bytes());
}
fn get_ref(r: &mut Reader<'_>) -> Result<WatchRecordRef> {
    Ok(WatchRecordRef {
        number: r.u64()?,
        digest: Digest32::from_bytes(r.array()?),
    })
}
fn encode_event(event: &Event) -> Vec<u8> {
    let mut out = Vec::new();
    match event {
        Event::Register(spec, at, digest) => {
            out.push(1);
            spec.encode(&mut out);
            put_ref(&mut out, *at);
            out.extend_from_slice(digest.as_bytes());
        }
        Event::Cancel(key, digest, at) => {
            out.push(2);
            out.push(key.len() as u8);
            out.extend_from_slice(key.as_bytes());
            out.extend_from_slice(digest.as_bytes());
            put_ref(&mut out, *at);
        }
    }
    out
}
fn decode_event(bytes: &[u8]) -> Result<Event> {
    let mut r = Reader { bytes };
    let tag = r.byte()?;
    let size = usize::from(r.byte()?);
    if size == 0 || size > MAX_WATCH_KEY {
        return Err(corrupt("invalid watch key extent"));
    }
    let key = std::str::from_utf8(r.take(size)?)
        .map_err(|_| corrupt("watch key is not UTF-8"))?
        .to_owned();
    let event = match tag {
        1 => {
            let order = r.u32()?;
            let kind = r.byte()?;
            let threshold = r.u32()?;
            let goal = match (kind, threshold) {
                (1, 0) => WatchGoal::Validated,
                (2, 0) => WatchGoal::Active,
                (3, n) => WatchGoal::RemainingAtMost(n),
                _ => return Err(corrupt("noncanonical watch goal")),
            };
            let spec = WatchSpec::new(&key, order, goal, r.u64()?, r.u64()?, r.byte()?)?;
            let at = get_ref(&mut r)?;
            Event::Register(spec, at, Digest32::from_bytes(r.array()?))
        }
        2 => {
            let digest = Digest32::from_bytes(r.array()?);
            Event::Cancel(key, digest, get_ref(&mut r)?)
        }
        _ => return Err(corrupt("unknown watch book event")),
    };
    if !r.bytes.is_empty() {
        return Err(corrupt("trailing watch event bytes"));
    }
    Ok(event)
}
fn header_hash(bytes: &[u8]) -> Digest32 {
    let mut out = b"dfmcp-progress-watch-header/1\0".to_vec();
    out.extend_from_slice(bytes);
    Digest32::of_bytes(&out)
}
fn frame_hash(id: Digest32, prefix: &[u8], body: &[u8]) -> Digest32 {
    let mut out = b"dfmcp-progress-watch-frame/1\0".to_vec();
    out.extend_from_slice(id.as_bytes());
    out.extend_from_slice(prefix);
    out.extend_from_slice(body);
    Digest32::of_bytes(&out)
}

#[cfg(test)]
#[path = "watch_book_tests.rs"]
mod tests;
