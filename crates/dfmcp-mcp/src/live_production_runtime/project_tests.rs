//! Original-policy and typed-census mechanics use the existing qualified
//! compiler fixtures. These tests do not claim native Workshop completion.
use super::super::project_goal::ProjectSingleStockGoal;
use super::super::stock_target::StockTargetNoWork;
use super::super::stock_target_request::tests::{
    stock_assessment, stock_fixture, stock_policy, stock_request, stock_workers,
};
use super::*;
use dfmcp_world::PredicateEvidence;

fn single_goal(assign_labor: bool) -> ProjectSingleStockGoal {
    ProjectSingleStockGoal {
        recipe: "wooden_bed".into(),
        minimum_stack_units: 7,
        max_order_amount: 100,
        deadline_tick: 300,
        poll_interval_ticks: 5,
        stable_samples: 2,
        assign_labor,
    }
}

fn goal(assign_labor: bool) -> ProjectStockGoal {
    ProjectStockGoal::Single(single_goal(assign_labor))
}

#[test]
fn project_goal_is_closed_and_bounded_before_child_io() -> Result<()> {
    let original = goal(false);
    let raw = serde_json::to_value(&original)
        .map_err(|_| super::invalid("test goal serialization failed"))?;
    assert_eq!(
        serde_json::from_value::<ProjectStockGoal>(raw.clone())
            .map_err(|_| super::invalid("test goal decoding failed"))?,
        original
    );
    for (field, value) in [
        ("recipe", serde_json::json!("smelt_ore")),
        ("minimum_stack_units", serde_json::json!(0)),
        ("minimum_stack_units", serde_json::json!(1_000_001)),
        ("max_order_amount", serde_json::json!(0)),
        ("max_order_amount", serde_json::json!(101)),
        ("deadline_tick", serde_json::json!(0)),
        ("deadline_tick", serde_json::json!(u64::MAX)),
        ("poll_interval_ticks", serde_json::json!(0)),
        ("poll_interval_ticks", serde_json::json!(u64::MAX)),
        ("stable_samples", serde_json::json!(0)),
        ("stable_samples", serde_json::json!(65)),
    ] {
        let mut invalid = raw.clone();
        invalid[field] = value;
        assert!(
            serde_json::from_value::<ProjectStockGoal>(invalid)
                .map_err(|_| super::invalid("test goal field decoding failed"))?
                .validate()
                .is_err(),
            "{field}"
        );
    }
    let mut invalid = raw;
    invalid["amount"] = serde_json::json!(1);
    assert!(serde_json::from_value::<ProjectStockGoal>(invalid).is_err());
    for recipe in ["wooden_bed", "wooden_door", "wooden_table", "wooden_chair"] {
        let mut admitted = single_goal(false);
        admitted.recipe = recipe.into();
        ProjectStockGoal::Single(admitted).validate()?;
    }
    Ok(())
}

#[test]
fn production_child_cannot_retarget_or_renew_original_project_request() -> Result<()> {
    let original = goal(false);
    let request = stock_request(7, 100)?;
    original.validate_request(&request)?;
    let edits: [fn(&mut StockTargetRequest); 8] = [
        |value| value.recipe = "wooden_door".into(),
        |value| value.minimum_stack_units = 8,
        |value| value.max_order_amount = 99,
        |value| value.deadline_tick = 301,
        |value| value.poll_interval_ticks = 6,
        |value| value.stable_samples = 3,
        |value| value.assign_labor = true,
        |value| value.labor = "MINE".into(),
    ];
    for edit in edits {
        let mut changed = request.clone();
        edit(&mut changed);
        assert!(original.validate_request(&changed).is_err());
    }
    // Selection remains possible from the later, independently checked native
    // observation. It cannot rewrite the parent's fixed goal or grant a labor.
    let mut selected = request;
    selected.units = vec!["43".into()];
    selected.expected_observation = Digest32::of_bytes(b"another observed selection").to_hex();
    original.validate_request(&selected)?;
    Ok(())
}

#[test]
fn recovered_project_stock_target_keeps_exact_original_goal_keys_and_workshop() -> Result<()> {
    let (projection, mut c) = stock_fixture()?;
    // This test covers both the existing-worker order and the two-action
    // labor-assignment plus finite-order original.
    c.budget.max_actions = 2;
    let evidence =
        PredicateEvidence::scoped(&projection.snapshot, stock_policy(&projection.snapshot))?;
    let assessment = stock_assessment(&c, 7, 100, 5, 99)?;
    let origin = Digest32::of_bytes(b"immutable Workshop origin identity mechanics");
    let edits: [fn(&mut ProjectSingleStockGoal); 7] = [
        |value| value.recipe = "wooden_door".into(),
        |value| value.minimum_stack_units = 8,
        |value| value.max_order_amount = 99,
        |value| value.deadline_tick = 301,
        |value| value.poll_interval_ticks = 6,
        |value| value.stable_samples = 3,
        |value| value.assign_labor = !value.assign_labor,
    ];
    for assign_labor in [false, true] {
        let original_goal = goal(assign_labor);
        let mut request = stock_request(7, 100)?;
        request.assign_labor = assign_labor;
        let plan = if assign_labor {
            request.compile_with_labor_assignment(&assessment, &evidence, &c, Some(origin))?
        } else {
            request.compile(&assessment, &evidence, &stock_workers()?, &c, Some(origin))?
        };
        let limits = dfmcp_adapter::live_production::archive_limits();
        let (bytes, seal) = dfmcp_intent::plan_archive::encode(&plan, limits)?;
        let recovered = dfmcp_intent::plan_archive::decode(&bytes, seal, limits)?;
        original_goal.validate_original(&recovered, origin)?;
        assert_eq!(recovered, plan);
        for edit in edits {
            let mut changed = single_goal(assign_labor);
            edit(&mut changed);
            assert!(
                ProjectStockGoal::Single(changed)
                    .validate_original(&recovered, origin)
                    .is_err()
            );
        }
        assert!(
            original_goal
                .validate_original(&recovered, Digest32::of_bytes(b"other Workshop original"),)
                .is_err()
        );
        assert!(
            original_goal
                .validate_original(&recovered, Digest32::ZERO)
                .is_err()
        );
        let mut tampered = recovered;
        tampered
            .steps
            .last_mut()
            .ok_or_else(|| super::invalid("test original order is absent"))?
            .idempotency_key = "different-native-key".into();
        tampered.digest = tampered.compute_digest();
        tampered.id = tampered.expected_id();
        assert!(original_goal.validate_original(&tampered, origin).is_err());
    }
    let without_donor =
        stock_request(7, 100)?.compile(&assessment, &evidence, &stock_workers()?, &c, None)?;
    assert!(
        goal(false)
            .validate_original(&without_donor, origin)
            .is_err()
    );
    Ok(())
}

#[test]
fn no_work_project_uses_qualified_census_at_exact_anchor_without_original_proof() -> Result<()> {
    let (_, c) = stock_fixture()?;
    let request = stock_request(7, 100)?;
    let retained =
        StockTargetNoWork::new(stock_assessment(&c, 7, 100, 9, 99)?, request.clone(), &c)?;
    let census = satisfied_census(&goal(false), &retained, c.anchor)?;
    assert_eq!(census.anchor, c.anchor);
    assert_eq!(census.observed_stack_units, 9);
    assert_eq!(census.target_stack_units, 7);
    assert_eq!(census.witness, retained.assessment.witness());
    assert_eq!(census.source_digest, retained.assessment.source_digest());
    assert_eq!(
        census.source_counters,
        retained.assessment.source_counters()
    );
    let mut later = c.anchor;
    later.tick = dfmcp_core::GameTick(later.tick.get() + 1);
    assert!(satisfied_census(&goal(false), &retained, later).is_err());
    assert!(satisfied_census(&goal(true), &retained, c.anchor).is_err());
    assert!(StockTargetNoWork::new(stock_assessment(&c, 7, 100, 5, 99)?, request, &c,).is_err());
    let status = ProjectProductionStatus {
        original_plan_digest: None,
        original_goal_proven: false,
        current_goal_anchor: None,
        clocks_settled: true,
        already_satisfied: Some(census),
        no_work_stock_goal: serde_json::Value::Null,
    }
    .to_json();
    assert_eq!(status["original_goal_proven"], false);
    assert!(status["original_plan_digest"].is_null());
    assert_eq!(status["already_satisfied"]["no_order_required"], true);
    assert_eq!(
        status["already_satisfied"]["worker_prerequisite_checked"],
        false
    );
    assert_eq!(status["already_satisfied"]["stability_proven"], false);
    assert_eq!(status["already_satisfied"]["satisfied_at_anchor"], true);
    assert_eq!(status["already_satisfied"]["fresh_for_current_call"], false);
    assert_eq!(
        status["already_satisfied"]["current_inventory_proven"],
        false
    );
    Ok(())
}
