#![forbid(unsafe_code)]

//! Citizen-inclusive spatial/1.8 projection. The embedded spatial/1.6 payload is
//! a data codec acquired in the same native suspension, never an independent read.

use std::collections::{BTreeMap, BTreeSet};
use dfmcp_core::{DfmcpError,Digest32,EdgeId,EntityId,ErrorCode,MapCoord,ObservationCursor,Result};
use dfmcp_world::{EdgeKind,EdgeRecord,EntityKind,EntityRecord,Fact,FactPresence,FactSource,Value,WorldSnapshot};
use crate::live_jobs::JobPublication;
use crate::live_spatial::{LiveSpatialObservation,SpatialStateView};

pub const MAX_COHERENT_CITIZENS:usize=4096;
pub const MAX_SPATIAL_CITIZEN_BYTES:usize=16*1024*1024;
const MAGIC:&[u8;8]=b"DFMS1800";
const CITIZEN_MAGIC:&[u8;8]=b"DFMC1800";
const CITIZEN_NAMESPACE:u64=3<<40;
const MAX_IDENTITIES:usize=300_000;
const CITIZEN_FLAGS:u16=0x1ff;

fn invalid(text:&str)->DfmcpError{DfmcpError::new(ErrorCode::AdapterRejected,text)}
fn exhausted(text:&str)->DfmcpError{DfmcpError::new(ErrorCode::BudgetExceeded,text)}

#[derive(Clone,Debug,PartialEq,Eq)]
pub struct LiveCitizen{
    pub native_id:u32,pub name:String,pub race:String,pub profession:i32,pub position:MapCoord,pub flags:u16,
}
impl LiveCitizen{
    pub fn alive(&self)->bool{self.flags&1!=0}pub fn sane(&self)->bool{self.flags&2!=0}
    pub fn active(&self)->bool{self.flags&4!=0}pub fn visible(&self)->bool{self.flags&8!=0}
    pub fn citizen(&self)->bool{self.flags&16!=0}pub fn resident(&self)->bool{self.flags&32!=0}
    pub fn baby(&self)->bool{self.flags&64!=0}pub fn child(&self)->bool{self.flags&128!=0}pub fn adult(&self)->bool{self.flags&256!=0}
}

#[derive(Clone,Debug,PartialEq,Eq)]
pub struct LiveSpatialCitizenObservation{spatial:LiveSpatialObservation,citizens:Vec<LiveCitizen>}
impl LiveSpatialCitizenObservation{
    pub fn spatial(&self)->&LiveSpatialObservation{&self.spatial}
    pub fn citizens(&self)->&[LiveCitizen]{&self.citizens}
    pub fn validate(&self)->Result<()>{
        self.spatial.validate()?;
        if self.citizens.len()>MAX_COHERENT_CITIZENS{return Err(exhausted("citizen roster exceeds spatial/1.8 bound"));}
        let mut previous=None;let mut ids=BTreeSet::new();
        for citizen in &self.citizens{
            if citizen.native_id>i32::MAX as u32||previous.is_some_and(|id|id>=citizen.native_id)||!ids.insert(citizen.native_id)
                ||citizen.name.len()>256||citizen.race.len()>128||citizen.name.contains('\0')||citizen.race.contains('\0')
                ||citizen.profession<0||citizen.flags&!CITIZEN_FLAGS!=0||!citizen.citizen()||citizen.resident(){
                return Err(invalid("invalid, unordered, or non-strict citizen in spatial/1.8 roster"));
            }
            previous=Some(citizen.native_id);
        }
        Ok(())
    }
    pub fn decode_payload(bytes:&[u8],generation:u64,df:String,dfhack:String)->Result<Self>{
        if bytes.len()>MAX_SPATIAL_CITIZEN_BYTES{return Err(exhausted("spatial/1.8 capture exceeds byte bound"));}
        let mut r=Reader{bytes,offset:0};if r.take(8)?!=MAGIC{return Err(invalid("wrong spatial/1.8 capture profile"));}
        let n=r.u32()? as usize;let spatial=LiveSpatialObservation::decode_payload(r.take(n)?,generation,df,dfhack)?;
        let n=r.u32()? as usize;let citizens=decode_citizens(r.take(n)?)?;
        if r.offset!=bytes.len(){return Err(invalid("trailing spatial/1.8 capture bytes"));}
        let value=Self{spatial,citizens};value.validate()?;Ok(value)
    }
    pub fn encode_payload(&self)->Result<Vec<u8>>{
        self.validate()?;let base=self.spatial.encode_payload()?;let units=encode_citizens(&self.citizens)?;
        let total=base.len().checked_add(units.len()).and_then(|n|n.checked_add(16)).ok_or_else(||exhausted("spatial/1.8 length overflow"))?;
        if total>MAX_SPATIAL_CITIZEN_BYTES{return Err(exhausted("spatial/1.8 capture exceeds byte bound"));}
        let mut out=MAGIC.to_vec();put(&mut out,base.len() as u32);out.extend_from_slice(&base);put(&mut out,units.len() as u32);out.extend_from_slice(&units);Ok(out)
    }
    pub fn source_digest(&self)->Result<Digest32>{
        let mut bytes=b"dfmcp-spatial-citizen-source-1.8\0".to_vec();
        bytes.extend_from_slice(&self.spatial.terrain().bridge_generation.to_be_bytes());
        for text in [&self.spatial.terrain().df_version,&self.spatial.terrain().dfhack_version]{put(&mut bytes,text.len() as u32);bytes.extend_from_slice(text.as_bytes());}
        bytes.extend_from_slice(&self.encode_payload()?);Ok(Digest32::of_bytes(&bytes))
    }
}

fn encode_citizens(citizens:&[LiveCitizen])->Result<Vec<u8>>{
    let mut out=CITIZEN_MAGIC.to_vec();put(&mut out,citizens.len() as u32);
    for c in citizens{put(&mut out,c.native_id);text(&mut out,&c.name)?;text(&mut out,&c.race)?;signed(&mut out,c.profession);
        signed(&mut out,c.position.x);signed(&mut out,c.position.y);signed(&mut out,c.position.z);out.extend_from_slice(&c.flags.to_be_bytes());}
    Ok(out)
}
fn decode_citizens(bytes:&[u8])->Result<Vec<LiveCitizen>>{
    let mut r=Reader{bytes,offset:0};if r.take(8)?!=CITIZEN_MAGIC{return Err(invalid("wrong citizen component profile"));}
    let count=r.u32()? as usize;if count>MAX_COHERENT_CITIZENS{return Err(exhausted("citizen component count exceeds bound"));}
    let mut out=Vec::with_capacity(count);for _ in 0..count{out.push(LiveCitizen{native_id:r.u32()?,name:r.text(256)?,race:r.text(128)?,profession:r.i32()?,
        position:MapCoord::new(r.i32()?,r.i32()?,r.i32()?),flags:r.u16()?});}
    if r.offset!=bytes.len(){return Err(invalid("trailing citizen component bytes"));}Ok(out)
}
fn put(out:&mut Vec<u8>,value:u32){out.extend_from_slice(&value.to_be_bytes());}
fn signed(out:&mut Vec<u8>,value:i32){out.extend_from_slice(&value.to_be_bytes());}
fn text(out:&mut Vec<u8>,value:&str)->Result<()>{if value.len()>u16::MAX as usize{return Err(exhausted("citizen text length overflow"));}
    out.extend_from_slice(&(value.len() as u16).to_be_bytes());out.extend_from_slice(value.as_bytes());Ok(())}
struct Reader<'a>{bytes:&'a [u8],offset:usize}
impl<'a> Reader<'a>{
    fn take(&mut self,n:usize)->Result<&'a [u8]>{let end=self.offset.checked_add(n).ok_or_else(||invalid("spatial/1.8 length overflow"))?;
        let value=self.bytes.get(self.offset..end).ok_or_else(||invalid("truncated spatial/1.8 payload"))?;self.offset=end;Ok(value)}
    fn u32(&mut self)->Result<u32>{Ok(u32::from_be_bytes(self.take(4)?.try_into().map_err(|_|invalid("invalid citizen u32"))?))}
    fn i32(&mut self)->Result<i32>{Ok(self.u32()? as i32)}
    fn u16(&mut self)->Result<u16>{Ok(u16::from_be_bytes(self.take(2)?.try_into().map_err(|_|invalid("invalid citizen u16"))?))}
    fn text(&mut self,maximum:usize)->Result<String>{let n=self.u16()? as usize;if n>maximum{return Err(exhausted("citizen text exceeds bound"));}
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_|invalid("invalid citizen UTF-8"))}
}

#[must_use]pub fn citizen_entity_id(native:u32)->EntityId{EntityId::new(CITIZEN_NAMESPACE+u64::from(native))}
fn edge_id(kind:EdgeKind,from:EntityId,to:EntityId)->Result<EdgeId>{
    let mut bytes=b"dfmcp-spatial-citizen-edge/1\0".to_vec();bytes.extend_from_slice(kind.as_str().as_bytes());bytes.push(0);
    bytes.extend_from_slice(&from.get().to_be_bytes());bytes.extend_from_slice(&to.get().to_be_bytes());
    let digest=Digest32::of_bytes(&bytes);let raw:[u8;16]=digest.as_bytes()[..16].try_into().map_err(|_|invalid("edge digest width"))?;Ok(EdgeId::new(u128::from_be_bytes(raw)|1))
}

#[derive(Clone,Debug,Default)]
pub struct LiveSpatialCitizenState{
    observation:Option<LiveSpatialCitizenObservation>,snapshot:Option<WorldSnapshot>,generations:BTreeMap<EntityId,u32>,
}
impl LiveSpatialCitizenState{
    pub fn observation_full(&self)->Option<&LiveSpatialCitizenObservation>{self.observation.as_ref()}
    pub fn publish(&mut self,value:LiveSpatialCitizenObservation)->Result<JobPublication>{
        value.validate()?;let source=value.source_digest()?;let tick=value.spatial.terrain().tick();
        let mut cursor=ObservationCursor::ORIGIN;let mut outcome=JobPublication::Bootstrap;
        if let(Some(prior),Some(snapshot))=(&self.observation,&self.snapshot){
            let a=prior.spatial.terrain();let b=value.spatial.terrain();
            if a.world_folder!=b.world_folder||a.site_id!=b.site_id||a.df_version!=b.df_version||a.dfhack_version!=b.dfhack_version||a.map.region!=b.map.region{
                return Err(DfmcpError::new(ErrorCode::StaleAnchor,"spatial/1.8 world, region or software changed; reopen session"));}
            if prior==&value{return Ok(JobPublication::Heartbeat);}
            let reset=a.bridge_generation!=b.bridge_generation||b.tick()<a.tick()||a.map_dimensions!=b.map_dimensions
                ||value.spatial.operations().jobs.next_job_id<prior.spatial.operations().jobs.next_job_id
                ||value.spatial.operations().next_item_id<prior.spatial.operations().next_item_id
                ||value.spatial.operations().next_building_id<prior.spatial.operations().next_building_id;
            outcome=if reset{JobPublication::Reset}else{JobPublication::Advanced};
            cursor=if reset{ObservationCursor{epoch:snapshot.cursor.epoch.checked_add(1).ok_or_else(||exhausted("spatial/1.8 epoch exhausted"))?,sequence:0}}
                else{ObservationCursor{epoch:snapshot.cursor.epoch,sequence:snapshot.cursor.sequence.checked_add(1).ok_or_else(||exhausted("spatial/1.8 sequence exhausted"))?}};
        }
        let mut base=crate::live_spatial::LiveSpatialState::default();base.publish(value.spatial.clone())?;
        let mut graph=base.snapshot().ok_or_else(||invalid("embedded spatial projection absent"))?.graph.clone();
        let fact=|field:&str,v|Fact::known(v,tick,FactSource::DfhackField(format!("spatial/1.8.{field}")),source);
        if let Some(root)=graph.entities.get_mut(&EntityId::new(1)){
            root.label="Coherent fortress citizens, operations and terrain".to_owned();
            root.fields.insert("strict_citizen_count".to_owned(),fact("citizens.count",Value::U64(value.citizens.len() as u64)));
            root.fields.insert("observation_profile".to_owned(),fact("capture",Value::Text("spatial/1.8".to_owned())));
        }
        let citizen_ids:BTreeSet<_>=value.citizens.iter().map(|c|c.native_id).collect();
        for citizen in &value.citizens{
            let id=citizen_entity_id(citizen.native_id);if graph.entities.contains_key(&id){return Err(invalid("citizen entity namespace collision"));}
            let mut fields=BTreeMap::from([
                ("native_unit_id".to_owned(),fact("citizen.unit_id",Value::U64(u64::from(citizen.native_id)))),
                ("name".to_owned(),fact("citizen.name",Value::Text(citizen.name.clone()))),
                ("race".to_owned(),fact("citizen.race",Value::Text(citizen.race.clone()))),
                ("profession".to_owned(),fact("citizen.profession",Value::I64(i64::from(citizen.profession)))),
                ("position".to_owned(),fact("citizen.position",Value::Coord(citizen.position))),
            ]);
            for(name,value)in[("alive",citizen.alive()),("sane",citizen.sane()),("active",citizen.active()),("visible",citizen.visible()),
                ("citizen",citizen.citizen()),("resident",citizen.resident()),("baby",citizen.baby()),("child",citizen.child()),("adult",citizen.adult())]{
                fields.insert(name.to_owned(),fact(name,Value::Bool(value)));
            }
            graph.entities.insert(id,EntityRecord{id,generation:1,revision:1,kind:EntityKind::Unit,
                label:if citizen.name.is_empty(){format!("citizen-{}",citizen.native_id)}else{citizen.name.clone()},fields});
            let eid=edge_id(EdgeKind::MemberOf,id,EntityId::new(1))?;
            graph.edges.insert(eid,EdgeRecord{id:eid,revision:1,kind:EdgeKind::MemberOf,from:id,to:EntityId::new(1),
                fields:BTreeMap::from([("strict_citizen_membership".to_owned(),fact("citizen.citizen",Value::Bool(true)))])});
        }
        for job in &value.spatial.operations().jobs.jobs{
            let job_id=EntityId::new(u64::from(job.native_id)+2);
            let entity=graph.entities.get_mut(&job_id).ok_or_else(||invalid("job projection absent during citizen join"))?;
            match job.worker_native_id{
                None=>{entity.fields.insert("worker_entity".to_owned(),Fact::with_presence(FactPresence::Absent,tick,
                    FactSource::DfhackField("spatial/1.8.Job.getWorker".to_owned()),source));
                    entity.fields.insert("worker_is_strict_citizen".to_owned(),fact("Job.getWorker",Value::Bool(false)));}
                Some(worker)=>{
                    let strict=citizen_ids.contains(&worker);entity.fields.insert("worker_is_strict_citizen".to_owned(),fact("Job.getWorker",Value::Bool(strict)));
                    if strict{let worker_id=citizen_entity_id(worker);entity.fields.insert("worker_entity".to_owned(),fact("Job.getWorker",Value::Entity(worker_id)));
                        let eid=edge_id(EdgeKind::Performs,worker_id,job_id)?;graph.edges.insert(eid,EdgeRecord{id:eid,revision:1,kind:EdgeKind::Performs,from:worker_id,to:job_id,
                            fields:BTreeMap::from([("assignment_observed".to_owned(),fact("Job.getWorker",Value::Bool(true)))])});}
                    else{entity.fields.insert("worker_entity".to_owned(),Fact::with_presence(FactPresence::Unknown(
                        "assigned worker is outside the complete strict-citizen roster".to_owned()),tick,FactSource::DfhackField("spatial/1.8.Job.getWorker".to_owned()),source));}
                }
            }
        }
        let mut generations=self.generations.clone();let revision=cursor.sequence.checked_add(1).ok_or_else(||exhausted("spatial/1.8 revision exhausted"))?;
        for(id,entity)in &mut graph.entities{
            let present=self.snapshot.as_ref().is_some_and(|s|s.graph.entities.contains_key(id));
            let generation=match generations.get(id).copied(){Some(n)if !present||outcome==JobPublication::Reset=>n.checked_add(1).ok_or_else(||exhausted("spatial/1.8 generation exhausted"))?,Some(n)=>n,None=>1};
            if !generations.contains_key(id)&&generations.len()>=MAX_IDENTITIES{return Err(exhausted("spatial/1.8 identity history full"));}
            generations.insert(*id,generation);entity.generation=generation;entity.revision=revision;
            for fact in entity.fields.values_mut(){rebind(fact,source);}
        }
        for edge in graph.edges.values_mut(){edge.revision=revision;for fact in edge.fields.values_mut(){rebind(fact,source);}}
        let snapshot=WorldSnapshot::new(value.spatial.terrain().fortress_id()?,tick,cursor,value.spatial.terrain().paused,graph);
        self.observation=Some(value);self.snapshot=Some(snapshot);self.generations=generations;Ok(outcome)
    }
}
fn rebind(fact:&mut Fact,source:Digest32){fact.source_digest=source;if let FactSource::DfhackField(name)=&mut fact.source{
    if let Some(suffix)=name.strip_prefix("spatial/1.6."){*name=format!("spatial/1.8.{suffix}");}
}}
impl SpatialStateView for LiveSpatialCitizenState{
    fn snapshot(&self)->Option<&WorldSnapshot>{self.snapshot.as_ref()}
    fn spatial_observation(&self)->Option<&LiveSpatialObservation>{self.observation.as_ref().map(|v|v.spatial())}
    fn source_digest(&self)->Result<Digest32>{self.observation.as_ref().ok_or_else(||invalid("spatial/1.8 observation absent"))?.source_digest()}
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]fn citizen_namespace_does_not_overlap_jobs_buildings_items_or_tiles(){
        for id in [0,1,4095,i32::MAX as u32]{let unit=citizen_entity_id(id);assert!(unit.get()>=CITIZEN_NAMESPACE&&unit.get()<4<<40);}
        assert!((3<<40)>2+(i32::MAX as u64));
    }
}
