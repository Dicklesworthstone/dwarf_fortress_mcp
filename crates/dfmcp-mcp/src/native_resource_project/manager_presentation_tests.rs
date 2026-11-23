//! Agent guidance over the actual Manager owner's field shape. These fixtures
//! exercise safe next steps and complete output, not native source qualification.
use super::super::{failure, packet, render, texts};
use super::*;
use dfmcp_core::{DfmcpError, ErrorCode, RequestId};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn goal() -> Value {
    json!({"recipe":"wooden_bed","minimum_stack_units":5,"max_order_amount":20,
        "deadline_tick":900,"poll_interval_ticks":10,"stable_samples":2,"assign_labor":false})
}

fn source() -> Value {
    json!({"profile":"manager/1.23","witness":"a".repeat(64),"tick":100,
        "generation":7,"sequence":3,"paused":true,"eligible_at_anchor":true,
        "unit_id":256,"histfig_id":361,"manager_position_id":4,
        "assignment_id":12,"assignment_flags_hex":"0100","history_count":8,
        "next_event_id":9,"lookup_unit":-1,"manager_links":0,
        "group_role_buckets":[[],[],[],[],[]],"civ_role_buckets":[[],[],[],[],[]]})
}

fn offered() -> Value {
    json!({"ok":true,"mode":"control","stage":"manager","project_cancelled":false,
        "project_custody_currently_verified":true,
        "original":{"goal":goal(),"manager_policy":"e".repeat(64)},
        "original_goal_proven":false,
        "child":{"ok":true,"source_profile":"manager/1.23","mode":"control",
            "retained_state_currently_verified":true,"original":null,
            "observation":source(),"recommended_deadline_tick":12_100,
            "project_deadline_tick":900,"production_handoff_available":false,
            "project_source_frontier":{"high_tick":100,"high_generation":7,
                "high_sequence":3,"fenced":false},
            "custody":{"consumed_tick_floor":100,"dispatch_started":false,
                "uncertain_effect":false,"production_handoff_floor":null,
                "native_anchor":{"schema":"dfmcp.manager-native-projection-anchor/1",
                    "fortress_id":"1","generation":7,"sequence":3,"game_tick":100,
                    "state_hash":"a".repeat(64),"canonical_world_anchor":false}}}})
}

fn prepared() -> Value {
    let mut result = offered();
    result["child"]["observation"] = Value::Null;
    result["child"]["original"] = json!({"key":"manager-original",
        "plan_digest":"b".repeat(64),"review_seal":"c".repeat(64),
        "unit_id":256,"histfig_id":361,"deadline_tick":900,"state":"Prepared",
        "native_phase":0,"prepared_in_this_connection":true,"unresolved":false,
        "appointment_proved":false,"production_manager_cache_proved":false,
        "original_source":source(),"after":null});
    result
}

fn appointed() -> Value {
    let mut result = prepared();
    result["child"]["original"]["state"] = json!("Terminal");
    result["child"]["original"]["native_phase"] = json!(2);
    result["child"]["original"]["prepared_in_this_connection"] = json!(false);
    result["child"]["original"]["appointment_proved"] = json!(true);
    result["child"]["original"]["production_manager_cache_proved"] = json!(true);
    result["child"]["original"]["after"] = source();
    result["child"]["custody"]["dispatch_started"] = json!(true);
    result["child"]["production_handoff_available"] = json!(true);
    result
}

fn retained(result: &Value) -> Value {
    let child = &result["child"];
    let original = &child["original"];
    let reference = json!({
        "acknowledged":{"journal_id":"1".repeat(64),"head":"2".repeat(64),"frames":3},
        "journal_pin_unverified":{"journal_id":"1".repeat(64),"head":"2".repeat(64),"frames":3},
        "original_key":original["key"],"original_plan_digest":original["plan_digest"],
        "original_review_seal":original["review_seal"],"native_phase":original["native_phase"],
        "project_deadline_tick":900,
        "dispatch_started":child["custody"]["dispatch_started"],
        "uncertain_effect":child["custody"]["uncertain_effect"],
        "consumed_tick_floor":100,"project_source_frontier":child["project_source_frontier"],
        "production_handoff_floor":child["custody"]["production_handoff_floor"],
        "native_anchor":child["custody"]["native_anchor"],"automatic_redispatch":false,
    });
    json!({"original":result["original"],"authority_tick_floor":100,
        "parent_acknowledged":{"journal_id":"3".repeat(64),"head":"4".repeat(64),
            "frames":4,"authority_tick_floor":100},"controller_fenced":false,
        "retained_stage":{"stage":"manager","cancelled_from":null,
            "checkpoint":{"manager":reference,"manager_handoff":null,
                "tree":null,"workshop":null,"production":null}},
        "child_reference":reference})
}

fn shown(result: Value, reference: Value) -> Value {
    packet(
        "fortress.wait",
        Some(SessionId::new(1)),
        Some(RequestId::new(2)),
        result,
        reference,
    )
}

fn request(suggestion: &Value) -> std::result::Result<Value, Box<dyn std::error::Error>> {
    Ok(serde_json::from_str(
        suggestion["arguments"]["request"]
            .as_str()
            .ok_or("missing recommended request")?,
    )?)
}

#[test]
fn manager_offer_keeps_exact_candidate_witness_and_root_deadline() -> TestResult {
    let result = offered();
    let output = shown(result.clone(), retained(&result));
    let turn = &output["agent_turn"];
    let next = &turn["recommendations"][0];
    assert_eq!(next["tool"], "fortress_plan");
    assert_eq!(
        request(next)?,
        json!({"kind":"appoint_vacant_manager",
        "expected_observation":"a".repeat(64),"deadline_tick":900})
    );
    assert_eq!(next["grants_authority"], false);
    assert_eq!(next["estimated_cost"]["native_writer_calls_max"], 0);
    assert_eq!(next["estimated_cost"]["game_ticks"], 0);
    assert_eq!(turn["affordances"][0]["unit_id"], 256);
    assert_eq!(turn["affordances"][0]["histfig_id"], 361);
    assert_eq!(turn["affordances"][0]["risk"], "irreversible");
    assert_eq!(turn["briefing"]["original_goal"], goal());
    assert_eq!(turn["active_work"]["confirmations"], json!([]));
    assert!(turn["anchor"].is_null());
    assert_eq!(turn["continuity"]["status"], "indeterminate");
    assert_eq!(turn["briefing"]["original_goal_proven"], false);
    Ok(())
}

#[test]
fn manager_planning_rejects_stale_ineligible_or_unowned_offers() {
    for case in 0..12 {
        let mut result = offered();
        match case {
            0 => result["original"]["goal"]["deadline_tick"] = json!(100),
            1 => result["original"]["goal"]["deadline_tick"] = json!(12_101),
            2 => result["child"]["observation"]["eligible_at_anchor"] = json!(false),
            3 => result["child"]["observation"]["paused"] = json!(false),
            4 => result["child"]["source_profile"] = json!("manager/1.22"),
            5 => result["child"]["retained_state_currently_verified"] = json!(false),
            6 => result["child"]["observation"]["witness"] = json!("historical-label"),
            7 => result["child"]["project_source_frontier"]["fenced"] = json!(true),
            8 => result["child"]["custody"]["production_handoff_floor"] = json!({"fenced":true}),
            9 => result["project_cancelled"] = json!(true),
            10 => result["project_custody_currently_verified"] = json!(false),
            _ => {
                result["mode"] = json!("recover");
                result["child"]["mode"] = json!("recover");
            }
        }
        assert!(observed_plan(&result).is_none(), "case {case}");
        let output = shown(result.clone(), retained(&result));
        assert_ne!(
            output["agent_turn"]["recommendations"][0]["tool"], "fortress_plan",
            "case {case}"
        );
        assert_eq!(
            output["agent_turn"]["affordances"],
            json!([]),
            "case {case}"
        );
    }
    let mut opening = offered();
    let reference = retained(&opening);
    opening["child"] = json!({"opened":true});
    let output = shown(opening, reference);
    assert_eq!(
        output["agent_turn"]["recommendations"][0]["tool"],
        "fortress_observe"
    );
}

#[test]
fn manager_confirmation_requires_the_current_exact_prepared_connection() {
    let result = prepared();
    let output = shown(result.clone(), retained(&result));
    let turn = &output["agent_turn"];
    assert_eq!(turn["recommendations"][0]["tool"], "fortress_commit");
    assert_eq!(turn["recommendations"][0]["arguments"]["scope"], "goal");
    assert_eq!(
        turn["recommendations"][0]["arguments"]["review_seal"],
        "c".repeat(64)
    );
    assert_eq!(turn["recommendations"][0]["risk"], "irreversible");
    assert_eq!(turn["recommendations"][0]["requires_confirmation"], true);
    assert_eq!(
        turn["active_work"]["confirmations"][0]["complete_evidence_pointer"],
        "/result/child"
    );
    assert_eq!(
        turn["active_work"]["actions"][0]["original"]["key"],
        "manager-original"
    );
    for case in 0..11 {
        let mut result = prepared();
        match case {
            0 => result["child"]["original"]["prepared_in_this_connection"] = json!(false),
            1 => result["child"]["original"]["review_seal"] = json!("stale-review"),
            2 => result["child"]["original"]["state"] = json!("Dispatched"),
            3 => result["child"]["original"]["native_phase"] = json!(1),
            4 => result["child"]["original"]["deadline_tick"] = json!(901),
            5 => result["child"]["custody"]["uncertain_effect"] = json!(true),
            6 => result["child"]["project_source_frontier"]["high_tick"] = json!(900),
            7 => result["child"]["project_source_frontier"]["fenced"] = json!(true),
            8 => result["project_cancelled"] = json!(true),
            9 => result["child"]["source_profile"] = json!("manager/1.22"),
            _ => {
                result["mode"] = json!("recover");
                result["child"]["mode"] = json!("recover");
            }
        }
        assert!(review(&result).is_none(), "case {case}");
        let output = shown(result.clone(), retained(&result));
        assert_eq!(
            output["agent_turn"]["active_work"]["confirmations"],
            json!([]),
            "case {case}"
        );
        assert_ne!(
            output["agent_turn"]["recommendations"][0]["tool"], "fortress_commit",
            "case {case}"
        );
    }
}

#[test]
fn parent_fence_or_historical_seal_never_recreates_a_manager_review() {
    let mut result = prepared();
    let mut reference = retained(&result);
    reference["controller_fenced"] = json!(true);
    let output = shown(result.clone(), reference);
    assert_eq!(
        output["agent_turn"]["active_work"]["confirmations"],
        json!([])
    );
    assert_eq!(
        output["agent_turn"]["recommendations"][0]["tool"],
        "fortress_query"
    );
    result["child"]["original"]["prepared_in_this_connection"] = json!(false);
    let output = shown(result.clone(), retained(&result));
    assert_eq!(
        output["agent_turn"]["recommendations"][0]["tool"],
        "fortress_cancel"
    );
    assert_eq!(
        output["agent_turn"]["recommendations"][0]["arguments"]["scope"],
        "project"
    );
    assert_eq!(
        output["agent_turn"]["active_work"]["confirmations"],
        json!([])
    );
    let offer = offered();
    let mut reference = retained(&offer);
    reference["controller_fenced"] = json!(true);
    let output = shown(offer, reference);
    assert_eq!(output["agent_turn"]["affordances"], json!([]));
}

#[test]
fn uncertain_manager_effect_keeps_original_reconciliation_after_project_stop() {
    for cancelled in [false, true] {
        let mut result = prepared();
        result["child"]["original"]["state"] = json!("Dispatched");
        result["child"]["original"]["native_phase"] = json!(1);
        result["child"]["original"]["prepared_in_this_connection"] = json!(false);
        result["child"]["original"]["unresolved"] = json!(true);
        result["child"]["custody"]["uncertain_effect"] = json!(true);
        result["child"]["custody"]["dispatch_started"] = json!(true);
        let mut reference = retained(&result);
        if cancelled {
            result["stage"] = json!("cancelled");
            result["project_cancelled"] = json!(true);
            reference["retained_stage"]["stage"] = json!("cancelled");
            reference["retained_stage"]["cancelled_from"] = json!("manager");
        }
        let output = shown(result, reference);
        let turn = &output["agent_turn"];
        assert_eq!(turn["recommendations"][0]["tool"], "fortress_wait");
        assert_eq!(turn["recommendations"][0]["arguments"]["scope"], "goal");
        assert_eq!(
            turn["active_work"]["indeterminate_effects"][0]["scope"],
            "manager"
        );
        assert_eq!(
            turn["active_work"]["indeterminate_effects"][0]["automatic_redispatch"],
            false
        );
        assert_eq!(turn["active_work"]["confirmations"], json!([]));
        assert_eq!(turn["briefing"]["original_goal_proven"], false);
    }
}

#[test]
fn proved_manager_transfers_only_the_exact_parent_head_without_stock_proof() -> TestResult {
    let result = appointed();
    let output = shown(result.clone(), retained(&result));
    let turn = &output["agent_turn"];
    assert_eq!(turn["recommendations"][0]["tool"], "fortress_plan");
    assert_eq!(
        request(&turn["recommendations"][0])?,
        json!({"kind":"advance_from_manager","expected_head":"4".repeat(64)})
    );
    assert_eq!(
        turn["recommendations"][0]["estimated_cost"]["native_writer_calls_max"],
        0
    );
    assert_eq!(
        turn["briefing"]["manager_appointment"]["historical_appointment_proved"],
        true
    );
    assert_eq!(
        turn["briefing"]["manager_appointment"]["historical_cache_transaction_proved"],
        true
    );
    assert_eq!(
        turn["briefing"]["manager_appointment"]["current_production_readiness_proven"],
        false
    );
    assert_eq!(turn["briefing"]["original_goal_proven"], false);
    assert!(turn["anchor"].is_null());
    assert_eq!(turn["active_work"]["confirmations"], json!([]));
    for case in 0..9 {
        let mut result = appointed();
        let mut reference = retained(&result);
        match case {
            0 => result["child"]["original"]["production_manager_cache_proved"] = json!(false),
            1 => result["child"]["production_handoff_available"] = json!(false),
            2 => result["child"]["original"]["deadline_tick"] = json!(901),
            3 => result["child"]["custody"]["production_handoff_floor"] = json!({"fenced":true}),
            4 => result["child"]["original"]["native_phase"] = json!(4),
            5 => result["child"]["original"]["prepared_in_this_connection"] = json!(true),
            6 => reference["parent_acknowledged"]["authority_tick_floor"] = json!(900),
            7 => reference["parent_acknowledged"]["head"] = json!("old-label"),
            _ => {
                result["mode"] = json!("recover");
                result["child"]["mode"] = json!("recover");
            }
        }
        let output = shown(result, reference);
        assert_ne!(
            output["agent_turn"]["recommendations"][0]["tool"], "fortress_plan",
            "case {case}"
        );
        assert_ne!(
            output["agent_turn"]["recommendations"][0]["tool"], "fortress_commit",
            "case {case}"
        );
    }
    Ok(())
}

#[test]
fn retained_manager_transition_requires_handoff_and_forward_control() -> TestResult {
    for tree in [false, true] {
        let mut result = appointed();
        let mut reference = retained(&result);
        result["stage"] = json!("manager_transition");
        result["child"] = Value::Null;
        reference["retained_stage"]["stage"] = json!("manager_transition");
        reference["retained_stage"]["checkpoint"]["manager_handoff"] =
            json!({"schema":"dfmcp.manager-project-handoff/1","project_deadline_tick":900});
        reference["child_reference"] = Value::Null;
        if tree {
            result["original"]["tree_policy"] = json!("f".repeat(64));
            reference["original"] = result["original"].clone();
        }
        let output = shown(result.clone(), reference.clone());
        assert_eq!(
            request(&output["agent_turn"]["recommendations"][0])?,
            json!({"kind":"advance_from_manager","expected_head":"4".repeat(64)})
        );
        assert_eq!(
            output["agent_turn"]["active_work"]["obligations"][0]["manager_handoff_pointer"],
            "/retained_reference/retained_stage/checkpoint/manager_handoff"
        );
        assert_eq!(
            output["agent_turn"]["active_work"]["obligations"][0]["manager_original"]["key"],
            "manager-original"
        );
        result["mode"] = json!("recover");
        let output = shown(result.clone(), reference.clone());
        assert_eq!(
            output["agent_turn"]["recommendations"][0]["tool"],
            "fortress_query"
        );
        result["mode"] = json!("control");
        reference["retained_stage"]["checkpoint"]["manager_handoff"] = Value::Null;
        let output = shown(result, reference);
        assert_eq!(
            output["agent_turn"]["recommendations"][0]["tool"],
            "fortress_query"
        );
    }
    Ok(())
}

#[test]
fn later_resource_stages_keep_manager_custody_without_replaying_its_review() {
    let mut result = appointed();
    let mut reference = retained(&result);
    result["stage"] = json!("workshop");
    result["child"] = json!({"opened":true});
    reference["retained_stage"]["stage"] = json!("workshop");
    reference["retained_stage"]["checkpoint"]["manager_handoff"] = json!({"retained":true});
    reference["child_reference"] = json!({"original":null});
    let output = shown(result, reference);
    let turn = &output["agent_turn"];
    assert_eq!(turn["recommendations"][0]["tool"], "fortress_observe");
    assert_eq!(turn["active_work"]["confirmations"], json!([]));
    assert_eq!(
        turn["active_work"]["actions"][0]["original"]["key"],
        "manager-original"
    );
    assert_eq!(
        turn["active_work"]["actions"][0]["complete_evidence_pointer"],
        "/retained_reference/retained_stage/checkpoint/manager"
    );
    assert_eq!(
        turn["briefing"]["manager_appointment"]["handoff_retained"],
        true
    );
    assert_eq!(
        turn["briefing"]["manager_appointment"]["historical_appointment_proved"],
        false
    );
    assert_eq!(
        turn["briefing"]["manager_appointment"]["current_production_readiness_proven"],
        false
    );
}

#[test]
fn manager_failure_keeps_complete_original_and_floors_without_a_confirmable_seal() {
    let result = prepared();
    let mut reference = retained(&result);
    reference["child_reference"]["uncertain_effect"] = json!(true);
    reference["child_reference"]["production_handoff_floor"] =
        json!({"tick":450,"generation":8,"sequence":9,"fenced":true});
    let output = failure(
        "fortress.commit",
        Some(SessionId::new(1)),
        Some(RequestId::new(3)),
        DfmcpError::new(ErrorCode::BudgetExceeded, "publication failed"),
        reference.clone(),
    );
    let turn = &output["agent_turn"];
    assert_eq!(output["retained_reference"], reference);
    assert_eq!(turn["briefing"]["original_goal"], goal());
    assert_eq!(turn["briefing"]["original_goal_proven"], false);
    assert_eq!(
        turn["briefing"]["manager_appointment"]["source_fenced"],
        true
    );
    assert_eq!(turn["active_work"]["confirmations"], json!([]));
    assert_eq!(
        turn["active_work"]["actions"][0]["original"]["key"],
        "manager-original"
    );
    assert_eq!(
        turn["active_work"]["actions"][0]["complete_evidence_pointer"],
        "/retained_reference/child_reference"
    );
    assert_eq!(turn["recommendations"][0]["tool"], "fortress_query");
    assert!(turn["anchor"].is_null());
}

#[test]
fn manager_project_review_retains_semantic_approval_and_named_capture_commitment() -> TestResult {
    let mut result = prepared();
    let commitment = json!({"schema":"dfmcp.manager-project-capture-commitment/1",
        "byte_length":128_000,"sha256":"9".repeat(64),"raw_bytes_inline":false,
        "retained_by":"original_manager_journal"});
    let approval = json!({"unit_id":256,"histfig_id":361,"manager_position_id":4,
        "assignment_id":12,"assignment_flags_hex":"0100","history_events_added":1,
        "responsibility":"MANAGE_PRODUCTION","risk":"irreversible",
        "checkpoint":"disposable-fortress-no-checkpoint","lease_ticks":1200,
        "authority_expires_tick":20_000,"project_deadline_tick":900,
        "local_lease_global_fence":false,"clock_advanced":false});
    result["child"]["original"]["original_source"]["native_capture_commitment"] =
        commitment.clone();
    result["child"]["original"]["original_source"]["approval"] = approval.clone();
    let output = shown(result.clone(), retained(&result));
    let parts = texts(output.clone()).ok_or("Manager review did not fit")?;
    assert_eq!(parts.len(), 1);
    let first: Value = serde_json::from_str(&parts[0])?;
    let source = &first["result"]["child"]["original"]["original_source"];
    assert_eq!(source["native_capture_commitment"], commitment);
    assert_eq!(source["approval"], approval);
    assert!(source.get("native_capture_hex").is_none());
    assert!(source["native_capture_commitment"].get("blocks").is_none());
    assert!(render(output).is_some());
    Ok(())
}

fn reassemble(
    descriptor: &Value,
    parts: &[String],
) -> std::result::Result<String, Box<dyn std::error::Error>> {
    let mut raw = String::new();
    let mut offset = 0;
    for block in descriptor["blocks"].as_array().ok_or("missing blocks")? {
        assert_eq!(block["offset_bytes"], offset);
        let index = usize::try_from(
            block["content_index"]
                .as_u64()
                .ok_or("missing block index")?,
        )?;
        let part = parts.get(index).ok_or("missing block")?;
        assert_eq!(block["byte_length"], part.len());
        assert_eq!(
            block["sha256"],
            Digest32::of_bytes(part.as_bytes()).to_hex()
        );
        offset += part.len();
        raw.push_str(part);
    }
    assert_eq!(descriptor["byte_length"], raw.len());
    assert_eq!(
        descriptor["sha256"],
        Digest32::of_bytes(raw.as_bytes()).to_hex()
    );
    Ok(raw)
}

#[test]
fn manager_recovery_chunks_retain_unicode_originals_and_handoff_floors() -> TestResult {
    let result = appointed();
    let mut reference = retained(&result);
    reference["retained_stage"]["checkpoint"]["manager_handoff"] = json!({"project_deadline_tick":900,"manager_prerequisite_checkpoint":{"tick":100,"fenced":false}});
    reference["notes"] = json!(vec!["Manager と工房 ".repeat(200); 24]);
    let output = failure(
        "fortress.wait",
        Some(SessionId::new(1)),
        Some(RequestId::new(4)),
        DfmcpError::new(ErrorCode::BudgetExceeded, "retain original Manager custody"),
        reference.clone(),
    );
    let parts = texts(output.clone()).ok_or("Manager recovery packet did not fit")?;
    let first: Value = serde_json::from_str(&parts[0])?;
    assert_eq!(
        first["retained_reference"]["parent_acknowledged"],
        reference["parent_acknowledged"]
    );
    let raw = reassemble(&first["retained_reference"]["complete_recovery"], &parts)?;
    assert_eq!(serde_json::from_str::<Value>(&raw)?, reference);
    assert!(parts.iter().all(|part| part.len() < 65_536));
    assert_eq!(
        first["agent_turn"]["active_work"]["confirmations"],
        json!([])
    );
    assert!(render(output).is_some());
    Ok(())
}

#[test]
fn oversized_manager_review_refuses_and_keeps_complete_nonconfirming_recovery() {
    let mut result = prepared();
    let reference = retained(&result);
    result["child"]["oversized_approval"] = json!("x".repeat(300_000));
    assert!(render(shown(result, reference.clone())).is_none());
    let output = failure(
        "fortress.plan",
        Some(SessionId::new(1)),
        Some(RequestId::new(5)),
        DfmcpError::new(ErrorCode::BudgetExceeded, "review withdrawn"),
        reference.clone(),
    );
    assert_eq!(output["retained_reference"], reference);
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
