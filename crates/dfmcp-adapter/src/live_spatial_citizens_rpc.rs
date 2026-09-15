//! Fixed spatial/1.8 transport. Citizens, operations and terrain arrive in one
//! immutable native capture; caller-selected plugin/method names are impossible.

use crate::live_jobs_rpc::{DeadlineStream,JobsManifest,Message,bytes,call,checked_timeout,failure,io_failure,malformed,number};
use crate::live_jobs_rpc::operations::paged::snapshot::{SnapshotAssembler,SnapshotManifest,SnapshotPage};
use crate::live_spatial::citizens::{LiveSpatialCitizenObservation,MAX_COHERENT_CITIZENS};
use dfmcp_core::{Digest32,ErrorCode,Result};
use std::io::{Read,Write};
use std::net::{SocketAddr,TcpStream};
use std::time::{Duration,Instant};
use super::SpatialLimits;

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct CitizenSpatialLimits{pub spatial:SpatialLimits,pub citizens:u32}
impl CitizenSpatialLimits{
    pub fn validate(self)->Result<()>{self.spatial.validate()?;if self.citizens==0||self.citizens as usize>MAX_COHERENT_CITIZENS{
        return Err(failure(ErrorCode::BudgetExceeded,"spatial/1.8 citizen bound must be 1..4096"));}Ok(())}
    pub fn entity_limit(self)->u32{self.spatial.entity_limit().saturating_add(self.citizens)}
}
fn request(token:&[u8],nonce:&[u8],limits:CitizenSpatialLimits,snapshot:&[u8],offset:usize,release:bool)->Vec<u8>{
    let mut out=Vec::new();bytes(&mut out,1,token);bytes(&mut out,2,nonce);let spatial=limits.spatial;let op=spatial.operations;
    for(field,value)in[(3,1),(4,8),(5,u64::from(op.jobs)),(6,u64::from(op.buildings)),(7,u64::from(op.items)),
        (8,op.payload_bytes as u64),(10,offset as u64),(11,op.page_bytes as u64),(12,u64::from(release)),(19,u64::from(limits.citizens))]{number(&mut out,field,value);}
    if !snapshot.is_empty(){bytes(&mut out,9,snapshot);}
    for(i,value)in spatial.region.origin.into_iter().chain(spatial.region.size).enumerate(){number(&mut out,13+i as u32,u64::from(value));}out
}
fn envelope<'a>(data:&'a[u8],nonce:&[u8])->Result<(Message<'a>,JobsManifest)>{
    let m=Message::parse(data,14)?;if m.number(4)?!=1||m.number(5)?!=8{return Err(failure(ErrorCode::VersionMismatch,"citizen spatial transport requires exactly protocol 1.8"));}
    if m.bytes(3,64)?!=nonce{return Err(malformed());}let accepted=m.number(1)?;let code=m.number(2)?;
    if accepted==0&&code!=0{if(9..=14).any(|f|m.0.contains_key(&f)){return Err(malformed());}
        return Err(failure(match code{1=>ErrorCode::CapabilityDenied,2=>ErrorCode::VersionMismatch,3=>ErrorCode::BudgetExceeded,
            4=>ErrorCode::FortressNotLoaded,6=>ErrorCode::StaleAnchor,_=>ErrorCode::AdapterFailure},"spatial/1.8 capture refused; no partial world was published"));}
    if accepted!=1||code!=0{return Err(malformed());}
    let source=JobsManifest{generation:m.number(6)?,df_version:m.text(7)?,dfhack_version:m.text(8)?};
    if source.generation==0||source.generation==u64::MAX{return Err(malformed());}Ok((m,source))
}
fn bind<S:Read+Write>(stream:&mut S,name:&str)->Result<i16>{
    let mut request=Vec::new();for(f,text)in[(1,name),(2,"dfmcp.spatial.v1_8.Request"),(3,"dfmcp.spatial.v1_8.Reply"),(4,"dfmcp_spatial_v1_8")]{bytes(&mut request,f,text.as_bytes());}
    let reply=call(stream,0,&request,1024)?;let id=i16::try_from(Message::parse(&reply,1)?.number(1)?).map_err(|_|malformed())?;
    if id<2{return Err(malformed());}Ok(id)
}
fn page(data:&[u8],nonce:&[u8],maximum:usize)->Result<SnapshotPage>{
    let(m,s)=envelope(data,nonce)?;let complete=match m.number(14)?{0=>false,1=>true,_=>return Err(malformed())};
    Ok(SnapshotPage{manifest:SnapshotManifest{token:m.bytes(10,16)?.try_into().map_err(|_|malformed())?,generation:s.generation,
        df_version:s.df_version,dfhack_version:s.dfhack_version,total_bytes:usize::try_from(m.number(12)?).map_err(|_|malformed())?,
        payload_digest:Digest32::from_bytes(m.bytes(13,32)?.try_into().map_err(|_|malformed())?)},
        offset:usize::try_from(m.number(11)?).map_err(|_|malformed())?,bytes:m.bytes(9,maximum)?.to_vec(),complete})
}

pub struct CitizenSpatialRpcClient<S>{stream:S,token:Vec<u8>,nonce:Vec<u8>,limits:CitizenSpatialLimits,
    manifest:JobsManifest,method:i16,poisoned:bool,last_pages:u32}
impl<S:Read+Write> CitizenSpatialRpcClient<S>{
    pub fn negotiate(mut stream:S,token:Vec<u8>,nonce:Vec<u8>,limits:CitizenSpatialLimits)->Result<Self>{
        limits.validate()?;if !(32..=256).contains(&token.len())||!(16..=64).contains(&nonce.len()){
            return Err(failure(ErrorCode::InvalidRequest,"invalid spatial/1.8 credentials"));}
        let mut hello=b"DFHack?\n".to_vec();hello.extend_from_slice(&1i32.to_le_bytes());stream.write_all(&hello).map_err(io_failure)?;stream.flush().map_err(io_failure)?;
        let mut answer=[0;12];stream.read_exact(&mut answer).map_err(io_failure)?;if &answer[..8]!=b"DFHack!\n"||answer[8..]!=1i32.to_le_bytes(){return Err(malformed());}
        let handshake=bind(&mut stream,"Handshake")?;let method=bind(&mut stream,"ReadObservation")?;if handshake==method{return Err(malformed());}
        let reply=call(&mut stream,handshake,&request(&token,&nonce,limits,&[],0,false),4096)?;let(m,manifest)=envelope(&reply,&nonce)?;
        if(9..=14).any(|f|m.0.contains_key(&f)){return Err(malformed());}
        Ok(Self{stream,token,nonce,limits,manifest,method,poisoned:false,last_pages:0})
    }
    pub fn poisoned(&self)->bool{self.poisoned}pub fn fence(&mut self){self.poisoned=true;}pub fn last_page_count(&self)->u32{self.last_pages}
    pub fn read_observation(&mut self)->Result<LiveSpatialCitizenObservation>{
        if self.poisoned{return Err(failure(ErrorCode::AdapterUnavailable,"spatial/1.8 source is fenced; reopen session"));}
        let result=self.acquire();if result.is_err(){self.fence();}result
    }
    fn acquire(&mut self)->Result<LiveSpatialCitizenObservation>{
        let op=self.limits.spatial.operations;let mut assembly=SnapshotAssembler::new(op.payload_bytes,op.page_bytes)?;let mut count=0;
        while !assembly.complete(){if count>=1024{return Err(failure(ErrorCode::BudgetExceeded,"spatial/1.8 acquisition exceeded 1024 pages"));}
            let token=assembly.manifest().map_or(&[][..],|m|m.token.as_slice());let req=request(&self.token,&self.nonce,self.limits,token,assembly.offset(),false);
            let reply=call(&mut self.stream,self.method,&req,op.page_bytes+4096)?;let p=page(&reply,&self.nonce,op.page_bytes)?;
            if p.manifest.generation<self.manifest.generation||p.manifest.df_version!=self.manifest.df_version||p.manifest.dfhack_version!=self.manifest.dfhack_version{
                return Err(failure(ErrorCode::StaleAnchor,"spatial/1.8 manifest changed incompatibly"));}
            assembly.push(p)?;count+=1;
        }
        let(manifest,payload)=assembly.finish()?;let value=LiveSpatialCitizenObservation::decode_payload(&payload,manifest.generation,manifest.df_version.clone(),manifest.dfhack_version.clone())?;
        let observed=value.spatial().operations();
        if observed.jobs.jobs.len()>op.jobs as usize||observed.buildings.len()>op.buildings as usize||observed.items.len()>op.items as usize
            ||value.citizens().len()>self.limits.citizens as usize||value.spatial().terrain().map.region!=self.limits.spatial.region{
            return Err(failure(ErrorCode::AdapterRejected,"spatial/1.8 capture differs from negotiated region or rosters"));}
        let req=request(&self.token,&self.nonce,self.limits,&manifest.token,0,true);let reply=call(&mut self.stream,self.method,&req,4096)?;
        let(ack,source)=envelope(&reply,&self.nonce)?;
        if ack.bytes(10,16)?!=manifest.token.as_slice()||ack.0.contains_key(&9)||(11..=14).any(|f|ack.0.contains_key(&f))
            ||source.generation!=manifest.generation||source.df_version!=manifest.df_version||source.dfhack_version!=manifest.dfhack_version{return Err(malformed());}
        self.manifest=source;self.last_pages=count;Ok(value)
    }
}
impl CitizenSpatialRpcClient<DeadlineStream>{
    pub fn connect(endpoint:SocketAddr,token:Vec<u8>,nonce:Vec<u8>,timeout:Duration,limits:CitizenSpatialLimits)->Result<Self>{
        limits.validate()?;checked_timeout(timeout)?;if !(32..=256).contains(&token.len())||!(16..=64).contains(&nonce.len()){
            return Err(failure(ErrorCode::InvalidRequest,"invalid spatial/1.8 credentials"));}
        if !endpoint.ip().is_loopback()||endpoint.port()==0{return Err(failure(ErrorCode::CapabilityDenied,"spatial/1.8 endpoint must be numeric loopback"));}
        let deadline=Instant::now().checked_add(timeout).ok_or_else(||failure(ErrorCode::BudgetExceeded,"spatial/1.8 deadline overflow"))?;
        let stream=TcpStream::connect_timeout(&endpoint,timeout).map_err(io_failure)?;stream.set_nodelay(true).map_err(io_failure)?;
        Self::negotiate(DeadlineStream{stream,deadline},token,nonce,limits)
    }
    pub fn refresh(&mut self,timeout:Duration)->Result<LiveSpatialCitizenObservation>{
        self.stream.deadline=Instant::now().checked_add(checked_timeout(timeout)?).ok_or_else(||failure(ErrorCode::BudgetExceeded,"spatial/1.8 deadline overflow"))?;
        self.read_observation()
    }
}
