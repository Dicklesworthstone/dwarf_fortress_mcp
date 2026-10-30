use super::*;

fn acknowledge(store: &mut MonitorStore<Memory>, fixture: &Fixture, tick: u64) -> Result<()> {
    store.begin_read(&fixture.context, &mut admit, &mut guard)?;
    let evidence = fixtures::sample(store.definition().goal(), tick);
    store.publish_readiness(&fixture.context, &evidence, &mut admit, &mut guard)
}

#[test]
fn durable_readiness_preserves_existing_streak_until_a_later_ordinary_sample() -> Result<()> {
    let fixture = fixture()?;
    let disk = Memory::default();
    let mut store = open(&fixture, disk.clone())?;
    acknowledge(&mut store, &fixture, fixtures::TICK)?;
    assert_eq!(
        (store.progress().observations, store.progress().streak),
        (1, 0)
    );
    sample(&mut store, &fixture, fixtures::TICK + 10)?;
    assert_eq!(store.progress().streak, 1);
    acknowledge(&mut store, &fixture, fixtures::TICK + 10)?;
    acknowledge(&mut store, &fixture, fixtures::TICK + 10)?;
    assert_eq!(
        (store.progress().observations, store.progress().streak),
        (4, 1)
    );
    assert!(!store.progress().reading && !store.read_owned());
    sample(&mut store, &fixture, fixtures::TICK + 20)?;
    assert_eq!(
        (store.progress().phase, store.progress().observations),
        ("satisfied", 5)
    );
    let completed = store.progress().clone();
    drop(store);
    let replayed = MonitorStore::open(disk, &fixture.context, BuildMode::Offline, None, false)?;
    assert_eq!(replayed.progress(), &completed);
    Ok(())
}

#[test]
fn readiness_replay_preserves_all_floors_and_budget_but_reopening_resets_unfinished_stability()
-> Result<()> {
    let fixture = fixture()?;
    let disk = Memory::default();
    let mut store = open(&fixture, disk.clone())?;
    sample(&mut store, &fixture, fixtures::TICK)?;
    acknowledge(&mut store, &fixture, fixtures::TICK + 10)?;
    let retained = store.progress().clone();
    let head = store.head();
    let raw = disk.disk.borrow().bytes.clone();
    drop(store);
    let mut reopened = MonitorStore::open(
        disk.clone(),
        &fixture.context,
        BuildMode::Recover,
        None,
        false,
    )?;
    assert_eq!(reopened.head(), head);
    assert_eq!(disk.disk.borrow().bytes, raw);
    assert_eq!(reopened.progress().observations, 2);
    assert_eq!(reopened.progress().last_tick, retained.last_tick);
    assert_eq!(reopened.progress().source, retained.source);
    assert_eq!(reopened.progress().horizons, retained.horizons);
    assert_eq!(reopened.progress().assessments, retained.assessments);
    assert_eq!(reopened.progress().streak, 0);
    let evidence = fixtures::sample(reopened.definition().goal(), fixtures::TICK + 20);
    assert!(
        reopened
            .publish_readiness(&fixture.context, &evidence, &mut admit, &mut guard)
            .is_err()
    );
    acknowledge(&mut reopened, &fixture, fixtures::TICK + 20)?;
    assert_eq!(
        (reopened.progress().observations, reopened.progress().streak),
        (3, 0)
    );
    assert_eq!(reopened.progress().interruptions, 1);
    Ok(())
}

#[test]
fn failed_readiness_sync_fences_live_owner_and_complete_replay_earns_no_credit() -> Result<()> {
    let fixture = fixture()?;
    let disk = Memory::default();
    let mut store = open(&fixture, disk.clone())?;
    sample(&mut store, &fixture, fixtures::TICK)?;
    store.begin_read(&fixture.context, &mut admit, &mut guard)?;
    let evidence = fixtures::sample(store.definition().goal(), fixtures::TICK + 10);
    disk.disk.borrow_mut().fail_sync = true;
    assert!(
        store
            .publish_readiness(&fixture.context, &evidence, &mut admit, &mut guard)
            .is_err()
    );
    assert!(store.is_fenced() && !store.read_owned());
    disk.disk.borrow_mut().fail_sync = false;
    drop(store);
    let recovered = MonitorStore::open(disk, &fixture.context, BuildMode::Recover, None, false)?;
    assert_eq!(
        (
            recovered.progress().observations,
            recovered.progress().streak
        ),
        (2, 0)
    );
    assert_eq!(recovered.progress().last_tick, Some(fixtures::TICK + 10));
    assert!(!recovered.progress().terminal());
    Ok(())
}

#[test]
fn readiness_requires_current_custody_and_cannot_replay_as_an_ordinary_goal_sample() -> Result<()> {
    let fixture = fixture()?;
    let disk = Memory::default();
    let mut store = open(&fixture, disk.clone())?;
    acknowledge(&mut store, &fixture, fixtures::TICK)?;
    acknowledge(&mut store, &fixture, fixtures::TICK + 10)?;
    assert_eq!(store.progress().streak, 0);
    let count = store.progress().observations;
    store.begin_read(&fixture.context, &mut admit, &mut guard)?;
    let evidence = fixtures::sample(store.definition().goal(), fixtures::TICK + 20);
    let mut denied = |_: &Origin, _: &OperationContext| Err(corrupt());
    assert!(
        store
            .publish_readiness(&fixture.context, &evidence, &mut admit, &mut denied)
            .is_err()
    );
    store.abandon_read();
    assert_eq!(store.progress().observations, count);
    assert_eq!(store.progress().streak, 0);
    drop(store);
    let mut offline = MonitorStore::open(disk, &fixture.context, BuildMode::Offline, None, false)?;
    assert_eq!(offline.progress().observations, count);
    assert!(
        offline
            .begin_read(&fixture.context, &mut admit, &mut guard)
            .is_err()
    );
    Ok(())
}
