//! Actual journal/codec tests with injected native replies and storage faults.
use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, ObservationCursor, RequestId, StateAnchor, WorkBudget,
};
use std::cell::RefCell;
use std::io::{self, Cursor, Read, Seek, Write};
use std::net::SocketAddr;
use std::rc::Rc;

fn fixture(name: &str) -> Result<Vec<u8>> {
    let text =
        include_str!("../../../../../bridge/common/tests/fixtures/build_placement_v1_19.json");
    let needle = format!("\"{name}\": \"");
    let start = text.find(&needle).ok_or_else(corrupt)? + needle.len();
    let hex = text[start..].split('"').next().ok_or_else(corrupt)?;
    hex.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            u8::from_str_radix(std::str::from_utf8(pair).map_err(|_| corrupt())?, 16)
                .map_err(|_| corrupt())
        })
        .collect()
}
fn record(name: &str) -> Result<BuildRecord> {
    BuildRecord::decode(&fixture(name)?)
}
fn context() -> Result<OperationContext> {
    let record = record("prepared")?;
    Ok(OperationContext {
        session_id: SessionId::new(11),
        request_id: RequestId::new(12),
        anchor: StateAnchor {
            fortress_id: record.plan().before().fortress_id(),
            tick: GameTick(record.plan().before().tick()),
            cursor: ObservationCursor::ORIGIN,
            state_hash: record.plan().before().witness(),
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
            scope: CapabilityScope::default(),
            max_risk: RiskTier::Guarded,
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
        cancellation_requested: false,
    })
}
#[derive(Default)]
struct Disk {
    cursor: Cursor<Vec<u8>>,
    syncs: usize,
    fail_sync: bool,
    partial: bool,
    invalid: bool,
}
#[derive(Clone, Default)]
struct Memory(Rc<RefCell<Disk>>);
impl Read for Memory {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.0.borrow_mut().cursor.read(bytes)
    }
}
impl Seek for Memory {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.0.borrow_mut().cursor.seek(position)
    }
}
impl Write for Memory {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut disk = self.0.borrow_mut();
        if disk.cursor.position() != disk.cursor.get_ref().len() as u64 {
            return Err(io::Error::other("append only"));
        }
        if disk.partial {
            disk.cursor.write_all(&bytes[..bytes.len().min(3)])?;
            return Err(io::Error::other("partial append"));
        }
        disk.cursor.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        let mut disk = self.0.borrow_mut();
        disk.syncs += 1;
        if disk.fail_sync {
            Err(io::Error::other("sync failed"))
        } else {
            Ok(())
        }
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("repair forbidden"))
    }
    fn validate_identity(&self) -> io::Result<()> {
        if self.0.borrow().invalid {
            Err(io::Error::other("custody changed"))
        } else {
            Ok(())
        }
    }
}
struct Guard {
    deny: bool,
    calls: usize,
}
impl BuildGuard for Guard {
    fn check(
        &mut self,
        stage: BuildStage,
        _: &BuildBinding,
        _: Option<&BuildPlan>,
        _: BuildSelection,
        _: &OperationContext,
    ) -> Result<()> {
        self.calls += 1;
        check(stage == BuildStage::Query)?;
        if self.deny {
            Err(error(ErrorCode::CapabilityDenied, "revoked"))
        } else {
            Ok(())
        }
    }
}
struct Native {
    binding: BuildBinding,
    reply: Option<BuildRecord>,
    queries: usize,
    fenced: bool,
    forbidden: usize,
    changed_after_query: bool,
    disk: Memory,
    corrupt_after_query: bool,
}
impl BuildSource for Native {
    fn binding(&self) -> &BuildBinding {
        &self.binding
    }
    fn native_summary(&self) -> BuildNativeSummary {
        BuildNativeSummary {
            unresolved: self
                .reply
                .as_ref()
                .is_some_and(|r| r.phase() == BuildPhase::Indeterminate),
            retained_records: u16::from(self.reply.is_some()),
        }
    }
    fn fence(&mut self) {
        self.fenced = true;
    }
    fn observe(
        &mut self,
        _: BuildSelection,
        _: &OperationContext,
        _: Duration,
    ) -> Result<BuildCapture> {
        self.forbidden += 1;
        Err(corrupt())
    }
    fn prepare(
        &mut self,
        _: &BuildPlan,
        _: &OperationContext,
        _: Duration,
    ) -> Result<BuildPreparation> {
        self.forbidden += 1;
        Err(corrupt())
    }
    fn commit(
        &mut self,
        _: BuildDispatch<'_>,
        _: &OperationContext,
        _: Duration,
    ) -> Result<BuildRecord> {
        self.forbidden += 1;
        Err(corrupt())
    }
    fn cancel(&mut self, _: &BuildPlan, _: &OperationContext, _: Duration) -> Result<BuildRecord> {
        self.forbidden += 1;
        Err(corrupt())
    }
    fn query(
        &mut self,
        _: &BuildPlan,
        _: &OperationContext,
        _: Duration,
    ) -> Result<Option<BuildRecord>> {
        check(!self.fenced)?;
        self.queries += 1;
        if self.changed_after_query {
            self.binding = self
                .binding
                .recovery_generation(self.binding.generation() + 1)?;
        }
        if self.corrupt_after_query {
            self.disk.0.borrow_mut().invalid = true;
        }
        Ok(self.reply.clone())
    }
}
fn setup() -> Result<(
    Memory,
    BuildJournal<Memory>,
    Native,
    Guard,
    OperationContext,
)> {
    let context = context()?;
    let prepared = record("prepared")?;
    let plan = prepared.plan().clone();
    let binding = BuildBinding::new(
        SocketAddr::from(([127, 0, 0, 1], 5000)),
        "df",
        "dfhack",
        plan.before(),
    )?;
    let memory = Memory::default();
    let mut journal = BuildJournal::open(
        memory.clone(),
        &context,
        BuildMode::Control,
        Some(binding.clone()),
        Some([3; 32]),
    )?;
    let mut work = Work::new(&context)?;
    let mut entry = BuildEntry {
        plan,
        state: BuildState::Intent,
        native: None,
        dispatch_started: false,
        cancel_requested: false,
    };
    journal.retain(entry.clone(), &mut work)?;
    entry.state = BuildState::Prepared;
    entry.native = Some(prepared);
    journal.retain(entry.clone(), &mut work)?;
    entry.state = BuildState::DispatchStarted;
    entry.dispatch_started = true;
    journal.retain(entry.clone(), &mut work)?;
    entry.state = BuildState::Terminal;
    entry.native = Some(record("indeterminate")?);
    journal.retain(entry, &mut work)?;
    let source = Native {
        binding,
        reply: Some(record("placed")?),
        queries: 0,
        fenced: false,
        forbidden: 0,
        changed_after_query: false,
        disk: memory.clone(),
        corrupt_after_query: false,
    };
    Ok((
        memory,
        journal,
        source,
        Guard {
            deny: false,
            calls: 0,
        },
        context,
    ))
}
fn reconcile(
    journal: &mut BuildJournal<Memory>,
    source: &mut Native,
    guard: &mut Guard,
    context: &OperationContext,
) -> Result<BuildEntry> {
    let plan = record("prepared")?.plan().clone();
    journal.reconcile_receipt(source, plan.key(), plan.digest(), context, guard)
}

#[test]
fn exact_recovery_appends_proof_before_ack_and_reopens_without_dispatch_permission() -> Result<()> {
    let (memory, mut journal, mut source, mut guard, mut context) = setup()?;
    context.grants.retain(|g| g.capability == Capability::Query);
    let original = journal.raw.clone();
    let syncs = memory.0.borrow().syncs;
    let resolved = reconcile(&mut journal, &mut source, &mut guard, &context)?;
    assert!(!resolved.unresolved());
    assert!(resolved.dispatch_started());
    assert!(!resolved.cancel_requested());
    assert!(journal.raw.starts_with(&original));
    assert_eq!(memory.0.borrow().syncs, syncs + 1);
    assert_eq!(source.queries, 1);
    assert_eq!(source.forbidden, 0);
    assert!(source.fenced);
    assert!(!journal.has_permit());
    assert_eq!(resolved.native(), Some(&record("placed")?));
    let raw = journal.raw.clone();
    drop(journal);
    let mut reopened = BuildJournal::open(memory, &context, BuildMode::Recover, None, None)?;
    assert_eq!(reopened.raw, raw);
    assert_eq!(
        reconcile(&mut reopened, &mut source, &mut guard, &context)?,
        resolved
    );
    assert_eq!(source.queries, 1);
    assert!(!reopened.has_permit());
    Ok(())
}

#[test]
fn unchanged_or_absent_evidence_keeps_original_history_and_fence() -> Result<()> {
    for absent in [false, true] {
        let (_, mut journal, mut source, mut guard, context) = setup()?;
        let raw = journal.raw.clone();
        source.reply = if absent {
            None
        } else {
            Some(record("indeterminate")?)
        };
        let outcome = reconcile(&mut journal, &mut source, &mut guard, &context);
        assert_eq!(outcome.is_err(), absent);
        assert_eq!(journal.raw, raw);
        assert!(source.fenced);
        assert_eq!(journal.inventory(&context)?.pending_count(), 1);
        assert!(!journal.has_permit());
        assert_eq!(source.forbidden, 0);
    }
    Ok(())
}

#[test]
fn ordinary_successor_and_cancel_cannot_resolve_uncertainty() -> Result<()> {
    let old = record("indeterminate")?;
    assert!(old.validate_successor(&record("placed")?).is_err());
    let (_, mut journal, mut source, mut guard, context) = setup()?;
    let plan = old.plan();
    let unchanged = journal.recover(
        &mut source,
        plan.key(),
        plan.digest(),
        true,
        &context,
        &mut guard,
    )?;
    assert_eq!(unchanged.native(), Some(&old));
    assert_eq!(source.queries, 0);
    assert_eq!(source.forbidden, 0);
    for name in ["prepared", "expired", "cancelled"] {
        let (_, mut journal, mut source, mut guard, context) = setup()?;
        source.reply = Some(record(name)?);
        let raw = journal.raw.clone();
        assert!(reconcile(&mut journal, &mut source, &mut guard, &context).is_err());
        assert_eq!(journal.raw, raw);
        assert!(source.fenced);
    }
    Ok(())
}

#[test]
fn exact_source_is_required_before_contact_and_again_before_publication() -> Result<()> {
    for after in [false, true] {
        let (_, mut journal, mut source, mut guard, context) = setup()?;
        let raw = journal.raw.clone();
        if after {
            source.changed_after_query = true;
        } else {
            source.binding = source
                .binding
                .recovery_generation(source.binding.generation() + 1)?;
        }
        assert!(reconcile(&mut journal, &mut source, &mut guard, &context).is_err());
        assert_eq!(journal.raw, raw);
        assert!(source.fenced);
        assert_eq!(source.queries, usize::from(after));
    }
    Ok(())
}

#[test]
fn authority_custody_budget_and_retention_refuse_before_query() -> Result<()> {
    for case in 0..6 {
        let (memory, mut journal, mut source, mut guard, mut context) = setup()?;
        let raw = journal.raw.clone();
        match case {
            0 => context.grants.clear(),
            1 => guard.deny = true,
            2 => context.budget.max_bytes = 1,
            3 => journal.frames = MAX_FRAMES,
            4 => memory.0.borrow_mut().invalid = true,
            _ => journal.mode = BuildMode::Offline,
        }
        assert!(reconcile(&mut journal, &mut source, &mut guard, &context).is_err());
        assert_eq!(journal.raw, raw);
        assert_eq!(source.queries, 0);
        assert!(source.fenced);
    }
    Ok(())
}

#[test]
fn sync_failure_preserves_old_root_and_complete_bytes_recover_without_another_query() -> Result<()>
{
    let (memory, mut journal, mut source, mut guard, context) = setup()?;
    let raw = journal.raw.clone();
    memory.0.borrow_mut().fail_sync = true;
    assert!(reconcile(&mut journal, &mut source, &mut guard, &context).is_err());
    assert!(journal.is_fenced());
    assert_eq!(journal.raw, raw);
    assert!(memory.0.borrow().cursor.get_ref().starts_with(&raw));
    drop(journal);
    memory.0.borrow_mut().fail_sync = false;
    let mut reopened = BuildJournal::open(memory, &context, BuildMode::Recover, None, None)?;
    assert!(!reconcile(&mut reopened, &mut source, &mut guard, &context)?.unresolved());
    assert_eq!(source.queries, 1);
    assert!(!reopened.has_permit());
    Ok(())
}

#[test]
fn torn_reconciliation_is_not_repaired_and_post_read_custody_loss_publishes_nothing() -> Result<()>
{
    for partial in [true, false] {
        let (memory, mut journal, mut source, mut guard, context) = setup()?;
        let raw = journal.raw.clone();
        memory.0.borrow_mut().partial = partial;
        source.corrupt_after_query = !partial;
        assert!(reconcile(&mut journal, &mut source, &mut guard, &context).is_err());
        assert!(journal.is_fenced());
        assert_eq!(journal.raw, raw);
        assert!(source.fenced);
        assert_eq!(source.queries, 1);
        if partial {
            drop(journal);
            memory.0.borrow_mut().partial = false;
            assert!(BuildJournal::open(memory, &context, BuildMode::Recover, None, None).is_err());
        } else {
            assert_eq!(memory.0.borrow().cursor.get_ref(), &raw);
        }
    }
    Ok(())
}

#[test]
fn all_post_receipt_transitions_keep_key_history_and_only_accept_exact_placed() -> Result<()> {
    let (_, journal, _, _, _) = setup()?;
    let template = journal.entries.get("golden").ok_or_else(corrupt)?.clone();
    for prior in ["indeterminate", "placed", "expired", "cancelled"] {
        for current in [
            "prepared",
            "indeterminate",
            "placed",
            "expired",
            "cancelled",
        ] {
            for flags in 0..4 {
                let mut old = template.clone();
                old.native = Some(record(prior)?);
                let mut next = old.clone();
                next.native = Some(record(current)?);
                next.dispatch_started = flags & 1 != 0;
                next.cancel_requested = flags & 2 != 0;
                let allowed = prior == "indeterminate" && current == "placed" && flags == 1;
                assert_eq!(transition(Some(&old), &next).is_ok(), allowed);
            }
        }
    }
    Ok(())
}
