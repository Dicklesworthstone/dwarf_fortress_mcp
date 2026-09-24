//! Production diagnosis and conservative supply allocation in the coherent
//! spatial/1.8 universe. Historical callers supply replayed states, not a new
//! operations-only projection. This module never reads the bridge or assigns work.
use super::*;
use dfmcp_core::Digest32;
use std::time::Instant;

#[path = "operations_production.rs"]
mod shared;

#[path = "production_chain_query.rs"]
mod chain;

pub(super) fn handles(input: &Value) -> bool {
    chain::handles(input) || shared::handles(input) || workforce_queries::portfolio::handles(input)
}
pub(super) fn extend_schema(base: Value) -> Result<Value> {
    chain::extend_schema(workforce_queries::portfolio::extend_schema(
        shared::extend_schema(base)?,
    )?)
}
pub(super) fn execute(
    state: &LiveSpatialCitizenState,
    context: &OperationContext,
    input: &Value,
) -> Result<Value> {
    if chain::handles(input) {
        chain::execute(state, context, input)
    } else if workforce_queries::portfolio::handles(input) {
        workforce_queries::portfolio::execute(state, context, input)
    } else {
        shared::execute(state, context, input)
    }
}

fn custody(session: &mut Session, context: &OperationContext) -> Result<()> {
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if session.anchor()? != context.anchor {
        return Err(error(
            ErrorCode::StaleAnchor,
            "production context differs from the coherent session",
        ));
    }
    if let Some(journal) = session.journal.as_mut() {
        journal.validate_custody(context)?;
        if journal.state().snapshot().map(|s| s.anchor()) != Some(context.anchor) {
            return Err(error(
                ErrorCode::CorruptLedger,
                "production observation archive disagrees with published state",
            ));
        }
    }
    Ok(())
}

pub(super) fn live(
    session: &mut Session,
    context: &OperationContext,
    input: &Value,
) -> Result<String> {
    let started = Instant::now();
    if session.source.archive_only() {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "archive production reads must use the historical response path",
        ));
    }
    if session.source.poisoned() {
        return Err(error(
            ErrorCode::AdapterUnavailable,
            "production source fenced; use verified historical queries",
        ));
    }
    custody(session, context)?;
    let projection = situation_presentation::tactical(session, context)?;
    let mut publication_context = context.clone();
    publication_context.budget.max_bytes = projection
        .result_byte_budget()?
        .checked_sub(64)
        .ok_or_else(|| {
            error(
                ErrorCode::BudgetExceeded,
                "production metadata leaves no result budget",
            )
        })? as u64;
    // Rows use the allowance AFTER reserving watches. Publication must use the
    // pre-reservation allowance, since it adds that same metadata exactly once.
    let mut query_context = semantic_query::result_context(&publication_context)?;
    let elapsed = started.elapsed().as_millis();
    if elapsed >= u128::from(context.budget.max_wall_millis) {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "production preflight exhausted wall-time budget",
        ));
    }
    query_context.budget.max_wall_millis -= elapsed as u64;
    let mut value = execute(&session.state, &query_context, input)?;
    value["native_captures"] = json!(0);
    custody(session, context)?;
    semantic_query::publish_with_active_work(&publication_context, value, |value| {
        let encoded = finish(&projection, value)?;
        if started.elapsed().as_millis() >= u128::from(context.budget.max_wall_millis) {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "production response exhausted wall-time budget",
            ));
        }
        Ok(encoded)
    })
}

/// A job/worker or attachment inspection derived from an old diagnostic must
/// follow the very same archive record, even after the live observation advances.
/// Both original links carry the full anchor and were counted in the row budget.
pub(super) fn pin_historical(value: &mut Value, record: u64, digest: Digest32) -> Result<()> {
    if let Some(rows) = value.get_mut("rows").and_then(Value::as_array_mut) {
        for row in rows {
            for (field, kind) in [
                ("inspect_assignment", "inspect"),
                ("inspect_relationships", "traverse"),
            ] {
                if let Some(request) = row.get(field) {
                    let query = request
                        .get("query")
                        .filter(|q| q.get("kind").and_then(Value::as_str) == Some(kind))
                        .ok_or_else(|| {
                            error(
                                ErrorCode::InternalInvariantViolation,
                                "production inspection has the wrong fixed query kind",
                            )
                        })?;
                    let replacement = json!({"schema":"dfmcp.query/1","query":{"kind":"historical_query",
                        "record":record,"record_digest":digest.to_string(),"query":query}});
                    if replacement.to_string().len() > request.to_string().len() {
                        return Err(error(
                            ErrorCode::InternalInvariantViolation,
                            "historical inspection exceeds reserved production row size",
                        ));
                    }
                    row[field] = replacement;
                }
            }
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
#[path = "spatial_portfolio_tests.rs"]
mod portfolio_tests;
#[cfg(all(test, unix))]
#[path = "spatial_portfolio_sites_harness.rs"]
mod site_tests;
#[cfg(all(test, unix))]
#[path = "spatial_production_tests.rs"]
mod tests;
