//! Recover the exact original construction clock from monitor and run custody.
//!
//! This owner never opens placement or parent files, acquires construction
//! evidence, changes the monitor, or prepares/dispatches a new interval.
use super::{
    Action, Query, Work, allocation, clock, completion, error, exhausted, presentation, runtime,
};
use dfmcp_adapter::bounded_run::journal::RunMode;
use dfmcp_adapter::bounded_run::rpc::RunSource;
use dfmcp_adapter::build_placement::BuildBinding;
use dfmcp_adapter::build_placement::journal::BuildMode;
use dfmcp_adapter::construction_plan::clock::{Clock, ClockView, OPERATION_BYTES};
use dfmcp_adapter::construction_plan::store::{MAX_STORE_BYTES, MonitorStore, Recovery};
use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
use dfmcp_core::{
    Capability, CapabilityGrant, Digest32, ErrorCode, FortressId, GameTick, ObservationCursor,
    OperationContext, RequestId, Result, RiskTier, SessionId, WorkBudget,
};
use serde_json::{Value, json};
use std::time::Instant;

const CONNECT_BYTES: u64 = 4 * 1024 * 1024;
const MONITOR_OVERHEAD: u64 = 8192;

fn monitor_reserve(length: usize) -> Result<u64> {
    if length > MAX_STORE_BYTES {
        return Err(exhausted());
    }
    (length as u64)
        .checked_add(MONITOR_OVERHEAD)
        .ok_or_else(exhausted)
}

/// Includes the shared handler's output/inventory reservations and exact reads
/// of both histories before and after constructing the complete opening result.
pub(super) fn inspect_reserve(monitor_bytes: usize) -> Result<u64> {
    monitor_reserve(monitor_bytes)?
        .checked_mul(2)
        .and_then(|bytes| bytes.checked_add(2 * clock::VIEW_BYTES))
        .and_then(|bytes| bytes.checked_add(presentation::OUTPUT_BYTES + 2 * super::VIEW_BYTES))
        .ok_or_else(exhausted)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Offline,
    Recovery,
}
impl Mode {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Offline => "clock-offline",
            Self::Recovery => "clock-recover",
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
        "independent construction clock recovery permits inspection and original outcome recovery only",
    )
}
fn exact_error() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::StaleAnchor,
        "construction clock recovery differs from its original monitor or acknowledged custody",
    )
}
fn failure(cause: &dfmcp_core::DfmcpError, effect_may_have_occurred: bool) -> Value {
    let mut result = presentation::failure(cause);
    result["scope"] = json!("clock");
    result["clock_recovery_only"] = json!(true);
    result["effect_may_have_occurred"] = json!(effect_may_have_occurred);
    result["goal_sample_published"] = json!(false);
    result
}
fn restricted(grants: &[CapabilityGrant], control: bool) -> Vec<CapabilityGrant> {
    grants
        .iter()
        .filter(|grant| {
            grant.capability == Capability::Query
                || (control && grant.capability == Capability::ControlClock)
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
    selected: clock::Config,
    monitor_pin: Recovery,
    id: SessionId,
    request: u128,
    budget: WorkBudget,
    grants: Vec<CapabilityGrant>,
    native_grants: Vec<CapabilityGrant>,
    authority_tick: u64,
    mode: Mode,
}

impl<B: EffectJournalStorage, C: EffectJournalStorage> RecoveryState<B, C> {
    /// The shell opens the monitor read-only and supplies Clock::new with the
    /// configured exact run recovery pin. Neither original placement path is
    /// opened. This constructor repeats monitor ancestry and both current files.
    pub(super) fn new(
        monitor: MonitorStore<B>,
        mut clock: Clock<C>,
        c: &OperationContext,
        n: &OperationContext,
        config: &runtime::Config,
        mode: Mode,
    ) -> Result<Self> {
        if c.budget.max_game_ticks != 0
            || n.budget.max_game_ticks != 0
            || !config.clock_only
            || config.completion_only
            || config.completion_path.is_none()
            || config.mode
                != match mode {
                    Mode::Offline => BuildMode::Offline,
                    Mode::Recovery => BuildMode::Recover,
                }
        {
            return Err(denied());
        }
        let monitor_pin = config.clock_monitor_recovery.ok_or_else(denied)?;
        monitor_pin.validate()?;
        let selected = config.clock.as_ref().ok_or_else(denied)?;
        let definition = monitor.definition();
        let origin = definition.origin();
        let binding = origin.definition().binding();
        config.matches(binding)?;
        selected.matches(definition)?;
        if selected.initialize
            || monitor_pin.monitor_id != definition.id()
            || selected
                .recovery
                .is_none_or(|pin| pin.journal_id != clock.recovery().journal_id)
            || !(clock.mode() == mode.run_mode()
                || (mode == Mode::Recovery && clock.mode() == RunMode::Recover))
            || clock.binding() != &selected.binding
            || clock.definition().canonical_bytes() != definition.canonical_bytes()
            || monitor.first_source().is_none_or(|source| {
                source.furniture_generation != binding.generation()
                    || &source.operations != clock.operations()
            })
            || origin.child_identity().path != config.path
            || config.batch_path.as_ref() != Some(&origin.parent_identity().path)
        {
            return Err(exact_error());
        }
        clock.abandon_preparation();
        let control = mode == Mode::Recovery && clock.is_control();
        let original_floor = origin
            .receipts()
            .iter()
            .flat_map(|record| {
                [
                    record.plan().before().tick(),
                    record.after().map_or(0, |capture| capture.tick()),
                ]
            })
            .chain(monitor.progress().last_tick)
            .max()
            .unwrap_or(0);
        let mut state = Self {
            authority_tick: original_floor
                .max(clock.high_tick())
                .max(c.anchor.tick.get())
                .max(n.anchor.tick.get()),
            monitor,
            clock,
            selected: selected.clone(),
            monitor_pin,
            id: c.session_id,
            request: c.request_id.get(),
            budget: common_budget(c.budget, n.budget),
            grants: restricted(&c.grants, control),
            native_grants: restricted(&n.grants, control),
            mode,
        };
        state.inspect(c, n)?;
        Ok(state)
    }

    pub(super) fn id(&self) -> SessionId {
        self.id
    }
    pub(super) fn binding(&self) -> &BuildBinding {
        self.clock.definition().origin().definition().binding()
    }
    pub(super) fn mode(&self) -> Mode {
        self.mode
    }
    pub(super) fn high_tick(&self) -> u64 {
        self.authority_tick
            .max(self.clock.high_tick())
            .max(self.monitor.progress().last_tick.unwrap_or(0))
    }
    /// Only the already negotiated request allowance may be restored after the
    /// shell has separately charged opening work. This never adds future ticks.
    pub(super) fn set_budget(&mut self, mut budget: WorkBudget) {
        budget.max_game_ticks = 0;
        self.budget = budget;
    }
    pub(super) fn inspect_reserve(&self) -> Result<u64> {
        inspect_reserve(self.monitor.byte_len())
    }
    pub(super) fn context(&mut self, wall: Option<u64>) -> Result<OperationContext> {
        self.request = self.request.checked_add(1).ok_or_else(exhausted)?;
        let mut budget = self.budget;
        if let Some(wall) = wall {
            budget.max_wall_millis = budget.max_wall_millis.min(wall);
        }
        budget.max_game_ticks = 0;
        budget.validate()?;
        let mut current = super::context(
            self.id,
            RequestId::new(self.request),
            self.binding().fortress().fortress_id(),
            self.high_tick(),
            budget,
            BuildMode::Recover,
            false,
        );
        current.grants = self.grants.clone();
        Ok(current)
    }
    /// Build the independently issued native context without authorizing its
    /// use. This also lets explicit release redact evidence after Query expiry.
    pub(super) fn native_context(&self, c: &OperationContext) -> Result<OperationContext> {
        let mut native = c.clone();
        native.anchor.fortress_id = FortressId::NIL;
        native.anchor.cursor = ObservationCursor::ORIGIN;
        native.anchor.state_hash = Digest32::ZERO;
        native.anchor.tick = GameTick(native.anchor.tick.get().max(self.high_tick()));
        native.budget.max_game_ticks = 0;
        native.grants = self.native_grants.clone();
        Ok(native)
    }
    fn control(&self) -> bool {
        self.mode == Mode::Recovery && self.clock.is_control()
    }
    fn retain_floor(&mut self) {
        self.authority_tick = self.high_tick();
        self.clock.abandon_preparation();
    }
    fn pair(
        &mut self,
        c: &OperationContext,
        n: &OperationContext,
    ) -> Result<(OperationContext, OperationContext)> {
        if c.session_id != self.id
            || n.session_id != self.id
            || c.request_id != n.request_id
            || c.anchor.fortress_id != self.binding().fortress().fortress_id()
            || n.anchor.fortress_id != FortressId::NIL
        {
            return Err(denied());
        }
        self.authority_tick = self
            .high_tick()
            .max(c.anchor.tick.get())
            .max(n.anchor.tick.get());
        let mut current = c.clone();
        let mut native = n.clone();
        current.anchor.tick = GameTick(self.high_tick());
        native.anchor.tick = current.anchor.tick;
        let budget = common_budget(common_budget(current.budget, native.budget), self.budget);
        current.budget = budget;
        native.budget = budget;
        current.grants = restricted(&current.grants, self.control());
        native.grants = restricted(&native.grants, self.control());
        Ok((current, native))
    }
    fn check_authority(
        &self,
        c: &OperationContext,
        n: &OperationContext,
        effect: bool,
    ) -> Result<()> {
        if c.session_id != self.id
            || n.session_id != self.id
            || c.request_id != n.request_id
            || c.anchor.fortress_id != self.binding().fortress().fortress_id()
            || n.anchor.fortress_id != FortressId::NIL
            || (effect && !self.control())
        {
            return Err(denied());
        }
        let mut current = c.clone();
        let mut native = n.clone();
        current.anchor.tick = GameTick(
            self.high_tick()
                .max(c.anchor.tick.get())
                .max(n.anchor.tick.get()),
        );
        native.anchor.tick = current.anchor.tick;
        self.clock.authorize_disclosure(&current, &native)?;
        for retained in [false, true] {
            let mut canonical_check = current.clone();
            let mut native_check = native.clone();
            if retained {
                canonical_check.grants = self.grants.clone();
                native_check.grants = self.native_grants.clone();
            }
            canonical_check.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
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
        Ok(())
    }
    fn authorize(
        &mut self,
        c: &OperationContext,
        n: &OperationContext,
        effect: bool,
    ) -> Result<(OperationContext, OperationContext)> {
        let (current, native) = self.pair(c, n)?;
        self.check_authority(&current, &native, effect)?;
        Ok((current, native))
    }
    fn bounded(
        &mut self,
        work: &Work,
        reserved: &OperationContext,
        n: &OperationContext,
    ) -> Result<(OperationContext, OperationContext)> {
        let mut current = reserved.clone();
        current.budget.max_wall_millis = current
            .budget
            .max_wall_millis
            .min(work.current(&current)?.budget.max_wall_millis);
        let mut native = n.clone();
        native.budget = common_budget(native.budget, current.budget);
        self.authorize(&current, &native, false)
    }
    fn exact(&self, id: Digest32) -> Result<()> {
        if id == self.clock.definition().id() {
            Ok(())
        } else {
            Err(exact_error())
        }
    }
    fn inspect_clock(
        &mut self,
        work: &Work,
        reserved: &OperationContext,
        n: &OperationContext,
    ) -> Result<ClockView> {
        let (current, native) = self.bounded(work, reserved, n)?;
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
    fn inspect_monitor(
        &mut self,
        work: &Work,
        reserved: &OperationContext,
        n: &OperationContext,
    ) -> Result<()> {
        let (current, _) = self.bounded(work, reserved, n)?;
        let result = self.monitor.verify_recovery(&current, self.monitor_pin);
        self.retain_floor();
        result
    }

    pub(super) fn monitor_reference(&self) -> Value {
        let definition = self.monitor.definition();
        json!({"monitor_id":definition.id().to_string(),
            "goal_digest":definition.goal().digest().to_string(),
            "origin_digest":definition.origin().digest().to_string(),
            "recovery":{"monitor_id":definition.id().to_string(),"head":self.monitor.head().to_string()},
            "journal":{"head":self.monitor.head().to_string(),"frames":self.monitor.frames(),"byte_length":self.monitor.byte_len()},
            "history_verified":false,"monitor_inventory_verified":false,"monitor_terminal":false,
            "historical_phase":self.monitor.progress().phase,"phase":"unverified",
            "observations":self.monitor.progress().observations,"last_tick":self.monitor.progress().last_tick,
            "authority_tick_floor":self.high_tick(),"origin_verified":false,"inventory_verified":false,
            "sampled_condition_satisfied":false,"terminal":false,"historical_evidence_only":true,
            "clock_recovery_only":true,"fresh_sample_acquired":false,"goal_sample_published":false})
    }
    pub(super) fn clock_reference(&self) -> Value {
        let mut reference = clock::reference(&self.clock);
        reference["authority_tick_floor"] = json!(self.high_tick());
        historical_clock(&mut reference);
        reference
    }
    fn monitor_display(&self) -> Value {
        let mut value =
            completion::projected(self.monitor.definition(), self.monitor.progress(), false);
        value["history_verified"] = json!(true);
        value["monitor_inventory_verified"] = json!(true);
        value["monitor_terminal"] = json!(self.monitor.progress().terminal());
        value["recovery"] = json!({"monitor_id":self.monitor.definition().id().to_string(),"head":self.monitor.head().to_string()});
        value["journal"] = json!({"head":self.monitor.head().to_string(),"frames":self.monitor.frames(),"byte_length":self.monitor.byte_len()});
        value["authority_tick_floor"] = json!(self.high_tick());
        value["historical_evidence_only"] = json!(true);
        value["clock_recovery_only"] = json!(true);
        value["fresh_sample_acquired"] = json!(false);
        value["goal_sample_published"] = json!(false);
        value
    }
    fn clock_display(&self, view: &ClockView, offset: usize, limit: usize) -> Result<Value> {
        let mut value = clock::display(view, offset, limit)?;
        value["authority_tick_floor"] = json!(self.high_tick());
        historical_clock(&mut value);
        Ok(value)
    }
    fn retain(&self, mut result: Value) -> Value {
        if result.get("batch").is_none() {
            result["batch"] = completion::recovery_batch(self.monitor.definition().origin());
        }
        if result.get("completion").is_none() {
            result["completion"] = self.monitor_reference();
        }
        if result.get("construction_clock").is_none() {
            result["construction_clock"] = self.clock_reference();
        }
        result["clock_recovery_only"] = json!(true);
        result["profile"] = json!(self.mode.as_str());
        result["evidence_scope"] = json!("this_original_monitor_and_clock_history_only");
        result["native_placement_custody_opened"] = json!(false);
        result["furniture_parent_custody_opened"] = json!(false);
        result["original_inventory_verified"] = json!(false);
        result["original_placements_verified"] = json!(false);
        result["current_pause_proven"] = json!(false);
        result["fresh_sample_acquired"] = json!(false);
        result["goal_sample_published"] = json!(false);
        result["sampled_construction_condition_proven"] = json!(false);
        result["new_run_permitted"] = json!(false);
        result["retry_commit_permitted"] = json!(false);
        result["dispatch_authorized"] = json!(false);
        result
    }
    /// Pure, paired-Query-authorized rendering. Revocation suppresses every
    /// cached original/monitor/clock field, including failure handoffs.
    pub(super) fn reply(
        &self,
        op: &str,
        c: &OperationContext,
        n: &OperationContext,
        result: Value,
    ) -> String {
        let effect = result["effect_may_have_occurred"] == true;
        if let Err(cause) = self.check_authority(c, n, false) {
            let redacted = if result["closed"] == true && result["release_for_recovery"] == true {
                json!({"ok":true,"scope":"session","closed":true,"release_for_recovery":true,
                    "clock_recovery_only":true,"effects_cancelled":false,"history_erased":false,
                    "native_quiescence_proven":false,"effect_may_have_occurred":false})
            } else {
                failure(&cause, effect)
            };
            return presentation::packet(op, redacted, None, None, None, None, None, None, None);
        }
        let mut current = c.clone();
        current.anchor.tick = GameTick(
            self.high_tick()
                .max(c.anchor.tick.get())
                .max(n.anchor.tick.get()),
        );
        current.budget.max_game_ticks = 0;
        current.grants = restricted(&current.grants, self.control());
        let mut result = self.retain(result);
        let open = result["closed"] != true;
        result["clock_reconciliation_permitted"] = json!(open && self.mode == Mode::Recovery);
        result["clock_safety_cancel_permitted"] =
            json!(open && self.check_authority(c, n, true).is_ok());
        presentation::packet(
            op,
            result,
            Some(&current),
            Some(self.binding()),
            None,
            None,
            None,
            None,
            None,
        )
    }

    /// Exact local inspection only. The shell reserves inspect_reserve(len)
    /// before opening work and again before the final opening boundary check.
    pub(super) fn inspect(&mut self, c: &OperationContext, n: &OperationContext) -> Result<Value> {
        let (current, native) = self.authorize(c, n, false)?;
        let mut work = Work::new(&current, Instant::now())?;
        let bytes = monitor_reserve(self.monitor.byte_len())?;
        let monitor_before = work.take(&current, bytes)?;
        let clock_before = work.take(&current, clock::VIEW_BYTES)?;
        let monitor_after = work.take(&current, bytes)?;
        let clock_after = work.take(&current, clock::VIEW_BYTES)?;
        self.inspect_monitor(&work, &monitor_before, &native)?;
        let view = self.inspect_clock(&work, &clock_before, &native)?;
        let result = self.retain(json!({"ok":true,"completion":self.monitor_display(),
            "construction_clock":self.clock_display(&view,0,8)?,
            "native_calls":0,"game_mutations":0,"live_health_checked":false}));
        self.inspect_monitor(&work, &monitor_after, &native)?;
        let after = self.inspect_clock(&work, &clock_after, &native)?;
        if after.recovery != view.recovery {
            return Err(exact_error());
        }
        self.check_authority(&current, &native, false)?;
        work.current(&current)?;
        Ok(result)
    }

    fn perform<I: clock::ClockIo>(
        &mut self,
        c: &OperationContext,
        n: &OperationContext,
        work: &mut Work,
        action: Action,
        io: &mut I,
        effect_started: &mut bool,
    ) -> Result<Value> {
        match action {
            Action::Clock(clock::Action::Inspect {
                id,
                head,
                offset,
                limit,
            }) => {
                self.exact(id)?;
                let reserved = work.take(c, clock::VIEW_BYTES)?;
                io.checkpoint(&work.current(c)?, false)?;
                let view = self.inspect_clock(work, &reserved, n)?;
                if head != view.recovery.head {
                    return Err(exact_error());
                }
                Ok(
                    json!({"ok":true,"construction_clock":self.clock_display(&view,offset,limit)?,"native_calls":0,"game_mutations":0}),
                )
            }
            Action::Completion(completion::Action::Inspect)
            | Action::Query(Query::Batch {} | Query::Allocation { .. })
            | Action::Inventory => {
                let monitor_context = work.take(c, monitor_reserve(self.monitor.byte_len())?)?;
                let clock_context = work.take(c, clock::VIEW_BYTES)?;
                io.checkpoint(&work.current(c)?, false)?;
                self.inspect_monitor(work, &monitor_context, n)?;
                let view = self.inspect_clock(work, &clock_context, n)?;
                let mut result = json!({"ok":true,"completion":self.monitor_display(),
                    "construction_clock":self.clock_display(&view,0,8)?,"live_health_checked":false,
                    "native_calls":0,"game_mutations":0});
                if let Action::Query(Query::Allocation { view }) = action {
                    let handoff = self
                        .monitor
                        .definition()
                        .origin()
                        .definition()
                        .handoff()
                        .ok_or_else(|| {
                            error(
                                ErrorCode::InvalidRequest,
                                "original batch has no allocation origin",
                            )
                        })?;
                    result["allocation"] = allocation::display(handoff, view)?;
                    result["allocation"]["inventory_verified"] = json!(false);
                    result["allocation"]["monitor_copy_verified"] = json!(true);
                }
                Ok(result)
            }
            Action::Query(Query::Schema {}) => {
                io.checkpoint(&work.current(c)?, false)?;
                Ok(
                    json!({"ok":true,"query_schema":query_schema(),"native_calls":0,"game_mutations":0}),
                )
            }
            Action::Clock(clock::Action::Wait(id) | clock::Action::Cancel(id)) => {
                let cancel = matches!(action, Action::Clock(clock::Action::Cancel(_)));
                self.exact(id)?;
                if self.mode != Mode::Recovery {
                    return Err(denied());
                }
                let inspection = work.take(c, clock::VIEW_BYTES)?;
                let connection = work.take(c, CONNECT_BYTES)?;
                let operation = work.take(c, OPERATION_BYTES)?;
                let (current, native) = self.bounded(work, &inspection, n)?;
                self.authorize(&current, &native, cancel)?;
                io.checkpoint(&current, cancel)?;
                let before = self.inspect_clock(work, &inspection, n)?;
                let needs_source = if cancel {
                    before.cancellation_needs_source()
                } else {
                    before.needs_reconciliation()
                };
                let mut source = if needs_source {
                    let (current, native) = self.bounded(work, &connection, n)?;
                    self.authorize(&current, &native, cancel)?;
                    io.checkpoint(&current, cancel)?;
                    Some(io.connect(&self.selected, &native, cancel)?)
                } else {
                    None
                };
                let (current, native) = self.bounded(work, &operation, n)?;
                self.authorize(&current, &native, cancel)?;
                io.checkpoint(&current, cancel)?;
                let outcome = if cancel {
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
                    json!({"ok":true,"scope":"clock","construction_clock":self.clock_display(&view,0,8)?,
                    "native_source_contacted":needs_source,"safety_cancel_requested":cancel,
                    "effect_may_have_occurred":*effect_started,"new_unpause_attempted":false,
                    "replayed_locally":!needs_source,"monitor_history_used_as_effect_precondition":false}),
                )
            }
            _ => Err(denied()),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn run_action<I: clock::ClockIo>(
        &mut self,
        c: OperationContext,
        n: OperationContext,
        op: &str,
        action: Result<Action>,
        started: Instant,
        io: &mut I,
    ) -> String {
        self.clock.abandon_preparation();
        let (current, native) = match self.authorize(&c, &n, false) {
            Ok(pair) => pair,
            Err(cause) => return self.reply(op, &c, &n, failure(&cause, false)),
        };
        let mut work = match Work::new(&current, started) {
            Ok(work) => work,
            Err(cause) => return self.reply(op, &current, &native, failure(&cause, false)),
        };
        let final_monitor_needed = matches!(
            &action,
            Ok(Action::Inventory
                | Action::Completion(completion::Action::Inspect)
                | Action::Query(Query::Batch {} | Query::Allocation { .. }))
        );
        let cancel = self.mode == Mode::Recovery
            && matches!(&action, Ok(Action::Clock(clock::Action::Cancel(_))));
        let final_contexts = (|| {
            let final_clock = work.take(&current, clock::VIEW_BYTES)?;
            let final_monitor = if final_monitor_needed {
                Some(work.take(&current, monitor_reserve(self.monitor.byte_len())?)?)
            } else {
                None
            };
            Ok((final_clock, final_monitor))
        })();
        let (final_clock, final_monitor) = match final_contexts {
            Ok(contexts) => contexts,
            Err(cause) => return self.reply(op, &current, &native, failure(&cause, false)),
        };
        let mut effect_started = false;
        let outcome = action.and_then(|action| {
            self.perform(
                &current,
                &native,
                &mut work,
                action,
                io,
                &mut effect_started,
            )
        });
        self.retain_floor();
        let expected = self.clock.recovery();
        let published_monitor = outcome
            .as_ref()
            .is_ok_and(|value| value["completion"]["history_verified"] == true);
        let mut final_context = current;
        final_context.anchor.tick = GameTick(self.high_tick());
        let rendered = self.reply(
            op,
            &final_context,
            &native,
            match outcome {
                Ok(value) => value,
                Err(cause) => failure(&cause, effect_started),
            },
        );
        let publication = (|| {
            if rendered.len() as u64 > presentation::OUTPUT_BYTES {
                return Err(exhausted());
            }
            let (current, clock_native) = self.bounded(&work, &final_clock, &native)?;
            io.checkpoint(&current, cancel)?;
            self.authorize(&current, &clock_native, cancel)?;
            if published_monitor {
                self.inspect_monitor(
                    &work,
                    final_monitor.as_ref().ok_or_else(exhausted)?,
                    &native,
                )?;
            }
            let view = self.inspect_clock(&work, &final_clock, &native)?;
            if view.recovery != expected {
                return Err(exact_error());
            }
            self.check_authority(&current, &clock_native, false)?;
            work.current(&final_context)?;
            Ok(())
        })();
        match publication {
            Ok(()) => rendered,
            Err(cause) => {
                self.retain_floor();
                final_context.anchor.tick = GameTick(self.high_tick());
                self.reply(op, &final_context, &native, failure(&cause, effect_started))
            }
        }
    }

    /// Normal close needs intact settled clock custody. Explicit release only
    /// relinquishes local ownership; its callback checks runtime and deadlines,
    /// allowing release after a source configuration change or Query revocation.
    pub(super) fn close(
        &mut self,
        c: OperationContext,
        n: OperationContext,
        release: bool,
        started: Instant,
        mut checkpoint: impl FnMut(&OperationContext) -> Result<()>,
    ) -> String {
        let op = "fortress.cancel";
        self.clock.abandon_preparation();
        let (current, native) = match self.pair(&c, &n) {
            Ok(pair) => pair,
            Err(cause) => return self.reply(op, &c, &n, failure(&cause, false)),
        };
        let mut work = match Work::new(&current, started) {
            Ok(work) => work,
            Err(cause) => return self.reply(op, &current, &native, failure(&cause, false)),
        };
        let expected = self.clock.recovery();
        let final_clock = if release {
            None
        } else {
            match work.take(&current, clock::VIEW_BYTES) {
                Ok(reserved) => Some(reserved),
                Err(cause) => return self.reply(op, &current, &native, failure(&cause, false)),
            }
        };
        let outcome = (|| {
            if current.cancellation_requested || native.cancellation_requested {
                return Err(error(
                    ErrorCode::CancellationRequested,
                    "construction clock recovery close cancelled",
                ));
            }
            checkpoint(&work.current(&current)?)?;
            let view = if release {
                self.clock_reference()
            } else {
                let reserved = work.take(&current, clock::VIEW_BYTES)?;
                let view = self.inspect_clock(&work, &reserved, &native)?;
                if view.pending || view.unresolved {
                    return Err(error(
                        ErrorCode::EffectIndeterminate,
                        "unsettled original clock requires explicit recovery release",
                    ));
                }
                self.clock_display(&view, 0, 8)?
            };
            Ok(
                json!({"ok":true,"scope":"session","closed":true,"release_for_recovery":release,
                "construction_clock":view,"effects_cancelled":false,"monitor_cancelled":false,
                "history_erased":false,"native_quiescence_proven":false,"effect_may_have_occurred":false}),
            )
        })();
        self.retain_floor();
        let mut final_context = current;
        final_context.anchor.tick = GameTick(self.high_tick());
        let output = self.reply(
            op,
            &final_context,
            &native,
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
                let view = self.inspect_clock(&work, &reserved, &native)?;
                if view.recovery != expected {
                    return Err(exact_error());
                }
            }
            work.current(&final_context)?;
            Ok(())
        })();
        match publication {
            Ok(()) => output,
            Err(cause) => self.reply(op, &final_context, &native, failure(&cause, false)),
        }
    }
}

fn historical_clock(value: &mut Value) {
    value["fresh_goal_sampling_permitted"] = json!(false);
    value["historical_evidence_only"] = json!(true);
    value["current_pause_proven"] = json!(false);
    value["sampled_construction_condition_proven"] = json!(false);
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
fn query_schema() -> Value {
    let mut schema = super::schema();
    if let Some(modes) = schema.get_mut("modes").and_then(Value::as_object_mut) {
        modes.retain(|name, _| {
            matches!(
                name.as_str(),
                "schema" | "completion" | "construction_clock" | "batch" | "allocation"
            )
        });
    }
    schema["schema"] = json!("dfmcp.furniture-clock-recovery-query/1");
    schema
}
