use super::*;
use crate::dig_designation::{DigObservation, rpc::DigManifest};
use dfmcp_core::{Capability, CapabilityGrant, CapabilityScope, MapCoord, ObservationCursor,
    RequestId, RiskTier, StateAnchor, WorkBudget};

fn plan() -> Result<DigPlan> {
    let raw = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/native/dig_designation/vectors/observation.hex")).trim();
    let bytes = (0..raw.len()).step_by(2).map(|i| u8::from_str_radix(&raw[i..i+2], 16)
        .map_err(|_| error(ErrorCode::InvalidRequest, "bad fixture"))).collect::<Result<Vec<_>>>()?;
    DigPlan::new("dig-001", false, DigObservation::decode(&bytes)?)
}
fn scope() -> MapCuboid { MapCuboid { min: MapCoord::new(0,0,0), max: MapCoord::new(63,63,7) } }
fn context() -> Result<OperationContext> {
    let p = plan()?;
    Ok(OperationContext { session_id: SessionId::new(1), request_id: RequestId::new(1),
        anchor: StateAnchor { fortress_id:p.before().fortress_id(), cursor:ObservationCursor::ORIGIN,
            tick:GameTick(p.before().tick()), state_hash:p.before().witness() },
        budget:WorkBudget { max_wall_millis:10_000, max_game_ticks:0, max_entities:300,
            max_bytes:16*1024*1024, max_output_tokens:8192, max_actions:1 },
        grants:[Capability::Query,Capability::Observe,Capability::Plan,Capability::Designate].into_iter().map(|capability|
            CapabilityGrant { capability, scope:CapabilityScope { fortress_id:Some(p.before().fortress_id()),
                map_area:Some(scope()), ..CapabilityScope::default() }, max_risk:RiskTier::Guarded,
                expires_at_tick:None, remaining_uses:None }).collect(), cancellation_requested:false })
}
fn policy(book:&mut LeaseManager, mode:DigCheckpointPolicy, protected:Vec<MapCuboid>) -> Result<DigControlPolicy> {
    let p=plan()?; let c=context()?;
    let lease=book.acquire_spatial_lease(c.session_id, scope(), true, c.anchor.tick, 10)?;
    let binding=DigBinding::new("127.0.0.1:5000".parse().map_err(|_|error(ErrorCode::InvalidRequest,"endpoint"))?,
        DigManifest { generation:7,df_version:"test-df".into(),dfhack_version:"test-dfhack".into() },p.before(),scope())?;
    DigControlPolicy::new(binding,Digest32::of_bytes(b"journal"),c.session_id,lease,protected,mode)
}
#[derive(Default)]
struct Runtime { revoked:bool, calls:usize }
impl DigGuard for Runtime {
    fn check(&mut self,_:DigStage,_:&DigPlan,_:&OperationContext)->Result<()> {
        self.calls+=1;
        if self.revoked {Err(error(ErrorCode::CapabilityDenied,"revoked"))}else{Ok(())}
    }
}
impl DigSessionGuard for Runtime {
    fn connect(&mut self,_:&DigBinding,_:DigRegion,_:&OperationContext)->Result<()> {Ok(())}
    fn observe(&mut self,_:&DigBinding,_:DigRegion,_:&OperationContext)->Result<()> {Ok(())}
}

#[test]
fn default_checkpoint_policy_refuses_without_inventing_evidence()->Result<()> {
    let mut book=LeaseManager::new();let policy=policy(&mut book,DigCheckpointPolicy::default(),vec![])?;
    let denied=policy.evaluate(&plan()?,&context()?,&book);
    assert!(matches!(denied,Err(e) if e.code==ErrorCode::CheckpointRequired));
    Ok(())
}
#[test]
fn protected_shared_block_outside_target_still_blocks_designation()->Result<()> {
    let p=plan()?; assert_eq!(p.before().region().coordinates(),[15,15,2,2,2]);
    let protected=MapCuboid {min:MapCoord::new(0,0,2),max:MapCoord::new(0,0,2)};
    let mut book=LeaseManager::new();let policy=policy(&mut book,DigCheckpointPolicy::DisposableFortress,vec![protected])?;
    assert!(policy.evaluate(&p,&context()?,&book).is_err());
    Ok(())
}
#[test]
fn protected_scope_is_canonical_bounded_and_part_of_review()->Result<()> {
    let a=MapCuboid {min:MapCoord::new(40,40,2),max:MapCoord::new(41,41,2)};
    let b=MapCuboid {min:MapCoord::new(50,50,2),max:MapCoord::new(51,51,2)};
    let mut book=LeaseManager::new();let one=policy(&mut book,DigCheckpointPolicy::DisposableFortress,vec![a,b])?;
    let two=DigControlPolicy::new(one.binding.clone(),one.journal,one.session,one.lease,vec![b,a,a],one.checkpoint)?;
    assert_eq!(one,two);
    assert!(DigControlPolicy::new(one.binding.clone(),one.journal,one.session,one.lease,vec![a;33],one.checkpoint).is_err());
    let other=DigControlPolicy::new(one.binding.clone(),one.journal,one.session,one.lease,vec![],one.checkpoint)?;
    assert_ne!(one.review(&plan()?,&context()?,&book)?.seal(),other.review(&plan()?,&context()?,&book)?.seal());
    Ok(())
}
#[test]
fn review_seal_binds_key_journal_session_policy_and_lease()->Result<()> {
    let mut book=LeaseManager::new();let policy=policy(&mut book,DigCheckpointPolicy::DisposableFortress,vec![])?;
    let p=plan()?;let c=context()?;let review=policy.review(&p,&c,&book)?;
    let seal=review.seal();assert!(review.confirm(Digest32::ZERO).is_err());
    let confirmed=policy.review(&p,&c,&book)?.confirm(seal)?;
    let mut runtime=Runtime::default();
    PolicyDigGuard::new(&policy,&book,&mut runtime,Some(&confirmed)).check(DigStage::Commit,&p,&c)?;
    let other=DigPlan::new("other-key",false,p.before().clone())?;
    assert!(PolicyDigGuard::new(&policy,&book,&mut runtime,Some(&confirmed)).check(DigStage::Commit,&other,&c).is_err());
    let mut changed=policy.clone();changed.journal=Digest32::of_bytes(b"different");
    let changed=DigControlPolicy::new(changed.binding,changed.journal,changed.session,changed.lease,changed.protected,changed.checkpoint)?;
    assert!(PolicyDigGuard::new(&changed,&book,&mut runtime,Some(&confirmed)).check(DigStage::Commit,&p,&c).is_err());
    Ok(())
}
#[test]
fn actual_lease_release_or_expiry_revokes_even_a_confirmed_review()->Result<()> {
    let mut book=LeaseManager::new();let policy=policy(&mut book,DigCheckpointPolicy::DisposableFortress,vec![])?;
    let p=plan()?;let mut c=context()?;let review=policy.review(&p,&c,&book)?;let seal=review.seal();let confirmed=review.confirm(seal)?;
    let mut runtime=Runtime::default();
    c.anchor.tick=GameTick(c.anchor.tick.get()+10);
    assert!(PolicyDigGuard::new(&policy,&book,&mut runtime,Some(&confirmed)).check(DigStage::Commit,&p,&c).is_err());
    c=context()?;book.release_lease(policy.lease,c.session_id)?;
    assert!(PolicyDigGuard::new(&policy,&book,&mut runtime,Some(&confirmed)).check(DigStage::Commit,&p,&c).is_err());
    Ok(())
}
#[test]
fn runtime_revocation_and_current_authority_are_rechecked()->Result<()> {
    let mut book=LeaseManager::new();let policy=policy(&mut book,DigCheckpointPolicy::DisposableFortress,vec![])?;
    let p=plan()?;let mut c=context()?;let mut runtime=Runtime::default();
    PolicyDigGuard::new(&policy,&book,&mut runtime,None).check(DigStage::Prepare,&p,&c)?;
    assert!(PolicyDigGuard::new(&policy,&book,&mut runtime,None).check(DigStage::Commit,&p,&c).is_err());
    runtime.revoked=true;
    assert!(PolicyDigGuard::new(&policy,&book,&mut runtime,None).check(DigStage::Prepare,&p,&c).is_err());
    runtime.revoked=false;c.grants.retain(|g|g.capability!=Capability::Designate);
    assert!(PolicyDigGuard::new(&policy,&book,&mut runtime,None).check(DigStage::Prepare,&p,&c).is_err());
    c=context()?;c.session_id=SessionId::new(2);assert!(policy.evaluate(&p,&c,&book).is_err());
    c=context()?;c.cancellation_requested=true;assert!(policy.evaluate(&p,&c,&book).is_err());
    Ok(())
}
#[test]
fn query_and_retirement_are_not_blocked_by_unavailable_checkpoint_or_lease()->Result<()> {
    let mut book=LeaseManager::new();let policy=policy(&mut book,DigCheckpointPolicy::Required,vec![scope()])?;
    let c=context()?;book.release_lease(policy.lease,c.session_id)?;
    let mut runtime=Runtime::default();let mut guard=PolicyDigGuard::new(&policy,&book,&mut runtime,None);
    guard.check(DigStage::Query,&plan()?,&c)?;guard.check(DigStage::Cancel,&plan()?,&c)?;
    assert!(guard.check(DigStage::Prepare,&plan()?,&c).is_err());
    Ok(())
}
