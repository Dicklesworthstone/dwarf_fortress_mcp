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

use dfmcp_core::Result;
use dfmcp_intent::RequestedAction;

pub(crate) fn parse_steps(raw: &str) -> Result<Vec<RequestedAction>> {
    if raw.trim_start().starts_with('{') {
        labor_workflow::parse(raw)
    } else {
        base::parse_steps(raw)
    }
}

#[cfg(test)]
#[path = "labor_workflow_tests.rs"]
mod labor_workflow_tests;
