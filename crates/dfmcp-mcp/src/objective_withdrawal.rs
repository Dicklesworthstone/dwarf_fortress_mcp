//! Explicitly stop pursuing a quiet original while retaining its exact history.
use super::*;
use dfmcp_lab::durable::DurableCommit;
use dfmcp_world::WorldSnapshot;

const MAX_STEPS: usize = 64;
const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;

fn refused(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::PreconditionsFailed, message)
}

fn exhausted(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

fn precedes(before: StateAnchor, after: StateAnchor) -> bool {
    before.fortress_id == after.fortress_id
        && before.tick <= after.tick
        && before.cursor.epoch <= after.cursor.epoch
        && (before.cursor.epoch < after.cursor.epoch
            || before.cursor.sequence < after.cursor.sequence
            || before == after)
}

pub(super) fn nondispatch_covers(
    withdrawal: &DurableObjectiveWithdrawal,
    step: &dfmcp_intent::PlanStep,
    snapshot: &WorldSnapshot,
) -> bool {
    snapshot.hash_is_valid()
        && precedes(withdrawal.anchor, snapshot.anchor())
        && goal_custody::effect_identity_absent(step, snapshot)
}

pub(super) fn projection(withdrawal: &DurableObjectiveWithdrawal) -> Value {
    json!({
        "schema":"dfmcp.original-goal-withdrawal/1",
        "anchor":anchor_json(&withdrawal.anchor),
        "requester_session_id":withdrawal.requester_session_id.to_string(),
        "proof_digest":withdrawal.proof_digest.to_hex(),
        "verification":"verified",
        "never_dispatched_steps":withdrawal.never_dispatched_steps,
        "physical_quiescent_at_withdrawal":true,
        "goal_success_claimed":false,
        "historical_evidence_retained":true,
        "dispatch_authority_restored":false,
    })
}

fn source_record(objective: &Objective, plan: &PreparedPlan) -> DurableObjective {
    DurableObjective {
        fortress_id: plan.anchor.fortress_id,
        plan_digest: objective.plan_digest,
        sealed_state_hash: objective.sealed_state_hash,
        intent_id: plan.intent_id.get(),
        source: objective.source.durable(),
        owner_session_id: objective.owner_session_id,
        first_satisfied_anchor: objective.first_satisfied_anchor,
        restore_abandoned_anchor: objective.restore_abandoned_anchor,
        retired_never_dispatched: objective
            .retired_never_dispatched
            .iter()
            .map(|(step, proof)| (*step, proof.anchor()))
            .collect(),
    }
}

fn terminal_state(state: CommitState) -> Result<&'static str> {
    match state {
        CommitState::Verified => Ok("verified"),
        CommitState::Failed => Ok("failed"),
        CommitState::Cancelled => Ok("cancelled"),
        CommitState::Compensated => Ok("compensated"),
        _ => Err(refused(
            "every original action must be terminal before withdrawal; drain the exact plan first",
        )),
    }
}

fn complete_census(plan: &PreparedPlan, terminal: &DurableCommit) -> Result<()> {
    if terminal.plan_digest != plan.digest
        || terminal.intent_id != plan.intent_id.get()
        || terminal.sealed_state_hash != plan.anchor.state_hash
        || terminal.steps.len() != plan.steps.len()
        || terminal.step_anchors.len() != plan.steps.len()
        || plan.steps.iter().any(|step| {
            !terminal.steps.contains_key(&step.id.get())
                || !terminal.step_anchors.contains_key(&step.id.get())
        })
    {
        return Err(refused(
            "withdrawal requires the complete exact original terminal action census",
        ));
    }
    Ok(())
}

fn quiet_at(plan: &PreparedPlan, never: &BTreeSet<u32>, snapshot: &WorldSnapshot) -> Result<()> {
    if never
        .iter()
        .any(|id| !plan.steps.iter().any(|step| step.id.get() == *id))
    {
        return Err(refused(
            "withdrawal nondispatch evidence names a foreign original step",
        ));
    }
    for step in &plan.steps {
        if never.contains(&step.id.get()) {
            if !goal_custody::effect_identity_absent(step, snapshot) {
                return Err(refused(
                    "an original creator contradicts positive nondispatch custody",
                ));
            }
        } else if !dfmcp_intent::inspect_effect_work(
            snapshot,
            &step.action,
            &step.idempotency_key,
            true,
        )?
        .is_quiescent()
        {
            return Err(refused(
                "original physical work is active or unknown; explicitly drain or reconcile it before withdrawal",
            ));
        }
    }
    Ok(())
}

fn authorize(session: &LabSession) -> Result<OperationContext> {
    let context = read_context(session);
    context.budget.validate()?;
    authorize_entry(&context, Capability::Observe, RiskTier::ReadOnly)?;
    authorize_entry(&context, Capability::Query, RiskTier::ReadOnly)?;
    // Withdrawal changes the objective ledger. Default read-only planning
    // grants cannot authorize this explicit disposition.
    authorize_entry(&context, Capability::Plan, RiskTier::Reversible)?;
    if session.objectives.len() > MAX_RETAINED_OBJECTIVES_PER_FORTRESS
        || session.objectives.len() > context.budget.max_entities as usize
    {
        return Err(exhausted(
            "withdrawal inspection exceeds the complete retained-goal budget",
        ));
    }
    Ok(context)
}

fn prepare(session: &LabSession, digest: Digest32) -> Result<DurableObjectiveWithdrawal> {
    let started = std::time::Instant::now();
    let context = authorize(session)?;
    let check_time = || {
        if started.elapsed() >= std::time::Duration::from_millis(context.budget.max_wall_millis) {
            Err(exhausted(
                "withdrawal exhausted its complete inspection deadline",
            ))
        } else {
            Ok(())
        }
    };
    let mut selected = session
        .objectives
        .iter()
        .filter(|goal| goal.plan_digest == digest);
    let objective = selected
        .next()
        .ok_or_else(|| refused("withdrawal requires one retained exact original goal"))?;
    if selected.next().is_some() || objective.verification_error.is_some() {
        return Err(refused(
            "original withdrawal custody is ambiguous or unverifiable",
        ));
    }
    if objective.originating_process && objective.owner_session_id != session.session_id {
        return Err(refused(
            "another live session owns this original goal; its owner must withdraw it",
        ));
    }
    if !objective.originating_process && session.durable_scenario.is_none() {
        return Err(refused(
            "recovered withdrawal requires its owning durable fortress",
        ));
    }
    query_budget::admit_sources(&[session.adapter.snapshot()], &context)?;
    if let Some(previous) = &objective.withdrawal {
        if session.durable_scenario.is_some() {
            with_durable_store(|store| {
                if store.objective_withdrawal(session.fortress_id, digest) != Some(previous) {
                    return Err(refused(
                        "withdrawn history lost its exact durable disposition",
                    ));
                }
                Ok(())
            })?;
        }
        return Ok(previous.clone());
    }
    if session
        .objectives
        .iter()
        .filter(|goal| goal.is_withdrawn())
        .count()
        >= MAX_WITHDRAWN_OBJECTIVES_PER_FORTRESS
    {
        return Err(exhausted(
            "withdrawn original archive is full; existing audit history remains retained",
        ));
    }
    if objective.abandoned(session) {
        return Err(refused(
            "restore-abandoned pursuit has no active withdrawal lineage; archived absence cannot prove current physical quiet",
        ));
    }
    let plan = objective
        .plan
        .as_ref()
        .ok_or_else(|| refused("withdrawal lost the original sealed plan"))?;
    if !plan.digest_is_valid()
        || !plan.id_is_valid()
        || plan.digest != digest
        || plan.steps.is_empty()
        || plan.steps.len() > MAX_STEPS
        || plan.steps.len() > context.budget.max_actions as usize
    {
        return Err(exhausted(
            "withdrawal requires a valid original seal and a complete bounded step census",
        ));
    }
    if objective.originating_process {
        let selected = super::super::plan_cancellation::select(session, digest)?;
        if selected.actual != digest || plan.anchor.cursor.epoch != context.anchor.cursor.epoch {
            return Err(refused(
                "withdrawal selects the exact current-epoch original, not a request alias",
            ));
        }
    } else {
        // Custody is observation evidence; the fresh Plan grant above is the
        // authority for this new ledger disposition.
        objective.selected_wait_recovery(session)?;
    }
    if !goal_work_quiescent(session, objective)? {
        return Err(refused(
            "original work is active or unknown; drain the exact plan before withdrawing its goal",
        ));
    }
    let source = source_record(objective, plan);
    let mut never = BTreeSet::new();
    let mut live_terminal = DurableCommit {
        fortress_id: source.fortress_id,
        plan_digest: digest,
        sealed_state_hash: source.sealed_state_hash,
        intent_id: source.intent_id,
        source: source.source.clone(),
        steps: BTreeMap::new(),
        step_anchors: BTreeMap::new(),
    };
    for step in &plan.steps {
        check_time()?;
        if objective.original_handles_live {
            let receipt = session
                .adapter
                .step_receipt(plan.id, step.id)
                .ok_or_else(|| refused("an original action handle is unavailable"))?;
            let state = terminal_state(receipt.state)?;
            live_terminal.steps.insert(step.id.get(), state.to_owned());
            live_terminal
                .step_anchors
                .insert(step.id.get(), receipt.observed_anchor);
            if session.adapter.action_work_state(receipt.action_id)?
                == EffectWorkState::NeverDispatched
            {
                never.insert(step.id.get());
            }
        } else if objective
            .retired_never_dispatched
            .contains_key(&step.id.get())
            || session.carried.iter().any(|row| {
                row.plan_digest == digest
                    && row.step == Some(step.id)
                    && row.is_final()
                    && row.work_state == EffectWorkState::NeverDispatched
            })
        {
            never.insert(step.id.get());
        }
    }
    let mut sources = BTreeMap::new();
    sources.insert(
        context.anchor.state_hash,
        session.adapter.snapshot().clone(),
    );
    let terminal = if session.durable_scenario.is_some() {
        with_durable_store(|store| {
            if store.commit(session.fortress_id, digest).is_some() {
                return Err(refused(
                    "the original still owns an unfinished durable action commit",
                ));
            }
            let retained = store
                .objective(session.fortress_id, digest)
                .ok_or_else(|| refused("durable original source is unavailable"))?;
            if retained.source != source.source
                || retained.sealed_state_hash != source.sealed_state_hash
                || retained.owner_session_id != source.owner_session_id
                || retained.intent_id != source.intent_id
            {
                return Err(refused(
                    "durable source no longer matches the selected original",
                ));
            }
            let terminal = store
                .retired_commit(session.fortress_id, digest)
                .ok_or_else(|| {
                    refused("withdrawal requires the original's complete archived final receipts")
                })?
                .clone();
            complete_census(plan, &terminal)?;
            for hash in std::iter::once(source.sealed_state_hash).chain(
                terminal
                    .step_anchors
                    .values()
                    .map(|anchor| anchor.state_hash),
            ) {
                check_time()?;
                if let std::collections::btree_map::Entry::Vacant(entry) = sources.entry(hash) {
                    entry.insert(store.load_snapshot(hash)?);
                    // Stop loading archived worlds as soon as the cumulative
                    // source domain reaches the caller's negotiated bound.
                    let admitted: Vec<_> = sources.values().collect();
                    query_budget::admit_sources(&admitted, &context)?;
                    check_time()?;
                }
            }
            Ok(terminal)
        })?
    } else {
        let sealed = session
            .history
            .get(&source.sealed_state_hash)
            .ok_or_else(|| refused("the original sealed world is no longer retained"))?;
        sources.insert(source.sealed_state_hash, sealed.clone());
        live_terminal
    };
    complete_census(plan, &terminal)?;
    let refs: Vec<_> = sources.values().collect();
    query_budget::admit_sources(&refs, &context)?;
    let source_bytes = sources
        .values()
        .map(|snapshot| snapshot.canonical_bytes().len() as u64)
        .sum::<u64>()
        .saturating_add(source_summary(&objective.source).len() as u64)
        .saturating_add(
            objective
                .resource_goal(session)
                .request_json
                .map_or(0, str::len) as u64,
        )
        .saturating_add(
            session
                .commit_receipts
                .get(&digest.to_hex())
                .map_or(0, String::len) as u64,
        );
    let remaining = context
        .budget
        .max_bytes
        .min(MAX_SOURCE_BYTES)
        .saturating_sub(source_bytes);
    let _ = dfmcp_intent::plan_archive::encode_with_check(
        plan,
        dfmcp_intent::plan_archive::ArchiveLimits {
            max_bytes: remaining as usize,
            max_steps: MAX_STEPS,
            ..dfmcp_intent::plan_archive::ArchiveLimits::default()
        },
        check_time,
    )?;
    quiet_at(plan, &never, session.adapter.snapshot())?;
    check_time()?;
    DurableObjectiveWithdrawal::new(
        &source,
        &terminal,
        context.anchor,
        session.session_id,
        never,
    )
}

fn response(session: &LabSession, withdrawal: &DurableObjectiveWithdrawal, already: bool) -> Value {
    let archived = session
        .objectives
        .iter()
        .filter(|goal| goal.is_withdrawn())
        .count()
        + usize::from(!already);
    json!({"ok":true,"session_id":session.session_id.to_string(),
        "plan_digest":withdrawal.plan_digest.to_hex(),"status":"withdrawn",
        "selection_scope":"exact_original_goal","descendant_plans_included":false,
        "already_withdrawn":already,"withdrawal":projection(withdrawal),
        "archived":true,"physical_work_changed":false,"goal_success_claimed":false,
        "active_objectives":session.objectives.len().saturating_sub(archived),
        "archived_objectives":archived,"active_objective_limit":MAX_OBJECTIVES_PER_FORTRESS,
        "archive_limit":MAX_WITHDRAWN_OBJECTIVES_PER_FORTRESS,
        "observed_anchor":anchor_json(&session.adapter.snapshot().anchor())})
}

pub(super) fn withdraw_goal(session: &mut LabSession, digest: &Digest32) -> Result<Value> {
    let withdrawal = prepare(session, *digest)?;
    let already = session
        .objectives
        .iter()
        .any(|goal| goal.plan_digest == *digest && goal.is_withdrawn());
    let output = response(session, &withdrawal, already);
    admit_receipt(session, &output)?;
    if already {
        return Ok(output);
    }
    if session.durable_scenario.is_some() {
        if let Some(fault) = &session.durability_fault {
            return Err(DfmcpError::new(
                ErrorCode::AdapterUnavailable,
                format!("withdrawal requires its already-published frontier: {fault}"),
            ));
        }
        if let Err(error) = with_durable_store(|store| {
            store.persist_objective_withdrawal(&withdrawal, session.adapter.snapshot())
        }) {
            session.durability_fault = Some(format!("{}: {}", error.code.as_str(), error.message));
            return Err(error);
        }
    }
    let objective = session
        .objectives
        .iter_mut()
        .find(|goal| goal.plan_digest == *digest)
        .ok_or_else(|| {
            refused("the selected original disappeared before withdrawal publication")
        })?;
    objective.withdrawal = Some(withdrawal);
    objective.original_handles_live = false;
    objective.active_observation_anchor = None;
    Ok(output)
}

fn admit_receipt(session: &LabSession, output: &Value) -> Result<()> {
    let limit = session
        .budget
        .max_bytes
        .min(u64::from(session.budget.max_output_tokens) * 4);
    // Leave room for the normal Agent Turn envelope before publishing a new
    // ledger disposition. The complete receipt is never trimmed to success.
    if (output.to_string().len() as u64).saturating_add(2_048) > limit {
        return Err(exhausted(
            "the withdrawal receipt and Agent Turn exceed the output budget",
        ));
    }
    Ok(())
}

pub(super) fn offer(session: &LabSession, digest: Digest32) -> Value {
    match prepare(session, digest).and_then(|withdrawal| {
        let already = session
            .objectives
            .iter()
            .any(|goal| goal.plan_digest == digest && goal.is_withdrawn());
        admit_receipt(session, &response(session, &withdrawal, already))?;
        Ok(withdrawal)
    }) {
        Ok(withdrawal) => {
            let already = session
                .objectives
                .iter()
                .any(|goal| goal.plan_digest == digest && goal.is_withdrawn());
            json!({"available":!already,"already_withdrawn":already,
                "scope":"exact_original_goal","required_capability":"plan","required_risk":"reversible",
                "historical_evidence_retained":true,"goal_success_claimed":false,
                "withdrawal":already.then(|| projection(&withdrawal)),
                "request":(!already).then(|| json!({"tool":"fortress.cancel","arguments":{
                    "session_id":session.session_id.to_string(),"scope":format!("goal:{digest}"),"mode":"withdraw_goal"}}))})
        }
        Err(error) => json!({"available":false,"scope":"exact_original_goal",
            "reason":error.message,"code":error.code.as_str(),
            "required_capability":"plan","required_risk":"reversible"}),
    }
}

/// Recheck the archived quiet frontier, independently of the current world or
/// a later restore. This restores history only, never an active pursuit.
pub(super) fn recover(
    objective: &mut Objective,
    store: &DurableLabStore,
    retained: &DurableObjective,
) -> Result<()> {
    let withdrawal = objective
        .withdrawal
        .as_ref()
        .ok_or_else(|| refused("withdrawal marker missing"))?;
    let plan = objective
        .plan
        .as_ref()
        .ok_or_else(|| refused("withdrawn original seal missing"))?;
    let terminal = store
        .retired_commit(retained.fortress_id, retained.plan_digest)
        .ok_or_else(|| refused("withdrawn original final receipts missing"))?;
    complete_census(plan, terminal)?;
    withdrawal.validate_source(retained, terminal)?;
    if !withdrawal.proof_matches(retained, terminal) {
        return Err(refused(
            "withdrawal no longer binds its exact original evidence",
        ));
    }
    let snapshot = store.load_snapshot(withdrawal.anchor.state_hash)?;
    if snapshot.anchor() != withdrawal.anchor || !snapshot.hash_is_valid() {
        return Err(refused("withdrawal lost its exact archived world"));
    }
    quiet_at(plan, &withdrawal.never_dispatched_steps, &snapshot)?;
    if let Some(anchor) =
        store.objective_reserve_observation(retained.fortress_id, retained.plan_digest)
    {
        if objective.selected_wait_reserve_horizon()? != Some(anchor.tick.0)
            || !precedes(plan.anchor, anchor)
            || !precedes(anchor, withdrawal.anchor)
        {
            return Err(refused(
                "withdrawn reserve endpoint escaped its exact original history",
            ));
        }
        let endpoint = store.load_snapshot(anchor.state_hash)?;
        if endpoint.anchor() != anchor {
            return Err(refused("withdrawn reserve endpoint changed"));
        }
        let truth = super::super::selected_plan_wait::observed_truth(
            objective.resource_output_truth_at(&endpoint),
        )?
        .0;
        objective.selected_wait_reserve_observation = Some((anchor, truth));
        objective.selected_wait_reserve_durable = true;
    }
    Ok(())
}
