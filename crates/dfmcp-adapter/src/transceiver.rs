#![forbid(unsafe_code)]

//! High-level bidirectional IPC transceiver for the DFHack out-of-process bridge.
//!
//! Provides single-flight request/response exchange over a caller-supplied stream.
//!
//! The current frame format has no request identifier, so this type intentionally
//! does not claim sequence correlation and must not be shared across concurrent
//! requests. `S::read` must be non-blocking or externally timeout-bounded; a generic
//! blocking [`Read`] implementation cannot be interrupted by this synchronous API.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::thread;
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
    /// Maximum number of unsolicited frames retained while awaiting a response.
    pub max_pending_responses: usize,
}

impl Default for TransceiverConfig {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_millis(5000),
            heartbeat_interval: Duration::from_millis(1000),
            reconnection_policy: ReconnectionPolicy::default(),
            max_pending_responses: 128,
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
    pending_responses: VecDeque<IpcFrame>,
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
            pending_responses: VecDeque::new(),
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

    /// Send a framed message without expecting an immediate response.
    pub fn send_frame(&mut self, frame: &IpcFrame) -> Result<()> {
        if !matches!(self.state, IpcConnectionState::Connected { .. }) {
            return Err(DfmcpError::new(
                ErrorCode::AdapterUnavailable,
                "cannot send frame unless the transport stream is connected",
            ));
        }

        let encoded = frame.encode()?;
        self.stream.write_all(&encoded).map_err(|err| {
            self.state = IpcConnectionState::Disconnected;
            DfmcpError::new(
                ErrorCode::AdapterUnavailable,
                format!("failed to write frame to IPC stream: {err}"),
            )
        })?;

        self.stream.flush().map_err(|err| {
            self.state = IpcConnectionState::Disconnected;
            DfmcpError::new(
                ErrorCode::AdapterUnavailable,
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
                    ErrorCode::AdapterUnavailable,
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
                    ErrorCode::AdapterUnavailable,
                    format!("IPC stream read error: {err}"),
                ));
            }
        };

        self.telemetry.bytes_received = self
            .telemetry
            .bytes_received
            .saturating_add(bytes_read as u64);

        if let Err(error) = self.decoder.push_bytes(&buffer[..bytes_read]) {
            self.state = IpcConnectionState::Degraded {
                reason: error.message.clone(),
            };
            return Err(error);
        }
        let mut count = 0;
        loop {
            let frame = match self.decoder.poll_next_frame() {
                Ok(Some(frame)) => frame,
                Ok(None) => break,
                Err(error) => {
                    if error.message.contains("CRC32 mismatch") {
                        self.telemetry.crc_errors = self.telemetry.crc_errors.saturating_add(1);
                    }
                    self.state = IpcConnectionState::Degraded {
                        reason: error.message.clone(),
                    };
                    return Err(error);
                }
            };
            if self.pending_responses.len() >= self.config.max_pending_responses {
                self.state = IpcConnectionState::Degraded {
                    reason: "pending IPC response queue exceeded its configured bound".to_owned(),
                };
                return Err(DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    "pending IPC response queue exceeded its configured bound",
                ));
            }
            count += 1;
            self.telemetry.frames_received = self.telemetry.frames_received.saturating_add(1);
            self.pending_responses.push_back(frame);
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
        if self.pending_responses.iter().any(|frame| {
            frame.message_type == expected_response_type
                || frame.message_type == IpcMessageType::ErrorResponse
        }) {
            return Err(DfmcpError::new(
                ErrorCode::AdapterFailure,
                "ambiguous stale IPC response was queued before a new request",
            ));
        }

        let req_frame = IpcFrame::new(request_type, payload)?;
        self.send_frame(&req_frame)?;

        let deadline = Instant::now()
            .checked_add(self.config.request_timeout)
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::InvalidRequest,
                    "IPC request timeout is too large for the platform clock",
                )
            })?;

        loop {
            // Check if we have an unconsumed response matching our expected type
            let found_index = self
                .pending_responses
                .iter()
                .position(|frame| frame.message_type == expected_response_type);

            if let Some(index) = found_index
                && let Some(frame) = self.pending_responses.remove(index)
            {
                return Ok(frame);
            }

            // Check for error responses
            let error_index = self
                .pending_responses
                .iter()
                .position(|frame| frame.message_type == IpcMessageType::ErrorResponse);

            if let Some(index) = error_index
                && let Some(err_frame) = self.pending_responses.remove(index)
            {
                let err_msg = String::from_utf8_lossy(&err_frame.payload).to_string();
                return Err(DfmcpError::new(ErrorCode::AdapterUnavailable, err_msg));
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

            if self.poll_incoming()? == 0 {
                let remaining = deadline.saturating_duration_since(Instant::now());
                thread::sleep(remaining.min(Duration::from_millis(1)));
            }
        }
    }

    /// Perform a heartbeat round trip to verify bridge liveness.
    pub fn ping(&mut self) -> Result<()> {
        self.request(
            IpcMessageType::Heartbeat,
            Vec::new(),
            IpcMessageType::Heartbeat,
        )?;
        Ok(())
    }
}
