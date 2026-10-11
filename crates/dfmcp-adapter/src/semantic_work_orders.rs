//! Preserve one original semantic production plan through native work-order
//! creation and restart. Native Created means insertion, never original-goal proof.
//!
//! This owner adds no bridge protocol, source grant, canonical ID issuer or MCP
//! route. Its only native effect path is the existing guarded WorkOrderSession.

use std::time::{Duration, Instant};

use dfmcp_core::{
    Capability, Digest32, ErrorCode, GameTick, OperationContext, Result, RiskTier, StateAnchor,
    StepId,
};
use dfmcp_intent::{Action, ObligationSpec, PreparedPlan};
use dfmcp_world::Predicate;

use crate::control_effect_journal::EffectJournalStorage;
use crate::live_routing::{LiveRoutingEvidence, resolve_work_order_step};
use crate::work_order_control::session::WorkOrderSession;
use crate::work_order_control::{
    CreationRecord, CreationState, CreationSummary, JournalMode, MAX_RECORD_BYTES,
    RPC_RESERVE_BYTES, WorkOrderSource,
};
use crate::work_orders::{WorkOrderObservation, WorkOrderPlan, WorkOrderState};

pub mod store;
use store::{Association, MAX_ASSOCIATIONS, MAX_STORE_BYTES};
pub use store::{AssociationStore, PrivateAssociationStore, open_private_association_store};

/// The observing shell owns fresh acquisition and independent source authority.
/// An exact queue capture is an input to refresh, never a policy or canonical
/// observation supplied by an MCP caller.
pub trait WorkOrderEvidenceOwner {
    fn refresh_after_observation(
        &mut self,
        observation: &WorkOrderObservation,
        context: &OperationContext,
    ) -> Result<()>;
    fn routing_evidence(&self) -> Result<LiveRoutingEvidence<'_>>;
}

// Reuse the concrete complete-roster acquisition owner and its independently
// provided policy issuer. Resolution below checks the queue's exact source and
// paused tick against the refreshed typed projection.
impl<F, A, S> WorkOrderEvidenceOwner
    for crate::semantic_workforce::evidence_owner::CitizenWorkforceEvidenceOwner<F, A>
where
    S: crate::semantic_workforce::evidence_owner::CitizenEvidenceSource,
    F: FnMut(&OperationContext) -> Result<S>,
    A: FnMut(
        &crate::LiveObservationCapsule,
        &crate::live_projection::LiveWorldProjection,
        &OperationContext,
    ) -> Result<dfmcp_world::EvidencePolicy>,
{
    fn refresh_after_observation(
        &mut self,
        _: &WorkOrderObservation,
        context: &OperationContext,
    ) -> Result<()> {
        self.refresh::<S>(context)
    }
    fn routing_evidence(&self) -> Result<LiveRoutingEvidence<'_>> {
        crate::semantic_workforce::WorkforceEvidenceOwner::routing_evidence(self)
    }
}

const MAX_ORIGINAL_BYTES: usize = 64 * 1024;
const MAX_ORIGINAL_PREDICATES: usize = 512;
const EVIDENCE_BYTES: u64 = 16 * 1024 * 1024;
const LOCAL_BYTES: u64 = 1024 * 1024;
const STORE_BYTES: u64 = MAX_STORE_BYTES as u64;
const GUARD_BYTES: u64 = EVIDENCE_BYTES + 2 * STORE_BYTES + LOCAL_BYTES + 1;
const VIEW_BYTES: u64 = 2 * MAX_ASSOCIATIONS as u64 * MAX_RECORD_BYTES as u64 + 4096;
const VIEW_COST: u64 = 2 * STORE_BYTES + VIEW_BYTES;
const NATIVE_BYTES: u64 = RPC_RESERVE_BYTES + 4 * MAX_RECORD_BYTES as u64 + 4096;

fn error(code: ErrorCode, message: &str) -> dfmcp_core::DfmcpError {
    dfmcp_core::DfmcpError::new(code, message)
}
fn exhausted() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "semantic work-order handoff exhausted its bounded allowance",
    )
}
fn custody() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::CorruptLedger,
        "semantic/native work-order custody differs; retain history and do not redispatch",
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
fn original(plan: &PreparedPlan) -> Result<()> {
    if plan.steps.len() != 1 || plan.summary.len() > 4096 || plan.requires_checkpoint {
        return Err(refused(
            "semantic work-order handoff requires one step and cannot provide a mandatory checkpoint",
        ));
    }
    let step = &plan.steps[0];
    let Action::CreateWorkOrder {
        name,
        job_token,
        amount: _,
        conditions,
    } = &step.action
    else {
        return Err(refused(
            "semantic work-order handoff supports CreateWorkOrder only",
        ));
    };
    if name.len() > 256
        || job_token.len() > 128
        || !conditions.is_empty()
        || !step.depends_on.is_empty()
        || step.compensation.is_some()
        || step.preconditions.len() > 64
        || step.postconditions.len() > 64
    {
        return Err(refused(
            "native work-order creation cannot drop conditions, dependencies, compensation or oversized requirements",
        ));
    }
    let mut bytes = plan.summary.len() + name.len() + job_token.len();
    let mut nodes = 0usize;
    for predicate in step
        .preconditions
        .iter()
        .chain(&step.postconditions)
        .chain(std::iter::once(&plan.terminal_condition))
        .chain(step.obligation.iter().map(|value| &value.terminal))
        .chain(
            step.obligation
                .iter()
                .filter_map(|value| value.failure.as_ref()),
        )
    {
        predicate.validate_shape()?;
        nodes = nodes
            .checked_add(predicate.complexity())
            .ok_or_else(exhausted)?;
        if nodes > MAX_ORIGINAL_PREDICATES {
            return Err(exhausted());
        }
        bytes = bytes
            .checked_add(predicate.canonical_bytes().len())
            .ok_or_else(exhausted)?;
        if bytes > MAX_ORIGINAL_BYTES {
            return Err(exhausted());
        }
    }
    plan.validate_structure()?;
    if step.preconditions.iter().any(|p| !protected(p)) {
        return Err(refused(
            "original preconditions exceed the paused state protected by the exact native queue capture",
        ));
    }
    let route = crate::live_routing::route_step(step)
        .outcome
        .map_err(|r| refused(&r.reason))?;
    if !matches!(
        route.request,
        crate::live_routing::LiveRequest::WorkOrder { .. }
    ) {
        return Err(refused(
            "original action has no closed native work-order recipe",
        ));
    }
    Ok(())
}
fn authorize_scope(plan: &PreparedPlan, c: &OperationContext) -> Result<()> {
    if c.budget.max_actions == 0 {
        return Err(exhausted());
    }
    if c.anchor.fortress_id != plan.anchor.fortress_id {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "semantic work-order authority belongs to another fortress",
        ));
    }
    c.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
    c.authorize(
        Capability::ConfigureProduction,
        RiskTier::Guarded,
        &[],
        None,
    )
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
            "original semantic work-order anchor or deadline no longer permits control",
        ));
    }
    Ok(())
}

/// Both sealed plans and their exact source/journal association, ready for review.
#[derive(Clone, Debug)]
pub struct SemanticWorkOrderReview {
    original: PreparedPlan,
    native: WorkOrderPlan,
    association: Association,
    seal: Digest32,
}
impl SemanticWorkOrderReview {
    pub fn original_plan(&self) -> &PreparedPlan {
        &self.original
    }
    pub fn native_plan(&self) -> &WorkOrderPlan {
        &self.native
    }
    pub const fn seal(&self) -> Digest32 {
        self.seal
    }
}

/// Historical native insertion readback, deliberately separate from semantic
/// postconditions, workshop activity, produced stock and the original obligation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeOrderCreation {
    Unverified,
    Inserted {
        native_order_id: u32,
        receipt: Digest32,
    },
}
#[derive(Clone, Debug)]
pub struct SemanticWorkOrderResult {
    review: SemanticWorkOrderReview,
    native: CreationRecord,
    creation: NativeOrderCreation,
}
impl SemanticWorkOrderResult {
    pub fn review(&self) -> &SemanticWorkOrderReview {
        &self.review
    }
    pub fn native_record(&self) -> &CreationRecord {
        &self.native
    }
    pub fn creation_result(&self) -> &NativeOrderCreation {
        &self.creation
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
    fn admit(&self, minimum: u64) -> Result<()> {
        self.current()?;
        if self.bytes < minimum {
            return Err(exhausted());
        }
        Ok(())
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
        self.bytes = self
            .bytes
            .checked_sub(bytes)
            .filter(|remaining| *remaining > 0)
            .ok_or_else(exhausted)?;
        c.budget.max_bytes = bytes;
        Ok(c)
    }
    fn edge(&self, reserved: &OperationContext) -> Result<OperationContext> {
        let mut current = self.current()?;
        current.budget.max_bytes = reserved.budget.max_bytes;
        Ok(current)
    }
    fn native(
        &mut self,
        observation: &WorkOrderObservation,
        guard: bool,
    ) -> Result<OperationContext> {
        let mut c = self.reserve(NATIVE_BYTES + if guard { GUARD_BYTES } else { 0 })?;
        c.anchor.state_hash = observation.witness();
        // Control entry has already required the unchanged canonical tick. Native
        // queue witnesses are never assigned a canonical observation cursor.
        if c.anchor.tick.get() != observation.tick() {
            return Err(custody());
        }
        Ok(c)
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
    fn observed(observation: &WorkOrderObservation, epoch: u64) -> Self {
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
#[derive(Default)]
struct SourceFloor {
    canonical: Option<StateAnchor>,
    native: Option<NativeClock>,
}
impl SourceFloor {
    fn retain_anchor(&mut self, anchor: StateAnchor) -> Result<()> {
        if self
            .native
            .is_some_and(|clock| clock.epoch != anchor.cursor.epoch)
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
        self.canonical = Some(anchor);
        Ok(())
    }
    fn retain_native(&mut self, clock: NativeClock) -> Result<()> {
        if self
            .canonical
            .is_some_and(|anchor| anchor.cursor.epoch != clock.epoch)
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
        Ok(())
    }
    fn permit_anchor(&self, anchor: StateAnchor) -> Result<()> {
        if self.canonical.is_some_and(|old| {
            anchor.fortress_id != old.fortress_id
                || anchor.cursor.epoch != old.cursor.epoch
                || anchor.cursor.sequence < old.cursor.sequence
                || anchor.tick < old.tick
                || (anchor.cursor.sequence == old.cursor.sequence && anchor != old)
        }) {
            return Err(error(
                ErrorCode::StaleAnchor,
                "original work-order source predates or forks retained canonical evidence",
            ));
        }
        Ok(())
    }
    fn permit_native(&self, clock: NativeClock) -> Result<()> {
        if self.native.is_some_and(|old| {
            clock.epoch != old.epoch
                || clock.generation != old.generation
                || clock.sequence < old.sequence
                || clock.tick < old.tick
        }) {
            return Err(error(
                ErrorCode::StaleAnchor,
                "native work-order capture predates retained evidence or changes incarnation",
            ));
        }
        Ok(())
    }
    fn authority(&self, call: &mut Call) -> Result<()> {
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

/// Exclusive native coordinator and separate immutable semantic custody. Opening
/// another association store cannot adopt existing native keys, even exact bytes.
pub struct SemanticWorkOrderSession<S, B, N> {
    native: WorkOrderSession<S, N>,
    associations: AssociationStore<B>,
    attached: Option<SemanticWorkOrderReview>,
    floor: SourceFloor,
}
impl<S: EffectJournalStorage, B: EffectJournalStorage, N: WorkOrderSource>
    SemanticWorkOrderSession<S, B, N>
{
    pub fn new(
        native: WorkOrderSession<S, N>,
        associations: AssociationStore<B>,
        c: &OperationContext,
    ) -> Result<Self> {
        let mut out = Self {
            native,
            associations,
            attached: None,
            floor: SourceFloor::default(),
        };
        out.view(&mut Call::new(c)?)?;
        Ok(out)
    }
    fn view(&mut self, call: &mut Call) -> Result<(CreationSummary, Vec<CreationRecord>)> {
        self.floor.authority(call)?;
        self.associations.verify(&call.reserve(STORE_BYTES)?)?;
        let mut retained = SourceFloor::default();
        let mut anchors = self.associations.anchors()?;
        anchors.sort_by_key(|anchor| (anchor.cursor.epoch, anchor.cursor.sequence));
        for anchor in anchors {
            retained.retain_anchor(anchor)?;
        }
        if let Some(anchor) = retained.canonical {
            self.floor.retain_anchor(anchor)?;
        }
        self.floor.authority(call)?;
        let c = call.reserve(VIEW_BYTES)?;
        let summary = self.native.summary(&c)?;
        if summary.journal_id != self.associations.native_journal()
            || summary.records > MAX_ASSOCIATIONS
            || summary.records > c.budget.max_entities as usize
        {
            return Err(custody());
        }
        let records = if summary.records == 0 {
            Vec::new()
        } else {
            let page =
                self.native
                    .records_page(summary.head, None, summary.records, &call.edge(&c)?)?;
            if page.next_after.is_some() || page.records.len() != summary.records {
                return Err(custody());
            }
            page.records
        };
        let mut clocks = Vec::with_capacity(records.len());
        for record in &records {
            let association = self
                .associations
                .get(record.plan().key())
                .ok_or_else(custody)?;
            if association.native != record.plan().digest()
                || association.witness != record.plan().observation().witness()
                || association.anchor.fortress_id != summary.fortress_id
                || association.step != StepId::ZERO
            {
                return Err(custody());
            }
            let mut clock =
                NativeClock::observed(record.plan().observation(), association.anchor.cursor.epoch);
            if record.state() == CreationState::Created {
                // Strict journal replay has proved exact before.sequence + 1
                // insertion readback, not a new canonical observation cursor.
                clock.sequence = clock.sequence.checked_add(1).ok_or_else(custody)?;
            }
            clocks.push(clock);
        }
        // Validate older retained captures against each other before merging the
        // maximum. A newer runtime observation must not mask a historical fork.
        clocks.sort_by_key(|clock| (clock.epoch, clock.generation, clock.sequence, clock.tick));
        for clock in clocks {
            retained.retain_native(clock)?;
        }
        if let Some(clock) = retained.native {
            self.floor.retain_native(clock)?;
        }
        self.floor.authority(call)?;
        self.associations.verify(&call.reserve(STORE_BYTES)?)?;
        Ok((summary, records))
    }
    fn observe_native(&mut self, call: &mut Call) -> Result<WorkOrderObservation> {
        let observation = self.native.observe(&call.reserve(NATIVE_BYTES)?)?;
        let clock = NativeClock::observed(&observation, call.context.anchor.cursor.epoch);
        if let Err(error) = self.floor.permit_native(clock) {
            self.native.clear_selection();
            return Err(error);
        }
        self.floor.retain_native(clock)?;
        if let Err(error) = self.floor.authority(call) {
            self.native.clear_selection();
            return Err(error);
        }
        Ok(observation)
    }
    fn writable(&self, summary: &CreationSummary) -> Result<()> {
        if self.associations.is_read_only() || summary.mode != JournalMode::Control {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "recovery or read-only semantic custody cannot control native creation",
            ));
        }
        Ok(())
    }
    fn attached(&self, seal: Digest32) -> Result<SemanticWorkOrderReview> {
        self.attached
            .as_ref()
            .filter(|r| r.seal == seal)
            .cloned()
            .ok_or_else(|| {
                error(
                    ErrorCode::Conflict,
                    "attach the exact original semantic work-order plan and paired review seal",
                )
            })
    }
    fn exact(&self, review: &SemanticWorkOrderReview) -> Result<()> {
        if self.associations.get(review.native.key()) != Some(&review.association) {
            return Err(custody());
        }
        Ok(())
    }
    fn finish(
        &mut self,
        review: SemanticWorkOrderReview,
        record: CreationRecord,
        call: &mut Call,
    ) -> Result<SemanticWorkOrderResult> {
        let (_, records) = self.view(call)?;
        self.exact(&review)?;
        if record.plan() != &review.native || !records.contains(&record) {
            return Err(custody());
        }
        let effect = crate::work_orders::WorkOrderEffect::decode(
            record.effect().canonical_bytes(),
            &review.native,
        )?;
        let creation = if record.state() == CreationState::Created
            && effect.state() == WorkOrderState::Created
        {
            NativeOrderCreation::Inserted {
                native_order_id: effect.created_order_id().ok_or_else(custody)?,
                receipt: effect.receipt().ok_or_else(custody)?,
            }
        } else {
            NativeOrderCreation::Unverified
        };
        call.current()?;
        Ok(SemanticWorkOrderResult {
            review,
            native: record,
            creation,
        })
    }
    pub fn inventory(&mut self, context: &OperationContext) -> Result<Vec<CreationRecord>> {
        self.view(&mut Call::new(context)?)
            .map(|(_, records)| records)
    }
    pub fn observe<E: WorkOrderEvidenceOwner>(
        &mut self,
        plan: PreparedPlan,
        evidence: &mut E,
        context: &OperationContext,
    ) -> Result<SemanticWorkOrderReview> {
        self.attached = None;
        self.native.clear_selection();
        original(&plan)?;
        let mut call = Call::new(context)?;
        call.admit(LOCAL_BYTES + 2 * VIEW_COST + NATIVE_BYTES + EVIDENCE_BYTES + 1)?;
        call.reserve(LOCAL_BYTES)?;
        let (summary, records) = self.view(&mut call)?;
        authorize_original(&plan, &call.current()?)?;
        self.floor.permit_anchor(plan.anchor)?;
        let key = &plan.steps[0].idempotency_key;
        if records.iter().any(|r| r.plan().key() == key) {
            return Err(error(
                ErrorCode::Conflict,
                "native key exists; reattach only its exact original semantic plan",
            ));
        }
        let observation = self.observe_native(&mut call)?;
        authorize_original(&plan, &call.current()?)?;
        evidence.refresh_after_observation(&observation, &call.reserve(EVIDENCE_BYTES)?)?;
        let route = resolve_work_order_step(
            &plan,
            StepId::ZERO,
            &evidence.routing_evidence()?,
            &observation,
        )?;
        self.floor.retain_anchor(plan.anchor)?;
        let association = Association {
            key: key.clone(),
            semantic: plan.digest,
            step: StepId::ZERO,
            anchor: plan.anchor,
            source: route.source_digest(),
            native: route.native_plan().digest(),
            witness: observation.witness(),
        };
        if self
            .associations
            .get(key)
            .is_some_and(|old| old != &association)
        {
            return Err(error(
                ErrorCode::Conflict,
                "original work-order key already binds another semantic source",
            ));
        }
        let review = SemanticWorkOrderReview {
            seal: association.seal(summary.journal_id),
            original: plan,
            native: route.native_plan().clone(),
            association,
        };
        self.view(&mut call)?;
        authorize_original(&review.original, &call.current()?)?;
        self.attached = Some(review.clone());
        Ok(review)
    }
    pub fn reattach(
        &mut self,
        plan: PreparedPlan,
        context: &OperationContext,
    ) -> Result<SemanticWorkOrderReview> {
        self.attached = None;
        self.native.clear_selection();
        original(&plan)?;
        let mut call = Call::new(context)?;
        call.reserve(LOCAL_BYTES)?;
        let (summary, records) = self.view(&mut call)?;
        let key = &plan.steps[0].idempotency_key;
        let association = self.associations.get(key).ok_or_else(custody)?.clone();
        if association.semantic != plan.digest
            || association.anchor != plan.anchor
            || association.step != StepId::ZERO
        {
            return Err(error(
                ErrorCode::Conflict,
                "reattachment changed the original semantic work-order plan",
            ));
        }
        let native = records.iter().find(|r| r.plan().key() == key).ok_or_else(||
            error(ErrorCode::Conflict, "semantic association precedes native preparation; observe the exact original plan again"))?.plan().clone();
        let review = SemanticWorkOrderReview {
            seal: association.seal(summary.journal_id),
            original: plan,
            native,
            association,
        };
        self.attached = Some(review.clone());
        Ok(review)
    }
    /// Reacquire an unchanged original queue after reattachment. This does not
    /// renew native preparation, change a seal, or dispatch an order.
    pub fn refresh_prepared<E: WorkOrderEvidenceOwner>(
        &mut self,
        seal: Digest32,
        evidence: &mut E,
        context: &OperationContext,
    ) -> Result<SemanticWorkOrderReview> {
        self.native.clear_selection();
        let review = self.attached(seal)?;
        let mut call = Call::new(context)?;
        call.reserve(LOCAL_BYTES)?;
        call.admit(VIEW_COST + NATIVE_BYTES + GUARD_BYTES + 1)?;
        let (_, records) = self.view(&mut call)?;
        self.exact(&review)?;
        authorize_original(&review.original, &call.current()?)?;
        self.floor.permit_anchor(review.original.anchor)?;
        let record = records
            .iter()
            .find(|r| r.plan() == &review.native)
            .ok_or_else(custody)?;
        if record.state() != CreationState::Prepared {
            return Err(error(
                ErrorCode::Conflict,
                "only undispatched original work-order preparation can reacquire a selection",
            ));
        }
        let observation = self.observe_native(&mut call)?;
        if observation != *review.native.observation() {
            self.native.clear_selection();
            return Err(error(
                ErrorCode::StaleAnchor,
                "current queue differs from the original work-order preparation",
            ));
        }
        let result = guard(
            &review,
            &self.floor,
            &mut self.associations,
            evidence,
            &review.native,
            &call.reserve(GUARD_BYTES)?,
        );
        if result.is_err() {
            self.native.clear_selection();
        }
        result?;
        call.current()?;
        Ok(review)
    }
    pub fn prepare<E: WorkOrderEvidenceOwner>(
        &mut self,
        seal: Digest32,
        evidence: &mut E,
        context: &OperationContext,
    ) -> Result<SemanticWorkOrderResult> {
        let review = self.attached(seal)?;
        let mut call = Call::new(context)?;
        call.admit(2 * LOCAL_BYTES + 2 * VIEW_COST + NATIVE_BYTES + GUARD_BYTES + 1)?;
        call.reserve(LOCAL_BYTES)?;
        let (summary, records) = self.view(&mut call)?;
        self.writable(&summary)?;
        authorize_original(&review.original, &call.current()?)?;
        self.floor.permit_anchor(review.original.anchor)?;
        if !records
            .iter()
            .any(|record| record.plan().key() == review.native.key())
        {
            if summary.unresolved != 0 {
                return Err(error(
                    ErrorCode::EffectIndeterminate,
                    "retained native creation is unresolved; reconcile before preparing another order",
                ));
            }
            self.floor.permit_native(NativeClock::observed(
                review.native.observation(),
                review.original.anchor.cursor.epoch,
            ))?;
        }
        // Sync semantic intent before any native preparation can be contacted.
        self.associations
            .retain(review.association.clone(), &call.reserve(LOCAL_BYTES)?)?;
        let native_context = call.native(review.native.observation(), true)?;
        let associations = &mut self.associations;
        let floor = &self.floor;
        let record = self.native.plan_guarded(
            review.native.key(),
            review.native.spec(),
            review.native.observation().witness(),
            &native_context,
            GUARD_BYTES,
            |native, current| guard(&review, floor, associations, evidence, native, current),
        )?;
        self.finish(review, record, &mut call)
    }
    pub fn commit<E: WorkOrderEvidenceOwner>(
        &mut self,
        seal: Digest32,
        confirmed: bool,
        evidence: &mut E,
        context: &OperationContext,
    ) -> Result<SemanticWorkOrderResult> {
        let review = self.attached(seal)?;
        if !confirmed {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "confirm the exact paired semantic/native work-order seal",
            ));
        }
        let mut call = Call::new(context)?;
        call.admit(LOCAL_BYTES + 2 * VIEW_COST + NATIVE_BYTES + GUARD_BYTES + 1)?;
        call.reserve(LOCAL_BYTES)?;
        let (summary, records) = self.view(&mut call)?;
        self.writable(&summary)?;
        self.exact(&review)?;
        authorize_original(&review.original, &call.current()?)?;
        self.floor.permit_anchor(review.original.anchor)?;
        if records.iter().any(|record| {
            record.plan() == &review.native && record.state() == CreationState::Prepared
        }) {
            self.floor.permit_native(NativeClock::observed(
                review.native.observation(),
                review.original.anchor.cursor.epoch,
            ))?;
        }
        let native_context = call.native(review.native.observation(), true)?;
        let associations = &mut self.associations;
        let floor = &self.floor;
        let record = self.native.commit_guarded(
            review.native.key(),
            review.native.digest(),
            review.native.observation().witness(),
            &native_context,
            GUARD_BYTES,
            |native, current| guard(&review, floor, associations, evidence, native, current),
        )?;
        self.finish(review, record, &mut call)
    }
    pub fn inspect(
        &mut self,
        seal: Digest32,
        context: &OperationContext,
    ) -> Result<SemanticWorkOrderResult> {
        let review = self.attached(seal)?;
        let mut call = Call::new(context)?;
        call.reserve(LOCAL_BYTES)?;
        let (_, records) = self.view(&mut call)?;
        let record = records
            .into_iter()
            .find(|r| r.plan() == &review.native)
            .ok_or_else(custody)?;
        self.finish(review, record, &mut call)
    }
    pub fn reconcile(
        &mut self,
        seal: Digest32,
        context: &OperationContext,
    ) -> Result<SemanticWorkOrderResult> {
        let review = self.attached(seal)?;
        let mut call = Call::new(context)?;
        call.admit(LOCAL_BYTES + 2 * VIEW_COST + NATIVE_BYTES + 1)?;
        call.reserve(LOCAL_BYTES)?;
        self.view(&mut call)?;
        self.exact(&review)?;
        // Preserve the caller's current authority tick during historical recovery.
        let record = self.native.reconcile(
            review.native.key(),
            review.native.digest(),
            &call.reserve(NATIVE_BYTES)?,
        )?;
        self.finish(review, record, &mut call)
    }
    /// Retires only a local Prepared record. It cannot remove a created manager
    /// order or bypass uncertainty, and it remains available after plan expiry.
    pub fn cancel(
        &mut self,
        seal: Digest32,
        context: &OperationContext,
    ) -> Result<SemanticWorkOrderResult> {
        let review = self.attached(seal)?;
        let mut call = Call::new(context)?;
        call.admit(LOCAL_BYTES + 2 * VIEW_COST + NATIVE_BYTES + 1)?;
        call.reserve(LOCAL_BYTES)?;
        let (summary, _) = self.view(&mut call)?;
        self.writable(&summary)?;
        self.exact(&review)?;
        authorize_scope(&review.original, &call.current()?)?;
        let record = self.native.cancel(
            review.native.key(),
            review.native.digest(),
            &call.reserve(NATIVE_BYTES)?,
        )?;
        self.finish(review, record, &mut call)
    }
}

fn guard<B: EffectJournalStorage, E: WorkOrderEvidenceOwner>(
    review: &SemanticWorkOrderReview,
    floor: &SourceFloor,
    associations: &mut AssociationStore<B>,
    evidence: &mut E,
    native: &WorkOrderPlan,
    context: &OperationContext,
) -> Result<()> {
    if native != &review.native
        || context.anchor.fortress_id != review.original.anchor.fortress_id
        || context.anchor.tick != review.original.anchor.tick
    {
        return Err(custody());
    }
    let mut canonical = context.clone();
    canonical.anchor = review.original.anchor;
    let mut call = Call::new(&canonical)?;
    floor.permit_anchor(review.original.anchor)?;
    floor.permit_native(NativeClock::observed(
        native.observation(),
        review.original.anchor.cursor.epoch,
    ))?;
    floor.authority(&mut call)?;
    call.reserve(LOCAL_BYTES)?;
    associations.verify(&call.reserve(STORE_BYTES)?)?;
    if associations.get(native.key()) != Some(&review.association) {
        return Err(custody());
    }
    authorize_original(&review.original, &call.current()?)?;
    evidence.refresh_after_observation(native.observation(), &call.reserve(EVIDENCE_BYTES)?)?;
    let route = resolve_work_order_step(
        &review.original,
        StepId::ZERO,
        &evidence.routing_evidence()?,
        native.observation(),
    )?;
    if route.native_plan() != native || route.source_digest() != review.association.source {
        return Err(custody());
    }
    associations.verify(&call.reserve(STORE_BYTES)?)?;
    authorize_original(&review.original, &call.current()?)
}
