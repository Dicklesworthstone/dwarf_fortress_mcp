#![forbid(unsafe_code)]

//! Comprehensive edge-case and boundary property tests for dfmcp-core.

use dfmcp_core::clock::{ClockGovernor, ClockPolicy};
use dfmcp_core::lease::cuboids_intersect;
use dfmcp_core::{MapCoord, MapCuboid, Result, SessionId};

#[test]
fn test_cuboid_intersection_boundary_conditions() -> Result<()> {
    // 1. Point intersection at boundary
    let c1 = MapCuboid::new(
        MapCoord { x: 0, y: 0, z: 0 },
        MapCoord {
            x: 10,
            y: 10,
            z: 10,
        },
    )?;
    let c2 = MapCuboid::new(
        MapCoord {
            x: 10,
            y: 10,
            z: 10,
        },
        MapCoord {
            x: 20,
            y: 20,
            z: 20,
        },
    )?;
    assert!(cuboids_intersect(&c1, &c2));

    // 2. Adjacent non-overlapping cuboid
    let c3 = MapCuboid::new(
        MapCoord { x: 11, y: 0, z: 0 },
        MapCoord {
            x: 20,
            y: 10,
            z: 10,
        },
    )?;
    assert!(!cuboids_intersect(&c1, &c3));

    // 3. Completely contained cuboid
    let c4 = MapCuboid::new(MapCoord { x: 2, y: 2, z: 2 }, MapCoord { x: 5, y: 5, z: 5 })?;
    assert!(cuboids_intersect(&c1, &c4));
    assert!(cuboids_intersect(&c4, &c1));

    // 4. Negative coordinates
    let c_neg1 = MapCuboid::new(
        MapCoord {
            x: -10,
            y: -10,
            z: -10,
        },
        MapCoord {
            x: -1,
            y: -1,
            z: -1,
        },
    )?;
    let c_neg2 = MapCuboid::new(
        MapCoord {
            x: -5,
            y: -5,
            z: -5,
        },
        MapCoord { x: 5, y: 5, z: 5 },
    )?;
    assert!(cuboids_intersect(&c_neg1, &c_neg2));

    Ok(())
}

#[test]
fn test_clock_governor_high_churn_consensus() -> Result<()> {
    let mut governor = ClockGovernor::new(ClockPolicy::MajorityUnpause);

    // Register 10 sessions
    let sessions: Vec<SessionId> = (1..=10).map(SessionId::new).collect();
    for &s in &sessions {
        governor.register_session(s, 500);
    }

    // First 5 vote unpause (5/10 = 50% -> not majority, still paused)
    for &s in &sessions[0..5] {
        governor.vote_unpause(s);
    }
    assert!(!governor.is_unpaused());

    // 6th session votes unpause (6/10 = 60% > 50% -> majority reached!)
    governor.vote_unpause(sessions[5]);
    assert!(governor.is_unpaused());

    // 1st session requests emergency pause -> instantly pauses despite 6 unpause votes
    governor.request_emergency_pause(sessions[0]);
    assert!(!governor.is_unpaused());

    // 1st session releases emergency pause -> unpaused resumed
    governor.release_emergency_pause(sessions[0]);
    assert!(governor.is_unpaused());

    // Disconnect 4 sessions -> 6 remaining sessions, of which 5 voted unpause (5/6 > 50% -> unpaused)
    for &s in &sessions[6..10] {
        governor.unregister_session(s);
    }
    assert!(governor.is_unpaused());

    Ok(())
}
