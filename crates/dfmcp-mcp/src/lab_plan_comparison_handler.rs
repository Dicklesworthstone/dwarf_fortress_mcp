//! Own a laboratory candidate request through complete response publication.
//! Forecasts are discarded. Only successful selection installs a pending plan.

use super::*;
use std::time::{Duration, Instant};

fn check(current: &OperationContext, started: Instant) -> Result<()> {
    for capability in [Capability::Observe, Capability::Query, Capability::Plan] {
        authorize_entry(current, capability, RiskTier::ReadOnly)?;
    }
    if started.elapsed() >= Duration::from_millis(current.budget.max_wall_millis) {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "candidate planning exhausted its complete request wall-time budget",
        ));
    }
    Ok(())
}

fn remaining(current: &OperationContext, started: Instant) -> Result<OperationContext> {
    check(current, started)?;
    let spent = started.elapsed().as_nanos().div_ceil(1_000_000);
    if spent >= u128::from(current.budget.max_wall_millis) {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "candidate planning has no wall-time allowance remaining",
        ));
    }
    let mut remaining = current.clone();
    remaining.budget.max_wall_millis -= spent as u64;
    Ok(remaining)
}

fn complete_query_work(current: &OperationContext, started: Instant) -> Result<serde_json::Value> {
    let baselines = temporal_query::retained_baseline_work(&remaining(current, started)?)?;
    let watches = temporal_query::retained_watch_work(&remaining(current, started)?)?;
    Ok(json!({
        "query_baselines": baselines,
        "query_watches": watches["query_watches"],
        "condition_watches": watches["condition_watches"],
    }))
}

pub(super) fn execute(
    session_id: Option<String>,
    request: &plan_comparison::Request,
    started: Instant,
) -> Result<String> {
    let session = resolve_session(session_id)?;
    let _admission = LAB_ADMISSION.read().map_err(|_| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "laboratory admission lock poisoned",
        )
    })?;
    ensure_durable_owner(&session)?;
    with_session_view(
        &session,
        || {
            Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "candidate planning session poisoned",
            ))
        },
        |guard| execute_owned(guard, request, started),
        false,
    )
}

/// New stock-objective intake shares complete publication with candidate
/// selection. A refused response leaves the previously reviewed plan intact.
pub(super) fn execute_resource_acquisition(
    session_id: Option<String>,
    summary: Option<String>,
    raw: &str,
    started: Instant,
) -> Result<String> {
    let request = resource_acquisition::ResourceAcquisitionRequest::parse_current(raw)?;
    let summary = summary.unwrap_or_default();
    if summary.len() > MAX_SUMMARY_BYTES {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "resource acquisition summary exceeds its explicit byte bound",
        ));
    }
    let source = PlanSource::Blueprint {
        summary,
        raw: request.canonical_json(),
    };
    execute_original_source(session_id, source, "resource_acquisition", started)
}

/// The original output objective owns both stages. Only the stage compiled
/// from current observed input stock is proposed; production is never queued
/// behind a predicted harvest in this request.
pub(super) fn execute_resource_production(
    session_id: Option<String>,
    summary: Option<String>,
    raw: &str,
    started: Instant,
) -> Result<String> {
    let request = crate::lab_world::ProductionRequest::parse_current(raw)?;
    if !request.is_resource_aware() {
        return Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "resource production requires the fixed resource-aware policy",
        ));
    }
    let summary = summary.map_or_else(|| "meet production quotas".to_owned(), |value| value);
    if summary.len() > MAX_SUMMARY_BYTES {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "resource production summary exceeds its explicit byte bound",
        ));
    }
    let source = PlanSource::Production {
        summary,
        raw: request.canonical_json(),
    };
    execute_original_source(session_id, source, "resource_production", started)
}

/// A room source keeps its complete geometry, furnishings and access request
/// while the compiler chooses one currently funded stage.
pub(super) fn execute_resource_room(
    session_id: Option<String>,
    summary: Option<String>,
    raw: &str,
    started: Instant,
) -> Result<String> {
    let summary = summary.unwrap_or_default();
    if summary.len() > MAX_SUMMARY_BYTES {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "resource-aware room summary exceeds its explicit byte bound",
        ));
    }
    let source = if connected_room::is_connected_room(raw) {
        let request = connected_room::ConnectedRoomRequest::parse_current(raw)?;
        PlanSource::ConnectedRoom {
            summary,
            raw: request.canonical_json(),
        }
    } else {
        let request = room_provisioning::ProvisionedRoomRequest::parse_current(raw)?;
        PlanSource::Blueprint {
            summary,
            raw: request.canonical_json(),
        }
    };
    if !source.is_resource_aware_room() {
        return Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "resource-aware room intake requires its original acquisition policy and fixed compiler",
        ));
    }
    execute_original_source(session_id, source, "resource_room", started)
}

fn execute_original_source(
    session_id: Option<String>,
    source: PlanSource,
    kind: &str,
    started: Instant,
) -> Result<String> {
    let session = resolve_session(session_id)?;
    let _admission = LAB_ADMISSION.read().map_err(|_| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "laboratory admission lock poisoned",
        )
    })?;
    ensure_durable_owner(&session)?;
    with_session_view(
        &session,
        || {
            Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "resource acquisition session poisoned",
            ))
        },
        |session| {
            let (rid, current) = next_context(session)?;
            check(&current, started)?;
            let snapshot = session.adapter.snapshot();
            let source_work =
                query_budget::admit_sources(&[snapshot], &remaining(&current, started)?)?;
            let (mut payload, pending) =
                prepare_source(session, source, rid, &current, source_work, kind, started)?;
            let forecast = pending.source.forecast(
                &session.adapter,
                &pending.plan,
                &remaining(&current, started)?,
            );
            payload["forecast"] = plan_comparison::compact_prediction(&forecast);
            publish_owned(session, &current, started, payload, Some(pending))
        },
        false,
    )
}

/// A supply-dependent continuation holds the same owner through current
/// lineage, complete source admission and response publication. Other retained
/// goal families keep their established intake path and authority.
pub(super) fn execute_resource_continuation(
    session_id: Option<String>,
    summary: Option<String>,
    parent: Digest32,
    started: Instant,
) -> Result<Option<String>> {
    if summary
        .as_ref()
        .is_some_and(|summary| summary.len() > MAX_SUMMARY_BYTES)
    {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "resource continuation summary exceeds its explicit byte bound",
        ));
    }
    let session = resolve_session(session_id)?;
    let _admission = LAB_ADMISSION.read().map_err(|_| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "laboratory admission lock poisoned",
        )
    })?;
    ensure_durable_owner(&session)?;
    with_session_view(
        &session,
        || {
            Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "resource continuation session poisoned",
            ))
        },
        |session| {
            if !session.objectives.iter().any(|objective| {
                objective.plan_digest == parent
                    && (objective.is_resource_acquisition()
                        || objective.is_resource_aware_production()
                        || objective.is_resource_aware_room())
            }) {
                return Ok(None);
            }
            let (rid, current) = next_context(session)?;
            check(&current, started)?;
            let source_work = query_budget::admit_sources(
                &[session.adapter.snapshot()],
                &remaining(&current, started)?,
            )?;
            let source = objectives::continuation_source(session, parent, summary, None)?;
            if !source.requires_complete_source_review() {
                return Err(DfmcpError::new(
                    ErrorCode::InternalInvariantViolation,
                    "resource continuation did not retain its original source family",
                ));
            }
            let kind = if source.is_resource_aware_room() {
                "resource_room_continuation"
            } else if source.is_resource_aware_production() {
                "resource_production_continuation"
            } else {
                "resource_acquisition_continuation"
            };
            let (mut payload, pending) =
                prepare_source(session, source, rid, &current, source_work, kind, started)?;
            let forecast = pending.source.forecast(
                &session.adapter,
                &pending.plan,
                &remaining(&current, started)?,
            );
            payload["forecast"] = plan_comparison::compact_prediction(&forecast);
            publish_owned(session, &current, started, payload, Some(pending)).map(Some)
        },
        false,
    )
}

fn prepare_source(
    session: &LabSession,
    source: PlanSource,
    rid: u128,
    current: &OperationContext,
    source_work: serde_json::Value,
    kind: &str,
    started: Instant,
) -> Result<(serde_json::Value, PendingPlan)> {
    check(current, started)?;
    let snapshot = session.adapter.snapshot();
    let (intent, analysis) = source.compile(IntentId::new(rid), snapshot)?;
    check(current, started)?;
    let plan = StaticPlanner::default().prepare_laboratory(snapshot, &intent, current)?;
    check(current, started)?;
    let digest = plan.digest.to_string();
    let acquisition = source.is_resource_acquisition();
    let resource_production = source.is_resource_aware_production();
    let resource_room = source.is_resource_aware_room();
    let mut result = json!({
        "kind": kind,
        "schema": if acquisition { "dfmcp.resource-acquisition/1" } else if resource_production { "dfmcp.resource-production/1" } else if resource_room { "dfmcp.resource-room/1" } else { "dfmcp.plan-comparison/1" },
        "version": 1,
        "plan_id": plan.id.to_string(),
        "plan_digest": digest,
        "terminal_condition": format!("{:?}", intent.terminal_condition),
        "max_risk": plan.max_risk.as_str(),
        "required_capabilities": plan.required_capabilities.iter()
            .map(|capability| capability.as_str()).collect::<Vec<_>>(),
        "requires_checkpoint": plan.requires_checkpoint,
        "expires_at_tick": plan.expires_at_tick.0,
        "steps": crate::lab_world::plan_steps_json(&plan),
        "production": if acquisition || resource_room { None } else { analysis.as_ref() },
        "resource_acquisition": if acquisition { analysis.as_ref() } else { None },
        "live_routing": live_routing_json(&plan),
        "source_work": source_work,
        "forecast": null,
        "effect_evidence": false,
        "pending_plan_changed": true,
        "next_step": {"tool": "fortress.commit", "arguments": {
            "session_id": session.session_id.to_string(), "plan_digest": digest,
        }},
        "note": "freshly prepared ordinary plan; commit still revalidates current authority, source and registered effect policy",
    });
    if resource_room {
        result["room"] = analysis
            .as_ref()
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        result["goal_scope"] = result["room"]["goal_scope"].clone();
        result["forecast_scope"] = json!("current_reviewed_stage_only");
    }
    if resource_room || resource_production {
        result["schema"] = json!(if resource_room {
            "dfmcp.resource-room/2"
        } else {
            "dfmcp.resource-production/2"
        });
        result["version"] = json!(2);
        crate::sealed_plan_review::apply(&mut result, &plan)?;
    }
    Ok((
        result,
        PendingPlan {
            plan,
            digest,
            source,
        },
    ))
}

fn execute_owned(
    session: &mut LabSession,
    request: &plan_comparison::Request,
    started: Instant,
) -> Result<String> {
    let (rid, current) = next_context(session)?;
    check(&current, started)?;
    let mut selected = None;
    let payload = match request {
        plan_comparison::Request::Compare(_) => plan_comparison::compare(
            &session.adapter,
            &current,
            IntentId::new(rid),
            request,
            started,
        )?,
        plan_comparison::Request::Select(_) => {
            let snapshot = session.adapter.snapshot();
            let source_work =
                query_budget::admit_sources(&[snapshot], &remaining(&current, started)?)?;
            let source = plan_comparison::selected_source(request, snapshot, &current)?
                .ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::InternalInvariantViolation,
                        "selected candidate did not yield a plan source",
                    )
                })?;
            let (result, pending) = prepare_source(
                session,
                source,
                rid,
                &current,
                source_work,
                "select_candidate",
                started,
            )?;
            selected = Some(pending);
            result
        }
    };
    publish_owned(session, &current, started, payload, selected)
}

fn publish_owned(
    session: &mut LabSession,
    current: &OperationContext,
    started: Instant,
    mut payload: serde_json::Value,
    selected: Option<PendingPlan>,
) -> Result<String> {
    payload["ok"] = json!(true);
    payload["session_id"] = json!(session.session_id.to_string());
    payload["anchor"] = anchor_json(&current.anchor);
    check(current, started)?;
    let mut projection = temporal_handler::projection(session, current)?;
    // Recovered work has no live action handle. Keep its retained descriptors
    // visible without sampling or rewriting historical proof during comparison.
    // A terminal goal can still own active or unknown physical work.
    projection.active_work["carried_obligations"] = json!(
        session
            .carried
            .iter()
            .filter(|carried| !carried.is_final())
            .map(CarriedStep::to_json)
            .collect::<Vec<_>>()
    );
    let query_work = complete_query_work(current, started)?;
    let selected_pending = selected.as_ref().map(|pending| {
        plan_comparison_response::pending_plan_entry(
            &pending.plan,
            session.session_id,
            pending.source.requires_complete_source_review(),
        )
    });
    let output =
        plan_comparison_response::finish(&projection, payload, query_work, selected_pending)?;
    // The session and shared-world owner remain held through this final check.
    // Nothing fallible may replace the accepted packet after pending changes.
    check(current, started)?;
    if let Some(pending) = selected {
        session.pending = Some(pending);
    }
    Ok(output)
}
