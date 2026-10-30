use super::*;
use crate::construction_plan::{Timing, fixtures};
use dfmcp_core::ErrorCode;

fn goal(count: usize, max_observations: u32) -> Result<Goal> {
    Goal::new(
        fixtures::goal(count).records().to_vec(),
        Timing {
            deadline: fixtures::TICK + 1000,
            interval: 10,
            stable_samples: 2,
            stable_span: 10,
            max_gap: 100,
            max_observations,
        },
    )
}
fn ordinary(state: &Progress, goal: &Goal, tick: u64) -> Result<Progress> {
    state.begin_read()?.advance(
        goal,
        &fixtures::sample(goal, tick),
        &fixtures::context(goal),
    )
}
fn acknowledge(state: &Progress, goal: &Goal, tick: u64) -> Result<Progress> {
    advance(
        &state.begin_read()?,
        goal,
        &fixtures::sample(goal, tick),
        &fixtures::context(goal),
    )
}

#[test]
fn readiness_preserves_one_shared_streak_without_earning_any_positive_sample() -> Result<()> {
    for count in [1, 2, 32] {
        let goal = goal(count, 20)?;
        let initial = Progress::new(&goal);
        let first = acknowledge(&initial, &goal, fixtures::TICK)?;
        assert_eq!((first.observations, first.streak), (1, 0));
        assert_eq!((first.first_tick, first.counted_tick), (None, None));
        assert_eq!(first.condition_met_count(), count);
        assert!(!first.reading && !first.terminal());
        // An interval advances before the first ordinary goal sample.
        let candidate = ordinary(&first, &goal, fixtures::TICK + 10)?;
        assert_eq!((candidate.observations, candidate.streak), (2, 1));
        let prepared = acknowledge(&candidate, &goal, fixtures::TICK + 10)?;
        let dispatched = acknowledge(&prepared, &goal, fixtures::TICK + 10)?;
        assert_eq!((dispatched.observations, dispatched.streak), (4, 1));
        assert_eq!(dispatched.phase, "candidate");
        assert_eq!(dispatched.first_tick, candidate.first_tick);
        assert_eq!(dispatched.counted_tick, candidate.counted_tick);
        // Only the separate ordinary sample after another interval completes it.
        let completed = ordinary(&dispatched, &goal, fixtures::TICK + 20)?;
        assert_eq!(
            (completed.phase, completed.observations, completed.streak),
            ("satisfied", 5, 2)
        );
        assert_eq!(completed.condition_met_count(), count);
    }
    Ok(())
}

#[test]
fn probe_rechecks_pre_and_post_acknowledgement_without_double_charging_last_allowance() -> Result<()>
{
    let goal = goal(2, 4)?;
    let first = ordinary(&Progress::new(&goal), &goal, fixtures::TICK)?;
    let before = acknowledge(&first, &goal, fixtures::TICK)?;
    let sample = fixtures::sample(&goal, fixtures::TICK + 10);
    let after = advance(
        &before.begin_read()?,
        &goal,
        &sample,
        &fixtures::context(&goal),
    )?;
    assert_eq!((before.observations, after.observations), (2, 3));
    for retained in [&before, &after] {
        let checked = probe(
            &retained.begin_read()?,
            &goal,
            &sample,
            &fixtures::context(&goal),
        )?;
        assert_eq!(checked.observations, retained.observations);
        assert_eq!(checked.streak, 1);
        assert_eq!(checked.first_tick, first.first_tick);
        assert_eq!(checked.counted_tick, first.counted_tick);
        assert_eq!(checked.phase, "candidate");
    }
    // A real fourth acquisition expires even though normal goal evaluation at
    // this advancing tick could otherwise return satisfied before the cap check.
    let exhausted = acknowledge(&after, &goal, fixtures::TICK + 20)?;
    assert_eq!(
        (exhausted.phase, exhausted.reason, exhausted.observations),
        ("expired", "sample_budget_exhausted", 4)
    );
    assert_eq!(exhausted.streak, 0);
    assert!(exhausted.begin_read().is_err());
    Ok(())
}

#[test]
fn readiness_that_would_complete_a_goal_still_expires_at_original_acquisition_limit() -> Result<()>
{
    let goal = goal(2, 2)?;
    let candidate = ordinary(&Progress::new(&goal), &goal, fixtures::TICK)?;
    let ordinary_completion = ordinary(&candidate, &goal, fixtures::TICK + 10)?;
    assert_eq!(ordinary_completion.phase, "satisfied");
    let readiness = acknowledge(&candidate, &goal, fixtures::TICK + 10)?;
    assert_eq!(
        (readiness.phase, readiness.reason),
        ("expired", "sample_budget_exhausted")
    );
    assert_eq!((readiness.observations, readiness.streak), (2, 0));
    assert_eq!((readiness.first_tick, readiness.counted_tick), (None, None));
    Ok(())
}

#[test]
fn same_tick_changes_gaps_unpaused_evidence_and_unverified_members_break_continuity() -> Result<()>
{
    let goal = goal(2, 20)?;
    let candidate = ordinary(&Progress::new(&goal), &goal, fixtures::TICK)?;
    for variant in 0..4 {
        let tick = if variant == 1 {
            fixtures::TICK + 101
        } else {
            fixtures::TICK
        };
        let mut observed = fixtures::observation(&goal, tick);
        match variant {
            0 => observed.next_item_id += 1,
            1 => {}
            2 => observed.jobs.paused = false,
            _ => observed.items[1].flags |= 1 << 1,
        }
        let evidence = fixtures::with_observation(&goal, observed);
        let reset = advance(
            &candidate.begin_read()?,
            &goal,
            &evidence,
            &fixtures::context(&goal),
        )?;
        assert_eq!(reset.streak, 0, "variant {variant}");
        assert_eq!((reset.first_tick, reset.counted_tick), (None, None));
        assert!(!reset.terminal());
        assert_eq!(reset.observations, 2);
        if variant == 0 {
            let repeated = advance(
                &reset.begin_read()?,
                &goal,
                &evidence,
                &fixtures::context(&goal),
            )?;
            assert_eq!(repeated.streak, 0);
            let ordinary_same_tick =
                repeated
                    .begin_read()?
                    .advance(&goal, &evidence, &fixtures::context(&goal))?;
            assert_eq!(ordinary_same_tick.streak, 0);
        }
    }
    Ok(())
}

#[test]
fn absorbing_negative_evidence_remains_terminal_in_acknowledgements_and_probes() -> Result<()> {
    let goal = goal(2, 20)?;
    let candidate = ordinary(&Progress::new(&goal), &goal, fixtures::TICK)?;
    for variant in 0..4 {
        let tick = if variant == 3 {
            goal.timing().deadline
        } else {
            fixtures::TICK + 10
        };
        let mut observed = fixtures::observation(&goal, tick);
        let expected = match variant {
            0 => {
                observed.jobs.bridge_generation += 1;
                "invalidated"
            }
            1 => {
                observed.buildings[1].build_stage -= 1;
                "invalidated"
            }
            2 => {
                let missing = observed
                    .buildings
                    .pop()
                    .ok_or_else(|| {
                        dfmcp_core::DfmcpError::new(
                            ErrorCode::InternalInvariantViolation,
                            "missing test target",
                        )
                    })?
                    .native_id;
                for item in &mut observed.items {
                    if item.holder_building_native_id == Some(missing) {
                        item.holder_building_native_id = None;
                    }
                }
                "invalidated"
            }
            _ => "expired",
        };
        let evidence = fixtures::with_observation(&goal, observed);
        let terminal = advance(
            &candidate.begin_read()?,
            &goal,
            &evidence,
            &fixtures::context(&goal),
        )?;
        assert_eq!(terminal.phase, expected, "variant {variant}");
        assert_eq!(terminal.observations, 2);
        assert_eq!(terminal.streak, 0);
        let projected = probe(
            &candidate.begin_read()?,
            &goal,
            &evidence,
            &fixtures::context(&goal),
        )?;
        assert_eq!(projected.phase, expected);
        assert_eq!(projected.observations, 1);
    }
    Ok(())
}

#[test]
fn interruption_and_current_authority_cannot_be_hidden_by_readiness() -> Result<()> {
    let goal = goal(2, 20)?;
    let candidate = ordinary(&Progress::new(&goal), &goal, fixtures::TICK)?;
    let interrupted = candidate.begin_read()?.begin_read()?;
    let evidence = fixtures::sample(&goal, fixtures::TICK + 10);
    let resumed = advance(&interrupted, &goal, &evidence, &fixtures::context(&goal))?;
    assert_eq!((resumed.streak, resumed.interruptions), (0, 1));
    let mut denied = fixtures::context(&goal);
    denied.grants.clear();
    assert!(advance(&candidate.begin_read()?, &goal, &evidence, &denied).is_err());
    let mut expired = fixtures::context(&goal);
    for grant in &mut expired.grants {
        grant.expires_at_tick = Some(dfmcp_core::GameTick(fixtures::TICK + 5));
    }
    assert!(probe(&candidate.begin_read()?, &goal, &evidence, &expired).is_err());
    assert_eq!((candidate.observations, candidate.streak), (1, 1));
    Ok(())
}
