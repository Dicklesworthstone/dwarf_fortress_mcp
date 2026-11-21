//! A fitted resource-release receipt with no cached game facts or admission
//! claim. Rendering is passed into semantic-query cleanup before any mutation.

use dfmcp_core::{DfmcpError, ErrorCode, Result, SessionId, WorkBudget};
use serde_json::{Value, json};

use crate::agent_turn::{AgentPhase, AgentTurnBuilder, ContinuityStatus};
use crate::read_session_release::MAX_CLOSE_RECEIPTS;

pub(crate) fn render(
    id: SessionId,
    budget: WorkBudget,
    profile: &str,
    unadmitted_development: bool,
    poisoned: bool,
    released: Value,
) -> Result<String> {
    let mut value = json!({
        "ok": true,
        "scope": "session",
        "closed": true,
        "session_id": id.to_string(),
        "profile": profile,
        "release_only": true,
        "process_local_resources": released,
        "session_mutex_poisoned": poisoned,
        "native_captures": 0,
        "files_modified": false,
        "game_effects_cancelled": false,
        "mutation_dispatched": false,
        "game_state_observed": false,
        "close_receipt_retention": MAX_CLOSE_RECEIPTS,
        "receipt_durable": false,
        "reopen_requires_new_session": true,
    });
    let mut briefing = json!({
        "session_closed": true,
        "mutation_admissible": false,
        "game_state_accessed": false,
        "active_work_scope": "released_process_session_only",
    });
    if unadmitted_development {
        value["runtime"] = json!("unadmitted_development");
        value["runtime_admitted"] = json!(false);
        briefing["runtime"] = json!("unadmitted_development");
        briefing["runtime_admitted"] = json!(false);
        briefing["compatibility_admitted"] = json!(false);
    }
    let out = AgentTurnBuilder::new("fortress.cancel", AgentPhase::Inspect)
        .session_id(id.to_string())
        .continuity(
            ContinuityStatus::Partial,
            None,
            Some(json!({"reason": "session_closed_without_game_observation"})),
            None,
        )
        .briefing(briefing)
        .coverage(json!({
            "status": "partial",
            "complete_domains": ["session_resource_release"],
            "omitted_domains": ["current_game_state", "game_effect_outcomes"],
        }))
        .attach(value);
    let maximum = budget
        .max_bytes
        .min(u64::from(budget.max_output_tokens) * 4);
    if out.len() as u64 > maximum {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "session close receipt does not fit; no resources were released",
        ));
    }
    Ok(out)
}

#[cfg(test)]
#[path = "read_session_release_packet_tests.rs"]
mod tests;
