#![forbid(unsafe_code)]

//! Multi-Agent Role & Scoped Capability Delegation Subsystem.
//!
//! WP-LEA-03: Provides an in-process role registry and integrity-sealed delegation
//! records. These records are not cryptographic bearer tokens: authenticity comes
//! from exact membership in the manager's private registry.

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

/// Manager-issued capability delegation record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DelegationToken {
    pub token_id: u64,
    pub issuer_session: SessionId,
    pub delegatee_session: SessionId,
    pub grants: Vec<CapabilityGrant>,
    pub expires_at_tick: Option<GameTick>,
    pub integrity_digest: Digest32,
}

impl DelegationToken {
    /// Compute a deterministic integrity digest for a delegation record.
    ///
    /// This is deliberately not called a signature: it contains no secret or private key.
    #[must_use]
    pub fn compute_integrity_digest(
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
        hasher_bytes.extend_from_slice(&(grants.len() as u64).to_be_bytes());
        for grant in grants {
            put_len_prefixed(&mut hasher_bytes, grant.capability.as_str().as_bytes());
            put_len_prefixed(&mut hasher_bytes, grant.max_risk.as_str().as_bytes());
            match grant.scope.fortress_id {
                Some(fortress_id) => {
                    hasher_bytes.push(1);
                    hasher_bytes.extend_from_slice(&fortress_id.get().to_be_bytes());
                }
                None => hasher_bytes.push(0),
            }
            hasher_bytes.extend_from_slice(&(grant.scope.entity_ids.len() as u64).to_be_bytes());
            for entity_id in &grant.scope.entity_ids {
                hasher_bytes.extend_from_slice(&entity_id.get().to_be_bytes());
            }
            match grant.scope.map_area {
                Some(area) => {
                    hasher_bytes.push(1);
                    for coordinate in [area.min, area.max] {
                        hasher_bytes.extend_from_slice(&coordinate.x.to_be_bytes());
                        hasher_bytes.extend_from_slice(&coordinate.y.to_be_bytes());
                        hasher_bytes.extend_from_slice(&coordinate.z.to_be_bytes());
                    }
                }
                None => hasher_bytes.push(0),
            }
            put_optional_u64(&mut hasher_bytes, grant.expires_at_tick.map(|tick| tick.0));
            match grant.remaining_uses {
                Some(uses) => {
                    hasher_bytes.push(1);
                    hasher_bytes.extend_from_slice(&uses.to_be_bytes());
                }
                None => hasher_bytes.push(0),
            }
        }
        put_optional_u64(&mut hasher_bytes, expires.map(|tick| tick.0));
        Digest32::of_bytes(&hasher_bytes)
    }
}

fn put_len_prefixed(output: &mut Vec<u8>, value: &[u8]) {
    output.extend_from_slice(&(value.len() as u64).to_be_bytes());
    output.extend_from_slice(value);
}

fn put_optional_u64(output: &mut Vec<u8>, value: Option<u64>) {
    match value {
        Some(value) => {
            output.push(1);
            output.extend_from_slice(&value.to_be_bytes());
        }
        None => output.push(0),
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
        if grants.is_empty() {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "delegation must contain at least one capability grant",
            ));
        }
        let issuer_role = self.session_roles.get(&issuer).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::CapabilityDenied,
                "delegation issuer has no assigned role",
            )
        })?;
        let issuer_grants = issuer_role.default_grants();
        for requested in &grants {
            if !issuer_grants
                .iter()
                .any(|owned| grant_covers(owned, requested))
            {
                return Err(DfmcpError::new(
                    ErrorCode::CapabilityDenied,
                    format!(
                        "issuer role cannot delegate capability {} at the requested risk or scope",
                        requested.capability.as_str()
                    ),
                ));
            }
            if expires_at_tick
                .is_some_and(|outer| requested.expires_at_tick.is_some_and(|inner| inner > outer))
            {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidRequest,
                    "grant expiry cannot exceed delegation expiry",
                ));
            }
        }

        let token_id = self.next_token_seq;
        let next_token_seq = self.next_token_seq.checked_add(1).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "delegation token identifier space exhausted",
            )
        })?;

        let integrity_digest = DelegationToken::compute_integrity_digest(
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
            integrity_digest,
        };

        if self.delegations.insert(token_id, token.clone()).is_some() {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "delegation token identifier collision",
            ));
        }
        self.next_token_seq = next_token_seq;
        Ok(token)
    }

    /// Validate that a delegation record is manager-issued, unchanged, and unexpired.
    pub fn validate_token(&self, token: &DelegationToken, current_tick: GameTick) -> Result<()> {
        let stored = self.delegations.get(&token.token_id).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::CapabilityDenied,
                "delegation token was not issued by this role manager",
            )
        })?;
        if stored != token {
            return Err(DfmcpError::new(
                ErrorCode::CapabilityDenied,
                "delegation token does not match the manager-issued record",
            ));
        }

        let expected_digest = DelegationToken::compute_integrity_digest(
            token.token_id,
            token.issuer_session,
            token.delegatee_session,
            &token.grants,
            token.expires_at_tick,
        );

        if token.integrity_digest != expected_digest {
            return Err(DfmcpError::new(
                ErrorCode::CapabilityDenied,
                "delegation token integrity digest mismatch",
            ));
        }

        if let Some(exp) = token.expires_at_tick
            && current_tick > exp
        {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "delegation token has expired",
            ));
        }

        if token.grants.iter().any(|grant| {
            grant
                .expires_at_tick
                .is_some_and(|expiry| current_tick > expiry)
        }) {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "a delegated capability grant has expired",
            ));
        }

        Ok(())
    }
}

fn grant_covers(owned: &CapabilityGrant, requested: &CapabilityGrant) -> bool {
    let capability_covers =
        owned.capability == Capability::Admin || owned.capability == requested.capability;
    let expiry_covers = match (owned.expires_at_tick, requested.expires_at_tick) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(owned_expiry), Some(requested_expiry)) => requested_expiry <= owned_expiry,
    };
    let uses_cover = match (owned.remaining_uses, requested.remaining_uses) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(owned_uses), Some(requested_uses)) => requested_uses <= owned_uses,
    };
    capability_covers
        && requested.max_risk <= owned.max_risk
        && expiry_covers
        && uses_cover
        && scope_covers(&owned.scope, &requested.scope)
}

fn scope_covers(owned: &CapabilityScope, requested: &CapabilityScope) -> bool {
    let fortress_covers = match (owned.fortress_id, requested.fortress_id) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(owned_id), Some(requested_id)) => owned_id == requested_id,
    };
    let entities_cover = owned.entity_ids.is_empty()
        || (!requested.entity_ids.is_empty()
            && requested
                .entity_ids
                .iter()
                .all(|entity_id| owned.entity_ids.contains(entity_id)));
    let area_covers = match (owned.map_area, requested.map_area) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(owned_area), Some(requested_area)) => owned_area.contains_cuboid(requested_area),
    };
    fortress_covers && entities_cover && area_covers
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
