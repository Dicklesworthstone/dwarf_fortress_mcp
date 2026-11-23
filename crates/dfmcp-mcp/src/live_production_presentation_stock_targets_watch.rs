//! Complete-target validation for a no-work portfolio. One shared temporal
//! certificate must cover every original minimum at the same current anchor.
use super::{MAX_SPAN, counters, optional_u64, temporal_sample, valid_anchor};
use crate::live_production_presentation::digest;
use serde_json::{Value, json};

fn item_type(recipe: &str) -> Option<&'static str> {
    match recipe {
        "wooden_bed" => Some("BED"),
        "wooden_chair" => Some("CHAIR"),
        "wooden_door" => Some("DOOR"),
        "wooden_table" => Some("TABLE"),
        _ => None,
    }
}

fn identity(view: &Value) -> Option<Vec<Value>> {
    let goal = &view["no_work_stock_goal"];
    let original = &view["stock_targets"];
    let rows = goal["targets"].as_array()?;
    let originals = original["targets"].as_array()?;
    if rows.is_empty() || rows.len() > 4 || rows.len() != originals.len() {
        return None;
    }
    let start = original["baseline_anchor"]["tick"].as_u64()?;
    let deadline = goal["deadline_tick"].as_u64()?;
    let span = deadline.checked_sub(start)?;
    let interval = goal["poll_interval_ticks"].as_u64()?;
    let required = goal["stable_samples_required"].as_u64()?;
    let valid = goal["schema"] == "dfmcp.stock-targets-watch/1"
        && goal["scope"] == "original_no_work_inventory_portfolio"
        && original["schema"] == "dfmcp.stock-targets/1"
        && original["status"] == "already_satisfied"
        && original["work_required"] == false
        && original["mutation_planned"] == false
        && original["labor_assignment_planned"] == false
        && original["worker_prerequisite_checked"] == false
        && original["satisfied_at_anchor"] == true
        && view["original_plan"].is_null()
        && view["stock_target"].is_null()
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
    if !valid {
        return None;
    }
    let mut previous = None;
    let mut targets = Vec::with_capacity(rows.len());
    for (row, original_row) in rows.iter().zip(originals) {
        let recipe = row["recipe"].as_str()?;
        let kind = item_type(recipe)?;
        let minimum = row["target_stack_units"].as_u64()?;
        if previous.is_some_and(|last| last >= recipe)
            || !(1..=1_000_000).contains(&minimum)
            || original_row["schema"] != "dfmcp.stock-target/1"
            || original_row["status"] != "already_satisfied"
            || row["item_type"] != kind
            || original_row["recipe"] != recipe
            || original_row["item_type"] != kind
            || original_row["target_stack_units"] != minimum
            || original_row["baseline_stack_units"].as_u64()? < minimum
            || original_row["deficit_stack_units"] != 0
            || original_row["amount"] != 0
            || !original_row["order_step_id"].is_null()
            || original_row["work_required"] != false
            || original_row["mutation_planned"] != false
            || original_row["labor_assignment_planned"] != false
            || original_row["satisfied_at_anchor"] != true
            || digest(&row["assessment_witness"]).is_none()
            || row["assessment_witness"] != original_row["assessment_witness"]
            || original_row["baseline_anchor"] != original["baseline_anchor"]
            || original_row["source_digest"] != original["source_digest"]
            || original_row["source_generation"] != original["source_generation"]
            || original_row["source_counters"] != original["source_counters"]
        {
            return None;
        }
        previous = Some(recipe);
        targets.push(json!({
            "recipe":recipe,"item_type":kind,"target_stack_units":minimum,
            "assessment_witness":digest(&row["assessment_witness"]),
        }));
    }
    Some(targets)
}

fn current_sample(view: &Value, anchor: &Value, targets: &[Value]) -> Option<Value> {
    let rows = view["no_work_stock_goal"]["sample"]["targets"].as_array()?;
    if rows.len() != targets.len() {
        return None;
    }
    let mut current = Vec::with_capacity(rows.len());
    let mut any_false = false;
    let mut any_unknown = false;
    for (row, target) in rows.iter().zip(targets) {
        if row["recipe"] != target["recipe"]
            || row["item_type"] != target["item_type"]
            || row["target_stack_units"] != target["target_stack_units"]
        {
            return None;
        }
        let count = optional_u64(&row["observed_stack_units"])?;
        let minimum = target["target_stack_units"].as_u64()?;
        let truth = match count {
            Some(value) if value >= minimum => "true",
            Some(_) => "false",
            None => "unknown",
        };
        if row["current_truth"] != truth {
            return None;
        }
        any_false |= truth == "false";
        any_unknown |= truth == "unknown";
        current.push(json!({
            "recipe":target["recipe"],"item_type":target["item_type"],
            "target_stack_units":minimum,"observed_stack_units":count,"current_truth":truth,
        }));
    }
    let truth = if any_false {
        "false"
    } else if any_unknown {
        "unknown"
    } else {
        "true"
    };
    let mut sample = temporal_sample(view, anchor, &view["stock_targets"], truth)?;
    sample["targets"] = json!(current);
    Some(sample)
}

pub(super) fn summary(view: &Value, anchor: Option<&Value>, allow_current: bool) -> Value {
    let Some(targets) = identity(view) else {
        return Value::Null;
    };
    let goal = &view["no_work_stock_goal"];
    let sample = if allow_current {
        anchor.and_then(|anchor| current_sample(view, anchor, &targets))
    } else {
        None
    };
    let proven = sample.as_ref().is_some_and(|sample| {
        sample["achieved_at_tick"].is_u64() && sample["current_truth"] == "true"
    });
    json!({
        "schema":"dfmcp.stock-targets-watch/1",
        "scope":"original_no_work_inventory_portfolio",
        "goal_witness":digest(&goal["goal_witness"]),
        "assessment_witness":digest(&goal["assessment_witness"]),
        "targets":targets,
        "deadline_tick":goal["deadline_tick"].as_u64(),
        "poll_interval_ticks":goal["poll_interval_ticks"].as_u64(),
        "stable_samples_required":goal["stable_samples_required"].as_u64(),
        "cancelled":goal["cancelled"].as_bool(),
        "source_fenced":goal["source_fenced"].as_bool(),
        "consumed_tick_floor":goal["consumed_tick_floor"].as_u64(),
        "original_goal_proven":proven,"current_inventory_proven":proven,
        "sample":sample,"fresh_for_current_call":false,
        "native_effect_dispatched":false,"worker_prerequisite_checked":false,
        "labor_assignment_planned":false,"continuous_maintenance":false,
        "production_attribution":"unknown","inventory_semantics":"all_observed_type_stack_units",
        "sample_history_durable":false,"recovery_restores_current_proof":false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live_production_presentation::{anchor_value, compact, packet};
    use dfmcp_core::{Digest32, GameTick, SessionId, StateAnchor};

    fn fixture(count: usize) -> (StateAnchor, Value) {
        let (anchor, mut view) = super::super::tests::fixture();
        let original = view["stock_target"].clone();
        let mut originals = Vec::new();
        let mut identities = Vec::new();
        let mut samples = Vec::new();
        for (index, (recipe, kind)) in [
            ("wooden_bed", "BED"),
            ("wooden_chair", "CHAIR"),
            ("wooden_door", "DOOR"),
            ("wooden_table", "TABLE"),
        ]
        .into_iter()
        .take(count)
        .enumerate()
        {
            let witness = (index + 1).to_string().repeat(64);
            let mut row = original.clone();
            row["recipe"] = json!(recipe);
            row["item_type"] = json!(kind);
            row["assessment_witness"] = json!(witness);
            row["item_rows"] = json!(4);
            originals.push(row);
            identities.push(json!({
                "recipe":recipe,"item_type":kind,"target_stack_units":10,
                "assessment_witness":witness,
            }));
            samples.push(json!({
                "recipe":recipe,"item_type":kind,"target_stack_units":10,
                "observed_stack_units":20,"current_truth":"true",
            }));
        }
        view["stock_targets"] = json!({
            "schema":"dfmcp.stock-targets/1","status":"already_satisfied",
            "targets":originals,"baseline_anchor":original["baseline_anchor"],
            "source_digest":original["source_digest"],"source_generation":601,
            "source_counters":[100,200,300],"item_rows":4,
            "assessment_witness":"9".repeat(64),"consumed_tick_floor":140,
            "revalidation_failed":false,"last_check_witness":null,
            "work_required":false,"mutation_planned":false,
            "worker_prerequisite_checked":false,"satisfied_at_anchor":true,
            "labor_assignment_requested":true,"labor_assignment_planned":false,
            "fresh_for_current_call":false,"current_inventory_proven":false,
            "all_target_minima_retained":true,"continuous_maintenance":false,
            "native_recipe_feasibility_proven":false,"production_attribution":"unknown",
            "inventory_semantics":"all_observed_type_stack_units",
        });
        view["stock_target"] = Value::Null;
        let goal = &mut view["no_work_stock_goal"];
        goal["schema"] = json!("dfmcp.stock-targets-watch/1");
        goal["scope"] = json!("original_no_work_inventory_portfolio");
        goal["assessment_witness"] = json!("9".repeat(64));
        goal["targets"] = json!(identities);
        goal["sample"]["targets"] = json!(samples);
        if let Some(object) = goal.as_object_mut() {
            object.remove("recipe");
            object.remove("item_type");
            object.remove("target_stack_units");
        }
        if let Some(object) = goal["sample"].as_object_mut() {
            object.remove("observed_stack_units");
        }
        (anchor, view)
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
    fn no_work_stock_portfolio_displays_one_complete_joint_sample()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        for count in 1..=4 {
            let (anchor, view) = fixture(count);
            let output = render(anchor, &view, true, "control", false)?;
            assert_eq!(output["original_goal_proven"], true, "{count}");
            assert_eq!(
                output["agent_turn"]["briefing"]["original_goal_proven"],
                true
            );
            let goal = &output["no_work_stock_goal"];
            assert_eq!(goal["sample"]["anchor"], anchor_value(anchor));
            assert_eq!(
                goal["sample"]["targets"].as_array().map(Vec::len),
                Some(count)
            );
            assert_eq!(goal["sample"]["stable_samples_observed"], 2);
            assert_eq!(goal["current_inventory_proven"], true);
            assert_eq!(output["stock_targets"]["current_inventory_proven"], false);
            assert_eq!(output["agent_turn"]["active_work"]["actions"], json!([]));
            assert_eq!(
                output["agent_turn"]["active_work"]["confirmations"],
                json!([])
            );
            assert_eq!(output["current_state_proven"], false);
            let repeated = render(anchor, &view, true, "control", false)?;
            assert_eq!(repeated["no_work_stock_goal"], *goal);
        }
        Ok(())
    }

    #[test]
    fn no_work_stock_portfolio_never_drops_or_substitutes_an_original_minimum()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        for cause in 0..13 {
            let (anchor, mut view) = fixture(4);
            match cause {
                0 => view["no_work_stock_goal"]["targets"]
                    .as_array_mut()
                    .ok_or("fixture must contain complete target rows")?
                    .pop()
                    .map(|_| ())
                    .ok_or("fixture must contain complete target rows")?,
                1 => view["stock_targets"]["targets"]
                    .as_array_mut()
                    .ok_or("fixture must contain complete target rows")?
                    .pop()
                    .map(|_| ())
                    .ok_or("fixture must contain complete target rows")?,
                2 => {
                    view["no_work_stock_goal"]["targets"][1] =
                        view["no_work_stock_goal"]["targets"][0].clone()
                }
                3 => view["no_work_stock_goal"]["targets"][1]["target_stack_units"] = json!(9),
                4 => {
                    view["no_work_stock_goal"]["targets"][1]["assessment_witness"] =
                        json!("f".repeat(64))
                }
                5 => view["stock_targets"]["targets"][1]["source_digest"] = json!("f".repeat(64)),
                6 => view["stock_targets"]["targets"][1]["baseline_anchor"]["tick"] = json!(121),
                7 => view["stock_targets"]["targets"][1]["source_generation"] = json!(602),
                8 => {
                    view["stock_targets"]["targets"][1]["source_counters"] = json!([100, 200, 301])
                }
                9 => view["stock_targets"]["targets"][1]["order_step_id"] = json!(0),
                10 => view["stock_targets"]["targets"][1]["mutation_planned"] = json!(true),
                11 => view["stock_targets"]["assessment_witness"] = json!("f".repeat(64)),
                _ => view["no_work_stock_goal"]["targets"]
                    .as_array_mut()
                    .ok_or("fixture must contain complete target rows")?
                    .swap(0, 1),
            }
            let output = render(anchor, &view, true, "control", false)?;
            assert_eq!(output["original_goal_proven"], false, "{cause}");
            assert!(output["no_work_stock_goal"].is_null(), "{cause}");
            assert_eq!(
                output["agent_turn"]["active_work"]["confirmations"],
                json!([])
            );
        }
        Ok(())
    }

    #[test]
    fn no_work_stock_portfolio_rejects_partial_or_inconsistent_current_evidence()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        for cause in 0..21 {
            let (anchor, mut view) = fixture(4);
            match cause {
                0 => view["no_work_stock_goal"]["sample"]["targets"]
                    .as_array_mut()
                    .ok_or("fixture must contain complete target rows")?
                    .pop()
                    .map(|_| ())
                    .ok_or("fixture must contain complete target rows")?,
                1 => {
                    view["no_work_stock_goal"]["sample"]["targets"][1] =
                        view["no_work_stock_goal"]["sample"]["targets"][0].clone()
                }
                2 => {
                    view["no_work_stock_goal"]["sample"]["targets"][1]["observed_stack_units"] =
                        json!(9)
                }
                3 => {
                    view["no_work_stock_goal"]["sample"]["targets"][1]["observed_stack_units"] =
                        Value::Null
                }
                4 => {
                    view["no_work_stock_goal"]["sample"]["targets"][1]["current_truth"] =
                        json!("false")
                }
                5 => {
                    view["no_work_stock_goal"]["sample"]["targets"][1]["target_stack_units"] =
                        json!(9)
                }
                6 => view["no_work_stock_goal"]["sample"]["targets"][1]["item_type"] = json!("BED"),
                7 => view["no_work_stock_goal"]["sample"]["current_truth"] = json!("false"),
                8 => {
                    view["no_work_stock_goal"]["sample"]["source_counters"] = json!([99, 200, 300])
                }
                9 => view["no_work_stock_goal"]["sample"]["source_generation"] = json!(602),
                10 => {
                    view["no_work_stock_goal"]["sample"]["anchor"]["state_hash"] =
                        json!("f".repeat(64))
                }
                11 => view["no_work_stock_goal"]["sample"]["achieved_at_tick"] = json!(130),
                12 => view["no_work_stock_goal"]["sample"]["deadline_reached"] = json!(true),
                13 => view["no_work_stock_goal"]["sample"]["stable_samples_observed"] = json!(1),
                14 => view["no_work_stock_goal"]["sample"]["achieved_at_tick"] = Value::Null,
                15 => view["observation_tick_floor"] = json!(141),
                16 => view["no_work_stock_goal"]["sample"]["last_counted_tick"] = json!(130),
                17 => {
                    view["no_work_stock_goal"]["sample"]["targets"][1]["observed_stack_units"] =
                        json!("20")
                }
                18 => view["no_work_stock_goal"]["source_fenced"] = json!(true),
                19 => view["no_work_stock_goal"]["cancelled"] = json!(true),
                _ => view["no_work_stock_goal"]["sample"]["witness"] = json!("unbound"),
            }
            let output = render(anchor, &view, true, "control", false)?;
            assert_eq!(output["original_goal_proven"], false, "{cause}");
            assert!(output["no_work_stock_goal"]["sample"].is_null(), "{cause}");
            assert_eq!(
                output["agent_turn"]["briefing"]["original_goal_proven"],
                false
            );
            assert_eq!(
                output["agent_turn"]["active_work"]["confirmations"],
                json!([])
            );
        }
        Ok(())
    }

    #[test]
    fn no_work_stock_portfolio_preserves_achievement_but_exposes_loss_and_unknown()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let (mut anchor, mut view) = fixture(4);
        anchor.tick = GameTick(210);
        anchor.cursor.sequence = 10;
        anchor.state_hash = Digest32::of_bytes(b"later joint portfolio sample");
        view["anchor"] = anchor_value(anchor);
        view["observation_tick_floor"] = json!(210);
        view["original_goal_proven"] = json!(false);
        view["no_work_stock_goal"]["consumed_tick_floor"] = json!(210);
        view["no_work_stock_goal"]["original_goal_proven"] = json!(false);
        view["no_work_stock_goal"]["current_inventory_proven"] = json!(false);
        view["no_work_stock_goal"]["sample"]["anchor"] = anchor_value(anchor);
        view["no_work_stock_goal"]["sample"]["deadline_reached"] = json!(true);
        for (count, truth) in [(json!(9), "false"), (Value::Null, "unknown")] {
            view["no_work_stock_goal"]["sample"]["targets"][2]["observed_stack_units"] =
                count.clone();
            view["no_work_stock_goal"]["sample"]["targets"][2]["current_truth"] = json!(truth);
            view["no_work_stock_goal"]["sample"]["current_truth"] = json!(truth);
            let output = render(anchor, &view, true, "control", false)?;
            assert_eq!(output["original_goal_proven"], false);
            let sample = &output["no_work_stock_goal"]["sample"];
            assert_eq!(sample["current_truth"], truth);
            assert_eq!(sample["targets"][2]["observed_stack_units"], count);
            assert_eq!(sample["achieved_at_tick"], 140);
            assert_eq!(sample["expired"], false);
            assert_eq!(
                output["agent_turn"]["recommendations"][0]["tool"],
                "fortress_wait"
            );
        }
        // A definite deficit remains false even if another minimum is Unknown.
        view["no_work_stock_goal"]["sample"]["targets"][0]["observed_stack_units"] = json!(9);
        view["no_work_stock_goal"]["sample"]["targets"][0]["current_truth"] = json!("false");
        view["no_work_stock_goal"]["sample"]["current_truth"] = json!("false");
        let output = render(anchor, &view, true, "control", false)?;
        assert_eq!(
            output["no_work_stock_goal"]["sample"]["current_truth"],
            "false"
        );
        Ok(())
    }

    #[test]
    fn no_work_stock_portfolio_pending_guidance_owns_all_original_targets()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let (anchor, mut view) = fixture(4);
        super::super::tests::pending(&mut view);
        view["observation"] = json!({
            "seal":"f".repeat(64),"paused":true,"workforce_choices":{
                "options":[{"labor":"CARPENTER"}],
                "existing_labor_options":[{"labor":"CARPENTER"}],
                "next_observation_request":{"units":["7"],"option_offset":8},
                "next_existing_labor_observation_request":{"units":["7"],"existing_labor_offset":8},
            }
        });
        let output = render(anchor, &view, true, "control", false)?;
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
        assert_eq!(output["original_goal_proven"], false);
        assert_eq!(
            output["no_work_stock_goal"]["targets"]
                .as_array()
                .map(Vec::len),
            Some(4)
        );
        let text = output["agent_turn"].to_string();
        assert!(text.contains("original_no_work_inventory_portfolio"));
        assert!(!text.contains("original_no_work_inventory_goal"));
        Ok(())
    }

    #[test]
    fn no_work_stock_portfolio_stale_offline_and_unsettled_views_withhold_the_cohort()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let (anchor, view) = fixture(4);
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
        }
        for flag in [
            "clock_invalidates_goal",
            "inventory_clock_invalidates_goal",
            "parent_goal_observation_fenced",
            "custody_initialization_may_be_partial",
        ] {
            let mut changed = view.clone();
            changed[flag] = json!(true);
            let output = render(anchor, &changed, true, "control", false)?;
            assert_eq!(output["original_goal_proven"], false, "{flag}");
            assert!(output["no_work_stock_goal"]["sample"].is_null(), "{flag}");
        }
        Ok(())
    }

    #[test]
    fn no_work_stock_portfolio_compaction_and_overflow_keep_all_identity_without_proof()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let (anchor, mut view) = fixture(4);
        for retained in [compact(&view), compact(&compact(&view))] {
            let goal = &retained["no_work_stock_goal"];
            assert_eq!(goal["targets"], view["no_work_stock_goal"]["targets"]);
            assert_eq!(
                goal["goal_witness"],
                view["no_work_stock_goal"]["goal_witness"]
            );
            assert_eq!(goal["deadline_tick"], 200);
            assert_eq!(goal["original_goal_proven"], false);
            assert!(goal["sample"].is_null());
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
        let retained = &output["retained_reference"]["no_work_stock_goal"];
        assert!(retained["sample"].is_null());
        assert_eq!(retained["targets"], view["no_work_stock_goal"]["targets"]);
        assert_eq!(retained["deadline_tick"], 200);
        Ok(())
    }
}
