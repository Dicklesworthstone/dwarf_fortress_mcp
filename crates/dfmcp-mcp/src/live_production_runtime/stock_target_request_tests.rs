#![allow(dead_code, unused_imports)]
include!("../semantic_workforce_runtime/fixtures.rs");
use super::StockTargetRequest;
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
