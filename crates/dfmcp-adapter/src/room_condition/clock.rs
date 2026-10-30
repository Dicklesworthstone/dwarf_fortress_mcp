//! Separately reviewed finite global-clock intervals for one original room.
//!
//! The private engine shares only native run custody and authority mechanics
//! with furnishing clocks. The complete room definition and first independent
//! map, operations and furniture source identities have their own target,
//! idempotency and review domains. No ordinary monitor is promoted into a room
//! goal, and no room review claims that native excavation work exists.
use std::time::{Duration, Instant};

use dfmcp_core::{
    Capability, DfmcpError, Digest32, ErrorCode, GameTick, OperationContext, Result, RiskTier,
};

use super::{Definition, Progress, Sample, SourceIdentity, TerrainAssessment};
use crate::bounded_run::RunPlan;
use crate::bounded_run::journal::{RunBinding, RunJournal, RunMode};
use crate::bounded_run::rpc::RunSource;
use crate::construction_plan::{Manifest, clock as shared};
use crate::control_effect_journal::EffectJournalStorage;

pub use shared::{
    ClockRunView, ClockView, Edge, MAX_RUNS, Readiness, Recovery, RunRequest, VIEW_BYTES,
};

/// Whole bracket clock retention, complete joint reassessment and review bytes.
pub const READINESS_BYTES: u64 = 192 * 1024 * 1024;
/// Readiness plus the native intent/receipt journal and final exact inspection.
/// Original monitor custody and the new acquisition are prepaid by the host.
pub const OPERATION_BYTES: u64 = 256 * 1024 * 1024;

fn stale() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::StaleAnchor,
        "whole original room readiness and its first independent sources no longer agree",
    )
}
fn bounded() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::BudgetExceeded,
        "complete room clock readiness exceeds the original finite allowance",
    )
}

/// A complete new joint acquisition and the verified progress obtained after
/// its durable readiness acknowledgment. Pure reassessment may also use a
/// previous accepted progress value; it never charges an acquisition twice.
#[derive(Clone, Debug)]
pub struct RoomClockSample {
    pub progress: Progress,
    pub sample: Sample,
}
pub type ClockSample = RoomClockSample;

pub(crate) fn source_bytes(definition: &Definition, source: &SourceIdentity) -> Result<Vec<u8>> {
    let binding = definition.origin().definition().binding();
    let operations = source.furniture.operations.canonical_bytes()?;
    let map = source.map.canonical_bytes()?;
    if source.furniture.furniture_generation != binding.generation()
        || source.furniture.operations.df_version != binding.df_version()
        || source.furniture.operations.dfhack_version != binding.dfhack_version()
        || source.map.df_version != binding.df_version()
        || source.map.dfhack_version != binding.dfhack_version()
    {
        return Err(stale());
    }
    let mut bytes = b"dfmcp-room-clock-sources/1\0".to_vec();
    bytes.extend_from_slice(&source.furniture.furniture_generation.to_be_bytes());
    bytes.extend_from_slice(&(operations.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&operations);
    bytes.extend_from_slice(&(map.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&map);
    Ok(bytes)
}

pub(crate) struct Assessment {
    pub tick: u64,
    pub pending_count: u32,
    pub complete_count: u32,
    pub withdraws_stability: bool,
    pub terrain: TerrainAssessment,
}

pub(crate) fn assess(
    definition: &Definition,
    source: &SourceIdentity,
    progress: &Progress,
    sample: &Sample,
    context: &OperationContext,
) -> Result<Assessment> {
    context.budget.validate()?;
    if context.budget.max_bytes < READINESS_BYTES {
        return Err(bounded());
    }
    source_bytes(definition, source)?;
    let goal = definition.goal();
    if progress.definition_id != definition.id()
        || progress.goal_digest != goal.digest()
        || progress.terminal()
        || progress.reading
        || progress.observations == 0
        || progress.observations >= goal.timing().max_observations
        || progress.source.as_ref() != Some(source)
        || progress.construction.source.as_ref() != Some(&source.furniture)
        || sample.before != source.map
        || sample.after != source.map
        || sample.furnishings.operations != source.furniture.operations
    {
        return Err(stale());
    }
    let before = sample.map(true)?;
    let operations = sample.operations()?;
    let tick = before.tick().get().max(operations.jobs.tick().get());
    let original_tick = definition
        .origin()
        .receipts()
        .iter()
        .fold(0, |tick, record| {
            tick.max(record.plan().before().tick())
                .max(record.after().map_or(0, |after| after.tick()))
        });
    if !before.paused
        || !operations.jobs.paused
        || tick < context.anchor.tick.get()
        || tick < progress.observed_tick_floor
        || tick < original_tick
        || tick >= goal.timing().deadline
    {
        return Err(stale());
    }
    let mut current = context.clone();
    current.anchor.tick = GameTick(tick.max(current.anchor.tick.get()));
    current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    current.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
    let checked = super::readiness::probe(&progress.begin_read()?, definition, sample, &current)?;
    if checked.terminal()
        || checked.source.as_ref() != Some(source)
        || checked.terrain.unknown()
        || checked.assessments.len() != goal.records().len()
        || checked
            .assessments
            .iter()
            .any(|row| !matches!(row.condition.status, "pending" | "condition_met"))
    {
        return Err(stale());
    }
    let complete_count = checked.condition_met_count() as u32;
    Ok(Assessment {
        tick,
        pending_count: goal.records().len() as u32 - complete_count,
        complete_count,
        withdraws_stability: progress.streak > 0 && checked.streak < progress.streak,
        terrain: checked.terrain,
    })
}

/// Reassess a complete joint bracket without publishing completion, charging a
/// second acquisition, or issuing an effect permit. Known terrain deficits are
/// returned in full; hidden or missing required terrain refuses clock review.
pub fn capture_readiness(
    definition: &Definition,
    source: &SourceIdentity,
    progress: &Progress,
    sample: &Sample,
    context: &OperationContext,
) -> Result<Readiness> {
    context.budget.validate()?;
    if context.budget.max_bytes < READINESS_BYTES || context.budget.max_wall_millis > 60_000 {
        return Err(bounded());
    }
    let deadline = Instant::now()
        .checked_add(Duration::from_millis(context.budget.max_wall_millis))
        .ok_or_else(bounded)?;
    let ready = Readiness::from_room_capture(definition, source, progress, sample, context)?;
    if deadline
        .checked_duration_since(Instant::now())
        .is_none_or(|left| left.as_millis() == 0)
    {
        return Err(bounded());
    }
    Ok(ready)
}

/// Native clock custody permanently associated with an original joint room
/// goal. This wrapper exposes no conversion to the ordinary furnishing owner.
pub struct Clock<S: EffectJournalStorage> {
    inner: shared::Clock<S>,
    definition: Definition,
    source: SourceIdentity,
}
impl<S: EffectJournalStorage> Clock<S> {
    pub fn new(
        journal: RunJournal<S>,
        definition: Definition,
        source: SourceIdentity,
        recovery: Option<Recovery>,
        c: &OperationContext,
        n: &OperationContext,
    ) -> Result<Self> {
        let inner =
            shared::Clock::new_room(journal, definition.clone(), source.clone(), recovery, c, n)?;
        Ok(Self {
            inner,
            definition,
            source,
        })
    }
    pub fn spec(&self) -> &Definition {
        &self.definition
    }
    pub fn definition(&self) -> &Definition {
        &self.definition
    }
    pub fn source(&self) -> &SourceIdentity {
        &self.source
    }
    pub fn operations(&self) -> &Manifest {
        &self.source.furniture.operations
    }
    pub fn binding(&self) -> &RunBinding {
        self.inner.binding()
    }
    pub fn recovery(&self) -> Recovery {
        self.inner.recovery()
    }
    pub fn high_tick(&self) -> u64 {
        self.inner.high_tick()
    }
    pub fn high_sequence(&self) -> u64 {
        self.inner.high_sequence()
    }
    pub fn mode(&self) -> RunMode {
        self.inner.mode()
    }
    pub fn is_control(&self) -> bool {
        self.inner.is_control()
    }
    pub fn prepared_seal(&self) -> Option<Digest32> {
        self.inner.prepared_seal()
    }
    pub fn abandon_preparation(&mut self) {
        self.inner.abandon_preparation();
    }
    pub fn cached_view(&self) -> ClockView {
        self.inner.cached_view()
    }
    pub fn authorize_disclosure(&self, c: &OperationContext, n: &OperationContext) -> Result<()> {
        self.inner.authorize_disclosure(c, n)
    }
    pub fn inspect(&mut self, c: &OperationContext, n: &OperationContext) -> Result<ClockView> {
        self.inner.inspect(c, n)
    }
    pub fn prepare_guarded<N, F>(
        &mut self,
        source: &mut N,
        request: &RunRequest,
        c: &OperationContext,
        n: &OperationContext,
        guard: F,
    ) -> Result<ClockView>
    where
        N: RunSource,
        F: FnMut(Edge, &RunPlan, &OperationContext) -> Result<RoomClockSample>,
    {
        self.inner
            .prepare_room_guarded(source, request, c, n, guard)
    }
    pub fn commit_guarded<N, F>(
        &mut self,
        source: &mut N,
        seal: Digest32,
        confirmed: bool,
        c: &OperationContext,
        n: &OperationContext,
        guard: F,
    ) -> Result<ClockView>
    where
        N: RunSource,
        F: FnMut(Edge, &RunPlan, &OperationContext) -> Result<RoomClockSample>,
    {
        self.inner
            .commit_room_guarded(source, seal, confirmed, c, n, guard)
    }
    pub fn reconcile<N: RunSource>(
        &mut self,
        source: &mut N,
        c: &OperationContext,
        n: &OperationContext,
    ) -> Result<ClockView> {
        self.inner.reconcile(source, c, n)
    }
    pub fn cancel(
        &mut self,
        source: Option<&mut dyn RunSource>,
        c: &OperationContext,
        n: &OperationContext,
    ) -> Result<ClockView> {
        self.inner.cancel(source, c, n)
    }
}

#[cfg(test)]
mod tests;
