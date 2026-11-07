//! Join qualified original leaves under one complete native progress bracket.
//! This accepts adapter-owned sample results, never caller-supplied truth flags.
use super::parent_goal::SelectedParentGoalResult;
use super::*;
use crate::work_order_progress::ProgressObservation;
use crate::workforce_control::WorkforceCapture;
use dfmcp_core::StateAnchor;
use dfmcp_world::{Predicate, PredicateTruth, WorldSnapshot};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionPortfolioGoalProgress {
    pub anchor: StateAnchor,
    pub selected_anchors: Vec<(StepId, StateAnchor)>,
    pub current_truth: PredicateTruth,
    pub original_goal_proven: bool,
    pub source_digest: Digest32,
    pub workforce_source_digest: Digest32,
    pub labor: Vec<SelectedParentLaborFinding>,
}

/// The outer capture brackets every selected durable sample. Each leaf has
/// already independently checked its source grants, exact Created custody and
/// current labor bracket. Equal operations and workforce captures prevent a
/// cross-leaf join from mixing inventory, citizens or native incarnations.
pub fn compose_production_portfolio_goal<O, W>(
    original: &PreparedPlan,
    samples: &[SelectedParentGoalResult],
    before: &ProgressObservation,
    after: &ProgressObservation,
    c: &OperationContext,
    order_policy: O,
    mut workforce_policy: W,
) -> Result<ProductionPortfolioGoalProgress>
where
    O: FnMut(
        StepId,
        &crate::semantic_work_orders::WorkOrderGoalProjection,
        &OperationContext,
    ) -> Result<dfmcp_world::EvidencePolicy>,
    W: FnMut(
        &crate::semantic_workforce::WorkforceGoalProjection,
        &OperationContext,
    ) -> Result<dfmcp_world::EvidencePolicy>,
{
    compose(
        original, samples, before, after, c, order_policy,
        |sample, context| workforce_policy(sample.workforce.as_ref().ok_or_else(custody)?, context),
        false,
    )
}

/// Join every dependency-free original order with the same positively observed
/// existing workers. This accepts no assignment receipt and publishes no whole
/// result until every exact leaf shares one paused source bracket.
pub fn compose_existing_labor_portfolio_goal<O, W>(
    original: &PreparedPlan,
    samples: &[SelectedParentGoalResult],
    before: &ProgressObservation,
    after: &ProgressObservation,
    c: &OperationContext,
    order_policy: O,
    mut workforce_policy: W,
) -> Result<ProductionPortfolioGoalProgress>
where
    O: FnMut(
        StepId,
        &crate::semantic_work_orders::WorkOrderGoalProjection,
        &OperationContext,
    ) -> Result<dfmcp_world::EvidencePolicy>,
    W: FnMut(
        &ExistingLaborProjection,
        &OperationContext,
    ) -> Result<dfmcp_world::EvidencePolicy>,
{
    compose(
        original, samples, before, after, c, order_policy,
        |sample, context| workforce_policy(sample.existing_labor.as_ref().ok_or_else(custody)?, context),
        true,
    )
}

fn workforce(
    sample: &SelectedParentGoalResult,
    existing: bool,
) -> Result<(&WorldSnapshot, &WorkforceCapture, StateAnchor)> {
    match (&sample.workforce, &sample.existing_labor, existing) {
        (Some(projection), None, false) => Ok((
            projection.snapshot(), projection.capture(), projection.original_anchor(),
        )),
        (None, Some(projection), true) => Ok((
            projection.snapshot(), projection.capture(), projection.original_anchor(),
        )),
        _ => Err(custody()),
    }
}

fn compose<O, W>(
    original: &PreparedPlan,
    samples: &[SelectedParentGoalResult],
    before: &ProgressObservation,
    after: &ProgressObservation,
    c: &OperationContext,
    mut order_policy: O,
    mut workforce_policy: W,
    existing: bool,
) -> Result<ProductionPortfolioGoalProgress>
where
    O: FnMut(
        StepId,
        &crate::semantic_work_orders::WorkOrderGoalProjection,
        &OperationContext,
    ) -> Result<dfmcp_world::EvidencePolicy>,
    W: FnMut(
        &SelectedParentGoalResult,
        &OperationContext,
    ) -> Result<dfmcp_world::EvidencePolicy>,
{
    c.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
    let layout = ProductionLayout::from_plan(original)?;
    if samples.is_empty()
        || samples.len() > 4
        || samples.len() != layout.order_steps().len()
        || existing != layout.existing_labor().is_some()
    {
        return Err(custody());
    }
    let first = samples.first().ok_or_else(custody)?;
    let (first_workforce, first_capture, _) = workforce(first, existing)?;
    let first_order = first.selected.inner.projection();
    let mut terminal = if let Some(requirement) = layout.existing_labor() {
        vec![requirement.predicate()]
    } else {
        policy::selected(original, layout.assignment_step().ok_or_else(custody)?)?
            .postconditions.clone()
    };
    let mut ids = Vec::with_capacity(samples.len());
    let mut graph = first_order.snapshot().graph.clone();
    let mut cursor = c.anchor.cursor;
    cursor.sequence = cursor.sequence.checked_add(1).ok_or_else(exhausted)?;
    let mut current_truth = PredicateTruth::True;
    let mut proved = true;
    let mut selected_anchors = Vec::with_capacity(samples.len());
    let mut identity = if existing {
        b"dfmcp-existing-labor-production-portfolio-goal/1\0".to_vec()
    } else {
        b"dfmcp-production-portfolio-goal/1\0".to_vec()
    };
    identity.extend_from_slice(original.digest.as_bytes());
    identity.extend_from_slice(before.witness().as_bytes());
    identity.extend_from_slice(after.witness().as_bytes());
    for (step_id, sample) in layout.order_steps().iter().zip(samples) {
        let step = policy::selected(original, *step_id)?;
        let order = sample.selected.inner.projection();
        let (workforce_snapshot, workforce_capture, workforce_original) = workforce(sample, existing)?;
        let mut authorization = c.clone();
        authorization.budget.max_bytes = c.budget.max_bytes / 16;
        authorization.anchor = order.anchor();
        if order_policy(step.id, order, &authorization)? != *sample.selected.inner.policy() {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "portfolio order source policy changed before whole-plan publication",
            ));
        }
        authorization.anchor = workforce_snapshot.anchor();
        if workforce_policy(sample, &authorization)? != sample.workforce_policy {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "portfolio workforce source policy changed before whole-plan publication",
            ));
        }
        let native = order.progress_capture();
        if step.id != sample.step
            || step.depends_on != layout.assignment_step().into_iter().collect::<Vec<_>>()
            || !matches!(step.action, dfmcp_intent::Action::CreateWorkOrder { .. })
            || sample.progress().plan_digest != original.digest
            || order.original_anchor() != original.anchor
            || workforce_original != original.anchor
            || workforce_capture != first_capture
            || (existing
                && sample.existing_labor.as_ref().map(ExistingLaborProjection::requirement)
                    != layout.existing_labor())
            || sample.parent.labor != first.parent.labor
            || order.operations() != first_order.operations()
            || order.queue_capture() != first_order.queue_capture()
            || order.progress_manifest() != first_order.progress_manifest()
            || order.anchor().tick != first_order.anchor().tick
            || order.anchor().cursor.epoch != original.anchor.cursor.epoch
            || native.ids().len() != 1
            || native.generation() != before.generation()
            || order.progress_before_capture().sequence() <= before.sequence()
            || native.sequence() >= after.sequence()
            || native.tick() != before.tick()
            || native.site() != before.site()
            || native.world_folder() != before.world_folder()
            || native.next_order_id() != before.next_order_id()
            || native.queue_count() != before.queue_count()
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "portfolio leaves do not share the complete original paused source bracket",
            ));
        }
        let row = native.rows().first().ok_or_else(custody)?;
        if before
            .rows()
            .iter()
            .find(|candidate| candidate.native_order_id == row.native_order_id)
            != Some(row)
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "original portfolio order changed across the complete sample",
            ));
        }
        ids.push(row.native_order_id);
        let obligation = step.obligation.as_ref().ok_or_else(custody)?;
        if step.postconditions != [obligation.terminal.clone()] || obligation.failure.is_some() {
            return Err(custody());
        }
        terminal.push(obligation.terminal.clone());
        current_truth = match (current_truth, sample.parent.current_truth) {
            (PredicateTruth::False, _) | (_, PredicateTruth::False) => PredicateTruth::False,
            (PredicateTruth::Unknown, _) | (_, PredicateTruth::Unknown) => PredicateTruth::Unknown,
            _ => PredicateTruth::True,
        };
        proved &= sample.selected_goal_proven();
        cursor.sequence = cursor.sequence.max(order.anchor().cursor.sequence);
        selected_anchors.push((step.id, order.anchor()));
        identity.extend_from_slice(order.source_digest().as_bytes());
        identity.extend_from_slice(sample.parent.source_digest.as_bytes());
        // Common operations entities were compared as complete native captures.
        // Only the distinct receipt-linked semantic order record is added.
        let id = order.original_order_entity();
        if let Some(entity) = order.snapshot().graph.entities.get(&id) {
            if let Some(old) = graph.entities.get(&id) {
                if id != first_order.original_order_entity() || old != entity {
                    return Err(custody());
                }
            } else {
                graph.entities.insert(id, entity.clone());
            }
        }
    }
    ids.sort_unstable();
    if ids.windows(2).any(|pair| pair[0] == pair[1])
        || before.ids() != ids
        || after.ids() != ids
        || before.generation() != after.generation()
        || before.sequence() >= after.sequence()
        || before.tick() != after.tick()
        || before.site() != after.site()
        || before.world_folder() != after.world_folder()
        || !before.paused()
        || !after.paused()
        || before.next_order_id() != after.next_order_id()
        || before.queue_count() != after.queue_count()
        || before.rows() != after.rows()
        || original.terminal_condition != Predicate::All(terminal).normalized()
        || before.fortress_id() != original.anchor.fortress_id
        || c.anchor.fortress_id != original.anchor.fortress_id
        || c.anchor.cursor.epoch != original.anchor.cursor.epoch
        || before.tick() < c.anchor.tick.get()
    {
        return Err(error(
            ErrorCode::StaleAnchor,
            "portfolio whole-plan predicate or complete observation bracket changed",
        ));
    }
    for (id, entity) in &first_workforce.graph.entities {
        if graph.entities.insert(*id, entity.clone()).is_some() {
            return Err(custody());
        }
    }
    if graph.entities.len() > c.budget.max_entities as usize || graph.entities.len() > 4096 + 32 + 4
    {
        return Err(exhausted());
    }
    for entity in graph.entities.values_mut() {
        entity.revision = cursor.sequence;
    }
    let snapshot = WorldSnapshot::new(
        original.anchor.fortress_id,
        GameTick(before.tick()),
        cursor,
        true,
        graph,
    );
    identity.extend_from_slice(snapshot.state_hash.as_bytes());
    Ok(ProductionPortfolioGoalProgress {
        anchor: snapshot.anchor(),
        selected_anchors,
        current_truth,
        original_goal_proven: current_truth == PredicateTruth::True && proved,
        source_digest: Digest32::of_bytes(&identity),
        workforce_source_digest: first.parent.workforce_source_digest,
        labor: first.parent.labor.clone(),
    })
}
