#![allow(dead_code, unused_imports)]
include!("../semantic_workforce_runtime/fixtures.rs");
use super::ExistingLaborPortfolioRequest;
use dfmcp_adapter::live_production::{ExistingLaborRequirement, ProductionLayout};

fn raw() -> serde_json::Value {
    serde_json::json!({"kind":"existing_labor_portfolio","expected_observation":"11".repeat(32),
        "units":["44","43"],"labor":"MINE",
        "orders":[{"recipe":"wooden_table","amount":4}, {"recipe":"wooden_bed","amount":2},
            {"recipe":"wooden_door","amount":3,"objective":{"kind":"observed_inventory_minimum","item_type":"DOOR","minimum_stack_units":3}}],
        "deadline_tick":300,"poll_interval_ticks":5,"stable_samples":2})
}
fn requirement() -> Result<ExistingLaborRequirement> {
    ExistingLaborRequirement::new(
        "MINE".into(),
        vec![(EntityId::new(43), 142), (EntityId::new(44), 143)],
    )
}

#[test]
fn workshop_bound_existing_portfolio_rebuilds_every_order_and_preserves_worker_requirement()
-> Result<()> {
    let (_, projection) = observation(&[42, 43])?;
    let mut c = context(&projection.snapshot);
    c.budget.max_actions = 4;
    let evidence = PredicateEvidence::scoped(&projection.snapshot, policy(&projection.snapshot))?;
    let baselines =
        crate::semantic_work_order_runtime::inventory_baseline::InventoryBaselines::default();
    let required = requirement()?;
    let request = ExistingLaborPortfolioRequest::parse(&raw().to_string())?;
    let legacy = request.compile(&evidence, &required, &c)?;
    assert_eq!(
        legacy,
        request.compile_with_workshop(&evidence, &required, &baselines, &c, None)?
    );
    let first_origin = Digest32::of_bytes(b"first completed workshop");
    let second_origin = Digest32::of_bytes(b"second completed workshop");
    let first =
        request.compile_with_workshop(&evidence, &required, &baselines, &c, Some(first_origin))?;
    let second =
        request.compile_with_workshop(&evidence, &required, &baselines, &c, Some(second_origin))?;
    super::super::workshop_binding::assert_compiled_binding(
        &legacy,
        &first,
        &second,
        first_origin,
        second_origin,
    )?;
    assert_eq!(
        ProductionLayout::from_plan(&first)?.existing_labor(),
        Some(&required)
    );
    assert!(
        request
            .compile_with_workshop(&evidence, &required, &baselines, &c, Some(Digest32::ZERO))
            .is_err()
    );
    Ok(())
}

#[test]
fn all_original_leaves_have_actual_zero_based_keys_and_no_labor_receipt_dependency() -> Result<()> {
    let (_, projection) = observation(&[42, 43])?;
    let mut c = context(&projection.snapshot);
    c.budget.max_actions = 4;
    c.grants
        .retain(|grant| grant.capability != Capability::ConfigureLabor);
    let evidence = PredicateEvidence::scoped(&projection.snapshot, policy(&projection.snapshot))?;
    let request = ExistingLaborPortfolioRequest::parse(&raw().to_string())?;
    let requirement = requirement()?;
    let plan = request.compile(&evidence, &requirement, &c)?;
    let layout = ProductionLayout::from_plan(&plan)?;
    assert_eq!(layout.assignment_step(), None);
    assert_eq!(
        layout.order_steps(),
        &[StepId::ZERO, StepId::new(1), StepId::new(2)]
    );
    assert_eq!(layout.existing_labor(), Some(&requirement));
    assert!(
        !plan
            .required_capabilities
            .contains(&Capability::ConfigureLabor)
    );
    assert!(
        plan.required_capabilities
            .contains(&Capability::ConfigureProduction)
    );
    for (index, step) in plan.steps.iter().enumerate() {
        assert_eq!(step.id, StepId::new(index as u32));
        assert_eq!(layout.order_ordinal(step.id), Some(index));
        assert!(step.depends_on.is_empty());
        assert!(step.compensation.is_none());
        assert!(matches!(step.action, Action::CreateWorkOrder { .. }));
        assert_eq!(step.preconditions, [Predicate::Paused(true)]);
        assert_eq!(
            step.idempotency_key,
            derive_step_idempotency_key(plan.intent_id, c.anchor, step.id, &step.action)
        );
        let obligation = step
            .obligation
            .as_ref()
            .ok_or_else(|| invalid("obligation missing"))?;
        assert_eq!(obligation.deadline_tick, GameTick(300));
        assert_eq!(obligation.poll_interval_ticks, 5);
        assert_eq!(obligation.stable_for_observations, 2);
    }
    assert!(
        dfmcp_adapter::semantic_work_orders::ObservedInventoryObjective::from_selected_plan(
            &plan,
            StepId::new(1)
        )
        .is_some()
    );
    let mut parent = vec![requirement.predicate()];
    parent.extend(
        plan.steps
            .iter()
            .flat_map(|step| step.postconditions.clone()),
    );
    assert_eq!(plan.terminal_condition, Predicate::All(parent).normalized());
    let limits = dfmcp_adapter::live_production::archive_limits();
    let (bytes, seal) = dfmcp_intent::plan_archive::encode(&plan, limits)?;
    assert_eq!(
        dfmcp_intent::plan_archive::decode(&bytes, seal, limits)?,
        plan
    );
    assert_eq!(super::super::host_plan(&plan)?.archive_seal(), seal);
    Ok(())
}

#[test]
fn normalized_inputs_and_every_leaf_are_bound_into_a_separate_original_identity() -> Result<()> {
    let (_, projection) = observation(&[42, 43])?;
    let mut c = context(&projection.snapshot);
    c.budget.max_actions = 4;
    let evidence = PredicateEvidence::scoped(&projection.snapshot, policy(&projection.snapshot))?;
    let required = requirement()?;
    let request = ExistingLaborPortfolioRequest::parse(&raw().to_string())?;
    let original = request.compile(&evidence, &required, &c)?;
    let mut changed = raw();
    changed["orders"]
        .as_array_mut()
        .ok_or_else(|| invalid("test orders"))?
        .reverse();
    changed["units"] = serde_json::json!(["43", "44"]);
    assert_eq!(
        ExistingLaborPortfolioRequest::parse(&changed.to_string())?
            .compile(&evidence, &required, &c)?,
        original
    );
    for changed in [
        {
            let mut v = raw();
            v["orders"][0]["amount"] = 5.into();
            v
        },
        {
            let mut v = raw();
            v["orders"][2]["objective"]["minimum_stack_units"] = 4.into();
            v
        },
        {
            let mut v = raw();
            v["deadline_tick"] = 301.into();
            v
        },
        {
            let mut v = raw();
            v["poll_interval_ticks"] = 6.into();
            v
        },
        {
            let mut v = raw();
            v["stable_samples"] = 3.into();
            v
        },
    ] {
        let changed = ExistingLaborPortfolioRequest::parse(&changed.to_string())?
            .compile(&evidence, &required, &c)?;
        assert_ne!(changed.intent_id, original.intent_id);
        assert!(
            changed
                .steps
                .iter()
                .zip(&original.steps)
                .all(|(a, b)| a.idempotency_key != b.idempotency_key)
        );
    }
    let replaced = ExistingLaborRequirement::new(
        "MINE".into(),
        vec![(EntityId::new(43), 242), (EntityId::new(44), 143)],
    )?;
    assert_ne!(
        request.compile(&evidence, &replaced, &c)?.intent_id,
        original.intent_id
    );
    let mut one = request.clone();
    one.orders.truncate(1);
    let portfolio = one.compile(&evidence, &required, &c)?;
    let single = one
        .request(&one.orders[0])
        .compile(&evidence, &required, &c)?;
    assert_eq!(portfolio.steps.len(), 1);
    assert_ne!(portfolio.intent_id, single.intent_id);
    assert_ne!(
        portfolio.steps[0].idempotency_key,
        single.steps[0].idempotency_key
    );
    Ok(())
}

#[test]
fn closed_portfolio_rejects_unbounded_duplicate_or_caller_asserted_worker_facts() {
    for key in [
        "enabled",
        "eligible",
        "capture",
        "historical_figure_id",
        "native_unit_id",
        "recipe",
    ] {
        let mut value = raw();
        value[key] = true.into();
        assert!(
            ExistingLaborPortfolioRequest::parse(&value.to_string()).is_err(),
            "{key}"
        );
    }
    for value in [
        {
            let mut v = raw();
            v["kind"] = "portfolio".into();
            v
        },
        {
            let mut v = raw();
            v["orders"] = serde_json::json!([]);
            v
        },
        {
            let mut v = raw();
            v["orders"] = serde_json::Value::Array(vec![
                serde_json::json!({"recipe":"wooden_bed","amount":1});
                5
            ]);
            v
        },
        {
            let mut v = raw();
            v["orders"][0] = v["orders"][1].clone();
            v
        },
        {
            let mut v = raw();
            v["orders"][0]["amount"] = 101.into();
            v
        },
        {
            let mut v = raw();
            v["orders"][0]["depends_on"] = serde_json::json!([0]);
            v
        },
        {
            let mut v = raw();
            v["orders"][2]["objective"]["item_type"] = "BED".into();
            v
        },
        {
            let mut v = raw();
            v["units"] = serde_json::json!(["43", "43"]);
            v
        },
    ] {
        assert!(
            ExistingLaborPortfolioRequest::parse(&value.to_string()).is_err(),
            "{value}"
        );
    }
    assert!(ExistingLaborPortfolioRequest::parse(&" ".repeat(8193)).is_err());
}

#[test]
fn portfolio_requires_qualified_identity_exact_selected_workers_and_original_timing() -> Result<()>
{
    let (_, projection) = observation(&[42, 43])?;
    let mut c = context(&projection.snapshot);
    c.budget.max_actions = 4;
    let evidence = PredicateEvidence::scoped(&projection.snapshot, policy(&projection.snapshot))?;
    let required = requirement()?;
    let request = ExistingLaborPortfolioRequest::parse(&raw().to_string())?;
    assert!(
        request
            .compile(
                &PredicateEvidence::untrusted(&projection.snapshot)?,
                &required,
                &c
            )
            .is_err()
    );
    let subset = ExistingLaborRequirement::new("MINE".into(), vec![(EntityId::new(43), 142)])?;
    assert!(request.compile(&evidence, &subset, &c).is_err());
    let mut changed = request.clone();
    changed.deadline_tick = 129;
    assert!(changed.compile(&evidence, &required, &c).is_err());
    let mut stale = c.clone();
    stale.anchor.cursor.sequence += 1;
    assert!(request.compile(&evidence, &required, &stale).is_err());
    c.budget.max_actions = 2;
    assert!(request.compile(&evidence, &required, &c).is_err());
    Ok(())
}
