//! Compiler generations survive source replay and participate in actual seals.

use super::*;
use crate::lab_world::scenario_snapshot;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, FortressId, OperationContext, RequestId,
    SessionId, WorkBudget,
};
use dfmcp_intent::{Action, PreparedPlan, effects};
use dfmcp_lab::MemoryAdapter;

fn world() -> Result<WorldSnapshot> {
    scenario_snapshot("starter_fortress", FortressId::new(745), false)
}

fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget::default(),
        cancellation_requested: false,
        grants: vec![CapabilityGrant {
            capability: Capability::Plan,
            max_risk: RiskTier::ReadOnly,
            scope: CapabilityScope {
                fortress_id: Some(snapshot.fortress_id),
                ..CapabilityScope::default()
            },
            expires_at_tick: None,
            remaining_uses: None,
        }],
    }
}

fn prepared(snapshot: &WorldSnapshot, intent: &Intent) -> Result<PreparedPlan> {
    StaticPlanner::default().prepare_laboratory(snapshot, intent, &context(snapshot))
}

const ORIGINAL: &str = r#"{"quotas":[{"item":"DRINK","minimum":60}],"template":"production"}"#;

#[test]
fn historical_source_keeps_its_exact_original_intent_and_summary() -> Result<()> {
    let snapshot = world()?;
    let request = ProductionRequest::parse(ORIGINAL)?;
    assert_eq!(request.canonical_json(), ORIGINAL);
    assert_eq!(request.planner(), None);
    let lowered = request.compile(&snapshot)?;
    let mut expected = Intent {
        id: IntentId::new(7),
        anchor: snapshot.anchor(),
        summary: "original reserve".to_owned(),
        terminal_condition: lowered.terminal.clone(),
        constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions: parse_steps(&lowered.actions)?,
    };
    lowered.apply_capacity_horizon(&mut expected)?;
    let (actual, analysis) = compile(&request, IntentId::new(7), &snapshot, "original reserve")?;
    assert_eq!(actual, expected);
    assert_eq!(
        prepared(&snapshot, &actual)?,
        prepared(&snapshot, &expected)?
    );
    assert_eq!(analysis, lowered.analysis);
    assert_eq!(actual.summary, "original reserve");
    Ok(())
}

#[test]
fn new_source_generation_has_a_distinct_seal_even_when_actions_are_identical() -> Result<()> {
    let snapshot = world()?;
    let legacy = ProductionRequest::parse(ORIGINAL)?;
    let current = ProductionRequest::parse_current(ORIGINAL)?;
    let (old, _) = compile(&legacy, IntentId::new(7), &snapshot, "reserve")?;
    let (new, analysis) = compile(&current, IntentId::new(7), &snapshot, "reserve")?;
    let old_plan = prepared(&snapshot, &old)?;
    let new_plan = prepared(&snapshot, &new)?;
    assert_eq!(old_plan.steps.len(), new_plan.steps.len());
    assert!(
        old_plan
            .steps
            .iter()
            .zip(&new_plan.steps)
            .all(|(old, new)| old.action == new.action)
    );
    assert_eq!(old.terminal_condition, new.terminal_condition);
    assert_ne!(old.summary, new.summary);
    assert_ne!(old_plan.digest, new_plan.digest);
    assert_eq!(
        analysis["source_binding"]["planner"],
        "consumption_aware_v1"
    );
    assert_eq!(
        analysis["source_binding"]["production_source_digest"],
        source_digest(&current).to_hex()
    );
    Ok(())
}

#[test]
fn unused_permitted_workshop_site_changes_the_actual_production_seal() -> Result<()> {
    let snapshot = world()?;
    let make = |x| {
        ProductionRequest::parse_current(
            &json!({
                "template":"production", "quotas":[{"item":"DRINK","minimum":60}],
                "prerequisites":{"assign_labor":false,"workshops":[{
                    "building":"workshop:Still","location":[x,0,10]
                }]}
            })
            .to_string(),
        )
    };
    let a = make(0)?;
    let b = make(1)?;
    let (first, _) = compile(&a, IntentId::new(7), &snapshot, "same human summary")?;
    let (second, _) = compile(&b, IntentId::new(7), &snapshot, "same human summary")?;
    assert_eq!(first.requested_actions, second.requested_actions);
    assert!(
        !first
            .requested_actions
            .iter()
            .any(|step| matches!(step.action, Action::Build { .. }))
    );
    assert_ne!(source_digest(&a), source_digest(&b));
    assert_ne!(
        prepared(&snapshot, &first)?.digest,
        prepared(&snapshot, &second)?.digest
    );
    Ok(())
}

#[test]
fn canonical_current_source_reopens_to_the_identical_plan_and_analysis() -> Result<()> {
    let mut adapter = MemoryAdapter::new(world()?);
    adapter.advance_ticks(1099)?;
    let request = ProductionRequest::parse_current(ORIGINAL)?;
    let source = request.canonical_json();
    let reopened = ProductionRequest::parse(&source)?;
    assert_eq!(request, reopened);
    assert_eq!(source, reopened.canonical_json());
    let before = adapter.snapshot().clone();
    let first = compile(
        &request,
        IntentId::new(7),
        &before,
        "consumption-aware reserve",
    )?;
    let second = compile(
        &reopened,
        IntentId::new(7),
        &before,
        "consumption-aware reserve",
    )?;
    assert_eq!(first, second);
    assert_eq!(prepared(&before, &first.0)?, prepared(&before, &second.0)?);
    assert_eq!(adapter.snapshot(), &before);
    assert!(
        first.0.requested_actions.iter().any(
            |step| matches!(step.action, Action::CreateWorkOrder { amount, .. } if amount > 4)
        )
    );
    Ok(())
}

#[test]
fn upgrading_new_intake_does_not_change_the_legacy_compiler_at_a_meal_boundary() -> Result<()> {
    let mut adapter = MemoryAdapter::new(world()?);
    adapter.advance_ticks(effects::DRINK_INTERVAL_TICKS - 101)?;
    let legacy = ProductionRequest::parse(ORIGINAL)?;
    let before = compile(&legacy, IntentId::new(7), adapter.snapshot(), "legacy")?;
    let current = ProductionRequest::parse_current(ORIGINAL)?;
    let upgraded = compile(&current, IntentId::new(7), adapter.snapshot(), "new")?;
    let after = compile(
        &ProductionRequest::parse(ORIGINAL)?,
        IntentId::new(7),
        adapter.snapshot(),
        "legacy",
    )?;
    assert_eq!(before, after);
    assert!(matches!(
        before.0.requested_actions[0].action,
        Action::CreateWorkOrder { amount: 4, .. }
    ));
    assert!(matches!(upgraded.0.requested_actions[0].action,
        Action::CreateWorkOrder { amount, .. } if amount > 4));
    assert_eq!(before.0.terminal_condition, upgraded.0.terminal_condition);
    Ok(())
}

#[test]
fn full_source_seal_obeys_summary_bounds_without_changing_legacy_summaries() -> Result<()> {
    let snapshot = world()?;
    let legacy = ProductionRequest::parse(ORIGINAL)?;
    let current = ProductionRequest::parse_current(ORIGINAL)?;
    let bound = StaticPlanner::default().policy.max_string_bytes;
    let summary = "x".repeat(bound);
    assert_eq!(
        compile(&legacy, IntentId::new(7), &snapshot, &summary)?
            .0
            .summary,
        summary
    );
    assert_eq!(
        compile(&current, IntentId::new(7), &snapshot, &summary)
            .err()
            .map(|error| error.code),
        Some(ErrorCode::InvalidRequest)
    );
    let suffix = format!(" [production source {}]", source_digest(&current).to_hex());
    let maximum = "x".repeat(bound - suffix.len());
    assert_eq!(
        compile(&current, IntentId::new(7), &snapshot, &maximum)?
            .0
            .summary
            .len(),
        bound
    );
    assert!(compile(&current, IntentId::new(7), &snapshot, "bad\0summary").is_err());
    Ok(())
}
