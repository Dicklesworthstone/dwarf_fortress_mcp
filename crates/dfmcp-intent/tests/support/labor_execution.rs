use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, DfmcpError, Digest32, EntityId, ErrorCode,
    FortressId, GameTick, ObservationCursor, OperationContext, RequestId, Result, RiskTier,
    SessionId, WorkBudget,
};
use dfmcp_intent::{DwarfLaborProfile, LaborDemand};
use dfmcp_world::{EntityKind, EntityRecord, Fact, FactSource, Value, WorldGraph, WorldSnapshot};

pub fn profiles() -> Vec<DwarfLaborProfile> {
    vec![
        DwarfLaborProfile {
            id: EntityId::new(1),
            skills: BTreeMap::from([("MINING".to_owned(), 20), ("BREW".to_owned(), 10)]),
            stress_level: 100,
            assigned_labors: BTreeSet::from(["BREW".to_owned()]),
        },
        DwarfLaborProfile {
            id: EntityId::new(2),
            skills: BTreeMap::from([("BREW".to_owned(), 9)]),
            stress_level: 100,
            assigned_labors: BTreeSet::new(),
        },
    ]
}
pub fn demands() -> Vec<LaborDemand> {
    [("MINING", 0), ("BREW", 1)].into_iter().map(|(name, priority)| LaborDemand {
        labor: name.to_owned(), skill: name.to_owned(), workers: 1,
        minimum_skill: 1, priority,
    }).collect()
}
pub fn fact(value: bool, tick: GameTick) -> Fact {
    Fact::known(Value::Bool(value), tick, FactSource::Derived("dfmcp.lab-scenario/1".to_owned()), Digest32::ZERO)
}
pub fn world(rows: &[DwarfLaborProfile], roles: &[LaborDemand]) -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    for row in rows {
        let mut fields = BTreeMap::from([("alive".to_owned(), fact(true, GameTick(10)))]);
        for role in roles {
            fields.insert(format!("labor.{}", role.labor), fact(row.assigned_labors.contains(&role.labor), GameTick(10)));
        }
        for labor in &row.assigned_labors {
            fields.insert(format!("labor.{labor}"), fact(true, GameTick(10)));
        }
        graph.entities.insert(row.id, EntityRecord {
            id: row.id, generation: 1, revision: 1, kind: EntityKind::Unit,
            label: row.id.to_string(), fields,
        });
    }
    WorldSnapshot::new(FortressId::new(7), GameTick(10), ObservationCursor::ORIGIN, true, graph)
}
pub fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1), request_id: RequestId::new(1), anchor: snapshot.anchor(),
        budget: WorkBudget::default(),
        grants: [Capability::Observe, Capability::Plan, Capability::ConfigureLabor].into_iter().map(|capability| CapabilityGrant {
            capability, scope: CapabilityScope { fortress_id: Some(snapshot.fortress_id), ..CapabilityScope::default() },
            max_risk: RiskTier::Reversible, expires_at_tick: None, remaining_uses: None,
        }).collect(),
        cancellation_requested: false,
    }
}
pub fn set(snapshot: &mut WorldSnapshot, id: EntityId, name: &str, value: bool) -> Result<()> {
    let row = snapshot.graph.entities.get_mut(&id).ok_or_else(|| DfmcpError::new(ErrorCode::InternalInvariantViolation, "fixture unit missing"))?;
    row.fields.insert(name.to_owned(), fact(value, snapshot.tick));
    row.revision += 1;
    snapshot.cursor = snapshot.cursor.checked_next().ok_or_else(|| DfmcpError::new(ErrorCode::BudgetExceeded, "fixture cursor overflow"))?;
    snapshot.refresh_hash();
    Ok(())
}
