//! Joint clock readiness shares all original conditions, with no proof credit.
use super::*;

fn retimed(definition: &Definition, timing: Timing) -> Result<Definition> {
    Definition::new(MonitorDefinition::new(
        definition.origin().clone(),
        Goal::new(definition.goal().records().to_vec(), timing)?,
    )?)
}

fn acknowledge(
    state: &Progress,
    definition: &Definition,
    sample: &Sample,
    context: &OperationContext,
) -> Result<Progress> {
    readiness::advance(&state.begin_read()?, definition, sample, context)
}

fn retain(
    store: &mut MonitorStore<Memory>,
    context: &OperationContext,
    sample: &Sample,
) -> Result<()> {
    store.begin_read(context, &mut admit, &mut guard)?;
    store.publish_readiness(context, sample, &mut admit, &mut guard)
}

#[test]
fn complete_joint_readiness_preserves_one_streak_without_earning_goal_samples() -> Result<()> {
    for count in [2, 32] {
        let (definition, c) = fixture(count)?;
        let first = acknowledge(
            &Progress::new(&definition),
            &definition,
            &evidence(&definition, fixtures::TICK)?,
            &c,
        )?;
        assert_eq!((first.observations, first.streak), (1, 0));
        assert_eq!((first.first_tick, first.counted_tick), (None, None));
        assert!(first.terrain.condition_met());
        assert_eq!(first.condition_met_count(), count);
        assert!(!first.reading && !first.terminal());

        let sample = evidence(&definition, fixtures::TICK + 10)?;
        let candidate = step(&first, &definition, &sample, &c)?;
        assert_eq!((candidate.observations, candidate.streak), (2, 1));
        let prepared = acknowledge(&candidate, &definition, &sample, &c)?;
        let committed = acknowledge(&prepared, &definition, &sample, &c)?;
        assert_eq!((committed.observations, committed.streak), (4, 1));
        assert_eq!(committed.phase, "candidate");
        assert_eq!(committed.first_tick, candidate.first_tick);
        assert_eq!(committed.counted_tick, candidate.counted_tick);

        let completed = step(
            &committed,
            &definition,
            &evidence(&definition, fixtures::TICK + 20)?,
            &c,
        )?;
        assert_eq!(
            (completed.phase, completed.observations, completed.streak),
            ("satisfied", 5, 2)
        );
        assert_eq!(completed.reason, "joint_original_room_sampled_condition");
        assert_eq!(completed.terrain.furnishings.matched, count as u32);
    }
    let (original, c) = fixture(2)?;
    let definition = retimed(
        &original,
        Timing {
            interval: 1,
            stable_samples: 2,
            stable_span: 1,
            ..original.goal().timing()
        },
    )?;
    let retained = acknowledge(
        &Progress::new(&definition),
        &definition,
        &evidence(&definition, fixtures::TICK)?,
        &c,
    )?;
    assert_eq!((retained.phase, retained.streak), ("active", 0));
    assert_eq!(
        step(
            &retained,
            &definition,
            &evidence(&definition, fixtures::TICK)?,
            &c
        )?
        .streak,
        0
    );
    let first = step(
        &retained,
        &definition,
        &evidence(&definition, fixtures::TICK + 1)?,
        &c,
    )?;
    assert_eq!((first.phase, first.streak), ("candidate", 1));
    assert_eq!(
        step(
            &first,
            &definition,
            &evidence(&definition, fixtures::TICK + 2)?,
            &c
        )?
        .phase,
        "satisfied"
    );
    Ok(())
}

#[test]
fn probe_does_not_charge_a_second_acquisition_and_readiness_cannot_spend_past_original_cap()
-> Result<()> {
    let (original, c) = fixture(2)?;
    let definition = retimed(
        &original,
        Timing {
            max_observations: 4,
            ..original.goal().timing()
        },
    )?;
    let first = step(
        &Progress::new(&definition),
        &definition,
        &evidence(&definition, fixtures::TICK)?,
        &c,
    )?;
    let before = acknowledge(
        &first,
        &definition,
        &evidence(&definition, fixtures::TICK)?,
        &c,
    )?;
    let sample = evidence(&definition, fixtures::TICK + 10)?;
    let after = acknowledge(&before, &definition, &sample, &c)?;
    assert_eq!((before.observations, after.observations), (2, 3));
    for progress in [&before, &after] {
        let checked = readiness::probe(&progress.begin_read()?, &definition, &sample, &c)?;
        assert_eq!(checked.observations, progress.observations);
        assert_eq!(checked.streak, 1);
        assert_eq!(checked.first_tick, first.first_tick);
        assert_eq!(checked.counted_tick, first.counted_tick);
        assert_eq!(checked.phase, "candidate");
    }
    let exhausted = acknowledge(
        &after,
        &definition,
        &evidence(&definition, fixtures::TICK + 20)?,
        &c,
    )?;
    assert_eq!(
        (exhausted.phase, exhausted.reason, exhausted.observations),
        ("expired", "sample_budget_exhausted", 4)
    );
    assert_eq!(exhausted.streak, 0);
    assert!(exhausted.begin_read().is_err());

    // One initial ordinary sample plus preparation/commit readiness must fit
    // the original allowance; no clock request silently extends that lifetime.
    for cap in [2, 3] {
        let limited = retimed(
            &original,
            Timing {
                max_observations: cap,
                ..original.goal().timing()
            },
        )?;
        let sample = evidence(&limited, fixtures::TICK)?;
        let mut progress = step(&Progress::new(&limited), &limited, &sample, &c)?;
        for _ in 1..cap {
            progress = acknowledge(&progress, &limited, &sample, &c)?;
        }
        assert_eq!((progress.phase, progress.observations), ("expired", cap));
        assert_eq!(progress.reason, "sample_budget_exhausted");
        assert_eq!(
            (progress.streak, progress.first_tick, progress.counted_tick),
            (0, None, None)
        );
    }
    Ok(())
}

#[test]
fn every_joint_requirement_withdraws_continuity_including_the_last_of_32_targets() -> Result<()> {
    let (definition, c) = fixture(32)?;
    let candidate = step(
        &Progress::new(&definition),
        &definition,
        &evidence(&definition, fixtures::TICK)?,
        &c,
    )?;
    let floor = definition.room().terrain().floors()[0];
    let wall = definition.room().terrain().walls()[0];
    let last = definition
        .goal()
        .records()
        .last()
        .ok_or_else(corrupt)?
        .insertion()
        .ok_or_else(corrupt)?
        .position();
    for variant in 0..8 {
        let mut sample = evidence(&definition, fixtures::TICK + 10)?;
        if variant == 7 {
            let mut operations = sample.operations()?;
            operations.items.last_mut().ok_or_else(corrupt)?.flags |= 1 << 1;
            sample.furnishings = fixtures::with_observation(definition.goal(), operations);
        } else {
            let target = match variant {
                4 | 5 => wall,
                6 => last,
                _ => floor,
            };
            change_map(&mut sample, |map| {
                let index = map.map.region.index(target).ok_or_else(corrupt)?;
                let Cell::Visible(mut tile) = map.map.cells[index] else {
                    return Err(corrupt());
                };
                map.map.cells[index] = match variant {
                    0 => Cell::Hidden,
                    1 => Cell::Unallocated,
                    2 => {
                        tile.liquid_depth = 1;
                        Cell::Visible(tile)
                    }
                    3 => {
                        tile.dig_designation = 1;
                        Cell::Visible(tile)
                    }
                    4 => {
                        tile.shape = Shape::Floor;
                        Cell::Visible(tile)
                    }
                    5 => {
                        tile.unit_occupancy = 1;
                        Cell::Visible(tile)
                    }
                    _ => {
                        tile.building_occupancy = 0;
                        Cell::Visible(tile)
                    }
                };
                Ok(())
            })?;
        }
        let lost = acknowledge(&candidate, &definition, &sample, &c)?;
        assert_eq!(
            (lost.observations, lost.streak),
            (2, 0),
            "variant {variant}"
        );
        assert_eq!((lost.first_tick, lost.counted_tick), (None, None));
        assert!(!lost.terminal());
        assert_eq!(lost.terrain.unknown(), variant < 2);
        if variant < 7 {
            assert!(!lost.terrain.condition_met());
            assert!(lost.terrain.deficit_count() > 0);
        } else {
            assert!(lost.terrain.condition_met());
            assert_eq!(lost.condition_met_count(), 31);
        }
        let later = step(
            &lost,
            &definition,
            &evidence(&definition, fixtures::TICK + 20)?,
            &c,
        )?;
        assert_eq!(later.streak, 1);
        assert_eq!(later.phase, "candidate");
    }
    Ok(())
}

#[test]
fn same_tick_map_padding_gap_and_unpaused_readiness_cannot_preserve_a_streak() -> Result<()> {
    let (original, c) = fixture(2)?;
    let definition = retimed(
        &original,
        Timing {
            max_gap: 30,
            ..original.goal().timing()
        },
    )?;
    let candidate = step(
        &Progress::new(&definition),
        &definition,
        &evidence(&definition, fixtures::TICK)?,
        &c,
    )?;
    for variant in 0..4 {
        let tick = if variant == 1 {
            fixtures::TICK + 31
        } else {
            fixtures::TICK
        };
        let mut sample = evidence(&definition, tick)?;
        match variant {
            0 => change_map(&mut sample, |map| {
                let index = map.map.cells.len() - 1;
                let Cell::Visible(mut tile) = map.map.cells[index] else {
                    return Err(corrupt());
                };
                tile.traffic = 1;
                map.map.cells[index] = Cell::Visible(tile);
                Ok(())
            })?,
            1 => {}
            2 => change_map(&mut sample, |map| {
                map.paused = false;
                Ok(())
            })?,
            _ => {
                let mut operations = sample.operations()?;
                operations.jobs.paused = false;
                sample.furnishings = fixtures::with_observation(definition.goal(), operations);
            }
        }
        let reset = acknowledge(&candidate, &definition, &sample, &c)?;
        assert_eq!(reset.streak, 0, "variant {variant}");
        assert_eq!((reset.first_tick, reset.counted_tick), (None, None));
        assert!(reset.terrain.condition_met());
        assert!(!reset.terminal());
        if variant == 0 {
            let again = acknowledge(&reset, &definition, &sample, &c)?;
            assert_eq!(again.streak, 0);
            assert_eq!(step(&again, &definition, &sample, &c)?.streak, 0);
        }
    }
    Ok(())
}

#[test]
fn joint_source_stage_tick_and_deadline_failures_remain_terminal_for_readiness_and_probe()
-> Result<()> {
    let (definition, c) = fixture(32)?;
    let candidate = step(
        &Progress::new(&definition),
        &definition,
        &evidence(&definition, fixtures::TICK + 10)?,
        &c,
    )?;
    for variant in 0..6 {
        let tick = match variant {
            4 => fixtures::TICK + 5,
            5 => definition.goal().timing().deadline,
            _ => fixtures::TICK + 20,
        };
        let mut sample = evidence(&definition, tick)?;
        match variant {
            0 => {
                sample.before.generation += 1;
                sample.after.generation += 1;
            }
            1..=3 => {
                let mut operations = sample.operations()?;
                match variant {
                    1 => operations.jobs.bridge_generation += 1,
                    2 => {
                        operations
                            .buildings
                            .last_mut()
                            .ok_or_else(corrupt)?
                            .build_stage = 0
                    }
                    _ => {
                        let missing = operations.buildings.pop().ok_or_else(corrupt)?.native_id;
                        for item in &mut operations.items {
                            if item.holder_building_native_id == Some(missing) {
                                item.holder_building_native_id = None;
                            }
                        }
                    }
                }
                sample.furnishings = fixtures::with_observation(definition.goal(), operations);
            }
            _ => {}
        }
        let retained = acknowledge(&candidate, &definition, &sample, &c)?;
        let probed = readiness::probe(&candidate.begin_read()?, &definition, &sample, &c)?;
        assert_eq!(
            retained.phase,
            if variant == 5 {
                "expired"
            } else {
                "invalidated"
            }
        );
        assert_eq!(probed.phase, retained.phase);
        assert_eq!(probed.reason, retained.reason);
        assert_eq!((retained.observations, probed.observations), (2, 1));
        assert_eq!((retained.streak, probed.streak), (0, 0));
        assert!(retained.begin_read().is_err());
    }
    Ok(())
}

#[test]
fn full_readiness_replay_retains_first_sources_and_never_replays_as_a_goal_sample() -> Result<()> {
    let (definition, c) = fixture(2)?;
    let disk = Memory::default();
    let mut store = opened(&definition, &c, disk.clone())?;
    let initial = store.recovery();
    retain(&mut store, &c, &evidence(&definition, fixtures::TICK)?)?;
    assert_eq!(
        (store.progress().observations, store.progress().streak),
        (1, 0)
    );
    let first_source = store.first_source().ok_or_else(corrupt)?.clone();
    publish(&mut store, &c, &evidence(&definition, fixtures::TICK + 10)?)?;
    assert_eq!(store.progress().streak, 1);
    retain(&mut store, &c, &evidence(&definition, fixtures::TICK + 20)?)?;
    assert_eq!(
        (store.progress().phase, store.progress().observations),
        ("candidate", 3)
    );
    assert_eq!(store.progress().counted_tick, Some(fixtures::TICK + 10));
    assert!(!store.read_owned());
    assert!(!store.progress().reading);
    let acknowledged = store.recovery();
    drop(store);
    let mut reopened = MonitorStore::open(disk, &c, BuildMode::Recover, None, false)?;
    reopened.verify_recovery(&c, initial)?;
    reopened.verify_recovery(&c, acknowledged)?;
    assert_eq!(reopened.first_source(), Some(&first_source));
    assert_eq!(
        (reopened.progress().observations, reopened.progress().streak),
        (3, 0)
    );
    assert!(!reopened.progress().terminal());
    let bytes_before = reopened.byte_len();
    assert!(
        reopened
            .publish_readiness(
                &c,
                &evidence(&definition, fixtures::TICK + 30)?,
                &mut admit,
                &mut guard,
            )
            .is_err()
    );
    assert_eq!(reopened.byte_len(), bytes_before);
    let mut changed = evidence(&definition, fixtures::TICK + 30)?;
    changed.before.generation += 1;
    changed.after.generation += 1;
    retain(&mut reopened, &c, &changed)?;
    assert_eq!(reopened.progress().phase, "invalidated");
    assert_eq!(reopened.first_source(), Some(&first_source));
    assert_ne!(reopened.progress().source.as_ref(), Some(&first_source));
    Ok(())
}

#[test]
fn readiness_sync_loss_recovers_complete_descendant_but_never_restores_stability_or_read_rights()
-> Result<()> {
    let (definition, c) = fixture(2)?;
    let disk = Memory::default();
    let mut store = opened(&definition, &c, disk.clone())?;
    publish(&mut store, &c, &evidence(&definition, fixtures::TICK)?)?;
    let pin = store.recovery();
    let first_source = store.first_source().cloned();
    store.begin_read(&c, &mut admit, &mut guard)?;
    disk.disk.borrow_mut().fail_sync = true;
    assert!(
        store
            .publish_readiness(
                &c,
                &evidence(&definition, fixtures::TICK + 10)?,
                &mut admit,
                &mut guard,
            )
            .is_err()
    );
    assert!(store.is_fenced());
    assert!(!store.read_owned());
    assert_eq!(store.progress().streak, 0);
    disk.disk.borrow_mut().fail_sync = false;
    drop(store);
    let mut reopened = MonitorStore::open(disk.clone(), &c, BuildMode::Recover, None, false)?;
    reopened.verify_recovery(&c, pin)?;
    assert_eq!(reopened.first_source(), first_source.as_ref());
    assert_eq!(
        (reopened.progress().observations, reopened.progress().streak),
        (2, 0)
    );
    assert!(!reopened.progress().terminal());
    assert!(!reopened.read_owned());

    let bytes = disk.disk.borrow().bytes.clone();
    for cut in [bytes.len() - 1, bytes.len() - 31] {
        assert!(
            MonitorStore::open(
                Memory::from_bytes(bytes[..cut].to_vec()),
                &c,
                BuildMode::Recover,
                None,
                false,
            )
            .is_err()
        );
    }
    publish(
        &mut reopened,
        &c,
        &evidence(&definition, fixtures::TICK + 20)?,
    )?;
    assert_eq!(reopened.progress().streak, 1);
    publish(
        &mut reopened,
        &c,
        &evidence(&definition, fixtures::TICK + 30)?,
    )?;
    assert_eq!(reopened.progress().phase, "satisfied");
    Ok(())
}

#[test]
fn original_observation_exhaustion_is_durable_for_both_preparation_and_commit_readiness()
-> Result<()> {
    let (original, c) = fixture(2)?;
    for cap in [2, 3] {
        let definition = retimed(
            &original,
            Timing {
                max_observations: cap,
                ..original.goal().timing()
            },
        )?;
        let disk = Memory::default();
        let mut store = opened(&definition, &c, disk.clone())?;
        let initial = store.recovery();
        let sample = evidence(&definition, fixtures::TICK)?;
        publish(&mut store, &c, &sample)?;
        let first_source = store.first_source().cloned();
        for _ in 1..cap {
            retain(&mut store, &c, &sample)?;
        }
        assert_eq!(
            (store.progress().phase, store.progress().observations),
            ("expired", cap)
        );
        assert_eq!(store.progress().reason, "sample_budget_exhausted");
        let final_pin = store.recovery();
        drop(store);
        let mut offline = MonitorStore::open(disk, &c, BuildMode::Offline, None, false)?;
        offline.verify_recovery(&c, initial)?;
        offline.verify_recovery(&c, final_pin)?;
        assert_eq!(offline.first_source(), first_source.as_ref());
        assert_eq!(
            (offline.progress().phase, offline.progress().observations),
            ("expired", cap)
        );
        assert_eq!(
            (offline.progress().streak, offline.progress().first_tick),
            (0, None)
        );
        assert!(!offline.read_owned());
        assert!(
            offline
                .publish_readiness(&c, &sample, &mut admit, &mut guard)
                .is_err()
        );
        assert_eq!(offline.recovery(), final_pin);
    }
    Ok(())
}

#[test]
fn failed_readiness_retains_consumed_floor_and_authority_loss_withdraws_unfinished_credit()
-> Result<()> {
    let (definition, c) = fixture(2)?;
    let disk = Memory::default();
    let mut store = opened(&definition, &c, disk.clone())?;
    publish(&mut store, &c, &evidence(&definition, fixtures::TICK)?)?;
    store.begin_read(&c, &mut admit, &mut guard)?;
    let mut incomplete = evidence(&definition, fixtures::TICK + 80)?;
    incomplete.after_capture.clear();
    assert!(
        store
            .publish_readiness(&c, &incomplete, &mut admit, &mut guard)
            .is_err()
    );
    assert_eq!(store.authority_tick_floor(), fixtures::TICK + 80);
    assert_eq!(
        (store.progress().observations, store.progress().streak),
        (1, 0)
    );
    assert!(!store.read_owned());
    let mut expired = c.clone();
    expired.grants[0].expires_at_tick = Some(GameTick(fixtures::TICK + 70));
    assert!(store.verify(&expired).is_err());
    store.begin_read(&c, &mut admit, &mut guard)?;
    let pin = store.recovery();
    drop(store);
    let mut reopened = MonitorStore::open(disk, &c, BuildMode::Recover, None, false)?;
    reopened.verify_recovery(&c, pin)?;
    assert_eq!(reopened.authority_tick_floor(), fixtures::TICK + 80);
    retain(
        &mut reopened,
        &c,
        &evidence(&definition, fixtures::TICK + 10)?,
    )?;
    assert_eq!(reopened.progress().phase, "invalidated");
    assert_eq!(reopened.progress().reason, "room_game_clock_regressed");

    let mut live = opened(&definition, &c, Memory::default())?;
    publish(&mut live, &c, &evidence(&definition, fixtures::TICK)?)?;
    live.begin_read(&c, &mut admit, &mut guard)?;
    let pin = live.recovery();
    let mut denied = c.clone();
    denied.grants.clear();
    assert!(
        live.publish_readiness(
            &denied,
            &evidence(&definition, fixtures::TICK + 10)?,
            &mut admit,
            &mut guard,
        )
        .is_err()
    );
    assert_eq!(live.recovery(), pin);
    assert_eq!(live.progress().streak, 0);
    assert!(!live.read_owned());
    publish(&mut live, &c, &evidence(&definition, fixtures::TICK + 10)?)?;
    assert_eq!(live.progress().streak, 1);
    assert_ne!(live.progress().phase, "satisfied");
    Ok(())
}
