//! Cross-family live mutation dispatch for sealed semantic plans.
//!
//! The narrow live families already own their native preparation, journals,
//! confirmation and effect-specific reconciliation. This module adds the missing
//! deterministic coordinator above them: exactly one prepared/uncertain family
//! operation may exist at a time, dependencies advance only after verified
//! application, and a lost commit reply blocks all later work until the original
//! idempotency key is reconciled. It performs no native I/O by itself and grants
//! no authority; hosts implement [LiveFamilyExecutor] with the existing family
//! coordinators.
//!
//! Bead: df-dfhack-bridge-plane-c-pic.4.

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, OperationContext, Result, StepId};
use dfmcp_intent::{PlanStep, PreparedPlan};

use crate::live_routing::{LiveFamily, RoutedStep, route_plan};

pub mod durable;

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

fn invalid(message: impl Into<String>) -> DfmcpError {
    error(ErrorCode::InvalidPlan, message)
}

fn conflict(message: impl Into<String>) -> DfmcpError {
    error(ErrorCode::Conflict, message)
}

fn indeterminate(message: impl Into<String>) -> DfmcpError {
    error(ErrorCode::EffectIndeterminate, message)
}

/// Opaque family preparation identity. The family implementation remains
/// responsible for validating its own native plan and durable preparation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FamilyPreparation {
    pub family: LiveFamily,
    pub digest: Digest32,
}

impl FamilyPreparation {
    pub fn new(family: LiveFamily, digest: Digest32) -> Result<Self> {
        if digest == Digest32::ZERO {
            return Err(invalid("live family preparation identity must be nonzero"));
        }
        Ok(Self { family, digest })
    }
}

/// Opaque verified family receipt. It records only cross-family sequencing
/// evidence; effect-specific proof stays in the family's own journal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FamilyReceipt {
    pub family: LiveFamily,
    pub digest: Digest32,
}

impl FamilyReceipt {
    pub fn new(family: LiveFamily, digest: Digest32) -> Result<Self> {
        if digest == Digest32::ZERO {
            return Err(invalid("live family receipt identity must be nonzero"));
        }
        Ok(Self { family, digest })
    }
}

/// Result of querying the exact original family operation after uncertainty.
/// NotDispatched is stronger than absence: the family must prove its durable
/// dispatch marker was never synchronized before the coordinator permits a
/// fresh preparation of that semantic step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReconcileOutcome {
    Applied(FamilyReceipt),
    Failed(Option<FamilyReceipt>),
    NotDispatched,
    Indeterminate,
}

/// Host bridge from the cross-family coordinator to the existing narrow family
/// owners. Implementations must use the exact semantic step idempotency key and
/// may not invent another native operation key.
pub trait LiveFamilyExecutor {
    fn prepare(
        &mut self,
        plan: &PreparedPlan,
        step: &PlanStep,
        route: &RoutedStep,
        context: &OperationContext,
    ) -> Result<FamilyPreparation>;

    fn commit(
        &mut self,
        plan: &PreparedPlan,
        step: &PlanStep,
        route: &RoutedStep,
        preparation: FamilyPreparation,
        context: &OperationContext,
    ) -> Result<FamilyReceipt>;

    fn reconcile(
        &mut self,
        plan: &PreparedPlan,
        step: &PlanStep,
        route: &RoutedStep,
        context: &OperationContext,
    ) -> Result<ReconcileOutcome>;

    /// Retire a still-prepared operation. This is not compensation and must not
    /// undo an effect that may have started.
    fn cancel_prepared(
        &mut self,
        plan: &PreparedPlan,
        step: &PlanStep,
        route: &RoutedStep,
        preparation: FamilyPreparation,
        context: &OperationContext,
    ) -> Result<Option<FamilyReceipt>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchState {
    Pending,
    Prepared(FamilyPreparation),
    DispatchStarted(FamilyPreparation),
    Applied(FamilyReceipt),
    Failed(Option<FamilyReceipt>),
    Cancelled(Option<FamilyReceipt>),
    Indeterminate(FamilyPreparation),
}

impl DispatchState {
    #[must_use]
    pub const fn terminal(self) -> bool {
        matches!(
            self,
            Self::Applied(_) | Self::Failed(_) | Self::Cancelled(_)
        )
    }

    #[must_use]
    pub const fn uncertain(self) -> bool {
        matches!(self, Self::DispatchStarted(_) | Self::Indeterminate(_))
    }

    #[must_use]
    pub const fn applied(self) -> bool {
        matches!(self, Self::Applied(_))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DispatchEntry {
    pub step: StepId,
    pub idempotency_key: String,
    pub family: LiveFamily,
    pub depends_on: Vec<StepId>,
    pub state: DispatchState,
}

#[derive(Clone, Debug)]
struct CompiledStep {
    semantic: PlanStep,
    route: RoutedStep,
}

/// One in-memory execution frontier for one immutable semantic plan. Durable
/// family journals remain the source of truth after process loss; a host can
/// rebuild this frontier by reconciling those exact original keys.
pub struct LiveDispatchCoordinator {
    plan: PreparedPlan,
    steps: Vec<CompiledStep>,
    entries: Vec<DispatchEntry>,
}

impl LiveDispatchCoordinator {
    /// Compile a fully routable sealed plan. Unsupported action families are
    /// rejected before any executor can be called.
    pub fn new(plan: PreparedPlan) -> Result<Self> {
        plan.validate_structure()?;
        let routed = route_plan(&plan)?;
        if routed.plan_digest != plan.digest || routed.steps.len() != plan.steps.len() {
            return Err(invalid("live route does not match the sealed semantic plan"));
        }

        let mut steps = Vec::with_capacity(plan.steps.len());
        let mut entries = Vec::with_capacity(plan.steps.len());
        for (semantic, route) in plan.steps.iter().cloned().zip(routed.steps) {
            if semantic.id != route.step || semantic.idempotency_key != route.idempotency_key {
                return Err(invalid("live route changed semantic step identity"));
            }
            let routed = route.outcome.map_err(|refusal| {
                error(
                    ErrorCode::AdapterRejected,
                    format!(
                        "semantic step {} has no executable live route: {}",
                        semantic.id.get(),
                        refusal.reason
                    ),
                )
            })?;
            entries.push(DispatchEntry {
                step: semantic.id,
                idempotency_key: semantic.idempotency_key.clone(),
                family: routed.family,
                depends_on: semantic.depends_on.clone(),
                state: DispatchState::Pending,
            });
            steps.push(CompiledStep {
                semantic,
                route: routed,
            });
        }
        Ok(Self {
            plan,
            steps,
            entries,
        })
    }

    #[must_use]
    pub const fn plan(&self) -> &PreparedPlan {
        &self.plan
    }

    #[must_use]
    pub fn entries(&self) -> &[DispatchEntry] {
        &self.entries
    }

    #[must_use]
    pub fn all_applied(&self) -> bool {
        self.entries.iter().all(|entry| entry.state.applied())
    }

    #[must_use]
    pub fn has_uncertain_effect(&self) -> bool {
        self.entries.iter().any(|entry| entry.state.uncertain())
    }

    /// Stable identity of the complete cross-family frontier. Hosts can bind
    /// this digest into their own durable handoff without treating it as a
    /// family receipt or mutation authority.
    #[must_use]
    pub fn frontier_digest(&self) -> Digest32 {
        fn family_code(family: LiveFamily) -> u8 {
            match family {
                LiveFamily::ControlV1_7 => 1,
                LiveFamily::DigV1_16 => 2,
                LiveFamily::BuildV1_19 => 3,
                LiveFamily::WorkOrdersV1_10 => 4,
                LiveFamily::WorkforceV1_17 => 5,
            }
        }
        let mut bytes = b"dfmcp-live-dispatch-frontier/1".to_vec();
        bytes.extend_from_slice(self.plan.digest.as_bytes());
        bytes.extend_from_slice(&(self.entries.len() as u64).to_be_bytes());
        for entry in &self.entries {
            bytes.extend_from_slice(&entry.step.get().to_be_bytes());
            bytes.push(family_code(entry.family));
            bytes.extend_from_slice(&(entry.idempotency_key.len() as u64).to_be_bytes());
            bytes.extend_from_slice(entry.idempotency_key.as_bytes());
            bytes.extend_from_slice(&(entry.depends_on.len() as u64).to_be_bytes());
            for dependency in &entry.depends_on {
                bytes.extend_from_slice(&dependency.get().to_be_bytes());
            }
            match entry.state {
                DispatchState::Pending => bytes.push(0),
                DispatchState::Prepared(preparation) => {
                    bytes.push(1);
                    bytes.extend_from_slice(preparation.digest.as_bytes());
                }
                DispatchState::DispatchStarted(preparation) => {
                    bytes.push(2);
                    bytes.extend_from_slice(preparation.digest.as_bytes());
                }
                DispatchState::Applied(receipt) => {
                    bytes.push(3);
                    bytes.extend_from_slice(receipt.digest.as_bytes());
                }
                DispatchState::Failed(receipt) => {
                    bytes.push(4);
                    match receipt {
                        Some(receipt) => {
                            bytes.push(1);
                            bytes.extend_from_slice(receipt.digest.as_bytes());
                        }
                        None => bytes.push(0),
                    }
                }
                DispatchState::Cancelled(receipt) => {
                    bytes.push(5);
                    match receipt {
                        Some(receipt) => {
                            bytes.push(1);
                            bytes.extend_from_slice(receipt.digest.as_bytes());
                        }
                        None => bytes.push(0),
                    }
                }
                DispatchState::Indeterminate(preparation) => {
                    bytes.push(6);
                    bytes.extend_from_slice(preparation.digest.as_bytes());
                }
            }
        }
        Digest32::of_bytes(&bytes)
    }

    fn index(&self, step: StepId) -> Result<usize> {
        self.entries
            .iter()
            .position(|entry| entry.step == step)
            .ok_or_else(|| invalid("semantic step is absent from the dispatch frontier"))
    }

    fn active_index(&self) -> Option<usize> {
        self.entries.iter().position(|entry| {
            matches!(
                entry.state,
                DispatchState::Prepared(_)
                    | DispatchState::DispatchStarted(_)
                    | DispatchState::Indeterminate(_)
            )
        })
    }

    fn dependencies_applied(&self, index: usize) -> Result<bool> {
        self.entries[index]
            .depends_on
            .iter()
            .map(|dependency| {
                let dependency = self.index(*dependency)?;
                Ok(self.entries[dependency].state.applied())
            })
            .try_fold(true, |all, applied| applied.map(|applied| all && applied))
    }

    /// First executable step in canonical plan order. A prepared or uncertain
    /// operation blocks the entire frontier even when later steps are otherwise
    /// independent; this prevents cross-family effects from racing an unknown
    /// outcome without a durable multi-owner transaction protocol.
    pub fn next_ready(&self) -> Result<Option<StepId>> {
        if self.active_index().is_some() {
            return Ok(None);
        }
        for (index, entry) in self.entries.iter().enumerate() {
            if entry.state == DispatchState::Pending && self.dependencies_applied(index)? {
                return Ok(Some(entry.step));
            }
        }
        Ok(None)
    }

    fn authorize_step(&self, index: usize, context: &OperationContext) -> Result<()> {
        let step = &self.steps[index].semantic;
        let scope = step.action.scope();
        context.authorize(
            step.required_capability,
            step.risk,
            &scope.entity_ids,
            scope.map_area,
        )?;
        if context.anchor.fortress_id != self.plan.anchor.fortress_id {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "live dispatch context belongs to another fortress",
            ));
        }
        if context.anchor.tick > self.plan.expires_at_tick {
            return Err(error(
                ErrorCode::StaleAnchor,
                "sealed semantic plan expired before live dispatch",
            ));
        }
        Ok(())
    }

    /// Prepare only the deterministic current frontier step. Family preparation
    /// is treated as non-effectful: an error leaves this coordinator Pending.
    pub fn prepare_next<E: LiveFamilyExecutor>(
        &mut self,
        executor: &mut E,
        context: &OperationContext,
    ) -> Result<FamilyPreparation> {
        if let Some(index) = self.active_index() {
            return Err(conflict(format!(
                "semantic step {} already owns the live dispatch frontier",
                self.entries[index].step.get()
            )));
        }
        let step = self
            .next_ready()?
            .ok_or_else(|| conflict("no dependency-satisfied live step is ready"))?;
        let index = self.index(step)?;
        self.authorize_step(index, context)?;
        let prepared = executor.prepare(
            &self.plan,
            &self.steps[index].semantic,
            &self.steps[index].route,
            context,
        )?;
        if prepared.family != self.entries[index].family {
            return Err(invalid("family executor prepared the wrong live family"));
        }
        if prepared.digest == Digest32::ZERO {
            return Err(invalid("family executor returned a zero preparation identity"));
        }
        self.entries[index].state = DispatchState::Prepared(prepared);
        Ok(prepared)
    }

    /// Commit the exact prepared step. DispatchStarted is recorded before the
    /// family call. Therefore any error from commit becomes Indeterminate and
    /// cannot be retried by another call to this method.
    pub fn commit_prepared<E: LiveFamilyExecutor>(
        &mut self,
        step: StepId,
        executor: &mut E,
        context: &OperationContext,
    ) -> Result<FamilyReceipt> {
        let index = self.index(step)?;
        let DispatchState::Prepared(preparation) = self.entries[index].state else {
            return Err(if self.entries[index].state.uncertain() {
                indeterminate("original live dispatch is uncertain; reconcile its exact key")
            } else {
                conflict("semantic step is not the current prepared live operation")
            });
        };
        self.authorize_step(index, context)?;
        self.entries[index].state = DispatchState::DispatchStarted(preparation);
        match executor.commit(
            &self.plan,
            &self.steps[index].semantic,
            &self.steps[index].route,
            preparation,
            context,
        ) {
            Ok(receipt) if receipt.family == self.entries[index].family && receipt.digest != Digest32::ZERO => {
                self.entries[index].state = DispatchState::Applied(receipt);
                Ok(receipt)
            }
            Ok(_) => {
                self.entries[index].state = DispatchState::Indeterminate(preparation);
                Err(invalid("family executor returned a mismatched or zero receipt"))
            }
            Err(cause) => {
                self.entries[index].state = DispatchState::Indeterminate(preparation);
                Err(cause)
            }
        }
    }

    /// Query the exact original family key after an uncertain dispatch. Only an
    /// explicit NotDispatched result makes the same semantic step Pending again.
    pub fn reconcile<E: LiveFamilyExecutor>(
        &mut self,
        step: StepId,
        executor: &mut E,
        context: &OperationContext,
    ) -> Result<ReconcileOutcome> {
        let index = self.index(step)?;
        if !self.entries[index].state.uncertain() {
            return Err(conflict("semantic step has no uncertain live dispatch to reconcile"));
        }
        self.authorize_step(index, context)?;
        let outcome = executor.reconcile(
            &self.plan,
            &self.steps[index].semantic,
            &self.steps[index].route,
            context,
        )?;
        match outcome {
            ReconcileOutcome::Applied(receipt)
                if receipt.family == self.entries[index].family
                    && receipt.digest != Digest32::ZERO =>
            {
                self.entries[index].state = DispatchState::Applied(receipt);
            }
            ReconcileOutcome::Failed(receipt) => {
                if receipt.is_some_and(|receipt| {
                    receipt.family != self.entries[index].family || receipt.digest == Digest32::ZERO
                }) {
                    return Err(invalid("reconciliation returned an invalid family receipt"));
                }
                self.entries[index].state = DispatchState::Failed(receipt);
            }
            ReconcileOutcome::NotDispatched => {
                self.entries[index].state = DispatchState::Pending;
            }
            ReconcileOutcome::Indeterminate => {
                let preparation = match self.entries[index].state {
                    DispatchState::DispatchStarted(value) | DispatchState::Indeterminate(value) => value,
                    _ => return Err(conflict("original live dispatch no longer has an uncertain state")),
                };
                self.entries[index].state = DispatchState::Indeterminate(preparation);
            }
            ReconcileOutcome::Applied(_) => {
                return Err(invalid("reconciliation returned a receipt for another family"));
            }
        }
        Ok(outcome)
    }

    /// Retire a prepared operation before dispatch. Once DispatchStarted exists,
    /// cancellation must go through the family's recovery/safety path instead of
    /// pretending the effect can be locally undone.
    pub fn cancel_prepared<E: LiveFamilyExecutor>(
        &mut self,
        step: StepId,
        executor: &mut E,
        context: &OperationContext,
    ) -> Result<Option<FamilyReceipt>> {
        let index = self.index(step)?;
        let DispatchState::Prepared(preparation) = self.entries[index].state else {
            return Err(if self.entries[index].state.uncertain() {
                indeterminate("dispatch may have started; use exact family reconciliation or safety cancellation")
            } else {
                conflict("semantic step is not a cancellable prepared live operation")
            });
        };
        self.authorize_step(index, context)?;
        let receipt = executor.cancel_prepared(
            &self.plan,
            &self.steps[index].semantic,
            &self.steps[index].route,
            preparation,
            context,
        )?;
        if receipt.is_some_and(|receipt| {
            receipt.family != self.entries[index].family || receipt.digest == Digest32::ZERO
        }) {
            return Err(invalid("prepared cancellation returned an invalid family receipt"));
        }
        self.entries[index].state = DispatchState::Cancelled(receipt);
        Ok(receipt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dfmcp_core::{
        Capability, CapabilityGrant, CapabilityScope, FortressId, GameTick, IntentId,
        ObservationCursor, RequestId, RiskTier, SessionId, StateAnchor, WorkBudget,
    };
    use dfmcp_intent::{Action, PlanStep, PreparedPlan};
    use dfmcp_world::Predicate;
    use std::collections::BTreeSet;

    fn anchor() -> StateAnchor {
        StateAnchor {
            fortress_id: FortressId::new(9),
            cursor: ObservationCursor { epoch: 1, sequence: 7 },
            tick: GameTick(100),
            state_hash: Digest32::of_bytes(b"live-dispatch-test-anchor"),
        }
    }

    fn step(id: u32, action: Action, depends_on: Vec<StepId>) -> PlanStep {
        let id = StepId::new(id);
        let key = dfmcp_intent::derive_step_idempotency_key(
            IntentId::new(44),
            anchor(),
            id,
            &action,
        );
        PlanStep {
            id,
            required_capability: action.capability(),
            risk: action.risk(),
            action,
            preconditions: Vec::new(),
            postconditions: vec![Predicate::Paused(true)],
            compensation: None,
            obligation: None,
            depends_on,
            idempotency_key: key,
        }
    }

    fn plan() -> PreparedPlan {
        let first = step(0, Action::Pause { paused: true }, Vec::new());
        let second = step(
            1,
            Action::SetLabor {
                units: vec![dfmcp_core::EntityId::new(17)],
                labor: "MINE".to_owned(),
                enabled: true,
            },
            vec![first.id],
        );
        let mut required = BTreeSet::new();
        required.insert(first.required_capability);
        required.insert(second.required_capability);
        PreparedPlan::builder(
            IntentId::new(44),
            anchor(),
            "mixed live dispatch",
            Predicate::Paused(true),
        )
        .steps(vec![first, second])
        .max_risk(RiskTier::Reversible)
        .required_capabilities(required)
        .expires_at_tick(GameTick(500))
        .build()
    }

    fn context() -> OperationContext {
        let mut grants = Vec::new();
        for capability in [Capability::ControlClock, Capability::ConfigureLabor] {
            grants.push(CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(anchor().fortress_id),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::Guarded,
                expires_at_tick: None,
                remaining_uses: None,
            });
        }
        OperationContext {
            session_id: SessionId::new(8),
            request_id: RequestId::new(1),
            anchor: anchor(),
            grants,
            budget: WorkBudget {
                max_wall_millis: 1000,
                max_game_ticks: 0,
                max_bytes: 1024 * 1024,
                max_entities: 64,
                max_output_tokens: 4096,
                max_actions: 2,
            },
            cancellation_requested: false,
        }
    }

    #[derive(Default)]
    struct Fake {
        calls: Vec<String>,
        fail_commit: bool,
        reconciliation: Option<ReconcileOutcome>,
    }

    impl LiveFamilyExecutor for Fake {
        fn prepare(
            &mut self,
            _: &PreparedPlan,
            step: &PlanStep,
            route: &RoutedStep,
            _: &OperationContext,
        ) -> Result<FamilyPreparation> {
            self.calls.push(format!("prepare:{}", step.id.get()));
            FamilyPreparation::new(
                route.family,
                Digest32::of_bytes(format!("prepared:{}", step.id.get()).as_bytes()),
            )
        }

        fn commit(
            &mut self,
            _: &PreparedPlan,
            step: &PlanStep,
            route: &RoutedStep,
            _: FamilyPreparation,
            _: &OperationContext,
        ) -> Result<FamilyReceipt> {
            self.calls.push(format!("commit:{}", step.id.get()));
            if self.fail_commit {
                return Err(error(ErrorCode::AdapterUnavailable, "lost family commit reply"));
            }
            FamilyReceipt::new(
                route.family,
                Digest32::of_bytes(format!("applied:{}", step.id.get()).as_bytes()),
            )
        }

        fn reconcile(
            &mut self,
            _: &PreparedPlan,
            step: &PlanStep,
            _: &RoutedStep,
            _: &OperationContext,
        ) -> Result<ReconcileOutcome> {
            self.calls.push(format!("reconcile:{}", step.id.get()));
            self.reconciliation
                .take()
                .ok_or_else(|| indeterminate("fixture has no reconciliation result"))
        }

        fn cancel_prepared(
            &mut self,
            _: &PreparedPlan,
            step: &PlanStep,
            route: &RoutedStep,
            _: FamilyPreparation,
            _: &OperationContext,
        ) -> Result<Option<FamilyReceipt>> {
            self.calls.push(format!("cancel:{}", step.id.get()));
            Ok(Some(FamilyReceipt::new(
                route.family,
                Digest32::of_bytes(format!("cancelled:{}", step.id.get()).as_bytes()),
            )?))
        }
    }

    #[test]
    fn mixed_families_advance_only_after_verified_dependency() -> Result<()> {
        let mut coordinator = LiveDispatchCoordinator::new(plan())?;
        let mut fake = Fake::default();
        let c = context();
        assert_eq!(coordinator.next_ready()?, Some(StepId::new(0)));
        let first = coordinator.prepare_next(&mut fake, &c)?;
        assert_eq!(first.family, LiveFamily::ControlV1_7);
        assert_eq!(coordinator.next_ready()?, None);
        coordinator.commit_prepared(StepId::new(0), &mut fake, &c)?;
        assert_eq!(coordinator.next_ready()?, Some(StepId::new(1)));
        let second = coordinator.prepare_next(&mut fake, &c)?;
        assert_eq!(second.family, LiveFamily::WorkforceV1_17);
        coordinator.commit_prepared(StepId::new(1), &mut fake, &c)?;
        assert!(coordinator.all_applied());
        assert_eq!(
            fake.calls,
            ["prepare:0", "commit:0", "prepare:1", "commit:1"]
        );
        Ok(())
    }

    #[test]
    fn lost_commit_reply_blocks_every_later_family_until_exact_reconciliation() -> Result<()> {
        let mut coordinator = LiveDispatchCoordinator::new(plan())?;
        let mut fake = Fake::default();
        let c = context();
        coordinator.prepare_next(&mut fake, &c)?;
        fake.fail_commit = true;
        assert!(coordinator
            .commit_prepared(StepId::new(0), &mut fake, &c)
            .is_err());
        assert!(coordinator.has_uncertain_effect());
        assert_eq!(coordinator.next_ready()?, None);
        assert!(coordinator.prepare_next(&mut fake, &c).is_err());
        assert!(coordinator
            .commit_prepared(StepId::new(0), &mut fake, &c)
            .is_err());

        fake.reconciliation = Some(ReconcileOutcome::Applied(FamilyReceipt::new(
            LiveFamily::ControlV1_7,
            Digest32::of_bytes(b"reconciled-pause"),
        )?));
        let outcome = coordinator.reconcile(StepId::new(0), &mut fake, &c)?;
        assert!(matches!(outcome, ReconcileOutcome::Applied(_)));
        assert_eq!(coordinator.next_ready()?, Some(StepId::new(1)));
        Ok(())
    }

    #[test]
    fn explicit_no_send_is_the_only_uncertain_path_back_to_pending() -> Result<()> {
        let mut coordinator = LiveDispatchCoordinator::new(plan())?;
        let mut fake = Fake::default();
        let c = context();
        coordinator.prepare_next(&mut fake, &c)?;
        fake.fail_commit = true;
        assert!(coordinator
            .commit_prepared(StepId::new(0), &mut fake, &c)
            .is_err());
        fake.reconciliation = Some(ReconcileOutcome::NotDispatched);
        assert_eq!(
            coordinator.reconcile(StepId::new(0), &mut fake, &c)?,
            ReconcileOutcome::NotDispatched
        );
        assert_eq!(coordinator.next_ready()?, Some(StepId::new(0)));
        assert!(matches!(
            coordinator.entries()[0].state,
            DispatchState::Pending
        ));
        Ok(())
    }

    #[test]
    fn prepared_cancellation_never_claims_to_undo_a_started_effect() -> Result<()> {
        let mut coordinator = LiveDispatchCoordinator::new(plan())?;
        let mut fake = Fake::default();
        let c = context();
        coordinator.prepare_next(&mut fake, &c)?;
        let receipt = coordinator
            .cancel_prepared(StepId::new(0), &mut fake, &c)?
            .ok_or_else(|| invalid("missing cancellation receipt"))?;
        assert_eq!(receipt.family, LiveFamily::ControlV1_7);
        assert!(matches!(
            coordinator.entries()[0].state,
            DispatchState::Cancelled(_)
        ));

        let mut coordinator = LiveDispatchCoordinator::new(plan())?;
        coordinator.prepare_next(&mut fake, &c)?;
        fake.fail_commit = true;
        assert!(coordinator
            .commit_prepared(StepId::new(0), &mut fake, &c)
            .is_err());
        assert!(coordinator
            .cancel_prepared(StepId::new(0), &mut fake, &c)
            .is_err());
        assert!(matches!(
            coordinator.entries()[0].state,
            DispatchState::Indeterminate(_)
        ));
        Ok(())
    }

    #[test]
    fn unsupported_family_is_rejected_before_executor_contact() {
        let action = Action::SetStandingOrder {
            key: "autolabor".to_owned(),
            value: "enabled".to_owned(),
        };
        let unsupported = step(0, action, Vec::new());
        let mut required = BTreeSet::new();
        required.insert(unsupported.required_capability);
        let plan = PreparedPlan::builder(
            IntentId::new(44),
            anchor(),
            "unsupported live dispatch",
            Predicate::Paused(true),
        )
        .steps(vec![unsupported])
        .max_risk(RiskTier::Guarded)
        .required_capabilities(required)
        .expires_at_tick(GameTick(500))
        .build();
        assert!(LiveDispatchCoordinator::new(plan).is_err());
    }

    #[test]
    fn frontier_digest_binds_state_without_becoming_family_authority() -> Result<()> {
        let mut coordinator = LiveDispatchCoordinator::new(plan())?;
        let mut fake = Fake::default();
        let c = context();
        let initial = coordinator.frontier_digest();
        coordinator.prepare_next(&mut fake, &c)?;
        let prepared = coordinator.frontier_digest();
        assert_ne!(prepared, initial);
        coordinator.commit_prepared(StepId::new(0), &mut fake, &c)?;
        let applied = coordinator.frontier_digest();
        assert_ne!(applied, prepared);
        assert_ne!(applied, initial);
        assert_eq!(coordinator.plan().digest, plan().digest);
        Ok(())
    }

    #[test]
    fn authority_is_rechecked_at_prepare_and_commit() -> Result<()> {
        let mut coordinator = LiveDispatchCoordinator::new(plan())?;
        let mut fake = Fake::default();
        let mut c = context();
        c.grants
            .retain(|grant| grant.capability != Capability::ControlClock);
        assert!(coordinator.prepare_next(&mut fake, &c).is_err());
        assert!(fake.calls.is_empty());

        let mut c = context();
        coordinator.prepare_next(&mut fake, &c)?;
        c.grants
            .retain(|grant| grant.capability != Capability::ControlClock);
        assert!(coordinator
            .commit_prepared(StepId::new(0), &mut fake, &c)
            .is_err());
        assert!(matches!(
            coordinator.entries()[0].state,
            DispatchState::Prepared(_)
        ));
        Ok(())
    }
}
