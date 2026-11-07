//! Optional read-only production discovery before an original plan exists.
//! The operations source keeps its own canonical generations and cursor. It is
//! never substituted for the citizen anchor, goal evidence, or native custody.
use super::*;
use dfmcp_adapter::live_operations::{LiveOperationsObservation, LiveOperationsState};
use dfmcp_adapter::operations_inspection::{
    OPERATIONS_INSPECTION_MAX_EDGES, OPERATIONS_INSPECTION_MAX_ENTITIES,
    OPERATIONS_INSPECTION_PROJECTION_BYTES,
};
use dfmcp_adapter::semantic_work_orders::OperationsGoalSource;
use dfmcp_core::{DfmcpError, ErrorCode};

pub(super) const PROJECTION_BYTES: u64 = OPERATIONS_INSPECTION_PROJECTION_BYTES;
pub(super) const CAPTURE_BYTES: u64 = 32 * MIB + PROJECTION_BYTES;

#[derive(Clone, Copy)]
struct Frontier {
    tick: GameTick,
    counters: [u32; 3],
}

/// Withdrawal revokes public availability without forgetting observed native
/// floors or entity generations. A rejected read cannot rewind either one.
#[derive(Default)]
pub(super) struct PlanningInspection {
    state: LiveOperationsState,
    source_digest: Option<Digest32>,
    canonical_anchor: Option<StateAnchor>,
    // This identity survives withdrawal and failures after source publication.
    accepted_parent: Option<(SessionId, StateAnchor)>,
    frontier: Option<Frontier>,
    fenced: bool,
}

impl PlanningInspection {
    pub(super) fn withdraw(&mut self) {
        self.canonical_anchor = None;
    }

    fn check_parent_owner(&self, context: &OperationContext) -> Result<()> {
        if let Some((session, parent)) = self.accepted_parent {
            if context.session_id != session {
                return Err(denied(
                    "planning operations history belongs to another session",
                ));
            }
            if context.anchor.fortress_id != parent.fortress_id
                || context.anchor.cursor.epoch != parent.cursor.epoch
            {
                return Err(DfmcpError::new(
                    ErrorCode::StaleAnchor,
                    "planning operations cannot cross their canonical fortress or epoch",
                ));
            }
        }
        Ok(())
    }

    /// A qualified capture may outlive a failed enclosing Observe. Do not reuse
    /// its provisional parent cursor for a different canonical citizen source.
    pub(super) fn next_cursor(&self, context: &OperationContext) -> Result<ObservationCursor> {
        self.check_parent_owner(context)?;
        let sequence = context
            .anchor
            .cursor
            .sequence
            .max(
                self.accepted_parent
                    .map_or(0, |(_, parent)| parent.cursor.sequence),
            )
            .checked_add(1)
            .ok_or_else(|| exhausted("combined observation cursor exhausted"))?;
        Ok(ObservationCursor {
            epoch: context.anchor.cursor.epoch,
            sequence,
        })
    }

    fn admit_parent(&self, context: &OperationContext) -> Result<()> {
        self.check_parent_owner(context)?;
        if self.accepted_parent.is_some_and(|(_, parent)| {
            context.anchor.cursor.sequence < parent.cursor.sequence
                || context.anchor.tick < parent.tick
                || (context.anchor.cursor.sequence == parent.cursor.sequence
                    && context.anchor != parent)
        }) {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "planning operations cannot replace or rewind their accepted canonical parent",
            ));
        }
        Ok(())
    }

    fn admit_capture(&self) -> Result<()> {
        if self.fenced {
            return Err(denied(
                "planning operations source regressed; retain recovery and reopen the session",
            ));
        }
        Ok(())
    }

    /// Retain the frontier of every valid capture from the independently pinned
    /// source, including one later rejected for a different citizen tick.
    fn note_observation(
        &mut self,
        observation: &LiveOperationsObservation,
        scope: &orders::policy::SourceAcceptance,
    ) -> Result<()> {
        self.withdraw();
        self.admit_capture()?;
        observation.validate()?;
        if observation.jobs.world_folder != scope.world_folder
            || observation.jobs.site_id != scope.site_id as i32
            || observation.jobs.df_version != scope.df_version
            || observation.jobs.dfhack_version != scope.dfhack_version
            || observation.jobs.bridge_generation != scope.operations_generation
        {
            // Source identity is checked independently by the policy issuer.
            // A foreign source must not replace this source's allocator floor.
            return Ok(());
        }
        let next = Frontier {
            tick: observation.jobs.tick(),
            counters: [
                observation.jobs.next_job_id,
                observation.next_building_id,
                observation.next_item_id,
            ],
        };
        if self.frontier.is_some_and(|prior| {
            next.tick < prior.tick
                || next
                    .counters
                    .iter()
                    .zip(prior.counters)
                    .any(|(new, old)| *new < old)
        }) {
            self.fenced = true;
            return Err(denied(
                "planning operations tick or allocator regressed; reopen the session",
            ));
        }
        self.frontier = Some(next);
        Ok(())
    }

    fn publish(
        &mut self,
        observation: LiveOperationsObservation,
        scope: &orders::policy::SourceAcceptance,
        context: &OperationContext,
    ) -> Result<()> {
        self.withdraw();
        self.admit_capture()?;
        scope.planning_operations(&observation, context)?;
        context.budget.validate()?;
        self.admit_parent(context)?;
        // Admit the complete graph before projecting or hashing it. A smaller
        // request cannot silently acquire a truncated census or prove absence.
        let records = observation
            .jobs
            .jobs
            .len()
            .saturating_add(observation.buildings.len())
            .saturating_add(observation.items.len())
            .saturating_add(2);
        if context.budget.max_bytes < PROJECTION_BYTES
            || records > OPERATIONS_INSPECTION_MAX_ENTITIES
            || records > context.budget.max_entities as usize
        {
            return Err(exhausted(
                "complete planning operations exceed the source budget",
            ));
        }
        let edges = observation
            .attachments
            .len()
            .saturating_add(
                observation
                    .jobs
                    .jobs
                    .iter()
                    .filter(|job| job.holder_native_id.is_some())
                    .count(),
            )
            .saturating_add(
                observation
                    .items
                    .iter()
                    .map(|item| {
                        usize::from(item.container_native_id.is_some())
                            + usize::from(item.holder_building_native_id.is_some())
                    })
                    .sum::<usize>(),
            );
        if edges > OPERATIONS_INSPECTION_MAX_EDGES {
            return Err(exhausted(
                "complete planning operations exceed the projection edge bound",
            ));
        }
        self.state.publish(observation)?;
        self.accepted_parent = Some((context.session_id, context.anchor));
        // Descriptor rendering must be constant work. Cache this identity
        // while the complete source projection allowance is still in scope.
        self.source_digest = Some(self.state.source_digest()?);
        self.canonical_anchor = Some(context.anchor);
        Ok(())
    }

    pub(super) fn current(
        &self,
        canonical_anchor: StateAnchor,
        tick_floor: GameTick,
    ) -> Option<&LiveOperationsState> {
        if self.fenced
            || self.canonical_anchor != Some(canonical_anchor)
            || canonical_anchor.tick < tick_floor
        {
            return None;
        }
        let snapshot = self.state.snapshot()?;
        (snapshot.fortress_id == canonical_anchor.fortress_id
            && snapshot.tick == canonical_anchor.tick
            && snapshot.paused)
            .then_some(&self.state)
    }
}

impl Runtime {
    pub(super) fn capture_planning_inspection(&mut self, work: &mut Work) -> Result<()> {
        self.planning_inspection.admit_capture()?;
        inventory_baseline::authorize_observe(&work.current()?, self.observation_tick_floor)?;
        let mut source = self.config.orders.operations(&work.take(16 * MIB)?)?;
        let observation = source.read_operations(&work.take(16 * MIB)?)?;
        // Release the one native connection before local projection and query.
        drop(source);
        self.observation_tick_floor = self.observation_tick_floor.max(observation.jobs.tick());
        self.planning_inspection
            .note_observation(&observation, self.config.orders.acceptance())?;
        inventory_baseline::authorize_observe(&work.current()?, self.observation_tick_floor)?;
        self.config.current()?;
        self.planning_inspection.publish(
            observation,
            self.config.orders.acceptance(),
            &work.take(PROJECTION_BYTES)?,
        )?;
        self.config.current()?;
        work.current()?;
        Ok(())
    }

    pub(super) fn current_planning_inspection(&self) -> Option<&LiveOperationsState> {
        if self.original.is_some()
            || self.mode == Mode::Offline
            || self.fenced
            || self.basis.as_ref()?.projection.snapshot.anchor() != self.anchor
        {
            return None;
        }
        self.planning_inspection
            .current(self.anchor, self.observation_tick_floor)
    }

    pub(super) fn planning_inspection_json(&self) -> Value {
        let Some(source) = self.current_planning_inspection() else {
            return Value::Null;
        };
        let (Some(snapshot), Some(observation), Some(digest)) = (
            source.snapshot(),
            source.observation(),
            self.planning_inspection.source_digest,
        ) else {
            return Value::Null;
        };
        let anchor = anchor_json(snapshot.anchor());
        json!({"scope":"preplan_captured_operations","attribution":"not_order_attributed",
            "observation_phase":"before_plan",
            "order_step_id":null,"anchor":anchor,"parent_anchor":anchor_json(self.anchor),
            "canonical_anchor":anchor_json(self.anchor),"source_digest":digest.to_hex(),
            "source_protocol":"operations/1.3","source_generation":observation.jobs.bridge_generation,
            "restored_from_history":false,"mutation_authority":false,"commit_compatible":false,
            "coverage":{"jobs":"complete","buildings":"complete","items":"complete",
                "job_item_attachments":"complete","native_recipe_feasibility":"unknown"},
            "diagnosis_request":{"schema":"dfmcp.query/1","expected_anchor":anchor,
                "query":{"kind":"production_diagnosis","limit":4}},
            "inventory_request":{"schema":"dfmcp.query/1","expected_anchor":anchor,
                "query":{"kind":"entities","kinds":["item"],
                    "fields":["type_key","stack_size","material_type","material_index"],"limit":16}},
            "inventory_totals_request":inspection::inventory_totals_request(snapshot.anchor()),
            "inventory_totals_semantics":inspection::INVENTORY_TOTALS_SEMANTICS,
            "query_kinds":inspection::QUERY_KINDS,
            "unknown":["native_recipe_requirements","workshop_subtype","path_access",
                "labor_eligibility","production_attribution","future_production_success"],
            "original_goal_evidence":false,"baseline_reused_for_planning":false})
    }
}

#[cfg(test)]
#[path = "planning_inspection_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "preflight_tests.rs"]
mod native_tests;
