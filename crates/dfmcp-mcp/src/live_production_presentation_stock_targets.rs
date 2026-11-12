//! Lossless bounded stock-portfolio identity and complete-target proof checks.
use super::{bounded, compact_anchor, digest, stock_target_summary};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

fn item_type(recipe: &str) -> Option<&'static str> {
    match recipe {
        "wooden_bed" => Some("BED"),
        "wooden_door" => Some("DOOR"),
        "wooden_table" => Some("TABLE"),
        "wooden_chair" => Some("CHAIR"),
        _ => None,
    }
}

fn targets(value: &Value) -> Option<&[Value]> {
    if value["schema"] != "dfmcp.stock-targets/1" {
        return None;
    }
    let rows = value["targets"].as_array()?;
    if rows.is_empty() || rows.len() > 4 {
        return None;
    }
    let mut seen = BTreeSet::new();
    for row in rows {
        let kind = item_type(row["recipe"].as_str()?)?;
        let minimum = row["target_stack_units"].as_u64()?;
        if row["item_type"] != kind || !(1..=1_000_000).contains(&minimum) || !seen.insert(kind) {
            return None;
        }
    }
    Some(rows)
}

/// Every requested target survives compaction, including targets with no order.
/// Historical sizing never becomes a current inventory observation or permit.
pub(super) fn summary(value: &Value) -> Value {
    let Some(rows) = targets(value) else {
        return Value::Null;
    };
    let targets: Vec<_> = rows
        .iter()
        .map(|row| {
            let mut target = stock_target_summary(row);
            target["empty_queue_required_before_insertion"] = json!(false);
            target["queue_policy"] = json!("exact_receipt_backed_original_prefix");
            target["item_rows"] = json!(row["item_rows"].as_u64());
            target
        })
        .collect();
    json!({
        "schema":"dfmcp.stock-targets/1",
        "status":bounded(&value["status"],32),
        "targets":targets,
        "target_count":rows.len(),
        "order_count":rows.iter().filter(|row| row["order_step_id"].is_u64()).count(),
        "baseline_anchor":compact_anchor(&value["baseline_anchor"]),
        "source_digest":digest(&value["source_digest"]),
        "source_generation":value["source_generation"].as_u64(),
        "item_rows":value["item_rows"].as_u64(),
        "last_check_order_step_id":null,
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
        "worker_prerequisite_checked":value["worker_prerequisite_checked"].as_bool(),
        "satisfied_at_anchor":value["status"]=="already_satisfied"
            && value["satisfied_at_anchor"]==true,
        "all_target_minima_retained":true,
        "exact_original_census_required_before_insertion":value["status"]=="planned",
        "queue_policy":"exact_receipt_backed_original_prefix",
        "fresh_for_current_call":false,
        "current_inventory_proven":false,
        "continuous_maintenance":false,
        "native_recipe_feasibility_proven":false,
        "production_attribution":"unknown",
        "inventory_semantics":"all_observed_type_stack_units"
    })
}

/// Reconstruct the inventory contribution from a complete set of qualified
/// current findings. Missing rows cannot silently remove an initially met goal.
pub(super) fn inventory_truth(view: &Value, parent: &Value) -> Option<&'static str> {
    let requested = &view["stock_targets"];
    if requested.is_null() {
        return match parent.get("inventory_requirements") {
            None | Some(Value::Null) => Some("true"),
            Some(Value::Array(rows)) if rows.is_empty() => Some("true"),
            _ => None,
        };
    }
    let expected: BTreeMap<_, _> = targets(requested)?
        .iter()
        .map(|row| {
            Some((
                row["item_type"].as_str()?,
                row["target_stack_units"].as_u64()?,
            ))
        })
        .collect::<Option<_>>()?;
    let rows = parent["inventory_requirements"].as_array()?;
    if rows.len() != expected.len() {
        return None;
    }
    let mut previous = None;
    let mut false_goal = false;
    let mut unknown = false;
    for row in rows {
        let kind = row["item_type"].as_str()?;
        let minimum = row["minimum_stack_units"].as_u64()?;
        if previous.is_some_and(|last| last >= kind) || expected.get(kind) != Some(&minimum) {
            return None;
        }
        previous = Some(kind);
        let truth = match row.get("observed_stack_units")? {
            Value::Null => "unknown",
            value => {
                if value.as_u64()? >= minimum {
                    "true"
                } else {
                    "false"
                }
            }
        };
        if row["current_truth"] != truth {
            return None;
        }
        false_goal |= truth == "false";
        unknown |= truth == "unknown";
    }
    Some(if false_goal {
        "false"
    } else if unknown {
        "unknown"
    } else {
        "true"
    })
}

pub(super) fn current_inventory_consistent(view: &Value, parent: &Value) -> bool {
    match inventory_truth(view, parent) {
        Some("false") => parent["current_truth"] == "false",
        Some("unknown") => matches!(parent["current_truth"].as_str(), Some("false" | "unknown")),
        Some("true") => true,
        _ => false,
    }
}

/// A completed leaf's historical streak cannot substitute for the original
/// all-target cohort's cadence and stability, particularly after stock loss.
pub(super) fn stability_proven(view: &Value, parent: &Value) -> Option<bool> {
    if view["stock_targets"].is_null() {
        return Some(true);
    }
    let requested = &view["stock_targets"];
    let rows = targets(requested)?;
    let steps = view["original_plan"]["steps"].as_array()?;
    let offset = match steps.first()?["action"]["kind"].as_str()? {
        "set_labor"
            if steps[0]["step_id"] == 0
                && steps[0]["depends_on"] == json!([])
                && steps[0]["obligation"].is_null() =>
        {
            1
        }
        "create_work_order" => 0,
        _ => return None,
    };
    let assignment = offset == 1;
    if requested["status"] != "planned"
        || requested["labor_assignment_requested"] != assignment
        || requested["labor_assignment_planned"] != assignment
    {
        return None;
    }
    // The assignment is a dependency, not an inventory stability sample.
    // Timing belongs to the original positive-deficit orders at their real IDs.
    let orders = steps.get(offset..)?;
    if orders.is_empty() || orders.len() > 4 {
        return None;
    }
    let mut next_order = offset;
    for row in rows {
        let positive = row["amount"].as_u64()? > 0;
        if row["labor_assignment_requested"] != assignment
            || row["labor_assignment_planned"] != (assignment && positive)
            || (positive && row["order_step_id"] != next_order)
            || (!positive && !row["order_step_id"].is_null())
        {
            return None;
        }
        next_order += usize::from(positive);
    }
    if next_order != steps.len() {
        return None;
    }
    let first = orders.first()?;
    let required = first["obligation"]["stable_samples"].as_u64()?;
    let cadence = first["obligation"]["poll_interval_ticks"].as_u64()?;
    let deadline = first["obligation"]["deadline_tick"].as_u64()?;
    if required == 0
        || cadence == 0
        || orders.iter().enumerate().any(|(ordinal, step)| {
            step["action"]["kind"] != "create_work_order"
                || step["step_id"] != ordinal + offset
                || step["depends_on"] != if assignment { json!([0]) } else { json!([]) }
                || step["obligation"]["stable_samples"] != required
                || step["obligation"]["poll_interval_ticks"] != cadence
                || step["obligation"]["deadline_tick"] != deadline
        })
    {
        return None;
    }
    let sample = &parent["parent_stability"];
    let accepted = sample["accepted_samples"].as_u64()?;
    let tick = parent["anchor"]["tick"].as_u64()?;
    if sample["required_samples"] != required
        || sample["poll_interval_ticks"] != cadence
        || sample["deadline_tick"] != deadline
        || accepted > required
    {
        return None;
    }
    let fulfilled = sample["fulfilled_at_tick"].as_u64();
    if !sample["fulfilled_at_tick"].is_null() && fulfilled.is_none() {
        return None;
    }
    if accepted == 0 {
        if !sample["first_candidate_tick"].is_null()
            || !sample["last_sample_tick"].is_null()
            || fulfilled.is_some()
        {
            return None;
        }
    } else {
        let first_tick = sample["first_candidate_tick"].as_u64()?;
        let last_tick = sample["last_sample_tick"].as_u64()?;
        let minimum_span = cadence.checked_mul(accepted.checked_sub(1)?)?;
        if last_tick != tick || last_tick.checked_sub(first_tick)? < minimum_span {
            return None;
        }
        if let Some(fulfilled_tick) = fulfilled {
            if accepted != required
                || fulfilled_tick > deadline
                || fulfilled_tick > last_tick
                || fulfilled_tick.checked_sub(first_tick)? < minimum_span
            {
                return None;
            }
        } else if accepted == required || tick >= deadline || parent["current_truth"] != "true" {
            return None;
        }
    }
    let proven = fulfilled.is_some() && parent["current_truth"] == "true";
    let expected_status = if fulfilled.is_some() {
        "fulfilled"
    } else if tick >= deadline {
        "expired"
    } else {
        "pending"
    };
    if sample["proven_at_anchor"] != proven
        || sample["status"] != expected_status
        || sample["restored_from_history"] != false
    {
        return None;
    }
    Some(proven)
}
