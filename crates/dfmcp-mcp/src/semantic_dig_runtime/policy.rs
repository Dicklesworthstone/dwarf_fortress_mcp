//! Independent operator acceptance for exact dig/1.16 and terrain/1.8 sources.
//! Development source acceptance grants no compatibility admission or checkpoint.
use super::{denied, invalid};
use dfmcp_adapter::dig_designation::DigObservation;
use dfmcp_adapter::dig_designation::journal::DigBinding;
use dfmcp_adapter::live_spatial::citizens::LiveSpatialCitizenObservation;
use dfmcp_adapter::semantic_dig::goal_projection::DigGoalProjection;
use dfmcp_adapter::semantic_dig::terrain::DigTerrainProjection;
use dfmcp_core::{Capability, FortressId, OperationContext, Result, RiskTier};
use dfmcp_world::EvidencePolicy;
use dfmcp_world::map_region::Region;
use serde::Deserialize;
pub const ACCEPTANCE_KIND: &str = "operator-development-source-acceptance/1";
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceAcceptance {
    pub kind: String,
    pub world_folder: String,
    pub site_id: u32,
    pub df_version: String,
    pub dfhack_version: String,
    pub spatial_generation: u64,
    pub dig_generation: u64,
    pub map_dimensions: [u32; 3],
}
impl SourceAcceptance {
    pub fn validate(&self) -> Result<()> {
        if self.kind != ACCEPTANCE_KIND
            || self.world_folder.is_empty()
            || self.world_folder.len() > 512
            || self.world_folder.chars().any(char::is_control)
            || self.site_id > i32::MAX as u32
            || [&self.df_version, &self.dfhack_version]
                .iter()
                .any(|v| v.is_empty() || v.len() > 128 || v.chars().any(char::is_control))
            || [self.spatial_generation, self.dig_generation]
                .into_iter()
                .any(|n| n == 0 || n == u64::MAX)
            || self.map_dimensions.into_iter().any(|n| n == 0 || n > 32768)
        {
            return Err(invalid(
                "invalid exact operator excavation source acceptance",
            ));
        }
        Ok(())
    }
    pub fn fortress(&self) -> FortressId {
        dfmcp_adapter::workforce_control::fortress_id(&self.world_folder, self.site_id)
    }
    fn context(&self, c: &OperationContext) -> Result<()> {
        self.validate()?;
        if c.anchor.fortress_id != self.fortress() {
            return Err(denied("dig source acceptance belongs to another fortress"));
        }
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        c.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)
    }
    pub fn binding(&self, b: &DigBinding) -> Result<()> {
        self.validate()?;
        if b.fortress_id() != self.fortress()
            || b.folder() != self.world_folder
            || b.site() != self.site_id
            || b.manifest().generation != self.dig_generation
            || b.manifest().df_version != self.df_version
            || b.manifest().dfhack_version != self.dfhack_version
        {
            return Err(denied(
                "native dig binding differs from exact operator pins",
            ));
        }
        Ok(())
    }
    pub fn capture(&self, b: &DigObservation) -> Result<()> {
        self.validate()?;
        if b.fortress_id() != self.fortress()
            || b.folder() != self.world_folder
            || b.site() != self.site_id
            || b.generation() != self.dig_generation
            || b.dimensions() != self.map_dimensions
        {
            return Err(denied(
                "native dig capture differs from original source pins",
            ));
        }
        Ok(())
    }
    pub fn observation(
        &self,
        observed: &LiveSpatialCitizenObservation,
        region: Region,
    ) -> Result<()> {
        self.validate()?;
        observed.validate()?;
        let t = observed.spatial().terrain();
        if t.fortress_id()? != self.fortress()
            || t.world_folder != self.world_folder
            || t.site_id != self.site_id
            || t.bridge_generation != self.spatial_generation
            || t.df_version != self.df_version
            || t.dfhack_version != self.dfhack_version
            || t.map_dimensions != self.map_dimensions
            || t.map.region != region
            || !t.paused
        {
            return Err(denied(
                "complete terrain source differs from exact operator excavation pins",
            ));
        }
        Ok(())
    }
    pub fn spatial(
        &self,
        p: &DigTerrainProjection,
        region: Region,
        c: &OperationContext,
    ) -> Result<EvidencePolicy> {
        self.context(c)?;
        self.observation(p.observation(), region)?;
        if p.anchor() != c.anchor {
            return Err(denied(
                "terrain policy requires the exact canonical publication",
            ));
        }
        let mut policy = EvidencePolicy::at(p.anchor());
        policy.paused = true;
        policy.terrain_regions = p.terrain_regions().to_vec();
        p.evidence(policy.clone())?;
        Ok(policy)
    }
    pub fn clock_terrain(
        &self,
        p: &DigGoalProjection,
        region: Region,
        c: &OperationContext,
    ) -> Result<()> {
        self.context(c)?;
        self.observation(p.raw(), region)?;
        if p.anchor() != c.anchor || p.original_anchor().fortress_id != self.fortress() {
            return Err(denied(
                "clock terrain requires the exact independently accepted canonical excavation projection",
            ));
        }
        Ok(())
    }
    pub fn goal(
        &self,
        p: &DigGoalProjection,
        region: Region,
        c: &OperationContext,
    ) -> Result<EvidencePolicy> {
        self.context(c)?;
        self.observation(p.raw(), region)?;
        if p.anchor().fortress_id != self.fortress()
            || p.original_anchor().fortress_id != self.fortress()
            || c.anchor.cursor.epoch != p.anchor().cursor.epoch
            || c.anchor.tick < p.anchor().tick
        {
            return Err(denied(
                "original excavation goal differs from exact operator source pins",
            ));
        }
        let mut policy = EvidencePolicy::at(p.anchor());
        policy.paused = true;
        policy.terrain_regions = p.terrain_regions().to_vec();
        policy.sources = p.evidence_sources();
        p.evidence(policy.clone())?;
        Ok(policy)
    }
}
