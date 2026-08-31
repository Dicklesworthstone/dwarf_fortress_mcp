use std::collections::BTreeSet;

use dfmcp_core::{
    Capability, DfmcpError, Digest32, EntityId, ErrorCode, GameTick, MapCuboid, OperationContext,
    Result, RiskTier, StepId,
};
use dfmcp_world::{Predicate, WorldSnapshot, evaluate};

use crate::{Action, Constraint, Intent, ObligationSpec, PlanStep, PreparedPlan, RequestedAction};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlanPolicy {
    pub max_steps: u32,
    pub max_entities_per_action: u32,
    pub max_area_tiles: u64,
    pub plan_ttl_ticks: u64,
    pub require_checkpoint_at_or_above: RiskTier,
    pub max_string_bytes: usize,
}

impl Default for PlanPolicy {
    fn default() -> Self {
        Self {
            max_steps: 64,
            max_entities_per_action: 512,
            max_area_tiles: 32_768,
            plan_ttl_ticks: 1_200,
            require_checkpoint_at_or_above: RiskTier::Guarded,
            max_string_bytes: 256,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StaticPlanner {
    pub policy: PlanPolicy,
}

impl StaticPlanner {
    #[must_use]
    pub const fn new(policy: PlanPolicy) -> Self {
        Self { policy }
    }

    pub fn prepare(
        &self,
        snapshot: &WorldSnapshot,
        intent: &Intent,
        context: &OperationContext,
    ) -> Result<PreparedPlan> {
        if !snapshot.hash_is_valid() {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "cannot plan from a snapshot with an invalid state hash",
            ));
        }
        if context.anchor != snapshot.anchor() || intent.anchor != snapshot.anchor() {
            return Err(
                DfmcpError::new(ErrorCode::StaleAnchor, "planning anchor is stale").retryable(true),
            );
        }
        context.authorize(Capability::Plan, RiskTier::ReadOnly, &[], None)?;
        if intent.id == dfmcp_core::IntentId::NIL {
            return Err(DfmcpError::new(
                ErrorCode::InvalidIntent,
                "intent identifier zero is reserved",
            ));
        }
        if intent.summary.is_empty() || intent.summary.len() > self.policy.max_string_bytes {
            return Err(DfmcpError::new(
                ErrorCode::InvalidIntent,
                "intent summary is empty or exceeds the configured byte limit",
            ));
        }
        if intent.requested_actions.is_empty() {
            return Err(DfmcpError::new(
                ErrorCode::InvalidIntent,
                "static planner requires at least one semantic action",
            ));
        }
        if evaluate(snapshot, &intent.terminal_condition) {
            return Err(DfmcpError::new(
                ErrorCode::InvalidIntent,
                "intent terminal condition is already satisfied",
            ));
        }

        let step_limit = intent
            .max_steps()
            .map_or(self.policy.max_steps, |limit| limit)
            .min(self.policy.max_steps)
            .min(context.budget.max_actions);
        if intent.requested_actions.len() > step_limit as usize {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                format!("intent exceeds the effective plan step limit {step_limit}"),
            ));
        }
        if let Some(deadline) = intent.deadline()
            && deadline < snapshot.tick
        {
            return Err(DfmcpError::new(
                ErrorCode::InvalidIntent,
                "intent deadline is already in the past",
            ));
        }

        let max_allowed_risk = intent.max_risk();
        let mut steps = Vec::with_capacity(intent.requested_actions.len());
        let mut required_capabilities = BTreeSet::new();
        for (index, requested) in intent.requested_actions.iter().enumerate() {
            let normalized = normalize_requested(requested);
            validate_action(&normalized.action, &self.policy, context)?;
            validate_constraints(intent, &normalized.action)?;
            if normalized.action.risk() > max_allowed_risk {
                return Err(DfmcpError::new(
                    ErrorCode::RiskCeilingExceeded,
                    format!(
                        "action {} has risk {} above intent ceiling {}",
                        index,
                        normalized.action.risk().as_str(),
                        max_allowed_risk.as_str()
                    ),
                ));
            }
            for precondition in &normalized.preconditions {
                if !evaluate(snapshot, precondition) {
                    return Err(DfmcpError::new(
                        ErrorCode::PreconditionsFailed,
                        format!("precondition for requested action {index} is false"),
                    ));
                }
            }
            validate_requested(index, &normalized, snapshot.tick)?;
            let step_id = StepId::new(index as u32);
            let dependencies = validate_dependencies(index, &normalized.depends_on)?;
            let capability = normalized.action.capability();
            required_capabilities.insert(capability);
            let idempotency_key = step_key(intent, step_id, &normalized.action);
            steps.push(PlanStep {
                id: step_id,
                action: normalized.action.clone(),
                preconditions: normalized.preconditions,
                postconditions: normalized.postconditions,
                compensation: normalized.compensation,
                obligation: normalized.obligation,
                depends_on: dependencies,
                risk: normalized.action.risk(),
                required_capability: capability,
                idempotency_key,
            });
        }

        let max_risk = steps
            .iter()
            .map(|step| step.risk)
            .max()
            .map_or(RiskTier::ReadOnly, |risk| risk);
        let requires_checkpoint =
            intent.requires_checkpoint() || max_risk >= self.policy.require_checkpoint_at_or_above;
        if requires_checkpoint {
            required_capabilities.insert(Capability::Checkpoint);
        }
        let intent_expiry = intent.deadline().map_or(GameTick(u64::MAX), |tick| tick);
        let policy_expiry = GameTick(
            snapshot
                .tick
                .0
                .checked_add(self.policy.plan_ttl_ticks)
                .ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::BudgetExceeded,
                        "plan expiry tick exceeds the representable game-time horizon",
                    )
                })?,
        );
        let expires_at_tick = intent_expiry.min(policy_expiry);
        let plan = PreparedPlan::builder(
            intent.id,
            intent.anchor,
            intent.summary.clone(),
            intent.terminal_condition.clone(),
        )
        .steps(steps)
        .max_risk(max_risk)
        .required_capabilities(required_capabilities)
        .requires_checkpoint(requires_checkpoint)
        .expires_at_tick(expires_at_tick)
        .build();
        validate_plan(&plan, &self.policy)?;
        Ok(plan)
    }
}

fn normalize_requested(requested: &RequestedAction) -> RequestedAction {
    let action = requested.action.normalized();
    let compensation = requested
        .compensation
        .as_ref()
        .map(Action::normalized)
        .or(match &action {
            Action::Pause { paused } => Some(Action::Pause { paused: !*paused }),
            _ => None,
        });
    let mut postconditions = requested.postconditions.clone();
    if postconditions.is_empty() {
        match &action {
            Action::Pause { paused } => postconditions.push(Predicate::Paused(*paused)),
            _ => postconditions.push(Predicate::True),
        }
    }
    RequestedAction {
        action,
        preconditions: normalize_predicates(&requested.preconditions),
        postconditions: normalize_predicates(&postconditions),
        compensation,
        obligation: requested.obligation.as_ref().map(normalize_obligation),
        depends_on: requested.depends_on.clone(),
    }
}

fn normalize_predicates(predicates: &[Predicate]) -> Vec<Predicate> {
    let mut output: Vec<_> = predicates.iter().map(Predicate::normalized).collect();
    output.sort_by_key(Predicate::canonical_bytes);
    output.dedup();
    output
}

fn normalize_obligation(obligation: &ObligationSpec) -> ObligationSpec {
    ObligationSpec {
        terminal: obligation.terminal.normalized(),
        failure: obligation.failure.as_ref().map(Predicate::normalized),
        deadline_tick: obligation.deadline_tick,
        poll_interval_ticks: obligation.poll_interval_ticks,
        stable_for_observations: obligation.stable_for_observations,
    }
}

fn validate_action(action: &Action, policy: &PlanPolicy, context: &OperationContext) -> Result<()> {
    let scope = action.scope();
    if scope.entity_ids.len() > policy.max_entities_per_action as usize
        || scope.entity_ids.len() > context.budget.max_entities as usize
    {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "action entity scope exceeds an explicit limit",
        ));
    }
    if scope.entity_ids.contains(&EntityId::NIL) {
        return Err(DfmcpError::new(
            ErrorCode::InvalidIntent,
            "action scope contains reserved entity identifier zero",
        ));
    }
    if let Some(area) = scope.map_area {
        let tile_count = area.tile_count().ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InvalidIntent,
                "action map area tile count overflowed",
            )
        })?;
        if tile_count > policy.max_area_tiles {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                format!(
                    "action map area contains {tile_count} tiles, above limit {}",
                    policy.max_area_tiles
                ),
            ));
        }
    }
    match action {
        Action::Pause { .. } => {}
        Action::DesignateDig { .. } => {}
        Action::Build {
            location,
            footprint,
            material,
            ..
        } => {
            if !footprint.contains(*location) {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidIntent,
                    "building location is outside its footprint",
                ));
            }
            validate_tokens(
                material
                    .required_tokens
                    .iter()
                    .chain(material.forbidden_tokens.iter()),
                policy.max_string_bytes,
            )?;
        }
        Action::SetLabor { units, labor, .. } => {
            if units.is_empty() {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidIntent,
                    "labor action requires at least one unit",
                ));
            }
            validate_string(labor, policy.max_string_bytes, "labor token")?;
        }
        Action::CreateWorkOrder {
            name,
            job_token,
            amount,
            ..
        } => {
            if *amount == 0 {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidIntent,
                    "work-order amount must be nonzero",
                ));
            }
            validate_string(name, policy.max_string_bytes, "work-order name")?;
            validate_string(job_token, policy.max_string_bytes, "job token")?;
        }
        Action::ConfigureStockpile { accepts, .. } => {
            validate_tokens(accepts.iter(), policy.max_string_bytes)?;
        }
        Action::AssignSquad { units, .. } | Action::SetBurrowMembership { units, .. } => {
            if units.is_empty() {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidIntent,
                    "assignment action requires at least one unit",
                ));
            }
        }
        Action::SetStandingOrder { key, value } => {
            validate_string(key, policy.max_string_bytes, "standing-order key")?;
            validate_string(value, policy.max_string_bytes, "standing-order value")?;
        }
        Action::Extension {
            namespace,
            name,
            parameters,
        } => {
            validate_string(namespace, policy.max_string_bytes, "extension namespace")?;
            validate_string(name, policy.max_string_bytes, "extension action name")?;
            if parameters.len() > 64 {
                return Err(DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    "extension parameter count exceeds 64",
                ));
            }
            for (key, value) in parameters {
                validate_string(key, policy.max_string_bytes, "extension parameter key")?;
                validate_string(value, 4_096, "extension parameter value")?;
            }
        }
    }
    Ok(())
}

fn validate_tokens<'a>(values: impl Iterator<Item = &'a String>, max_bytes: usize) -> Result<()> {
    for value in values {
        validate_string(value, max_bytes, "token")?;
    }
    Ok(())
}

fn validate_string(value: &str, max_bytes: usize, label: &str) -> Result<()> {
    if value.is_empty() || value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(DfmcpError::new(
            ErrorCode::InvalidIntent,
            format!("{label} is empty, too long, or contains control characters"),
        ));
    }
    Ok(())
}

fn validate_constraints(intent: &Intent, action: &Action) -> Result<()> {
    let scope = action.scope();
    for constraint in &intent.constraints {
        match constraint {
            Constraint::KeepPaused => {
                if matches!(action, Action::Pause { paused: false }) {
                    return Err(DfmcpError::new(
                        ErrorCode::InvalidIntent,
                        "action would violate keep-paused constraint",
                    ));
                }
            }
            Constraint::ProtectEntities(protected) => {
                if scope.entity_ids.iter().any(|id| protected.contains(id)) {
                    return Err(DfmcpError::new(
                        ErrorCode::InvalidIntent,
                        "action scope intersects protected entities",
                    ));
                }
            }
            Constraint::ExcludeArea(excluded) => {
                if scope
                    .map_area
                    .is_some_and(|requested| cuboids_intersect(requested, *excluded))
                {
                    return Err(DfmcpError::new(
                        ErrorCode::InvalidIntent,
                        "action scope intersects an excluded map area",
                    ));
                }
            }
            Constraint::MaxRisk(_)
            | Constraint::Deadline(_)
            | Constraint::RequireCheckpoint
            | Constraint::MaxSteps(_) => {}
        }
    }
    Ok(())
}

fn cuboids_intersect(left: MapCuboid, right: MapCuboid) -> bool {
    left.min.x <= right.max.x
        && left.max.x >= right.min.x
        && left.min.y <= right.max.y
        && left.max.y >= right.min.y
        && left.min.z <= right.max.z
        && left.max.z >= right.min.z
}

fn validate_requested(
    index: usize,
    requested: &RequestedAction,
    current_tick: GameTick,
) -> Result<()> {
    if requested.postconditions.is_empty() {
        return Err(DfmcpError::new(
            ErrorCode::InvalidIntent,
            format!("requested action {index} has no semantic postcondition"),
        ));
    }
    if requested.action.naturally_temporal() && requested.obligation.is_none() {
        return Err(DfmcpError::new(
            ErrorCode::InvalidIntent,
            format!("requested action {index} requires a bounded obligation"),
        ));
    }
    if let Some(obligation) = &requested.obligation {
        validate_obligation(index, obligation, current_tick)?;
    }
    if let Some(compensation) = &requested.compensation
        && compensation.risk() == RiskTier::Irreversible
    {
        return Err(DfmcpError::new(
            ErrorCode::InvalidIntent,
            format!("compensation for action {index} is irreversible"),
        ));
    }
    Ok(())
}

fn validate_obligation(
    index: usize,
    obligation: &ObligationSpec,
    current_tick: GameTick,
) -> Result<()> {
    if obligation.deadline_tick <= current_tick {
        return Err(DfmcpError::new(
            ErrorCode::InvalidIntent,
            format!("obligation for action {index} has no future deadline"),
        ));
    }
    if obligation.poll_interval_ticks == 0 || obligation.stable_for_observations == 0 {
        return Err(DfmcpError::new(
            ErrorCode::InvalidIntent,
            format!("obligation for action {index} has a zero liveness bound"),
        ));
    }
    Ok(())
}

fn validate_dependencies(index: usize, dependencies: &[u32]) -> Result<Vec<StepId>> {
    let mut seen = BTreeSet::new();
    let mut output = Vec::with_capacity(dependencies.len());
    for dependency in dependencies {
        if *dependency as usize >= index {
            return Err(DfmcpError::new(
                ErrorCode::InvalidIntent,
                format!("step {index} depends on itself or a later step"),
            ));
        }
        if !seen.insert(*dependency) {
            return Err(DfmcpError::new(
                ErrorCode::InvalidIntent,
                format!("step {index} repeats dependency {dependency}"),
            ));
        }
        output.push(StepId::new(*dependency));
    }
    output.sort_unstable();
    Ok(output)
}

fn step_key(intent: &Intent, step_id: StepId, action: &Action) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"dfmcp-step-idempotency-v1");
    bytes.extend_from_slice(&intent.id.get().to_be_bytes());
    bytes.extend_from_slice(&step_id.get().to_be_bytes());
    bytes.extend_from_slice(intent.anchor.state_hash.as_bytes());
    action.encode(&mut bytes);
    Digest32::of_bytes(&bytes).to_hex()
}

fn validate_plan(plan: &PreparedPlan, policy: &PlanPolicy) -> Result<()> {
    plan.validate_structure().map_err(|error| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            format!("planner emitted an invalid plan: {}", error.message),
        )
    })?;
    if plan.steps.is_empty() || plan.steps.len() > policy.max_steps as usize {
        return Err(DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "prepared plan step count violates policy",
        ));
    }
    let mut capabilities = BTreeSet::new();
    for (index, step) in plan.steps.iter().enumerate() {
        if step.id.get() as usize != index {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "prepared plan step identifiers are not contiguous",
            ));
        }
        if step.risk != step.action.risk() || step.required_capability != step.action.capability() {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "prepared plan action metadata disagrees with the action",
            ));
        }
        if step
            .depends_on
            .iter()
            .any(|dependency| dependency.get() >= step.id.get())
        {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "prepared plan dependency graph is not topologically ordered",
            ));
        }
        capabilities.insert(step.required_capability);
    }
    if plan.requires_checkpoint {
        capabilities.insert(Capability::Checkpoint);
    }
    if capabilities != plan.required_capabilities {
        return Err(DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "prepared plan capability summary is inconsistent",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use dfmcp_core::{
        Capability, CapabilityGrant, CapabilityScope, DfmcpError, FortressId, GameTick, IntentId,
        ObservationCursor, OperationContext, RequestId, RiskTier, SessionId, WorkBudget,
    };
    use dfmcp_world::{Predicate, WorldGraph, WorldSnapshot};

    use super::StaticPlanner;
    use crate::{Action, Constraint, Intent, RequestedAction};

    fn snapshot() -> WorldSnapshot {
        WorldSnapshot::new(
            FortressId::new(1),
            GameTick(10),
            ObservationCursor::ORIGIN,
            true,
            WorldGraph::default(),
        )
    }

    fn context(snapshot: &WorldSnapshot) -> OperationContext {
        OperationContext {
            session_id: SessionId::new(1),
            request_id: RequestId::new(1),
            anchor: snapshot.anchor(),
            budget: WorkBudget::default(),
            grants: vec![CapabilityGrant {
                capability: Capability::Plan,
                scope: CapabilityScope {
                    fortress_id: Some(snapshot.fortress_id),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: None,
                remaining_uses: None,
            }],
            cancellation_requested: false,
        }
    }

    #[test]
    fn pause_plan_is_sealed_and_compensable() -> Result<(), DfmcpError> {
        let snapshot = snapshot();
        let intent = Intent {
            id: IntentId::new(1),
            anchor: snapshot.anchor(),
            summary: "unpause the simulation".to_owned(),
            terminal_condition: Predicate::Paused(false),
            constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
            requested_actions: vec![RequestedAction {
                action: Action::Pause { paused: false },
                preconditions: vec![Predicate::Paused(true)],
                postconditions: Vec::new(),
                compensation: None,
                obligation: None,
                depends_on: Vec::new(),
            }],
        };
        let plan = StaticPlanner::default().prepare(&snapshot, &intent, &context(&snapshot))?;
        assert!(plan.digest_is_valid());
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(
            plan.steps[0].compensation,
            Some(Action::Pause { paused: true })
        );
        Ok(())
    }
}
