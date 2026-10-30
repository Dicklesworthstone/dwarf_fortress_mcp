//! Separately reviewed finite clock control for one whole original blueprint.
//!
//! Readiness captures authorize a bounded run; they are never published as a
//! completion sample. The native run journal owns dispatch and reconciliation.
use super::{Action, Query, State, Work, batch, error, exhausted, monitor, presentation};
use dfmcp_adapter::bounded_run::RunSpec;
use dfmcp_adapter::bounded_run::journal::RunBinding;
use dfmcp_adapter::bounded_run::rpc::{RunManifest, RunSource};
use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
use dfmcp_adapter::dig_blueprint::clock::{
    Clock, ClockRunView, ClockView, Edge, OPERATION_BYTES, Recovery, RunRequest,
};
use dfmcp_adapter::dig_blueprint::monitor::MonitorSpec;
use dfmcp_adapter::dig_designation::journal::DigMode;
use dfmcp_adapter::dig_designation::journal::session::COMPLETE_INVENTORY_BYTES;
use dfmcp_adapter::dig_designation::rpc::DigSource;
use dfmcp_adapter::live_map::LiveMapObservation;
use dfmcp_core::{
    Capability, CapabilityGrant, Digest32, ErrorCode, FortressId, GameTick, ObservationCursor,
    OperationContext, Result, RiskTier, StateAnchor,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub(super) const VIEW_BYTES: u64 = dfmcp_adapter::dig_blueprint::clock::VIEW_BYTES;
pub(super) const OPEN_BYTES: u64 = 3 * VIEW_BYTES;
const CONNECT_BYTES: u64 = 4 * 1024 * 1024;
const AUDIT_BYTES: u64 = COMPLETE_INVENTORY_BYTES + batch::PUBLICATION_BYTES + monitor::FINAL_BYTES;
const GUARD_BYTES: u64 = 2 * AUDIT_BYTES + dfmcp_adapter::dig_blueprint::monitor::io::READ_BYTES;

fn denied(message: &str) -> dfmcp_core::DfmcpError {
    error(ErrorCode::CapabilityDenied, message)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Config {
    pub path: PathBuf,
    pub initialize: bool,
    pub binding: RunBinding,
    pub recovery: Option<Recovery>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigInput {
    path: String,
    initialize: bool,
    source: SourceInput,
    recovery: Option<RecoveryInput>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceInput {
    endpoint: String,
    generation: u64,
    df_version: String,
    dfhack_version: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryInput {
    journal_id: String,
    head: String,
}

fn no_null(raw: &str, maximum: usize) -> Result<()> {
    if raw.is_empty() || raw.len() > maximum {
        return Err(exhausted());
    }
    let value: Value = serde_json::from_str(raw).map_err(|_| {
        error(
            ErrorCode::InvalidRequest,
            "invalid closed blueprint clock JSON",
        )
    })?;
    if value
        .as_object()
        .is_none_or(|fields| fields.values().any(Value::is_null))
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "omit optional clock values instead of null",
        ));
    }
    Ok(())
}
impl Config {
    pub fn parse(raw: &str) -> Result<Self> {
        no_null(raw, 8192)?;
        let value: ConfigInput = serde_json::from_str(raw).map_err(|_| {
            error(
                ErrorCode::InvalidRequest,
                "invalid closed blueprint clock configuration",
            )
        })?;
        if value.path.len() > 4096
            || !value.path.starts_with('/')
            || value.path.contains('\0')
            || value.path[1..]
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
            || value.initialize == value.recovery.is_some()
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "clock initialization, path or recovery differs",
            ));
        }
        let endpoint: SocketAddr = value.source.endpoint.parse().map_err(|_| {
            error(
                ErrorCode::InvalidRequest,
                "clock endpoint requires canonical numeric loopback",
            )
        })?;
        if endpoint.to_string() != value.source.endpoint {
            return Err(error(
                ErrorCode::InvalidRequest,
                "noncanonical clock endpoint",
            ));
        }
        let binding = RunBinding::new(
            endpoint,
            RunManifest {
                generation: value.source.generation,
                df_version: value.source.df_version,
                dfhack_version: value.source.dfhack_version,
            },
        )?;
        let recovery = value
            .recovery
            .map(|value| {
                let journal_id = presentation::digest(&value.journal_id)?;
                let head = presentation::digest(&value.head)?;
                if journal_id == Digest32::ZERO || head == Digest32::ZERO {
                    return Err(error(
                        ErrorCode::InvalidRequest,
                        "reserved blueprint clock recovery identity",
                    ));
                }
                Ok(Recovery { journal_id, head })
            })
            .transpose()?;
        Ok(Self {
            path: value.path.into(),
            initialize: value.initialize,
            binding,
            recovery,
        })
    }
    pub fn matches(&self, spec: &MonitorSpec) -> Result<()> {
        if self.binding.endpoint() != spec.source().endpoint()
            || self.binding.manifest().df_version != spec.source().df_version()
            || self.binding.manifest().dfhack_version != spec.source().dfhack_version()
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "clock source differs from the original blueprint endpoint or software",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Request {
    pub monitor_id: Digest32,
    pub game_ticks: u32,
    pub wall_millis: u32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestInput {
    monitor_id: String,
    game_ticks: u32,
    wall_millis: u32,
}
impl Request {
    pub fn parse(raw: &str) -> Result<Self> {
        no_null(raw, 2048)?;
        let value: RequestInput = serde_json::from_str(raw).map_err(|_| {
            error(
                ErrorCode::InvalidRequest,
                "invalid closed blueprint clock request",
            )
        })?;
        let monitor_id = presentation::digest(&value.monitor_id)?;
        if monitor_id == Digest32::ZERO {
            return Err(error(
                ErrorCode::InvalidRequest,
                "reserved monitor identity",
            ));
        }
        RunSpec::new(value.game_ticks, value.wall_millis)?;
        Ok(Self {
            monitor_id,
            game_ticks: value.game_ticks,
            wall_millis: value.wall_millis,
        })
    }
    fn typed(&self) -> RunRequest {
        RunRequest {
            monitor_id: self.monitor_id,
            game_ticks: self.game_ticks,
            wall_millis: self.wall_millis,
        }
    }
}

pub(super) trait ClockIo<C: EffectJournalStorage> {
    type Source: RunSource;
    /// Retain a header already published by open, including when a later
    /// boundary or owner-construction check prevents returning its owner.
    fn opening_recovery(&self) -> Option<Recovery> {
        None
    }
    fn open(
        &mut self,
        spec: &MonitorSpec,
        c: &OperationContext,
        n: &OperationContext,
    ) -> Result<Clock<C>>;
    fn connect(&mut self, n: &OperationContext, control: bool) -> Result<Self::Source>;
    fn checkpoint(&mut self, c: &OperationContext, control: bool) -> Result<()>;
}

pub(super) fn native_context(
    c: &OperationContext,
    grants: &[CapabilityGrant],
) -> Result<OperationContext> {
    let native = OperationContext {
        session_id: c.session_id,
        request_id: c.request_id,
        anchor: StateAnchor {
            fortress_id: FortressId::NIL,
            cursor: ObservationCursor::ORIGIN,
            tick: c.anchor.tick,
            state_hash: Digest32::ZERO,
        },
        budget: c.budget,
        cancellation_requested: c.cancellation_requested,
        grants: grants.to_vec(),
    };
    native.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    Ok(native)
}

pub(super) fn recovery(value: Recovery) -> Value {
    json!({"journal_id":value.journal_id.to_string(),"head":value.head.to_string()})
}

fn run_row(value: &ClockRunView, review: bool) -> Value {
    let plan = value.entry.plan();
    let native = value.entry.native().map(|record| {
        json!({
            "phase":record.phase().as_str(),
            "reason":record.reason().as_str(),
            "unpause_attempted":record.unpause_attempted(),
            "historical_pause_verified":record.pause_verified(),
            "observed_tick":record.observed_tick(),
            "observed_ticks_advanced":record.observed_ticks_advanced(),
            "observed_tick_overshoot":record.observed_tick_overshoot(),
            "receipt":record.receipt().to_string()
        })
    });
    json!({
        "key":plan.key(),"plan_digest":plan.digest().to_string(),
        "review_seal":review.then_some(value.review_seal).flatten().map(|v|v.to_string()),
        "state":value.entry.state().as_str(),"terminal":value.entry.terminal(),
        "unresolved":value.entry.unresolved(),"before_tick":plan.before().tick(),
        "source_generation":plan.before().generation(),
        "source_sequence":plan.before().sequence(),"game_ticks":plan.spec().game_ticks(),
        "wall_millis":plan.spec().wall_ms(),"native":native
    })
}

pub(super) fn reference<C: EffectJournalStorage>(clock: &Clock<C>) -> Value {
    let view = clock.cached_view();
    json!({
        "monitor_id":clock.spec().id().to_string(),
        "blueprint_id":clock.spec().parent().id().to_string(),
        "recovery":recovery(view.recovery),"mode":view.mode.as_str(),
        "history_verified":false,"authority_tick_floor":view.high_tick,
        "run_count":view.runs.len(),"pending":view.pending,
        "unresolved":view.unresolved,"may_be_running":view.may_be_running(),
        "active_run":view.active().map(|value|run_row(value,false)),
        "retry_commit_permitted":false,"current_pause_proven":false,
        "sampled_excavation_condition_proven":false,"shared_process_identity_proven":false,
        "mining_causality_proven":false
    })
}

pub(super) fn display(view: &ClockView, offset: usize, limit: usize) -> Result<Value> {
    if !(1..=8).contains(&limit)
        || offset > view.runs.len()
        || (offset == view.runs.len() && offset != 0)
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "invalid bounded clock history page",
        ));
    }
    let rows: Vec<_> = view
        .runs
        .iter()
        .skip(offset)
        .take(limit)
        .map(|value| run_row(value, true))
        .collect();
    let readiness = view.readiness.as_ref().map(|value| {
        json!({
            "tick":value.tick(),"digest":value.digest().to_string(),
            "pending_tiles":value.pending_tiles(),"cleared_tiles":value.cleared_tiles(),
            "historical_readiness_only":true,"goal_sample_published":false,
            "structural_safety_proven":false,"worker_reachability_proven":false
        })
    });
    let next_offset = (offset + limit < view.runs.len()).then_some(offset + limit);
    let next_query = next_offset.map(|offset| {
        json!({
            "mode":"blueprint_clock","monitor_id":view.monitor_id.to_string(),
            "expected_head":view.recovery.head.to_string(),"offset":offset,"limit":limit
        })
    });
    Ok(json!({
        "monitor_id":view.monitor_id.to_string(),"recovery":recovery(view.recovery),
        "history_verified":true,"mode":view.mode.as_str(),"run_count":view.runs.len(),
        "pending":view.pending,"unresolved":view.unresolved,
        "needs_reconciliation":view.needs_reconciliation(),"may_be_running":view.may_be_running(),
        "fresh_goal_sampling_permitted":view.can_sample_goal() && !view.pending,
        "authority_tick_floor":view.high_tick,"source_sequence_floor":view.high_sequence,
        "active_run":view.active().map(|value|run_row(value,true)),"readiness":readiness,
        "current_pause_proven":false,"sampled_excavation_condition_proven":false,
        "shared_process_identity_proven":false,"mining_causality_proven":false,
        "source_identity_semantics":"independent_run_incarnation_without_named_fortress_identity",
        "clock_floor_semantics":"native_journal_floors_and_process_local_consumed_read_floors",
        "runs_page":{"offset":offset,"total":view.runs.len(),"runs":rows,
            "next_offset":next_offset,"next_query":next_query}
    }))
}

fn retain_floor<S, N, B, C>(state: &mut State<S, N, B, C>)
where
    S: EffectJournalStorage,
    N: DigSource,
    B: EffectJournalStorage,
    C: EffectJournalStorage,
{
    if let Some(clock) = &state.clock {
        state.authority_tick = state.authority_tick.max(clock.high_tick());
        state.clock_recovery = Some(clock.recovery());
    }
}

pub(super) fn blocks_sampling<S, N, B, C>(state: &State<S, N, B, C>) -> bool
where
    S: EffectJournalStorage,
    N: DigSource,
    B: EffectJournalStorage,
    C: EffectJournalStorage,
{
    match &state.clock {
        Some(clock) => {
            let view = clock.cached_view();
            view.pending || view.unresolved
        }
        None => state.clock_recovery.is_some() || state.clock_config.is_some(),
    }
}

pub(super) fn inspect<S, N, B, C>(
    state: &mut State<S, N, B, C>,
    c: &OperationContext,
) -> Result<Option<ClockView>>
where
    S: EffectJournalStorage,
    N: DigSource,
    B: EffectJournalStorage,
    C: EffectJournalStorage,
{
    if state.clock.is_none() {
        if blocks_sampling(state) {
            return Err(denied(
                "configured clock custody must be opened before fresh blueprint sampling",
            ));
        }
        return Ok(None);
    }
    let mut current = c.clone();
    current.anchor.tick = GameTick(current.anchor.tick.get().max(state.authority_tick));
    let native = native_context(&current, &state.clock_grants)?;
    let outcome = state
        .clock
        .as_mut()
        .ok_or_else(exhausted)?
        .inspect(&current, &native);
    retain_floor(state);
    outcome.map(Some)
}

fn narrowed(work: &Work, reserved: &OperationContext, tick: u64) -> Result<OperationContext> {
    let mut c = reserved.clone();
    c.anchor.tick = GameTick(c.anchor.tick.get().max(tick));
    c.budget.max_wall_millis = work.current(&c)?.budget.max_wall_millis;
    Ok(c)
}

fn inspect_current<S, N, B, C>(
    state: &mut State<S, N, B, C>,
    work: &Work,
    reserved: &OperationContext,
) -> Result<ClockView>
where
    S: EffectJournalStorage,
    N: DigSource,
    B: EffectJournalStorage,
    C: EffectJournalStorage,
{
    let current = narrowed(work, reserved, state.authority_tick)?;
    inspect(state, &current)?.ok_or_else(exhausted)
}

/// Establish exact configured clock custody before any fresh goal sample.
/// The caller must forward the same started instant to the monitor owner.
pub(super) fn ensure_for_sample<S, N, B, C, I>(
    state: &mut State<S, N, B, C>,
    c: &mut OperationContext,
    started: Instant,
    io: &mut I,
) -> Result<()>
where
    S: EffectJournalStorage,
    N: DigSource,
    B: EffectJournalStorage,
    C: EffectJournalStorage,
    I: ClockIo<C>,
{
    if state.clock.is_some() {
        return Ok(());
    }
    let Some(config) = state.clock_config.clone() else {
        return if state.clock_recovery.is_some() {
            Err(denied(
                "retained clock recovery cannot be omitted before sampling",
            ))
        } else {
            Ok(())
        };
    };
    if config.initialize && state.clock_recovery.is_some() {
        return Err(denied("retained clock custody cannot be initialized again"));
    }
    let bytes = OPEN_BYTES + VIEW_BYTES + monitor::FINAL_BYTES;
    let mut work = Work::new(c, started)?;
    let opening = work.take(c, OPEN_BYTES)?;
    let inspection = work.take(c, VIEW_BYTES)?;
    let history = work.take(c, monitor::FINAL_BYTES)?;
    let store = state.monitor.as_mut().ok_or_else(|| {
        error(
            ErrorCode::StaleAnchor,
            "clock custody requires its retained original monitor",
        )
    })?;
    state.authority_tick = state.authority_tick.max(store.high_tick());
    c.anchor.tick = GameTick(c.anchor.tick.get().max(state.authority_tick));
    store.verify(&narrowed(&work, &history, state.authority_tick)?)?;
    let spec = store.spec().clone();
    config.matches(&spec)?;
    state
        .monitor_config
        .as_ref()
        .ok_or_else(|| denied("monitor source configuration absent"))?
        .matches(&spec)?;
    let current = narrowed(&work, &opening, state.authority_tick)?;
    io.checkpoint(&current, config.initialize)?;
    let native = native_context(&current, &state.clock_grants)?;
    let opened = io.open(&spec, &current, &native);
    if let Some(recovery) = io.opening_recovery() {
        state.clock_recovery = Some(recovery);
    }
    let clock = opened?;
    let matches = clock.spec() == &spec && clock.binding() == &config.binding;
    state.clock_recovery = Some(clock.recovery());
    state.clock = Some(clock);
    retain_floor(state);
    c.anchor.tick = GameTick(c.anchor.tick.get().max(state.authority_tick));
    if !matches {
        return Err(error(
            ErrorCode::StaleAnchor,
            "opened clock differs from the configured original target",
        ));
    }
    let inspected = inspect_current(state, &work, &inspection);
    c.anchor.tick = GameTick(c.anchor.tick.get().max(state.authority_tick));
    inspected?;
    io.checkpoint(&work.current(c)?, config.initialize)?;
    work.current(c)?;
    c.budget.max_bytes = c
        .budget
        .max_bytes
        .checked_sub(bytes)
        .ok_or_else(exhausted)?;
    Ok(())
}

fn active_original<S, N, B, C>(
    state: &mut State<S, N, B, C>,
    spec: &MonitorSpec,
    c: &OperationContext,
) -> Result<()>
where
    S: EffectJournalStorage,
    N: DigSource,
    B: EffectJournalStorage,
    C: EffectJournalStorage,
{
    if c.budget.max_bytes < AUDIT_BYTES || state.control.mode() != DigMode::Control {
        return Err(denied(
            "blueprint clock control requires prepaid writable original custody",
        ));
    }
    if state.policy.checkpoint_policy()
        != dfmcp_adapter::dig_control_policy::DigCheckpointPolicy::DisposableFortress
    {
        return Err(error(
            ErrorCode::CheckpointRequired,
            "new clock advancement requires the current explicit disposable-fortress checkpoint exception",
        ));
    }
    let floor = state.authority_tick.max(state.control.high_tick()).max(
        state
            .monitor
            .as_ref()
            .map_or(spec.start_tick(), |store| store.high_tick()),
    );
    let mut current = c.clone();
    current.anchor.tick = GameTick(current.anchor.tick.get().max(floor));
    spec.authorize(&current, true)?;
    current.authorize(
        Capability::Plan,
        RiskTier::Guarded,
        &[],
        Some(spec.parent().binding().scope()),
    )?;
    current.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
    let started = Instant::now();
    let (parent, view, records) = monitor::audit(state, &current)?;
    if state
        .blueprint
        .as_ref()
        .is_none_or(|parent| parent.stopped() || parent.is_fenced())
    {
        return Err(denied(
            "stopped or fenced original blueprint cannot advance the game clock",
        ));
    }
    spec.verify_original(&parent, &view, &records)?;
    state
        .clock_config
        .as_ref()
        .ok_or_else(|| denied("clock source configuration absent"))?
        .matches(spec)?;
    state
        .monitor_config
        .as_ref()
        .ok_or_else(|| denied("monitor source configuration absent"))?
        .matches(spec)?;
    let store = state
        .monitor
        .as_mut()
        .filter(|store| store.spec() == spec)
        .ok_or_else(|| {
            error(
                ErrorCode::StaleAnchor,
                "clock original monitor is not retained unchanged",
            )
        })?;
    let mut history = current;
    history.budget.max_bytes = monitor::FINAL_BYTES;
    history.budget.max_wall_millis = history
        .budget
        .max_wall_millis
        .checked_sub(u64::try_from(started.elapsed().as_millis()).map_err(|_| exhausted())?)
        .filter(|left| *left > 0)
        .ok_or_else(exhausted)?;
    history.anchor.tick = GameTick(history.anchor.tick.get().max(store.high_tick()));
    store.verify(&history)?;
    if store.is_fenced()
        || store.state().status().terminal()
        || store.state().attempts() >= spec.policy().max_samples()
    {
        return Err(denied(
            "clock advancement requires an unfinished unfenced monitor with remaining samples",
        ));
    }
    Ok(())
}

fn preflight<S, N, B, C>(
    state: &State<S, N, B, C>,
    spec: &MonitorSpec,
    c: &OperationContext,
    ticks: u32,
) -> Result<()>
where
    S: EffectJournalStorage,
    N: DigSource,
    B: EffectJournalStorage,
    C: EffectJournalStorage,
{
    dfmcp_adapter::dig_blueprint::monitor::io::preflight(spec, c)?;
    if u64::from(ticks) > c.budget.max_game_ticks
        || u64::from(ticks) > spec.policy().max_gap_ticks()
        || state
            .clock
            .as_ref()
            .is_some_and(|clock| !clock.is_control())
    {
        return Err(exhausted());
    }
    let horizon = c
        .anchor
        .tick
        .get()
        .max(state.authority_tick)
        .checked_add(u64::from(ticks))
        .filter(|tick| *tick <= spec.policy().deadline_tick())
        .ok_or_else(exhausted)?;
    let mut future = c.clone();
    future.anchor.tick = GameTick(horizon);
    spec.authorize(&future, true)?;
    future.authorize(
        Capability::Plan,
        RiskTier::Guarded,
        &[],
        Some(spec.parent().binding().scope()),
    )?;
    future.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
    let native = native_context(&future, &state.clock_grants)?;
    native.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
    native.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
    Ok(())
}

fn guard<S, N, B, C, M, I>(
    state: &mut State<S, N, B, C>,
    spec: &MonitorSpec,
    edge: &OperationContext,
    reserved: &OperationContext,
    work: &Work,
    maps: &mut M,
    io: &mut I,
) -> Result<LiveMapObservation>
where
    S: EffectJournalStorage,
    N: DigSource,
    B: EffectJournalStorage,
    C: EffectJournalStorage,
    M: monitor::MonitorIo<B>,
    I: ClockIo<C>,
{
    let deadline = Instant::now()
        .checked_add(Duration::from_millis(edge.budget.max_wall_millis))
        .ok_or_else(exhausted)?;
    let remaining = || {
        let left = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(exhausted)?
            .as_millis();
        u64::try_from(left)
            .ok()
            .filter(|left| *left > 0)
            .ok_or_else(exhausted)
    };
    let mut c = narrowed(
        work,
        reserved,
        edge.anchor.tick.get().max(state.authority_tick),
    )?;
    c.budget.max_wall_millis = c.budget.max_wall_millis.min(remaining()?);
    c.budget.max_bytes = AUDIT_BYTES;
    io.checkpoint(&c, true)?;
    maps.checkpoint(&c)?;
    active_original(state, spec, &c)?;
    let mut read = narrowed(work, &c, state.authority_tick)?;
    read.budget.max_wall_millis = read.budget.max_wall_millis.min(remaining()?);
    read.budget.max_bytes = dfmcp_adapter::dig_blueprint::monitor::io::READ_BYTES;
    let raw = maps.read_map(spec, &read)?;
    state.authority_tick = state.authority_tick.max(raw.tick().get());
    let mut final_check = narrowed(work, &c, state.authority_tick)?;
    final_check.budget.max_wall_millis = final_check.budget.max_wall_millis.min(remaining()?);
    final_check.budget.max_bytes = AUDIT_BYTES;
    io.checkpoint(&final_check, true)?;
    maps.checkpoint(&final_check)?;
    active_original(state, spec, &final_check)?;
    let (candidate, breaks_continuity) = {
        let previous = state.monitor.as_ref().ok_or_else(exhausted)?.state();
        let candidate = previous.sample(raw.clone())?;
        let breaks = previous.streak() > 0
            && (candidate.streak() < previous.streak()
                || candidate.since_tick() != previous.since_tick());
        (candidate, breaks)
    };
    if breaks_continuity {
        // Readiness contributes no positive proof. Its known contradictions
        // still invalidate an older matching window: the next sampled read
        // durably resets that window before contacting the map source.
        state.withdraw_monitor();
        if state.monitor.is_none() {
            return Err(error(
                ErrorCode::CorruptLedger,
                "monitor continuity could not be withdrawn",
            ));
        }
    }
    if candidate.status() == dfmcp_adapter::dig_blueprint::monitor::Status::Satisfied {
        return Err(denied(
            "fresh terrain already satisfies the original goal; settle or cancel the retained clock, then publish a monitor sample",
        ));
    }
    work.current(&final_check)?;
    remaining()?;
    Ok(raw)
}

fn exact<S, N, B, C>(state: &State<S, N, B, C>, id: Digest32) -> Result<()>
where
    S: EffectJournalStorage,
    N: DigSource,
    B: EffectJournalStorage,
    C: EffectJournalStorage,
{
    if state
        .clock
        .as_ref()
        .is_some_and(|clock| clock.spec().id() == id)
    {
        Ok(())
    } else {
        Err(error(
            ErrorCode::StaleAnchor,
            "exact original blueprint clock is not retained",
        ))
    }
}

fn perform<S, N, B, C, M, I>(
    state: &mut State<S, N, B, C>,
    c: &OperationContext,
    work: &mut Work,
    action: Action,
    maps: &mut M,
    io: &mut I,
) -> Result<Value>
where
    S: EffectJournalStorage,
    N: DigSource,
    B: EffectJournalStorage,
    C: EffectJournalStorage,
    M: monitor::MonitorIo<B>,
    I: ClockIo<C>,
{
    match action {
        Action::PrepareClock(request) => {
            let spec = state
                .monitor
                .as_ref()
                .filter(|store| store.spec().id() == request.monitor_id)
                .map(|store| store.spec().clone())
                .ok_or_else(|| {
                    error(
                        ErrorCode::StaleAnchor,
                        "clock requires the exact existing blueprint monitor",
                    )
                })?;
            let initial = work.take(c, AUDIT_BYTES)?;
            let guard_budget = work.take(c, GUARD_BYTES)?;
            let opening = work.take(c, OPEN_BYTES)?;
            let initial_clock = work.take(c, VIEW_BYTES)?;
            let connect = work.take(c, CONNECT_BYTES)?;
            let operation = work.take(c, OPERATION_BYTES)?;
            let final_check = work.take(c, VIEW_BYTES)?;
            let initial = narrowed(work, &initial, state.authority_tick)?;
            active_original(state, &spec, &initial)?;
            preflight(
                state,
                &spec,
                &narrowed(work, &operation, state.authority_tick)?,
                request.game_ticks,
            )?;
            io.checkpoint(&work.current(c)?, true)?;
            if state.clock.is_none() {
                let config = state
                    .clock_config
                    .as_ref()
                    .ok_or_else(|| denied("clock is not configured"))?;
                if !config.initialize || state.clock_recovery.is_some() {
                    return Err(denied("clock recovery must open exact existing custody"));
                }
                let current = narrowed(work, &opening, state.authority_tick)?;
                let native = native_context(&current, &state.clock_grants)?;
                let expected = config.binding.clone();
                let opened = io.open(&spec, &current, &native);
                if let Some(recovery) = io.opening_recovery() {
                    state.clock_recovery = Some(recovery);
                }
                let clock = opened?;
                let matches = clock.spec() == &spec && clock.binding() == &expected;
                state.clock_recovery = Some(clock.recovery());
                state.clock = Some(clock);
                if !matches {
                    return Err(error(
                        ErrorCode::StaleAnchor,
                        "opened clock differs from the configured original target",
                    ));
                }
            }
            exact(state, request.monitor_id)?;
            let before = inspect_current(state, work, &initial_clock)?;
            if before.pending {
                return Err(error(
                    ErrorCode::EffectIndeterminate,
                    "settle the retained clock interval before preparing another",
                ));
            }
            if before.runs.len()
                >= dfmcp_adapter::dig_blueprint::clock::MAX_RUNS
                    .min(spec.policy().max_samples() as usize)
            {
                return Err(exhausted());
            }
            let current = narrowed(work, &connect, state.authority_tick)?;
            let native = native_context(&current, &state.clock_grants)?;
            let mut source = io.connect(&native, true)?;
            let current = narrowed(work, &operation, state.authority_tick)?;
            let native = native_context(&current, &state.clock_grants)?;
            let mut owner = state.clock.take().ok_or_else(exhausted)?;
            let mut budget = Some(guard_budget);
            let outcome = owner.prepare_guarded(
                &mut source,
                &request.typed(),
                &current,
                &native,
                |edge, _, current| {
                    if edge != Edge::Prepare {
                        return Err(error(
                            ErrorCode::InternalInvariantViolation,
                            "wrong clock prepare edge",
                        ));
                    }
                    let reserved = budget.take().ok_or_else(exhausted)?;
                    guard(state, &spec, current, &reserved, work, maps, io)
                },
            );
            state.clock = Some(owner);
            retain_floor(state);
            outcome?;
            let checked = inspect_current(state, work, &final_check)?;
            io.checkpoint(&work.current(c)?, true)?;
            Ok(json!({
                "ok":true,"blueprint_clock":display(&checked,0,8)?,
                "confirmation_required":true,"game_unpause_requested":false,
                "goal_sample_published":false
            }))
        }
        Action::CommitClock { monitor_id, seal } => {
            exact(state, monitor_id)?;
            let spec = state.clock.as_ref().ok_or_else(exhausted)?.spec().clone();
            let initial = work.take(c, AUDIT_BYTES)?;
            let initial_clock = work.take(c, VIEW_BYTES)?;
            let guard_budget = work.take(c, GUARD_BYTES)?;
            let connect = work.take(c, CONNECT_BYTES)?;
            let operation = work.take(c, OPERATION_BYTES)?;
            let final_check = work.take(c, VIEW_BYTES)?;
            let initial = narrowed(work, &initial, state.authority_tick)?;
            active_original(state, &spec, &initial)?;
            let before = inspect_current(state, work, &initial_clock)?;
            let prepared = before
                .active()
                .filter(|run| run.review_seal == Some(seal))
                .ok_or_else(|| {
                    denied("clock commit requires its current separately reviewed seal")
                })?;
            preflight(
                state,
                &spec,
                &narrowed(work, &operation, state.authority_tick)?,
                prepared.entry.plan().spec().game_ticks(),
            )?;
            io.checkpoint(&work.current(c)?, true)?;
            let current = narrowed(work, &connect, state.authority_tick)?;
            let native = native_context(&current, &state.clock_grants)?;
            let mut source = io.connect(&native, true)?;
            let current = narrowed(work, &operation, state.authority_tick)?;
            let native = native_context(&current, &state.clock_grants)?;
            let mut owner = state.clock.take().ok_or_else(exhausted)?;
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
                            "wrong clock commit edge",
                        ));
                    }
                    let reserved = budget.take().ok_or_else(exhausted)?;
                    guard(state, &spec, current, &reserved, work, maps, io)
                },
            );
            state.clock = Some(owner);
            retain_floor(state);
            outcome?;
            let checked = inspect_current(state, work, &final_check)?;
            io.checkpoint(&work.current(c)?, true)?;
            Ok(json!({
                "ok":true,"blueprint_clock":display(&checked,0,8)?,
                "game_unpause_requested":true,"goal_sample_published":false,
                "next_step":"reconcile_clock_then_acquire_a_separate_whole_blueprint_goal_sample"
            }))
        }
        Action::WaitClock(id) | Action::CancelClock(id) => {
            let cancel = matches!(action, Action::CancelClock(_));
            exact(state, id)?;
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
                    return Err(denied("clock recovery mode cannot request a safety pause"));
                }
                let current = narrowed(work, &operation, state.authority_tick)?;
                current.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
                let native = native_context(&current, &state.clock_grants)?;
                native.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
            } else if needs_source
                && before.mode == dfmcp_adapter::bounded_run::journal::RunMode::Offline
            {
                return Err(denied(
                    "offline clock inspection cannot contact the native source",
                ));
            }
            let mut source = if needs_source {
                let current = narrowed(work, &connect, state.authority_tick)?;
                let native = native_context(&current, &state.clock_grants)?;
                Some(io.connect(&native, cancel)?)
            } else {
                None
            };
            let current = narrowed(work, &operation, state.authority_tick)?;
            let native = native_context(&current, &state.clock_grants)?;
            let owner = state.clock.as_mut().ok_or_else(exhausted)?;
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
            Ok(json!({
                "ok":true,"scope":"clock","blueprint_clock":display(&checked,0,8)?,
                "new_unpause_attempted":false,"goal_sample_published":false,
                "replayed_locally":!needs_source
            }))
        }
        Action::Query(Query::BlueprintClock {
            monitor_id,
            expected_head,
            offset,
            limit,
        }) => {
            exact(state, presentation::digest(&monitor_id)?)?;
            let reserved = work.take(c, VIEW_BYTES)?;
            io.checkpoint(&work.current(c)?, false)?;
            let view = inspect_current(state, work, &reserved)?;
            if expected_head
                .as_deref()
                .map(presentation::digest)
                .transpose()?
                .is_some_and(|head| head != view.recovery.head)
            {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "clock page belongs to an older journal head",
                ));
            }
            io.checkpoint(&work.current(c)?, false)?;
            Ok(json!({
                "ok":true,"blueprint_clock":display(&view,offset.unwrap_or(0),limit.unwrap_or(8))?,
                "native_calls":0,"game_mutations":0
            }))
        }
        _ => Err(error(
            ErrorCode::InternalInvariantViolation,
            "wrong blueprint clock action",
        )),
    }
}

pub(super) fn run_action<S, N, B, C, M, I>(
    state: &mut State<S, N, B, C>,
    mut c: OperationContext,
    op: &str,
    action: Action,
    started: Instant,
    maps: &mut M,
    io: &mut I,
) -> String
where
    S: EffectJournalStorage,
    N: DigSource,
    B: EffectJournalStorage,
    C: EffectJournalStorage,
    M: monitor::MonitorIo<B>,
    I: ClockIo<C>,
{
    state.abandon();
    let control = matches!(
        &action,
        Action::PrepareClock(_) | Action::CommitClock { .. } | Action::CancelClock(_)
    );
    let packet = |state: &State<S, N, B, C>, result: Value, c: &OperationContext| {
        presentation::packet(
            op,
            super::retain_blueprint(state, result),
            Some(c),
            Some(&state.binding),
            None,
            None,
            None,
        )
    };
    let mut work = match Work::new(&c, started) {
        Ok(work) => work,
        Err(cause) => {
            state.withdraw_monitor();
            if let Some(clock) = &mut state.clock {
                clock.abandon_preparation();
            }
            return packet(state, presentation::failure(&cause), &c);
        }
    };
    let outcome = perform(state, &c, &mut work, action, maps, io);
    retain_floor(state);
    c.anchor.tick = GameTick(c.anchor.tick.get().max(state.authority_tick));
    let result = match outcome {
        Ok(result) => result,
        Err(cause) => {
            state.withdraw_monitor();
            if let Some(clock) = &mut state.clock {
                clock.abandon_preparation();
            }
            presentation::failure(&cause)
        }
    };
    let rendered = packet(state, result, &c);
    let final_check = (|| {
        if rendered.len() as u64 > presentation::OUTPUT_BYTES {
            return Err(exhausted());
        }
        io.checkpoint(&work.current(&c)?, control)?;
        work.current(&c)?;
        Ok(())
    })();
    match final_check {
        Ok(()) => rendered,
        Err(cause) => {
            state.withdraw_monitor();
            if let Some(clock) = &mut state.clock {
                clock.abandon_preparation();
            }
            packet(state, presentation::failure(&cause), &c)
        }
    }
}

#[cfg(test)]
#[path = "clock/tests.rs"]
mod tests;
