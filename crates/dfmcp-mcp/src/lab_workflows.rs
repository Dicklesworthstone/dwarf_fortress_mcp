//! Closed high-level action requests over the unchanged laboratory vocabulary.
//!
//! Existing scenarios, projections and action arrays retain their implementation.
//! Workflow lowering adds no dispatch path: the server still seals and executes
//! the returned RequestedActions through its ordinary authority-bearing path.

#[path = "lab_world.rs"]
mod base;
pub(crate) use base::*;

#[path = "labor_workflow.rs"]
mod labor_workflow;
#[path = "civilian_protection_workflow.rs"]
mod civilian_protection_workflow;
#[path = "labor_analysis_query.rs"]
mod labor_analysis_query;

use dfmcp_core::{DfmcpError, ErrorCode, Result};
use dfmcp_intent::RequestedAction;
use dfmcp_world::WorldSnapshot;
use serde::Deserialize;
use serde_json::Value;

// Decode only the selector, then let the original closed decoder inspect the
// unchanged bytes. Unlike a Value round trip, this rejects duplicate selectors.
#[derive(Deserialize)]
struct WorkflowName { workflow: String }
#[derive(Deserialize)]
struct QueryName { mode: String }

pub(crate) fn parse_steps(raw: &str) -> Result<Vec<RequestedAction>> {
    if !raw.trim_start().starts_with('{') { return base::parse_steps(raw); }
    if raw.len() > MAX_ACTIONS_JSON_BYTES {
        return Err(DfmcpError::new(ErrorCode::BudgetExceeded, "workflow exceeds action request byte limit"));
    }
    let selector: WorkflowName = serde_json::from_str(raw).map_err(|_| {
        DfmcpError::new(ErrorCode::InvalidRequest, "workflow requires one unambiguous version selector")
    })?;
    match selector.workflow.as_str() {
        "labor_allocation_v1" => labor_workflow::parse(raw),
        "civilian_protection_v1" => civilian_protection_workflow::parse(raw),
        _ => Err(DfmcpError::new(ErrorCode::InvalidRequest, "unknown action workflow version")),
    }
}

pub(crate) fn query(snapshot: &WorldSnapshot, raw: &str) -> Result<Value> {
    if raw.len() > MAX_QUERY_JSON_BYTES {
        return Err(DfmcpError::new(ErrorCode::BudgetExceeded, "query exceeds request byte limit"));
    }
    if raw.trim_start().starts_with('{')
        && let Ok(selector) = serde_json::from_str::<QueryName>(raw)
    {
        match selector.mode.as_str() {
            "civilian_protection" => return civilian_protection_workflow::query(snapshot, raw),
            "labor_allocation" => return labor_analysis_query::query(snapshot, raw),
            _ => {}
        }
    }
    base::query(snapshot, raw)
}

#[cfg(test)]
#[path = "labor_workflow_tests.rs"]
mod labor_workflow_tests;
#[cfg(test)]
#[path = "civilian_protection_workflow_tests.rs"]
mod civilian_protection_workflow_tests;
#[cfg(test)]
#[path = "labor_analysis_query_tests.rs"]
mod labor_analysis_query_tests;
