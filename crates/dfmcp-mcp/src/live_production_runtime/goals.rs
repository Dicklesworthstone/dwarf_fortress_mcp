//! Foreground proof of the complete original labor/inventory goal. Current
//! labor brackets the selected inventory sample; historical receipts only
//! establish custody and never become current labor facts.
use super::*;
use dfmcp_adapter::live_routing::LiveRoutingEvidence;
use dfmcp_adapter::semantic_work_orders::{ObservedInventoryObjective, WorkOrderGoalProgress};
use dfmcp_core::StepId;
use dfmcp_intent::ObligationStatus;
use dfmcp_world::PredicateTruth;

fn selected(plan: &PreparedPlan) -> Option<(StepId, ObservedInventoryObjective)> {
    let step = plan
        .steps
        .iter()
        .find(|step| matches!(step.action, Action::CreateWorkOrder { .. }))?;
    ObservedInventoryObjective::from_selected_plan(plan, step.id).map(|goal| (step.id, goal))
}

/// Empty initialization has a fixed bound; a retained history may use the
/// caller's larger configured allowance. Custody/output reservations have
/// already been removed, and this does not allocate those bytes eagerly.
pub(super) fn history_budget(work: &mut Work, initialize: bool) -> Result<OperationContext> {
    if initialize {
        work.take(2 * 1024 * MIB)
    } else {
        work.rest(MAX_BYTES)
    }
}

fn truth(value: PredicateTruth) -> &'static str {
    match value {
        PredicateTruth::True => "true",
        PredicateTruth::False => "false",
        PredicateTruth::Unknown => "unknown",
    }
}

impl Runtime {
    pub(super) fn parent_goal_observation_fenced(&self) -> bool {
        self.goal_monitor
            .as_ref()
            .is_some_and(|monitor| monitor.parent_observation_fenced())
    }

    fn current_parent_goal(&self) -> Option<&SelectedParentGoalProgress> {
        let parent = self.parent_goal.as_ref()?;
        let selected = self.goal_monitor.as_ref()?.latest()?;
        // A durable selected history alone cannot revive a parent observation.
        // Both projections must still belong to the same published sample.
        (parent.anchor == self.anchor
            && parent.selected_anchor == selected.anchor
            && parent.anchor.cursor == selected.anchor.cursor
            && parent.anchor.tick == selected.anchor.tick)
            .then_some(parent)
    }

    pub(super) fn parent_goal_proven(&self) -> bool {
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
        self.current_parent_goal().map_or(Value::Null, |parent| {
            json!({
                "scope":"complete_original_labor_and_inventory_goal",
                "current_truth":truth(parent.current_truth),
                "original_goal_proven":self.parent_goal_proven(),
                "anchor":anchor_json(parent.anchor),
                "selected_anchor":anchor_json(parent.selected_anchor),
                "source_digest":parent.source_digest.to_hex(),
                "workforce_source_digest":parent.workforce_source_digest.to_hex(),
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
        omissions.extend([
            "order-attributed output",
            "workshop/material feasibility",
            "game clock control",
        ]);
        omissions
    }

    pub(super) fn selected_goal_json(&self) -> Value {
        let Some((step, objective)) = self.original.as_ref().and_then(selected) else {
            return Value::Null;
        };
        let monitor = self.goal_monitor.as_ref();
        let obligation = self
            .original
            .as_ref()
            .and_then(|plan| plan.steps.iter().find(|candidate| candidate.id == step))
            .and_then(|step| step.obligation.as_ref());
        let current = monitor.and_then(|monitor| monitor.latest());
        let status = monitor.and_then(|monitor| monitor.historical_obligation_status());
        let failed = status.is_some_and(|status| {
            matches!(
                status,
                ObligationStatus::Failed { .. } | ObligationStatus::Cancelled { .. }
            )
        });
        json!({
            "scope":"selected_original_order_step", "step_id":step.get(),
            "kind":"observed_inventory_minimum", "item_type":objective.item_type,
            "minimum_stack_units":objective.min_stack_units,
            "quantity_semantics":"sum_of_observed_stack_size",
            "deadline_tick":obligation.map(|spec| spec.deadline_tick.get()),
            "poll_interval_ticks":obligation.map(|spec| spec.poll_interval_ticks),
            "stable_samples":obligation.map(|spec| spec.stable_for_observations),
            "current_truth":current.map(|sample| truth(sample.current_goal)),
            "proven_at_anchor":current.is_some_and(WorkOrderGoalProgress::current_goal_proven),
            "anchor":current.map(|sample| anchor_json(sample.anchor)),
            "source_digest":current.map(|sample| sample.source_digest.to_hex()),
            "parent_terminal_truth":current.map(|sample| truth(sample.plan_terminal)),
            "parent_terminal_evidence_scope":"selected_inventory_projection_only",
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
            "preexisting_items_included":true, "production_attribution":"unknown",
            "material_classification":"unknown", "eligibility":"not_assessed"
        })
    }

    pub(super) fn raise_goal_floor(&mut self) {
        if let Some(execution) = &self.execution {
            // Replay can consume a newer authority tick and then refuse before
            // returning a monitor. That floor still belongs to this owner.
            self.observation_tick_floor = self
                .observation_tick_floor
                .max(GameTick(execution.high_tick()));
        }
        if let Some(monitor) = &self.goal_monitor {
            self.observation_tick_floor = self.observation_tick_floor.max(monitor.high_tick());
            if let Some(anchor) = monitor.last_observation_anchor()
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
        let (step, _) = selected(&original)
            .ok_or_else(|| denied("original plan has no selected inventory objective"))?;
        let frontier = self
            .frontier
            .as_mut()
            .ok_or_else(|| denied("original frontier absent"))?
            .inspect(&work.take(8 * MIB)?)?;
        if !frontier.all_applied() || frontier.needs_reconciliation() {
            return Err(denied(
                "inventory monitoring requires both original native effects to be settled",
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
        let path = self.config.path("goal.history");
        let config = &self.config;
        // A failed private-file create/replay may have published custody. Retain
        // that fact and never replace a possibly existing history on retry.
        self.goal_history_present = true;
        let result = self
            .execution
            .as_mut()
            .ok_or_else(|| denied("original executor absent"))?
            .open_private_selected_order_goal_monitor(
                step,
                &path,
                initialize,
                self.mode == Mode::Offline,
                &budget,
                |projection, current| {
                    config.current()?;
                    config
                        .orders
                        .acceptance()
                        .goal_for_selected(&original, step, projection, current)
                },
            );
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
        if result.is_err() {
            // Includes refusals before the first durable poll event. The next
            // poll persists the interruption; no failed call preserves a streak.
            self.abandon_preparation();
        }
        result
    }

    fn wait_with_goal_inner(&mut self, c: &OperationContext) -> Result<View> {
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
        let should_sample = settled.all_applied()
            && !settled.needs_reconciliation()
            && self.original.as_ref().and_then(selected).is_some();
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
            let identity_policy = self.config.workforce.acceptance().historical_citizens(
                &basis.capsule,
                &basis.projection,
                &identity_context,
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
                sequence: self
                    .anchor
                    .cursor
                    .sequence
                    .max(monitor.high_cursor().sequence)
                    .checked_add(1)
                    .ok_or_else(|| exhausted("selected goal cursor exhausted"))?,
            };
            let config = &self.config;
            let result = self
                .execution
                .as_mut()
                .ok_or_else(|| denied("original executor absent"))?
                .poll_selected_parent_goal(
                    monitor,
                    &identity,
                    cursor,
                    &work.rest(MAX_BYTES)?,
                    (
                        |current| {
                            config.current()?;
                            custody.verify(current)?;
                            config.orders.progress(current)
                        },
                        |current| {
                            config.current()?;
                            custody.verify(current)?;
                            config.orders.operations(current)
                        },
                    ),
                    (
                        |projection, current| {
                            config.current()?;
                            custody.verify(current)?;
                            config
                                .orders
                                .acceptance()
                                .goal_for_selected(&original, step, projection, current)
                        },
                        |projection, current| {
                            config.current()?;
                            custody.verify(current)?;
                            config.workforce.acceptance().goal(projection, current)
                        },
                    ),
                );
            self.raise_goal_floor();
            work.context.anchor = self.anchor;
            let verified = custody.verify(&work.narrow(&output)?);
            let disarmed = custody.disarm();
            let result = result?;
            verified?;
            disarmed?;
            // The joined snapshot has its own digest; the selected inventory
            // anchor remains separately visible and is never relabelled.
            self.anchor = result.parent_progress().anchor;
            self.parent_goal = Some(result.parent_progress().clone());
        }
        self.config.current()?;
        work.context.anchor = self.anchor;
        self.view(&mut Work::new(&work.narrow(&output)?)?)
    }
}
