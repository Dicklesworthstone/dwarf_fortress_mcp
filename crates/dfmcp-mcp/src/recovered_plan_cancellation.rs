//! Stop an exact durable original under fresh authority after recovery.
//!
//! The archived complete action census proves what was dispatched. It never
//! restores dispatch handles, compensation, or old capability grants. Each
//! physical stop publishes its new world and disposition together before the
//! next stop, while historical terminal proof remains immutable.

use std::time::{Duration, Instant};

use dfmcp_adapter::CancelMode;
use dfmcp_core::{Capability, DfmcpError, Digest32, ErrorCode, Result, RiskTier};
use dfmcp_intent::{Action, EffectWorkState, PreparedPlan, plan_archive};
use dfmcp_lab::{RecoveredWorkDrainReceipt, RecoveredWorkRef};
use serde_json::{Value, json};

use super::{LabSession, selected_plan_wait::RecoveredStepCustody};

const MAX_STEPS: usize = 64;
const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;

fn refused(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::Conflict, message)
}

fn exhausted(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

fn check_time(session: &LabSession, started: Instant) -> Result<()> {
    if started.elapsed() >= Duration::from_millis(session.budget.max_wall_millis) {
        return Err(exhausted(
            "recovered cancellation exhausted its shared wall-time allowance",
        ));
    }
    Ok(())
}

fn remaining_context(
    session: &LabSession,
    started: Instant,
) -> Result<dfmcp_core::OperationContext> {
    check_time(session, started)?;
    let mut context = super::read_context(session);
    let spent = started.elapsed().as_nanos().div_ceil(1_000_000);
    if spent >= u128::from(context.budget.max_wall_millis) {
        return Err(exhausted(
            "recovered cancellation has no remaining effect-inspection time",
        ));
    }
    context.budget.max_wall_millis -= spent as u64;
    Ok(context)
}

fn original(session: &LabSession, digest: Digest32) -> Result<&super::Objective> {
    let mut originals = session
        .objectives
        .iter()
        .filter(|goal| goal.plan_digest == digest);
    let original = originals.next().ok_or_else(|| {
        refused("no exact verified recovered original matches the selected plan digest")
    })?;
    if originals.next().is_some() {
        return Err(refused(
            "selected recovered original has ambiguous source custody",
        ));
    }
    Ok(original)
}

struct Selection {
    plan: PreparedPlan,
    steps: Vec<RecoveredStepCustody>,
    original_owner: String,
    archive_digest: Digest32,
    custody: Value,
    started: Instant,
}

fn needs_drain(row: &RecoveredStepCustody) -> bool {
    row.state == "dispatched" || !row.work.is_quiescent()
}

fn reference<'a>(plan: &'a PreparedPlan, row: &RecoveredStepCustody) -> RecoveredWorkRef<'a> {
    RecoveredWorkRef {
        plan,
        step_id: row.step,
        recorded_anchor: row.recorded_anchor,
        recovery_anchor: row.recovery_anchor,
        custody_digest: row.custody_digest,
    }
}

/// Admit the entire original and source domain before cloning a plan or stopping
/// any effect. Current reads, writes, conflicting custody, and the independent
/// emergency pause all share this request's budgets.
fn select(session: &LabSession, digest: Digest32, mode: CancelMode) -> Result<Selection> {
    let started = Instant::now();
    session.budget.validate()?;
    let context = super::read_context(session);
    super::authorize_entry(&context, Capability::Observe, RiskTier::ReadOnly)?;
    super::authorize_entry(&context, Capability::Query, RiskTier::ReadOnly)?;
    if session.durable_scenario.is_none() {
        return Err(refused(
            "recovered cancellation requires exact durable original custody",
        ));
    }
    if mode == CancelMode::CompensateReversible {
        return Err(refused(
            "recovery does not restore compensation handles; use stop_future_steps or emergency_pause_and_drain for the exact original",
        ));
    }
    if session.budget.max_output_tokens < 1_500 || session.budget.max_bytes < 6_000 {
        return Err(exhausted(
            "recovered cancellation requires room for its complete bounded outcome",
        ));
    }
    let source = super::query_budget::admit_sources(&[session.adapter.snapshot()], &context)?;
    let source_records = source["records"].as_u64().ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "cancellation source count missing",
        )
    })?;
    let source_bytes = source["canonical_bytes"].as_u64().ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "cancellation source size missing",
        )
    })?;
    let selected = original(session, digest)?;
    let entry = selected.resource_goal(session);
    let plan = entry
        .plan
        .ok_or_else(|| refused("the exact original sealed plan is unavailable"))?;
    let records = source_records
        .saturating_add(session.objectives.len() as u64)
        .saturating_add(session.carried.len() as u64)
        .saturating_add(session.leases.by_action.len() as u64)
        .saturating_add(session.leases.manager.active_lease_count() as u64)
        .saturating_add(plan.steps.len() as u64);
    if plan.steps.is_empty()
        || plan.steps.len() > MAX_STEPS
        || plan.steps.len() > session.budget.max_actions as usize
        || records > u64::from(session.budget.max_entities)
    {
        return Err(exhausted(
            "complete recovered original and ownership census exceed the request budget",
        ));
    }
    let bytes = source_bytes
        .saturating_add(entry.summary.len() as u64)
        .saturating_add(entry.request_json.map_or(0, str::len) as u64);
    let maximum = session.budget.max_bytes.min(MAX_SOURCE_BYTES);
    if bytes >= maximum {
        return Err(exhausted(
            "complete recovered cancellation source exceeds its byte allowance",
        ));
    }
    let (archive_bytes, archive) = plan_archive::encode_with_check(
        plan,
        plan_archive::ArchiveLimits {
            max_bytes: ((maximum - bytes) as usize).min(plan_archive::MAX_ARCHIVE_BYTES),
            max_steps: MAX_STEPS,
            ..plan_archive::ArchiveLimits::default()
        },
        || check_time(session, started),
    )?;
    // The archive visitor bounds depth and size before custody validation can
    // rehash the original seal. Its complete receipt map shares the same cap.
    let custody = selected.selected_wait_recovery(session)?;
    if bytes
        .saturating_add(archive_bytes.len() as u64)
        .saturating_add(custody.bytes()? as u64)
        > maximum
    {
        return Err(exhausted(
            "complete original archive and recovered receipt custody exceed the shared byte allowance",
        ));
    }
    let steps = custody.drain_census(session, plan)?;
    let mut effects = usize::from(
        mode == CancelMode::EmergencyPauseAndDrain && !session.adapter.snapshot().paused,
    );
    let untracked = super::untracked_work(session)?;
    // Test reservations on a private copy. No lease or counter is acquired by
    // a refused preflight, and the shared world lock spans all real stops.
    let mut leases = session.leases.manager.clone();
    for (row, step) in steps.iter().zip(&plan.steps) {
        check_time(session, started)?;
        if !needs_drain(row) {
            continue;
        }
        if !session
            .carried
            .iter()
            .any(|carried| carried.plan_digest == digest && carried.step == Some(row.step))
        {
            return Err(refused(
                "selected physical work has no retained recovered cleanup owner",
            ));
        }
        if !matches!(
            step.action,
            Action::DesignateDig { .. }
                | Action::HarvestResource { .. }
                | Action::Build { .. }
                | Action::PlaceFurniture { .. }
                | Action::CreateWorkOrder { .. }
        ) {
            return Err(refused(
                "an unfinished immediate original lacks final action proof; reconcile it before cancelling recovered work",
            ));
        }
        if session.adapter.inspect_recovered_work_drain(
            &reference(plan, row),
            &remaining_context(session, started)?,
        )? != row.work
        {
            return Err(refused(
                "selected original physical identity changed during admission",
            ));
        }
        effects += usize::from(matches!(row.work, EffectWorkState::Active { .. }));
        let scope = step.action.scope();
        if let Some(area) = scope.map_area {
            if untracked.iter().any(|work| work.conflicts_with(&area)) {
                return Err(refused(
                    "untracked physical work overlaps the selected original; reconcile its custody before stopping this region",
                ));
            }
            leases.acquire_spatial_lease(session.session_id, area, true, context.anchor.tick, 1)?;
        }
        for entity in &scope.entity_ids {
            leases.acquire_entity_lease(
                session.session_id,
                *entity,
                true,
                context.anchor.tick,
                1,
            )?;
        }
        // Expiry never releases another member's unresolved physical work,
        // including a harvested source or furnishing item held by entity lease.
        for (action, (owner, _)) in &session.leases.by_action {
            if *owner == session.session_id
                || super::action_fully_drained(&session.adapter, *action)
            {
                continue;
            }
            let held = session
                .adapter
                .action_step(*action)
                .ok_or_else(|| {
                    refused(
                        "another member's unresolved physical lease has lost its original action",
                    )
                })?
                .action
                .scope();
            if scope
                .entity_ids
                .iter()
                .any(|entity| held.entity_ids.contains(entity))
                || scope
                    .map_area
                    .zip(held.map_area)
                    .is_some_and(|(area, held)| dfmcp_core::lease::cuboids_intersect(&area, &held))
            {
                return Err(refused(
                    "another member retains overlapping unresolved physical work",
                ));
            }
        }
    }
    if effects > session.budget.max_actions as usize {
        return Err(exhausted(
            "recovered physical stops and emergency pause exceed the aggregate action budget",
        ));
    }
    if mode == CancelMode::EmergencyPauseAndDrain {
        super::authorize_entry(&context, Capability::ControlClock, RiskTier::Reversible)?;
    }
    check_time(session, started)?;
    Ok(Selection {
        plan: plan.clone(),
        steps,
        original_owner: entry.owner_session_id.to_string(),
        archive_digest: archive.object_digest,
        custody: custody.evidence_json()?,
        started,
    })
}

pub(super) fn receipt_json(receipt: &RecoveredWorkDrainReceipt) -> Value {
    json!({
        "schema":"dfmcp.recovered-work-drain/1",
        "plan_digest":receipt.plan_digest.to_hex(),"step":receipt.step_id.get(),
        "custody_digest":receipt.custody_digest.to_hex(),
        "recorded_anchor":super::anchor_json(&receipt.recorded_anchor),
        "recovery_anchor":super::anchor_json(&receipt.recovery_anchor),
        "before_anchor":super::anchor_json(&receipt.before_anchor),
        "observed_anchor":super::anchor_json(&receipt.observed_anchor),
        "before":super::effect_work_json(&receipt.before),
        "after":super::effect_work_json(&receipt.after),"stopped_work":receipt.stopped_work,
        "historical_terminal_proof_preserved":true,"dispatch_handles_restored":false,
        "evidence":receipt.evidence.iter().map(|evidence| json!({
            "evidence_id":evidence.id.to_string(),"digest":evidence.digest.to_hex(),
            "kind":format!("{:?}",evidence.kind),"summary":evidence.summary,
            "anchor":super::anchor_json(&evidence.anchor),
        })).collect::<Vec<_>>(),
    })
}

fn publish_stop(session: &mut LabSession, receipt: RecoveredWorkDrainReceipt) -> Result<()> {
    let row = session
        .carried
        .iter_mut()
        .find(|row| row.plan_digest == receipt.plan_digest && row.step == Some(receipt.step_id))
        .ok_or_else(|| {
            refused(
                "the selected recovered physical step lost its retained custody before publication",
            )
        })?;
    // An unfinished proof receives a new cancellation disposition. Old failure
    // and success proof, including their exact observation anchors, is retained.
    if row.state == "dispatched" {
        row.state = "cancelled".to_owned();
        row.proof_anchor = Some(receipt.observed_anchor);
    }
    row.work_state = receipt.after.clone();
    row.work_anchor = Some(receipt.observed_anchor);
    row.observation_error = None;
    row.recovered_drain = Some(receipt);
    super::persist_durable_head(session);
    if let Some(fault) = &session.durability_fault {
        return Err(DfmcpError::new(
            ErrorCode::AdapterUnavailable,
            format!("recovered physical work was stopped but durable publication failed: {fault}"),
        ));
    }
    Ok(())
}

pub(crate) fn cancel(session: &mut LabSession, mode: CancelMode, digest: Digest32) -> String {
    let selected = match select(session, digest, mode) {
        Ok(selected) => selected,
        Err(error) => return super::dfmcp_error_payload("fortress.cancel", &error),
    };
    let mut failure = None;
    let mut emergency = None;
    if mode == CancelMode::EmergencyPauseAndDrain {
        let before = session.adapter.snapshot().anchor();
        let changed = !session.adapter.snapshot().paused;
        match super::next_context(session)
            .and_then(|_| remaining_context(session, selected.started))
            .and_then(|context| session.adapter.pause_for_emergency_drain(&context))
        {
            Ok(evidence) => {
                super::reset_emergency_unpause_consent(session, mode);
                super::persist_durable_head(session);
                emergency = Some(json!({"requested":true,"changed":changed,"paused":true,
                    "before_anchor":super::anchor_json(&before),
                    "observed_anchor":super::anchor_json(&evidence.anchor),
                    "game_ticks_advanced":0,"action_handles_created":false,
                    "evidence_digest":evidence.digest.to_hex(),"durable":true,
                    "publication_confirmed":session.durability_fault.is_none()}));
                if let Some(fault) = &session.durability_fault {
                    failure = Some(DfmcpError::new(
                        ErrorCode::AdapterUnavailable,
                        format!(
                            "emergency pause is current but durable publication failed: {fault}"
                        ),
                    ));
                }
            }
            Err(error) => failure = Some(error),
        }
    }
    let mut stopped = 0usize;
    let mut cancelled = 0usize;
    let mut terminal_stopped = 0usize;
    let mut drains = Vec::new();
    for row in selected.steps.iter().rev().filter(|row| needs_drain(row)) {
        if failure.is_some() {
            break;
        }
        let outcome = check_time(session, selected.started)
            .and_then(|()| super::next_context(session))
            .and_then(|_| remaining_context(session, selected.started))
            .and_then(|context| {
                session
                    .adapter
                    .drain_recovered_work(&reference(&selected.plan, row), &context)
            });
        match outcome {
            Ok(receipt) => {
                let unfinished = session.carried.iter().any(|carried| {
                    carried.plan_digest == digest
                        && carried.step == Some(row.step)
                        && carried.state == "dispatched"
                });
                stopped += usize::from(receipt.stopped_work);
                cancelled += usize::from(unfinished);
                terminal_stopped += usize::from(!unfinished && receipt.stopped_work);
                drains.push(receipt_json(&receipt));
                if let Err(error) = publish_stop(session, receipt) {
                    failure = Some(error);
                }
            }
            Err(error) => failure = Some(error),
        }
    }
    drains.reverse();
    let census = original(session, digest)
        .and_then(|goal| goal.selected_wait_recovery(session))
        .and_then(|custody| custody.drain_census(session, &selected.plan));
    let mut final_rows = Vec::new();
    let (remaining_nonterminal, remaining_work) =
        match census {
            Ok(rows) => {
                let nonterminal = rows.iter().filter(|row| row.state == "dispatched").count();
                let work = rows.iter().filter(|row| !row.work.is_quiescent()).count();
                final_rows = rows.iter().map(|row| json!({
                "step":row.step.get(),"recorded_state":row.recorded_state,"state":row.state,
                "recorded_anchor":super::anchor_json(&row.recorded_anchor),
                "proof_anchor":row.proof_anchor.as_ref().map(super::anchor_json),
                "work_state":super::effect_work_json(&row.work),
            })).collect();
                (Some(nonterminal), Some(work))
            }
            Err(error) => {
                if failure.is_none() {
                    failure = Some(error);
                }
                (None, None)
            }
        };
    let physical_quiet = remaining_work == Some(0);
    if failure.is_none() {
        failure = check_time(session, selected.started).err();
    }
    let quiescent = failure.is_none()
        && session.durability_fault.is_none()
        && remaining_nonterminal == Some(0)
        && physical_quiet;
    let anchor = super::anchor_json(&session.adapter.snapshot().anchor());
    let certificate = quiescent.then(|| {
        let canonical = json!({"schema":"dfmcp-recovered-plan-drain-certificate/1",
            "plan_digest":digest.to_hex(),"plan_object_digest":selected.archive_digest.to_hex(),
            "custody":selected.custody,"steps":final_rows,"anchor":anchor});
        json!({"digest":Digest32::of_bytes(canonical.to_string().as_bytes()).to_hex(),
            "anchor":anchor,"plan_digest":digest.to_hex(),
            "custody_digest":selected.custody["digest"],"publication_confirmed":true,
            "statement":"every original step has a final disposition and its exact physical work is currently proven quiescent"})
    });
    let mut result = json!({"ok":failure.is_none(),"session_id":session.session_id.to_string(),
        "scope":format!("plan:{digest}"),"requested_plan_digest":digest.to_hex(),
        "plan_digest":digest.to_hex(),"plan_id":selected.plan.id.to_string(),
        "plan_object_digest":selected.archive_digest.to_hex(),
        "selection_scope":"exact_recovered_original","selection_origin":"durable_recovery",
        "original_owner_session_id":selected.original_owner,"dispatch_handles_restored":false,
        "descendant_plans_included":false,"recovery_evidence":selected.custody,
        "drain_progress":{"actions_total":selected.steps.len(),
            "already_terminal":selected.steps.iter().filter(|row| row.state != "dispatched").count(),
            "drained":stopped,"cancelled":cancelled,"compensated":0,
            "terminal_work_stopped":terminal_stopped,"remaining_nonterminal":remaining_nonterminal,
            "remaining_work":remaining_work,"physical_quiescent":physical_quiet,"quiescent":quiescent},
        "steps":final_rows,"physical_drains":drains,"finalize_certificate":certificate,
        "paused":session.adapter.snapshot().paused,"emergency_pause":emergency,
        "observed_anchor":anchor,"durability_pending":session.durability_fault.is_some(),
        "publication_confirmed":session.durability_fault.is_none(),
        "note":"fresh cleanup uses current authority; original terminal proof and partial game progress remain history"});
    if let Some(error) = failure {
        result["error"] = json!({"operation":"fortress.cancel","code":error.code.as_str(),
            "message":error.message,"details":error.details,"retryable":error.retryable,
            "effect_may_have_occurred":!drains.is_empty() || emergency.is_some(),
            "recovery":{"class":"reconciliation_required","next_tool":"fortress.observe"}});
    }
    result.to_string()
}

/// The explanation pin includes current grants, custody and the full selection.
/// Inspecting this offer never polls a monitor or publishes durable progress.
pub(crate) fn offer(session: &LabSession, digest: Digest32) -> Value {
    let selected = match select(session, digest, CancelMode::StopFutureSteps) {
        Ok(selected) => selected,
        Err(error) => {
            return json!({"available":false,"scope":"exact_recovered_original",
            "reason":error.message,"code":error.code.as_str()});
        }
    };
    if !selected.steps.iter().any(needs_drain) {
        return json!({"available":false,"scope":"exact_recovered_original",
            "reason":"original dispositions are already final and physical work is quiescent"});
    }
    json!({"available":true,"scope":"exact_recovered_original","plan_digest":digest.to_hex(),
        "custody_digest":selected.custody["digest"],"dispatch_handles_restored":false,
        "descendant_plans_included":false,
        "request":{"tool":"fortress.cancel","arguments":{"session_id":session.session_id.to_string(),
            "scope":format!("plan:{digest}"),"mode":"stop_future_steps"}},
        "effect":"stop exact recovered original work and publish fresh quiet evidence while preserving historical terminal proof",
        "authority":"current scoped action grants are rechecked at every physical stop"})
}

#[cfg(test)]
#[path = "recovered_plan_cancellation_tests.rs"]
mod tests;
