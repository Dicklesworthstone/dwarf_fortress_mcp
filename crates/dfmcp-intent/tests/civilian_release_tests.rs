#[path = "support/civilian_protection.rs"]
#[allow(dead_code)]
mod support;

use dfmcp_core::{Capability, EntityId, ErrorCode, IntentId, Result};
use dfmcp_intent::{Action, CivilianReleasePlan};
use dfmcp_world::{
    EntityKind, FactSource, LaboratoryEntityIdentity, Predicate, PredicateEvidence, PredicateTruth,
    Value,
};
use support::*;

fn exact(id: EntityId) -> LaboratoryEntityIdentity {
    LaboratoryEntityIdentity { id, generation: 1 }
}
fn release() -> Result<CivilianReleasePlan> {
    CivilianReleasePlan::compile(exact(BURROW), &[exact(OTHER), exact(SECOND), exact(FIRST)])
}

#[test]
fn release_lowers_complete_incarnations_before_global_order_with_protective_compensation()
-> Result<()> {
    let release = release()?;
    assert_eq!(
        release.civilians(),
        &[exact(FIRST), exact(SECOND), exact(OTHER)]
    );
    assert_eq!(release.burrow(), exact(BURROW));
    let snapshot = world();
    let steps = release.requested_actions_for(snapshot.fortress_id);
    assert_eq!(steps.len(), 2);
    assert_eq!(
        steps[0].preconditions,
        vec![release.safety_predicate().clone()]
    );
    assert_eq!(
        steps[1].preconditions,
        vec![release.safety_predicate().clone()]
    );
    assert!(steps[0].depends_on.is_empty());
    assert_eq!(steps[1].depends_on, vec![0]);
    assert!(
        matches!(&steps[0].action, Action::SetBurrowMembership { units, burrow, assigned: false }
        if *burrow == BURROW && units == &[FIRST, SECOND, OTHER])
    );
    assert!(matches!(
        &steps[0].compensation,
        Some(Action::SetBurrowMembership { assigned: true, .. })
    ));
    assert!(
        matches!(&steps[1].action, Action::SetStandingOrder { value, .. } if value == "INACTIVE")
    );
    assert!(
        matches!(&steps[1].compensation, Some(Action::SetStandingOrder { value, .. }) if value == "ACTIVE")
    );
    assert_eq!(
        steps[1].postconditions,
        vec![release.configuration_goal(snapshot.fortress_id)]
    );
    Ok(())
}

#[test]
fn release_rejects_empty_partial_duplicate_nil_and_generationless_rosters() -> Result<()> {
    for ids in [
        vec![],
        vec![exact(FIRST); 2],
        vec![exact(EntityId::NIL)],
        vec![exact(BURROW)],
        vec![exact(FIRST); 65],
        vec![LaboratoryEntityIdentity {
            id: FIRST,
            generation: 0,
        }],
    ] {
        assert!(CivilianReleasePlan::compile(exact(BURROW), &ids).is_err());
    }
    let snapshot = complete(world())?;
    let partial = CivilianReleasePlan::compile(exact(BURROW), &[exact(FIRST), exact(SECOND)])?;
    assert!(
        matches!(partial.prepare(IntentId::new(301), "release subset".into(),
        &PredicateEvidence::laboratory(&snapshot)?, &context(&snapshot)),
        Err(error) if error.code == ErrorCode::PreconditionsFailed)
    );
    Ok(())
}

#[test]
fn running_world_release_still_requires_full_current_qualified_safety_and_authority() -> Result<()>
{
    let mut snapshot = complete(world())?;
    snapshot.paused = false;
    snapshot.refresh_hash();
    let release = release()?;
    let plan = release.prepare(
        IntentId::new(302),
        "reviewed release".into(),
        &PredicateEvidence::laboratory(&snapshot)?,
        &context(&snapshot),
    )?;
    assert!(plan.requires_checkpoint);
    assert_eq!(
        plan.steps[1].postconditions,
        vec![plan.terminal_condition.clone()]
    );
    assert_eq!(
        plan.terminal_condition,
        release.configuration_goal(snapshot.fortress_id)
    );
    assert!(
        !plan
            .steps
            .iter()
            .flat_map(|step| &step.preconditions)
            .any(|predicate| matches!(predicate, Predicate::Paused(_)))
    );
    assert!(
        release
            .prepare(
                IntentId::new(303),
                "untrusted release".into(),
                &PredicateEvidence::untrusted(&snapshot)?,
                &context(&snapshot)
            )
            .is_err()
    );
    let mut asserted = snapshot.clone();
    let mut alive = fact(Value::Bool(true));
    alive.source = FactSource::AgentAssertion("civilian assertion".into());
    set(&mut asserted, FIRST, "alive", alive)?;
    assert!(
        release
            .prepare(
                IntentId::new(304),
                "asserted release".into(),
                &PredicateEvidence::laboratory(&asserted)?,
                &context(&asserted)
            )
            .is_err()
    );
    for capability in [Capability::Observe, Capability::Plan] {
        let mut denied = context(&snapshot);
        denied.grants.retain(|grant| grant.capability != capability);
        assert!(
            matches!(release.prepare(IntentId::new(305), "denied release".into(),
            &PredicateEvidence::laboratory(&snapshot)?, &denied),
            Err(error) if error.code == ErrorCode::CapabilityDenied)
        );
    }
    let mut bounded = context(&snapshot);
    bounded.budget.max_entities = (snapshot.graph.entities.len() - 1) as u32;
    assert!(
        matches!(release.prepare(IntentId::new(306), "bounded release".into(),
        &PredicateEvidence::laboratory(&snapshot)?, &bounded),
        Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn final_configuration_fails_when_threat_roster_or_incarnation_changes() -> Result<()> {
    let release = release()?;
    let mut snapshot = complete(world())?;
    for id in [FIRST, SECOND, OTHER] {
        set(&mut snapshot, id, &field_name(), fact(Value::Bool(false)))?;
    }
    let settings = dfmcp_intent::effects::fortress_settings_entity_id(snapshot.fortress_id);
    set(
        &mut snapshot,
        settings,
        "standing_order.CIVILIAN_BURROW_RESTRICTION",
        fact(Value::Text("INACTIVE".into())),
    )?;
    let goal = release.configuration_goal(snapshot.fortress_id);
    assert_eq!(
        PredicateEvidence::laboratory(&snapshot)?.evaluate(&goal)?,
        PredicateTruth::True
    );
    for mutation in 0..4 {
        let mut changed = snapshot.clone();
        match mutation {
            0 => {
                set(&mut changed, FIRST, "alive", fact(Value::Bool(false)))?;
            }
            1 => {
                changed
                    .graph
                    .entities
                    .get_mut(&FIRST)
                    .ok_or_else(|| {
                        dfmcp_core::DfmcpError::new(ErrorCode::InvalidRequest, "fixture missing")
                    })?
                    .generation += 1;
                changed.refresh_hash();
            }
            2 => {
                let mut hostile = record(EntityId::new(30), EntityKind::Creature);
                hostile
                    .fields
                    .insert("hostile".into(), fact(Value::Bool(true)));
                hostile.fields.insert(
                    "threat_status".into(),
                    fact(Value::Text("approaching".into())),
                );
                changed.graph.entities.insert(hostile.id, hostile);
                changed.refresh_hash();
            }
            _ => {
                let mut newcomer = record(EntityId::new(14), EntityKind::Unit);
                newcomer
                    .fields
                    .insert("alive".into(), fact(Value::Bool(true)));
                changed.graph.entities.insert(newcomer.id, newcomer);
                changed.refresh_hash();
            }
        }
        assert_eq!(
            PredicateEvidence::laboratory(&changed)?.evaluate(&goal)?,
            PredicateTruth::False
        );
    }
    Ok(())
}

#[test]
fn protective_compensation_keeps_exact_incarnations_without_requiring_clear_threats() -> Result<()>
{
    use dfmcp_intent::civilian_release::validate_release_compensation_identities;
    let snapshot = complete(world())?;
    let plan = release()?.prepare(
        IntentId::new(307),
        "reviewed release".into(),
        &PredicateEvidence::laboratory(&snapshot)?,
        &context(&snapshot),
    )?;
    let mut dangerous = snapshot.clone();
    let mut hostile = record(EntityId::new(30), EntityKind::Creature);
    hostile
        .fields
        .insert("hostile".into(), fact(Value::Bool(true)));
    hostile.fields.insert(
        "threat_status".into(),
        fact(Value::Text("attacking".into())),
    );
    dangerous.graph.entities.insert(hostile.id, hostile);
    dangerous.refresh_hash();
    for step in &plan.steps {
        validate_release_compensation_identities(step, &dangerous)?;
    }
    for id in [FIRST, BURROW] {
        let mut changed = dangerous.clone();
        changed
            .graph
            .entities
            .get_mut(&id)
            .ok_or_else(|| {
                dfmcp_core::DfmcpError::new(ErrorCode::InvalidRequest, "fixture missing")
            })?
            .generation += 1;
        changed.refresh_hash();
        for step in &plan.steps {
            assert!(
                matches!(validate_release_compensation_identities(step, &changed),
                Err(error) if error.code == ErrorCode::CancellationIncomplete)
            );
        }
        // The existing named protection contract has no new release predicate.
        let mut legacy = plan.steps[0].clone();
        legacy.preconditions = vec![Predicate::Paused(true)];
        validate_release_compensation_identities(&legacy, &changed)?;
    }
    Ok(())
}
