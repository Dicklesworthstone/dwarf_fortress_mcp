//! Immutable operations inspection before any original plan or native effect.
//!
//! The canonical parent and the operations projection have distinct anchors.
//! Sharing a paused tick does not merge their source universes. An enclosing
//! trusted owner must independently check its configured native source pins;
//! this projection issues no source policy, receipt, goal proof or mutation
//! authority. Refresh preserves the operations identity history and refuses
//! resets instead of quietly starting another generation universe.

use std::time::{Duration, Instant};

use dfmcp_core::{
    Capability, DfmcpError, Digest32, ErrorCode, OperationContext, Result, RiskTier, SessionId,
    StateAnchor,
};
use dfmcp_world::WorldSnapshot;

use crate::live_jobs::JobPublication;
use crate::live_operations::{LiveOperationsObservation, LiveOperationsState};

/// Logical admission allowance for bounded validation, hashing, identity-history
/// cloning and complete projection. It is independent of rendered page width.
pub const OPERATIONS_INSPECTION_PROJECTION_BYTES: u64 = 128 * 1024 * 1024;
pub const OPERATIONS_INSPECTION_MAX_ENTITIES: usize = 4096;
pub const OPERATIONS_INSPECTION_MAX_EDGES: usize = 8192;

fn error(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message)
}

fn absent() -> DfmcpError {
    error(
        ErrorCode::InternalInvariantViolation,
        "published operations inspection source is absent",
    )
}

fn budget() -> DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "complete operations inspection exceeds its bounded allowance",
    )
}

fn authorize(parent: StateAnchor, context: &OperationContext) -> Result<()> {
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
    if context.anchor != parent {
        return Err(error(
            ErrorCode::StaleAnchor,
            "operations inspection context differs from its complete canonical parent",
        ));
    }
    if context.budget.max_bytes < OPERATIONS_INSPECTION_PROJECTION_BYTES {
        return Err(budget());
    }
    Ok(())
}

fn within_deadline(until: Instant) -> Result<()> {
    if Instant::now() >= until {
        return Err(budget());
    }
    Ok(())
}

fn source_bounds(operations: &LiveOperationsObservation, context: &OperationContext) -> Result<()> {
    let entities = operations
        .jobs
        .jobs
        .len()
        .saturating_add(operations.buildings.len())
        .saturating_add(operations.items.len())
        .saturating_add(1);
    if entities > OPERATIONS_INSPECTION_MAX_ENTITIES
        || entities > context.budget.max_entities as usize
        || operations.attachments.len() > OPERATIONS_INSPECTION_MAX_EDGES
    {
        return Err(budget());
    }
    let edges = operations
        .attachments
        .len()
        .saturating_add(
            operations
                .jobs
                .jobs
                .iter()
                .filter(|job| job.holder_native_id.is_some())
                .count(),
        )
        .saturating_add(
            operations
                .items
                .iter()
                .map(|item| {
                    usize::from(item.container_native_id.is_some())
                        + usize::from(item.holder_building_native_id.is_some())
                })
                .sum::<usize>(),
        );
    if edges > OPERATIONS_INSPECTION_MAX_EDGES {
        return Err(budget());
    }
    Ok(())
}

/// One complete bounded Operations/1.3 capture with an independently named
/// canonical parent. Construction never needs a plan, an order or a receipt.
#[derive(Clone, Debug)]
pub struct OperationsInspectionProjection {
    session: SessionId,
    parent_anchor: StateAnchor,
    anchor: StateAnchor,
    source_digest: Digest32,
    state: LiveOperationsState,
}

impl OperationsInspectionProjection {
    pub fn new(
        parent_anchor: StateAnchor,
        operations: LiveOperationsObservation,
        context: &OperationContext,
    ) -> Result<Self> {
        Self::publish(None, parent_anchor, operations, context)
    }

    /// Publish a successor without changing this retained source. The caller
    /// must keep the returned identity history even when it withdraws a source
    /// from public inspection; dropping history and calling `new` would lose
    /// evidence of retirement and native allocator progress.
    pub fn refresh(
        &self,
        parent_anchor: StateAnchor,
        operations: LiveOperationsObservation,
        context: &OperationContext,
    ) -> Result<Self> {
        Self::publish(Some(self), parent_anchor, operations, context)
    }

    fn publish(
        prior: Option<&Self>,
        parent_anchor: StateAnchor,
        operations: LiveOperationsObservation,
        context: &OperationContext,
    ) -> Result<Self> {
        authorize(parent_anchor, context)?;
        let until = Instant::now()
            .checked_add(Duration::from_millis(
                context.budget.max_wall_millis.min(60_000),
            ))
            .ok_or_else(budget)?;
        source_bounds(&operations, context)?;
        operations.validate()?;
        if !operations.jobs.paused
            || operations.jobs.tick() != parent_anchor.tick
            || operations.jobs.fortress_id()? != parent_anchor.fortress_id
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "operations inspection requires the same paused canonical fortress tick",
            ));
        }
        if let Some(prior) = prior {
            if context.session_id != prior.session {
                return Err(error(
                    ErrorCode::CapabilityDenied,
                    "operations inspection history belongs to another session",
                ));
            }
            let previous_parent = prior.parent_anchor;
            if parent_anchor.fortress_id != previous_parent.fortress_id
                || parent_anchor.cursor.epoch != previous_parent.cursor.epoch
                || parent_anchor.cursor.sequence < previous_parent.cursor.sequence
                || parent_anchor.tick < previous_parent.tick
                || (parent_anchor.cursor.sequence == previous_parent.cursor.sequence
                    && parent_anchor != previous_parent)
            {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "operations inspection cannot replace or rewind its canonical parent lineage",
                ));
            }
            let previous = prior.operations()?;
            if operations.jobs.world_folder != previous.jobs.world_folder
                || operations.jobs.site_id != previous.jobs.site_id
                || operations.jobs.df_version != previous.jobs.df_version
                || operations.jobs.dfhack_version != previous.jobs.dfhack_version
                || operations.jobs.bridge_generation != previous.jobs.bridge_generation
                || operations.jobs.next_job_id < previous.jobs.next_job_id
                || operations.next_building_id < previous.next_building_id
                || operations.next_item_id < previous.next_item_id
            {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "operations inspection source identity or allocator frontier changed",
                ));
            }
            if prior.snapshot()?.graph.entities.len() > context.budget.max_entities as usize {
                return Err(budget());
            }
        }
        within_deadline(until)?;
        // Cloning the complete publisher retains observed entity retirement and
        // generations. Publishing into a fresh state on every call would allow
        // an old entity handle to name a later incarnation of the same ID.
        let mut state =
            prior.map_or_else(LiveOperationsState::default, |value| value.state.clone());
        if state.publish(operations)? == JobPublication::Reset {
            return Err(error(
                ErrorCode::StaleAnchor,
                "operations inspection refuses a source reset",
            ));
        }
        let snapshot = state.snapshot().ok_or_else(absent)?;
        let anchor = snapshot.anchor();
        let source_digest = state.source_digest()?;
        within_deadline(until)?;
        authorize(parent_anchor, context)?;
        Ok(Self {
            session: context.session_id,
            parent_anchor,
            anchor,
            source_digest,
            state,
        })
    }

    pub const fn parent_anchor(&self) -> StateAnchor {
        self.parent_anchor
    }

    /// Source-local operations identity; never a replacement citizen anchor.
    pub const fn anchor(&self) -> StateAnchor {
        self.anchor
    }

    pub const fn source_digest(&self) -> Digest32 {
        self.source_digest
    }

    pub fn snapshot(&self) -> Result<&WorldSnapshot> {
        self.state.snapshot().ok_or_else(absent)
    }

    pub fn operations(&self) -> Result<&LiveOperationsObservation> {
        self.state.observation().ok_or_else(absent)
    }
}

#[cfg(test)]
#[path = "operations_inspection_tests.rs"]
mod tests;
