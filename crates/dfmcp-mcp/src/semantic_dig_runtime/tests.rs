//! Source-referencing tests of the full original dig runtime lifecycle.
use super::*;
#[path = "runtime_clock_tests.rs"]
mod runtime_clock_tests;
use runtime_clock_tests::{ClockNative, ClockResources};
#[allow(dead_code)]
#[path = "../../../dfmcp-adapter/tests/semantic_dig/fixtures.rs"]
pub(super) mod fixtures;
use dfmcp_adapter::dig_designation::DigPlan;
use dfmcp_adapter::dig_designation::journal::DigJournal;
use dfmcp_world::map_region::{Cell as MapCell, Shape};
use fixtures::{Evidence, EvidenceState, Memory, Native, SourceState};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
// Include the concrete default native evidence envelope in every joined case.
// The fixture still injects source I/O, while the runtime pays production-size
// owner acquisition and both native-boundary canonical refresh reservations.
const NATIVE_EVIDENCE_BYTES: u64 = 430 * MIB;
struct BudgetEvidence {
    inner: Evidence,
    reserve_bytes: u64,
}
impl DigEvidenceOwner for BudgetEvidence {
    fn refresh_reserve_bytes(&self) -> Result<u64> {
        Ok(self.reserve_bytes)
    }
    fn refresh_after_capture(
        &mut self,
        capture: &DigObservation,
        c: &OperationContext,
    ) -> Result<()> {
        if c.budget.max_bytes < self.reserve_bytes {
            return Err(exhausted("native-sized independent evidence allowance"));
        }
        self.inner.refresh_after_capture(capture, c)
    }
    fn routing_evidence(&self) -> Result<LiveRoutingEvidence<'_>> {
        self.inner.routing_evidence()
    }
}
#[derive(Default)]
struct GoalState {
    tick: u64,
    reads: usize,
    active: usize,
    policies: usize,
    denied: bool,
    corrupt_archive: bool,
    paused: bool,
    condition: &'static str,
}
struct GoalSource {
    state: Rc<RefCell<GoalState>>,
}
impl DigGoalSource for GoalSource {
    fn read_excavation(
        &mut self,
        original: &DigPlan,
        c: &OperationContext,
    ) -> Result<LiveSpatialCitizenObservation> {
        c.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        let mut state = self.state.borrow_mut();
        state.reads += 1;
        fixtures::goal_observation(original.before(), state.tick, |terrain| {
            terrain.paused = state.paused;
            if state.condition == "new_generation" {
                terrain.bridge_generation += 1;
            }
            let [x, y, z, w, h] = original.before().region().coordinates();
            for (index, cell) in terrain.map.cells.iter_mut().enumerate() {
                let Some(position) = terrain.map.region.position(index) else {
                    continue;
                };
                if position[0] >= x
                    && position[0] < x + w
                    && position[1] >= y
                    && position[1] < y + h
                    && position[2] == z
                {
                    match state.condition {
                        "hidden" => *cell = MapCell::Hidden,
                        "missing" => *cell = MapCell::Unallocated,
                        "pending" | "mixed" | "undesignated_wall" | "occupied_wall" => {
                            if let MapCell::Visible(tile) = cell {
                                tile.shape = if state.condition == "mixed" && position[0] == x {
                                    Shape::Floor
                                } else {
                                    Shape::Wall
                                };
                                tile.dig_designation = if state.condition == "undesignated_wall"
                                    || tile.shape == Shape::Floor
                                {
                                    0
                                } else {
                                    1
                                };
                                if state.condition == "occupied_wall" {
                                    tile.building_occupancy = 1;
                                }
                            }
                        }
                        _ => {
                            if let MapCell::Visible(tile) = cell {
                                tile.shape = Shape::Floor;
                                tile.dig_designation = if state.condition == "designated_floor" {
                                    1
                                } else {
                                    0
                                };
                                tile.liquid_depth = if state.condition == "wet" { 1 } else { 0 };
                            }
                        }
                    }
                }
            }
        })
    }
}
impl Drop for GoalSource {
    fn drop(&mut self) {
        let mut state = self.state.borrow_mut();
        state.active = state.active.saturating_sub(1);
    }
}
#[derive(Clone)]
struct Host {
    binding: DigBinding,
    native_disk: Memory,
    associations: Memory,
    archive: Memory,
    history: Memory,
    native: Rc<RefCell<SourceState>>,
    evidence: Rc<RefCell<EvidenceState>>,
    goal: Rc<RefCell<GoalState>>,
    enabled: Rc<Cell<bool>>,
    exception: Rc<Cell<bool>>,
    original_archive_ack_lost: Rc<Cell<bool>>,
    recovery: Option<Recovery>,
    policy: Rc<Cell<bool>>,
    open_calls: Rc<Cell<usize>>,
    evidence_cost: Rc<Cell<u64>>,
    evidence_opens: Rc<Cell<usize>>,
    clock: Option<ClockResources>,
    clock_terrain_enabled: Rc<Cell<bool>>,
}
impl Host {
    fn new() -> Result<Self> {
        let f = fixtures::Fixture::new()?;
        Ok(Self {
            binding: f.binding.clone(),
            native_disk: f.native_disk.clone(),
            associations: f.associations.clone(),
            archive: Memory::default(),
            history: Memory::default(),
            native: f.native.clone(),
            evidence: f.evidence_state.clone(),
            goal: Rc::new(RefCell::new(GoalState {
                tick: f.capture.tick(),
                paused: true,
                ..GoalState::default()
            })),
            enabled: Rc::new(Cell::new(true)),
            exception: Rc::new(Cell::new(true)),
            original_archive_ack_lost: Rc::new(Cell::new(false)),
            recovery: None,
            policy: Rc::new(Cell::new(true)),
            open_calls: Rc::new(Cell::new(0)),
            evidence_cost: Rc::new(Cell::new(NATIVE_EVIDENCE_BYTES)),
            evidence_opens: Rc::new(Cell::new(0)),
            clock: None,
            clock_terrain_enabled: Rc::new(Cell::new(true)),
        })
    }
    fn runtime(&self, mode: Mode) -> Result<Runtime<Self>> {
        self.runtime_with_bytes(
            mode,
            (if self.clock.is_some() {
                DEFAULT_CLOCK_BYTES
            } else {
                DEFAULT_BYTES
            }) - 256 * MIB
                - 128 * 1024,
        )
    }
    fn runtime_with_bytes(&self, mode: Mode, max_bytes: u64) -> Result<Runtime<Self>> {
        Runtime::from_shell(
            self.clone(),
            mode,
            SessionId::new(771),
            WorkBudget {
                max_wall_millis: 60_000,
                max_bytes,
                max_entities: 8192,
                max_actions: 1,
                max_output_tokens: MAX_OUTPUT_TOKENS,
                max_game_ticks: if self.clock.is_some() {
                    MAX_GAME_TICKS
                } else {
                    0
                },
            },
        )
    }
    fn recovered(&self, runtime: &Runtime<Self>, history_present: bool) -> Result<Self> {
        let mut out = self.clone();
        out.recovery = Some(Recovery {
            journal_id: runtime.journal,
            seal: runtime
                .archive_seal
                .ok_or_else(|| invalid("test original archive seal"))?,
            anchor: runtime
                .original
                .as_ref()
                .ok_or_else(|| invalid("test original"))?
                .anchor,
            history_present,
        });
        if let Some(clock) = &self.clock {
            let id = runtime
                .opening_clock
                .as_ref()
                .ok_or_else(|| invalid("retained clock absent"))?
                .journal_id;
            out.clock = Some(clock.recovered(id));
        }
        Ok(out)
    }
}
impl Shell for Host {
    type Journal = Memory;
    type Association = Memory;
    type Native = Native;
    type Archive = Memory;
    type History = Memory;
    type Evidence = BudgetEvidence;
    type Guard = fixtures::Guard;
    type GoalSource = GoalSource;
    type ClockJournal = Memory;
    type ClockSource = ClockNative;
    fn clock_enabled(&self) -> bool {
        self.clock.as_ref().is_some_and(ClockResources::enabled)
    }
    fn open_clock(&self, mode: Mode, c: &OperationContext) -> Result<Option<RunJournal<Memory>>> {
        self.clock
            .as_ref()
            .map(|clock| clock.open(mode, c))
            .transpose()
    }
    fn clock_context(&self, c: &OperationContext, control: bool) -> Result<OperationContext> {
        self.clock
            .as_ref()
            .ok_or_else(|| denied("clock not configured"))?
            .context(c, control)
    }
    fn clock_source(&self, c: &OperationContext) -> Result<ClockNative> {
        self.clock
            .as_ref()
            .ok_or_else(|| denied("clock not configured"))?
            .source(c)
    }
    fn authorize_clock_terrain(&self, p: &DigGoalProjection, c: &OperationContext) -> Result<()> {
        c.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        if !self.clock_terrain_enabled.get() || p.anchor() != c.anchor {
            return Err(denied("independent raw target policy unavailable"));
        }
        Ok(())
    }
    fn fortress(&self) -> FortressId {
        self.binding.fortress_id()
    }
    fn recovery(&self) -> Option<Recovery> {
        self.recovery
    }
    fn check_environment(&self) -> Result<()> {
        Ok(())
    }
    fn designation_enabled(&self) -> bool {
        self.enabled.get()
    }
    fn checkpoint_exception(&self) -> bool {
        self.exception.get()
    }
    fn open_control(
        &self,
        mode: Mode,
        c: &OperationContext,
    ) -> Result<(
        DigSession<Memory, Native>,
        AssociationStore<Memory>,
        Digest32,
    )> {
        self.open_calls.set(self.open_calls.get() + 1);
        let journal = DigJournal::open(
            self.native_disk.clone(),
            c,
            mode.journal(),
            Some(self.binding.clone()),
            None,
        )?;
        let id = journal.id();
        Ok((
            DigSession::new(journal, c)?,
            AssociationStore::open(
                self.associations.clone(),
                id,
                false,
                mode == Mode::Offline,
                c,
            )?,
            id,
        ))
    }
    fn native(
        &self,
        binding: &DigBinding,
        region: DigRegion,
        c: &OperationContext,
    ) -> Result<Native> {
        fixtures::factory(self.native.clone(), self.native_disk.clone())(binding, region, c)
    }
    fn guard(&self, _: Digest32, _: &OperationContext) -> Result<Self::Guard> {
        Ok(fixtures::Guard {
            permitted: self.policy.clone(),
            ..fixtures::Guard::default()
        })
    }
    fn acquire(&self, c: &OperationContext) -> Result<LiveSpatialCitizenObservation> {
        c.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        Ok(self.evidence.borrow().observation.clone())
    }
    fn spatial_policy(
        &self,
        state: &DigTerrainProjection,
        _: &OperationContext,
    ) -> Result<EvidencePolicy> {
        if !self.evidence.borrow().permitted {
            return Err(denied("test source policy revoked"));
        }
        Ok(fixtures::policy(state))
    }
    fn evidence_reserve_bytes(&self, _: &DigTerrainProjection) -> Result<u64> {
        Ok(self.evidence_cost.get())
    }
    fn evidence(
        &self,
        state: DigTerrainProjection,
        c: &OperationContext,
    ) -> Result<BudgetEvidence> {
        self.evidence_opens.set(self.evidence_opens.get() + 1);
        let reserve_bytes = self.evidence_cost.get();
        if c.budget.max_bytes < reserve_bytes {
            return Err(exhausted("native-sized original owner allowance"));
        }
        Ok(BudgetEvidence {
            inner: fixtures::evidence_owner(state, self.evidence.clone(), c)?,
            reserve_bytes,
        })
    }
    fn create_archive(
        &self,
        plan: &PreparedPlan,
        c: &OperationContext,
    ) -> Result<PlanArchiveStore<Memory>> {
        let archive = PlanArchiveStore::create(self.archive.clone(), plan, archive_limits(), c)?;
        if self.original_archive_ack_lost.get() {
            return Err(DfmcpError::new(
                ErrorCode::AdapterUnavailable,
                "archive persisted; acknowledgement lost",
            ));
        }
        Ok(archive)
    }
    fn recover_archive(
        &self,
        recovery: Recovery,
        c: &OperationContext,
    ) -> Result<PlanArchiveStore<Memory>> {
        PlanArchiveStore::open_at_authority_floor(
            self.archive.clone(),
            recovery.seal,
            archive_limits(),
            c,
        )
    }
    fn history(
        &self,
        journal: Digest32,
        review: &SemanticDigReview,
        initialize: bool,
        read_only: bool,
        c: &OperationContext,
    ) -> Result<GoalHistoryStore<Memory>> {
        GoalHistoryStore::open(
            self.history.clone(),
            journal,
            review,
            initialize,
            read_only,
            c,
        )
    }
    fn goal_source(&self, binding: &DigBinding, c: &OperationContext) -> Result<GoalSource> {
        c.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        assert_eq!(binding, &self.binding);
        self.goal.borrow_mut().active += 1;
        Ok(GoalSource {
            state: self.goal.clone(),
        })
    }
    fn goal_policy(&self, p: &DigGoalProjection, c: &OperationContext) -> Result<EvidencePolicy> {
        c.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        let mut state = self.goal.borrow_mut();
        assert_eq!(
            state.active, 0,
            "excavation source must close before independent policy"
        );
        state.policies += 1;
        if state.denied {
            return Err(denied("test excavation authority revoked"));
        }
        if state.corrupt_archive {
            self.archive.corrupt();
        }
        let mut policy = EvidencePolicy::at(p.anchor());
        policy.paused = true;
        policy.terrain_regions = p.terrain_regions().to_vec();
        policy.sources = p.evidence_sources();
        Ok(policy)
    }
}
fn observe_plan(runtime: &mut Runtime<Host>, stable: u32) -> Result<View> {
    let c = runtime.context(false)?;
    let observed = runtime.observe(&c)?;
    let seal = observed
        .observation
        .as_ref()
        .and_then(|v| v["seal"].as_str())
        .ok_or_else(|| invalid("test observation seal"))?
        .to_owned();
    let capture = runtime.shell.native.borrow().capture.clone();
    let request = Request {
        expected_observation: seal,
        kind: "mine".into(),
        target: capture.region().coordinates(),
        deadline_tick: capture.tick() + 1000,
        poll_interval_ticks: 5,
        stable_samples: stable,
    };
    let c = runtime.context(false)?;
    runtime.plan(request, &c)
}
fn commit(runtime: &mut Runtime<Host>) -> Result<View> {
    let seal = runtime
        .review
        .as_ref()
        .ok_or_else(|| invalid("test paired review"))?
        .seal();
    let c = runtime.context(false)?;
    runtime.commit(seal, true, &c)
}
fn wait(runtime: &mut Runtime<Host>) -> Result<View> {
    let c = runtime.context(false)?;
    runtime.wait(&c)
}
#[test]
fn interrupted_wait_withdraws_cached_proof_and_breaks_unfinished_stability() -> Result<()> {
    for before_runtime_dispatch in [false, true] {
        let host = Host::new()?;
        let mut runtime = host.runtime(Mode::Control)?;
        let planned = observe_plan(&mut runtime, 2)?;
        let original = planned.original.clone();
        commit(&mut runtime)?;
        wait(&mut runtime)?;
        host.goal.borrow_mut().tick += 5;
        let first = wait(&mut runtime)?;
        assert!(first.goal.is_some());
        assert!(
            !first
                .goal
                .as_ref()
                .ok_or_else(|| invalid("first interrupted-wait sample absent"))?
                .current_goal_proven()
        );
        let reads = host.goal.borrow().reads;
        let history = host.history.durable();

        if before_runtime_dispatch {
            // The facade can fail ambient context or budget checks before it
            // calls Runtime::wait. Its pure failure notification spends no I/O.
            runtime.mark_observation_interrupted();
        } else {
            let mut c = runtime.context(false)?;
            c.budget.max_bytes = 1;
            assert!(
                matches!(runtime.wait(&c), Err(cause) if cause.code == ErrorCode::BudgetExceeded)
            );
        }
        assert_eq!(host.goal.borrow().reads, reads);
        assert_eq!(host.history.durable(), history);
        let c = runtime.context(false)?;
        let interrupted = runtime.inspect(&c)?;
        assert!(interrupted.goal.is_none());
        assert_eq!(interrupted.to_json()["current_goal_available"], false);
        assert_eq!(interrupted.original, original);

        host.goal.borrow_mut().tick += 5;
        let resumed = wait(&mut runtime)?;
        assert!(
            !resumed
                .goal
                .as_ref()
                .ok_or_else(|| invalid("resumed interrupted-wait sample absent"))?
                .current_goal_proven()
        );
        host.goal.borrow_mut().tick += 5;
        let complete = wait(&mut runtime)?;
        assert!(
            complete
                .goal
                .as_ref()
                .ok_or_else(|| invalid("completed interrupted-wait sample absent"))?
                .current_goal_proven()
        );
        let achieved = complete.first_satisfied_anchor;
        assert!(achieved.is_some());

        // A later failed wait withdraws current proof, while the already
        // established historical achievement and original custody survive.
        let c = runtime.context(true)?;
        assert!(runtime.wait(&c).is_err());
        let c = runtime.context(false)?;
        let historical = runtime.inspect(&c)?;
        assert!(historical.goal.is_none());
        assert_eq!(historical.first_satisfied_anchor, achieved);
        assert_eq!(historical.original, original);
        host.goal.borrow_mut().tick += 5;
        let fresh = wait(&mut runtime)?;
        assert!(
            fresh
                .goal
                .as_ref()
                .ok_or_else(|| invalid("fresh post-achievement sample absent"))?
                .current_goal_proven()
        );
        assert_eq!(fresh.first_satisfied_anchor, achieved);
        assert_eq!(host.native.borrow().commits, 1);
    }
    Ok(())
}

#[test]
fn insufficient_complete_opening_budget_refuses_before_any_custody_io() -> Result<()> {
    let host = Host::new()?;
    let native_before = host.native_disk.durable();
    let associations_before = host.associations.durable();
    let result = Runtime::from_shell(
        host.clone(),
        Mode::Control,
        SessionId::new(771),
        WorkBudget {
            max_wall_millis: 60_000,
            max_bytes: 512 * MIB - 1,
            max_entities: 8192,
            max_actions: 1,
            max_output_tokens: MAX_OUTPUT_TOKENS,
            max_game_ticks: 0,
        },
    );
    assert!(matches!(result, Err(cause) if cause.code == ErrorCode::BudgetExceeded));
    assert_eq!(host.open_calls.get(), 0);
    assert_eq!(host.native.borrow().connections, 0);
    assert_eq!(host.native_disk.durable(), native_before);
    assert_eq!(host.associations.durable(), associations_before);
    assert!(host.archive.bytes().is_empty());
    Ok(())
}
#[test]
fn original_plan_places_once_and_two_cadenced_samples_prove_original_goal() -> Result<()> {
    let host = Host::new()?;
    let mut runtime = host.runtime(Mode::Control)?;
    let prepared = observe_plan(&mut runtime, 2)?;
    assert!(prepared.prepared_in_opening);
    assert_eq!(
        prepared.native.as_ref().map(|n| n.state()),
        Some(DigState::Prepared)
    );
    assert_eq!(prepared.to_json()["effect_uncertain"], false);
    let original = prepared
        .original
        .clone()
        .ok_or_else(|| invalid("original plan absent"))?;
    assert!(!original.requires_checkpoint);
    assert!(matches!(
        &original.steps[0].action,
        dfmcp_intent::Action::DesignateDig { .. }
    ));
    assert_eq!(
        original.steps[0].postconditions,
        dfmcp_intent::effects::default_postconditions(
            &original.steps[0].action,
            &original.steps[0].idempotency_key,
            original.anchor.fortress_id
        )
    );
    let placed = commit(&mut runtime)?;
    assert_eq!(host.native.borrow().commits, 1);
    assert!(placed.goal.is_none());
    assert_eq!(
        placed.to_json()["designation"]["excavation_complete"],
        false
    );
    let initialized = wait(&mut runtime)?;
    assert!(initialized.history_present);
    assert!(initialized.goal.is_none());
    assert_eq!(host.goal.borrow().reads, 0);
    let early = wait(&mut runtime)?;
    let early = early.goal.as_ref().ok_or_else(|| invalid("early sample"))?;
    assert!(early.excavation_proven);
    assert!(!early.current_goal_proven());
    assert!(matches!(
        early.obligation,
        Some(ObligationStatus::Active {
            consecutive_stable_observations: 0,
            ..
        })
    ));
    host.goal.borrow_mut().tick += 5;
    let first = wait(&mut runtime)?;
    assert!(
        !first
            .goal
            .as_ref()
            .ok_or_else(|| invalid("first excavation sample"))?
            .current_goal_proven()
    );
    host.goal.borrow_mut().tick += 5;
    let second = wait(&mut runtime)?;
    assert!(
        second
            .goal
            .as_ref()
            .ok_or_else(|| invalid("second excavation sample"))?
            .current_goal_proven()
    );
    assert_eq!(
        second.to_json()["current_goal"]["semantic_completion_proven"],
        true
    );
    assert_eq!(second.original, Some(original));
    assert_eq!(host.native.borrow().commits, 1);
    assert_eq!(host.goal.borrow().reads, 3);
    Ok(())
}
#[test]
fn lost_commit_reply_recovers_original_once_and_does_not_reacquire_dispatch_permission()
-> Result<()> {
    let host = Host::new()?;
    let mut runtime = host.runtime(Mode::Control)?;
    observe_plan(&mut runtime, 1)?;
    host.native.borrow_mut().lost_commit_reply = true;
    assert!(commit(&mut runtime).is_err());
    assert_eq!(host.native.borrow().commits, 1);
    assert!(commit(&mut runtime).is_err());
    let recovered = host.recovered(&runtime, false)?;
    drop(runtime);
    recovered.enabled.set(false);
    let mut runtime = recovered.runtime(Mode::Reconcile)?;
    let at_open = recovered.native.borrow().connections;
    assert!(!runtime.prepared_in_opening);
    assert!(runtime.evidence.is_none());
    let view = wait(&mut runtime)?;
    assert!(matches!(
        view.designation,
        Some(NativeDigDesignation::Designated { .. })
    ));
    assert_eq!(recovered.native.borrow().commits, 1);
    assert_eq!(recovered.native.borrow().connections, at_open + 1);
    Ok(())
}
#[test]
fn required_checkpoint_cannot_be_disabled_by_client_request() -> Result<()> {
    let host = Host::new()?;
    host.exception.set(false);
    let mut runtime = host.runtime(Mode::Control)?;
    assert!(observe_plan(&mut runtime, 1).is_err());
    assert!(host.archive.bytes().is_empty());
    assert_eq!(host.native.borrow().prepares, 0);
    Ok(())
}
#[test]
fn archive_changes_after_dispatch_marker_refuse_native_writer() -> Result<()> {
    let host = Host::new()?;
    let mut runtime = host.runtime(Mode::Control)?;
    observe_plan(&mut runtime, 1)?;
    let archive = host.archive.clone();
    host.native_disk.after_next_sync(move || archive.corrupt());
    assert!(commit(&mut runtime).is_err());
    assert_eq!(host.native.borrow().commits, 0);
    assert!(!runtime.prepared_in_opening);
    Ok(())
}
#[test]
fn lost_archive_acknowledgement_retains_original_and_reopens_without_a_native_permit() -> Result<()>
{
    let host = Host::new()?;
    host.original_archive_ack_lost.set(true);
    let mut runtime = host.runtime(Mode::Control)?;
    assert!(observe_plan(&mut runtime, 1).is_err());
    assert!(runtime.original.is_some());
    assert!(runtime.archive.is_none());
    let hint = runtime
        .recovery_hint()
        .ok_or_else(|| invalid("partial archive hint"))?;
    assert_eq!(hint["archive_publication_unknown"], true);
    assert!(!host.archive.durable().is_empty());
    assert_eq!(host.native.borrow().connections, 0);
    let recovered = host.recovered(&runtime, false)?;
    drop(runtime);
    let mut runtime = recovered.runtime(Mode::Offline)?;
    let c = runtime.context(false)?;
    let view = runtime.inspect(&c)?;
    assert!(view.original.is_some());
    assert!(view.review.is_none());
    assert!(view.native.is_none());
    assert!(!view.prepared_in_opening);
    assert_eq!(recovered.native.borrow().connections, 0);
    Ok(())
}
#[test]
fn durable_goal_reopen_preserves_achievement_but_requires_a_new_current_sample() -> Result<()> {
    let host = Host::new()?;
    let mut runtime = host.runtime(Mode::Control)?;
    observe_plan(&mut runtime, 1)?;
    commit(&mut runtime)?;
    wait(&mut runtime)?;
    host.goal.borrow_mut().tick += 5;
    let fulfilled = wait(&mut runtime)?;
    let first = fulfilled
        .first_satisfied_anchor
        .ok_or_else(|| invalid("first achievement"))?;
    let recovered = host.recovered(&runtime, true)?;
    let calls = host.native.borrow().connections;
    drop(runtime);
    recovered.enabled.set(false);
    let mut runtime = recovered.runtime(Mode::Reconcile)?;
    let c = runtime.context(false)?;
    let reopened = runtime.inspect(&c)?;
    assert!(reopened.goal.is_none());
    assert_eq!(reopened.first_satisfied_anchor, Some(first));
    assert!(!reopened.prepared_in_opening);
    assert_eq!(host.native.borrow().connections, calls);
    host.goal.borrow_mut().tick += 5;
    let current = wait(&mut runtime)?;
    assert!(
        current
            .goal
            .as_ref()
            .ok_or_else(|| invalid("fresh resumed goal"))?
            .current_goal_proven()
    );
    assert_eq!(host.native.borrow().commits, 1);
    Ok(())
}
#[test]
fn original_archive_is_rechecked_after_goal_source_closes_before_publication() -> Result<()> {
    let host = Host::new()?;
    let mut runtime = host.runtime(Mode::Control)?;
    observe_plan(&mut runtime, 1)?;
    commit(&mut runtime)?;
    wait(&mut runtime)?;
    host.goal.borrow_mut().corrupt_archive = true;
    assert!(wait(&mut runtime).is_err());
    assert_eq!(host.goal.borrow().reads, 1);
    assert_eq!(host.goal.borrow().active, 0);
    assert!(
        runtime
            .monitor
            .as_ref()
            .is_none_or(|m| m.latest().is_none())
    );
    assert_eq!(host.native.borrow().commits, 1);
    Ok(())
}

#[test]
fn unknown_hidden_and_wet_terrain_never_prove_original_excavation() -> Result<()> {
    for condition in ["hidden", "missing", "wet", "designated_floor", "pending"] {
        let host = Host::new()?;
        let mut runtime = host.runtime(Mode::Control)?;
        observe_plan(&mut runtime, 1)?;
        commit(&mut runtime)?;
        wait(&mut runtime)?;
        host.goal.borrow_mut().condition = condition;
        host.goal.borrow_mut().tick += 5;
        let value = wait(&mut runtime)?;
        assert_eq!(
            value.to_json()["current_goal"]["semantic_completion_proven"],
            false,
            "{condition}"
        );
        assert_eq!(host.native.borrow().commits, 1);
    }
    Ok(())
}
#[test]
fn changed_spatial_incarnation_cannot_reuse_original_goal_history() -> Result<()> {
    let host = Host::new()?;
    let mut runtime = host.runtime(Mode::Control)?;
    observe_plan(&mut runtime, 1)?;
    commit(&mut runtime)?;
    wait(&mut runtime)?;
    host.goal.borrow_mut().condition = "new_generation";
    host.goal.borrow_mut().tick += 5;
    assert!(wait(&mut runtime).is_err());
    assert_eq!(host.native.borrow().commits, 1);
    assert!(
        runtime
            .monitor
            .as_ref()
            .is_none_or(|m| m.latest().is_none())
    );
    Ok(())
}
#[test]
fn queried_prepared_can_retire_after_checkpoint_exception_revoked() -> Result<()> {
    let host = Host::new()?;
    let mut runtime = host.runtime(Mode::Control)?;
    observe_plan(&mut runtime, 1)?;
    let recovered = host.recovered(&runtime, false)?;
    drop(runtime);
    recovered.exception.set(false);
    let mut runtime = recovered.runtime(Mode::Reconcile)?;
    assert!(!runtime.designation_control_enabled());
    assert!(runtime.retirement_enabled());
    let queried = wait(&mut runtime)?;
    assert_eq!(queried.to_json()["native_state"], "tracking");
    assert_eq!(queried.to_json()["native_phase"], "prepared");
    assert_eq!(queried.to_json()["effect_uncertain"], false);
    let c = runtime.context(false)?;
    let retired = runtime.cancel(&c)?;
    assert_eq!(retired.to_json()["native_phase"], "refused");
    assert_eq!(host.native.borrow().cancels, 1);
    assert_eq!(host.native.borrow().commits, 0);
    Ok(())
}
#[test]
fn permanent_native_unknown_never_polls_or_retries_designation() -> Result<()> {
    let host = Host::new()?;
    let mut runtime = host.runtime(Mode::Control)?;
    observe_plan(&mut runtime, 1)?;
    host.native.borrow_mut().unknown_commit = true;
    let unknown = commit(&mut runtime)?;
    assert_eq!(unknown.to_json()["effect_uncertain"], true);
    assert_eq!(unknown.to_json()["native_phase"], "unknown");
    let connections = host.native.borrow().connections;
    wait(&mut runtime)?;
    let c = runtime.context(false)?;
    runtime.cancel(&c)?;
    assert_eq!(host.native.borrow().connections, connections);
    assert_eq!(host.native.borrow().commits, 1);
    assert_eq!(host.goal.borrow().reads, 0);
    Ok(())
}
#[test]
fn independent_policy_denial_consumes_private_cursor_without_current_proof_and_retry_advances()
-> Result<()> {
    let host = Host::new()?;
    let mut runtime = host.runtime(Mode::Control)?;
    observe_plan(&mut runtime, 1)?;
    commit(&mut runtime)?;
    wait(&mut runtime)?;
    let original = runtime.anchor;
    host.goal.borrow_mut().tick += 5;
    host.goal.borrow_mut().denied = true;
    assert!(wait(&mut runtime).is_err());
    assert_eq!(runtime.anchor, original);
    let denied_floor = runtime.authority.cursor;
    host.goal.borrow_mut().denied = false;
    host.goal.borrow_mut().tick += 5;
    let next = wait(&mut runtime)?;
    assert!(
        next.anchor
            .is_some_and(|a| a.cursor.sequence > denied_floor.sequence)
    );
    assert_eq!(host.native.borrow().commits, 1);
    Ok(())
}
#[test]
fn terrain_pages_require_current_source_seal_and_strict_mine_request_rejects_policy_fields()
-> Result<()> {
    let host = Host::new()?;
    let mut runtime = host.runtime(Mode::Control)?;
    let c = runtime.context(false)?;
    let observed = runtime.observe(&c)?;
    let seal = digest(
        observed
            .observation
            .as_ref()
            .and_then(|v| v["seal"].as_str())
            .ok_or_else(|| invalid("observation seal"))?,
    )?;
    let c = runtime.context(false)?;
    let page = runtime.tiles_page(seal, 32, 16, &c)?;
    assert_eq!(
        page.observation.as_ref().map(|v| v["returned"].clone()),
        Some(json!(16))
    );
    assert!(
        runtime
            .tiles_page(Digest32::of_bytes(b"other"), 0, 32, &c)
            .is_err()
    );
    let mut request = json!({"expected_observation":seal.to_hex(),"kind":"mine","target":host.native.borrow().capture.region().coordinates(),"deadline_tick":host.native.borrow().capture.tick()+100});
    Request::parse(&request.to_string())?;
    for field in [
        "native_key",
        "allow_hidden_neighbors",
        "checkpoint",
        "source_policy",
    ] {
        request[field] = json!(true);
        assert!(Request::parse(&request.to_string()).is_err());
        request
            .as_object_mut()
            .ok_or_else(|| invalid("request object"))?
            .remove(field);
    }
    request["kind"] = json!("channel");
    assert!(Request::parse(&request.to_string()).is_err());
    Ok(())
}

#[test]
fn complete_planning_budget_refuses_before_source_or_archive_and_preserves_the_basis() -> Result<()>
{
    let host = Host::new()?;
    host.evidence_cost.set(520 * MIB);
    let mut runtime = host.runtime_with_bytes(Mode::Control, DEFAULT_BYTES)?;
    let c = runtime.context(false)?;
    let observed = runtime.observe(&c)?;
    let basis = runtime
        .basis
        .as_ref()
        .ok_or_else(|| invalid("test basis"))?;
    let basis_seal = basis.seal;
    let basis_anchor = basis.state.anchor();
    let capture = host.native.borrow().capture.clone();
    let request = Request {
        expected_observation: basis_seal.to_hex(),
        kind: "mine".into(),
        target: capture.region().coordinates(),
        deadline_tick: capture.tick() + 1000,
        poll_interval_ticks: 5,
        stable_samples: 1,
    };
    let native_before = host.native_disk.durable();
    let associations_before = host.associations.durable();
    let mut c = runtime.context(false)?;
    c.budget.max_bytes = DEFAULT_BYTES - 256 * MIB - 128 * 1024;
    let result = runtime.plan(request.clone(), &c);
    assert!(matches!(result, Err(cause) if cause.code == ErrorCode::BudgetExceeded));
    assert_eq!(host.evidence_opens.get(), 0);
    assert!(host.archive.bytes().is_empty());
    assert!(runtime.original.is_none());
    assert!(runtime.archive.is_none());
    assert!(runtime.archive_seal.is_none());
    assert!(runtime.evidence.is_none());
    assert!(runtime.review.is_none());
    let retained = runtime
        .basis
        .as_ref()
        .ok_or_else(|| invalid("retained basis"))?;
    assert_eq!(retained.seal, basis_seal);
    assert_eq!(retained.state.anchor(), basis_anchor);
    assert_eq!(runtime.anchor, observed.anchor);
    assert_eq!(host.native_disk.durable(), native_before);
    assert_eq!(host.associations.durable(), associations_before);
    assert_eq!(host.native.borrow().connections, 0);
    assert_eq!(host.native.borrow().prepares, 0);

    // A request may use the already negotiated full allowance after refusal,
    // preserving the exact observation, intent and native idempotency identity.
    let c = runtime.context(false)?;
    let prepared = runtime.plan(request, &c)?;
    assert!(prepared.prepared_in_opening);
    assert_eq!(host.evidence_opens.get(), 1);
    assert_eq!(host.native.borrow().prepares, 1);
    assert_eq!(host.native.borrow().commits, 0);
    Ok(())
}
