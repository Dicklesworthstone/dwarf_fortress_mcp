use super::*;

fn context() -> Result<OperationContext> {
    let fortress = FortressId::new(7);
    Ok(OperationContext { session_id: SessionId::new(1), request_id: RequestId::new(2),
        anchor: StateAnchor { fortress_id:fortress, cursor:ObservationCursor::ORIGIN,
            tick:GameTick(0),state_hash:dfmcp_core::Digest32::ZERO },
        budget:WorkBudget { max_bytes:MAX_BYTES,max_output_tokens:65_536,..WorkBudget::default() },
        grants:operator_grants(Mode::Control, fortress, true)?, cancellation_requested:false })
}

#[test]
fn development_environment_is_closed_and_production_is_separately_opted_in() -> Result<()> {
    let allowed: Vec<String> = ALLOWED_ENVIRONMENT.iter().map(|s|(*s).to_owned()).collect();
    environment_contract(Some("1"), None, &allowed, false)?;
    environment_contract(Some("1"), Some("1"), &allowed, false)?;
    for flag in [None, Some("0"), Some("true"), Some("")] {
        assert!(environment_contract(flag, None, &allowed, false).is_err());
    }
    assert!(environment_contract(Some("1"), Some("0"), &allowed, false).is_err());
    assert!(environment_contract(Some("1"), None, &allowed, true).is_err());
    for forbidden in ["DFMCP_ADMITTED_BRIDGE_PROTOCOL", "DFMCP_CONTROL_TOKEN", "DFMCP_JOB_CONTROL_JOURNAL_REPAIR"] {
        let mut keys = allowed.clone(); keys.push(forbidden.to_owned());
        assert!(environment_contract(Some("1"), None, &keys, false).is_err());
    }
    Ok(())
}

#[test]
fn query_modes_never_acquire_production_even_with_operator_enablement() -> Result<()> {
    let fortress = FortressId::new(7);
    for mode in [Mode::Offline, Mode::Reconcile] {
        for enabled in [false, true] {
            let grants = operator_grants(mode, fortress, enabled)?;
            assert_eq!(grants.len(), 1); assert_eq!(grants[0].capability, Capability::Query);
            assert_eq!(grants[0].scope.fortress_id, Some(fortress));
            assert_eq!(grants[0].max_risk, RiskTier::ReadOnly);
        }
    }
    assert!(operator_grants(Mode::Control, fortress, false).is_err());
    assert_eq!(operator_grants(Mode::Control, fortress, true)?.len(), 2);
    assert!(Mode::parse("arbitrary-profile").is_err()); Ok(())
}

#[test]
fn response_reservation_precedes_work_and_cannot_expand_a_session_budget() -> Result<()> {
    let original = context()?;
    let (display, work) = narrowed(original.clone(), &Limits::default(), 8)?;
    assert_eq!(work.budget.max_bytes + BASE_RESERVE + 8 * RECORD_RESERVE, display.budget.max_bytes);
    let (display, _) = narrowed(original.clone(), &Limits { bytes:Some(u64::MAX),
        wall:Some(u64::MAX),tokens:Some(u32::MAX) }, 1)?;
    assert_eq!(display.budget, original.budget);
    assert!(narrowed(original.clone(), &Limits { tokens:Some(1), ..Limits::default() }, 1).is_err());
    assert!(narrowed(original.clone(), &Limits { bytes:Some(BASE_RESERVE), ..Limits::default() }, 1).is_err());
    assert!(narrowed(original.clone(), &Limits { wall:Some(0), ..Limits::default() }, 1).is_err());
    assert!(narrowed(original, &Limits::default(), MAX_PAGE + 1).is_err()); Ok(())
}

#[test]
fn deadlines_do_not_restart_between_bootstrap_stages() -> Result<()> {
    let original = context()?;
    let started = Instant::now().checked_sub(Duration::from_millis(10))
        .ok_or_else(||error(ErrorCode::InternalInvariantViolation,"test clock underflow"))?;
    let current = remaining_context(original.clone(), started, original.budget.max_wall_millis)?;
    let next = remaining_context(current.clone(), started, original.budget.max_wall_millis)?;
    assert!(next.budget.max_wall_millis <= current.budget.max_wall_millis);
    assert!(current.budget.max_wall_millis < original.budget.max_wall_millis);
    assert!(remaining_context(original, started, 1).is_err()); Ok(())
}

#[test]
fn job_session_ids_are_canonical_process_scoped_and_not_pause_handles() -> Result<()> {
    let first = next_id()?; let second = next_id()?;
    assert_ne!(first, second); assert!(first.is_process_scoped_live());
    assert_eq!(session_id(&first.to_string())?, first);
    let pause = SessionId::new((1u128 << 127) | (1u128 << 57) | 1);
    assert!(session_id(&pause.to_string()).is_err());
    assert!(session_id(&first.to_string().to_uppercase()).is_err());
    assert!(session_id(&SessionId::new(1).to_string()).is_err());
    assert!(session_id(&"0".repeat(33)).is_err()); Ok(())
}

#[test]
fn frozen_tools_route_to_the_session_loop_without_a_direct_native_setter() -> Result<()> {
    // Structural wiring check, not an executed MCP transport test.
    let source = include_str!("live_job_control_server.rs");
    for tool in ["OpenSession", "Observe", "Query", "Plan", "Commit", "Wait", "Cancel",
        "Checkpoint", "Restore", "Explain", "Doctor"] {
        assert_eq!(source.matches(&format!(".tool(Fortress{tool})")).count(), 1);
    }
    assert_eq!(source.matches("#[tool(").count(), 11);
    assert!(!source.contains(".commit_prepared("));
    assert!(source.contains("session.control.commit("));
    assert!(source.contains("drop(guard.take())"));
    Ok(())
}

#[test]
fn job_io_preserves_inherited_runtime_restrictions_and_cancellation()
    -> std::result::Result<(), Box<dyn std::error::Error>>
{
    use fastmcp_rust::asupersync::{Cx, cx::cap};
    assert!(Cx::current().is_none());
    assert!(runtime_io().is_err());
    crate::run_with_runtime_cx(|cx| async move {
        runtime_io()?;
        {
            let _restricted = cx.restrict::<cap::None>().set_current_restricted();
            assert!(runtime_io().is_err());
        }
        runtime_io()?;
        cx.cancel_with(fastmcp_rust::asupersync::types::CancelKind::User, Some("test job cancellation"));
        assert!(matches!(runtime_io(), Err(e) if e.code == ErrorCode::CancellationRequested));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    Ok(())
}
