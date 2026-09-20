use super::*;
use dfmcp_core::{Capability, CapabilityGrant, CapabilityScope, FortressId, GameTick,
    ObservationCursor, OperationContext, RequestId, RiskTier, SessionId, StateAnchor, WorkBudget};

pub(super) fn observation() -> Result<RunObservation> {
    let mut bytes = b"DFMRO013".to_vec();
    for value in [41u64, 0, 100] { bytes.extend_from_slice(&value.to_be_bytes()); }
    bytes.extend_from_slice(&[1, 1, 1]); RunObservation::decode(&bytes)
}
pub(super) fn plan() -> Result<RunPlan> { RunPlan::new("test", RunSpec::new(10, 1000)?, observation()?) }
pub(super) fn context() -> OperationContext {
    OperationContext { session_id: SessionId::new(1), request_id: RequestId::new(2),
        anchor: StateAnchor { fortress_id: FortressId::NIL, cursor: ObservationCursor { epoch:41, sequence:0 },
            tick: GameTick(100), state_hash: Digest32::ZERO },
        budget: WorkBudget { max_wall_millis: 60_000, max_output_tokens: 4096, ..WorkBudget::default() },
        grants: [Capability::Query, Capability::Plan, Capability::ControlClock].into_iter().map(|capability|
            CapabilityGrant { capability, scope: CapabilityScope { fortress_id: Some(FortressId::NIL), ..CapabilityScope::default() },
                max_risk: RiskTier::Guarded, expires_at_tick: None, remaining_uses: None }).collect(),
        cancellation_requested:false }
}
pub(super) fn raw_record(plan: &RunPlan, phase: u8, reason: u8, attempted: bool, verified: bool, tick: Option<u64>) -> Vec<u8> {
    let mut bytes = b"DFMRE013".to_vec(); bytes.extend_from_slice(&plan.canonical_bytes());
    bytes.extend_from_slice(plan.digest().as_bytes()); bytes.extend_from_slice(plan.token());
    bytes.extend_from_slice(&[phase, reason, u8::from(attempted), u8::from(verified), u8::from(tick.is_some())]);
    bytes.extend_from_slice(&tick.unwrap_or(0).to_be_bytes());
    let receipt = hash(b"dfmcp-bounded-run-receipt/1", &bytes); bytes.extend_from_slice(receipt.as_bytes()); bytes
}
#[test]
fn sealed_intent_and_observation_round_trip() -> Result<()> {
    let plan = plan()?; assert_eq!(RunPlan::decode(&plan.canonical_bytes())?, plan);
    assert_eq!(observation()?.tick(), Some(100));
    for bytes in [vec![], vec![0; 34], vec![0; 36]] { assert!(RunObservation::decode(&bytes).is_err()); }
    let mut bytes = *plan.before().canonical_bytes(); bytes[32] = 0;
    assert!(RunObservation::decode(&bytes).is_err());
    for spec in [(0,1),(1201,1),(1,0),(1,60001)] { assert!(RunSpec::new(spec.0,spec.1).is_err()); }
    assert!(RunPlan::new("bad key",plan.spec(),observation()?).is_err()); Ok(())
}
#[test]
fn all_phase_reason_flags_preserve_uncertainty() -> Result<()> {
    let plan = plan()?;
    for phase in 0..6 { for reason in 0..10 { for flags in 0..8 {
        let attempted = flags & 1 != 0; let verified = flags & 2 != 0; let known = flags & 4 != 0;
        let state_ok = match phase {
            0 => reason == 0 && !known, 1 => reason == 0 && known,
            2 => [1,2,3,5,6,8].contains(&reason), 3 => [1,2,3,4,5,6,8].contains(&reason),
            4 => [3,7,9].contains(&reason) && !known, _ => reason == 7 && !known,
        };
        let expected = state_ok && attempted == [1,2,3,5].contains(&phase) && verified == (phase == 3);
        let bytes = raw_record(&plan, phase, reason, attempted, verified, known.then_some(100));
        assert_eq!(RunRecord::decode(&bytes).is_ok(), expected, "phase={phase} reason={reason} flags={flags}");
    } } } Ok(())
}
#[test]
fn record_corruption_and_every_truncated_prefix_rejected() -> Result<()> {
    let bytes = raw_record(&plan()?,3,1,true,true,Some(110));
    for i in 0..bytes.len() {
        assert!(RunRecord::decode(&bytes[..i]).is_err());
        let mut corrupt = bytes.clone(); corrupt[i] ^= 1; assert!(RunRecord::decode(&corrupt).is_err());
    }
    let mut trailing = bytes; trailing.push(0); assert!(RunRecord::decode(&trailing).is_err()); Ok(())
}
#[test]
fn stopped_is_historical_and_overshoot_is_not_hidden() -> Result<()> {
    let plan = plan()?; let record = RunRecord::decode(&raw_record(&plan,3,2,true,true,Some(125)))?;
    assert_eq!(record.observed_ticks_advanced(),Some(25)); assert_eq!(record.observed_tick_overshoot(),Some(15));
    let record = RunRecord::decode(&raw_record(&plan,3,5,true,true,None))?;
    assert!(record.pause_verified()); assert_eq!(record.observed_tick(),None);
    let lost = RunRecord::decode(&raw_record(&plan,5,7,true,false,None))?;
    assert!(!lost.pause_verified()); assert!(lost.unpause_attempted()); Ok(())
}
#[test]
fn canonical_reference_fixture_matches_exact_rust_encoding() -> Result<()> {
    let text = include_str!("../../tests/fixtures/bounded_run_stopped_v1_13.hex").trim();
    let mut bytes = Vec::new();
    for pair in text.as_bytes().chunks_exact(2) {
        let pair = std::str::from_utf8(pair).map_err(|_|error(ErrorCode::InvalidRequest,"fixture UTF-8"))?;
        bytes.push(u8::from_str_radix(pair,16).map_err(|_|error(ErrorCode::InvalidRequest,"fixture hex"))?);
    }
    assert_eq!(bytes,raw_record(&plan()?,3,1,true,true,Some(110)));
    assert_eq!(RunRecord::decode(&bytes)?.phase(),RunPhase::Stopped); Ok(())
}
#[test]
fn control_requires_full_risk_scope_and_run_horizon() -> Result<()> {
    let plan = plan()?; let mut c = context(); rpc::authorize_plan(&c,&plan)?;
    c.grants[2].expires_at_tick = Some(GameTick(109)); assert!(rpc::authorize_plan(&c,&plan).is_err());
    c.grants[2].expires_at_tick = Some(GameTick(110)); rpc::authorize_plan(&c,&plan)?;
    c.grants[2].remaining_uses = Some(1); assert!(rpc::authorize_plan(&c,&plan).is_err());
    c.grants[2].remaining_uses = None; c.budget.max_game_ticks = 9; assert!(rpc::authorize_plan(&c,&plan).is_err());
    c.budget.max_game_ticks = 10; c.anchor.fortress_id = FortressId::new(1); assert!(rpc::authorize_plan(&c,&plan).is_err());
    c.anchor.fortress_id = FortressId::NIL; c.cancellation_requested = true; assert!(rpc::authorize_plan(&c,&plan).is_err()); Ok(())
}
