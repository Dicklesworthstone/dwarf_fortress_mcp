#![allow(dead_code, unused_imports)]
include!("../semantic_workforce_runtime/fixtures.rs");
use super::{StockTargetBinding, StockTargetRequest};
use crate::semantic_work_order_runtime::{
    inventory_baseline::InventoryBaselines, request::Objective,
};
use dfmcp_adapter::live_jobs::LiveJobObservation;
use dfmcp_adapter::live_operations::{LiveItem, LiveOperationsObservation};
use dfmcp_adapter::live_production::{ExistingLaborRequirement, ProductionLayout};
use dfmcp_adapter::semantic_work_orders::{
    InventoryBaselineProjection, ObservedInventoryIncreaseObjective, StockTargetAssessment,
};
use dfmcp_adapter::work_orders::WorkOrderRecipe;
use dfmcp_core::MapCoord;

pub(crate) fn stock_fixture() -> Result<(LiveWorldProjection, OperationContext)> {
    let (_, projection) = observation(&[42, 43])?;
    let mut c = context(&projection.snapshot);
    for capability in [Capability::Observe, Capability::ConfigureProduction] {
        c.grants.push(CapabilityGrant {
            capability,
            scope: CapabilityScope {
                fortress_id: Some(c.anchor.fortress_id),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::Guarded,
            expires_at_tick: None,
            remaining_uses: None,
        });
    }
    Ok((projection, c))
}

pub(crate) fn stock_policy(snapshot: &WorldSnapshot) -> EvidencePolicy {
    policy(snapshot)
}

pub(crate) fn stock_request(target: u64, maximum: u32) -> Result<StockTargetRequest> {
    StockTargetRequest::parse(
        &serde_json::json!({
            "kind":"stock_target", "expected_observation":"11".repeat(32),
            "units":["44","43"], "labor":"CARPENTER", "recipe":"wooden_bed",
            "minimum_stack_units":target, "max_order_amount":maximum,
            "deadline_tick":300, "poll_interval_ticks":5, "stable_samples":2,
        })
        .to_string(),
    )
}

pub(crate) fn stock_workers() -> Result<ExistingLaborRequirement> {
    ExistingLaborRequirement::new(
        "CARPENTER".into(),
        vec![(EntityId::new(43), 142), (EntityId::new(44), 143)],
    )
}

pub(crate) fn stock_assessment(
    c: &OperationContext,
    target: u64,
    maximum: u32,
    baseline: u32,
    unrelated: u32,
) -> Result<StockTargetAssessment> {
    let items = [(1, "BED", baseline), (2, "DOOR", unrelated)]
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
    let mut accepted = EvidencePolicy::at(projection.snapshot().anchor());
    accepted.paused = true;
    accepted
        .entity_kinds
        .insert(EntityKind::Item, EvidenceCoverage::Complete);
    accepted.sources = projection.observed_sources();
    projection.stock_target(WorkOrderRecipe::WoodenBed, target, maximum, accepted, c)
}

#[test]
fn stock_target_closed_request_accepts_only_intent_and_observed_worker_selection() -> Result<()> {
    let base = serde_json::json!({
        "kind":"stock_target", "expected_observation":"11".repeat(32),
        "units":["44","43"], "labor":"CARPENTER", "recipe":"wooden_bed",
        "minimum_stack_units":7, "deadline_tick":300,
    });
    let parsed = StockTargetRequest::parse(&base.to_string())?;
    assert!(!parsed.assign_labor);
    assert_eq!(parsed.max_order_amount, 100);
    assert_eq!(parsed.poll_interval_ticks, 10);
    assert_eq!(parsed.stable_samples, 1);
    assert_eq!(parsed.unit_ids()?, [EntityId::new(43), EntityId::new(44)]);
    assert_eq!(
        parsed.observation()?,
        crate::semantic_workforce_runtime::digest(&"11".repeat(32))?
    );
    for (field, value) in [
        ("kind", serde_json::json!("existing_labor")),
        ("minimum_stack_units", serde_json::json!(0)),
        ("minimum_stack_units", serde_json::json!(1_000_001)),
        ("minimum_stack_units", serde_json::json!(-1)),
        ("minimum_stack_units", serde_json::json!(1.5)),
        ("max_order_amount", serde_json::json!(0)),
        ("max_order_amount", serde_json::json!(101)),
        ("max_order_amount", serde_json::json!(null)),
        ("assign_labor", serde_json::json!(null)),
        ("assign_labor", serde_json::json!("true")),
        ("assign_labor", serde_json::json!(1)),
        ("recipe", serde_json::json!("wooden_stool")),
        ("units", serde_json::json!(["43", "43"])),
        ("units", serde_json::json!(["043"])),
        ("labor", serde_json::json!("")),
        ("poll_interval_ticks", serde_json::json!(0)),
        ("stable_samples", serde_json::json!(65)),
    ] {
        let mut changed = base.clone();
        changed[field] = value;
        assert!(
            StockTargetRequest::parse(&changed.to_string()).is_err(),
            "{field}"
        );
    }
    for field in [
        "amount",
        "enabled",
        "objective",
        "baseline_stack_units",
        "target_stack_units",
        "assessment_witness",
        "source_digest",
        "historical_figure_id",
        "orders",
        "repeat",
    ] {
        let mut changed = base.clone();
        changed[field] = serde_json::json!(1);
        assert!(
            StockTargetRequest::parse(&changed.to_string()).is_err(),
            "{field}"
        );
    }
    for recipe in ["wooden_bed", "wooden_door", "wooden_table", "wooden_chair"] {
        let mut supported = base.clone();
        supported["recipe"] = recipe.into();
        supported["minimum_stack_units"] = 1_000_000.into();
        StockTargetRequest::parse(&supported.to_string())?;
    }
    assert!(StockTargetRequest::parse(&" ".repeat(4097)).is_err());
    Ok(())
}

#[test]
fn stock_target_compiles_exact_deficit_with_existing_workers_and_sealed_growth() -> Result<()> {
    let (projection, c) = stock_fixture()?;
    let evidence =
        PredicateEvidence::scoped(&projection.snapshot, stock_policy(&projection.snapshot))?;
    let request = stock_request(7, 100)?;
    let assessment = stock_assessment(&c, 7, 100, 5, 99)?;
    let workers = stock_workers()?;
    let plan = request.compile(&assessment, &evidence, &workers, &c, None)?;
    let layout = ProductionLayout::from_plan(&plan)?;
    assert_eq!(layout.assignment_step(), None);
    assert_eq!(layout.order_steps(), [StepId::ZERO]);
    assert_eq!(layout.existing_labor(), Some(&workers));
    assert_eq!(plan.steps.len(), 1);
    assert!(matches!(&plan.steps[0].action, Action::CreateWorkOrder {
        amount:2, job_token, conditions, ..
    } if job_token == "CONSTRUCT_BED" && conditions.is_empty()));
    assert!(
        !plan
            .required_capabilities
            .contains(&Capability::ConfigureLabor)
    );
    let goal = ObservedInventoryIncreaseObjective::from_selected_plan(&plan, StepId::ZERO)
        .ok_or_else(|| invalid("stock target objective missing"))?;
    assert_eq!(
        goal,
        assessment.objective().expect("positive deficit").clone()
    );
    assert_eq!(goal.baseline_stack_units(), 5);
    assert_eq!(goal.increase_stack_units(), 2);
    assert_eq!(goal.target_stack_units(), 7);
    assert_eq!(goal.baseline_anchor(), plan.anchor);
    let obligation = plan.steps[0]
        .obligation
        .as_ref()
        .expect("finite stock obligation");
    assert_eq!(obligation.deadline_tick, GameTick(300));
    assert_eq!(obligation.poll_interval_ticks, 5);
    assert_eq!(obligation.stable_for_observations, 2);
    let binding = super::super::stock_target_binding::requirement(&plan)?
        .ok_or_else(|| invalid("stock target binding missing"))?;
    assert_eq!(binding.objective(), &goal);
    assert_eq!(binding.witness(), assessment.witness());
    assert_eq!(binding.max_order_amount(), 100);
    assert_eq!(binding.recipe(), WorkOrderRecipe::WoodenBed);
    Ok(())
}

#[test]
fn stock_target_identity_separates_manual_growth_bounds_census_and_workshop() -> Result<()> {
    use crate::semantic_work_order_runtime::{
        inventory_baseline::InventoryBaselines, request::Objective,
    };
    let (projection, c) = stock_fixture()?;
    let evidence =
        PredicateEvidence::scoped(&projection.snapshot, stock_policy(&projection.snapshot))?;
    let workers = stock_workers()?;
    let request = stock_request(7, 100)?;
    let assessment = stock_assessment(&c, 7, 100, 5, 99)?;
    let original = request.compile(&assessment, &evidence, &workers, &c, None)?;
    let baseline = assessment.objective().expect("positive deficit").clone();
    let baselines = InventoryBaselines::from_qualified(baseline.clone());
    let manual = request.existing(
        2,
        Some(Objective::ObservedInventoryIncrease {
            item_type: "BED".into(),
            increase_stack_units: 2,
        }),
    );
    let legacy = manual.compile_with_inventory(&evidence, &workers, &baselines, &c)?;
    assert_eq!(
        legacy,
        manual.compile_with_stock_target(&evidence, &workers, &baselines, &c, None, None)?
    );
    assert_eq!(legacy.steps[0].action, original.steps[0].action);
    assert_eq!(legacy.steps[0].obligation, original.steps[0].obligation);
    assert_eq!(
        super::super::stock_target_binding::requirement(&legacy)?,
        None
    );
    let changed_bound = stock_request(7, 99)?.compile(
        &stock_assessment(&c, 7, 99, 5, 99)?,
        &evidence,
        &workers,
        &c,
        None,
    )?;
    let changed_census = request.compile(
        &stock_assessment(&c, 7, 100, 5, 100)?,
        &evidence,
        &workers,
        &c,
        None,
    )?;
    let changed_target = stock_request(8, 100)?.compile(
        &stock_assessment(&c, 8, 100, 6, 99)?,
        &evidence,
        &workers,
        &c,
        None,
    )?;
    let workshop = request.compile(
        &assessment,
        &evidence,
        &workers,
        &c,
        Some(Digest32::of_bytes(b"completed original workshop")),
    )?;
    let mut later_request = request.clone();
    later_request.deadline_tick += 1;
    let changed_deadline = later_request.compile(&assessment, &evidence, &workers, &c, None)?;
    let mut cadence_request = request.clone();
    cadence_request.poll_interval_ticks += 1;
    let changed_cadence = cadence_request.compile(&assessment, &evidence, &workers, &c, None)?;
    let mut stability_request = request.clone();
    stability_request.stable_samples += 1;
    let changed_stability =
        stability_request.compile(&assessment, &evidence, &workers, &c, None)?;
    for changed in [
        &legacy,
        &changed_bound,
        &changed_census,
        &changed_target,
        &workshop,
        &changed_deadline,
        &changed_cadence,
        &changed_stability,
    ] {
        assert_ne!(changed.intent_id, original.intent_id);
        assert_ne!(changed.id, original.id);
        assert_ne!(changed.digest, original.digest);
        assert_ne!(
            changed.steps[0].idempotency_key,
            original.steps[0].idempotency_key
        );
        assert_eq!(
            changed.steps[0].idempotency_key,
            derive_step_idempotency_key(
                changed.intent_id,
                changed.anchor,
                StepId::ZERO,
                &changed.steps[0].action,
            )
        );
    }
    let mut reordered = request;
    reordered.units.reverse();
    assert_eq!(
        original,
        reordered.compile(&assessment, &evidence, &workers, &c, None)?
    );
    Ok(())
}

#[test]
fn stock_target_requires_positive_matching_assessment_and_original_evidence() -> Result<()> {
    let (projection, c) = stock_fixture()?;
    let evidence =
        PredicateEvidence::scoped(&projection.snapshot, stock_policy(&projection.snapshot))?;
    let workers = stock_workers()?;
    let request = stock_request(7, 100)?;
    let assessment = stock_assessment(&c, 7, 100, 5, 99)?;
    for satisfied in [7, 8] {
        assert!(
            request
                .compile(
                    &stock_assessment(&c, 7, 100, satisfied, 99)?,
                    &evidence,
                    &workers,
                    &c,
                    None
                )
                .is_err()
        );
    }
    for field in 0..5 {
        let mut changed = request.clone();
        match field {
            0 => changed.recipe = "wooden_door".into(),
            1 => changed.minimum_stack_units = 8,
            2 => changed.max_order_amount = 99,
            3 => changed.deadline_tick = 129,
            _ => changed.units = vec!["43".into()],
        }
        assert!(
            changed
                .compile(&assessment, &evidence, &workers, &c, None)
                .is_err(),
            "case {field}"
        );
    }
    let mut stale = c.clone();
    stale.anchor.cursor.sequence += 1;
    assert!(
        request
            .compile(&assessment, &evidence, &workers, &stale, None)
            .is_err()
    );
    assert!(
        request
            .compile(
                &assessment,
                &PredicateEvidence::untrusted(&projection.snapshot)?,
                &workers,
                &c,
                None
            )
            .is_err()
    );
    assert!(stock_assessment(&c, 106, 100, 5, 99).is_err());
    Ok(())
}

#[test]
fn stock_target_archive_retains_target_and_outer_original_workshop_requirement() -> Result<()> {
    let (projection, c) = stock_fixture()?;
    let evidence =
        PredicateEvidence::scoped(&projection.snapshot, stock_policy(&projection.snapshot))?;
    let assessment = stock_assessment(&c, 7, 100, 5, 99)?;
    let origin = Digest32::of_bytes(b"one exact acknowledged Carpenter workshop");
    for workshop in [None, Some(origin)] {
        let original = stock_request(7, 100)?.compile(
            &assessment,
            &evidence,
            &stock_workers()?,
            &c,
            workshop,
        )?;
        let limits = dfmcp_adapter::live_production::archive_limits();
        let (bytes, seal) = dfmcp_intent::plan_archive::encode(&original, limits)?;
        let recovered = dfmcp_intent::plan_archive::decode(&bytes, seal, limits)?;
        assert_eq!(recovered, original);
        assert_eq!(
            super::super::workshop_binding::requirement(&recovered)?,
            workshop
        );
        let binding = super::super::stock_target_binding::requirement(&recovered)?
            .ok_or_else(|| invalid("recovered stock target missing"))?;
        assert_eq!(binding.witness(), assessment.witness());
        assert_eq!(
            binding.objective(),
            assessment.objective().expect("positive deficit")
        );
        let lines: Vec<_> = recovered.summary.lines().collect();
        assert!(lines[usize::from(workshop.is_some())].starts_with("Stock target/1 "));
        assert!(original.summary.len() <= 256);
        let mut dropped = recovered.clone();
        dropped.summary = "Original manual growth goal".into();
        assert!(super::super::stock_target_binding::requirement(&dropped).is_err());
        assert!(
            dfmcp_intent::plan_archive::decode(
                &bytes,
                dfmcp_intent::plan_archive::PlanArchiveSeal {
                    plan_digest: dropped.compute_digest(),
                    ..seal
                },
                limits
            )
            .is_err()
        );
    }
    let maximum = stock_assessment(&c, 1_000_000, 100, 999_998, 99)?;
    let bounded = stock_request(1_000_000, 100)?.compile(
        &maximum,
        &evidence,
        &stock_workers()?,
        &c,
        Some(origin),
    )?;
    assert!(
        bounded.summary.len() <= 256,
        "Workshop plus maximum target stays inside the unchanged planner limit"
    );
    assert_eq!(
        super::super::stock_target_binding::requirement(&bounded)?
            .expect("bounded target")
            .objective()
            .target_stack_units(),
        1_000_000
    );
    Ok(())
}

#[test]
fn assigned_stock_target_seals_enable_then_exact_deficit_with_original_growth() -> Result<()> {
    let (projection, mut c) = stock_fixture()?;
    c.budget.max_actions = 2;
    let evidence =
        PredicateEvidence::scoped(&projection.snapshot, stock_policy(&projection.snapshot))?;
    let assessment = stock_assessment(&c, 7, 100, 5, 99)?;
    let mut request = stock_request(7, 100)?;
    request.assign_labor = true;
    let plan = request.compile_with_labor_assignment(&assessment, &evidence, &c, None)?;
    let layout = ProductionLayout::from_plan(&plan)?;
    assert_eq!(plan.steps.len(), 2);
    assert_eq!(layout.assignment_step(), Some(StepId::ZERO));
    assert_eq!(layout.order_steps(), [StepId::new(1)]);
    assert_eq!(layout.existing_labor(), None);
    assert!(matches!(&plan.steps[0].action, Action::SetLabor {
        units, labor, enabled: true
    } if units == &[EntityId::new(43), EntityId::new(44)] && labor == "CARPENTER"));
    assert!(plan.steps[0].obligation.is_none());
    assert!(plan.steps[0].depends_on.is_empty());
    assert_eq!(plan.steps[1].depends_on, [StepId::ZERO]);
    assert!(matches!(&plan.steps[1].action, Action::CreateWorkOrder {
        name, amount: 2, job_token, conditions
    } if name == "native stock-target/2 wooden_bed goal"
        && job_token == "CONSTRUCT_BED" && conditions.is_empty()));
    assert!(
        plan.required_capabilities
            .contains(&Capability::ConfigureLabor)
    );
    assert!(
        plan.required_capabilities
            .contains(&Capability::ConfigureProduction)
    );
    let goal = ObservedInventoryIncreaseObjective::from_selected_plan(&plan, StepId::new(1))
        .ok_or_else(|| invalid("assigned stock target lost its original growth objective"))?;
    assert_eq!(&goal, assessment.objective().expect("positive deficit"));
    assert_eq!(goal.target_stack_units(), 7);
    assert_eq!(goal.baseline_stack_units(), 5);
    let obligation = plan.steps[1]
        .obligation
        .as_ref()
        .expect("original obligation");
    assert_eq!(obligation.deadline_tick, GameTick(300));
    assert_eq!(obligation.poll_interval_ticks, 5);
    assert_eq!(obligation.stable_for_observations, 2);
    assert!(obligation.failure.is_none());
    let binding = super::super::stock_target_binding::requirement(&plan)?
        .ok_or_else(|| invalid("assigned stock target binding missing"))?;
    assert!(binding.assign_labor());
    assert_eq!(binding.witness(), assessment.witness());
    assert_eq!(binding.objective(), &goal);
    assert!(plan.summary.starts_with("Stock target/2 "));
    for step in &plan.steps {
        assert_eq!(
            step.idempotency_key,
            derive_step_idempotency_key(plan.intent_id, plan.anchor, step.id, &step.action)
        );
        assert!(step.compensation.is_none());
    }
    let limits = dfmcp_adapter::live_production::archive_limits();
    let (bytes, seal) = dfmcp_intent::plan_archive::encode(&plan, limits)?;
    let recovered = dfmcp_intent::plan_archive::decode(&bytes, seal, limits)?;
    assert_eq!(recovered, plan);
    assert_eq!(
        super::super::stock_target_binding::requirement(&recovered)?,
        Some(binding)
    );
    Ok(())
}

#[test]
fn stock_assignment_is_explicit_and_satisfied_targets_have_no_compilable_actions() -> Result<()> {
    let (projection, mut c) = stock_fixture()?;
    c.budget.max_actions = 2;
    let evidence =
        PredicateEvidence::scoped(&projection.snapshot, stock_policy(&projection.snapshot))?;
    let assessment = stock_assessment(&c, 7, 100, 5, 99)?;
    let request = stock_request(7, 100)?;
    assert!(
        request
            .compile_with_labor_assignment(&assessment, &evidence, &c, None)
            .is_err()
    );
    let mut assigned = request.clone();
    assigned.assign_labor = true;
    assert!(
        assigned
            .compile(&assessment, &evidence, &stock_workers()?, &c, None)
            .is_err()
    );
    for baseline in [7, 8] {
        let met = stock_assessment(&c, 7, 100, baseline, 99)?;
        assert_eq!(met.amount(), None);
        assert!(
            assigned
                .compile_with_labor_assignment(&met, &evidence, &c, None)
                .is_err()
        );
    }
    let mut insufficient = c.clone();
    insufficient.budget.max_actions = 1;
    assert!(
        assigned
            .compile_with_labor_assignment(&assessment, &evidence, &insufficient, None)
            .is_err()
    );
    assert!(
        assigned
            .compile_with_labor_assignment(
                &assessment,
                &PredicateEvidence::untrusted(&projection.snapshot)?,
                &c,
                None,
            )
            .is_err()
    );
    let mut stale = c.clone();
    stale.anchor.cursor.sequence += 1;
    assert!(
        assigned
            .compile_with_labor_assignment(&assessment, &evidence, &stale, None)
            .is_err()
    );
    let explicit = serde_json::json!({
        "kind":"stock_target", "expected_observation":"11".repeat(32),
        "units":["44","43"], "labor":"CARPENTER", "recipe":"wooden_bed",
        "minimum_stack_units":7, "max_order_amount":100, "deadline_tick":300,
        "poll_interval_ticks":5, "stable_samples":2, "assign_labor":true,
    });
    assert_eq!(StockTargetRequest::parse(&explicit.to_string())?, assigned);
    let mut disabled = explicit;
    disabled["enabled"] = false.into();
    assert!(StockTargetRequest::parse(&disabled.to_string()).is_err());
    Ok(())
}

#[test]
fn assigned_stock_identity_preserves_legacy_and_binds_selection_census_and_timing() -> Result<()> {
    let (projection, mut c) = stock_fixture()?;
    c.budget.max_actions = 2;
    let evidence =
        PredicateEvidence::scoped(&projection.snapshot, stock_policy(&projection.snapshot))?;
    let assessment = stock_assessment(&c, 7, 100, 5, 99)?;
    let request = stock_request(7, 100)?;
    let legacy = request.compile(&assessment, &evidence, &stock_workers()?, &c, None)?;
    let mut assigned = request.clone();
    assigned.assign_labor = true;
    let original = assigned.compile_with_labor_assignment(&assessment, &evidence, &c, None)?;
    let baselines =
        InventoryBaselines::from_qualified(assessment.objective().expect("baseline").clone());
    let manual = super::super::request::Request {
        expected_observation: assigned.expected_observation.clone(),
        units: assigned.units.clone(),
        labor: assigned.labor.clone(),
        enabled: true,
        recipe: assigned.recipe.clone(),
        amount: 2,
        deadline_tick: assigned.deadline_tick,
        poll_interval_ticks: assigned.poll_interval_ticks,
        stable_samples: assigned.stable_samples,
        objective: Some(Objective::ObservedInventoryIncrease {
            item_type: "BED".into(),
            increase_stack_units: 2,
        }),
    };
    let manual_plan = manual.compile_with_inventory(&evidence, &baselines, &c)?;
    assert_eq!(
        manual_plan,
        manual.compile_with_stock_target(&evidence, &baselines, &c, None, None)?
    );
    assert_eq!(
        super::super::stock_target_binding::requirement(&manual_plan)?,
        None
    );
    assert_eq!(manual_plan.steps[0].action, original.steps[0].action);
    assert_eq!(
        manual_plan.steps[1].obligation,
        original.steps[1].obligation
    );
    assert_ne!(manual_plan.intent_id, original.intent_id);
    assert_ne!(legacy.intent_id, original.intent_id);
    assert!(legacy.summary.starts_with("Stock target/1 "));
    assert!(
        !super::super::stock_target_binding::requirement(&legacy)?
            .expect("legacy binding")
            .assign_labor()
    );
    assert_eq!(
        legacy,
        request.compile(&assessment, &evidence, &stock_workers()?, &c, None)?
    );
    let mut reordered = assigned.clone();
    reordered.units.reverse();
    assert_eq!(
        original,
        reordered.compile_with_labor_assignment(&assessment, &evidence, &c, None)?
    );
    for case in 0..8 {
        let mut changed = assigned.clone();
        let mut current_assessment = assessment.clone();
        let mut origin = None;
        match case {
            0 => changed.units = vec!["43".into()],
            1 => changed.labor = "MINE".into(),
            2 => changed.deadline_tick += 1,
            3 => changed.poll_interval_ticks += 1,
            4 => changed.stable_samples += 1,
            5 => {
                changed.max_order_amount = 99;
                current_assessment = stock_assessment(&c, 7, 99, 5, 99)?;
            }
            6 => current_assessment = stock_assessment(&c, 7, 100, 5, 100)?,
            _ => origin = Some(Digest32::of_bytes(b"completed original Carpenter")),
        }
        let other =
            changed.compile_with_labor_assignment(&current_assessment, &evidence, &c, origin)?;
        assert_ne!(other.intent_id, original.intent_id, "case {case}");
        assert_ne!(other.digest, original.digest, "case {case}");
        for index in 0..2 {
            assert_ne!(
                other.steps[index].idempotency_key, original.steps[index].idempotency_key,
                "case {case}, step {index}"
            );
        }
        let limits = dfmcp_adapter::live_production::archive_limits();
        let (bytes, seal) = dfmcp_intent::plan_archive::encode(&other, limits)?;
        let recovered = dfmcp_intent::plan_archive::decode(&bytes, seal, limits)?;
        assert_eq!(recovered, other);
        assert_eq!(
            super::super::workshop_binding::requirement(&recovered)?,
            origin
        );
        assert!(
            super::super::stock_target_binding::requirement(&recovered)?
                .expect("assigned binding")
                .assign_labor()
        );
        assert!(recovered.summary.len() <= 256);
    }
    let binding = StockTargetBinding::from_assessment_with_assignment(&assessment)?;
    let mut disabled = manual;
    disabled.enabled = false;
    assert!(
        disabled
            .compile_with_stock_target(&evidence, &baselines, &c, None, Some(&binding))
            .is_err()
    );
    Ok(())
}

#[test]
fn manager_and_workshop_stock_target_preserves_original_sizing_and_keys_on_recovery() -> Result<()>
{
    use super::super::prerequisite_binding::{self, Origins};
    let (projection, mut c) = stock_fixture()?;
    c.budget.max_actions = 2;
    let evidence =
        PredicateEvidence::scoped(&projection.snapshot, stock_policy(&projection.snapshot))?;
    let assessment = stock_assessment(&c, 7, 100, 5, 99)?;
    for assignment in [false, true] {
        let mut request = stock_request(7, 100)?;
        request.assign_labor = assignment;
        let compile = |origins: Origins| {
            if assignment {
                request.compile_with_labor_assignment(&assessment, &evidence, &c, origins)
            } else {
                request.compile(&assessment, &evidence, &stock_workers()?, &c, origins)
            }
        };
        let legacy = compile(Origins::default())?;
        prerequisite_binding::assert_manager_variants(&legacy, compile)?;
        let origins = Origins {
            workshop: Some(Digest32::of_bytes(b"workshop")),
            manager: Some(Digest32::of_bytes(b"manager")),
        };
        let original = compile(origins)?;
        let bound =
            super::super::stock_target_binding::requirement(&original)?.expect("exact target");
        assert_eq!(bound.witness(), assessment.witness());
        assert_eq!(bound.assign_labor(), assignment);
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
            assert!(super::super::stock_target_binding::requirement(&stripped).is_err());
        }
    }
    Ok(())
}
