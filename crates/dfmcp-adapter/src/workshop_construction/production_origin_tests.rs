//! Real journal/ACK codecs and original native vectors with a counted native
//! source double. These tests establish adapter custody, not a native game run.
use super::*;
use crate::workshop_construction::tests::{capture, plan, progress, record, vector};
use crate::workshop_construction::{WorkshopCapture, WorkshopNativeSummary, WorkshopPreparation};
use std::cell::RefCell;
use std::io::{self, Read, Seek, Write};
use std::net::SocketAddr;
use std::rc::Rc;

#[derive(Default)]
struct Disk {
    bytes: Vec<u8>,
    reads: usize,
    writes: usize,
}
#[derive(Clone, Default)]
struct Memory {
    disk: Rc<RefCell<Disk>>,
    position: usize,
}
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let mut disk = self.disk.borrow_mut();
        disk.reads += 1;
        let start = self.position.min(disk.bytes.len());
        let length = out.len().min(disk.bytes.len() - start);
        out[..length].copy_from_slice(&disk.bytes[start..start + length]);
        self.position += length;
        Ok(length)
    }
}
impl Seek for Memory {
    fn seek(&mut self, offset: SeekFrom) -> io::Result<u64> {
        let position = match offset {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::End(n) => self.disk.borrow().bytes.len() as i128 + i128::from(n),
            SeekFrom::Current(n) => self.position as i128 + i128::from(n),
        };
        self.position = usize::try_from(position).map_err(|_| io::Error::other("invalid seek"))?;
        Ok(self.position as u64)
    }
}
impl Write for Memory {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut disk = self.disk.borrow_mut();
        if self.position != disk.bytes.len() {
            return Err(io::Error::other("append only"));
        }
        disk.writes += 1;
        disk.bytes.extend_from_slice(bytes);
        self.position += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("repair forbidden"))
    }
}

struct Fixture {
    journal: WorkshopJournal<Memory>,
    ack: WorkshopAcknowledgements<Memory>,
    journal_disk: Rc<RefCell<Disk>>,
    ack_disk: Rc<RefCell<Disk>>,
    context: OperationContext,
}
impl Fixture {
    fn new(phase: &str, samples: &[WorkshopProgress]) -> Result<Self> {
        let context = super::super::tests::context(71)?;
        let before = capture()?;
        let binding = WorkshopBinding::from_source_pins(
            SocketAddr::from(([127, 0, 0, 1], 5000)),
            before.fortress().clone(),
            before.generation(),
            before.dimensions(),
            "test-df",
            "test-dfhack",
        )?;
        let storage = Memory::default();
        let ack_storage = Memory::default();
        let mut journal = WorkshopJournal::open(
            storage.clone(),
            &context,
            WorkshopMode::Control,
            Some(binding.clone()),
            Some([9; 32]),
            None,
        )?;
        let mut ack = WorkshopAcknowledgements::open(
            ack_storage.clone(),
            &context,
            WorkshopMode::Control,
            true,
            None,
        )?;
        ack.acknowledge(journal.pin(), &context)?;
        let mut entry = WorkshopEntry {
            plan: plan()?,
            state: WorkshopState::Intent,
            native: None,
            dispatch_started: false,
            cancel_requested: false,
            samples: 0,
            latest: None,
            high_tick: before.tick(),
            high_generation: before.generation(),
            continuity_lost: false,
        };
        journal.retain(entry.clone(), &mut Work::new(&context)?)?;
        ack.acknowledge(journal.pin(), &context)?;
        entry.state = WorkshopState::Prepared;
        entry.native = Some(record("prepared")?);
        journal.retain(entry.clone(), &mut Work::new(&context)?)?;
        ack.acknowledge(journal.pin(), &context)?;
        if phase != "prepared" {
            if matches!(phase, "placed" | "indeterminate") {
                entry.state = WorkshopState::DispatchStarted;
                entry.dispatch_started = true;
                journal.retain(entry.clone(), &mut Work::new(&context)?)?;
                ack.acknowledge(journal.pin(), &context)?;
            }
            entry.state = WorkshopState::Terminal;
            entry.native = Some(record(phase)?);
            entry = journal.retain(entry, &mut Work::new(&context)?)?;
            ack.acknowledge(journal.pin(), &context)?;
        }
        for sample in samples {
            entry.samples += 1;
            entry.latest = Some(sample.clone());
            entry = journal.retain(entry, &mut Work::new(&context)?)?;
            ack.acknowledge(journal.pin(), &context)?;
        }
        let pin = journal.pin();
        drop(journal);
        drop(ack);
        let journal = WorkshopJournal::open(
            storage.clone(),
            &context,
            WorkshopMode::Offline,
            Some(binding),
            None,
            Some(pin),
        )?;
        let ack = WorkshopAcknowledgements::open(
            ack_storage.clone(),
            &context,
            WorkshopMode::Offline,
            false,
            Some(pin),
        )?;
        let mut context = context;
        context
            .grants
            .retain(|g| matches!(g.capability, Capability::Query | Capability::Observe));
        Ok(Self {
            journal,
            ack,
            journal_disk: storage.disk,
            ack_disk: ack_storage.disk,
            context,
        })
    }
    fn origin(&mut self) -> Result<WorkshopProductionOrigin> {
        let original = plan()?;
        self.journal.production_origin(
            &mut self.ack,
            original.key(),
            original.digest(),
            &self.context,
        )
    }
    fn bytes(&self) -> (Vec<u8>, Vec<u8>) {
        (
            self.journal_disk.borrow().bytes.clone(),
            self.ack_disk.borrow().bytes.clone(),
        )
    }
    fn writes(&self) -> (usize, usize) {
        (
            self.journal_disk.borrow().writes,
            self.ack_disk.borrow().writes,
        )
    }
    fn reads(&self) -> (usize, usize) {
        (
            self.journal_disk.borrow().reads,
            self.ack_disk.borrow().reads,
        )
    }
    fn refresh(
        &mut self,
        origin: &mut WorkshopProductionOrigin,
        source: &mut Native,
    ) -> Result<CompletedWorkshopProduction> {
        origin.refresh(
            &mut self.journal,
            &mut self.ack,
            source,
            self.context.anchor.tick,
            &self.context,
        )
    }
}

fn sample(name: &str, tick: u64, horizon: u32) -> Result<WorkshopProgress> {
    let mut raw = vector(name)?;
    raw[16..24].copy_from_slice(&tick.to_be_bytes());
    let folder = u16::from_be_bytes(raw[40..42].try_into().map_err(|_| corrupt())?) as usize;
    let offset = 42 + folder;
    raw[offset..offset + 4].copy_from_slice(&horizon.to_be_bytes());
    WorkshopProgress::decode(&raw)
}

struct Native {
    manifest: WorkshopManifest,
    next: Option<WorkshopProgress>,
    progress_calls: usize,
    other_calls: usize,
    fences: usize,
    after: Option<Box<dyn FnMut()>>,
    change_manifest: bool,
}
impl Native {
    fn new(next: WorkshopProgress) -> Result<Self> {
        let cap = capture()?;
        Ok(Self {
            manifest: WorkshopManifest {
                endpoint: SocketAddr::from(([127, 0, 0, 1], 5000)),
                fortress: cap.fortress().clone(),
                generation: cap.generation(),
                df_version: "test-df".into(),
                dfhack_version: "test-dfhack".into(),
            },
            next: Some(next),
            progress_calls: 0,
            other_calls: 0,
            fences: 0,
            after: None,
            change_manifest: false,
        })
    }
    fn forbidden<T>(&mut self) -> Result<T> {
        self.other_calls += 1;
        Err(denied("only original progress may be read"))
    }
}
impl WorkshopSource for Native {
    fn manifest(&self) -> &WorkshopManifest {
        &self.manifest
    }
    fn native_summary(&self) -> WorkshopNativeSummary {
        WorkshopNativeSummary {
            unresolved: false,
            retained_records: 0,
        }
    }
    fn fence(&mut self) {
        self.fences += 1;
    }
    fn observe(
        &mut self,
        _: WorkshopSelection,
        _: &OperationContext,
        _: Duration,
    ) -> Result<WorkshopCapture> {
        self.forbidden()
    }
    fn prepare(
        &mut self,
        _: &WorkshopPlan,
        _: &OperationContext,
        _: Duration,
    ) -> Result<WorkshopPreparation> {
        self.forbidden()
    }
    fn commit(
        &mut self,
        _: WorkshopDispatch<'_>,
        _: &OperationContext,
        _: Duration,
    ) -> Result<WorkshopRecord> {
        self.forbidden()
    }
    fn query(
        &mut self,
        _: &WorkshopPlan,
        _: &OperationContext,
        _: Duration,
    ) -> Result<Option<WorkshopRecord>> {
        self.forbidden()
    }
    fn cancel(
        &mut self,
        _: &WorkshopPlan,
        _: &OperationContext,
        _: Duration,
    ) -> Result<WorkshopRecord> {
        self.forbidden()
    }
    fn progress(
        &mut self,
        building: u32,
        item: u32,
        c: &OperationContext,
        _: Duration,
    ) -> Result<WorkshopProgress> {
        self.progress_calls += 1;
        assert_eq!((building, item), (70, 42));
        assert!(c.budget.max_bytes <= RPC_RESERVE);
        assert!(
            c.grants
                .iter()
                .all(|g| matches!(g.capability, Capability::Query | Capability::Observe))
        );
        if let Some(after) = &mut self.after {
            after();
        }
        if self.change_manifest {
            self.manifest.generation += 1;
        }
        self.next
            .take()
            .ok_or_else(|| error(ErrorCode::AdapterUnavailable, "lost progress reply"))
    }
}

#[test]
fn production_origin_reads_exact_original_and_fresh_completion_without_donor_writes() -> Result<()>
{
    let mut fixture = Fixture::new("placed", &[])?;
    let bytes = fixture.bytes();
    let writes = fixture.writes();
    let mut origin = fixture.origin()?;
    let descriptor = origin.canonical_bytes().to_vec();
    let witness = origin.witness();
    assert_eq!(origin.plan(), &plan()?);
    assert_eq!(origin.receipt(), record("placed")?.receipt());
    let now = sample("progress_complete", 13000, 100)?;
    fixture.context.anchor.tick = GameTick(now.tick());
    let mut source = Native::new(now.clone())?;
    let ready = fixture.refresh(&mut origin, &mut source)?;
    assert_eq!(ready.progress(), &now);
    assert_eq!(ready.anchor(), fixture.context.anchor);
    assert_eq!(ready.origin_witness(), witness);
    assert_ne!(ready.witness(), Digest32::ZERO);
    assert_eq!(
        (source.progress_calls, source.other_calls, source.fences),
        (1, 0, 1)
    );
    assert_eq!(
        (origin.high_tick(), origin.high_next_building_id()),
        (13000, 100)
    );
    assert_eq!(origin.canonical_bytes(), descriptor);
    assert_eq!(origin.witness(), witness);
    assert_eq!(fixture.bytes(), bytes);
    assert_eq!(fixture.writes(), writes);
    assert!(!fixture.journal.has_permit(plan()?.key()));
    Ok(())
}

#[test]
fn production_origin_historical_completion_never_substitutes_for_current_building() -> Result<()> {
    let historical = progress("progress_complete")?;
    let now = historical.tick() + 1;
    let mut fixture = Fixture::new("placed", &[historical])?;
    let mut origin = fixture.origin()?;
    let mut source = Native::new(sample("progress_absent_continuous", now, 100)?)?;
    fixture.context.anchor.tick = GameTick(now);
    assert!(fixture.refresh(&mut origin, &mut source).is_err());
    assert_eq!(source.progress_calls, 1);
    assert!(origin.is_fenced());
    let mut restored = Native::new(sample("progress_complete", now, 100)?)?;
    assert!(fixture.refresh(&mut origin, &mut restored).is_err());
    assert_eq!(restored.progress_calls, 0);
    Ok(())
}

#[test]
fn production_origin_incomplete_read_can_later_finish_without_renewing_original() -> Result<()> {
    let mut fixture = Fixture::new("placed", &[])?;
    let mut origin = fixture.origin()?;
    let identity = origin.witness();
    fixture.context.anchor.tick = GameTick(13000);
    let mut source = Native::new(sample("progress_stage0", 13000, 100)?)?;
    assert_eq!(
        fixture
            .refresh(&mut origin, &mut source)
            .err()
            .map(|e| e.code),
        Some(ErrorCode::Conflict)
    );
    assert!(!origin.is_fenced());
    fixture.context.anchor.tick = GameTick(13001);
    let mut source = Native::new(sample("progress_complete", 13001, 100)?)?;
    assert!(fixture.refresh(&mut origin, &mut source).is_ok());
    assert_eq!(origin.witness(), identity);
    Ok(())
}

#[test]
fn production_origin_refuses_unresolved_receipts_and_wrong_original_selection() -> Result<()> {
    for phase in ["prepared", "indeterminate", "refused", "cancelled"] {
        let mut fixture = Fixture::new(phase, &[])?;
        let before = fixture.bytes();
        assert!(fixture.origin().is_err(), "{phase}");
        assert_eq!(fixture.bytes(), before);
    }
    let mut fixture = Fixture::new("placed", &[])?;
    assert!(
        fixture
            .journal
            .production_origin(
                &mut fixture.ack,
                "another-original",
                plan()?.digest(),
                &fixture.context
            )
            .is_err()
    );
    assert!(
        fixture
            .journal
            .production_origin(
                &mut fixture.ack,
                plan()?.key(),
                Digest32::of_bytes(b"wrong"),
                &fixture.context
            )
            .is_err()
    );
    Ok(())
}

#[test]
fn production_origin_replays_all_retained_allocator_horizons() -> Result<()> {
    let high = sample("progress_stage0", 13000, 100)?;
    let lower = sample("progress_complete", 13001, 90)?;
    let mut regressed = Fixture::new("placed", &[high.clone(), lower])?;
    assert!(regressed.origin().is_err());
    let mut fixture = Fixture::new("placed", &[high])?;
    let mut origin = fixture.origin()?;
    assert_eq!(origin.high_next_building_id(), 100);
    fixture.context.anchor.tick = GameTick(13001);
    let mut source = Native::new(sample("progress_complete", 13001, 99)?)?;
    assert!(fixture.refresh(&mut origin, &mut source).is_err());
    assert!(origin.is_fenced());
    assert_eq!(origin.high_next_building_id(), 100);
    Ok(())
}

#[test]
fn production_origin_refuses_retained_generation_break_and_identity_resurrection() -> Result<()> {
    let mut raw = vector("progress_stage0")?;
    raw[8..16].copy_from_slice(&(capture()?.generation() + 1).to_be_bytes());
    let mut changed = Fixture::new("placed", &[WorkshopProgress::decode(&raw)?])?;
    assert!(changed.origin().is_err());
    let mut absent = Fixture::new(
        "placed",
        &[
            sample("progress_absent_continuous", 13000, 100)?,
            sample("progress_complete", 13001, 100)?,
        ],
    )?;
    assert!(absent.origin().is_err());
    Ok(())
}

#[test]
fn production_origin_consumes_failed_tick_join_before_any_retry() -> Result<()> {
    let mut fixture = Fixture::new("placed", &[])?;
    let mut origin = fixture.origin()?;
    fixture.context.anchor.tick = GameTick(13000);
    let mut source = Native::new(sample("progress_complete", 13010, 120)?)?;
    assert!(fixture.refresh(&mut origin, &mut source).is_err());
    assert!(!origin.is_fenced());
    assert_eq!(
        (origin.high_tick(), origin.high_next_building_id()),
        (13010, 120)
    );
    let mut old = Native::new(sample("progress_complete", 13000, 100)?)?;
    assert!(fixture.refresh(&mut origin, &mut old).is_err());
    assert_eq!(old.progress_calls, 0);
    fixture.context.anchor.tick = GameTick(13010);
    let mut current = Native::new(sample("progress_complete", 13010, 120)?)?;
    assert!(fixture.refresh(&mut origin, &mut current).is_ok());
    Ok(())
}

#[test]
fn production_origin_consumes_frontier_before_custody_or_authority_failure() -> Result<()> {
    for corrupt_ack in [false, true] {
        let mut fixture = Fixture::new("placed", &[])?;
        let mut origin = fixture.origin()?;
        fixture.context.anchor.tick = GameTick(13000);
        let mut source = Native::new(sample("progress_complete", 13010, 120)?)?;
        if corrupt_ack {
            let disk = fixture.ack_disk.clone();
            source.after = Some(Box::new(move || disk.borrow_mut().bytes[0] ^= 1));
        } else {
            for grant in &mut fixture.context.grants {
                grant.expires_at_tick = Some(GameTick(13005));
            }
        }
        assert!(fixture.refresh(&mut origin, &mut source).is_err());
        assert_eq!(
            (origin.high_tick(), origin.high_next_building_id()),
            (13010, 120)
        );
        if corrupt_ack {
            assert!(origin.is_fenced());
        }
        assert_eq!(source.other_calls, 0);
    }
    Ok(())
}

#[test]
fn production_origin_consumes_absence_before_later_authority_failure() -> Result<()> {
    let mut fixture = Fixture::new("placed", &[])?;
    let mut origin = fixture.origin()?;
    fixture.context.anchor.tick = GameTick(13000);
    for grant in &mut fixture.context.grants {
        grant.expires_at_tick = Some(GameTick(13005));
    }
    let mut source = Native::new(sample("progress_absent_continuous", 13010, 120)?)?;
    assert!(fixture.refresh(&mut origin, &mut source).is_err());
    assert!(origin.is_fenced());
    assert_eq!(origin.high_tick(), 13010);
    fixture.context.anchor.tick = GameTick(13010);
    for grant in &mut fixture.context.grants {
        grant.expires_at_tick = None;
    }
    let mut restored = Native::new(sample("progress_complete", 13010, 120)?)?;
    assert!(fixture.refresh(&mut origin, &mut restored).is_err());
    assert_eq!(restored.progress_calls, 0);
    Ok(())
}

#[test]
fn production_origin_native_errors_and_changed_manifest_cannot_restore_old_source() -> Result<()> {
    for changed in [false, true] {
        let mut fixture = Fixture::new("placed", &[])?;
        let mut origin = fixture.origin()?;
        fixture.context.anchor.tick = GameTick(13000);
        let mut source = Native::new(sample("progress_complete", 13000, 100)?)?;
        if changed {
            source.change_manifest = true;
        } else {
            source.next = None;
        }
        assert!(fixture.refresh(&mut origin, &mut source).is_err());
        assert!(origin.is_fenced());
        if changed {
            assert_eq!(origin.high_generation(), capture()?.generation() + 1);
        }
        let mut original = Native::new(sample("progress_complete", 13000, 100)?)?;
        assert!(fixture.refresh(&mut origin, &mut original).is_err());
        assert_eq!(original.progress_calls, 0);
    }
    Ok(())
}

#[test]
fn production_origin_requires_full_budget_and_observe_before_native_contact() -> Result<()> {
    let mut fixture = Fixture::new("placed", &[])?;
    let reads = fixture.reads();
    fixture.context.budget.max_bytes = IMPORT_WORK_BYTES - 1;
    assert_eq!(
        fixture.origin().err().map(|e| e.code),
        Some(ErrorCode::BudgetExceeded)
    );
    assert_eq!(fixture.reads(), reads);
    fixture.context.budget.max_bytes = IMPORT_WORK_BYTES;
    let mut origin = fixture.origin()?;
    fixture.context.anchor.tick = GameTick(13000);
    let mut source = Native::new(sample("progress_complete", 13000, 100)?)?;
    fixture.context.budget.max_bytes = REFRESH_WORK_BYTES - 1;
    let reads = fixture.reads();
    assert_eq!(
        fixture
            .refresh(&mut origin, &mut source)
            .err()
            .map(|e| e.code),
        Some(ErrorCode::BudgetExceeded)
    );
    assert_eq!(fixture.reads(), reads);
    assert_eq!(source.progress_calls, 0);
    fixture.context.budget.max_bytes = REFRESH_WORK_BYTES;
    fixture
        .context
        .grants
        .retain(|g| g.capability == Capability::Query);
    assert!(fixture.refresh(&mut origin, &mut source).is_err());
    assert_eq!(source.progress_calls, 0);
    origin.verify(&mut fixture.journal, &mut fixture.ack, &fixture.context)?;
    Ok(())
}

#[test]
fn production_origin_local_verification_rejects_substitution_and_other_session() -> Result<()> {
    let mut fixture = Fixture::new("placed", &[])?;
    let mut origin = fixture.origin()?;
    let before = fixture.bytes();
    let writes = fixture.writes();
    origin.verify(&mut fixture.journal, &mut fixture.ack, &fixture.context)?;
    assert_eq!(fixture.bytes(), before);
    assert_eq!(fixture.writes(), writes);
    let mut other = fixture.context.clone();
    other.session_id = SessionId::new(72);
    assert!(
        origin
            .verify(&mut fixture.journal, &mut fixture.ack, &other)
            .is_err()
    );
    other = fixture.context.clone();
    other.anchor.cursor.epoch += 1;
    assert!(
        origin
            .verify(&mut fixture.journal, &mut fixture.ack, &other)
            .is_err()
    );
    fixture.journal_disk.borrow_mut().bytes[0] ^= 1;
    assert!(
        origin
            .verify(&mut fixture.journal, &mut fixture.ack, &fixture.context)
            .is_err()
    );
    assert!(origin.is_fenced());
    Ok(())
}

#[test]
fn production_origin_requires_offline_ack_and_rejects_valid_rolled_back_journal_prefix()
-> Result<()> {
    let mut fixture = Fixture::new("placed", &[])?;
    fixture.ack = WorkshopAcknowledgements::open(
        Memory {
            disk: fixture.ack_disk.clone(),
            position: 0,
        },
        &fixture.context,
        WorkshopMode::Control,
        false,
        Some(fixture.journal.pin()),
    )?;
    assert!(fixture.origin().is_err());

    let mut fixture = Fixture::new("placed", &[])?;
    let raw = fixture.journal.raw.clone();
    let mut r = Reader(&raw);
    r.take(8)?;
    r.field(MAX_BINDING_BYTES)?;
    r.take(64)?;
    let mut last_frame = raw.len() - r.0.len();
    while !r.0.is_empty() {
        last_frame = raw.len() - r.0.len();
        r.take(8)?;
        let length = r.number()? as usize;
        r.take(40 + length + 40)?;
    }
    fixture.journal_disk.borrow_mut().bytes.truncate(last_frame);
    fixture.journal = WorkshopJournal::open(
        Memory {
            disk: fixture.journal_disk.clone(),
            position: 0,
        },
        &fixture.context,
        WorkshopMode::Offline,
        Some(fixture.journal.binding().clone()),
        None,
        None,
    )?;
    assert!(fixture.origin().is_err());
    Ok(())
}

#[test]
fn production_origin_recovery_floors_only_narrow_and_preserve_sticky_fences() -> Result<()> {
    let mut fixture = Fixture::new("placed", &[])?;
    let mut origin = fixture.origin()?;
    let descriptor = origin.canonical_bytes().to_vec();
    let generation = origin.binding().generation();
    fixture.context.anchor.tick = GameTick(13000);
    origin.retain_recovery_floor(13000, generation, 100, false, &fixture.context)?;
    assert_eq!(origin.canonical_bytes(), descriptor);
    for (tick, native_generation, horizon) in [
        (12999, generation, 100),
        (13000, generation, 99),
        (13000, 0, 100),
        (13000, u64::MAX, 100),
        (u64::MAX, generation, 100),
        (13000, generation, u32::MAX),
    ] {
        assert!(
            origin
                .retain_recovery_floor(tick, native_generation, horizon, false, &fixture.context)
                .is_err()
        );
    }
    assert_eq!(
        (origin.high_tick(), origin.high_next_building_id()),
        (13000, 100)
    );
    let mut source = Native::new(sample("progress_complete", 13000, 100)?)?;
    assert!(fixture.refresh(&mut origin, &mut source).is_ok());
    origin.retain_recovery_floor(13001, generation + 1, 101, false, &fixture.context)?;
    assert!(origin.is_fenced());
    assert!(
        origin
            .retain_recovery_floor(13001, generation, 101, false, &fixture.context)
            .is_err()
    );
    origin.retain_recovery_floor(13001, generation + 1, 101, false, &fixture.context)?;
    assert!(origin.is_fenced());
    assert_eq!(origin.canonical_bytes(), descriptor);

    let mut fixture = Fixture::new("placed", &[])?;
    let mut origin = fixture.origin()?;
    origin.retain_recovery_floor(13000, generation, 100, true, &fixture.context)?;
    origin.retain_recovery_floor(13000, generation, 100, false, &fixture.context)?;
    assert!(origin.is_fenced());
    let mut other = fixture.context.clone();
    other.session_id = SessionId::new(999);
    assert!(
        origin
            .retain_recovery_floor(14000, generation, 200, true, &other)
            .is_err()
    );
    assert_eq!(origin.high_tick(), 13000);
    Ok(())
}

#[test]
fn production_origin_host_fence_survives_local_verification_and_recovery_without_native_contact()
-> Result<()> {
    let mut fixture = Fixture::new("placed", &[])?;
    let mut origin = fixture.origin()?;
    let identity = origin.witness();
    let bytes = fixture.bytes();
    let writes = fixture.writes();
    origin.fence();
    origin.fence();
    assert!(origin.is_fenced());
    let mut query = fixture.context.clone();
    query
        .grants
        .retain(|grant| grant.capability == Capability::Query);
    origin.verify(&mut fixture.journal, &mut fixture.ack, &query)?;
    origin.retain_recovery_floor(13000, capture()?.generation(), 100, false, &query)?;
    assert!(origin.is_fenced());
    fixture.context.anchor.tick = GameTick(13000);
    let mut source = Native::new(sample("progress_complete", 13000, 100)?)?;
    assert!(fixture.refresh(&mut origin, &mut source).is_err());
    assert_eq!((source.progress_calls, source.other_calls), (0, 0));
    assert_eq!(origin.witness(), identity);
    assert_eq!(fixture.bytes(), bytes);
    assert_eq!(fixture.writes(), writes);
    Ok(())
}

#[test]
fn production_origin_operator_binding_pins_roundtrip_and_refuse_invalid_sources() -> Result<()> {
    let before = capture()?;
    let endpoint = SocketAddr::from(([127, 0, 0, 1], 5000));
    let binding = WorkshopBinding::from_source_pins(
        endpoint,
        before.fortress().clone(),
        before.generation(),
        before.dimensions(),
        "test-df",
        "test-dfhack",
    )?;
    assert_eq!(WorkshopBinding::decode(&binding.encode())?, binding);
    assert_eq!(
        WorkshopBinding::new(endpoint, "test-df", "test-dfhack", &before)?,
        binding
    );
    for generation in [0, u64::MAX] {
        assert!(
            WorkshopBinding::from_source_pins(
                endpoint,
                before.fortress().clone(),
                generation,
                before.dimensions(),
                "test-df",
                "test-dfhack"
            )
            .is_err()
        );
    }
    for dimensions in [[0, 1, 1], [32769, 1, 1]] {
        assert!(
            WorkshopBinding::from_source_pins(
                endpoint,
                before.fortress().clone(),
                1,
                dimensions,
                "test-df",
                "test-dfhack"
            )
            .is_err()
        );
    }
    for endpoint in [
        SocketAddr::from(([0, 0, 0, 0], 5000)),
        SocketAddr::from(([127, 0, 0, 1], 0)),
        SocketAddr::from(([0u16, 0, 0, 0, 0, 0, 0, 1], 5000)),
    ] {
        assert!(
            WorkshopBinding::from_source_pins(
                endpoint,
                before.fortress().clone(),
                1,
                before.dimensions(),
                "test-df",
                "test-dfhack"
            )
            .is_err()
        );
    }
    assert!(
        WorkshopBinding::from_source_pins(
            endpoint,
            before.fortress().clone(),
            1,
            before.dimensions(),
            "",
            "test-dfhack"
        )
        .is_err()
    );
    Ok(())
}
