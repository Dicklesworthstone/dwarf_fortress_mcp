//! Typed query dispatch over the exact session-owned canonical projection.
//! Pure queries remain stateless. Query baselines use a publisher boundary so
//! retained state never advances before the complete Agent Turn can be rendered.

#[path = "semantic_query_core.rs"]
mod core_query;
#[path = "operational_query.rs"]
mod operational_query;
#[path = "query_history.rs"]
mod query_history;

use dfmcp_core::{OperationContext, Result};
use dfmcp_world::WorldSnapshot;
use serde_json::Value;

pub fn execute(snapshot: &WorldSnapshot, context: &OperationContext, input: &Value) -> Result<Value> {
    match input.get("query").and_then(|query| query.get("kind")).and_then(Value::as_str) {
        Some("aggregate" | "search") => operational_query::execute(snapshot, context, input),
        _ => core_query::execute(snapshot, context, input),
    }
}

/// Render before publishing any process-local query baseline state. The callback
/// must enforce the complete response budget, including the Agent Turn packet.
pub fn execute_with_publisher<F>(snapshot: &WorldSnapshot, context: &OperationContext,
    input: &Value, publish: F) -> Result<String>
where F: FnOnce(Value) -> Result<String> {
    match input.get("query").and_then(|query| query.get("kind")).and_then(Value::as_str) {
        Some("capture" | "changes" | "baselines" | "release_baseline") => {
            query_history::execute(snapshot, context, input, publish)
        }
        _ => publish(execute(snapshot, context, input)?),
    }
}
