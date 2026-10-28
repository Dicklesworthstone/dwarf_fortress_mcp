//! One original semantic Mine plan through exact native designation custody.
//!
//! A paired review preserves the original action, predicates, obligation and
//! archive identity. A native Designated receipt proves designation only; excavation
//! completion still needs the original goal and fresh independent evidence.
//! No source grant, admission, checkpoint or canonical identity is issued here.

use std::sync::Arc;
use std::time::{Duration, Instant};

use dfmcp_core::{
    Capability, Digest32, ErrorCode, GameTick, OperationContext, Result, RiskTier, StateAnchor,
    StepId,
};
use dfmcp_intent::{Action, ObligationSpec, PreparedPlan};
use dfmcp_world::Predicate;

use crate::control_effect_journal::EffectJournalStorage;
use crate::dig_designation::journal::session::{DigSession, DigSessionGuard, DigSessionView};
use crate::dig_designation::journal::{
    DigBinding, DigGuard, DigMode, DigRecord, DigStage, DigState,
};
use crate::dig_designation::rpc::DigSource;
use crate::dig_designation::{DigEffect, DigObservation, DigPhase, DigPlan, DigRegion};
use crate::live_routing::{LiveRoutingEvidence, resolve_dig_step};
pub mod goal_history;
pub mod goal_monitor;
pub mod goal_projection;
pub mod store;
pub use goal_history::{
    DurableDigGoalMonitor, GoalHistoryStore, PrivateGoalHistoryStore,
    open_private_goal_history_store,
};
pub use goal_monitor::{DigGoalMonitor, DigGoalProgress, DigGoalResult, DigGoalSource};
pub use goal_projection::DigGoalProjection;
pub mod terrain;
use store::{Association, MAX_ASSOCIATIONS, MAX_STORE_BYTES};
pub use store::{AssociationStore, PrivateAssociationStore, open_private_association_store};

/// The trusted shell acquires a complete independently qualified spatial source.
/// The native capture constrains that source; it never becomes source authority.
pub trait DigEvidenceOwner {
    fn refresh_after_capture(
        &mut self,
        capture: &DigObservation,
        context: &OperationContext,
    ) -> Result<()>;
    fn routing_evidence(&self) -> Result<LiveRoutingEvidence<'_>>;
    /// Complete bounded acquisition/projection cost, prepaid independently from
    /// the native RPC allowance. The caller's operation budget is never renewed.
    fn refresh_reserve_bytes(&self) -> Result<u64> {
        Ok(64 * 1024 * 1024)
    }
}

const MAX_ORIGINAL_BYTES: usize = 64 * 1024;
const LOCAL_BYTES: u64 = 1024 * 1024;
const STORE_BYTES: u64 = MAX_STORE_BYTES as u64;
// At most sixteen policy edges, each bracketed by exact association reads.
const GUARD_BYTES: u64 = 32 * STORE_BYTES + LOCAL_BYTES + 1;

fn error(code: ErrorCode, message: &str) -> dfmcp_core::DfmcpError {
    dfmcp_core::DfmcpError::new(code, message)
}
fn exhausted() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "semantic dig handoff exhausted its bounded allowance",
    )
}
fn custody() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::CorruptLedger,
        "semantic/native dig custody differs; retain history and do not redispatch",
    )
}
fn refused(message: &str) -> dfmcp_core::DfmcpError {
    error(ErrorCode::AdapterRejected, message)
}
fn protected(predicate: &Predicate) -> bool {
    match predicate {
        Predicate::True | Predicate::Paused(true) => true,
        Predicate::All(children) => children.iter().all(protected),
        _ => false,
    }
}
fn original(plan: &PreparedPlan, call: &Call) -> Result<Digest32> {
    // The lossless bounded visitor runs before recursive semantic validation or
    // hashing. It also distinguishes typed variants with older digest aliases.
    let (_, archive) = dfmcp_intent::plan_archive::encode_with_check(
        plan,
        dfmcp_intent::plan_archive::ArchiveLimits {
            max_bytes: MAX_ORIGINAL_BYTES,
            max_steps: 1,
            max_nodes: 1024,
            max_depth: 32,
        },
        || call.current().map(|_| ()),
    )?;
    if plan.steps.len() != 1 || plan.summary.len() > 4096 || plan.requires_checkpoint {
        return Err(refused(
            "semantic dig requires one original step and cannot supply a mandatory checkpoint",
        ));
    }
    let step = &plan.steps[0];
    if !matches!(
        step.action,
        Action::DesignateDig {
            mode: dfmcp_intent::DigMode::Mine,
            ..
        }
    ) || !step.depends_on.is_empty()
        || step.compensation.is_some()
        || step.preconditions.len() > 64
        || step.postconditions.len() > 64
        || step.preconditions.iter().any(|p| !protected(p))
    {
        return Err(refused(
            "original dig supports only one exact Mine action without unprotected conditions, dependencies or compensation",
        ));
    }
    let route = crate::live_routing::route_step(step)
        .outcome
        .map_err(|r| refused(&r.reason))?;
    if !matches!(route.request,crate::live_routing::LiveRequest::Dig{ref regions} if regions.len()==1)
    {
        return Err(refused(
            "original Mine must be one 1..8 by 1..8 single-level rectangle",
        ));
    }
    Ok(archive.object_digest)
}
fn authorize_scope(plan: &PreparedPlan, c: &OperationContext) -> Result<()> {
    if c.budget.max_actions == 0 {
        return Err(exhausted());
    }
    if c.anchor.fortress_id != plan.anchor.fortress_id {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "semantic dig authority belongs to another fortress",
        ));
    }
    c.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
    c.authorize(Capability::Designate, RiskTier::Guarded, &[], None)
}
fn authorize_original(plan: &PreparedPlan, c: &OperationContext) -> Result<()> {
    authorize_scope(plan, c)?;
    if c.anchor != plan.anchor
        || c.anchor.tick >= plan.expires_at_tick
        || plan.steps[0]
            .obligation
            .as_ref()
            .is_some_and(|o| c.anchor.tick >= o.deadline_tick)
    {
        return Err(error(
            ErrorCode::StaleAnchor,
            "original semantic dig anchor or deadline no longer permits control",
        ));
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct SemanticDigReview {
    original: PreparedPlan,
    native: DigPlan,
    association: Association,
    seal: Digest32,
}
impl SemanticDigReview {
    pub fn original_plan(&self) -> &PreparedPlan {
        &self.original
    }
    pub fn native_plan(&self) -> &DigPlan {
        &self.native
    }
    pub const fn seal(&self) -> Digest32 {
        self.seal
    }
    pub fn source_identity_digest(&self) -> Digest32 {
        self.association.source_identity
    }
    pub fn original_object_digest(&self) -> Digest32 {
        self.association.original_object
    }
}

/// Historical designation readback. It cannot establish completion, current
/// physical truth or any semantic postcondition merely by being retained.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeDigDesignation {
    Unverified,
    Designated {
        receipt: Digest32,
        designated_count: u32,
    },
}
#[derive(Clone, Debug)]
pub struct SemanticDigResult {
    review: SemanticDigReview,
    native: DigRecord,
    designation: NativeDigDesignation,
}
impl SemanticDigResult {
    pub fn review(&self) -> &SemanticDigReview {
        &self.review
    }
    pub fn native_record(&self) -> &DigRecord {
        &self.native
    }
    pub fn designation(&self) -> &NativeDigDesignation {
        &self.designation
    }
    pub fn designation_result(&self) -> &NativeDigDesignation {
        &self.designation
    }
    pub const fn original_goal_proven(&self) -> bool {
        false
    }
    pub fn pending_obligation(&self) -> Option<&ObligationSpec> {
        self.review.original.steps[0].obligation.as_ref()
    }
}

struct DigInventory {
    journal_id: Digest32,
    native: DigSessionView,
    entry: Option<DigRecord>,
}
impl DigInventory {
    fn entry(&self, key: &str) -> Option<&DigRecord> {
        self.entry
            .as_ref()
            .filter(|entry| entry.plan().key() == key)
    }
    fn pending_count(&self) -> usize {
        usize::from(self.native.pending.is_some())
    }
}

struct Call {
    context: OperationContext,
    until: Instant,
    bytes: u64,
}
impl Call {
    fn new(c: &OperationContext) -> Result<Self> {
        c.budget.validate()?;
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        Ok(Self {
            context: c.clone(),
            until: Instant::now()
                .checked_add(Duration::from_millis(c.budget.max_wall_millis.min(60_000)))
                .ok_or_else(exhausted)?,
            bytes: c.budget.max_bytes,
        })
    }
    fn current(&self) -> Result<OperationContext> {
        let remaining = self
            .until
            .checked_duration_since(Instant::now())
            .filter(|d| *d >= Duration::from_millis(1))
            .ok_or_else(exhausted)?;
        let mut c = self.context.clone();
        c.budget.max_wall_millis = u64::try_from(remaining.as_millis()).map_err(|_| exhausted())?;
        c.budget.max_bytes = self.bytes;
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        Ok(c)
    }
    fn reserve(&mut self, bytes: u64) -> Result<OperationContext> {
        let mut c = self.current()?;
        if bytes == 0 {
            return Err(exhausted());
        }
        self.bytes = self
            .bytes
            .checked_sub(bytes)
            .filter(|n| *n > 0)
            .ok_or_else(exhausted)?;
        c.budget.max_bytes = bytes;
        Ok(c)
    }
    /// Native callbacks carry a separate RPC reservation. This guard already
    /// owns prepaid bytes from the same outer call; only authority and the
    /// shrinking deadline are refreshed from the callback, never its allowance.
    fn boundary(&mut self, c: &OperationContext) -> Result<()> {
        if c.session_id != self.context.session_id
            || c.request_id != self.context.request_id
            || c.grants != self.context.grants
            || c.anchor.fortress_id != self.context.anchor.fortress_id
            || c.anchor.cursor != self.context.anchor.cursor
            || c.anchor.state_hash != self.context.anchor.state_hash
            || c.anchor.tick < self.context.anchor.tick
        {
            return Err(custody());
        }
        self.context.anchor.tick = c.anchor.tick;
        self.context.cancellation_requested |= c.cancellation_requested;
        self.until = self.until.min(
            Instant::now()
                .checked_add(Duration::from_millis(c.budget.max_wall_millis))
                .ok_or_else(exhausted)?,
        );
        self.current().map(|_| ())
    }
}

#[derive(Clone, Copy)]
struct NativeClock {
    epoch: u64,
    generation: u64,
    sequence: u64,
    tick: u64,
}
impl NativeClock {
    fn observed(observation: &DigObservation, epoch: u64) -> Self {
        Self {
            epoch,
            generation: observation.generation(),
            sequence: observation.sequence(),
            tick: observation.tick(),
        }
    }
}

/// This owner does not issue restore epochs or bridge incarnations. A retained
/// source binds one of each; a separately established recovery/session owns any
/// transition. Canonical cursors and native intervention sequences stay distinct.
#[derive(Clone, Default)]
struct SourceFloor {
    canonical: Option<StateAnchor>,
    native: Option<NativeClock>,
    // Valid source observations may advance game time even when a later family
    // or proof publication fails. Replacing a monitor cannot revive older time.
    authority_tick: Option<(u64, GameTick)>,
}
impl SourceFloor {
    fn retain_tick(&mut self, epoch: u64, tick: GameTick) -> Result<()> {
        if self
            .canonical
            .is_some_and(|anchor| anchor.cursor.epoch != epoch)
            || self.native.is_some_and(|clock| clock.epoch != epoch)
            || self
                .authority_tick
                .is_some_and(|(old_epoch, _)| old_epoch != epoch)
        {
            return Err(custody());
        }
        let high = self.authority_tick.map_or(tick, |(_, high)| high.max(tick));
        self.authority_tick = Some((epoch, high));
        Ok(())
    }
    fn retain_anchor(&mut self, anchor: StateAnchor) -> Result<()> {
        if self
            .native
            .is_some_and(|clock| clock.epoch != anchor.cursor.epoch)
            || self
                .authority_tick
                .is_some_and(|(epoch, _)| epoch != anchor.cursor.epoch)
        {
            return Err(custody());
        }
        if let Some(old) = self.canonical {
            if anchor.fortress_id != old.fortress_id || anchor.cursor.epoch != old.cursor.epoch {
                return Err(custody());
            }
            match anchor.cursor.sequence.cmp(&old.cursor.sequence) {
                std::cmp::Ordering::Less if anchor.tick <= old.tick => return Ok(()),
                std::cmp::Ordering::Equal if anchor == old => return Ok(()),
                std::cmp::Ordering::Greater if anchor.tick >= old.tick => {}
                _ => return Err(custody()),
            }
        }
        self.retain_tick(anchor.cursor.epoch, anchor.tick)?;
        self.canonical = Some(anchor);
        Ok(())
    }
    fn retain_native(&mut self, clock: NativeClock) -> Result<()> {
        if self
            .canonical
            .is_some_and(|anchor| anchor.cursor.epoch != clock.epoch)
            || self
                .authority_tick
                .is_some_and(|(epoch, _)| epoch != clock.epoch)
        {
            return Err(custody());
        }
        if let Some(old) = self.native {
            if clock.epoch != old.epoch
                || clock.generation != old.generation
                || (clock.sequence < old.sequence && clock.tick > old.tick)
                || (clock.sequence > old.sequence && clock.tick < old.tick)
            {
                return Err(custody());
            }
            self.native = Some(NativeClock {
                epoch: old.epoch,
                generation: old.generation,
                sequence: old.sequence.max(clock.sequence),
                tick: old.tick.max(clock.tick),
            });
        } else {
            self.native = Some(clock);
        }
        self.retain_tick(clock.epoch, GameTick(clock.tick))?;
        Ok(())
    }
    fn permit_anchor(&self, anchor: StateAnchor) -> Result<()> {
        if self
            .authority_tick
            .is_some_and(|(epoch, tick)| anchor.cursor.epoch != epoch || anchor.tick < tick)
            || self.canonical.is_some_and(|old| {
                anchor.fortress_id != old.fortress_id
                    || anchor.cursor.epoch != old.cursor.epoch
                    || anchor.cursor.sequence < old.cursor.sequence
                    || anchor.tick < old.tick
                    || (anchor.cursor.sequence == old.cursor.sequence && anchor != old)
            })
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "original dig source predates or forks retained canonical evidence",
            ));
        }
        Ok(())
    }
    fn permit_native(&self, clock: NativeClock) -> Result<()> {
        if self
            .authority_tick
            .is_some_and(|(epoch, tick)| clock.epoch != epoch || GameTick(clock.tick) < tick)
            || self.native.is_some_and(|old| {
                clock.epoch != old.epoch
                    || clock.generation != old.generation
                    || clock.sequence < old.sequence
                    || clock.tick < old.tick
            })
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "native dig capture predates retained evidence or changes incarnation",
            ));
        }
        Ok(())
    }
    fn authority(&self, call: &mut Call) -> Result<()> {
        if let Some((epoch, tick)) = self.authority_tick {
            if call.context.anchor.cursor.epoch < epoch {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "authority predates the retained source epoch",
                ));
            }
            if call.context.anchor.cursor.epoch == epoch {
                call.context.anchor.tick = call.context.anchor.tick.max(tick);
            }
        }
        if let Some(anchor) = self.canonical {
            if call.context.anchor.cursor.epoch < anchor.cursor.epoch {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "authority predates the retained restore epoch",
                ));
            }
            if call.context.anchor.cursor.epoch == anchor.cursor.epoch {
                call.context.anchor.tick = GameTick(
                    call.context
                        .anchor
                        .tick
                        .get()
                        .max(anchor.tick.get())
                        .max(self.native.map_or(0, |clock| clock.tick)),
                );
            }
        } else if let Some(clock) = self.native {
            if call.context.anchor.cursor.epoch < clock.epoch {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "authority predates the retained restore epoch",
                ));
            }
            if call.context.anchor.cursor.epoch == clock.epoch {
                call.context.anchor.tick = GameTick(call.context.anchor.tick.get().max(clock.tick));
            }
        }
        call.current()?;
        Ok(())
    }
}

/// Exclusive native coordinator plus immutable original-plan custody. A new
/// association store cannot adopt an existing native key, even with equal bytes.
pub struct SemanticDigSession<S, B, N> {
    native: DigSession<S, N>,
    associations: AssociationStore<B>,
    attached: Option<SemanticDigReview>,
    floor: SourceFloor,
    goal_owner: Option<Arc<()>>,
    goal_floor: goal_monitor::DigSourceFloor,
}
impl<S: EffectJournalStorage, B: EffectJournalStorage, N: DigSource> SemanticDigSession<S, B, N> {
    pub fn new(
        native: DigSession<S, N>,
        associations: AssociationStore<B>,
        c: &OperationContext,
    ) -> Result<Self> {
        let mut out = Self {
            native,
            associations,
            attached: None,
            floor: SourceFloor::default(),
            goal_owner: None,
            goal_floor: goal_monitor::DigSourceFloor::default(),
        };
        out.view(&mut Call::new(c)?)?;
        Ok(out)
    }
    pub(super) fn abandon_preparation(&mut self) {
        self.native.abandon_preparation();
    }

    fn view_reserve(&self) -> u64 {
        // A prepare/recovery/commit may append several bounded frames before
        // the final view. Prepay that growth before any persistence or effect.
        2 * self.native.operation_reserve(false)
            + 2 * STORE_BYTES
            + 8 * 32 * crate::dig_designation::journal::MAX_FRAME_BYTES as u64
    }
    fn view(&mut self, call: &mut Call) -> Result<DigInventory> {
        self.floor.retain_tick(
            call.context.anchor.cursor.epoch,
            GameTick(self.native.high_tick()),
        )?;
        self.floor.authority(call)?;
        self.associations.verify(&call.reserve(STORE_BYTES)?)?;
        if let Some(a) = self.associations.sole() {
            self.floor.retain_anchor(a.anchor)?;
        }
        self.floor.authority(call)?;
        let native = self
            .native
            .view(&call.reserve(self.native.operation_reserve(false))?)?;
        if native.journal_id != self.associations.native_journal()
            || native.total_records > MAX_ASSOCIATIONS
        {
            return Err(custody());
        }
        let entry = if native.total_records == 1 {
            let a = self.associations.sole().ok_or_else(custody)?.clone();
            let record = self.native.get(
                &a.key,
                a.native,
                &call.reserve(self.native.operation_reserve(false))?,
            )?;
            if a.witness != record.plan().before().witness()
                || a.anchor.fortress_id != self.native.binding().fortress_id()
                || a.step != StepId::ZERO
                || record.plan().allow_hidden_neighbors()
            {
                return Err(custody());
            }
            let mut clock = NativeClock::observed(record.plan().before(), a.anchor.cursor.epoch);
            if record
                .effect()
                .is_some_and(|e| e.phase() == DigPhase::Designated)
            {
                clock.sequence = clock.sequence.checked_add(1).ok_or_else(custody)?;
            }
            self.floor.retain_native(clock)?;
            Some(record)
        } else {
            None
        };
        self.floor.authority(call)?;
        self.associations.verify(&call.reserve(STORE_BYTES)?)?;
        Ok(DigInventory {
            journal_id: native.journal_id,
            native,
            entry,
        })
    }
    fn writable(&self, control: bool) -> Result<()> {
        if self.associations.is_read_only()
            || self.native.mode() == DigMode::Offline
            || (control && self.native.mode() != DigMode::Control)
        {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "offline or recovery dig custody cannot authorize this native operation",
            ));
        }
        Ok(())
    }
    fn attached(&self, seal: Digest32) -> Result<SemanticDigReview> {
        self.attached
            .as_ref()
            .filter(|r| r.seal == seal)
            .cloned()
            .ok_or_else(|| {
                error(
                    ErrorCode::Conflict,
                    "attach the exact original dig plan and paired review seal",
                )
            })
    }
    fn exact(&self, review: &SemanticDigReview) -> Result<()> {
        if self.associations.get(review.native.key()) != Some(&review.association) {
            return Err(custody());
        }
        Ok(())
    }
    fn finish(
        &mut self,
        review: SemanticDigReview,
        native: DigRecord,
        call: &mut Call,
    ) -> Result<SemanticDigResult> {
        let inventory = self.view(call)?;
        self.exact(&review)?;
        if native.plan() != &review.native || inventory.entry(native.plan().key()) != Some(&native)
        {
            return Err(custody());
        }
        let designation = match native.effect() {
            Some(effect)
                if native.state() == DigState::Terminal
                    && effect.phase() == DigPhase::Designated =>
            {
                let decoded = DigEffect::decode(effect.canonical_bytes(), &review.native)?;
                NativeDigDesignation::Designated {
                    receipt: decoded.receipt().ok_or_else(custody)?,
                    designated_count: decoded.designated_count().ok_or_else(custody)?,
                }
            }
            _ => NativeDigDesignation::Unverified,
        };
        call.current()?;
        Ok(SemanticDigResult {
            review,
            native,
            designation,
        })
    }
    pub fn inventory(&mut self, c: &OperationContext) -> Result<DigSessionView> {
        Ok(self.view(&mut Call::new(c)?)?.native)
    }
    pub fn high_tick(&self) -> u64 {
        self.native.high_tick()
    }
    pub fn binding(&self) -> &DigBinding {
        self.native.binding()
    }
    pub fn observe<E, F, G>(
        &mut self,
        plan: PreparedPlan,
        evidence: &mut E,
        c: &OperationContext,
        factory: F,
        guard: &mut G,
    ) -> Result<SemanticDigReview>
    where
        E: DigEvidenceOwner,
        F: FnOnce(&DigBinding, DigRegion, &OperationContext) -> Result<N>,
        G: DigSessionGuard,
    {
        self.attached = None;
        self.abandon_preparation();
        let result = (|| {
            let mut call = Call::new(c)?;
            call.reserve(LOCAL_BYTES)?;
            let original_object = original(&plan, &call)?;
            let inventory = self.view(&mut call)?;
            self.writable(true)?;
            authorize_original(&plan, &call.current()?)?;
            self.floor.permit_anchor(plan.anchor)?;
            let key = &plan.steps[0].idempotency_key;
            if inventory.native.total_records != 0 {
                return Err(error(
                    ErrorCode::Conflict,
                    "native key exists; reattach only its exact original dig plan",
                ));
            }
            // Prepay everything remaining before connecting; each subsystem gets
            // its own reservation from this one monotonically shrinking call.
            let evidence_context = call.reserve(evidence.refresh_reserve_bytes()?)?;
            let final_context = call.reserve(self.view_reserve())?;
            let native_context = call.reserve(self.native.operation_reserve(true))?;
            let route = crate::live_routing::route_step(&plan.steps[0])
                .outcome
                .map_err(|r| refused(&r.reason))?;
            let crate::live_routing::LiveRequest::Dig { regions } = route.request else {
                return Err(custody());
            };
            let region = *regions.first().ok_or_else(custody)?;
            let captured = self.native.observe(region, &native_context, factory, guard);
            self.floor
                .retain_tick(plan.anchor.cursor.epoch, GameTick(self.native.high_tick()))?;
            let capture = captured?;
            let clock = NativeClock::observed(&capture, plan.anchor.cursor.epoch);
            self.floor.permit_native(clock)?;
            self.floor.retain_native(clock)?;
            self.floor.authority(&mut call)?;
            authorize_original(&plan, &call.current()?)?;
            let mut current = call.current()?;
            current.budget.max_bytes = evidence_context.budget.max_bytes;
            evidence.refresh_after_capture(&capture, &current)?;
            let route =
                resolve_dig_step(&plan, StepId::ZERO, &evidence.routing_evidence()?, &capture)?;
            self.floor.retain_anchor(plan.anchor)?;
            let association = Association {
                key: key.clone(),
                semantic: plan.digest,
                original_object,
                step: StepId::ZERO,
                anchor: plan.anchor,
                source: route.source_digest(),
                source_identity: route.source_identity_digest(),
                native: route.native_plan().digest(),
                witness: capture.witness(),
            };
            if self
                .associations
                .get(key)
                .is_some_and(|old| old != &association)
            {
                return Err(error(
                    ErrorCode::Conflict,
                    "original dig key already binds another semantic source",
                ));
            }
            let review = SemanticDigReview {
                seal: association.seal(inventory.journal_id),
                original: plan,
                native: route.native_plan().clone(),
                association,
            };
            let mut final_context = final_context;
            final_context.budget.max_wall_millis = call.current()?.budget.max_wall_millis;
            self.view(&mut Call::new(&final_context)?)?;
            authorize_original(&review.original, &call.current()?)?;
            self.goal_owner = None;
            self.attached = Some(review.clone());
            Ok(review)
        })();
        if result.is_err() {
            self.abandon_preparation();
        }
        result
    }
    /// Reattachment restores historical evidence, never the native one-shot
    /// preparation connection. It cannot renew expiry or authorize designation.
    pub fn reattach(
        &mut self,
        plan: PreparedPlan,
        c: &OperationContext,
    ) -> Result<SemanticDigReview> {
        self.attached = None;
        self.abandon_preparation();
        let mut call = Call::new(c)?;
        call.reserve(LOCAL_BYTES)?;
        let original_object = original(&plan, &call)?;
        let inventory = self.view(&mut call)?;
        let key = &plan.steps[0].idempotency_key;
        let association = self.associations.get(key).ok_or_else(custody)?.clone();
        if association.semantic != plan.digest
            || association.original_object != original_object
            || association.anchor != plan.anchor
            || association.step != StepId::ZERO
        {
            return Err(error(
                ErrorCode::Conflict,
                "reattachment changed the exact original semantic dig plan",
            ));
        }
        let native = inventory.entry(key).ok_or_else(|| error(ErrorCode::Conflict,
            "semantic association precedes native preparation; observe the exact original plan again"))?
            .plan().clone();
        let review = SemanticDigReview {
            seal: association.seal(inventory.journal_id),
            original: plan,
            native,
            association,
        };
        self.goal_owner = None;
        self.attached = Some(review.clone());
        Ok(review)
    }
    pub fn prepare<E: DigEvidenceOwner, G: DigSessionGuard>(
        &mut self,
        seal: Digest32,
        evidence: &mut E,
        c: &OperationContext,
        guard: &mut G,
    ) -> Result<SemanticDigResult> {
        let review = self.attached(seal)?;
        let result = (|| {
            let mut call = Call::new(c)?;
            call.reserve(LOCAL_BYTES)?;
            let inventory = self.view(&mut call)?;
            self.writable(true)?;
            authorize_original(&review.original, &call.current()?)?;
            self.floor.permit_anchor(review.original.anchor)?;
            if let Some(entry) = inventory.entry(review.native.key()) {
                // Exact retained evidence cannot renew a preparation connection
                // or permit. A retry in this opening also cannot consume it.
                self.exact(&review)?;
                return self.finish(review, entry.clone(), &mut call);
            }
            self.floor.permit_native(NativeClock::observed(
                review.native.before(),
                review.original.anchor.cursor.epoch,
            ))?;
            if inventory.pending_count() != 0 {
                return Err(error(
                    ErrorCode::EffectIndeterminate,
                    "retained dig is unresolved; recover it before preparing another designation",
                ));
            }
            let guard_context = call.reserve(
                GUARD_BYTES
                    .checked_add(evidence.refresh_reserve_bytes()?)
                    .ok_or_else(exhausted)?,
            )?;
            let native_context = call.reserve(self.native.operation_reserve(false))?;
            let final_context = call.reserve(self.view_reserve())?;
            // Immutable original association is durable before native Intent or
            // preparation. Exact retries cannot bind the key to another plan.
            self.associations
                .retain(review.association.clone(), &call.reserve(LOCAL_BYTES)?)?;
            let mut guarded = SemanticGuard {
                custody: CustodyGuard::new(
                    &review,
                    &self.floor,
                    &mut self.associations,
                    guard,
                    &guard_context,
                    true,
                )?,
                evidence,
                refreshed: false,
            };
            let mut native_context = native_context;
            native_context.budget.max_wall_millis = call.current()?.budget.max_wall_millis;
            let entry = self.native.prepare(
                review.native.key(),
                false,
                review.native.before().witness(),
                &native_context,
                &mut guarded,
            )?;
            let mut final_context = final_context;
            final_context.budget.max_wall_millis = call.current()?.budget.max_wall_millis;
            self.finish(review, entry, &mut Call::new(&final_context)?)
        })();
        if result.is_err() {
            self.abandon_preparation();
        }
        result
    }
    pub fn commit<E: DigEvidenceOwner, G: DigSessionGuard>(
        &mut self,
        seal: Digest32,
        confirmed: bool,
        evidence: &mut E,
        c: &OperationContext,
        guard: &mut G,
    ) -> Result<SemanticDigResult> {
        let review = self.attached(seal)?;
        if !confirmed {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "confirm the exact paired semantic/native dig seal",
            ));
        }
        let result = (|| {
            let mut call = Call::new(c)?;
            call.reserve(LOCAL_BYTES)?;
            let inventory = self.view(&mut call)?;
            self.writable(true)?;
            self.exact(&review)?;
            authorize_original(&review.original, &call.current()?)?;
            self.floor.permit_anchor(review.original.anchor)?;
            if let Some(entry) = inventory.entry(review.native.key())
                && !entry.needs_reconciliation()
            {
                return self.finish(review, entry.clone(), &mut call);
            }
            self.floor.permit_native(NativeClock::observed(
                review.native.before(),
                review.original.anchor.cursor.epoch,
            ))?;
            let guard_context = call.reserve(
                GUARD_BYTES
                    .checked_add(evidence.refresh_reserve_bytes()?)
                    .ok_or_else(exhausted)?,
            )?;
            let native_context = call.reserve(self.native.operation_reserve(false))?;
            let final_context = call.reserve(self.view_reserve())?;
            let mut guarded = SemanticGuard {
                custody: CustodyGuard::new(
                    &review,
                    &self.floor,
                    &mut self.associations,
                    guard,
                    &guard_context,
                    true,
                )?,
                evidence,
                refreshed: false,
            };
            let mut native_context = native_context;
            native_context.budget.max_wall_millis = call.current()?.budget.max_wall_millis;
            let entry = self.native.commit(
                review.native.key(),
                review.native.digest(),
                &native_context,
                &mut guarded,
            )?;
            let mut final_context = final_context;
            final_context.budget.max_wall_millis = call.current()?.budget.max_wall_millis;
            self.finish(review, entry, &mut Call::new(&final_context)?)
        })();
        if result.is_err() {
            self.abandon_preparation();
        }
        result
    }
    pub fn inspect(&mut self, seal: Digest32, c: &OperationContext) -> Result<SemanticDigResult> {
        let review = self.attached(seal)?;
        let mut call = Call::new(c)?;
        call.reserve(LOCAL_BYTES)?;
        let inventory = self.view(&mut call)?;
        let entry = inventory
            .entry(review.native.key())
            .ok_or_else(custody)?
            .clone();
        self.finish(review, entry, &mut call)
    }
    pub fn reconcile<F, G>(
        &mut self,
        seal: Digest32,
        c: &OperationContext,
        factory: F,
        guard: &mut G,
    ) -> Result<SemanticDigResult>
    where
        F: FnOnce(&DigBinding, DigRegion, &OperationContext) -> Result<N>,
        G: DigSessionGuard,
    {
        self.recover(seal, false, c, factory, guard)
    }
    /// Retires preparation under current Query and Designate authority, even
    /// after original plan expiry. It never clears designations or terrain.
    pub fn cancel<F, G>(
        &mut self,
        seal: Digest32,
        c: &OperationContext,
        factory: F,
        guard: &mut G,
    ) -> Result<SemanticDigResult>
    where
        F: FnOnce(&DigBinding, DigRegion, &OperationContext) -> Result<N>,
        G: DigSessionGuard,
    {
        self.recover(seal, true, c, factory, guard)
    }
    fn recover<F, G>(
        &mut self,
        seal: Digest32,
        cancel: bool,
        c: &OperationContext,
        factory: F,
        guard: &mut G,
    ) -> Result<SemanticDigResult>
    where
        F: FnOnce(&DigBinding, DigRegion, &OperationContext) -> Result<N>,
        G: DigSessionGuard,
    {
        let review = self.attached(seal)?;
        self.abandon_preparation();
        let mut call = Call::new(c)?;
        call.reserve(LOCAL_BYTES)?;
        self.view(&mut call)?;
        self.writable(false)?;
        self.exact(&review)?;
        let guard_context = call.reserve(GUARD_BYTES)?;
        let native_context = call.reserve(self.native.operation_reserve(true))?;
        let final_context = call.reserve(self.view_reserve())?;
        let mut guarded = CustodyGuard::new(
            &review,
            &self.floor,
            &mut self.associations,
            guard,
            &guard_context,
            false,
        )?;
        let mut native_context = native_context;
        native_context.budget.max_wall_millis = call.current()?.budget.max_wall_millis;
        let entry = if cancel {
            self.native.cancel(
                review.native.key(),
                review.native.digest(),
                &native_context,
                factory,
                &mut guarded,
            )?
        } else {
            self.native.reconcile(
                review.native.key(),
                review.native.digest(),
                &native_context,
                factory,
                &mut guarded,
            )?
        };
        let mut final_context = final_context;
        final_context.budget.max_wall_millis = call.current()?.budget.max_wall_millis;
        self.finish(review, entry, &mut Call::new(&final_context)?)
    }
}

struct CustodyGuard<'a, B, G> {
    review: &'a SemanticDigReview,
    floor: &'a SourceFloor,
    associations: &'a mut AssociationStore<B>,
    policy: &'a mut G,
    call: Call,
    control: bool,
}
impl<'a, B: EffectJournalStorage, G: DigSessionGuard> CustodyGuard<'a, B, G> {
    fn new(
        review: &'a SemanticDigReview,
        floor: &'a SourceFloor,
        associations: &'a mut AssociationStore<B>,
        policy: &'a mut G,
        c: &OperationContext,
        control: bool,
    ) -> Result<Self> {
        Ok(Self {
            review,
            floor,
            associations,
            policy,
            call: Call::new(c)?,
            control,
        })
    }
    fn authority(&mut self, c: &OperationContext) -> Result<OperationContext> {
        self.call.boundary(c)?;
        self.floor.authority(&mut self.call)?;
        let current = self.call.current()?;
        if self.control {
            self.floor.permit_anchor(self.review.original.anchor)?;
            self.floor.permit_native(NativeClock::observed(
                self.review.native.before(),
                self.review.original.anchor.cursor.epoch,
            ))?;
            authorize_original(&self.review.original, &current)?;
        }
        Ok(current)
    }
    fn verify(&mut self, c: &OperationContext) -> Result<()> {
        self.authority(c)?;
        self.associations.verify(&self.call.reserve(STORE_BYTES)?)?;
        if self.associations.get(self.review.native.key()) != Some(&self.review.association) {
            return Err(custody());
        }
        Ok(())
    }
    fn connection(
        &mut self,
        binding: &DigBinding,
        region: DigRegion,
        c: &OperationContext,
        observe: bool,
    ) -> Result<()> {
        if binding.fortress_id() != self.review.original.anchor.fortress_id
            || binding.manifest().generation != self.review.native.before().generation()
            || binding.folder() != self.review.native.before().folder()
            || binding.site() != self.review.native.before().site()
            || region != self.review.native.before().region()
        {
            return Err(custody());
        }
        self.verify(c)?;
        let current = self.authority(c)?;
        if observe {
            self.policy.observe(binding, region, &current)?;
        } else {
            self.policy.connect(binding, region, &current)?;
        }
        self.verify(c)
    }
}
impl<B: EffectJournalStorage, G: DigSessionGuard> DigGuard for CustodyGuard<'_, B, G> {
    fn check(&mut self, stage: DigStage, plan: &DigPlan, c: &OperationContext) -> Result<()> {
        if plan != &self.review.native {
            return Err(custody());
        }
        self.verify(c)?;
        let current = self.authority(c)?;
        self.policy.check(stage, plan, &current)?;
        self.verify(c)
    }
}
impl<B: EffectJournalStorage, G: DigSessionGuard> DigSessionGuard for CustodyGuard<'_, B, G> {
    fn connect(
        &mut self,
        binding: &DigBinding,
        region: DigRegion,
        c: &OperationContext,
    ) -> Result<()> {
        self.connection(binding, region, c, false)
    }
    fn observe(
        &mut self,
        binding: &DigBinding,
        region: DigRegion,
        c: &OperationContext,
    ) -> Result<()> {
        self.connection(binding, region, c, true)
    }
}
struct SemanticGuard<'a, B, E, G> {
    custody: CustodyGuard<'a, B, G>,
    evidence: &'a mut E,
    refreshed: bool,
}
impl<B: EffectJournalStorage, E: DigEvidenceOwner, G: DigSessionGuard> DigGuard
    for SemanticGuard<'_, B, E, G>
{
    fn check(&mut self, stage: DigStage, plan: &DigPlan, c: &OperationContext) -> Result<()> {
        self.custody.check(stage, plan, c)?;
        if matches!(stage, DigStage::Prepare | DigStage::Commit) && !self.refreshed {
            // First Prepare follows durable Intent; first Commit follows
            // durable DispatchStarted. Native rechecks custody after this edge.
            self.refreshed = true;
            let review = self.custody.review;
            let current = self
                .custody
                .call
                .reserve(self.evidence.refresh_reserve_bytes()?)?;
            self.evidence
                .refresh_after_capture(review.native.before(), &current)?;
            let route = resolve_dig_step(
                &review.original,
                StepId::ZERO,
                &self.evidence.routing_evidence()?,
                review.native.before(),
            )?;
            if route.native_plan() != &review.native
                || route.source_digest() != review.association.source
                || route.source_identity_digest() != review.association.source_identity
            {
                return Err(custody());
            }
            self.custody.check(stage, plan, c)?;
        }
        Ok(())
    }
}
impl<B: EffectJournalStorage, E: DigEvidenceOwner, G: DigSessionGuard> DigSessionGuard
    for SemanticGuard<'_, B, E, G>
{
    fn connect(
        &mut self,
        binding: &DigBinding,
        region: DigRegion,
        c: &OperationContext,
    ) -> Result<()> {
        self.custody.connect(binding, region, c)
    }
    fn observe(
        &mut self,
        binding: &DigBinding,
        region: DigRegion,
        c: &OperationContext,
    ) -> Result<()> {
        self.custody.observe(binding, region, c)
    }
}
