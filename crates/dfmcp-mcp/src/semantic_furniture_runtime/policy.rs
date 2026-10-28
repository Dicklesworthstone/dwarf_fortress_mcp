//! Exact operator source acceptance and host-local construction scope.
//! No compatibility entry, process admission or checkpoint is issued here.

use super::{denied, invalid};
use dfmcp_adapter::build_placement::{BuildBinding, BuildCapture};
use dfmcp_adapter::live_spatial::SpatialStateView;
use dfmcp_adapter::live_spatial::citizens::{
    LiveSpatialCitizenObservation, LiveSpatialCitizenState,
};
use dfmcp_adapter::semantic_furniture::goal_projection::FurnitureGoalProjection;
use dfmcp_core::{Capability, FortressId, OperationContext, Result, RiskTier};
use dfmcp_world::map_region::Region;
use dfmcp_world::{EntityKind, EvidenceCoverage, EvidencePolicy, EvidenceSource};
use serde::Deserialize;

pub const ACCEPTANCE_KIND: &str = "operator-development-source-acceptance/1";

/// All three native generations belong to independent plugin namespaces.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceAcceptance {
    pub kind: String,
    pub world_folder: String,
    pub site_id: u32,
    pub df_version: String,
    pub dfhack_version: String,
    pub spatial_generation: u64,
    pub furniture_generation: u64,
    pub operations_generation: u64,
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
                .any(|value| {
                    value.is_empty() || value.len() > 128 || value.chars().any(char::is_control)
                })
            || [
                self.spatial_generation,
                self.furniture_generation,
                self.operations_generation,
            ]
            .into_iter()
            .any(|generation| generation == 0 || generation == u64::MAX)
            || self.map_dimensions.into_iter().any(|n| n == 0 || n > 32768)
        {
            return Err(invalid(
                "invalid exact operator furniture source acceptance",
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
                "furniture source acceptance belongs to another fortress",
            ));
        }
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)
    }
    pub fn binding(&self, binding: &BuildBinding) -> Result<()> {
        self.validate()?;
        if binding.fortress().fortress_id() != self.fortress()
            || binding.fortress().folder() != self.world_folder
            || binding.fortress().site() != self.site_id
            || binding.generation() != self.furniture_generation
            || binding.dimensions() != self.map_dimensions
            || binding.df_version() != self.df_version
            || binding.dfhack_version() != self.dfhack_version
        {
            return Err(denied(
                "native furniture binding differs from exact operator pins",
            ));
        }
        Ok(())
    }
    pub fn capture(&self, capture: &BuildCapture) -> Result<()> {
        self.validate()?;
        if capture.fortress_id() != self.fortress()
            || capture.folder() != self.world_folder
            || capture.site() != self.site_id
            || capture.generation() != self.furniture_generation
            || capture.dimensions() != self.map_dimensions
        {
            return Err(denied(
                "native furniture capture differs from original source pins",
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
        let terrain = observed.spatial().terrain();
        if terrain.fortress_id()? != self.fortress()
            || terrain.world_folder != self.world_folder
            || terrain.site_id != self.site_id
            || terrain.bridge_generation != self.spatial_generation
            || terrain.df_version != self.df_version
            || terrain.dfhack_version != self.dfhack_version
            || terrain.map_dimensions != self.map_dimensions
            || terrain.map.region != region
            || !terrain.paused
        {
            return Err(denied(
                "complete spatial source differs from exact operator furniture pins",
            ));
        }
        Ok(())
    }
    pub fn spatial(
        &self,
        state: &LiveSpatialCitizenState,
        region: Region,
        context: &OperationContext,
    ) -> Result<EvidencePolicy> {
        self.context(context)?;
        let observed = state
            .observation_full()
            .ok_or_else(|| invalid("spatial source is absent"))?;
        let snapshot = state
            .snapshot()
            .ok_or_else(|| invalid("spatial publication is absent"))?;
        self.observation(observed, region)?;
        if snapshot.anchor() != context.anchor {
            return Err(denied(
                "spatial furniture policy requires the exact canonical publication",
            ));
        }
        let digest = state.source_digest()?;
        let mut policy = EvidencePolicy::at(snapshot.anchor());
        policy.paused = true;
        policy
            .entity_kinds
            .insert(EntityKind::Item, EvidenceCoverage::Observed);
        // These are the exact raw item facts supplied by the existing complete
        // spatial profile, not caller assertions or complete-domain absence.
        for field in [
            "item.id",
            "item.getType",
            "item.type_key",
            "item.getSubtype",
            "item.getMaterial",
            "item.getMaterialIndex",
            "item.getStackSize",
            "item.pos",
            "forbidden",
            "in_job",
            "dump",
            "removed",
            "rotten",
            "trader",
            "on_ground",
            "in_inventory",
            "in_building",
            "Items.getContainer",
            "Items.getHolderBuilding",
        ] {
            policy.sources.insert(EvidenceSource::Observed {
                field: format!("spatial/1.8.operations.{field}"),
                source_digest: digest,
            });
        }
        Ok(policy)
    }
    pub fn goal(
        &self,
        projection: &FurnitureGoalProjection,
        context: &OperationContext,
    ) -> Result<EvidencePolicy> {
        self.context(context)?;
        let sample = projection.sample();
        let raw = &projection.assessment().operations;
        if projection.anchor().fortress_id != self.fortress()
            || projection.original_anchor().fortress_id != self.fortress()
            || context.anchor.cursor.epoch != projection.anchor().cursor.epoch
            || context.anchor.tick < projection.anchor().tick
            || sample.before != sample.after
            || sample.before.generation != self.furniture_generation
            || sample.operations.generation != self.operations_generation
            || [&sample.before, &sample.operations, &sample.after]
                .iter()
                .any(|source| {
                    source.df_version != self.df_version
                        || source.dfhack_version != self.dfhack_version
                })
            || raw.jobs.world_folder != self.world_folder
            || raw.jobs.site_id != self.site_id as i32
            || raw.jobs.bridge_generation != self.operations_generation
            || !raw.jobs.paused
        {
            return Err(denied(
                "original construction goal differs from exact operator source pins",
            ));
        }
        let mut policy = EvidencePolicy::at(projection.anchor());
        policy.paused = true;
        policy
            .entity_kinds
            .insert(EntityKind::Building, EvidenceCoverage::Observed);
        // The registered derivation validates exact receipt brackets, original
        // item/footprint/stage and the complete operations roster. These input
        // identities have been independently accepted above at the authority floor.
        policy.sources = projection.evidence_sources();
        projection.evidence(policy.clone())?;
        Ok(policy)
    }
}
