//! Exact clock recovery without opening native designation or parent custody.
//!
//! The original monitor is read-only evidence needed to reconstruct the sealed
//! clock target. This owner can inspect or settle that original run, but never
//! acquire terrain, prepare another interval or recreate a dispatch permit.
use super::{Action, Query, Work, clock, error, exhausted, monitor, presentation, runtime};
use dfmcp_adapter::bounded_run::journal::RunMode;
use dfmcp_adapter::bounded_run::rpc::RunSource;
use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
use dfmcp_adapter::dig_blueprint::clock::{Clock, ClockView, OPERATION_BYTES};
use dfmcp_adapter::dig_blueprint::monitor::store::MonitorStore;
use dfmcp_adapter::dig_designation::journal::DigBinding;
use dfmcp_core::{
    Capability, CapabilityGrant, Digest32, ErrorCode, FortressId, GameTick, OperationContext,
    RequestId, Result, RiskTier, SessionId, WorkBudget,
};
use serde_json::{Value, json};
use std::time::Instant;

const CONNECT_BYTES: u64 = 4 * 1024 * 1024;
/// Both histories are checked before and after rendering during opening.
pub(super) const INSPECT_BYTES: u64 = presentation::OUTPUT_BYTES
    + 2 * super::VIEW_BYTES
    + 2 * monitor::FINAL_BYTES
    + 2 * clock::VIEW_BYTES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Offline,
    Recovery,
}
impl Mode {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Offline => "clock-offline",
            Self::Recovery => "clock-recovery",
        }
    }
    pub(super) fn run_mode(self) -> RunMode {
        match self {
            Self::Offline => RunMode::Offline,
            Self::Recovery => RunMode::Control,
        }
    }
}

fn denied() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::CapabilityDenied,
        "independent clock recovery permits history queries and original outcome recovery only",
    )
}
fn exact_error() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::StaleAnchor,
        "clock recovery differs from its exact original monitor or acknowledged custody",
    )
}
fn failure(cause: &dfmcp_core::DfmcpError, effect_may_have_occurred: bool) -> Value {
    let mut result = presentation::failure(cause);
    result["effect_may_have_occurred"] = json!(effect_may_have_occurred);
    result
}
fn restricted(grants: &[CapabilityGrant], mode: Mode) -> Vec<CapabilityGrant> {
    grants
        .iter()
        .filter(|grant| {
            grant.capability == Capability::Query
                || (mode == Mode::Recovery && grant.capability == Capability::ControlClock)
        })
        .cloned()
        .collect()
}
fn common_budget(a: WorkBudget, b: WorkBudget) -> WorkBudget {
    WorkBudget {
        max_wall_millis: a.max_wall_millis.min(b.max_wall_millis),
        max_game_ticks: 0,
        max_bytes: a.max_bytes.min(b.max_bytes),
        max_entities: a.max_entities.min(b.max_entities),
        max_output_tokens: a.max_output_tokens.min(b.max_output_tokens),
        max_actions: a.max_actions.min(b.max_actions),
    }
}

pub(super) struct RecoveryState<B: EffectJournalStorage, C: EffectJournalStorage> {
    monitor: MonitorStore<B>,
    clock: Clock<C>,
    id: SessionId,
    request: u128,
    budget: WorkBudget,
    grants: Vec<CapabilityGrant>,
    native_grants: Vec<CapabilityGrant>,
    authority_tick: u64,
    mode: Mode,
}

impl<B: EffectJournalStorage, C: EffectJournalStorage> RecoveryState<B, C> {
    /// Both supplied owners must already have opened the configured exact
    /// recovery pins. This repeats their current custody and source association.
    /// Native source authority is retained separately from canonical grants.
    pub(super) fn new(
        monitor: MonitorStore<B>,
        mut clock: Clock<C>,
        c: &OperationContext,
        n: &OperationContext,
        config: &runtime::Config,
        mode: Mode,
    ) -> Result<Self> {
        if c.budget.max_game_ticks != 0 || n.budget.max_game_ticks != 0 {
            return Err(denied());
        }
        config.matches(monitor.spec().parent().binding())?;
        let selected_monitor = config.monitor.as_ref().ok_or_else(denied)?;
        let selected_clock = config.clock.as_ref().ok_or_else(denied)?;
        if selected_monitor.initialize
            || selected_clock.initialize
            || selected_monitor
                .recovery
                .is_none_or(|pin| pin.monitor_id != monitor.spec().id())
            || selected_clock
                .recovery
                .is_none_or(|pin| pin.journal_id != clock.recovery().journal_id)
            || !(clock.mode() == mode.run_mode()
                || (mode == Mode::Recovery && clock.mode() == RunMode::Recover))
            || clock.binding() != &selected_clock.binding
            || clock.spec().canonical_bytes() != monitor.spec().canonical_bytes()
        {
            return Err(exact_error());
        }
        selected_monitor.matches(monitor.spec())?;
        selected_clock.matches(monitor.spec())?;
        clock.abandon_preparation();
        // A gate can disappear between grant issuance and opening. The fixed
        // private Recover mode must never retain or later advertise Clock.
        let grant_mode = if mode == Mode::Recovery && clock.is_control() {
            Mode::Recovery
        } else {
            Mode::Offline
        };
        let mut state = Self {
            authority_tick: monitor
                .high_tick()
                .max(clock.high_tick())
                .max(c.anchor.tick.get())
                .max(n.anchor.tick.get()),
            monitor,
            clock,
            id: c.session_id,
            request: c.request_id.get(),
            budget: c.budget,
            grants: restricted(&c.grants, grant_mode),
            native_grants: restricted(&n.grants, grant_mode),
            mode,
        };
        let (current, native) = state.authorize(c, n, false)?;
        state.inspect(&current, &native)?;
        Ok(state)
    }

    pub(super) fn id(&self) -> SessionId {
        self.id
    }
    pub(super) fn binding(&self) -> &DigBinding {
        self.clock.spec().parent().binding()
    }
    pub(super) fn mode(&self) -> Mode {
        self.mode
    }
    pub(super) fn high_tick(&self) -> u64 {
        self.authority_tick
            .max(self.monitor.high_tick())
            .max(self.clock.high_tick())
    }
    /// Restore only the already admitted request ceilings after opening work.
    pub(super) fn set_budget(&mut self, mut budget: WorkBudget) {
        budget.max_game_ticks = 0;
        self.budget = budget;
    }
    pub(super) fn context(&mut self, wall: Option<u64>) -> Result<OperationContext> {
        self.request = self.request.checked_add(1).ok_or_else(exhausted)?;
        let mut budget = self.budget;
        if let Some(wall) = wall {
            budget.max_wall_millis = budget.max_wall_millis.min(wall);
        }
        budget.max_game_ticks = 0;
        budget.validate()?;
        let mut context = super::context(
            self.id,
            RequestId::new(self.request),
            self.binding(),
            self.high_tick(),
            budget,
            false,
        );
        context.grants = self.grants.clone();
        Ok(context)
    }

    pub(super) fn monitor_reference(&self) -> Value {
        let mut reference = monitor::reference(&self.monitor);
        reference["authority_tick_floor"] = json!(self.high_tick());
        historical_monitor(&mut reference);
        reference
    }
    pub(super) fn clock_reference(&self) -> Value {
        let mut reference = clock::reference(&self.clock);
        reference["authority_tick_floor"] = json!(self.high_tick());
        historical_clock(&mut reference);
        reference
    }

    fn retain_floor(&mut self) {
        self.authority_tick = self.high_tick();
        self.clock.abandon_preparation();
    }
    fn grant_mode(&self) -> Mode {
        if self.mode == Mode::Recovery && self.clock.is_control() {
            Mode::Recovery
        } else {
            Mode::Offline
        }
    }
    fn current(&mut self, c: &OperationContext) -> Result<OperationContext> {
        if c.session_id != self.id || c.anchor.fortress_id != self.binding().fortress_id() {
            return Err(denied());
        }
        self.authority_tick = self.high_tick().max(c.anchor.tick.get());
        let mut current = c.clone();
        current.anchor.tick = GameTick(self.high_tick());
        current.budget = common_budget(current.budget, self.budget);
        current.grants = restricted(&current.grants, self.grant_mode());
        Ok(current)
    }
    fn authorize(
        &mut self,
        c: &OperationContext,
        n: &OperationContext,
        effect: bool,
    ) -> Result<(OperationContext, OperationContext)> {
        if n.session_id != self.id
            || c.request_id != n.request_id
            || n.anchor.fortress_id != FortressId::NIL
            || (effect && (self.mode != Mode::Recovery || !self.clock.is_control()))
        {
            return Err(denied());
        }
        let mut current = self.current(c)?;
        self.authority_tick = self.high_tick().max(n.anchor.tick.get());
        current.anchor.tick = GameTick(self.high_tick());
        let mut native = n.clone();
        native.anchor.tick = current.anchor.tick;
        let budget = common_budget(current.budget, native.budget);
        current.budget = budget;
        native.budget = budget;
        native.grants = restricted(&native.grants, self.grant_mode());
        for retained in [false, true] {
            let mut canonical_check = current.clone();
            let mut native_check = native.clone();
            if retained {
                canonical_check.grants = self.grants.clone();
                native_check.grants = self.native_grants.clone();
            }
            self.clock.spec().authorize(&canonical_check, false)?;
            native_check.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
            if effect {
                canonical_check.authorize(
                    Capability::ControlClock,
                    RiskTier::Guarded,
                    &[],
                    None,
                )?;
                native_check.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
            }
        }
        Ok((current, native))
    }
    fn bounded(
        &mut self,
        work: &Work,
        reserved: &OperationContext,
    ) -> Result<(OperationContext, OperationContext)> {
        let mut current = self.current(reserved)?;
        current.budget.max_wall_millis = work.current(&current)?.budget.max_wall_millis;
        let native = clock::native_context(&current, &self.native_grants)?;
        self.authorize(&current, &native, false)
    }
    fn exact(&self, id: Digest32) -> Result<()> {
        if id == self.clock.spec().id() {
            Ok(())
        } else {
            Err(exact_error())
        }
    }
    fn inspect_clock(&mut self, work: &Work, reserved: &OperationContext) -> Result<ClockView> {
        let (current, native) = self.bounded(work, reserved)?;
        let result = self.clock.inspect(&current, &native);
        self.retain_floor();
        let view = result?;
        if !view.verified {
            return Err(error(
                ErrorCode::CorruptLedger,
                "original clock custody remains unverified",
            ));
        }
        Ok(view)
    }
    fn inspect_monitor(&mut self, work: &Work, reserved: &OperationContext) -> Result<()> {
        let (current, _) = self.bounded(work, reserved)?;
        let result = self.monitor.verify(&current);
        self.retain_floor();
        result
    }
    fn monitor_display(&self, offset: usize, limit: usize) -> Result<Value> {
        let mut value = monitor::display(&self.monitor, false, false, offset, limit)?;
        historical_monitor(&mut value);
        Ok(value)
    }
    fn clock_display(&self, view: &ClockView, offset: usize, limit: usize) -> Result<Value> {
        let mut value = clock::display(view, offset, limit)?;
        historical_clock(&mut value);
        Ok(value)
    }

    fn retain(&self, mut result: Value) -> Value {
        if result.get("blueprint_monitor").is_none() {
            result["blueprint_monitor"] = self.monitor_reference();
        }
        if result.get("blueprint_clock").is_none() {
            result["blueprint_clock"] = self.clock_reference();
        }
        result["clock_recovery_only"] = json!(true);
        result["profile"] = json!(self.mode.as_str());
        result["evidence_scope"] = json!("this_original_monitor_and_clock_history_only");
        result["native_designation_custody_opened"] = json!(false);
        result["blueprint_parent_custody_opened"] = json!(false);
        result["original_inventory_verified"] = json!(false);
        result["original_designations_verified"] = json!(false);
        result["current_terrain_proven"] = json!(false);
        result["current_pause_proven"] = json!(false);
        result["fresh_sample_acquired"] = json!(false);
        result["sampled_excavation_condition_proven"] = json!(false);
        result["new_run_permitted"] = json!(false);
        result["retry_commit_permitted"] = json!(false);
        result["dispatch_authorized"] = json!(false);
        result
    }
    fn packet(&self, op: &str, c: &OperationContext, result: Value) -> String {
        let mut current = c.clone();
        current.anchor.tick = current.anchor.tick.max(GameTick(self.high_tick()));
        current.budget.max_game_ticks = 0;
        current.grants = restricted(&current.grants, self.grant_mode());
        presentation::packet(
            op,
            self.retain(result),
            Some(&current),
            Some(self.binding()),
            None,
            None,
            None,
        )
    }

    /// Opening-only inspection. The caller supplies one already narrowed wall
    /// window and prepays INSPECT_BYTES; this method contacts no native source.
    pub(super) fn inspect(&mut self, c: &OperationContext, n: &OperationContext) -> Result<Value> {
        let (current, _) = self.authorize(c, n, false)?;
        let mut work = Work::new(&current, Instant::now())?;
        let monitor_before = work.take(&current, monitor::FINAL_BYTES)?;
        let clock_before = work.take(&current, clock::VIEW_BYTES)?;
        let monitor_after = work.take(&current, monitor::FINAL_BYTES)?;
        let clock_after = work.take(&current, clock::VIEW_BYTES)?;
        self.inspect_monitor(&work, &monitor_before)?;
        let view = self.inspect_clock(&work, &clock_before)?;
        let result = self.retain(json!({"ok":true,
            "blueprint_monitor":self.monitor_display(0,16)?,
            "blueprint_clock":self.clock_display(&view,0,8)?,
            "native_calls":0,"game_mutations":0,"live_health_checked":false}));
        self.inspect_monitor(&work, &monitor_after)?;
        let final_view = self.inspect_clock(&work, &clock_after)?;
        if final_view.recovery != view.recovery {
            return Err(exact_error());
        }
        work.current(&current)?;
        Ok(result)
    }

    fn perform<I: clock::ClockIo<C>>(
        &mut self,
        c: &OperationContext,
        work: &mut Work,
        action: Action,
        io: &mut I,
        effect_started: &mut bool,
    ) -> Result<Value> {
        match action {
            Action::Query(Query::BlueprintClock {
                monitor_id,
                expected_head,
                offset,
                limit,
            }) => {
                self.exact(presentation::digest(&monitor_id)?)?;
                let reserved = work.take(c, clock::VIEW_BYTES)?;
                io.checkpoint(&work.current(c)?, false)?;
                let view = self.inspect_clock(work, &reserved)?;
                if expected_head
                    .as_deref()
                    .map(presentation::digest)
                    .transpose()?
                    .is_some_and(|head| head != view.recovery.head)
                {
                    return Err(exact_error());
                }
                Ok(
                    json!({"ok":true,"blueprint_clock":self.clock_display(&view,offset.map_or(0, |value| value),limit.map_or(8, |value| value))?,
                    "native_calls":0,"game_mutations":0}),
                )
            }
            Action::Query(Query::BlueprintMonitor {
                monitor_id,
                expected_head,
                offset,
                limit,
            }) => {
                self.exact(presentation::digest(&monitor_id)?)?;
                let reserved = work.take(c, monitor::FINAL_BYTES)?;
                io.checkpoint(&work.current(c)?, false)?;
                self.inspect_monitor(work, &reserved)?;
                if expected_head
                    .as_deref()
                    .map(presentation::digest)
                    .transpose()?
                    .is_some_and(|head| head != self.monitor.head())
                {
                    return Err(exact_error());
                }
                Ok(
                    json!({"ok":true,"blueprint_monitor":self.monitor_display(offset.map_or(0, |value| value),limit.map_or(16, |value| value))?,
                    "native_calls":0,"game_mutations":0}),
                )
            }
            Action::Inventory => {
                let monitor_context = work.take(c, monitor::FINAL_BYTES)?;
                let clock_context = work.take(c, clock::VIEW_BYTES)?;
                io.checkpoint(&work.current(c)?, false)?;
                self.inspect_monitor(work, &monitor_context)?;
                let view = self.inspect_clock(work, &clock_context)?;
                Ok(
                    json!({"ok":true,"blueprint_monitor":self.monitor_display(0,16)?,
                    "blueprint_clock":self.clock_display(&view,0,8)?,"live_health_checked":false,
                    "native_calls":0,"game_mutations":0}),
                )
            }
            Action::Query(Query::Schema {}) => {
                io.checkpoint(&work.current(c)?, false)?;
                Ok(
                    json!({"ok":true,"query_schema":query_schema()?,"native_calls":0,"game_mutations":0}),
                )
            }
            Action::WaitClock(id) | Action::CancelClock(id) => {
                let cancel = matches!(action, Action::CancelClock(_));
                self.exact(id)?;
                if self.mode != Mode::Recovery {
                    return Err(denied());
                }
                let inspection = work.take(c, clock::VIEW_BYTES)?;
                let connection = work.take(c, CONNECT_BYTES)?;
                let operation = work.take(c, OPERATION_BYTES)?;
                let (current, native) = self.bounded(work, &inspection)?;
                self.authorize(&current, &native, cancel)?;
                io.checkpoint(&current, cancel)?;
                let before = self.inspect_clock(work, &inspection)?;
                let needs_source = if cancel {
                    before.cancellation_needs_source()
                } else {
                    before.needs_reconciliation()
                };
                let mut source = if needs_source {
                    let (current, native) = self.bounded(work, &connection)?;
                    self.authorize(&current, &native, cancel)?;
                    io.checkpoint(&current, cancel)?;
                    Some(io.connect(&native, cancel)?)
                } else {
                    None
                };
                let (current, native) = self.bounded(work, &operation)?;
                self.authorize(&current, &native, cancel)?;
                io.checkpoint(&current, cancel)?;
                let outcome = if cancel {
                    // Retain uncertainty before entering the original safety
                    // effect boundary. No local retirement or read sets it.
                    *effect_started = needs_source;
                    self.clock.cancel(
                        source.as_mut().map(|source| source as &mut dyn RunSource),
                        &current,
                        &native,
                    )
                } else if let Some(source) = source.as_mut() {
                    self.clock.reconcile(source, &current, &native)
                } else {
                    Ok(before)
                };
                self.retain_floor();
                let view = outcome?;
                Ok(
                    json!({"ok":true,"blueprint_clock":self.clock_display(&view,0,8)?,
                    "native_source_contacted":needs_source,"safety_cancel_requested":cancel,
                    "monitor_history_used_as_effect_precondition":false}),
                )
            }
            _ => Err(denied()),
        }
    }

    pub(super) fn run_action<I: clock::ClockIo<C>>(
        &mut self,
        c: OperationContext,
        op: &str,
        action: Result<Action>,
        started: Instant,
        io: &mut I,
    ) -> String {
        let current = match clock::native_context(&c, &self.native_grants)
            .and_then(|native| self.authorize(&c, &native, false))
        {
            Ok((current, _)) => current,
            Err(cause) => return self.packet(op, &c, failure(&cause, false)),
        };
        let mut work = match Work::new(&current, started) {
            Ok(work) => work,
            Err(cause) => return self.packet(op, &current, failure(&cause, false)),
        };
        let final_monitor_needed = matches!(
            action,
            Ok(Action::Inventory | Action::Query(Query::BlueprintMonitor { .. }))
        );
        let cancel_requested =
            self.mode == Mode::Recovery && matches!(&action, Ok(Action::CancelClock(_)));
        let final_contexts = (|| {
            let final_clock = work.take(&current, clock::VIEW_BYTES)?;
            let final_monitor = if final_monitor_needed {
                Some(work.take(&current, monitor::FINAL_BYTES)?)
            } else {
                None
            };
            Ok((final_clock, final_monitor))
        })();
        let (final_clock, final_monitor) = match final_contexts {
            Ok(contexts) => contexts,
            Err(cause) => return self.packet(op, &current, failure(&cause, false)),
        };
        let mut effect_started = false;
        let outcome = action
            .and_then(|action| self.perform(&current, &mut work, action, io, &mut effect_started));
        self.retain_floor();
        let mut final_context = current;
        final_context.anchor.tick = GameTick(self.high_tick());
        let expected = self.clock.recovery();
        let published_monitor = outcome
            .as_ref()
            .is_ok_and(|value| value["blueprint_monitor"]["history_verified"] == true);
        let rendered = self.packet(
            op,
            &final_context,
            match outcome {
                Ok(value) => value,
                Err(cause) => failure(&cause, effect_started),
            },
        );
        let publication = (|| {
            if rendered.len() as u64 > presentation::OUTPUT_BYTES {
                return Err(exhausted());
            }
            let (current, native) = self.bounded(&work, &final_clock)?;
            io.checkpoint(&current, cancel_requested)?;
            self.authorize(&current, &native, cancel_requested)?;
            if published_monitor {
                let reserved = final_monitor.as_ref().ok_or_else(exhausted)?;
                self.inspect_monitor(&work, reserved)?;
            }
            let view = self.inspect_clock(&work, &final_clock)?;
            if view.recovery != expected {
                return Err(exact_error());
            }
            work.current(&final_context)?;
            Ok(())
        })();
        match publication {
            Ok(()) => rendered,
            Err(cause) => {
                self.retain_floor();
                final_context.anchor.tick = GameTick(self.high_tick());
                self.packet(op, &final_context, failure(&cause, effect_started))
            }
        }
    }

    /// Normal close requires intact settled clock custody. Recovery release
    /// keeps unverified identities even after either history or policy changes;
    /// its callback must check only runtime cancellation and the request window.
    pub(super) fn close(
        &mut self,
        c: OperationContext,
        release: bool,
        started: Instant,
        mut checkpoint: impl FnMut(&OperationContext) -> Result<()>,
    ) -> String {
        let op = "fortress.cancel";
        let current = match self.current(&c) {
            Ok(current) => current,
            Err(cause) => return self.packet(op, &c, failure(&cause, false)),
        };
        let mut work = match Work::new(&current, started) {
            Ok(work) => work,
            Err(cause) => return self.packet(op, &current, failure(&cause, false)),
        };
        let expected = self.clock.recovery();
        let final_clock = if release {
            None
        } else {
            match work.take(&current, clock::VIEW_BYTES) {
                Ok(reserved) => Some(reserved),
                Err(cause) => return self.packet(op, &current, failure(&cause, false)),
            }
        };
        let mut final_context = current.clone();
        let outcome = (|| {
            if current.cancellation_requested {
                return Err(error(
                    ErrorCode::CancellationRequested,
                    "clock recovery close cancelled",
                ));
            }
            checkpoint(&work.current(&current)?)?;
            let view = if release {
                self.clock_reference()
            } else {
                let reserved = work.take(&current, clock::VIEW_BYTES)?;
                let view = self.inspect_clock(&work, &reserved)?;
                if view.pending || view.unresolved {
                    return Err(error(
                        ErrorCode::EffectIndeterminate,
                        "unsettled original clock requires explicit recovery release",
                    ));
                }
                self.clock_display(&view, 0, 8)?
            };
            Ok(json!({"ok":true,"scope":"session","closed":true,
                "release_for_recovery":release,"blueprint_clock":view,
                "effects_cancelled":false,"monitor_cancelled":false,
                "history_erased":false,"native_quiescence_proven":false}))
        })();
        self.retain_floor();
        final_context.anchor.tick = GameTick(self.high_tick());
        let output = self.packet(
            op,
            &final_context,
            match outcome {
                Ok(value) => value,
                Err(cause) => failure(&cause, false),
            },
        );
        let publication = (|| {
            if output.len() as u64 > presentation::OUTPUT_BYTES {
                return Err(exhausted());
            }
            checkpoint(&work.current(&final_context)?)?;
            if let Some(reserved) = final_clock {
                let view = self.inspect_clock(&work, &reserved)?;
                if view.recovery != expected {
                    return Err(exact_error());
                }
            }
            work.current(&final_context)?;
            Ok(())
        })();
        match publication {
            Ok(()) => output,
            Err(cause) => self.packet(op, &final_context, failure(&cause, false)),
        }
    }
}

fn historical_monitor(value: &mut Value) {
    value["original_designations_verified"] = json!(false);
    value["historical_evidence_only"] = json!(true);
    value["fresh_sample_acquired"] = json!(false);
    value["sampled_excavation_condition_proven"] = json!(false);
}
fn historical_clock(value: &mut Value) {
    value["fresh_goal_sampling_permitted"] = json!(false);
    value["historical_evidence_only"] = json!(true);
    value["current_pause_proven"] = json!(false);
    value["sampled_excavation_condition_proven"] = json!(false);
    value["new_run_permitted"] = json!(false);
    value["review_restored"] = json!(false);
    if let Some(active) = value
        .get_mut("active_run")
        .filter(|active| active.is_object())
    {
        active["review_seal"] = Value::Null;
    }
    if let Some(rows) = value
        .get_mut("runs_page")
        .and_then(|page| page.get_mut("runs"))
        .and_then(Value::as_array_mut)
    {
        for row in rows {
            row["review_seal"] = Value::Null;
        }
    }
}
fn query_schema() -> Result<Value> {
    let mut schema: Value = serde_json::from_str(include_str!(
        "../../../../schemas/dig_control_query.json"
    ))
    .map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "invalid clock recovery schema",
        )
    })?;
    let alternatives = schema
        .get_mut("oneOf")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "query alternatives absent",
            )
        })?;
    alternatives.retain(|value| {
        matches!(
            value["properties"]["mode"]["const"].as_str(),
            Some("schema" | "blueprint_clock" | "blueprint_monitor")
        )
    });
    if alternatives.len() != 3 {
        return Err(error(
            ErrorCode::InternalInvariantViolation,
            "clock recovery schema changed",
        ));
    }
    schema["$id"] = json!("dfmcp.blueprint-clock-recovery-query/1");
    Ok(schema)
}
