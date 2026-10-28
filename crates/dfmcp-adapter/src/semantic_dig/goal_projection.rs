//! Exact original Mine receipt plus independently qualified current terrain.
//! Designated is historical configuration. Only a new complete spatial capture
//! can establish the original RegionTerrain goal, and only under host authority.
use super::terrain::{DigTerrainProjection, source_identity_digest};
use super::{SemanticDigReview, custody, error};
use crate::dig_designation::journal::{DigBinding, DigRecord, DigState};
use crate::dig_designation::{DigEffect, DigPhase};
use crate::live_spatial::citizens::LiveSpatialCitizenObservation;
use dfmcp_core::{
    Digest32, ErrorCode, MapCuboid, ObservationCursor, OperationContext, Result, StateAnchor,
};
use dfmcp_world::{
    EvidencePolicy, EvidenceSource, Predicate, PredicateEvidence, PredicateTruth, WorldSnapshot,
};
use std::collections::BTreeSet;
pub const DIG_GOAL_PROJECTION_SCHEMA: &str = "dfmcp.original_dig_goal_projection/1";
#[derive(Clone, Debug)]
pub struct DigGoalProjection {
    terrain: DigTerrainProjection,
    original_anchor: StateAnchor,
    review_seal: Digest32,
    source_digest: Digest32,
    target: MapCuboid,
}
impl DigGoalProjection {
    pub(super) fn new(
        review: &SemanticDigReview,
        binding: &DigBinding,
        entry: &DigRecord,
        raw: LiveSpatialCitizenObservation,
        cursor: ObservationCursor,
        c: &OperationContext,
    ) -> Result<Self> {
        let effect = entry
            .effect()
            .filter(|e| entry.state() == DigState::Terminal && e.phase() == DigPhase::Designated)
            .ok_or_else(|| {
                error(
                    ErrorCode::EffectIndeterminate,
                    "original excavation requires reconciled historical designation",
                )
            })?;
        DigEffect::decode(effect.canonical_bytes(), review.native_plan())?;
        let original = review.original_plan();
        let map = raw.spatial().terrain();
        if entry.plan() != review.native_plan()
            || binding.fortress_id() != original.anchor.fortress_id
            || binding.folder() != map.world_folder
            || binding.site() != map.site_id
            || binding.manifest().generation != review.native_plan().before().generation()
            || binding.manifest().df_version != map.df_version
            || binding.manifest().dfhack_version != map.dfhack_version
            || map.map_dimensions != review.native_plan().before().dimensions()
            || source_identity_digest(&raw)? != review.source_identity_digest()
            || map.tick() < original.anchor.tick
            || cursor.epoch != original.anchor.cursor.epoch
            || cursor.sequence <= original.anchor.cursor.sequence
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "original excavation source, receipt, dimensions or canonical epoch changed",
            ));
        }
        let dfmcp_intent::Action::DesignateDig {
            area,
            mode: dfmcp_intent::DigMode::Mine,
        } = &original.steps[0].action
        else {
            return Err(custody());
        };
        let target = *area;
        let terrain = DigTerrainProjection::historical(raw, cursor, c)?;
        let mut identity = b"dfmcp.original-dig-goal-source/1\0".to_vec();
        identity.extend_from_slice(review.seal().as_bytes());
        identity.extend_from_slice(terrain.source_digest().as_bytes());
        identity.extend_from_slice(effect.receipt().ok_or_else(custody)?.as_bytes());
        Ok(Self {
            terrain,
            original_anchor: original.anchor,
            review_seal: review.seal(),
            source_digest: Digest32::of_bytes(&identity),
            target,
        })
    }
    pub fn snapshot(&self) -> &WorldSnapshot {
        self.terrain.snapshot()
    }
    pub fn anchor(&self) -> StateAnchor {
        self.terrain.anchor()
    }
    pub fn raw(&self) -> &LiveSpatialCitizenObservation {
        self.terrain.raw()
    }
    pub fn sample(&self) -> &LiveSpatialCitizenObservation {
        self.raw()
    }
    pub fn source_digest(&self) -> Digest32 {
        self.source_digest
    }
    pub fn source_identity_digest(&self) -> Digest32 {
        self.terrain.source_identity_digest()
    }
    pub fn original_anchor(&self) -> StateAnchor {
        self.original_anchor
    }
    pub fn review_seal(&self) -> Digest32 {
        self.review_seal
    }
    pub fn receipt_linked(&self) -> bool {
        true
    }
    pub const fn schema(&self) -> &'static str {
        DIG_GOAL_PROJECTION_SCHEMA
    }
    pub fn evidence_sources(&self) -> BTreeSet<EvidenceSource> {
        self.terrain.evidence_sources()
    }
    pub fn terrain_regions(&self) -> &[MapCuboid] {
        self.terrain.terrain_regions()
    }
    pub fn evidence(&self, policy: EvidencePolicy) -> Result<PredicateEvidence<'_>> {
        self.terrain.evidence(policy)
    }
    pub(super) fn excavation_proven(&self, evidence: &PredicateEvidence<'_>) -> Result<bool> {
        Ok(evidence.evaluate(&Predicate::RegionTerrain {
            area: self.target,
            tile_code: dfmcp_world::terrain::tile_codes::FLOOR,
        })? == PredicateTruth::True)
    }
    pub fn excavation_condition(&self) -> &'static str {
        let mut unknown = false;
        for coord in dfmcp_world::terrain::region_tiles(self.target) {
            if !self.terrain_regions().iter().any(|a| a.contains(coord)) {
                unknown = true;
                continue;
            }
            match self.snapshot().tile_code_at(coord) {
                Some(dfmcp_world::terrain::tile_codes::FLOOR) => {}
                Some(_) => return "pending",
                None => unknown = true,
            }
        }
        if unknown {
            "unobserved_or_uncleared"
        } else {
            "condition_met"
        }
    }
}
