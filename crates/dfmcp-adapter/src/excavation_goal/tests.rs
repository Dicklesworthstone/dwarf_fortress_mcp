use super::*;
use dfmcp_world::map_region::{MapRegion, Tile};

fn capture(tick: u32) -> LiveMapObservation {
    let tile = Tile {
        native_tiletype: 3,
        shape: Shape::Floor,
        liquid_depth: 0,
        magma: false,
        traffic: 0,
        dig_designation: 0,
        building_occupancy: 0,
        unit_occupancy: 0,
        walkable_region: 1,
        temperature_1: 10015,
        temperature_2: 10015,
    };
    LiveMapObservation {
        bridge_generation: 7,
        df_version: "df".into(),
        dfhack_version: "dfhack".into(),
        year: 0,
        year_tick: tick,
        paused: true,
        site_id: 1,
        world_folder: "region1".into(),
        map_dimensions: [32, 32, 3],
        map: MapRegion {
            region: Region {
                origin: [15, 15, 2],
                size: [2, 2, 1],
            },
            cells: vec![Cell::Visible(tile); 4],
        },
    }
}
fn goal() -> Result<FloorGoal> {
    FloorGoal::new(capture(0).map.region, "region1".into(), 1, 100, 10, 2, 20)
}

#[test]
fn matching_requires_advancing_ticks_and_elapsed_span() -> Result<()> {
    let p = goal()?.begin(capture(10))?;
    assert_eq!(p.status(), FloorStatus::Stabilizing);
    let same = p.sample(capture(10))?;
    assert_eq!(same.streak(), 1);
    assert_eq!(same.status(), FloorStatus::Stabilizing);
    assert_eq!(same.sample(capture(19))?.status(), FloorStatus::Stabilizing);
    assert_eq!(same.sample(capture(20))?.status(), FloorStatus::Satisfied);
    Ok(())
}

#[test]
fn all_shape_liquid_designation_combinations_match_exact_predicate() -> Result<()> {
    let g = goal()?;
    for shape in 0..=8 {
        for liquid in 0..=7 {
            for dig in 0..=7 {
                let mut c = capture(1);
                if let Cell::Visible(tile) = &mut c.map.cells[0] {
                    tile.shape = Shape::from_tag(shape).map_err(map_error)?;
                    tile.liquid_depth = liquid;
                    tile.dig_designation = dig;
                }
                assert_eq!(
                    g.classify(&c)?.floor_goal == 4,
                    shape == 3 && liquid == 0 && dig == 0
                );
            }
        }
    }
    Ok(())
}

#[test]
fn missing_hidden_and_contradictions_reset_streak() -> Result<()> {
    let p = goal()?.begin(capture(1))?;
    for (cell, status) in [
        (Cell::Hidden, FloorStatus::Unknown),
        (Cell::Unallocated, FloorStatus::Unknown),
    ] {
        let mut c = capture(11);
        c.map.cells[0] = cell;
        let next = p.sample(c)?;
        assert_eq!(next.status(), status);
        assert_eq!(next.streak(), 0);
        assert_eq!(next.sample(capture(12))?.streak(), 1);
    }
    let mut c = capture(11);
    if let Cell::Visible(tile) = &mut c.map.cells[0] {
        tile.shape = Shape::Wall;
    }
    assert_eq!(p.sample(c)?.status(), FloorStatus::Pending);
    Ok(())
}

#[test]
fn failed_and_unfinished_reads_cannot_bridge_stability() -> Result<()> {
    let p = goal()?.begin(capture(1))?;
    for failed in [false, true] {
        let interrupted = p.interrupt(failed)?;
        assert_eq!(interrupted.status(), FloorStatus::Unknown);
        assert_eq!(
            interrupted.sample(capture(11))?.status(),
            FloorStatus::Stabilizing
        );
        assert_eq!(interrupted.sample(capture(11))?.streak(), 1);
    }
    Ok(())
}

#[test]
fn large_gaps_reset_and_deadline_is_inclusive_and_fixed() -> Result<()> {
    let p = goal()?.begin(capture(1))?;
    let gap = p.sample(capture(22))?;
    assert_eq!(gap.status(), FloorStatus::Stabilizing);
    assert_eq!(gap.since_tick(), Some(22));
    assert_eq!(gap.interruption(), Some(FloorInterruption::SampleGap));
    let p = goal()?.begin(capture(90))?;
    assert_eq!(p.sample(capture(100))?.status(), FloorStatus::Satisfied);
    assert_eq!(p.sample(capture(101))?.status(), FloorStatus::Expired);
    Ok(())
}

#[test]
fn source_drift_invalidates_without_replacing_last_accepted_sample() -> Result<()> {
    let p = goal()?.begin(capture(10))?;
    for changed in 0..7 {
        let mut c = capture(20);
        match changed {
            0 => c.bridge_generation += 1,
            1 => c.df_version.push('x'),
            2 => c.dfhack_version.push('x'),
            3 => c.world_folder.push('x'),
            4 => c.site_id += 1,
            5 => c.map_dimensions[0] += 1,
            _ => c.year_tick = 9,
        }
        let next = p.sample(c)?;
        assert_eq!(next.status(), FloorStatus::Invalidated);
        assert_eq!(next.latest(), p.latest());
        assert_eq!(next.observations(), 1);
    }
    Ok(())
}

#[test]
fn selection_substitution_and_invalid_native_fields_are_errors() -> Result<()> {
    let p = goal()?.begin(capture(10))?;
    let mut c = capture(20);
    c.map.region.origin[0] += 1;
    assert!(p.sample(c).is_err());
    let mut c = capture(20);
    if let Cell::Visible(t) = &mut c.map.cells[0] {
        t.liquid_depth = 8;
    }
    assert!(p.sample(c).is_err());
    assert_eq!(p.status(), FloorStatus::Stabilizing);
    Ok(())
}

#[test]
fn terminal_history_cannot_be_extended_interrupted_or_cancelled() -> Result<()> {
    let p = goal()?.begin(capture(90))?;
    for terminal in [
        p.sample(capture(100))?,
        p.sample(capture(101))?,
        p.cancel()?,
    ] {
        assert!(terminal.status().terminal());
        assert!(terminal.sample(capture(102)).is_err());
        assert!(terminal.interrupt(false).is_err());
        assert!(terminal.cancel().is_err());
    }
    Ok(())
}

#[test]
fn goal_bounds_and_initial_source_are_checked() -> Result<()> {
    let c = capture(0);
    assert!(FloorGoal::new(c.map.region, "".into(), 1, 100, 10, 2, 20).is_err());
    assert!(
        FloorGoal::new(
            c.map.region,
            "region1".into(),
            1,
            MAX_GAME_TICK + 1,
            10,
            2,
            20
        )
        .is_err()
    );
    assert!(FloorGoal::new(c.map.region, "region1".into(), 1, 100, 10, 0, 20).is_err());
    assert!(FloorGoal::new(c.map.region, "region1".into(), 1, 100, 10, 2, 0).is_err());
    assert!(goal()?.begin(capture(101)).is_err());
    let mut wrong = capture(10);
    wrong.site_id = 2;
    assert!(goal()?.begin(wrong).is_err());
    Ok(())
}

#[test]
fn single_sample_goal_is_explicit_and_does_not_check_unclaimed_safety() -> Result<()> {
    let mut c = capture(0);
    if let Cell::Visible(t) = &mut c.map.cells[0] {
        t.building_occupancy = 7;
        t.unit_occupancy = 3;
        t.walkable_region = 0;
        t.temperature_1 = u16::MAX;
    }
    let g = FloorGoal::new(c.map.region, "region1".into(), 1, 100, 0, 1, 20)?;
    assert_eq!(g.begin(c)?.status(), FloorStatus::Satisfied);
    Ok(())
}
