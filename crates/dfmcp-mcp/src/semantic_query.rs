//! Typed query dispatch over the exact session-owned canonical projection.
//! Stateful foreground reads publish only after the complete Agent Turn fits.

#[path = "semantic_query_core.rs"]
mod core_query;
#[path = "operational_query.rs"]
mod operational_query;
#[path = "query_history.rs"]
mod query_history;
#[path = "query_watch.rs"]
mod query_watch;
#[cfg(test)]
#[path = "query_history_tests.rs"]
mod history_tests;

use dfmcp_core::{Capability, DfmcpError, Digest32, ErrorCode, OperationContext, Result,
    RiskTier, StateAnchor};
use dfmcp_world::WorldSnapshot;
use serde_json::{Value, json};

pub(crate) fn extend_watch_count_schema(schema: Value) -> Result<Value> {
    query_watch::extend_count_schema(schema)
}

/// Ownership of a durable watch registry belongs to the enclosing session.
/// Dropping it releases only process-local ownership, never durable intent.
pub(crate) type WatchJournalGuard = query_watch::WatchJournalGuard;

/// The runtime must exclusively own the resolved session across this call.
/// This release-only path returns no world facts, does not evaluate predicates,
/// and never changes journal bytes. Render failure leaves both registries intact.
pub(crate) fn release_session_resources<F>(session: dfmcp_core::SessionId,
    discard_process_local: bool, publish: F) -> Result<String>
where F: FnOnce(Value) -> Result<String> {
    query_history::release_session(session, discard_process_local, publish)
}

/// Private, single-use preparation: selections cannot be replaced by an MCP
/// handle after a runtime has acquired the one optional observation.
pub(crate) type PreparedWatchBatch = query_watch::batch::Prepared;
pub(crate) fn prepare_watch_batch<F>(snapshot: &WorldSnapshot, context: &OperationContext,
    input: &Value, preview: F) -> Result<PreparedWatchBatch>
where F: FnOnce(Value) -> Result<String> {
    query_watch::batch::prepare(snapshot,context,input,preview)
}
pub(crate) fn complete_watch_batch<F>(snapshot: &WorldSnapshot, context: &OperationContext,
    prepared: PreparedWatchBatch, observation_acquired: bool, publish: F) -> Result<String>
where F: FnOnce(Value) -> Result<String> {
    query_watch::batch::complete(snapshot,context,prepared,observation_acquired,publish)
}

/// The spatial/1.8 caller supplies exact verified archive anchors after syncing
/// its fresh bootstrap capture. No MCP argument can select a journal or profile.
pub(crate) fn attach_watch_journal<F>(snapshot: &WorldSnapshot, context: &OperationContext,
    path: &std::path::Path, archive: Digest32, observations: &[StateAnchor], value: Value, publish: F)
    -> Result<(String, WatchJournalGuard)>
where F: FnOnce(Value) -> Result<String> {
    query_watch::attach_journal(snapshot, context, path, archive, observations, value, publish)
}

/// The caller verifies archive custody and exact record identities. Historical
/// anchors do not replace current authority, and this never creates local work.
pub(super) fn compare_endpoints(before: &WorldSnapshot, after: &WorldSnapshot,
    context: &OperationContext, selection: &Value, binding: Digest32,
    limit: Option<u32>, continuation: Option<&str>) -> Result<Value> {
    query_history::compare_endpoints(before, after, context, selection, binding, limit, continuation)
}

pub fn execute(snapshot: &WorldSnapshot, context: &OperationContext, input: &Value) -> Result<Value> {
    match input.get("query").and_then(|query| query.get("kind")).and_then(Value::as_str) {
        Some("aggregate" | "search") => operational_query::execute(snapshot, context, input),
        _ => core_query::execute(snapshot, context, input),
    }
}

/// Validate an await handle and its pre-refresh anchor without sampling a watch.
/// The bounded metadata query retains the same session/authority checks as polling.
pub fn prepare_await(snapshot: &WorldSnapshot, context: &OperationContext, input: &Value) -> Result<bool> {
    let invalid = || DfmcpError::new(ErrorCode::InvalidRequest,"invalid await_watch envelope or handle");
    let object = input.as_object().ok_or_else(invalid)?;
    if object.len()>3 || object.keys().any(|key| !matches!(key.as_str(),"schema"|"expected_anchor"|"query"))
        || input["schema"]!="dfmcp.query/1" { return Err(invalid()); }
    let request = input["query"].as_object().ok_or_else(invalid)?;
    if request.len()!=2 || input["query"]["kind"]!="await_watch" { return Err(invalid()); }
    let handle = input["query"]["watch"].as_str().ok_or_else(invalid)?;
    let hash = handle.strip_prefix("watch:").ok_or_else(invalid)?;
    if hash.len()!=64 || !hash.bytes().all(|byte|byte.is_ascii_digit()||(b'a'..=b'f').contains(&byte)) {
        return Err(invalid());
    }
    if input.get("expected_anchor").filter(|value|!value.is_null())
        .is_some_and(|value|value!=&core_query::anchor_json(context.anchor)) {
        return Err(DfmcpError::new(ErrorCode::StaleAnchor,"await_watch expected_anchor differs from the pre-refresh anchor"));
    }
    let mut needs_observation = false;
    query_watch::execute(snapshot,context,&json!({"schema":"dfmcp.query/1","query":{"kind":"watches"}}),|metadata| {
        let record = metadata["records"].as_array().and_then(|records|records.iter()
            .find(|record|record["watch"].as_str()==Some(handle))).ok_or_else(invalid)?;
        needs_observation = !record["terminal"].as_bool().ok_or_else(invalid)?;
        Ok(String::new())
    })?;
    Ok(needs_observation)
}

/// Reserve active-work bytes before allowing a stateless result to fill the page.
pub fn result_context(context: &OperationContext) -> Result<OperationContext> {
    context.authorize(Capability::Query,RiskTier::ReadOnly,&[],None)?;
    let mut reserved = 0u64;
    query_watch::with_active_work(context,json!({}),|metadata| {
        reserved = u64::try_from(metadata.to_string().len()).map_err(|_| {
            DfmcpError::new(ErrorCode::BudgetExceeded,"watch projection byte count cannot be represented")
        })?.saturating_add(32);
        Ok(String::new())
    })?;
    let mut narrowed = context.clone();
    narrowed.budget.max_bytes = context.budget.max_bytes
        .min(u64::from(context.budget.max_output_tokens).saturating_mul(4))
        .checked_sub(reserved).filter(|bytes|*bytes>0).ok_or_else(|| {
            DfmcpError::new(ErrorCode::BudgetExceeded,"active watches leave no room for query results")
        })?;
    Ok(narrowed)
}

pub fn publish_with_active_work<F>(context: &OperationContext, value: Value, publish: F) -> Result<String>
where F: FnOnce(Value) -> Result<String> {
    context.authorize(Capability::Query,RiskTier::ReadOnly,&[],None)?;
    query_watch::with_active_work(context,value,publish)
}

/// No watch/baseline mutation is visible before the full response is accepted.
pub fn execute_with_publisher<F>(snapshot: &WorldSnapshot, context: &OperationContext,
    input: &Value, publish: F) -> Result<String>
where F: FnOnce(Value) -> Result<String> {
    match input.get("query").and_then(|query| query.get("kind")).and_then(Value::as_str) {
        Some("watch" | "poll_watch" | "watches" | "cancel_watch" | "release_watch") => {
            query_watch::execute(snapshot,context,input,publish)
        }
        Some("capture" | "changes" | "baselines" | "release_baseline") => {
            let narrowed = result_context(context)?;
            query_history::execute(snapshot,&narrowed,input,|value|publish_with_active_work(context,value,publish))
        }
        _ => {
            let narrowed = result_context(context)?;
            publish_with_active_work(context,execute(snapshot,&narrowed,input)?,publish)
        }
    }
}
