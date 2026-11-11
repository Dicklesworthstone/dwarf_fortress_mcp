//! Complete publication preserves shared proof, current anchors and goal truth.

mod shared_predicates {
    //! Complete publication of shared plan proof remains an atomic read contract.

    use super::super::*;
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
    fn compact_review_preserves_complete_proof_current_work_and_exact_budget_boundary() -> Result<()> {
        let mut view = projection();
        let mut input = payload(&view);
        input["steps"][0]["postconditions"] =
            json!(vec![json!({"$ref":"/predicates/1"}); 64]);
        let original_steps = crate::sealed_plan_review::expanded_steps(&input)?;
        view.budget.max_bytes = 9_999;
        let full = finish(&view, input.clone(), query_work(), Some(pending(&input)))?;
        let v1: Value = serde_json::from_str(&full).map_err(|_| invariant("test packet"))?;
        assert_eq!(
            v1["predicate_encoding"]["schema"],
            "dfmcp.plan-predicate-references/1"
        );
        view.budget.max_bytes = (full.len() - 1) as u64;
        let shrunk = finish(&view, input.clone(), query_work(), Some(pending(&input)))?;
        let result: Value = serde_json::from_str(&shrunk).map_err(|_| invariant("test packet"))?;
        assert_eq!(
            result["predicate_encoding"]["schema"],
            "dfmcp.plan-predicate-references/2"
        );
        assert_eq!(crate::sealed_plan_review::expanded_steps(&result)?, original_steps);
        assert_eq!(
            result["agent_turn"]["briefing"]["objectives"],
            v1["agent_turn"]["briefing"]["objectives"]
        );
        assert_eq!(
            result["agent_turn"]["active_work"],
            v1["agent_turn"]["active_work"]
        );
        for field in ["production", "forecast", "anchor", "current_anchor", "anchor_encoding"] {
            assert_eq!(result[field], v1[field], "{field}");
        }
        assert!(shrunk.len() < full.len());
        view.budget.max_bytes = shrunk.len() as u64;
        assert_eq!(
            finish(&view, input.clone(), query_work(), Some(pending(&input)))?.len(),
            shrunk.len()
        );
        view.budget.max_bytes -= 1;
        assert!(
            matches!(finish(&view, input.clone(), query_work(), Some(pending(&input))),
                Err(error) if error.code == ErrorCode::BudgetExceeded)
        );
        assert_eq!(crate::sealed_plan_review::expanded_steps(&input)?, original_steps);
        Ok(())
    }

    #[test]
    fn ordinary_reserve_continuation_shares_complete_review_without_resource_acquisition() -> Result<()> {
        let view = projection();
        let mut input = payload(&view);
        input["kind"] = json!("production_reserve_continuation");
        input["schema"] = json!("dfmcp.production-reserve/1");
        input["version"] = json!(1);
        input["production"]
            .as_object_mut()
            .ok_or_else(|| invariant("test production"))?
            .remove("resource_workflow");
        input["production"]["consumption"] = json!({"reserve_until_tick":6001});
        let result: Value = serde_json::from_str(
            &finish(&view, input.clone(), query_work(), Some(pending(&input)))?
        ).map_err(|_| invariant("test packet"))?;
        assert_eq!(result["publication"], "complete_before_pending_plan_change");
        assert_eq!(result["production"], input["production"]);
        assert_eq!(
            crate::sealed_plan_review::expanded_steps(&result)?,
            crate::sealed_plan_review::expanded_steps(&input)?
        );
        assert_eq!(
            result["agent_turn"]["active_work"]["pending_plans"][0]["steps_reference"]["predicate_encoding_pointer"],
            "/predicate_encoding"
        );
        let mut malformed = input.clone();
        malformed["version"] = json!(2);
        assert!(finish(&view, malformed, query_work(), Some(pending(&input))).is_err());
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

    #[test]
    fn shared_anchor_counts_remain_exact_when_historical_goal_artifacts_move_to_details() -> Result<()> {
        for literal_field in ["original_source", "terminal_condition"] {
            let mut view = projection();
            view.budget.max_output_tokens = 4_096;
            view.objectives[0]["plan_digest"] = json!("d".repeat(64));
            view.objectives[0]["original_source"] = json!({"retained": "x".repeat(16_384)});
            view.objectives[0]["terminal_condition"] = json!({"retained": "y".repeat(16_384)});
            // These two artifacts are opaque data. A reference-shaped value here
            // must never be charged as a declared current-anchor reference.
            view.objectives[0][literal_field] = json!({"$ref": "/current_anchor"});
            for field in [
                "sealed_anchor",
                "first_satisfied_anchor",
                "recorded_first_satisfied_anchor",
                "observed_anchor",
            ] {
                view.objectives[0][field] = view.anchor.clone();
            }
            view.objectives[0]["semantic_output_goal"]["observed_anchor"] = view.anchor.clone();
            view.active_work["actions"][0]["work_state"]["observed_anchor"] = view.anchor.clone();
            let input = payload(&view);
            let output = finish(&view, input.clone(), query_work(), Some(pending(&input)))?;
            assert!(output.len() <= 4 * 4_096);
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
            assert_eq!(result["anchor_encoding"]["reference_count"], 4);
            assert_eq!(result["anchor_encoding"]["complete"], true);
            for pointer in [
                "/agent_turn/active_work/pending_plans/0/anchor",
                "/agent_turn/active_work/actions/0/work_state/observed_anchor",
                "/agent_turn/briefing/objectives/0/observed_anchor",
                "/agent_turn/briefing/objectives/0/semantic_output_goal/observed_anchor",
            ] {
                assert_eq!(
                    result.pointer(pointer),
                    Some(&json!({"$ref": "/current_anchor"})),
                    "retained anchor {pointer}"
                );
            }
            assert_eq!(result["current_anchor"], view.anchor);
            let briefing = &result["agent_turn"]["briefing"];
            let goal = &briefing["objectives"][0];
            for field in [
                "plan_digest",
                "status",
                "predicate_truth",
                "resource_reserve",
                "physical_quiescent",
                "needs_replan",
            ] {
                assert_eq!(goal[field], view.objectives[0][field], "goal {field}");
            }
            assert_eq!(goal["semantic_output_goal"]["predicate_truth"], "false");
            assert_eq!(
                goal["details"],
                format!("df://session/801/goal-{}", "d".repeat(64))
            );
            for field in [
                "original_source",
                "terminal_condition",
                "sealed_anchor",
                "first_satisfied_anchor",
                "recorded_first_satisfied_anchor",
            ] {
                assert!(goal.get(field).is_none(), "archived {field} stayed inline");
            }
            assert_eq!(briefing["objective_projection"]["complete_record_membership"], true);
            assert_eq!(briefing["objective_projection"]["complete_goal_evidence"], false);
        }
        Ok(())
    }
}

mod objective_briefing {
    //! Complete proposal publication must not grow with archived proof detail.

    use super::super::*;
    use crate::agent_turn::empty_active_work;
    use dfmcp_core::WorkBudget;

    fn projection(tokens: u32) -> TemporalResponseProjection {
        let anchor = json!({"fortress_id":"7", "epoch":1, "sequence":3,
            "game_tick":101, "state_hash":"a".repeat(64)});
        let mut active_work = empty_active_work();
        active_work["actions"] = json!([{"action_id":"original-harvest", "state":"Verified",
            "work_state":{"physical_quiescent":true}}]);
        active_work["indeterminate_effects"] = json!([{"action_id":"other-action", "state":"unknown"}]);
        active_work["carried_obligations"] =
            json!([{"plan_digest":"retained", "proof":"retained_not_reobserved"}]);
        let goal = json!({
            "plan_digest":"b".repeat(64), "summary":"retain original supplies through winter",
            "original_source":{"kind":"production", "request":{"unmodified":"x".repeat(8_192)}},
            "terminal_condition":{"original":"y".repeat(8_192)},
            "sealed_anchor":anchor, "first_satisfied_anchor":anchor,
            "recorded_first_satisfied_anchor":anchor, "observed_anchor":anchor,
            "status":"satisfied", "predicate_truth":"true", "historical_achievement":"verified",
            "semantic_output_goal":{"predicate_truth":"true", "observed_anchor":anchor,
                "historical_action_proof_rewritten":false},
            "resource_reserve":{"status":"work_required", "epistemic":"predicted",
                "reserve_until_tick":12_001, "continuous_maintenance_proven":false},
            "physical_quiescent":true, "needs_replan":true, "pursuit_active":false,
            "continuation_request":{"tool":"fortress.plan", "arguments":{"session_id":"71",
                "blueprint":"original continuation request"}},
            "blind_retry_allowed":false, "future_safety_field":{"must_survive":true},
        });
        TemporalResponseProjection {
            session_id: "71".to_owned(),
            request_id: "13".to_owned(),
            anchor,
            paused: false,
            budget: WorkBudget {
                max_bytes: 1_048_576,
                max_output_tokens: tokens,
                ..WorkBudget::default()
            },
            active_work,
            objectives: json!([goal]),
        }
    }

    fn candidate(view: &TemporalResponseProjection) -> (Value, Value, Value) {
        let digest = "c".repeat(64);
        let payload = json!({
            "ok":true, "kind":"resource_production_continuation", "session_id":view.session_id,
            "anchor":view.anchor, "plan_digest":digest, "max_risk":"guarded",
            "steps":[{"step":1, "kind":"create_work_order", "postconditions":[
                {"kind":"original_order_proof", "original_quantity":22}],
                "obligation":{"deadline_tick":3_000, "stable_observations":2}}],
            "production":{"resource_workflow":{"stage":"production", "original_request":{
                "quotas":[{"item":"BED", "minimum":22}], "reserve_until_tick":6_001}},
                "continuation":{"parent_plan_digest":"b".repeat(64), "root_plan_digest":"b".repeat(64),
                    "source_schema":"dfmcp.production-continuation/5"}},
            "forecast":{"epistemic_state":"predicted", "available":false, "reason":"model unavailable"},
            "source_work":{"records":21, "bytes":1_024},
            "next_step":{"tool":"fortress.commit", "arguments":{"session_id":view.session_id, "plan_digest":digest}},
        });
        let pending = json!({"plan_digest":digest, "state":"prepared", "step_count":1,
            "steps_reference":{"scope":"same_response", "json_pointer":"/steps",
                "plan_digest":digest, "complete":true}});
        let queries = json!({
            "query_baselines":[{"baseline":"retained-baseline"}],
            "query_watches":[{"watch":"pending-watch", "status":"active"}],
            "condition_watches":[{"watch":"unknown-condition", "truth":"unknown"}],
        });
        (payload, pending, queries)
    }

    fn decoded(raw: &str) -> Result<Value> {
        serde_json::from_str(raw).map_err(|_| invariant("invalid test JSON"))
    }

    #[test]
    fn compact_prior_goals_keep_current_truth_complete_candidate_and_every_active_record() -> Result<()>
    {
        let view = projection(4_096);
        let (payload, pending, queries) = candidate(&view);
        let raw = finish(
            &view,
            payload.clone(),
            queries.clone(),
            Some(pending.clone()),
        )?;
        assert!(raw.len() <= 4 * 4_096);
        let result = decoded(&raw)?;
        for field in [
            "steps",
            "production",
            "forecast",
            "source_work",
            "next_step",
        ] {
            assert_eq!(result[field], payload[field], "candidate {field} changed");
        }
        let work = &result["agent_turn"]["active_work"];
        for (field, value) in view
            .active_work
            .as_object()
            .ok_or_else(|| invariant("work"))?
        {
            if field != "pending_plans" {
                assert_eq!(work[field], *value, "current {field} changed");
            }
        }
        assert_eq!(work["pending_plans"], json!([pending]));
        for field in ["query_baselines", "query_watches", "condition_watches"] {
            assert_eq!(work[field], queries[field]);
        }
        let summary = &result["agent_turn"]["briefing"]["objectives"][0];
        for field in [
            "plan_digest",
            "summary",
            "status",
            "predicate_truth",
            "historical_achievement",
            "resource_reserve",
            "physical_quiescent",
            "needs_replan",
            "pursuit_active",
            "continuation_request",
            "blind_retry_allowed",
            "future_safety_field",
        ] {
            assert_eq!(
                summary[field], view.objectives[0][field],
                "goal {field} changed"
            );
        }
        assert_eq!(summary["semantic_output_goal"]["predicate_truth"], "true");
        assert_eq!(summary["observed_anchor_reference"], "/anchor");
        assert_eq!(
            summary["semantic_output_goal"]["observed_anchor_reference"],
            "/anchor"
        );
        assert_eq!(result.pointer("/anchor"), Some(&view.anchor));
        let projection = &result["agent_turn"]["briefing"]["objective_projection"];
        assert_eq!(projection["records"], 1);
        assert_eq!(projection["complete_record_membership"], true);
        assert_eq!(projection["complete_goal_evidence"], false);
        assert_eq!(projection["details_field"], "details");
        assert_eq!(
            summary["details"],
            format!("df://session/71/goal-{}", "b".repeat(64))
        );
        assert_eq!(result["agent_turn"]["continuity"]["status"], "partial");
        assert_eq!(
            result["agent_turn"]["uncertainty"][0]["epistemic_state"],
            "predicted"
        );
        assert_eq!(
            raw,
            finish(&view, payload, queries, Some(candidate(&view).1))?
        );
        Ok(())
    }

    #[test]
    fn compact_goal_projection_never_relabels_missing_unknown_or_historical_anchors() -> Result<()> {
        let mut view = projection(4_096);
        let mut historical = view.anchor.clone();
        historical["sequence"] = json!(1);
        historical["game_tick"] = json!(1);
        view.objectives[0]["observed_anchor"] = historical.clone();
        view.objectives[0]["semantic_output_goal"] = json!({"predicate_truth":"unknown",
            "observed_anchor":null, "error":{"code":"preconditions_failed", "message":"unqualified stock"}});
        view.objectives[0]["semantic_room_goal"] = json!({"truth":"unknown", "error":"missing source"});
        view.objectives[0]["continuation_refused"] = json!({"code":"capability_denied"});
        let (payload, pending, queries) = candidate(&view);
        let output = decoded(&finish(&view, payload, queries, Some(pending))?)?;
        let goal = &output["agent_turn"]["briefing"]["objectives"][0];
        assert_eq!(goal["observed_anchor"], historical);
        assert!(goal.get("observed_anchor_reference").is_none());
        for field in [
            "semantic_output_goal",
            "semantic_room_goal",
            "continuation_refused",
        ] {
            assert_eq!(goal[field], view.objectives[0][field]);
        }
        Ok(())
    }

    #[test]
    fn fitting_proposals_keep_the_complete_original_briefing() -> Result<()> {
        let view = projection(16_384);
        let (payload, pending, queries) = candidate(&view);
        let result = decoded(&finish(&view, payload, queries, Some(pending))?)?;
        assert_eq!(
            result["agent_turn"]["briefing"]["objectives"],
            view.objectives
        );
        assert!(
            result["agent_turn"]["briefing"]
                .get("objective_projection")
                .is_none()
        );
        Ok(())
    }

    #[test]
    fn candidate_proof_and_active_work_overflow_still_refuse_without_partial_success() {
        for source in ["steps", "active_work", "bytes", "tokens"] {
            let mut view = projection(4_096);
            let (mut payload, pending, queries) = candidate(&view);
            match source {
                "steps" => payload["steps"][0]["required_proof"] = json!("x".repeat(16_384)),
                "active_work" => {
                    view.active_work["indeterminate_effects"] = json!([{"evidence":"x".repeat(16_384)}])
                }
                "bytes" => view.budget.max_bytes = 2_048,
                _ => view.budget.max_output_tokens = 512,
            }
            assert!(
                matches!(finish(&view, payload, queries, Some(pending)),
                Err(error) if error.code == ErrorCode::BudgetExceeded),
                "{source} was clipped"
            );
        }
    }

    #[test]
    fn historical_anchor_projection_refuses_missing_or_inconsistent_reference_counts() {
        for count in [Value::Null, json!(0), json!(-1), json!(1.5), json!("1")] {
            let mut payload = json!({
                "session_id": "71",
                "anchor": {"source": "current"},
                "anchor_encoding": {"reference_count": count},
                "agent_turn": {
                    "briefing": {"objectives": [{
                        "plan_digest": "b".repeat(64),
                        "sealed_anchor": {"$ref": "/current_anchor"}
                    }]},
                    "coverage": {"partial_domains": []}
                }
            });
            assert!(
                matches!(compact_objective_briefing(&mut payload),
                    Err(error) if error.code == ErrorCode::InternalInvariantViolation)
            );
        }
    }
}
