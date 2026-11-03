use super::*;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, FortressId, ObservationCursor, RequestId,
    SessionId, WorkBudget,
};
use dfmcp_world::{EntityRecord, Fact, FactSource, WorldGraph, WorldSnapshot};
use std::collections::BTreeMap;

fn snapshot() -> WorldSnapshot {
    let mut snapshot = WorldSnapshot::new(
        FortressId::new(1),
        GameTick(100),
        ObservationCursor::ORIGIN,
        true,
        WorldGraph::default(),
    );
    for (id, native) in [(EntityId::new(43), 42), (EntityId::new(44), 43)] {
        snapshot.graph.entities.insert(
            id,
            EntityRecord {
                id,
                generation: 1,
                revision: 1,
                kind: EntityKind::Unit,
                label: format!("citizen {native}"),
                fields: BTreeMap::from([(
                    "raw_unit_id".into(),
                    Fact::known(
                        Value::I64(native),
                        snapshot.tick,
                        FactSource::Derived("dfmcp.lab-scenario/1".into()),
                        Digest32::ZERO,
                    ),
                )]),
            },
        );
    }
    snapshot.refresh_hash();
    snapshot
}
fn context(s: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: s.anchor(),
        budget: WorkBudget::default(),
        grants: vec![CapabilityGrant {
            capability: Capability::Plan,
            scope: CapabilityScope::default(),
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        }],
        cancellation_requested: false,
    }
}
fn request() -> Request {
    Request {
        expected_observation: "11".repeat(32),
        units: vec!["44".into(), "43".into()],
        labor: "CARPENTER".into(),
        enabled: true,
        recipe: "wooden_bed".into(),
        amount: 1,
        deadline_tick: 300,
        poll_interval_ticks: 10,
        stable_samples: 2,
    }
}
#[test]
fn combined_plan_is_sealed_once_and_uses_original_step_keys() -> Result<()> {
    let snapshot = snapshot();
    let context = context(&snapshot);
    let evidence = PredicateEvidence::laboratory(&snapshot)?;
    let plan = request().compile(&evidence, &context)?;
    assert!(plan.digest_is_valid());
    assert_eq!(plan.steps.len(), 2);
    assert_eq!(plan.steps[0].id, StepId::ZERO);
    assert_eq!(plan.steps[1].id, StepId::new(1));
    assert!(plan.steps[0].depends_on.is_empty());
    assert_eq!(plan.steps[1].depends_on, vec![StepId::ZERO]);
    assert!(plan.steps[0].obligation.is_none());
    assert!(plan.steps[1].obligation.is_some());
    assert!(plan.steps.iter().all(|s| s.compensation.is_none()));
    for step in &plan.steps {
        assert_eq!(
            step.idempotency_key,
            derive_step_idempotency_key(plan.intent_id, context.anchor, step.id, &step.action)
        );
    }
    let entity = created_entity_id(&plan.steps[1].idempotency_key, 0);
    let goal = &plan.steps[1].postconditions;
    assert!(goal.iter().any(|p| matches!(p, Predicate::All(children) if children.iter().any(|p| matches!(p, Predicate::FieldCompare { entity_id, field, value: Value::U64(0), .. } if *entity_id == entity && field == "amount_remaining")))));
    dfmcp_adapter::live_production::validate_plan(&plan)?;
    let mut reordered = request();
    reordered.units.reverse();
    assert_eq!(plan, reordered.compile(&evidence, &context)?);
    let mut changed = request();
    changed.expected_observation = "22".repeat(32);
    assert_ne!(
        plan.intent_id,
        changed.compile(&evidence, &context)?.intent_id
    );
    Ok(())
}
#[test]
fn closed_request_and_roster_reject_hostile_inputs() -> Result<()> {
    let snapshot = snapshot();
    let context = context(&snapshot);
    let evidence = PredicateEvidence::laboratory(&snapshot)?;
    let value = serde_json::json!({"expected_observation":"11".repeat(32),"units":["43"],"labor":"CARPENTER","enabled":true,"recipe":"wooden_bed","amount":1,"deadline_tick":300});
    for key in ["native_unit_id", "native_job_id", "path", "plan"] {
        let mut hostile = value.clone();
        hostile[key] = 42.into();
        assert!(Request::parse(&hostile.to_string()).is_err());
    }
    assert!(Request::parse(&" ".repeat(4097)).is_err());
    let mut r = request();
    r.units.push("43".into());
    assert!(r.validate().is_err());
    let mut r = request();
    r.units = vec!["999".into()];
    assert!(r.compile(&evidence, &context).is_err());
    let mut r = request();
    r.recipe = "unlisted".into();
    assert!(r.validate().is_err());
    let mut r = request();
    r.amount = 101;
    assert!(r.validate().is_err());
    let mut r = request();
    r.deadline_tick = 119;
    assert!(r.compile(&evidence, &context).is_err());
    let mut r = request();
    r.units = (1..=33).map(|n| n.to_string()).collect();
    assert!(r.validate().is_err());
    Ok(())
}
