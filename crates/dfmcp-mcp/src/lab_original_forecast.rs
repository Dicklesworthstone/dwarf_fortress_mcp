//! Exact original-set forecast through the existing laboratory query tool.
//! Current custody is admitted before cloning; predictions never enter history.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use dfmcp_core::{Capability, DfmcpError, Digest32, ErrorCode, OperationContext, Result, RiskTier};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{LabSession, anchor_json, selected_plan_wait::SelectedPlanWait};

#[path = "original_forecast_model.rs"]
mod model;

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
    let members = SelectedPlanWait::set_members(session, &digests, started)?;
    let source = SelectedPlanWait::admit_retained_set_source(&members, session)?;
    let mut originals = Vec::with_capacity(members.len());
    let mut goals = BTreeMap::new();
    let mut action_union = BTreeSet::new();
    let mut projects = Vec::new();
    let mut project_snapshots = Vec::new();
    let mut admitted_snapshots = BTreeSet::from([context.anchor.state_hash]);
    for member in &members {
        if let Some(original) = session
            .objectives
            .iter()
            .find(|goal| goal.plan_digest == member.actual_digest())
            && let Some((anchor, _)) = original.selected_wait_reserve_observation
        {
            admitted_snapshots.insert(anchor.state_hash);
        }
    }
    for member in &members {
        remaining(context, started)?;
        // Recovery-only originals remain inspectable, but cannot synthesize
        // dispatch handles or reset obligation runtimes for a prediction.
        let selected = super::plan_cancellation::select(session, member.requested_digest())?;
        let original = session
            .objectives
            .iter()
            .find(|original| original.plan_digest == selected.actual)
            .ok_or_else(|| invalid("forecast original goal disappeared"))?;
        for action in &selected.actions {
            let receipt = session
                .adapter
                .action_receipt(*action)
                .ok_or_else(|| invalid("forecast original action disappeared"))?;
            if (!receipt.state.is_terminal()
                || !session.adapter.action_work_state(*action)?.is_quiescent())
                && !session.open_actions.contains(action)
            {
                return Err(DfmcpError::new(
                    ErrorCode::Conflict,
                    "unfinished forecast original is absent from the owning progression set",
                ));
            }
            action_union.insert(*action);
        }
        original.selected_wait_truth(session)?;
        original.selected_wait_stage_truth(session)?;
        if request.forecast_scope == Scope::CompleteProject {
            let project = original.forecast_project_source(session)?;
            let plan = original
                .resource_plan()
                .ok_or_else(|| invalid("project original seal disappeared"))?;
            let sealed = session
                .history
                .get(&plan.anchor.state_hash)
                .filter(|snapshot| snapshot.anchor() == plan.anchor)
                .ok_or_else(|| {
                    invalid("complete_project requires the exact retained original source")
                })?;
            // Admit exact source bytes before checking the original ledger or
            // preparing any hypothetical future stage. No historical hydration.
            if admitted_snapshots.insert(sealed.state_hash) {
                project_snapshots.push(sealed);
            }
            super::query_budget::admit_sources(&[sealed], &remaining(context, started)?)?;
            let ledger = crate::lab_world::ProductionGoalLedger::capture(sealed)?;
            ledger.validate_plan(plan)?;
            ledger.validate_current(session.adapter.snapshot())?;
            projects.push(model::Project::new(selected.actual, project, ledger));
        }
        goals.insert(selected.actual, original);
        originals.push(model::Original {
            requested: member.requested_digest(),
            actual: member.actual_digest(),
            actions: selected.actions,
            reserve_until: original.selected_wait_reserve_horizon()?,
            reserve_observation: original
                .selected_wait_reserve_observation
                .map(|(_, truth)| truth),
            can_continue: original.selected_wait_can_continue(),
        });
    }
    let mut owned_plans = BTreeSet::new();
    let mut receipt_bytes = 0u64;
    for id in &session.open_actions {
        remaining(context, started)?;
        action_union.insert(*id);
        let commit = session
            .adapter
            .action_plan_receipt(*id)
            .ok_or_else(|| invalid("current session progression lost an original commit"))?;
        if owned_plans.insert(commit.plan_digest) {
            let selected =
                super::plan_cancellation::select_for_census(session, commit.plan_digest)?;
            if !selected.actions.contains(id) {
                return Err(invalid(
                    "current progression action has inconsistent owned custody",
                ));
            }
            receipt_bytes = receipt_bytes.saturating_add(
                session
                    .commit_receipts
                    .get(&commit.plan_digest.to_hex())
                    .map_or(0, String::len) as u64,
            );
        }
    }
    if action_union.len() > context.budget.max_actions as usize {
        return Err(exhausted(
            "selected and competing original actions exceed the shared action budget",
        ));
    }
    let current_source = super::query_budget::admit_sources(
        &[session.adapter.snapshot()],
        &remaining(context, started)?,
    )?;
    let current_records = current_source["records"]
        .as_u64()
        .ok_or_else(|| invalid("forecast source record accounting missing"))?;
    let current_bytes = current_source["canonical_bytes"]
        .as_u64()
        .ok_or_else(|| invalid("forecast source byte accounting missing"))?;
    let project_sources = if project_snapshots.is_empty() {
        json!({"records":0,"canonical_bytes":0})
    } else {
        super::query_budget::admit_sources(&project_snapshots, &remaining(context, started)?)?
    };
    let project_records = project_sources["records"]
        .as_u64()
        .ok_or_else(|| invalid("project source record accounting missing"))?;
    let project_bytes = project_sources["canonical_bytes"]
        .as_u64()
        .ok_or_else(|| invalid("project source byte accounting missing"))?;
    let fixed_records = source
        .records
        .saturating_sub(current_records)
        .saturating_add(project_records)
        .saturating_add(session.open_actions.len() as u64);
    let fixed_bytes = source
        .bytes
        .saturating_sub(current_bytes)
        .saturating_add(receipt_bytes)
        .saturating_add(project_bytes);
    let mut inspections = 0u64;
    let mut inspected_records = 0u64;
    let mut inspected_bytes = 0u64;
    let mut predicate_node_visits_bound = 0u64;
    let project_goal_node_bound = if request.forecast_scope == Scope::CompleteProject {
        // One extra semantic goal and one possible remaining-reserve pass,
        // using the same closed-model bound as selected original inspection.
        super::selected_plan_wait::SEMANTIC_GOAL_NODE_RESERVE * 2
    } else {
        0
    };
    let prediction = model::forecast(
        &session.adapter,
        &originals,
        &session.open_actions,
        context,
        request.horizon_ticks,
        started,
        (request.forecast_scope == Scope::CompleteProject).then_some(projects),
        |snapshot, modeled_actions, modeled_plan_bytes| {
            let remaining = remaining(context, started)?;
            let work = super::query_budget::admit_sources(&[snapshot], &remaining)?;
            let records = work["records"]
                .as_u64()
                .ok_or_else(|| invalid("forecast source record accounting missing"))?
                .saturating_add(fixed_records)
                .saturating_add(modeled_actions);
            let bytes = work["canonical_bytes"]
                .as_u64()
                .ok_or_else(|| invalid("forecast source byte accounting missing"))?
                .saturating_add(fixed_bytes)
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
                .saturating_add(source.predicate_node_bound)
                .saturating_add(project_goal_node_bound);
            if predicate_node_visits_bound > 4_000_000 {
                return Err(exhausted(
                    "joint forecast exceeds its aggregate predicate-work bound",
                ));
            }
            goals
                .get(&digest)
                .ok_or_else(|| {
                    invalid("selected original goal disappeared during private forecast")
                })?
                .forecast_truth_at(snapshot)
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
            json!(project_goal_node_bound);
        response["source_work"]["additional_original_snapshots"] = json!(project_snapshots.len());
        response["source_work"]["additional_original_records"] = json!(project_records);
        response["source_work"]["additional_original_bytes"] = json!(project_bytes);
        response["next_step"]["reason"] = json!(
            "observe actual original progress and separately review any continuation; hypothetical stages are not commit permits"
        );
    }
    Ok(response)
}

#[cfg(test)]
#[path = "lab_original_forecast_tests.rs"]
mod tests;
