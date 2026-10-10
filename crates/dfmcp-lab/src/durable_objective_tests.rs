//! Original-goal durability must not depend on whether action handles remain.

use super::*;
use dfmcp_world::WorldGraph;

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-objectives-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id(),
        ));
        let _ = fs::remove_dir_all(&path);
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn snapshot(fortress: u64, tick: u64) -> WorldSnapshot {
    WorldSnapshot::new(
        FortressId::new(fortress),
        GameTick(tick),
        ObservationCursor {
            epoch: 1,
            sequence: tick,
        },
        false,
        WorldGraph::default(),
    )
}

fn source() -> DurablePlanSource {
    DurablePlanSource::Production {
        summary: "retain both original quotas".to_owned(),
        raw: r#"{"quotas":[{"item":"DRINK","minimum":60},{"item":"FOOD","minimum":50}],"template":"production"}"#.to_owned(),
    }
}

fn digest(number: u64) -> Digest32 {
    Digest32::of_bytes(&number.to_le_bytes())
}

fn admit(store: &mut DurableLabStore, sealed: &WorldSnapshot, digest: Digest32) -> Result<()> {
    store.persist_objective_commit(sealed, digest, 41, source(), SessionId::new(9), &[])
}

fn objective(store: &DurableLabStore, digest: Digest32) -> Result<DurableObjective> {
    store
        .objective(FortressId::new(7), digest)
        .cloned()
        .ok_or_else(|| corrupt("test lost original objective"))
}

fn update(plan_digest: Digest32, state: &str) -> DurableStepUpdate {
    DurableStepUpdate {
        plan_digest,
        step: 1,
        state: state.to_owned(),
    }
}

#[test]
fn retired_actions_keep_the_unfulfilled_original_goal_across_compaction() -> Result<()> {
    let dir = TempDir::new("retired-unfulfilled");
    let sealed = snapshot(7, 10);
    let finished = snapshot(7, 20);
    let current = snapshot(7, 30);
    let plan = digest(1);
    let legacy = digest(2);
    {
        let mut store = DurableLabStore::open(&dir.0)?;
        admit(&mut store, &sealed, plan)?;
        store.persist_commit(&sealed, legacy, 42, source())?;
        store.persist_progress(
            "empty",
            &finished,
            &[update(plan, "verified"), update(legacy, "verified")],
            &[plan, legacy],
        )?;
        store.persist_head("empty", &current)?;
        assert_eq!(store.commits(sealed.fortress_id).count(), 0);
        assert!(objective(&store, plan)?.first_satisfied_anchor.is_none());
        assert!(store.objective(sealed.fortress_id, legacy).is_none());
        store.compact()?;
    }
    let mut store = DurableLabStore::open(&dir.0)?;
    let goal = objective(&store, plan)?;
    assert_eq!(goal.source, source());
    assert_eq!(goal.owner_session_id, SessionId::new(9));
    assert_eq!(goal.intent_id, 41);
    assert_eq!(goal.first_satisfied_anchor, None);
    assert_eq!(goal.restore_abandoned_anchor, None);
    assert_eq!(store.load_snapshot(goal.sealed_state_hash)?, sealed);
    assert_eq!(store.objectives(sealed.fortress_id).count(), 1);
    assert_eq!(store.commits(sealed.fortress_id).count(), 0);

    // Historical admission retry cannot reintroduce retired work.
    let records = store.report().records;
    admit(&mut store, &sealed, plan)?;
    assert_eq!(store.report().records, records);
    assert_eq!(store.commits(sealed.fortress_id).count(), 0);
    assert!(store.persist_commit(&sealed, plan, 41, source()).is_err());
    assert_eq!(store.report().records, records);
    Ok(())
}

#[test]
fn first_satisfaction_and_restore_abandonment_are_immutable_history() -> Result<()> {
    let dir = TempDir::new("immutable-proof");
    let sealed = snapshot(7, 10);
    let proof = snapshot(7, 20);
    let restored = snapshot(7, 5);
    let current = snapshot(7, 30);
    let proven = digest(1);
    let unfinished = digest(2);
    {
        let mut store = DurableLabStore::open(&dir.0)?;
        admit(&mut store, &sealed, proven)?;
        admit(&mut store, &sealed, unfinished)?;
        store.persist_progress_with_objectives(
            "empty",
            &proof,
            &[update(proven, "verified")],
            &[proven],
            &[proven],
            &[],
        )?;
        store.persist_progress_with_objectives(
            "empty",
            &restored,
            &[update(unfinished, "abandoned")],
            &[unfinished],
            &[],
            &[proven, unfinished],
        )?;
        store.persist_progress_with_objectives("empty", &current, &[], &[], &[proven], &[])?;
        store.persist_progress_with_objectives(
            "empty",
            &current,
            &[],
            &[],
            &[],
            &[proven, unfinished],
        )?;
        let records = store.report().records;
        assert!(
            store
                .persist_progress_with_objectives(
                    "empty",
                    &snapshot(7, 99),
                    &[],
                    &[],
                    &[unfinished],
                    &[]
                )
                .is_err()
        );
        assert_eq!(store.report().records, records);
        assert_eq!(
            store.head(sealed.fortress_id).map(|head| head.anchor),
            Some(current.anchor())
        );
        assert_eq!(
            objective(&store, proven)?.first_satisfied_anchor,
            Some(proof.anchor())
        );
        assert_eq!(
            objective(&store, proven)?.restore_abandoned_anchor,
            Some(restored.anchor())
        );
        assert_eq!(objective(&store, unfinished)?.first_satisfied_anchor, None);
        store.compact()?;
    }
    let store = DurableLabStore::open(&dir.0)?;
    assert_eq!(
        objective(&store, proven)?.first_satisfied_anchor,
        Some(proof.anchor())
    );
    assert_eq!(
        objective(&store, proven)?.restore_abandoned_anchor,
        Some(restored.anchor())
    );
    assert_eq!(
        objective(&store, unfinished)?.restore_abandoned_anchor,
        Some(restored.anchor())
    );
    assert_eq!(store.load_snapshot(sealed.state_hash)?, sealed);
    assert_eq!(store.load_snapshot(proof.state_hash)?, proof);
    assert_eq!(store.load_snapshot(restored.state_hash)?, restored);
    assert_eq!(store.load_snapshot(current.state_hash)?, current);
    assert_eq!(store.commits(sealed.fortress_id).count(), 0);
    Ok(())
}

#[test]
fn full_goal_book_refuses_new_work_and_only_evicts_achieved_retired_history() -> Result<()> {
    let dir = TempDir::new("capacity");
    let sealed = snapshot(7, 1);
    let incoming = snapshot(7, 99);
    let mut store = DurableLabStore::open(&dir.0)?;
    for number in 0..MAX_OBJECTIVES_PER_FORTRESS as u64 {
        admit(&mut store, &sealed, digest(number))?;
    }
    let new_plan = digest(100);
    let records = store.report().records;
    assert!(matches!(admit(&mut store, &incoming, new_plan),
        Err(error) if error.code == ErrorCode::BudgetExceeded));
    assert_eq!(store.report().records, records);
    assert!(!store.object_path(incoming.state_hash).exists());
    assert!(store.commit(sealed.fortress_id, new_plan).is_none());

    let abandoned = digest(0);
    store.persist_progress_with_objectives(
        "empty",
        &sealed,
        &[update(abandoned, "abandoned")],
        &[abandoned],
        &[],
        &[abandoned],
    )?;
    assert!(
        store
            .persist_objective_commit(
                &incoming,
                new_plan,
                41,
                source(),
                SessionId::new(9),
                &[abandoned]
            )
            .is_err()
    );
    let achieved = digest(1);
    store.persist_progress_with_objectives("empty", &sealed, &[], &[], &[achieved], &[])?;
    assert!(
        store
            .persist_objective_commit(
                &incoming,
                new_plan,
                41,
                source(),
                SessionId::new(9),
                &[achieved]
            )
            .is_err()
    );
    store.persist_progress(
        "empty",
        &sealed,
        &[update(achieved, "verified")],
        &[achieved],
    )?;
    store.persist_objective_commit(
        &incoming,
        new_plan,
        41,
        source(),
        SessionId::new(9),
        &[achieved],
    )?;
    assert_eq!(
        store.objectives(sealed.fortress_id).count(),
        MAX_OBJECTIVES_PER_FORTRESS
    );
    assert!(store.objective(sealed.fortress_id, achieved).is_none());
    assert!(store.objective(sealed.fortress_id, abandoned).is_some());
    assert!(store.commit(sealed.fortress_id, new_plan).is_some());
    store.compact()?;
    drop(store);
    let store = DurableLabStore::open(&dir.0)?;
    assert_eq!(
        store.objectives(sealed.fortress_id).count(),
        MAX_OBJECTIVES_PER_FORTRESS
    );
    assert!(store.objective(sealed.fortress_id, achieved).is_none());
    assert_eq!(
        objective(&store, new_plan)?.sealed_state_hash,
        incoming.state_hash
    );
    Ok(())
}

#[test]
fn invalid_goal_progress_is_refused_before_any_world_or_frontier_publication() -> Result<()> {
    let dir = TempDir::new("invalid-progress");
    let sealed = snapshot(7, 1);
    let incoming = snapshot(7, 2);
    let plan = digest(1);
    let unknown = digest(99);
    let mut store = DurableLabStore::open(&dir.0)?;
    admit(&mut store, &sealed, plan)?;
    store.persist_head("empty", &sealed)?;
    for (satisfied, abandoned) in [
        (vec![plan, plan], vec![]),
        (vec![], vec![plan, plan]),
        (vec![plan], vec![plan]),
        (vec![unknown], vec![]),
        (vec![], vec![unknown]),
        (vec![plan; MAX_OBJECTIVES_PER_FORTRESS + 1], vec![]),
        (vec![], vec![plan; MAX_OBJECTIVES_PER_FORTRESS + 1]),
    ] {
        let records = store.report().records;
        assert!(
            store
                .persist_progress_with_objectives(
                    "empty",
                    &incoming,
                    &[update(plan, "verified")],
                    &[plan],
                    &satisfied,
                    &abandoned
                )
                .is_err()
        );
        assert_eq!(store.report().records, records);
        assert!(store.commit(sealed.fortress_id, plan).is_some());
        assert_eq!(objective(&store, plan)?.first_satisfied_anchor, None);
        assert_eq!(
            store.head(sealed.fortress_id).map(|head| head.anchor),
            Some(sealed.anchor())
        );
        assert!(!store.object_path(incoming.state_hash).exists());
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct RetainedState {
    head: Option<DurableHead>,
    commits: Vec<DurableCommit>,
    objectives: Vec<DurableObjective>,
}

fn retained_state(store: &DurableLabStore) -> RetainedState {
    let fortress = FortressId::new(7);
    RetainedState {
        head: store.head(fortress).cloned(),
        commits: store.commits(fortress).cloned().collect(),
        objectives: store.objectives(fortress).cloned().collect(),
    }
}

#[test]
fn every_crash_prefix_keeps_goal_admission_retirement_and_restore_atomic() -> Result<()> {
    let dir = TempDir::new("goal-crash-campaign");
    let sealed = snapshot(7, 10);
    let proof = snapshot(7, 20);
    let restored = snapshot(7, 5);
    let first = digest(1);
    let second = digest(2);
    let replacement = digest(3);
    let journal = dir.0.join("journal");
    let mut states = Vec::new();
    let capture = |states: &mut Vec<_>, store: &DurableLabStore| -> Result<()> {
        let size = fs::metadata(&journal)
            .map_err(|e| io("test journal metadata", &e))?
            .len();
        states.push((size as usize, retained_state(store)));
        Ok(())
    };
    {
        let mut store = DurableLabStore::open(&dir.0)?;
        capture(&mut states, &store)?;
        store.persist_head("empty", &sealed)?;
        capture(&mut states, &store)?;
        admit(&mut store, &sealed, first)?;
        capture(&mut states, &store)?;
        admit(&mut store, &sealed, second)?;
        capture(&mut states, &store)?;
        store.persist_progress_with_objectives(
            "empty",
            &proof,
            &[update(first, "verified")],
            &[first],
            &[first],
            &[],
        )?;
        capture(&mut states, &store)?;
        store.persist_progress_with_objectives(
            "empty",
            &restored,
            &[update(second, "abandoned")],
            &[second],
            &[],
            &[first, second],
        )?;
        capture(&mut states, &store)?;
        store.persist_objective_commit(
            &restored,
            replacement,
            41,
            source(),
            SessionId::new(9),
            &[first],
        )?;
        capture(&mut states, &store)?;
    }
    let complete = fs::read(&journal).map_err(|e| io("test read journal", &e))?;
    for length in 0..=complete.len() {
        fs::write(&journal, &complete[..length]).map_err(|e| io("test write crash prefix", &e))?;
        let store = DurableLabStore::open(&dir.0)?;
        let (record_end, expected) = states
            .iter()
            .rev()
            .find(|(size, _)| *size <= length)
            .ok_or_else(|| corrupt("missing expected crash state"))?;
        assert_eq!(&retained_state(&store), expected, "crash at byte {length}");
        assert_eq!(store.report().torn_tail_bytes as usize, length - record_end);
    }
    let mut store = DurableLabStore::open(&dir.0)?;
    let before = retained_state(&store);
    store.compact()?;
    drop(store);
    let store = DurableLabStore::open(&dir.0)?;
    assert_eq!(retained_state(&store), before);
    assert_eq!(store.load_snapshot(restored.state_hash)?, restored);
    Ok(())
}

#[test]
fn uncertain_admission_fences_retries_and_reopens_without_double_eviction() -> Result<()> {
    for complete in [false, true] {
        let dir = TempDir::new(if complete {
            "goal-lost-ack"
        } else {
            "goal-partial-write"
        });
        let sealed = snapshot(7, 1);
        let incoming = snapshot(7, 3);
        let old = digest(1);
        let new = digest(2);
        let mut store = DurableLabStore::open(&dir.0)?;
        admit(&mut store, &sealed, old)?;
        store.persist_progress_with_objectives(
            "empty",
            &sealed,
            &[update(old, "verified")],
            &[old],
            &[old],
            &[],
        )?;
        let incoming_goal = DurableObjective {
            fortress_id: incoming.fortress_id,
            plan_digest: new,
            sealed_state_hash: incoming.state_hash,
            intent_id: 41,
            source: source(),
            owner_session_id: SessionId::new(9),
            first_satisfied_anchor: None,
            restore_abandoned_anchor: None,
        };
        store.write_object(&incoming)?;
        let payload = Record::ObjectiveCommit {
            objective: incoming_goal,
            evicted_history: vec![old],
        }
        .payload();
        let line = format!("{} {payload}\n", chain_next(store.chain, &payload).to_hex());
        assert!(
            store
                .write_record_with(line.as_bytes(), |journal, bytes| {
                    let length = if complete {
                        bytes.len()
                    } else {
                        bytes.len() / 2
                    };
                    journal.write_all(&bytes[..length])?;
                    Err(std::io::Error::other(
                        "injected uncertain objective admission",
                    ))
                })
                .is_err()
        );
        assert!(admit(&mut store, &sealed, old).is_err());
        assert!(
            store
                .persist_progress_with_objectives("empty", &sealed, &[], &[], &[old], &[])
                .is_err()
        );
        assert!(store.compact().is_err());
        drop(store);

        let mut store = DurableLabStore::open(&dir.0)?;
        assert_eq!(store.objective(sealed.fortress_id, new).is_some(), complete);
        assert_eq!(
            store.objective(sealed.fortress_id, old).is_some(),
            !complete
        );
        store.persist_objective_commit(&incoming, new, 41, source(), SessionId::new(9), &[old])?;
        assert_eq!(store.objectives(sealed.fortress_id).count(), 1);
        assert_eq!(store.commits(sealed.fortress_id).count(), 1);
        let records = store.report().records;
        store.persist_objective_commit(&incoming, new, 41, source(), SessionId::new(9), &[old])?;
        assert_eq!(store.report().records, records);
        drop(store);
        let store = DurableLabStore::open(&dir.0)?;
        assert_eq!(
            objective(&store, new)?.sealed_state_hash,
            incoming.state_hash
        );
        assert!(store.objective(sealed.fortress_id, old).is_none());
    }
    Ok(())
}

#[test]
fn all_retained_source_proof_and_abandonment_objects_are_required_on_reopen() -> Result<()> {
    for missing in 0..3 {
        let dir = TempDir::new("required-goal-object");
        let snapshots = [snapshot(7, 10), snapshot(7, 20), snapshot(7, 5)];
        let plan = digest(1);
        {
            let mut store = DurableLabStore::open(&dir.0)?;
            admit(&mut store, &snapshots[0], plan)?;
            store.persist_progress_with_objectives(
                "empty",
                &snapshots[1],
                &[update(plan, "verified")],
                &[plan],
                &[plan],
                &[],
            )?;
            store.persist_progress_with_objectives(
                "empty",
                &snapshots[2],
                &[],
                &[],
                &[],
                &[plan],
            )?;
            store.persist_head("empty", &snapshot(7, 30))?;
            store.compact()?;
            for snapshot in &snapshots {
                assert!(store.object_path(snapshot.state_hash).exists());
            }
            fs::remove_file(store.object_path(snapshots[missing].state_hash))
                .map_err(|e| io("test remove retained objective object", &e))?;
        }
        assert!(matches!(DurableLabStore::open(&dir.0),
            Err(error) if error.code == ErrorCode::CorruptLedger));
    }
    Ok(())
}

#[test]
fn valid_chain_cannot_hide_mismatched_original_source_or_evidence_anchors() -> Result<()> {
    for corrupt_field in 0..3 {
        let dir = TempDir::new("mismatched-goal-object");
        let sealed = snapshot(7, 10);
        let proof = snapshot(7, 20);
        let restored = snapshot(7, 5);
        let other = snapshot(8, 10);
        let mut goal = DurableObjective {
            fortress_id: sealed.fortress_id,
            plan_digest: digest(1),
            sealed_state_hash: sealed.state_hash,
            intent_id: 41,
            source: source(),
            owner_session_id: SessionId::new(9),
            first_satisfied_anchor: Some(proof.anchor()),
            restore_abandoned_anchor: Some(restored.anchor()),
        };
        match corrupt_field {
            0 => goal.sealed_state_hash = other.state_hash,
            1 => {
                goal.first_satisfied_anchor = Some(StateAnchor {
                    tick: GameTick(999),
                    ..proof.anchor()
                })
            }
            _ => {
                goal.restore_abandoned_anchor = Some(StateAnchor {
                    state_hash: other.state_hash,
                    ..restored.anchor()
                })
            }
        }
        {
            let mut store = DurableLabStore::open(&dir.0)?;
            for snapshot in [&sealed, &proof, &restored, &other] {
                store.write_object(snapshot)?;
            }
            // Compaction records are private to the store. Inject a correctly
            // chained corrupt one to exercise verification of retained roots.
            store.append(Record::Objective(goal), None)?;
        }
        assert!(matches!(DurableLabStore::open(&dir.0),
            Err(error) if error.code == ErrorCode::CorruptLedger));
    }
    Ok(())
}

#[test]
fn objective_codecs_refuse_unbounded_or_ambiguous_new_records() -> Result<()> {
    let sealed = snapshot(7, 10);
    let goal = DurableObjective {
        fortress_id: sealed.fortress_id,
        plan_digest: digest(1),
        sealed_state_hash: sealed.state_hash,
        intent_id: 41,
        source: source(),
        owner_session_id: SessionId::new(9),
        first_satisfied_anchor: None,
        restore_abandoned_anchor: None,
    };
    let admission = Record::ObjectiveCommit {
        objective: goal.clone(),
        evicted_history: vec![],
    };
    let retained = Record::Objective(goal);
    let progress = Record::ProgressWithObjectives {
        head: DurableHead {
            fortress_id: sealed.fortress_id,
            scenario: "empty".to_owned(),
            anchor: sealed.anchor(),
        },
        updates: vec![],
        retired: vec![],
        satisfied: vec![digest(1)],
        abandoned: vec![digest(2)],
    };
    for record in [&admission, &retained, &progress] {
        assert_eq!(Record::parse(&record.payload())?, *record);
    }
    let g = admission.payload();
    let o = retained.payload();
    let v = progress.payload();
    let replace = |payload: &str, index: usize, value: &str| -> String {
        let mut fields: Vec<_> = payload.split(' ').map(str::to_owned).collect();
        fields[index] = value.to_owned();
        fields.join(" ")
    };
    let mut ordered = [digest(1), digest(2)];
    ordered.sort();
    let malformed = [
        replace(&g, 0, "goal"),
        replace(&g, 4, &"0".repeat(32)),
        replace(&g, 5, "inferred_production"),
        replace(&g, 6, &"61".repeat(MAX_PLAN_SUMMARY_BYTES + 1)),
        replace(&g, 7, &"61".repeat(MAX_PLAN_REQUEST_BYTES + 1)),
        replace(&g, 7, "ff"),
        replace(&g, 7, "0"),
        replace(&g, 8, &"0".repeat(32)),
        replace(&g, 8, &"A".repeat(32)),
        replace(&g, 9, &(MAX_OBJECTIVES_PER_FORTRESS + 1).to_string()),
        replace(&g, 9, &u64::MAX.to_string()),
        format!(
            "{} {} {}",
            replace(&g, 9, "2"),
            digest(1).to_hex(),
            digest(1).to_hex()
        ),
        format!(
            "{} {} {}",
            replace(&g, 9, "2"),
            ordered[1].to_hex(),
            ordered[0].to_hex()
        ),
        format!("{g} trailing"),
        replace(&o, 4, &"0".repeat(32)),
        replace(&o, 8, &"0".repeat(32)),
        replace(&o, 9, &"q".repeat(64)),
        replace(&o, 10, "1"),
        replace(&o, 14, "1"),
        replace(&o, 15, "-1"),
        format!("{o} trailing"),
        replace(&v, 7, &(MAX_PROGRESS_UPDATES + 1).to_string()),
        replace(&v, 8, &(MAX_COMMITS_PER_FORTRESS + 1).to_string()),
        replace(&v, 9, &(MAX_OBJECTIVES_PER_FORTRESS + 1).to_string()),
        replace(&v, 10, &(MAX_OBJECTIVES_PER_FORTRESS + 1).to_string()),
        replace(&v, 9, &u64::MAX.to_string()),
        replace(&v, 12, &digest(1).to_hex()),
        format!("{} {}", replace(&v, 9, "2"), digest(1).to_hex()),
        format!("{v} trailing"),
        "G ".to_owned() + &"x".repeat(MAX_RECORD_BYTES),
    ];
    for (index, payload) in malformed.into_iter().enumerate() {
        assert!(
            Record::parse(&payload).is_err(),
            "accepted malformed objective case {index}"
        );
    }
    // The new nonzero-owner/intent rule does not reinterpret old P records.
    let legacy = Record::Commit(DurableCommit {
        fortress_id: sealed.fortress_id,
        plan_digest: digest(3),
        sealed_state_hash: sealed.state_hash,
        intent_id: 0,
        source: source(),
        steps: BTreeMap::new(),
        step_anchors: BTreeMap::new(),
    });
    assert_eq!(Record::parse(&legacy.payload())?, legacy);
    Ok(())
}

#[test]
fn objective_admission_refuses_identity_conflicts_and_invalid_sources_before_writes() -> Result<()>
{
    let dir = TempDir::new("invalid-admission");
    let sealed = snapshot(7, 1);
    let incoming = snapshot(7, 2);
    let plan = digest(1);
    let mut store = DurableLabStore::open(&dir.0)?;
    admit(&mut store, &sealed, plan)?;
    let before = retained_state(&store);
    let records = store.report().records;
    let cases = [
        (&sealed, 41, source(), SessionId::NIL),
        (&sealed, 0, source(), SessionId::new(9)),
        (
            &sealed,
            41,
            DurablePlanSource::Production {
                summary: "oversized source".to_owned(),
                raw: "x".repeat(MAX_PLAN_REQUEST_BYTES + 1),
            },
            SessionId::new(9),
        ),
        (
            &sealed,
            41,
            DurablePlanSource::Pause {
                summary: "x".repeat(MAX_PLAN_SUMMARY_BYTES + 1),
                paused: false,
            },
            SessionId::new(9),
        ),
        (
            &sealed,
            41,
            DurablePlanSource::Actions {
                summary: "lost original quota".to_owned(),
                raw: "[]".to_owned(),
            },
            SessionId::new(9),
        ),
        (&sealed, 42, source(), SessionId::new(9)),
        (&sealed, 41, source(), SessionId::new(10)),
        (&incoming, 41, source(), SessionId::new(9)),
    ];
    for (sealed, intent, source, owner) in cases {
        assert!(
            store
                .persist_objective_commit(sealed, plan, intent, source, owner, &[])
                .is_err()
        );
        assert_eq!(store.report().records, records);
        assert_eq!(retained_state(&store), before);
        assert!(!store.object_path(incoming.state_hash).exists());
    }
    let new = digest(2);
    for evicted in [
        vec![digest(99)],
        vec![plan],
        vec![plan, plan],
        vec![new],
        vec![digest(99); MAX_OBJECTIVES_PER_FORTRESS + 1],
    ] {
        assert!(
            store
                .persist_objective_commit(&incoming, new, 41, source(), SessionId::new(9), &evicted)
                .is_err()
        );
        assert_eq!(store.report().records, records);
        assert_eq!(retained_state(&store), before);
        assert!(!store.object_path(incoming.state_hash).exists());
    }
    let legacy = digest(3);
    store.persist_commit(&sealed, legacy, 41, source())?;
    let before = retained_state(&store);
    let records = store.report().records;
    assert!(
        store
            .persist_objective_commit(&sealed, legacy, 41, source(), SessionId::new(9), &[])
            .is_err()
    );
    assert_eq!(store.report().records, records);
    assert_eq!(retained_state(&store), before);
    assert!(store.objective(sealed.fortress_id, legacy).is_none());
    Ok(())
}

#[test]
fn post_admission_compaction_failure_fences_even_identical_retries_until_reopen() -> Result<()> {
    let dir = TempDir::new("goal-compaction-fault");
    let sealed = snapshot(7, 10);
    let plan = digest(1);
    let mut store = DurableLabStore::open(&dir.0)?;
    store.persist_head("empty", &sealed)?;
    // Force the automatic-compaction boundary without a thousand records.
    store.records = COMPACT_AFTER_RECORDS - 1;
    let blocked = dir.0.join("journal.compact");
    fs::create_dir(&blocked).map_err(|e| io("test block compaction publication", &e))?;
    assert!(matches!(admit(&mut store, &sealed, plan),
        Err(error) if error.code == ErrorCode::AdapterUnavailable && !error.retryable));
    // Admission reached the journal before compaction failed. The failed
    // caller must reconcile instead of clearing the fault with a head no-op.
    assert!(store.objective(sealed.fortress_id, plan).is_some());
    assert!(store.commit(sealed.fortress_id, plan).is_some());
    assert!(store.persist_head("empty", &sealed).is_err());
    assert!(admit(&mut store, &sealed, plan).is_err());
    assert!(
        store
            .persist_progress_with_objectives("empty", &sealed, &[], &[], &[plan], &[])
            .is_err()
    );
    assert!(store.retire_commit(sealed.fortress_id, plan).is_err());
    fs::remove_dir(&blocked).map_err(|e| io("test unblock compaction", &e))?;
    drop(store);
    let mut store = DurableLabStore::open(&dir.0)?;
    assert_eq!(objective(&store, plan)?.source, source());
    assert!(store.commit(sealed.fortress_id, plan).is_some());
    assert_eq!(store.load_snapshot(sealed.state_hash)?, sealed);
    let records = store.report().records;
    admit(&mut store, &sealed, plan)?;
    assert_eq!(store.report().records, records);
    store.compact()?;
    Ok(())
}
