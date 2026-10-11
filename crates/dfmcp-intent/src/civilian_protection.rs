//! Guarded civilian protection through ordinary semantic plans.
//!
//! This configures named civilians; it does not prove evacuation, safe terrain,
//! complete population coverage, or threat clearance. Cancellation retains
//! protection instead of synthesizing a release of pre-existing burrow members.

use std::collections::BTreeSet;

use dfmcp_core::{
    Capability, DfmcpError, EntityId, ErrorCode, FortressId, IntentId, OperationContext, Result,
    RiskTier, StateAnchor,
};
use dfmcp_world::{CompareOp, EntityKind, Predicate, PredicateEvidence, PredicateTruth, Value};

use crate::{Action, Constraint, Intent, PreparedPlan, RequestedAction, StaticPlanner, effects};

pub const MAX_PROTECTED_CIVILIANS: usize = 64;
pub const MAX_PROTECTION_BYTES: usize = 256 * 1024;
pub const CIVILIAN_RESTRICTION: &str = "CIVILIAN_BURROW_RESTRICTION";

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}
fn exhausted() -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, "civilian protection exceeds its call allowance")
}
fn eq(id: EntityId, field: String, value: Value) -> Predicate {
    Predicate::FieldCompare { entity_id: id, field, op: CompareOp::Eq, value }
}
fn membership(id: EntityId, burrow: EntityId) -> Predicate {
    eq(id, format!("{}{}", effects::BURROW_FIELD_PREFIX, burrow.get()), Value::Bool(true))
}

/// Current configuration at exactly one authorized observation. Membership
/// counts are not arrival counts. False and unknown goals remain distinct.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtectionProgress {
    pub anchor: StateAnchor,
    pub requested_civilians: usize,
    pub assigned: usize,
    pub unassigned: usize,
    pub unknown_memberships: usize,
    pub configuration: PredicateTruth,
}

/// Immutable bounded lowering. The fortress must already be observed paused;
/// use the ordinary pause plan first when it is running. This prevents time
/// advancing between restriction activation and membership configuration.
#[derive(Clone, Debug)]
pub struct CivilianProtectionPlan {
    burrow: EntityId,
    civilians: Vec<EntityId>,
    steps: Vec<RequestedAction>,
    membership_goal: Predicate,
    encoded_bytes: usize,
}

impl CivilianProtectionPlan {
    pub fn compile(burrow: EntityId, civilians: &[EntityId]) -> Result<Self> {
        if civilians.is_empty() || civilians.len() > MAX_PROTECTED_CIVILIANS {
            return Err(exhausted());
        }
        let unique: BTreeSet<_> = civilians.iter().copied().collect();
        if burrow == EntityId::NIL || unique.contains(&EntityId::NIL)
            || unique.contains(&burrow) || unique.len() != civilians.len()
        {
            return Err(invalid("protection requires a distinct burrow and unique nonzero civilians"));
        }
        let civilians: Vec<_> = unique.into_iter().collect();
        let mut guards = vec![
            Predicate::Paused(true),
            Predicate::EntityKind { entity_id: burrow, kind: EntityKind::Burrow },
        ];
        for id in &civilians {
            guards.push(Predicate::EntityKind { entity_id: *id, kind: EntityKind::Unit });
            guards.push(eq(*id, "alive".to_owned(), Value::Bool(true)));
        }
        let guard = Predicate::All(guards).normalized();
        let mut target = vec![guard.clone()];
        target.extend(civilians.iter().map(|id| membership(*id, burrow)));
        let membership_goal = Predicate::All(target).normalized();
        let restriction = Action::SetStandingOrder {
            key: CIVILIAN_RESTRICTION.to_owned(), value: "ACTIVE".to_owned(),
        };
        let assignment = Action::SetBurrowMembership {
            units: civilians.clone(), burrow, assigned: true,
        };
        let steps = vec![
            RequestedAction {
                action: restriction.clone(),
                preconditions: vec![guard.clone()],
                // The ordinary planner binds this to the current fortress's
                // settings entity using the reference action semantics.
                postconditions: Vec::new(),
                compensation: Some(restriction),
                obligation: None,
                depends_on: Vec::new(),
            },
            RequestedAction {
                action: assignment.clone(),
                preconditions: vec![guard],
                postconditions: vec![membership_goal.clone()],
                // Reassert, never invert: cancellation is not threat clearance.
                // Unchanged existing members must never be removed by an undo.
                compensation: Some(assignment),
                obligation: None,
                depends_on: vec![0],
            },
        ];
        let mut encoded_bytes = 4096 + civilians.len() * 32;
        for predicate in steps.iter().flat_map(|step| {
            step.preconditions.iter().chain(&step.postconditions)
        }).chain(std::iter::once(&membership_goal)) {
            predicate.validate_shape()?;
            encoded_bytes += predicate.canonical_bytes().len();
        }
        // Reserve duplicated goal/step representations plus generated defaults.
        encoded_bytes = encoded_bytes.checked_mul(4).ok_or_else(exhausted)?;
        if encoded_bytes > MAX_PROTECTION_BYTES { return Err(exhausted()); }
        Ok(Self { burrow, civilians, steps, membership_goal, encoded_bytes })
    }

    #[must_use]
    pub fn civilians(&self) -> &[EntityId] { &self.civilians }
    #[must_use]
    pub const fn burrow(&self) -> EntityId { self.burrow }
    #[must_use]
    pub fn requested_actions(&self) -> &[RequestedAction] { &self.steps }
    #[must_use]
    pub const fn encoded_bytes(&self) -> usize { self.encoded_bytes }

    /// For the existing MCP action-intent compiler, which constructs its
    /// terminal from all action defaults. The last step additionally preserves
    /// every named citizen's liveness, kind, membership and paused-world proof.
    #[must_use]
    pub fn into_requested_actions(self) -> Vec<RequestedAction> { self.steps }

    fn goal(&self, fortress: FortressId) -> Predicate {
        let mut predicates = vec![self.membership_goal.clone()];
        predicates.extend(effects::default_postconditions(&self.steps[0].action, "", fortress));
        Predicate::All(predicates).normalized()
    }

    fn authorize(&self, evidence: &PredicateEvidence<'_>, context: &OperationContext) -> Result<()> {
        if context.anchor != evidence.snapshot().anchor() {
            return Err(DfmcpError::new(ErrorCode::StaleAnchor, "protection requires one exact current anchor"));
        }
        if self.civilians.len() + 2 > context.budget.max_entities as usize
            || self.encoded_bytes as u64 > context.budget.max_bytes
        {
            return Err(exhausted());
        }
        // Pause and the standing order are fortress-wide observations. A narrow
        // entity grant cannot be widened to observe those global settings.
        context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)
    }

    /// Seal with independently issued predicate evidence. Source labels, caller
    /// assertions and canonical hashes alone do not establish the input guards.
    /// Effect grants, checkpoint policy, idempotency and dispatch stay with the
    /// ordinary planner/adapter. No native bridge or alternate authority exists.
    pub fn prepare(
        &self, id: IntentId, summary: String, evidence: &PredicateEvidence<'_>,
        context: &OperationContext,
    ) -> Result<PreparedPlan> {
        self.authorize(evidence, context)?;
        if context.budget.max_actions < self.steps.len() as u32 { return Err(exhausted()); }
        let goal = self.goal(evidence.snapshot().fortress_id);
        let mut steps = self.steps.clone();
        steps[1].postconditions = vec![goal.clone()];
        let risk = steps.iter().map(|step| step.action.risk()).max().ok_or_else(exhausted)?;
        let intent = Intent {
            id, anchor: evidence.snapshot().anchor(), summary, terminal_condition: goal,
            constraints: vec![Constraint::MaxRisk(risk), Constraint::KeepPaused],
            requested_actions: steps,
        };
        StaticPlanner::default().prepare_with_evidence(evidence, &intent, context)
    }

    /// Read current truth without changing the world, remembering a clear
    /// streak, confirming an FSM level, or granting release authority.
    pub fn evaluate(
        &self, evidence: &PredicateEvidence<'_>, context: &OperationContext,
    ) -> Result<ProtectionProgress> {
        self.authorize(evidence, context)?;
        let mut progress = ProtectionProgress {
            anchor: context.anchor, requested_civilians: self.civilians.len(), assigned: 0,
            unassigned: 0, unknown_memberships: 0,
            configuration: evidence.evaluate(&self.goal(context.anchor.fortress_id))?,
        };
        for id in &self.civilians {
            match evidence.evaluate(&membership(*id, self.burrow))? {
                PredicateTruth::True => progress.assigned += 1,
                PredicateTruth::False => progress.unassigned += 1,
                PredicateTruth::Unknown => progress.unknown_memberships += 1,
            }
        }
        Ok(progress)
    }
}
