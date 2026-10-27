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

#[test]
fn mixed_furniture_and_food_source_replays_the_identical_sealed_original_goal() -> Result<()> {
    let snapshot = world()?;
    let request = ProductionRequest::parse_current(
        r#"{"template":"production","quotas":[{"item":"BED","minimum":2},{"item":"DOOR","minimum":2},{"item":"TABLE","minimum":1},{"item":"CHAIR","minimum":1},{"item":"DRINK","minimum":40},{"item":"FOOD","minimum":60}]}"#,
    )?;
    let archived = request.canonical_json();
    let reopened = ProductionRequest::parse(&archived)?;
    let first = compile(&request, IntentId::new(74), &snapshot, "supply four rooms")?;
    let replay = compile(&reopened, IntentId::new(74), &snapshot, "supply four rooms")?;
    assert_eq!(first, replay);
    assert_eq!(
        prepared(&snapshot, &first.0)?,
        prepared(&snapshot, &replay.0)?
    );
    assert_eq!(archived, reopened.canonical_json());
    let terminal = format!("{:?}", first.0.terminal_condition);
    for field in [
        effects::STOCK_BED_FIELD,
        effects::STOCK_DOOR_FIELD,
        effects::STOCK_TABLE_FIELD,
        effects::STOCK_CHAIR_FIELD,
        effects::STOCK_DRINK_FIELD,
        effects::STOCK_FOOD_FIELD,
    ] {
        assert!(terminal.contains(field), "original quota missing: {field}");
    }
    assert_ne!(
        prepared(&snapshot, &first.0)?.digest,
        dfmcp_core::Digest32::ZERO
    );
    Ok(())
}

#[test]
fn exact_future_reserve_tick_is_sealed_even_when_the_action_program_is_identical() -> Result<()> {
    let snapshot = world()?;
    let make = |until| {
        ProductionRequest::parse_current(
            &json!({
                "template":"production", "quotas":[{"item":"DRINK","minimum":60}],
                "reserve_until_tick":until,
            })
            .to_string(),
        )
    };
    let first_request = make(2401)?;
    let second_request = make(2501)?;
    let (first, first_analysis) = compile(
        &first_request,
        IntentId::new(74),
        &snapshot,
        "future reserve",
    )?;
    let (second, _) = compile(
        &second_request,
        IntentId::new(74),
        &snapshot,
        "future reserve",
    )?;
    assert_eq!(first.requested_actions, second.requested_actions);
    assert_eq!(first.terminal_condition, second.terminal_condition);
    assert_ne!(
        source_digest(&first_request),
        source_digest(&second_request)
    );
    assert_ne!(
        prepared(&snapshot, &first)?.digest,
        prepared(&snapshot, &second)?.digest
    );
    assert_eq!(
        first_analysis["source_binding"]["planner"],
        "consumption_reserve_v1"
    );
    assert_eq!(
        first_analysis["source_binding"]["production_source_digest"],
        source_digest(&first_request).to_hex()
    );
    assert_eq!(first_analysis["consumption"]["reserve_until_tick"], 2401);
    Ok(())
}

#[test]
fn archived_reserve_source_recreates_the_exact_intent_analysis_and_prepared_seal() -> Result<()> {
    let snapshot = world()?;
    let request = ProductionRequest::parse_current(
        r#"{"template":"production","quotas":[{"item":"DRINK","minimum":40},{"item":"FOOD","minimum":60},{"item":"BED","minimum":2}],"reserve_until_tick":2401,"prerequisites":{"assign_labor":false,"workshops":[{"building":"workshop:Still","location":[0,0,10]}]}}"#,
    )?;
    let original_bytes = request.canonical_json();
    let reopened = ProductionRequest::parse(&original_bytes)?;
    assert_eq!(reopened, request);
    assert_eq!(reopened.canonical_json(), original_bytes);
    let before = snapshot.clone();
    let first = compile(
        &request,
        IntentId::new(75),
        &snapshot,
        "original future reserve",
    )?;
    let again = compile(
        &reopened,
        IntentId::new(75),
        &snapshot,
        "original future reserve",
    )?;
    assert_eq!(first, again);
    assert_eq!(
        prepared(&snapshot, &first.0)?,
        prepared(&snapshot, &again.0)?
    );
    assert_eq!(snapshot, before);
    assert_eq!(request.reserve_until_tick(), Some(2401));
    Ok(())
}

#[test]
fn explicit_reserve_is_a_separate_generation_without_altering_historical_source_bytes() -> Result<()>
{
    let snapshot = world()?;
    let legacy = ProductionRequest::parse(ORIGINAL)?;
    let previous = ProductionRequest::parse_current(ORIGINAL)?;
    let reserve = ProductionRequest::parse_current(
        r#"{"template":"production","quotas":[{"item":"DRINK","minimum":60}],"reserve_until_tick":100}"#,
    )?;
    assert_eq!(legacy.canonical_json(), ORIGINAL);
    assert_eq!(
        previous.canonical_json(),
        r#"{"planner":"consumption_aware_v1","quotas":[{"item":"DRINK","minimum":60}],"template":"production"}"#
    );
    assert_eq!(previous.reserve_until_tick(), None);
    assert_eq!(
        reserve.planner(),
        Some(ProductionPlanner::ConsumptionReserveV1)
    );
    let old = compile(&previous, IntentId::new(76), &snapshot, "reserve")?;
    let new = compile(&reserve, IntentId::new(76), &snapshot, "reserve")?;
    assert_eq!(old.0.requested_actions, new.0.requested_actions);
    assert_eq!(old.0.terminal_condition, new.0.terminal_condition);
    assert_ne!(
        prepared(&snapshot, &old.0)?.digest,
        prepared(&snapshot, &new.0)?.digest
    );
    assert!(old.1["consumption"].get("reserve_until_tick").is_none());
    assert_eq!(
        compile(
            &ProductionRequest::parse(&previous.canonical_json())?,
            IntentId::new(76),
            &snapshot,
            "reserve"
        )?,
        old,
    );
    Ok(())
}
