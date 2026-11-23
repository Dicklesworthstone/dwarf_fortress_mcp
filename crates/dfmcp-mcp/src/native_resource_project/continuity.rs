//! Parent checkpoint comparisons supplement each child's own durable validation.
use super::error;
use dfmcp_core::{Digest32, ErrorCode, Result};
use serde_json::Value;
use std::collections::BTreeSet;

pub(super) fn tick(reference: &Value) -> u64 {
    let scalar = [
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
    .unwrap_or(0);
    let portfolio = reference
        .get("portfolio_clocks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|slot| ["clock", "inventory_clock"].map(|family| &slot[family]))
        .filter_map(|pin| pin["authority_tick_floor"].as_u64())
        .max()
        .unwrap_or(0);
    scalar.max(portfolio)
}

/// Recovery may acquire a newer valid descendant, never another original.
/// Source generations of different native families are deliberately not joined.
pub(super) fn same_original(before: &Value, after: &Value) -> Result<()> {
    same_portfolio(before, after)?;
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

/// Portfolio history belongs to every original order, including inactive
/// leaves. Scalar summaries stay bound to the first original order, while the
/// complete clock vector retains every owner's identity and rejection floors.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PortfolioSlot {
    step_id: u32,
    goal_history_present: bool,
    clock_journal_id: Option<String>,
    inventory_clock_journal_id: Option<String>,
}

#[derive(PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ClockPin {
    journal_id: String,
    head: String,
    authority_tick_floor: u64,
    high_source_sequence: u64,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PortfolioClockSlot {
    step_id: u32,
    clock: Option<ClockPin>,
    inventory_clock: Option<ClockPin>,
}

struct PortfolioReference {
    slots: Vec<PortfolioSlot>,
    clocks: Option<Vec<PortfolioClockSlot>>,
}

fn portfolio_conflict() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::Conflict,
        "project child changed its complete original portfolio custody",
    )
}

fn optional_value<'a>(reference: &'a Value, path: &str) -> Option<&'a Value> {
    reference.pointer(path).filter(|value| !value.is_null())
}

fn canonical_digest(value: &str) -> bool {
    Digest32::from_hex(value)
        .is_some_and(|digest| digest != Digest32::ZERO && digest.to_hex() == value)
}

fn clock_pin(reference: &Value, path: &str) -> Result<Option<ClockPin>> {
    optional_value(reference, path)
        .map(|value| serde_json::from_value(value.clone()).map_err(|_| portfolio_conflict()))
        .transpose()
}

fn portfolio_slots(reference: &Value) -> Result<Option<PortfolioReference>> {
    let Some(raw) = optional_value(reference, "/recovery/portfolio") else {
        if optional_value(reference, "/portfolio_clocks").is_some() {
            return Err(portfolio_conflict());
        }
        return Ok(None);
    };
    if raw
        .as_array()
        .is_none_or(|slots| slots.is_empty() || slots.len() > 4)
    {
        return Err(portfolio_conflict());
    }
    let slots: Vec<PortfolioSlot> =
        serde_json::from_value(raw.clone()).map_err(|_| portfolio_conflict())?;
    let first = slots.first().ok_or_else(portfolio_conflict)?;
    if first.step_id > 1
        || reference
            .pointer("/recovery/goal_history_present")
            .and_then(Value::as_bool)
            != Some(first.goal_history_present)
        || slots.iter().enumerate().any(|(index, slot)| {
            slot.step_id != first.step_id + index as u32
                || slot.clock_journal_id.is_some() != first.clock_journal_id.is_some()
                || slot.inventory_clock_journal_id.is_some()
                    != first.inventory_clock_journal_id.is_some()
        })
    {
        return Err(portfolio_conflict());
    }
    for (path, id) in [
        ("/recovery/clock_journal_id", &first.clock_journal_id),
        (
            "/recovery/inventory_clock_journal_id",
            &first.inventory_clock_journal_id,
        ),
    ] {
        let actual: Option<String> = optional_value(reference, path)
            .map(|value| serde_json::from_value(value.clone()).map_err(|_| portfolio_conflict()))
            .transpose()?;
        if &actual != id {
            return Err(portfolio_conflict());
        }
    }
    let clock = clock_pin(reference, "/clock")?;
    let inventory_clock = clock_pin(reference, "/inventory_clock")?;
    let Some(raw_clocks) = optional_value(reference, "/portfolio_clocks") else {
        if first.clock_journal_id.is_some()
            || first.inventory_clock_journal_id.is_some()
            || clock.is_some()
            || inventory_clock.is_some()
        {
            return Err(portfolio_conflict());
        }
        return Ok(Some(PortfolioReference {
            slots,
            clocks: None,
        }));
    };
    if raw_clocks
        .as_array()
        .is_none_or(|clocks| clocks.len() != slots.len())
        || (clock.is_none() && inventory_clock.is_none())
    {
        return Err(portfolio_conflict());
    }
    let clocks: Vec<PortfolioClockSlot> =
        serde_json::from_value(raw_clocks.clone()).map_err(|_| portfolio_conflict())?;
    let first_clock = clocks.first().ok_or_else(portfolio_conflict)?;
    if clock != first_clock.clock || inventory_clock != first_clock.inventory_clock {
        return Err(portfolio_conflict());
    }
    let mut identities = BTreeSet::new();
    for (slot, clocks) in slots.iter().zip(&clocks) {
        if slot.step_id != clocks.step_id {
            return Err(portfolio_conflict());
        }
        for (id, pin) in [
            (slot.clock_journal_id.as_ref(), clocks.clock.as_ref()),
            (
                slot.inventory_clock_journal_id.as_ref(),
                clocks.inventory_clock.as_ref(),
            ),
        ] {
            match (id, pin) {
                (None, None) => {}
                (Some(id), Some(pin))
                    if id == &pin.journal_id
                        && canonical_digest(&pin.journal_id)
                        && canonical_digest(&pin.head)
                        && pin.authority_tick_floor
                            <= dfmcp_adapter::bounded_run::MAX_NATIVE_TICK
                        && identities.insert(pin.journal_id.clone()) => {}
                _ => return Err(portfolio_conflict()),
            }
        }
    }
    Ok(Some(PortfolioReference {
        slots,
        clocks: Some(clocks),
    }))
}

fn same_clock(before: Option<&ClockPin>, after: Option<&ClockPin>) -> bool {
    match (before, after) {
        (None, None) => true,
        (Some(before), Some(after)) => {
            // The real child's recovery verifies that the new head descends
            // from this pin. Parent metadata cannot confer journal ancestry.
            before.journal_id == after.journal_id
                && before.authority_tick_floor <= after.authority_tick_floor
                && before.high_source_sequence <= after.high_source_sequence
        }
        _ => false,
    }
}

fn same_portfolio_clocks(
    before: Option<&[PortfolioClockSlot]>,
    after: Option<&[PortfolioClockSlot]>,
) -> bool {
    match (before, after) {
        (None, None) => true,
        (Some(before), Some(after)) => {
            before.len() == after.len()
                && before.iter().zip(after).all(|(before, after)| {
                    before.step_id == after.step_id
                        && same_clock(before.clock.as_ref(), after.clock.as_ref())
                        && same_clock(
                            before.inventory_clock.as_ref(),
                            after.inventory_clock.as_ref(),
                        )
                })
        }
        _ => false,
    }
}

fn same_portfolio(before: &Value, after: &Value) -> Result<()> {
    let prior = portfolio_slots(before)?;
    let current = portfolio_slots(after)?;
    let valid = match (prior, current) {
        (None, None) => true,
        // The separately ACKed publication intent already owns the complete
        // prospective vector before the child first publishes its original.
        (None, Some(_)) => {
            before.pointer("/recovery") == Some(&Value::Null)
                && ["/clock", "/inventory_clock"].iter().all(|path| {
                    optional_value(before, path).is_some()
                        == optional_value(after, path).is_some()
                })
        }
        (Some(prior), Some(current)) => {
            prior.slots.len() == current.slots.len()
                && prior
                    .slots
                    .iter()
                    .zip(&current.slots)
                    .all(|(before, after)| {
                        before.step_id == after.step_id
                            && before.clock_journal_id == after.clock_journal_id
                            && before.inventory_clock_journal_id
                                == after.inventory_clock_journal_id
                            && (!before.goal_history_present || after.goal_history_present)
                    })
                && same_portfolio_clocks(prior.clocks.as_deref(), current.clocks.as_deref())
        }
        (Some(_), None) => false,
    };
    if !valid {
        return Err(error(
            ErrorCode::Conflict,
            "project child replaced an original order or erased its goal history",
        ));
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

    fn portfolio_reference(first: u32, count: usize) -> Value {
        json!({
            "native_custody": {"workforce_journal_id": "workforce", "order_journal_id": "orders"},
            "observation_tick_floor": 150, "clock": null, "inventory_clock": null,
            "recovery": {
                "plan_digest": "complete original", "object_digest": "complete archive",
                "frontier": {"journal_id": "execution", "head": "acknowledged", "authority_tick_floor": 150},
                "goal_history_present": false, "clock_journal_id": null, "inventory_clock_journal_id": null,
                "portfolio": (0..count).map(|index| json!({
                    "step_id": first + index as u32, "goal_history_present": false,
                    "clock_journal_id": null, "inventory_clock_journal_id": null,
                })).collect::<Vec<_>>(),
            }
        })
    }

    #[test]
    fn portfolio_publication_keeps_all_order_slots_and_accepts_monotone_history() -> Result<()> {
        for first in [0, 1] {
            for count in 1..=4 {
                let planned = portfolio_reference(first, count);
                let mut preplan = planned.clone();
                preplan["recovery"] = Value::Null;
                same_original(&preplan, &planned)?;
                let mut prior = planned;
                for index in (0..count).rev() {
                    let mut after = prior.clone();
                    after["recovery"]["portfolio"][index]["goal_history_present"] = json!(true);
                    if index == 0 {
                        after["recovery"]["goal_history_present"] = json!(true);
                    }
                    after["recovery"]["frontier"]["head"] = json!("verified descendant");
                    same_original(&prior, &after)?;
                    assert!(
                        same_original(&after, &prior).is_err(),
                        "{first}/{count}/{index}"
                    );
                    prior = after;
                }
            }
        }
        Ok(())
    }

    #[test]
    fn portfolio_replay_cannot_drop_reorder_replace_or_reinterpret_original_orders() {
        let before = portfolio_reference(0, 4);
        let edits: [fn(&mut Value); 8] = [
            |v| v["recovery"] = Value::Null,
            |v| v["recovery"]["portfolio"] = Value::Null,
            |v| v["recovery"]["portfolio"] = json!([]),
            |v| v["recovery"]["portfolio"] = json!([v["recovery"]["portfolio"][0].clone()]),
            |v| v["recovery"]["portfolio"][0]["step_id"] = json!(1),
            |v| v["recovery"]["portfolio"][1]["step_id"] = json!(0),
            |v| v["recovery"]["portfolio"][2]["step_id"] = json!(3),
            |v| v["recovery"]["portfolio"][3]["unknown"] = json!(true),
        ];
        for (index, edit) in edits.into_iter().enumerate() {
            let mut after = before.clone();
            edit(&mut after);
            assert!(same_original(&before, &after).is_err(), "{index}");
        }
        let mut longer = before.clone();
        longer["recovery"]["portfolio"] = json!([
            {"step_id":0,"goal_history_present":false},
            {"step_id":1,"goal_history_present":false},
            {"step_id":2,"goal_history_present":false},
            {"step_id":3,"goal_history_present":false},
            {"step_id":4,"goal_history_present":false}
        ]);
        assert!(same_original(&before, &longer).is_err());
        let mut scalar = before.clone();
        scalar["recovery"]["portfolio"] = Value::Null;
        assert!(same_original(&scalar, &before).is_err());
        assert!(same_original(&before, &scalar).is_err());
    }

    #[test]
    fn inactive_portfolio_history_and_first_order_summary_cannot_diverge() -> Result<()> {
        let mut before = portfolio_reference(1, 4);
        before["recovery"]["portfolio"][3]["goal_history_present"] = json!(true);
        same_original(&before, &before)?;
        let mut forgotten = before.clone();
        forgotten["recovery"]["portfolio"][3]["goal_history_present"] = json!(false);
        assert!(same_original(&before, &forgotten).is_err());
        for path in [
            "/recovery/goal_history_present",
            "/recovery/portfolio/0/goal_history_present",
        ] {
            let mut mismatch = before.clone();
            *mismatch.pointer_mut(path).ok_or_else(|| {
                error(
                    ErrorCode::InternalInvariantViolation,
                    "test portfolio summary absent",
                )
            })? = json!(true);
            assert!(same_original(&before, &mismatch).is_err(), "{path}");
        }
        let mut missing = before.clone();
        missing["recovery"]["portfolio"][2]["goal_history_present"] = Value::Null;
        assert!(same_original(&before, &missing).is_err());
        Ok(())
    }

    #[test]
    fn portfolio_parent_replay_refuses_incomplete_selected_or_inactive_clock_custody() {
        let before = portfolio_reference(0, 4);
        for path in [
            "/recovery/clock_journal_id",
            "/recovery/inventory_clock_journal_id",
            "/clock",
            "/inventory_clock",
        ] {
            let mut after = before.clone();
            if let Some(value) = after.pointer_mut(path) {
                *value = json!("unacknowledged clock");
            }
            assert!(same_original(&before, &after).is_err(), "{path}");
        }
        for index in 0..4 {
            for field in ["clock_journal_id", "inventory_clock_journal_id"] {
                let mut after = before.clone();
                after["recovery"]["portfolio"][index][field] = json!("unacknowledged clock");
                assert!(same_original(&before, &after).is_err(), "{index}/{field}");
            }
        }
    }


    fn portfolio_with_clocks(
        first: u32,
        count: usize,
        order: bool,
        inventory: bool,
    ) -> Value {
        let mut reference = portfolio_reference(first, count);
        if !order && !inventory {
            return reference;
        }
        let mut clocks = Vec::new();
        for index in 0..count {
            let step = first + index as u32;
            let pin = |family: &str| json!({
                "journal_id": Digest32::of_bytes(format!("original {family} {step}").as_bytes()).to_hex(),
                "head": Digest32::of_bytes(format!("retained {family} head {step}").as_bytes()).to_hex(),
                "authority_tick_floor": 120 + index as u64,
                "high_source_sequence": 10 + index as u64,
            });
            let clock = order.then(|| pin("clock"));
            let inventory_clock = inventory.then(|| pin("inventory_clock"));
            reference["recovery"]["portfolio"][index]["clock_journal_id"] =
                clock.as_ref().map_or(Value::Null, |pin| pin["journal_id"].clone());
            reference["recovery"]["portfolio"][index]["inventory_clock_journal_id"] =
                inventory_clock.as_ref().map_or(Value::Null, |pin| pin["journal_id"].clone());
            clocks.push(json!({
                "step_id": step, "clock": clock, "inventory_clock": inventory_clock,
            }));
        }
        reference["portfolio_clocks"] = json!(clocks);
        for family in ["clock", "inventory_clock"] {
            reference[family] = reference["portfolio_clocks"][0][family].clone();
            let field = format!("{family}_journal_id");
            reference["recovery"][&field] = reference["recovery"]["portfolio"][0][&field].clone();
        }
        reference
    }

    fn mirror_first_pin(reference: &mut Value, index: usize, family: &str) {
        if index == 0 {
            reference[family] = reference["portfolio_clocks"][0][family].clone();
            let field = format!("{family}_journal_id");
            reference["recovery"][&field] = reference["recovery"]["portfolio"][0][&field].clone();
        }
    }

    #[test]
    fn portfolio_clock_publication_and_each_original_frontier_remain_monotone() -> Result<()> {
        for first in [0, 1] {
            for count in 1..=4 {
                for (order, inventory) in [(true, false), (false, true), (true, true)] {
                    let mut prior = portfolio_with_clocks(first, count, order, inventory);
                    let mut preplan = prior.clone();
                    preplan["recovery"] = Value::Null;
                    preplan.as_object_mut().ok_or_else(portfolio_conflict)?
                        .remove("portfolio_clocks");
                    same_original(&preplan, &prior)?;
                    for index in (0..count).rev() {
                        let mut after = prior.clone();
                        for (family, enabled) in [("clock", order), ("inventory_clock", inventory)] {
                            if !enabled {
                                continue;
                            }
                            after["portfolio_clocks"][index][family]["head"] =
                                json!(Digest32::of_bytes(format!("descendant {family} {index}").as_bytes()).to_hex());
                            after["portfolio_clocks"][index][family]["authority_tick_floor"] =
                                json!(140 + index as u64);
                            after["portfolio_clocks"][index][family]["high_source_sequence"] =
                                json!(20 + index as u64);
                            mirror_first_pin(&mut after, index, family);
                        }
                        after["recovery"]["portfolio"][index]["goal_history_present"] = json!(true);
                        if index == 0 {
                            after["recovery"]["goal_history_present"] = json!(true);
                        }
                        same_original(&prior, &after)?;
                        assert!(same_original(&after, &prior).is_err());
                        prior = after;
                    }
                }
            }
        }
        Ok(())
    }

    #[test]
    fn inactive_portfolio_clock_counters_cannot_hide_behind_a_higher_parent_floor() -> Result<()> {
        let before = portfolio_with_clocks(1, 4, true, true);
        same_original(&before, &before)?;
        for index in 0..4 {
            for family in ["clock", "inventory_clock"] {
                for counter in ["authority_tick_floor", "high_source_sequence"] {
                    let mut after = before.clone();
                    after["portfolio_clocks"][index][family][counter] = json!(0);
                    mirror_first_pin(&mut after, index, family);
                    // Both global summaries still have the larger tick 150.
                    assert_eq!(tick(&before), tick(&after));
                    assert!(same_original(&before, &after).is_err(), "{index}/{family}/{counter}");
                }
            }
        }
        Ok(())
    }

    #[test]
    fn portfolio_clock_ids_and_enabled_families_are_fixed_after_publication() -> Result<()> {
        let before = portfolio_with_clocks(0, 4, true, true);
        for index in 0..4 {
            for family in ["clock", "inventory_clock"] {
                let mut after = before.clone();
                let replacement = json!(Digest32::of_bytes(b"replacement clock journal").to_hex());
                let field = format!("{family}_journal_id");
                after["portfolio_clocks"][index][family]["journal_id"] = replacement.clone();
                after["recovery"]["portfolio"][index][&field] = replacement;
                mirror_first_pin(&mut after, index, family);
                // The new descriptor is internally consistent, but it names
                // a replacement owner instead of the retained original.
                same_original(&after, &after)?;
                assert!(same_original(&before, &after).is_err(), "{index}/{family}");
            }
        }
        for (order, inventory) in [(false, false), (true, false), (false, true)] {
            let changed = portfolio_with_clocks(0, 4, order, inventory);
            same_original(&changed, &changed)?;
            assert!(same_original(&before, &changed).is_err());
            assert!(same_original(&changed, &before).is_err());
        }
        Ok(())
    }

    #[test]
    fn portfolio_clock_vector_requires_every_slot_and_exact_first_slot_summaries() {
        let before = portfolio_with_clocks(0, 4, true, true);
        let edits: [fn(&mut Value); 9] = [
            |v| v["portfolio_clocks"] = Value::Null,
            |v| v["portfolio_clocks"] = json!([]),
            |v| v["portfolio_clocks"] = json!([v["portfolio_clocks"][0].clone()]),
            |v| v["portfolio_clocks"][3]["step_id"] = json!(2),
            |v| v["portfolio_clocks"][2]["clock"] = Value::Null,
            |v| v["portfolio_clocks"][2]["inventory_clock"] = Value::Null,
            |v| v["portfolio_clocks"][1]["unknown"] = json!(true),
            |v| v["clock"] = v["portfolio_clocks"][1]["clock"].clone(),
            |v| v["inventory_clock"] = v["portfolio_clocks"][3]["inventory_clock"].clone(),
        ];
        for (index, edit) in edits.into_iter().enumerate() {
            let mut after = before.clone();
            edit(&mut after);
            assert!(same_original(&before, &after).is_err(), "{index}");
        }
        let mut scalar = before.clone();
        scalar["recovery"]["portfolio"] = Value::Null;
        assert!(same_original(&scalar, &scalar).is_err());
        let mut preplan = before;
        preplan["recovery"] = Value::Null;
        assert!(same_original(&preplan, &preplan).is_err());
    }

    #[test]
    fn portfolio_clock_pins_are_closed_canonical_and_cannot_alias_another_owner() {
        let before = portfolio_with_clocks(1, 4, true, true);
        let edits: [fn(&mut Value); 8] = [
            |v| v["portfolio_clocks"][2]["clock"]["head"] = Value::Null,
            |v| v["portfolio_clocks"][2]["clock"]["head"] = json!("ab".repeat(31)),
            |v| v["portfolio_clocks"][2]["clock"]["head"] = json!("AB".repeat(32)),
            |v| v["portfolio_clocks"][2]["clock"]["head"] = json!("00".repeat(32)),
            |v| v["portfolio_clocks"][2]["clock"]["authority_tick_floor"] =
                json!(dfmcp_adapter::bounded_run::MAX_NATIVE_TICK + 1),
            |v| v["portfolio_clocks"][2]["inventory_clock"]["high_source_sequence"] = Value::Null,
            |v| v["portfolio_clocks"][2]["inventory_clock"]["review_available"] = json!(true),
            |v| v["portfolio_clocks"][2]["inventory_clock"]["journal_id"] = json!(17),
        ];
        for (index, edit) in edits.into_iter().enumerate() {
            let mut after = before.clone();
            edit(&mut after);
            assert!(same_original(&after, &after).is_err(), "{index}");
        }
        for family in ["clock", "inventory_clock"] {
            let mut duplicate = before.clone();
            let id = duplicate["portfolio_clocks"][0]["clock"]["journal_id"].clone();
            duplicate["portfolio_clocks"][3][family]["journal_id"] = id.clone();
            duplicate["recovery"]["portfolio"][3][format!("{family}_journal_id")] = id;
            assert!(same_original(&duplicate, &duplicate).is_err(), "{family}");
        }
    }

    #[test]
    fn first_portfolio_publication_cannot_introduce_a_previously_unconfigured_clock() {
        let after = portfolio_with_clocks(0, 2, true, true);
        let mut before = after.clone();
        before["recovery"] = Value::Null;
        if let Some(object) = before.as_object_mut() {
            object.remove("portfolio_clocks");
        }
        for family in ["clock", "inventory_clock"] {
            let mut absent = before.clone();
            absent[family] = Value::Null;
            assert!(same_original(&absent, &after).is_err(), "{family}");
        }
    }

    #[test]
    fn parent_tick_includes_every_inactive_portfolio_clock() {
        let before = portfolio_with_clocks(0, 4, true, true);
        assert_eq!(tick(&before), 150);
        for index in 0..4 {
            for family in ["clock", "inventory_clock"] {
                let mut after = before.clone();
                after["portfolio_clocks"][index][family]["authority_tick_floor"] = json!(777);
                assert_eq!(tick(&after), 777, "{index}/{family}");
            }
        }
    }

    #[test]
    fn scalar_child_clock_continuity_remains_compatible() -> Result<()> {
        let before = json!({
            "recovery": {
                "plan_digest":"scalar","goal_history_present":true,
                "clock_journal_id":"original-clock",
            },
            "clock": {
                "journal_id":"original-clock","head":"ack",
                "authority_tick_floor":150,"high_source_sequence":9
            }
        });
        let mut after = before.clone();
        after["clock"]["head"] = json!("descendant");
        after["clock"]["authority_tick_floor"] = json!(151);
        after["clock"]["high_source_sequence"] = json!(10);
        same_original(&before, &after)?;
        after["clock"]["high_source_sequence"] = json!(8);
        assert!(same_original(&before, &after).is_err());
        Ok(())
    }
}
