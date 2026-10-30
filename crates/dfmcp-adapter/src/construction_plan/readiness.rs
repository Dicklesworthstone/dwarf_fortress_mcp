//! Complete construction readiness without positive completion credit.
//!
//! Retained readiness consumes the original acquisition allowance and records
//! source and identity floors. A probe only assesses those same facts. Neither
//! purpose can count a stability sample or satisfy the original goal.
use super::{Goal, LinkedSample, Progress, SamplePurpose};
use dfmcp_core::{OperationContext, Result};

/// Assess one completed readiness acquisition. Only the monitor journal's
/// guarded publication makes this returned transition durable.
pub fn advance(
    state: &Progress,
    goal: &Goal,
    sample: &LinkedSample,
    context: &OperationContext,
) -> Result<Progress> {
    advance_with_counter(state, goal, sample, context, &mut 0)
}

/// Reassess retained readiness without charging another acquisition or adding
/// proof credit. The caller must not publish this result as a goal sample.
pub fn probe(
    state: &Progress,
    goal: &Goal,
    sample: &LinkedSample,
    context: &OperationContext,
) -> Result<Progress> {
    super::advance_for_purpose(state, goal, sample, context, &mut 0, SamplePurpose::Probe)
}

pub(super) fn advance_with_counter(
    state: &Progress,
    goal: &Goal,
    sample: &LinkedSample,
    context: &OperationContext,
    used: &mut u64,
) -> Result<Progress> {
    super::advance_for_purpose(state, goal, sample, context, used, SamplePurpose::Readiness)
}

#[cfg(test)]
mod tests;
