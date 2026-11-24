//! Candidate-stage and complete-project forecasts under authenticated owned work.
//! The shared original forecast engine owns all polling and time progression.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use dfmcp_adapter::{ActionReceipt, GameAdapter};
use dfmcp_core::{
    ActionId, Capability, Digest32, OperationContext, Result, RiskTier, SessionId, StateAnchor,
};
use dfmcp_intent::{PlanStep, PreparedPlan};
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::{Predicate, WorldSnapshot};
use serde_json::{Value, json};

use super::super::{
    CarriedStep, LabSession, original_forecast::model, plan_cancellation,
    project_forecast::ProjectSource, recovered_step_progress,
};
use super::{ForecastBudget, exhausted, invalid};
use crate::lab_world::ProductionGoalLedger;
use model::ForecastMeter;

const MAX_TRACKED: usize = 1_024;
const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;

pub(crate) struct OwnedWork {
    session: SessionId,
    anchor: StateAnchor,
    actions: Vec<ActionId>,
    carried: Vec<CarriedStep>,
    live_originals: Vec<Digest32>,
    recovered_originals: Vec<Digest32>,
    records: u64,
    bytes: u64,
}

fn archive(plan: &PreparedPlan, context: &OperationContext, started: Instant) -> Result<Vec<u8>> {
    dfmcp_intent::plan_archive::encode_with_check(
        plan,
        dfmcp_intent::plan_archive::ArchiveLimits {
            max_bytes: usize::try_from(context.budget.max_bytes.min(MAX_SOURCE_BYTES)).map_err(
                |_| exhausted("candidate original custody exceeds the host byte domain"),
            )?,
            max_steps: 64,
            ..dfmcp_intent::plan_archive::ArchiveLimits::default()
        },
        || super::check_wall(context, started),
    )
    .map(|(bytes, _)| bytes)
}

fn nodes<'a>(predicates: impl IntoIterator<Item = &'a Predicate>) -> Result<u64> {
    let mut count = 0u64;
    for predicate in predicates {
        predicate.validate_shape()?;
        count = count.saturating_add(predicate.complexity() as u64);
        if count > 4_000_000 {
            return Err(exhausted(
                "owned candidate predicate domain exceeds its bound",
            ));
        }
    }
    Ok(count)
}

fn step_nodes(step: &PlanStep) -> Result<u64> {
    let count = nodes(
        step.preconditions
            .iter()
            .chain(&step.postconditions)
            .chain(step.obligation.iter().map(|spec| &spec.terminal))
            .chain(
                step.obligation
                    .iter()
                    .filter_map(|spec| spec.failure.as_ref()),
            ),
    )?;
    // Deferred readiness, monitor registration and post-dispatch observation
    // may evaluate the same failure/proof twice. Include conjunction roots.
    Ok(count.saturating_add(2).saturating_mul(4))
}

impl OwnedWork {
    pub(crate) fn admit(
        session: &LabSession,
        context: &OperationContext,
        started: Instant,
    ) -> Result<Self> {
        super::authorize(session.adapter.snapshot(), context)?;
        super::check_wall(context, started)?;
        if context.session_id != session.session_id {
            return Err(invalid("owned candidate work requires its current session"));
        }
        if session.open_actions.len() > MAX_TRACKED
            || session.open_actions.len() > context.budget.max_actions as usize
            || session.open_actions.len() > context.budget.max_entities as usize
        {
            return Err(exhausted(
                "owned candidate live action domain exceeds its shared budget",
            ));
        }
        let unique: BTreeSet<_> = session.open_actions.iter().copied().collect();
        if unique.len() != session.open_actions.len() {
            return Err(invalid(
                "owned candidate progression contains duplicate live actions",
            ));
        }
        let mut live_originals = Vec::new();
        let mut admitted = BTreeSet::new();
        let mut records = 0u64;
        let mut bytes = 0u64;
        for action in &session.open_actions {
            super::check_wall(context, started)?;
            let receipt = session
                .adapter
                .action_plan_receipt(*action)
                .ok_or_else(|| invalid("competing live action lost its original commit"))?;
            if !admitted.insert(receipt.plan_digest) {
                continue;
            }
            let selected = plan_cancellation::select_for_census(session, receipt.plan_digest)?;
            let original = session
                .objectives
                .iter()
                .find(|original| original.plan_digest == selected.actual)
                .and_then(|original| original.resource_plan())
                .ok_or_else(|| invalid("competing live original lost its sealed plan"))?;
            if original.digest != selected.actual
                || original.id.to_string() != selected.plan_id
                || original.steps.len() != selected.actions.len()
                || !selected.actions.contains(action)
            {
                return Err(invalid(
                    "competing live original has inconsistent complete custody",
                ));
            }
            for (id, step) in selected.actions.iter().zip(&original.steps) {
                if session.adapter.action_step(*id) != Some(step) {
                    return Err(invalid(
                        "competing live original changed its sealed action definition",
                    ));
                }
                if !unique.contains(id)
                    && !super::super::action_fully_drained(&session.adapter, *id)
                {
                    return Err(invalid(
                        "competing live original omitted unfinished work from owned progression",
                    ));
                }
            }
            bytes = bytes
                .saturating_add(archive(original, context, started)?.len() as u64)
                .saturating_add(
                    session
                        .commit_receipts
                        .get(&selected.actual.to_hex())
                        .ok_or_else(|| invalid("competing original commit custody disappeared"))?
                        .len() as u64,
                );
            records = records.saturating_add(original.steps.len() as u64);
            live_originals.push(selected.actual);
        }
        let recovered_originals: Vec<_> = session
            .carried
            .iter()
            .filter(|row| !row.is_final())
            .map(|row| row.plan_digest)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut carried = Vec::new();
        for digest in &recovered_originals {
            super::check_wall(context, started)?;
            if admitted.contains(digest) {
                return Err(invalid(
                    "owned candidate work cannot mix live and recovered custody for one original",
                ));
            }
            let owner = session
                .objectives
                .iter()
                .find(|original| original.plan_digest == *digest)
                .ok_or_else(|| invalid("competing recovered work lost its original owner"))?;
            let (rows, custody_bytes) = owner.forecast_carried_progress(session)?;
            let plan = owner
                .resource_plan()
                .ok_or_else(|| invalid("competing recovered work lost its sealed plan"))?;
            bytes = bytes
                .saturating_add(custody_bytes as u64)
                .saturating_add(archive(plan, context, started)?.len() as u64);
            records = records.saturating_add(plan.steps.len() as u64);
            for row in &rows {
                bytes = bytes.saturating_add(row.to_json().to_string().len() as u64);
            }
            carried.extend(rows);
        }
        let complete_actions = session.open_actions.len().saturating_add(carried.len());
        if complete_actions > MAX_TRACKED
            || complete_actions > context.budget.max_actions as usize
            || complete_actions > context.budget.max_entities as usize
        {
            return Err(exhausted(
                "owned candidate live and recovered work exceeds its complete action budget",
            ));
        }
        model::recovered_predicate_node_bound(&carried)?;
        let spent = started.elapsed().as_nanos().div_ceil(1_000_000);
        if spent >= u128::from(context.budget.max_wall_millis) {
            return Err(exhausted(
                "owned candidate custody exhausted the shared wall-time budget",
            ));
        }
        let mut remaining = context.clone();
        remaining.budget.max_wall_millis -= spent as u64;
        let source =
            super::super::query_budget::admit_sources(&[session.adapter.snapshot()], &remaining)?;
        let source_records = source["records"]
            .as_u64()
            .ok_or_else(|| invalid("owned candidate source record accounting is missing"))?;
        let source_bytes = source["canonical_bytes"]
            .as_u64()
            .ok_or_else(|| invalid("owned candidate source byte accounting is missing"))?;
        if source_records.saturating_add(records) > u64::from(context.budget.max_entities)
            || source_bytes.saturating_add(bytes) > context.budget.max_bytes.min(MAX_SOURCE_BYTES)
        {
            return Err(exhausted(
                "owned candidate source and complete original custody exceed the shared budget",
            ));
        }
        super::check_wall(context, started)?;
        Ok(Self {
            session: context.session_id,
            anchor: context.anchor,
            actions: session.open_actions.clone(),
            carried,
            live_originals,
            recovered_originals,
            records,
            bytes,
        })
    }

    pub(super) fn describe(&self) -> Value {
        json!({
            "scope":"authenticated_current_session_open_work",
            "live_originals":self.live_originals.iter().map(|digest| digest.to_hex()).collect::<Vec<_>>(),
            "recovered_originals":self.recovered_originals.iter().map(|digest| digest.to_hex()).collect::<Vec<_>>(),
            "live_action_count":self.actions.len(), "recovered_observation_steps":self.carried.len(),
            "retained_custody_records":self.records, "retained_custody_bytes":self.bytes,
            "live_action_order":"existing_session_commit_order_then_candidate",
            "other_sessions_deferred_actions_included":false,
            "recovered_dispatch_handles_restored":false,
            "peer_original_goals_selected":false,
        })
    }

    pub(super) fn forecast(
        &self,
        adapter: &MemoryAdapter,
        plan: &PreparedPlan,
        context: &OperationContext,
        reserve: Option<u64>,
        started: Instant,
        budget: &mut ForecastBudget,
    ) -> Result<Value> {
        self.forecast_candidate(adapter, None, plan, context, reserve, started, budget)
    }

    pub(super) fn forecast_project(
        &self,
        adapter: &MemoryAdapter,
        source: &ProjectSource,
        plan: &PreparedPlan,
        context: &OperationContext,
        reserve: Option<u64>,
        started: Instant,
        budget: &mut ForecastBudget,
    ) -> Result<Value> {
        if source.reserve_until_tick() != reserve {
            return Err(invalid(
                "owned candidate project changed its original reserve frontier",
            ));
        }
        self.forecast_candidate(
            adapter,
            Some(source),
            plan,
            context,
            reserve,
            started,
            budget,
        )
    }

    fn forecast_candidate(
        &self,
        adapter: &MemoryAdapter,
        project: Option<&ProjectSource>,
        plan: &PreparedPlan,
        context: &OperationContext,
        reserve: Option<u64>,
        started: Instant,
        budget: &mut ForecastBudget,
    ) -> Result<Value> {
        if self.session != context.session_id
            || self.anchor != context.anchor
            || adapter.snapshot().anchor() != self.anchor
        {
            return Err(invalid(
                "owned candidate progression changed its admitted session or source",
            ));
        }
        if project.is_some() {
            budget.begin_project(adapter.snapshot(), plan, context)?;
        } else {
            budget.begin_candidate(adapter.snapshot(), plan, context)?;
        }
        let plan_bytes = archive(plan, context, started)?.len() as u64;
        budget.admit_owned_candidate(
            self.actions.len().saturating_add(self.carried.len()) as u64,
            self.records.saturating_add(plan.steps.len() as u64),
            self.bytes.saturating_add(plan_bytes),
            context,
        )?;
        // The existing engine snapshots a committed private world. Account for
        // this first clone and its second model clone under one shared meter.
        budget.before_fork(adapter.snapshot(), context)?;
        let mut fork = adapter.clone();
        let result = self.forecast_committed(
            adapter, &mut fork, project, plan, context, reserve, started, budget,
        );
        budget.check(context)?;
        match result {
            Err(error)
                if !matches!(
                    error.code,
                    dfmcp_core::ErrorCode::BudgetExceeded
                        | dfmcp_core::ErrorCode::CancellationRequested
                ) =>
            {
                match project {
                    Some(source) => self.project_unavailable(
                        &error,
                        source,
                        adapter.snapshot(),
                        vec![stage_cost(source, plan, 1)],
                    ),
                    None => Ok(json!({"epistemic_state":"predicted", "available":false,
                        "progression_scope":"owned_session", "competing_work":self.describe(),
                        "reason":{"code":error.code.as_str(),"message":error.message}})),
                }
            }
            other => other,
        }
    }

    fn project_unavailable(
        &self,
        error: &dfmcp_core::DfmcpError,
        source: &ProjectSource,
        snapshot: &WorldSnapshot,
        stages: Vec<Value>,
    ) -> Result<Value> {
        let mut value = json!({
            "epistemic_state":"predicted", "available":false,
            "progression_scope":"owned_session", "competing_work":self.describe(),
            "reason":{"code":error.code.as_str(),"message":error.message},
            "stage_outcomes_available":false,
            "effect_evidence":false, "actual_game_ticks_advanced":0,
        });
        decorate_project_stages(&mut value, source, snapshot, stages)?;
        Ok(value)
    }

    fn forecast_committed(
        &self,
        adapter: &MemoryAdapter,
        fork: &mut MemoryAdapter,
        project: Option<&ProjectSource>,
        plan: &PreparedPlan,
        context: &OperationContext,
        reserve: Option<u64>,
        started: Instant,
        budget: &mut ForecastBudget,
    ) -> Result<Value> {
        if reserve.is_some_and(|tick| {
            tick <= context.anchor.tick.0
                || tick - context.anchor.tick.0
                    > dfmcp_intent::effects::MAX_DEFAULT_OBLIGATION_TICKS
        }) {
            return Err(invalid(
                "owned candidate reserve requires its original bounded future frontier",
            ));
        }
        let model_project = match project {
            Some(source) => {
                let ledger = ProductionGoalLedger::capture(adapter.snapshot())?;
                ledger.validate_plan(plan)?;
                Some(model::Project::new(plan.digest, source.clone(), ledger))
            }
            None => None,
        };
        let commit_nodes = plan.steps.iter().try_fold(0u64, |total, step| {
            step_nodes(step).map(|nodes| total.saturating_add(nodes))
        })?;
        // Cover prepare/commit revalidation and initial monitor observations
        // before either adapter boundary executes on this private candidate.
        budget.before_predicates(commit_nodes.saturating_mul(2), context)?;
        let prepared = fork.prepare(plan, context)?;
        budget.check(context)?;
        let receipt = fork.commit(plan, &prepared, context)?;
        budget.check(context)?;
        if fork.snapshot().tick != context.anchor.tick {
            return Err(invalid("candidate commit unexpectedly advanced model time"));
        }
        let actions: Vec<_> = receipt
            .actions
            .iter()
            .map(|action| action.action_id)
            .collect();
        if actions.len() != plan.steps.len()
            || actions
                .iter()
                .any(|id| adapter.action_receipt(*id).is_some())
        {
            return Err(invalid(
                "candidate reused a retained original action identity",
            ));
        }
        let mut tracked = self.actions.clone();
        tracked.extend(&actions);
        let mut meter = Meter {
            budget,
            current: context,
            project,
            admitted_stages: project
                .map_or_else(Vec::new, |source| vec![stage_cost(source, plan, 1)]),
            goal_nodes: nodes(
                std::iter::once(&plan.terminal_condition)
                    .chain(plan.steps.iter().flat_map(|step| &step.postconditions)),
            )?
            .saturating_add(plan.steps.len() as u64)
            // Full semantic goal and remaining-reserve checks use the same
            // conservative bound as the original-project query.
            .saturating_add(if project.is_some() {
                super::super::selected_plan_wait::SEMANTIC_GOAL_NODE_RESERVE * 2
            } else {
                0
            }),
            candidate_ticks: receipt
                .actions
                .iter()
                .map(|action| {
                    (
                        action.action_id,
                        action
                            .state
                            .is_terminal()
                            .then_some(action.observed_anchor.tick.0),
                    )
                })
                .collect(),
        };
        meter.inspect_source(fork.snapshot())?;
        // Real commit publication refreshes recovered monitors at this same
        // tick. Preserve that transition before any candidate-complete shortcut.
        let polls = self.carried.iter().filter(|row| !row.is_final()).count() as u64;
        let mut carried = if polls > 0 {
            let modeled = OperationContext {
                anchor: fork.snapshot().anchor(),
                ..context.clone()
            };
            modeled.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
            meter.before_recovered_observation(
                polls,
                model::recovered_predicate_node_bound(&self.carried)?,
            )?;
            recovered_step_progress::observe(fork.snapshot(), &self.carried)?
        } else {
            Vec::new()
        };
        if carried
            .iter()
            .any(|row| row.state == "failed" || row.observation_error.is_some())
        {
            return Err(invalid(
                "candidate commit invalidated an authenticated recovered peer",
            ));
        }
        let finished_at_commit = carried.iter().filter(|row| row.is_final()).count();
        carried.retain(|row| !row.is_final());
        let original = model::Original {
            requested: plan.digest,
            actual: plan.digest,
            actions: actions.clone(),
            reserve_until: reserve,
            reserve_observation: None,
            can_continue: project.is_some(),
        };
        let modeled = OperationContext {
            anchor: fork.snapshot().anchor(),
            ..context.clone()
        };
        let evaluate = |digest: Digest32, snapshot: &WorldSnapshot| {
            if digest != plan.digest {
                return Err(invalid("candidate goal changed during private progression"));
            }
            let truth = crate::lab_world::plan_goal_truth(snapshot, plan)?;
            Ok((truth, truth))
        };
        let prediction = match model_project {
            Some(project) => model::forecast_project_metered(
                fork,
                &[original],
                &tracked,
                &modeled,
                meter.budget.horizon_ticks(),
                started,
                vec![project],
                carried,
                |_, _, _| Ok(()),
                evaluate,
                &mut meter,
            ),
            None => model::forecast_current_stage_metered(
                fork,
                &[original],
                &tracked,
                &modeled,
                meter.budget.horizon_ticks(),
                started,
                carried,
                |_, _, _| Ok(()),
                evaluate,
                &mut meter,
            ),
        };
        let prediction = match prediction {
            Ok(prediction) => prediction,
            Err(error)
                if project.is_some()
                    && !matches!(
                        error.code,
                        dfmcp_core::ErrorCode::BudgetExceeded
                            | dfmcp_core::ErrorCode::CancellationRequested
                    ) =>
            {
                let source =
                    project.ok_or_else(|| invalid("complete candidate source disappeared"))?;
                return self.project_unavailable(
                    &error,
                    source,
                    adapter.snapshot(),
                    meter.admitted_stages,
                );
            }
            Err(error) => return Err(error),
        };
        let row = prediction["originals"]
            .as_array()
            .filter(|rows| rows.len() == 1)
            .and_then(|rows| rows.first())
            .ok_or_else(|| invalid("shared candidate model omitted its exact selected outcome"))?;
        let tick = prediction["predicted_frontier_tick"]
            .as_u64()
            .ok_or_else(|| invalid("shared candidate model omitted its predicted frontier"))?;
        let complete = row["predicted_complete"] == true;
        let actions_complete = row[if project.is_some() {
            "predicted_project_actions_complete"
        } else {
            "predicted_actions_complete"
        }] == true;
        let actions_at = if actions_complete {
            meter
                .candidate_ticks
                .values()
                .copied()
                .collect::<Option<Vec<_>>>()
                .and_then(|ticks| ticks.into_iter().max())
        } else {
            None
        };
        let horizon = plan
            .steps
            .iter()
            .filter_map(|step| step.obligation.as_ref().map(|spec| spec.deadline_tick.0))
            .max()
            .unwrap_or(context.anchor.tick.0);
        let mut value = json!({
            "epistemic_state":"predicted", "available":true,
            "method":"current_candidate_with_real_owned_foreground_progression",
            "progression_scope":"owned_session", "from_tick":context.anchor.tick.0,
            "horizon_tick":horizon, "simulation_horizon_tick":meter.budget.horizon_tick(),
            "simulation_horizon_reached":tick >= meter.budget.horizon_tick(),
            "simulation_horizon_truncated":!complete && tick >= meter.budget.horizon_tick() && tick < horizon,
            "predicted_complete":complete, "predicted_completion_tick":complete.then_some(tick),
            "predicted_actions_complete":actions_complete,
            "predicted_actions_completion_tick":actions_at,
            "predicted_goal_complete":row["predicted_goal_truth"] == "true",
            "predicted_goal_truth":row["predicted_goal_truth"],
            "predicted_physical_quiescent":row["predicted_physical_quiescent"],
            "predicted_frontier_tick":tick,
            "blocked_by_pause":prediction["stop_reason"] == "paused",
            "sampling_policy":"real foreground cadence with exact candidate reserve endpoint and original live/recovered deadlines",
            "competing_work":self.describe(),
            "shared_progression_work":prediction["forecast_work"],
            "recovered_peers_finished_at_candidate_commit":finished_at_commit,
            "recovered_commit_observation_polls":polls,
            "simulation_slices":prediction["forecast_work"]["simulation_slices"],
            "effect_evidence":false, "actual_game_ticks_advanced":0,
        });
        if let Some(until) = reserve {
            let observed = !row["reserve_frontier_truth"].is_null();
            value["reserve_forecast"] = json!({
                "epistemic_state":"predicted", "available":observed,
                "reserve_until_tick":until, "predicted_tick":observed.then_some(until),
                "predicted_goal_truth":row["reserve_frontier_truth"],
                "continuous_maintenance_proven":false, "changes_action_deadlines":false,
                "scope":"original candidate goal at its exact requested future frontier",
            });
        }
        if let Some(source) = project {
            decorate_project(&mut value, row, source, plan, adapter.snapshot())?;
            value["horizon_tick"] = json!(meter.budget.horizon_tick());
            value["simulation_horizon_truncated"] =
                json!(!complete && tick >= meter.budget.horizon_tick());
            value["stop_reason"] = prediction["stop_reason"].clone();
        }
        Ok(value)
    }
}

/// Aggregate only the candidate's own stages; competing work constrains the
/// prediction but cannot become its success, risk or action-count evidence.
fn decorate_project(
    value: &mut Value,
    row: &Value,
    source: &ProjectSource,
    plan: &PreparedPlan,
    snapshot: &WorldSnapshot,
) -> Result<()> {
    let mut initial_steps = row["steps"].clone();
    if let Some(steps) = initial_steps.as_array_mut() {
        for step in steps {
            if let Some(fields) = step.as_object_mut()
                && let Some(id) = fields.remove("action_id")
            {
                fields.insert("model_action_id".to_owned(), id);
            }
        }
    }
    let mut initial = stage_cost(source, plan, 1);
    initial["predicted_actions_complete"] = row["predicted_actions_complete"].clone();
    initial["predicted_physical_quiescent"] = row["predicted_original_physical_quiescent"].clone();
    initial["predicted_stage_complete"] = json!(
        row["predicted_actions_complete"] == true
            && row["predicted_original_physical_quiescent"] == true
    );
    initial["steps"] = initial_steps;
    let mut stages = vec![initial];
    stages.extend(
        row["future_stages"]
            .as_array()
            .ok_or_else(|| invalid("complete candidate model omitted its future stages"))?
            .iter()
            .cloned(),
    );
    decorate_project_stages(value, source, snapshot, stages)
}

fn stage_cost(source: &ProjectSource, plan: &PreparedPlan, index: usize) -> Value {
    json!({
        "index":index, "stage":source.stage_name(plan),
        "model_plan_digest":plan.digest.to_hex(),
        "executable_review":false, "model_actions_are_live_handles":false,
        "from_tick":plan.anchor.tick.0,
        "action_count":plan.steps.len(), "max_risk":plan.max_risk.as_str(),
        "requires_checkpoint":plan.requires_checkpoint,
        "required_capabilities":plan.required_capabilities.iter()
            .map(|capability| capability.as_str()).collect::<Vec<_>>(),
        "obligation_deadlines":plan.steps.iter().filter_map(|step| step.obligation.as_ref()
            .map(|spec| json!({
                "step":step.id.get(), "deadline_tick":spec.deadline_tick.0,
                "poll_interval_ticks":spec.poll_interval_ticks,
                "stable_observations":spec.stable_for_observations,
            }))).collect::<Vec<_>>(),
    })
}

fn decorate_project_stages(
    value: &mut Value,
    source: &ProjectSource,
    snapshot: &WorldSnapshot,
    stages: Vec<Value>,
) -> Result<()> {
    let mut risk = RiskTier::ReadOnly;
    let mut checkpoint = false;
    let mut actions = 0u64;
    let mut capabilities = BTreeSet::new();
    for stage in &stages {
        let stage_risk = match stage["max_risk"].as_str() {
            Some("read_only") => RiskTier::ReadOnly,
            Some("reversible") => RiskTier::Reversible,
            Some("guarded") => RiskTier::Guarded,
            Some("irreversible") => RiskTier::Irreversible,
            _ => return Err(invalid("complete candidate stage omitted its risk")),
        };
        risk = risk.max(stage_risk);
        checkpoint |= stage["requires_checkpoint"].as_bool().ok_or_else(|| {
            invalid("complete candidate stage omitted its checkpoint requirement")
        })?;
        actions = actions.saturating_add(
            stage["action_count"]
                .as_u64()
                .ok_or_else(|| invalid("complete candidate stage omitted its action count"))?,
        );
        for capability in stage["required_capabilities"]
            .as_array()
            .ok_or_else(|| invalid("complete candidate stage omitted its capabilities"))?
        {
            capabilities.insert(
                capability
                    .as_str()
                    .ok_or_else(|| invalid("complete candidate stage has an invalid capability"))?
                    .to_owned(),
            );
        }
    }
    value["schema"] = json!("dfmcp.project-forecast/2");
    value["method"] = json!("complete_candidate_project_with_real_owned_foreground_progression");
    value["forecast_scope"] = json!("complete_project");
    value["project_source_digest"] = json!(source.source_digest().to_hex());
    value["semantic_goal"] = source.semantic_key(snapshot)?;
    value["project_max_risk"] = json!(risk.as_str());
    value["project_requires_checkpoint"] = json!(checkpoint);
    value["project_action_count"] = json!(actions);
    value["project_required_capabilities"] = json!(capabilities);
    value["stages"] = json!(stages);
    value["requires_review_between_stages"] = json!(true);
    value["future_stage_plans_are_executable"] = json!(false);
    Ok(())
}

struct Meter<'a> {
    budget: &'a mut ForecastBudget,
    current: &'a OperationContext,
    project: Option<&'a ProjectSource>,
    admitted_stages: Vec<Value>,
    goal_nodes: u64,
    candidate_ticks: BTreeMap<ActionId, Option<u64>>,
}

impl ForecastMeter for Meter<'_> {
    fn before_fork(&mut self, snapshot: &WorldSnapshot) -> Result<()> {
        self.budget.before_fork(snapshot, self.current)
    }
    fn inspect_source(&mut self, snapshot: &WorldSnapshot) -> Result<()> {
        self.budget.inspect_owned_source(snapshot, self.current)
    }
    fn before_goal_evaluation(&mut self) -> Result<()> {
        self.budget.before_predicates(self.goal_nodes, self.current)
    }
    fn before_round(&mut self, ticks: u64) -> Result<()> {
        self.budget.before_advance(ticks, self.current)
    }
    fn advanced(&mut self, ticks: u64) {
        self.budget.advanced(ticks);
    }
    fn before_action_poll(&mut self, step: &PlanStep) -> Result<()> {
        self.budget
            .before_predicates(step_nodes(step)?, self.current)?;
        self.budget.before_poll(self.current)
    }
    fn after_action_poll(&mut self, receipt: &ActionReceipt) {
        if receipt.state.is_terminal()
            && let Some(tick) = self.candidate_ticks.get_mut(&receipt.action_id)
        {
            *tick = Some(receipt.observed_anchor.tick.0);
        }
    }
    fn before_project_stage(
        &mut self,
        snapshot: &WorldSnapshot,
        plan: &PreparedPlan,
        archive_bytes: u64,
    ) -> Result<()> {
        self.budget
            .admit_owned_project_stage(snapshot, plan, archive_bytes, self.current)?;
        let nodes = plan.steps.iter().try_fold(0u64, |total, step| {
            step_nodes(step).map(|nodes| total.saturating_add(nodes))
        })?;
        self.budget
            .before_predicates(nodes.saturating_mul(2), self.current)?;
        self.goal_nodes = self.goal_nodes.saturating_add(nodes);
        let source = self
            .project
            .ok_or_else(|| invalid("candidate successor lacks its project source"))?;
        self.admitted_stages
            .push(stage_cost(source, plan, self.admitted_stages.len() + 1));
        Ok(())
    }
    fn after_project_commit(&mut self, actions: &[ActionReceipt]) {
        for action in actions {
            self.candidate_ticks.insert(
                action.action_id,
                action
                    .state
                    .is_terminal()
                    .then_some(action.observed_anchor.tick.0),
            );
        }
    }
    fn after_project_observation(&mut self, carried: &[CarriedStep]) -> Result<()> {
        if carried
            .iter()
            .any(|row| row.state == "failed" || row.observation_error.is_some())
        {
            return Err(invalid(
                "candidate successor encountered a failed authenticated recovered peer",
            ));
        }
        Ok(())
    }
    fn before_recovered_observation(&mut self, polls: u64, nodes: u64) -> Result<()> {
        self.budget.before_predicates(nodes, self.current)?;
        for _ in 0..polls {
            self.budget.before_poll(self.current)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "owned_candidate_forecast_tests.rs"]
mod tests;
