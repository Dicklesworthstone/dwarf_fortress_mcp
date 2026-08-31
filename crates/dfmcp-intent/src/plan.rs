use std::collections::BTreeSet;

use dfmcp_core::{
    Capability, DfmcpError, Digest32, EntityId, ErrorCode, GameTick, IntentId, MapCuboid, PlanId,
    Result, RiskTier, StateAnchor, StepId,
};
use dfmcp_world::Predicate;

use crate::Action;

const MAX_PLAN_STEPS: usize = 4_096;
const MAX_PLAN_SUMMARY_BYTES: usize = 4_096;
const MAX_PLAN_PREDICATES_PER_STEP: usize = 64;
const MAX_PLAN_DEPENDENCIES_PER_STEP: usize = 4_096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Constraint {
    MaxRisk(RiskTier),
    KeepPaused,
    ProtectEntities(BTreeSet<EntityId>),
    ExcludeArea(MapCuboid),
    Deadline(GameTick),
    RequireCheckpoint,
    MaxSteps(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObligationSpec {
    pub terminal: Predicate,
    pub failure: Option<Predicate>,
    pub deadline_tick: GameTick,
    pub poll_interval_ticks: u64,
    pub stable_for_observations: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestedAction {
    pub action: Action,
    pub preconditions: Vec<Predicate>,
    pub postconditions: Vec<Predicate>,
    pub compensation: Option<Action>,
    pub obligation: Option<ObligationSpec>,
    pub depends_on: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Intent {
    pub id: IntentId,
    pub anchor: StateAnchor,
    pub summary: String,
    pub terminal_condition: Predicate,
    pub constraints: Vec<Constraint>,
    pub requested_actions: Vec<RequestedAction>,
}

impl Intent {
    #[must_use]
    pub fn max_risk(&self) -> RiskTier {
        self.constraints
            .iter()
            .filter_map(|constraint| match constraint {
                Constraint::MaxRisk(value) => Some(*value),
                _ => None,
            })
            .min()
            .map_or(RiskTier::Guarded, |risk| risk)
    }

    #[must_use]
    pub fn deadline(&self) -> Option<GameTick> {
        self.constraints
            .iter()
            .filter_map(|constraint| match constraint {
                Constraint::Deadline(value) => Some(*value),
                _ => None,
            })
            .min()
    }

    #[must_use]
    pub fn max_steps(&self) -> Option<u32> {
        self.constraints
            .iter()
            .filter_map(|constraint| match constraint {
                Constraint::MaxSteps(value) => Some(*value),
                _ => None,
            })
            .min()
    }

    #[must_use]
    pub fn requires_checkpoint(&self) -> bool {
        self.constraints
            .iter()
            .any(|constraint| matches!(constraint, Constraint::RequireCheckpoint))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanStep {
    pub id: StepId,
    pub action: Action,
    pub preconditions: Vec<Predicate>,
    pub postconditions: Vec<Predicate>,
    pub compensation: Option<Action>,
    pub obligation: Option<ObligationSpec>,
    pub depends_on: Vec<StepId>,
    pub risk: RiskTier,
    pub required_capability: Capability,
    pub idempotency_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedPlan {
    pub id: PlanId,
    pub intent_id: IntentId,
    pub anchor: StateAnchor,
    pub summary: String,
    pub terminal_condition: Predicate,
    pub steps: Vec<PlanStep>,
    pub max_risk: RiskTier,
    pub required_capabilities: BTreeSet<Capability>,
    pub requires_checkpoint: bool,
    pub expires_at_tick: GameTick,
    pub digest: Digest32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedPlanBuilder {
    pub intent_id: IntentId,
    pub anchor: StateAnchor,
    pub summary: String,
    pub terminal_condition: Predicate,
    pub steps: Vec<PlanStep>,
    pub max_risk: RiskTier,
    pub required_capabilities: BTreeSet<Capability>,
    pub requires_checkpoint: bool,
    pub expires_at_tick: GameTick,
}

impl PreparedPlanBuilder {
    #[must_use]
    pub fn new(
        intent_id: IntentId,
        anchor: StateAnchor,
        summary: impl Into<String>,
        terminal_condition: Predicate,
    ) -> Self {
        Self {
            intent_id,
            anchor,
            summary: summary.into(),
            terminal_condition: terminal_condition.normalized(),
            steps: Vec::new(),
            max_risk: RiskTier::ReadOnly,
            required_capabilities: BTreeSet::new(),
            requires_checkpoint: false,
            expires_at_tick: GameTick(u64::MAX),
        }
    }

    #[must_use]
    pub fn steps(mut self, steps: Vec<PlanStep>) -> Self {
        self.steps = steps;
        self
    }

    #[must_use]
    pub fn max_risk(mut self, max_risk: RiskTier) -> Self {
        self.max_risk = max_risk;
        self
    }

    #[must_use]
    pub fn required_capabilities(mut self, required_capabilities: BTreeSet<Capability>) -> Self {
        self.required_capabilities = required_capabilities;
        self
    }

    #[must_use]
    pub fn requires_checkpoint(mut self, requires_checkpoint: bool) -> Self {
        self.requires_checkpoint = requires_checkpoint;
        self
    }

    #[must_use]
    pub fn expires_at_tick(mut self, expires_at_tick: GameTick) -> Self {
        self.expires_at_tick = expires_at_tick;
        self
    }

    #[must_use]
    pub fn build(self) -> PreparedPlan {
        let mut plan = PreparedPlan {
            id: PlanId::NIL,
            intent_id: self.intent_id,
            anchor: self.anchor,
            summary: self.summary,
            terminal_condition: self.terminal_condition.normalized(),
            steps: self.steps,
            max_risk: self.max_risk,
            required_capabilities: self.required_capabilities,
            requires_checkpoint: self.requires_checkpoint,
            expires_at_tick: self.expires_at_tick,
            digest: Digest32::ZERO,
        };
        plan.digest = plan.compute_digest();
        let candidate = plan.digest.first_u128();
        plan.id = PlanId::new(if candidate == 0 { 1 } else { candidate });
        plan
    }
}

#[must_use]
pub(crate) fn derive_step_idempotency_key(
    intent_id: IntentId,
    anchor: StateAnchor,
    step_id: StepId,
    action: &Action,
) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"dfmcp-step-idempotency-v1");
    bytes.extend_from_slice(&intent_id.get().to_be_bytes());
    bytes.extend_from_slice(&step_id.get().to_be_bytes());
    bytes.extend_from_slice(anchor.state_hash.as_bytes());
    action.encode(&mut bytes);
    Digest32::of_bytes(&bytes).to_hex()
}

impl PreparedPlan {
    #[must_use]
    pub fn builder(
        intent_id: IntentId,
        anchor: StateAnchor,
        summary: impl Into<String>,
        terminal_condition: Predicate,
    ) -> PreparedPlanBuilder {
        PreparedPlanBuilder::new(intent_id, anchor, summary, terminal_condition)
    }

    #[must_use]
    pub fn compute_digest(&self) -> Digest32 {
        let mut output = Vec::new();
        put_str(&mut output, "dfmcp-prepared-plan-v1");
        output.extend_from_slice(&self.intent_id.get().to_be_bytes());
        put_anchor(&mut output, self.anchor);
        put_str(&mut output, &self.summary);
        put_bytes(&mut output, &self.terminal_condition.canonical_bytes());
        put_u64(&mut output, self.steps.len() as u64);
        for step in &self.steps {
            put_u32(&mut output, step.id.get());
            let mut action = Vec::new();
            step.action.encode(&mut action);
            put_bytes(&mut output, &action);
            put_predicates(&mut output, &step.preconditions);
            put_predicates(&mut output, &step.postconditions);
            match &step.compensation {
                Some(compensation) => {
                    output.push(1);
                    let mut encoded = Vec::new();
                    compensation.encode(&mut encoded);
                    put_bytes(&mut output, &encoded);
                }
                None => output.push(0),
            }
            match &step.obligation {
                Some(obligation) => {
                    output.push(1);
                    put_obligation(&mut output, obligation);
                }
                None => output.push(0),
            }
            put_u64(&mut output, step.depends_on.len() as u64);
            for dependency in &step.depends_on {
                put_u32(&mut output, dependency.get());
            }
            output.push(risk_code(step.risk));
            output.push(capability_code(step.required_capability));
            put_str(&mut output, &step.idempotency_key);
        }
        output.push(risk_code(self.max_risk));
        put_u64(&mut output, self.required_capabilities.len() as u64);
        for capability in &self.required_capabilities {
            output.push(capability_code(*capability));
        }
        output.push(u8::from(self.requires_checkpoint));
        put_u64(&mut output, self.expires_at_tick.0);
        Digest32::of_bytes(&output)
    }

    #[must_use]
    pub fn expected_id(&self) -> PlanId {
        let candidate = self.compute_digest().first_u128();
        PlanId::new(if candidate == 0 { 1 } else { candidate })
    }

    #[must_use]
    pub fn digest_is_valid(&self) -> bool {
        self.digest == self.compute_digest()
    }

    #[must_use]
    pub fn id_is_valid(&self) -> bool {
        self.id != PlanId::NIL && self.id == self.expected_id()
    }

    pub fn validate_structure(&self) -> Result<()> {
        if self.intent_id == IntentId::NIL
            || self.summary.is_empty()
            || self.summary.len() > MAX_PLAN_SUMMARY_BYTES
        {
            return Err(DfmcpError::new(
                ErrorCode::InvalidPlan,
                "plan intent identifier and bounded summary must be nonempty",
            ));
        }
        if self.steps.is_empty()
            || self.steps.len() > MAX_PLAN_STEPS
            || self.expires_at_tick <= self.anchor.tick
        {
            return Err(DfmcpError::new(
                ErrorCode::InvalidPlan,
                "plan step count or expiry violates its structural bound",
            ));
        }
        self.terminal_condition.validate_shape()?;
        if self.terminal_condition != self.terminal_condition.normalized() {
            return Err(DfmcpError::new(
                ErrorCode::InvalidPlan,
                "plan terminal condition is not canonical",
            ));
        }
        for step in &self.steps {
            if step.preconditions.len() > MAX_PLAN_PREDICATES_PER_STEP
                || step.postconditions.len() > MAX_PLAN_PREDICATES_PER_STEP
                || step.depends_on.len() > MAX_PLAN_DEPENDENCIES_PER_STEP
            {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidPlan,
                    "plan step predicate or dependency count exceeds its structural bound",
                ));
            }
            for predicate in step.preconditions.iter().chain(step.postconditions.iter()) {
                predicate.validate_shape()?;
            }
            if let Some(obligation) = &step.obligation {
                obligation.terminal.validate_shape()?;
                if let Some(failure) = &obligation.failure {
                    failure.validate_shape()?;
                }
            }
        }
        let computed_digest = self.compute_digest();
        let candidate = computed_digest.first_u128();
        let expected_id = PlanId::new(if candidate == 0 { 1 } else { candidate });
        if self.digest != computed_digest || self.id == PlanId::NIL || self.id != expected_id {
            return Err(DfmcpError::new(
                ErrorCode::InvalidPlan,
                "plan digest or digest-derived identifier is invalid",
            ));
        }

        let mut capabilities = BTreeSet::new();
        let mut maximum_risk = RiskTier::ReadOnly;
        for (index, step) in self.steps.iter().enumerate() {
            let expected_step = u32::try_from(index).map_err(|_| {
                DfmcpError::new(ErrorCode::InvalidPlan, "plan contains too many steps")
            })?;
            if step.id != StepId::new(expected_step) {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidPlan,
                    "plan step identifiers are not contiguous and zero-based",
                ));
            }
            if step.action != step.action.normalized()
                || step.risk != step.action.risk()
                || step.required_capability != step.action.capability()
            {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidPlan,
                    "plan step action or derived metadata is not canonical",
                ));
            }
            if step.postconditions.is_empty()
                || step
                    .postconditions
                    .iter()
                    .all(|predicate| matches!(predicate, Predicate::True))
                || step
                    .postconditions
                    .iter()
                    .any(|predicate| matches!(predicate, Predicate::False))
                || !predicates_are_canonical(&step.preconditions)
                || !predicates_are_canonical(&step.postconditions)
            {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidPlan,
                    "plan step postconditions are trivial, impossible, empty, or noncanonical",
                ));
            }
            if step.depends_on.windows(2).any(|pair| pair[0] >= pair[1])
                || step
                    .depends_on
                    .iter()
                    .any(|dependency| dependency.get() >= step.id.get())
            {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidPlan,
                    "plan dependency graph is not strictly ordered and acyclic",
                ));
            }
            let expected_key =
                derive_step_idempotency_key(self.intent_id, self.anchor, step.id, &step.action);
            if step.idempotency_key != expected_key {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidPlan,
                    "plan step idempotency key is not the deterministic key for its intent, anchor, step, and action",
                ));
            }
            if step.action.naturally_temporal() && step.obligation.is_none() {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidPlan,
                    "temporal action lacks a bounded obligation",
                ));
            }
            if let Some(obligation) = &step.obligation
                && (obligation.deadline_tick <= self.anchor.tick
                    || obligation.poll_interval_ticks == 0
                    || obligation.stable_for_observations == 0
                    || matches!(obligation.terminal, Predicate::True | Predicate::False)
                    || obligation.terminal != obligation.terminal.normalized()
                    || obligation.failure.as_ref().is_some_and(|failure| {
                        matches!(failure, Predicate::True | Predicate::False)
                            || failure != &failure.normalized()
                    }))
            {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidPlan,
                    "plan obligation is trivial, impossible, unbounded, or noncanonical",
                ));
            }
            if step.compensation.as_ref().is_some_and(|action| {
                action != &action.normalized()
                    || action.risk() == RiskTier::Irreversible
                    || action.risk() > self.max_risk
            }) {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidPlan,
                    "plan compensation is noncanonical, irreversible, or above the plan risk ceiling",
                ));
            }
            capabilities.insert(step.required_capability);
            maximum_risk = maximum_risk.max(step.risk);
        }
        if self.requires_checkpoint {
            capabilities.insert(Capability::Checkpoint);
        }
        if capabilities != self.required_capabilities || maximum_risk != self.max_risk {
            return Err(DfmcpError::new(
                ErrorCode::InvalidPlan,
                "plan capability or risk summary is inconsistent",
            ));
        }
        Ok(())
    }
}

fn predicates_are_canonical(predicates: &[Predicate]) -> bool {
    predicates
        .iter()
        .all(|predicate| predicate == &predicate.normalized())
        && predicates
            .windows(2)
            .all(|pair| pair[0].canonical_bytes() < pair[1].canonical_bytes())
}

fn put_obligation(output: &mut Vec<u8>, obligation: &ObligationSpec) {
    put_bytes(output, &obligation.terminal.canonical_bytes());
    match &obligation.failure {
        Some(failure) => {
            output.push(1);
            put_bytes(output, &failure.canonical_bytes());
        }
        None => output.push(0),
    }
    put_u64(output, obligation.deadline_tick.0);
    put_u64(output, obligation.poll_interval_ticks);
    put_u32(output, obligation.stable_for_observations);
}

fn put_predicates(output: &mut Vec<u8>, predicates: &[Predicate]) {
    put_u64(output, predicates.len() as u64);
    for predicate in predicates {
        put_bytes(output, &predicate.canonical_bytes());
    }
}

fn put_anchor(output: &mut Vec<u8>, anchor: StateAnchor) {
    put_u64(output, anchor.fortress_id.get());
    put_u64(output, anchor.cursor.epoch);
    put_u64(output, anchor.cursor.sequence);
    put_u64(output, anchor.tick.0);
    put_bytes(output, anchor.state_hash.as_bytes());
}

fn risk_code(risk: RiskTier) -> u8 {
    match risk {
        RiskTier::ReadOnly => 0,
        RiskTier::Reversible => 1,
        RiskTier::Guarded => 2,
        RiskTier::Irreversible => 3,
    }
}

fn capability_code(capability: Capability) -> u8 {
    match capability {
        Capability::Observe => 0,
        Capability::Query => 1,
        Capability::Plan => 2,
        Capability::Designate => 3,
        Capability::Construct => 4,
        Capability::ConfigureLabor => 5,
        Capability::ConfigureProduction => 6,
        Capability::ConfigureLogistics => 7,
        Capability::ConfigureMilitary => 8,
        Capability::ControlClock => 9,
        Capability::Checkpoint => 10,
        Capability::Restore => 11,
        Capability::Extension => 12,
        Capability::DiagnosticRaw => 13,
        Capability::Doctor => 14,
        Capability::RepairPlan => 15,
        Capability::RepairApply => 16,
        Capability::Admin => 17,
    }
}

fn put_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_be_bytes());
}

fn put_u64(output: &mut Vec<u8>, value: u64) {
    output.extend_from_slice(&value.to_be_bytes());
}

fn put_bytes(output: &mut Vec<u8>, value: &[u8]) {
    put_u64(output, value.len() as u64);
    output.extend_from_slice(value);
}

fn put_str(output: &mut Vec<u8>, value: &str) {
    put_bytes(output, value.as_bytes());
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use dfmcp_core::{
        Capability, FortressId, GameTick, IntentId, ObservationCursor, RiskTier, StepId,
    };
    use dfmcp_world::{Predicate, WorldGraph, WorldSnapshot};

    use super::{PlanStep, PreparedPlan, derive_step_idempotency_key};
    use crate::Action;

    fn snapshot() -> WorldSnapshot {
        WorldSnapshot::new(
            FortressId::new(42),
            GameTick(100),
            ObservationCursor::ORIGIN,
            true,
            WorldGraph::default(),
        )
    }

    fn valid_plan() -> PreparedPlan {
        let snapshot = snapshot();
        let intent_id = IntentId::new(99);
        let action = Action::Pause { paused: false };
        let step = PlanStep {
            id: StepId::ZERO,
            action: action.clone(),
            preconditions: vec![Predicate::Paused(true)],
            postconditions: vec![Predicate::Paused(false)],
            compensation: Some(Action::Pause { paused: true }),
            obligation: None,
            depends_on: Vec::new(),
            risk: RiskTier::Reversible,
            required_capability: Capability::ControlClock,
            idempotency_key: derive_step_idempotency_key(
                intent_id,
                snapshot.anchor(),
                StepId::ZERO,
                &action,
            ),
        };
        PreparedPlan::builder(
            intent_id,
            snapshot.anchor(),
            "test unpause plan",
            Predicate::Paused(false),
        )
        .steps(vec![step])
        .max_risk(RiskTier::Reversible)
        .required_capabilities(BTreeSet::from([Capability::ControlClock]))
        .requires_checkpoint(false)
        .expires_at_tick(GameTick(200))
        .build()
    }

    fn reseal(plan: &mut PreparedPlan) {
        plan.digest = plan.compute_digest();
        plan.id = plan.expected_id();
    }

    #[test]
    fn builder_produces_consistent_digest_and_id() {
        let plan = valid_plan();
        assert_eq!(plan.intent_id, IntentId::new(99));
        assert_eq!(plan.summary, "test unpause plan");
        assert_eq!(plan.terminal_condition, Predicate::Paused(false));
        assert_eq!(plan.max_risk, RiskTier::Reversible);
        assert_eq!(
            plan.required_capabilities,
            BTreeSet::from([Capability::ControlClock])
        );
        assert_eq!(plan.expires_at_tick, GameTick(200));
        assert!(!plan.requires_checkpoint);
        assert_eq!(plan.digest, plan.compute_digest());
        assert_ne!(plan.digest, dfmcp_core::Digest32::ZERO);
        assert_ne!(plan.id, dfmcp_core::PlanId::NIL);
        assert!(plan.validate_structure().is_ok());
    }

    #[test]
    fn arbitrary_digest_shaped_idempotency_key_is_rejected() {
        let mut plan = valid_plan();
        plan.steps[0].idempotency_key = dfmcp_core::Digest32::of_bytes(b"arbitrary").to_hex();
        reseal(&mut plan);
        assert!(plan.validate_structure().is_err());
    }

    #[test]
    fn true_only_postcondition_is_rejected_even_after_resealing() {
        let mut plan = valid_plan();
        plan.steps[0].postconditions = vec![Predicate::True];
        reseal(&mut plan);
        assert!(plan.validate_structure().is_err());
    }

    #[test]
    fn plan_expiring_at_its_anchor_tick_is_rejected() {
        let mut plan = valid_plan();
        plan.expires_at_tick = plan.anchor.tick;
        reseal(&mut plan);
        assert!(plan.validate_structure().is_err());
    }
}
