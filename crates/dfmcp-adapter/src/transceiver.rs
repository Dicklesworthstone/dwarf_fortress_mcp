#![forbid(unsafe_code)]

//! High-level bidirectional IPC transceiver for the DFHack out-of-process bridge.
//!
//! Provides sequence-correlated request/response message exchange over any
//! stream implementing `std::io::Read + std::io::Write`.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::time::{Duration, Instant};

use dfmcp_core::{DfmcpError, ErrorCode, Result};

use crate::ipc::{
    IncrementalFrameDecoder, IpcConnectionState, IpcFrame, IpcMessageType, IpcTelemetry,
    ReconnectionPolicy,
};

/// Configuration for the IPC transceiver.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransceiverConfig {
    /// Maximum time to wait for a synchronous response.
    pub request_timeout: Duration,
    /// Keep-alive heartbeat interval.
    pub heartbeat_interval: Duration,
    /// Reconnection policy upon stream failure.
    pub reconnection_policy: ReconnectionPolicy,
}

impl Default for TransceiverConfig {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_millis(5000),
            heartbeat_interval: Duration::from_millis(1000),
            reconnection_policy: ReconnectionPolicy::default(),
        }
    }
}

/// Bidirectional IPC Transceiver over a duplex stream `S`.
pub struct IpcTransceiver<S> {
    stream: S,
    config: TransceiverConfig,
    state: IpcConnectionState,
    decoder: IncrementalFrameDecoder,
    telemetry: IpcTelemetry,
    next_sequence_id: u64,
    pending_responses: BTreeMap<u64, IpcFrame>,
    last_activity: Instant,
}

impl<S: Read + Write> IpcTransceiver<S> {
    /// Create a new transceiver wrapping the provided duplex stream.
    pub fn new(stream: S, config: TransceiverConfig) -> Self {
        Self {
            stream,
            config,
            state: IpcConnectionState::Connected {
                target_path: "stream".to_owned(),
            },
            decoder: IncrementalFrameDecoder::new(),
            telemetry: IpcTelemetry::default(),
            next_sequence_id: 1,
            pending_responses: BTreeMap::new(),
            last_activity: Instant::now(),
        }
    }

    /// Access current connection state.
    #[must_use]
    pub fn state(&self) -> IpcConnectionState {
        self.state.clone()
    }

    /// Access current telemetry counters.
    #[must_use]
    pub fn telemetry(&self) -> &IpcTelemetry {
        &self.telemetry
    }

    /// Send a framed message without expecting an immediate sequence-correlated response.
    pub fn send_frame(&mut self, frame: &IpcFrame) -> Result<()> {
        if matches!(self.state, IpcConnectionState::Disconnected) {
            return Err(DfmcpError::new(
                ErrorCode::BridgeConnectionFailed,
                "cannot send frame on disconnected transceiver",
            ));
        }

        let encoded = frame.encode()?;
        self.stream.write_all(&encoded).map_err(|err| {
            self.state = IpcConnectionState::Disconnected;
            DfmcpError::new(
                ErrorCode::BridgeConnectionFailed,
                format!("failed to write frame to IPC stream: {err}"),
            )
        })?;

        self.stream.flush().map_err(|err| {
            self.state = IpcConnectionState::Disconnected;
            DfmcpError::new(
                ErrorCode::BridgeConnectionFailed,
                format!("failed to flush IPC stream: {err}"),
            )
        })?;

        self.telemetry.frames_sent = self.telemetry.frames_sent.saturating_add(1);
        self.telemetry.bytes_sent = self
            .telemetry
            .bytes_sent
            .saturating_add(encoded.len() as u64);
        self.last_activity = Instant::now();

        Ok(())
    }

    /// Read and decode incoming frames from the stream into the internal queue.
    pub fn poll_incoming(&mut self) -> Result<usize> {
        let mut buffer = [0u8; 8192];
        let bytes_read = match self.stream.read(&mut buffer) {
            Ok(0) => {
                self.state = IpcConnectionState::Disconnected;
                return Err(DfmcpError::new(
                    ErrorCode::BridgeConnectionFailed,
                    "IPC stream closed by remote bridge peer",
                ));
            }
            Ok(n) => n,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                return Ok(0);
            }
            Err(err) => {
                self.state = IpcConnectionState::Disconnected;
                return Err(DfmcpError::new(
                    ErrorCode::BridgeConnectionFailed,
                    format!("IPC stream read error: {err}"),
                ));
            }
        };

        self.telemetry.bytes_received = self
            .telemetry
            .bytes_received
            .saturating_add(bytes_read as u64);

        self.decoder.push_bytes(&buffer[..bytes_read]);
        let mut count = 0;
        while let Some(frame) = self.decoder.poll_next_frame()? {
            count += 1;
            self.telemetry.frames_received = self.telemetry.frames_received.saturating_add(1);
            let seq = self.next_sequence_id;
            self.next_sequence_id = self.next_sequence_id.saturating_add(1);
            self.pending_responses.insert(seq, frame);
        }

        self.last_activity = Instant::now();
        Ok(count)
    }

    /// Execute a synchronous request-response round-trip.
    pub fn request(
        &mut self,
        request_type: IpcMessageType,
        payload: Vec<u8>,
        expected_response_type: IpcMessageType,
    ) -> Result<IpcFrame> {
        let req_frame = IpcFrame::new(request_type, payload)?;
        self.send_frame(&req_frame)?;

        let deadline = Instant::now() + self.config.request_timeout;

        loop {
            // Check if we have an unconsumed response matching our expected type
            let found_seq = self
                .pending_responses
                .iter()
                .find(|(_, frame)| frame.message_type == expected_response_type)
                .map(|(seq, _)| *seq);

            if let Some(seq) = found_seq {
                if let Some(frame) = self.pending_responses.remove(&seq) {
                    return Ok(frame);
                }
            }

            // Check for error responses
            let error_seq = self
                .pending_responses
                .iter()
                .find(|(_, frame)| frame.message_type == IpcMessageType::ErrorResponse)
                .map(|(seq, _)| *seq);

            if let Some(seq) = error_seq {
                if let Some(err_frame) = self.pending_responses.remove(&seq) {
                    let err_msg = String::from_utf8_lossy(&err_frame.payload).to_string();
                    return Err(DfmcpError::new(ErrorCode::BridgeProtocolError, err_msg));
                }
            }

            if Instant::now() >= deadline {
                return Err(DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    format!(
                        "IPC request {:?} timed out waiting for response {:?}",
                        request_type, expected_response_type
                    ),
                ));
            }

            self.poll_incoming()?;
        }
    }

    /// Send a heartbeat frame to verify bridge liveness.
    pub fn ping(&mut self) -> Result<()> {
        let heartbeat = IpcFrame::new(IpcMessageType::Heartbeat, Vec::new())?;
        self.send_frame(&heartbeat)
    }
}
