//! Whole-selection receipt-linked furniture conditions over one complete capture.
//!
//! All original placements share one observation and one advancing-game-tick
//! stability streak. Receipt bytes are evidence, never dispatch authority, a
//! canonical world anchor, effect discharge, or proof of current usability.
//! Beads: df-dfhack-bridge-plane-c-pic.4/.5, df-action-coordinator-exec-ero.4.

pub mod origin;
pub mod rpc;
pub mod store;
mod wire;

use crate::build_placement::{BuildItem, BuildKind, BuildRecord};
use crate::live_operations::LiveOperationsObservation;
use dfmcp_core::{
    Capability, DfmcpError, Digest32, ErrorCode, GameTick, OperationContext, Result, RiskTier,
};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

pub const POLICY: &str = "dfmcp.receipt-construction-plan-condition/1";
pub const MAX_TARGETS: usize = 32;
pub const MAX_CAPTURE: usize = 16 * 1024 * 1024;
pub const MAX_OBSERVATIONS: u32 = 512;
pub const MAX_GOAL: usize = MAX_TARGETS * (crate::build_placement::MAX_RECORD_BYTES + 2) + 128;
pub const MAX_SAMPLE: usize =
    MAX_CAPTURE + 2 * MAX_TARGETS * (crate::build_placement::MAX_RECORD_BYTES + 2) + 4096;
pub const MAX_WORK: u64 = 20_000_000;

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::AdapterRejected, message)
}
fn bounded(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}
fn require(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(invalid(message))
    }
}

/// Fixed original game deadline and sampled-stability policy. Reopening never
/// renews these fields or the maximum number of completed acquisitions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    pub deadline: u64,
    pub interval: u32,
    pub stable_samples: u32,
    pub stable_span: u64,
    pub max_gap: u32,
    pub max_observations: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Goal {
    records: Vec<BuildRecord>,
    timing: Timing,
    canonical: Vec<u8>,
    digest: Digest32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub generation: u64,
    pub df_version: String,
    pub dfhack_version: String,
}

/// A fixed query transport must establish the same-connection bracket. Decoding
/// this serialized evidence alone makes no claim about contacting a native peer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedSample {
    pub before: Manifest,
    pub before_records: Vec<Vec<u8>>,
    pub operations: Manifest,
    pub capture: Vec<u8>,
    pub after: Manifest,
    pub after_records: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Condition {
    pub status: &'static str,
    pub stage: Option<i32>,
    pub max_stage: u32,
    pub building_type: Option<i32>,
    pub construction_jobs: u32,
    pub removal_jobs: u32,
    pub suspended_jobs: u32,
    pub item_job_links: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assessment {
    pub building_id: u32,
    pub item_id: u32,
    pub job_id: u32,
    pub key: String,
    pub receipt_digest: Digest32,
    pub condition: Condition,
}

/// The two independently numbered plugin generations must remain distinct.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceIdentity {
    pub furniture_generation: u64,
    pub operations: Manifest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    pub goal_digest: Digest32,
    pub phase: &'static str,
    pub reason: &'static str,
    pub reason_building: Option<u32>,
    pub observations: u32,
    pub streak: u32,
    pub first_tick: Option<u64>,
    pub counted_tick: Option<u64>,
    pub last_tick: Option<u64>,
    pub last_capture: Option<Digest32>,
    pub source: Option<SourceIdentity>,
    /// Native allocation horizons, in job, building, item order.
    pub horizons: Option<[u32; 3]>,
    pub reading: bool,
    pub interruptions: u32,
    pub assessments: Vec<Assessment>,
}

impl Progress {
    pub fn new(goal: &Goal) -> Self {
        Self {
            goal_digest: goal.digest(),
            phase: "active",
            reason: "not_sampled",
            reason_building: None,
            observations: 0,
            streak: 0,
            first_tick: None,
            counted_tick: None,
            last_tick: None,
            last_capture: None,
            source: None,
            horizons: None,
            reading: false,
            interruptions: 0,
            assessments: Vec::new(),
        }
    }
    pub fn terminal(&self) -> bool {
        matches!(
            self.phase,
            "satisfied" | "failed" | "invalidated" | "expired" | "cancelled"
        )
    }
    pub fn condition_met_count(&self) -> usize {
        self.assessments
            .iter()
            .filter(|row| row.condition.status == "condition_met")
            .count()
    }
    pub fn begin_read(&self) -> Result<Self> {
        begin_read(self)
    }
    pub fn cancel(&self) -> Self {
        cancel(self)
    }
    pub fn advance(
        &self,
        goal: &Goal,
        sample: &LinkedSample,
        context: &OperationContext,
    ) -> Result<Self> {
        advance(self, goal, sample, context)
    }
    fn stop(&mut self, phase: &'static str, reason: &'static str, building: Option<u32>) {
        self.phase = phase;
        self.reason = reason;
        self.reason_building = building;
        self.streak = 0;
        self.first_tick = None;
        self.counted_tick = None;
    }
}

/// Durable storage must synchronize this transition before making native reads.
pub fn begin_read(state: &Progress) -> Result<Progress> {
    require(
        !state.terminal(),
        "terminal construction plan cannot acquire observations",
    )?;
    let mut next = state.clone();
    if next.reading {
        next.interruptions = next
            .interruptions
            .checked_add(1)
            .ok_or_else(|| bounded("construction interrupted-read count overflow"))?;
        next.streak = 0;
        next.first_tick = None;
        next.counted_tick = None;
        next.reason = "interrupted_read";
        next.reason_building = None;
    }
    next.reading = true;
    Ok(next)
}

/// Stops only monitoring. It neither cancels native jobs nor discharges effects.
pub fn cancel(state: &Progress) -> Progress {
    let mut next = state.clone();
    if !next.terminal() {
        next.stop("cancelled", "monitor_cancelled_only", None);
        next.reading = false;
    }
    next
}

struct Work {
    context: OperationContext,
    start: Instant,
    used: u64,
}
impl Work {
    fn new(goal: &Goal, context: &OperationContext, floor: u64) -> Result<Self> {
        let original = goal
            .records
            .first()
            .ok_or_else(|| invalid("empty construction goal"))?;
        if context.anchor.fortress_id != original.plan().before().fortress_id() {
            return Err(DfmcpError::new(
                ErrorCode::CapabilityDenied,
                "construction context names another fortress",
            ));
        }
        let mut current = context.clone();
        let original_floor = goal
            .records
            .iter()
            .map(|record| record.plan().before().tick())
            .max()
            .unwrap_or(0);
        current.anchor.tick = GameTick(current.anchor.tick.get().max(floor).max(original_floor));
        let mut work = Self {
            context: current,
            start: Instant::now(),
            used: 0,
        };
        work.charge(1)?;
        Ok(work)
    }
    fn charge(&mut self, count: u64) -> Result<()> {
        self.used = self
            .used
            .checked_add(count)
            .ok_or_else(|| bounded("construction work overflow"))?;
        if self.used > MAX_WORK
            || self.start.elapsed().as_millis() >= u128::from(self.context.budget.max_wall_millis)
        {
            return Err(bounded(
                "construction foreground work or wall allowance exhausted",
            ));
        }
        self.context
            .authorize(Capability::Query, RiskTier::ReadOnly, &[], None)
    }
    fn observe_tick(&mut self, tick: u64) -> Result<()> {
        self.context.anchor.tick = GameTick(self.context.anchor.tick.get().max(tick));
        self.charge(1)
    }
}

/// Bounded pure assessment using one complete decoded operations roster. The
/// caller cannot obtain a positive result from a missing construction job alone.
fn assess(
    record: &BuildRecord,
    observed: &LiveOperationsObservation,
    work: &mut Work,
) -> Result<Condition> {
    work.charge(1)?;
    let p = record
        .insertion()
        .ok_or_else(|| invalid("missing placed insertion"))?;
    let before = record.plan().before();
    let after = record
        .after()
        .ok_or_else(|| invalid("missing placed after capture"))?;
    let building = observed
        .buildings
        .binary_search_by_key(&p.building_id(), |v| v.native_id)
        .ok()
        .map(|index| &observed.buildings[index]);
    let item = observed
        .items
        .binary_search_by_key(&p.item_id(), |v| v.native_id)
        .ok()
        .map(|index| &observed.items[index]);
    let mut outcome = Condition {
        status: "pending",
        stage: building.map(|v| v.build_stage),
        max_stage: p.max_stage(),
        building_type: building.map(|v| v.building_type),
        construction_jobs: 0,
        removal_jobs: 0,
        suspended_jobs: 0,
        item_job_links: 0,
    };
    let early = if observed.jobs.site_id != before.site() as i32
        || observed.jobs.world_folder != before.folder()
    {
        Some("world_identity_mismatch")
    } else if observed.jobs.tick().get() < before.tick()
        || observed.jobs.next_job_id < after.next_job_id()
        || observed.next_building_id < after.next_building_id()
    {
        Some("source_regressed")
    } else if building.is_none() {
        Some("building_missing")
    } else {
        None
    };
    if let Some(reason) = early {
        outcome.status = reason;
        return Ok(outcome);
    }
    let building = building.ok_or_else(|| invalid("missing assessed building"))?;
    let [x, y, z] = p.position().map(|v| v as i32);
    let (building_kind, item_kind) = match p.kind() {
        BuildKind::Bed => ("Bed", "BED"),
        BuildKind::Chair => ("Chair", "CHAIR"),
        BuildKind::Table => ("Table", "TABLE"),
    };
    if building.type_key != building_kind
        || [
            building.x1,
            building.y1,
            building.x2,
            building.y2,
            building.z,
        ] != [x, y, x, y, z]
        || building.max_build_stage != p.max_stage() as i32
    {
        outcome.status = "building_identity_mismatch";
        return Ok(outcome);
    }
    let Some(item) = item else {
        outcome.status = "item_missing";
        return Ok(outcome);
    };
    let BuildItem::Visible(expected) = before.item() else {
        return Err(invalid("placed item was not visible"));
    };
    if item.item_type != expected.native_type() as i32
        || item.type_key != item_kind
        || item.subtype != expected.subtype()
        || item.material_type != expected.material()
        || item.material_index != expected.material_index()
    {
        outcome.status = "item_identity_mismatch";
        return Ok(outcome);
    }
    for job in &observed.jobs.jobs {
        work.charge(1)?;
        if job.holder_native_id == Some(p.building_id()) {
            if job.type_key == "ConstructBuilding" {
                outcome.construction_jobs += 1;
                outcome.suspended_jobs += u32::from(job.suspended);
            } else if job.type_key == "DestroyBuilding" {
                outcome.removal_jobs += 1;
            }
        }
    }
    let mut linked = BTreeSet::new();
    for attachment in &observed.attachments {
        work.charge(1)?;
        if attachment.item_native_id == p.item_id() {
            linked.insert(attachment.job_native_id);
        }
    }
    outcome.item_job_links = linked.len() as u32;
    let original_job = observed
        .jobs
        .jobs
        .binary_search_by_key(&p.job_id(), |v| v.native_id)
        .ok()
        .map(|index| &observed.jobs.jobs[index]);
    outcome.status = if outcome.removal_jobs > 0 {
        "removal_pending"
    } else if original_job.is_some_and(|job| {
        job.type_key != "ConstructBuilding" || job.holder_native_id != Some(p.building_id())
    }) {
        "original_job_identity_mismatch"
    } else if outcome.construction_jobs > 0 {
        if outcome.construction_jobs == outcome.suspended_jobs {
            "suspended"
        } else {
            "pending"
        }
    } else if building.build_stage != p.max_stage() as i32 {
        "no_construction_job"
    } else if item.holder_building_native_id != Some(p.building_id())
        || item.container_native_id.is_some()
        || item.flags & (1 << 8) == 0
        || item.flags & ((1 << 1) | (1 << 3) | (1 << 6) | (1 << 7)) != 0
        || item.stack_size != 1
        || !linked.is_empty()
    {
        "item_unverified"
    } else {
        "condition_met"
    };
    Ok(outcome)
}

/// Replay one fully bracketed acquisition. A false/unknown member clears the
/// whole-plan streak; independently timed per-building successes never latch.
pub fn advance(
    state: &Progress,
    goal: &Goal,
    sample: &LinkedSample,
    context: &OperationContext,
) -> Result<Progress> {
    let mut used = 0;
    advance_with_counter(state, goal, sample, context, &mut used)
}

/// Journal replay retains one semantic-work allowance across every frame. The
/// counter is updated even when a sample is rejected, so callers cannot renew
/// work by retrying an invalid frame. Byte and wall allowances remain separate.
pub(super) fn advance_with_counter(
    state: &Progress,
    goal: &Goal,
    sample: &LinkedSample,
    context: &OperationContext,
    used: &mut u64,
) -> Result<Progress> {
    let mut work = Work::new(goal, context, state.last_tick.unwrap_or(0))?;
    work.used = used
        .checked_add(work.used)
        .ok_or_else(|| bounded("construction work overflow"))?;
    let result = work
        .charge(0)
        .and_then(|()| advance_work(state, goal, sample, &mut work));
    *used = work.used;
    result
}

fn advance_work(
    state: &Progress,
    goal: &Goal,
    sample: &LinkedSample,
    work: &mut Work,
) -> Result<Progress> {
    require(
        state.goal_digest == goal.digest() && state.reading && !state.terminal(),
        "invalid construction plan transition",
    )?;
    require(
        state.observations < goal.timing.max_observations && state.assessments.len() <= MAX_TARGETS,
        "construction progress exceeds its original allowance",
    )?;
    let observed = sample.validate_work(goal, work)?;
    let tick = observed.jobs.tick().get();
    let capture = Digest32::of_bytes(&sample.capture);
    work.charge(1)?;
    let mut findings = Vec::with_capacity(goal.records.len());
    for record in &goal.records {
        work.charge(1)?;
        let p = record
            .insertion()
            .ok_or_else(|| invalid("missing placed insertion"))?;
        findings.push(Assessment {
            building_id: p.building_id(),
            item_id: p.item_id(),
            job_id: p.job_id(),
            key: record.plan().key().to_owned(),
            receipt_digest: record.receipt(),
            condition: assess(record, &observed, work)?,
        });
    }
    work.charge(1)?;
    let source = SourceIdentity {
        furniture_generation: sample.before.generation,
        operations: sample.operations.clone(),
    };
    let horizons = [
        observed.jobs.next_job_id,
        observed.next_building_id,
        observed.next_item_id,
    ];
    let mut current = state.clone();
    current.reading = false;
    current.observations += 1;
    current.last_tick = Some(tick);
    current.last_capture = Some(capture);
    current.source = Some(source.clone());
    current.horizons = Some(horizons);
    current.assessments = findings;
    current.reason_building = None;
    for row in &current.assessments {
        if matches!(
            row.condition.status,
            "world_identity_mismatch"
                | "source_regressed"
                | "building_missing"
                | "item_missing"
                | "building_identity_mismatch"
                | "item_identity_mismatch"
                | "original_job_identity_mismatch"
        ) {
            let (reason, id) = (row.condition.status, row.building_id);
            current.stop("invalidated", reason, Some(id));
            return Ok(current);
        }
    }
    let reason = if state.source.as_ref().is_some_and(|old| old != &source) {
        Some("native_source_changed")
    } else if state
        .horizons
        .is_some_and(|old| horizons.iter().zip(old).any(|(a, b)| *a < b))
    {
        Some("native_horizon_regressed")
    } else if state.last_tick.is_some_and(|old| tick < old) {
        Some("game_clock_regressed")
    } else {
        None
    };
    if let Some(reason) = reason {
        current.stop("invalidated", reason, None);
        return Ok(current);
    }
    let prior: BTreeMap<_, _> = state
        .assessments
        .iter()
        .map(|row| (row.building_id, &row.condition))
        .collect();
    for row in &current.assessments {
        work.charge(1)?;
        if let Some(old) = prior.get(&row.building_id) {
            let reason = if old.building_type.is_some()
                && old.building_type != row.condition.building_type
            {
                Some("native_building_type_changed")
            } else if old
                .stage
                .is_some_and(|stage| row.condition.stage.is_none_or(|value| value < stage))
            {
                Some("construction_stage_regressed")
            } else {
                None
            };
            if let Some(reason) = reason {
                let id = row.building_id;
                current.stop("invalidated", reason, Some(id));
                return Ok(current);
            }
        }
    }
    if let Some(row) = current
        .assessments
        .iter()
        .find(|row| row.condition.status == "removal_pending")
    {
        let id = row.building_id;
        current.stop("failed", "removal_pending", Some(id));
        return Ok(current);
    }
    if tick >= goal.timing.deadline {
        current.stop("expired", "game_deadline_reached", None);
        return Ok(current);
    }
    if let Some(row) = current
        .assessments
        .iter()
        .find(|row| row.condition.status != "condition_met")
    {
        let (reason, id) = (row.condition.status, row.building_id);
        current.stop("active", reason, Some(id));
    } else {
        if state.last_tick.is_some_and(|old| {
            tick - old > u64::from(goal.timing.max_gap)
                || (tick == old && state.last_capture != Some(capture))
        }) {
            current.stop("active", "observation_gap_or_same_tick_change", None);
        }
        if state.last_tick != Some(tick)
            && current
                .counted_tick
                .is_none_or(|old| tick.saturating_sub(old) >= u64::from(goal.timing.interval))
        {
            current.phase = "candidate";
            current.reason = "awaiting_stability";
            current.first_tick = Some(current.first_tick.unwrap_or(tick));
            current.counted_tick = Some(tick);
            current.streak += 1;
        }
        if current.streak >= goal.timing.stable_samples
            && current
                .first_tick
                .is_some_and(|first| tick.saturating_sub(first) >= goal.timing.stable_span)
            && current.counted_tick == Some(tick)
        {
            current.phase = "satisfied";
            current.reason = "receipt_linked_plan_sampled_condition";
            work.charge(1)?;
            return Ok(current);
        }
    }
    if current.observations >= goal.timing.max_observations {
        current.stop("expired", "sample_budget_exhausted", None);
    }
    work.charge(1)?;
    Ok(current)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) mod fixtures;
