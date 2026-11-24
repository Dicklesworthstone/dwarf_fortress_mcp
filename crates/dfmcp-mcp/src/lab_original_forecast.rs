//! Exact original-set forecast through the existing laboratory query tool.
//! Current custody is admitted before cloning; predictions never enter history.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use dfmcp_core::{
    ActionId, Capability, CommitState, DfmcpError, Digest32, ErrorCode, OperationContext, Result,
    RiskTier,
};
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

fn admit_live_actions(
    session: &LabSession,
    selected: &super::plan_cancellation::Selection,
    action_union: &mut BTreeSet<ActionId>,
    in_continued_project: bool,
    predecessor: bool,
) -> Result<()> {
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
        if in_continued_project {
            let work = session.adapter.action_work_state(*action)?;
            if !matches!(
                receipt.state,
                CommitState::Prepared
                    | CommitState::AppliedAwaitingVerification
                    | CommitState::Verified
            ) || matches!(work, dfmcp_intent::EffectWorkState::Unknown { .. })
            {
                return Err(DfmcpError::new(
                    ErrorCode::Conflict,
                    "complete-project continuation requires uninterrupted, known original action custody",
                ));
            }
            if predecessor && (receipt.state != CommitState::Verified || !work.is_quiescent()) {
                return Err(DfmcpError::new(
                    ErrorCode::Conflict,
                    "complete-project predecessors must already have Verified, physically quiet actions",
                ));
            }
        }
        action_union.insert(*action);
    }
    Ok(())
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
    let mut source = SelectedPlanWait::admit_retained_set_source(&members, session)?;
    let mut project_lineages = BTreeMap::new();
    let mut complete_custody = None;
    if request.forecast_scope == Scope::CompleteProject {
        let mut custody_digests = digests.clone();
        for member in &members {
            remaining(context, started)?;
            let original = session
                .objectives
                .iter()
                .find(|original| original.plan_digest == member.actual_digest())
                .ok_or_else(|| invalid("forecast original goal disappeared"))?;
            let lineage = original.forecast_project_source(session)?;
            custody_digests.extend(lineage.1.iter().map(|(digest, _)| *digest));
            project_lineages.insert(member.actual_digest(), lineage);
        }
        if custody_digests.len() > digests.len() {
            let admitted = SelectedPlanWait::set_members(session, &custody_digests, started)?;
            source = SelectedPlanWait::admit_retained_set_source(&admitted, session)?;
            complete_custody = Some(admitted);
        }
    }
    let mut originals = Vec::with_capacity(members.len());
    let mut goals = BTreeMap::new();
    let mut action_union = BTreeSet::new();
    let mut projects = Vec::new();
    let mut project_snapshots = Vec::new();
    let mut recovered_goals = BTreeSet::new();
    let mut recovered_sources = Vec::new();
    let mut recovered_records = 0u64;
    let mut recovered_bytes = 0u64;
    let mut recovered_steps = 0usize;
    let mut verified_recovered_steps = 0usize;
    let mut unfinished_recovered_steps = 0usize;
    let mut recovered_digests = BTreeSet::new();
    let mut recovered_shape = (0u64, 0u64);
    let mut reserve_snapshots = Vec::new();
    let mut next_wait_digests = Vec::new();
    let mut reserve_endpoint_pursuits = Vec::new();
    let mut admitted_snapshots = BTreeSet::from([context.anchor.state_hash]);
    for member in complete_custody.as_deref().unwrap_or(&members) {
        if let Some(original) = session
            .objectives
            .iter()
            .find(|goal| goal.plan_digest == member.actual_digest())
            && let Some((anchor, _)) = original.selected_wait_reserve_observation
        {
            if admitted_snapshots.insert(anchor.state_hash) {
                let snapshot = session
                    .history
                    .get(&anchor.state_hash)
                    .filter(|snapshot| snapshot.anchor() == anchor)
                    .ok_or_else(|| invalid("forecast reserve source disappeared"))?;
                reserve_snapshots.push(snapshot);
            }
        }
    }
    for member in &members {
        remaining(context, started)?;
        let original = session
            .objectives
            .iter()
            .find(|original| original.plan_digest == member.actual_digest())
            .ok_or_else(|| invalid("forecast original goal disappeared"))?;
        // A present live receipt remains authoritative: malformed live custody
        // cannot fall back to recovery. The default current-stage route keeps
        // its existing live-only behavior.
        let recovery = if request.forecast_scope == Scope::CompleteProject
            && !session
                .commit_receipts
                .contains_key(&member.requested_digest().to_hex())
        {
            let plan = original
                .resource_plan()
                .ok_or_else(|| invalid("recovered project seal disappeared"))?;
            let predecessor = project_lineages.get(&member.actual_digest())
                .is_some_and(|(_, descendants)| !descendants.is_empty());
            let (source, census, evidence) = original.forecast_recovered_project(session, predecessor)?;
            let verified = census.iter().filter(|row| row.state == "verified").count();
            verified_recovered_steps = verified_recovered_steps.saturating_add(verified);
            unfinished_recovered_steps = unfinished_recovered_steps.saturating_add(census.len() - verified);
            recovered_digests.insert(member.actual_digest());
            let recovered = model::RecoveredOriginal::new(
                plan, census, evidence, session.adapter.snapshot(), &session.carried,
            )?;
            recovered_steps = recovered_steps.saturating_add(plan.steps.len());
            recovered_goals.insert(member.actual_digest());
            Some((source, recovered))
        } else {
            None
        };
        let selected = if recovery.is_some() {
            super::plan_cancellation::Selection {
                requested: member.requested_digest(),
                actual: member.actual_digest(),
                plan_id: original
                    .resource_plan()
                    .ok_or_else(|| invalid("recovered project seal disappeared"))?
                    .id
                    .to_string(),
                actions: Vec::new(),
            }
        } else {
            super::plan_cancellation::select(session, member.requested_digest())?
        };
        let continued = project_lineages
            .get(&selected.actual)
            .is_some_and(|(_, descendants)| !descendants.is_empty());
        if recovery.is_none() {
            admit_live_actions(session, &selected, &mut action_union, continued, continued)?;
        }
        original.selected_wait_truth(session)?;
        original.selected_wait_stage_truth(session)?;
        if request.forecast_scope == Scope::CompleteProject {
            let (project_source, descendants) = project_lineages
                .remove(&selected.actual)
                .ok_or_else(|| invalid("complete-project lineage disappeared"))?;
            let plan = original
                .resource_plan()
                .ok_or_else(|| invalid("project original seal disappeared"))?;
            let (mut project, ledger) = if let Some((archived, recovered)) = recovery {
                if admitted_snapshots.insert(archived.anchor.state_hash) {
                    recovered_records = recovered_records.saturating_add(archived.records);
                    recovered_bytes = recovered_bytes.saturating_add(archived.bytes);
                    recovered_shape.0 = recovered_shape.0.saturating_add(archived.structural_bytes);
                    recovered_shape.1 = recovered_shape.1.saturating_add(archived.structural_work);
                    recovered_sources.push(json!({
                        "anchor":anchor_json(&archived.anchor),
                        "records":archived.records,"canonical_bytes":archived.bytes,
                        "structural_bytes":archived.structural_bytes,"structural_work":archived.structural_work,
                        "verification":"exact_sealed_snapshot_during_durable_recovery",
                        "query_archive_reads":0,
                    }));
                }
                (
                    model::Project::new(selected.actual, project_source, archived.ledger)
                        .with_recovery(recovered)?,
                    archived.ledger,
                )
            } else {
                let sealed = session
                    .history
                    .get(&plan.anchor.state_hash)
                    .filter(|snapshot| snapshot.anchor() == plan.anchor)
                    .ok_or_else(|| {
                        invalid("complete_project requires the exact retained original source")
                    })?;
                // Admit exact source bytes before checking the original ledger
                // or preparing any future stage. No historical hydration.
                if admitted_snapshots.insert(sealed.state_hash) {
                    project_snapshots.push(sealed);
                }
                super::query_budget::admit_sources(&[sealed], &remaining(context, started)?)?;
                let ledger = crate::lab_world::ProductionGoalLedger::capture(sealed)?;
                ledger.validate_plan(plan)?;
                ledger.validate_current(session.adapter.snapshot())?;
                (
                    model::Project::new(selected.actual, project_source, ledger),
                    ledger,
                )
            };
            let root_reserve = original.selected_wait_reserve_horizon()?;
            next_wait_digests.push(descendants.last().map_or_else(
                || member.requested_digest().to_hex(),
                |(digest, _)| digest.to_hex(),
            ));
            if let Some(until) = root_reserve {
                reserve_endpoint_pursuits.push(json!({
                    "plan_digest":selected.actual.to_hex(),"reserve_until_tick":until,
                    "sample_retained":original.selected_wait_reserve_observation.is_some(),
                }));
            }
            for (index, (digest, parent)) in descendants.iter().enumerate() {
                remaining(context, started)?;
                let objective = session
                    .objectives
                    .iter()
                    .find(|objective| objective.plan_digest == *digest)
                    .ok_or_else(|| invalid("retained project stage disappeared"))?;
                objective.selected_wait_truth(session)?;
                objective.selected_wait_stage_truth(session)?;
                let reserve_until = objective.selected_wait_reserve_horizon()?;
                if reserve_until != root_reserve {
                    return Err(invalid(
                        "continuation changed the original absolute reserve horizon",
                    ));
                }
                if let Some(until) = reserve_until {
                    reserve_endpoint_pursuits.push(json!({
                        "plan_digest":digest.to_hex(),"reserve_until_tick":until,
                        "sample_retained":objective.selected_wait_reserve_observation.is_some(),
                    }));
                }
                let plan = objective
                    .resource_plan()
                    .ok_or_else(|| invalid("retained continuation seal disappeared"))?;
                let retained_original = model::Original {
                    requested: *digest,
                    actual: *digest,
                    actions: Vec::new(),
                    reserve_until,
                    reserve_observation: objective
                        .selected_wait_reserve_observation
                        .map(|(_, truth)| truth),
                    can_continue: true,
                };
                if !session.commit_receipts.contains_key(&digest.to_hex()) {
                    let (archived, census, evidence) =
                        objective.forecast_recovered_project(session, index + 1 < descendants.len())?;
                    let verified = census.iter().filter(|row| row.state == "verified").count();
                    verified_recovered_steps = verified_recovered_steps.saturating_add(verified);
                    unfinished_recovered_steps = unfinished_recovered_steps.saturating_add(census.len() - verified);
                    recovered_digests.insert(*digest);
                    let recovered = model::RecoveredOriginal::new(
                        plan,
                        census,
                        evidence,
                        session.adapter.snapshot(),
                        &session.carried,
                    )?;
                    recovered_steps = recovered_steps.saturating_add(plan.steps.len());
                    recovered_goals.insert(*digest);
                    if admitted_snapshots.insert(archived.anchor.state_hash) {
                        recovered_records = recovered_records.saturating_add(archived.records);
                        recovered_bytes = recovered_bytes.saturating_add(archived.bytes);
                        recovered_shape.0 =
                            recovered_shape.0.saturating_add(archived.structural_bytes);
                        recovered_shape.1 =
                            recovered_shape.1.saturating_add(archived.structural_work);
                        recovered_sources.push(json!({
                            "anchor":anchor_json(&archived.anchor),
                            "records":archived.records,"canonical_bytes":archived.bytes,
                            "structural_bytes":archived.structural_bytes,"structural_work":archived.structural_work,
                            "verification":"exact_sealed_snapshot_during_durable_recovery",
                            "query_archive_reads":0,
                        }));
                    }
                    project.retain_recovered_stage(
                        plan.clone(),
                        *parent,
                        retained_original,
                        recovered,
                        session.adapter.snapshot(),
                    )?;
                    continue;
                }
                let retained = super::plan_cancellation::select(session, *digest)?;
                if retained.actual != *digest {
                    return Err(invalid(
                        "continuation custody cannot alias another sealed stage",
                    ));
                }
                admit_live_actions(
                    session,
                    &retained,
                    &mut action_union,
                    true,
                    index + 1 < descendants.len(),
                )?;
                let sealed = session
                    .history
                    .get(&plan.anchor.state_hash)
                    .filter(|snapshot| snapshot.anchor() == plan.anchor)
                    .ok_or_else(|| {
                        invalid("complete_project requires each exact retained stage source")
                    })?;
                super::query_budget::admit_sources(&[sealed], &remaining(context, started)?)?;
                ledger.validate_current(sealed)?;
                crate::lab_world::ProductionGoalLedger::capture(sealed)?.validate_plan(plan)?;
                if admitted_snapshots.insert(sealed.state_hash) {
                    project_snapshots.push(sealed);
                }
                project.retain_stage(
                    plan.clone(),
                    *parent,
                    model::Original {
                        actions: retained.actions,
                        ..retained_original
                    },
                    sealed,
                )?;
            }
            projects.push(project);
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
    let mut carried_progress = Vec::new();
    let mut carried_custody_bytes = 0u64;
    let mut carried_custody_records = 0u64;
    let mut additional_carried_bytes = 0u64;
    if request.forecast_scope == Scope::CompleteProject {
        let carried_originals: BTreeSet<_> = session.carried.iter()
            .filter(|row| !row.is_final()).map(|row| row.plan_digest).collect();
        for digest in carried_originals {
            remaining(context, started)?;
            let owner = session.objectives.iter().find(|original| original.plan_digest == digest)
                .ok_or_else(|| invalid("competing recovered work lost its exact original owner"))?;
            let (rows, custody_bytes) = owner.forecast_carried_progress(session)?;
            if !recovered_digests.contains(&digest) {
                carried_custody_bytes = carried_custody_bytes.saturating_add(custody_bytes as u64);
                carried_custody_records = carried_custody_records.saturating_add(
                    owner.resource_plan().map_or(0, |plan| plan.steps.len()) as u64,
                );
            }
            if recovered_goals.is_empty() {
                for row in &rows {
                    additional_carried_bytes = additional_carried_bytes.saturating_add(row.to_json().to_string().len() as u64);
                }
            }
            carried_progress.extend(rows);
            if carried_progress.len() > context.budget.max_actions as usize {
                return Err(exhausted("competing recovered monitors exceed the shared action budget"));
            }
        }
    }
    let recovered_peer_steps = carried_progress.iter()
        .filter(|row| !recovered_digests.contains(&row.plan_digest)).count();
    let recovered_predicate_bound = model::recovered_predicate_node_bound(&carried_progress)?;
    if action_union.len().saturating_add(recovered_steps).saturating_add(recovered_peer_steps)
        > context.budget.max_actions as usize
    {
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
        .ok_or_else(|| invalid("project source record accounting missing"))?
        .saturating_add(recovered_records);
    let project_bytes = project_sources["canonical_bytes"]
        .as_u64()
        .ok_or_else(|| invalid("project source byte accounting missing"))?
        .saturating_add(recovered_bytes);
    // Archived shapes have already passed the same iterative inspector during
    // recovery. Combine their authenticated bounds with all retained original
    // and reserve sources; every private current source joins this union below.
    let mut fixed_shape = recovered_shape;
    if !recovered_goals.is_empty() {
        for snapshot in project_snapshots.iter().chain(&reserve_snapshots) {
            let shape = super::query_budget::source_shape(
                snapshot,
                &remaining(context, started)?,
                Instant::now(),
            )?;
            fixed_shape.0 = fixed_shape.0.saturating_add(shape.0);
            fixed_shape.1 = fixed_shape.1.saturating_add(shape.1);
        }
    }
    let fixed_records = source
        .records
        .saturating_sub(current_records)
        .saturating_add(project_records)
        .saturating_add(carried_custody_records)
        .saturating_add(session.open_actions.len() as u64);
    let fixed_bytes = source
        .bytes
        .saturating_sub(current_bytes)
        .saturating_add(receipt_bytes)
        .saturating_add(project_bytes)
        .saturating_add(carried_custody_bytes)
        .saturating_add(additional_carried_bytes);
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
        carried_progress,
        |snapshot, modeled_actions, modeled_plan_bytes| {
            if !recovered_goals.is_empty() {
                let shape = super::query_budget::source_shape(
                    snapshot,
                    &remaining(context, started)?,
                    Instant::now(),
                )?;
                if fixed_shape.0.saturating_add(shape.0) > 16 * 1024 * 1024
                    || fixed_shape.1.saturating_add(shape.1) > 4_000_000
                {
                    return Err(exhausted(
                        "recovered forecast exceeds its complete structural source byte or work bound",
                    ));
                }
            }
            let work =
                super::query_budget::admit_sources(&[snapshot], &remaining(context, started)?)?;
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
                .saturating_add(project_goal_node_bound)
                // At most one carried observation follows each modeled loop;
                // precharge the entire admitted monitor domain before it runs.
                .saturating_add(recovered_predicate_bound);
            if predicate_node_visits_bound > 4_000_000 {
                return Err(exhausted(
                    "joint forecast exceeds its aggregate predicate-work bound",
                ));
            }
            let original = goals.get(&digest).ok_or_else(|| {
                invalid("selected original goal disappeared during private forecast")
            })?;
            if recovered_goals.contains(&digest) {
                original.forecast_recovered_truth_at(snapshot)
            } else {
                original.forecast_truth_at(snapshot)
            }
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
            "recovered_predicate_nodes_per_evaluation":recovered_predicate_bound,
            "competing_recovered_custody_records":carried_custody_records,
            "competing_recovered_custody_bytes":carried_custody_bytes,
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
            "observe the latest committed stages and separately review any continuation; hypothetical stages are not commit permits"
        );
        response["next_step"]["arguments"]["plan_digests"] = json!(next_wait_digests);
        response["next_step"]["reserve_endpoint_pursuits"] = json!(reserve_endpoint_pursuits);
        response["next_step"]["reserve_capture_rule"] = json!(
            "each pursuit retains its own exact endpoint; a tip wait does not retain predecessor samples; use explicit zero-tick selected waits in groups of at most eight at that absolute tick before advancing again"
        );
        if !recovered_goals.is_empty() {
            response["schema"] = json!(if unfinished_recovered_steps > 0 {
                "dfmcp.lab-original-forecast/4"
            } else {
                "dfmcp.lab-original-forecast/3"
            });
            response["source_work"]["recovered_original_source_witnesses"] =
                json!(recovered_sources);
            response["source_work"]["recovered_original_steps"] = json!(recovered_steps);
            response["source_work"]["verified_recovered_original_steps"] = json!(verified_recovered_steps);
            response["source_work"]["recovered_observation_steps"] = json!(unfinished_recovered_steps);
            response["source_work"]["fixed_structural_bytes"] = json!(fixed_shape.0);
            response["source_work"]["fixed_structural_work"] = json!(fixed_shape.1);
        }
    }
    Ok(response)
}

#[cfg(test)]
#[path = "lab_original_forecast_tests.rs"]
mod tests;
