#![forbid(unsafe_code)]

pub mod announcement_wire;
pub mod api;
pub mod delta_scanner;
pub mod dfhack_probe;
/// Compatibility path for callers that imported the first unqualified client
/// module. It contains no implementation; the audited wire is the sole source.
#[doc(hidden)]
pub mod dfhack_rpc {
    pub use crate::dfhack_wire::*;
}
pub mod dfhack_wire;
pub mod dfhack_wire_v1_1;
pub mod dispatcher;
pub mod fenced_live_source;
pub mod fenced_live_source_v1_1;
/// Legacy process-local framing laboratory. This is not the live DFHack wire.
pub mod ipc;
pub mod legacy_bridge_probe;
pub mod live_adapter;
pub mod live_adapter_v1_1;
pub mod live_announcement_batch;
pub mod live_announcement_briefing;
pub mod live_announcement_projection;
pub mod live_bootstrap;
pub mod live_bootstrap_v1_1;
pub mod live_briefing;
pub mod live_compatibility;
pub mod live_connect;
pub mod live_connect_v1_1;
pub mod live_evidence;
pub mod live_identity;
pub mod live_observation;
pub mod live_observation_publication_v1_1;
pub mod live_observation_v1_1;
pub mod live_projection;
pub mod live_projection_v1_1;
pub mod live_session;
pub mod live_session_v1_1;
pub mod live_version;
pub mod transceiver;

pub use announcement_wire::{
    ANNOUNCEMENT_AFTER_ID_FIELD, ANNOUNCEMENT_COMPLETE_THROUGH_LATEST_FIELD,
    ANNOUNCEMENT_GAP_BEFORE_WINDOW_FIELD, ANNOUNCEMENT_LATEST_AVAILABLE_ID_FIELD,
    ANNOUNCEMENT_OLDEST_AVAILABLE_ID_FIELD, ANNOUNCEMENT_RECORD_FIELD,
    ANNOUNCEMENT_REQUESTED_AFTER_ID_FIELD, MAX_ANNOUNCEMENTS_FIELD,
    decode_announcement_reply_fields, encode_announcement_request_fields,
};
pub use api::*;
pub use delta_scanner::{
    ContinuousDeltaStreamer, DirtyChunkTracker, EntityDeltaTracker, EventRingBuffer,
    MAX_EVENT_BUFFER_CAPACITY,
};
pub use dfhack_probe::{
    DfHackProbeClient, MAX_PROBE_FIELD_BYTES, MAX_PROBE_METHODS,
    MAX_PROBE_TEXT_NOTIFICATION_BYTES, ProbeHandshakeReply, ProbeHandshakeRequest,
    ProbeObservationReply, ProbeObservationRequest,
};
pub use dfhack_wire::{
    BRIDGE_PROTOCOL_MAJOR, BRIDGE_PROTOCOL_MINOR, BridgeCredentials, BridgeManifest,
    CitizenRecord, DFHACK_RPC_VERSION, DfHackRpcClient, MAX_CITIZENS_PER_PAGE,
    MAX_CLIENT_NAME_BYTES, MAX_CLIENT_VERSION_BYTES, MAX_RACE_NAME_BYTES,
    MAX_RPC_PAYLOAD_BYTES, MAX_TEXT_NOTIFICATIONS_PER_CALL,
    MAX_TEXT_NOTIFICATION_TOTAL_BYTES, MAX_UNIT_NAME_BYTES, MAX_WORLD_FOLDER_BYTES,
    MAX_WORLD_NAME_BYTES, ObservationPage,
};
pub use dfhack_wire_v1_1::{
    BRIDGE_PROTOCOL_V1_1_MAJOR, BRIDGE_PROTOCOL_V1_1_MINOR, BridgeCredentialsV1_1,
    DFHACK_RPC_V1_1_VERSION, DfHackRpcClientV1_1, MAX_RPC_V1_1_PAYLOAD_BYTES,
    MAX_V1_1_BRIDGE_TOKEN_BYTES, MAX_V1_1_CITIZENS_PER_PAGE,
    MAX_V1_1_CLIENT_NAME_BYTES, MAX_V1_1_CLIENT_VERSION_BYTES,
    MAX_V1_1_NONCE_BYTES, MAX_V1_1_RACE_NAME_BYTES,
    MAX_V1_1_TEXT_NOTIFICATION_BYTES, MAX_V1_1_TEXT_NOTIFICATIONS_PER_CALL,
    MAX_V1_1_TEXT_NOTIFICATION_TOTAL_BYTES, MAX_V1_1_UNIT_NAME_BYTES,
    MAX_V1_1_WORLD_FOLDER_BYTES, MAX_V1_1_WORLD_NAME_BYTES,
    MIN_V1_1_BRIDGE_TOKEN_BYTES, MIN_V1_1_NONCE_BYTES, ObservationPageV1_1,
};
pub use dispatcher::{EffectJournal, EffectJournalRecord, MutationDispatcher};
pub use fenced_live_source::FencedLiveSource;
pub use fenced_live_source_v1_1::FencedLiveSourceV1_1;
pub use ipc::{
    FRAME_HEADER_SIZE, IncrementalFrameDecoder, IpcConnectionState, IpcFrame, IpcMessageType,
    IpcTelemetry, MAX_FRAME_PAYLOAD_SIZE, ReconnectionPolicy, compute_crc32,
};
pub use legacy_bridge_probe::{LegacyBridgeProbeAdapter, LegacyBridgeProbeConfig};
pub use live_adapter::{LiveReadAdapter, LiveReadAdapterConfig};
pub use live_adapter_v1_1::{LiveReadAdapterConfigV1_1, LiveReadAdapterV1_1};
pub use live_announcement_batch::{
    AnnouncementBatchRecord, AnnouncementContinuity, AnnouncementCoverage,
    AnnouncementReplyContext, LiveAnnouncementBatch, MAX_ANNOUNCEMENTS_PER_BATCH,
    MAX_ANNOUNCEMENT_TEXT_BYTES, MAX_CANONICAL_ANNOUNCEMENT_BATCH_BYTES,
};
pub use live_announcement_briefing::{
    AnnouncementAttentionItem, AnnouncementAttentionSeverity, LiveAnnouncementBriefing,
    LiveAnnouncementChangeSummary, MAX_ANNOUNCEMENT_ATTENTION_ITEMS,
    MAX_ANNOUNCEMENT_BRIEFING_RECORDS, MAX_ANNOUNCEMENT_CHANGE_IDS,
    build_live_announcement_briefing, summarize_live_announcement_change,
};
pub use live_announcement_projection::{
    LiveAnnouncementProjection, announcement_entity_id_to_report_id,
    project_live_announcement_batch, report_id_to_announcement_entity_id,
};
pub use live_bootstrap::{
    DEFAULT_MAX_LIVE_CITIZENS, LiveReadBootstrapConfig, PrimedLiveSource,
    bootstrap_live_read_adapter,
};
pub use live_bootstrap_v1_1::{
    DEFAULT_LIVE_ANNOUNCEMENT_PAGE_SIZE, DEFAULT_MAX_LIVE_ANNOUNCEMENTS,
    LiveReadBootstrapConfigV1_1, PrimedLiveSourceV1_1,
    bootstrap_live_read_adapter_v1_1,
};
pub use live_briefing::{
    CitizenStatusCounts, LiveAttentionItem, LiveAttentionSeverity, LiveChangeSummary,
    LiveCoverageDomain, LiveCoverageEntry, LiveCoverageStatus, LiveFortressBriefing,
    MAX_BRIEFING_ATTENTION_ITEMS, MAX_BRIEFING_CHANGE_IDS, build_live_briefing,
    summarize_live_change,
};
pub use live_compatibility::{LiveCompatibilityPolicy, LiveCompatibilityVerdict};
pub use live_connect::{
    AuthenticatedLiveSource, LiveConnectionConfig, MAX_ENDPOINT_BYTES,
    MAX_SOCKET_TIMEOUT_MILLIS, connect_authenticated_live_source, parse_loopback_endpoint,
};
pub use live_connect_v1_1::{
    AuthenticatedLiveSourceV1_1, connect_authenticated_live_source_v1_1,
};
pub use live_evidence::LiveObservationReceipt;
pub use live_identity::derive_live_fortress_id;
pub use live_observation::{
    CitizenCoverage, LiveObservationCapsule, MAX_CANONICAL_CAPSULE_BYTES,
    MAX_CAPSULE_CITIZENS, ObservationAssembler,
};
pub use live_observation_publication_v1_1::{
    LiveObservationPublicationConfigV1_1, read_publishable_observation_v1_1,
};
pub use live_observation_v1_1::{
    LiveObservationCapsuleV1_1, MAX_CANONICAL_CAPSULE_V1_1_BYTES,
    ObservationAssemblerV1_1,
};
pub use live_projection::{
    DAYS_PER_MONTH, DwarfFortressClock, FORTRESS_ENTITY_ID, LIVE_PROJECTION_SCHEMA,
    LiveProjectionReceipt, LiveWorldProjection, MONTHS_PER_YEAR, TICKS_PER_DAY, TICKS_PER_YEAR,
    entity_id_to_raw_unit_id, project_live_capsule, raw_unit_id_to_entity_id,
};
pub use live_projection_v1_1::{
    LIVE_PROJECTION_V1_1_SCHEMA, LiveProjectionReceiptV1_1, LiveWorldProjectionV1_1,
    project_live_capsule_v1_1,
};
pub use live_session::{
    LiveObservationSource, read_complete_observation, read_complete_observation_bounded,
};
pub use live_session_v1_1::{
    LiveObservationSourceV1_1, read_complete_observation_v1_1,
    read_complete_observation_v1_1_bounded,
};
pub use live_version::{
    LiveCompatibilityIdentity, LiveEpochResetReason, LiveVersionDecision, LiveVersionTracker,
    LiveWorldIdentity,
};
pub use transceiver::{IpcTransceiver, TransceiverConfig};
