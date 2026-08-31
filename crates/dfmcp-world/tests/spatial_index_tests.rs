#![forbid(unsafe_code)]

//! Integration tests for WP-WLD-01 3D Spatial Octree / Chunk Multi-Index.

use std::collections::BTreeMap;
use std::error::Error;

use dfmcp_core::{MapCoord, MapCuboid};
use dfmcp_world::spatial_index::ChunkSpatialIndex;
use dfmcp_world::{ChunkCoord, MapChunk, TerrainRun};

#[test]
fn test_spatial_index_multi_level_chunks() -> Result<(), Box<dyn Error>> {
    let mut index = ChunkSpatialIndex::new();

    // Create 3 vertical z-level chunks
    for z in 90..=92 {
        let tile_code = if z == 91 {
            1 // Floor
        } else if z == 92 {
            0 // OpenSpace
        } else {
            2 // SolidWall
        };

        let chunk = MapChunk {
            coord: ChunkCoord { x: 0, y: 0, z },
            revision: 1,
            width: 16,
            height: 16,
            terrain_runs: vec![TerrainRun {
                length: 256,
                tile_code,
            }],
            sparse_overlays: BTreeMap::new(),
        };
        index.insert_or_update_chunk(&chunk);
    }

    assert_eq!(index.chunk_count(), 3);

    let cuboid = MapCuboid::new(
        MapCoord { x: 0, y: 0, z: 90 },
        MapCoord { x: 1, y: 1, z: 92 },
    )?;
    let tiles = index.find_cuboid(&cuboid);
    // 2 * 2 * 3 = 12 tiles
    assert_eq!(tiles.len(), 12);

    Ok(())
}

#[test]
fn test_spatial_index_walkable_connectivity() {
    let mut index = ChunkSpatialIndex::new();
    let chunk = MapChunk {
        coord: ChunkCoord { x: 0, y: 0, z: 100 },
        revision: 1,
        width: 16,
        height: 16,
        terrain_runs: vec![TerrainRun {
            length: 256,
            tile_code: 1, // Floor
        }],
        sparse_overlays: BTreeMap::new(),
    };
    index.insert_or_update_chunk(&chunk);

    let neighbors = index.find_walkable_neighbors(MapCoord { x: 8, y: 8, z: 100 });
    assert!(!neighbors.is_empty());
    assert!(neighbors.contains(&MapCoord { x: 9, y: 8, z: 100 }));
    assert!(neighbors.contains(&MapCoord { x: 7, y: 8, z: 100 }));
    assert!(neighbors.contains(&MapCoord { x: 8, y: 9, z: 100 }));
    assert!(neighbors.contains(&MapCoord { x: 8, y: 7, z: 100 }));
}
