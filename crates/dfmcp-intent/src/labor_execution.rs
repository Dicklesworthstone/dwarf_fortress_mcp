//! Lower a declared staffing model to guarded, ordered semantic work.
//!
//! Skill and stress inputs remain advisory model inputs. They do not become
//! observations or grant authority. The proposed initial labor matrix, citizen
//! kinds and liveness must be independently established by the ordinary planner.
//! Completion means the requested labor configuration, not job execution.

use std::collections::BTreeMap;

use dfmcp_core::{
    Capability, DfmcpError, EntityId, ErrorCode, IntentId, OperationContext, Result, RiskTier,
};
use dfmcp_world::{CompareOp, EntityKind, Predicate, PredicateEvidence, Value};

use crate::{
    Action, Constraint, DwarfLaborProfile, Intent, LaborAllocation,
    LaborAllocationLimits, LaborAllocator, LaborDemand, PreparedPlan, RequestedAction,
    StaticPlanner,
};

pub const MAX_EXECUTION_ROSTER: usize = 64;
pub const MAX_EXECUTION_DEMANDS: usize = 16;
pub const MAX_EXECUTION_BYTES: usize = 2 * 1024 * 1024;
const MAX_LABOR_BYTES: usize = 128;

type Matrix = BTreeMap<(EntityId, String), (bool, bool)>;

fn refused(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::PreconditionsFailed, message)
}
fn exhausted() -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, "labor execution compilation exhausted its allowance")
}
fn field(id: EntityId, name: String, value: bool) -> Predicate {
    Predicate::FieldCompare {
        entity_id: id,
        field: name,
        op: CompareOp::Eq,
        value: Value::Bool(value),
    }
}
fn labor(id: EntityId, name: &str, enabled: bool) -> Predicate {
    field(id, format!("{}{name}", crate::effects::LABOR_FIELD_PREFIX), enabled)
}
fn citizen(id: EntityId) -> Predicate {
    Predicate::All(vec![
        Predicate::EntityKind { entity_id: id, kind: EntityKind::Unit },
        field(id, "alive".to_owned(), true),
    ])
}

/// An immutable proposal with a complete target over the explicitly named
/// roster. It is not a native workforce transaction or a compatibility claim.
#[derive(Clone, Debug)]
pub struct LaborExecutionPlan {
    allocation: LaborAllocation,
    roster: Vec<EntityId>,
    steps: Vec<RequestedAction>,
    goal: Predicate,
    encoded_bytes: usize,
    work_used: u64,
}
impl LaborExecutionPlan {
    #[must_use]
    pub fn allocation(&self) -> &LaborAllocation { &self.allocation }
    #[must_use]
    pub fn requested_actions(&self) -> &[RequestedAction] { &self.steps }
    #[must_use]
    pub fn goal(&self) -> &Predicate { &self.goal }
    #[must_use]
    pub const fn encoded_bytes(&self) -> usize { self.encoded_bytes }
    #[must_use]
    pub const fn work_used(&self) -> u64 { self.work_used }

    /// An already modeled target is not a reason to dispatch a no-op or invent
    /// an inverse compensation. Inspect current truth or submit a fresh model.
    pub fn into_requested_actions(self) -> Result<Vec<RequestedAction>> {
        if self.steps.is_empty() {
            return Err(refused("labor model requests no changes; inspect the current configuration"));
        }
        Ok(self.steps)
    }

    /// Seal through the existing source-qualified planner. This checks current
    /// Observe authority for the whole named roster, while the planner retains
    /// its normal Plan, anchor, risk, shape, deadline and source checks. Mutation
    /// authority is still required at dispatch, not supplied by this proposal.
    pub fn prepare(
        &self,
        id: IntentId,
        summary: String,
        evidence: &PredicateEvidence<'_>,
        context: &OperationContext,
    ) -> Result<PreparedPlan> {
        if self.steps.is_empty() {
            return Err(refused("labor model requests no changes; inspect the current configuration"));
        }
        if self.encoded_bytes as u64 > context.budget.max_bytes
            || self.roster.len() > context.budget.max_entities as usize
            || self.steps.len() > context.budget.max_actions as usize
        {
            return Err(exhausted());
        }
        context.authorize(Capability::Observe, RiskTier::ReadOnly, &self.roster, None)?;
        let intent = Intent {
            id,
            anchor: evidence.snapshot().anchor(),
            summary,
            terminal_condition: self.goal.clone(),
            constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
            requested_actions: self.steps.clone(),
        };
        StaticPlanner::default().prepare_with_evidence(evidence, &intent, context)
    }
}

impl LaborAllocator {
    /// Refuse shortages before returning executable proposals. Only actual
    /// modeled changes become effects. Every later step depends on the prior
    /// step, so all disables must be proved before any enable can dispatch.
    /// Each write requires its independently observed opposite initial value;
    /// missing fields do not establish disabled labor. The first step guards
    /// the entire initial matrix, including workers that will not be changed.
    pub fn compile_execution(
        &self,
        roster: &[DwarfLaborProfile],
        demands: &[LaborDemand],
        limits: LaborAllocationLimits,
    ) -> Result<LaborExecutionPlan> {
        if roster.len() > MAX_EXECUTION_ROSTER || demands.len() > MAX_EXECUTION_DEMANDS {
            return Err(exhausted());
        }
        if demands.iter().any(|d| d.labor.len() > MAX_LABOR_BYTES || d.labor.chars().any(char::is_control)) {
            return Err(DfmcpError::new(ErrorCode::InvalidRequest, "execution labor tokens require at most 128 non-control bytes"));
        }
        let allocation = self.allocate_demands(roster, demands, limits)?;
        if !allocation.is_fully_staffed() {
            return Err(refused("staffing targets are infeasible; inspect allocate_demands shortages before requesting effects"));
        }
        let mut work = allocation.work_used();
        let mut charge = |amount: usize| -> Result<()> {
            work = work.checked_add(amount as u64).ok_or_else(exhausted)?;
            if work > limits.max_work { return Err(exhausted()); }
            Ok(())
        };
        let mut ids: Vec<_> = roster.iter().map(|row| row.id).collect();
        ids.sort_unstable();
        let mut matrix = Matrix::new();
        for row in roster {
            for role in allocation.staffing() {
                charge(1)?;
                matrix.insert((row.id, role.labor.clone()), (
                    row.assigned_labors.contains(&role.labor),
                    role.assigned_units.binary_search(&row.id).is_ok(),
                ));
            }
        }
        // The allocator may also propose relief outside the managed roles.
        // Derive those targets from its actual output instead of maintaining
        // a second heavy-labor policy here.
        let rows: BTreeMap<_, _> = roster.iter().map(|row| (row.id, row)).collect();
        for action in allocation.actions() {
            let Action::SetLabor { units, labor: name, enabled } = action else {
                return Err(DfmcpError::new(ErrorCode::InternalInvariantViolation, "allocator emitted a non-labor action"));
            };
            for id in units {
                charge(1)?;
                let row = rows.get(id).ok_or_else(|| DfmcpError::new(ErrorCode::InternalInvariantViolation, "allocator selected an unknown citizen"))?;
                matrix.entry((*id, name.clone())).or_insert((row.assigned_labors.contains(name), *enabled));
            }
        }
        charge(ids.len() * 6 + matrix.len() * 2)?;
        let mut initial: Vec<_> = ids.iter().copied().map(citizen).collect();
        let mut terminal = initial.clone();
        for ((id, name), (before, after)) in &matrix {
            initial.push(labor(*id, name, *before));
            terminal.push(labor(*id, name, *after));
        }
        let initial = Predicate::All(initial).normalized();
        let goal = Predicate::All(terminal).normalized();
        initial.validate_shape()?;
        goal.validate_shape()?;
        let mut steps = Vec::with_capacity(allocation.actions().len());
        for (index, action) in allocation.actions().iter().enumerate() {
            let Action::SetLabor { units, labor: name, enabled } = action else {
                return Err(DfmcpError::new(ErrorCode::InternalInvariantViolation, "allocator emitted a non-labor action"));
            };
            let mut before = Vec::new();
            let mut after = Vec::new();
            for id in units {
                charge(5)?;
                if matrix.get(&(*id, name.clone())) != Some(&(!*enabled, *enabled)) {
                    return Err(DfmcpError::new(ErrorCode::InternalInvariantViolation, "labor effect differs from its target matrix"));
                }
                before.push(citizen(*id));
                before.push(labor(*id, name, !*enabled));
                after.push(labor(*id, name, *enabled));
            }
            if index == 0 {
                charge(initial.complexity())?;
                before.push(initial.clone());
            }
            // Also retain unchanged assignments in the last action's proof.
            // Generic action-intent consumers must not mistake changed bits
            // alone for achievement of the complete staffing configuration.
            if index + 1 == allocation.actions().len() {
                charge(goal.complexity())?;
                after.push(goal.clone());
            }
            let depends_on = if index == 0 { Vec::new() } else {
                vec![u32::try_from(index - 1).map_err(|_| exhausted())?]
            };
            steps.push(RequestedAction {
                action: action.clone(),
                preconditions: vec![Predicate::All(before).normalized()],
                postconditions: vec![Predicate::All(after).normalized()],
                compensation: Some(Action::SetLabor { units: units.clone(), labor: name.clone(), enabled: !*enabled }),
                obligation: None,
                depends_on,
            });
        }
        let mut encoded_bytes = goal.canonical_bytes().len();
        for step in &steps {
            for predicate in step.preconditions.iter().chain(&step.postconditions) {
                predicate.validate_shape()?;
                charge(predicate.complexity())?;
                encoded_bytes = encoded_bytes.checked_add(predicate.canonical_bytes().len()).ok_or_else(exhausted)?;
                if encoded_bytes > MAX_EXECUTION_BYTES { return Err(exhausted()); }
            }
            let Action::SetLabor { units, labor, .. } = &step.action else { return Err(exhausted()); };
            // Include action, compensation, dependency and fixed record costs.
            encoded_bytes = encoded_bytes.checked_add(256 + 2 * (units.len() * 8 + labor.len())).ok_or_else(exhausted)?;
        }
        if encoded_bytes > MAX_EXECUTION_BYTES { return Err(exhausted()); }
        Ok(LaborExecutionPlan { allocation, roster: ids, steps, goal, encoded_bytes, work_used: work })
    }
}
