//! Join a selected original order to independently observed existing workers.
use dfmcp_core::ObservationCursor;
use dfmcp_world::{
    CompareOp, EntityKind, EvidenceCoverage, EvidencePolicy, Predicate, PredicateTruth, Value,
    WorldSnapshot,
};

use crate::semantic_work_orders::{OperationsGoalSource, WorkOrderGoalProjection};
use crate::work_order_progress::ProgressSource;

use super::*;

fn stale(message: &str) -> DfmcpError {
    error(ErrorCode::StaleAnchor, message)
}

fn compose(
    original: &PreparedPlan,
    layout: &ProductionLayout,
    step: StepId,
    selected: &SelectedWorkOrderGoalResult,
    workforce: &ExistingLaborProjection,
    workforce_policy: &EvidencePolicy,
    context: &OperationContext,
    parent_cursor: Option<ObservationCursor>,
) -> Result<SelectedParentGoalProgress> {
    let order = selected.inner.projection();
    let selected_anchor = order.anchor();
    if workforce.anchor().fortress_id != selected_anchor.fortress_id
        || workforce.anchor().cursor != selected_anchor.cursor
        || workforce.anchor().tick != selected_anchor.tick
        || !order.snapshot().paused
        || !workforce.snapshot().paused
        || workforce.capture().folder() != order.queue_capture().world_folder()
        || workforce.capture().site() != order.queue_capture().site_id()
        || order.original_anchor() != original.anchor
        || workforce.original_anchor() != original.anchor
        || selected.progress().plan_digest != original.digest
        || layout.existing_labor() != Some(workforce.requirement())
        || layout.order_ordinal(step).is_none()
    {
        return Err(stale(
            "existing workers and original order do not share one paused frontier",
        ));
    }
    let current_inventory = order.evidence(selected.inner.policy().clone())?;
    let inventory_requirements =
        super::parent_goal::inventory_findings(layout.inventory_minima(), &current_inventory)?;
    let current_workers = workforce.evidence(workforce_policy.clone())?;
    let labor = workforce
        .requirement()
        .units()
        .into_iter()
        .map(|unit_id| {
            let compare = |enabled| {
                current_workers.establishes(&Predicate::FieldCompare {
                    entity_id: unit_id,
                    field: format!("labor.{}", workforce.requirement().labor()),
                    op: CompareOp::Eq,
                    value: Value::Bool(enabled),
                })
            };
            let observed_enabled = if compare(true)? {
                Some(true)
            } else if compare(false)? {
                Some(false)
            } else {
                None
            };
            Ok(SelectedParentLaborFinding {
                unit_id,
                labor: workforce.requirement().labor().to_owned(),
                requested_enabled: true,
                observed_enabled,
                current_truth: match observed_enabled {
                    Some(true) => PredicateTruth::True,
                    Some(false) => PredicateTruth::False,
                    None => PredicateTruth::Unknown,
                },
            })
        })
        .collect::<Result<Vec<_>>>()?;
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
        if entity.kind != EntityKind::Unit || graph.entities.insert(*id, entity.clone()).is_some() {
            return Err(custody());
        }
    }
    let publication_cursor = parent_cursor.unwrap_or(selected_anchor.cursor);
    if publication_cursor.epoch != selected_anchor.cursor.epoch
        || publication_cursor.sequence < selected_anchor.cursor.sequence
    {
        return Err(stale(
            "existing-worker parent publication cursor precedes its selected source",
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
    let unit_coverage = if workforce_policy.all_entities == EvidenceCoverage::Observed {
        EvidenceCoverage::Observed
    } else {
        workforce_policy
            .entity_kinds
            .get(&EntityKind::Unit)
            .copied()
            .map_or(EvidenceCoverage::Unknown, |coverage| coverage)
    };
    policy.entity_kinds.insert(EntityKind::Unit, unit_coverage);
    policy
        .sources
        .extend(workforce_policy.sources.iter().cloned());
    policy.paused &= workforce_policy.paused;
    let evidence = dfmcp_world::PredicateEvidence::scoped(&snapshot, policy)?;
    let current_truth = if parent_cursor.is_none() {
        evidence.evaluate(&original.terminal_condition)?
    } else {
        let selected_step = policy::selected(original, step)?;
        if dfmcp_intent::effects::created_entity_id(&selected_step.idempotency_key, 0)
            != order.original_order_entity()
        {
            return Err(custody());
        }
        let mut predicates = selected_step.postconditions.clone();
        predicates.push(workforce.requirement().predicate());
        predicates.extend(layout.inventory_minima().iter().cloned());
        evidence.evaluate(&Predicate::All(predicates))?
    };
    // These findings were qualified before the workforce policy was joined.
    // The complete parent cannot borrow item authority from another domain.
    let current_truth = inventory_requirements
        .iter()
        .fold(current_truth, |truth, finding| {
            match (truth, finding.current_truth) {
                (PredicateTruth::False, _) | (_, PredicateTruth::False) => PredicateTruth::False,
                (PredicateTruth::Unknown, _) | (_, PredicateTruth::Unknown) => {
                    PredicateTruth::Unknown
                }
                _ => PredicateTruth::True,
            }
        });
    let mut identity = if parent_cursor.is_some() {
        b"dfmcp-existing-labor-parent-goal/2\0".to_vec()
    } else {
        b"dfmcp-existing-labor-parent-goal/1\0".to_vec()
    };
    for digest in [
        original.digest,
        order.review_seal(),
        workforce.selection_seal(),
        order.source_digest(),
        workforce.source_digest(),
        snapshot.state_hash,
    ] {
        identity.extend_from_slice(digest.as_bytes());
    }
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
        inventory_requirements,
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
    pub(super) fn poll_existing_labor_parent_goal<H, P, O, PF, OF, A>(
        &mut self,
        monitor: &mut SelectedWorkOrderGoalMonitor<H>,
        original_identity: &LiveRoutingEvidence<'_>,
        cursor: ObservationCursor,
        context: &OperationContext,
        sources: (PF, OF),
        mut order_policy: A,
        parent_cursor: Option<ObservationCursor>,
    ) -> Result<SelectedParentGoalResult>
    where
        H: EffectJournalStorage,
        P: ProgressSource,
        O: OperationsGoalSource,
        PF: FnOnce(&OperationContext) -> Result<P>,
        OF: FnOnce(&OperationContext) -> Result<O>,
        A: FnMut(&WorkOrderGoalProjection, &OperationContext) -> Result<EvidencePolicy>,
    {
        let result = (|| {
            let layout = self.layout.as_ref().ok_or_else(custody)?.clone();
            let requirement = layout.existing_labor().ok_or_else(custody)?;
            let mut call = self.call(context)?;
            call.floor(monitor.high_tick().get());
            let outcome = (|| {
                call.current()?
                    .authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
                if monitor.original_plan() != &self.original
                    || monitor.archive != self.archive
                    || layout.order_ordinal(monitor.step).is_none()
                    || monitor.portfolio != parent_cursor.is_some()
                    || (parent_cursor.is_none() && layout.order_steps().len() != 1)
                {
                    return Err(custody());
                }
                if monitor.is_read_only()
                    || monitor.parent_workforce_fenced
                    || self.existing_labor_fenced
                {
                    return Err(error(
                        ErrorCode::CapabilityDenied,
                        "existing-labor goal observation is read-only or fenced",
                    ));
                }
                if cursor.epoch != self.original.anchor.cursor.epoch
                    || cursor.sequence <= monitor.high_cursor().sequence
                    || cursor.sequence <= context.anchor.cursor.sequence
                {
                    return Err(stale(
                        "existing-labor goal cursor was reused, regressed or crossed a restore",
                    ));
                }
                let (review, dependencies) = self.selected_goal_review(monitor.step, &mut call)?;
                if review.seal() != monitor.review_seal()
                    || dependencies.journal != monitor.dependencies.journal
                    || !dependencies.records.is_empty()
                    || !monitor.dependencies.records.is_empty()
                {
                    return Err(custody());
                }
                let inventory = self.inventory(&mut call)?;
                if !inventory.workforce.records.is_empty() {
                    return Err(custody());
                }
                let binding = inventory.workforce.binding.clone();
                let native_bytes = existing_labor::observation_bytes(inventory.labor_cost())?;
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
                call.take(256 * MIB)?;
                let remaining = call.take(call.bytes.checked_sub(1).ok_or_else(exhausted)?)?;
                let before = existing_labor::observe(
                    &self.original,
                    requirement,
                    original_identity,
                    &mut self.workforce,
                    &mut self.factory,
                    cursor,
                    &call.edge(&first_read, self.high_tick)?,
                    &mut self.existing_labor_floor,
                    &mut self.existing_labor_fenced,
                    &mut self.high_tick,
                )?;
                self.retain_floor(&mut call);
                monitor.parent_high_tick = monitor.parent_high_tick.max(GameTick(self.high_tick));
                let mut policy_context = call.edge(&first_policy, self.high_tick)?;
                policy_context.anchor = before.anchor();
                let granted = self
                    .factory
                    .existing_labor_policy(&before, &policy_context)?;
                before.evidence(granted.clone())?;
                let selected = self.poll_selected_order_goal(
                    monitor,
                    cursor,
                    &call.edge(&remaining, self.high_tick)?,
                    sources.0,
                    sources.1,
                    &mut order_policy,
                )?;
                self.retain_floor(&mut call);
                call.floor(monitor.high_tick().get());
                let manifest = selected.inner.projection().progress_manifest();
                if binding.manifest().df_version != manifest.df_version
                    || binding.manifest().dfhack_version != manifest.dfhack_version
                {
                    return Err(stale(
                        "existing-worker production families have different software identities",
                    ));
                }
                let after = existing_labor::observe(
                    &self.original,
                    requirement,
                    original_identity,
                    &mut self.workforce,
                    &mut self.factory,
                    cursor,
                    &call.edge(&last_read, self.high_tick)?,
                    &mut self.existing_labor_floor,
                    &mut self.existing_labor_fenced,
                    &mut self.high_tick,
                )?;
                self.retain_floor(&mut call);
                if before.capture() != after.capture() {
                    return Err(stale(
                        "existing labor changed across the original order observation",
                    ));
                }
                let mut policy_context = call.edge(&last_policy, self.high_tick)?;
                policy_context.anchor = after.anchor();
                if self
                    .factory
                    .existing_labor_policy(&after, &policy_context)?
                    != granted
                {
                    return Err(error(
                        ErrorCode::CapabilityDenied,
                        "existing labor policy changed during goal acquisition",
                    ));
                }
                let parent = compose(
                    &self.original,
                    &layout,
                    monitor.step,
                    &selected,
                    &after,
                    &granted,
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
                final_context.anchor = selected.progress().anchor;
                if order_policy(selected.inner.projection(), &final_context)?
                    != *selected.inner.policy()
                {
                    return Err(error(
                        ErrorCode::CapabilityDenied,
                        "original order source policy changed before publication",
                    ));
                }
                final_context = call.edge(&final_workforce_policy, self.high_tick)?;
                final_context.anchor = after.anchor();
                if self.factory.existing_labor_policy(&after, &final_context)? != granted {
                    return Err(error(
                        ErrorCode::CapabilityDenied,
                        "existing labor source policy changed before publication",
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
                    selected,
                    parent,
                    workforce: None,
                    existing_labor: Some(after),
                    workforce_policy: granted,
                })
            })();
            self.retain_floor(&mut call);
            outcome
        })();
        monitor.parent_high_tick = monitor.parent_high_tick.max(GameTick(self.high_tick));
        monitor.parent_workforce_fenced |= self.existing_labor_fenced;
        monitor.parent_workforce_floor = self.existing_labor_floor.clone();
        self.high_tick = self.high_tick.max(monitor.high_tick().get());
        if result.is_err() {
            monitor.invalidate_current_sample()?;
        }
        result
    }
}
