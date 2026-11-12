//! Bounded Agent Turn presentation; retained views are not fresh evidence.
use crate::agent_turn::{AgentPhase, AgentTurnBuilder, ContinuityStatus, recommendation};
use dfmcp_core::{RequestId, SessionId, StateAnchor, WorkBudget};
use serde_json::{Value, json};

pub(super) const RESPONSE_RESERVE: u64 = 128 * 1024;
const TEXT_LIMIT: usize = 60_000;
const LABOR_BLOCKER_LIMIT: usize = 8;

#[path = "live_production_presentation_stock_targets.rs"]
mod stock_targets;

pub(super) fn compact(view: &Value) -> Value {
    let entries: Vec<Value> = view["frontier"]["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .take(16)
        .map(|entry| {
            json!({"step_id": entry["step_id"].as_u64(),
            "family": bounded(&entry["family"], 64), "phase": bounded(&entry["phase"], 32),
            "idempotency_key": bounded(&entry["idempotency_key"], 128),
            "dispatch_attempted": entry["dispatch_attempted"].as_bool(),
            "preparation_digest": bounded(&entry["preparation_digest"], 64),
            "receipt_digest": bounded(&entry["receipt_digest"], 64)})
        })
        .collect();
    let r = &view["recovery"];
    let mut compacted = json!({"observation_tick_floor":view["observation_tick_floor"].as_u64(),
        "recovery":{"plan_digest":bounded(&r["plan_digest"],64),
        "object_digest":bounded(&r["object_digest"],64), "identity_digest":bounded(&r["identity_digest"],64),
        "goal_history_present":r["goal_history_present"].as_bool(),
        "clock_journal_id":digest(&r["clock_journal_id"]),
        "inventory_clock_journal_id":digest(&r["inventory_clock_journal_id"]),
        "portfolio":r["portfolio"].as_array().map(|rows|rows.iter().take(4).map(|row|json!({
            "step_id":row["step_id"].as_u64(),"goal_history_present":row["goal_history_present"].as_bool(),
            "clock_journal_id":digest(&row["clock_journal_id"]),"inventory_clock_journal_id":digest(&row["inventory_clock_journal_id"])
        })).collect::<Vec<_>>()),
        "anchor":compact_anchor(&r["anchor"]),
        "frontier":{"journal_id":bounded(&r["frontier"]["journal_id"],64),
            "head":bounded(&r["frontier"]["head"],64),"authority_tick_floor":r["frontier"]["authority_tick_floor"].as_u64()}},
        "frontier":{"entries":entries,"next_ready":view["frontier"]["next_ready"].as_u64(),"needs_reconciliation":view["frontier"]["needs_reconciliation"].as_bool(),
            "all_applied":view["frontier"]["all_applied"].as_bool(),"review":null},
        "selected_inventory_goal":compact_goal(&view["selected_inventory_goal"]),
        "selected_order_goal":compact_goal(&view["selected_order_goal"]),
        "portfolio":compact_portfolio(&view["portfolio"]),
        "workshop_prerequisite":workshop_summary(&view["workshop_prerequisite"],false),
        "stock_target":stock_target_summary(&view["stock_target"]),
        "stock_targets":stock_targets::summary(&view["stock_targets"]),
        "selected_parent_goal":null,
        "production_inspection_source":null,"production_inspection":null,
        "current_parent_goal":null,
        "parent_goal_observation_fenced":view["parent_goal_observation_fenced"] == true,
        "clock":clock_summary(&view["clock"],false,ORDER_CLOCK),"clock_control_enabled":false,
        "inventory_clock":clock_summary(&view["inventory_clock"],false,INVENTORY_CLOCK),
        "inventory_clock_control_enabled":false,
        "clock_invalidates_goal":ORDER_CLOCK.invalidates_goal(view),
        "inventory_clock_invalidates_goal":INVENTORY_CLOCK.invalidates_goal(view),
        "original_goal_proven":false,"current_state_proven":false,"details_omitted":true});
    // Absent optional fields stay absent in legacy recovery objects. A retained
    // donor keeps both its association and consumed frontier, never a fresh read.
    if !r["workshop_prerequisite_digest"].is_null() || !r["workshop_prerequisite_floor"].is_null() {
        compacted["recovery"]["workshop_prerequisite_digest"] =
            json!(digest(&r["workshop_prerequisite_digest"]));
        compacted["recovery"]["workshop_prerequisite_floor"] =
            workshop_floor(&r["workshop_prerequisite_floor"]);
    }
    compacted
}

/// A retained census describes why the finite amount was chosen. It never
/// becomes a current count, a renewed plan or an insertion permit on Query.
fn stock_target_summary(value: &Value) -> Value {
    if !value.is_object() {
        return Value::Null;
    }
    json!({"schema":"dfmcp.stock-target/1","status":bounded(&value["status"],32),
        "recipe":bounded(&value["recipe"],32),"item_type":bounded(&value["item_type"],16),
        "baseline_stack_units":value["baseline_stack_units"].as_u64(),
        "target_stack_units":value["target_stack_units"].as_u64(),
        "deficit_stack_units":value["deficit_stack_units"].as_u64(),
        "amount":value["amount"].as_u64(),"max_order_amount":value["max_order_amount"].as_u64(),
        "baseline_anchor":compact_anchor(&value["baseline_anchor"]),
        "source_digest":digest(&value["source_digest"]),
        "source_generation":value["source_generation"].as_u64(),
        "source_counters":value["source_counters"].as_array().filter(|rows|rows.len()==3)
            .map(|rows|rows.iter().map(Value::as_u64).collect::<Vec<_>>()),
        "assessment_witness":digest(&value["assessment_witness"]),
        "consumed_tick_floor":value["consumed_tick_floor"].as_u64(),
        "revalidation_failed":value["revalidation_failed"] == true,
        "last_check_witness":null,
        "work_required":value["work_required"].as_bool(),
        "mutation_planned":value["mutation_planned"].as_bool(),
        "labor_assignment_requested":value["labor_assignment_requested"].as_bool(),
        "labor_assignment_planned":value["labor_assignment_planned"].as_bool(),
        "order_step_id":value["order_step_id"].as_u64(),
        "worker_prerequisite_checked":value["worker_prerequisite_checked"].as_bool(),
        "satisfied_at_anchor":value["status"]=="already_satisfied" && value["satisfied_at_anchor"]==true,
        "empty_queue_required_before_insertion":value["status"]=="planned",
        "exact_original_census_required_before_insertion":value["status"]=="planned",
        "fresh_for_current_call":false,"current_inventory_proven":false,
        "continuous_maintenance":false,"native_recipe_feasibility_proven":false,
        "production_attribution":"unknown","inventory_semantics":"all_observed_type_stack_units"})
}

fn workshop_floor(value: &Value) -> Value {
    if !value.is_object() {
        return Value::Null;
    }
    json!({"tick":value["tick"].as_u64(),"generation":value["generation"].as_u64(),
        "next_building_id":value["next_building_id"].as_u64(),"fenced":value["fenced"].as_bool()})
}

fn workshop_checkpoint(value: &Value) -> Value {
    let mut checkpoint = workshop_floor(value);
    if checkpoint.is_object() {
        checkpoint["origin_witness"] = json!(digest(&value["origin_witness"]));
    }
    checkpoint
}

/// Historical source and association identity only. Even a successful packet
/// must not turn an earlier prerequisite read into current completion evidence.
fn workshop_summary(value: &Value, retain_last: bool) -> Value {
    if !value.is_object() {
        return Value::Null;
    }
    let last = &value["last_qualified"];
    let historical = if retain_last
        && last.is_object()
        && last["status"] == "complete"
        && digest(&last["witness"]).is_some()
    {
        json!({"tick":last["tick"].as_u64(),"witness":digest(&last["witness"]),
            "canonical_parent":compact_anchor(&last["canonical_parent"]),
            "status":"complete","fresh_for_current_call":false,"current_completion_proven":false})
    } else {
        Value::Null
    };
    json!({"schema":bounded(&value["schema"],64),"source_protocol":bounded(&value["source_protocol"],32),
        "scope":"original_workshop_dispatch_prerequisite","status":bounded(&value["status"],32),
        "origin_witness":digest(&value["origin_witness"]),
        "workshop_key":bounded(&value["workshop_key"],128),
        "workshop_plan_digest":digest(&value["workshop_plan_digest"]),
        "workshop_receipt":digest(&value["workshop_receipt"]),
        "building_id":value["building_id"].as_u64(),"source_generation":value["source_generation"].as_u64(),
        "association_digest":digest(&value["association_digest"]),
        "consumed_tick_floor":value["consumed_tick_floor"].as_u64(),"source_fenced":value["source_fenced"].as_bool(),
        "checkpoint":workshop_checkpoint(&value["checkpoint"]),
        "last_qualified":historical,"current_state_proven":false,"current_completion_proven":false,
        "canonical_workshop_evidence":false,"production_feasibility_proven":false,"mutation_authority":false})
}

fn labor_step(view: &Value, id: &Value) -> bool {
    id.is_u64()
        && view["original_plan"]["steps"]
            .as_array()
            .is_some_and(|steps| {
                steps
                    .iter()
                    .any(|step| step["step_id"] == *id && step["action"]["kind"] == "set_labor")
            })
}
fn compact_portfolio(value: &Value) -> Value {
    if value["schema"] != "dfmcp.production-portfolio/1" {
        return Value::Null;
    }
    json!({"schema":"dfmcp.production-portfolio/1","selected_order_step_id":value["selected_order_step_id"].as_u64(),
        "orders":value["orders"].as_array().map(|orders|orders.iter().take(4).map(|order|json!({
            "step_id":order["step_id"].as_u64(),"goal":compact_goal(&order["goal"]),
            "clock":clock_summary(&order["clock"],false,ORDER_CLOCK),
            "inventory_clock":clock_summary(&order["inventory_clock"],false,INVENTORY_CLOCK)
        })).collect::<Vec<_>>()), "complete_original_goal_proven":false,"sampled_orders":0})
}

fn portfolio_parent_valid(view: &Value, parent: &Value, anchor: Option<StateAnchor>) -> bool {
    let Some(orders) = view["portfolio"]["orders"].as_array() else {
        return false;
    };
    let Some(steps) = view["original_plan"]["steps"].as_array() else {
        return false;
    };
    let Some(anchors) = parent["selected_anchors"].as_array() else {
        return false;
    };
    let offset = match steps
        .first()
        .and_then(|step| step["action"]["kind"].as_str())
    {
        Some("set_labor") if steps[0]["step_id"] == 0 && steps[0]["depends_on"] == json!([]) => 1,
        Some("create_work_order") => 0,
        _ => return false,
    };
    if view["portfolio"]["schema"] != "dfmcp.production-portfolio/1"
        || orders.is_empty()
        || orders.len() > 4
        || steps.len() != orders.len() + offset
        || anchors.len() != orders.len()
        || parent["scope"] != "complete_original_production_portfolio"
        || !anchor.is_some_and(|a| parent["anchor"] == anchor_value(a))
        || parent_labor_diagnostics(parent).is_none()
        || parent["original_goal_proven"] != view["original_goal_proven"]
        || parent["original_goal_proven"] != view["portfolio"]["complete_original_goal_proven"]
    {
        return false;
    }
    let mut all_proven = true;
    let mut false_goal = false;
    let mut unknown = false;
    for (index, ((order, selected), step)) in orders
        .iter()
        .zip(anchors)
        .zip(steps.iter().skip(offset))
        .enumerate()
    {
        let goal = &order["goal"];
        if order["step_id"] != (index + offset)
            || selected["step_id"] != order["step_id"]
            || step["step_id"] != order["step_id"]
            || goal["step_id"] != order["step_id"]
            || step["action"]["kind"] != "create_work_order"
            || step["depends_on"] != if offset == 0 { json!([]) } else { json!([0]) }
            || goal["scope"] != "selected_original_order_step"
            || goal["anchor"] != selected["anchor"]
            || !["fortress_id", "epoch", "tick"]
                .iter()
                .all(|field| selected["anchor"][field] == parent["anchor"][field])
            || !selected["anchor"]["sequence"]
                .as_u64()
                .zip(parent["anchor"]["sequence"].as_u64())
                .is_some_and(|(child, parent)| child <= parent)
            || goal["deadline_tick"] != step["obligation"]["deadline_tick"]
            || goal["poll_interval_ticks"] != step["obligation"]["poll_interval_ticks"]
            || goal["stable_samples"] != step["obligation"]["stable_samples"]
            || !matches!(
                goal["kind"].as_str(),
                Some(
                    "retained_order_completion"
                        | "observed_inventory_minimum"
                        | "observed_inventory_increase"
                )
            )
            || !matches!(
                goal["current_truth"].as_str(),
                Some("true" | "false" | "unknown")
            )
            || (goal["proven_at_anchor"] == true
                && (goal["current_truth"] != "true" || goal["history"]["fenced"] == true))
            || (goal["kind"] == "retained_order_completion"
                && goal["proven_at_anchor"] == true
                && goal["receipt_linked"] != true)
        {
            return false;
        }
        all_proven &= goal["proven_at_anchor"] == true;
        false_goal |= goal["current_truth"] == "false";
        unknown |= goal["current_truth"] == "unknown";
    }
    for labor in parent["labor"].as_array().into_iter().flatten() {
        false_goal |= labor["current_truth"] == "false";
        unknown |= labor["current_truth"] == "unknown";
    }
    let Some(inventory_truth) = stock_targets::inventory_truth(view, parent) else {
        return false;
    };
    false_goal |= inventory_truth == "false";
    unknown |= inventory_truth == "unknown";
    let Some(cohort_proven) = stock_targets::stability_proven(view, parent) else {
        return false;
    };
    let truth = if false_goal {
        "false"
    } else if unknown {
        "unknown"
    } else {
        "true"
    };
    let reported_truth = parent["current_truth"].as_str();
    // Existing workers also retain eligibility and historical-identity facts.
    // Negative knowledge from that sealed prerequisite can strengthen the
    // complete parent's failure even when every displayed labor bit matches.
    // Positive parent proof still requires every displayed leaf to be True.
    let consistent = match (offset, truth, reported_truth) {
        (0, "true", Some("false" | "unknown")) | (0, "unknown", Some("false")) => true,
        (_, expected, Some(actual)) => expected == actual,
        _ => false,
    };
    consistent
        && parent["original_goal_proven"]
            == (reported_truth == Some("true") && all_proven && cohort_proven)
}

#[cfg(test)]
#[path = "live_production_presentation_portfolio_tests.rs"]
mod portfolio_tests;
fn compact_goal(value: &Value) -> Value {
    if !value.is_object() {
        return Value::Null;
    }
    let mut result = json!({"scope":"selected_original_order_step",
        "step_id":value["step_id"].as_u64(),"kind":bounded(&value["kind"],64),
        "item_type":bounded(&value["item_type"],16),
        "minimum_stack_units":value["minimum_stack_units"].as_u64(),
        "deadline_tick":value["deadline_tick"].as_u64(),
        "poll_interval_ticks":value["poll_interval_ticks"].as_u64(),
        "stable_samples":value["stable_samples"].as_u64(),
        "first_satisfied_anchor":if value["first_satisfied_anchor"].is_object() {
            compact_anchor(&value["first_satisfied_anchor"])
        } else {Value::Null},
        "history":{"present":value["history"]["present"].as_bool(),
            "fenced":value["history"]["fenced"].as_bool()},
        "current_truth":null,"proven_at_anchor":false,"original_goal_proven":false,
        "production_attribution":"unknown"});
    if value["kind"] == "retained_order_completion" {
        result["order_entity_id"] = json!(bounded(&value["order_entity_id"], 20));
        result["required_order_state"] = json!({
            "amount_total":value["required_order_state"]["amount_total"].as_u64(),
            "amount_remaining":0,"validated":true,"active":false
        });
        result["disappearance_proves_completion"] = json!(false);
        if let Some(object) = result.as_object_mut() {
            object.remove("item_type");
            object.remove("minimum_stack_units");
        }
    } else if value["kind"] == "observed_inventory_increase" {
        for field in [
            "baseline_stack_units",
            "increase_stack_units",
            "target_stack_units",
            "baseline_source_generation",
        ] {
            result[field] = json!(value[field].as_u64());
        }
        result["baseline_anchor"] = compact_anchor(&value["baseline_anchor"]);
        result["baseline_source_digest"] = json!(digest(&value["baseline_source_digest"]));
        result["baseline_source_protocol"] = json!("operations/1.3");
        result["baseline_source_counters"] = json!(
            value["baseline_source_counters"].as_array().map(|rows| rows
                .iter()
                .take(3)
                .map(Value::as_u64)
                .collect::<Vec<_>>())
        );
        result["preexisting_stock_alone_satisfies_goal"] = json!(false);
        result["new_production"] = json!("unknown");
        if let Some(object) = result.as_object_mut() {
            object.remove("minimum_stack_units");
        }
    }
    result
}
fn bounded(value: &Value, max: usize) -> Option<&str> {
    value
        .as_str()
        .filter(|s| s.len() <= max && !s.chars().any(char::is_control))
}
fn digest(value: &Value) -> Option<&str> {
    bounded(value, 64).filter(|s| {
        s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            && s.bytes().any(|b| b != b'0')
    })
}

/// One presentation path serves both separately authorized clock owners. Their
/// identities, confirmation scopes and retention limits never substitute.
#[derive(Clone, Copy, PartialEq, Eq)]
struct ClockOwner {
    field: &'static str,
    scope: &'static str,
    source: &'static str,
    target_scope: &'static str,
    authority_field: &'static str,
    invalidates_field: &'static str,
    recovery_field: &'static str,
    coverage_domain: &'static str,
    max_runs: u64,
}
const ORDER_CLOCK: ClockOwner = ClockOwner {
    field: "clock",
    scope: "run",
    source: "order-run/1.14",
    target_scope: "selected_original_order_step",
    authority_field: "clock_control_enabled",
    invalidates_field: "clock_invalidates_goal",
    recovery_field: "clock_journal_id",
    coverage_domain: "original_order_clock_custody",
    max_runs: 32,
};
const INVENTORY_CLOCK: ClockOwner = ClockOwner {
    field: "inventory_clock",
    scope: "inventory_run",
    source: "run/1.13",
    target_scope: "selected_original_inventory_goal",
    authority_field: "inventory_clock_control_enabled",
    invalidates_field: "inventory_clock_invalidates_goal",
    recovery_field: "inventory_clock_journal_id",
    coverage_domain: "original_inventory_clock_custody",
    max_runs: 64,
};
const CLOCK_OWNERS: [ClockOwner; 2] = [ORDER_CLOCK, INVENTORY_CLOCK];
impl ClockOwner {
    fn view(self, view: &Value) -> &Value {
        &view[self.field]
    }
    fn linked(self, view: &Value) -> bool {
        let clock = self.view(view);
        let recovery = if view["portfolio"]["schema"] == "dfmcp.production-portfolio/1" {
            view["recovery"]["portfolio"]
                .as_array()
                .and_then(|entries| {
                    entries.iter().find(|entry| {
                        entry["step_id"] == view["portfolio"]["selected_order_step_id"]
                    })
                })
                .unwrap_or(&Value::Null)
        } else {
            &view["recovery"]
        };
        clock["source"] == self.source
            && clock["target_scope"] == self.target_scope
            && digest(&clock["journal_id"]).is_some()
            && clock["journal_id"] == recovery[self.recovery_field]
            && digest(&clock["target_digest"]).is_some()
            && digest(&clock["original_paired_seal"]).is_some()
    }
    fn invalidates_goal(self, view: &Value) -> bool {
        view[self.invalidates_field] == true || clock_unresolved(self.view(view))
    }
    fn other(self) -> Self {
        if self == ORDER_CLOCK {
            INVENTORY_CLOCK
        } else {
            ORDER_CLOCK
        }
    }
}
fn clock_unresolved(clock: &Value) -> bool {
    clock["inspection_required"] == true
        || clock["unresolved"] == true
        || clock["may_be_running"] == true
        || clock["active_run"]["unresolved"] == true
        || matches!(
            clock["active_run"]["state"].as_str(),
            Some("dispatch_started" | "tracking" | "cancel_requested")
        )
        || matches!(
            clock["active_run"]["native"]["phase"].as_str(),
            Some("running" | "stopping" | "source_lost")
        )
}
fn clock_reconciliation(clock: &Value) -> bool {
    clock_unresolved(clock)
        || (clock["needs_reconciliation"] == true && clock["active_run"]["state"] != "prepared")
}
fn clock_settled(clock: &Value) -> bool {
    clock.is_null()
        || (clock.is_object()
            && clock["pending"] == false
            && clock["unresolved"] == false
            && clock["may_be_running"] == false
            && clock["needs_reconciliation"] != true
            && !clock_unresolved(clock)
            && clock["active_run"].is_null())
}
/// Preserve bounded recovery identity without carrying an old confirmation.
fn clock_summary(clock: &Value, reviewed: bool, owner: ClockOwner) -> Value {
    if !clock.is_object() {
        return Value::Null;
    }
    let active = &clock["active_run"];
    let native = &active["native"];
    let native_summary = if native.is_object() {
        json!({"phase":bounded(&native["phase"],32),
            "reason":bounded(&native["reason"],64),"trigger":bounded(&native["trigger"],64),
            "receipt":digest(&native["receipt"]),"observed_tick":native["observed_tick"].as_u64(),
            "unpause_attempted":native["unpause_attempted"].as_bool(),
            "historical_pause_verified":native["historical_pause_verified"].as_bool(),
            "current_pause_proven":false,"original_goal_proven":false})
    } else {
        Value::Null
    };
    let active_summary = if active.is_object() {
        json!({"review_seal":if reviewed {digest(&active["review_seal"])}else{None},
            "key":bounded(&active["key"],128),"plan_digest":digest(&active["plan_digest"]),
            "state":bounded(&active["state"],32),"pending":active["pending"].as_bool(),
            "unresolved":active["unresolved"].as_bool(),"native_order_id":active["native_order_id"].as_u64(),
            "before_tick":active["before_tick"].as_u64(),"game_ticks":active["game_ticks"].as_u64(),
            "wall_millis":active["wall_millis"].as_u64(),"purpose":bounded(&active["purpose"],64),
            "native":native_summary})
    } else {
        Value::Null
    };
    json!({"journal_id":digest(&clock["journal_id"]),"head":digest(&clock["head"]),
        "mode":bounded(&clock["mode"],16),"source":bounded(&clock["source"],32),
        "target_scope":bounded(&clock["target_scope"],64),
        "independent_authority":clock["independent_authority"].as_bool(),
        "source_identity_semantics":bounded(&clock["source_identity_semantics"],96),
        "target_digest":digest(&clock["target_digest"]),"original_paired_seal":digest(&clock["original_paired_seal"]),
        "pending":clock["pending"].as_bool(),"unresolved":clock_unresolved(clock),
        "may_be_running":clock_unresolved(clock),"needs_reconciliation":clock["needs_reconciliation"].as_bool(),
        "inspection_required":clock["inspection_required"] == true,
        "run_count":clock["run_count"].as_u64().filter(|n|*n<=owner.max_runs),"high_tick":clock["high_tick"].as_u64(),
        "review_available":reviewed,"current_pause_proven":false,"original_goal_proven":false,
        "active_run":active_summary})
}
fn compact_anchor(value: &Value) -> Value {
    json!({"fortress_id":bounded(&value["fortress_id"],20),"epoch":value["epoch"].as_u64(),
        "sequence":value["sequence"].as_u64(),"tick":value["tick"].as_u64(),
        "state_hash":bounded(&value["state_hash"],64)})
}
fn anchor_value(a: StateAnchor) -> Value {
    json!({"fortress_id":a.fortress_id.get().to_string(),"epoch":a.cursor.epoch,
        "sequence":a.cursor.sequence,"tick":a.tick.get(),"state_hash":a.state_hash.to_hex()})
}

/// An optional operations capture retains its own source-local cursor/hash.
/// Its citizen anchor binds the planning context, without turning inspection
/// into an original goal sample or mutation precondition evidence.
fn preplan_inspection_matches(view: &Value, source: &Value) -> bool {
    let source_anchor = &source["anchor"];
    view["original_plan"].is_null()
        && view["observation"]["paused"] == true
        && digest(&view["observation"]["seal"]).is_some()
        && source["scope"] == "preplan_captured_operations"
        && source["attribution"] == "not_order_attributed"
        && source["order_step_id"].is_null()
        && source["parent_anchor"] == view["anchor"]
        && source["canonical_anchor"] == view["anchor"]
        && source_anchor["fortress_id"] == view["anchor"]["fortress_id"]
        && source_anchor["tick"] == view["anchor"]["tick"]
        && source_anchor["epoch"].is_u64()
        && source_anchor["sequence"].is_u64()
        && digest(&source_anchor["state_hash"]).is_some()
        && digest(&source["source_digest"]).is_some()
        && source["mutation_authority"] == false
        && source["restored_from_history"] == false
        && source["diagnosis_request"]["schema"] == "dfmcp.query/1"
        && source["diagnosis_request"]["query"]["kind"] == "production_diagnosis"
        && source["diagnosis_request"]["expected_anchor"] == *source_anchor
}

/// A new read preserves the caller's selected citizens and choice windows.
/// Only existing closed selector fields are copied into the suggested call.
fn production_observation_request(view: &Value) -> Value {
    let selection = &view["observation"]["workforce_choices"]["selection_request"];
    let mut request = json!({"include_production":true});
    for field in ["units", "option_offset", "existing_labor_offset"] {
        if let Some(value) = selection.get(field) {
            request[field] = value.clone();
        }
    }
    request
}

struct ParentLaborDiagnostics {
    summary: Value,
    blockers: Vec<Value>,
    omitted: usize,
}

fn parent_labor_diagnostics(parent: &Value) -> Option<ParentLaborDiagnostics> {
    let rows = parent["labor"].as_array()?;
    if rows.is_empty() || rows.len() > 32 {
        return None;
    }
    let mut previous_id = 0;
    let (mut matched, mut mismatched, mut unknown) = (0, 0, 0);
    let mut blockers = Vec::with_capacity(LABOR_BLOCKER_LIMIT);
    for row in rows {
        let unit_id = bounded(&row["unit_id"], 20)?;
        let id = unit_id.parse::<u64>().ok()?;
        let labor = bounded(&row["labor"], 64).filter(|labor| !labor.is_empty())?;
        let requested = row["requested_enabled"].as_bool()?;
        let observed = match row.get("observed_enabled")? {
            Value::Null => None,
            Value::Bool(enabled) => Some(*enabled),
            _ => return None,
        };
        // Diagnostics are all-or-nothing: missing or inconsistent rows cannot
        // become a zero-blocker summary. Canonical IDs stay decimal strings.
        if id <= previous_id || id.to_string() != unit_id {
            return None;
        }
        previous_id = id;
        let truth = match observed {
            Some(enabled) if enabled == requested => {
                matched += 1;
                "true"
            }
            Some(_) => {
                mismatched += 1;
                "false"
            }
            None => {
                unknown += 1;
                "unknown"
            }
        };
        if row["current_truth"] != truth {
            return None;
        }
        if truth != "true" && blockers.len() < LABOR_BLOCKER_LIMIT {
            blockers.push(json!({"unit_id":unit_id,"labor":labor,
                "requested_enabled":requested,"observed_enabled":observed,
                "current_truth":truth}));
        }
    }
    if (parent["current_truth"] == "true" && mismatched + unknown > 0)
        || (parent["current_truth"] == "unknown" && mismatched > 0)
    {
        return None;
    }
    Some(ParentLaborDiagnostics {
        summary: json!({"citizens":rows.len(),"matched":matched,
            "mismatched":mismatched,"unknown":unknown}),
        omitted: mismatched + unknown - blockers.len(),
        blockers,
    })
}

#[allow(clippy::too_many_arguments)]
fn encode(
    operation: &str,
    session: Option<SessionId>,
    anchor: Option<StateAnchor>,
    mode: Option<&str>,
    request: Option<RequestId>,
    budget: Option<WorkBudget>,
    mut result: Value,
    retained: Option<&Value>,
    verified: bool,
    uncertain: bool,
) -> String {
    let phase = match operation {
        "fortress.open_session" => AgentPhase::Bootstrap,
        "fortress.observe" => AgentPhase::Orient,
        "fortress.plan" => AgentPhase::Propose,
        "fortress.commit" => AgentPhase::Commit,
        "fortress.wait" => AgentPhase::Verify,
        "fortress.cancel" => AgentPhase::Reconcile,
        _ => AgentPhase::Inspect,
    };
    let view = retained.unwrap_or(&Value::Null);
    let stock_target = stock_target_summary(&view["stock_target"]);
    let stock_targets = stock_targets::summary(&view["stock_targets"]);
    let stock_target_failed = stock_target["revalidation_failed"] == true
        || stock_targets["revalidation_failed"] == true
        || (!view["stock_targets"].is_null() && stock_targets.is_null());
    let workshop_fenced = view["workshop_prerequisite"]["source_fenced"] == true;
    let portfolio = view["portfolio"]["schema"] == "dfmcp.production-portfolio/1";
    let clock_reconciliation_needed = CLOCK_OWNERS
        .iter()
        .any(|owner| clock_reconciliation(owner.view(view)))
        || view["portfolio"]["orders"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|order| {
                clock_reconciliation(&order["clock"])
                    || clock_reconciliation(&order["inventory_clock"])
            });
    let uncertain = uncertain || clock_reconciliation_needed;
    let workshop = workshop_summary(
        &view["workshop_prerequisite"],
        verified && !uncertain && !workshop_fenced && mode != Some("offline"),
    );
    let clock_invalidates_goal = CLOCK_OWNERS
        .iter()
        .any(|owner| owner.invalidates_goal(view));
    let entries = view["frontier"]["entries"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let workshop_blocks_review = (workshop_fenced || stock_target_failed)
        && !(view["review"]["family"] == "workforce/1.17"
            && labor_step(view, &view["review"]["step_id"]));
    let review = if verified && !uncertain && !workshop_blocks_review {
        view["review"].clone()
    } else {
        Value::Null
    };
    // The original has exactly one selected objective. Conflicting projections
    // cannot choose whichever one happens to advertise success.
    let inventory_goal = &view["selected_inventory_goal"];
    let order_goal = &view["selected_order_goal"];
    let retained_order = inventory_goal.is_null()
        && order_goal["kind"] == "retained_order_completion"
        && order_goal["scope"] == "selected_original_order_step";
    let raw_selected_goal = if retained_order {
        order_goal
    } else if order_goal.is_null()
        && matches!(
            inventory_goal["kind"].as_str(),
            Some("observed_inventory_minimum" | "observed_inventory_increase")
        )
    {
        inventory_goal
    } else {
        &Value::Null
    };
    let parent_scope = if portfolio {
        "selected_original_labor_and_production_goal"
    } else if retained_order {
        "complete_original_labor_and_order_goal"
    } else {
        "complete_original_labor_and_inventory_goal"
    };
    let selected_goal = if verified && !uncertain {
        if clock_invalidates_goal {
            compact_goal(raw_selected_goal)
        } else {
            raw_selected_goal.clone()
        }
    } else {
        Value::Null
    };
    let selected_proven = selected_goal["proven_at_anchor"] == true;
    let parent_fenced = view["parent_goal_observation_fenced"] == true;
    // The joined sample has a different state hash from its selected child.
    // Present it only at the exact composite anchor, linked to that child; a
    // retained receipt, inventory achievement or top-level flag cannot fill it.
    let candidate = if portfolio {
        &view["selected_parent_goal"]
    } else {
        &view["current_parent_goal"]
    };
    let joined_goal = if verified
        && !uncertain
        && !clock_invalidates_goal
        && !parent_fenced
        && mode != Some("offline")
        && candidate["scope"] == parent_scope
        && (view["stock_targets"].is_null() || portfolio)
        && stock_targets::current_inventory_consistent(view, candidate)
        && matches!(
            candidate["current_truth"].as_str(),
            Some("true" | "false" | "unknown")
        )
        && anchor.is_some_and(|a| candidate["anchor"] == anchor_value(a))
        && candidate["selected_anchor"].is_object()
        && candidate["selected_anchor"] == selected_goal["anchor"]
        && ["fortress_id", "epoch", "tick"]
            .iter()
            .all(|&field| candidate["anchor"][field] == candidate["selected_anchor"][field])
        && if portfolio {
            candidate["selected_anchor"]["sequence"]
                .as_u64()
                .zip(candidate["anchor"]["sequence"].as_u64())
                .is_some_and(|(source, published)| source <= published)
        } else {
            candidate["anchor"]["sequence"] == candidate["selected_anchor"]["sequence"]
        }
        && candidate["original_goal_proven"]
            == (!portfolio && candidate["current_truth"] == "true" && selected_proven)
        && (portfolio || candidate["original_goal_proven"] == view["original_goal_proven"])
    {
        candidate.clone()
    } else {
        Value::Null
    };
    let parent_goal = if portfolio {
        if verified
            && !uncertain
            && !clock_invalidates_goal
            && !parent_fenced
            && mode != Some("offline")
            && portfolio_parent_valid(view, &view["current_parent_goal"], anchor)
        {
            view["current_parent_goal"].clone()
        } else {
            Value::Null
        }
    } else {
        joined_goal.clone()
    };
    let parent_proven = parent_goal["original_goal_proven"] == true;
    let parent_truth = parent_goal["current_truth"].as_str();
    let control_available =
        verified && session.is_some() && result["closed"] != true && mode == Some("control");
    let clock_authority = |owner: ClockOwner| {
        control_available
            && view[owner.authority_field] == true
            && owner.view(view)["mode"] == "control"
    };
    let selected_step = view["original_plan"]["steps"].as_array().and_then(|steps| {
        steps.iter().find(|step| {
            step["step_id"] == selected_goal["step_id"]
                && step["action"]["kind"] == "create_work_order"
        })
    });
    let deadline = selected_goal["deadline_tick"]
        .as_u64()
        .filter(|deadline| {
            selected_step.is_some_and(|step| step["obligation"]["deadline_tick"] == *deadline)
        })
        .map(|deadline| {
            deadline.min(
                view["original_plan"]["expires_at_tick"]
                    .as_u64()
                    .unwrap_or(deadline),
            )
        });
    let known_tick = anchor.map(|a| {
        a.tick
            .get()
            .max(view["observation_tick_floor"].as_u64().unwrap_or(0))
            .max(view["clock"]["high_tick"].as_u64().unwrap_or(0))
            .max(view["inventory_clock"]["high_tick"].as_u64().unwrap_or(0))
    });
    let current_tick = anchor
        .is_some_and(|a| view["anchor"] == anchor_value(a) && known_tick == Some(a.tick.get()));
    // A complete cohort has its own public anchor. Its exact selected leaf
    // remains usable clock evidence without relabeling the selected anchor.
    let cohort_covers_selected = portfolio
        && parent_goal.is_object()
        && selected_goal["step_id"] == view["portfolio"]["selected_order_step_id"]
        && view["portfolio"]["orders"]
            .as_array()
            .is_some_and(|orders| {
                orders.iter().any(|order| {
                    order["step_id"] == selected_goal["step_id"] && order["goal"] == selected_goal
                })
            });
    let clock_parent = if joined_goal.is_object() {
        &joined_goal
    } else if cohort_covers_selected {
        &parent_goal
    } else {
        &Value::Null
    };
    let parent_stability_needs_time = portfolio
        && stock_targets.is_object()
        && clock_parent["current_truth"] == "true"
        && (joined_goal["parent_stability_pending"] == true
            || (cohort_covers_selected && parent_goal["parent_stability"]["status"] == "pending"))
        && stock_targets::inventory_truth(view, clock_parent) == Some("true")
        && (view["current_parent_goal"].is_null() || parent_goal.is_object())
        && deadline
            .zip(known_tick)
            .is_some_and(|(deadline, tick)| tick < deadline);
    let current_goal_ready = !uncertain
        && !clock_invalidates_goal
        && !parent_fenced
        && clock_parent.is_object()
        && !parent_proven
        && view["frontier"]["all_applied"] == true
        && view["frontier"]["needs_reconciliation"] != true
        && selected_goal["scope"] == "selected_original_order_step"
        && matches!(
            selected_goal["kind"].as_str(),
            Some(
                "observed_inventory_minimum"
                    | "observed_inventory_increase"
                    | "retained_order_completion"
            )
        )
        && (!selected_proven || parent_stability_needs_time)
        && selected_goal["terminal_failure"] != true
        && selected_goal["history"]["fenced"] != true
        && current_tick;
    let clock_goal_ready = |owner: ClockOwner| {
        current_goal_ready
            && if owner == INVENTORY_CLOCK {
                matches!(
                    selected_goal["kind"].as_str(),
                    Some("observed_inventory_minimum" | "observed_inventory_increase")
                ) && selected_goal["current_truth"] == "true"
                    && clock_parent["current_truth"] == "true"
            } else {
                selected_goal["receipt_linked"] == true
                    && ((selected_goal["current_truth"] == "false"
                        && clock_parent["current_truth"] == "false")
                        || (selected_goal["current_truth"] == "true"
                            && clock_parent["current_truth"] == "true"))
            }
    };
    let allowed_ticks = deadline
        .zip(known_tick)
        .and_then(|(deadline, tick)| deadline.checked_sub(tick))
        .zip(budget)
        .map(|(remaining, budget)| remaining.min(budget.max_game_ticks).min(1200))
        .filter(|ticks| *ticks > 0);
    let proposed_ticks = if selected_goal["current_truth"] == "true" {
        allowed_ticks
            .zip(selected_goal["poll_interval_ticks"].as_u64())
            .map(|(ticks, cadence)| ticks.min(cadence))
            .filter(|ticks| *ticks > 0)
    } else {
        allowed_ticks
    };
    let proposed_wall = budget
        .map(|b| b.max_wall_millis.min(5000))
        .filter(|wall| *wall > 0);
    let inactive_pending_order = if portfolio {
        view["portfolio"]["orders"]
            .as_array()
            .into_iter()
            .flatten()
            .find_map(|order| {
                (order["step_id"] != view["portfolio"]["selected_order_step_id"]
                    && ["clock", "inventory_clock"].iter().any(|field| {
                        order[*field]["pending"] == true
                            || order[*field]["active_run"]["state"] == "prepared"
                    }))
                .then(|| order["step_id"].as_u64())
                .flatten()
            })
    } else {
        None
    };
    let new_run_proposal = |owner: ClockOwner| {
        let clock = owner.view(view);
        (inactive_pending_order.is_none() && clock_authority(owner)
            && owner.linked(view)
            && clock_goal_ready(owner)
            && clock_settled(clock)
            && clock_settled(owner.other().view(view))
            && clock["run_count"]
                .as_u64()
                .is_some_and(|count| count < owner.max_runs))
        .then_some(proposed_ticks.zip(proposed_wall))
        .flatten()
        .map(|(ticks, wall)| {
            let mut request = json!({"kind":owner.scope,"original_paired_seal":clock["original_paired_seal"],
                "game_ticks":ticks,"wall_millis":wall});
            if portfolio { request["order_step_id"] = view["portfolio"]["selected_order_step_id"].clone(); }
            request
        })
    };
    let order_proposal = new_run_proposal(ORDER_CLOCK);
    let inventory_proposal = new_run_proposal(INVENTORY_CLOCK);
    // Retained work takes precedence over a new run in either owner. Once both
    // settle, prefer the independent inventory path for already true stock;
    // it remains available after the original manager order has retired.
    let clock_owner = CLOCK_OWNERS
        .into_iter()
        .find(|owner| clock_reconciliation(owner.view(view)))
        .or_else(|| {
            CLOCK_OWNERS.into_iter().find(|owner| {
                let clock = owner.view(view);
                clock["pending"] == true || clock["active_run"].is_object()
            })
        })
        .unwrap_or_else(|| {
            if inventory_proposal.is_some() || !view["clock"].is_object() {
                INVENTORY_CLOCK
            } else {
                ORDER_CLOCK
            }
        });
    let clock = clock_owner.view(view);
    let clock_active = &clock["active_run"];
    let clock_uncertain = clock_unresolved(clock);
    let clock_prepared = clock_authority(clock_owner)
        && clock_owner.linked(view)
        && clock_goal_ready(clock_owner)
        && clock_settled(clock_owner.other().view(view))
        && clock["pending"] == true
        && !clock_uncertain
        && clock_active["state"] == "prepared"
        && clock_active["pending"] == true
        && clock_active["unresolved"] == false
        && digest(&clock_active["review_seal"]).is_some()
        && clock_active["before_tick"].as_u64() == known_tick
        && clock_active["game_ticks"]
            .as_u64()
            .zip(allowed_ticks)
            .is_some_and(|(ticks, allowed)| ticks > 0 && ticks <= allowed)
        && clock_active["wall_millis"]
            .as_u64()
            .zip(budget)
            .is_some_and(|(wall, budget)| {
                wall > 0 && wall <= 60000 && wall <= budget.max_wall_millis
            });
    let clock_commit = clock_prepared && clock["review_available"] == true;
    let clock_rereview = clock_prepared && clock["review_available"] == false;
    let mut clock_proposal = if clock_rereview {
        Some(
            json!({"kind":clock_owner.scope,"original_paired_seal":clock["original_paired_seal"],
            "game_ticks":clock_active["game_ticks"],"wall_millis":clock_active["wall_millis"]}),
        )
    } else if clock_owner == INVENTORY_CLOCK {
        inventory_proposal.clone()
    } else {
        order_proposal.clone()
    };
    if portfolio && let Some(request) = &mut clock_proposal {
        request["order_step_id"] = view["portfolio"]["selected_order_step_id"].clone();
    }
    // Stopping an existing run does not require an unexpired or unsatisfied goal.
    let clock_cancel = |owner: ClockOwner| {
        let clock = owner.view(view);
        clock_authority(owner)
            && owner.linked(view)
            && clock["active_run"].is_object()
            && (clock["pending"] == true || clock_unresolved(clock))
    };
    let clock_source_lost =
        clock_active["native"]["phase"] == "source_lost" && clock["needs_reconciliation"] == false;
    let labor = parent_labor_diagnostics(&parent_goal);
    let inspection_source = &view["production_inspection_source"];
    let current_inspection_allowed = verified
        && !uncertain
        && mode != Some("offline")
        && !clock_invalidates_goal
        && !parent_fenced
        && current_tick;
    let selected_inspection_available = current_inspection_allowed
        && inspection_source["scope"] == "captured_operations"
        && inspection_source["attribution"] == "not_order_attributed"
        && inspection_source["parent_anchor"] == view["anchor"]
        && inspection_source["anchor"] == selected_goal["anchor"]
        && inspection_source["order_step_id"] == selected_goal["step_id"]
        && digest(&inspection_source["source_digest"]).is_some();
    let preplan_inspection_available = current_inspection_allowed
        && session.is_some()
        && result["closed"] != true
        && preplan_inspection_matches(view, inspection_source);
    let inspection_available = selected_inspection_available || preplan_inspection_available;
    let inspection = &view["production_inspection"];
    let diagnosis_seen = inspection_available
        && inspection["kind"] == "production_diagnosis"
        && inspection["anchor"] == inspection_source["anchor"]
        && inspection["source_digest"] == inspection_source["source_digest"]
        && (!preplan_inspection_available
            || (inspection["inspection_scope"] == "preplan_captured_operations"
                && inspection["canonical_anchor"] == view["anchor"]));
    let preplan_capture_needed = current_inspection_allowed
        && view["original_plan"].is_null()
        && view["observation"]["paused"] == true
        && digest(&view["observation"]["seal"]).is_some()
        && !preplan_inspection_available;
    let preplan_diagnosis_needed = preplan_inspection_available && !diagnosis_seen;
    let recommendation = if session.is_none() || result["closed"] == true {
        None
    } else if uncertain && mode == Some("offline") {
        Some((
            "fortress_cancel",
            "Retain the original recovery reference, close this offline session, and reopen in reconcile mode; closing does not cancel the effect.",
            false,
            json!({"scope":"session"}),
        ))
    } else if uncertain
        && clock_source_lost
        && view["frontier"]["all_applied"] == true
        && view["frontier"]["needs_reconciliation"] != true
    {
        Some((
            "fortress_doctor",
            "Inspect the original clock custody and resolve the lost source; its receipt cannot establish that the fortress is paused.",
            false,
            json!({}),
        ))
    } else if uncertain
        && parent_fenced
        && !clock_reconciliation_needed
        && view["frontier"]["all_applied"] == true
        && view["frontier"]["needs_reconciliation"] != true
    {
        // A failed goal observation cannot be retried past its source fence.
        // This only removes guidance; retained effects remain indeterminate.
        None
    } else if uncertain {
        Some((
            "fortress_wait",
            if clock_reconciliation_needed {
                "Reconcile the retained clock run before another dispatch or fresh goal sample; wait does not start another run."
            } else {
                "Reconcile the original uncertain key before any new dispatch."
            },
            false,
            json!({}),
        ))
    } else if !verified {
        None
    } else if let Some(step) = inactive_pending_order {
        Some((
            "fortress_query",
            "Inspect the original portfolio order with retained clock work before reviewing another run.",
            false,
            json!({"order_step_id":step}),
        ))
    } else if clock_commit {
        Some((
            "fortress_commit",
            "Confirm this separate clock review to attempt one bounded unpause after fresh original-custody checks.",
            true,
            json!({"paired_seal":clock_active["review_seal"],"confirmed":true,"scope":clock_owner.scope}),
        ))
    } else if clock_rereview {
        Some((
            "fortress_plan",
            "Request a fresh review of the exact retained prepared run; this does not start game time or restore the prior confirmation.",
            false,
            json!({"request":clock_proposal.as_ref().map(Value::to_string)}),
        ))
    } else if clock_cancel(clock_owner) && clock_active["state"] == "prepared" {
        Some((
            "fortress_cancel",
            "Cancel the retained run preparation before choosing a new bounded clock slice.",
            false,
            json!({"scope":clock_owner.scope}),
        ))
    } else if parent_proven || parent_fenced {
        None
    } else if !review.is_null() {
        Some((
            "fortress_commit",
            "Confirm the exact current paired review.",
            true,
            json!({"paired_seal":review["paired_seal"],"confirmed":true}),
        ))
    } else if view["frontier"]["next_ready"].is_u64()
        && view["original_plan"]["plan_digest"].is_string()
        && !stock_target_failed
        && (!workshop_fenced || labor_step(view, &view["frontier"]["next_ready"]))
    {
        Some((
            "fortress_plan",
            "Prepare the next dependency-satisfied original step.",
            false,
            json!({"request":json!({"kind":"step", "original_plan_digest":view["original_plan"]["plan_digest"]}).to_string()}),
        ))
    } else if inspection_available
        && selected_goal["current_truth"] == "false"
        && !diagnosis_seen
        && inspection_source["diagnosis_request"].is_object()
    {
        let mut arguments = json!({
            "request":inspection_source["diagnosis_request"].to_string()
        });
        if portfolio {
            arguments["order_step_id"] = inspection_source["order_step_id"].clone();
        }
        Some((
            "fortress_query",
            "Inspect observed job conditions before choosing another bounded run. The captured jobs have no proven attribution to this original order.",
            false,
            arguments,
        ))
    } else if let Some(request) = &clock_proposal {
        Some((
            "fortress_plan",
            if retained_order && selected_goal["current_truth"] == "true" {
                "Prepare bounded game-time progress for the original retained-order stability interval, then review its separate clock seal."
            } else if retained_order {
                "Prepare a bounded run for the pending original finite order, then review its separate clock seal."
            } else if selected_goal["current_truth"] == "true" {
                "Prepare bounded game-time progress for the original inventory stability interval, then review its separate clock seal."
            } else {
                "Prepare a bounded run for the pending original inventory minimum, then review its separate clock seal."
            },
            false,
            json!({"request":request.to_string()}),
        ))
    } else if portfolio
        && current_goal_ready
        && CLOCK_OWNERS
            .iter()
            .any(|owner| clock_authority(*owner) && !owner.linked(view))
    {
        Some((
            "fortress_query",
            "Inspect this original portfolio order's retained clock custody before reviewing a bounded run.",
            false,
            json!({"order_step_id":view["portfolio"]["selected_order_step_id"]}),
        ))
    } else if portfolio
        && mode != Some("offline")
        && view["frontier"]["all_applied"] == true
        && !parent_proven
    {
        Some((
            "fortress_wait",
            "Observe the next original portfolio goal; whole-plan proof requires the complete coherent cohort.",
            false,
            json!({}),
        ))
    } else if mode != Some("offline")
        && view["frontier"]["all_applied"] == true
        && selected_goal.is_object()
        && !selected_proven
        && selected_goal["terminal_failure"] != true
        && selected_goal["history"]["fenced"] != true
    {
        Some((
            "fortress_wait",
            if retained_order && view["clock"].is_object() {
                "Take one fresh coherent sample of the original labor and retained-order completion goal; disappearance is unknown and wait does not start a clock run."
            } else if retained_order {
                "Take one coherent sample of the original labor and retained-order completion goal; disappearance is unknown and additional stability samples require separately managed game-time progress."
            } else if CLOCK_OWNERS
                .iter()
                .any(|owner| owner.view(view).is_object())
            {
                "Take one fresh coherent sample of the original labor and inventory goal; wait does not start a clock run."
            } else {
                "Take one coherent sample of the original labor and inventory goal; additional stability samples require separately managed game-time progress."
            },
            false,
            json!({}),
        ))
    } else if view["original_plan"].is_null()
        && (view["observation"].is_null() || view["observation"]["workforce_choices"].is_null())
    {
        Some((
            "fortress_observe",
            "Acquire the paused canonical roster and advisory workforce choices before planning.",
            false,
            json!({}),
        ))
    } else if (stock_target["status"] == "already_satisfied"
        || stock_targets["status"] == "already_satisfied")
        && view["original_plan"].is_null()
        && mode != Some("offline")
    {
        Some((
            "fortress_observe",
            "All requested stock targets already held at the reported anchor and no order was planned. Acquire a new observation before considering a later production goal.",
            false,
            json!({}),
        ))
    } else if preplan_capture_needed {
        Some((
            "fortress_observe",
            "Inspect complete captured jobs, buildings and inventory before choosing finite production amounts; acquire a fresh paused observation with the same workforce selection.",
            false,
            json!({"request":production_observation_request(view).to_string()}),
        ))
    } else if preplan_diagnosis_needed {
        Some((
            "fortress_query",
            "Inspect observed production conditions before compiling the original plan. This captured source does not prove native recipe feasibility or reserve materials.",
            false,
            json!({"request":inspection_source["diagnosis_request"].to_string()}),
        ))
    } else if view["original_plan"].is_null()
        && !workshop_fenced
        && mode == Some("control")
        && view["production_control_enabled"] == true
        && view["observation"]["workforce_choices"]["existing_labor_options"]
            .as_array()
            .is_some_and(|v| !v.is_empty())
    {
        Some((
            "fortress_plan",
            "Compile kind=stock_targets to calculate up to four distinct furniture deficits from one qualified census while retaining every requested minimum, kind=stock_target for one target, kind=existing_labor for an explicit amount, or kind=existing_labor_portfolio for up to four recipes using observed eligible workers whose selected labor is already enabled. Each order retains the original deadline and rechecks current labor without assigning it.",
            false,
            json!({}),
        ))
    } else if view["original_plan"].is_null()
        && !workshop_fenced
        && mode == Some("control")
        && view["labor_control_enabled"] == true
        && view["production_control_enabled"] == true
        && view["observation"]["workforce_choices"]["options"]
            .as_array()
            .is_some_and(|v| !v.is_empty())
    {
        Some((
            "fortress_plan",
            "Compile kind=stock_targets with assign_labor=true to size up to four finite inventory deficits and review enabling the selected workers before their orders, kind=stock_target for one deficit, or supply explicit amounts with kind=portfolio. The assignment and every order require separate review and confirmation; all requested stock minima remain in the original goal.",
            false,
            json!({}),
        ))
    } else {
        None
    };
    let mut recommendations = recommendation
        .into_iter()
        .map(|(tool, reason, confirm, mut arguments)| {
            if let Some(id) = session {
                arguments["session_id"] = json!(id.to_string());
            }
            if portfolio && matches!(arguments["scope"].as_str(), Some("run" | "inventory_run")) {
                arguments["order_step_id"] = view["portfolio"]["selected_order_step_id"].clone();
            }
            recommendation_fn(tool, reason, confirm, arguments)
        })
        .collect::<Vec<_>>();
    if (preplan_capture_needed || preplan_diagnosis_needed)
        && let Some(next) = recommendations.first_mut()
        && matches!(
            next["tool"].as_str(),
            Some("fortress_observe" | "fortress_query")
        )
    {
        next["recommendation_id"] = json!(if preplan_capture_needed {
            "inspect-production-before-planning"
        } else {
            "diagnose-captured-production-before-planning"
        });
        next["expected_utility"] =
            json!("Choose finite amounts and inventory objectives using observed operations.");
        next["expected_information_value"] = json!(
            "Identify existing stock and observed job conditions before committing resources."
        );
        next["reversibility"] = json!("not_applicable");
        next["estimated_cost"]["game_ticks"] = json!(0);
        if preplan_diagnosis_needed {
            next["estimated_cost"]["bridge_bytes"] = json!(0);
        }
        next["prerequisites"] = json!(["current Query and Observe authority", "no original plan"]);
        next["invalidating_conditions"] = json!([
            "original plan created",
            "canonical observation changed",
            "source withdrawn",
            "read authority revoked"
        ]);
        let mut evidence = json!({"canonical_anchor":view["anchor"],
            "observation_seal":view["observation"]["seal"]});
        if preplan_inspection_available {
            evidence["source_anchor"] = inspection_source["anchor"].clone();
            evidence["source_digest"] = inspection_source["source_digest"].clone();
        }
        next["confidence"]["evidence"] = json!([evidence]);
    }
    for owner in CLOCK_OWNERS {
        if clock_cancel(owner)
            && result["closed"] != true
            && let Some(id) = session
            && !recommendations.iter().any(|item| {
                item["tool"] == "fortress_cancel" && item["arguments"]["scope"] == owner.scope
            })
        {
            let mut arguments = json!({"session_id":id.to_string(),"scope":owner.scope});
            if portfolio {
                arguments["order_step_id"] = view["portfolio"]["selected_order_step_id"].clone();
            }
            recommendations.push(recommendation_fn(
                "fortress_cancel",
                "Cancel the retained clock run; an active run requests a stop and keeps its original recovery custody.",
                false,
                arguments,
            ));
        }
    }
    // This suggestion requests a new capture; it never joins pages into a
    // canonical snapshot or carries mutation/preparation authority.
    if verified
        && !uncertain
        && mode != Some("offline")
        && view["original_plan"].is_null()
        && let Some(id) = session
        && view["observation"]["workforce_choices"]["next_observation_request"].is_object()
    {
        recommendations.push(recommendation_fn(
            "fortress_observe",
            "Inspect the next bounded labor choices for these same citizens with a fresh observation; use its new seal for planning.",
            false,
            json!({"session_id":id.to_string(),
                "request":view["observation"]["workforce_choices"]["next_observation_request"].to_string()}),
        ));
    }
    if verified
        && !uncertain
        && mode != Some("offline")
        && view["original_plan"].is_null()
        && let Some(id) = session
        && view["observation"]["workforce_choices"]["next_existing_labor_observation_request"]
            .is_object()
    {
        recommendations.push(recommendation_fn(
            "fortress_observe",
            "Inspect the next bounded existing-labor options for these same citizens with a fresh observation; use its new seal for planning.",
            false,
            json!({"session_id":id.to_string(),
                "request":view["observation"]["workforce_choices"]["next_existing_labor_observation_request"].to_string()}),
        ));
    }
    let mut partial_domains = vec!["retained_original_plan"];
    if workshop.is_object() {
        partial_domains.push("original_workshop_prerequisite_custody");
    }
    if stock_targets.is_object() {
        partial_domains.push("original_stock_target_portfolio");
    }
    if preplan_inspection_available {
        partial_domains.push("preplan_captured_operations");
    }
    if diagnosis_seen {
        partial_domains.push("observed_production_conditions");
    }
    if selected_goal.is_object() {
        partial_domains.push(if retained_order {
            "selected_original_order_completion_goal"
        } else {
            "selected_original_inventory_goal"
        });
    }
    let mut omitted_domains = vec![
        "physical_output",
        "production_attribution",
        "other_controllers",
    ];
    if workshop.is_object() {
        omitted_domains.extend([
            "current_workshop_completion",
            "workshop_production_feasibility",
        ]);
    }
    if parent_goal.is_object() {
        partial_domains.push("current_parent_terminal_predicate");
    } else {
        omitted_domains.insert(0, "current_parent_goal");
    }
    for owner in CLOCK_OWNERS {
        if owner.view(view).is_object() {
            partial_domains.push(owner.coverage_domain);
        }
    }
    if CLOCK_OWNERS
        .iter()
        .any(|owner| owner.view(view).is_object())
    {
        omitted_domains.push("current_pause");
    }
    let mut uncertainty = if parent_fenced {
        vec![json!({
            "scope":parent_scope,
            "epistemic_status":"unknown", "state":"observation_fenced",
            "retryable_in_session":false,
            "finding":"Current parent observation is fenced after a source-lineage failure.",
            "resolution":"Retain the original recovery reference and reconcile any uncertain effect under its original key. Inspect custody with fortress_doctor and resolve the source lineage before reopening; further goal waits in this session cannot clear this fence."
        })]
    } else {
        match parent_truth {
            Some("false") => vec![json!({
                "scope":parent_scope,
                "epistemic_status":"contradicted", "anchor":parent_goal["anchor"],
                "finding":"The complete original terminal predicate is false at this sample anchor.",
                "resolution":"Review the current parent predicate and original plan before choosing any corrective action; selected-goal polling alone does not establish labor truth.",
                "selected_inventory_goal_proven_at_anchor":selected_proven && !retained_order,
                "selected_order_goal_proven_at_anchor":selected_proven && retained_order
            })],
            Some("unknown") => vec![json!({
                "scope":parent_scope,
                "epistemic_status":"unknown", "anchor":parent_goal["anchor"],
                "finding":"Current evidence does not establish the complete original terminal predicate.",
                "resolution":"Review source coverage and current authority before another bounded sample.",
                "selected_inventory_goal_proven_at_anchor":selected_proven && !retained_order,
                "selected_order_goal_proven_at_anchor":selected_proven && retained_order
            })],
            None if selected_goal.is_object() => vec![json!({
                "scope":parent_scope,
                "epistemic_status":"unknown",
                "finding":"No current joined labor and production-goal sample is published.",
                "resolution":"A fresh coherent parent sample is required; selected-goal history and historical labor receipts are insufficient."
            })],
            _ => Vec::new(),
        }
    };
    if preplan_inspection_available {
        uncertainty.push(json!({"scope":"preplan_captured_operations","epistemic_status":"unknown",
            "anchor":inspection_source["anchor"],"source_digest":inspection_source["source_digest"],
            "finding":"Captured stock and job conditions do not establish native recipe feasibility, path access, reservations or goal completion.",
            "resolution":"Inspect the source-bound inventory query and declared-demand allocation before choosing the original request; planning and commit independently revalidate their required sources."}));
    }
    if workshop_fenced {
        uncertainty.push(json!({"scope":"original_workshop_dispatch_prerequisite",
            "epistemic_status":"unknown","state":"source_fenced","retryable_in_session":false,
            "origin_witness":workshop["origin_witness"],"association_digest":workshop["association_digest"],
            "consumed_tick_floor":workshop["consumed_tick_floor"],
            "finding":"Source continuity for the original workshop prerequisite is fenced; earlier completion is historical only.",
            "resolution":"Preserve the original association and consumed recovery floors. Further order dispatch is withheld; inspect source continuity and reconcile any already attempted effect under its original key."}));
    }
    if stock_target_failed {
        uncertainty.push(json!({"scope":if stock_targets.is_object() {
                "original_stock_targets_dispatch_basis"
            } else {"original_stock_target_dispatch_basis"},
            "epistemic_status":"unknown","state":"revalidation_failed",
            "assessment_witness":if stock_targets.is_object() {
                &stock_targets["assessment_witness"]
            } else {&stock_target["assessment_witness"]},
            "finding":"The original inventory basis or exact queue requirement was not re-established. Sealed amounts and target minima have not been changed.",
            "resolution":"Retain the exact original recovery reference and reconcile any attempted effect. Inspect the failed source or authority check; changing the target or amount requires a new original, never a replacement key for uncertain work."}));
    }
    for owner in CLOCK_OWNERS {
        let clock = owner.view(view);
        let reconciliation = clock_reconciliation(clock);
        if reconciliation || owner.invalidates_goal(view) {
            uncertainty.push(json!({"scope":owner.scope,"epistemic_status":"unknown",
                "clock_may_be_running":clock_unresolved(clock),"current_pause_proven":false,
                "finding":"Native clock custody cannot establish current pause or completion of the original goal.",
                "resolution":if clock["active_run"]["native"]["phase"] == "source_lost"
                    && clock["needs_reconciliation"] == false {
                    "Inspect custody and resolve the original clock source; repeated polling cannot prove pause after source loss."
                } else if reconciliation {
                    "Reconcile or cancel the original run before a fresh joined labor and inventory sample."
                } else {
                    "Take a fresh joined labor and inventory sample after the run has settled."
                }}));
        }
    }
    let clock_views = CLOCK_OWNERS.map(|owner| {
        clock_summary(
            owner.view(view),
            clock_commit && owner == clock_owner,
            owner,
        )
    });
    let mut actions = entries;
    let mut cancellation_drains = Vec::new();
    let mut affordances = Vec::new();
    for (owner, summary) in CLOCK_OWNERS.into_iter().zip(&clock_views) {
        let clock = owner.view(view);
        let active = &clock["active_run"];
        if active.is_object() {
            actions.push(
                json!({"scope":owner.scope,"clock_run":summary["active_run"],
                "currently_verified":verified,"clock_receipt_is_goal_proof":false}),
            );
        }
        if active["state"] == "cancel_requested" {
            cancellation_drains.push(json!({"scope":owner.scope,
                "key":bounded(&active["key"],128),"currently_verified":verified}));
        }
        if clock.is_object() {
            let proposal = if owner == clock_owner {
                clock_proposal.is_some()
            } else if owner == ORDER_CLOCK {
                order_proposal.is_some()
            } else {
                inventory_proposal.is_some()
            };
            affordances.extend([
                json!({"action":"fortress_plan","scope":owner.scope,"enabled":proposal,
                    "request_kind":owner.scope,"grants_authority":false}),
                json!({"action":"fortress_commit","scope":owner.scope,
                    "enabled":clock_commit && owner == clock_owner,
                    "requires_confirmation":true,"grants_authority":false}),
                json!({"action":"fortress_cancel","scope":owner.scope,"enabled":clock_cancel(owner),
                    "grants_authority":false}),
            ]);
        }
    }
    let mut confirmations = if review.is_null() {
        vec![]
    } else {
        vec![review]
    };
    let pending_plans = if clock_commit {
        let confirmation = json!({"scope":clock_owner.scope,"paired_seal":clock_active["review_seal"],
            "game_ticks":clock_active["game_ticks"],"wall_millis":clock_active["wall_millis"],
            "requires_confirmation":true,"commit_available":true,
            "purpose":"bounded game-time progress for the original inventory goal"});
        confirmations.push(confirmation.clone());
        vec![confirmation]
    } else {
        vec![]
    };
    if !inspection_available {
        if result.get("production_inspection_source").is_some() {
            result["production_inspection_source"] = Value::Null;
        }
        if result.get("production_inspection").is_some() {
            result["production_inspection"] = Value::Null;
        }
    }
    if result.get("workshop_prerequisite").is_some() {
        result["workshop_prerequisite"] = workshop.clone();
    }
    if result.get("stock_targets").is_some() {
        result["stock_targets"] = stock_targets.clone();
    }
    if result["retained_reference"]["stock_targets"].is_object() {
        result["retained_reference"]["stock_targets"] =
            stock_targets::summary(&result["retained_reference"]["stock_targets"]);
    }
    if !view["stock_targets"].is_null()
        || view["current_parent_goal"]["inventory_requirements"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty())
    {
        // A partial cohort or missing target cannot survive in top-level
        // fields after the Agent Turn has correctly withheld its proof.
        result["original_goal_proven"] = json!(parent_proven);
        result["current_state_proven"] = json!(false);
        if result.get("current_parent_goal").is_some() {
            result["current_parent_goal"] = parent_goal.clone();
        }
        if result.get("selected_parent_goal").is_some() {
            result["selected_parent_goal"] = joined_goal.clone();
        }
        if result["portfolio"].is_object() {
            result["portfolio"]["complete_original_goal_proven"] = json!(parent_proven);
        }
    }
    if result.get("stock_target").is_some() {
        result["stock_target"] = stock_target.clone();
    }
    if result["retained_reference"]["stock_target"].is_object() {
        result["retained_reference"]["stock_target"] =
            stock_target_summary(&result["retained_reference"]["stock_target"]);
    }
    if result["retained_reference"]["workshop_prerequisite"].is_object() {
        result["retained_reference"]["workshop_prerequisite"] = workshop_summary(
            &result["retained_reference"]["workshop_prerequisite"],
            false,
        );
    }
    if workshop_blocks_review {
        if result.get("review").is_some() {
            result["review"] = Value::Null;
        }
        if result["frontier"].is_object() {
            result["frontier"]["review"] = Value::Null;
        }
    }
    if clock_invalidates_goal {
        result["original_goal_proven"] = json!(false);
        result["current_state_proven"] = json!(false);
        if result.get("current_parent_goal").is_some() {
            result["current_parent_goal"] = Value::Null;
        }
        if result.get("selected_inventory_goal").is_some() {
            result["selected_inventory_goal"] = compact_goal(&result["selected_inventory_goal"]);
        }
        if result.get("selected_order_goal").is_some() {
            result["selected_order_goal"] = compact_goal(&result["selected_order_goal"]);
        }
    }
    if let Some(labor) = &labor
        && let Some(finding) = uncertainty.first_mut()
    {
        finding["labor_blockers"] = json!(labor.blockers);
        finding["labor_blockers_omitted"] = json!(labor.omitted);
        if !labor.blockers.is_empty() {
            finding["resolution"] = json!(
                "Review the listed mismatched or unobserved labor settings against the original plan. Any corrective action requires a fresh observation and newly authorized plan. Preserve completed original keys for reconciliation only."
            );
        }
    }
    let mut turn = AgentTurnBuilder::new(operation, phase)
        .continuity(if uncertain { ContinuityStatus::Indeterminate } else if verified { ContinuityStatus::Partial } else if retained.is_some() { ContinuityStatus::Stale } else { ContinuityStatus::Bootstrap }, None, None, None)
        .briefing(json!({"runtime":"unadmitted_combined_production_development", "runtime_admitted":false,"mode":mode,
            "retained_state_currently_verified":verified,"current_state_proven":false,"original_goal_proven":parent_proven,
            "current_parent_goal_truth":parent_truth,
            "current_parent_labor":labor.as_ref().map(|labor| &labor.summary),
            "current_parent_inventory":parent_goal["inventory_requirements"],
            "current_parent_stability":parent_goal["parent_stability"],
            "production_inspection_available":inspection_available,
            "production_inspection_scope":if inspection_available {inspection_source["scope"].as_str()} else {None},
            "preplan_production_inspection_available":preplan_inspection_available,
            "observed_production_conditions":if diagnosis_seen {Some(&inspection["summary"])} else {None},
            "parent_goal_observation_fenced":parent_fenced,
            "stock_target":stock_target,
            "stock_targets":stock_targets,
            "workshop_prerequisite":if workshop.is_object() {Some(json!({
                "scope":"original_workshop_dispatch_prerequisite","origin_witness":workshop["origin_witness"],
                "association_digest":workshop["association_digest"],"source_fenced":workshop["source_fenced"],
                "consumed_tick_floor":workshop["consumed_tick_floor"],"current_completion_proven":false,
                "canonical_workshop_evidence":false,"production_feasibility_proven":false}))}else{None},
            "clock_control_enabled":clock_authority(ORDER_CLOCK),
            "inventory_clock_control_enabled":clock_authority(INVENTORY_CLOCK),
            "clock_may_be_running":clock_unresolved(&view["clock"]),
            "inventory_clock_may_be_running":clock_unresolved(&view["inventory_clock"]),
            "clock_invalidates_goal":ORDER_CLOCK.invalidates_goal(view),
            "inventory_clock_invalidates_goal":INVENTORY_CLOCK.invalidates_goal(view),"current_pause_proven":false,
            "clock_receipt_is_goal_proof":false,
            "selected_inventory_goal_proven_at_anchor":selected_proven && !retained_order,
            "selected_order_goal_proven_at_anchor":selected_proven && retained_order}))
        .active_work(json!({"pending_plans":pending_plans,"actions":actions,"clock":clock_views[0],
            "inventory_clock":clock_views[1],
            "workshop_prerequisite":workshop,
            "stock_target":stock_target,
            "stock_targets":stock_targets,
            "workshop_prerequisite_floor":workshop_floor(&view["recovery"]["workshop_prerequisite_floor"]),
            "production_inspection_source":if inspection_available {Some(inspection_source)} else {None},
            "obligations":if selected_goal.is_null() {vec![]} else {vec![selected_goal]},
            "current_parent_goal":parent_goal,"cancellation_drains":cancellation_drains,
            "indeterminate_effects":if uncertain {vec![json!({"state":"unknown","reconcile_original_key":true,
                "clock_may_be_running":clock_unresolved(&view["clock"]),
                "inventory_clock_may_be_running":clock_unresolved(&view["inventory_clock"])})]} else {vec![]},
            "publications":[],"confirmations":confirmations}))
        .affordances(affordances)
        .recommendations(recommendations)
        .uncertainty(uncertainty)
        .coverage(json!({"status":"partial","complete_domains":[],"partial_domains":partial_domains,
            "omitted_domains":omitted_domains,"continuation":null}));
    if let Some(id) = session {
        turn = turn.session_id(id.to_string());
    }
    if let Some(id) = request {
        turn = turn.request_id(id.to_string());
    }
    if let Some(a) = anchor {
        turn = turn.anchor(anchor_value(a));
    }
    if let Some(b) = budget {
        turn = turn.budget(json!({"requested":null,"admitted":{"max_wall_millis":b.max_wall_millis,
        "max_bytes":b.max_bytes,"max_output_tokens":b.max_output_tokens,"max_game_ticks":b.max_game_ticks},
        "consumed":{},"remaining":null,"soft_stop_reason":null,"hard_stop_reason":null}));
    }
    let mut agent_turn = turn.build();
    if let Some(briefing) = agent_turn["briefing"].as_object_mut() {
        briefing.remove("admission");
    }
    if let Some(object) = result.as_object_mut() {
        object.insert("agent_turn".into(), agent_turn);
    }
    result.to_string()
}
fn recommendation_fn(tool: &str, reason: &str, confirm: bool, arguments: Value) -> Value {
    let clock_control = matches!(arguments["scope"].as_str(), Some("run" | "inventory_run"))
        || arguments["request"].as_str().is_some_and(|raw| {
            serde_json::from_str::<Value>(raw).is_ok_and(|request| {
                matches!(request["kind"].as_str(), Some("run" | "inventory_run"))
            })
        });
    recommendation(
        tool,
        tool,
        reason,
        "advance the retained original plan",
        "resolve the next bounded step",
        if confirm || clock_control {
            "guarded"
        } else {
            "read_only"
        },
        "not_assessed",
        confirm,
        arguments,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn packet(
    operation: &str,
    session: Option<SessionId>,
    anchor: Option<StateAnchor>,
    mode: Option<&str>,
    request: Option<RequestId>,
    budget: Option<WorkBudget>,
    result: Value,
    retained: Option<&Value>,
    verified: bool,
    uncertain: bool,
) -> (String, bool) {
    let closed = result["closed"] == true;
    let full = encode(
        operation, session, anchor, mode, request, budget, result, retained, verified, uncertain,
    );
    if serde_json::to_string(&full).is_ok_and(|s| s.len() <= TEXT_LIMIT) {
        return (full, false);
    }
    let fallback = json!({"ok":false,"closed":closed,"error":{"code":"budget_exceeded","message":"complete response exceeds output limit; local review withdrawn","retryable":false},
        "output_limit_bytes":TEXT_LIMIT,"retained_reference":retained.map(compact),"retained_state_currently_verified":false,
        "automatic_redispatch":false,"current_state_proven":false,"original_goal_proven":false});
    let compacted = retained.map(compact);
    (
        encode(
            operation,
            session,
            anchor,
            mode,
            request,
            budget,
            fallback,
            compacted.as_ref(),
            false,
            uncertain,
        ),
        true,
    )
}

#[cfg(test)]
#[path = "live_production_presentation_clock_tests.rs"]
mod clock_tests;

#[cfg(test)]
#[path = "live_production_presentation_inventory_clock_tests.rs"]
mod inventory_clock_tests;

#[cfg(test)]
#[path = "live_production_presentation_preplan_tests.rs"]
mod preplan_tests;

#[cfg(test)]
#[path = "live_production_presentation_stock_target_tests.rs"]
mod stock_target_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use dfmcp_core::{Digest32, FortressId, GameTick, ObservationCursor};

    fn workshop_view(anchor: StateAnchor, fenced: bool) -> Value {
        json!({"schema":"dfmcp.completed-workshop-prerequisite/1","source_protocol":"workshop/1.20",
            "scope":"original_workshop_dispatch_prerequisite","origin_witness":"5".repeat(64),
            "workshop_key":"original-carpenters","workshop_plan_digest":"6".repeat(64),
            "workshop_receipt":"7".repeat(64),"building_id":42,"source_generation":30,
            "association_digest":"8".repeat(64),"consumed_tick_floor":anchor.tick.get(),
            "checkpoint":{"origin_witness":"5".repeat(64),"tick":anchor.tick.get(),
                "generation":30,"next_building_id":43,"fenced":fenced},
            "source_fenced":fenced,"last_qualified":{"tick":anchor.tick.get(),
                "witness":"9".repeat(64),"canonical_parent":anchor_value(anchor),"status":"complete",
                "fresh_for_current_call":false},"current_state_proven":false,
            "canonical_workshop_evidence":false,"production_feasibility_proven":false,"mutation_authority":false})
    }

    #[test]
    fn workshop_fence_withholds_only_new_order_dispatch_and_keeps_original_progress_guidance() {
        let (anchor, goal_view) = joined_order_goal_view("false", false);
        let call = |view: &Value, anchor, uncertain, ticks| {
            let (raw, overflow) = packet(
                "fortress.query",
                Some(SessionId::new(1)),
                anchor,
                Some("control"),
                None,
                Some(WorkBudget {
                    max_game_ticks: ticks,
                    max_wall_millis: 60000,
                    ..WorkBudget::default()
                }),
                view.clone(),
                Some(view),
                true,
                uncertain,
            );
            assert!(!overflow);
            serde_json::from_str::<Value>(&raw).expect("workshop packet")
        };
        let mut view = json!({"anchor":anchor_value(anchor),"original_plan":{"plan_digest":"a".repeat(64),
            "steps":[{"step_id":0,"action":{"kind":"set_labor"}},
                {"step_id":1,"action":{"kind":"create_work_order"}}]},
            "frontier":{"next_ready":1,"all_applied":false,"needs_reconciliation":false,"entries":[]},
            "workshop_prerequisite":workshop_view(anchor,false)});
        assert_eq!(
            call(&view, Some(anchor), false, 0)["agent_turn"]["recommendations"][0]["tool"],
            "fortress_plan"
        );
        view["workshop_prerequisite"] = workshop_view(anchor, true);
        let fenced = call(&view, Some(anchor), false, 0);
        assert_eq!(fenced["agent_turn"]["recommendations"], json!([]));
        assert_eq!(
            fenced["agent_turn"]["briefing"]["workshop_prerequisite"]["source_fenced"],
            true
        );
        assert!(
            fenced["agent_turn"]["uncertainty"]
                .as_array()
                .expect("uncertainty")
                .iter()
                .any(
                    |item| item["scope"] == "original_workshop_dispatch_prerequisite"
                        && item["state"] == "source_fenced"
                )
        );
        view["frontier"]["next_ready"] = json!(0);
        assert_eq!(
            call(&view, Some(anchor), false, 0)["agent_turn"]["recommendations"][0]["tool"],
            "fortress_plan"
        );
        view["frontier"]["next_ready"] = Value::Null;
        view["review"] =
            json!({"step_id":1,"family":"work-orders/1.10","paired_seal":"b".repeat(64)});
        let blocked = call(&view, Some(anchor), false, 0);
        assert_eq!(blocked["agent_turn"]["recommendations"], json!([]));
        assert_eq!(
            blocked["agent_turn"]["active_work"]["confirmations"],
            json!([])
        );
        assert!(blocked["review"].is_null());
        view["review"] =
            json!({"step_id":0,"family":"workforce/1.17","paired_seal":"b".repeat(64)});
        assert_eq!(
            call(&view, Some(anchor), false, 0)["agent_turn"]["recommendations"][0]["tool"],
            "fortress_commit"
        );
        view["frontier"]["needs_reconciliation"] = json!(true);
        assert_eq!(
            call(&view, Some(anchor), true, 0)["agent_turn"]["recommendations"][0]["tool"],
            "fortress_wait"
        );

        let unplanned = json!({"observation":{"seal":"a".repeat(64),"workforce_choices":{
            "existing_labor_options":[{"labor":"CARPENTER","eligible_enabled_ids":["43"]}]}},
            "production_control_enabled":true,"workshop_prerequisite":workshop_view(anchor,true)});
        assert_eq!(
            call(&unplanned, None, false, 0)["agent_turn"]["recommendations"],
            json!([])
        );
        let mut unplanned_ready = unplanned;
        unplanned_ready["workshop_prerequisite"] = workshop_view(anchor, false);
        assert_eq!(
            call(&unplanned_ready, None, false, 0)["agent_turn"]["recommendations"][0]["tool"],
            "fortress_plan"
        );

        let mut settled = goal_view;
        settled["workshop_prerequisite"] = workshop_view(anchor, true);
        assert_eq!(
            call(&settled, Some(anchor), false, 0)["agent_turn"]["recommendations"][0]["tool"],
            "fortress_wait"
        );
        settled["selected_order_goal"]["current_truth"] = json!("false");
        settled["selected_order_goal"]["receipt_linked"] = json!(true);
        settled["original_plan"]["expires_at_tick"] = json!(300);
        settled["original_plan"]["steps"] = json!([
            {"step_id":0,"action":{"kind":"set_labor"}},
            {"step_id":1,"action":{"kind":"create_work_order"},"obligation":{
                "deadline_tick":300,"poll_interval_ticks":5,"stable_samples":2}}]);
        settled["clock_control_enabled"] = json!(true);
        settled["recovery"]["clock_journal_id"] = json!("1".repeat(64));
        settled["clock"] = json!({"source":"order-run/1.14","target_scope":"selected_original_order_step",
            "journal_id":"1".repeat(64),"target_digest":"2".repeat(64),"original_paired_seal":"3".repeat(64),
            "mode":"control","pending":false,"unresolved":false,"may_be_running":false,
            "needs_reconciliation":false,"active_run":null,"run_count":0,"high_tick":anchor.tick.get()});
        let clock = call(&settled, Some(anchor), false, 10);
        assert_eq!(
            clock["agent_turn"]["recommendations"][0]["tool"],
            "fortress_plan"
        );
        let request: Value = serde_json::from_str(
            clock["agent_turn"]["recommendations"][0]["arguments"]["request"]
                .as_str()
                .expect("clock request"),
        )
        .expect("clock JSON");
        assert_eq!(request["kind"], "run");
        assert_eq!(
            clock["agent_turn"]["briefing"]["workshop_prerequisite"]["current_completion_proven"],
            false
        );
    }

    #[test]
    fn workshop_error_and_overflow_keep_original_custody_and_floors_without_current_proof() {
        let (anchor, mut view) = joined_order_goal_view("true", true);
        view["workshop_prerequisite"] = workshop_view(anchor, false);
        let floor = json!({"tick":130,"generation":30,"next_building_id":43,"fenced":false});
        view["recovery"]["workshop_prerequisite_digest"] = json!("8".repeat(64));
        view["recovery"]["workshop_prerequisite_floor"] = floor.clone();
        let checkpoint = view["workshop_prerequisite"]["checkpoint"].clone();
        let successful = joined_packet(anchor, &view, "control", true, false);
        let historical = &successful["agent_turn"]["active_work"]["workshop_prerequisite"];
        assert_eq!(
            historical["last_qualified"]["fresh_for_current_call"],
            false
        );
        assert_eq!(historical["current_completion_proven"], false);
        assert_eq!(historical["canonical_workshop_evidence"], false);
        assert_eq!(historical["production_feasibility_proven"], false);
        for retained in [compact(&view), compact(&compact(&view))] {
            assert_eq!(retained["recovery"]["workshop_prerequisite_floor"], floor);
            assert_eq!(
                retained["recovery"]["workshop_prerequisite_digest"],
                "8".repeat(64)
            );
            assert_eq!(
                retained["workshop_prerequisite"]["workshop_key"],
                "original-carpenters"
            );
            assert_eq!(
                retained["workshop_prerequisite"]["workshop_receipt"],
                "7".repeat(64)
            );
            assert!(retained["workshop_prerequisite"]["last_qualified"].is_null());
            assert_eq!(retained["workshop_prerequisite"]["checkpoint"], checkpoint);
        }
        let failed = joined_packet(anchor, &view, "control", false, true);
        assert_eq!(
            failed["agent_turn"]["active_work"]["workshop_prerequisite"]["checkpoint"],
            checkpoint
        );
        assert!(
            failed["agent_turn"]["active_work"]["workshop_prerequisite"]["last_qualified"]
                .is_null()
        );
        assert_eq!(
            failed["agent_turn"]["active_work"]["workshop_prerequisite_floor"],
            floor
        );
        let (raw, overflow) = packet(
            "fortress.query",
            Some(SessionId::new(1)),
            Some(anchor),
            Some("control"),
            None,
            None,
            json!({"ok":true,"payload":"x".repeat(70_000)}),
            Some(&view),
            true,
            false,
        );
        assert!(overflow);
        let fallback: Value = serde_json::from_str(&raw).expect("overflow packet");
        assert_eq!(
            fallback["retained_reference"]["workshop_prerequisite"]["checkpoint"],
            checkpoint
        );
        assert_eq!(
            fallback["retained_reference"]["recovery"]["workshop_prerequisite_floor"],
            floor
        );
        assert_eq!(
            fallback["retained_reference"]["workshop_prerequisite"]["association_digest"],
            "8".repeat(64)
        );
        assert!(
            fallback["retained_reference"]["workshop_prerequisite"]["last_qualified"].is_null()
        );
        assert!(
            fallback["agent_turn"]["active_work"]["workshop_prerequisite"]["last_qualified"]
                .is_null()
        );
        assert_eq!(
            fallback["agent_turn"]["briefing"]["original_goal_proven"],
            false
        );
        let legacy = compact(&json!({"recovery":{"plan_digest":"a".repeat(64)}}));
        assert!(
            legacy["recovery"]
                .get("workshop_prerequisite_digest")
                .is_none()
        );
        assert!(
            legacy["recovery"]
                .get("workshop_prerequisite_floor")
                .is_none()
        );
        let mut preplan = view;
        preplan["original_plan"] = Value::Null;
        preplan["recovery"] = Value::Null;
        preplan["workshop_prerequisite"]["association_digest"] = Value::Null;
        preplan["workshop_prerequisite"]["source_fenced"] = json!(true);
        preplan["workshop_prerequisite"]["checkpoint"]["tick"] = json!(131);
        preplan["workshop_prerequisite"]["checkpoint"]["generation"] = json!(31);
        preplan["workshop_prerequisite"]["checkpoint"]["fenced"] = json!(true);
        let preplan_retained = compact(&preplan);
        assert_eq!(
            preplan_retained["workshop_prerequisite"]["checkpoint"],
            preplan["workshop_prerequisite"]["checkpoint"]
        );
        assert!(preplan_retained["workshop_prerequisite"]["last_qualified"].is_null());
        let preplan_failed = joined_packet(anchor, &preplan, "control", false, false);
        assert_eq!(
            preplan_failed["agent_turn"]["active_work"]["workshop_prerequisite"]["checkpoint"],
            preplan["workshop_prerequisite"]["checkpoint"]
        );
    }

    fn joined_goal_view(truth: &str, stable: bool) -> (StateAnchor, Value) {
        let anchor = StateAnchor {
            fortress_id: FortressId::new(7),
            tick: GameTick(130),
            cursor: ObservationCursor {
                epoch: 1,
                sequence: 5,
            },
            state_hash: Digest32::of_bytes(b"joined parent"),
        };
        let selected_anchor = StateAnchor {
            state_hash: Digest32::of_bytes(b"inventory child"),
            ..anchor
        };
        let proven = truth == "true" && stable;
        let view = json!({"anchor":anchor_value(anchor),
            "original_plan":{"plan_digest":"a".repeat(64)},
            "frontier":{"all_applied":true,"next_ready":null,"entries":[]},
            "review":null,"original_goal_proven":proven,
            "selected_inventory_goal":{"scope":"selected_original_order_step",
                "step_id":1,"kind":"observed_inventory_minimum","item_type":"BED",
                "minimum_stack_units":1,"current_truth":"true","proven_at_anchor":stable,
                "anchor":anchor_value(selected_anchor),
                "parent_terminal_truth":"unknown","original_goal_proven":false,
                "history":{"present":true,"fenced":false},"terminal_failure":false,
                "first_satisfied_anchor":if stable {anchor_value(selected_anchor)} else {Value::Null}},
            "current_parent_goal":{"scope":"complete_original_labor_and_inventory_goal",
                "anchor":anchor_value(anchor),"selected_anchor":anchor_value(selected_anchor),
                "current_truth":truth,"original_goal_proven":proven,
                "labor":[{"unit_id":"43","labor":"MINE","requested_enabled":true,
                    "observed_enabled":if truth == "unknown" {Value::Null} else {json!(truth == "true")},
                    "current_truth":truth}],
                "source_digest":"b".repeat(64),"workforce_source_digest":"c".repeat(64)}});
        (anchor, view)
    }

    fn joined_order_goal_view(truth: &str, stable: bool) -> (StateAnchor, Value) {
        let (anchor, mut view) = joined_goal_view(truth, stable);
        let mut goal = view["selected_inventory_goal"].take();
        goal["kind"] = json!("retained_order_completion");
        goal["order_entity_id"] = json!("12345");
        goal["required_order_state"] = json!({"amount_total":4,"amount_remaining":0,
            "validated":true,"active":false});
        goal["deadline_tick"] = json!(300);
        goal["poll_interval_ticks"] = json!(5);
        goal["stable_samples"] = json!(2);
        goal["disappearance_proves_completion"] = json!(false);
        if let Some(object) = goal.as_object_mut() {
            object.remove("item_type");
            object.remove("minimum_stack_units");
        }
        view["selected_order_goal"] = goal;
        view["current_parent_goal"]["scope"] = json!("complete_original_labor_and_order_goal");
        (anchor, view)
    }

    #[test]
    fn diagnosis_guidance_requires_the_current_selected_source() {
        let (anchor, mut view) = joined_order_goal_view("false", false);
        view["selected_order_goal"]["current_truth"] = json!("false");
        let selected = view["selected_order_goal"]["anchor"].clone();
        let source = json!({"scope":"captured_operations","attribution":"not_order_attributed",
            "anchor":selected,"parent_anchor":anchor_value(anchor),"order_step_id":1,
            "source_digest":"d".repeat(64),"diagnosis_request":{"schema":"dfmcp.query/1",
                "expected_anchor":selected,"query":{"kind":"production_diagnosis","limit":4}}});
        view["production_inspection_source"] = source.clone();
        let turn = joined_packet(anchor, &view, "control", true, false);
        assert_eq!(
            turn["agent_turn"]["briefing"]["production_inspection_available"],
            true
        );
        assert_eq!(
            turn["agent_turn"]["briefing"]["original_goal_proven"],
            false
        );
        assert_eq!(
            turn["agent_turn"]["recommendations"][0]["tool"],
            "fortress_query"
        );
        let request: Value = serde_json::from_str(
            turn["agent_turn"]["recommendations"][0]["arguments"]["request"]
                .as_str()
                .expect("query request"),
        )
        .expect("query envelope");
        assert_eq!(request["expected_anchor"], selected);

        view["production_inspection"] = json!({"kind":"production_diagnosis",
            "anchor":selected,"source_digest":"d".repeat(64),"summary":{"jobs_with_findings":1}});
        let diagnosed = joined_packet(anchor, &view, "control", true, false);
        assert_eq!(
            diagnosed["agent_turn"]["briefing"]["observed_production_conditions"]["jobs_with_findings"],
            1
        );
        assert_eq!(
            diagnosed["agent_turn"]["briefing"]["original_goal_proven"],
            false
        );
        assert_ne!(
            diagnosed["agent_turn"]["recommendations"][0]["tool"],
            "fortress_query"
        );

        for field in ["anchor", "parent_anchor", "order_step_id", "source_digest"] {
            view["production_inspection_source"] = source.clone();
            view["production_inspection_source"][field] = Value::Null;
            let stale = joined_packet(anchor, &view, "control", true, false);
            assert_eq!(
                stale["agent_turn"]["briefing"]["production_inspection_available"],
                false
            );
            assert!(stale["agent_turn"]["active_work"]["production_inspection_source"].is_null());
        }
        view["production_inspection_source"] = source;
        for (mode, verified, uncertain) in [
            ("offline", true, false),
            ("control", false, false),
            ("control", true, true),
        ] {
            let refused = joined_packet(anchor, &view, mode, verified, uncertain);
            assert_eq!(
                refused["agent_turn"]["briefing"]["production_inspection_available"],
                false
            );
            assert!(refused["agent_turn"]["briefing"]["observed_production_conditions"].is_null());
        }
    }

    #[test]
    fn retained_order_goal_guides_wait_and_reports_its_own_parent_proof() {
        for stable in [false, true] {
            let (anchor, view) = joined_order_goal_view("true", stable);
            let value = joined_packet(anchor, &view, "control", true, false);
            let turn = &value["agent_turn"];
            assert_eq!(turn["briefing"]["original_goal_proven"], stable);
            assert_eq!(
                turn["briefing"]["selected_order_goal_proven_at_anchor"],
                stable
            );
            assert_eq!(
                turn["briefing"]["selected_inventory_goal_proven_at_anchor"],
                false
            );
            assert_eq!(
                turn["active_work"]["obligations"][0],
                view["selected_order_goal"]
            );
            assert_eq!(
                turn["active_work"]["current_parent_goal"],
                view["current_parent_goal"]
            );
            let coverage = turn["coverage"]["partial_domains"]
                .as_array()
                .expect("coverage");
            assert!(coverage.contains(&json!("selected_original_order_completion_goal")));
            assert!(!coverage.contains(&json!("selected_original_inventory_goal")));
            let next = turn["recommendations"].as_array().expect("recommendations");
            if stable {
                assert!(next.is_empty());
            } else {
                assert_eq!(next.len(), 1);
                assert_eq!(next[0]["tool"], "fortress_wait");
                assert!(
                    next[0]["reason"]
                        .as_str()
                        .expect("reason")
                        .contains("retained-order")
                );
            }
        }
    }

    #[test]
    fn retained_order_success_cannot_replace_current_labor_or_another_objective() {
        for truth in ["false", "unknown"] {
            let (anchor, view) = joined_order_goal_view(truth, true);
            let value = joined_packet(anchor, &view, "control", true, false);
            assert_eq!(
                value["agent_turn"]["briefing"]["original_goal_proven"],
                false
            );
            assert_eq!(
                value["agent_turn"]["briefing"]["selected_order_goal_proven_at_anchor"],
                true
            );
            assert_eq!(
                value["agent_turn"]["uncertainty"][0]["scope"],
                "complete_original_labor_and_order_goal"
            );
        }
        let (anchor, original) = joined_order_goal_view("true", true);
        for mismatch in 0..4 {
            let mut view = original.clone();
            match mismatch {
                0 => {
                    view["current_parent_goal"]["scope"] =
                        json!("complete_original_labor_and_inventory_goal")
                }
                1 => {
                    view["selected_inventory_goal"] =
                        joined_goal_view("true", true).1["selected_inventory_goal"].clone()
                }
                2 => view["selected_order_goal"]["kind"] = json!("observed_inventory_minimum"),
                _ => view["current_parent_goal"]["selected_anchor"]["tick"] = json!(131),
            }
            let value = joined_packet(anchor, &view, "control", true, false);
            assert_eq!(
                value["agent_turn"]["briefing"]["original_goal_proven"],
                false
            );
            assert!(value["agent_turn"]["active_work"]["current_parent_goal"].is_null());
        }
    }

    #[test]
    fn retained_order_recovery_withdraws_current_proof_and_preserves_exact_goal_identity() {
        let (anchor, view) = joined_order_goal_view("true", true);
        let retained = compact(&view);
        let goal = &retained["selected_order_goal"];
        assert_eq!(goal["order_entity_id"], "12345");
        assert_eq!(
            goal["required_order_state"],
            view["selected_order_goal"]["required_order_state"]
        );
        assert_eq!(
            goal["first_satisfied_anchor"],
            view["selected_order_goal"]["first_satisfied_anchor"]
        );
        assert_eq!(goal["deadline_tick"], 300);
        assert_eq!(goal["stable_samples"], 2);
        assert_eq!(goal["proven_at_anchor"], false);
        assert!(goal["current_truth"].is_null());
        assert!(retained["current_parent_goal"].is_null());
        assert!(goal.get("item_type").is_none());
        for (mode, verified, uncertain) in [
            ("offline", true, false),
            ("control", false, false),
            ("control", true, true),
        ] {
            let value = joined_packet(anchor, &retained, mode, verified, uncertain);
            assert_eq!(
                value["agent_turn"]["briefing"]["original_goal_proven"],
                false
            );
            assert_eq!(
                value["agent_turn"]["briefing"]["selected_order_goal_proven_at_anchor"],
                false
            );
        }
    }

    fn joined_packet(
        anchor: StateAnchor,
        view: &Value,
        mode: &str,
        verified: bool,
        uncertain: bool,
    ) -> Value {
        let (raw, overflow) = packet(
            "fortress.wait",
            Some(SessionId::new((1u128 << 127) | (24u128 << 57) | 1)),
            Some(anchor),
            Some(mode),
            None,
            None,
            json!({"ok":verified}),
            Some(view),
            verified,
            uncertain,
        );
        assert!(!overflow);
        serde_json::from_str(&raw).expect("joined packet")
    }

    #[test]
    fn joined_parent_truth_requires_original_inventory_stability_and_preserves_both_anchors() {
        for stable in [false, true] {
            let (anchor, view) = joined_goal_view("true", stable);
            let value = joined_packet(anchor, &view, "control", true, false);
            let turn = &value["agent_turn"];
            assert_eq!(turn["briefing"]["original_goal_proven"], stable);
            assert_eq!(turn["briefing"]["current_parent_goal_truth"], "true");
            assert_eq!(turn["briefing"]["current_state_proven"], false);
            assert_eq!(turn["briefing"]["runtime_admitted"], false);
            assert_eq!(
                turn["briefing"]["current_parent_labor"],
                json!({"citizens":1,"matched":1,"mismatched":0,"unknown":0})
            );
            assert_eq!(turn["anchor"], view["current_parent_goal"]["anchor"]);
            assert_eq!(
                turn["active_work"]["current_parent_goal"],
                view["current_parent_goal"]
            );
            assert_eq!(
                turn["active_work"]["obligations"][0]["anchor"],
                view["selected_inventory_goal"]["anchor"]
            );
            assert_ne!(
                turn["anchor"]["state_hash"],
                turn["active_work"]["obligations"][0]["anchor"]["state_hash"]
            );
            assert_eq!(turn["coverage"]["complete_domains"], json!([]));
            assert!(
                turn["coverage"]["partial_domains"]
                    .as_array()
                    .expect("domains")
                    .contains(&json!("current_parent_terminal_predicate"))
            );
            if stable {
                assert_eq!(turn["recommendations"], json!([]));
            } else {
                assert_eq!(turn["recommendations"][0]["tool"], "fortress_wait");
            }
        }
    }

    #[test]
    fn satisfied_inventory_with_false_or_unknown_parent_stops_inventory_only_polling() {
        for (truth, epistemic) in [("false", "contradicted"), ("unknown", "unknown")] {
            let (anchor, view) = joined_goal_view(truth, true);
            let value = joined_packet(anchor, &view, "control", true, false);
            let turn = &value["agent_turn"];
            assert_eq!(
                turn["briefing"]["selected_inventory_goal_proven_at_anchor"],
                true
            );
            assert_eq!(turn["briefing"]["original_goal_proven"], false);
            assert_eq!(turn["briefing"]["current_parent_goal_truth"], truth);
            assert_eq!(
                turn["active_work"]["current_parent_goal"]["current_truth"],
                truth
            );
            assert_eq!(turn["recommendations"], json!([]));
            assert_eq!(turn["uncertainty"][0]["epistemic_status"], epistemic);
            assert_eq!(
                turn["uncertainty"][0]["anchor"],
                view["current_parent_goal"]["anchor"]
            );
        }
    }

    #[test]
    fn parent_labor_blockers_identify_each_mismatched_or_unknown_citizen() {
        for requested in [false, true] {
            let (anchor, mut view) = joined_goal_view("false", true);
            let rows = json!([
                {"unit_id":"43","labor":"MINE","requested_enabled":requested,
                    "observed_enabled":requested,"current_truth":"true"},
                {"unit_id":"101","labor":"MINE","requested_enabled":requested,
                    "observed_enabled":!requested,"current_truth":"false"},
                {"unit_id":"9007199254740993","labor":"MINE","requested_enabled":requested,
                    "observed_enabled":null,"current_truth":"unknown"}
            ]);
            view["current_parent_goal"]["labor"] = rows.clone();
            let value = joined_packet(anchor, &view, "control", true, false);
            let turn = &value["agent_turn"];
            assert_eq!(
                turn["briefing"]["current_parent_labor"],
                json!({"citizens":3,"matched":1,"mismatched":1,"unknown":1})
            );
            assert_eq!(
                turn["uncertainty"][0]["labor_blockers"],
                json!([rows[1], rows[2]])
            );
            assert_eq!(turn["uncertainty"][0]["labor_blockers_omitted"], 0);
            assert_eq!(turn["uncertainty"][0]["anchor"], anchor_value(anchor));
            assert_eq!(turn["active_work"]["current_parent_goal"]["labor"], rows);
            assert_eq!(turn["recommendations"], json!([]));
            assert_eq!(turn["briefing"]["original_goal_proven"], false);
            assert!(
                turn["uncertainty"][0]["resolution"]
                    .as_str()
                    .expect("corrective guidance")
                    .contains("newly authorized plan")
            );
            for invalidated in [false, true] {
                let mut stale = view.clone();
                if invalidated {
                    stale["current_parent_goal"] = Value::Null;
                } else {
                    stale["current_parent_goal"]["anchor"]["state_hash"] = json!("d".repeat(64));
                }
                let value = joined_packet(anchor, &stale, "control", true, false);
                let turn = &value["agent_turn"];
                assert!(turn["active_work"]["current_parent_goal"].is_null());
                assert!(turn["briefing"]["current_parent_labor"].is_null());
                assert!(
                    turn["uncertainty"]
                        .as_array()
                        .expect("uncertainty")
                        .iter()
                        .all(|finding| finding.get("labor_blockers").is_none())
                );
            }
        }

        let (anchor, view) = joined_goal_view("unknown", true);
        let value = joined_packet(anchor, &view, "control", true, false);
        let turn = &value["agent_turn"];
        assert_eq!(
            turn["briefing"]["current_parent_labor"],
            json!({"citizens":1,"matched":0,"mismatched":0,"unknown":1})
        );
        assert_eq!(turn["uncertainty"][0]["labor_blockers"][0]["unit_id"], "43");
        assert!(turn["uncertainty"][0]["labor_blockers"][0]["observed_enabled"].is_null());
        assert_eq!(turn["uncertainty"][0]["epistemic_status"], "unknown");
    }

    #[test]
    fn parent_labor_blockers_bound_repetition_and_report_omitted_citizens() {
        let (anchor, mut view) = joined_goal_view("false", true);
        let rows = (1..=32)
            .map(|id| {
                json!({"unit_id":id.to_string(),"labor":"MINE",
            "requested_enabled":true,"observed_enabled":false,"current_truth":"false"})
            })
            .collect::<Vec<_>>();
        view["current_parent_goal"]["labor"] = json!(rows);
        let value = joined_packet(anchor, &view, "control", true, false);
        let turn = &value["agent_turn"];
        assert_eq!(
            turn["briefing"]["current_parent_labor"],
            json!({"citizens":32,"matched":0,"mismatched":32,"unknown":0})
        );
        assert_eq!(
            turn["uncertainty"][0]["labor_blockers"],
            json!(rows[..LABOR_BLOCKER_LIMIT])
        );
        assert_eq!(turn["uncertainty"][0]["labor_blockers_omitted"], 24);
        assert_eq!(
            turn["active_work"]["current_parent_goal"]["labor"],
            json!(rows)
        );
    }

    #[test]
    fn clock_advancement_withdraws_current_labor_findings_until_a_fresh_parent_sample() {
        for case in 0..3 {
            let (anchor, mut view) = joined_goal_view("false", true);
            match case {
                0 => view["clock_invalidates_goal"] = json!(true),
                1 => view["clock"] = json!({"unresolved":true}),
                _ => view["clock"] = json!({"inspection_required":true}),
            }
            let value = joined_packet(anchor, &view, "control", true, false);
            let turn = &value["agent_turn"];
            assert!(
                turn["active_work"]["current_parent_goal"].is_null(),
                "case {case}"
            );
            assert!(
                turn["briefing"]["current_parent_labor"].is_null(),
                "case {case}"
            );
            assert!(
                turn["uncertainty"]
                    .as_array()
                    .expect("uncertainty")
                    .iter()
                    .all(|finding| finding.get("labor_blockers").is_none()),
                "case {case}"
            );
            assert!(compact(&view)["current_parent_goal"].is_null());
            assert_eq!(turn["briefing"]["original_goal_proven"], false);
        }
    }

    #[test]
    fn incomplete_or_inconsistent_labor_findings_never_become_successful_summaries() {
        let (anchor, original) = joined_goal_view("false", true);
        for case in 0..12 {
            let mut view = original.clone();
            let row = &mut view["current_parent_goal"]["labor"][0];
            match case {
                0 => {
                    row.as_object_mut()
                        .expect("finding")
                        .remove("observed_enabled");
                }
                1 => row["current_truth"] = json!("true"),
                2 => row["requested_enabled"] = json!("true"),
                3 => row["unit_id"] = json!("043"),
                4 => row["unit_id"] = json!(43),
                5 => row["unit_id"] = json!("0"),
                6 => row["labor"] = json!("x".repeat(65)),
                7 => row["current_truth"] = json!("unknown"),
                8 => view["current_parent_goal"]["labor"] = json!(vec![row.clone(); 33]),
                9 => view["current_parent_goal"]["labor"] = json!([]),
                10 => view["current_parent_goal"]["labor"] = json!([row.clone(), row.clone()]),
                _ => view["current_parent_goal"]["labor"] = Value::Null,
            }
            let value = joined_packet(anchor, &view, "control", true, false);
            let turn = &value["agent_turn"];
            assert!(
                turn["briefing"]["current_parent_labor"].is_null(),
                "case {case}"
            );
            assert!(
                turn["uncertainty"][0].get("labor_blockers").is_none(),
                "case {case}"
            );
            assert_eq!(
                turn["briefing"]["original_goal_proven"], false,
                "case {case}"
            );
        }
    }

    #[test]
    fn missing_unverified_recovered_or_mismatched_parent_never_promotes_inventory_history() {
        let (anchor, original) = joined_goal_view("true", true);
        for case in 0..10 {
            let mut view = original.clone();
            let mut verified = true;
            let mut uncertain = false;
            let mut mode = "control";
            match case {
                0 => view["current_parent_goal"] = Value::Null,
                1 => verified = false,
                2 => uncertain = true,
                3 => mode = "offline",
                4 => view["current_parent_goal"]["anchor"]["state_hash"] = json!("d".repeat(64)),
                5 => view["current_parent_goal"]["selected_anchor"]["tick"] = json!(131),
                6 => view["selected_inventory_goal"]["proven_at_anchor"] = json!(false),
                7 => view["current_parent_goal"]["current_truth"] = json!("false"),
                8 => view["original_goal_proven"] = json!(false),
                _ => {
                    // Recovery can retain first achievement without a current
                    // sample. Replaying history never recreates the join.
                    view["current_parent_goal"] = Value::Null;
                    view["selected_inventory_goal"]["current_truth"] = Value::Null;
                    view["selected_inventory_goal"]["anchor"] = Value::Null;
                    view["selected_inventory_goal"]["proven_at_anchor"] = json!(false);
                    mode = "reconcile";
                }
            }
            let value = joined_packet(anchor, &view, mode, verified, uncertain);
            let turn = &value["agent_turn"];
            assert_eq!(
                turn["briefing"]["original_goal_proven"], false,
                "case {case}"
            );
            assert!(
                turn["briefing"]["current_parent_goal_truth"].is_null(),
                "case {case}"
            );
            assert!(
                turn["active_work"]["current_parent_goal"].is_null(),
                "case {case}"
            );
            assert!(
                turn["briefing"]["current_parent_labor"].is_null(),
                "case {case}"
            );
            assert!(
                turn["uncertainty"]
                    .as_array()
                    .expect("uncertainty")
                    .iter()
                    .all(|finding| finding.get("labor_blockers").is_none()),
                "case {case}"
            );
            assert!(
                turn["coverage"]["omitted_domains"]
                    .as_array()
                    .expect("omissions")
                    .contains(&json!("current_parent_goal"))
            );
        }
    }

    #[test]
    fn compact_and_overflow_withdraw_joined_proof_but_preserve_selected_achievement() {
        let (anchor, view) = joined_goal_view("true", true);
        let compacted = compact(&view);
        assert!(compacted["current_parent_goal"].is_null());
        assert_eq!(compacted["original_goal_proven"], false);
        assert_eq!(
            compacted["selected_inventory_goal"]["proven_at_anchor"],
            false
        );
        assert_eq!(
            compacted["selected_inventory_goal"]["first_satisfied_anchor"],
            view["selected_inventory_goal"]["first_satisfied_anchor"]
        );
        let (raw, overflow) = packet(
            "fortress.wait",
            Some(SessionId::new((1u128 << 127) | (24u128 << 57) | 1)),
            Some(anchor),
            Some("control"),
            None,
            None,
            json!({"ok":true,"payload":"x".repeat(70_000)}),
            Some(&view),
            true,
            false,
        );
        assert!(overflow);
        let value: Value = serde_json::from_str(&raw).expect("overflow packet");
        assert_eq!(value["original_goal_proven"], false);
        assert!(value["retained_reference"]["current_parent_goal"].is_null());
        assert_eq!(
            value["agent_turn"]["briefing"]["original_goal_proven"],
            false
        );
        assert!(value["agent_turn"]["active_work"]["current_parent_goal"].is_null());
        assert!(value["agent_turn"]["briefing"]["current_parent_labor"].is_null());
        assert_eq!(
            value["retained_reference"]["selected_inventory_goal"]["first_satisfied_anchor"],
            view["selected_inventory_goal"]["first_satisfied_anchor"]
        );
        assert_eq!(value["agent_turn"]["recommendations"], json!([]));
    }
    #[test]
    fn fenced_parent_observation_preserves_inspection_guidance_without_goal_waits() {
        let (anchor, mut view) = joined_goal_view("true", true);
        view["parent_goal_observation_fenced"] = json!(true);
        for verified in [false, true] {
            let value = joined_packet(anchor, &view, "control", verified, false);
            let turn = &value["agent_turn"];
            assert_eq!(turn["briefing"]["parent_goal_observation_fenced"], true);
            assert_eq!(turn["briefing"]["original_goal_proven"], false);
            assert!(turn["active_work"]["current_parent_goal"].is_null());
            assert!(turn["briefing"]["current_parent_labor"].is_null());
            assert_eq!(turn["recommendations"], json!([]));
            assert_eq!(turn["uncertainty"][0]["state"], "observation_fenced");
            assert_eq!(turn["uncertainty"][0]["retryable_in_session"], false);
            assert!(
                turn["uncertainty"][0]["resolution"]
                    .as_str()
                    .expect("recovery guidance")
                    .contains("fortress_doctor")
            );
        }
        // Clearing current proof after a failure must not hide the fence.
        view["current_parent_goal"] = Value::Null;
        view["original_goal_proven"] = json!(false);
        view["selected_inventory_goal"]["proven_at_anchor"] = json!(false);
        assert_eq!(compact(&view)["parent_goal_observation_fenced"], true);
        let compacted = compact(&view);
        let turn = joined_packet(anchor, &compacted, "control", false, false);
        assert_eq!(
            turn["agent_turn"]["briefing"]["parent_goal_observation_fenced"],
            true
        );
        assert_eq!(
            turn["agent_turn"]["uncertainty"][0]["retryable_in_session"],
            false
        );
        assert_eq!(turn["agent_turn"]["recommendations"], json!([]));
        let (raw, overflow) = packet(
            "fortress.wait",
            Some(SessionId::new((1u128 << 127) | (24u128 << 57) | 1)),
            Some(anchor),
            Some("control"),
            None,
            None,
            json!({"ok":true,"payload":"x".repeat(70_000)}),
            Some(&view),
            true,
            false,
        );
        assert!(overflow);
        let value: Value = serde_json::from_str(&raw).expect("fenced overflow packet");
        assert_eq!(
            value["retained_reference"]["parent_goal_observation_fenced"],
            true
        );
        assert_eq!(
            value["agent_turn"]["briefing"]["parent_goal_observation_fenced"],
            true
        );
        assert_eq!(value["agent_turn"]["recommendations"], json!([]));
        // Uncertain effects still require same-key reconciliation. This is not
        // another ordinary parent acquisition and takes priority over the fence.
        view["frontier"]["all_applied"] = json!(false);
        view["frontier"]["needs_reconciliation"] = json!(true);
        let uncertain = joined_packet(anchor, &view, "reconcile", false, true);
        assert_eq!(
            uncertain["agent_turn"]["recommendations"][0]["tool"],
            "fortress_wait"
        );
        assert!(
            uncertain["agent_turn"]["recommendations"][0]["reason"]
                .as_str()
                .expect("reconciliation guidance")
                .contains("original uncertain key")
        );
    }

    #[test]
    fn retained_parent_source_fence_stops_poll_retries_without_clearing_effect_uncertainty() {
        let (anchor, mut original) = joined_goal_view("true", true);
        original["parent_goal_observation_fenced"] = json!(true);
        original["frontier"]["needs_reconciliation"] = json!(false);
        let compacted = compact(&original);
        assert_eq!(compacted["parent_goal_observation_fenced"], true);
        assert!(compacted["current_parent_goal"].is_null());
        for view in [&original, &compacted] {
            let result = joined_packet(anchor, view, "control", false, true);
            let turn = &result["agent_turn"];
            assert_eq!(turn["briefing"]["parent_goal_observation_fenced"], true);
            assert_eq!(turn["briefing"]["original_goal_proven"], false);
            assert!(turn["active_work"]["current_parent_goal"].is_null());
            assert_eq!(
                turn["active_work"]["indeterminate_effects"][0]["state"],
                "unknown"
            );
            assert_eq!(turn["uncertainty"][0]["retryable_in_session"], false);
            assert_eq!(turn["recommendations"], json!([]));
        }
        for (all_applied, needs_reconciliation, fenced) in [
            (false, false, true),
            (true, true, true),
            (true, false, false),
        ] {
            let mut unsettled = compacted.clone();
            unsettled["frontier"]["all_applied"] = json!(all_applied);
            unsettled["frontier"]["needs_reconciliation"] = json!(needs_reconciliation);
            unsettled["parent_goal_observation_fenced"] = json!(fenced);
            assert_eq!(
                joined_packet(anchor, &unsettled, "control", false, true)["agent_turn"]["recommendations"]
                    [0]["tool"],
                "fortress_wait"
            );
        }
    }

    #[test]
    fn selected_inventory_proof_guides_polling_without_promoting_parent_success() {
        let session = SessionId::new((1u128 << 127) | (24u128 << 57) | 1);
        let mut view = json!({"original_plan":{"plan_digest":"a".repeat(64)},
            "frontier":{"all_applied":true,"next_ready":null,"entries":[]},
            "review":null,"selected_inventory_goal":{"scope":"selected_original_order_step",
                "step_id":1,"kind":"observed_inventory_minimum","item_type":"BED",
                "minimum_stack_units":1,"current_truth":"true","proven_at_anchor":false,
                "history":{"present":true,"fenced":false},"terminal_failure":false}});
        let call = |view: &Value, mode: &str, verified: bool| {
            let (raw, overflow) = packet(
                "fortress.wait",
                Some(session),
                None,
                Some(mode),
                None,
                None,
                json!({"ok":verified}),
                Some(view),
                verified,
                false,
            );
            assert!(!overflow);
            serde_json::from_str::<Value>(&raw).expect("valid packet")
        };
        let pending = call(&view, "control", true);
        assert_eq!(
            pending["agent_turn"]["recommendations"][0]["tool"],
            "fortress_wait"
        );
        assert_eq!(
            pending["agent_turn"]["active_work"]["obligations"][0]["step_id"],
            1
        );
        view["selected_inventory_goal"]["proven_at_anchor"] = json!(true);
        let proved = call(&view, "control", true);
        assert_eq!(proved["agent_turn"]["recommendations"], json!([]));
        assert_eq!(
            proved["agent_turn"]["briefing"]["selected_inventory_goal_proven_at_anchor"],
            true
        );
        assert_eq!(
            proved["agent_turn"]["briefing"]["original_goal_proven"],
            false
        );
        assert_eq!(
            proved["agent_turn"]["briefing"]["current_state_proven"],
            false
        );
        let failed = call(&view, "control", false);
        assert_eq!(
            failed["agent_turn"]["briefing"]["selected_inventory_goal_proven_at_anchor"],
            false
        );
        assert_eq!(
            failed["agent_turn"]["active_work"]["obligations"],
            json!([])
        );
        view["selected_inventory_goal"]["proven_at_anchor"] = json!(false);
        assert_eq!(
            call(&view, "offline", true)["agent_turn"]["recommendations"],
            json!([])
        );
        for field in ["terminal_failure", "fenced"] {
            let mut stopped = view.clone();
            if field == "fenced" {
                stopped["selected_inventory_goal"]["history"][field] = json!(true);
            } else {
                stopped["selected_inventory_goal"][field] = json!(true);
            }
            assert_eq!(
                call(&stopped, "control", true)["agent_turn"]["recommendations"],
                json!([])
            );
        }
    }

    #[test]
    fn compact_inventory_recovery_retains_custody_and_withdraws_current_proof() {
        let view = json!({"recovery":{"goal_history_present":true},
            "selected_inventory_goal":{"scope":"selected_original_order_step","step_id":1,
                "kind":"observed_inventory_minimum","item_type":"BED","minimum_stack_units":1,
                "current_truth":"true","proven_at_anchor":true,
                "history":{"present":true,"fenced":false},
                "first_satisfied_anchor":{"fortress_id":"7","epoch":1,"sequence":5,
                    "tick":130,"state_hash":"f".repeat(64)}}});
        let retained = compact(&view);
        assert_eq!(retained["recovery"]["goal_history_present"], true);
        assert_eq!(
            retained["selected_inventory_goal"]["first_satisfied_anchor"]["tick"],
            130
        );
        assert_eq!(
            retained["selected_inventory_goal"]["current_truth"],
            Value::Null
        );
        assert_eq!(
            retained["selected_inventory_goal"]["proven_at_anchor"],
            false
        );
        assert_eq!(retained["original_goal_proven"], false);
    }

    #[test]
    fn later_choice_pages_are_explicit_fresh_reads_and_withdrawn_on_error() {
        let session = SessionId::new((1u128 << 127) | (24u128 << 57) | 1);
        let next = json!({"units":["43"],"option_offset":8});
        let view = json!({"original_plan":null,"labor_control_enabled":true,
            "production_control_enabled":true,"observation":{"seal":"a".repeat(64),
                "workforce_choices":{"options":[{"labor":"MINE"}],
                    "next_observation_request":next}}});
        for (verified, uncertain, expected) in
            [(true, false, 1), (false, false, 0), (true, true, 0)]
        {
            let (raw, overflow) = packet(
                "fortress.query",
                Some(session),
                None,
                Some("control"),
                None,
                None,
                json!({"ok":verified}),
                Some(&view),
                verified,
                uncertain,
            );
            assert!(!overflow);
            let response: Value = serde_json::from_str(&raw).expect("packet JSON");
            let suggestions = response["agent_turn"]["recommendations"]
                .as_array()
                .expect("recommendations");
            let pages = suggestions
                .iter()
                .filter(|item| item["tool"] == "fortress_observe")
                .collect::<Vec<_>>();
            assert_eq!(pages.len(), expected);
            if verified && !uncertain {
                assert_eq!(suggestions[0]["tool"], "fortress_plan");
                let arguments = &pages[0]["arguments"];
                assert_eq!(arguments["session_id"], session.to_string());
                let request: Value = serde_json::from_str(
                    arguments["request"]
                        .as_str()
                        .expect("closed observation request"),
                )
                .expect("observation request JSON");
                assert_eq!(request, next);
            }
        }
    }

    #[test]
    fn existing_worker_guidance_needs_production_authority_and_retains_its_fresh_page() {
        let session = SessionId::new((1u128 << 127) | (24u128 << 57) | 2);
        let next = json!({"units":["43"],"option_offset":0,"existing_labor_offset":8});
        let mut view = json!({"original_plan":null,"labor_control_enabled":false,
            "production_control_enabled":true,"observation":{"seal":"a".repeat(64),
                "workforce_choices":{"options":[],
                    "existing_labor_options":[{"labor":"MINE","eligible_enabled_ids":["43"]}],
                    "next_existing_labor_observation_request":next}}});
        let call = |view: &Value, verified| {
            let (raw, overflow) = packet(
                "fortress.query",
                Some(session),
                None,
                Some("control"),
                None,
                None,
                json!({"ok":verified}),
                Some(view),
                verified,
                false,
            );
            assert!(!overflow);
            serde_json::from_str::<Value>(&raw).expect("packet JSON")
        };
        let response = call(&view, true);
        let recommendations = response["agent_turn"]["recommendations"]
            .as_array()
            .expect("recommendations");
        assert_eq!(recommendations[0]["tool"], "fortress_plan");
        assert!(recommendations[0].to_string().contains("existing_labor"));
        let page = recommendations
            .iter()
            .find(|r| r["tool"] == "fortress_observe")
            .expect("fresh existing-labor page");
        assert_eq!(
            serde_json::from_str::<Value>(page["arguments"]["request"].as_str().expect("request"))
                .expect("request JSON"),
            next
        );
        view["production_control_enabled"] = json!(false);
        assert!(
            call(&view, true)["agent_turn"]["recommendations"]
                .as_array()
                .expect("recommendations")
                .iter()
                .all(|r| r["tool"] != "fortress_plan")
        );
        assert!(
            call(&view, false)["agent_turn"]["recommendations"]
                .as_array()
                .expect("recommendations")
                .iter()
                .all(|r| r["tool"] != "fortress_plan" && r["tool"] != "fortress_observe")
        );
    }

    #[test]
    fn recommendations_follow_observation_and_numeric_next_step_without_claiming_goal() {
        let original = "a".repeat(64);
        let session = SessionId::new((1u128 << 127) | (24u128 << 57) | 1);
        let mut view = json!({"observation":null,"original_plan":null,"frontier":null,
            "labor_control_enabled":true,"production_control_enabled":true});
        let call = |view: &Value, verified| {
            let (raw, overflow) = packet(
                "fortress.query",
                Some(session),
                None,
                Some("control"),
                None,
                None,
                json!({"ok":true}),
                Some(view),
                verified,
                false,
            );
            assert!(!overflow);
            serde_json::from_str::<Value>(&raw).expect("packet JSON")
        };
        assert_eq!(
            call(&view, true)["agent_turn"]["recommendations"][0]["tool"],
            "fortress_observe"
        );
        view["observation"] = json!({"seal":"b".repeat(64),
            "workforce_choices":{"options":[{"labor":"MINE","enable_candidates":["43"]}]}});
        assert_eq!(
            call(&view, true)["agent_turn"]["recommendations"][0]["tool"],
            "fortress_plan"
        );
        view["labor_control_enabled"] = json!(false);
        assert_eq!(
            call(&view, true)["agent_turn"]["recommendations"],
            json!([])
        );
        view["labor_control_enabled"] = json!(true);
        view["original_plan"] = json!({"plan_digest":original});
        view["frontier"] = json!({"next_ready":0,"review":null,"entries":[]});
        let response = call(&view, true);
        let recommendation = &response["agent_turn"]["recommendations"][0];
        assert_eq!(recommendation["tool"], "fortress_plan");
        assert_eq!(
            recommendation["arguments"]["session_id"],
            session.to_string()
        );
        let request: Value = serde_json::from_str(
            recommendation["arguments"]["request"]
                .as_str()
                .expect("request"),
        )
        .expect("request JSON");
        assert_eq!(request["kind"], "step");
        assert_eq!(request["original_plan_digest"], original);
        assert!(
            call(&view, false)["agent_turn"]["recommendations"]
                .as_array()
                .expect("recommendations")
                .is_empty()
        );
        view["frontier"]["next_ready"] = Value::Null;
        view["frontier"]["all_applied"] = json!(true);
        assert!(
            call(&view, true)["agent_turn"]["recommendations"]
                .as_array()
                .expect("recommendations")
                .is_empty()
        );
        assert_eq!(
            call(&view, true)["agent_turn"]["briefing"]["original_goal_proven"],
            false
        );
        view["review"] = json!({"paired_seal":"c".repeat(64),"native_review":{"detail_index":1}});
        let reviewed = call(&view, true);
        assert_eq!(
            reviewed["agent_turn"]["active_work"]["confirmations"][0]["native_review"]["detail_index"],
            1
        );
        assert_eq!(
            reviewed["agent_turn"]["recommendations"][0]["tool"],
            "fortress_commit"
        );
        let (closed, _) = packet(
            "fortress.cancel",
            Some(session),
            None,
            None,
            None,
            None,
            json!({"ok":true,"closed":true}),
            Some(&view),
            true,
            true,
        );
        let closed: Value = serde_json::from_str(&closed).expect("closed packet");
        assert!(
            closed["agent_turn"]["recommendations"]
                .as_array()
                .expect("recommendations")
                .is_empty()
        );
        let (unbound, _) = packet(
            "fortress.query",
            None,
            None,
            None,
            None,
            None,
            json!({"ok":false}),
            Some(&view),
            true,
            true,
        );
        let unbound: Value = serde_json::from_str(&unbound).expect("unbound packet");
        assert!(
            unbound["agent_turn"]["recommendations"]
                .as_array()
                .expect("recommendations")
                .is_empty()
        );
    }
    #[test]
    fn offline_uncertainty_offers_close_and_reconcile_not_a_nonprogressing_wait() {
        let session = SessionId::new((1u128 << 127) | (24u128 << 57) | 1);
        let view = json!({"frontier":{"needs_reconciliation":true,"entries":[{"step_id":0,"phase":"indeterminate"}]}});
        let (raw, overflow) = packet(
            "fortress.query",
            Some(session),
            None,
            Some("offline"),
            None,
            None,
            json!({"ok":true}),
            Some(&view),
            true,
            true,
        );
        assert!(!overflow);
        let value: Value = serde_json::from_str(&raw).expect("packet JSON");
        let next = &value["agent_turn"]["recommendations"][0];
        assert_eq!(next["tool"], "fortress_cancel");
        assert_eq!(next["arguments"]["scope"], "session");
        assert_eq!(next["arguments"]["session_id"], session.to_string());
        assert!(
            next["reason"]
                .as_str()
                .expect("reason")
                .contains("reconcile")
        );
    }
    #[test]
    fn overflow_withdraws_review_and_keeps_bounded_uncertainty() {
        let retained = json!({"frontier":{"entries":[{"step_id":0, "idempotency_key":"original-key", "family":"workforce",
            "phase":"indeterminate", "dispatch_attempted":true}],"needs_reconciliation":true,
            "review":{"paired_seal":"b".repeat(64)}},
            "recovery":{"plan_digest":"c".repeat(64),"object_digest":"d".repeat(64),
                "identity_digest":"e".repeat(64),"anchor":{"fortress_id":"1","epoch":0,"sequence":1,"tick":10,"state_hash":"f".repeat(64)},
                "frontier":{"journal_id":"1".repeat(64),"head":"2".repeat(64),"authority_tick_floor":10}}});
        let (raw, withdrawn) = packet(
            "fortress.plan",
            None,
            None,
            None,
            None,
            None,
            json!({"ok":true,"payload":"x".repeat(70_000)}),
            Some(&retained),
            true,
            true,
        );
        assert!(withdrawn);
        assert!(serde_json::to_string(&raw).is_ok_and(|s| s.len() <= TEXT_LIMIT));
        let value: Value = serde_json::from_str(&raw).expect("packet JSON");
        assert_eq!(
            value["retained_reference"]["frontier"]["entries"][0]["phase"],
            "indeterminate"
        );
        assert_eq!(
            value["retained_reference"]["frontier"]["entries"][0]["step_id"],
            0
        );
        assert_eq!(
            value["retained_reference"]["frontier"]["entries"][0]["idempotency_key"],
            "original-key"
        );
        assert_eq!(
            value["retained_reference"]["frontier"]["needs_reconciliation"],
            true
        );
        assert!(value["retained_reference"]["frontier"]["review"].is_null());
        assert_eq!(
            value["retained_reference"]["recovery"]["frontier"]["authority_tick_floor"],
            10
        );
        assert_eq!(value["original_goal_proven"], false);
    }
}
