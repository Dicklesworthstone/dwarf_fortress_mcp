//! The parent resumes exact original preparations without recreating effects.
use super::*;

fn prepared_fixture(labor: bool) -> (Value, Value) {
    let anchor = json!({
        "fortress_id":"7","epoch":1,"sequence":5,"tick":130,
        "state_hash":"c".repeat(64),
    });
    let digest = "a".repeat(64);
    let child = json!({
        "ok":true,"mode":"control","anchor":anchor,
        "observation_tick_floor":130,
        "original_plan":{
            "anchor":anchor,"plan_digest":digest,"expires_at_tick":300,
            "steps":[{
                "step_id":0,"key":"same-original-key",
                "action":{"kind":if labor {"set_labor"} else {"create_work_order"}},
                "obligation":{"deadline_tick":300},
            }],
        },
        "frontier":{
            "needs_reconciliation":false,
            "entries":[{
                "step_id":0,
                "family":if labor {"workforce/1.17"} else {"work-orders/1.10"},
                "phase":"prepared","idempotency_key":"same-original-key",
                "preparation_digest":"b".repeat(64),"dispatch_attempted":false,
            }],
            "next_ready":null,
        },
        "labor_control_enabled":labor,
        "production_control_enabled":!labor,
        "review":null,
    });
    let result = json!({
        "ok":true,"mode":"control","stage":"production",
        "project_custody_currently_verified":true,
        "project_cancelled":false,"original_goal_proven":false,
        "original":{"goal":{"deadline_tick":300}},
        "child":child,
    });
    let reference = json!({
        "controller_fenced":false,
        "parent_acknowledged":{"head":"d".repeat(64)},
        "retained_stage":{"stage":"production","checkpoint":{
            "production":{"recovery":{"plan_digest":digest}},
        }},
    });
    (result, reference)
}

fn output(result: Value, reference: Value) -> Value {
    packet(
        "fortress.query",
        Some(SessionId::new(1)),
        Some(RequestId::new(2)),
        result,
        reference,
    )
}

fn recommended_kind(value: &Value) -> Option<String> {
    value["agent_turn"]["recommendations"][0]["arguments"]["request"]
        .as_str()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .and_then(|request| request["kind"].as_str().map(str::to_owned))
}

#[test]
fn parent_resumes_each_original_prepared_family_before_preparing_another_step() {
    for labor in [false, true] {
        for next_ready in [Value::Null, json!(0)] {
            let (mut result, reference) = prepared_fixture(labor);
            result["child"]["frontier"]["next_ready"] = next_ready;
            let view = output(result, reference);
            assert_eq!(recommended_kind(&view).as_deref(), Some("review_prepared"));
            let next = &view["agent_turn"]["recommendations"][0];
            assert_eq!(next["tool"], "fortress_plan");
            assert_eq!(next["original_step_id"], 0);
            assert_eq!(next["requires_confirmation"], false);
            assert_eq!(next["separate_commit_confirmation_required"], true);
            assert_eq!(next["prepares_review_only"], true);
            assert_eq!(next["grants_authority"], false);
            assert_eq!(view["agent_turn"]["active_work"]["confirmations"], json!([]));
            let offer = &view["agent_turn"]["affordances"][0];
            assert_eq!(offer["intent"], "review_prepared_original_step");
            assert_eq!(offer["request"]["original_plan_digest"], "a".repeat(64));
            assert_eq!(offer["native_preparation_repeated"], false);
            assert_eq!(
                offer["complete_evidence_pointer"],
                "/result/child/frontier"
            );
            assert_eq!(
                next["confidence"]["evidence"][0]["complete_evidence_pointer"],
                "/result/child/frontier"
            );
        }
    }
}

#[test]
fn a_current_original_review_still_requires_its_separate_exact_confirmation() {
    let (mut result, reference) = prepared_fixture(false);
    result["child"]["review"] = json!({"family":"work-orders/1.10"});
    result["child"]["frontier"]["review"] = json!({"paired_seal":"e".repeat(64)});
    let view = output(result, reference);
    let next = &view["agent_turn"]["recommendations"][0];
    assert_eq!(next["tool"], "fortress_commit");
    assert_eq!(next["arguments"]["review_seal"], "e".repeat(64));
    assert_eq!(next["requires_confirmation"], true);
    assert!(recommended_kind(&view).is_none());
    assert_eq!(view["agent_turn"]["affordances"], json!([]));
}

#[test]
fn retained_or_foreign_parent_context_never_advertises_a_fresh_original_review() {
    for cause in 0..14 {
        let (mut result, mut reference) = prepared_fixture(false);
        match cause {
            0 => result["ok"] = json!(false),
            1 => result["mode"] = json!("recover"),
            2 => result["mode"] = json!("offline"),
            3 => result["project_cancelled"] = json!(true),
            4 => result["session_closed"] = json!(true),
            5 => result["project_custody_currently_verified"] = json!(false),
            6 => result["original_goal_proven"] = json!(true),
            7 => reference["controller_fenced"] = json!(true),
            8 => result["stage"] = json!("workshop"),
            9 => reference["retained_stage"]["stage"] = json!("production_opening"),
            10 => {
                reference["retained_stage"]["checkpoint"]["production"]["recovery"]["plan_digest"] =
                    json!("f".repeat(64));
            }
            11 => {
                reference["retained_stage"]["checkpoint"]["production"]["recovery"] = Value::Null;
                reference["retained_stage"]["checkpoint"]["production_intent"] =
                    json!({"recovery_reference":{"recovery":{"plan_digest":"a".repeat(64)}}});
            }
            12 => result["current_stock_goal"] = json!({"already_satisfied":{}}),
            _ => {
                reference["child_reference"] = result["child"].clone();
                result["child"] = json!({"local_parent_only":true});
            }
        }
        let view = output(result, reference);
        assert_ne!(recommended_kind(&view).as_deref(), Some("review_prepared"), "{cause}");
        assert_eq!(view["agent_turn"]["affordances"], json!([]), "{cause}");
    }
}

#[test]
fn parent_review_advice_rejects_changed_source_lifetime_authority_and_original_key() {
    for cause in 0..16 {
        let (mut result, reference) = prepared_fixture(false);
        let child = &mut result["child"];
        match cause {
            0 => child["frontier"]["entries"][0]["dispatch_attempted"] = json!(true),
            1 => child["frontier"]["entries"][0]["phase"] = json!("dispatch_started"),
            2 => child["frontier"]["needs_reconciliation"] = json!(true),
            3 => child["original_plan"]["expires_at_tick"] = json!(130),
            4 => child["original_plan"]["steps"][0]["obligation"]["deadline_tick"] = json!(130),
            5 => child["observation_tick_floor"] = json!(131),
            6 => child["production_control_enabled"] = json!(false),
            7 => child["workshop_prerequisite"] = json!({"source_fenced":true}),
            8 => child["manager_prerequisite"] = json!({"source_fenced":true}),
            9 => child["input_supply"] = json!({"revalidation_failed":true}),
            10 => child["stock_target"] = json!({"revalidation_failed":true}),
            11 => child["frontier"]["entries"][0]["idempotency_key"] = json!("another-key"),
            12 => child["frontier"]["entries"][0]["preparation_digest"] = json!("0".repeat(64)),
            13 => child["original_plan"]["anchor"]["sequence"] = json!(6),
            14 => child["custody_initialization_may_be_partial"] = json!(true),
            _ => {
                child["portfolio"] = json!({"orders":[
                    {"step_id":0,"clock":null,"inventory_clock":null},
                    {"step_id":1,"clock":{"pending":true,"high_tick":130}},
                ]});
            }
        }
        let view = output(result, reference);
        assert_ne!(recommended_kind(&view).as_deref(), Some("review_prepared"), "{cause}");
        assert_eq!(view["agent_turn"]["affordances"], json!([]), "{cause}");
    }
}


fn clock_fixture(portfolio: bool, inventory: bool, prepared: bool) -> (Value, Value) {
    let child = crate::live_production_runtime::clock_guidance::tests::fixture(
        portfolio, inventory, prepared,
    );
    let digest = child["original_plan"]["plan_digest"].clone();
    (
        json!({
            "ok":true,"mode":"control","stage":"production",
            "project_custody_currently_verified":true,
            "project_cancelled":false,"original_goal_proven":false,
            "original":{"goal":{"deadline_tick":300}},
            "child":child,
        }),
        json!({
            "controller_fenced":false,
            "parent_acknowledged":{"head":"d".repeat(64)},
            "retained_stage":{"stage":"production","checkpoint":{
                "production":{"recovery":{"plan_digest":digest}},
            }},
        }),
    )
}

fn clock_blocker(result: &mut Value, code: &str, action: &str, step: u32) {
    result["child"]["clock_guidance"]["offers"] = json!([]);
    result["child"]["clock_guidance"]["blockers"] = json!([{
        "scope":"all","code":code,"message":"Resolve the exact original clock boundary.",
        "next_action":action,"order_step_id":step,
        "evidence":["/original_plan","/observation_tick_floor"],
    }]);
}

#[test]
fn parent_proposes_actual_scalar_and_portfolio_clock_requests_without_confirmation() {
    for portfolio in [false, true] {
        for inventory in [false, true] {
            for prepared in [false, true] {
                let (result, reference) = clock_fixture(portfolio, inventory, prepared);
                let expected = result["child"]["clock_guidance"]["offers"][0]["plan_request"].clone();
                let view = output(result.clone(), reference);
                let next = &view["agent_turn"]["recommendations"][0];
                assert_eq!(next["tool"], "fortress_plan");
                let request: Value = serde_json::from_str(
                    next["arguments"]["request"].as_str().expect("exact Plan request"),
                ).expect("closed request JSON");
                assert_eq!(request, expected);
                assert_eq!(request["order_step_id"],
                    if portfolio { json!(3) } else { Value::Null });
                assert_eq!(next["original_step_id"], if portfolio { 3 } else { 1 });
                assert_eq!(next["clock_scope"], if inventory { "inventory_run" } else { "run" });
                assert_eq!(next["clock_proposal_state"],
                    if prepared { "review_prepared" } else { "new_interval" });
                assert_eq!(next["requires_confirmation"], false);
                assert_eq!(next["separate_commit_confirmation_required"], true);
                assert_eq!(next["grants_authority"], false);
                assert_eq!(next["estimated_cost"]["game_ticks_after_separate_commit"], 5);
                assert_eq!(next["estimated_cost"]["native_time_advanced_by_planning"], 0);
                assert_eq!(view["agent_turn"]["active_work"]["confirmations"], json!([]));
                let briefing = &view["agent_turn"]["briefing"]["production_clock_guidance"];
                assert_eq!(briefing["source_anchor"], result["child"]["anchor"]);
                assert_eq!(briefing["plan_offers"], 1);
                assert_eq!(briefing["restored_from_recovery"], false);
                let affordance = &view["agent_turn"]["affordances"][0];
                assert_eq!(affordance["request"], expected);
                assert_eq!(affordance["native_time_advanced_by_planning"], 0);
                assert_eq!(affordance["complete_evidence_pointer"], "/result/child/clock_guidance");
                let field = if inventory { "inventory_clock" } else { "clock" };
                assert_eq!(
                    next["confidence"]["evidence"][0]["complete_evidence_pointer"],
                    format!("/result/child/{field}")
                );
                assert_eq!(view["result"]["child"], result["child"]);
            }
        }
    }
}

#[test]
fn a_current_clock_review_keeps_exact_confirmation_ahead_of_re_review_advice() {
    for portfolio in [false, true] {
        for inventory in [false, true] {
            let (mut result, reference) = clock_fixture(portfolio, inventory, true);
            let field = if inventory { "inventory_clock" } else { "clock" };
            result["child"][field]["review_available"] = json!(true);
            let active = result["child"][field].clone();
            if portfolio {
                result["child"]["portfolio"]["orders"][2][field] = active.clone();
            }
            let view = output(result, reference);
            let next = &view["agent_turn"]["recommendations"][0];
            assert_eq!(next["tool"], "fortress_commit");
            assert_eq!(next["arguments"]["scope"], field);
            assert_eq!(next["arguments"]["review_seal"], active["active_run"]["review_seal"]);
            assert_eq!(next["arguments"]["order_step_id"],
                if portfolio { json!(3) } else { Value::Null });
            assert_eq!(next["requires_confirmation"], true);
            assert_eq!(view["agent_turn"]["affordances"], json!([]));
        }
    }
}

#[test]
fn parent_routes_clock_blockers_to_reconciliation_exact_inspection_or_operator() {
    for portfolio in [false, true] {
        for action in ["wait", "inspect", "operator", "none"] {
            let (mut result, reference) = clock_fixture(portfolio, false, false);
            let step = if portfolio { 2 } else { 1 };
            clock_blocker(&mut result, "retained_original_boundary", action, step);
            let view = output(result, reference);
            let next = &view["agent_turn"]["recommendations"];
            if action == "none" {
                assert_eq!(next, &json!([]));
                continue;
            }
            let next = &next[0];
            let tool = match action {
                "wait" => "fortress_wait",
                "inspect" if portfolio => "fortress_query",
                _ => "fortress_explain",
            };
            assert_eq!(next["tool"], tool, "{portfolio}/{action}");
            assert_eq!(next["arguments"]["order_step_id"],
                if action == "inspect" && portfolio { json!(step) } else { Value::Null });
            assert_eq!(next["arguments"]["scope"],
                if action == "wait" { json!("goal") } else { Value::Null });
            assert_eq!(next["operator_action_required"], action == "operator");
            assert_eq!(next["blocker_code"], "retained_original_boundary");
            assert_eq!(next["grants_authority"], false);
            assert_eq!(view["agent_turn"]["affordances"], json!([]));
            assert_eq!(
                view["agent_turn"]["briefing"]["production_clock_guidance"]["blockers"][0]["code"],
                "retained_original_boundary"
            );
        }
    }
}

#[test]
fn parent_preserves_fresh_sample_guidance_when_a_consumed_floor_is_newer() {
    for portfolio in [false, true] {
        let (mut result, reference) = clock_fixture(portfolio, false, false);
        let step = if portfolio { 3 } else { 1 };
        result["child"]["observation_tick_floor"] = json!(111);
        result["child"]["selected_order_goal"] = Value::Null;
        clock_blocker(&mut result, "fresh_goal_sample_required", "wait", step);
        let view = output(result, reference);
        assert_eq!(view["agent_turn"]["recommendations"][0]["tool"], "fortress_wait");
        assert_eq!(view["agent_turn"]["recommendations"][0]["blocker_code"],
            "fresh_goal_sample_required");
        assert_eq!(view["agent_turn"]["affordances"], json!([]));
        assert_eq!(view["agent_turn"]["active_work"]["confirmations"], json!([]));
    }
}

#[test]
fn parent_clock_offers_require_current_verified_original_and_exact_child_guidance() {
    for cause in 0..13 {
        let (mut result, mut reference) = clock_fixture(true, true, false);
        match cause {
            0 => result["ok"] = json!(false),
            1 => result["mode"] = json!("recover"),
            2 => result["mode"] = json!("offline"),
            3 => result["project_cancelled"] = json!(true),
            4 => result["session_closed"] = json!(true),
            5 => result["project_custody_currently_verified"] = json!(false),
            6 => reference["controller_fenced"] = json!(true),
            7 => {
                reference["retained_stage"]["checkpoint"]["production"]["recovery"]["plan_digest"] =
                    json!("f".repeat(64));
            }
            8 => result["child"]["clock_guidance"]["anchor"]["sequence"] = json!(10),
            9 => result["child"]["clock_guidance"]["offers"][0]["plan_request"]["order_step_id"] = json!(2),
            10 => result["child"]["clock_guidance"] = Value::Null,
            11 => result["child"]["observation_tick_floor"] = json!(111),
            _ => {
                reference["child_reference"] = result["child"].clone();
                result["child"] = json!({"local_parent_only":true});
            }
        }
        let view = output(result, reference);
        assert!(!matches!(recommended_kind(&view).as_deref(), Some("run" | "inventory_run")), "{cause}");
        assert_eq!(view["agent_turn"]["affordances"], json!([]), "{cause}");
        assert!(view["agent_turn"]["briefing"]["production_clock_guidance"].is_null(), "{cause}");
        assert_eq!(view["agent_turn"]["active_work"]["confirmations"], json!([]), "{cause}");
    }
}
