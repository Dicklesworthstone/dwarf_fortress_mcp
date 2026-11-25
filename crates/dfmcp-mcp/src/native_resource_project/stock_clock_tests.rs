//! Real target and bounded-run coordinator regressions. Only storage/native
//! replies are fixtures; no manager order or Created receipt is invented.
use super::*;
use dfmcp_adapter::bounded_run::RunSpec;
use dfmcp_adapter::bounded_run::journal::{RunBinding, RunMode};
use dfmcp_adapter::bounded_run::rpc::RunManifest;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, GameTick, ObservationCursor, RequestId, SessionId, WorkBudget,
};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::net::SocketAddr;
use std::rc::Rc;

fn goal() -> Result<ProjectStockGoal> {
    serde_json::from_value(json!({
        "targets":[
            {"recipe":"wooden_bed","minimum_stack_units":5,"max_order_amount":10},
            {"recipe":"wooden_door","minimum_stack_units":2,"max_order_amount":3}
        ],
        "deadline_tick":200,"poll_interval_ticks":10,"stable_samples":2,"assign_labor":false,
    }))
    .map_err(|_| corrupt("fixture goal"))
}
fn original(goal: &ProjectStockGoal) -> Value {
    json!({
        "schema":"dfmcp.native-resource-project-original/1",
        "goal":goal, "directory":"/private/project", "stock_watch_clock":true,
        "workshop_policy":Digest32::of_bytes(b"workshop policy").to_hex(),
        "production_policy":Digest32::of_bytes(b"production policy").to_hex(),
    })
}
fn production() -> Value {
    json!({
        "schema":"dfmcp.project-production-recovery/1",
        "native_custody":{
            "workforce_journal_id":Digest32::of_bytes(b"workforce custody").to_hex(),
            "order_journal_id":Digest32::of_bytes(b"order custody").to_hex(),
        },
        "observation_tick_floor":100,
        "recovery":null, "original_goal_proven":false, "preparation_restored":false,
    })
}
fn context(fortress: FortressId) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1601),
        request_id: RequestId::new(1602),
        anchor: StateAnchor {
            fortress_id: fortress,
            cursor: ObservationCursor {
                epoch: 3,
                sequence: 1,
            },
            tick: GameTick(100),
            state_hash: Digest32::of_bytes(b"qualified paused stock"),
        },
        budget: WorkBudget {
            max_bytes: 512 * 1024 * 1024,
            max_wall_millis: 60_000,
            max_entities: 4096,
            max_actions: 1,
            max_output_tokens: 32768,
            max_game_ticks: 1200,
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
fn target() -> Result<WatchClockTarget> {
    let goal = goal()?;
    WatchClockTarget::new(
        Digest32::of_bytes(b"parent journal"),
        original_digest(&original(&goal))?,
        &goal,
        production_source_digest(&production())?,
        context(FortressId::new(7)).anchor,
    )
}
fn observation(tick: u64, sequence: u64, paused: bool) -> Result<RunObservation> {
    let mut bytes = b"DFMRO013".to_vec();
    for value in [41u64, sequence, tick] {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    bytes.extend_from_slice(&[1, 1, u8::from(paused)]);
    RunObservation::decode(&bytes)
}
fn plan(target: &WatchClockTarget, tick: u64, ticks: u32) -> Result<RunPlan> {
    let before = observation(tick, 1, true)?;
    let spec = RunSpec::new(ticks, 1000)?;
    let candidate = RunPlan::new(WatchClockTarget::CANDIDATE_KEY, spec, before.clone())?;
    RunPlan::new(&target.key(candidate.digest()), spec, before)
}
fn request(target: &WatchClockTarget) -> Result<RunRequest> {
    RunRequest::parse(
        &json!({
            "kind":"stock_watch_run", "original_paired_seal":target.original_paired_seal().to_hex(),
            "game_ticks":10,"wall_millis":1000,
        })
        .to_string(),
        WatchClockTarget::REQUEST_KIND,
    )
}

#[derive(Clone, Default)]
struct Memory(Rc<RefCell<Cursor<Vec<u8>>>>);
impl Read for Memory {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.0.borrow_mut().read(output)
    }
}
impl Write for Memory {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Seek for Memory {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.0.borrow_mut().seek(position)
    }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("no fixture repair"))
    }
}
fn receipt(
    plan: &RunPlan,
    phase: u8,
    reason: u8,
    tick: Option<u64>,
) -> Result<dfmcp_adapter::bounded_run::RunRecord> {
    let mut bytes = b"DFMRE013".to_vec();
    bytes.extend_from_slice(&plan.canonical_bytes());
    bytes.extend_from_slice(plan.digest().as_bytes());
    bytes.extend_from_slice(plan.token());
    bytes.extend_from_slice(&[
        phase,
        reason,
        u8::from(matches!(phase, 1..=3 | 5)),
        u8::from(phase == 3),
        u8::from(tick.is_some()),
    ]);
    bytes.extend_from_slice(&tick.unwrap_or(0).to_be_bytes());
    let mut identity = b"dfmcp-bounded-run-receipt/1\0".to_vec();
    identity.extend_from_slice(&bytes);
    bytes.extend_from_slice(Digest32::of_bytes(&identity).as_bytes());
    dfmcp_adapter::bounded_run::RunRecord::decode(&bytes)
}
struct Source {
    storage: Memory,
    manifest: RunManifest,
    records: BTreeMap<String, dfmcp_adapter::bounded_run::RunRecord>,
    tick: u64,
    sequence: u64,
    paused: bool,
    prepares: usize,
    commits: usize,
    queries: usize,
    cancels: usize,
    lose_commit: bool,
}
impl Source {
    fn check(&self, c: &OperationContext, control: bool) -> Result<()> {
        assert_eq!(c.anchor.fortress_id, FortressId::NIL);
        assert!(
            c.grants
                .iter()
                .all(|grant| grant.scope.fortress_id == Some(FortressId::NIL))
        );
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if control {
            c.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
        }
        Ok(())
    }
    fn marker(&self, c: &OperationContext, plan: &RunPlan, state: RunState) -> Result<()> {
        let mut journal = RunJournal::open(self.storage.clone(), c, RunMode::Offline, None, false)?;
        assert_eq!(
            journal
                .records(c)?
                .iter()
                .find(|row| row.plan() == plan)
                .map(|row| row.state()),
            Some(state),
        );
        Ok(())
    }
    fn stop(&mut self, tick: u64) -> Result<()> {
        let plan = self
            .records
            .values()
            .next()
            .ok_or_else(|| corrupt("missing fixture run"))?
            .plan()
            .clone();
        self.tick = tick;
        self.paused = true;
        self.records
            .insert(plan.key().into(), receipt(&plan, 3, 1, Some(tick))?);
        Ok(())
    }
}
impl RunSource for Source {
    fn manifest(&self) -> &RunManifest {
        &self.manifest
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        Some(SocketAddr::from(([127, 0, 0, 1], 5000)))
    }
    fn observe(&mut self, c: &OperationContext) -> Result<RunObservation> {
        self.check(c, false)?;
        observation(self.tick, self.sequence, self.paused)
    }
    fn prepare(
        &mut self,
        p: &RunPlan,
        c: &OperationContext,
    ) -> Result<dfmcp_adapter::bounded_run::RunRecord> {
        self.check(c, true)?;
        self.marker(c, p, RunState::Intent)?;
        self.prepares += 1;
        let record = receipt(p, 0, 0, None)?;
        self.records.insert(p.key().into(), record.clone());
        Ok(record)
    }
    fn commit(
        &mut self,
        p: &RunPlan,
        c: &OperationContext,
    ) -> Result<dfmcp_adapter::bounded_run::RunRecord> {
        self.check(c, true)?;
        self.marker(c, p, RunState::DispatchStarted)?;
        self.commits += 1;
        self.paused = false;
        self.sequence += 1;
        let record = receipt(p, 1, 0, Some(self.tick))?;
        self.records.insert(p.key().into(), record.clone());
        if self.lose_commit {
            Err(DfmcpError::new(
                ErrorCode::AdapterUnavailable,
                "lost native clock reply",
            ))
        } else {
            Ok(record)
        }
    }
    fn query(
        &mut self,
        p: &RunPlan,
        c: &OperationContext,
    ) -> Result<Option<dfmcp_adapter::bounded_run::RunRecord>> {
        self.check(c, false)?;
        self.queries += 1;
        Ok(self.records.get(p.key()).cloned())
    }
    fn cancel(
        &mut self,
        p: &RunPlan,
        c: &OperationContext,
    ) -> Result<dfmcp_adapter::bounded_run::RunRecord> {
        self.check(c, true)?;
        self.marker(c, p, RunState::CancelRequested)?;
        self.cancels += 1;
        self.paused = true;
        let record = receipt(p, 3, 3, Some(self.tick))?;
        self.records.insert(p.key().into(), record.clone());
        Ok(record)
    }
}
fn setup() -> Result<(
    StockClock<Memory>,
    Memory,
    Source,
    OperationContext,
    OperationContext,
)> {
    let c = context(FortressId::new(7));
    let n = context(FortressId::NIL);
    let storage = Memory::default();
    let source = Source {
        storage: storage.clone(),
        manifest: RunManifest {
            generation: 41,
            df_version: "fixture-df".into(),
            dfhack_version: "fixture-dfhack".into(),
        },
        records: BTreeMap::new(),
        tick: 100,
        sequence: 1,
        paused: true,
        prepares: 0,
        commits: 0,
        queries: 0,
        cancels: 0,
        lose_commit: false,
    };
    let binding = RunBinding::new(
        source
            .endpoint()
            .ok_or_else(|| corrupt("fixture endpoint"))?,
        source.manifest.clone(),
    )?;
    let journal = RunJournal::open(storage.clone(), &n, RunMode::Control, Some(binding), true)?;
    Ok((StockClock::new(journal, target()?), storage, source, c, n))
}
fn marker_guard(storage: &Memory, n: &OperationContext, expected: RunState) -> Result<()> {
    let mut journal = RunJournal::open(storage.clone(), n, RunMode::Offline, None, false)?;
    let rows = journal.records(n)?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].state(), expected);
    Ok(())
}

#[test]
fn durable_target_keeps_every_original_minimum_and_stable_native_custodian() -> Result<()> {
    let target = target()?;
    let goal = goal()?;
    let parent = Digest32::of_bytes(b"parent journal");
    let original = original(&goal);
    let source = production();
    target.matches_original(
        parent,
        original_digest(&original)?,
        &goal,
        production_source_digest(&source)?,
    )?;
    let restored = WatchClockTarget::from_json(&target.to_json())?;
    assert_eq!(restored, target);
    assert!(target.to_json().get("native_order_id").is_none());
    assert!(target.to_json().get("goal_witness").is_none());

    let mut later = source.clone();
    later["observation_tick_floor"] = json!(190);
    later["clock"] = json!({"head":Digest32::of_bytes(b"later head").to_hex()});
    assert_eq!(
        production_source_digest(&later)?,
        production_source_digest(&source)?
    );
    later["native_custody"]["order_journal_id"] =
        json!(Digest32::of_bytes(b"other order custody").to_hex());
    assert!(
        target
            .matches_original(
                parent,
                original_digest(&original)?,
                &goal,
                production_source_digest(&later)?
            )
            .is_err()
    );
    for field in [
        "deadline_tick",
        "poll_interval_ticks",
        "stable_samples",
        "assign_labor",
    ] {
        let mut value = serde_json::to_value(&goal).map_err(|_| corrupt("fixture goal encode"))?;
        value[field] = match field {
            "deadline_tick" => json!(201),
            "poll_interval_ticks" => json!(11),
            "stable_samples" => json!(3),
            _ => json!(true),
        };
        let changed: ProjectStockGoal =
            serde_json::from_value(value).map_err(|_| corrupt("fixture changed goal"))?;
        assert!(target.matches_goal(&changed).is_err(), "{field}");
    }
    let mut value = serde_json::to_value(&goal).map_err(|_| corrupt("fixture goal encode"))?;
    value["targets"]
        .as_array_mut()
        .ok_or_else(|| corrupt("fixture target vector"))?
        .pop();
    let dropped: ProjectStockGoal =
        serde_json::from_value(value).map_err(|_| corrupt("fixture goal"))?;
    assert!(target.matches_goal(&dropped).is_err());
    Ok(())
}

#[test]
fn recovery_descriptor_is_closed_and_preserves_absolute_deadline_after_expiry() -> Result<()> {
    let target = target()?;
    let pin = ClockCheckpoint::prospective(target.clone(), Digest32::of_bytes(b"clock header"))?;
    assert!(pin.is_initial());
    assert_eq!(ClockCheckpoint::from_json(&pin.to_json())?, pin);
    let mut unknown = target.to_json();
    unknown["native_order_id"] = json!(7);
    assert!(WatchClockTarget::from_json(&unknown).is_err());
    let mut changed = target.to_json();
    changed["deadline_tick"] = json!(201);
    assert!(WatchClockTarget::from_json(&changed).is_err());
    let mut malformed = pin.to_json();
    malformed["restored_review"] = json!(true);
    assert!(ClockCheckpoint::from_json(&malformed).is_err());

    let mut advanced = pin.to_json();
    advanced["head"] = json!(Digest32::of_bytes(b"later clock head").to_hex());
    advanced["authority_tick_floor"] = json!(210);
    advanced["high_source_sequence"] = json!(2);
    let advanced = ClockCheckpoint::from_json(&advanced)?;
    pin.validate_successor(&advanced)?;
    assert!(advanced.validate_successor(&pin).is_err());
    let mut after = context(FortressId::new(7));
    after.anchor.tick = GameTick(250);
    let recovered = WatchClockTarget::from_json(&advanced.target().to_json())?;
    recovered.context(&after)?;
    recovered.plan(&plan(&recovered, 190, 10)?)?;
    assert!(recovered.plan(&plan(&recovered, 190, 11)?).is_err());
    assert!(recovered.plan(&plan(&recovered, 200, 1)?).is_err());
    Ok(())
}

#[test]
fn no_order_clock_review_and_single_unpause_use_actual_durable_coordinator() -> Result<()> {
    let (mut clock, storage, mut source, c, n) = setup()?;
    let request = request(clock.target())?;
    let prepared = clock.prepare_guarded(&request, &mut source, &c, &n, |_| {
        marker_guard(&storage, &n, RunState::Intent)
    })?;
    let seal = prepared
        .active()
        .ok_or_else(|| corrupt("fixture prepared clock"))?
        .review_seal;
    assert_eq!(source.prepares, 1);
    assert_eq!(source.commits, 0);
    assert_eq!(clock.current_review(), Some(seal));
    clock.commit_guarded(seal, true, &mut source, &c, &n, |_| {
        marker_guard(&storage, &n, RunState::DispatchStarted)
    })?;
    assert_eq!(source.commits, 1);
    assert!(clock.may_be_running());
    assert!(
        clock
            .commit_guarded(seal, true, &mut source, &c, &n, |_| Ok(()))
            .is_err()
    );
    assert_eq!(source.commits, 1);
    source.stop(110)?;
    let mut current = c.clone();
    current.anchor.tick = GameTick(110);
    let mut native = n.clone();
    native.anchor.tick = GameTick(110);
    let settled = clock.reconcile(&mut source, &current, &native)?;
    assert!(!settled.pending && !settled.unresolved);
    assert_eq!(settled.high_tick, 110);
    assert_eq!(clock.to_json()["original_goal_proven"], false);
    assert_eq!(clock.to_json()["current_pause_proven"], false);
    assert_eq!(clock.to_json()["sample_history_durable"], false);
    Ok(())
}

#[test]
fn final_guard_refusal_keeps_dispatch_marker_without_unpause_or_current_review() -> Result<()> {
    let (mut clock, storage, mut source, c, n) = setup()?;
    let request = request(clock.target())?;
    let prepared = clock.prepare_guarded(&request, &mut source, &c, &n, |_| Ok(()))?;
    let seal = prepared
        .active()
        .ok_or_else(|| corrupt("fixture prepared clock"))?
        .review_seal;
    let result = clock.commit_guarded(seal, true, &mut source, &c, &n, |_| {
        marker_guard(&storage, &n, RunState::DispatchStarted)?;
        Err(denied("fresh original stock is no longer True"))
    });
    assert!(result.is_err());
    assert_eq!(source.commits, 0);
    assert!(clock.current_review().is_none());
    assert!(clock.may_be_running());
    let mut retained = RunJournal::open(storage, &n, RunMode::Offline, None, false)?;
    assert_eq!(retained.records(&n)?[0].state(), RunState::DispatchStarted);
    assert!(
        clock
            .commit_guarded(seal, true, &mut source, &c, &n, |_| Ok(()))
            .is_err()
    );
    assert_eq!(source.commits, 0);
    Ok(())
}

#[test]
fn prepared_recovery_never_restores_confirmation_and_foreign_target_cannot_adopt_history()
-> Result<()> {
    let (mut clock, storage, mut source, c, n) = setup()?;
    let request = request(clock.target())?;
    let prepared = clock.prepare_guarded(&request, &mut source, &c, &n, |_| Ok(()))?;
    let seal = prepared
        .active()
        .ok_or_else(|| corrupt("fixture prepared clock"))?
        .review_seal;
    let checkpoint = clock.recovery_checkpoint();
    // Even unchanged source floors cannot erase acknowledged prepared work by
    // replacing its current head with the earlier empty journal header.
    let prepared_checkpoint = ClockCheckpoint::from_json(&checkpoint)?;
    assert_ne!(prepared_checkpoint.head(), prepared_checkpoint.journal_id());
    let mut erased = checkpoint.clone();
    erased["head"] = json!(prepared_checkpoint.journal_id().to_hex());
    assert!(
        prepared_checkpoint
            .validate_successor(&ClockCheckpoint::from_json(&erased)?)
            .is_err()
    );
    let target = WatchClockTarget::from_json(&clock.target().to_json())?;
    drop(clock);
    let journal = RunJournal::open(storage.clone(), &n, RunMode::Control, None, false)?;
    let mut reopened = StockClock::new(journal, target);
    reopened.verify_recovery(&checkpoint, &c, &n)?;
    reopened.inspect(&c, &n)?;
    assert!(
        !reopened.to_json()["review_available"]
            .as_bool()
            .unwrap_or(true)
    );
    assert!(
        reopened
            .commit_guarded(seal, true, &mut source, &c, &n, |_| Ok(()))
            .is_err()
    );
    assert_eq!(source.commits, 0);
    assert_eq!(source.prepares, 1);

    let goal = goal()?;
    let other = WatchClockTarget::new(
        Digest32::of_bytes(b"other parent"),
        original_digest(&original(&goal))?,
        &goal,
        production_source_digest(&production())?,
        c.anchor,
    )?;
    let journal = RunJournal::open(storage, &n, RunMode::Offline, None, false)?;
    let mut foreign = StockClock::new(journal, other);
    assert!(foreign.verify_recovery(&checkpoint, &c, &n).is_err());
    assert!(foreign.inspect(&c, &n).is_err());
    Ok(())
}

#[test]
fn lost_reply_recovers_original_key_and_safety_cancel_survives_goal_expiry() -> Result<()> {
    let (mut clock, storage, mut source, c, n) = setup()?;
    let request = request(clock.target())?;
    let prepared = clock.prepare_guarded(&request, &mut source, &c, &n, |_| Ok(()))?;
    let seal = prepared
        .active()
        .ok_or_else(|| corrupt("fixture prepared clock"))?
        .review_seal;
    source.lose_commit = true;
    assert!(
        clock
            .commit_guarded(seal, true, &mut source, &c, &n, |_| Ok(()))
            .is_err()
    );
    assert_eq!(source.commits, 1);
    let checkpoint = clock.recovery_checkpoint();
    let failed_view = clock.to_json();
    assert_eq!(failed_view["inspection_required"], true);
    assert_eq!(failed_view["review_available"], false);
    assert_eq!(failed_view["head"], prepared.head.to_hex());
    assert_ne!(checkpoint["head"], failed_view["head"]);
    assert_eq!(failed_view["recovery_checkpoint"], checkpoint);
    assert_eq!(
        failed_view["high_source_sequence"],
        checkpoint["high_source_sequence"]
    );
    let target = WatchClockTarget::from_json(&clock.target().to_json())?;
    drop(clock);
    let mut canonical = c;
    let mut native = n;
    for context in [&mut canonical, &mut native] {
        context.anchor.tick = GameTick(250);
        context.grants.retain(|grant| {
            matches!(
                grant.capability,
                Capability::Query | Capability::ControlClock
            )
        });
    }
    let journal = RunJournal::open(storage, &native, RunMode::Control, None, false)?;
    let mut recovered = StockClock::new(journal, target);
    recovered.verify_recovery(&checkpoint, &canonical, &native)?;
    recovered.inspect(&canonical, &native)?;
    assert!(recovered.may_be_running());
    source.tick = 250;
    recovered.cancel(Some(&mut source), &canonical, &native)?;
    assert_eq!(source.commits, 1);
    assert_eq!(source.cancels, 1);
    assert!(!recovered.may_be_running());
    assert!(recovered.current_review().is_none());
    assert_eq!(recovered.to_json()["original_goal_proven"], false);
    Ok(())
}

#[test]
fn clock_cannot_borrow_fortress_authority_or_exceed_any_interval_bound() -> Result<()> {
    let (mut clock, _, mut source, c, n) = setup()?;
    let request = request(clock.target())?;
    assert!(
        clock
            .prepare_guarded(&request, &mut source, &c, &c, |_| Ok(()))
            .is_err()
    );
    let mut no_clock = c.clone();
    no_clock
        .grants
        .retain(|grant| grant.capability != Capability::ControlClock);
    assert!(
        clock
            .prepare_guarded(&request, &mut source, &no_clock, &n, |_| Ok(()))
            .is_err()
    );
    let mut no_ticks = c.clone();
    no_ticks.budget.max_game_ticks = 0;
    assert!(
        clock
            .prepare_guarded(&request, &mut source, &no_ticks, &n, |_| Ok(()))
            .is_err()
    );
    let mut no_native_ticks = n;
    no_native_ticks.budget.max_game_ticks = 0;
    assert!(
        clock
            .prepare_guarded(&request, &mut source, &c, &no_native_ticks, |_| Ok(()))
            .is_err()
    );
    let mut over_deadline = request.clone();
    over_deadline.game_ticks = 101;
    assert!(
        clock
            .prepare_guarded(
                &over_deadline,
                &mut source,
                &c,
                &context(FortressId::NIL),
                |_| Ok(())
            )
            .is_err()
    );
    assert_eq!(source.prepares, 0);
    assert_eq!(source.commits, 0);
    let mut selected = json!({
        "kind":"stock_watch_run","original_paired_seal":clock.target().original_paired_seal().to_hex(),
        "game_ticks":10,"wall_millis":1000,
    });
    selected["order_step_id"] = json!(0);
    assert!(RunRequest::parse(&selected.to_string(), WatchClockTarget::REQUEST_KIND).is_err());
    Ok(())
}

#[test]
fn retained_stock_interval_gets_a_fresh_review_without_repreparing_or_unpausing() -> Result<()> {
    let (mut clock, storage, mut source, mut c, mut n) = setup()?;
    let mut request = request(clock.target())?;
    request.wall_millis = 60_000;
    let prepared = clock.prepare_guarded(&request, &mut source, &c, &n, |_| Ok(()))?;
    let original = prepared
        .active()
        .ok_or_else(|| corrupt("fixture original run"))?;
    let seal = original.review_seal;
    let plan = original.entry.plan().clone();
    let bytes = storage.0.borrow().get_ref().clone();
    clock.withdraw_review();
    assert!(clock.current_review().is_none());
    assert!(
        clock
            .commit_guarded(seal, true, &mut source, &c, &n, |_| Ok(()))
            .is_err()
    );
    clock.inspect(&c, &n)?;
    // Native wall time is asynchronous; fresh census/RPC work has its own
    // remaining foreground deadline and cannot shorten the retained interval.
    c.budget.max_wall_millis = 59_999;
    n.budget.max_wall_millis = 59_999;
    let offered = clock
        .retained_review_request(c.anchor.tick.get(), c.budget)
        .ok_or_else(|| corrupt("missing retained review request"))?;
    assert_eq!(offered, request);
    let mut checked = 0;
    let reviewed = clock.prepare_guarded(&offered, &mut source, &c, &n, |edge| {
        assert_eq!(edge.anchor, c.anchor);
        marker_guard(&storage, &n, RunState::Prepared)?;
        checked += 1;
        Ok(())
    })?;
    assert_eq!(
        checked, 1,
        "re-review skipped current stock/custody qualification"
    );
    assert_eq!(reviewed.active().map(|run| run.entry.plan()), Some(&plan));
    assert_eq!(reviewed.head, prepared.head);
    assert_eq!(storage.0.borrow().get_ref(), &bytes);
    assert_eq!(source.prepares, 1);
    assert_eq!(source.commits, 0);
    assert_eq!(clock.current_review(), Some(seal));
    clock.commit_guarded(seal, true, &mut source, &c, &n, |_| Ok(()))?;
    assert_eq!(source.commits, 1);
    assert!(
        clock
            .prepare_guarded(&offered, &mut source, &c, &n, |_| Ok(()))
            .is_ok()
    );
    assert!(
        clock.current_review().is_none(),
        "dispatched work regained a review"
    );
    assert!(
        clock
            .commit_guarded(seal, true, &mut source, &c, &n, |_| Ok(()))
            .is_err()
    );
    assert_eq!(source.prepares, 1);
    assert_eq!(source.commits, 1);
    Ok(())
}

#[test]
fn retained_stock_review_rechecks_full_interval_authority_and_current_parent_evidence() -> Result<()>
{
    for case in 0..13 {
        let (mut clock, storage, mut source, c, n) = setup()?;
        let request = request(clock.target())?;
        clock.prepare_guarded(&request, &mut source, &c, &n, |_| Ok(()))?;
        clock.withdraw_review();
        let bytes = storage.0.borrow().get_ref().clone();
        let mut changed_request = request.clone();
        let mut current = c.clone();
        let mut native = n.clone();
        match case {
            0 => current.budget.max_game_ticks = 9,
            1 => native.budget.max_game_ticks = 9,
            2 => current
                .grants
                .retain(|grant| grant.capability != Capability::ControlClock),
            3 => native
                .grants
                .retain(|grant| grant.capability != Capability::Plan),
            4 => current
                .grants
                .retain(|grant| grant.capability != Capability::Plan),
            5 => native
                .grants
                .retain(|grant| grant.capability != Capability::ControlClock),
            6 | 7 => {
                let grants = if case == 6 {
                    &mut current.grants
                } else {
                    &mut native.grants
                };
                for grant in grants {
                    if grant.capability == Capability::ControlClock {
                        grant.expires_at_tick = Some(GameTick(105));
                    }
                }
            }
            8 => current.anchor.tick = GameTick(101),
            9 => native.anchor.tick = GameTick(101),
            10 => changed_request.game_ticks = 5,
            11 => changed_request.wall_millis = 500,
            _ => {}
        }
        let result =
            clock.prepare_guarded(&changed_request, &mut source, &current, &native, |_| {
                if case == 12 {
                    Err(denied("fresh complete stock is False"))
                } else {
                    Ok(())
                }
            });
        assert!(
            result.is_err(),
            "retained review accepted changed boundary {case}"
        );
        assert!(clock.current_review().is_none(), "case {case}");
        assert_eq!(storage.0.borrow().get_ref(), &bytes, "case {case}");
        assert_eq!(source.prepares, 1, "case {case}");
        assert_eq!(source.commits, 0, "case {case}");
    }
    Ok(())
}
