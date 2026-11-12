//! Explicit output-only negotiation for an existing laboratory session.
//!
//! This is the opening issuer's bounded control-plane operation, never a child
//! context widening its own authority. The owner, world, grants and five other
//! work limits stay fixed. Complete acknowledgement precedes the only mutation.

use super::*;
use crate::agent_turn::{
    AgentPhase, AgentTurnBuilder, ContinuityStatus, ObservationProfile, uncertainty,
};
use serde_json::Value;
use std::time::{Duration, Instant};

fn exhausted(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

fn check(context: &OperationContext, started: Instant) -> Result<()> {
    for capability in [Capability::Observe, Capability::Query] {
        authorize_entry(context, capability, RiskTier::ReadOnly)?;
    }
    if started.elapsed() >= Duration::from_millis(context.budget.max_wall_millis) {
        return Err(exhausted(
            "session output negotiation exhausted its complete wall-time allowance",
        ));
    }
    Ok(())
}

fn remaining(context: &OperationContext, started: Instant) -> Result<OperationContext> {
    check(context, started)?;
    let spent = started.elapsed().as_nanos().div_ceil(1_000_000);
    if spent >= u128::from(context.budget.max_wall_millis) {
        return Err(exhausted(
            "session output negotiation has no wall-time allowance remaining",
        ));
    }
    let mut remaining = context.clone();
    remaining.budget.max_wall_millis -= spent as u64;
    Ok(remaining)
}

fn budget_json(budget: WorkBudget) -> Value {
    json!({
        "max_wall_millis": budget.max_wall_millis,
        "max_game_ticks": budget.max_game_ticks,
        "max_entities": budget.max_entities,
        "max_bytes": budget.max_bytes,
        "max_output_tokens": budget.max_output_tokens,
        "max_actions": budget.max_actions,
    })
}

/// One logical read ledger covers the canonical world and retained work. The
/// projection category counts actual escaped JSON without allocating a second
/// copy; it is distinct from the final response's independent output bound.
struct ReadWork {
    world: Value,
    records: u64,
    bytes: u64,
    retained_records: u64,
    source_text_bytes: u64,
    projection_bytes: u64,
}

impl ReadWork {
    fn new(world: Value) -> Self {
        Self {
            records: world["records"].as_u64().unwrap_or(u64::MAX),
            bytes: world["canonical_bytes"].as_u64().unwrap_or(u64::MAX),
            world,
            retained_records: 0,
            source_text_bytes: 0,
            projection_bytes: 0,
        }
    }

    fn records(&mut self, count: usize, context: &OperationContext) -> Result<()> {
        self.records = self.records.saturating_add(count as u64);
        self.retained_records = self.retained_records.saturating_add(count as u64);
        if self.records > u64::from(context.budget.max_entities) {
            return Err(exhausted(
                "session output negotiation exceeds its combined world and retained-work record budget",
            ));
        }
        Ok(())
    }

    fn source_text(&mut self, bytes: usize, context: &OperationContext) -> Result<()> {
        self.bytes = self.bytes.saturating_add(bytes as u64);
        self.source_text_bytes = self.source_text_bytes.saturating_add(bytes as u64);
        if self.bytes > context.budget.max_bytes {
            return Err(exhausted(
                "session output negotiation exceeds its combined world and retained-source byte budget",
            ));
        }
        Ok(())
    }

    fn projection(
        &mut self,
        value: &Value,
        context: &OperationContext,
        started: Instant,
    ) -> Result<()> {
        struct Counter {
            count: u64,
            maximum: u64,
            wall: Duration,
            started: Instant,
        }
        impl std::io::Write for Counter {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if self.count.saturating_add(bytes.len() as u64) > self.maximum
                    || self.started.elapsed() >= self.wall
                {
                    return Err(std::io::Error::other("retained work exhausted its bound"));
                }
                self.count += bytes.len() as u64;
                Ok(bytes.len())
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut counter = Counter {
            count: 0,
            maximum: context.budget.max_bytes.saturating_sub(self.bytes),
            wall: Duration::from_millis(context.budget.max_wall_millis),
            started,
        };
        serde_json::to_writer(&mut counter, value).map_err(|_| {
            exhausted(
                "session output negotiation exceeds its combined source, retained-work byte or wall allowance",
            )
        })?;
        self.bytes = self.bytes.saturating_add(counter.count);
        self.projection_bytes = self.projection_bytes.saturating_add(counter.count);
        Ok(())
    }

    fn json(self) -> Value {
        json!({
            "world": self.world,
            "retained_records_and_steps": self.retained_records,
            "original_source_text_bytes": self.source_text_bytes,
            "retained_projection_bytes": self.projection_bytes,
            "combined_records": self.records,
            "combined_bytes": self.bytes,
        })
    }
}

fn original_source_bytes(source: &PlanSource) -> usize {
    match source {
        PlanSource::Pause { summary, .. } => summary.len(),
        PlanSource::Actions { summary, raw }
        | PlanSource::Blueprint { summary, raw }
        | PlanSource::Production { summary, raw }
        | PlanSource::ProductionContinuation { summary, raw }
        | PlanSource::RoomContinuation { summary, raw }
        | PlanSource::CivilianRelease { summary, raw }
        | PlanSource::ConnectedRoom { summary, raw }
        | PlanSource::ConnectedRoomContinuation { summary, raw }
        | PlanSource::ResourceAcquisitionContinuation { summary, raw } => {
            summary.len().saturating_add(raw.len())
        }
    }
}

fn admit_retained_work(
    session: &LabSession,
    current: &OperationContext,
    started: Instant,
    work: &mut ReadWork,
) -> Result<()> {
    work.records(plan_resources::count(session), current)?;
    work.records(session.carried.len(), current)?;
    for entry in plan_resources::entries(session) {
        check(current, started)?;
        if let Some(plan) = entry.plan {
            work.records(plan.steps.len(), current)?;
        }
    }
    let actions = session
        .open_actions
        .iter()
        .chain(&session.last_plan_actions)
        .copied()
        .chain(session.last_action)
        .collect::<BTreeSet<_>>();
    work.records(actions.len(), current)?;
    if let Some(pending) = &session.pending {
        work.source_text(original_source_bytes(&pending.source), current)?;
    }
    for objective in &session.objectives {
        check(current, started)?;
        let original = objective.resource_goal(session);
        work.source_text(original.summary.len(), current)?;
        work.source_text(original.request_json.map_or(0, str::len), current)?;
    }
    Ok(())
}

/// Presentation must use the owner's current allowance, never a lagging cache.
/// Budget metadata discloses no world facts and remains available on authority
/// refusals. This inspection performs no progress or request-sequence mutation.
pub(crate) fn current_budget(session_id: &str) -> Result<WorkBudget> {
    let session = resolve_session(Some(session_id.to_owned()))?;
    with_session_read(&session, Err, |session| Ok(session.budget))
}

/// The caller has explicitly selected an existing session and one new output
/// ceiling. Successful bytes are final; a facade must not refit them or reopen
/// the session after the acknowledged change has been published.
pub(crate) fn renegotiate(
    session_id: String,
    max_output_tokens: u32,
    started: Instant,
) -> Result<String> {
    let session = resolve_session(Some(session_id))?;
    let _admission = LAB_ADMISSION.read().map_err(|_| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "laboratory session admission poisoned",
        )
    })?;
    // Resolution can race a private durable reopen. Keep the late owner fence
    // and admission guard until the complete response and budget are accepted.
    ensure_durable_owner(&session)?;
    with_session_view(
        &session,
        || {
            Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "laboratory output negotiation session poisoned",
            ))
        },
        |session| renegotiate_owned(session, max_output_tokens, started),
        false,
    )
}

fn renegotiate_owned(
    session: &mut LabSession,
    max_output_tokens: u32,
    started: Instant,
) -> Result<String> {
    let current = read_context(session);
    check(&current, started)?;
    let mut proposed = current.clone();
    proposed.budget.max_output_tokens = max_output_tokens;
    validate_lab_budget(proposed.budget)?;
    let request = session.next_request_id.checked_add(1).ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "session request counter exhausted",
        )
    })?;
    proposed.request_id = RequestId::new(request);

    // The complete source uses the original work allowance. A larger response
    // does not buy a larger scan, source, game-time interval or action count.
    let source_work = query_budget::admit_sources(
        &[session.adapter.snapshot()],
        &remaining(&current, started)?,
    )?;
    let mut read_work = ReadWork::new(source_work);
    admit_retained_work(session, &current, started, &mut read_work)?;
    let mut projection = temporal_handler::projection(session, &proposed)?;
    projection.active_work["carried_obligations"] = json!(
        session
            .carried
            .iter()
            .filter(|carried| !carried.is_final())
            .map(CarriedStep::to_json)
            .collect::<Vec<_>>()
    );
    projection.active_work["query_baselines"] =
        temporal_query::retained_baseline_work(&remaining(&proposed, started)?)?;
    let watches = temporal_query::retained_watch_work(&remaining(&proposed, started)?)?;
    projection.active_work["query_watches"] = watches["query_watches"].clone();
    projection.active_work["condition_watches"] = watches["condition_watches"].clone();
    // Discovery scans the complete retained task membership, even though the
    // orientation lists only the usual four active-first handles.
    let task_page = crate::task_service::session_tasks(&projection.session_id, 0);
    let task_count = task_page["total"].as_u64().ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::AdapterUnavailable,
            "complete retained task membership is unavailable; the session output budget is unchanged",
        )
    })?;
    read_work.records(
        usize::try_from(task_count)
            .map_err(|_| exhausted("task membership is not representable"))?,
        &current,
    )?;
    read_work.projection(&task_page, &current, started)?;
    projection.active_work["mcp_tasks"] = json!(
        task_page["tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .take(4)
            .cloned()
            .collect::<Vec<_>>()
    );
    let mut task_coverage = crate::task_service::session_handle_coverage(&projection.session_id);
    task_coverage["retained_total"] = json!(task_count);
    task_coverage["service_error"] = task_page["service_error"].clone();
    for name in ["query_baselines", "query_watches"] {
        read_work.records(
            projection.active_work[name].as_array().map_or(1, Vec::len),
            &current,
        )?;
    }
    read_work.projection(&projection.active_work, &current, started)?;
    read_work.projection(&projection.objectives, &current, started)?;
    read_work.projection(&task_coverage, &current, started)?;
    let output = render(
        &projection,
        current.budget,
        read_work.json(),
        task_coverage,
        session.durability_fault.as_deref(),
    )?;
    // Source admission, catalog acquisition and complete serialization share the
    // original deadline. Nothing fallible follows this last check and mutation.
    check(&current, started)?;
    session.budget.max_output_tokens = max_output_tokens;
    session.next_request_id = request;
    Ok(output)
}

fn render(
    projection: &temporal_response::TemporalResponseProjection,
    previous_budget: WorkBudget,
    source_work: Value,
    task_coverage: Value,
    durability_fault: Option<&str>,
) -> Result<String> {
    let maximum = projection
        .budget
        .max_bytes
        .min(u64::from(projection.budget.max_output_tokens).saturating_mul(4));
    let admitted = budget_json(projection.budget);
    let payload = json!({
        "ok": true,
        "schema": "dfmcp.session-output-budget/1",
        "kind": "output_budget_renegotiation",
        "session_id": projection.session_id,
        "request_id": projection.request_id,
        "anchor": projection.anchor,
        "current_anchor": projection.anchor,
        "game_tick": projection.anchor["game_tick"],
        "paused": projection.paused,
        "previous_budget": budget_json(previous_budget),
        "budget": admitted,
        "changed": projection.budget.max_output_tokens != previous_budget.max_output_tokens,
        "maximum_response_bytes": maximum,
        "game_effect": "none",
        "publication": "complete_before_session_output_budget_change",
        "source_work": source_work,
    });
    let output = AgentTurnBuilder::new("fortress.open_session", AgentPhase::Orient)
        .session_id(projection.session_id.clone())
        .request_id(projection.request_id.clone())
        .turn_id(format!("lab-output-budget-turn-{}", projection.request_id))
        .anchor(projection.anchor.clone())
        .continuity(
            ContinuityStatus::Partial,
            Some(projection.anchor.clone()),
            None,
            None,
        )
        .profile(ObservationProfile::Briefing)
        .briefing(json!({
            "runtime": "laboratory_reference_world",
            "adapter": "memory",
            "paused": projection.paused,
            "objectives": projection.objectives,
            "session_replaced": false,
            "capability_grants_changed": false,
            "canonical_time_advanced": 0,
            "recovered_work_observation": "retained_not_reobserved",
            "durability_fault": durability_fault,
            "live_game_qualification": false,
        }))
        .active_work(projection.active_work.clone())
        .uncertainty(vec![uncertainty(
            "session-budget-work-evidence",
            "unknown",
            "Output negotiation retains current work and prior proof without polling actions or sampling watches.",
            "A larger response allowance does not prove completion, renew a deadline, or authorize an effect.",
            Some("fortress.observe"),
            json!({"session_id": projection.session_id}),
        )])
        .coverage(json!({
            "status": "partial",
            "complete_domains": ["laboratory.session_output_budget",
                "laboratory.pending_plan_catalog", "laboratory.session_query_baseline_catalog",
                "laboratory.session_query_watch_catalog", "laboratory.nonfinal_carried_obligation_catalog"],
            "partial_domains": [{"domain": "laboratory.current_game_work",
                "reason": "owned receipts and recovery records; proof unrefreshed and other sessions' deferred work incomplete"}],
            "omitted_domains": ["live_dwarf_fortress_state"],
            "mcp_tasks": task_coverage,
            "absence_proof_scope": [],
            "continuation": null,
            "source_anchor": projection.anchor,
            "eligible_as_mutation_evidence": false,
        }))
        .budget(json!({
            "requested": {"max_output_tokens": projection.budget.max_output_tokens},
            "admitted": admitted,
            "consumed": {"canonical_source": source_work, "canonical_game_ticks": 0},
            "remaining": null, "soft_stop_reason": null, "hard_stop_reason": null,
        }))
        .references(vec![json!({
            "kind": "laboratory_world",
            "state_hash": projection.anchor["state_hash"],
            "anchor": projection.anchor,
        })])
        .attach(payload);
    if output.len() as u64 > maximum {
        return Err(exhausted(
            "complete session output acknowledgement and current work exceed the proposed allowance; the previous budget is unchanged",
        ));
    }
    Ok(output)
}

#[cfg(test)]
#[path = "lab_session_output_budget_tests.rs"]
mod tests;
