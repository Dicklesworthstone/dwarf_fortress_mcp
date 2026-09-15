#![forbid(unsafe_code)]

//! Closed control/1.7 client. Only pause prepare/commit/query are bindable.

use crate::live_jobs_rpc::{DeadlineStream, JobsManifest, Message, bytes, call, checked_timeout, io_failure, number};
use dfmcp_core::{DfmcpError, Digest32, ErrorCode, Result};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

fn invalid(message:&str)->DfmcpError{DfmcpError::new(ErrorCode::AdapterRejected,message)}
fn request(token:&[u8],nonce:&[u8],key:Option<&str>,digest:Option<Digest32>,prepare:Option<&[u8]>,paused:Option<bool>,tick:Option<u64>)->Vec<u8>{
    let mut out=Vec::new();bytes(&mut out,1,token);bytes(&mut out,2,nonce);number(&mut out,3,1);number(&mut out,4,7);
    if let Some(key)=key{bytes(&mut out,5,key.as_bytes());}if let Some(digest)=digest{bytes(&mut out,6,digest.as_bytes());}
    if let Some(prepare)=prepare{bytes(&mut out,7,prepare);}if let Some(paused)=paused{number(&mut out,8,u64::from(paused));}
    if let Some(tick)=tick{number(&mut out,9,tick);}out
}
fn bind<S:Read+Write>(stream:&mut S,name:&str)->Result<i16>{
    let mut body=Vec::new();for(f,v)in[(1,name),(2,"dfmcp.control.v1_7.Request"),(3,"dfmcp.control.v1_7.Reply"),(4,"dfmcp_control_v1_7")]{bytes(&mut body,f,v.as_bytes());}
    let response=call(stream,0,&body,1024)?;let id=i16::try_from(Message::parse(&response,1)?.number(1)?).map_err(|_|invalid("control method ID overflow"))?;
    if id<2{return Err(invalid("control method resolved to reserved core ID"));}Ok(id)
}
#[derive(Clone,Debug,PartialEq,Eq)]
pub struct PauseEffect{
    pub bridge_generation:u64,
    pub known:bool,
    pub applied:bool,
    pub paused:bool,
    pub observed_tick:u64,
    pub prepare_token:Vec<u8>,
    pub receipt_digest:Vec<u8>,
}
fn decode(data:&[u8],nonce:&[u8])->Result<(JobsManifest,PauseEffect)>{
    let m=Message::parse(data,14)?;if m.number(4)?!=1||m.number(5)?!=7{return Err(DfmcpError::new(ErrorCode::VersionMismatch,"control bridge must be exactly 1.7"));}
    if m.bytes(3,64)?!=nonce{return Err(invalid("control nonce mismatch"));}let accepted=m.number(1)?;let code=m.number(2)?;
    if accepted==0{return Err(DfmcpError::new(match code{1=>ErrorCode::CapabilityDenied,2=>ErrorCode::VersionMismatch,3=>ErrorCode::InvalidRequest,4=>ErrorCode::FortressNotLoaded,6=>ErrorCode::StaleAnchor,7=>ErrorCode::Conflict,_=>ErrorCode::AdapterFailure},"control bridge rejected request"));}
    if accepted!=1||code!=0{return Err(invalid("inconsistent control acceptance"));}
    let manifest=JobsManifest{generation:m.number(6)?,df_version:m.text(7)?,dfhack_version:m.text(8)?};if manifest.generation==0||manifest.generation==u64::MAX{return Err(invalid("invalid control generation"));}
    let token=m.0.get(&9).map(|_|m.bytes(9,16).map(ToOwned::to_owned)).transpose()?.unwrap_or_default();
    let known=m.0.get(&10).is_some_and(|_|m.number(10).ok()==Some(1));let applied=m.0.get(&11).is_some_and(|_|m.number(11).ok()==Some(1));
    let paused=m.0.get(&12).is_some_and(|_|m.number(12).ok()==Some(1));let observed_tick=if m.0.contains_key(&13){m.number(13)?}else{0};
    let receipt=m.0.get(&14).map(|_|m.bytes(14,32).map(ToOwned::to_owned)).transpose()?.unwrap_or_default();
    Ok((manifest.clone(),PauseEffect{bridge_generation:manifest.generation,known,applied,paused,observed_tick,prepare_token:token,receipt_digest:receipt}))
}
pub struct ControlRpcClient<S>{stream:S,token:Vec<u8>,nonce:Vec<u8>,manifest:JobsManifest,prepare:i16,commit:i16,query:i16,fenced:bool}
impl<S:Read+Write> ControlRpcClient<S>{
    pub fn negotiate(mut stream:S,token:Vec<u8>,nonce:Vec<u8>)->Result<Self>{
        if !(32..=256).contains(&token.len())||!(16..=64).contains(&nonce.len()){return Err(DfmcpError::new(ErrorCode::InvalidRequest,"invalid control credentials"));}
        let mut hello=b"DFHack?\n".to_vec();hello.extend_from_slice(&1i32.to_le_bytes());stream.write_all(&hello).map_err(io_failure)?;stream.flush().map_err(io_failure)?;
        let mut reply=[0;12];stream.read_exact(&mut reply).map_err(io_failure)?;if &reply[..8]!=b"DFHack!\n"||reply[8..]!=1i32.to_le_bytes(){return Err(invalid("invalid control handshake"));}
        let handshake=bind(&mut stream,"Handshake")?;let prepare=bind(&mut stream,"PreparePause")?;let commit=bind(&mut stream,"CommitPause")?;let query=bind(&mut stream,"QueryPause")?;
        if [handshake,prepare,commit,query].iter().enumerate().any(|(i,v)|[handshake,prepare,commit,query][..i].contains(v)){return Err(invalid("control method IDs alias"));}
        let response=call(&mut stream,handshake,&request(&token,&nonce,None,None,None,None,None),4096)?;let (manifest,_)=decode(&response,&nonce)?;
        Ok(Self{stream,token,nonce,manifest,prepare,commit,query,fenced:false})
    }
    pub fn poisoned(&self)->bool{self.fenced}
    pub fn fence(&mut self){self.fenced=true;}
    pub fn bridge_generation(&self)->u64{self.manifest.generation}
    pub fn manifest(&self)->&JobsManifest{&self.manifest}
    fn invoke(&mut self,method:i16,key:&str,digest:Digest32,prepare:Option<&[u8]>,paused:Option<bool>,tick:Option<u64>)->Result<PauseEffect>{
        if self.fenced{return Err(DfmcpError::new(ErrorCode::AdapterUnavailable,"control source fenced; reopen session"));}
        if key.is_empty()||key.len()>512||key.chars().any(char::is_control){return Err(DfmcpError::new(ErrorCode::InvalidRequest,"invalid idempotency key"));}
        let result=(||{let response=call(&mut self.stream,method,&request(&self.token,&self.nonce,Some(key),Some(digest),prepare,paused,tick),4096)?;
            let (manifest,effect)=decode(&response,&self.nonce)?;if manifest.generation<self.manifest.generation||manifest.df_version!=self.manifest.df_version||manifest.dfhack_version!=self.manifest.dfhack_version{return Err(DfmcpError::new(ErrorCode::StaleAnchor,"control source version or generation changed"));}
            self.manifest=manifest;Ok(effect)})();
        if result.as_ref().err().is_some_and(|error|matches!(error.code,
            ErrorCode::AdapterUnavailable|ErrorCode::AdapterFailure|ErrorCode::AdapterRejected|ErrorCode::VersionMismatch|ErrorCode::StaleAnchor|ErrorCode::CapabilityDenied)){
            self.fenced=true;
        }
        result
    }
    pub fn prepare_pause(&mut self,key:&str,digest:Digest32,paused:bool,tick:u64)->Result<PauseEffect>{self.invoke(self.prepare,key,digest,None,Some(paused),Some(tick))}
    pub fn commit_pause(&mut self,key:&str,digest:Digest32,prepare:&[u8])->Result<PauseEffect>{self.invoke(self.commit,key,digest,Some(prepare),None,None)}
    pub fn query_pause(&mut self,key:&str,digest:Digest32)->Result<PauseEffect>{self.invoke(self.query,key,digest,None,None,None)}
}
impl ControlRpcClient<DeadlineStream>{
    pub fn connect(endpoint:SocketAddr,token:Vec<u8>,nonce:Vec<u8>,timeout:Duration)->Result<Self>{checked_timeout(timeout)?;if !endpoint.ip().is_loopback()||endpoint.port()==0{return Err(DfmcpError::new(ErrorCode::CapabilityDenied,"control endpoint must be numeric loopback"));}
        let deadline=Instant::now().checked_add(timeout).ok_or_else(||invalid("control deadline overflow"))?;let stream=TcpStream::connect_timeout(&endpoint,timeout).map_err(io_failure)?;stream.set_nodelay(true).map_err(io_failure)?;
        Self::negotiate(DeadlineStream::new(stream,deadline),token,nonce)}
}
