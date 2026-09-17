//! One-shot pause execution over the durable coordinator. Preparation and
//! reconciliation are separate protocols; this shell never retries a mutation.
use super::*;
use crate::control_effect_journal::DurablePauseState;

/// The runtime supplies its already-authenticated connection. Preflight must
/// inspect only local readiness/generation and configure its deadline: it must
/// not reconnect, prepare, query or dispatch. commit_prepared sends at most once.
/// A failed or contradictory wire result must make the connection unusable.
pub trait PauseCommitSource {
    fn preflight(&mut self, remaining: Duration, context: &OperationContext) -> Result<u64>;
    fn commit_prepared(&mut self, record: &DurablePauseRecord, remaining: Duration,
        context: &OperationContext) -> Result<PauseEffect>;
    fn fence(&mut self);
}

#[derive(Clone, Debug)]
pub struct PauseCommitOutcome {
    pub record: DurablePauseRecord,
    /// A source commit call was entered. This does not prove native application.
    pub dispatch_attempted: bool,
    pub replayed_terminal: bool,
}

fn remaining(context: &OperationContext, elapsed: Duration) -> Result<Duration> {
    context.authorize(Capability::ControlClock, RiskTier::Reversible, &[], None)?;
    Duration::from_millis(context.budget.max_wall_millis).checked_sub(elapsed)
        .filter(|value| *value >= Duration::from_millis(1))
        .ok_or_else(||DfmcpError::new(ErrorCode::BudgetExceeded,"pause commit deadline exhausted"))
}
fn ambiguous<S: EffectJournalStorage>(journal: &mut ControlEffectJournal<S>,
    record: &DurablePauseRecord, context: &OperationContext, message: &str) -> DfmcpError {
    // Failed sync may already have written a frame. Never acknowledge a new state
    // from this best-effort annotation; CommitStarted itself is recoverable.
    let _=journal.mark_indeterminate(&record.idempotency_key,record.plan_digest,context);
    DfmcpError::new(ErrorCode::EffectIndeterminate,message)
}

/// Require durable intent before one source invocation and verified, synced
/// terminal evidence before success. An identical retained native outcome may be
/// returned without a connection or an action allowance; cancellation is never a
/// native outcome. Callers must reserve their complete response BEFORE this call.
pub fn commit_once<S: EffectJournalStorage, Q: PauseCommitSource>(
    journal: &mut ControlEffectJournal<S>, source: &mut Q, key: &str,
    plan: Digest32, token: &[u8;16], context: &OperationContext) -> Result<PauseCommitOutcome> {
    let started=Instant::now();
    commit_with_clock(journal,source,key,plan,token,context,||started.elapsed())
}

fn commit_with_clock<S: EffectJournalStorage, Q: PauseCommitSource>(
    journal: &mut ControlEffectJournal<S>, source: &mut Q, key: &str,
    plan: Digest32, token: &[u8;16], context: &OperationContext,
    mut elapsed: impl FnMut()->Duration) -> Result<PauseCommitOutcome> {
    context.authorize(Capability::ControlClock,RiskTier::Reversible,&[],None)?;
    if journal.read_only() {
        return Err(DfmcpError::new(ErrorCode::CapabilityDenied,"offline evidence recovery cannot commit effects"));
    }
    if key.is_empty() || key.len()>512 || key.chars().any(char::is_control) {
        return Err(DfmcpError::new(ErrorCode::InvalidRequest,"invalid pause idempotency key"));
    }
    let _=journal.records(context)?;
    let record=journal.lookup(key).cloned().ok_or_else(||
        DfmcpError::new(ErrorCode::InvalidRequest,"pause effect must be durably prepared before commit"))?;
    if record.plan_digest!=plan || &record.prepare_token!=token {
        return Err(DfmcpError::new(ErrorCode::Conflict,"commit differs from the durable preparation"));
    }
    if record.state==DurablePauseState::CancelledBeforeDispatch {
        return Err(DfmcpError::new(ErrorCode::Conflict,"cancelled pause key remains permanently retired"));
    }
    if record.state.terminal() {
        return Ok(PauseCommitOutcome {record,dispatch_attempted:false,replayed_terminal:true});
    }
    if record.state.reconciliation_required() {
        return Err(DfmcpError::new(ErrorCode::EffectIndeterminate,
            "a previous commit attempt is unresolved; reconcile without retrying the mutation"));
    }
    if context.budget.max_actions==0 {
        return Err(DfmcpError::new(ErrorCode::BudgetExceeded,"pause commit requires one action in the request budget"));
    }
    let generation=source.preflight(remaining(context,elapsed())?,context)?;
    if !record.safe_to_dispatch(generation) {
        return Err(DfmcpError::new(ErrorCode::EffectIndeterminate,
            "preparation belongs to another bridge generation; no dispatch was attempted"));
    }
    let _=remaining(context,elapsed())?;
    // A failed durability boundary never calls commit_prepared. Reopening may
    // recover a complete uncertain frame, but this invocation performed no RPC.
    let started=journal.begin_commit(key,plan,generation,context)?;
    let allowance=remaining(context,elapsed()).map_err(|_|ambiguous(journal,&started,context,
        "deadline or authority ended after durable commit intent; no same-effect retry is allowed"))?;
    let effect=match source.commit_prepared(&started,allowance,context) {
        Ok(effect)=>effect,
        Err(_)=>{
            source.fence();
            return Err(ambiguous(journal,&started,context,
                "pause dispatch outcome is ambiguous; recover and reconcile rather than retry"));
        }
    };
    // Persist already-obtained evidence even if wall time expired while the
    // native call completed. Storage is synchronous/cooperative, not preemptible.
    // Current authority is still rechecked by reconcile_reply and the journal.
    match reconcile_reply(journal,key,plan,&effect,context) {
        Ok(record)=>Ok(PauseCommitOutcome {record,dispatch_attempted:true,replayed_terminal:false}),
        Err(_)=>{
            source.fence();
            Err(ambiguous(journal,&started,context,
                "pause reply lacks durably acknowledged verified evidence; reopen and reconcile"))
        }
    }
}

#[cfg(test)]
#[path = "pause_commit_tests.rs"]
mod tests;
