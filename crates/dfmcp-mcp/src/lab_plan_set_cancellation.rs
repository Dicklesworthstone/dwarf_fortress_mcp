//! One foreground drain of an explicitly selected set of live originals.
//!
//! Admission is atomic: every original, current grant, inverse, lease and shared
//! budget is checked before the existing drain engine can pause or stop work.
//! Effects remain progressively durable; an execution/publication failure is an
//! honest partial outcome, never an implied rollback or a session-wide proof.
use std::collections::BTreeSet;
use std::time::Instant;

use dfmcp_adapter::CancelMode;
use dfmcp_core::{ActionId, DfmcpError, Digest32, ErrorCode, Result};
use dfmcp_intent::EffectWorkState;
use serde_json::{Value, json};

use super::{LabSession, plan_cancellation};

const MAX_ORIGINALS: usize = 8;
const MAX_SCOPE_BYTES: usize = 6 + MAX_ORIGINALS * 64 + MAX_ORIGINALS - 1;
const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;

fn refused(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::Conflict, message)
}

fn exhausted(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

/// Closed syntax keeps the existing eleven-tool API and Rust handler stable.
/// Canonical request order is independent of the order supplied by the caller.
pub(crate) fn parse_scope(raw: &str) -> Result<Vec<Digest32>> {
    let invalid = || {
        DfmcpError::new(
            ErrorCode::InvalidRequest,
            "selected-set cancellation requires plans:<digest>[,<digest>...] with one to eight distinct 64-lowercase-hex digests",
        )
    };
    if raw.len() > MAX_SCOPE_BYTES {
        return Err(invalid());
    }
    let members = raw.strip_prefix("plans:").ok_or_else(invalid)?;
    let mut digests = BTreeSet::new();
    for member in members.split(',') {
        let digest = plan_cancellation::parse_scope(&format!("plan:{member}"))?;
        if !digests.insert(digest) || digests.len() > MAX_ORIGINALS {
            return Err(invalid());
        }
    }
    Ok(digests.into_iter().collect())
}

struct Selection {
    originals: Vec<plan_cancellation::Selection>,
    actions: Vec<ActionId>,
    identities: Value,
    digest: Digest32,
    receipt_bytes: u64,
}

fn check_time(session: &LabSession, started: Instant) -> Result<()> {
    if started.elapsed().as_nanos().div_ceil(1_000_000)
        >= u128::from(session.budget.max_wall_millis)
    {
        return Err(exhausted(
            "selected cancellation exhausted its shared wall-time allowance",
        ));
    }
    Ok(())
}

fn charge(session: &LabSession, started: Instant, bytes: u64, records: u64) -> Result<()> {
    check_time(session, started)?;
    if bytes > session.budget.max_bytes.min(MAX_SOURCE_BYTES)
        || records > u64::from(session.budget.max_entities)
    {
        return Err(exhausted(
            "the complete selected original set exceeds its shared source allowance; select fewer originals",
        ));
    }
    Ok(())
}

fn select(session: &LabSession, requested: &[Digest32], started: Instant) -> Result<Selection> {
    session.budget.validate()?;
    let mut originals = Vec::with_capacity(requested.len());
    let mut actuals = BTreeSet::new();
    let mut actions = BTreeSet::new();
    let mut receipt_bytes = 0u64;
    for digest in requested {
        check_time(session, started)?;
        let raw = session.commit_receipts.get(&digest.to_hex()).ok_or_else(|| refused(
            "every selected original must have complete live commit custody in this session; recovered, foreign, pending and unknown originals cannot participate in a selected-set drain"))?;
        receipt_bytes = receipt_bytes.saturating_add(raw.len() as u64);
        charge(
            session,
            started,
            receipt_bytes,
            requested.len() as u64 + actions.len() as u64,
        )?;
        let original = plan_cancellation::select_for_census(session, *digest)?;
        if !actuals.insert(original.actual) {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "different requested digests resolve to the same original commit; select each actual original only once",
            ));
        }
        for id in &original.actions {
            if !actions.insert(*id) {
                return Err(refused(
                    "two selected originals claim the same action identity",
                ));
            }
        }
        originals.push(original);
    }
    // Identities use canonical requested order. Execution separately follows
    // original commit order so dependents stop before earlier prerequisites.
    let identities = json!({"schema":"dfmcp-selected-originals-cancellation-selection/1",
        "session_id":session.session_id.to_string(),
        "originals":originals.iter().map(|original| json!({
            "requested_plan_digest":original.requested.to_hex(),
            "plan_digest":original.actual.to_hex(),"plan_id":original.plan_id,
            "actions":original.actions.iter().map(ToString::to_string).collect::<Vec<_>>()
        })).collect::<Vec<_>>()});
    let digest = Digest32::of_bytes(identities.to_string().as_bytes());
    originals.sort_by_key(|original| {
        // Complete selection above proved that every action retains this
        // immutable original receipt. The session lock spans admission/drain.
        let receipt = session.adapter.action_plan_receipt(original.actions[0]);
        receipt.map(|receipt| {
            (
                receipt.observed_anchor.cursor.epoch,
                receipt.observed_anchor.cursor.sequence,
                original.actual,
            )
        })
    });
    let actions = originals
        .iter()
        .flat_map(|original| original.actions.iter().copied())
        .collect();
    Ok(Selection {
        originals,
        actions,
        identities,
        digest,
        receipt_bytes,
    })
}

fn authorize(
    session: &LabSession,
    mode: CancelMode,
    scope: &str,
    requested: &[Digest32],
    selected: &Selection,
    started: Instant,
) -> Result<()> {
    // Preserve the complete identity object in both the outcome and final
    // certificate, with room for compact orientation, emergency and failure
    // evidence. Refuse before effects rather than losing a selected member.
    let output_bytes = 8_192u64
        .saturating_add((selected.identities.to_string().len() as u64).saturating_mul(2))
        .saturating_add((scope.len() as u64).saturating_mul(2))
        .saturating_add((requested.len() as u64).saturating_mul(68));
    if output_bytes > session.budget.max_bytes
        || output_bytes.div_ceil(4) > u64::from(session.budget.max_output_tokens)
    {
        return Err(exhausted(
            "selected-set cancellation cannot reserve its complete original identity and drain outcome within the output allowance",
        ));
    }
    // Count the selected actions and every stop/inverse plus at most one pause
    // against one allowance before admitting any individual original.
    super::authorize_drain_budget(session, &selected.actions, mode)?;
    check_time(session, started)?;
    let mut context = super::read_context(session);
    context.budget.max_wall_millis = context
        .budget
        .max_wall_millis
        .checked_sub(started.elapsed().as_nanos().div_ceil(1_000_000) as u64)
        .filter(|remaining| *remaining > 0)
        .ok_or_else(|| {
            exhausted("selected cancellation exhausted its shared wall-time allowance")
        })?;
    let source =
        super::query_budget::admit_observation_sources(&[session.adapter.snapshot()], &context)?;
    let mut bytes = source["canonical_bytes"]
        .as_u64()
        .ok_or_else(|| refused("selected cancellation source size is unavailable"))?
        .saturating_add(selected.receipt_bytes)
        .saturating_add(output_bytes);
    let mut records = source["records"]
        .as_u64()
        .ok_or_else(|| refused("selected cancellation source count is unavailable"))?
        .saturating_add(selected.actions.len() as u64)
        .saturating_add(selected.originals.len() as u64)
        .saturating_add(session.leases.by_action.len() as u64);
    let mut leases = session.leases.manager.active_lease_count() as u64;
    charge(session, started, bytes, records)?;
    for _ in session.adapter.retained_action_ids() {
        records = records.saturating_add(1);
        bytes = bytes.saturating_add(16);
        charge(session, started, bytes, records)?;
    }
    for original in &selected.originals {
        plan_cancellation::authorize(session, original, mode)?;
        check_time(session, started)?;
        for id in &original.actions {
            if super::action_fully_drained(&session.adapter, *id) {
                continue;
            }
            let work = session.adapter.action_work_state(*id)?;
            if matches!(work, EffectWorkState::Unknown { .. }) {
                return Err(refused(
                    "selected original has unresolved physical work identity",
                ));
            }
            if session
                .leases
                .by_action
                .get(id)
                .is_some_and(|(owner, _)| *owner != session.session_id)
            {
                return Err(refused(
                    "selected original action has contradictory foreign ownership",
                ));
            }
            let step = session
                .adapter
                .action_step(*id)
                .ok_or_else(|| refused("selected original lost its action definition"))?;
            leases = leases.saturating_add(
                super::recovered_plan_cancellation::live_scope_lease_count(&step.action),
            );
            if mode == CancelMode::CompensateReversible
                && session
                    .adapter
                    .action_receipt(*id)
                    .is_some_and(|receipt| !receipt.state.is_terminal())
                && work != EffectWorkState::NeverDispatched
                && let Some(inverse) = &step.compensation
            {
                leases = leases.saturating_add(
                    super::recovered_plan_cancellation::live_scope_lease_count(inverse),
                );
            }
        }
    }
    bytes = bytes.saturating_add(leases.saturating_mul(192));
    records = records.saturating_add(leases);
    charge(session, started, bytes, records)?;
    let mut reservations = session.leases.manager.clone();
    for id in &selected.actions {
        if super::action_fully_drained(&session.adapter, *id) {
            continue;
        }
        let step = session
            .adapter
            .action_step(*id)
            .ok_or_else(|| refused("selected original lost its action definition"))?;
        super::recovered_plan_cancellation::admit_live_scope(
            session,
            &mut reservations,
            &step.action,
            started,
        )?;
        if mode == CancelMode::CompensateReversible
            && session
                .adapter
                .action_receipt(*id)
                .is_some_and(|receipt| !receipt.state.is_terminal())
            && session.adapter.action_work_state(*id)? != EffectWorkState::NeverDispatched
            && let Some(inverse) = &step.compensation
        {
            super::recovered_plan_cancellation::admit_live_scope(
                session,
                &mut reservations,
                inverse,
                started,
            )?;
        }
    }
    check_time(session, started)
}

fn identify(payload: &mut Value, requested: &[Digest32], selected: Option<&Selection>) {
    payload["selection_scope"] = json!("exact_original_set");
    payload["requested_plan_digests"] =
        json!(requested.iter().map(Digest32::to_hex).collect::<Vec<_>>());
    payload["selection_resolved"] = json!(selected.is_some());
    payload["selected_plans"] =
        selected.map_or(Value::Null, |selected| selected.identities.clone());
    payload["selected_plans_digest"] =
        selected.map_or(Value::Null, |selected| json!(selected.digest.to_hex()));
    payload["descendant_plans_included"] = json!(false);
    payload["unselected_plans_included"] = json!(false);
    payload["drain_progress"]["original_plans_total"] = json!(requested.len());
    if selected.is_none() {
        // Unresolved custody cannot prove that the requested originals contain
        // zero actions or that none of their physical work remains.
        for field in ["actions_total", "remaining_nonterminal", "remaining_work"] {
            payload["drain_progress"][field] = Value::Null;
        }
    }
}

fn refusal(
    session: &LabSession,
    scope: &str,
    requested: &[Digest32],
    selected: Option<&Selection>,
    error: &DfmcpError,
) -> String {
    let actions = selected.map_or(&[][..], |selected| selected.actions.as_slice());
    let mut payload: Value = serde_json::from_str(&super::drain_refusal_payload(
        session, scope, actions, error, None,
    ))
    .unwrap_or_else(|_| json!({"ok":false}));
    identify(&mut payload, requested, selected);
    payload["error"]["effect_may_have_occurred"] = json!(false);
    payload.to_string()
}

pub(crate) fn cancel(
    session: &mut LabSession,
    mode: CancelMode,
    requested: Vec<Digest32>,
) -> String {
    let started = Instant::now();
    let scope = format!(
        "plans:{}",
        requested
            .iter()
            .map(Digest32::to_hex)
            .collect::<Vec<_>>()
            .join(",")
    );
    let selected = match select(session, &requested, started) {
        Ok(selected) => selected,
        Err(error) => return refusal(session, &scope, &requested, None, &error),
    };
    if let Err(error) = authorize(session, mode, &scope, &requested, &selected, started) {
        return refusal(session, &scope, &requested, Some(&selected), &error);
    }
    let mut payload: Value = match serde_json::from_str(&super::drain_actions_at(
        session,
        mode,
        &scope,
        selected.actions.clone(),
        started,
    )) {
        Ok(payload) => payload,
        Err(_) => {
            let mut failure: Value = serde_json::from_str(&refusal(session, &scope, &requested, Some(&selected),
                &DfmcpError::new(ErrorCode::InternalInvariantViolation,
                    "selected cancellation returned an unreadable outcome; reconciliation is required")))
                .unwrap_or_else(|_| json!({"ok":false}));
            failure["error"]["effect_may_have_occurred"] = json!(true);
            return failure.to_string();
        }
    };
    identify(&mut payload, &requested, Some(&selected));
    if payload["error"].is_object() {
        payload["error"]["effect_may_have_occurred"] = json!(
            payload["steps"]
                .as_array()
                .is_some_and(|steps| !steps.is_empty())
                || payload["emergency_pause"].is_object()
        );
    }
    if payload["finalize_certificate"].is_object() {
        let canonical = json!({"schema":"dfmcp-selected-originals-drain-certificate/1",
            "scope":scope,"session_id":session.session_id.to_string(),
            "selected_plans_digest":selected.digest.to_hex(),"selected_plans":selected.identities,
            "original_action_certificate":payload["finalize_certificate"],
            "anchor":payload["observed_anchor"],"publication_confirmed":true,
            "descendant_plans_included":false,"unselected_plans_included":false});
        let mut certificate = canonical.clone();
        certificate["digest"] =
            json!(Digest32::of_bytes(canonical.to_string().as_bytes()).to_hex());
        certificate["statement"] = json!(
            "every explicitly selected original action is terminal and its exact physical work is currently quiescent; other originals and descendants are outside this proof"
        );
        payload["finalize_certificate"] = certificate;
    }
    payload.to_string()
}
