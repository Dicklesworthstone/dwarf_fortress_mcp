//! Bounded witnesses for application-owned Task cancellation replay.
//! Inspection never polls, advances time, grants authority, reconstructs a
//! Task handle or repairs missing custody.

use super::*;

fn digest(domain: &[u8], value: &Value) -> String {
    let mut bytes = domain.to_vec();
    bytes.extend_from_slice(value.to_string().as_bytes());
    Digest32::of_bytes(&bytes).to_hex()
}

fn actions(session: &LabSession, requested: &str) -> Result<Value> {
    let original = original_receipt(session, requested)?;
    let ids = original_actions(session, requested)?;
    if ids.len() > 64 {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "Task replay cannot witness more than 64 original actions",
        ));
    }
    let rows = ids.into_iter().map(|id| {
        let receipt = session.adapter.action_receipt(id).map(|receipt| json!({
            "state":format!("{:?}", receipt.state),
            "digest":receipt.adapter_receipt_digest.to_hex(),
            "anchor":anchor_json(&receipt.observed_anchor),
            "evidence":receipt.evidence.iter().map(|evidence| json!({
                "id":evidence.id.to_string(), "digest":evidence.digest.to_hex(),
                "anchor":anchor_json(&evidence.anchor),
            })).collect::<Vec<_>>(),
        }));
        let work = match session.adapter.action_work_state(id) {
            Ok(work) => effect_work_json(&work),
            Err(error) => json!({"error_code":error.code.as_str(),"message":error.message}),
        };
        let leases = session.leases.by_action.get(&id).map(|(holder, leases)| json!({
            "holder":if *holder == session.session_id {
                "owning_session".to_owned()
            } else {
                holder.to_string()
            },
            "leases":leases.iter().map(ToString::to_string).collect::<Vec<_>>(),
        }));
        json!({
            "action_id":id.to_string(), "receipt":receipt, "work":work,
            "open":session.open_actions.contains(&id), "leases":leases,
        })
    }).collect::<Vec<_>>();
    Ok(json!({
        "plan_id":original["plan_id"], "plan_digest":original["plan_digest"],
        "requested_plan_digest":requested, "actions":rows,
    }))
}

pub(super) fn frontier(session: &LabSession, requested: &str) -> Value {
    let grants = session.grants.iter().map(|grant| json!({
        "capability":grant.capability.as_str(), "max_risk":grant.max_risk.as_str(),
        "expires_at_tick":grant.expires_at_tick.map(|tick| tick.0),
        "remaining_uses":grant.remaining_uses,
        "scope":{
            "fortress_id":grant.scope.fortress_id.map(|id| id.get().to_string()),
            "entity_ids":grant.scope.entity_ids.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "map_area":grant.scope.map_area.map(|area| json!({
                "min":[area.min.x,area.min.y,area.min.z],
                "max":[area.max.x,area.max.y,area.max.z],
            })),
        },
    })).collect::<Vec<_>>();
    let original = match actions(session, requested) {
        Ok(actions) => actions,
        Err(error) => json!({"error_code":error.code.as_str(),"message":error.message}),
    };
    json!({
        "anchor":anchor_json(&session.adapter.snapshot().anchor()),
        "next_request_id":session.next_request_id.to_string(),
        "authority_digest":digest(b"dfmcp-task-replay-authority/1\0", &json!(grants)),
        "original_actions_digest":digest(b"dfmcp-task-replay-original-actions/1\0", &original),
        "budget":{
            "max_wall_millis":session.budget.max_wall_millis,
            "max_game_ticks":session.budget.max_game_ticks,
            "max_entities":session.budget.max_entities,
            "max_bytes":session.budget.max_bytes,
            "max_output_tokens":session.budget.max_output_tokens,
            "max_actions":session.budget.max_actions,
        },
    })
}

/// Read the expected input without publishing objective or snapshot history.
#[cfg(test)]
pub(crate) fn frontier_for(session_id: &str, requested: &str) -> Result<Value> {
    with_task_read(session_id, |session| Ok(frontier(session, requested)))
}


/// Complete comparison and transition under one engine/admission owner. A
/// mismatched expected input publishes no snapshot, goal progress or call log.
pub(crate) enum CheckedPhase {
    InputDivergence(Value),
    Executed {
        result: Result<Value>,
        after: Value,
        evidence_digest: String,
    },
}

pub(crate) fn checked_drain(
    session_id: &str,
    requested: &str,
    finalize: bool,
    expected_before: &Value,
) -> Result<CheckedPhase> {
    let session = resolve_session(Some(session_id.to_owned()))?;
    let _admission = LAB_ADMISSION.read().map_err(|_| session_error())?;
    ensure_durable_owner(&session)?;
    with_session_view(&session, || Err(session_error()), |session| {
        let before = frontier(session, requested);
        if &before != expected_before {
            return Ok(CheckedPhase::InputDivergence(before));
        }
        if session.shared.is_some() || session.durable_scenario.is_some() {
            return Err(DfmcpError::new(
                ErrorCode::CapabilityDenied,
                "Task replay requires a fresh private laboratory owner",
            ));
        }
        // Match the ordinary with_session publication ordering, but only once
        // the exact expected frontier has been admitted under this same lock.
        session.civilian_recovery.observe(session.adapter.snapshot());
        let result = drain_recorded(session, requested, finalize);
        let after = frontier(session, requested);
        let (evidence_digest, _) = session_evidence_digest(&result, session);
        persist_durable_head(session);
        remember_version(session);
        Ok(CheckedPhase::Executed { result, after, evidence_digest })
    }, false)
}

fn certificate_material(payload: &Value) -> Value {
    json!({
        "plan_digest":payload["plan_digest"], "steps":payload["steps"],
        "anchor":payload["observed_anchor"], "proof_status":payload["proof_status"],
        "original_goal":payload["original_goal"],
        "selected_plan_wait":payload["selected_plan_wait"],
        "action_proof_status":payload["action_proof_status"],
        "drain_progress":payload["drain_progress"],
    })
}

/// Only the typed process-owner references are normalized. Goal text, original
/// effect identities, anchors, receipt digests and evidence remain exact.
/// Retained certificates are unchanged. Their local digest must first verify;
/// replay then compares the same material with just owner references mapped.
pub(crate) fn evidence_digest(result: &Result<Value>, session_id: &str) -> String {
    normalized_evidence_digest(result, session_id, None)
}

/// Derive a process-neutral comparison only after verifying the exact typed
/// withdrawal source and terminal census. Unsupported custody remains explicit.
pub(crate) fn session_evidence_digest(
    result: &Result<Value>,
    session: &LabSession,
) -> (String, bool) {
    let session_id = session.session_id.to_string();
    let Some(payload) = result.as_ref().ok()
        .filter(|payload| payload["original_goal"]["withdrawal"].is_object())
    else {
        return (evidence_digest(result, &session_id), true);
    };
    let checked = payload["original_goal"]["plan_digest"].as_str()
        .and_then(|digest| super::super::objectives::replay_withdrawal(session, digest).ok())
        .flatten();
    let verified = checked.as_ref()
        .filter(|(original, _)| *original == payload["original_goal"]["withdrawal"]);
    (
        normalized_evidence_digest(result, &session_id, verified),
        verified.is_some(),
    )
}

fn normalized_evidence_digest(
    result: &Result<Value>,
    session_id: &str,
    withdrawal: Option<&(Value, Value)>,
) -> String {
    let value = match result {
        Err(error) => json!({
            "ok":false, "error_code":error.code.as_str(), "message":error.message,
        }),
        Ok(payload) => {
            let mut payload = payload.clone();
            let certificate_valid = if payload["finalize_certificate"].is_object() {
                let expected = Digest32::of_bytes(certificate_material(&payload).to_string().as_bytes()).to_hex();
                Some(payload["finalize_certificate"]["digest"] == expected)
            } else {
                None
            };
            let withdrawal_integrity = payload["original_goal"]["withdrawal"].is_object()
                .then_some(withdrawal.is_some());
            if let Some((_, normalized)) = withdrawal {
                payload["original_goal"]["withdrawal"] = normalized.clone();
            }
            for path in [
                "/original_goal/owner_session_id",
                "/original_goal/continuation_request/arguments/session_id",
                "/selected_plan_wait/original_owner_session_id",
                "/selected_plan_wait/continuation_request/arguments/session_id",
                "/selected_plan_wait/next_step/arguments/session_id",
            ] {
                if let Some(value) = payload.pointer_mut(path)
                    && value.as_str() == Some(session_id)
                {
                    *value = json!("owning_session");
                }
            }
            if certificate_valid == Some(true) {
                payload["finalize_certificate"]["digest"] = json!(
                    Digest32::of_bytes(certificate_material(&payload).to_string().as_bytes()).to_hex()
                );
            }
            json!({"ok":true,"result":payload,"certificate_integrity":certificate_valid,
                "withdrawal_integrity":withdrawal_integrity})
        }
    };
    digest(b"dfmcp-task-replay-evidence/1\0", &value)
}
