#![forbid(unsafe_code)]

//! Integration tests for WP-LEA-02 Multi-Agent Clock & Game-Speed Governance.

use dfmcp_core::clock::{ClockGovernor, ClockPolicy};
use dfmcp_core::{Result, SessionId};

#[test]
fn test_majority_unpause_governance() -> Result<()> {
    let mut governor = ClockGovernor::new(ClockPolicy::MajorityUnpause);
    let s1 = SessionId::new(1);
    let s2 = SessionId::new(2);
    let s3 = SessionId::new(3);

    governor.register_session(s1, 100);
    governor.register_session(s2, 100);
    governor.register_session(s3, 100);

    // 1 vote out of 3 -> still paused
    governor.vote_unpause(s1);
    assert!(!governor.is_unpaused());

    // 2 votes out of 3 -> majority reached -> unpaused!
    governor.vote_unpause(s2);
    assert!(governor.is_unpaused());

    // Consume tick budget
    governor.consume_tick_budget(s1, 50)?;
    assert!(governor.consume_tick_budget(s1, 60).is_err()); // Exceeds budget

    Ok(())
}
