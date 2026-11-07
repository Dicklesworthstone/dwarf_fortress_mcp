use super::*;
use crate::live_production::{
    ProductionLayout, SelectedOrderClockCustody, compose_existing_labor_portfolio_goal,
    compose_production_portfolio_goal,
};

const RECIPES: [(&str, &str, u8, &str); 4] = [
    ("CONSTRUCT_BED", "BED", 1, "ConstructBed"),
    ("CONSTRUCT_CHAIR", "CHAIR", 4, "ConstructThrone"),
    ("CONSTRUCT_DOOR", "DOOR", 2, "ConstructDoor"),
    ("CONSTRUCT_TABLE", "TABLE", 3, "ConstructTable"),
];

fn portfolio_fixture(count: usize) -> Result<ProductionFixture> {
    let mut f = existing_fixture()?;
    let requirement = ExistingLaborRequirement::from_plan(&f.plan)?
        .ok_or_else(|| invalid("fixture requirement absent"))?;
    let template = f.plan.steps[0].clone();
    let mut terminal = vec![requirement.predicate()];
    f.plan.steps.clear();
    for (index, (job, output, _, _)) in RECIPES.iter().take(count).enumerate() {
        let mut step = template.clone();
        step.id = StepId::new(index as u32);
        step.action = Action::CreateWorkOrder {
            name: format!("existing-worker original {output} quota"),
            job_token: (*job).into(),
            amount: 2,
            conditions: Vec::new(),
        }.normalized();
        step.idempotency_key = derive_step_idempotency_key(
            f.plan.intent_id, f.plan.anchor, step.id, &step.action,
        );
        let goal = Predicate::EntityQuantity {
            kind: EntityKind::Item,
            filter_field: "type_key".into(),
            filter_value: Value::Text((*output).into()),
            quantity_field: Some("stack_size".into()),
            op: CompareOp::Ge,
            value: 2,
        };
        step.postconditions = vec![goal.clone()];
        step.obligation.as_mut().ok_or_else(|| invalid("fixture obligation absent"))?.terminal = goal.clone();
        terminal.push(goal);
        f.plan.steps.push(step);
    }
    f.plan.terminal_condition = Predicate::All(terminal).normalized();
    f.plan.summary = "Complete a furniture portfolio with observed existing workers".into();
    reseal(&mut f.plan);
    let Executor { workforce, orders, evidence, factory, .. } = f.executor;
    f.executor = Executor::new(f.plan.clone(), workforce, orders, evidence, factory, &f.c)?;
    Ok(f)
}

fn set_tick(f: &mut ProductionFixture, tick: u64, enabled: bool) -> Result<()> {
    {
        let mut workforce = f.workforce.data.borrow_mut();
        workforce.fixture.tick = tick;
        workforce.fixture.citizens[0].labors[1] = u8::from(enabled);
    }
    let old = f.orders.borrow().queue.clone();
    let mut bytes = old.canonical_bytes().to_vec();
    bytes[16..24].copy_from_slice(&(old.sequence() + 1).to_be_bytes());
    bytes[24..32].copy_from_slice(&tick.to_be_bytes());
    f.orders.borrow_mut().queue = WorkOrderObservation::decode(&bytes)?;
    Ok(())
}

fn cohort_progress(
    queue: &WorkOrderObservation,
    steps: &[u32],
    sequence: u64,
) -> Result<ProgressObservation> {
    let mut raw = b"DFMWP012".to_vec();
    for value in [501u64, sequence, queue.tick()] { raw.extend_from_slice(&value.to_be_bytes()); }
    for value in [queue.site_id(), queue.next_order_id(), queue.order_ids().len() as u32] {
        raw.extend_from_slice(&value.to_be_bytes());
    }
    raw.push(1);
    text(&mut raw, queue.world_folder());
    raw.extend_from_slice(&(steps.len() as u32).to_be_bytes());
    let mut ids = Vec::new();
    for step in steps {
        let id = 100 + step;
        ids.push(id);
        raw.extend_from_slice(&id.to_be_bytes());
        let present = queue.order_ids().contains(&id);
        raw.push(u8::from(present));
        if present {
            let (_, _, recipe, job) = RECIPES[*step as usize];
            raw.extend_from_slice(&77i32.to_be_bytes());
            raw.push(recipe);
            for value in [2i32, 2] { raw.extend_from_slice(&value.to_be_bytes()); }
            raw.extend_from_slice(&3u32.to_be_bytes());
            for value in [0i32, -1, 1, 0, 0] { raw.extend_from_slice(&value.to_be_bytes()); }
            raw.extend_from_slice(&0u32.to_be_bytes());
            raw.extend_from_slice(&0u32.to_be_bytes());
            text(&mut raw, job);
            text(&mut raw, "");
        }
    }
    ProgressObservation::decode(&raw, &ids)
}

fn cohort_operations(queue: &WorkOrderObservation, count: usize) -> LiveOperationsObservation {
    let mut result = operations(queue, 0);
    for (index, (_, output, _, _)) in RECIPES.iter().take(count).enumerate() {
        result.items.push(LiveItem {
            native_id: 2 + index as u32,
            item_type: 5 + index as i32,
            type_key: (*output).into(),
            subtype: -1,
            material_type: 420,
            material_index: 0,
            stack_size: 2,
            raw_position: MapCoord::new(1, 1, 1),
            flags: 64,
            container_native_id: None,
            holder_building_native_id: None,
        });
    }
    result
}

fn portfolio_open(
    f: &mut ProductionFixture,
    step: u32,
    storage: Storage,
    initialize: bool,
) -> Result<SelectedWorkOrderGoalMonitor<Storage>> {
    f.executor.begin_portfolio_order_goal_monitor(
        StepId::new(step), storage, initialize, false, &f.c, |p, _| Ok(grant(p)),
    )
}

fn portfolio_sample(
    f: &mut ProductionFixture,
    monitor: &mut SelectedWorkOrderGoalMonitor<Storage>,
    source_sequence: u64,
    progress_sequence: u64,
    units: usize,
) -> Result<SelectedParentGoalResult> {
    let queue = f.orders.borrow().queue.clone();
    let step = monitor.step_id().get();
    let before = cohort_progress(&queue, &[step], progress_sequence)?;
    let after = cohort_progress(&queue, &[step], progress_sequence + 1)?;
    let operations = cohort_operations(&queue, units);
    let (capsule, original) = observation(&[42])?;
    let identity = LiveRoutingEvidence::citizens_v1(
        PredicateEvidence::scoped(&original.snapshot, policy(&original.snapshot))?,
        &original,
        &capsule,
    )?;
    let cursor = ObservationCursor { epoch: f.c.anchor.cursor.epoch, sequence: source_sequence };
    f.executor.poll_selected_portfolio_goal(
        monitor,
        &identity,
        cursor,
        &f.c,
        (
            |_| Ok(Progress {
                manifest: ProgressManifest {
                    generation: 501,
                    df_version: "0.51.11".into(),
                    dfhack_version: "0.51.11-r1".into(),
                },
                captures: vec![before, after],
            }),
            |_| Ok(Operations(operations)),
        ),
        (|p, _| Ok(grant(p)), |_, _| Err(invalid("assignment policy must not be consulted"))),
        ObservationCursor { epoch: cursor.epoch, sequence: source_sequence * 4 + u64::from(step) },
    )
}

fn join(
    f: &ProductionFixture,
    samples: &[SelectedParentGoalResult],
    sequence: u64,
) -> Result<crate::live_production::ProductionPortfolioGoalProgress> {
    let queue = f.orders.borrow().queue.clone();
    let steps: Vec<_> = f.plan.steps.iter().map(|step| step.id.get()).collect();
    let before = cohort_progress(&queue, &steps, sequence)?;
    let after = cohort_progress(&queue, &steps, sequence + 20)?;
    compose_existing_labor_portfolio_goal(
        &f.plan, samples, &before, &after, &f.c,
        |_, p, _| Ok(grant(p)), |p, _| Ok(policy(p.snapshot())),
    )
}

#[test]
fn existing_portfolio_layout_retains_zero_based_orders_and_rejects_incomplete_shapes() -> Result<()> {
    for count in 1..=4 {
        let f = portfolio_fixture(count)?;
        let layout = ProductionLayout::from_plan(&f.plan)?;
        assert_eq!(layout.assignment_step(), None);
        assert_eq!(layout.order_steps(), &(0..count).map(|n| StepId::new(n as u32)).collect::<Vec<_>>());
        assert_eq!(layout.archive_seal(), validate_plan(&f.plan)?);
        assert_eq!(layout.order_ordinal(StepId::ZERO), Some(0));
        assert_eq!(layout.order_ordinal(StepId::new(count as u32)), None);
        assert!(layout.existing_labor().is_some());
    }
    let f = portfolio_fixture(4)?;
    for case in 0..5 {
        let mut plan = f.plan.clone();
        match case {
            0 => {
                let terminal = plan.steps[2].obligation.as_ref().ok_or_else(|| invalid("fixture"))?.terminal.clone();
                if let Predicate::All(predicates) = &mut plan.terminal_condition { predicates.retain(|p| p != &terminal); }
            }
            1 => {
                plan.steps[1].action = plan.steps[0].action.clone();
                plan.steps[1].postconditions = plan.steps[0].postconditions.clone();
                plan.steps[1].obligation = plan.steps[0].obligation.clone();
                plan.steps[1].idempotency_key = derive_step_idempotency_key(
                    plan.intent_id, plan.anchor, plan.steps[1].id, &plan.steps[1].action,
                );
            }
            2 => plan.steps[1].depends_on = vec![StepId::ZERO],
            3 => plan.steps[2].obligation.as_mut().ok_or_else(|| invalid("fixture"))?.failure = Some(Predicate::False),
            _ => plan.terminal_condition = Predicate::All(vec![plan.terminal_condition, Predicate::Paused(true)]),
        }
        plan.terminal_condition = plan.terminal_condition.normalized();
        reseal(&mut plan);
        assert!(ProductionLayout::from_plan(&plan).is_err(), "case {case}");
        assert!(validate_plan(&plan).is_err(), "case {case}");
    }
    let legacy = ProductionFixture::new(2)?;
    let layout = ProductionLayout::from_plan(&legacy.plan)?;
    assert_eq!(layout.assignment_step(), Some(StepId::ZERO));
    assert_eq!(layout.order_steps(), &[StepId::new(1), StepId::new(2)]);
    assert!(layout.existing_labor().is_none());
    Ok(())
}

#[test]
fn four_existing_worker_orders_execute_with_original_keys_and_zero_assignment_effects() -> Result<()> {
    let mut f = portfolio_fixture(4)?;
    let original = f.plan.clone();
    let workforce_bytes = f.workforce.journal.0.borrow().file.get_ref().clone();
    let association_bytes = f.workforce.associations.0.borrow().file.get_ref().clone();
    for step in 0..4 { f.applied(step)?; }
    assert_eq!(f.orders.borrow().commits, 4);
    assert_eq!(&f.workforce.data.borrow().calls[1..], &[0; 4]);
    assert_eq!(*f.workforce.journal.0.borrow().file.get_ref(), workforce_bytes);
    assert_eq!(*f.workforce.associations.0.borrow().file.get_ref(), association_bytes);
    for step in &original.steps {
        assert!(f.orders.borrow().retained.contains_key(&step.idempotency_key));
        let inspected = f.executor.inspect_selected_order_custody(step.id, &f.c)?;
        assert_eq!(inspected.review().original_plan(), &original);
        assert_eq!(inspected.review().step_id(), step.id);
        assert!(!inspected.original_goal_proven());
    }
    assert_eq!(f.executor.original_plan(), &original);
    assert!(!f.executor.original_goal_proven());
    assert!(f.prepare(3).is_err());
    assert_eq!(f.orders.borrow().commits, 4);
    Ok(())
}

#[test]
fn later_existing_order_rechecks_labor_after_marker_without_changing_prior_receipts() -> Result<()> {
    let mut f = portfolio_fixture(3)?;
    f.applied(0)?;
    let original_receipt = f.orders.borrow().retained[&f.plan.steps[0].idempotency_key].clone();
    let prepared = f.prepare(1)?;
    f.workforce.data.borrow_mut().fixture.citizens[0].labors[1] = 0;
    assert!(f.commit(1, prepared).is_err());
    assert_eq!(f.orders.borrow().commits, 1);
    assert_eq!(f.orders.borrow().retained[&f.plan.steps[0].idempotency_key], original_receipt);
    f.workforce.data.borrow_mut().fixture.citizens[0].labors[1] = 1;
    assert!(f.prepare(2).is_err(), "uncertain original second order must fence later work");
    assert_eq!(&f.workforce.data.borrow().calls[1..], &[0; 4]);
    Ok(())
}

#[test]
fn complete_existing_portfolio_needs_every_temporal_leaf_and_shared_current_workers() -> Result<()> {
    let mut f = portfolio_fixture(4)?;
    for step in 0..4 { f.applied(step)?; }
    let mut monitors = (0..4).map(|step| portfolio_open(&mut f, step, Storage::default(), true)).collect::<Result<Vec<_>>>()?;
    for (cohort, tick, enabled) in [(1, 125, true), (2, 130, true), (3, 135, false)] {
        set_tick(&mut f, tick, enabled)?;
        let base = cohort * 100;
        let mut samples = Vec::new();
        for (index, monitor) in monitors.iter_mut().enumerate() {
            let sample = portfolio_sample(&mut f, monitor, cohort, base + 1 + 2 * index as u64, 4)?;
            assert!(!sample.original_goal_proven(), "a selected leaf is not the complete portfolio");
            assert_eq!(sample.step_id(), StepId::new(index as u32));
            samples.push(sample);
        }
        assert!(join(&f, &samples[..3], base).is_err());
        let result = join(&f, &samples, base)?;
        assert_eq!(result.selected_anchors.len(), 4);
        assert_eq!(result.original_goal_proven, cohort == 2);
        assert_eq!(result.current_truth, if enabled { PredicateTruth::True } else { PredicateTruth::False });
        if cohort == 2 {
            for id in 100..104 { retire(&f, id)?; }
        }
    }
    assert!(monitors.iter().all(|monitor| monitor.first_satisfied_anchor().is_some()));
    assert_eq!(f.orders.borrow().commits, 4);
    assert_eq!(&f.workforce.data.borrow().calls[1..], &[0; 4]);
    Ok(())
}

#[test]
fn existing_portfolio_join_refuses_cross_leaf_stock_drift_policy_loss_and_wrong_mode() -> Result<()> {
    let mut f = portfolio_fixture(2)?;
    f.applied(0)?;
    f.applied(1)?;
    let mut first = portfolio_open(&mut f, 0, Storage::default(), true)?;
    let mut second = portfolio_open(&mut f, 1, Storage::default(), true)?;
    set_tick(&mut f, 125, true)?;
    let a = portfolio_sample(&mut f, &mut first, 1, 101, 2)?;
    let b = portfolio_sample(&mut f, &mut second, 1, 103, 1)?;
    assert!(join(&f, &[a.clone(), b], 100).is_err());
    let b = portfolio_sample(&mut f, &mut second, 2, 105, 2)?;
    let queue = f.orders.borrow().queue.clone();
    let before = cohort_progress(&queue, &[0, 1], 100)?;
    let after = cohort_progress(&queue, &[0, 1], 120)?;
    let samples = [a, b];
    assert!(compose_existing_labor_portfolio_goal(
        &f.plan, &samples, &before, &after, &f.c, |_, p, _| Ok(grant(p)),
        |p, _| Ok(EvidencePolicy::at(p.anchor())),
    ).is_err());
    assert!(compose_production_portfolio_goal(
        &f.plan, &samples, &before, &after, &f.c, |_, p, _| Ok(grant(p)),
        |p, _| Ok(policy(p.snapshot())),
    ).is_err());
    assert!(!join(&f, &samples, 100)?.original_goal_proven);
    Ok(())
}

#[test]
fn existing_portfolio_independent_clock_custody_binds_actual_step_zero_and_every_original_key() -> Result<()> {
    let mut f = portfolio_fixture(4)?;
    for step in 0..4 { f.applied(step)?; }
    let archive = validate_plan(&f.plan)?;
    let journal = WorkOrderJournal::open(f.order_journal.clone(), &f.c, JournalMode::Offline, false)?.summary(&f.c)?.journal_id;
    let mut identities = BTreeSet::new();
    for index in 0..4 {
        let step = StepId::new(index);
        let result = f.executor.inspect_selected_order_custody(step, &f.c)?;
        let custody = SelectedOrderClockCustody::new(&f.plan, archive, journal, &result)?;
        assert_eq!(custody.step_id(), step);
        assert_eq!(custody.native_plan().key(), f.plan.steps[index as usize].idempotency_key);
        assert_eq!(custody.deadline_tick(), GameTick(220));
        assert!(identities.insert(custody.identity_bytes().to_vec()));
    }
    assert_eq!(f.orders.borrow().commits, 4);
    assert_eq!(&f.workforce.data.borrow().calls[1..], &[0; 4]);
    Ok(())
}

#[test]
fn existing_portfolio_lost_reply_reconciles_only_original_leaf_and_never_repeats_insertions() -> Result<()> {
    let mut f = portfolio_fixture(3)?;
    f.applied(0)?;
    f.orders.borrow_mut().lose_reply = true;
    let prepared = f.prepare(1)?;
    assert!(f.commit(1, prepared).is_err());
    assert!(f.prepare(2).is_err());
    let original = f.plan.clone();
    let mut f = f.reopen_query()?;
    let route = f.route(1)?;
    assert!(matches!(f.executor.recover_step(&f.plan, &f.plan.steps[1], &route, &f.c)?, RecoveredFamily::Applied(_)));
    assert_eq!(f.executor.original_plan(), &original);
    assert_eq!(f.orders.borrow().commits, 2);
    assert_eq!(&f.workforce.data.borrow().calls[1..], &[0; 4]);
    assert!(f.prepare(2).is_err());
    Ok(())
}

#[test]
fn existing_portfolio_history_reopen_keeps_original_deadline_and_withdraws_current_proof() -> Result<()> {
    let mut f = portfolio_fixture(2)?;
    f.applied(0)?;
    f.applied(1)?;
    let history = Storage::default();
    let mut first = portfolio_open(&mut f, 0, history.clone(), true)?;
    let mut second = portfolio_open(&mut f, 1, Storage::default(), true)?;
    set_tick(&mut f, 221, true)?;
    let a = portfolio_sample(&mut f, &mut first, 1, 101, 2)?;
    let b = portfolio_sample(&mut f, &mut second, 1, 103, 2)?;
    assert!(matches!(a.progress().obligation, Some(ObligationStatus::Failed { .. })));
    assert!(matches!(b.progress().obligation, Some(ObligationStatus::Failed { .. })));
    assert!(!join(&f, &[a, b], 100)?.original_goal_proven);
    let status = first.historical_obligation_status().cloned();
    drop(first);
    let mut reopened = portfolio_open(&mut f, 0, history, false)?;
    assert!(reopened.latest().is_none());
    assert_eq!(reopened.historical_obligation_status(), status.as_ref());
    assert_eq!(reopened.original_plan(), &f.plan);
    set_tick(&mut f, 226, true)?;
    let sample = portfolio_sample(&mut f, &mut reopened, 2, 201, 2)?;
    assert!(!sample.selected_goal_proven());
    assert!(!sample.original_goal_proven());
    assert!(matches!(sample.progress().obligation, Some(ObligationStatus::Failed { .. })));
    Ok(())
}
