//! Exact original-plan cancellation with separate live and recovered custody.
//!
//! The selector names the original commit, not whichever plan happens to be
//! latest. Complete durable custody can select fresh recovered cleanup; it and
//! another session's shared-world receipts never replace current effect authority.

use dfmcp_adapter::CancelMode;
use dfmcp_core::{ActionId, Capability, DfmcpError, Digest32, ErrorCode, Result, RiskTier};
use serde_json::{Value, json};

use super::LabSession;

const MAX_ACTIONS: usize = 64;
const MAX_RECEIPT_BYTES: u64 = 1024 * 1024;

fn refused(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::Conflict, message)
}

pub(crate) fn parse_scope(raw: &str) -> Result<Digest32> {
    let digest = raw.strip_prefix("plan:").ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::InvalidRequest,
            "expected plan:<original-plan-digest>",
        )
    })?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "selected-plan cancellation requires exactly 64 lowercase hexadecimal digits",
        ));
    }
    Digest32::from_hex(digest)
        .ok_or_else(|| DfmcpError::new(ErrorCode::InvalidRequest, "invalid original plan digest"))
}

pub(super) struct Selection {
    pub(super) requested: Digest32,
    pub(super) actual: Digest32,
    pub(super) plan_id: String,
    pub(super) actions: Vec<ActionId>,
}

/// Admit the complete original action set before the first effect. In particular,
/// never certify a truncated client-visible receipt as a complete plan drain.
pub(super) fn select(session: &LabSession, requested: Digest32) -> Result<Selection> {
    select_custody(session, requested, session.budget.max_actions as usize)
}

/// A complete source census does not spend an effect allowance on already quiet
/// history. The aggregate coordinator separately admits every selected action.
pub(super) fn select_for_census(session: &LabSession, requested: Digest32) -> Result<Selection> {
    select_custody(session, requested, MAX_ACTIONS)
}

fn select_custody(session: &LabSession, requested: Digest32, action_limit: usize) -> Result<Selection> {
    session.budget.validate()?;
    let raw = session.commit_receipts.get(&requested.to_hex()).ok_or_else(|| {
        refused("no original commit owned by this session matches the selected digest; pending plans, restored sources and other sessions do not grant cancellation authority")
    })?;
    if raw.len() as u64 > session.budget.max_bytes.min(MAX_RECEIPT_BYTES) {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "the complete original commit receipt exceeds the cancellation source byte budget",
        ));
    }
    let value: Value = serde_json::from_str(raw)
        .map_err(|_| refused("the retained original commit receipt is unreadable"))?;
    let owner = session.session_id.to_string();
    if value["ok"] != true || value["session_id"].as_str() != Some(owner.as_str()) {
        return Err(refused(
            "the original commit receipt does not belong to this session",
        ));
    }
    let digest = value["plan_digest"]
        .as_str()
        .ok_or_else(|| refused("the original commit has no sealed plan digest"))?;
    let actual = parse_scope(&format!("plan:{digest}"))
        .map_err(|_| refused("the original commit digest is not canonical"))?;
    let rows = value["actions"]
        .as_array()
        .ok_or_else(|| refused("the original commit has no complete action list"))?;
    if rows.is_empty() {
        return Err(refused(
            "an empty original commit cannot establish selected-plan quiescence",
        ));
    }
    if rows.len() > MAX_ACTIONS
        || rows.len() > action_limit
        || rows.len() as u64 > u64::from(session.budget.max_entities)
    {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "the complete original action set exceeds the selected-plan cancellation budget",
        ));
    }
    let mut actions = Vec::with_capacity(rows.len());
    for row in rows {
        let raw_id = row["action_id"]
            .as_str()
            .ok_or_else(|| refused("an original action identity is missing"))?;
        let number = u128::from_str_radix(raw_id, 16)
            .map_err(|_| refused("an original action identity is invalid"))?;
        let id = ActionId::new(number);
        if id.to_string() != raw_id || actions.contains(&id) {
            return Err(refused(
                "the original action list is noncanonical or contains duplicates",
            ));
        }
        actions.push(id);
    }
    let original = session
        .adapter
        .action_plan_receipt(actions[0])
        .ok_or_else(|| {
            refused("the original adapter commit is no longer retained; reconciliation is required")
        })?;
    let plan_id = original.plan_id.to_string();
    if original.plan_digest != actual
        || value["plan_id"].as_str() != Some(plan_id.as_str())
        || original.actions.len() != actions.len()
    {
        return Err(refused(
            "the selected receipt no longer names the complete original adapter commit",
        ));
    }
    for ((id, row), original_row) in actions.iter().zip(rows).zip(&original.actions) {
        let retained = session
            .adapter
            .action_receipt(*id)
            .ok_or_else(|| refused("an original action receipt is no longer retained"))?;
        let step = session
            .adapter
            .action_step(*id)
            .ok_or_else(|| refused("an original action definition is no longer retained"))?;
        let owner = session
            .adapter
            .action_plan_receipt(*id)
            .ok_or_else(|| refused("an original action lost its plan custody"))?;
        if original_row.action_id != *id
            || row["step"].as_u64() != Some(u64::from(original_row.step_id.get()))
            || retained.step_id != original_row.step_id
            || step.id != original_row.step_id
            || owner.plan_id != original.plan_id
            || owner.plan_digest != original.plan_digest
        {
            return Err(refused(
                "original action order, step identity or plan custody changed",
            ));
        }
    }
    Ok(Selection {
        requested,
        actual,
        plan_id,
        actions,
    })
}

/// Check every selected live obligation before any drain begins. The adapter
/// still checks each effect against a fresh context at its own boundary.
pub(super) fn authorize(session: &LabSession, selected: &Selection, mode: CancelMode) -> Result<()> {
    let context = super::read_context(session);
    super::authorize_entry(&context, Capability::Observe, RiskTier::ReadOnly)?;
    super::authorize_drain_budget(session, &selected.actions, mode)?;
    for id in &selected.actions {
        if super::action_fully_drained(&session.adapter, *id) {
            continue;
        }
        let step = session
            .adapter
            .action_step(*id)
            .ok_or_else(|| refused("the selected original action definition is missing"))?;
        let scope = step.action.scope();
        context.authorize(
            step.required_capability,
            step.risk,
            &scope.entity_ids,
            scope.map_area,
        )?;
        // The adapter checks the inverse independently even for an undispatched
        // prepared step. Admit every current grant before a newer original can
        // be stopped; a terminal original only needs physical cleanup.
        if mode == CancelMode::CompensateReversible
            && session.adapter.action_receipt(*id).is_some_and(|receipt| !receipt.state.is_terminal())
            && let Some(compensation) = &step.compensation
        {
            if matches!(compensation, dfmcp_intent::Action::Extension { .. })
                || !dfmcp_adapter::GameAdapter::identity(&session.adapter)
                    .capabilities.contains(&compensation.capability())
            {
                return Err(DfmcpError::new(ErrorCode::AdapterRejected,
                    "laboratory adapter cannot execute the compensation action"));
            }
            let inverse_scope = compensation.scope();
            context.authorize(compensation.capability(), compensation.risk(),
                &inverse_scope.entity_ids, inverse_scope.map_area)?;
            session.adapter.validate_action_compensation(*id)?;
            if compensation.naturally_temporal()
                && session.adapter.action_work_state(*id)? != dfmcp_intent::EffectWorkState::NeverDispatched
            {
                return Err(DfmcpError::new(ErrorCode::CancellationIncomplete,
                    "temporal compensation requires a separate bounded plan and proof"));
            }
        }
    }
    if mode == CancelMode::EmergencyPauseAndDrain {
        super::authorize_entry(&context, Capability::ControlClock, RiskTier::Reversible)?;
    }
    Ok(())
}

pub(crate) fn cancel(session: &mut LabSession, mode: CancelMode, digest: Digest32) -> String {
    // Present live custody remains authoritative, including malformed receipts.
    // Only its absence permits selection of an exact verified durable original.
    if !session.commit_receipts.contains_key(&digest.to_hex()) {
        return super::recovered_plan_cancellation::cancel(session, mode, digest);
    }
    let selected = match select(session, digest).and_then(|selected| {
        authorize(session, &selected, mode)?;
        Ok(selected)
    }) {
        Ok(selected) => selected,
        Err(error) => return super::dfmcp_error_payload("fortress.cancel", &error),
    };
    // Keep the requested exact identity in the certificate. Witness-rebased
    // aliases also disclose the actual sealed original; neither chooses a
    // different latest plan or a continuation descendant.
    let scope = format!("plan:{}", selected.requested);
    let mut payload: Value = serde_json::from_str(&super::drain_actions(
        session,
        mode,
        &scope,
        selected.actions,
    ))
    .unwrap_or_else(|_| json!({"ok":false}));
    payload["requested_plan_digest"] = json!(selected.requested.to_hex());
    payload["plan_digest"] = json!(selected.actual.to_hex());
    payload["plan_id"] = json!(selected.plan_id);
    payload["selection_scope"] = json!("exact_original_commit");
    payload["descendant_plans_included"] = json!(false);
    payload.to_string()
}

/// Read-only affordance for typed explanation. Bind this projection into the
/// explanation pin: changing custody or grants at the same world anchor must
/// invalidate an old page. No operation context is consumed and no work polls.
pub(crate) fn offer(session: &LabSession, digest: Digest32) -> Value {
    if !session.commit_receipts.contains_key(&digest.to_hex()) {
        return super::recovered_plan_cancellation::offer(session, digest);
    }
    let selected = match select(session, digest).and_then(|selected| {
        authorize(session, &selected, CancelMode::StopFutureSteps)?;
        Ok(selected)
    }) {
        Ok(selected) => selected,
        Err(error) => {
            return json!({"available":false,
            "reason":error.message,"code":error.code.as_str(),
            "scope":"exact_original_commit"});
        }
    };
    if selected
        .actions
        .iter()
        .all(|id| super::action_fully_drained(&session.adapter, *id))
    {
        return json!({"available":false,"reason":"original actions are already terminal and physically quiescent",
            "scope":"exact_original_commit"});
    }
    json!({"available":true,"scope":"exact_original_commit",
        "plan_digest":selected.actual.to_hex(),"descendant_plans_included":false,
        "request":{"tool":"fortress.cancel","arguments":{
            "session_id":session.session_id.to_string(),
            "scope":format!("plan:{}",selected.requested),
            "mode":"stop_future_steps"}},
        "effect":"stop selected original work; retain historical proof and unrelated plans",
        "authority":"current authority is rechecked when cancellation is called"})
}
