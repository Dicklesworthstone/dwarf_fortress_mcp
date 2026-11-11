//! Complete original projects evaluated only on discarded laboratory worlds.
//!
//! Future stages use observed simulated yields and the unchanged typed request.
//! They are predictions of separate reviews, never newly admitted game work.

use std::collections::BTreeSet;

use dfmcp_core::{
    Capability, DfmcpError, Digest32, ErrorCode, IntentId, OperationContext, Result, RiskTier,
};
use dfmcp_intent::{Action, Intent, PreparedPlan, StaticPlanner, effects};
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::{PredicateTruth, Value as WorldValue, WorldSnapshot, laboratory_fact_value};
use serde_json::{Value, json};

use super::connected_room::ConnectedRoomRequest;
use super::plan_forecast::{ForecastBudget, MAX_PROJECT_STAGES, forecast_stage_bounded};
use super::room_provisioning::ProvisionedRoomRequest;
use crate::lab_world::{ProductionGoalLedger, ProductionPlanner, ProductionRequest};

/// Fixed original source generations, independent of mutable session history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ProjectSource {
    Production(ProductionRequest),
    Room(ProvisionedRoomRequest),
    ConnectedRoom(ConnectedRoomRequest),
}

fn invalid(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn invariant(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::InternalInvariantViolation, message)
}

impl ProjectSource {
    pub(crate) fn parse_production(raw: &str) -> Result<Self> {
        ProductionRequest::parse(raw).map(Self::Production)
    }

    pub(crate) fn parse_room(raw: &str) -> Result<Self> {
        ProvisionedRoomRequest::parse(raw).map(Self::Room)
    }

    pub(crate) fn parse_connected_room(raw: &str) -> Result<Self> {
        ConnectedRoomRequest::parse(raw).map(Self::ConnectedRoom)
    }

    fn canonical_json(&self) -> String {
        match self {
            Self::Production(request) => request.canonical_json(),
            Self::Room(request) => request.canonical_json(),
            Self::ConnectedRoom(request) => request.canonical_json(),
        }
    }

    fn source_digest(&self) -> Digest32 {
        let mut bytes = b"dfmcp-project-forecast-source/1\0".to_vec();
        bytes.extend_from_slice(self.canonical_json().as_bytes());
        Digest32::of_bytes(&bytes)
    }

    /// The intended result is separate from its means. Setup permissions and
    /// work-order lowering remain in each exact source but are not room/stock
    /// goals. The source's unique original ledger is qualified before comparison.
    pub(crate) fn semantic_key(&self, snapshot: &WorldSnapshot) -> Result<Value> {
        ProductionGoalLedger::capture(snapshot)?;
        let ledger = effects::stock_ledger(snapshot)
            .and_then(|id| snapshot.graph.entities.get(&id))
            .ok_or_else(|| invariant("qualified project ledger disappeared"))?;
        let source: Value = serde_json::from_str(&self.canonical_json())
            .map_err(|error| invariant(format!("canonical project source: {error}")))?;
        let goal = match self {
            Self::Production(_) => json!({
                "kind":"production",
                "quotas":source["quotas"],
                "reserve_until_tick":source.get("reserve_until_tick"),
            }),
            Self::Room(_) => json!({
                "kind":"provisioned_room",
                "layout":source["layout"],
                "furnishing_model":"registered_layout_slots_v1",
            }),
            Self::ConnectedRoom(request) => {
                let movement = match request.planner() {
                    "connected_room_v1" | "connected_room_resources_v1" => "cardinal_v1",
                    "connected_room_stairs_v1" | "connected_room_stairs_resources_v1" => {
                        "paired_stairs_v1"
                    }
                    _ => return Err(invalid("unsupported project room movement generation")),
                };
                json!({
                    "kind":"connected_room",
                    "layout":source["room"]["layout"],
                    "furnishing_model":"registered_layout_slots_v1",
                    "access":source["access"],
                    "movement":movement,
                })
            }
        };
        Ok(json!({
            "schema":"dfmcp.project-semantic-goal/1",
            "ledger":{"entity_id":ledger.id.to_string(),"generation":ledger.generation},
            "goal":goal,
        }))
    }

    fn reserve_until_tick(&self) -> Option<u64> {
        match self {
            Self::Production(request) => request.reserve_until_tick(),
            Self::Room(_) | Self::ConnectedRoom(_) => None,
        }
    }

    fn remaining_reserve_required(&self, snapshot: &WorldSnapshot) -> Result<bool> {
        let Self::Production(request) = self else {
            return Ok(false);
        };
        if request
            .reserve_until_tick()
            .is_none_or(|until| until <= snapshot.tick.0)
        {
            return Ok(false);
        }
        match request.planner() {
            Some(ProductionPlanner::ResourceReserveV1) => {
                request.resource_reserve_work_required(snapshot)
            }
            Some(ProductionPlanner::ConsumptionReserveV1) => {
                request.consumption_reserve_work_required(snapshot)
            }
            _ => Err(invariant(
                "project reserve lost its exact compiler generation",
            )),
        }
    }

    fn goal_truth(
        &self,
        snapshot: &WorldSnapshot,
        ledger: ProductionGoalLedger,
    ) -> Result<PredicateTruth> {
        ledger.validate_current(snapshot)?;
        match self {
            Self::Production(request) => request.semantic_goal_truth_in_ledger(snapshot, ledger),
            Self::Room(request) => request.semantic_goal_truth(snapshot),
            Self::ConnectedRoom(request) => request.semantic_goal_truth(snapshot),
        }
    }

    fn compile_next(&self, id: IntentId, snapshot: &WorldSnapshot) -> Result<(Intent, Value)> {
        const SUMMARY: &str = "hypothetical separately reviewed original project stage";
        match self {
            Self::Production(request) => {
                super::production_source::compile(request, id, snapshot, SUMMARY)
            }
            Self::Room(request) => request.compile_residual(id, snapshot, SUMMARY),
            Self::ConnectedRoom(request) => request.compile_residual(id, snapshot, SUMMARY),
        }
    }

    fn stage_name(&self, plan: &PreparedPlan) -> &'static str {
        if plan
            .steps
            .iter()
            .any(|step| matches!(step.action, Action::HarvestResource { .. }))
        {
            "acquire_resources"
        } else {
            match self {
                Self::Production(_) => "production",
                Self::Room(_) | Self::ConnectedRoom(_) => "room",
            }
        }
    }
}

struct ProjectProgress {
    stages: Vec<Value>,
    plans: Vec<PreparedPlan>,
    max_risk: RiskTier,
    checkpoint: bool,
    capabilities: BTreeSet<Capability>,
    actions: u64,
    slices: u64,
    reserve: Option<Value>,
}

impl ProjectProgress {
    fn new() -> Self {
        Self {
            stages: Vec::new(),
            plans: Vec::new(),
            max_risk: RiskTier::ReadOnly,
            checkpoint: false,
            capabilities: BTreeSet::new(),
            actions: 0,
            slices: 0,
            reserve: None,
        }
    }

    fn admit(&mut self, plan: &PreparedPlan) {
        self.max_risk = self.max_risk.max(plan.max_risk);
        self.checkpoint |= plan.requires_checkpoint;
        self.capabilities
            .extend(plan.required_capabilities.iter().copied());
        self.actions += plan.steps.len() as u64;
        self.plans.push(plan.clone());
    }

    fn record(&mut self, source: &ProjectSource, plan: &PreparedPlan, prediction: &Value) {
        let complete = prediction["available"] == true
            && prediction["predicted_actions_complete"] == true
            && prediction["predicted_physical_quiescent"] == true;
        self.slices += prediction["simulation_slices"].as_u64().unwrap_or(0);
        if self.reserve.is_none() && prediction["reserve_forecast"]["available"] == true {
            self.reserve = Some(prediction["reserve_forecast"].clone());
        }
        self.stages.push(json!({
            "index":self.stages.len() + 1,
            "stage":source.stage_name(plan),
            "model_plan_digest":plan.digest.to_hex(),
            "executable_review":false,
            "from_tick":plan.anchor.tick.0,
            "predicted_frontier_tick":prediction["predicted_frontier_tick"],
            "predicted_actions_complete":prediction["predicted_actions_complete"],
            "predicted_goal_truth":prediction["predicted_goal_truth"],
            "predicted_physical_quiescent":prediction["predicted_physical_quiescent"],
            "predicted_stage_complete":complete,
            "action_count":plan.steps.len(),
            "max_risk":plan.max_risk.as_str(),
            "requires_checkpoint":plan.requires_checkpoint,
            "simulation_slices":prediction["simulation_slices"],
            "available":prediction["available"],
            "reason":prediction.get("reason"),
        }));
    }

    fn decorate(&self, result: &mut Value) {
        result["stages"] = json!(self.stages);
        result["project_max_risk"] = json!(self.max_risk.as_str());
        result["project_requires_checkpoint"] = json!(self.checkpoint);
        result["project_action_count"] = json!(self.actions);
        result["project_required_capabilities"] = json!(
            self.capabilities
                .iter()
                .map(|capability| capability.as_str())
                .collect::<Vec<_>>()
        );
        result["simulation_slices"] = json!(self.slices);
    }

    fn quiescent(&self, adapter: &MemoryAdapter) -> Result<bool> {
        for plan in &self.plans {
            for step in &plan.steps {
                let receipt = adapter.step_receipt(plan.id, step.id).ok_or_else(|| {
                    invariant("project forecast lost its original stage action custody")
                })?;
                if !adapter.action_work_state(receipt.action_id)?.is_quiescent() {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
}

fn truth_name(truth: PredicateTruth) -> &'static str {
    match truth {
        PredicateTruth::True => "true",
        PredicateTruth::False => "false",
        PredicateTruth::Unknown => "unknown",
    }
}

fn model_error(error: DfmcpError, progress: &ProjectProgress) -> Result<Value> {
    if matches!(
        error.code,
        ErrorCode::BudgetExceeded | ErrorCode::CancellationRequested
    ) {
        return Err(error);
    }
    let mut report = json!({
        "schema":"dfmcp.project-forecast/1",
        "epistemic_state":"predicted",
        "available":false,
        "reason":{"code":error.code.as_str(),"message":error.message},
        "effect_evidence":false,
        "actual_game_ticks_advanced":0,
    });
    progress.decorate(&mut report);
    Ok(report)
}

/// Planning authority can describe a future stage without authorizing its
/// effects. Refuse that stage before charging or forking its simulation; the
/// adapter still rechecks these grants during ordinary preparation and commit.
fn authorize_model_stage(plan: &PreparedPlan, context: &OperationContext) -> Result<()> {
    for step in &plan.steps {
        let scope = step.action.scope();
        context.authorize(
            step.required_capability,
            step.risk,
            &scope.entity_ids,
            scope.map_area,
        )?;
    }
    if plan.requires_checkpoint {
        context.authorize(Capability::Checkpoint, RiskTier::Guarded, &[], None)?;
    }
    Ok(())
}

/// Registered stock fields only; unknown evidence stays explicitly unknown.
fn stock_frontier(snapshot: &WorldSnapshot, ledger: ProductionGoalLedger) -> Result<Value> {
    ledger.validate_current(snapshot)?;
    let record = effects::stock_ledger(snapshot)
        .and_then(|id| snapshot.graph.entities.get(&id))
        .ok_or_else(|| invariant("project stock ledger disappeared"))?;
    let stocks = [
        ("WOOD", effects::STOCK_WOOD_FIELD),
        ("PLANT", effects::STOCK_PLANT_FIELD),
        ("DRINK", effects::STOCK_DRINK_FIELD),
        ("FOOD", effects::STOCK_FOOD_FIELD),
        ("BED", effects::STOCK_BED_FIELD),
        ("DOOR", effects::STOCK_DOOR_FIELD),
        ("TABLE", effects::STOCK_TABLE_FIELD),
        ("CHAIR", effects::STOCK_CHAIR_FIELD),
    ]
    .into_iter()
    .map(|(item, field)| {
        let quantity = match record
            .fields
            .get(field)
            .and_then(|fact| laboratory_fact_value(fact, snapshot.tick))
        {
            Some(WorldValue::U64(quantity)) => Some(*quantity),
            _ => None,
        };
        json!({"item":item,"quantity":quantity,"known":quantity.is_some()})
    })
    .collect::<Vec<_>>();
    Ok(json!({
        "epistemic_state":"predicted",
        "evidence_scope":"discarded_laboratory_project",
        "predicted_tick":snapshot.tick.0,
        "ledger":{"entity_id":record.id.to_string(),"generation":record.generation},
        "stocks":stocks,
    }))
}

fn reserve_frontier(
    source: &ProjectSource,
    adapter: &MemoryAdapter,
    ledger: ProductionGoalLedger,
    progress: &ProjectProgress,
) -> Result<Value> {
    Ok(json!({
        "epistemic_state":"predicted",
        "available":true,
        "predicted_tick":adapter.snapshot().tick.0,
        "predicted_state_hash":adapter.snapshot().state_hash.to_hex(),
        "predicted_goal_truth":truth_name(source.goal_truth(adapter.snapshot(), ledger)?),
        "predicted_physical_quiescent":progress.quiescent(adapter)?,
    }))
}

/// Forecast at most four successive separately reviewed stages, preserving one
/// original request, owner, stock ledger, horizon and aggregate work allowance.
/// Only the first plan is an actual current review. All later plans stay private
/// model artifacts and every simulated world is discarded at this boundary.
pub(crate) fn forecast_project_bounded(
    adapter: &MemoryAdapter,
    source: &ProjectSource,
    initial_plan: &PreparedPlan,
    template: &OperationContext,
    budget: &mut ForecastBudget,
) -> Result<Value> {
    budget.begin_project(adapter.snapshot(), initial_plan, template)?;
    let ledger = ProductionGoalLedger::capture(adapter.snapshot())?;
    ledger.validate_plan(initial_plan)?;
    let semantic_key = source.semantic_key(adapter.snapshot())?;
    let source_digest = source.source_digest();
    let until = source.reserve_until_tick();
    let mut progress = ProjectProgress::new();
    let mut current = None;
    let mut plan = initial_plan.clone();
    let mut completed = false;
    let mut stop = "stage_failed";
    let mut last_prediction = Value::Null;

    for index in 0..MAX_PROJECT_STAGES {
        budget.check(template)?;
        progress.admit(&plan);
        let input = current.as_ref().map_or(adapter, |world| world);
        let stage = forecast_stage_bounded(
            input,
            &plan,
            template,
            until,
            &|snapshot| source.goal_truth(snapshot, ledger),
            budget,
        )?;
        progress.record(source, &plan, &stage.prediction);
        if stage.prediction["available"] != true {
            let mut unavailable = stage.prediction;
            unavailable["schema"] = json!("dfmcp.project-forecast/1");
            unavailable["effect_evidence"] = json!(false);
            unavailable["actual_game_ticks_advanced"] = json!(0);
            progress.decorate(&mut unavailable);
            return Ok(unavailable);
        }
        let Some(frontier) = stage.frontier else {
            return Err(invariant("available project stage lost its model frontier"));
        };
        let verified = stage.prediction["predicted_actions_complete"] == true;
        let quiet = stage.prediction["predicted_physical_quiescent"] == true;
        last_prediction = stage.prediction;
        current = Some(frontier);
        let world = current
            .as_ref()
            .ok_or_else(|| invariant("project model frontier missing"))?;
        budget.inspect_project_source(world.snapshot(), template)?;
        if !verified || !quiet {
            stop = if last_prediction["blocked_by_pause"] == true {
                "paused"
            } else if last_prediction["simulation_horizon_truncated"] == true {
                "common_horizon"
            } else {
                "stage_failed_or_unquiet"
            };
            break;
        }
        let truth = match source.goal_truth(world.snapshot(), ledger) {
            Ok(truth) => truth,
            Err(error) => return model_error(error, &progress),
        };
        if truth == PredicateTruth::Unknown {
            stop = "unknown_original_goal";
            break;
        }
        // The original future frontier belongs to the complete project. Later
        // stock cannot rescue a missed frontier, even when every action now
        // verifies and the current quota is True. A True exact endpoint may
        // precede final stability observation: it is not an action deadline.
        // Always require the captured original goal, never an inferred endpoint.
        if until.is_some_and(|until| {
            world.snapshot().tick.0 >= until
                && !progress.reserve.as_ref().is_some_and(|reserve| {
                    reserve["available"] == true
                        && reserve["predicted_tick"] == until
                        && reserve["predicted_goal_truth"] == "true"
                })
        }) {
            stop = "original_reserve_expired";
            break;
        }
        let reserve_needed = match source.remaining_reserve_required(world.snapshot()) {
            Ok(required) => required,
            Err(error) => return model_error(error, &progress),
        };
        if truth == PredicateTruth::True && !reserve_needed {
            completed = true;
            stop = "complete";
            break;
        }
        if until.is_some_and(|tick| tick <= world.snapshot().tick.0) {
            stop = "original_reserve_expired";
            break;
        }
        if world.snapshot().tick.0 >= budget.horizon_tick() {
            stop = "common_horizon";
            break;
        }
        if index + 1 == MAX_PROJECT_STAGES {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "complete project needs more than four separately reviewed model stages",
            ));
        }
        // This identity is deterministic and belongs only to this discarded
        // project. Actual selection/continuation receives a new ordinary ID.
        let identity = format!(
            "dfmcp-project-forecast-stage/1\0{}:{}:{}:{}",
            initial_plan.digest.to_hex(),
            source_digest.to_hex(),
            index + 2,
            world.snapshot().state_hash.to_hex(),
        );
        let id = IntentId::new(Digest32::of_bytes(identity.as_bytes()).first_u128().max(1));
        budget.check(template)?;
        let (intent, _) = match source.compile_next(id, world.snapshot()) {
            Ok(compiled) => compiled,
            Err(error) => return model_error(error, &progress),
        };
        budget.check(template)?;
        let model_context = OperationContext {
            anchor: world.snapshot().anchor(),
            ..template.clone()
        };
        let next = match StaticPlanner::default().prepare_laboratory(
            world.snapshot(),
            &intent,
            &model_context,
        ) {
            Ok(plan) => plan,
            Err(error) => return model_error(error, &progress),
        };
        if let Err(error) = authorize_model_stage(&next, &model_context) {
            return model_error(error, &progress);
        }
        budget.admit_project_stage(world.snapshot(), &next, template)?;
        plan = next;
    }

    let final_world = current
        .as_ref()
        .ok_or_else(|| invariant("project never evaluated a stage"))?;
    budget.check(template)?;
    let truth = match source.goal_truth(final_world.snapshot(), ledger) {
        Ok(truth) => truth,
        Err(error) => return model_error(error, &progress),
    };
    let quiet = match progress.quiescent(final_world) {
        Ok(quiet) => quiet,
        Err(error) => return model_error(error, &progress),
    };
    let all_verified = progress
        .stages
        .iter()
        .all(|stage| stage["predicted_stage_complete"] == true);
    let complete = completed && all_verified && quiet && truth == PredicateTruth::True;
    let predicted_stock = match stock_frontier(final_world.snapshot(), ledger) {
        Ok(stock) => stock,
        Err(error) => return model_error(error, &progress),
    };
    let mut report = json!({
        "schema":"dfmcp.project-forecast/1",
        "epistemic_state":"predicted",
        "available":true,
        "method":"complete_original_project_on_discarded_laboratory_stages",
        "forecast_scope":"complete_project",
        "project_source_digest":source_digest.to_hex(),
        "semantic_goal":semantic_key,
        "from_tick":adapter.snapshot().tick.0,
        "horizon_tick":budget.horizon_tick(),
        "simulation_horizon_tick":budget.horizon_tick(),
        "simulation_horizon_reached":final_world.snapshot().tick.0 >= budget.horizon_tick(),
        "simulation_horizon_truncated":stop == "common_horizon",
        "predicted_complete":complete,
        "predicted_completion_tick":complete.then_some(final_world.snapshot().tick.0),
        "predicted_actions_complete":all_verified,
        "predicted_goal_complete":truth == PredicateTruth::True,
        "predicted_goal_truth":truth_name(truth),
        "predicted_physical_quiescent":quiet,
        "predicted_frontier_tick":final_world.snapshot().tick.0,
        "predicted_stock":predicted_stock,
        "blocked_by_pause":last_prediction["blocked_by_pause"],
        "stop_reason":stop,
        "requires_review_between_stages":true,
        "future_stage_plans_are_executable":false,
        "effect_evidence":false,
        "actual_game_ticks_advanced":0,
        "assumptions":"the agent promptly reviews each successful stage; no other agent acts; only this project's deferred actions dispatch; ordinary current authority and proof must be checked again in the real world",
    });

    if let Some(until) = until {
        if progress.reserve.is_none()
            && complete
            && !final_world.snapshot().paused
            && final_world.snapshot().tick.0 < until
            && until <= budget.horizon_tick()
        {
            budget.before_fork(final_world.snapshot(), template)?;
            let mut tail = final_world.clone();
            let advance = until - tail.snapshot().tick.0;
            budget.before_advance(advance, template)?;
            if let Err(error) = tail.advance_ticks(advance) {
                return model_error(error, &progress);
            }
            budget.advanced(advance);
            progress.slices += 1;
            budget.check(template)?;
            progress.reserve = Some(match reserve_frontier(source, &tail, ledger, &progress) {
                Ok(frontier) => frontier,
                Err(error) => return model_error(error, &progress),
            });
        }
        let mut reserve = progress.reserve.clone().unwrap_or_else(|| json!({
            "epistemic_state":"predicted",
            "available":false,
            "reason":"the complete project did not establish the original requested frontier within its bounded simulation",
        }));
        reserve["reserve_until_tick"] = json!(until);
        reserve["continuous_maintenance_proven"] = json!(false);
        reserve["changes_action_deadlines"] = json!(false);
        let reserve_satisfied = reserve["available"] == true
            && reserve["predicted_tick"] == until
            && reserve["predicted_goal_truth"] == "true";
        if complete && !reserve_satisfied {
            report["predicted_complete"] = json!(false);
            report["predicted_completion_tick"] = Value::Null;
            report["stop_reason"] = json!(if reserve["available"] == true {
                "original_reserve_unmet"
            } else {
                "original_reserve_unavailable"
            });
        }
        report["reserve_forecast"] = reserve;
    }
    progress.decorate(&mut report);
    budget.check(template)?;
    Ok(report)
}

#[cfg(test)]
#[path = "lab_project_forecast_tests.rs"]
mod tests;
