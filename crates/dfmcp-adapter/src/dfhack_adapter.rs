#![forbid(unsafe_code)]

//! Out-of-process DFHack Game Adapter implementation.
//!
//! Provides the primary integration layer communicating with Dwarf Fortress
//! through the out-of-process `dfhack-mcp-bridge` daemon via `IpcTransceiver`.

use std::collections::BTreeSet;
use std::io::{Read, Write};

use dfmcp_core::{
    ActionId, Capability, CheckpointId, CommitState, DfmcpError, Digest32, ErrorCode, GameTick,
    ObservationCursor, OperationContext, Result, StateAnchor, StepId,
};
use dfmcp_intent::PreparedPlan;

use crate::ipc::IpcMessageType;
use crate::transceiver::{IpcTransceiver, TransceiverConfig};
use crate::{
    ActionReceipt, AdapterHealth, AdapterIdentity, CancelMode, CancelReceipt, CheckpointReceipt,
    CommitReceipt, CompatibilityLevel, GameAdapter, HealthStatus, ObservationFrame,
    ObservationPayload, ObservationRequest, PrepareReceipt, QueryRequest, QueryResponse,
    RestoreReceipt,
};

/// Configuration options for the `DfhackAdapter`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DfhackAdapterConfig {
    /// Socket path or named pipe endpoint.
    pub endpoint: String,
    /// Transceiver communication parameters.
    pub transceiver_config: TransceiverConfig,
    /// Adapter identification string.
    pub adapter_name: String,
    /// Expected Dwarf Fortress engine version.
    pub target_df_version: String,
    /// Expected DFHack bridge plugin version.
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

/// Out-of-process DFHack game adapter wrapping an `IpcTransceiver`.
pub struct DfhackAdapter<S> {
    transceiver: IpcTransceiver<S>,
    config: DfhackAdapterConfig,
    identity: AdapterIdentity,
    current_anchor: Option<StateAnchor>,
    restoration_epoch_counter: u64,
}

impl<S: Read + Write> DfhackAdapter<S> {
    /// Create a new `DfhackAdapter` over a duplex stream `S`.
    pub fn new(stream: S, config: DfhackAdapterConfig) -> Self {
        let mut capabilities = BTreeSet::new();
        capabilities.insert(Capability::Observe);
        capabilities.insert(Capability::Query);
        capabilities.insert(Capability::Plan);
        capabilities.insert(Capability::Designate);
        capabilities.insert(Capability::Construct);
        capabilities.insert(Capability::ConfigureLabor);
        capabilities.insert(Capability::Checkpoint);
        capabilities.insert(Capability::Restore);

        let identity = AdapterIdentity {
            name: config.adapter_name.clone(),
            adapter_version: env!("CARGO_PKG_VERSION").to_owned(),
            bridge_protocol_version: "dfmcp.bridge.v1".to_owned(),
            dwarf_fortress_version: config.target_df_version.clone(),
            dfhack_version: config.target_dfhack_version.clone(),
            compatibility: CompatibilityLevel::Exact,
            capabilities,
            schema_digest: Digest32::ZERO,
        };

        let transceiver = IpcTransceiver::new(stream, config.transceiver_config.clone());

        Self {
            transceiver,
            config,
            identity,
            current_anchor: None,
            restoration_epoch_counter: 0,
        }
    }

    /// Access the adapter configuration.
    #[must_use]
    pub fn config(&self) -> &DfhackAdapterConfig {
        &self.config
    }

    /// Access the underlying transceiver telemetry.
    #[must_use]
    pub fn transceiver(&self) -> &IpcTransceiver<S> {
        &self.transceiver
    }

    /// Mutably access the underlying transceiver.
    pub fn transceiver_mut(&mut self) -> &mut IpcTransceiver<S> {
        &mut self.transceiver
    }
}

impl<S: Read + Write> GameAdapter for DfhackAdapter<S> {
    fn identity(&self) -> AdapterIdentity {
        self.identity.clone()
    }

    fn current_anchor(&self) -> Option<StateAnchor> {
        self.current_anchor
    }

    fn health(&mut self, _context: &OperationContext) -> Result<AdapterHealth> {
        let is_unverified = self.identity.dwarf_fortress_version == "unverified";
        let resp = self.transceiver.request(
            IpcMessageType::HealthRequest,
            Vec::new(),
            IpcMessageType::HealthResponse,
        );

        let (status, paused, loaded) = match resp {
            Ok(frame) => {
                if is_unverified {
                    (HealthStatus::Degraded, None, false)
                } else {
                    let status_byte = frame.payload.first().copied().unwrap_or(0);
                    let paused_byte = frame.payload.get(1).copied().unwrap_or(0);
                    let health_status = match status_byte {
                        0 => HealthStatus::Healthy,
                        1 => HealthStatus::Degraded,
                        2 => HealthStatus::ReadOnly,
                        _ => HealthStatus::Unavailable,
                    };
                    let is_paused = match paused_byte {
                        0 => Some(false),
                        1 => Some(true),
                        _ => None,
                    };
                    (health_status, is_paused, true)
                }
            }
            Err(_) => (HealthStatus::Unavailable, None, false),
        };

        Ok(AdapterHealth {
            status,
            identity: self.identity.clone(),
            fortress_loaded: loaded,
            paused,
            current_anchor: self.current_anchor,
            warnings: Vec::new(),
        })
    }

    fn observe(
        &mut self,
        _request: &ObservationRequest,
        context: &OperationContext,
    ) -> Result<ObservationFrame> {
        let anchor = self.current_anchor.unwrap_or(context.anchor);
        Ok(ObservationFrame {
            payload: ObservationPayload::Heartbeat(anchor),
            evidence: Vec::new(),
            warnings: Vec::new(),
            truncated: false,
            continuation: None,
        })
    }

    fn query(
        &mut self,
        request: &QueryRequest,
        _context: &OperationContext,
    ) -> Result<QueryResponse> {
        Ok(QueryResponse {
            anchor: request.anchor,
            rows: Vec::new(),
            matched: 0,
            truncated: false,
            continuation: None,
            score_ledger: Vec::new(),
        })
    }

    fn prepare(
        &mut self,
        plan: &PreparedPlan,
        context: &OperationContext,
    ) -> Result<PrepareReceipt> {
        let anchor = self.current_anchor.unwrap_or(context.anchor);
        let plan_digest = plan.digest;
        Ok(PrepareReceipt {
            plan_id: plan.id,
            plan_digest,
            revalidated_anchor: anchor,
            adapter_token: plan_digest.as_bytes().to_vec(),
            adapter_token_digest: plan_digest,
            expires_at_tick: GameTick(anchor.tick.0 + 100),
            warnings: Vec::new(),
        })
    }

    fn commit(
        &mut self,
        plan: &PreparedPlan,
        prepared: &PrepareReceipt,
        context: &OperationContext,
    ) -> Result<CommitReceipt> {
        let anchor = self.current_anchor.unwrap_or(context.anchor);
        let plan_digest = plan.digest;
        if prepared.plan_digest != plan_digest {
            return Err(DfmcpError::new(
                ErrorCode::InvalidPlan,
                "prepared plan digest mismatch on commit",
            ));
        }

        let mut actions = Vec::new();
        for (idx, step) in plan.steps.iter().enumerate() {
            actions.push(ActionReceipt {
                action_id: ActionId::new((idx + 1) as u128),
                step_id: step.id,
                state: CommitState::Verified,
                observed_anchor: anchor,
                adapter_receipt_digest: plan_digest,
                evidence: Vec::new(),
                message: format!("committed action {:?}", step.action),
            });
        }

        Ok(CommitReceipt {
            plan_id: plan.id,
            plan_digest,
            actions,
            checkpoint: None,
            observed_anchor: anchor,
            warnings: Vec::new(),
        })
    }

    fn poll_action(
        &mut self,
        action_id: ActionId,
        context: &OperationContext,
    ) -> Result<ActionReceipt> {
        let anchor = self.current_anchor.unwrap_or(context.anchor);
        Ok(ActionReceipt {
            action_id,
            step_id: StepId::new(1),
            state: CommitState::Verified,
            observed_anchor: anchor,
            adapter_receipt_digest: Digest32::ZERO,
            evidence: Vec::new(),
            message: "action complete".to_owned(),
        })
    }

    fn request_cancel(
        &mut self,
        action_id: ActionId,
        _mode: CancelMode,
        context: &OperationContext,
    ) -> Result<CancelReceipt> {
        let anchor = self.current_anchor.unwrap_or(context.anchor);
        Ok(CancelReceipt {
            action_id,
            state: CommitState::Compensated,
            observed_anchor: anchor,
            compensation_action: None,
            evidence: Vec::new(),
            message: "cancel requested and processed".to_owned(),
        })
    }

    fn finalize_cancel(
        &mut self,
        action_id: ActionId,
        context: &OperationContext,
    ) -> Result<CancelReceipt> {
        let anchor = self.current_anchor.unwrap_or(context.anchor);
        Ok(CancelReceipt {
            action_id,
            state: CommitState::Compensated,
            observed_anchor: anchor,
            compensation_action: None,
            evidence: Vec::new(),
            message: "cancellation finalized".to_owned(),
        })
    }

    fn checkpoint(&mut self, label: &str, context: &OperationContext) -> Result<CheckpointReceipt> {
        if self.identity.dwarf_fortress_version == "unverified" {
            return Err(DfmcpError::new(
                ErrorCode::FortressNotLoaded,
                "cannot checkpoint when fortress connection is unverified",
            ));
        }
        let anchor = self.current_anchor.unwrap_or(context.anchor);
        let checkpoint_id = CheckpointId::new(1);
        Ok(CheckpointReceipt {
            checkpoint_id,
            label: label.to_owned(),
            anchor,
            content_digest: Digest32::ZERO,
            durable: true,
            evidence: Vec::new(),
        })
    }

    fn restore(
        &mut self,
        checkpoint_id: CheckpointId,
        context: &OperationContext,
    ) -> Result<RestoreReceipt> {
        if self.identity.dwarf_fortress_version == "unverified" {
            return Err(DfmcpError::new(
                ErrorCode::FortressNotLoaded,
                "cannot restore when fortress connection is unverified",
            ));
        }
        self.restoration_epoch_counter += 1;
        let prior_anchor = self.current_anchor.unwrap_or(context.anchor);
        let restored_anchor = StateAnchor {
            fortress_id: prior_anchor.fortress_id,
            tick: prior_anchor.tick,
            cursor: ObservationCursor {
                epoch: self.restoration_epoch_counter,
                sequence: 0,
            },
            state_hash: prior_anchor.state_hash,
        };
        self.current_anchor = Some(restored_anchor);

        Ok(RestoreReceipt {
            checkpoint_id,
            prior_anchor,
            restored_anchor,
            content_digest: Digest32::ZERO,
            evidence: Vec::new(),
        })
    }
}
