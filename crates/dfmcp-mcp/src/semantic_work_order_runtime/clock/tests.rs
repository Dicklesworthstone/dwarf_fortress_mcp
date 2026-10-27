use super::*;
use dfmcp_adapter::order_run::journal::OrderRunBinding;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, GameTick, ObservationCursor, RequestId, SessionId,
    StateAnchor, WorkBudget,
};
use std::cell::{Cell, RefCell};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::rc::Rc;

#[derive(Clone, Default)]
struct Memory {
    bytes: Rc<RefCell<Vec<u8>>>,
    position: u64,
    syncs: Rc<Cell<u32>>,
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
        self.syncs.set(self.syncs.get() + 1);
        Ok(())
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("no repair"))
    }
}
fn field(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    out.extend_from_slice(bytes);
}
fn capture(
    generation: u64,
    sequence: u64,
    tick: u64,
    order: u32,
    recipe: u8,
    total: i32,
    remaining: i32,
) -> Result<OrderCapture> {
    let mut out = b"DFMOR014".to_vec();
    for n in [generation, sequence, tick] {
        out.extend_from_slice(&n.to_be_bytes());
    }
    out.extend_from_slice(&7u32.to_be_bytes());
    field(&mut out, b"region1");
    out.push(1);
    out.extend_from_slice(&order.to_be_bytes());
    out.extend_from_slice(&100u32.to_be_bytes());
    out.extend_from_slice(&[1, recipe]);
    out.extend_from_slice(&total.to_be_bytes());
    out.extend_from_slice(&remaining.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
    OrderCapture::decode(&out)
}
fn status(value: OrderCapture, status: u32) -> Result<OrderCapture> {
    let mut bytes = value.canonical_bytes().to_vec();
    let end = bytes.len();
    bytes[end - 4..].copy_from_slice(&status.to_be_bytes());
    OrderCapture::decode(&bytes)
}
fn native_record(p: &OrderRunPlan, phase: u8, reason: u8, tick: u64) -> Result<OrderRunRecord> {
    let mut out = b"DFMOE014".to_vec();
    field(&mut out, p.key().as_bytes());
    field(&mut out, p.native_bytes());
    out.extend_from_slice(p.digest().as_bytes());
    out.extend_from_slice(p.token());
    let known = matches!(phase, 1..=3);
    out.extend_from_slice(&[
        phase,
        reason,
        0,
        u8::from(matches!(phase, 1..=3 | 5)),
        u8::from(phase == 3),
        u8::from(known),
    ]);
    out.extend_from_slice(&(if known { tick } else { 0 }).to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&p.before().tick().to_be_bytes());
    field(&mut out, &[]);
    let mut bytes = b"dfmcp-order-run-receipt/1\0".to_vec();
    bytes.extend_from_slice(&out);
    out.extend_from_slice(Digest32::of_bytes(&bytes).as_bytes());
    OrderRunRecord::decode(&out)
}
struct Native {
    manifest: OrderRunManifest,
    fortress: FortressIdentity,
    before: OrderCapture,
    record: Option<OrderRunRecord>,
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
            manifest: OrderRunManifest {
                generation: 41,
                df_version: "df".into(),
                dfhack_version: "dfhack".into(),
            },
            fortress: FortressIdentity::new("region1", 7)?,
            before: capture(41, 3, 100, 9, 1, 5, 5)?,
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
    fn assert_durable(
        &self,
        c: &OperationContext,
        p: &OrderRunPlan,
        state: OrderRunState,
    ) -> Result<()> {
        let mut journal = OrderRunJournal::open(
            self.memory.clone(),
            c,
            OrderRunMode::Offline,
            &self.fortress,
            None,
            false,
        )?;
        let view = journal.view(c)?;
        assert_eq!(
            view.entries
                .iter()
                .find(|r| r.plan() == p)
                .map(OrderRunEntry::state),
            Some(state)
        );
        Ok(())
    }
}
impl OrderRunSource for Native {
    fn manifest(&self) -> &OrderRunManifest {
        &self.manifest
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        Some(SocketAddr::from(([127, 0, 0, 1], 5000)))
    }
    fn fortress(&self) -> &FortressIdentity {
        &self.fortress
    }
    fn fence(&mut self) {}
    fn observe(&mut self, _: u32, _: &OperationContext, _: Duration) -> Result<OrderCapture> {
        self.reads += 1;
        Ok(self.before.clone())
    }
    fn prepare(
        &mut self,
        p: &OrderRunPlan,
        c: &OperationContext,
        _: Duration,
    ) -> Result<OrderRunRecord> {
        self.assert_durable(c, p, OrderRunState::Intent)?;
        self.prepares += 1;
        let record = native_record(p, 0, 0, 0)?;
        self.record = Some(record.clone());
        Ok(record)
    }
    fn commit(
        &mut self,
        p: &OrderRunPlan,
        c: &OperationContext,
        _: Duration,
    ) -> Result<OrderRunRecord> {
        self.assert_durable(c, p, OrderRunState::DispatchStarted)?;
        self.commits += 1;
        let record = native_record(p, 1, 0, p.before().tick())?;
        self.record = Some(record.clone());
        if self.fail_receipt_sync {
            self.memory.fail_sync.set(true);
        }
        if self.fail_commit {
            return Err(DfmcpError::new(
                ErrorCode::AdapterUnavailable,
                "injected lost commit reply",
            ));
        }
        Ok(record)
    }
    fn query(
        &mut self,
        _: &OrderRunPlan,
        _: &OperationContext,
        _: Duration,
    ) -> Result<Option<OrderRunRecord>> {
        self.queries += 1;
        Ok(self.record.clone())
    }
    fn cancel(
        &mut self,
        p: &OrderRunPlan,
        c: &OperationContext,
        _: Duration,
    ) -> Result<OrderRunRecord> {
        self.assert_durable(c, p, OrderRunState::CancelRequested)?;
        self.cancels += 1;
        let record = native_record(p, 3, 3, p.before().tick() + 1)?;
        self.record = Some(record.clone());
        Ok(record)
    }
}
fn target() -> Result<ClockTarget> {
    Ok(ClockTarget {
        digest: Digest32::of_bytes(b"retained-original-archive-creation-association"),
        original_seal: Digest32::of_bytes(b"original-semantic-review"),
        fortress: FortressIdentity::new("region1", 7)?,
        epoch: 0,
        original_tick: 100,
        order: 9,
        recipe: 1,
        amount: 5,
        deadline: 500,
    })
}
fn context(target: &ClockTarget) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: StateAnchor {
            fortress_id: target.fortress.fortress_id(),
            cursor: ObservationCursor {
                epoch: 0,
                sequence: 2,
            },
            tick: GameTick(100),
            state_hash: Digest32::of_bytes(b"current-qualified-goal-sample"),
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
                fortress_id: Some(target.fortress.fortress_id()),
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
fn open(
    memory: Memory,
    n: &Native,
    c: &OperationContext,
    mode: OrderRunMode,
    initialize: bool,
) -> Result<Clock<Memory>> {
    Ok(Clock::new(OrderRunJournal::open(
        memory,
        c,
        mode,
        n.fortress(),
        if initialize {
            Some(OrderRunBinding::from_source(n)?)
        } else {
            None
        },
        initialize,
    )?))
}
fn fixture() -> Result<(ClockTarget, OperationContext, Memory, Native, Clock<Memory>)> {
    let t = target()?;
    let c = context(&t);
    let memory = Memory::default();
    let n = Native::new(memory.clone())?;
    let clock = open(memory.clone(), &n, &c, OrderRunMode::Control, true)?;
    Ok((t, c, memory, n, clock))
}
fn active(view: &ClockView) -> &ClockRunView {
    view.active().expect("fixture has one active run")
}

#[test]
fn closed_clock_request_cannot_choose_targets_or_stop_predicates() -> Result<()> {
    let t = target()?;
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
        good.replace("1000}", "1000,\"native_order_id\":9}"),
        good.replace("1000}", "1000,\"predicate\":\"approved\"}"),
        good.replace("1000}", "1000,\"game_ticks\":20}"),
        good.replace("20,", "null,"),
    ] {
        assert!(RunRequest::parse(&raw).is_err(), "{raw}");
    }
    assert!(RunRequest::parse(&" ".repeat(2049)).is_err());
    Ok(())
}

#[test]
fn real_clock_journal_executes_once_and_keeps_clock_history_separate() -> Result<()> {
    let (t, c, _, mut n, mut clock) = fixture()?;
    let prepared = clock.prepare(&t, &request(&t), &mut n, &c)?;
    let seal = active(&prepared).review_seal;
    assert_ne!(seal, t.original_seal);
    assert!(!prepared.may_be_running());
    assert!(prepared.pending);
    assert_eq!(n.prepares, 1);
    assert_eq!(
        clock.prepare(&t, &request(&t), &mut n, &c)?.head,
        prepared.head
    );
    assert_eq!(n.prepares, 1);
    let running = clock.commit(&t, seal, true, &mut n, &c)?;
    assert!(running.may_be_running());
    assert!(!running.can_sample_goal());
    assert_eq!(n.commits, 1);
    assert!(clock.commit(&t, seal, true, &mut n, &c).is_err());
    assert_eq!(n.commits, 1);
    let p = active(&running).entry.plan().clone();
    n.record = Some(native_record(&p, 3, 1, 120)?);
    let stopped = clock.reconcile(&t, &mut n, &c)?;
    assert!(!stopped.pending);
    assert!(stopped.can_sample_goal());
    assert_eq!(stopped.high_tick, 120);
    assert_eq!(stopped.high_sequence, 4);
    let json = stopped.to_json();
    assert_eq!(json["original_goal_proven"], false);
    assert_eq!(json["current_pause_proven"], false);
    assert_eq!(json["runs"][0]["native"]["historical_pause_verified"], true);
    clock.commit(&t, seal, true, &mut n, &c)?;
    assert_eq!(n.commits, 1);
    Ok(())
}

#[test]
fn lost_reply_recovers_read_only_without_unpause_replay() -> Result<()> {
    let (t, c, memory, mut n, mut clock) = fixture()?;
    let prepared = clock.prepare(&t, &request(&t), &mut n, &c)?;
    let seal = active(&prepared).review_seal;
    n.fail_commit = true;
    assert!(clock.commit(&t, seal, true, &mut n, &c).is_err());
    assert_eq!(n.commits, 1);
    drop(clock);
    let mut recovered = open(memory, &n, &c, OrderRunMode::Recover, false)?;
    assert!(!recovered.is_control());
    assert!(recovered.inspect(Some(&t), &c)?.unresolved);
    assert!(recovered.commit(&t, seal, true, &mut n, &c).is_err());
    assert!(recovered.prepare(&t, &request(&t), &mut n, &c).is_err());
    assert!(recovered.cancel(&t, Some(&mut n), &c).is_err());
    let mut query = c.clone();
    query.grants.retain(|g| g.capability == Capability::Query);
    recovered.reconcile(&t, &mut n, &query)?;
    assert_eq!(n.queries, 1);
    assert_eq!(n.commits, 1);
    n.record = None;
    assert!(recovered.reconcile(&t, &mut n, &query).is_err());
    assert_eq!(n.commits, 1);
    Ok(())
}

#[test]
fn archive_guard_after_dispatch_marker_blocks_unpause_permanently() -> Result<()> {
    let (t, c, memory, mut n, mut clock) = fixture()?;
    let prepared = clock.prepare(&t, &request(&t), &mut n, &c)?;
    let seal = active(&prepared).review_seal;
    let plan = active(&prepared).entry.plan().clone();
    let calls = Cell::new(0);
    let mut guard = |edge: &OperationContext| {
        calls.set(calls.get() + 1);
        let mut j = OrderRunJournal::open(
            memory.clone(),
            edge,
            OrderRunMode::Offline,
            &t.fortress,
            None,
            false,
        )?;
        assert_eq!(
            j.view(edge)?
                .entries
                .iter()
                .find(|e| e.plan() == &plan)
                .map(OrderRunEntry::state),
            Some(OrderRunState::DispatchStarted)
        );
        Err(corrupt("injected replaced original archive"))
    };
    assert!(
        clock
            .commit_guarded(&t, seal, true, &mut n, &c, &mut guard)
            .is_err()
    );
    assert_eq!(calls.get(), 1);
    assert_eq!(n.commits, 0);
    assert!(clock.inspect(Some(&t), &c)?.unresolved);
    // Querying the native Prepared receipt after dispatch remains Tracking.
    let recovered = clock.reconcile(&t, &mut n, &c)?;
    assert_eq!(active(&recovered).entry.state(), OrderRunState::Tracking);
    assert!(clock.commit(&t, seal, true, &mut n, &c).is_err());
    assert_eq!(n.commits, 0);
    Ok(())
}

#[test]
fn prepare_guard_runs_after_intent_and_denial_cannot_retry_preparation() -> Result<()> {
    let (t, c, memory, mut n, mut clock) = fixture()?;
    let calls = Cell::new(0);
    assert!(
        clock
            .prepare_guarded(&t, &request(&t), &mut n, &c, |edge| {
                calls.set(calls.get() + 1);
                let mut j = OrderRunJournal::open(
                    memory.clone(),
                    edge,
                    OrderRunMode::Offline,
                    &t.fortress,
                    None,
                    false,
                )?;
                assert_eq!(j.view(edge)?.entries[0].state(), OrderRunState::Intent);
                Err(denied("injected revoked custody"))
            })
            .is_err()
    );
    assert_eq!(calls.get(), 1);
    assert_eq!(n.prepares, 0);
    let replay = clock.prepare(&t, &request(&t), &mut n, &c)?;
    assert_eq!(active(&replay).entry.state(), OrderRunState::Intent);
    assert_eq!(n.prepares, 0);
    assert!(clock.reconcile(&t, &mut n, &c).is_err());
    let cancelled = clock.cancel::<Native>(&t, None, &c)?;
    assert!(!cancelled.pending);
    assert_eq!(n.cancels, 0);
    Ok(())
}

#[test]
fn target_tick_deadline_and_authority_refuse_before_prepare() -> Result<()> {
    for which in 0..9 {
        let (mut t, mut c, _, mut n, mut clock) = fixture()?;
        let mut r = request(&t);
        match which {
            0 => n.before = capture(41, 3, 101, 9, 1, 5, 5)?,
            1 => n.before = capture(41, 3, 100, 10, 1, 5, 5)?,
            2 => n.before = capture(41, 3, 100, 9, 2, 5, 5)?,
            3 => n.before = capture(41, 3, 100, 9, 1, 6, 5)?,
            4 => n.before = capture(41, 3, 100, 9, 1, 5, 0)?,
            5 => t.deadline = 119,
            6 => c.budget.max_game_ticks = 19,
            7 => c
                .grants
                .retain(|g| g.capability != Capability::ControlClock),
            _ => r.original_paired_seal = Digest32::of_bytes(b"wrong original").to_hex(),
        }
        assert!(clock.prepare(&t, &r, &mut n, &c).is_err(), "case {which}");
        assert_eq!(n.prepares, 0);
        assert_eq!(n.commits, 0);
    }
    Ok(())
}

#[test]
fn changed_source_and_changed_original_history_are_rejected() -> Result<()> {
    let (t, c, memory, mut n, mut clock) = fixture()?;
    clock.prepare(&t, &request(&t), &mut n, &c)?;
    let mut changed = t.clone();
    changed.digest = Digest32::of_bytes(b"other original archive");
    assert!(clock.inspect(Some(&changed), &c).is_err());
    assert!(clock.inspect(None, &c).is_err());
    drop(clock);
    let mut recovered = open(memory, &n, &c, OrderRunMode::Control, false)?;
    let view = recovered.inspect(Some(&t), &c)?;
    n.manifest.generation += 1;
    assert!(
        recovered
            .commit(&t, active(&view).review_seal, true, &mut n, &c)
            .is_err()
    );
    assert_eq!(n.commits, 0);
    Ok(())
}

#[test]
fn sync_failure_after_native_dispatch_reopens_as_unresolved() -> Result<()> {
    let (t, c, memory, mut n, mut clock) = fixture()?;
    let view = clock.prepare(&t, &request(&t), &mut n, &c)?;
    let seal = active(&view).review_seal;
    n.fail_receipt_sync = true;
    assert!(clock.commit(&t, seal, true, &mut n, &c).is_err());
    assert!(clock.inspect(Some(&t), &c).is_err());
    assert_eq!(n.commits, 1);
    memory.fail_sync.set(false);
    drop(clock);
    let mut recovered = open(memory, &n, &c, OrderRunMode::Recover, false)?;
    assert!(recovered.inspect(Some(&t), &c)?.unresolved);
    recovered.reconcile(&t, &mut n, &c)?;
    assert_eq!(n.commits, 1);
    Ok(())
}

#[test]
fn stopped_history_floors_reject_regression_and_allow_explicit_next_slice() -> Result<()> {
    let (t, mut c, _, mut n, mut clock) = fixture()?;
    let prepared = clock.prepare(&t, &request(&t), &mut n, &c)?;
    let seal = active(&prepared).review_seal;
    let running = clock.commit(&t, seal, true, &mut n, &c)?;
    n.record = Some(native_record(active(&running).entry.plan(), 3, 1, 120)?);
    clock.reconcile(&t, &mut n, &c)?;
    let mut r = request(&t);
    r.wall_millis = 900;
    assert!(clock.prepare(&t, &r, &mut n, &c).is_err());
    assert_eq!(n.prepares, 1);
    c.anchor.tick = GameTick(120);
    n.before = capture(41, 3, 120, 9, 1, 5, 4)?;
    assert!(clock.prepare(&t, &r, &mut n, &c).is_err());
    assert_eq!(n.prepares, 1);
    n.before = capture(41, 4, 120, 9, 1, 5, 4)?;
    let next = clock.prepare(&t, &r, &mut n, &c)?;
    assert_eq!(next.runs.len(), 2);
    assert_eq!(n.prepares, 2);
    assert_ne!(active(&next).review_seal, seal);
    assert_ne!(
        active(&next).entry.plan().key(),
        active(&prepared).entry.plan().key()
    );
    Ok(())
}

#[test]
fn safety_cancel_survives_goal_expiry_without_observe_or_plan_grants() -> Result<()> {
    let (t, c, _, mut n, mut clock) = fixture()?;
    let view = clock.prepare(&t, &request(&t), &mut n, &c)?;
    clock.commit(&t, active(&view).review_seal, true, &mut n, &c)?;
    let mut cleanup = c.clone();
    cleanup.anchor.tick = GameTick(900);
    cleanup
        .grants
        .retain(|g| matches!(g.capability, Capability::Query | Capability::ControlClock));
    let done = clock.cancel(&t, Some(&mut n), &cleanup)?;
    assert!(!done.pending);
    assert_eq!(n.cancels, 1);
    clock.cancel(&t, Some(&mut n), &cleanup)?;
    assert_eq!(n.cancels, 1);
    assert_eq!(n.commits, 1);
    Ok(())
}

#[test]
fn retention_bound_and_complete_json_do_not_prune_prior_runs() -> Result<()> {
    let (t, c, _, mut n, mut clock) = fixture()?;
    for index in 0..MAX_RUNS {
        let mut r = request(&t);
        r.wall_millis += index as u32;
        clock.prepare(&t, &r, &mut n, &c)?;
        clock.cancel::<Native>(&t, None, &c)?;
    }
    let full = clock.inspect(Some(&t), &c)?;
    assert_eq!(full.runs.len(), MAX_RUNS);
    assert!(
        serde_json::to_vec(&full.to_json())
            .map_err(|_| invalid("test serialization"))?
            .len()
            < 128 * 1024
    );
    let mut r = request(&t);
    r.wall_millis = 5000;
    assert!(clock.prepare(&t, &r, &mut n, &c).is_err());
    assert_eq!(n.prepares, MAX_RUNS as u32);
    assert_eq!(clock.inspect(Some(&t), &c)?.runs.len(), MAX_RUNS);
    // An exact old run is returned, never re-prepared or resurrected.
    clock.prepare(&t, &request(&t), &mut n, &c)?;
    assert_eq!(n.prepares, MAX_RUNS as u32);
    Ok(())
}

#[test]
fn offline_history_never_calls_native_and_injected_grants_do_not_promote_it() -> Result<()> {
    let (t, c, memory, mut n, mut clock) = fixture()?;
    clock.prepare(&t, &request(&t), &mut n, &c)?;
    drop(clock);
    let mut offline = open(memory, &n, &c, OrderRunMode::Offline, false)?;
    let view = offline.inspect(Some(&t), &c)?;
    assert_eq!(view.mode, OrderRunMode::Offline);
    let reads = n.reads;
    assert!(offline.reconcile(&t, &mut n, &c).is_err());
    assert_eq!(n.queries, 0);
    assert!(offline.prepare(&t, &request(&t), &mut n, &c).is_err());
    assert!(
        offline
            .commit(&t, active(&view).review_seal, true, &mut n, &c)
            .is_err()
    );
    assert!(offline.cancel(&t, Some(&mut n), &c).is_err());
    assert_eq!(n.reads, reads);
    Ok(())
}

#[test]
fn completed_order_can_advance_stability_and_quiescence_with_reactivation_guard() -> Result<()> {
    let (t, c, _, mut n, mut clock) = fixture()?;
    n.before = status(capture(41, 3, 100, 9, 1, 5, 0)?, 1)?;
    let prepared = clock.prepare(&t, &request(&t), &mut n, &c)?;
    assert_eq!(
        active(&prepared).entry.plan().spec().predicate(),
        OrderPredicate::Active
    );
    assert_eq!(
        prepared.to_json()["active_run"]["purpose"],
        "advance_for_original_stability_or_quiescence"
    );
    assert_eq!(prepared.to_json()["active_run"]["stop_predicate"], "active");
    let seal = active(&prepared).review_seal;
    let running = clock.commit(&t, seal, true, &mut n, &c)?;
    n.record = Some(native_record(active(&running).entry.plan(), 3, 1, 120)?);
    let stopped = clock.reconcile(&t, &mut n, &c)?;
    assert!(stopped.can_sample_goal());
    assert_eq!(stopped.high_tick, 120);
    assert_eq!(stopped.to_json()["original_goal_proven"], false);
    assert_eq!(
        stopped.to_json()["runs"][0]["native"]["predicate_observed"],
        false
    );
    Ok(())
}

#[test]
fn completed_order_requires_both_validation_and_inactivity() -> Result<()> {
    for flags in [0, 2, 3] {
        let (t, c, _, mut n, mut clock) = fixture()?;
        n.before = status(capture(41, 3, 100, 9, 1, 5, 0)?, flags)?;
        assert!(clock.prepare(&t, &request(&t), &mut n, &c).is_err());
        assert_eq!(n.prepares, 0);
    }
    Ok(())
}

#[test]
fn retained_observed_tick_never_renews_expired_query_or_clock_grants() -> Result<()> {
    let (t, c, _, mut n, mut clock) = fixture()?;
    let prepared = clock.prepare(&t, &request(&t), &mut n, &c)?;
    let running = clock.commit(&t, active(&prepared).review_seal, true, &mut n, &c)?;
    n.record = Some(native_record(active(&running).entry.plan(), 3, 1, 120)?);
    clock.reconcile(&t, &mut n, &c)?;
    let mut expired = c.clone();
    for grant in &mut expired.grants {
        grant.expires_at_tick = Some(GameTick(110));
    }
    assert!(clock.inspect(Some(&t), &expired).is_err());
    assert!(clock.cancel::<Native>(&t, None, &expired).is_err());
    assert_eq!(c.anchor.tick, GameTick(100));
    Ok(())
}

#[test]
fn rejected_complete_captures_retain_same_owner_clock_floors() -> Result<()> {
    let (t, mut c, _, mut n, mut clock) = fixture()?;
    let anchor = c.anchor;
    n.before = capture(41, 5, 110, 9, 1, 5, 5)?;
    assert!(clock.prepare(&t, &request(&t), &mut n, &c).is_err());
    let view = clock.inspect(Some(&t), &c)?;
    assert_eq!(view.high_tick, 110);
    assert_eq!(view.high_sequence, 5);
    assert!(!view.has_runs());
    assert_eq!(c.anchor, anchor);
    n.before = capture(41, 3, 100, 9, 1, 5, 5)?;
    assert!(clock.prepare(&t, &request(&t), &mut n, &c).is_err());
    c.anchor.tick = GameTick(110);
    n.before = capture(41, 4, 110, 9, 1, 5, 5)?;
    assert!(clock.prepare(&t, &request(&t), &mut n, &c).is_err());
    n.before = capture(41, 5, 110, 9, 1, 5, 5)?;
    clock.prepare(&t, &request(&t), &mut n, &c)?;
    assert_eq!(n.prepares, 1);
    Ok(())
}
