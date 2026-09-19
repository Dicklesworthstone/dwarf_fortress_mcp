//! Session lifecycle boundaries. Release itself performs no observation,
//! journal verification/repair, checkpoint, cancellation or game action.
//! The separate source-gap boundary requires read authority and may checkpoint
//! interrupted monitoring. Lock order remains WATCHES -> JOURNALS.
use super::*;

#[path = "watch_source_gap.rs"]
mod source_gap;

impl WatchJournalGuard {
    /// Called only after the runtime has exclusively acquired the session being
    /// closed. A release does not need renewed game-data authority: the callback
    /// receives ownership counts, never definitions, observations or evidence.
    /// The full close acknowledgement must render before either registry changes.
    pub(crate) fn release_session<F>(session: SessionId, discard_process_local: bool,
        publish: F) -> Result<String>
    where F: FnOnce(Value) -> Result<String> {
        // Poisoned registries remain poisoned for ordinary operations. Teardown
        // can still discard this session's safely owned allocations; it never
        // treats their possibly incomplete contents as verified world evidence.
        let (mut watches, watch_poisoned) = match WATCHES.lock() {
            Ok(guard) => (guard, false),
            Err(poison) => (poison.into_inner(), true),
        };
        let (mut registry, journal_poisoned) = match JOURNALS.lock() {
            Ok(guard) => (guard, false),
            Err(poison) => (poison.into_inner(), true),
        };
        let count = watches.entries.keys().filter(|(id, _)| *id == session).count();
        let durable = registry.contains_key(&session);
        if count > 0 && !durable && !discard_process_local {
            return Err(failure(ErrorCode::Conflict,
                "closing would discard process-local watches; explicitly set discard_process_local_work=true"));
        }
        let out = publish(json!({
            "watch_handles_released": count,
            "process_local_watch_records_discarded": if durable { 0 } else { count },
            "watch_journal_present": durable,
            "watch_journal_modified": false,
            "watch_evidence_revalidated": false,
            "durable_watch_recovery_required": durable,
            "watch_registry_poisoned": watch_poisoned || journal_poisoned,
            "game_effects_cancelled": false
        }))?;
        // No fallible work follows acknowledgement preparation. Removing Entry
        // only drops the owned descriptor; it does not append a checkpoint.
        watches.entries.retain(|(id, _), _| *id != session);
        registry.remove(&session);
        Ok(out)
    }
}
