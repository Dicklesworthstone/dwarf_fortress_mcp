//! The coordinator can stop its own prepared effect, not undo a dispatched one.
use super::*;

impl<S: EffectJournalStorage> ControlEffectJournal<S> {
    /// Permanently prevent this journal from dispatching a prepared key. The
    /// cancellation retains the immutable effect identity and is not a native
    /// VerifiedNotApplied receipt. A started/indeterminate attempt is refused.
    pub fn cancel_prepared(&mut self, key: &str, plan_digest: Digest32,
        context: &OperationContext) -> Result<DurablePauseRecord> {
        self.cancel_prepared_with(key, plan_digest, context, |record, _, _| Ok(record.clone()))
    }

    /// Render a complete acknowledgement before writing cancellation evidence.
    /// `publish` receives the exact prospective record, total journal byte count
    /// after this operation, and whether this is an idempotent replay. Its result
    /// is returned only after successful sync and in-memory publication. It must
    /// be a pure renderer: do not transmit or otherwise publish from the callback.
    ///
    /// On replay the record may precede newer unrelated journal transitions, so
    /// its transition_number/digest must not replace the journal's current head.
    /// A write/sync failure fences the journal; the rendered value is discarded.
    pub fn cancel_prepared_with<T, F>(&mut self, key: &str, plan_digest: Digest32,
        context: &OperationContext, publish: F) -> Result<T>
    where F: FnOnce(&DurablePauseRecord, u64, bool) -> Result<T> {
        self.authorize_write(context)?;
        self.ensure_healthy(context)?;
        let current = self.require(key, plan_digest)?;
        match current.state {
            DurablePauseState::Prepared => {
                let mut next = current;
                next.state = DurablePauseState::CancelledBeforeDispatch;
                self.append_with(next, context, |record, length| publish(record, length, false))
                    .map(|(_, rendered)| rendered)
            }
            DurablePauseState::CancelledBeforeDispatch => publish(&current, self.length, true),
            DurablePauseState::CommitStarted | DurablePauseState::Indeterminate => {
                Err(DfmcpError::new(ErrorCode::EffectIndeterminate,
                    "pause commit already started; cancellation cannot erase uncertainty or make the effect safe to retry"))
            }
            DurablePauseState::VerifiedApplied | DurablePauseState::VerifiedNotApplied => {
                Err(conflict("a verified native outcome cannot be replaced by pre-dispatch cancellation"))
            }
        }
    }
}
