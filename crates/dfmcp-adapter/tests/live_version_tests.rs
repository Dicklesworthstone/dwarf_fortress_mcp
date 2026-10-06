#![forbid(unsafe_code)]

use dfmcp_adapter::live_version::LiveVersionTracker;
use dfmcp_core::{FortressId, Result};

#[test]
fn live_version_tracker_rejects_zero_lineage() {
    assert!(LiveVersionTracker::new(FortressId::NIL, 0).is_err());
}

#[test]
fn live_version_tracker_is_linked_into_the_adapter_test_graph() -> Result<()> {
    let tracker = LiveVersionTracker::new(FortressId::new(1), 0)?;
    assert_eq!(tracker.fortress_id(), FortressId::new(1));
    assert!(tracker.cursor().is_none());
    Ok(())
}
