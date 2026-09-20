//! Pure temporal predicates over exact progress-archive records. A satisfied
//! predicate is an observation result, never goods production or effect authority.
use dfmcp_core::{Digest32, ErrorCode, FortressId, Result};
use super::{error, OrderStatus, ProgressManifest};
use super::archive::ArchivedProgress;

#[path = "watch_book.rs"]
mod book;
pub use book::{WatchBook, WatchBookSummary, WatchBatch, RetainedWatch, PrivateWatchFile,
    open_watch_book, MAX_BOOK_BYTES, MAX_BOOK_EVENTS, BOOK_OPEN_RESERVE};

pub const MAX_WATCHES: usize = 32;
pub const MAX_WATCH_KEY: usize = 64;
pub const MAX_WATCH_HORIZON: u64 = 120_000;
pub const MAX_STABLE_SAMPLES: u8 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchGoal { Validated, Active, RemainingAtMost(u32) }
impl WatchGoal {
    pub fn name(self) -> &'static str {
        match self { Self::Validated => "validated", Self::Active => "active",
            Self::RemainingAtMost(_) => "remaining_at_most" }
    }
    fn holds(self, row: &OrderStatus) -> bool {
        row.validated() && match self {
            Self::Validated => true,
            Self::Active => row.active() && row.remaining > 0,
            Self::RemainingAtMost(n) => row.remaining >= 0 && row.remaining as u32 <= n,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchSpec {
    key: String, order: u32, goal: WatchGoal, deadline: u64, cadence: u64, stable: u8,
}
impl WatchSpec {
    pub fn new(key: &str, order: u32, goal: WatchGoal, deadline: u64,
        cadence: u64, stable: u8) -> Result<Self>
    {
        if key.is_empty() || key.len() > MAX_WATCH_KEY
            || !key.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            || order > i32::MAX as u32 || deadline > super::MAX_TICK
            || !(1..=10_000).contains(&cadence) || !(1..=MAX_STABLE_SAMPLES).contains(&stable)
            || matches!(goal, WatchGoal::RemainingAtMost(n) if n > 100)
        { return Err(error(ErrorCode::InvalidRequest, "invalid bounded progress watch specification")); }
        Ok(Self { key: key.to_owned(), order, goal, deadline, cadence, stable })
    }
    pub fn key(&self) -> &str { &self.key }
    pub fn native_order_id(&self) -> u32 { self.order }
    pub fn goal(&self) -> WatchGoal { self.goal }
    pub fn deadline(&self) -> u64 { self.deadline }
    pub fn cadence(&self) -> u64 { self.cadence }
    pub fn stable_samples(&self) -> u8 { self.stable }
    pub(super) fn encode(&self, out: &mut Vec<u8>) {
        out.push(self.key.len() as u8); out.extend_from_slice(self.key.as_bytes());
        out.extend_from_slice(&self.order.to_be_bytes());
        let (tag, threshold) = match self.goal { WatchGoal::Validated => (1, 0),
            WatchGoal::Active => (2, 0), WatchGoal::RemainingAtMost(n) => (3, n) };
        out.push(tag); out.extend_from_slice(&threshold.to_be_bytes());
        out.extend_from_slice(&self.deadline.to_be_bytes());
        out.extend_from_slice(&self.cadence.to_be_bytes()); out.push(self.stable);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WatchRecordRef { pub number: u64, pub digest: Digest32 }
impl WatchRecordRef {
    pub fn of(record: &ArchivedProgress) -> Self {
        Self { number: record.entry.number, digest: record.entry.record_digest }
    }
}

/// Sealed to one exact registration record and its recognized configuration.
/// Construction is pure; registering this definition requires separate authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchDefinition {
    spec: WatchSpec, archive_id: Digest32, digest: Digest32, origin: WatchRecordRef,
    segment: u64, tick: u64, fortress: FortressId, baseline: OrderStatus,
    manifest: ProgressManifest, selection: Vec<u32>, sequence: u64, horizon: u32,
}
fn checked_record(record: &ArchivedProgress) -> Result<()> {
    let o = &record.observation;
    if record.entry.number == 0 || record.entry.number > super::archive::MAX_ARCHIVE_RECORDS as u64
        || record.entry.segment == 0
        || record.entry.witness != o.witness() || record.entry.game_tick != o.tick()
        || record.entry.native_order_ids != o.ids() || record.manifest.generation != o.generation()
        || [&record.manifest.df_version, &record.manifest.dfhack_version].iter()
            .any(|s| s.is_empty() || s.len() > 128 || s.contains('\0'))
    { return Err(error(ErrorCode::CorruptLedger, "watch evidence metadata contradicts its complete observation")); }
    Ok(())
}
impl WatchDefinition {
    pub fn new(spec: WatchSpec, archive_id: Digest32, origin: &ArchivedProgress) -> Result<Self> {
        checked_record(origin)?;
        let o = &origin.observation;
        let baseline = o.rows().iter().find(|r| r.native_order_id == spec.order)
            .and_then(|r| r.order.as_ref()).filter(|row| row.recipe != 0)
            .ok_or_else(|| error(ErrorCode::PreconditionsFailed,
                "watch registration requires a present recognized finite order in its exact archive record"))?;
        let horizon = spec.deadline.checked_sub(o.tick()).filter(|n| *n > 0 && *n <= MAX_WATCH_HORIZON)
            .ok_or_else(|| error(ErrorCode::InvalidRequest, "watch deadline must be after registration and within 120000 ticks"))?;
        if archive_id == Digest32::ZERO || spec.cadence * u64::from(spec.stable) > horizon
            || matches!(spec.goal, WatchGoal::RemainingAtMost(n) if n > baseline.total as u32)
        { return Err(error(ErrorCode::InvalidRequest, "watch predicate or sampling schedule cannot fit its finite bound")); }
        let mut bytes = b"dfmcp-progress-watch-definition/1\0".to_vec();
        bytes.extend_from_slice(archive_id.as_bytes()); spec.encode(&mut bytes);
        bytes.extend_from_slice(&origin.entry.number.to_be_bytes());
        bytes.extend_from_slice(origin.entry.record_digest.as_bytes());
        bytes.extend_from_slice(&origin.entry.segment.to_be_bytes());
        bytes.extend_from_slice(o.witness().as_bytes());
        Ok(Self { spec, archive_id, digest: Digest32::of_bytes(&bytes), origin: WatchRecordRef::of(origin),
            segment: origin.entry.segment, tick: o.tick(), fortress: o.fortress_id(),
            baseline: baseline.clone(), manifest: origin.manifest.clone(), selection: o.ids(),
            sequence: o.sequence(), horizon: o.next_order_id() })
    }
    pub fn spec(&self) -> &WatchSpec { &self.spec }
    pub fn archive_id(&self) -> Digest32 { self.archive_id }
    pub fn digest(&self) -> Digest32 { self.digest }
    pub fn origin(&self) -> WatchRecordRef { self.origin }
    pub fn segment(&self) -> u64 { self.segment }
    pub fn registered_tick(&self) -> u64 { self.tick }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchState {
    Pending, SatisfiedObservation, Expired, Cancelled, MissingOutcomeUnknown,
    ConfigurationChanged, CounterIncreased, ContinuityLost,
}
impl WatchState {
    pub fn terminal(self) -> bool { self != Self::Pending }
    pub fn name(self) -> &'static str {
        match self {
            Self::Pending => "pending", Self::SatisfiedObservation => "satisfied_observation",
            Self::Expired => "expired", Self::Cancelled => "cancelled",
            Self::MissingOutcomeUnknown => "missing_outcome_unknown",
            Self::ConfigurationChanged => "configuration_changed", Self::CounterIncreased => "counter_increased",
            Self::ContinuityLost => "continuity_lost",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchEvaluation {
    definition: WatchDefinition, state: WatchState, through: WatchRecordRef,
    last_tick: u64, last_sequence: u64, last_horizon: u32, last_remaining: i32,
    last_counted_tick: u64, evaluated_records: u32, samples: Vec<WatchRecordRef>,
}
impl WatchEvaluation {
    /// The registration observation is a baseline, not a stability sample.
    pub fn new(definition: WatchDefinition) -> Self {
        Self { state: WatchState::Pending, through: definition.origin, last_tick: definition.tick,
            last_sequence: definition.sequence, last_horizon: definition.horizon,
            last_remaining: definition.baseline.remaining, last_counted_tick: definition.tick,
            evaluated_records: 0, samples: Vec::new(), definition }
    }
    pub fn definition(&self) -> &WatchDefinition { &self.definition }
    pub fn state(&self) -> WatchState { self.state }
    pub fn through(&self) -> WatchRecordRef { self.through }
    pub fn evaluated_records(&self) -> u32 { self.evaluated_records }
    pub fn samples(&self) -> &[WatchRecordRef] { &self.samples }
    pub fn next_sample_tick(&self) -> Option<u64> {
        if self.state.terminal() { None } else {
            self.last_counted_tick.checked_add(self.definition.spec.cadence)
                .and_then(|t| self.last_tick.checked_add(1).map(|next| t.max(next)))
                .filter(|t| *t <= self.definition.spec.deadline)
        }
    }
    /// Consume every subsequent record in order, including negative observations
    /// between cadence points. No caller may skip evidence that breaks stability.
    pub fn advance(&mut self, record: &ArchivedProgress) -> Result<()> {
        if self.state.terminal() { return Ok(()); }
        checked_record(record)?;
        if self.through.number.checked_add(1) != Some(record.entry.number)
            || record.entry.previous_digest != self.through.digest
            || record.observation.fortress_id() != self.definition.fortress
        { return Err(error(ErrorCode::CorruptLedger, "watch replay omitted or substituted an archive record")); }
        let mut next = self.clone();
        next.through = WatchRecordRef::of(record);
        next.evaluated_records += 1; // At most the archive's 4096 records.
        if record.entry.segment != self.definition.segment {
            next.stop(WatchState::ContinuityLost);
        } else {
            next.advance_same_segment(record)?;
        }
        *self = next; Ok(())
    }
    fn stop(&mut self, state: WatchState) { self.state = state; self.samples.clear(); }
    fn advance_same_segment(&mut self, record: &ArchivedProgress) -> Result<()> {
        let o = &record.observation;
        if record.manifest != self.definition.manifest || o.ids() != self.definition.selection
            || o.sequence() <= self.last_sequence || o.tick() < self.last_tick
            || o.next_order_id() < self.last_horizon
        { return Err(error(ErrorCode::CorruptLedger, "watch archive segment crosses source, selection, sequence or clock boundaries")); }
        self.last_tick = o.tick(); self.last_sequence = o.sequence(); self.last_horizon = o.next_order_id();
        if o.tick() > self.definition.spec.deadline { self.stop(WatchState::Expired); return Ok(()); }
        let Some(row) = o.rows().iter().find(|r| r.native_order_id == self.definition.spec.order)
            .and_then(|r| r.order.as_ref()) else {
                self.stop(WatchState::MissingOutcomeUnknown); return Ok(());
            };
        if !self.definition.baseline.same_configuration(row) {
            self.stop(WatchState::ConfigurationChanged); return Ok(());
        }
        if row.remaining > self.last_remaining { self.stop(WatchState::CounterIncreased); return Ok(()); }
        self.last_remaining = row.remaining;
        if !self.definition.spec.goal.holds(row) {
            // Even an off-cadence false observation interrupts positive stability.
            self.samples.clear();
        } else if self.last_counted_tick.checked_add(self.definition.spec.cadence)
            .is_some_and(|tick| o.tick() >= tick)
        {
            self.samples.push(self.through); self.last_counted_tick = o.tick();
            if self.samples.len() == usize::from(self.definition.spec.stable) {
                self.state = WatchState::SatisfiedObservation;
            }
        }
        // A sample at the deadline may satisfy; otherwise that same record expires.
        if o.tick() == self.definition.spec.deadline && !self.state.terminal() {
            self.stop(WatchState::Expired);
        }
        Ok(())
    }
    /// Local durable cancellation is ordered AFTER all observations up to `at`.
    /// A terminal observation cannot be rewritten into cancellation.
    pub fn cancel_at(&mut self, at: WatchRecordRef) -> Result<()> {
        if self.state == WatchState::Cancelled && self.through == at { return Ok(()); }
        if self.state.terminal() || self.through != at {
            return Err(error(ErrorCode::Conflict, "watch cancellation must name the exact pending evaluation frontier"));
        }
        self.stop(WatchState::Cancelled); Ok(())
    }
}

#[cfg(test)]
#[path = "watches_tests.rs"]
mod tests;
