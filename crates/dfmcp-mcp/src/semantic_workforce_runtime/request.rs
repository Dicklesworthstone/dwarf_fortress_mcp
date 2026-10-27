//! Closed SetLabor goal compiler; native IDs and work details are never inputs.
use super::{digest, exhausted, invalid};
use dfmcp_core::{Digest32, EntityId, GameTick, IntentId, OperationContext, Result, RiskTier};
use dfmcp_intent::effects::MAX_DEFAULT_OBLIGATION_TICKS;
use dfmcp_intent::{
    Action, Constraint, Intent, ObligationSpec, PreparedPlan, RequestedAction, StaticPlanner,
};
use dfmcp_world::{CompareOp, EntityKind, Predicate, PredicateEvidence, Value};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub expected_observation: String,
    pub units: Vec<String>,
    pub labor: String,
    pub enabled: bool,
    pub deadline_tick: u64,
    #[serde(default = "interval")]
    pub poll_interval_ticks: u64,
    #[serde(default = "samples")]
    pub stable_samples: u32,
}
fn interval() -> u64 {
    10
}
fn samples() -> u32 {
    1
}
impl Request {
    pub fn parse(raw: &str) -> Result<Self> {
        if raw.len() > 4096 {
            return Err(exhausted("semantic workforce request exceeds 4096 bytes"));
        }
        let request: Self = serde_json::from_str(raw)
            .map_err(|_| invalid("invalid closed semantic workforce request"))?;
        request.validate()?;
        Ok(request)
    }
    pub fn observation(&self) -> Result<Digest32> {
        digest(&self.expected_observation)
    }
    pub fn unit_ids(&self) -> Result<Vec<EntityId>> {
        if self.units.is_empty() || self.units.len() > 32 {
            return Err(invalid("select between one and 32 canonical citizens"));
        }
        let mut units = self
            .units
            .iter()
            .map(|raw| {
                let n = raw
                    .parse::<u64>()
                    .map_err(|_| invalid("unit identity must be canonical decimal"))?;
                if n == 0 || raw != &n.to_string() {
                    return Err(invalid("unit identity must be nonzero canonical decimal"));
                }
                Ok(EntityId::new(n))
            })
            .collect::<Result<Vec<_>>>()?;
        units.sort();
        if units.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(invalid(
                "duplicate canonical citizen identities are refused",
            ));
        }
        Ok(units)
    }
    pub fn validate(&self) -> Result<()> {
        self.observation()?;
        self.unit_ids()?;
        if self.labor.is_empty()
            || self.labor.len() > 64
            || !self
                .labor
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
            || self.poll_interval_ticks == 0
            || self.poll_interval_ticks > MAX_DEFAULT_OBLIGATION_TICKS
            || !(1..=64).contains(&self.stable_samples)
        {
            return Err(invalid(
                "labor key and original cadence/stability must be bounded",
            ));
        }
        Ok(())
    }
    pub fn compile(
        &self,
        evidence: &PredicateEvidence<'_>,
        c: &OperationContext,
    ) -> Result<PreparedPlan> {
        self.validate()?;
        if evidence.snapshot().anchor() != c.anchor {
            return Err(invalid(
                "workforce compiler requires the exact observation anchor",
            ));
        }
        let span = self
            .deadline_tick
            .checked_sub(c.anchor.tick.get())
            .filter(|n| *n > 0 && *n <= MAX_DEFAULT_OBLIGATION_TICKS)
            .ok_or_else(|| {
                invalid("goal deadline must be future and at most one game year away")
            })?;
        if self
            .poll_interval_ticks
            .checked_mul(u64::from(self.stable_samples))
            .is_none_or(|n| n > span)
        {
            return Err(invalid(
                "original deadline cannot contain the requested cadence and stability",
            ));
        }
        let units = self.unit_ids()?;
        let mut preconditions = vec![Predicate::Paused(true)];
        let mut goals = Vec::new();
        for id in &units {
            let record = evidence
                .snapshot()
                .graph
                .entities
                .get(id)
                .filter(|r| r.kind == EntityKind::Unit)
                .ok_or_else(|| {
                    invalid("selected citizen is absent from the exact canonical roster")
                })?;
            let identity = record
                .fields
                .get("raw_unit_id")
                .ok_or_else(|| invalid("selected citizen has no typed native identity"))?;
            let native = match &identity.value {
                Value::I64(n) if *n >= 0 => *n,
                _ => return Err(invalid("selected citizen has no observed native identity")),
            };
            preconditions.push(Predicate::EntityExists(*id));
            preconditions.push(Predicate::EntityKind {
                entity_id: *id,
                kind: EntityKind::Unit,
            });
            preconditions.push(Predicate::FieldCompare {
                entity_id: *id,
                field: "raw_unit_id".into(),
                op: CompareOp::Eq,
                value: Value::I64(native),
            });
            goals.push(Predicate::FieldCompare {
                entity_id: *id,
                field: format!("labor.{}", self.labor),
                op: CompareOp::Eq,
                value: Value::Bool(self.enabled),
            });
        }
        // The adapter accepts at most 64 top-level preconditions; one bounded
        // conjunction preserves every selected identity without weakening it.
        let preconditions = vec![Predicate::All(preconditions)];
        let terminal = Predicate::All(goals);
        let action = Action::SetLabor {
            units: units.clone(),
            labor: self.labor.clone(),
            enabled: self.enabled,
        };
        let mut bytes = b"dfmcp-native-semantic-workforce-request/1\0".to_vec();
        bytes.extend_from_slice(self.observation()?.as_bytes());
        for id in &units {
            bytes.extend_from_slice(&id.get().to_be_bytes());
        }
        bytes.extend_from_slice(&(self.labor.len() as u32).to_be_bytes());
        bytes.extend_from_slice(self.labor.as_bytes());
        bytes.push(u8::from(self.enabled));
        for n in [
            self.deadline_tick,
            self.poll_interval_ticks,
            u64::from(self.stable_samples),
        ] {
            bytes.extend_from_slice(&n.to_be_bytes());
        }
        let hash = Digest32::of_bytes(&bytes);
        let id = IntentId::new(u128::from_be_bytes(
            hash.as_bytes()[..16]
                .try_into()
                .map_err(|_| invalid("intent identity width"))?,
        ));
        if id == IntentId::NIL {
            return Err(invalid("reserved intent identity"));
        }
        let intent = Intent {
            id,
            anchor: c.anchor,
            summary: format!(
                "{} {} for {} selected citizens",
                if self.enabled { "Enable" } else { "Disable" },
                self.labor,
                units.len()
            ),
            terminal_condition: terminal.clone(),
            constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
            requested_actions: vec![RequestedAction {
                action,
                preconditions,
                postconditions: vec![terminal.clone()],
                compensation: None,
                depends_on: Vec::new(),
                obligation: Some(ObligationSpec {
                    terminal,
                    failure: None,
                    deadline_tick: GameTick(self.deadline_tick),
                    poll_interval_ticks: self.poll_interval_ticks,
                    stable_for_observations: self.stable_samples,
                }),
            }],
        };
        let mut plan = StaticPlanner::default().prepare_with_evidence(evidence, &intent, c)?;
        // The shared laboratory planner supplies an automatic inverse action.
        // This closed native profile promises no rollback or compensation: its
        // cancellation retires preparation and never undoes assigned labor.
        // Define that contract before the original plan is sealed, archived or
        // offered for review; retain all planner witnesses and original goals.
        for step in &mut plan.steps {
            step.compensation = None;
        }
        plan.digest = plan.compute_digest();
        plan.id = plan.expected_id();
        plan.validate_structure()?;
        Ok(plan)
    }
}
