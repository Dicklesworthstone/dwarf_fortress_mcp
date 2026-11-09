//! Qualify advisory remedies against the current session's real authority.
//!
//! These checks neither prepare nor publish a plan. The ordinary plan/commit
//! path still revalidates facts, budgets, leases and effects. Capability names
//! in a presentation cache are not enough to establish risk, scope or expiry.

use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{Capability, DfmcpError, ErrorCode, IntentId, OperationContext, Result, RiskTier};
use dfmcp_intent::{Intent, StaticPlanner};
use dfmcp_world::WorldSnapshot;
use serde_json::{Map, Value, json};

use super::{PlanSource, civilian_recovery};

pub(super) fn qualify(
    snapshot: &WorldSnapshot,
    recovery: &civilian_recovery::CivilianRecovery,
    context: &OperationContext,
    alerts: &mut [Value],
) {
    // Several hostiles can share one recruitment remedy. Lower identical
    // advice only once within this immutable observation, never across anchors.
    let mut assessed = BTreeMap::<String, Map<String, Value>>::new();
    for alert in alerts {
        let Some(remedy) = alert.get_mut("remedy").filter(|value| value.is_object()) else {
            continue;
        };
        let fields = assessed
            .entry(remedy.to_string())
            .or_insert_with(|| assessment(context, lower(snapshot, recovery, context, remedy)));
        if let Some(remedy) = remedy.as_object_mut() {
            remedy.extend(fields.clone());
        }
    }
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
            raw: raw.to_owned(),
        }
    } else if let Some(paused_target) = arguments.get("paused_target").and_then(Value::as_bool) {
        PlanSource::Pause {
            summary,
            paused_target,
        }
    } else if let Some(raw) = arguments.get("blueprint").and_then(Value::as_str) {
        // Objective lowering reads the complete relevant world domain. An
        // observation's small output allowance does not authorize a larger read.
        if snapshot.graph.entities.len() > context.budget.max_entities as usize {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "world remedy objective exceeds the current entity allowance",
            ));
        }
        if crate::lab_world::is_production_objective(raw) {
            PlanSource::Production {
                summary,
                raw: crate::lab_world::ProductionRequest::parse_current(raw)?.canonical_json(),
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

fn assessment(context: &OperationContext, intent: Result<Intent>) -> Map<String, Value> {
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
        && intent.requested_actions.iter().all(|step| {
            dfmcp_intent::effects::default_compensation(&step.action).is_some_and(|inverse| {
                step.compensation
                    .as_ref()
                    .is_none_or(|compensation| *compensation == inverse)
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
