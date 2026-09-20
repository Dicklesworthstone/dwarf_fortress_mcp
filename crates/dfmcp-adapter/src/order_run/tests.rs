use super::*;
use dfmcp_core::{Capability, CapabilityGrant, CapabilityScope, GameTick, ObservationCursor,
    OperationContext, RequestId, RiskTier, SessionId, StateAnchor, WorkBudget};

pub(super) fn capture_bytes(folder: &str, sequence: u64, tick: u64, paused: bool, status: u32) -> Vec<u8> {
    let mut out = b"DFMOR014".to_vec();
    for n in [41u64, sequence, tick] { out.extend_from_slice(&n.to_be_bytes()); }
    out.extend_from_slice(&7u32.to_be_bytes()); put_field(&mut out, folder.as_bytes()); out.push(u8::from(paused));
    for n in [9u32, 10] { out.extend_from_slice(&n.to_be_bytes()); }
    out.extend_from_slice(&[1, 1]);
    for n in [5u32, 5, status] { out.extend_from_slice(&n.to_be_bytes()); }
    out
}
pub(super) fn plan() -> Result<OrderRunPlan> {
    OrderRunPlan::new("approval", OrderRunSpec::new(20, 1000, OrderPredicate::Approved, 2, 2)?,
        OrderCapture::decode(&capture_bytes("region1", 3, 100, true, 0))?)
}
pub(super) fn context() -> Result<OperationContext> {
    let fortress = FortressIdentity::new("region1", 7)?.fortress_id();
    Ok(OperationContext { session_id: SessionId::new(1), request_id: RequestId::new(1),
        anchor: StateAnchor { fortress_id: fortress, cursor: ObservationCursor::ORIGIN, tick: GameTick(100), state_hash: Digest32::ZERO },
        budget: WorkBudget { max_bytes: 16 * 1024 * 1024, max_wall_millis: 5000, max_game_ticks: 1200,
            max_entities: 256, max_actions: 1, max_output_tokens: 65536 }, cancellation_requested: false,
        grants: [Capability::Query, Capability::Plan, Capability::ControlClock].into_iter().map(|capability| CapabilityGrant {
            capability, scope: CapabilityScope { fortress_id: Some(fortress), ..CapabilityScope::default() },
            max_risk: RiskTier::Guarded, expires_at_tick: None, remaining_uses: None,
        }).collect() })
}
pub(super) fn record_bytes(plan: &OrderRunPlan, phase: u8, reason: u8, trigger: u8,
    sample: Option<&[u8]>, count: u32) -> Vec<u8>
{
    let mut out = b"DFMOE014".to_vec(); put_field(&mut out, plan.key().as_bytes()); put_field(&mut out, plan.native_bytes());
    out.extend_from_slice(plan.digest().as_bytes()); out.extend_from_slice(plan.token());
    let known = matches!(phase, 1..=3);
    out.extend_from_slice(&[phase, reason, trigger, u8::from(matches!(phase, 1..=3 | 5)), u8::from(phase == 3), u8::from(known)]);
    out.extend_from_slice(&(if known { 105u64 } else { 0 }).to_be_bytes()); out.extend_from_slice(&count.to_be_bytes());
    out.extend_from_slice(&(if count > 0 { 105u64 } else { 100 }).to_be_bytes());
    put_field(&mut out, sample.unwrap_or_default());
    let proof = hash(b"dfmcp-order-run-receipt/1", &out); out.extend_from_slice(proof.as_bytes()); out
}
fn unhex(raw: &str) -> Result<Vec<u8>> {
    raw.trim().as_bytes().chunks(2).map(|pair| {
        let s = std::str::from_utf8(pair).map_err(|_| error(ErrorCode::InvalidRequest, "fixture UTF-8"))?;
        u8::from_str_radix(s, 16).map_err(|_| error(ErrorCode::InvalidRequest, "fixture hex"))
    }).collect()
}
#[test]
fn independent_predicate_fixture_and_sealed_round_trip() -> Result<()> {
    let fixture = unhex(include_str!("../../tests/fixtures/order_run_predicate_v1_14.hex"))?;
    let value = OrderRunRecord::decode(&fixture)?;
    assert_eq!(value.plan().key(), "approval"); assert!(value.pause_verified() && value.predicate_observed());
    assert_eq!(value.reported_stable_samples(), 2); assert_eq!(value.counted_tick(), 105);
    assert_eq!(value.plan().before().fortress().folder(), "region1");
    assert_eq!(value.plan().before().fortress().site(), 7);
    assert_eq!(OrderRunPlan::decode(&value.plan().canonical_bytes())?, *value.plan());
    assert_eq!(value.canonical_bytes(), fixture); Ok(())
}
#[test]
fn every_corruption_and_truncated_prefix_is_rejected() -> Result<()> {
    let raw = unhex(include_str!("../../tests/fixtures/order_run_predicate_v1_14.hex"))?;
    for i in 0..raw.len() {
        assert!(OrderRunRecord::decode(&raw[..i]).is_err());
        let mut corrupt = raw.clone(); corrupt[i] ^= 1; assert!(OrderRunRecord::decode(&corrupt).is_err());
    }
    let mut extra = raw; extra.push(0); assert!(OrderRunRecord::decode(&extra).is_err()); Ok(())
}
#[test]
fn condition_already_true_and_bad_cadence_never_seal() -> Result<()> {
    for predicate in [OrderPredicate::Approved, OrderPredicate::Active, OrderPredicate::RemainingAtMost(5)] {
        let spec = OrderRunSpec::new(20, 1000, predicate, 2, 2)?;
        let before = OrderCapture::decode(&capture_bytes("region1", 3, 100, true, 3))?;
        assert!(OrderRunPlan::new("bad", spec, before).is_err());
    }
    for (samples, interval, ticks) in [(0,1,10), (17,1,20), (1,0,20), (2,11,20), (1,1201,1200)] {
        assert!(OrderRunSpec::new(ticks, 1000, OrderPredicate::Approved, samples, interval).is_err());
    }
    assert!(OrderPredicate::from_code(1,1).is_err()); assert!(OrderPredicate::from_code(3,101).is_err()); Ok(())
}
#[test]
fn exact_identity_and_intent_changes_are_committed() -> Result<()> {
    let p = plan()?;
    let another_key = OrderRunPlan::new("another", p.spec(), p.before().clone())?;
    assert_eq!(p.digest(), another_key.digest()); assert_ne!(p.token(), another_key.token());
    let elsewhere = OrderRunPlan::new(p.key(), p.spec(), OrderCapture::decode(&capture_bytes("region2", 3, 100, true, 0))?)?;
    assert_ne!(p.digest(), elsewhere.digest());
    assert_ne!(p.before().fortress().fortress_id(), elsewhere.before().fortress().fortress_id());
    assert!(OrderRunRecord::decode(&record_bytes(&p,0,0,0,None,0))?.validate_successor(
        &OrderRunRecord::decode(&record_bytes(&elsewhere,0,0,0,None,0))?).is_err()); Ok(())
}
#[test]
fn rehashed_false_goal_and_wrong_sample_source_are_rejected() -> Result<()> {
    let p = plan()?;
    let negative = capture_bytes("region1",4,105,false,0);
    let same_tick = capture_bytes("region1",4,100,false,1);
    let wrong_source = capture_bytes("region2",4,105,false,1);
    let wrong_sequence = capture_bytes("region1",3,105,false,1);
    for sample in [negative, same_tick, wrong_source, wrong_sequence] {
        assert!(OrderRunRecord::decode(&record_bytes(&p,3,3,1,Some(&sample),2)).is_err());
    }
    assert!(OrderRunRecord::decode(&record_bytes(&p,3,1,1,Some(&capture_bytes("region1",4,105,false,1)),2)).is_err());
    Ok(())
}
#[test]
fn maximal_native_and_intent_bounds_round_trip() -> Result<()> {
    let p = OrderRunPlan::new(&"k".repeat(128), OrderRunSpec::new(20,1000,OrderPredicate::Approved,2,2)?,
        OrderCapture::decode(&capture_bytes(&"x".repeat(512),3,100,true,0))?)?;
    assert_eq!(p.native_bytes().len(), MAX_PLAN_BYTES); assert_eq!(p.canonical_bytes().len(), MAX_INTENT_BYTES);
    let sample = capture_bytes(&"x".repeat(512),4,105,false,1);
    let raw = record_bytes(&p,3,3,1,Some(&sample),2);
    assert_eq!(raw.len(),MAX_RECORD_BYTES); assert!(OrderRunRecord::decode(&raw)?.predicate_observed()); Ok(())
}
#[test]
fn terminal_and_trigger_evidence_cannot_change_on_recovery() -> Result<()> {
    let p = plan()?; let sample = capture_bytes("region1",4,105,false,1);
    let stopping = OrderRunRecord::decode(&record_bytes(&p,2,3,1,Some(&sample),2))?;
    let stopped = OrderRunRecord::decode(&record_bytes(&p,3,3,1,Some(&sample),2))?;
    stopping.validate_successor(&stopped)?; stopped.validate_successor(&stopped)?;
    let clock_stop = OrderRunRecord::decode(&record_bytes(&p,2,2,0,None,0))?;
    assert!(clock_stop.validate_successor(&stopped).is_err());
    let source_lost = OrderRunRecord::decode(&record_bytes(&p,5,7,6,None,0))?;
    clock_stop.validate_successor(&source_lost)?;
    assert!(stopped.validate_successor(&stopping).is_err());
    let running = OrderRunRecord::decode(&record_bytes(&p,1,0,0,None,0))?;
    assert!(stopping.validate_successor(&running).is_err());
    assert!(running.validate_successor(&OrderRunRecord::decode(&record_bytes(&p,0,0,0,None,0))?).is_err()); Ok(())
}
#[test]
fn authority_covers_exact_fortress_risk_and_entire_tick_horizon() -> Result<()> {
    let p = plan()?; let c = context()?; rpc::authorize_plan(&c,&p)?;
    let mut bad = c.clone(); bad.anchor.fortress_id = FortressId::new(22); assert!(rpc::authorize_plan(&bad,&p).is_err());
    bad = c.clone(); bad.budget.max_game_ticks = 19; assert!(rpc::authorize_plan(&bad,&p).is_err());
    bad = c.clone(); bad.grants.retain(|g|g.capability != Capability::Plan); assert!(rpc::authorize_plan(&bad,&p).is_err());
    bad = c.clone(); bad.cancellation_requested = true; assert!(rpc::authorize_plan(&bad,&p).is_err());
    bad = c.clone(); for g in &mut bad.grants { g.expires_at_tick = Some(GameTick(119)); } assert!(rpc::authorize_plan(&bad,&p).is_err());
    bad = c.clone(); for g in &mut bad.grants { g.remaining_uses = Some(1); } assert!(rpc::authorize_plan(&bad,&p).is_err()); Ok(())
}
