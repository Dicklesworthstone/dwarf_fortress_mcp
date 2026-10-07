//! Crash recovery must publish the world and its action progress as one fact.
//! These tests use the public store API and real journal byte cuts; they do not
//! mutate the store's in-memory index or rely on the record's encoding.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use dfmcp_core::{Digest32, ErrorCode, FortressId, GameTick, ObservationCursor, StateAnchor};
use dfmcp_lab::durable::{
    DurableLabStore, DurablePlanSource, DurableStepUpdate, MAX_RECORD_BYTES, MAX_STEPS_PER_COMMIT,
};
use dfmcp_world::{WorldGraph, WorldSnapshot};

type TestResult = Result<(), Box<dyn std::error::Error>>;

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct StateDir(PathBuf);

impl StateDir {
    fn new(case: &str) -> Self {
        Self(std::env::temp_dir().join(format!(
            "dfmcp-progress-{case}-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed),
        )))
    }

    fn journal(&self) -> PathBuf {
        self.0.join("journal")
    }

    fn object(&self, snapshot: &WorldSnapshot) -> PathBuf {
        self.0
            .join("objects")
            .join(format!("{}.snap", snapshot.state_hash.to_hex()))
    }
}

impl Drop for StateDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn snapshot(tick: u64, paused: bool) -> WorldSnapshot {
    WorldSnapshot::new(
        FortressId::new(7001),
        GameTick(tick),
        ObservationCursor {
            epoch: 1,
            sequence: tick,
        },
        paused,
        WorldGraph::default(),
    )
}

fn plan(name: &str) -> Digest32 {
    Digest32::of_bytes(name.as_bytes())
}

fn update(digest: Digest32, step: u32, state: &str) -> DurableStepUpdate {
    DurableStepUpdate {
        plan_digest: digest,
        step,
        state: state.to_owned(),
    }
}

fn register(store: &mut DurableLabStore, sealed: &WorldSnapshot, digest: Digest32) -> TestResult {
    store.persist_commit(
        sealed,
        digest,
        44,
        DurablePlanSource::Pause {
            summary: "pause while retaining original progress".to_owned(),
            paused: true,
        },
    )?;
    Ok(())
}

fn head(store: &DurableLabStore) -> Option<StateAnchor> {
    store.head(FortressId::new(7001)).map(|head| head.anchor)
}

fn state(store: &DurableLabStore, digest: Digest32, step: u32) -> Option<&str> {
    store
        .commit(FortressId::new(7001), digest)
        .and_then(|commit| commit.steps.get(&step))
        .map(String::as_str)
}

fn step_anchor(store: &DurableLabStore, digest: Digest32, step: u32) -> Option<StateAnchor> {
    store
        .commit(FortressId::new(7001), digest)
        .and_then(|commit| commit.step_anchors.get(&step))
        .copied()
}

#[test]
fn every_byte_cut_of_mixed_progress_exposes_one_complete_frontier() -> TestResult {
    let dir = StateDir::new("all-byte-cuts");
    let before = snapshot(1, false);
    let after = snapshot(2, true);
    let retained = plan("mixed retained");
    let retired = plan("mixed retired");
    let updates = [
        update(retained, 3, "not_dispatched"),
        update(retired, 1, "cancelled"),
        update(retained, 2, "dispatched"),
        update(retained, 1, "verified"),
    ];
    let base_len;
    {
        let mut store = DurableLabStore::open(&dir.0)?;
        store.persist_head("empty", &before)?;
        register(&mut store, &before, retained)?;
        register(&mut store, &before, retired)?;
        store.persist_step(before.fortress_id, retained, 1, "dispatched")?;
        store.persist_step(before.fortress_id, retired, 1, "dispatched")?;
        base_len = fs::read(dir.journal())?.len();
        let records = store.report().records;
        store.persist_progress("empty", &after, &updates, &[retired])?;
        assert_eq!(store.report().records, records + 1);
    }
    let full = fs::read(dir.journal())?;
    assert!(full.len() > base_len);

    for cut in base_len..=full.len() {
        fs::write(dir.journal(), &full[..cut])?;
        let store = DurableLabStore::open(&dir.0)?;
        if cut == full.len() {
            assert_eq!(head(&store), Some(after.anchor()));
            assert_eq!(state(&store, retained, 1), Some("verified"));
            assert_eq!(state(&store, retained, 2), Some("dispatched"));
            assert_eq!(state(&store, retained, 3), Some("not_dispatched"));
            for step in 1..=3 {
                assert_eq!(step_anchor(&store, retained, step), Some(after.anchor()));
            }
            assert!(store.commit(before.fortress_id, retired).is_none());
            assert_eq!(store.load_snapshot(after.state_hash)?, after);
            assert_eq!(store.report().torn_tail_bytes, 0);
        } else {
            assert_eq!(head(&store), Some(before.anchor()), "cut={cut}");
            assert_eq!(state(&store, retained, 1), Some("dispatched"), "cut={cut}");
            assert_eq!(state(&store, retained, 2), None, "cut={cut}");
            assert_eq!(state(&store, retained, 3), None, "cut={cut}");
            assert_eq!(state(&store, retired, 1), Some("dispatched"), "cut={cut}");
            assert_eq!(step_anchor(&store, retained, 1), None, "cut={cut}");
            assert_eq!(store.load_snapshot(before.state_hash)?, before);
            assert_eq!(store.report().torn_tail_bytes as usize, cut - base_len);
        }
    }
    Ok(())
}

#[test]
fn failed_pause_publication_cannot_leave_verified_ahead_of_world() -> TestResult {
    let dir = StateDir::new("pause-crash");
    let before = snapshot(10, false);
    let after = snapshot(11, true);
    let digest = plan("immediate pause");
    let updates = [update(digest, 1, "verified")];
    {
        let mut store = DurableLabStore::open(&dir.0)?;
        store.persist_head("empty", &before)?;
        register(&mut store, &before, digest)?;
        store.set_append_budget(Some(0));
        assert!(
            store
                .persist_progress("empty", &after, &updates, &[])
                .is_err()
        );
        assert_eq!(head(&store), Some(before.anchor()));
        assert_eq!(state(&store, digest, 1), None);
    }
    {
        let mut store = DurableLabStore::open(&dir.0)?;
        assert_eq!(head(&store), Some(before.anchor()));
        assert!(!store.load_snapshot(before.state_hash)?.paused);
        assert_eq!(state(&store, digest, 1), None);
        store.persist_progress("empty", &after, &updates, &[])?;
    }
    let store = DurableLabStore::open(&dir.0)?;
    assert_eq!(state(&store, digest, 1), Some("verified"));
    assert_eq!(step_anchor(&store, digest, 1), Some(after.anchor()));
    assert_eq!(head(&store), Some(after.anchor()));
    assert_eq!(store.load_snapshot(after.state_hash)?, after);
    Ok(())
}

#[test]
fn invalid_trailing_update_rejects_all_changes_before_object_or_journal_write() -> TestResult {
    for case in [
        "unknown_plan",
        "unknown_state",
        "duplicate_step",
        "duplicate_retirement",
    ] {
        let dir = StateDir::new(case);
        let before = snapshot(20, false);
        let after = snapshot(21, true);
        let digest = plan("valid first update");
        let mut store = DurableLabStore::open(&dir.0)?;
        store.persist_head("empty", &before)?;
        register(&mut store, &before, digest)?;
        let bytes = fs::read(dir.journal())?;
        let report = store.report();
        let mut updates = vec![update(digest, 1, "verified")];
        let retired = if case == "duplicate_retirement" {
            vec![digest, digest]
        } else {
            Vec::new()
        };
        match case {
            "unknown_plan" => updates.push(update(plan("unknown"), 1, "dispatched")),
            "unknown_state" => updates.push(update(digest, 2, "assumed_complete")),
            "duplicate_step" => updates.push(update(digest, 1, "failed")),
            _ => {}
        }
        assert!(
            store
                .persist_progress("empty", &after, &updates, &retired)
                .is_err(),
            "case={case}"
        );
        assert_eq!(head(&store), Some(before.anchor()), "case={case}");
        assert_eq!(state(&store, digest, 1), None, "case={case}");
        assert_eq!(store.report(), report, "case={case}");
        assert_eq!(fs::read(dir.journal())?, bytes, "case={case}");
        assert!(!dir.object(&after).exists(), "case={case}");
        drop(store);
        let store = DurableLabStore::open(&dir.0)?;
        assert_eq!(head(&store), Some(before.anchor()));
        assert_eq!(state(&store, digest, 1), None);
    }
    Ok(())
}

#[test]
fn identical_progress_is_noop_even_with_no_append_budget_and_different_input_order() -> TestResult {
    let dir = StateDir::new("idempotent");
    let before = snapshot(30, false);
    let after = snapshot(31, true);
    let digest = plan("idempotent progress");
    let mut updates = vec![
        update(digest, 2, "dispatched"),
        update(digest, 1, "verified"),
    ];
    let mut store = DurableLabStore::open(&dir.0)?;
    store.persist_head("empty", &before)?;
    register(&mut store, &before, digest)?;
    store.persist_progress("empty", &after, &updates, &[])?;
    let report = store.report();
    let bytes = fs::read(dir.journal())?;
    updates.reverse();
    store.set_append_budget(Some(0));
    store.persist_progress("empty", &after, &updates, &[])?;
    assert_eq!(store.report(), report);
    assert_eq!(fs::read(dir.journal())?, bytes);
    Ok(())
}

#[test]
fn compacted_progress_preserves_world_steps_retirement_and_sealed_basis() -> TestResult {
    let dir = StateDir::new("compaction");
    let before = snapshot(40, false);
    let after = snapshot(41, true);
    let retained = plan("retained during compaction");
    let retired = plan("retired during compaction");
    {
        let mut store = DurableLabStore::open(&dir.0)?;
        store.persist_head("empty", &before)?;
        register(&mut store, &before, retained)?;
        register(&mut store, &before, retired)?;
        store.persist_progress(
            "empty",
            &after,
            &[
                update(retained, 1, "verified"),
                update(retained, 2, "dispatched"),
            ],
            &[retired],
        )?;
        store.compact()?;
        assert_eq!(store.report().compactions, 1);
    }
    let mut store = DurableLabStore::open(&dir.0)?;
    assert_eq!(head(&store), Some(after.anchor()));
    assert_eq!(state(&store, retained, 1), Some("verified"));
    assert_eq!(state(&store, retained, 2), Some("dispatched"));
    assert_eq!(step_anchor(&store, retained, 1), Some(after.anchor()));
    assert_eq!(step_anchor(&store, retained, 2), Some(after.anchor()));
    assert!(store.commit(before.fortress_id, retired).is_none());
    let commit = store
        .commit(before.fortress_id, retained)
        .ok_or("retained plan lost")?;
    assert_eq!(commit.sealed_state_hash, before.state_hash);
    assert_eq!(store.load_snapshot(commit.sealed_state_hash)?, before);
    assert_eq!(store.load_snapshot(after.state_hash)?, after);
    // Another compaction is representation-only and cannot change the frontier.
    store.compact()?;
    drop(store);
    let store = DurableLabStore::open(&dir.0)?;
    assert_eq!(head(&store), Some(after.anchor()));
    assert_eq!(state(&store, retained, 1), Some("verified"));
    assert_eq!(state(&store, retained, 2), Some("dispatched"));
    assert_eq!(step_anchor(&store, retained, 1), Some(after.anchor()));
    assert_eq!(step_anchor(&store, retained, 2), Some(after.anchor()));
    Ok(())
}

#[test]
fn later_world_does_not_reanchor_terminal_evidence_and_compaction_retains_its_snapshot()
-> TestResult {
    let dir = StateDir::new("immutable-proof");
    let sealed = snapshot(42, false);
    let proved = snapshot(43, true);
    let later = snapshot(49, false);
    let digest = plan("historical terminal proof");
    {
        let mut store = DurableLabStore::open(&dir.0)?;
        store.persist_head("empty", &sealed)?;
        register(&mut store, &sealed, digest)?;
        store.persist_progress(
            "empty",
            &proved,
            &[
                update(digest, 1, "verified"),
                update(digest, 2, "dispatched"),
            ],
            &[],
        )?;
        store.persist_progress(
            "empty",
            &later,
            &[update(digest, 1, "verified"), update(digest, 2, "failed")],
            &[],
        )?;
        assert_eq!(head(&store), Some(later.anchor()));
        assert_eq!(step_anchor(&store, digest, 1), Some(proved.anchor()));
        assert_eq!(step_anchor(&store, digest, 2), Some(later.anchor()));
        store.compact()?;
    }
    let store = DurableLabStore::open(&dir.0)?;
    assert_eq!(head(&store), Some(later.anchor()));
    assert_eq!(state(&store, digest, 1), Some("verified"));
    assert_eq!(step_anchor(&store, digest, 1), Some(proved.anchor()));
    assert_eq!(step_anchor(&store, digest, 2), Some(later.anchor()));
    assert_eq!(store.load_snapshot(proved.state_hash)?, proved);
    assert_eq!(store.load_snapshot(sealed.state_hash)?, sealed);
    assert_eq!(store.load_snapshot(later.state_hash)?, later);
    Ok(())
}

#[test]
fn torn_progress_can_be_retried_without_duplicate_retirement_or_step_state() -> TestResult {
    let dir = StateDir::new("torn-retry");
    let before = snapshot(50, false);
    let after = snapshot(51, true);
    let retained = plan("retry retained");
    let retired = plan("retry retired");
    let updates = [
        update(retained, 1, "verified"),
        update(retained, 2, "dispatched"),
    ];
    let base_len;
    {
        let mut store = DurableLabStore::open(&dir.0)?;
        store.persist_head("empty", &before)?;
        register(&mut store, &before, retained)?;
        register(&mut store, &before, retired)?;
        base_len = fs::read(dir.journal())?.len();
        store.persist_progress("empty", &after, &updates, &[retired])?;
    }
    let full = fs::read(dir.journal())?;
    fs::write(dir.journal(), &full[..full.len() - 1])?;
    {
        let mut store = DurableLabStore::open(&dir.0)?;
        assert_eq!(head(&store), Some(before.anchor()));
        assert!(store.commit(before.fortress_id, retired).is_some());
        assert_eq!(state(&store, retained, 1), None);
        assert_eq!(fs::read(dir.journal())?.len(), base_len);
        store.persist_progress("empty", &after, &updates, &[retired])?;
    }
    assert_eq!(fs::read(dir.journal())?, full);
    let store = DurableLabStore::open(&dir.0)?;
    assert_eq!(head(&store), Some(after.anchor()));
    assert_eq!(state(&store, retained, 1), Some("verified"));
    assert!(store.commit(before.fortress_id, retired).is_none());
    assert_eq!(store.report().torn_tail_bytes, 0);
    Ok(())
}

#[test]
fn input_order_cannot_change_canonical_progress_bytes() -> TestResult {
    let first = StateDir::new("ordered-one");
    let second = StateDir::new("ordered-two");
    let before = snapshot(60, false);
    let after = snapshot(61, true);
    let a = plan("canonical a");
    let b = plan("canonical b");
    let c = plan("canonical c");
    let mut updates = vec![
        update(a, 3, "failed"),
        update(a, 1, "verified"),
        update(a, 2, "dispatched"),
    ];
    let mut retired = vec![b, c];
    for dir in [&first, &second] {
        let mut store = DurableLabStore::open(&dir.0)?;
        store.persist_head("empty", &before)?;
        for digest in [a, b, c] {
            register(&mut store, &before, digest)?;
        }
        store.persist_progress("empty", &after, &updates, &retired)?;
        updates.reverse();
        retired.reverse();
    }
    assert_eq!(fs::read(first.journal())?, fs::read(second.journal())?);
    Ok(())
}

#[test]
fn progress_with_unchanged_world_still_persists_new_step_evidence() -> TestResult {
    let dir = StateDir::new("same-world");
    let observed = snapshot(70, true);
    let digest = plan("new proof same observed world");
    {
        let mut store = DurableLabStore::open(&dir.0)?;
        store.persist_head("empty", &observed)?;
        register(&mut store, &observed, digest)?;
        let records = store.report().records;
        store.persist_progress("empty", &observed, &[update(digest, 1, "verified")], &[])?;
        assert_eq!(store.report().records, records + 1);
    }
    let store = DurableLabStore::open(&dir.0)?;
    assert_eq!(head(&store), Some(observed.anchor()));
    assert_eq!(state(&store, digest, 1), Some("verified"));
    Ok(())
}

#[test]
fn valid_frontier_larger_than_legacy_record_limit_survives_reopen_and_compaction() -> TestResult {
    let dir = StateDir::new("large-frontier");
    let before = snapshot(80, false);
    let after = snapshot(81, true);
    let digests: Vec<_> = (0..9)
        .map(|index| plan(&format!("large-{index}")))
        .collect();
    let mut updates = Vec::new();
    {
        let mut store = DurableLabStore::open(&dir.0)?;
        store.persist_head("empty", &before)?;
        for digest in &digests {
            register(&mut store, &before, *digest)?;
            for step in 1..=MAX_STEPS_PER_COMMIT {
                updates.push(update(*digest, u32::try_from(step)?, "dispatched"));
            }
        }
        let prefix = fs::read(dir.journal())?.len();
        store.persist_progress("empty", &after, &updates, &[])?;
        let appended = fs::read(dir.journal())?.len() - prefix;
        assert!(appended > MAX_RECORD_BYTES + 65);
    }
    let mut store = DurableLabStore::open(&dir.0)?;
    assert_eq!(head(&store), Some(after.anchor()));
    for digest in &digests {
        assert_eq!(
            store
                .commit(before.fortress_id, *digest)
                .map(|commit| commit.steps.len()),
            Some(MAX_STEPS_PER_COMMIT)
        );
    }
    store.compact()?;
    drop(store);
    let store = DurableLabStore::open(&dir.0)?;
    assert_eq!(head(&store), Some(after.anchor()));
    for update in &updates {
        assert_eq!(
            state(&store, update.plan_digest, update.step),
            Some("dispatched")
        );
    }
    Ok(())
}

#[test]
fn unchanged_anchor_cannot_bypass_snapshot_validation_on_noop_progress() -> TestResult {
    let dir = StateDir::new("forged-noop");
    let observed = snapshot(90, false);
    let digest = plan("validate even no-op");
    let updates = [update(digest, 1, "verified")];
    let mut store = DurableLabStore::open(&dir.0)?;
    register(&mut store, &observed, digest)?;
    store.persist_progress("empty", &observed, &updates, &[])?;
    let report = store.report();
    let bytes = fs::read(dir.journal())?;
    let mut forged = observed.clone();
    forged.paused = true;
    assert_eq!(forged.anchor(), observed.anchor());
    assert!(!forged.hash_is_valid());
    assert!(
        store
            .persist_progress("empty", &forged, &updates, &[])
            .is_err()
    );
    assert_eq!(store.report(), report);
    assert_eq!(fs::read(dir.journal())?, bytes);
    assert_eq!(store.load_snapshot(observed.state_hash)?, observed);
    Ok(())
}

#[test]
fn retirement_with_unresolved_dispatch_refuses_the_entire_new_frontier() -> TestResult {
    let dir = StateDir::new("unfinished-retirement");
    let before = snapshot(100, false);
    let after = snapshot(101, true);
    let digest = plan("unfinished retirement");
    let mut store = DurableLabStore::open(&dir.0)?;
    register(&mut store, &before, digest)?;
    store.persist_progress("empty", &before, &[update(digest, 1, "dispatched")], &[])?;
    let report = store.report();
    let bytes = fs::read(dir.journal())?;
    // Another step has a valid final update, but step1 remains unresolved.
    assert!(
        store
            .persist_progress("empty", &after, &[update(digest, 2, "verified")], &[digest],)
            .is_err()
    );
    assert_eq!(head(&store), Some(before.anchor()));
    assert_eq!(state(&store, digest, 1), Some("dispatched"));
    assert_eq!(state(&store, digest, 2), None);
    assert_eq!(store.report(), report);
    assert_eq!(fs::read(dir.journal())?, bytes);
    assert!(!dir.object(&after).exists());
    drop(store);
    let store = DurableLabStore::open(&dir.0)?;
    assert_eq!(head(&store), Some(before.anchor()));
    assert_eq!(state(&store, digest, 1), Some("dispatched"));
    assert_eq!(state(&store, digest, 2), None);
    Ok(())
}

#[test]
fn valid_object_hash_cannot_certify_a_different_anchored_step_tick() -> TestResult {
    let dir = StateDir::new("wrong-proof-anchor");
    let observed = snapshot(110, true);
    let digest = plan("wrong anchor with valid content hash");
    {
        let mut store = DurableLabStore::open(&dir.0)?;
        register(&mut store, &observed, digest)?;
        store.persist_progress("empty", &observed, &[update(digest, 1, "verified")], &[])?;
        store.compact()?;
    }
    // Repair every chain digest after changing only the A-record tick. The
    // content-addressed snapshot remains valid, so rejection must come from
    // anchor-to-object equality, independently of hash-chain validation.
    let source = fs::read_to_string(dir.journal())?;
    let mut chain = Digest32::ZERO;
    let mut altered = String::new();
    let mut changes = 0;
    for line in source.lines() {
        let (_, payload) = line.split_once(' ').ok_or("journal line lacks payload")?;
        let payload = if payload.starts_with("A ") {
            let mut fields: Vec<_> = payload.split(' ').map(str::to_owned).collect();
            assert_eq!(fields.len(), 9);
            assert_eq!(fields[5], observed.state_hash.to_hex());
            fields[6] = (observed.tick.0 + 1).to_string();
            changes += 1;
            fields.join(" ")
        } else {
            payload.to_owned()
        };
        let mut material = b"dfmcp-lab-journal/1\0".to_vec();
        material.extend_from_slice(chain.as_bytes());
        material.extend_from_slice(payload.as_bytes());
        chain = Digest32::of_bytes(&material);
        altered.push_str(&format!("{} {payload}\n", chain.to_hex()));
    }
    assert_eq!(changes, 1);
    fs::write(dir.journal(), altered)?;
    let error = DurableLabStore::open(&dir.0)
        .err()
        .ok_or("incorrect proof anchor accepted")?;
    assert_eq!(error.code, ErrorCode::CorruptLedger);
    assert!(error.message.contains("step anchor"), "{}", error.message);
    Ok(())
}
