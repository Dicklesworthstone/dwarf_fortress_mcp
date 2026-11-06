//! Foreground proof of the complete original labor/production goal. Current
//! labor brackets the selected order or inventory sample; historical receipts only
//! establish custody and never become current labor facts.
use super::*;
use dfmcp_adapter::live_routing::LiveRoutingEvidence;
use dfmcp_adapter::semantic_work_orders::{
    ObservedInventoryObjective, RetainedOrderObjective, WorkOrderGoalProgress,
};
use dfmcp_core::StepId;
use dfmcp_intent::ObligationStatus;
use dfmcp_world::PredicateTruth;

enum SelectedObjective {
    Inventory(ObservedInventoryObjective),
    RetainedOrder(RetainedOrderObjective),
}
impl SelectedObjective {
    fn parent_scope(&self) -> &'static str {
        match self {
            Self::Inventory(_) => "complete_original_labor_and_inventory_goal",
            Self::RetainedOrder(_) => "complete_original_labor_and_order_goal",
        }
    }
}

fn selected(plan: &PreparedPlan, selected_id: StepId) -> Option<(StepId, SelectedObjective)> {
    let step = plan.steps.iter().find(|step| {
        step.id == selected_id && matches!(step.action, Action::CreateWorkOrder { .. })
    })?;
    let objective = ObservedInventoryObjective::from_selected_plan(plan, step.id)
        .map(SelectedObjective::Inventory)
        .or_else(|| {
            RetainedOrderObjective::from_selected_plan(plan, step.id)
                .map(SelectedObjective::RetainedOrder)
        })?;
    Some((step.id, objective))
}

/// Empty initialization has a fixed bound; a retained history may use the
/// caller's larger configured allowance. Custody/output reservations have
/// already been removed, and this does not allocate those bytes eagerly. Keep
/// enough of the ordinary allowance for the first joined sample when an order
/// clock has also prepaid its separate native and original-custody checks.
///
/// The host has one labor predecessor. Its 144 MiB inventory reservation pays
/// two native views, bounding each view cost below 72 MiB. Selected custody,
/// predecessor checks and final publication therefore reserve at most 937 MiB:
/// 160 MiB inventories + 32 MiB order reattachment/inspection + 162 MiB final
/// publication + eight workforce views + 7 MiB local work. Empty history adds
/// the 129 MiB bounded reader, three order views and two small header checks,
/// keeping the complete initialization below 1080 MiB. Nonempty replay keeps
/// its separate caller-sized allowance because it can reproject many captures.
pub(super) fn history_budget(work: &mut Work, initialize: bool) -> Result<OperationContext> {
    if initialize {
        work.take(1536 * MIB)
    } else {
        work.rest(MAX_BYTES)
    }
}

pub(super) fn truth(value: PredicateTruth) -> &'static str {
    match value {
        PredicateTruth::True => "true",
        PredicateTruth::False => "false",
        PredicateTruth::Unknown => "unknown",
    }
}

impl Runtime {
    fn selected_objective(&self) -> Option<(StepId, SelectedObjective)> {
        selected(self.original.as_ref()?, self.selected_order_step()?)
    }
    pub(super) fn parent_goal_observation_fenced(&self) -> bool {
        self.portfolio_fenced()
            || self
                .goal_monitor
                .as_ref()
                .is_some_and(|monitor| monitor.parent_observation_fenced())
    }

    fn current_parent_goal(&self) -> Option<&SelectedParentGoalProgress> {
        if self.clock_withholds_goal() {
            return None;
        }
        let parent = self.parent_goal.as_ref()?;
        let selected = self.goal_monitor.as_ref()?.latest()?;
        // A durable selected history alone cannot revive a parent observation.
        // Both projections must still belong to the same published sample.
        (parent.anchor == self.anchor
            && parent.selected_anchor == selected.anchor
            && (self.portfolio.is_some() || parent.anchor.cursor == selected.anchor.cursor)
            && parent.anchor.tick == selected.anchor.tick)
            .then_some(parent)
    }

    pub(super) fn parent_goal_proven(&self) -> bool {
        if self.portfolio.is_some() {
            return self.portfolio_proven();
        }
        self.current_parent_goal().is_some_and(|parent| {
            parent.original_goal_proven
                && parent.current_truth == PredicateTruth::True
                && self
                    .goal_monitor
                    .as_ref()
                    .and_then(|monitor| monitor.latest())
                    .is_some_and(WorkOrderGoalProgress::current_goal_proven)
        })
    }

    pub(super) fn parent_goal_json(&self) -> Value {
        if self.portfolio.is_some() {
            return self.portfolio_parent_json();
        }
        self.selected_parent_goal_json()
    }
    pub(super) fn selected_parent_goal_json(&self) -> Value {
        let Some((_, objective)) = self.selected_objective() else {
            return Value::Null;
        };
        self.current_parent_goal().map_or(Value::Null, |parent| {
            json!({
                "scope":if self.portfolio.is_some() {"selected_original_labor_and_production_goal"} else {objective.parent_scope()},
                "current_truth":truth(parent.current_truth),
                "original_goal_proven":self.portfolio.is_none() && self.parent_goal_proven(),
                "anchor":anchor_json(parent.anchor),
                "selected_anchor":anchor_json(parent.selected_anchor),
                "source_digest":parent.source_digest.to_hex(),
                "workforce_source_digest":parent.workforce_source_digest.to_hex(),
                "labor":parent.labor.iter().map(|finding| json!({
                    "unit_id":finding.unit_id.get().to_string(),
                    "labor":finding.labor,
                    "requested_enabled":finding.requested_enabled,
                    "observed_enabled":finding.observed_enabled,
                    "current_truth":truth(finding.current_truth)
                })).collect::<Vec<_>>(),
                "historical_labor_receipt_is_current_evidence":false,
                "restored_from_history":false,
                "production_attribution":"unknown"
            })
        })
    }

    pub(super) fn goal_coverage_omissions(&self) -> Vec<&'static str> {
        let mut omissions = Vec::new();
        if self.current_parent_goal().is_none() {
            omissions.extend(["parent goal completion", "current labor after its receipt"]);
        } else if !self.parent_goal_proven() {
            omissions.push("parent goal completion");
        }
        omissions.extend(["order-attributed output", "workshop/material feasibility"]);
        if !self.clock_control_enabled() && !self.inventory_clock_control_enabled() {
            omissions.push("game clock control");
        }
        omissions
    }

    pub(super) fn selected_goal_json(&self) -> Value {
        let Some((step, objective)) = self.selected_objective() else {
            return Value::Null;
        };
        let monitor = self.goal_monitor.as_ref();
        let obligation = self
            .original
            .as_ref()
            .and_then(|plan| plan.steps.iter().find(|candidate| candidate.id == step))
            .and_then(|step| step.obligation.as_ref());
        let current = monitor
            .and_then(|monitor| monitor.latest())
            .filter(|_| !self.clock_withholds_goal());
        let status = monitor.and_then(|monitor| monitor.historical_obligation_status());
        let failed = status.is_some_and(|status| {
            matches!(
                status,
                ObligationStatus::Failed { .. } | ObligationStatus::Cancelled { .. }
            )
        });
        let mut value = json!({
            "scope":"selected_original_order_step", "step_id":step.get(),
            "deadline_tick":obligation.map(|spec| spec.deadline_tick.get()),
            "poll_interval_ticks":obligation.map(|spec| spec.poll_interval_ticks),
            "stable_samples":obligation.map(|spec| spec.stable_for_observations),
            "current_truth":current.map(|sample| truth(sample.current_goal)),
            "proven_at_anchor":current.is_some_and(WorkOrderGoalProgress::current_goal_proven),
            "anchor":current.map(|sample| anchor_json(sample.anchor)),
            "source_digest":current.map(|sample| sample.source_digest.to_hex()),
            "parent_terminal_truth":current.map(|sample| truth(sample.plan_terminal)),
            "original_goal_proven":false,
            "receipt_linked":current.map(|sample| sample.receipt_linked),
            "physical_work":current.map(|sample| format!("{:?}",sample.physical_work)),
            "continuous":current.map(|sample| sample.continuous),
            "temporal_status":current.and_then(|sample| sample.obligation.as_ref()).map(|status| format!("{status:?}")),
            "first_satisfied_anchor":monitor.and_then(|monitor| monitor.first_satisfied_anchor()).map(anchor_json),
            "historical_obligation":status.map(|status| format!("{status:?}")),
            "terminal_failure":failed,
            "history":{"present":self.goal_history_present,
                "events":monitor.map_or(0, |monitor| monitor.history_event_count()),
                "bytes":monitor.map_or(0, |monitor| monitor.history_byte_len()),
                "fenced":monitor.is_some_and(|monitor| monitor.is_fenced())},
            "production_attribution":"unknown"
        });
        match objective {
            SelectedObjective::Inventory(objective) => {
                value["kind"] = json!("observed_inventory_minimum");
                value["item_type"] = json!(objective.item_type);
                value["minimum_stack_units"] = json!(objective.min_stack_units);
                value["quantity_semantics"] = json!("sum_of_observed_stack_size");
                value["parent_terminal_evidence_scope"] =
                    json!("selected_inventory_projection_only");
                value["preexisting_items_included"] = json!(true);
                value["material_classification"] = json!("unknown");
                value["eligibility"] = json!("not_assessed");
            }
            SelectedObjective::RetainedOrder(objective) => {
                value["kind"] = json!("retained_order_completion");
                value["order_entity_id"] = json!(objective.order_entity_id.get().to_string());
                value["required_order_state"] = json!({
                    "amount_total":objective.amount_total,"amount_remaining":0,
                    "validated":true,"active":false
                });
                value["parent_terminal_evidence_scope"] = json!("selected_order_projection_only");
                value["disappearance_proves_completion"] = json!(false);
            }
        }
        value
    }

    pub(super) fn raise_goal_floor(&mut self) {
        self.observation_tick_floor = self
            .observation_tick_floor
            .max(GameTick(self.portfolio_floors()));
        if let Some(execution) = &self.execution {
            // Replay can consume a newer authority tick and then refuse before
            // returning a monitor. That floor still belongs to this owner.
            self.observation_tick_floor = self
                .observation_tick_floor
                .max(GameTick(execution.high_tick()));
        }
        if let Some(monitor) = &self.goal_monitor {
            self.observation_tick_floor = self.observation_tick_floor.max(monitor.high_tick());
            if self.portfolio.is_none()
                && let Some(anchor) = monitor.last_observation_anchor()
                && anchor.cursor.sequence > self.anchor.cursor.sequence
                && anchor.tick >= self.anchor.tick
            {
                // A consumed authority tick alone cannot manufacture a state
                // hash. Only a complete published anchor can replace this one.
                self.anchor = anchor;
            }
        }
    }

    pub(super) fn start_goal_monitor(&mut self, initialize: bool, work: &mut Work) -> Result<()> {
        if self.goal_monitor.is_some() {
            return Ok(());
        }
        let original = self
            .original
            .as_ref()
            .ok_or_else(|| invalid("original combined plan absent"))?
            .clone();
        let (step, _) = self.selected_objective().ok_or_else(|| {
            denied("original plan has no supported selected production objective")
        })?;
        let frontier = self
            .frontier
            .as_mut()
            .ok_or_else(|| denied("original frontier absent"))?
            .inspect(&work.take(8 * MIB)?)?;
        if !frontier.all_applied() || frontier.needs_reconciliation() {
            return Err(denied(
                "goal monitoring requires both original native effects to be settled",
            ));
        }
        self.arm(work)?;
        let custody = self
            .custody
            .as_ref()
            .ok_or_else(|| denied("original custody absent"))?
            .clone();
        // The separately armed custody owner has prepaid its actual reads.
        // Keep a positive final-check context before allocating replay work.
        let final_check = work.take(1)?;
        let budget = history_budget(work, initialize)?;
        custody.verify(&budget)?;
        let path = self.config.path(&if step.get() == 1 {
            "goal.history".into()
        } else {
            format!("goal-{}.history", step.get())
        });
        let config = &self.config;
        // A failed private-file create/replay may have published custody. Retain
        // that fact and never replace a possibly existing history on retry.
        self.goal_history_present = true;
        let policy = |projection: &dfmcp_adapter::semantic_work_orders::WorkOrderGoalProjection,
                      current: &OperationContext| {
            config.current()?;
            config
                .orders
                .acceptance()
                .goal_for_selected(&original, step, projection, current)
        };
        let execution = self
            .execution
            .as_mut()
            .ok_or_else(|| denied("original executor absent"))?;
        let result = if self.portfolio.is_some() {
            execution.open_private_portfolio_order_goal_monitor(
                step,
                &path,
                initialize,
                self.mode == Mode::Offline,
                &budget,
                policy,
            )
        } else {
            execution.open_private_selected_order_goal_monitor(
                step,
                &path,
                initialize,
                self.mode == Mode::Offline,
                &budget,
                policy,
            )
        };
        let verified = custody.verify(&work.narrow(&final_check)?);
        let disarmed = custody.disarm();
        let monitor = result?;
        verified?;
        disarmed?;
        self.goal_monitor = Some(monitor);
        Ok(())
    }

    pub(super) fn wait_with_goal(&mut self, c: &OperationContext) -> Result<View> {
        // No earlier complete-parent proof survives an attempted new sample,
        // including failures before the first durable selected-history event.
        self.parent_goal = None;
        let result = self.wait_with_goal_inner(c);
        self.raise_goal_floor();
        self.raise_clock_floor();
        if result.is_err() {
            // Includes refusals before the first durable poll event. The next
            // poll persists the interruption; no failed call preserves a streak.
            self.abandon_preparation();
        }
        result
    }

    pub(super) fn wait_with_goal_inner(&mut self, c: &OperationContext) -> Result<View> {
        self.check(c, false)?;
        if self.mode == Mode::Offline {
            return self.inspect(c);
        }
        let mut work = Work::new(c)?;
        let output = work.take(256 * MIB)?;
        self.arm(&mut work)?;
        let custody = self
            .custody
            .as_ref()
            .ok_or_else(|| invalid("no original combined execution"))?
            .clone();
        custody.verify(&work.current()?)?;
        let frontier = self
            .frontier
            .as_mut()
            .ok_or_else(|| invalid("original frontier absent"))?;
        let execution = self
            .execution
            .as_mut()
            .ok_or_else(|| invalid("original executor absent"))?;
        let before = frontier.inspect(&work.take(8 * MIB)?)?;
        let settled_budget = work.take(8 * MIB)?;
        if let Some(entry) = before.entries.iter().find(|entry| {
            entry.phase.uncertain() || (entry.phase == Phase::Prepared && before.review.is_none())
        }) {
            frontier.reconcile(entry.step, execution, &work.rest(MAX_FRONTIER_BYTES)?)?;
        }
        let settled = frontier.inspect(&work.narrow(&settled_budget)?)?;
        custody.verify(&work.current()?)?;
        custody.disarm()?;
        self.reconcile_clock(&mut work)?;
        self.reconcile_inventory_clock(&mut work)?;
        let should_sample = settled.all_applied()
            && !settled.needs_reconciliation()
            && !self
                .clock
                .as_ref()
                .is_some_and(ProductionClock::may_be_running)
            && !self
                .inventory_clock
                .as_ref()
                .is_some_and(ProductionInventoryClock::may_be_running)
            && !self.inactive_clock_may_be_running()
            && self.selected_objective().is_some();
        if should_sample {
            c.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
            if self.goal_monitor.is_none() {
                let replay = self.goal_history_present;
                self.start_goal_monitor(!replay, &mut work)?;
                if replay {
                    // A retained history can use the full admitted replay work.
                    // Publish its historical recovery now; a subsequent Wait
                    // obtains a fresh sample under a new bounded operation.
                    self.raise_goal_floor();
                    work.context.anchor = self.anchor;
                    self.config.current()?;
                    return self.view(&mut Work::new(&work.narrow(&output)?)?);
                }
            }
            let portfolio_cursor = if self.portfolio.is_some() {
                let monitor = self
                    .goal_monitor
                    .as_ref()
                    .ok_or_else(|| denied("portfolio selected history absent"))?;
                let cursor = ObservationCursor {
                    epoch: self.anchor.cursor.epoch,
                    sequence: self
                        .anchor
                        .cursor
                        .sequence
                        .max(monitor.high_cursor().sequence)
                        .checked_add(1)
                        .ok_or_else(|| exhausted("portfolio publication cursor exhausted"))?,
                };
                // Each selected history is a separate review-bound observation
                // stream. Its strict source sequence does not skip when another
                // portfolio leaf publishes a sample or reviews a clock.
                work.context.anchor = monitor
                    .last_observation_anchor()
                    .unwrap_or_else(|| monitor.original_plan().anchor);
                work.context.anchor.tick =
                    work.context.anchor.tick.max(self.observation_tick_floor);
                Some(cursor)
            } else {
                None
            };
            self.arm(&mut work)?;
            let original = self
                .original
                .as_ref()
                .ok_or_else(|| denied("original plan absent"))?
                .clone();
            let basis = self
                .basis
                .as_ref()
                .ok_or_else(|| denied("original citizen identity absent"))?;
            let identity_context = work.take(MIB)?;
            custody.verify(&identity_context)?;
            identity_context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
            identity_context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
            let mut identity_policy_context = identity_context.clone();
            if self.portfolio.is_some() {
                // Citizen mapping is historical and anchored to the last real
                // parent publication. Authorization above uses the consumed
                // tick floor; never relabel that old canonical hash at a later
                // tick merely to authorize this immutable identity mapping.
                identity_policy_context.anchor = self.anchor;
            }
            let identity_policy = self.config.workforce.acceptance().historical_citizens(
                &basis.capsule,
                &basis.projection,
                &identity_policy_context,
            )?;
            let identity = LiveRoutingEvidence::citizens_v1(
                PredicateEvidence::scoped(&basis.projection.snapshot, identity_policy)?,
                &basis.projection,
                &basis.capsule,
            )?;
            let monitor = self
                .goal_monitor
                .as_mut()
                .ok_or_else(|| denied("selected monitor absent"))?;
            let step = monitor.step_id();
            let cursor = ObservationCursor {
                epoch: self.anchor.cursor.epoch,
                sequence: if portfolio_cursor.is_some() {
                    monitor.high_cursor().sequence
                } else {
                    self.anchor
                        .cursor
                        .sequence
                        .max(monitor.high_cursor().sequence)
                }
                .checked_add(1)
                .ok_or_else(|| exhausted("selected goal cursor exhausted"))?,
            };
            let config = &self.config;
            let clock_floor = self
                .clock
                .as_ref()
                .map_or(0, ProductionClock::high_tick)
                .max(
                    self.inventory_clock
                        .as_ref()
                        .map_or(0, ProductionInventoryClock::high_tick),
                )
                // The external recovery reference also retains reads rejected
                // before a clock journal event could publish their newer tick.
                .max(self.observation_tick_floor.get());
            let sources = (
                |current: &OperationContext| {
                    config.current()?;
                    custody.verify(current)?;
                    config.orders.progress(current)
                },
                |current: &OperationContext| {
                    config.current()?;
                    custody.verify(current)?;
                    config.orders.operations(current)
                },
            );
            let policies = (
                |projection: &dfmcp_adapter::semantic_work_orders::WorkOrderGoalProjection,
                 current: &OperationContext| {
                    config.current()?;
                    custody.verify(current)?;
                    if projection.anchor().tick.get() < clock_floor {
                        return Err(denied("production evidence predates consumed clock time"));
                    }
                    config
                        .orders
                        .acceptance()
                        .goal_for_selected(&original, step, projection, current)
                },
                |projection: &dfmcp_adapter::semantic_workforce::WorkforceGoalProjection,
                 current: &OperationContext| {
                    config.current()?;
                    custody.verify(current)?;
                    if projection.anchor().tick.get() < clock_floor {
                        return Err(denied("labor evidence predates consumed clock time"));
                    }
                    config.workforce.acceptance().goal(projection, current)
                },
            );
            let current = work.rest(MAX_BYTES)?;
            let execution = self
                .execution
                .as_mut()
                .ok_or_else(|| denied("original executor absent"))?;
            let result = if let Some(parent_cursor) = portfolio_cursor {
                execution.poll_selected_portfolio_goal(
                    monitor,
                    &identity,
                    cursor,
                    &current,
                    sources,
                    policies,
                    parent_cursor,
                )
            } else {
                execution.poll_selected_parent_goal(
                    monitor, &identity, cursor, &current, sources, policies,
                )
            };
            self.raise_goal_floor();
            work.context.anchor = result
                .as_ref()
                .map_or(self.anchor, |result| result.parent_progress().anchor);
            let verified = custody.verify(&work.narrow(&output)?);
            let disarmed = custody.disarm();
            let result = result?;
            verified?;
            disarmed?;
            // The joined snapshot has its own digest; the selected production
            // anchor remains separately visible and is never relabelled.
            self.anchor = result.parent_progress().anchor;
            self.parent_goal = Some(result.parent_progress().clone());
            if self.portfolio.is_some() {
                self.portfolio_sample = Some(result);
            }
            self.clock_invalidates_goal = false;
            self.inventory_clock_invalidates_goal = false;
            self.goal_current_withdrawn = false;
        }
        self.config.current()?;
        work.context.anchor = self.anchor;
        self.view(&mut Work::new(&work.narrow(&output)?)?)
    }
}
