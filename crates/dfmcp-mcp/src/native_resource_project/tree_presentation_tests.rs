//! Presentation regressions over the actual Tree owner's published field shape.
//! These inputs exercise guidance and encoding, not native source qualification.
use super::super::{failure, packet, render, texts};
use super::*;
use dfmcp_core::{DfmcpError, ErrorCode, RequestId};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn root_goal() -> Value {
    json!({"recipe":"wooden_bed","minimum_stack_units":5,"max_order_amount":20,
        "deadline_tick":900,"poll_interval_ticks":10,"stable_samples":2,"assign_labor":false})
}

fn original() -> Value {
    json!({"key":"tree-original","plan_digest":"a".repeat(64),
        "review_seal":"b".repeat(64),"deadline_tick":900,"native_phase":2,
        "prepared_in_this_connection":false,"designation_acknowledged":true,
        "unresolved":false,"sample_publication_pending":false,"continuity_lost":false,
        "historical_supply_observed_tick":null,"fresh_supply_observed":false})
}

fn clock() -> Value {
    json!({"enabled":true,"verified":true,"target_id":"c".repeat(64),
        "recovery":{"journal_id":"d".repeat(64),"head":"e".repeat(64)},
        "deadline_tick":900,"pending":false,"unresolved":false,"active_run":null,
        "advancement_fenced":false,"commit_available":false,"cancel_available":true,
        "new_run_available":true,"clock_ready":true,"recommended_game_ticks":100,
        "remaining_horizon_ticks":800,"remaining_tree_samples":15})
}

fn ready() -> Value {
    json!({"ok":true,"mode":"control","stage":"tree","project_cancelled":false,
        "project_custody_currently_verified":true,"original":{"goal":root_goal()},
        "original_goal_proven":false,"budget":{"max_game_ticks":100},
        "child":{"ok":true,"source_profile":"tree-harvest/1.21","mode":"control",
            "retained_state_currently_verified":true,"max_game_ticks":100,
            "original":original(),"observation":null,"clock":clock()}})
}

fn reference(value: &Value) -> Value {
    let mut retained_clock = value["child"]["clock"].clone();
    if let Some(clock) = retained_clock.as_object_mut() {
        clock.insert("verified".into(), json!(false));
        clock.remove("commit_available");
        clock.remove("cancel_available");
        clock.remove("new_run_available");
        if let Some(active) = clock.get_mut("active_run").and_then(Value::as_object_mut) {
            active.insert("review_seal".into(), Value::Null);
        }
    }
    let child = json!({
        "acknowledged":{"journal_id":"1".repeat(64),"head":"2".repeat(64),"frames":5},
        "original_key":"tree-original","original":{"key":"tree-original",
            "plan_digest":"a".repeat(64),"deadline_tick":900,
            "dispatch_started":true,"unresolved":false},
        "consumed_tick_floor":100,"automatic_redispatch":false,
        "project_source_frontier":{"high_tick":100,"high_generation":1,
            "high_sequence":2,"high_next_item_id":50,"fenced":false},
        "clock":retained_clock});
    json!({"parent_acknowledged":{"journal_id":"3".repeat(64),"head":"4".repeat(64),
            "frames":4,"authority_tick_floor":100},
        "original":{"goal":root_goal()},"authority_tick_floor":100,"controller_fenced":false,
        "retained_stage":{"stage":"tree","cancelled_from":null,
            "checkpoint":{"tree":child,"workshop":null,"handoff":null,"production":null}},
        "child_reference":child})
}

fn shown(value: Value, retained: Value) -> Value {
    packet(
        "fortress.wait",
        Some(SessionId::new(1)),
        Some(RequestId::new(2)),
        value,
        retained,
    )
}

fn parsed_request(suggestion: &Value) -> std::result::Result<Value, Box<dyn std::error::Error>> {
    let raw = suggestion["arguments"]["request"]
        .as_str()
        .ok_or("missing request")?;
    Ok(serde_json::from_str(raw)?)
}

fn offered() -> Value {
    let mut value = ready();
    value["child"]["original"] = Value::Null;
    value["child"]["clock"] = Value::Null;
    value["child"]["recommended_deadline_tick"] = json!(12_100);
    value["child"]["observation"] = json!({"tick":100,"witness":"5".repeat(64),
        "root":[15,15,2],"species":7,"visible_domain_complete":true,"eligible_at_anchor":true,
        "observed_eligible_workers":[11],"tool_offers":[{"item_id":23,"offered_at_anchor":true}]});
    value
}

#[test]
fn tree_offer_uses_the_fixed_root_deadline_and_exact_published_witness() -> TestResult {
    let value = offered();
    let output = shown(value.clone(), reference(&value));
    let suggestion = &output["agent_turn"]["recommendations"][0];
    assert_eq!(suggestion["tool"], "fortress_plan");
    assert_eq!(
        parsed_request(suggestion)?,
        json!({"kind":"fell_one_visible_tree",
        "expected_observation":"5".repeat(64),"deadline_tick":900})
    );
    assert_eq!(
        output["agent_turn"]["briefing"]["original_goal"],
        root_goal()
    );
    assert_eq!(suggestion["grants_authority"], false);
    assert_eq!(suggestion["estimated_cost"]["native_writer_calls_max"], 0);
    let mut opening = value.clone();
    let mut retained = reference(&value);
    retained["child_reference"]["original"] = Value::Null;
    retained["retained_stage"]["checkpoint"]["tree"]["original"] = Value::Null;
    opening["child"] = json!({"opened":true,"child":retained["child_reference"]});
    let opened = shown(opening, retained);
    assert_eq!(
        opened["agent_turn"]["recommendations"][0]["tool"],
        "fortress_observe"
    );
    for case in 0..8 {
        let mut blocked = value.clone();
        match case {
            0 => blocked["original"]["goal"]["deadline_tick"] = json!(100),
            1 => blocked["original"]["goal"]["deadline_tick"] = json!(12_101),
            2 => blocked["child"]["observation"]["eligible_at_anchor"] = json!(false),
            3 => blocked["child"]["observation"]["visible_domain_complete"] = json!(false),
            4 => blocked["child"]["retained_state_currently_verified"] = json!(false),
            5 => blocked["child"]["observation"]["witness"] = json!("historical-label"),
            6 => blocked["child"]["project_source_frontier"] = json!({"fenced":true}),
            _ => {
                blocked["mode"] = json!("recover");
                blocked["child"]["mode"] = json!("recover");
            }
        }
        assert!(observed_plan(&blocked).is_none(), "case {case}");
        let output = shown(blocked.clone(), reference(&blocked));
        assert_ne!(
            output["agent_turn"]["recommendations"][0]["tool"], "fortress_plan",
            "case {case}"
        );
    }
    Ok(())
}

fn prepared_clock() -> Value {
    let mut value = ready();
    value["child"]["clock"]["new_run_available"] = json!(false);
    value["child"]["clock"]["commit_available"] = json!(true);
    value["child"]["clock"]["pending"] = json!(true);
    value["child"]["clock"]["active_run"] = json!({"key":"tree-clock-original",
        "state":"prepared","review_seal":"6".repeat(64),"game_ticks":100,
        "native_unpause_attempted":false});
    value
}

#[test]
fn tree_designation_and_clock_reviews_keep_their_actual_scopes() -> TestResult {
    let mut value = ready();
    value["child"]["clock"] = Value::Null;
    value["child"]["original"]["prepared_in_this_connection"] = json!(true);
    value["child"]["original"]["native_phase"] = json!(1);
    let output = shown(value.clone(), reference(&value));
    let suggestion = &output["agent_turn"]["recommendations"][0];
    assert_eq!(suggestion["tool"], "fortress_commit");
    assert_eq!(suggestion["arguments"]["scope"], "goal");
    assert_eq!(suggestion["arguments"]["review_seal"], "b".repeat(64));
    assert_eq!(suggestion["estimated_cost"]["game_ticks"], 0);
    assert_eq!(suggestion["risk"], "irreversible");
    let value = prepared_clock();
    let output = shown(value.clone(), reference(&value));
    let suggestion = &output["agent_turn"]["recommendations"][0];
    assert_eq!(suggestion["tool"], "fortress_commit");
    assert_eq!(suggestion["arguments"]["scope"], "clock");
    assert_eq!(suggestion["arguments"]["review_seal"], "6".repeat(64));
    assert_eq!(suggestion["estimated_cost"]["game_ticks"], 100);
    assert_eq!(suggestion["requires_confirmation"], true);
    let confirmations = output["agent_turn"]["active_work"]["confirmations"]
        .as_array()
        .ok_or("missing confirmations")?;
    assert_eq!(confirmations.len(), 1);
    assert_eq!(confirmations[0]["scope"], "clock");
    for case in 0..10 {
        let mut blocked = value.clone();
        match case {
            0 => blocked["child"]["clock"]["active_run"]["state"] = json!("intent_recorded"),
            1 => blocked["child"]["clock"]["active_run"]["state"] = json!("tracking"),
            2 => blocked["child"]["clock"]["active_run"]["state"] = json!("terminal"),
            3 => blocked["child"]["clock"]["unresolved"] = json!(true),
            4 => blocked["child"]["clock"]["pending"] = json!(false),
            5 => blocked["child"]["clock"]["verified"] = json!(false),
            6 => blocked["child"]["clock"]["advancement_fenced"] = json!(true),
            7 => blocked["child"]["clock"]["active_run"]["review_seal"] = json!(""),
            8 => blocked["child"]["retained_state_currently_verified"] = json!(false),
            _ => {
                blocked["mode"] = json!("recover");
                blocked["child"]["mode"] = json!("recover");
            }
        }
        let output = shown(blocked.clone(), reference(&blocked));
        assert_ne!(
            output["agent_turn"]["recommendations"][0]["tool"], "fortress_commit",
            "case {case}"
        );
        assert_eq!(
            output["agent_turn"]["active_work"]["confirmations"],
            json!([]),
            "case {case}"
        );
    }
    Ok(())
}

#[test]
fn tree_clock_proposal_requires_the_real_target_and_complete_remaining_budget() -> TestResult {
    let value = ready();
    let output = shown(value.clone(), reference(&value));
    let suggestion = &output["agent_turn"]["recommendations"][0];
    assert_eq!(
        parsed_request(suggestion)?,
        json!({"kind":"run_tree",
        "target_id":"c".repeat(64),"game_ticks":100,"wall_millis":1000})
    );
    assert_eq!(suggestion["requires_confirmation"], false);
    assert_eq!(suggestion["estimated_cost"]["game_ticks"], 0);
    for case in 0..10 {
        let mut blocked = value.clone();
        match case {
            0 => blocked["budget"]["max_game_ticks"] = json!(0),
            1 => blocked["child"]["max_game_ticks"] = json!(0),
            2 => blocked["child"]["clock"]["clock_ready"] = json!(false),
            3 => blocked["child"]["clock"]["remaining_horizon_ticks"] = json!(99),
            4 => blocked["child"]["clock"]["remaining_tree_samples"] = json!(2),
            5 => blocked["child"]["clock"]["deadline_tick"] = json!(901),
            6 => blocked["child"]["original"]["deadline_tick"] = json!(901),
            7 => blocked["child"]["clock"]["advancement_fenced"] = json!(true),
            8 => blocked["child"]["original"]["sample_publication_pending"] = json!(true),
            _ => blocked["child"]["clock"]["recommended_game_ticks"] = json!(1201),
        }
        assert!(run_request(&blocked).is_none(), "case {case}");
        let output = shown(blocked.clone(), reference(&blocked));
        assert_ne!(
            output["agent_turn"]["recommendations"][0]["tool"], "fortress_plan",
            "case {case}"
        );
    }
    Ok(())
}

#[test]
fn in_flight_tree_clock_keeps_wait_and_authorized_safety_stop_after_project_cancel() -> TestResult {
    for cancelled in [false, true] {
        let mut value = prepared_clock();
        value["child"]["clock"]["active_run"]["state"] = json!("tracking");
        value["child"]["clock"]["active_run"]["native_unpause_attempted"] = json!(true);
        value["child"]["clock"]["unresolved"] = json!(true);
        value["project_cancelled"] = json!(cancelled);
        let mut retained = reference(&value);
        if cancelled {
            value["stage"] = json!("cancelled");
            retained["retained_stage"]["stage"] = json!("cancelled");
            retained["retained_stage"]["cancelled_from"] = json!("tree");
        }
        let output = shown(value.clone(), retained.clone());
        let suggestions = output["agent_turn"]["recommendations"]
            .as_array()
            .ok_or("missing suggestions")?;
        assert_eq!(suggestions.len(), 2);
        assert_eq!(suggestions[0]["tool"], "fortress_wait");
        assert_eq!(suggestions[0]["arguments"]["scope"], "clock");
        assert_eq!(suggestions[1]["tool"], "fortress_cancel");
        assert_eq!(suggestions[1]["arguments"]["scope"], "clock");
        assert_eq!(
            suggestions[1]["estimated_cost"]["native_writer_calls_max"],
            1
        );
        assert_eq!(
            output["agent_turn"]["active_work"]["confirmations"],
            json!([])
        );
        assert_eq!(
            output["agent_turn"]["briefing"]["original_goal_proven"],
            false
        );
        assert_eq!(output["retained_reference"], retained);
        value["mode"] = json!("recover");
        value["child"]["mode"] = json!("recover");
        let output = shown(value, retained);
        let suggestions = output["agent_turn"]["recommendations"]
            .as_array()
            .ok_or("missing recovered suggestions")?;
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0]["tool"], "fortress_wait");
    }
    Ok(())
}

#[test]
fn lost_tree_clock_review_retires_only_the_unattempted_interval() -> TestResult {
    let mut value = prepared_clock();
    value["child"]["clock"]["commit_available"] = json!(false);
    value["child"]["clock"]["active_run"]["review_seal"] = Value::Null;
    let output = shown(value.clone(), reference(&value));
    let suggestion = &output["agent_turn"]["recommendations"][0];
    assert_eq!(suggestion["tool"], "fortress_cancel");
    assert_eq!(suggestion["arguments"]["scope"], "clock");
    assert_eq!(suggestion["estimated_cost"]["native_writer_calls_max"], 0);
    assert_eq!(suggestion["estimated_cost"]["game_ticks"], 0);
    value["mode"] = json!("recover");
    value["child"]["mode"] = json!("recover");
    let output = shown(value.clone(), reference(&value));
    assert_eq!(
        output["agent_turn"]["recommendations"][0]["tool"],
        "fortress_explain"
    );
    assert_eq!(
        output["agent_turn"]["active_work"]["confirmations"],
        json!([])
    );
    Ok(())
}

#[test]
fn terminal_tree_clock_uncertainty_cannot_become_a_review_or_new_wait_loop() -> TestResult {
    let mut value = prepared_clock();
    value["child"]["clock"]["active_run"]["state"] = json!("terminal");
    value["child"]["clock"]["active_run"]["native_phase"] = json!("source_lost");
    value["child"]["clock"]["unresolved"] = json!(true);
    let output = shown(value.clone(), reference(&value));
    let suggestions = output["agent_turn"]["recommendations"]
        .as_array()
        .ok_or("missing suggestions")?;
    assert_eq!(suggestions.len(), 1);
    assert_eq!(suggestions[0]["tool"], "fortress_explain");
    assert_eq!(
        output["agent_turn"]["active_work"]["confirmations"],
        json!([])
    );
    assert!(
        !output["agent_turn"]["active_work"]["indeterminate_effects"]
            .as_array()
            .ok_or("missing uncertainty")?
            .is_empty()
    );
    assert_eq!(
        output["agent_turn"]["active_work"]["actions"][0]["current_pause_proven"],
        false
    );
    Ok(())
}

fn completed() -> Value {
    let mut value = ready();
    value["child"]["original"]["historical_supply_observed_tick"] = json!(200);
    value["child"]["clock"]["new_run_available"] = json!(false);
    value
}

#[test]
fn historical_tree_completion_unlocks_only_exact_head_handoff_and_never_final_stock_proof()
-> TestResult {
    let mut value = completed();
    // Even an erroneous aggregate success flag cannot promote another stage's proof.
    value["original_goal_proven"] = json!(true);
    value["child"]["original"]["fresh_supply_observed"] = json!(true);
    let output = shown(value.clone(), reference(&value));
    let suggestion = &output["agent_turn"]["recommendations"][0];
    assert_eq!(
        parsed_request(suggestion)?,
        json!({"kind":"advance_to_workshop",
        "expected_head":"4".repeat(64)})
    );
    assert_eq!(
        output["agent_turn"]["briefing"]["original_goal"],
        root_goal()
    );
    assert_eq!(
        output["agent_turn"]["briefing"]["original_goal_proven"],
        false
    );
    assert_eq!(
        output["agent_turn"]["briefing"]["current_goal_anchor"],
        Value::Null
    );
    assert_eq!(
        output["agent_turn"]["briefing"]["tree_supply"]["project_goal_proven"],
        false
    );
    assert_eq!(
        output["agent_turn"]["briefing"]["tree_supply"]["fresh_supply_observed_this_call"],
        true
    );
    assert_eq!(output["agent_turn"]["anchor"], Value::Null);
    for field in [
        "unresolved",
        "sample_publication_pending",
        "continuity_lost",
        "prepared_in_this_connection",
    ] {
        let mut blocked = completed();
        blocked["child"]["original"][field] = json!(true);
        blocked["child"]["original"]["review_seal"] = Value::Null;
        let output = shown(blocked.clone(), reference(&blocked));
        assert_ne!(
            output["agent_turn"]["recommendations"][0]["tool"], "fortress_plan",
            "{field}"
        );
    }
    for case in 0..6 {
        let mut blocked = completed();
        let mut retained = reference(&blocked);
        match case {
            0 => blocked["child"]["clock"]["advancement_fenced"] = json!(true),
            1 => blocked["child"]["project_source_frontier"] = json!({"fenced":true}),
            2 => blocked["child"]["clock"]["consumed_tick_floor"] = json!(900),
            3 => retained["authority_tick_floor"] = json!(900),
            4 => retained["parent_acknowledged"]["authority_tick_floor"] = json!(901),
            _ => blocked["child"]["project_source_frontier"] = json!({"high_tick":1000}),
        }
        assert!(!ready_to_handoff(&blocked, &retained), "case {case}");
        let output = shown(blocked, retained);
        assert_ne!(
            output["agent_turn"]["recommendations"][0]["tool"], "fortress_plan",
            "case {case}"
        );
    }
    let mut retained = reference(&value);
    retained["parent_acknowledged"]["head"] = Value::Null;
    let output = shown(value, retained);
    assert_eq!(
        output["agent_turn"]["recommendations"][0]["tool"],
        "fortress_query"
    );
    Ok(())
}

#[test]
fn acknowledged_tree_transition_resumes_without_another_tree_plan() -> TestResult {
    let mut value = completed();
    let mut retained = reference(&value);
    value["stage"] = json!("tree_transition");
    value["child"] = Value::Null;
    retained["retained_stage"]["stage"] = json!("tree_transition");
    retained["retained_stage"]["checkpoint"]["tree_handoff"] =
        json!({"original_key":"tree-original"});
    let output = shown(value.clone(), retained.clone());
    assert_eq!(
        parsed_request(&output["agent_turn"]["recommendations"][0])?,
        json!({"kind":"advance_to_workshop","expected_head":"4".repeat(64)})
    );
    assert_eq!(
        output["agent_turn"]["active_work"]["obligations"][0]["tree_handoff_pointer"],
        "/retained_reference/retained_stage/checkpoint/tree_handoff"
    );
    retained["retained_stage"]["checkpoint"]["tree_handoff"] = Value::Null;
    let output = shown(value, retained);
    assert_eq!(
        output["agent_turn"]["recommendations"][0]["tool"],
        "fortress_query"
    );
    Ok(())
}

#[test]
fn tree_failure_preserves_both_native_custodies_without_restored_proof_or_confirmation() {
    let value = prepared_clock();
    let mut retained = reference(&value);
    retained["child_reference"]["clock"]["unresolved"] = json!(true);
    let output = failure(
        "fortress.commit",
        Some(SessionId::new(1)),
        Some(RequestId::new(3)),
        DfmcpError::new(ErrorCode::BudgetExceeded, "publication failed"),
        retained.clone(),
    );
    assert_eq!(output["retained_reference"], retained);
    assert_eq!(
        output["agent_turn"]["briefing"]["original_goal"],
        root_goal()
    );
    assert_eq!(
        output["agent_turn"]["briefing"]["original_goal_proven"],
        false
    );
    assert_eq!(
        output["agent_turn"]["active_work"]["confirmations"],
        json!([])
    );
    assert_eq!(
        output["agent_turn"]["recommendations"][0]["tool"],
        "fortress_query"
    );
    assert_eq!(
        output["agent_turn"]["active_work"]["actions"][0]["complete_evidence_pointer"],
        "/retained_reference/child_reference/clock"
    );
    assert_eq!(
        output["agent_turn"]["active_work"]["obligations"][0]["tree_original"]["key"],
        "tree-original"
    );
    assert_eq!(
        output["agent_turn"]["briefing"]["tree_supply"]["fresh_supply_observed_this_call"],
        false
    );
}

fn reassemble(
    descriptor: &Value,
    parts: &[String],
) -> std::result::Result<String, Box<dyn std::error::Error>> {
    let mut raw = String::new();
    let mut offset = 0u64;
    for block in descriptor["blocks"].as_array().ok_or("missing blocks")? {
        assert_eq!(block["offset_bytes"], offset);
        let index = usize::try_from(
            block["content_index"]
                .as_u64()
                .ok_or("missing block index")?,
        )?;
        let content = parts.get(index).ok_or("missing block content")?;
        assert_eq!(block["byte_length"], content.len());
        assert_eq!(
            block["sha256"],
            Digest32::of_bytes(content.as_bytes()).to_hex()
        );
        offset += u64::try_from(content.len())?;
        raw.push_str(content);
    }
    assert_eq!(descriptor["byte_length"], raw.len());
    assert_eq!(
        descriptor["sha256"],
        Digest32::of_bytes(raw.as_bytes()).to_hex()
    );
    assert_eq!(descriptor["grants_authority"], false);
    Ok(raw)
}

#[test]
fn tree_recovery_chunks_preserve_unicode_exact_parent_pin_and_all_clock_custody() -> TestResult {
    let value = prepared_clock();
    let mut retained = reference(&value);
    retained["notes"] = json!(vec!["木材 and custody ".repeat(200); 24]);
    let output = failure(
        "fortress.wait",
        Some(SessionId::new(1)),
        Some(RequestId::new(4)),
        DfmcpError::new(ErrorCode::BudgetExceeded, "retain original custody"),
        retained.clone(),
    );
    let parts = texts(output.clone()).ok_or("recovery packet did not fit")?;
    let first: Value = serde_json::from_str(&parts[0])?;
    assert_eq!(
        first["retained_reference"]["parent_acknowledged"],
        retained["parent_acknowledged"]
    );
    let raw = reassemble(&first["retained_reference"]["complete_recovery"], &parts)?;
    assert_eq!(serde_json::from_str::<Value>(&raw)?, retained);
    assert!(parts.iter().all(|part| part.len() < 65_536));
    assert!(render(output).is_some());
    assert_eq!(
        first["agent_turn"]["active_work"]["confirmations"],
        json!([])
    );
    Ok(())
}

#[test]
fn compact_tree_review_preserves_complete_semantic_terms_without_claiming_raw_chunks() -> TestResult
{
    let mut value = offered();
    let commitment = json!({"schema":"dfmcp.tree-project-capture-commitment/1",
        "byte_length":128 * 1024,"sha256":"9".repeat(64),"raw_bytes_inline":false,
        "retained_by":"current_tree_owner"});
    let approval = json!({"root":[15,15,2],"species":7,"tree_dimensions":[3,4,5],
        "computed_designation":[16,16,2],"effect_scope":[0,0,2,31,31,6],
        "host_effect_scope":[0,0,0,63,63,7],"protected":[],
        "visible_domain_complete":true,"visible_cell_count":224,"body_cell_count":60,
        "belowground_roots_observed":false,"paused_at_anchor":true,"mature":true,
        "alive":true,"existing_felling_job":false,"eligible_at_anchor":true,
        "workers":[{"id":11,"position":[14,16,2],"facts":7,"eligible_at_anchor":true}],
        "tools":[{"item_id":23,"position":[15,16,2],"holder_id":31,"role":2,
            "flags":0,"flags2":0,"facts":3,"offered_at_anchor":true,
            "equipment_or_scheduling_proven":false}],
        "checkpoint":"disposable-fortress-no-checkpoint","lease_ticks":1200,
        "authority_expires_tick":20_000,"project_deadline_tick":900,
        "future_falling_safety_proven":false,"global_controller_fence":false});
    value["child"]["observation"]["native_capture_commitment"] = commitment.clone();
    value["child"]["observation"]["approval"] = approval.clone();
    let output = shown(value.clone(), reference(&value));
    let parts = texts(output.clone()).ok_or("compact Tree packet did not fit")?;
    assert_eq!(parts.len(), 1);
    let first: Value = serde_json::from_str(&parts[0])?;
    assert_eq!(
        first["result"]["child"]["observation"]["native_capture_commitment"],
        commitment
    );
    assert_eq!(
        first["result"]["child"]["observation"]["approval"],
        approval
    );
    assert!(
        first["result"]["child"]["observation"]
            .get("native_capture_hex")
            .is_none()
    );
    assert!(
        first["result"]["child"]["observation"]["native_capture_commitment"]
            .get("blocks")
            .is_none()
    );
    assert!(render(output).is_some());
    Ok(())
}

#[test]
fn oversized_tree_review_requires_withdrawal_and_retains_compact_recovery() {
    let mut value = prepared_clock();
    value["child"]["oversized_semantic_evidence"] = json!("x".repeat(300_000));
    let retained = reference(&value);
    assert!(render(shown(value, retained.clone())).is_none());
    let output = failure(
        "fortress.plan",
        Some(SessionId::new(1)),
        Some(RequestId::new(5)),
        DfmcpError::new(ErrorCode::BudgetExceeded, "review was withdrawn"),
        retained.clone(),
    );
    assert_eq!(output["retained_reference"], retained);
    assert_eq!(
        output["agent_turn"]["active_work"]["confirmations"],
        json!([])
    );
    assert_eq!(
        output["agent_turn"]["briefing"]["development_review_available"],
        false
    );
    assert!(render(output).is_some());
}
