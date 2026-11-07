use super::*;
use crate::semantic_work_orders::{
    InventoryBaselineProjection, ObservedInventoryGoal, ObservedInventoryIncreaseObjective,
};

fn increase_fixture(
    baseline: u32,
    increase: u64,
) -> Result<(ProductionFixture, ObservedInventoryIncreaseObjective)> {
    let mut f = inventory_fixture(1)?;
    let raw = operations(&f.orders.borrow().queue, baseline);
    let projection = InventoryBaselineProjection::new(f.plan.anchor, raw, &f.c)?;
    let mut policy = EvidencePolicy::at(projection.snapshot().anchor());
    policy.paused = true;
    policy.entity_kinds.insert(EntityKind::Item, EvidenceCoverage::Complete);
    policy.sources = projection.observed_sources();
    let objective = projection.increase("BED", increase, policy, &f.c)?;
    let goal = objective.predicate();
    f.plan.steps[1].postconditions = vec![goal.clone()];
    f.plan.steps[1].obligation.as_mut()
        .ok_or_else(|| invalid("missing selected increase obligation"))?.terminal = goal.clone();
    f.plan.terminal_condition =
        Predicate::All(vec![f.plan.steps[0].postconditions[0].clone(), goal]).normalized();
    reseal(&mut f.plan);
    let Executor { workforce, orders, evidence, factory, .. } = f.executor;
    f.executor = Executor::new(f.plan.clone(), workforce, orders, evidence, factory, &f.c)?;
    Ok((f, objective))
}

fn changed_source_sample(
    f: &mut ProductionFixture,
    monitor: &mut SelectedWorkOrderGoalMonitor<Storage>,
    change: usize,
) -> Result<SelectedWorkOrderGoalResult> {
    let old = f.orders.borrow().queue.clone();
    let mut raw = old.canonical_bytes().to_vec();
    raw[16..24].copy_from_slice(&(old.sequence() + 1).to_be_bytes());
    raw[24..32].copy_from_slice(&125u64.to_be_bytes());
    let queue = WorkOrderObservation::decode(&raw)?;
    f.orders.borrow_mut().queue = queue.clone();
    let before = progress(&queue, 100, 2)?;
    let after = progress(&queue, 100, 3)?;
    let mut operations = operations(&queue, 7);
    match change {
        0 => operations.jobs.bridge_generation += 1,
        1 => operations.jobs.next_job_id -= 1,
        2 => operations.next_building_id -= 1,
        _ => operations.next_item_id -= 1,
    }
    f.executor.poll_selected_order_goal(
        monitor,
        ObservationCursor { epoch: f.c.anchor.cursor.epoch, sequence: 1 },
        &f.c,
        |_| Ok(Progress {
            manifest: ProgressManifest {
                generation: 501,
                df_version: "0.51.11".into(),
                dfhack_version: "0.51.11-r1".into(),
            },
            captures: vec![before, after],
        }),
        |_| Ok(Operations(operations)),
        |_, _| Err(invalid("regressed baseline must refuse before source-policy publication")),
    )
}

#[test]
fn selected_inventory_increase_requires_net_growth_and_survives_order_retirement() -> Result<()> {
    let (mut f, objective) = increase_fixture(5, 2)?;
    f.applied(0)?;
    f.applied(1)?;
    let original = f.plan.clone();
    let mut monitor = open(&mut f, 1, Storage::default(), true)?;
    let existing = sample(&mut f, &mut monitor, 1, 125, 5, |p, _| Ok(grant(p)))?;
    assert_eq!(existing.progress().current_goal, PredicateTruth::False);
    assert!(!existing.selected_goal_proven());
    let partial = sample(&mut f, &mut monitor, 2, 130, 6, |p, _| Ok(grant(p)))?;
    assert_eq!(partial.progress().current_goal, PredicateTruth::False);
    retire(&f, 100)?;
    let first = sample(&mut f, &mut monitor, 3, 135, 7, |p, _| Ok(grant(p)))?;
    assert_eq!(first.progress().current_goal, PredicateTruth::True);
    assert!(!first.selected_goal_proven());
    let proved = sample(&mut f, &mut monitor, 4, 140, 7, |p, _| Ok(grant(p)))?;
    assert!(proved.selected_goal_proven());
    assert!(!proved.original_goal_proven());
    assert!(!proved.progress().receipt_linked);
    assert!(matches!(proved.progress().physical_work, WorkOrderPhysicalState::Unknown { .. }));
    assert_eq!(monitor.original_plan(), &original);
    assert_eq!(
        ObservedInventoryGoal::from_selected_plan(monitor.original_plan(), StepId::new(1)),
        Some(ObservedInventoryGoal::Increase(objective)),
    );
    assert_eq!(f.orders.borrow().prepares, 1);
    assert_eq!(f.orders.borrow().commits, 1);
    Ok(())
}

#[test]
fn selected_inventory_increase_replay_keeps_baseline_and_historical_achievement() -> Result<()> {
    let (mut f, objective) = increase_fixture(5, 2)?;
    f.applied(0)?;
    f.applied(1)?;
    retire(&f, 100)?;
    let history = Storage::default();
    let mut monitor = open(&mut f, 1, history.clone(), true)?;
    sample(&mut f, &mut monitor, 1, 125, 7, |p, _| Ok(grant(p)))?;
    let proved = sample(&mut f, &mut monitor, 2, 130, 7, |p, _| Ok(grant(p)))?;
    assert!(proved.selected_goal_proven());
    let first = monitor.first_satisfied_anchor();
    let bytes = monitor.history_byte_len();
    drop(monitor);
    let mut f = f.reopen(false)?;
    let mut monitor = open(&mut f, 1, history, false)?;
    assert!(monitor.latest().is_none());
    assert_eq!(monitor.first_satisfied_anchor(), first);
    assert_eq!(monitor.history_byte_len(), bytes);
    assert_eq!(
        ObservedInventoryIncreaseObjective::from_selected_plan(monitor.original_plan(), StepId::new(1)),
        Some(objective),
    );
    let consumed = sample(&mut f, &mut monitor, 3, 135, 6, |p, _| Ok(grant(p)))?;
    assert_eq!(consumed.progress().current_goal, PredicateTruth::False);
    assert!(!consumed.selected_goal_proven());
    assert_eq!(consumed.progress().first_satisfied_anchor, first);
    assert_eq!(f.orders.borrow().commits, 1);
    Ok(())
}

#[test]
fn selected_inventory_increase_first_capture_regression_fences_live_and_replay() -> Result<()> {
    for change in 0..4 {
        let (mut f, _) = increase_fixture(5, 2)?;
        f.applied(0)?;
        f.applied(1)?;
        let history = Storage::default();
        let mut monitor = open(&mut f, 1, history.clone(), true)?;
        assert!(changed_source_sample(&mut f, &mut monitor, change).is_err(), "case {change}");
        assert!(monitor.is_fenced(), "case {change}");
        assert!(monitor.latest().is_none());
        assert!(monitor.high_tick() >= GameTick(125));
        drop(monitor);
        let mut f = f.reopen(false)?;
        let mut monitor = open(&mut f, 1, history, false)?;
        assert!(monitor.is_fenced());
        assert!(monitor.first_satisfied_anchor().is_none());
        let reads = f.orders.borrow().reads;
        assert!(sample(&mut f, &mut monitor, 2, 130, 7, |p, _| Ok(grant(p))).is_err());
        assert_eq!(f.orders.borrow().reads, reads);
        assert_eq!(f.orders.borrow().commits, 1);
    }
    Ok(())
}

#[test]
fn selected_inventory_increase_failed_publication_resets_stability_not_the_baseline() -> Result<()> {
    let (mut f, objective) = increase_fixture(5, 2)?;
    f.applied(0)?;
    f.applied(1)?;
    let mut monitor = open(&mut f, 1, Storage::default(), true)?;
    sample(&mut f, &mut monitor, 1, 125, 7, |p, _| Ok(grant(p)))?;
    let mut calls = 0;
    assert!(sample(&mut f, &mut monitor, 2, 130, 7, |p, _| {
        calls += 1;
        if calls == 2 { Err(invalid("deny final inventory growth publication")) }
        else { Ok(grant(p)) }
    }).is_err());
    assert_eq!(calls, 2);
    assert!(monitor.first_satisfied_anchor().is_none());
    let first = sample(&mut f, &mut monitor, 3, 135, 7, |p, _| Ok(grant(p)))?;
    assert!(!first.selected_goal_proven());
    let second = sample(&mut f, &mut monitor, 4, 140, 7, |p, _| Ok(grant(p)))?;
    assert!(second.selected_goal_proven());
    assert_eq!(
        ObservedInventoryIncreaseObjective::from_selected_plan(monitor.original_plan(), StepId::new(1)),
        Some(objective),
    );
    assert_eq!(f.orders.borrow().commits, 1);
    Ok(())
}

#[test]
fn selected_inventory_increase_reopen_preserves_the_absolute_deadline() -> Result<()> {
    let (mut f, objective) = increase_fixture(5, 2)?;
    f.applied(0)?;
    f.applied(1)?;
    let history = Storage::default();
    let mut monitor = open(&mut f, 1, history.clone(), true)?;
    let first = sample(&mut f, &mut monitor, 1, 215, 7, |p, _| Ok(grant(p)))?;
    assert!(!first.selected_goal_proven());
    drop(monitor);
    let mut f = f.reopen(false)?;
    let mut monitor = open(&mut f, 1, history, false)?;
    assert!(monitor.latest().is_none());
    let late = sample(&mut f, &mut monitor, 2, 225, 7, |p, _| Ok(grant(p)))?;
    assert_eq!(late.progress().current_goal, PredicateTruth::True);
    assert!(!late.selected_goal_proven());
    assert!(matches!(late.progress().obligation, Some(ObligationStatus::Failed { .. })));
    assert!(monitor.first_satisfied_anchor().is_none());
    assert_eq!(
        ObservedInventoryIncreaseObjective::from_selected_plan(monitor.original_plan(), StepId::new(1)),
        Some(objective),
    );
    assert_eq!(monitor.original_plan().steps[1].obligation.as_ref().map(|spec| spec.deadline_tick), Some(GameTick(220)));
    Ok(())
}
