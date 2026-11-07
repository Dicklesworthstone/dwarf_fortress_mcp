//! Fresh complete-parent truth for the closed original labor/order pair.
//!
//! Selected inventory history keeps its original format and temporal semantics.
//! A separate process-local result joins that exact published sample to current
//! labor captured on both sides of it. Native effect receipts supply custody and
//! identity only; they never supply any field in this current projection.

use dfmcp_core::{EntityId, ObservationCursor, StateAnchor};
use dfmcp_intent::Action;
use dfmcp_world::{
    CompareOp, EntityKind, EvidenceCoverage, EvidencePolicy, Predicate, PredicateEvidence,
    PredicateTruth, Value, WorldSnapshot,
};

use crate::semantic_work_orders::{
    OperationsGoalSource, WorkOrderGoalProgress, WorkOrderGoalProjection,
};
use crate::semantic_workforce::WorkforceGoalProjection;
use crate::work_order_progress::ProgressSource;
use crate::workforce_control::WorkforceCapture;

use super::*;

/// One requested citizen/labor comparison at the parent's complete anchor.
/// The optional observed value is exposed only through separately qualified
/// workforce evidence. A retained native receipt cannot populate this finding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedParentLaborFinding {
    pub unit_id: EntityId,
    pub labor: String,
    pub requested_enabled: bool,
    pub observed_enabled: Option<bool>,
    pub current_truth: PredicateTruth,
}

/// Current evidence, not a durable achievement record or an authority token.
/// Reopening never reconstructs this value from selected-inventory history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedParentGoalProgress {
    pub anchor: StateAnchor,
    pub selected_anchor: StateAnchor,
    pub current_truth: PredicateTruth,
    pub original_goal_proven: bool,
    pub source_digest: Digest32,
    pub workforce_source_digest: Digest32,
    /// The original selection only, in numeric canonical-ID order, at most 32.
    pub labor: Vec<SelectedParentLaborFinding>,
}

#[derive(Clone, Debug)]
pub struct SelectedParentGoalResult {
    pub(super) step: StepId,
    pub(super) selected: SelectedWorkOrderGoalResult,
    pub(super) parent: SelectedParentGoalProgress,
    pub(super) workforce: Option<WorkforceGoalProjection>,
    pub(super) existing_labor: Option<ExistingLaborProjection>,
    pub(super) workforce_policy: EvidencePolicy,
}
impl SelectedParentGoalResult {
    pub const fn step_id(&self) -> StepId {
        self.step
    }
    pub fn progress(&self) -> &WorkOrderGoalProgress {
        self.selected.progress()
    }
    pub fn parent_progress(&self) -> &SelectedParentGoalProgress {
        &self.parent
    }
    /// Borrow the exact selected order's immutable operations/progress sample.
    /// Its anchor and source digest remain distinct from the enclosing parent
    /// projection. Read consumers must recheck current authority; possession of
    /// this sample proves neither current freshness nor order-attributed output.
    pub fn operations_projection(&self) -> &WorkOrderGoalProjection {
        self.selected.inner.projection()
    }
    /// Present only for the read-only existing-worker path; this source is not
    /// backed by an assignment action or a historical workforce receipt.
    pub fn existing_labor_projection(&self) -> Option<&ExistingLaborProjection> {
        self.existing_labor.as_ref()
    }
    pub fn selected_goal_proven(&self) -> bool {
        self.selected.selected_goal_proven()
    }
    pub fn original_goal_proven(&self) -> bool {
        self.parent.original_goal_proven
    }
}

fn stale(message: &str) -> DfmcpError {
    error(ErrorCode::StaleAnchor, message)
}

pub(super) fn labor_findings(
    action: &Action,
    workforce: &PredicateEvidence<'_>,
) -> Result<Vec<SelectedParentLaborFinding>> {
    let Action::SetLabor {
        units,
        labor,
        enabled,
    } = action
    else {
        return Err(error(
            ErrorCode::AdapterRejected,
            "parent labor findings require the original selected labor action",
        ));
    };
    if units.is_empty()
        || units.len() > 32
        || units.contains(&EntityId::NIL)
        || labor.is_empty()
        || labor.len() > 64
        || labor.contains('\0')
    {
        return Err(error(
            ErrorCode::AdapterRejected,
            "parent labor findings exceed the original selection bounds",
        ));
    }
    let mut units = units.clone();
    units.sort_unstable();
    if units.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(error(
            ErrorCode::AdapterRejected,
            "parent labor findings require distinct canonical citizens",
        ));
    }
    let field = format!("labor.{labor}");
    units
        .into_iter()
        .map(|unit_id| {
            let observed = |value| {
                workforce.establishes(&Predicate::FieldCompare {
                    entity_id: unit_id,
                    field: field.clone(),
                    op: CompareOp::Eq,
                    value: Value::Bool(value),
                })
            };
            // A false comparison alone does not establish the opposite bool:
            // absence, incompatible values and withheld grants stay unknown.
            let observed_enabled = if observed(true)? {
                Some(true)
            } else if observed(false)? {
                Some(false)
            } else {
                None
            };
            let current_truth = match observed_enabled {
                Some(value) if value == *enabled => PredicateTruth::True,
                Some(_) => PredicateTruth::False,
                None => PredicateTruth::Unknown,
            };
            Ok(SelectedParentLaborFinding {
                unit_id,
                labor: labor.clone(),
                requested_enabled: *enabled,
                observed_enabled,
                current_truth,
            })
        })
        .collect()
}

fn retain_workforce_floor<H: EffectJournalStorage>(
    monitor: &mut SelectedWorkOrderGoalMonitor<H>,
    capture: &WorkforceCapture,
) -> Result<()> {
    // A refused identity or post-read custody check must still consume time.
    // This floor grants no source eligibility and cannot publish any fact.
    monitor.parent_high_tick = monitor.parent_high_tick.max(GameTick(capture.tick()));
    if monitor.parent_workforce_floor.as_ref().is_some_and(|old| {
        capture.generation() != old.generation()
            || capture.sequence() < old.sequence()
            || capture.tick() < old.tick()
            || capture.folder() != old.folder()
            || capture.site() != old.site()
            || capture.labor_keys() != old.labor_keys()
            || capture.citizens().len() != old.citizens().len()
            || capture
                .citizens()
                .iter()
                .zip(old.citizens())
                .any(|(now, old)| {
                    now.id() != old.id() || now.historical_id() != old.historical_id()
                })
    }) {
        monitor.parent_workforce_fenced = true;
        return Err(stale(
            "current parent workforce source regressed or changed incarnation",
        ));
    }
    // A later refusal must not allow an older source fence in this process.
    monitor.parent_workforce_floor = Some(capture.clone());
    Ok(())
}

fn compose(
    original: &PreparedPlan,
    layout: &ProductionLayout,
    selected: &SelectedWorkOrderGoalResult,
    workforce: &WorkforceGoalProjection,
    workforce_policy: &EvidencePolicy,
    context: &OperationContext,
    parent_cursor: Option<ObservationCursor>,
) -> Result<SelectedParentGoalProgress> {
    let order = selected.inner.projection();
    let selected_anchor = order.anchor();
    let workforce_anchor = workforce.anchor();
    if workforce_anchor.fortress_id != selected_anchor.fortress_id
        || workforce_anchor.cursor != selected_anchor.cursor
        || workforce_anchor.tick != selected_anchor.tick
        || !order.snapshot().paused
        || !workforce.snapshot().paused
        || workforce.capture().folder() != order.queue_capture().world_folder()
        || workforce.capture().site() != order.queue_capture().site_id()
        || order.original_anchor() != original.anchor
        || workforce.original_anchor() != original.anchor
        || selected.progress().plan_digest != original.digest
    {
        return Err(stale(
            "parent labor and inventory do not share one original paused frontier",
        ));
    }
    // Each child independently enforces its source/domain ceiling before any
    // grants are combined. Unit observation never expands inventory coverage.
    order.evidence(selected.inner.policy().clone())?;
    let workforce_evidence = workforce.evidence(workforce_policy.clone())?;
    let assignment = policy::selected(original, layout.assignment_step().ok_or_else(custody)?)?;
    let labor = labor_findings(&assignment.action, &workforce_evidence)?;
    let total = order
        .snapshot()
        .graph
        .entities
        .len()
        .checked_add(workforce.snapshot().graph.entities.len())
        .ok_or_else(exhausted)?;
    if total > context.budget.max_entities as usize || total > 4096 + 32 {
        return Err(exhausted());
    }
    let mut graph = order.snapshot().graph.clone();
    for (id, entity) in &workforce.snapshot().graph.entities {
        if entity.kind != EntityKind::Unit || graph.entities.contains_key(id) {
            return Err(error(
                ErrorCode::Conflict,
                "current parent projections have overlapping canonical identities",
            ));
        }
        graph.entities.insert(*id, entity.clone());
    }
    let publication_cursor = parent_cursor.unwrap_or(selected_anchor.cursor);
    if publication_cursor.epoch != selected_anchor.cursor.epoch
        || publication_cursor.sequence < selected_anchor.cursor.sequence
    {
        return Err(stale(
            "portfolio parent publication cursor precedes its selected source",
        ));
    }
    if parent_cursor.is_some() {
        for entity in graph.entities.values_mut() {
            entity.revision = publication_cursor.sequence;
        }
    }
    let snapshot = WorldSnapshot::new(
        selected_anchor.fortress_id,
        selected_anchor.tick,
        publication_cursor,
        true,
        graph,
    );
    let mut policy = selected.inner.policy().clone();
    policy.anchor = Some(snapshot.anchor());
    // The selected-workforce policy may use all_entities=Observed for its own
    // small snapshot. Transfer that permission only to the Unit kind here.
    let units = if workforce_policy.all_entities == EvidenceCoverage::Observed {
        EvidenceCoverage::Observed
    } else {
        workforce_policy
            .entity_kinds
            .get(&EntityKind::Unit)
            .map_or(EvidenceCoverage::Unknown, |coverage| *coverage)
    };
    policy.entity_kinds.insert(EntityKind::Unit, units);
    policy
        .sources
        .extend(workforce_policy.sources.iter().cloned());
    policy.paused &= workforce_policy.paused;
    let evidence = PredicateEvidence::scoped(&snapshot, policy)?;
    let current_truth = if layout.order_steps().len() == 1 {
        evidence.evaluate(&original.terminal_condition)?
    } else {
        let order_step = original
            .steps
            .iter()
            .find(|step| {
                layout.order_ordinal(step.id).is_some()
                    && dfmcp_intent::effects::created_entity_id(&step.idempotency_key, 0)
                        == order.original_order_entity()
            })
            .ok_or_else(custody)?;
        let mut predicates = assignment.postconditions.clone();
        predicates.extend(order_step.postconditions.clone());
        evidence.evaluate(&Predicate::All(predicates))?
    };
    let mut identity = if parent_cursor.is_some() {
        b"dfmcp-selected-parent-goal/2\0".to_vec()
    } else {
        b"dfmcp-selected-parent-goal/1\0".to_vec()
    };
    for digest in [
        original.digest,
        order.review_seal(),
        workforce.review_seal(),
        order.source_digest(),
        workforce.source_digest(),
        snapshot.state_hash,
    ] {
        identity.extend_from_slice(digest.as_bytes());
    }
    // Findings are a bounded projection of the already bound original action
    // and workforce snapshot under its independent policy. They neither change
    // canonical state nor add another source or durable publication identity.
    Ok(SelectedParentGoalProgress {
        anchor: snapshot.anchor(),
        selected_anchor,
        current_truth,
        original_goal_proven: parent_cursor.is_none()
            && layout.order_steps().len() == 1
            && current_truth == PredicateTruth::True
            && selected.selected_goal_proven(),
        source_digest: Digest32::of_bytes(&identity),
        workforce_source_digest: workforce.source_digest(),
        labor,
    })
}

impl<WJ, WA, OJ, OA, ON, E, F> Executor<WJ, WA, OJ, OA, ON, E, F>
where
    WJ: EffectJournalStorage,
    WA: EffectJournalStorage,
    OJ: EffectJournalStorage,
    OA: EffectJournalStorage,
    ON: WorkOrderSource,
    E: WorkforceEvidenceOwner + WorkOrderEvidenceOwner,
    F: WorkforceFactory,
{
    /// Acquire one coherent current-parent sample without a new action, clock
    /// effect, deadline, or temporal obligation. The labor predecessor has no
    /// temporal requirement; the selected order retains its exact original one.
    /// Sources and independently issued policies are trusted embedding inputs.
    pub fn poll_selected_parent_goal<H, P, O, PF, OF, A, W>(
        &mut self,
        monitor: &mut SelectedWorkOrderGoalMonitor<H>,
        original_identity: &LiveRoutingEvidence<'_>,
        cursor: ObservationCursor,
        context: &OperationContext,
        sources: (PF, OF),
        policies: (A, W),
    ) -> Result<SelectedParentGoalResult>
    where
        H: EffectJournalStorage,
        P: ProgressSource,
        O: OperationsGoalSource,
        PF: FnOnce(&OperationContext) -> Result<P>,
        OF: FnOnce(&OperationContext) -> Result<O>,
        A: FnMut(&WorkOrderGoalProjection, &OperationContext) -> Result<EvidencePolicy>,
        W: FnMut(&WorkforceGoalProjection, &OperationContext) -> Result<EvidencePolicy>,
    {
        if self
            .layout
            .as_ref()
            .and_then(ProductionLayout::existing_labor)
            .is_some()
        {
            return self.poll_existing_labor_parent_goal(
                monitor,
                original_identity,
                cursor,
                context,
                sources,
                policies.0,
                None,
            );
        }
        self.poll_selected_parent_goal_inner(
            monitor,
            original_identity,
            cursor,
            context,
            sources,
            policies,
            None,
        )
    }

    /// Portfolio leaves own separate, strictly sequential source cursors in
    /// their exact selected review/history. The parent publication cursor is a
    /// different monotone namespace; interleaving another original leaf cannot
    /// manufacture a missing sample in this leaf's retained history.
    pub fn poll_selected_portfolio_goal<H, P, O, PF, OF, A, W>(
        &mut self,
        monitor: &mut SelectedWorkOrderGoalMonitor<H>,
        original_identity: &LiveRoutingEvidence<'_>,
        cursor: ObservationCursor,
        context: &OperationContext,
        sources: (PF, OF),
        policies: (A, W),
        parent_cursor: ObservationCursor,
    ) -> Result<SelectedParentGoalResult>
    where
        H: EffectJournalStorage,
        P: ProgressSource,
        O: OperationsGoalSource,
        PF: FnOnce(&OperationContext) -> Result<P>,
        OF: FnOnce(&OperationContext) -> Result<O>,
        A: FnMut(&WorkOrderGoalProjection, &OperationContext) -> Result<EvidencePolicy>,
        W: FnMut(&WorkforceGoalProjection, &OperationContext) -> Result<EvidencePolicy>,
    {
        if self
            .layout
            .as_ref()
            .and_then(ProductionLayout::existing_labor)
            .is_some()
        {
            return self.poll_existing_labor_parent_goal(
                monitor,
                original_identity,
                cursor,
                context,
                sources,
                policies.0,
                Some(parent_cursor),
            );
        }
        self.poll_selected_parent_goal_inner(
            monitor,
            original_identity,
            cursor,
            context,
            sources,
            policies,
            Some(parent_cursor),
        )
    }

    fn poll_selected_parent_goal_inner<H, P, O, PF, OF, A, W>(
        &mut self,
        monitor: &mut SelectedWorkOrderGoalMonitor<H>,
        original_identity: &LiveRoutingEvidence<'_>,
        cursor: ObservationCursor,
        context: &OperationContext,
        sources: (PF, OF),
        policies: (A, W),
        parent_cursor: Option<ObservationCursor>,
    ) -> Result<SelectedParentGoalResult>
    where
        H: EffectJournalStorage,
        P: ProgressSource,
        O: OperationsGoalSource,
        PF: FnOnce(&OperationContext) -> Result<P>,
        OF: FnOnce(&OperationContext) -> Result<O>,
        A: FnMut(&WorkOrderGoalProjection, &OperationContext) -> Result<EvidencePolicy>,
        W: FnMut(&WorkforceGoalProjection, &OperationContext) -> Result<EvidencePolicy>,
    {
        let (progress_factory, operations_factory) = sources;
        let (mut order_policy, mut workforce_policy) = policies;
        let result = (|| {
            let mut call = self.call(context)?;
            call.floor(monitor.high_tick().get());
            let outcome = (|| {
                call.current()?
                    .authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
                if monitor.original_plan() != &self.original || monitor.archive != self.archive {
                    return Err(custody());
                }
                if monitor.portfolio != parent_cursor.is_some() {
                    return Err(custody());
                }
                if monitor.is_read_only() {
                    return Err(error(
                        ErrorCode::CapabilityDenied,
                        "read-only selected history cannot publish a parent sample",
                    ));
                }
                if monitor.parent_workforce_fenced {
                    return Err(stale("current parent workforce lineage is fenced"));
                }
                if cursor.epoch != self.original.anchor.cursor.epoch
                    || cursor.sequence <= monitor.high_cursor().sequence
                    || cursor.sequence <= context.anchor.cursor.sequence
                {
                    return Err(stale(
                        "parent observation cursor was reused, regressed or crossed a restore",
                    ));
                }
                let layout = self.layout.as_ref().ok_or_else(custody)?;
                if layout.order_ordinal(monitor.step).is_none() {
                    return Err(error(
                        ErrorCode::AdapterRejected,
                        "current labor/order proof requires original labor and one to four dependent order leaves",
                    ));
                }
                let labor = policy::selected(
                    &self.original,
                    layout.assignment_step().ok_or_else(custody)?,
                )?
                .clone();
                let (review, dependencies) = self.selected_goal_review(monitor.step, &mut call)?;
                if review.seal() != monitor.review_seal()
                    || dependencies.journal != monitor.dependencies.journal
                    || dependencies.records != monitor.dependencies.records
                {
                    return Err(custody());
                }
                let inventory = self.inventory(&mut call)?;
                let binding = inventory.workforce.binding.clone();
                let native_bytes = workforce_operation_bytes(inventory.labor_cost())?
                    .checked_add(16 * MIB)
                    .ok_or_else(exhausted)?;
                let reattach = call.take(labor_read_bytes(inventory.labor_cost(), 2)?)?;
                let first_read = call.take(native_bytes)?;
                let last_read = call.take(native_bytes)?;
                let first_policy = call.take(MIB)?;
                let last_policy = call.take(MIB)?;
                let final_order_policy = call.take(MIB)?;
                let final_workforce_policy = call.take(MIB)?;
                let publication = call.take(WORKFORCE_VIEW_BYTES + ORDER_VIEW_BYTES + 2 * MIB)?;
                let dependency_check = call.take(dependencies.check_bytes)?;
                let final_publication =
                    call.take(WORKFORCE_VIEW_BYTES + ORDER_VIEW_BYTES + 2 * MIB)?;
                let final_dependency_check = call.take(dependencies.check_bytes)?;
                call.take(256 * MIB)?; // Bounded child/composite encodings and retained copies.
                let remaining = call.take(call.bytes.checked_sub(1).ok_or_else(exhausted)?)?;
                let labor_review = self.workforce.reattach_selected(
                    self.selection(labor.id)?,
                    &call.edge(&reattach, self.high_tick)?,
                )?;
                self.retain_floor(&mut call);
                if monitor.parent_workforce_floor.is_none() {
                    monitor.parent_workforce_floor =
                        Some(labor_review.native_plan().before().clone());
                }
                let factory = &mut self.factory;
                let before = self.workforce.observe_selected_goal_projection(
                    labor_review.seal(),
                    original_identity,
                    cursor,
                    &call.edge(&first_read, self.high_tick)?,
                    (
                        |b, c| factory.connect(b, c),
                        |capture: &WorkforceCapture| retain_workforce_floor(monitor, capture),
                    ),
                );
                self.retain_floor(&mut call);
                let before = before?;
                let mut policy_context = call.edge(&first_policy, self.high_tick)?;
                policy_context.anchor = before.anchor();
                let granted_workforce = workforce_policy(&before, &policy_context)?;
                before.evidence(granted_workforce.clone())?;
                let sampled = self.poll_selected_order_goal(
                    monitor,
                    cursor,
                    &call.edge(&remaining, self.high_tick)?,
                    progress_factory,
                    operations_factory,
                    &mut order_policy,
                );
                self.retain_floor(&mut call);
                call.floor(monitor.high_tick().get());
                let sampled = sampled?;
                let manifest = sampled.inner.projection().progress_manifest();
                if binding.manifest().df_version != manifest.df_version
                    || binding.manifest().dfhack_version != manifest.dfhack_version
                {
                    return Err(stale(
                        "current parent families have different software identities",
                    ));
                }
                let factory = &mut self.factory;
                let after = self.workforce.observe_selected_goal_projection(
                    labor_review.seal(),
                    original_identity,
                    cursor,
                    &call.edge(&last_read, self.high_tick)?,
                    (
                        |b, c| factory.connect(b, c),
                        |capture: &WorkforceCapture| retain_workforce_floor(monitor, capture),
                    ),
                );
                self.retain_floor(&mut call);
                let after = after?;
                // ObserveWorkforce does not advance this native intervention
                // fence. Equal bytes also rule out selected masks/detail drift
                // while the game remains paused at the same tick.
                if before.capture() != after.capture() {
                    return Err(stale(
                        "selected labor changed across the inventory observation",
                    ));
                }
                let mut policy_context = call.edge(&last_policy, self.high_tick)?;
                policy_context.anchor = after.anchor();
                if workforce_policy(&after, &policy_context)? != granted_workforce {
                    return Err(error(
                        ErrorCode::CapabilityDenied,
                        "parent workforce policy changed during acquisition",
                    ));
                }
                let parent = compose(
                    &self.original,
                    self.layout.as_ref().ok_or_else(custody)?,
                    &sampled,
                    &after,
                    &granted_workforce,
                    context,
                    parent_cursor,
                )?;
                self.final_check(&publication, &mut call)?;
                dependencies.verify(
                    &mut self.workforce,
                    &call.edge(&dependency_check, self.high_tick)?,
                )?;
                self.retain_floor(&mut call);
                let mut final_context = call.edge(&final_order_policy, self.high_tick)?;
                final_context.anchor = sampled.progress().anchor;
                if order_policy(sampled.inner.projection(), &final_context)?
                    != *sampled.inner.policy()
                {
                    return Err(error(
                        ErrorCode::CapabilityDenied,
                        "parent inventory policy changed before publication",
                    ));
                }
                final_context = call.edge(&final_workforce_policy, self.high_tick)?;
                final_context.anchor = after.anchor();
                if workforce_policy(&after, &final_context)? != granted_workforce {
                    return Err(error(
                        ErrorCode::CapabilityDenied,
                        "parent workforce policy changed before publication",
                    ));
                }
                self.final_check(&final_publication, &mut call)?;
                dependencies.verify(
                    &mut self.workforce,
                    &call.edge(&final_dependency_check, self.high_tick)?,
                )?;
                self.retain_floor(&mut call);
                call.current()?
                    .authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
                Ok(SelectedParentGoalResult {
                    step: monitor.step,
                    selected: sampled,
                    parent,
                    workforce: Some(after),
                    existing_labor: None,
                    workforce_policy: granted_workforce,
                })
            })();
            self.retain_floor(&mut call);
            self.high_tick = self.high_tick.max(monitor.high_tick().get());
            outcome
        })();
        if result.is_err() {
            monitor.invalidate_current_sample()?;
        }
        result
    }
}
