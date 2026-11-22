//! One read-only Workshop attachment to an already opened, inert production owner.
//! The parent owns both phases; this module creates no native journal or effect.
use super::*;
use crate::workshop_construction_server::runtime::ProductionHandoff;

/// This is charged once for the complete local handoff, before either donor
/// import or native-custody inspection. No frame-size or eager allocation claim.
pub(crate) const ATTACH_BYTES: u64 =
    workshop_prerequisite::OPEN_BYTES + 160 * MIB + 16 * MIB + 2 * MIB;

fn within(inner: WorkBudget, outer: WorkBudget) -> bool {
    inner.max_bytes <= outer.max_bytes
        && inner.max_wall_millis <= outer.max_wall_millis
        && inner.max_entities <= outer.max_entities
        && inner.max_actions <= outer.max_actions
        && inner.max_output_tokens <= outer.max_output_tokens
        && inner.max_game_ticks <= outer.max_game_ticks
}

impl Runtime {
    /// The opening reservation shares the parent's current deadline and bytes;
    /// future requests keep the independently validated full session ceiling.
    pub(crate) fn open_for_parent(
        config: Config,
        mode: Mode,
        session: SessionId,
        session_budget: WorkBudget,
        opening_budget: WorkBudget,
        authority_tick_floor: u64,
    ) -> Result<Self> {
        session_budget.validate()?;
        opening_budget.validate()?;
        if config.parent_configuration_digest().is_none()
            || !within(opening_budget, session_budget)
            || authority_tick_floor > dfmcp_adapter::bounded_run::MAX_NATIVE_TICK
        {
            return Err(denied(
                "invalid staged production opening reservation or authority floor",
            ));
        }
        Self::open_reserved(
            config,
            mode,
            session,
            session_budget,
            opening_budget,
            authority_tick_floor,
        )
    }

    pub(crate) fn parent_authority_tick_floor(&self) -> u64 {
        self.authority_tick_floor()
            .get()
            .max(self.anchor.tick.get())
            .max(
                self.frontier
                    .as_ref()
                    .map_or(0, |frontier| frontier.recovery().authority_tick_floor),
            )
            .max(self.execution.as_ref().map_or(0, Execution::high_tick))
            .max(
                self.goal_monitor
                    .as_ref()
                    .map_or(0, |monitor| monitor.high_tick().get()),
            )
            .max(self.clock.as_ref().map_or(0, ProductionClock::high_tick))
            .max(
                self.inventory_clock
                    .as_ref()
                    .map_or(0, ProductionInventoryClock::high_tick),
            )
            .max(self.portfolio_floors())
    }

    pub(crate) fn retain_parent_authority_floor(&mut self, tick: u64) {
        self.observation_tick_floor = GameTick(self.parent_authority_tick_floor().max(tick));
    }

    /// Only the exact parent configuration may add its verified donor, before
    /// any canonical observation or original production identity exists.
    pub(crate) fn attach_parent_workshop(
        &mut self,
        config: Config,
        handoff: &ProductionHandoff,
        c: &OperationContext,
    ) -> Result<()> {
        // Consumed Workshop/Run time survives every subsequent refusal.
        self.retain_parent_authority_floor(handoff.authority_tick_floor());
        self.check(c, false)?;
        config.current()?;
        let original_configuration = self
            .config
            .parent_configuration_digest()
            .ok_or_else(|| denied("only a staged production owner can import its Workshop"))?;
        if config.parent_configuration_digest() != Some(original_configuration)
            || self.config.workshop_prerequisite.is_some()
            || self.workshop.is_some()
            || config.recovery.is_some()
            || self.original.is_some()
            || self.basis.is_some()
            || self.layout.is_some()
            || self.execution.is_some()
            || self.frontier.is_some()
            || self.custody.is_some()
            || self.seal.is_some()
            || self.identity_digest.is_some()
            || self.goal_monitor.is_some()
            || self.portfolio.is_some()
            || self.initialization_failed
            || self.fenced
            || self.clock.as_ref().is_some_and(ProductionClock::has_runs)
            || self
                .inventory_clock
                .as_ref()
                .is_some_and(ProductionInventoryClock::has_runs)
        {
            return Err(denied(
                "Workshop can attach only once to the untouched staged production owner",
            ));
        }
        config.require_parent_workshop(handoff.binding(), handoff.directory())?;
        let selected = config.workshop_prerequisite.clone().ok_or_else(|| {
            denied("staged production requires its verified Workshop prerequisite")
        })?;
        if selected.acknowledged != handoff.pin()
            || selected.original_key != handoff.original_key()
            || selected.original_plan_digest != handoff.original_plan_digest()
        {
            return Err(denied(
                "staged production lost its exact acknowledged Workshop original",
            ));
        }
        if c.budget.max_bytes < ATTACH_BYTES {
            return Err(exhausted(
                "complete staged Workshop attachment exceeds its reservation",
            ));
        }
        let mut work = Work::new(c)?;
        work.context.anchor.tick = work
            .context
            .anchor
            .tick
            .max(GameTick(self.parent_authority_tick_floor()));
        let import = work.take(workshop_prerequisite::OPEN_BYTES)?;
        let labor_read = work.take(144 * MIB)?;
        let order_read = work.take(16 * MIB)?;
        let clock_read = work.take(8 * MIB)?;
        let inventory_clock_read = work.take(8 * MIB)?;
        let absence = work.take(MIB)?;
        let final_read = work.take(MIB)?;
        let result = (|| {
            let (labor, orders) = self
                .parts
                .as_mut()
                .ok_or_else(|| denied("staged native custodians are unavailable"))?;
            if !labor
                .inventory(&work.narrow(&labor_read)?)?
                .records
                .is_empty()
                || !orders.inventory(&work.narrow(&order_read)?)?.is_empty()
            {
                return Err(denied(
                    "staged native custody already contains an original effect",
                ));
            }
            self.refresh_clock_view(&work.narrow(&clock_read)?)?;
            self.refresh_inventory_clock_view(&work.narrow(&inventory_clock_read)?)?;
            if self.clock.as_ref().is_some_and(ProductionClock::has_runs)
                || self
                    .inventory_clock
                    .as_ref()
                    .is_some_and(ProductionInventoryClock::has_runs)
            {
                return Err(denied(
                    "staged production clocks already contain original runs",
                ));
            }
            workshop_prerequisite::require_absent(
                &config.path(workshop_prerequisite::FILE_NAME),
                &work.narrow(&absence)?,
            )?;
            let workshop =
                SharedWorkshop::open(selected, config.read_guard(), &work.narrow(&import)?)?;
            self.retain_parent_authority_floor(workshop.high_tick());
            config.current()?;
            let mut current = work.narrow(&final_read)?;
            current.anchor.tick = current
                .anchor
                .tick
                .max(GameTick(self.parent_authority_tick_floor()));
            current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
            self.config = config;
            self.workshop = Some(workshop);
            // No fallible work follows this one-way policy installation. The
            // parent advances stage before attempting response publication.
            Ok(())
        })();
        if result.is_err() {
            self.abandon_preparation();
        }
        result
    }

    pub(crate) fn mark_parent_effect_uncertain(&mut self) {
        if self.has_native_execution() {
            self.parent_frontier_uncertain = true;
        }
    }

    pub(crate) fn parent_effect_uncertain(&self) -> bool {
        self.parent_frontier_uncertain
            || self.inactive_clock_may_be_running()
            || self
                .clock
                .as_ref()
                .is_some_and(ProductionClock::may_be_running)
            || self
                .inventory_clock
                .as_ref()
                .is_some_and(ProductionInventoryClock::may_be_running)
    }

    /// Exact caller-held recovery, without duplicating the full original plan,
    /// inspection rows, goals or historical run payloads into every parent reply.
    pub(crate) fn parent_recovery_reference(&self) -> Value {
        let workshop = self.workshop.as_ref().map(SharedWorkshop::status);
        let manager = self.manager.as_ref().map(SharedManager::status);
        let input = self.input_supply.as_ref().map(SharedInputSupply::status);
        json!({
            "native_custody":{
                "workforce_journal_id":self.workforce_journal.to_hex(),
                "order_journal_id":self.order_journal.to_hex(),
            },
            "recovery":self.recovery_json(),
            "clock_journal_id":self.clock.as_ref().and_then(ProductionClock::journal_id).map(|id|id.to_hex()),
            "inventory_clock_journal_id":self.inventory_clock.as_ref().and_then(ProductionInventoryClock::journal_id).map(|id|id.to_hex()),
            "workshop_prerequisite_checkpoint":workshop.as_ref().and_then(|value|value.get("checkpoint")),
            "manager_prerequisite_checkpoint":manager.as_ref().and_then(|value|value.get("checkpoint")),
            "input_supply_checkpoint":input.as_ref().and_then(|value|value.get("checkpoint")),
            "authority_tick_floor":self.parent_authority_tick_floor(),
            "effect_uncertain":self.parent_effect_uncertain(),
            "custody_initialization_may_be_partial":self.initialization_failed,
            "custody_currently_verified":false,
            "current_state_proven":false,"original_goal_proven":false,
            "automatic_redispatch":false,
        })
    }
}
