//! Explicit operator acceptance of a small development source contract.
//! This is not compatibility-registry admission and cannot be configured by MCP.
use super::{denied, invalid};
use dfmcp_adapter::live_projection::LiveWorldProjection;
use dfmcp_adapter::semantic_work_orders::WorkOrderGoalProjection;
use dfmcp_adapter::semantic_work_orders::goal_projection::{
    ORDER_COUNTER_SOURCE, ORDER_STATUS_SOURCE,
};
use dfmcp_adapter::work_orders::rpc::WorkOrderManifest;
use dfmcp_adapter::{BridgeManifest, LiveObservationCapsule};
use dfmcp_core::{Capability, FortressId, OperationContext, Result, RiskTier};
use dfmcp_world::{EntityKind, EvidenceCoverage, EvidencePolicy, EvidenceSource};
use serde::Deserialize;

pub const ACCEPTANCE_KIND: &str = "operator-development-source-acceptance/1";

/// Exact operator pins are a development trust decision, not a claim that the
/// checked-in compatibility registry supports this tuple. Incarnations belong
/// to four different plugin namespaces and are never compared to one another.
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
    pub work_order_generation: u64,
    pub progress_generation: u64,
    pub operations_generation: u64,
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
            .any(|s| s.is_empty() || s.len() > 64 || s.chars().any(char::is_control))
            || [
                self.citizen_generation,
                self.work_order_generation,
                self.progress_generation,
                self.operations_generation,
            ]
            .iter()
            .any(|g| *g == 0 || *g == u64::MAX)
        {
            return Err(invalid(
                "invalid exact operator development source acceptance",
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
                "development source acceptance belongs to another fortress",
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
                "citizen source differs from operator development pins",
            ));
        }
        Ok(())
    }
    pub fn native_manifest(&self, manifest: &WorkOrderManifest) -> Result<()> {
        if manifest.generation != self.work_order_generation
            || manifest.df_version != self.df_version
            || manifest.dfhack_version != self.dfhack_version
        {
            return Err(denied(
                "work-order source differs from operator development pins",
            ));
        }
        Ok(())
    }
    pub fn citizens(
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
            || projection.snapshot.anchor() != context.anchor
            || projection.snapshot.fortress_id != self.fortress()
        {
            return Err(denied(
                "canonical citizen observation differs from exact operator source scope",
            ));
        }
        let mut policy = EvidencePolicy::at(projection.snapshot.anchor());
        policy.paused = true;
        // Positive identity only. No global absence, labor, item, terrain or
        // arbitrary observed field is accepted by this development policy.
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
        projection: &WorkOrderGoalProjection,
        context: &OperationContext,
    ) -> Result<EvidencePolicy> {
        self.context(context)?;
        let operations = projection.operations();
        operations.validate()?;
        let progress = projection.progress_capture();
        let before = projection.progress_before_capture();
        let manifest = projection.progress_manifest();
        let queue = projection.queue_capture();
        if context.anchor.fortress_id != projection.anchor().fortress_id
            || context.anchor.cursor.epoch != projection.anchor().cursor.epoch
            || context.anchor.tick < projection.anchor().tick
            || operations.jobs.world_folder != self.world_folder
            || operations.jobs.site_id != self.site_id as i32
            || operations.jobs.df_version != self.df_version
            || operations.jobs.dfhack_version != self.dfhack_version
            || operations.jobs.bridge_generation != self.operations_generation
            || manifest.generation != self.progress_generation
            || manifest.df_version != self.df_version
            || manifest.dfhack_version != self.dfhack_version
            || before.generation() != self.progress_generation
            || progress.generation() != self.progress_generation
            || queue.generation() != self.work_order_generation
            || queue.world_folder() != self.world_folder
            || queue.site_id() != self.site_id
            || !operations.jobs.paused
            || !progress.paused()
            || !before.paused()
            || !queue.paused()
        {
            return Err(denied(
                "original-goal sources differ from exact operator development pins",
            ));
        }
        let mut policy = EvidencePolicy::at(projection.anchor());
        policy.paused = true;
        policy
            .entity_kinds
            .insert(EntityKind::Fortress, EvidenceCoverage::Observed);
        policy
            .entity_kinds
            .insert(EntityKind::Job, EvidenceCoverage::Complete);
        policy
            .entity_kinds
            .insert(EntityKind::WorkOrder, EvidenceCoverage::Observed);
        policy.sources.insert(EvidenceSource::Observed {
            field: "operations/1.3.job_count".into(),
            source_digest: operations.source_digest()?,
        });
        // The closed projection links this row only to an exact Created receipt.
        // These source names are fixed here; the projection cannot nominate any
        // additional trusted field or claim a complete WorkOrder domain.
        if projection.receipt_linked() {
            for field in [ORDER_COUNTER_SOURCE, ORDER_STATUS_SOURCE] {
                policy.sources.insert(EvidenceSource::Observed {
                    field: field.into(),
                    source_digest: progress.witness(),
                });
            }
        }
        projection.evidence(policy.clone())?;
        Ok(policy)
    }
}
