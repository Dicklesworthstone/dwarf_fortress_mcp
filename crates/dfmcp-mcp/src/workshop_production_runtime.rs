//! One session owns a completed Workshop original and then its finite production
//! original. There is no cross-family canonical transaction or automatic effect.
use crate::live_production_runtime as production;
pub(crate) use crate::live_production_runtime::Mode;
use crate::live_production_server::Action as ProductionAction;
use crate::semantic_workforce_runtime::{denied, exhausted, invalid};
use crate::workshop_construction_server::runtime::{
    self as workshop, Action as WorkshopAction, ProductionHandoff,
};
pub(crate) use crate::workshop_production_config::{Config, Stage};
use dfmcp_adapter::workshop_construction::journal::WorkshopMode;
use dfmcp_core::{
    Capability, Digest32, GameTick, OperationContext, RequestId, Result, RiskTier, SessionId,
    StateAnchor, WorkBudget,
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const MIB: u64 = 1024 * 1024;
const WORKSHOP_OPEN_BYTES: u64 = 512 * MIB;
const HANDOFF_BYTES: u64 = 256 * MIB;
const ATTACH_BYTES: u64 = 512 * MIB;
const INSPECT_BYTES: u64 = 512 * MIB;

fn within(inner: WorkBudget, outer: WorkBudget) -> bool {
    inner.max_bytes <= outer.max_bytes
        && inner.max_wall_millis <= outer.max_wall_millis
        && inner.max_entities <= outer.max_entities
        && inner.max_actions <= outer.max_actions
        && inner.max_output_tokens <= outer.max_output_tokens
        && inner.max_game_ticks <= outer.max_game_ticks
}

/// Reservations share the same absolute deadline. Each child receives only its
/// prepaid work and the remaining wall time; no phase starts another full clock.
struct Work {
    budget: WorkBudget,
    bytes: u64,
    until: Instant,
}
impl Work {
    fn new(budget: WorkBudget) -> Result<Self> {
        budget.validate()?;
        Ok(Self {
            bytes: budget.max_bytes,
            budget,
            until: Instant::now()
                .checked_add(Duration::from_millis(budget.max_wall_millis))
                .ok_or_else(|| exhausted("staged operation deadline overflow"))?,
        })
    }
    fn wall(&self) -> Result<u64> {
        self.until
            .checked_duration_since(Instant::now())
            .and_then(|remaining| u64::try_from(remaining.as_millis()).ok())
            .filter(|remaining| *remaining > 0)
            .ok_or_else(|| exhausted("staged operation deadline expired"))
    }
    fn reserve(&mut self, bytes: u64) -> Result<WorkBudget> {
        self.bytes = self
            .bytes
            .checked_sub(bytes)
            .ok_or_else(|| exhausted("staged operation byte-work exhausted"))?;
        let mut budget = self.budget;
        budget.max_bytes = bytes;
        budget.max_wall_millis = self.wall()?;
        Ok(budget)
    }
    fn narrow(&self, mut reserved: WorkBudget) -> Result<WorkBudget> {
        reserved.max_wall_millis = reserved.max_wall_millis.min(self.wall()?);
        Ok(reserved)
    }
    fn context(&self, c: &OperationContext, reserved: WorkBudget) -> Result<OperationContext> {
        let mut current = c.clone();
        let reserved = self.narrow(reserved)?;
        current.budget.max_bytes = reserved.max_bytes.min(c.budget.max_bytes);
        current.budget.max_wall_millis = reserved.max_wall_millis.min(c.budget.max_wall_millis);
        current.budget.max_game_ticks = reserved.max_game_ticks.min(c.budget.max_game_ticks);
        current.budget.max_entities = reserved.max_entities.min(c.budget.max_entities);
        current.budget.max_actions = reserved.max_actions.min(c.budget.max_actions);
        current.budget.max_output_tokens =
            reserved.max_output_tokens.min(c.budget.max_output_tokens);
        Ok(current)
    }
}

fn workshop_mode(mode: Mode) -> WorkshopMode {
    match mode {
        Mode::Control => WorkshopMode::Control,
        Mode::Reconcile => WorkshopMode::Recover,
        Mode::Offline => WorkshopMode::Offline,
    }
}

pub(crate) struct Runtime {
    config: Config,
    session: SessionId,
    mode: Mode,
    budget: WorkBudget,
    stage: Stage,
    request: u128,
    issued: Option<(Stage, OperationContext)>,
    authority_tick_floor: u64,
    workshop: Option<workshop::Runtime>,
    production: production::Runtime,
    handoff: Option<ProductionHandoff>,
    retained_workshop: Value,
    attached: bool,
}

impl Runtime {
    pub(crate) fn open(
        config: Config,
        mode: Mode,
        session: SessionId,
        budget: WorkBudget,
    ) -> Result<Self> {
        let mut work = Work::new(budget)?;
        config.current()?;
        if session == SessionId::NIL {
            return Err(invalid("staged session identity is absent"));
        }
        if budget.max_bytes > production::MAX_BYTES
            || budget.max_wall_millis > production::MAX_WALL_MILLIS
            || budget.max_output_tokens > production::MAX_OUTPUT_TOKENS
            || budget.max_entities < 4096
            || !(1..=5).contains(&budget.max_actions)
            || budget.max_game_ticks > 1200
        {
            return Err(exhausted(
                "staged session exceeds the bounded production contract",
            ));
        }
        let stage = config
            .recovery
            .map_or(Stage::Workshop, |recovery| recovery.stage);
        let initial_floor = config
            .recovery
            .map_or(0, |recovery| recovery.authority_tick_floor);
        let workshop_config = config.workshop_config();
        let mut workshop_budget = budget;
        if !workshop_config.clock_configured() {
            workshop_budget.max_game_ticks = 0;
        }
        let mut workshop_reservation = work.reserve(WORKSHOP_OPEN_BYTES)?;
        workshop_reservation.max_game_ticks = workshop_budget.max_game_ticks;

        // Every child parser/source/directory policy is checked before any
        // initialized owner can become reachable. Recovered post-Workshop
        // stages first need an Offline read to derive their exact donor input.
        let (workshop, mut production, handoff, retained_workshop, attached) = if stage
            == Stage::Workshop
        {
            let production_config = config.production_config(None)?;
            let mut production_budget = budget;
            if production_config.clock.is_none() && production_config.inventory_clock.is_none() {
                production_budget.max_game_ticks = 0;
            }
            let mut production_reservation = work.reserve(work.bytes)?;
            production_reservation.max_game_ticks = production_budget.max_game_ticks;
            // Empty native custodians open BEFORE construction is reachable.
            // No later transition may retry these initialization operations.
            let production = production::Runtime::open_for_parent(
                production_config,
                mode,
                session,
                production_budget,
                work.narrow(production_reservation)?,
                initial_floor,
            )?;
            let mut workshop = workshop::Runtime::open_for_parent(
                workshop_config,
                workshop_mode(mode),
                session,
                work.narrow(workshop_reservation)?,
                initial_floor.max(production.parent_authority_tick_floor()),
            )?;
            workshop.budget = workshop_budget;
            workshop.retain_parent_authority_floor(initial_floor);
            (Some(workshop), production, None, Value::Null, false)
        } else {
            let handoff_reservation = work.reserve(HANDOFF_BYTES)?;
            let production_reservation = work.reserve(work.bytes)?;
            let mut donor = workshop::Runtime::open_for_parent(
                workshop_config,
                WorkshopMode::Offline,
                session,
                work.narrow(workshop_reservation)?,
                initial_floor,
            )?;
            donor.budget = workshop_budget;
            donor.retain_parent_authority_floor(initial_floor);
            let handoff_context = work.context(&donor.context()?, handoff_reservation)?;
            let handoff = donor.production_handoff(&handoff_context)?;
            if config.recovery.and_then(|recovery| recovery.handoff_digest)
                != Some(handoff.digest())
            {
                return Err(denied(
                    "staged recovery does not name the exact settled Workshop handoff",
                ));
            }
            let retained = donor.parent_recovery_reference();
            // The old writer/ACK/Run file ownership is fully released before
            // the production donor importer opens those same exact files.
            drop(donor);
            let production_config = config.production_config(Some(&handoff.prerequisite_json()))?;
            let mut production_budget = budget;
            if production_config.clock.is_none() && production_config.inventory_clock.is_none() {
                production_budget.max_game_ticks = 0;
            }
            let mut opening = work.narrow(production_reservation)?;
            opening.max_game_ticks = production_budget.max_game_ticks;
            let production = production::Runtime::open_for_parent(
                production_config,
                mode,
                session,
                production_budget,
                opening,
                initial_floor.max(handoff.authority_tick_floor()),
            )?;
            (None, production, Some(handoff), retained, true)
        };
        let authority_tick_floor = initial_floor
            .max(
                workshop
                    .as_ref()
                    .map_or(0, workshop::Runtime::parent_authority_tick_floor),
            )
            .max(
                handoff
                    .as_ref()
                    .map_or(0, ProductionHandoff::authority_tick_floor),
            )
            .max(production.parent_authority_tick_floor());
        production.retain_parent_authority_floor(authority_tick_floor);
        config.current()?;
        work.wall()?;
        Ok(Self {
            config,
            session,
            mode,
            budget,
            stage,
            // Child opening contexts use request IDs one and two. All public
            // calls and both later phases share this single increasing owner.
            request: 2,
            issued: None,
            authority_tick_floor,
            workshop,
            production,
            handoff,
            retained_workshop,
            attached,
        })
    }

    pub(crate) fn session_id(&self) -> SessionId {
        self.session
    }
    pub(crate) fn mode(&self) -> Mode {
        self.mode
    }
    pub(crate) fn budget(&self) -> WorkBudget {
        self.budget
    }
    pub(crate) fn stage(&self) -> Stage {
        self.stage
    }

    fn floor(&self) -> u64 {
        self.authority_tick_floor
            .max(
                self.workshop
                    .as_ref()
                    .map_or(0, workshop::Runtime::parent_authority_tick_floor),
            )
            .max(self.production.parent_authority_tick_floor())
            .max(
                self.handoff
                    .as_ref()
                    .map_or(0, ProductionHandoff::authority_tick_floor),
            )
    }

    fn retain_floors(&mut self) {
        self.authority_tick_floor = self.floor();
        if let Some(workshop) = &mut self.workshop {
            workshop.retain_parent_authority_floor(self.authority_tick_floor);
        }
        self.production
            .retain_parent_authority_floor(self.authority_tick_floor);
    }

    pub(crate) fn context(&mut self) -> Result<OperationContext> {
        self.config.current()?;
        self.retain_floors();
        self.request = self
            .request
            .checked_add(1)
            .ok_or_else(|| exhausted("staged request identities exhausted"))?;
        let mut c = if self.stage == Stage::Workshop {
            self.workshop
                .as_mut()
                .ok_or_else(|| denied("staged Workshop owner unavailable"))?
                .context()?
        } else {
            self.production.context(false)?
        };
        c.request_id = RequestId::new(self.request);
        self.issued = Some((self.stage, c.clone()));
        Ok(c)
    }

    fn check(&self, c: &OperationContext) -> Result<()> {
        self.config.current()?;
        let (stage, issued) = self
            .issued
            .as_ref()
            .ok_or_else(|| denied("staged operation has no current owned context"))?;
        if *stage != self.stage
            || c.session_id != self.session
            || c.request_id != issued.request_id
            || c.anchor != issued.anchor
            || c.grants != issued.grants
            || !within(c.budget, issued.budget)
        {
            return Err(denied(
                "staged operation differs from its exact issued phase or authority",
            ));
        }
        let mut authority = c.clone();
        authority.anchor.tick = authority.anchor.tick.max(GameTick(self.floor()));
        authority.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)
    }

    fn finish<T>(&mut self, result: Result<T>) -> Result<T> {
        self.retain_floors();
        if result.is_err() {
            self.withdraw();
        }
        result
    }

    pub(crate) fn workshop_action(
        &mut self,
        action: WorkshopAction,
        c: &OperationContext,
    ) -> Result<Value> {
        let result = (|| {
            self.check(c)?;
            if self.stage != Stage::Workshop {
                return Err(denied(
                    "the completed Workshop phase cannot be entered again",
                ));
            }
            self.workshop
                .as_mut()
                .ok_or_else(|| denied("staged Workshop owner unavailable"))?
                .perform(action, c)
        })();
        self.finish(result)
    }

    pub(crate) fn production_action(
        &mut self,
        action: ProductionAction,
        c: &OperationContext,
    ) -> Result<Value> {
        let result = (|| {
            self.check(c)?;
            if self.stage != Stage::Production || !self.attached {
                return Err(denied(
                    "complete and explicitly hand off the original Workshop before production",
                ));
            }
            if matches!(
                &action,
                ProductionAction::Prepare(_)
                    | ProductionAction::ReviewPrepared(_)
                    | ProductionAction::PlanRun(..)
                    | ProductionAction::PlanInventoryRun(..)
                    | ProductionAction::Commit(..)
                    | ProductionAction::CommitRun(..)
                    | ProductionAction::CommitInventoryRun(..)
                    | ProductionAction::Wait
                    | ProductionAction::Cancel
                    | ProductionAction::CancelRun(..)
                    | ProductionAction::CancelInventoryRun(..)
            ) {
                self.production.mark_parent_effect_uncertain();
            }
            action
                .perform(&mut self.production, c)
                .map(|view| view.to_json())
        })();
        self.finish(result)
    }

    /// Explicit, one-way ownership transfer. This never initializes production
    /// stores, prepares an original, dispatches, unpauses or changes native keys.
    pub(crate) fn begin_production(&mut self, c: &OperationContext) -> Result<Value> {
        let result = self.begin_production_inner(c);
        self.finish(result)
    }

    fn begin_production_inner(&mut self, c: &OperationContext) -> Result<Value> {
        let mut work = Work::new(c.budget)?;
        self.check(c)?;
        if self.stage == Stage::Production {
            // A lost response can safely repeat the read-only phase transition.
            return self
                .production
                .inspect(&work.context(c, c.budget)?)
                .map(|view| view.to_json());
        }
        // Reserve every phase and final inspection before releasing any owner.
        let export = work.reserve(HANDOFF_BYTES)?;
        let attach = work.reserve(ATTACH_BYTES)?;
        let inspect = work.reserve(INSPECT_BYTES)?;
        if self.stage == Stage::Workshop {
            let workshop = self
                .workshop
                .as_mut()
                .ok_or_else(|| denied("staged Workshop owner unavailable"))?;
            let handoff = workshop.production_handoff(&work.context(c, export)?)?;
            self.authority_tick_floor = self
                .authority_tick_floor
                .max(handoff.authority_tick_floor());
            self.retained_workshop = workshop.parent_recovery_reference();
            self.handoff = Some(handoff);
            // Retain phase + exact handoff BEFORE dropping the writer. A later
            // refusal has only a read-only import retry, never construction.
            self.stage = Stage::Transition;
            drop(self.workshop.take());
        }
        self.retain_floors();
        let handoff = self
            .handoff
            .as_ref()
            .ok_or_else(|| denied("staged transition lost its verified handoff"))?;
        if !self.attached {
            let config = self
                .config
                .production_config(Some(&handoff.prerequisite_json()))?;
            let mut current = self.production.context(c.cancellation_requested)?;
            current.request_id = c.request_id;
            let current = work.context(&current, attach)?;
            self.production
                .attach_parent_workshop(config, handoff, &current)?;
            self.attached = true;
        }
        // This precedes fallible inspection/publication; no output failure may
        // restore the old writer, permit or stage.
        self.stage = Stage::Production;
        let mut current = self.production.context(c.cancellation_requested)?;
        current.request_id = c.request_id;
        let value = self
            .production
            .inspect(&work.context(&current, inspect)?)?
            .to_json();
        self.config.current()?;
        work.wall()?;
        Ok(value)
    }

    pub(crate) fn inspect(&mut self, c: &OperationContext) -> Result<Value> {
        match self.stage {
            Stage::Workshop => self.workshop_action(WorkshopAction::Inspect, c),
            Stage::Production => self.production_action(ProductionAction::Inspect, c),
            Stage::Transition => {
                let result = (|| {
                    self.check(c)?;
                    let mut value = self.production.inspect(c)?.to_json();
                    value["transition_pending"] = json!(true);
                    Ok(value)
                })();
                self.finish(result)
            }
        }
    }

    pub(crate) fn withdraw(&mut self) {
        self.retain_floors();
        if let Some(workshop) = &mut self.workshop {
            workshop.withdraw();
        }
        self.production.abandon_preparation();
    }

    pub(crate) fn canonical_anchor(&self) -> Option<StateAnchor> {
        let anchor = self.production.anchor();
        (self.stage == Stage::Production && anchor.state_hash != Digest32::ZERO).then_some(anchor)
    }

    pub(crate) fn effect_uncertain(&self) -> bool {
        self.workshop
            .as_ref()
            .is_some_and(workshop::Runtime::parent_effect_uncertain)
            || self.production.parent_effect_uncertain()
    }

    /// Caller-held checkpoint only. Neither this object nor its immutable
    /// handoff digest claims a durable acknowledgement of later source reads.
    pub(crate) fn recovery(&self) -> Value {
        let mut parent = json!({
            "stage":self.stage.as_str(),
            "authority_tick_floor":self.floor(),
        });
        let handoff = self.handoff.as_ref().map(|handoff| {
            parent["handoff_digest"] = json!(handoff.digest().to_hex());
            let mut cached = handoff.to_json();
            cached["retained_custody_verified"] = json!(false);
            cached["verified_at_export"] = json!(true);
            cached["verification_scope"] = json!("historical_completed_handoff");
            if cached["clock"]["enabled"].as_bool() == Some(true) {
                cached["clock"]["verified"] = json!(false);
            }
            cached
        });
        json!({
            "schema":"dfmcp.workshop-production-recovery/1",
            "parent":parent,
            "workshop":self.workshop.as_ref()
                .map_or_else(||self.retained_workshop.clone(), workshop::Runtime::parent_recovery_reference),
            "production":self.production.parent_recovery_reference(),
            "production_handoff":handoff,
            "effect_uncertain":self.effect_uncertain(),
            "external_checkpoint_required":true,"parent_journal":false,
            "cross_stage_canonical_continuity":"unknown",
            "current_workshop_completion_proven":false,
            "complete_cross_stage_goal_proven":false,
            "automatic_redispatch":false,
        })
    }
}

#[cfg(test)]
#[path = "workshop_production_runtime_tests.rs"]
mod tests;
