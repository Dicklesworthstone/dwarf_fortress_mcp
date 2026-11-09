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
        marker.replace("target/1", "target/3"),
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

fn assigned_original() -> Result<PreparedPlan> {
    let (projection, mut c) = stock_fixture()?;
    c.budget.max_actions = 2;
    let evidence =
        PredicateEvidence::scoped(&projection.snapshot, stock_policy(&projection.snapshot))?;
    let assessment = stock_assessment(&c, 7, 100, 5, 99)?;
    let mut request = stock_request(7, 100)?;
    request.assign_labor = true;
    request.compile_with_labor_assignment(&assessment, &evidence, &c, None)
}

fn rekey_and_reseal(plan: &mut PreparedPlan) {
    for step in &mut plan.steps {
        step.idempotency_key =
            derive_step_idempotency_key(plan.intent_id, plan.anchor, step.id, &step.action);
        step.preconditions = step.preconditions.iter().map(Predicate::normalized).collect();
        step.preconditions.sort_by_key(Predicate::canonical_bytes);
        step.postconditions = step.postconditions.iter().map(Predicate::normalized).collect();
        step.postconditions.sort_by_key(Predicate::canonical_bytes);
    }
    plan.terminal_condition = plan.terminal_condition.normalized();
    reseal(plan);
}

fn changed_growth_source(predicate: &Predicate, generation: bool) -> Predicate {
    let mut changed = match predicate {
        Predicate::All(children) => Predicate::All(
            children
                .iter()
                .map(|child| changed_growth_source(child, generation))
                .collect(),
        ),
        other => other.clone(),
    };
    if let Predicate::EntityQuantityIncrease {
        source_generation,
        baseline_source_counters,
        ..
    } = &mut changed
    {
        if generation {
            *source_generation += 1;
        } else {
            baseline_source_counters[2] += 1;
        }
    }
    changed.normalized()
}

#[test]
fn assigned_stock_marker_rejects_loss_downgrade_and_untagged_manual_adoption() -> Result<()> {
    let original = assigned_original()?;
    let marker = parse_summary(&original.summary)?.expect("assignment marker");
    assert!(marker.assign_labor);
    let binding = requirement(&original)?.expect("assigned requirement");
    assert!(binding.assign_labor());
    for summary in [
        "Original manual growth goal".to_owned(),
        original.summary.replace("target/2", "target/1"),
        original.summary.replace("target/2", "target/3"),
        original.summary.replace(">=7", ">=8"),
        original.summary.replace("max=100", "max=99"),
    ] {
        let mut changed = original.clone();
        changed.summary = summary;
        rekey_and_reseal(&mut changed);
        assert!(
            requirement(&changed).is_err(),
            "stock-target policy cannot be dropped, downgraded or changed by resealing"
        );
    }
    let mut untagged = original.clone();
    if let Action::CreateWorkOrder { name, .. } = &mut untagged.steps[1].action {
        *name = "native wooden_bed goal".into();
    }
    rekey_and_reseal(&mut untagged);
    assert!(requirement(&untagged).is_err());

    let (projection, mut c) = stock_fixture()?;
    c.budget.max_actions = 2;
    let evidence =
        PredicateEvidence::scoped(&projection.snapshot, stock_policy(&projection.snapshot))?;
    let assessment = stock_assessment(&c, 7, 100, 5, 99)?;
    let legacy = stock_request(7, 100)?.compile(
        &assessment, &evidence, &stock_workers()?, &c, None,
    )?;
    let mut promoted = legacy;
    promoted.summary = original.summary.clone();
    rekey_and_reseal(&mut promoted);
    assert!(
        requirement(&promoted).is_err(),
        "a version-two marker cannot grant assignment to a version-one original"
    );
    Ok(())
}

#[test]
fn assigned_stock_binding_refuses_weakened_identity_labor_and_parent_truth() -> Result<()> {
    let original = assigned_original()?;
    for case in 0..10 {
        let mut changed = original.clone();
        match case {
            0 => changed.steps[0].preconditions = vec![Predicate::Paused(true)],
            1 => {
                let Predicate::All(children) = &mut changed.steps[0].preconditions[0] else {
                    panic!("complete original identity conjunction");
                };
                let native = children.iter_mut().find_map(|child| match child {
                    Predicate::FieldCompare {
                        field,
                        value: Value::I64(value),
                        ..
                    } if field == "raw_unit_id" => Some(value),
                    _ => None,
                }).expect("original native identity");
                *native += 1;
            }
            2 => changed.steps[0].postconditions = vec![Predicate::FieldCompare {
                entity_id: dfmcp_core::EntityId::new(43),
                field: "labor.CARPENTER".into(),
                op: CompareOp::Eq,
                value: Value::Bool(true),
            }],
            3 => {
                changed.terminal_condition =
                    changed.steps[1].obligation.as_ref().expect("obligation").terminal.clone();
            }
            4 => changed.steps[1].preconditions = vec![Predicate::True],
            5 => changed.steps[1].depends_on.clear(),
            6 => {
                changed.steps[1]
                    .obligation.as_mut().expect("obligation")
                    .failure = Some(Predicate::False);
            }
            7 => {
                changed.steps[0].compensation = Some(Action::SetLabor {
                    units: vec![dfmcp_core::EntityId::new(43), dfmcp_core::EntityId::new(44)],
                    labor: "CARPENTER".into(),
                    enabled: false,
                });
            }
            8 => {
                if let Action::SetLabor { enabled, .. } = &mut changed.steps[0].action {
                    *enabled = false;
                }
            }
            _ => {
                changed.steps[1].postconditions = vec![Predicate::Paused(true)];
                changed.steps[1].obligation.as_mut().expect("obligation").terminal =
                    Predicate::Paused(true);
            }
        }
        rekey_and_reseal(&mut changed);
        assert!(
            requirement(&changed).is_err(),
            "accepted weakened assignment stock original case {case}"
        );
    }
    Ok(())
}

#[test]
fn assigned_stock_binding_fences_inventory_incarnation_and_original_step_keys() -> Result<()> {
    let original = assigned_original()?;
    for generation in [true, false] {
        let mut changed = original.clone();
        changed.terminal_condition = changed_growth_source(&changed.terminal_condition, generation);
        for step in &mut changed.steps {
            step.postconditions = step.postconditions
                .iter()
                .map(|predicate| changed_growth_source(predicate, generation))
                .collect();
            if let Some(obligation) = &mut step.obligation {
                obligation.terminal = changed_growth_source(&obligation.terminal, generation);
            }
        }
        rekey_and_reseal(&mut changed);
        assert!(
            requirement(&changed).is_err(),
            "a newer inventory source cannot replace the sealed original census"
        );
    }
    for index in 0..2 {
        let mut changed = original.clone();
        changed.steps[index].idempotency_key =
            Digest32::of_bytes(b"substituted original step key").to_hex();
        reseal(&mut changed);
        assert!(requirement(&changed).is_err(), "step {index}");
    }
    Ok(())
}
