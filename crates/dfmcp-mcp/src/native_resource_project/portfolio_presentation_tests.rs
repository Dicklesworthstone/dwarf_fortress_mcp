//! Complete portfolio roots, observations and joint no-work proof stay attached.
use super::*;
use crate::live_production_runtime::{ProjectStockGoal, StockTargetsRequest};

fn fixture(count: usize, assigning: bool) -> Value {
    let (_, mut child) = stock_watch::tests::fixture();
    let scalar = child["stock_target"].clone();
    let mut requested = Vec::new();
    let mut assessments = Vec::new();
    let mut identities = Vec::new();
    let mut samples = Vec::new();
    for (index, (recipe, item_type)) in [
        ("wooden_bed", "BED"),
        ("wooden_chair", "CHAIR"),
        ("wooden_door", "DOOR"),
        ("wooden_table", "TABLE"),
    ]
    .into_iter()
    .take(count)
    .enumerate()
    {
        let minimum = 10 + index;
        let maximum = 1 + index;
        let witness = (index + 1).to_string().repeat(64);
        requested.push(json!({
            "recipe":recipe,"minimum_stack_units":minimum,"max_order_amount":maximum
        }));
        let mut assessment = scalar.clone();
        assessment["recipe"] = json!(recipe);
        assessment["item_type"] = json!(item_type);
        assessment["target_stack_units"] = json!(minimum);
        assessment["max_order_amount"] = json!(maximum);
        assessment["assessment_witness"] = json!(witness);
        assessment["labor_assignment_requested"] = json!(assigning);
        assessments.push(assessment);
        identities.push(json!({
            "recipe":recipe,"item_type":item_type,"target_stack_units":minimum,
            "assessment_witness":witness
        }));
        samples.push(json!({
            "recipe":recipe,"item_type":item_type,"target_stack_units":minimum,
            "observed_stack_units":20,"current_truth":"true"
        }));
    }
    child["stock_targets"] = json!({
        "schema":"dfmcp.stock-targets/1","status":"already_satisfied","targets":assessments,
        "baseline_anchor":scalar["baseline_anchor"],"source_digest":scalar["source_digest"],
        "source_generation":scalar["source_generation"],"source_counters":scalar["source_counters"],
        "assessment_witness":"9".repeat(64),"consumed_tick_floor":140,
        "work_required":false,"mutation_planned":false,"labor_assignment_requested":assigning,
        "labor_assignment_planned":false,"worker_prerequisite_checked":false,
        "satisfied_at_anchor":true,"current_inventory_proven":false,
    });
    child["stock_target"] = Value::Null;
    let watch = &mut child["no_work_stock_goal"];
    watch["schema"] = json!("dfmcp.stock-targets-watch/1");
    watch["scope"] = json!("original_no_work_inventory_portfolio");
    watch["assessment_witness"] = json!("9".repeat(64));
    watch["targets"] = json!(identities);
    watch["sample"]["targets"] = json!(samples);
    if let Some(object) = watch.as_object_mut() {
        object.remove("recipe");
        object.remove("item_type");
        object.remove("target_stack_units");
    }
    if let Some(object) = watch["sample"].as_object_mut() {
        object.remove("observed_stack_units");
    }
    json!({
        "ok":true,"mode":"control","stage":"production",
        "project_cancelled":false,"original_goal_proven":true,
        "original":{"goal":{
            "targets":requested,"deadline_tick":200,"poll_interval_ticks":10,
            "stable_samples":2,"assign_labor":assigning,
        }},
        "current_stock_goal":{
            "original_goal_proven":true,"current_goal_anchor":child["anchor"],
            "no_work_stock_goal":child["no_work_stock_goal"],
            "already_satisfied":child["stock_targets"],
        },
        "child":child,
    })
}

fn refresh_parent(result: &mut Value) {
    result["current_stock_goal"]["no_work_stock_goal"] =
        result["child"]["no_work_stock_goal"].clone();
    let proven = result["child"]["original_goal_proven"] == true;
    result["current_stock_goal"]["original_goal_proven"] = json!(proven);
    result["current_stock_goal"]["current_goal_anchor"] = if proven {
        result["child"]["anchor"].clone()
    } else {
        Value::Null
    };
    result["original_goal_proven"] = json!(proven);
}

fn choices(result: &mut Value) {
    result["child"]["observation"] = json!({
        "seal":"f".repeat(64),"workforce_choices":{
            "options":[{"labor":"CARPENTER","enable_candidates":["7"]}],
            "existing_labor_options":[{"labor":"CARPENTER","eligible_enabled_ids":["9"]}],
        }
    });
}

fn rendered(result: Value, reference: Value) -> Value {
    packet(
        "fortress.wait",
        Some(SessionId::new(1)),
        None,
        result,
        reference,
    )
}

#[test]
fn observed_portfolio_request_preserves_all_root_terms_and_selected_labor_mode()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    for count in 1..=4 {
        for assigning in [false, true] {
            let mut result = fixture(count, assigning);
            result["child"]["no_work_stock_goal"] = Value::Null;
            result["current_stock_goal"] = Value::Null;
            choices(&mut result);
            let request = observed_plan(&result).ok_or("missing portfolio recommendation")?;
            assert_eq!(request["kind"], "stock_targets");
            for key in [
                "targets",
                "deadline_tick",
                "poll_interval_ticks",
                "stable_samples",
                "assign_labor",
            ] {
                assert_eq!(request[key], result["original"]["goal"][key], "{key}");
            }
            assert!(request.get("recipe").is_none());
            assert!(request.get("minimum_stack_units").is_none());
            assert!(request.get("max_order_amount").is_none());
            assert_eq!(
                request["units"],
                if assigning {
                    json!(["7"])
                } else {
                    json!(["9"])
                }
            );
            assert_eq!(request["labor"], "CARPENTER");
            let goal: ProjectStockGoal =
                serde_json::from_value(result["original"]["goal"].clone())?;
            goal.validate_targets_request(&StockTargetsRequest::parse(&request.to_string())?)?;
        }
    }
    Ok(())
}

#[test]
fn portfolio_no_work_goal_proves_only_one_complete_joint_sample() {
    for count in 1..=4 {
        for assigning in [false, true] {
            let result = fixture(count, assigning);
            let output = rendered(result.clone(), Value::Null);
            assert_eq!(output["result"]["original_goal_proven"], true);
            assert_eq!(
                output["agent_turn"]["briefing"]["original_goal_proven"],
                true
            );
            assert_eq!(
                output["agent_turn"]["briefing"]["current_goal_anchor"],
                result["child"]["anchor"]
            );
            assert_eq!(
                output["agent_turn"]["briefing"]["no_work_stock_goal"]["targets"],
                result["child"]["no_work_stock_goal"]["targets"]
            );
            assert_eq!(
                output["agent_turn"]["active_work"]["no_work_stock_goal"]["sample"]["targets"]
                    .as_array()
                    .map(Vec::len),
                Some(count)
            );
            assert_eq!(
                output["agent_turn"]["active_work"]["confirmations"],
                json!([])
            );
            assert_eq!(
                output["agent_turn"]["recommendations"][0]["tool"],
                "fortress_explain"
            );
            assert!(render(output).is_some());
        }
    }
}

#[test]
fn pending_portfolio_watch_owns_every_minimum_without_a_replacement_plan_or_review() {
    let mut result = fixture(4, true);
    stock_watch::tests::pending(&mut result["child"]);
    choices(&mut result);
    result["child"]["review"] = json!({"retained":true});
    result["child"]["frontier"] = json!({"review":{"paired_seal":"old-review"}});
    refresh_parent(&mut result);
    let output = rendered(result, Value::Null);
    assert_eq!(output["result"]["original_goal_proven"], false);
    assert!(output["agent_turn"]["briefing"]["current_goal_anchor"].is_null());
    assert_eq!(
        output["agent_turn"]["active_work"]["no_work_stock_goal"]["targets"]
            .as_array()
            .map(Vec::len),
        Some(4)
    );
    assert_eq!(
        output["agent_turn"]["recommendations"][0]["tool"],
        "fortress_wait"
    );
    assert_eq!(
        output["agent_turn"]["recommendations"][0]["arguments"]["scope"],
        "goal"
    );
    assert_eq!(
        output["agent_turn"]["active_work"]["confirmations"],
        json!([])
    );
    assert!(output["result"]["child"]["review"].is_null());
}

#[test]
fn portfolio_no_work_proof_rejects_changed_roots_and_incomplete_current_cohorts() {
    for cause in 0..16 {
        let mut result = fixture(4, true);
        choices(&mut result);
        match cause {
            0 => {
                assert!(
                    result["original"]["goal"]["targets"]
                        .as_array_mut()
                        .expect("targets")
                        .pop()
                        .is_some()
                );
            }
            1 => result["original"]["goal"]["targets"][2]["minimum_stack_units"] = json!(11),
            2 => result["original"]["goal"]["targets"][2]["max_order_amount"] = json!(4),
            3 => result["original"]["goal"]["assign_labor"] = json!(false),
            4 => result["original"]["goal"]["deadline_tick"] = json!(210),
            5 => result["original"]["goal"]["poll_interval_ticks"] = json!(5),
            6 => result["original"]["goal"]["stable_samples"] = json!(1),
            7 => {
                assert!(
                    result["child"]["no_work_stock_goal"]["sample"]["targets"]
                        .as_array_mut()
                        .expect("targets")
                        .pop()
                        .is_some()
                );
            }
            8 => {
                result["child"]["no_work_stock_goal"]["sample"]["targets"][2]["observed_stack_units"] =
                    json!(1)
            }
            9 => {
                result["child"]["no_work_stock_goal"]["sample"]["targets"][2]["observed_stack_units"] =
                    Value::Null
            }
            10 => {
                result["child"]["no_work_stock_goal"]["sample"]["stable_samples_observed"] =
                    json!(1)
            }
            11 => result["current_stock_goal"]["current_goal_anchor"]["tick"] = json!(141),
            12 => result["child"]["no_work_stock_goal"]["goal_witness"] = json!("invalid"),
            13 => result["mode"] = json!("offline"),
            14 => result["project_cancelled"] = json!(true),
            _ => result["ok"] = json!(false),
        }
        let output = rendered(result, Value::Null);
        assert_eq!(output["result"]["original_goal_proven"], false, "{cause}");
        assert_eq!(
            output["agent_turn"]["briefing"]["original_goal_proven"], false,
            "{cause}"
        );
        assert!(
            output["result"]["current_stock_goal"]["current_goal_anchor"].is_null(),
            "{cause}"
        );
        assert!(
            output["result"]["child"]["no_work_stock_goal"]["sample"].is_null(),
            "{cause}"
        );
        assert_eq!(
            output["agent_turn"]["active_work"]["confirmations"],
            json!([]),
            "{cause}"
        );
        assert!(
            !matches!(
                output["agent_turn"]["recommendations"][0]["tool"].as_str(),
                Some("fortress_plan" | "fortress_commit")
            ),
            "{cause}"
        );
    }
}

#[test]
fn portfolio_stock_loss_or_unknown_withdraws_current_proof_without_forgetting_achievement() {
    for (count, truth) in [(json!(11), "false"), (Value::Null, "unknown")] {
        let mut result = fixture(4, true);
        let child = &mut result["child"];
        child["anchor"]["tick"] = json!(210);
        child["anchor"]["sequence"] = json!(10);
        child["observation_tick_floor"] = json!(210);
        child["original_goal_proven"] = json!(false);
        let anchor = child["anchor"].clone();
        let watch = &mut child["no_work_stock_goal"];
        watch["consumed_tick_floor"] = json!(210);
        watch["original_goal_proven"] = json!(false);
        watch["current_inventory_proven"] = json!(false);
        watch["sample"]["anchor"] = anchor;
        watch["sample"]["deadline_reached"] = json!(true);
        watch["sample"]["current_truth"] = json!(truth);
        watch["sample"]["targets"][2]["observed_stack_units"] = count.clone();
        watch["sample"]["targets"][2]["current_truth"] = json!(truth);
        refresh_parent(&mut result);
        let output = rendered(result, Value::Null);
        let sample = &output["result"]["current_stock_goal"]["no_work_stock_goal"]["sample"];
        assert_eq!(output["result"]["original_goal_proven"], false);
        assert_eq!(sample["current_truth"], truth);
        assert_eq!(sample["targets"][2]["observed_stack_units"], count);
        assert_eq!(sample["achieved_at_tick"], 140);
        assert_eq!(sample["expired"], false);
        assert_eq!(
            output["agent_turn"]["recommendations"][0]["tool"],
            "fortress_wait"
        );
        assert_eq!(
            output["agent_turn"]["active_work"]["confirmations"],
            json!([])
        );
    }
}

#[test]
fn retained_portfolio_custody_keeps_all_targets_but_restores_no_sample_or_confirmation() {
    let result = fixture(4, false);
    let reference = json!({
        "original":result["original"],
        "child_reference":result["child"],
        "retained_stage":{"stage":"production","checkpoint":{
            "production":result["child"],
            "production_intent":{"recovery_reference":result["child"]},
        }},
    });
    let output = failure(
        "fortress.wait",
        Some(SessionId::new(1)),
        None,
        super::super::error(
            dfmcp_core::ErrorCode::BudgetExceeded,
            "sample publication refused",
        ),
        reference,
    );
    for pointer in [
        "/retained_reference/child_reference",
        "/retained_reference/retained_stage/checkpoint/production",
        "/retained_reference/retained_stage/checkpoint/production_intent/recovery_reference",
    ] {
        let child = output.pointer(pointer).expect("retained original");
        assert_eq!(
            child["no_work_stock_goal"]["targets"],
            result["child"]["no_work_stock_goal"]["targets"]
        );
        assert_eq!(
            child["stock_targets"]["targets"],
            result["child"]["stock_targets"]["targets"]
        );
        assert!(child["no_work_stock_goal"]["sample"].is_null());
        assert_eq!(child["no_work_stock_goal"]["original_goal_proven"], false);
        assert_eq!(child["original_goal_proven"], false);
    }
    assert_eq!(
        output["agent_turn"]["briefing"]["original_goal"],
        result["original"]["goal"]
    );
    assert_eq!(
        output["agent_turn"]["briefing"]["original_goal_proven"],
        false
    );
    assert_eq!(
        output["agent_turn"]["active_work"]["confirmations"],
        json!([])
    );
}
