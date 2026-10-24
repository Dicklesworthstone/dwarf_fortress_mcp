use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, Digest32, EntityId, FortressId, GameTick,
    IntentId, MapCoord, MapCuboid, ObservationCursor, OperationContext, RequestId, RiskTier,
    SessionId, WorkBudget,
};
use dfmcp_world::{ChunkCoord, EntityRecord, Fact, FactSource, WorldGraph};

use super::*;
use crate::{BuildingKind, Constraint, Intent, MaterialSelector, StaticPlanner};

const LEDGER: EntityId = EntityId::new(91);

fn put(snapshot: &mut WorldSnapshot, id: EntityId, kind: EntityKind, fields: Vec<(&str, Value)>) {
    snapshot.graph.entities.insert(
        id,
        EntityRecord {
            id,
            generation: 1,
            revision: 1,
            kind,
            label: "material planning fixture".to_owned(),
            fields: fields
                .into_iter()
                .map(|(key, value)| {
                    (
                        key.to_owned(),
                        Fact::known(
                            value,
                            snapshot.tick,
                            FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                            Digest32::ZERO,
                        ),
                    )
                })
                .collect(),
        },
    );
    snapshot.refresh_hash();
}

fn source(logs: u64, materials: bool) -> WorldSnapshot {
    let mut source = WorldSnapshot::new(
        FortressId::new(706),
        GameTick(1_000),
        ObservationCursor::ORIGIN,
        true,
        WorldGraph::default(),
    );
    let chunk = ChunkCoord { x: 0, y: 0, z: 0 };
    source.graph.chunks.insert(
        chunk,
        dfmcp_world::terrain::uniform_chunk(chunk, dfmcp_world::tile_codes::FLOOR),
    );
    let mut markers = vec![(
        effects::LABORATORY_WORKFORCE_MODEL_FIELD,
        Value::Text(effects::LABORATORY_WORKFORCE_MODEL.to_owned()),
    )];
    if materials {
        markers.push((
            effects::LABORATORY_CONSTRUCTION_MATERIAL_MODEL_FIELD,
            Value::Text(effects::LABORATORY_CONSTRUCTION_MATERIAL_MODEL.to_owned()),
        ));
    }
    let settings = effects::fortress_settings_entity_id(source.fortress_id);
    put(
        &mut source,
        settings,
        EntityKind::Other(effects::FORTRESS_SETTINGS_KIND.to_owned()),
        markers,
    );
    put(
        &mut source,
        LEDGER,
        EntityKind::Other(effects::STOCK_LEDGER_KIND.to_owned()),
        vec![
            (effects::STOCK_WOOD_FIELD, Value::U64(logs)),
            (effects::STOCK_BED_FIELD, Value::U64(0)),
            (effects::STOCK_DRINK_FIELD, Value::U64(100)),
            (effects::STOCK_FOOD_FIELD, Value::U64(100)),
            (effects::METABOLISM_TICKS_FIELD, Value::U64(0)),
        ],
    );
    source
}

fn context(source: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: source.anchor(),
        budget: WorkBudget::default(),
        cancellation_requested: false,
        grants: vec![CapabilityGrant {
            capability: Capability::Plan,
            scope: CapabilityScope::default(),
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        }],
    }
}

fn action(action: Action) -> RequestedAction {
    RequestedAction {
        action,
        preconditions: Vec::new(),
        postconditions: Vec::new(),
        obligation: None,
        compensation: None,
        depends_on: Vec::new(),
    }
}

fn build(x: i32) -> RequestedAction {
    let at = MapCoord::new(x, 1, 0);
    action(Action::Build {
        kind: BuildingKind::Workshop("Carpenters".to_owned()),
        location: at,
        footprint: MapCuboid { min: at, max: at },
        material: MaterialSelector::default(),
    })
}

fn furniture(amount: u32) -> RequestedAction {
    action(Action::CreateWorkOrder {
        name: "beds".to_owned(),
        job_token: "CONSTRUCT_BED".to_owned(),
        amount,
        conditions: Vec::new(),
    })
}

fn intent(source: &WorldSnapshot, actions: Vec<RequestedAction>) -> Intent {
    Intent {
        id: IntentId::new(706),
        anchor: source.anchor(),
        summary: "construct against finite material inventory".to_owned(),
        terminal_condition: Predicate::EntityExists(EntityId::new(987_654)),
        constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
        requested_actions: actions,
    }
}

#[test]
fn aggregate_workshop_and_furniture_costs_refuse_before_any_effect() -> Result<()> {
    for actions in [
        vec![build(1), build(2), build(3)],
        vec![build(1), furniture(2)],
    ] {
        let source = source(2, true);
        let original = source.clone();
        let request = intent(&source, actions);
        let error =
            StaticPlanner::default().prepare_laboratory(&source, &request, &context(&source));
        assert!(
            matches!(error, Err(error) if error.code == ErrorCode::PreconditionsFailed && error.message.contains("3 available WOOD"))
        );
        assert_eq!(source, original);
    }
    let source = source(3, true);
    let request = intent(&source, vec![build(1), furniture(2)]);
    assert!(
        StaticPlanner::default()
            .prepare_laboratory(&source, &request, &context(&source))?
            .digest_is_valid()
    );
    Ok(())
}

#[test]
fn queued_furniture_demand_cannot_be_spent_on_new_workshops() -> Result<()> {
    let mut source = source(2, true);
    effects::apply_effect(&mut source, &furniture(2).action, "existing-carpentry")?;
    source.refresh_hash();
    let request = intent(&source, vec![build(1)]);
    assert!(
        matches!(StaticPlanner::default().prepare_laboratory(&source, &request, &context(&source)), Err(error) if error.code == ErrorCode::PreconditionsFailed && error.message.contains("existing carpentry"))
    );
    Ok(())
}

#[test]
fn material_proof_is_mandatory_and_explicit_deadline_is_preserved() -> Result<()> {
    let source = source(1, true);
    let mut requested = build(1);
    let mut obligation = effects::default_obligation(
        &requested.action,
        "explicit",
        source.fortress_id,
        source.tick,
    )?
    .ok_or_else(bounded)?;
    obligation.deadline_tick = GameTick(1_750);
    obligation.terminal = Predicate::Paused(true);
    requested.obligation = Some(obligation);
    requested.postconditions = vec![Predicate::Paused(true)];
    let request = intent(&source, vec![requested]);
    let plan = StaticPlanner::default().prepare_laboratory(&source, &request, &context(&source))?;
    let step = &plan.steps[0];
    assert!(step.preconditions.iter().any(|p| matches!(p, Predicate::FieldCompare {entity_id,field,op:CompareOp::Ge,value:Value::U64(1)} if *entity_id == LEDGER && field == effects::STOCK_WOOD_FIELD)));
    let required =
        effects::construction_completion_predicates(&source, &step.action, &step.idempotency_key)?;
    assert!(!required.is_empty());
    assert!(
        required
            .iter()
            .all(|predicate| step.postconditions.contains(&predicate.normalized()))
    );
    let obligation = step.obligation.as_ref().ok_or_else(bounded)?;
    assert_eq!(obligation.deadline_tick, GameTick(1_750));
    assert!(!dfmcp_world::evaluate(&source, &obligation.terminal));
    assert!(
        matches!(StaticPlanner::default().prepare(&source, &request, &context(&source)), Err(error) if error.code == ErrorCode::PreconditionsFailed)
    );
    Ok(())
}

#[test]
fn model_absence_preserves_default_build_semantics() -> Result<()> {
    let source = source(0, false);
    let request = intent(&source, vec![build(1)]);
    let plan = StaticPlanner::default().prepare_laboratory(&source, &request, &context(&source))?;
    let step = &plan.steps[0];
    assert_eq!(
        step.postconditions,
        effects::default_postconditions(&step.action, &step.idempotency_key, source.fortress_id)
    );
    assert!(!step.preconditions.iter().any(|p| matches!(p, Predicate::FieldCompare {field,..} if field == effects::LABORATORY_CONSTRUCTION_MATERIAL_MODEL_FIELD)));
    Ok(())
}
