//! Bounded retained manager custody; earlier readiness never becomes current
//! production feasibility or an independently usable dispatch permission.
use super::{bounded, compact_anchor, digest};
use serde_json::{Value, json};

pub(super) fn floor(value: &Value) -> Value {
    if !value.is_object() {
        return Value::Null;
    }
    json!({"tick":value["tick"].as_u64(),"generation":value["generation"].as_u64(),
        "sequence":value["sequence"].as_u64(),"next_event":value["next_event"].as_u64(),
        "history_count":value["history_count"].as_u64(),
        "group_next_position":value["group_next_position"].as_u64(),
        "group_next_assignment":value["group_next_assignment"].as_u64(),
        "civ_next_position":value["civ_next_position"].as_u64(),
        "civ_next_assignment":value["civ_next_assignment"].as_u64(),
        "fenced":value["fenced"].as_bool()})
}

fn checkpoint(value: &Value) -> Value {
    let mut result = floor(value);
    if result.is_object() {
        result["origin_witness"] = json!(digest(&value["origin_witness"]));
    }
    result
}

pub(super) fn summary(value: &Value, retain_last: bool) -> Value {
    if !value.is_object() {
        return Value::Null;
    }
    let last = &value["last_qualified"];
    let historical = if retain_last
        && last.is_object()
        && last["status"] == "ready"
        && digest(&last["witness"]).is_some()
    {
        json!({"tick":last["tick"].as_u64(),"witness":digest(&last["witness"]),
            "canonical_parent":compact_anchor(&last["canonical_parent"]),"status":"ready",
            "fresh_for_current_call":false,"current_readiness_proven":false})
    } else {
        Value::Null
    };
    json!({"schema":bounded(&value["schema"],64),"source_protocol":bounded(&value["source_protocol"],32),
        "scope":"original_manager_dispatch_prerequisite","status":bounded(&value["status"],32),
        "origin_witness":digest(&value["origin_witness"]),
        "manager_key":bounded(&value["manager_key"],128),
        "manager_plan_digest":digest(&value["manager_plan_digest"]),
        "manager_record_digest":digest(&value["manager_record_digest"]),
        "manager_receipt":digest(&value["manager_receipt"]),
        "unit_id":value["unit_id"].as_i64(),"historical_figure_id":value["historical_figure_id"].as_i64(),
        "group_id":value["group_id"].as_i64(),"assignment_id":value["assignment_id"].as_i64(),
        "association_digest":digest(&value["association_digest"]),
        "source_generation":value["source_generation"].as_u64(),
        "consumed_tick_floor":value["consumed_tick_floor"].as_u64(),
        "source_fenced":value["source_fenced"].as_bool(),"checkpoint":checkpoint(&value["checkpoint"]),
        "last_qualified":historical,"current_readiness_proven":false,"current_state_proven":false,
        "canonical_manager_evidence":false,"production_feasibility_proven":false,"mutation_authority":false})
}
