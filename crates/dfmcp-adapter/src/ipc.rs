#![forbid(unsafe_code)]

//! Out-of-process framed binary IPC transceiver for the DFHack bridge.
//!
//! WP-DFH-01: Dwarf Fortress and DFHack execute in an isolated process to strictly
//! preserve memory safety (`unsafe_code = forbid`) and eliminate C/C++ FFI hazards
//! in the Rust trust domain (INV-001).
//!
//! ## Frame Format (Big-Endian)
//! - 4 bytes: `payload_length` (`u32`, max 16MB)
//! - 2 bytes: `message_type` (`u16`, `IpcMessageType`)
//! - 4 bytes: `crc32` (`u32`, CRC-32 checksum of payload bytes)
//! - `payload_length` bytes: Protobuf / payload data

use std::io::{Read, Write};

use dfmcp_core::{DfmcpError, ErrorCode, Result};

/// Maximum allowed payload size in bytes (16MB).
pub const MAX_FRAME_PAYLOAD_SIZE: usize = 16 * 1024 * 1024;

/// Header size: 4 (len) + 2 (type) + 4 (crc32) = 10 bytes.
pub const FRAME_HEADER_SIZE: usize = 10;

/// Big-endian IPC message type discriminator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
pub enum IpcMessageType {
    HandshakeRequest = 1,
    HandshakeResponse = 2,
    HealthRequest = 3,
    HealthResponse = 4,
    ProbeCompatibilityRequest = 5,
    ProbeCompatibilityResponse = 6,
    ReadSnapshotRequest = 7,
    ReadSnapshotResponse = 8,
    ReadDeltaRequest = 9,
    ReadDeltaResponse = 10,
    PrepareMutationRequest = 11,
    PrepareMutationResponse = 12,
    CommitMutationRequest = 13,
    CommitMutationResponse = 14,
    LookupOperationRequest = 15,
    LookupOperationResponse = 16,
    CancelOperationRequest = 17,
    CancelOperationResponse = 18,
    CreateCheckpointRequest = 19,
    CreateCheckpointResponse = 20,
    RestoreCheckpointRequest = 21,
    RestoreCheckpointResponse = 22,
    Heartbeat = 23,
    ErrorResponse = 24,
    Custom(u16),
}

impl IpcMessageType {
    #[must_use]
    pub const fn from_u16(val: u16) -> Self {
        match val {
            1 => Self::HandshakeRequest,
            2 => Self::HandshakeResponse,
            3 => Self::HealthRequest,
            4 => Self::HealthResponse,
            5 => Self::ProbeCompatibilityRequest,
            6 => Self::ProbeCompatibilityResponse,
            7 => Self::ReadSnapshotRequest,
            8 => Self::ReadSnapshotResponse,
            9 => Self::ReadDeltaRequest,
            10 => Self::ReadDeltaResponse,
            11 => Self::PrepareMutationRequest,
            12 => Self::PrepareMutationResponse,
            13 => Self::CommitMutationRequest,
            14 => Self::CommitMutationResponse,
            15 => Self::LookupOperationRequest,
            16 => Self::LookupOperationResponse,
            17 => Self::CancelOperationRequest,
            18 => Self::CancelOperationResponse,
            19 => Self::CreateCheckpointRequest,
            20 => Self::CreateCheckpointResponse,
            21 => Self::RestoreCheckpointRequest,
            22 => Self::RestoreCheckpointResponse,
            23 => Self::Heartbeat,
            24 => Self::ErrorResponse,
            other => Self::Custom(other),
        }
    }

    #[must_use]
    pub const fn as_u16(self) -> u16 {
        match self {
            Self::HandshakeRequest => 1,
            Self::HandshakeResponse => 2,
            Self::HealthRequest => 3,
            Self::HealthResponse => 4,
            Self::ProbeCompatibilityRequest => 5,
            Self::ProbeCompatibilityResponse => 6,
            Self::ReadSnapshotRequest => 7,
            Self::ReadSnapshotResponse => 8,
            Self::ReadDeltaRequest => 9,
            Self::ReadDeltaResponse => 10,
            Self::PrepareMutationRequest => 11,
            Self::PrepareMutationResponse => 12,
            Self::CommitMutationRequest => 13,
            Self::CommitMutationResponse => 14,
            Self::LookupOperationRequest => 15,
            Self::LookupOperationResponse => 16,
            Self::CancelOperationRequest => 17,
            Self::CancelOperationResponse => 18,
            Self::CreateCheckpointRequest => 19,
            Self::CreateCheckpointResponse => 20,
            Self::RestoreCheckpointRequest => 21,
            Self::RestoreCheckpointResponse => 22,
            Self::Heartbeat => 23,
            Self::ErrorResponse => 24,
            Self::Custom(other) => other,
        }
    }
}

/// Compute standard IEEE 802.3 CRC-32 checksum in pure safe Rust.
#[must_use]
pub fn compute_crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

/// Framed binary IPC message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IpcFrame {
    pub message_type: IpcMessageType,
    pub payload: Vec<u8>,
}

impl IpcFrame {
    /// Create a new IPC frame with the given message type and payload.
    pub fn new(message_type: IpcMessageType, payload: Vec<u8>) -> Result<Self> {
        if payload.len() > MAX_FRAME_PAYLOAD_SIZE {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "IPC frame payload exceeds maximum allowed size of 16MB",
            ));
        }
        Ok(Self {
            message_type,
            payload,
        })
    }

    /// Encode this frame into a byte vector with length prefix, message type, and CRC32.
    pub fn encode(&self) -> Result<Vec<u8>> {
        if self.payload.len() > MAX_FRAME_PAYLOAD_SIZE {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "IPC frame payload exceeds maximum allowed size of 16MB",
            ));
        }
        let payload_len = self.payload.len() as u32;
        let crc = compute_crc32(&self.payload);
        let mut bytes = Vec::with_capacity(FRAME_HEADER_SIZE + self.payload.len());
        bytes.extend_from_slice(&payload_len.to_be_bytes());
        bytes.extend_from_slice(&self.message_type.as_u16().to_be_bytes());
        bytes.extend_from_slice(&crc.to_be_bytes());
        bytes.extend_from_slice(&self.payload);
        Ok(bytes)
    }

    /// Write this encoded frame directly to a stream.
    pub fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        let encoded = self.encode()?;
        writer.write_all(&encoded).map_err(|e| {
            DfmcpError::new(
                ErrorCode::AdapterUnavailable,
                format!("failed to write IPC frame to socket: {e}"),
            )
        })?;
        writer.flush().map_err(|e| {
            DfmcpError::new(
                ErrorCode::AdapterUnavailable,
                format!("failed to flush IPC stream: {e}"),
            )
        })?;
        Ok(())
    }

    /// Read and decode exactly one complete frame from a synchronous reader.
    pub fn read_from<R: Read>(reader: &mut R) -> Result<Self> {
        let mut header = [0u8; FRAME_HEADER_SIZE];
        reader.read_exact(&mut header).map_err(|e| {
            DfmcpError::new(
                ErrorCode::AdapterUnavailable,
                format!("failed to read IPC frame header: {e}"),
            )
        })?;

        let payload_len = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        let msg_type_raw = u16::from_be_bytes([header[4], header[5]]);
        let expected_crc = u32::from_be_bytes([header[6], header[7], header[8], header[9]]);

        if payload_len > MAX_FRAME_PAYLOAD_SIZE {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                format!("IPC frame payload length {payload_len} exceeds 16MB limit"),
            ));
        }

        let mut payload = vec![0u8; payload_len];
        if payload_len > 0 {
            reader.read_exact(&mut payload).map_err(|e| {
                DfmcpError::new(
                    ErrorCode::AdapterUnavailable,
                    format!("failed to read IPC frame payload: {e}"),
                )
            })?;
        }

        let actual_crc = compute_crc32(&payload);
        if actual_crc != expected_crc {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                format!(
                    "IPC frame CRC32 mismatch: expected {expected_crc:#010x}, calculated {actual_crc:#010x}"
                ),
            ));
        }

        Ok(Self {
            message_type: IpcMessageType::from_u16(msg_type_raw),
            payload,
        })
    }
}

/// Incremental buffer-based frame decoder for non-blocking or chunked streams.
#[derive(Debug, Default)]
pub struct IncrementalFrameDecoder {
    buffer: Vec<u8>,
}

impl IncrementalFrameDecoder {
    #[must_use]
    pub fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(4096),
        }
    }

    /// Push fresh chunk data into the internal stream buffer.
    pub fn push_bytes(&mut self, chunk: &[u8]) {
        self.buffer.extend_from_slice(chunk);
    }

    /// Attempt to decode the next available frame in the buffer.
    /// Returns `Ok(Some(frame))` if a complete valid frame is found,
    /// `Ok(None)` if more bytes are needed,
    /// or `Err` if the stream header or CRC32 is corrupt.
    pub fn poll_next_frame(&mut self) -> Result<Option<IpcFrame>> {
        if self.buffer.len() < FRAME_HEADER_SIZE {
            return Ok(None);
        }

        let payload_len = u32::from_be_bytes([
            self.buffer[0],
            self.buffer[1],
            self.buffer[2],
            self.buffer[3],
        ]) as usize;

        if payload_len > MAX_FRAME_PAYLOAD_SIZE {
            self.buffer.clear();
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                format!("IPC frame payload length {payload_len} exceeds 16MB limit"),
            ));
        }

        let total_frame_len = FRAME_HEADER_SIZE + payload_len;
        if self.buffer.len() < total_frame_len {
            return Ok(None);
        }

        let msg_type_raw = u16::from_be_bytes([self.buffer[4], self.buffer[5]]);
        let expected_crc = u32::from_be_bytes([
            self.buffer[6],
            self.buffer[7],
            self.buffer[8],
            self.buffer[9],
        ]);

        let payload = self.buffer[FRAME_HEADER_SIZE..total_frame_len].to_vec();
        let actual_crc = compute_crc32(&payload);

        if actual_crc != expected_crc {
            self.buffer.drain(..total_frame_len);
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                format!(
                    "IPC frame CRC32 mismatch: expected {expected_crc:#010x}, calculated {actual_crc:#010x}"
                ),
            ));
        }

        self.buffer.drain(..total_frame_len);

        Ok(Some(IpcFrame {
            message_type: IpcMessageType::from_u16(msg_type_raw),
            payload,
        }))
    }

    /// Number of unprocessed bytes currently held in buffer.
    #[must_use]
    pub fn buffered_len(&self) -> usize {
        self.buffer.len()
    }

    /// Clear all unconsumed bytes in the buffer.
    pub fn clear(&mut self) {
        self.buffer.clear();
    }
}

/// Telemetry metrics for IPC transceiver sessions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IpcTelemetry {
    pub frames_sent: u64,
    pub frames_received: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub crc_errors: u64,
    pub reconnect_attempts: u32,
}

/// Connection lifecycle states for out-of-process bridge IPC.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IpcConnectionState {
    Disconnected,
    Connecting { attempt: u32, backoff_millis: u64 },
    Connected { target_path: String },
    Degraded { reason: String },
    Closed,
}

/// Reconnection policy manager enforcing exponential backoff with jitter bounds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReconnectionPolicy {
    pub initial_backoff_millis: u64,
    pub max_backoff_millis: u64,
    pub max_attempts: u32,
}

impl Default for ReconnectionPolicy {
    fn default() -> Self {
        Self {
            initial_backoff_millis: 100,
            max_backoff_millis: 5000,
            max_attempts: 10,
        }
    }
}

impl ReconnectionPolicy {
    /// Compute backoff delay for attempt `attempt` (1-indexed).
    #[must_use]
    pub fn backoff_for_attempt(&self, attempt: u32) -> u64 {
        if attempt == 0 {
            return 0;
        }
        let shift = (attempt - 1).min(16);
        let multiplier = 1u64.checked_shl(shift).unwrap_or(u64::MAX);
        let raw = self.initial_backoff_millis.saturating_mul(multiplier);
        raw.min(self.max_backoff_millis)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_crc32_standard_vector() {
        // Standard check: CRC-32 for b"123456789" is 0xCBF43926
        let vector = b"123456789";
        let crc = compute_crc32(vector);
        assert_eq!(crc, 0xCBF4_3926);
    }

    #[test]
    fn test_crc32_empty_data() {
        let crc = compute_crc32(b"");
        assert_eq!(crc, 0x0000_0000);
    }

    #[test]
    fn test_frame_encode_decode_round_trip() -> Result<()> {
        let payload = b"DFHack out-of-process bridge payload test".to_vec();
        let frame = IpcFrame::new(IpcMessageType::HandshakeRequest, payload.clone())?;
        let encoded = frame.encode()?;

        let mut cursor = Cursor::new(encoded);
        let decoded = IpcFrame::read_from(&mut cursor)?;

        assert_eq!(decoded.message_type, IpcMessageType::HandshakeRequest);
        assert_eq!(decoded.payload, payload);
        Ok(())
    }

    #[test]
    fn test_frame_corrupt_crc_rejection() -> Result<()> {
        let payload = b"verified data".to_vec();
        let frame = IpcFrame::new(IpcMessageType::ReadSnapshotResponse, payload)?;
        let mut encoded = frame.encode()?;

        // Corrupt a payload byte
        let last_idx = encoded.len() - 1;
        encoded[last_idx] ^= 0xFF;

        let mut cursor = Cursor::new(encoded);
        let result = IpcFrame::read_from(&mut cursor);
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn test_incremental_frame_decoder_fragmentation() -> Result<()> {
        let payload1 = b"first message chunk".to_vec();
        let payload2 = b"second message chunk".to_vec();

        let frame1 = IpcFrame::new(IpcMessageType::HealthRequest, payload1.clone())?;
        let frame2 = IpcFrame::new(IpcMessageType::HealthResponse, payload2.clone())?;

        let mut stream_bytes = frame1.encode()?;
        stream_bytes.extend_from_slice(&frame2.encode()?);

        let mut decoder = IncrementalFrameDecoder::new();

        // Feed bytes 3 at a time to simulate fragmented network delivery
        let mut decoded_frames = Vec::new();
        for chunk in stream_bytes.chunks(3) {
            decoder.push_bytes(chunk);
            while let Some(frame) = decoder.poll_next_frame()? {
                decoded_frames.push(frame);
            }
        }

        assert_eq!(decoded_frames.len(), 2);
        assert_eq!(
            decoded_frames[0].message_type,
            IpcMessageType::HealthRequest
        );
        assert_eq!(decoded_frames[0].payload, payload1);
        assert_eq!(
            decoded_frames[1].message_type,
            IpcMessageType::HealthResponse
        );
        assert_eq!(decoded_frames[1].payload, payload2);
        assert_eq!(decoder.buffered_len(), 0);

        Ok(())
    }

    #[test]
    fn test_reconnection_policy_backoff_clamping() {
        let policy = ReconnectionPolicy {
            initial_backoff_millis: 100,
            max_backoff_millis: 5000,
            max_attempts: 10,
        };

        assert_eq!(policy.backoff_for_attempt(1), 100);
        assert_eq!(policy.backoff_for_attempt(2), 200);
        assert_eq!(policy.backoff_for_attempt(3), 400);
        assert_eq!(policy.backoff_for_attempt(4), 800);
        assert_eq!(policy.backoff_for_attempt(5), 1600);
        assert_eq!(policy.backoff_for_attempt(6), 3200);
        assert_eq!(policy.backoff_for_attempt(7), 5000); // Clamped at 5000
        assert_eq!(policy.backoff_for_attempt(10), 5000);
    }
}
