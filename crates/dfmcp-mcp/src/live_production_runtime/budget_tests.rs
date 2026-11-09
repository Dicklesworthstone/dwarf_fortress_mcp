use super::*;

#[test]
fn stock_target_dispatch_budget_fits_with_workshop_and_native_frontier() {
    let source = 33 * MIB
        + dfmcp_adapter::semantic_work_orders::INVENTORY_BASELINE_PROJECTION_BYTES
        + dfmcp_adapter::semantic_work_orders::INVENTORY_BASELINE_QUALIFY_BYTES
        + orders::native::STOCK_TARGET_QUEUE_BYTES;
    assert_eq!(stock_target::CHECK_BYTES, source);
    // Six reads cover both semantic observation and native preparation, with
    // the existing-worker reread. No second whole public budget is created.
    let custody = WORKSHOP_CUSTODY_DISPATCH_BYTES + 6 * source;
    let output = 256 * MIB;
    let public = 8 * 1024 * MIB - 512 * MIB - 128 * 1024;
    assert!(custody < (public - output) / 2);
    assert!(custody + MAX_FRONTIER_BYTES + output + 8 * MIB < public);
}

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
    let target = &host["stock_target"];
    assert_eq!(target["plan_census_bytes"], stock_target::CENSUS_BYTES);
    assert_eq!(
        target["read_qualification_bytes"],
        stock_target::CHECK_BYTES
    );
    assert_eq!(
        target["dispatch_custody_max_bytes"],
        WORKSHOP_CUSTODY_DISPATCH_BYTES + 6 * stock_target::CHECK_BYTES
    );
    assert_eq!(target["maximum_fresh_checks_per_prepare"], 6);
    for flag in [
        "fresh_plan_census_required",
        "exact_original_census_required_before_insertion",
        "strict_policy_rebuilt_from_original_archive",
        "consumed_tick_reaches_frontier_recovery_floor",
        "recovered_target_restores_authority_tick_floor",
        "fresh_native_queue_read_after_each_census",
        "fresh_queue_must_match_native_original_observation",
    ] {
        assert_eq!(target[flag], true, "stock target {flag}");
    }
    for flag in [
        "advisory_capture_reused",
        "client_amount_or_baseline",
        "additional_private_sidecar",
        "offline_needs_native_credential",
        "recovery_restores_confirmation",
        "no_work_worker_prerequisite_checked",
        "revalidation_failure_is_durable_source_fence",
    ] {
        assert_eq!(target[flag], false, "stock target {flag}");
    }
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
    for file in [
        "choices.rs",
        "choices_tests.rs",
        "clock.rs",
        "workshop_config.rs",
        "workshop_binding.rs",
        "workshop_prerequisite.rs",
        "workshop_association_tests.rs",
        "workshop_prerequisite_tests.rs",
    ] {
        assert!(paths.contains(&json!(format!(
            "crates/dfmcp-mcp/src/live_production_runtime/{file}"
        ))));
    }
    for file in [
        "crates/dfmcp-adapter/src/workshop_construction/production_origin.rs",
        "crates/dfmcp-adapter/src/workshop_construction/production_origin_tests.rs",
        "tests/stdio/combined_workshop_runtime_peer.py",
        "crates/dfmcp-mcp/src/spatial_entity_query.rs",
        "crates/dfmcp-mcp/src/spatial_selector.rs",
        "crates/dfmcp-mcp/src/related_entity_query.rs",
        "crates/dfmcp-mcp/src/relation_selector.rs",
        "crates/dfmcp-mcp/src/ranked_entity_query.rs",
    ] {
        assert!(paths.contains(&json!(file)), "{file}");
    }
    let workshop = &host["workshop_prerequisite"];
    assert_eq!(workshop["source_protocol"], "workshop/1.20");
    assert_eq!(workshop["donor_mode"], "Offline");
    assert_eq!(workshop["read_credential_env"], workshop_config::TOKEN);
    assert_eq!(
        workshop["association_file"],
        workshop_prerequisite::FILE_NAME
    );
    assert!(
        host["optional_parent_files"]
            .as_array()
            .ok_or_else(|| invalid("missing optional parent files"))?
            .contains(&json!(workshop_prerequisite::FILE_NAME))
    );
    assert_eq!(
        workshop["association_max_bytes"],
        dfmcp_adapter::workshop_construction::production_origin::ASSOCIATION_MAX_BYTES
    );
    assert_eq!(
        workshop["import_work_bytes"],
        workshop_prerequisite::OPEN_BYTES
    );
    assert_eq!(
        workshop["qualification_work_bytes"],
        workshop_prerequisite::CHECK_BYTES
    );
    assert_eq!(
        workshop["local_verification_work_bytes"],
        workshop_prerequisite::LOCAL_BYTES
    );
    assert_eq!(
        workshop["read_custody_work_max"],
        WORKSHOP_CUSTODY_READ_BYTES
    );
    assert_eq!(
        workshop["dispatch_custody_work_max"],
        WORKSHOP_CUSTODY_DISPATCH_BYTES
    );
    assert_eq!(
        workshop["qualify_at"],
        json!([
            "original_plan",
            "each_original_order_prepare",
            "each_original_order_commit_after_dispatch_started"
        ])
    );
    assert_eq!(
        workshop["preplan_checkpoint_fields"],
        json!([
            "origin_witness",
            "tick",
            "generation",
            "next_building_id",
            "fenced"
        ])
    );
    for flag in [
        "origin_binds_original_request_identity",
        "preplan_checkpoint_mutually_exclusive_with_original_recovery",
        "preplan_checkpoint_restored_before_native_contact",
        "consumed_native_floors_retained_after_refusal",
        "source_fence_is_sticky",
        "failed_publication_retains_latest_checkpoint",
        "unconfigured_original_identity_bytes_unchanged",
        "full_read_budget_reserved_before_source_contact",
        "decoded_manifest_consumed_before_refresh_preconditions",
    ] {
        assert_eq!(workshop[flag], true, "workshop {flag}");
    }
    for flag in [
        "preplan_checkpoint_omission_independently_detectable",
        "local_query_and_offline_need_native_credential",
        "offline_restores_current_qualification",
        "donor_writes",
        "construction_permission",
        "canonical_workshop_evidence",
        "production_feasibility_proven",
        "new_native_protocol",
        "new_public_tool",
        "production_admission_added",
    ] {
        assert_eq!(workshop[flag], false, "workshop {flag}");
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
        json!([
            "retained_order_completion",
            "observed_inventory_minimum",
            "observed_inventory_increase"
        ])
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
