//! Current stockpile request lowering with exact historical source retention.
//!
//! Unmarked stored arrays retain their filter-only proof. Every fresh stockpile
//! request receives the closed complete-settings marker before source custody.

use dfmcp_core::{DfmcpError, ErrorCode, Result};
use dfmcp_intent::Action;
use serde_json::{Value, json};

pub(crate) fn canonical_current_actions(raw: &str) -> Result<String> {
    // Validate the original closed bytes first. A JSON Value round-trip must
    // not erase duplicate fields or launder unknown proof generations.
    let steps = super::parse_steps(raw)?;
    if !steps
        .iter()
        .any(|step| matches!(step.action, Action::ConfigureStockpile { .. }))
    {
        return Ok(raw.to_owned());
    }
    let mut source: Value = serde_json::from_str(raw).map_err(|error| {
        DfmcpError::new(
            ErrorCode::InvalidRequest,
            format!("invalid actions source: {error}"),
        )
    })?;
    let rows = source.as_array_mut().ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::InvalidRequest,
            "stockpile configuration requires an explicit semantic action array",
        )
    })?;
    if rows.len() != steps.len() {
        return Err(DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "stockpile action source and parsed step counts differ",
        ));
    }
    for (row, step) in rows.iter_mut().zip(steps) {
        if matches!(step.action, Action::ConfigureStockpile { .. }) {
            row["proof"] = json!("stockpile_settings_v1");
        }
    }
    let encoded = source.to_string();
    if encoded.len() > super::MAX_ACTIONS_JSON_BYTES {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "complete stockpile source exceeds the action request byte bound",
        ));
    }
    // The persisted bytes must obey the very parser used by exact recovery.
    super::parse_steps(&encoded)?;
    Ok(encoded)
}

#[cfg(test)]
#[path = "stockpile_source_tests.rs"]
mod tests;
