#![forbid(unsafe_code)]

pub use dfmcp_adapter::{
    ActionReceipt, AdapterHealth, AdapterIdentity, BRIDGE_PROTOCOL_MAJOR, BRIDGE_PROTOCOL_MINOR,
    BridgeManifest, CancelMode, CancelReceipt, CheckpointReceipt, CitizenRecord, CommitReceipt,
    CompatibilityLevel, dfhack_rpc, DwarfFortressClock, GameAdapter, HealthStatus, InterestSet,
    MAX_CITIZENS_PER_PAGE, ObservationFrame, ObservationPage, ObservationPayload,
    ObservationRequest, PrepareReceipt, Projection, QueryRequest, QueryResponse, QueryRow,
    RestoreReceipt, TICKS_PER_YEAR,
};

#[path = "../src/live_observation.rs"]
pub mod live_observation;
pub use live_observation::{LiveObservationCapsule, MAX_CAPSULE_CITIZENS, ObservationAssembler};

#[path = "../src/live_session.rs"]
pub mod live_session;
pub use live_session::LiveObservationSource;

#[path = "../src/live_projection.rs"]
pub mod live_projection;
pub use live_projection::{LiveWorldProjection, project_live_capsule};

#[path = "../src/live_adapter.rs"]
pub mod live_adapter;

pub use live_session::{read_complete_observation, read_complete_observation_bounded};

use live_adapter::{DWARF_FORTRESS_TICKS_PER_YEAR, LiveReadAdapterConfig};

#[test]
fn calendar_constant_matches_the_protocol_contract() {
    assert_eq!(DWARF_FORTRESS_TICKS_PER_YEAR, 403_200);
}

#[test]
fn live_adapter_config_rejects_zero_page_size() {
    let config = LiveReadAdapterConfig {
        fortress_id: dfmcp_core::FortressId::new(1),
        page_size: 0,
        max_citizens: 1000,
        include_names: true,
        initial_epoch: 0,
    };
    assert!(config.validate().is_err());
}
