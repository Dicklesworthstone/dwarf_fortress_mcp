use super::*;

use dfmcp_core::{Digest32, FortressId, ObservationCursor};

use crate::{
    EvidenceCoverage, EvidencePolicy, EvidenceSource, Fact, FactPresence, FactSource, Predicate,
    PredicateEvidence, QueryOrder, WorldGraph, WorldQuery,
};

fn area(width: i32, height: i32) -> MapCuboid {
    MapCuboid {
        min: MapCoord::new(0, 0, 10),
        max: MapCoord::new(width - 1, height - 1, 10),
    }
}

fn snapshot(area: MapCuboid, terrain: u32) -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    let chunks: BTreeSet<_> = crate::terrain::region_tiles(area)
        .map(|at| crate::terrain::tile_slot(at).chunk)
        .collect();
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

fn building(id: u64, footprint: MapCuboid, stage: &str) -> EntityRecord {
    EntityRecord {
        id: EntityId::new(id),
        generation: 1,
        revision: 1,
        kind: EntityKind::Building,
        label: "access obstacle".to_owned(),
        fields: BTreeMap::from([
            (
                "construction_stage".to_owned(),
                fact(Value::Text(stage.to_owned())),
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

fn placement(id: u64, at: MapCoord, stage: &str) -> EntityRecord {
    let mut record = building(id, MapCuboid { min: at, max: at }, stage);
    for (name, value) in [
        ("placement_model", Value::Text("furniture/1".to_owned())),
        ("building_kind", Value::Text("furniture:Bed".to_owned())),
        ("source_item", Value::Entity(EntityId::new(99))),
        ("source_item_generation", Value::U64(1)),
        ("position", Value::Coord(at)),
        ("required_ticks", Value::U64(500)),
        (
            "progress_ticks",
            Value::U64(if stage == "complete" { 500 } else { 25 }),
        ),
        (
            "attachment_state",
            Value::Text(
                if stage == "cancelled" {
                    "released"
                } else {
                    "attached"
                }
                .to_owned(),
            ),
        ),
    ] {
        record.fields.insert(name.to_owned(), fact(value));
    }
    record
}

fn proof(area: MapCuboid, entry: MapCoord, targets: Vec<MapCoord>) -> Predicate {
    Predicate::LaboratoryRoomAccess {
        area,
        entry,
        targets,
    }
}

#[test]
fn all_binary_three_by_three_maps_match_an_independent_closure_oracle() -> Result<()> {
    let region = area(3, 3);
    let entry = region.min;
    let target = region.max;
    for mask in 0..512u32 {
        let mut world = snapshot(region, tile_codes::SOLID_WALL);
        for index in 0..9 {
            if mask & (1 << index) != 0 {
                world.set_tile_code(MapCoord::new(index % 3, index / 3, 10), tile_codes::FLOOR)?;
            }
        }
        let mut reachable = BTreeSet::new();
        if mask & 1 != 0 {
            reachable.insert((0i32, 0i32));
        }
        loop {
            let before = reachable.clone();
            for y in 0..3i32 {
                for x in 0..3i32 {
                    if (x, y) == (2, 2) || mask & (1 << (y * 3 + x)) == 0 {
                        continue;
                    }
                    if before
                        .iter()
                        .any(|(a, b)| (x - a).abs() + (y - b).abs() == 1)
                    {
                        reachable.insert((x, y));
                    }
                }
            }
            if reachable == before {
                break;
            }
        }
        let expected = if reachable.contains(&(1, 2)) || reachable.contains(&(2, 1)) {
            PredicateTruth::True
        } else {
            PredicateTruth::False
        };
        assert_eq!(
            laboratory_room_access_truth(&world, region, entry, &[target])?,
            expected,
            "mask {mask}"
        );
    }
    Ok(())
}

#[test]
fn every_target_needs_an_approach_and_buildings_allow_only_actual_detours() -> Result<()> {
    let region = area(5, 3);
    let entry = MapCoord::new(0, 1, 10);
    let target = MapCoord::new(4, 1, 10);
    let mut world = snapshot(region, tile_codes::FLOOR);
    let obstacle = building(
        1,
        MapCuboid {
            min: MapCoord::new(2, 1, 10),
            max: MapCoord::new(2, 1, 10),
        },
        "complete",
    );
    world.graph.entities.insert(obstacle.id, obstacle);
    assert_eq!(
        laboratory_room_access_truth(&world, region, entry, &[target])?,
        PredicateTruth::True
    );
    let wall = building(
        1,
        MapCuboid {
            min: MapCoord::new(2, 0, 10),
            max: MapCoord::new(2, 2, 10),
        },
        "planned",
    );
    world.graph.entities.insert(wall.id, wall);
    assert_eq!(
        laboratory_room_access_truth(&world, region, entry, &[MapCoord::new(0, 2, 10), target])?,
        PredicateTruth::False
    );
    // Floors immediately outside the selected region and on another level do
    // not authorize a detour through the bounded model's cut.
    world.graph.chunks.insert(
        crate::ChunkCoord { x: 0, y: 0, z: 11 },
        crate::terrain::uniform_chunk(crate::ChunkCoord { x: 0, y: 0, z: 11 }, tile_codes::FLOOR),
    );
    assert_eq!(
        laboratory_room_access_truth(&world, region, entry, &[target])?,
        PredicateTruth::False
    );
    // Targets themselves block a one-tile corridor even without item records.
    let line = area(5, 1);
    let empty = snapshot(line, tile_codes::FLOOR);
    assert_eq!(
        laboratory_room_access_truth(&empty, line, line.min, &[MapCoord::new(2, 0, 10), line.max])?,
        PredicateTruth::False
    );
    Ok(())
}

#[test]
fn missing_terrain_is_potential_access_and_known_cuts_remain_false() -> Result<()> {
    let region = MapCuboid {
        min: MapCoord::new(14, 0, 10),
        max: MapCoord::new(17, 0, 10),
    };
    let mut world = snapshot(
        MapCuboid {
            min: region.min,
            max: region.min,
        },
        tile_codes::FLOOR,
    );
    assert_eq!(
        laboratory_room_access_truth(&world, region, region.min, &[region.max])?,
        PredicateTruth::Unknown
    );
    world.set_tile_code(MapCoord::new(15, 0, 10), tile_codes::SOLID_WALL)?;
    assert_eq!(
        laboratory_room_access_truth(&world, region, region.min, &[region.max])?,
        PredicateTruth::False
    );
    world.set_tile_code(MapCoord::new(15, 0, 10), tile_codes::FLOOR)?;
    world.graph.chunks.insert(
        crate::ChunkCoord { x: 1, y: 0, z: 10 },
        crate::terrain::uniform_chunk(crate::ChunkCoord { x: 1, y: 0, z: 10 }, tile_codes::FLOOR),
    );
    assert_eq!(
        laboratory_room_access_truth(&world, region, region.min, &[region.max])?,
        PredicateTruth::True
    );
    let chunk_coord = crate::ChunkCoord { x: 1, y: 0, z: 10 };
    for mutation in 0..4 {
        let mut chunk = crate::terrain::uniform_chunk(chunk_coord, tile_codes::FLOOR);
        match mutation {
            0 => chunk.terrain_runs.extend(std::iter::repeat_n(
                crate::TerrainRun {
                    tile_code: tile_codes::FLOOR,
                    length: 0,
                },
                256,
            )),
            1 => chunk.terrain_runs.push(crate::TerrainRun {
                tile_code: tile_codes::FLOOR,
                length: 0,
            }),
            2 => chunk.terrain_runs[0].length = 255,
            _ => chunk.coord.x = 2,
        }
        world.graph.chunks.insert(chunk_coord, chunk);
        assert_eq!(
            laboratory_room_access_truth(&world, region, region.min, &[region.max])?,
            PredicateTruth::Unknown
        );
    }
    Ok(())
}

#[test]
fn complete_census_rejects_unqualified_malformed_and_aliased_occupancy() -> Result<()> {
    let region = area(4, 1);
    let obstacle = MapCuboid {
        min: MapCoord::new(1, 0, 10),
        max: MapCoord::new(1, 0, 10),
    };
    for mutation in 0..9 {
        let mut world = snapshot(region, tile_codes::FLOOR);
        let mut record = building(1, obstacle, "complete");
        match mutation {
            0 => {
                record.fields.remove("footprint_max");
            }
            1 => {
                record.fields.insert(
                    "footprint_max".to_owned(),
                    fact(Value::Coord(MapCoord::new(0, 0, 10))),
                );
            }
            2 => {
                record.fields.insert(
                    "construction_stage".to_owned(),
                    fact(Value::Text("unrecognized".to_owned())),
                );
            }
            3 => {
                record
                    .fields
                    .get_mut("footprint_min")
                    .ok_or_else(|| invalid("fixture field missing"))?
                    .source = FactSource::Replay;
            }
            4 => {
                record
                    .fields
                    .get_mut("footprint_min")
                    .ok_or_else(|| invalid("fixture field missing"))?
                    .observed_at = GameTick(11);
            }
            5 => {
                record
                    .fields
                    .get_mut("footprint_min")
                    .ok_or_else(|| invalid("fixture field missing"))?
                    .source_digest = Digest32::of_bytes(b"unqualified");
            }
            6 => {
                record
                    .fields
                    .get_mut("footprint_min")
                    .ok_or_else(|| invalid("fixture field missing"))?
                    .presence = Some(FactPresence::Unknown("unobserved".to_owned()));
            }
            7 => {
                record.kind = EntityKind::Other("building".to_owned());
            }
            _ => {
                record.id = EntityId::new(2);
            }
        }
        world.graph.entities.insert(EntityId::new(1), record);
        let expected = if mutation == 7 {
            PredicateTruth::False
        } else {
            PredicateTruth::Unknown
        };
        assert_eq!(
            laboratory_room_access_truth(&world, region, region.min, &[region.max])?,
            expected,
            "mutation {mutation}"
        );
        assert_eq!(
            laboratory_building_footprints(&world).is_ok(),
            mutation == 7
        );
    }
    // An unlocated building cannot be dismissed as outside the requested area.
    let mut world = snapshot(region, tile_codes::FLOOR);
    let mut record = building(1, obstacle, "complete");
    record.fields.clear();
    world.graph.entities.insert(record.id, record);
    assert_eq!(
        laboratory_room_access_truth(&world, region, region.min, &[region.max])?,
        PredicateTruth::Unknown
    );
    Ok(())
}

#[test]
fn released_furniture_requires_original_identity_before_freeing_access() -> Result<()> {
    let region = area(4, 1);
    let at = MapCoord::new(1, 0, 10);
    let mut world = snapshot(region, tile_codes::FLOOR);
    for stage in ["complete", "under_construction", "cancelled"] {
        world
            .graph
            .entities
            .insert(EntityId::new(1), placement(1, at, stage));
        assert_eq!(
            laboratory_room_access_truth(&world, region, region.min, &[region.max])?,
            if stage == "cancelled" {
                PredicateTruth::True
            } else {
                PredicateTruth::False
            }
        );
    }
    // Released history deliberately needs no current item or producer: a later
    // placement may already have reused them under another exact identity.
    assert!(laboratory_building_footprints(&world)?.is_empty());
    for mutation in 0..5 {
        let mut record = placement(1, at, "cancelled");
        match mutation {
            0 => record.generation = 2,
            1 => {
                record.fields.insert(
                    "attachment_state".to_owned(),
                    fact(Value::Text("attached".to_owned())),
                );
            }
            2 => {
                record
                    .fields
                    .insert("progress_ticks".to_owned(), fact(Value::U64(500)));
            }
            3 => {
                record.fields.remove("source_item_generation");
            }
            _ => {
                record.fields.insert(
                    "footprint_max".to_owned(),
                    fact(Value::Coord(MapCoord::new(2, 0, 10))),
                );
            }
        }
        world.graph.entities.insert(record.id, record);
        assert_eq!(
            laboratory_room_access_truth(&world, region, region.min, &[region.max])?,
            PredicateTruth::Unknown
        );
    }
    let generic = building(1, MapCuboid { min: at, max: at }, "cancelled");
    world.graph.entities.insert(generic.id, generic);
    assert_eq!(
        laboratory_room_access_truth(&world, region, region.min, &[region.max])?,
        PredicateTruth::True
    );
    Ok(())
}

#[test]
fn only_explicit_laboratory_evidence_can_establish_the_model_predicate() -> Result<()> {
    let region = area(4, 2);
    let world = snapshot(region, tile_codes::FLOOR);
    let predicate = proof(region, region.min, vec![region.max]);
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
    policy.terrain_regions.push(region);
    policy.sources.insert(EvidenceSource::Observed {
        field: "native.map".to_owned(),
        source_digest: Digest32::of_bytes(b"qualified native map"),
    });
    let live = PredicateEvidence::scoped(&world, policy)?;
    for untrusted in [
        predicate.clone(),
        Predicate::Not(Box::new(predicate.clone())),
        Predicate::All(vec![Predicate::True, predicate.clone()]),
    ] {
        assert_eq!(live.evaluate(&untrusted)?, PredicateTruth::Unknown);
    }
    assert!(crate::evaluate(&world, &predicate));
    Ok(())
}

#[test]
fn bounds_precede_allocations_and_normalization_cannot_erase_invalid_targets() -> Result<()> {
    let region = area(32, 32);
    let world = snapshot(region, tile_codes::FLOOR);
    let targets: Vec<_> = (0..16).map(|x| MapCoord::new(x * 2, 31, 10)).collect();
    let valid = proof(region, region.min, targets.clone());
    assert_eq!(
        PredicateEvidence::laboratory(&world)?.evaluate(&valid)?,
        PredicateTruth::True
    );
    let mut invalid_inputs = vec![
        proof(area(1025, 1), region.min, vec![region.min]),
        proof(
            MapCuboid {
                min: region.max,
                max: region.min,
            },
            region.min,
            vec![region.min],
        ),
        proof(
            MapCuboid {
                min: region.min,
                max: MapCoord::new(1, 1, 11),
            },
            region.min,
            vec![region.min],
        ),
        proof(region, MapCoord::new(-1, 0, 10), vec![region.max]),
        proof(region, region.min, vec![MapCoord::new(32, 31, 10)]),
        proof(region, region.min, vec![]),
        proof(region, region.min, vec![region.max; 2]),
        proof(
            region,
            region.min,
            (0..17).map(|x| MapCoord::new(x, 31, 10)).collect(),
        ),
    ];
    invalid_inputs.push(Predicate::All(vec![valid; 19]));
    for invalid in invalid_inputs {
        assert!(invalid.validate_shape().is_err());
        assert_eq!(
            crate::evaluate_truth_for(&world, EntityId::NIL, &invalid),
            PredicateTruth::Unknown
        );
        assert!(!crate::evaluate(&world, &invalid));
        assert!(
            PredicateEvidence::laboratory(&world)?
                .evaluate(&invalid)
                .is_err()
        );
    }
    let duplicate = proof(region, region.min, vec![region.max; 2]).normalized();
    assert!(duplicate.validate_shape().is_err());
    let near_limit = MapCuboid {
        min: MapCoord::new(i32::MAX - 3, i32::MIN, -2),
        max: MapCoord::new(i32::MAX, i32::MIN, -2),
    };
    assert_eq!(
        laboratory_room_access_truth(
            &snapshot(near_limit, tile_codes::FLOOR),
            near_limit,
            near_limit.min,
            &[near_limit.max]
        )?,
        PredicateTruth::True
    );
    let mut too_many = WorldGraph::default();
    for raw in 1..=MAX_LABORATORY_ACCESS_ENTITIES as u64 + 1 {
        let mut record = building(raw, area(1, 1), "cancelled");
        record.kind = EntityKind::Unit;
        record.fields.clear();
        too_many.entities.insert(record.id, record);
    }
    let excessive = WorldSnapshot::new(
        FortressId::new(1),
        GameTick(10),
        ObservationCursor::ORIGIN,
        true,
        too_many,
    );
    assert!(
        matches!(laboratory_building_footprints(&excessive), Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn access_canonical_bytes_are_order_independent_and_append_the_existing_tags() -> Result<()> {
    let region = area(5, 3);
    let first = MapCoord::new(4, 2, 10);
    let second = MapCoord::new(2, 1, 10);
    let a = proof(region, region.min, vec![first, second]);
    let b = proof(region, region.min, vec![second, first]);
    assert_eq!(a.normalized(), b.normalized());
    assert_eq!(a.canonical_bytes(), b.canonical_bytes());
    assert_eq!(a.canonical_bytes()[0], 11);
    assert_eq!(a.canonical_bytes().len(), 1 + 36 + 8 + 24);
    assert_eq!(Predicate::Paused(false).canonical_bytes(), vec![6, 0]);
    let mut expected = vec![10];
    for coordinate in [0i32, 0, 10, 4, 2, 10] {
        expected.extend_from_slice(&coordinate.to_be_bytes());
    }
    expected.extend_from_slice(&1u32.to_be_bytes());
    assert_eq!(
        Predicate::RegionTerrain {
            area: region,
            tile_code: 1
        }
        .canonical_bytes(),
        expected
    );
    let mut world = snapshot(region, tile_codes::FLOOR);
    for id in 1..=3 {
        let mut unit = building(id, area(1, 1), "cancelled");
        unit.kind = EntityKind::Unit;
        unit.fields.clear();
        world.graph.entities.insert(unit.id, unit);
    }
    world.refresh_hash();
    let mut query = WorldQuery {
        kinds: vec![EntityKind::Unit],
        predicate: Some(a),
        order: QueryOrder::EntityIdAscending,
        limit: 1,
        continuation: None,
    };
    query.continuation = crate::execute_query(&world, &query, 10)?.continuation;
    assert!(query.continuation.is_some());
    query.predicate = Some(b);
    assert_eq!(
        crate::execute_query(&world, &query, 10)?.entities[0]
            .id
            .get(),
        2
    );
    query.predicate = Some(proof(region, MapCoord::new(1, 0, 10), vec![second, first]));
    assert!(
        matches!(crate::execute_query(&world, &query, 10), Err(error) if error.code == ErrorCode::StaleAnchor)
    );
    Ok(())
}

#[test]
fn direct_and_paged_queries_charge_global_access_work_for_every_row() -> Result<()> {
    let region = area(32, 32);
    let mut world = snapshot(region, tile_codes::FLOOR);
    for raw in 1..=20 {
        let mut unit = building(raw, area(1, 1), "cancelled");
        unit.kind = EntityKind::Unit;
        unit.fields.clear();
        world.graph.entities.insert(unit.id, unit);
    }
    world.refresh_hash();
    let query = WorldQuery {
        kinds: vec![EntityKind::Unit],
        predicate: Some(proof(region, region.min, vec![region.max])),
        order: QueryOrder::EntityIdAscending,
        limit: 1,
        continuation: None,
    };
    for result in [
        crate::query::execute_query(&world, &query, 10),
        crate::execute_query(&world, &query, 10),
    ] {
        assert!(matches!(result, Err(error) if error.code == ErrorCode::BudgetExceeded));
    }
    // The new cost cannot silently narrow the existing ordinary query model.
    let ordinary = WorldQuery {
        predicate: Some(Predicate::True),
        ..query
    };
    assert_eq!(
        crate::query::execute_query(&world, &ordinary, 10)?.matched,
        20
    );
    Ok(())
}

#[test]
fn overlapping_rectangle_rasterization_matches_union_occupancy() -> Result<()> {
    let region = MapCuboid {
        min: MapCoord::new(-2, -2, 10),
        max: MapCoord::new(2, 2, 10),
    };
    let grid = Grid {
        area: region,
        width: 5,
        height: 5,
    };
    let rectangles = [
        MapCuboid {
            min: MapCoord::new(-5, -1, 9),
            max: MapCoord::new(0, 1, 11),
        },
        MapCuboid {
            min: MapCoord::new(-1, -5, 10),
            max: MapCoord::new(1, 5, 10),
        },
        MapCuboid {
            min: MapCoord::new(2, 2, 10),
            max: MapCoord::new(4, 4, 10),
        },
        MapCuboid {
            min: MapCoord::new(-2, -2, 11),
            max: MapCoord::new(2, 2, 11),
        },
        MapCuboid {
            min: MapCoord::new(3, -2, 10),
            max: MapCoord::new(4, 2, 10),
        },
    ];
    for mask in 0..32u32 {
        let selected: BTreeMap<_, _> = rectangles
            .iter()
            .enumerate()
            .filter(|(index, _)| mask & (1 << index) != 0)
            .map(|(index, area)| (EntityId::new(index as u64 + 1), *area))
            .collect();
        let expected: Vec<_> = crate::terrain::region_tiles(region)
            .map(|at| selected.values().any(|area| area.contains(at)))
            .collect();
        assert_eq!(grid.occupied(&selected), expected, "mask {mask}");
    }
    Ok(())
}
