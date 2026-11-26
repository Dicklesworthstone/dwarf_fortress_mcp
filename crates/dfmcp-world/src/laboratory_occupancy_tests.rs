use super::*;

fn naive(area: MapCuboid, footprints: &BTreeMap<EntityId, MapCuboid>) -> Vec<bool> {
    crate::terrain::region_tiles(area)
        .map(|at| footprints.values().any(|footprint| footprint.contains(at)))
        .collect()
}

fn error_code<T>(result: Result<T>) -> Result<ErrorCode> {
    match result {
        Err(error) => Ok(error.code),
        Ok(_) => Err(invalid("occupancy test unexpectedly succeeded")),
    }
}

fn cube() -> MapCuboid {
    MapCuboid {
        min: MapCoord::new(0, 0, 0),
        max: MapCoord::new(1, 1, 1),
    }
}

#[test]
fn every_small_point_subset_matches_naive_occupancy() -> Result<()> {
    let area = cube();
    let points: Vec<_> = crate::terrain::region_tiles(area).collect();
    for subset in 0u16..256 {
        let footprints: BTreeMap<_, _> = points
            .iter()
            .enumerate()
            .filter(|(index, _)| subset & (1 << index) != 0)
            .map(|(index, at)| {
                (
                    EntityId::new(index as u64 + 1),
                    MapCuboid { min: *at, max: *at },
                )
            })
            .collect();
        assert_eq!(
            laboratory_building_occupancy_mask(area, &footprints)?,
            naive(area, &footprints),
            "point subset {subset}",
        );
    }
    Ok(())
}

#[test]
fn every_small_cuboid_pair_matches_naive_occupancy() -> Result<()> {
    let area = cube();
    let points: Vec<_> = crate::terrain::region_tiles(area).collect();
    let cuboids: Vec<_> = points
        .iter()
        .flat_map(|min| {
            points
                .iter()
                .filter_map(move |max| MapCuboid::new(*min, *max).ok())
        })
        .collect();
    assert_eq!(cuboids.len(), 27);
    for first in &cuboids {
        for second in &cuboids {
            let footprints =
                BTreeMap::from([(EntityId::new(1), *first), (EntityId::new(2), *second)]);
            assert_eq!(
                laboratory_building_occupancy_mask(area, &footprints)?,
                naive(area, &footprints),
                "cuboid pair {first:?}, {second:?}",
            );
        }
    }
    Ok(())
}

#[test]
fn footprints_clip_to_requested_levels_and_boundaries() -> Result<()> {
    let area = MapCuboid::new(MapCoord::new(-2, 3, 7), MapCoord::new(0, 4, 8))?;
    let footprints = BTreeMap::from([
        (
            EntityId::new(1),
            MapCuboid::new(MapCoord::new(-4, 3, 6), MapCoord::new(-1, 3, 8))?,
        ),
        (
            EntityId::new(2),
            MapCuboid::new(MapCoord::new(-1, 4, 8), MapCoord::new(4, 8, 9))?,
        ),
        (
            EntityId::new(3),
            MapCuboid::new(MapCoord::new(-2, 3, 9), MapCoord::new(0, 4, 10))?,
        ),
        (
            EntityId::new(4),
            MapCuboid::new(MapCoord::new(1, 3, 7), MapCoord::new(2, 4, 8))?,
        ),
    ]);
    let mask = laboratory_building_occupancy_mask(area, &footprints)?;
    assert_eq!(mask, naive(area, &footprints));
    assert_eq!(mask.iter().filter(|occupied| **occupied).count(), 6);
    Ok(())
}

#[test]
fn tall_thin_region_uses_the_full_bounded_volume() -> Result<()> {
    let area = MapCuboid::new(
        MapCoord::new(i32::MAX, i32::MIN, -32_768),
        MapCoord::new(i32::MAX, i32::MIN, 32_767),
    )?;
    let footprints = BTreeMap::from([(
        EntityId::new(1),
        MapCuboid::new(
            MapCoord::new(i32::MIN, i32::MIN, -1),
            MapCoord::new(i32::MAX, i32::MAX, 1),
        )?,
    )]);
    let mask = laboratory_building_occupancy_mask(area, &footprints)?;
    assert_eq!(mask.len(), 65_536);
    assert_eq!(mask, naive(area, &footprints));
    assert_eq!(mask.iter().filter(|occupied| **occupied).count(), 3);
    Ok(())
}

#[test]
fn extreme_coordinates_clip_without_counting_unbounded_footprint_volume() -> Result<()> {
    let area = MapCuboid::new(
        MapCoord::new(i32::MAX - 1, i32::MIN, i32::MAX - 1),
        MapCoord::new(i32::MAX, i32::MIN + 1, i32::MAX),
    )?;
    let footprint = MapCuboid::new(
        MapCoord::new(i32::MIN, i32::MIN, i32::MIN),
        MapCoord::new(i32::MAX, i32::MAX, i32::MAX),
    )?;
    assert_eq!(footprint.tile_count(), None);
    let footprints = BTreeMap::from([(EntityId::new(1), footprint)]);
    assert_eq!(
        laboratory_building_occupancy_mask(area, &footprints)?,
        vec![true; 8],
    );
    Ok(())
}

#[test]
fn invalid_shapes_and_oversized_domains_are_refused() -> Result<()> {
    let empty = BTreeMap::new();
    for max in [
        MapCoord::new(-1, 0, 0),
        MapCoord::new(0, -1, 0),
        MapCoord::new(0, 0, -1),
    ] {
        let malformed = MapCuboid {
            min: MapCoord::new(0, 0, 0),
            max,
        };
        assert_eq!(
            error_code(laboratory_building_occupancy_mask(malformed, &empty))?,
            ErrorCode::InvalidRequest,
        );
        let footprints = BTreeMap::from([(EntityId::new(1), malformed)]);
        assert_eq!(
            error_code(laboratory_building_occupancy_mask(cube(), &footprints))?,
            ErrorCode::InvalidRequest,
        );
    }
    for max in [
        MapCoord::new(65_536, 0, 0),
        MapCoord::new(i32::MAX, i32::MAX, i32::MAX),
    ] {
        assert_eq!(
            error_code(laboratory_building_occupancy_mask(
                MapCuboid {
                    min: MapCoord::new(0, 0, 0),
                    max
                },
                &empty,
            ))?,
            ErrorCode::BudgetExceeded,
        );
    }
    let mut footprints: BTreeMap<_, _> =
        (1..=65_537).map(|id| (EntityId::new(id), cube())).collect();
    assert_eq!(
        error_code(laboratory_building_occupancy_mask(cube(), &footprints))?,
        ErrorCode::BudgetExceeded,
    );
    footprints.remove(&EntityId::new(65_537));
    assert_eq!(
        laboratory_building_occupancy_mask(cube(), &footprints)?,
        vec![true; 8]
    );
    Ok(())
}
