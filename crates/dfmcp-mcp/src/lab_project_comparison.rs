//! Explicit complete-project comparisons. The historical v1 comparison remains
//! a current-stage forecast; this generation follows separately reviewed future
//! stages only inside discarded model forks.

use super::super::project_forecast::{ProjectSource, forecast_project_bounded};
use super::*;

const PROJECT_POLICY: &str =
    "complete_project_predictions_then_risk_checkpoint_completion_tick_action_count_key_v1";

pub(super) fn validate_candidate(candidate: &Candidate) -> Result<()> {
    if candidate.actions.is_some()
        || candidate.paused_target.is_some()
        || !matches!(
            candidate
                .blueprint
                .as_ref()
                .and_then(|value| value["template"].as_str()),
            Some("production" | "provisioned_room" | "connected_room")
        )
    {
        return Err(invalid(
            "complete-project comparisons require production, provisioned_room or connected_room blueprint candidates",
        ));
    }
    Ok(())
}

fn project_source(source: &PlanSource) -> Result<ProjectSource> {
    match source {
        PlanSource::Production { raw, .. } => ProjectSource::parse_production(raw),
        PlanSource::Blueprint { raw, .. }
            if super::super::room_provisioning::is_provisioned_room(raw) =>
        {
            ProjectSource::parse_room(raw)
        }
        PlanSource::ConnectedRoom { raw, .. } => ProjectSource::parse_connected_room(raw),
        _ => Err(invalid(
            "complete-project comparison lost its normalized original project source",
        )),
    }
}

struct Source {
    candidate: Candidate,
    source: PlanSource,
    project: ProjectSource,
    reserve_until: Option<u64>,
}

fn candidate_failure(
    rows: &mut Vec<Value>,
    key: &str,
    stage: &str,
    error: DfmcpError,
) -> Result<()> {
    // A shared work or cancellation refusal cannot become an ordinary loss that
    // would make another candidate appear preferable.
    if matches!(
        error.code,
        ErrorCode::BudgetExceeded | ErrorCode::CancellationRequested
    ) {
        return Err(error);
    }
    rows.push(unavailable(key, stage, &error));
    Ok(())
}

fn project_cost(prediction: &Value) -> Result<(RiskTier, bool, u64)> {
    let risk = match prediction["project_max_risk"].as_str() {
        Some("read_only") => RiskTier::ReadOnly,
        Some("reversible") => RiskTier::Reversible,
        Some("guarded") => RiskTier::Guarded,
        Some("irreversible") => RiskTier::Irreversible,
        _ => {
            return Err(invariant(
                "available project forecast omitted its aggregate risk",
            ));
        }
    };
    let checkpoint = prediction["project_requires_checkpoint"]
        .as_bool()
        .ok_or_else(|| invariant("available project forecast omitted its checkpoint burden"))?;
    let actions = prediction["project_action_count"]
        .as_u64()
        .ok_or_else(|| invariant("available project forecast omitted its complete action count"))?;
    if !prediction["project_required_capabilities"]
        .as_array()
        .is_some_and(|values| values.iter().all(Value::is_string))
    {
        return Err(invariant(
            "available project forecast omitted its complete capability requirements",
        ));
    }
    Ok((risk, checkpoint, actions))
}

fn invariant(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InternalInvariantViolation, message)
}

/// Compare the exact original semantic objective before compiling any stage.
/// Different setup and acquisition policies remain in each candidate's complete
/// source/digest, but action-derived IDs do not define project equivalence.
pub(super) fn compare(
    adapter: &MemoryAdapter,
    context: &OperationContext,
    comparison: &Comparison,
    started: Instant,
) -> Result<Value> {
    let snapshot = adapter.snapshot();
    authorize(snapshot, context)?;
    check_wall(context, started)?;
    let mut budget = ForecastBudget::new_project(context, started, comparison.horizon_ticks)?;
    let source_work = budget.admit_source(snapshot, context)?;
    let mut candidates: Vec<_> = comparison.candidates.iter().collect();
    candidates.sort_by(|left, right| left.key.cmp(&right.key));
    let identity = json!({
        "schema":"dfmcp.lab-project-comparison-fork/1",
        "anchor":super::super::anchor_json(&snapshot.anchor()),
        "candidates":candidates,
    });
    let intent_id = IntentId::new(
        Digest32::of_bytes(identity.to_string().as_bytes())
            .first_u128()
            .max(1),
    );
    let mut rows = Vec::with_capacity(candidates.len());
    let mut sources = Vec::with_capacity(candidates.len());
    let mut common_goal: Option<Value> = None;
    let mut common_reserve: Option<Option<u64>> = None;
    for candidate in candidates {
        check_wall(context, started)?;
        let parsed = candidate.source().and_then(|source| {
            let normalized = candidate.normalized(&source)?;
            let project = project_source(&source)?;
            let semantic_goal = project.semantic_key(snapshot)?;
            let reserve_until = reserve_until(&source)?;
            Ok((
                Source {
                    candidate: normalized,
                    source,
                    project,
                    reserve_until,
                },
                semantic_goal,
            ))
        });
        let (source, goal) = match parsed {
            Ok(value) => value,
            Err(error) => {
                candidate_failure(&mut rows, &candidate.key, "source", error)?;
                continue;
            }
        };
        if common_goal
            .as_ref()
            .is_some_and(|existing| existing != &goal)
            || common_reserve.is_some_and(|existing| existing != source.reserve_until)
        {
            return Err(invalid(
                "complete-project candidates must share the same original output quotas and reserve frontier, or complete room layout, furnishings and access goal",
            ));
        }
        common_goal = Some(goal);
        common_reserve = Some(source.reserve_until);
        sources.push(source);
    }
    // This is deliberately a separate pass. A valid but unequal objective may
    // not be hidden behind a later compilation or preparation failure.
    let mut prepared: Vec<(Source, PreparedPlan)> = Vec::with_capacity(sources.len());
    let mut compiled_initial_actions = 0_usize;
    for source in sources {
        check_wall(context, started)?;
        let intent = match source.source.compile(intent_id, snapshot) {
            Ok((intent, _)) => intent,
            Err(error) => {
                candidate_failure(&mut rows, &source.candidate.key, "compile", error)?;
                continue;
            }
        };
        compiled_initial_actions =
            compiled_initial_actions.saturating_add(intent.requested_actions.len());
        if compiled_initial_actions > context.budget.max_actions as usize {
            return Err(exhausted(
                "project candidate compilation exceeds the aggregate action allowance",
            ));
        }
        match StaticPlanner::default().prepare_laboratory(snapshot, &intent, context) {
            Ok(plan) => prepared.push((source, plan)),
            Err(error) => candidate_failure(&mut rows, &source.candidate.key, "prepare", error)?,
        }
    }
    let mut ranking = Vec::new();
    let mut evidence_complete = rows.is_empty();
    for (source, plan) in prepared {
        check_wall(context, started)?;
        let prediction =
            forecast_project_bounded(adapter, &source.project, &plan, context, &mut budget)?;
        let available = prediction["available"] == true;
        let reserve = &prediction["reserve_forecast"];
        let reserve_known = source.reserve_until.is_none()
            || (reserve["available"] == true
                && matches!(
                    reserve["predicted_goal_truth"].as_str(),
                    Some("true" | "false")
                ));
        let reserve_complete = source.reserve_until.is_none()
            || (reserve["available"] == true && reserve["predicted_goal_truth"] == "true");
        let known_goal = matches!(
            prediction["predicted_goal_truth"].as_str(),
            Some("true" | "false")
        );
        evidence_complete &= available && known_goal && reserve_known;
        if available {
            let (risk, checkpoint, actions) = project_cost(&prediction)?;
            if prediction["predicted_complete"] == true && reserve_complete {
                let completion_tick = prediction["predicted_completion_tick"]
                    .as_u64()
                    .ok_or_else(|| {
                        invariant("complete project forecast omitted its completion tick")
                    })?;
                ranking.push((
                    risk,
                    checkpoint,
                    completion_tick,
                    actions,
                    source.candidate.key.clone(),
                ));
            }
        }
        // Preserve the complete staged forecast. The common publisher either
        // publishes every decision-bearing field or refuses before any state
        // change; a compact current-stage projection would hide the new work.
        rows.push(json!({
            "key":source.candidate.key,
            "status":if available { "evaluated" } else { "unavailable" },
            "epistemic_state":"predicted",
            "candidate_digest":candidate_digest(snapshot, &source.candidate)?,
            "summary":plan.summary,
            "goal_scope":"complete_original_project",
            "forecast_scope":"complete_project",
            "max_risk":prediction["project_max_risk"],
            "requires_checkpoint":prediction["project_requires_checkpoint"],
            "action_count":prediction["project_action_count"],
            "required_capabilities":prediction["project_required_capabilities"],
            "cost_scope":"all_simulated_project_stages",
            "current_stage":{
                "action_count":plan.steps.len(),
                "max_risk":plan.max_risk.as_str(),
                "requires_checkpoint":plan.requires_checkpoint,
                "required_capabilities":plan.required_capabilities.iter().map(|capability| capability.as_str()).collect::<Vec<_>>(),
            },
            "prediction":prediction,
            "selection_available":true,
            "next_step":selection_step(snapshot, context, &source.candidate)?,
        }));
    }
    check_wall(context, started)?;
    rows.sort_by(|left, right| left["key"].as_str().cmp(&right["key"].as_str()));
    ranking.sort();
    let recommendation = evidence_complete
        .then(|| ranking.first().map(|row| row.4.clone()))
        .flatten();
    let reason = if !evidence_complete {
        "at least one project lacks a complete comparable prediction or requested reserve frontier; no recommendation is justified"
    } else if recommendation.is_none() {
        "no project predicts its complete original goal, verified stage actions and physical quiescence within the common horizon"
    } else {
        "among complete project predictions, minimize aggregate risk, checkpoint burden, completion tick, total action count, then lexical key"
    };
    let goal_digest = common_goal
        .as_ref()
        .map(|goal| Digest32::of_bytes(goal.to_string().as_bytes()).to_hex());
    Ok(json!({
        "ok":true,
        "schema":"dfmcp.plan-comparison/2",
        "kind":"compare_candidates",
        "mode":"compare_candidates",
        "version":2,
        "session_id":context.session_id.to_string(),
        "anchor":super::super::anchor_json(&snapshot.anchor()),
        "epistemic_state":"predicted",
        "method":"same_original_project_on_discarded_laboratory_forks",
        "forecast_scope":"complete_project",
        "goal_scope":"complete_original_project",
        "recommendation_scope":"complete_project",
        "assumptions":"deterministic laboratory effects; no other agent acts; each predicted next stage is compiled from the preceding quiet verified fork and reviewed without delay; other retained plans' deferred actions are not dispatched; actual future stages still require separate current review and commit",
        "horizon_ticks":comparison.horizon_ticks,
        "semantic_project_goal":common_goal,
        "semantic_project_goal_digest":goal_digest,
        "reserve_until_tick":common_reserve.flatten(),
        "candidates":rows,
        "recommended_candidate":recommendation,
        "recommendation":{"policy":PROJECT_POLICY,"reason":reason,"evidence_complete":evidence_complete},
        "source_work":source_work,
        "compiled_initial_actions":compiled_initial_actions,
        "forecast_work":budget.work_json(),
        "effect_evidence":false,
        "pending_plan_changed":false,
        "automatic_continuation":false,
        "selection_scope":"current_reviewed_stage_only",
        "note":"selection revalidates the exact candidate source and prepares only its current executable stage; predicted later stages never become pending plans, goal evidence or effect authority",
    }))
}
