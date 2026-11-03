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
    for field in [
        "recovery_restores_commit_permit",
        "wait_dispatches_or_advances_time",
        "checkpoint_or_restore_supported",
        "current_state_proven",
        "original_goal_proven",
        "new_bridge_generation",
        "production_admission_added",
    ] {
        assert_eq!(host[field], false, "{field}");
    }
    Ok(())
}
