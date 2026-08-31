#![forbid(unsafe_code)]

//! Integration tests for WP-LEA-03 Multi-Agent Role & Capability Delegation.

use dfmcp_core::roles::{RoleManager, SwarmRole};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, GameTick, Result, RiskTier, SessionId,
};

#[test]
fn test_all_swarm_roles_default_grants() {
    let roles = [
        SwarmRole::ExpeditionLeader,
        SwarmRole::MiningOverseer,
        SwarmRole::ProductionManager,
        SwarmRole::MilitiaCommander,
        SwarmRole::ChiefMedicalDwarf,
        SwarmRole::TimeKeeper,
        SwarmRole::Custom("Architect".to_owned()),
    ];

    for role in roles {
        let grants = role.default_grants();
        assert!(!grants.is_empty());
        // All roles have at least Observe and Query
        let caps: Vec<Capability> = grants.iter().map(|g| g.capability).collect();
        assert!(caps.contains(&Capability::Observe));
        assert!(caps.contains(&Capability::Query));
    }
}

#[test]
fn test_delegation_tamper_detection() -> Result<()> {
    let mut manager = RoleManager::new();
    let s1 = SessionId::new(1);
    let s2 = SessionId::new(2);

    let grants = vec![CapabilityGrant {
        capability: Capability::Designate,
        scope: CapabilityScope::default(),
        max_risk: RiskTier::Guarded,
        expires_at_tick: None,
        remaining_uses: None,
    }];

    let mut token = manager.issue_delegation(s1, s2, grants, None)?;
    assert!(manager.validate_token(&token, GameTick(100)).is_ok());

    // Tamper with delegatee
    token.delegatee_session = SessionId::new(3);
    assert!(manager.validate_token(&token, GameTick(100)).is_err());

    Ok(())
}
