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
        objective: None,
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

#[test]
fn inventory_objective_retains_whole_parent_and_exact_selected_step() -> Result<()> {
    use dfmcp_adapter::semantic_work_orders::ObservedInventoryObjective;
    let snapshot = snapshot();
    let context = context(&snapshot);
    let evidence = PredicateEvidence::laboratory(&snapshot)?;
    let legacy = request().compile(&evidence, &context)?;
    let mut requested = request();
    requested.objective = Some(Objective::ObservedInventoryMinimum {
        item_type: "BED".into(),
        minimum_stack_units: 3,
    });
    let plan = requested.compile(&evidence, &context)?;
    assert_ne!(plan.intent_id, legacy.intent_id);
    assert_eq!(plan.anchor, legacy.anchor);
    assert_eq!(plan.steps.len(), 2);
    assert_eq!(plan.steps[1].depends_on, [plan.steps[0].id]);
    assert_eq!(plan.steps[0].action, legacy.steps[0].action);
    assert_eq!(plan.steps[1].action, legacy.steps[1].action);
    assert!(ObservedInventoryObjective::from_plan(&plan).is_none());
    assert!(ObservedInventoryObjective::from_selected_plan(&plan, plan.steps[0].id).is_none());
    let selected = ObservedInventoryObjective::from_selected_plan(&plan, plan.steps[1].id)
        .ok_or_else(|| invalid("selected inventory objective missing"))?;
    assert_eq!(selected.item_type, "BED");
    assert_eq!(selected.min_stack_units, 3);
    let obligation = plan.steps[1]
        .obligation
        .as_ref()
        .ok_or_else(|| invalid("obligation missing"))?;
    assert_eq!(obligation.deadline_tick, GameTick(requested.deadline_tick));
    assert_eq!(
        obligation.poll_interval_ticks,
        requested.poll_interval_ticks
    );
    assert_eq!(obligation.stable_for_observations, requested.stable_samples);
    assert_eq!(plan.steps[1].postconditions, [obligation.terminal.clone()]);
    assert_eq!(
        plan.terminal_condition,
        dfmcp_world::Predicate::All(vec![
            Predicate::All(plan.steps[0].postconditions.clone()).normalized(),
            obligation.terminal.clone(),
        ])
        .normalized()
    );
    let limits = dfmcp_adapter::live_production::archive_limits();
    let (archive, seal) = dfmcp_intent::plan_archive::encode(&plan, limits)?;
    assert_eq!(
        dfmcp_intent::plan_archive::decode(&archive, seal, limits)?,
        plan
    );
    for step in &plan.steps {
        assert_eq!(
            step.idempotency_key,
            derive_step_idempotency_key(plan.intent_id, context.anchor, step.id, &step.action)
        );
    }
    Ok(())
}

#[test]
fn inventory_choice_is_closed_recipe_matched_and_identity_bound() -> Result<()> {
    let base = serde_json::json!({"expected_observation":"11".repeat(32),"units":["43"],
        "labor":"CARPENTER","enabled":true,"recipe":"wooden_bed","amount":1,"deadline_tick":300});
    let snapshot = snapshot();
    let c = context(&snapshot);
    let evidence = PredicateEvidence::laboratory(&snapshot)?;
    let legacy = Request::parse(&base.to_string())?.compile(&evidence, &c)?;
    let mut explicit_null = base.clone();
    explicit_null["objective"] = serde_json::Value::Null;
    assert_eq!(
        legacy,
        Request::parse(&explicit_null.to_string())?.compile(&evidence, &c)?
    );
    let mut selected = base.clone();
    selected["objective"] = serde_json::json!({"kind":"observed_inventory_minimum",
        "item_type":"BED","minimum_stack_units":1});
    let one = Request::parse(&selected.to_string())?.compile(&evidence, &c)?;
    selected["objective"]["minimum_stack_units"] = 2.into();
    let two = Request::parse(&selected.to_string())?.compile(&evidence, &c)?;
    assert_ne!(one.intent_id, two.intent_id);
    assert_ne!(one.steps[1].idempotency_key, two.steps[1].idempotency_key);
    for (field, value) in [
        ("item_type", serde_json::json!("DOOR")),
        ("item_type", serde_json::json!("bed")),
        ("minimum_stack_units", serde_json::json!(0)),
        ("minimum_stack_units", serde_json::json!(1_000_001)),
        ("minimum_stack_units", serde_json::json!(-1)),
        ("kind", serde_json::json!("infer_order_completion")),
        ("material", serde_json::json!("WOOD")),
    ] {
        let mut bad = selected.clone();
        bad["objective"][field] = value;
        assert!(
            Request::parse(&bad.to_string()).is_err(),
            "accepted {field}"
        );
    }
    for (recipe, item_type) in [
        ("wooden_bed", "BED"),
        ("wooden_door", "DOOR"),
        ("wooden_table", "TABLE"),
        ("wooden_chair", "CHAIR"),
    ] {
        let mut matched = selected.clone();
        matched["recipe"] = recipe.into();
        matched["objective"]["item_type"] = item_type.into();
        assert!(
            Request::parse(&matched.to_string())?
                .compile(&evidence, &c)
                .is_ok()
        );
    }
    Ok(())
}

#[test]
fn retained_order_objective_binds_the_original_step_and_exact_completion_predicates() -> Result<()>
{
    use dfmcp_adapter::semantic_work_orders::{ObservedInventoryObjective, RetainedOrderObjective};
    let snapshot = snapshot();
    let c = context(&snapshot);
    let evidence = PredicateEvidence::laboratory(&snapshot)?;
    for recipe in ["wooden_bed", "wooden_door", "wooden_table", "wooden_chair"] {
        let mut request = request();
        request.recipe = recipe.into();
        request.amount = 7;
        let plan = request.compile(&evidence, &c)?;
        let unchanged = plan.clone();
        let step = &plan.steps[1];
        let goal = RetainedOrderObjective::from_selected_plan(&plan, step.id)
            .ok_or_else(|| invalid("default retained-order objective absent"))?;
        assert_eq!(
            goal.order_entity_id,
            created_entity_id(&step.idempotency_key, 0)
        );
        assert_eq!(goal.amount_total, 7);
        assert!(ObservedInventoryObjective::from_selected_plan(&plan, step.id).is_none());
        assert!(RetainedOrderObjective::from_selected_plan(&plan, StepId::ZERO).is_none());
        assert!(RetainedOrderObjective::from_selected_plan(&plan, StepId::new(2)).is_none());
        assert_eq!(plan, unchanged);
    }
    Ok(())
}

#[test]
fn retained_order_recognition_refuses_weakened_substituted_and_inventory_goals() -> Result<()> {
    use dfmcp_adapter::semantic_work_orders::RetainedOrderObjective;
    let snapshot = snapshot();
    let c = context(&snapshot);
    let evidence = PredicateEvidence::laboratory(&snapshot)?;
    let original = request().compile(&evidence, &c)?;
    for changed_field in [
        "amount_total",
        "amount_remaining",
        "validated",
        "active",
        "entity",
    ] {
        let mut changed = original.clone();
        let step = &mut changed.steps[1];
        let spec = step
            .obligation
            .as_mut()
            .ok_or_else(|| invalid("original obligation absent"))?;
        let Predicate::All(clauses) = &mut spec.terminal else {
            return Err(invalid("default goal must be a conjunction"));
        };
        for clause in clauses {
            if let Predicate::FieldCompare {
                entity_id,
                field,
                value,
                ..
            } = clause
            {
                if changed_field == "entity" {
                    *entity_id = EntityId::new(42);
                } else if field == changed_field {
                    *value = match changed_field {
                        "amount_total" => Value::U64(2),
                        "amount_remaining" => Value::U64(1),
                        "validated" => Value::Bool(false),
                        _ => Value::Bool(true),
                    };
                }
            }
        }
        step.postconditions = vec![spec.terminal.clone()];
        changed.digest = changed.compute_digest();
        changed.id = changed.expected_id();
        changed.validate_structure()?;
        assert!(RetainedOrderObjective::from_selected_plan(&changed, StepId::new(1)).is_none());
    }
    for mode in 0..3 {
        let mut changed = original.clone();
        match mode {
            0 => {
                // A valid but weaker predicate isolates objective recognition
                // from the plan validator's independent trivial-goal refusal.
                changed.steps[1].postconditions = vec![Predicate::FieldCompare {
                    entity_id: created_entity_id(&changed.steps[1].idempotency_key, 0),
                    field: "amount_remaining".into(),
                    op: CompareOp::Eq,
                    value: Value::U64(0),
                }]
            }
            1 => changed.steps[1].obligation = None,
            _ => {
                changed.steps[1]
                    .obligation
                    .as_mut()
                    .ok_or_else(|| invalid("obligation absent"))?
                    .failure = Some(Predicate::Paused(false))
            }
        }
        changed.digest = changed.compute_digest();
        changed.id = changed.expected_id();
        if mode == 1 {
            // Temporal work orders already require a bounded obligation at
            // the structural boundary; recognition must reject them too.
            assert!(changed.validate_structure().is_err());
        } else {
            changed.validate_structure()?;
        }
        assert!(RetainedOrderObjective::from_selected_plan(&changed, StepId::new(1)).is_none());
    }
    let mut inventory = request();
    inventory.objective = Some(Objective::ObservedInventoryMinimum {
        item_type: "BED".into(),
        minimum_stack_units: 1,
    });
    let inventory = inventory.compile(&evidence, &c)?;
    assert!(RetainedOrderObjective::from_selected_plan(&inventory, StepId::new(1)).is_none());
    Ok(())
}
