#![forbid(unsafe_code)]

//! Bootstrap an authenticated read-only MCP session from a caller-owned stream.
//!
//! Socket creation, async runtime ownership, deadlines, and cancellation remain
//! outside this module. The caller supplies a connected bounded `Read + Write`
//! stream. This module negotiates the native DFHack protocol, authenticates the
//! plugin, constructs the live adapter, derives only read capabilities, and
//! returns the adapter-neutral [`ReadSession`](crate::ReadSession).

use std::io::{Read, Write};

use dfmcp_adapter::{
    AdapterIdentity, BridgeCredentials, DfHackRpcClient, GameAdapter, LiveReadAdapter,
    LiveReadAdapterConfig,
};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, DfmcpError, Digest32, ErrorCode, GameTick,
    ObservationCursor, Result, RiskTier, SessionId, StateAnchor, WorkBudget,
};

use crate::{ReadSession, ReadSessionMetadata};

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveReadOpenReceipt {
    pub session_id: SessionId,
    pub fortress_id: dfmcp_core::FortressId,
    pub observation_epoch: u64,
    pub adapter_name: String,
    pub adapter_version: String,
    pub bridge_protocol_version: String,
    pub bridge_generation: u64,
    pub dwarf_fortress_version: String,
    pub dfhack_version: String,
    pub bridge_version: String,
    pub world_loaded: bool,
    pub fortress_mode: bool,
    pub granted_capabilities: Vec<Capability>,
    pub bootstrap_anchor: StateAnchor,
    pub bootstrap_anchor_is_authoritative: bool,
}

pub struct OpenedLiveReadSession<S> {
    pub session: ReadSession<LiveReadAdapter<DfHackRpcClient<S>>>,
    pub receipt: LiveReadOpenReceipt,
}

fn live_read_grants(
    identity: &AdapterIdentity,
    fortress_id: dfmcp_core::FortressId,
) -> Result<Vec<CapabilityGrant>> {
    let mut grants = Vec::new();
    for capability in &identity.capabilities {
        if !matches!(
            capability,
            Capability::Observe | Capability::Query | Capability::Doctor
        ) {
            return Err(error(
                ErrorCode::InternalInvariantViolation,
                format!(
                    "read-only live adapter advertised mutating capability {}",
                    capability.as_str()
                ),
            ));
        }
        grants.push(CapabilityGrant {
            capability: *capability,
            scope: CapabilityScope {
                fortress_id: Some(fortress_id),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        });
    }
    if !grants
        .iter()
        .any(|grant| grant.capability == Capability::Observe)
    {
        return Err(error(
            ErrorCode::InternalInvariantViolation,
            "live adapter does not advertise the required observe capability",
        ));
    }
    if !grants
        .iter()
        .any(|grant| grant.capability == Capability::Doctor)
    {
        return Err(error(
            ErrorCode::InternalInvariantViolation,
            "live adapter does not advertise the required doctor capability",
        ));
    }
    Ok(grants)
}

pub fn open_live_read_session<S: Read + Write>(
    stream: S,
    credentials: BridgeCredentials,
    config: LiveReadAdapterConfig,
    session_id: SessionId,
    budget: WorkBudget,
    client_name: &str,
    client_version: &str,
) -> Result<OpenedLiveReadSession<S>> {
    if session_id == SessionId::NIL {
        return Err(error(
            ErrorCode::InvalidRequest,
            "live read session identity must not be zero",
        ));
    }
    budget.validate()?;
    config.validate()?;

    let fortress_id = config.fortress_id;
    let observation_epoch = config.observation_epoch;
    let client = DfHackRpcClient::negotiate(
        stream,
        credentials,
        client_name,
        client_version,
    )?;
    let manifest = client.manifest().clone();
    let adapter = LiveReadAdapter::new(client, config)?;
    let identity = adapter.identity();
    let grants = live_read_grants(&identity, fortress_id)?;
    let granted_capabilities = grants
        .iter()
        .map(|grant| grant.capability)
        .collect::<Vec<_>>();
    let bootstrap_anchor = StateAnchor {
        fortress_id,
        cursor: ObservationCursor {
            epoch: observation_epoch,
            sequence: 0,
        },
        tick: GameTick::new(0),
        state_hash: Digest32::ZERO,
    };
    let session = ReadSession::new(
        ReadSessionMetadata {
            session_id,
            fortress_id,
            budget,
            grants,
        },
        adapter,
        bootstrap_anchor,
    )?;
    let receipt = LiveReadOpenReceipt {
        session_id,
        fortress_id,
        observation_epoch,
        adapter_name: identity.name,
        adapter_version: identity.adapter_version,
        bridge_protocol_version: identity.bridge_protocol_version,
        bridge_generation: manifest.bridge_generation,
        dwarf_fortress_version: manifest.df_version,
        dfhack_version: manifest.dfhack_version,
        bridge_version: manifest.bridge_version,
        world_loaded: manifest.world_loaded,
        fortress_mode: manifest.fortress_mode,
        granted_capabilities,
        bootstrap_anchor,
        bootstrap_anchor_is_authoritative: false,
    };
    Ok(OpenedLiveReadSession { session, receipt })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use dfmcp_adapter::CompatibilityLevel;
    use dfmcp_core::FortressId;

    use super::*;

    fn identity(capabilities: BTreeSet<Capability>) -> AdapterIdentity {
        AdapterIdentity {
            name: "test-live-adapter".to_owned(),
            adapter_version: "0.0.1".to_owned(),
            bridge_protocol_version: "dfmcp-bridge/1.0".to_owned(),
            dwarf_fortress_version: "0.51.11".to_owned(),
            dfhack_version: "0.51.11-r1".to_owned(),
            compatibility: CompatibilityLevel::DegradedReadOnly,
            capabilities,
            schema_digest: Digest32::of_bytes(b"schema"),
        }
    }

    #[test]
    fn live_grants_are_exactly_adapter_advertised_and_fortress_scoped() -> Result<()> {
        let grants = live_read_grants(
            &identity(BTreeSet::from([Capability::Observe, Capability::Doctor])),
            FortressId::new(7),
        )?;
        assert_eq!(grants.len(), 2);
        assert!(grants.iter().all(|grant| {
            grant.max_risk == RiskTier::ReadOnly
                && grant.scope.fortress_id == Some(FortressId::new(7))
        }));
        Ok(())
    }

    #[test]
    fn mutating_adapter_capability_fails_closed() {
        let result = live_read_grants(
            &identity(BTreeSet::from([
                Capability::Observe,
                Capability::Doctor,
                Capability::ControlClock,
            ])),
            FortressId::new(7),
        );
        assert!(result.is_err());
    }

    #[test]
    fn observe_and_doctor_are_required() {
        assert!(
            live_read_grants(
                &identity(BTreeSet::from([Capability::Doctor])),
                FortressId::new(7),
            )
            .is_err()
        );
        assert!(
            live_read_grants(
                &identity(BTreeSet::from([Capability::Observe])),
                FortressId::new(7),
            )
            .is_err()
        );
    }
}
