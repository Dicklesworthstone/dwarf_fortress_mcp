#![forbid(unsafe_code)]

//! Integration tests for the out-of-process `DfhackAdapter` and `IpcTransceiver`.

use std::collections::VecDeque;
use std::error::Error;
use std::io::{Read, Write};
use std::time::Duration;

use dfmcp_adapter::{
    DfhackAdapter, DfhackAdapterConfig, GameAdapter, HealthStatus, IpcFrame, IpcMessageType,
    IpcTransceiver, TransceiverConfig,
};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, Digest32, FortressId, GameTick,
    ObservationCursor, OperationContext, RequestId, RiskTier, SessionId, StateAnchor, WorkBudget,
};

/// Duplex in-memory pipe simulating an IPC channel to DFHack.
struct MockDuplexStream {
    pub incoming: VecDeque<u8>,
    pub outgoing: Vec<u8>,
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
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.incoming.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "no data available",
            ));
        }

        let mut read_count = 0;
        for byte in buf.iter_mut() {
            if let Some(b) = self.incoming.pop_front() {
                *byte = b;
                read_count += 1;
            } else {
                break;
            }
        }
        Ok(read_count)
    }
}

impl Write for MockDuplexStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.outgoing.extend_from_slice(buf);
        Ok(buf.len())
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
fn test_transceiver_request_response_round_trip() -> Result<(), Box<dyn Error>> {
    let mut stream = MockDuplexStream::new();
    let resp_frame = IpcFrame::new(IpcMessageType::HealthResponse, vec![0x00, 0x01])?;
    stream.queue_response(&resp_frame)?;

    let config = TransceiverConfig {
        request_timeout: Duration::from_millis(500),
        ..TransceiverConfig::default()
    };
    let mut transceiver = IpcTransceiver::new(stream, config);

    let result = transceiver.request(
        IpcMessageType::HealthRequest,
        Vec::new(),
        IpcMessageType::HealthResponse,
    )?;

    assert_eq!(result.message_type, IpcMessageType::HealthResponse);
    assert_eq!(result.payload, vec![0x00, 0x01]);
    assert_eq!(transceiver.telemetry().frames_sent, 1);
    assert_eq!(transceiver.telemetry().frames_received, 1);
    Ok(())
}

#[test]
fn test_dfhack_adapter_health_check() -> Result<(), Box<dyn Error>> {
    let mut stream = MockDuplexStream::new();
    // Status 0 (Healthy), Paused 1 (true)
    let resp_frame = IpcFrame::new(IpcMessageType::HealthResponse, vec![0x00, 0x01])?;
    stream.queue_response(&resp_frame)?;

    let mut adapter = DfhackAdapter::new(stream, DfhackAdapterConfig::default());
    let ctx = test_context();

    let health = adapter.health(&ctx)?;
    assert_eq!(health.status, HealthStatus::Degraded);
    assert_eq!(health.paused, None);
    assert!(!health.fortress_loaded);
    assert_eq!(health.identity.name, "dfhack-oop-bridge-probe");
    assert_eq!(health.identity.dwarf_fortress_version, "unverified");
    Ok(())
}

#[test]
fn test_dfhack_adapter_checkpoint_and_restore_fail_closed() -> Result<(), Box<dyn Error>> {
    let stream = MockDuplexStream::new();
    let mut adapter = DfhackAdapter::new(stream, DfhackAdapterConfig::default());
    let ctx = test_context();

    let checkpoint = adapter.checkpoint("pre-siege-save", &ctx);
    assert!(checkpoint.is_err());

    let restore = adapter.restore(dfmcp_core::CheckpointId::new(1), &ctx);
    assert!(restore.is_err());
    Ok(())
}
