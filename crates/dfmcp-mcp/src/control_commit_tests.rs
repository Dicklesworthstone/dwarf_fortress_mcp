//! Real MCP handlers, private files and the actual TCP/protobuf client. The local
//! peer is an explicit protocol fixture, not DFHack or native qualification.
use super::*;
use std::fs;
use std::io::{self,Read,Write};
use std::net::{TcpListener,TcpStream};
use std::os::unix::fs::DirBuilderExt;
use std::thread::JoinHandle;

static FILE_ID:AtomicUsize=AtomicUsize::new(0);
fn io_error(_:io::Error)->DfmcpError{err(ErrorCode::AdapterUnavailable,"control fixture I/O")}
fn context()->OperationContext {
    OperationContext {session_id:SessionId::new(812),request_id:RequestId::new(1),anchor:coordinator_anchor(),
        budget:WorkBudget::default(),cancellation_requested:false,
        grants:[Capability::ControlClock,Capability::Query].into_iter().map(|capability|CapabilityGrant {
            capability,scope:CapabilityScope::default(),max_risk:RiskTier::Reversible,
            expires_at_tick:None,remaining_uses:None}).collect()}
}
fn plan()->Digest32{Digest32::of_bytes(b"key")}
struct Fixture{directory:PathBuf,path:PathBuf}
impl Fixture {
    fn new()->Result<Self>{
        let directory=std::env::temp_dir().canonicalize().map_err(io_error)?.join(format!(
            "dfmcp-commit-runtime-{}-{}",std::process::id(),FILE_ID.fetch_add(1,Ordering::Relaxed)));
        fs::DirBuilder::new().mode(0o700).create(&directory).map_err(io_error)?;
        let f=Self {path:directory.join("effects.bin"),directory};
        let mut journal=open_private_control_journal(&f.path,&context(),7,EffectTailRecovery::Refuse)?;
        journal.record_prepared("key".into(),plan(),true,10,7,[1;16],&context())?;
        Ok(f)
    }
    fn session(&self,connection:Option<ControlConnection>)->Result<ControlSession>{
        let c=context();let id=next_id()?;let slot=Slot::reserve()?;
        let journal=open_private_control_journal(&self.path,&c,7,EffectTailRecovery::Refuse)?;
        Ok(ControlSession {id,connection,journal,request:0,budget:c.budget,grants:c.grants,_slot:slot})
    }
}
impl Drop for Fixture{fn drop(&mut self){let _=fs::remove_file(&self.path);let _=fs::remove_dir(&self.directory);}}
struct Registered(SessionId);
impl Registered{fn raw(&self)->Option<String>{Some(self.0.to_string())}}
impl Drop for Registered{fn drop(&mut self){if let Ok(mut sessions)=SESSIONS.lock(){sessions.remove(&self.0);}}}
fn register(session:ControlSession)->Result<Registered>{let id=session.id;publish_session(session)?;Ok(Registered(id))}
fn parse(raw:String)->Result<Value>{serde_json::from_str(&raw).map_err(|_|err(ErrorCode::InvalidRequest,"fixture JSON"))}
fn commit(s:&Registered)->Result<Value>{parse(fortress_commit(s.raw(),"key".into(),plan().to_string(),"01".repeat(16)))}
fn close(s:&Registered)->Result<Value>{parse(fortress_cancel(s.raw(),None,None,None,None,Some("session".into())))}

#[test]
fn commit_budget_refusal_precedes_connection_use_and_durable_intent()->Result<()> {
    let _serial=lock(&SESSION_TESTS)?;let f=Fixture::new()?;let s=register(f.session(None)?)?;
    let bytes=fs::read(&f.path).map_err(io_error)?;
    for limit in [0,1,64,200] {
        {let h=resolve(s.raw())?;let mut owned=lock(&h)?;
            owned.as_mut().ok_or_else(||err(ErrorCode::SessionNotFound,"fixture"))?.budget.max_bytes=limit;}
        let expected=if limit==0 {"invalid_request"}else{"budget_exceeded"};
        assert_eq!(commit(&s)?["result"]["error"]["code"],expected);
        assert_eq!(fs::read(&f.path).map_err(io_error)?,bytes);
    }
    {let h=resolve(s.raw())?;let mut owned=lock(&h)?;
        let session=owned.as_mut().ok_or_else(||err(ErrorCode::SessionNotFound,"fixture"))?;
        session.budget=context().budget;session.budget.max_actions=0;}
    assert_eq!(commit(&s)?["result"]["error"]["code"],"invalid_request");
    assert_eq!(fs::read(&f.path).map_err(io_error)?,bytes);assert_eq!(close(&s)?["result"]["ok"],true);Ok(())
}

#[test]
fn failed_opening_publication_does_not_trap_capacity_or_the_journal_lock()->Result<()> {
    let _serial=lock(&SESSION_TESTS)?;let f=Fixture::new()?;let mut session=f.session(None)?;
    let id=session.id;session.budget.max_bytes=1;
    assert!(matches!(publish_session(session),Err(e)if e.code==ErrorCode::BudgetExceeded));
    assert!(!lock(&SESSIONS)?.contains_key(&id));assert_eq!(SLOTS.load(Ordering::Acquire),0);
    drop(open_private_control_recovery(&f.path,&context())?);
    let s=register(f.session(None)?)?;assert_eq!(close(&s)?["result"]["ok"],true);Ok(())
}

#[test]
fn already_resolved_stale_handles_cannot_enter_an_operation_after_close()->Result<()> {
    let _serial=lock(&SESSION_TESTS)?;let f=Fixture::new()?;let s=register(f.session(None)?)?;
    let stale=resolve(s.raw())?;assert_eq!(close(&s)?["result"]["ok"],true);
    let mut entered=false;
    let result=parse(with_handle(&stale,"fortress.commit",|_,_|{entered=true;Ok(json!({"ok":true}))}))?;
    assert!(!entered);assert_eq!(result["result"]["error"]["code"],"session_not_found");
    let next=register(f.session(None)?)?;assert_eq!(close(&next)?["result"]["ok"],true);Ok(())
}

fn varint(out:&mut Vec<u8>,mut n:u64){while n>=128{out.push((n as u8&127)|128);n>>=7;}out.push(n as u8);}
fn number(out:&mut Vec<u8>,field:u64,n:u64){varint(out,field<<3);varint(out,n);}
fn bytes(out:&mut Vec<u8>,field:u64,b:&[u8]){varint(out,(field<<3)|2);varint(out,b.len() as u64);out.extend_from_slice(b);}
fn reply(stream:&mut TcpStream,body:&[u8])->io::Result<()> {
    let mut h=[0u8;8];h[..2].copy_from_slice(&(-1i16).to_le_bytes());h[4..].copy_from_slice(&(body.len() as i32).to_le_bytes());
    stream.write_all(&h)?;stream.write_all(body)?;stream.flush()
}
fn request(stream:&mut TcpStream)->io::Result<Option<(i16,Vec<u8>)>> {
    let mut h=[0u8;8];
    if stream.read(&mut h[..1])?==0{return Ok(None);}
    stream.read_exact(&mut h[1..])?;
    let length=i32::from_le_bytes(h[4..].try_into().map_err(|_|io::Error::other("frame length"))?);
    if !(0..=8192).contains(&length){return Err(io::Error::other("oversized fixture request"));}
    let mut body=vec![0;length as usize];stream.read_exact(&mut body)?;
    Ok(Some((i16::from_le_bytes([h[0],h[1]]),body)))
}
fn common()->Vec<u8>{
    let mut out=Vec::new();number(&mut out,1,1);number(&mut out,2,0);bytes(&mut out,3,&[4;16]);
    number(&mut out,4,1);number(&mut out,5,7);number(&mut out,6,7);
    bytes(&mut out,7,b"df");bytes(&mut out,8,b"dfhack");out
}
fn terminal()->Vec<u8>{
    let mut out=common();number(&mut out,10,1);number(&mut out,11,1);number(&mut out,12,1);number(&mut out,13,11);
    let mut seal=b"dfmcp-control-receipt-v2\0".to_vec();seal.extend_from_slice(&7u64.to_be_bytes());
    seal.extend_from_slice(b"key\0");seal.extend_from_slice(plan().as_bytes());seal.push(1);seal.extend_from_slice(&11u64.to_be_bytes());
    bytes(&mut out,14,Digest32::of_bytes(&seal).as_bytes());out
}
struct Bridge {endpoint:SocketAddr,worker:Option<JoinHandle<io::Result<usize>>>}
impl Bridge {
    fn new(lose_reply:bool)->Result<Self>{
        let listener=TcpListener::bind("127.0.0.1:0").map_err(io_error)?;
        let endpoint=listener.local_addr().map_err(io_error)?;listener.set_nonblocking(true).map_err(io_error)?;
        let worker=std::thread::spawn(move||->io::Result<usize>{
            let until=Instant::now()+Duration::from_secs(5);
            let mut stream=loop {match listener.accept(){Ok((stream,_))=>break stream,
                Err(e)if e.kind()==io::ErrorKind::WouldBlock&&Instant::now()<until=>std::thread::sleep(Duration::from_millis(1)),
                Err(e)=>return Err(e)}};
            stream.set_read_timeout(Some(Duration::from_secs(3)))?;stream.set_write_timeout(Some(Duration::from_secs(3)))?;
            let mut hello=[0;12];stream.read_exact(&mut hello)?;
            let mut expected=b"DFHack?\n".to_vec();expected.extend_from_slice(&1i32.to_le_bytes());
            if hello.as_slice()!=expected.as_slice(){return Err(io::Error::other("native hello mismatch"));}
            let mut welcome=b"DFHack!\n".to_vec();welcome.extend_from_slice(&1i32.to_le_bytes());stream.write_all(&welcome)?;
            for id in 2..=5 {
                if !matches!(request(&mut stream)?,Some((0,_))){return Err(io::Error::other("expected method bind"));}
                let mut body=Vec::new();number(&mut body,1,id);reply(&mut stream,&body)?;
            }
            if !matches!(request(&mut stream)?,Some((2,_))){return Err(io::Error::other("expected plugin handshake"));}
            reply(&mut stream,&common())?;
            let mut commits=0;
            while let Some((method,body))=request(&mut stream)? {
                if method!=4||commits!=0{return Err(io::Error::other("mutation retried or unexpected bridge method"));}
                let mut expected=Vec::new();bytes(&mut expected,1,&[b't';32]);bytes(&mut expected,2,&[4;16]);
                number(&mut expected,3,1);number(&mut expected,4,7);bytes(&mut expected,5,b"key");
                bytes(&mut expected,6,plan().as_bytes());bytes(&mut expected,7,&[1;16]);
                if body!=expected{return Err(io::Error::other("commit identity changed on wire"));}
                commits+=1;if lose_reply {return Ok(commits);}
                reply(&mut stream,&terminal())?;
            }
            Ok(commits)
        });
        Ok(Self{endpoint,worker:Some(worker)})
    }
    fn connect(&self)->Result<ControlConnection>{
        let timeout=Duration::from_secs(2);let token=vec![b't';32];let nonce=vec![4;16];
        let client=ControlRpcClient::connect(self.endpoint,token.clone(),nonce.clone(),timeout)?;
        Ok(ControlConnection {client,endpoint:self.endpoint,token,nonce,timeout})
    }
    fn finish(mut self)->Result<usize>{
        self.worker.take().ok_or_else(||err(ErrorCode::InternalInvariantViolation,"worker absent"))?
            .join().map_err(|_|err(ErrorCode::InternalInvariantViolation,"protocol fixture panic"))?.map_err(io_error)
    }
}
impl Drop for Bridge {fn drop(&mut self){if let Some(worker)=self.worker.take(){let _=worker.join();}}}

#[test]
fn real_wire_commit_close_and_reopen_preserve_terminal_or_unresolved_evidence()->Result<()> {
    let _serial=lock(&SESSION_TESTS)?;
    for lose_reply in [false,true] {
        let f=Fixture::new()?;let bridge=Bridge::new(lose_reply)?;let s=register(f.session(Some(bridge.connect()?))?)?;
        let first=commit(&s)?;
        if lose_reply {assert_eq!(first["result"]["error"]["code"],"effect_indeterminate");}
        else {assert_eq!(first["result"]["ok"],true);assert_eq!(first["result"]["mutation_dispatched"],true);}
        let retry=commit(&s)?;
        if lose_reply {assert_eq!(retry["result"]["error"]["code"],"effect_indeterminate");}
        else {assert_eq!(retry["result"]["replayed_terminal"],true);assert_eq!(retry["result"]["mutation_dispatched"],false);}
        let bytes=fs::read(&f.path).map_err(io_error)?;assert_eq!(close(&s)?["result"]["ok"],true);
        assert_eq!(bridge.finish()?,1);
        let recovered=open_private_control_recovery(&f.path,&context())?;
        let expected=if lose_reply {DurablePauseState::Indeterminate}else{DurablePauseState::VerifiedApplied};
        assert_eq!(recovered.lookup("key").map(|r|r.state),Some(expected));drop(recovered);
        assert_eq!(fs::read(&f.path).map_err(io_error)?,bytes);
        let next=register(f.session(None)?)?;
        let replay=commit(&next)?;
        if lose_reply {assert_eq!(replay["result"]["error"]["code"],"effect_indeterminate");}
        else {assert_eq!(replay["result"]["replayed_terminal"],true);}
        assert_eq!(close(&next)?["result"]["ok"],true);
    }
    Ok(())
}

#[test]
fn close_of_a_live_connection_sends_no_native_cancellation_or_mutation()->Result<()> {
    let _serial=lock(&SESSION_TESTS)?;let f=Fixture::new()?;let bridge=Bridge::new(false)?;
    let s=register(f.session(Some(bridge.connect()?))?)?;
    let before=fs::read(&f.path).map_err(io_error)?;assert_eq!(close(&s)?["result"]["ok"],true);
    assert_eq!(bridge.finish()?,0);assert_eq!(fs::read(&f.path).map_err(io_error)?,before);Ok(())
}
