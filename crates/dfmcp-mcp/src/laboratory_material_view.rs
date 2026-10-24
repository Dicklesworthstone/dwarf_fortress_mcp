//! Source-qualified construction allocations in the opt-in material economy.
//! Available stock, held construction material and historical consumption
//! remain separate; rendering this report grants no construction authority.

use dfmcp_core::{EntityId, Result};
use dfmcp_intent::effects;
use dfmcp_world::WorldSnapshot;
use serde_json::{Value as Json, json};

pub(crate) fn construction_material_json(snapshot: &WorldSnapshot) -> Result<Option<Json>> {
    let Some(report) = effects::construction_material_report(snapshot)? else {
        return Ok(None);
    };
    Ok(Some(json!({
        "model": report.model,
        "available": true,
        "epistemic_state": "certified_derived",
        "evidence_scope": "laboratory_reference_world",
        "source_anchor": crate::observation_projection::anchor_json(&snapshot.anchor()),
        "material": "WOOD",
        "ledger_entity_id": report.ledger.to_string(),
        "ledger_generation": report.ledger_generation,
        "available_wood": report.available_wood,
        "held_wood": report.held_wood,
        "consumed_wood": report.consumed_wood,
        "released_wood": report.released_wood,
        "allocations": report.allocations.iter()
            .take(super::MAX_BRIEFING_ITEMS)
            .map(allocation_json)
            .collect::<Vec<_>>(),
        "allocations_omitted": report.allocations.len().saturating_sub(super::MAX_BRIEFING_ITEMS),
        "material_rule": "one WOOD unit per new Still, Kitchen or Carpenters workshop; held at dispatch, consumed on completion, released once if unfinished construction is cancelled",
    })))
}

/// Filter the full qualified census before rendering one action's allocation.
pub(crate) fn construction_material_action_json(
    snapshot: &WorldSnapshot,
    building: EntityId,
) -> Result<Option<Json>> {
    let Some(report) = effects::construction_material_report(snapshot)? else {
        return Ok(None);
    };
    let Some(allocation) = report
        .allocations
        .iter()
        .find(|allocation| allocation.building == building)
    else {
        return Ok(None);
    };
    Ok(Some(json!({
        "model": report.model,
        "epistemic_state": "certified_derived",
        "evidence_scope": "laboratory_reference_world",
        "source_anchor": crate::observation_projection::anchor_json(&snapshot.anchor()),
        "ledger_entity_id": report.ledger.to_string(),
        "ledger_generation": report.ledger_generation,
        "allocation": allocation_json(allocation),
    })))
}

fn allocation_json(allocation: &effects::ConstructionMaterialAllocation) -> Json {
    json!({
        "building_entity_id": allocation.building.to_string(),
        "building_kind": allocation.building_kind,
        "material": "WOOD",
        "wood_units": allocation.wood_units,
        "state": allocation.state,
    })
}
