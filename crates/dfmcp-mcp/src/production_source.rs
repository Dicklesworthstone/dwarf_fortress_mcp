//! Compile a retained production source without changing its compiler generation.
//!
//! Historical sources retain their original summary and lowering. New sources
//! additionally bind every original request field and the fixed planner version
//! into the prepared-plan seal, including setup choices unused at this anchor.

use crate::lab_world::{ProductionPlanner, ProductionRequest, parse_steps};
use dfmcp_core::{DfmcpError, Digest32, ErrorCode, IntentId, Result, RiskTier};
use dfmcp_intent::{Constraint, Intent, StaticPlanner};
use dfmcp_world::WorldSnapshot;
use serde_json::{Value, json};

fn source_digest(request: &ProductionRequest) -> Digest32 {
    let mut bytes = b"dfmcp-production-source/1\0".to_vec();
    bytes.extend_from_slice(request.canonical_json().as_bytes());
    Digest32::of_bytes(&bytes)
}

pub(super) fn compile(
    request: &ProductionRequest,
    id: IntentId,
    snapshot: &WorldSnapshot,
    summary: &str,
) -> Result<(Intent, Value)> {
    let compiled = request.compile(snapshot)?;
    let mut analysis = compiled.analysis.clone();
    let summary = match request.planner() {
        None => summary.to_owned(),
        Some(ProductionPlanner::ConsumptionAwareV1) => {
            let digest = source_digest(request);
            let sealed = format!("{summary} [production source {}]", digest.to_hex());
            if sealed.len() > StaticPlanner::default().policy.max_string_bytes
                || sealed.contains('\0')
            {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidRequest,
                    "production summary plus its source seal exceeds the planner bound; supply a shorter summary",
                ));
            }
            analysis["source_binding"] = json!({
                "planner": "consumption_aware_v1",
                "production_source_digest": digest.to_hex(),
                "original_request_preserved": true,
            });
            sealed
        }
    };
    let requested_actions = parse_steps(&compiled.actions)?;
    let max_risk = requested_actions
        .iter()
        .map(|requested| requested.action.risk())
        .max()
        .unwrap_or(RiskTier::ReadOnly);
    let mut intent = Intent {
        id,
        anchor: snapshot.anchor(),
        summary,
        terminal_condition: compiled.terminal.clone(),
        constraints: vec![Constraint::MaxRisk(max_risk)],
        requested_actions,
    };
    compiled.apply_capacity_horizon(&mut intent)?;
    Ok((intent, analysis))
}

#[cfg(test)]
#[path = "production_source_tests.rs"]
mod tests;
