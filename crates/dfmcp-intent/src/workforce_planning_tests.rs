use std::collections::BTreeMap;

use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, Digest32, EntityId, FortressId, IntentId,
    MapCoord, MapCuboid, ObservationCursor, OperationContext, RequestId, RiskTier, SessionId,
    WorkBudget,
};
use dfmcp_world::{ChunkCoord, EntityKind, EntityRecord, Fact, FactSource, WorldGraph};

use super::*;
use crate::{BuildingKind, Constraint, Intent, MaterialSelector, StaticPlanner};

fn source(shared: bool) -> WorldSnapshot {
    let mut snapshot = WorldSnapshot::new(
        FortressId::new(703),
        GameTick(1_000),
        ObservationCursor::ORIGIN,
        true,
        WorldGraph::default(),
    );
    let chunk = ChunkCoord { x: 0, y: 0, z: 0 };
    snapshot.graph.chunks.insert(
        chunk,
        dfmcp_world::terrain::uniform_chunk(chunk, dfmcp_world::tile_codes::FLOOR),
    );
    if shared {
        let id = effects::fortress_settings_entity_id(snapshot.fortress_id);
        snapshot.graph.entities.insert(
            id,
            EntityRecord {
                id,
                generation: 1,
                revision: 1,
                kind: EntityKind::Other(effects::FORTRESS_SETTINGS_KIND.to_owned()),
                label: "shared workforce fixture".to_owned(),
                fields: BTreeMap::from([(
                    effects::LABORATORY_WORKFORCE_MODEL_FIELD.to_owned(),
                    Fact::known(
                        Value::Text(effects::LABORATORY_WORKFORCE_MODEL.to_owned()),
                        snapshot.tick,
                        FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                        Digest32::ZERO,
                    ),
                )]),
            },
        );
    }
    snapshot.refresh_hash();
    snapshot
}

fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
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
    }
}

fn construction(x: i32) -> RequestedAction {
    let location = MapCoord { x, y: 0, z: 0 };
    RequestedAction {
        action: Action::Build {
            kind: BuildingKind::Workshop("Still".to_owned()),
            location,
            footprint: MapCuboid {
                min: location,
                max: location,
            },
            material: MaterialSelector::default(),
        },
        preconditions: Vec::new(),
        postconditions: Vec::new(),
        compensation: None,
        obligation: None,
        depends_on: Vec::new(),
    }
}

fn intent(snapshot: &WorldSnapshot, actions: Vec<RequestedAction>) -> Intent {
    Intent {
        id: IntentId::new(703),
        anchor: snapshot.anchor(),
        summary: "construct through the shared laboratory workforce".to_owned(),
        terminal_condition: Predicate::EntityExists(EntityId::new(987_654)),
        constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
        requested_actions: actions,
    }
}

#[test]
fn queued_construction_and_peer_work_expand_synthesized_deadlines() -> Result<()> {
    let mut source = source(true);
    effects::apply_effect(&mut source, &construction(10).action, "queued-construction")?;
    source.refresh_hash();
    let original = source.clone();
    let workload = LaboratoryWorkforcePlanning::capture(&source)?.ok_or_else(horizon_error)?;
    assert_eq!(workload.queued_activities, 1);
    assert_eq!(workload.queued_service_ticks, effects::BUILD_TICKS);
    let intent = intent(&source, (0..6).map(construction).collect());
    let floor = workload.deadline_floor(&intent.requested_actions)?;
    // Seven activities require 3,500 worker-ticks even if there is just one
    // constructor; the old 2,000-tick per-building deadline is insufficient.
    assert!(floor.0 - source.tick.0 >= 7_000);
    let plan = StaticPlanner::default().prepare_laboratory(&source, &intent, &context(&source))?;
    assert!(plan_uses_shared_workforce(&plan));
    assert!(plan.steps.iter().all(|step| {
        step.obligation
            .as_ref()
            .is_some_and(|obligation| obligation.deadline_tick >= floor)
    }));
    assert!(
        plan.steps
            .iter()
            .all(|step| step.preconditions.contains(&workload.model_precondition()))
    );
    assert!(plan.digest_is_valid());
    assert_eq!(source, original);
    Ok(())
}

#[test]
fn explicit_deadlines_remain_exact_and_dependencies_remain_ordered() -> Result<()> {
    let source = source(true);
    let mut actions: Vec<_> = (0..6).map(construction).collect();
    let mut explicit = effects::default_obligation(
        &actions[0].action,
        "caller-supplied",
        source.fortress_id,
        source.tick,
    )?
    .ok_or_else(horizon_error)?;
    explicit.deadline_tick = GameTick(1_100);
    actions[0].obligation = Some(explicit.clone());
    actions[5].depends_on = vec![4];
    let intent = intent(&source, actions);
    let plan = StaticPlanner::default().prepare_laboratory(&source, &intent, &context(&source))?;
    assert_eq!(plan.steps[0].obligation, Some(explicit));
    let before = plan.steps[4]
        .obligation
        .as_ref()
        .ok_or_else(horizon_error)?;
    let after = plan.steps[5]
        .obligation
        .as_ref()
        .ok_or_else(horizon_error)?;
    assert!(after.deadline_tick > before.deadline_tick);
    Ok(())
}

#[test]
fn absent_model_preserves_original_deadlines_and_preconditions() -> Result<()> {
    let source = source(false);
    assert!(LaboratoryWorkforcePlanning::capture(&source)?.is_none());
    let intent = intent(&source, (0..6).map(construction).collect());
    let plan = StaticPlanner::default().prepare_laboratory(&source, &intent, &context(&source))?;
    assert!(!plan_uses_shared_workforce(&plan));
    for step in &plan.steps {
        assert!(step.preconditions.is_empty());
        assert_eq!(
            step.obligation,
            effects::default_obligation(
                &step.action,
                &step.idempotency_key,
                source.fortress_id,
                source.tick,
            )?
        );
    }
    Ok(())
}

#[test]
fn claimed_model_cannot_grant_untrusted_planning_authority() {
    let source = source(true);
    let intent = intent(&source, vec![construction(0)]);
    assert!(matches!(
        StaticPlanner::default().prepare(&source, &intent, &context(&source)),
        Err(error) if error.code == ErrorCode::PreconditionsFailed
    ));
}

#[test]
fn service_and_action_bounds_refuse_instead_of_saturating() -> Result<()> {
    let source = source(true);
    let workload = LaboratoryWorkforcePlanning::capture(&source)?.ok_or_else(horizon_error)?;
    assert!(
        matches!(workload.deadline_floor(&(0..65).map(construction).collect::<Vec<_>>()),
        Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    let mut request = construction(0);
    request.action = Action::CreateWorkOrder {
        name: "oversized".to_owned(),
        job_token: "BREW_DRINK_FROM_PLANT".to_owned(),
        amount: u32::MAX,
        conditions: Vec::new(),
    };
    assert!(matches!(workload.deadline_floor(&[request]),
        Err(error) if error.code == ErrorCode::BudgetExceeded));
    let overflow = LaboratoryWorkforcePlanning {
        queued_service_ticks: u64::MAX,
        ..workload
    };
    assert!(matches!(overflow.deadline_floor(&[construction(0)]),
        Err(error) if error.code == ErrorCode::BudgetExceeded));
    Ok(())
}
