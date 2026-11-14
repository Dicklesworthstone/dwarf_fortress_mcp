//! Explicit close of a quiet laboratory session.
//!
//! Closing is a lifetime transition, never a drain. It proves the retained
//! work quiet, fits its complete receipt, then revokes authority and releases
//! process-local custody. The admission write guard excludes foreground calls;
//! Task custody and registration locks close the delayed-creation races.
use super::*;
use crate::agent_turn::{AgentPhase, AgentTurnBuilder, ContinuityStatus, ObservationProfile};
use serde_json::Value;
use std::collections::VecDeque;
use std::time::Instant;

const MAX_CLOSE_RECEIPTS: usize = 32;
const MAX_CLOSE_ACTIONS: usize = 16_384;
static CLOSED: LazyLock<Mutex<VecDeque<(SessionId, String)>>> =
    LazyLock::new(|| Mutex::new(VecDeque::new()));

fn unavailable(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InternalInvariantViolation, message)
}

fn conflict(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::Conflict, message)
}

fn recent_receipt(id: SessionId) -> Result<String> {
    CLOSED
        .lock()
        .map_err(|_| unavailable("session close receipts poisoned"))?
        .iter()
        .find(|(closed, _)| *closed == id)
        .map(|(_, output)| output.clone())
        .ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::SessionNotFound,
                "session is not open and no recent close receipt is retained",
            )
        })
}

struct Quiet {
    anchor: StateAnchor,
    actions: BTreeSet<ActionId>,
    certificate: Digest32,
    census_digest: Digest32,
    whole_world: bool,
    durable: bool,
    shared: bool,
    members_remaining: usize,
    leases: usize,
    pending_reviews: usize,
    objectives: usize,
    carried: usize,
    source_bytes: u64,
    source_records: u64,
    started: Instant,
}

fn bound(session: &LabSession, started: Instant, actions: usize, source_records: u64) -> Result<()> {
    if actions > MAX_CLOSE_ACTIONS
        || source_records.saturating_add(actions as u64) > u64::from(session.budget.max_entities)
    {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "complete session work exceeds the close census budget; no resources were released",
        ));
    }
    if started.elapsed().as_nanos().div_ceil(1_000_000)
        > u128::from(session.budget.max_wall_millis)
    {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "session close census exhausted its wall-time allowance",
        ));
    }
    Ok(())
}

/// Historical terminal proof is insufficient: inspect every owned original
/// against the current physical world. The last owner also covers the entire
/// adapter index so a missing foreground handle cannot hide work it would drop.
fn inspect(session: &LabSession) -> Result<Quiet> {
    let started = Instant::now();
    session.budget.validate()?;
    let current = read_context(session);
    authorize_entry(&current, Capability::Observe, RiskTier::ReadOnly)?;
    let snapshot = session.adapter.snapshot();
    // Admit complete terrain, edges, events and nested values before hashing or
    // any physical-work scan. A small final receipt cannot hide a large source.
    let admitted = query_budget::admit_observation_sources(&[snapshot], &current)?;
    let source_records = admitted["records"].as_u64()
        .ok_or_else(|| unavailable("close source record measure is unavailable"))?;
    let mut source_bytes = admitted["canonical_bytes"].as_u64()
        .ok_or_else(|| unavailable("close source byte measure is unavailable"))?;
    if !snapshot.hash_is_valid() {
        return Err(conflict("session close cannot certify an unsealed world"));
    }
    let shared = session.shared.is_some();
    if shared && session.shared_members == 0 {
        return Err(conflict("shared session membership is unavailable"));
    }
    let whole_world = !shared || session.shared_members == 1;
    let mut actions = session
        .open_actions
        .iter()
        .chain(&session.last_plan_actions)
        .copied()
        .chain(session.last_action)
        .collect::<BTreeSet<_>>();
    for (raw_digest, receipt) in &session.commit_receipts {
        source_bytes = source_bytes.checked_add(receipt.len() as u64).ok_or_else(|| {
            DfmcpError::new(ErrorCode::BudgetExceeded, "session close source byte overflow")
        })?;
        if source_bytes > session.budget.max_bytes {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "complete original commit custody exceeds the close source byte budget",
            ));
        }
        let digest = plan_cancellation::parse_scope(&format!("plan:{raw_digest}"))?;
        actions.extend(plan_cancellation::select(session, digest)?.actions);
        bound(session, started, actions.len(), source_records)?;
    }
    let mut leases = 0;
    for (id, (holder, held)) in &session.leases.by_action {
        if *holder == session.session_id {
            actions.insert(*id);
            leases += held.len();
        }
    }
    if whole_world {
        for id in session.adapter.retained_action_ids() {
            actions.insert(id);
            bound(session, started, actions.len(), source_records)?;
        }
    }
    bound(session, started, actions.len(), source_records)?;
    let admitted_source_bytes = source_bytes;
    let mut action_census = Vec::with_capacity(actions.len());
    for id in &actions {
        if !action_fully_drained(&session.adapter, *id) {
            return Err(conflict(
                "session still owns nonterminal or active/unknown physical work; explicitly drain or reconcile it before close",
            ));
        }
        let receipt = session.adapter.action_receipt(*id)
            .ok_or_else(|| conflict("original proof receipt is unavailable"))?;
        let original = session.adapter.action_plan_receipt(*id)
            .ok_or_else(|| conflict("original commit identity is unavailable"))?;
        let step = session.adapter.action_step(*id)
            .ok_or_else(|| conflict("original action identity is unavailable"))?;
        let row = json!({
            "action_id":id.to_string(),"plan_id":original.plan_id.to_string(),
            "plan_digest":original.plan_digest.to_hex(),"step":receipt.step_id.get(),
            "idempotency_key":step.idempotency_key,
            "proof_state":format!("{:?}",receipt.state),
            "proof_receipt_digest":receipt.adapter_receipt_digest.to_hex(),
            "proof_anchor":anchor_json(&receipt.observed_anchor),
            "proof_message":receipt.message,
            "physical_work":effect_work_json(&session.adapter.action_work_state(*id)?),
            "physical_observed_anchor":anchor_json(&snapshot.anchor()),
        });
        charge_census(session, &mut source_bytes, &row)?;
        action_census.push(row);
        bound(session, started, actions.len(), source_records)?;
    }
    let mut recovered_census = Vec::with_capacity(session.carried.len());
    // Recovered work has no foreground dispatch handle. Re-read its physical
    // identities even when its historical proof was already final.
    for carried in &session.carried {
        if !carried.is_final() {
            return Err(conflict(
                "recovered work is unfinished or physically unknown; session ownership must remain available",
            ));
        }
        let step = carried.work_step.as_ref().ok_or_else(|| {
            conflict("recovered work lacks its original physical identity; close requires reconciliation")
        })?;
        let dispatched = carried.state != "not_dispatched";
        if !dispatched && !goal_custody::effect_identity_absent(step, snapshot) {
            return Err(conflict("current work contradicts recovered nondispatch custody"));
        }
        let physical = dfmcp_intent::inspect_effect_work(
            snapshot, &step.action, &step.idempotency_key, dispatched,
        )?;
        if !physical.is_quiescent() {
            return Err(conflict("recovered physical work is not currently proven quiet"));
        }
        let row = json!({
            "retained_proof":carried.to_json(),
            "original_step":step.id.get(),"idempotency_key":step.idempotency_key,
            "physical_work":effect_work_json(&physical),
            "physical_observed_anchor":anchor_json(&snapshot.anchor()),
        });
        charge_census(session, &mut source_bytes, &row)?;
        recovered_census.push(row);
        bound(session, started, actions.len() + session.carried.len(), source_records)?;
    }
    if !untracked_work(session)?.is_empty() {
        return Err(conflict(
            "untracked active or unknown physical work prevents session close; no cancellation authority is reconstructed",
        ));
    }
    bound(session, started, actions.len() + session.carried.len(), source_records)?;
    let census = json!({"schema":"dfmcp-lab-session-close-census/1",
        "actions":action_census,"recovered_steps":recovered_census});
    let census_bytes = census.to_string();
    source_bytes = admitted_source_bytes.checked_add(census_bytes.len() as u64).ok_or_else(|| {
        DfmcpError::new(ErrorCode::BudgetExceeded, "complete close source byte overflow")
    })?;
    if source_bytes > session.budget.max_bytes {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "complete canonical world, original custody and census exceed the close source byte budget",
        ));
    }
    let census_digest = Digest32::of_bytes(census_bytes.as_bytes());
    let canonical = json!({
        "schema":"dfmcp-lab-session-close-quiescence/1",
        "census_digest":census_digest.to_hex(),
        "session_id":session.session_id.to_string(),
        "anchor":anchor_json(&snapshot.anchor()),
        "actions":actions.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "scope":if whole_world {"last_owner_world"} else {"owned_session_actions"},
        "recovered_steps":session.carried.len(),
        "untracked_work_quiescent":true,
    });
    Ok(Quiet {
        anchor: snapshot.anchor(),
        actions,
        certificate: Digest32::of_bytes(canonical.to_string().as_bytes()),
        census_digest,
        whole_world,
        durable: session.durable_scenario.is_some(),
        shared,
        members_remaining: if shared { session.shared_members - 1 } else { 0 },
        leases,
        pending_reviews: usize::from(session.pending.is_some()),
        objectives: session.objectives.len(),
        carried: session.carried.len(),
        source_bytes,
        source_records,
        started,
    })
}

fn charge_census(session: &LabSession, bytes: &mut u64, row: &Value) -> Result<()> {
    *bytes = bytes.checked_add(row.to_string().len() as u64).ok_or_else(|| {
        DfmcpError::new(ErrorCode::BudgetExceeded, "session close census byte overflow")
    })?;
    if *bytes > session.budget.max_bytes {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "complete close proof and physical-work census exceeds the source byte budget",
        ));
    }
    Ok(())
}

fn render(session: &LabSession, quiet: &Quiet, terminal_tasks: usize, released: Value) -> Result<String> {
    let output = AgentTurnBuilder::new("fortress.cancel", AgentPhase::Reconcile)
        .session_id(session.session_id.to_string())
        .anchor(anchor_json(&quiet.anchor))
        .profile(ObservationProfile::Pulse)
        .continuity(
            ContinuityStatus::Partial,
            None,
            Some(json!({"reason":"session_authority_released"})),
            None,
        )
        .briefing(json!({
            "session_closed":true,"mutation_admissible":false,
            "active_work_scope":"released_session_only",
            "owned_actions_terminal_and_quiet":true,
            "shared_members_remaining":quiet.members_remaining,
            "durable_world_retained":quiet.durable,
            "historical_proof_is_not_goal_success":true,
        }))
        .coverage(json!({
            "status":"partial",
            "complete_domains":["session_resource_release","owned_work_quiescence"],
            "omitted_domains":["current_goal_success","other_sessions","future_world_state"],
            "continuation":null,
        }))
        .budget(json!({
            "requested":null,
            "admitted":{"max_bytes":session.budget.max_bytes,
                "max_output_tokens":session.budget.max_output_tokens},
            "consumed":{"close_source_bytes":quiet.source_bytes,
                "world_source_records":quiet.source_records,
                "actions_inspected":quiet.actions.len(),"recovered_steps_inspected":quiet.carried,
                "game_ticks":0,"effects":0},
            "remaining":null,"soft_stop_reason":null,"hard_stop_reason":null,
        }))
        .attach(json!({
            "ok":true,"session_id":session.session_id.to_string(),
            "scope":"session","mode":"close_session","closed":true,"release_only":true,
            "process_local_resources":released,
            "session_slot_released":true,"orientation_slot_released":true,
            "pending_reviews_discarded":quiet.pending_reviews,
            "volatile_history_released":true,
            "goal_records_discarded":if quiet.durable || quiet.members_remaining > 0 {0} else {quiet.objectives},
            "goals_withdrawn":false,"goal_success_proven":false,
            "shared_membership_released":quiet.shared,
            "shared_members_remaining":quiet.members_remaining,
            "shared_world_released":quiet.shared && quiet.members_remaining == 0,
            "process_local_world_released":quiet.whole_world,
            "durable_world_retained":quiet.durable,
            "durable_head_published":quiet.durable,
            "durable_private_owner_released":quiet.durable && !quiet.shared,
            "owned_leases_released":quiet.leases,
            "terminal_task_records_retained":terminal_tasks,
            "terminal_task_reads_require_open_session_authority":true,
            "game_effects_cancelled":false,"game_ticks_advanced":0,"mutation_dispatched":false,
            "finalize_certificate":{
                "digest":quiet.certificate.to_hex(),
                "census_digest":quiet.census_digest.to_hex(),
                "census_schema":"dfmcp-lab-session-close-census/1",
                "scope":if quiet.whole_world {"last_owner_world"} else {"owned_session_actions"},
                "actions_total":quiet.actions.len(),"recovered_steps":quiet.carried,
                "observed_anchor":anchor_json(&quiet.anchor),
                "statement":"selected retained actions are terminal and their current physical work is quiet; this close dispatched no effects",
            },
            "close_receipt_retention":MAX_CLOSE_RECEIPTS,"receipt_durable":false,
            "reopen_requires_new_session":true,
        }));
    let maximum = session.budget.max_bytes.min(
        u64::from(session.budget.max_output_tokens)
            .saturating_mul(crate::output_budget::BYTES_PER_TOKEN as u64),
    );
    if output.len() as u64 > maximum {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "complete session close receipt does not fit; no session or query resources were released",
        ));
    }
    Ok(output)
}

fn release_leases(leases: &mut LeaseBook, owner: SessionId) {
    let owned = leases.by_action.iter()
        .filter(|(_, (holder, _))| *holder == owner)
        .map(|(action, _)| *action)
        .collect::<Vec<_>>();
    for action in owned {
        if let Some((holder, held)) = leases.by_action.remove(&action) {
            for lease in held {
                // An already expired lease may have been collected. Work was
                // independently proven quiet before dropping this bookkeeping.
                let _ = leases.manager.release_lease(lease, holder);
            }
        }
    }
    leases.unpause_consent.remove(&owner);
}

/// Revoke stale Arc holders and release retained payloads even when a caller
/// still owns an Arc. Shared world custody has already been returned to its owner.
fn revoke(session: &mut LabSession) {
    session.closed = true;
    session.grants.clear();
    session.pending = None;
    session.last_action = None;
    session.last_plan_actions.clear();
    session.open_actions.clear();
    session.commit_receipts.clear();
    session.commit_authority.clear();
    session.shared = None;
    session.leases = LeaseBook::default();
    session.shared_members = 0;
    session.durable_scenario = None;
    session.durability_fault = None;
    session.durable_plans.clear();
    session.durable_restore.clear();
    session.carried.clear();
    session.replay = crate::replay::ReplayLog::default();
    session.history = new_history();
    session.objectives.clear();
    session.objective_restore.clear();
    session.civilian_recovery = civilian_recovery::CivilianRecovery::default();
    session.adapter = MemoryAdapter::new(WorldSnapshot::new(
        session.fortress_id,
        GameTick(0),
        dfmcp_core::ObservationCursor::ORIGIN,
        true,
        dfmcp_world::WorldGraph::default(),
    ));
}

fn close_admitted(handle: &Arc<Mutex<LabSession>>, terminal_tasks: usize) -> Result<String> {
    ensure_durable_owner(handle)?;
    let quiet = with_session_view(
        handle,
        || Err(unavailable("session or shared world poisoned; quiet close is unproven")),
        |session| {
            let quiet = inspect(session)?;
            if quiet.durable {
                // Publish the exact current world/frontier before relinquishing
                // its durable owner. A failed save keeps all ownership intact.
                persist_durable_head(session);
                if session.durability_fault.is_some() {
                    return Err(DfmcpError::new(
                        ErrorCode::AdapterUnavailable,
                        "durable head publication failed; session ownership was retained",
                    ));
                }
                // A durable admission may precede an adapter commit refusal.
                // Such a source has no foreground action handle, but is still
                // unfinished custody. Only an explicit drain/reconciliation
                // may retire it; closing never invents nondispatch evidence.
                with_durable_store(|store| {
                    if store.head(session.fortress_id).map(|head| head.anchor) != Some(quiet.anchor) {
                        return Err(conflict("durable head does not match the closing observation"));
                    }
                    for commit in store.commits(session.fortress_id) {
                        let owned_or_unknown = quiet.whole_world
                            || store.objective(session.fortress_id, commit.plan_digest)
                                .is_none_or(|objective| objective.owner_session_id == session.session_id);
                        if owned_or_unknown {
                            return Err(conflict(
                                "an owned or unattributed durable commit still has unfinished custody; close cannot abandon deferred or unproven nondispatch work",
                            ));
                        }
                    }
                    Ok(())
                })?;
            }
            Ok(quiet)
        },
        false,
    )?;
    let shared = handle.lock().map_err(|_| unavailable("session poisoned"))?.shared.clone();
    let mut worlds = SHARED_WORLDS.lock().map_err(|_| unavailable("shared world registry poisoned"))?;
    let mut world = shared.as_ref().map(|shared| shared.lock())
        .transpose().map_err(|_| unavailable("shared world poisoned"))?;
    let mut session = handle.lock().map_err(|_| unavailable("session poisoned"))?;
    let id = session.session_id;
    if let Some(shared) = shared.as_ref() {
        if !worlds.get(&session.fortress_id).is_some_and(|current| Arc::ptr_eq(current, shared))
            || !world.as_ref().is_some_and(|world| world.members.contains(&id))
        {
            return Err(conflict("shared world membership changed before close publication"));
        }
    }
    let mut registry = SESSIONS.lock().map_err(|_| unavailable("session registry poisoned"))?;
    let mut receipts = CLOSED.lock().map_err(|_| unavailable("close receipt registry poisoned"))?;
    if session.closed || !registry.get(&id).is_some_and(|current| Arc::ptr_eq(current, handle)) {
        return Err(conflict("session ownership changed before close publication"));
    }
    // Explicit close consents to discarding pending reviews and private derived
    // query handles. The canonical release fits the packet before deleting any.
    let output = semantic_query::release_session_resources(id, true, |released| {
        let output = render(&session, &quiet, terminal_tasks, released)?;
        bound(
            &session, quiet.started, quiet.actions.len() + quiet.carried, quiet.source_records,
        )?;
        Ok(output)
    })?;
    // No fallible operation after query-resource publication.
    if let Some(world) = world.as_mut() {
        release_leases(&mut world.leases, id);
        world.members.remove(&id);
        if world.members.is_empty() {
            worlds.remove(&session.fortress_id);
        }
    } else if quiet.durable {
        let mut durable = durable_lab();
        if durable.owners.get(&session.fortress_id) == Some(&id) {
            durable.owners.remove(&session.fortress_id);
        }
    }
    revoke(&mut session);
    registry.remove(&id);
    crate::agent_facade::release_session_orientation(&id.to_string());
    receipts.push_back((id, output.clone()));
    while receipts.len() > MAX_CLOSE_RECEIPTS {
        receipts.pop_front();
    }
    Ok(output)
}

fn close(raw: Option<String>) -> Result<String> {
    let raw = raw.ok_or_else(|| DfmcpError::new(
        ErrorCode::InvalidRequest, "close_session requires an explicit session_id",
    ))?;
    let id = parse_session_id_arg(&raw)?;
    let _admission = LAB_ADMISSION.write().map_err(|_| unavailable("session close admission poisoned"))?;
    let handle = match lookup_session(id) {
        Ok(handle) => handle,
        Err(error) if error.code == ErrorCode::SessionNotFound => return recent_receipt(id),
        Err(error) => return Err(error),
    };
    // This guard remains held through registry removal. A Task whose initial
    // validation completed earlier must recheck registration under the same
    // custody lock before creating its upstream record.
    crate::task_service::with_session_idle(&id.to_string(), |terminal_tasks| {
        close_admitted(&handle, terminal_tasks)
    })
}

pub(crate) fn cancel(raw: Option<String>, scope: Option<String>) -> String {
    let session_id = raw.as_deref()
        .and_then(|raw| parse_session_id_arg(raw).ok())
        .map(|id| id.to_string());
    let result = if scope.as_deref() == Some("session") {
        close(raw)
    } else {
        Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "close_session requires explicit scope=session; ordinary session cancellation remains a drain",
        ))
    };
    match result {
        Ok(output) => output,
        Err(error) => {
            let mut builder = AgentTurnBuilder::new("fortress.cancel", AgentPhase::Reconcile)
                .continuity(
                    ContinuityStatus::Partial, None,
                    Some(json!({"reason":"session_close_refused"})), None,
                )
                .briefing(json!({
                    "session_close_refused":true,"resources_released_by_this_call":false,
                    "mutation_dispatched":false,"active_work_scope":"not_certified",
                }))
                .coverage(json!({
                    "status":"partial","complete_domains":["session_close_refusal"],
                    "omitted_domains":["current_game_state","owned_work_quiescence"],
                    "continuation":null,
                }));
            if let Some(id) = session_id.as_ref() {
                builder = builder.session_id(id);
            }
            builder.attach(json!({
                "ok":false,"session_id":session_id,"mode":"close_session","scope":"session",
                "closed":null,"resources_released":false,"release_scope":"this_call",
                "mutation_dispatched":false,
                "error":{"operation":"fortress.cancel","code":error.code.as_str(),
                    "message":error.message,"retryable":false,"details":[]},
            }))
        }
    }
}

#[cfg(test)]
#[path = "lab_session_close_tests.rs"]
mod tests;
