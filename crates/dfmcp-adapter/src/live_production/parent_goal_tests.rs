use super::*;
use crate::live_production::SelectedParentGoalResult;
use crate::live_production::parent_goal::labor_findings;
use crate::semantic_workforce::WorkforceGoalProjection;
use crate::semantic_workforce::projection::LABOR_SOURCE;
use crate::workforce_control::WorkforceCapture;
use dfmcp_world::{EntityRecord, Fact, WorldGraph};

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
    let findings = &first.parent_progress().labor;
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].unit_id, EntityId::new(43));
    assert_eq!(findings[0].labor, "MINE");
    assert!(findings[0].requested_enabled);
    assert_eq!(findings[0].observed_enabled, Some(true));
    assert_eq!(findings[0].current_truth, PredicateTruth::True);
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
    assert_eq!(
        lost.parent_progress().labor[0].observed_enabled,
        Some(false)
    );
    assert_eq!(
        lost.parent_progress().labor[0].current_truth,
        PredicateTruth::False
    );
    assert_ne!(
        lost.parent_progress().workforce_source_digest,
        achieved.parent_progress().workforce_source_digest
    );
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
    assert_eq!(unknown.parent_progress().labor.len(), 1);
    assert_eq!(
        unknown.parent_progress().labor[0].unit_id,
        EntityId::new(43)
    );
    assert_eq!(unknown.parent_progress().labor[0].observed_enabled, None);
    assert_eq!(
        unknown.parent_progress().labor[0].current_truth,
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

fn finding_world(values: &[(u64, Option<Value>)]) -> (WorldSnapshot, EvidencePolicy) {
    let tick = GameTick(125);
    let source_digest = Digest32::of_bytes(b"exact test workforce capture");
    let mut graph = WorldGraph::default();
    for (id, value) in values {
        let id = EntityId::new(*id);
        let fields = value
            .iter()
            .map(|value| {
                (
                    "labor.MINE".to_owned(),
                    Fact::known(
                        value.clone(),
                        tick,
                        FactSource::DfhackField(LABOR_SOURCE.into()),
                        source_digest,
                    ),
                )
            })
            .collect();
        graph.entities.insert(
            id,
            EntityRecord {
                id,
                generation: 0,
                revision: 1,
                kind: EntityKind::Unit,
                label: format!("citizen {}", id.get()),
                fields,
            },
        );
    }
    let snapshot = WorldSnapshot::new(
        crate::workforce_control::fortress_id("region1", 4),
        tick,
        ObservationCursor {
            epoch: 1,
            sequence: 1,
        },
        true,
        graph,
    );
    let mut policy = EvidencePolicy::at(snapshot.anchor());
    policy
        .entity_kinds
        .insert(EntityKind::Unit, EvidenceCoverage::Observed);
    policy.sources.insert(EvidenceSource::Observed {
        field: LABOR_SOURCE.into(),
        source_digest,
    });
    (snapshot, policy)
}

#[test]
fn labor_findings_keep_disabled_requests_missing_facts_and_withheld_sources_distinct() -> Result<()>
{
    let (snapshot, granted) = finding_world(&[
        (1, Some(Value::Bool(true))),
        (2, Some(Value::Bool(false))),
        (3, Some(Value::Text("false".into()))),
        (4, None),
    ]);
    let action = Action::SetLabor {
        units: [99, 4, 2, 3, 1].map(EntityId::new).to_vec(),
        labor: "MINE".into(),
        enabled: false,
    };
    let findings = labor_findings(
        &action,
        &PredicateEvidence::scoped(&snapshot, granted.clone())?,
    )?;
    assert_eq!(
        findings
            .iter()
            .map(|row| row.unit_id.get())
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 99]
    );
    assert!(findings.iter().all(|row| !row.requested_enabled));
    assert_eq!(findings[0].observed_enabled, Some(true));
    assert_eq!(findings[0].current_truth, PredicateTruth::False);
    assert_eq!(findings[1].observed_enabled, Some(false));
    assert_eq!(findings[1].current_truth, PredicateTruth::True);
    assert!(findings[2..].iter().all(|row| {
        row.observed_enabled.is_none() && row.current_truth == PredicateTruth::Unknown
    }));
    for withheld in 0..3 {
        let mut policy = granted.clone();
        match withheld {
            0 => policy.sources.clear(),
            1 => policy.entity_kinds.clear(),
            _ => {
                policy.sources = BTreeSet::from([EvidenceSource::Observed {
                    field: LABOR_SOURCE.into(),
                    source_digest: Digest32::of_bytes(b"different workforce capture"),
                }]);
            }
        }
        let findings = labor_findings(&action, &PredicateEvidence::scoped(&snapshot, policy)?)?;
        assert!(findings.iter().all(|row| {
            row.observed_enabled.is_none() && row.current_truth == PredicateTruth::Unknown
        }));
    }
    Ok(())
}

#[test]
fn labor_findings_preserve_the_full_32_citizen_selection_in_numeric_order() -> Result<()> {
    let values = (1..=32)
        .map(|id| (id, Some(Value::Bool(id % 2 == 0))))
        .collect::<Vec<_>>();
    let (snapshot, policy) = finding_world(&values);
    let evidence = PredicateEvidence::scoped(&snapshot, policy)?;
    let action = Action::SetLabor {
        units: (1..=32).rev().map(EntityId::new).collect(),
        labor: "MINE".into(),
        enabled: true,
    };
    let findings = labor_findings(&action, &evidence)?;
    assert_eq!(findings.len(), 32);
    assert_eq!(
        findings
            .iter()
            .map(|row| row.unit_id.get())
            .collect::<Vec<_>>(),
        (1..=32).collect::<Vec<_>>()
    );
    for row in &findings {
        assert_eq!(row.labor, "MINE");
        assert!(row.requested_enabled);
        assert_eq!(row.observed_enabled, Some(row.unit_id.get() % 2 == 0));
        assert_eq!(
            row.current_truth,
            if row.unit_id.get() % 2 == 0 {
                PredicateTruth::True
            } else {
                PredicateTruth::False
            }
        );
    }
    assert_eq!(findings, labor_findings(&action, &evidence)?);
    Ok(())
}

#[test]
fn labor_findings_reject_empty_duplicate_nil_and_oversized_original_selections() -> Result<()> {
    let (snapshot, policy) = finding_world(&[(1, Some(Value::Bool(true)))]);
    let evidence = PredicateEvidence::scoped(&snapshot, policy)?;
    for units in [vec![], vec![0], vec![1, 1], (1..=33).collect()] {
        let action = Action::SetLabor {
            units: units.into_iter().map(EntityId::new).collect(),
            labor: "MINE".into(),
            enabled: true,
        };
        assert_eq!(
            labor_findings(&action, &evidence)
                .expect_err("invalid selection cannot be truncated or deduplicated")
                .code,
            ErrorCode::AdapterRejected
        );
    }
    for labor in [String::new(), "x".repeat(65), "MINE\0".into()] {
        let action = Action::SetLabor {
            units: vec![EntityId::new(1)],
            labor,
            enabled: true,
        };
        assert!(labor_findings(&action, &evidence).is_err());
    }
    Ok(())
}
