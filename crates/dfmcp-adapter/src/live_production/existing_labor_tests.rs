use super::*;
use crate::live_production::{
    ExistingLaborProjection, ExistingLaborRequirement, SelectedParentGoalResult,
};

fn existing_fixture() -> Result<ProductionFixture> {
    let mut f = inventory_fixture(1)?;
    f.c.budget.max_bytes = 8 * 1024 * 1024 * 1024;
    f.c.grants
        .retain(|grant| grant.capability != Capability::ConfigureLabor);
    f.workforce.context = f.c.clone();
    let mut order = f.plan.steps.remove(1);
    order.id = StepId::ZERO;
    order.depends_on.clear();
    order.idempotency_key =
        derive_step_idempotency_key(f.plan.intent_id, f.plan.anchor, order.id, &order.action);
    let requirement =
        ExistingLaborRequirement::new("CARPENTRY".to_owned(), vec![(EntityId::new(43), 142)])?;
    let terminal = order
        .obligation
        .as_ref()
        .ok_or_else(|| invalid("missing order obligation"))?
        .terminal
        .clone();
    f.plan.steps = vec![order];
    f.plan.terminal_condition =
        Predicate::All(vec![requirement.predicate(), terminal]).normalized();
    f.plan.required_capabilities = BTreeSet::from([Capability::ConfigureProduction]);
    f.plan.summary = "Use observed existing workers for one finite inventory goal".to_owned();
    reseal(&mut f.plan);
    {
        let mut native = f.workforce.data.borrow_mut();
        native.fixture.automatic = false;
        native.fixture.details[0].selected_only = false;
        native.fixture.details[0].labors = vec![1, 1];
        native.fixture.citizens[0].labors = vec![1, 1];
    }
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

fn sample_existing<A>(
    f: &mut ProductionFixture,
    monitor: &mut SelectedWorkOrderGoalMonitor<Storage>,
    sequence: u64,
    tick: u64,
    enabled: bool,
    change_during: bool,
    order_policy: A,
) -> Result<SelectedParentGoalResult>
where
    A: FnMut(&WorkOrderGoalProjection, &OperationContext) -> Result<EvidencePolicy>,
{
    {
        let mut source = f.workforce.data.borrow_mut();
        source.fixture.tick = tick;
        source.fixture.citizens[0].labors[1] = u8::from(enabled);
    }
    let old = f.orders.borrow().queue.clone();
    let mut raw = old.canonical_bytes().to_vec();
    raw[16..24].copy_from_slice(&(old.sequence() + 1).to_be_bytes());
    raw[24..32].copy_from_slice(&tick.to_be_bytes());
    let queue = WorkOrderObservation::decode(&raw)?;
    f.orders.borrow_mut().queue = queue.clone();
    let before = progress(&queue, 100, 2 * sequence)?;
    let after = progress(&queue, 100, 2 * sequence + 1)?;
    let operations = operations(&queue, 2);
    let (capsule, original) = observation(&[42])?;
    let identity = LiveRoutingEvidence::citizens_v1(
        PredicateEvidence::scoped(&original.snapshot, policy(&original.snapshot))?,
        &original,
        &capsule,
    )?;
    let source = Rc::clone(&f.workforce.data);
    f.executor.poll_selected_parent_goal(
        monitor,
        &identity,
        ObservationCursor {
            epoch: f.c.anchor.cursor.epoch,
            sequence,
        },
        &f.c,
        (
            |_| {
                Ok(Progress {
                    manifest: ProgressManifest {
                        generation: 501,
                        df_version: "0.51.11".into(),
                        dfhack_version: "0.51.11-r1".into(),
                    },
                    captures: vec![before, after],
                })
            },
            |_| {
                if change_during {
                    source.borrow_mut().fixture.citizens[0].labors[1] = u8::from(!enabled);
                }
                Ok(Operations(operations))
            },
        ),
        (order_policy, |_, _| {
            Err(invalid(
                "existing workers must not use an assignment review policy",
            ))
        }),
    )
}

#[test]
fn existing_labor_shape_is_complete_and_does_not_accept_partial_or_negative_requirements()
-> Result<()> {
    let f = existing_fixture()?;
    let expected =
        ExistingLaborRequirement::new("CARPENTRY".to_owned(), vec![(EntityId::new(43), 142)])?;
    assert_eq!(
        ExistingLaborRequirement::from_plan(&f.plan)?,
        Some(expected)
    );
    validate_plan(&f.plan)?;
    for field in [
        "labor.CARPENTRY",
        "historical_figure_id",
        "workforce_eligible",
    ] {
        let mut plan = f.plan.clone();
        if let Predicate::All(predicates) = &mut plan.terminal_condition {
            predicates.retain(|predicate| !matches!(predicate, Predicate::FieldCompare { field: name, .. } if name == field));
        }
        plan.terminal_condition = plan.terminal_condition.normalized();
        reseal(&mut plan);
        assert!(
            ExistingLaborRequirement::from_plan(&plan).is_err(),
            "missing {field}"
        );
        assert!(validate_plan(&plan).is_err());
    }
    let mut negative = f.plan.clone();
    if let Predicate::All(predicates) = &mut negative.terminal_condition {
        for predicate in predicates {
            if let Predicate::FieldCompare { field, value, .. } = predicate
                && field == "labor.CARPENTRY"
            {
                *value = Value::Bool(false);
            }
        }
    }
    negative.terminal_condition = negative.terminal_condition.normalized();
    reseal(&mut negative);
    assert!(ExistingLaborRequirement::from_plan(&negative).is_err());
    let mut negated = f.plan.clone();
    negated.terminal_condition = Predicate::Not(Box::new(negated.terminal_condition));
    reseal(&mut negated);
    assert!(ExistingLaborRequirement::from_plan(&negated).is_err());
    let mut deep = f.plan.clone();
    for _ in 0..64 {
        deep.terminal_condition = Predicate::Not(Box::new(deep.terminal_condition));
    }
    // No reseal: the bounded visitor must reject before the older validator
    // attempts to normalize or hash caller-supplied excessive structure.
    assert!(ExistingLaborRequirement::from_plan(&deep).is_err());
    let mut oversized = f.plan.clone();
    oversized.summary = "x".repeat(crate::live_production::MAX_PLAN_BYTES + 1);
    assert!(ExistingLaborRequirement::from_plan(&oversized).is_err());
    Ok(())
}

#[test]
fn already_enabled_broad_workers_insert_once_without_a_workforce_mutation_or_labor_grant()
-> Result<()> {
    let mut f = existing_fixture()?;
    let journal = f.workforce.journal.0.borrow().file.get_ref().clone();
    let associations = f.workforce.associations.0.borrow().file.get_ref().clone();
    f.applied(0)?;
    assert_eq!(f.orders.borrow().commits, 1);
    assert_eq!(&f.workforce.data.borrow().calls[1..], &[0; 4]);
    assert!(f.workforce.data.borrow().calls[0] >= 3);
    assert_eq!(*f.workforce.journal.0.borrow().file.get_ref(), journal);
    assert_eq!(
        *f.workforce.associations.0.borrow().file.get_ref(),
        associations
    );
    assert_eq!(f.executor.original_plan(), &f.plan);
    assert!(!f.executor.original_goal_proven());
    assert!(f.prepare(0).is_err());
    assert_eq!(f.orders.borrow().commits, 1);
    Ok(())
}

#[test]
fn existing_labor_is_rechecked_before_preparation_and_after_the_order_dispatch_marker() -> Result<()>
{
    for after_preparation in [false, true] {
        let mut f = existing_fixture()?;
        let prepared = after_preparation.then(|| f.prepare(0)).transpose()?;
        f.workforce.data.borrow_mut().fixture.citizens[0].labors[1] = 0;
        if let Some(prepared) = prepared {
            assert!(f.commit(0, prepared).is_err());
            let journal = WorkOrderJournal::open(
                Storage::from_bytes(f.order_journal.0.borrow().synced.clone()),
                &f.c,
                JournalMode::Offline,
                false,
            )?;
            let summary = journal.summary(&f.c)?;
            let page = journal.records_page(summary.head, None, 1, &f.c)?;
            assert_eq!(page.records[0].state(), CreationState::DispatchStarted);
            assert!(f.commit(0, prepared).is_err());
        } else {
            assert!(f.prepare(0).is_err());
            assert_eq!(f.orders.borrow().prepares, 0);
        }
        assert_eq!(f.orders.borrow().commits, 0);
        assert_eq!(&f.workforce.data.borrow().calls[1..], &[0; 4]);
    }
    Ok(())
}

#[test]
fn unknown_ineligible_and_replaced_existing_workers_cannot_authorize_an_order() -> Result<()> {
    for case in 0..4 {
        let mut f = existing_fixture()?;
        match case {
            0 => f.executor.factory.1.withhold = true,
            1 => f.executor.factory.1.deny = true,
            2 => f.workforce.data.borrow_mut().fixture.citizens[0].eligible = false,
            _ => f.workforce.data.borrow_mut().observed_historical_id = Some(777),
        }
        assert!(f.prepare(0).is_err());
        assert_eq!(f.orders.borrow().prepares, 0);
        assert_eq!(f.orders.borrow().commits, 0);
        assert_eq!(&f.workforce.data.borrow().calls[1..], &[0; 4]);
    }
    Ok(())
}

#[test]
fn existing_labor_policy_and_custody_are_checked_after_dispatch_is_synced() -> Result<()> {
    for case in 0..5 {
        let mut f = existing_fixture()?;
        let prepared = f.prepare(0)?;
        let next = f.executor.factory.1.calls + 1;
        let final_source = f.hooks.borrow().orders + 2;
        match case {
            0 => f.executor.factory.1.withdraw_at = Some(next),
            1 => f.executor.factory.1.corrupt_at = Some((next, f.workforce.journal.clone())),
            2 => f.hooks.borrow_mut().deny_at = Some(final_source),
            3 => {
                f.hooks.borrow_mut().corrupt_at = Some((final_source, f.workforce.journal.clone()))
            }
            _ => f.hooks.borrow_mut().labor_at = Some((final_source, f.workforce.clone())),
        }
        assert!(f.commit(0, prepared).is_err());
        assert_eq!(f.orders.borrow().commits, 0);
        assert_eq!(&f.workforce.data.borrow().calls[1..], &[0; 4]);
    }
    Ok(())
}

#[test]
fn existing_labor_parent_proof_survives_order_retirement_and_withdraws_on_worker_change()
-> Result<()> {
    let mut f = existing_fixture()?;
    f.applied(0)?;
    let mut monitor = open(&mut f, 0, Storage::default(), true)?;
    let first = sample_existing(&mut f, &mut monitor, 1, 125, true, false, |p, _| {
        Ok(grant(p))
    })?;
    assert_eq!(first.parent_progress().current_truth, PredicateTruth::True);
    assert!(!first.original_goal_proven());
    assert!(first.existing_labor_projection().is_some());
    retire(&f, 100)?;
    let done = sample_existing(&mut f, &mut monitor, 2, 130, true, false, |p, _| {
        Ok(grant(p))
    })?;
    assert!(done.original_goal_proven());
    assert!(!done.progress().receipt_linked);
    let achieved = monitor.first_satisfied_anchor();
    let lost = sample_existing(&mut f, &mut monitor, 3, 135, false, false, |p, _| {
        Ok(grant(p))
    })?;
    assert_eq!(lost.parent_progress().current_truth, PredicateTruth::False);
    assert!(!lost.original_goal_proven());
    assert!(lost.selected_goal_proven());
    assert_eq!(monitor.first_satisfied_anchor(), achieved);
    f.executor.factory.1.withhold = true;
    let unknown = sample_existing(&mut f, &mut monitor, 4, 140, true, false, |p, _| {
        Ok(grant(p))
    })?;
    assert_eq!(
        unknown.parent_progress().current_truth,
        PredicateTruth::Unknown
    );
    assert_eq!(unknown.parent_progress().labor[0].observed_enabled, None);
    assert!(!unknown.original_goal_proven());
    assert_eq!(monitor.first_satisfied_anchor(), achieved);
    assert_eq!(f.orders.borrow().commits, 1);
    assert_eq!(&f.workforce.data.borrow().calls[1..], &[0; 4]);
    Ok(())
}

#[test]
fn within_bracket_labor_changes_interrupt_original_stability_and_consume_source_floors()
-> Result<()> {
    let mut f = existing_fixture()?;
    f.plan.steps[0]
        .obligation
        .as_mut()
        .ok_or_else(|| invalid("missing obligation"))?
        .stable_for_observations = 3;
    reseal(&mut f.plan);
    let Executor {
        workforce,
        orders,
        evidence,
        factory,
        ..
    } = f.executor;
    f.executor = Executor::new(f.plan.clone(), workforce, orders, evidence, factory, &f.c)?;
    f.applied(0)?;
    let mut monitor = open(&mut f, 0, Storage::default(), true)?;
    sample_existing(&mut f, &mut monitor, 1, 125, true, false, |p, _| {
        Ok(grant(p))
    })?;
    assert!(
        sample_existing(&mut f, &mut monitor, 2, 130, true, true, |p, _| Ok(grant(
            p
        )))
        .is_err()
    );
    assert!(monitor.latest().is_none());
    assert!(monitor.high_tick() >= GameTick(130));
    let resumed = sample_existing(&mut f, &mut monitor, 3, 135, true, false, |p, _| {
        Ok(grant(p))
    })?;
    assert!(!resumed.original_goal_proven());
    assert!(matches!(
        resumed.progress().obligation,
        Some(ObligationStatus::Active {
            consecutive_stable_observations: 1,
            ..
        })
    ));
    let next = sample_existing(&mut f, &mut monitor, 4, 140, true, false, |p, _| {
        Ok(grant(p))
    })?;
    assert!(!next.original_goal_proven());
    let completed = sample_existing(&mut f, &mut monitor, 5, 145, true, false, |p, _| {
        Ok(grant(p))
    })?;
    assert!(completed.original_goal_proven());
    Ok(())
}

#[test]
fn existing_worker_identity_and_source_regressions_fence_future_sampling() -> Result<()> {
    for historical in [false, true] {
        let mut f = existing_fixture()?;
        f.applied(0)?;
        let mut monitor = open(&mut f, 0, Storage::default(), true)?;
        sample_existing(&mut f, &mut monitor, 1, 125, true, false, |p, _| {
            Ok(grant(p))
        })?;
        if historical {
            f.workforce.data.borrow_mut().observed_historical_id = Some(999);
        } else {
            f.workforce.data.borrow_mut().fixture.sequence = 2;
        }
        assert!(
            sample_existing(&mut f, &mut monitor, 2, 130, true, false, |p, _| Ok(grant(
                p
            )))
            .is_err()
        );
        assert!(monitor.parent_observation_fenced());
        let reads = f.workforce.data.borrow().calls[0];
        f.workforce.data.borrow_mut().observed_historical_id = None;
        f.workforce.data.borrow_mut().fixture.sequence = 3;
        assert!(
            sample_existing(&mut f, &mut monitor, 3, 135, true, false, |p, _| Ok(grant(
                p
            )))
            .is_err()
        );
        assert_eq!(f.workforce.data.borrow().calls[0], reads);
        assert_eq!(f.orders.borrow().commits, 1);
    }
    Ok(())
}

#[test]
fn lost_order_reply_recovers_the_original_key_without_assigning_existing_workers() -> Result<()> {
    let mut f = existing_fixture()?;
    f.orders.borrow_mut().lose_reply = true;
    let prepared = f.prepare(0)?;
    assert!(f.commit(0, prepared).is_err());
    assert_eq!(f.orders.borrow().commits, 1);
    let mut f = f.reopen_query()?;
    let route = f.route(0)?;
    let recovered = f
        .executor
        .recover_step(&f.plan, &f.plan.steps[0], &route, &f.c)?;
    assert!(matches!(recovered, RecoveredFamily::Applied(_)));
    assert_eq!(f.orders.borrow().commits, 1);
    assert_eq!(&f.workforce.data.borrow().calls[1..], &[0; 4]);
    assert!(f.prepare(0).is_err());
    Ok(())
}

#[test]
fn existing_labor_observation_and_reopening_never_renew_the_original_order_deadline() -> Result<()>
{
    let mut f = existing_fixture()?;
    f.applied(0)?;
    let history = Storage::default();
    let mut monitor = open(&mut f, 0, history.clone(), true)?;
    let late = sample_existing(&mut f, &mut monitor, 1, 221, true, false, |p, _| {
        Ok(grant(p))
    })?;
    assert_eq!(late.parent_progress().current_truth, PredicateTruth::True);
    assert!(!late.selected_goal_proven());
    assert!(!late.original_goal_proven());
    assert!(matches!(
        late.progress().obligation,
        Some(ObligationStatus::Failed { .. })
    ));
    assert!(monitor.first_satisfied_anchor().is_none());
    let history_state = monitor.historical_obligation_status().cloned();
    drop(monitor);
    let mut reopened = open(&mut f, 0, history, false)?;
    assert!(reopened.latest().is_none());
    assert_eq!(
        reopened.historical_obligation_status(),
        history_state.as_ref()
    );
    assert_eq!(reopened.original_plan(), &f.plan);
    let current = sample_existing(&mut f, &mut reopened, 2, 226, true, false, |p, _| {
        Ok(grant(p))
    })?;
    assert!(!current.original_goal_proven());
    assert!(matches!(
        current.progress().obligation,
        Some(ObligationStatus::Failed { .. })
    ));
    assert_eq!(f.orders.borrow().commits, 1);
    assert_eq!(&f.workforce.data.borrow().calls[1..], &[0; 4]);
    Ok(())
}

#[test]
fn existing_labor_projection_accepts_a_bounded_superset_but_never_grants_missing_sources()
-> Result<()> {
    let mut f = existing_fixture()?;
    let (capsule, canonical) = observation(&[42, 99])?;
    f.plan.anchor = canonical.snapshot.anchor();
    let step = &mut f.plan.steps[0];
    step.idempotency_key =
        derive_step_idempotency_key(f.plan.intent_id, f.plan.anchor, step.id, &step.action);
    reseal(&mut f.plan);
    let identity = LiveRoutingEvidence::citizens_v1(
        PredicateEvidence::scoped(&canonical.snapshot, policy(&canonical.snapshot))?,
        &canonical,
        &capsule,
    )?;
    let mut source = f.workforce.data.borrow().fixture.clone();
    source.citizens.push(CitizenFixture {
        id: 99,
        eligible: true,
        labors: vec![1, 1],
    });
    let capture = source.capture()?;
    let projection = ExistingLaborProjection::new(
        &f.plan,
        &identity,
        capture.clone(),
        canonical.snapshot.cursor,
    )?;
    assert_eq!(projection.snapshot().graph.entities.len(), 1);
    assert_eq!(projection.source_digest(), capture.witness());
    assert_eq!(projection.requirement().units(), vec![EntityId::new(43)]);
    assert_eq!(
        projection
            .evidence(policy(projection.snapshot()))?
            .evaluate(&projection.requirement().predicate())?,
        PredicateTruth::True
    );
    let unknown = EvidencePolicy::at(projection.anchor());
    assert_eq!(
        projection
            .evidence(unknown)?
            .evaluate(&projection.requirement().predicate())?,
        PredicateTruth::Unknown
    );
    let mut too_wide = policy(projection.snapshot());
    too_wide
        .entity_kinds
        .insert(EntityKind::Unit, EvidenceCoverage::Complete);
    assert!(projection.evidence(too_wide).is_err());
    Ok(())
}
