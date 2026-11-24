//! Prefer a compatible retained supply pursuit without changing its source.
//!
//! Discovery chooses an exact original for the ordinary continuation gate.
//! It does not compile effects, authorize a future stage, or publish a review.

use super::*;
use crate::lab_world::ProductionRequest;
use dfmcp_intent::plan_archive;
use std::cmp::Reverse;
use std::time::{Duration, Instant};

const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ORIGINAL_STEPS: usize = 64;

fn exhausted(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

fn check_budget(
    context: &OperationContext,
    started: Instant,
    records: u64,
    bytes: u64,
) -> Result<()> {
    if context.cancellation_requested {
        return Err(DfmcpError::new(
            ErrorCode::CancellationRequested,
            "retained supply-goal discovery was cancelled",
        ));
    }
    if records > u64::from(context.budget.max_entities)
        || bytes > context.budget.max_bytes.min(MAX_SOURCE_BYTES)
    {
        return Err(exhausted(
            "retained supply-goal discovery exceeds its complete source or repeated inspection budget",
        ));
    }
    if started.elapsed() >= Duration::from_millis(context.budget.max_wall_millis) {
        return Err(exhausted(
            "retained supply-goal discovery exceeded its shared inspection deadline",
        ));
    }
    Ok(())
}

/// An older supply source may retain an additional FOOD or DRINK goal whose
/// deficit has since disappeared. Never replace those quotas or import an
/// unrelated furniture project into an urgent supply recommendation.
fn covers(original: &ProductionRequest, proposed: &ProductionRequest) -> bool {
    original
        .quotas()
        .keys()
        .all(|item| matches!(item.as_str(), "FOOD" | "DRINK"))
        && proposed.quotas().iter().all(|(item, minimum)| {
            original
                .quotas()
                .get(item)
                .is_some_and(|original| original >= minimum)
        })
}

/// Prefer the newest compatible retained supply pursuit, keeping all original
/// quotas and policies. Any refusal from its ordinary continuation gate stays
/// unavailable; it cannot select an older pursuit or synthesize a new root.
/// Current effect grants and compilation remain the caller's next gate.
pub(in crate::server) fn supply_recovery_continuation(
    session: &LabSession,
    proposed: &ProductionRequest,
    context: &OperationContext,
) -> Result<Option<PlanSource>> {
    let started = Instant::now();
    if context.session_id != session.session_id
        || context.anchor != session.adapter.snapshot().anchor()
    {
        return Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "retained supply-goal discovery requires the current owning observation",
        ));
    }
    authorize_entry(context, Capability::Observe, RiskTier::ReadOnly)?;
    authorize_entry(context, Capability::Plan, RiskTier::ReadOnly)?;
    if !proposed
        .quotas()
        .keys()
        .all(|item| matches!(item.as_str(), "FOOD" | "DRINK"))
    {
        return Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "supply recovery requires only the registered FOOD and DRINK goals",
        ));
    }
    if session.objectives.len() > MAX_RETAINED_OBJECTIVES_PER_FORTRESS {
        return Err(exhausted(
            "retained supply-goal discovery exceeds the complete goal-book bound",
        ));
    }
    let mut records =
        (session.objectives.len() as u64).saturating_add(session.carried.len() as u64);
    let mut bytes = proposed.canonical_json().len() as u64;
    check_budget(context, started, records, bytes)?;
    let mut identities = BTreeSet::new();
    let mut candidates = Vec::new();
    for objective in &session.objectives {
        if !identities.insert(objective.plan_digest) {
            return Err(DfmcpError::new(
                ErrorCode::CorruptLedger,
                "retained supply-goal discovery found duplicate original custody",
            ));
        }
        let entry = objective.resource_goal(session);
        bytes = bytes
            .saturating_add(entry.request_json.map_or(0, |raw| raw.len() as u64))
            .saturating_add(entry.summary.len() as u64);
        check_budget(context, started, records, bytes)?;
        if let Some(plan) = objective.resource_plan() {
            records = records.saturating_add(plan.steps.len() as u64);
            check_budget(context, started, records, bytes)?;
            // The continuation gate can inspect every retained sibling seal.
            // Admit their complete archives before it traverses any of them.
            let (archive, _) = plan_archive::encode_with_check(
                plan,
                plan_archive::ArchiveLimits {
                    max_bytes: context
                        .budget
                        .max_bytes
                        .min(MAX_SOURCE_BYTES)
                        .saturating_sub(bytes)
                        .min(plan_archive::MAX_ARCHIVE_BYTES as u64)
                        as usize,
                    max_steps: MAX_ORIGINAL_STEPS,
                    ..plan_archive::ArchiveLimits::default()
                },
                || check_budget(context, started, records, bytes),
            )?;
            bytes = bytes.saturating_add(archive.len() as u64);
            check_budget(context, started, records, bytes)?;
        }
        if objective.abandoned(session) || objective.is_withdrawn() {
            continue;
        }
        let (compatible, canonical_bytes) = match &objective.source {
            PlanSource::Production { raw, .. } => {
                let source = ProductionRequest::parse(raw)?;
                (covers(&source, proposed), source.canonical_json().len())
            }
            PlanSource::ProductionContinuation { raw, .. } => {
                let source = goal_continuation::ProductionContinuation::parse(raw)?;
                (
                    covers(source.request(), proposed),
                    source.canonical_json().len(),
                )
            }
            _ => continue,
        };
        // Charge both retained intake and its normalized typed representation;
        // whitespace or compiler defaults cannot evade the source allowance.
        bytes = bytes.saturating_add(canonical_bytes as u64);
        check_budget(context, started, records, bytes)?;
        if compatible {
            let frontier = objective
                .sealed_anchor
                .map(|anchor| (anchor.cursor.epoch, anchor.tick, anchor.cursor.sequence));
            candidates.push((
                Reverse(frontier),
                objective.plan_digest,
                objective.source.requires_complete_source_review()
                    || objective.is_consumption_reserve(),
            ));
        }
    }
    candidates.sort_unstable();
    let Some((_, parent, requires_query)) = candidates.into_iter().next() else {
        return Ok(None);
    };

    let world = query_budget::admit_observation_sources(&[session.adapter.snapshot()], context)?;
    let world_count = |field: &str| {
        world[field].as_u64().ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "supply-goal source admission lost its complete census",
            )
        })
    };
    // Discovery has already inspected the retained book. The one ordinary
    // continuation gate may inspect it and the complete world again; charge
    // both visits under the same allowance before entering that gate.
    let census_records = records.saturating_add(world_count("records")?);
    let census_bytes = bytes.saturating_add(world_count("canonical_bytes")?);
    records = records.saturating_add(census_records);
    bytes = bytes.saturating_add(census_bytes);
    check_budget(context, started, records, bytes)?;
    if requires_query {
        authorize_entry(context, Capability::Query, RiskTier::ReadOnly)?;
    }
    let result = continuation_source(session, parent, None, None);
    check_budget(context, started, records, bytes)?;
    result.map(Some)
}

#[cfg(test)]
#[path = "objective_supply_advice_tests.rs"]
mod tests;
