//! Foreground proof of one already-satisfied finite inventory target.
//!
//! A no-work assessment fixes the goal and its source lineage without inventing
//! an order. Later complete censuses can prove the unchanged minimum over the
//! original cadence and deadline. This process-local owner has no effect path,
//! does not advance time and cannot restore proof from a retained count.

use dfmcp_core::{
    Digest32, ErrorCode, GameTick, ObservationCursor, OperationContext, Result, StateAnchor,
};
use dfmcp_world::{EntityKind, EvidencePolicy, PredicateTruth, Value};

use crate::live_operations::LiveOperationsObservation;

use super::inventory_baseline::{
    INVENTORY_BASELINE_QUALIFY_BYTES, InventoryBaselineProjection, authorize,
};
use super::stock_target::StockTargetAssessment;
use super::{Call, error};

/// Qualification work for one complete source-bound minimum, after projection.
pub const STOCK_TARGET_WATCH_BYTES: u64 = INVENTORY_BASELINE_QUALIFY_BYTES;

/// One accepted sample. Historical achievement and current truth are separate;
/// a later contradiction cannot erase the former or establish the latter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StockTargetWatchProgress {
    pub anchor: StateAnchor,
    pub source_digest: Digest32,
    pub source_generation: u64,
    pub source_counters: [u64; 3],
    pub observed_stack_units: Option<u64>,
    pub target_stack_units: u64,
    pub current_truth: PredicateTruth,
    pub stable_samples_observed: u32,
    pub stable_samples_required: u32,
    pub last_counted_tick: Option<GameTick>,
    pub achieved_at: Option<GameTick>,
    pub deadline_tick: GameTick,
    pub deadline_reached: bool,
    pub expired: bool,
    pub continuous: bool,
    pub current_goal_proven: bool,
    pub witness: Digest32,
}

/// One non-cloneable no-work goal owner. Reopening a controller must establish
/// a new observation streak under the unchanged original deadline; this type
/// exposes no deserializer or constructor accepting historical success.
#[derive(Debug)]
pub struct StockTargetWatch {
    original: StockTargetAssessment,
    deadline_tick: GameTick,
    poll_interval_ticks: u64,
    stable_samples_required: u32,
    witness: Digest32,
    high_tick: GameTick,
    high_counters: [u64; 3],
    high_anchor: StateAnchor,
    stable_samples_observed: u32,
    last_counted_tick: Option<GameTick>,
    achieved_at: Option<GameTick>,
    expired: bool,
    last_sample_anchor: Option<StateAnchor>,
    last_sample_continuous: bool,
    interrupted: bool,
    fenced: bool,
    latest: Option<StockTargetWatchProgress>,
}

impl StockTargetWatch {
    pub fn new(
        original: StockTargetAssessment,
        deadline_tick: GameTick,
        poll_interval_ticks: u64,
        stable_samples: u32,
        context: &OperationContext,
    ) -> Result<Self> {
        let call = Call::new(context)?;
        call.admit(STOCK_TARGET_WATCH_BYTES)?;
        authorize(original.original_anchor(), &call.current()?)?;
        let span = deadline_tick
            .get()
            .checked_sub(original.original_anchor().tick.get());
        if original.amount().is_some()
            || original.deficit_stack_units() != 0
            || original.baseline_stack_units() < original.target_stack_units()
            || span.is_none_or(|ticks| {
                ticks == 0 || ticks > dfmcp_intent::effects::MAX_DEFAULT_OBLIGATION_TICKS
            })
            || !(1..=dfmcp_intent::effects::MAX_DEFAULT_OBLIGATION_TICKS)
                .contains(&poll_interval_ticks)
            || !(1..=64).contains(&stable_samples)
            || poll_interval_ticks
                .checked_mul(u64::from(stable_samples))
                .is_none_or(|ticks| span.is_none_or(|span| ticks > span))
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "no-work stock watch requires a met original and feasible fixed timing",
            ));
        }
        let mut bytes = b"dfmcp-no-work-stock-watch/1\0".to_vec();
        bytes.extend_from_slice(original.witness().as_bytes());
        bytes.extend_from_slice(&deadline_tick.get().to_be_bytes());
        bytes.extend_from_slice(&poll_interval_ticks.to_be_bytes());
        bytes.extend_from_slice(&stable_samples.to_be_bytes());
        let witness = Digest32::of_bytes(&bytes);
        authorize(original.original_anchor(), &call.current()?)?;
        Ok(Self {
            high_tick: original.original_anchor().tick,
            high_counters: original.source_counters(),
            high_anchor: original.original_anchor(),
            original,
            deadline_tick,
            poll_interval_ticks,
            stable_samples_required: stable_samples,
            witness,
            stable_samples_observed: 0,
            last_counted_tick: None,
            achieved_at: None,
            expired: false,
            last_sample_anchor: None,
            last_sample_continuous: false,
            interrupted: false,
            fenced: false,
            latest: None,
        })
    }

    pub const fn original(&self) -> &StockTargetAssessment {
        &self.original
    }

    pub const fn witness(&self) -> Digest32 {
        self.witness
    }

    pub const fn high_tick(&self) -> GameTick {
        self.high_tick
    }

    pub const fn high_cursor(&self) -> ObservationCursor {
        self.high_anchor.cursor
    }

    pub const fn fenced(&self) -> bool {
        self.fenced
    }

    pub const fn latest(&self) -> Option<&StockTargetWatchProgress> {
        self.latest.as_ref()
    }

    pub fn current_goal_proven(&self) -> bool {
        !self.fenced && self.latest.is_some_and(|sample| sample.current_goal_proven)
    }

    /// A failed acquisition or publication withdraws current proof and breaks
    /// only unfinished stability. Cadence and historical achievement survive.
    pub fn invalidate_current_sample(&mut self) {
        self.latest = None;
        self.interrupted = true;
        if self.achieved_at.is_none() {
            self.stable_samples_observed = 0;
        }
    }

    /// Retain decoded rejection floors before projection or independent policy
    /// qualification can fail. This performs no census qualification, publishes
    /// no canonical anchor and never contributes a positive stability sample.
    pub fn note_capture(&mut self, operations: &LiveOperationsObservation) -> Result<()> {
        self.latest = None;
        let tick = operations.jobs.tick();
        let counters = [
            u64::from(operations.jobs.next_job_id),
            u64::from(operations.next_building_id),
            u64::from(operations.next_item_id),
        ];
        let regressed = tick < self.high_tick
            || counters
                .iter()
                .zip(self.high_counters)
                .any(|(incoming, high)| *incoming < high);
        self.high_tick = self.high_tick.max(tick);
        for (high, incoming) in self.high_counters.iter_mut().zip(counters) {
            *high = (*high).max(incoming);
        }
        // The shared helper validates the bounded native source before hashing
        // the exact same fortress identity as the canonical citizen profile.
        let same_fortress = operations
            .jobs
            .fortress_id()
            .is_ok_and(|fortress| fortress == self.original.original_anchor().fortress_id);
        if self.fenced
            || regressed
            || operations.jobs.bridge_generation != self.original.source_generation()
            || !same_fortress
        {
            self.fenced = true;
            self.invalidate_current_sample();
            return Err(error(
                ErrorCode::StaleAnchor,
                "no-work stock watch source changed or regressed; retain its original lineage",
            ));
        }
        Ok(())
    }

    /// Qualify a fresh complete Item census. A changed deficit or outstanding
    /// producer is irrelevant: this proves a minimum and never proposes work.
    /// Unknown counts and off-cadence contradictions interrupt stability.
    pub fn sample(
        &mut self,
        projection: &InventoryBaselineProjection,
        policy: EvidencePolicy,
        context: &OperationContext,
    ) -> Result<StockTargetWatchProgress> {
        self.latest = None;
        let result = self.sample_inner(projection, policy, context);
        if result.is_err() {
            self.invalidate_current_sample();
        }
        result
    }

    fn sample_inner(
        &mut self,
        projection: &InventoryBaselineProjection,
        policy: EvidencePolicy,
        context: &OperationContext,
    ) -> Result<StockTargetWatchProgress> {
        let previous = self.note_projection(projection)?;
        let anchor = projection.original_anchor();
        let call = Call::new(context)?;
        call.admit(STOCK_TARGET_WATCH_BYTES)?;
        authorize(anchor, &call.current()?)?;
        if projection.snapshot().graph.entities.len() > context.budget.max_entities as usize {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "no-work stock watch exceeds its complete census entity allowance",
            ));
        }
        let evidence = projection.evidence(policy)?;
        let observed_stack_units = evidence.entity_quantity_total(
            &EntityKind::Item,
            "type_key",
            &Value::Text(self.original.item_type().to_owned()),
            Some("stack_size"),
        )?;
        let current_truth = observed_stack_units.map_or(PredicateTruth::Unknown, |units| {
            if units >= self.original.target_stack_units() {
                PredicateTruth::True
            } else {
                PredicateTruth::False
            }
        });
        let replay = self.last_sample_anchor == Some(anchor);
        let adjacent = anchor == previous
            || previous.cursor.sequence.checked_add(1) == Some(anchor.cursor.sequence);
        let interrupted = self.interrupted || (!replay && !adjacent);
        // An exact replay preserves its gap label without repeatedly resetting
        // the same accepted streak; only a new discontinuity interrupts it.
        let continuous = !interrupted && (!replay || self.last_sample_continuous);
        let mut count = self.stable_samples_observed;
        let mut counted_tick = self.last_counted_tick;
        let mut achieved_at = self.achieved_at;
        let mut expired = self.expired;
        if achieved_at.is_none() && !expired {
            if interrupted || current_truth != PredicateTruth::True {
                count = 0;
            }
            let cadence_basis =
                counted_tick.map_or(self.original.original_anchor().tick.get(), GameTick::get);
            if current_truth == PredicateTruth::True
                && anchor.tick <= self.deadline_tick
                && anchor.tick.get().saturating_sub(cadence_basis) >= self.poll_interval_ticks
            {
                count = count.saturating_add(1).min(self.stable_samples_required);
                counted_tick = Some(anchor.tick);
                if count == self.stable_samples_required {
                    achieved_at = Some(anchor.tick);
                }
            }
            expired = anchor.tick >= self.deadline_tick && achieved_at.is_none();
        }
        let mut progress = StockTargetWatchProgress {
            anchor,
            source_digest: projection.source_digest(),
            source_generation: projection.operations().jobs.bridge_generation,
            source_counters: [
                u64::from(projection.operations().jobs.next_job_id),
                u64::from(projection.operations().next_building_id),
                u64::from(projection.operations().next_item_id),
            ],
            observed_stack_units,
            target_stack_units: self.original.target_stack_units(),
            current_truth,
            stable_samples_observed: count,
            stable_samples_required: self.stable_samples_required,
            last_counted_tick: counted_tick,
            achieved_at,
            deadline_tick: self.deadline_tick,
            deadline_reached: anchor.tick >= self.deadline_tick,
            expired,
            continuous,
            current_goal_proven: achieved_at.is_some() && current_truth == PredicateTruth::True,
            witness: Digest32::ZERO,
        };
        progress.witness = progress_witness(self.witness, &progress);
        authorize(anchor, &call.current()?)?;
        self.stable_samples_observed = count;
        self.last_counted_tick = counted_tick;
        self.achieved_at = achieved_at;
        self.expired = expired;
        self.last_sample_anchor = Some(anchor);
        self.last_sample_continuous = continuous;
        self.interrupted = false;
        self.latest = Some(progress);
        Ok(progress)
    }

    /// Retain source and canonical rejection floors without qualifying a count.
    /// The returned previous anchor lets a portfolio preserve cohort continuity
    /// while prepaying its complete multi-target qualification before sampling.
    pub(super) fn note_projection(
        &mut self,
        projection: &InventoryBaselineProjection,
    ) -> Result<StateAnchor> {
        self.note_capture(projection.operations())?;
        let anchor = projection.original_anchor();
        let previous = self.high_anchor;
        if anchor.fortress_id != self.original.original_anchor().fortress_id
            || anchor.cursor.epoch != self.original.original_anchor().cursor.epoch
            || anchor.cursor.sequence < previous.cursor.sequence
            || anchor.tick < self.high_tick
            || anchor.tick < previous.tick
            || (anchor.cursor == previous.cursor && anchor != previous)
        {
            self.fenced = true;
            return Err(error(
                ErrorCode::StaleAnchor,
                "no-work stock watch canonical source changed lineage, regressed or forked a cursor",
            ));
        }
        // A decoded but unqualified newer anchor cannot later be rolled back.
        self.high_anchor = anchor;
        Ok(previous)
    }
}

fn progress_witness(original: Digest32, progress: &StockTargetWatchProgress) -> Digest32 {
    let mut bytes = b"dfmcp-no-work-stock-watch-sample/1\0".to_vec();
    bytes.extend_from_slice(original.as_bytes());
    bytes.extend_from_slice(&progress.anchor.fortress_id.get().to_be_bytes());
    bytes.extend_from_slice(&progress.anchor.cursor.epoch.to_be_bytes());
    bytes.extend_from_slice(&progress.anchor.cursor.sequence.to_be_bytes());
    bytes.extend_from_slice(&progress.anchor.tick.get().to_be_bytes());
    bytes.extend_from_slice(progress.anchor.state_hash.as_bytes());
    bytes.extend_from_slice(progress.source_digest.as_bytes());
    bytes.extend_from_slice(&progress.source_generation.to_be_bytes());
    for counter in progress.source_counters {
        bytes.extend_from_slice(&counter.to_be_bytes());
    }
    for value in [
        progress.observed_stack_units,
        progress.last_counted_tick.map(GameTick::get),
        progress.achieved_at.map(GameTick::get),
    ] {
        bytes.push(u8::from(value.is_some()));
        if let Some(value) = value {
            bytes.extend_from_slice(&value.to_be_bytes());
        }
    }
    bytes.extend_from_slice(&progress.stable_samples_observed.to_be_bytes());
    bytes.push(u8::from(progress.continuous));
    bytes.push(u8::from(progress.expired));
    Digest32::of_bytes(&bytes)
}

#[cfg(test)]
#[path = "stock_target_watch_tests.rs"]
mod tests;
