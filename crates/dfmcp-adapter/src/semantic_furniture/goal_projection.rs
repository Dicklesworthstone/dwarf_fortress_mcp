//! Exact original construction identity and independently authorized evidence.
//!
//! Native registration is historical. The construction-stage derivation uses
//! both original receipt brackets and the complete current operations roster.
//! Its source digest is an inventory of inputs, never a source-policy grant.

use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{
    Digest32, EntityId, ErrorCode, ObservationCursor, OperationContext, Result, StateAnchor,
};
use dfmcp_intent::effects::{CONSTRUCTION_STAGE_FIELD, STAGE_COMPLETE, created_entity_id};
use dfmcp_world::{
    CompareOp, EntityKind, EntityRecord, EvidenceCoverage, EvidencePolicy, EvidenceSource, Fact,
    FactSource, Predicate, PredicateEvidence, PredicateTruth, Value, WorldGraph, WorldSnapshot,
};

use super::SemanticFurnitureReview;
use crate::build_placement::journal::{BuildEntry, BuildState};
use crate::build_placement::{BuildBinding, BuildPhase};
use crate::construction_plan::{LinkedSample, ReceiptAssessment, assess_receipt};

pub const FURNITURE_GOAL_PROJECTION_SCHEMA: &str = "dfmcp.original_furniture_goal_projection/1";
pub const CONSTRUCTION_DERIVATION: &str = "dfmcp.original-furniture-construction/1";
pub const MAX_GOAL_ENTITIES: usize = 8192;
pub const MAX_GOAL_ATTACHMENTS: usize = 16384;

fn error(code: ErrorCode, message: &str) -> dfmcp_core::DfmcpError {
    dfmcp_core::DfmcpError::new(code, message)
}

/// Selected-building evidence only. No global building, item, job, terrain or
/// entity-absence domain is asserted by this projection.
#[derive(Clone, Debug)]
pub struct FurnitureGoalProjection {
    snapshot: WorldSnapshot,
    sample: LinkedSample,
    assessment: ReceiptAssessment,
    original_anchor: StateAnchor,
    review_seal: Digest32,
    source_digest: Digest32,
    entity: EntityId,
    linked: bool,
}

impl FurnitureGoalProjection {
    pub(super) fn new(
        review: &SemanticFurnitureReview,
        binding: &BuildBinding,
        entry: &BuildEntry,
        sample: LinkedSample,
        cursor: ObservationCursor,
        context: &OperationContext,
    ) -> Result<Self> {
        let original = review.original_plan();
        let record = entry
            .native()
            .filter(|record| {
                entry.state() == BuildState::Terminal && record.phase() == BuildPhase::Placed
            })
            .ok_or_else(|| {
                error(
                    ErrorCode::EffectIndeterminate,
                    "original construction proof requires reconciled historical placement",
                )
            })?;
        if entry.plan() != review.native_plan()
            || record.plan() != review.native_plan()
            || !binding.capture_matches(record.plan().before())
            || cursor.epoch != original.anchor.cursor.epoch
            || cursor.sequence <= original.anchor.cursor.sequence
            || sample.before.generation != binding.generation()
            || sample.before.df_version != binding.df_version()
            || sample.before.dfhack_version != binding.dfhack_version()
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "construction source or canonical cursor differs from the original placement",
            ));
        }
        let assessment = assess_receipt(record, &sample, context)?;
        let raw = &assessment.operations;
        if !raw.jobs.paused
            || raw.jobs.fortress_id()? != original.anchor.fortress_id
            || raw.jobs.world_folder != binding.fortress().folder()
            || raw.jobs.site_id < 0
            || raw.jobs.site_id as u32 != binding.fortress().site()
            || raw.jobs.tick() < original.anchor.tick
            || raw.jobs.df_version != binding.df_version()
            || raw.jobs.dfhack_version != binding.dfhack_version()
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "construction capture is not the original paused fortress and software",
            ));
        }
        if 1usize
            .saturating_add(raw.jobs.jobs.len())
            .saturating_add(raw.buildings.len())
            .saturating_add(raw.items.len())
            > MAX_GOAL_ENTITIES
            || raw.attachments.len() > MAX_GOAL_ATTACHMENTS
        {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "original construction capture exceeds the retained proof domain",
            ));
        }
        let insertion = record.insertion().ok_or_else(|| {
            error(
                ErrorCode::CorruptLedger,
                "original Placed receipt has no insertion identity",
            )
        })?;
        let entity = created_entity_id(&original.steps[0].idempotency_key, 0);
        let linked = !matches!(
            assessment.condition.status,
            "world_identity_mismatch"
                | "source_regressed"
                | "building_missing"
                | "item_missing"
                | "building_identity_mismatch"
                | "item_identity_mismatch"
                | "original_job_identity_mismatch"
        );
        let mut identity = b"dfmcp.original-furniture-goal-source/1\0".to_vec();
        identity.extend_from_slice(review.seal().as_bytes());
        identity.extend_from_slice(assessment.source_digest.as_bytes());
        identity.extend_from_slice(&binding.canonical_bytes());
        let source_digest = Digest32::of_bytes(&identity);
        let mut graph = WorldGraph::default();
        if linked {
            // This is a certified interpretation of the entire linked sample,
            // not a raw field falsely labeled as direct native observation.
            let fact = |value| {
                Fact::known(
                    value,
                    raw.jobs.tick(),
                    FactSource::Derived(CONSTRUCTION_DERIVATION.to_owned()),
                    source_digest,
                )
            };
            let complete = assessment.condition.status == "condition_met";
            graph.entities.insert(
                entity,
                EntityRecord {
                    id: entity,
                    generation: 1,
                    revision: cursor.sequence,
                    kind: EntityKind::Building,
                    label: format!("original furniture {}", insertion.building_id()),
                    fields: BTreeMap::from([
                        (
                            CONSTRUCTION_STAGE_FIELD.to_owned(),
                            fact(Value::Text(
                                if complete { STAGE_COMPLETE } else { "pending" }.to_owned(),
                            )),
                        ),
                        (
                            "construction_condition".to_owned(),
                            fact(Value::Text(assessment.condition.status.to_owned())),
                        ),
                        (
                            "native_building_id".to_owned(),
                            fact(Value::U64(u64::from(insertion.building_id()))),
                        ),
                        (
                            "original_item_id".to_owned(),
                            fact(Value::U64(u64::from(insertion.item_id()))),
                        ),
                        (
                            "original_construction_job_id".to_owned(),
                            fact(Value::U64(u64::from(insertion.job_id()))),
                        ),
                        (
                            "construction_jobs".to_owned(),
                            fact(Value::U64(u64::from(
                                assessment.condition.construction_jobs,
                            ))),
                        ),
                        (
                            "removal_jobs".to_owned(),
                            fact(Value::U64(u64::from(assessment.condition.removal_jobs))),
                        ),
                        (
                            "item_job_links".to_owned(),
                            fact(Value::U64(u64::from(assessment.condition.item_job_links))),
                        ),
                        (
                            "raw_build_stage".to_owned(),
                            fact(Value::I64(i64::from(
                                assessment.condition.stage.ok_or_else(|| {
                                    error(
                                        ErrorCode::CorruptLedger,
                                        "linked construction lacks its current stage",
                                    )
                                })?,
                            ))),
                        ),
                        (
                            "max_build_stage".to_owned(),
                            fact(Value::U64(u64::from(assessment.condition.max_stage))),
                        ),
                    ]),
                },
            );
        }
        let snapshot = WorldSnapshot::new(
            original.anchor.fortress_id,
            raw.jobs.tick(),
            cursor,
            true,
            graph,
        );
        Ok(Self {
            snapshot,
            sample,
            assessment,
            original_anchor: original.anchor,
            review_seal: review.seal(),
            source_digest,
            entity,
            linked,
        })
    }

    pub fn snapshot(&self) -> &WorldSnapshot {
        &self.snapshot
    }
    pub fn anchor(&self) -> StateAnchor {
        self.snapshot.anchor()
    }
    pub fn sample(&self) -> &LinkedSample {
        &self.sample
    }
    pub fn assessment(&self) -> &ReceiptAssessment {
        &self.assessment
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
    pub fn original_building_entity(&self) -> EntityId {
        self.entity
    }
    pub fn receipt_linked(&self) -> bool {
        self.linked
    }
    pub const fn schema(&self) -> &'static str {
        FURNITURE_GOAL_PROJECTION_SCHEMA
    }

    /// Exact derivation inputs available for independent validation. This set
    /// carries no authority and is never accepted from a client as a grant.
    pub fn evidence_sources(&self) -> BTreeSet<EvidenceSource> {
        if self.linked {
            BTreeSet::from([EvidenceSource::CertifiedDerived {
                derivation: CONSTRUCTION_DERIVATION.to_owned(),
                source_digest: self.source_digest,
            }])
        } else {
            BTreeSet::new()
        }
    }

    pub fn evidence(&self, policy: EvidencePolicy) -> Result<PredicateEvidence<'_>> {
        if policy.all_entities != EvidenceCoverage::Unknown
            || policy.all_edges != EvidenceCoverage::Unknown
            || !policy.edge_kinds.is_empty()
            || !policy.terrain_regions.is_empty()
            || policy.entity_kinds.iter().any(|(kind, coverage)| {
                *kind != EntityKind::Building || *coverage == EvidenceCoverage::Complete
            })
            || policy
                .sources
                .iter()
                .any(|source| !self.evidence_sources().contains(source))
        {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "construction policy exceeds the exact receipt-linked building domain",
            ));
        }
        PredicateEvidence::scoped(&self.snapshot, policy)
    }

    pub(super) fn construction_proven(&self, evidence: &PredicateEvidence<'_>) -> Result<bool> {
        Ok(self.linked
            && self.assessment.condition.status == "condition_met"
            && evidence.evaluate(&Predicate::FieldCompare {
                entity_id: self.entity,
                field: CONSTRUCTION_STAGE_FIELD.to_owned(),
                op: CompareOp::Eq,
                value: Value::Text(STAGE_COMPLETE.to_owned()),
            })? == PredicateTruth::True)
    }
}
