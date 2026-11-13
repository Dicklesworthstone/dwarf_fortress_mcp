//! Current, read-only explanations of exact original laboratory plans/goals.
//!
//! Inspection never polls an obligation or certifies historical achievement.
//! Current predicate samples, retained action proof, physical work and the
//! complete original project remain separate, including after recovery.

use std::time::{Duration, Instant};

use dfmcp_core::{Capability, DfmcpError, Digest32, ErrorCode, Result, RiskTier};
use dfmcp_intent::{PreparedPlan, plan_archive};
use dfmcp_world::{Predicate, PredicateEvidence, PredicateTruth};
use serde_json::{Value, json};

use super::{LabSession, PlanSource, anchor_json, authorize_entry, read_context};
use crate::agent_turn::{AgentPhase, AgentTurnBuilder, ContinuityStatus, ObservationProfile};

const OPERATION: &str = "fortress.explain";
const SCHEMA: &str = "dfmcp.lab-goal-explanation/1";
const PIN_SCHEMA: &str = "dfmcp.lab-goal-explanation-pin/1";
const MAX_STEPS: usize = 64;
const MAX_REQUIREMENTS: usize = 1_024;
const PAGE_ROWS: usize = 8;
const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn exhausted(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

pub(crate) fn is_subject(raw: &str) -> bool {
    raw.starts_with("plan:") || raw.starts_with("goal:")
}

#[derive(Debug, PartialEq, Eq)]
struct Subject {
    goal_only: bool,
    plan: Digest32,
    object: Option<Digest32>,
    observation: Option<Digest32>,
    section: &'static str,
    offset: usize,
}

fn digest(raw: &str) -> Result<Digest32> {
    if raw.len() != 64
        || !raw
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid(
            "explanation digests require 64 lowercase hexadecimal digits",
        ));
    }
    Digest32::from_hex(raw).ok_or_else(|| invalid("invalid explanation digest"))
}

fn subject(raw: &str) -> Result<Subject> {
    if raw.len() > 240 {
        return Err(invalid("explanation selector exceeds its byte bound"));
    }
    let (goal_only, raw) = if let Some(raw) = raw.strip_prefix("goal:") {
        (true, raw)
    } else if let Some(raw) = raw.strip_prefix("plan:") {
        (false, raw)
    } else {
        return Err(invalid("typed explanation requires plan: or goal:"));
    };
    let mut parts = raw.split('~');
    let plan = digest(parts.next().ok_or_else(|| invalid("missing plan digest"))?)?;
    let object = parts.next().map(digest).transpose()?;
    let observation = parts.next().map(digest).transpose()?;
    let (section, offset) = if observation.is_some() {
        let section = match parts.next() {
            Some("requirements") => "requirements",
            Some("steps") => "steps",
            _ => return Err(invalid("explanation page requires requirements or steps")),
        };
        let raw = parts
            .next()
            .ok_or_else(|| invalid("missing explanation page offset"))?;
        let offset = raw
            .parse::<usize>()
            .map_err(|_| invalid("invalid explanation page offset"))?;
        if offset.to_string() != raw {
            return Err(invalid(
                "explanation offsets require canonical decimal integers",
            ));
        }
        (section, offset)
    } else {
        ("requirements", 0)
    };
    if parts.next().is_some() {
        return Err(invalid("extra explanation selector segments"));
    }
    Ok(Subject {
        goal_only,
        plan,
        object,
        observation,
        section,
        offset,
    })
}

fn request(session: &str, selector: String) -> Value {
    json!({"tool":OPERATION,"arguments":{"session_id":session,"entity_id":selector}})
}

fn page_request(
    session: &str,
    target: &Subject,
    object: Digest32,
    pin: Digest32,
    section: &str,
    offset: usize,
) -> Value {
    request(
        session,
        format!(
            "{}:{}~{object}~{pin}~{section}~{offset}",
            if target.goal_only { "goal" } else { "plan" },
            target.plan
        ),
    )
}

fn check_time(session: &LabSession, started: Instant) -> Result<()> {
    if started.elapsed() >= Duration::from_millis(session.budget.max_wall_millis) {
        return Err(exhausted(
            "original-goal explanation exceeded its complete wall-time budget",
        ));
    }
    Ok(())
}

/// Keep original request text in its lossless resources. Hashing its exact
/// bytes here prevents continuation across replacement at an unchanged world.
fn source_identity(source: &PlanSource) -> Value {
    match source {
        PlanSource::Pause {
            summary,
            paused_target,
        } => json!({
            "kind":"pause","summary":summary,"paused_target":paused_target}),
        PlanSource::Actions { summary, raw }
        | PlanSource::Blueprint { summary, raw }
        | PlanSource::Production { summary, raw }
        | PlanSource::ProductionContinuation { summary, raw }
        | PlanSource::RoomContinuation { summary, raw }
        | PlanSource::CivilianRelease { summary, raw }
        | PlanSource::ConnectedRoom { summary, raw }
        | PlanSource::ConnectedRoomContinuation { summary, raw }
        | PlanSource::ResourceAcquisitionContinuation { summary, raw } => json!({
            "kind":super::plan_resources::source_kind(source),"summary":summary,
            "request_digest":Digest32::of_bytes(raw.as_bytes()).to_hex()}),
    }
}

fn source_bytes(source: &PlanSource) -> usize {
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

fn truth_name(truth: PredicateTruth) -> &'static str {
    match truth {
        PredicateTruth::True => "true",
        PredicateTruth::False => "false",
        PredicateTruth::Unknown => "unknown",
    }
}

fn evidence_result(result: Result<PredicateTruth>) -> Value {
    match result {
        Ok(truth) => {
            json!({"truth":truth_name(truth),"epistemic":if truth == PredicateTruth::Unknown {
            "unknown" } else { "certified_derived" }})
        }
        Err(error) => json!({"truth":"unknown","epistemic":"unknown",
            "error":{"code":error.code.as_str(),"message":error.message}}),
    }
}

/// Flatten conjunctions only. A false alternative of an already-true Any is
/// not a blocker; Not and Any therefore remain indivisible logical clauses.
fn requirements(
    plan: &PreparedPlan,
    evidence: &PredicateEvidence<'_>,
    session: &LabSession,
    started: Instant,
) -> Result<Vec<Value>> {
    plan.terminal_condition.validate_shape()?;
    if plan.terminal_condition.complexity() > MAX_REQUIREMENTS {
        return Err(exhausted(
            "original terminal condition exceeds the explanation node bound",
        ));
    }
    let mut pending = vec![(&plan.terminal_condition, "/terminal_condition".to_owned())];
    let mut rows = Vec::new();
    let mut bytes = 0usize;
    while let Some((predicate, path)) = pending.pop() {
        check_time(session, started)?;
        if let Predicate::All(children) = predicate {
            for (index, child) in children.iter().enumerate().rev() {
                pending.push((child, format!("{path}/all/{index}")));
            }
        } else {
            let mut row = evidence_result(evidence.evaluate(predicate));
            row["path"] = json!(path);
            row["condition"] = crate::lab_world::predicate_json(predicate);
            bytes = bytes.saturating_add(row.to_string().len());
            if bytes as u64 > session.budget.max_bytes.min(MAX_SOURCE_BYTES) {
                return Err(exhausted(
                    "complete requirement diagnostics exceed their byte-work budget",
                ));
            }
            rows.push(row);
        }
    }
    Ok(rows)
}

fn steps(
    plan: &PreparedPlan,
    session: &LabSession,
    pending: bool,
    started: Instant,
) -> Result<Vec<Value>> {
    let mut rows = Vec::with_capacity(plan.steps.len());
    let mut bytes = 0usize;
    for step in &plan.steps {
        check_time(session, started)?;
        let mut row = json!({"step":step.id.get(),"kind":crate::lab_world::action_kind(&step.action),
            "depends_on":step.depends_on.iter().map(|id| id.get()).collect::<Vec<_>>(),
            "capability":step.required_capability.as_str(),"risk":step.risk.as_str(),
            "obligation":step.obligation.as_ref().map(|obligation| json!({
                "deadline_tick":obligation.deadline_tick.0,
                "poll_interval_ticks":obligation.poll_interval_ticks,
                "stable_observations_required":obligation.stable_for_observations,
                "has_failure_condition":obligation.failure.is_some(),
                "sampling_performed":false})),
            "receipt":null,"recovered_evidence":null,"blind_retry_allowed":false});
        if let Some(receipt) = session.adapter.step_receipt(plan.id, step.id) {
            row["dispatch_status"] = json!("retained_receipt");
            row["receipt"] = json!({"action_id":receipt.action_id.to_string(),
                "state":format!("{:?}",receipt.state),
                "observed_anchor":anchor_json(&receipt.observed_anchor),
                "receipt_digest":receipt.adapter_receipt_digest.to_hex()});
            row["physical_work"] = super::action_work_json(session, receipt.action_id);
        } else if let Some(carried) = session
            .carried
            .iter()
            .find(|carried| carried.plan_digest == plan.digest && carried.step == Some(step.id))
        {
            row["dispatch_status"] = json!("recovered_custody");
            row["recovered_evidence"] = carried.to_json();
            row["physical_work"] = json!({"state":"not_resampled", "quiescent":null,
                "reason":"recovered evidence keeps its recorded anchor; inspection does not poll it"});
        } else {
            row["dispatch_status"] = json!(if pending {
                "prepared_not_dispatched"
            } else {
                "receipt_unavailable_or_deferred"
            });
            row["physical_work"] = json!({"state":"unknown", "quiescent":null,
                "reason":"absence of an action receipt does not prove absence or completion of physical work"});
        }
        bytes = bytes.saturating_add(row.to_string().len());
        if bytes as u64 > session.budget.max_bytes.min(MAX_SOURCE_BYTES) {
            return Err(exhausted(
                "complete original-step diagnostics exceed their byte-work budget",
            ));
        }
        rows.push(row);
    }
    Ok(rows)
}

fn selected_goal(mut goal: Value, session: &str, plan: Digest32) -> Value {
    if let Some(object) = goal.as_object_mut() {
        // Current truth, project scope, reserve status, continuation, quiescence
        // and every historical proof anchor are retained without reinterpretation.
        object.remove("original_source");
        object.remove("terminal_condition");
    }
    goal["original_details"] = json!({"uri":format!("df://session/{session}/goal-{plan}"),
        "omitted_fields":["original_source","terminal_condition"],
        "reason":"lossless original details are separately retrievable"});
    goal
}

fn conclusion(goal: &Value, pending: bool, stale: bool) -> &'static str {
    if pending {
        return if stale {
            "prepared_plan_requires_fresh_review"
        } else {
            "prepared_plan_not_committed"
        };
    }
    if goal["status"] == "abandoned" {
        return "original_pursuit_abandoned";
    }
    if goal["status"] == "withdrawn" {
        return "original_goal_explicitly_withdrawn";
    }
    if goal["continuation_request"].is_object() {
        return "original_goal_requires_reviewed_continuation";
    }
    if ["resource_reserve", "production_reserve"]
        .iter()
        .any(|field| goal[*field]["status"] == "expired")
    {
        return "original_reserve_horizon_expired";
    }
    if ["resource_reserve", "production_reserve"]
        .iter()
        .any(|field| goal[*field]["status"] == "work_required")
    {
        return "future_reserve_requires_work";
    }
    if goal["semantic_room_goal"]["predicate_truth"] == "false" {
        return "original_room_is_incomplete";
    }
    if goal["semantic_output_goal"]["predicate_truth"] == "false" {
        return "original_output_goal_is_unmet";
    }
    if (goal["semantic_room_goal"].is_object()
        && goal["semantic_room_goal"]["predicate_truth"] != "true")
        || (goal["semantic_output_goal"].is_object()
            && goal["semantic_output_goal"]["predicate_truth"] != "true")
        || ["resource_reserve", "production_reserve"]
            .iter()
            .any(|field| goal[*field]["status"] == "unknown")
    {
        return "original_project_truth_is_unknown";
    }
    if goal["predicate_truth"] == "unknown" || goal["status"] == "indeterminate" {
        return "original_goal_truth_is_unknown";
    }
    if goal["physical_quiescent"] != true {
        return "physical_work_is_active_or_unknown";
    }
    if goal["predicate_truth"] == "true" {
        "original_stage_holds_at_current_anchor"
    } else {
        "original_stage_does_not_hold"
    }
}

fn inspect(session: &LabSession, raw: &str, started: Instant) -> Result<String> {
    let context = read_context(session);
    context.budget.validate()?;
    authorize_entry(&context, Capability::Observe, RiskTier::ReadOnly)?;
    authorize_entry(&context, Capability::Query, RiskTier::ReadOnly)?;
    check_time(session, started)?;
    let target = subject(raw)?;
    // Whole-source admission precedes original lookup and current evaluation.
    let source_work = super::query_budget::admit_sources(&[session.adapter.snapshot()], &context)?;
    let mut input_bytes = source_work["canonical_bytes"].as_u64().ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "query admission omitted its canonical source byte measure",
        )
    })?;
    let mut records = source_work["records"].as_u64().ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "query admission omitted its complete source record measure",
        )
    })?;
    records = records
        .saturating_add(super::plan_resources::count(session) as u64)
        .saturating_add(session.carried.len() as u64);
    // Selected goal continuation can inspect siblings. Admit every retained
    // original source before that existing shared oracle is allowed to run.
    for objective in &session.objectives {
        let entry = objective.resource_goal(session);
        input_bytes = input_bytes
            .saturating_add(entry.summary.len() as u64)
            .saturating_add(entry.request_json.map_or(0, str::len) as u64)
            .saturating_add(entry.verification_error.map_or(0, str::len) as u64);
        records = records.saturating_add(entry.plan.map_or(0, |plan| plan.steps.len()) as u64);
    }
    if let Some(pending) = &session.pending {
        input_bytes = input_bytes.saturating_add(source_bytes(&pending.source) as u64);
    }
    // The cancellation affordance reads the selected private commit receipt.
    // Charge those complete bytes before parsing it or encoding the archive.
    if let Some(receipt) = session.commit_receipts.get(&target.plan.to_hex()) {
        input_bytes = input_bytes.saturating_add(receipt.len() as u64);
    }
    if records > u64::from(context.budget.max_entities)
        || input_bytes > context.budget.max_bytes.min(MAX_SOURCE_BYTES)
    {
        return Err(exhausted(
            "original explanation exceeds its complete world and retained-source budget",
        ));
    }
    let original = super::plan_resources::goal(session, target.plan);
    let pending = session
        .pending
        .as_ref()
        .filter(|pending| !target.goal_only && pending.plan.digest == target.plan);
    let plan = match &original {
        Some(entry) => entry.plan,
        None => pending.map(|pending| &pending.plan),
    }
    .ok_or_else(|| DfmcpError::new(ErrorCode::StaleAnchor,
        "the exact verified original plan is no longer retained; inspection cannot reconstruct it"))?;
    if target.goal_only && original.is_none() {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "the committed original goal is not retained",
        ));
    }
    if plan.steps.len() > MAX_STEPS
        || records.saturating_add(plan.steps.len() as u64) > u64::from(context.budget.max_entities)
    {
        return Err(exhausted(
            "original explanation exceeds its retained step budget",
        ));
    }
    let available = context
        .budget
        .max_bytes
        .min(MAX_SOURCE_BYTES)
        .saturating_sub(input_bytes)
        .min(plan_archive::MAX_ARCHIVE_BYTES as u64) as usize;
    if available < 45 {
        return Err(exhausted(
            "original explanation has no complete archive allowance",
        ));
    }
    let (archive, seal) = plan_archive::encode_with_check(
        plan,
        plan_archive::ArchiveLimits {
            max_bytes: available,
            max_steps: MAX_STEPS,
            ..plan_archive::ArchiveLimits::default()
        },
        || check_time(session, started),
    )?;
    if target
        .object
        .is_some_and(|expected| expected != seal.object_digest)
    {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "the original typed plan object changed",
        ));
    }
    if let (Some(_), Some(pending)) = (&original, pending) {
        let (other, other_seal) = plan_archive::encode_with_check(
            &pending.plan,
            plan_archive::ArchiveLimits {
                max_bytes: available.saturating_sub(archive.len()),
                max_steps: MAX_STEPS,
                ..plan_archive::ArchiveLimits::default()
            },
            || check_time(session, started),
        )?;
        input_bytes = input_bytes.saturating_add(other.len() as u64);
        if other_seal != seal {
            return Err(DfmcpError::new(
                ErrorCode::CorruptLedger,
                "one semantic plan digest names conflicting retained typed objects",
            ));
        }
    }
    let anchor = anchor_json(&context.anchor);
    let source = if let Some(original) = &original {
        json!({"kind":original.source_kind,"summary":original.summary,
            "owner":original.owner_session_id.to_string(),
            "sealed_anchor":original.sealed_anchor.as_ref().map(anchor_json),
            "request_digest":original.request_json.map(|raw|Digest32::of_bytes(raw.as_bytes()).to_hex()),
            "paused_target":original.paused_target})
    } else if let Some(pending) = pending {
        source_identity(&pending.source)
    } else {
        Value::Null
    };
    let is_pending = original.is_none();
    let cancellation = super::plan_cancellation::offer(session, target.plan);
    let withdrawal = super::objectives::withdrawal_offer(session, target.plan);
    let goal = if original.is_some() {
        super::objectives::original_goal_observation(session, &target.plan.to_hex())?.1
    } else {
        let mut value = evidence_result(crate::lab_world::plan_goal_truth(
            session.adapter.snapshot(),
            plan,
        ));
        value["predicate_truth"] = value["truth"].clone();
        value["historical_achievement"] = json!("not_committed");
        value["completion_scope"] = json!("sealed_plan_terminal_condition_only");
        value["physical_quiescent"] = Value::Null;
        value
    };
    let evidence = PredicateEvidence::laboratory(session.adapter.snapshot())?;
    let requirements = requirements(plan, &evidence, session, started)?;
    let steps = steps(plan, session, is_pending, started)?;
    let current_stale = plan.anchor != context.anchor || plan.expires_at_tick < context.anchor.tick;
    let session_id = session.session_id.to_string();
    // The world anchor alone cannot pin an explanation: receipt verification,
    // retained goal proof and restore custody may change without a game tick or
    // cursor advancing. Bind the complete selected projection before paging or
    // omitting lossless source fields, and bind the exact inspecting session.
    let pin_material = json!({"schema":PIN_SCHEMA,"session_id":session_id,
        "retention":if is_pending {"pending_plan"} else {"retained_original_goal"},
        "anchor":anchor,"object":seal.object_digest.to_hex(),"source":source,
        "goal":goal,"requirements":requirements,"steps":steps,"cancellation":cancellation,
        "withdrawal":withdrawal})
    .to_string();
    if pin_material.len() as u64 > context.budget.max_bytes.min(MAX_SOURCE_BYTES) {
        return Err(exhausted(
            "complete explanation pin exceeds its diagnostic byte-work budget",
        ));
    }
    let observation_pin = Digest32::of_bytes(pin_material.as_bytes());
    check_time(session, started)?;
    if target
        .observation
        .is_some_and(|expected| expected != observation_pin)
    {
        return Err(DfmcpError::new(
            ErrorCode::CursorGap,
            "the current world, selected proof or original custody changed during explanation; restart from the original selector",
        ));
    }
    let goal = if is_pending {
        goal
    } else {
        selected_goal(goal, &session_id, target.plan)
    };
    let plan_uri = format!(
        "df://session/{session_id}/plan-{}~{}",
        target.plan, seal.object_digest
    );
    let mut payload = json!({"schema":SCHEMA,"ok":true,"operation":OPERATION,"session_id":session_id,
        "subject":if target.goal_only {"goal"} else {"plan"},"plan_digest":target.plan.to_hex(),
        "object_digest":seal.object_digest.to_hex(),"observation_pin":observation_pin.to_hex(),
        "current_anchor":anchor,"sealed_anchor":anchor_json(&plan.anchor),"summary":plan.summary,
        "retention":if is_pending {"pending_plan"} else {"retained_original_goal"},
        "conclusion":conclusion(&goal,is_pending,current_stale),"goal":goal,"cancellation":cancellation,
        "withdrawal":withdrawal,
        "current_goal_evaluation":"read_only_no_historical_proof_update",
        "requirement_samples_are_original_goal_proof":false,"game_effect":"none",
        "requirements_summary":{"total":requirements.len(),
            "true":requirements.iter().filter(|row|row["truth"] == "true").count(),
            "false":requirements.iter().filter(|row|row["truth"] == "false").count(),
            "unknown":requirements.iter().filter(|row|row["truth"] == "unknown").count(),
            "semantics":"conjunctive terminal clauses; Any and Not stay intact; exact goal additionally validates original identity and custody"},
        "step_count":steps.len(),
        "original_steps":page_request(&session_id,&target,seal.object_digest,observation_pin,"steps",0),
        "original_requirements":page_request(&session_id,&target,seal.object_digest,observation_pin,"requirements",0),
        "source_work":{"world":source_work,"retained_records":records,
            "complete_input_bytes":input_bytes.saturating_add(archive.len() as u64),"archive_bytes":archive.len()},
        "original_archive":{"uri":format!("{plan_uri}~archive~0"),"format":"DFMPAR01",
            "sha256":Digest32::of_bytes(&archive).to_hex(),"bytes":archive.len()},
        "original_plan":{"uri":plan_uri},"blind_retry_allowed":false,
        "section":target.section,"offset":target.offset,"rows":[],"next":null});
    if let Some(next) = payload["goal"].get("continuation_request").cloned() {
        payload["next_step"] = next;
    }
    let rows = if target.section == "steps" {
        &steps
    } else {
        &requirements
    };
    if target.offset > rows.len() || (target.offset == rows.len() && target.offset != 0) {
        return Err(invalid(
            "explanation offset is outside the selected complete domain",
        ));
    }
    let maximum = context
        .budget
        .max_bytes
        .min(u64::from(context.budget.max_output_tokens).saturating_mul(4))
        as usize;
    let mut count = (rows.len() - target.offset).min(PAGE_ROWS);
    loop {
        check_time(session, started)?;
        payload["rows"] = json!(&rows[target.offset..target.offset + count]);
        payload["total"] = json!(rows.len());
        payload["returned"] = json!(count);
        let complete = target.offset + count == rows.len();
        payload["complete"] = json!(complete);
        payload["complete_domain"] = json!(complete && target.offset == 0);
        payload["next"] = if complete {
            Value::Null
        } else {
            page_request(
                &session_id,
                &target,
                seal.object_digest,
                observation_pin,
                target.section,
                target.offset + count,
            )
        };
        let mut active_work = crate::agent_turn::empty_active_work();
        active_work["coverage"] = json!({"status":"partial","scope":"selected_original_plan",
            "session_work_complete":false,"absence_proven":false,
            "handoff":format!("df://session/{session_id}/handoff")});
        if is_pending {
            active_work["pending_plans"] = json!([{"plan_digest":target.plan.to_hex(),
                "state":if current_stale {"review_required"} else {"prepared"},
                "commit_authority_granted":false}]);
        }
        active_work["selected_plan_work"] = payload["original_steps"].clone();
        let turn = AgentTurnBuilder::new(OPERATION,AgentPhase::Inspect)
            .session_id(session_id.clone()).anchor(anchor.clone())
            .continuity(ContinuityStatus::Partial,None,None,None)
            .profile(ObservationProfile::Forensic)
            .briefing(json!({"runtime":"laboratory_reference_world","paused":session.adapter.snapshot().paused,
                "plan_digest":target.plan.to_hex(),"conclusion":payload["conclusion"],
                "historical_proof_updated":false,"sampling_performed":false}))
            .active_work(active_work)
            .coverage(json!({"status":"partial","complete_domains":["selected_original_plan_identity","current_source_anchor"],
                "absence_proof_scope":[],"current_evaluation_scope":"selected_original_goal_and_terminal_clauses",
                "omitted_domains":["unselected_session_work","live_dwarf_fortress_state"],"continuation":payload["next"]}))
            .budget(json!({"maximum_response_bytes":maximum,"source_work":payload["source_work"]}))
            .references(vec![json!({"kind":"resource","uri":plan_uri}),
                json!({"kind":"resource","uri":format!("df://session/{session_id}/handoff")})]);
        let output = turn.attach(payload.clone());
        check_time(session, started)?;
        if output.len() <= maximum {
            return Ok(output);
        }
        if count <= 1 {
            return Err(exhausted(
                "complete original-goal explanation metadata and one intact row exceed the response budget",
            ));
        }
        count /= 2;
    }
}

fn refusal(error: DfmcpError, session_id: Option<&str>) -> String {
    let mut turn = AgentTurnBuilder::new(OPERATION, AgentPhase::Inspect)
        .continuity(ContinuityStatus::Partial, None, None, None)
        .coverage(json!({"status":"unknown","complete_domains":[],"absence_proof_scope":[]}));
    if let Some(session) = session_id {
        turn = turn.session_id(session.to_owned());
    }
    turn.attach(
        json!({"schema":SCHEMA,"ok":false,"error":{"operation":OPERATION,
        "code":error.code.as_str(),"message":error.message,"retryable":error.retryable,
        "effect_may_have_occurred":false},"historical_proof_updated":false}),
    )
}

pub(crate) fn execute(session_id: Option<String>, raw: &str) -> String {
    let started = Instant::now();
    let session = match super::resolve_session(session_id.clone()) {
        Ok(session) => session,
        Err(error) => return refusal(error, session_id.as_deref()),
    };
    super::with_session_read(
        &session,
        |error| refusal(error, session_id.as_deref()),
        |session| match inspect(session, raw, started) {
            Ok(output) => output,
            Err(error) => crate::output_budget::fit(
                &refusal(error, Some(&session.session_id.to_string())),
                u64::from(session.budget.max_output_tokens).min(session.budget.max_bytes / 4),
            ),
        },
    )
}

#[cfg(test)]
#[path = "lab_goal_explanation_tests.rs"]
mod tests;
