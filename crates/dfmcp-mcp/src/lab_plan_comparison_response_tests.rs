//! Complete publication of shared plan proof remains an atomic read contract.

use super::*;
use dfmcp_core::WorkBudget;

fn projection() -> TemporalResponseProjection {
    let mut active_work = crate::agent_turn::empty_active_work();
    active_work["actions"] = json!([{"action_id":"previous-harvest", "state":"Verified",
        "work_state":{"state":"quiescent", "quiescent":true}}]);
    active_work["carried_obligations"] =
        json!([{"action_id":"recovered-step", "state":"indeterminate"}]);
    TemporalResponseProjection {
        session_id: "801".to_owned(),
        request_id: "12".to_owned(),
        anchor: json!({"fortress_id":"19", "epoch":1, "sequence":3,
            "game_tick":101, "state_hash":"b".repeat(64)}),
        paused: false,
        budget: WorkBudget {
            max_bytes: 65_536,
            max_output_tokens: 16_384,
            ..WorkBudget::default()
        },
        active_work,
        objectives: json!([{"plan_digest":"old-plan", "status":"achieved",
            "predicate_truth":"true", "semantic_output_goal":{"predicate_truth":"false"},
            "resource_reserve":{"status":"work_required", "epistemic":"predicted"},
            "physical_quiescent":true, "needs_replan":true}]),
    }
}

fn payload(view: &TemporalResponseProjection) -> Value {
    let digest = "a".repeat(64);
    json!({
        "ok":true, "kind":"resource_production_continuation",
        "schema":"dfmcp.resource-production/2", "version":2,
        "plan_digest":digest, "session_id":view.session_id, "anchor":view.anchor,
        "terminal_condition":{"$ref":"/predicates/0"},
        "steps":[{"step":0, "kind":"create_work_order", "capability":"configure_production",
            "risk":"reversible", "depends_on":[], "compensable":false,
            "postconditions":[{"$ref":"/predicates/1"}],
            "obligation":{"terminal":{"$ref":"/predicates/1"}, "deadline_tick":501,
                "poll_interval_ticks":10, "stable_observations":2}}],
        "predicates":[
            {"field":{"entity_id":"9000", "field":"stock.item.BED", "op":"ge", "value":22}},
            {"field":{"entity_id":"9001", "field":"amount_remaining", "op":"eq", "value":0}}
        ],
        "predicate_encoding":{"schema":"dfmcp.plan-predicate-references/1", "scope":"same_response",
            "plan_digest":digest, "values_pointer":"/predicates", "complete":true,
            "projection":"readable_predicate_json", "exact_typed_contract":"retained_plan_archive",
            "order":"terminal_then_steps_postconditions_then_obligation_children_first"},
        "production":{"resource_workflow":{"schema":"dfmcp.production-resource-workflow/2",
            "stage":"production", "original_request":{"template":"production",
                "quotas":[{"item":"BED", "minimum":22}], "reserve_until_tick":6001}},
            "continuation":{"parent_plan_digest":"parent", "root_plan_digest":"root",
                "reserve_until_tick":6001, "requires_explicit_commit":true}},
        "forecast":{"epistemic_state":"predicted", "predicted_complete":true,
            "reserve_forecast":{"predicted_goal_truth":"true", "continuous_maintenance_proven":false}},
        "source_work":{"records":20, "canonical_bytes":4096},
        "max_risk":"reversible", "requires_checkpoint":false,
        "next_step":{"tool":"fortress.commit", "arguments":{
            "session_id":view.session_id, "plan_digest":digest}}
    })
}

fn pending(input: &Value) -> Value {
    json!({"plan_digest":input["plan_digest"], "state":"prepared", "anchor":input["anchor"],
        "step_count":1, "steps_reference":{"scope":"same_response", "json_pointer":"/steps",
            "plan_digest":input["plan_digest"], "complete":true},
        "minimum_safe_next_step":input["next_step"]})
}

fn query_work() -> Value {
    json!({"query_baselines":[{"baseline":"qb1:retained"}],
        "query_watches":[], "condition_watches":[]})
}

#[test]
fn shared_review_publishes_all_proofs_current_work_and_goal_distinctions() -> Result<()> {
    let view = projection();
    let input = payload(&view);
    let output = finish(&view, input.clone(), query_work(), Some(pending(&input)))?;
    let result: Value = serde_json::from_str(&output).map_err(|_| invariant("test packet"))?;
    for field in [
        "steps",
        "predicates",
        "predicate_encoding",
        "terminal_condition",
        "production",
        "forecast",
    ] {
        assert_eq!(result[field], input[field], "complete {field}");
    }
    assert_eq!(
        result["agent_turn"]["briefing"]["objectives"],
        view.objectives
    );
    for field in ["actions", "carried_obligations"] {
        assert_eq!(
            result["agent_turn"]["active_work"][field],
            view.active_work[field]
        );
    }
    assert_eq!(
        result["agent_turn"]["active_work"]["query_baselines"],
        query_work()["query_baselines"]
    );
    let reference = &result["agent_turn"]["active_work"]["pending_plans"][0]["steps_reference"];
    assert_eq!(reference["complete"], true);
    assert_eq!(
        reference["predicate_encoding_pointer"],
        "/predicate_encoding"
    );
    assert_eq!(result["publication"], "complete_before_pending_plan_change");
    assert_eq!(result["game_effect"], "none");
    Ok(())
}

#[test]
fn publication_rejects_incomplete_or_misbound_shared_proof_before_budgeting() {
    let view = projection();
    for case in 0..7 {
        let mut input = payload(&view);
        match case {
            0 => input["predicates"] = json!([]),
            1 => input["steps"][0]["postconditions"][0] = json!({"$ref":"/predicates/7"}),
            2 => input["predicate_encoding"]["plan_digest"] = json!("c".repeat(64)),
            3 => input["predicate_encoding"]["complete"] = json!(false),
            4 => input["schema"] = json!("dfmcp.resource-production/1"),
            5 => input["version"] = json!(1),
            _ => input["predicates"][0] = json!({"not":{"$ref":"/predicates/0"}}),
        }
        assert!(
            finish(&view, input.clone(), query_work(), Some(pending(&input))).is_err(),
            "case {case} published a partial or mismatched proof"
        );
    }
}

#[test]
fn the_complete_shared_proof_is_refused_instead_of_shortened_at_its_byte_boundary() -> Result<()> {
    let mut view = projection();
    let input = payload(&view);
    // All measured byte ceilings have four digits, so the admitted budget
    // fields retain their width at the exact publication boundary.
    view.budget.max_bytes = 9_999;
    let full = finish(&view, input.clone(), query_work(), Some(pending(&input)))?;
    assert!((1_001..9_999).contains(&full.len()));
    view.budget.max_bytes = full.len() as u64;
    let exact = finish(&view, input.clone(), query_work(), Some(pending(&input)))?;
    assert_eq!(exact.len(), full.len());
    view.budget.max_bytes -= 1;
    let refusal = finish(&view, input.clone(), query_work(), Some(pending(&input)));
    assert!(matches!(refusal, Err(error) if error.code == ErrorCode::BudgetExceeded));
    assert_eq!(
        input,
        payload(&projection()),
        "rendering changed input custody"
    );
    Ok(())
}

#[test]
fn shared_review_keeps_one_complete_current_anchor_and_reconstructible_work() -> Result<()> {
    let mut view = projection();
    view.active_work["actions"][0]["work_state"]["observed_anchor"] = view.anchor.clone();
    view.objectives[0]["observed_anchor"] = view.anchor.clone();
    view.objectives[0]["semantic_output_goal"]["observed_anchor"] = view.anchor.clone();
    let historical = json!({"fortress_id":"19", "epoch":1, "sequence":0,
        "game_tick":1, "state_hash":"c".repeat(64)});
    view.objectives[0]["sealed_anchor"] = historical.clone();
    let input = payload(&view);
    let output = finish(&view, input.clone(), query_work(), Some(pending(&input)))?;
    let result: Value = serde_json::from_str(&output).map_err(|_| invariant("test packet"))?;
    assert_eq!(result["current_anchor"], view.anchor);
    assert_eq!(result["agent_turn"]["anchor"], view.anchor);
    assert_eq!(
        result["anchor_encoding"]["schema"],
        "dfmcp.current-anchor-references/1"
    );
    assert_eq!(result["anchor_encoding"]["complete"], true);
    assert_eq!(result["anchor_encoding"]["reference_count"], 4);
    for pointer in [
        "/agent_turn/active_work/pending_plans/0/anchor",
        "/agent_turn/active_work/actions/0/work_state/observed_anchor",
        "/agent_turn/briefing/objectives/0/observed_anchor",
        "/agent_turn/briefing/objectives/0/semantic_output_goal/observed_anchor",
    ] {
        let reference = result
            .pointer(pointer)
            .ok_or_else(|| invariant("missing shared anchor"))?;
        assert_eq!(reference, &json!({"$ref":"/current_anchor"}));
        assert_eq!(
            result.pointer(reference["$ref"].as_str().unwrap_or_default()),
            Some(&view.anchor)
        );
    }
    assert_eq!(
        result["agent_turn"]["briefing"]["objectives"][0]["sealed_anchor"],
        historical
    );
    assert_eq!(
        result["agent_turn"]["briefing"]["objectives"][0]["semantic_output_goal"]["predicate_truth"],
        "false"
    );
    assert_eq!(
        result["agent_turn"]["active_work"]["carried_obligations"],
        view.active_work["carried_obligations"]
    );
    let mut supplied = input.clone();
    supplied["anchor_encoding"] = result["anchor_encoding"].clone();
    assert!(finish(&view, supplied, query_work(), Some(pending(&input))).is_err());
    Ok(())
}
