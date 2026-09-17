//! Private-file and injected-source harness for actual spatial MCP handlers.
use super::*;
use std::collections::{BTreeSet, VecDeque};
use std::fs;
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use dfmcp_adapter::operations_journal::TailRecovery;

static SERIAL:Mutex<()>=Mutex::new(());
static FILE_ID:AtomicUsize=AtomicUsize::new(0);
fn io_error(_:std::io::Error)->DfmcpError {error(ErrorCode::CorruptLedger,"multisite fixture I/O")}
struct Files {directory:PathBuf,observations:PathBuf,watches:PathBuf}
impl Files {
    fn new()->Result<Self> {
        let directory=std::env::temp_dir().canonicalize().map_err(io_error)?.join(format!(
            "dfmcp-production-sites-{}-{}",std::process::id(),FILE_ID.fetch_add(1,Ordering::Relaxed)));
        fs::DirBuilder::new().mode(0o700).create(&directory).map_err(io_error)?;
        Ok(Self {observations:directory.join("observations.bin"),watches:directory.join("watches.bin"),directory})
    }
}
impl Drop for Files {fn drop(&mut self) {
    let _=fs::remove_file(&self.watches);let _=fs::remove_file(&self.observations);let _=fs::remove_dir(&self.directory);
}}
struct Script {values:VecDeque<LiveSpatialCitizenObservation>,calls:Arc<AtomicUsize>,fenced:bool}
impl Source for Script {
    fn read(&mut self,_:Duration)->Result<LiveSpatialCitizenObservation> {
        self.calls.fetch_add(1,Ordering::SeqCst);
        self.values.pop_front().ok_or_else(||error(ErrorCode::AdapterUnavailable,"site fixture exhausted"))
    }
    fn poisoned(&self)->bool {self.fenced}
    fn fence(&mut self) {self.fenced=true;}
    fn pages(&self)->u32 {1}
}
struct Registered {id:SessionId,calls:Arc<AtomicUsize>}
impl Registered {fn handle(&self)->Option<String> {Some(self.id.to_string())}}
impl Drop for Registered {fn drop(&mut self) {if let Ok(mut sessions)=SESSIONS.lock(){sessions.remove(&self.id);}}}
fn decode(raw:&str)->Result<Value> {serde_json::from_str(raw).map_err(|_|error(ErrorCode::InvalidRequest,"site fixture JSON"))}
fn ok(value:Value)->Value {assert_eq!(value["ok"],true,"{value}");value}
fn ask(s:&Registered,query:Value)->Result<Value> {
    decode(&fortress_query(s.handle(),None,Some(json!({"schema":"dfmcp.query/1","query":query}))))
}
fn task(key:&str,priority:u32,workers:u32,units:u32)->Value {
    json!({"key":key,"priority":priority,"workers":workers,"skill_key":"CARPENTRY",
        "materials":[{"key":"wood","units":units,"item_types":["item_type_3"]}]})
}
fn historical(entry:&Value,query:Value)->Value {
    json!({"kind":"historical_query","record":entry["record"],"record_digest":entry["record_digest"],"query":query})
}
fn retain_watch(s:&Registered)->Result<()> {
    ok(ask(s,json!({"kind":"watch","key":"site-progress","condition":{"op":"paused","value":true},
        "deadline_tick":105u64*403200+100,"stable_observations":64}))?);Ok(())
}

#[path="spatial_portfolio_sites_tests.rs"]
mod tests;
