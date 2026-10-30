//! Complete room evidence used to decide whether a separate clock interval may
//! proceed. A retained readiness acquisition consumes the original monitor's
//! observation allowance and updates all source, clock and condition floors.
//! It never creates positive completion credit or satisfies the room goal.
//!
//! Compatible whole-room truth may preserve an existing unfinished streak.
//! Unknown, unmet, interrupted or contradictory evidence withdraws that streak
//! through the same reducer used for ordinary room completion samples.

use dfmcp_core::{OperationContext, Result};

use super::{Definition, Progress, Sample};
use crate::construction_plan::SamplePurpose;

/// Assess one completed acquisition without crediting a completion sample.
/// The caller must hold a read intent obtained from `Progress::begin_read`.
/// Durable callers use `MonitorStore::publish_readiness` instead of storing
/// this derived projection themselves.
pub fn advance(
    state: &Progress,
    definition: &Definition,
    sample: &Sample,
    context: &OperationContext,
) -> Result<Progress> {
    advance_with_counter(state, definition, sample, context, &mut 0)
}

/// Validate readiness against prior or already acknowledged room progress.
/// This pure reassessment grants no observation, stability or effect authority
/// and does not charge a second acquisition against the original lifetime.
pub fn probe(
    state: &Progress,
    definition: &Definition,
    sample: &Sample,
    context: &OperationContext,
) -> Result<Progress> {
    super::advance_for_purpose(
        state,
        definition,
        sample,
        context,
        &mut 0,
        SamplePurpose::Probe,
    )
}

pub(super) fn advance_with_counter(
    state: &Progress,
    definition: &Definition,
    sample: &Sample,
    context: &OperationContext,
    used: &mut u64,
) -> Result<Progress> {
    super::advance_for_purpose(
        state,
        definition,
        sample,
        context,
        used,
        SamplePurpose::Readiness,
    )
}
