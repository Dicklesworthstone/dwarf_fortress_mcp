//! Explicit finite resource sources and bounded harvesting projections.
//! Supplies enter the stock ledger only after observed completed harvest work.

use super::{lab_fact, provisions_starter_graph, record, starter};
use dfmcp_core::{EntityId, FortressId, Result};
use dfmcp_intent::{HarvestResource, effects};
use dfmcp_world::{EntityKind, Value, WorldGraph, WorldSnapshot};
use serde_json::{Value as Json, json};

pub(super) const WOOD_SOURCE: EntityId = EntityId::new(8_001);
pub(super) const PLANT_SOURCE: EntityId = EntityId::new(8_002);

/// A separate scenario preserves every previously sealed scenario generation.
pub(super) fn starter_graph(fortress: FortressId) -> Result<WorldGraph> {
    let mut graph = provisions_starter_graph(fortress)?;
    let settings = graph
        .entities
        .get_mut(&effects::fortress_settings_entity_id(fortress))
        .ok_or_else(|| super::invalid("starter harvest settings are missing"))?;
    settings.fields.insert(
        effects::LABORATORY_HARVEST_MODEL_FIELD.to_owned(),
        lab_fact(Value::Text(effects::LABORATORY_HARVEST_MODEL.to_owned())),
    );
    for index in 0..7 {
        let unit = graph
            .entities
            .get_mut(&EntityId::new(starter::FIRST_DWARF + index))
            .ok_or_else(|| super::invalid("starter harvest worker is missing"))?;
        for (resource, assigned) in [(HarvestResource::Wood, 3), (HarvestResource::Plant, 4)] {
            unit.fields.insert(
                format!("{}{}", effects::LABOR_FIELD_PREFIX, resource.labor()),
                lab_fact(Value::Bool(index == assigned)),
            );
        }
    }
    for (source, resource, units, label) in [
        (WOOD_SOURCE, HarvestResource::Wood, 32, "Finite wood stand"),
        (
            PLANT_SOURCE,
            HarvestResource::Plant,
            40,
            "Finite plant patch",
        ),
    ] {
        graph.entities.insert(
            source,
            record(
                source,
                EntityKind::Plant,
                label,
                vec![
                    (
                        effects::HARVEST_SOURCE_MODEL_FIELD,
                        Value::Text(effects::LABORATORY_HARVEST_MODEL.to_owned()),
                    ),
                    (
                        effects::HARVEST_RESOURCE_FIELD,
                        Value::Text(resource.as_str().to_owned()),
                    ),
                    (effects::HARVEST_INITIAL_FIELD, Value::U64(units)),
                    (effects::HARVEST_REMAINING_FIELD, Value::U64(units)),
                    (effects::HARVEST_YIELDED_FIELD, Value::U64(0)),
                ],
            ),
        );
    }
    Ok(graph)
}

pub(crate) fn harvest_json(snapshot: &WorldSnapshot) -> Result<Option<Json>> {
    let Some(report) = effects::harvest_report(snapshot)? else {
        return Ok(None);
    };
    Ok(Some(json!({
        "model": report.model,
        "available": true,
        "epistemic_state": "certified_derived",
        "evidence_scope": "laboratory_reference_world",
        "source_anchor": crate::observation_projection::anchor_json(&snapshot.anchor()),
        "ledger_entity_id": report.ledger.to_string(),
        "ledger_generation": report.ledger_generation,
        "sources": report.sources.iter().take(super::MAX_BRIEFING_ITEMS)
            .map(source_json).collect::<Vec<_>>(),
        "sources_total": report.sources.len(),
        "sources_omitted": report.sources.len().saturating_sub(super::MAX_BRIEFING_ITEMS),
        "tasks": report.tasks.iter().take(super::MAX_BRIEFING_ITEMS)
            .map(task_json).collect::<Vec<_>>(),
        "tasks_total": report.tasks.len(),
        "tasks_omitted": report.tasks.len().saturating_sub(super::MAX_BRIEFING_ITEMS),
        "source_units_reserved": false,
        "credit_on_completion_only": true,
        "input_rule": "one completed harvest unit depletes one original source unit and credits one WOOD or PLANT; partial work creates no supply and cancellation preserves completed yield",
    })))
}

/// Select from the complete qualified census before limiting display rows.
pub(crate) fn harvest_action_json(
    snapshot: &WorldSnapshot,
    entity: EntityId,
) -> Result<Option<Json>> {
    if !snapshot.graph.entities.get(&entity).is_some_and(|record| {
        matches!(&record.kind, EntityKind::Other(kind) if kind == effects::HARVEST_TASK_KIND)
    }) {
        return Ok(None);
    }
    let Some(report) = effects::harvest_report(snapshot)? else {
        return Ok(None);
    };
    let Some(task) = report.tasks.iter().find(|task| task.task == entity) else {
        return Ok(None);
    };
    Ok(Some(json!({
        "model": report.model,
        "epistemic_state": "certified_derived",
        "evidence_scope": "laboratory_reference_world",
        "source_anchor": crate::observation_projection::anchor_json(&snapshot.anchor()),
        "task": task_json(task),
        "source": report.sources.iter().find(|source| source.source == task.source).map(source_json),
        "credit_on_completion_only": true,
    })))
}

fn source_json(source: &effects::HarvestSource) -> Json {
    json!({
        "source_entity_id": source.source.to_string(),
        "source_generation": source.generation,
        "resource": source.resource.as_str(),
        "initial_units": source.initial,
        "remaining_units": source.remaining,
        "harvested_units": source.harvested,
        "active_demand_units": source.active_demand,
        "available_for_new_plan": source.remaining.saturating_sub(source.active_demand),
        "required_labor": source.resource.labor(),
    })
}

fn task_json(task: &effects::HarvestTask) -> Json {
    json!({
        "task_entity_id": task.task.to_string(),
        "source_entity_id": task.source.to_string(),
        "source_generation": task.source_generation,
        "resource": task.resource.as_str(),
        "ledger_entity_id": task.ledger.to_string(),
        "ledger_generation": task.ledger_generation,
        "status": task.status,
        "amount_total": task.amount_total,
        "amount_remaining": task.amount_remaining,
        "harvested_units": task.harvested,
        "earned_work_ticks": task.work_ticks,
    })
}
