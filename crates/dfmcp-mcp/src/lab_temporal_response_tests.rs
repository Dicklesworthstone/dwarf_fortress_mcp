use super::*;
use crate::agent_turn::empty_active_work;

fn projection(bytes: u64) -> TemporalResponseProjection {
    let mut active = empty_active_work();
    active["pending_plans"] = json!([{"plan_digest":"pinned-plan","confirmation_required":true}]);
    active["actions"] = json!([{"action_id":"retained-action","state":"waiting"}]);
    active["objectives"] = json!([{"goal":"furnish first bedroom","status":"pending"}]);
    TemporalResponseProjection {
        session_id: "741".into(),
        request_id: "13".into(),
        anchor: json!({"fortress_id":"7","epoch":1,"sequence":3,"game_tick":12,"state_hash":"a".repeat(64)}),
        paused: true,
        budget: WorkBudget {
            max_bytes: bytes,
            max_output_tokens: (bytes / 4) as u32,
            ..WorkBudget::default()
        },
        active_work: active,
        objectives: json!([{"goal":"furnish first bedroom","status":"pending"}]),
    }
}

fn payload(view: &TemporalResponseProjection) -> Value {
    json!({"ok":true,"mode":"temporal","schema":"dfmcp.query.result/1",
        "session_id":view.session_id,"anchor":view.anchor,"current_anchor":view.anchor,
        "kind":"baselines","baselines":[],"truncated":false,"continuation":null,
        "_query_baseline_work":[{"kind":"query_baseline","baseline":"qb1:fixture","status":"retained"}],
        "_query_watch_work":[], "_condition_watch_work":[]})
}

fn decoded(raw: String) -> Result<Value> {
    serde_json::from_str(&raw).map_err(|_| invariant("test JSON"))
}

#[test]
fn baseline_packet_keeps_existing_pending_actions_objectives_and_exact_catalog() -> Result<()> {
    let view = projection(16_384);
    let original = payload(&view);
    let result = decoded(view.finish(original.clone())?)?;
    let work = &result["agent_turn"]["active_work"];
    for key in ["pending_plans", "actions", "objectives"] {
        assert_eq!(work[key], view.active_work[key]);
    }
    assert_eq!(work["query_baselines"], original["_query_baseline_work"]);
    assert_eq!(
        result["agent_turn"]["briefing"]["objectives"],
        view.objectives
    );
    assert!(result.get("_query_baseline_work").is_none());
    assert!(result.get("_condition_watch_work").is_none());
    assert_eq!(result["agent_turn"]["turn_id"], "lab-temporal-turn-13");
    let mode = result["query_help"]["arguments"]["mode"]
        .as_str()
        .ok_or_else(|| invariant("test mode"))?;
    let query: Value = serde_json::from_str(mode).map_err(|_| invariant("test help"))?;
    assert_eq!(query["mode"], "temporal");
    assert_eq!(query["request"]["query"]["kind"], "baselines");
    let affordance = &result["agent_turn"]["affordances"][0];
    assert_eq!(affordance["affordance_id"], "query-baselines");
    assert_eq!(affordance["intent_family"], "query");
    assert_eq!(affordance["arguments"], result["query_help"]["arguments"]);
    assert_eq!(affordance["reversibility"], "not_applicable");
    assert!(affordance["known_preconditions"].is_array());
    assert_eq!(
        result["agent_turn"]["uncertainty"][0]["epistemic_state"],
        "unknown"
    );
    assert!(
        result["agent_turn"]["uncertainty"][0]
            .get("state")
            .is_none()
    );
    Ok(())
}

#[test]
fn comparison_names_the_original_basis_without_claiming_continuous_history() -> Result<()> {
    let view = projection(16_384);
    let mut input = payload(&view);
    let mut basis = view.anchor.clone();
    basis["sequence"] = json!(1);
    basis["game_tick"] = json!(10);
    input["kind"] = json!("changes");
    input["basis"] = basis.clone();
    input["baseline"] = json!("qb1:fixture");
    input["anchor_advanced"] = json!(true);
    input["change_count"] = json!(3);
    input["returned"] = json!(1);
    input["truncated"] = json!(true);
    input["continuation"] = json!("qh1:1:bound");
    let result = decoded(view.finish(input)?)?;
    assert_eq!(result["agent_turn"]["continuity"]["basis"], basis);
    assert_eq!(result["agent_turn"]["continuity"]["status"], "partial");
    assert_eq!(
        result["agent_turn"]["coverage"]["continuation"],
        result["continuation"]
    );
    assert_eq!(
        result["agent_turn"]["coverage"]["query_temporal_coverage"]["continuous_between_observations"],
        false
    );
    assert_eq!(result["agent_turn"]["changes"][0]["change_count"], 3);
    assert_eq!(result["agent_turn"]["changes"][0]["basis"], basis);
    assert_eq!(result["agent_turn"]["changes"][0]["target"], view.anchor);
    assert_eq!(
        result["agent_turn"]["changes"][0]["epistemic_state"],
        "inferred"
    );
    assert_eq!(
        decoded(view.finish(payload(&view))?)?["agent_turn"]["continuity"]["status"],
        "heartbeat"
    );
    Ok(())
}

#[test]
fn overflow_never_clips_rows_or_hides_current_game_work() -> Result<()> {
    let view = projection(8192);
    assert!(view.result_byte_budget()? > 0);
    let mut input = payload(&view);
    input["changes"] = json!([{"data":"x".repeat(8192)}]);
    assert!(matches!(view.finish(input),Err(error) if error.code==ErrorCode::BudgetExceeded));
    let mut crowded = projection(8192);
    crowded.active_work["pending_plans"] = json!([{"data":"x".repeat(8192)}]);
    assert!(
        matches!(crowded.result_byte_budget(),Err(error) if error.code==ErrorCode::BudgetExceeded)
    );
    assert!(projection(4).result_byte_budget().is_err());
    Ok(())
}

#[test]
fn a_reserved_result_and_complete_catalog_fit_with_all_required_context() -> Result<()> {
    let view = projection(16_384);
    let remaining = view.result_byte_budget()?;
    let mut input = payload(&view);
    input["rows"] = json!([{"value":"z".repeat(remaining.saturating_sub(1024))}]);
    let output = view.finish(input)?;
    assert!(output.len() <= 16_384);
    let result = decoded(output)?;
    assert!(
        result["rows"][0]["value"]
            .as_str()
            .is_some_and(|value| value.len() > 1000)
    );
    assert_eq!(
        result["agent_turn"]["active_work"]["pending_plans"],
        view.active_work["pending_plans"]
    );
    assert_eq!(
        result["agent_turn"]["coverage"]["absence_proof_scope"],
        json!([])
    );
    Ok(())
}

#[test]
fn changed_anchor_foreign_session_and_malformed_work_refuse() {
    let view = projection(16_384);
    for case in 0..5 {
        let mut value = payload(&view);
        match case {
            0 => value["anchor"]["sequence"] = json!(999),
            1 => value["current_anchor"]["sequence"] = json!(999),
            2 => value["session_id"] = json!("other"),
            3 => value["_query_baseline_work"] = json!({}),
            _ => value["_condition_watch_work"] = json!({"watch":"malformed"}),
        }
        assert!(
            matches!(view.finish(value),Err(error) if error.code==ErrorCode::InternalInvariantViolation)
        );
    }
}

#[test]
fn watch_packet_preserves_game_obligations_and_both_exact_watch_catalogs() -> Result<()> {
    let mut view = projection(16_384);
    view.active_work["obligations"] = json!([{"action_id":"game-action", "status":"waiting"}]);
    let mut input = payload(&view);
    input["kind"] = json!("poll_watches");
    input["records"] = json!([{"watch":"watch:active", "status":"candidate"}]);
    input["_query_watch_work"] = json!([
        {"watch":"watch:active", "terminal":false},
        {"watch":"watch:terminal", "terminal":true},
    ]);
    input["_condition_watch_work"] = json!([{"watch":"watch:active", "terminal":false}]);
    let output = decoded(view.finish(input.clone())?)?;
    let work = &output["agent_turn"]["active_work"];
    assert_eq!(work["obligations"], view.active_work["obligations"]);
    assert_eq!(work["query_watches"], input["_query_watch_work"]);
    assert_eq!(work["condition_watches"], input["_condition_watch_work"]);
    assert_eq!(work["query_baselines"], input["_query_baseline_work"]);
    assert!(output.get("_query_watch_work").is_none());
    assert!(output.get("_condition_watch_work").is_none());
    assert_eq!(output["agent_turn"]["continuity"]["status"], "partial");
    assert_eq!(
        output["agent_turn"]["coverage"]["query_temporal_coverage"]["status"],
        "explicit_observation_samples_only"
    );
    assert_eq!(
        output["agent_turn"]["coverage"]["query_temporal_coverage"]["eligible_as_mutation_evidence"],
        false
    );
    let help: Value = serde_json::from_str(
        output["query_help"]["arguments"]["mode"]
            .as_str()
            .ok_or_else(|| invariant("watch help"))?,
    )
    .map_err(|_| invariant("help JSON"))?;
    assert_eq!(help["request"]["query"]["kind"], "watches");
    Ok(())
}

#[test]
fn full_watch_catalog_overflow_is_refused_without_clipping_terminal_or_active_work() {
    let view = projection(8192);
    let mut input = payload(&view);
    input["kind"] = json!("watch");
    input["_query_watch_work"] = json!([{"watch":"retained", "label":"x".repeat(8192)}]);
    assert!(matches!(view.finish(input), Err(error) if error.code == ErrorCode::BudgetExceeded));
    for key in ["_query_watch_work", "_condition_watch_work"] {
        let mut input = payload(&view);
        input[key] = json!(vec![json!({"watch":"retained"}); 9]);
        assert!(
            matches!(view.finish(input), Err(error) if error.code == ErrorCode::InternalInvariantViolation)
        );
    }
}
