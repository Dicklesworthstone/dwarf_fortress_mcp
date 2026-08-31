#![forbid(unsafe_code)]

//! Integration tests for the in-memory savegame archive contract prototype.

use dfmcp_core::{FortressId, GameTick, ObservationCursor, Result};
use dfmcp_world::franken_fs::{SavegameArchive, SavegameScrubber};
use dfmcp_world::{WorldGraph, WorldSnapshot};

#[test]
fn test_savegame_archive_roundtrip() -> Result<()> {
    let mut archive = SavegameArchive::new();
    let snapshot = WorldSnapshot::new(
        FortressId::new(99),
        GameTick(250),
        ObservationCursor {
            epoch: 1,
            sequence: 5,
        },
        true,
        WorldGraph::default(),
    );

    let blocks = archive.store_snapshot(&snapshot)?;
    assert!(!blocks.is_empty());

    let payload = archive.read_snapshot_payload(&snapshot.state_hash)?;
    assert!(!payload.is_empty());

    let scrubber = SavegameScrubber;
    let report = scrubber.scrub_archive(&archive);
    assert!(report.is_clean);

    Ok(())
}
