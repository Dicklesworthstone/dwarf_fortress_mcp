use super::*;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::rc::Rc;
use dfmcp_adapter::work_order_control::{CreationRecord, WorkOrderJournal, WorkOrderQuerySource};
use dfmcp_adapter::work_orders::{WorkOrderEffect, WorkOrderObservation, WorkOrderPlan,
    WorkOrderRecipe, WorkOrderState, MAX_NATIVE_TICK};
use dfmcp_adapter::work_orders::rpc::WorkOrderManifest;

#[derive(Clone)]
pub(super) struct Memory {
    bytes: Rc<RefCell<Cursor<Vec<u8>>>>,
    valid: Rc<Cell<bool>>,
}
impl Memory {
    fn new(bytes: Vec<u8>) -> Self {
        Self { bytes:Rc::new(RefCell::new(Cursor::new(bytes))),valid:Rc::new(Cell::new(true)) }
    }
}
impl Read for Memory {
    fn read(&mut self, out:&mut [u8]) -> io::Result<usize> { self.bytes.borrow_mut().read(out) }
}
impl Seek for Memory {
    fn seek(&mut self, from:SeekFrom) -> io::Result<u64> { self.bytes.borrow_mut().seek(from) }
}
impl Write for Memory {
    fn write(&mut self, data:&[u8]) -> io::Result<usize> { self.bytes.borrow_mut().write(data) }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> { self.validate_identity() }
    fn truncate(&mut self, _:u64) -> io::Result<()> { Err(io::Error::other("test forbids repair")) }
    fn validate_identity(&self) -> io::Result<()> {
        if self.valid.get() { Ok(()) } else { Err(io::Error::other("lost custody")) }
    }
}

fn invalid(text:&str) -> dfmcp_core::DfmcpError { error(ErrorCode::InvalidRequest,text) }
fn decode_hex(raw:&str) -> Result<Vec<u8>> {
    let raw = raw.trim();
    if raw.len() % 2 != 0 { return Err(invalid("odd fixture")); }
    raw.as_bytes().chunks_exact(2).map(|pair| {
        let a = char::from(pair[0]).to_digit(16).ok_or_else(||invalid("bad fixture"))?;
        let b = char::from(pair[1]).to_digit(16).ok_or_else(||invalid("bad fixture"))?;
        Ok((a*16+b) as u8)
    }).collect()
}
fn observation() -> Result<WorkOrderObservation> {
    WorkOrderObservation::decode(&decode_hex(include_str!("../../dfmcp-adapter/tests/fixtures/work_order_observation_v1_10.hex"))?)
}
fn after(plan:&WorkOrderPlan) -> Result<WorkOrderObservation> {
    let before = plan.observation();
    let mut bytes = before.canonical_bytes().to_vec();
    bytes[16..24].copy_from_slice(&(before.sequence()+1).to_be_bytes());
    bytes[32..36].copy_from_slice(&(before.next_order_id()+1).to_be_bytes());
    let count_offset = 43 + before.world_folder().len();
    bytes[count_offset..count_offset+4].copy_from_slice(&((before.order_ids().len()+1) as u32).to_be_bytes());
    bytes.extend_from_slice(&before.next_order_id().to_be_bytes());
    WorkOrderObservation::decode(&bytes)
}
fn native_effect(plan:&WorkOrderPlan, state:WorkOrderState) -> Result<WorkOrderEffect> {
    let o = plan.observation();
    let mut bytes = b"DFMWOE10".to_vec();
    for n in [o.generation(),o.sequence(),o.tick()] { bytes.extend_from_slice(&n.to_be_bytes()); }
    bytes.extend_from_slice(&o.next_order_id().to_be_bytes());
    bytes.push(plan.spec().recipe() as u8); bytes.extend_from_slice(&plan.spec().amount().to_be_bytes());
    bytes.extend_from_slice(o.witness().as_bytes()); bytes.extend_from_slice(plan.digest().as_bytes());
    bytes.extend_from_slice(plan.prepare_token());
    let known = state == WorkOrderState::Created;
    bytes.extend_from_slice(&[state as u8,u8::from(known)]);
    bytes.extend_from_slice(&(if known { o.tick() } else { 0 }).to_be_bytes());
    let (after_hash,config_hash) = if known {
        let mut config = b"DFMWOC10".to_vec(); config.extend_from_slice(&o.next_order_id().to_be_bytes());
        config.push(plan.spec().recipe() as u8); config.extend_from_slice(&plan.spec().amount().to_be_bytes());
        (after(plan)?.witness(),Digest32::of_bytes(&config))
    } else { (Digest32::ZERO,Digest32::ZERO) };
    bytes.extend_from_slice(after_hash.as_bytes()); bytes.extend_from_slice(config_hash.as_bytes());
    let mut key = (plan.key().len() as u16).to_be_bytes().to_vec(); key.extend_from_slice(plan.key().as_bytes());
    let receipt = if state.terminal() {
        let mut data = b"dfmcp-work-order-receipt/1\0".to_vec(); data.extend_from_slice(&o.generation().to_be_bytes());
        data.extend_from_slice(&key); data.extend_from_slice(&bytes[73..195]); Digest32::of_bytes(&data)
    } else { Digest32::ZERO };
    bytes.extend_from_slice(receipt.as_bytes()); bytes.extend_from_slice(&key);
    WorkOrderEffect::decode(&bytes,plan)
}

struct Native {
    observation:WorkOrderObservation, reads:usize, prepares:usize, commits:usize, queries:usize,
    lost_reply:bool, missing_record:bool, fail_read:bool, retained:BTreeMap<String,WorkOrderEffect>,
}
pub(super) struct Source { manifest:WorkOrderManifest, native:Rc<RefCell<Native>>, fenced:bool }
impl Source {
    fn ready(&self) -> Result<()> {
        if self.fenced { Err(error(ErrorCode::AdapterUnavailable,"test source fenced; reopen explicitly")) } else { Ok(()) }
    }
}
impl WorkOrderQuerySource for Source {
    fn manifest(&self) -> &WorkOrderManifest { &self.manifest }
    fn fence(&mut self) { self.fenced = true; }
    fn read_orders(&mut self, _:Duration) -> Result<WorkOrderObservation> {
        self.ready()?;
        let mut n = self.native.borrow_mut(); n.reads += 1;
        if n.fail_read { return Err(error(ErrorCode::AdapterUnavailable,"injected read failure")); }
        Ok(n.observation.clone())
    }
    fn query(&mut self, p:&WorkOrderPlan, _:Duration) -> Result<Option<WorkOrderEffect>> {
        self.ready()?;
        let mut n = self.native.borrow_mut(); n.queries += 1;
        Ok(if n.missing_record { None } else { n.retained.get(p.key()).cloned() })
    }
}
impl WorkOrderSource for Source {
    fn prepare(&mut self, p:&WorkOrderPlan, _:Duration) -> Result<WorkOrderEffect> {
        self.ready()?;
        let mut n = self.native.borrow_mut(); n.prepares += 1;
        let effect = native_effect(p,WorkOrderState::Prepared)?;
        n.retained.insert(p.key().to_owned(),effect.clone()); Ok(effect)
    }
    fn commit(&mut self, p:&WorkOrderPlan, _: &WorkOrderEffect, _:Duration) -> Result<WorkOrderEffect> {
        self.ready()?;
        let mut n = self.native.borrow_mut(); n.commits += 1;
        let effect = native_effect(p,WorkOrderState::Created)?;
        n.observation = after(p)?; n.retained.insert(p.key().to_owned(),effect.clone());
        if n.lost_reply { Err(error(ErrorCode::EffectIndeterminate,"created, but reply lost")) } else { Ok(effect) }
    }
}

pub(super) struct Fixture {
    state:State<Memory,Source>, store:Memory, native:Rc<RefCell<Native>>, manifest:WorkOrderManifest,
}
impl Fixture {
    pub fn new() -> Result<Self> { Self::with_observation(observation()?) }
    fn with_observation(observation:WorkOrderObservation) -> Result<Self> {
        let manifest = WorkOrderManifest { generation:observation.generation(),df_version:"df".into(),dfhack_version:"dfhack".into() };
        Self::with_manifest(observation,manifest)
    }
    fn with_manifest(observation:WorkOrderObservation,manifest:WorkOrderManifest) -> Result<Self> {
        let native = Rc::new(RefCell::new(Native { observation,reads:0,prepares:0,commits:0,queries:0,
            lost_reply:false,missing_record:false,fail_read:false,retained:BTreeMap::new() }));
        let store = Memory::new(Vec::new());
        let state = Self::open(store.clone(),native.clone(),manifest.clone(),JournalMode::Control,11,true)?;
        Ok(Self { state,store,native,manifest })
    }
    fn open(store:Memory,native:Rc<RefCell<Native>>,manifest:WorkOrderManifest,
        mode:JournalMode,sequence:u64,initialize:bool) -> Result<State<Memory,Source>>
    {
        let fortress = native.borrow().observation.fortress_id();
        let id = SessionId::new((1u128<<127)|FAMILY|u128::from(sequence));
        let anchor = StateAnchor { fortress_id:fortress,cursor:ObservationCursor::ORIGIN,tick:GameTick(0),state_hash:Digest32::ZERO };
        let budget = WorkBudget { max_wall_millis:60_000,max_bytes:MAX_BYTES,max_entities:4096,
            max_output_tokens:262_144,max_actions:1,max_game_ticks:0 };
        let grants = grants(mode,fortress,true)?;
        let c = OperationContext { session_id:id,request_id:RequestId::new(1),anchor,budget,
            grants:grants.clone(),cancellation_requested:false };
        let journal = WorkOrderJournal::open(store,&c,mode,initialize)?;
        let source = if mode == JournalMode::Offline { None } else { Some(Source { manifest,native,fenced:false }) };
        Ok(State { id,request:1,anchor,budget,grants,mode,
            control:WorkOrderSession::new(journal,source,&c)?,cursors:Continuations::default() })
    }
    fn reopen(self,mode:JournalMode,sequence:u64) -> Result<Self> {
        let Self { state,store,native,manifest } = self;
        drop(state);
        let state = Self::open(store.clone(),native.clone(),manifest.clone(),mode,sequence,false)?;
        Ok(Self { state,store,native,manifest })
    }
    fn call(&mut self,operation:&str,action:Action,rows:usize) -> Result<Value> {
        self.call_with(operation,action,rows,Limits::default(),true,false)
    }
    fn call_with(&mut self,operation:&str,action:Action,rows:usize,limits:Limits,
        production:bool,cancelled:bool) -> Result<Value>
    {
        let context = self.state.context(production,cancelled)?;
        let out = run_action(&mut self.state,context,operation,limits,rows,Ok(action));
        assert!(out.len() as u64 <= self.state.budget.max_bytes.min(u64::from(self.state.budget.max_output_tokens)*4));
        let value:Value = serde_json::from_str(&out).map_err(|_|invalid("invalid rendered JSON"))?;
        assert_eq!(value["agent_turn"]["schema"],"dfmcp.agent_turn/1");
        assert_eq!(value["agent_turn"]["briefing"]["runtime_admitted"],false);
        assert!(value["agent_turn"]["briefing"].get("admission").is_none());
        Ok(value)
    }
    pub fn prepare(&mut self,key:&str) -> Result<CreationRecord> {
        self.prepare_spec(key,WorkOrderSpec::new(WorkOrderRecipe::WoodenBed,5)?)
    }
    fn prepare_spec(&mut self,key:&str,spec:WorkOrderSpec) -> Result<CreationRecord> {
        let observed = self.call("fortress.observe",Action::Observe,1)?;
        assert_eq!(observed["result"]["ok"],true);
        let witness = digest(observed["result"]["observation"]["witness"].as_str().ok_or_else(||invalid("missing witness"))?)?;
        let result = self.call("fortress.plan",Action::Plan { key:key.into(),spec,witness },1)?;
        assert_eq!(result["result"]["ok"],true);
        let plan = digest(result["result"]["effect"]["plan_digest"].as_str().ok_or_else(||invalid("missing plan"))?)?;
        let context = self.state.context(true,false)?;
        self.state.control.record(key,plan,&context)
    }
    fn commit_action(record:&CreationRecord) -> Action {
        Action::Commit { key:record.plan().key().into(),plan:record.plan().digest(),witness:record.plan().observation().witness() }
    }
}

#[test]
fn actual_handler_loop_seals_discovers_commits_and_replays_once() -> Result<()> {
    let mut f = Fixture::new()?; let r = f.prepare("order-001")?;
    let explained = f.call("fortress.explain",Action::Explain { key:r.plan().key().into(),plan:r.plan().digest() },1)?;
    assert_eq!(explained["result"]["effect"]["state"],"prepared");
    let page = f.call("fortress.query",Action::Query { filter:Filter::Pending,limit:2,continuation:None },2)?;
    assert_eq!(page["result"]["records"].as_array().map(Vec::len),Some(1));
    let done = f.call("fortress.commit",Fixture::commit_action(&r),1)?;
    assert_eq!(done["result"]["creation_verified"],true);
    assert_eq!(done["result"]["production_goal_completion_proven"],false);
    assert_eq!(done["result"]["effect"]["native_effect_hex"],include_str!("../../dfmcp-adapter/tests/fixtures/work_order_created_v1_10.hex").trim());
    let replay = f.call("fortress.commit",Fixture::commit_action(&r),1)?;
    assert_eq!(replay["result"]["effect"],done["result"]["effect"]);
    assert_eq!(f.native.borrow().commits,1); assert_eq!(f.native.borrow().prepares,1);
    Ok(())
}

#[test]
fn lost_reply_is_visible_as_active_work_and_query_recovers_without_production() -> Result<()> {
    let mut f = Fixture::new()?; let r = f.prepare("order-001")?;
    f.native.borrow_mut().lost_reply = true;
    let uncertain = f.call("fortress.commit",Fixture::commit_action(&r),1)?;
    assert_eq!(uncertain["result"]["creation_verified"],false);
    assert_eq!(uncertain["result"]["effect"]["state"],"indeterminate");
    assert_eq!(uncertain["agent_turn"]["active_work"]["unresolved_count"],1);
    let blocked = f.call("fortress.plan",Action::Plan { key:"another-key".into(),spec:r.plan().spec(),witness:r.plan().observation().witness() },1)?;
    assert_eq!(blocked["result"]["error"]["code"],"effect_indeterminate");
    let fenced = f.call_with("fortress.wait",Action::Wait { key:r.plan().key().into(),plan:r.plan().digest() },1,Limits::default(),false,false)?;
    assert_eq!(fenced["result"]["effect"]["state"],"indeterminate");
    assert_eq!(f.native.borrow().queries,0);
    let mut f = f.reopen(JournalMode::Reconcile,12)?;
    let recovered = f.call_with("fortress.wait",Action::Wait { key:r.plan().key().into(),plan:r.plan().digest() },1,Limits::default(),false,false)?;
    assert_eq!(recovered["result"]["effect"]["state"],"created");
    assert_eq!(recovered["agent_turn"]["briefing"]["development_production_granted"],false);
    assert_eq!(f.native.borrow().commits,1); assert_eq!(f.native.borrow().queries,1);
    Ok(())
}

#[test]
fn restart_offline_discovers_uncertainty_and_connected_query_mode_cannot_promote() -> Result<()> {
    let mut f = Fixture::new()?; let r = f.prepare("order-001")?;
    f.native.borrow_mut().lost_reply = true;
    f.call("fortress.commit",Fixture::commit_action(&r),1)?;
    let mut f = f.reopen(JournalMode::Offline,12)?;
    let page = f.call("fortress.query",Action::Query { filter:Filter::Pending,limit:2,continuation:None },2)?;
    assert_eq!(page["result"]["records"][0]["state"],"indeterminate");
    let blocked = f.call("fortress.wait",Action::Wait { key:r.plan().key().into(),plan:r.plan().digest() },1)?;
    assert_eq!(blocked["result"]["error"]["code"],"capability_denied"); assert_eq!(f.native.borrow().queries,0);
    let mut f = f.reopen(JournalMode::Reconcile,13)?;
    // An injected later grant does not change the journal mode or enable creation.
    f.state.grants = grants(JournalMode::Control,f.state.anchor.fortress_id,true)?;
    let denied = f.call("fortress.plan",Action::Plan { key:"new".into(),spec:r.plan().spec(),witness:r.plan().observation().witness() },1)?;
    assert_eq!(denied["result"]["error"]["code"],"capability_denied");
    let done = f.call("fortress.wait",Action::Wait { key:r.plan().key().into(),plan:r.plan().digest() },1)?;
    assert_eq!(done["result"]["effect"]["state"],"created"); assert_eq!(f.native.borrow().commits,1);
    Ok(())
}

#[test]
fn cancelled_preparation_is_retained_and_wait_never_queries_it() -> Result<()> {
    let mut f = Fixture::new()?; let r = f.prepare("cancelled")?;
    let cancelled = f.call("fortress.cancel",Action::Cancel { key:r.plan().key().into(),plan:r.plan().digest() },1)?;
    assert_eq!(cancelled["result"]["effect"]["state"],"cancelled_before_dispatch");
    assert_eq!(cancelled["result"]["manager_order_deleted"],false);
    let done = f.call("fortress.commit",Fixture::commit_action(&r),1)?;
    assert_eq!(done["result"]["creation_verified"],false);
    let replay = f.call("fortress.wait",Action::Wait { key:r.plan().key().into(),plan:r.plan().digest() },1)?;
    assert_eq!(replay["result"]["effect"],cancelled["result"]["effect"]);
    assert_eq!(f.native.borrow().commits,0); assert_eq!(f.native.borrow().queries,0);
    Ok(())
}

#[test]
fn output_reservation_failure_cannot_start_dispatch_and_preserves_discoverable_work() -> Result<()> {
    let mut f = Fixture::new()?; let r = f.prepare("budget")?;
    let before = f.store.bytes.borrow().get_ref().clone();
    for limits in [Limits { bytes:Some(BASE_RESERVE+RECORD_RESERVE),..Limits::default() },
        Limits { tokens:Some(1),..Limits::default() }]
    {
        let refused = f.call_with("fortress.commit",Fixture::commit_action(&r),1,limits,true,false)?;
        assert_eq!(refused["result"]["error"]["code"],"budget_exceeded");
        assert_eq!(refused["agent_turn"]["active_work"]["prepared_count"],1);
        assert_eq!(f.native.borrow().commits,0);
        assert_eq!(*f.store.bytes.borrow().get_ref(),before);
    }
    assert_eq!(f.call("fortress.commit",Fixture::commit_action(&r),1)?["result"]["creation_verified"],true);
    Ok(())
}

#[test]
fn current_grants_cancellation_and_exact_identity_are_rechecked_before_commit() -> Result<()> {
    let mut f = Fixture::new()?; let r = f.prepare("authority")?;
    let denied = f.call_with("fortress.commit",Fixture::commit_action(&r),1,Limits::default(),false,false)?;
    assert_eq!(denied["result"]["error"]["code"],"capability_denied");
    let cancelled = f.call_with("fortress.commit",Fixture::commit_action(&r),1,Limits::default(),true,true)?;
    assert_eq!(cancelled["result"]["ok"],false);
    for action in [Action::Commit { key:r.plan().key().into(),plan:Digest32::ZERO,witness:r.plan().observation().witness() },
        Action::Commit { key:r.plan().key().into(),plan:r.plan().digest(),witness:Digest32::ZERO }]
    { assert_eq!(f.call("fortress.commit",action,1)?["result"]["ok"],false); }
    assert_eq!(f.native.borrow().commits,0);
    Ok(())
}

#[test]
fn filtered_pages_advance_without_false_absence_and_reject_stale_or_rebound_cursors() -> Result<()> {
    let mut f = Fixture::new()?;
    for i in 0..65 {
        let r = f.prepare(&format!("a-{i:03}"))?;
        f.call("fortress.cancel",Action::Cancel { key:r.plan().key().into(),plan:r.plan().digest() },1)?;
    }
    let pending = f.prepare("z-pending")?;
    let first = f.call("fortress.query",Action::Query { filter:Filter::Pending,limit:2,continuation:None },2)?;
    assert_eq!(first["result"]["records"],json!([])); assert_eq!(first["result"]["matching_records_in_journal"],1);
    assert_eq!(first["result"]["complete_matching_set_in_this_response"],false);
    let token = first["result"]["continuation"].as_str().ok_or_else(||invalid("missing continuation"))?.to_owned();
    let wrong = f.call("fortress.query",Action::Query { filter:Filter::All,limit:2,continuation:Some(token.clone()) },2)?;
    assert_eq!(wrong["result"]["error"]["code"],"stale_anchor");
    let last = f.call("fortress.query",Action::Query { filter:Filter::Pending,limit:2,continuation:Some(token.clone()) },2)?;
    assert_eq!(last["result"]["records"][0]["idempotency_key"],"z-pending");
    assert!(last["result"]["continuation"].is_null());
    f.call("fortress.cancel",Action::Cancel { key:pending.plan().key().into(),plan:pending.plan().digest() },1)?;
    let stale = f.call("fortress.query",Action::Query { filter:Filter::Pending,limit:2,continuation:Some(token) },2)?;
    assert_eq!(stale["result"]["error"]["code"],"stale_anchor");
    Ok(())
}

#[test]
fn largest_complete_queue_and_record_fit_the_reserved_final_response() -> Result<()> {
    let mut bytes = b"DFMWO010".to_vec();
    for n in [u64::MAX-2,u64::MAX-3,MAX_NATIVE_TICK] { bytes.extend_from_slice(&n.to_be_bytes()); }
    bytes.extend_from_slice(&((i32::MAX-1) as u32).to_be_bytes());
    bytes.extend_from_slice(&(i32::MAX as u32).to_be_bytes()); bytes.push(1);
    let folder = "\u{0001}".repeat(512);
    bytes.extend_from_slice(&(folder.len() as u16).to_be_bytes()); bytes.extend_from_slice(folder.as_bytes());
    bytes.extend_from_slice(&4095u32.to_be_bytes());
    for n in (i32::MAX-5000)..(i32::MAX-905) { bytes.extend_from_slice(&(n as u32).to_be_bytes()); }
    let observation = WorkOrderObservation::decode(&bytes)?;
    let manifest = WorkOrderManifest { generation:observation.generation(),
        df_version:"\u{0001}".repeat(128),dfhack_version:"\u{0002}".repeat(128) };
    let mut f = Fixture::with_manifest(observation,manifest)?;
    let r = f.prepare_spec(&"x".repeat(128),WorkOrderSpec::new(WorkOrderRecipe::WoodenChair,100)?)?;
    assert!((record_json(&r).to_string().len() as u64) < RECORD_RESERVE);
    let result = f.call_with("fortress.explain",Action::Explain { key:r.plan().key().into(),plan:r.plan().digest() },1,
        Limits { tokens:Some(((BASE_RESERVE+RECORD_RESERVE)/4) as u32),..Limits::default() },true,false)?;
    assert_eq!(result["result"]["ok"],true);
    assert!(result.to_string().len() as u64 <= BASE_RESERVE+RECORD_RESERVE);
    assert_eq!(result["result"]["effect"]["original_observation"]["order_ids"].as_array().map(Vec::len),Some(4095));
    Ok(())
}

#[test]
fn malformed_requests_failed_refresh_and_lost_custody_do_not_offer_creation() -> Result<()> {
    let mut f = Fixture::new()?; let r = f.prepare("custody")?;
    f.native.borrow_mut().fail_read = true;
    let failed = f.call("fortress.observe",Action::Observe,1)?;
    assert_eq!(failed["agent_turn"]["affordances"][0]["enabled"],false);
    f.store.valid.set(false);
    let denied = f.call("fortress.commit",Fixture::commit_action(&r),1)?;
    assert_eq!(denied["result"]["ok"],false); assert_eq!(f.native.borrow().commits,0);
    assert_eq!(denied["agent_turn"]["active_work"]["state_known"],false);
    Ok(())
}

#[test]
fn exact_operator_gates_modes_and_session_namespaces_never_grant_ambient_admission() -> Result<()> {
    let keys = ALLOWED.iter().map(|s|s.to_string()).collect::<Vec<_>>();
    environment_contract(Some("1"),None,&keys,false)?;
    environment_contract(Some("1"),Some("1"),&keys,false)?;
    for opt in [None,Some("0"),Some("true"),Some(" 1")] {
        assert!(environment_contract(opt,None,&keys,false).is_err());
    }
    assert!(environment_contract(Some("1"),Some("0"),&keys,false).is_err());
    assert!(environment_contract(Some("1"),None,&keys,true).is_err());
    for name in ["DFMCP_ADMITTED_BRIDGE_PROTOCOL","DFMCP_JOB_CONTROL_TOKEN","DFMCP_ADMISSION_TICKET"] {
        let mut bad = keys.clone(); bad.push(name.into());
        assert!(environment_contract(Some("1"),None,&bad,false).is_err());
    }
    let fortress = FortressId::new(1);
    assert!(grants(JournalMode::Control,fortress,false).is_err());
    for mode in [JournalMode::Offline,JournalMode::Reconcile] {
        assert_eq!(grants(mode,fortress,true)?.iter().map(|g|g.capability).collect::<Vec<_>>(),[Capability::Query]);
    }
    let id = SessionId::new((1u128<<127)|FAMILY|1);
    assert_eq!(session_id(&id.to_string())?,id);
    let old = SessionId::new((1u128<<127)|(9u128<<57)|1);
    assert!(session_id(&old.to_string()).is_err());
    assert!(session_id(&id.to_string().to_uppercase()).is_err());
    Ok(())
}

#[test]
fn runtime_io_preserves_inherited_restrictions_and_cancellation() -> std::result::Result<(),Box<dyn std::error::Error>> {
    use fastmcp_rust::asupersync::{cx::cap,types::CancelKind};
    crate::run_with_runtime_cx(|cx| async move {
        runtime_io()?;
        {
            let _restricted = cx.restrict::<cap::None>().set_current_restricted();
            assert!(matches!(runtime_io(),Err(e) if e.code == ErrorCode::CapabilityDenied));
        }
        runtime_io()?;
        cx.cancel_with(CancelKind::User,Some("creation I/O boundary test"));
        assert!(matches!(runtime_io(),Err(e) if e.code == ErrorCode::CancellationRequested));
        Ok::<_,Box<dyn std::error::Error>>(())
    })??;
    Ok(())
}
