//! Target-bound Run/1.13 checks. The target fixture represents already verified
//! original custody; it does not manufacture a SemanticWorkOrderResult or claim
//! to exercise the combined executor's custody acquisition.
use super::*;
use dfmcp_adapter::bounded_run::RunSpec;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, GameTick, ObservationCursor, RequestId, SessionId,
    StateAnchor, WorkBudget,
};

fn target() -> ClockTarget {
    ClockTarget {
        digest: Digest32::from_bytes([0x11; 32]),
        original_seal: Digest32::from_bytes([0x22; 32]),
        fortress: dfmcp_adapter::workforce_control::fortress_id("region1", 7),
        epoch: 3,
        original_tick: 120,
        deadline: 220,
    }
}

fn capture(tick: u64, loaded: bool, valid: bool, paused: bool) -> Result<RunObservation> {
    let mut bytes = b"DFMRO013".to_vec();
    for value in [41u64, 3, tick] {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    bytes.extend_from_slice(&[u8::from(loaded), u8::from(valid), u8::from(paused)]);
    RunObservation::decode(&bytes)
}

fn plan(target: &ClockTarget, tick: u64, ticks: u32) -> Result<RunPlan> {
    let before = capture(tick, true, true, true)?;
    let spec = RunSpec::new(ticks, 1000)?;
    let candidate = RunPlan::new(ClockTarget::CANDIDATE_KEY, spec, before.clone())?;
    RunPlan::new(&target.key(candidate.digest()), spec, before)
}

fn context(target: &ClockTarget) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: StateAnchor {
            fortress_id: target.fortress,
            cursor: ObservationCursor {
                epoch: target.epoch,
                sequence: 2,
            },
            tick: GameTick(200),
            state_hash: Digest32::from_bytes([0x44; 32]),
        },
        budget: WorkBudget::default(),
        grants: vec![CapabilityGrant {
            capability: Capability::Query,
            scope: CapabilityScope {
                fortress_id: Some(target.fortress),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        }],
        cancellation_requested: false,
    }
}

#[test]
fn inventory_stability_slices_keep_the_original_absolute_deadline() -> Result<()> {
    let target = target();
    for (before, ticks) in [(120, 100), (200, 20), (219, 1)] {
        target.plan(&plan(&target, before, ticks)?)?;
    }
    for (before, ticks) in [(120, 101), (200, 21), (219, 2), (220, 1)] {
        assert_eq!(
            target
                .plan(&plan(&target, before, ticks)?)
                .expect_err("original deadline")
                .code,
            ErrorCode::CorruptLedger,
        );
    }
    assert_eq!(target.original_tick(), 120);
    assert_eq!(target.deadline, 220);
    Ok(())
}

#[test]
fn clock_keys_and_review_seals_preserve_bytes_and_bind_target_and_journal() -> Result<()> {
    let target = target();
    let plan = plan(&target, 200, 20)?;
    let journal = Digest32::from_bytes([0x33; 32]);
    // Frozen pre-extension domains and framing; extending selected-parent
    // targets must not invalidate standalone keys or retained clock reviews.
    assert_eq!(
        plan.key(),
        "inventory-clock-d2b60011e82f28001c5702a0d60d58ec1019b05acd503eb649450e075357ad1d",
    );
    assert_eq!(
        target.seal(journal, &plan).to_hex(),
        "9ad39b2ab17fdb506d0467ee7a63d68624882ac11c99c4c7a0c7b4ffb90d727d",
    );
    let other = ClockTarget {
        digest: Digest32::from_bytes([0x55; 32]),
        ..target.clone()
    };
    assert_ne!(other.key(plan.digest()), plan.key());
    assert!(other.plan(&plan).is_err());
    assert_ne!(other.seal(journal, &plan), target.seal(journal, &plan));
    assert_ne!(
        target.seal(Digest32::from_bytes([0x66; 32]), &plan),
        target.seal(journal, &plan),
    );
    Ok(())
}

#[test]
fn inventory_clock_requires_current_original_authority_and_paused_source() -> Result<()> {
    let target = target();
    let current = context(&target);
    target.context(&current)?;
    // Run/1.13 needs no present manager-order row, allowing a retired original
    // order's separately established inventory objective to finish its streak.
    assert_eq!(target.capture(&capture(200, true, true, true)?)?, 200);
    for value in [
        capture(119, true, true, true)?,
        capture(200, true, true, false)?,
        capture(0, false, false, false)?,
    ] {
        assert_eq!(
            target
                .capture(&value)
                .expect_err("paused current source")
                .code,
            ErrorCode::StaleAnchor
        );
    }
    let mut changed = current.clone();
    changed.anchor.cursor.epoch += 1;
    assert!(target.context(&changed).is_err());
    changed = current.clone();
    changed.anchor.fortress_id = FortressId::NIL;
    assert!(target.context(&changed).is_err());
    changed = current.clone();
    changed.anchor.tick = GameTick(119);
    assert!(target.context(&changed).is_err());
    changed = current;
    changed.grants.clear();
    assert!(target.context(&changed).is_err());
    Ok(())
}
