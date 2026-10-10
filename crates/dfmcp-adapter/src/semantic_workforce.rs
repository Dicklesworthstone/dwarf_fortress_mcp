//! One sealed SetLabor step handed to the existing workforce coordinator.
//!
//! This owner issues no evidence, capability or compatibility grant. It preserves
//! the original plan and reports historical single-labor readback separately from
//! explicit fresh goal observations. Native operation results alone never prove
//! the original predicates or optional obligation; poll_original_goal does.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use dfmcp_core::{
    Capability, Digest32, EntityId, ErrorCode, GameTick, OperationContext, Result, RiskTier, StepId,
};
use dfmcp_intent::{Action, ObligationSpec, PreparedPlan};
use dfmcp_world::{CompareOp, EntityKind, Predicate};

use crate::bounded_run::error;
use crate::control_effect_journal::EffectJournalStorage;
use crate::live_routing::{LiveRoutingEvidence, resolve_workforce_step};
use crate::workforce_control::journal::{
    AssignmentRecord, MAX_FRAME, WorkforceBinding, WorkforceView,
};
use crate::workforce_control::rpc::{CONNECT_BYTES, RPC_BYTES, WorkforceManifest, WorkforceSource};
use crate::workforce_control::{
    AssignmentEffect, AssignmentPhase, AssignmentPlan, WorkforceCapture,
};
use crate::workforce_session::{WorkforceSession, view_cost};

pub mod evidence_owner;
pub mod goal_history;
pub mod goal_monitor;
pub use goal_history::{
    DurableWorkforceGoalMonitor, GoalHistoryStore, PrivateGoalHistoryStore,
    open_private_goal_history_store,
};
pub mod projection;
pub mod store;
pub use goal_monitor::{WorkforceGoalMonitor, WorkforceGoalProgress, WorkforceGoalResult};
pub use projection::WorkforceGoalProjection;
use store::{Association, AssociationStore};
pub use store::{PrivateAssociationStore, open_private_association_store};

/// An observing shell owns acquisition and independently issued source/domain
/// grants. Refresh must acquire current evidence AFTER the native capture. The
/// borrowed scope cannot escape its owner, and client JSON cannot create it here.
pub trait WorkforceEvidenceOwner {
    fn refresh_after_capture(
        &mut self,
        capture: &WorkforceCapture,
        context: &OperationContext,
    ) -> Result<()>;
    fn routing_evidence(&self) -> Result<LiveRoutingEvidence<'_>>;
}

const MAX_ORIGINAL_BYTES: usize = 64 * 1024;
const MAX_ORIGINAL_PREDICATES: usize = 512;
const MAX_EVIDENCE_REFRESH_BYTES: u64 = 16 * 1024 * 1024;
// A full 64-key association store is <=26,832 bytes. Twelve verification
// reads plus retain's 6*length+4096 bound consume <480 KiB; this reservation
// also covers bounded original-plan/review encodings and retained copies.
const LOCAL_RESERVE: u64 = 16 * 64 * 1024;

fn refused(message: &str) -> dfmcp_core::DfmcpError {
    error(ErrorCode::AdapterRejected, message)
}
fn conflict(message: &str) -> dfmcp_core::DfmcpError {
    error(ErrorCode::Conflict, message)
}
fn custody() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::CorruptLedger,
        "semantic/native workforce custody differs; no new binding or dispatch is permitted",
    )
}
fn exhausted() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "semantic workforce handoff exhausted its bounded call allowance",
    )
}

fn protected(predicate: &Predicate, units: &[EntityId]) -> bool {
    match predicate {
        Predicate::True | Predicate::Paused(true) => true,
        Predicate::EntityExists(id) => units.contains(id),
        Predicate::EntityKind {
            entity_id,
            kind: EntityKind::Unit,
        } => units.contains(entity_id),
        Predicate::FieldCompare {
            entity_id,
            field,
            op: CompareOp::Eq,
            ..
        } => {
            units.contains(entity_id) && matches!(field.as_str(), "raw_unit_id" | "native_unit_id")
        }
        Predicate::All(children) => children.iter().all(|child| protected(child, units)),
        _ => false,
    }
}

fn original(plan: &PreparedPlan) -> Result<()> {
    if plan.steps.len() != 1 || plan.summary.len() > 4096 || plan.requires_checkpoint {
        return Err(refused(
            "semantic workforce handoff requires one step and cannot supply a mandatory checkpoint",
        ));
    }
    let step = &plan.steps[0];
    let Action::SetLabor { units, labor, .. } = &step.action else {
        return Err(refused("semantic workforce handoff supports SetLabor only"));
    };
    if units.is_empty()
        || units.len() > 32
        || labor.is_empty()
        || labor.len() > 64
        || !step.depends_on.is_empty()
        || step.compensation.is_some()
        || step.preconditions.len() > 64
        || step.postconditions.len() > 64
    {
        return Err(refused(
            "semantic workforce handoff cannot drop dependencies, compensation or oversized action requirements",
        ));
    }
    let mut bytes = plan.summary.len() + labor.len() + units.len() * 8;
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
    if step
        .preconditions
        .iter()
        .any(|predicate| !protected(predicate, units))
    {
        return Err(refused(
            "original precondition is outside the exact workforce capture; cross-family facts cannot authorize this handoff",
        ));
    }
    Ok(())
}

fn authorize_scope(plan: &PreparedPlan, c: &OperationContext) -> Result<()> {
    let Action::SetLabor { units, .. } = &plan.steps[0].action else {
        return Err(custody());
    };
    if c.budget.max_actions == 0 {
        return Err(exhausted());
    }
    if c.anchor.fortress_id != plan.anchor.fortress_id {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "semantic workforce authority belongs to another fortress",
        ));
    }
    c.authorize(Capability::Plan, RiskTier::Guarded, units, None)?;
    c.authorize(Capability::ConfigureLabor, RiskTier::Guarded, units, None)
}

fn authorize_original(plan: &PreparedPlan, c: &OperationContext) -> Result<()> {
    authorize_scope(plan, c)?;
    if c.anchor.tick >= plan.expires_at_tick
        || plan.steps[0]
            .obligation
            .as_ref()
            .is_some_and(|obligation| c.anchor.tick >= obligation.deadline_tick)
    {
        return Err(error(
            ErrorCode::StaleAnchor,
            "original semantic plan fortress or expiry no longer permits control",
        ));
    }
    Ok(())
}

/// Immutable review information. The seal binds BOTH semantic and native plans,
/// their original source, and the exact native journal incarnation.
#[derive(Clone, Debug)]
pub struct SemanticWorkforceReview {
    original: PreparedPlan,
    native: AssignmentPlan,
    association: Association,
    seal: Digest32,
}
impl SemanticWorkforceReview {
    pub fn original_plan(&self) -> &PreparedPlan {
        &self.original
    }
    pub fn native_plan(&self) -> &AssignmentPlan {
        &self.native
    }
    pub fn seal(&self) -> Digest32 {
        self.seal
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SingleLaborResult {
    /// Native history is pending, Unknown, or did not contain an Applied effect.
    Unverified,
    /// Applied receipt verified for exactly the requested labor, at its
    /// historical native capture. This does not prove current state.
    Verified { receipt: Digest32 },
    /// Native Applied changed additional labor columns or did not set the
    /// requested value. Preserve the native receipt and expose the mismatch.
    AppliedOutsideSemantics { receipt: Digest32 },
}

#[derive(Clone, Debug)]
pub struct SemanticWorkforceResult {
    review: SemanticWorkforceReview,
    native: AssignmentRecord,
    action: SingleLaborResult,
}
impl SemanticWorkforceResult {
    pub fn review(&self) -> &SemanticWorkforceReview {
        &self.review
    }
    pub fn native_record(&self) -> &AssignmentRecord {
        &self.native
    }
    pub fn action_result(&self) -> &SingleLaborResult {
        &self.action
    }
    pub const fn original_goal_proven(&self) -> bool {
        false
    }
    pub fn pending_obligation(&self) -> Option<&ObligationSpec> {
        self.review.original.steps[0].obligation.as_ref()
    }
}

// The outer budget reserves entire nested native operations before invoking
// them. The evidence owner receives at most two separately reserved refreshes;
// every context shares one shrinking absolute wall deadline.
struct Call {
    context: OperationContext,
    deadline: Instant,
    remaining: u64,
    refreshes: u8,
}
impl Call {
    fn new(c: &OperationContext) -> Result<Self> {
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        let deadline = Instant::now()
            .checked_add(Duration::from_millis(c.budget.max_wall_millis))
            .ok_or_else(exhausted)?;
        let remaining = c
            .budget
            .max_bytes
            .checked_sub(LOCAL_RESERVE)
            .ok_or_else(exhausted)?;
        Ok(Self {
            context: c.clone(),
            deadline,
            remaining,
            refreshes: 0,
        })
    }
    fn context(&self) -> Result<OperationContext> {
        let millis = self
            .deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(exhausted)?
            .as_millis();
        if millis == 0 {
            return Err(exhausted());
        }
        let mut c = self.context.clone();
        c.budget.max_wall_millis = u64::try_from(millis)
            .map_err(|_| exhausted())?
            .min(c.budget.max_wall_millis);
        c.budget.max_bytes = self.remaining;
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        Ok(c)
    }
    // Preserve an already reserved native byte allowance and its grants. Only
    // advance its clock floor and narrow its wall deadline immediately before I/O.
    fn edge(&mut self, edge: &OperationContext) -> Result<OperationContext> {
        if edge.session_id != self.context.session_id
            || edge.anchor.fortress_id != self.context.anchor.fortress_id
        {
            return Err(custody());
        }
        self.context.anchor.tick =
            GameTick(self.context.anchor.tick.get().max(edge.anchor.tick.get()));
        let remaining = self.context()?;
        let mut current = edge.clone();
        current.anchor.tick = remaining.anchor.tick;
        current.budget.max_wall_millis = current
            .budget
            .max_wall_millis
            .min(remaining.budget.max_wall_millis);
        current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        Ok(current)
    }
    fn reserve(&mut self, bytes: u64) -> Result<OperationContext> {
        self.remaining = self
            .remaining
            .checked_sub(bytes)
            .filter(|left| *left > 0)
            .ok_or_else(exhausted)?;
        let mut c = self.context()?;
        c.budget.max_bytes = bytes;
        Ok(c)
    }
    fn native(&mut self, view: &WorkforceView) -> Result<OperationContext> {
        // WorkforceSession reserves one connection before its journal reserves
        // fresh observation plus prepare/commit RPCs. Cover those fixed costs
        // explicitly, in addition to bounded journal replay/append/readback.
        let bytes = 12u64
            .checked_mul(view_cost(view) + 4 * MAX_FRAME as u64)
            .and_then(|bytes| bytes.checked_add(CONNECT_BYTES + 2 * RPC_BYTES))
            .ok_or_else(exhausted)?;
        // Two refreshes and a final native view must still fit after dispatch.
        if self.remaining
            <= bytes + 2 * MAX_EVIDENCE_REFRESH_BYTES + 4 * (view_cost(view) + MAX_FRAME as u64)
        {
            return Err(exhausted());
        }
        self.reserve(bytes)
    }
    fn refresh<E: WorkforceEvidenceOwner>(
        &mut self,
        owner: &mut E,
        capture: &WorkforceCapture,
        edge: &OperationContext,
    ) -> Result<()> {
        if self.refreshes == 2 {
            return Err(exhausted());
        }
        self.refreshes += 1;
        self.context.anchor.tick = GameTick(
            self.context
                .anchor
                .tick
                .get()
                .max(edge.anchor.tick.get())
                .max(capture.tick()),
        );
        let c = self.reserve(MAX_EVIDENCE_REFRESH_BYTES)?;
        owner.refresh_after_capture(capture, &c)?;
        self.context()?;
        Ok(())
    }
}

pub struct SemanticWorkforceSession<S, B> {
    native: WorkforceSession<S>,
    associations: AssociationStore<B>,
    attached: Option<SemanticWorkforceReview>,
}
impl<S: EffectJournalStorage, B: EffectJournalStorage> SemanticWorkforceSession<S, B> {
    pub fn new(
        native: WorkforceSession<S>,
        associations: AssociationStore<B>,
        context: &OperationContext,
    ) -> Result<Self> {
        let mut out = Self {
            native,
            associations,
            attached: None,
        };
        let mut call = Call::new(context)?;
        out.view(&mut call)?;
        Ok(out)
    }
    fn view(&mut self, call: &mut Call) -> Result<WorkforceView> {
        self.associations.verify(&call.context()?)?;
        let view = self.native.view(&call.context()?)?;
        call.reserve(2 * view_cost(&view))?;
        if view.id != self.associations.native_journal() {
            return Err(custody());
        }
        for record in &view.records {
            let stored = self
                .associations
                .get(record.plan().key())
                .ok_or_else(custody)?;
            if stored.native != record.plan().digest()
                || stored.witness != record.plan().before().witness()
            {
                return Err(custody());
            }
        }
        call.context.anchor.tick =
            GameTick(call.context.anchor.tick.get().max(self.native.high_tick()));
        self.associations.verify(&call.context()?)?;
        Ok(view)
    }
    fn control_store(&self) -> Result<()> {
        if self.associations.is_read_only() {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "read-only semantic custody permits history and reconciliation, not control",
            ));
        }
        Ok(())
    }
    fn attached(&self, seal: Digest32) -> Result<SemanticWorkforceReview> {
        self.attached
            .as_ref()
            .filter(|review| review.seal == seal)
            .cloned()
            .ok_or_else(|| {
                conflict(
                    "exact original semantic plan and review seal must be attached in this session",
                )
            })
    }
    /// Historical native inventory is available without reconstructing any
    /// original plan. It never enables semantic dispatch or goal verification.
    pub fn inventory(&mut self, context: &OperationContext) -> Result<WorkforceView> {
        self.view(&mut Call::new(context)?)
    }

    /// Select through the EXISTING workforce owner, then independently refresh
    /// canonical evidence and resolve the original single-step plan.
    pub fn observe<N, F, E>(
        &mut self,
        plan: PreparedPlan,
        evidence: &mut E,
        context: &OperationContext,
        factory: F,
    ) -> Result<SemanticWorkforceReview>
    where
        N: WorkforceSource,
        F: FnOnce(&WorkforceBinding, &OperationContext) -> Result<N>,
        E: WorkforceEvidenceOwner,
    {
        self.attached = None;
        self.native.clear_selection();
        original(&plan)?;
        let mut call = Call::new(context)?;
        let view = self.view(&mut call)?;
        authorize_original(&plan, &call.context()?)?;
        let key = &plan.steps[0].idempotency_key;
        if view.records.iter().any(|record| record.plan().key() == key) {
            return Err(conflict(
                "native key already exists; only exact original-plan reattachment is permitted",
            ));
        }
        let Action::SetLabor { units, .. } = &plan.steps[0].action else {
            return Err(custody());
        };
        let ids = evidence
            .routing_evidence()?
            .resolve_units(units)?
            .iter()
            .map(|unit| unit.native_id)
            .collect::<Vec<_>>();
        let native_context = call.native(&view)?;
        let store = &mut self.associations;
        let guard_call = &mut call;
        let guard_original = &plan;
        let scoped = move |binding: &WorkforceBinding, c: &OperationContext| -> Result<_> {
            store.verify(&guard_call.context()?)?;
            let current = guard_call.edge(c)?;
            authorize_original(guard_original, &current)?;
            let native = factory(binding, &current)?;
            Ok(Scoped {
                native,
                original: guard_original,
                association: None,
                associations: store,
                call: guard_call,
            })
        };
        let capture = self.native.observe(&ids, &native_context, scoped)?;
        call.refresh(evidence, &capture, &native_context)?;
        let candidate =
            resolve_workforce_step(&plan, StepId::ZERO, &evidence.routing_evidence()?, &capture)?;
        let association = Association {
            key: key.clone(),
            semantic: plan.digest,
            step: StepId::ZERO,
            anchor: plan.anchor,
            source: candidate.source_digest(),
            native: candidate.native_plan().digest(),
            witness: capture.witness(),
        };
        if self
            .associations
            .get(key)
            .is_some_and(|old| old != &association)
        {
            return Err(conflict(
                "original key already binds another semantic plan or source",
            ));
        }
        let review = SemanticWorkforceReview {
            seal: association.seal(view.id),
            original: plan,
            native: candidate.native_plan().clone(),
            association,
        };
        self.view(&mut call)?;
        authorize_original(&review.original, &call.context()?)?;
        self.attached = Some(review.clone());
        Ok(review)
    }

    /// A restart retains native history and the semantic seal, never the
    /// original plan's authority. Reattach the exact plan before control or
    /// action-result interpretation. Current evidence is still required at commit.
    pub fn reattach(
        &mut self,
        plan: PreparedPlan,
        context: &OperationContext,
    ) -> Result<SemanticWorkforceReview> {
        self.attached = None;
        self.native.clear_selection();
        original(&plan)?;
        let mut call = Call::new(context)?;
        let view = self.view(&mut call)?;
        let key = &plan.steps[0].idempotency_key;
        let stored = self.associations.get(key).ok_or_else(custody)?.clone();
        if stored.semantic != plan.digest
            || stored.anchor != plan.anchor
            || stored.step != StepId::ZERO
        {
            return Err(conflict(
                "reattachment changed the original semantic digest, anchor or step",
            ));
        }
        let native = view
            .records
            .iter()
            .find(|record| record.plan().key() == key)
            .ok_or_else(|| {
                conflict(
                    "association precedes native intent; observe the exact original plan again",
                )
            })?
            .plan()
            .clone();
        let review = SemanticWorkforceReview {
            seal: stored.seal(view.id),
            original: plan,
            native,
            association: stored,
        };
        self.attached = Some(review.clone());
        Ok(review)
    }

    pub fn prepare<N, F, E>(
        &mut self,
        seal: Digest32,
        evidence: &mut E,
        context: &OperationContext,
        factory: F,
    ) -> Result<SemanticWorkforceResult>
    where
        N: WorkforceSource,
        F: FnOnce(&WorkforceBinding, &OperationContext) -> Result<N>,
        E: WorkforceEvidenceOwner,
    {
        self.control_store()?;
        let review = self.attached(seal)?;
        let mut call = Call::new(context)?;
        let view = self.view(&mut call)?;
        authorize_original(&review.original, &call.context()?)?;
        // Critically, missing semantic custody cannot adopt a pre-existing
        // native key, even when its native plan digest happens to match.
        if self.associations.get(review.native.key()).is_none()
            && view
                .records
                .iter()
                .any(|record| record.plan().key() == review.native.key())
        {
            return Err(custody());
        }
        let native_context = call.native(&view)?;
        self.associations
            .retain(review.association.clone(), &call.context()?)?;
        let native_context = call.edge(&native_context)?;
        authorize_original(&review.original, &native_context)?;
        let store = &mut self.associations;
        let guard_call = &mut call;
        let guard_review = &review;
        let guarded = move |binding: &WorkforceBinding, c: &OperationContext| -> Result<_> {
            store.verify(&guard_call.context()?)?;
            let current = guard_call.edge(c)?;
            authorize_original(&guard_review.original, &current)?;
            let native = factory(binding, &current)?;
            Ok(Guarded {
                native,
                evidence,
                review: guard_review,
                associations: store,
                call: guard_call,
            })
        };
        let record = self.native.prepare(
            review.native.key(),
            review.native.spec(),
            review.native.before().witness(),
            &native_context,
            guarded,
        )?;
        self.finish(review, record, &mut call)
    }

    pub fn commit<N, F, E>(
        &mut self,
        seal: Digest32,
        confirmed: bool,
        evidence: &mut E,
        context: &OperationContext,
        factory: F,
    ) -> Result<SemanticWorkforceResult>
    where
        N: WorkforceSource,
        F: FnOnce(&WorkforceBinding, &OperationContext) -> Result<N>,
        E: WorkforceEvidenceOwner,
    {
        self.control_store()?;
        let review = self.attached(seal)?;
        if !confirmed {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "confirm the exact semantic and native review seal for this commit",
            ));
        }
        let mut call = Call::new(context)?;
        let view = self.view(&mut call)?;
        self.exact_association(&review)?;
        authorize_original(&review.original, &call.context()?)?;
        let native_context = call.native(&view)?;
        let store = &mut self.associations;
        let guard_call = &mut call;
        let guard_review = &review;
        let guarded = move |binding: &WorkforceBinding, c: &OperationContext| -> Result<_> {
            store.verify(&guard_call.context()?)?;
            let current = guard_call.edge(c)?;
            authorize_original(&guard_review.original, &current)?;
            let native = factory(binding, &current)?;
            Ok(Guarded {
                native,
                evidence,
                review: guard_review,
                associations: store,
                call: guard_call,
            })
        };
        let record = self.native.commit(
            review.native.key(),
            review.native.digest(),
            true,
            &native_context,
            guarded,
        )?;
        self.finish(review, record, &mut call)
    }

    /// Reconciliation only queries the original native record. It cannot
    /// reacquire preparation or redispatch, including after Unknown.
    pub fn reconcile<N, F>(
        &mut self,
        seal: Digest32,
        context: &OperationContext,
        factory: F,
    ) -> Result<SemanticWorkforceResult>
    where
        N: WorkforceSource,
        F: FnOnce(&WorkforceBinding, &OperationContext) -> Result<N>,
    {
        let review = self.attached(seal)?;
        let mut call = Call::new(context)?;
        let view = self.view(&mut call)?;
        self.exact_association(&review)?;
        let c = call.native(&view)?;
        let store = &mut self.associations;
        let guard_call = &mut call;
        let guard_review = &review;
        let scoped = move |binding: &WorkforceBinding, c: &OperationContext| -> Result<_> {
            store.verify(&guard_call.context()?)?;
            let current = guard_call.edge(c)?;
            let native = factory(binding, &current)?;
            Ok(Scoped {
                native,
                original: &guard_review.original,
                association: Some(&guard_review.association),
                associations: store,
                call: guard_call,
            })
        };
        let record =
            self.native
                .reconcile(review.native.key(), review.native.digest(), &c, scoped)?;
        self.finish(review, record, &mut call)
    }
    pub fn inspect(
        &mut self,
        seal: Digest32,
        context: &OperationContext,
    ) -> Result<SemanticWorkforceResult> {
        let review = self.attached(seal)?;
        let mut call = Call::new(context)?;
        let view = self.view(&mut call)?;
        let record = view
            .records
            .iter()
            .find(|record| record.plan() == &review.native)
            .cloned()
            .ok_or_else(custody)?;
        self.finish(review, record, &mut call)
    }
    pub fn cancel<N, F>(
        &mut self,
        seal: Digest32,
        context: &OperationContext,
        factory: F,
    ) -> Result<SemanticWorkforceResult>
    where
        N: WorkforceSource,
        F: FnOnce(&WorkforceBinding, &OperationContext) -> Result<N>,
    {
        self.control_store()?;
        let review = self.attached(seal)?;
        let mut call = Call::new(context)?;
        let view = self.view(&mut call)?;
        self.exact_association(&review)?;
        // Cleanup needs current scoped authority, but an expired semantic plan
        // must not strand already prepared or uncertain native work.
        authorize_scope(&review.original, &call.context()?)?;
        let c = call.native(&view)?;
        let store = &mut self.associations;
        let guard_call = &mut call;
        let guard_review = &review;
        let scoped = move |binding: &WorkforceBinding, c: &OperationContext| -> Result<_> {
            store.verify(&guard_call.context()?)?;
            let current = guard_call.edge(c)?;
            authorize_scope(&guard_review.original, &current)?;
            let native = factory(binding, &current)?;
            Ok(Scoped {
                native,
                original: &guard_review.original,
                association: Some(&guard_review.association),
                associations: store,
                call: guard_call,
            })
        };
        let record = self
            .native
            .cancel(review.native.key(), review.native.digest(), &c, scoped)?;
        self.finish(review, record, &mut call)
    }
    fn exact_association(&self, review: &SemanticWorkforceReview) -> Result<()> {
        if self.associations.get(review.native.key()) != Some(&review.association)
            || review.seal != review.association.seal(self.associations.native_journal())
        {
            return Err(custody());
        }
        Ok(())
    }
    fn finish(
        &mut self,
        review: SemanticWorkforceReview,
        record: AssignmentRecord,
        call: &mut Call,
    ) -> Result<SemanticWorkforceResult> {
        self.view(call)?;
        self.exact_association(&review)?;
        if record.plan() != &review.native {
            return Err(custody());
        }
        let action = action_result(&review, &record)?;
        Ok(SemanticWorkforceResult {
            review,
            native: record,
            action,
        })
    }
}

// Only this wrapper reaches prepare/commit. Existing WorkforceJournal writes
// and syncs Intent / DispatchStarted before invoking these two methods.
struct Guarded<'a, N, E, B> {
    native: N,
    evidence: &'a mut E,
    review: &'a SemanticWorkforceReview,
    associations: &'a mut AssociationStore<B>,
    call: &'a mut Call,
}
impl<N: WorkforceSource, E: WorkforceEvidenceOwner, B: EffectJournalStorage> Guarded<'_, N, E, B> {
    fn check(&mut self, plan: &AssignmentPlan, context: &OperationContext) -> Result<()> {
        if plan != &self.review.native {
            return Err(custody());
        }
        self.associations.verify(&self.call.context()?)?;
        if self.associations.get(plan.key()) != Some(&self.review.association) {
            return Err(custody());
        }
        self.call.refresh(self.evidence, plan.before(), context)?;
        let scope = self.evidence.routing_evidence()?;
        let current =
            resolve_workforce_step(&self.review.original, StepId::ZERO, &scope, plan.before())?;
        if current.native_plan() != plan
            || current.source_digest() != self.review.association.source
        {
            return Err(conflict(
                "fresh semantic evidence differs from the original reviewed native binding",
            ));
        }
        authorize_original(&self.review.original, &self.call.context()?)?;
        self.associations.verify(&self.call.context()?)?;
        Ok(())
    }
}
impl<N: WorkforceSource, E: WorkforceEvidenceOwner, B: EffectJournalStorage> WorkforceSource
    for Guarded<'_, N, E, B>
{
    fn manifest(&self) -> &WorkforceManifest {
        self.native.manifest()
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        self.native.endpoint()
    }
    fn observe(&mut self, ids: &[u32], context: &OperationContext) -> Result<WorkforceCapture> {
        self.associations.verify(&self.call.context()?)?;
        let current = self.call.edge(context)?;
        authorize_original(&self.review.original, &current)?;
        let capture = self.native.observe(ids, &current)?;
        if &capture != self.review.native.before() {
            return Err(error(
                ErrorCode::StaleAnchor,
                "native workforce differs from the original semantic capture",
            ));
        }
        self.check(&self.review.native.clone(), context)?;
        Ok(capture)
    }
    fn prepare(
        &mut self,
        plan: &AssignmentPlan,
        context: &OperationContext,
    ) -> Result<AssignmentEffect> {
        self.check(plan, context)?;
        let current = self.call.edge(context)?;
        authorize_original(&self.review.original, &current)?;
        self.native.prepare(plan, &current)
    }
    fn commit(
        &mut self,
        plan: &AssignmentPlan,
        context: &OperationContext,
    ) -> Result<AssignmentEffect> {
        self.check(plan, context)?;
        let current = self.call.edge(context)?;
        authorize_original(&self.review.original, &current)?;
        self.native.commit(plan, &current)
    }
    fn query(
        &mut self,
        _: &AssignmentPlan,
        _: &OperationContext,
    ) -> Result<Option<AssignmentEffect>> {
        Err(refused(
            "effect preparation cannot be reused for recovery queries",
        ))
    }
    fn cancel(&mut self, _: &AssignmentPlan, _: &OperationContext) -> Result<AssignmentEffect> {
        Err(refused(
            "effect preparation cannot be reused for cancellation",
        ))
    }
}

// Read/recovery boundaries never gain preparation or commit authority. They
// still share the outer deadline, retain semantic custody, and check the exact
// canonical unit scope at native cancellation, including after plan expiry.
struct Scoped<'a, N, B> {
    native: N,
    original: &'a PreparedPlan,
    association: Option<&'a Association>,
    associations: &'a mut AssociationStore<B>,
    call: &'a mut Call,
}
impl<N: WorkforceSource, B: EffectJournalStorage> Scoped<'_, N, B> {
    fn current(&mut self, context: &OperationContext) -> Result<OperationContext> {
        self.associations.verify(&self.call.context()?)?;
        self.call.edge(context)
    }
    fn bound(&self, plan: &AssignmentPlan) -> Result<()> {
        let association = self.association.ok_or_else(custody)?;
        if association.key != plan.key()
            || association.native != plan.digest()
            || association.witness != plan.before().witness()
            || association.semantic != self.original.digest
            || association.anchor != self.original.anchor
            || self.associations.get(plan.key()) != Some(association)
        {
            return Err(custody());
        }
        Ok(())
    }
}
impl<N: WorkforceSource, B: EffectJournalStorage> WorkforceSource for Scoped<'_, N, B> {
    fn manifest(&self) -> &WorkforceManifest {
        self.native.manifest()
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        self.native.endpoint()
    }
    fn observe(&mut self, ids: &[u32], context: &OperationContext) -> Result<WorkforceCapture> {
        if self.association.is_some() {
            return Err(refused(
                "recovery cannot reacquire original effect preparation",
            ));
        }
        let current = self.current(context)?;
        authorize_original(self.original, &current)?;
        self.native.observe(ids, &current)
    }
    fn prepare(&mut self, _: &AssignmentPlan, _: &OperationContext) -> Result<AssignmentEffect> {
        Err(refused("read/recovery boundary cannot prepare an effect"))
    }
    fn commit(&mut self, _: &AssignmentPlan, _: &OperationContext) -> Result<AssignmentEffect> {
        Err(refused("read/recovery boundary cannot dispatch an effect"))
    }
    fn query(
        &mut self,
        plan: &AssignmentPlan,
        context: &OperationContext,
    ) -> Result<Option<AssignmentEffect>> {
        let current = self.current(context)?;
        self.bound(plan)?;
        self.native.query(plan, &current)
    }
    fn cancel(
        &mut self,
        plan: &AssignmentPlan,
        context: &OperationContext,
    ) -> Result<AssignmentEffect> {
        let current = self.current(context)?;
        self.bound(plan)?;
        authorize_scope(self.original, &current)?;
        self.native.cancel(plan, &current)
    }
}

fn action_result(
    review: &SemanticWorkforceReview,
    record: &AssignmentRecord,
) -> Result<SingleLaborResult> {
    let Some(effect) = record.effect() else {
        return Ok(SingleLaborResult::Unverified);
    };
    let effect = AssignmentEffect::decode(effect.canonical_bytes(), &review.native)?;
    if effect.phase() != AssignmentPhase::Applied {
        return Ok(SingleLaborResult::Unverified);
    }
    let Action::SetLabor { labor, enabled, .. } = &review.original.steps[0].action else {
        return Err(custody());
    };
    let column = review
        .native
        .before()
        .labor_keys()
        .iter()
        .position(|key| key == labor)
        .ok_or_else(custody)?;
    let exact = review
        .native
        .before()
        .citizens()
        .iter()
        .zip(effect.post_citizens())
        .all(|(before, after)| {
            before
                .labors()
                .iter()
                .zip(after.labors())
                .enumerate()
                .all(|(index, (old, new))| {
                    if index == column {
                        *new == u8::from(*enabled)
                    } else {
                        old == new
                    }
                })
        });
    Ok(if exact {
        SingleLaborResult::Verified {
            receipt: effect.receipt(),
        }
    } else {
        SingleLaborResult::AppliedOutsideSemantics {
            receipt: effect.receipt(),
        }
    })
}

#[cfg(test)]
mod tests;
