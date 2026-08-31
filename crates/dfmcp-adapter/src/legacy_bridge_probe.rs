#![forbid(unsafe_code)]

//! Legacy opaque-framing transport probe.
//!
//! This module is retained only as a bounded transceiver laboratory. It is not
//! the supported DFHack wire, performs no native DFHack handshake, decodes no
//! bridge payload, establishes no canonical anchor, and exercises no game
//! effect. New integrations must use [`crate::LiveReadAdapter`] over the
//! authenticated [`crate::DfHackRpcClient`].

use std::collections::BTreeSet;
use std::io::{Read, Write};

use dfmcp_core::{
    ActionId, Capability, CheckpointId, DfmcpError, Digest32, ErrorCode, OperationContext, Result,
};
use dfmcp_intent::PreparedPlan;

use crate::ipc::IpcMessageType;
use crate::transceiver::{IpcTransceiver, TransceiverConfig};
use crate::{
    ActionReceipt, AdapterHealth, AdapterIdentity, CancelMode, CancelReceipt, CheckpointReceipt,
    CommitReceipt, CompatibilityLevel, GameAdapter, HealthStatus, ObservationFrame,
    ObservationRequest, PrepareReceipt, QueryRequest, QueryResponse, RestoreReceipt,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyBridgeProbeConfig {
    pub endpoint: String,
    pub transceiver_config: TransceiverConfig,
    pub adapter_name: String,
    /// Operator expectation only; never an authenticated observation.
    pub target_df_version: String,
    /// Operator expectation only; never an authenticated observation.
    pub target_dfhack_version: String,
}

impl Default for LegacyBridgeProbeConfig {
    fn default() -> Self {
        Self {
            endpoint: "/tmp/dfhack-mcp-legacy-probe.sock".to_owned(),
            transceiver_config: TransceiverConfig::default(),
            adapter_name: "dfhack-opaque-framing-laboratory".to_owned(),
            target_df_version: "unverified".to_owned(),
            target_dfhack_version: "unverified".to_owned(),
        }
    }
}

pub struct LegacyBridgeProbeAdapter<S> {
    transceiver: IpcTransceiver<S>,
    config: LegacyBridgeProbeConfig,
    identity: AdapterIdentity,
}

impl<S: Read + Write> LegacyBridgeProbeAdapter<S> {
    #[must_use]
    pub fn new(stream: S, config: LegacyBridgeProbeConfig) -> Self {
        let identity = AdapterIdentity {
            name: config.adapter_name.clone(),
            adapter_version: env!("CARGO_PKG_VERSION").to_owned(),
            bridge_protocol_version: "legacy-opaque-framing-laboratory".to_owned(),
            dwarf_fortress_version: "unverified".to_owned(),
            dfhack_version: "unverified".to_owned(),
            compatibility: CompatibilityLevel::Unknown,
            capabilities: BTreeSet::from([Capability::Doctor]),
            schema_digest: Digest32::ZERO,
        };
        let transceiver = IpcTransceiver::new(stream, config.transceiver_config.clone());
        Self {
            transceiver,
            config,
            identity,
        }
    }

    #[must_use]
    pub const fn config(&self) -> &LegacyBridgeProbeConfig {
        &self.config
    }

    #[must_use]
    pub const fn transceiver(&self) -> &IpcTransceiver<S> {
        &self.transceiver
    }

    pub fn transceiver_mut(&mut self) -> &mut IpcTransceiver<S> {
        &mut self.transceiver
    }
}

fn unavailable<T>(operation: &str) -> Result<T> {
    Err(DfmcpError::new(
        ErrorCode::CompatibilityUnknown,
        format!(
            "legacy opaque framing laboratory cannot perform DFHack {operation}; use the authenticated live adapter"
        ),
    ))
}

impl<S: Read + Write> GameAdapter for LegacyBridgeProbeAdapter<S> {
    fn identity(&self) -> AdapterIdentity {
        self.identity.clone()
    }

    fn health(&mut self, context: &OperationContext) -> Result<AdapterHealth> {
        context.authorize(
            Capability::Doctor,
            dfmcp_core::RiskTier::ReadOnly,
            &[],
            None,
        )?;
        let result = self.transceiver.request(
            IpcMessageType::HealthRequest,
            Vec::new(),
            IpcMessageType::HealthResponse,
            context,
        );
        let (status, warning) = match result {
            Ok(_) => (
                HealthStatus::Degraded,
                "opaque framing laboratory responded; no DFHack identity, compatibility, fortress state, or canonical observation was established",
            ),
            Err(_) => (
                HealthStatus::Unavailable,
                "opaque framing laboratory did not complete its bounded liveness probe",
            ),
        };
        Ok(AdapterHealth {
            status,
            identity: self.identity.clone(),
            fortress_loaded: false,
            paused: None,
            current_anchor: None,
            warnings: vec![warning.to_owned()],
        })
    }

    fn observe(
        &mut self,
        _request: &ObservationRequest,
        _context: &OperationContext,
    ) -> Result<ObservationFrame> {
        unavailable("observation")
    }

    fn query(
        &mut self,
        _request: &QueryRequest,
        _context: &OperationContext,
    ) -> Result<QueryResponse> {
        unavailable("query")
    }

    fn prepare(
        &mut self,
        _plan: &PreparedPlan,
        _context: &OperationContext,
    ) -> Result<PrepareReceipt> {
        unavailable("prepare")
    }

    fn commit(
        &mut self,
        _plan: &PreparedPlan,
        _prepared: &PrepareReceipt,
        _context: &OperationContext,
    ) -> Result<CommitReceipt> {
        unavailable("commit")
    }

    fn poll_action(
        &mut self,
        _action_id: ActionId,
        _context: &OperationContext,
    ) -> Result<ActionReceipt> {
        unavailable("action polling")
    }

    fn request_cancel(
        &mut self,
        _action_id: ActionId,
        _mode: CancelMode,
        _context: &OperationContext,
    ) -> Result<CancelReceipt> {
        unavailable("cancellation")
    }

    fn finalize_cancel(
        &mut self,
        _action_id: ActionId,
        _context: &OperationContext,
    ) -> Result<CancelReceipt> {
        unavailable("cancellation finalization")
    }

    fn checkpoint(
        &mut self,
        _label: &str,
        _context: &OperationContext,
    ) -> Result<CheckpointReceipt> {
        unavailable("checkpoint")
    }

    fn restore(
        &mut self,
        _checkpoint_id: CheckpointId,
        _context: &OperationContext,
    ) -> Result<RestoreReceipt> {
        unavailable("restore")
    }
}
