//! Fresh physical cleanup of exact durable originals, never recovered dispatch.
//!
//! The coordinator must verify complete durable custody and any current lease
//! before constructing a reference. These references are evidence identities,
//! not grants. The adapter independently checks the bounded original plan,
//! recovery lineage, current authority and exact effect identity before stopping
//! work. Historical action and obligation receipts remain untouched.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use dfmcp_core::{
    Capability, DfmcpError, Digest32, EntityId, ErrorCode, Evidence, EvidenceKind,
    OperationContext, Result, RiskTier, StateAnchor, StepId,
};
use dfmcp_intent::{
    Action, EffectWorkState, PlanStep, PreparedPlan, inspect_effect_work,
    plan_archive::{self, ArchiveLimits},
};
use dfmcp_world::{Fact, FactPresence, FactSource, Value, WorldSnapshot};

use crate::{LabEvent, MemoryAdapter, drain_step_work, evidence};

const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_SOURCE_RECORDS: usize = 100_000;
const MAX_SOURCE_NODES: usize = 65_536;
const MAX_SOURCE_DEPTH: usize = 64;

/// Exact original evidence selected by a coordinator that has independently
/// verified a durable dispatched/final receipt and its complete plan custody.
/// No field here proves dispatch, restores a handle, or grants effect authority.
#[derive(Clone, Copy, Debug)]
pub struct RecoveredWorkRef<'a> {
    pub plan: &'a PreparedPlan,
    pub step_id: StepId,
    pub recorded_anchor: StateAnchor,
    pub recovery_anchor: StateAnchor,
    pub custody_digest: Digest32,
}

/// Fresh current-world evidence of cleanup, separate from the original proof.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveredWorkDrainReceipt {
    pub plan_digest: Digest32,
    pub step_id: StepId,
    pub custody_digest: Digest32,
    pub recorded_anchor: StateAnchor,
    pub recovery_anchor: StateAnchor,
    pub before_anchor: StateAnchor,
    pub observed_anchor: StateAnchor,
    pub before: EffectWorkState,
    pub after: EffectWorkState,
    pub stopped_work: bool,
    pub evidence: Vec<Evidence>,
}

impl MemoryAdapter {
    /// Inspect whether the exact recovered temporal work can be drained under
    /// current authority. This does not advance time, stop work, or retain proof.
    pub fn inspect_recovered_work_drain(
        &self,
        original: &RecoveredWorkRef<'_>,
        context: &OperationContext,
    ) -> Result<EffectWorkState> {
        let mut budget = SourceBudget::new(context)?;
        let step = self.preflight_recovered_work(original, context, &mut budget)?;
        let work = inspect_effect_work(&self.snapshot, &step.action, &step.idempotency_key, true)?;
        require_known(&work)?;
        budget.check()?;
        Ok(work)
    }

    /// Stop only the physical work identified by an exact archived step. The
    /// caller owns complete-plan preflight, lease fencing and durable publication.
    /// On any refusal the adapter, transcript and historical receipts are intact.
    pub fn drain_recovered_work(
        &mut self,
        original: &RecoveredWorkRef<'_>,
        context: &OperationContext,
    ) -> Result<RecoveredWorkDrainReceipt> {
        let mut budget = SourceBudget::new(context)?;
        let step = self.preflight_recovered_work(original, context, &mut budget)?;
        let before_anchor = self.snapshot.anchor();
        // The complete source is bounded before either cloning or recursive
        // canonical hashing. Publication stays on the shadow until all checks
        // and the final physical-quiescence observation have succeeded.
        let mut next = self.snapshot.clone();
        let (before, after, stopped_work) = drain_step_work(&mut next, step, true, context)?;
        budget.check()?;
        let observed_anchor = next.anchor();
        let receipt = RecoveredWorkDrainReceipt {
            plan_digest: original.plan.digest,
            step_id: original.step_id,
            custody_digest: original.custody_digest,
            recorded_anchor: original.recorded_anchor,
            recovery_anchor: original.recovery_anchor,
            before_anchor,
            observed_anchor,
            before,
            after,
            stopped_work,
            evidence: vec![evidence(
                observed_anchor,
                EvidenceKind::Postcondition,
                "exact recovered physical work is quiescent; original proof and dispatch custody retained",
            )],
        };
        self.snapshot = next;
        if stopped_work {
            self.record_event(LabEvent::RecoveredEffectDrained(
                original.plan.digest,
                original.step_id,
                observed_anchor,
            ));
        }
        Ok(receipt)
    }

    fn preflight_recovered_work<'a>(
        &self,
        original: &RecoveredWorkRef<'a>,
        context: &OperationContext,
        budget: &mut SourceBudget,
    ) -> Result<&'a PlanStep> {
        self.check_anchor(context.anchor)?;
        context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        let (recovered_from, recovered_at) = self
            .recovery_origin
            .ok_or_else(|| invalid("adapter has no intact recovered-world custody"))?;
        if original.custody_digest == Digest32::ZERO
            || original.recovery_anchor != recovered_at
            || context.anchor.cursor.epoch != recovered_at.cursor.epoch
            || !precedes(original.plan.anchor, original.recorded_anchor)
            || !precedes(original.recorded_anchor, recovered_from)
            || !precedes(recovered_at, context.anchor)
        {
            return Err(invalid(
                "recovered work reference crosses its exact recovery lineage",
            ));
        }
        // This visitor bounds every original field before the legacy semantic
        // validator can normalize, clone or hash the prepared plan.
        let limits = ArchiveLimits {
            max_bytes: budget.maximum_bytes.min(plan_archive::MAX_ARCHIVE_BYTES),
            ..ArchiveLimits::default()
        };
        let (archive, _) =
            plan_archive::encode_with_check(original.plan, limits, || budget.check())?;
        budget.charge(archive.len(), 0)?;
        let step = original
            .plan
            .steps
            .iter()
            .find(|step| step.id == original.step_id)
            .ok_or_else(|| invalid("recovered work step is absent from its exact original plan"))?;
        if !matches!(
            step.action,
            Action::DesignateDig { .. }
                | Action::HarvestResource { .. }
                | Action::Build { .. }
                | Action::PlaceFurniture { .. }
                | Action::CreateWorkOrder { .. }
        ) {
            return Err(invalid(
                "recovered cleanup requires a supported temporal effect family",
            ));
        }
        self.authorize_step(step, context)?;
        budget.snapshot(&self.snapshot, context)?;
        let work = inspect_effect_work(&self.snapshot, &step.action, &step.idempotency_key, true)?;
        require_known(&work)?;
        budget.check()?;
        Ok(step)
    }
}

fn precedes(before: StateAnchor, after: StateAnchor) -> bool {
    before.fortress_id == after.fortress_id
        && before.state_hash != Digest32::ZERO
        && after.state_hash != Digest32::ZERO
        && before.tick <= after.tick
        && before.cursor <= after.cursor
        && (before.cursor != after.cursor || before == after)
}

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::CancellationIncomplete, message)
}

fn require_known(work: &EffectWorkState) -> Result<()> {
    match work {
        EffectWorkState::Active { .. } | EffectWorkState::Quiescent { entity_id: Some(_) } => {
            Ok(())
        }
        EffectWorkState::Unknown { reason, .. } => Err(invalid(reason)),
        _ => Err(invalid(
            "recovered dispatch has no exact physical work identity",
        )),
    }
}

/// Conservative encoded-size admission. All compatibility values, including
/// values hidden by presence markers, are visited before canonical serialization.
/// The iterative frontier is admitted before allocation; nesting cannot reach
/// the recursive hash/clone helpers until it is within this fixed profile.
struct SourceBudget {
    bytes: usize,
    nodes: usize,
    maximum_bytes: usize,
    started: Instant,
    wall: Duration,
}

impl SourceBudget {
    fn new(context: &OperationContext) -> Result<Self> {
        context.budget.validate()?;
        let maximum_bytes = usize::try_from(context.budget.max_bytes.min(MAX_SOURCE_BYTES))
            .map_err(|_| exhausted())?;
        Ok(Self {
            bytes: 256,
            nodes: 0,
            maximum_bytes,
            started: Instant::now(),
            wall: Duration::from_millis(context.budget.max_wall_millis),
        })
    }

    fn check(&self) -> Result<()> {
        if self.bytes > self.maximum_bytes
            || self.nodes > MAX_SOURCE_NODES
            || self.started.elapsed() >= self.wall
        {
            return Err(exhausted());
        }
        Ok(())
    }

    fn charge(&mut self, bytes: usize, nodes: usize) -> Result<()> {
        self.bytes = self.bytes.saturating_add(bytes);
        self.nodes = self.nodes.saturating_add(nodes);
        self.check()
    }

    fn value(&mut self, root: &Value) -> Result<()> {
        let mut pending = vec![(root, 0usize)];
        while let Some((value, depth)) = pending.pop() {
            if depth > MAX_SOURCE_DEPTH {
                return Err(exhausted());
            }
            self.charge(32, 1)?;
            let children = match value {
                Value::List(values) => values.len(),
                Value::Object(values) => values.len(),
                _ => 0,
            };
            let frontier = pending.len().saturating_add(children);
            if self.nodes.saturating_add(frontier) > MAX_SOURCE_NODES
                || self.bytes.saturating_add(frontier.saturating_mul(32)) > self.maximum_bytes
            {
                return Err(exhausted());
            }
            match value {
                Value::Text(text) => self.charge(text.len(), 0)?,
                Value::Bytes(bytes) => self.charge(bytes.len(), 0)?,
                Value::List(values) => {
                    pending.extend(values.iter().map(|value| (value, depth + 1)))
                }
                Value::Object(values) => {
                    for (key, value) in values {
                        self.charge(key.len().saturating_add(8), 0)?;
                        pending.push((value, depth + 1));
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn facts(&mut self, facts: &BTreeMap<String, Fact>) -> Result<()> {
        for (name, fact) in facts {
            self.charge(256usize.saturating_add(name.len()), 1)?;
            match &fact.source {
                FactSource::DfhackField(text)
                | FactSource::Derived(text)
                | FactSource::AgentAssertion(text) => self.charge(text.len(), 0)?,
                FactSource::Replay => {}
            }
            self.value(&fact.value)?;
            match &fact.presence {
                Some(FactPresence::Known(value)) => self.value(value)?,
                Some(
                    FactPresence::Unknown(text)
                    | FactPresence::Unsupported(text)
                    | FactPresence::Omitted(text)
                    | FactPresence::Redacted(text),
                ) => self.charge(text.len(), 0)?,
                _ => {}
            }
        }
        Ok(())
    }

    fn values(&mut self, values: &BTreeMap<String, Value>) -> Result<()> {
        for (name, value) in values {
            self.charge(32usize.saturating_add(name.len()), 1)?;
            self.value(value)?;
        }
        Ok(())
    }

    fn snapshot(&mut self, snapshot: &WorldSnapshot, context: &OperationContext) -> Result<()> {
        let graph = &snapshot.graph;
        let records = graph
            .entities
            .len()
            .saturating_add(graph.edges.len())
            .saturating_add(graph.chunks.len())
            .saturating_add(graph.events.len());
        if records > (context.budget.max_entities as usize).min(MAX_SOURCE_RECORDS) {
            return Err(exhausted());
        }
        for (id, entity) in &graph.entities {
            if *id == EntityId::NIL || *id != entity.id {
                return Err(invalid(
                    "recovered work source has inconsistent entity identities",
                ));
            }
            self.charge(
                128usize
                    .saturating_add(entity.label.len())
                    .saturating_add(entity.kind.as_str().len()),
                1,
            )?;
            self.facts(&entity.fields)?;
        }
        for (id, edge) in &graph.edges {
            if *id != edge.id {
                return Err(invalid(
                    "recovered work source has inconsistent edge identities",
                ));
            }
            self.charge(128usize.saturating_add(edge.kind.as_str().len()), 1)?;
            self.facts(&edge.fields)?;
        }
        for (coord, chunk) in &graph.chunks {
            if *coord != chunk.coord {
                return Err(invalid(
                    "recovered work source has inconsistent chunk identities",
                ));
            }
            self.charge(
                128usize.saturating_add(chunk.terrain_runs.len().saturating_mul(16)),
                chunk.terrain_runs.len().saturating_add(1),
            )?;
            for values in chunk.sparse_overlays.values() {
                self.charge(32, 1)?;
                self.values(values)?;
            }
        }
        for (id, event) in &graph.events {
            if *id != event.id {
                return Err(invalid(
                    "recovered work source has inconsistent event identities",
                ));
            }
            self.charge(
                128usize
                    .saturating_add(event.summary.len())
                    .saturating_add(event.kind.as_str().len()),
                1,
            )?;
            self.values(&event.fields)?;
        }
        self.check()
    }
}

fn exhausted() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::BudgetExceeded,
        "recovered cleanup exceeds its complete-source byte, record, node, depth or wall-time allowance",
    )
}
