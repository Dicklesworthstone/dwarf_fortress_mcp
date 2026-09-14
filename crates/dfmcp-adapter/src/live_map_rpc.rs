//! Closed map/1.5 client, nested below operations solely to reuse bounded native
//! framing and the absolute-deadline stream. No caller-selected methods exist.

use super::super::{Message,JobsManifest,DeadlineStream,bytes,number,call,checked_timeout,io_failure};
use std::io::{Read,Write};
use std::net::{SocketAddr,TcpStream};
use std::time::{Duration,Instant};
use dfmcp_core::{DfmcpError,ErrorCode,Result};
use dfmcp_world::map_region::Region;
use crate::live_map::{LiveMapObservation,MAX_MAP_BYTES,map_error};

fn invalid(message:&str)->DfmcpError{DfmcpError::new(ErrorCode::AdapterRejected,message)}
fn configuration(token:&[u8],nonce:&[u8],region:Region,maximum:usize)->Result<()>{
    region.volume().map_err(map_error)?;
    if !(32..=256).contains(&token.len())||!(16..=64).contains(&nonce.len())||!(1024..=MAX_MAP_BYTES).contains(&maximum){
        return Err(DfmcpError::new(ErrorCode::InvalidRequest,"invalid map credentials or byte budget"));}
    Ok(())
}
fn request(token:&[u8],nonce:&[u8],region:Region,maximum:usize)->Vec<u8>{
    let mut out=Vec::new();bytes(&mut out,1,token);bytes(&mut out,2,nonce);number(&mut out,3,1);number(&mut out,4,5);
    for (index,v) in region.origin.into_iter().chain(region.size).enumerate(){number(&mut out,index as u32+5,u64::from(v));}
    number(&mut out,11,maximum as u64);out
}
fn bind<S:Read+Write>(stream:&mut S,method:&str)->Result<i16>{
    let mut input=Vec::new();for (f,v) in [(1,method),(2,"dfmcp.map.v1_5.Request"),(3,"dfmcp.map.v1_5.Reply"),(4,"dfmcp_map_v1_5")]{bytes(&mut input,f,v.as_bytes());}
    let reply=call(stream,0,&input,1024)?;
    let id=i16::try_from(Message::parse(&reply,1)?.number(1)?).map_err(|_|invalid("map method ID overflow"))?;
    if id<2{return Err(invalid("map method resolved to reserved native core method"));}Ok(id)
}
fn decode<'a>(data:&'a [u8],nonce:&[u8],read:bool,maximum:usize)->Result<(JobsManifest,Option<&'a [u8]>)>{
    let m=Message::parse(data,9)?;
    if m.number(4)?!=1||m.number(5)?!=5{return Err(DfmcpError::new(ErrorCode::VersionMismatch,"map bridge must be exactly 1.5"));}
    if m.bytes(3,64)?!=nonce{return Err(invalid("map nonce mismatch"));}
    let accepted=m.number(1)?;let code=m.number(2)?;
    if accepted==0{
        if code==0||m.0.contains_key(&9){return Err(invalid("inconsistent rejected map reply"));}
        return Err(DfmcpError::new(match code{1=>ErrorCode::CapabilityDenied,2=>ErrorCode::VersionMismatch,
            3=>ErrorCode::BudgetExceeded,4=>ErrorCode::FortressNotLoaded,_=>ErrorCode::AdapterFailure},"map bridge refused; no region published"));
    }
    if accepted!=1||code!=0{return Err(invalid("invalid map acceptance fields"));}
    let manifest=JobsManifest{generation:m.number(6)?,df_version:m.text(7)?,dfhack_version:m.text(8)?};
    if manifest.generation==0||manifest.generation==u64::MAX{return Err(invalid("invalid map generation"));}
    let payload=if read{Some(m.bytes(9,maximum)?)}else if m.0.contains_key(&9){return Err(invalid("map handshake carried terrain"));}else{None};
    Ok((manifest,payload))
}

pub struct MapRpcClient<S>{stream:S,token:Vec<u8>,nonce:Vec<u8>,region:Region,maximum:usize,
    manifest:JobsManifest,method:i16,fenced:bool}
impl<S:Read+Write> MapRpcClient<S>{
    pub fn negotiate(mut stream:S,token:Vec<u8>,nonce:Vec<u8>,region:Region,maximum:usize)->Result<Self>{
        configuration(&token,&nonce,region,maximum)?;
        let mut hello=b"DFHack?\n".to_vec();hello.extend_from_slice(&1i32.to_le_bytes());
        stream.write_all(&hello).map_err(io_failure)?;stream.flush().map_err(io_failure)?;
        let mut reply=[0;12];stream.read_exact(&mut reply).map_err(io_failure)?;
        if &reply[..8]!=b"DFHack!\n"||reply[8..]!=1i32.to_le_bytes(){return Err(invalid("invalid map native handshake"));}
        let handshake=bind(&mut stream,"Handshake")?;let method=bind(&mut stream,"ReadObservation")?;
        if handshake==method{return Err(invalid("map native methods alias"));}
        let reply=call(&mut stream,handshake,&request(&token,&nonce,region,maximum),4096)?;
        let (manifest,_)=decode(&reply,&nonce,false,maximum)?;
        Ok(Self{stream,token,nonce,region,maximum,manifest,method,fenced:false})
    }
    pub fn poisoned(&self)->bool{self.fenced}
    pub fn fence(&mut self){self.fenced=true;}
    pub fn read_observation(&mut self)->Result<LiveMapObservation>{
        if self.fenced{return Err(DfmcpError::new(ErrorCode::AdapterUnavailable,"map source fenced; reopen session"));}
        let result=(||{
            let reply=call(&mut self.stream,self.method,&request(&self.token,&self.nonce,self.region,self.maximum),self.maximum+4096)?;
            let (manifest,payload)=decode(&reply,&self.nonce,true,self.maximum)?;
            if manifest.generation<self.manifest.generation||manifest.df_version!=self.manifest.df_version||manifest.dfhack_version!=self.manifest.dfhack_version{
                return Err(DfmcpError::new(ErrorCode::StaleAnchor,"map software or generation changed incompatibly"));}
            let observation=LiveMapObservation::decode_payload(payload.ok_or_else(||invalid("map payload absent"))?,manifest.generation,
                manifest.df_version.clone(),manifest.dfhack_version.clone())?;
            if observation.map.region!=self.region{return Err(invalid("map bridge returned a different region"));}
            self.manifest=manifest;Ok(observation)
        })();if result.is_err(){self.fenced=true;}result
    }
}
impl MapRpcClient<DeadlineStream>{
    pub fn connect(endpoint:SocketAddr,token:Vec<u8>,nonce:Vec<u8>,region:Region,maximum:usize,timeout:Duration)->Result<Self>{
        configuration(&token,&nonce,region,maximum)?;checked_timeout(timeout)?;
        if !endpoint.ip().is_loopback()||endpoint.port()==0{return Err(DfmcpError::new(ErrorCode::CapabilityDenied,"map endpoint must be numeric loopback"));}
        let deadline=Instant::now().checked_add(timeout).ok_or_else(||invalid("map deadline overflow"))?;
        let stream=TcpStream::connect_timeout(&endpoint,timeout).map_err(io_failure)?;stream.set_nodelay(true).map_err(io_failure)?;
        Self::negotiate(DeadlineStream{stream,deadline},token,nonce,region,maximum)
    }
    pub fn refresh(&mut self,timeout:Duration)->Result<LiveMapObservation>{
        self.stream.deadline=Instant::now().checked_add(checked_timeout(timeout)?).ok_or_else(||invalid("map deadline overflow"))?;
        self.read_observation()
    }
}

#[cfg(test)]
mod tests{
    use super::*;use std::io::{self,Cursor};use super::super::super::header;
    struct Script{input:Cursor<Vec<u8>>,output:Vec<u8>}
    impl Read for Script{fn read(&mut self,out:&mut [u8])->io::Result<usize>{self.input.read(out)}}
    impl Write for Script{fn write(&mut self,v:&[u8])->io::Result<usize>{self.output.extend_from_slice(v);Ok(v.len())}fn flush(&mut self)->io::Result<()>{Ok(())}}
    fn reply(minor:u64,payload:Option<&[u8]>)->Vec<u8>{let mut out=Vec::new();for(f,v)in[(1,1),(2,0),(4,1),(5,minor),(6,7)]{number(&mut out,f,v);}
        bytes(&mut out,3,&[b'n';16]);bytes(&mut out,7,b"df");bytes(&mut out,8,b"dfhack");if let Some(p)=payload{bytes(&mut out,9,p);}out}
    fn frame(v:&[u8])->Vec<u8>{let mut b=header(-1,v.len() as i32).to_vec();b.extend_from_slice(v);b}
    fn golden()->Result<Vec<u8>>{let h=include_str!("../tests/fixtures/map_v1_5.hex").trim();
        (0..h.len()).step_by(2).map(|i|u8::from_str_radix(&h[i..i+2],16).map_err(|_|invalid("bad test hex"))).collect()}
    #[test]
    fn actual_wire_decodes_native_fixture_and_fences_truncation()->Result<()>{
        let mut input=b"DFHack!\n".to_vec();input.extend_from_slice(&1i32.to_le_bytes());
        for id in [2,3]{let mut v=Vec::new();number(&mut v,1,id);input.extend(frame(&v));}
        input.extend(frame(&reply(5,None)));input.extend(frame(&reply(5,Some(&golden()?))));
        let mut c=MapRpcClient::negotiate(Script{input:Cursor::new(input),output:Vec::new()},vec![b't';32],vec![b'n';16],
            Region{origin:[14,15,1],size:[4,3,2]},MAX_MAP_BYTES)?;
        assert_eq!(c.read_observation()?.map.cells.len(),24);assert!(!c.poisoned());assert!(c.read_observation().is_err());
        let sent=c.stream.output.len();assert!(c.poisoned());assert!(c.read_observation().is_err());assert_eq!(sent,c.stream.output.len());Ok(())
    }
    #[test]
    fn nonce_protocol_and_payload_roles_are_not_interchangeable(){
        assert!(decode(&reply(4,None),&[b'n';16],false,1024).is_err());assert!(decode(&reply(5,None),&[b'x';16],false,1024).is_err());
        assert!(decode(&reply(5,Some(b"x")),&[b'n';16],false,1024).is_err());assert!(decode(&reply(5,None),&[b'n';16],true,1024).is_err());
    }
}
