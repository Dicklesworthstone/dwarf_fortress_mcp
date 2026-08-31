#![forbid(unsafe_code)]

//! Unit and integration tests for WP-DFH-01 framed binary IPC transceiver.

use std::io::Cursor;

use dfmcp_adapter::ipc::{
    FRAME_HEADER_SIZE, IncrementalFrameDecoder, IpcFrame, IpcMessageType, MAX_FRAME_PAYLOAD_SIZE,
    ReconnectionPolicy, compute_crc32,
};
use dfmcp_core::Result;

#[test]
fn test_crc32_calculation_vectors() {
    assert_eq!(compute_crc32(b""), 0);
    assert_eq!(compute_crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(compute_crc32(b"Dwarf Fortress MCP"), 0x437B_7980);
}

#[test]
fn test_all_ipc_message_types_round_trip() -> Result<()> {
    let message_types = [
        IpcMessageType::HandshakeRequest,
        IpcMessageType::HandshakeResponse,
        IpcMessageType::HealthRequest,
        IpcMessageType::HealthResponse,
        IpcMessageType::ProbeCompatibilityRequest,
        IpcMessageType::ProbeCompatibilityResponse,
        IpcMessageType::ReadSnapshotRequest,
        IpcMessageType::ReadSnapshotResponse,
        IpcMessageType::ReadDeltaRequest,
        IpcMessageType::ReadDeltaResponse,
        IpcMessageType::PrepareMutationRequest,
        IpcMessageType::PrepareMutationResponse,
        IpcMessageType::CommitMutationRequest,
        IpcMessageType::CommitMutationResponse,
        IpcMessageType::LookupOperationRequest,
        IpcMessageType::LookupOperationResponse,
        IpcMessageType::CancelOperationRequest,
        IpcMessageType::CancelOperationResponse,
        IpcMessageType::CreateCheckpointRequest,
        IpcMessageType::CreateCheckpointResponse,
        IpcMessageType::RestoreCheckpointRequest,
        IpcMessageType::RestoreCheckpointResponse,
        IpcMessageType::Heartbeat,
        IpcMessageType::ErrorResponse,
        IpcMessageType::Custom(999),
    ];

    for msg_type in message_types {
        let payload = format!("test payload for {:?}", msg_type).into_bytes();
        let frame = IpcFrame::new(msg_type, payload.clone())?;
        let encoded = frame.encode()?;

        assert_eq!(encoded.len(), FRAME_HEADER_SIZE + payload.len());

        let mut cursor = Cursor::new(encoded);
        let decoded = IpcFrame::read_from(&mut cursor)?;

        assert_eq!(decoded.message_type, msg_type);
        assert_eq!(decoded.payload, payload);
    }
    Ok(())
}

#[test]
fn test_frame_size_limit_rejection() {
    let huge_payload = vec![0u8; MAX_FRAME_PAYLOAD_SIZE + 1];
    let result = IpcFrame::new(IpcMessageType::ReadSnapshotResponse, huge_payload);
    assert!(result.is_err());
}

#[test]
fn test_corrupt_payload_crc_detection() -> Result<()> {
    let payload = b"critical fortress mutation data".to_vec();
    let frame = IpcFrame::new(IpcMessageType::CommitMutationRequest, payload)?;
    let mut encoded = frame.encode()?;

    // Modify a payload byte
    let payload_byte_idx = FRAME_HEADER_SIZE + 2;
    encoded[payload_byte_idx] ^= 0x01;

    let mut cursor = Cursor::new(encoded);
    let result = IpcFrame::read_from(&mut cursor);
    assert!(result.is_err());
    Ok(())
}

#[test]
fn test_incremental_decoder_chunk_assembly() -> Result<()> {
    let mut frames = Vec::new();
    let mut expected_bytes = Vec::new();

    for i in 0..10 {
        let payload = format!("frame payload number {}", i).into_bytes();
        let frame = IpcFrame::new(IpcMessageType::from_u16((i + 1) as u16), payload)?;
        let encoded = frame.encode()?;
        expected_bytes.extend_from_slice(&encoded);
        frames.push(frame);
    }

    let mut decoder = IncrementalFrameDecoder::new();
    let mut decoded_frames = Vec::new();

    // Push in small 7-byte chunks
    for chunk in expected_bytes.chunks(7) {
        decoder.push_bytes(chunk)?;
        while let Some(frame) = decoder.poll_next_frame()? {
            decoded_frames.push(frame);
        }
    }

    assert_eq!(decoded_frames.len(), frames.len());
    for (actual, expected) in decoded_frames.iter().zip(frames.iter()) {
        assert_eq!(actual.message_type, expected.message_type);
        assert_eq!(actual.payload, expected.payload);
    }
    assert_eq!(decoder.buffered_len(), 0);

    Ok(())
}

#[test]
fn test_reconnection_policy_progression() {
    let policy = ReconnectionPolicy {
        initial_backoff_millis: 50,
        max_backoff_millis: 1000,
        max_attempts: 8,
    };

    assert_eq!(policy.backoff_for_attempt(0), 0);
    assert_eq!(policy.backoff_for_attempt(1), 50);
    assert_eq!(policy.backoff_for_attempt(2), 100);
    assert_eq!(policy.backoff_for_attempt(3), 200);
    assert_eq!(policy.backoff_for_attempt(4), 400);
    assert_eq!(policy.backoff_for_attempt(5), 800);
    assert_eq!(policy.backoff_for_attempt(6), 1000); // clamped at 1000
    assert_eq!(policy.backoff_for_attempt(7), 1000);
}
