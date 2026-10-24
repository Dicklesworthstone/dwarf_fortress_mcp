//! Source-qualified finite ingredients in the opt-in provisions model.
//! Pending demand is separate from current stock and historical consumption;
//! observing a budget neither reserves ingredients nor predicts completion.

use dfmcp_core::{EntityId, Result};
use dfmcp_intent::effects;
use dfmcp_world::WorldSnapshot;
use serde_json::{Value as Json, json};

/// Screen the proposed low-stock remedy against necessary current inputs.
/// A positive screen is not feasibility proof: the quota compiler also budgets
/// consumption reserves and service. Pending demand is a planning debit only.
pub(super) fn alert_input_blocker(
    snapshot: &WorldSnapshot,
    provisions: &Result<Option<effects::ProvisionsReport>>,
    minimum_batches: u64,
) -> Option<(String, Json)> {
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
            (reason, json!({
                "status": "blocked",
                "epistemic_state": "certified_derived",
                "model": report.model,
                "available_plants": report.available_plants,
                "remaining_demand_plants": report.remaining_demand_plants,
                "available_for_new_plan": available,
                "minimum_new_batches": minimum_batches,
                "pending_demand_is_planning_debit_only": true,
                "next_step": "inspect current ingredients and original orders; replan after sufficient qualified input becomes available",
            }))
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
    Some((reason, readiness))
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
