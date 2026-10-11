#[path = "support/civilian_protection.rs"]
mod support;

use dfmcp_core::{Capability, EntityId, ErrorCode, GameTick, IntentId, Result};
use dfmcp_intent::{Action, CivilianProtectionPlan};
use dfmcp_world::{FactSource, PredicateEvidence, PredicateTruth, Value};
use support::*;

#[test]
fn canonical_lowering_orders_restriction_before_membership_and_never_inverts_protection() -> Result<()> {
    let plan = compiled()?;
    assert_eq!(plan.civilians(), &[FIRST, SECOND]);
    assert_eq!(plan.burrow(), BURROW);
    assert_eq!(plan.requested_actions(), CivilianProtectionPlan::compile(BURROW, &[FIRST, SECOND])?.requested_actions());
    let steps = plan.requested_actions();
    assert_eq!(steps.len(), 2);
    assert!(steps[0].depends_on.is_empty());
    assert_eq!(steps[1].depends_on, vec![0]);
    assert!(matches!(&steps[1].action, Action::SetBurrowMembership { assigned: true, .. }));
    for step in steps { assert_eq!(step.compensation.as_ref(), Some(&step.action)); }
    Ok(())
}

#[test]
fn fixed_bounds_reject_empty_duplicate_nil_and_aliased_rosters() {
    for ids in [vec![], vec![FIRST, FIRST], vec![EntityId::NIL], vec![BURROW], vec![FIRST; 65]] {
        assert!(CivilianProtectionPlan::compile(BURROW, &ids).is_err());
    }
    assert!(CivilianProtectionPlan::compile(EntityId::NIL, &[FIRST]).is_err());
}

#[test]
fn eligible_observations_seal_a_complete_goal_without_assuming_missing_memberships_false() -> Result<()> {
    let mut snapshot = world();
    snapshot.graph.entities.get_mut(&SECOND).ok_or_else(|| dfmcp_core::DfmcpError::new(ErrorCode::InvalidRequest, "fixture missing"))?.fields.remove(&field_name());
    snapshot.refresh_hash();
    let evidence = PredicateEvidence::laboratory(&snapshot)?;
    let compiled = compiled()?;
    let progress = compiled.evaluate(&evidence, &context(&snapshot))?;
    assert_eq!((progress.assigned, progress.unassigned, progress.unknown_memberships), (1, 0, 1));
    assert_eq!(progress.configuration, PredicateTruth::Unknown);
    let plan = compiled.prepare(IntentId::new(101), "protect civilians".to_owned(), &evidence, &context(&snapshot))?;
    assert!(plan.digest_is_valid());
    assert_eq!(plan.steps[1].postconditions, vec![plan.terminal_condition.clone()]);
    assert!(plan.requires_checkpoint);
    Ok(())
}

#[test]
fn liveness_source_and_presence_are_required_not_just_a_matching_value() -> Result<()> {
    for mode in 0..4 {
        let mut snapshot = world();
        let mut alive = fact(Value::Bool(true));
        match mode {
            0 => alive.source = FactSource::AgentAssertion("not authority".to_owned()),
            1 => alive.source = FactSource::Replay,
            2 => alive.observed_at = GameTick(2),
            _ => alive.presence = Some(dfmcp_world::FactPresence::Unknown("not observed".to_owned())),
        }
        set(&mut snapshot, FIRST, "alive", alive)?;
        let result = compiled()?.prepare(IntentId::new(102), "protect".to_owned(), &PredicateEvidence::laboratory(&snapshot)?, &context(&snapshot));
        assert!(matches!(result, Err(e) if e.code == ErrorCode::PreconditionsFailed));
    }
    let snapshot = world();
    assert!(compiled()?.prepare(IntentId::new(103), "protect".to_owned(), &PredicateEvidence::untrusted(&snapshot)?, &context(&snapshot)).is_err());
    Ok(())
}

#[test]
fn complete_goal_is_current_configuration_not_just_membership_or_historical_success() -> Result<()> {
    let plan = compiled()?;
    let snapshot = complete(world())?;
    assert_eq!(plan.evaluate(&PredicateEvidence::laboratory(&snapshot)?, &context(&snapshot))?.configuration, PredicateTruth::True);
    let mut changed = snapshot.clone();
    set(&mut changed, FIRST, "alive", fact(Value::Bool(false)))?;
    let progress = plan.evaluate(&PredicateEvidence::laboratory(&changed)?, &context(&changed))?;
    assert_eq!(progress.assigned, 2);
    assert_eq!(progress.configuration, PredicateTruth::False);
    let mut changed = snapshot.clone();
    let settings = dfmcp_intent::effects::fortress_settings_entity_id(changed.fortress_id);
    set(&mut changed, settings, "standing_order.CIVILIAN_BURROW_RESTRICTION", fact(Value::Text("INACTIVE".to_owned())))?;
    assert_eq!(plan.evaluate(&PredicateEvidence::laboratory(&changed)?, &context(&changed))?.configuration, PredicateTruth::False);
    assert_eq!(plan.evaluate(&PredicateEvidence::untrusted(&snapshot)?, &context(&snapshot))?.configuration, PredicateTruth::Unknown);
    assert!(plan.prepare(IntentId::new(104), "already protected".to_owned(), &PredicateEvidence::laboratory(&snapshot)?, &context(&snapshot)).is_err());
    Ok(())
}

#[test]
fn exact_anchor_pause_authority_and_call_budgets_gate_preparation() -> Result<()> {
    let compiled = compiled()?;
    let snapshot = world();
    let evidence = PredicateEvidence::laboratory(&snapshot)?;
    for mode in 0..6 {
        let mut ctx = context(&snapshot);
        match mode {
            0 => ctx.anchor.tick = GameTick(2),
            1 => ctx.grants.retain(|g| g.capability != Capability::Observe),
            2 => ctx.budget.max_entities = 3,
            3 => ctx.budget.max_actions = 1,
            4 => ctx.budget.max_bytes = compiled.encoded_bytes() as u64 - 1,
            _ => ctx.cancellation_requested = true,
        }
        assert!(compiled.prepare(IntentId::new(105), "protect".to_owned(), &evidence, &ctx).is_err());
    }
    let mut running = snapshot;
    running.paused = false;
    running.refresh_hash();
    assert!(matches!(compiled.prepare(IntentId::new(106), "protect".to_owned(), &PredicateEvidence::laboratory(&running)?, &context(&running)), Err(e) if e.code == ErrorCode::PreconditionsFailed));
    Ok(())
}

#[test]
fn identical_requests_seal_identically_and_status_never_mutates_state() -> Result<()> {
    let snapshot = world();
    let before = snapshot.anchor();
    let evidence = PredicateEvidence::laboratory(&snapshot)?;
    let a = compiled()?;
    let b = CivilianProtectionPlan::compile(BURROW, &[FIRST, SECOND])?;
    assert_eq!(a.prepare(IntentId::new(107), "protect".to_owned(), &evidence, &context(&snapshot))?, b.prepare(IntentId::new(107), "protect".to_owned(), &evidence, &context(&snapshot))?);
    assert_eq!(a.evaluate(&evidence, &context(&snapshot))?, b.evaluate(&evidence, &context(&snapshot))?);
    assert_eq!(before, snapshot.anchor());
    Ok(())
}

#[test]
fn dependency_proof_does_not_replace_current_pause_and_citizen_guards() -> Result<()> {
    use dfmcp_core::CommitState;
    use dfmcp_intent::execution::{DeferredStepDecision, deferred_step_decision_with_evidence};
    let snapshot = world();
    let plan = compiled()?.prepare(IntentId::new(108), "protect".to_owned(), &PredicateEvidence::laboratory(&snapshot)?, &context(&snapshot))?;
    for mode in 0..2 {
        let mut changed = snapshot.clone();
        if mode == 0 {
            changed.paused = false;
            changed.refresh_hash();
        } else {
            set(&mut changed, SECOND, "alive", fact(Value::Bool(false)))?;
        }
        assert!(matches!(deferred_step_decision_with_evidence(&plan.steps[1], &PredicateEvidence::laboratory(&changed)?, |_| Some(CommitState::Verified))?, DeferredStepDecision::Failed(_)));
    }
    let maximum: Vec<_> = (100..164).map(EntityId::new).collect();
    assert!(CivilianProtectionPlan::compile(BURROW, &maximum)?.encoded_bytes() <= dfmcp_intent::civilian_protection::MAX_PROTECTION_BYTES);
    Ok(())
}
