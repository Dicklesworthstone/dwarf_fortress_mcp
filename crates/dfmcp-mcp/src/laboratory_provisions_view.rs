//! Source-qualified finite ingredients in the opt-in provisions model.
//! Pending demand is separate from current stock and historical consumption;
//! observing a budget neither reserves ingredients nor predicts completion.

use dfmcp_core::{EntityId, Result};
use dfmcp_intent::{HarvestResource, effects};
use dfmcp_world::{EntityKind, Value, WorldSnapshot};
use serde_json::{Value as Json, json};

pub(super) struct AlertInputBlocker {
    pub reason: String,
    pub readiness: Json,
    pub remedy: Json,
}

/// Screen the proposed low-stock remedy against necessary current inputs.
/// A positive screen is not feasibility proof: the quota compiler also budgets
/// consumption reserves and service. Pending demand is a planning debit only.
pub(super) fn alert_input_blocker(
    snapshot: &WorldSnapshot,
    provisions: &Result<Option<effects::ProvisionsReport>>,
    minimum_batches: u64,
) -> Option<AlertInputBlocker> {
    let mut remedy = Json::Null;
    let (reason, mut readiness) = match provisions {
        Ok(Some(report)) => {
            let available = report.available_plants.saturating_sub(report.remaining_demand_plants);
            if available >= minimum_batches {
                return None;
            }
            let reason = format!(
                "the proposed reserve needs at least {minimum_batches} PLANT ingredients, but {} are available and existing food/drink orders need {}, leaving {available} for new work",
                report.available_plants, report.remaining_demand_plants,
            );
            let mut readiness = json!({
                "status": "blocked",
                "epistemic_state": "certified_derived",
                "model": report.model,
                "available_plants": report.available_plants,
                "remaining_demand_plants": report.remaining_demand_plants,
                "available_for_new_plan": available,
                "minimum_new_batches": minimum_batches,
                "pending_demand_is_planning_debit_only": true,
                "next_step": "inspect current ingredients and original orders; replan after sufficient qualified input becomes available",
            });
            if let Some(advice) = harvest_input_remedy(snapshot, report, minimum_batches) {
                readiness["harvesting"] = advice.readiness;
                remedy = advice.remedy;
            }
            (reason, readiness)
        }
        Err(error) => (
            "qualified PLANT inventory or original input history is unavailable; reconcile that evidence before planning additional food or drink production".to_owned(),
            json!({
                "status": "unknown",
                "epistemic_state": "unknown",
                "reason": {"code": error.code.as_str(), "message": error.message},
                "next_step": "observe the original stock ledger and input history before proposing more production",
            }),
        ),
        Ok(None) => return None,
    };
    readiness["evidence_scope"] = json!("laboratory_reference_world");
    readiness["source_anchor"] = crate::observation_projection::anchor_json(&snapshot.anchor());
    Some(AlertInputBlocker {
        reason,
        readiness,
        remedy,
    })
}

struct HarvestAdvice {
    readiness: Json,
    remedy: Json,
}

/// Suggest one bounded acquisition from a complete, qualified source census.
/// Current input remains insufficient until completed work has actually yielded
/// stock. Existing harvest work is observed before proposing additional demand.
fn harvest_input_remedy(
    snapshot: &WorldSnapshot,
    provisions: &effects::ProvisionsReport,
    minimum_batches: u64,
) -> Option<HarvestAdvice> {
    let mut readiness = json!({
        "status": "blocked",
        "epistemic_state": "certified_derived",
        "model": effects::LABORATORY_HARVEST_MODEL,
        "required_labor": HarvestResource::Plant.labor(),
        "evidence_scope": "laboratory_reference_world",
        "source_anchor": crate::observation_projection::anchor_json(&snapshot.anchor()),
        "credit_on_completion_only": true,
        "next_step": "observe completed harvest and replan production against current PLANT stock, pending demand and consumption reserves",
    });
    let report = match effects::harvest_report(snapshot) {
        // Preserve the existing advice in scenarios with no harvesting model.
        Ok(None) => return None,
        Ok(Some(report)) => report,
        Err(error) => {
            readiness["status"] = json!("unknown");
            readiness["epistemic_state"] = json!("unknown");
            readiness["reason"] = json!({"code": error.code.as_str(), "message": error.message});
            return Some(HarvestAdvice {
                readiness,
                remedy: Json::Null,
            });
        }
    };
    if report.ledger != provisions.ledger
        || report.ledger_generation != provisions.ledger_generation
    {
        readiness["status"] = json!("unknown");
        readiness["epistemic_state"] = json!("unknown");
        readiness["reason"] =
            json!("finite harvest and provisions do not share the original stock ledger");
        return Some(HarvestAdvice {
            readiness,
            remedy: Json::Null,
        });
    }
    let Some(shortage) = provisions
        .remaining_demand_plants
        .checked_add(minimum_batches)
        .and_then(|required| required.checked_sub(provisions.available_plants))
        .filter(|shortage| *shortage > 0)
    else {
        readiness["status"] = json!("unknown");
        readiness["epistemic_state"] = json!("unknown");
        readiness["reason"] = json!(
            "the necessary finite input shortage cannot be established within its quantity bound"
        );
        return Some(HarvestAdvice {
            readiness,
            remedy: Json::Null,
        });
    };
    readiness["minimum_missing_plants"] = json!(shortage);
    if report.tasks.iter().any(|task| {
        task.resource == HarvestResource::Plant && task.status == effects::STATUS_ACTIVE
    }) {
        readiness["status"] = json!("awaiting_harvest");
        readiness["reason"] = json!(
            "original PLANT harvest work remains active; observe its completed yield before proposing more acquisition"
        );
        return Some(HarvestAdvice {
            readiness,
            remedy: Json::Null,
        });
    }
    let labor = format!(
        "{}{}",
        effects::LABOR_FIELD_PREFIX,
        HarvestResource::Plant.labor()
    );
    let worker = snapshot.graph.entities.iter().find_map(|(id, unit)| {
        (unit.kind == EntityKind::Unit
            && unit.id == *id
            && *id != EntityId::NIL
            && unit.generation > 0
            && super::production_fact(unit, "alive", snapshot.tick) == Some(&Value::Bool(true))
            && super::production_fact(unit, &labor, snapshot.tick) == Some(&Value::Bool(true)))
        .then_some(unit)
    });
    let Some(worker) = worker else {
        readiness["reason"] =
            json!("a qualified living worker with PLANT_GATHERING enabled is not established");
        return Some(HarvestAdvice {
            readiness,
            remedy: Json::Null,
        });
    };
    let request_limit = shortage.min(u64::from(effects::MAX_HARVEST_AMOUNT));
    let capacity = |source: &effects::HarvestSource| {
        source
            .remaining
            .saturating_sub(source.active_demand)
            .min(request_limit)
    };
    // Prefer a source that covers the largest bounded tranche, then the lowest
    // canonical identity. Choosing it never reserves or credits future yield.
    let selected = report
        .sources
        .iter()
        .filter(|source| source.resource == HarvestResource::Plant && capacity(source) > 0)
        .min_by_key(|source| (std::cmp::Reverse(capacity(source)), source.source));
    let Some(source) = selected else {
        readiness["reason"] =
            json!("no qualified finite PLANT source has uncommitted units available");
        return Some(HarvestAdvice {
            readiness,
            remedy: Json::Null,
        });
    };
    let amount = capacity(source);
    readiness["status"] = json!("ready");
    readiness["reason"] = json!(
        "observed finite supply and enabled gathering labor support a bounded acquisition proposal"
    );
    readiness["source_entity_id"] = json!(source.source.to_string());
    readiness["source_generation"] = json!(source.generation);
    readiness["source_available_units"] =
        json!(source.remaining.saturating_sub(source.active_demand));
    readiness["proposed_harvest_units"] = json!(amount);
    readiness["worker_entity_id"] = json!(worker.id.to_string());
    readiness["worker_generation"] = json!(worker.generation);
    Some(HarvestAdvice {
        readiness,
        remedy: json!({
            "tool": "fortress.plan",
            "requires": "designate",
            "arguments": {"actions": json!([{"action": {
                "kind": "harvest_resource", "source": source.source.to_string(),
                "source_generation": source.generation, "resource": "PLANT", "amount": amount,
            }}]).to_string()},
            "then": "observe completed harvest and replan production against current PLANT stock, pending demand and consumption reserves",
        }),
    })
}

pub(crate) fn provisions_json(snapshot: &WorldSnapshot) -> Result<Option<Json>> {
    let Some(report) = effects::provisions_report(snapshot)? else {
        return Ok(None);
    };
    Ok(Some(json!({
        "model": report.model,
        "available": true,
        "epistemic_state": "certified_derived",
        "evidence_scope": "laboratory_reference_world",
        "source_anchor": crate::observation_projection::anchor_json(&snapshot.anchor()),
        "material": "PLANT",
        "ledger_entity_id": report.ledger.to_string(),
        "ledger_generation": report.ledger_generation,
        "available_plants": report.available_plants,
        "consumed_plants": report.consumed_plants,
        "remaining_demand_plants": report.remaining_demand_plants,
        "missing_plants_for_pending_orders": report.remaining_demand_plants.saturating_sub(report.available_plants),
        "ingredients_reserved": false,
        "orders": report.orders.iter()
            .take(super::MAX_BRIEFING_ITEMS)
            .map(|order| order_json(order, report.available_plants))
            .collect::<Vec<_>>(),
        "orders_omitted": report.orders.len().saturating_sub(super::MAX_BRIEFING_ITEMS),
        "input_rule": "each completed BREW_DRINK, PREPARE_MEAL or COOK_MEAL batch consumes one shared PLANT ingredient and yields five output units; partial work consumes nothing; cancellation preserves consumed history and releases no reserved input",
    })))
}

/// Find one original order in the complete qualified census before applying
/// the briefing row bound. An omitted row is never mistaken for absent work.
pub(crate) fn provisions_action_json(
    snapshot: &WorldSnapshot,
    entity: EntityId,
) -> Result<Option<Json>> {
    let Some(report) = effects::provisions_report(snapshot)? else {
        return Ok(None);
    };
    let Some(order) = report.orders.iter().find(|order| order.order == entity) else {
        return Ok(None);
    };
    Ok(Some(json!({
        "model": report.model,
        "epistemic_state": "certified_derived",
        "evidence_scope": "laboratory_reference_world",
        "source_anchor": crate::observation_projection::anchor_json(&snapshot.anchor()),
        "ledger_entity_id": report.ledger.to_string(),
        "ledger_generation": report.ledger_generation,
        "available_plants": report.available_plants,
        "ingredients_reserved": false,
        "order": order_json(order, report.available_plants),
    })))
}

fn order_json(order: &effects::ProvisionsOrder, available_plants: u64) -> Json {
    let blocked = order.status == effects::STATUS_ACTIVE
        && order.amount_remaining > 0
        && available_plants == 0;
    json!({
        "order_entity_id": order.order.to_string(),
        "job_token": order.job,
        "status": order.status,
        "amount_total": order.amount_total,
        "amount_remaining": order.amount_remaining,
        "consumed_plants": order.consumed_plants,
        "earned_work_ticks": order.work_ticks,
        "input_blocker": blocked.then_some("missing PLANT ingredient for the next completed batch"),
    })
}

#[cfg(test)]
#[path = "laboratory_provisions_harvest_tests.rs"]
mod harvest_tests;
