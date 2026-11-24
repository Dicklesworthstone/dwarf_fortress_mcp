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

fn project_cost(prediction: &Value, portfolio: bool) -> Result<(RiskTier, bool, u64)> {
    let (risk, checkpoint, actions, capabilities) = if portfolio {
        (
            &prediction["portfolio_max_risk"],
            &prediction["portfolio_requires_checkpoint"],
            &prediction["portfolio_action_count"],
            &prediction["portfolio_required_capabilities"],
        )
    } else {
        (
            &prediction["project_max_risk"],
            &prediction["project_requires_checkpoint"],
            &prediction["project_action_count"],
            &prediction["project_required_capabilities"],
        )
    };
    let risk = match risk.as_str() {
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
    let checkpoint = checkpoint
        .as_bool()
        .ok_or_else(|| invariant("available project forecast omitted its checkpoint burden"))?;
    let actions = actions
        .as_u64()
        .ok_or_else(|| invariant("available project forecast omitted its complete action count"))?;
    if !capabilities
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
    owned: Option<&owned_progression::OwnedWork>,
    peers: Option<&super::super::original_forecast::AdmittedProjectSet<'_>>,
) -> Result<Value> {
    let snapshot = adapter.snapshot();
    let portfolio = peers.is_some();
    if portfolio != comparison.competing_plan_digests.is_some() || (portfolio && owned.is_none()) {
        return Err(invalid(
            "complete portfolio comparison lost its admitted peer selection",
        ));
    }
    authorize(snapshot, context)?;
    check_wall(context, started)?;
    let mut budget = ForecastBudget::new_project(context, started, comparison.horizon_ticks)?;
    let source_work = budget.admit_source(snapshot, context)?;
    let mut candidates: Vec<_> = comparison.candidates.iter().collect();
    candidates.sort_by(|left, right| left.key.cmp(&right.key));
    let mut identity = json!({
        "schema":"dfmcp.lab-project-comparison-fork/1",
        "anchor":super::super::anchor_json(&snapshot.anchor()),
        "candidates":candidates,
    });
    if let Some(peers) = peers {
        identity["schema"] = json!("dfmcp.lab-project-portfolio-comparison-fork/1");
        identity["competing_plan_digests"] = json!(
            peers
                .requested_digests()
                .iter()
                .map(|digest| digest.to_hex())
                .collect::<Vec<_>>()
        );
    }
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
        let prediction = if let (Some(owned), Some(peers)) = (owned, peers) {
            owned.forecast_project_with_peers(
                adapter,
                &source.project,
                &plan,
                context,
                source.reserve_until,
                started,
                &mut budget,
                peers,
            )?
        } else if let Some(owned) = owned {
            owned.forecast_project(
                adapter,
                &source.project,
                &plan,
                context,
                source.reserve_until,
                started,
                &mut budget,
            )?
        } else {
            forecast_project_bounded(adapter, &source.project, &plan, context, &mut budget)?
        };
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
        evidence_complete &= available
            && known_goal
            && reserve_known
            && (!portfolio || prediction["portfolio_evidence_complete"] == true);
        if available {
            let (risk, checkpoint, actions) = project_cost(&prediction, portfolio)?;
            let complete = if portfolio {
                prediction["predicted_portfolio_complete"] == true
            } else {
                prediction["predicted_complete"] == true
            };
            if complete && reserve_complete {
                let completion = if portfolio {
                    &prediction["portfolio_completion_tick"]
                } else {
                    &prediction["predicted_completion_tick"]
                };
                let completion_tick = completion.as_u64().ok_or_else(|| {
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
        let (risk_field, checkpoint_field, actions_field, capabilities_field) = if portfolio {
            (
                "portfolio_max_risk",
                "portfolio_requires_checkpoint",
                "portfolio_action_count",
                "portfolio_required_capabilities",
            )
        } else {
            (
                "project_max_risk",
                "project_requires_checkpoint",
                "project_action_count",
                "project_required_capabilities",
            )
        };
        rows.push(json!({
            "key":source.candidate.key,
            "status":if available { "evaluated" } else { "unavailable" },
            "epistemic_state":"predicted",
            "candidate_digest":candidate_digest(snapshot, &source.candidate)?,
            "summary":plan.summary,
            "goal_scope":"complete_original_project",
            "forecast_scope":"complete_project",
            "max_risk":prediction[risk_field],
            "requires_checkpoint":prediction[checkpoint_field],
            "action_count":prediction[actions_field],
            "required_capabilities":prediction[capabilities_field],
            "cost_scope":if portfolio {
                "candidate_and_hypothetical_peer_stages"
            } else {
                "all_simulated_project_stages"
            },
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
    let reason = if portfolio && !evidence_complete {
        "at least one candidate or selected peer lacks comparable known goal or exact reserve evidence; no portfolio recommendation is justified"
    } else if portfolio && recommendation.is_none() {
        "no alternative predicts all selected original goals, verified stages and physical quiescence together within the common horizon"
    } else if portfolio {
        "among complete selected portfolios, minimize new-stage aggregate risk, checkpoint burden, joint completion tick, new action count, then lexical candidate key"
    } else if !evidence_complete {
        "at least one project lacks a complete comparable prediction or requested reserve frontier; no recommendation is justified"
    } else if recommendation.is_none() {
        "no project predicts its complete original goal, verified stage actions and physical quiescence within the common horizon"
    } else {
        "among complete project predictions, minimize aggregate risk, checkpoint burden, completion tick, total action count, then lexical key"
    };
    let goal_digest = common_goal
        .as_ref()
        .map(|goal| Digest32::of_bytes(goal.to_string().as_bytes()).to_hex());
    let mut result = json!({
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
    });
    if let Some(owned) = owned {
        result["schema"] = json!("dfmcp.plan-comparison/4");
        result["version"] = json!(4);
        result["progression_scope"] = json!("owned_session");
        result["owned_work"] = owned.describe();
        result["method"] =
            json!("same_original_project_with_owned_session_progression_on_discarded_forks");
        result["assumptions"] = json!(
            "every authenticated current-session open action progresses in retained commit order; unfinished recovered peers keep their original cadence; each candidate's next stage is compiled only from a quiet verified fork and reviewed without delay; other originals' future stages and other sessions' deferred dispatch are excluded; actual future stages still require separate current review and commit"
        );
    }
    if let Some(peers) = peers {
        result["schema"] = json!("dfmcp.plan-comparison/5");
        result["version"] = json!(5);
        result["competing_plan_digests"] = json!(
            peers
                .requested_digests()
                .iter()
                .map(|digest| digest.to_hex())
                .collect::<Vec<_>>()
        );
        result["selected_peer_projects"] = peers.describe();
        result["owned_work"]["peer_original_goals_selected"] = json!(true);
        result["recommendation_scope"] = json!("complete_selected_portfolio");
        result["recommendation"]["policy"] = json!(
            "complete_selected_portfolios_then_new_risk_checkpoint_joint_completion_tick_new_action_count_key_v1"
        );
        result["method"] = json!("same_candidate_goal_with_selected_complete_peer_projects");
        result["assumptions"] = json!(
            "the candidate and each explicitly selected committed project review future stages without delay after verified quiet predecessors; all current-session work shares finite resources and authentic recovered cadence; unselected originals contribute committed stages only; other sessions' deferred dispatch is excluded; actual future stages still require separate current review and commit"
        );
        result["note"] = json!(
            "recommendations require the whole selected portfolio to complete; selection prepares only the current candidate stage, and every real peer or candidate continuation still requires ordinary review and Commit"
        );
    }
    Ok(result)
}
