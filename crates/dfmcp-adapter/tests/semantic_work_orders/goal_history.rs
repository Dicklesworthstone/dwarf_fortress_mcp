//! Actual semantic coordinator recovery: persisted observations never restore
//! control authority, current truth, or an unfinished observation streak.

use super::*;
use dfmcp_adapter::semantic_work_orders::{
    DurableWorkOrderGoalMonitor, GoalHistoryStore, SemanticWorkOrderReview,
    open_private_goal_history_store,
};

type DurableMonitor = DurableWorkOrderGoalMonitor<Memory>;

fn history_fixture() -> Result<(Fixture, Session, SemanticWorkOrderReview, Memory)> {
    let (mut fixture, session, review) = goal_fixture()?;
    // Replaying several complete source projections charges their full bounded
    // work, even though these concrete fixtures contain only one item.
    fixture.context.budget.max_bytes = 1024 * 1024 * 1024;
    let storage = Memory::new("goal-history", Rc::clone(&fixture.trace));
    Ok((fixture, session, review, storage))
}

fn journal_id(fixture: &Fixture) -> Result<Digest32> {
    let journal = WorkOrderJournal::open(
        fixture.journal.clone(),
        &fixture.context,
        JournalMode::Offline,
        false,
    )?;
    Ok(journal.summary(&fixture.context)?.journal_id)
}

fn open_history(
    fixture: &Fixture,
    review: &SemanticWorkOrderReview,
    storage: &Memory,
    initialize: bool,
    read_only: bool,
) -> Result<GoalHistoryStore<Memory>> {
    GoalHistoryStore::open(
        storage.clone(),
        journal_id(fixture)?,
        review,
        initialize,
        read_only,
        &fixture.context,
    )
}

fn begin(
    fixture: &Fixture,
    session: &mut Session,
    review: &SemanticWorkOrderReview,
    storage: &Memory,
    initialize: bool,
) -> Result<DurableMonitor> {
    session.begin_durable_goal_monitor(
        open_history(fixture, review, storage, initialize, false)?,
        review.seal(),
        &fixture.context,
        |projection, context| {
            assert!(context.anchor.tick >= projection.anchor().tick);
            Ok(grant(projection))
        },
    )
}

fn cursor(fixture: &Fixture, n: u64) -> ObservationCursor {
    ObservationCursor {
        epoch: fixture.original.anchor.cursor.epoch,
        sequence: fixture.original.anchor.cursor.sequence + n,
    }
}

#[allow(clippy::too_many_arguments)]
fn durable_poll<H, A>(
    fixture: &Fixture,
    session: &mut Session,
    monitor: &mut DurableWorkOrderGoalMonitor<H>,
    n: u64,
    tick: u64,
    remaining: i32,
    items: bool,
    jobs: bool,
    mut policy: A,
) -> Result<WorkOrderGoalResult>
where
    H: EffectJournalStorage,
    A: FnMut(&WorkOrderGoalProjection) -> Result<EvidencePolicy>,
{
    let observed = advance(fixture, tick)?;
    let expected = queue()?.next_order_id();
    let status = if remaining == 0 { 1 } else { 3 };
    let captures = vec![
        progress(&observed, expected, 2 * n - 1, remaining, 1, status)?,
        progress(&observed, expected, 2 * n, remaining, 1, status)?,
    ];
    let closed = Rc::new(Cell::new(0));
    let source = native_progress(captures, Rc::clone(&closed));
    let operations = OperationsFixture {
        raw: operations(&observed, items, jobs),
        closed: Rc::clone(&closed),
    };
    session.poll_original_goal_durable(
        monitor,
        cursor(fixture, n),
        &fixture.context,
        |_| Ok(source),
        |_| Ok(operations),
        |projection, context| {
            assert_eq!(closed.get(), 2);
            assert_eq!(context.anchor, projection.anchor());
            policy(projection)
        },
    )
}

#[test]
fn pure_invalidation_withholds_current_proof_and_the_next_durable_sample_retains_the_gap()
-> Result<()> {
    let (mut fixture, mut session, review, storage) = history_fixture()?;
    let mut monitor = begin(&fixture, &mut session, &review, &storage, true)?;
    let first = durable_poll(
        &fixture,
        &mut session,
        &mut monitor,
        1,
        10,
        0,
        true,
        false,
        |p| Ok(grant(p)),
    )?;
    assert_eq!(first.progress().current_goal, PredicateTruth::True);
    assert!(!first.original_goal_proven());
    let grants = fixture.context.grants.clone();
    fixture.context.grants.clear();
    let bytes = storage.stored_bytes();
    let trace = fixture.trace.borrow().clone();
    let cursor = monitor.high_cursor();
    monitor.invalidate_current_sample()?;
    assert!(monitor.latest().is_none());
    assert!(monitor.first_satisfied_anchor().is_none());
    assert_eq!(monitor.high_cursor(), cursor);
    assert_eq!(storage.stored_bytes(), bytes);
    assert_eq!(
        *fixture.trace.borrow(),
        trace,
        "invalidation performs no I/O"
    );
    fixture.context.grants = grants;

    let after_gap = durable_poll(
        &fixture,
        &mut session,
        &mut monitor,
        2,
        20,
        0,
        true,
        false,
        |p| Ok(grant(p)),
    )?;
    assert!(!after_gap.progress().continuous);
    assert!(!after_gap.original_goal_proven());
    drop(monitor);
    drop(session);

    let mut recovered = fixture.session(false, JournalMode::Reconcile, true)?;
    let review = recovered.reattach(fixture.original.clone(), &fixture.context)?;
    let mut monitor = begin(&fixture, &mut recovered, &review, &storage, false)?;
    assert!(monitor.latest().is_none());
    assert!(
        monitor.first_satisfied_anchor().is_none(),
        "replay must retain the gap between the two historical True samples"
    );
    assert!(!matches!(
        monitor.historical_obligation_status(),
        Some(ObligationStatus::Fulfilled { .. })
    ));
    let pending = durable_poll(
        &fixture,
        &mut recovered,
        &mut monitor,
        3,
        30,
        0,
        true,
        false,
        |p| Ok(grant(p)),
    )?;
    assert!(!pending.original_goal_proven());
    let fulfilled = durable_poll(
        &fixture,
        &mut recovered,
        &mut monitor,
        4,
        40,
        0,
        true,
        false,
        |p| Ok(grant(p)),
    )?;
    assert!(fulfilled.original_goal_proven());
    let proof = fulfilled.progress().anchor;
    let bytes = storage.stored_bytes();
    let trace = fixture.trace.borrow().clone();
    fixture.context.grants.clear();
    monitor.invalidate_current_sample()?;
    assert!(monitor.latest().is_none());
    assert_eq!(monitor.first_satisfied_anchor(), Some(proof));
    assert!(matches!(
        monitor.historical_obligation_status(),
        Some(ObligationStatus::Fulfilled { .. })
    ));
    assert_eq!(storage.stored_bytes(), bytes);
    assert_eq!(*fixture.trace.borrow(), trace);
    Ok(())
}

#[test]
fn retired_order_inventory_achievement_replays_exactly_without_current_truth_or_causation()
-> Result<()> {
    let (mut fixture, mut session, review) = inventory_fixture(1)?;
    fixture.context.budget.max_bytes = 1024 * 1024 * 1024;
    let storage = Memory::new("inventory-goal-history", Rc::clone(&fixture.trace));
    let mut monitor = session.begin_durable_goal_monitor(
        open_history(&fixture, &review, &storage, true, false)?,
        review.seal(),
        &fixture.context,
        |p, _| Ok(inventory_grant(p)),
    )?;
    let pending = durable_poll(
        &fixture,
        &mut session,
        &mut monitor,
        1,
        10,
        2,
        true,
        true,
        |p| Ok(inventory_grant(p)),
    )?;
    assert!(!pending.original_goal_proven());
    retire_original_order(&fixture)?;
    let retired = durable_poll(
        &fixture,
        &mut session,
        &mut monitor,
        2,
        20,
        0,
        true,
        false,
        |p| Ok(inventory_grant(p)),
    )?;
    assert!(retired.original_goal_proven());
    assert!(!retired.progress().receipt_linked);
    assert!(matches!(
        retired.progress().physical_work,
        WorkOrderPhysicalState::Unknown { .. }
    ));
    assert!(!retired.semantic_completion_proven());
    let proof = retired.progress().anchor;
    let bytes = storage.stored_bytes();
    drop(monitor);
    drop(session);

    fixture
        .context
        .grants
        .retain(|grant| matches!(grant.capability, Capability::Query | Capability::Observe));
    let mut recovered = fixture.session(false, JournalMode::Reconcile, true)?;
    let review = recovered.reattach(fixture.original.clone(), &fixture.context)?;
    let reads = fixture.native.borrow().reads;
    let checks = Cell::new(0);
    let mut monitor = recovered.begin_durable_goal_monitor(
        open_history(&fixture, &review, &storage, false, false)?,
        review.seal(),
        &fixture.context,
        |p, context| {
            checks.set(checks.get() + 1);
            assert!(context.anchor.tick >= proof.tick);
            Ok(inventory_grant(p))
        },
    )?;
    assert_eq!(
        checks.get(),
        4,
        "each stored sample requires two independent policy checks"
    );
    assert_eq!(fixture.native.borrow().reads, reads);
    assert_eq!(
        storage.stored_bytes(),
        bytes,
        "replay cannot rewrite history"
    );
    assert!(monitor.latest().is_none());
    assert_eq!(monitor.first_satisfied_anchor(), Some(proof));
    assert_eq!(monitor.last_observation_anchor(), Some(proof));
    assert_eq!(monitor.original_plan(), &fixture.original);
    assert_eq!(
        ObservedInventoryObjective::from_plan(monitor.original_plan()),
        Some(ObservedInventoryObjective {
            item_type: "BED".into(),
            min_stack_units: 1
        })
    );
    assert!(matches!(
        monitor.historical_obligation_status(),
        Some(ObligationStatus::Fulfilled { .. })
    ));

    let lost = durable_poll(
        &fixture,
        &mut recovered,
        &mut monitor,
        3,
        30,
        0,
        false,
        false,
        |p| Ok(inventory_grant(p)),
    )?;
    assert_eq!(lost.progress().current_goal, PredicateTruth::False);
    assert!(!lost.original_goal_proven());
    assert!(!lost.semantic_completion_proven());
    assert_eq!(lost.progress().first_satisfied_anchor, Some(proof));
    let last = lost.progress().anchor;
    drop(monitor);
    drop(recovered);

    let mut recovered = fixture.session(false, JournalMode::Reconcile, true)?;
    let review = recovered.reattach(fixture.original.clone(), &fixture.context)?;
    let bytes = storage.stored_bytes();
    let monitor = recovered.begin_durable_goal_monitor(
        open_history(&fixture, &review, &storage, false, true)?,
        review.seal(),
        &fixture.context,
        |p, _| Ok(inventory_grant(p)),
    )?;
    assert!(monitor.latest().is_none());
    assert_eq!(monitor.first_satisfied_anchor(), Some(proof));
    assert_eq!(monitor.last_observation_anchor(), Some(last));
    assert_eq!(storage.stored_bytes(), bytes);
    assert_eq!(fixture.native.borrow().prepares, 1);
    assert_eq!(fixture.native.borrow().commits, 1);
    assert_eq!(fixture.native.borrow().queries, 0);
    Ok(())
}

#[test]
fn inventory_history_requires_its_exact_original_policy_and_cannot_upgrade_unknown_samples()
-> Result<()> {
    let policy_for = |p: &WorkOrderGoalProjection, allow_items: bool| {
        let mut policy = inventory_grant(p);
        if !allow_items {
            policy.entity_kinds.remove(&EntityKind::Item);
            policy.sources.retain(|source| {
                !matches!(source,
                EvidenceSource::Observed { field, .. } if field.starts_with("operations/1.3.item."))
            });
        }
        policy
    };
    for originally_allowed in [false, true] {
        let (mut fixture, mut session, review) = inventory_fixture(1)?;
        fixture.context.budget.max_bytes = 1024 * 1024 * 1024;
        let storage = Memory::new("inventory-policy-history", Rc::clone(&fixture.trace));
        let mut monitor = session.begin_durable_goal_monitor(
            open_history(&fixture, &review, &storage, true, false)?,
            review.seal(),
            &fixture.context,
            |p, _| Ok(policy_for(p, originally_allowed)),
        )?;
        for n in 1..=2 {
            let result = durable_poll(
                &fixture,
                &mut session,
                &mut monitor,
                n,
                10 * n,
                0,
                true,
                false,
                |p| Ok(policy_for(p, originally_allowed)),
            )?;
            assert_eq!(
                result.progress().current_goal,
                if originally_allowed {
                    PredicateTruth::True
                } else {
                    PredicateTruth::Unknown
                }
            );
            assert_eq!(result.original_goal_proven(), originally_allowed && n == 2);
        }
        let proof = monitor.first_satisfied_anchor();
        drop(monitor);
        drop(session);
        let mut recovered = fixture.session(false, JournalMode::Reconcile, true)?;
        let review = recovered.reattach(fixture.original.clone(), &fixture.context)?;
        let bytes = storage.stored_bytes();
        let reads = fixture.native.borrow().reads;
        let changed = recovered.begin_durable_goal_monitor(
            open_history(&fixture, &review, &storage, false, true)?,
            review.seal(),
            &fixture.context,
            |p, _| Ok(policy_for(p, !originally_allowed)),
        );
        assert_eq!(
            changed.err().ok_or_else(rejected)?.code,
            ErrorCode::CapabilityDenied
        );
        assert_eq!(fixture.native.borrow().reads, reads);
        assert_eq!(storage.stored_bytes(), bytes);
        let exact = recovered.begin_durable_goal_monitor(
            open_history(&fixture, &review, &storage, false, true)?,
            review.seal(),
            &fixture.context,
            |p, _| Ok(policy_for(p, originally_allowed)),
        )?;
        assert!(exact.latest().is_none());
        assert_eq!(exact.first_satisfied_anchor(), proof);
        assert_eq!(
            matches!(
                exact.historical_obligation_status(),
                Some(ObligationStatus::Fulfilled { .. })
            ),
            originally_allowed
        );
        assert_eq!(storage.stored_bytes(), bytes);
    }
    Ok(())
}

#[test]
fn original_achievement_survives_two_reopens_while_current_false_and_unknown_remain_visible()
-> Result<()> {
    let (fixture, mut session, review, storage) = history_fixture()?;
    let mut monitor = begin(&fixture, &mut session, &review, &storage, true)?;
    assert!(monitor.first_satisfied_anchor().is_none());
    assert!(
        !durable_poll(
            &fixture,
            &mut session,
            &mut monitor,
            1,
            10,
            3,
            true,
            true,
            |p| { Ok(grant(p)) }
        )?
        .original_goal_proven()
    );
    let done = durable_poll(
        &fixture,
        &mut session,
        &mut monitor,
        2,
        20,
        0,
        true,
        false,
        |p| Ok(grant(p)),
    )?;
    assert!(done.semantic_completion_proven());
    let proof = done.progress().anchor;
    drop(monitor);
    drop(session);

    let mut recovered = fixture.session(false, JournalMode::Reconcile, true)?;
    let review = recovered.reattach(fixture.original.clone(), &fixture.context)?;
    let reads = fixture.native.borrow().reads;
    let mut monitor = begin(&fixture, &mut recovered, &review, &storage, false)?;
    assert_eq!(fixture.native.borrow().reads, reads);
    assert!(monitor.latest().is_none());
    assert_eq!(monitor.first_satisfied_anchor(), Some(proof));
    assert!(matches!(
        monitor.historical_obligation_status(),
        Some(ObligationStatus::Fulfilled { .. })
    ));
    assert_eq!(monitor.original_plan(), &fixture.original);
    let consumed = durable_poll(
        &fixture,
        &mut recovered,
        &mut monitor,
        3,
        30,
        0,
        false,
        false,
        |p| Ok(grant(p)),
    )?;
    assert_eq!(consumed.progress().current_goal, PredicateTruth::False);
    assert!(!consumed.original_goal_proven());
    assert_eq!(consumed.progress().first_satisfied_anchor, Some(proof));
    drop(monitor);
    drop(recovered);

    let mut recovered = fixture.session(false, JournalMode::Reconcile, true)?;
    let review = recovered.reattach(fixture.original.clone(), &fixture.context)?;
    let mut monitor = begin(&fixture, &mut recovered, &review, &storage, false)?;
    assert!(monitor.latest().is_none());
    assert_eq!(monitor.first_satisfied_anchor(), Some(proof));
    let unknown = durable_poll(
        &fixture,
        &mut recovered,
        &mut monitor,
        4,
        40,
        0,
        true,
        false,
        |p| Ok(EvidencePolicy::at(p.anchor())),
    )?;
    assert_eq!(unknown.progress().current_goal, PredicateTruth::Unknown);
    assert!(!unknown.semantic_completion_proven());
    assert_eq!(unknown.progress().first_satisfied_anchor, Some(proof));
    assert_eq!(fixture.native.borrow().prepares, 1);
    assert_eq!(fixture.native.borrow().commits, 1);
    assert_eq!(fixture.native.borrow().queries, 0);
    Ok(())
}

#[test]
fn recovered_pending_stability_resets_without_renewing_original_deadline_or_cadence() -> Result<()>
{
    for expired in [false, true] {
        let (fixture, mut session, review, storage) = history_fixture()?;
        let mut monitor = begin(&fixture, &mut session, &review, &storage, true)?;
        let before_tick = if expired { 90 } else { 10 };
        let first = durable_poll(
            &fixture,
            &mut session,
            &mut monitor,
            1,
            before_tick,
            0,
            true,
            false,
            |p| Ok(grant(p)),
        )?;
        assert!(!first.original_goal_proven());
        drop(monitor);
        drop(session);
        let mut recovered = fixture.session(false, JournalMode::Reconcile, true)?;
        let review = recovered.reattach(fixture.original.clone(), &fixture.context)?;
        let mut monitor = begin(&fixture, &mut recovered, &review, &storage, false)?;
        assert!(monitor.first_satisfied_anchor().is_none());
        assert_eq!(
            monitor.original_plan().steps[0].obligation,
            fixture.original.steps[0].obligation
        );
        let second = durable_poll(
            &fixture,
            &mut recovered,
            &mut monitor,
            2,
            if expired { 101 } else { 20 },
            0,
            true,
            false,
            |p| Ok(grant(p)),
        )?;
        assert!(!second.original_goal_proven());
        assert!(!second.progress().continuous);
        if expired {
            assert!(matches!(
                second.progress().obligation,
                Some(ObligationStatus::Failed { .. })
            ));
        } else {
            let same_tick = durable_poll(
                &fixture,
                &mut recovered,
                &mut monitor,
                3,
                20,
                0,
                true,
                false,
                |p| Ok(grant(p)),
            )?;
            assert!(!same_tick.original_goal_proven());
            let done = durable_poll(
                &fixture,
                &mut recovered,
                &mut monitor,
                4,
                30,
                0,
                true,
                false,
                |p| Ok(grant(p)),
            )?;
            assert!(done.semantic_completion_proven());
        }
    }
    Ok(())
}

#[test]
fn denied_final_publication_remains_an_interruption_after_restart() -> Result<()> {
    let (fixture, mut session, review, storage) = history_fixture()?;
    let mut monitor = begin(&fixture, &mut session, &review, &storage, true)?;
    durable_poll(
        &fixture,
        &mut session,
        &mut monitor,
        1,
        10,
        0,
        true,
        false,
        |p| Ok(grant(p)),
    )?;
    let mut checks = 0;
    assert!(
        durable_poll(
            &fixture,
            &mut session,
            &mut monitor,
            2,
            20,
            0,
            true,
            false,
            |p| {
                checks += 1;
                if checks == 2 {
                    Err(rejected())
                } else {
                    Ok(grant(p))
                }
            },
        )
        .is_err()
    );
    assert_eq!(checks, 2);
    assert!(monitor.latest().is_none());
    assert!(monitor.first_satisfied_anchor().is_none());
    drop(monitor);
    drop(session);
    let mut recovered = fixture.session(false, JournalMode::Reconcile, true)?;
    let review = recovered.reattach(fixture.original.clone(), &fixture.context)?;
    let mut monitor = begin(&fixture, &mut recovered, &review, &storage, false)?;
    assert!(
        !durable_poll(
            &fixture,
            &mut recovered,
            &mut monitor,
            3,
            30,
            0,
            true,
            false,
            |p| Ok(grant(p)),
        )?
        .original_goal_proven()
    );
    assert!(
        durable_poll(
            &fixture,
            &mut recovered,
            &mut monitor,
            4,
            40,
            0,
            true,
            false,
            |p| Ok(grant(p)),
        )?
        .semantic_completion_proven()
    );
    Ok(())
}

#[test]
fn historical_policy_is_reissued_exactly_twice_at_the_latest_authority_floor() -> Result<()> {
    let (mut fixture, mut session, review, storage) = history_fixture()?;
    let mut monitor = begin(&fixture, &mut session, &review, &storage, true)?;
    for n in 1..=2 {
        durable_poll(
            &fixture,
            &mut session,
            &mut monitor,
            n,
            10 * n,
            0,
            true,
            false,
            |p| Ok(grant(p)),
        )?;
    }
    let floor = GameTick(fixture.original.anchor.tick.get() + 20);
    drop(monitor);
    drop(session);
    let stored = storage.stored_bytes();
    for changed in 0..4 {
        let mut recovered = fixture.session(false, JournalMode::Reconcile, true)?;
        let review = recovered.reattach(fixture.original.clone(), &fixture.context)?;
        let history = open_history(&fixture, &review, &storage, false, true)?;
        let reads = fixture.native.borrow().reads;
        let mut policies = 0;
        let result = recovered.begin_durable_goal_monitor(
            history,
            review.seal(),
            &fixture.context,
            |p, c| {
                assert!(c.anchor.tick >= floor);
                policies += 1;
                let mut policy = grant(p);
                match changed {
                    0 => {
                        return Err(error(
                            ErrorCode::CapabilityDenied,
                            "historical issuer denied",
                        ));
                    }
                    1 => {
                        policy
                            .entity_kinds
                            .insert(EntityKind::Item, EvidenceCoverage::Observed);
                    }
                    2 => {
                        policy.sources.pop_first();
                    }
                    _ if policies == 2 => {
                        return Err(error(
                            ErrorCode::CapabilityDenied,
                            "final historical issuer denied",
                        ));
                    }
                    _ => {}
                }
                Ok(policy)
            },
        );
        assert!(result.is_err(), "changed policy {changed}");
        assert!(policies > 0);
        assert_eq!(fixture.native.borrow().reads, reads);
        assert_eq!(storage.stored_bytes(), stored);
    }
    for grant in &mut fixture.context.grants {
        if matches!(grant.capability, Capability::Observe | Capability::Query) {
            grant.expires_at_tick = Some(GameTick(fixture.original.anchor.tick.get() + 15));
        }
    }
    let mut recovered = fixture.session(false, JournalMode::Reconcile, true)?;
    let review = recovered.reattach(fixture.original.clone(), &fixture.context)?;
    let callbacks = Cell::new(0);
    let attempted = open_history(&fixture, &review, &storage, false, true).and_then(|history| {
        recovered.begin_durable_goal_monitor(history, review.seal(), &fixture.context, |p, _| {
            callbacks.set(callbacks.get() + 1);
            Ok(grant(p))
        })
    });
    assert!(attempted.is_err());
    assert_eq!(
        callbacks.get(),
        0,
        "retained ticks cannot revive expired authority"
    );
    assert_eq!(storage.stored_bytes(), stored);
    Ok(())
}

#[test]
fn failed_partial_reads_preserve_progress_and_operations_floors_across_reopen() -> Result<()> {
    for regression in [
        "progress_sequence",
        "remaining_counter",
        "operations_horizon",
    ] {
        let (fixture, mut session, review, storage) = history_fixture()?;
        let mut monitor = begin(&fixture, &mut session, &review, &storage, true)?;
        durable_poll(
            &fixture,
            &mut session,
            &mut monitor,
            1,
            10,
            3,
            true,
            false,
            |p| Ok(grant(p)),
        )?;
        let observed = advance(&fixture, 20)?;
        let id = review.native_plan().observation().next_order_id();
        let closed = Rc::new(Cell::new(0));
        // Operations have advanced their allocation horizon, but the second
        // progress read fails. Neither consumed source can be forgotten.
        let source = native_progress(
            vec![progress(&observed, id, 3, 2, 1, 3)?],
            Rc::clone(&closed),
        );
        let mut raw = operations(&observed, true, false);
        raw.next_item_id = 200;
        assert!(
            session
                .poll_original_goal_durable(
                    &mut monitor,
                    cursor(&fixture, 2),
                    &fixture.context,
                    |_| Ok(source),
                    |_| Ok(OperationsFixture { raw, closed }),
                    |p, _| Ok(grant(p)),
                )
                .is_err()
        );
        assert!(monitor.latest().is_none());
        assert_eq!(monitor.high_cursor(), cursor(&fixture, 2));
        drop(monitor);
        drop(session);

        let mut recovered = fixture.session(false, JournalMode::Reconcile, true)?;
        let review = recovered.reattach(fixture.original.clone(), &fixture.context)?;
        let mut monitor = begin(&fixture, &mut recovered, &review, &storage, false)?;
        let observed = advance(&fixture, 30)?;
        let before_sequence = if regression == "progress_sequence" {
            2
        } else {
            5
        };
        let remaining = if regression == "remaining_counter" {
            3
        } else {
            2
        };
        let captures = vec![
            progress(&observed, id, before_sequence, remaining, 1, 3)?,
            progress(&observed, id, before_sequence + 1, remaining, 1, 3)?,
        ];
        let closed = Rc::new(Cell::new(0));
        let source = native_progress(captures, Rc::clone(&closed));
        let mut raw = operations(&observed, true, false);
        if regression != "operations_horizon" {
            raw.next_item_id = 200;
        }
        assert!(
            recovered
                .poll_original_goal_durable(
                    &mut monitor,
                    cursor(&fixture, 3),
                    &fixture.context,
                    |_| Ok(source),
                    |_| Ok(OperationsFixture { raw, closed }),
                    |p, _| Ok(grant(p)),
                )
                .is_err(),
            "lost {regression}"
        );
        assert!(monitor.is_fenced(), "regression must fence {regression}");
        assert!(monitor.latest().is_none());
        assert!(monitor.first_satisfied_anchor().is_none());
        assert_eq!(fixture.native.borrow().commits, 1);
    }
    Ok(())
}

#[test]
fn broader_reissued_policy_cannot_turn_historical_unknown_samples_into_achievement() -> Result<()> {
    let (fixture, mut session, review, storage) = history_fixture()?;
    let mut monitor = begin(&fixture, &mut session, &review, &storage, true)?;
    for n in 1..=2 {
        let unknown = durable_poll(
            &fixture,
            &mut session,
            &mut monitor,
            n,
            10 * n,
            0,
            true,
            false,
            |p| Ok(EvidencePolicy::at(p.anchor())),
        )?;
        assert_eq!(unknown.progress().current_goal, PredicateTruth::Unknown);
        assert!(!unknown.original_goal_proven());
    }
    assert!(monitor.first_satisfied_anchor().is_none());
    drop(monitor);
    drop(session);
    let mut recovered = fixture.session(false, JournalMode::Reconcile, true)?;
    let review = recovered.reattach(fixture.original.clone(), &fixture.context)?;
    let history = open_history(&fixture, &review, &storage, false, true)?;
    let bytes = storage.stored_bytes();
    let reads = fixture.native.borrow().reads;
    let upgrade =
        recovered.begin_durable_goal_monitor(history, review.seal(), &fixture.context, |p, _| {
            Ok(grant(p))
        });
    assert_eq!(
        upgrade.err().ok_or_else(rejected)?.code,
        ErrorCode::CapabilityDenied
    );
    assert_eq!(storage.stored_bytes(), bytes);
    assert_eq!(fixture.native.borrow().reads, reads);
    let history = open_history(&fixture, &review, &storage, false, true)?;
    let exact = recovered.begin_durable_goal_monitor(
        history,
        review.seal(),
        &fixture.context,
        |p, _| Ok(EvidencePolicy::at(p.anchor())),
    )?;
    assert!(exact.latest().is_none());
    assert!(exact.first_satisfied_anchor().is_none());
    assert!(!matches!(
        exact.historical_obligation_status(),
        Some(ObligationStatus::Fulfilled { .. })
    ));
    Ok(())
}

#[test]
fn byte_authority_read_only_and_custody_refusals_contact_no_native_source() -> Result<()> {
    for mode in 0..7 {
        let (fixture, mut session, review, storage) = history_fixture()?;
        let history = open_history(&fixture, &review, &storage, true, false)?;
        let history = if mode == 4 {
            drop(history);
            open_history(&fixture, &review, &storage, false, true)?
        } else {
            history
        };
        let mut monitor = session.begin_durable_goal_monitor(
            history,
            review.seal(),
            &fixture.context,
            |p, _| Ok(grant(p)),
        )?;
        let mut context = fixture.context.clone();
        match mode {
            0 => context.budget.max_bytes = 4 * 1024 * 1024,
            1 => context.budget.max_entities = 4095,
            2 => context.budget.max_output_tokens = 1,
            3 => context
                .grants
                .retain(|g| g.capability != Capability::Observe),
            5 => storage.data.borrow_mut().valid = false,
            6 => {
                let next = storage.data.borrow().syncs + 1;
                storage.data.borrow_mut().fail_sync = Some(next);
            }
            _ => {}
        }
        let before = storage.stored_bytes();
        let reads = fixture.native.borrow().reads;
        let progress_contacts = Cell::new(0);
        let operations_contacts = Cell::new(0);
        let outcome = session.poll_original_goal_durable(
            &mut monitor,
            cursor(&fixture, 1),
            &context,
            |_| {
                progress_contacts.set(progress_contacts.get() + 1);
                Err::<ProgressFixture, _>(rejected())
            },
            |_| {
                operations_contacts.set(operations_contacts.get() + 1);
                Err::<OperationsFixture, _>(rejected())
            },
            |_, _| Err(rejected()),
        );
        assert!(outcome.is_err(), "refusal mode {mode}");
        assert_eq!(fixture.native.borrow().reads, reads, "refusal mode {mode}");
        assert_eq!(progress_contacts.get(), 0);
        assert_eq!(operations_contacts.get(), 0);
        assert!(monitor.latest().is_none());
        if mode != 6 {
            assert_eq!(storage.stored_bytes(), before);
        }
    }
    Ok(())
}

#[test]
fn complete_published_record_with_lost_sync_ack_recovers_only_as_historical_achievement()
-> Result<()> {
    let (fixture, mut session, review, storage) = history_fixture()?;
    let mut monitor = begin(&fixture, &mut session, &review, &storage, true)?;
    durable_poll(
        &fixture,
        &mut session,
        &mut monitor,
        1,
        10,
        0,
        true,
        false,
        |p| Ok(grant(p)),
    )?;
    // Started, queue, progress-before, operations, progress-after, queue,
    // Published. Fail acknowledgement of the complete final publication frame.
    let publication_sync = storage.data.borrow().syncs + 7;
    storage.data.borrow_mut().fail_sync = Some(publication_sync);
    assert!(
        durable_poll(
            &fixture,
            &mut session,
            &mut monitor,
            2,
            20,
            0,
            true,
            false,
            |p| Ok(grant(p)),
        )
        .is_err()
    );
    assert!(monitor.is_fenced());
    assert!(monitor.latest().is_none());
    assert!(monitor.first_satisfied_anchor().is_none());
    drop(monitor);
    drop(session);
    storage.data.borrow_mut().fail_sync = None;
    let mut recovered = fixture.session(false, JournalMode::Reconcile, true)?;
    let review = recovered.reattach(fixture.original.clone(), &fixture.context)?;
    let monitor = begin(&fixture, &mut recovered, &review, &storage, false)?;
    assert!(monitor.latest().is_none());
    let proof = monitor.first_satisfied_anchor().ok_or_else(rejected)?;
    assert_eq!(proof.cursor, cursor(&fixture, 2));
    assert_eq!(
        proof.tick,
        GameTick(fixture.original.anchor.tick.get() + 20)
    );
    assert!(matches!(
        monitor.historical_obligation_status(),
        Some(ObligationStatus::Fulfilled { .. })
    ));
    assert_eq!(fixture.native.borrow().commits, 1);
    Ok(())
}

#[test]
fn explicitly_interrupted_history_cannot_be_bypassed_by_restarting_the_monitor() -> Result<()> {
    let (fixture, mut session, review, storage) = history_fixture()?;
    let mut monitor = begin(&fixture, &mut session, &review, &storage, true)?;
    durable_poll(
        &fixture,
        &mut session,
        &mut monitor,
        1,
        10,
        0,
        true,
        false,
        |p| Ok(grant(p)),
    )?;
    let reads = fixture.native.borrow().reads;
    let old_len = storage.stored_bytes().len();
    monitor.observation_interrupted(&fixture.context)?;
    assert!(storage.stored_bytes().len() > old_len);
    assert_eq!(fixture.native.borrow().reads, reads);
    drop(monitor);
    let mut replacement = begin(&fixture, &mut session, &review, &storage, false)?;
    assert!(replacement.latest().is_none());
    let result = durable_poll(
        &fixture,
        &mut session,
        &mut replacement,
        2,
        20,
        0,
        true,
        false,
        |p| Ok(grant(p)),
    )?;
    assert!(!result.original_goal_proven());
    assert!(!result.progress().continuous);
    Ok(())
}

#[test]
fn original_unknown_creation_never_gains_a_receipt_from_durable_goal_achievement() -> Result<()> {
    let (mut fixture, mut session, review) = goal_fixture_with_state(WorkOrderState::Unknown)?;
    fixture.context.budget.max_bytes = 1024 * 1024 * 1024;
    let storage = Memory::new("goal-history", Rc::clone(&fixture.trace));
    let mut monitor = begin(&fixture, &mut session, &review, &storage, true)?;
    for n in 1..=2 {
        let result = durable_poll(
            &fixture,
            &mut session,
            &mut monitor,
            n,
            n * 10,
            0,
            true,
            false,
            |p| Ok(grant(p)),
        )?;
        assert_eq!(result.original_goal_proven(), n == 2);
        assert!(!result.semantic_completion_proven());
        assert!(!result.progress().receipt_linked);
    }
    let proof = monitor.first_satisfied_anchor();
    drop(monitor);
    drop(session);
    let mut recovered = fixture.session(false, JournalMode::Reconcile, true)?;
    let review = recovered.reattach(fixture.original.clone(), &fixture.context)?;
    let mut monitor = begin(&fixture, &mut recovered, &review, &storage, false)?;
    assert_eq!(monitor.first_satisfied_anchor(), proof);
    let result = durable_poll(
        &fixture,
        &mut recovered,
        &mut monitor,
        3,
        30,
        0,
        true,
        false,
        |p| Ok(grant(p)),
    )?;
    assert!(result.original_goal_proven());
    assert_eq!(result.creation_result(), &NativeOrderCreation::Unverified);
    assert!(!result.semantic_completion_proven());
    assert_eq!(fixture.native.borrow().commits, 1);
    assert_eq!(fixture.native.borrow().queries, 0);
    Ok(())
}

#[test]
fn later_receipt_reconciliation_cannot_retroactively_link_older_goal_projections() -> Result<()> {
    let mut fixture = Fixture::new()?;
    fixture.context.budget.max_bytes = 1024 * 1024 * 1024;
    let mut observe = fixture.context.grants[0].clone();
    observe.capability = Capability::Observe;
    fixture.context.grants.push(observe);
    let mut original = fixture.original.clone();
    original.terminal_condition = observed_goal();
    original.steps[0].postconditions = vec![observed_goal()];
    original.steps[0]
        .obligation
        .as_mut()
        .ok_or_else(rejected)?
        .terminal = observed_goal();
    fixture.original = reseal(original);
    fixture.native.borrow_mut().lose_commit = true;
    let mut session = fixture.session(true, JournalMode::Control, false)?;
    let review = session.observe(
        fixture.original.clone(),
        &mut fixture.observer,
        &fixture.context,
    )?;
    session.prepare(review.seal(), &mut fixture.observer, &fixture.context)?;
    let uncertain = session.commit(review.seal(), true, &mut fixture.observer, &fixture.context)?;
    assert_eq!(
        uncertain.native_record().state(),
        CreationState::Indeterminate
    );
    assert_eq!(
        uncertain.creation_result(),
        &NativeOrderCreation::Unverified
    );
    drop(session);
    fixture.native.borrow_mut().lose_commit = false;
    let mut session = fixture.session(false, JournalMode::Reconcile, true)?;
    let review = session.reattach(fixture.original.clone(), &fixture.context)?;
    let storage = Memory::new("goal-history", Rc::clone(&fixture.trace));
    let mut monitor = begin(&fixture, &mut session, &review, &storage, true)?;
    for n in 1..=2 {
        let result = durable_poll(
            &fixture,
            &mut session,
            &mut monitor,
            n,
            n * 10,
            0,
            true,
            false,
            |projection| {
                assert!(!projection.receipt_linked());
                assert!(
                    !projection
                        .snapshot()
                        .graph
                        .entities
                        .contains_key(&projection.original_order_entity())
                );
                Ok(grant(projection))
            },
        )?;
        assert_eq!(result.creation_result(), &NativeOrderCreation::Unverified);
        assert_eq!(result.original_goal_proven(), n == 2);
        assert!(!result.semantic_completion_proven());
    }
    let historical = monitor.first_satisfied_anchor();
    let reconciled = session.reconcile(review.seal(), &fixture.context)?;
    assert!(matches!(
        reconciled.creation_result(),
        NativeOrderCreation::Inserted { .. }
    ));
    drop(monitor);
    drop(session);

    let mut recovered = fixture.session(false, JournalMode::Reconcile, true)?;
    let review = recovered.reattach(fixture.original.clone(), &fixture.context)?;
    let history = open_history(&fixture, &review, &storage, false, false)?;
    let checks = Cell::new(0);
    let mut monitor = recovered.begin_durable_goal_monitor(
        history,
        review.seal(),
        &fixture.context,
        |projection, _| {
            checks.set(checks.get() + 1);
            assert!(
                !projection.receipt_linked(),
                "new Created receipt is not historical linkage"
            );
            assert!(
                !projection
                    .snapshot()
                    .graph
                    .entities
                    .contains_key(&projection.original_order_entity())
            );
            Ok(grant(projection))
        },
    )?;
    assert_eq!(checks.get(), 4);
    assert_eq!(monitor.first_satisfied_anchor(), historical);
    assert!(monitor.latest().is_none());
    let fresh = durable_poll(
        &fixture,
        &mut recovered,
        &mut monitor,
        3,
        30,
        0,
        true,
        false,
        |projection| {
            assert!(projection.receipt_linked());
            Ok(grant(projection))
        },
    )?;
    assert!(fresh.semantic_completion_proven());
    assert_eq!(fresh.progress().first_satisfied_anchor, historical);
    assert_eq!(fixture.native.borrow().queries, 1);
    assert_eq!(fixture.native.borrow().commits, 1);
    Ok(())
}

#[test]
fn unfinished_crash_prefix_retains_prior_achievement_and_fences_new_source_contact() -> Result<()> {
    let (fixture, mut session, review, storage) = history_fixture()?;
    let mut monitor = begin(&fixture, &mut session, &review, &storage, true)?;
    for n in 1..=2 {
        durable_poll(
            &fixture,
            &mut session,
            &mut monitor,
            n,
            n * 10,
            0,
            true,
            false,
            |p| Ok(grant(p)),
        )?;
    }
    let historical = monitor.first_satisfied_anchor();
    // A complete progress-before frame survives, while its sync acknowledgement
    // fails and the fenced writer cannot publish an interruption afterward.
    let partial_sync = storage.data.borrow().syncs + 3;
    storage.data.borrow_mut().fail_sync = Some(partial_sync);
    assert!(
        durable_poll(
            &fixture,
            &mut session,
            &mut monitor,
            3,
            30,
            0,
            true,
            false,
            |p| Ok(grant(p)),
        )
        .is_err()
    );
    assert!(monitor.is_fenced());
    drop(monitor);
    drop(session);
    storage.data.borrow_mut().fail_sync = None;
    let mut recovered = fixture.session(false, JournalMode::Reconcile, true)?;
    let review = recovered.reattach(fixture.original.clone(), &fixture.context)?;
    let mut monitor = begin(&fixture, &mut recovered, &review, &storage, false)?;
    assert!(monitor.is_fenced());
    assert!(monitor.latest().is_none());
    assert_eq!(monitor.first_satisfied_anchor(), historical);
    let reads = fixture.native.borrow().reads;
    let progress_contacts = Cell::new(0);
    let operations_contacts = Cell::new(0);
    assert!(
        recovered
            .poll_original_goal_durable(
                &mut monitor,
                cursor(&fixture, 4),
                &fixture.context,
                |_| {
                    progress_contacts.set(progress_contacts.get() + 1);
                    Err::<ProgressFixture, _>(rejected())
                },
                |_| {
                    operations_contacts.set(operations_contacts.get() + 1);
                    Err::<OperationsFixture, _>(rejected())
                },
                |_, _| Err(rejected()),
            )
            .is_err()
    );
    assert_eq!(fixture.native.borrow().reads, reads);
    assert_eq!(progress_contacts.get(), 0);
    assert_eq!(operations_contacts.get(), 0);
    Ok(())
}

#[test]
fn real_private_history_reopens_exact_proof_and_read_only_mode_never_publishes() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let (fixture, mut session, review, _) = history_fixture()?;
    let base =
        std::env::temp_dir().join(format!("dfmcp-native-order-goals-{}", std::process::id()));
    std::fs::create_dir(&base).map_err(|_| rejected())?;
    std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o700))
        .map_err(|_| rejected())?;
    let path = base.join("goals.bin");
    let journal = journal_id(&fixture)?;
    let history =
        open_private_goal_history_store(&path, journal, &review, true, false, &fixture.context)?;
    assert!(
        open_private_goal_history_store(&path, journal, &review, false, true, &fixture.context)
            .is_err()
    );
    let mut monitor =
        session.begin_durable_goal_monitor(history, review.seal(), &fixture.context, |p, _| {
            Ok(grant(p))
        })?;
    for n in 1..=2 {
        durable_poll(
            &fixture,
            &mut session,
            &mut monitor,
            n,
            n * 10,
            0,
            true,
            false,
            |p| Ok(grant(p)),
        )?;
    }
    let proof = monitor.first_satisfied_anchor();
    drop(monitor);
    drop(session);
    let bytes = std::fs::read(&path).map_err(|_| rejected())?;
    let mut recovered = fixture.session(false, JournalMode::Reconcile, true)?;
    let review = recovered.reattach(fixture.original.clone(), &fixture.context)?;
    let history =
        open_private_goal_history_store(&path, journal, &review, false, true, &fixture.context)?;
    assert!(history.is_read_only());
    let mut monitor = recovered.begin_durable_goal_monitor(
        history,
        review.seal(),
        &fixture.context,
        |p, _| Ok(grant(p)),
    )?;
    assert_eq!(monitor.first_satisfied_anchor(), proof);
    assert!(monitor.latest().is_none());
    let reads = fixture.native.borrow().reads;
    assert!(
        durable_poll(
            &fixture,
            &mut recovered,
            &mut monitor,
            3,
            30,
            0,
            true,
            false,
            |p| Ok(grant(p))
        )
        .is_err()
    );
    assert_eq!(fixture.native.borrow().reads, reads);
    assert_eq!(std::fs::read(&path).map_err(|_| rejected())?, bytes);
    drop(monitor);
    drop(recovered);
    std::fs::remove_file(&path).map_err(|_| rejected())?;
    std::fs::remove_dir(&base).map_err(|_| rejected())?;
    Ok(())
}

#[test]
fn torn_corrupt_foreign_and_rebound_history_never_repairs_or_imports_proof() -> Result<()> {
    let (fixture, mut session, review, storage) = history_fixture()?;
    let mut monitor = begin(&fixture, &mut session, &review, &storage, true)?;
    durable_poll(
        &fixture,
        &mut session,
        &mut monitor,
        1,
        10,
        0,
        true,
        false,
        |p| Ok(grant(p)),
    )?;
    drop(monitor);
    drop(session);
    let pristine = storage.stored_bytes();
    let mut corrupted = pristine.clone();
    let last = corrupted.last_mut().ok_or_else(rejected)?;
    *last ^= 1;
    for bytes in [
        Vec::new(),
        pristine[..pristine.len() - 1].to_vec(),
        corrupted,
        fixture.associations.stored_bytes(),
    ] {
        let copy = storage.with_stored_bytes(bytes.clone());
        assert!(open_history(&fixture, &review, &copy, false, true).is_err());
        assert_eq!(copy.stored_bytes(), bytes);
    }
    assert!(
        GoalHistoryStore::open(
            storage.clone(),
            Digest32::of_bytes(b"different native journal"),
            &review,
            false,
            true,
            &fixture.context,
        )
        .is_err()
    );
    assert_eq!(storage.stored_bytes(), pristine);
    Ok(())
}

#[test]
fn changed_source_identity_and_oversize_captures_remain_fenced_after_reopen() -> Result<()> {
    for changed in [
        "operations_software",
        "operations_world",
        "operations_site",
        "operations_generation",
        "oversize_operations",
        "before_world",
        "after_world",
        "before_site",
        "after_site",
        "before_generation",
        "after_generation",
    ] {
        let (fixture, mut session, review, storage) = history_fixture()?;
        let mut monitor = begin(&fixture, &mut session, &review, &storage, true)?;
        durable_poll(
            &fixture,
            &mut session,
            &mut monitor,
            1,
            10,
            3,
            true,
            false,
            |p| Ok(grant(p)),
        )?;
        let observed = advance(&fixture, 20)?;
        let id = review.native_plan().observation().next_order_id();
        let mut captures = vec![
            progress(&observed, id, 3, 2, 1, 3)?,
            progress(&observed, id, 4, 2, 1, 3)?,
        ];
        if changed.starts_with("before_") || changed.starts_with("after_") {
            let index = usize::from(changed.starts_with("after_"));
            let mut bytes = captures[index].canonical_bytes().to_vec();
            if changed.ends_with("generation") {
                bytes[8..16].copy_from_slice(&502u64.to_be_bytes());
            } else if changed.ends_with("site") {
                bytes[32..36].copy_from_slice(&(observed.site_id() + 1).to_be_bytes());
            } else {
                // Progress has a u16-length folder after its fixed47-byte prefix.
                bytes[47] = if bytes[47] == b'x' { b'y' } else { b'x' };
            }
            captures[index] = ProgressObservation::decode(&bytes, &[id])?;
        }
        let mut raw = operations(&observed, true, false);
        match changed {
            "operations_software" => raw.jobs.dfhack_version.push_str("-changed"),
            "operations_world" => raw.jobs.world_folder.push_str("-changed"),
            "operations_site" => raw.jobs.site_id += 1,
            "operations_generation" => raw.jobs.bridge_generation += 1,
            "oversize_operations" => {
                let item = raw.items[0].clone();
                raw.next_item_id = 10_000;
                raw.items = (2..4097)
                    .map(|native_id| {
                        let mut current = item.clone();
                        current.native_id = native_id;
                        current
                    })
                    .collect();
                raw.validate()?;
            }
            _ => {}
        }
        let closed = Rc::new(Cell::new(0));
        let source = native_progress(captures, Rc::clone(&closed));
        assert!(
            session
                .poll_original_goal_durable(
                    &mut monitor,
                    cursor(&fixture, 2),
                    &fixture.context,
                    |_| Ok(source),
                    |_| Ok(OperationsFixture { raw, closed }),
                    |p, _| Ok(grant(p)),
                )
                .is_err(),
            "source change {changed}"
        );
        assert!(monitor.is_fenced(), "source change {changed}");
        assert!(monitor.first_satisfied_anchor().is_none());
        drop(monitor);
        drop(session);
        let mut recovered = fixture.session(false, JournalMode::Reconcile, true)?;
        let review = recovered.reattach(fixture.original.clone(), &fixture.context)?;
        let mut monitor = begin(&fixture, &mut recovered, &review, &storage, false)?;
        assert!(monitor.is_fenced(), "recovered source change {changed}");
        assert!(monitor.latest().is_none());
        let reads = fixture.native.borrow().reads;
        let source_contacts = Cell::new(0);
        assert!(
            recovered
                .poll_original_goal_durable(
                    &mut monitor,
                    cursor(&fixture, 3),
                    &fixture.context,
                    |_| {
                        source_contacts.set(source_contacts.get() + 1);
                        Err::<ProgressFixture, _>(rejected())
                    },
                    |_| {
                        source_contacts.set(source_contacts.get() + 1);
                        Err::<OperationsFixture, _>(rejected())
                    },
                    |_, _| Err(rejected()),
                )
                .is_err()
        );
        assert_eq!(fixture.native.borrow().reads, reads, "reopened {changed}");
        assert_eq!(source_contacts.get(), 0, "reopened {changed}");
    }
    Ok(())
}

#[test]
fn a_replacement_monitor_cannot_lower_the_tick_consumed_by_a_failed_partial_read() -> Result<()> {
    let (fixture, mut session, review) = goal_fixture()?;
    let mut monitor = session.begin_goal_monitor(review.seal(), &fixture.context)?;
    poll(
        &fixture,
        &mut session,
        &mut monitor,
        1,
        10,
        4,
        true,
        false,
        |p| Ok(grant(p)),
    )?;
    let observed = advance(&fixture, 20)?;
    let newer = changed_clock(
        &observed,
        observed.generation(),
        observed.sequence(),
        observed.tick() + 10,
    )?;
    let id = review.native_plan().observation().next_order_id();
    let source = native_progress(
        vec![progress(&newer, id, 3, 2, 1, 3)?],
        Rc::new(Cell::new(0)),
    );
    assert!(
        session
            .poll_original_goal(
                &mut monitor,
                cursor(&fixture, 2),
                &fixture.context,
                |_| Ok(source),
                |_| Err::<OperationsFixture, _>(rejected()),
                |p, _| Ok(grant(p)),
            )
            .is_err()
    );
    assert!(
        !monitor.is_fenced(),
        "a tick mismatch only interrupts this source"
    );
    let reads = fixture.native.borrow().reads;
    for expired in [Capability::Query, Capability::Observe] {
        let mut context = fixture.context.clone();
        for grant in &mut context.grants {
            if grant.capability == expired {
                grant.expires_at_tick = Some(GameTick(fixture.original.anchor.tick.get() + 25));
            }
        }
        let replacement = session.begin_goal_monitor(review.seal(), &context);
        assert_eq!(
            replacement.err().ok_or_else(rejected)?.code,
            ErrorCode::CapabilityDenied
        );
        assert_eq!(fixture.native.borrow().reads, reads);
    }
    let mut replacement = session.begin_goal_monitor(review.seal(), &fixture.context)?;
    assert!(
        poll(
            &fixture,
            &mut session,
            &mut replacement,
            3,
            25,
            1,
            true,
            false,
            |p| Ok(grant(p)),
        )
        .is_err()
    );
    assert!(replacement.latest().is_none());
    assert!(replacement.first_satisfied_anchor().is_none());
    Ok(())
}
