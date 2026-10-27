//! Exact operator acceptance for this isolated development runtime. These pins
//! neither populate the compatibility registry nor authorize another profile.
use super::{denied, invalid};
use dfmcp_adapter::live_projection::LiveWorldProjection;
use dfmcp_adapter::semantic_workforce::WorkforceGoalProjection;
use dfmcp_adapter::semantic_workforce::projection::{
    ELIGIBILITY_SOURCE, HISTORICAL_ID_SOURCE, LABOR_SOURCE, UNIT_ID_SOURCE,
};
use dfmcp_adapter::workforce_control::rpc::WorkforceManifest;
use dfmcp_adapter::{BridgeManifest, LiveObservationCapsule};
use dfmcp_core::{Capability, FortressId, OperationContext, Result, RiskTier};
use dfmcp_world::{EntityKind, EvidenceCoverage, EvidencePolicy, EvidenceSource};
use serde::Deserialize;

pub const ACCEPTANCE_KIND: &str = "operator-development-source-acceptance/1";

/// The citizen and workforce incarnation numbers belong to independent plugin
/// namespaces. Each exact source is pinned; their numbers need not be equal.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceAcceptance {
    pub kind: String,
    pub world_folder: String,
    pub site_id: u32,
    pub df_version: String,
    pub dfhack_version: String,
    pub citizen_bridge_version: String,
    pub citizen_generation: u64,
    pub workforce_generation: u64,
}
impl SourceAcceptance {
    pub fn validate(&self) -> Result<()> {
        if self.kind != ACCEPTANCE_KIND
            || self.world_folder.is_empty()
            || self.world_folder.len() > 512
            || self.world_folder.chars().any(char::is_control)
            || self.site_id > i32::MAX as u32
            || [
                &self.df_version,
                &self.dfhack_version,
                &self.citizen_bridge_version,
            ]
            .iter()
            .any(|value| {
                value.is_empty() || value.len() > 64 || value.chars().any(char::is_control)
            })
            || [self.citizen_generation, self.workforce_generation]
                .iter()
                .any(|generation| *generation == 0 || *generation == u64::MAX)
        {
            return Err(invalid(
                "invalid exact operator workforce source acceptance",
            ));
        }
        Ok(())
    }
    pub fn fortress(&self) -> FortressId {
        dfmcp_adapter::workforce_control::fortress_id(&self.world_folder, self.site_id)
    }
    fn context(&self, context: &OperationContext) -> Result<()> {
        self.validate()?;
        if context.anchor.fortress_id != self.fortress() {
            return Err(denied(
                "workforce source acceptance belongs to another fortress",
            ));
        }
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)
    }
    pub fn citizen_manifest(&self, manifest: &BridgeManifest) -> Result<()> {
        manifest.validate()?;
        if manifest.bridge_generation != self.citizen_generation
            || manifest.bridge_version != self.citizen_bridge_version
            || manifest.df_version != self.df_version
            || manifest.dfhack_version != self.dfhack_version
            || !manifest.world_loaded
            || !manifest.fortress_mode
        {
            return Err(denied(
                "citizen source differs from operator workforce pins",
            ));
        }
        Ok(())
    }
    pub fn native_manifest(&self, manifest: &WorkforceManifest) -> Result<()> {
        if manifest.generation != self.workforce_generation
            || manifest.df_version != self.df_version
            || manifest.dfhack_version != self.dfhack_version
        {
            return Err(denied("native workforce source differs from operator pins"));
        }
        Ok(())
    }
    pub fn citizens(
        &self,
        capsule: &LiveObservationCapsule,
        projection: &LiveWorldProjection,
        context: &OperationContext,
    ) -> Result<EvidencePolicy> {
        if projection.snapshot.anchor() != context.anchor {
            return Err(denied(
                "fresh citizen policy requires the exact current anchor",
            ));
        }
        self.historical_citizens(capsule, projection, context)
    }
    /// Authorize original identity lineage under the current authority floor.
    /// The resulting scope remains anchored to the original capsule and cannot
    /// establish that the current game still has those facts.
    pub fn historical_citizens(
        &self,
        capsule: &LiveObservationCapsule,
        projection: &LiveWorldProjection,
        context: &OperationContext,
    ) -> Result<EvidencePolicy> {
        self.context(context)?;
        capsule.validate()?;
        projection.validate_against(capsule)?;
        self.citizen_manifest(&capsule.bridge)?;
        if capsule.world_folder != self.world_folder
            || capsule.site_id != self.site_id as i32
            || !capsule.paused
            || projection.snapshot.fortress_id != self.fortress()
            || context.anchor.cursor.epoch != projection.snapshot.cursor.epoch
            || context.anchor.cursor.sequence < projection.snapshot.cursor.sequence
            || context.anchor.tick < projection.snapshot.tick
            || (context.anchor.cursor == projection.snapshot.cursor
                && context.anchor != projection.snapshot.anchor())
        {
            return Err(denied(
                "canonical citizens differ from exact operator workforce scope",
            ));
        }
        let mut policy = EvidencePolicy::at(projection.snapshot.anchor());
        policy.paused = true;
        policy
            .entity_kinds
            .insert(EntityKind::Unit, EvidenceCoverage::Observed);
        policy.sources.insert(EvidenceSource::Observed {
            field: "dfmcp_bridge.ReadObservation.citizen.unit_id".into(),
            source_digest: capsule.content_digest,
        });
        Ok(policy)
    }
    pub fn goal(
        &self,
        projection: &WorkforceGoalProjection,
        context: &OperationContext,
    ) -> Result<EvidencePolicy> {
        self.context(context)?;
        let capture = projection.capture();
        if projection.anchor().fortress_id != self.fortress()
            || context.anchor.cursor.epoch != projection.anchor().cursor.epoch
            || context.anchor.tick < projection.anchor().tick
            || capture.folder() != self.world_folder
            || capture.site() != self.site_id
            || capture.generation() != self.workforce_generation
            || !capture.paused()
        {
            return Err(denied(
                "original labor goal differs from exact operator workforce pins",
            ));
        }
        let mut policy = EvidencePolicy::at(projection.anchor());
        policy.paused = true;
        // A selected capture grants positive facts only: it does not establish
        // complete-roster absence, job execution, productivity or path access.
        policy
            .entity_kinds
            .insert(EntityKind::Unit, EvidenceCoverage::Observed);
        for field in [
            UNIT_ID_SOURCE,
            HISTORICAL_ID_SOURCE,
            ELIGIBILITY_SOURCE,
            LABOR_SOURCE,
        ] {
            policy.sources.insert(EvidenceSource::Observed {
                field: field.into(),
                source_digest: projection.source_digest(),
            });
        }
        projection.evidence(policy.clone())?;
        Ok(policy)
    }
}
