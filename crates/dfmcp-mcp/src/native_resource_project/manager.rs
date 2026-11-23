//! Optional original Manager prefix. Every later child retains the exact
//! acknowledged appointment and its independently exported production donor.
use super::*;

impl Runtime {
    pub(super) fn manager_handoff(&self) -> Result<ManagerProjectHandoff> {
        let config = self
            .config
            .manager
            .as_ref()
            .ok_or_else(|| denied("this original project has no Manager stage"))?;
        let checkpoint = &self.journal.snapshot().checkpoint;
        let value = checkpoint
            .manager_handoff
            .as_ref()
            .ok_or_else(|| denied("project has no acknowledged Manager handoff"))?;
        let handoff = ManagerProjectHandoff::from_json(value)?;
        config.validate_project_handoff(&handoff)?;
        handoff.validate_project_checkpoint(
            checkpoint
                .manager
                .as_ref()
                .ok_or_else(|| denied("project lost its original Manager checkpoint"))?,
        )?;
        if handoff.project_deadline_tick() != self.config.goal.deadline_tick
            || self
                .config
                .manager_handoff
                .as_ref()
                .is_some_and(|configured| configured.to_json() != *value)
        {
            return Err(denied(
                "project Manager handoff or original deadline changed on recovery",
            ));
        }
        Ok(handoff)
    }

    pub(super) fn production_manager_handoff(&self) -> Result<Option<ManagerProjectHandoff>> {
        if self.config.manager.is_some() {
            self.manager_handoff().map(Some)
        } else {
            Ok(None)
        }
    }

    fn manager_budget(&self, work: &mut Work, reserve_after: u64) -> Result<WorkBudget> {
        let mut budget = work.child(WORKSHOP_BYTES, reserve_after)?;
        budget.max_actions = 1;
        budget.max_entities = budget.max_entities.min(4096);
        budget.max_output_tokens = budget.max_output_tokens.min(32_768);
        budget.max_game_ticks = 0;
        Ok(budget)
    }

    pub(super) fn open_manager(&mut self, work: &mut Work) -> Result<()> {
        let config = self
            .config
            .manager
            .as_ref()
            .ok_or_else(|| denied("retained Manager stage has no original Manager policy"))?
            .clone();
        if (!self.config.initialize && config.project_initializing())
            || self.config.manager_handoff.is_some()
            || self.config.tree_handoff.is_some()
            || self.config.handoff.is_some()
        {
            return Err(denied(
                "recovery cannot initialize or substitute the original Manager child",
            ));
        }
        let prior = self.journal.snapshot().checkpoint.manager.clone();
        if let Some(reference) = &prior {
            config.validate_project_recovery(reference)?;
        }
        let budget = self.manager_budget(work, journal::WORK_BYTES)?;
        let (child, _) = ManagerRuntime::open_project(
            config,
            manager_mode(self.mode),
            self.session,
            budget,
            Instant::now(),
            self.high_tick,
        )?;
        // Retain the opened owner before checking a recovered descendant. A
        // rejected original stays inspectable and never becomes a new permit.
        self.child = Some(Child::Manager(Box::new(child)));
        if prior.is_none()
            && let Some(Child::Manager(child)) = &self.child
            && child.project_has_original_history()
        {
            return Err(denied(
                "an unacknowledged Manager opening cannot adopt preexisting appointment history",
            ));
        }
        if let Some(reference) = &prior
            && let Some(Child::Manager(child)) = &mut self.child
        {
            child.retain_project_recovery(reference)?;
            child.raise_project_tick_floor(self.high_tick)?;
            continuity::same_original(reference, &child.project_recovery_reference())?;
        }
        self.save_child(work, Some(Stage::Manager))
    }

    pub(super) fn advance_from_manager(&mut self, raw: &str, work: &mut Work) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Advance {
            kind: String,
            expected_head: String,
        }
        let request: Advance = serde_json::from_str(raw)
            .map_err(|_| invalid("invalid closed Manager transition request"))?;
        if request.kind != "advance_from_manager"
            || request.expected_head != self.journal.pin().head
        {
            return Err(denied(
                "Manager transition requires the exact current parent head",
            ));
        }
        self.authorize_forward(work)?;
        if self.effective_stage() == Stage::Manager {
            let budget = self.manager_budget(work, 4 * journal::WORK_BYTES)?;
            let Some(Child::Manager(child)) = &mut self.child else {
                return Err(denied("original Manager owner is unavailable"));
            };
            child.raise_project_tick_floor(self.high_tick)?;
            let handoff = child.project_handoff(budget)?;
            // The export read consumed rejection floors even if a later
            // parent write fails. Save those before freezing the donor.
            self.save_child(work, None)?;
            self.authorize_forward(work)?;
            let mut checkpoint = self.journal.snapshot().checkpoint.clone();
            handoff.validate_project_checkpoint(
                checkpoint
                    .manager
                    .as_ref()
                    .ok_or_else(|| denied("Manager export lost its acknowledged original"))?,
            )?;
            checkpoint.manager_handoff = Some(handoff.to_json());
            self.append(work, Stage::ManagerTransition, checkpoint)?;
            // Only after both parent stores acknowledge transfer may the
            // exclusive donor writer be released for eventual Offline import.
            if let Some(child) = &mut self.child {
                child.withdraw();
            }
            self.child = None;
        }
        if self.effective_stage() != Stage::ManagerTransition {
            return Err(denied(
                "project cannot skip, repeat or replace its original Manager stage",
            ));
        }
        self.manager_handoff()?;
        self.authorize_forward(work)?;
        let next = if self.config.tree.is_some() {
            Stage::TreeOpening
        } else {
            Stage::WorkshopOpening
        };
        if let Some(config) = &self.config.tree {
            if !config.project_initializing() {
                return Err(denied(
                    "first Tree entry requires fresh custody; recover retained opening markers separately",
                ));
            }
        } else if !self.workshop_config.project_initializing() {
            return Err(denied(
                "first Workshop entry requires fresh custody; recover retained opening markers separately",
            ));
        }
        self.append(work, next, self.journal.snapshot().checkpoint.clone())?;
        let destination = if let Some(config) = self.config.tree.clone() {
            let budget = self.tree_budget(work, journal::WORK_BYTES)?;
            let (child, _) = TreeRuntime::open_for_resource_project(
                config,
                tree_mode(self.mode),
                self.session,
                budget,
                Instant::now(),
                self.high_tick,
            )?;
            self.child = Some(Child::Tree(Box::new(child)));
            self.save_child(work, Some(Stage::Tree))?;
            "tree"
        } else {
            let config = self.workshop_config.activate(None)?;
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
            "workshop"
        };
        Ok(
            json!({"advanced_to":destination, "native_effect_dispatched":false,
            "observe_required":true,"fresh_review_required":true,
            "manager_historical_appointment_is_not_current_production_readiness":true,
            "child":self.last_child_reference}),
        )
    }
}

pub(super) fn action(action: Action, offline: bool, deadline: u64) -> Result<ManagerAction> {
    Ok(match action {
        Action::Observe(None) => ManagerAction::Observe,
        Action::Observe(Some(_)) => {
            return Err(invalid("Manager observation has no caller-selected source"));
        }
        Action::Inspect => ManagerAction::Query(None),
        Action::Plan(raw) => {
            // Deserialize the original raw request, preserving duplicate-field
            // rejection before the existing child receives exactly these bytes.
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Appointment {
                kind: String,
                expected_observation: String,
                deadline_tick: u64,
            }
            let request: Appointment = serde_json::from_str(&raw)
                .map_err(|_| invalid("invalid closed project Manager request"))?;
            if request.kind != "appoint_vacant_manager" || request.deadline_tick != deadline {
                return Err(denied(
                    "Manager request changed the original project goal or deadline",
                ));
            }
            digest(&request.expected_observation)?;
            ManagerAction::Plan(raw)
        }
        Action::Commit {
            seal,
            confirmed,
            scope,
        } => {
            if scope.as_deref().unwrap_or("goal") != "goal" {
                return Err(invalid(
                    "Manager appointment has only the original goal commit scope",
                ));
            }
            ManagerAction::Commit(seal, confirmed)
        }
        Action::Wait(scope) => match scope.as_deref().unwrap_or("goal") {
            "goal" | "project" if offline => ManagerAction::Query(None),
            "goal" | "project" => ManagerAction::Wait,
            _ => return Err(invalid("Manager appointment has no clock wait scope")),
        },
        Action::Cancel(scope) => match scope.as_deref().unwrap_or("project") {
            "goal" | "project" => ManagerAction::Cancel,
            _ => {
                return Err(invalid(
                    "Manager appointment has no clock cancellation scope",
                ));
            }
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manager_only_routes_its_original_fixed_deadline_appointment() -> Result<()> {
        let witness = Digest32::of_bytes(b"original Manager observation").to_hex();
        let request = json!({"kind":"appoint_vacant_manager","expected_observation":witness,"deadline_tick":200});
        assert!(matches!(
            action(Action::Plan(request.to_string()), false, 200)?,
            ManagerAction::Plan(_)
        ));
        assert!(action(Action::Plan(request.to_string()), false, 201).is_err());
        let duplicate = format!(
            "{{\"kind\":\"appoint_vacant_manager\",\"expected_observation\":\"{witness}\",\"deadline_tick\":199,\"deadline_tick\":200}}"
        );
        assert!(action(Action::Plan(duplicate), false, 200).is_err());
        for field in [
            "unit_id",
            "role",
            "protocol",
            "manager_prerequisite",
            "confirmed",
        ] {
            let mut changed = request.clone();
            changed[field] = json!(true);
            assert!(action(Action::Plan(changed.to_string()), false, 200).is_err());
        }
        Ok(())
    }

    #[test]
    fn manager_retains_reconciliation_without_any_clock_route() -> Result<()> {
        assert!(matches!(
            action(Action::Wait(None), true, 200)?,
            ManagerAction::Query(None)
        ));
        assert!(matches!(
            action(Action::Wait(None), false, 200)?,
            ManagerAction::Wait
        ));
        assert!(matches!(
            action(Action::Cancel(None), false, 200)?,
            ManagerAction::Cancel
        ));
        for scope in ["clock", "inventory_clock", "session"] {
            assert!(action(Action::Wait(Some(scope.into())), false, 200).is_err());
            assert!(action(Action::Cancel(Some(scope.into())), false, 200).is_err());
            assert!(
                action(
                    Action::Commit {
                        seal: "seal".into(),
                        confirmed: true,
                        scope: Some(scope.into())
                    },
                    false,
                    200
                )
                .is_err()
            );
        }
        assert!(action(Action::Observe(Some("{}".into())), false, 200).is_err());
        Ok(())
    }
}
