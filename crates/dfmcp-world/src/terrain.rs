//! Tile-level access to the canonical chunked terrain and region predicates.
//!
//! Canonical terrain is stored as run-length encoded 16x16x1 chunks keyed by
//! `ChunkCoord { x: x.div_euclid(16), y: y.div_euclid(16), z }`, matching the
//! spatial index. Each chunk must match its lookup coordinate and contain at
//! most 256 positive runs covering exactly 256 tiles. A tile whose chunk is
//! absent, malformed, or not 16x16 is unknown: region predicates never treat
//! missing terrain as a match. Partially observed chunks retain explicit UNKNOWN
//! cells instead of inventing terrain.

use dfmcp_core::{DfmcpError, ErrorCode, MapCoord, MapCuboid, Result};

use crate::model::{ChunkCoord, MapChunk, TerrainRun, WorldSnapshot};

/// Edge length of a canonical terrain chunk.
pub const TERRAIN_CHUNK_EDGE: i32 = 16;
/// Tiles in one canonical terrain chunk.
pub const TERRAIN_CHUNK_TILES: u32 = 256;
/// Largest region a single terrain predicate or effect may cover.
pub const MAX_REGION_TERRAIN_TILES: u64 = 65_536;

/// Canonical tile codes, shared with [`crate::TileType::from_tile_code`].
pub mod tile_codes {
    /// No observed canonical terrain at this cell, including hidden, missing
    /// and unsupported source values. Never a predicate match or effect input.
    pub const UNKNOWN: u32 = u32::MAX;
    pub const OPEN_SPACE: u32 = 0;
    pub const FLOOR: u32 = 1;
    pub const SOLID_WALL: u32 = 2;
    pub const STAIR: u32 = 3;
    pub const RAMP: u32 = 4;
    pub const FORTIFICATION: u32 = 5;
    pub const TREE: u32 = 6;
    pub const MAGMA_WALL: u32 = 7;
    pub const CHASM: u32 = 8;
}

/// Location of one tile inside canonical chunk storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileSlot {
    pub chunk: ChunkCoord,
    pub offset: u32,
}

#[must_use]
pub fn tile_slot(coord: MapCoord) -> TileSlot {
    let local_x = coord.x.rem_euclid(TERRAIN_CHUNK_EDGE);
    let local_y = coord.y.rem_euclid(TERRAIN_CHUNK_EDGE);
    TileSlot {
        chunk: ChunkCoord {
            x: coord.x.div_euclid(TERRAIN_CHUNK_EDGE),
            y: coord.y.div_euclid(TERRAIN_CHUNK_EDGE),
            z: coord.z,
        },
        // Both locals are in 0..16, so the offset is in 0..256.
        offset: (local_y * TERRAIN_CHUNK_EDGE + local_x).unsigned_abs(),
    }
}

fn chunk_is_canonical(chunk: &MapChunk, expected: ChunkCoord) -> bool {
    chunk.coord == expected
        && i32::from(chunk.width) == TERRAIN_CHUNK_EDGE
        && i32::from(chunk.height) == TERRAIN_CHUNK_EDGE
        // Bound the source before scanning or allocating. Zero-length runs can
        // otherwise give an arbitrarily large vector an apparently valid sum.
        && !chunk.terrain_runs.is_empty()
        && chunk.terrain_runs.len() <= TERRAIN_CHUNK_TILES as usize
        && chunk.terrain_runs.iter().all(|run| run.length > 0)
        && chunk.encoded_tile_count() == Some(TERRAIN_CHUNK_TILES)
}

fn code_in_chunk(chunk: &MapChunk, slot: TileSlot) -> Option<u32> {
    if !chunk_is_canonical(chunk, slot.chunk) {
        return None;
    }
    let mut start = 0u32;
    for run in &chunk.terrain_runs {
        let end = start.checked_add(run.length)?;
        if slot.offset < end {
            return Some(run.tile_code);
        }
        start = end;
    }
    None
}

/// Run-length encode tile codes, merging equal neighbours.
fn encode_runs(codes: &[u32]) -> Vec<TerrainRun> {
    let mut runs: Vec<TerrainRun> = Vec::new();
    for code in codes {
        match runs.last_mut() {
            Some(run) if run.tile_code == *code => run.length += 1,
            _ => runs.push(TerrainRun {
                tile_code: *code,
                length: 1,
            }),
        }
    }
    runs
}

fn decode_runs(chunk: &MapChunk, expected: ChunkCoord) -> Option<Vec<u32>> {
    if !chunk_is_canonical(chunk, expected) {
        return None;
    }
    let mut codes = Vec::with_capacity(TERRAIN_CHUNK_TILES as usize);
    for run in &chunk.terrain_runs {
        codes.extend(std::iter::repeat_n(run.tile_code, run.length as usize));
    }
    Some(codes)
}

/// Three-valued result of comparing a region against an expected tile code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionTruth {
    /// Every tile is known and matches.
    Matches,
    /// At least one known tile differs.
    Differs,
    /// No known tile differs, but at least one tile is unknown.
    Unknown,
}

/// Validate a region before it is evaluated or mutated.
pub fn validate_region(area: MapCuboid) -> Result<u64> {
    if area.min.x > area.max.x || area.min.y > area.max.y || area.min.z > area.max.z {
        return Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "terrain region minimum must not exceed maximum",
        ));
    }
    let tiles = area.tile_count().ok_or_else(|| {
        DfmcpError::new(ErrorCode::BudgetExceeded, "terrain region size overflows")
    })?;
    if tiles > MAX_REGION_TERRAIN_TILES {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            format!("terrain region exceeds {MAX_REGION_TERRAIN_TILES} tiles"),
        ));
    }
    Ok(tiles)
}

/// Deterministic tile order used by region scans and effects: z, then y, then x.
pub fn region_tiles(area: MapCuboid) -> impl Iterator<Item = MapCoord> {
    (area.min.z..=area.max.z).flat_map(move |z| {
        (area.min.y..=area.max.y)
            .flat_map(move |y| (area.min.x..=area.max.x).map(move |x| MapCoord::new(x, y, z)))
    })
}

impl WorldSnapshot {
    /// The canonical tile code at `coord`, or `None` when the terrain there is
    /// not observed in a canonical 16x16 chunk.
    #[must_use]
    pub fn tile_code_at(&self, coord: MapCoord) -> Option<u32> {
        let slot = tile_slot(coord);
        self.graph
            .chunks
            .get(&slot.chunk)
            .and_then(|chunk| code_in_chunk(chunk, slot))
            .filter(|code| *code != tile_codes::UNKNOWN)
    }

    /// Compare every tile of `area` with `expected`. Oversized or inverted
    /// regions are `Unknown`; callers that need an error validate first.
    #[must_use]
    pub fn region_terrain_truth(&self, area: MapCuboid, expected: u32) -> RegionTruth {
        if validate_region(area).is_err() {
            return RegionTruth::Unknown;
        }
        let mut unknown = false;
        for coord in region_tiles(area) {
            match self.tile_code_at(coord) {
                Some(code) if code == expected => {}
                Some(_) => return RegionTruth::Differs,
                None => unknown = true,
            }
        }
        if unknown {
            RegionTruth::Unknown
        } else {
            RegionTruth::Matches
        }
    }

    /// Replace the tile code at `coord` inside an existing canonical chunk.
    /// Returns whether the tile changed. The chunk revision advances on change;
    /// the caller remains responsible for the snapshot cursor and hash.
    pub fn set_tile_code(&mut self, coord: MapCoord, code: u32) -> Result<bool> {
        self.set_tile_code_checked(coord, code, |_| Ok(()))
    }

    /// Validate the actual observed source and replace it using one bounded
    /// chunk decode. The check runs before any write, even for an otherwise
    /// unchanged target. Refusal preserves the complete chunk and its revision.
    /// Malformed chunks and unobserved source cells are rejected before the
    /// caller's check runs. This carries no authority beyond the existing
    /// terrain writer; an effect owner supplies its semantic transition rule.
    pub fn set_tile_code_checked(
        &mut self,
        coord: MapCoord,
        code: u32,
        check: impl FnOnce(u32) -> Result<()>,
    ) -> Result<bool> {
        let slot = tile_slot(coord);
        let chunk = self.graph.chunks.get_mut(&slot.chunk).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::PreconditionsFailed,
                format!("terrain at {coord:?} is not observed"),
            )
        })?;
        let mut codes = decode_runs(chunk, slot.chunk).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::PreconditionsFailed,
                format!("terrain chunk containing {coord:?} is not a canonical 16x16 chunk"),
            )
        })?;
        let index = slot.offset as usize;
        let Some(current) = codes.get_mut(index) else {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "terrain tile offset is outside its chunk",
            ));
        };
        if *current == tile_codes::UNKNOWN {
            return Err(DfmcpError::new(
                ErrorCode::PreconditionsFailed,
                format!("terrain at {coord:?} has not been observed"),
            ));
        }
        check(*current)?;
        if *current == code {
            return Ok(false);
        }
        *current = code;
        chunk.revision = chunk.revision.checked_add(1).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "terrain chunk revision is exhausted",
            )
        })?;
        chunk.terrain_runs = encode_runs(&codes);
        Ok(true)
    }
}

/// A complete canonical chunk of one tile code, for fixtures and laboratories.
#[must_use]
pub fn uniform_chunk(coord: ChunkCoord, tile_code: u32) -> MapChunk {
    MapChunk {
        coord,
        revision: 1,
        width: 16,
        height: 16,
        terrain_runs: vec![TerrainRun {
            tile_code,
            length: TERRAIN_CHUNK_TILES,
        }],
        sparse_overlays: std::collections::BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::WorldGraph;
    use dfmcp_core::{FortressId, GameTick, ObservationCursor};

    fn snapshot() -> WorldSnapshot {
        let mut graph = WorldGraph::default();
        for (x, y) in [(0, 0), (-1, 0)] {
            let coord = ChunkCoord { x, y, z: 5 };
            graph
                .chunks
                .insert(coord, uniform_chunk(coord, tile_codes::SOLID_WALL));
        }
        WorldSnapshot::new(
            FortressId::new(1),
            GameTick(0),
            ObservationCursor::ORIGIN,
            true,
            graph,
        )
    }

    fn area(a: (i32, i32, i32), b: (i32, i32, i32)) -> Result<MapCuboid> {
        MapCuboid::new(MapCoord::new(a.0, a.1, a.2), MapCoord::new(b.0, b.1, b.2))
    }

    #[test]
    fn negative_coordinates_map_into_their_own_chunk() {
        assert_eq!(
            tile_slot(MapCoord::new(-1, 0, 5)),
            TileSlot {
                chunk: ChunkCoord { x: -1, y: 0, z: 5 },
                offset: 15
            }
        );
        assert_eq!(
            tile_slot(MapCoord::new(17, 18, 2)),
            TileSlot {
                chunk: ChunkCoord { x: 1, y: 1, z: 2 },
                offset: 2 * 16 + 1
            }
        );
    }

    #[test]
    fn set_tile_round_trips_and_merges_runs() -> Result<()> {
        let mut s = snapshot();
        let coord = MapCoord::new(3, 2, 5);
        assert_eq!(s.tile_code_at(coord), Some(tile_codes::SOLID_WALL));
        assert!(s.set_tile_code(coord, tile_codes::FLOOR)?);
        assert!(!s.set_tile_code(coord, tile_codes::FLOOR)?);
        assert_eq!(s.tile_code_at(coord), Some(tile_codes::FLOOR));
        let chunk = &s.graph.chunks[&ChunkCoord { x: 0, y: 0, z: 5 }];
        assert_eq!(chunk.revision, 2);
        assert_eq!(chunk.terrain_runs.len(), 3);
        assert!(s.set_tile_code(coord, tile_codes::SOLID_WALL)?);
        let chunk = &s.graph.chunks[&ChunkCoord { x: 0, y: 0, z: 5 }];
        assert_eq!(chunk.terrain_runs.len(), 1);
        assert_eq!(chunk.encoded_tile_count(), Some(256));
        Ok(())
    }

    #[test]
    fn checked_tile_replacement_validates_current_source_before_any_write() -> Result<()> {
        let mut source = snapshot();
        let at = MapCoord::new(3, 2, 5);
        let key = tile_slot(at).chunk;
        for target in [tile_codes::SOLID_WALL, tile_codes::FLOOR] {
            let before = source.clone();
            let mut calls = 0;
            let refused = source.set_tile_code_checked(at, target, |observed| {
                calls += 1;
                assert_eq!(observed, tile_codes::SOLID_WALL);
                Err(DfmcpError::new(
                    ErrorCode::PreconditionsFailed,
                    "test transition is refused",
                ))
            });
            assert_eq!(calls, 1);
            assert!(refused.is_err_and(|error| error.code == ErrorCode::PreconditionsFailed));
            assert_eq!(source, before);
        }
        let revision = source.graph.chunks[&key].revision;
        assert!(
            source.set_tile_code_checked(at, tile_codes::FLOOR, |observed| {
                assert_eq!(observed, tile_codes::SOLID_WALL);
                Ok(())
            })?
        );
        assert_eq!(source.tile_code_at(at), Some(tile_codes::FLOOR));
        assert_eq!(source.graph.chunks[&key].revision, revision + 1);
        assert!(
            !source.set_tile_code_checked(at, tile_codes::FLOOR, |observed| {
                assert_eq!(observed, tile_codes::FLOOR);
                Ok(())
            })?
        );
        assert_eq!(source.graph.chunks[&key].revision, revision + 1);
        Ok(())
    }

    #[test]
    fn checked_tile_replacement_never_promotes_unknown_or_malformed_source() -> Result<()> {
        let at = MapCoord::new(3, 2, 5);
        let key = tile_slot(at).chunk;
        for variant in 0..14 {
            let mut source = snapshot();
            if variant < 12 {
                let chunk = source.graph.chunks.get_mut(&key).ok_or_else(|| {
                    DfmcpError::new(ErrorCode::InternalInvariantViolation, "test chunk missing")
                })?;
                corrupt_chunk(chunk, variant);
            } else if variant == 12 {
                source.set_tile_code(at, tile_codes::UNKNOWN)?;
            } else {
                source.graph.chunks.remove(&key);
            }
            let before = source.clone();
            let mut calls = 0;
            assert!(
                source
                    .set_tile_code_checked(at, tile_codes::FLOOR, |_| {
                        calls += 1;
                        Ok(())
                    })
                    .is_err_and(|error| error.code == ErrorCode::PreconditionsFailed)
            );
            assert_eq!(calls, 0, "variant {variant}");
            assert_eq!(source, before, "variant {variant}");
        }
        Ok(())
    }

    #[test]
    fn unobserved_terrain_is_unknown_and_cannot_be_mutated() -> Result<()> {
        let mut s = snapshot();
        assert_eq!(s.tile_code_at(MapCoord::new(0, 0, 6)), None);
        assert!(s.set_tile_code(MapCoord::new(0, 0, 6), 1).is_err());
        let spanning = area((-2, 0, 5), (1, 0, 5))?;
        assert_eq!(
            s.region_terrain_truth(spanning, tile_codes::SOLID_WALL),
            RegionTruth::Matches
        );
        let partly_unknown = area((0, 0, 5), (0, 0, 6))?;
        assert_eq!(
            s.region_terrain_truth(partly_unknown, tile_codes::SOLID_WALL),
            RegionTruth::Unknown
        );
        s.set_tile_code(MapCoord::new(0, 0, 5), tile_codes::FLOOR)?;
        // A known mismatch is definite even when other tiles are unknown.
        assert_eq!(
            s.region_terrain_truth(partly_unknown, tile_codes::SOLID_WALL),
            RegionTruth::Differs
        );
        Ok(())
    }

    #[test]
    fn oversized_regions_are_rejected() -> Result<()> {
        let huge = area((0, 0, 0), (256, 256, 0))?;
        assert!(validate_region(huge).is_err());
        assert_eq!(
            snapshot().region_terrain_truth(huge, 2),
            RegionTruth::Unknown
        );
        Ok(())
    }

    fn corrupt_chunk(chunk: &mut MapChunk, variant: usize) {
        match variant {
            0 => chunk.coord.x += 1,
            1 => chunk.coord.y -= 1,
            2 => chunk.coord.z += 1,
            3 => chunk.terrain_runs.insert(
                0,
                TerrainRun {
                    tile_code: tile_codes::FLOOR,
                    length: 0,
                },
            ),
            4 => chunk.terrain_runs.push(TerrainRun {
                tile_code: tile_codes::FLOOR,
                length: 0,
            }),
            5 => chunk.terrain_runs.extend(std::iter::repeat_n(
                TerrainRun {
                    tile_code: tile_codes::SOLID_WALL,
                    length: 0,
                },
                TERRAIN_CHUNK_TILES as usize,
            )),
            6 => chunk.width = 15,
            7 => chunk.height = 17,
            8 => chunk.terrain_runs[0].length = TERRAIN_CHUNK_TILES - 1,
            9 => chunk.terrain_runs[0].length = TERRAIN_CHUNK_TILES + 1,
            10 => chunk.terrain_runs.clear(),
            _ => chunk.terrain_runs.push(TerrainRun {
                tile_code: tile_codes::SOLID_WALL,
                length: u32::MAX,
            }),
        }
    }

    #[test]
    fn malformed_chunks_are_unknown_for_reads_proof_and_inert_for_writes() -> Result<()> {
        let at = MapCoord::new(3, 2, 5);
        let key = tile_slot(at).chunk;
        for variant in 0..12 {
            let mut source = snapshot();
            let chunk = source.graph.chunks.get_mut(&key).ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::InternalInvariantViolation,
                    "terrain fixture missing",
                )
            })?;
            corrupt_chunk(chunk, variant);
            source.refresh_hash();
            assert_eq!(source.tile_code_at(at), None, "variant {variant}");
            assert_eq!(
                source.region_terrain_truth(area((3, 2, 5), (3, 2, 5))?, tile_codes::SOLID_WALL),
                RegionTruth::Unknown,
                "variant {variant}",
            );
            let proof = crate::Predicate::RegionTerrain {
                area: area((3, 2, 5), (3, 2, 5))?,
                tile_code: tile_codes::SOLID_WALL,
            };
            if variant < 3 {
                // Scoped evidence already refuses aliased map keys at admission.
                assert!(crate::PredicateEvidence::laboratory(&source).is_err());
            } else {
                let evidence = crate::PredicateEvidence::laboratory(&source)?;
                assert_eq!(evidence.evaluate(&proof)?, crate::PredicateTruth::Unknown);
                assert_eq!(
                    evidence.evaluate(&crate::Predicate::Not(Box::new(proof)))?,
                    crate::PredicateTruth::Unknown,
                );
            }
            // Even a nominal no-op must not endorse malformed source geometry.
            for code in [tile_codes::SOLID_WALL, tile_codes::FLOOR] {
                let mut attempted = source.clone();
                let error = attempted.set_tile_code(at, code).err().ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::InternalInvariantViolation,
                        "malformed terrain unexpectedly accepted mutation",
                    )
                })?;
                assert_eq!(
                    error.code,
                    ErrorCode::PreconditionsFailed,
                    "variant {variant}"
                );
                assert_eq!(attempted, source, "variant {variant}");
            }
        }
        Ok(())
    }

    #[test]
    fn maximum_run_count_and_coordinate_extremes_remain_observed() -> Result<()> {
        for at in [
            MapCoord::new(i32::MIN, i32::MIN, i32::MIN),
            MapCoord::new(-1, -1, -1),
            MapCoord::new(0, 0, 0),
            MapCoord::new(15, 15, 5),
            MapCoord::new(i32::MAX, i32::MAX, i32::MAX),
        ] {
            let slot = tile_slot(at);
            let mut source = snapshot();
            let mut chunk = uniform_chunk(slot.chunk, tile_codes::SOLID_WALL);
            chunk.terrain_runs = (0..TERRAIN_CHUNK_TILES)
                .map(|offset| TerrainRun {
                    tile_code: if offset % 2 == 0 {
                        tile_codes::SOLID_WALL
                    } else {
                        tile_codes::FLOOR
                    },
                    length: 1,
                })
                .collect();
            let expected = chunk.terrain_runs[slot.offset as usize].tile_code;
            source.graph.chunks.insert(slot.chunk, chunk);
            assert_eq!(source.tile_code_at(at), Some(expected));
            assert!(!source.set_tile_code(at, expected)?);
            assert!(source.set_tile_code(at, tile_codes::STAIR)?);
            assert_eq!(source.tile_code_at(at), Some(tile_codes::STAIR));
            assert_eq!(source.graph.chunks[&slot.chunk].revision, 2);
        }
        Ok(())
    }

    #[test]
    fn unknown_tiles_remain_explicit_and_do_not_hide_known_region_mismatches() -> Result<()> {
        let mut source = snapshot();
        let unknown = MapCoord::new(0, 0, 5);
        source.set_tile_code(unknown, tile_codes::UNKNOWN)?;
        assert_eq!(source.tile_code_at(unknown), None);
        assert_eq!(
            crate::laboratory_access::laboratory_bounded_tile_code(&source, unknown),
            Some(tile_codes::UNKNOWN),
        );
        let before = source.clone();
        assert!(source.set_tile_code(unknown, tile_codes::FLOOR).is_err());
        assert_eq!(source, before);
        let span = area((0, 0, 5), (1, 0, 5))?;
        assert_eq!(
            source.region_terrain_truth(span, tile_codes::SOLID_WALL),
            RegionTruth::Unknown,
        );
        source.set_tile_code(MapCoord::new(1, 0, 5), tile_codes::FLOOR)?;
        assert_eq!(
            source.region_terrain_truth(span, tile_codes::SOLID_WALL),
            RegionTruth::Differs,
        );
        Ok(())
    }
}
