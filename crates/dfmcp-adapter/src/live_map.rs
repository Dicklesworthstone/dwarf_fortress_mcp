#![forbid(unsafe_code)]

//! Fixed map/1.5 observation profile. This is one bounded region, not a join
//! with independently acquired operations/citizen state. Hidden cells are redacted.

use std::collections::BTreeMap;
use dfmcp_core::{DfmcpError,Digest32,EntityId,ErrorCode,FortressId,GameTick,MapCoord,ObservationCursor,Result};
use dfmcp_world::{EntityKind,EntityRecord,Fact,FactPresence,FactSource,Value,WorldGraph,WorldSnapshot};
use dfmcp_world::map_region::{Cell,MapError,MapRegion,Region,Shape,Tile,MAX_MAP_TILES};
use crate::live_jobs::JobPublication;

pub const MAX_MAP_BYTES: usize = 1024 * 1024;
const MAGIC: &[u8;8] = b"DFMM1500";
const FIELDS: [&str;11] = ["tiletype","shape","liquid_depth","magma","traffic","dig_designation",
    "building_occupancy","unit_occupancy","walkable_region","temperature_1_raw","temperature_2_raw"];
fn invalid(message:&str)->DfmcpError {DfmcpError::new(ErrorCode::AdapterRejected,message)}
pub fn map_error(error:MapError)->DfmcpError {
    DfmcpError::new(if error==MapError::BudgetExceeded {ErrorCode::BudgetExceeded}else{ErrorCode::InvalidRequest},
        format!("map request or observation rejected: {error:?}"))
}
fn text_valid(text:&str,maximum:usize)->bool {!text.is_empty() && text.len()<=maximum && !text.contains('\0')}
fn put(out:&mut Vec<u8>,value:u32){out.extend_from_slice(&value.to_be_bytes());}
fn text(out:&mut Vec<u8>,value:&str){out.extend_from_slice(&(value.len() as u16).to_be_bytes());out.extend_from_slice(value.as_bytes());}

#[derive(Clone,Debug,PartialEq,Eq)]
pub struct LiveMapObservation {
    pub bridge_generation:u64,
    pub df_version:String,
    pub dfhack_version:String,
    pub year:u32,
    pub year_tick:u32,
    pub paused:bool,
    pub site_id:u32,
    pub world_folder:String,
    pub map_dimensions:[u32;3],
    pub map:MapRegion,
}
impl LiveMapObservation {
    pub fn validate(&self)->Result<()> {
        if self.bridge_generation==0 || self.bridge_generation==u64::MAX || self.year_tick>=403200
            || self.site_id>i32::MAX as u32 || !text_valid(&self.df_version,128)
            || !text_valid(&self.dfhack_version,128) || !text_valid(&self.world_folder,512) {
            return Err(invalid("invalid map manifest, world identity or clock"));
        }
        self.map.validate().map_err(map_error)?;
        for axis in 0..3 {
            if self.map_dimensions[axis]==0 || self.map_dimensions[axis]>32768
                || self.map.region.origin[axis]+self.map.region.size[axis]>self.map_dimensions[axis] {
                return Err(invalid("map region exceeds the observed map dimensions"));
            }
        }
        Ok(())
    }
    pub fn tick(&self)->GameTick {GameTick(u64::from(self.year)*403200+u64::from(self.year_tick))}
    pub fn fortress_id(&self)->Result<FortressId> {
        self.validate()?;
        let mut bytes=b"dfmcp-live-fortress-id-v1\0".to_vec();bytes.extend_from_slice(self.world_folder.as_bytes());bytes.push(0);
        bytes.extend_from_slice(&self.site_id.to_be_bytes());
        let digest=Digest32::of_bytes(&bytes);
        let mut id=[0;8];id.copy_from_slice(&digest.as_bytes()[..8]);
        Ok(FortressId::new(u64::from_be_bytes(id)|1))
    }
    pub fn encode_payload(&self)->Result<Vec<u8>> {
        self.validate()?;
        let mut out=MAGIC.to_vec();put(&mut out,self.year);put(&mut out,self.year_tick);
        out.push(u8::from(self.paused));put(&mut out,self.site_id);text(&mut out,&self.world_folder);
        for array in [self.map_dimensions,self.map.region.origin,self.map.region.size] {for n in array {put(&mut out,n);}}
        put(&mut out,self.map.cells.len() as u32);
        for cell in &self.map.cells {
            match cell {
                Cell::Unallocated=>out.push(0),Cell::Hidden=>out.push(1),Cell::Visible(t)=>{
                    out.push(2);put(&mut out,t.native_tiletype);
                    out.extend_from_slice(&[t.shape as u8,t.liquid_depth,u8::from(t.magma),t.traffic,
                        t.dig_designation,t.building_occupancy,t.unit_occupancy]);
                    put(&mut out,t.walkable_region);out.extend_from_slice(&t.temperature_1.to_be_bytes());
                    out.extend_from_slice(&t.temperature_2.to_be_bytes());
                }
            }
        }
        if out.len()>MAX_MAP_BYTES {return Err(DfmcpError::new(ErrorCode::BudgetExceeded,"map payload too large"));}
        Ok(out)
    }
    pub fn decode_payload(bytes:&[u8],bridge_generation:u64,df_version:String,dfhack_version:String)->Result<Self> {
        if bytes.len()>MAX_MAP_BYTES {return Err(DfmcpError::new(ErrorCode::BudgetExceeded,"map payload too large"));}
        let mut r=Reader{bytes,offset:0};if r.take(8)?!=MAGIC {return Err(invalid("wrong map payload profile"));}
        let year=r.u32()?;let year_tick=r.u32()?;let paused=r.boolean()?;let site_id=r.u32()?;let world_folder=r.text(512)?;
        let map_dimensions=r.vector()?;let region=Region{origin:r.vector()?,size:r.vector()?};
        let count=r.u32()? as usize;
        if count!=region.volume().map_err(map_error)? || count>MAX_MAP_TILES {return Err(invalid("map cell count differs from region volume"));}
        let mut cells=Vec::with_capacity(count);
        for _ in 0..count {
            cells.push(match r.byte()? {
                0=>Cell::Unallocated,1=>Cell::Hidden,2=>Cell::Visible(Tile{
                    native_tiletype:r.u32()?,shape:Shape::from_tag(r.byte()?).map_err(map_error)?,
                    liquid_depth:r.byte()?,magma:r.boolean()?,traffic:r.byte()?,dig_designation:r.byte()?,
                    building_occupancy:r.byte()?,unit_occupancy:r.byte()?,walkable_region:r.u32()?,
                    temperature_1:r.u16()?,temperature_2:r.u16()?,
                }),_=>return Err(invalid("unknown map cell presence tag")),
            });
        }
        if r.offset!=bytes.len(){return Err(invalid("trailing map payload bytes"));}
        let value=Self{bridge_generation,df_version,dfhack_version,year,year_tick,paused,site_id,world_folder,
            map_dimensions,map:MapRegion{region,cells}};
        value.validate()?;Ok(value)
    }
    pub fn source_digest(&self)->Result<Digest32> {
        let payload=self.encode_payload()?;
        let mut out=b"dfmcp-map-source-1.5\0".to_vec();out.extend_from_slice(&self.bridge_generation.to_be_bytes());
        text(&mut out,&self.df_version);text(&mut out,&self.dfhack_version);out.extend_from_slice(&payload);
        Ok(Digest32::of_bytes(&out))
    }
}
struct Reader<'a>{bytes:&'a [u8],offset:usize}
impl<'a> Reader<'a>{
    fn take(&mut self,n:usize)->Result<&'a [u8]>{let end=self.offset.checked_add(n).ok_or_else(||invalid("map length overflow"))?;
        let v=self.bytes.get(self.offset..end).ok_or_else(||invalid("truncated map payload"))?;self.offset=end;Ok(v)}
    fn byte(&mut self)->Result<u8>{Ok(self.take(1)?[0])}
    fn boolean(&mut self)->Result<bool>{match self.byte()?{0=>Ok(false),1=>Ok(true),_=>Err(invalid("noncanonical map Boolean"))}}
    fn u32(&mut self)->Result<u32>{Ok(u32::from_be_bytes(self.take(4)?.try_into().map_err(|_|invalid("invalid u32"))?))}
    fn u16(&mut self)->Result<u16>{Ok(u16::from_be_bytes(self.take(2)?.try_into().map_err(|_|invalid("invalid u16"))?))}
    fn vector(&mut self)->Result<[u32;3]>{Ok([self.u32()?,self.u32()?,self.u32()?])}
    fn text(&mut self,limit:usize)->Result<String>{let n=self.u16()? as usize;if n>limit{return Err(invalid("map string too large"));}
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_|invalid("invalid map UTF-8"))}
}

pub fn tile_entity_id(position:[u32;3])->Result<EntityId>{
    if position.iter().any(|v|*v>=32768){return Err(invalid("map coordinate exceeds identity range"));}
    Ok(EntityId::new((3u64<<60)|(u64::from(position[2])<<30)|(u64::from(position[1])<<15)|u64::from(position[0])))
}
fn coord(v:[u32;3])->Value {Value::Coord(MapCoord::new(v[0] as i32,v[1] as i32,v[2] as i32))}

#[derive(Clone,Debug,Default)]
pub struct LiveMapState{observation:Option<LiveMapObservation>,snapshot:Option<WorldSnapshot>}
impl LiveMapState{
    pub fn snapshot(&self)->Option<&WorldSnapshot>{self.snapshot.as_ref()}
    pub fn observation(&self)->Option<&LiveMapObservation>{self.observation.as_ref()}
    pub fn publish(&mut self,value:LiveMapObservation)->Result<JobPublication>{
        value.validate()?;let fortress=value.fortress_id()?;let tick=value.tick();let source=value.source_digest()?;
        let mut cursor=ObservationCursor::ORIGIN;let mut outcome=JobPublication::Bootstrap;
        if let (Some(prior),Some(snapshot))=(&self.observation,&self.snapshot){
            if value.world_folder!=prior.world_folder || value.site_id!=prior.site_id
                || value.map.region!=prior.map.region || value.df_version!=prior.df_version || value.dfhack_version!=prior.dfhack_version {
                return Err(DfmcpError::new(ErrorCode::StaleAnchor,"map source identity, region or software changed; reopen session"));
            }
            if &value==prior{return Ok(JobPublication::Heartbeat);}
            let reset=value.bridge_generation!=prior.bridge_generation || tick<prior.tick() || value.map_dimensions!=prior.map_dimensions;
            outcome=if reset{JobPublication::Reset}else{JobPublication::Advanced};
            cursor=if reset{ObservationCursor{epoch:snapshot.cursor.epoch.checked_add(1).ok_or_else(||invalid("map epoch exhausted"))?,sequence:0}}
                else{ObservationCursor{epoch:snapshot.cursor.epoch,sequence:snapshot.cursor.sequence.checked_add(1).ok_or_else(||invalid("map sequence exhausted"))?}};
        }
        let generation=u32::try_from(cursor.epoch.checked_add(1).ok_or_else(||invalid("map epoch exhausted"))?)
            .map_err(|_|invalid("map generation exhausted"))?;
        let revision=cursor.sequence.checked_add(1).ok_or_else(||invalid("map revision exhausted"))?;
        let fact=|field:&str,v|Fact::known(v,tick,FactSource::DfhackField(format!("map/1.5.{field}")),source);
        let mut graph=WorldGraph::default();
        graph.entities.insert(EntityId::new(1),EntityRecord{id:EntityId::new(1),generation,revision,kind:EntityKind::Fortress,
            label:"Bounded fortress terrain observation".to_owned(),fields:BTreeMap::from([
                ("region_origin".to_owned(),fact("requested_region",coord(value.map.region.origin))),
                ("region_size".to_owned(),fact("requested_region",coord(value.map.region.size))),
                ("map_dimensions".to_owned(),fact("Maps.getTileSize",coord(value.map_dimensions))),
                ("paused".to_owned(),fact("World.ReadPauseState",Value::Bool(value.paused))),
            ])});
        for (index,cell) in value.map.cells.iter().enumerate(){
            let p=value.map.region.position(index).ok_or_else(||invalid("tile index outside region"))?;let id=tile_entity_id(p)?;
            let status=match cell{Cell::Unallocated=>"unallocated",Cell::Hidden=>"hidden",Cell::Visible(_)=>"visible"};
            let mut fields=BTreeMap::from([("position".to_owned(),fact("tile_position",coord(p))),
                ("visibility".to_owned(),fact("tile_visibility",Value::Text(status.to_owned())))]);
            match cell {
                Cell::Visible(t)=>{
                    let values=[Value::U64(u64::from(t.native_tiletype)),Value::Text(t.shape.name().to_owned()),
                        Value::U64(u64::from(t.liquid_depth)),Value::Bool(t.magma),Value::U64(u64::from(t.traffic)),
                        Value::U64(u64::from(t.dig_designation)),Value::U64(u64::from(t.building_occupancy)),
                        Value::U64(u64::from(t.unit_occupancy)),Value::U64(u64::from(t.walkable_region)),
                        Value::U64(u64::from(t.temperature_1)),Value::U64(u64::from(t.temperature_2))];
                    for (name,v) in FIELDS.into_iter().zip(values){fields.insert(name.to_owned(),fact(name,v));}
                }
                _=>for name in FIELDS{
                    let presence=if *cell==Cell::Hidden{FactPresence::Redacted("undiscovered terrain".to_owned())}
                        else{FactPresence::Unknown("map block is unallocated".to_owned())};
                    fields.insert(name.to_owned(),Fact::with_presence(presence,tick,
                        FactSource::DfhackField(format!("map/1.5.{name}")),source));
                },
            }
            graph.entities.insert(id,EntityRecord{id,generation,revision,kind:EntityKind::TileFeature,
                label:format!("Tile ({},{},{})",p[0],p[1],p[2]),fields});
        }
        let snapshot=WorldSnapshot::new(fortress,tick,cursor,value.paused,graph);
        self.observation=Some(value);self.snapshot=Some(snapshot);Ok(outcome)
    }
}
