use super::*;
use dfmcp_world::WorldGraph;

struct TempDir(PathBuf);
impl TempDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-withdrawal-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
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
        FortressId::new(87),
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
fn admit(store: &mut DurableLabStore, world: &WorldSnapshot, number: u64) -> Result<()> {
    store.persist_objective_commit(
        world,
        digest(number),
        u128::from(number + 1),
        DurablePlanSource::Actions {
            summary: "withdraw unwanted original".to_owned(),
            raw: "[]".to_owned(),
        },
        SessionId::new(9),
        &[],
    )
}
fn retire(store: &mut DurableLabStore, world: &WorldSnapshot, number: u64) -> Result<()> {
    store.persist_progress(
        "empty",
        world,
        &[DurableStepUpdate {
            plan_digest: digest(number),
            step: 1,
            state: "cancelled".to_owned(),
        }],
        &[digest(number)],
    )
}
fn withdrawal(
    store: &DurableLabStore,
    world: &WorldSnapshot,
    number: u64,
) -> Result<DurableObjectiveWithdrawal> {
    DurableObjectiveWithdrawal::new(
        store
            .objective(world.fortress_id, digest(number))
            .ok_or_else(|| corrupt("fixture original missing"))?,
        store
            .retired_commit(world.fortress_id, digest(number))
            .ok_or_else(|| corrupt("fixture final receipts missing"))?,
        world.anchor(),
        SessionId::new(11),
        BTreeSet::from([1]),
    )
}

#[test]
fn explicit_withdrawal_releases_active_capacity_without_erasing_failure_history() -> Result<()> {
    let dir = TempDir::new("capacity");
    let world = snapshot(10);
    let saved;
    {
        let mut store = DurableLabStore::open(&dir.0)?;
        for number in 0..MAX_OBJECTIVES_PER_FORTRESS as u64 {
            admit(&mut store, &world, number)?;
            retire(&mut store, &world, number)?;
        }
        assert_eq!(
            admit(&mut store, &world, 65).err().map(|e| e.code),
            Some(ErrorCode::BudgetExceeded)
        );
        let source = store.objective(world.fortress_id, digest(0)).cloned();
        let receipts = store.retired_commit(world.fortress_id, digest(0)).cloned();
        store.persist_objective_reserve_observation(digest(0), &world)?;
        saved = withdrawal(&store, &world, 0)?;
        store.persist_objective_withdrawal(&saved, &world)?;
        assert_eq!(store.index.active_objective_count(world.fortress_id), 63);
        assert_eq!(
            store.objective(world.fortress_id, digest(0)),
            source.as_ref()
        );
        assert_eq!(
            store.retired_commit(world.fortress_id, digest(0)),
            receipts.as_ref()
        );
        assert!(source.is_some_and(|source| source.first_satisfied_anchor.is_none()));
        admit(&mut store, &world, 65)?;
        assert_eq!(store.objectives(world.fortress_id).count(), 65);
        assert_eq!(store.index.active_objective_count(world.fortress_id), 64);
        store.compact()?;
    }
    for _ in 0..2 {
        let mut store = DurableLabStore::open(&dir.0)?;
        assert_eq!(
            store.objective_withdrawal(world.fortress_id, digest(0)),
            Some(&saved)
        );
        assert_eq!(
            store.objective_reserve_observation(world.fortress_id, digest(0)),
            Some(world.anchor())
        );
        assert_eq!(
            store
                .retired_commit(world.fortress_id, digest(0))
                .and_then(|record| record.steps.get(&1))
                .map(String::as_str),
            Some("cancelled")
        );
        assert_eq!(store.index.active_objective_count(world.fortress_id), 64);
        assert!(store.commit(world.fortress_id, digest(0)).is_none());
        store.compact()?;
    }
    Ok(())
}

#[test]
fn withdrawal_is_immutable_and_cannot_create_success_or_replace_an_endpoint() -> Result<()> {
    let dir = TempDir::new("immutable");
    let world = snapshot(10);
    let mut store = DurableLabStore::open(&dir.0)?;
    admit(&mut store, &world, 1)?;
    retire(&mut store, &world, 1)?;
    store.persist_objective_reserve_observation(digest(1), &world)?;
    let saved = withdrawal(&store, &world, 1)?;
    store.persist_objective_withdrawal(&saved, &world)?;
    let records = store.report().records;
    store.persist_objective_withdrawal(&saved, &world)?;
    assert_eq!(store.report().records, records);
    let later = snapshot(20);
    store.persist_head("empty", &later)?;
    let mut different = saved.clone();
    different.anchor = later.anchor();
    assert_eq!(
        store
            .persist_objective_withdrawal(&different, &later)
            .err()
            .map(|e| e.code),
        Some(ErrorCode::Conflict)
    );
    assert!(
        store
            .persist_objective_reserve_observation(digest(1), &later)
            .is_err()
    );
    assert!(
        store
            .persist_progress_with_objectives("empty", &later, &[], &[], &[digest(1)], &[])
            .is_err()
    );
    assert!(
        store
            .persist_progress_with_objectives("empty", &later, &[], &[], &[], &[digest(1)])
            .is_err()
    );
    assert_eq!(
        store.objective_withdrawal(world.fortress_id, digest(1)),
        Some(&saved)
    );
    assert_eq!(
        store.objective_reserve_observation(world.fortress_id, digest(1)),
        Some(world.anchor())
    );
    Ok(())
}

#[test]
fn failed_publication_and_unfinished_or_forged_custody_do_not_archive() -> Result<()> {
    let dir = TempDir::new("refusals");
    let world = snapshot(10);
    let mut store = DurableLabStore::open(&dir.0)?;
    admit(&mut store, &world, 1)?;
    assert!(withdrawal(&store, &world, 1).is_err());
    retire(&mut store, &world, 1)?;
    let saved = withdrawal(&store, &world, 1)?;
    let mut corrupt_proof = saved.clone();
    corrupt_proof.proof_digest = digest(999);
    assert!(
        store
            .persist_objective_withdrawal(&corrupt_proof, &world)
            .is_err()
    );
    let records = store.report().records;
    store.set_append_budget(Some(0));
    assert!(store.persist_objective_withdrawal(&saved, &world).is_err());
    assert!(
        store
            .objective_withdrawal(world.fortress_id, digest(1))
            .is_none()
    );
    assert_eq!(store.report().records, records);
    assert_eq!(store.index.active_objective_count(world.fortress_id), 1);
    store.set_append_budget(None);
    store.persist_objective_withdrawal(&saved, &world)?;
    assert_eq!(
        Record::parse(&saved.payload())?,
        Record::ObjectiveWithdrawal(saved.clone())
    );
    assert!(Record::parse(&format!("{} 1", saved.payload())).is_err());
    assert!(
        store
            .index
            .apply(Record::ObjectiveWithdrawal(saved))
            .is_err()
    );
    Ok(())
}

#[test]
fn archive_has_a_separate_hard_bound_and_never_steals_active_slots() -> Result<()> {
    let dir = TempDir::new("archive-bound");
    let world = snapshot(10);
    let mut store = DurableLabStore::open(&dir.0)?;
    for number in 0..MAX_WITHDRAWN_OBJECTIVES_PER_FORTRESS as u64 {
        admit(&mut store, &world, number)?;
        retire(&mut store, &world, number)?;
        let saved = withdrawal(&store, &world, number)?;
        store.persist_objective_withdrawal(&saved, &world)?;
    }
    assert_eq!(store.index.active_objective_count(world.fortress_id), 0);
    for number in 256..320 {
        admit(&mut store, &world, number)?;
    }
    retire(&mut store, &world, 256)?;
    let extra = withdrawal(&store, &world, 256)?;
    assert_eq!(
        store
            .persist_objective_withdrawal(&extra, &world)
            .err()
            .map(|e| e.code),
        Some(ErrorCode::BudgetExceeded)
    );
    assert_eq!(
        store.objectives(world.fortress_id).count(),
        MAX_RETAINED_OBJECTIVES_PER_FORTRESS
    );
    assert_eq!(
        store.index.active_objective_count(world.fortress_id),
        MAX_OBJECTIVES_PER_FORTRESS
    );
    assert_eq!(
        admit(&mut store, &world, 321).err().map(|e| e.code),
        Some(ErrorCode::BudgetExceeded)
    );
    store.compact()?;
    drop(store);
    let store = DurableLabStore::open(&dir.0)?;
    assert_eq!(
        store.objectives(world.fortress_id).count(),
        MAX_RETAINED_OBJECTIVES_PER_FORTRESS
    );
    assert_eq!(
        store.index.active_objective_count(world.fortress_id),
        MAX_OBJECTIVES_PER_FORTRESS
    );
    Ok(())
}
