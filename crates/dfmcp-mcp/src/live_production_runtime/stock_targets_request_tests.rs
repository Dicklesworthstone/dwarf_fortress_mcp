use super::super::stock_target_request::tests::{stock_fixture, stock_policy, stock_workers};
use super::super::stock_targets_binding;
use super::*;
use dfmcp_adapter::live_jobs::LiveJobObservation;
use dfmcp_adapter::live_operations::{LiveItem, LiveOperationsObservation};
use dfmcp_adapter::live_production::ProductionLayout;
use dfmcp_adapter::semantic_work_orders::{
    InventoryBaselineProjection, ObservedInventoryIncreaseObjective,
};
use dfmcp_core::{Capability, ErrorCode, GameTick, MapCoord, StepId};
use dfmcp_intent::Action;
use dfmcp_world::{EntityKind, EvidenceCoverage, EvidencePolicy, Predicate};

fn portfolio_fixture() -> Result<(
    dfmcp_adapter::live_projection::LiveWorldProjection,
    OperationContext,
)> {
    let (projection, mut c) = stock_fixture()?;
    c.budget.max_actions = 5;
    Ok((projection, c))
}

fn request() -> Result<StockTargetsRequest> {
    StockTargetsRequest::parse(
        &serde_json::json!({
            "kind":"stock_targets","expected_observation":"11".repeat(32),
            "units":["44","43"],"labor":"CARPENTER",
            "targets":[
                {"recipe":"wooden_door","minimum_stack_units":3},
                {"recipe":"wooden_bed","minimum_stack_units":6}
            ],
            "deadline_tick":300,"poll_interval_ticks":5,"stable_samples":2
        })
        .to_string(),
    )
}

fn assessments(
    request: &StockTargetsRequest,
    c: &OperationContext,
    bed: u32,
    door: u32,
) -> Result<Vec<StockTargetAssessment>> {
    let items = [(1, "BED", bed), (2, "DOOR", door)]
        .into_iter()
        .map(|(native_id, kind, stack_size)| LiveItem {
            native_id,
            item_type: 5,
            type_key: kind.into(),
            subtype: -1,
            material_type: 420,
            material_index: 0,
            stack_size,
            raw_position: MapCoord::new(1, 1, 1),
            flags: 0,
            container_native_id: None,
            holder_building_native_id: None,
        })
        .collect();
    let projection = InventoryBaselineProjection::new(
        c.anchor,
        LiveOperationsObservation {
            jobs: LiveJobObservation {
                bridge_generation: 601,
                df_version: "0.51.11".into(),
                dfhack_version: "0.51.11-r1".into(),
                year: 0,
                year_tick: 120,
                paused: true,
                site_id: 4,
                world_folder: "region1".into(),
                next_job_id: 100,
                jobs: Vec::new(),
            },
            next_building_id: 200,
            next_item_id: 300,
            buildings: Vec::new(),
            items,
            attachments: Vec::new(),
        },
        c,
    )?;
    let mut policy = EvidencePolicy::at(projection.snapshot().anchor());
    policy.paused = true;
    policy
        .entity_kinds
        .insert(EntityKind::Item, EvidenceCoverage::Complete);
    policy.sources = projection.observed_sources();
    request
        .targets
        .iter()
        .map(|target| {
            projection.stock_target(
                recipe(&target.recipe)?.0,
                target.minimum_stack_units,
                target.max_order_amount,
                policy.clone(),
                c,
            )
        })
        .collect()
}

#[test]
fn stock_targets_request_is_closed_bounded_and_canonical() -> Result<()> {
    let request = request()?;
    assert!(!request.assign_labor);
    assert_eq!(request.targets[0].recipe, "wooden_bed");
    assert_eq!(request.targets[1].recipe, "wooden_door");
    assert_eq!(request.targets[0].max_order_amount, 100);
    let base = serde_json::json!({
        "kind":"stock_targets","expected_observation":"11".repeat(32),
        "units":["43"],"labor":"CARPENTER",
        "targets":[{"recipe":"wooden_bed","minimum_stack_units":6}],
        "deadline_tick":300
    });
    let defaults = StockTargetsRequest::parse(&base.to_string())?;
    assert_eq!(defaults.poll_interval_ticks, 10);
    assert_eq!(defaults.stable_samples, 1);
    for selection in [false, true] {
        let mut explicit = base.clone();
        explicit["assign_labor"] = selection.into();
        assert_eq!(
            StockTargetsRequest::parse(&explicit.to_string())?.assign_labor,
            selection
        );
    }
    let duplicate =
        base.to_string()
            .replacen("{", "{\"assign_labor\":false,\"assign_labor\":true,", 1);
    assert!(StockTargetsRequest::parse(&duplicate).is_err());
    for (field, value) in [
        ("targets", serde_json::json!([])),
        (
            "targets",
            serde_json::json!([
                {"recipe":"wooden_bed","minimum_stack_units":6},
                {"recipe":"wooden_bed","minimum_stack_units":7}
            ]),
        ),
        ("units", serde_json::json!(["043"])),
        ("units", serde_json::json!(["43", "43"])),
        ("labor", serde_json::json!("")),
        ("stable_samples", serde_json::json!(65)),
        ("poll_interval_ticks", serde_json::json!(0)),
        ("kind", serde_json::json!("stock_target")),
        ("amount", serde_json::json!(1)),
        ("assign_labor", serde_json::json!(null)),
        ("assign_labor", serde_json::json!("true")),
        ("assign_labor", serde_json::json!(1)),
        ("baseline_stack_units", serde_json::json!(2)),
        ("source_digest", serde_json::json!("11".repeat(32))),
    ] {
        let mut changed = base.clone();
        changed[field] = value;
        assert!(
            StockTargetsRequest::parse(&changed.to_string()).is_err(),
            "{field}"
        );
    }
    for (field, value) in [
        ("minimum_stack_units", serde_json::json!(0)),
        ("minimum_stack_units", serde_json::json!(1_000_001)),
        ("max_order_amount", serde_json::json!(0)),
        ("max_order_amount", serde_json::json!(101)),
        ("max_order_amount", serde_json::json!(null)),
        ("recipe", serde_json::json!("wooden_stool")),
        ("amount", serde_json::json!(4)),
        ("objective", serde_json::json!({})),
        ("baseline_stack_units", serde_json::json!(2)),
    ] {
        let mut changed = base.clone();
        changed["targets"][0][field] = value;
        assert!(
            StockTargetsRequest::parse(&changed.to_string()).is_err(),
            "{field}"
        );
    }
    let mut over = base.clone();
    over["targets"] = serde_json::json!([
        {"recipe":"wooden_bed","minimum_stack_units":6},
        {"recipe":"wooden_chair","minimum_stack_units":1},
        {"recipe":"wooden_door","minimum_stack_units":1},
        {"recipe":"wooden_table","minimum_stack_units":1},
        {"recipe":"wooden_bed","minimum_stack_units":1}
    ]);
    assert!(StockTargetsRequest::parse(&over.to_string()).is_err());
    assert!(StockTargetsRequest::parse(&" ".repeat(8193)).is_err());
    Ok(())
}

#[test]
fn stock_targets_mixed_plan_retains_every_minimum_and_only_positive_orders() -> Result<()> {
    let (projection, c) = portfolio_fixture()?;
    let evidence =
        PredicateEvidence::scoped(&projection.snapshot, stock_policy(&projection.snapshot))?;
    let request = request()?;
    let assessments = assessments(&request, &c, 2, 3)?;
    let plan = request.compile(&assessments, &evidence, &stock_workers()?, &c, None)?;
    let layout = ProductionLayout::from_plan(&plan)?;
    assert_eq!(layout.assignment_step(), None);
    assert_eq!(layout.order_steps(), [StepId::ZERO]);
    assert_eq!(
        layout
            .existing_labor()
            .expect("workers")
            .inventory_minima()
            .len(),
        2
    );
    assert_eq!(
        layout.existing_labor().expect("workers").predicate(),
        stock_workers()?.predicate()
    );
    assert!(
        !plan
            .required_capabilities
            .contains(&Capability::ConfigureLabor)
    );
    assert!(matches!(&plan.steps[0].action, Action::CreateWorkOrder {
        amount: 4, job_token, conditions, ..
    } if job_token == "CONSTRUCT_BED" && conditions.is_empty()));
    let goal = ObservedInventoryIncreaseObjective::from_selected_plan(&plan, StepId::ZERO)
        .expect("typed positive increase");
    assert_eq!(goal.baseline_stack_units(), 2);
    assert_eq!(goal.increase_stack_units(), 4);
    assert_eq!(goal.target_stack_units(), 6);
    let binding = stock_targets_binding::requirement(&plan)?.expect("complete binding");
    assert_eq!(
        binding,
        StockTargetsBinding::from_assessments(&assessments)?
    );
    assert_eq!(binding.rows[1].deficit, 0);
    assert!(super::super::stock_target_binding::requirement(&plan)?.is_none());
    assert_eq!(
        plan.steps[0]
            .obligation
            .as_ref()
            .expect("obligation")
            .deadline_tick,
        GameTick(300)
    );
    Ok(())
}

#[test]
fn stock_targets_multiple_deficits_share_exact_census_and_original_timing() -> Result<()> {
    let (projection, c) = portfolio_fixture()?;
    let evidence =
        PredicateEvidence::scoped(&projection.snapshot, stock_policy(&projection.snapshot))?;
    let request = request()?;
    let assessments = assessments(&request, &c, 2, 1)?;
    let plan = request.compile(&assessments, &evidence, &stock_workers()?, &c, None)?;
    assert_eq!(plan.steps.len(), 2);
    assert_ne!(plan.steps[0].idempotency_key, plan.steps[1].idempotency_key);
    for (index, amount) in [4, 2].into_iter().enumerate() {
        let step = &plan.steps[index];
        assert_eq!(step.id, StepId::new(index as u32));
        assert!(step.depends_on.is_empty());
        assert!(
            matches!(step.action, Action::CreateWorkOrder { amount: actual, .. } if actual == amount)
        );
        let goal = ObservedInventoryIncreaseObjective::from_selected_plan(&plan, step.id)
            .expect("positive stock goal");
        assert_eq!(
            goal.baseline_source_digest(),
            assessments[0].source_digest()
        );
        assert_eq!(goal.baseline_anchor(), c.anchor);
        assert_eq!(
            step.obligation
                .as_ref()
                .expect("timing")
                .stable_for_observations,
            2
        );
    }
    assert_eq!(
        stock_targets_binding::requirement(&plan)?
            .expect("binding")
            .rows
            .len(),
        2
    );
    Ok(())
}

#[test]
fn stock_targets_met_minimum_and_complete_census_are_part_of_original_identity() -> Result<()> {
    let (projection, c) = portfolio_fixture()?;
    let evidence =
        PredicateEvidence::scoped(&projection.snapshot, stock_policy(&projection.snapshot))?;
    let request = request()?;
    let values = assessments(&request, &c, 2, 3)?;
    let original = request.compile(&values, &evidence, &stock_workers()?, &c, None)?;
    let mut changed = request.clone();
    changed.targets[1].minimum_stack_units = 2;
    let other = changed.compile(
        &assessments(&changed, &c, 2, 3)?,
        &evidence,
        &stock_workers()?,
        &c,
        None,
    )?;
    assert_ne!(original.intent_id, other.intent_id);
    assert_ne!(
        original.steps[0].idempotency_key,
        other.steps[0].idempotency_key
    );
    let mut mixed = values.clone();
    mixed[1] = assessments(&request, &c, 2, 4)?[1].clone();
    assert!(
        request
            .compile(&mixed, &evidence, &stock_workers()?, &c, None)
            .is_err()
    );
    let all_met = assessments(&request, &c, 6, 3)?;
    assert!(
        request
            .compile(&all_met, &evidence, &stock_workers()?, &c, None)
            .is_err()
    );
    let mut capped = request.clone();
    capped.targets[0].max_order_amount = 2;
    assert!(assessments(&capped, &c, 2, 3).is_err());
    Ok(())
}

#[test]
fn stock_targets_archive_recovery_rejects_dropped_or_changed_target_policy() -> Result<()> {
    let (projection, c) = portfolio_fixture()?;
    let evidence =
        PredicateEvidence::scoped(&projection.snapshot, stock_policy(&projection.snapshot))?;
    let request = request()?;
    let values = assessments(&request, &c, 2, 3)?;
    for origin in [
        None,
        Some(Digest32::of_bytes(b"completed original workshop")),
    ] {
        let plan = request.compile(&values, &evidence, &stock_workers()?, &c, origin)?;
        assert!(plan.summary.len() <= 1024);
        let limits = dfmcp_adapter::live_production::archive_limits();
        let (bytes, seal) = dfmcp_intent::plan_archive::encode(&plan, limits)?;
        let recovered = dfmcp_intent::plan_archive::decode(&bytes, seal, limits)?;
        assert_eq!(recovered, plan);
        assert_eq!(
            stock_targets_binding::requirement(&recovered)?,
            Some(StockTargetsBinding::from_assessments(&values)?)
        );
        assert_eq!(
            super::super::workshop_binding::requirement(&recovered)?,
            origin
        );
        let mut stripped = recovered.clone();
        stripped.summary = "manual existing-worker portfolio".into();
        stripped.digest = stripped.compute_digest();
        stripped.id = stripped.expected_id();
        assert!(stock_targets_binding::requirement(&stripped).is_err());
        let mut changed = recovered.clone();
        let Predicate::All(ref mut atoms) = changed.terminal_condition else {
            panic!("portfolio has a conjunction");
        };
        atoms.retain(|atom| {
            !matches!(atom, Predicate::EntityQuantity {
            filter_value: dfmcp_world::Value::Text(item), ..
        } if item == "DOOR")
        });
        changed.terminal_condition = changed.terminal_condition.normalized();
        changed.digest = changed.compute_digest();
        changed.id = changed.expected_id();
        assert!(stock_targets_binding::requirement(&changed).is_err());
    }
    Ok(())
}

#[test]
fn stock_targets_four_recipes_fit_original_archive_and_fail_atomically_over_action_budget()
-> Result<()> {
    let (projection, c) = portfolio_fixture()?;
    let evidence =
        PredicateEvidence::scoped(&projection.snapshot, stock_policy(&projection.snapshot))?;
    let mut request = request()?;
    request.targets.extend([
        StockTarget {
            recipe: "wooden_chair".into(),
            minimum_stack_units: 1,
            max_order_amount: 100,
        },
        StockTarget {
            recipe: "wooden_table".into(),
            minimum_stack_units: 1,
            max_order_amount: 100,
        },
    ]);
    request.targets.sort_by(|a, b| a.recipe.cmp(&b.recipe));
    let values = assessments(&request, &c, 2, 1)?;
    for origin in [
        None,
        Some(Digest32::of_bytes(
            b"one complete original carpenter workshop",
        )),
    ] {
        let plan = request.compile(&values, &evidence, &stock_workers()?, &c, origin)?;
        assert_eq!(plan.steps.len(), 4);
        assert!(plan.summary.len() > 256 && plan.summary.len() <= 1024);
        let layout = ProductionLayout::from_plan(&plan)?;
        assert_eq!(
            layout.order_steps(),
            [StepId::ZERO, StepId::new(1), StepId::new(2), StepId::new(3)]
        );
        assert_eq!(
            layout
                .existing_labor()
                .expect("workers")
                .inventory_minima()
                .len(),
            4
        );
        let limits = dfmcp_adapter::live_production::archive_limits();
        let (bytes, seal) = dfmcp_intent::plan_archive::encode(&plan, limits)?;
        assert_eq!(
            dfmcp_intent::plan_archive::decode(&bytes, seal, limits)?,
            plan
        );
        assert_eq!(
            stock_targets_binding::requirement(&plan)?
                .expect("binding")
                .rows
                .len(),
            4
        );
    }
    let mut bounded = c.clone();
    bounded.budget.max_actions = 3;
    assert!(
        request
            .compile(&values, &evidence, &stock_workers()?, &bounded, None)
            .is_err()
    );
    // Compilation needs Plan authority and seals the effects required later;
    // it does not authorize native preparation or dispatch by itself.
    let original = request.compile(&values, &evidence, &stock_workers()?, &c, None)?;
    let mut planning_only = c.clone();
    planning_only
        .grants
        .retain(|grant| grant.capability == Capability::Plan);
    let planned = request.compile(&values, &evidence, &stock_workers()?, &planning_only, None)?;
    assert_eq!(planned, original);
    assert!(
        planned
            .required_capabilities
            .contains(&Capability::ConfigureProduction)
    );
    planning_only.grants.clear();
    assert_eq!(
        request
            .compile(&values, &evidence, &stock_workers()?, &planning_only, None)
            .err()
            .map(|cause| cause.code),
        Some(ErrorCode::CapabilityDenied)
    );
    Ok(())
}

#[test]
fn stock_targets_assignment_keeps_default_bytes_and_retains_all_original_minima() -> Result<()> {
    let (projection, c) = portfolio_fixture()?;
    let evidence =
        PredicateEvidence::scoped(&projection.snapshot, stock_policy(&projection.snapshot))?;
    let default = request()?;
    let values = assessments(&default, &c, 2, 3)?;
    let original = default.compile(&values, &evidence, &stock_workers()?, &c, None)?;
    let mut explicit_false = default.clone();
    explicit_false.assign_labor = false;
    assert_eq!(
        explicit_false.compile(&values, &evidence, &stock_workers()?, &c, None)?,
        original
    );
    assert!(original.summary.starts_with("Inventory targets/1 "));
    assert!(
        !stock_targets_binding::requirement(&original)?
            .expect("binding")
            .assign_labor()
    );
    assert!(
        default
            .compile_with_labor_assignment(&values, &evidence, &c, None)
            .is_err()
    );

    let mut assigned = default.clone();
    assigned.assign_labor = true;
    assert!(
        assigned
            .compile(&values, &evidence, &stock_workers()?, &c, None)
            .is_err()
    );
    let plan = assigned.compile_with_labor_assignment(&values, &evidence, &c, None)?;
    let layout = ProductionLayout::from_plan(&plan)?;
    assert_eq!(layout.assignment_step(), Some(StepId::ZERO));
    assert_eq!(layout.order_steps(), [StepId::new(1)]);
    assert!(layout.existing_labor().is_none());
    assert_eq!(layout.inventory_minima().len(), 2);
    assert!(
        plan.required_capabilities
            .contains(&Capability::ConfigureLabor)
    );
    assert!(
        plan.required_capabilities
            .contains(&Capability::ConfigureProduction)
    );
    assert!(plan.summary.starts_with("Inventory targets/2 "));
    assert_ne!(plan.intent_id, original.intent_id);
    assert_ne!(
        plan.steps[1].idempotency_key,
        original.steps[0].idempotency_key
    );
    assert!(
        matches!(&plan.steps[0].action, Action::SetLabor { units, labor, enabled: true }
        if units == &[EntityId::new(43), EntityId::new(44)] && labor == "CARPENTER")
    );
    assert!(plan.steps[0].depends_on.is_empty());
    assert!(plan.steps[0].obligation.is_none());
    assert_eq!(plan.steps[1].depends_on, [StepId::ZERO]);
    assert!(
        matches!(&plan.steps[1].action, Action::CreateWorkOrder { name, amount: 4, .. }
        if name == "native inventory-targets/2 wooden_bed goal")
    );
    let binding = stock_targets_binding::requirement(&plan)?.expect("assigned complete binding");
    assert_eq!(
        binding,
        StockTargetsBinding::from_assessments_with_assignment(&values)?
    );
    assert_eq!(binding.rows[1].deficit, 0);
    assert!(super::super::stock_target_binding::requirement(&plan)?.is_none());
    let mut reordered = assigned.clone();
    reordered.units.reverse();
    assert_eq!(
        reordered.compile_with_labor_assignment(&values, &evidence, &c, None)?,
        plan
    );
    let met = assessments(&assigned, &c, 6, 3)?;
    assert!(
        assigned
            .compile_with_labor_assignment(&met, &evidence, &c, None)
            .is_err()
    );
    Ok(())
}

#[test]
fn assigned_stock_targets_archive_binds_complete_assignment_source_and_target_set() -> Result<()> {
    let (projection, c) = portfolio_fixture()?;
    let evidence =
        PredicateEvidence::scoped(&projection.snapshot, stock_policy(&projection.snapshot))?;
    let mut request = request()?;
    request.assign_labor = true;
    let values = assessments(&request, &c, 2, 3)?;
    let mut baseline = None;
    for origin in [
        None,
        Some(Digest32::of_bytes(b"exact completed Carpenter original")),
    ] {
        let plan = request.compile_with_labor_assignment(&values, &evidence, &c, origin)?;
        let limits = dfmcp_adapter::live_production::archive_limits();
        let (bytes, seal) = dfmcp_intent::plan_archive::encode(&plan, limits)?;
        let recovered = dfmcp_intent::plan_archive::decode(&bytes, seal, limits)?;
        assert_eq!(recovered, plan);
        assert_eq!(
            stock_targets_binding::requirement(&recovered)?,
            Some(StockTargetsBinding::from_assessments_with_assignment(
                &values
            )?)
        );
        assert_eq!(
            super::super::workshop_binding::requirement(&recovered)?,
            origin
        );
        if let Some(previous) = baseline.replace(plan.intent_id) {
            assert_ne!(previous, plan.intent_id);
        }
        let mut changed_plans = Vec::new();
        let mut changed = plan.clone();
        changed.summary = "manual assignment portfolio".into();
        changed_plans.push(changed);
        let mut changed = plan.clone();
        changed.summary = changed
            .summary
            .replace("Inventory targets/2 ", "Inventory targets/1 ");
        changed_plans.push(changed);
        let mut changed = plan.clone();
        if let Action::CreateWorkOrder { name, .. } = &mut changed.steps[1].action {
            *name = "native inventory-targets/1 wooden_bed goal".into();
        }
        changed_plans.push(changed);
        let mut changed = plan.clone();
        if let Action::SetLabor { enabled, .. } = &mut changed.steps[0].action {
            *enabled = false;
        }
        changed_plans.push(changed);
        let mut changed = plan.clone();
        changed.steps[0].preconditions = vec![Predicate::Paused(true)];
        changed_plans.push(changed);
        let mut changed = plan.clone();
        changed.steps[0].postconditions = vec![Predicate::True];
        changed_plans.push(changed);
        let mut changed = plan.clone();
        let Predicate::All(atoms) = &mut changed.terminal_condition else {
            panic!("complete stock goal is a conjunction");
        };
        atoms.retain(|atom| {
            !matches!(atom, Predicate::EntityQuantity {
            filter_value: dfmcp_world::Value::Text(item), ..
        } if item == "DOOR")
        });
        changed.terminal_condition = changed.terminal_condition.normalized();
        changed_plans.push(changed);
        let mut changed = plan.clone();
        changed.steps[1].depends_on.clear();
        changed_plans.push(changed);
        for mut changed in changed_plans {
            changed.digest = changed.compute_digest();
            changed.id = changed.expected_id();
            assert!(stock_targets_binding::requirement(&changed).is_err());
        }
    }
    let original = request.compile_with_labor_assignment(&values, &evidence, &c, None)?;
    for field in 0..5 {
        let mut changed = request.clone();
        match field {
            0 => changed.targets[1].minimum_stack_units = 2,
            1 => changed.targets[0].max_order_amount = 50,
            2 => changed.deadline_tick += 1,
            3 => changed.poll_interval_ticks += 1,
            4 => changed.stable_samples += 1,
            _ => unreachable!(),
        }
        let values = assessments(&changed, &c, 2, 3)?;
        let other = changed.compile_with_labor_assignment(&values, &evidence, &c, None)?;
        assert_ne!(original.intent_id, other.intent_id);
        assert_ne!(
            original.steps[0].idempotency_key,
            other.steps[0].idempotency_key
        );
        assert_ne!(
            original.steps[1].idempotency_key,
            other.steps[1].idempotency_key
        );
    }
    Ok(())
}

#[test]
fn assigned_stock_targets_four_deficits_require_all_five_original_actions() -> Result<()> {
    let (projection, c) = portfolio_fixture()?;
    let evidence =
        PredicateEvidence::scoped(&projection.snapshot, stock_policy(&projection.snapshot))?;
    let mut request = request()?;
    request.assign_labor = true;
    request.targets.extend([
        StockTarget {
            recipe: "wooden_chair".into(),
            minimum_stack_units: 1,
            max_order_amount: 100,
        },
        StockTarget {
            recipe: "wooden_table".into(),
            minimum_stack_units: 1,
            max_order_amount: 100,
        },
    ]);
    request.targets.sort_by(|a, b| a.recipe.cmp(&b.recipe));
    let values = assessments(&request, &c, 2, 1)?;
    let origin = Some(Digest32::of_bytes(b"completed original workshop"));
    let plan = request.compile_with_labor_assignment(&values, &evidence, &c, origin)?;
    assert_eq!(plan.steps.len(), 5);
    assert!(plan.summary.len() <= 1024);
    let layout = ProductionLayout::from_plan(&plan)?;
    assert_eq!(
        layout.order_steps(),
        [
            StepId::new(1),
            StepId::new(2),
            StepId::new(3),
            StepId::new(4)
        ]
    );
    assert_eq!(layout.inventory_minima().len(), 4);
    for step in &plan.steps[1..] {
        assert_eq!(step.depends_on, [StepId::ZERO]);
        let goal = ObservedInventoryIncreaseObjective::from_selected_plan(&plan, step.id)
            .expect("exact original growth objective");
        assert_eq!(goal.baseline_source_digest(), values[0].source_digest());
        assert_eq!(goal.baseline_anchor(), c.anchor);
    }
    let limits = dfmcp_adapter::live_production::archive_limits();
    let (bytes, seal) = dfmcp_intent::plan_archive::encode(&plan, limits)?;
    assert_eq!(
        dfmcp_intent::plan_archive::decode(&bytes, seal, limits)?,
        plan
    );
    let mut bounded = c.clone();
    bounded.budget.max_actions = 4;
    assert!(
        request
            .compile_with_labor_assignment(&values, &evidence, &bounded, None)
            .is_err()
    );
    // Pure compilation preserves Plan/effect separation. The public Runtime
    // checks current effect authority before publishing a positive assignment
    // plan; the native owners check it again at their effect boundaries.
    let mut planning_only = c.clone();
    planning_only
        .grants
        .retain(|grant| grant.capability == Capability::Plan);
    let planned =
        request.compile_with_labor_assignment(&values, &evidence, &planning_only, origin)?;
    assert_eq!(planned, plan);
    for capability in [Capability::ConfigureLabor, Capability::ConfigureProduction] {
        assert!(planned.required_capabilities.contains(&capability));
    }
    planning_only.grants.clear();
    assert_eq!(
        request
            .compile_with_labor_assignment(&values, &evidence, &planning_only, origin)
            .err()
            .map(|cause| cause.code),
        Some(ErrorCode::CapabilityDenied)
    );
    Ok(())
}

#[test]
fn manager_and_workshop_four_target_original_recovers_all_minima_and_exact_policies() -> Result<()>
{
    use super::super::prerequisite_binding::{self, Origins};
    let (projection, c) = portfolio_fixture()?;
    let evidence =
        PredicateEvidence::scoped(&projection.snapshot, stock_policy(&projection.snapshot))?;
    for assignment in [false, true] {
        let mut request = request()?;
        request.assign_labor = assignment;
        request.targets.extend([
            StockTarget {
                recipe: "wooden_chair".into(),
                minimum_stack_units: 1,
                max_order_amount: 100,
            },
            StockTarget {
                recipe: "wooden_table".into(),
                minimum_stack_units: 1,
                max_order_amount: 100,
            },
        ]);
        request.targets.sort_by(|a, b| a.recipe.cmp(&b.recipe));
        let values = assessments(&request, &c, 2, 1)?;
        let compile = |origins: Origins| {
            if assignment {
                request.compile_with_labor_assignment(&values, &evidence, &c, origins)
            } else {
                request.compile(&values, &evidence, &stock_workers()?, &c, origins)
            }
        };
        let legacy = compile(Origins::default())?;
        prerequisite_binding::assert_manager_variants(&legacy, compile)?;
        let origins = Origins {
            workshop: Some(Digest32::of_bytes(b"workshop")),
            manager: Some(Digest32::of_bytes(b"manager")),
        };
        let original = compile(origins)?;
        assert!(original.summary.len() <= 1024);
        assert_eq!(
            ProductionLayout::from_plan(&original)?
                .inventory_minima()
                .len(),
            4
        );
        assert!(stock_targets_binding::requirement(&original)?.is_some());
        for retained in [
            Origins::default(),
            Origins::from(origins.workshop),
            Origins {
                workshop: None,
                manager: origins.manager,
            },
        ] {
            let mut stripped = original.clone();
            stripped.summary = prerequisite_binding::summary(
                prerequisite_binding::body(&original)?.to_owned(),
                retained,
            )?;
            stripped.digest = stripped.compute_digest();
            stripped.id = stripped.expected_id();
            assert!(stock_targets_binding::requirement(&stripped).is_err());
        }
    }
    Ok(())
}
