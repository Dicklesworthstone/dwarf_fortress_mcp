#![forbid(unsafe_code)]

use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, ErrorCode, FortressId, GameTick, IntentId,
    MapCoord, MapCuboid, ObservationCursor, OperationContext, RequestId, RiskTier, SessionId,
};
use dfmcp_intent::{Action, Constraint, Intent, RequestedAction, StaticPlanner};
use dfmcp_world::{Predicate, WorldGraph, WorldSnapshot};
use std::error::Error;

fn make_snapshot() -> WorldSnapshot {
    WorldSnapshot::new(
        FortressId::new(1),
        GameTick(100),
        ObservationCursor::ORIGIN,
        true,
        WorldGraph::default(),
    )
}

fn make_context(snapshot: &WorldSnapshot, grants: Vec<CapabilityGrant>) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: dfmcp_core::WorkBudget::CONSERVATIVE_DEFAULT,
        grants,
        cancellation_requested: false,
    }
}

#[test]
fn test_static_planner_risk_and_capability_authorization() -> Result<(), Box<dyn Error>> {
    let snapshot = make_snapshot();

    // Intent requiring ControlClock (Reversible)
    let intent = Intent {
        id: IntentId::new(10),
        anchor: snapshot.anchor(),
        summary: "unpause simulation".to_string(),
        terminal_condition: Predicate::Paused(false),
        constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions: vec![RequestedAction {
            action: Action::Pause { paused: false },
            preconditions: vec![Predicate::Paused(true)],
            postconditions: Vec::new(),
            compensation: None,
            obligation: None,
            depends_on: Vec::new(),
        }],
    };

    let planner = StaticPlanner::default();

    let plan_grant = CapabilityGrant {
        capability: Capability::Plan,
        scope: CapabilityScope::default(),
        max_risk: RiskTier::ReadOnly,
        expires_at_tick: None,
        remaining_uses: None,
    };

    // 1. Context with no grants -> CapabilityDenied (missing Plan)
    let ctx_no_grants = make_context(&snapshot, Vec::new());
    let res = planner.prepare(&snapshot, &intent, &ctx_no_grants);
    let Err(err) = res else {
        return Err("expected CapabilityDenied error".into());
    };
    assert_eq!(err.code, ErrorCode::CapabilityDenied);

    // 3. Intent with MaxRisk(ReadOnly) constraint -> RiskCeilingExceeded
    let mut intent_restricted = intent.clone();
    intent_restricted.constraints = vec![Constraint::MaxRisk(RiskTier::ReadOnly)];
    let valid_clock_grant = CapabilityGrant {
        capability: Capability::ControlClock,
        scope: CapabilityScope::default(),
        max_risk: RiskTier::Reversible,
        expires_at_tick: None,
        remaining_uses: None,
    };
    let ctx_valid = make_context(
        &snapshot,
        vec![plan_grant.clone(), valid_clock_grant.clone()],
    );
    let res = planner.prepare(&snapshot, &intent_restricted, &ctx_valid);
    let Err(err) = res else {
        return Err("expected RiskCeilingExceeded error".into());
    };
    assert_eq!(err.code, ErrorCode::RiskCeilingExceeded);

    // 4. Authorized context & matching intent -> Success
    let plan = planner.prepare(&snapshot, &intent, &ctx_valid)?;
    assert_eq!(plan.max_risk, RiskTier::Reversible);
    assert!(
        plan.required_capabilities
            .contains(&Capability::ControlClock)
    );
    assert!(plan.digest_is_valid());
    Ok(())
}

#[test]
fn test_risk_monotonicity_multi_step() -> Result<(), Box<dyn Error>> {
    let min = MapCoord { x: 0, y: 0, z: 10 };
    let max = MapCoord { x: 1, y: 1, z: 10 };
    let area = MapCuboid::new(min, max)?;

    let step_reversible = Action::Pause { paused: true };
    let step_guarded = Action::Build {
        kind: dfmcp_intent::BuildingKind::FarmPlot,
        location: min,
        footprint: area,
        material: dfmcp_intent::MaterialSelector::default(),
    };

    assert_eq!(step_reversible.risk(), RiskTier::Reversible);
    assert_eq!(step_guarded.risk(), RiskTier::Guarded);

    // Supremum of Reversible and Guarded is Guarded
    let max_risk = step_reversible.risk().max(step_guarded.risk());
    assert_eq!(max_risk, RiskTier::Guarded);
    Ok(())
}
