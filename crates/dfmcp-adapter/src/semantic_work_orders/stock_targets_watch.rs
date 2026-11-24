//! Coherent foreground proof of an originally satisfied inventory portfolio.
//!
//! Every target is fixed by the same original qualified census. A later sample
//! qualifies all minima against one projection and advances one shared stability
//! window. Independently achieved member histories cannot prove the portfolio.
//! This process-local owner creates no effects and restores no current proof.

use dfmcp_core::{
    Digest32, ErrorCode, GameTick, ObservationCursor, OperationContext, Result, StateAnchor,
};
use dfmcp_world::{EntityKind, EvidencePolicy, PredicateTruth, Value};

use crate::live_operations::LiveOperationsObservation;
use crate::work_orders::WorkOrderRecipe;

use super::inventory_baseline::{
    INVENTORY_BASELINE_QUALIFY_BYTES, InventoryBaselineProjection, authorize,
};
use super::{Call, StockTargetAssessment, StockTargetWatch, error};

/// Prepaid qualification work for the complete bounded four-target portfolio.
pub const STOCK_TARGETS_WATCH_BYTES: u64 = 4 * INVENTORY_BASELINE_QUALIFY_BYTES;

/// One member's current count in the shared capture. No independent historical
/// achievement is exposed or used to satisfy the complete portfolio.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StockTargetsWatchCount {
    pub recipe: WorkOrderRecipe,
    pub item_type: &'static str,
    pub observed_stack_units: Option<u64>,
    pub target_stack_units: u64,
    pub current_truth: PredicateTruth,
}

/// One atomically published complete cohort with shared temporal proof.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StockTargetsWatchProgress {
    pub anchor: StateAnchor,
    pub source_digest: Digest32,
    pub source_generation: u64,
    pub source_counters: [u64; 3],
    pub targets: Vec<StockTargetsWatchCount>,
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

/// One non-cloneable owner of a complete, originally satisfied target vector.
///
/// The private single-target guard supplies the established source, cursor and
/// rejection-floor checks. Its individual stability and achievement are never
/// used as portfolio proof: the portfolio has its own simultaneous streak.
#[derive(Debug)]
pub struct StockTargetsWatch {
    originals: Vec<StockTargetAssessment>,
    source_guard: StockTargetWatch,
    deadline_tick: GameTick,
    poll_interval_ticks: u64,
    stable_samples_required: u32,
    witness: Digest32,
    stable_samples_observed: u32,
    last_counted_tick: Option<GameTick>,
    achieved_at: Option<GameTick>,
    expired: bool,
    last_sample_anchor: Option<StateAnchor>,
    last_sample_continuous: bool,
    interrupted: bool,
    latest: Option<StockTargetsWatchProgress>,
}

impl StockTargetsWatch {
    pub fn new(
        mut originals: Vec<StockTargetAssessment>,
        deadline_tick: GameTick,
        poll_interval_ticks: u64,
        stable_samples: u32,
        context: &OperationContext,
    ) -> Result<Self> {
        let call = Call::new(context)?;
        call.admit(STOCK_TARGETS_WATCH_BYTES)?;
        if originals.is_empty() || originals.len() > 4 {
            return Err(error(
                ErrorCode::InvalidRequest,
                "no-work portfolio requires one to four original stock targets",
            ));
        }
        originals.sort_by(|left, right| left.recipe().as_str().cmp(right.recipe().as_str()));
        let first = &originals[0];
        if originals
            .windows(2)
            .any(|pair| pair[0].recipe() == pair[1].recipe())
            || originals.iter().any(|original| {
                original.amount().is_some()
                    || original.deficit_stack_units() != 0
                    || original.baseline_stack_units() < original.target_stack_units()
                    || original.original_anchor() != first.original_anchor()
                    || original.source_digest() != first.source_digest()
                    || original.source_generation() != first.source_generation()
                    || original.source_counters() != first.source_counters()
                    || original.item_rows() != first.item_rows()
            })
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "no-work portfolio requires distinct satisfied targets from one exact census",
            ));
        }
        let source_guard = StockTargetWatch::new(
            first.clone(),
            deadline_tick,
            poll_interval_ticks,
            stable_samples,
            &call.current()?,
        )?;
        let mut bytes = b"dfmcp-no-work-stock-targets-watch/1\0".to_vec();
        bytes.extend_from_slice(&(originals.len() as u32).to_be_bytes());
        for original in &originals {
            bytes.extend_from_slice(original.witness().as_bytes());
        }
        bytes.extend_from_slice(&deadline_tick.get().to_be_bytes());
        bytes.extend_from_slice(&poll_interval_ticks.to_be_bytes());
        bytes.extend_from_slice(&stable_samples.to_be_bytes());
        let witness = Digest32::of_bytes(&bytes);
        authorize(first.original_anchor(), &call.current()?)?;
        Ok(Self {
            originals,
            source_guard,
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
            latest: None,
        })
    }

    pub fn originals(&self) -> &[StockTargetAssessment] {
        &self.originals
    }

    pub const fn witness(&self) -> Digest32 {
        self.witness
    }

    pub const fn high_tick(&self) -> GameTick {
        self.source_guard.high_tick()
    }

    pub const fn high_cursor(&self) -> ObservationCursor {
        self.source_guard.high_cursor()
    }

    pub const fn fenced(&self) -> bool {
        self.source_guard.fenced()
    }

    pub const fn latest(&self) -> Option<&StockTargetsWatchProgress> {
        self.latest.as_ref()
    }

    /// Historical joint achievement is independent of current proof withdrawal.
    pub const fn achieved_at(&self) -> Option<GameTick> {
        self.achieved_at
    }

    pub fn current_goal_proven(&self) -> bool {
        !self.fenced()
            && self
                .latest
                .as_ref()
                .is_some_and(|sample| sample.current_goal_proven)
    }

    /// Failed acquisition/publication withdraws the entire cohort. It cannot
    /// leave earlier members visible or turn an incomplete sample into proof.
    pub fn invalidate_current_sample(&mut self) {
        self.latest = None;
        self.source_guard.invalidate_current_sample();
        self.interrupted = true;
        if self.achieved_at.is_none() {
            self.stable_samples_observed = 0;
        }
    }

    /// Consume common native rejection floors before independent qualification.
    /// No count or positive stability sample is published by this operation.
    pub fn note_capture(&mut self, operations: &LiveOperationsObservation) -> Result<()> {
        self.latest = None;
        let result = self.source_guard.note_capture(operations);
        if result.is_err() {
            self.invalidate_current_sample();
        }
        result
    }

    pub fn sample(
        &mut self,
        projection: &InventoryBaselineProjection,
        policy: EvidencePolicy,
        context: &OperationContext,
    ) -> Result<StockTargetsWatchProgress> {
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
    ) -> Result<StockTargetsWatchProgress> {
        // Even a later authority, entity-budget or policy refusal must retain
        // the decoded native frontier. The caller also uses note_capture before
        // constructing the projection, which may itself refuse.
        let previous = self.source_guard.note_projection(projection)?;
        let call = Call::new(context)?;
        call.admit(STOCK_TARGETS_WATCH_BYTES)?;
        let first = self
            .source_guard
            .sample(projection, policy.clone(), &call.current()?)?;
        let evidence = projection.evidence(policy)?;
        let mut targets = Vec::with_capacity(self.originals.len());
        for (index, original) in self.originals.iter().enumerate() {
            // Reuse the guard's independently qualified first count. Every
            // remaining count uses the same immutable projection and policy.
            let observed_stack_units = if index == 0 {
                first.observed_stack_units
            } else {
                evidence.entity_quantity_total(
                    &EntityKind::Item,
                    "type_key",
                    &Value::Text(original.item_type().to_owned()),
                    Some("stack_size"),
                )?
            };
            let current_truth = observed_stack_units.map_or(PredicateTruth::Unknown, |units| {
                if units >= original.target_stack_units() {
                    PredicateTruth::True
                } else {
                    PredicateTruth::False
                }
            });
            targets.push(StockTargetsWatchCount {
                recipe: original.recipe(),
                item_type: original.item_type(),
                observed_stack_units,
                target_stack_units: original.target_stack_units(),
                current_truth,
            });
        }
        let current_truth = if targets
            .iter()
            .any(|target| target.current_truth == PredicateTruth::False)
        {
            PredicateTruth::False
        } else if targets
            .iter()
            .any(|target| target.current_truth == PredicateTruth::Unknown)
        {
            PredicateTruth::Unknown
        } else {
            PredicateTruth::True
        };
        let anchor = projection.original_anchor();
        let replay = self.last_sample_anchor == Some(anchor);
        let adjacent = anchor == previous
            || previous.cursor.sequence.checked_add(1) == Some(anchor.cursor.sequence);
        let interrupted = self.interrupted || (!replay && !adjacent);
        let continuous = !interrupted && (!replay || self.last_sample_continuous);
        let mut count = self.stable_samples_observed;
        let mut counted_tick = self.last_counted_tick;
        let mut achieved_at = self.achieved_at;
        let mut expired = self.expired;
        if achieved_at.is_none() && !expired {
            if interrupted || current_truth != PredicateTruth::True {
                count = 0;
            }
            let cadence_basis = counted_tick.map_or(
                self.source_guard.original().original_anchor().tick.get(),
                GameTick::get,
            );
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
        let mut progress = StockTargetsWatchProgress {
            anchor,
            source_digest: first.source_digest,
            source_generation: first.source_generation,
            source_counters: first.source_counters,
            targets,
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
        self.latest = Some(progress.clone());
        Ok(progress)
    }
}

fn progress_witness(original: Digest32, progress: &StockTargetsWatchProgress) -> Digest32 {
    let mut bytes = b"dfmcp-no-work-stock-targets-watch-sample/1\0".to_vec();
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
    bytes.extend_from_slice(&(progress.targets.len() as u32).to_be_bytes());
    for target in &progress.targets {
        bytes.push(target.recipe as u8);
        bytes.extend_from_slice(&target.target_stack_units.to_be_bytes());
        bytes.push(u8::from(target.observed_stack_units.is_some()));
        if let Some(units) = target.observed_stack_units {
            bytes.extend_from_slice(&units.to_be_bytes());
        }
    }
    for value in [
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
#[path = "stock_targets_watch_tests.rs"]
mod tests;
