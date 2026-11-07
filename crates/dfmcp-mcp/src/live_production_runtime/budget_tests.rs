use super::*;

#[test]
fn prepaid_work_cannot_be_reused_or_renew_the_operation_deadline() -> Result<()> {
    let c = OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: StateAnchor {
            fortress_id: dfmcp_core::FortressId::new(1),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(1),
            state_hash: Digest32::of_bytes(b"combined budget component"),
        },
        budget: WorkBudget {
            max_bytes: 128 * MIB,
            max_wall_millis: MAX_WALL_MILLIS,
            max_actions: 2,
            max_entities: 8192,
            max_output_tokens: MAX_OUTPUT_TOKENS,
            max_game_ticks: 0,
        },
        grants: Vec::new(),
        cancellation_requested: false,
    };
    let mut work = Work::new(&c)?;
    let deadline = work.until;
    let output = work.take(32 * MIB)?;
    let evidence = work.take(64 * MIB)?;
    assert!(work.take(33 * MIB).is_err());
    let final_work = work.rest(MAX_FRONTIER_BYTES)?;
    assert_eq!(
        output.budget.max_bytes + evidence.budget.max_bytes + final_work.budget.max_bytes,
        c.budget.max_bytes
    );
    assert_eq!(work.bytes, 0);
    assert!(work.take(1).is_err());
    assert_eq!(work.until, deadline);
    for reserved in [output, evidence, final_work] {
        assert_eq!(reserved.session_id, c.session_id);
        assert_eq!(reserved.request_id, c.request_id);
        assert_eq!(reserved.anchor, c.anchor);
        assert_eq!(reserved.budget.max_game_ticks, 0);
        assert!(reserved.budget.max_wall_millis <= c.budget.max_wall_millis);
    }
    work.until = Instant::now();
    assert!(work.current().is_err());
    assert!(work.rest(MAX_FRONTIER_BYTES).is_err());
    let mut large = c;
    large.budget.max_bytes = MAX_BYTES;
    let mut parent = Work::new(&large)?;
    let child = parent.rest(MAX_FRONTIER_BYTES)?;
    assert_eq!(child.budget.max_bytes, 2 * 1024 * MIB);
    assert_eq!(parent.bytes, MAX_BYTES - child.budget.max_bytes);
    // A long retained history is not trapped behind the empty-history bound.
    // The final publication/custody contexts remain prepaid and cannot be spent
    // a second time by replay, even at the largest configured operation size.
    let mut recovery = Work::new(&large)?;
    let publication = recovery.take(256 * MIB)?;
    let custody = recovery.take(512 * MIB)?;
    let final_check = recovery.take(1)?;
    let replay = goals::history_budget(&mut recovery, false)?;
    assert!(replay.budget.max_bytes > 30 * 1024 * MIB);
    assert_eq!(recovery.bytes, 0);
    assert_eq!(
        publication.budget.max_bytes
            + custody.budget.max_bytes
            + final_check.budget.max_bytes
            + replay.budget.max_bytes,
        large.budget.max_bytes
    );
    recovery.until = Instant::now();
    assert!(recovery.narrow(&final_check).is_err());
    let mut initial = Work::new(&large)?;
    let initial = goals::history_budget(&mut initial, true)?.budget.max_bytes;
    assert_eq!(initial, 1536 * MIB);
    // Cross-check the empty-history reservation against the bounded native
    // custody path and the actual child codec limits, not the size of this
    // fixture's tiny journals. A 144 MiB semantic workforce inventory charges
    // two native views, so each subsequent native-view cost is below 72 MiB.
    let selected_custody = (160 + 32 + 162 + 7 + 8 * 72) * MIB;
    let order_view = 2 * dfmcp_adapter::semantic_work_orders::store::MAX_STORE_BYTES as u64
        + 2 * dfmcp_adapter::semantic_work_orders::store::MAX_ASSOCIATIONS as u64
            * dfmcp_adapter::work_order_control::MAX_RECORD_BYTES as u64
        + 4096;
    let history_reader =
        8 * dfmcp_adapter::semantic_work_orders::goal_history::MAX_GOAL_HISTORY_BYTES as u64 + MIB;
    let header_checks = 2 * (MIB + 112 + 8192);
    let maximum_initialization =
        selected_custody + history_reader + 3 * order_view + header_checks + 1;
    assert!(maximum_initialization < 1080 * MIB);
    assert!(initial > maximum_initialization);
    Ok(())
}

#[test]
fn machine_host_contract_preserves_bounded_development_scope() -> Result<()> {
    let contract: Value = serde_json::from_str(include_str!(
        "../../../../architecture/live_production_executor_v1.json"
    ))
    .map_err(|_| invalid("invalid production contract JSON"))?;
    let host = &contract["development_mcp_host"];
    assert_eq!(
        host["cli_profile"],
        json!(["serve-gameplay-dev", "labor-order"])
    );
    assert_eq!(host["wall_millis_max"], MAX_WALL_MILLIS);
    assert_eq!(host["byte_work_max"], MAX_BYTES);
    assert_eq!(host["game_ticks_default"], 0);
    assert_eq!(host["game_ticks_max"], 1200);
    assert_eq!(
        host["original_steps"],
        json!(["SetLabor", "CreateWorkOrder"])
    );
    assert_eq!(
        host["fixed_child_protocols"],
        json!(["workforce/1.17", "work-orders/1.10"])
    );
    assert_eq!(
        host["parent_files"],
        json!(["original.plan", "original.citizens", "execution.journal"])
    );
    assert_eq!(host["tool_names"].as_array().map(Vec::len), Some(11));
    let choices = &host["workforce_choices"];
    assert_eq!(choices["schema"], "dfmcp.combined-workforce-choices/1");
    assert_eq!(choices["options_max"], super::choices::MAX_CHOICES);
    assert_eq!(
        choices["selected_citizens_max"],
        dfmcp_adapter::workforce_control::MAX_UNITS
    );
    for flag in [
        "source_tick_pause_and_exact_selection_checked",
        "consumed_observation_tick_floor_retained_on_error",
        "source_witness_and_coverage_omissions_explicit",
        "query_and_observe_required",
        "native_preparation_and_commit_revalidate",
    ] {
        assert_eq!(choices[flag], true, "{flag}");
    }
    for flag in [
        "mutating_precondition_evidence",
        "mutation_authority",
        "retained_after_original_plan",
        "restored_by_recovery",
    ] {
        assert_eq!(choices[flag], false, "{flag}");
    }
    let paths = host["source_paths"]
        .as_array()
        .ok_or_else(|| invalid("missing source inventory"))?;
    for file in ["choices.rs", "choices_tests.rs", "clock.rs"] {
        assert!(paths.contains(&json!(format!(
            "crates/dfmcp-mcp/src/live_production_runtime/{file}"
        ))));
    }
    for field in [
        "recovery_restores_commit_permit",
        "wait_dispatches_or_advances_time",
        "checkpoint_or_restore_supported",
        "current_state_proven",
        "original_goal_proven_by_historical_effects",
        "new_bridge_generation",
        "production_admission_added",
    ] {
        assert_eq!(host[field], false, "{field}");
    }
    let parent = &host["current_parent_goal"];
    assert_eq!(parent["response_field"], "current_parent_goal");
    assert_eq!(
        parent["scope"],
        "complete_original_labor_and_inventory_goal"
    );
    assert_eq!(
        parent["retained_order_scope"],
        "complete_original_labor_and_order_goal"
    );
    assert_eq!(
        parent["adapter_entry"],
        "Executor::poll_selected_parent_goal"
    );
    assert_eq!(
        parent["labor_findings"]["maximum"],
        dfmcp_adapter::workforce_control::MAX_UNITS
    );
    assert_eq!(parent["labor_findings"]["agent_turn_blockers_max"], 8);
    assert_eq!(
        parent["labor_findings"]["raw_capture_is_fact_authority"],
        false
    );
    assert!(parent["labor_findings"]["unqualified_observed_enabled"].is_null());
    for field in [
        "requires_settled_original_effects",
        "child_source_policies_independently_checked",
        "same_original_identity_epoch_fortress_and_paused_tick",
        "composite_and_selected_anchors_distinct",
        "failed_acquisition_or_publication_withdraws_current_proof",
        "consumed_workforce_source_floors_retained",
        "source_or_identity_regression_fences_parent_reads",
    ] {
        assert_eq!(parent[field], true, "{field}");
    }
    for field in [
        "requires_explicit_original_inventory_objective",
        "current_state_proven_for_whole_world",
        "historical_labor_receipt_supplies_current_facts",
        "query_and_explain_acquire_new_sample",
        "restored_from_inventory_history",
        "restored_from_order_history",
        "new_deadline_or_obligation",
        "production_attribution_proven",
    ] {
        assert_eq!(parent[field], false, "{field}");
    }
    let selected = &host["selected_order_goal"];
    assert_eq!(selected["kind"], "retained_order_completion");
    assert_eq!(selected["scope"], "selected_original_order_step");
    assert_eq!(
        selected["required_order_state"],
        json!({"amount_total":"exact original requested finite amount",
            "amount_remaining":0,"validated":true,"active":false})
    );
    for field in [
        "request_plan_step_key_and_predicate_bytes_changed",
        "inventory_source_authority_added",
        "offline_replay_restores_current_proof",
        "disappearance_proves_completion",
        "preexisting_inventory_proves_completion",
        "game_time_control",
    ] {
        assert_eq!(selected[field], false, "{field}");
    }
    assert_eq!(
        host["optional_inventory_clock"]["requires_explicit_original_inventory_objective"],
        true
    );
    let clock = &host["optional_order_clock"];
    assert_eq!(clock["protocol"], "order-run/1.14");
    assert_eq!(clock["parent_recovery_field"], "clock_journal_id");
    assert_eq!(clock["new_unpause_attempts_per_run"], 1);
    assert_eq!(
        clock["supported_original_objectives"],
        json!(["retained_order_completion", "observed_inventory_minimum", "observed_inventory_increase"])
    );
    for field in [
        "requires_settled_original_effects",
        "original_deadline_cadence_stability_unchanged",
        "original_custody_rechecked_after_durable_clock_marker",
        "independent_operator_clock_authority",
        "unresolved_run_blocks_goal_sampling",
        "missing_replaced_or_omitted_journal_refused",
    ] {
        assert_eq!(clock[field], true, "{field}");
    }
    for field in [
        "requires_explicit_original_inventory_objective",
        "local_review_restored_after_restart_or_failed_publication",
        "native_clock_receipt_proves_current_pause_or_goal",
        "supports_retired_original_order",
        "existing_selected_target_digest_bytes_changed",
        "legacy_single_step_clock_target_bytes_changed",
    ] {
        assert_eq!(clock[field], false, "{field}");
    }
    Ok(())
}
