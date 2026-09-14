use dfmcp_adapter::live_map::{LiveMapObservation,LiveMapState,tile_entity_id};
use dfmcp_adapter::live_jobs::JobPublication;
use dfmcp_core::{DfmcpError,ErrorCode,Result};
use dfmcp_world::{FactPresence,Value};
use dfmcp_world::map_region::Cell;
fn fixture()->Result<LiveMapObservation>{let h=include_str!("fixtures/map_v1_5.hex").trim();
    let b=(0..h.len()).step_by(2).map(|i|u8::from_str_radix(&h[i..i+2],16)
        .map_err(|_|DfmcpError::new(ErrorCode::InvalidRequest,"bad fixture"))).collect::<Result<Vec<_>>>()?;
    LiveMapObservation::decode_payload(&b,7,"df".to_owned(),"dfhack".to_owned())}
#[test]
fn native_golden_roundtrips_and_all_truncated_prefixes_fail()->Result<()>{
    let v=fixture()?;let b=v.encode_payload()?;assert_eq!(b.len(),455);
    for n in 0..b.len(){assert!(LiveMapObservation::decode_payload(&b[..n],7,"df".to_owned(),"dfhack".to_owned()).is_err());}
    assert_eq!(LiveMapObservation::decode_payload(&b,7,"df".to_owned(),"dfhack".to_owned())?,v);
    let mut extra=b;extra.push(0);assert!(LiveMapObservation::decode_payload(&extra,7,"df".to_owned(),"dfhack".to_owned()).is_err());Ok(())
}
#[test]
fn hidden_and_unallocated_project_as_nonknowledge_not_empty_floor()->Result<()>{
    let v=fixture()?;let mut state=LiveMapState::default();state.publish(v.clone())?;
    let snapshot=state.snapshot().ok_or_else(||DfmcpError::new(ErrorCode::InternalInvariantViolation,"fixture"))?;
    let hidden=&snapshot.graph.entities[&tile_entity_id([15,15,1])?];
    assert!(matches!(hidden.fields["shape"].presence,Some(FactPresence::Redacted(_))));
    assert_eq!(hidden.fields["shape"].value,Value::Null);
    let missing=&snapshot.graph.entities[&tile_entity_id([16,16,2])?];
    assert!(matches!(missing.fields["shape"].presence,Some(FactPresence::Unknown(_))));
    assert_eq!(v.map.cells.iter().filter(|c|**c==Cell::Hidden).count(),1);
    assert_eq!(v.map.cells.iter().filter(|c|**c==Cell::Unallocated).count(),4);Ok(())
}
#[test]
fn publication_is_atomic_region_bound_and_epoch_aware()->Result<()>{
    let mut v=fixture()?;let mut state=LiveMapState::default();assert_eq!(state.publish(v.clone())?,JobPublication::Bootstrap);
    let before=state.snapshot().cloned();assert_eq!(state.publish(v.clone())?,JobPublication::Heartbeat);
    let mut bad=v.clone();bad.map.cells.pop();assert!(state.publish(bad).is_err());assert_eq!(state.snapshot(),before.as_ref());
    let mut other=v.clone();other.map.region.origin[0]-=1;assert!(state.publish(other).is_err());
    v.year_tick+=1;assert_eq!(state.publish(v.clone())?,JobPublication::Advanced);
    v.bridge_generation+=1;assert_eq!(state.publish(v)?,JobPublication::Reset);
    let current=state.snapshot().ok_or_else(||DfmcpError::new(ErrorCode::InternalInvariantViolation,"fixture"))?;
    assert_eq!(current.cursor.epoch,1);assert_eq!(current.cursor.sequence,0);
    assert_eq!(current.graph.entities[&tile_entity_id([14,15,1])?].generation,2);Ok(())
}
