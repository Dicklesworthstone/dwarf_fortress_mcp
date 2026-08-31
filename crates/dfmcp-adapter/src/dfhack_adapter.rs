#![forbid(unsafe_code)]

//! Fail-closed probe for the proposed out-of-process DFHack bridge.
//!
//! The repository does not yet define a negotiated bridge handshake or canonical
//! payload codecs. A framed response therefore proves transport liveness only; it
//! does not prove DF/DFHack compatibility, a loaded fortress, an observation, or
//! any game effect.

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

/// Configuration for the unnegotiated bridge probe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DfhackAdapterConfig {
    /// Intended socket path or named-pipe endpoint. The caller supplies the
    /// already-open stream; this field is diagnostic metadata only.
    pub endpoint: String,
    pub transceiver_config: TransceiverConfig,
    pub adapter_name: String,
    /// Operator expectation, not an authenticated observation.
    pub target_df_version: String,
    /// Operator expectation, not an authenticated observation.
    pub target_dfhack_version: String,
}

impl Default for DfhackAdapterConfig {
    fn default() -> Self {
        Self {
            endpoint: "/tmp/dfhack-mcp.sock".to_owned(),
            transceiver_config: TransceiverConfig::default(),
            adapter_name: "dfhack-oop-bridge-probe".to_owned(),
            target_df_version: "unverified".to_owned(),
            target_dfhack_version: "unverified".to_owned(),
        }
    }
}

/// A transport-liveness probe. All semantic adapter operations remain disabled
/// until a real authenticated handshake and payload codecs exist.
pub struct DfhackAdapter<S> {
    transceiver: IpcTransceiver<S>,
    config: DfhackAdapterConfig,
    identity: AdapterIdentity,
}

impl<S: Read + Write> DfhackAdapter<S> {
    #[must_use]
    pub fn new(stream: S, config: DfhackAdapterConfig) -> Self {
        let capabilities = BTreeSet::from([Capability::Doctor]);
        let identity = AdapterIdentity {
            name: config.adapter_name.clone(),
            adapter_version: env!("CARGO_PKG_VERSION").to_owned(),
            bridge_protocol_version: "dfmcp/0.1-unnegotiated".to_owned(),
            dwarf_fortress_version: "unverified".to_owned(),
            dfhack_version: "unverified".to_owned(),
            compatibility: CompatibilityLevel::Unknown,
            capabilities,
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
    pub fn config(&self) -> &DfhackAdapterConfig {
        &self.config
    }

    #[must_use]
    pub fn transceiver(&self) -> &IpcTransceiver<S> {
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
            "DFHack {operation} is disabled until a bridge handshake and canonical payload codec are implemented"
        ),
    ))
}

impl<S: Read + Write> GameAdapter for DfhackAdapter<S> {
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
        );
        let (status, warning) = match result {
            Ok(_) => (
                HealthStatus::Degraded,
                "bridge transport responded, but its identity and payload are not authenticated or decoded",
            ),
            Err(_) => (
                HealthStatus::Unavailable,
                "bridge transport did not complete the opaque liveness probe",
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
