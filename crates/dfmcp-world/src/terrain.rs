//! Tile-level access to the canonical chunked terrain and region predicates.
//!
//! Canonical terrain is stored as run-length encoded 16x16x1 chunks keyed by
//! `ChunkCoord { x: x.div_euclid(16), y: y.div_euclid(16), z }`, matching the
//! spatial index. A tile whose chunk is absent, malformed, or not 16x16 is
//! unknown: region predicates never treat missing terrain as a match.

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

fn chunk_is_canonical(chunk: &MapChunk) -> bool {
    i32::from(chunk.width) == TERRAIN_CHUNK_EDGE
        && i32::from(chunk.height) == TERRAIN_CHUNK_EDGE
        && chunk.encoded_tile_count() == Some(TERRAIN_CHUNK_TILES)
}

fn code_in_chunk(chunk: &MapChunk, offset: u32) -> Option<u32> {
    if !chunk_is_canonical(chunk) {
        return None;
    }
    let mut start = 0u32;
    for run in &chunk.terrain_runs {
        let end = start.checked_add(run.length)?;
        if offset < end {
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

fn decode_runs(chunk: &MapChunk) -> Option<Vec<u32>> {
    if !chunk_is_canonical(chunk) {
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
            .and_then(|chunk| code_in_chunk(chunk, slot.offset))
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
        let slot = tile_slot(coord);
        let chunk = self.graph.chunks.get_mut(&slot.chunk).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::PreconditionsFailed,
                format!("terrain at {coord:?} is not observed"),
            )
        })?;
        let mut codes = decode_runs(chunk).ok_or_else(|| {
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
}
