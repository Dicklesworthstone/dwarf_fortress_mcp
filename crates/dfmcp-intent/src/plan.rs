use std::collections::BTreeSet;

use dfmcp_core::{
    Capability, DfmcpError, Digest32, EntityId, ErrorCode, GameTick, IntentId, MapCuboid, PlanId,
    Result, RiskTier, StateAnchor, StepId,
};
use dfmcp_world::Predicate;

use crate::Action;

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
            .unwrap_or(RiskTier::Guarded)
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

impl PreparedPlan {
    #[must_use]
    #[allow(clippy::too_many_arguments)] // sealed-plan constructor mirrors the frozen plan shape
    pub fn from_parts(
        intent_id: IntentId,
        anchor: StateAnchor,
        summary: String,
        terminal_condition: Predicate,
        steps: Vec<PlanStep>,
        max_risk: RiskTier,
        required_capabilities: BTreeSet<Capability>,
        requires_checkpoint: bool,
        expires_at_tick: GameTick,
    ) -> Self {
        let mut plan = Self {
            id: PlanId::NIL,
            intent_id,
            anchor,
            summary,
            terminal_condition: terminal_condition.normalized(),
            steps,
            max_risk,
            required_capabilities,
            requires_checkpoint,
            expires_at_tick,
            digest: Digest32::ZERO,
        };
        plan.digest = plan.compute_digest();
        let candidate = plan.digest.first_u128();
        plan.id = PlanId::new(if candidate == 0 { 1 } else { candidate });
        plan
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
        if self.intent_id == IntentId::NIL || self.summary.is_empty() {
            return Err(DfmcpError::new(
                ErrorCode::InvalidPlan,
                "plan intent identifier and summary must be nonempty",
            ));
        }
        if !self.digest_is_valid() || !self.id_is_valid() {
            return Err(DfmcpError::new(
                ErrorCode::InvalidPlan,
                "plan digest or digest-derived identifier is invalid",
            ));
        }
        if self.terminal_condition != self.terminal_condition.normalized() {
            return Err(DfmcpError::new(
                ErrorCode::InvalidPlan,
                "plan terminal condition is not canonical",
            ));
        }
        if self.steps.is_empty() || self.expires_at_tick < self.anchor.tick {
            return Err(DfmcpError::new(
                ErrorCode::InvalidPlan,
                "plan has no steps or expires before its anchor",
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
                || !predicates_are_canonical(&step.preconditions)
                || !predicates_are_canonical(&step.postconditions)
            {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidPlan,
                    "plan step predicates are empty or noncanonical",
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
            if Digest32::from_hex(&step.idempotency_key).is_none() {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidPlan,
                    "plan step idempotency key is not a SHA-256 digest",
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
                    || obligation.terminal != obligation.terminal.normalized()
                    || obligation
                        .failure
                        .as_ref()
                        .is_some_and(|failure| failure != &failure.normalized()))
            {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidPlan,
                    "plan obligation is unbounded or noncanonical",
                ));
            }
            if step.compensation.as_ref().is_some_and(|action| {
                action != &action.normalized() || action.risk() == RiskTier::Irreversible
            }) {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidPlan,
                    "plan compensation is noncanonical or irreversible",
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
