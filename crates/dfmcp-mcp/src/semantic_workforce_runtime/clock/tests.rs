use super::*;
use dfmcp_adapter::bounded_run::journal::RunBinding;
use dfmcp_core::{CapabilityGrant, CapabilityScope, RequestId, SessionId};
use std::cell::{Cell, RefCell};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::rc::Rc;

#[derive(Clone, Default)]
struct Memory {
    bytes: Rc<RefCell<Vec<u8>>>,
    position: u64,
    fail_sync: Rc<Cell<bool>>,
}
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let bytes = self.bytes.borrow();
        let start = (self.position as usize).min(bytes.len());
        let n = out.len().min(bytes.len() - start);
        out[..n].copy_from_slice(&bytes[start..start + n]);
        self.position += n as u64;
        Ok(n)
    }
}
impl Write for Memory {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        let mut bytes = self.bytes.borrow_mut();
        let start = self.position as usize;
        let length = bytes.len().max(start + input.len());
        bytes.resize(length, 0);
        bytes[start..start + input.len()].copy_from_slice(input);
        self.position += input.len() as u64;
        Ok(input.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Seek for Memory {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let next = match from {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::End(n) => self.bytes.borrow().len() as i128 + i128::from(n),
            SeekFrom::Current(n) => i128::from(self.position) + i128::from(n),
        };
        self.position = u64::try_from(next).map_err(|_| io::Error::other("negative test seek"))?;
        Ok(self.position)
    }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        if self.fail_sync.get() {
            return Err(io::Error::other("injected sync failure"));
        }
        Ok(())
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("no repair"))
    }
}
fn capture(generation: u64, sequence: u64, tick: u64) -> Result<RunObservation> {
    let mut out = b"DFMRO013".to_vec();
    for n in [generation, sequence, tick] {
        out.extend_from_slice(&n.to_be_bytes());
    }
    out.extend_from_slice(&[1, 1, 1]);
    RunObservation::decode(&out)
}
fn native_record(p: &RunPlan, phase: u8, reason: u8, tick: u64) -> Result<RunRecord> {
    let mut out = b"DFMRE013".to_vec();
    out.extend_from_slice(&(p.key().len() as u16).to_be_bytes());
    out.extend_from_slice(p.key().as_bytes());
    out.extend_from_slice(&p.spec().game_ticks().to_be_bytes());
    out.extend_from_slice(&p.spec().wall_ms().to_be_bytes());
    out.extend_from_slice(p.before().canonical_bytes());
    out.extend_from_slice(p.digest().as_bytes());
    out.extend_from_slice(p.token());
    let known = matches!(phase, 1..=3);
    out.extend_from_slice(&[
        phase,
        reason,
        u8::from(matches!(phase, 1..=3 | 5)),
        u8::from(phase == 3),
        u8::from(known),
    ]);
    out.extend_from_slice(&(if known { tick } else { 0 }).to_be_bytes());
    let mut bytes = b"dfmcp-bounded-run-receipt/1\0".to_vec();
    bytes.extend_from_slice(&out);
    out.extend_from_slice(Digest32::of_bytes(&bytes).as_bytes());
    RunRecord::decode(&out)
}
struct Native {
    manifest: RunManifest,
    before: RunObservation,
    record: Option<RunRecord>,
    reads: u32,
    prepares: u32,
    commits: u32,
    queries: u32,
    cancels: u32,
    fail_commit: bool,
    fail_receipt_sync: bool,
    memory: Memory,
}
impl Native {
    fn new(memory: Memory) -> Result<Self> {
        Ok(Self {
            manifest: RunManifest {
                generation: 41,
                df_version: "df".into(),
                dfhack_version: "dfhack".into(),
            },
            before: capture(41, 3, 100)?,
            record: None,
            reads: 0,
            prepares: 0,
            commits: 0,
            queries: 0,
            cancels: 0,
            fail_commit: false,
            fail_receipt_sync: false,
            memory,
        })
    }
    fn assert_durable(&self, c: &OperationContext, p: &RunPlan, state: RunState) -> Result<()> {
        assert_eq!(c.anchor.fortress_id, FortressId::NIL);
        let mut journal = RunJournal::open(self.memory.clone(), c, RunMode::Offline, None, false)?;
        assert_eq!(
            journal
                .records(c)?
                .iter()
                .find(|r| r.plan() == p)
                .map(DurableRun::state),
            Some(state)
        );
        Ok(())
    }
}
impl RunSource for Native {
    fn manifest(&self) -> &RunManifest {
        &self.manifest
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        Some(SocketAddr::from(([127, 0, 0, 1], 5000)))
    }
    fn observe(&mut self, c: &OperationContext) -> Result<RunObservation> {
        assert_eq!(c.anchor.fortress_id, FortressId::NIL);
        self.reads += 1;
        Ok(self.before.clone())
    }
    fn prepare(&mut self, p: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        self.assert_durable(c, p, RunState::Intent)?;
        self.prepares += 1;
        let record = native_record(p, 0, 0, 0)?;
        self.record = Some(record.clone());
        Ok(record)
    }
    fn commit(&mut self, p: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        self.assert_durable(c, p, RunState::DispatchStarted)?;
        self.commits += 1;
        let record = native_record(p, 1, 0, 100)?;
        self.record = Some(record.clone());
        if self.fail_receipt_sync {
            self.memory.fail_sync.set(true);
        }
        if self.fail_commit {
            return Err(DfmcpError::new(
                ErrorCode::AdapterUnavailable,
                "injected lost reply",
            ));
        }
        Ok(record)
    }
    fn query(&mut self, _: &RunPlan, _: &OperationContext) -> Result<Option<RunRecord>> {
        self.queries += 1;
        Ok(self.record.clone())
    }
    fn cancel(&mut self, p: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        self.assert_durable(c, p, RunState::CancelRequested)?;
        self.cancels += 1;
        let record = native_record(p, 3, 3, 101)?;
        self.record = Some(record.clone());
        Ok(record)
    }
}
fn target() -> ClockTarget {
    ClockTarget {
        digest: Digest32::of_bytes(b"retained-original-plan-identity-and-assignment"),
        original_seal: Digest32::of_bytes(b"original-workforce-review"),
        fortress: FortressId::new(71),
        epoch: 0,
        original_tick: 100,
        deadline: 1000,
    }
}
fn context(fortress: FortressId) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: StateAnchor {
            fortress_id: fortress,
            cursor: ObservationCursor {
                epoch: 0,
                sequence: 2,
            },
            tick: GameTick(100),
            state_hash: Digest32::of_bytes(b"current-qualified-labor-sample"),
        },
        budget: WorkBudget {
            max_bytes: 128 * 1024 * 1024,
            max_wall_millis: 5000,
            max_game_ticks: 1200,
            max_entities: 4096,
            max_actions: 1,
            max_output_tokens: 32768,
        },
        grants: [
            Capability::Query,
            Capability::Observe,
            Capability::Plan,
            Capability::ControlClock,
        ]
        .into_iter()
        .map(|capability| CapabilityGrant {
            capability,
            scope: CapabilityScope {
                fortress_id: Some(fortress),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::Guarded,
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
        cancellation_requested: false,
    }
}
fn request(t: &ClockTarget) -> RunRequest {
    RunRequest {
        kind: "run".into(),
        original_paired_seal: t.original_seal.to_hex(),
        game_ticks: 20,
        wall_millis: 1000,
    }
}
struct Fixture {
    target: ClockTarget,
    canonical: OperationContext,
    native_context: OperationContext,
    memory: Memory,
    source: Native,
    clock: Clock<Memory>,
}
impl Fixture {
    fn new() -> Result<Self> {
        let target = target();
        let canonical = context(target.fortress);
        let native_context = context(FortressId::NIL);
        let memory = Memory::default();
        let source = Native::new(memory.clone())?;
        let journal = RunJournal::open(
            memory.clone(),
            &native_context,
            RunMode::Control,
            Some(RunBinding::new(
                SocketAddr::from(([127, 0, 0, 1], 5000)),
                source.manifest.clone(),
            )?),
            true,
        )?;
        Ok(Self {
            target,
            canonical,
            native_context,
            memory,
            source,
            clock: Clock::new(journal),
        })
    }
    fn prepare(&mut self) -> Result<ClockView> {
        self.clock.prepare_guarded(
            &self.target,
            &request(&self.target),
            &mut self.source,
            &self.canonical,
            &self.native_context,
            |_| Ok(()),
        )
    }
    fn commit(&mut self, seal: Digest32) -> Result<ClockView> {
        self.clock.commit_guarded(
            &self.target,
            seal,
            true,
            &mut self.source,
            &self.canonical,
            &self.native_context,
            |_| Ok(()),
        )
    }
    fn inspect(&mut self) -> Result<ClockView> {
        self.clock
            .inspect(Some(&self.target), &self.canonical, &self.native_context)
    }
    fn reopen(&mut self, mode: RunMode) -> Result<()> {
        self.clock = Clock::new(RunJournal::open(
            self.memory.clone(),
            &self.native_context,
            mode,
            None,
            false,
        )?);
        Ok(())
    }
    fn reconcile(&mut self) -> Result<ClockView> {
        self.clock.reconcile(
            &self.target,
            &mut self.source,
            &self.canonical,
            &self.native_context,
        )
    }
    fn cancel(&mut self, source: bool) -> Result<ClockView> {
        self.clock.cancel(
            &self.target,
            if source { Some(&mut self.source) } else { None },
            &self.canonical,
            &self.native_context,
        )
    }
}
fn active(view: &ClockView) -> &ClockRunView {
    view.active().expect("fixture has one active run")
}

#[test]
fn closed_request_cannot_choose_native_keys_or_source_identity() -> Result<()> {
    let t = target();
    let good = format!(
        "{{\"kind\":\"run\",\"original_paired_seal\":\"{}\",\"game_ticks\":20,\"wall_millis\":1000}}",
        t.original_seal.to_hex()
    );
    assert_eq!(RunRequest::parse(&good)?, request(&t));
    for raw in [
        good.replace("\"run\"", "\"clock\""),
        good.replace("20,", "0,"),
        good.replace("20,", "1201,"),
        good.replace("1000}", "60001}"),
        good.replace("1000}", "0}"),
        good.replace("1000}", "1000,\"key\":\"replay\"}"),
        good.replace("1000}", "1000,\"generation\":9}"),
        good.replace("1000}", "1000,\"game_ticks\":20}"),
        good.replace("20,", "null,"),
    ] {
        assert!(RunRequest::parse(&raw).is_err(), "{raw}");
    }
    assert!(RunRequest::parse(&" ".repeat(2049)).is_err());
    Ok(())
}

#[test]
fn reviewed_clock_executes_once_and_never_becomes_labor_proof() -> Result<()> {
    let mut f = Fixture::new()?;
    let prepared = f.prepare()?;
    let seal = active(&prepared).review_seal;
    assert_ne!(seal, f.target.original_seal);
    assert!(!prepared.may_be_running());
    assert!(prepared.pending);
    assert_eq!(f.prepare()?.head, prepared.head);
    assert_eq!(f.source.prepares, 1);
    let running = f.commit(seal)?;
    assert!(running.may_be_running());
    assert!(!running.can_sample_goal());
    assert!(f.commit(seal).is_err());
    assert_eq!(f.source.commits, 1);
    f.source.record = Some(native_record(active(&running).entry.plan(), 3, 1, 120)?);
    let stopped = f.reconcile()?;
    assert!(!stopped.pending);
    assert!(stopped.can_sample_goal());
    assert_eq!((stopped.high_tick, stopped.high_sequence), (120, 4));
    let json = stopped.to_json();
    assert_eq!(json["original_goal_proven"], false);
    assert_eq!(json["current_pause_proven"], false);
    assert_eq!(json["runs"][0]["native"]["historical_pause_verified"], true);
    assert_eq!(
        json["source_identity_semantics"],
        "source_incarnation_only_not_named_fortress_lineage"
    );
    f.commit(seal)?;
    assert_eq!(f.source.commits, 1);
    Ok(())
}

#[test]
fn lost_reply_reopens_read_only_without_unpause_replay() -> Result<()> {
    let mut f = Fixture::new()?;
    let seal = active(&f.prepare()?).review_seal;
    f.source.fail_commit = true;
    assert!(f.commit(seal).is_err());
    assert_eq!(f.source.commits, 1);
    f.reopen(RunMode::Recover)?;
    assert!(!f.clock.is_control());
    assert!(f.inspect()?.unresolved);
    assert!(f.commit(seal).is_err());
    assert!(f.prepare().is_err());
    assert!(f.cancel(true).is_err());
    for c in [&mut f.canonical, &mut f.native_context] {
        c.grants.retain(|g| g.capability == Capability::Query);
    }
    f.reconcile()?;
    assert_eq!(f.source.queries, 1);
    f.source.record = None;
    assert!(f.reconcile().is_err());
    assert_eq!(f.source.commits, 1);
    Ok(())
}

#[test]
fn separate_source_grants_cannot_be_derived_from_canonical_scope() -> Result<()> {
    for which in 0..7 {
        let mut f = Fixture::new()?;
        match which {
            0 => f.native_context = f.canonical.clone(),
            1 => f.native_context.grants = f.canonical.grants.clone(),
            2 => f.canonical.grants = f.native_context.grants.clone(),
            3 => f.native_context.request_id = RequestId::new(2),
            4 => f.native_context.session_id = SessionId::new(2),
            5 => f
                .native_context
                .grants
                .retain(|g| g.capability != Capability::ControlClock),
            _ => f
                .canonical
                .grants
                .retain(|g| g.capability != Capability::ControlClock),
        }
        assert!(f.prepare().is_err(), "case {which}");
        assert_eq!(f.source.prepares, 0);
        assert_eq!(f.source.commits, 0);
    }
    Ok(())
}

#[test]
fn post_dispatch_guard_preserves_canonical_scope_and_blocks_unpause_forever() -> Result<()> {
    let mut f = Fixture::new()?;
    let prepared = f.prepare()?;
    let seal = active(&prepared).review_seal;
    let plan = active(&prepared).entry.plan().clone();
    let calls = Cell::new(0);
    let memory = f.memory.clone();
    let native = f.native_context.clone();
    let expected = f.canonical.anchor;
    assert!(
        f.clock
            .commit_guarded(
                &f.target,
                seal,
                true,
                &mut f.source,
                &f.canonical,
                &f.native_context,
                |edge| {
                    calls.set(calls.get() + 1);
                    assert_eq!(edge.anchor, expected);
                    assert!(
                        edge.grants
                            .iter()
                            .all(|g| g.scope.fortress_id == Some(expected.fortress_id))
                    );
                    let mut journal =
                        RunJournal::open(memory.clone(), &native, RunMode::Offline, None, false)?;
                    assert_eq!(
                        journal
                            .records(&native)?
                            .iter()
                            .find(|e| e.plan() == &plan)
                            .map(DurableRun::state),
                        Some(RunState::DispatchStarted)
                    );
                    Err(corrupt("injected replaced original citizen archive"))
                },
            )
            .is_err()
    );
    assert_eq!(calls.get(), 1);
    assert_eq!(f.source.commits, 0);
    assert!(f.inspect()?.unresolved);
    f.reopen(RunMode::Control)?;
    let recovered = f.reconcile()?;
    assert_eq!(active(&recovered).entry.state(), RunState::Tracking);
    assert!(f.commit(seal).is_err());
    assert_eq!(f.source.commits, 0);
    Ok(())
}

#[test]
fn post_intent_guard_failure_retains_one_preparation_without_retry() -> Result<()> {
    let mut f = Fixture::new()?;
    let memory = f.memory.clone();
    let native = f.native_context.clone();
    let calls = Cell::new(0);
    assert!(
        f.clock
            .prepare_guarded(
                &f.target,
                &request(&f.target),
                &mut f.source,
                &f.canonical,
                &f.native_context,
                |_| {
                    calls.set(calls.get() + 1);
                    let mut journal =
                        RunJournal::open(memory.clone(), &native, RunMode::Offline, None, false)?;
                    assert_eq!(journal.records(&native)?[0].state(), RunState::Intent);
                    Err(denied("injected revoked original authority"))
                },
            )
            .is_err()
    );
    assert_eq!(calls.get(), 1);
    assert_eq!(f.source.prepares, 0);
    assert_eq!(active(&f.prepare()?).entry.state(), RunState::Intent);
    assert!(f.reconcile().is_err());
    assert!(!f.cancel(false)?.pending);
    assert_eq!(f.source.prepares, 0);
    assert_eq!(f.source.cancels, 0);
    Ok(())
}

#[test]
fn changed_target_deadline_tick_or_either_budget_refuses_preparation() -> Result<()> {
    for which in 0..8 {
        let mut f = Fixture::new()?;
        match which {
            0 => f.source.before = capture(41, 3, 101)?,
            1 => f.target.deadline = 119,
            2 => f.canonical.budget.max_game_ticks = 19,
            3 => f.native_context.budget.max_game_ticks = 19,
            4 => f.native_context.budget.max_bytes = VIEW_BYTES - 1,
            5 => f.native_context.anchor.tick = GameTick(101),
            6 => f.target.epoch = 1,
            _ => f.target.fortress = FortressId::new(72),
        }
        assert!(f.prepare().is_err(), "case {which}");
        assert_eq!(f.source.prepares, 0);
        assert_eq!(f.source.commits, 0);
    }
    Ok(())
}

#[test]
fn original_or_clock_source_substitution_is_refused_on_reopen() -> Result<()> {
    let mut f = Fixture::new()?;
    let seal = active(&f.prepare()?).review_seal;
    let mut changed = f.target.clone();
    changed.digest = Digest32::of_bytes(b"other-original-plan-or-citizen-identity");
    assert!(
        f.clock
            .inspect(Some(&changed), &f.canonical, &f.native_context)
            .is_err()
    );
    assert!(
        f.clock
            .inspect(None, &f.canonical, &f.native_context)
            .is_err()
    );
    f.reopen(RunMode::Control)?;
    f.source.manifest.generation += 1;
    assert!(f.commit(seal).is_err());
    assert_eq!(f.source.commits, 0);
    Ok(())
}

#[test]
fn sync_failure_after_dispatch_keeps_recovery_without_another_effect() -> Result<()> {
    let mut f = Fixture::new()?;
    let seal = active(&f.prepare()?).review_seal;
    f.source.fail_receipt_sync = true;
    assert!(f.commit(seal).is_err());
    assert!(f.inspect().is_err());
    assert_eq!(f.source.commits, 1);
    f.memory.fail_sync.set(false);
    f.reopen(RunMode::Recover)?;
    assert!(f.inspect()?.unresolved);
    f.reconcile()?;
    assert_eq!(f.source.commits, 1);
    Ok(())
}

#[test]
fn stopped_history_floors_require_later_valid_samples_for_next_slice() -> Result<()> {
    let mut f = Fixture::new()?;
    let seal = active(&f.prepare()?).review_seal;
    let running = f.commit(seal)?;
    f.source.record = Some(native_record(active(&running).entry.plan(), 3, 1, 120)?);
    f.reconcile()?;
    assert!(f.prepare().is_err());
    f.canonical.anchor.tick = GameTick(120);
    f.native_context.anchor.tick = GameTick(120);
    f.source.before = capture(41, 3, 120)?;
    assert!(f.prepare().is_err());
    f.source.before = capture(41, 4, 120)?;
    let next = f.prepare()?;
    assert_eq!(next.runs.len(), 2);
    assert_eq!(f.source.prepares, 2);
    assert_ne!(active(&next).review_seal, seal);
    Ok(())
}

#[test]
fn safety_cancel_survives_goal_expiry_without_observe_or_plan_authority() -> Result<()> {
    let mut f = Fixture::new()?;
    let seal = active(&f.prepare()?).review_seal;
    f.commit(seal)?;
    for c in [&mut f.canonical, &mut f.native_context] {
        c.anchor.tick = GameTick(2000);
        c.grants
            .retain(|g| matches!(g.capability, Capability::Query | Capability::ControlClock));
    }
    assert!(!f.cancel(true)?.pending);
    f.cancel(true)?;
    assert_eq!(f.source.cancels, 1);
    assert_eq!(f.source.commits, 1);
    Ok(())
}

#[test]
fn source_loss_remains_unresolved_without_querying_or_pausing_replacement() -> Result<()> {
    let mut f = Fixture::new()?;
    let seal = active(&f.prepare()?).review_seal;
    let running = f.commit(seal)?;
    f.source.record = Some(native_record(active(&running).entry.plan(), 5, 7, 0)?);
    let lost = f.reconcile()?;
    assert!(lost.pending);
    assert!(lost.unresolved);
    assert!(!lost.can_sample_goal());
    assert!(!lost.needs_reconciliation());
    assert!(!lost.cancellation_needs_source());
    f.reconcile()?;
    f.cancel(false)?;
    assert_eq!(f.source.queries, 1);
    assert_eq!(f.source.cancels, 0);
    assert!(f.commit(seal).is_err());
    Ok(())
}

#[test]
fn retention_covers_64_samples_and_preserves_complete_bounded_output() -> Result<()> {
    let mut f = Fixture::new()?;
    for index in 0..MAX_RUNS {
        let mut r = request(&f.target);
        r.wall_millis += index as u32;
        f.clock.prepare_guarded(
            &f.target,
            &r,
            &mut f.source,
            &f.canonical,
            &f.native_context,
            |_| Ok(()),
        )?;
        f.cancel(false)?;
    }
    let full = f.inspect()?;
    assert_eq!(full.runs.len(), MAX_RUNS);
    assert!(
        serde_json::to_vec(&full.to_json())
            .map_err(|_| invalid("fixture JSON"))?
            .len()
            < 96 * 1024
    );
    let mut r = request(&f.target);
    r.wall_millis = 5000;
    assert!(
        f.clock
            .prepare_guarded(
                &f.target,
                &r,
                &mut f.source,
                &f.canonical,
                &f.native_context,
                |_| Ok(()),
            )
            .is_err()
    );
    assert_eq!(f.source.prepares, MAX_RUNS as u32);
    assert_eq!(f.inspect()?.runs.len(), MAX_RUNS);
    f.prepare()?;
    assert_eq!(f.source.prepares, MAX_RUNS as u32);
    Ok(())
}

#[test]
fn offline_history_never_contacts_native_or_gains_control() -> Result<()> {
    let mut f = Fixture::new()?;
    let seal = active(&f.prepare()?).review_seal;
    f.reopen(RunMode::Offline)?;
    let reads = f.source.reads;
    assert_eq!(f.inspect()?.mode, RunMode::Offline);
    assert!(f.reconcile().is_err());
    assert!(f.prepare().is_err());
    assert!(f.commit(seal).is_err());
    assert!(f.cancel(true).is_err());
    assert_eq!(f.source.reads, reads);
    assert_eq!(f.source.queries, 0);
    Ok(())
}

#[test]
fn retained_tick_floors_do_not_renew_either_authority_scope() -> Result<()> {
    for native in [false, true] {
        let mut f = Fixture::new()?;
        let seal = active(&f.prepare()?).review_seal;
        let running = f.commit(seal)?;
        f.source.record = Some(native_record(active(&running).entry.plan(), 3, 1, 120)?);
        f.reconcile()?;
        let c = if native {
            &mut f.native_context
        } else {
            &mut f.canonical
        };
        for grant in &mut c.grants {
            grant.expires_at_tick = Some(GameTick(110));
        }
        assert!(f.inspect().is_err());
        assert!(f.cancel(false).is_err());
    }
    Ok(())
}

#[test]
fn rejected_complete_captures_retain_exclusive_owner_clock_floors() -> Result<()> {
    let mut f = Fixture::new()?;
    let anchor = f.canonical.anchor;
    f.source.before = capture(41, 5, 110)?;
    assert!(f.prepare().is_err());
    let view = f.inspect()?;
    assert_eq!((view.high_tick, view.high_sequence), (110, 5));
    assert!(!view.has_runs());
    assert_eq!(f.canonical.anchor, anchor);
    f.source.before = capture(41, 3, 100)?;
    assert!(f.prepare().is_err());
    f.canonical.anchor.tick = GameTick(110);
    f.native_context.anchor.tick = GameTick(110);
    f.source.before = capture(41, 4, 110)?;
    assert!(f.prepare().is_err());
    f.source.before = capture(41, 5, 110)?;
    f.prepare()?;
    assert_eq!(f.source.prepares, 1);
    Ok(())
}

mod original_target {
    #![allow(dead_code, unused_imports)]
    include!("../fixtures.rs");
    use super::super::ClockTarget;
    use dfmcp_intent::plan_archive::{ArchiveLimits, PlanArchiveSeal, encode};

    #[test]
    fn real_original_target_binds_plan_identity_journal_and_verified_receipt() -> Result<()> {
        let mut f = fixture()?;
        let review = prepared(&mut f)?;
        let result = f.owner.commit(
            review.seal(),
            true,
            &mut f.observer,
            &f.context,
            |binding, context| f.native.connect(binding, context),
        )?;
        assert!(matches!(
            result.action_result(),
            SingleLaborResult::Verified { .. }
        ));
        let (_, archive) = encode(&f.plan, ArchiveLimits::default())?;
        let journal = f.owner.inventory(&f.context)?.id;
        let identity = f.observer.capsule.content_digest;
        let target = ClockTarget::new(
            &f.plan,
            archive,
            journal,
            identity,
            &review,
            result.action_result(),
        )?;
        assert_eq!(target.original_paired_seal(), review.seal());
        assert_eq!(target.deadline_tick(), 200);
        for (changed_archive, changed_journal, changed_identity) in [
            (
                PlanArchiveSeal {
                    object_digest: Digest32::of_bytes(b"other archive"),
                    ..archive
                },
                journal,
                identity,
            ),
            (
                archive,
                Digest32::of_bytes(b"other assignment journal"),
                identity,
            ),
            (
                archive,
                journal,
                Digest32::of_bytes(b"other original citizens"),
            ),
        ] {
            let changed = ClockTarget::new(
                &f.plan,
                changed_archive,
                changed_journal,
                changed_identity,
                &review,
                result.action_result(),
            )?;
            assert_ne!(changed.digest(), target.digest());
        }
        for action in [
            SingleLaborResult::Unverified,
            SingleLaborResult::AppliedOutsideSemantics {
                receipt: Digest32::of_bytes(b"other labors changed"),
            },
            SingleLaborResult::Verified {
                receipt: Digest32::ZERO,
            },
        ] {
            assert!(
                ClockTarget::new(&f.plan, archive, journal, identity, &review, &action).is_err()
            );
        }
        for (journal, identity) in [(Digest32::ZERO, identity), (journal, Digest32::ZERO)] {
            assert!(
                ClockTarget::new(
                    &f.plan,
                    archive,
                    journal,
                    identity,
                    &review,
                    result.action_result(),
                )
                .is_err()
            );
        }
        let mut changed = f.plan.clone();
        changed.summary.push_str(" changed");
        reseal(&mut changed);
        assert!(
            ClockTarget::new(
                &changed,
                archive,
                journal,
                identity,
                &review,
                result.action_result(),
            )
            .is_err()
        );
        Ok(())
    }
}
