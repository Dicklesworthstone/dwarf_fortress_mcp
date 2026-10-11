#![forbid(unsafe_code)]

#[path = "support/labor_execution.rs"]
mod support;

use dfmcp_core::{Capability, EntityId, ErrorCode, IntentId, Result};
use dfmcp_intent::{Action, LaborAllocationLimits, LaborAllocator};
use dfmcp_world::{FactPresence, FactSource, PredicateEvidence, Value};
use support::{context, demands, profiles, set, world};

#[test]
fn staffing_lowering_seals_real_predicates_and_preserves_order() -> Result<()> {
    let mut snapshot = world(&profiles(), &demands());
    let compiled = LaborAllocator.compile_execution(&profiles(), &demands(), LaborAllocationLimits::default())?;
    let evidence = PredicateEvidence::laboratory(&snapshot)?;
    let plan = compiled.prepare(IntentId::new(41), "staff mining and brewing".to_owned(), &evidence, &context(&snapshot))?;
    plan.validate_structure()?;
    assert_eq!(plan.steps.len(), 3);
    assert_eq!(plan.terminal_condition, *compiled.goal());
    assert!(!evidence.establishes(&plan.terminal_condition)?);
    assert!(plan.steps[0].depends_on.is_empty());
    for (index, step) in plan.steps.iter().enumerate() {
        if index > 0 {
            assert_eq!(step.depends_on, vec![plan.steps[index - 1].id]);
        }
        let current = PredicateEvidence::laboratory(&snapshot)?;
        for predicate in &step.preconditions { assert!(current.establishes(predicate)?); }
        let Action::SetLabor { units, labor, enabled } = &step.action else { return unreachable_for_test(); };
        assert_eq!(step.compensation, Some(Action::SetLabor { units: units.clone(), labor: labor.clone(), enabled: !*enabled }));
        for id in units { set(&mut snapshot, *id, &format!("labor.{labor}"), *enabled)?; }
        let after = PredicateEvidence::laboratory(&snapshot)?;
        for predicate in &step.postconditions { assert!(after.establishes(predicate)?); }
    }
    assert!(PredicateEvidence::laboratory(&snapshot)?.establishes(compiled.goal())?);
    Ok(())
}
fn unreachable_for_test() -> Result<()> {
    Err(dfmcp_core::DfmcpError::new(ErrorCode::InternalInvariantViolation, "expected labor action"))
}

#[test]
fn reordered_inputs_have_identical_sealed_plans() -> Result<()> {
    let rows = profiles();
    let roles = demands();
    let snapshot = world(&rows, &roles);
    let mut reversed_rows = rows.clone(); reversed_rows.reverse();
    let mut reversed_roles = roles.clone(); reversed_roles.reverse();
    let a = LaborAllocator.compile_execution(&rows, &roles, LaborAllocationLimits::default())?;
    let b = LaborAllocator.compile_execution(&reversed_rows, &reversed_roles, LaborAllocationLimits::default())?;
    assert_eq!(a.requested_actions(), b.requested_actions());
    let evidence = PredicateEvidence::laboratory(&snapshot)?;
    assert_eq!(
        a.prepare(IntentId::new(41), "staff".to_owned(), &evidence, &context(&snapshot))?,
        b.prepare(IntentId::new(41), "staff".to_owned(), &evidence, &context(&snapshot))?,
    );
    Ok(())
}

#[test]
fn missing_unknown_untrusted_future_or_contradicted_initial_bits_refuse() -> Result<()> {
    let compiled = LaborAllocator.compile_execution(&profiles(), &demands(), LaborAllocationLimits::default())?;
    for case in 0..5 {
        let mut snapshot = world(&profiles(), &demands());
        let row = snapshot.graph.entities.get_mut(&EntityId::new(2)).ok_or_else(|| dfmcp_core::DfmcpError::new(ErrorCode::InternalInvariantViolation, "fixture"))?;
        if case == 0 { row.fields.remove("labor.MINING"); } else {
            let value = row.fields.get_mut("labor.MINING").ok_or_else(|| dfmcp_core::DfmcpError::new(ErrorCode::InternalInvariantViolation, "fixture field"))?;
            match case {
                1 => value.presence = Some(FactPresence::Unknown("not observed".to_owned())),
                2 => value.source = FactSource::AgentAssertion("claim".to_owned()),
                3 => value.observed_at.0 += 1,
                _ => { value.value = Value::Bool(true); value.presence = None; }
            }
        }
        snapshot.refresh_hash();
        assert!(matches!(compiled.prepare(IntentId::new(1), "staff".to_owned(), &PredicateEvidence::laboratory(&snapshot)?, &context(&snapshot)), Err(e) if e.code == ErrorCode::PreconditionsFailed));
    }
    let snapshot = world(&profiles(), &demands());
    assert!(matches!(compiled.prepare(IntentId::new(1), "staff".to_owned(), &PredicateEvidence::untrusted(&snapshot)?, &context(&snapshot)), Err(e) if e.code == ErrorCode::PreconditionsFailed));
    Ok(())
}

#[test]
fn unchanged_unselected_workers_are_part_of_the_final_proof() -> Result<()> {
    let mut rows = profiles();
    rows[0].assigned_labors.clear();
    let mut roles = demands();
    roles.retain(|role| role.labor == "MINING");
    let mut snapshot = world(&rows, &roles);
    let compiled = LaborAllocator.compile_execution(&rows, &roles, LaborAllocationLimits::default())?;
    assert_eq!(compiled.requested_actions().len(), 1);
    set(&mut snapshot, EntityId::new(1), "labor.MINING", true)?;
    assert!(PredicateEvidence::laboratory(&snapshot)?.establishes(compiled.goal())?);
    set(&mut snapshot, EntityId::new(2), "labor.MINING", true)?;
    let evidence = PredicateEvidence::laboratory(&snapshot)?;
    assert!(!evidence.establishes(compiled.goal())?);
    assert!(!evidence.establishes(&compiled.requested_actions()[0].postconditions[0])?);
    Ok(())
}

#[test]
fn later_steps_refuse_an_external_change_instead_of_noop_compensation() -> Result<()> {
    let compiled = LaborAllocator.compile_execution(&profiles(), &demands(), LaborAllocationLimits::default())?;
    let mut snapshot = world(&profiles(), &demands());
    let step = &compiled.requested_actions()[1];
    let Action::SetLabor { units, labor, enabled } = &step.action else { return unreachable_for_test(); };
    for id in units { set(&mut snapshot, *id, &format!("labor.{labor}"), *enabled)?; }
    assert!(!PredicateEvidence::laboratory(&snapshot)?.establishes(&step.preconditions[0])?);
    Ok(())
}

#[test]
fn shortages_noops_dead_citizens_and_inadequate_authority_do_not_create_work() -> Result<()> {
    let mut roles = demands(); roles[0].workers = 2;
    assert!(matches!(LaborAllocator.compile_execution(&profiles(), &roles, LaborAllocationLimits::default()), Err(e) if e.code == ErrorCode::PreconditionsFailed));
    let mut rows = profiles(); rows[0].assigned_labors = ["MINING".to_owned()].into_iter().collect(); rows[1].assigned_labors.insert("BREW".to_owned());
    let noop = LaborAllocator.compile_execution(&rows, &demands(), LaborAllocationLimits::default())?;
    assert!(noop.requested_actions().is_empty());
    assert!(noop.into_requested_actions().is_err());
    let compiled = LaborAllocator.compile_execution(&profiles(), &demands(), LaborAllocationLimits::default())?;
    let mut snapshot = world(&profiles(), &demands());
    let mut ctx = context(&snapshot); ctx.grants.retain(|g| g.capability != Capability::Observe);
    assert!(matches!(compiled.prepare(IntentId::new(1), "staff".to_owned(), &PredicateEvidence::laboratory(&snapshot)?, &ctx), Err(e) if e.code == ErrorCode::CapabilityDenied));
    set(&mut snapshot, EntityId::new(2), "alive", false)?;
    assert!(matches!(compiled.prepare(IntentId::new(1), "staff".to_owned(), &PredicateEvidence::laboratory(&snapshot)?, &context(&snapshot)), Err(e) if e.code == ErrorCode::PreconditionsFailed));
    Ok(())
}

#[test]
fn lowering_and_sealing_share_explicit_limits() -> Result<()> {
    let allocation = LaborAllocator.allocate_demands(&profiles(), &demands(), LaborAllocationLimits::default())?;
    let limits = LaborAllocationLimits { max_work: allocation.work_used(), ..LaborAllocationLimits::default() };
    assert!(matches!(LaborAllocator.compile_execution(&profiles(), &demands(), limits), Err(e) if e.code == ErrorCode::BudgetExceeded));
    let compiled = LaborAllocator.compile_execution(&profiles(), &demands(), LaborAllocationLimits::default())?;
    let snapshot = world(&profiles(), &demands());
    for dimension in 0..3 {
        let mut ctx = context(&snapshot);
        match dimension { 0 => ctx.budget.max_bytes = 1, 1 => ctx.budget.max_entities = 1, _ => ctx.budget.max_actions = 1 }
        assert!(matches!(compiled.prepare(IntentId::new(1), "staff".to_owned(), &PredicateEvidence::laboratory(&snapshot)?, &ctx), Err(e) if e.code == ErrorCode::BudgetExceeded));
    }
    Ok(())
}
