//! Owned blocking execution and a second, query-only native boundary.
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::time::Instant;
use dfmcp_adapter::dig_designation::{DigEffect, DigObservation, DigPlan, DigRegion};
use dfmcp_adapter::dig_designation::rpc::{DigManifest, DigPreparation, DigRpcClient, DigSource, DigTcpStream};
use dfmcp_adapter::dig_designation::journal::{DigBinding, DigGuard, DigMode, DigStage};
use dfmcp_adapter::dig_designation::journal::session::DigSessionGuard;
use dfmcp_core::{ErrorCode, MapCoord, MapCuboid, OperationContext, Result};
use fastmcp_rust::asupersync::Cx;
use super::{error,unbound};

const ENVIRONMENT:[&str;8]=["DFMCP_ALLOW_UNADMITTED_DIG_RECOVERY_V1_16","DFMCP_DIG_WORLD_FOLDER",
    "DFMCP_DIG_SITE_ID","DFMCP_DIG_SCOPE","DFMCP_DIG_JOURNAL","DFMCP_DIG_RECOVERY_ONLINE","DFMCP_DIG_TOKEN",super::goals::ENVIRONMENT];
fn denied()->dfmcp_core::DfmcpError {error(ErrorCode::CapabilityDenied,"mining recovery boundary refused")}
#[derive(Clone,Debug,PartialEq,Eq)]
pub(super) struct Config {
    pub path:PathBuf, pub scope:MapCuboid, folder:String, site:u32, online:bool,
    pub goal_files:super::goals::Files,
}
impl Config {
    pub fn mode(&self)->DigMode {if self.online{DigMode::Recover}else{DigMode::Offline}}
    pub fn fortress(&self)->dfmcp_core::FortressId {dfmcp_adapter::workforce_control::fortress_id(&self.folder,self.site)}
    pub fn matches(&self,b:&DigBinding)->Result<()> {
        if b.folder()!=self.folder || b.site()!=self.site || b.scope()!=self.scope{return Err(denied());}Ok(())
    }
}
fn value(name:&str,limit:usize)->Result<String> {
    let raw=std::env::var(name).map_err(|_|denied())?;
    if raw.is_empty()||raw.len()>limit||raw.contains('\0'){return Err(denied());}Ok(raw)
}
fn scope(raw:&str)->Result<MapCuboid> {
    if raw.len()>128{return Err(denied());}
    let v:[i32;6]=serde_json::from_str(raw).map_err(|_|denied())?;
    if v.iter().any(|n|!(0..=32767).contains(n)){return Err(denied());}
    MapCuboid::new(MapCoord::new(v[0],v[1],v[2]),MapCoord::new(v[3],v[4],v[5]))
}
fn environment_contract(opt:&str,online:Option<&str>,names:impl IntoIterator<Item=String>,admitted:bool)->Result<bool> {
    if opt!="1"||admitted{return Err(denied());}
    let mut count=0;
    for name in names {
        count+=1;if count>1024||name.len()>512{return Err(denied());}
        if name.starts_with("DFMCP_")&&!ENVIRONMENT.contains(&name.as_str()){return Err(denied());}
    }
    match online {None=>Ok(false),Some("1")=>Ok(true),_=>Err(denied())}
}
fn configured(folder:String,site:String,area:String,path:String,online:bool)->Result<Config> {
    for (value,limit) in [(&folder,512),(&site,10),(&area,128),(&path,4096)] {
        if value.is_empty()||value.len()>limit||value.contains('\0'){return Err(denied());}
    }
    let number:u32=site.parse().map_err(|_|denied())?;
    if number>i32::MAX as u32||number.to_string()!=site{return Err(denied());}
    if !path.starts_with('/')||path[1..].split('/').any(|p|p.is_empty()||p=="."||p=="..") {return Err(denied());}
    Ok(Config{folder,site:number,online,path:PathBuf::from(path),scope:scope(&area)?,goal_files:super::goals::Files::default()})
}
pub(super) fn configuration()->Result<Config> {
    let opt=value("DFMCP_ALLOW_UNADMITTED_DIG_RECOVERY_V1_16",1)?;
    let online=match std::env::var("DFMCP_DIG_RECOVERY_ONLINE") {
        Err(std::env::VarError::NotPresent)=>None,Ok(v)=>Some(v),_=>return Err(denied()),
    };
    let online=environment_contract(&opt,online.as_deref(),
        std::env::vars_os().map(|(k,_)|k.to_string_lossy().into_owned()),
        crate::admission::current_admission_provenance().is_some())?;
    let mut config=configured(value("DFMCP_DIG_WORLD_FOLDER",512)?,value("DFMCP_DIG_SITE_ID",10)?,
        value("DFMCP_DIG_SCOPE",128)?,value("DFMCP_DIG_JOURNAL",4096)?,online)?;
    config.goal_files=super::goals::Files::environment()?;
    Ok(config)
}

pub(super) struct RequestControl {pub started:Instant,parent:Cx,worker:Cx,abandoned:Arc<AtomicBool>}
impl RequestControl {
    pub(super) fn checkpoint(&self)->Result<()> {
        if self.abandoned.load(Ordering::Acquire){return Err(error(ErrorCode::CancellationRequested,"request abandoned"));}
        self.parent.checkpoint().map_err(|_|error(ErrorCode::CancellationRequested,"parent request cancelled"))?;
        self.worker.checkpoint().map_err(|_|error(ErrorCode::CancellationRequested,"blocking request cancelled"))?;
        Ok(())
    }
    fn check(&self)->Result<()> {
        self.checkpoint()?;
        if self.parent.io().is_none()||self.worker.io().is_none(){return Err(denied());}Ok(())
    }
}
struct CancelOnDrop(Arc<AtomicBool>);
impl Drop for CancelOnDrop {fn drop(&mut self){self.0.store(true,Ordering::Release);}}

pub(super) async fn owned<F>(op:&'static str,operation:F)->String
where F:FnOnce(RequestControl)->String+Send+'static {
    let Some(cx)=Cx::current() else{return unbound(op,&denied());};
    if cx.checkpoint().is_err(){return unbound(op,&error(ErrorCode::CancellationRequested,"request cancelled"));}
    let started=Instant::now();let flag=Arc::new(AtomicBool::new(false));
    let _cancel=CancelOnDrop(flag.clone());let parent=cx.clone();
    // Cx::spawn_blocking registers work with the existing runtime/region. Do not
    // use runtime::spawn_blocking, its fallback threads, or a detached std thread.
    let task=cx.spawn_blocking(move|worker|{
        let _ambient=Cx::set_current(Some(worker.clone()));
        let control=RequestControl{started,parent,worker,abandoned:flag};
        if let Err(e)=control.checkpoint(){return unbound(op,&e);}
        operation(control)
    });
    let mut task=match task{Ok(task)=>task,Err(_)=>return unbound(op,&denied())};
    match task.join(&cx).await {
        Ok(value)=>value,
        Err(_)=>unbound(op,&error(ErrorCode::CancellationIncomplete,"owned recovery work did not yield an acknowledged result")),
    }
}
pub(super) fn boundary(control:&RequestControl,expected:&Config)->Result<()> {
    control.check()?;
    if &configuration()?!=expected{return Err(denied());}Ok(())
}

/// Defense in depth: even an erroneous upper-layer call cannot reach native
/// observation, prepare, commit or cancellation through this recovery source.
pub(super) struct QueryOnly<N>(pub N);
impl<N:DigSource> DigSource for QueryOnly<N> {
    fn manifest(&self)->&DigManifest{self.0.manifest()}
    fn endpoint(&self)->Option<SocketAddr>{self.0.endpoint()}
    fn observe(&mut self,_:DigRegion,_:&OperationContext)->Result<DigObservation>{Err(denied())}
    fn prepare(&mut self,_:&DigPlan,_:&OperationContext)->Result<DigPreparation>{Err(denied())}
    fn commit(&mut self,_:&DigPlan,_:&OperationContext)->Result<DigEffect>{Err(denied())}
    fn cancel(&mut self,_:&DigPlan,_:&OperationContext)->Result<DigEffect>{Err(denied())}
    fn query(&mut self,p:&DigPlan,c:&OperationContext)->Result<Option<DigEffect>>{self.0.query(p,c)}
}
pub(super) struct Guard<'a> {control:&'a RequestControl,config:&'a Config,binding:&'a DigBinding}
impl<'a> Guard<'a> {
    pub fn new(control:&'a RequestControl,config:&'a Config,binding:&'a DigBinding)->Self{Self{control,config,binding}}
}
impl DigGuard for Guard<'_> {
    fn check(&mut self,stage:DigStage,_:&DigPlan,_:&OperationContext)->Result<()> {
        if stage!=DigStage::Query{return Err(denied());}
        boundary(self.control,self.config)?;self.config.matches(self.binding)
    }
}
impl DigSessionGuard for Guard<'_> {
    fn connect(&mut self,b:&DigBinding,_:DigRegion,_:&OperationContext)->Result<()> {
        boundary(self.control,self.config)?;
        if !self.config.online||b!=self.binding{return Err(denied());}self.config.matches(b)
    }
    fn observe(&mut self,_:&DigBinding,_:DigRegion,_:&OperationContext)->Result<()> {Err(denied())}
}
pub(super) fn connect(control:&RequestControl,config:&Config,b:&DigBinding,r:DigRegion,c:&OperationContext)
    ->Result<QueryOnly<DigRpcClient<DigTcpStream>>>
{
    boundary(control,config)?;config.matches(b)?;
    if !config.online{return Err(denied());}
    // Credentials are read only here, never for bootstrap or verified local history.
    let token=value("DFMCP_DIG_TOKEN",256)?.into_bytes();
    if token.len()<32{return Err(denied());}
    let mut nonce=c.session_id.get().to_be_bytes().to_vec();nonce.extend_from_slice(&c.request_id.get().to_be_bytes());
    Ok(QueryOnly(DigRpcClient::connect(b.endpoint(),token,nonce,r,c)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_scope_and_noncanonical_types_are_checked() {
        assert!(scope("[0,0,0,63,63,7]").is_ok());
        for raw in ["[0,0,0,63,63]","[true,0,0,1,1,1]","[-1,0,0,1,1,1]","[2,0,0,1,1,1]",
            "[0,0,0,32768,1,1]","[0,0,0,1.0,1,1]","{}"] {assert!(scope(raw).is_err());}
    }
    #[test]
    fn environment_isolation_cannot_enable_mutation_or_production() {
        let names=ENVIRONMENT.iter().map(|s|(*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(environment_contract("1",None,names.clone(),false).ok(),Some(false));
        assert_eq!(environment_contract("1",Some("1"),names.clone(),false).ok(),Some(true));
        for opt in ["","true","0"] {assert!(environment_contract(opt,None,names.clone(),false).is_err());}
        for online in ["0","true",""] {assert!(environment_contract("1",Some(online),names.clone(),false).is_err());}
        assert!(environment_contract("1",None,names.clone(),true).is_err());
        for extra in ["DFMCP_ADMITTED_BRIDGE_PROTOCOL","DFMCP_DIG_ALLOW_DESIGNATE","DFMCP_ALLOW_UNADMITTED_WORKFORCE_V1_17"] {
            let mut changed=names.clone();changed.push(extra.into());
            assert!(environment_contract("1",None,changed,false).is_err());
        }
    }
    #[test]
    fn operator_source_and_path_must_be_bounded_exact_and_normalized() {
        let make=|folder:&str,site:&str,path:&str|configured(folder.into(),site.into(),"[0,0,0,63,63,7]".into(),path.into(),false);
        assert!(make("region1","1","/private/mining/journal").is_ok());
        for site in ["01","-1","+1","2147483648",""] {assert!(make("region1",site,"/private/journal").is_err());}
        for path in ["relative","/private/../journal","/private/./journal","/private//journal","/"] {
            assert!(make("region1","1",path).is_err());
        }
        assert!(make("","1","/private/journal").is_err());
        assert!(make(&"x".repeat(513),"1","/private/journal").is_err());
    }
    #[test]
    fn abandoned_request_marks_its_worker_cancelled() {
        let flag=Arc::new(AtomicBool::new(false));
        {let _guard=CancelOnDrop(flag.clone());assert!(!flag.load(Ordering::Acquire));}
        assert!(flag.load(Ordering::Acquire));
    }
    #[test]
    fn owned_entry_executes_on_a_blocking_worker_and_restores_ambient_context()->std::result::Result<(),Box<dyn std::error::Error>> {
        let caller=std::thread::current().id();
        let value=crate::run_with_runtime_cx(|_|async move{
            owned("fortress.doctor",move|c|{
                assert_ne!(std::thread::current().id(),caller);assert!(c.check().is_ok());
                assert!(Cx::current().is_some());"owned".to_owned()
            }).await
        })?;
        assert_eq!(value,"owned");assert!(Cx::current().is_none());Ok(())
    }
    #[test]
    fn inherited_runtime_restrictions_cannot_fall_back_to_threads()->std::result::Result<(),Box<dyn std::error::Error>> {
        let called=Arc::new(AtomicBool::new(false));let worker_flag=called.clone();
        let value=crate::run_with_runtime_cx(|cx|async move{
            let _restricted=cx.restrict::<fastmcp_rust::asupersync::cx::cap::None>().set_current_restricted();
            owned("fortress.doctor",move|_|{worker_flag.store(true,Ordering::Release);String::new()}).await
        })?;
        assert!(!called.load(Ordering::Acquire));
        let packet:serde_json::Value=serde_json::from_str(&value)?;
        assert_eq!(packet["result"]["ok"],false);assert!(packet["agent_turn"].is_object());Ok(())
    }
}
