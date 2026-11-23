//! Parent checkpoint comparisons supplement each child's own durable validation.
use super::error;
use dfmcp_core::{ErrorCode, Result};
use serde_json::Value;

pub(super) fn tick(reference: &Value) -> u64 {
    [
        "/project_source_frontier/high_tick",
        "/observation_tick_floor",
        "/consumed_tick_floor",
        "/production_handoff_floor/tick",
        "/recovery/frontier/authority_tick_floor",
        "/clock/authority_tick_floor",
        "/clock/authority_tick",
        "/clock/consumed_tick_floor",
        "/inventory_clock/authority_tick_floor",
    ]
    .iter()
    .filter_map(|path| reference.pointer(path).and_then(Value::as_u64))
    .max()
    .unwrap_or(0)
}

/// Recovery may acquire a newer valid descendant, never another original.
/// Source generations of different native families are deliberately not joined.
pub(super) fn same_original(before: &Value, after: &Value) -> Result<()> {
    for path in [
        "/native_custody/workforce_journal_id",
        "/native_custody/order_journal_id",
        "/acknowledged/journal_id",
        "/journal_pin_unverified/journal_id",
        "/original/key",
        "/original/plan_digest",
        "/original/deadline_tick",
        "/original_key",
        "/original_plan_digest",
        "/original_plan",
        "/recovery/plan_digest",
        "/recovery/object_digest",
        "/recovery/identity_digest",
        "/recovery/anchor",
        "/recovery/frontier/journal_id",
        "/recovery/clock_journal_id",
        "/recovery/inventory_clock_journal_id",
        "/recovery/workshop_prerequisite_digest",
        "/recovery/manager_prerequisite_digest",
        "/recovery/input_supply_digest",
        "/clock/journal_id",
        "/clock/recovery/journal_id",
        "/clock/deadline_tick",
        "/clock/target_id",
        "/inventory_clock/journal_id",
    ] {
        if let Some(retained) = before.pointer(path).filter(|value| !value.is_null())
            && after.pointer(path) != Some(retained)
        {
            return Err(error(
                ErrorCode::Conflict,
                "project child changed its retained original identity",
            ));
        }
    }
    if tick(after) < tick(before) {
        return Err(error(
            ErrorCode::Conflict,
            "project child consumed tick floor regressed",
        ));
    }
    for path in [
        "/project_source_frontier",
        "/production_handoff_floor",
        "/recovery/workshop_prerequisite_floor",
        "/recovery/manager_prerequisite_floor",
        "/recovery/input_supply_floor",
        "/workshop_prerequisite/checkpoint",
        "/manager_prerequisite/checkpoint",
        "/input_supply/checkpoint",
        "/clock",
        "/inventory_clock",
    ] {
        if let Some(retained) = before.pointer(path).filter(|value| value.is_object()) {
            let current = after.pointer(path).ok_or_else(|| {
                error(
                    ErrorCode::Conflict,
                    "project child erased a retained source frontier",
                )
            })?;
            for key in [
                "tick",
                "high_tick",
                "generation",
                "high_generation",
                "next_building_id",
                "high_next_building_id",
                "next_order_id",
                "next_item_id",
                "next_job_id",
                "sequence",
                "high_sequence",
                "high_next_item_id",
                "next_event",
                "history_count",
                "group_next_position",
                "group_next_assignment",
                "civ_next_position",
                "civ_next_assignment",
                "authority_tick_floor",
                "consumed_tick_floor",
                "high_source_sequence",
            ] {
                if let Some(floor) = retained.get(key).and_then(Value::as_u64)
                    && current
                        .get(key)
                        .and_then(Value::as_u64)
                        .is_none_or(|now| now < floor)
                {
                    return Err(error(
                        ErrorCode::Conflict,
                        "project child source frontier regressed",
                    ));
                }
            }
            for field in ["fenced", "advancement_fenced"] {
                if retained[field] == true && current[field] != true {
                    return Err(error(
                        ErrorCode::Conflict,
                        "project child erased sticky source discontinuity",
                    ));
                }
            }
        }
    }
    for path in [
        "/recovery/goal_history_present",
        "/original/dispatch_started",
        "/dispatch_started",
    ] {
        if before.pointer(path) == Some(&Value::Bool(true))
            && after.pointer(path) != Some(&Value::Bool(true))
        {
            return Err(error(
                ErrorCode::Conflict,
                "project child erased original dispatch or goal history",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn exact_child_custodians_and_source_floors_survive_reopen() -> Result<()> {
        let before = json!({
            "native_custody":{"workforce_journal_id":"a","order_journal_id":"b"},
            "observation_tick_floor":40,
            "recovery":{"plan_digest":"p","frontier":{"journal_id":"j","head":"h","authority_tick_floor":40},
                "workshop_prerequisite_floor":{"tick":40,"generation":7,"next_building_id":12,"fenced":true}}
        });
        let mut after = before.clone();
        after["observation_tick_floor"] = json!(42);
        after["recovery"]["frontier"]["head"] = json!("descendant");
        assert!(same_original(&before, &after).is_ok());
        for path in [
            "/native_custody/order_journal_id",
            "/recovery/plan_digest",
            "/recovery/frontier/journal_id",
        ] {
            let mut changed = after.clone();
            *changed.pointer_mut(path).ok_or_else(|| {
                error(
                    ErrorCode::InternalInvariantViolation,
                    "test pointer missing",
                )
            })? = json!("replacement");
            assert!(same_original(&before, &changed).is_err());
        }
        after["recovery"]["workshop_prerequisite_floor"]["fenced"] = json!(false);
        assert!(same_original(&before, &after).is_err());
        Ok(())
    }

    #[test]
    fn workshop_dispatch_and_allocator_cannot_be_forgotten() {
        let before = json!({"acknowledged":{"journal_id":"j"},
            "original":{"key":"original","plan_digest":"p","dispatch_started":true},
            "project_source_frontier":{"high_tick":91,"high_generation":8,
                "high_next_building_id":22,"fenced":false}});
        for field in ["high_tick", "high_generation", "high_next_building_id"] {
            let mut after = before.clone();
            after["project_source_frontier"][field] = json!(0);
            assert!(same_original(&before, &after).is_err());
        }
        let mut after = before.clone();
        after["original"]["dispatch_started"] = json!(false);
        assert!(same_original(&before, &after).is_err());
    }

    #[test]
    fn tree_original_clock_and_consumed_supply_floors_survive_recovery() -> Result<()> {
        let before = json!({"acknowledged":{"journal_id":"tree"},
            "journal_pin_unverified":{"journal_id":"tree"},"original_key":"tree-key",
            "original":{"key":"tree-key","plan_digest":"plan","deadline_tick":200,"dispatch_started":true},
            "consumed_tick_floor":90,
            "project_source_frontier":{"high_tick":90,"high_generation":2,"high_sequence":8,"high_next_item_id":44,"fenced":true},
            "clock":{"recovery":{"journal_id":"clock","head":"ancestor"},"authority_tick_floor":90,"advancement_fenced":true}});
        let mut descendant = before.clone();
        descendant["clock"]["recovery"]["head"] = json!("descendant");
        descendant["project_source_frontier"]["high_sequence"] = json!(9);
        same_original(&before, &descendant)?;
        for path in [
            "/original/deadline_tick",
            "/original_key",
            "/clock/recovery/journal_id",
            "/project_source_frontier/high_sequence",
            "/project_source_frontier/high_next_item_id",
            "/project_source_frontier/fenced",
            "/clock/advancement_fenced",
        ] {
            let mut changed = descendant.clone();
            *changed.pointer_mut(path).ok_or_else(|| {
                error(
                    ErrorCode::InternalInvariantViolation,
                    "missing Tree test pointer",
                )
            })? = Value::Null;
            assert!(same_original(&before, &changed).is_err(), "{path}");
        }
        let mut regressed = descendant;
        regressed["consumed_tick_floor"] = json!(0);
        regressed["project_source_frontier"]["high_tick"] = json!(0);
        regressed["clock"]["authority_tick_floor"] = json!(0);
        assert!(same_original(&before, &regressed).is_err());
        Ok(())
    }

    #[test]
    fn manager_original_and_each_consumed_export_floor_survive_recovery() -> Result<()> {
        let before = json!({
            "acknowledged":{"journal_id":"manager","head":"ack"},
            "journal_pin_unverified":{"journal_id":"manager","head":"ack"},
            "original_key":"appointment","original_plan_digest":"plan","dispatch_started":true,
            "original":{"key":"appointment","plan_digest":"plan","deadline_tick":200,"dispatch_started":true},
            "consumed_tick_floor":90,
            "production_handoff_floor":{"tick":90,"generation":7,"sequence":9,
                "next_event":21,"history_count":19,"group_next_position":31,
                "group_next_assignment":41,"civ_next_position":51,"civ_next_assignment":61,"fenced":true}
        });
        let mut after = before.clone();
        after["acknowledged"]["head"] = json!("descendant");
        after["production_handoff_floor"]["sequence"] = json!(10);
        same_original(&before, &after)?;
        for field in [
            "tick",
            "generation",
            "sequence",
            "next_event",
            "history_count",
            "group_next_position",
            "group_next_assignment",
            "civ_next_position",
            "civ_next_assignment",
        ] {
            let mut changed = after.clone();
            changed["production_handoff_floor"][field] = json!(0);
            assert!(same_original(&before, &changed).is_err(), "{field}");
        }
        for path in [
            "/original_key",
            "/original_plan_digest",
            "/original/deadline_tick",
            "/dispatch_started",
            "/production_handoff_floor/fenced",
        ] {
            let mut changed = after.clone();
            *changed.pointer_mut(path).ok_or_else(|| {
                error(
                    ErrorCode::InternalInvariantViolation,
                    "missing Manager continuity fixture",
                )
            })? = Value::Null;
            assert!(same_original(&before, &changed).is_err(), "{path}");
        }
        Ok(())
    }
}
