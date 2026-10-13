//! Session-level proof that retained terminal uncertainty reaches explicit
//! exact-source receipt reconciliation without recreating dispatch authority.
use super::*;
use crate::build_placement::{BuildPhase, BuildPreparation, BuildRecord};
use crate::control_effect_journal::EffectJournalStorage;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, ObservationCursor, RequestId, RiskTier,
    SessionId, StateAnchor, WorkBudget,
};
use std::cell::RefCell;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::net::SocketAddr;
use std::rc::Rc;

fn fixture(name: &str) -> Result<Vec<u8>> {
    let text = include_str!("../../../../bridge/common/tests/fixtures/build_placement_v1_19.json");
    let needle = format!("\"{name}\": \"");
    let start = text.find(&needle).ok_or_else(|| error(ErrorCode::CorruptLedger, "fixture"))? + needle.len();
    let hex = text[start..].split('"').next().ok_or_else(|| error(ErrorCode::CorruptLedger, "fixture"))?;
    hex.as_bytes().chunks_exact(2).map(|pair| {
        u8::from_str_radix(std::str::from_utf8(pair).map_err(|_| error(ErrorCode::CorruptLedger, "fixture"))?, 16)
            .map_err(|_| error(ErrorCode::CorruptLedger, "fixture"))
    }).collect()
}
fn record(name: &str) -> Result<BuildRecord> { BuildRecord::decode(&fixture(name)?) }

#[derive(Clone, Default)]
struct Memory(Rc<RefCell<Cursor<Vec<u8>>>>);
impl Read for Memory { fn read(&mut self,b:&mut[u8])->io::Result<usize>{self.0.borrow_mut().read(b)}}
impl Write for Memory {
    fn write(&mut self,b:&[u8])->io::Result<usize>{self.0.borrow_mut().write(b)}
    fn flush(&mut self)->io::Result<()> {Ok(())}
}
impl Seek for Memory { fn seek(&mut self,p:SeekFrom)->io::Result<u64>{self.0.borrow_mut().seek(p)}}
impl EffectJournalStorage for Memory {
    fn sync(&mut self)->io::Result<()> {Ok(())}
    fn truncate(&mut self,_:u64)->io::Result<()> {Err(io::Error::other("repair forbidden"))}
}

#[derive(Default)]
struct Counters { queries: usize, prepares: usize, commits: usize, fenced: usize }
struct Native {
    binding: BuildBinding,
    recovery: bool,
    counters: Rc<RefCell<Counters>>,
}
impl BuildSource for Native {
    fn binding(&self)->&BuildBinding { &self.binding }
    fn native_summary(&self)->BuildNativeSummary {
        BuildNativeSummary::new(false, if self.recovery {1} else {0}).unwrap()
    }
    fn fence(&mut self){ self.counters.borrow_mut().fenced += 1; }
    fn observe(&mut self,_:BuildSelection,_:&OperationContext,_:Duration)->Result<BuildCapture>{
        BuildCapture::decode(&fixture("capture")?)
    }
    fn prepare(&mut self,_:&BuildPlan,_:&OperationContext,_:Duration)->Result<BuildPreparation>{
        self.counters.borrow_mut().prepares += 1;
        BuildPreparation::new(record("prepared")?, false)
    }
    fn commit(&mut self,_:super::super::journal::BuildDispatch<'_>,_:&OperationContext,_:Duration)->Result<BuildRecord>{
        self.counters.borrow_mut().commits += 1;
        record("indeterminate")
    }
    fn query(&mut self,_:&BuildPlan,_:&OperationContext,_:Duration)->Result<Option<BuildRecord>>{
        self.counters.borrow_mut().queries += 1;
        Ok(if self.recovery { Some(record("placed")?) } else { None })
    }
    fn cancel(&mut self,_:&BuildPlan,_:&OperationContext,_:Duration)->Result<BuildRecord>{
        Err(error(ErrorCode::InternalInvariantViolation,"cancel must stay offline"))
    }
}
struct Guard;
impl BuildGuard for Guard {
    fn check(&mut self,_:BuildStage,_:&BuildBinding,_:Option<&BuildPlan>,_:BuildSelection,_:&OperationContext)->Result<()> { Ok(()) }
}
fn context(query_only: bool)->Result<OperationContext>{
    let before=BuildCapture::decode(&fixture("capture")?)?;
    let caps: Vec<_> = if query_only { vec![Capability::Query] } else {
        vec![Capability::Query,Capability::Observe,Capability::Plan,Capability::Construct]
    };
    Ok(OperationContext{
        session_id:SessionId::new(91), request_id:RequestId::new(92),
        anchor:StateAnchor{fortress_id:before.fortress_id(),tick:GameTick(before.tick()),
            cursor:ObservationCursor::ORIGIN,state_hash:before.witness()},
        budget:WorkBudget{max_wall_millis:60_000,max_bytes:128*1024*1024,..WorkBudget::CONSERVATIVE_DEFAULT},
        grants:caps.into_iter().map(|capability|CapabilityGrant{capability,scope:CapabilityScope::default(),
            max_risk:RiskTier::Guarded,expires_at_tick:None,remaining_uses:None}).collect(),
        cancellation_requested:false,
    })
}

fn uncertain_session() -> Result<(BuildSession<Memory,Native>, Digest32, Rc<RefCell<Counters>>)> {
    let full=context(false)?;
    let before=BuildCapture::decode(&fixture("capture")?)?;
    let binding=BuildBinding::new(SocketAddr::from(([127,0,0,1],5000)),"df","dfhack",&before)?;
    let memory=Memory::default();
    let journal=BuildJournal::open(memory,&full,BuildMode::Control,Some(binding.clone()),Some([7;32]))?;
    let mut session=BuildSession::new(journal,&full)?;
    let counts=Rc::new(RefCell::new(Counters::default()));
    let initial=counts.clone();
    session.observe(before.selection(),&full,|_,_,_|Ok(Native{binding:binding.clone(),recovery:false,counters:initial}),&mut Guard)?;
    let prepared=session.prepare("golden",before.witness(),&full,&mut Guard)?;
    let digest=prepared.plan().digest();
    let uncertain=session.commit("golden",digest,&full,&mut Guard)?;
    assert_eq!(uncertain.native().map(BuildRecord::phase),Some(BuildPhase::Indeterminate));
    Ok((session,digest,counts))
}

#[test]
fn query_recovery_reaches_exact_positive_receipt_without_redispatch() -> Result<()> {
    let (mut session,digest,counts)=uncertain_session()?;
    let query=context(true)?;
    let binding=session.binding().clone();
    let shared=counts.clone();
    let resolved=session.recover("golden",digest,false,&query,
        |_,_,_|Ok(Native{binding,recovery:true,counters:shared}),&mut Guard)?;
    assert_eq!(resolved.native().map(BuildRecord::phase),Some(BuildPhase::Placed));
    assert!(!resolved.unresolved());
    let c=counts.borrow();
    assert_eq!(c.prepares,1); assert_eq!(c.commits,1); assert_eq!(c.queries,2);
    assert!(!session.has_preparation_connection());
    Ok(())
}

#[test]
fn cancellation_of_attempted_terminal_uncertainty_stays_offline() -> Result<()> {
    let (mut session,digest,counts)=uncertain_session()?;
    let query=context(true)?;
    let before=counts.borrow().queries;
    let unresolved=session.recover("golden",digest,true,&query,
        |_,_,_|Err(error(ErrorCode::InternalInvariantViolation,"factory must not run")),&mut Guard)?;
    assert_eq!(unresolved.native().map(BuildRecord::phase),Some(BuildPhase::Indeterminate));
    assert_eq!(counts.borrow().queries,before);
    Ok(())
}
