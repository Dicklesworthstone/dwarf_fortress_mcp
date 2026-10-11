//! Retiring action handles must not erase proof of never-dispatched successors.

use super::*;
use dfmcp_world::WorldGraph;

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-custody-{name}-{}-{:?}",
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
        summary: "finish the original drinks setup".to_owned(),
        raw: r#"{"quotas":[{"item":"DRINK","minimum":60}],"template":"production"}"#.to_owned(),
    }
}

fn digest() -> Digest32 {
    Digest32::of_bytes(b"original-production-setup")
}

fn admit(store: &mut DurableLabStore, sealed: &WorldSnapshot) -> Result<()> {
    store.persist_objective_commit(sealed, digest(), 41, source(), SessionId::new(9), &[])
}

fn objective(store: &DurableLabStore) -> Result<&DurableObjective> {
    store
        .objective(FortressId::new(7), digest())
        .ok_or_else(|| corrupt("test lost the original objective"))
}

fn update(step: u32, state: &str) -> DurableStepUpdate {
    DurableStepUpdate {
        plan_digest: digest(),
        step,
        state: state.to_owned(),
    }
}

fn custody(anchor: StateAnchor) -> Record {
    Record::ObjectiveCustody {
        fortress_id: FortressId::new(7),
        plan_digest: digest(),
        steps: BTreeMap::from([(2, anchor)]),
    }
}

#[test]
fn retirement_keeps_exact_nondispatch_custody_after_compaction_and_two_reopens() -> Result<()> {
    for direct in [false, true] {
        let dir = TempDir::new(if direct {
            "single-retire"
        } else {
            "atomic-retire"
        });
        let sealed = snapshot(7, 10);
        let witnessed = snapshot(7, 15);
        let finished = snapshot(7, 20);
        let current = snapshot(7, 30);
        {
            let mut store = DurableLabStore::open(&dir.0)?;
            admit(&mut store, &sealed)?;
            store.persist_progress(
                "empty",
                &witnessed,
                &[
                    update(1, "dispatched"),
                    update(2, "not_dispatched"),
                    update(3, "cancelled"),
                    update(5, "abandoned"),
                    update(6, "failed"),
                ],
                &[],
            )?;
            // A legacy independent record and a wholly missing row are not
            // evidence. Prepared work also remains live until retirement.
            store.persist_step(sealed.fortress_id, digest(), 4, "not_dispatched")?;
            assert!(objective(&store)?.retired_never_dispatched.is_empty());
            let retired = [digest()];
            store.persist_progress(
                "empty",
                &finished,
                &[update(1, "verified")],
                if direct { &[] } else { &retired },
            )?;
            if direct {
                assert!(objective(&store)?.retired_never_dispatched.is_empty());
                store.retire_commit(sealed.fortress_id, digest())?;
            }
            assert_eq!(
                objective(&store)?.retired_never_dispatched,
                BTreeMap::from([(2, witnessed.anchor())]),
            );
            assert!(store.commit(sealed.fortress_id, digest()).is_none());
            store.persist_head("empty", &current)?;
            store.compact()?;
            assert_eq!(store.load_snapshot(witnessed.state_hash)?, witnessed);
            assert!(!store.object_path(finished.state_hash).exists());
        }
        for _ in 0..2 {
            let mut store = DurableLabStore::open(&dir.0)?;
            let goal = objective(&store)?;
            assert_eq!(
                goal.retired_never_dispatched,
                BTreeMap::from([(2, witnessed.anchor())])
            );
            assert_eq!(goal.sealed_state_hash, sealed.state_hash);
            assert_eq!(goal.intent_id, 41);
            assert_eq!(goal.source, source());
            assert_eq!(goal.owner_session_id, SessionId::new(9));
            assert_eq!(goal.first_satisfied_anchor, None);
            assert_eq!(goal.restore_abandoned_anchor, None);
            assert_eq!(store.load_snapshot(goal.sealed_state_hash)?, sealed);
            assert_eq!(store.load_snapshot(witnessed.state_hash)?, witnessed);
            assert_eq!(
                store.head(sealed.fortress_id).map(|head| head.anchor),
                Some(current.anchor())
            );
            let records = store.report().records;
            admit(&mut store, &sealed)?;
            assert_eq!(store.report().records, records);
            assert!(store.commit(sealed.fortress_id, digest()).is_none());
            store.compact()?;
        }
    }
    Ok(())
}

#[test]
fn a_later_dispatch_supersedes_prepared_absence_and_blocks_early_retirement() -> Result<()> {
    let dir = TempDir::new("prepared-then-dispatched");
    let sealed = snapshot(7, 10);
    let sent = snapshot(7, 20);
    let finished = snapshot(7, 30);
    let mut store = DurableLabStore::open(&dir.0)?;
    admit(&mut store, &sealed)?;
    store.persist_progress("empty", &sealed, &[update(2, "not_dispatched")], &[])?;
    assert!(objective(&store)?.retired_never_dispatched.is_empty());
    store.persist_progress("empty", &sent, &[update(2, "dispatched")], &[])?;
    let records = store.report().records;
    assert!(store.retire_commit(sealed.fortress_id, digest()).is_err());
    assert!(
        store
            .persist_progress("empty", &finished, &[], &[digest()])
            .is_err()
    );
    assert_eq!(store.report().records, records);
    assert_eq!(
        store.head(sealed.fortress_id).map(|head| head.anchor),
        Some(sent.anchor())
    );
    assert!(objective(&store)?.retired_never_dispatched.is_empty());
    store.persist_progress("empty", &finished, &[update(2, "verified")], &[digest()])?;
    store.compact()?;
    drop(store);
    let store = DurableLabStore::open(&dir.0)?;
    assert!(objective(&store)?.retired_never_dispatched.is_empty());
    assert!(store.commit(sealed.fortress_id, digest()).is_none());
    Ok(())
}

#[test]
fn legacy_retirement_records_do_not_invent_lost_step_custody() -> Result<()> {
    for legacy in 0..3 {
        let dir = TempDir::new("legacy-retirement");
        let sealed = snapshot(7, 10);
        let mut store = DurableLabStore::open(&dir.0)?;
        admit(&mut store, &sealed)?;
        store.persist_progress("empty", &sealed, &[update(2, "not_dispatched")], &[])?;
        let head = DurableHead {
            fortress_id: sealed.fortress_id,
            scenario: "empty".to_owned(),
            anchor: sealed.anchor(),
        };
        let record = match legacy {
            0 => Record::Done {
                fortress_id: sealed.fortress_id,
                plan_digest: digest(),
                retain_nondispatch: false,
            },
            1 => Record::Progress {
                head,
                updates: vec![],
                retired: vec![digest()],
            },
            _ => Record::ProgressWithObjectives {
                head,
                updates: vec![],
                retired: vec![digest()],
                satisfied: vec![],
                abandoned: vec![],
                retain_nondispatch: false,
            },
        };
        assert!(matches!(record.payload().as_bytes()[0], b'D' | b'F' | b'V'));
        store.append(record, None)?;
        drop(store);
        let mut store = DurableLabStore::open(&dir.0)?;
        assert!(objective(&store)?.retired_never_dispatched.is_empty());
        store.compact()?;
        drop(store);
        let store = DurableLabStore::open(&dir.0)?;
        assert!(objective(&store)?.retired_never_dispatched.is_empty());
    }
    Ok(())
}

#[test]
fn custody_import_rejects_live_unknown_duplicate_and_unbound_history() -> Result<()> {
    let dir = TempDir::new("invalid-custody-import");
    let sealed = snapshot(7, 10);
    let mut store = DurableLabStore::open(&dir.0)?;
    admit(&mut store, &sealed)?;
    let mut fabricated = objective(&store)?.clone();
    fabricated
        .retired_never_dispatched
        .insert(2, sealed.anchor());
    assert!(
        store
            .index
            .validate_objective_admission(&fabricated, &[])
            .is_err()
    );
    assert!(store.append(Record::Objective(fabricated), None).is_err());
    assert!(store.append(custody(sealed.anchor()), None).is_err());
    assert!(
        store
            .append(
                Record::ObjectiveCustody {
                    fortress_id: sealed.fortress_id,
                    plan_digest: Digest32::of_bytes(b"unknown-goal"),
                    steps: BTreeMap::from([(2, sealed.anchor())]),
                },
                None
            )
            .is_err()
    );
    store.append(
        Record::Done {
            fortress_id: sealed.fortress_id,
            plan_digest: digest(),
            retain_nondispatch: false,
        },
        None,
    )?;
    for steps in [
        BTreeMap::new(),
        (0..=MAX_STEPS_PER_COMMIT as u32)
            .map(|step| (step, sealed.anchor()))
            .collect(),
        BTreeMap::from([(2, snapshot(8, 10).anchor())]),
    ] {
        assert!(
            store
                .append(
                    Record::ObjectiveCustody {
                        fortress_id: sealed.fortress_id,
                        plan_digest: digest(),
                        steps,
                    },
                    None
                )
                .is_err()
        );
    }
    store.append(custody(sealed.anchor()), None)?;
    let records = store.report().records;
    for anchor in [sealed.anchor(), snapshot(7, 20).anchor()] {
        assert!(store.append(custody(anchor), None).is_err());
    }
    assert_eq!(store.report().records, records);
    assert_eq!(
        objective(&store)?.retired_never_dispatched,
        BTreeMap::from([(2, sealed.anchor())])
    );
    Ok(())
}

#[test]
fn correctly_chained_custody_still_requires_the_exact_archived_anchor() -> Result<()> {
    for mismatch in 0..3 {
        let dir = TempDir::new("bad-custody-anchor");
        let sealed = snapshot(7, 10);
        let other = snapshot(8, 10);
        {
            let mut store = DurableLabStore::open(&dir.0)?;
            admit(&mut store, &sealed)?;
            store.retire_commit(sealed.fortress_id, digest())?;
            store.write_object(&other)?;
            let mut anchor = sealed.anchor();
            match mismatch {
                0 => anchor.tick = GameTick(999),
                1 => anchor.state_hash = other.state_hash,
                _ => anchor.state_hash = Digest32::of_bytes(b"missing-custody-object"),
            }
            store.append(custody(anchor), None)?;
        }
        assert!(
            matches!(DurableLabStore::open(&dir.0), Err(error) if error.code == ErrorCode::CorruptLedger)
        );
    }
    Ok(())
}

#[test]
fn custody_codecs_are_bounded_canonical_and_separate_from_large_sources() -> Result<()> {
    let sealed = snapshot(7, 10);
    let prefix = format!("N 7 {}", digest().to_hex());
    let row = format!("2 {} 10 1 10", sealed.state_hash.to_hex());
    let earlier = format!("1 {} 10 1 10", sealed.state_hash.to_hex());
    for payload in [
        format!("{prefix} 0"),
        format!("{prefix} {}", MAX_STEPS_PER_COMMIT + 1),
        format!("{prefix} 2 {row} {row}"),
        format!("{prefix} 2 {row} {earlier}"),
        format!(
            "{prefix} 1 4294967296 {} 10 1 10",
            sealed.state_hash.to_hex()
        ),
        format!("{prefix} 1 {row} extra"),
        format!("{prefix} 2 {row}"),
    ] {
        assert!(Record::parse(&payload).is_err(), "accepted {payload}");
    }
    let dir = TempDir::new("maximal-custody");
    {
        let mut store = DurableLabStore::open(&dir.0)?;
        store.persist_objective_commit(
            &sealed,
            digest(),
            41,
            DurablePlanSource::Production {
                summary: "s".repeat(MAX_PLAN_SUMMARY_BYTES),
                raw: "r".repeat(MAX_PLAN_REQUEST_BYTES),
            },
            SessionId::new(9),
            &[],
        )?;
        let updates: Vec<_> = (1..=MAX_STEPS_PER_COMMIT as u32)
            .map(|step| update(step, "not_dispatched"))
            .collect();
        store.persist_progress("empty", &sealed, &updates, &[digest()])?;
        store.compact()?;
        let journal = fs::read_to_string(dir.0.join("journal"))
            .map_err(|error| io("cannot read custody fixture journal", &error))?;
        let mut custody_records = 0;
        for line in journal.lines() {
            let (_, payload) = line
                .split_once(' ')
                .ok_or_else(|| corrupt("test record lacks chain"))?;
            assert!(payload.len() <= MAX_RECORD_BYTES);
            let record = Record::parse(payload)?;
            assert_eq!(Record::parse(&record.payload())?, record);
            custody_records += usize::from(matches!(record, Record::ObjectiveCustody { .. }));
        }
        assert_eq!(custody_records, 1);
    }
    let store = DurableLabStore::open(&dir.0)?;
    assert_eq!(
        objective(&store)?.retired_never_dispatched.len(),
        MAX_STEPS_PER_COMMIT
    );
    Ok(())
}

#[test]
fn failed_retirement_append_keeps_recoverable_live_frontier() -> Result<()> {
    for direct in [false, true] {
        let dir = TempDir::new(if direct {
            "crash-single"
        } else {
            "crash-atomic"
        });
        let sealed = snapshot(7, 10);
        let mut store = DurableLabStore::open(&dir.0)?;
        admit(&mut store, &sealed)?;
        store.persist_progress("empty", &sealed, &[update(2, "not_dispatched")], &[])?;
        store.set_append_budget(Some(0));
        let result = if direct {
            store.retire_commit(sealed.fortress_id, digest())
        } else {
            store.persist_progress("empty", &sealed, &[], &[digest()])
        };
        assert!(matches!(result, Err(error) if error.code == ErrorCode::AdapterUnavailable));
        assert!(objective(&store)?.retired_never_dispatched.is_empty());
        assert!(store.commit(sealed.fortress_id, digest()).is_some());
        drop(store);
        let mut store = DurableLabStore::open(&dir.0)?;
        assert!(objective(&store)?.retired_never_dispatched.is_empty());
        assert!(store.commit(sealed.fortress_id, digest()).is_some());
        store.retire_commit(sealed.fortress_id, digest())?;
        store.compact()?;
        drop(store);
        let store = DurableLabStore::open(&dir.0)?;
        assert_eq!(
            objective(&store)?.retired_never_dispatched,
            BTreeMap::from([(2, sealed.anchor())])
        );
    }
    Ok(())
}

#[test]
fn uncertain_retirement_append_reopens_as_one_complete_custody_frontier() -> Result<()> {
    for direct in [false, true] {
        for complete in [false, true] {
            let dir = TempDir::new("uncertain-retirement");
            let sealed = snapshot(7, 10);
            let witnessed = snapshot(7, 15);
            let finished = snapshot(7, 20);
            let mut store = DurableLabStore::open(&dir.0)?;
            admit(&mut store, &sealed)?;
            store.persist_progress(
                "empty",
                &witnessed,
                &[update(1, "dispatched"), update(2, "not_dispatched")],
                &[],
            )?;
            let record = if direct {
                store.persist_progress("empty", &finished, &[update(1, "verified")], &[])?;
                Record::Done {
                    fortress_id: sealed.fortress_id,
                    plan_digest: digest(),
                    retain_nondispatch: true,
                }
            } else {
                store.write_object(&finished)?;
                Record::ProgressWithObjectives {
                    head: DurableHead {
                        fortress_id: sealed.fortress_id,
                        scenario: "empty".to_owned(),
                        anchor: finished.anchor(),
                    },
                    updates: vec![update(1, "verified")],
                    retired: vec![digest()],
                    satisfied: vec![],
                    abandoned: vec![],
                    retain_nondispatch: true,
                }
            };
            let payload = record.payload();
            assert_eq!(Record::parse(&payload)?, record);
            assert!(matches!(payload.as_bytes()[0], b'R' | b'W'));
            let line = format!("{} {payload}\n", chain_next(store.chain, &payload).to_hex());
            assert!(
                store
                    .write_record_with(line.as_bytes(), |journal, bytes| {
                        journal.write_all(if complete {
                            bytes
                        } else {
                            &bytes[..bytes.len() / 2]
                        })?;
                        Err(std::io::Error::other("lost retirement acknowledgment"))
                    })
                    .is_err()
            );
            assert!(store.retire_commit(sealed.fortress_id, digest()).is_err());
            assert!(store.persist_head("empty", &finished).is_err());
            drop(store);
            let mut store = DurableLabStore::open(&dir.0)?;
            assert_eq!(
                store.commit(sealed.fortress_id, digest()).is_none(),
                complete
            );
            assert_eq!(
                objective(&store)?.retired_never_dispatched.is_empty(),
                !complete
            );
            let expected_head = if direct || complete {
                finished.anchor()
            } else {
                witnessed.anchor()
            };
            assert_eq!(
                store.head(sealed.fortress_id).map(|head| head.anchor),
                Some(expected_head)
            );
            assert_eq!(store.report().torn_tail_bytes > 0, !complete);
            if !complete {
                store.persist_progress(
                    "empty",
                    &finished,
                    &[update(1, "verified")],
                    &[digest()],
                )?;
            }
            assert_eq!(
                objective(&store)?.retired_never_dispatched,
                BTreeMap::from([(2, witnessed.anchor())])
            );
            store.compact()?;
            drop(store);
            let store = DurableLabStore::open(&dir.0)?;
            assert_eq!(
                objective(&store)?.retired_never_dispatched,
                BTreeMap::from([(2, witnessed.anchor())])
            );
        }
    }
    Ok(())
}
