use super::*;

use dfmcp_core::{Digest32, FortressId, ObservationCursor};

use crate::{
    EvidenceCoverage, EvidencePolicy, EvidenceSource, Fact, FactPresence, FactSource, Predicate,
    PredicateEvidence, QueryOrder, WorldGraph, WorldQuery,
};

fn region(width: i32, height: i32, depth: i32) -> MapCuboid {
    MapCuboid {
        min: MapCoord::new(0, 0, 0),
        max: MapCoord::new(width - 1, height - 1, depth - 1),
    }
}

fn snapshot(area: MapCuboid, terrain: u32) -> WorldSnapshot {
    let chunks: BTreeSet<_> = crate::terrain::region_tiles(area)
        .map(|at| crate::terrain::tile_slot(at).chunk)
        .collect();
    let mut graph = WorldGraph::default();
    for coord in chunks {
        graph
            .chunks
            .insert(coord, crate::terrain::uniform_chunk(coord, terrain));
    }
    WorldSnapshot::new(
        FortressId::new(1),
        GameTick(10),
        ObservationCursor::ORIGIN,
        true,
        graph,
    )
}

fn fact(value: Value) -> Fact {
    Fact::known(
        value,
        GameTick(10),
        FactSource::Derived("dfmcp.reference-effects/1".to_owned()),
        Digest32::ZERO,
    )
}

fn building(id: u64, footprint: MapCuboid) -> EntityRecord {
    EntityRecord {
        id: EntityId::new(id),
        generation: 1,
        revision: 1,
        kind: EntityKind::Building,
        label: "paired-stair obstacle".to_owned(),
        fields: BTreeMap::from([
            (
                "construction_stage".to_owned(),
                fact(Value::Text("complete".to_owned())),
            ),
            (
                "footprint_min".to_owned(),
                fact(Value::Coord(footprint.min)),
            ),
            (
                "footprint_max".to_owned(),
                fact(Value::Coord(footprint.max)),
            ),
        ]),
    }
}

fn proof(area: MapCuboid, entry: MapCoord, targets: Vec<MapCoord>) -> Predicate {
    Predicate::LaboratoryRoomAccessPairedStairs {
        area,
        entry,
        targets,
    }
}

fn truth(world: &WorldSnapshot, area: MapCuboid) -> Result<PredicateTruth> {
    laboratory_room_access_paired_stairs_truth(world, area, area.min, &[area.max])
}

/// Deliberately use a coordinate-pair transitive closure rather than the
/// implementation's index arithmetic, direction generators or breadth-first walk.
fn oracle(cells: &[(MapCoord, u32)], entry: MapCoord, target: MapCoord) -> PredicateTruth {
    let reached = |potential: bool| {
        let traversable = |at: MapCoord, code: u32| {
            at != target
                && (matches!(code, tile_codes::FLOOR | tile_codes::STAIR)
                    || (potential && code == tile_codes::UNKNOWN))
        };
        let stairs = |code| code == tile_codes::STAIR || (potential && code == tile_codes::UNKNOWN);
        let mut seen = BTreeSet::new();
        for (at, code) in cells {
            if *at == entry && traversable(*at, *code) {
                seen.insert(*at);
            }
        }
        loop {
            let before = seen.clone();
            for (to, code) in cells {
                if !traversable(*to, *code) {
                    continue;
                }
                for (from, from_code) in cells {
                    if !before.contains(from) {
                        continue;
                    }
                    let horizontal = to.z == from.z
                        && (i64::from(to.x) - i64::from(from.x)).abs()
                            + (i64::from(to.y) - i64::from(from.y)).abs()
                            == 1;
                    let vertical = to.x == from.x
                        && to.y == from.y
                        && (i64::from(to.z) - i64::from(from.z)).abs() == 1
                        && stairs(*code)
                        && stairs(*from_code);
                    if horizontal || vertical {
                        seen.insert(*to);
                    }
                }
            }
            if seen == before {
                return seen;
            }
        }
    };
    let approach = |seen: BTreeSet<MapCoord>| {
        seen.iter().any(|at| {
            at.z == target.z
                && (i64::from(at.x) - i64::from(target.x)).abs()
                    + (i64::from(at.y) - i64::from(target.y)).abs()
                    == 1
        })
    };
    if approach(reached(false)) {
        PredicateTruth::True
    } else if approach(reached(true)) {
        PredicateTruth::Unknown
    } else {
        PredicateTruth::False
    }
}

#[test]
fn small_three_dimensional_terrain_matches_an_independent_closure_oracle() -> Result<()> {
    let codes = [
        tile_codes::SOLID_WALL,
        tile_codes::FLOOR,
        tile_codes::STAIR,
        tile_codes::UNKNOWN,
    ];
    for (area, alternatives, cell_count) in [(region(3, 1, 2), 4u32, 6), (region(2, 2, 2), 3, 8)] {
        for encoding in 0..alternatives.pow(cell_count) {
            let mut value = encoding;
            let mut cells = Vec::new();
            let mut world = snapshot(area, tile_codes::SOLID_WALL);
            for at in crate::terrain::region_tiles(area) {
                let code = codes[(value % alternatives) as usize];
                value /= alternatives;
                world.set_tile_code(at, code)?;
                cells.push((at, code));
            }
            assert_eq!(
                truth(&world, area)?,
                oracle(&cells, area.min, area.max),
                "area {area:?}, terrain encoding {encoding}"
            );
        }
    }
    Ok(())
}

#[test]
fn vertical_movement_requires_contiguous_pairs_and_is_bidirectional() -> Result<()> {
    let area = region(4, 1, 3);
    let mut world = snapshot(area, tile_codes::FLOOR);
    assert_eq!(truth(&world, area)?, PredicateTruth::False);
    for z in [0, 2] {
        world.set_tile_code(MapCoord::new(1, 0, z), tile_codes::STAIR)?;
    }
    assert_eq!(truth(&world, area)?, PredicateTruth::False);
    world.set_tile_code(MapCoord::new(1, 0, 1), tile_codes::STAIR)?;
    assert_eq!(truth(&world, area)?, PredicateTruth::True);
    assert_eq!(
        laboratory_room_access_paired_stairs_truth(
            &world,
            area,
            MapCoord::new(0, 0, 2),
            &[MapCoord::new(3, 0, 0)]
        )?,
        PredicateTruth::True
    );
    // Furnishings require horizontal approach, even directly above a stair.
    let shaft = region(1, 1, 2);
    assert_eq!(
        truth(&snapshot(shaft, tile_codes::STAIR), shaft)?,
        PredicateTruth::False
    );
    // A target blocks travel through its cell even without a building record.
    let line = region(5, 1, 1);
    assert_eq!(
        laboratory_room_access_paired_stairs_truth(
            &snapshot(line, tile_codes::STAIR),
            line,
            line.min,
            &[MapCoord::new(2, 0, 0), line.max]
        )?,
        PredicateTruth::False
    );
    Ok(())
}

#[test]
fn unknown_stair_halves_are_potential_but_never_override_a_known_floor() -> Result<()> {
    let area = region(3, 1, 2);
    let mut world = snapshot(area, tile_codes::FLOOR);
    let lower = MapCoord::new(1, 0, 0);
    let upper = MapCoord::new(1, 0, 1);
    world.set_tile_code(lower, tile_codes::STAIR)?;
    world.set_tile_code(upper, tile_codes::UNKNOWN)?;
    assert_eq!(truth(&world, area)?, PredicateTruth::Unknown);
    world.set_tile_code(lower, tile_codes::FLOOR)?;
    assert_eq!(truth(&world, area)?, PredicateTruth::False);
    world.set_tile_code(lower, tile_codes::UNKNOWN)?;
    assert_eq!(truth(&world, area)?, PredicateTruth::Unknown);
    // Install newly observed source terrain. Ordinary effects correctly refuse
    // to overwrite a cell whose prior canonical value is UNKNOWN.
    let upper_chunk = crate::terrain::tile_slot(upper).chunk;
    world.graph.chunks.insert(
        upper_chunk,
        crate::terrain::uniform_chunk(upper_chunk, tile_codes::FLOOR),
    );
    world.set_tile_code(upper, tile_codes::STAIR)?;
    assert_eq!(truth(&world, area)?, PredicateTruth::Unknown);
    let lower_chunk = crate::terrain::tile_slot(lower).chunk;
    world.graph.chunks.insert(
        lower_chunk,
        crate::terrain::uniform_chunk(lower_chunk, tile_codes::FLOOR),
    );
    world.set_tile_code(lower, tile_codes::STAIR)?;
    assert_eq!(truth(&world, area)?, PredicateTruth::True);
    world.graph.chunks.remove(&upper_chunk);
    assert_eq!(truth(&world, area)?, PredicateTruth::Unknown);
    let mut malformed = crate::terrain::uniform_chunk(upper_chunk, tile_codes::STAIR);
    malformed.terrain_runs.push(crate::TerrainRun {
        tile_code: tile_codes::FLOOR,
        length: 0,
    });
    world.graph.chunks.insert(upper_chunk, malformed);
    assert_eq!(truth(&world, area)?, PredicateTruth::Unknown);
    Ok(())
}

#[test]
fn three_dimensional_occupancy_clipping_matches_direct_cuboid_union() -> Result<()> {
    let area = MapCuboid {
        min: MapCoord::new(-2, -2, -1),
        max: MapCoord::new(2, 2, 1),
    };
    let grid = PairedStairsGrid::new(area)?;
    let footprints = [
        MapCuboid {
            min: MapCoord::new(-5, -1, -2),
            max: MapCoord::new(0, 1, 0),
        },
        MapCuboid {
            min: MapCoord::new(-1, -5, 0),
            max: MapCoord::new(1, 5, 0),
        },
        MapCuboid {
            min: MapCoord::new(2, 2, 1),
            max: MapCoord::new(4, 4, 3),
        },
        MapCuboid {
            min: MapCoord::new(-2, -2, 2),
            max: MapCoord::new(2, 2, 3),
        },
        MapCuboid {
            min: MapCoord::new(3, -2, -2),
            max: MapCoord::new(4, 2, 2),
        },
        MapCuboid {
            min: MapCoord::new(i32::MIN, i32::MIN, i32::MIN),
            max: MapCoord::new(i32::MAX, i32::MAX, i32::MAX),
        },
    ];
    for mask in 0..64u32 {
        let selected: BTreeMap<_, _> = footprints
            .iter()
            .enumerate()
            .filter(|(index, _)| mask & (1 << index) != 0)
            .map(|(index, footprint)| (EntityId::new(index as u64 + 1), *footprint))
            .collect();
        let expected: Vec<_> = crate::terrain::region_tiles(area)
            .map(|at| selected.values().any(|footprint| footprint.contains(at)))
            .collect();
        assert_eq!(grid.occupied(&selected), expected, "footprint mask {mask}");
    }
    Ok(())
}

#[test]
fn occupancy_blocks_each_actual_level_and_unqualified_census_remains_unknown() -> Result<()> {
    let area = region(4, 1, 2);
    let mut world = snapshot(area, tile_codes::FLOOR);
    for z in 0..=1 {
        world.set_tile_code(MapCoord::new(1, 0, z), tile_codes::STAIR)?;
    }
    let landing = MapCoord::new(1, 0, 1);
    let obstruction = building(
        1,
        MapCuboid {
            min: landing,
            max: landing,
        },
    );
    world
        .graph
        .entities
        .insert(obstruction.id, obstruction.clone());
    assert_eq!(truth(&world, area)?, PredicateTruth::False);
    let other_level = MapCoord::new(1, 0, 2);
    let outside = building(
        1,
        MapCuboid {
            min: other_level,
            max: other_level,
        },
    );
    world.graph.entities.insert(outside.id, outside);
    assert_eq!(truth(&world, area)?, PredicateTruth::True);
    for mutation in 0..5 {
        let mut bad = obstruction.clone();
        match mutation {
            0 => {
                bad.fields.remove("footprint_max");
            }
            1 => {
                bad.fields
                    .get_mut("footprint_min")
                    .ok_or_else(|| invalid("fixture field missing"))?
                    .source = FactSource::Replay;
            }
            2 => {
                bad.fields
                    .get_mut("footprint_min")
                    .ok_or_else(|| invalid("fixture field missing"))?
                    .observed_at = GameTick(11);
            }
            3 => {
                bad.fields
                    .get_mut("footprint_min")
                    .ok_or_else(|| invalid("fixture field missing"))?
                    .presence = Some(FactPresence::Unknown("unobserved".to_owned()));
            }
            _ => bad.id = EntityId::new(2),
        }
        world.graph.entities.insert(EntityId::new(1), bad);
        assert_eq!(
            truth(&world, area)?,
            PredicateTruth::Unknown,
            "mutation {mutation}"
        );
    }
    Ok(())
}

#[test]
fn paired_stair_evidence_remains_laboratory_only() -> Result<()> {
    let area = region(3, 2, 2);
    let world = snapshot(area, tile_codes::STAIR);
    let predicate = proof(area, area.min, vec![area.max]);
    assert_eq!(
        PredicateEvidence::laboratory(&world)?.evaluate(&predicate)?,
        PredicateTruth::True
    );
    assert_eq!(
        PredicateEvidence::untrusted(&world)?.evaluate(&predicate)?,
        PredicateTruth::Unknown
    );
    let mut policy = EvidencePolicy::at(world.anchor());
    policy.all_entities = EvidenceCoverage::Complete;
    policy.terrain_regions.push(area);
    policy.sources.insert(EvidenceSource::Observed {
        field: "native.map".to_owned(),
        source_digest: Digest32::of_bytes(b"native terrain"),
    });
    let scoped = PredicateEvidence::scoped(&world, policy)?;
    for candidate in [
        predicate.clone(),
        Predicate::Not(Box::new(predicate.clone())),
        Predicate::All(vec![Predicate::True, predicate.clone()]),
    ] {
        assert_eq!(scoped.evaluate(&candidate)?, PredicateTruth::Unknown);
    }
    assert!(crate::evaluate(&world, &predicate));
    Ok(())
}

#[test]
fn bounds_cover_volume_targets_and_coordinate_extremes_before_allocation() -> Result<()> {
    let area = region(8, 8, 16);
    let valid = proof(area, area.min, vec![area.max]);
    valid.validate_shape()?;
    let world = snapshot(area, tile_codes::STAIR);
    assert_eq!(truth(&world, area)?, PredicateTruth::True);
    for invalid in [
        proof(region(8, 8, 17), area.min, vec![area.max]),
        proof(
            MapCuboid {
                min: area.max,
                max: area.min,
            },
            area.min,
            vec![area.max],
        ),
        proof(area, MapCoord::new(-1, 0, 0), vec![area.max]),
        proof(area, area.min, vec![MapCoord::new(7, 7, 16)]),
        proof(area, area.min, vec![]),
        proof(area, area.min, vec![area.max; 2]),
        proof(
            area,
            area.min,
            (0..17).map(|x| MapCoord::new(x, 0, 0)).collect(),
        ),
        Predicate::All(vec![valid; 19]),
    ] {
        assert!(invalid.validate_shape().is_err());
        assert!(!crate::evaluate(&world, &invalid));
        assert!(
            PredicateEvidence::laboratory(&world)?
                .evaluate(&invalid)
                .is_err()
        );
    }
    assert!(
        proof(area, area.min, vec![area.max; 2])
            .normalized()
            .validate_shape()
            .is_err()
    );
    for z in [i32::MIN, i32::MAX - 1] {
        let extreme = MapCuboid {
            min: MapCoord::new(i32::MAX - 3, i32::MIN, z),
            max: MapCoord::new(i32::MAX, i32::MIN, z + 1),
        };
        assert_eq!(
            truth(&snapshot(extreme, tile_codes::STAIR), extreme)?,
            PredicateTruth::True
        );
    }
    let tall = region(1, 1, 1024);
    assert_eq!(
        truth(&snapshot(tall, tile_codes::STAIR), tall)?,
        PredicateTruth::False
    );
    Ok(())
}

#[test]
fn appended_tag_preserves_legacy_bytes_and_legacy_floor_only_meaning() -> Result<()> {
    let area = region(4, 2, 1);
    let first = MapCoord::new(3, 1, 0);
    let second = MapCoord::new(2, 1, 0);
    let paired = proof(area, area.min, vec![first, second]);
    let reordered = proof(area, area.min, vec![second, first]);
    assert_eq!(paired.normalized(), reordered.normalized());
    assert_eq!(paired.canonical_bytes(), reordered.canonical_bytes());
    let legacy = Predicate::LaboratoryRoomAccess {
        area,
        entry: area.min,
        targets: vec![first, second],
    };
    let mut expected = vec![11];
    for at in [area.min, area.max, area.min] {
        for component in [at.x, at.y, at.z] {
            expected.extend_from_slice(&component.to_be_bytes());
        }
    }
    expected.extend_from_slice(&2u64.to_be_bytes());
    for at in [second, first] {
        for component in [at.x, at.y, at.z] {
            expected.extend_from_slice(&component.to_be_bytes());
        }
    }
    assert_eq!(legacy.canonical_bytes(), expected);
    expected[0] = 13;
    assert_eq!(paired.canonical_bytes(), expected);
    let world = snapshot(area, tile_codes::STAIR);
    assert_eq!(
        PredicateEvidence::laboratory(&world)?.evaluate(&legacy)?,
        PredicateTruth::False
    );
    assert_eq!(
        PredicateEvidence::laboratory(&world)?.evaluate(&paired)?,
        PredicateTruth::True
    );
    assert!(
        Predicate::LaboratoryRoomAccess {
            area: region(3, 1, 2),
            entry: MapCoord::new(0, 0, 0),
            targets: vec![MapCoord::new(2, 0, 1)],
        }
        .validate_shape()
        .is_err()
    );
    assert_ne!(
        LABORATORY_ACCESS_POLICY,
        LABORATORY_ACCESS_PAIRED_STAIRS_POLICY
    );
    Ok(())
}

#[test]
fn query_tokens_bind_the_new_policy_and_all_rows_charge_access_work() -> Result<()> {
    let area = region(4, 2, 1);
    let mut world = snapshot(area, tile_codes::FLOOR);
    for id in 1..=3 {
        let mut unit = building(id, area);
        unit.kind = EntityKind::Unit;
        unit.fields.clear();
        world.graph.entities.insert(unit.id, unit);
    }
    world.refresh_hash();
    let mut query = WorldQuery {
        kinds: vec![EntityKind::Unit],
        predicate: Some(proof(
            area,
            area.min,
            vec![area.max, MapCoord::new(2, 1, 0)],
        )),
        order: QueryOrder::EntityIdAscending,
        limit: 1,
        continuation: None,
    };
    query.continuation = crate::execute_query(&world, &query, 10)?.continuation;
    assert!(query.continuation.is_some());
    query.predicate = Some(proof(
        area,
        area.min,
        vec![MapCoord::new(2, 1, 0), area.max],
    ));
    assert_eq!(
        crate::execute_query(&world, &query, 10)?.entities[0]
            .id
            .get(),
        2
    );
    query.predicate = Some(Predicate::LaboratoryRoomAccess {
        area,
        entry: area.min,
        targets: vec![MapCoord::new(2, 1, 0), area.max],
    });
    assert!(matches!(
        crate::execute_query(&world, &query, 10),
        Err(error) if error.code == ErrorCode::StaleAnchor
    ));
    let large = region(8, 8, 16);
    let mut world = snapshot(large, tile_codes::STAIR);
    for id in 1..=20 {
        let mut unit = building(id, area);
        unit.kind = EntityKind::Unit;
        unit.fields.clear();
        world.graph.entities.insert(unit.id, unit);
    }
    world.refresh_hash();
    let query = WorldQuery {
        predicate: Some(proof(large, large.min, vec![large.max])),
        continuation: None,
        ..query
    };
    for result in [
        crate::query::execute_query(&world, &query, 10),
        crate::execute_query(&world, &query, 10),
    ] {
        assert!(matches!(result, Err(error) if error.code == ErrorCode::BudgetExceeded));
    }
    Ok(())
}
