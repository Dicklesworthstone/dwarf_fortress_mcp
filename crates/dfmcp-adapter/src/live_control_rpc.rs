#![forbid(unsafe_code)]

//! Closed control/1.7 client. Only the fixed pause prepare/commit/query methods
//! can be bound. This module owns its framing so mutation transport does not rely
//! on private implementation details of an unrelated read profile.

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, Result};

const MAX_RPC_BYTES: usize = 8 * 1024;
const MAX_TEXT_NOTIFICATION_BYTES: usize = 64 * 1024;
const MAX_TEXT_NOTIFICATION_TOTAL_BYTES: usize = 256 * 1024;
const MAX_NOTIFICATIONS: usize = 8;
const PLUGIN: &str = "dfmcp_control_v1_7";
const REQUEST_TYPE: &str = "dfmcp.control.v1_7.Request";
const REPLY_TYPE: &str = "dfmcp.control.v1_7.Reply";

fn invalid(message:&str)->DfmcpError{DfmcpError::new(ErrorCode::AdapterRejected,message)}
fn failure(code:ErrorCode,message:&str)->DfmcpError{DfmcpError::new(code,message)}
fn malformed()->DfmcpError{invalid("invalid control/1.7 native RPC framing or protobuf")}
fn io_failure(_:io::Error)->DfmcpError{failure(ErrorCode::AdapterUnavailable,"control RPC I/O failed or exceeded its deadline")}
fn checked_timeout(timeout:Duration)->Result<Duration>{
    if timeout<Duration::from_millis(1)||timeout>Duration::from_secs(60){return Err(failure(ErrorCode::BudgetExceeded,"control deadline must be 1..60000 milliseconds"));}
    Ok(timeout)
}
fn remaining_timeout(timeout:Duration,elapsed:Duration)->Result<Duration>{
    checked_timeout(timeout)?;
    timeout.checked_sub(elapsed).filter(|value|*value>=Duration::from_millis(1))
        .ok_or_else(||failure(ErrorCode::BudgetExceeded,"control deadline exhausted before handshake"))
}
fn varint(out:&mut Vec<u8>,mut value:u64){while value>=128{out.push((value as u8&127)|128);value>>=7;}out.push(value as u8);}
fn number(out:&mut Vec<u8>,field:u32,value:u64){varint(out,u64::from(field)<<3);varint(out,value);}
fn bytes(out:&mut Vec<u8>,field:u32,value:&[u8]){varint(out,(u64::from(field)<<3)|2);varint(out,value.len() as u64);out.extend_from_slice(value);}
fn read_varint(input:&[u8],offset:&mut usize)->Result<u64>{
    let mut result=0u64;
    for index in 0..10{
        let byte=*input.get(*offset).ok_or_else(malformed)?;*offset+=1;
        if index==9&&byte>1{return Err(malformed());}
        result|=u64::from(byte&127)<<(index*7);
        if byte<128{if index>0&&byte==0{return Err(malformed());}return Ok(result);}
    }
    Err(malformed())
}
#[derive(Clone,Copy)]enum Field<'a>{Number(u64),Bytes(&'a[u8])}
struct Message<'a>(BTreeMap<u32,Field<'a>>);
impl<'a> Message<'a>{
    fn parse(input:&'a[u8],maximum_field:u32)->Result<Self>{
        if input.len()>MAX_RPC_BYTES{return Err(failure(ErrorCode::BudgetExceeded,"control RPC payload exceeds 8 KiB"));}
        let mut offset=0;let mut fields=BTreeMap::new();
        while offset<input.len(){
            let key=read_varint(input,&mut offset)?;let field=u32::try_from(key>>3).map_err(|_|malformed())?;
            if field==0||field>maximum_field||fields.contains_key(&field){return Err(malformed());}
            let value=match key&7{
                0=>Field::Number(read_varint(input,&mut offset)?),
                2=>{let length=usize::try_from(read_varint(input,&mut offset)?).map_err(|_|malformed())?;
                    let end=offset.checked_add(length).ok_or_else(malformed)?;let value=input.get(offset..end).ok_or_else(malformed)?;offset=end;Field::Bytes(value)},
                _=>return Err(malformed()),
            };
            fields.insert(field,value);
        }
        Ok(Self(fields))
    }
    fn has(&self,field:u32)->bool{self.0.contains_key(&field)}
    fn number(&self,field:u32)->Result<u64>{match self.0.get(&field){Some(Field::Number(value))=>Ok(*value),_=>Err(malformed())}}
    fn boolean(&self,field:u32)->Result<bool>{match self.number(field)?{0=>Ok(false),1=>Ok(true),_=>Err(malformed())}}
    fn bytes(&self,field:u32,maximum:usize)->Result<&'a[u8]>{match self.0.get(&field){Some(Field::Bytes(value))if value.len()<=maximum=>Ok(value),_=>Err(malformed())}}
    fn text(&self,field:u32,maximum:usize)->Result<String>{
        let value=std::str::from_utf8(self.bytes(field,maximum)?).map_err(|_|malformed())?;
        if value.is_empty()||value.contains('\0'){return Err(malformed());}Ok(value.to_owned())
    }
}
fn header(id:i16,length:i32)->[u8;8]{let mut out=[0u8;8];out[..2].copy_from_slice(&id.to_le_bytes());out[4..].copy_from_slice(&length.to_le_bytes());out}
fn call<S:Read+Write>(stream:&mut S,method:i16,request:&[u8],limit:usize)->Result<Vec<u8>>{
    if request.len()>MAX_RPC_BYTES||limit>MAX_RPC_BYTES{return Err(failure(ErrorCode::BudgetExceeded,"control RPC request or reply limit exceeds 8 KiB"));}
    let length=i32::try_from(request.len()).map_err(|_|malformed())?;
    stream.write_all(&header(method,length)).map_err(io_failure)?;stream.write_all(request).map_err(io_failure)?;stream.flush().map_err(io_failure)?;
    let mut notification_bytes=0usize;
    for _ in 0..=MAX_NOTIFICATIONS{
        let mut head=[0u8;8];stream.read_exact(&mut head).map_err(io_failure)?;
        let id=i16::from_le_bytes([head[0],head[1]]);let signed=i32::from_le_bytes([head[4],head[5],head[6],head[7]]);
        if id==-2{return Err(failure(ErrorCode::AdapterFailure,"DFHack rejected the control RPC"));}
        if id!=-1&&id!=-3{return Err(malformed());}
        let length=usize::try_from(signed).map_err(|_|malformed())?;
        let ceiling=if id==-3{MAX_TEXT_NOTIFICATION_BYTES}else{limit};
        if length>ceiling{return Err(failure(ErrorCode::BudgetExceeded,"control RPC reply exceeds its byte bound"));}
        let mut payload=vec![0u8;length];stream.read_exact(&mut payload).map_err(io_failure)?;
        if id==-1{return Ok(payload);}
        notification_bytes=notification_bytes.checked_add(length).ok_or_else(malformed)?;
        if notification_bytes>MAX_TEXT_NOTIFICATION_TOTAL_BYTES{return Err(failure(ErrorCode::BudgetExceeded,"control RPC text notifications exceeded their aggregate bound"));}
    }
    Err(failure(ErrorCode::BudgetExceeded,"control RPC emitted too many text notifications"))
}
fn bind<S:Read+Write>(stream:&mut S,name:&str)->Result<i16>{
    let mut body=Vec::new();for(field,value)in[(1,name),(2,REQUEST_TYPE),(3,REPLY_TYPE),(4,PLUGIN)]{bytes(&mut body,field,value.as_bytes());}
    let response=call(stream,0,&body,1024)?;let id=i16::try_from(Message::parse(&response,1)?.number(1)?).map_err(|_|invalid("control method ID overflow"))?;
    if id<2{return Err(invalid("control method resolved to reserved core ID"));}Ok(id)
}
fn request(token:&[u8],nonce:&[u8],key:Option<&str>,digest:Option<Digest32>,prepare:Option<&[u8]>,paused:Option<bool>,tick:Option<u64>)->Vec<u8>{
    let mut out=Vec::new();bytes(&mut out,1,token);bytes(&mut out,2,nonce);number(&mut out,3,1);number(&mut out,4,7);
    if let Some(key)=key{bytes(&mut out,5,key.as_bytes());}if let Some(digest)=digest{bytes(&mut out,6,digest.as_bytes());}
    if let Some(prepare)=prepare{bytes(&mut out,7,prepare);}if let Some(paused)=paused{number(&mut out,8,u64::from(paused));}
    if let Some(tick)=tick{number(&mut out,9,tick);}out
}

#[derive(Clone,Debug,PartialEq,Eq)]
pub struct ControlManifest{pub generation:u64,pub df_version:String,pub dfhack_version:String}
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
fn decode(data:&[u8],nonce:&[u8])->Result<(ControlManifest,PauseEffect)>{
    let message=Message::parse(data,14)?;
    if message.number(4)?!=1||message.number(5)?!=7{return Err(failure(ErrorCode::VersionMismatch,"control bridge must be exactly profile 1.7"));}
    if message.bytes(3,64)?!=nonce{return Err(invalid("control nonce mismatch"));}
    let accepted=message.boolean(1)?;let code=message.number(2)?;
    if !accepted{
        if code==0||[9,10,11,12,13,14].into_iter().any(|field|message.has(field)){return Err(malformed());}
        return Err(failure(match code{1=>ErrorCode::CapabilityDenied,2=>ErrorCode::VersionMismatch,3=>ErrorCode::InvalidRequest,
            4=>ErrorCode::FortressNotLoaded,6=>ErrorCode::StaleAnchor,7=>ErrorCode::Conflict,_=>ErrorCode::AdapterFailure},
            "control bridge rejected the request without a mutation receipt"));
    }
    if code!=0&&code!=5{return Err(malformed());}
    let manifest=ControlManifest{generation:message.number(6)?,df_version:message.text(7,128)?,dfhack_version:message.text(8,128)?};
    if manifest.generation==0||manifest.generation==u64::MAX{return Err(invalid("invalid control generation"));}
    let prepare_token=if message.has(9){message.bytes(9,16)?.to_vec()}else{Vec::new()};
    let known=if message.has(10){message.boolean(10)?}else{false};
    let applied=if message.has(11){message.boolean(11)?}else{false};
    let paused=if message.has(12){message.boolean(12)?}else{false};
    let observed_tick=if message.has(13){message.number(13)?}else{0};
    let receipt_digest=if message.has(14){message.bytes(14,32)?.to_vec()}else{Vec::new()};
    if message.has(9)&&prepare_token.len()!=16{return Err(malformed());}
    if message.has(14)&&receipt_digest.len()!=32{return Err(malformed());}
    // Missing fields are not observations of false/zero. A known record must
    // explicitly carry its outcome and observation, including for pending work.
    if known&&[11,12,13].into_iter().any(|field|!message.has(field)){return Err(malformed());}
    if applied&&(!known||receipt_digest.is_empty()){return Err(malformed());}
    if code==5&&(!known||applied){return Err(malformed());}
    if !known&&(message.has(11)||message.has(12)||message.has(13)||message.has(14)){return Err(malformed());}
    Ok((manifest.clone(),PauseEffect{bridge_generation:manifest.generation,known,applied,paused,observed_tick,prepare_token,receipt_digest}))
}

pub struct ControlRpcClient<S>{stream:S,token:Vec<u8>,nonce:Vec<u8>,manifest:ControlManifest,prepare:i16,commit:i16,query:i16,fenced:bool}
impl<S:Read+Write> ControlRpcClient<S>{
    pub fn negotiate(mut stream:S,token:Vec<u8>,nonce:Vec<u8>)->Result<Self>{
        if !(32..=256).contains(&token.len())||!(16..=64).contains(&nonce.len()){return Err(failure(ErrorCode::InvalidRequest,"invalid control credentials"));}
        let mut hello=b"DFHack?\n".to_vec();hello.extend_from_slice(&1i32.to_le_bytes());
        stream.write_all(&hello).map_err(io_failure)?;stream.flush().map_err(io_failure)?;
        let mut reply=[0u8;12];stream.read_exact(&mut reply).map_err(io_failure)?;
        if &reply[..8]!=b"DFHack!\n"||reply[8..]!=1i32.to_le_bytes(){return Err(invalid("invalid control native handshake"));}
        let handshake=bind(&mut stream,"Handshake")?;let prepare=bind(&mut stream,"PreparePause")?;
        let commit=bind(&mut stream,"CommitPause")?;let query=bind(&mut stream,"QueryPause")?;
        let ids=[handshake,prepare,commit,query];
        if ids.iter().enumerate().any(|(index,value)|ids[..index].contains(value)){return Err(invalid("control native method IDs alias"));}
        let response=call(&mut stream,handshake,&request(&token,&nonce,None,None,None,None,None),4096)?;
        let (manifest,effect)=decode(&response,&nonce)?;
        if effect.known||!effect.prepare_token.is_empty()||!effect.receipt_digest.is_empty(){return Err(malformed());}
        Ok(Self{stream,token,nonce,manifest,prepare,commit,query,fenced:false})
    }
    pub fn poisoned(&self)->bool{self.fenced}
    pub fn fence(&mut self){self.fenced=true;}
    pub fn bridge_generation(&self)->u64{self.manifest.generation}
    pub fn manifest(&self)->&ControlManifest{&self.manifest}
    fn invoke(&mut self,method:i16,key:&str,digest:Digest32,prepare:Option<&[u8]>,paused:Option<bool>,tick:Option<u64>)->Result<PauseEffect>{
        if self.fenced{return Err(failure(ErrorCode::AdapterUnavailable,"control source fenced; reopen session"));}
        if key.is_empty()||key.len()>512||key.chars().any(char::is_control){return Err(failure(ErrorCode::InvalidRequest,"invalid control idempotency key"));}
        let result=(||{
            let response=call(&mut self.stream,method,&request(&self.token,&self.nonce,Some(key),Some(digest),prepare,paused,tick),4096)?;
            let (manifest,effect)=decode(&response,&self.nonce)?;
            if manifest.generation<self.manifest.generation||manifest.df_version!=self.manifest.df_version||manifest.dfhack_version!=self.manifest.dfhack_version{
                return Err(failure(ErrorCode::StaleAnchor,"control source software changed or generation regressed"));
            }
            self.manifest=manifest;Ok(effect)
        })();
        // A bounded decoder may stop before consuming the frame. Fence EVERY
        // failed wire call so unread bytes cannot become another request's reply.
        // Local validation above remains non-I/O and does not poison the stream.
        if result.is_err(){self.fenced=true;}
        result
    }
    pub fn prepare_pause(&mut self,key:&str,digest:Digest32,paused:bool,tick:u64)->Result<PauseEffect>{self.invoke(self.prepare,key,digest,None,Some(paused),Some(tick))}
    pub fn commit_pause(&mut self,key:&str,digest:Digest32,prepare:&[u8])->Result<PauseEffect>{
        if prepare.len()!=16{return Err(failure(ErrorCode::InvalidRequest,"control prepare token must contain exactly 16 bytes"));}
        self.invoke(self.commit,key,digest,Some(prepare),None,None)
    }
    pub fn query_pause(&mut self,key:&str,digest:Digest32)->Result<PauseEffect>{self.invoke(self.query,key,digest,None,None,None)}
}

pub struct ControlDeadlineStream{stream:TcpStream,deadline:Instant}
impl ControlDeadlineStream{
    fn new(stream:TcpStream,timeout:Duration)->Result<Self>{
        let deadline=Instant::now().checked_add(checked_timeout(timeout)?).ok_or_else(||failure(ErrorCode::BudgetExceeded,"control deadline overflow"))?;
        Ok(Self{stream,deadline})
    }
    fn reset(&mut self,timeout:Duration)->Result<()> {
        self.deadline=Instant::now().checked_add(checked_timeout(timeout)?).ok_or_else(||failure(ErrorCode::BudgetExceeded,"control deadline overflow"))?;Ok(())
    }
    fn remaining(&self)->io::Result<Duration>{self.deadline.checked_duration_since(Instant::now()).filter(|value|!value.is_zero())
        .ok_or_else(||io::Error::new(io::ErrorKind::TimedOut,"control call deadline"))}
}
impl Read for ControlDeadlineStream{
    fn read(&mut self,out:&mut[u8])->io::Result<usize>{self.stream.set_read_timeout(Some(self.remaining()?))?;self.stream.read(out)}
}
impl Write for ControlDeadlineStream{
    fn write(&mut self,data:&[u8])->io::Result<usize>{self.stream.set_write_timeout(Some(self.remaining()?))?;self.stream.write(data)}
    fn flush(&mut self)->io::Result<()>{self.remaining()?;self.stream.flush()}
}
impl ControlRpcClient<ControlDeadlineStream>{
    pub fn connect(endpoint:SocketAddr,token:Vec<u8>,nonce:Vec<u8>,timeout:Duration)->Result<Self>{
        checked_timeout(timeout)?;
        if !(32..=256).contains(&token.len())||!(16..=64).contains(&nonce.len()){return Err(failure(ErrorCode::InvalidRequest,"invalid control credentials"));}
        if !endpoint.ip().is_loopback()||endpoint.port()==0{return Err(failure(ErrorCode::CapabilityDenied,"control endpoint must be numeric loopback with a nonzero port"));}
        let started=Instant::now();
        let stream=TcpStream::connect_timeout(&endpoint,timeout).map_err(io_failure)?;stream.set_nodelay(true).map_err(io_failure)?;
        let remaining=remaining_timeout(timeout,started.elapsed())?;
        Self::negotiate(ControlDeadlineStream::new(stream,remaining)?,token,nonce)
    }
    pub fn reset_deadline(&mut self,timeout:Duration)->Result<()> {self.stream.reset(timeout)}
}

#[cfg(test)]
mod tests{
    use super::*;
    use std::io::Cursor;
    struct Script{input:Cursor<Vec<u8>>,output:Vec<u8>}
    impl Read for Script{fn read(&mut self,out:&mut[u8])->io::Result<usize>{self.input.read(out)}}
    impl Write for Script{fn write(&mut self,data:&[u8])->io::Result<usize>{self.output.extend_from_slice(data);Ok(data.len())}fn flush(&mut self)->io::Result<()>{Ok(())}}
    fn frame(payload:&[u8])->Vec<u8>{let mut out=header(-1,payload.len() as i32).to_vec();out.extend_from_slice(payload);out}
    fn reply(code:u64,known:Option<bool>,applied:Option<bool>)->Vec<u8>{
        let mut out=Vec::new();number(&mut out,1,1);number(&mut out,2,code);bytes(&mut out,3,&[b'n';16]);number(&mut out,4,1);number(&mut out,5,7);
        number(&mut out,6,9);bytes(&mut out,7,b"df");bytes(&mut out,8,b"dfhack");
        if let Some(known)=known{number(&mut out,10,u64::from(known));}if let Some(applied)=applied{number(&mut out,11,u64::from(applied));}
        if known==Some(true){number(&mut out,12,1);number(&mut out,13,42);if applied==Some(true){bytes(&mut out,14,&[7u8;32]);}}
        out
    }
    #[test]
    fn strict_decoder_accepts_ambiguous_known_record_without_terminal_receipt()->Result<()> {
        let (_,effect)=decode(&reply(5,Some(true),Some(false)),&[b'n';16])?;assert!(effect.known);assert!(!effect.applied);assert_eq!(effect.observed_tick,42);assert!(effect.receipt_digest.is_empty());Ok(())
    }
    #[test]
    fn strict_decoder_rejects_noncanonical_boolean_and_applied_without_known(){
        let mut bad=reply(0,Some(false),Some(true));assert!(decode(&bad,&[b'n';16]).is_err());
        bad=reply(0,Some(false),None);number(&mut bad,12,2);assert!(decode(&bad,&[b'n';16]).is_err());
    }
    #[test]
    fn parser_rejects_duplicate_fields_and_overlong_varints(){
        assert!(Message::parse(&[8,1,8,1],14).is_err());assert!(Message::parse(&[8,128,0],14).is_err());
    }
    #[test]
    fn fixed_method_bootstrap_uses_four_distinct_bindings()->Result<()> {
        let mut input=b"DFHack!\n".to_vec();input.extend_from_slice(&1i32.to_le_bytes());
        for id in [2,3,4,5]{let mut binding=Vec::new();number(&mut binding,1,id);input.extend(frame(&binding));}
        input.extend(frame(&reply(0,None,None)));
        let client=ControlRpcClient::negotiate(Script{input:Cursor::new(input),output:Vec::new()},vec![b't';32],vec![b'n';16])?;
        assert_eq!(client.bridge_generation(),9);assert!(!client.poisoned());Ok(())
    }
    #[test]
    fn endpoint_and_deadline_are_rejected_before_connect(){
        let endpoint=SocketAddr::from(([192,0,2,1],5000));
        assert!(ControlRpcClient::connect(endpoint,vec![b't';32],vec![b'n';16],Duration::from_millis(1)).is_err());
        assert!(checked_timeout(Duration::ZERO).is_err());assert!(checked_timeout(Duration::from_secs(61)).is_err());
    }
}

#[cfg(test)]
#[path = "live_control_outcome_wire_tests.rs"]
mod outcome_wire_tests;
