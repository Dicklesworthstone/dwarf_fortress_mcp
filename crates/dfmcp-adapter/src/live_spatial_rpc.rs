//! Fixed spatial/1.6 transport, compiled below live_jobs_rpc for shared framing.

#[path = "live_spatial_citizens_rpc.rs"]
pub mod citizens;

use crate::live_jobs_rpc::{DeadlineStream, JobsManifest, Message, bytes, call,
    checked_timeout, failure, io_failure, malformed, number};
use crate::live_jobs_rpc::operations::paged::{PagedOperationsLimits, snapshot::{SnapshotAssembler, SnapshotManifest, SnapshotPage}};
use crate::live_map::map_error;
use crate::live_spatial::LiveSpatialObservation;
use dfmcp_core::{Digest32, ErrorCode, Result};
use dfmcp_world::map_region::Region;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpatialLimits { pub operations: PagedOperationsLimits, pub region: Region }
impl SpatialLimits {
    pub fn validate(self) -> Result<()> { self.operations.validate()?; self.region.volume().map_err(map_error)?; Ok(()) }
    pub fn entity_limit(self) -> u32 { self.operations.entity_limit().saturating_add(self.region.volume().unwrap_or(0) as u32) }
}
fn request(token: &[u8], nonce: &[u8], limits: SpatialLimits, snapshot: &[u8], offset: usize, release: bool) -> Vec<u8> {
    let mut out=Vec::new();bytes(&mut out,1,token);bytes(&mut out,2,nonce);
    let op=limits.operations;
    for (field,value) in [(3,1),(4,6),(5,u64::from(op.jobs)),(6,u64::from(op.buildings)),(7,u64::from(op.items)),
        (8,op.payload_bytes as u64),(10,offset as u64),(11,op.page_bytes as u64),(12,u64::from(release))] { number(&mut out,field,value); }
    if !snapshot.is_empty() {bytes(&mut out,9,snapshot);}
    for (i,value) in limits.region.origin.into_iter().chain(limits.region.size).enumerate() { number(&mut out,13+i as u32,u64::from(value)); }
    out
}
fn envelope<'a>(data: &'a [u8], nonce: &[u8]) -> Result<(Message<'a>,JobsManifest)> {
    let m=Message::parse(data,14)?;
    if m.number(4)?!=1 || m.number(5)?!=6 {return Err(failure(ErrorCode::VersionMismatch,"spatial transport requires exactly protocol 1.6"));}
    if m.bytes(3,64)?!=nonce {return Err(malformed());}
    let accepted=m.number(1)?;let code=m.number(2)?;
    if accepted==0 && code!=0 {
        if (9..=14).any(|f|m.0.contains_key(&f)) {return Err(malformed());}
        return Err(failure(match code {1=>ErrorCode::CapabilityDenied,2=>ErrorCode::VersionMismatch,
            3=>ErrorCode::BudgetExceeded,4=>ErrorCode::FortressNotLoaded,6=>ErrorCode::StaleAnchor,_=>ErrorCode::AdapterFailure},
            "spatial capture refused; no partial world was published"));
    }
    if accepted!=1 || code!=0 {return Err(malformed());}
    let source=JobsManifest{generation:m.number(6)?,df_version:m.text(7)?,dfhack_version:m.text(8)?};
    if source.generation==0 || source.generation==u64::MAX {return Err(malformed());}
    Ok((m,source))
}
fn bind<S:Read+Write>(stream:&mut S,name:&str)->Result<i16>{
    let mut r=Vec::new();
    for (f,text) in [(1,name),(2,"dfmcp.spatial.v1_6.Request"),(3,"dfmcp.spatial.v1_6.Reply"),(4,"dfmcp_spatial_v1_6")] {bytes(&mut r,f,text.as_bytes());}
    let reply=call(stream,0,&r,1024)?;
    let id=i16::try_from(Message::parse(&reply,1)?.number(1)?).map_err(|_|malformed())?;
    if id<2 {return Err(malformed());}Ok(id)
}
fn page(data:&[u8],nonce:&[u8],maximum:usize)->Result<SnapshotPage>{
    let (m,s)=envelope(data,nonce)?;
    let complete=match m.number(14)?{0=>false,1=>true,_=>return Err(malformed())};
    Ok(SnapshotPage{manifest:SnapshotManifest{
        token:m.bytes(10,16)?.try_into().map_err(|_|malformed())?,generation:s.generation,
        df_version:s.df_version,dfhack_version:s.dfhack_version,
        total_bytes:usize::try_from(m.number(12)?).map_err(|_|malformed())?,
        payload_digest:Digest32::from_bytes(m.bytes(13,32)?.try_into().map_err(|_|malformed())?),
    },offset:usize::try_from(m.number(11)?).map_err(|_|malformed())?,bytes:m.bytes(9,maximum)?.to_vec(),complete})
}
/// Credentials have no Debug representation and never enter observations.
pub struct SpatialRpcClient<S>{stream:S,token:Vec<u8>,nonce:Vec<u8>,limits:SpatialLimits,
    manifest:JobsManifest,method:i16,poisoned:bool,last_pages:u32}
impl<S:Read+Write> SpatialRpcClient<S>{
    pub fn negotiate(mut stream:S,token:Vec<u8>,nonce:Vec<u8>,limits:SpatialLimits)->Result<Self>{
        limits.validate()?;
        if !(32..=256).contains(&token.len()) || !(16..=64).contains(&nonce.len()) {return Err(failure(ErrorCode::InvalidRequest,"invalid spatial credentials"));}
        let mut hello=b"DFHack?\n".to_vec();hello.extend_from_slice(&1i32.to_le_bytes());stream.write_all(&hello).map_err(io_failure)?;stream.flush().map_err(io_failure)?;
        let mut answer=[0;12];stream.read_exact(&mut answer).map_err(io_failure)?;
        if &answer[..8]!=b"DFHack!\n" || answer[8..]!=1i32.to_le_bytes(){return Err(malformed());}
        let handshake=bind(&mut stream,"Handshake")?;let method=bind(&mut stream,"ReadObservation")?;
        if handshake==method{return Err(malformed());}
        let reply=call(&mut stream,handshake,&request(&token,&nonce,limits,&[],0,false),4096)?;
        let (m,manifest)=envelope(&reply,&nonce)?;
        if (9..=14).any(|f|m.0.contains_key(&f)){return Err(malformed());}
        Ok(Self{stream,token,nonce,limits,manifest,method,poisoned:false,last_pages:0})
    }
    pub fn poisoned(&self)->bool{self.poisoned}
    pub fn fence(&mut self){self.poisoned=true;}
    pub fn last_page_count(&self)->u32{self.last_pages}
    pub fn read_observation(&mut self)->Result<LiveSpatialObservation>{
        if self.poisoned{return Err(failure(ErrorCode::AdapterUnavailable,"spatial source is fenced; reopen session"));}
        let result=self.acquire();if result.is_err(){self.fence();}result
    }
    fn acquire(&mut self)->Result<LiveSpatialObservation>{
        let op=self.limits.operations;let mut assembly=SnapshotAssembler::new(op.payload_bytes,op.page_bytes)?;let mut count=0;
        while !assembly.complete(){
            if count>=1024{return Err(failure(ErrorCode::BudgetExceeded,"spatial acquisition exceeded 1024 pages"));}
            let token=assembly.manifest().map_or(&[][..],|m|m.token.as_slice());
            let r=request(&self.token,&self.nonce,self.limits,token,assembly.offset(),false);
            let reply=call(&mut self.stream,self.method,&r,op.page_bytes+4096)?;
            let p=page(&reply,&self.nonce,op.page_bytes)?;
            if p.manifest.generation<self.manifest.generation || p.manifest.df_version!=self.manifest.df_version
                || p.manifest.dfhack_version!=self.manifest.dfhack_version {return Err(failure(ErrorCode::StaleAnchor,"spatial manifest changed incompatibly"));}
            assembly.push(p)?;count+=1;
        }
        let (manifest,payload)=assembly.finish()?;
        let value=LiveSpatialObservation::decode_payload(&payload,manifest.generation,manifest.df_version.clone(),manifest.dfhack_version.clone())?;
        let observed=value.operations();
        if observed.jobs.jobs.len()>op.jobs as usize || observed.buildings.len()>op.buildings as usize || observed.items.len()>op.items as usize
            || value.terrain().map.region!=self.limits.region {return Err(failure(ErrorCode::AdapterRejected,"spatial capture differs from negotiated region or rosters"));}
        let r=request(&self.token,&self.nonce,self.limits,&manifest.token,0,true);
        let reply=call(&mut self.stream,self.method,&r,4096)?;let (ack,source)=envelope(&reply,&self.nonce)?;
        if ack.bytes(10,16)?!=manifest.token.as_slice() || ack.0.contains_key(&9) || (11..=14).any(|f|ack.0.contains_key(&f))
            || source.generation!=manifest.generation || source.df_version!=manifest.df_version || source.dfhack_version!=manifest.dfhack_version {return Err(malformed());}
        self.manifest=source;self.last_pages=count;Ok(value)
    }
}
impl SpatialRpcClient<DeadlineStream>{
    pub fn connect(endpoint:SocketAddr,token:Vec<u8>,nonce:Vec<u8>,timeout:Duration,limits:SpatialLimits)->Result<Self>{
        limits.validate()?;checked_timeout(timeout)?;
        if !(32..=256).contains(&token.len()) || !(16..=64).contains(&nonce.len()){return Err(failure(ErrorCode::InvalidRequest,"invalid spatial credentials"));}
        if !endpoint.ip().is_loopback() || endpoint.port()==0{return Err(failure(ErrorCode::CapabilityDenied,"spatial endpoint must be numeric loopback"));}
        let deadline=Instant::now().checked_add(timeout).ok_or_else(||failure(ErrorCode::BudgetExceeded,"spatial deadline overflow"))?;
        let stream=TcpStream::connect_timeout(&endpoint,timeout).map_err(io_failure)?;stream.set_nodelay(true).map_err(io_failure)?;
        Self::negotiate(DeadlineStream{stream,deadline},token,nonce,limits)
    }
    pub fn refresh(&mut self,timeout:Duration)->Result<LiveSpatialObservation>{
        self.stream.deadline=Instant::now().checked_add(checked_timeout(timeout)?).ok_or_else(||failure(ErrorCode::BudgetExceeded,"spatial deadline overflow"))?;
        self.read_observation()
    }
}

#[cfg(test)]
mod tests{
    use super::*;
    use std::io::{self,Cursor};
    use crate::live_jobs_rpc::header;
    struct Script{input:Cursor<Vec<u8>>,output:Vec<u8>}
    impl Read for Script{fn read(&mut self,out:&mut[u8])->io::Result<usize>{let n=out.len().min(7);self.input.read(&mut out[..n])}}
    impl Write for Script{fn write(&mut self,b:&[u8])->io::Result<usize>{self.output.extend_from_slice(b);Ok(b.len())}fn flush(&mut self)->io::Result<()>{Ok(())}}
    fn fixture()->Result<Vec<u8>>{let s=include_str!("../tests/fixtures/spatial_v1_6.hex").trim();
        (0..s.len()).step_by(2).map(|i|u8::from_str_radix(&s[i..i+2],16).map_err(|_|malformed())).collect()}
    fn reply()->Vec<u8>{let mut r=Vec::new();for(f,n)in[(1,1),(2,0),(4,1),(5,6),(6,7)]{number(&mut r,f,n);}
        bytes(&mut r,3,&[b'n';16]);bytes(&mut r,7,b"df");bytes(&mut r,8,b"dfhack");r}
    fn frame(p:&[u8])->Vec<u8>{let mut v=header(-1,p.len() as i32).to_vec();v.extend_from_slice(p);v}
    fn limits()->SpatialLimits{SpatialLimits{operations:PagedOperationsLimits::default(),region:Region{origin:[0,0,5],size:[4,4,1]}}}
    fn script(corrupt:bool,release_ok:bool)->Result<Script>{
        let payload=fixture()?;let mut r=b"DFHack!\n".to_vec();r.extend_from_slice(&1i32.to_le_bytes());
        for id in [2,3]{let mut p=Vec::new();number(&mut p,1,id);r.extend(frame(&p));}r.extend(frame(&reply()));
        let mut p=reply();bytes(&mut p,9,&payload);bytes(&mut p,10,&[1;16]);number(&mut p,11,0);number(&mut p,12,payload.len() as u64);
        let digest=if corrupt{Digest32::ZERO}else{Digest32::of_bytes(&payload)};bytes(&mut p,13,digest.as_bytes());number(&mut p,14,1);r.extend(frame(&p));
        let mut p=reply();bytes(&mut p,10,if release_ok{&[1;16]}else{&[2;16]});r.extend(frame(&p));
        Ok(Script{input:Cursor::new(r),output:Vec::new()})
    }
    #[test]fn fragmented_capture_and_release_then_terminal_fencing()->Result<()>{
        let mut c=SpatialRpcClient::negotiate(script(false,true)?,vec![b't';32],vec![b'n';16],limits())?;
        let v=c.read_observation()?;assert_eq!(v.operations().items.len(),3);assert_eq!(v.terrain().map.cells.len(),16);assert_eq!(c.last_page_count(),1);
        assert!(c.read_observation().is_err());let n=c.stream.output.len();assert!(c.poisoned());assert!(c.read_observation().is_err());assert_eq!(c.stream.output.len(),n);Ok(())
    }
    #[test]fn corrupt_payload_or_release_never_returns_a_world()->Result<()>{
        for (corrupt,release_ok) in [(true,true),(false,false)]{
            let mut c=SpatialRpcClient::negotiate(script(corrupt,release_ok)?,vec![b't';32],vec![b'n';16],limits())?;
            assert!(c.read_observation().is_err());assert!(c.poisoned());assert_eq!(c.last_page_count(),0);
        }Ok(())
    }
    #[test]fn negotiated_region_is_checked_before_publication()->Result<()>{
        let mut limits=limits();limits.region.size[0]=3;
        let mut c=SpatialRpcClient::negotiate(script(false,true)?,vec![b't';32],vec![b'n';16],limits)?;
        assert!(c.read_observation().is_err());assert!(c.poisoned());Ok(())
    }
    #[test]fn requests_bind_all_region_coordinates_and_refuse_foreign_nonce()->Result<()>{
        let r=request(&[b't';32],&[b'n';16],limits(),&[],0,false);let m=Message::parse(&r,18)?;
        for (field,n) in [(13,0),(14,0),(15,5),(16,4),(17,4),(18,1)]{assert_eq!(m.number(field)?,n);}
        assert!(envelope(&reply(),&[b'x';16]).is_err());
        assert!(SpatialRpcClient::connect(SocketAddr::from(([192,0,2,1],5000)),vec![b't';32],vec![b'n';16],Duration::from_millis(1),limits()).is_err());Ok(())
    }
}
