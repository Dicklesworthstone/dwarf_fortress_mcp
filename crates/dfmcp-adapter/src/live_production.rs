//! Concrete bounded labor/work-order execution over the existing native owners.
//!
//! One original plan, one paused canonical source and two independently bound
//! mutation journals are retained. The public dispatcher trait cannot bypass
//! dependency receipts or the executor's local preparation ownership. Native
//! effects are historical action evidence, never original goal completion.
//!
//! Bead: df-dfhack-bridge-plane-c-pic.4.
use std::time::{Duration, Instant};

use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, DfmcpError, Digest32, ErrorCode, GameTick,
    OperationContext, Result, RiskTier, SessionId, StepId,
};
use dfmcp_intent::plan_archive::PlanArchiveSeal;
use dfmcp_intent::{PlanStep, PreparedPlan};

use crate::control_effect_journal::EffectJournalStorage;
use crate::live_dispatch::durable::{DurableFamilyExecutor, RecoveredFamily};
use crate::live_dispatch::{
    FamilyPreparation, FamilyReceipt, LiveFamilyExecutor, ReconcileOutcome,
};
use crate::live_routing::{LiveFamily, LiveRoutingEvidence, RoutedStep};
use crate::semantic_selection::SelectedSemanticStep;
use crate::semantic_work_orders::{
    NativeOrderCreation, SemanticWorkOrderResult, SemanticWorkOrderSession, WorkOrderEvidenceOwner,
};
use crate::semantic_workforce::{
    SemanticWorkforceResult, SemanticWorkforceSession, SingleLaborResult, WorkforceEvidenceOwner,
};
use crate::work_order_control::{CreationRecord, CreationState, WorkOrderSource};
use crate::work_orders::WorkOrderObservation;
use crate::workforce_control::AssignmentPhase;
use crate::workforce_control::journal::{
    AssignmentRecord, AssignmentState, MAX_FRAME, WorkforceBinding, WorkforceView,
};
use crate::workforce_control::rpc::WorkforceSource;

pub mod policy;
pub use policy::{MAX_PLAN_BYTES, MAX_STEPS, archive_limits, validate as validate_plan};

const MIB: u64 = 1024 * 1024;
// A native workforce journal is <=64 MiB plus <=64 bounded latest records.
// Its semantic inventory charges two complete views. Order inventory has its
// own fixed 64-key bound. These are work reservations, not allocations.
const WORKFORCE_VIEW_BYTES: u64 = 144 * MIB;
const ORDER_VIEW_BYTES: u64 = 16 * MIB;
const ORDER_OPERATION_BYTES: u64 = 64 * MIB;
const LOCAL_BYTES: u64 = MIB;

fn error(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message)
}
fn exhausted() -> DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "combined native production exhausted its single bounded operation allowance",
    )
}
fn custody() -> DfmcpError {
    error(
        ErrorCode::CorruptLedger,
        "original production plan and retained family custody differ; no later step may dispatch",
    )
}
fn blocked() -> DfmcpError {
    error(
        ErrorCode::EffectIndeterminate,
        "an original family step is prepared or unresolved; inspect, reconcile or cancel its exact identity",
    )
}

// Both existing native coordinators require unlimited whole-fortress Query,
// Plan and their family capability. Intersect that closed profile, keeping the
// earliest expiry and narrower risk in every child context. An entity/map-only
// grant cannot be converted into the empty-scope native authority it lacks.
fn attenuate(c: &OperationContext, opening: &[CapabilityGrant]) -> Result<Vec<CapabilityGrant>> {
    if c.grants.len() > 64 || opening.len() > 64 {
        return Err(exhausted());
    }
    let mut out = Vec::new();
    for capability in [
        Capability::Query,
        Capability::Observe,
        Capability::Plan,
        Capability::ConfigureLabor,
        Capability::ConfigureProduction,
    ] {
        for current in &c.grants {
            if current.is_limited()
                || !(current.capability == capability || current.capability == Capability::Admin)
                || !current.scope.permits(c.anchor.fortress_id, &[], None)
            {
                continue;
            }
            for retained in opening {
                if retained.is_limited()
                    || !(retained.capability == capability
                        || retained.capability == Capability::Admin)
                    || !retained.scope.permits(c.anchor.fortress_id, &[], None)
                {
                    continue;
                }
                let expires_at_tick = match (current.expires_at_tick, retained.expires_at_tick) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (Some(a), None) | (None, Some(a)) => Some(a),
                    (None, None) => None,
                };
                let grant = CapabilityGrant {
                    capability,
                    scope: CapabilityScope {
                        fortress_id: Some(c.anchor.fortress_id),
                        ..CapabilityScope::default()
                    },
                    max_risk: current.max_risk.min(retained.max_risk),
                    expires_at_tick,
                    remaining_uses: None,
                };
                if !out.contains(&grant) {
                    if out.len() == 64 {
                        return Err(exhausted());
                    }
                    out.push(grant);
                }
            }
        }
    }
    Ok(out)
}

/// Trusted native source factory. Endpoints, credentials and source acceptance
/// remain owned by the embedding shell and cannot be selected by a plan.
pub trait WorkforceFactory {
    type Source: WorkforceSource;
    fn connect(&mut self, binding: &WorkforceBinding, c: &OperationContext)
    -> Result<Self::Source>;
}
impl<F, N> WorkforceFactory for F
where
    F: FnMut(&WorkforceBinding, &OperationContext) -> Result<N>,
    N: WorkforceSource,
{
    type Source = N;
    fn connect(&mut self, binding: &WorkforceBinding, c: &OperationContext) -> Result<N> {
        self(binding, c)
    }
}

#[derive(Clone, Debug)]
pub enum StepEvidence {
    Labor(SemanticWorkforceResult),
    WorkOrder(SemanticWorkOrderResult),
}
impl StepEvidence {
    pub fn step_id(&self) -> StepId {
        match self {
            Self::Labor(value) => value.review().step_id(),
            Self::WorkOrder(value) => value.review().step_id(),
        }
    }
    pub fn original_plan(&self) -> &PreparedPlan {
        match self {
            Self::Labor(value) => value.review().original_plan(),
            Self::WorkOrder(value) => value.review().original_plan(),
        }
    }
    pub const fn original_goal_proven(&self) -> bool {
        false
    }
    pub const fn current_state_proven(&self) -> bool {
        false
    }
}

/// Exclusively owns both real semantic/native coordinators. No mutable family
/// session is exposed after construction, and a recovered review never becomes
/// the local single-use preparation required by `commit`.
pub struct Executor<WJ, WA, OJ, OA, ON, E, F> {
    original: PreparedPlan,
    archive: PlanArchiveSeal,
    session: SessionId,
    grants: Vec<CapabilityGrant>,
    workforce: SemanticWorkforceSession<WJ, WA>,
    orders: SemanticWorkOrderSession<OJ, OA, ON>,
    evidence: Option<E>,
    factory: F,
    prepared: Option<(StepId, FamilyPreparation)>,
    high_tick: u64,
    latest: Option<StepEvidence>,
}

struct Call {
    context: OperationContext,
    until: Instant,
    bytes: u64,
}
impl Call {
    fn new(c: &OperationContext, floor: u64) -> Result<Self> {
        c.budget.validate()?;
        let mut context = c.clone();
        context.anchor.tick = GameTick(context.anchor.tick.get().max(floor));
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        Ok(Self {
            until: Instant::now()
                .checked_add(Duration::from_millis(c.budget.max_wall_millis.min(60_000)))
                .ok_or_else(exhausted)?,
            context,
            bytes: c
                .budget
                .max_bytes
                .checked_sub(LOCAL_BYTES)
                .ok_or_else(exhausted)?,
        })
    }
    fn current(&self) -> Result<OperationContext> {
        let millis = self
            .until
            .checked_duration_since(Instant::now())
            .ok_or_else(exhausted)?
            .as_millis();
        if millis == 0 || self.bytes == 0 {
            return Err(exhausted());
        }
        let mut c = self.context.clone();
        c.budget.max_wall_millis = u64::try_from(millis)
            .map_err(|_| exhausted())?
            .min(c.budget.max_wall_millis);
        c.budget.max_bytes = self.bytes;
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        Ok(c)
    }
    fn take(&mut self, bytes: u64) -> Result<OperationContext> {
        let mut c = self.current()?;
        self.bytes = self
            .bytes
            .checked_sub(bytes)
            .filter(|n| *n > 0)
            .ok_or_else(exhausted)?;
        c.budget.max_bytes = bytes;
        Ok(c)
    }
    fn admit(&self, bytes: u64) -> Result<()> {
        self.current()?;
        if self.bytes <= bytes {
            Err(exhausted())
        } else {
            Ok(())
        }
    }
    fn floor(&mut self, tick: u64) {
        self.context.anchor.tick = GameTick(self.context.anchor.tick.get().max(tick));
    }
    fn edge(&self, reserved: &OperationContext, tick: u64) -> Result<OperationContext> {
        let now = self.current()?;
        let mut c = reserved.clone();
        c.anchor.tick = GameTick(c.anchor.tick.get().max(tick).max(now.anchor.tick.get()));
        c.budget.max_wall_millis = c.budget.max_wall_millis.min(now.budget.max_wall_millis);
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        Ok(c)
    }
}

struct Inventory {
    workforce: WorkforceView,
    orders: Vec<CreationRecord>,
}
impl Inventory {
    fn labor_cost(&self) -> u64 {
        crate::workforce_session::view_cost(&self.workforce)
    }
    fn blocked_other_than(&self, key: Option<&str>) -> bool {
        self.workforce
            .records
            .iter()
            .any(|r| !r.state().settled() && Some(r.plan().key()) != key)
            || self
                .orders
                .iter()
                .any(|r| !r.state().terminal() && Some(r.plan().key()) != key)
    }
}
fn workforce_operation_bytes(view_cost: u64) -> Result<u64> {
    view_cost
        .checked_mul(24)
        .and_then(|n| n.checked_add(64 * MAX_FRAME as u64 + 64 * MIB))
        .ok_or_else(exhausted)
}
fn labor_read_bytes(view_cost: u64, views: u64) -> Result<u64> {
    view_cost
        .checked_mul(views)
        .and_then(|n| n.checked_add(2 * MIB))
        .ok_or_else(exhausted)
}

impl<WJ, WA, OJ, OA, ON, E, F> Executor<WJ, WA, OJ, OA, ON, E, F>
where
    WJ: EffectJournalStorage,
    WA: EffectJournalStorage,
    OJ: EffectJournalStorage,
    OA: EffectJournalStorage,
    ON: WorkOrderSource,
    E: WorkforceEvidenceOwner + WorkOrderEvidenceOwner,
    F: WorkforceFactory,
{
    pub fn new(
        original: PreparedPlan,
        workforce: SemanticWorkforceSession<WJ, WA>,
        orders: SemanticWorkOrderSession<OJ, OA, ON>,
        evidence: Option<E>,
        factory: F,
        c: &OperationContext,
    ) -> Result<Self> {
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if c.grants.len() > 64 {
            return Err(exhausted());
        }
        let archive = validate_plan(&original)?;
        if c.anchor.fortress_id != original.anchor.fortress_id {
            return Err(custody());
        }
        let mut out = Self {
            original,
            archive,
            session: c.session_id,
            grants: c.grants.clone(),
            workforce,
            orders,
            evidence,
            factory,
            prepared: None,
            high_tick: c.anchor.tick.get(),
            latest: None,
        };
        let mut call = out.call(c)?;
        let inventory = out.inventory(&mut call)?;
        // Every retained native key must already be associated with this exact
        // complete original parent. Merely matching action keys is insufficient.
        for record in &inventory.workforce.records {
            let step = out.step_for_key(record.plan().key(), LiveFamily::WorkforceV1_17)?;
            let child = call.take(labor_read_bytes(inventory.labor_cost(), 2)?)?;
            out.workforce
                .reattach_selected(out.selection(step.id)?, &child)?;
            out.retain_floor(&mut call);
        }
        for record in &inventory.orders {
            let step = out.step_for_key(record.plan().key(), LiveFamily::WorkOrdersV1_10)?;
            let child = call.take(ORDER_VIEW_BYTES)?;
            out.orders
                .reattach_selected(out.selection(step.id)?, &child)?;
            out.retain_floor(&mut call);
        }
        call.current()?;
        Ok(out)
    }
    pub fn original_plan(&self) -> &PreparedPlan {
        &self.original
    }
    pub const fn archive_seal(&self) -> PlanArchiveSeal {
        self.archive
    }
    pub const fn high_tick(&self) -> u64 {
        self.high_tick
    }
    pub const fn original_goal_proven(&self) -> bool {
        false
    }
    pub fn latest_step_evidence(&self) -> Option<&StepEvidence> {
        self.latest.as_ref()
    }
    /// A failed enclosing response may withdraw only the local permission. The
    /// durable native records and their exact recovery identities remain owned.
    pub fn abandon_preparation(&mut self) {
        self.prepared = None;
        self.latest = None;
    }
    fn selection(&self, step: StepId) -> Result<SelectedSemanticStep> {
        SelectedSemanticStep::new(self.original.clone(), step)
    }
    fn call(&self, c: &OperationContext) -> Result<Call> {
        if c.session_id != self.session
            || c.anchor.fortress_id != self.original.anchor.fortress_id
            || c.anchor.cursor.epoch != self.original.anchor.cursor.epoch
        {
            return Err(custody());
        }
        let mut bounded = c.clone();
        bounded.grants = attenuate(c, &self.grants)?;
        let call = Call::new(&bounded, self.high_tick)?;
        let current = call.current()?;
        let mut retained = current.clone();
        retained.grants = self.grants.clone();
        retained.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        Ok(call)
    }
    fn retained_control(
        &self,
        step: &PlanStep,
        c: &OperationContext,
        new_effect: bool,
    ) -> Result<()> {
        let mut retained = c.clone();
        retained.grants = self.grants.clone();
        let scope = step.action.scope();
        for context in [c, &retained] {
            context.authorize(
                Capability::Plan,
                RiskTier::Guarded,
                &scope.entity_ids,
                scope.map_area,
            )?;
            context.authorize(
                step.required_capability,
                RiskTier::Guarded,
                &scope.entity_ids,
                scope.map_area,
            )?;
        }
        if new_effect
            && (c.anchor != self.original.anchor || c.anchor.tick >= self.original.expires_at_tick)
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "new production effects require the unchanged original paused canonical anchor and deadline",
            ));
        }
        Ok(())
    }
    fn retain_floor(&mut self, call: &mut Call) {
        self.high_tick = self
            .high_tick
            .max(self.workforce.high_tick())
            .max(self.orders.high_tick());
        call.floor(self.high_tick);
    }
    fn step_for_key(&self, key: &str, family: LiveFamily) -> Result<PlanStep> {
        let step = self
            .original
            .steps
            .iter()
            .find(|step| step.idempotency_key == key)
            .ok_or_else(custody)?;
        if policy::route(step)?.family != family {
            return Err(custody());
        }
        Ok(step.clone())
    }
    fn inventory(&mut self, call: &mut Call) -> Result<Inventory> {
        // Both complete inventories are reserved before reading either owner.
        let workforce_c = call.take(WORKFORCE_VIEW_BYTES)?;
        let orders_c = call.take(ORDER_VIEW_BYTES)?;
        let result = self
            .workforce
            .inventory(&call.edge(&workforce_c, self.high_tick)?);
        self.retain_floor(call);
        let workforce = result?;
        let result = self
            .orders
            .inventory(&call.edge(&orders_c, self.high_tick)?);
        self.retain_floor(call);
        let orders = result?;
        if workforce.records.len() + orders.len() > self.original.steps.len() {
            return Err(custody());
        }
        for record in &workforce.records {
            self.step_for_key(record.plan().key(), LiveFamily::WorkforceV1_17)?;
        }
        for record in &orders {
            self.step_for_key(record.plan().key(), LiveFamily::WorkOrdersV1_10)?;
        }
        call.current()?;
        Ok(Inventory { workforce, orders })
    }
    fn dependencies(
        &mut self,
        step: &PlanStep,
        inventory: &Inventory,
        call: &mut Call,
    ) -> Result<Dependencies> {
        let mut records = Vec::with_capacity(step.depends_on.len());
        for id in &step.depends_on {
            let predecessor = policy::selected(&self.original, *id)?;
            let record = inventory
                .workforce
                .records
                .iter()
                .find(|r| r.plan().key() == predecessor.idempotency_key)
                .ok_or_else(blocked)?;
            let child = call.take(labor_read_bytes(inventory.labor_cost(), 2)?)?;
            let review = self
                .workforce
                .reattach_selected(self.selection(*id)?, &child)?;
            self.retain_floor(call);
            let child = call.take(labor_read_bytes(inventory.labor_cost(), 4)?)?;
            let result = self.workforce.inspect(review.seal(), &child);
            self.retain_floor(call);
            let result = result?;
            if result.native_record() != record
                || !matches!(result.action_result(), SingleLaborResult::Verified { .. })
            {
                return Err(blocked());
            }
            records.push(record.clone());
        }
        Ok(Dependencies {
            journal: inventory.workforce.id,
            records,
            check_bytes: labor_read_bytes(inventory.labor_cost(), 2)?,
        })
    }
    fn check_call(&self, plan: &PreparedPlan, step: &PlanStep, route: &RoutedStep) -> Result<()> {
        policy::verify_call(&self.original, plan, step, route)
    }
    fn preflight(
        &self,
        step: &PlanStep,
        inventory: &Inventory,
        operation_bytes: u64,
        call: &Call,
    ) -> Result<()> {
        let dependency_bytes = labor_read_bytes(inventory.labor_cost(), 6)?
            .checked_mul(step.depends_on.len() as u64)
            .ok_or_else(exhausted)?;
        let guard_bytes = labor_read_bytes(inventory.labor_cost(), 2)?
            .checked_mul(3)
            .ok_or_else(exhausted)?;
        call.admit(
            operation_bytes
                .checked_add(dependency_bytes)
                .and_then(|n| n.checked_add(guard_bytes))
                .and_then(|n| {
                    n.checked_add(
                        WORKFORCE_VIEW_BYTES + ORDER_VIEW_BYTES + ORDER_VIEW_BYTES + 4 * MIB,
                    )
                })
                .ok_or_else(exhausted)?,
        )
    }
    fn finish_evidence(
        &mut self,
        evidence: StepEvidence,
        call: &mut Call,
    ) -> Result<RecoveredFamily> {
        self.retain_floor(call);
        let outcome = classify(&evidence)?;
        self.latest = Some(evidence);
        call.current()?;
        Ok(outcome)
    }
}

#[derive(Clone)]
struct Dependencies {
    journal: Digest32,
    records: Vec<AssignmentRecord>,
    check_bytes: u64,
}
impl Dependencies {
    fn verify<WJ: EffectJournalStorage, WA: EffectJournalStorage>(
        &self,
        owner: &mut SemanticWorkforceSession<WJ, WA>,
        c: &OperationContext,
    ) -> Result<()> {
        let current = owner.inventory(c)?;
        if current.id != self.journal
            || self.records.iter().any(|old| {
                current
                    .records
                    .iter()
                    .find(|record| record.plan().key() == old.plan().key())
                    != Some(old)
            })
        {
            return Err(custody());
        }
        Ok(())
    }
}

/// A work-order's own final semantic refresh occurs after its durable dispatch
/// marker. Reverify every original labor predecessor at that exact boundary,
/// using separately prepaid custody work from the same outer operation.
struct OrderEvidence<'a, WJ, WA, E> {
    source: &'a mut E,
    workforce: &'a mut SemanticWorkforceSession<WJ, WA>,
    dependencies: &'a Dependencies,
    checks: Vec<OperationContext>,
    until: Instant,
}
impl<WJ: EffectJournalStorage, WA: EffectJournalStorage, E: WorkOrderEvidenceOwner>
    WorkOrderEvidenceOwner for OrderEvidence<'_, WJ, WA, E>
{
    fn refresh_after_observation(
        &mut self,
        observation: &WorkOrderObservation,
        c: &OperationContext,
    ) -> Result<()> {
        self.source.refresh_after_observation(observation, c)?;
        let mut check = self.checks.pop().ok_or_else(exhausted)?;
        if check.session_id != c.session_id || check.request_id != c.request_id {
            return Err(custody());
        }
        // Current nested grants can narrow this independent reservation but can
        // never acquire a grant from the outer prepaid child.
        check.grants = c.grants.clone();
        check.cancellation_requested |= c.cancellation_requested;
        check.anchor.tick = GameTick(
            check
                .anchor
                .tick
                .get()
                .max(c.anchor.tick.get())
                .max(self.workforce.high_tick()),
        );
        let millis = self
            .until
            .checked_duration_since(Instant::now())
            .ok_or_else(exhausted)?
            .as_millis();
        if millis == 0 {
            return Err(exhausted());
        }
        check.budget.max_wall_millis = check
            .budget
            .max_wall_millis
            .min(c.budget.max_wall_millis)
            .min(u64::try_from(millis).map_err(|_| exhausted())?);
        self.dependencies.verify(self.workforce, &check)
    }
    fn routing_evidence(&self) -> Result<LiveRoutingEvidence<'_>> {
        self.source.routing_evidence()
    }
}

fn receipt(family: LiveFamily, seal: Digest32, native: Digest32) -> Result<FamilyReceipt> {
    policy::nonzero(seal)?;
    policy::nonzero(native)?;
    let mut bytes = b"dfmcp-live-production-family-receipt/1\0".to_vec();
    bytes.extend_from_slice(family.protocol().as_bytes());
    bytes.extend_from_slice(seal.as_bytes());
    bytes.extend_from_slice(native.as_bytes());
    FamilyReceipt::new(family, Digest32::of_bytes(&bytes))
}
fn classify(value: &StepEvidence) -> Result<RecoveredFamily> {
    match value {
        StepEvidence::Labor(value) => {
            let seal = value.review().seal();
            let family = LiveFamily::WorkforceV1_17;
            match value.action_result() {
                SingleLaborResult::Verified { receipt: proof } => {
                    Ok(RecoveredFamily::Applied(receipt(family, seal, *proof)?))
                }
                SingleLaborResult::AppliedOutsideSemantics { receipt: proof } => Ok(
                    RecoveredFamily::Failed(Some(receipt(family, seal, *proof)?)),
                ),
                SingleLaborResult::Unverified => match value.native_record().state() {
                    AssignmentState::Prepared => Ok(RecoveredFamily::Prepared(
                        FamilyPreparation::new(family, seal)?,
                    )),
                    AssignmentState::CancelledBeforeDispatch => {
                        Ok(RecoveredFamily::Cancelled(None))
                    }
                    AssignmentState::Terminal => match value.native_record().effect() {
                        Some(effect) if effect.phase() == AssignmentPhase::Refused => Ok(
                            RecoveredFamily::Failed(Some(receipt(family, seal, effect.receipt())?)),
                        ),
                        Some(effect) if effect.phase() == AssignmentPhase::Cancelled => {
                            Ok(RecoveredFamily::Cancelled(Some(receipt(
                                family,
                                seal,
                                effect.receipt(),
                            )?)))
                        }
                        _ => Err(custody()),
                    },
                    _ => Ok(RecoveredFamily::Indeterminate),
                },
            }
        }
        StepEvidence::WorkOrder(value) => {
            let family = LiveFamily::WorkOrdersV1_10;
            let seal = value.review().seal();
            match value.creation_result() {
                NativeOrderCreation::Inserted { receipt: proof, .. } => {
                    Ok(RecoveredFamily::Applied(receipt(family, seal, *proof)?))
                }
                NativeOrderCreation::Unverified => match value.native_record().state() {
                    CreationState::Prepared => Ok(RecoveredFamily::Prepared(
                        FamilyPreparation::new(family, seal)?,
                    )),
                    CreationState::CancelledBeforeDispatch => Ok(RecoveredFamily::Cancelled(None)),
                    CreationState::Refused => Ok(RecoveredFamily::Failed(
                        value
                            .native_record()
                            .effect()
                            .receipt()
                            .map(|proof| receipt(family, seal, proof))
                            .transpose()?,
                    )),
                    _ => Ok(RecoveredFamily::Indeterminate),
                },
            }
        }
    }
}

impl<WJ, WA, OJ, OA, ON, E, F> Executor<WJ, WA, OJ, OA, ON, E, F>
where
    WJ: EffectJournalStorage,
    WA: EffectJournalStorage,
    OJ: EffectJournalStorage,
    OA: EffectJournalStorage,
    ON: WorkOrderSource,
    E: WorkforceEvidenceOwner + WorkOrderEvidenceOwner,
    F: WorkforceFactory,
{
    fn final_check(&mut self, reserved: &OperationContext, call: &mut Call) -> Result<()> {
        self.retain_floor(call);
        let c = call.edge(reserved, self.high_tick)?;
        let mut last = self.call(&c)?;
        self.inventory(&mut last)?;
        self.retain_floor(call);
        let current = call.current()?;
        let mut retained = current.clone();
        retained.grants = self.grants.clone();
        retained.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        Ok(())
    }
    fn fresh_prepare(
        &mut self,
        step: &PlanStep,
        route: &RoutedStep,
        call: &mut Call,
    ) -> Result<FamilyPreparation> {
        if self.prepared.is_some() {
            return Err(blocked());
        }
        self.retained_control(step, &call.current()?, true)?;
        let inventory = self.inventory(call)?;
        if inventory.blocked_other_than(None) {
            return Err(blocked());
        }
        if inventory
            .workforce
            .records
            .iter()
            .any(|r| r.plan().key() == step.idempotency_key)
            || inventory
                .orders
                .iter()
                .any(|r| r.plan().key() == step.idempotency_key)
        {
            return Err(error(
                ErrorCode::Conflict,
                "original native key already exists; reconstruct its historical outcome instead of preparing it again",
            ));
        }
        let native_bytes = if route.family == LiveFamily::WorkforceV1_17 {
            workforce_operation_bytes(inventory.labor_cost())?
        } else {
            ORDER_OPERATION_BYTES
        };
        self.preflight(
            step,
            &inventory,
            native_bytes.checked_mul(2).ok_or_else(exhausted)?,
            call,
        )?;
        let publication = call.take(WORKFORCE_VIEW_BYTES + ORDER_VIEW_BYTES + 2 * MIB)?;
        let dependencies = self.dependencies(step, &inventory, call)?;
        let selection = self.selection(step.id)?;
        let observe = call.take(native_bytes)?;
        let prepare = call.take(native_bytes)?;
        let checks = (0..2)
            .map(|_| call.take(dependencies.check_bytes))
            .collect::<Result<Vec<_>>>()?;
        let until = call.until;
        let value = match route.family {
            LiveFamily::WorkforceV1_17 => {
                let source = self.evidence.as_mut().ok_or_else(|| error(ErrorCode::CapabilityDenied, "new native production requires independently owned original canonical evidence"))?;
                let factory = &mut self.factory;
                let observe = call.edge(&observe, self.workforce.high_tick())?;
                let review =
                    self.workforce
                        .observe_selected(selection, source, &observe, |b, c| {
                            factory.connect(b, c)
                        })?;
                let current = call.edge(&prepare, self.workforce.high_tick())?;
                StepEvidence::Labor(self.workforce.prepare(
                    review.seal(),
                    source,
                    &current,
                    |b, c| factory.connect(b, c),
                )?)
            }
            LiveFamily::WorkOrdersV1_10 => {
                let source = self.evidence.as_mut().ok_or_else(|| error(ErrorCode::CapabilityDenied, "new native production requires independently owned original canonical evidence"))?;
                let mut guarded = OrderEvidence {
                    source,
                    workforce: &mut self.workforce,
                    dependencies: &dependencies,
                    checks,
                    until,
                };
                let observe = call.edge(
                    &observe,
                    self.orders.high_tick().max(guarded.workforce.high_tick()),
                )?;
                let review = self
                    .orders
                    .observe_selected(selection, &mut guarded, &observe)?;
                let current = call.edge(
                    &prepare,
                    self.orders.high_tick().max(guarded.workforce.high_tick()),
                )?;
                StepEvidence::WorkOrder(self.orders.prepare(
                    review.seal(),
                    &mut guarded,
                    &current,
                )?)
            }
            _ => return Err(custody()),
        };
        let outcome = self.finish_evidence(value, call)?;
        self.final_check(&publication, call)?;
        self.retained_control(step, &call.current()?, true)?;
        let RecoveredFamily::Prepared(preparation) = outcome else {
            return Err(error(
                ErrorCode::AdapterRejected,
                "native preparation did not produce a complete original prepared review",
            ));
        };
        self.prepared = Some((step.id, preparation));
        Ok(preparation)
    }
    fn dispatch(
        &mut self,
        step: &PlanStep,
        route: &RoutedStep,
        preparation: FamilyPreparation,
        call: &mut Call,
    ) -> Result<FamilyReceipt> {
        if self.prepared.take() != Some((step.id, preparation))
            || preparation.family != route.family
        {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "commit requires this opening's exact single-use family preparation; recovery does not restore it",
            ));
        }
        self.retained_control(step, &call.current()?, true)?;
        let inventory = self.inventory(call)?;
        if inventory.blocked_other_than(Some(&step.idempotency_key)) {
            return Err(blocked());
        }
        let native_bytes = if route.family == LiveFamily::WorkforceV1_17 {
            workforce_operation_bytes(inventory.labor_cost())?
        } else {
            ORDER_OPERATION_BYTES
        };
        self.preflight(step, &inventory, native_bytes, call)?;
        let publication = call.take(WORKFORCE_VIEW_BYTES + ORDER_VIEW_BYTES + 2 * MIB)?;
        let dependencies = self.dependencies(step, &inventory, call)?;
        let native = call.take(native_bytes)?;
        let checks = vec![call.take(dependencies.check_bytes)?];
        let value = match route.family {
            LiveFamily::WorkforceV1_17 => {
                // Dependency inspection may have attached another labor step;
                // exact reattachment preserves this locally retained seal and
                // does not restore or renew its native preparation.
                let child = call.take(labor_read_bytes(inventory.labor_cost(), 2)?)?;
                let review = self
                    .workforce
                    .reattach_selected(self.selection(step.id)?, &child)?;
                if review.seal() != preparation.digest {
                    return Err(custody());
                }
                let source = self.evidence.as_mut().ok_or_else(|| {
                    error(
                        ErrorCode::CapabilityDenied,
                        "current canonical evidence is unavailable",
                    )
                })?;
                let factory = &mut self.factory;
                let native = call.edge(&native, self.workforce.high_tick())?;
                StepEvidence::Labor(self.workforce.commit(
                    preparation.digest,
                    true,
                    source,
                    &native,
                    |b, c| factory.connect(b, c),
                )?)
            }
            LiveFamily::WorkOrdersV1_10 => {
                let source = self.evidence.as_mut().ok_or_else(|| {
                    error(
                        ErrorCode::CapabilityDenied,
                        "current canonical evidence is unavailable",
                    )
                })?;
                let mut guarded = OrderEvidence {
                    source,
                    workforce: &mut self.workforce,
                    dependencies: &dependencies,
                    checks,
                    until: call.until,
                };
                let native = call.edge(
                    &native,
                    self.orders.high_tick().max(guarded.workforce.high_tick()),
                )?;
                StepEvidence::WorkOrder(self.orders.commit(
                    preparation.digest,
                    true,
                    &mut guarded,
                    &native,
                )?)
            }
            _ => return Err(custody()),
        };
        let outcome = self.finish_evidence(value, call)?;
        self.final_check(&publication, call)?;
        match outcome {
            RecoveredFamily::Applied(receipt) => Ok(receipt),
            RecoveredFamily::Failed(_) => Err(error(
                ErrorCode::AdapterRejected,
                "native action was refused or exceeded the original selected semantics",
            )),
            _ => Err(blocked()),
        }
    }
    fn historical(
        &mut self,
        step: &PlanStep,
        route: &RoutedStep,
        call: &mut Call,
        query: bool,
    ) -> Result<RecoveredFamily> {
        let inventory = self.inventory(call)?;
        let publication = call.take(WORKFORCE_VIEW_BYTES + ORDER_VIEW_BYTES + 2 * MIB)?;
        let selection = self.selection(step.id)?;
        let value = match route.family {
            LiveFamily::WorkforceV1_17 => {
                if !inventory
                    .workforce
                    .records
                    .iter()
                    .any(|r| r.plan().key() == step.idempotency_key)
                {
                    self.final_check(&publication, call)?;
                    return Ok(RecoveredFamily::AbsentBeforeDispatch);
                }
                let attach = call.take(labor_read_bytes(inventory.labor_cost(), 2)?)?;
                let review = self.workforce.reattach_selected(selection, &attach)?;
                let result = if query {
                    let current = call.take(workforce_operation_bytes(inventory.labor_cost())?)?;
                    let factory = &mut self.factory;
                    self.workforce
                        .reconcile(review.seal(), &current, |b, c| factory.connect(b, c))?
                } else {
                    self.workforce.inspect(
                        review.seal(),
                        &call.take(labor_read_bytes(inventory.labor_cost(), 4)?)?,
                    )?
                };
                StepEvidence::Labor(result)
            }
            LiveFamily::WorkOrdersV1_10 => {
                if !inventory
                    .orders
                    .iter()
                    .any(|r| r.plan().key() == step.idempotency_key)
                {
                    self.final_check(&publication, call)?;
                    return Ok(RecoveredFamily::AbsentBeforeDispatch);
                }
                let review = self
                    .orders
                    .reattach_selected(selection, &call.take(ORDER_VIEW_BYTES)?)?;
                let result = if query {
                    self.orders
                        .reconcile(review.seal(), &call.take(ORDER_OPERATION_BYTES)?)?
                } else {
                    self.orders
                        .inspect(review.seal(), &call.take(ORDER_VIEW_BYTES)?)?
                };
                StepEvidence::WorkOrder(result)
            }
            _ => return Err(custody()),
        };
        let result = self.finish_evidence(value, call)?;
        self.final_check(&publication, call)?;
        Ok(result)
    }
    fn cancel(
        &mut self,
        step: &PlanStep,
        route: &RoutedStep,
        preparation: FamilyPreparation,
        call: &mut Call,
    ) -> Result<Option<FamilyReceipt>> {
        self.prepared = None;
        self.retained_control(step, &call.current()?, false)?;
        let inventory = self.inventory(call)?;
        let publication = call.take(WORKFORCE_VIEW_BYTES + ORDER_VIEW_BYTES + 2 * MIB)?;
        let selection = self.selection(step.id)?;
        let value = match route.family {
            LiveFamily::WorkforceV1_17 => {
                let record = inventory
                    .workforce
                    .records
                    .iter()
                    .find(|r| r.plan().key() == step.idempotency_key)
                    .ok_or_else(custody)?;
                if record.state() != AssignmentState::Prepared {
                    return Err(blocked());
                }
                let review = self.workforce.reattach_selected(
                    selection,
                    &call.take(labor_read_bytes(inventory.labor_cost(), 2)?)?,
                )?;
                if preparation != FamilyPreparation::new(route.family, review.seal())? {
                    return Err(custody());
                }
                let current = call.take(workforce_operation_bytes(inventory.labor_cost())?)?;
                let factory = &mut self.factory;
                StepEvidence::Labor(
                    self.workforce
                        .cancel(review.seal(), &current, |b, c| factory.connect(b, c))?,
                )
            }
            LiveFamily::WorkOrdersV1_10 => {
                let record = inventory
                    .orders
                    .iter()
                    .find(|r| r.plan().key() == step.idempotency_key)
                    .ok_or_else(custody)?;
                if record.state() != CreationState::Prepared {
                    return Err(blocked());
                }
                let review = self
                    .orders
                    .reattach_selected(selection, &call.take(ORDER_VIEW_BYTES)?)?;
                if preparation != FamilyPreparation::new(route.family, review.seal())? {
                    return Err(custody());
                }
                StepEvidence::WorkOrder(
                    self.orders
                        .cancel(review.seal(), &call.take(ORDER_OPERATION_BYTES)?)?,
                )
            }
            _ => return Err(custody()),
        };
        let outcome = self.finish_evidence(value, call)?;
        self.final_check(&publication, call)?;
        match outcome {
            RecoveredFamily::Cancelled(receipt) => Ok(receipt),
            _ => Err(blocked()),
        }
    }
}

impl<WJ, WA, OJ, OA, ON, E, F> LiveFamilyExecutor for Executor<WJ, WA, OJ, OA, ON, E, F>
where
    WJ: EffectJournalStorage,
    WA: EffectJournalStorage,
    OJ: EffectJournalStorage,
    OA: EffectJournalStorage,
    ON: WorkOrderSource,
    E: WorkforceEvidenceOwner + WorkOrderEvidenceOwner,
    F: WorkforceFactory,
{
    fn prepare(
        &mut self,
        plan: &PreparedPlan,
        step: &PlanStep,
        route: &RoutedStep,
        c: &OperationContext,
    ) -> Result<FamilyPreparation> {
        self.latest = None;
        if let Err(cause) = self.check_call(plan, step, route) {
            self.prepared = None;
            return Err(cause);
        }
        let mut call = match self.call(c) {
            Ok(call) => call,
            Err(cause) => {
                self.prepared = None;
                return Err(cause);
            }
        };
        self.latest = None;
        let outcome = self.fresh_prepare(step, route, &mut call);
        self.retain_floor(&mut call);
        if outcome.is_err() {
            self.prepared = None;
            self.latest = None;
        }
        outcome
    }
    fn commit(
        &mut self,
        plan: &PreparedPlan,
        step: &PlanStep,
        route: &RoutedStep,
        preparation: FamilyPreparation,
        c: &OperationContext,
    ) -> Result<FamilyReceipt> {
        self.latest = None;
        if let Err(cause) = self.check_call(plan, step, route) {
            self.prepared = None;
            return Err(cause);
        }
        let mut call = match self.call(c) {
            Ok(call) => call,
            Err(cause) => {
                self.prepared = None;
                return Err(cause);
            }
        };
        self.latest = None;
        let outcome = self.dispatch(step, route, preparation, &mut call);
        self.retain_floor(&mut call);
        self.prepared = None;
        if outcome.is_err() {
            self.latest = None;
        }
        outcome
    }
    fn reconcile(
        &mut self,
        plan: &PreparedPlan,
        step: &PlanStep,
        route: &RoutedStep,
        c: &OperationContext,
    ) -> Result<ReconcileOutcome> {
        self.latest = None;
        if let Err(cause) = self.check_call(plan, step, route) {
            self.prepared = None;
            return Err(cause);
        }
        let mut call = match self.call(c) {
            Ok(call) => call,
            Err(cause) => {
                self.prepared = None;
                return Err(cause);
            }
        };
        self.prepared = None;
        let outcome = self.historical(step, route, &mut call, true);
        self.retain_floor(&mut call);
        if outcome.is_err() {
            self.latest = None;
        }
        Ok(match outcome? {
            RecoveredFamily::Applied(receipt) => ReconcileOutcome::Applied(receipt),
            RecoveredFamily::Failed(receipt) | RecoveredFamily::Cancelled(receipt) => {
                ReconcileOutcome::Failed(receipt)
            }
            // Absence alone can never erase a dispatch marker in another owner.
            _ => ReconcileOutcome::Indeterminate,
        })
    }
    fn cancel_prepared(
        &mut self,
        plan: &PreparedPlan,
        step: &PlanStep,
        route: &RoutedStep,
        preparation: FamilyPreparation,
        c: &OperationContext,
    ) -> Result<Option<FamilyReceipt>> {
        self.latest = None;
        if let Err(cause) = self.check_call(plan, step, route) {
            self.prepared = None;
            return Err(cause);
        }
        let mut call = match self.call(c) {
            Ok(call) => call,
            Err(cause) => {
                self.prepared = None;
                return Err(cause);
            }
        };
        let outcome = self.cancel(step, route, preparation, &mut call);
        self.retain_floor(&mut call);
        self.prepared = None;
        if outcome.is_err() {
            self.latest = None;
        }
        outcome
    }
}
impl<WJ, WA, OJ, OA, ON, E, F> DurableFamilyExecutor for Executor<WJ, WA, OJ, OA, ON, E, F>
where
    WJ: EffectJournalStorage,
    WA: EffectJournalStorage,
    OJ: EffectJournalStorage,
    OA: EffectJournalStorage,
    ON: WorkOrderSource,
    E: WorkforceEvidenceOwner + WorkOrderEvidenceOwner,
    F: WorkforceFactory,
{
    fn recover_step(
        &mut self,
        plan: &PreparedPlan,
        step: &PlanStep,
        route: &RoutedStep,
        c: &OperationContext,
    ) -> Result<RecoveredFamily> {
        self.latest = None;
        if let Err(cause) = self.check_call(plan, step, route) {
            self.prepared = None;
            return Err(cause);
        }
        let mut call = match self.call(c) {
            Ok(call) => call,
            Err(cause) => {
                self.prepared = None;
                return Err(cause);
            }
        };
        self.prepared = None;
        let outcome = self.historical(step, route, &mut call, true);
        self.retain_floor(&mut call);
        if outcome.is_err() {
            self.latest = None;
        }
        outcome
    }
    fn observed_tick_floor(&self) -> u64 {
        self.high_tick
    }
    fn abandon_preparation(&mut self) {
        self.prepared = None;
        self.latest = None;
    }
}

#[cfg(test)]
mod tests;
