use super::*;
use crate::build_placement::session::BuildSession;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, ObservationCursor, RequestId, StateAnchor, WorkBudget,
};
use std::cell::RefCell;
use std::io::{self, Read, Seek, Write};
use std::net::SocketAddr;
use std::rc::Rc;

fn fixture(name: &str) -> Result<Vec<u8>> {
    let needle = format!("\"{name}\": \"");
    let text =
        include_str!("../../../../../bridge/common/tests/fixtures/build_placement_v1_19.json");
    let start = text.find(&needle).ok_or_else(corrupt)? + needle.len();
    let encoded = text[start..].split('"').next().ok_or_else(corrupt)?;
    encoded
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let hex = std::str::from_utf8(pair).map_err(|_| corrupt())?;
            u8::from_str_radix(hex, 16).map_err(|_| corrupt())
        })
        .collect()
}
fn plan() -> Result<BuildPlan> {
    BuildPlan::new("golden", BuildCapture::decode(&fixture("capture")?)?)
}
fn native(name: &str) -> Result<BuildRecord> {
    BuildRecord::decode(&fixture(name)?)
}
fn binding() -> Result<BuildBinding> {
    BuildBinding::new(
        SocketAddr::from(([127, 0, 0, 1], 5000)),
        "df",
        "dfhack",
        plan()?.before(),
    )
}
fn context() -> Result<OperationContext> {
    let plan = plan()?;
    let fortress_id = plan.before().fortress_id();
    Ok(OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: StateAnchor {
            fortress_id,
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(plan.before().tick()),
            state_hash: plan.before().witness(),
        },
        budget: WorkBudget {
            max_wall_millis: 60_000,
            max_bytes: 64 * 1024 * 1024,
            ..WorkBudget::CONSERVATIVE_DEFAULT
        },
        grants: [
            Capability::Query,
            Capability::Observe,
            Capability::Plan,
            Capability::Construct,
        ]
        .into_iter()
        .map(|capability| CapabilityGrant {
            capability,
            max_risk: RiskTier::Guarded,
            scope: CapabilityScope {
                fortress_id: Some(fortress_id),
                ..CapabilityScope::default()
            },
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
        cancellation_requested: false,
    })
}
#[derive(Default)]
struct Disk {
    bytes: Vec<u8>,
    syncs: usize,
    synced_length: usize,
    fail_sync: Option<usize>,
    partial: bool,
    invalid: bool,
    revoke_at_dispatch: bool,
    revoked: bool,
}
#[derive(Clone, Default)]
struct Memory {
    disk: Rc<RefCell<Disk>>,
    position: usize,
}
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let disk = self.disk.borrow();
        let start = self.position.min(disk.bytes.len());
        let count = out.len().min(disk.bytes.len() - start);
        out[..count].copy_from_slice(&disk.bytes[start..start + count]);
        self.position += count;
        Ok(count)
    }
}
impl Seek for Memory {
    fn seek(&mut self, offset: SeekFrom) -> io::Result<u64> {
        let next = match offset {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::End(n) => self.disk.borrow().bytes.len() as i128 + i128::from(n),
            SeekFrom::Current(n) => self.position as i128 + i128::from(n),
        };
        self.position = usize::try_from(next).map_err(|_| io::Error::other("invalid seek"))?;
        Ok(self.position as u64)
    }
}
impl Write for Memory {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut disk = self.disk.borrow_mut();
        if disk.invalid || self.position != disk.bytes.len() {
            return Err(io::Error::other("custody or append failure"));
        }
        if disk.partial {
            disk.bytes.extend_from_slice(&bytes[..bytes.len().min(3)]);
            return Err(io::Error::other("injected partial write"));
        }
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
        let mut disk = self.disk.borrow_mut();
        disk.syncs += 1;
        if disk.fail_sync == Some(disk.syncs) {
            return Err(io::Error::other("injected sync failure"));
        }
        disk.synced_length = disk.bytes.len();
        if disk.revoke_at_dispatch && last_state(&disk.bytes) == Some(BuildState::DispatchStarted) {
            disk.revoked = true;
        }
        Ok(())
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("repair forbidden"))
    }
    fn validate_identity(&self) -> io::Result<()> {
        if self.disk.borrow().invalid {
            Err(io::Error::other("changed custody"))
        } else {
            Ok(())
        }
    }
}
fn last_state(raw: &[u8]) -> Option<BuildState> {
    let mut r = Reader(raw);
    r.take(8).ok()?;
    r.field(MAX_BINDING_BYTES).ok()?;
    r.take(64).ok()?;
    let mut state = None;
    while !r.0.is_empty() {
        r.take(8).ok()?;
        let n = r.number().ok()? as usize;
        r.take(40).ok()?;
        let body = r.take(n).ok()?;
        state = BuildState::decode(*body.first()?).ok();
        r.take(40).ok()?;
    }
    state
}
#[derive(Default)]
struct Calls {
    observed: usize,
    prepared: usize,
    committed: usize,
    queried: usize,
    cancelled: usize,
    fenced: usize,
}
struct Native {
    binding: BuildBinding,
    disk: Rc<RefCell<Disk>>,
    calls: Rc<RefCell<Calls>>,
    retained: Option<BuildRecord>,
    lose_commit: bool,
    indeterminate: bool,
    replayed: bool,
    change_observation: bool,
    later_tick: Option<u64>,
}
impl Native {
    fn new(memory: &Memory) -> Result<Self> {
        Ok(Self {
            binding: binding()?,
            disk: memory.disk.clone(),
            calls: Rc::new(RefCell::new(Calls::default())),
            retained: None,
            lose_commit: false,
            indeterminate: false,
            replayed: false,
            change_observation: false,
            later_tick: None,
        })
    }
    fn published(&self, state: BuildState) {
        let disk = self.disk.borrow();
        assert_eq!(last_state(&disk.bytes), Some(state));
        assert_eq!(disk.synced_length, disk.bytes.len());
    }
}
impl BuildSource for Native {
    fn binding(&self) -> &BuildBinding {
        &self.binding
    }
    fn native_summary(&self) -> BuildNativeSummary {
        BuildNativeSummary {
            unresolved: self
                .retained
                .as_ref()
                .is_some_and(|record| record.phase() == BuildPhase::Indeterminate),
            retained_records: u16::from(self.retained.is_some()),
        }
    }
    fn fence(&mut self) {
        self.calls.borrow_mut().fenced += 1;
    }
    fn observe(
        &mut self,
        _: BuildSelection,
        _: &OperationContext,
        _: Duration,
    ) -> Result<BuildCapture> {
        self.calls.borrow_mut().observed += 1;
        if let Some(tick) = self.later_tick {
            let mut bytes = fixture("capture")?;
            bytes[24..32].copy_from_slice(&tick.to_be_bytes());
            return BuildCapture::decode(&bytes);
        }
        if self.change_observation {
            plan()?.before().expected_after()
        } else {
            Ok(plan()?.before().clone())
        }
    }
    fn prepare(
        &mut self,
        _: &BuildPlan,
        _: &OperationContext,
        _: Duration,
    ) -> Result<BuildPreparation> {
        self.published(BuildState::Intent);
        self.calls.borrow_mut().prepared += 1;
        let record = native("prepared")?;
        self.retained = Some(record.clone());
        BuildPreparation::new(record, self.replayed)
    }
    fn commit(
        &mut self,
        dispatch: BuildDispatch<'_>,
        _: &OperationContext,
        _: Duration,
    ) -> Result<BuildRecord> {
        self.published(BuildState::DispatchStarted);
        assert_eq!(dispatch.plan(), &plan()?);
        assert_ne!(dispatch.journal_head(), Digest32::ZERO);
        self.calls.borrow_mut().committed += 1;
        let record = native(if self.indeterminate {
            "indeterminate"
        } else {
            "placed"
        })?;
        self.retained = Some(record.clone());
        if self.lose_commit {
            Err(error(ErrorCode::AdapterFailure, "injected lost reply"))
        } else {
            Ok(record)
        }
    }
    fn query(
        &mut self,
        _: &BuildPlan,
        _: &OperationContext,
        _: Duration,
    ) -> Result<Option<BuildRecord>> {
        self.calls.borrow_mut().queried += 1;
        Ok(self.retained.clone())
    }
    fn cancel(&mut self, _: &BuildPlan, _: &OperationContext, _: Duration) -> Result<BuildRecord> {
        self.published(BuildState::CancelRequested);
        self.calls.borrow_mut().cancelled += 1;
        let record = native("cancelled")?;
        self.retained = Some(record.clone());
        Ok(record)
    }
}
struct Guard {
    disk: Rc<RefCell<Disk>>,
}
impl BuildGuard for Guard {
    fn check(
        &mut self,
        _: BuildStage,
        _: &BuildBinding,
        _: Option<&BuildPlan>,
        _: BuildSelection,
        _: &OperationContext,
    ) -> Result<()> {
        if self.disk.borrow().revoked {
            return Err(error(ErrorCode::CapabilityDenied, "runtime revoked"));
        }
        Ok(())
    }
}
fn setup() -> Result<(
    Memory,
    BuildJournal<Memory>,
    Native,
    Guard,
    OperationContext,
)> {
    let memory = Memory::default();
    let context = context()?;
    let journal = BuildJournal::open(
        memory.clone(),
        &context,
        BuildMode::Control,
        Some(binding()?),
        Some([7; 32]),
    )?;
    let source = Native::new(&memory)?;
    let guard = Guard {
        disk: memory.disk.clone(),
    };
    Ok((memory, journal, source, guard, context))
}

#[test]
fn intent_and_dispatch_are_synced_before_effect_and_terminal_before_ack() -> Result<()> {
    let (memory, mut journal, mut source, mut guard, context) = setup()?;
    let plan = plan()?;
    let prepared = journal.prepare(&mut source, &plan, &context, &mut guard)?;
    assert_eq!(prepared.state(), BuildState::Prepared);
    assert!(journal.has_permit());
    let placed = journal.commit(&mut source, plan.key(), plan.digest(), &context, &mut guard)?;
    assert!(!placed.unresolved());
    assert!(placed.dispatch_started());
    assert_eq!(memory.disk.borrow().syncs, 5);
    assert_eq!(
        journal.commit(&mut source, plan.key(), plan.digest(), &context, &mut guard)?,
        placed
    );
    assert_eq!(source.calls.borrow().committed, 1);
    assert_eq!(
        memory.disk.borrow().synced_length,
        memory.disk.borrow().bytes.len()
    );
    Ok(())
}

#[test]
fn every_sync_failure_stops_next_native_edge_and_complete_history_replays() -> Result<()> {
    for failure in 2..=5 {
        let (memory, mut journal, mut source, mut guard, context) = setup()?;
        let plan = plan()?;
        memory.disk.borrow_mut().fail_sync = Some(failure);
        let prepared = journal.prepare(&mut source, &plan, &context, &mut guard);
        if prepared.is_ok() {
            assert!(
                journal
                    .commit(&mut source, plan.key(), plan.digest(), &context, &mut guard)
                    .is_err()
            );
        } else {
            assert!(failure <= 3);
        }
        assert!(journal.is_fenced());
        assert_eq!(source.calls.borrow().prepared, usize::from(failure >= 3));
        assert_eq!(source.calls.borrow().committed, usize::from(failure == 5));
        drop(journal);
        memory.disk.borrow_mut().fail_sync = None;
        let recovered = BuildJournal::open(
            memory.clone(),
            &context,
            BuildMode::Control,
            Some(binding()?),
            None,
        )?;
        assert!(!recovered.has_permit());
        assert_eq!(
            recovered
                .entries
                .get(plan.key())
                .ok_or_else(corrupt)?
                .unresolved(),
            failure != 5
        );
    }
    Ok(())
}

#[test]
fn reopened_preparation_cannot_dispatch_but_query_and_cancel_remain_available() -> Result<()> {
    let (memory, mut journal, mut source, mut guard, context) = setup()?;
    let plan = plan()?;
    journal.prepare(&mut source, &plan, &context, &mut guard)?;
    drop(journal);
    let mut reopened =
        BuildJournal::open(memory, &context, BuildMode::Control, Some(binding()?), None)?;
    assert!(
        reopened
            .commit(&mut source, plan.key(), plan.digest(), &context, &mut guard)
            .is_err()
    );
    let tracking = reopened.recover(
        &mut source,
        plan.key(),
        plan.digest(),
        false,
        &context,
        &mut guard,
    )?;
    assert_eq!(tracking.state(), BuildState::Tracking);
    assert!(!reopened.has_permit());
    let cancelled = reopened.recover(
        &mut source,
        plan.key(),
        plan.digest(),
        true,
        &context,
        &mut guard,
    )?;
    assert_eq!(
        cancelled.native().map(BuildRecord::phase),
        Some(BuildPhase::Cancelled)
    );
    assert!(cancelled.cancel_requested());
    assert!(!cancelled.dispatch_started());
    assert_eq!(source.calls.borrow().committed, 0);
    Ok(())
}

#[test]
fn query_only_recovery_can_retire_preparation_after_placement_grants_are_revoked() -> Result<()> {
    let (memory, mut journal, mut source, mut guard, mut context) = setup()?;
    let plan = plan()?;
    journal.prepare(&mut source, &plan, &context, &mut guard)?;
    drop(journal);
    context
        .grants
        .retain(|grant| grant.capability == Capability::Query);
    let mut reopened = BuildJournal::open(memory, &context, BuildMode::Recover, None, None)?;
    let cancelled = reopened.recover(
        &mut source,
        plan.key(),
        plan.digest(),
        true,
        &context,
        &mut guard,
    )?;
    assert_eq!(
        cancelled.native().map(BuildRecord::phase),
        Some(BuildPhase::Cancelled)
    );
    assert_eq!(source.calls.borrow().committed, 0);
    assert!(
        reopened
            .prepare(&mut source, &plan, &context, &mut guard)
            .is_err()
    );
    Ok(())
}

#[test]
fn lost_reply_reconciles_the_original_record_without_recommit() -> Result<()> {
    let (memory, mut journal, mut source, mut guard, context) = setup()?;
    let plan = plan()?;
    source.lose_commit = true;
    journal.prepare(&mut source, &plan, &context, &mut guard)?;
    assert!(
        journal
            .commit(&mut source, plan.key(), plan.digest(), &context, &mut guard)
            .is_err()
    );
    assert!(journal.inventory(&context)?.pending().is_some());
    drop(journal);
    let mut reopened = BuildJournal::open(memory, &context, BuildMode::Recover, None, None)?;
    let placed = reopened.recover(
        &mut source,
        plan.key(),
        plan.digest(),
        false,
        &context,
        &mut guard,
    )?;
    assert!(!placed.unresolved());
    assert!(placed.dispatch_started());
    assert_eq!(source.calls.borrow().committed, 1);
    Ok(())
}

#[test]
fn immutable_indeterminate_record_keeps_cross_key_fence_without_more_native_calls() -> Result<()> {
    let (_, mut journal, mut source, mut guard, context) = setup()?;
    let plan = plan()?;
    source.indeterminate = true;
    journal.prepare(&mut source, &plan, &context, &mut guard)?;
    let unknown = journal.commit(&mut source, plan.key(), plan.digest(), &context, &mut guard)?;
    assert!(unknown.unresolved());
    assert!(!unknown.needs_reconciliation());
    let query_count = source.calls.borrow().queried;
    assert_eq!(
        journal.recover(
            &mut source,
            plan.key(),
            plan.digest(),
            false,
            &context,
            &mut guard
        )?,
        unknown
    );
    assert_eq!(source.calls.borrow().queried, query_count);
    let other = BuildPlan::new("new-key", plan.before().clone())?;
    assert!(
        journal
            .prepare(&mut source, &other, &context, &mut guard)
            .is_err()
    );
    assert_eq!(journal.inventory(&context)?.pending_count(), 1);
    Ok(())
}

#[test]
fn replayed_prepare_is_tracking_and_has_no_commit_permit() -> Result<()> {
    let (_, mut journal, mut source, mut guard, context) = setup()?;
    let plan = plan()?;
    source.replayed = true;
    let entry = journal.prepare(&mut source, &plan, &context, &mut guard)?;
    assert_eq!(entry.state(), BuildState::Tracking);
    assert!(!journal.has_permit());
    assert!(
        journal
            .commit(&mut source, plan.key(), plan.digest(), &context, &mut guard)
            .is_err()
    );
    assert_eq!(source.calls.borrow().committed, 0);
    Ok(())
}

#[test]
fn runtime_revocation_during_dispatch_sync_prevents_native_commit() -> Result<()> {
    let (memory, mut journal, mut source, mut guard, context) = setup()?;
    let plan = plan()?;
    journal.prepare(&mut source, &plan, &context, &mut guard)?;
    memory.disk.borrow_mut().revoke_at_dispatch = true;
    assert!(
        journal
            .commit(&mut source, plan.key(), plan.digest(), &context, &mut guard)
            .is_err()
    );
    assert_eq!(source.calls.borrow().committed, 0);
    assert!(!journal.has_permit());
    assert!(
        journal
            .get(plan.key(), plan.digest(), &context)?
            .dispatch_started()
    );
    Ok(())
}

#[test]
fn absent_native_key_and_generation_loss_preserve_pending_intent() -> Result<()> {
    let (memory, mut journal, mut source, mut guard, context) = setup()?;
    let plan = plan()?;
    journal.prepare(&mut source, &plan, &context, &mut guard)?;
    drop(journal);
    source.retained = None;
    source.binding = source
        .binding
        .recovery_generation(source.binding.generation() + 1)?;
    let mut reopened = BuildJournal::open(memory, &context, BuildMode::Recover, None, None)?;
    assert!(
        reopened
            .recover(
                &mut source,
                plan.key(),
                plan.digest(),
                false,
                &context,
                &mut guard
            )
            .is_err()
    );
    assert!(
        reopened
            .get(plan.key(), plan.digest(), &context)?
            .unresolved()
    );
    assert_eq!(source.calls.borrow().committed, 0);
    Ok(())
}

#[test]
fn corrupt_same_length_bytes_and_partial_tail_are_refused_without_repair() -> Result<()> {
    let (memory, mut journal, mut source, mut guard, context) = setup()?;
    let plan = plan()?;
    journal.prepare(&mut source, &plan, &context, &mut guard)?;
    let complete = memory.disk.borrow().bytes.clone();
    let last = memory.disk.borrow().bytes.len() - 1;
    memory.disk.borrow_mut().bytes[last] ^= 1;
    assert!(journal.inventory(&context).is_err());
    assert!(journal.is_fenced());
    drop(journal);
    assert!(BuildJournal::open(memory.clone(), &context, BuildMode::Offline, None, None).is_err());
    memory.disk.borrow_mut().bytes = complete.clone();
    memory.disk.borrow_mut().bytes.pop();
    let torn = memory.disk.borrow().bytes.clone();
    assert!(BuildJournal::open(memory.clone(), &context, BuildMode::Recover, None, None).is_err());
    assert_eq!(memory.disk.borrow().bytes, torn);
    Ok(())
}

#[test]
fn partial_intent_write_never_calls_prepare_and_cannot_reopen() -> Result<()> {
    let (memory, mut journal, mut source, mut guard, context) = setup()?;
    let plan = plan()?;
    memory.disk.borrow_mut().partial = true;
    assert!(
        journal
            .prepare(&mut source, &plan, &context, &mut guard)
            .is_err()
    );
    assert_eq!(source.calls.borrow().prepared, 0);
    drop(journal);
    assert!(BuildJournal::open(memory, &context, BuildMode::Recover, None, None).is_err());
    Ok(())
}

#[test]
fn offline_discovery_is_read_only_and_current_authority_cannot_be_replayed() -> Result<()> {
    let (memory, mut journal, mut source, mut guard, context) = setup()?;
    let plan = plan()?;
    journal.prepare(&mut source, &plan, &context, &mut guard)?;
    drop(journal);
    let syncs = memory.disk.borrow().syncs;
    let mut offline = BuildJournal::open(memory.clone(), &context, BuildMode::Offline, None, None)?;
    assert_eq!(offline.inventory(&context)?.pending_count(), 1);
    assert!(
        offline
            .recover(
                &mut source,
                plan.key(),
                plan.digest(),
                false,
                &context,
                &mut guard
            )
            .is_err()
    );
    assert_eq!(memory.disk.borrow().syncs, syncs);
    let mut other = context.clone();
    other.session_id = SessionId::new(2);
    assert!(offline.inventory(&other).is_err());
    other = context.clone();
    other.grants.clear();
    assert!(offline.inventory(&other).is_err());
    other = context.clone();
    other.grants[0].remaining_uses = Some(1);
    assert!(offline.inventory(&other).is_err());
    other = context;
    other.cancellation_requested = true;
    assert!(offline.inventory(&other).is_err());
    Ok(())
}

#[test]
fn stale_capture_and_insufficient_budget_stop_before_intent() -> Result<()> {
    let (memory, mut journal, mut source, mut guard, mut context) = setup()?;
    let plan = plan()?;
    source.change_observation = true;
    assert!(
        journal
            .prepare(&mut source, &plan, &context, &mut guard)
            .is_err()
    );
    assert_eq!(journal.inventory(&context)?.total_records(), 0);
    source.change_observation = false;
    context.budget.max_bytes = 1;
    assert!(
        journal
            .prepare(&mut source, &plan, &context, &mut guard)
            .is_err()
    );
    assert_eq!(source.calls.borrow().prepared, 0);
    assert_eq!(memory.disk.borrow().syncs, 1);
    Ok(())
}

#[test]
fn session_keeps_original_source_and_refresh_discards_preparation_only() -> Result<()> {
    let (memory, journal, source, mut guard, context) = setup()?;
    let plan = plan()?;
    let calls = source.calls.clone();
    let mut owner: BuildSession<Memory, Native> = BuildSession::new(journal, &context)?;
    let capture = owner.observe(
        plan.before().selection(),
        &context,
        |_, _, _| Ok(source),
        &mut guard,
    )?;
    owner.prepare(plan.key(), capture.witness(), &context, &mut guard)?;
    assert!(owner.has_preparation_connection());
    let second = Native::new(&memory)?;
    owner.observe(
        plan.before().selection(),
        &context,
        |_, _, _| Ok(second),
        &mut guard,
    )?;
    assert!(!owner.has_preparation_connection());
    assert!(
        owner
            .commit(plan.key(), plan.digest(), &context, &mut guard)
            .is_err()
    );
    assert_eq!(calls.borrow().committed, 0);
    assert!(owner.inventory(&context)?.pending().is_some());
    Ok(())
}

#[test]
fn session_success_and_offline_terminal_recovery_need_no_reconnection() -> Result<()> {
    let (memory, journal, source, mut guard, context) = setup()?;
    let plan = plan()?;
    let mut owner: BuildSession<Memory, Native> = BuildSession::new(journal, &context)?;
    let capture = owner.observe(
        plan.before().selection(),
        &context,
        |_, _, _| Ok(source),
        &mut guard,
    )?;
    owner.prepare(plan.key(), capture.witness(), &context, &mut guard)?;
    let placed = owner.commit(plan.key(), plan.digest(), &context, &mut guard)?;
    assert!(!placed.unresolved());
    drop(owner);
    let journal = BuildJournal::open(memory, &context, BuildMode::Offline, None, None)?;
    let mut owner: BuildSession<Memory, Native> = BuildSession::new(journal, &context)?;
    let history = owner.recover(
        plan.key(),
        plan.digest(),
        false,
        &context,
        |_, _, _| {
            Err(error(
                ErrorCode::InternalInvariantViolation,
                "offline must not connect",
            ))
        },
        &mut guard,
    )?;
    assert_eq!(history, placed);
    Ok(())
}

#[test]
fn valid_later_capture_does_not_allow_expired_grants_to_revive_after_failure() -> Result<()> {
    let (_, journal, mut source, mut guard, mut context) = setup()?;
    let plan = plan()?;
    let later = plan.before().tick() + 2;
    source.later_tick = Some(later);
    for grant in &mut context.grants {
        grant.expires_at_tick = Some(GameTick(later - 1));
    }
    let mut owner: BuildSession<Memory, Native> = BuildSession::new(journal, &context)?;
    assert!(
        owner
            .observe(
                plan.before().selection(),
                &context,
                |_, _, _| Ok(source),
                &mut guard
            )
            .is_err()
    );
    assert_eq!(owner.high_tick(), later);
    assert!(owner.selected().is_none());
    assert!(owner.inventory(&context).is_err());
    Ok(())
}

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod private {
    use super::*;
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Directory(PathBuf);
    impl Directory {
        fn create() -> io::Result<Self> {
            let path = std::env::temp_dir().join(format!(
                "dfmcp-build-journal-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path)?;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
            Ok(Self(path))
        }
        fn journal(&self) -> PathBuf {
            self.0.join("placement.journal")
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn private_storage_holds_exclusive_lock_and_offline_preserves_bytes()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let directory = Directory::create()?;
        let context = context()?;
        let mut owner = private_file::open_private_build(
            &directory.journal(),
            &context,
            BuildMode::Control,
            Some(binding()?),
        )?;
        assert!(
            private_file::open_private_build(
                &directory.journal(),
                &context,
                BuildMode::Offline,
                None
            )
            .is_err()
        );
        assert_eq!(owner.inventory(&context)?.total_records(), 0);
        drop(owner);
        let before = fs::read(directory.journal())?;
        let mut offline = private_file::open_private_build(
            &directory.journal(),
            &context,
            BuildMode::Offline,
            None,
        )?;
        assert_eq!(offline.inventory(&context)?.total_records(), 0);
        drop(offline);
        assert_eq!(fs::read(directory.journal())?, before);
        Ok(())
    }
    #[test]
    fn missing_empty_symlink_and_wrong_mode_are_not_initialized()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let directory = Directory::create()?;
        let context = context()?;
        let path = directory.journal();
        assert!(
            private_file::open_private_build(&path, &context, BuildMode::Offline, None).is_err()
        );
        assert!(!path.exists());
        fs::write(&path, [])?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        assert!(
            private_file::open_private_build(&path, &context, BuildMode::Control, Some(binding()?))
                .is_err()
        );
        assert_eq!(fs::metadata(&path)?.len(), 0);
        fs::remove_file(&path)?;
        let target = directory.0.join("target");
        fs::write(&target, [])?;
        symlink(&target, &path)?;
        assert!(
            private_file::open_private_build(&path, &context, BuildMode::Control, Some(binding()?))
                .is_err()
        );
        fs::remove_file(&path)?;
        fs::set_permissions(&directory.0, fs::Permissions::from_mode(0o750))?;
        assert!(
            private_file::open_private_build(&path, &context, BuildMode::Control, Some(binding()?))
                .is_err()
        );
        Ok(())
    }
    #[test]
    fn inode_replacement_and_hardlinks_fence_existing_owner()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let directory = Directory::create()?;
        let context = context()?;
        let path = directory.journal();
        let mut owner = private_file::open_private_build(
            &path,
            &context,
            BuildMode::Control,
            Some(binding()?),
        )?;
        fs::hard_link(&path, directory.0.join("link"))?;
        assert!(owner.inventory(&context).is_err());
        drop(owner);
        fs::remove_file(directory.0.join("link"))?;
        let mut owner =
            private_file::open_private_build(&path, &context, BuildMode::Offline, None)?;
        let bytes = fs::read(&path)?;
        fs::rename(&path, directory.0.join("old"))?;
        fs::write(&path, bytes)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        assert!(owner.inventory(&context).is_err());
        Ok(())
    }
}
