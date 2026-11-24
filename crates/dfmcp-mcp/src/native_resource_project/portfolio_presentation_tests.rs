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


fn clocked_portfolio_view(first: u32, count: usize, selected: usize, reviewed: &str) -> Value {
    let mut result = fixture(count, first == 1);
    let orders: Vec<_> = (0..count)
        .map(|index| {
            let step = first + index as u32;
            let clock = |scope: &str| {
                let identity = Digest32::of_bytes(format!("portfolio {scope} {step}").as_bytes());
                let is_reviewed = index == selected && scope == reviewed;
                json!({
                    "journal_id":identity.to_hex(),"head":identity.to_hex(),
                    "target_digest":Digest32::of_bytes(format!("target {scope} {step}").as_bytes()).to_hex(),
                    "high_tick":140,"high_source_sequence":index,
                    "pending":is_reviewed,"unresolved":false,"may_be_running":false,
                    "review_available":is_reviewed,"commit_available":false,
                    "run_count":usize::from(is_reviewed),
                    "active_run":if is_reviewed { json!({
                        "key":format!("run {scope} {step}"),"state":"prepared",
                        "review_seal":Digest32::of_bytes(format!("review {scope} {step}").as_bytes()).to_hex(),
                        "native":{"unpause_attempted":false},
                    }) } else { Value::Null },
                })
            };
            json!({"step_id":step,"clock":clock("clock"),"inventory_clock":clock("inventory_clock")})
        })
        .collect();
    result["child"] = json!({
        "original_plan":{"plan_digest":Digest32::of_bytes(b"original portfolio plan").to_hex()},
        "portfolio":{
            "schema":"dfmcp.production-portfolio/1",
            "selected_order_step_id":first + selected as u32,
            "orders":orders,
        },
        "clock":orders[selected]["clock"],
        "inventory_clock":orders[selected]["inventory_clock"],
        "frontier":{"next_ready":null,"review":null},
        "review":null,"original_goal_proven":false,
    });
    result["original_goal_proven"] = json!(false);
    result["current_stock_goal"] = Value::Null;
    result
}

#[test]
fn portfolio_clock_confirmation_names_the_exact_selected_original_step() {
    for first in [0, 1] {
        for count in 1..=4 {
            for selected in 0..count {
                for scope in ["clock", "inventory_clock"] {
                    let result = clocked_portfolio_view(first, count, selected, scope);
                    let expected_seal = result["child"][scope]["active_run"]["review_seal"].clone();
                    let output = rendered(result, Value::Null);
                    let turn = &output["agent_turn"];
                    let confirmations = &turn["active_work"]["confirmations"];
                    assert_eq!(confirmations.as_array().map(Vec::len), Some(1));
                    assert_eq!(confirmations[0]["scope"], scope);
                    assert_eq!(confirmations[0]["order_step_id"], first + selected as u32);
                    assert_eq!(confirmations[0]["review_seal"], expected_seal);
                    assert_eq!(confirmations[0]["restored_from_recovery"], false);
                    assert_eq!(
                        confirmations[0]["complete_evidence_pointer"],
                        format!("/result/child/portfolio/orders/{selected}/{scope}")
                    );
                    let next = &turn["recommendations"][0];
                    assert_eq!(next["tool"], "fortress_commit");
                    assert_eq!(next["arguments"]["scope"], scope);
                    assert_eq!(next["arguments"]["order_step_id"], first + selected as u32);
                    assert_eq!(next["arguments"]["review_seal"], expected_seal);
                    assert_eq!(next["arguments"]["confirmed"], true);
                    assert!(render(output).is_some());
                }
            }
        }
    }
}

#[test]
fn portfolio_clock_review_cannot_borrow_a_missing_duplicate_or_different_selected_row() {
    for scope in ["clock", "inventory_clock"] {
        for cause in 0..10 {
            let mut result = clocked_portfolio_view(1, 4, 3, scope);
            match cause {
                0 => result["child"]["portfolio"]["selected_order_step_id"] = Value::Null,
                1 => result["child"]["portfolio"]["orders"][2]["step_id"] = json!(4),
                2 => {
                    result["child"]["portfolio"]["orders"][3][scope] =
                        result["child"]["portfolio"]["orders"][2][scope].clone()
                }
                3 => result["child"][scope]["journal_id"] = json!("another clock"),
                4 => result["child"]["portfolio"]["schema"] = json!("another portfolio"),
                5 => result["child"]["portfolio"]["selected_order_step_id"] = json!(-1),
                6 => {
                    result["child"]["portfolio"]["selected_order_step_id"] = json!(u64::MAX)
                }
                7 => {
                    assert!(
                        result["child"]["portfolio"]["orders"]
                            .as_array_mut()
                            .expect("orders")
                            .pop()
                            .is_some()
                    );
                }
                8 => {
                    result["child"]["portfolio"]["orders"]
                        .as_array_mut()
                        .expect("orders")
                        .swap(1, 2);
                }
                _ => result["child"]["portfolio"] = Value::Null,
            }
            assert!(review(&result).is_none(), "{scope} {cause}");
            let output = rendered(result, Value::Null);
            assert_eq!(
                output["agent_turn"]["active_work"]["confirmations"],
                json!([]),
                "{scope} {cause}"
            );
            assert_ne!(
                output["agent_turn"]["recommendations"][0]["tool"],
                "fortress_commit",
                "{scope} {cause}"
            );
        }
    }
}

#[test]
fn inactive_or_retained_portfolio_clock_reviews_never_become_current_confirmations() {
    for scope in ["clock", "inventory_clock"] {
        let mut inactive = clocked_portfolio_view(1, 4, 0, "");
        let reviewed = clocked_portfolio_view(1, 4, 3, scope);
        inactive["child"]["portfolio"]["orders"][3][scope] =
            reviewed["child"][scope].clone();
        let output = rendered(inactive, Value::Null);
        assert_eq!(output["agent_turn"]["active_work"]["confirmations"], json!([]));
        assert_eq!(output["agent_turn"]["recommendations"][0]["tool"], "fortress_wait");
        assert_eq!(
            output["agent_turn"]["recommendations"][0]["arguments"]["scope"],
            "goal"
        );
        assert!(
            output["agent_turn"]["recommendations"][0]["arguments"]
                .get("order_step_id")
                .is_none()
        );

        for mode in ["offline", "recover"] {
            let mut result = reviewed.clone();
            result["mode"] = json!(mode);
            let output = rendered(result, Value::Null);
            assert_eq!(output["agent_turn"]["active_work"]["confirmations"], json!([]));
            assert_ne!(output["agent_turn"]["recommendations"][0]["tool"], "fortress_commit");
        }

        let reference = json!({
            "original":reviewed["original"],
            "child_reference":reviewed["child"],
            "retained_stage":{"stage":"production","checkpoint":{"production":reviewed["child"]}},
        });
        let failed = failure(
            "fortress.commit",
            Some(SessionId::new(1)),
            None,
            super::super::error(dfmcp_core::ErrorCode::BudgetExceeded, "publication refused"),
            reference,
        );
        assert_eq!(failed["agent_turn"]["active_work"]["confirmations"], json!([]));
        assert_eq!(failed["agent_turn"]["recommendations"][0]["tool"], "fortress_query");
        let actions = failed["agent_turn"]["active_work"]["actions"]
            .as_array()
            .expect("clock custody actions");
        assert_eq!(actions.len(), 8);
        assert!(actions.iter().all(|action| action.get("review_seal").is_none()));
    }
}

#[test]
fn every_inactive_portfolio_clock_remains_visible_with_its_own_evidence_and_uncertainty() {
    let mut result = clocked_portfolio_view(1, 4, 2, "");
    for index in 0..4 {
        for scope in ["clock", "inventory_clock"] {
            let clock = &mut result["child"]["portfolio"]["orders"][index][scope];
            clock["active_run"] = json!({
                "key":format!("retained {scope} {index}"),"state":"prepared",
                "native":{"unpause_attempted":false},
            });
            clock["run_count"] = json!(1);
        }
    }
    result["child"]["portfolio"]["orders"][3]["clock"]["unresolved"] = json!(true);
    result["child"]["portfolio"]["orders"][1]["inventory_clock"]["may_be_running"] = json!(true);
    for scope in ["clock", "inventory_clock"] {
        result["child"][scope] = result["child"]["portfolio"]["orders"][2][scope].clone();
    }
    let output = rendered(result, Value::Null);
    let actions = output["agent_turn"]["active_work"]["actions"]
        .as_array()
        .expect("clock custody actions");
    assert_eq!(actions.len(), 8);
    for index in 0..4 {
        for (offset, scope) in ["clock", "inventory_clock"].into_iter().enumerate() {
            let action = &actions[2 * index + offset];
            assert_eq!(action["scope"], scope);
            assert_eq!(action["order_step_id"], index + 1);
            assert_eq!(action["original_key"], format!("retained {scope} {index}"));
            assert_eq!(action["native_unpause_attempted"], false);
            assert_eq!(action["current_pause_proven"], false);
            assert_eq!(action["automatic_redispatch"], false);
            assert_eq!(
                action["complete_evidence_pointer"],
                format!("/result/child/portfolio/orders/{index}/{scope}")
            );
            let source = output
                .pointer(action["complete_evidence_pointer"].as_str().expect("pointer"))
                .expect("complete source evidence");
            assert_eq!(source["journal_id"], action["journal_id"]);
        }
    }
    let effects = output["agent_turn"]["active_work"]["indeterminate_effects"]
        .as_array()
        .expect("clock uncertainty");
    assert_eq!(effects.len(), 2);
    assert!(effects.iter().any(|effect| {
        effect["scope"] == "clock" && effect["order_step_id"] == 4
    }));
    assert!(effects.iter().any(|effect| {
        effect["scope"] == "inventory_clock" && effect["order_step_id"] == 2
    }));
    assert_eq!(output["agent_turn"]["active_work"]["confirmations"], json!([]));
}

#[test]
fn compact_parent_recovery_keeps_all_portfolio_clock_pins_visible_without_reviews() {
    let result = clocked_portfolio_view(0, 4, 2, "clock");
    let pins: Vec<_> = result["child"]["portfolio"]["orders"]
        .as_array()
        .expect("orders")
        .iter()
        .map(|order| {
            let pin = |scope: &str| json!({
                "journal_id":order[scope]["journal_id"],"head":order[scope]["head"],
                "authority_tick_floor":order[scope]["high_tick"],
                "high_source_sequence":order[scope]["high_source_sequence"],
            });
            json!({"step_id":order["step_id"],"clock":pin("clock"),"inventory_clock":pin("inventory_clock")})
        })
        .collect();
    let reference = json!({
        "original":result["original"],
        "child_reference":{
            "schema":"dfmcp.project-production-recovery/1",
            "clock":pins[0]["clock"],"inventory_clock":pins[0]["inventory_clock"],
            "portfolio_clocks":pins,
        },
        "retained_stage":{"stage":"production"},
    });
    let output = failure(
        "fortress.wait",
        Some(SessionId::new(1)),
        None,
        super::super::error(dfmcp_core::ErrorCode::CorruptLedger, "inactive clock inspection refused"),
        reference,
    );
    let actions = output["agent_turn"]["active_work"]["actions"]
        .as_array()
        .expect("clock custody actions");
    assert_eq!(actions.len(), 8);
    for index in 0..4 {
        for (offset, scope) in ["clock", "inventory_clock"].into_iter().enumerate() {
            let action = &actions[2 * index + offset];
            assert_eq!(action["scope"], scope);
            assert_eq!(action["order_step_id"], index);
            assert_eq!(action["journal_id"], pins[index][scope]["journal_id"]);
            assert_eq!(action["authority_tick_floor"], 140);
            assert_eq!(
                action["complete_evidence_pointer"],
                format!("/retained_reference/child_reference/portfolio_clocks/{index}/{scope}")
            );
            assert!(action.get("review_seal").is_none());
        }
    }
    assert_eq!(output["agent_turn"]["active_work"]["confirmations"], json!([]));
    assert!(render(output).is_some());
}


#[test]
fn fenced_local_parent_query_keeps_inactive_clock_custody_visible() {
    let historical = clocked_portfolio_view(1, 4, 3, "inventory_clock");
    let reference = json!({
        "controller_fenced":true,
        "original":historical["original"],
        "child_reference":historical["child"],
        "retained_stage":{"stage":"production"},
    });
    let result = json!({
        "ok":true,"mode":"control","stage":"production",
        "child":{"local_parent_only":true,"child_custody_currently_verified":false},
    });
    let output = rendered(result, reference);
    let actions = output["agent_turn"]["active_work"]["actions"]
        .as_array()
        .expect("retained clock custody actions");
    assert_eq!(actions.len(), 8);
    for (index, action) in actions.iter().enumerate() {
        assert_eq!(action["order_step_id"], index / 2 + 1);
        assert!(
            action["complete_evidence_pointer"]
                .as_str()
                .is_some_and(|pointer| pointer.starts_with("/retained_reference/child_reference/portfolio/orders/"))
        );
        assert!(output.pointer(action["complete_evidence_pointer"].as_str().expect("pointer")).is_some());
        assert!(action.get("review_seal").is_none());
    }
    assert_eq!(output["agent_turn"]["active_work"]["confirmations"], json!([]));
    assert_eq!(output["agent_turn"]["recommendations"][0]["tool"], "fortress_query");
}

#[test]
fn portfolio_goal_review_and_prefix_or_scalar_clocks_keep_their_existing_scopes() {
    let mut goal = clocked_portfolio_view(1, 4, 2, "");
    goal["child"]["review"] = json!({"current":true});
    goal["child"]["frontier"]["review"] = json!({"paired_seal":"original goal review"});
    let output = rendered(goal, Value::Null);
    assert_eq!(output["agent_turn"]["active_work"]["confirmations"][0]["scope"], "goal");
    assert!(
        output["agent_turn"]["active_work"]["confirmations"][0]
            .get("order_step_id")
            .is_none()
    );
    assert_eq!(output["agent_turn"]["recommendations"][0]["arguments"]["scope"], "goal");
    assert!(
        output["agent_turn"]["recommendations"][0]["arguments"]
            .get("order_step_id")
            .is_none()
    );
    for stage in ["workshop", "production"] {
        let mut result = clocked_portfolio_view(1, 4, 2, "clock");
        result["stage"] = json!(stage);
        result["child"]["portfolio"] = Value::Null;
        if stage == "production" {
            result["original"]["goal"] = json!({
                "recipe":"wooden_bed","minimum_stack_units":2,
                "max_order_amount":2,"deadline_tick":200,
            });
        }
        let output = rendered(result, Value::Null);
        assert_eq!(output["agent_turn"]["active_work"]["confirmations"][0]["scope"], "clock");
        assert!(
            output["agent_turn"]["active_work"]["confirmations"][0]
                .get("order_step_id")
                .is_none()
        );
        assert_eq!(output["agent_turn"]["recommendations"][0]["tool"], "fortress_commit");
        assert!(
            output["agent_turn"]["recommendations"][0]["arguments"]
                .get("order_step_id")
                .is_none()
        );
    }
}
