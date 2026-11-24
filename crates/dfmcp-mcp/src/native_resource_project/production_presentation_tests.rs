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
