//! Current-owner advice for the next separately reviewed bounded clock interval.
//!
//! This projection performs no native call, journal write, preparation or grant
//! issuance. Plan still refreshes the source and checks its complete original
//! custody. A historical run never restores a process-local confirmation.
use super::*;
use dfmcp_adapter::semantic_work_orders::ObservedInventoryGoal;
use dfmcp_core::StepId;
use dfmcp_intent::ObligationStatus;
use dfmcp_world::PredicateTruth;
use serde::{Deserialize, Serialize};

const SCHEMA: &str = "dfmcp.production-clock-guidance/1";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NextAction {
    Wait,
    Inspect,
    Operator,
    None,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Guidance {
    schema: String,
    authority_free: bool,
    pub(crate) anchor: Value,
    pub(crate) original_plan_digest: Option<String>,
    pub(crate) selected_order_step_id: Option<u32>,
    max_game_ticks: u64,
    max_wall_millis: u64,
    pub(crate) offers: Vec<Offer>,
    pub(crate) blockers: Vec<Blocker>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Offer {
    pub(crate) scope: String,
    pub(crate) order_step_id: u32,
    pub(crate) plan_request: Value,
    pub(crate) state: String,
    pub(crate) reason: String,
    pub(crate) evidence: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Blocker {
    pub(crate) scope: String,
    pub(crate) code: String,
    pub(crate) message: String,
    pub(crate) next_action: NextAction,
    pub(crate) order_step_id: Option<u32>,
    pub(crate) evidence: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PlanRequest {
    kind: String,
    original_paired_seal: String,
    game_ticks: u32,
    wall_millis: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    order_step_id: Option<u32>,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Prepared {
    pub(super) before_tick: u64,
    pub(super) game_ticks: u32,
    pub(super) wall_millis: u32,
}

#[derive(Clone, Debug)]
pub(super) struct Owner {
    pub(super) inspected: bool,
    pub(super) target_bound: bool,
    pub(super) original_paired_seal: Option<Digest32>,
    pub(super) pending: bool,
    pub(super) unresolved: bool,
    pub(super) run_count: usize,
    pub(super) run_limit: usize,
    pub(super) prepared: Option<Prepared>,
}

fn blocker(
    scope: &str,
    code: &str,
    message: &str,
    next_action: NextAction,
    step: Option<u32>,
    evidence: &[&str],
) -> Blocker {
    Blocker {
        scope: scope.into(),
        code: code.into(),
        message: message.into(),
        next_action,
        order_step_id: step,
        evidence: evidence.iter().map(|pointer| (*pointer).into()).collect(),
    }
}

/// The existing native specifications admit an inclusive immutable end tick.
/// Never shorten a retained preparation: only its exact interval can be reviewed.
fn interval(
    owner: &Owner,
    tick: u64,
    deadline: u64,
    cadence: u64,
    stability: bool,
    budget: WorkBudget,
) -> std::result::Result<(u32, u32, &'static str), (&'static str, &'static str, NextAction)> {
    if !owner.inspected || owner.unresolved {
        return Err(("clock_reconciliation_required",
            "Reconcile the retained original clock before considering another interval.",
            NextAction::Wait));
    }
    if !owner.target_bound || owner.original_paired_seal.is_none() {
        return Err(("original_clock_target_unavailable",
            "Inspect the exact original Created custody before selecting a bounded clock.",
            NextAction::Inspect));
    }
    if budget.max_game_ticks == 0 {
        return Err(("game_tick_allowance_zero",
            "This session has no game-tick allowance. Manage game time separately or reopen the exact retained owner with an explicit bounded allowance.",
            NextAction::Operator));
    }
    if budget.max_wall_millis == 0 {
        return Err(("clock_wall_allowance_zero",
            "No wall-time allowance remains for a reviewed clock interval.",
            NextAction::Operator));
    }
    let remaining = deadline.saturating_sub(tick);
    if remaining == 0 {
        return Err(("original_deadline_reached",
            "The unchanged original deadline leaves no time for a new clock interval.",
            NextAction::Operator));
    }
    let ticks = remaining.min(budget.max_game_ticks).min(1200);
    let wall = budget.max_wall_millis.min(60_000);
    if let Some(prepared) = owner.prepared {
        if prepared.before_tick != tick
            || prepared.game_ticks == 0
            || u64::from(prepared.game_ticks) > ticks
            || prepared.wall_millis == 0
            || u64::from(prepared.wall_millis) > wall
        {
            return Err(("retained_preparation_requires_inspection",
                "The retained interval cannot be changed or reviewed at this tick or allowance. Inspect it and explicitly cancel if appropriate.",
                NextAction::Inspect));
        }
        return Ok((prepared.game_ticks, prepared.wall_millis, "review_prepared"));
    }
    if owner.pending {
        return Err(("clock_reconciliation_required",
            "Settle the retained original clock before preparing another interval.",
            NextAction::Wait));
    }
    if owner.run_count >= owner.run_limit {
        return Err(("clock_history_limit",
            "This original clock has reached its finite retained-run limit.",
            NextAction::Operator));
    }
    if cadence == 0 {
        return Err(("original_timing_unavailable",
            "The selected original has no valid retained monitoring cadence.",
            NextAction::Inspect));
    }
    let ticks = if stability { ticks.min(cadence) } else { ticks };
    Ok((ticks as u32, wall.min(5000) as u32, "new_interval"))
}

impl Runtime {
    pub(super) fn clock_guidance_json(
        &self,
        frontier: Option<&durable::View>,
        c: &OperationContext,
    ) -> Value {
        let step = self.selected_order_step().map(StepId::get);
        let mut guide = Guidance {
            schema: SCHEMA.into(),
            authority_free: true,
            anchor: anchor_json(self.anchor),
            original_plan_digest: self.original.as_ref().map(|plan| plan.digest.to_hex()),
            selected_order_step_id: step,
            max_game_ticks: self.budget.max_game_ticks.min(c.budget.max_game_ticks),
            // This is a recommendation for the next foreground call. The
            // current publication reservation must not shrink a retained run's
            // immutable wall interval or renew this call's Work deadline.
            max_wall_millis: self.budget.max_wall_millis,
            offers: Vec::new(),
            blockers: Vec::new(),
        };
        let mut current = c.clone();
        current.anchor = self.anchor;
        current.budget.max_game_ticks = guide.max_game_ticks;
        current.budget.max_wall_millis = guide.max_wall_millis;
        if let Some(reason) = self.clock_guidance_global(frontier, &current) {
            guide.blockers.push(reason);
            return json!(guide);
        }
        let Some(original) = self.original.as_ref() else { return json!(guide) };
        let Some(selected) = self.selected_order_step() else { return json!(guide) };
        let Some(obligation) = original.steps.iter()
            .find(|candidate| candidate.id == selected)
            .and_then(|candidate| candidate.obligation.as_ref())
        else { return json!(guide) };
        let Some(sample) = self.goal_monitor.as_ref().and_then(|monitor| monitor.latest())
        else { return json!(guide) };
        let inventory = ObservedInventoryGoal::from_selected_plan(original, selected).is_some();
        let inventory_preferred = inventory && sample.current_goal == PredicateTruth::True;
        let order_owner = self.clock.as_ref().map(ProductionClock::guidance_owner);
        let inventory_owner = self.inventory_clock.as_ref()
            .map(ProductionInventoryClock::guidance_owner);
        // Prefer the configured owner that can finish True inventory stability
        // after normal native retirement. Otherwise retain the original order.
        let families = if inventory_preferred && inventory_owner.is_some()
            || order_owner.is_none() && inventory_owner.is_some()
        {
            ["inventory_run", "run"]
        } else {
            ["run", "inventory_run"]
        };
        for scope in families {
            let (owner, sibling_pending, enabled, field, ready) = if scope == "run" {
                (order_owner.as_ref(),
                    inventory_owner.as_ref().is_some_and(|owner| owner.pending),
                    self.clock_control_enabled(), "clock",
                    self.clock_goal_ready().is_ok())
            } else {
                (inventory_owner.as_ref(),
                    order_owner.as_ref().is_some_and(|owner| owner.pending),
                    self.inventory_clock_control_enabled(), "inventory_clock",
                    inventory && self.inventory_clock_goal_ready().is_ok())
            };
            let evidence = vec![format!("/{field}"), "/original_plan".into(),
                if inventory { "/selected_inventory_goal".into() } else { "/selected_order_goal".into() },
                if self.portfolio.is_some() { "/portfolio".into() } else { "/current_parent_goal".into() }];
            let reason = if sibling_pending {
                Some(("other_clock_pending", "Inspect and settle the other retained clock before preparing this owner.",
                    NextAction::Inspect))
            } else if owner.is_none() {
                Some(("clock_not_configured", "This clock is not configured. Paused goal observations do not advance game time.",
                    NextAction::Operator))
            } else if owner.is_some_and(|owner| !owner.inspected || owner.unresolved) {
                Some(("clock_reconciliation_required", "Reconcile this retained clock owner before considering another interval.",
                    NextAction::Wait))
            } else if owner.is_some_and(|owner| !owner.target_bound || owner.original_paired_seal.is_none()) {
                Some(("original_clock_target_unavailable", "Select and inspect this exact original order to bind its verified Created custody.",
                    NextAction::Inspect))
            } else if !enabled {
                Some(("independent_clock_authority_unavailable", "The current owner lacks its separate operator clock authority.",
                    NextAction::Operator))
            } else if scope == "inventory_run" && !inventory {
                Some(("inventory_objective_required", "An inventory clock requires an explicit original inventory objective.",
                    NextAction::Operator))
            } else if scope == "inventory_run" && sample.current_goal != PredicateTruth::True {
                Some(("inventory_goal_not_yet_true", "An inventory clock only advances stability after the original stock goal is observed True. Manage production time separately or use an eligible original order clock.",
                    NextAction::Operator))
            } else if scope == "run" && !sample.receipt_linked {
                Some(("original_order_not_receipt_linked", "The current sample cannot link the original native order. Its disappearance does not prove completion.",
                    NextAction::Inspect))
            } else if !ready {
                Some(("current_goal_not_clock_ready", "Inspect the unmet original requirements. Inventory advancement requires qualified True stock and labor with unfinished original stability.",
                    NextAction::Inspect))
            } else { None };
            if let Some((code, message, next)) = reason {
                let mut blocked = blocker(scope, code, message, next, step, &[]);
                blocked.evidence = evidence;
                guide.blockers.push(blocked);
                continue;
            }
            let Some(owner) = owner else { continue };
            let interval = interval(owner, self.anchor.tick.get(),
                obligation.deadline_tick.get().min(original.expires_at_tick.get()),
                obligation.poll_interval_ticks, sample.current_goal == PredicateTruth::True,
                current.budget);
            let (game_ticks, wall_millis, state) = match interval {
                Ok(interval) => interval,
                Err((code, message, next)) => {
                    let mut blocked = blocker(scope, code, message, next, step, &[]);
                    blocked.evidence = evidence;
                    guide.blockers.push(blocked);
                    continue;
                }
            };
            let mut horizon = current.clone();
            horizon.anchor.tick = GameTick(self.anchor.tick.get() + u64::from(game_ticks));
            let canonical_authorized = horizon.authorize(
                Capability::ControlClock, RiskTier::Guarded, &[], None).is_ok();
            let native_authorized = scope != "inventory_run"
                || self.config.orders.inventory_clock_context(&horizon, true)
                    .is_ok_and(|native| native.authorize(
                        Capability::ControlClock, RiskTier::Guarded, &[], None).is_ok());
            if !canonical_authorized || !native_authorized {
                let mut blocked = blocker(scope, "clock_horizon_not_authorized",
                    "Current independent clock authority does not cover the complete proposed interval.",
                    NextAction::Operator, step, &[]);
                blocked.evidence = evidence;
                guide.blockers.push(blocked);
                continue;
            }
            let Some(original_seal) = owner.original_paired_seal else { continue };
            let request = PlanRequest {
                kind: scope.into(),
                original_paired_seal: original_seal.to_hex(),
                game_ticks,
                wall_millis,
                order_step_id: self.portfolio.as_ref().map(|_| selected.get()),
            };
            guide.offers.push(Offer {
                scope: scope.into(),
                order_step_id: selected.get(),
                plan_request: json!(request),
                state: state.into(),
                reason: if state == "review_prepared" {
                    "Freshly review the exact retained interval; its historical seal is not confirmation authority."
                } else if sample.current_goal == PredicateTruth::True {
                    "Prepare one bounded interval for the original remaining stability cadence, then review and confirm it separately."
                } else {
                    "Prepare one bounded interval toward the original unfinished order goal, then review and confirm it separately."
                }.into(),
                evidence,
            });
        }
        json!(guide)
    }

    fn clock_guidance_global(
        &self,
        frontier: Option<&durable::View>,
        c: &OperationContext,
    ) -> Option<Blocker> {
        let step = self.selected_order_step().map(StepId::get);
        let block = |code, message, next, evidence: &[&str]| {
            Some(blocker("all", code, message, next, step, evidence))
        };
        if self.mode != Mode::Control {
            return block("control_owner_required",
                "A read or recovery owner cannot prepare a new clock interval.",
                NextAction::Operator, &["/mode"]);
        }
        if self.has_active_stock_watch() {
            return block("no_work_clock_target_absent",
                "This read-only no-work goal has no original Created clock target. Advance game time separately before another cadence sample.",
                NextAction::Operator, &["/no_work_stock_goal"]);
        }
        if self.original.is_none() {
            return block("original_order_absent", "No original production order has been published.",
                NextAction::None, &["/original_plan"]);
        }
        if self.initialization_failed || self.fenced || self.parent_goal_observation_fenced()
            || self.goal_monitor.as_ref().is_some_and(|monitor| monitor.is_fenced())
        {
            return block("original_source_or_custody_fenced",
                "Inspect the retained original source or custody failure; repeated Wait cannot clear its fence.",
                NextAction::Operator, &["/parent_goal_observation_fenced", "/recovery"]);
        }
        if self.parent_goal_proven() {
            return block("original_goal_proven", "The complete original goal is proved at this anchor.",
                NextAction::None, &["/current_parent_goal"]);
        }
        if frontier.is_none_or(|view| !view.all_applied() || view.needs_reconciliation()) {
            return block("original_effects_unsettled",
                "Finish or reconcile the exact original effects before considering game-time advancement.",
                NextAction::Wait, &["/frontier"]);
        }
        if let Some(pending) = self.pending_portfolio_steps().into_iter()
            .find(|pending| Some(pending.get()) != step)
        {
            return Some(blocker("all", "inactive_clock_pending",
                "Select the original portfolio leaf that owns retained clock work before considering another interval.",
                NextAction::Inspect, Some(pending.get()), &["/portfolio"]));
        }
        if self.clock.as_ref().is_some_and(ProductionClock::may_be_running)
            || self.inventory_clock.as_ref().is_some_and(ProductionInventoryClock::may_be_running)
        {
            return block("clock_reconciliation_required",
                "Reconcile the retained original clock key before acquiring another paused goal sample.",
                NextAction::Wait, &["/clock", "/inventory_clock"]);
        }
        if self.authority_tick_floor() > self.anchor.tick || self.clock_withholds_goal() {
            return block("fresh_goal_sample_required",
                "Acquire a fresh joined goal sample at or above every consumed source tick.",
                NextAction::Wait, &["/observation_tick_floor", "/anchor"]);
        }
        let Some(monitor) = self.goal_monitor.as_ref() else {
            return block("fresh_goal_sample_required",
                "Wait once to initialize or replay the original goal monitor, then acquire a current sample.",
                NextAction::Wait, &["/selected_inventory_goal", "/selected_order_goal"]);
        };
        if matches!(monitor.historical_obligation_status(),
            Some(ObligationStatus::Failed { .. } | ObligationStatus::Cancelled { .. }))
        {
            return block("original_obligation_terminal",
                "The unchanged original obligation is failed or cancelled.",
                NextAction::Operator, &["/selected_inventory_goal", "/selected_order_goal"]);
        }
        let Some(sample) = monitor.latest().filter(|sample| {
            sample.anchor.tick == self.anchor.tick
                && sample.anchor.fortress_id == self.anchor.fortress_id
                && sample.anchor.cursor.epoch == self.anchor.cursor.epoch
                && (self.portfolio.is_some() || sample.anchor.cursor == self.anchor.cursor)
        }) else {
            return block("fresh_goal_sample_required",
                "Historical goal custody supplies no current paused source sample. Acquire a fresh original-goal observation.",
                NextAction::Wait, &["/selected_inventory_goal", "/selected_order_goal"]);
        };
        let selected = self.parent_goal.as_ref().filter(|parent| {
            parent.anchor == self.anchor && parent.selected_anchor == sample.anchor
        });
        let complete = self.portfolio.as_ref().and_then(|portfolio| portfolio.current.as_ref())
            .filter(|parent| parent.anchor == self.anchor
                && parent.selected_anchors.contains(&(monitor.step_id(), sample.anchor)));
        let parent_truth = selected.map(|parent| parent.current_truth)
            .or_else(|| complete.map(|parent| parent.current_truth));
        let labor = selected.map(|parent| &parent.labor)
            .or_else(|| complete.map(|parent| &parent.labor));
        let Some(labor) = labor else {
            return block("fresh_joined_parent_required",
                "Acquire a fresh original labor and selected-goal bracket before choosing a clock.",
                NextAction::Wait, &["/selected_parent_goal", "/current_parent_goal"]);
        };
        if labor.is_empty() || labor.iter().any(|finding| finding.current_truth != PredicateTruth::True) {
            return block("current_labor_unproved",
                "Current selected labor is false or unknown. Inspect the observed labor findings before advancing time.",
                NextAction::Inspect, &["/selected_parent_goal", "/current_parent_goal"]);
        }
        if parent_truth == Some(PredicateTruth::Unknown) {
            return block("fresh_joined_parent_required",
                "The current parent truth is Unknown. Inspect its evidence or acquire the remaining original portfolio samples.",
                if self.portfolio.is_some() { NextAction::Wait } else { NextAction::Inspect },
                &["/selected_parent_goal", "/current_parent_goal"]);
        }
        if sample.current_goal == PredicateTruth::Unknown {
            return block("selected_goal_unknown",
                "The current original goal is Unknown. Inspect its source coverage; absence is not completion.",
                NextAction::Inspect, &["/selected_inventory_goal", "/selected_order_goal"]);
        }
        if sample.current_goal_proven() && !self.parent_stability_needs_time() {
            return block("selected_goal_already_proven",
                "This selected original already fulfilled its timing requirement. Inspect or sample the remaining original portfolio.",
                if self.portfolio.is_some() { NextAction::Wait } else { NextAction::Inspect },
                &["/selected_inventory_goal", "/selected_order_goal", "/current_parent_goal"]);
        }
        for (capability, risk) in [(Capability::Query, RiskTier::ReadOnly),
            (Capability::Observe, RiskTier::ReadOnly), (Capability::Plan, RiskTier::Guarded)]
        {
            if c.authorize(capability, risk, &[], None).is_err() {
                return block("current_planning_authority_unavailable",
                    "Current read and planning authority is required before proposing another interval.",
                    NextAction::Operator, &["/mode"]);
            }
        }
        None
    }
}

fn canonical_digest(value: &Value) -> bool {
    value.as_str().is_some_and(|text| {
        text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            && text.bytes().any(|byte| byte != b'0')
    })
}

fn anchor_shape(value: &Value) -> bool {
    value.as_object().is_some_and(|object| object.len() == 5
        && value["fortress_id"].as_str().is_some_and(|text| {
            text.parse::<u128>().is_ok_and(|id| id != 0 && id.to_string() == text)
        })
        && ["epoch", "sequence", "tick"].iter().all(|field| value[*field].as_u64().is_some())
        && value["state_hash"].as_str().is_some_and(|text| {
            text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        }))
}

fn same_tick_lineage(a: &Value, b: &Value) -> bool {
    anchor_shape(a) && anchor_shape(b) && a["fortress_id"] == b["fortress_id"]
        && a["epoch"] == b["epoch"] && a["tick"] == b["tick"]
}

fn clock_settled(clock: &Value) -> bool {
    clock.is_null() || (clock.is_object()
        && clock["inspection_required"] == false
        && clock["pending"] == false && clock["unresolved"] == false
        && clock["may_be_running"] == false && clock["needs_reconciliation"] == false
        && clock["active_run"].is_null())
}

fn current_labor(parent: &Value) -> bool {
    canonical_digest(&parent["source_digest"])
        && canonical_digest(&parent["workforce_source_digest"])
        && parent["restored_from_history"] == false
        && parent["labor"].as_array().is_some_and(|labor| !labor.is_empty()
            && labor.len() <= 32 && labor.iter().all(|finding| {
                finding["current_truth"] == "true"
                    && finding["observed_enabled"].as_bool().is_some()
                    && finding["observed_enabled"] == finding["requested_enabled"]
            }))
}

fn evidence_valid(view: &Value, evidence: &[String]) -> bool {
    !evidence.is_empty() && evidence.len() <= 8 && evidence.iter().all(|pointer| {
        pointer.len() <= 128 && pointer.starts_with('/')
            && !pointer.starts_with("/recovery/")
            && view.pointer(pointer).is_some()
    })
}

/// One shared closed decoder for every presentation surface. A blocked guide can
/// explain a consumed newer tick or missing current sample. Offers additionally
/// require exact current original/selected/clock/source bindings. No recovery
/// object is searched, and callers must retain their successful-owner guards.
pub(crate) fn from_view(view: &Value) -> Option<Guidance> {
    if view["ok"] != true || !view["clock_guidance"].is_object() {
        return None;
    }
    let guide: Guidance = serde_json::from_value(view["clock_guidance"].clone()).ok()?;
    if guide.schema != SCHEMA || !guide.authority_free
        || !anchor_shape(&guide.anchor) || guide.anchor != view["anchor"]
        || guide.max_game_ticks > 1200 || guide.max_wall_millis > 60_000
        || guide.offers.len() > 2 || guide.blockers.len() > 4
        || guide.offers.is_empty() && guide.blockers.is_empty()
    {
        return None;
    }
    let original = &view["original_plan"];
    if guide.original_plan_digest.as_deref() != original["plan_digest"].as_str()
        || original.is_object() && !canonical_digest(&original["plan_digest"])
    {
        return None;
    }
    let ids = match original["steps"].as_array() {
        Some(steps) => steps.iter()
            .filter(|step| step["action"]["kind"] == "create_work_order")
            .map(|step| step["step_id"].as_u64().and_then(|id| u32::try_from(id).ok()))
            .collect::<Option<Vec<_>>>()?,
        None if original.is_null() => Vec::new(),
        None => return None,
    };
    let portfolio = view["portfolio"].is_object();
    if original.is_object() && (ids.is_empty() || ids.len() > 4
        || ids.windows(2).any(|pair| pair[0] >= pair[1]))
    {
        return None;
    }
    let selected = if portfolio {
        if view["portfolio"]["schema"] != "dfmcp.production-portfolio/1" {
            return None;
        }
        let rows = view["portfolio"]["orders"].as_array()?;
        let recovery = view["recovery"]["portfolio"].as_array()?;
        if rows.len() != ids.len() || recovery.len() != ids.len()
            || rows.iter().zip(&ids).any(|(row, id)| row["step_id"] != *id)
            || recovery.iter().zip(&ids).any(|(row, id)| row["step_id"] != *id)
        {
            return None;
        }
        view["portfolio"]["selected_order_step_id"].as_u64()
            .and_then(|id| u32::try_from(id).ok())
    } else {
        (ids.len() == 1).then(|| ids[0])
    };
    if guide.selected_order_step_id != selected
        || selected.is_some_and(|selected| !ids.contains(&selected))
    {
        return None;
    }
    for blocker in &guide.blockers {
        if !matches!(blocker.scope.as_str(), "all" | "run" | "inventory_run")
            || blocker.code.is_empty() || blocker.code.len() > 64
            || blocker.message.is_empty() || blocker.message.len() > 512
            || blocker.order_step_id.is_some_and(|id| !ids.contains(&id))
            || !evidence_valid(view, &blocker.evidence)
        {
            return None;
        }
    }
    if guide.offers.is_empty() {
        return Some(guide);
    }
    let selected = selected?;
    let tick = guide.anchor["tick"].as_u64()?;
    if view["mode"] != "control" || view["original_goal_proven"] != false
        || view["custody_initialization_may_be_partial"] != false
        || view["parent_goal_observation_fenced"] != false
        || view["frontier"]["all_applied"] != true
        || view["frontier"]["needs_reconciliation"] != false
        || view["clock_invalidates_goal"] != false
        || view["inventory_clock_invalidates_goal"] != false
        || view["observation_tick_floor"].as_u64()? > tick
        || view["no_work_stock_goal"].is_object()
        || !canonical_digest(&guide.anchor["state_hash"])
    {
        return None;
    }
    let inventory = view["selected_inventory_goal"].is_object();
    if inventory == view["selected_order_goal"].is_object() {
        return None;
    }
    let goal = if inventory { &view["selected_inventory_goal"] } else { &view["selected_order_goal"] };
    if goal["scope"] != "selected_original_order_step" || goal["step_id"] != selected
        || if inventory {
            !matches!(goal["kind"].as_str(), Some("observed_inventory_minimum" | "observed_inventory_increase"))
        } else { goal["kind"] != "retained_order_completion" }
        || goal["terminal_failure"] != false || goal["history"]["fenced"] != false
        || !matches!(goal["current_truth"].as_str(), Some("true" | "false"))
        || !same_tick_lineage(&goal["anchor"], &guide.anchor)
        || !canonical_digest(&goal["anchor"]["state_hash"])
        || !canonical_digest(&goal["source_digest"])
        || !portfolio && goal["anchor"]["sequence"] != guide.anchor["sequence"]
    {
        return None;
    }
    if portfolio && view["portfolio"]["orders"].as_array()?.iter()
        .find(|row| row["step_id"] == selected)?["goal"] != *goal
    {
        return None;
    }
    let step = original["steps"].as_array()?.iter()
        .find(|step| step["step_id"] == selected)?;
    let deadline = step["obligation"]["deadline_tick"].as_u64()?
        .min(original["expires_at_tick"].as_u64()?);
    let cadence = step["obligation"]["poll_interval_ticks"].as_u64()?;
    if cadence == 0 || goal["deadline_tick"] != step["obligation"]["deadline_tick"]
        || goal["poll_interval_ticks"] != cadence || tick >= deadline
    {
        return None;
    }
    let selected_parent = if portfolio { &view["selected_parent_goal"] } else { &view["current_parent_goal"] };
    let selected_joined = selected_parent["anchor"] == guide.anchor
        && selected_parent["selected_anchor"] == goal["anchor"] && current_labor(selected_parent)
        && selected_parent["scope"] == if portfolio { "selected_original_labor_and_production_goal" }
            else if inventory { "complete_original_labor_and_inventory_goal" }
            else { "complete_original_labor_and_order_goal" };
    let complete = &view["current_parent_goal"];
    let cohort_joined = portfolio && complete["scope"] == "complete_original_production_portfolio"
        && complete["anchor"] == guide.anchor && current_labor(complete)
        && complete["selected_anchors"].as_array().is_some_and(|anchors| {
            anchors.len() == ids.len() && anchors.iter().zip(&ids).all(|(row, id)| {
                row["step_id"] == *id && same_tick_lineage(&row["anchor"], &guide.anchor)
            }) && anchors.iter().any(|row| row["step_id"] == selected && row["anchor"] == goal["anchor"])
        });
    if !selected_joined && !cohort_joined {
        return None;
    }
    let parent = if selected_joined { selected_parent } else { complete };
    if !matches!(parent["current_truth"].as_str(), Some("true" | "false"))
        || parent["current_truth"] == "true" && goal["current_truth"] != "true"
        || !portfolio && parent["current_truth"] != goal["current_truth"]
    {
        return None;
    }
    let parent_stability = portfolio && parent["current_truth"] == "true"
        && (selected_joined && selected_parent["parent_stability_pending"] == true
            || cohort_joined && complete["parent_stability"]["status"] == "pending")
        && parent["inventory_requirements"].as_array().is_some_and(|requirements| {
            !requirements.is_empty() && requirements.len() <= 4
                && requirements.iter().all(|requirement| requirement["current_truth"] == "true")
        });
    if goal["proven_at_anchor"] == true && !parent_stability {
        return None;
    }
    let mut scopes = std::collections::BTreeSet::new();
    for offer in &guide.offers {
        let request: PlanRequest = serde_json::from_value(offer.plan_request.clone()).ok()?;
        let (field, source, target_scope, max_runs) = match offer.scope.as_str() {
            "run" => ("clock", "order-run/1.14", "selected_original_order_step", orders::clock::MAX_RUNS),
            "inventory_run" => ("inventory_clock", "run/1.13", "selected_original_inventory_goal", orders::inventory_clock::MAX_RUNS),
            _ => return None,
        };
        if !scopes.insert(offer.scope.as_str()) || offer.order_step_id != selected
            || request.kind != offer.scope
            || request.order_step_id != portfolio.then_some(selected)
            || offer.plan_request.as_object()?.len() != if portfolio { 5 } else { 4 }
            || offer.reason.is_empty() || offer.reason.len() > 512
            || !evidence_valid(view, &offer.evidence)
            || request.game_ticks == 0 || u64::from(request.game_ticks) > guide.max_game_ticks
            || request.game_ticks > 1200 || u64::from(request.game_ticks) > deadline - tick
            || request.wall_millis == 0 || u64::from(request.wall_millis) > guide.max_wall_millis
            || request.wall_millis > 60_000
            || view[format!("{field}_control_enabled")] != true
            || offer.scope == "run" && goal["receipt_linked"] != true
            || offer.scope == "inventory_run" && (!inventory || goal["current_truth"] != "true"
                || parent["current_truth"] != "true")
        {
            return None;
        }
        let clock = &view[field];
        if clock["source"] != source || clock["target_scope"] != target_scope
            || clock["mode"] != "control" || clock["inspection_required"] != false
            || clock["unresolved"] != false || clock["may_be_running"] != false
            || clock["high_tick"].as_u64()? > tick
            || !canonical_digest(&clock["journal_id"]) || !canonical_digest(&clock["head"])
            || !canonical_digest(&clock["target_digest"])
            || !canonical_digest(&clock["original_paired_seal"])
            || clock["original_paired_seal"] != request.original_paired_seal
            || clock["run_count"].as_u64()? != clock["runs"].as_array()?.len() as u64
        {
            return None;
        }
        let count = clock["runs"].as_array()?.len();
        let active = &clock["active_run"];
        match offer.state.as_str() {
            "new_interval" if clock_settled(clock) && count < max_runs
                && (goal["current_truth"] != "true" || u64::from(request.game_ticks) <= cadence) => {}
            "review_prepared" if count <= max_runs && count > 0
                && clock["pending"] == true && active["state"] == "prepared"
                && active["unresolved"] == false && active["before_tick"] == tick
                && active["game_ticks"] == request.game_ticks
                && active["wall_millis"] == request.wall_millis
                && active["native"]["phase"] == "prepared"
                && active["native"]["unpause_attempted"] == false
                && clock["runs"].as_array()?.iter().filter(|run| *run == active).count() == 1 => {}
            _ => return None,
        }
        let rows = if portfolio { view["portfolio"]["orders"].as_array()?.clone() }
            else { vec![json!({"step_id":selected,"clock":view["clock"],"inventory_clock":view["inventory_clock"]})] };
        let mut journal_ids = std::collections::BTreeSet::new();
        for row in rows {
            for other_field in ["clock", "inventory_clock"] {
                let other = &row[other_field];
                let is_selected = row["step_id"] == selected && other_field == field;
                if row["step_id"] == selected && other != &view[other_field] {
                    return None;
                }
                if !is_selected && !clock_settled(other) {
                    return None;
                }
                if other.is_null() { continue; }
                if other["high_tick"].as_u64()? > tick || !canonical_digest(&other["journal_id"])
                    || !journal_ids.insert(other["journal_id"].as_str()?.to_owned())
                {
                    return None;
                }
                let retained = if portfolio {
                    view["recovery"]["portfolio"].as_array()?.iter()
                        .find(|entry| entry["step_id"] == row["step_id"])?
                } else { &view["recovery"] };
                if retained[format!("{other_field}_journal_id")] != other["journal_id"] {
                    return None;
                }
            }
        }
    }
    Some(guide)
}

#[cfg(test)]
#[path = "clock_guidance_tests.rs"]
pub(crate) mod tests;
