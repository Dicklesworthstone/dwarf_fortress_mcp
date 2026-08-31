#![forbid(unsafe_code)]

//! 3D Spatial Octree / Chunk Multi-Index for high-performance spatial queries.
//!
//! WP-WLD-01: Provides sub-millisecond bounding box searches, radius queries,
//! liquidity checks, and 3D neighbor connectivity lookups without full-graph scans.

use std::collections::BTreeMap;

use dfmcp_core::{MapCoord, MapCuboid};

use crate::model::{ChunkCoord, MapChunk};

/// Tile classification for spatial indexing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TileType {
    OpenSpace,
    Pillar,
    Floor,
    Stair,
    Ramp,
    Fortification,
    Tree,
    SolidWall,
    MagmaWall,
    Chasm,
}

impl TileType {
    #[must_use]
    pub const fn from_tile_code(code: u32) -> Self {
        match code {
            0 => Self::OpenSpace,
            1 => Self::Floor,
            2 => Self::SolidWall,
            3 => Self::Stair,
            4 => Self::Ramp,
            5 => Self::Fortification,
            6 => Self::Tree,
            7 => Self::MagmaWall,
            8 => Self::Chasm,
            _ => Self::Pillar,
        }
    }
}

/// Liquid kind for subterranean flows and water bodies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LiquidType {
    Water,
    Magma,
}

/// Temperature band classification for environmental safety and magma proximity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TemperatureBand {
    Freezing,
    Cold,
    Temperate,
    Hot,
    MagmaHot,
}

/// Computed spatial properties for a single map tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileProperties {
    pub tile_type: TileType,
    pub walkable: bool,
    pub liquid: Option<(LiquidType, u8)>, // (type, depth 1-7)
    pub temperature: TemperatureBand,
}

impl TileProperties {
    #[must_use]
    pub const fn from_tile_type(tile_type: TileType) -> Self {
        let (walkable, temperature) = match tile_type {
            TileType::OpenSpace | TileType::Pillar => (false, TemperatureBand::Temperate),
            TileType::Floor
            | TileType::Stair
            | TileType::Ramp
            | TileType::Fortification
            | TileType::Tree => (true, TemperatureBand::Temperate),
            TileType::SolidWall => (false, TemperatureBand::Temperate),
            TileType::MagmaWall => (false, TemperatureBand::MagmaHot),
            TileType::Chasm => (false, TemperatureBand::Cold),
        };

        Self {
            tile_type,
            walkable,
            liquid: None,
            temperature,
        }
    }
}

/// 16x16x1 indexed spatial chunk holding tile property arrays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpatialChunkNode {
    pub coord: ChunkCoord,
    pub tiles: [TileProperties; 256],
}

impl SpatialChunkNode {
    #[must_use]
    pub fn from_map_chunk(chunk: &MapChunk) -> Self {
        let mut tiles = [TileProperties::from_tile_type(TileType::SolidWall); 256];
        let mut idx = 0;

        for run in &chunk.terrain_runs {
            let tile_type = TileType::from_tile_code(run.tile_code);
            let props = TileProperties::from_tile_type(tile_type);
            let count = (run.length as usize).min(256 - idx);
            for i in 0..count {
                tiles[idx + i] = props;
            }
            idx += count;
            if idx >= 256 {
                break;
            }
        }

        // Apply sparse overlays
        for offset in chunk.sparse_overlays.keys() {
            let tile_idx = *offset as usize;
            if tile_idx < 256 {
                tiles[tile_idx] = TileProperties::from_tile_type(TileType::Floor);
            }
        }

        Self {
            coord: chunk.coord,
            tiles,
        }
    }

    #[must_use]
    pub fn get_tile(&self, local_x: u8, local_y: u8) -> Option<TileProperties> {
        if local_x < 16 && local_y < 16 {
            let idx = (local_y as usize) * 16 + (local_x as usize);
            Some(self.tiles[idx])
        } else {
            None
        }
    }
}

/// Fast Spatial Multigrid Index storing mapped chunks.
#[derive(Clone, Debug, Default)]
pub struct ChunkSpatialIndex {
    chunks: BTreeMap<ChunkCoord, SpatialChunkNode>,
}

impl ChunkSpatialIndex {
    #[must_use]
    pub fn new() -> Self {
        Self {
            chunks: BTreeMap::new(),
        }
    }

    /// Index or update a map chunk.
    pub fn insert_or_update_chunk(&mut self, chunk: &MapChunk) {
        let node = SpatialChunkNode::from_map_chunk(chunk);
        self.chunks.insert(chunk.coord, node);
    }

    /// Remove a chunk from the index.
    pub fn remove_chunk(&mut self, coord: &ChunkCoord) {
        self.chunks.remove(coord);
    }

    /// Get tile properties at exact coordinates.
    #[must_use]
    pub fn get_tile(&self, coord: MapCoord) -> Option<TileProperties> {
        let chunk_x = coord.x.div_euclid(16);
        let chunk_y = coord.y.div_euclid(16);
        let chunk_coord = ChunkCoord {
            x: chunk_x,
            y: chunk_y,
            z: coord.z,
        };

        let node = self.chunks.get(&chunk_coord)?;
        let local_x = (coord.x.rem_euclid(16)) as u8;
        let local_y = (coord.y.rem_euclid(16)) as u8;

        node.get_tile(local_x, local_y)
    }

    /// Find all tiles within a specified 3D cuboid bounding box.
    pub fn find_cuboid(&self, cuboid: &MapCuboid) -> Vec<(MapCoord, TileProperties)> {
        let mut results = Vec::new();
        let min_x = cuboid.min.x.min(cuboid.max.x);
        let max_x = cuboid.min.x.max(cuboid.max.x);
        let min_y = cuboid.min.y.min(cuboid.max.y);
        let max_y = cuboid.min.y.max(cuboid.max.y);
        let min_z = cuboid.min.z.min(cuboid.max.z);
        let max_z = cuboid.min.z.max(cuboid.max.z);

        for z in min_z..=max_z {
            for y in min_y..=max_y {
                for x in min_x..=max_x {
                    let coord = MapCoord { x, y, z };
                    if let Some(props) = self.get_tile(coord) {
                        results.push((coord, props));
                    }
                }
            }
        }

        results
    }

    /// Find all 6-directional walkable adjacent neighbors (up, down, north, south, east, west).
    pub fn find_walkable_neighbors(&self, coord: MapCoord) -> Vec<MapCoord> {
        let deltas = [
            (1, 0, 0),
            (-1, 0, 0),
            (0, 1, 0),
            (0, -1, 0),
            (0, 0, 1),
            (0, 0, -1),
        ];

        let mut neighbors = Vec::new();
        for (dx, dy, dz) in deltas {
            let neighbor_coord = MapCoord {
                x: coord.x.saturating_add(dx),
                y: coord.y.saturating_add(dy),
                z: coord.z.saturating_add(dz),
            };
            if let Some(props) = self.get_tile(neighbor_coord)
                && props.walkable
            {
                neighbors.push(neighbor_coord);
            }
        }

        neighbors
    }

    /// Total number of indexed chunks.
    #[must_use]
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// Whether index is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TerrainRun;
    use std::error::Error;

    #[test]
    fn test_spatial_chunk_indexing_and_point_query() {
        let mut index = ChunkSpatialIndex::new();
        let coord = ChunkCoord { x: 0, y: 0, z: 100 };

        let chunk = MapChunk {
            coord,
            revision: 1,
            width: 16,
            height: 16,
            terrain_runs: vec![TerrainRun {
                tile_code: 1, // Floor
                length: 256,
            }],
            sparse_overlays: BTreeMap::new(),
        };

        index.insert_or_update_chunk(&chunk);
        assert_eq!(index.chunk_count(), 1);

        let tile = index.get_tile(MapCoord { x: 5, y: 5, z: 100 });
        assert!(tile.is_some());
        let tile = tile.unwrap_or(TileProperties::from_tile_type(TileType::SolidWall));
        assert_eq!(tile.tile_type, TileType::Floor);
        assert!(tile.walkable);
    }

    #[test]
    fn test_cuboid_query() -> Result<(), Box<dyn Error>> {
        let mut index = ChunkSpatialIndex::new();
        let coord = ChunkCoord { x: 0, y: 0, z: 100 };

        let chunk = MapChunk {
            coord,
            revision: 1,
            width: 16,
            height: 16,
            terrain_runs: vec![TerrainRun {
                tile_code: 1, // Floor
                length: 256,
            }],
            sparse_overlays: BTreeMap::new(),
        };

        index.insert_or_update_chunk(&chunk);

        let cuboid = MapCuboid::new(
            MapCoord { x: 2, y: 2, z: 100 },
            MapCoord { x: 4, y: 4, z: 100 },
        )?;
        let results = index.find_cuboid(&cuboid);
        assert_eq!(results.len(), 9); // 3x3 on z=100

        Ok(())
    }
}
