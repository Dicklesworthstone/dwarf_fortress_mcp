use super::*;

use crate::semantic_workforce::projection::LABOR_SOURCE;
use crate::semantic_workforce::{
    DurableWorkforceGoalMonitor, GoalHistoryStore, WorkforceGoalProjection, WorkforceGoalResult,
};
use dfmcp_intent::ObligationStatus;
use dfmcp_world::PredicateTruth;
use std::cell::Cell;

fn goal() -> Predicate {
    Predicate::FieldCompare {
        entity_id: EntityId::new(43),
        field: "labor.MINE".to_owned(),
        op: CompareOp::Eq,
        value: Value::Bool(true),
    }
}
fn goal_fixture(stable: Option<u32>) -> Result<Fixture> {
    let mut f = fixture()?;
    let mut grant = f.context.grants[0].clone();
    grant.capability = Capability::Observe;
    f.context.grants.push(grant);
    // Full replay prepays normalization and two independent checks per capture.
    f.context.budget.max_bytes = 1024 * 1024 * 1024;
    f.plan.terminal_condition = goal();
    f.plan.steps[0].postconditions = vec![goal()];
    f.plan.steps[0].obligation = stable.map(|stable| ObligationSpec {
        terminal: goal(),
        failure: None,
        deadline_tick: GameTick(150),
        poll_interval_ticks: 5,
        stable_for_observations: stable,
    });
    reseal(&mut f.plan);
    Ok(f)
}
fn c(f: &Fixture) -> OperationContext {
    let mut c = f.context.clone();
    c.grants
        .retain(|g| matches!(g.capability, Capability::Query | Capability::Observe));
    c
}
fn committed(f: &mut Fixture) -> Result<SemanticWorkforceReview> {
    let review = prepared(f)?;
    f.owner.commit(
        review.seal(),
        true,
        &mut f.observer,
        &f.context,
        |binding, current| f.native.connect(binding, current),
    )?;
    f.native.data.borrow_mut().fixture.sequence += 1;
    Ok(review)
}
fn source(f: &Fixture, tick: u64, enabled: bool) {
    let mut native = f.native.data.borrow_mut();
    native.fixture.tick = tick;
    native.fixture.citizens[0].labors[0] = u8::from(enabled);
}
fn historical_policy(
    p: &WorkforceGoalProjection,
    current: &OperationContext,
) -> Result<EvidencePolicy> {
    assert!(current.anchor.tick >= p.anchor().tick);
    assert_eq!(current.budget.max_bytes, 1024 * 1024);
    Ok(policy(p.snapshot()))
}
fn open(
    f: &mut Fixture,
    review: &SemanticWorkforceReview,
    storage: &Storage,
    initialize: bool,
    read_only: bool,
) -> Result<GoalHistoryStore<Storage>> {
    let current = c(f);
    let view = f.owner.inventory(&current)?;
    GoalHistoryStore::open(
        storage.clone(),
        view.id,
        review,
        initialize,
        read_only,
        &current,
    )
}
fn begin(
    f: &mut Fixture,
    review: &SemanticWorkforceReview,
    storage: &Storage,
    initialize: bool,
) -> Result<DurableWorkforceGoalMonitor<Storage>> {
    let current = c(f);
    let store = open(f, review, storage, initialize, false)?;
    f.owner.begin_durable_goal_monitor(
        store,
        review.seal(),
        &f.observer.routing_evidence()?,
        &current,
        historical_policy,
    )
}
fn poll(
    f: &mut Fixture,
    m: &mut DurableWorkforceGoalMonitor<Storage>,
    sequence: u64,
) -> Result<WorkforceGoalResult> {
    let current = c(f);
    f.owner.poll_original_goal_durable(
        m,
        ObservationCursor { epoch: 0, sequence },
        &current,
        |binding, current| f.native.connect(binding, current),
        historical_policy,
    )
}
fn stable(result: &WorkforceGoalResult) -> u32 {
    match result.progress().obligation {
        Some(ObligationStatus::Active {
            consecutive_stable_observations,
            ..
        }) => consecutive_stable_observations,
        _ => 0,
    }
}

#[test]
fn durable_replay_retains_verified_first_achievement_but_requires_fresh_current_truth() -> Result<()>
{
    let mut f = goal_fixture(Some(2))?;
    let review = committed(&mut f)?;
    let storage = Storage::default();
    let mut m = begin(&mut f, &review, &storage, true)?;
    source(&f, 125, true);
    assert_eq!(stable(&poll(&mut f, &mut m, 1)?), 1);
    source(&f, 130, true);
    let fulfilled = poll(&mut f, &mut m, 2)?;
    let first = fulfilled.progress().first_satisfied_anchor;
    assert!(fulfilled.semantic_completion_proven());
    assert_eq!(m.history_event_count(), 6);
    drop(m);
    let calls = f.native.data.borrow().calls;
    let mut m = begin(&mut f, &review, &storage, false)?;
    assert_eq!(f.native.data.borrow().calls, calls);
    assert_eq!(m.first_satisfied_anchor(), first);
    assert!(m.latest().is_none());
    assert!(matches!(
        m.historical_obligation_status(),
        Some(ObligationStatus::Fulfilled {
            fulfilled_at_tick: GameTick(130),
            ..
        })
    ));
    source(&f, 140, false);
    let lost = poll(&mut f, &mut m, 3)?;
    assert!(!lost.original_goal_proven());
    assert_eq!(lost.progress().current_goal, PredicateTruth::False);
    assert_eq!(lost.progress().first_satisfied_anchor, first);
    assert_eq!(&f.native.data.borrow().calls[1..], &calls[1..]);
    Ok(())
}

#[test]
fn recovered_pending_stability_resets_and_keeps_original_cadence_and_deadline() -> Result<()> {
    let mut f = goal_fixture(Some(2))?;
    let review = committed(&mut f)?;
    let storage = Storage::default();
    let mut m = begin(&mut f, &review, &storage, true)?;
    source(&f, 125, true);
    poll(&mut f, &mut m, 1)?;
    drop(m);
    let mut m = begin(&mut f, &review, &storage, false)?;
    assert!(m.first_satisfied_anchor().is_none());
    source(&f, 126, true);
    assert_eq!(stable(&poll(&mut f, &mut m, 2)?), 0);
    source(&f, 130, true);
    assert_eq!(stable(&poll(&mut f, &mut m, 3)?), 1);
    source(&f, 151, true);
    let late = poll(&mut f, &mut m, 4)?;
    assert!(!late.original_goal_proven());
    assert!(matches!(
        late.progress().obligation,
        Some(ObligationStatus::Failed {
            failed_at_tick: GameTick(151),
            ..
        })
    ));
    drop(m);
    let mut m = begin(&mut f, &review, &storage, false)?;
    source(&f, 155, true);
    assert!(!poll(&mut f, &mut m, 5)?.original_goal_proven());
    assert!(matches!(
        m.historical_obligation_status(),
        Some(ObligationStatus::Failed {
            failed_at_tick: GameTick(151),
            ..
        })
    ));
    Ok(())
}

#[test]
fn historical_policy_must_be_independently_reissued_without_retrospective_widening() -> Result<()> {
    let mut f = goal_fixture(None)?;
    let review = committed(&mut f)?;
    let storage = Storage::default();
    let mut m = begin(&mut f, &review, &storage, true)?;
    source(&f, 125, true);
    poll(&mut f, &mut m, 1)?;
    drop(m);
    let current = c(&f);
    let store = open(&mut f, &review, &storage, false, false)?;
    let calls = f.native.data.borrow().calls;
    assert!(
        f.owner
            .begin_durable_goal_monitor(
                store,
                review.seal(),
                &f.observer.routing_evidence()?,
                &current,
                |_, _| Err(error(
                    ErrorCode::CapabilityDenied,
                    "historical authority unavailable"
                ))
            )
            .is_err()
    );
    assert_eq!(f.native.data.borrow().calls, calls);
    let store = open(&mut f, &review, &storage, false, false)?;
    assert!(f.owner.begin_durable_goal_monitor(store, review.seal(), &f.observer.routing_evidence()?,
        &current, |p, current| { let mut out = historical_policy(p, current)?;
            out.sources.retain(|s| !matches!(s, EvidenceSource::Observed { field, .. } if field == LABOR_SOURCE)); Ok(out) }).is_err());

    let mut g = goal_fixture(None)?;
    let review = committed(&mut g)?;
    let storage = Storage::default();
    let mut m = begin(&mut g, &review, &storage, true)?;
    source(&g, 125, true);
    let current = c(&g);
    let unknown = g.owner.poll_original_goal_durable(
        &mut m,
        ObservationCursor {
            epoch: 0,
            sequence: 1,
        },
        &current,
        |binding, c| g.native.connect(binding, c),
        |p, c| {
            let mut out = historical_policy(p, c)?;
            out.sources.retain(
                |s| !matches!(s, EvidenceSource::Observed { field, .. } if field == LABOR_SOURCE),
            );
            Ok(out)
        },
    )?;
    assert_eq!(unknown.progress().current_goal, PredicateTruth::Unknown);
    drop(m);
    // Newly broader source grants cannot turn an originally Unknown sample into
    // historical success. The original fingerprint is comparison-only custody.
    assert!(begin(&mut g, &review, &storage, false).is_err());
    Ok(())
}

#[test]
fn denied_publication_retains_native_tick_and_reserved_cursor_across_restart() -> Result<()> {
    let mut f = goal_fixture(Some(2))?;
    let review = committed(&mut f)?;
    let storage = Storage::default();
    let mut m = begin(&mut f, &review, &storage, true)?;
    source(&f, 125, true);
    poll(&mut f, &mut m, 1)?;
    source(&f, 140, true);
    let current = c(&f);
    assert!(
        f.owner
            .poll_original_goal_durable(
                &mut m,
                ObservationCursor {
                    epoch: 0,
                    sequence: 2
                },
                &current,
                |binding, c| f.native.connect(binding, c),
                |_, _| Err(error(ErrorCode::CapabilityDenied, "revoked"))
            )
            .is_err()
    );
    assert_eq!(m.high_tick(), GameTick(140));
    assert_eq!(m.high_cursor().sequence, 2);
    drop(m);
    let mut m = begin(&mut f, &review, &storage, false)?;
    let calls = f.native.data.borrow().calls;
    assert!(poll(&mut f, &mut m, 2).is_err());
    assert_eq!(f.native.data.borrow().calls, calls);
    source(&f, 139, true);
    assert!(poll(&mut f, &mut m, 3).is_err());
    source(&f, 145, true);
    assert_eq!(stable(&poll(&mut f, &mut m, 4)?), 1);
    source(&f, 150, true);
    assert!(poll(&mut f, &mut m, 5)?.original_goal_proven());
    Ok(())
}

#[test]
fn current_observe_expiry_cannot_be_bypassed_by_replaying_old_ticks() -> Result<()> {
    let mut f = goal_fixture(None)?;
    let review = committed(&mut f)?;
    let storage = Storage::default();
    let mut m = begin(&mut f, &review, &storage, true)?;
    source(&f, 125, true);
    poll(&mut f, &mut m, 1)?;
    source(&f, 140, true);
    for grant in &mut f.context.grants {
        if grant.capability == Capability::Observe {
            grant.expires_at_tick = Some(GameTick(130));
        }
    }
    assert!(poll(&mut f, &mut m, 2).is_err());
    assert_eq!(m.high_tick(), GameTick(140));
    drop(m);
    let current = c(&f);
    let store = open(&mut f, &review, &storage, false, false)?;
    let issued = Cell::new(0);
    assert!(
        f.owner
            .begin_durable_goal_monitor(
                store,
                review.seal(),
                &f.observer.routing_evidence()?,
                &current,
                |p, c| {
                    issued.set(issued.get() + 1);
                    historical_policy(p, c)
                }
            )
            .is_err()
    );
    assert_eq!(issued.get(), 0);
    Ok(())
}

#[test]
fn crash_after_raw_capture_before_publication_never_restores_unsynced_achievement() -> Result<()> {
    let mut f = goal_fixture(Some(1))?;
    let review = committed(&mut f)?;
    let storage = Storage::default();
    let mut m = begin(&mut f, &review, &storage, true)?;
    source(&f, 125, true);
    let current = c(&f);
    let issued = Cell::new(0);
    assert!(
        f.owner
            .poll_original_goal_durable(
                &mut m,
                ObservationCursor {
                    epoch: 0,
                    sequence: 1
                },
                &current,
                |binding, c| f.native.connect(binding, c),
                |p, c| {
                    issued.set(issued.get() + 1);
                    if issued.get() == 2 {
                        storage.0.borrow_mut().fail_sync = true;
                    }
                    historical_policy(p, c)
                }
            )
            .is_err()
    );
    assert!(m.is_fenced());
    assert!(m.latest().is_none());
    assert!(m.first_satisfied_anchor().is_none());
    drop(m);
    storage.crash();
    let calls = f.native.data.borrow().calls;
    let mut m = begin(&mut f, &review, &storage, false)?;
    assert_eq!(m.high_cursor().sequence, 1);
    assert_eq!(m.high_tick(), GameTick(125));
    assert!(m.first_satisfied_anchor().is_none());
    assert_eq!(f.native.data.borrow().calls, calls);
    source(&f, 130, true);
    assert!(poll(&mut f, &mut m, 2)?.original_goal_proven());
    Ok(())
}

#[test]
fn interrupted_capture_and_complete_unknown_native_history_never_authorize_dispatch() -> Result<()>
{
    let mut f = goal_fixture(None)?;
    f.native.data.borrow_mut().unknown = true;
    let review = committed(&mut f)?;
    let storage = Storage::default();
    let mut m = begin(&mut f, &review, &storage, true)?;
    source(&f, 125, true);
    let true_goal = poll(&mut f, &mut m, 1)?;
    assert!(true_goal.original_goal_proven());
    assert!(!true_goal.semantic_completion_proven());
    drop(m);
    let calls = f.native.data.borrow().calls;
    let mut m = begin(&mut f, &review, &storage, false)?;
    source(&f, 130, true);
    let current = poll(&mut f, &mut m, 2)?;
    assert!(current.original_goal_proven());
    assert!(!current.semantic_completion_proven());
    assert_eq!(current.native_phase(), Some(AssignmentPhase::Unknown));
    assert_eq!(&f.native.data.borrow().calls[1..], &calls[1..]);
    Ok(())
}

#[test]
fn changed_history_bytes_and_final_historical_policy_are_refused() -> Result<()> {
    let mut f = goal_fixture(None)?;
    let review = committed(&mut f)?;
    let storage = Storage::default();
    let mut m = begin(&mut f, &review, &storage, true)?;
    source(&f, 125, true);
    poll(&mut f, &mut m, 1)?;
    drop(m);
    let bytes = storage.0.borrow().synced.clone();
    let current = c(&f);
    let store = open(&mut f, &review, &storage, false, false)?;
    let issued = Cell::new(0);
    assert!(
        f.owner
            .begin_durable_goal_monitor(
                store,
                review.seal(),
                &f.observer.routing_evidence()?,
                &current,
                |p, c| {
                    issued.set(issued.get() + 1);
                    if issued.get() == 2 {
                        storage.0.borrow_mut().file.get_mut()[0] ^= 1;
                    }
                    historical_policy(p, c)
                }
            )
            .is_err()
    );
    storage.0.borrow_mut().file = Cursor::new(bytes.clone());
    let store = open(&mut f, &review, &storage, false, false)?;
    let issued = Cell::new(0);
    assert!(
        f.owner
            .begin_durable_goal_monitor(
                store,
                review.seal(),
                &f.observer.routing_evidence()?,
                &current,
                |p, c| {
                    issued.set(issued.get() + 1);
                    if issued.get() == 2 {
                        Err(error(
                            ErrorCode::CapabilityDenied,
                            "final authority changed",
                        ))
                    } else {
                        historical_policy(p, c)
                    }
                }
            )
            .is_err()
    );
    for cut in [0, 1, 111, bytes.len() - 1] {
        let corrupt = Storage::from_bytes(bytes[..cut].to_vec());
        assert!(open(&mut f, &review, &corrupt, false, false).is_err());
    }
    let wrong = Digest32::of_bytes(b"another native journal");
    assert!(
        GoalHistoryStore::open(storage.clone(), wrong, &review, false, true, &current).is_err()
    );
    Ok(())
}

#[test]
fn explicit_interruption_is_durable_and_low_budget_refuses_before_source_contact() -> Result<()> {
    let mut f = goal_fixture(Some(2))?;
    let review = committed(&mut f)?;
    let storage = Storage::default();
    let mut m = begin(&mut f, &review, &storage, true)?;
    source(&f, 125, true);
    poll(&mut f, &mut m, 1)?;
    m.observation_interrupted(&c(&f))?;
    source(&f, 130, true);
    assert_eq!(stable(&poll(&mut f, &mut m, 2)?), 1);
    let mut current = c(&f);
    current.budget.max_bytes = 1024 * 1024;
    let before = f.native.data.borrow().calls;
    let bytes = storage.0.borrow().synced.len();
    assert!(
        f.owner
            .poll_original_goal_durable(
                &mut m,
                ObservationCursor {
                    epoch: 0,
                    sequence: 3
                },
                &current,
                |binding, c| f.native.connect(binding, c),
                historical_policy
            )
            .is_err()
    );
    assert_eq!(f.native.data.borrow().calls, before);
    assert_eq!(storage.0.borrow().synced.len(), bytes);
    let mut current = c(&f);
    current.budget.max_entities = 1;
    assert!(
        f.owner
            .poll_original_goal_durable(
                &mut m,
                ObservationCursor {
                    epoch: 0,
                    sequence: 3
                },
                &current,
                |binding, c| f.native.connect(binding, c),
                historical_policy
            )
            .is_err()
    );
    assert_eq!(f.native.data.borrow().calls, before);
    assert_eq!(storage.0.borrow().synced.len(), bytes);
    source(&f, 135, true);
    assert_eq!(stable(&poll(&mut f, &mut m, 3)?), 1);
    source(&f, 140, true);
    let achieved = poll(&mut f, &mut m, 4)?;
    assert!(achieved.original_goal_proven());
    assert_eq!(
        achieved.progress().first_satisfied_anchor.map(|a| a.tick),
        Some(GameTick(140))
    );
    drop(m);
    let m = begin(&mut f, &review, &storage, false)?;
    // The rejected sample had no budget to append an interruption immediately.
    // The next Started event must still preserve that lost stability sample.
    assert_eq!(
        m.first_satisfied_anchor().map(|a| a.tick),
        Some(GameTick(140))
    );
    Ok(())
}

#[test]
fn query_observe_only_native_recovery_replays_history_without_control_grants() -> Result<()> {
    let mut f = goal_fixture(Some(1))?;
    let review = committed(&mut f)?;
    let storage = Storage::default();
    let mut m = begin(&mut f, &review, &storage, true)?;
    source(&f, 125, true);
    let proven = poll(&mut f, &mut m, 1)?;
    drop(m);
    let current = c(&f);
    let journal = WorkforceJournal::open(
        f.native.journal.clone(),
        &current,
        WorkforceMode::Recover,
        None,
    )?;
    let (session, view) = WorkforceSession::new(journal, &current)?;
    let association = AssociationStore::open(
        f.native.associations.clone(),
        view.id,
        false,
        true,
        &current,
    )?;
    let mut recovered = SemanticWorkforceSession::new(session, association, &current)?;
    let reattached = recovered.reattach(f.plan.clone(), &current)?;
    let history = GoalHistoryStore::open(
        storage.clone(),
        view.id,
        &reattached,
        false,
        false,
        &current,
    )?;
    let calls = f.native.data.borrow().calls;
    let mut m = recovered.begin_durable_goal_monitor(
        history,
        reattached.seal(),
        &f.observer.routing_evidence()?,
        &current,
        historical_policy,
    )?;
    assert_eq!(
        m.first_satisfied_anchor(),
        proven.progress().first_satisfied_anchor
    );
    assert_eq!(f.native.data.borrow().calls, calls);
    source(&f, 130, true);
    let current_goal = recovered.poll_original_goal_durable(
        &mut m,
        ObservationCursor {
            epoch: 0,
            sequence: 2,
        },
        &current,
        |binding, c| f.native.connect(binding, c),
        historical_policy,
    )?;
    assert!(current_goal.semantic_completion_proven());
    assert_eq!(&f.native.data.borrow().calls[1..], &calls[1..]);
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn private_goal_file_retains_real_bytes_and_read_only_recovery_cannot_publish() -> Result<()> {
    use crate::semantic_workforce::open_private_goal_history_store;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let mut f = goal_fixture(Some(1))?;
    let review = committed(&mut f)?;
    let current = c(&f);
    let id = f.owner.inventory(&current)?.id;
    let directory = std::env::temp_dir().join(format!(
        "dfmcp-workforce-goal-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let path = directory.join("goal.history");
    let io = |_: std::io::Error| invalid("private goal history test I/O failed");
    std::fs::create_dir(&directory).map_err(io)?;
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).map_err(io)?;
    let history = open_private_goal_history_store(&path, id, &review, true, false, &current)?;
    assert!(open_private_goal_history_store(&path, id, &review, false, true, &current).is_err());
    let mut m = f.owner.begin_durable_goal_monitor(
        history,
        review.seal(),
        &f.observer.routing_evidence()?,
        &current,
        historical_policy,
    )?;
    source(&f, 125, true);
    let achieved = f.owner.poll_original_goal_durable(
        &mut m,
        ObservationCursor {
            epoch: 0,
            sequence: 1,
        },
        &current,
        |binding, c| f.native.connect(binding, c),
        historical_policy,
    )?;
    let first = achieved.progress().first_satisfied_anchor;
    drop(m);
    let bytes = std::fs::read(&path).map_err(io)?;
    let history = open_private_goal_history_store(&path, id, &review, false, true, &current)?;
    assert!(history.is_read_only());
    let mut m = f.owner.begin_durable_goal_monitor(
        history,
        review.seal(),
        &f.observer.routing_evidence()?,
        &current,
        historical_policy,
    )?;
    assert_eq!(m.first_satisfied_anchor(), first);
    assert!(m.latest().is_none());
    let calls = f.native.data.borrow().calls;
    assert!(
        f.owner
            .poll_original_goal_durable(
                &mut m,
                ObservationCursor {
                    epoch: 0,
                    sequence: 2
                },
                &current,
                |binding, c| f.native.connect(binding, c),
                historical_policy
            )
            .is_err()
    );
    assert_eq!(f.native.data.borrow().calls, calls);
    assert_eq!(std::fs::read(&path).map_err(io)?, bytes);
    drop(m);
    assert!(
        open_private_goal_history_store(
            &directory.join("missing"),
            id,
            &review,
            false,
            true,
            &current
        )
        .is_err()
    );
    assert!(open_private_goal_history_store(&path, id, &review, true, false, &current).is_err());
    let link = directory.join("link");
    symlink(&path, &link).map_err(io)?;
    assert!(open_private_goal_history_store(&link, id, &review, false, true, &current).is_err());
    std::fs::remove_file(&link).map_err(io)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).map_err(io)?;
    assert!(open_private_goal_history_store(&path, id, &review, false, true, &current).is_err());
    std::fs::remove_file(&path).map_err(io)?;
    std::fs::remove_dir(&directory).map_err(io)?;
    Ok(())
}
