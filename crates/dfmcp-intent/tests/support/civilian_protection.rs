use std::collections::BTreeMap;

use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, Digest32, EntityId, ErrorCode, DfmcpError,
    FortressId, GameTick, ObservationCursor, OperationContext, RequestId, Result, RiskTier,
    SessionId, WorkBudget,
};
use dfmcp_intent::{CivilianProtectionPlan, effects};
use dfmcp_world::{EntityKind, EntityRecord, Fact, FactSource, Value, WorldGraph, WorldSnapshot};

pub const FIRST: EntityId = EntityId::new(11);
pub const SECOND: EntityId = EntityId::new(12);
pub const OTHER: EntityId = EntityId::new(13);
pub const BURROW: EntityId = EntityId::new(20);

pub fn fact(value: Value) -> Fact {
    Fact::known(value, GameTick(1), FactSource::Derived("dfmcp.lab-scenario/1".to_owned()), Digest32::ZERO)
}
pub fn field_name() -> String { format!("burrow.{}", BURROW.get()) }
pub fn record(id: EntityId, kind: EntityKind) -> EntityRecord {
    EntityRecord { id, generation: 1, revision: 1, kind, label: String::new(), fields: BTreeMap::new() }
}
pub fn world() -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    for (id, assigned) in [(FIRST, true), (SECOND, false), (OTHER, true)] {
        let mut unit = record(id, EntityKind::Unit);
        unit.fields.insert("alive".to_owned(), fact(Value::Bool(true)));
        unit.fields.insert(field_name(), fact(Value::Bool(assigned)));
        graph.entities.insert(id, unit);
    }
    graph.entities.insert(BURROW, record(BURROW, EntityKind::Burrow));
    WorldSnapshot::new(FortressId::new(7), GameTick(1), ObservationCursor::ORIGIN, true, graph)
}
pub fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1), request_id: RequestId::new(1), anchor: snapshot.anchor(),
        budget: WorkBudget::default(), cancellation_requested: false,
        grants: [
            (Capability::Observe, RiskTier::ReadOnly),
            (Capability::Plan, RiskTier::ReadOnly),
            (Capability::ConfigureLogistics, RiskTier::Guarded),
            (Capability::Checkpoint, RiskTier::Guarded),
        ].into_iter().map(|(capability, max_risk)| CapabilityGrant {
            capability, max_risk, expires_at_tick: None, remaining_uses: None,
            scope: CapabilityScope { fortress_id: Some(snapshot.fortress_id), ..CapabilityScope::default() },
        }).collect(),
    }
}
pub fn compiled() -> Result<CivilianProtectionPlan> {
    CivilianProtectionPlan::compile(BURROW, &[SECOND, FIRST])
}
pub fn set(snapshot: &mut WorldSnapshot, id: EntityId, key: &str, value: Fact) -> Result<()> {
    snapshot.graph.entities.get_mut(&id)
        .ok_or_else(|| DfmcpError::new(ErrorCode::InvalidRequest, "fixture entity missing"))?
        .fields.insert(key.to_owned(), value);
    snapshot.refresh_hash();
    Ok(())
}
pub fn complete(mut snapshot: WorldSnapshot) -> Result<WorldSnapshot> {
    set(&mut snapshot, SECOND, &field_name(), fact(Value::Bool(true)))?;
    let id = effects::fortress_settings_entity_id(snapshot.fortress_id);
    let mut settings = record(id, EntityKind::Other(effects::FORTRESS_SETTINGS_KIND.to_owned()));
    settings.fields.insert("standing_order.CIVILIAN_BURROW_RESTRICTION".to_owned(), fact(Value::Text("ACTIVE".to_owned())));
    snapshot.graph.entities.insert(id, settings);
    snapshot.refresh_hash();
    Ok(snapshot)
}
