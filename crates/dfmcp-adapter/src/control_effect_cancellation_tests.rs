//! Cancellation is a durable coordinator stop, not a native outcome receipt.
use super::*;
use std::io::Cursor;
use dfmcp_core::{CapabilityGrant, CapabilityScope, FortressId, GameTick,
    ObservationCursor, RequestId, SessionId, StateAnchor, WorkBudget};

#[derive(Default)]
struct Memory {
    bytes: Cursor<Vec<u8>>,
    writes: usize,
    syncs: usize,
    sync_fails: bool,
    remaining_write_bytes: Option<usize>,
}
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> { self.bytes.read(out) }
}
impl Seek for Memory {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> { self.bytes.seek(from) }
}
impl Write for Memory {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        self.writes += 1;
        let length = match self.remaining_write_bytes.as_mut() {
            Some(0) => return Err(io::Error::other("injected partial write")),
            Some(left) => { let length = input.len().min(*left); *left -= length; length }
            None => input.len(),
        };
        self.bytes.write(&input[..length])
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        self.syncs += 1;
        if self.sync_fails { Err(io::Error::other("injected sync failure")) } else { Ok(()) }
    }
    fn truncate(&mut self, length: u64) -> io::Result<()> {
        self.bytes.get_mut().truncate(length as usize); Ok(())
    }
}
fn context() -> OperationContext {
    OperationContext { session_id:SessionId::new(81), request_id:RequestId::new(1),
        anchor:StateAnchor { fortress_id:FortressId::new(1), cursor:ObservationCursor::ORIGIN,
            tick:GameTick(10), state_hash:Digest32::ZERO }, budget:WorkBudget::default(),
        cancellation_requested:false, grants:[Capability::ControlClock, Capability::Query]
            .into_iter().map(|capability|CapabilityGrant { capability,
                scope:CapabilityScope::default(), max_risk:RiskTier::Reversible,
                expires_at_tick:None, remaining_uses:None }).collect() }
}
fn plan() -> Digest32 { Digest32::of_bytes(b"cancel-this-pause-plan") }
fn fixture() -> Result<ControlEffectJournal<Memory>> {
    let mut j = ControlEffectJournal::open(Memory::default(), &context(), true, 7, EffectTailRecovery::Refuse)?;
    j.record_prepared("key".into(), plan(), true, 10, 7, [1;16], &context())?;
    Ok(j)
}
fn reopen(bytes: Vec<u8>) -> Result<ControlEffectJournal<Memory>> {
    ControlEffectJournal::open(Memory { bytes:Cursor::new(bytes), ..Memory::default() },
        &context(), false, 7, EffectTailRecovery::Refuse)
}
fn current(j: &ControlEffectJournal<Memory>) -> Result<DurablePauseRecord> {
    j.lookup("key").cloned().ok_or_else(||invalid("fixture key absent"))
}

#[test]
fn cancelled_record_survives_replay_without_inventing_a_native_outcome() -> Result<()> {
    let mut j = fixture()?; let prepared = current(&j)?;
    let cancelled = j.cancel_prepared("key", plan(), &context())?;
    assert_eq!(cancelled.state, DurablePauseState::CancelledBeforeDispatch);
    assert!(cancelled.state.terminal()); assert!(!cancelled.state.reconciliation_required());
    assert!(!cancelled.safe_to_dispatch(7));
    assert_eq!((cancelled.plan_digest, cancelled.prepare_token, cancelled.expected_game_tick),
        (prepared.plan_digest, prepared.prepare_token, prepared.expected_game_tick));
    assert!(!cancelled.effect_known); assert!(!cancelled.effect_applied);
    assert_eq!((cancelled.observed_paused, cancelled.observed_game_tick, cancelled.receipt_digest), (None,None,None));
    assert_eq!((cancelled.revision, cancelled.transition_number), (2,2));
    assert_eq!(cancelled.previous_digest, prepared.record_digest);
    let reopened = reopen(j.storage.bytes.into_inner())?;
    assert_eq!(current(&reopened)?, cancelled);
    Ok(())
}

#[test]
fn repeated_cancellation_is_exact_and_does_not_rewind_a_later_journal_head() -> Result<()> {
    let mut j = fixture()?; let cancelled = j.cancel_prepared("key",plan(),&context())?;
    j.record_prepared("other".into(),Digest32::of_bytes(b"other"),false,11,7,[2;16],&context())?;
    let before = (j.head(),j.transition_count(),j.retained_bytes(),j.storage.writes,j.storage.syncs);
    let result = j.cancel_prepared_with("key",plan(),&context(),|record,bytes,replayed| {
        assert!(replayed); assert_eq!(record,&cancelled); assert_eq!(bytes,before.2); Ok(record.clone())
    })?;
    assert_eq!(result,cancelled);
    assert_eq!((j.head(),j.transition_count(),j.retained_bytes(),j.storage.writes,j.storage.syncs),before);
    Ok(())
}

#[test]
fn cancellation_cannot_be_reprepared_committed_or_reconciled_into_an_effect() -> Result<()> {
    let mut j = fixture()?; j.cancel_prepared("key",plan(),&context())?;
    let mut j = reopen(j.storage.bytes.into_inner())?;
    let before = j.head();
    assert_eq!(j.record_prepared("key".into(),plan(),true,10,7,[1;16],&context())?.state,
        DurablePauseState::CancelledBeforeDispatch);
    for generation in [7,8] {
        assert!(matches!(j.begin_commit("key",plan(),generation,&context()),Err(e) if e.code==ErrorCode::Conflict));
    }
    assert!(matches!(j.mark_indeterminate("key",plan(),&context()),Err(e) if e.code==ErrorCode::Conflict));
    assert!(matches!(j.record_reconciliation("key",plan(),7,true,true,true,11,
        Some(Digest32::of_bytes(b"foreign receipt")),&context()),Err(e) if e.code==ErrorCode::Conflict));
    assert_eq!(j.head(),before);
    Ok(())
}

#[test]
fn started_indeterminate_and_verified_effects_cannot_be_cancelled() -> Result<()> {
    for variant in 0..4 {
        let mut j=fixture()?; j.begin_commit("key",plan(),7,&context())?;
        if variant==1 { j.mark_indeterminate("key",plan(),&context())?; }
        if variant>=2 { j.record_reconciliation("key",plan(),7,true,variant==2,variant==2,11,
            Some(Digest32::of_bytes(b"receipt")),&context())?; }
        let before=(j.head(),j.storage.bytes.get_ref().clone());
        let result=j.cancel_prepared("key",plan(),&context());
        assert!(matches!(result,Err(e) if e.code==if variant<2 {ErrorCode::EffectIndeterminate} else {ErrorCode::Conflict}));
        assert_eq!((j.head(),j.storage.bytes.get_ref().clone()),before);
    }
    Ok(())
}

#[test]
fn response_refusal_precedes_storage_and_preserves_dispatch_decision() -> Result<()> {
    let mut j=fixture()?; let before=(j.head(),j.storage.writes,j.storage.syncs,j.retained_bytes());
    let result:Result<()> = j.cancel_prepared_with("key",plan(),&context(),|record,length,replayed| {
        assert!(!replayed); assert_ne!(record.record_digest,Digest32::ZERO);
        assert_eq!(record.state,DurablePauseState::CancelledBeforeDispatch);
        assert!(length>before.3); Err(exhausted("injected full response rejection"))
    });
    assert!(matches!(result,Err(e) if e.code==ErrorCode::BudgetExceeded));
    assert_eq!((j.head(),j.storage.writes,j.storage.syncs,j.retained_bytes()),before);
    assert!(current(&j)?.safe_to_dispatch(7)); assert!(!j.fenced());
    j.cancel_prepared("key",plan(),&context())?;
    Ok(())
}

#[test]
fn uncertain_sync_does_not_acknowledge_but_complete_replay_retains_cancellation() -> Result<()> {
    let mut j=fixture()?; j.storage.sync_fails=true;
    assert!(j.cancel_prepared("key",plan(),&context()).is_err()); assert!(j.fenced());
    assert_eq!(current(&j)?.state,DurablePauseState::Prepared);
    assert!(j.begin_commit("key",plan(),7,&context()).is_err());
    let mut recovered=reopen(j.storage.bytes.into_inner())?;
    assert_eq!(current(&recovered)?.state,DurablePauseState::CancelledBeforeDispatch);
    assert!(matches!(recovered.begin_commit("key",plan(),7,&context()),Err(e) if e.code==ErrorCode::Conflict));
    Ok(())
}

#[test]
fn partial_cancellation_write_fences_and_recovery_never_silently_drops_it() -> Result<()> {
    let mut j=fixture()?; j.storage.remaining_write_bytes=Some(17);
    assert!(j.cancel_prepared("key",plan(),&context()).is_err()); assert!(j.fenced());
    assert!(j.begin_commit("key",plan(),7,&context()).is_err());
    assert!(matches!(reopen(j.storage.bytes.into_inner()),Err(e) if e.code==ErrorCode::CorruptLedger));
    Ok(())
}

#[test]
fn cancellation_checks_identity_authority_expiry_and_read_only_custody() -> Result<()> {
    let mut j=fixture()?; let before=j.head();
    assert!(matches!(j.cancel_prepared("key",Digest32::ZERO,&context()),Err(e) if e.code==ErrorCode::Conflict));
    assert!(j.cancel_prepared("missing",plan(),&context()).is_err());
    for variant in 0..4 {
        let mut c=context();
        match variant {
            0=>c.grants.clear(), 1=>c.cancellation_requested=true,
            2=>for grant in &mut c.grants { grant.expires_at_tick=Some(GameTick(9)); },
            _=>for grant in &mut c.grants { grant.remaining_uses=Some(0); },
        }
        assert!(j.cancel_prepared("key",plan(),&c).is_err());
    }
    assert_eq!(j.head(),before);
    let mut read_only=ControlEffectJournal::open_read_only(
        Memory {bytes:Cursor::new(j.storage.bytes.into_inner()),..Memory::default()},&context())?;
    assert!(matches!(read_only.cancel_prepared("key",plan(),&context()),Err(e) if e.code==ErrorCode::CapabilityDenied));
    assert_eq!(read_only.storage.writes,0);
    Ok(())
}

#[test]
fn replay_rejects_cancel_after_dispatch_and_cancel_with_outcome_evidence() -> Result<()> {
    for variant in 0..7 {
        let mut j=fixture()?;
        if variant==0 { j.begin_commit("key",plan(),7,&context())?; }
        if variant==1 { j.cancel_prepared("key",plan(),&context())?; }
        let mut next=current(&j)?; next.state=DurablePauseState::CancelledBeforeDispatch;
        next.revision+=1; next.transition_number=j.transition_count() as u64+1;next.previous_digest=j.head();
        match variant {
            2=>next.effect_known=true,3=>next.effect_applied=true,4=>next.observed_paused=Some(false),
            5=>next.observed_game_tick=Some(11),6=>next.receipt_digest=Some(Digest32::of_bytes(b"receipt")),_=>{}
        }
        let frame=encode_frame(j.id(),&next)?;j.storage.bytes.get_mut().extend_from_slice(&frame);
        assert!(matches!(reopen(j.storage.bytes.into_inner()),Err(e) if e.code==ErrorCode::CorruptLedger),"variant={variant}");
    }
    Ok(())
}

#[test]
fn cached_cancellation_and_existing_mutator_replays_recheck_changed_storage() -> Result<()> {
    for variant in 0..5 {
        let mut j=fixture()?;
        match variant {
            0=>{j.cancel_prepared("key",plan(),&context())?;}
            1=>{}
            _=>{j.begin_commit("key",plan(),7,&context())?;
                if variant==2 {j.mark_indeterminate("key",plan(),&context())?;}
                else {j.record_reconciliation("key",plan(),7,true,true,true,11,Some(plan()),&context())?;}}
        }
        j.storage.bytes.get_mut().push(0);
        let result=match variant {
            0=>j.cancel_prepared("key",plan(),&context()),
            1=>j.record_prepared("key".into(),plan(),true,10,7,[1;16],&context()),
            2=>j.mark_indeterminate("key",plan(),&context()),
            3=>j.begin_commit("key",plan(),7,&context()),
            _=>j.record_reconciliation("key",plan(),7,true,true,true,11,Some(plan()),&context()),
        };
        assert!(matches!(result,Err(e) if e.code==ErrorCode::CorruptLedger));assert!(j.fenced());
    }
    Ok(())
}
