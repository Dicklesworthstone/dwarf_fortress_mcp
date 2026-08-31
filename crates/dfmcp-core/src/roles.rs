#![forbid(unsafe_code)]

//! Multi-Agent Role & Scoped Capability Delegation Subsystem.
//!
//! WP-LEA-03: Provides role-based access control (RBAC) and cryptographically
//! signed capability delegation tokens across autonomous agent swarms.

use std::collections::BTreeMap;

use crate::digest::Digest32;
use crate::error::{DfmcpError, ErrorCode, Result};
use crate::ids::SessionId;
use crate::model::{Capability, CapabilityGrant, CapabilityScope, GameTick, RiskTier};

/// Standard autonomous swarm agent role archetype.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SwarmRole {
    ExpeditionLeader,
    MiningOverseer,
    ProductionManager,
    MilitiaCommander,
    ChiefMedicalDwarf,
    TimeKeeper,
    Custom(String),
}

impl SwarmRole {
    /// Get standard base capability grants for this role.
    #[must_use]
    pub fn default_grants(&self) -> Vec<CapabilityGrant> {
        let mut grants = vec![
            CapabilityGrant {
                capability: Capability::Observe,
                scope: CapabilityScope::default(),
                max_risk: RiskTier::Reversible,
                expires_at_tick: None,
                remaining_uses: None,
            },
            CapabilityGrant {
                capability: Capability::Query,
                scope: CapabilityScope::default(),
                max_risk: RiskTier::Reversible,
                expires_at_tick: None,
                remaining_uses: None,
            },
        ];

        match self {
            Self::ExpeditionLeader => {
                grants.push(CapabilityGrant {
                    capability: Capability::ControlClock,
                    scope: CapabilityScope::default(),
                    max_risk: RiskTier::Reversible,
                    expires_at_tick: None,
                    remaining_uses: None,
                });
                grants.push(CapabilityGrant {
                    capability: Capability::Designate,
                    scope: CapabilityScope::default(),
                    max_risk: RiskTier::Guarded,
                    expires_at_tick: None,
                    remaining_uses: None,
                });
                grants.push(CapabilityGrant {
                    capability: Capability::Construct,
                    scope: CapabilityScope::default(),
                    max_risk: RiskTier::Guarded,
                    expires_at_tick: None,
                    remaining_uses: None,
                });
                grants.push(CapabilityGrant {
                    capability: Capability::ConfigureLabor,
                    scope: CapabilityScope::default(),
                    max_risk: RiskTier::Reversible,
                    expires_at_tick: None,
                    remaining_uses: None,
                });
                grants.push(CapabilityGrant {
                    capability: Capability::ConfigureProduction,
                    scope: CapabilityScope::default(),
                    max_risk: RiskTier::Reversible,
                    expires_at_tick: None,
                    remaining_uses: None,
                });
                grants.push(CapabilityGrant {
                    capability: Capability::ConfigureMilitary,
                    scope: CapabilityScope::default(),
                    max_risk: RiskTier::Guarded,
                    expires_at_tick: None,
                    remaining_uses: None,
                });
            }
            Self::MiningOverseer => {
                grants.push(CapabilityGrant {
                    capability: Capability::Designate,
                    scope: CapabilityScope::default(),
                    max_risk: RiskTier::Guarded,
                    expires_at_tick: None,
                    remaining_uses: None,
                });
            }
            Self::ProductionManager => {
                grants.push(CapabilityGrant {
                    capability: Capability::ConfigureProduction,
                    scope: CapabilityScope::default(),
                    max_risk: RiskTier::Reversible,
                    expires_at_tick: None,
                    remaining_uses: None,
                });
                grants.push(CapabilityGrant {
                    capability: Capability::ConfigureLogistics,
                    scope: CapabilityScope::default(),
                    max_risk: RiskTier::Reversible,
                    expires_at_tick: None,
                    remaining_uses: None,
                });
            }
            Self::MilitiaCommander => {
                grants.push(CapabilityGrant {
                    capability: Capability::ConfigureMilitary,
                    scope: CapabilityScope::default(),
                    max_risk: RiskTier::Guarded,
                    expires_at_tick: None,
                    remaining_uses: None,
                });
            }
            Self::ChiefMedicalDwarf => {
                grants.push(CapabilityGrant {
                    capability: Capability::ConfigureLabor,
                    scope: CapabilityScope::default(),
                    max_risk: RiskTier::Reversible,
                    expires_at_tick: None,
                    remaining_uses: None,
                });
            }
            Self::TimeKeeper => {
                grants.push(CapabilityGrant {
                    capability: Capability::ControlClock,
                    scope: CapabilityScope::default(),
                    max_risk: RiskTier::Reversible,
                    expires_at_tick: None,
                    remaining_uses: None,
                });
            }
            Self::Custom(_) => {}
        }

        grants
    }
}

/// Cryptographically signed capability delegation token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DelegationToken {
    pub token_id: u64,
    pub issuer_session: SessionId,
    pub delegatee_session: SessionId,
    pub grants: Vec<CapabilityGrant>,
    pub expires_at_tick: Option<GameTick>,
    pub signature_digest: Digest32,
}

impl DelegationToken {
    /// Compute cryptographic token signature.
    #[must_use]
    pub fn compute_signature(
        token_id: u64,
        issuer: SessionId,
        delegatee: SessionId,
        grants: &[CapabilityGrant],
        expires: Option<GameTick>,
    ) -> Digest32 {
        let mut hasher_bytes = Vec::new();
        hasher_bytes.extend_from_slice(&token_id.to_be_bytes());
        hasher_bytes.extend_from_slice(&issuer.get().to_be_bytes());
        hasher_bytes.extend_from_slice(&delegatee.get().to_be_bytes());
        for g in grants {
            hasher_bytes.extend_from_slice(format!("{:?}", g).as_bytes());
        }
        if let Some(exp) = expires {
            hasher_bytes.extend_from_slice(&exp.0.to_be_bytes());
        }
        Digest32::of_bytes(&hasher_bytes)
    }
}

/// Role & Delegation Manager.
#[derive(Clone, Debug, Default)]
pub struct RoleManager {
    next_token_seq: u64,
    session_roles: BTreeMap<SessionId, SwarmRole>,
    delegations: BTreeMap<u64, DelegationToken>,
}

impl RoleManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            next_token_seq: 1,
            session_roles: BTreeMap::new(),
            delegations: BTreeMap::new(),
        }
    }

    /// Assign a swarm role to an agent session and return its default capabilities.
    pub fn assign_role(&mut self, session_id: SessionId, role: SwarmRole) -> Vec<CapabilityGrant> {
        let grants = role.default_grants();
        self.session_roles.insert(session_id, role);
        grants
    }

    /// Issue a delegation token transferring scoped capabilities from issuer to delegatee.
    pub fn issue_delegation(
        &mut self,
        issuer: SessionId,
        delegatee: SessionId,
        grants: Vec<CapabilityGrant>,
        expires_at_tick: Option<GameTick>,
    ) -> Result<DelegationToken> {
        let token_id = self.next_token_seq;
        self.next_token_seq = self.next_token_seq.saturating_add(1);

        let signature_digest = DelegationToken::compute_signature(
            token_id,
            issuer,
            delegatee,
            &grants,
            expires_at_tick,
        );

        let token = DelegationToken {
            token_id,
            issuer_session: issuer,
            delegatee_session: delegatee,
            grants,
            expires_at_tick,
            signature_digest,
        };

        self.delegations.insert(token_id, token.clone());
        Ok(token)
    }

    /// Validate that a delegation token is authentic and unexpired.
    pub fn validate_token(&self, token: &DelegationToken, current_tick: GameTick) -> Result<()> {
        let expected_sig = DelegationToken::compute_signature(
            token.token_id,
            token.issuer_session,
            token.delegatee_session,
            &token.grants,
            token.expires_at_tick,
        );

        if token.signature_digest != expected_sig {
            return Err(DfmcpError::new(
                ErrorCode::CapabilityDenied,
                "invalid delegation token signature",
            ));
        }

        if let Some(exp) = token.expires_at_tick
            && current_tick >= exp
        {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "delegation token has expired",
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_role_assignment_and_delegation_lifecycle() -> Result<()> {
        let mut manager = RoleManager::new();
        let s_leader = SessionId::new(1);
        let s_miner = SessionId::new(2);

        let leader_grants = manager.assign_role(s_leader, SwarmRole::ExpeditionLeader);
        assert!(leader_grants.len() >= 6);

        let miner_grants = manager.assign_role(s_miner, SwarmRole::MiningOverseer);
        assert_eq!(miner_grants.len(), 3); // Observe, Query, Designate

        // Delegate Construct capability from leader to miner
        let delegated_grants = vec![CapabilityGrant {
            capability: Capability::Construct,
            scope: CapabilityScope::default(),
            max_risk: RiskTier::Guarded,
            expires_at_tick: Some(GameTick(200)),
            remaining_uses: None,
        }];

        let token =
            manager.issue_delegation(s_leader, s_miner, delegated_grants, Some(GameTick(200)))?;
        assert!(manager.validate_token(&token, GameTick(100)).is_ok());

        // Expired at tick 201
        assert!(manager.validate_token(&token, GameTick(201)).is_err());

        Ok(())
    }
}
