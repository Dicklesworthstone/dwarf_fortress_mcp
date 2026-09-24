//! Synthetic full captures: available skilled citizens and one ground stack.
//! Existing fixed operations encoding is reused; no native service is involved.
#[path = "production_spatial.rs"]
mod base;
use dfmcp_adapter::live_operations::OperationsProfile;
use dfmcp_adapter::live_spatial::citizens::LiveSpatialCitizenObservation;
use dfmcp_core::{DfmcpError, ErrorCode, Result};
use dfmcp_world::map_region::{Cell, MapRegion, Region, Shape, Tile};

fn put(out: &mut Vec<u8>, n: u32) {
    out.extend_from_slice(&n.to_be_bytes());
}
fn text(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u16).to_be_bytes());
    out.extend_from_slice(s.as_bytes());
}
fn part(out: &mut Vec<u8>, bytes: &[u8]) {
    put(out, bytes.len() as u32);
    out.extend_from_slice(bytes);
}

pub fn observation(
    tick: u32,
    workers: u32,
    units: u32,
    forbidden: bool,
) -> Result<LiveSpatialCitizenObservation> {
    let base = base::observation(tick, 0, units, false)?;
    let mut operations = base.spatial().operations().clone();
    let mut terrain = base.spatial().terrain().clone();
    operations.buildings.clear();
    operations.items.retain(|i| i.native_id == 32);
    let item = operations.items.first_mut().ok_or_else(|| {
        DfmcpError::new(ErrorCode::InvalidRequest, "portfolio fixture item missing")
    })?;
    item.flags = 64 | u32::from(forbidden);
    item.raw_position.x = 1;
    item.raw_position.y = 1;
    item.raw_position.z = 5;
    let tile = Tile {
        native_tiletype: 1,
        shape: Shape::Floor,
        liquid_depth: 0,
        magma: false,
        traffic: 0,
        dig_designation: 0,
        building_occupancy: 0,
        unit_occupancy: 0,
        walkable_region: 1,
        temperature_1: 10015,
        temperature_2: 10015,
    };
    terrain.map = MapRegion {
        region: Region {
            origin: [0, 0, 5],
            size: [3, 3, 1],
        },
        cells: vec![Cell::Visible(tile); 9],
    };
    let mut spatial = b"DFMS1600".to_vec();
    part(
        &mut spatial,
        &operations.encode_profile(OperationsProfile::PagedV1_4)?,
    );
    part(&mut spatial, &terrain.encode_payload()?);
    let mut citizens = b"DFMC1800".to_vec();
    put(&mut citizens, workers);
    for i in 0..workers {
        put(&mut citizens, 10 + i);
        text(&mut citizens, "Urist");
        text(&mut citizens, "DWARF");
        for n in [0, 2, 2, 5] {
            put(&mut citizens, n);
        }
        citizens.extend_from_slice(&0x11fu16.to_be_bytes());
        put(&mut citizens, 6);
        citizens.extend_from_slice(&[1, 1]);
        citizens.extend_from_slice(&1u16.to_be_bytes());
        put(&mut citizens, 0);
        text(&mut citizens, "CARPENTRY");
        for n in [5, 5, 1] {
            put(&mut citizens, n);
        }
    }
    let mut combined = b"DFMS1800".to_vec();
    part(&mut combined, &spatial);
    part(&mut combined, &citizens);
    LiveSpatialCitizenObservation::decode_payload(&combined, 7, "df".into(), "dfhack".into())
}
