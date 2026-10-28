//! One original bounded Mine intent. Native identifiers and operator policy are not inputs.
use super::{denied, digest, exhausted, invalid};
use dfmcp_adapter::dig_designation::DigRegion;
use dfmcp_core::{
    Digest32, GameTick, IntentId, MapCoord, MapCuboid, OperationContext, Result, RiskTier, StepId,
};
use dfmcp_intent::effects::{MAX_DEFAULT_OBLIGATION_TICKS, default_postconditions};
use dfmcp_intent::{
    Action, Constraint, DigMode, Intent, ObligationSpec, PlanPolicy, PreparedPlan, RequestedAction,
    StaticPlanner, derive_step_idempotency_key,
};
use dfmcp_world::{Predicate, PredicateEvidence};
use serde::Deserialize;
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub expected_observation: String,
    pub kind: String,
    pub target: [u32; 5],
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
            return Err(exhausted("semantic dig request exceeds 4096 bytes"));
        }
        let value: Self = serde_json::from_str(raw)
            .map_err(|_| invalid("invalid closed semantic dig request"))?;
        value.validate()?;
        Ok(value)
    }
    pub fn observation(&self) -> Result<Digest32> {
        digest(&self.expected_observation)
    }
    pub fn region(&self) -> Result<DigRegion> {
        let [x, y, z, w, h] = self.target;
        DigRegion::new(x, y, z, w, h)
    }
    pub fn area(&self) -> Result<MapCuboid> {
        self.region()?;
        let [x, y, z, w, h] = self.target;
        MapCuboid::new(
            MapCoord::new(x as i32, y as i32, z as i32),
            MapCoord::new((x + w - 1) as i32, (y + h - 1) as i32, z as i32),
        )
    }
    pub fn validate(&self) -> Result<()> {
        self.observation()?;
        self.region()?;
        if self.kind != "mine"
            || self.poll_interval_ticks == 0
            || self.poll_interval_ticks > MAX_DEFAULT_OBLIGATION_TICKS
            || !(1..=64).contains(&self.stable_samples)
        {
            return Err(invalid(
                "kind must be mine with bounded positive cadence and stability 1..64",
            ));
        }
        Ok(())
    }
    pub fn compile(
        &self,
        evidence: &PredicateEvidence<'_>,
        c: &OperationContext,
        checkpoint_exception: bool,
    ) -> Result<PreparedPlan> {
        self.validate()?;
        if !checkpoint_exception {
            return Err(denied(
                "this profile has no checkpoint provider; operator development exception is required before planning",
            ));
        }
        if evidence.snapshot().anchor() != c.anchor {
            return Err(invalid(
                "dig planning requires the exact canonical terrain source anchor",
            ));
        }
        let span = self
            .deadline_tick
            .checked_sub(c.anchor.tick.get())
            .filter(|n| *n > 0 && *n <= MAX_DEFAULT_OBLIGATION_TICKS)
            .ok_or_else(|| {
                invalid(
                    "original excavation deadline must be future and at most one game year away",
                )
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
        let action = Action::DesignateDig {
            area: self.area()?,
            mode: DigMode::Mine,
        };
        let mut identity = b"dfmcp-original-dig-request/1\0".to_vec();
        identity.extend_from_slice(self.observation()?.as_bytes());
        for n in self.target {
            identity.extend_from_slice(&n.to_be_bytes());
        }
        identity.extend_from_slice(&self.deadline_tick.to_be_bytes());
        identity.extend_from_slice(&self.poll_interval_ticks.to_be_bytes());
        identity.extend_from_slice(&self.stable_samples.to_be_bytes());
        let hash = Digest32::of_bytes(&identity);
        let id = IntentId::new(u128::from_be_bytes(
            hash.as_bytes()[..16]
                .try_into()
                .map_err(|_| invalid("invalid original excavation identity width"))?,
        ));
        if id == IntentId::NIL {
            return Err(invalid("original excavation identity is reserved"));
        }
        let key = derive_step_idempotency_key(id, c.anchor, StepId::ZERO, &action);
        let postconditions = default_postconditions(&action, &key, c.anchor.fortress_id);
        let terminal = Predicate::All(postconditions.clone());
        let intent = Intent {
            id,
            anchor: c.anchor,
            summary: format!(
                "Excavate the original {} by {} Mine rectangle at [{},{},{}]",
                self.target[3], self.target[4], self.target[0], self.target[1], self.target[2]
            ),
            terminal_condition: terminal.clone(),
            constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
            requested_actions: vec![RequestedAction {
                action,
                preconditions: vec![Predicate::Paused(true)],
                postconditions,
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
        let policy = PlanPolicy {
            max_steps: 1,
            max_area_tiles: 64,
            require_checkpoint_at_or_above: RiskTier::Irreversible,
            ..PlanPolicy::default()
        };
        StaticPlanner::new(policy).prepare_with_evidence(evidence, &intent, c)
    }
}
