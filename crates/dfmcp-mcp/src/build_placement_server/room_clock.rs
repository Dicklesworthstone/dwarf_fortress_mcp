//! Separately reviewed finite advancement for one exact original joint room goal.
//! Readiness retains both map endpoints and every original furnishing, without
//! positive goal credit. This owner cannot adopt an ordinary furniture clock.
pub(super) use super::clock::{Action, Config, Request, native_context, recovery};
use super::{
    State, Work, clock, completion, error, exhausted, presentation, room_monitor, runtime,
};
use dfmcp_adapter::bounded_run::private_file::PrivateRunFile;
use dfmcp_adapter::bounded_run::rpc::RunSource;
use dfmcp_adapter::build_placement::journal::BuildMode;
use dfmcp_adapter::build_placement::journal::BuildSource;
use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
pub(super) use dfmcp_adapter::room_condition::clock::Recovery;
use dfmcp_adapter::room_condition::clock::{
    Clock, ClockSample, ClockView, Edge, OPERATION_BYTES, RunRequest,
};
use dfmcp_adapter::room_condition::{Definition, Sample, SourceIdentity};
use dfmcp_core::{Capability, Digest32, ErrorCode, GameTick, OperationContext, Result, RiskTier};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
pub(super) const VIEW_BYTES: u64 = dfmcp_adapter::room_condition::clock::VIEW_BYTES;
pub(super) const OPEN_BYTES: u64 = 3 * VIEW_BYTES;
const CONNECT_BYTES: u64 = 4 * 1024 * 1024;
const SOURCE_BYTES: u64 = 32 * 1024 * 1024;
const DECODE_BYTES: u64 = 64 * 1024 * 1024;
fn denied(message: &str) -> dfmcp_core::DfmcpError {
    error(ErrorCode::CapabilityDenied, message)
}

pub(super) trait Io {
    type Source: RunSource;
    fn opening_recovery(&self) -> Option<Recovery> {
        None
    }
    fn open(
        &mut self,
        config: &Config,
        definition: &Definition,
        source: &SourceIdentity,
        c: &OperationContext,
        n: &OperationContext,
    ) -> Result<Clock<PrivateRunFile>>;
    fn connect(
        &mut self,
        config: &Config,
        n: &OperationContext,
        control: bool,
    ) -> Result<Self::Source>;
    fn sample(&mut self, definition: &Definition, c: &OperationContext) -> Result<Sample>;
    fn checkpoint(&mut self, c: &OperationContext, control: bool) -> Result<()>;
    fn observed_tick_floor(&self) -> u64 {
        0
    }
}

pub(super) fn reference<C: EffectJournalStorage>(owner: &Clock<C>) -> Value {
    let view = owner.cached_view();
    json!({"monitor_id":owner.definition().id().to_string(),
        "batch_id":owner.definition().origin().definition().id().to_string(),
        "room_intent_digest":owner.definition().room().digest().to_string(),
        "recovery":recovery(view.recovery),"mode":view.mode.as_str(),
        "history_verified":false,"authority_tick_floor":view.high_tick,
        "run_count":view.runs.len(),"pending":view.pending,"unresolved":view.unresolved,
        "may_be_running":view.may_be_running(),"active_run":view.active().map(|row|clock::run_row(row,false)),
        "retry_commit_permitted":false,"current_pause_proven":false,
        "sampled_joint_room_condition_proven":false,"shared_process_identity_proven":false,
        "construction_causality_proven":false,"room_goal_promoted_from_furniture_clock":false,
        "evidence_scope":"original_joint_room_clock_history_only"})
}
pub(super) fn display(view: &ClockView, offset: usize, limit: usize) -> Result<Value> {
    let mut value = clock::display(view, offset, limit)?;
    value["sampled_joint_room_condition_proven"] = json!(false);
    value["room_goal_promoted_from_furniture_clock"] = json!(false);
    value["evidence_scope"] = json!("original_joint_room_clock_history_only");
    if value["runs_page"]["next_query"].is_object() {
        value["runs_page"]["next_query"]["mode"] = json!("room_clock");
    }
    if let Some(readiness) = &view.readiness {
        value["readiness"]["evidence_scope"] = json!("pre_dispatch_original_joint_room_bracket");
        value["readiness"]["terrain"] = readiness
            .terrain()
            .map_or(Value::Null, room_monitor::terrain);
        value["readiness"]["room_condition_proven"] = json!(false);
        value["readiness"]["active_dig_work_proven"] = json!(false);
    }
    Ok(value)
}
pub(super) fn bootstrap_pending<S: EffectJournalStorage, N: BuildSource>(
    state: &State<S, N>,
) -> bool {
    state.room_clock_bootstrap
        && state.room_clock.is_none()
        && state.room_clock_recovery.is_none()
        && state
            .room_clock_config
            .as_ref()
            .is_some_and(|config| config.initialize && config.recovery.is_none())
}
fn verify_absent<S: EffectJournalStorage, N: BuildSource>(
    state: &State<S, N>,
    c: &OperationContext,
) -> Result<()> {
    c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    let config = state.room_clock_config.as_ref().ok_or_else(exhausted)?;
    if !config.initialize
        || config.recovery.is_some()
        || state.room_clock_recovery.is_some()
        || state.room_clock.is_some()
    {
        return Err(denied("initial room clock custody is not absent"));
    }
    match std::fs::symlink_metadata(&config.path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err(error(
            ErrorCode::CorruptLedger,
            "configured room clock path already exists or cannot be verified absent",
        )),
    }
}
pub(super) fn admit_bootstrap<S: EffectJournalStorage, N: BuildSource>(
    state: &mut State<S, N>,
    c: &OperationContext,
) -> Result<()> {
    if state.room_clock_config.is_none() || state.room_clock.is_some() {
        return Ok(());
    }
    if state
        .room_completion
        .as_ref()
        .is_some_and(|monitor| monitor.store.first_source().is_some())
    {
        return Err(denied(
            "accepted joint source requires exact room clock custody before another acquisition",
        ));
    }
    verify_absent(state, c)?;
    state.room_clock_bootstrap = true;
    Ok(())
}
pub(super) fn ensure_for_joint_sample<S: EffectJournalStorage, N: BuildSource>(
    state: &mut State<S, N>,
    c: &OperationContext,
) -> Result<()> {
    inspect(state, c)?;
    if blocks_sampling(state) {
        return Err(error(
            ErrorCode::EffectIndeterminate,
            "settle and verify the retained room clock before acquiring joint room evidence",
        ));
    }
    Ok(())
}
fn opening_audit<S: EffectJournalStorage, N: BuildSource>(
    state: &mut State<S, N>,
    c: &OperationContext,
) -> Result<()> {
    let monitor = state
        .room_completion
        .as_mut()
        .ok_or_else(|| denied("original joint monitor is absent"))?;
    let bytes = monitor.store.byte_len() as u64 + 8192;
    let start = Instant::now();
    let mut history = c.clone();
    history.budget.max_bytes = bytes;
    monitor.store.verify(&history)?;
    let mut source = c.clone();
    source.budget.max_bytes = source
        .budget
        .max_bytes
        .checked_sub(bytes)
        .ok_or_else(exhausted)?;
    source.budget.max_wall_millis = source
        .budget
        .max_wall_millis
        .checked_sub(u64::try_from(start.elapsed().as_millis()).map_err(|_| exhausted())?)
        .filter(|left| *left > 0)
        .ok_or_else(exhausted)?;
    let view = completion::verify_source(
        &mut state.control,
        &mut state.batch,
        &state.binding,
        monitor.store.definition().origin(),
        (&monitor.parent, &monitor.child),
        &source,
    )?;
    state.historical = Some(view);
    monitor.store_verified = true;
    monitor.verified = true;
    state.authority_tick = state
        .authority_tick
        .max(monitor.store.authority_tick_floor())
        .max(state.control.high_tick());
    Ok(())
}
pub(super) fn open<S: EffectJournalStorage, N: BuildSource, I: Io>(
    state: &mut State<S, N>,
    c: &OperationContext,
    work: &mut Work,
    io: &mut I,
) -> Result<()> {
    let Some(config) = state.room_clock_config.clone() else {
        return Ok(());
    };
    if state.room_clock.is_some() {
        inspect(state, &work.take(c, VIEW_BYTES)?)?;
        return Ok(());
    }
    let source = state
        .room_completion
        .as_ref()
        .and_then(|monitor| monitor.store.first_source())
        .cloned();
    let Some(source) = source else {
        return admit_bootstrap(state, c);
    };
    let definition = state
        .room_completion
        .as_ref()
        .ok_or_else(exhausted)?
        .store
        .definition()
        .clone();
    config.matches(definition.original())?;
    if config.initialize {
        c.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
        c.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
        verify_absent(state, c)?;
    }
    let audit = audit_bytes(state)?;
    let initial = work.take(c, audit)?;
    let opening = work.take(c, OPEN_BYTES)?;
    let final_source = work.take(c, audit)?;
    let final_clock = work.take(c, VIEW_BYTES)?;
    io.checkpoint(&work.current(c)?, config.initialize)?;
    opening_audit(state, &narrowed(work, &initial, state.authority_tick)?)?;
    let opening = narrowed(work, &opening, state.authority_tick)?;
    let native = native_context(&opening, &state.room_clock_grants)?;
    state.room_clock_bootstrap = false;
    let opened = io.open(&config, &definition, &source, &opening, &native);
    if let Some(pin) = io.opening_recovery() {
        state.room_clock_recovery = Some(pin);
    }
    let owner = opened?;
    let agrees = owner.definition().canonical_bytes() == definition.canonical_bytes()
        && owner.source() == &source
        && owner.binding() == &config.binding;
    state.room_clock_recovery = Some(owner.recovery());
    state.authority_tick = state.authority_tick.max(owner.high_tick());
    state.room_clock = Some(owner);
    if !agrees {
        return Err(error(
            ErrorCode::StaleAnchor,
            "opened room clock differs from the original joint goal/source",
        ));
    }
    io.checkpoint(&work.current(c)?, config.initialize)?;
    opening_audit(state, &narrowed(work, &final_source, state.authority_tick)?)?;
    inspect(state, &narrowed(work, &final_clock, state.authority_tick)?)?;
    work.current(c)?;
    Ok(())
}
fn retain_floor<S: EffectJournalStorage, N: BuildSource>(state: &mut State<S, N>) {
    if let Some(monitor) = &state.room_completion {
        state.authority_tick = state
            .authority_tick
            .max(monitor.store.authority_tick_floor());
        state.room_recovery = Some(monitor.store.recovery());
    }
    if let Some(clock) = &state.room_clock {
        state.authority_tick = state.authority_tick.max(clock.high_tick());
        state.room_clock_recovery = Some(clock.recovery());
    }
}

pub(super) fn blocks_sampling<S: EffectJournalStorage, N: BuildSource>(
    state: &State<S, N>,
) -> bool {
    match &state.room_clock {
        Some(clock) => {
            let view = clock.cached_view();
            !state.room_clock_verified || view.pending || view.unresolved
        }
        None => {
            (state.room_clock_recovery.is_some() || state.room_clock_config.is_some())
                && (!bootstrap_pending(state)
                    || state
                        .room_completion
                        .as_ref()
                        .is_some_and(|m| m.store.first_source().is_some()))
        }
    }
}

pub(super) fn inspect<S: EffectJournalStorage, N: BuildSource>(
    state: &mut State<S, N>,
    c: &OperationContext,
) -> Result<Option<ClockView>> {
    state.room_clock_verified = false;
    if state.room_clock.is_none() {
        if bootstrap_pending(state) {
            verify_absent(state, c)?;
            return Ok(None);
        }
        return if state.room_clock_recovery.is_some() || state.room_clock_config.is_some() {
            Err(denied("configured room clock custody is not open"))
        } else {
            Ok(None)
        };
    }
    let mut current = c.clone();
    current.anchor.tick = GameTick(current.anchor.tick.get().max(state.authority_tick));
    let native = native_context(&current, &state.room_clock_grants)?;
    let outcome = state
        .room_clock
        .as_mut()
        .ok_or_else(exhausted)?
        .inspect(&current, &native);
    retain_floor(state);
    if outcome.is_ok() {
        state.room_clock_verified = true;
    }
    outcome.map(Some)
}

/// Configured custody is established eagerly when reopening a completion
/// monitor with an accepted original source. Sampling never initializes it.
pub(super) fn ensure_for_sample<S: EffectJournalStorage, N: BuildSource>(
    state: &mut State<S, N>,
    c: &OperationContext,
) -> Result<()> {
    inspect(state, c)?;
    if blocks_sampling(state) || bootstrap_pending(state) {
        return Err(error(
            ErrorCode::EffectIndeterminate,
            "settle and verify the retained clock before acquiring joint room proof",
        ));
    }
    Ok(())
}

fn narrowed(work: &Work, reserved: &OperationContext, tick: u64) -> Result<OperationContext> {
    let mut current = reserved.clone();
    current.anchor.tick = GameTick(current.anchor.tick.get().max(tick));
    current.budget.max_wall_millis = current
        .budget
        .max_wall_millis
        .min(work.current(&current)?.budget.max_wall_millis);
    Ok(current)
}
fn inspect_current<S: EffectJournalStorage, N: BuildSource>(
    state: &mut State<S, N>,
    work: &Work,
    reserved: &OperationContext,
) -> Result<ClockView> {
    let current = narrowed(work, reserved, state.authority_tick)?;
    inspect(state, &current)?.ok_or_else(exhausted)
}
fn exact<S: EffectJournalStorage, N: BuildSource>(state: &State<S, N>, id: Digest32) -> Result<()> {
    if state
        .room_clock
        .as_ref()
        .is_some_and(|clock| clock.spec().id() == id)
    {
        Ok(())
    } else {
        Err(error(
            ErrorCode::StaleAnchor,
            "exact original room clock is not retained",
        ))
    }
}
fn audit_bytes<S: EffectJournalStorage, N: BuildSource>(state: &State<S, N>) -> Result<u64> {
    let monitor = state
        .room_completion
        .as_ref()
        .ok_or_else(|| denied("original completion monitor absent"))?;
    completion::source_reserve(monitor.store.definition().origin())?
        .checked_add(monitor.store.byte_len() as u64 + 8192)
        .ok_or_else(exhausted)
}
fn active_original<S: EffectJournalStorage, N: BuildSource>(
    state: &mut State<S, N>,
    spec: &Definition,
    c: &OperationContext,
) -> Result<()> {
    if state.control.mode() != BuildMode::Control {
        return Err(denied(
            "clock advancement requires writable original furnishing custody",
        ));
    }
    if state.policy.config.checkpoint != runtime::CheckpointPolicy::DisposableFortress {
        return Err(error(
            ErrorCode::CheckpointRequired,
            "new clock advancement requires the current explicit disposable-fortress checkpoint exception",
        ));
    }
    let mut current = c.clone();
    current.anchor.tick = GameTick(
        current
            .anchor
            .tick
            .get()
            .max(state.authority_tick)
            .max(state.control.high_tick()),
    );
    for (capability, risk) in [
        (Capability::Query, RiskTier::ReadOnly),
        (Capability::Observe, RiskTier::ReadOnly),
        (Capability::Plan, RiskTier::Guarded),
        (Capability::ControlClock, RiskTier::Guarded),
    ] {
        current.authorize(capability, risk, &[], None)?;
    }
    state
        .room_clock_config
        .as_ref()
        .ok_or_else(|| denied("clock configuration absent"))?
        .matches(spec.original())?;
    let monitor = state
        .room_completion
        .as_mut()
        .ok_or_else(|| denied("original completion monitor absent"))?;
    if monitor.store.definition().canonical_bytes() != spec.canonical_bytes()
        || monitor.store.is_fenced()
        || monitor.store.progress().reading
        || monitor.store.progress().terminal()
        || monitor.store.progress().observations == 0
        || monitor.store.progress().observations >= spec.goal().timing().max_observations
    {
        return Err(denied(
            "clock requires the complete unfinished idle original monitor",
        ));
    }
    let start = Instant::now();
    let monitor_bytes = monitor.store.byte_len() as u64 + 8192;
    if current.budget.max_bytes < monitor_bytes {
        return Err(exhausted());
    }
    let mut history = current.clone();
    history.budget.max_bytes = monitor_bytes;
    monitor.store.verify(&history)?;
    let source = monitor
        .store
        .progress()
        .source
        .as_ref()
        .ok_or_else(|| denied("accepted original source absent"))?;
    if source.furniture.furniture_generation != state.binding.generation()
        || state
            .room_clock
            .as_ref()
            .is_some_and(|clock| source != clock.source())
    {
        return Err(error(
            ErrorCode::StaleAnchor,
            "original completion source changed",
        ));
    }
    let mut origin_context = current;
    origin_context.budget.max_bytes = origin_context
        .budget
        .max_bytes
        .checked_sub(monitor_bytes)
        .ok_or_else(exhausted)?;
    origin_context.budget.max_wall_millis = origin_context
        .budget
        .max_wall_millis
        .checked_sub(u64::try_from(start.elapsed().as_millis()).map_err(|_| exhausted())?)
        .filter(|left| *left > 0)
        .ok_or_else(exhausted)?;
    let view = completion::verify_source(
        &mut state.control,
        &mut state.batch,
        &state.binding,
        spec.origin(),
        (&monitor.parent, &monitor.child),
        &origin_context,
    )?;
    state.historical = Some(view);
    if state
        .batch
        .as_ref()
        .is_none_or(|parent| parent.stopped() || parent.is_fenced())
    {
        return Err(denied(
            "stopped or fenced original furnishing batch cannot advance the clock",
        ));
    }
    Ok(())
}
fn preflight<S: EffectJournalStorage, N: BuildSource>(
    state: &State<S, N>,
    spec: &Definition,
    c: &OperationContext,
    ticks: u32,
) -> Result<()> {
    let cells = spec.room().terrain().region().volume().map_err(|_| {
        error(
            ErrorCode::CorruptLedger,
            "retained original room region is invalid",
        )
    })?;
    if cells
        .saturating_mul(2)
        .saturating_add(spec.goal().records().len())
        > c.budget.max_entities as usize
    {
        return Err(exhausted());
    }
    if u64::from(ticks) > c.budget.max_game_ticks
        || ticks > spec.goal().timing().max_gap
        || state
            .room_clock
            .as_ref()
            .is_none_or(|owner| !owner.is_control())
    {
        return Err(exhausted());
    }
    let horizon = c
        .anchor
        .tick
        .get()
        .max(state.authority_tick)
        .checked_add(u64::from(ticks))
        .filter(|tick| *tick <= spec.goal().timing().deadline)
        .ok_or_else(exhausted)?;
    let mut future = c.clone();
    future.anchor.tick = GameTick(horizon);
    for (capability, risk) in [
        (Capability::Query, RiskTier::ReadOnly),
        (Capability::Observe, RiskTier::ReadOnly),
        (Capability::Plan, RiskTier::Guarded),
        (Capability::ControlClock, RiskTier::Guarded),
    ] {
        future.authorize(capability, risk, &[], None)?;
    }
    let native = native_context(&future, &state.room_clock_grants)?;
    native.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
    native.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)
}

/// One separately prepaid allowance for the complete write-ahead readiness
/// acquisition. All nested custody checks share the outer and native deadline.
struct GuardWork {
    context: OperationContext,
    deadline: Instant,
    bytes: u64,
}
impl GuardWork {
    fn new(work: &Work, reserved: &OperationContext, edge: &OperationContext) -> Result<Self> {
        let mut context = narrowed(work, reserved, edge.anchor.tick.get())?;
        context.grants = edge.grants.clone();
        context.cancellation_requested |= edge.cancellation_requested;
        context.budget.max_wall_millis = context
            .budget
            .max_wall_millis
            .min(edge.budget.max_wall_millis);
        Ok(Self {
            deadline: Instant::now()
                .checked_add(Duration::from_millis(context.budget.max_wall_millis))
                .ok_or_else(exhausted)?,
            bytes: context.budget.max_bytes,
            context,
        })
    }
    fn take(&mut self, bytes: u64, tick: u64) -> Result<OperationContext> {
        self.bytes = self.bytes.checked_sub(bytes).ok_or_else(exhausted)?;
        let mut current = self.context.clone();
        current.anchor.tick = GameTick(current.anchor.tick.get().max(tick));
        current.budget.max_bytes = bytes;
        current.budget.max_wall_millis = self
            .deadline
            .checked_duration_since(Instant::now())
            .and_then(|left| u64::try_from(left.as_millis()).ok())
            .filter(|left| *left > 0)
            .ok_or_else(exhausted)?;
        Ok(current)
    }
}
fn readiness_reserve<S: EffectJournalStorage, N: BuildSource>(state: &State<S, N>) -> Result<u64> {
    let store = &state.room_completion.as_ref().ok_or_else(exhausted)?.store;
    let source = completion::source_reserve(store.definition().origin())?;
    // A reopened monitor may need two read-intent frames. Every source guard,
    // both complete journal checks, publication and decode are prepaid before
    // the clock can synchronize its Intent or DispatchStarted marker.
    source
        .checked_add(if state.clock_config.is_some() || state.clock.is_some() {
            clock::VIEW_BYTES
        } else {
            0
        })
        .and_then(|bytes| bytes.checked_mul(12))
        .and_then(|bytes| bytes.checked_add(2 * (store.byte_len() as u64 + 8192)))
        .and_then(|bytes| bytes.checked_add(store.operation_reserve(false)))
        .and_then(|bytes| bytes.checked_add(store.operation_reserve(true)))
        .and_then(|bytes| bytes.checked_add(SOURCE_BYTES + DECODE_BYTES + 16_384))
        .ok_or_else(exhausted)
}

#[allow(clippy::too_many_arguments)]
fn guard<S: EffectJournalStorage, N: BuildSource, I: Io>(
    state: &mut State<S, N>,
    spec: &Definition,
    edge: &OperationContext,
    reserved: &OperationContext,
    work: &Work,
    audit: u64,
    io: &mut I,
) -> Result<ClockSample> {
    let mut budget = GuardWork::new(work, reserved, edge)?;
    let initial = budget.take(audit, state.authority_tick)?;
    io.checkpoint(&initial, true)?;
    active_original(state, spec, &initial)?;
    if state.clock_config.is_some() || state.clock.is_some() {
        let other = budget.take(clock::VIEW_BYTES, state.authority_tick)?;
        clock::ensure_for_sample(state, &other)?;
    }
    let mut monitor = state.room_completion.take().ok_or_else(exhausted)?;
    let outcome = (|| {
        let parent = monitor.parent.clone();
        let child = monitor.child.clone();
        let source_bytes = completion::source_reserve(spec.origin())?;
        let begin = budget.take(monitor.store.operation_reserve(false), state.authority_tick)?;
        {
            let mut admit =
                |definition: &Definition, progress: &dfmcp_adapter::room_condition::Progress| {
                    if definition.canonical_bytes() != spec.canonical_bytes() || progress.terminal()
                    {
                        return Err(denied("readiness requires the unchanged unfinished goal"));
                    }
                    Ok(())
                };
            let mut source_guard = |origin: &dfmcp_adapter::construction_plan::origin::Origin,
                                    c: &OperationContext| {
                let mut current =
                    budget.take(source_bytes, state.authority_tick.max(c.anchor.tick.get()))?;
                current.budget.max_wall_millis =
                    current.budget.max_wall_millis.min(c.budget.max_wall_millis);
                io.checkpoint(&current, true)?;
                if state.clock_config.is_some() || state.clock.is_some() {
                    let other = budget.take(clock::VIEW_BYTES, current.anchor.tick.get())?;
                    clock::ensure_for_sample(state, &other)?;
                    current.anchor.tick =
                        GameTick(current.anchor.tick.get().max(state.authority_tick));
                }
                let view = completion::verify_source(
                    &mut state.control,
                    &mut state.batch,
                    &state.binding,
                    origin,
                    (&parent, &child),
                    &current,
                )?;
                state.historical = Some(view);
                Ok(())
            };
            monitor
                .store
                .begin_read(&begin, &mut admit, &mut source_guard)?;
        }
        let read = budget.take(SOURCE_BYTES, state.authority_tick)?;
        io.checkpoint(&read, true)?;
        let _decode = budget.take(DECODE_BYTES, state.authority_tick)?;
        let acquired = io.sample(spec, &read);
        state.authority_tick = state.authority_tick.max(io.observed_tick_floor());
        monitor
            .store
            .retain_observed_tick_floor(state.authority_tick);
        let sample = acquired?;
        let decoded = sample.retain_observed_tick(&mut state.authority_tick);
        monitor
            .store
            .retain_observed_tick_floor(state.authority_tick);
        decoded?;
        let history = budget.take(monitor.store.byte_len() as u64 + 8192, state.authority_tick)?;
        monitor.store.verify(&history)?;
        let publish_bytes = monitor
            .store
            .operation_reserve(true)
            .checked_add(8192)
            .ok_or_else(exhausted)?;
        let publish = budget.take(publish_bytes, state.authority_tick)?;
        let mut admit = |definition: &Definition, _: &dfmcp_adapter::room_condition::Progress| {
            if definition.canonical_bytes() != spec.canonical_bytes() {
                return Err(denied("readiness names another original joint room goal"));
            }
            Ok(())
        };
        let mut source_guard = |origin: &dfmcp_adapter::construction_plan::origin::Origin,
                                c: &OperationContext| {
            let mut current =
                budget.take(source_bytes, state.authority_tick.max(c.anchor.tick.get()))?;
            current.budget.max_wall_millis =
                current.budget.max_wall_millis.min(c.budget.max_wall_millis);
            io.checkpoint(&current, true)?;
            if state.clock_config.is_some() || state.clock.is_some() {
                let other = budget.take(clock::VIEW_BYTES, current.anchor.tick.get())?;
                clock::ensure_for_sample(state, &other)?;
                current.anchor.tick = GameTick(current.anchor.tick.get().max(state.authority_tick));
            }
            let view = completion::verify_source(
                &mut state.control,
                &mut state.batch,
                &state.binding,
                origin,
                (&parent, &child),
                &current,
            )?;
            state.historical = Some(view);
            Ok(())
        };
        // This distinct durable frame consumes the original acquisition budget
        // but never credits stability. Valid all-member readiness preserves an
        // existing streak; unknown, pending and interrupted evidence clears it.
        // Absorbing negative evidence is retained before refusing clock control.
        monitor
            .store
            .publish_readiness(&publish, &sample, &mut admit, &mut source_guard)?;
        if monitor.store.progress().terminal() {
            return Err(denied(
                "readiness terminated the original joint room goal; no new run is permitted",
            ));
        }
        let progress = monitor.store.progress().clone();
        let final_check = budget.take(0, state.authority_tick)?;
        work.current(&final_check)?;
        monitor.verified = true;
        monitor.store_verified = true;
        Ok(ClockSample { progress, sample })
    })();
    monitor.store.abandon_read();
    if outcome.is_err() {
        monitor.verified = false;
        monitor.store_verified = false;
    }
    state.authority_tick = state
        .authority_tick
        .max(monitor.store.authority_tick_floor());
    state.room_recovery = Some(monitor.store.recovery());
    state.room_completion = Some(monitor);
    outcome
}
fn retain_readiness<S: EffectJournalStorage, N: BuildSource>(
    state: &mut State<S, N>,
    view: &ClockView,
) {
    if view
        .readiness
        .as_ref()
        .is_some_and(|ready| ready.withdraws_stability())
    {
        state.withdraw_monitor();
    }
}
fn configuration<S: EffectJournalStorage, N: BuildSource>(state: &State<S, N>) -> Result<Config> {
    state
        .room_clock_config
        .clone()
        .ok_or_else(|| denied("clock is not configured"))
}

fn perform<S: EffectJournalStorage, N: BuildSource, I: Io>(
    state: &mut State<S, N>,
    c: &OperationContext,
    work: &mut Work,
    action: Action,
    io: &mut I,
) -> Result<Value> {
    match action {
        Action::Prepare(request) => {
            exact(state, request.monitor_id)?;
            let spec = state
                .room_clock
                .as_ref()
                .ok_or_else(exhausted)?
                .spec()
                .clone();
            let config = configuration(state)?;
            let audit = audit_bytes(state)?;
            let initial = work.take(c, audit)?;
            let guard_budget = work.take(c, readiness_reserve(state)?)?;
            let initial_clock = work.take(c, VIEW_BYTES)?;
            let connect = work.take(c, CONNECT_BYTES)?;
            let operation = work.take(c, OPERATION_BYTES)?;
            let final_check = work.take(c, VIEW_BYTES)?;
            if state.clock_config.is_some() || state.clock.is_some() {
                let other = work.take(c, clock::VIEW_BYTES)?;
                clock::ensure_for_sample(state, &narrowed(work, &other, state.authority_tick)?)?;
            }
            active_original(
                state,
                &spec,
                &narrowed(work, &initial, state.authority_tick)?,
            )?;
            preflight(
                state,
                &spec,
                &narrowed(work, &operation, state.authority_tick)?,
                request.game_ticks,
            )?;
            io.checkpoint(&work.current(c)?, true)?;
            let before = inspect_current(state, work, &initial_clock)?;
            if before.pending {
                return Err(error(
                    ErrorCode::EffectIndeterminate,
                    "settle retained room clock before another interval",
                ));
            }
            if before.runs.len()
                >= dfmcp_adapter::room_condition::clock::MAX_RUNS
                    .min(spec.goal().timing().max_observations as usize)
            {
                return Err(exhausted());
            }
            let current = narrowed(work, &connect, state.authority_tick)?;
            let native = native_context(&current, &state.room_clock_grants)?;
            let mut source = io.connect(&config, &native, true)?;
            let current = narrowed(work, &operation, state.authority_tick)?;
            let native = native_context(&current, &state.room_clock_grants)?;
            let mut owner = state.room_clock.take().ok_or_else(exhausted)?;
            let mut budget = Some(guard_budget);
            let outcome = owner.prepare_guarded(
                &mut source,
                &RunRequest {
                    monitor_id: request.monitor_id,
                    game_ticks: request.game_ticks,
                    wall_millis: request.wall_millis,
                },
                &current,
                &native,
                |edge, _, current| {
                    if edge != Edge::Prepare {
                        return Err(error(
                            ErrorCode::InternalInvariantViolation,
                            "wrong clock prepare boundary",
                        ));
                    }
                    let reserved = budget.take().ok_or_else(exhausted)?;
                    guard(state, &spec, current, &reserved, work, audit, io)
                },
            );
            state.room_clock = Some(owner);
            retain_floor(state);
            let view = outcome?;
            retain_readiness(state, &view);
            let checked = inspect_current(state, work, &final_check)?;
            io.checkpoint(&work.current(c)?, true)?;
            Ok(json!({"ok":true,"room_clock":display(&checked,0,8)?,
                "confirmation_required":true,"game_unpause_requested":false,"goal_sample_published":false,
                "readiness_acquisition_retained":true,"readiness_earns_stability_credit":false}))
        }
        Action::Commit { id, seal } => {
            exact(state, id)?;
            let spec = state
                .room_clock
                .as_ref()
                .ok_or_else(exhausted)?
                .spec()
                .clone();
            let config = configuration(state)?;
            let audit = audit_bytes(state)?;
            let initial = work.take(c, audit)?;
            let initial_clock = work.take(c, VIEW_BYTES)?;
            let guard_budget = work.take(c, readiness_reserve(state)?)?;
            let connect = work.take(c, CONNECT_BYTES)?;
            let operation = work.take(c, OPERATION_BYTES)?;
            let final_check = work.take(c, VIEW_BYTES)?;
            if state.clock_config.is_some() || state.clock.is_some() {
                let other = work.take(c, clock::VIEW_BYTES)?;
                clock::ensure_for_sample(state, &narrowed(work, &other, state.authority_tick)?)?;
            }
            active_original(
                state,
                &spec,
                &narrowed(work, &initial, state.authority_tick)?,
            )?;
            let before = inspect_current(state, work, &initial_clock)?;
            let prepared = before
                .active()
                .filter(|run| run.review_seal == Some(seal))
                .ok_or_else(|| denied("clock commit requires current separately reviewed seal"))?;
            preflight(
                state,
                &spec,
                &narrowed(work, &operation, state.authority_tick)?,
                prepared.entry.plan().spec().game_ticks(),
            )?;
            io.checkpoint(&work.current(c)?, true)?;
            let current = narrowed(work, &connect, state.authority_tick)?;
            let native = native_context(&current, &state.room_clock_grants)?;
            let mut source = io.connect(&config, &native, true)?;
            let current = narrowed(work, &operation, state.authority_tick)?;
            let native = native_context(&current, &state.room_clock_grants)?;
            let mut owner = state.room_clock.take().ok_or_else(exhausted)?;
            let mut budget = Some(guard_budget);
            let outcome = owner.commit_guarded(
                &mut source,
                seal,
                true,
                &current,
                &native,
                |edge, _, current| {
                    if edge != Edge::Commit {
                        return Err(error(
                            ErrorCode::InternalInvariantViolation,
                            "wrong clock commit boundary",
                        ));
                    }
                    let reserved = budget.take().ok_or_else(exhausted)?;
                    guard(state, &spec, current, &reserved, work, audit, io)
                },
            );
            state.room_clock = Some(owner);
            retain_floor(state);
            let view = outcome?;
            retain_readiness(state, &view);
            let checked = inspect_current(state, work, &final_check)?;
            io.checkpoint(&work.current(c)?, true)?;
            Ok(json!({"ok":true,"room_clock":display(&checked,0,8)?,
                "game_unpause_requested":true,"goal_sample_published":false,
                "readiness_acquisition_retained":true,"readiness_earns_stability_credit":false,
                "next_step":"reconcile_clock_then_acquire_a_separate_whole_plan_construction_sample"}))
        }
        Action::Wait(id) | Action::Cancel(id) => {
            let cancel = matches!(action, Action::Cancel(_));
            exact(state, id)?;
            let config = configuration(state)?;
            let initial = work.take(c, VIEW_BYTES)?;
            let connect = work.take(c, CONNECT_BYTES)?;
            let operation = work.take(c, OPERATION_BYTES)?;
            let final_check = work.take(c, VIEW_BYTES)?;
            io.checkpoint(&work.current(c)?, cancel)?;
            let before = inspect_current(state, work, &initial)?;
            let needs_source = if cancel {
                before.cancellation_needs_source()
            } else {
                before.needs_reconciliation()
            };
            if cancel {
                if before.mode != dfmcp_adapter::bounded_run::journal::RunMode::Control {
                    return Err(denied("clock mode cannot request a safety pause"));
                }
                let current = narrowed(work, &operation, state.authority_tick)?;
                current.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
                let native = native_context(&current, &state.room_clock_grants)?;
                native.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
            } else if needs_source
                && before.mode == dfmcp_adapter::bounded_run::journal::RunMode::Offline
            {
                return Err(denied(
                    "offline clock inspection cannot contact native source",
                ));
            }
            let mut source = if needs_source {
                let current = narrowed(work, &connect, state.authority_tick)?;
                let native = native_context(&current, &state.room_clock_grants)?;
                Some(io.connect(&config, &native, cancel)?)
            } else {
                None
            };
            let current = narrowed(work, &operation, state.authority_tick)?;
            let native = native_context(&current, &state.room_clock_grants)?;
            let owner = state.room_clock.as_mut().ok_or_else(exhausted)?;
            let outcome = if cancel {
                owner.cancel(
                    source.as_mut().map(|source| source as &mut dyn RunSource),
                    &current,
                    &native,
                )
            } else if let Some(source) = source.as_mut() {
                owner.reconcile(source, &current, &native)
            } else {
                Ok(before)
            };
            retain_floor(state);
            outcome?;
            let checked = inspect_current(state, work, &final_check)?;
            io.checkpoint(&work.current(c)?, cancel)?;
            Ok(
                json!({"ok":true,"scope":"room_clock","room_clock":display(&checked,0,8)?,
                "new_unpause_attempted":false,"goal_sample_published":false,"replayed_locally":!needs_source}),
            )
        }
        Action::Inspect {
            id,
            head,
            offset,
            limit,
        } => {
            exact(state, id)?;
            let reserved = work.take(c, VIEW_BYTES)?;
            io.checkpoint(&work.current(c)?, false)?;
            let view = inspect_current(state, work, &reserved)?;
            if head != view.recovery.head {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "clock page belongs to another journal head",
                ));
            }
            io.checkpoint(&work.current(c)?, false)?;
            Ok(
                json!({"ok":true,"room_clock":display(&view,offset,limit)?,"native_calls":0,"game_mutations":0}),
            )
        }
    }
}

pub(super) fn run_action<S: EffectJournalStorage, N: BuildSource, I: Io>(
    state: &mut State<S, N>,
    mut c: OperationContext,
    op: &str,
    action: Action,
    started: Instant,
    io: &mut I,
) -> String {
    state.abandon();
    if let Some(other) = &mut state.clock {
        other.abandon_preparation();
    }
    state.room_clock_verified = false;
    let control = matches!(
        &action,
        Action::Prepare(_) | Action::Commit { .. } | Action::Cancel(_)
    );
    let may_have_effect = matches!(&action, Action::Commit { .. } | Action::Cancel(_));
    let failure = |cause: &dfmcp_core::DfmcpError| {
        let mut result = presentation::failure(cause);
        result["scope"] = json!("room_clock");
        result["effect_may_have_occurred"] = json!(may_have_effect);
        result["goal_sample_published"] = json!(false);
        if cause.code == ErrorCode::EffectIndeterminate {
            result["error"]["message"] = json!(
                "Retain and reconcile this exact room clock run; do not repeat unpause or publish fresh completion proof while it is unsettled."
            );
        }
        result
    };
    let packet = |state: &State<S, N>, mut result: Value, c: &OperationContext| {
        let current = match state.disclosure_context(c) {
            Ok(current) => current,
            Err(cause) => return super::unbound(op, &cause),
        };
        if let Some(owner) = &state.room_clock {
            let disclosure = native_context(&current, &state.room_clock_grants)
                .and_then(|native| owner.authorize_disclosure(&current, &native));
            if let Err(cause) = disclosure {
                return super::unbound(op, &cause);
            }
        }
        state.attach_batch(&mut result, state.historical.as_ref(), false);
        presentation::packet(
            op,
            result,
            Some(&current),
            Some(&state.binding),
            None,
            state.historical.as_ref(),
            Some(&state.policy),
            None,
            None,
        )
    };
    let mut work = match Work::new(&c, started) {
        Ok(work) => work,
        Err(cause) => {
            state.withdraw_monitor();
            state.abandon_all();
            retain_floor(state);
            c.anchor.tick = GameTick(c.anchor.tick.get().max(state.authority_tick));
            return packet(state, failure(&cause), &c);
        }
    };
    let page = match &action {
        Action::Inspect { offset, limit, .. } => (*offset, *limit),
        _ => (0, 8),
    };
    let advancement = matches!(&action, Action::Prepare(_) | Action::Commit { .. });
    if !advancement && let Some(monitor) = &mut state.room_completion {
        monitor.verified = false;
        monitor.store_verified = false;
    }

    let opened = (|| {
        if matches!(&action, Action::Prepare(_)) && state.room_clock.is_none() {
            open(state, &c, &mut work, io)?;
        }
        Ok::<_, dfmcp_core::DfmcpError>(())
    })();
    let publication = work.take(&c, VIEW_BYTES);
    let other_publication = if state.clock_config.is_some() || state.clock.is_some() {
        work.take(&c, clock::VIEW_BYTES).map(Some)
    } else {
        Ok(None)
    };
    let mut optional_work = Work {
        deadline: work.deadline,
        bytes: work.bytes,
    };
    let room_publication = match room_monitor::Publication::reserve_growing(
        state,
        &c,
        &mut optional_work,
        if advancement {
            dfmcp_adapter::room_condition::MAX_SAMPLE as u64 + 128 * 1024
        } else {
            8192
        },
    ) {
        Ok(reserved) => {
            work.bytes = optional_work.bytes;
            Ok(reserved)
        }
        Err(cause) if advancement => Err(cause),
        Err(_) => Ok(None),
    };
    let outcome = (|| {
        opened?;
        publication.as_ref().map_err(Clone::clone)?;
        other_publication.as_ref().map_err(Clone::clone)?;
        room_publication.as_ref().map_err(Clone::clone)?;
        perform(state, &c, &mut work, action, io)
    })();
    retain_floor(state);
    c.anchor.tick = GameTick(c.anchor.tick.get().max(state.authority_tick));
    let failed = outcome.is_err();
    let mut result = match outcome {
        Ok(result) => result,
        Err(cause) => {
            state.withdraw_monitor();
            state.abandon_all();
            state.room_clock_verified = false;
            failure(&cause)
        }
    };
    let rendered = packet(state, result.clone(), &c);
    let expected = state.room_clock_recovery;
    let expected_other = state.clock_recovery;
    let checked = (|| {
        if rendered.len() as u64 > presentation::OUTPUT_BYTES {
            return Err(exhausted());
        }
        io.checkpoint(&work.current(&c)?, control)?;
        if let Some(mut reserved) = room_publication? {
            reserved.retain_current_head(state)?;
            let checked = reserved.verify(state, expected_other, false, !advancement, |_| Ok(()));
            if advancement {
                checked?;
            } else if checked.is_err()
                && let Some(monitor) = &mut state.room_completion
            {
                monitor.verified = false;
                monitor.store_verified = false;
            }
        }
        if let Some(reserved) = other_publication.as_ref().map_err(Clone::clone)? {
            let context = narrowed(&work, reserved, state.authority_tick)?;
            let other = clock::inspect(state, &context);
            if advancement {
                other?;
                if state.clock_recovery != expected_other || clock::blocks_sampling(state) {
                    return Err(error(
                        ErrorCode::EffectIndeterminate,
                        "other configured clock is unsettled or changed at room-clock publication",
                    ));
                }
            }
        }
        if expected.is_some() {
            let view = inspect_current(state, &work, publication.as_ref().map_err(Clone::clone)?)?;
            if Some(view.recovery) != expected {
                return Err(error(
                    ErrorCode::CorruptLedger,
                    "room clock changed at final publication",
                ));
            }
            // Final custody flags and floors govern the published history page.
            if result["ok"] == true {
                result["room_clock"] = display(&view, page.0, page.1)?;
            }
        }
        work.current(&c)?;
        Ok(())
    })();
    match checked {
        Ok(()) => {
            if failed {
                state.room_clock_verified = false;
            }
            c.anchor.tick = GameTick(c.anchor.tick.get().max(state.authority_tick));
            let output = packet(state, result, &c);
            if output.len() as u64 > presentation::OUTPUT_BYTES || work.current(&c).is_err() {
                state.withdraw_monitor();
                state.abandon_all();
                state.room_clock_verified = false;
                packet(state, failure(&exhausted()), &c)
            } else {
                output
            }
        }
        Err(cause) => {
            state.withdraw_monitor();
            state.abandon_all();
            state.room_clock_verified = false;
            retain_floor(state);
            c.anchor.tick = GameTick(c.anchor.tick.get().max(state.authority_tick));
            packet(state, failure(&cause), &c)
        }
    }
}
