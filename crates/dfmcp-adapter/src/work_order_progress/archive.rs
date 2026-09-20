//! Exact, append-before-publication history for progress/1.12. This is neither
//! a game save nor an effect journal. Reopening starts a new comparison segment.
use std::io::{self, SeekFrom};
use std::time::{Duration, Instant};

use dfmcp_core::{Capability, Digest32, ErrorCode, FortressId, GameTick, OperationContext, Result};
use crate::operations_journal::JournalStorage;
use super::{ProgressComparison, ProgressManifest, ProgressObservation, Reader, compare, error, validate_targets};

#[path = "archive_file.rs"]
mod file;
pub use file::{PrivateProgressArchiveFile, open_progress_archive};

pub const MAX_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_ARCHIVE_RECORDS: usize = 4096;
pub const MAX_FRAME_BYTES: u64 = 18 * 1024 + 92;
const MAGIC: &[u8; 8] = b"DFMWPA12";
const FRAME: &[u8; 8] = b"DFMWPR12";
const FOOTER: &[u8; 8] = b"DFMWPE12";
const HEADER_BYTES: u64 = 80;
const PREFIX_BYTES: usize = 52;
const MAX_BODY: usize = 18 * 1024;

fn corrupt(text: &str) -> dfmcp_core::DfmcpError { error(ErrorCode::CorruptLedger, text) }
fn exhausted() -> dfmcp_core::DfmcpError { error(ErrorCode::BudgetExceeded, "progress archive exceeds its explicit work or retention allowance") }
fn storage_error(_: io::Error) -> dfmcp_core::DfmcpError { corrupt("progress archive custody or I/O failed; reopen without repairing evidence") }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveMode { Live, Offline }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgressArchiveEntry {
    pub number: u64,
    pub segment: u64,
    pub record_digest: Digest32,
    pub previous_digest: Digest32,
    pub witness: Digest32,
    pub game_tick: u64,
    pub native_order_ids: Vec<u32>,
    offset: u64,
    body_bytes: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchivedProgress {
    pub entry: ProgressArchiveEntry,
    pub manifest: ProgressManifest,
    pub observation: ProgressObservation,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgressArchiveSummary {
    pub archive_id: Digest32,
    pub head: Digest32,
    pub fortress_id: FortressId,
    pub records: usize,
    pub segments: u64,
    pub retained_bytes: u64,
    pub authority_tick_floor: u64,
    pub read_only: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgressArchivePage {
    pub entries: Vec<ProgressArchiveEntry>,
    pub next_after: Option<u64>,
}

struct Budget { start: Instant, allowance: Duration, bytes: u64 }
impl Budget {
    fn new(c: &OperationContext) -> Result<Self> {
        c.budget.validate()?;
        Ok(Self { start: Instant::now(), allowance: Duration::from_millis(c.budget.max_wall_millis.min(60_000)), bytes: c.budget.max_bytes })
    }
    fn check(&self) -> Result<()> { if self.start.elapsed() >= self.allowance { Err(exhausted()) } else { Ok(()) } }
    fn charge(&mut self, n: u64) -> Result<()> {
        self.check()?; self.bytes = self.bytes.checked_sub(n).ok_or_else(exhausted)?; Ok(())
    }
}

/// Only the index and last complete capture are retained in memory. Exact-record
/// queries reread and verify bytes. A failed append/sync permanently fences this owner.
pub struct ProgressArchive<S> {
    storage: S,
    fortress: FortressId,
    id: Digest32,
    head: Digest32,
    length: u64,
    entries: Vec<ProgressArchiveEntry>,
    last: Option<ArchivedProgress>,
    tick_floor: u64,
    mode: ArchiveMode,
    new_segment: bool,
    fenced: bool,
}
impl<S: JournalStorage> ProgressArchive<S> {
    /// initialize_empty is only for an exclusively created new store. Existing
    /// empty/corrupt/incomplete files are refused, never silently initialized.
    pub fn open(mut storage: S, mode: ArchiveMode, initialize_empty: bool, c: &OperationContext) -> Result<Self> {
        let mut budget = Budget::new(c)?;
        c.authorize(Capability::Query, super::RiskTier::ReadOnly, &[], None)?;
        if c.anchor.fortress_id == FortressId::NIL { return Err(corrupt("progress archive requires a concrete fortress")); }
        if mode == ArchiveMode::Live { c.authorize(Capability::Observe, super::RiskTier::ReadOnly, &[], None)?; }
        if initialize_empty && mode == ArchiveMode::Offline { return Err(error(ErrorCode::CapabilityDenied, "offline archive cannot initialize bytes")); }
        storage.validate_identity().map_err(storage_error)?;
        let mut length = storage.seek(SeekFrom::End(0)).map_err(storage_error)?;
        if length > MAX_ARCHIVE_BYTES { return Err(exhausted()); }
        budget.charge(length.max(HEADER_BYTES))?;
        if length == 0 && initialize_empty {
            let mut seed = b"dfmcp-progress-archive-identity/1\0".to_vec();
            seed.extend_from_slice(&c.session_id.get().to_be_bytes());
            seed.extend_from_slice(&c.request_id.get().to_be_bytes());
            seed.extend_from_slice(&c.anchor.fortress_id.get().to_be_bytes());
            let mut header = MAGIC.to_vec(); header.extend_from_slice(&c.anchor.fortress_id.get().to_be_bytes());
            header.extend_from_slice(Digest32::of_bytes(&seed).as_bytes());
            let digest = header_hash(&header); header.extend_from_slice(digest.as_bytes());
            budget.charge(HEADER_BYTES)?;
            storage.seek(SeekFrom::Start(0)).map_err(storage_error)?;
            storage.write_all(&header).map_err(storage_error)?; storage.sync().map_err(storage_error)?;
            length = HEADER_BYTES;
        }
        if length < HEADER_BYTES { return Err(corrupt("incomplete progress archive header; bytes unchanged")); }
        storage.seek(SeekFrom::Start(0)).map_err(storage_error)?;
        let mut header = [0; HEADER_BYTES as usize]; storage.read_exact(&mut header).map_err(storage_error)?;
        let mut r = Reader { bytes: &header };
        if r.take(8)? != MAGIC { return Err(corrupt("not a progress/1.12 archive")); }
        let fortress = FortressId::new(r.u64()?); let id = Digest32::from_bytes(r.array()?); let head = Digest32::from_bytes(r.array()?);
        if fortress != c.anchor.fortress_id || head != header_hash(&header[..48]) { return Err(corrupt("archive identity or header checksum mismatch")); }
        let mut out = Self { storage, fortress, id, head, length: HEADER_BYTES, entries: Vec::new(), last: None,
            tick_floor: 0, mode, new_segment: true, fenced: false };
        while out.length < length {
            budget.check()?;
            if out.entries.len() >= MAX_ARCHIVE_RECORDS || out.entries.len() >= c.budget.max_entities as usize {
                return Err(exhausted());
            }
            let record = out.read_frame(out.length, length)?;
            if record.observation.rows().len() > c.budget.max_entities as usize { return Err(exhausted()); }
            out.validate_next(&record)?;
            out.length += PREFIX_BYTES as u64 + u64::from(record.entry.body_bytes) + 40;
            out.head = record.entry.record_digest;
            out.tick_floor = out.tick_floor.max(record.observation.tick());
            out.entries.push(record.entry.clone()); out.last = Some(record);
        }
        out.access(c)?; budget.check()?; Ok(out)
    }

    fn authority(&self, c: &OperationContext, write: bool) -> Result<()> {
        if c.anchor.fortress_id != self.fortress { return Err(error(ErrorCode::CapabilityDenied, "archive belongs to another fortress")); }
        let mut current = c.clone(); current.anchor.tick = GameTick(self.tick_floor.max(c.anchor.tick.get()));
        current.authorize(Capability::Query, super::RiskTier::ReadOnly, &[], None)?;
        if write {
            if self.mode != ArchiveMode::Live { return Err(error(ErrorCode::CapabilityDenied, "offline archive cannot append, even with an injected Observe grant")); }
            current.authorize(Capability::Observe, super::RiskTier::ReadOnly, &[], None)?;
        }
        Ok(())
    }
    pub fn access(&mut self, c: &OperationContext) -> Result<()> {
        self.authority(c, false)?;
        if self.fenced { return Err(corrupt("progress archive fenced; reopen for verified recovery")); }
        let check = (|| {
            self.storage.validate_identity().map_err(storage_error)?;
            if self.storage.seek(SeekFrom::End(0)).map_err(storage_error)? != self.length { return Err(corrupt("archive extent changed outside its owner")); }
            Ok(())
        })();
        if check.is_err() { self.fenced = true; }
        check
    }
    pub fn summary(&mut self, c: &OperationContext) -> Result<ProgressArchiveSummary> {
        let budget = Budget::new(c)?; self.access(c)?; budget.check()?;
        Ok(ProgressArchiveSummary { archive_id: self.id, head: self.head, fortress_id: self.fortress,
            records: self.entries.len(), segments: self.last.as_ref().map_or(0, |r| r.entry.segment),
            retained_bytes: self.length, authority_tick_floor: self.tick_floor, read_only: self.mode == ArchiveMode::Offline })
    }
    /// Call before contacting the native source; capacity cannot be recovered by
    /// eviction, truncation, or skipping a capture. No writes occur here.
    pub fn reserve_capture(&mut self, c: &OperationContext) -> Result<()> {
        let mut budget = Budget::new(c)?; self.access(c)?; self.authority(c, true)?;
        if self.entries.len() >= MAX_ARCHIVE_RECORDS || self.length + MAX_FRAME_BYTES > MAX_ARCHIVE_BYTES { return Err(exhausted()); }
        budget.charge(MAX_FRAME_BYTES)
    }
    pub fn append(&mut self, manifest: &ProgressManifest, observation: &ProgressObservation,
        c: &OperationContext) -> Result<ProgressArchiveEntry>
    {
        let mut budget = Budget::new(c)?;
        self.reserve_capture(c)?;
        let mut fresh = c.clone(); fresh.anchor.tick = GameTick(c.anchor.tick.get().max(observation.tick()));
        self.authority(&fresh, true)?;
        if observation.rows().len() > c.budget.max_entities as usize { return Err(exhausted()); }
        let observation = ProgressObservation::decode(observation.canonical_bytes(), &observation.ids())?;
        validate_manifest(manifest, &observation)?;
        if observation.fortress_id() != self.fortress { return Err(corrupt("capture crosses archive fortress lineage")); }
        let segment = match &self.last {
            None => 1,
            Some(old) => {
                // Reopen always starts a segment; no downtime continuity is invented.
                let reset = self.new_segment || old.manifest != *manifest
                    || compare(Some(&old.observation), &observation)?.status == "reset";
                old.entry.segment.checked_add(u64::from(reset)).ok_or_else(exhausted)?
            }
        };
        let body = encode_body(segment, manifest, &observation)?;
        let number = self.entries.len() as u64 + 1;
        let mut prefix = FRAME.to_vec(); prefix.extend_from_slice(&(body.len() as u32).to_be_bytes());
        prefix.extend_from_slice(&number.to_be_bytes()); prefix.extend_from_slice(self.head.as_bytes());
        let digest = frame_hash(self.id, &prefix, &body);
        let entry = ProgressArchiveEntry { number, segment, record_digest: digest, previous_digest: self.head,
            witness: observation.witness(), game_tick: observation.tick(), native_order_ids: observation.ids(),
            offset: self.length, body_bytes: body.len() as u32 };
        let record = ArchivedProgress { entry: entry.clone(), manifest: manifest.clone(), observation };
        self.validate_next(&record)?;
        let mut bytes = prefix; bytes.extend_from_slice(&body); bytes.extend_from_slice(digest.as_bytes()); bytes.extend_from_slice(FOOTER);
        budget.charge(bytes.len() as u64)?;
        let result = (|| {
            self.access(&fresh)?;
            self.storage.seek(SeekFrom::Start(self.length)).map_err(storage_error)?;
            self.storage.write_all(&bytes).map_err(storage_error)?;
            self.storage.sync().map_err(storage_error)?;
            self.storage.validate_identity().map_err(storage_error)?;
            if self.storage.seek(SeekFrom::End(0)).map_err(storage_error)? != self.length + bytes.len() as u64 {
                return Err(corrupt("archive write did not retain the exact complete frame"));
            }
            budget.check()?; Ok(())
        })();
        if result.is_err() { self.fenced = true; }
        result?;
        self.length += bytes.len() as u64; self.head = digest; self.tick_floor = self.tick_floor.max(entry.game_tick);
        self.entries.push(entry.clone()); self.last = Some(record); self.new_segment = false;
        Ok(entry)
    }
    pub fn latest(&mut self, c: &OperationContext) -> Result<Option<ArchivedProgress>> {
        self.access(c)?;
        let identity = self.entries.last().map(|r| (r.number, r.record_digest));
        identity.map(|(number, digest)| self.record(number, digest, c)).transpose()
    }
    pub fn record(&mut self, number: u64, digest: Digest32, c: &OperationContext) -> Result<ArchivedProgress> {
        let mut budget = Budget::new(c)?; self.access(c)?;
        let index = number.checked_sub(1).and_then(|n| usize::try_from(n).ok()).ok_or_else(|| error(ErrorCode::InvalidRequest, "invalid archive record number"))?;
        let expected = self.entries.get(index).ok_or_else(|| error(ErrorCode::InvalidRequest, "archive record is not retained"))?.clone();
        if expected.native_order_ids.len() > c.budget.max_entities as usize { return Err(exhausted()); }
        if expected.record_digest != digest { return Err(error(ErrorCode::StaleAnchor, "archive record digest does not match")); }
        budget.charge(u64::from(expected.body_bytes) + MAX_FRAME_BYTES - MAX_BODY as u64)?;
        let result = (|| {
            let record = self.read_frame(expected.offset, self.length)?;
            if record.entry != expected { return Err(corrupt("archive record changed after replay")); }
            self.access(c)?; budget.check()?; Ok(record)
        })();
        if result.as_ref().is_err_and(|e| matches!(e.code, ErrorCode::CorruptLedger | ErrorCode::AdapterRejected)) { self.fenced = true; }
        result
    }
    /// Whole metadata rows, not unbounded payloads. Returned references remain
    /// usable after restart; continuation binding belongs to the MCP session.
    pub fn page(&mut self, expected_head: Digest32, after: u64, limit: usize, c: &OperationContext) -> Result<ProgressArchivePage> {
        let mut budget = Budget::new(c)?; self.access(c)?;
        if expected_head != self.head { return Err(error(ErrorCode::StaleAnchor, "archive changed; restart history discovery")); }
        if !(1..=64).contains(&limit) || limit > c.budget.max_entities as usize { return Err(exhausted()); }
        let start = usize::try_from(after).map_err(|_| error(ErrorCode::InvalidRequest, "invalid history offset"))?;
        if start > self.entries.len() { return Err(error(ErrorCode::InvalidRequest, "history offset exceeds retained records")); }
        let count = limit.min(self.entries.len() - start); budget.charge(count as u64 * 512)?;
        let entries = self.entries[start..start + count].to_vec();
        let next_after = (start + count < self.entries.len()).then_some((start + count) as u64);
        self.access(c)?; budget.check()?; Ok(ProgressArchivePage { entries, next_after })
    }
    pub fn compare_records(&mut self, before: (u64, Digest32), after: (u64, Digest32), c: &OperationContext)
        -> Result<(ArchivedProgress, ArchivedProgress, ProgressComparison)>
    {
        let mut budget = Budget::new(c)?; self.access(c)?;
        if before.0 >= after.0 { return Err(error(ErrorCode::InvalidRequest, "history comparison requires strictly ordered distinct records")); }
        budget.charge(2 * MAX_FRAME_BYTES)?;
        let a = self.record(before.0, before.1, c)?; let b = self.record(after.0, after.1, c)?;
        if a.entry.segment != b.entry.segment {
            return Err(error(ErrorCode::StaleAnchor, "history comparison crosses a restart, selection or source discontinuity"));
        }
        let comparison = compare(Some(&a.observation), &b.observation)?;
        if comparison.status != "compared" { return Err(corrupt("history segment contradicts endpoint continuity")); }
        self.access(c)?; budget.check()?; Ok((a, b, comparison))
    }

    fn validate_next(&self, next: &ArchivedProgress) -> Result<()> {
        if next.entry.number != self.entries.len() as u64 + 1 || next.entry.previous_digest != self.head
            || next.observation.fortress_id() != self.fortress { return Err(corrupt("archive record order, predecessor or fortress mismatch")); }
        validate_manifest(&next.manifest, &next.observation)?;
        match &self.last {
            None if next.entry.segment != 1 => return Err(corrupt("archive first segment must be one")),
            Some(old) => {
                if next.entry.segment == old.entry.segment {
                    if next.manifest != old.manifest || compare(Some(&old.observation), &next.observation)?.status != "compared" {
                        return Err(corrupt("archive continued across a discontinuity"));
                    }
                } else if old.entry.segment.checked_add(1) != Some(next.entry.segment) {
                    return Err(corrupt("archive segment sequence is not consecutive"));
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn read_frame(&mut self, offset: u64, extent: u64) -> Result<ArchivedProgress> {
        if extent.saturating_sub(offset) < PREFIX_BYTES as u64 + 40 { return Err(corrupt("incomplete progress archive tail; bytes unchanged")); }
        self.storage.seek(SeekFrom::Start(offset)).map_err(storage_error)?;
        let mut prefix = [0; PREFIX_BYTES]; self.storage.read_exact(&mut prefix).map_err(storage_error)?;
        let mut r = Reader { bytes: &prefix };
        if r.take(8)? != FRAME { return Err(corrupt("invalid progress archive frame")); }
        let size = r.u32()?; let number = r.u64()?; let previous_digest = Digest32::from_bytes(r.array()?);
        if size as usize > MAX_BODY || u64::from(size) + PREFIX_BYTES as u64 + 40 > extent - offset {
            return Err(corrupt("oversized or incomplete progress archive record"));
        }
        let mut body = vec![0; size as usize]; self.storage.read_exact(&mut body).map_err(storage_error)?;
        let mut trailer = [0; 40]; self.storage.read_exact(&mut trailer).map_err(storage_error)?;
        let mut r = Reader { bytes: &trailer }; let record_digest = Digest32::from_bytes(r.array()?);
        if r.take(8)? != FOOTER || record_digest != frame_hash(self.id, &prefix, &body) { return Err(corrupt("progress record checksum or footer mismatch")); }
        let (segment, manifest, observation) = decode_body(&body)?;
        Ok(ArchivedProgress { entry: ProgressArchiveEntry { number, segment, record_digest, previous_digest,
            witness: observation.witness(), game_tick: observation.tick(), native_order_ids: observation.ids(), offset, body_bytes: size },
            manifest, observation })
    }
}
fn validate_manifest(m: &ProgressManifest, o: &ProgressObservation) -> Result<()> {
    if m.generation != o.generation() || [&m.df_version, &m.dfhack_version].iter().any(|v| v.is_empty() || v.len() > 128 || v.contains('\0')) {
        return Err(corrupt("archive manifest does not match complete capture"));
    }
    Ok(())
}
fn header_hash(data: &[u8]) -> Digest32 {
    let mut bytes = b"dfmcp-progress-archive-header/1\0".to_vec(); bytes.extend_from_slice(data); Digest32::of_bytes(&bytes)
}
fn frame_hash(id: Digest32, prefix: &[u8], body: &[u8]) -> Digest32 {
    let mut bytes = b"dfmcp-progress-archive-frame/1\0".to_vec(); bytes.extend_from_slice(id.as_bytes());
    bytes.extend_from_slice(prefix); bytes.extend_from_slice(body); Digest32::of_bytes(&bytes)
}
fn text(out: &mut Vec<u8>, value: &str) { out.extend_from_slice(&(value.len() as u16).to_be_bytes()); out.extend_from_slice(value.as_bytes()); }
fn encode_body(segment: u64, m: &ProgressManifest, o: &ProgressObservation) -> Result<Vec<u8>> {
    validate_manifest(m, o)?;
    let mut out = segment.to_be_bytes().to_vec(); out.extend_from_slice(&m.generation.to_be_bytes());
    text(&mut out, &m.df_version); text(&mut out, &m.dfhack_version);
    let ids = o.ids(); out.push(ids.len() as u8); for id in ids { out.extend_from_slice(&id.to_be_bytes()); }
    out.extend_from_slice(&(o.canonical_bytes().len() as u32).to_be_bytes()); out.extend_from_slice(o.canonical_bytes());
    if out.len() > MAX_BODY { return Err(exhausted()); } Ok(out)
}
fn decode_body(bytes: &[u8]) -> Result<(u64, ProgressManifest, ProgressObservation)> {
    let mut r = Reader { bytes }; let segment = r.u64()?; let generation = r.u64()?;
    let manifest = ProgressManifest { generation, df_version: r.text(128, false)?, dfhack_version: r.text(128, false)? };
    let count = usize::from(r.byte()?);
    if segment == 0 || !(1..=super::MAX_TARGETS).contains(&count) { return Err(corrupt("invalid progress archive selection or segment")); }
    let mut ids = Vec::with_capacity(count); for _ in 0..count { ids.push(r.u32()?); } validate_targets(&ids)?;
    let size = r.u32()? as usize;
    let observation = ProgressObservation::decode(r.take(size)?, &ids)?;
    if !r.bytes.is_empty() { return Err(corrupt("trailing progress archive payload")); }
    validate_manifest(&manifest, &observation)?; Ok((segment, manifest, observation))
}

#[cfg(test)]
#[path = "archive_tests.rs"]
mod tests;
