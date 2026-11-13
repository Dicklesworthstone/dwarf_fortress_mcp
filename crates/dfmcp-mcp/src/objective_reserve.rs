//! Immutable original reserve endpoints across foreground waits and restarts.
//!
//! The durable store retains exact snapshot bytes, not a Boolean conclusion.
//! The unchanged original request and sealed output ledger derive the truth.

use super::*;

fn conflict(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::Conflict, message)
}

fn precedes(before: StateAnchor, after: StateAnchor) -> bool {
    before.fortress_id == after.fortress_id
        && before.tick <= after.tick
        && before.cursor.epoch <= after.cursor.epoch
        && (before.cursor.epoch < after.cursor.epoch
            || before.cursor.sequence < after.cursor.sequence
            || before == after)
}

/// Resolve the same complete original output predicate used by foreground
/// inspection. Missing current ledger identity remains Unknown at that exact
/// snapshot; a later repaired world must not upgrade that immutable result.
fn endpoint_truth(
    objective: &Objective,
    snapshot: &dfmcp_world::WorldSnapshot,
) -> Result<PredicateTruth> {
    objective.bound_resource_output_ledger()?;
    super::super::selected_plan_wait::observed_truth(objective.resource_output_truth_at(snapshot))
        .map(|(truth, _)| truth)
}

pub(super) fn recover_reserve_observation(
    objective: &mut Objective,
    store: &DurableLabStore,
    recovery: &goal_continuation::RecoveredGoalLineage,
) -> Result<()> {
    // Restored history stays in the store for audit, but cannot be inherited
    // as evidence for an abandoned pursuit in a different world lineage.
    if objective.restore_abandoned_anchor.is_some() {
        return Ok(());
    }
    let fortress = recovery.previous_anchor().fortress_id;
    let Some(anchor) = store.objective_reserve_observation(fortress, objective.plan_digest) else {
        return Ok(());
    };
    let sealed = objective
        .sealed_anchor
        .ok_or_else(|| conflict("original reserve observation lacks its verified sealed source"))?;
    if objective.selected_wait_reserve_horizon()? != Some(anchor.tick.0)
        || !precedes(sealed, anchor)
        || !precedes(anchor, recovery.previous_anchor())
        || objective.active_observation_anchor != Some(recovery.active_anchor())
    {
        return Err(DfmcpError::new(
            ErrorCode::CorruptLedger,
            "retained reserve endpoint disagrees with its original horizon or recovery lineage",
        ));
    }
    let snapshot = store.load_snapshot(anchor.state_hash)?;
    if snapshot.anchor() != anchor {
        return Err(DfmcpError::new(
            ErrorCode::CorruptLedger,
            "retained reserve endpoint has no exact archived snapshot",
        ));
    }
    let truth = endpoint_truth(objective, &snapshot)?;
    objective.selected_wait_reserve_observation = Some((anchor, truth));
    objective.selected_wait_reserve_durable = true;
    Ok(())
}

/// Capacity admission must retain an original reserve until its exact endpoint
/// is proved. Predicted funding and elapsed time do not complete that history.
/// Use the same original ledger, immutable source and accepted observation
/// lineage as selected waits; this read never records or repairs an endpoint.
pub(super) fn eviction_is_complete(objective: &Objective, session: &LabSession) -> Result<bool> {
    let Some(until) = objective.selected_wait_reserve_horizon()? else {
        return Ok(true);
    };
    let Some((anchor, PredicateTruth::True)) = objective.selected_wait_reserve_observation else {
        return Ok(false);
    };
    let context = read_context(session);
    authorize_entry(&context, Capability::Observe, RiskTier::ReadOnly)?;
    authorize_entry(&context, Capability::Query, RiskTier::ReadOnly)?;
    let sealed = objective
        .sealed_anchor
        .ok_or_else(|| conflict("reserve eviction lacks its verified original seal"))?;
    let active = objective
        .active_observation_anchor
        .ok_or_else(|| conflict("reserve eviction lacks its accepted active lineage"))?;
    let current = session.adapter.snapshot();
    if objective.abandoned(session)
        || objective.verification_error.is_some()
        || anchor.tick.0 != until
        || !precedes(sealed, anchor)
        || !precedes(anchor, current.anchor())
        || active.cursor.epoch != current.cursor.epoch
        || !precedes(active, current.anchor())
        || (anchor.cursor.epoch != current.cursor.epoch
            && (objective.originating_process || !objective.selected_wait_reserve_durable))
    {
        return Err(conflict(
            "reserve eviction requires the exact original endpoint in its accepted active lineage",
        ));
    }
    let snapshot = if objective.selected_wait_reserve_durable {
        if session.durable_scenario.is_none() {
            return Err(conflict("durable reserve endpoint lost its owning store"));
        }
        with_durable_store(|store| {
            if store.objective_reserve_observation(session.fortress_id, objective.plan_digest)
                != Some(anchor)
            {
                return Err(conflict(
                    "durable reserve endpoint changed or lost original custody",
                ));
            }
            store.load_snapshot(anchor.state_hash)
        })?
    } else if current.anchor() == anchor {
        current.clone()
    } else {
        session
            .history
            .get(&anchor.state_hash)
            .filter(|snapshot| snapshot.anchor() == anchor)
            .cloned()
            .ok_or_else(|| conflict("the exact original reserve snapshot is no longer retained"))?
    };
    let sources = if current.anchor() == anchor {
        vec![current]
    } else {
        vec![current, &snapshot]
    };
    query_budget::admit_sources(&sources, &context)?;
    if !snapshot.hash_is_valid() || snapshot.anchor() != anchor {
        return Err(DfmcpError::new(
            ErrorCode::CorruptLedger,
            "reserve eviction has no exact content-verified endpoint snapshot",
        ));
    }
    Ok(endpoint_truth(objective, &snapshot)? == PredicateTruth::True)
}

/// Load an admitted exact recovered endpoint into the ordinary bounded version
/// book. Subsequent source budgeting and canonical state reads share those
/// same bytes; no new current observation or world mutation occurs.
pub(in super::super) fn ensure_reserve_snapshot(
    session: &mut LabSession,
    digest: Digest32,
) -> Result<()> {
    let context = read_context(session);
    authorize_entry(&context, Capability::Observe, RiskTier::ReadOnly)?;
    authorize_entry(&context, Capability::Query, RiskTier::ReadOnly)?;
    let objective = session
        .objectives
        .iter()
        .find(|objective| objective.plan_digest == digest)
        .ok_or_else(|| conflict("selected reserve original lost custody"))?;
    let Some((anchor, truth)) = objective.selected_wait_reserve_observation else {
        return Ok(());
    };
    if session.adapter.snapshot().anchor() == anchor
        || session
            .history
            .get(&anchor.state_hash)
            .is_some_and(|snapshot| snapshot.anchor() == anchor)
    {
        return Ok(());
    }
    if !objective.selected_wait_reserve_durable
        || session.durable_scenario.is_none()
        || objective.abandoned(session)
        || objective.verification_error.is_some()
    {
        return Err(conflict(
            "the exact original reserve snapshot is no longer retained",
        ));
    }
    let snapshot = with_durable_store(|store| {
        if store.objective_reserve_observation(session.fortress_id, digest) != Some(anchor) {
            return Err(conflict(
                "durable reserve endpoint changed or lost original custody",
            ));
        }
        store.load_snapshot(anchor.state_hash)
    })?;
    query_budget::admit_sources(&[session.adapter.snapshot(), &snapshot], &context)?;
    if snapshot.anchor() != anchor || endpoint_truth(objective, &snapshot)? != truth {
        return Err(DfmcpError::new(
            ErrorCode::CorruptLedger,
            "archived original reserve endpoint disagrees with recovered truth",
        ));
    }
    session.history.record(&snapshot);
    Ok(())
}

/// Retain the first observed endpoint only after the complete world frontier
/// is durable. Failed publication installs no cache and ends this selected
/// request before another progress slice. An uncertain store write retains
/// the existing reopen-and-reconcile fence on later effects.
pub(in super::super) fn retain_reserve_observation(
    session: &mut LabSession,
    digest: Digest32,
    anchor: StateAnchor,
    truth: PredicateTruth,
) -> Result<bool> {
    let context = read_context(session);
    authorize_entry(&context, Capability::Observe, RiskTier::ReadOnly)?;
    authorize_entry(&context, Capability::Query, RiskTier::ReadOnly)?;
    let objective = session
        .objectives
        .iter()
        .find(|objective| objective.plan_digest == digest)
        .ok_or_else(|| conflict("selected reserve original lost custody"))?;
    if let Some(previous) = objective.selected_wait_reserve_observation {
        return if previous == (anchor, truth) {
            Ok(objective.selected_wait_reserve_durable)
        } else {
            Err(conflict(
                "the first original reserve observation cannot be replaced",
            ))
        };
    }
    if objective.abandoned(session)
        || objective.verification_error.is_some()
        || objective.selected_wait_reserve_horizon()? != Some(anchor.tick.0)
        || session.adapter.snapshot().anchor() != anchor
        || endpoint_truth(objective, session.adapter.snapshot())? != truth
    {
        return Err(conflict(
            "reserve capture requires the unchanged original at its exact current endpoint",
        ));
    }
    let durable = session.durable_scenario.is_some();
    if durable {
        if let Some(fault) = &session.durability_fault {
            return Err(DfmcpError::new(
                ErrorCode::AdapterUnavailable,
                format!("reserve endpoint cannot be retained before its world frontier: {fault}"),
            ));
        }
        if let Err(error) = with_durable_store(|store| {
            store.persist_objective_reserve_observation(digest, session.adapter.snapshot())
        }) {
            session.durability_fault = Some(format!("{}: {}", error.code.as_str(), error.message));
            return Err(error);
        }
    }
    let objective = session
        .objectives
        .iter_mut()
        .find(|objective| objective.plan_digest == digest)
        .ok_or_else(|| conflict("selected reserve original disappeared during publication"))?;
    objective.selected_wait_reserve_observation = Some((anchor, truth));
    objective.selected_wait_reserve_durable = durable;
    session.history.record(session.adapter.snapshot());
    Ok(durable)
}

#[cfg(test)]
#[path = "objective_reserve_tests.rs"]
mod tests;
