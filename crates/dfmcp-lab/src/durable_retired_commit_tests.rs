//! Final action receipts remain inspectable after retirement and repeated restart.

use super::*;
use dfmcp_world::WorldGraph;

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-retired-commit-{name}-{}-{:?}",
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

fn snapshot(tick: u64) -> WorldSnapshot {
    WorldSnapshot::new(
        FortressId::new(23),
        GameTick(tick),
        ObservationCursor {
            epoch: 1,
            sequence: tick,
        },
        false,
        WorldGraph::default(),
    )
}

fn digest(number: u64) -> Digest32 {
    Digest32::of_bytes(&number.to_be_bytes())
}

fn admit(
    store: &mut DurableLabStore,
    sealed: &WorldSnapshot,
    number: u64,
    evicted: &[Digest32],
) -> Result<()> {
    store.persist_objective_commit(
        sealed,
        digest(number),
        41,
        DurablePlanSource::Actions {
            summary: "retained original".to_owned(),
            raw: "[]".to_owned(),
        },
        SessionId::new(9),
        evicted,
    )
}

fn update(step: u32, state: &str) -> DurableStepUpdate {
    DurableStepUpdate {
        plan_digest: digest(1),
        step,
        state: state.to_owned(),
    }
}

fn history(store: &DurableLabStore) -> Result<&DurableCommit> {
    store
        .retired_commit(FortressId::new(23), digest(1))
        .ok_or_else(|| corrupt("fixture lost terminal receipt history"))
}

fn record(anchor: StateAnchor, state: &str) -> Record {
    Record::ObjectiveRetiredCommit {
        fortress_id: anchor.fortress_id,
        plan_digest: digest(1),
        steps: BTreeMap::from([(1, (state.to_owned(), anchor))]),
    }
}

#[test]
fn complete_terminal_receipts_survive_retirement_compaction_and_repeated_reopen() -> Result<()> {
    for direct in [false, true] {
        let dir = TempDir::new(if direct { "direct" } else { "atomic" });
        let sealed = snapshot(10);
        let earlier = snapshot(20);
        let terminal = snapshot(30);
        let expected;
        {
            let mut store = DurableLabStore::open(&dir.0)?;
            admit(&mut store, &sealed, 1, &[])?;
            store.persist_progress(
                "empty",
                &earlier,
                &[
                    update(0, "verified"),
                    update(1, "dispatched"),
                    update(2, "not_dispatched"),
                    update(3, "cancelled"),
                    update(4, "compensated"),
                    update(5, "failed"),
                ],
                &[],
            )?;
            assert!(
                store
                    .retired_commit(sealed.fortress_id, digest(1))
                    .is_none()
            );
            store.persist_progress("empty", &terminal, &[update(1, "verified")], &[])?;
            expected = store
                .commit(sealed.fortress_id, digest(1))
                .ok_or_else(|| corrupt("fixture lost active commit"))?
                .clone();
            if direct {
                store.retire_commit(sealed.fortress_id, digest(1))?;
            } else {
                store.persist_progress("empty", &terminal, &[], &[digest(1)])?;
            }
            assert_eq!(history(&store)?, &expected);
            assert_eq!(store.commits(sealed.fortress_id).count(), 0);
            store.persist_head("empty", &snapshot(60))?;
            store.compact()?;
        }
        for _ in 0..2 {
            let mut store = DurableLabStore::open(&dir.0)?;
            assert_eq!(history(&store)?, &expected);
            assert_eq!(store.load_snapshot(earlier.state_hash)?, earlier);
            assert_eq!(store.load_snapshot(terminal.state_hash)?, terminal);
            assert!(store.commit(sealed.fortress_id, digest(1)).is_none());
            let records = store.report().records;
            store.retire_commit(sealed.fortress_id, digest(1))?;
            assert_eq!(store.report().records, records);
            store.compact()?;
        }
    }
    Ok(())
}

#[test]
fn legacy_missing_unanchored_and_live_receipts_never_become_complete_history() -> Result<()> {
    for missing in 0..3 {
        let dir = TempDir::new("missing");
        let sealed = snapshot(10);
        let mut store = DurableLabStore::open(&dir.0)?;
        admit(&mut store, &sealed, 1, &[])?;
        match missing {
            0 => {} // Empty original receipt map.
            1 => {
                store.persist_progress("empty", &sealed, &[update(0, "verified")], &[])?;
                store.persist_step(sealed.fortress_id, digest(1), 1, "verified")?;
            }
            _ => {
                store.persist_progress("empty", &sealed, &[update(0, "verified")], &[])?;
                store.append(
                    Record::Done {
                        fortress_id: sealed.fortress_id,
                        plan_digest: digest(1),
                        retain_nondispatch: false,
                    },
                    None,
                )?;
            }
        }
        store.retire_commit(sealed.fortress_id, digest(1))?;
        assert!(
            store
                .retired_commit(sealed.fortress_id, digest(1))
                .is_none()
        );
        store.compact()?;
        drop(store);
        let reopened = DurableLabStore::open(&dir.0)?;
        assert!(
            reopened
                .retired_commit(sealed.fortress_id, digest(1))
                .is_none()
        );
    }
    let dir = TempDir::new("active");
    let sealed = snapshot(10);
    let mut store = DurableLabStore::open(&dir.0)?;
    admit(&mut store, &sealed, 1, &[])?;
    store.persist_progress("empty", &sealed, &[update(0, "dispatched")], &[])?;
    assert!(store.retire_commit(sealed.fortress_id, digest(1)).is_err());
    assert!(
        store
            .retired_commit(sealed.fortress_id, digest(1))
            .is_none()
    );
    assert!(store.commit(sealed.fortress_id, digest(1)).is_some());
    Ok(())
}

#[test]
fn terminal_archive_import_rejects_live_unknown_duplicate_and_conflicting_custody() -> Result<()> {
    let dir = TempDir::new("import");
    let sealed = snapshot(10);
    let mut store = DurableLabStore::open(&dir.0)?;
    assert!(
        store
            .append(record(sealed.anchor(), "verified"), None)
            .is_err()
    );
    admit(&mut store, &sealed, 1, &[])?;
    assert!(
        store
            .append(record(sealed.anchor(), "verified"), None)
            .is_err()
    );
    store.retire_commit(sealed.fortress_id, digest(1))?;
    for state in ["dispatched", "unknown", "not_dispatched"] {
        assert!(store.append(record(sealed.anchor(), state), None).is_err());
    }
    store.append(record(sealed.anchor(), "verified"), None)?;
    let before = store.report();
    assert!(
        store
            .append(record(sealed.anchor(), "verified"), None)
            .is_err()
    );
    assert!(
        store
            .append(record(sealed.anchor(), "failed"), None)
            .is_err()
    );
    assert_eq!(store.report(), before);
    assert_eq!(
        history(&store)?.steps.get(&1).map(String::as_str),
        Some("verified")
    );
    Ok(())
}

#[test]
fn invalid_terminal_snapshot_metadata_or_bytes_refuse_reopen() -> Result<()> {
    for corrupt_bytes in [false, true] {
        let dir = TempDir::new(if corrupt_bytes {
            "bad-object"
        } else {
            "bad-anchor"
        });
        let sealed = snapshot(10);
        let proof = snapshot(20);
        {
            let mut store = DurableLabStore::open(&dir.0)?;
            admit(&mut store, &sealed, 1, &[])?;
            if corrupt_bytes {
                store.persist_progress("empty", &proof, &[update(1, "verified")], &[digest(1)])?;
                store.persist_head("empty", &snapshot(60))?;
                store.compact()?;
                fs::write(store.object_path(proof.state_hash), b"corrupt proof")
                    .map_err(|error| io("corrupt terminal fixture", &error))?;
            } else {
                store.retire_commit(sealed.fortress_id, digest(1))?;
                let mut wrong = sealed.anchor();
                wrong.tick = GameTick(99);
                store.append(record(wrong, "verified"), None)?;
            }
        }
        assert!(
            matches!(DurableLabStore::open(&dir.0), Err(error) if error.code == ErrorCode::CorruptLedger)
        );
    }
    Ok(())
}

#[test]
fn terminal_history_obeys_original_eviction_and_collects_only_unreferenced_proofs() -> Result<()> {
    let dir = TempDir::new("eviction");
    let sealed = snapshot(10);
    let proof = snapshot(20);
    let current = snapshot(60);
    let mut store = DurableLabStore::open(&dir.0)?;
    admit(&mut store, &sealed, 1, &[])?;
    store.persist_progress("empty", &proof, &[update(1, "verified")], &[digest(1)])?;
    store.persist_progress_with_objectives("empty", &current, &[], &[], &[digest(1)], &[])?;
    admit(&mut store, &current, 2, &[digest(1)])?;
    assert!(
        store
            .retired_commit(sealed.fortress_id, digest(1))
            .is_none()
    );
    store.compact()?;
    assert!(!store.object_path(proof.state_hash).exists());
    drop(store);
    let reopened = DurableLabStore::open(&dir.0)?;
    assert!(
        reopened
            .retired_commit(sealed.fortress_id, digest(1))
            .is_none()
    );
    assert!(reopened.commit(sealed.fortress_id, digest(2)).is_some());
    Ok(())
}

#[test]
fn failed_atomic_retirement_never_partially_installs_terminal_history() -> Result<()> {
    let dir = TempDir::new("failure");
    let sealed = snapshot(10);
    let proof = snapshot(20);
    {
        let mut store = DurableLabStore::open(&dir.0)?;
        admit(&mut store, &sealed, 1, &[])?;
        store.persist_progress("empty", &sealed, &[update(1, "dispatched")], &[])?;
        store.set_append_budget(Some(0));
        assert!(
            store
                .persist_progress("empty", &proof, &[update(1, "verified")], &[digest(1)])
                .is_err()
        );
        assert!(
            store
                .retired_commit(sealed.fortress_id, digest(1))
                .is_none()
        );
        assert!(store.commit(sealed.fortress_id, digest(1)).is_some());
    }
    let mut reopened = DurableLabStore::open(&dir.0)?;
    assert!(
        reopened
            .retired_commit(sealed.fortress_id, digest(1))
            .is_none()
    );
    reopened.persist_progress("empty", &proof, &[update(1, "verified")], &[digest(1)])?;
    assert_eq!(
        history(&reopened)?.step_anchors.get(&1),
        Some(&proof.anchor())
    );
    Ok(())
}

#[test]
fn terminal_receipt_codec_is_ordered_bounded_and_independent_of_source_size() -> Result<()> {
    let sealed = snapshot(10);
    let prefix = format!("T 23 {}", digest(1));
    let row = format!("1 verified {} 10 1 10", sealed.state_hash);
    for raw in [
        format!("{prefix} 0"),
        format!("{prefix} {}", MAX_STEPS_PER_COMMIT + 1),
        format!("{prefix} 2 {row} {row}"),
        format!("{prefix} 2 {row}"),
        format!("{prefix} 1 {row} trailing"),
        format!("{prefix} 1 1 dispatched {} 10 1 10", sealed.state_hash),
    ] {
        assert!(Record::parse(&raw).is_err(), "accepted {raw}");
    }
    let record = Record::ObjectiveRetiredCommit {
        fortress_id: sealed.fortress_id,
        plan_digest: digest(1),
        steps: (0..MAX_STEPS_PER_COMMIT as u32)
            .map(|step| (step, ("not_dispatched".to_owned(), sealed.anchor())))
            .collect(),
    };
    assert!(record.payload().len() <= MAX_RECORD_BYTES);
    assert_eq!(Record::parse(&record.payload())?, record);
    Ok(())
}
