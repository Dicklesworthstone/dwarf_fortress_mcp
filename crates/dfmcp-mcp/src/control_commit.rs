//! MCP presentation and the fixed native transport for the one-shot coordinator.
use super::*;
use std::time::Instant;
use dfmcp_adapter::pause_reconciliation::{PauseCommitOutcome,PauseCommitSource,commit_once};

struct Connection<'a>(&'a mut Option<ControlConnection>);
impl PauseCommitSource for Connection<'_> {
    fn preflight(&mut self,remaining:Duration,context:&OperationContext)->Result<u64> {
        context.authorize(Capability::ControlClock,RiskTier::Reversible,&[],None)?;
        let connection=self.0.as_mut().ok_or_else(||err(ErrorCode::CapabilityDenied,"no live control connection"))?;
        if connection.client.poisoned() {
            return Err(err(ErrorCode::AdapterUnavailable,"control source fenced before dispatch; reopen for recovery"));
        }
        connection.client.reset_deadline(remaining)?;
        Ok(connection.client.bridge_generation())
    }
    fn commit_prepared(&mut self,record:&DurablePauseRecord,remaining:Duration,
        context:&OperationContext)->Result<PauseEffect> {
        context.authorize(Capability::ControlClock,RiskTier::Reversible,&[],None)?;
        let connection=self.0.as_mut().ok_or_else(||err(ErrorCode::CapabilityDenied,"no live control connection"))?;
        // Remaining time includes the commit-intent sync. Do not grant another
        // complete timeout after durable work or reconnect on a mutation path.
        connection.client.reset_deadline(remaining)?;
        connection.client.commit_pause(&record.idempotency_key,record.plan_digest,&record.prepare_token)
    }
    fn fence(&mut self) {
        if let Some(connection)=self.0.as_mut() {connection.client.fence();}
    }
}

fn payload(outcome:&PauseCommitOutcome,journal:Value)->Value {
    let record=&outcome.record;
    json!({"ok":record.effect_applied,"state":state_name(record.state),"effect":record_json(record),
        "replayed_terminal":outcome.replayed_terminal,"mutation_dispatched":outcome.dispatch_attempted,
        "reconciliation_required":record.state.reconciliation_required(),
        "current_freshness_proven":false,"safe_to_retry_same_effect":false,"durable_effect_journal":journal})
}
fn reserve(record:&DurablePauseRecord,mut journal:Value,maximum:u64)->Result<()> {
    let mut record=record.clone();
    // All immutable strings and preparation fields retain their exact widths.
    // Mutable numbers, nullables and Boolean spellings use worst-case widths.
    record.state=DurablePauseState::VerifiedNotApplied;
    record.effect_known=false;record.effect_applied=false;
    record.observed_paused=Some(false);record.observed_game_tick=Some(u64::MAX);
    record.receipt_digest=Some(Digest32::ZERO);record.revision=u64::MAX;record.transition_number=u64::MAX;
    for field in ["effects","transitions","retained_bytes","repaired_tail_bytes"] {journal[field]=json!(u64::MAX);}
    journal["fenced"]=json!(false);
    let outcome=PauseCommitOutcome {record,dispatch_attempted:false,replayed_terminal:false};
    if packet("fortress.commit",payload(&outcome,journal),false).len() as u64>maximum {
        return Err(err(ErrorCode::BudgetExceeded,"complete pause outcome cannot fit the response budget; no commit was started"));
    }
    Ok(())
}

pub(super) fn execute(session:&mut ControlSession,context:&OperationContext,key:&str,
    plan:Digest32,token:&[u8;16])->Result<Value> {
    let started=Instant::now();
    context.authorize(Capability::ControlClock,RiskTier::Reversible,&[],None)?;
    session.writable()?;
    let record=session.journal.lookup(key).cloned()
        .ok_or_else(||err(ErrorCode::InvalidRequest,"effect must be durably prepared before commit"))?;
    if record.plan_digest!=plan||&record.prepare_token!=token {
        return Err(err(ErrorCode::Conflict,"commit differs from the durable preparation"));
    }
    if record.state==DurablePauseState::CancelledBeforeDispatch {
        return Err(err(ErrorCode::Conflict,"cancelled pause key remains permanently retired"));
    }
    if record.state.reconciliation_required() {
        return Err(err(ErrorCode::EffectIndeterminate,"existing commit attempt requires reconciliation, never redispatch"));
    }
    let maximum=context.budget.max_bytes.min(u64::from(context.budget.max_output_tokens)*4);
    if record.state.terminal() {
        let replay=PauseCommitOutcome {record:record.clone(),dispatch_attempted:false,replayed_terminal:true};
        if packet("fortress.commit",payload(&replay,journal_json(&session.journal)),false).len() as u64>maximum {
            return Err(err(ErrorCode::BudgetExceeded,"retained pause outcome does not fit the response budget"));
        }
    }else{
        reserve(&record,journal_json(&session.journal),maximum)?;
    }
    let mut narrowed=context.clone();
    narrowed.budget.max_wall_millis=u64::try_from(Duration::from_millis(context.budget.max_wall_millis)
        .checked_sub(started.elapsed()).filter(|d|*d>=Duration::from_millis(1))
        .ok_or_else(||err(ErrorCode::BudgetExceeded,"pause preflight exhausted its deadline before dispatch"))?.as_millis())
        .map_err(|_|err(ErrorCode::BudgetExceeded,"pause deadline overflow"))?;
    let mut source=Connection(&mut session.connection);
    let outcome=commit_once(&mut session.journal,&mut source,key,plan,token,&narrowed)?;
    let out=payload(&outcome,journal_json(&session.journal));
    if packet("fortress.commit",out.clone(),false).len() as u64>maximum {
        return Err(err(ErrorCode::EffectIndeterminate,
            "commit response exceeded its reservation; inspect durable evidence before any further action"));
    }
    Ok(out)
}

#[cfg(all(test,unix))]
#[path = "control_commit_tests.rs"]
mod tests;
