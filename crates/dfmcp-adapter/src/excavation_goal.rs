//! Sampled terrain goals, separate from designation receipts and mutation authority.
//!
//! This is the bounded evaluator used by the mining progress-journal projection.
//! It accepts only the existing typed map/1.5 observation. It performs no I/O,
//! does not advance the game, and cannot settle a native dig-effect obligation.
use dfmcp_core::{DfmcpError, ErrorCode, Result};
use dfmcp_world::map_region::{Cell, Region, Shape};

use crate::live_map::{LiveMapObservation, map_error};

pub const MAX_GAME_TICK: u64 = u32::MAX as u64 * 403_200 + 403_199;
pub const MAX_SAMPLES: u32 = 129; // Baseline plus 128 explicit read attempts.

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::AdapterRejected, message)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FloorGoal {
    region: Region,
    folder: String,
    site: u32,
    deadline: u64,
    stable_ticks: u64,
    required_samples: u32,
    max_gap_ticks: u64,
}

impl FloorGoal {
    pub fn new(region: Region, folder: String, site: u32, deadline: u64,
        stable_ticks: u64, required_samples: u32, max_gap_ticks: u64) -> Result<Self>
    {
        region.volume().map_err(map_error)?;
        if region.size[2] != 1 || region.size[0] > 8 || region.size[1] > 8
            || folder.is_empty() || folder.len() > 512 || folder.contains('\0')
            || site > i32::MAX as u32 || deadline > MAX_GAME_TICK
            || stable_ticks > 403_200 || !(1..=128).contains(&required_samples)
            || !(1..=403_200).contains(&max_gap_ticks)
        {
            return Err(invalid("invalid bounded floor goal"));
        }
        Ok(Self { region, folder, site, deadline, stable_ticks, required_samples, max_gap_ticks })
    }

    pub fn region(&self) -> Region { self.region }
    pub fn folder(&self) -> &str { &self.folder }
    pub fn site(&self) -> u32 { self.site }
    pub fn deadline(&self) -> u64 { self.deadline }
    pub fn stable_ticks(&self) -> u64 { self.stable_ticks }
    pub fn required_samples(&self) -> u32 { self.required_samples }
    pub fn max_gap_ticks(&self) -> u64 { self.max_gap_ticks }

    /// All capture fields are validated even when a tile is not a goal match.
    /// Hidden and unallocated cells have no attributes to evaluate.
    pub fn classify(&self, capture: &LiveMapObservation) -> Result<FloorCounts> {
        capture.validate()?;
        if capture.map.region != self.region {
            return Err(invalid("floor observation substituted the selected region"));
        }
        let mut counts = FloorCounts::default();
        for cell in &capture.map.cells {
            match cell {
                Cell::Hidden => counts.hidden += 1,
                Cell::Unallocated => counts.missing += 1,
                Cell::Visible(tile) => {
                    counts.active_designations += u32::from(tile.dig_designation != 0);
                    if tile.shape == Shape::Wall { counts.wall += 1; }
                    else if tile.shape != Shape::Floor { counts.other_shape += 1; }
                    else if tile.liquid_depth != 0 { counts.wet_floor += 1; }
                    else if tile.dig_designation != 0 { counts.designated_floor += 1; }
                    else { counts.floor_goal += 1; }
                }
            }
        }
        Ok(counts)
    }

    pub fn begin(&self, capture: LiveMapObservation) -> Result<FloorProgress> {
        self.classify(&capture)?;
        if capture.world_folder != self.folder || capture.site_id != self.site
            || capture.tick().get() > self.deadline
        {
            return Err(invalid("floor goal starts at the wrong fortress or after its deadline"));
        }
        let mut progress = FloorProgress {
            goal: self.clone(), first: capture.clone(), latest: capture,
            status: FloorStatus::Pending, streak: 0, since_tick: None,
            observations: 0, interruption: None,
        };
        progress.evaluate(false)?;
        Ok(progress)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FloorCounts {
    pub floor_goal: u32,
    pub wall: u32,
    pub other_shape: u32,
    pub wet_floor: u32,
    pub designated_floor: u32,
    pub hidden: u32,
    pub missing: u32,
    pub active_designations: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloorStatus { Pending, Unknown, Stabilizing, Satisfied, Expired, Invalidated, Cancelled }
impl FloorStatus {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Satisfied | Self::Expired | Self::Invalidated | Self::Cancelled)
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending", Self::Unknown => "unknown", Self::Stabilizing => "stabilizing",
            Self::Satisfied => "satisfied", Self::Expired => "expired",
            Self::Invalidated => "invalidated", Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloorInterruption { UnfinishedRead, ReadFailed, SampleGap, SourceChanged, MonitorCancelled }
impl FloorInterruption {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnfinishedRead => "unfinished_read", Self::ReadFailed => "read_failed",
            Self::SampleGap => "sample_gap_reset", Self::SourceChanged => "source_identity_or_clock_changed",
            Self::MonitorCancelled => "monitor_cancelled_not_game_action",
        }
    }
}

/// Opaque transition state. Public consumers cannot install a claimed success.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FloorProgress {
    goal: FloorGoal,
    first: LiveMapObservation,
    latest: LiveMapObservation,
    status: FloorStatus,
    streak: u32,
    since_tick: Option<u64>,
    observations: u32,
    interruption: Option<FloorInterruption>,
}
impl FloorProgress {
    pub fn goal(&self) -> &FloorGoal { &self.goal }
    pub fn first(&self) -> &LiveMapObservation { &self.first }
    pub fn latest(&self) -> &LiveMapObservation { &self.latest }
    pub fn status(&self) -> FloorStatus { self.status }
    pub fn streak(&self) -> u32 { self.streak }
    pub fn since_tick(&self) -> Option<u64> { self.since_tick }
    pub fn observations(&self) -> u32 { self.observations }
    pub fn interruption(&self) -> Option<FloorInterruption> { self.interruption }
    pub fn counts(&self) -> Result<FloorCounts> { self.goal.classify(&self.latest) }

    fn unfinished(&self) -> Result<()> {
        if self.status.terminal() { Err(invalid("terminal floor-goal history is immutable")) }
        else { Ok(()) }
    }

    pub fn interrupt(&self, failed: bool) -> Result<Self> {
        self.unfinished()?;
        let mut next = self.clone();
        next.status = FloorStatus::Unknown;
        next.streak = 0;
        next.since_tick = None;
        next.interruption = Some(if failed { FloorInterruption::ReadFailed } else { FloorInterruption::UnfinishedRead });
        Ok(next)
    }

    pub fn cancel(&self) -> Result<Self> {
        self.unfinished()?;
        let mut next = self.clone();
        next.status = FloorStatus::Cancelled;
        next.streak = 0;
        next.since_tick = None;
        next.interruption = Some(FloorInterruption::MonitorCancelled);
        Ok(next)
    }

    pub fn sample(&self, capture: LiveMapObservation) -> Result<Self> {
        self.unfinished()?;
        self.goal.classify(&capture)?;
        let mut next = self.clone();
        if capture.bridge_generation != self.first.bridge_generation
            || capture.df_version != self.first.df_version || capture.dfhack_version != self.first.dfhack_version
            || capture.world_folder != self.first.world_folder || capture.site_id != self.first.site_id
            || capture.map_dimensions != self.first.map_dimensions || capture.tick() < self.latest.tick()
        {
            next.status = FloorStatus::Invalidated;
            next.streak = 0;
            next.since_tick = None;
            next.interruption = Some(FloorInterruption::SourceChanged);
            return Ok(next); // Last accepted sample remains from the original source.
        }
        let gap = capture.tick().get() - self.latest.tick().get() > self.goal.max_gap_ticks;
        let advanced = capture.tick() > self.latest.tick();
        next.latest = capture;
        if gap { next.streak = 0; next.since_tick = None; }
        next.evaluate(advanced)?;
        if gap && matches!(next.status, FloorStatus::Stabilizing | FloorStatus::Satisfied) {
            next.interruption = Some(FloorInterruption::SampleGap);
        }
        Ok(next)
    }

    fn evaluate(&mut self, advanced: bool) -> Result<()> {
        self.observations = self.observations.checked_add(1)
            .filter(|n| *n <= MAX_SAMPLES).ok_or_else(|| invalid("floor-goal sample allowance exhausted"))?;
        let counts = self.counts()?;
        let tick = self.latest.tick().get();
        self.interruption = None;
        self.status = if tick > self.goal.deadline { FloorStatus::Expired }
            else if counts.hidden != 0 || counts.missing != 0 { FloorStatus::Unknown }
            else if counts.floor_goal as usize != self.goal.region.volume().map_err(map_error)? { FloorStatus::Pending }
            else {
                if self.streak == 0 { self.streak = 1; self.since_tick = Some(tick); }
                else if advanced { self.streak += 1; }
                let since = self.since_tick.ok_or_else(|| invalid("matching floor streak lost its start"))?;
                if self.streak >= self.goal.required_samples && tick - since >= self.goal.stable_ticks {
                    FloorStatus::Satisfied
                } else { FloorStatus::Stabilizing }
            };
        if !matches!(self.status, FloorStatus::Satisfied | FloorStatus::Stabilizing) {
            self.streak = 0;
            self.since_tick = None;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
