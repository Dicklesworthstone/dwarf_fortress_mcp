//! Original-order observations from existing complete operations and selected
//! progress captures. Normalization supplies no source or compatibility authority.

use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{Digest32, EntityId, ErrorCode, ObservationCursor, Result, StateAnchor};
use dfmcp_intent::effects::created_entity_id;
use dfmcp_world::{
    CompareOp, EntityKind, EntityRecord, EvidenceCoverage, EvidencePolicy, EvidenceSource, Fact,
    FactSource, Predicate, PredicateEvidence, PredicateTruth, Value, WorldSnapshot,
};

use crate::live_operations::{LiveOperationsObservation, LiveOperationsState};
use crate::work_order_progress::{ProgressManifest, ProgressObservation};
use crate::work_orders::WorkOrderObservation;

use super::{NativeOrderCreation, SemanticWorkOrderReview, error};

pub const WORK_ORDER_GOAL_PROJECTION_SCHEMA: &str = "dfmcp.work_order_goal_projection/1";
pub const ORDER_ID_SOURCE: &str = "work-order-progress/1.12.manager_order.id";
pub const ORDER_COUNTER_SOURCE: &str = "work-order-progress/1.12.manager_order.counters";
pub const ORDER_STATUS_SOURCE: &str = "work-order-progress/1.12.manager_order.status";
pub const ORDER_TYPE_SOURCE: &str = "work-order-progress/1.12.manager_order.type";
pub const MAX_GOAL_ENTITIES: usize = 4096;
pub const MAX_GOAL_EDGES: usize = 8192;

/// Native counters alone cannot establish that all workshop work has stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkOrderPhysicalState {
    Outstanding {
        native_order_id: u32,
        remaining: u32,
    },
    /// A finite, exact original order reports zero remaining, while the same
    /// paused frontier has an independently qualified complete empty job roster.
    Quiescent {
        native_order_id: u32,
    },
    Unknown {
        reason: &'static str,
    },
}
impl WorkOrderPhysicalState {
    pub const fn is_quiescent(&self) -> bool {
        matches!(self, Self::Quiescent { .. })
    }
}

/// Immutable source material for one foreground proof sample. All three native
/// generations remain distinct. The supplied canonical cursor is allocated by
/// the observing owner, never copied from a native intervention/capture counter.
#[derive(Clone, Debug)]
pub struct WorkOrderGoalProjection {
    snapshot: WorldSnapshot,
    operations: LiveOperationsObservation,
    progress_before: ProgressObservation,
    progress: ProgressObservation,
    progress_manifest: ProgressManifest,
    queue: WorkOrderObservation,
    review_seal: Digest32,
    original_anchor: StateAnchor,
    source_digest: Digest32,
    order_entity: EntityId,
    linked: bool,
}
impl WorkOrderGoalProjection {
    pub(super) fn new(
        review: &SemanticWorkOrderReview,
        creation: &NativeOrderCreation,
        operations: &LiveOperationsState,
        progress_captures: (ProgressObservation, ProgressObservation, ProgressManifest),
        queue: WorkOrderObservation,
        cursor: ObservationCursor,
    ) -> Result<Self> {
        let (progress_before, progress, progress_manifest) = progress_captures;
        let raw = operations.observation().ok_or_else(|| {
            error(
                ErrorCode::AdapterRejected,
                "original-goal operations capture is absent",
            )
        })?;
        let base = operations.snapshot().ok_or_else(|| {
            error(
                ErrorCode::AdapterRejected,
                "original-goal operations projection is absent",
            )
        })?;
        if cursor.epoch != review.original.anchor.cursor.epoch
            || cursor.sequence <= review.original.anchor.cursor.sequence
            || !raw.jobs.paused
            || !progress.paused()
            || !queue.paused()
            || raw.jobs.tick().get() != progress.tick()
            || queue.tick() != progress.tick()
            || raw.jobs.fortress_id()? != review.original.anchor.fortress_id
            || progress.fortress_id() != review.original.anchor.fortress_id
            || queue.fortress_id() != review.original.anchor.fortress_id
            || raw.jobs.world_folder != queue.world_folder()
            || raw.jobs.site_id as u32 != queue.site_id()
            || progress.world_folder() != queue.world_folder()
            || progress.site() != queue.site_id()
            || raw.jobs.df_version != progress_manifest.df_version
            || raw.jobs.dfhack_version != progress_manifest.dfhack_version
            || progress.generation() != progress_manifest.generation
            || progress.next_order_id() != queue.next_order_id()
            || progress.queue_count() as usize != queue.order_ids().len()
            || progress.ids() != [review.native.observation().next_order_id()]
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "original-goal captures do not share one paused fortress frontier",
            ));
        }
        let expected = review.native.observation().next_order_id();
        let row = progress.rows().first().ok_or_else(|| {
            error(
                ErrorCode::AdapterRejected,
                "selected original-order row is absent",
            )
        })?;
        if row.order.is_some() != queue.order_ids().contains(&expected) {
            return Err(error(
                ErrorCode::StaleAnchor,
                "selected progress and complete queue disagree about original-order presence",
            ));
        }
        let linked = matches!(creation, NativeOrderCreation::Inserted { native_order_id, .. } if *native_order_id == expected)
            && row.order.as_ref().is_some_and(|order| {
                order.recipe == review.native.spec().recipe() as u8
                    && order.total == i32::try_from(review.native.spec().amount()).unwrap_or(-1)
            });
        let mut graph = base.graph.clone();
        if graph.entities.len() >= MAX_GOAL_ENTITIES || graph.edges.len() > MAX_GOAL_EDGES {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "complete original-goal operations projection exceeds its bounded domain",
            ));
        }
        let order_entity = created_entity_id(&review.original.steps[0].idempotency_key, 0);
        if graph.entities.contains_key(&order_entity) {
            return Err(error(
                ErrorCode::Conflict,
                "original-order canonical identity collides with an operations entity",
            ));
        }
        if linked && let Some(order) = &row.order {
            let fact = |source: &str, value| {
                Fact::known(
                    value,
                    base.tick,
                    FactSource::DfhackField(source.to_owned()),
                    progress.witness(),
                )
            };
            graph.entities.insert(
                order_entity,
                EntityRecord {
                    id: order_entity,
                    generation: 1,
                    revision: cursor.sequence,
                    kind: EntityKind::WorkOrder,
                    label: format!("native order {expected}"),
                    fields: BTreeMap::from([
                        (
                            "native_order_id".into(),
                            fact(ORDER_ID_SOURCE, Value::U64(u64::from(expected))),
                        ),
                        (
                            "amount_remaining".into(),
                            fact(ORDER_COUNTER_SOURCE, Value::U64(order.remaining as u64)),
                        ),
                        (
                            "amount_total".into(),
                            fact(ORDER_COUNTER_SOURCE, Value::U64(order.total as u64)),
                        ),
                        (
                            "validated".into(),
                            fact(ORDER_STATUS_SOURCE, Value::Bool(order.validated())),
                        ),
                        (
                            "active".into(),
                            fact(ORDER_STATUS_SOURCE, Value::Bool(order.active())),
                        ),
                        (
                            "native_status".into(),
                            fact(
                                ORDER_STATUS_SOURCE,
                                Value::U64(u64::from(order.status_bits)),
                            ),
                        ),
                        (
                            "type_key".into(),
                            fact(ORDER_TYPE_SOURCE, Value::Text(order.type_key.clone())),
                        ),
                        (
                            "reaction".into(),
                            fact(ORDER_TYPE_SOURCE, Value::Text(order.reaction.clone())),
                        ),
                    ]),
                },
            );
        }
        for entity in graph.entities.values_mut() {
            entity.revision = cursor.sequence;
        }
        let snapshot = WorldSnapshot::new(base.fortress_id, base.tick, cursor, true, graph);
        let mut identity = b"dfmcp-work-order-goal-source/1\0".to_vec();
        identity.extend_from_slice(review.seal.as_bytes());
        identity.extend_from_slice(operations.source_digest()?.as_bytes());
        identity.extend_from_slice(progress_before.witness().as_bytes());
        identity.extend_from_slice(progress.witness().as_bytes());
        identity.extend_from_slice(queue.witness().as_bytes());
        identity.extend_from_slice(&progress_manifest.generation.to_be_bytes());
        for value in [
            &progress_manifest.df_version,
            &progress_manifest.dfhack_version,
        ] {
            identity.extend_from_slice(&(value.len() as u64).to_be_bytes());
            identity.extend_from_slice(value.as_bytes());
        }
        Ok(Self {
            snapshot,
            operations: raw.clone(),
            progress_before,
            progress,
            progress_manifest,
            queue,
            review_seal: review.seal,
            original_anchor: review.original.anchor,
            source_digest: Digest32::of_bytes(&identity),
            order_entity,
            linked,
        })
    }
    pub fn snapshot(&self) -> &WorldSnapshot {
        &self.snapshot
    }
    pub fn operations(&self) -> &LiveOperationsObservation {
        &self.operations
    }
    pub fn progress_capture(&self) -> &ProgressObservation {
        &self.progress
    }
    /// The first independently acquired progress endpoint, retained together
    /// with the second so a policy owner can inspect the full capture bracket.
    pub fn progress_before_capture(&self) -> &ProgressObservation {
        &self.progress_before
    }
    pub fn progress_manifest(&self) -> &ProgressManifest {
        &self.progress_manifest
    }
    pub fn queue_capture(&self) -> &WorkOrderObservation {
        &self.queue
    }
    pub fn anchor(&self) -> StateAnchor {
        self.snapshot.anchor()
    }
    pub fn source_digest(&self) -> Digest32 {
        self.source_digest
    }
    pub fn review_seal(&self) -> Digest32 {
        self.review_seal
    }
    pub fn original_anchor(&self) -> StateAnchor {
        self.original_anchor
    }
    pub fn original_order_entity(&self) -> EntityId {
        self.order_entity
    }
    pub fn receipt_linked(&self) -> bool {
        self.linked
    }
    pub const fn schema(&self) -> &'static str {
        WORK_ORDER_GOAL_PROJECTION_SCHEMA
    }

    /// Inventory of source labels/digests actually carried by this projection.
    /// This is not a grant; an independent issuer must validate each source.
    pub fn observed_sources(&self) -> BTreeSet<EvidenceSource> {
        self.snapshot
            .graph
            .entities
            .values()
            .flat_map(|entity| entity.fields.values())
            .chain(
                self.snapshot
                    .graph
                    .edges
                    .values()
                    .flat_map(|edge| edge.fields.values()),
            )
            .filter_map(|fact| match &fact.source {
                FactSource::DfhackField(field) => Some(EvidenceSource::Observed {
                    field: field.clone(),
                    source_digest: fact.source_digest,
                }),
                _ => None,
            })
            .collect()
    }
    pub fn evidence(&self, policy: EvidencePolicy) -> Result<PredicateEvidence<'_>> {
        let sources = self.observed_sources();
        if policy.all_entities != EvidenceCoverage::Unknown
            || policy.all_edges != EvidenceCoverage::Unknown
            || !policy.edge_kinds.is_empty()
            || !policy.terrain_regions.is_empty()
            || policy
                .entity_kinds
                .iter()
                .any(|(kind, coverage)| match kind {
                    EntityKind::Fortress
                    | EntityKind::Job
                    | EntityKind::Item
                    | EntityKind::Building => false,
                    EntityKind::WorkOrder => *coverage == EvidenceCoverage::Complete,
                    _ => true,
                })
            || policy
                .sources
                .iter()
                .any(|source| !sources.contains(source))
        {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "original-goal policy exceeds the exact operations and selected-order domains",
            ));
        }
        PredicateEvidence::scoped(&self.snapshot, policy)
    }
    pub(super) fn physical(
        &self,
        evidence: &PredicateEvidence<'_>,
        policy: &EvidencePolicy,
    ) -> Result<WorkOrderPhysicalState> {
        let unknown = |reason| WorkOrderPhysicalState::Unknown { reason };
        if !self.linked {
            return Ok(unknown(
                "original receipt is unverified, missing, or no longer matches the selected finite order",
            ));
        }
        let row = self
            .progress
            .rows()
            .first()
            .and_then(|row| row.order.as_ref())
            .ok_or_else(|| {
                error(
                    ErrorCode::AdapterRejected,
                    "linked original-order row is absent",
                )
            })?;
        let remaining = Predicate::FieldCompare {
            entity_id: self.order_entity,
            field: "amount_remaining".into(),
            op: CompareOp::Eq,
            value: Value::U64(row.remaining as u64),
        };
        if evidence.evaluate(&remaining)? != PredicateTruth::True {
            return Ok(unknown(
                "original-order counters lack independently qualified evidence",
            ));
        }
        let native_order_id = self.progress.rows()[0].native_order_id;
        if row.remaining > 0 {
            return Ok(WorkOrderPhysicalState::Outstanding {
                native_order_id,
                remaining: row.remaining as u32,
            });
        }
        if !row.validated() || row.active() {
            return Ok(unknown(
                "zero remaining still has unvalidated or active native order status",
            ));
        }
        let status = Predicate::All(vec![
            Predicate::FieldCompare {
                entity_id: self.order_entity,
                field: "validated".into(),
                op: CompareOp::Eq,
                value: Value::Bool(true),
            },
            Predicate::FieldCompare {
                entity_id: self.order_entity,
                field: "active".into(),
                op: CompareOp::Eq,
                value: Value::Bool(false),
            },
        ]);
        if evidence.evaluate(&status)? != PredicateTruth::True
            || policy.entity_kinds.get(&EntityKind::Job) != Some(&EvidenceCoverage::Complete)
        {
            return Ok(unknown(
                "finite completion status or the complete job domain is not qualified",
            ));
        }
        let no_jobs = Predicate::FieldCompare {
            entity_id: EntityId::new(1),
            field: "job_count".into(),
            op: CompareOp::Eq,
            value: Value::U64(0),
        };
        if evidence.evaluate(&no_jobs)? != PredicateTruth::True {
            return Ok(unknown(
                "the complete observed job roster is nonempty or lacks qualified empty-roster evidence",
            ));
        }
        Ok(WorkOrderPhysicalState::Quiescent { native_order_id })
    }
}
