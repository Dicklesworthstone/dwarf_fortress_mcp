//! One original semantic furniture plan through exact native placement custody.
//!
//! A paired review preserves the original action, predicates, obligation and
//! archive identity. A native Placed receipt proves insertion only; construction
//! completion still needs the original goal and fresh independent evidence.
//! No source grant, admission, checkpoint or canonical identity is issued here.

use std::sync::Arc;
use std::time::{Duration, Instant};

use dfmcp_core::{
    Capability, Digest32, ErrorCode, GameTick, OperationContext, Result, RiskTier, StateAnchor,
    StepId,
};
use dfmcp_intent::{Action, BuildingKind, MaterialSelector, ObligationSpec, PreparedPlan};
use dfmcp_world::Predicate;

use crate::build_placement::journal::{
    BuildEntry, BuildGuard, BuildInventory, BuildMode, BuildSource, BuildStage,
};
use crate::build_placement::session::BuildSession;
use crate::build_placement::{
    BuildBinding, BuildCapture, BuildPhase, BuildPlan, BuildRecord, BuildSelection,
};
use crate::control_effect_journal::EffectJournalStorage;
use crate::live_routing::{LiveRoutingEvidence, resolve_furniture_step};

pub mod evidence_owner;
pub mod goal_history;
pub mod goal_monitor;
pub mod goal_projection;
pub mod store;
pub use goal_history::{
    DurableFurnitureGoalMonitor, GoalHistoryStore, PrivateGoalHistoryStore,
    open_private_goal_history_store,
};
pub use goal_monitor::{
    FurnitureGoalMonitor, FurnitureGoalProgress, FurnitureGoalResult, FurnitureGoalSource,
    NativeFurnitureGoalSource,
};
pub use goal_projection::FurnitureGoalProjection;
use store::{Association, MAX_ASSOCIATIONS, MAX_STORE_BYTES};
pub use store::{AssociationStore, PrivateAssociationStore, open_private_association_store};

/// The trusted shell acquires a complete independently qualified spatial source.
/// The native capture constrains that source; it never becomes source authority.
pub trait FurnitureEvidenceOwner {
    fn refresh_after_capture(
        &mut self,
        capture: &BuildCapture,
        context: &OperationContext,
    ) -> Result<()>;
    fn routing_evidence(&self) -> Result<LiveRoutingEvidence<'_>>;
    /// Complete bounded acquisition/projection cost, prepaid independently from
    /// the native RPC allowance. The caller's operation budget is never renewed.
    fn refresh_reserve_bytes(&self) -> Result<u64> {
        Ok(16 * 1024 * 1024)
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
        "semantic furniture handoff exhausted its bounded allowance",
    )
}
fn custody() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::CorruptLedger,
        "semantic/native furniture custody differs; retain history and do not redispatch",
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
            "semantic furniture requires one original step and cannot supply a mandatory checkpoint",
        ));
    }
    let step = &plan.steps[0];
    let Action::Build {
        kind: BuildingKind::Furniture(name),
        material,
        ..
    } = &step.action
    else {
        return Err(refused(
            "semantic furniture supports the original single-tile Build action only",
        ));
    };
    // The existing closed resolver owns its accepted furniture spellings.
    // Preserve the exact original name; never normalize or reseal it here.
    if name.len() > 256
        || material != &MaterialSelector::default()
        || !step.depends_on.is_empty()
        || step.compensation.is_some()
        || step.preconditions.len() > 64
        || step.postconditions.len() > 64
        || step.preconditions.iter().any(|p| !protected(p))
    {
        return Err(refused(
            "native furniture cannot drop selectors, dependencies, compensation or unprotected preconditions",
        ));
    }
    let route = crate::live_routing::route_step(step)
        .outcome
        .map_err(|r| refused(&r.reason))?;
    if !matches!(
        route.request,
        crate::live_routing::LiveRequest::Furniture { .. }
    ) {
        return Err(refused(
            "original action has no closed native furniture recipe",
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
            "semantic furniture authority belongs to another fortress",
        ));
    }
    c.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
    c.authorize(Capability::Construct, RiskTier::Guarded, &[], None)
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
            "original semantic furniture anchor or deadline no longer permits control",
        ));
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct SemanticFurnitureReview {
    original: PreparedPlan,
    native: BuildPlan,
    association: Association,
    seal: Digest32,
}
impl SemanticFurnitureReview {
    pub fn original_plan(&self) -> &PreparedPlan {
        &self.original
    }
    pub fn native_plan(&self) -> &BuildPlan {
        &self.native
    }
    pub const fn seal(&self) -> Digest32 {
        self.seal
    }
    pub fn original_object_digest(&self) -> Digest32 {
        self.association.original_object
    }
}

/// Historical insertion readback. It cannot establish completion, current
/// physical truth or any semantic postcondition merely by being retained.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeFurniturePlacement {
    Unverified,
    Placed {
        native_building_id: u32,
        native_job_id: u32,
        native_item_id: u32,
        receipt: Digest32,
    },
}
#[derive(Clone, Debug)]
pub struct SemanticFurnitureResult {
    review: SemanticFurnitureReview,
    native: BuildEntry,
    placement: NativeFurniturePlacement,
}
impl SemanticFurnitureResult {
    pub fn review(&self) -> &SemanticFurnitureReview {
        &self.review
    }
    pub fn native_record(&self) -> &BuildEntry {
        &self.native
    }
    pub fn placement(&self) -> &NativeFurniturePlacement {
        &self.placement
    }
    pub fn placement_result(&self) -> &NativeFurniturePlacement {
        &self.placement
    }
    pub const fn original_goal_proven(&self) -> bool {
        false
    }
    pub fn pending_obligation(&self) -> Option<&ObligationSpec> {
        self.review.original.steps[0].obligation.as_ref()
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
    fn observed(observation: &BuildCapture, epoch: u64) -> Self {
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
                "original furniture source predates or forks retained canonical evidence",
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
                "native furniture capture predates retained evidence or changes incarnation",
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
pub struct SemanticFurnitureSession<S, B, N> {
    native: BuildSession<S, N>,
    associations: AssociationStore<B>,
    attached: Option<SemanticFurnitureReview>,
    floor: SourceFloor,
    goal_owner: Option<Arc<()>>,
    goal_floor: goal_monitor::FurnitureSourceFloor,
}
impl<S: EffectJournalStorage, B: EffectJournalStorage, N: BuildSource>
    SemanticFurnitureSession<S, B, N>
{
    pub fn new(
        native: BuildSession<S, N>,
        associations: AssociationStore<B>,
        c: &OperationContext,
    ) -> Result<Self> {
        let mut out = Self {
            native,
            associations,
            attached: None,
            floor: SourceFloor::default(),
            goal_owner: None,
            goal_floor: goal_monitor::FurnitureSourceFloor::default(),
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
        self.native.operation_reserve(false)
            + 2 * STORE_BYTES
            + 8 * 32 * crate::build_placement::journal::MAX_FRAME_BYTES as u64
    }
    fn view(&mut self, call: &mut Call) -> Result<BuildInventory> {
        self.floor.retain_tick(
            call.context.anchor.cursor.epoch,
            GameTick(self.native.high_tick()),
        )?;
        self.floor.authority(call)?;
        self.associations.verify(&call.reserve(STORE_BYTES)?)?;
        let mut retained = SourceFloor::default();
        let mut anchors = self.associations.anchors()?;
        anchors.sort_by_key(|a| (a.cursor.epoch, a.cursor.sequence));
        for anchor in anchors {
            retained.retain_anchor(anchor)?;
        }
        if let Some(anchor) = retained.canonical {
            self.floor.retain_anchor(anchor)?;
        }
        self.floor.authority(call)?;
        let inventory = self
            .native
            .inventory(&call.reserve(self.native.operation_reserve(false))?)?;
        if inventory.journal_id != self.associations.native_journal()
            || inventory.total_records() > MAX_ASSOCIATIONS
        {
            return Err(custody());
        }
        let mut clocks = Vec::with_capacity(inventory.total_records());
        for entry in inventory.entries() {
            let association = self
                .associations
                .get(entry.plan().key())
                .ok_or_else(custody)?;
            if association.native != entry.plan().digest()
                || association.witness != entry.plan().before().witness()
                || association.anchor.fortress_id != self.native.binding().fortress().fortress_id()
                || association.step != StepId::ZERO
            {
                return Err(custody());
            }
            let capture = match entry.native().and_then(BuildRecord::after) {
                Some(after) => after,
                None => entry.plan().before(),
            };
            clocks.push(NativeClock::observed(
                capture,
                association.anchor.cursor.epoch,
            ));
        }
        // Compare historical captures before merging the runtime maximum, so a
        // newer valid observation cannot conceal a fork in retained history.
        clocks.sort_by_key(|c| (c.epoch, c.generation, c.sequence, c.tick));
        for clock in clocks {
            retained.retain_native(clock)?;
        }
        if let Some(clock) = retained.native {
            self.floor.retain_native(clock)?;
        }
        self.floor.authority(call)?;
        self.associations.verify(&call.reserve(STORE_BYTES)?)?;
        Ok(inventory)
    }
    fn writable(&self, control: bool) -> Result<()> {
        if self.associations.is_read_only()
            || self.native.mode() == BuildMode::Offline
            || (control && self.native.mode() != BuildMode::Control)
        {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "offline or recovery furniture custody cannot authorize this native operation",
            ));
        }
        Ok(())
    }
    fn attached(&self, seal: Digest32) -> Result<SemanticFurnitureReview> {
        self.attached
            .as_ref()
            .filter(|r| r.seal == seal)
            .cloned()
            .ok_or_else(|| {
                error(
                    ErrorCode::Conflict,
                    "attach the exact original furniture plan and paired review seal",
                )
            })
    }
    fn exact(&self, review: &SemanticFurnitureReview) -> Result<()> {
        if self.associations.get(review.native.key()) != Some(&review.association) {
            return Err(custody());
        }
        Ok(())
    }
    fn finish(
        &mut self,
        review: SemanticFurnitureReview,
        native: BuildEntry,
        call: &mut Call,
    ) -> Result<SemanticFurnitureResult> {
        let inventory = self.view(call)?;
        self.exact(&review)?;
        if native.plan() != &review.native || inventory.entry(native.plan().key()) != Some(&native)
        {
            return Err(custody());
        }
        let placement = match native.native() {
            Some(record) if record.phase() == BuildPhase::Placed => {
                let decoded = BuildRecord::decode(record.canonical_bytes())?;
                decoded.verify_plan(&review.native)?;
                let insertion = decoded.insertion().ok_or_else(custody)?;
                NativeFurniturePlacement::Placed {
                    native_building_id: insertion.building_id(),
                    native_job_id: insertion.job_id(),
                    native_item_id: insertion.item_id(),
                    receipt: decoded.receipt(),
                }
            }
            _ => NativeFurniturePlacement::Unverified,
        };
        call.current()?;
        Ok(SemanticFurnitureResult {
            review,
            native,
            placement,
        })
    }
    pub fn inventory(&mut self, c: &OperationContext) -> Result<BuildInventory> {
        self.view(&mut Call::new(c)?)
    }
    pub fn observe<E, F, G>(
        &mut self,
        plan: PreparedPlan,
        selection: BuildSelection,
        evidence: &mut E,
        c: &OperationContext,
        factory: F,
        guard: &mut G,
    ) -> Result<SemanticFurnitureReview>
    where
        E: FurnitureEvidenceOwner,
        F: FnOnce(&BuildBinding, &OperationContext, Duration) -> Result<N>,
        G: BuildGuard,
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
            if inventory.entry(key).is_some() {
                return Err(error(
                    ErrorCode::Conflict,
                    "native key exists; reattach only its exact original furniture plan",
                ));
            }
            // Prepay everything remaining before connecting; each subsystem gets
            // its own reservation from this one monotonically shrinking call.
            let evidence_context = call.reserve(evidence.refresh_reserve_bytes()?)?;
            let final_context = call.reserve(self.view_reserve())?;
            let native_context = call.reserve(self.native.operation_reserve(true))?;
            let captured = self
                .native
                .observe(selection, &native_context, factory, guard);
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
            let route = resolve_furniture_step(
                &plan,
                StepId::ZERO,
                &evidence.routing_evidence()?,
                &capture,
            )?;
            self.floor.retain_anchor(plan.anchor)?;
            let association = Association {
                key: key.clone(),
                semantic: plan.digest,
                original_object,
                step: StepId::ZERO,
                anchor: plan.anchor,
                source: route.source_digest(),
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
                    "original furniture key already binds another semantic source",
                ));
            }
            let review = SemanticFurnitureReview {
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
    /// preparation connection. It cannot renew expiry or authorize placement.
    pub fn reattach(
        &mut self,
        plan: PreparedPlan,
        c: &OperationContext,
    ) -> Result<SemanticFurnitureReview> {
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
                "reattachment changed the exact original semantic furniture plan",
            ));
        }
        let native = inventory.entry(key).ok_or_else(|| error(ErrorCode::Conflict,
            "semantic association precedes native preparation; observe the exact original plan again"))?
            .plan().clone();
        let review = SemanticFurnitureReview {
            seal: association.seal(inventory.journal_id),
            original: plan,
            native,
            association,
        };
        self.goal_owner = None;
        self.attached = Some(review.clone());
        Ok(review)
    }
    pub fn prepare<E: FurnitureEvidenceOwner, G: BuildGuard>(
        &mut self,
        seal: Digest32,
        evidence: &mut E,
        c: &OperationContext,
        guard: &mut G,
    ) -> Result<SemanticFurnitureResult> {
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
                    "retained furniture is unresolved; recover it before preparing another placement",
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
                prepare_refreshed: false,
            };
            let mut native_context = native_context;
            native_context.budget.max_wall_millis = call.current()?.budget.max_wall_millis;
            let entry = self.native.prepare(
                review.native.key(),
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
    pub fn commit<E: FurnitureEvidenceOwner, G: BuildGuard>(
        &mut self,
        seal: Digest32,
        confirmed: bool,
        evidence: &mut E,
        c: &OperationContext,
        guard: &mut G,
    ) -> Result<SemanticFurnitureResult> {
        let review = self.attached(seal)?;
        if !confirmed {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "confirm the exact paired semantic/native furniture seal",
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
                prepare_refreshed: false,
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
    pub fn inspect(
        &mut self,
        seal: Digest32,
        c: &OperationContext,
    ) -> Result<SemanticFurnitureResult> {
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
    ) -> Result<SemanticFurnitureResult>
    where
        F: FnOnce(&BuildBinding, &OperationContext, Duration) -> Result<N>,
        G: BuildGuard,
    {
        self.recover(seal, false, c, factory, guard)
    }
    /// Retires native preparation under current Query authority even after the
    /// original plan or construction grant expires. It never removes furniture.
    pub fn cancel<F, G>(
        &mut self,
        seal: Digest32,
        c: &OperationContext,
        factory: F,
        guard: &mut G,
    ) -> Result<SemanticFurnitureResult>
    where
        F: FnOnce(&BuildBinding, &OperationContext, Duration) -> Result<N>,
        G: BuildGuard,
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
    ) -> Result<SemanticFurnitureResult>
    where
        F: FnOnce(&BuildBinding, &OperationContext, Duration) -> Result<N>,
        G: BuildGuard,
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
        let entry = self.native.recover(
            review.native.key(),
            review.native.digest(),
            cancel,
            &native_context,
            factory,
            &mut guarded,
        )?;
        let mut final_context = final_context;
        final_context.budget.max_wall_millis = call.current()?.budget.max_wall_millis;
        self.finish(review, entry, &mut Call::new(&final_context)?)
    }
}

struct CustodyGuard<'a, B, G> {
    review: &'a SemanticFurnitureReview,
    floor: &'a SourceFloor,
    associations: &'a mut AssociationStore<B>,
    policy: &'a mut G,
    call: Call,
    control: bool,
}
impl<'a, B: EffectJournalStorage, G: BuildGuard> CustodyGuard<'a, B, G> {
    fn new(
        review: &'a SemanticFurnitureReview,
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
    fn pre(
        &mut self,
        binding: &BuildBinding,
        plan: Option<&BuildPlan>,
        selection: BuildSelection,
        c: &OperationContext,
    ) -> Result<OperationContext> {
        if plan != Some(&self.review.native)
            || selection != self.review.native.before().selection()
            || !binding.capture_matches(self.review.native.before())
        {
            return Err(custody());
        }
        self.verify(c)?;
        self.authority(c)
    }
}
impl<B: EffectJournalStorage, G: BuildGuard> BuildGuard for CustodyGuard<'_, B, G> {
    fn check(
        &mut self,
        stage: BuildStage,
        binding: &BuildBinding,
        plan: Option<&BuildPlan>,
        selection: BuildSelection,
        c: &OperationContext,
    ) -> Result<()> {
        let current = self.pre(binding, plan, selection, c)?;
        self.policy
            .check(stage, binding, plan, selection, &current)?;
        self.verify(c)
    }
    fn before_dispatch(
        &mut self,
        binding: &BuildBinding,
        plan: &BuildPlan,
        c: &OperationContext,
    ) -> Result<()> {
        let current = self.pre(binding, Some(plan), plan.before().selection(), c)?;
        self.policy.before_dispatch(binding, plan, &current)?;
        self.verify(c)
    }
}

struct SemanticGuard<'a, B, E, G> {
    custody: CustodyGuard<'a, B, G>,
    evidence: &'a mut E,
    prepare_refreshed: bool,
}
impl<B: EffectJournalStorage, E: FurnitureEvidenceOwner, G: BuildGuard> SemanticGuard<'_, B, E, G> {
    fn refresh(&mut self, c: &OperationContext) -> Result<()> {
        self.custody.authority(c)?;
        let review = self.custody.review;
        let current = self
            .custody
            .call
            .reserve(self.evidence.refresh_reserve_bytes()?)?;
        self.evidence
            .refresh_after_capture(review.native.before(), &current)?;
        let route = resolve_furniture_step(
            &review.original,
            StepId::ZERO,
            &self.evidence.routing_evidence()?,
            review.native.before(),
        )?;
        if route.native_plan() != &review.native
            || route.source_digest() != review.association.source
        {
            return Err(custody());
        }
        self.custody.verify(c)
    }
}
impl<B: EffectJournalStorage, E: FurnitureEvidenceOwner, G: BuildGuard> BuildGuard
    for SemanticGuard<'_, B, E, G>
{
    fn check(
        &mut self,
        stage: BuildStage,
        binding: &BuildBinding,
        plan: Option<&BuildPlan>,
        selection: BuildSelection,
        c: &OperationContext,
    ) -> Result<()> {
        self.custody.check(stage, binding, plan, selection, c)?;
        if stage == BuildStage::Prepare && !self.prepare_refreshed {
            // This first Prepare edge follows durable native Intent. Later
            // Prepare/post-call checks preserve custody without another capture.
            self.prepare_refreshed = true;
            self.refresh(c)?;
            self.custody.check(stage, binding, plan, selection, c)?;
        }
        Ok(())
    }
    fn before_dispatch(
        &mut self,
        binding: &BuildBinding,
        plan: &BuildPlan,
        c: &OperationContext,
    ) -> Result<()> {
        self.custody
            .pre(binding, Some(plan), plan.before().selection(), c)?;
        self.refresh(c)?;
        // Recheck independent runtime/lease/checkpoint policy after acquisition,
        // then the native journal re-verifies DispatchStarted before writing.
        self.custody.before_dispatch(binding, plan, c)
    }
}
