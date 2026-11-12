//! Retained input assessments explain the original requirement. They never
//! reserve inventory or authorize another order from a cached successful read.
use super::{bounded, compact_anchor, digest};
use serde_json::{Value, json};

const EXCLUSIONS: &[&str] = &[
    "zero_stack_size",
    "removed_in_chain",
    "forbidden_in_chain",
    "rotten_in_chain",
    "trader_in_chain",
    "attached_to_job_in_chain",
    "in_job_in_chain",
    "dump_in_chain",
    "in_inventory_in_chain",
    "in_building_in_chain",
    "building_holder_in_chain",
];

pub(super) fn floor(value: &Value) -> Value {
    if !value.is_object() {
        return Value::Null;
    }
    json!({"tick":value["tick"].as_u64(),"generation":value["generation"].as_u64(),
        "next_job_id":value["next_job_id"].as_u64(),
        "next_building_id":value["next_building_id"].as_u64(),
        "next_item_id":value["next_item_id"].as_u64(),"fenced":value["fenced"].as_bool()})
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
    // A refused positive plan still needs an intelligible shortage diagnosis.
    // This is an earlier observation, including in an error/compact response.
    let historical = if last.is_object()
        && digest(&last["witness"]).is_some()
        && ((retain_last && last["status"] == "ready") || last["status"] == "shortage")
    {
        let mut excluded = serde_json::Map::new();
        for reason in EXCLUSIONS {
            if let Some(count) = last["excluded_items"][reason].as_u64() {
                excluded.insert((*reason).into(), json!(count));
            }
        }
        json!({"tick":last["tick"].as_u64(),"witness":digest(&last["witness"]),
            "canonical_parent":compact_anchor(&last["canonical_parent"]),
            "status":bounded(&last["status"],16),
            "required_stack_units":last["required_stack_units"].as_u64(),
            "eligible_stack_units":last["eligible_stack_units"].as_u64(),
            "shortfall_stack_units":last["shortfall_stack_units"].as_u64(),
            "excluded_items":excluded,"fresh_for_current_call":false,
            "current_supply_proven":false,"reservation_created":false})
    } else {
        Value::Null
    };
    let orders: Vec<_> = value["orders"]
        .as_array()
        .into_iter()
        .flatten()
        .take(4)
        .map(|order| {
            json!({"recipe":bounded(&order["recipe"],32),"amount":order["amount"].as_u64()})
        })
        .collect();
    json!({"schema":bounded(&value["schema"],64),"policy":bounded(&value["policy"],64),
        "source_protocol":"operations/1.3","scope":"original_furniture_input_supply",
        "status":bounded(&value["status"],32),"origin_witness":digest(&value["origin_witness"]),
        "binding_digest":digest(&value["binding_digest"]),
        "source_generation":value["source_generation"].as_u64(),
        "required_stack_units":value["required_stack_units"].as_u64(),
        "reserve_wood_stack_units":value["reserve_wood_stack_units"].as_u64(),"orders":orders,
        "consumed_tick_floor":value["consumed_tick_floor"].as_u64(),
        "source_fenced":value["source_fenced"].as_bool(),
        "revalidation_failed":value["revalidation_failed"].as_bool(),
        "checkpoint":checkpoint(&value["checkpoint"]),"last_qualified":historical,
        "fresh_for_current_call":false,"current_supply_proven":false,"current_state_proven":false,
        "reservation_created":false,"native_recipe_feasibility_proven":false,
        "production_feasibility_proven":false,"mutation_authority":false})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortage_explanation_survives_compaction_without_becoming_current_supply() {
        let value = json!({"orders":[{"recipe":"wooden_bed","amount":2}],
            "last_qualified":{"witness":"a".repeat(64),"status":"shortage",
                "required_stack_units":3,"eligible_stack_units":1,"shortfall_stack_units":2,
                "excluded_items":{"forbidden_in_chain":4,"unbounded_untrusted_reason":123},
                "current_supply_proven":true,"fresh_for_current_call":true}});
        let once = summary(&value, false);
        let twice = summary(&once, false);
        assert_eq!(once, twice);
        assert_eq!(once["last_qualified"]["shortfall_stack_units"], 2);
        assert_eq!(
            once["last_qualified"]["excluded_items"],
            json!({"forbidden_in_chain":4})
        );
        assert_eq!(once["last_qualified"]["fresh_for_current_call"], false);
        assert_eq!(once["current_supply_proven"], false);
        assert_eq!(once["reservation_created"], false);
        assert_eq!(once["mutation_authority"], false);
    }

    #[test]
    fn retained_success_withdraws_without_losing_complete_continuity_floor() {
        let checkpoint = json!({"origin_witness":"b".repeat(64),"tick":12,"generation":20,
            "next_job_id":40,"next_building_id":50,"next_item_id":60,"fenced":true});
        let value = json!({"checkpoint":checkpoint,"last_qualified":{
            "status":"ready","witness":"a".repeat(64),"eligible_stack_units":8,
            "required_stack_units":3,"shortfall_stack_units":0}});
        let retained = summary(&value, false);
        assert_eq!(retained["checkpoint"], checkpoint);
        assert!(retained["last_qualified"].is_null());
        let historical = summary(&value, true);
        assert_eq!(historical["last_qualified"]["status"], "ready");
        assert_eq!(historical["last_qualified"]["current_supply_proven"], false);
    }
}
