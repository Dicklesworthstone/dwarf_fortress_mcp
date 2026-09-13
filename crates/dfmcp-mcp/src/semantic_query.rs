//! Typed query dispatch over the exact session-owned canonical projection.
//! Basic inspection and graph queries retain their original implementation;
//! summaries and lexical search share the same schema and authority boundary.

#[path = "semantic_query_core.rs"]
mod core_query;
#[path = "operational_query.rs"]
mod operational_query;

use dfmcp_core::{OperationContext, Result};
use dfmcp_world::WorldSnapshot;
use serde_json::Value;

pub fn execute(snapshot: &WorldSnapshot, context: &OperationContext, input: &Value) -> Result<Value> {
    match input.get("query").and_then(|query| query.get("kind")).and_then(Value::as_str) {
        Some("aggregate" | "search") => operational_query::execute(snapshot, context, input),
        _ => core_query::execute(snapshot, context, input),
    }
}
