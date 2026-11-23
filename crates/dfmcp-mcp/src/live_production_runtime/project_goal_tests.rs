//! Original scalar compatibility and complete portfolio reconstruction using
//! the real qualified stock census and production compilers.
use super::super::stock_target_request::tests::{stock_fixture, stock_policy, stock_workers};
use super::super::stock_targets_request::{StockTarget, StockTargetsRequest};
use super::*;
use dfmcp_adapter::live_jobs::LiveJobObservation;
use dfmcp_adapter::live_operations::{LiveItem, LiveOperationsObservation};
use dfmcp_adapter::semantic_work_orders::{InventoryBaselineProjection, StockTargetAssessment};
use dfmcp_core::{MapCoord, OperationContext, StepId};
use dfmcp_world::{EntityKind, EvidenceCoverage, EvidencePolicy, PredicateEvidence};
use serde_json::{Value, json};

fn portfolio(assigned: bool) -> ProjectStockGoal {
    ProjectStockGoal::Portfolio(ProjectStockTargetsGoal {
        targets: vec![
            StockTarget {
                recipe: "wooden_bed".into(),
                minimum_stack_units: 6,
                max_order_amount: 100,
            },
            StockTarget {
                recipe: "wooden_door".into(),
                minimum_stack_units: 3,
                max_order_amount: 20,
            },
        ],
        deadline_tick: 300,
        poll_interval_ticks: 5,
        stable_samples: 2,
        assign_labor: assigned,
    })
}

fn request(goal: &ProjectStockGoal) -> Result<StockTargetsRequest> {
    let mut value = serde_json::to_value(goal).map_err(|_| invalid("test goal encoding"))?;
    value["kind"] = json!("stock_targets");
    value["expected_observation"] = json!("11".repeat(32));
    value["units"] = json!(["44", "43"]);
    value["labor"] = json!("CARPENTER");
    StockTargetsRequest::parse(&value.to_string())
}

fn assessments(
    request: &StockTargetsRequest,
    c: &OperationContext,
    counts: [u32; 4],
) -> Result<Vec<StockTargetAssessment>> {
    let items = ["BED", "CHAIR", "DOOR", "TABLE"]
        .into_iter()
        .zip(counts)
        .enumerate()
        .filter(|(_, (_, count))| *count > 0)
        .map(|(index, (kind, stack_size))| LiveItem {
            native_id: index as u32 + 1,
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
                super::request::recipe(&target.recipe)?.0,
                target.minimum_stack_units,
                target.max_order_amount,
                policy.clone(),
                c,
            )
        })
        .collect()
}

#[test]
fn project_scalar_json_retains_legacy_bytes_without_variant_tags() -> Result<()> {
    let raw = r#"{"recipe":"wooden_bed","minimum_stack_units":6,"max_order_amount":100,"deadline_tick":300,"poll_interval_ticks":5,"stable_samples":2,"assign_labor":false}"#;
    let mut goal: ProjectStockGoal =
        serde_json::from_str(raw).map_err(|_| invalid("legacy goal decoding"))?;
    assert!(!goal.is_portfolio());
    goal.canonicalize()?;
    assert_eq!(
        serde_json::to_string(&goal).map_err(|_| invalid("legacy encoding"))?,
        raw
    );
    for unknown in ["targets", "kind", "native_clock", "amount"] {
        let mut value: Value = serde_json::from_str(raw).map_err(|_| invalid("legacy value"))?;
        value[unknown] = json!([]);
        assert!(serde_json::from_value::<ProjectStockGoal>(value).is_err());
    }
    let duplicate = raw.replacen("{", "{\"recipe\":\"wooden_door\",", 1);
    assert!(serde_json::from_str::<ProjectStockGoal>(&duplicate).is_err());
    Ok(())
}

#[test]
fn project_portfolio_is_closed_canonical_and_bounded_before_io() -> Result<()> {
    let original = portfolio(false);
    let value = serde_json::to_value(&original).map_err(|_| invalid("portfolio encoding"))?;
    for (field, invalid_value) in [
        ("targets", json!([])),
        ("targets", json!(vec![value["targets"][0].clone(); 5])),
        ("recipe", json!("wooden_bed")),
        ("minimum_stack_units", json!(6)),
        ("max_order_amount", json!(100)),
        ("deadline_tick", json!(0)),
        ("deadline_tick", json!(u64::MAX)),
        ("poll_interval_ticks", json!(0)),
        ("poll_interval_ticks", json!(u64::MAX)),
        ("stable_samples", json!(65)),
    ] {
        let mut changed = value.clone();
        changed[field] = invalid_value;
        assert!(
            serde_json::from_value::<ProjectStockGoal>(changed)
                .map_or(true, |goal| goal.validate().is_err()),
            "{field}"
        );
    }
    for (field, invalid_value) in [
        ("recipe", json!("smelt_ore")),
        ("minimum_stack_units", json!(0)),
        ("minimum_stack_units", json!(1_000_001)),
        ("max_order_amount", json!(0)),
        ("max_order_amount", json!(101)),
        ("amount", json!(1)),
    ] {
        let mut changed = value.clone();
        changed["targets"][0][field] = invalid_value;
        assert!(
            serde_json::from_value::<ProjectStockGoal>(changed)
                .map_or(true, |goal| goal.validate().is_err()),
            "{field}"
        );
    }
    let mut changed = original.clone();
    let ProjectStockGoal::Portfolio(goal) = &mut changed else {
        return Err(invalid("test portfolio variant"));
    };
    goal.targets.reverse();
    assert!(
        changed.validate().is_err(),
        "retained target order is part of identity"
    );
    changed.canonicalize()?;
    assert_eq!(changed, original);
    let mut duplicate = value;
    duplicate["targets"][1] = duplicate["targets"][0].clone();
    assert!(
        serde_json::from_value::<ProjectStockGoal>(duplicate)
            .map_or(true, |mut goal| goal.canonicalize().is_err())
    );
    Ok(())
}

#[test]
fn project_portfolio_request_retains_every_target_and_shared_temporal_term() -> Result<()> {
    for assigned in [false, true] {
        let goal = portfolio(assigned);
        let original = request(&goal)?;
        goal.validate_targets_request(&original)?;
        let edits: [fn(&mut StockTargetsRequest); 9] = [
            |value| {
                value.targets.pop();
            },
            |value| value.targets[1].minimum_stack_units += 1,
            |value| value.targets[1].max_order_amount += 1,
            |value| value.targets[1].recipe = "wooden_table".into(),
            |value| value.deadline_tick += 1,
            |value| value.poll_interval_ticks += 1,
            |value| value.stable_samples += 1,
            |value| value.assign_labor = !value.assign_labor,
            |value| value.labor = "MINE".into(),
        ];
        for edit in edits {
            let mut changed = original.clone();
            edit(&mut changed);
            assert!(goal.validate_targets_request(&changed).is_err());
        }
        let mut fresh_choice = original;
        fresh_choice.units = vec!["43".into()];
        fresh_choice.expected_observation = Digest32::of_bytes(b"new published choice").to_hex();
        goal.validate_targets_request(&fresh_choice)?;
        let scalar = super::super::stock_target_request::tests::stock_request(6, 100)?;
        assert!(goal.validate_request(&scalar).is_err());
    }
    Ok(())
}

#[test]
fn project_mixed_portfolio_reconstruction_keeps_met_minima_and_native_keys() -> Result<()> {
    let (projection, mut c) = stock_fixture()?;
    c.budget.max_actions = 5;
    let evidence =
        PredicateEvidence::scoped(&projection.snapshot, stock_policy(&projection.snapshot))?;
    let origin = Digest32::of_bytes(b"project portfolio original Workshop");
    for assigned in [false, true] {
        let goal = portfolio(assigned);
        let request = request(&goal)?;
        let values = assessments(&request, &c, [2, 0, 3, 0])?;
        let plan = if assigned {
            request.compile_with_labor_assignment(&values, &evidence, &c, Some(origin))?
        } else {
            request.compile(&values, &evidence, &stock_workers()?, &c, Some(origin))?
        };
        let layout = ProductionLayout::from_plan(&plan)?;
        assert_eq!(layout.order_steps(), [StepId::new(u32::from(assigned))]);
        assert_eq!(layout.inventory_minima().len(), 2);
        let limits = dfmcp_adapter::live_production::archive_limits();
        let (bytes, seal) = dfmcp_intent::plan_archive::encode(&plan, limits)?;
        let recovered = dfmcp_intent::plan_archive::decode(&bytes, seal, limits)?;
        goal.validate_original(&recovered, origin)?;
        assert_eq!(plan, recovered);
        for changed_goal in [
            {
                let mut v = serde_json::to_value(&goal).map_err(|_| invalid("goal"))?;
                v["targets"][1]["minimum_stack_units"] = json!(2);
                v
            },
            {
                let mut v = serde_json::to_value(&goal).map_err(|_| invalid("goal"))?;
                v["targets"][1]["max_order_amount"] = json!(19);
                v
            },
            {
                let mut v = serde_json::to_value(&goal).map_err(|_| invalid("goal"))?;
                v["deadline_tick"] = json!(301);
                v
            },
            {
                let mut v = serde_json::to_value(&goal).map_err(|_| invalid("goal"))?;
                v["assign_labor"] = json!(!assigned);
                v
            },
        ] {
            let changed: ProjectStockGoal =
                serde_json::from_value(changed_goal).map_err(|_| invalid("changed goal"))?;
            assert!(changed.validate_original(&recovered, origin).is_err());
        }
        assert!(goal.validate_original(&recovered, Digest32::ZERO).is_err());
        assert!(
            goal.validate_original(&recovered, Digest32::of_bytes(b"other donor"))
                .is_err()
        );
        let mut changed = recovered;
        changed
            .steps
            .last_mut()
            .ok_or_else(|| invalid("order"))?
            .idempotency_key = "substituted-key".into();
        changed.digest = changed.compute_digest();
        changed.id = changed.expected_id();
        assert!(goal.validate_original(&changed, origin).is_err());
    }
    Ok(())
}

#[test]
fn project_four_target_assignment_fits_existing_five_action_budget() -> Result<()> {
    let (projection, mut c) = stock_fixture()?;
    c.budget.max_actions = 5;
    let evidence =
        PredicateEvidence::scoped(&projection.snapshot, stock_policy(&projection.snapshot))?;
    let mut goal = portfolio(true);
    let ProjectStockGoal::Portfolio(value) = &mut goal else {
        return Err(invalid("test portfolio"));
    };
    for recipe in ["wooden_chair", "wooden_table"] {
        value.targets.push(StockTarget {
            recipe: recipe.into(),
            minimum_stack_units: 1,
            max_order_amount: 100,
        });
    }
    goal.canonicalize()?;
    let request = request(&goal)?;
    let values = assessments(&request, &c, [0, 0, 0, 0])?;
    let origin = Digest32::of_bytes(b"four-target Workshop");
    let plan = request.compile_with_labor_assignment(&values, &evidence, &c, Some(origin))?;
    goal.validate_original(&plan, origin)?;
    assert_eq!(plan.steps.len(), 5);
    assert_eq!(
        ProductionLayout::from_plan(&plan)?.inventory_minima().len(),
        4
    );
    c.budget.max_actions = 4;
    assert!(
        request
            .compile_with_labor_assignment(&values, &evidence, &c, Some(origin))
            .is_err()
    );
    Ok(())
}

#[test]
fn all_met_project_portfolio_keeps_complete_watch_without_compiling_effects() -> Result<()> {
    let (_, c) = stock_fixture()?;
    let goal = portfolio(true);
    let request = request(&goal)?;
    let values = assessments(&request, &c, [6, 0, 3, 0])?;
    assert!(values.iter().all(|value| value.amount().is_none()));
    let binding =
        super::super::stock_targets_binding::StockTargetsBinding::from_assessments_with_assignment(
            &values,
        )?;
    let watch =
        super::super::stock_targets_watch::StockTargetsNoWork::new(values, binding, request, &c)?;
    goal.validate_targets_request(&watch.request)?;
    assert_eq!(watch.monitor.originals().len(), 2);
    assert!(!watch.monitor.current_goal_proven());
    assert!(
        watch.monitor.latest().is_none(),
        "planning is not a stability sample"
    );
    Ok(())
}
