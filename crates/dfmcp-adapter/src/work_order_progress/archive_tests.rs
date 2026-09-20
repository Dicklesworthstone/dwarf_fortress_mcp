use super::*;
use std::cell::RefCell;
use std::io::{Read, Seek, Write};
use std::rc::Rc;
use dfmcp_core::{CapabilityGrant, CapabilityScope, ObservationCursor, RequestId, RiskTier, SessionId, StateAnchor, WorkBudget};

fn capture(sequence: u64, tick: u64, remaining: i32) -> Result<ProgressObservation> {
    let text = include_str!("../../tests/fixtures/work_order_progress_v1_12.hex").trim();
    let mut bytes: Vec<u8> = text.as_bytes().chunks_exact(2).map(|pair| {
        let s = std::str::from_utf8(pair).map_err(|_| corrupt("fixture UTF-8"))?;
        u8::from_str_radix(s, 16).map_err(|_| corrupt("fixture hex"))
    }).collect::<Result<_>>()?;
    bytes[16..24].copy_from_slice(&sequence.to_be_bytes()); bytes[24..32].copy_from_slice(&tick.to_be_bytes());
    bytes[68..72].copy_from_slice(&remaining.to_be_bytes()); ProgressObservation::decode(&bytes, &[3, 8])
}
fn context() -> Result<OperationContext> {
    Ok(OperationContext { session_id: SessionId::new(1), request_id: RequestId::new(2),
        anchor: StateAnchor { fortress_id: capture(1, 10, 5)?.fortress_id(), cursor: ObservationCursor::ORIGIN,
            tick: GameTick(0), state_hash: Digest32::ZERO },
        budget: WorkBudget { max_entities: 4096, ..WorkBudget::default() }, cancellation_requested: false,
        grants: [Capability::Query, Capability::Observe].into_iter().map(|capability| CapabilityGrant {
            capability, scope: CapabilityScope::default(), max_risk: RiskTier::ReadOnly, expires_at_tick: None, remaining_uses: None,
        }).collect() })
}
fn manifest() -> ProgressManifest { ProgressManifest { generation: 7, df_version: "df".into(), dfhack_version: "dfhack".into() } }
#[derive(Clone, Default)]
struct Shared { bytes: Rc<RefCell<Vec<u8>>>, events: Rc<RefCell<Vec<&'static str>>>, fail_sync: Rc<RefCell<bool>> }
struct Memory { shared: Shared, position: usize }
impl Memory { fn new(shared: Shared) -> Self { Self { shared, position: 0 } } }
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let bytes = self.shared.bytes.borrow();
        let n = out.len().min(bytes.len().saturating_sub(self.position));
        if n > 0 { out[..n].copy_from_slice(&bytes[self.position..self.position + n]); }
        self.position += n; Ok(n)
    }
}
impl Write for Memory {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.shared.events.borrow_mut().push("write"); let mut bytes = self.shared.bytes.borrow_mut();
        let length = bytes.len().max(self.position + data.len()); bytes.resize(length, 0);
        bytes[self.position..self.position + data.len()].copy_from_slice(data); self.position += data.len(); Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
impl Seek for Memory {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let n = match from { SeekFrom::Start(n) => i128::from(n), SeekFrom::Current(n) => self.position as i128 + i128::from(n),
            SeekFrom::End(n) => self.shared.bytes.borrow().len() as i128 + i128::from(n) };
        self.position = usize::try_from(n).map_err(|_| io::Error::other("invalid test seek"))?; Ok(self.position as u64)
    }
}
impl JournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        self.shared.events.borrow_mut().push("sync");
        if *self.shared.fail_sync.borrow() { Err(io::Error::other("injected sync failure")) } else { Ok(()) }
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> { Err(io::Error::other("no truncation permitted")) }
}
fn setup() -> Result<(ProgressArchive<Memory>, Shared, OperationContext)> {
    let shared = Shared::default(); let c = context()?;
    let a = ProgressArchive::open(Memory::new(shared.clone()), ArchiveMode::Live, true, &c)?;
    Ok((a, shared, c))
}
fn reopen(bytes: Vec<u8>, mode: ArchiveMode, c: &OperationContext) -> Result<ProgressArchive<Memory>> {
    let shared = Shared::default(); *shared.bytes.borrow_mut() = bytes;
    ProgressArchive::open(Memory::new(shared), mode, false, c)
}
#[test]
fn replay_preserves_complete_native_evidence_and_exact_references() -> Result<()> {
    let (mut a, shared, c) = setup()?;
    let first = a.append(&manifest(), &capture(1, 10, 5)?, &c)?;
    let second = a.append(&manifest(), &capture(2, 20, 2)?, &c)?;
    assert_eq!(shared.events.borrow().as_slice(), &["write", "sync", "write", "sync", "write", "sync"]);
    let mut recovered = reopen(shared.bytes.borrow().clone(), ArchiveMode::Offline, &c)?;
    assert_eq!(recovered.record(first.number, first.record_digest, &c)?.observation, capture(1, 10, 5)?);
    let (_, _, comparison) = recovered.compare_records((first.number, first.record_digest), (second.number, second.record_digest), &c)?;
    assert_eq!(comparison.changes[0].remaining_decrease, Some(3));
    assert_eq!(recovered.summary(&c)?.records, 2); Ok(())
}
#[test]
fn reopen_starts_a_new_segment_and_does_not_invent_downtime_continuity() -> Result<()> {
    let (mut a, shared, c) = setup()?;
    let old = a.append(&manifest(), &capture(1, 10, 5)?, &c)?;
    let mut recovered = reopen(shared.bytes.borrow().clone(), ArchiveMode::Live, &c)?;
    let fresh = recovered.append(&manifest(), &capture(2, 20, 1)?, &c)?;
    assert_eq!(fresh.segment, old.segment + 1);
    assert!(recovered.compare_records((old.number, old.record_digest), (fresh.number, fresh.record_digest), &c).is_err());
    let later = recovered.append(&manifest(), &capture(3, 21, 0)?, &c)?;
    assert_eq!(fresh.segment, later.segment);
    assert!(recovered.compare_records((fresh.number, fresh.record_digest), (later.number, later.record_digest), &c).is_ok()); Ok(())
}
#[test]
fn resets_and_software_changes_partition_history() -> Result<()> {
    let (mut a, _, c) = setup()?;
    assert_eq!(a.append(&manifest(), &capture(1, 10, 5)?, &c)?.segment, 1);
    assert_eq!(a.append(&manifest(), &capture(2, 9, 5)?, &c)?.segment, 2);
    let mut software = manifest(); software.df_version.push('2');
    assert_eq!(a.append(&software, &capture(3, 10, 4)?, &c)?.segment, 3);
    assert!(a.append(&software, &capture(3, 11, 3)?, &c).is_err());
    assert_eq!(a.summary(&c)?.records, 3); Ok(())
}
#[test]
fn offline_mode_cannot_be_promoted_and_old_ticks_cannot_revive_expired_grants() -> Result<()> {
    let (mut a, shared, c) = setup()?; a.append(&manifest(), &capture(1, 100, 5)?, &c)?;
    let bytes = shared.bytes.borrow().clone();
    let mut offline = reopen(bytes.clone(), ArchiveMode::Offline, &c)?;
    assert!(offline.append(&manifest(), &capture(2, 101, 4)?, &c).is_err());
    assert!(offline.reserve_capture(&c).is_err());
    assert_eq!(*offline.storage.shared.bytes.borrow(), bytes);
    let mut denied = c.clone(); for g in &mut denied.grants { g.expires_at_tick = Some(GameTick(99)); }
    assert!(offline.latest(&denied).is_err()); assert!(reopen(bytes, ArchiveMode::Offline, &denied).is_err());
    denied = c.clone(); denied.cancellation_requested = true; assert!(offline.latest(&denied).is_err());
    denied = c.clone(); denied.anchor.fortress_id = FortressId::new(99); assert!(offline.summary(&denied).is_err()); Ok(())
}
#[test]
fn failed_sync_fences_owner_but_complete_uncertain_frame_can_be_recovered() -> Result<()> {
    let (mut a, shared, c) = setup()?; *shared.fail_sync.borrow_mut() = true;
    assert!(a.append(&manifest(), &capture(1, 10, 5)?, &c).is_err());
    assert!(a.summary(&c).is_err()); assert!(a.last.is_none());
    let mut reopened = reopen(shared.bytes.borrow().clone(), ArchiveMode::Offline, &c)?;
    assert_eq!(reopened.summary(&c)?.records, 1); Ok(())
}
#[test]
fn torn_tails_and_every_byte_corruption_are_refused_without_repair() -> Result<()> {
    let (mut a, shared, c) = setup()?; a.append(&manifest(), &capture(1, 10, 5)?, &c)?;
    let bytes = shared.bytes.borrow().clone();
    for end in 0..bytes.len() {
        if end != HEADER_BYTES as usize { assert!(reopen(bytes[..end].to_vec(), ArchiveMode::Offline, &c).is_err(), "prefix {end}"); }
    }
    for at in 0..bytes.len() {
        let mut bad = bytes.clone(); bad[at] ^= 1;
        assert!(reopen(bad, ArchiveMode::Offline, &c).is_err(), "byte {at}");
    }
    assert!(reopen(Vec::new(), ArchiveMode::Live, &c).is_err()); Ok(())
}
#[test]
fn same_length_corruption_and_external_extent_changes_invalidate_reads() -> Result<()> {
    let (mut a, shared, c) = setup()?; let e = a.append(&manifest(), &capture(1, 10, 5)?, &c)?;
    shared.bytes.borrow_mut()[e.offset as usize + PREFIX_BYTES + 20] ^= 1;
    assert!(a.record(e.number, e.record_digest, &c).is_err()); assert!(a.fenced);
    let (mut a, shared, c) = setup()?; shared.bytes.borrow_mut().push(0);
    assert!(a.summary(&c).is_err()); assert!(a.fenced); Ok(())
}
#[test]
fn history_pages_and_exact_lookups_are_head_bound_and_budgeted() -> Result<()> {
    let (mut a, _, c) = setup()?;
    for n in 1..=3 { a.append(&manifest(), &capture(n, n * 10, 5)?, &c)?; }
    let summary = a.summary(&c)?;
    let page = a.page(summary.head, 0, 2, &c)?;
    assert_eq!(page.next_after, Some(2)); assert_eq!(page.entries.len(), 2);
    assert_eq!(a.page(summary.head, 2, 2, &c)?.entries.len(), 1);
    assert!(a.record(1, Digest32::ZERO, &c).is_err()); assert!(a.record(0, summary.head, &c).is_err());
    let mut small = c.clone(); small.budget.max_bytes = 511; assert!(a.page(summary.head, 0, 1, &small).is_err());
    small = c.clone(); small.budget.max_entities = 1; assert!(a.latest(&small).is_err());
    a.append(&manifest(), &capture(4, 40, 5)?, &c)?;
    assert!(a.page(summary.head, 2, 2, &c).is_err()); Ok(())
}
#[test]
fn capacity_refusal_occurs_without_writing_or_silently_evicting() -> Result<()> {
    let (mut a, shared, c) = setup()?;
    let e = a.append(&manifest(), &capture(1, 10, 5)?, &c)?;
    a.entries.resize(MAX_ARCHIVE_RECORDS, e);
    let before = shared.bytes.borrow().clone(); assert!(a.reserve_capture(&c).is_err());
    assert_eq!(*shared.bytes.borrow(), before);
    let mut small = c.clone(); small.budget.max_bytes = MAX_FRAME_BYTES - 1;
    let (mut empty, _, _) = setup()?; assert!(empty.reserve_capture(&small).is_err()); Ok(())
}
#[test]
fn rehashed_illegal_segment_and_reordered_samples_do_not_replay() -> Result<()> {
    let (mut a, shared, c) = setup()?;
    let first = a.append(&manifest(), &capture(1, 10, 5)?, &c)?;
    let second = a.append(&manifest(), &capture(2, 20, 4)?, &c)?;
    let original = shared.bytes.borrow().clone();
    for segment in [0u64, 3] {
        let mut bad = original.clone(); let at = second.offset as usize;
        bad[at + PREFIX_BYTES..at + PREFIX_BYTES + 8].copy_from_slice(&segment.to_be_bytes());
        let end = at + PREFIX_BYTES + second.body_bytes as usize;
        let digest = frame_hash(a.id, &bad[at..at + PREFIX_BYTES], &bad[at + PREFIX_BYTES..end]);
        bad[end..end + 32].copy_from_slice(digest.as_bytes());
        assert!(reopen(bad, ArchiveMode::Offline, &c).is_err());
    }
    let mut bad = original; let at = second.offset as usize;
    let body = encode_body(first.segment, &manifest(), &capture(1, 20, 4)?)?;
    bad[at + PREFIX_BYTES..at + PREFIX_BYTES + body.len()].copy_from_slice(&body);
    let digest = frame_hash(a.id, &bad[at..at + PREFIX_BYTES], &body);
    let end = at + PREFIX_BYTES + body.len(); bad[end..end + 32].copy_from_slice(digest.as_bytes());
    assert!(reopen(bad, ArchiveMode::Offline, &c).is_err()); Ok(())
}

#[cfg(unix)]
#[test]
fn private_archive_locks_permissions_offline_descriptor_and_renamed_custody() -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let root = std::env::temp_dir().canonicalize().map_err(storage_error)?
        .join(format!("dfmcp-progress-archive-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let mut builder = std::fs::DirBuilder::new(); builder.mode(0o700); builder.create(&root).map_err(storage_error)?;
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
    let _cleanup = Cleanup(root.clone()); let path = root.join("progress.bin"); let c = context()?;
    assert!(open_progress_archive(&path, ArchiveMode::Offline, &c).is_err()); assert!(!path.exists());
    let mut live = open_progress_archive(&path, ArchiveMode::Live, &c)?;
    live.append(&manifest(), &capture(1, 10, 5)?, &c)?;
    assert!(open_progress_archive(&path, ArchiveMode::Offline, &c).is_err()); drop(live);
    let mut offline = open_progress_archive(&path, ArchiveMode::Offline, &c)?;
    assert!(offline.storage.write_all(b"not allowed").is_err()); assert!(offline.storage.sync().is_err());
    assert!(offline.storage.truncate(0).is_err()); assert!(offline.latest(&c)?.is_some());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).map_err(storage_error)?;
    assert!(offline.summary(&c).is_err()); drop(offline);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).map_err(storage_error)?;
    let mut live = open_progress_archive(&path, ArchiveMode::Live, &c)?;
    std::fs::rename(&path, root.join("moved.bin")).map_err(storage_error)?;
    assert!(live.summary(&c).is_err()); Ok(())
}


struct Source { manifest: ProgressManifest, next: u64, repeat: bool, fenced: bool }
impl crate::work_order_progress::ProgressSource for Source {
    fn manifest(&self) -> &ProgressManifest { &self.manifest }
    fn fence(&mut self) { self.fenced = true; }
    fn read(&mut self, _: &[u32], _: Duration) -> Result<ProgressObservation> {
        if self.fenced { return Err(error(ErrorCode::AdapterUnavailable, "source fenced")); }
        let sequence = self.next; if !self.repeat { self.next += 1; }
        capture(sequence, sequence * 10, 3)
    }
}
#[test]
fn durable_publication_barrier_precedes_current_capture_and_failure_fences() -> Result<()> {
    let c = context()?;
    let source = Source { manifest: manifest(), next: 1, repeat: false, fenced: false };
    let mut session = crate::work_order_progress::ProgressSession::new(source, &c)?;
    let published = std::cell::Cell::new(false);
    session.refresh_with_publication(&[3, 8], &c, |manifest, capture, context| {
        assert_eq!(manifest.generation, capture.generation()); assert_eq!(context.anchor.tick, GameTick(10));
        assert!(context.budget.max_wall_millis <= c.budget.max_wall_millis);
        published.set(true); Ok(())
    })?;
    assert!(published.get()); assert_eq!(session.current(&c)?.map(ProgressObservation::tick), Some(10));
    assert!(session.refresh_with_publication(&[3, 8], &c, |_, _, _|
        Err(error(ErrorCode::CorruptLedger, "injected durability failure"))).is_err());
    assert!(session.current(&c)?.is_none());
    assert!(session.refresh(&[3, 8], &c).is_err()); Ok(())
}
#[test]
fn invalid_native_evidence_never_reaches_publication_callback() -> Result<()> {
    let c = context()?; let source = Source { manifest: manifest(), next: 1, repeat: true, fenced: false };
    let mut session = crate::work_order_progress::ProgressSession::new(source, &c)?; session.refresh(&[3, 8], &c)?;
    let called = std::cell::Cell::new(false);
    assert!(session.refresh_with_publication(&[3, 8], &c, |_, _, _| { called.set(true); Ok(()) }).is_err());
    assert!(!called.get()); assert!(session.current(&c)?.is_none()); Ok(())
}
