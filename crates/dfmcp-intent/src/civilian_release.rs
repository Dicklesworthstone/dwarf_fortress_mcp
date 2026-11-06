//! Reviewed release of one complete reference civilian roster.
//!
//! A trusted observing shell must first own a complete stable-clear window and
//! admit this exact source anchor. This pure lowering accepts no asserted clear
//! history and cannot grant release authority. Its current-safety predicate is
//! rechecked by the ordinary adapter before each effect, including deferred work.
//! Cancellation reasserts protection; it never invents a second release path.

use dfmcp_core::{
    Capability, DfmcpError, ErrorCode, FortressId, IntentId, OperationContext, Result, RiskTier,
};
use dfmcp_world::{
    CompareOp, LaboratoryEntityIdentity, Predicate, PredicateEvidence, Value, WorldSnapshot,
};

use crate::{
    Action, Constraint, Intent, PlanStep, PreparedPlan, RequestedAction, StaticPlanner,
    civilian_protection::CIVILIAN_RESTRICTION, effects,
};

pub const MAX_RELEASE_BYTES: usize = 256 * 1024;

fn exhausted() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::BudgetExceeded,
        "civilian release exceeds its bounded allowance",
    )
}

/// Preserve the incarnations named by a release plan when its protective
/// compensation becomes a new authorized action. This intentionally does not
/// require current threat clearance: returned danger is a reason to restore
/// protection, not a reason to refuse it. Legacy plans without the additive
/// release predicate retain their existing cancellation semantics.
pub fn validate_release_compensation_identities(
    step: &PlanStep,
    snapshot: &WorldSnapshot,
) -> Result<()> {
    for root in &step.preconditions {
        root.validate_shape()?;
        let mut pending = vec![root];
        while let Some(predicate) = pending.pop() {
            match predicate {
                Predicate::All(children) | Predicate::Any(children) => pending.extend(children),
                Predicate::Not(child) => pending.push(child),
                Predicate::LaboratoryCivilianReleaseSafety { burrow, civilians } => {
                    for (identity, kind) in std::iter::once((burrow, "burrow"))
                        .chain(civilians.iter().map(|civilian| (civilian, "unit")))
                    {
                        if !snapshot
                            .graph
                            .entities
                            .get(&identity.id)
                            .is_some_and(|record| {
                                record.id == identity.id
                                    && record.generation == identity.generation
                                    && record.kind.as_str() == kind
                            })
                        {
                            return Err(DfmcpError::new(
                                ErrorCode::CancellationIncomplete,
                                "civilian release compensation would target a missing or replaced incarnation; stop without compensation or review fresh protection",
                            ));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

/// Immutable lowering for a shell-admitted stable-clear decision. Entity IDs
/// alone are insufficient: every generation and the complete roster are sealed.
/// Running reference worlds are permitted; every dispatch rechecks current
/// threats and identities rather than assuming a pause owns the game clock.
#[derive(Clone, Debug)]
pub struct CivilianReleasePlan {
    burrow: LaboratoryEntityIdentity,
    civilians: Vec<LaboratoryEntityIdentity>,
    steps: Vec<RequestedAction>,
    safety: Predicate,
    membership_goal: Predicate,
    encoded_bytes: usize,
}

impl CivilianReleasePlan {
    pub fn compile(
        burrow: LaboratoryEntityIdentity,
        civilians: &[LaboratoryEntityIdentity],
    ) -> Result<Self> {
        if civilians.is_empty()
            || civilians.len() > dfmcp_world::laboratory_civilian::MAX_LABORATORY_CIVILIANS
        {
            return Err(exhausted());
        }
        let mut civilians = civilians.to_vec();
        let safety = Predicate::LaboratoryCivilianReleaseSafety {
            burrow,
            civilians: civilians.clone(),
        };
        // Validate before normalization: duplicate IDs must never disappear.
        safety.validate_shape()?;
        civilians.sort_unstable();
        let safety = safety.normalized();
        let mut memberships = vec![safety.clone()];
        memberships.extend(civilians.iter().map(|civilian| Predicate::FieldCompare {
            entity_id: civilian.id,
            field: format!("{}{}", effects::BURROW_FIELD_PREFIX, burrow.id.get()),
            op: CompareOp::Eq,
            value: Value::Bool(false),
        }));
        let membership_goal = Predicate::All(memberships).normalized();
        let units = civilians
            .iter()
            .map(|civilian| civilian.id)
            .collect::<Vec<_>>();
        let steps = vec![
            RequestedAction {
                action: Action::SetBurrowMembership {
                    units: units.clone(),
                    burrow: burrow.id,
                    assigned: false,
                },
                preconditions: vec![safety.clone()],
                postconditions: vec![membership_goal.clone()],
                compensation: Some(Action::SetBurrowMembership {
                    units,
                    burrow: burrow.id,
                    assigned: true,
                }),
                obligation: None,
                depends_on: Vec::new(),
            },
            RequestedAction {
                action: Action::SetStandingOrder {
                    key: CIVILIAN_RESTRICTION.to_owned(),
                    value: "INACTIVE".to_owned(),
                },
                preconditions: vec![safety.clone()],
                // The exact fortress-wide whole goal is bound by
                // requested_actions_for or prepare below.
                postconditions: Vec::new(),
                compensation: Some(Action::SetStandingOrder {
                    key: CIVILIAN_RESTRICTION.to_owned(),
                    value: "ACTIVE".to_owned(),
                }),
                obligation: None,
                depends_on: vec![0],
            },
        ];
        membership_goal.validate_shape()?;
        let encoded_bytes = (4096
            + civilians.len() * 64
            + safety.canonical_bytes().len() * 3
            + membership_goal.canonical_bytes().len() * 2)
            .checked_mul(4)
            .ok_or_else(exhausted)?;
        if encoded_bytes > MAX_RELEASE_BYTES {
            return Err(exhausted());
        }
        Ok(Self {
            burrow,
            civilians,
            steps,
            safety,
            membership_goal,
            encoded_bytes,
        })
    }

    #[must_use]
    pub const fn burrow(&self) -> LaboratoryEntityIdentity {
        self.burrow
    }

    #[must_use]
    pub fn civilians(&self) -> &[LaboratoryEntityIdentity] {
        &self.civilians
    }

    #[must_use]
    pub fn safety_predicate(&self) -> &Predicate {
        &self.safety
    }

    #[must_use]
    pub const fn encoded_bytes(&self) -> usize {
        self.encoded_bytes
    }

    /// Pure unbound lowering. Use `requested_actions_for` when constructing a
    /// complete semantic intent so the final step proves the whole configuration.
    #[must_use]
    pub fn requested_actions(&self) -> &[RequestedAction] {
        &self.steps
    }

    #[must_use]
    pub fn into_requested_actions(self) -> Vec<RequestedAction> {
        self.steps
    }

    /// Current full configuration, including all exact identities, complete
    /// current threat clearance, removed memberships and the inactive order.
    #[must_use]
    pub fn configuration_goal(&self, fortress: FortressId) -> Predicate {
        let mut predicates = vec![self.membership_goal.clone()];
        predicates.extend(effects::default_postconditions(
            &self.steps[1].action,
            "",
            fortress,
        ));
        Predicate::All(predicates).normalized()
    }

    #[must_use]
    pub fn requested_actions_for(&self, fortress: FortressId) -> Vec<RequestedAction> {
        let mut steps = self.steps.clone();
        steps[1].postconditions = vec![self.configuration_goal(fortress)];
        steps
    }

    /// Seal ordinary guarded actions after the trusted shell's reviewed clear
    /// window admission. This method independently enforces current qualified
    /// reference safety and normal capability/budget/checkpoint requirements.
    /// It does not create or restore the shell's historical clearance authority.
    pub fn prepare(
        &self,
        id: IntentId,
        summary: String,
        evidence: &PredicateEvidence<'_>,
        context: &OperationContext,
    ) -> Result<PreparedPlan> {
        if context.anchor != evidence.snapshot().anchor() {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "civilian release requires one exact current anchor",
            ));
        }
        if evidence.snapshot().graph.entities.len() > context.budget.max_entities as usize
            || self.encoded_bytes as u64 > context.budget.max_bytes
            || context.budget.max_actions < self.steps.len() as u32
        {
            return Err(exhausted());
        }
        // The safety condition observes the complete threat/civilian domain and
        // a fortress-wide order. A narrow entity grant cannot widen that read.
        context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        let steps = self.requested_actions_for(evidence.snapshot().fortress_id);
        let risk = steps
            .iter()
            .map(|step| step.action.risk())
            .max()
            .ok_or_else(exhausted)?;
        let intent = Intent {
            id,
            anchor: context.anchor,
            summary,
            terminal_condition: self.configuration_goal(evidence.snapshot().fortress_id),
            constraints: vec![Constraint::MaxRisk(risk)],
            requested_actions: steps,
        };
        StaticPlanner::default().prepare_with_evidence(evidence, &intent, context)
    }
}
