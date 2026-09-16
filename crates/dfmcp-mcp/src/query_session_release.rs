//! Transactional teardown of a session's derived query state. The outer runtime
//! serializes this against that session's calls and owns the connection/files.
//! Lock order matches baseline publication: HISTORY -> WATCHES -> JOURNALS.
use super::*;

pub(in super::super) fn release<F>(session: SessionId, discard_process_local: bool,
    publish: F) -> Result<String>
where F: FnOnce(Value) -> Result<String> {
    let (mut history, poisoned) = match HISTORY.lock() {
        Ok(guard) => (guard, false),
        Err(poison) => (poison.into_inner(), true),
    };
    let count = history.entries.keys().filter(|(id, _)| *id == session).count();
    if count > 0 && !discard_process_local {
        return Err(failure(ErrorCode::Conflict,
            "closing would discard process-local baselines; explicitly set discard_process_local_work=true"));
    }
    let out = super::super::WatchJournalGuard::release_session(session, discard_process_local, |mut value| {
        value["process_local_baselines_discarded"] = json!(count);
        value["baseline_registry_poisoned"] = json!(poisoned);
        publish(value)
    })?;
    history.entries.retain(|(id, _), _| *id != session);
    Ok(out)
}
