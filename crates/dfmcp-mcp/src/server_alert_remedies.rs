//! Qualify advisory remedies against the current session's real authority.
//!
//! These checks neither prepare nor publish a plan. The ordinary plan/commit
//! path still revalidates facts, budgets, leases and effects. Capability names
//! in a presentation cache are not enough to establish risk, scope or expiry.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use dfmcp_adapter::GameAdapter;
use dfmcp_core::{Capability, DfmcpError, ErrorCode, IntentId, OperationContext, Result, RiskTier};
use dfmcp_intent::{Intent, StaticPlanner};
use dfmcp_world::{PredicateEvidence, WorldSnapshot};
use serde_json::{Map, Value, json};

use super::{LabSession, PlanSource, civilian_recovery, goal_continuation, objectives};

pub(super) fn qualify(
    snapshot: &WorldSnapshot,
    recovery: &civilian_recovery::CivilianRecovery,
    context: &OperationContext,
    alerts: &mut [Value],
    owner: Option<&LabSession>,
) {
    // Several hostiles can share one recruitment remedy. Lower identical
    // advice only once within this immutable observation, never across anchors.
    let mut assessed = BTreeMap::<String, Map<String, Value>>::new();
    let started = Instant::now();
    for alert in alerts {
        let Some(remedy) = alert.get_mut("remedy").filter(|value| value.is_object()) else {
            continue;
        };
        let fields = assessed.entry(remedy.to_string()).or_insert_with(|| {
            let intent = remaining_context(context, started)
                .and_then(|current| match owner {
                    Some(session) => lower_owned(session, &current, remedy),
                    None => lower(snapshot, recovery, &current, remedy),
                })
                .and_then(|intent| remaining_context(context, started).map(|_| intent));
            let mut fields = assessment(snapshot, context, intent);
            if remedy["goal_scope"] == "joint_supply_recovery" {
                fields.insert(
                    "availability_scope".to_owned(),
                    json!("current_reviewed_stage_only"),
                );
                if let Some(capabilities) = fields
                    .get("required_capabilities")
                    .and_then(Value::as_array)
                {
                    let mut required: BTreeSet<String> = capabilities
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect();
                    required.insert(Capability::Observe.as_str().to_owned());
                    if remedy["retained_goal"]["requires_query"] == true
                        || remedy["arguments"]["blueprint"]
                            .as_str()
                            .is_some_and(|raw| {
                                crate::lab_world::ProductionRequest::parse_current(raw)
                                    .is_ok_and(|request| request.is_resource_aware())
                            })
                    {
                        required.insert(Capability::Query.as_str().to_owned());
                    }
                    fields.insert("required_capabilities".to_owned(), json!(required));
                }
            }
            if let Err(error) = remaining_context(context, started) {
                fields.insert("available".to_owned(), json!(false));
                fields.insert("unavailable_reason".to_owned(), json!(error.to_string()));
            }
            fields
        });
        if let Some(remedy) = remedy.as_object_mut() {
            remedy.extend(fields.clone());
        }
    }
}

/// An already reviewed supply objective takes precedence over a fresh root.
/// Selection reuses the owner's ordinary continuation gate; lowering and
/// effect authority still apply to that exact source, without widening it.
fn lower_owned(
    session: &LabSession,
    context: &OperationContext,
    remedy: &mut Value,
) -> Result<Intent> {
    let snapshot = session.adapter.snapshot();
    if context.session_id != session.session_id || context.anchor != snapshot.anchor() {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "owned supply advice requires this session's exact current assessment context",
        )
        .retryable(true));
    }
    if remedy["goal_scope"] != "joint_supply_recovery" {
        return lower(snapshot, &session.civilian_recovery, context, remedy);
    }
    if remedy["tool"] != "fortress.plan"
        || !remedy["arguments"]
            .as_object()
            .is_some_and(|arguments| arguments.len() == 1)
    {
        return Err(invalid(
            "supply recovery advice requires one registered planning source",
        ));
    }
    let raw = remedy["arguments"]["blueprint"]
        .as_str()
        .ok_or_else(|| invalid("supply recovery advice lost its production source"))?;
    let selected = if goal_continuation::is_request(raw) {
        let parent = goal_continuation::parse_request(raw)?;
        Some(objectives::continuation_source(
            session, parent, None, None,
        )?)
    } else if session.objectives.is_empty() {
        None
    } else {
        let proposed = crate::lab_world::ProductionRequest::parse_current(raw)?;
        objectives::supply_recovery_continuation(session, &proposed, context)?
    };
    let Some(selected) = selected else {
        return lower(snapshot, &session.civilian_recovery, context, remedy);
    };
    let PlanSource::ProductionContinuation { raw, .. } = &selected else {
        return Err(invalid(
            "supply advice selected a non-production continuation",
        ));
    };
    let source = goal_continuation::ProductionContinuation::parse(raw)?;
    let requires_query = source.is_resource_aware() || source.is_current_consumption_reserve();
    // Publish the exact retained identity even when a current effect grant is
    // missing. That refusal must not fall back to a new, broader source policy.
    remedy["arguments"] = json!({"blueprint":json!({
        "template":"continue_goal", "plan_digest":source.parent.to_hex(),
    }).to_string()});
    remedy["retained_goal"] = source.lineage_json();
    remedy["retained_goal"]["original_request"] =
        serde_json::from_str(&source.request().canonical_json())
            .map_err(|_| invalid("retained production request is not canonical JSON"))?;
    remedy["retained_goal"]["requires_query"] = json!(requires_query);
    remedy["separate_stage_review_required"] = json!(true);
    remedy["then"] = json!(
        "review this continuation of the retained original output goal, then explicitly commit the new stage; original quotas, policies and history remain unchanged"
    );
    super::query_budget::admit_observation_sources(&[snapshot], context)?;
    if requires_query {
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    }
    selected
        .compile(IntentId::new(1), snapshot)
        .map(|(intent, _)| intent)
}

fn remaining_context(context: &OperationContext, started: Instant) -> Result<OperationContext> {
    let spent = started.elapsed().as_millis();
    if spent >= u128::from(context.budget.max_wall_millis) {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "world remedy assessment exceeded the shared wall-time allowance",
        ));
    }
    let mut remaining = context.clone();
    remaining.budget.max_wall_millis -= spent as u64;
    Ok(remaining)
}

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

/// Accept only the request forms actually emitted by world-owned alerts, and
/// reuse their normal lowering rather than maintaining a second action model.
fn lower(
    snapshot: &WorldSnapshot,
    recovery: &civilian_recovery::CivilianRecovery,
    context: &OperationContext,
    remedy: &Value,
) -> Result<Intent> {
    if context.anchor != snapshot.anchor() {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "world remedy authority and source snapshot must share the current anchor",
        )
        .retryable(true));
    }
    if remedy["tool"] != "fortress.plan" {
        return Err(invalid(
            "world remedy does not name the registered planning tool",
        ));
    }
    let arguments = remedy["arguments"]
        .as_object()
        .filter(|arguments| arguments.len() == 1)
        .ok_or_else(|| invalid("world remedy must name one supported planning source"))?;
    let summary = "review world-alert remedy".to_owned();
    let source = if let Some(raw) = arguments.get("actions").and_then(Value::as_str) {
        PlanSource::Actions {
            summary,
            raw: crate::lab_world::canonical_current_actions(raw)?,
        }
    } else if let Some(paused_target) = arguments.get("paused_target").and_then(Value::as_bool) {
        PlanSource::Pause {
            summary,
            paused_target,
        }
    } else if let Some(raw) = arguments.get("blueprint").and_then(Value::as_str) {
        // Objective lowering reads the complete relevant world domain. An
        // observation's small output allowance does not authorize a larger read.
        super::query_budget::admit_observation_sources(&[snapshot], context)?;
        if crate::lab_world::is_production_objective(raw) {
            let request = crate::lab_world::ProductionRequest::parse_current(raw)?;
            if request.is_resource_aware() {
                // A resource project inspects the whole input/workload domain,
                // just as its ordinary Plan entry point does. Observation
                // advice cannot borrow a Query grant for that extra analysis.
                context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
            }
            PlanSource::Production {
                summary,
                raw: request.canonical_json(),
            }
        } else if civilian_recovery::is_release_request(raw) {
            let burrow = civilian_recovery::parse_release_request(raw)?;
            let source = recovery.release_source(snapshot, burrow)?;
            source.validate_budget(snapshot, context)?;
            PlanSource::CivilianRelease {
                summary,
                raw: source.canonical_json(),
            }
        } else {
            return Err(invalid(
                "world remedy blueprint has no registered advisory lowering",
            ));
        }
    } else {
        return Err(invalid("world remedy has no supported planning source"));
    };
    // A pure, local identity suffices for lowering. No request counter, pending
    // plan, lease, adapter state or capability use is consumed by this read.
    source
        .compile(IntentId::new(1), snapshot)
        .map(|(intent, _)| intent)
}

fn assessment(
    snapshot: &WorldSnapshot,
    context: &OperationContext,
    intent: Result<Intent>,
) -> Map<String, Value> {
    let mut fields = Map::new();
    fields.insert("available".to_owned(), json!(false));
    fields.insert("risk".to_owned(), Value::Null);
    fields.insert("reversibility".to_owned(), json!("unknown"));
    fields.insert("requires_checkpoint".to_owned(), Value::Null);
    fields.insert("requires_confirmation".to_owned(), json!(false));
    fields.insert(
        "availability_scope".to_owned(),
        json!("current_session_authority"),
    );
    let intent = match intent {
        Ok(intent) => intent,
        Err(error) => {
            fields.insert("unavailable_reason".to_owned(), json!(error.to_string()));
            return fields;
        }
    };
    let risk = intent
        .requested_actions
        .iter()
        .map(|step| step.action.risk())
        .max()
        .unwrap_or(RiskTier::ReadOnly);
    let requires_checkpoint = intent.requires_checkpoint()
        || risk
            >= StaticPlanner::default()
                .policy
                .require_checkpoint_at_or_above;
    fields.insert("risk".to_owned(), json!(risk.as_str()));
    let reversible = !intent.requested_actions.is_empty()
        && PredicateEvidence::laboratory(snapshot).is_ok_and(|evidence| {
            intent.requested_actions.iter().all(|step| {
                dfmcp_intent::effects::default_compensation_with_evidence(&step.action, &evidence)
                    .is_ok_and(|inverse| {
                        inverse.is_some_and(|inverse| {
                            step.compensation
                                .as_ref()
                                .is_none_or(|compensation| *compensation == inverse)
                        })
                    })
            })
        });
    fields.insert(
        "reversibility".to_owned(),
        json!(if reversible {
            "reversible"
        } else {
            "not_compensable"
        }),
    );
    fields.insert("requires_checkpoint".to_owned(), json!(requires_checkpoint));
    let mut capabilities = BTreeSet::from([Capability::Plan]);
    for step in &intent.requested_actions {
        capabilities.insert(step.action.capability());
        if let Some(compensation) = &step.compensation {
            capabilities.insert(compensation.capability());
        }
    }
    if requires_checkpoint {
        capabilities.insert(Capability::Checkpoint);
    }
    fields.insert(
        "required_capabilities".to_owned(),
        json!(
            capabilities
                .into_iter()
                .map(Capability::as_str)
                .collect::<Vec<_>>()
        ),
    );
    match authorize(context, &intent, requires_checkpoint) {
        Ok(()) => {
            fields.insert("available".to_owned(), json!(true));
            fields.insert("unavailable_reason".to_owned(), Value::Null);
        }
        Err(error) => {
            fields.insert("unavailable_reason".to_owned(), json!(error.to_string()));
        }
    }
    fields
}

fn authorize(context: &OperationContext, intent: &Intent, requires_checkpoint: bool) -> Result<()> {
    context.authorize(Capability::Plan, RiskTier::ReadOnly, &[], None)?;
    let policy = StaticPlanner::default().policy;
    if intent.requested_actions.is_empty()
        || intent.requested_actions.len()
            > context.budget.max_actions.min(policy.max_steps) as usize
    {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "world remedy exceeds the current semantic action allowance",
        ));
    }
    for step in &intent.requested_actions {
        for action in std::iter::once(&step.action).chain(step.compensation.iter()) {
            let scope = action.scope();
            if scope.entity_ids.len()
                > context
                    .budget
                    .max_entities
                    .min(policy.max_entities_per_action) as usize
            {
                return Err(DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    "world remedy exceeds the current action entity allowance",
                ));
            }
            context.authorize(
                action.capability(),
                action.risk(),
                &scope.entity_ids,
                scope.map_area,
            )?;
        }
    }
    if requires_checkpoint {
        context.authorize(Capability::Checkpoint, RiskTier::Guarded, &[], None)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "server_alert_remedies_tests.rs"]
mod tests;
