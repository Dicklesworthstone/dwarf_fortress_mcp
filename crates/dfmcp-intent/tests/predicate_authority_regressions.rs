#![forbid(unsafe_code)]

//! Source authority and complete-domain witnesses at planning and proof boundaries.

use std::collections::BTreeMap;

use dfmcp_core::{
    ActionId, Capability, CapabilityGrant, CapabilityScope, CommitState, Digest32, EntityId,
    ErrorCode, FortressId, GameTick, IntentId, ObservationCursor, OperationContext, RequestId,
    Result, RiskTier, SessionId, StepId, WorkBudget,
};
use dfmcp_intent::execution::{
    DeferredStepDecision, deferred_step_decision, deferred_step_decision_with_evidence,
};
use dfmcp_intent::{
    Action, Constraint, Intent, ObligationRuntime, ObligationSpec, ObligationStatus, PlanStep,
    RecoveredObligation, RequestedAction, StaticPlanner,
};
use dfmcp_world::{
    CompareOp, EntityKind, EntityRecord, EvidenceCoverage, EvidencePolicy, EvidenceSource, Fact,
    FactSource, Predicate, PredicateEvidence, Value, WorldGraph, WorldSnapshot,
};

const UNIT: EntityId = EntityId::new(1);
const MISSING: EntityId = EntityId::new(99);
const LAB_SOURCE: &str = "dfmcp.lab-scenario/1";

fn digest() -> Digest32 {
    Digest32::of_bytes(b"admitted test observation contents")
}

fn snapshot(tick: u64, epoch: u64, ready: bool, source: FactSource, digest: Digest32) -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    graph.entities.insert(
        UNIT,
        EntityRecord {
            id: UNIT,
            generation: 1,
            revision: tick.max(1),
            kind: EntityKind::Unit,
            label: "Urist".to_owned(),
            fields: BTreeMap::from([(
                "ready".to_owned(),
                Fact::known(Value::Bool(ready), GameTick(tick), source, digest),
            )]),
        },
    );
    WorldSnapshot::new(
        FortressId::new(1),
        GameTick(tick),
        ObservationCursor { epoch, sequence: tick },
        true,
        graph,
    )
}

fn laboratory_snapshot(tick: u64, epoch: u64, ready: bool) -> WorldSnapshot {
    snapshot(tick, epoch, ready, FactSource::Derived(LAB_SOURCE.to_owned()), Digest32::ZERO)
}

fn ready(expected: bool) -> Predicate {
    Predicate::FieldCompare {
        entity_id: UNIT,
        field: "ready".to_owned(),
        op: CompareOp::Eq,
        value: Value::Bool(expected),
    }
}

fn missing() -> Predicate {
    Predicate::Not(Box::new(Predicate::EntityExists(MISSING)))
}

fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget::CONSERVATIVE_DEFAULT,
        grants: [Capability::Plan, Capability::ControlClock].into_iter().map(|capability| CapabilityGrant {
            capability,
            scope: CapabilityScope::default(),
            max_risk: RiskTier::Reversible,
            expires_at_tick: None,
            remaining_uses: None,
        }).collect(),
        cancellation_requested: false,
    }
}

fn intent(snapshot: &WorldSnapshot, preconditions: Vec<Predicate>) -> Intent {
    Intent {
        id: IntentId::new(1),
        anchor: snapshot.anchor(),
        summary: "resume only when the worker state is established".to_owned(),
        terminal_condition: Predicate::Paused(false),
        constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions: vec![RequestedAction {
            action: Action::Pause { paused: false },
            preconditions,
            postconditions: Vec::new(),
            compensation: None,
            obligation: None,
            depends_on: Vec::new(),
        }],
    }
}

fn step(preconditions: Vec<Predicate>) -> PlanStep {
    PlanStep {
        id: StepId::new(1),
        action: Action::Pause { paused: false },
        preconditions,
        postconditions: vec![Predicate::Paused(false)],
        compensation: None,
        obligation: None,
        depends_on: vec![StepId::new(0)],
        risk: RiskTier::Reversible,
        required_capability: Capability::ControlClock,
        idempotency_key: "authority-regression-step".to_owned(),
    }
}

fn obligation(terminal: Predicate, failure: Option<Predicate>, stable: u32) -> ObligationSpec {
    ObligationSpec {
        terminal,
        failure,
        deadline_tick: GameTick(100),
        poll_interval_ticks: 10,
        stable_for_observations: stable,
    }
}

fn scoped_policy(snapshot: &WorldSnapshot) -> EvidencePolicy {
    let mut policy = EvidencePolicy::at(snapshot.anchor());
    policy.all_entities = EvidenceCoverage::Observed;
    policy.sources.insert(EvidenceSource::Observed {
        field: "unit.ready".to_owned(),
        source_digest: digest(),
    });
    policy
}

fn assert_streak(status: Option<&ObligationStatus>, expected: u32) {
    assert!(matches!(status, Some(ObligationStatus::Active {
        consecutive_stable_observations, ..
    }) if *consecutive_stable_observations == expected));
}

#[test]
fn conservative_planning_cannot_consume_a_claimed_known_precondition() -> Result<()> {
    let planner = StaticPlanner::default();
    for (source, source_digest) in [
        (FactSource::AgentAssertion("operator says ready".to_owned()), Digest32::ZERO),
        (FactSource::Derived("unregistered prediction".to_owned()), digest()),
        (FactSource::DfhackField("unit.ready".to_owned()), digest()),
        (FactSource::Replay, digest()),
        (FactSource::Derived(LAB_SOURCE.to_owned()), Digest32::ZERO),
    ] {
        let snapshot = snapshot(10, 0, true, source, source_digest);
        assert!(dfmcp_world::evaluate(&snapshot, &ready(true)));
        let request = intent(&snapshot, vec![ready(true)]);
        assert!(matches!(planner.prepare(&snapshot, &request, &context(&snapshot)), Err(error) if error.code == ErrorCode::PreconditionsFailed));
    }
    let snapshot = laboratory_snapshot(10, 0, true);
    let evidence = PredicateEvidence::laboratory(&snapshot)?;
    let plan = planner.prepare_with_evidence(&evidence, &intent(&snapshot, vec![ready(true)]), &context(&snapshot))?;
    assert!(plan.digest_is_valid());
    assert_eq!(plan.anchor, snapshot.anchor());
    Ok(())
}

#[test]
fn untrusted_terminal_claim_cannot_suppress_work_as_already_achieved() -> Result<()> {
    let planner = StaticPlanner::default();
    let snapshot = laboratory_snapshot(10, 0, true);
    let mut request = intent(&snapshot, Vec::new());
    request.terminal_condition = ready(true);
    // With no observed precondition, candidate generation may continue. Raw
    // values cannot be used to report that the requested goal is already done.
    let plan = planner.prepare(&snapshot, &request, &context(&snapshot))?;
    assert!(plan.digest_is_valid());
    let evidence = PredicateEvidence::laboratory(&snapshot)?;
    assert!(matches!(planner.prepare_with_evidence(&evidence, &request, &context(&snapshot)), Err(error) if error.code == ErrorCode::InvalidIntent && error.message.contains("already satisfied")));
    Ok(())
}

#[test]
fn scoped_source_and_exact_anchor_gate_planning_and_deferred_dispatch() -> Result<()> {
    let snapshot = snapshot(10, 0, true, FactSource::DfhackField("unit.ready".to_owned()), digest());
    let planner = StaticPlanner::default();
    let request = intent(&snapshot, vec![ready(true)]);
    let work = step(vec![ready(true)]);
    let policy = scoped_policy(&snapshot);
    let evidence = PredicateEvidence::scoped(&snapshot, policy.clone())?;
    assert!(planner.prepare_with_evidence(&evidence, &request, &context(&snapshot))?.digest_is_valid());
    assert_eq!(deferred_step_decision_with_evidence(&work, &evidence, |_| Some(CommitState::Verified))?, DeferredStepDecision::Ready);

    let mut forged = policy.clone();
    forged.sources = [EvidenceSource::Observed {
        field: "unit.ready".to_owned(), source_digest: Digest32::of_bytes(b"different frame"),
    }].into_iter().collect();
    let insufficient = PredicateEvidence::scoped(&snapshot, forged)?;
    assert!(matches!(planner.prepare_with_evidence(&insufficient, &request, &context(&snapshot)), Err(error) if error.code == ErrorCode::PreconditionsFailed));
    assert!(matches!(deferred_step_decision_with_evidence(&work, &insufficient, |_| Some(CommitState::Verified))?, DeferredStepDecision::Failed(_)));

    let mut stale_context = context(&snapshot);
    stale_context.anchor.cursor.sequence += 1;
    assert!(matches!(planner.prepare_with_evidence(&evidence, &request, &stale_context), Err(error) if error.code == ErrorCode::StaleAnchor));
    let mut stale_policy = policy;
    let mut stale_anchor = snapshot.anchor();
    stale_anchor.cursor.epoch += 1;
    stale_policy.anchor = Some(stale_anchor);
    assert!(matches!(PredicateEvidence::scoped(&snapshot, stale_policy), Err(error) if error.code == ErrorCode::StaleAnchor));
    Ok(())
}

#[test]
fn incomplete_absence_cannot_authorize_planning_dispatch_or_obligation_success() -> Result<()> {
    let partial_snapshot = laboratory_snapshot(10, 0, true);
    let planner = StaticPlanner::default();
    let request = intent(&partial_snapshot, vec![missing()]);
    let work = step(vec![missing()]);
    let mut policy = EvidencePolicy::at(partial_snapshot.anchor());
    policy.all_entities = EvidenceCoverage::Observed;
    let partial = PredicateEvidence::scoped(&partial_snapshot, policy.clone())?;
    assert!(matches!(planner.prepare_with_evidence(&partial, &request, &context(&partial_snapshot)), Err(error) if error.code == ErrorCode::PreconditionsFailed));
    assert!(matches!(deferred_step_decision_with_evidence(&work, &partial, |_| Some(CommitState::Verified))?, DeferredStepDecision::Failed(_)));

    let action = ActionId::new(1);
    let mut runtime = ObligationRuntime::new();
    runtime.register_obligation(action, obligation(missing(), None, 1), GameTick(0))?;
    runtime.step_tick_with_evidence(&partial)?;
    assert_streak(runtime.get_status(action), 0);

    policy.all_entities = EvidenceCoverage::Complete;
    let complete = PredicateEvidence::scoped(&partial_snapshot, policy)?;
    assert!(planner.prepare_with_evidence(&complete, &request, &context(&partial_snapshot))?.digest_is_valid());
    assert_eq!(deferred_step_decision_with_evidence(&work, &complete, |_| Some(CommitState::Verified))?, DeferredStepDecision::Ready);

    let next = laboratory_snapshot(20, 0, true);
    let mut policy = EvidencePolicy::at(next.anchor());
    policy.all_entities = EvidenceCoverage::Complete;
    runtime.step_tick_with_evidence(&PredicateEvidence::scoped(&next, policy)?)?;
    assert!(matches!(runtime.get_status(action), Some(ObligationStatus::Fulfilled { fulfilled_at_tick: GameTick(20), .. })));
    Ok(())
}

#[test]
fn unknown_negation_is_neither_failure_evidence_nor_terminal_proof() -> Result<()> {
    for value in [true, false] {
        let terminal = Predicate::Not(Box::new(ready(false)));
        let failure = Predicate::Not(Box::new(ready(true)));
        let spec = obligation(terminal, Some(failure.clone()), 1);
        let mut runtime = ObligationRuntime::new();
        let action = ActionId::new(1);
        runtime.register_obligation(action, spec.clone(), GameTick(0))?;
        let asserted = snapshot(10, 0, value, FactSource::AgentAssertion("current worker state".to_owned()), Digest32::ZERO);
        runtime.step_tick(&asserted)?;
        assert_streak(runtime.get_status(action), 0);

        let mut work = step(vec![Predicate::True]);
        work.obligation = Some(spec);
        assert_eq!(deferred_step_decision(&work, &asserted, |_| None), DeferredStepDecision::Waiting);
        let untrusted = PredicateEvidence::untrusted(&asserted)?;
        assert_eq!(deferred_step_decision_with_evidence(&work, &untrusted, |_| None)?, DeferredStepDecision::Waiting);

        let observed = laboratory_snapshot(20, 0, value);
        let evidence = PredicateEvidence::laboratory(&observed)?;
        runtime.step_tick_with_evidence(&evidence)?;
        if value {
            assert!(matches!(runtime.get_status(action), Some(ObligationStatus::Fulfilled { .. })));
        } else {
            assert!(matches!(runtime.get_status(action), Some(ObligationStatus::Failed { reason, .. }) if reason.contains("failure predicate")));
            assert!(matches!(deferred_step_decision_with_evidence(&work, &evidence, |_| None)?, DeferredStepDecision::Failed(reason) if reason.contains("failure predicate")));
        }
    }
    Ok(())
}

#[test]
fn loss_of_evidence_resets_stability_even_between_eligible_samples() -> Result<()> {
    let action = ActionId::new(1);
    let mut runtime = ObligationRuntime::new();
    runtime.register_obligation(action, obligation(ready(true), None, 2), GameTick(0))?;
    let first = laboratory_snapshot(10, 0, true);
    runtime.step_tick_with_evidence(&PredicateEvidence::laboratory(&first)?)?;
    assert_streak(runtime.get_status(action), 1);
    // Matching bytes without supplied source authority interrupt the proof.
    let unqualified = laboratory_snapshot(11, 0, true);
    runtime.step_tick(&unqualified)?;
    assert_streak(runtime.get_status(action), 0);
    let next = laboratory_snapshot(20, 0, true);
    runtime.step_tick_with_evidence(&PredicateEvidence::laboratory(&next)?)?;
    assert_streak(runtime.get_status(action), 1);
    let last = laboratory_snapshot(30, 0, true);
    runtime.step_tick_with_evidence(&PredicateEvidence::laboratory(&last)?)?;
    assert!(matches!(runtime.get_status(action), Some(ObligationStatus::Fulfilled { fulfilled_at_tick: GameTick(30), .. })));
    Ok(())
}

#[test]
fn recovered_defaults_need_explicit_evidence_before_counting_completion() -> Result<()> {
    let frontier = laboratory_snapshot(20, 2, true);
    let mut monitor = RecoveredObligation::new(
        ActionId::new(1), obligation(ready(true), None, 2), GameTick(0), &frontier,
    )?;
    assert_streak(monitor.status(), 0);
    let raw_sample = laboratory_snapshot(30, 2, true);
    monitor.observe(&raw_sample)?;
    assert_streak(monitor.status(), 0);
    let first = laboratory_snapshot(40, 2, true);
    monitor.observe_with_evidence(&PredicateEvidence::laboratory(&first)?)?;
    assert_streak(monitor.status(), 1);
    monitor.observe_with_evidence(&PredicateEvidence::laboratory(&first)?)?;
    assert_streak(monitor.status(), 1);
    let last = laboratory_snapshot(50, 2, true);
    monitor.observe_with_evidence(&PredicateEvidence::laboratory(&last)?)?;
    assert!(matches!(monitor.status(), Some(ObligationStatus::Fulfilled { fulfilled_at_tick: GameTick(50), .. })));
    assert_eq!(monitor.last_observation_anchor(), Some(last.anchor()));
    Ok(())
}

#[test]
fn recovered_explicit_evidence_restarts_cadence_without_inheriting_trust_or_streak() -> Result<()> {
    let frontier = laboratory_snapshot(20, 2, true);
    let mut monitor = RecoveredObligation::new_with_evidence(
        ActionId::new(1), obligation(ready(true), None, 2), GameTick(0), &PredicateEvidence::laboratory(&frontier)?,
    )?;
    let first = laboratory_snapshot(30, 2, true);
    monitor.observe_with_evidence(&PredicateEvidence::laboratory(&first)?)?;
    assert_streak(monitor.status(), 1);
    let restarted = laboratory_snapshot(35, 3, true);
    let mut conservative = monitor.after_restart(&restarted)?;
    let mut explicit = monitor.after_restart_with_evidence(&PredicateEvidence::laboratory(&restarted)?)?;
    assert_streak(conservative.status(), 0);
    assert_streak(explicit.status(), 0);
    let early = laboratory_snapshot(44, 3, true);
    explicit.observe_with_evidence(&PredicateEvidence::laboratory(&early)?)?;
    assert_streak(explicit.status(), 0);
    let eligible = laboratory_snapshot(45, 3, true);
    conservative.observe(&eligible)?;
    assert_streak(conservative.status(), 0);
    explicit.observe_with_evidence(&PredicateEvidence::laboratory(&eligible)?)?;
    assert_streak(explicit.status(), 1);
    let terminal = laboratory_snapshot(55, 3, true);
    explicit.observe_with_evidence(&PredicateEvidence::laboratory(&terminal)?)?;
    assert!(matches!(explicit.status(), Some(ObligationStatus::Fulfilled { fulfilled_at_tick: GameTick(55), .. })));
    let proof = explicit.last_observation_anchor();
    let another_restart = laboratory_snapshot(60, 4, false);
    let retained = explicit.after_restart(&another_restart)?;
    assert_eq!(retained.status(), explicit.status());
    assert_eq!(retained.last_observation_anchor(), proof);
    Ok(())
}
