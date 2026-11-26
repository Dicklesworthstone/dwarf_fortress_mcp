//! Malformed terrain cannot authorize effects or prove original room goals.
//! Owner: df-lab-connected-room-access-7c2.

use dfmcp_core::{
    DfmcpError, ErrorCode, FortressId, GameTick, MapCoord, MapCuboid, ObservationCursor, Result,
};
use dfmcp_intent::effects::{self, BUILD_TICKS, DIG_TICKS_PER_TILE};
use dfmcp_intent::{Action, BuildingKind, DigMode, MaterialSelector};
use dfmcp_world::terrain::{tile_codes, tile_slot, uniform_chunk};
use dfmcp_world::{MapChunk, Predicate, TerrainRun, WorldGraph, WorldSnapshot};

const SITE: MapCoord = MapCoord::new(3, 2, 5);

fn world(tile_code: u32) -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    let key = tile_slot(SITE).chunk;
    graph.chunks.insert(key, uniform_chunk(key, tile_code));
    WorldSnapshot::new(
        FortressId::new(91),
        GameTick(1_000),
        ObservationCursor::ORIGIN,
        false,
        graph,
    )
}

fn area() -> Result<MapCuboid> {
    MapCuboid::new(SITE, SITE)
}

fn mine() -> Result<Action> {
    Ok(Action::DesignateDig {
        area: area()?,
        mode: DigMode::Mine,
    })
}

fn workshop() -> Result<Action> {
    Ok(Action::Build {
        kind: BuildingKind::Workshop("Carpenters".to_owned()),
        location: SITE,
        footprint: area()?,
        material: MaterialSelector::default(),
    })
}

fn corrupt_chunk(chunk: &mut MapChunk, variant: usize) {
    match variant {
        0 => chunk.coord.z += 1,
        1 => chunk.terrain_runs.insert(
            0,
            TerrainRun {
                tile_code: tile_codes::FLOOR,
                length: 0,
            },
        ),
        2 => chunk.terrain_runs.push(TerrainRun {
            tile_code: tile_codes::FLOOR,
            length: 0,
        }),
        _ => chunk.terrain_runs.extend(std::iter::repeat_n(
            TerrainRun {
                tile_code: tile_codes::SOLID_WALL,
                length: 0,
            },
            256,
        )),
    }
}

fn corrupt(snapshot: &mut WorldSnapshot, variant: usize) -> Result<()> {
    let chunk = snapshot
        .graph
        .chunks
        .get_mut(&tile_slot(SITE).chunk)
        .ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "terrain fixture missing",
            )
        })?;
    corrupt_chunk(chunk, variant);
    snapshot.refresh_hash();
    Ok(())
}

fn require_precondition_refusal<T>(result: Result<T>) -> Result<()> {
    let error = result.err().ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "malformed terrain unexpectedly admitted an effect",
        )
    })?;
    assert_eq!(error.code, ErrorCode::PreconditionsFailed);
    Ok(())
}

#[test]
fn malformed_terrain_cannot_dispatch_work_or_establish_a_completed_room() -> Result<()> {
    for variant in 0..4 {
        for code in [tile_codes::SOLID_WALL, tile_codes::FLOOR] {
            let mut source = world(code);
            corrupt(&mut source, variant)?;
            let proof = Predicate::RegionTerrain {
                area: area()?,
                tile_code: code,
            };
            assert!(!dfmcp_world::evaluate(&source, &proof), "variant {variant}");
            for action in [mine()?, workshop()?] {
                let mut attempted = source.clone();
                require_precondition_refusal(effects::apply_effect(
                    &mut attempted,
                    &action,
                    "malformed-terrain",
                ))?;
                assert_eq!(attempted, source, "variant {variant}");
            }
        }
    }
    Ok(())
}

#[test]
fn retained_excavation_refuses_malformed_source_before_any_physical_progress() -> Result<()> {
    for variant in 0..4 {
        let mut source = world(tile_codes::SOLID_WALL);
        effects::apply_effect(&mut source, &mine()?, "original-dig")?;
        effects::apply_effect(
            &mut source,
            &Action::CreateWorkOrder {
                name: "unrelated work".to_owned(),
                job_token: "SMELT_IRON".to_owned(),
                amount: 1,
                conditions: Vec::new(),
            },
            "unrelated-order",
        )?;
        corrupt(&mut source, variant)?;
        source.tick = GameTick(source.tick.0 + DIG_TICKS_PER_TILE);
        let before = source.clone();
        require_precondition_refusal(effects::advance_effects(&mut source, DIG_TICKS_PER_TILE))?;
        assert_eq!(source, before, "variant {variant}");
    }
    Ok(())
}

#[test]
fn canonical_excavation_still_establishes_floor_then_completes_original_construction() -> Result<()>
{
    let mut source = world(tile_codes::SOLID_WALL);
    let original_dig = mine()?;
    effects::apply_effect(&mut source, &original_dig, "original-dig")?;
    source.tick = GameTick(source.tick.0 + DIG_TICKS_PER_TILE);
    effects::advance_effects(&mut source, DIG_TICKS_PER_TILE)?;
    for predicate in
        effects::default_postconditions(&original_dig, "original-dig", source.fortress_id)
    {
        assert!(dfmcp_world::evaluate(&source, &predicate));
    }
    let original_build = workshop()?;
    effects::apply_effect(&mut source, &original_build, "original-workshop")?;
    source.tick = GameTick(source.tick.0 + BUILD_TICKS);
    effects::advance_effects(&mut source, BUILD_TICKS)?;
    for predicate in
        effects::default_postconditions(&original_build, "original-workshop", source.fortress_id)
    {
        assert!(dfmcp_world::evaluate(&source, &predicate));
    }
    Ok(())
}
