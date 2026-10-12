//! Independently assembled citizen component through the production native codec.
use dfmcp_adapter::live_spatial::{
    LiveSpatialObservation, SpatialStateView,
    citizens::{LiveSpatialCitizenObservation, LiveSpatialCitizenState, citizen_entity_id},
};
use dfmcp_adapter::live_operations::OperationsProfile;
use dfmcp_adapter::shelter_analysis::{CitizenTarget, REQUIRED_BYTES, ShelterSelection};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, DfmcpError, ErrorCode, MapCoord, MapCuboid,
    OperationContext, RequestId, Result, RiskTier, SessionId, WorkBudget,
};

pub fn error(message: &str) -> DfmcpError { DfmcpError::new(ErrorCode::InvalidRequest, message) }
fn put(out: &mut Vec<u8>, n: u32) { out.extend_from_slice(&n.to_be_bytes()); }
fn text(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u16).to_be_bytes()); out.extend_from_slice(s.as_bytes());
}
pub fn observation(rows: &[(u32, [i32; 3], u16)], tick: u32, generation: u64) -> Result<LiveSpatialCitizenObservation> {
    let hex = include_str!("../fixtures/spatial_v1_6.hex").trim();
    let raw = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16)
        .map_err(|_| error("fixture hex"))).collect::<Result<Vec<_>>>()?;
    let base = LiveSpatialObservation::decode_payload(&raw, generation, "df".into(), "dfhack".into())?;
    let mut operations = base.operations().clone();
    operations.jobs.year_tick += tick;
    let mut terrain = base.terrain().clone();
    terrain.year_tick += tick;
    let mut spatial = b"DFMS1600".to_vec();
    for part in [operations.encode_profile(OperationsProfile::PagedV1_4)?, terrain.encode_payload()?] {
        put(&mut spatial, part.len() as u32); spatial.extend_from_slice(&part);
    }
    let mut citizens = b"DFMC1800".to_vec();
    put(&mut citizens, rows.len() as u32);
    for (id, position, flags) in rows {
        put(&mut citizens, *id); text(&mut citizens, "Urist"); text(&mut citizens, "DWARF");
        put(&mut citizens, 0);
        for n in position { put(&mut citizens, *n as u32); }
        citizens.extend_from_slice(&flags.to_be_bytes());
        put(&mut citizens, 0);
        citizens.extend_from_slice(&[0, 0]); // Busy citizens still have observed locations.
        citizens.extend_from_slice(&0u16.to_be_bytes()); // No invented skills.
    }
    let mut full = b"DFMS1800".to_vec();
    for part in [spatial, citizens] { put(&mut full, part.len() as u32); full.extend_from_slice(&part); }
    LiveSpatialCitizenObservation::decode_payload(&full, generation, "df".into(), "dfhack".into())
}
pub fn state(rows: &[(u32, [i32; 3], u16)]) -> Result<LiveSpatialCitizenState> {
    let mut state = LiveSpatialCitizenState::default(); state.publish(observation(rows, 0, 7)?)?; Ok(state)
}
pub fn context(state: &LiveSpatialCitizenState) -> Result<OperationContext> {
    let anchor = state.snapshot().ok_or_else(|| error("fixture snapshot"))?.anchor();
    Ok(OperationContext {
        session_id: SessionId::new(181), request_id: RequestId::new(1), anchor,
        budget: WorkBudget { max_wall_millis: 60_000, max_entities: 100_000,
            max_bytes: REQUIRED_BYTES, max_output_tokens: 65_536, ..WorkBudget::default() },
        grants: vec![CapabilityGrant { capability: Capability::Query, max_risk: RiskTier::ReadOnly,
            scope: CapabilityScope { fortress_id: Some(anchor.fortress_id), ..CapabilityScope::default() },
            expires_at_tick: None, remaining_uses: None }], cancellation_requested: false,
    })
}
pub fn target(native: u32, generation: u32) -> CitizenTarget {
    CitizenTarget { entity_id: citizen_entity_id(native), generation }
}
pub fn shelter() -> Result<MapCuboid> {
    MapCuboid::new(MapCoord::new(0, 0, 5), MapCoord::new(1, 1, 5))
}
pub fn selection(ids: &[u32]) -> Result<ShelterSelection> {
    ShelterSelection::new(&ids.iter().map(|id| target(*id, 1)).collect::<Vec<_>>(), &[shelter()?])
}
