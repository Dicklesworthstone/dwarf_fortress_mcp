//! Synthetic coherent captures derived from the existing fixed wire fixture.
//! No DFHack service, environment mutation or game state is involved.
use dfmcp_adapter::live_operations::OperationsProfile;
use dfmcp_adapter::live_spatial::{LiveSpatialObservation,citizens::LiveSpatialCitizenObservation};
use dfmcp_core::{DfmcpError,ErrorCode,Result};

fn invalid() -> DfmcpError { DfmcpError::new(ErrorCode::InvalidRequest,"production fixture is incomplete") }
fn put(out: &mut Vec<u8>, n: u32) { out.extend_from_slice(&n.to_be_bytes()); }
fn text(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&(value.len() as u16).to_be_bytes());out.extend_from_slice(value.as_bytes());
}
fn part(out: &mut Vec<u8>, bytes: &[u8]) { put(out,bytes.len() as u32);out.extend_from_slice(bytes); }

pub fn observation(tick: u32, jobs: u32, free_units: u32, empty: bool) -> Result<LiveSpatialCitizenObservation> {
    let hex=include_str!("../fixtures/spatial_v1_6.hex").trim();
    let bytes=(0..hex.len()).step_by(2).map(|i|u8::from_str_radix(&hex[i..i+2],16)
        .map_err(|_|invalid())).collect::<Result<Vec<_>>>()?;
    let base=LiveSpatialObservation::decode_payload(&bytes,7,"df".into(),"dfhack".into())?;
    let mut operations=base.operations().clone();let mut terrain=base.terrain().clone();
    operations.jobs.year_tick=tick;terrain.year_tick=tick;
    operations.jobs.paused=true;terrain.paused=true;
    // Empty rosters do not regress allocation counters or invent a restore.
    operations.jobs.next_job_id=10_000;operations.next_item_id=10_000;operations.next_building_id=10_000;
    let prototype=operations.jobs.jobs.first().cloned().ok_or_else(invalid)?;
    let attachment=operations.attachments.first().cloned().ok_or_else(invalid)?;
    operations.jobs.jobs.clear();operations.attachments.clear();
    for index in 0..jobs {
        let mut job=prototype.clone();job.native_id=7+index;job.suspended=index%2==0;
        job.worker_native_id=if index==0 {Some(10)} else {None};
        job.attached_item_count=1;job.required_item_filter_count=1;
        let mut attached=attachment.clone();attached.job_native_id=job.native_id;attached.filter_index=0;
        operations.jobs.jobs.push(job);operations.attachments.push(attached);
    }
    operations.items.iter_mut().find(|item|item.native_id==31).ok_or_else(invalid)?.flags=1;
    let free=operations.items.iter_mut().find(|item|item.native_id==32).ok_or_else(invalid)?;
    free.stack_size=free_units;free.flags=64;free.container_native_id=None;free.holder_building_native_id=None;
    if empty {operations.jobs.jobs.clear();operations.attachments.clear();operations.items.clear();operations.buildings.clear();}
    let mut spatial=b"DFMS1600".to_vec();
    part(&mut spatial,&operations.encode_profile(OperationsProfile::PagedV1_4)?);
    part(&mut spatial,&terrain.encode_payload()?);
    let mut citizens=b"DFMC1800".to_vec();put(&mut citizens,u32::from(!empty));
    if !empty {
        put(&mut citizens,10);text(&mut citizens,"Urist");text(&mut citizens,"DWARF");
        for n in [0,1,2,5] {put(&mut citizens,n);}
        citizens.extend_from_slice(&0x11fu16.to_be_bytes());put(&mut citizens,6);
        citizens.extend_from_slice(&[0,0]);citizens.extend_from_slice(&0u16.to_be_bytes());
    }
    let mut combined=b"DFMS1800".to_vec();part(&mut combined,&spatial);part(&mut combined,&citizens);
    LiveSpatialCitizenObservation::decode_payload(&combined,7,"df".into(),"dfhack".into())
}
