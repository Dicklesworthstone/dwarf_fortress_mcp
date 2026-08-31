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
use std::time::{Duration, Instant};

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, OperationContext, Result};

use crate::ipc::{
    FRAME_HEADER_SIZE, IncrementalFrameDecoder, IpcConnectionState, IpcFrame, IpcMessageType,
    IpcTelemetry, ReconnectionPolicy,
};

const MAX_CONFIGURED_PENDING_RESPONSES: usize = 1_024;
const MAX_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(300);
const MAX_RECONNECT_BACKOFF_MILLIS: u64 = 300_000;
const MAX_RECONNECT_ATTEMPTS: u32 = 100;
const MAX_BRIDGE_ERROR_BYTES: usize = 4_096;
const MAX_PENDING_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

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
    pending_response_bytes: usize,
    in_flight: Option<InFlightRequest>,
    last_activity: Instant,
}

#[derive(Clone, Copy, Debug)]
struct InFlightRequest {
    identity: Digest32,
    expected_response_type: IpcMessageType,
    deadline: Instant,
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
            pending_response_bytes: 0,
            in_flight: None,
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
    pub fn send_frame(&mut self, frame: &IpcFrame, context: &OperationContext) -> Result<()> {
        validate_context_and_config(context, &self.config)?;
        if !matches!(self.state, IpcConnectionState::Connected { .. }) {
            return Err(DfmcpError::new(
                ErrorCode::AdapterUnavailable,
                "cannot send frame unless the transport stream is connected",
            ));
        }

        let encoded_size = FRAME_HEADER_SIZE
            .checked_add(frame.payload.len())
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    "encoded IPC frame length overflowed",
                )
            })?;
        let encoded_len = u64::try_from(encoded_size).map_err(|_| {
            DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "encoded IPC frame length cannot be represented",
            )
        })?;
        if encoded_len > context.budget.max_bytes {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "encoded IPC frame exceeds the operation byte budget",
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
        self.telemetry.bytes_sent = self.telemetry.bytes_sent.saturating_add(encoded_len);
        self.last_activity = Instant::now();

        Ok(())
    }

    /// Read and decode incoming frames from the stream into the internal queue.
    pub fn poll_incoming(&mut self, context: &OperationContext) -> Result<usize> {
        validate_context_and_config(context, &self.config)?;
        if !matches!(self.state, IpcConnectionState::Connected { .. }) {
            return Err(DfmcpError::new(
                ErrorCode::AdapterUnavailable,
                "cannot poll frames unless the transport stream is connected",
            ));
        }
        let mut buffer = [0u8; 8192];
        let read_limit = usize::try_from(context.budget.max_bytes)
            .map_or(buffer.len(), |limit| limit.min(buffer.len()));
        let bytes_read = match self.stream.read(&mut buffer[..read_limit]) {
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

        let bytes_read_u64 = u64::try_from(bytes_read).map_err(|_| {
            DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "IPC read length cannot be represented",
            )
        })?;
        self.telemetry.bytes_received =
            self.telemetry.bytes_received.saturating_add(bytes_read_u64);

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
            let frame_wire_size = FRAME_HEADER_SIZE
                .checked_add(frame.payload.len())
                .ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::BudgetExceeded,
                        "IPC response frame length overflowed",
                    )
                })?;
            let frame_wire_len = u64::try_from(frame_wire_size).map_err(|_| {
                DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    "IPC response frame length cannot be represented",
                )
            })?;
            if frame_wire_len > context.budget.max_bytes {
                self.state = IpcConnectionState::Degraded {
                    reason: "IPC response exceeded the operation byte budget".to_owned(),
                };
                return Err(DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    "IPC response exceeded the operation byte budget",
                ));
            }
            let next_pending_bytes = self
                .pending_response_bytes
                .checked_add(frame_wire_size)
                .ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::BudgetExceeded,
                        "pending IPC response byte count overflowed",
                    )
                })?;
            if next_pending_bytes > MAX_PENDING_RESPONSE_BYTES
                || u64::try_from(next_pending_bytes)
                    .map_or(true, |queued| queued > context.budget.max_bytes)
            {
                self.state = IpcConnectionState::Degraded {
                    reason: "pending IPC response queue exceeded its byte bound".to_owned(),
                };
                return Err(DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    "pending IPC response queue exceeded its byte bound",
                ));
            }
            count += 1;
            self.telemetry.frames_received = self.telemetry.frames_received.saturating_add(1);
            self.pending_response_bytes = next_pending_bytes;
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
        context: &OperationContext,
    ) -> Result<IpcFrame> {
        validate_context_and_config(context, &self.config)?;
        let request_wire_size = FRAME_HEADER_SIZE
            .checked_add(payload.len())
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    "IPC request frame length overflowed",
                )
            })?;
        let request_wire_len = u64::try_from(request_wire_size).map_err(|_| {
            DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "IPC request frame length cannot be represented",
            )
        })?;
        if request_wire_len > context.budget.max_bytes {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "IPC request frame exceeds the operation byte budget",
            ));
        }
        let req_frame = IpcFrame::new(request_type, payload)?;
        let identity = request_identity(request_type, expected_response_type, &req_frame.payload);
        let deadline = if let Some(in_flight) = self.in_flight {
            if in_flight.identity != identity
                || in_flight.expected_response_type != expected_response_type
            {
                return Err(DfmcpError::new(
                    ErrorCode::AdapterFailure,
                    "a different IPC request is already in flight on this single-flight stream",
                ));
            }
            in_flight.deadline
        } else {
            if self.pending_responses.iter().any(|frame| {
                frame.message_type == expected_response_type
                    || frame.message_type == IpcMessageType::ErrorResponse
            }) {
                return Err(DfmcpError::new(
                    ErrorCode::AdapterFailure,
                    "ambiguous stale IPC response was queued before a new request",
                ));
            }
            let effective_timeout = self
                .config
                .request_timeout
                .min(Duration::from_millis(context.budget.max_wall_millis));
            let deadline = Instant::now()
                .checked_add(effective_timeout)
                .ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::InvalidRequest,
                        "IPC request timeout is too large for the platform clock",
                    )
                })?;
            self.send_frame(&req_frame, context)?;
            self.in_flight = Some(InFlightRequest {
                identity,
                expected_response_type,
                deadline,
            });
            deadline
        };

        loop {
            if Instant::now() >= deadline {
                self.in_flight = None;
                self.state = IpcConnectionState::Degraded {
                    reason: "IPC response deadline expired; stream cannot be safely reused without correlation identifiers"
                        .to_owned(),
                };
                return Err(DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    format!(
                        "IPC request {:?} timed out waiting for response {:?}",
                        request_type, expected_response_type
                    ),
                ));
            }

            // Check if we have an unconsumed response matching our expected type
            let found_index = self
                .pending_responses
                .iter()
                .position(|frame| frame.message_type == expected_response_type);

            if let Some(index) = found_index
                && let Some(frame) = self.remove_pending_response(index)?
            {
                let frame_wire_size = FRAME_HEADER_SIZE
                    .checked_add(frame.payload.len())
                    .ok_or_else(|| {
                        DfmcpError::new(
                            ErrorCode::BudgetExceeded,
                            "IPC response frame length overflowed",
                        )
                    })?;
                if frame_wire_size
                    > usize::try_from(context.budget.max_bytes).map_or(usize::MAX, |value| value)
                {
                    self.in_flight = None;
                    return Err(DfmcpError::new(
                        ErrorCode::BudgetExceeded,
                        "IPC response exceeded the operation byte budget",
                    ));
                }
                self.in_flight = None;
                return Ok(frame);
            }

            // Check for error responses
            let error_index = self
                .pending_responses
                .iter()
                .position(|frame| frame.message_type == IpcMessageType::ErrorResponse);

            if let Some(index) = error_index
                && let Some(err_frame) = self.remove_pending_response(index)?
            {
                self.in_flight = None;
                let retained = err_frame.payload.len().min(MAX_BRIDGE_ERROR_BYTES);
                let err_msg = String::from_utf8_lossy(&err_frame.payload[..retained]).to_string();
                return Err(DfmcpError::new(ErrorCode::AdapterUnavailable, err_msg));
            }

            match self.poll_incoming(context) {
                Ok(0) => {
                    return Err(DfmcpError::new(
                        ErrorCode::AdapterUnavailable,
                        "nonblocking IPC response is not ready; poll again with the identical request from the supervised runtime",
                    )
                    .retryable(true));
                }
                Ok(_) => {}
                Err(error) => {
                    self.in_flight = None;
                    return Err(error);
                }
            }
        }
    }

    /// Perform a heartbeat round trip to verify bridge liveness.
    pub fn ping(&mut self, context: &OperationContext) -> Result<()> {
        self.request(
            IpcMessageType::Heartbeat,
            Vec::new(),
            IpcMessageType::Heartbeat,
            context,
        )?;
        Ok(())
    }

    fn remove_pending_response(&mut self, index: usize) -> Result<Option<IpcFrame>> {
        let Some(frame) = self.pending_responses.remove(index) else {
            return Ok(None);
        };
        self.pending_response_bytes = self
            .pending_response_bytes
            .checked_sub(
                FRAME_HEADER_SIZE
                    .checked_add(frame.payload.len())
                    .ok_or_else(|| {
                        DfmcpError::new(
                            ErrorCode::InternalInvariantViolation,
                            "pending IPC response frame length overflowed",
                        )
                    })?,
            )
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::InternalInvariantViolation,
                    "pending IPC response byte accounting is inconsistent",
                )
            })?;
        Ok(Some(frame))
    }
}

fn request_identity(
    request_type: IpcMessageType,
    expected_response_type: IpcMessageType,
    payload: &[u8],
) -> Digest32 {
    let mut bytes = Vec::with_capacity(payload.len().saturating_add(32));
    bytes.extend_from_slice(b"dfmcp-ipc-in-flight-request-v1");
    bytes.extend_from_slice(&request_type.as_u16().to_be_bytes());
    bytes.extend_from_slice(&expected_response_type.as_u16().to_be_bytes());
    bytes.extend_from_slice(payload);
    Digest32::of_bytes(&bytes)
}

fn validate_context_and_config(
    context: &OperationContext,
    config: &TransceiverConfig,
) -> Result<()> {
    context.budget.validate()?;
    if context.cancellation_requested {
        return Err(DfmcpError::new(
            ErrorCode::CancellationRequested,
            "IPC operation was cancelled before transport I/O",
        ));
    }
    if config.request_timeout.is_zero()
        || config.request_timeout > MAX_REQUEST_TIMEOUT
        || config.heartbeat_interval.is_zero()
        || config.heartbeat_interval > MAX_HEARTBEAT_INTERVAL
        || config.max_pending_responses == 0
        || config.max_pending_responses > MAX_CONFIGURED_PENDING_RESPONSES
        || config.reconnection_policy.initial_backoff_millis == 0
        || config.reconnection_policy.initial_backoff_millis
            > config.reconnection_policy.max_backoff_millis
        || config.reconnection_policy.max_backoff_millis > MAX_RECONNECT_BACKOFF_MILLIS
        || config.reconnection_policy.max_attempts == 0
        || config.reconnection_policy.max_attempts > MAX_RECONNECT_ATTEMPTS
    {
        return Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "IPC transceiver configuration exceeds its explicit timeout or queue bounds",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::io::{Read, Write};

    use dfmcp_core::{
        Digest32, FortressId, GameTick, ObservationCursor, OperationContext, RequestId, SessionId,
        StateAnchor, WorkBudget,
    };

    use super::*;

    struct WouldBlockThenResponse {
        reads: usize,
        response: VecDeque<u8>,
        written: Vec<u8>,
    }

    impl Read for WouldBlockThenResponse {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.reads += 1;
            if self.reads == 1 {
                return Err(std::io::Error::from(std::io::ErrorKind::WouldBlock));
            }
            let mut count = 0;
            for slot in buffer {
                let Some(byte) = self.response.pop_front() else {
                    break;
                };
                *slot = byte;
                count += 1;
            }
            Ok(count)
        }
    }

    impl Write for WouldBlockThenResponse {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.written.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn context() -> OperationContext {
        OperationContext {
            session_id: SessionId::new(1),
            request_id: RequestId::new(1),
            anchor: StateAnchor {
                fortress_id: FortressId::new(1),
                tick: GameTick(1),
                cursor: ObservationCursor::ORIGIN,
                state_hash: Digest32::ZERO,
            },
            budget: WorkBudget::CONSERVATIVE_DEFAULT,
            grants: Vec::new(),
            cancellation_requested: false,
        }
    }

    #[test]
    fn nonblocking_retry_does_not_resend_the_in_flight_frame() -> Result<()> {
        let response = IpcFrame::new(IpcMessageType::HealthResponse, b"ok".to_vec())?.encode()?;
        let stream = WouldBlockThenResponse {
            reads: 0,
            response: response.into(),
            written: Vec::new(),
        };
        let mut transceiver = IpcTransceiver::new(stream, TransceiverConfig::default());

        let first = transceiver.request(
            IpcMessageType::HealthRequest,
            Vec::new(),
            IpcMessageType::HealthResponse,
            &context(),
        );
        assert!(matches!(first, Err(ref error) if error.retryable));
        assert_eq!(transceiver.telemetry.frames_sent, 1);

        let different = transceiver.request(
            IpcMessageType::Heartbeat,
            Vec::new(),
            IpcMessageType::Heartbeat,
            &context(),
        );
        assert!(matches!(different, Err(ref error) if error.code == ErrorCode::AdapterFailure));
        assert_eq!(transceiver.telemetry.frames_sent, 1);

        let second = transceiver.request(
            IpcMessageType::HealthRequest,
            Vec::new(),
            IpcMessageType::HealthResponse,
            &context(),
        )?;
        assert_eq!(second.payload, b"ok");
        assert_eq!(transceiver.telemetry.frames_sent, 1);
        assert!(transceiver.in_flight.is_none());
        Ok(())
    }
}
