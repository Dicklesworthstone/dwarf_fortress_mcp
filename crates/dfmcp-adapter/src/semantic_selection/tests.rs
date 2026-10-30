use super::*;
use dfmcp_core::{
    Digest32, EntityId, FortressId, GameTick, IntentId, ObservationCursor, StateAnchor,
};
use dfmcp_intent::{Action, PlanStep, derive_step_idempotency_key};
use dfmcp_world::{CompareOp, Predicate, Value};
use std::collections::BTreeSet;

fn plan(count: usize) -> PreparedPlan {
    let anchor = StateAnchor {
        fortress_id: FortressId::new(7),
        cursor: ObservationCursor::ORIGIN,
        tick: GameTick(120),
        state_hash: Digest32::of_bytes(b"original selected plan source"),
    };
    let intent = IntentId::new(5);
    let steps = (0..count)
        .map(|index| {
            let action = Action::SetLabor {
                units: vec![EntityId::new(43)],
                labor: format!("LABOR{index}"),
                enabled: true,
            };
            let id = StepId::new(index as u32);
            PlanStep {
                id,
                idempotency_key: derive_step_idempotency_key(intent, anchor, id, &action),
                risk: action.risk(),
                required_capability: action.capability(),
                action,
                preconditions: vec![Predicate::Paused(true)],
                postconditions: vec![Predicate::Paused(true)],
                compensation: None,
                obligation: None,
                depends_on: if index == 0 {
                    Vec::new()
                } else {
                    vec![StepId::new(index as u32 - 1)]
                },
            }
        })
        .collect::<Vec<_>>();
    let max_risk = steps
        .iter()
        .map(|step| step.risk)
        .max()
        .map_or(dfmcp_core::RiskTier::ReadOnly, |risk| risk);
    PreparedPlan::builder(
        intent,
        anchor,
        "complete original parent",
        Predicate::Paused(true),
    )
    .max_risk(max_risk)
    .required_capabilities(BTreeSet::from([dfmcp_core::Capability::ConfigureLabor]))
    .expires_at_tick(GameTick(220))
    .steps(steps)
    .build()
}
fn reseal(plan: &mut PreparedPlan) {
    plan.digest = plan.compute_digest();
    plan.id = plan.expected_id();
}

#[test]
fn selected_identity_retains_every_original_field_and_does_not_rekey() -> Result<()> {
    for count in [1, 2, 16] {
        let original = plan(count);
        original.validate_structure()?;
        let last = StepId::new(count as u32 - 1);
        let selected = SelectedSemanticStep::new(original.clone(), last)?;
        let (retained, id) = selected.into_parts();
        assert_eq!(id, last);
        assert_eq!(retained, original);
        assert_eq!(retained.digest, original.digest);
        assert_eq!(
            retained.steps[count - 1].idempotency_key,
            original.steps[count - 1].idempotency_key
        );
    }
    Ok(())
}

#[test]
fn whole_parent_mutation_missing_selection_and_capacity_refuse() -> Result<()> {
    let original = plan(2);
    let mut changed = original.clone();
    changed.steps[0].postconditions = vec![Predicate::Paused(false)];
    assert!(SelectedSemanticStep::new(changed, StepId::new(1)).is_err());
    assert!(SelectedSemanticStep::new(original.clone(), StepId::new(2)).is_err());
    assert!(SelectedSemanticStep::new(plan(17), StepId::ZERO).is_err());
    let mut checkpoint = original.clone();
    checkpoint.requires_checkpoint = true;
    reseal(&mut checkpoint);
    assert!(SelectedSemanticStep::new(checkpoint, StepId::ZERO).is_err());
    let mut huge = original;
    huge.steps[0].postconditions = vec![Predicate::FieldCompare {
        entity_id: EntityId::new(43),
        field: "unselected_original_fact".to_owned(),
        op: CompareOp::Eq,
        value: Value::Text("x".repeat(64 * 1024)),
    }];
    reseal(&mut huge);
    assert!(SelectedSemanticStep::new(huge, StepId::new(1)).is_err());
    Ok(())
}

#[test]
fn aggregate_original_predicate_bound_includes_unselected_steps() -> Result<()> {
    let mut original = plan(16);
    for (index, step) in original.steps.iter_mut().enumerate() {
        step.postconditions = (0..32)
            .map(|column| Predicate::FieldCompare {
                entity_id: EntityId::new(43),
                field: format!("bounded_{index:02}_{column:02}"),
                op: CompareOp::Eq,
                value: Value::Bool(true),
            })
            .collect();
    }
    reseal(&mut original);
    original.validate_structure()?;
    let err = SelectedSemanticStep::new(original, StepId::new(15))
        .err()
        .ok_or_else(|| refused("oversized predicate aggregate unexpectedly selected"))?;
    assert_eq!(err.code, ErrorCode::BudgetExceeded);
    Ok(())
}
