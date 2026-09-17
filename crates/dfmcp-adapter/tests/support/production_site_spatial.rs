//! Two work sites separated by an optional floor-to-ceiling modeled barrier.
//! Exactly two skilled citizens and two finite stacks share one capture.
#[path = "production_portfolio_spatial.rs"]
mod base;
use dfmcp_adapter::live_operations::OperationsProfile;
use dfmcp_adapter::live_spatial::citizens::LiveSpatialCitizenObservation;
use dfmcp_core::{DfmcpError, ErrorCode, Result};
use dfmcp_world::map_region::{Cell, MapRegion, Region, Shape, Tile};

fn put(out: &mut Vec<u8>, n: u32) { out.extend_from_slice(&n.to_be_bytes()); }
fn text(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&(value.len() as u16).to_be_bytes()); out.extend_from_slice(value.as_bytes());
}
fn part(out: &mut Vec<u8>, bytes: &[u8]) { put(out, bytes.len() as u32); out.extend_from_slice(bytes); }
pub fn observation(tick: u32, west: u32, east: u32, separated: bool) -> Result<LiveSpatialCitizenObservation> {
    let prior = base::observation(tick, 2, west, false)?;
    let mut operations = prior.spatial().operations().clone();
    let mut terrain = prior.spatial().terrain().clone();
    let mut other = operations.items.first().cloned().ok_or_else(|| {
        DfmcpError::new(ErrorCode::InvalidRequest, "site fixture stack missing")
    })?;
    other.native_id = 33; other.stack_size = east; other.raw_position.x = 3;
    operations.items.push(other);
    let floor = Tile { native_tiletype: 1, shape: Shape::Floor, liquid_depth: 0,
        magma: false, traffic: 0, dig_designation: 0, building_occupancy: 0,
        unit_occupancy: 0, walkable_region: 1, temperature_1: 10015, temperature_2: 10015 };
    let region = Region { origin: [0,0,5], size: [5,3,1] };
    let mut cells = vec![Cell::Visible(floor); 15];
    if separated {
        for y in 0..3 {
            let index = region.index([2,y,5]).ok_or_else(|| {
                DfmcpError::new(ErrorCode::InvalidRequest, "site fixture coordinate")
            })?;
            cells[index] = Cell::Visible(Tile { shape: Shape::Wall, walkable_region: 0, ..floor });
        }
    }
    terrain.map = MapRegion { region, cells };
    let mut spatial = b"DFMS1600".to_vec();
    part(&mut spatial, &operations.encode_profile(OperationsProfile::PagedV1_4)?);
    part(&mut spatial, &terrain.encode_payload()?);
    let mut citizens = b"DFMC1800".to_vec(); put(&mut citizens, 2);
    for (id, x) in [(10,1), (11,3)] {
        put(&mut citizens, id); text(&mut citizens, "Urist"); text(&mut citizens, "DWARF");
        for n in [0,x,2,5] { put(&mut citizens,n); }
        citizens.extend_from_slice(&0x11fu16.to_be_bytes()); put(&mut citizens,6);
        citizens.extend_from_slice(&[1,1]); citizens.extend_from_slice(&1u16.to_be_bytes());
        put(&mut citizens,0); text(&mut citizens,"CARPENTRY");
        for n in [5,5,1] { put(&mut citizens,n); }
    }
    let mut full = b"DFMS1800".to_vec(); part(&mut full,&spatial); part(&mut full,&citizens);
    LiveSpatialCitizenObservation::decode_payload(&full,7,"df".into(),"dfhack".into())
}
