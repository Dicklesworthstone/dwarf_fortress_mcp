//! Host mining policy around the existing coordinator, never native authority.
//!
//! The safe default requires a real game checkpoint and therefore refuses new
//! designation: this profile has no game-save verifier. An operator may instead
//! explicitly select a disposable-fortress development policy. Neither mode
//! claims structural safety, a global controller lease, or production admission.
use crate::bounded_run::{error, hash};
use crate::dig_designation::journal::session::DigSessionGuard;
use crate::dig_designation::journal::{DigBinding, DigGuard, DigStage};
use crate::dig_designation::rpc::authorize;
use crate::dig_designation::{DigPlan, DigRegion};
use dfmcp_core::{
    Digest32, ErrorCode, GameTick, LeaseId, LeaseManager, MapCuboid, OperationContext, Result,
    SessionId, cuboids_intersect,
};

fn append_text(bytes: &mut Vec<u8>, value: &str) {
    // Every source string is validated by DigBinding before this serializer.
    bytes.extend_from_slice(&(value.len() as u16).to_be_bytes());
    bytes.extend_from_slice(value.as_bytes());
}

pub const MAX_PROTECTED_AREAS: usize = 32;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DigCheckpointPolicy {
    #[default]
    Required,
    /// Only an explicit, trusted operator policy may choose this. No checkpoint
    /// or restore capability is implied, and the tool caller cannot override it.
    DisposableFortress,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DigControlPolicy {
    binding: DigBinding,
    journal: Digest32,
    session: SessionId,
    lease: LeaseId,
    protected: Vec<MapCuboid>,
    checkpoint: DigCheckpointPolicy,
    digest: Digest32,
}
fn coordinates(area: MapCuboid) -> [i32; 6] {
    [
        area.min.x, area.min.y, area.min.z, area.max.x, area.max.y, area.max.z,
    ]
}
fn valid_area(area: MapCuboid) -> Result<()> {
    MapCuboid::new(area.min, area.max)?;
    if coordinates(area).iter().any(|n| !(0..=32767).contains(n)) {
        return Err(error(
            ErrorCode::InvalidRequest,
            "protected region exceeds native map bounds",
        ));
    }
    Ok(())
}
fn append_area(bytes: &mut Vec<u8>, area: MapCuboid) {
    for value in coordinates(area) {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
}
impl DigControlPolicy {
    pub fn new(
        binding: DigBinding,
        journal: Digest32,
        session: SessionId,
        lease: LeaseId,
        mut protected: Vec<MapCuboid>,
        checkpoint: DigCheckpointPolicy,
    ) -> Result<Self> {
        if session == SessionId::NIL
            || lease == LeaseId::NIL
            || journal == Digest32::ZERO
            || protected.len() > MAX_PROTECTED_AREAS
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "invalid mining policy identity or protected-region bound",
            ));
        }
        for area in &protected {
            valid_area(*area)?;
        }
        protected.sort_by_key(|area| coordinates(*area));
        protected.dedup();
        let mut bytes = Vec::new();
        append_text(&mut bytes, &binding.endpoint().to_string());
        bytes.extend_from_slice(&binding.manifest().generation.to_be_bytes());
        append_text(&mut bytes, &binding.manifest().df_version);
        append_text(&mut bytes, &binding.manifest().dfhack_version);
        append_text(&mut bytes, binding.folder());
        bytes.extend_from_slice(&binding.site().to_be_bytes());
        append_area(&mut bytes, binding.scope());
        bytes.extend_from_slice(journal.as_bytes());
        bytes.extend_from_slice(&session.get().to_be_bytes());
        bytes.extend_from_slice(&lease.get().to_be_bytes());
        bytes.push(match checkpoint {
            DigCheckpointPolicy::Required => 0,
            DigCheckpointPolicy::DisposableFortress => 1,
        });
        bytes.extend_from_slice(&(protected.len() as u16).to_be_bytes());
        for area in &protected {
            append_area(&mut bytes, *area);
        }
        let digest = hash(b"dfmcp-dig-control-policy/1", &bytes);
        Ok(Self {
            binding,
            journal,
            session,
            lease,
            protected,
            checkpoint,
            digest,
        })
    }
    pub fn digest(&self) -> Digest32 {
        self.digest
    }
    pub fn journal_id(&self) -> Digest32 {
        self.journal
    }
    pub fn lease_id(&self) -> LeaseId {
        self.lease
    }
    pub fn checkpoint_policy(&self) -> DigCheckpointPolicy {
        self.checkpoint
    }
    pub fn protected_areas(&self) -> &[MapCuboid] {
        &self.protected
    }
    pub fn binding(&self) -> &DigBinding {
        &self.binding
    }

    fn scope(&self, context: &OperationContext, region: DigRegion) -> Result<()> {
        if context.session_id != self.session
            || context.anchor.fortress_id != self.binding.fortress_id()
            || !self.binding.scope().contains_cuboid(region.halo())
            || !self.binding.scope().contains_cuboid(region.write_area())
        {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "mining policy belongs to another session, fortress or scope",
            ));
        }
        Ok(())
    }
    fn source(&self, plan: &DigPlan, context: &OperationContext) -> Result<()> {
        self.scope(context, plan.before().region())?;
        if plan.before().generation() != self.binding.manifest().generation
            || plan.before().folder() != self.binding.folder()
            || plan.before().site() != self.binding.site()
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "mining review belongs to a different source",
            ));
        }
        Ok(())
    }
    /// Evaluate current authority, the manager's actual lease and protected
    /// shared blocks. A retained digest or client boolean cannot replace these.
    pub fn evaluate(
        &self,
        plan: &DigPlan,
        context: &OperationContext,
        leases: &LeaseManager,
    ) -> Result<()> {
        self.source(plan, context)?;
        authorize(
            context,
            self.binding.fortress_id(),
            plan.before().tick(),
            plan.before().region(),
            true,
            true,
            true,
        )?;
        leases.verify_exclusive_spatial(
            self.lease,
            self.session,
            plan.before().region().write_area(),
            GameTick(context.anchor.tick.get().max(plan.before().tick())),
        )?;
        if self
            .protected
            .iter()
            .any(|area| cuboids_intersect(area, &plan.before().region().write_area()))
        {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "mining scheduling writes intersect a protected region",
            ));
        }
        if self.checkpoint == DigCheckpointPolicy::Required {
            return Err(error(
                ErrorCode::CheckpointRequired,
                "a verified game checkpoint is required; this development profile cannot supply one",
            ));
        }
        Ok(())
    }
    fn seal(&self, plan: &DigPlan) -> Digest32 {
        let mut bytes = self.digest.as_bytes().to_vec();
        bytes.extend_from_slice(&plan.canonical_bytes());
        hash(b"dfmcp-dig-control-review/1", &bytes)
    }
    /// The caller must retain this non-cloneable review only for its own fresh
    /// preparation. The checksum binds review content; it is not a signature or
    /// evidence that a human actually reviewed the plan.
    pub fn review(
        &self,
        plan: &DigPlan,
        context: &OperationContext,
        leases: &LeaseManager,
    ) -> Result<DigReview> {
        self.evaluate(plan, context, leases)?;
        Ok(DigReview {
            seal: self.seal(plan),
        })
    }
}

#[derive(Debug)]
pub struct DigReview {
    seal: Digest32,
}
impl DigReview {
    pub fn seal(&self) -> Digest32 {
        self.seal
    }
    pub fn confirm(self, supplied: Digest32) -> Result<ConfirmedDigReview> {
        if supplied != self.seal {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "exact mining review seal was not confirmed",
            ));
        }
        Ok(ConfirmedDigReview { seal: self.seal })
    }
}
#[derive(Debug)]
pub struct ConfirmedDigReview {
    seal: Digest32,
}

/// The runtime remains mandatory and is checked first, including after journal
/// synchronization. The lease book is borrowed, never reconstructed from input.
pub struct PolicyDigGuard<'a, G> {
    policy: &'a DigControlPolicy,
    leases: &'a LeaseManager,
    runtime: &'a mut G,
    confirmation: Option<&'a ConfirmedDigReview>,
}
impl<'a, G: DigSessionGuard> PolicyDigGuard<'a, G> {
    pub fn new(
        policy: &'a DigControlPolicy,
        leases: &'a LeaseManager,
        runtime: &'a mut G,
        confirmation: Option<&'a ConfirmedDigReview>,
    ) -> Self {
        Self {
            policy,
            leases,
            runtime,
            confirmation,
        }
    }
}
impl<G: DigSessionGuard> DigGuard for PolicyDigGuard<'_, G> {
    fn check(&mut self, stage: DigStage, plan: &DigPlan, context: &OperationContext) -> Result<()> {
        self.runtime.check(stage, plan, context)?;
        self.policy.source(plan, context)?;
        if matches!(stage, DigStage::Prepare | DigStage::Commit) {
            self.policy.evaluate(plan, context, self.leases)?;
        }
        if stage == DigStage::Commit
            && !self
                .confirmation
                .is_some_and(|review| review.seal == self.policy.seal(plan))
        {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "mining commit has no current exact reviewed confirmation",
            ));
        }
        // Query/cancellation must remain possible after lease expiry or an
        // unavailable checkpoint. They cannot initiate a new designation.
        Ok(())
    }
}
impl<G: DigSessionGuard> DigSessionGuard for PolicyDigGuard<'_, G> {
    fn connect(
        &mut self,
        binding: &DigBinding,
        region: DigRegion,
        context: &OperationContext,
    ) -> Result<()> {
        self.runtime.connect(binding, region, context)?;
        if binding != self.policy.binding() {
            return Err(error(
                ErrorCode::StaleAnchor,
                "mining policy source changed",
            ));
        }
        self.policy.scope(context, region)
    }
    fn observe(
        &mut self,
        binding: &DigBinding,
        region: DigRegion,
        context: &OperationContext,
    ) -> Result<()> {
        self.runtime.observe(binding, region, context)?;
        if binding != self.policy.binding() {
            return Err(error(
                ErrorCode::StaleAnchor,
                "mining observation policy source changed",
            ));
        }
        self.policy.scope(context, region)
    }
}

#[cfg(test)]
mod tests;
