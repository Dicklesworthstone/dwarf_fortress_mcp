//! Explicit positive-proof recovery, separate from ordinary receipt replay.
//!
//! Beads: df-dfhack-bridge-plane-c-pic.4/.5. The original uncertain frame stays
//! in the append-only chain. No writer, cancellation or dispatch permit occurs.
use super::*;

impl BuildEntry {
    /// A retained attempted effect can acquire exact original positive proof.
    /// This does not change ordinary commit/recovery replay into native work.
    pub fn can_reconcile_receipt(&self) -> bool {
        self.state == BuildState::Terminal
            && self
                .native
                .as_ref()
                .is_some_and(|record| record.phase() == BuildPhase::Indeterminate)
    }
}

/// Replay the only permitted post-receipt transition. Ordinary native successor
/// validation remains immutable; this exception belongs to explicit custody.
pub(super) fn validate_transition(old: &BuildEntry, next: &BuildEntry) -> Result<()> {
    check(
        old.can_reconcile_receipt()
            && next.state == BuildState::Terminal
            && old.plan == next.plan
            && old.dispatch_started == next.dispatch_started
            && old.cancel_requested == next.cancel_requested,
    )?;
    let previous = old.native.as_ref().ok_or_else(corrupt)?;
    let next = next.native.as_ref().ok_or_else(corrupt)?;
    // Decode both complete records, not just hashes or typed phase labels. The
    // Placed decoder requires exact expected-after and building/job/item links.
    let previous = BuildRecord::decode(previous.canonical_bytes())?;
    let proved = BuildRecord::decode(next.canonical_bytes())?;
    previous.verify_plan(&old.plan)?;
    proved.verify_plan(&old.plan)?;
    check(
        previous.phase() == BuildPhase::Indeterminate && proved.phase() == BuildPhase::Placed,
    )
}

impl<S: EffectJournalStorage> BuildJournal<S> {
    /// Query an already-retained Indeterminate receipt on its exact original
    /// source. Only complete Placed evidence may append one final proof frame.
    /// Missing/repeated evidence preserves history; every exit retires the
    /// supplied connection. Reopen and imported receipts never grant dispatch.
    pub fn reconcile_receipt<N: BuildSource, G: BuildGuard>(
        &mut self,
        source: &mut N,
        key: &str,
        digest: Digest32,
        context: &OperationContext,
        guard: &mut G,
    ) -> Result<BuildEntry> {
        self.abandon();
        let result = (|| {
            let mut work = self.begin(context)?;
            let old = self.known(key, digest)?;
            if !old.unresolved() {
                return Ok(old);
            }
            check(old.can_reconcile_receipt())?;
            self.online(false)?;
            // Legacy journals may not have reserved a recovery frame. Refuse
            // before query can update native evidence if custody cannot hold it.
            if self.frames >= MAX_FRAMES
                || self.raw.len() > MAX_JOURNAL_BYTES - MAX_FRAME_BYTES
            {
                return Err(exhausted());
            }
            self.binding.source_matches(source.binding(), true)?;
            let current = self.edge(source, BuildStage::Query, &old.plan, &mut work, guard)?;
            self.binding.source_matches(source.binding(), true)?;
            let native = source.query(&old.plan, &current, work.remaining()?)?;
            self.after(source, BuildStage::Query, &old.plan, &mut work, guard)?;
            self.binding.source_matches(source.binding(), true)?;
            let native = native.ok_or_else(|| uncertain(key))?;
            native.verify_plan(&old.plan)?;
            if old.native.as_ref() == Some(&native) {
                return Ok(old); // Repeated uncertainty consumes no journal frame.
            }
            let mut next = old.clone();
            next.native = Some(native);
            validate_transition(&old, &next)?;
            // Existing retain validates the entire transition, appends, flushes,
            // synchronizes and verifies readback before publishing a new root.
            self.retain(next, &mut work)
        })();
        source.fence();
        result
    }
}

#[cfg(test)]
#[path = "receipt_reconciliation_tests.rs"]
mod tests;
