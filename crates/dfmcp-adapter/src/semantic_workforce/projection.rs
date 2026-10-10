//! Selected-citizen workforce/1.17 observations for original-goal proof.
//!
//! These are fresh capture facts, never reconstructed Applied receipt facts.
//! Projection is deterministic normalization, not source admission. A separate
//! trusted policy issuer must authorize the exact capture and resulting anchor.

use std::collections::BTreeMap;

use dfmcp_core::{Digest32, EntityId, ErrorCode, GameTick, ObservationCursor, Result, StateAnchor};
use dfmcp_world::{
    EntityKind, EntityRecord, EvidenceCoverage, EvidencePolicy, EvidenceSource, Fact, FactSource,
    PredicateEvidence, Value, WorldGraph, WorldSnapshot,
};

use crate::live_routing::{LiveIdentitySchema, LiveRoutingEvidence, NativeUnitIdentity, resolve_workforce_step};
use crate::workforce_control::WorkforceCapture;

use super::{SemanticWorkforceReview, custody, error};

pub const WORKFORCE_GOAL_PROJECTION_SCHEMA: &str = "dfmcp.workforce_goal_projection/1";
pub const UNIT_ID_SOURCE: &str = "workforce/1.17.ObserveWorkforce.citizen.unit_id";
pub const HISTORICAL_ID_SOURCE: &str = "workforce/1.17.ObserveWorkforce.citizen.historical_id";
pub const ELIGIBILITY_SOURCE: &str = "workforce/1.17.ObserveWorkforce.citizen.eligible";
pub const LABOR_SOURCE: &str = "workforce/1.17.ObserveWorkforce.citizen.labors";

fn stale(message: &str) -> dfmcp_core::DfmcpError {
    error(ErrorCode::StaleAnchor, message)
}

/// Exact identity lineage retained by one original reviewed action. Construction
/// requires the original independently eligible typed projection; no ID cast or
/// caller-selected schema can supply this mapping.
#[derive(Clone, Debug)]
pub(super) struct IdentityBinding {
    schema: LiveIdentitySchema,
    units: Vec<NativeUnitIdentity>,
    before: WorkforceCapture,
    original_anchor: StateAnchor,
    review_seal: Digest32,
}

impl IdentityBinding {
    pub(super) fn new(
        review: &SemanticWorkforceReview,
        evidence: &LiveRoutingEvidence<'_>,
    ) -> Result<Self> {
        let route = resolve_workforce_step(
            &review.original,
            review.association.step,
            evidence,
            review.native.before(),
        )?;
        if route.native_plan() != &review.native
            || route.source_digest() != review.association.source
            || evidence.anchor() != review.original.anchor
        {
            return Err(custody());
        }
        Ok(Self {
            schema: evidence.schema(),
            units: route.units().to_vec(),
            before: review.native.before().clone(),
            original_anchor: review.original.anchor,
            review_seal: review.seal,
        })
    }

    pub(super) fn units(&self) -> Vec<EntityId> {
        self.units.iter().map(|unit| unit.canonical_id).collect()
    }

    pub(super) fn native_ids(&self) -> Vec<u32> {
        self.units.iter().map(|unit| unit.native_id).collect()
    }

    pub(super) fn validate_capture(&self, capture: &WorkforceCapture) -> Result<()> {
        if capture.fortress_id() != self.original_anchor.fortress_id
            || capture.folder() != self.before.folder()
            || capture.site() != self.before.site()
            || capture.generation() != self.before.generation()
            || capture.tick() < self.before.tick()
            || capture.sequence() < self.before.sequence()
            || capture.labor_keys() != self.before.labor_keys()
            || capture.citizens().len() != self.units.len()
        {
            return Err(stale("workforce goal observation changed its original source, clock or labor schema"));
        }
        for ((identity, original), current) in self.units.iter()
            .zip(self.before.citizens())
            .zip(capture.citizens())
        {
            if identity.native_id != original.id()
                || current.id() != original.id()
                || current.historical_id() != original.historical_id()
            {
                return Err(stale("workforce goal observation replaced an original selected citizen"));
            }
        }
        Ok(())
    }

    pub(super) fn project(
        &self,
        capture: WorkforceCapture,
        cursor: ObservationCursor,
    ) -> Result<WorkforceGoalProjection> {
        self.validate_capture(&capture)?;
        if cursor.epoch != self.original_anchor.cursor.epoch
            || cursor.sequence <= self.original_anchor.cursor.sequence
        {
            return Err(stale("workforce goal projection requires a later cursor in the original epoch"));
        }
        let tick = GameTick(capture.tick());
        let digest = capture.witness();
        let mut entities = BTreeMap::new();
        for (identity, citizen) in self.units.iter().zip(capture.citizens()) {
            let mut fields = BTreeMap::new();
            let (field, value) = match self.schema {
                LiveIdentitySchema::CitizensV1 => ("raw_unit_id", Value::I64(i64::from(citizen.id()))),
                LiveIdentitySchema::SpatialV1_8 => ("native_unit_id", Value::U64(u64::from(citizen.id()))),
            };
            fields.insert(field.to_owned(), observed(value, tick, UNIT_ID_SOURCE, digest));
            fields.insert("historical_figure_id".to_owned(), observed(
                Value::U64(u64::from(citizen.historical_id())), tick, HISTORICAL_ID_SOURCE, digest,
            ));
            fields.insert("workforce_eligible".to_owned(), observed(
                Value::Bool(citizen.eligible()), tick, ELIGIBILITY_SOURCE, digest,
            ));
            for (key, bit) in capture.labor_keys().iter().zip(citizen.labors()) {
                fields.insert(format!("labor.{key}"), observed(
                    Value::Bool(*bit == 1), tick, LABOR_SOURCE, digest,
                ));
            }
            entities.insert(identity.canonical_id, EntityRecord {
                id: identity.canonical_id,
                generation: identity.canonical_generation,
                revision: cursor.sequence,
                kind: EntityKind::Unit,
                label: format!("citizen {}", citizen.id()),
                fields,
            });
        }
        let snapshot = WorldSnapshot::new(
            self.original_anchor.fortress_id, tick, cursor, capture.paused(),
            WorldGraph { entities, ..WorldGraph::default() },
        );
        Ok(WorkforceGoalProjection {
            snapshot, capture, original_anchor: self.original_anchor,
            review_seal: self.review_seal, schema: self.schema,
        })
    }
}

fn observed(value: Value, tick: GameTick, source: &str, digest: Digest32) -> Fact {
    Fact::known(value, tick, FactSource::DfhackField(source.to_owned()), digest)
}

/// One immutable, bounded selected-unit observation. Missing units, relations,
/// terrain and other fields are unknown. The caller allocates its canonical
/// cursor; the native dispatch sequence is never an observation cursor.
#[derive(Clone, Debug)]
pub struct WorkforceGoalProjection {
    snapshot: WorldSnapshot,
    capture: WorkforceCapture,
    original_anchor: StateAnchor,
    review_seal: Digest32,
    schema: LiveIdentitySchema,
}

impl WorkforceGoalProjection {
    pub fn snapshot(&self) -> &WorldSnapshot { &self.snapshot }
    pub fn capture(&self) -> &WorkforceCapture { &self.capture }
    pub fn anchor(&self) -> StateAnchor { self.snapshot.anchor() }
    pub fn source_digest(&self) -> Digest32 { self.capture.witness() }
    pub fn original_anchor(&self) -> StateAnchor { self.original_anchor }
    pub fn review_seal(&self) -> Digest32 { self.review_seal }
    pub fn identity_schema(&self) -> LiveIdentitySchema { self.schema }
    pub const fn schema(&self) -> &'static str { WORKFORCE_GOAL_PROJECTION_SCHEMA }

    /// Validate a separately issued policy against this projection's actual
    /// limits. A selected capture cannot certify whole-roster absence or any
    /// unobserved graph/terrain domain, even if an issuer accidentally requests it.
    pub fn evidence(&self, policy: EvidencePolicy) -> Result<PredicateEvidence<'_>> {
        if policy.all_entities == EvidenceCoverage::Complete
            || policy.entity_kinds.iter().any(|(kind, coverage)|
                kind != &EntityKind::Unit || *coverage == EvidenceCoverage::Complete)
            || policy.all_edges != EvidenceCoverage::Unknown
            || !policy.edge_kinds.is_empty()
            || !policy.terrain_regions.is_empty()
            || policy.sources.iter().any(|source| !matches!(source,
                EvidenceSource::Observed { field, source_digest }
                    if *source_digest == self.source_digest()
                        && [UNIT_ID_SOURCE, HISTORICAL_ID_SOURCE, ELIGIBILITY_SOURCE, LABOR_SOURCE]
                            .contains(&field.as_str())))
        {
            return Err(error(ErrorCode::CapabilityDenied,
                "workforce goal policy exceeds the exact observed selected-citizen domain"));
        }
        PredicateEvidence::scoped(&self.snapshot, policy)
    }
}
