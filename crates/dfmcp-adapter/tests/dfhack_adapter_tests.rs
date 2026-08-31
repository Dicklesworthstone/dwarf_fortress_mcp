#![forbid(unsafe_code)]

//! Integration tests for the explicitly legacy opaque-framing laboratory.

use std::collections::VecDeque;
use std::error::Error;
use std::io::{Read, Write};
use std::time::Duration;

use dfmcp_adapter::{
    CompatibilityLevel, GameAdapter, HealthStatus, IpcFrame, IpcMessageType, IpcTransceiver,
    LegacyBridgeProbeAdapter, LegacyBridgeProbeConfig, TransceiverConfig,
};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, Digest32, ErrorCode, FortressId, GameTick,
    ObservationCursor, OperationContext, RequestId, RiskTier, SessionId, StateAnchor, WorkBudget,
};

struct MockDuplexStream {
    incoming: VecDeque<u8>,
    outgoing: Vec<u8>,
}

impl MockDuplexStream {
    fn new() -> Self {
        Self {
            incoming: VecDeque::new(),
            outgoing: Vec::new(),
        }
    }

    fn queue_response(&mut self, frame: &IpcFrame) -> Result<(), Box<dyn Error>> {
        let encoded = frame.encode()?;
        self.incoming.extend(encoded);
        Ok(())
    }
}

impl Read for MockDuplexStream {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.incoming.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "no data available",
            ));
        }

        let mut read_count = 0;
        for byte in buffer.iter_mut() {
            if let Some(value) = self.incoming.pop_front() {
                *byte = value;
                read_count += 1;
            } else {
                break;
            }
        }
        Ok(read_count)
    }
}

impl Write for MockDuplexStream {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.outgoing.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn test_context() -> OperationContext {
    let grants = vec![
        CapabilityGrant {
            capability: Capability::Observe,
            scope: CapabilityScope::default(),
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        },
        CapabilityGrant {
            capability: Capability::Doctor,
            scope: CapabilityScope::default(),
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        },
        CapabilityGrant {
            capability: Capability::Query,
            scope: CapabilityScope::default(),
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        },
        CapabilityGrant {
            capability: Capability::Plan,
            scope: CapabilityScope::default(),
            max_risk: RiskTier::Reversible,
            expires_at_tick: None,
            remaining_uses: None,
        },
        CapabilityGrant {
            capability: Capability::Checkpoint,
            scope: CapabilityScope::default(),
            max_risk: RiskTier::Guarded,
            expires_at_tick: None,
            remaining_uses: None,
        },
        CapabilityGrant {
            capability: Capability::Restore,
            scope: CapabilityScope::default(),
            max_risk: RiskTier::Guarded,
            expires_at_tick: None,
            remaining_uses: None,
        },
    ];

    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: StateAnchor {
            fortress_id: FortressId::new(1),
            tick: GameTick(100),
            cursor: ObservationCursor::ORIGIN,
            state_hash: Digest32::ZERO,
        },
        budget: WorkBudget::default(),
        grants,
        cancellation_requested: false,
    }
}

#[test]
fn transceiver_request_response_round_trip() -> Result<(), Box<dyn Error>> {
    let mut stream = MockDuplexStream::new();
    let response = IpcFrame::new(IpcMessageType::HealthResponse, vec![0x00, 0x01])?;
    stream.queue_response(&response)?;

    let config = TransceiverConfig {
        request_timeout: Duration::from_millis(500),
        ..TransceiverConfig::default()
    };
    let mut transceiver = IpcTransceiver::new(stream, config);

    let result = transceiver.request(
        IpcMessageType::HealthRequest,
        Vec::new(),
        IpcMessageType::HealthResponse,
        &test_context(),
    )?;

    assert_eq!(result.message_type, IpcMessageType::HealthResponse);
    assert_eq!(result.payload, vec![0x00, 0x01]);
    assert_eq!(transceiver.telemetry().frames_sent, 1);
    assert_eq!(transceiver.telemetry().frames_received, 1);
    Ok(())
}

#[test]
fn transceiver_rejects_frame_header_above_byte_budget() {
    let stream = MockDuplexStream::new();
    let mut transceiver = IpcTransceiver::new(stream, TransceiverConfig::default());
    let mut context = test_context();
    context.budget.max_bytes = 1;
    let result = transceiver.request(
        IpcMessageType::HealthRequest,
        Vec::new(),
        IpcMessageType::HealthResponse,
        &context,
    );
    assert!(matches!(result, Err(ref error) if error.code == ErrorCode::BudgetExceeded));
    assert_eq!(transceiver.telemetry().frames_sent, 0);
}

#[test]
fn legacy_probe_health_propagates_context_cancellation() {
    let stream = MockDuplexStream::new();
    let mut adapter =
        LegacyBridgeProbeAdapter::new(stream, LegacyBridgeProbeConfig::default());
    let mut context = test_context();
    context.cancellation_requested = true;
    let result = adapter.health(&context);
    assert!(matches!(result, Err(ref error) if error.code == ErrorCode::CancellationRequested));
}

#[test]
fn legacy_probe_health_never_claims_dfhack_state() -> Result<(), Box<dyn Error>> {
    let mut stream = MockDuplexStream::new();
    let response = IpcFrame::new(IpcMessageType::HealthResponse, vec![0x00, 0x01])?;
    stream.queue_response(&response)?;

    let mut adapter =
        LegacyBridgeProbeAdapter::new(stream, LegacyBridgeProbeConfig::default());
    let health = adapter.health(&test_context())?;
    assert_eq!(health.status, HealthStatus::Degraded);
    assert_eq!(health.paused, None);
    assert!(!health.fortress_loaded);
    assert_eq!(health.identity.name, "dfhack-opaque-framing-laboratory");
    assert_eq!(health.identity.dwarf_fortress_version, "unverified");
    assert_eq!(health.identity.compatibility, CompatibilityLevel::Unknown);
    assert_eq!(
        health.identity.capabilities,
        std::collections::BTreeSet::from([Capability::Doctor])
    );
    Ok(())
}

#[test]
fn configured_version_expectations_do_not_claim_a_handshake() {
    let stream = MockDuplexStream::new();
    let config = LegacyBridgeProbeConfig {
        target_df_version: "53.16".to_owned(),
        target_dfhack_version: "53.16-r1.1".to_owned(),
        ..LegacyBridgeProbeConfig::default()
    };
    let adapter = LegacyBridgeProbeAdapter::new(stream, config);
    let identity = adapter.identity();
    assert_eq!(identity.dwarf_fortress_version, "unverified");
    assert_eq!(identity.dfhack_version, "unverified");
    assert_eq!(identity.compatibility, CompatibilityLevel::Unknown);
}

#[test]
fn legacy_probe_checkpoint_and_restore_fail_closed() {
    let stream = MockDuplexStream::new();
    let mut adapter =
        LegacyBridgeProbeAdapter::new(stream, LegacyBridgeProbeConfig::default());
    let context = test_context();

    assert!(adapter.checkpoint("pre-siege-save", &context).is_err());
    assert!(
        adapter
            .restore(dfmcp_core::CheckpointId::new(1), &context)
            .is_err()
    );
}
