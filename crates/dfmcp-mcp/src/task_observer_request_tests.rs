use super::*;

fn request() -> Value {
    json!({"session_id":"a".repeat(32),"plan_digest":"b".repeat(64),"as_task":true})
}

#[test]
fn observer_work_has_a_distinct_closed_schema_and_preserves_commit_descriptor_bytes()
-> McpResult<()> {
    let legacy =
        json!({"schema":WORK_SCHEMA,"session_id":"a".repeat(32),"plan_digest":"b".repeat(64)});
    let work = TaskWork::parse(&legacy)?;
    assert!(!work.is_observer());
    assert_eq!(work.cancellation_scope(), "original_plan");
    assert_eq!(
        serde_json::to_value(work).map_err(|error| McpError::internal_error(error.to_string()))?,
        legacy
    );

    let observer = observation_request(&request())?;
    assert!(observer.is_observer());
    assert_eq!(observer.monitor_kind(), "observe_original");
    assert_eq!(observer.cancellation_scope(), "monitor_only");
    let encoded = serde_json::to_value(observer)
        .map_err(|error| McpError::internal_error(error.to_string()))?;
    assert_eq!(
        encoded,
        json!({"schema":OBSERVATION_WORK_SCHEMA,
        "session_id":"a".repeat(32),"plan_digest":"b".repeat(64)})
    );
    for field in ["operation", "mode", "restore_dispatch_handles"] {
        let mut changed = encoded.clone();
        changed[field] = json!(true);
        assert!(TaskWork::parse(&changed).is_err(), "{field}");
    }
    let mut unknown = encoded;
    unknown["schema"] = json!("dfmcp.lab-original-observer-task-work/2");
    assert!(TaskWork::parse(&unknown).is_err());
    Ok(())
}

#[test]
fn observer_request_accepts_no_clock_and_refuses_other_wait_selectors_or_malformed_values()
-> McpResult<()> {
    for ticks in [None, Some(Value::Null), Some(json!(0))] {
        let mut valid = request();
        if let Some(ticks) = ticks {
            valid["max_game_ticks"] = ticks;
        }
        valid["plan_digests"] = Value::Null;
        valid["plan_policy"] = Value::Null;
        assert!(observation_request(&valid)?.is_observer());
    }
    for (field, invalid) in [
        ("max_game_ticks", json!(1)),
        ("max_game_ticks", json!(-1)),
        ("max_game_ticks", json!("0")),
        ("max_game_ticks", json!(0.5)),
        ("watch_id", json!("original-watch")),
        ("watch_ids", json!([])),
        ("watch_policy", json!("all_satisfied")),
        ("plan_digests", json!(["b".repeat(64)])),
        ("plan_digests", json!([])),
        ("plan_policy", json!("all")),
        ("as_task", json!(false)),
        ("as_task", Value::Null),
        ("as_task", json!("true")),
        ("plan_digest", json!("B".repeat(64))),
        ("plan_digest", json!("b".repeat(63))),
        ("plan_digest", Value::Null),
        ("session_id", Value::Null),
        ("actions", json!([])),
    ] {
        let mut changed = request();
        changed[field] = invalid;
        assert!(observation_request(&changed).is_err(), "{changed}");
    }
    for field in ["plan_digest", "session_id", "as_task"] {
        let mut missing = request();
        if let Some(fields) = missing.as_object_mut() {
            fields.remove(field);
        }
        assert!(observation_request(&missing).is_err());
    }
    assert!(observation_request(&Value::Null).is_err());
    Ok(())
}

#[test]
fn task_aware_wait_keeps_the_existing_wire_name_and_declares_explicit_monitor_only_semantics() {
    let tool = TaskAwareWait.definition();
    assert_eq!(tool.name, "fortress_wait");
    assert_eq!(
        tool.input_schema["properties"]["as_task"]["type"],
        "boolean"
    );
    assert!(TaskAwareWait.declares_final_tasks());
    for field in [
        "plan_digest",
        "plan_digests",
        "plan_policy",
        "watch_id",
        "watch_ids",
    ] {
        assert!(
            tool.input_schema["properties"].get(field).is_some(),
            "{field}"
        );
    }
    let description = tool.input_schema["properties"]["as_task"]["description"]
        .as_str()
        .unwrap_or("");
    assert!(description.contains("tasks/cancel stops only its observation"));
    assert!(description.contains("zero max_game_ticks"));
}

#[test]
fn fitted_observer_outcome_keeps_recovery_identity_and_complete_action_census() {
    let compact = compact_selected_outcome(&json!({"selected_plan_wait":{
        "requested_plan_digest":"a".repeat(64),"plan_digest":"b".repeat(64),
        "complete":true,"inspection_complete":true,"stop_reason":"complete",
        "goal_truth":"true","stage_goal_truth":"true",
        "selection_origin":"durable_recovery","selection_scope":"exact_recovered_original",
        "original_owner_session_id":"c".repeat(32),"action_count":2,
        "verified_actions":2,"not_dispatched_actions":0,
    }}));
    assert_eq!(compact["selection_origin"], "durable_recovery");
    assert_eq!(compact["original_owner_session_id"], "c".repeat(32));
    assert_eq!(compact["action_count"], 2);
    assert_eq!(compact["verified_actions"], 2);
    assert_eq!(compact["not_dispatched_actions"], 0);
    assert_eq!(compact["dispatch_handles_restored"], false);
    assert_eq!(compact["complete"], true);
}

#[test]
fn set_observer_request_preserves_order_defaults_policy_and_keeps_old_descriptor_bytes()
-> McpResult<()> {
    for schema in [WORK_SCHEMA, OBSERVATION_WORK_SCHEMA] {
        let work = TaskWork::parse(&json!({"schema":schema,
            "session_id":"a".repeat(32),"plan_digest":"b".repeat(64)}))?;
        assert_eq!(
            serde_json::to_string(&work)
                .map_err(|error| McpError::internal_error(error.to_string()))?,
            format!(
                "{{\"schema\":\"{schema}\",\"session_id\":\"{}\",\"plan_digest\":\"{}\"}}",
                "a".repeat(32),
                "b".repeat(64)
            )
        );
    }
    let digests = vec!["d".repeat(64), "b".repeat(64)];
    for requested_policy in [None, Some("all"), Some("any")] {
        let mut request = json!({"as_task":true,"session_id":"A".repeat(32),
            "plan_digests":digests,"max_game_ticks":0});
        if let Some(policy) = requested_policy {
            request["plan_policy"] = json!(policy);
        }
        let work = observation_request(&request)?;
        assert!(work.is_set_observer());
        assert!(work.is_observer());
        assert_eq!(work.monitor_kind(), "observe_original_set");
        assert_eq!(work.cancellation_scope(), "monitor_only");
        assert!(work.plan_digest().is_none());
        assert!(work.original_digest().is_err());
        assert_eq!(
            work.plan_set(),
            Some((digests.as_slice(), requested_policy.unwrap_or("all")))
        );
        let encoded = serde_json::to_value(work)
            .map_err(|error| McpError::internal_error(error.to_string()))?;
        assert_eq!(
            encoded,
            json!({"schema":SET_OBSERVATION_WORK_SCHEMA,
            "session_id":"a".repeat(32),"plan_digests":digests,
            "plan_policy":requested_policy.unwrap_or("all")})
        );
    }
    Ok(())
}

#[test]
fn set_observer_descriptor_rejects_mixed_unknown_duplicate_or_unbounded_work() -> McpResult<()> {
    let request = json!({"as_task":true,"session_id":"a".repeat(32),
        "plan_digests":["b".repeat(64),"d".repeat(64)],"plan_policy":"any"});
    let encoded = serde_json::to_value(observation_request(&request)?)
        .map_err(|error| McpError::internal_error(error.to_string()))?;
    for (field, invalid) in [
        ("schema", json!(OBSERVATION_WORK_SCHEMA)),
        ("plan_digest", json!("b".repeat(64))),
        ("plan_policy", json!("any_complete")),
        ("plan_policy", Value::Null),
        ("plan_digests", json!([])),
        ("plan_digests", json!(["b".repeat(64), "b".repeat(64)])),
        ("plan_digests", json!(["B".repeat(64)])),
        ("plan_digests", json!(["b".repeat(63)])),
        ("plan_digests", json!([1])),
        ("plan_digests", json!("b".repeat(64))),
        (
            "plan_digests",
            json!(
                (0..9)
                    .map(|index| format!("{index:064x}"))
                    .collect::<Vec<_>>()
            ),
        ),
        ("restore_dispatch_handles", json!(true)),
    ] {
        let mut changed = encoded.clone();
        changed[field] = invalid;
        assert!(TaskWork::parse(&changed).is_err(), "{changed}");
    }
    for (field, invalid) in [
        ("plan_digest", json!("b".repeat(64))),
        ("plan_digest", json!(17)),
        ("plan_policy", json!(false)),
        ("plan_digests", json!(["b".repeat(64), null])),
        ("max_game_ticks", json!(1)),
        ("watch_id", json!("watch-1")),
        ("watch_policy", json!("all_satisfied")),
        ("ignored", json!(true)),
    ] {
        let mut changed = request.clone();
        changed[field] = invalid;
        assert!(observation_request(&changed).is_err(), "{changed}");
    }
    Ok(())
}

#[test]
fn fitted_set_outcomes_keep_every_peer_currentness_and_negative_endpoint() -> McpResult<()> {
    let digests = vec!["b".repeat(64), "c".repeat(64)];
    let work = observation_request(&json!({"as_task":true,"session_id":"a".repeat(32),
        "plan_digests":digests,"plan_policy":"any"}))?;
    let anchor = json!({"game_tick":1234,"sequence":17});
    let mut payload = json!({
        "group_complete":true,"all_originals_complete":false,"proof_status":"completed",
        "action_proof_status":"working","physical_quiescent":false,
        "remaining_work":1,"cleanup_required":false,
        "physical_work":{"state":"active","quiescent":false,"active_actions":1},
        "observed_anchor":anchor,"actual_plan_digests":digests,
        "selected_plan_set_wait":{
            "schema":"dfmcp.selected-plan-set-wait/1","plan_policy":"any",
            "plan_digests":digests,"requested_plan_digests":digests,
            "group_complete":true,"all_complete":false,"inspection_complete":true,
            "stop_reason":"any_complete","unfinished_plan_digests":[digests[1]],
            "interrupted_plan_digests":[],"earlier_completion_is_latched":false,
            "plans":[
                {"requested_plan_digest":digests[0],"plan_digest":digests[0],
                    "complete":true,"inspection_complete":true,"stop_reason":"complete",
                    "goal_truth":"true","stage_goal_truth":"true","physical_quiescent":true,
                    "action_count":1,"verified_actions":1,"terminal_actions":1,
                    "active_work_actions":0,"unknown_work_actions":0},
                {"requested_plan_digest":digests[1],"plan_digest":digests[1],
                    "complete":false,"inspection_complete":true,"stop_reason":null,
                    "goal_truth":"false","stage_goal_truth":"false","physical_quiescent":false,
                    "action_count":1,"verified_actions":0,"terminal_actions":0,
                    "active_work_actions":1,"unknown_work_actions":0,
                    "source_work":{"large_diagnostic":"retained source ".repeat(8192)}}
            ]
        }
    });
    let summary =
        task_group_evidence::compact_set_terminal(&work, "monitor-1", &payload, None, 8192, 32768);
    assert_eq!(summary["plan_digests"], json!(digests));
    assert!(summary.get("plan_digest").is_none());
    assert_eq!(summary["action_proof_status"], "working");
    assert_eq!(summary["physical_quiescent"], false);
    assert_eq!(summary["cleanup_required"], false);
    assert_eq!(
        summary["selected_plan_set_wait"]["plans"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
    assert_eq!(
        summary["selected_plan_set_wait"]["unfinished_plan_digests"],
        json!([digests[1]])
    );
    assert_eq!(
        summary["selected_plan_set_wait"]["plans"][1]["active_work_actions"],
        1
    );
    assert_eq!(
        summary["evidence"]["digest"],
        dfmcp_core::Digest32::of_bytes(payload.to_string().as_bytes()).to_hex()
    );
    assert_eq!(
        summary["evidence"]["next"],
        format!("df://session/{}/task-monitor-1~evidence-0", "a".repeat(32))
    );

    payload["selected_plan_set_wait"]["group_complete"] = json!(false);
    payload["selected_plan_set_wait"]["stop_reason"] = json!("reserve_unmet");
    payload["selected_plan_set_wait"]["plans"][1]["stop_reason"] = json!("reserve_unmet");
    payload["selected_plan_set_wait"]["plans"][1]["goal_truth"] = json!("true");
    payload["selected_plan_set_wait"]["plans"][1]["reserve_frontier"] = json!({
        "reserve_until_tick":1234,"observed_anchor":anchor,"truth":"false",
        "available":true,"durable":true,"remaining_work_required":false,
        "error":{"message":"retained diagnostics ".repeat(4096)}});
    let failed = task_group_evidence::compact_set_terminal(
        &work,
        "monitor-2",
        &payload,
        Some("reserve failed"),
        8192,
        32768,
    );
    let member = &failed["selected_plan_set_wait"]["plans"][1];
    assert_eq!(member["goal_truth"], "true");
    assert_eq!(member["complete"], false);
    assert_eq!(member["stop_reason"], "reserve_unmet");
    assert_eq!(member["reserve_frontier"]["truth"], "false");
    assert_eq!(member["reserve_frontier"]["observed_anchor"], anchor);
    assert_eq!(
        failed["selected_plan_set_wait"]["plans"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
    assert!(failed.to_string().len() < 12_000);
    Ok(())
}

#[test]
fn admitted_set_identity_is_fixed_while_current_truth_and_anchor_can_change() -> McpResult<()> {
    let requested = "b".repeat(64);
    let actual = "c".repeat(64);
    let work = observation_request(&json!({"as_task":true,"session_id":"a".repeat(32),
        "plan_digests":[requested],"plan_policy":"all"}))?;
    let mut view = json!({"observed_anchor":{"game_tick":1200},"selected_plan_set_wait":{
        "plan_policy":"all","plan_digests":[actual],"requested_plan_digests":[requested],
        "plans":[{"requested_plan_digest":requested,"plan_digest":actual,
            "plan_id":"original-1","plan_object_digest":"d".repeat(64),
            "original_owner_session_id":"a".repeat(32),"goal_truth":"true",
            "physical_quiescent":true,"complete":true}]}});
    let binding = work.admitted_set_binding(&view)?;
    view["observed_anchor"]["game_tick"] = json!(1300);
    view["selected_plan_set_wait"]["plans"][0]["goal_truth"] = json!("false");
    view["selected_plan_set_wait"]["plans"][0]["complete"] = json!(false);
    assert!(work.matches_admitted_set(&binding, &view));
    for field in [
        "requested_plan_digest",
        "plan_digest",
        "plan_id",
        "plan_object_digest",
        "original_owner_session_id",
    ] {
        let mut changed = view.clone();
        changed["selected_plan_set_wait"]["plans"][0][field] = json!("different-original");
        assert!(!work.matches_admitted_set(&binding, &changed), "{field}");
    }
    let unknown = task_group_evidence::unresolved_set_outcome(&binding);
    assert_eq!(unknown["requested_plan_digests"], json!([requested]));
    assert_eq!(unknown["plan_digests"], json!([actual]));
    assert_eq!(unknown["inspection_complete"], false);
    assert_eq!(unknown["plans"][0]["goal_truth"], "unknown");
    assert!(unknown["plans"][0]["physical_quiescent"].is_null());
    assert!(unknown["plans"][0]["action_count"].is_null());
    assert!(unknown["observed_anchor"].is_null());
    assert_eq!(unknown["admitted_anchor"]["game_tick"], 1200);
    Ok(())
}
