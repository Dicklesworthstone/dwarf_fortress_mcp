//! Exact original reserve endpoints survive restart without becoming current truth.

use super::*;
use dfmcp_world::WorldGraph;

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-reserve-observation-{name}-{}-{:?}",
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
        FortressId::new(17),
        GameTick(tick),
        ObservationCursor {
            epoch: 1,
            sequence: tick,
        },
        false,
        WorldGraph::default(),
    )
}

fn plan(number: u64) -> Digest32 {
    Digest32::of_bytes(&number.to_be_bytes())
}

fn admit(
    store: &mut DurableLabStore,
    sealed: &WorldSnapshot,
    digest: Digest32,
    evicted: &[Digest32],
) -> Result<()> {
    store.persist_objective_commit(
        sealed,
        digest,
        41,
        DurablePlanSource::Production {
            summary: "original finite future reserve".to_owned(),
            raw: r#"{"template":"production","planner":"consumption_reserve_v1","quotas":[{"item":"DRINK","minimum":40}],"reserve_until_tick":30}"#.to_owned(),
        },
        SessionId::new(9),
        evicted,
    )
}

fn append_raw(path: &Path, payload: &str) -> Result<()> {
    let mut bytes = fs::read(path).map_err(|error| io("read test journal", &error))?;
    let text = std::str::from_utf8(&bytes).map_err(|_| corrupt("test journal is not UTF-8"))?;
    let previous = text
        .lines()
        .last()
        .and_then(|line| line.split_once(' '))
        .and_then(|(hash, _)| Digest32::from_hex(hash))
        .ok_or_else(|| corrupt("test journal has no chain"))?;
    let chain = chain_next(previous, payload);
    bytes.extend_from_slice(format!("{chain} {payload}\n").as_bytes());
    fs::write(path, bytes).map_err(|error| io("write test journal", &error))
}

#[test]
fn exact_reserve_endpoint_survives_retirement_compaction_and_restart() -> Result<()> {
    let dir = TempDir::new("retained");
    let sealed = snapshot(10);
    let endpoint = snapshot(30);
    let current = snapshot(60);
    let digest = plan(1);
    {
        let mut store = DurableLabStore::open(&dir.0)?;
        admit(&mut store, &sealed, digest, &[])?;
        store.persist_head("empty", &endpoint)?;
        store.persist_objective_reserve_observation(digest, &endpoint)?;
        store.persist_progress_with_objectives("empty", &current, &[], &[digest], &[], &[])?;
        assert!(store.commit(sealed.fortress_id, digest).is_none());
        store.compact()?;
    }
    let mut reopened = DurableLabStore::open(&dir.0)?;
    assert_eq!(
        reopened.objective_reserve_observation(sealed.fortress_id, digest),
        Some(endpoint.anchor())
    );
    assert_eq!(reopened.load_snapshot(endpoint.state_hash)?, endpoint);
    assert_eq!(
        reopened.head(sealed.fortress_id).map(|head| head.anchor),
        Some(current.anchor())
    );
    assert!(
        reopened
            .objective(sealed.fortress_id, digest)
            .is_some_and(|goal| goal.first_satisfied_anchor.is_none())
    );
    let records = reopened.report().records;
    reopened.persist_objective_reserve_observation(digest, &endpoint)?;
    assert_eq!(reopened.report().records, records);
    assert!(reopened.commit(sealed.fortress_id, digest).is_none());
    Ok(())
}

#[test]
fn reserve_capture_requires_the_exact_published_original_frontier() -> Result<()> {
    let dir = TempDir::new("admission");
    let sealed = snapshot(10);
    let endpoint = snapshot(30);
    let digest = plan(1);
    let mut store = DurableLabStore::open(&dir.0)?;
    admit(&mut store, &sealed, digest, &[])?;
    let before = store.report();
    assert!(
        store
            .persist_objective_reserve_observation(digest, &endpoint)
            .is_err()
    );
    assert_eq!(store.report(), before);
    store.persist_head("empty", &endpoint)?;
    let before = store.report();
    let mut same_tick = endpoint.clone();
    same_tick.paused = true;
    same_tick.refresh_hash();
    let mut foreign = endpoint.clone();
    foreign.fortress_id = FortressId::new(18);
    foreign.refresh_hash();
    let mut invalid_hash = endpoint.clone();
    invalid_hash.paused = true;
    for (candidate, selected) in [
        (same_tick, digest),
        (foreign, digest),
        (invalid_hash, digest),
        (endpoint.clone(), plan(2)),
        (sealed.clone(), digest),
    ] {
        assert!(
            store
                .persist_objective_reserve_observation(selected, &candidate)
                .is_err()
        );
        assert_eq!(store.report(), before);
        assert_eq!(
            store.objective_reserve_observation(sealed.fortress_id, digest),
            None
        );
    }
    let prior = snapshot(5);
    store.persist_head("empty", &prior)?;
    assert!(
        store
            .persist_objective_reserve_observation(digest, &prior)
            .is_err()
    );
    Ok(())
}

#[test]
fn first_endpoint_cannot_be_replaced_by_later_or_same_tick_stock() -> Result<()> {
    let dir = TempDir::new("immutable");
    let sealed = snapshot(10);
    let endpoint = snapshot(30);
    let digest = plan(1);
    let mut store = DurableLabStore::open(&dir.0)?;
    admit(&mut store, &sealed, digest, &[])?;
    store.persist_head("empty", &endpoint)?;
    store.persist_objective_reserve_observation(digest, &endpoint)?;
    let mut replacement = endpoint.clone();
    replacement.cursor.sequence += 1;
    replacement.paused = true;
    replacement.refresh_hash();
    for candidate in [replacement, snapshot(40)] {
        store.persist_head("empty", &candidate)?;
        let records = store.report().records;
        assert!(
            matches!(store.persist_objective_reserve_observation(digest, &candidate), Err(error) if error.code == ErrorCode::Conflict)
        );
        assert_eq!(store.report().records, records);
        assert_eq!(
            store.objective_reserve_observation(sealed.fortress_id, digest),
            Some(endpoint.anchor())
        );
    }
    Ok(())
}

#[test]
fn unpublished_and_torn_reserve_records_never_manufacture_evidence() -> Result<()> {
    for torn in [false, true] {
        let dir = TempDir::new(if torn { "torn" } else { "unpublished" });
        let sealed = snapshot(10);
        let endpoint = snapshot(30);
        let digest = plan(1);
        let bytes_before;
        {
            let mut store = DurableLabStore::open(&dir.0)?;
            admit(&mut store, &sealed, digest, &[])?;
            store.persist_head("empty", &endpoint)?;
            bytes_before = fs::metadata(dir.0.join("journal"))
                .map_err(|error| io("inspect test journal", &error))?
                .len();
            if torn {
                store.persist_objective_reserve_observation(digest, &endpoint)?;
            } else {
                store.set_append_budget(Some(0));
                assert!(
                    store
                        .persist_objective_reserve_observation(digest, &endpoint)
                        .is_err()
                );
                assert_eq!(
                    store.objective_reserve_observation(sealed.fortress_id, digest),
                    None
                );
            }
        }
        if torn {
            OpenOptions::new()
                .write(true)
                .open(dir.0.join("journal"))
                .and_then(|file| file.set_len(bytes_before + 80))
                .map_err(|error| io("tear reserve test append", &error))?;
        }
        let mut reopened = DurableLabStore::open(&dir.0)?;
        assert_eq!(
            reopened.objective_reserve_observation(sealed.fortress_id, digest),
            None
        );
        assert_eq!(
            reopened.head(sealed.fortress_id).map(|head| head.anchor),
            Some(endpoint.anchor())
        );
        assert_eq!(reopened.report().torn_tail_bytes, if torn { 80 } else { 0 });
        // The same current exact endpoint may be observed again. This does not
        // invent an unobserved past sample or change the original request.
        reopened.persist_objective_reserve_observation(digest, &endpoint)?;
    }
    Ok(())
}

#[test]
fn restore_retains_history_but_refuses_fresh_endpoint_custody() -> Result<()> {
    let dir = TempDir::new("restore");
    let sealed = snapshot(10);
    let endpoint = snapshot(30);
    let mut restored = snapshot(5);
    restored.cursor.epoch = 2;
    restored.refresh_hash();
    let digest = plan(1);
    {
        let mut store = DurableLabStore::open(&dir.0)?;
        admit(&mut store, &sealed, digest, &[])?;
        store.persist_head("empty", &endpoint)?;
        store.persist_objective_reserve_observation(digest, &endpoint)?;
        store.persist_progress_with_objectives(
            "empty",
            &restored,
            &[],
            &[digest],
            &[],
            &[digest],
        )?;
        assert!(
            store
                .persist_objective_reserve_observation(digest, &endpoint)
                .is_err()
        );
        store.compact()?;
    }
    let reopened = DurableLabStore::open(&dir.0)?;
    assert_eq!(
        reopened.objective_reserve_observation(sealed.fortress_id, digest),
        Some(endpoint.anchor())
    );
    assert_eq!(reopened.load_snapshot(endpoint.state_hash)?, endpoint);
    assert_eq!(
        reopened
            .objective(sealed.fortress_id, digest)
            .and_then(|goal| goal.restore_abandoned_anchor),
        Some(restored.anchor())
    );
    Ok(())
}

#[test]
fn eligible_original_eviction_releases_only_its_reserve_snapshot() -> Result<()> {
    let dir = TempDir::new("eviction");
    let sealed = snapshot(10);
    let endpoint = snapshot(30);
    let current = snapshot(60);
    let digest = plan(1);
    {
        let mut store = DurableLabStore::open(&dir.0)?;
        admit(&mut store, &sealed, digest, &[])?;
        store.persist_head("empty", &endpoint)?;
        store.persist_objective_reserve_observation(digest, &endpoint)?;
        store.persist_progress_with_objectives(
            "empty",
            &current,
            &[],
            &[digest],
            &[digest],
            &[],
        )?;
        admit(&mut store, &current, plan(2), &[digest])?;
        assert_eq!(
            store.objective_reserve_observation(sealed.fortress_id, digest),
            None
        );
        store.compact()?;
        assert!(!store.object_path(endpoint.state_hash).exists());
    }
    let reopened = DurableLabStore::open(&dir.0)?;
    assert_eq!(
        reopened.objective_reserve_observation(sealed.fortress_id, digest),
        None
    );
    assert!(reopened.objective(sealed.fortress_id, plan(2)).is_some());
    Ok(())
}

#[test]
fn complete_conflicting_or_duplicate_reserve_records_refuse_reopen() -> Result<()> {
    for duplicate in [false, true] {
        let dir = TempDir::new(if duplicate { "duplicate" } else { "mismatch" });
        let sealed = snapshot(10);
        let endpoint = snapshot(30);
        let digest = plan(1);
        {
            let mut store = DurableLabStore::open(&dir.0)?;
            admit(&mut store, &sealed, digest, &[])?;
            store.persist_head("empty", &endpoint)?;
            if duplicate {
                store.persist_objective_reserve_observation(digest, &endpoint)?;
            }
        }
        let mut anchor = endpoint.anchor();
        if !duplicate {
            anchor.tick = GameTick(31);
        }
        append_raw(
            &dir.0.join("journal"),
            &Record::ObjectiveReserveObservation {
                fortress_id: sealed.fortress_id,
                plan_digest: digest,
                anchor,
            }
            .payload(),
        )?;
        assert!(
            matches!(DurableLabStore::open(&dir.0), Err(error) if error.code == ErrorCode::CorruptLedger)
        );
    }
    Ok(())
}

#[test]
fn corrupt_retained_endpoint_is_rejected_even_when_current_head_is_intact() -> Result<()> {
    let dir = TempDir::new("corruption");
    let sealed = snapshot(10);
    let endpoint = snapshot(30);
    let digest = plan(1);
    {
        let mut store = DurableLabStore::open(&dir.0)?;
        admit(&mut store, &sealed, digest, &[])?;
        store.persist_head("empty", &endpoint)?;
        store.persist_objective_reserve_observation(digest, &endpoint)?;
        store.persist_head("empty", &snapshot(60))?;
        store.compact()?;
    }
    fs::write(
        dir.0
            .join("objects")
            .join(format!("{}.snap", endpoint.state_hash)),
        b"corrupt endpoint",
    )
    .map_err(|error| io("corrupt test endpoint", &error))?;
    assert!(
        matches!(DurableLabStore::open(&dir.0), Err(error) if error.code == ErrorCode::CorruptLedger)
    );
    Ok(())
}
