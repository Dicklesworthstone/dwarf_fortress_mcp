use super::super::existing_labor_request::ExistingLaborRequest;
use super::super::stock_target_request::tests::{
    stock_assessment, stock_fixture, stock_policy, stock_request, stock_workers,
};
use super::*;
use crate::semantic_work_order_runtime::inventory_baseline::InventoryBaselines;
use crate::semantic_work_order_runtime::request::Objective;
use dfmcp_world::{Predicate, PredicateEvidence};

fn reseal(plan: &mut PreparedPlan) {
    plan.digest = plan.compute_digest();
    plan.id = plan.expected_id();
}

#[test]
fn stock_marker_requires_exact_version_fields_numbers_witnesses_and_position() -> Result<()> {
    let witness = Digest32::of_bytes(b"qualified original stock assessment").to_hex();
    let marker = format!("{PREFIX}wooden_bed>=7 max=100 {witness}");
    let parsed = parse_summary(&marker)?.ok_or_else(|| invalid("marker missing"))?;
    assert_eq!(parsed.recipe, WorkOrderRecipe::WoodenBed);
    assert_eq!(parsed.target, 7);
    assert_eq!(parsed.max_order_amount, 100);
    assert!(parse_summary("Original manual growth goal")?.is_none());
    let mut unchanged = b"legacy identity bytes".to_vec();
    append_identity(&mut unchanged, None, &stock_workers()?, GameTick(300), 5, 2)?;
    assert_eq!(unchanged, b"legacy identity bytes");
    assert_eq!(summary("Legacy summary".into(), None)?, "Legacy summary");
    for malformed in [
        marker.replace("target/1", "target/2"),
        marker.replace(">=7", ">=0"),
        marker.replace(">=7", ">=1000001"),
        marker.replace(">=7", ">=07"),
        marker.replace(">=7", ">=+7"),
        marker.replace(">=7", ">=-1"),
        marker.replace(">=7", ">=18446744073709551616"),
        marker.replace("max=100", "max=0"),
        marker.replace("max=100", "max=101"),
        marker.replace("max=100", "max=0100"),
        marker.replace("max=100", "maximum=100"),
        marker.replace("wooden_bed", "BED"),
        marker.replace(&witness, &witness.to_uppercase()),
        marker.replace(&witness, &"0".repeat(64)),
        marker.replace(&format!(" {witness}"), ""),
        format!("{marker}\r\n"),
        format!("{marker} extra=true"),
        format!("{marker} "),
        marker.replace(">=7 ", ">=7  "),
        format!("{marker}\n"),
        format!(" {marker}"),
        format!("Other summary\n{marker}"),
        format!("{marker}\n{marker}"),
        format!("[dfmcp-stock-target/1] {witness}\nOriginal"),
    ] {
        assert!(parse_summary(&malformed).is_err(), "accepted {malformed:?}");
    }
    Ok(())
}

#[test]
fn stock_marker_cannot_adopt_manual_growth_or_change_bound_under_original_keys() -> Result<()> {
    let (projection, c) = stock_fixture()?;
    let evidence =
        PredicateEvidence::scoped(&projection.snapshot, stock_policy(&projection.snapshot))?;
    let assessment = stock_assessment(&c, 7, 100, 5, 99)?;
    let workers = stock_workers()?;
    let original = stock_request(7, 100)?.compile(&assessment, &evidence, &workers, &c, None)?;
    let objective = assessment.objective().expect("positive deficit").clone();
    let baselines = InventoryBaselines::from_qualified(objective);
    let manual = ExistingLaborRequest {
        kind: "existing_labor".into(),
        expected_observation: "11".repeat(32),
        units: vec!["44".into(), "43".into()],
        labor: "CARPENTER".into(),
        recipe: "wooden_bed".into(),
        amount: 2,
        deadline_tick: 300,
        poll_interval_ticks: 5,
        stable_samples: 2,
        objective: Some(Objective::ObservedInventoryIncrease {
            item_type: "BED".into(),
            increase_stack_units: 2,
        }),
    }
    .compile_with_inventory(&evidence, &workers, &baselines, &c)?;
    assert_eq!(requirement(&manual)?, None);
    let mut adopted = manual;
    adopted.summary = original.summary.clone();
    reseal(&mut adopted);
    assert!(
        requirement(&adopted).is_err(),
        "a marker cannot replace the request identity domain"
    );
    for summary in [
        original.summary.replace(">=7", ">=8"),
        original.summary.replace("max=100", "max=99"),
        original.summary.replace("wooden_bed", "wooden_door"),
        original.summary.replace(
            &assessment.witness().to_hex(),
            &Digest32::of_bytes(b"other assessment").to_hex(),
        ),
        format!("{}\nextra", original.summary),
    ] {
        let mut altered = original.clone();
        altered.summary = summary;
        reseal(&mut altered);
        assert!(requirement(&altered).is_err());
    }
    let mut no_marker = original.clone();
    no_marker.summary = "Original manual growth goal".into();
    assert!(
        requirement(&no_marker).is_err(),
        "removing policy invalidates retained archive identity"
    );
    Ok(())
}

#[test]
fn stock_binding_rejects_action_goal_or_baseline_substitution() -> Result<()> {
    let (projection, c) = stock_fixture()?;
    let evidence =
        PredicateEvidence::scoped(&projection.snapshot, stock_policy(&projection.snapshot))?;
    let assessment = stock_assessment(&c, 7, 100, 5, 99)?;
    let workers = stock_workers()?;
    let original = stock_request(7, 100)?.compile(&assessment, &evidence, &workers, &c, None)?;
    for case in 0..5 {
        let mut altered = original.clone();
        match case {
            0 => {
                if let Action::CreateWorkOrder { amount, .. } = &mut altered.steps[0].action {
                    *amount = 3;
                }
                altered.steps[0].idempotency_key = derive_step_idempotency_key(
                    altered.intent_id,
                    altered.anchor,
                    StepId::ZERO,
                    &altered.steps[0].action,
                );
            }
            1 => altered.steps[0].postconditions = vec![Predicate::True],
            2 => {
                altered.steps[0]
                    .obligation
                    .as_mut()
                    .expect("obligation")
                    .failure = Some(Predicate::False)
            }
            3 => {
                altered.terminal_condition = assessment.objective().expect("objective").predicate()
            }
            _ => {
                let mut predicate = assessment.objective().expect("objective").predicate();
                if let Predicate::EntityQuantityIncrease {
                    baseline_source_digest,
                    ..
                } = &mut predicate
                {
                    *baseline_source_digest = Digest32::of_bytes(b"different original census");
                }
                altered.steps[0].postconditions = vec![predicate.clone()];
                altered.steps[0]
                    .obligation
                    .as_mut()
                    .expect("obligation")
                    .terminal = predicate.clone();
                altered.terminal_condition =
                    Predicate::All(vec![workers.predicate(), predicate]).normalized();
            }
        }
        reseal(&mut altered);
        assert!(
            requirement(&altered).is_err(),
            "accepted altered original {case}"
        );
    }
    Ok(())
}
