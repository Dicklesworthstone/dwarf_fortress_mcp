//! Synthesized deadlines must retain the complete prerequisite frontier.
//! Immediate actions can connect temporal work without owning an obligation.

use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, DfmcpError, ErrorCode, FortressId, GameTick,
    IntentId, ObservationCursor, OperationContext, RequestId, Result, RiskTier, SessionId,
    WorkBudget,
};
use dfmcp_intent::{
    Action, Constraint, Intent, ObligationSpec, PreparedPlan, RequestedAction, StaticPlanner,
};
use dfmcp_world::{Predicate, WorldGraph, WorldSnapshot};

fn fixture(actions: Vec<RequestedAction>) -> (WorldSnapshot, Intent, OperationContext) {
    let snapshot = WorldSnapshot::new(
        FortressId::new(1),
        GameTick(10),
        ObservationCursor::ORIGIN,
        true,
        WorldGraph::default(),
    );
    let intent = Intent {
        id: IntentId::new(71),
        anchor: snapshot.anchor(),
        summary: "retain complete dependency timing".to_owned(),
        terminal_condition: Predicate::Paused(false),
        constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions: actions,
    };
    let context = OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget::default(),
        grants: vec![CapabilityGrant {
            capability: Capability::Plan,
            scope: CapabilityScope::default(),
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        }],
        cancellation_requested: false,
    };
    (snapshot, intent, context)
}

fn requested(action: Action, depends_on: Vec<u32>) -> RequestedAction {
    RequestedAction {
        action,
        preconditions: Vec::new(),
        postconditions: Vec::new(),
        obligation: None,
        compensation: None,
        depends_on,
    }
}

fn order(amount: u32, depends_on: Vec<u32>) -> RequestedAction {
    requested(
        Action::CreateWorkOrder {
            name: "bounded original work".to_owned(),
            job_token: "SMELT_IRON".to_owned(),
            amount,
            conditions: Vec::new(),
        },
        depends_on,
    )
}

fn immediate(depends_on: Vec<u32>) -> RequestedAction {
    requested(Action::Pause { paused: true }, depends_on)
}

fn prepare(actions: Vec<RequestedAction>) -> Result<PreparedPlan> {
    let (snapshot, intent, context) = fixture(actions);
    StaticPlanner::default().prepare_laboratory(&snapshot, &intent, &context)
}

fn deadline(plan: &PreparedPlan, step: usize) -> Result<GameTick> {
    plan.steps[step]
        .obligation
        .as_ref()
        .map(|obligation| obligation.deadline_tick)
        .ok_or_else(|| DfmcpError::new(ErrorCode::InvalidPlan, "test needs a temporal step"))
}

#[test]
fn immediate_chains_preserve_temporal_ancestor_deadlines() -> Result<()> {
    let plan = prepare(vec![
        order(100, Vec::new()),
        immediate(vec![0]),
        immediate(vec![1]),
        order(1, vec![2]),
    ])?;
    assert_eq!(deadline(&plan, 0)?, GameTick(10_110));
    assert!(plan.steps[1].obligation.is_none());
    assert!(plan.steps[2].obligation.is_none());
    assert_eq!(deadline(&plan, 3)?, GameTick(10_310));
    assert!(plan.digest_is_valid());
    Ok(())
}

#[test]
fn joined_branches_use_the_latest_transitive_frontier_only() -> Result<()> {
    let plan = prepare(vec![
        order(100, Vec::new()),
        order(1, Vec::new()),
        immediate(vec![0]),
        immediate(vec![1]),
        immediate(vec![2, 3]),
        order(1, vec![4]),
        order(1, vec![3]),
        order(1, Vec::new()),
    ])?;
    assert_eq!(deadline(&plan, 5)?, GameTick(10_310));
    assert_eq!(deadline(&plan, 6)?, GameTick(410));
    assert_eq!(deadline(&plan, 7)?, GameTick(210));
    Ok(())
}

#[test]
fn explicit_child_horizons_are_never_extended_by_dependency_inference() -> Result<()> {
    let mut child = order(1, vec![1]);
    child.obligation = Some(ObligationSpec {
        terminal: Predicate::Paused(false),
        failure: None,
        deadline_tick: GameTick(500),
        poll_interval_ticks: 5,
        stable_for_observations: 2,
    });
    let plan = prepare(vec![order(100, Vec::new()), immediate(vec![0]), child])?;
    let obligation = plan.steps[2]
        .obligation
        .as_ref()
        .ok_or_else(|| DfmcpError::new(ErrorCode::InvalidPlan, "explicit obligation missing"))?;
    assert_eq!(obligation.deadline_tick, GameTick(500));
    assert_eq!(obligation.poll_interval_ticks, 5);
    assert_eq!(obligation.stable_for_observations, 2);
    Ok(())
}

#[test]
fn immediate_obligations_and_their_ancestors_both_contribute_to_descendants() -> Result<()> {
    let mut shorter = immediate(vec![0]);
    shorter.obligation = Some(ObligationSpec {
        terminal: Predicate::Paused(false),
        failure: None,
        deadline_tick: GameTick(500),
        poll_interval_ticks: 1,
        stable_for_observations: 1,
    });
    let mut longer = immediate(vec![0]);
    longer.obligation = Some(ObligationSpec {
        terminal: Predicate::Paused(false),
        failure: None,
        deadline_tick: GameTick(12_000),
        poll_interval_ticks: 1,
        stable_for_observations: 1,
    });
    let plan = prepare(vec![
        order(100, Vec::new()),
        shorter,
        longer,
        order(1, vec![1]),
        order(1, vec![2]),
    ])?;
    assert_eq!(deadline(&plan, 1)?, GameTick(500));
    assert_eq!(deadline(&plan, 2)?, GameTick(12_000));
    assert_eq!(deadline(&plan, 3)?, GameTick(10_310));
    assert_eq!(deadline(&plan, 4)?, GameTick(12_200));
    Ok(())
}

#[test]
fn temporal_only_and_immediate_only_plans_keep_their_existing_seals() -> Result<()> {
    for actions in [
        vec![order(2, Vec::new()), order(1, vec![0])],
        vec![immediate(Vec::new()), immediate(vec![0])],
    ] {
        let (snapshot, intent, context) = fixture(actions);
        let planner = StaticPlanner::default();
        assert_eq!(
            planner.prepare_laboratory(&snapshot, &intent, &context)?,
            planner.prepare_laboratory_legacy_deadlines(&snapshot, &intent, &context)?
        );
    }
    Ok(())
}

#[test]
fn transitive_frontier_overflow_refuses_without_shortening_the_deadline() {
    let mut first = order(1, Vec::new());
    first.obligation = Some(ObligationSpec {
        terminal: Predicate::Paused(false),
        failure: None,
        deadline_tick: GameTick(u64::MAX - 100),
        poll_interval_ticks: 1,
        stable_for_observations: 1,
    });
    assert_eq!(
        prepare(vec![first, immediate(vec![0]), order(1, vec![1])])
            .err()
            .map(|error| error.code),
        Some(ErrorCode::BudgetExceeded)
    );
}
