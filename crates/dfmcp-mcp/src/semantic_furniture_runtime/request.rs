//! Closed original construction goal; native IDs, selectors and policy are not inputs.
use super::{denied, digest, exhausted, invalid};
use dfmcp_adapter::build_placement::{BuildKind, BuildSelection};
use dfmcp_adapter::live_operations::item_entity_id;
use dfmcp_adapter::live_spatial::SpatialStateView;
use dfmcp_adapter::live_spatial::citizens::LiveSpatialCitizenState;
use dfmcp_core::{
    Digest32, EntityId, GameTick, IntentId, MapCoord, MapCuboid, OperationContext, Result,
    RiskTier, StepId,
};
use dfmcp_intent::effects::{MAX_DEFAULT_OBLIGATION_TICKS, default_postconditions};
use dfmcp_intent::{
    Action, BuildingKind, Constraint, Intent, MaterialSelector, ObligationSpec, PlanPolicy,
    PreparedPlan, RequestedAction, StaticPlanner, derive_step_idempotency_key,
};
use dfmcp_world::{EntityKind, Predicate, PredicateEvidence};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub expected_observation: String,
    pub kind: String,
    pub item: String,
    pub target: [u32; 3],
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
            return Err(exhausted("semantic furniture request exceeds 4096 bytes"));
        }
        let value: Self = serde_json::from_str(raw)
            .map_err(|_| invalid("invalid closed semantic furniture request"))?;
        value.validate()?;
        Ok(value)
    }
    pub fn observation(&self) -> Result<Digest32> {
        digest(&self.expected_observation)
    }
    pub fn item_id(&self) -> Result<EntityId> {
        let id = self
            .item
            .parse::<u64>()
            .map_err(|_| invalid("item identity must be canonical decimal"))?;
        if id == 0 || id.to_string() != self.item {
            return Err(invalid("item identity must be nonzero canonical decimal"));
        }
        Ok(EntityId::new(id))
    }
    pub fn kind(&self) -> Result<BuildKind> {
        match self.kind.as_str() {
            "bed" => Ok(BuildKind::Bed),
            "chair" => Ok(BuildKind::Chair),
            "table" => Ok(BuildKind::Table),
            _ => Err(invalid("kind must be bed, chair or table")),
        }
    }
    pub fn validate(&self) -> Result<()> {
        self.observation()?;
        self.item_id()?;
        self.kind()?;
        if self.target.iter().any(|n| *n > 32767)
            || self.poll_interval_ticks == 0
            || self.poll_interval_ticks > MAX_DEFAULT_OBLIGATION_TICKS
            || !(1..=64).contains(&self.stable_samples)
        {
            return Err(invalid(
                "bounded target, cadence and stability are required",
            ));
        }
        Ok(())
    }
    /// Resolve a canonical item by looking it up in the exact typed publication.
    /// Arithmetic decoding of client-supplied native identities is not permitted.
    pub fn selection(&self, state: &LiveSpatialCitizenState) -> Result<BuildSelection> {
        self.validate()?;
        let id = self.item_id()?;
        let snapshot = state
            .snapshot()
            .ok_or_else(|| invalid("spatial observation is absent"))?;
        if snapshot
            .graph
            .entities
            .get(&id)
            .is_none_or(|record| record.kind != EntityKind::Item)
        {
            return Err(invalid(
                "selected canonical item is absent from the exact observation",
            ));
        }
        let observation = state
            .observation_full()
            .ok_or_else(|| invalid("typed spatial source is absent"))?;
        let item = observation
            .spatial()
            .operations()
            .items
            .iter()
            .find(|item| item_entity_id(item.native_id) == id)
            .ok_or_else(|| invalid("selected canonical item has no exact native association"))?;
        // Detailed eligibility is verified by the independent native capture and
        // canonical resolver before preparation. The item choice remains sealed
        // in the paired native plan alongside the unchanged default selector.
        BuildSelection::new(self.kind()?, item.native_id, self.target)
    }
    /// The checkpoint exception is a trusted host decision made before sealing.
    /// A client cannot supply it, and a previously sealed requirement is immutable.
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
                "furniture planning requires the exact canonical source anchor",
            ));
        }
        let span = self
            .deadline_tick
            .checked_sub(c.anchor.tick.get())
            .filter(|n| *n > 0 && *n <= MAX_DEFAULT_OBLIGATION_TICKS)
            .ok_or_else(|| {
                invalid(
                    "original construction deadline must be future and at most one game year away",
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
        let [x, y, z] = self.target;
        let target = MapCoord::new(x as i32, y as i32, z as i32);
        let action = Action::Build {
            kind: BuildingKind::Furniture(self.kind.clone()),
            location: target,
            footprint: MapCuboid::new(target, target)?,
            material: MaterialSelector::default(),
        };
        let mut identity = b"dfmcp-original-furniture-request/1\0".to_vec();
        identity.extend_from_slice(self.observation()?.as_bytes());
        identity.extend_from_slice(&self.item_id()?.get().to_be_bytes());
        identity.extend_from_slice(&(self.kind()?.values_tag()).to_be_bytes());
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
                .map_err(|_| invalid("invalid original construction identity width"))?,
        ));
        if id == IntentId::NIL {
            return Err(invalid("original construction identity is reserved"));
        }
        let key = derive_step_idempotency_key(id, c.anchor, StepId::ZERO, &action);
        let postconditions = default_postconditions(&action, &key, c.anchor.fortress_id);
        let terminal = Predicate::All(postconditions.clone());
        let intent = Intent {
            id,
            anchor: c.anchor,
            summary: format!("Complete the original {} at [{x},{y},{z}]", self.kind),
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
            max_area_tiles: 1,
            require_checkpoint_at_or_above: RiskTier::Irreversible,
            ..PlanPolicy::default()
        };
        StaticPlanner::new(policy).prepare_with_evidence(evidence, &intent, c)
    }
}
trait KindTag {
    fn values_tag(self) -> u32;
}
impl KindTag for BuildKind {
    fn values_tag(self) -> u32 {
        match self {
            Self::Bed => 0,
            Self::Chair => 1,
            Self::Table => 2,
        }
    }
}
