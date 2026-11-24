//! Tree prefix of the original resource project. A fresh typed handoff is
//! acknowledged before releasing the original writer or opening Workshop.
use super::*;

/// A failed parent append can return a later rejection floor than the last
/// acknowledged child checkpoint. Narrow that floor before new native reads;
/// all source, original and clock identities remain the actual retained ones.
fn recovery_with_parent_floor(reference: &Value, parent_tick: u64) -> Result<Value> {
    let mut reference = reference.clone();
    let floor = reference
        .pointer_mut("/project_source_frontier/high_tick")
        .ok_or_else(|| denied("Tree recovery lost its retained source tick floor"))?;
    let child_tick = floor
        .as_u64()
        .ok_or_else(|| denied("Tree recovery source tick floor is invalid"))?;
    *floor = json!(child_tick.max(parent_tick));
    Ok(reference)
}

impl Runtime {
    pub(super) fn tree_handoff(&self) -> Result<TreeProjectHandoff> {
        if self.config.tree.is_none() {
            return Err(denied("this original resource project has no Tree stage"));
        }
        let value = self
            .journal
            .snapshot()
            .checkpoint
            .tree_handoff
            .as_ref()
            .ok_or_else(|| denied("project has no acknowledged Tree handoff"))?;
        let handoff = TreeProjectHandoff::from_json(value)?;
        if handoff.project_deadline_tick() != self.config.goal.deadline_tick()
            || self
                .config
                .tree_handoff
                .as_ref()
                .is_some_and(|configured| configured.to_json() != *value)
        {
            return Err(denied(
                "project Tree handoff or original deadline changed on recovery",
            ));
        }
        Ok(handoff)
    }

    pub(super) fn tree_budget(&self, work: &mut Work, reserve_after: u64) -> Result<WorkBudget> {
        let mut budget = work.child(WORKSHOP_BYTES, reserve_after)?;
        budget.max_actions = 1;
        budget.max_entities = budget.max_entities.min(4096);
        budget.max_output_tokens = budget.max_output_tokens.min(32_768);
        if !self
            .config
            .tree
            .as_ref()
            .is_some_and(|config| config.project_clock_configured())
        {
            budget.max_game_ticks = 0;
        }
        Ok(budget)
    }

    pub(super) fn open_tree(&mut self, work: &mut Work) -> Result<()> {
        let config = self
            .config
            .tree
            .as_ref()
            .ok_or_else(|| denied("retained Tree stage has no original Tree policy"))?
            .clone();
        if (!self.config.initialize && config.project_initializing())
            || self.config.tree_handoff.is_some()
            || self.config.handoff.is_some()
        {
            return Err(denied(
                "recovery cannot initialize or substitute the original Tree child",
            ));
        }
        let prior = self.journal.snapshot().checkpoint.tree.clone();
        if let Some(reference) = &prior {
            config.validate_project_recovery(reference)?;
        }
        let budget = self.tree_budget(work, journal::WORK_BYTES)?;
        let (child, _) = TreeRuntime::open_for_resource_project(
            config,
            tree_mode(self.mode),
            self.session,
            budget,
            Instant::now(),
            self.high_tick,
        )?;
        // Keep custody before any retained-reference comparison can fail.
        self.child = Some(Child::Tree(Box::new(child)));
        if prior.is_none()
            && let Some(Child::Tree(child)) = &self.child
            && child.project_has_original_history()
        {
            return Err(denied(
                "an unacknowledged Tree opening cannot adopt preexisting original or clock history",
            ));
        }
        if let Some(reference) = &prior
            && let Some(Child::Tree(child)) = &mut self.child
        {
            let retained = recovery_with_parent_floor(reference, self.high_tick)?;
            child.retain_project_recovery(&retained)?;
            continuity::same_original(reference, &child.project_recovery_reference())?;
        }
        self.save_child(work, Some(Stage::Tree))
    }

    pub(super) fn advance_to_workshop(&mut self, raw: &str, work: &mut Work) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Advance {
            kind: String,
            expected_head: String,
        }
        let request: Advance = serde_json::from_str(raw)
            .map_err(|_| invalid("invalid closed Tree-to-Workshop transition request"))?;
        if request.kind != "advance_to_workshop" || request.expected_head != self.journal.pin().head
        {
            return Err(denied(
                "Tree transition requires the exact current parent head",
            ));
        }
        self.authorize_forward(work)?;
        if self.effective_stage() == Stage::Tree {
            let budget = self.tree_budget(work, 4 * journal::WORK_BYTES)?;
            let Some(Child::Tree(child)) = &mut self.child else {
                return Err(denied("original Tree owner is unavailable"));
            };
            child.retain_project_authority_tick(self.high_tick)?;
            let handoff = child.project_handoff(budget)?;
            // This revalidates current, paused supply; achieved_tick alone is
            // historical evidence. Consume its floors even if transfer fails.
            self.save_child(work, None)?;
            self.authorize_forward(work)?;
            let mut checkpoint = self.journal.snapshot().checkpoint.clone();
            checkpoint.tree_handoff = Some(handoff.to_json());
            self.append(work, Stage::TreeTransition, checkpoint)?;
            if let Some(child) = &mut self.child {
                child.withdraw();
            }
            self.child = None;
        }
        if self.effective_stage() != Stage::TreeTransition {
            return Err(denied(
                "project cannot skip, repeat or replace its original Tree stage",
            ));
        }
        let handoff = self.tree_handoff()?;
        let config = self.workshop_config.activate(Some(&handoff))?;
        if !config.project_initializing() {
            return Err(denied(
                "first Workshop entry requires fresh custody; recover retained opening markers separately",
            ));
        }
        self.authorize_forward(work)?;
        self.append(
            work,
            Stage::WorkshopOpening,
            self.journal.snapshot().checkpoint.clone(),
        )?;
        let budget = self.workshop_budget(work, journal::WORK_BYTES)?;
        let child = WorkshopRuntime::open_for_resource_project(
            config,
            workshop_mode(self.mode),
            self.session,
            budget,
            self.high_tick,
        )?;
        self.child = Some(Child::Workshop(Box::new(child)));
        self.save_child(work, Some(Stage::Workshop))?;
        Ok(json!({
            "advanced_to":"workshop", "native_effect_dispatched":false,
            "observe_required":true, "fresh_review_required":true,
            "tree_supply_is_not_final_stock_proof":true,
            "child":self.last_child_reference,
        }))
    }
}

pub(super) fn action(action: Action, offline: bool) -> Result<TreeAction> {
    action.validate_order_scope(false, false)?;
    Ok(match action {
        Action::Observe(None) => TreeAction::Observe,
        Action::Observe(Some(_)) => {
            return Err(invalid("Tree observation has no caller-selected source"));
        }
        Action::Inspect => TreeAction::Query,
        Action::InspectOrder(_) | Action::InspectProduction(..) => {
            return Err(invalid("Tree has no production inspection selector"));
        }
        Action::Plan(raw) => {
            let value: Value =
                serde_json::from_str(&raw).map_err(|_| invalid("invalid Tree project request"))?;
            match value["kind"].as_str() {
                Some("run_tree") => TreeAction::ClockPlan(raw),
                Some("fell_one_visible_tree") => TreeAction::Plan(raw),
                _ => {
                    return Err(invalid(
                        "project Tree permits only its original designation and independently reviewed clock",
                    ));
                }
            }
        }
        Action::Commit {
            seal,
            confirmed,
            scope,
            order_step_id: _,
        } => match scope.as_deref().unwrap_or("goal") {
            "goal" => TreeAction::Commit(seal, confirmed),
            "clock" => TreeAction::ClockCommit(seal, confirmed),
            _ => return Err(invalid("unknown Tree commit scope")),
        },
        Action::Wait(scope) => match scope.as_deref().unwrap_or("goal") {
            "goal" | "project" if offline => TreeAction::Query,
            "goal" | "project" => TreeAction::Wait,
            "clock" => TreeAction::ClockWait,
            _ => return Err(invalid("unknown Tree wait scope")),
        },
        Action::Cancel { scope, .. } => match scope.as_deref().unwrap_or("project") {
            "goal" | "project" => TreeAction::Cancel,
            "clock" => TreeAction::ClockCancel,
            _ => return Err(invalid("unknown Tree cancellation scope")),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_recovery_keeps_a_later_parent_rejection_floor_without_changing_identity() -> Result<()>
    {
        let reference = json!({"acknowledged":{"journal_id":"tree","head":"ack"},
            "original":{"key":"original","deadline_tick":200},
            "project_source_frontier":{"high_tick":80,"high_generation":2,"high_sequence":7,"high_next_item_id":31,"fenced":true},
            "clock":{"recovery":{"journal_id":"clock","head":"clock-ack"}}});
        let mut expected = reference.clone();
        expected["project_source_frontier"]["high_tick"] = json!(100);
        assert_eq!(recovery_with_parent_floor(&reference, 100)?, expected);
        assert_eq!(reference["project_source_frontier"]["high_tick"], 80);
        assert_eq!(recovery_with_parent_floor(&expected, 90)?, expected);
        assert!(recovery_with_parent_floor(&json!({"project_source_frontier":{}}), 100).is_err());
        assert!(
            recovery_with_parent_floor(&json!({"project_source_frontier":{"high_tick":"80"}}), 100)
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn tree_clock_and_effect_keep_independent_actions_and_scope() -> Result<()> {
        assert!(matches!(
            action(Action::Plan(json!({"kind":"run_tree"}).to_string()), false)?,
            TreeAction::ClockPlan(_)
        ));
        assert!(matches!(
            action(
                Action::Plan(json!({"kind":"fell_one_visible_tree"}).to_string()),
                false
            )?,
            TreeAction::Plan(_)
        ));
        assert!(matches!(
            action(
                Action::Commit {
                    seal: "s".into(),
                    confirmed: true,
                    scope: Some("clock".into()),
                    order_step_id: None,
                },
                false
            )?,
            TreeAction::ClockCommit(_, true)
        ));
        assert!(matches!(
            action(
                Action::Commit {
                    seal: "s".into(),
                    confirmed: true,
                    scope: None,
                    order_step_id: None,
                },
                false
            )?,
            TreeAction::Commit(_, true)
        ));
        assert!(matches!(
            action(Action::Cancel { scope: None, order_step_id: None }, false)?,
            TreeAction::Cancel
        ));
        assert!(matches!(
            action(Action::Cancel { scope: Some("clock".into()), order_step_id: None }, false)?,
            TreeAction::ClockCancel
        ));
        assert!(matches!(
            action(Action::Wait(None), true)?,
            TreeAction::Query
        ));
        assert!(matches!(
            action(Action::Wait(Some("clock".into())), false)?,
            TreeAction::ClockWait
        ));
        Ok(())
    }

    #[test]
    fn tree_does_not_route_foreign_goals_or_selectors() {
        for kind in ["stock_target", "run", "inventory_run", "plan_tree", "build"] {
            assert!(action(Action::Plan(json!({"kind":kind}).to_string()), false).is_err());
        }
        assert!(action(Action::Observe(Some("{}".into())), false).is_err());
        assert!(
            action(
                Action::Commit {
                    seal: "s".into(),
                    confirmed: true,
                    scope: Some("inventory_clock".into()),
                    order_step_id: None,
                },
                false
            )
            .is_err()
        );
        assert!(action(Action::Wait(Some("inventory_clock".into())), false).is_err());
        assert!(action(Action::Cancel { scope: Some("inventory_clock".into()), order_step_id: None }, false).is_err());
    }
}
