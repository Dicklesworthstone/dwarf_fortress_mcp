use super::*;
use crate::live_production::{
    ProductionLayout, SelectedParentGoalResult, compose_production_portfolio_goal,
};

fn minimum(item_type: &str, units: u64) -> Predicate {
    Predicate::EntityQuantity {
        kind: EntityKind::Item,
        filter_field: "type_key".into(),
        filter_value: Value::Text(item_type.into()),
        quantity_field: Some("stack_size".into()),
        op: CompareOp::Ge,
        value: units,
    }
}

fn assigned_fixture(count: usize, minima: &[(&str, u64)]) -> Result<ProductionFixture> {
    let mut f = inventory_fixture(count)?;
    f.c.budget.max_bytes = 8 * 1024 * 1024 * 1024;
    let mut terminal = vec![f.plan.terminal_condition.clone()];
    terminal.extend(
        minima
            .iter()
            .map(|(item_type, units)| minimum(item_type, *units)),
    );
    f.plan.terminal_condition = Predicate::All(terminal).normalized();
    reseal(&mut f.plan);
    let Executor {
        workforce,
        orders,
        evidence,
        factory,
        ..
    } = f.executor;
    f.executor = Executor::new(f.plan.clone(), workforce, orders, evidence, factory, &f.c)?;
    Ok(f)
}

fn apply_original(f: &mut ProductionFixture) -> Result<()> {
    for index in 0..f.plan.steps.len() {
        f.applied(index)?;
    }
    // The native effect fixture and observation fixture are independent. Move
    // the latter to the acknowledged assignment's actual intervention fence.
    f.workforce.data.borrow_mut().fixture.sequence += 1;
    Ok(())
}

fn set_tick(f: &mut ProductionFixture, tick: u64, enabled: bool) -> Result<()> {
    {
        let mut source = f.workforce.data.borrow_mut();
        source.fixture.tick = tick;
        source.fixture.citizens[0].labors[0] = u8::from(enabled);
    }
    let old = f.orders.borrow().queue.clone();
    let mut bytes = old.canonical_bytes().to_vec();
    bytes[16..24].copy_from_slice(&(old.sequence() + 1).to_be_bytes());
    bytes[24..32].copy_from_slice(&tick.to_be_bytes());
    f.orders.borrow_mut().queue = WorkOrderObservation::decode(&bytes)?;
    Ok(())
}

fn open_portfolio(
    f: &mut ProductionFixture,
    step: u32,
    history: Storage,
    initialize: bool,
) -> Result<SelectedWorkOrderGoalMonitor<Storage>> {
    f.executor.begin_portfolio_order_goal_monitor(
        StepId::new(step),
        history,
        initialize,
        false,
        &f.c,
        |p, _| Ok(grant(p)),
    )
}

fn sample<A>(
    f: &mut ProductionFixture,
    monitor: &mut SelectedWorkOrderGoalMonitor<Storage>,
    source_sequence: u64,
    progress_sequence: u64,
    doors: u32,
    order_policy: A,
    portfolio: bool,
) -> Result<SelectedParentGoalResult>
where
    A: FnMut(&WorkOrderGoalProjection, &OperationContext) -> Result<EvidencePolicy>,
{
    let queue = f.orders.borrow().queue.clone();
    let step = monitor.step_id().get();
    let before = progress(&queue, 99 + step, progress_sequence)?;
    let after = progress(&queue, 99 + step, progress_sequence + 1)?;
    let mut captured = operations(&queue, 2);
    if doors > 0 {
        captured.items.push(LiveItem {
            native_id: 3,
            item_type: 6,
            type_key: "DOOR".into(),
            subtype: -1,
            material_type: 420,
            material_index: 0,
            stack_size: doors,
            raw_position: MapCoord::new(1, 1, 1),
            flags: 64,
            container_native_id: None,
            holder_building_native_id: None,
        });
    }
    let (capsule, original) = observation(&[42])?;
    let identity = LiveRoutingEvidence::citizens_v1(
        PredicateEvidence::scoped(&original.snapshot, policy(&original.snapshot))?,
        &original,
        &capsule,
    )?;
    let cursor = ObservationCursor {
        epoch: f.c.anchor.cursor.epoch,
        sequence: source_sequence,
    };
    let sources = (
        |_: &OperationContext| {
            Ok(Progress {
                manifest: ProgressManifest {
                    generation: 501,
                    df_version: "0.51.11".into(),
                    dfhack_version: "0.51.11-r1".into(),
                },
                captures: vec![before, after],
            })
        },
        |_: &OperationContext| Ok(Operations(captured)),
    );
    let policies = (
        order_policy,
        |p: &crate::semantic_workforce::WorkforceGoalProjection, _: &OperationContext| {
            Ok(policy(p.snapshot()))
        },
    );
    if portfolio {
        f.executor.poll_selected_portfolio_goal(
            monitor,
            &identity,
            cursor,
            &f.c,
            sources,
            policies,
            ObservationCursor {
                epoch: cursor.epoch,
                sequence: source_sequence * 4 + u64::from(step),
            },
        )
    } else {
        f.executor
            .poll_selected_parent_goal(monitor, &identity, cursor, &f.c, sources, policies)
    }
}

fn all_progress(
    queue: &WorkOrderObservation,
    steps: &[u32],
    sequence: u64,
) -> Result<ProgressObservation> {
    let first = steps
        .first()
        .ok_or_else(|| invalid("original orders absent"))?;
    let template = progress(queue, 99 + first, sequence)?;
    let count_at = 47 + queue.world_folder().len();
    let mut bytes = template.canonical_bytes()[..count_at].to_vec();
    bytes.extend_from_slice(&(steps.len() as u32).to_be_bytes());
    let mut ids = Vec::new();
    for step in steps {
        let id = 99 + step;
        ids.push(id);
        let capture = progress(queue, id, sequence)?;
        bytes.extend_from_slice(&capture.canonical_bytes()[count_at + 4..]);
    }
    ProgressObservation::decode(&bytes, &ids)
}

fn join(
    f: &ProductionFixture,
    samples: &[SelectedParentGoalResult],
    sequence: u64,
) -> Result<crate::live_production::ProductionPortfolioGoalProgress> {
    let queue = f.orders.borrow().queue.clone();
    let steps: Vec<_> = f
        .plan
        .steps
        .iter()
        .skip(1)
        .map(|step| step.id.get())
        .collect();
    compose_production_portfolio_goal(
        &f.plan,
        samples,
        &all_progress(&queue, &steps, sequence)?,
        &all_progress(&queue, &steps, sequence + 20)?,
        &f.c,
        |_, p, _| Ok(grant(p)),
        |p, _| Ok(policy(p.snapshot())),
    )
}

#[test]
fn assigned_layout_retains_closed_stock_minima_and_exact_original_archive() -> Result<()> {
    let f = assigned_fixture(2, &[("TABLE", 2), ("BED", 3), ("DOOR", 2), ("CHAIR", 2)])?;
    let layout = ProductionLayout::from_plan(&f.plan)?;
    assert_eq!(layout.assignment_step(), Some(StepId::ZERO));
    assert_eq!(layout.order_steps(), &[StepId::new(1), StepId::new(2)]);
    assert!(
        layout.existing_labor().is_none(),
        "an assignment is not existing-worker evidence"
    );
    assert_eq!(
        layout.inventory_minima(),
        &[
            minimum("BED", 3),
            minimum("CHAIR", 2),
            minimum("DOOR", 2),
            minimum("TABLE", 2),
        ]
    );
    let (bytes, seal) =
        dfmcp_intent::plan_archive::encode(&f.plan, crate::live_production::archive_limits())?;
    let reopened =
        dfmcp_intent::plan_archive::decode(&bytes, seal, crate::live_production::archive_limits())?;
    assert_eq!(reopened, f.plan);
    assert_eq!(ProductionLayout::from_plan(&reopened)?, layout);
    for count in 1..=4 {
        let legacy = inventory_fixture(count)?;
        assert!(
            ProductionLayout::from_plan(&legacy.plan)?
                .inventory_minima()
                .is_empty()
        );
    }
    Ok(())
}

#[test]
fn assigned_layout_rejects_weakened_parent_and_unbounded_or_arbitrary_extras() -> Result<()> {
    let base = inventory_fixture(1)?.plan;
    for case in 0..8 {
        let mut extra = minimum("DOOR", 2);
        if let Predicate::EntityQuantity {
            kind,
            filter_field,
            filter_value,
            quantity_field,
            op,
            value,
        } = &mut extra
        {
            match case {
                0 => *kind = EntityKind::Building,
                1 => *filter_field = "label".into(),
                2 => *filter_value = Value::Text("FOOD".into()),
                3 => *quantity_field = None,
                4 => *quantity_field = Some("count".into()),
                5 => *op = CompareOp::Eq,
                6 => *value = 0,
                _ => *value = 1_000_001,
            }
        }
        let mut altered = base.clone();
        altered.terminal_condition =
            Predicate::All(vec![altered.terminal_condition, extra]).normalized();
        reseal(&mut altered);
        assert!(
            ProductionLayout::from_plan(&altered).is_err(),
            "case {case}"
        );
    }
    for terminal in [
        minimum("DOOR", 2),
        base.steps[0].postconditions[0].clone(),
        base.steps[1].postconditions[0].clone(),
        Predicate::All(vec![
            base.terminal_condition.clone(),
            minimum("DOOR", 2),
            minimum("DOOR", 3),
        ])
        .normalized(),
        Predicate::Any(vec![base.terminal_condition.clone(), minimum("DOOR", 2)]).normalized(),
    ] {
        let mut altered = base.clone();
        altered.terminal_condition = terminal;
        reseal(&mut altered);
        assert!(ProductionLayout::from_plan(&altered).is_err());
    }
    // The archive visitor must reject recursive or wide caller inputs before
    // normalizing, hashing or flattening this new complete-parent grammar.
    let mut recursive = base.clone();
    for _ in 0..32 {
        recursive.terminal_condition = Predicate::Not(Box::new(recursive.terminal_condition));
    }
    let mut wide = base;
    wide.terminal_condition = Predicate::All(vec![minimum("DOOR", 2); 4097]);
    for altered in [recursive, wide] {
        let refused = ProductionLayout::from_plan(&altered)
            .err()
            .ok_or_else(|| invalid("unbounded original parent was accepted"))?;
        assert_eq!(refused.code, ErrorCode::BudgetExceeded);
    }
    Ok(())
}

#[test]
fn assigned_current_parent_checks_no_action_stock_after_leaf_achievement_and_reopen() -> Result<()>
{
    let mut f = assigned_fixture(1, &[("DOOR", 2)])?;
    let original = f.plan.clone();
    apply_original(&mut f)?;
    let history = Storage::default();
    let mut monitor = open(&mut f, 1, history.clone(), true)?;
    for (sequence, tick, doors, enabled) in [
        (1, 125, 2, true),
        (2, 130, 0, true),
        (3, 135, 2, true),
        (4, 140, 2, false),
    ] {
        set_tick(&mut f, tick, enabled)?;
        let sampled = sample(
            &mut f,
            &mut monitor,
            sequence,
            sequence * 10,
            doors,
            |p, _| Ok(grant(p)),
            false,
        )?;
        let finding = &sampled.parent_progress().inventory_requirements[0];
        assert_eq!(finding.item_type, "DOOR");
        assert_eq!(finding.minimum_stack_units, 2);
        assert_eq!(finding.observed_stack_units, Some(u64::from(doors)));
        assert_eq!(
            sampled.parent_progress().current_truth,
            if doors >= 2 && enabled {
                PredicateTruth::True
            } else {
                PredicateTruth::False
            }
        );
        assert_eq!(sampled.original_goal_proven(), sequence == 3);
        assert_eq!(sampled.selected_goal_proven(), sequence >= 2);
    }
    assert_eq!(f.orders.borrow().commits, 1);
    assert_eq!(f.workforce.data.borrow().calls[2], 1);
    assert_eq!(f.executor.original_plan(), &original);
    let achieved = monitor.first_satisfied_anchor();
    drop(monitor);
    let mut f = f.reopen(false)?;
    let mut reopened = open(&mut f, 1, history, false)?;
    assert!(reopened.latest().is_none());
    assert_eq!(reopened.first_satisfied_anchor(), achieved);
    set_tick(&mut f, 145, true)?;
    let sampled = sample(&mut f, &mut reopened, 5, 50, 0, |p, _| Ok(grant(p)), false)?;
    assert!(sampled.selected_goal_proven());
    assert!(!sampled.original_goal_proven());
    assert_eq!(
        sampled.parent_progress().inventory_requirements[0].observed_stack_units,
        Some(0)
    );
    assert_eq!(reopened.original_plan(), &original);
    Ok(())
}

#[test]
fn assigned_stock_minimum_cannot_use_unqualified_inventory_as_zero_or_true() -> Result<()> {
    let mut f = assigned_fixture(1, &[("DOOR", 2)])?;
    apply_original(&mut f)?;
    let mut monitor = open(&mut f, 1, Storage::default(), true)?;
    set_tick(&mut f, 125, true)?;
    let sampled = sample(
        &mut f,
        &mut monitor,
        1,
        10,
        2,
        |p, _| {
            let mut accepted = grant(p);
            accepted
                .entity_kinds
                .insert(EntityKind::Item, EvidenceCoverage::Observed);
            Ok(accepted)
        },
        false,
    )?;
    assert_eq!(
        sampled.parent_progress().current_truth,
        PredicateTruth::Unknown
    );
    assert_eq!(
        sampled.parent_progress().inventory_requirements[0].observed_stack_units,
        None
    );
    assert_eq!(
        sampled.parent_progress().inventory_requirements[0].current_truth,
        PredicateTruth::Unknown
    );
    assert!(!sampled.original_goal_proven());
    Ok(())
}

#[test]
fn assigned_portfolio_requires_all_original_orders_and_no_action_minima_together() -> Result<()> {
    let mut f = assigned_fixture(2, &[("DOOR", 2)])?;
    apply_original(&mut f)?;
    let mut first = open_portfolio(&mut f, 1, Storage::default(), true)?;
    let mut second = open_portfolio(&mut f, 2, Storage::default(), true)?;
    for (cohort, tick, doors) in [(1, 125, 2), (2, 130, 0), (3, 135, 2)] {
        set_tick(&mut f, tick, true)?;
        let a = sample(
            &mut f,
            &mut first,
            cohort,
            cohort * 100 + 1,
            doors,
            |p, _| Ok(grant(p)),
            true,
        )?;
        let b = sample(
            &mut f,
            &mut second,
            cohort,
            cohort * 100 + 3,
            doors,
            |p, _| Ok(grant(p)),
            true,
        )?;
        assert!(!a.original_goal_proven());
        assert!(!b.original_goal_proven());
        assert!(join(&f, &[a.clone()], cohort * 100).is_err());
        let joined = join(&f, &[a, b], cohort * 100)?;
        assert_eq!(joined.inventory_requirements.len(), 1);
        assert_eq!(
            joined.inventory_requirements[0].observed_stack_units,
            Some(u64::from(doors))
        );
        assert_eq!(
            joined.current_truth,
            if doors >= 2 {
                PredicateTruth::True
            } else {
                PredicateTruth::False
            }
        );
        assert_eq!(joined.original_goal_proven, cohort == 3);
        assert_eq!(
            joined
                .selected_anchors
                .iter()
                .map(|(id, _)| *id)
                .collect::<Vec<_>>(),
            [StepId::new(1), StepId::new(2)]
        );
    }
    set_tick(&mut f, 140, true)?;
    let a = sample(&mut f, &mut first, 4, 401, 2, |p, _| Ok(grant(p)), true)?;
    let b = sample(&mut f, &mut second, 4, 403, 0, |p, _| Ok(grant(p)), true)?;
    assert!(
        join(&f, &[a, b], 400).is_err(),
        "stock from different captures cannot be combined"
    );
    assert_eq!(f.orders.borrow().commits, 2);
    assert_eq!(f.workforce.data.borrow().calls[2], 1);
    Ok(())
}
