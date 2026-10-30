//! Crate-private selection of one step without slicing or resealing its parent.
//!
//! Structural selection grants no dependency, source, native or capability
//! authority. Only the combined owner may expose this path, after validating its
//! complete supported plan and the real retained predecessor custody.

use dfmcp_core::{DfmcpError, ErrorCode, Result, StepId};
use dfmcp_intent::{PlanStep, PreparedPlan, plan_archive};

const MAX_STEPS: usize = 16;
const MAX_BYTES: usize = 64 * 1024;
const MAX_PREDICATES: usize = 512;

fn refused(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidPlan, message)
}

/// The original parent is retained byte-for-byte in semantic terms. The index
/// is validated against its unchanged contiguous StepId sequence.
#[derive(Clone, Debug)]
pub(crate) struct SelectedSemanticStep {
    original: PreparedPlan,
    step: StepId,
}
impl SelectedSemanticStep {
    pub(crate) fn new(original: PreparedPlan, step: StepId) -> Result<Self> {
        Self::validate(&original, step)?;
        Ok(Self { original, step })
    }

    /// Revalidate the whole sealed parent before a selected effect boundary.
    /// Archive's bounded visitor runs before semantic hashing or normalization.
    pub(crate) fn validate(original: &PreparedPlan, step: StepId) -> Result<&PlanStep> {
        let limits = plan_archive::ArchiveLimits {
            max_bytes: MAX_BYTES,
            max_steps: MAX_STEPS,
            max_nodes: 4096,
            max_depth: 16,
        };
        let _ = plan_archive::encode(original, limits)?;
        if original.requires_checkpoint {
            return Err(refused(
                "selected semantic families cannot supply a mandatory checkpoint",
            ));
        }
        let mut nodes = original.terminal_condition.complexity();
        for value in &original.steps {
            for predicate in value
                .preconditions
                .iter()
                .chain(&value.postconditions)
                .chain(value.obligation.iter().map(|spec| &spec.terminal))
                .chain(
                    value
                        .obligation
                        .iter()
                        .filter_map(|spec| spec.failure.as_ref()),
                )
            {
                nodes = nodes.checked_add(predicate.complexity()).ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::BudgetExceeded,
                        "selected original predicate bound overflowed",
                    )
                })?;
                if nodes > MAX_PREDICATES {
                    return Err(DfmcpError::new(
                        ErrorCode::BudgetExceeded,
                        "selected original exceeds the complete-parent predicate bound",
                    ));
                }
            }
        }
        original
            .steps
            .get(step.get() as usize)
            .filter(|selected| selected.id == step)
            .ok_or_else(|| refused("selected StepId is absent from the original prepared plan"))
    }

    pub(crate) fn into_parts(self) -> (PreparedPlan, StepId) {
        (self.original, self.step)
    }
}

#[cfg(test)]
mod tests;
