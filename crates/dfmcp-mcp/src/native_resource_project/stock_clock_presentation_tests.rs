//! Parent-owned no-order time control preserves complete root and clock evidence.
use super::super::runtime::stock_clock::{
    ClockCheckpoint, WatchClockTarget, original_digest, production_source_digest,
};
use super::*;
use crate::live_production_runtime::ProjectStockGoal;
use dfmcp_core::{FortressId, GameTick, ObservationCursor, StateAnchor};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn refresh(result: &mut Value) {
    let child = &result["child"];
    let proven = child["original_goal_proven"] == true;
    result["current_stock_goal"] = json!({
        "no_work_stock_goal":child["no_work_stock_goal"],
        "original_goal_proven":proven,
        "current_goal_anchor":if proven { child["anchor"].clone() } else { Value::Null },
    });
    result["original_goal_proven"] = json!(proven);
}

fn fixture(portfolio: bool) -> TestResult<(Value, Value)> {
    fixture_with_policy(portfolio, Some(true))
}

fn fixture_with_policy(portfolio: bool, opted_in: Option<bool>) -> TestResult<(Value, Value)> {
    let mut result = if portfolio {
        super::portfolio_tests::fixture(4, true)
    } else {
        super::tests::no_work_project_fixture()
    };
    result["project_custody_currently_verified"] = json!(true);
    result["original"] = json!({
        "schema":"dfmcp.native-resource-project-original/1",
        "goal":result["original"]["goal"],
        "directory":"/private/stock-clock-project-test",
        "workshop_policy":Digest32::of_bytes(b"fixed workshop policy").to_hex(),
        "production_policy":Digest32::of_bytes(b"fixed production policy").to_hex(),
    });
    if let Some(opted_in) = opted_in {
        result["original"]["stock_watch_clock"] = json!(opted_in);
    }
    let watch = &mut result["child"]["no_work_stock_goal"];
    watch["original_goal_proven"] = json!(false);
    watch["current_inventory_proven"] = json!(false);
    watch["sample"]["stable_samples_observed"] = json!(1);
    watch["sample"]["achieved_at_tick"] = Value::Null;
    watch["sample"]["last_counted_tick"] = json!(140);
    result["child"]["original_goal_proven"] = json!(false);
    refresh(&mut result);

    let production = json!({
        "schema":"dfmcp.project-production-recovery/1",
        "native_custody":{
            "workforce_journal_id":Digest32::of_bytes(b"original workforce journal").to_hex(),
            "order_journal_id":Digest32::of_bytes(b"original order journal").to_hex(),
        },
    });
    let parent = Digest32::of_bytes(b"original parent journal");
    let goal: ProjectStockGoal = serde_json::from_value(result["original"]["goal"].clone())?;
    let target = WatchClockTarget::new(
        parent,
        original_digest(&result["original"])?,
        &goal,
        production_source_digest(&production)?,
        StateAnchor {
            fortress_id: FortressId::new(7),
            cursor: ObservationCursor {
                epoch: 2,
                sequence: 7,
            },
            tick: GameTick(120),
            state_hash: Digest32::of_bytes(b"original no-work citizens"),
        },
    )?;
    let journal = Digest32::of_bytes(b"separate original stock clock journal");
    let mut checkpoint = ClockCheckpoint::prospective(target.clone(), journal)?.to_json();
    checkpoint["authority_tick_floor"] = json!(140);
    checkpoint["high_source_sequence"] = json!(9);
    let clock = json!({
        "schema":"dfmcp.native-resource-stock-clock-view/1",
        "mode":"control",
        "source":"run/1.13","target_scope":"original_resource_project_stock_goal",
        "target":target.to_json(),"target_digest":target.digest().to_hex(),
        "original_paired_seal":target.original_paired_seal().to_hex(),
        "journal_id":journal.to_hex(),"head":journal.to_hex(),
        "high_tick":140,"high_source_sequence":9,
        "inspection_required":false,"pending":false,"unresolved":false,
        "may_be_running":false,"advancement_fenced":false,"review_available":false,
        "active_run":null,"runs":[],"run_count":0,
        "current_pause_proven":false,"original_goal_proven":false,
        "sample_history_durable":false,"recovery_restores_current_proof":false,
    });
    result["stock_clock"] = clock.clone();
    result["stock_clock_guidance"] = json!({
        "schema":"dfmcp.resource-stock-clock-guidance/1",
        "target_digest":target.digest().to_hex(),
        "original_goal_digest":clock["target"]["original_goal_digest"],
        "goal_witness":result["child"]["no_work_stock_goal"]["goal_witness"],
        "plan_request":{
            "kind":"stock_watch_run",
            "original_paired_seal":target.original_paired_seal().to_hex(),
            "game_ticks":10,"wall_millis":1000,
        },
        "blocker":null,
    });
    let reference = json!({
        "original":result["original"],
        "controller_fenced":false,
        "parent_acknowledged":{
            "journal_id":parent.to_hex(),
            "head":Digest32::of_bytes(b"acknowledged parent head").to_hex(),
            "frames":3,"authority_tick_floor":140,
        },
        "retained_stage":{"stage":"production","checkpoint":{
            "production":production,"stock_clock":checkpoint,
        }},
        "child_reference":result["child"],
        "stock_clock":clock,
    });
    Ok((result, reference))
}

fn prepared(result: &mut Value, reference: &mut Value) {
    let head = Digest32::of_bytes(b"acknowledged prepared clock head").to_hex();
    let run = json!({
        "key":format!("stock-watch-clock-{}", Digest32::of_bytes(b"same original run").to_hex()),
        "plan_digest":Digest32::of_bytes(b"original bounded run").to_hex(),
        "review_seal":Digest32::of_bytes(b"current separate review").to_hex(),
        "state":"prepared","pending":true,"unresolved":false,
        "before_tick":140,"source_generation":71,"source_sequence":9,
        "game_ticks":10,"wall_millis":1000,
        "purpose":"advance_original_resource_stock_stability",
        "native":{"unpause_attempted":false},
    });
    result["stock_clock"]["head"] = json!(head);
    result["stock_clock"]["pending"] = json!(true);
    result["stock_clock"]["review_available"] = json!(true);
    result["stock_clock"]["run_count"] = json!(1);
    result["stock_clock"]["active_run"] = run.clone();
    result["stock_clock"]["runs"] = json!([run]);
    result["stock_clock_guidance"]["plan_request"] = Value::Null;
    result["stock_clock_guidance"]["blocker"] = Value::Null;
    reference["retained_stage"]["checkpoint"]["stock_clock"]["head"] = json!(head);
    reference["stock_clock"] = result["stock_clock"].clone();
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

fn request(output: &Value) -> TestResult<Value> {
    let raw = output["agent_turn"]["recommendations"][0]["arguments"]["request"]
        .as_str()
        .ok_or("next request is missing")?;
    Ok(serde_json::from_str(raw)?)
}

#[test]
fn scalar_and_complete_portfolio_offer_only_a_separately_confirmed_stock_interval() -> TestResult {
    for portfolio in [false, true] {
        let (result, reference) = fixture(portfolio)?;
        let expected = result["stock_clock_guidance"]["plan_request"].clone();
        let original = result["original"].clone();
        let rendered = output(result, reference);
        let next = &rendered["agent_turn"]["recommendations"][0];
        assert_eq!(next["tool"], "fortress_plan");
        assert_eq!(request(&rendered)?, expected);
        assert_eq!(expected.as_object().map(|fields| fields.len()), Some(4));
        assert!(expected.get("order_step_id").is_none());
        assert_eq!(next["clock_scope"], "stock_clock");
        assert_eq!(next["requires_confirmation"], false);
        assert_eq!(next["separate_commit_confirmation_required"], true);
        assert_eq!(
            next["estimated_cost"]["native_time_advanced_by_planning"],
            0
        );
        assert_eq!(
            next["estimated_cost"]["game_ticks_after_separate_commit"],
            10
        );
        assert_eq!(next["grants_authority"], false);
        assert_eq!(
            rendered["agent_turn"]["active_work"]["confirmations"],
            json!([])
        );
        assert_eq!(
            rendered["agent_turn"]["affordances"][0]["request"],
            expected
        );
        assert_eq!(
            rendered["agent_turn"]["briefing"]["stock_clock_guidance"]["plan_available"],
            true
        );
        assert_eq!(rendered["result"]["original"], original);
        assert_eq!(rendered["result"]["original_goal_proven"], false);
        assert!(render(rendered).is_some());
    }
    Ok(())
}

#[test]
fn read_only_stock_clock_blockers_need_no_fabricated_current_sample_witness() -> TestResult {
    for action in ["wait", "inspect", "operator"] {
        let (mut result, reference) = fixture(false)?;
        stock_watch::tests::pending(&mut result["child"]);
        refresh(&mut result);
        result["stock_clock_guidance"]["goal_witness"] = Value::Null;
        result["stock_clock_guidance"]["plan_request"] = Value::Null;
        result["stock_clock_guidance"]["blocker"] = json!({
            "code":"current_clock_boundary","message":"Resolve the current stock-clock boundary.",
            "next_action":action,
        });
        let rendered = output(result, reference);
        let next = &rendered["agent_turn"]["recommendations"][0];
        assert_eq!(
            next["tool"],
            if action == "wait" {
                "fortress_wait"
            } else {
                "fortress_explain"
            }
        );
        assert_eq!(next["blocker_code"], "current_clock_boundary");
        assert_eq!(next["operator_action_required"], action == "operator");
        assert_eq!(next["grants_authority"], false);
        assert_eq!(
            rendered["agent_turn"]["briefing"]["stock_clock_guidance"]["blocker"]["code"],
            "current_clock_boundary"
        );
        assert!(
            rendered["agent_turn"]["briefing"]["stock_clock_guidance"]["goal_witness"].is_null()
        );
        assert_eq!(rendered["agent_turn"]["affordances"], json!([]));
        assert_eq!(
            rendered["agent_turn"]["active_work"]["confirmations"],
            json!([])
        );
        assert_eq!(rendered["result"]["original_goal_proven"], false);
    }
    Ok(())
}

#[test]
fn prepared_stock_clock_owns_its_exact_confirmation_without_an_order_or_current_sample()
-> TestResult {
    for portfolio in [false, true] {
        let (mut result, mut reference) = fixture(portfolio)?;
        prepared(&mut result, &mut reference);
        let seal = result["stock_clock"]["active_run"]["review_seal"].clone();
        let rendered = output(result, reference);
        let next = &rendered["agent_turn"]["recommendations"][0];
        assert_eq!(next["tool"], "fortress_commit");
        assert_eq!(next["arguments"]["scope"], "stock_clock");
        assert_eq!(next["arguments"]["review_seal"], seal);
        assert_eq!(next["arguments"]["confirmed"], true);
        assert!(next["arguments"].get("order_step_id").is_none());
        assert_eq!(rendered["agent_turn"]["affordances"], json!([]));
        let confirmations = &rendered["agent_turn"]["active_work"]["confirmations"];
        assert_eq!(confirmations.as_array().map(Vec::len), Some(1));
        assert_eq!(confirmations[0]["scope"], "stock_clock");
        assert_eq!(
            confirmations[0]["complete_evidence_pointer"],
            "/result/stock_clock"
        );
        assert_eq!(confirmations[0]["restored_from_recovery"], false);
        assert!(confirmations[0].get("order_step_id").is_none());
        assert!(rendered["result"]["child"]["no_work_stock_goal"]["sample"].is_null());
        assert_eq!(rendered["result"]["original_goal_proven"], false);
        assert_eq!(
            rendered["retained_reference"]["stock_clock"]["review_available"],
            false
        );
        assert_eq!(
            rendered["retained_reference"]["stock_clock"]["active_run"]["review_seal"],
            seal
        );
    }
    Ok(())
}

#[test]
fn stock_clock_advice_rejects_changed_parent_source_target_and_closed_request() -> TestResult {
    for case in 0..20 {
        let (mut result, mut reference) = fixture(true)?;
        match case {
            0 => result["stock_clock_guidance"]["plan_request"]["order_step_id"] = json!(0),
            1 => {
                result["stock_clock_guidance"]["plan_request"]["goal_witness"] =
                    json!("f".repeat(64))
            }
            2 => {
                result["stock_clock_guidance"]["plan_request"]["original_paired_seal"] =
                    json!("f".repeat(64))
            }
            3 => result["stock_clock_guidance"]["plan_request"]["game_ticks"] = json!(61),
            4 => result["stock_clock_guidance"]["plan_request"]["wall_millis"] = json!(60_001),
            5 => result["stock_clock_guidance"]["goal_witness"] = json!("f".repeat(64)),
            6 => result["stock_clock_guidance"]["original_goal_digest"] = json!("f".repeat(64)),
            7 => result["stock_clock_guidance"]["target_digest"] = json!("f".repeat(64)),
            8 => result["stock_clock"]["target"]["deadline_tick"] = json!(210),
            9 => reference["parent_acknowledged"]["journal_id"] = json!("f".repeat(64)),
            10 => {
                reference["retained_stage"]["checkpoint"]["production"]["native_custody"]["order_journal_id"] =
                    json!("f".repeat(64))
            }
            11 => result["original"]["goal"]["targets"][3]["minimum_stack_units"] = json!(99),
            12 => result["stock_clock"]["head"] = json!("f".repeat(64)),
            13 => result["stock_clock"]["high_source_sequence"] = json!(8),
            14 => result["project_custody_currently_verified"] = json!(false),
            15 => result["mode"] = json!("offline"),
            16 => result["mode"] = json!("recover"),
            17 => reference["controller_fenced"] = json!(true),
            18 => result["stock_clock"] = Value::Null,
            _ => reference["retained_stage"]["stage"] = json!("workshop"),
        }
        let rendered = output(result, reference);
        assert_ne!(
            rendered["agent_turn"]["recommendations"][0]["tool"], "fortress_plan",
            "{case}"
        );
        assert_ne!(
            rendered["agent_turn"]["recommendations"][0]["tool"], "fortress_commit",
            "{case}"
        );
        assert_eq!(
            rendered["agent_turn"]["active_work"]["confirmations"],
            json!([]),
            "{case}"
        );
        assert_eq!(rendered["agent_turn"]["affordances"], json!([]), "{case}");
    }
    Ok(())
}

#[test]
fn a_self_consistent_clock_target_cannot_enable_a_legacy_parent_policy() -> TestResult {
    for opted_in in [None, Some(false)] {
        let (mut result, mut reference) = fixture_with_policy(true, opted_in)?;
        let output = output(result.clone(), reference.clone());
        assert_ne!(
            output["agent_turn"]["recommendations"][0]["tool"],
            "fortress_plan"
        );
        assert_eq!(output["agent_turn"]["affordances"], json!([]));
        prepared(&mut result, &mut reference);
        let output = packet(
            "fortress.plan",
            Some(SessionId::new(1)),
            None,
            result,
            reference,
        );
        assert_eq!(
            output["agent_turn"]["active_work"]["confirmations"],
            json!([])
        );
        assert_eq!(output["result"]["original_goal_proven"], false);
    }
    Ok(())
}

#[test]
fn stock_clock_confirmation_requires_current_review_and_the_original_interval() -> TestResult {
    for case in 0..12 {
        let (mut result, mut reference) = fixture(false)?;
        prepared(&mut result, &mut reference);
        match case {
            0 => result["stock_clock"]["review_available"] = json!(false),
            1 => result["stock_clock"]["inspection_required"] = json!(true),
            2 => result["stock_clock"]["unresolved"] = json!(true),
            3 => result["stock_clock"]["may_be_running"] = json!(true),
            4 => result["stock_clock"]["active_run"]["state"] = json!("dispatch_started"),
            5 => result["stock_clock"]["active_run"]["before_tick"] = json!(139),
            6 => result["stock_clock"]["active_run"]["game_ticks"] = json!(61),
            7 => result["stock_clock"]["active_run"]["review_seal"] = json!("invalid"),
            8 => result["project_cancelled"] = json!(true),
            9 => result["session_closed"] = json!(true),
            10 => result["child"]["no_work_stock_goal"]["goal_witness"] = json!("invalid"),
            _ => result["ok"] = json!(false),
        }
        let rendered = output(result, reference);
        assert_eq!(
            rendered["agent_turn"]["active_work"]["confirmations"],
            json!([]),
            "{case}"
        );
        assert!(
            rendered["agent_turn"]["recommendations"]
                .as_array()
                .is_some_and(|rows| { rows.iter().all(|row| row["tool"] != "fortress_commit") }),
            "{case}"
        );
    }
    Ok(())
}

#[test]
fn retained_preparation_without_a_review_does_not_loop_in_wait_or_restore_permission() -> TestResult
{
    let (mut result, mut reference) = fixture(false)?;
    prepared(&mut result, &mut reference);
    result["stock_clock"]["review_available"] = json!(false);
    let retained = output(result.clone(), reference.clone());
    assert_eq!(
        retained["agent_turn"]["recommendations"][0]["tool"],
        "fortress_explain"
    );
    assert_eq!(
        retained["agent_turn"]["recommendations"][0]["blocker_code"],
        "stock_clock_prepared_requires_review"
    );
    assert_eq!(
        retained["agent_turn"]["active_work"]["confirmations"],
        json!([])
    );

    result["child"]["no_work_stock_goal"] = Value::Null;
    result["current_stock_goal"] = Value::Null;
    for (mode, clock_mode, can_retire) in [
        ("control", "control", true),
        ("recover", "reconcile", false),
        ("recover", "control", false),
        ("control", "reconcile", false),
    ] {
        result["mode"] = json!(mode);
        result["stock_clock"]["mode"] = json!(clock_mode);
        let recovered = output(result.clone(), reference.clone());
        let next = &recovered["agent_turn"]["recommendations"][0];
        assert_eq!(
            next["tool"],
            if can_retire {
                "fortress_cancel"
            } else {
                "fortress_explain"
            }
        );
        assert_eq!(
            next["arguments"]["scope"],
            if can_retire {
                json!("stock_clock")
            } else {
                Value::Null
            }
        );
        assert_eq!(next["grants_authority"], false);
        assert!(next["arguments"].get("order_step_id").is_none());
        assert_eq!(
            recovered["agent_turn"]["active_work"]["confirmations"],
            json!([])
        );
    }
    for state in ["dispatch_started", "tracking", "cancel_requested"] {
        result["stock_clock"]["active_run"]["state"] = json!(state);
        result["stock_clock"]["unresolved"] = json!(true);
        result["stock_clock"]["may_be_running"] = json!(true);
        let running = output(result.clone(), reference.clone());
        assert_eq!(
            running["agent_turn"]["recommendations"][0]["tool"],
            "fortress_wait"
        );
        assert_eq!(
            running["agent_turn"]["recommendations"][0]["arguments"]["scope"],
            "stock_clock"
        );
    }
    Ok(())
}

#[test]
fn unsettled_stock_time_withdraws_proof_and_continuation_and_reconciles_after_project_stop()
-> TestResult {
    for deficit in [false, true] {
        for uncertainty in ["pending", "unresolved", "inspection_required"] {
            let (mut result, mut reference) = fixture(false)?;
            if deficit {
                stock_watch::tests::unfinished_deficit(&mut result["child"]);
            } else {
                result["child"]["no_work_stock_goal"]["sample"]["stable_samples_observed"] =
                    json!(2);
                result["child"]["no_work_stock_goal"]["sample"]["achieved_at_tick"] = json!(140);
                result["child"]["no_work_stock_goal"]["original_goal_proven"] = json!(true);
                result["child"]["no_work_stock_goal"]["current_inventory_proven"] = json!(true);
                result["child"]["original_goal_proven"] = json!(true);
            }
            refresh(&mut result);
            result["stock_clock"][uncertainty] = json!(true);
            let rendered = output(result.clone(), reference.clone());
            assert_eq!(rendered["result"]["original_goal_proven"], false);
            assert!(rendered["result"]["child"]["no_work_stock_goal"]["sample"].is_null());
            assert!(
                rendered["result"]["child"]["no_work_stock_goal"]["continuation_request"].is_null()
            );
            assert_eq!(
                rendered["agent_turn"]["recommendations"][0]["tool"],
                "fortress_wait"
            );
            assert_eq!(
                rendered["agent_turn"]["recommendations"][0]["arguments"]["scope"],
                "stock_clock"
            );
            assert_eq!(
                rendered["agent_turn"]["active_work"]["confirmations"],
                json!([])
            );

            result["stage"] = json!("cancelled");
            result["project_cancelled"] = json!(true);
            reference["retained_stage"]["stage"] = json!("cancelled");
            reference["retained_stage"]["cancelled_from"] = json!("production");
            let stopped = output(result, reference);
            assert_eq!(
                stopped["agent_turn"]["recommendations"][0]["arguments"]["scope"],
                "stock_clock"
            );
            assert_eq!(stopped["result"]["original_goal_proven"], false);
        }
    }
    let (mut result, reference) = fixture(false)?;
    result["child"]["no_work_stock_goal"]["sample"]["stable_samples_observed"] = json!(2);
    result["child"]["no_work_stock_goal"]["sample"]["achieved_at_tick"] = json!(140);
    result["child"]["no_work_stock_goal"]["original_goal_proven"] = json!(true);
    result["child"]["no_work_stock_goal"]["current_inventory_proven"] = json!(true);
    result["child"]["original_goal_proven"] = json!(true);
    refresh(&mut result);
    result["stock_clock"] = Value::Null;
    let missing = output(result, reference);
    assert_eq!(missing["result"]["original_goal_proven"], false);
    assert!(missing["result"]["child"]["no_work_stock_goal"]["sample"].is_null());
    assert_eq!(
        missing["agent_turn"]["recommendations"][0]["tool"],
        "fortress_explain"
    );
    assert_eq!(
        missing["agent_turn"]["active_work"]["confirmations"],
        json!([])
    );
    Ok(())
}

#[test]
fn settled_stock_clock_preserves_explicit_goal_continuation_and_fresh_joint_proof() -> TestResult {
    for portfolio in [false, true] {
        let (mut result, reference) = fixture(portfolio)?;
        if portfolio {
            let watch = &mut result["child"]["no_work_stock_goal"];
            watch["sample"]["targets"][2]["observed_stack_units"] = json!(0);
            watch["sample"]["targets"][2]["current_truth"] = json!("false");
            watch["sample"]["current_truth"] = json!("false");
            watch["sample"]["stable_samples_observed"] = json!(0);
            watch["sample"]["last_counted_tick"] = Value::Null;
            watch["continuation_request"] = json!({
                "kind":"continue_stock_goal","goal_witness":watch["goal_witness"],
            });
        } else {
            stock_watch::tests::unfinished_deficit(&mut result["child"]);
        }
        refresh(&mut result);
        result["stock_clock_guidance"]["plan_request"] = Value::Null;
        result["stock_clock_guidance"]["blocker"] = json!({
            "code":"stock_goal_false","message":"The original stock goal needs production.",
            "next_action":"operator",
        });
        let continuation = result["child"]["no_work_stock_goal"]["continuation_request"].clone();
        let rendered = output(result, reference);
        assert_eq!(request(&rendered)?, continuation);
        assert_eq!(
            rendered["agent_turn"]["active_work"]["confirmations"],
            json!([])
        );

        let (mut result, reference) = fixture(portfolio)?;
        result["child"]["no_work_stock_goal"]["sample"]["stable_samples_observed"] = json!(2);
        result["child"]["no_work_stock_goal"]["sample"]["achieved_at_tick"] = json!(140);
        result["child"]["no_work_stock_goal"]["original_goal_proven"] = json!(true);
        result["child"]["no_work_stock_goal"]["current_inventory_proven"] = json!(true);
        result["child"]["original_goal_proven"] = json!(true);
        refresh(&mut result);
        let rendered = output(result, reference);
        assert_eq!(rendered["result"]["original_goal_proven"], true);
        assert_eq!(
            rendered["agent_turn"]["briefing"]["current_goal_anchor"]["tick"],
            140
        );
        assert_eq!(
            rendered["agent_turn"]["recommendations"][0]["tool"],
            "fortress_explain"
        );
    }
    Ok(())
}

#[test]
fn failure_and_compact_recovery_keep_separate_stock_clock_custody_without_reviews() -> TestResult {
    let (mut result, mut reference) = fixture(true)?;
    prepared(&mut result, &mut reference);
    reference["stock_clock"]["unresolved"] = json!(true);
    reference["stock_clock"]["may_be_running"] = json!(true);
    reference["child_reference"]["portfolio"] = json!({"orders":[
        {"step_id":0,"clock":{"journal_id":"old order clock","pending":false}},
    ]});
    for compact in [false, true] {
        let mut retained = reference.clone();
        if compact {
            retained
                .as_object_mut()
                .ok_or("reference object missing")?
                .remove("stock_clock");
        }
        let rendered = failure(
            "fortress.wait",
            Some(SessionId::new(1)),
            None,
            super::super::error(dfmcp_core::ErrorCode::BudgetExceeded, "publication refused"),
            retained,
        );
        let actions = rendered["agent_turn"]["active_work"]["actions"]
            .as_array()
            .ok_or("clock custody missing")?;
        assert_eq!(actions.len(), 2);
        let clock = actions
            .iter()
            .find(|row| row["scope"] == "stock_clock")
            .ok_or("stock clock custody missing")?;
        assert_eq!(clock["journal_id"], result["stock_clock"]["journal_id"]);
        assert_eq!(clock["head"], result["stock_clock"]["head"]);
        assert_eq!(clock["authority_tick_floor"], 140);
        assert!(clock.get("order_step_id").is_none());
        assert!(clock.get("review_seal").is_none());
        assert_eq!(clock["current_pause_proven"], false);
        assert_eq!(clock["automatic_redispatch"], false);
        assert_eq!(
            rendered["agent_turn"]["active_work"]["confirmations"],
            json!([])
        );
        assert_eq!(
            rendered["agent_turn"]["recommendations"][0]["tool"],
            "fortress_query"
        );
        if !compact {
            assert_eq!(
                rendered["retained_reference"]["stock_clock"]["review_available"],
                false
            );
            assert!(
                rendered["agent_turn"]["active_work"]["indeterminate_effects"]
                    .as_array()
                    .ok_or("clock uncertainty missing")?
                    .iter()
                    .any(|row| row["scope"] == "stock_clock")
            );
        }
    }
    // A failed operation may advance actual custody after the last inspected
    // history view without advancing the independently acknowledged parent.
    let historical = reference["stock_clock"].clone();
    let acknowledged = reference["retained_stage"]["checkpoint"]["stock_clock"].clone();
    let mut actual = acknowledged.clone();
    actual["head"] = json!(Digest32::of_bytes(b"newer uninspected clock head").to_hex());
    actual["authority_tick_floor"] = json!(155);
    actual["high_source_sequence"] = json!(17);
    ClockCheckpoint::from_json(&acknowledged)?
        .validate_successor(&ClockCheckpoint::from_json(&actual)?)?;
    reference["stock_clock"]["recovery_checkpoint"] = actual.clone();
    reference["stock_clock"]["inspection_required"] = json!(true);
    let rendered = failure(
        "fortress.wait",
        Some(SessionId::new(1)),
        None,
        super::super::error(
            dfmcp_core::ErrorCode::BudgetExceeded,
            "newer clock custody survived a refused inspection",
        ),
        reference,
    );
    let clock = rendered["agent_turn"]["active_work"]["actions"]
        .as_array()
        .ok_or("clock custody missing")?
        .iter()
        .find(|row| row["scope"] == "stock_clock")
        .ok_or("actual stock clock custody missing")?;
    for field in [
        "journal_id",
        "head",
        "authority_tick_floor",
        "high_source_sequence",
    ] {
        assert_eq!(clock[field], actual[field], "{field}");
    }
    assert_ne!(clock["head"], historical["head"]);
    assert_eq!(clock["inspection_required"], true);
    assert_eq!(clock["current_pause_proven"], false);
    assert_eq!(clock["automatic_redispatch"], false);
    let pointer = clock["recovery_checkpoint_pointer"]
        .as_str()
        .ok_or("actual recovery checkpoint pointer missing")?;
    assert_eq!(
        pointer,
        "/retained_reference/stock_clock/recovery_checkpoint"
    );
    assert_eq!(rendered.pointer(pointer), Some(&actual));
    let retained = &rendered["retained_reference"]["stock_clock"];
    for field in [
        "head",
        "high_tick",
        "high_source_sequence",
        "active_run",
        "runs",
    ] {
        assert_eq!(retained[field], historical[field], "historical {field}");
    }
    assert_eq!(
        rendered["retained_reference"]["retained_stage"]["checkpoint"]["stock_clock"],
        acknowledged,
    );
    assert_eq!(retained["review_available"], false);
    assert_eq!(
        rendered["agent_turn"]["active_work"]["confirmations"],
        json!([])
    );
    assert_eq!(rendered["result"]["original_goal_proven"], false);
    Ok(())
}

#[test]
fn complete_stock_clock_history_chunks_round_trip_and_oversized_review_refuses() -> TestResult {
    let (mut result, mut reference) = fixture(true)?;
    prepared(&mut result, &mut reference);
    result["stock_clock"]["runs"] = json!((0..24).map(|index| json!({
        "ordinal":index,"retained_native_evidence":"本来の時計".repeat(250),
        "last_record_witness":Digest32::of_bytes(format!("run {index}").as_bytes()).to_hex(),
    })).collect::<Vec<_>>());
    let expected = result["stock_clock"].clone();
    let rendered = output(result.clone(), reference.clone());
    let parts = texts(rendered.clone()).ok_or("complete clock history did not fit")?;
    let first: Value = serde_json::from_str(&parts[0])?;
    let descriptor = &first["result"]["stock_clock"];
    assert_eq!(descriptor["field_path"], "/result/stock_clock");
    assert_eq!(descriptor["encoding"], "utf-8-json");
    let mut encoded = String::new();
    for block in descriptor["blocks"]
        .as_array()
        .ok_or("clock blocks missing")?
    {
        let index = usize::try_from(block["content_index"].as_u64().ok_or("index missing")?)?;
        encoded.push_str(parts.get(index).ok_or("clock content missing")?);
    }
    assert_eq!(serde_json::from_str::<Value>(&encoded)?, expected);
    assert_eq!(
        descriptor["sha256"],
        Digest32::of_bytes(encoded.as_bytes()).to_hex()
    );
    assert!(render(rendered).is_some());

    result["stock_clock"]["runs"][23]["retained_native_evidence"] = json!("x".repeat(260_000));
    assert!(render(output(result, reference)).is_none());
    Ok(())
}
