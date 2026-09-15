use dfmcp_adapter::live_jobs::JobPublication;
use dfmcp_adapter::live_map::tile_entity_id;
use dfmcp_adapter::live_operations::OperationsProfile;
use dfmcp_adapter::live_spatial::{LiveSpatialObservation,SpatialStateView,citizens::{LiveSpatialCitizenObservation,LiveSpatialCitizenState,citizen_entity_id}};
use dfmcp_world::{EdgeKind,FactPresence,Value};

fn base_fixture()->dfmcp_core::Result<LiveSpatialObservation>{
    let text=include_str!("fixtures/spatial_v1_6.hex").trim();
    let bytes=(0..text.len()).step_by(2).map(|i|u8::from_str_radix(&text[i..i+2],16)
        .map_err(|_|dfmcp_core::DfmcpError::new(dfmcp_core::ErrorCode::InvalidRequest,"fixture hex"))).collect::<dfmcp_core::Result<Vec<_>>>()?;
    LiveSpatialObservation::decode_payload(&bytes,7,"df".to_owned(),"dfhack".to_owned())
}
fn text(out:&mut Vec<u8>,value:&str){out.extend_from_slice(&(value.len() as u16).to_be_bytes());out.extend_from_slice(value.as_bytes());}
fn citizen_component(id:u32,name:&str,position:[i32;3],flags:u16,skills:&[(i32,&str,i32,i32,i32)])->Vec<u8>{
    let mut out=b"DFMC1800".to_vec();out.extend_from_slice(&1u32.to_be_bytes());out.extend_from_slice(&id.to_be_bytes());text(&mut out,name);text(&mut out,"dwarf");
    out.extend_from_slice(&3i32.to_be_bytes());for v in position{out.extend_from_slice(&v.to_be_bytes());}out.extend_from_slice(&flags.to_be_bytes());
    out.extend_from_slice(&(skills.len() as u16).to_be_bytes());for(id,key,nominal,effective,experience)in skills{
        out.extend_from_slice(&id.to_be_bytes());text(&mut out,key);out.extend_from_slice(&nominal.to_be_bytes());out.extend_from_slice(&effective.to_be_bytes());out.extend_from_slice(&experience.to_be_bytes());}out
}
fn combined_with_skills(worker:Option<u32>,citizen_id:u32,name:&str,flags:u16,skills:&[(i32,&str,i32,i32,i32)])->dfmcp_core::Result<LiveSpatialCitizenObservation>{
    let base=base_fixture()?;let mut op=base.operations().clone();
    if let Some(job)=op.jobs.jobs.first_mut(){job.worker_native_id=worker;}
    let op=op.encode_profile(OperationsProfile::PagedV1_4)?;let map=base.terrain().encode_payload()?;
    let mut spatial=b"DFMS1600".to_vec();for part in [&op,&map]{spatial.extend_from_slice(&(part.len() as u32).to_be_bytes());spatial.extend_from_slice(part);}
    let citizens=citizen_component(citizen_id,name,[1,1,5],flags,skills);
    let mut payload=b"DFMS1800".to_vec();for part in [&spatial,&citizens]{payload.extend_from_slice(&(part.len() as u32).to_be_bytes());payload.extend_from_slice(part);}
    LiveSpatialCitizenObservation::decode_payload(&payload,7,"df".to_owned(),"dfhack".to_owned())
}
fn combined(worker:Option<u32>,citizen_id:u32,name:&str,flags:u16)->dfmcp_core::Result<LiveSpatialCitizenObservation>{
    combined_with_skills(worker,citizen_id,name,flags,&[])
}

#[test]
fn strict_citizen_worker_becomes_generation_checked_performs_and_location_edges()->dfmcp_core::Result<()>{
    let value=combined(Some(42),42,"Urist",0b1_0101_1111)?;let mut state=LiveSpatialCitizenState::default();
    assert_eq!(state.publish(value)?,JobPublication::Bootstrap);let snapshot=state.snapshot().ok_or_else(||dfmcp_core::DfmcpError::new(dfmcp_core::ErrorCode::InternalInvariantViolation,"snapshot"))?;
    let unit=citizen_entity_id(42);let entity=&snapshot.graph.entities[&unit];assert_eq!(entity.label,"Urist");assert_eq!(entity.generation,1);
    let job=snapshot.graph.entities.values().find(|e|e.kind==dfmcp_world::EntityKind::Job).ok_or_else(||dfmcp_core::DfmcpError::new(dfmcp_core::ErrorCode::InvalidRequest,"job"))?;
    assert!(matches!(job.fields["worker_entity"].value,Value::Entity(id) if id==unit));
    assert_eq!(job.fields["worker_is_strict_citizen"].value,Value::Bool(true));
    assert!(snapshot.graph.edges.values().any(|e|e.kind==EdgeKind::Performs&&e.from==unit&&e.to==job.id));
    assert!(snapshot.graph.edges.values().any(|e|e.kind==EdgeKind::LocatedAt&&e.from==unit&&e.to==tile_entity_id([1,1,5]).unwrap()));
    assert!(snapshot.graph.edges.values().any(|e|e.kind==EdgeKind::LocatedAt&&e.from==job.id));Ok(())
}

#[test]
fn nonzero_skills_project_nominal_effective_experience_without_eligibility_claim()->dfmcp_core::Result<()>{
    let value=combined_with_skills(None,42,"Urist",0b1_0101_1111,&[(0,"MINING",7,5,1200),(2,"CARPENTRY",3,3,400)])?;
    let mut state=LiveSpatialCitizenState::default();state.publish(value)?;let snapshot=state.snapshot().ok_or_else(||dfmcp_core::DfmcpError::new(dfmcp_core::ErrorCode::InternalInvariantViolation,"snapshot"))?;
    let citizen=&snapshot.graph.entities[&citizen_entity_id(42)];assert_eq!(citizen.fields["skill_count"].value,Value::U64(2));
    assert_eq!(citizen.fields["skill.MINING.nominal"].value,Value::I64(7));assert_eq!(citizen.fields["skill.MINING.effective"].value,Value::I64(5));
    assert_eq!(citizen.fields["skill.MINING.experience"].value,Value::I64(1200));assert_eq!(citizen.fields["top_skill_key"].value,Value::Text("MINING".to_owned()));
    assert_eq!(citizen.fields["top_skill_effective"].value,Value::I64(5));assert!(!citizen.fields.contains_key("labor_eligible"));Ok(())
}

#[test]
fn assigned_non_citizen_is_not_invented_as_a_unit()->dfmcp_core::Result<()>{
    let value=combined(Some(77),42,"Urist",0b1_0101_1111)?;let mut state=LiveSpatialCitizenState::default();state.publish(value)?;
    let snapshot=state.snapshot().ok_or_else(||dfmcp_core::DfmcpError::new(dfmcp_core::ErrorCode::InternalInvariantViolation,"snapshot"))?;
    let job=snapshot.graph.entities.values().find(|e|e.kind==dfmcp_world::EntityKind::Job).ok_or_else(||dfmcp_core::DfmcpError::new(dfmcp_core::ErrorCode::InvalidRequest,"job"))?;
    assert_eq!(job.fields["worker_is_strict_citizen"].value,Value::Bool(false));
    assert!(matches!(job.fields["worker_entity"].presence,Some(FactPresence::Unknown(_))));
    assert!(!snapshot.graph.entities.contains_key(&citizen_entity_id(77)));Ok(())
}

#[test]
fn citizen_only_change_advances_combined_anchor_and_preserves_identity()->dfmcp_core::Result<()>{
    let mut state=LiveSpatialCitizenState::default();state.publish(combined(None,42,"Urist",0b1_0101_1111)?)?;
    let first=state.snapshot().ok_or_else(||dfmcp_core::DfmcpError::new(dfmcp_core::ErrorCode::InternalInvariantViolation,"snapshot"))?.clone();
    assert_eq!(state.publish(combined(None,42,"Domas",0b1_0101_1111)?)?,JobPublication::Advanced);
    let second=state.snapshot().ok_or_else(||dfmcp_core::DfmcpError::new(dfmcp_core::ErrorCode::InternalInvariantViolation,"snapshot"))?;
    assert_eq!(second.cursor.epoch,first.cursor.epoch);assert_eq!(second.cursor.sequence,first.cursor.sequence+1);
    assert_eq!(second.graph.entities[&citizen_entity_id(42)].generation,first.graph.entities[&citizen_entity_id(42)].generation);
    assert_eq!(second.graph.entities[&citizen_entity_id(42)].label,"Domas");Ok(())
}

#[test]
fn strict_roster_rejects_residents_malformed_skills_and_trailing_bytes()->dfmcp_core::Result<()>{
    let good=combined(None,42,"Urist",0b1_0101_1111)?;let mut bytes=good.encode_payload()?;bytes.push(0);
    assert!(LiveSpatialCitizenObservation::decode_payload(&bytes,7,"df".to_owned(),"dfhack".to_owned()).is_err());
    assert!(combined(None,42,"Resident",0b1_0111_1111).is_err());
    assert!(combined_with_skills(None,42,"Urist",0b1_0101_1111,&[(2,"CARPENTRY",1,1,1),(1,"MINING",1,1,1)]).is_err());
    assert!(combined_with_skills(None,42,"Urist",0b1_0101_1111,&[(1,"MINING",0,0,0)]).is_err());Ok(())
}
