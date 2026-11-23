//! Bounded presentation of one original no-work stock goal. A retained sizing
//! census cannot stand in for a source-qualified temporal sample.
use super::{bounded, compact_anchor, digest};
use serde_json::{Value, json};

#[path = "live_production_presentation_stock_targets_watch.rs"]
mod portfolio;

const MAX_SPAN: u64 = dfmcp_intent::effects::MAX_DEFAULT_OBLIGATION_TICKS;

fn counters(value: &Value) -> Option<[u64; 3]> {
    let rows = value.as_array()?;
    if rows.len() != 3 {
        return None;
    }
    let values = [rows[0].as_u64()?, rows[1].as_u64()?, rows[2].as_u64()?];
    values
        .iter()
        .all(|value| *value <= u64::from(u32::MAX))
        .then_some(values)
}

fn valid_anchor(value: &Value) -> bool {
    bounded(&value["fortress_id"], 20).is_some_and(|raw| {
        raw.parse::<u64>()
            .is_ok_and(|id| id != 0 && id.to_string() == raw)
    }) && value["epoch"].is_u64()
        && value["sequence"].is_u64()
        && value["tick"].is_u64()
        && digest(&value["state_hash"]).is_some()
}

fn optional_u64(value: &Value) -> Option<Option<u64>> {
    if value.is_null() {
        Some(None)
    } else {
        value.as_u64().map(Some)
    }
}

fn identity(view: &Value) -> Option<()> {
    let goal = &view["no_work_stock_goal"];
    let original = &view["stock_target"];
    let target = goal["target_stack_units"].as_u64()?;
    let interval = goal["poll_interval_ticks"].as_u64()?;
    let required = goal["stable_samples_required"].as_u64()?;
    let start = original["baseline_anchor"]["tick"].as_u64()?;
    let deadline = goal["deadline_tick"].as_u64()?;
    let span = deadline.checked_sub(start)?;
    let recipe_type = match goal["recipe"].as_str()? {
        "wooden_bed" => "BED",
        "wooden_door" => "DOOR",
        "wooden_table" => "TABLE",
        "wooden_chair" => "CHAIR",
        _ => return None,
    };
    let valid = goal["schema"] == "dfmcp.stock-target-watch/1"
        && goal["scope"] == "original_no_work_inventory_goal"
        && original["schema"] == "dfmcp.stock-target/1"
        && original["status"] == "already_satisfied"
        && view["original_plan"].is_null()
        && (1..=1_000_000).contains(&target)
        && original["target_stack_units"] == target
        && original["baseline_stack_units"]
            .as_u64()
            .is_some_and(|units| units >= target)
        && original["deficit_stack_units"] == 0
        && original["amount"] == 0
        && original["work_required"] == false
        && original["mutation_planned"] == false
        && original["labor_assignment_planned"] == false
        && original["satisfied_at_anchor"] == true
        && goal["recipe"] == original["recipe"]
        && goal["item_type"] == recipe_type
        && goal["item_type"] == original["item_type"]
        && digest(&goal["goal_witness"]).is_some()
        && digest(&goal["assessment_witness"]).is_some()
        && goal["assessment_witness"] == original["assessment_witness"]
        && digest(&original["source_digest"]).is_some()
        && original["source_generation"].is_u64()
        && counters(&original["source_counters"]).is_some()
        && valid_anchor(&original["baseline_anchor"])
        && (1..=MAX_SPAN).contains(&span)
        && (1..=MAX_SPAN).contains(&interval)
        && (1..=64).contains(&required)
        && interval
            .checked_mul(required)
            .is_some_and(|ticks| ticks <= span)
        && goal["cancelled"].is_boolean()
        && goal["source_fenced"].is_boolean()
        && goal["consumed_tick_floor"]
            .as_u64()
            .is_some_and(|tick| tick >= start)
        && goal["native_effect_dispatched"] == false
        && goal["worker_prerequisite_checked"] == false
        && goal["labor_assignment_planned"] == false
        && goal["continuous_maintenance"] == false
        && goal["sample_history_durable"] == false
        && goal["recovery_restores_current_proof"] == false
        && goal["production_attribution"] == "unknown"
        && goal["inventory_semantics"] == "all_observed_type_stack_units";
    valid.then_some(())
}

fn temporal_sample(view: &Value, anchor: &Value, original: &Value, truth: &str) -> Option<Value> {
    let goal = &view["no_work_stock_goal"];
    let sample = &goal["sample"];
    let tick = sample["anchor"]["tick"].as_u64()?;
    let start = original["baseline_anchor"]["tick"].as_u64()?;
    let deadline = goal["deadline_tick"].as_u64()?;
    let interval = goal["poll_interval_ticks"].as_u64()?;
    let required = goal["stable_samples_required"].as_u64()?;
    let observed = sample["stable_samples_observed"].as_u64()?;
    let achieved = optional_u64(&sample["achieved_at_tick"])?;
    let counted = optional_u64(&sample["last_counted_tick"])?;
    let expected_proof = achieved.is_some() && truth == "true";
    let source_counters = counters(&sample["source_counters"])?;
    let original_counters = counters(&original["source_counters"])?;
    let valid = goal["cancelled"] == false
        && goal["source_fenced"] == false
        && valid_anchor(anchor)
        && sample["anchor"] == *anchor
        && view["anchor"] == *anchor
        && sample["anchor"]["fortress_id"] == original["baseline_anchor"]["fortress_id"]
        && sample["anchor"]["epoch"] == original["baseline_anchor"]["epoch"]
        && sample["anchor"]["sequence"].as_u64()?
            >= original["baseline_anchor"]["sequence"].as_u64()?
        && (sample["anchor"]["sequence"] != original["baseline_anchor"]["sequence"]
            || sample["anchor"] == original["baseline_anchor"])
        && tick >= start
        && goal["consumed_tick_floor"] == tick
        && view["observation_tick_floor"]
            .as_u64()
            .is_none_or(|floor| floor <= tick)
        && digest(&sample["source_digest"]).is_some()
        && sample["source_generation"] == original["source_generation"]
        && source_counters
            .iter()
            .zip(original_counters)
            .all(|(new, old)| *new >= old)
        && digest(&sample["witness"]).is_some()
        && sample["current_truth"] == truth
        && sample["continuous"].is_boolean()
        && observed <= required
        && counted.is_none_or(|value| {
            value >= start.saturating_add(interval) && value <= tick && value <= deadline
        })
        && (observed == 0 || counted.is_some())
        && achieved.is_none_or(|value| {
            value >= start.saturating_add(interval.saturating_mul(required))
                && value <= deadline
                && value <= tick
                && counted == Some(value)
                && observed == required
        })
        && (achieved.is_some() || observed < required)
        && sample["deadline_reached"] == (tick >= deadline)
        && sample["expired"] == (achieved.is_none() && tick >= deadline)
        && (achieved.is_some() || tick >= deadline || truth == "true" || observed == 0)
        && goal["original_goal_proven"] == expected_proof
        && goal["current_inventory_proven"] == expected_proof
        && view["original_goal_proven"] == expected_proof
        && view["custody_initialization_may_be_partial"] != true
        && view["parent_goal_observation_fenced"] != true
        && view["clock_invalidates_goal"] != true
        && view["inventory_clock_invalidates_goal"] != true;
    if !valid {
        return None;
    }
    Some(json!({
        "anchor":compact_anchor(anchor),
        "source_digest":digest(&sample["source_digest"]),
        "source_generation":sample["source_generation"].as_u64(),
        "source_counters":source_counters,
        "current_truth":truth,
        "stable_samples_observed":observed,
        "achieved_at_tick":achieved,
        "last_counted_tick":counted,
        "deadline_reached":tick >= deadline,
        "expired":achieved.is_none() && tick >= deadline,
        "continuous":sample["continuous"].as_bool(),
        "witness":digest(&sample["witness"]),
    }))
}

fn current_sample(view: &Value, anchor: &Value) -> Option<Value> {
    let goal = &view["no_work_stock_goal"];
    let count = optional_u64(&goal["sample"]["observed_stack_units"])?;
    let target = goal["target_stack_units"].as_u64()?;
    let truth = match count {
        Some(units) if units >= target => "true",
        Some(_) => "false",
        None => "unknown",
    };
    let mut sample = temporal_sample(view, anchor, &view["stock_target"], truth)?;
    sample["observed_stack_units"] = json!(count);
    Some(sample)
}

/// Current presentation requires an independently accepted runtime view at the
/// exact supplied canonical anchor. Retention always drops the entire sample.
pub(crate) fn summary(view: &Value, anchor: Option<&Value>, allow_current: bool) -> Value {
    if view["no_work_stock_goal"]["schema"] == "dfmcp.stock-targets-watch/1" {
        return portfolio::summary(view, anchor, allow_current);
    }
    if identity(view).is_none() {
        return Value::Null;
    }
    let goal = &view["no_work_stock_goal"];
    let sample = if allow_current {
        anchor.and_then(|anchor| current_sample(view, anchor))
    } else {
        None
    };
    let proven = sample.as_ref().is_some_and(|sample| {
        sample["achieved_at_tick"].is_u64() && sample["current_truth"] == "true"
    });
    json!({
        "schema":"dfmcp.stock-target-watch/1",
        "scope":"original_no_work_inventory_goal",
        "goal_witness":digest(&goal["goal_witness"]),
        "assessment_witness":digest(&goal["assessment_witness"]),
        "recipe":bounded(&goal["recipe"],32),
        "item_type":bounded(&goal["item_type"],16),
        "target_stack_units":goal["target_stack_units"].as_u64(),
        "deadline_tick":goal["deadline_tick"].as_u64(),
        "poll_interval_ticks":goal["poll_interval_ticks"].as_u64(),
        "stable_samples_required":goal["stable_samples_required"].as_u64(),
        "cancelled":goal["cancelled"].as_bool(),
        "source_fenced":goal["source_fenced"].as_bool(),
        "consumed_tick_floor":goal["consumed_tick_floor"].as_u64(),
        "original_goal_proven":proven,
        "current_inventory_proven":proven,
        "sample":sample,
        "fresh_for_current_call":false,
        "native_effect_dispatched":false,
        "worker_prerequisite_checked":false,
        "labor_assignment_planned":false,
        "continuous_maintenance":false,
        "production_attribution":"unknown",
        "inventory_semantics":"all_observed_type_stack_units",
        "sample_history_durable":false,
        "recovery_restores_current_proof":false,
    })
}

/// The retained child and checkpoint describe ownership only. They cannot
/// restore a no-work sample or a current proof on a later packet.
pub(crate) fn withdraw(view: &mut Value) {
    if view.get("no_work_stock_goal").is_some() {
        let retained = summary(view, None, false);
        view["no_work_stock_goal"] = retained;
        view["original_goal_proven"] = json!(false);
        view["current_state_proven"] = json!(false);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::live_production_presentation::{anchor_value, compact, packet};
    use dfmcp_core::{Digest32, FortressId, GameTick, ObservationCursor, SessionId, StateAnchor};

    pub(crate) fn fixture() -> (StateAnchor, Value) {
        let original = StateAnchor {
            fortress_id: FortressId::new(7),
            cursor: ObservationCursor {
                epoch: 2,
                sequence: 7,
            },
            tick: GameTick(120),
            state_hash: Digest32::of_bytes(b"original no-work citizens"),
        };
        let anchor = StateAnchor {
            cursor: ObservationCursor {
                epoch: 2,
                sequence: 9,
            },
            tick: GameTick(140),
            state_hash: Digest32::of_bytes(b"current no-work citizens"),
            ..original
        };
        let assessment = json!({
            "schema":"dfmcp.stock-target/1","status":"already_satisfied",
            "recipe":"wooden_bed","item_type":"BED",
            "baseline_stack_units":20,"target_stack_units":10,
            "deficit_stack_units":0,"amount":0,"max_order_amount":1,
            "baseline_anchor":anchor_value(original),
            "source_digest":"a".repeat(64),"source_generation":601,
            "source_counters":[100,200,300],"assessment_witness":"b".repeat(64),
            "work_required":false,"mutation_planned":false,"satisfied_at_anchor":true,
            "labor_assignment_requested":true,"labor_assignment_planned":false,
            "order_step_id":null,"worker_prerequisite_checked":false,
            "current_inventory_proven":false,
        });
        let goal = json!({
            "schema":"dfmcp.stock-target-watch/1","scope":"original_no_work_inventory_goal",
            "goal_witness":"c".repeat(64),"assessment_witness":"b".repeat(64),
            "recipe":"wooden_bed","item_type":"BED","target_stack_units":10,
            "deadline_tick":200,"poll_interval_ticks":10,"stable_samples_required":2,
            "cancelled":false,"source_fenced":false,"consumed_tick_floor":140,
            "original_goal_proven":true,"current_inventory_proven":true,
            "sample":{
                "anchor":anchor_value(anchor),"source_digest":"d".repeat(64),
                "source_generation":601,"source_counters":[101,200,301],
                "observed_stack_units":20,"current_truth":"true",
                "stable_samples_observed":2,"achieved_at_tick":140,
                "last_counted_tick":140,"deadline_reached":false,"expired":false,
                "continuous":true,"witness":"e".repeat(64),
            },
            "fresh_for_current_call":false,"native_effect_dispatched":false,
            "worker_prerequisite_checked":false,"labor_assignment_planned":false,
            "continuous_maintenance":false,"production_attribution":"unknown",
            "inventory_semantics":"all_observed_type_stack_units",
            "sample_history_durable":false,"recovery_restores_current_proof":false,
        });
        (
            anchor,
            json!({
                "ok":true,"mode":"control","anchor":anchor_value(anchor),
                "observation_tick_floor":140,"stock_target":assessment,
                "no_work_stock_goal":goal,"original_plan":null,"frontier":null,"review":null,
                "observation":null,"production_control_enabled":true,"labor_control_enabled":true,
                "current_parent_goal":null,"current_state_proven":false,"original_goal_proven":true,
            }),
        )
    }

    pub(crate) fn pending(view: &mut Value) {
        view["no_work_stock_goal"]["sample"] = Value::Null;
        view["no_work_stock_goal"]["original_goal_proven"] = json!(false);
        view["no_work_stock_goal"]["current_inventory_proven"] = json!(false);
        view["original_goal_proven"] = json!(false);
    }

    fn render(
        anchor: StateAnchor,
        view: &Value,
        verified: bool,
        mode: &str,
        uncertain: bool,
    ) -> std::result::Result<Value, Box<dyn std::error::Error>> {
        let (raw, overflow) = packet(
            "fortress.wait",
            Some(SessionId::new(1)),
            Some(anchor),
            Some(mode),
            None,
            None,
            view.clone(),
            Some(view),
            verified,
            uncertain,
        );
        assert!(!overflow);
        Ok(serde_json::from_str(&raw)?)
    }

    #[test]
    fn no_work_stock_watch_current_proof_requires_the_exact_qualified_sample()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let (anchor, view) = fixture();
        let output = render(anchor, &view, true, "control", false)?;
        assert_eq!(output["original_goal_proven"], true);
        assert_eq!(
            output["agent_turn"]["briefing"]["original_goal_proven"],
            true
        );
        let watch = &output["no_work_stock_goal"];
        assert_eq!(watch["sample"]["anchor"], anchor_value(anchor));
        assert_eq!(watch["sample"]["stable_samples_observed"], 2);
        assert_eq!(watch["current_inventory_proven"], true);
        assert_eq!(output["stock_target"]["current_inventory_proven"], false);
        assert_eq!(output["agent_turn"]["active_work"]["actions"], json!([]));
        assert_eq!(
            output["agent_turn"]["active_work"]["confirmations"],
            json!([])
        );
        assert!(output["selected_inventory_goal"].is_null());
        assert_eq!(output["current_state_proven"], false);
        for (recipe, item_type) in [
            ("wooden_bed", "BED"),
            ("wooden_door", "DOOR"),
            ("wooden_table", "TABLE"),
            ("wooden_chair", "CHAIR"),
        ] {
            let mut same = view.clone();
            for field in ["stock_target", "no_work_stock_goal"] {
                same[field]["recipe"] = json!(recipe);
                same[field]["item_type"] = json!(item_type);
            }
            assert_eq!(
                summary(&same, Some(&same["anchor"]), true)["original_goal_proven"],
                true
            );
        }
        // Query presentation is pure: it does not count another sample.
        let queried = render(anchor, &view, true, "control", false)?;
        assert_eq!(queried["no_work_stock_goal"]["sample"], watch["sample"]);
        Ok(())
    }

    #[test]
    fn no_work_stock_watch_pending_wait_precedes_workforce_and_plan_guidance()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let (anchor, mut view) = fixture();
        pending(&mut view);
        view["observation"] = json!({
            "seal":"f".repeat(64),"paused":true,"workforce_choices":{
                "options":[{"labor":"CARPENTER"}],
                "existing_labor_options":[{"labor":"CARPENTER"}],
                "next_observation_request":{"units":["7"],"option_offset":8},
                "next_existing_labor_observation_request":{"units":["7"],"existing_labor_offset":8},
            }
        });
        let output = render(anchor, &view, true, "control", false)?;
        assert_eq!(output["original_goal_proven"], false);
        assert!(output["no_work_stock_goal"]["sample"].is_null());
        assert_eq!(
            output["agent_turn"]["recommendations"]
                .as_array()
                .map(Vec::len),
            Some(1)
        );
        assert_eq!(
            output["agent_turn"]["recommendations"][0]["tool"],
            "fortress_wait"
        );
        assert_eq!(
            output["agent_turn"]["active_work"]["confirmations"],
            json!([])
        );
        Ok(())
    }

    #[test]
    fn no_work_stock_watch_preserves_history_without_hiding_current_loss()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let (mut anchor, mut view) = fixture();
        anchor.tick = GameTick(210);
        anchor.cursor.sequence = 10;
        anchor.state_hash = Digest32::of_bytes(b"later no-work citizens");
        view["anchor"] = anchor_value(anchor);
        view["observation_tick_floor"] = json!(210);
        view["no_work_stock_goal"]["consumed_tick_floor"] = json!(210);
        view["no_work_stock_goal"]["original_goal_proven"] = json!(false);
        view["no_work_stock_goal"]["current_inventory_proven"] = json!(false);
        view["original_goal_proven"] = json!(false);
        view["no_work_stock_goal"]["sample"]["anchor"] = anchor_value(anchor);
        view["no_work_stock_goal"]["sample"]["observed_stack_units"] = json!(5);
        view["no_work_stock_goal"]["sample"]["current_truth"] = json!("false");
        view["no_work_stock_goal"]["sample"]["deadline_reached"] = json!(true);
        let lost = render(anchor, &view, true, "control", false)?;
        assert_eq!(
            lost["no_work_stock_goal"]["sample"]["achieved_at_tick"],
            140
        );
        assert_eq!(lost["no_work_stock_goal"]["sample"]["expired"], false);
        assert_eq!(lost["original_goal_proven"], false);
        assert_eq!(
            lost["agent_turn"]["recommendations"][0]["tool"],
            "fortress_wait"
        );
        view["no_work_stock_goal"]["sample"]["observed_stack_units"] = json!(20);
        view["no_work_stock_goal"]["sample"]["current_truth"] = json!("true");
        view["no_work_stock_goal"]["original_goal_proven"] = json!(true);
        view["no_work_stock_goal"]["current_inventory_proven"] = json!(true);
        view["original_goal_proven"] = json!(true);
        assert_eq!(
            render(anchor, &view, true, "control", false)?["original_goal_proven"],
            true
        );
        Ok(())
    }

    #[test]
    fn no_work_stock_watch_forged_or_inconsistent_claims_are_withdrawn()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        for cause in 0..17 {
            let (anchor, mut view) = fixture();
            match cause {
                0 => view["no_work_stock_goal"]["assessment_witness"] = json!("f".repeat(64)),
                1 => view["no_work_stock_goal"]["sample"]["achieved_at_tick"] = Value::Null,
                2 => view["no_work_stock_goal"]["sample"]["stable_samples_observed"] = json!(1),
                3 => view["no_work_stock_goal"]["sample"]["current_truth"] = json!("false"),
                4 => view["no_work_stock_goal"]["sample"]["source_generation"] = json!(602),
                5 => {
                    view["no_work_stock_goal"]["sample"]["source_counters"] = json!([99, 200, 300])
                }
                6 => view["no_work_stock_goal"]["sample"]["anchor"]["epoch"] = json!(3),
                7 => {
                    view["no_work_stock_goal"]["sample"]["anchor"]["state_hash"] =
                        json!("f".repeat(64))
                }
                8 => view["no_work_stock_goal"]["sample"]["achieved_at_tick"] = json!(130),
                9 => view["no_work_stock_goal"]["sample"]["deadline_reached"] = json!(true),
                10 => view["no_work_stock_goal"]["sample"]["witness"] = json!("unbound"),
                11 => view["no_work_stock_goal"]["source_fenced"] = json!(true),
                12 => view["no_work_stock_goal"]["cancelled"] = json!(true),
                13 => view["no_work_stock_goal"]["native_effect_dispatched"] = json!(true),
                14 => view["no_work_stock_goal"]["target_stack_units"] = json!(9),
                15 => view["observation_tick_floor"] = json!(141),
                _ => view["no_work_stock_goal"]["sample"]["achieved_at_tick"] = json!("140"),
            }
            let output = render(anchor, &view, true, "control", false)?;
            assert_eq!(output["original_goal_proven"], false, "{cause}");
            assert_eq!(
                output["agent_turn"]["briefing"]["original_goal_proven"], false,
                "{cause}"
            );
            assert!(output["no_work_stock_goal"]["sample"].is_null(), "{cause}");
            assert_eq!(
                output["agent_turn"]["active_work"]["confirmations"],
                json!([]),
                "{cause}"
            );
        }
        Ok(())
    }

    #[test]
    fn no_work_stock_watch_stale_offline_and_uncertain_views_keep_no_current_sample()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let (anchor, view) = fixture();
        let mut changed = anchor;
        changed.tick = GameTick(141);
        for (at, verified, mode, uncertain) in [
            (anchor, false, "control", false),
            (anchor, true, "offline", false),
            (anchor, true, "control", true),
            (changed, true, "control", false),
        ] {
            let output = render(at, &view, verified, mode, uncertain)?;
            assert_eq!(output["original_goal_proven"], false);
            assert!(output["no_work_stock_goal"]["sample"].is_null());
            assert_eq!(
                output["agent_turn"]["briefing"]["original_goal_proven"],
                false
            );
        }
        Ok(())
    }

    #[test]
    fn no_work_stock_watch_compaction_and_overflow_retain_identity_only()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let (anchor, mut view) = fixture();
        for retained in [compact(&view), compact(&compact(&view))] {
            let watch = &retained["no_work_stock_goal"];
            assert_eq!(
                watch["goal_witness"],
                view["no_work_stock_goal"]["goal_witness"]
            );
            assert_eq!(watch["deadline_tick"], 200);
            assert_eq!(watch["original_goal_proven"], false);
            assert!(watch["sample"].is_null());
            assert_eq!(watch["recovery_restores_current_proof"], false);
        }
        view["extra"] = json!("x".repeat(80_000));
        let (raw, overflow) = packet(
            "fortress.wait",
            Some(SessionId::new(1)),
            Some(anchor),
            Some("control"),
            None,
            None,
            view.clone(),
            Some(&view),
            true,
            false,
        );
        assert!(overflow);
        let output: Value = serde_json::from_str(&raw)?;
        assert_eq!(output["original_goal_proven"], false);
        assert!(output["retained_reference"]["no_work_stock_goal"]["sample"].is_null());
        assert_eq!(
            output["retained_reference"]["no_work_stock_goal"]["goal_witness"],
            view["no_work_stock_goal"]["goal_witness"]
        );
        assert_eq!(
            output["agent_turn"]["briefing"]["original_goal_proven"],
            false
        );
        Ok(())
    }
}
