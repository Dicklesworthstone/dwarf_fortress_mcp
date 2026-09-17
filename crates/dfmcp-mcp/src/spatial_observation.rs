//! A single acceptance boundary for live spatial captures, with or without a
//! journal. Receiving bytes never publishes an unauthorized or over-budget world.
//! The session mutex is held by the caller throughout this foreground operation.
use super::*;
use std::time::Instant;
#[path = "spatial_bootstrap.rs"]
mod bootstrap;
pub(super) use bootstrap::connect_and_capture;

fn remaining(context: &OperationContext, elapsed: Duration) -> Result<Duration> {
    context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
    bootstrap::allowance(context.budget, elapsed)
}

fn custody(session: &mut Session, context: &OperationContext) -> Result<()> {
    if let Some(journal) = session.journal.as_mut() {
        journal.validate_custody(context)?;
        if journal.recovery_only()
            || journal.state().snapshot().map(|snapshot| snapshot.anchor()) != Some(context.anchor) {
            return Err(error(ErrorCode::CorruptLedger,
                "live spatial journal is read-only or disagrees with the published anchor"));
        }
    }
    Ok(())
}

struct Candidate {
    observation: LiveSpatialCitizenObservation,
    state: LiveSpatialCitizenState,
    outcome: JobPublication,
    target: OperationContext,
}

fn check_bounds(observation: &LiveSpatialCitizenObservation, limits: CitizenSpatialLimits) -> Result<()> {
    let op = observation.spatial().operations();
    let counts = limits.spatial.operations;
    if op.jobs.jobs.len() > counts.jobs as usize
        || op.buildings.len() > counts.buildings as usize
        || op.items.len() > counts.items as usize
        || observation.citizens().len() > limits.citizens as usize
        || observation.spatial().terrain().map.region != limits.spatial.region
        || observation.encode_payload()?.len() > counts.payload_bytes {
        return Err(error(ErrorCode::BudgetExceeded,
            "spatial/1.8 observation exceeds negotiated acquisition bounds"));
    }
    Ok(())
}

fn stage(session: &Session, context: &OperationContext,
    observation: LiveSpatialCitizenObservation) -> Result<Candidate> {
    check_bounds(&observation, session.limits)?;
    // The candidate includes entity-generation history. A rejected projection
    // must not burn a generation or advance the session's source observation.
    let mut state = session.state.clone();
    let outcome = state.publish(observation.clone())?;
    let snapshot = state.snapshot().ok_or_else(|| error(ErrorCode::InternalInvariantViolation,
        "staged spatial observation has no canonical snapshot"))?;
    if snapshot.graph.entities.len() > context.budget.max_entities as usize {
        return Err(error(ErrorCode::BudgetExceeded,
            "staged spatial projection exceeds the request entity allowance"));
    }
    let mut target = context.clone();
    target.anchor = snapshot.anchor();
    // Never use a prior observation's tick or fortress to authorize new facts.
    // Observe-only, nonjournaled sessions remain valid; persistence requires
    // Query as well, both before acquisition and at the candidate anchor.
    target.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
    if session.journal.is_some() {
        target.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    }
    Ok(Candidate { observation, state, outcome, target })
}

pub(super) fn refresh(session: &mut Session, context: &OperationContext) -> Result<JobPublication> {
    let started = Instant::now();
    refresh_with_clock(session, context, || started.elapsed())
}

fn refresh_with_clock(session: &mut Session, context: &OperationContext,
    mut elapsed: impl FnMut() -> Duration) -> Result<JobPublication> {
    if session.source.closed() {
        return Err(error(ErrorCode::SessionNotFound, "spatial session is closed"));
    }
    if session.source.archive_only() {
        return Err(error(ErrorCode::CapabilityDenied, "archive-only sessions cannot acquire live observations"));
    }
    context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
    if context.session_id != session.id || context.anchor != session.anchor()? {
        return Err(error(ErrorCode::StaleAnchor, "spatial refresh names another session or observation"));
    }
    if session.source.poisoned() {
        return Err(error(ErrorCode::AdapterUnavailable, "spatial source is fenced; reopen the session"));
    }
    if let Err(failure) = custody(session, context) {
        if failure.code == ErrorCode::CorruptLedger { session.source.fence(); }
        return Err(failure);
    }
    // Preflight refusal has not touched the source and does not poison it.
    let allowance = remaining(context, elapsed())?;
    let result = (|| {
        let observation = session.source.read(allowance)?;
        // An injected or slow source cannot escape the enclosing deadline just
        // because its own transport returned successfully.
        remaining(context, elapsed())?;
        let candidate = stage(session, context, observation)?;
        custody(session, context)?;
        let allowance = remaining(&candidate.target, elapsed())?;
        match session.journal.as_mut() {
            Some(journal) => {
                let mut write_context = context.clone();
                write_context.budget.max_bytes = session.limits.spatial.operations.payload_bytes as u64;
                write_context.budget.max_wall_millis = allowance.as_millis() as u64;
                // append revalidates custody and Observe at its own canonical
                // candidate anchor, then syncs before publishing the journal root.
                // Do not retain both staging copies through the durable append.
                drop(candidate.state);
                let outcome = journal.append(candidate.observation, &write_context)?;
                session.state = journal.state().clone();
                // No fallible post-sync deadline check may hide a committed root.
                Ok(outcome)
            }
            None => {
                session.state = candidate.state;
                Ok(candidate.outcome)
            }
        }
    })();
    // A consumed-but-rejected capture cannot be silently treated as the current
    // source. Preserve the old world and require the explicit close/reopen path.
    if result.is_err() { session.source.fence(); }
    result
}

#[cfg(all(test, unix))]
#[path = "spatial_observation_tests.rs"]
mod tests;
