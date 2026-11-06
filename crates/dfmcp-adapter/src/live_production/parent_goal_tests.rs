use super::*;
use crate::live_production::SelectedParentGoalResult;
use crate::semantic_workforce::WorkforceGoalProjection;
use crate::workforce_control::WorkforceCapture;

fn setup() -> Result<(ProductionFixture, SelectedWorkOrderGoalMonitor<Storage>)> {
    setup_with_history(Storage::default())
}

fn setup_with_history(
    history: Storage,
) -> Result<(ProductionFixture, SelectedWorkOrderGoalMonitor<Storage>)> {
    let mut f = inventory_fixture(1)?;
    f.c.budget.max_bytes = 8 * 1024 * 1024 * 1024;
    f.applied(0)?;
    f.applied(1)?;
    // The injected receipt encoder does not modify its independent observation
    // fixture. Publish the actual post-intervention source fence explicitly.
    f.workforce.data.borrow_mut().fixture.sequence += 1;
    let monitor = open(&mut f, 1, history, true)?;
    Ok((f, monitor))
}

fn sample_parent<A, W>(
    f: &mut ProductionFixture,
    monitor: &mut SelectedWorkOrderGoalMonitor<Storage>,
    sequence: u64,
    tick: u64,
    labor: bool,
    policies: (A, W),
) -> Result<SelectedParentGoalResult>
where
    A: FnMut(&WorkOrderGoalProjection, &OperationContext) -> Result<EvidencePolicy>,
    W: FnMut(&WorkforceGoalProjection, &OperationContext) -> Result<EvidencePolicy>,
{
    {
        let mut source = f.workforce.data.borrow_mut();
        source.fixture.tick = tick;
        source.fixture.citizens[0].labors[0] = u8::from(labor);
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
            |_| Ok(Operations(operations)),
        ),
        policies,
    )
}

fn workforce_grant(p: &WorkforceGoalProjection, _: &OperationContext) -> Result<EvidencePolicy> {
    Ok(policy(p.snapshot()))
}

#[test]
fn parent_truth_needs_current_labor_and_original_inventory_stability() -> Result<()> {
    let (mut f, mut monitor) = setup()?;
    let first = sample_parent(
        &mut f,
        &mut monitor,
        1,
        125,
        true,
        (|p, _| Ok(grant(p)), workforce_grant),
    )?;
    assert_eq!(first.parent_progress().current_truth, PredicateTruth::True);
    assert!(!first.original_goal_proven());
    assert!(!first.selected_goal_proven());
    assert_eq!(first.progress().plan_terminal, PredicateTruth::Unknown);
    let duplicate_tick = sample_parent(
        &mut f,
        &mut monitor,
        2,
        125,
        true,
        (|p, _| Ok(grant(p)), workforce_grant),
    )?;
    assert!(!duplicate_tick.original_goal_proven());
    retire(&f, 100)?;
    let achieved = sample_parent(
        &mut f,
        &mut monitor,
        3,
        130,
        true,
        (|p, _| Ok(grant(p)), workforce_grant),
    )?;
    assert!(achieved.original_goal_proven());
    assert!(achieved.selected_goal_proven());
    assert!(!achieved.progress().receipt_linked);
    assert!(matches!(
        achieved.progress().physical_work,
        WorkOrderPhysicalState::Unknown { .. }
    ));
    let parent = achieved.parent_progress();
    assert_eq!(parent.anchor.cursor, parent.selected_anchor.cursor);
    assert_eq!(parent.anchor.tick, parent.selected_anchor.tick);
    assert_ne!(parent.anchor.state_hash, parent.selected_anchor.state_hash);
    assert_ne!(parent.source_digest, achieved.progress().source_digest);
    let lost = sample_parent(
        &mut f,
        &mut monitor,
        4,
        135,
        false,
        (|p, _| Ok(grant(p)), workforce_grant),
    )?;
    assert_eq!(lost.parent_progress().current_truth, PredicateTruth::False);
    assert!(lost.selected_goal_proven());
    assert!(!lost.original_goal_proven());
    assert_eq!(f.orders.borrow().commits, 1);
    assert_eq!(f.workforce.data.borrow().calls[2], 1);
    Ok(())
}

#[test]
fn historical_labor_receipt_or_unqualified_labor_never_proves_parent() -> Result<()> {
    let (mut f, mut monitor) = setup()?;
    sample_parent(
        &mut f,
        &mut monitor,
        1,
        125,
        false,
        (|p, _| Ok(grant(p)), workforce_grant),
    )?;
    let current_false = sample_parent(
        &mut f,
        &mut monitor,
        2,
        130,
        false,
        (|p, _| Ok(grant(p)), workforce_grant),
    )?;
    assert!(current_false.selected_goal_proven());
    assert_eq!(
        current_false.parent_progress().current_truth,
        PredicateTruth::False
    );
    assert!(!current_false.original_goal_proven());
    let unknown = sample_parent(
        &mut f,
        &mut monitor,
        3,
        135,
        true,
        (
            |p, _| Ok(grant(p)),
            |p: &WorkforceGoalProjection, _| Ok(EvidencePolicy::at(p.anchor())),
        ),
    )?;
    assert_eq!(
        unknown.parent_progress().current_truth,
        PredicateTruth::Unknown
    );
    assert!(!unknown.original_goal_proven());
    Ok(())
}

#[test]
fn changing_labor_inside_inventory_bracket_withdraws_selected_sample() -> Result<()> {
    let (mut f, mut monitor) = setup()?;
    sample_parent(
        &mut f,
        &mut monitor,
        1,
        125,
        true,
        (|p, _| Ok(grant(p)), workforce_grant),
    )?;
    let source = Rc::clone(&f.workforce.data);
    let mut calls = 0;
    assert!(
        sample_parent(
            &mut f,
            &mut monitor,
            2,
            130,
            true,
            (
                |p, _| {
                    calls += 1;
                    if calls == 1 {
                        let mut source = source.borrow_mut();
                        source.fixture.citizens[0].labors[0] = 0;
                        source.fixture.sequence += 1;
                    }
                    Ok(grant(p))
                },
                workforce_grant
            )
        )
        .is_err()
    );
    assert!(monitor.latest().is_none());
    assert!(f.executor.high_tick() >= 130);
    assert_eq!(f.orders.borrow().commits, 1);
    assert_eq!(f.workforce.data.borrow().calls[2], 1);
    Ok(())
}

#[test]
fn parent_refuses_revoked_observe_before_native_contact_and_keeps_source_floors() -> Result<()> {
    let (mut f, mut monitor) = setup()?;
    sample_parent(
        &mut f,
        &mut monitor,
        1,
        125,
        true,
        (|p, _| Ok(grant(p)), workforce_grant),
    )?;
    let calls = f.workforce.data.borrow().calls;
    let reads = f.orders.borrow().reads;
    f.c.grants
        .retain(|grant| grant.capability != Capability::Observe);
    assert!(
        sample_parent(
            &mut f,
            &mut monitor,
            2,
            130,
            true,
            (|p, _| Ok(grant(p)), workforce_grant)
        )
        .is_err()
    );
    assert_eq!(calls, f.workforce.data.borrow().calls);
    assert_eq!(reads, f.orders.borrow().reads);
    assert!(monitor.latest().is_none());
    Ok(())
}

#[test]
fn parent_source_policy_cannot_change_or_modify_custody_before_publication() -> Result<()> {
    for corrupt in [false, true] {
        let (mut f, mut monitor) = setup()?;
        let journal = f.workforce.journal.clone();
        let mut calls = 0;
        assert!(
            sample_parent(
                &mut f,
                &mut monitor,
                1,
                125,
                true,
                (
                    |p, _| Ok(grant(p)),
                    |p: &WorkforceGoalProjection, _| {
                        calls += 1;
                        if calls == 3 {
                            if corrupt {
                                if let Some(byte) = journal.0.borrow_mut().file.get_mut().last_mut()
                                {
                                    *byte ^= 1;
                                }
                            } else {
                                return Err(invalid("withheld final parent source policy"));
                            }
                        }
                        Ok(policy(p.snapshot()))
                    }
                )
            )
            .is_err()
        );
        assert_eq!(calls, 3);
        assert!(monitor.latest().is_none());
        assert_eq!(f.orders.borrow().commits, 1);
        assert_eq!(f.workforce.data.borrow().calls[2], 1);
    }
    Ok(())
}

#[test]
fn parent_sample_does_not_restore_current_proof_from_inventory_history() -> Result<()> {
    let history = Storage::default();
    let (mut f, mut monitor) = setup_with_history(history.clone())?;
    sample_parent(
        &mut f,
        &mut monitor,
        1,
        125,
        true,
        (|p, _| Ok(grant(p)), workforce_grant),
    )?;
    let achieved = sample_parent(
        &mut f,
        &mut monitor,
        2,
        130,
        true,
        (|p, _| Ok(grant(p)), workforce_grant),
    )?;
    assert!(achieved.original_goal_proven());
    let first = monitor.first_satisfied_anchor();
    drop(monitor);
    let mut f = f.reopen(false)?;
    let mut monitor = open(&mut f, 1, history, false)?;
    assert!(monitor.latest().is_none());
    assert_eq!(monitor.first_satisfied_anchor(), first);
    assert!(first.is_some());
    let current_false = sample_parent(
        &mut f,
        &mut monitor,
        3,
        135,
        false,
        (|p, _| Ok(grant(p)), workforce_grant),
    )?;
    assert!(current_false.selected_goal_proven());
    assert!(!current_false.original_goal_proven());
    assert_eq!(
        current_false.parent_progress().current_truth,
        PredicateTruth::False
    );
    Ok(())
}

#[test]
fn current_parent_refuses_source_regression_and_unit_coverage_escalation() -> Result<()> {
    for regress in [false, true] {
        let (mut f, mut monitor) = setup()?;
        f.workforce.data.borrow_mut().fixture.sequence = 6;
        sample_parent(
            &mut f,
            &mut monitor,
            1,
            125,
            true,
            (|p, _| Ok(grant(p)), workforce_grant),
        )?;
        let reads = f.orders.borrow().reads;
        if regress {
            f.workforce.data.borrow_mut().fixture.sequence = 5;
        }
        assert!(
            sample_parent(
                &mut f,
                &mut monitor,
                2,
                130,
                true,
                (
                    |p, _| Ok(grant(p)),
                    |p: &WorkforceGoalProjection, _| {
                        let mut granted = policy(p.snapshot());
                        if !regress {
                            granted
                                .entity_kinds
                                .insert(EntityKind::Unit, EvidenceCoverage::Complete);
                        }
                        Ok(granted)
                    }
                )
            )
            .is_err()
        );
        assert_eq!(f.orders.borrow().reads, reads);
        assert!(monitor.latest().is_none());
        assert!(f.executor.high_tick() >= 130);
    }
    Ok(())
}

#[test]
fn identity_failure_after_native_capture_cannot_erase_source_lineage() -> Result<()> {
    let (mut f, mut monitor) = setup()?;
    sample_parent(
        &mut f,
        &mut monitor,
        1,
        125,
        true,
        (|p, _| Ok(grant(p)), workforce_grant),
    )?;
    {
        let mut source = f.workforce.data.borrow_mut();
        source.fixture.sequence = 10;
        source.observed_historical_id = Some(9999);
    }
    assert!(
        sample_parent(
            &mut f,
            &mut monitor,
            2,
            130,
            true,
            (|p, _| Ok(grant(p)), workforce_grant)
        )
        .is_err()
    );
    assert!(monitor.latest().is_none());
    assert!(f.executor.high_tick() >= 130);
    let calls = f.workforce.data.borrow().calls;
    {
        let mut source = f.workforce.data.borrow_mut();
        source.fixture.sequence = 7;
        source.observed_historical_id = None;
    }
    assert!(
        sample_parent(
            &mut f,
            &mut monitor,
            2,
            130,
            true,
            (|p, _| Ok(grant(p)), workforce_grant)
        )
        .is_err()
    );
    assert_eq!(
        f.workforce.data.borrow().calls,
        calls,
        "a lineage change fences the parent before another source read"
    );
    assert_eq!(f.orders.borrow().commits, 1);
    assert_eq!(f.workforce.data.borrow().calls[2], 1);
    Ok(())
}

#[test]
fn native_post_read_custody_failure_retains_consumed_parent_source_floor() -> Result<()> {
    let (mut f, mut monitor) = setup()?;
    sample_parent(
        &mut f,
        &mut monitor,
        1,
        125,
        true,
        (|p, _| Ok(grant(p)), workforce_grant),
    )?;
    let bytes = f.workforce.journal.0.borrow().file.get_ref().clone();
    let reads = f.orders.borrow().reads;
    {
        let mut source = f.workforce.data.borrow_mut();
        source.fixture.sequence = 10;
        source.corrupt_journal_on_observe = true;
    }
    assert!(
        sample_parent(
            &mut f,
            &mut monitor,
            2,
            130,
            true,
            (|p, _| Ok(grant(p)), workforce_grant)
        )
        .is_err()
    );
    assert!(monitor.latest().is_none());
    assert_eq!(f.orders.borrow().reads, reads);
    assert_eq!(monitor.high_tick(), GameTick(130));
    assert!(f.executor.high_tick() >= 130);
    assert_eq!(
        monitor
            .parent_workforce_floor
            .as_ref()
            .map(WorkforceCapture::sequence),
        Some(10),
        "the source floor survives a native-owner refusal after its reply"
    );
    // Restoring bytes cannot un-fence the existing owner or erase consumed
    // source clocks. Reopening is separate recovery with no parent proof.
    *f.workforce.journal.0.borrow_mut().file.get_mut() = bytes;
    {
        let mut source = f.workforce.data.borrow_mut();
        source.fixture.sequence = 7;
        source.corrupt_journal_on_observe = false;
    }
    let calls = f.workforce.data.borrow().calls;
    assert!(
        sample_parent(
            &mut f,
            &mut monitor,
            2,
            130,
            true,
            (|p, _| Ok(grant(p)), workforce_grant)
        )
        .is_err()
    );
    assert_eq!(f.workforce.data.borrow().calls, calls);
    assert_eq!(f.orders.borrow().commits, 1);
    assert_eq!(f.workforce.data.borrow().calls[2], 1);
    Ok(())
}
