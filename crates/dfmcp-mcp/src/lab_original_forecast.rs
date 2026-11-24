//! Exact original-set forecast through the existing laboratory query tool.
//! Current custody is admitted before cloning; predictions never enter history.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use dfmcp_core::{Capability, DfmcpError, Digest32, ErrorCode, OperationContext, Result, RiskTier};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{LabSession, anchor_json};

#[path = "original_forecast_model.rs"]
pub(super) mod model;

#[path = "original_forecast_admission.rs"]
mod admission;
pub(super) use admission::{AdmittedProjectSet, admit_complete_projects};

#[derive(Deserialize)]
struct Selector {
    mode: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    ForecastOriginals,
}

#[derive(Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Scope {
    #[default]
    CurrentStage,
    CompleteProject,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    mode: Mode,
    plan_digests: Vec<String>,
    horizon_ticks: u64,
    #[serde(default)]
    forecast_scope: Scope,
}

fn invalid(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn exhausted(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

pub(super) fn parse(raw: &str) -> Result<Option<Request>> {
    if raw.len() > 65_536 {
        return Err(exhausted("query exceeds its request byte bound"));
    }
    if !raw.trim_start().starts_with('{') {
        return Ok(None);
    }
    let selector: Selector = serde_json::from_str(raw)
        .map_err(|error| invalid(format!("query requires one unambiguous mode: {error}")))?;
    if selector.mode != "forecast_originals" {
        return Ok(None);
    }
    if raw.len() > 2_048 {
        return Err(exhausted(
            "joint original forecast request exceeds 2048 bytes",
        ));
    }
    let request: Request = serde_json::from_str(raw)
        .map_err(|error| invalid(format!("invalid original forecast request: {error}")))?;
    let Mode::ForecastOriginals = request.mode;
    if request.plan_digests.is_empty() || request.plan_digests.len() > 8 {
        return Err(invalid(
            "forecast_originals requires one to eight plan_digests",
        ));
    }
    let mut distinct = BTreeSet::new();
    for digest in &request.plan_digests {
        let digest = super::selected_plan_wait::parse_digest(digest)?;
        if !distinct.insert(digest) {
            return Err(invalid("forecast_originals cannot duplicate an original"));
        }
    }
    Ok(Some(request))
}

fn remaining(context: &OperationContext, started: Instant) -> Result<OperationContext> {
    for capability in [Capability::Observe, Capability::Query, Capability::Plan] {
        context.authorize(capability, RiskTier::ReadOnly, &[], None)?;
    }
    let spent = started.elapsed().as_nanos().div_ceil(1_000_000);
    if spent >= u128::from(context.budget.max_wall_millis) {
        return Err(exhausted(
            "joint forecast exhausted its shared wall-time budget",
        ));
    }
    let mut result = context.clone();
    result.budget.max_wall_millis -= spent as u64;
    Ok(result)
}

pub(super) fn execute(
    session: &LabSession,
    context: &OperationContext,
    request: &Request,
    started: Instant,
) -> Result<Value> {
    remaining(context, started)?;
    if context.anchor != session.adapter.snapshot().anchor()
        || context.session_id != session.session_id
    {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "joint forecast lost its current session source",
        ));
    }
    if request.horizon_ticks > context.budget.max_game_ticks {
        return Err(exhausted(
            "joint forecast exceeds the negotiated simulated-tick budget",
        ));
    }
    let digests: Vec<Digest32> = request
        .plan_digests
        .iter()
        .map(|digest| super::selected_plan_wait::parse_digest(digest))
        .collect::<Result<_>>()?;
    let mut admitted = admission::admit_originals(
        session,
        &digests,
        context,
        started,
        request.forecast_scope == Scope::CompleteProject,
    )?;
    let projects = std::mem::take(&mut admitted.projects);
    let carried_progress = std::mem::take(&mut admitted.carried);
    let source = &admitted.source;
    let mut inspections = 0u64;
    let mut inspected_records = 0u64;
    let mut inspected_bytes = 0u64;
    let mut predicate_node_visits_bound = 0u64;
    let prediction = model::forecast(
        &session.adapter,
        &admitted.originals,
        admitted.owned_actions,
        context,
        request.horizon_ticks,
        started,
        (request.forecast_scope == Scope::CompleteProject).then_some(projects),
        carried_progress,
        |snapshot, modeled_actions, modeled_plan_bytes| {
            admitted.inspect_structure(snapshot, context, started)?;
            let work =
                super::query_budget::admit_sources(&[snapshot], &remaining(context, started)?)?;
            let records = work["records"]
                .as_u64()
                .ok_or_else(|| invalid("forecast source record accounting missing"))?
                .saturating_add(admitted.fixed_records)
                .saturating_add(modeled_actions);
            let bytes = work["canonical_bytes"]
                .as_u64()
                .ok_or_else(|| invalid("forecast source byte accounting missing"))?
                .saturating_add(admitted.fixed_bytes)
                .saturating_add(modeled_plan_bytes);
            if records > u64::from(context.budget.max_entities)
                || bytes > context.budget.max_bytes.min(16 * 1024 * 1024)
            {
                return Err(exhausted(
                    "joint forecast exceeds its complete current source and retained-custody budget",
                ));
            }
            if !snapshot.hash_is_valid() {
                return Err(DfmcpError::new(
                    ErrorCode::StaleAnchor,
                    "joint forecast source has an invalid canonical hash",
                ));
            }
            inspections += 1;
            inspected_records = inspected_records.saturating_add(records);
            inspected_bytes = inspected_bytes.saturating_add(bytes);
            Ok(())
        },
        |digest, snapshot| {
            // Charge the full selected predicate bound conservatively before
            // each original evaluation, including short-circuited clauses.
            predicate_node_visits_bound = predicate_node_visits_bound
                .saturating_add(admitted.predicate_node_bound)
                // At most one carried observation follows each modeled loop;
                // precharge the entire admitted monitor domain before it runs.
                .saturating_add(admitted.recovered_predicate_node_bound);
            if predicate_node_visits_bound > 4_000_000 {
                return Err(exhausted(
                    "joint forecast exceeds its aggregate predicate-work bound",
                ));
            }
            admitted.evaluate(digest, snapshot)
        },
    )?;
    let remaining = remaining(context, started)?;
    let work = super::query_work_json(&remaining);
    if started.elapsed() >= Duration::from_millis(context.budget.max_wall_millis) {
        return Err(exhausted(
            "joint forecast exhausted its deadline before publication",
        ));
    }
    let mut response = json!({
        "ok":true,"mode":"forecast_originals","schema":"dfmcp.lab-original-forecast/1",
        "session_id":session.session_id.to_string(),
        "anchor":anchor_json(&context.anchor),"query_work":work,
        "prediction":prediction,
        "source_work":{"retained_source_records":source.records,"retained_source_bytes":source.bytes,
            "retained_source_snapshots":source.snapshots,"predicate_node_bound":source.predicate_node_bound,
            "aggregate_predicate_node_bound":predicate_node_visits_bound,
            "recovered_predicate_nodes_per_evaluation":admitted.recovered_predicate_node_bound,
            "competing_recovered_custody_records":admitted.carried_custody_records,
            "competing_recovered_custody_bytes":admitted.carried_custody_bytes,
            "source_inspections":inspections,"inspected_records":inspected_records,
            "inspected_canonical_and_custody_bytes":inspected_bytes},
        "changes":[],"observation_only":true,
        "next_step":{"tool":"fortress.wait","reason":"a forecast proves no outcome; explicitly wait for current original evidence",
            "arguments":{"session_id":session.session_id.to_string(),"plan_digests":request.plan_digests}},
    });
    if request.forecast_scope == Scope::CompleteProject {
        response["schema"] = json!("dfmcp.lab-original-forecast/2");
        response["forecast_scope"] = json!("complete_project");
        response["source_work"]["additional_semantic_nodes_per_evaluation"] =
            json!(admitted.project_goal_node_bound);
        response["source_work"]["additional_original_snapshots"] =
            json!(admitted.project_snapshot_count);
        response["source_work"]["additional_original_records"] = json!(admitted.project_records);
        response["source_work"]["additional_original_bytes"] = json!(admitted.project_bytes);
        response["next_step"]["reason"] = json!(
            "observe the latest committed stages and separately review any continuation; hypothetical stages are not commit permits"
        );
        response["next_step"]["arguments"]["plan_digests"] = json!(admitted.next_wait_digests);
        response["next_step"]["reserve_endpoint_pursuits"] =
            json!(admitted.reserve_endpoint_pursuits);
        response["next_step"]["reserve_capture_rule"] = json!(
            "each pursuit retains its own exact endpoint; a tip wait does not retain predecessor samples; use explicit zero-tick selected waits in groups of at most eight at that absolute tick before advancing again"
        );
        if !admitted.recovered_goals.is_empty() {
            response["schema"] = json!(if admitted.unfinished_recovered_steps > 0 {
                "dfmcp.lab-original-forecast/4"
            } else {
                "dfmcp.lab-original-forecast/3"
            });
            response["source_work"]["recovered_original_source_witnesses"] =
                json!(admitted.recovered_sources);
            response["source_work"]["recovered_original_steps"] = json!(admitted.recovered_steps);
            response["source_work"]["verified_recovered_original_steps"] =
                json!(admitted.verified_recovered_steps);
            response["source_work"]["recovered_observation_steps"] =
                json!(admitted.unfinished_recovered_steps);
            response["source_work"]["fixed_structural_bytes"] = json!(admitted.fixed_shape.0);
            response["source_work"]["fixed_structural_work"] = json!(admitted.fixed_shape.1);
        }
    }
    Ok(response)
}

#[cfg(test)]
#[path = "lab_original_forecast_tests.rs"]
mod tests;
