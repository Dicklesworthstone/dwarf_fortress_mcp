//! Host-local review and write-scope policy, composed with the mandatory adapter guard.
use super::{
    error,
    runtime::{CheckpointPolicy, Config},
};
use dfmcp_adapter::build_placement::journal::{BuildGuard, BuildStage};
use dfmcp_adapter::build_placement::{BuildBinding, BuildPlan, BuildSelection};
use dfmcp_core::{
    Capability, Digest32, ErrorCode, GameTick, LeaseId, LeaseManager, MapCoord, MapCuboid,
    OperationContext, Result, RiskTier, SessionId, cuboids_intersect,
};

pub(super) struct Policy {
    pub config: Config,
    pub journal: Digest32,
    pub session: SessionId,
    pub lease: LeaseId,
    pub binding: BuildBinding,
    pub digest: Digest32,
}
impl Policy {
    pub fn new(
        config: Config,
        binding: BuildBinding,
        journal: Digest32,
        c: &OperationContext,
        leases: &mut LeaseManager,
    ) -> Result<Self> {
        config.matches(&binding)?;
        let lease =
            leases.acquire_spatial_lease(c.session_id, config.scope, true, c.anchor.tick, 1200)?;
        let mut bytes = b"dfmcp-build-mcp-policy/1\0".to_vec();
        bytes.extend_from_slice(&binding.canonical_bytes());
        bytes.extend_from_slice(journal.as_bytes());
        bytes.extend_from_slice(&c.session_id.get().to_be_bytes());
        bytes.extend_from_slice(&lease.get().to_be_bytes());
        bytes.extend_from_slice(config.checkpoint.as_str().as_bytes());
        for region in std::iter::once(&config.scope).chain(config.protected.iter()) {
            for v in [
                region.min.x,
                region.min.y,
                region.min.z,
                region.max.x,
                region.max.y,
                region.max.z,
            ] {
                bytes.extend_from_slice(&v.to_be_bytes());
            }
        }
        let digest = Digest32::of_bytes(&bytes);
        Ok(Self {
            config,
            journal,
            session: c.session_id,
            lease,
            binding,
            digest,
        })
    }
    pub fn seal(&self, plan: &BuildPlan) -> Digest32 {
        let mut bytes = b"dfmcp-build-mcp-review/1\0".to_vec();
        bytes.extend_from_slice(self.digest.as_bytes());
        bytes.extend_from_slice(&plan.canonical_bytes());
        Digest32::of_bytes(&bytes)
    }
    pub fn evaluate(
        &self,
        plan: &BuildPlan,
        c: &OperationContext,
        leases: &LeaseManager,
    ) -> Result<()> {
        if c.session_id != self.session
            || c.anchor.fortress_id != self.binding.fortress().fortress_id()
            || plan.before().fortress() != self.binding.fortress()
            || plan.before().generation() != self.binding.generation()
        {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "furniture review belongs to another session or source",
            ));
        }
        let halo = self.config.selection(plan.before().selection())?;
        let position = plan.before().item_position().ok_or_else(|| {
            error(
                ErrorCode::CapabilityDenied,
                "exact ground item position is required",
            )
        })?;
        let item = MapCuboid::new(
            MapCoord::new(position[0], position[1], position[2]),
            MapCoord::new(position[0], position[1], position[2]),
        )?;
        let tick = GameTick(c.anchor.tick.get().max(plan.before().tick()));
        let mut current = c.clone();
        current.anchor.tick = tick;
        for (capability, risk) in [
            (Capability::Query, RiskTier::ReadOnly),
            (Capability::Observe, RiskTier::ReadOnly),
            (Capability::Plan, RiskTier::Guarded),
            (Capability::Construct, RiskTier::Guarded),
        ] {
            current.authorize(capability, risk, &[], None)?;
        }
        for area in [halo, item] {
            leases.verify_exclusive_spatial(self.lease, self.session, area, tick)?;
            if self
                .config
                .protected
                .iter()
                .any(|protected| cuboids_intersect(protected, &area))
            {
                return Err(error(
                    ErrorCode::CapabilityDenied,
                    "furniture target or selected item intersects a protected region",
                ));
            }
        }
        if self.config.checkpoint == CheckpointPolicy::Required {
            return Err(error(
                ErrorCode::CheckpointRequired,
                "verified game checkpoint required and unavailable in this development profile",
            ));
        }
        Ok(())
    }
}
pub(super) struct Guard<'a, G> {
    pub policy: &'a Policy,
    pub leases: &'a LeaseManager,
    pub runtime: &'a mut G,
    pub confirmation: Option<Digest32>,
}
impl<G: BuildGuard> BuildGuard for Guard<'_, G> {
    fn check(
        &mut self,
        stage: BuildStage,
        binding: &BuildBinding,
        plan: Option<&BuildPlan>,
        selection: BuildSelection,
        c: &OperationContext,
    ) -> Result<()> {
        self.runtime.check(stage, binding, plan, selection, c)?;
        if binding != &self.policy.binding || c.session_id != self.policy.session {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "furniture policy binding changed",
            ));
        }
        if matches!(
            stage,
            BuildStage::Observe | BuildStage::Prepare | BuildStage::Commit
        ) {
            self.policy.config.selection(selection)?;
        }
        if matches!(stage, BuildStage::Prepare | BuildStage::Commit) {
            let plan = plan.ok_or_else(|| {
                error(
                    ErrorCode::InternalInvariantViolation,
                    "furniture effect guard requires its sealed plan",
                )
            })?;
            self.policy.evaluate(plan, c, self.leases)?;
            if stage == BuildStage::Commit && self.confirmation != Some(self.policy.seal(plan)) {
                return Err(error(
                    ErrorCode::CapabilityDenied,
                    "furniture commit requires its exact local review confirmation",
                ));
            }
        }
        Ok(())
    }
}
