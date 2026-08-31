#![forbid(unsafe_code)]

//! Integration tests for WP-LEA-01 Spatial and Entity Lease Manager.

use std::error::Error;

use dfmcp_core::lease::{LeaseManager, cuboids_intersect};
use dfmcp_core::{GameTick, MapCoord, MapCuboid, SessionId};

#[test]
fn test_cuboids_intersect_precision() -> Result<(), Box<dyn Error>> {
    let a = MapCuboid::new(
        MapCoord { x: 0, y: 0, z: 100 },
        MapCoord {
            x: 10,
            y: 10,
            z: 100,
        },
    )?;
    let b = MapCuboid::new(
        MapCoord {
            x: 10,
            y: 10,
            z: 100,
        },
        MapCoord {
            x: 20,
            y: 20,
            z: 100,
        },
    )?;
    let c = MapCuboid::new(
        MapCoord {
            x: 11,
            y: 11,
            z: 100,
        },
        MapCoord {
            x: 20,
            y: 20,
            z: 100,
        },
    )?;

    // Touching on corner tile (10, 10, 100) -> intersect
    assert!(cuboids_intersect(&a, &b));
    // Disjoint
    assert!(!cuboids_intersect(&a, &c));

    Ok(())
}

#[test]
fn test_lease_ttl_cleanup_lifecycle() -> Result<(), Box<dyn Error>> {
    let mut manager = LeaseManager::new();
    let s1 = SessionId::new(1);
    let cuboid = MapCuboid::new(
        MapCoord { x: 0, y: 0, z: 100 },
        MapCoord { x: 5, y: 5, z: 100 },
    )?;

    let l1 = manager.acquire_spatial_lease(s1, cuboid, true, GameTick(100), 20)?;
    assert_eq!(manager.active_lease_count(), 1);

    // Renew lease
    manager.renew_lease(l1, GameTick(110), 30)?;

    // At tick 130, lease is still active (expires at 100 + 20 + 30 = 150)
    let pruned = manager.cleanup_expired_leases(GameTick(130));
    assert_eq!(pruned, 0);
    assert_eq!(manager.active_lease_count(), 1);

    // At tick 160, lease is expired
    let pruned2 = manager.cleanup_expired_leases(GameTick(160));
    assert_eq!(pruned2, 1);
    assert_eq!(manager.active_lease_count(), 0);

    Ok(())
}
