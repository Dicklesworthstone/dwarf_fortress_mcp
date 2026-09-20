use super::*;
use dfmcp_core::{Capability, CapabilityGrant, CapabilityScope, GameTick, ObservationCursor,
    OperationContext, RequestId, RiskTier, SessionId, StateAnchor, WorkBudget};

pub(super) fn unhex(text: &str) -> Result<Vec<u8>> {
    let text = text.trim(); require(text.len().is_multiple_of(2), "odd fixture")?;
    text.as_bytes().chunks_exact(2).map(|b| {
        let s = std::str::from_utf8(b).map_err(|_| error(ErrorCode::InvalidRequest, "fixture UTF-8"))?;
        u8::from_str_radix(s, 16).map_err(|_| error(ErrorCode::InvalidRequest, "fixture hex"))
    }).collect()
}
pub(super) fn capture() -> Result<WorkforceCapture> {
    WorkforceCapture::decode(&unhex(include_str!("../../../../tests/native/workforce/vectors/capture.hex"))?)
}
pub(super) fn plan() -> Result<AssignmentPlan> { AssignmentPlan::new("assign", AssignmentSpec::new(0, true)?, capture()?) }
pub(super) fn applied() -> Result<Vec<u8>> { unhex(include_str!("../../../../tests/native/workforce/vectors/applied.hex")) }
pub(super) fn context() -> Result<OperationContext> {
    let c = capture()?; let fortress = c.fortress_id();
    Ok(OperationContext { session_id: SessionId::new(1), request_id: RequestId::new(1),
        anchor: StateAnchor { fortress_id: fortress, cursor: ObservationCursor::ORIGIN, tick: GameTick(c.tick()), state_hash: c.witness() },
        budget: WorkBudget { max_wall_millis: 10_000, max_bytes: 128 * 1024 * 1024, max_entities: 8192,
            max_output_tokens: 65_536, max_actions: 1, max_game_ticks: 0 },
        grants: [Capability::Query, Capability::Plan, Capability::ConfigureLabor].into_iter().map(|capability|
            CapabilityGrant { capability, scope: CapabilityScope { fortress_id: Some(fortress), ..CapabilityScope::default() },
                max_risk: RiskTier::Guarded, expires_at_tick: None, remaining_uses: None }).collect(), cancellation_requested: false })
}
pub(super) fn record(plan: &AssignmentPlan, phase: AssignmentPhase, after: Option<&WorkforceCapture>) -> Result<Vec<u8>> {
    let mut out = b"DFMWE017".to_vec(); put_text(&mut out, plan.key());
    out.extend_from_slice(plan.digest().as_bytes()); out.extend_from_slice(plan.token()); plan.spec().append(&mut out);
    out.extend_from_slice(plan.before().witness().as_bytes());
    for n in [plan.before().generation(), plan.before().sequence(), plan.before().tick()] { out.extend_from_slice(&n.to_be_bytes()); }
    out.push(phase as u8); out.extend_from_slice(after.map_or(Digest32::ZERO, WorkforceCapture::witness).as_bytes());
    out.extend_from_slice(&(plan.before().labor_keys().len() as u16).to_be_bytes());
    out.extend_from_slice(&(after.map_or(0, |a| a.citizens().len()) as u16).to_be_bytes());
    if let Some(after) = after { for unit in after.citizens() { unit.append(&mut out); } }
    let receipt = hash(b"dfmcp-workforce-receipt/1", &out); out.extend_from_slice(receipt.as_bytes()); Ok(out)
}
#[test]
fn real_cpp_capture_plan_and_applied_readback() -> Result<()> {
    let capture = capture()?; assert_eq!(capture.encode_values()?, capture.canonical_bytes());
    let plan = plan()?; assert_eq!(AssignmentPlan::decode(&plan.canonical_bytes())?, plan);
    assert_eq!(plan.changed_ids()?, [2]);
    let result = AssignmentEffect::decode(&applied()?, &plan)?;
    assert_eq!(result.phase(), AssignmentPhase::Applied);
    assert_eq!(result.post_citizens()[0].labors(), [1, 1, 1]);
    assert_eq!(result.post_citizens()[1].labors(), [1, 0, 1]); Ok(())
}
#[test]
fn corrupt_or_incomplete_effects_never_prove_application() -> Result<()> {
    let plan = plan()?; let raw = applied()?;
    for n in 0..raw.len() { assert!(AssignmentEffect::decode(&raw[..n], &plan).is_err()); }
    for n in 0..raw.len() { let mut bad = raw.clone(); bad[n] ^= 1; assert!(AssignmentEffect::decode(&bad, &plan).is_err()); }
    let mut extra = raw; extra.push(0); assert!(AssignmentEffect::decode(&extra, &plan).is_err()); Ok(())
}
#[test]
fn rehashed_false_poststate_and_unchanged_citizen_writes_are_rejected() -> Result<()> {
    let plan = plan()?;
    let (mut after, _) = plan.before.expected(plan.spec)?;
    // Newly assigned citizen did not acquire MINING, even with an intact checksum.
    let raw = record(&plan, AssignmentPhase::Applied, Some(&after))?;
    assert!(AssignmentEffect::decode(&raw, &plan).is_err());
    after.citizens[0].labors[0] = 1; after.bytes = after.encode_values()?;
    assert!(AssignmentEffect::decode(&record(&plan, AssignmentPhase::Applied, Some(&after))?, &plan).is_ok());
    after.citizens[1].labors[2] = 0; after.bytes = after.encode_values()?;
    assert!(AssignmentEffect::decode(&record(&plan, AssignmentPhase::Applied, Some(&after))?, &plan).is_err()); Ok(())
}
#[test]
fn removal_does_not_fabricate_exclusive_labor_denial() -> Result<()> {
    let plan = AssignmentPlan::new("remove", AssignmentSpec::new(0, false)?, capture()?)?;
    let (after, changed) = plan.before.expected(plan.spec)?; assert_eq!(changed, [5]);
    let value = AssignmentEffect::decode(&record(&plan, AssignmentPhase::Applied, Some(&after))?, &plan)?;
    assert_eq!(value.post_citizens()[1].labors()[0], 1); Ok(())
}
#[test]
fn exact_preconditions_and_all_membership_subsets() -> Result<()> {
    for mask in 0..256u32 {
        let mut c = capture()?;
        c.citizens = (0..8).map(|id| Citizen { id, historical_id: id + 100, eligible: true, labors: vec![0, 0, 0] }).collect();
        c.details[0].members = (0..8).filter(|id| mask & (1 << id) != 0).collect();
        c.bytes = c.encode_values()?;
        for assigned in [false, true] {
            let planned = AssignmentPlan::new("subset", AssignmentSpec::new(0, assigned)?, c.clone());
            let noop = if assigned { mask == 255 } else { mask == 0 };
            assert_eq!(planned.is_err(), noop);
            if let Ok(plan) = planned {
                let (after, changed) = plan.before.expected(plan.spec)?;
                assert_eq!(changed.len(), if assigned { 8 - mask.count_ones() as usize } else { mask.count_ones() as usize });
                assert_eq!(after.details[0].members.len(), if assigned { 8 } else { 0 });
            }
        }
    }
    for field in 0..5 {
        let mut c = capture()?;
        match field { 0 => c.paused = false, 1 => c.automatic = false, 2 => c.sequence = u64::MAX,
            3 => c.citizens[0].eligible = false, _ => c.details[0].selected_only = false }
        c.bytes = c.encode_values()?;
        assert!(AssignmentPlan::new("refuse", AssignmentSpec::new(0, true)?, c).is_err());
    }
    Ok(())
}
#[test]
fn unknown_and_terminal_evidence_cannot_be_promoted_or_rewritten() -> Result<()> {
    let p = plan()?; let unknown = AssignmentEffect::decode(&record(&p, AssignmentPhase::Unknown, None)?, &p)?;
    let applied = AssignmentEffect::decode(&applied()?, &p)?;
    assert!(applied.follows(&unknown).is_err()); assert!(unknown.follows(&applied).is_err());
    unknown.follows(&unknown)?; applied.follows(&applied)?; Ok(())
}
#[test]
fn wrong_fortress_expired_limited_or_cancelled_grants_fail() -> Result<()> {
    let c = context()?; rpc::authorize(&c, c.anchor.fortress_id, 100, true)?;
    for mode in 0..4 {
        let mut c = c.clone();
        match mode {
            0 => c.anchor.fortress_id = FortressId::new(99),
            1 => { for g in &mut c.grants { g.expires_at_tick = Some(GameTick(99)); } },
            2 => { for g in &mut c.grants { g.remaining_uses = Some(1); } },
            _ => c.cancellation_requested = true,
        }
        assert!(rpc::authorize(&c, capture()?.fortress_id(), 100, true).is_err());
    }
    Ok(())
}
