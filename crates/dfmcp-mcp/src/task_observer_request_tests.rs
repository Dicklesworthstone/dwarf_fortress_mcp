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
