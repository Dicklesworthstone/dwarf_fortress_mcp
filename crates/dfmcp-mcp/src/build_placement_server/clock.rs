//! Separately reviewed finite run/1.13 advancement for the entire original
//! furnishing batch. Durable receipt-bracket readiness never earns goal stability.
use super::{
    State, Work, completion, error, exhausted, presentation, room_clock, room_monitor, runtime,
};
use dfmcp_adapter::bounded_run::RunSpec;
use dfmcp_adapter::bounded_run::journal::RunBinding;
use dfmcp_adapter::bounded_run::private_file::PrivateRunFile;
use dfmcp_adapter::bounded_run::rpc::{RunManifest, RunSource};
use dfmcp_adapter::build_placement::journal::{BuildMode, BuildSource};
use dfmcp_adapter::construction_plan::clock::{
    Clock, ClockRunView, ClockSample, ClockView, Edge, OPERATION_BYTES, Recovery, RunRequest,
};
use dfmcp_adapter::construction_plan::origin::MonitorDefinition;
use dfmcp_adapter::construction_plan::{LinkedSample, Manifest};
use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
use dfmcp_adapter::live_operations::{LiveOperationsObservation, OperationsProfile};
use dfmcp_core::{
    Capability, CapabilityGrant, Digest32, ErrorCode, FortressId, GameTick, ObservationCursor,
    OperationContext, Result, RiskTier, StateAnchor,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub(super) const VIEW_BYTES: u64 = dfmcp_adapter::construction_plan::clock::VIEW_BYTES;
pub(super) const OPEN_BYTES: u64 = 3 * VIEW_BYTES;
const CONNECT_BYTES: u64 = 4 * 1024 * 1024;
const SOURCE_BYTES: u64 = 24 * 1024 * 1024;
const DECODE_BYTES: u64 = 64 * 1024 * 1024;

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
            "invalid closed construction clock JSON",
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
                "invalid closed construction clock configuration",
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
                        "reserved construction clock recovery identity",
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
    pub fn matches(&self, spec: &MonitorDefinition) -> Result<()> {
        if self.binding.endpoint() != spec.origin().definition().binding().endpoint()
            || self.binding.manifest().df_version
                != spec.origin().definition().binding().df_version()
            || self.binding.manifest().dfhack_version
                != spec.origin().definition().binding().dfhack_version()
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "clock source differs from the original construction endpoint or software",
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
                "invalid closed construction clock request",
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

pub(super) enum Action {
    Prepare(Request),
    Commit {
        id: Digest32,
        seal: Digest32,
    },
    Wait(Digest32),
    Cancel(Digest32),
    Inspect {
        id: Digest32,
        head: Digest32,
        offset: usize,
        limit: usize,
    },
}

/// The effectful shell owns native sockets and inherited runtime checks. Every
/// decoded observation is returned before later host checks so its clock floor
/// cannot be lost merely because those checks refuse publication.
pub(super) trait ClockIo {
    type Source: RunSource;
    fn opening_recovery(&self) -> Option<Recovery> {
        None
    }
    fn open(
        &mut self,
        config: &Config,
        definition: &MonitorDefinition,
        operations: &Manifest,
        c: &OperationContext,
        n: &OperationContext,
    ) -> Result<Clock<PrivateRunFile>>;
    fn connect(
        &mut self,
        config: &Config,
        n: &OperationContext,
        control: bool,
    ) -> Result<Self::Source>;
    fn sample(
        &mut self,
        definition: &MonitorDefinition,
        c: &OperationContext,
    ) -> Result<LinkedSample>;
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

pub(super) fn run_row(value: &ClockRunView, review: bool) -> Value {
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
        "batch_id":clock.spec().origin().definition().id().to_string(),
        "recovery":recovery(view.recovery),"mode":view.mode.as_str(),
        "history_verified":false,"authority_tick_floor":view.high_tick,
        "run_count":view.runs.len(),"pending":view.pending,
        "unresolved":view.unresolved,"may_be_running":view.may_be_running(),
        "active_run":view.active().map(|value|run_row(value,false)),
        "retry_commit_permitted":false,"current_pause_proven":false,
        "sampled_construction_condition_proven":false,"shared_process_identity_proven":false,
        "construction_causality_proven":false
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
            "pending_count":value.pending_count(),"complete_count":value.complete_count(),
            "historical_readiness_only":true,"evidence_scope":"pre_dispatch_receipt_bracket","goal_sample_published":false,
            "current_usability_proven":false,"worker_reachability_proven":false
        })
    });
    let next_offset = (offset + limit < view.runs.len()).then_some(offset + limit);
    let next_query = next_offset.map(|offset| {
        json!({
            "mode":"construction_clock","monitor_id":view.monitor_id.to_string(),
            "head":view.recovery.head.to_string(),"offset":offset,"limit":limit
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
        "current_pause_proven":false,"sampled_construction_condition_proven":false,
        "shared_process_identity_proven":false,"construction_causality_proven":false,
        "source_identity_semantics":"independent_run_incarnation_without_named_fortress_identity",
        "clock_floor_semantics":"native_journal_floors_and_process_local_consumed_read_floors",
        "runs_page":{"offset":offset,"total":view.runs.len(),"runs":rows,
            "next_offset":next_offset,"next_query":next_query}
    }))
}

fn retain_floor<S: EffectJournalStorage, N: BuildSource>(state: &mut State<S, N>) {
    if let Some(clock) = &state.clock {
        state.authority_tick = state.authority_tick.max(clock.high_tick());
        state.clock_recovery = Some(clock.recovery());
    }
}

pub(super) fn blocks_sampling<S: EffectJournalStorage, N: BuildSource>(
    state: &State<S, N>,
) -> bool {
    match &state.clock {
        Some(clock) => {
            let view = clock.cached_view();
            !state.clock_verified || view.pending || view.unresolved
        }
        None => state.clock_recovery.is_some() || state.clock_config.is_some(),
    }
}

pub(super) fn inspect<S: EffectJournalStorage, N: BuildSource>(
    state: &mut State<S, N>,
    c: &OperationContext,
) -> Result<Option<ClockView>> {
    state.clock_verified = false;
    if state.clock.is_none() {
        return if state.clock_recovery.is_some() || state.clock_config.is_some() {
            Err(denied("configured construction clock custody is not open"))
        } else {
            Ok(None)
        };
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
    if outcome.is_ok() {
        state.clock_verified = true;
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
    if blocks_sampling(state) {
        return Err(error(
            ErrorCode::EffectIndeterminate,
            "settle and verify the retained clock before acquiring construction proof",
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
        .clock
        .as_ref()
        .is_some_and(|clock| clock.spec().id() == id)
    {
        Ok(())
    } else {
        Err(error(
            ErrorCode::StaleAnchor,
            "exact original construction clock is not retained",
        ))
    }
}
fn audit_bytes<S: EffectJournalStorage, N: BuildSource>(state: &State<S, N>) -> Result<u64> {
    let monitor = state
        .completion
        .as_ref()
        .ok_or_else(|| denied("original completion monitor absent"))?;
    completion::source_reserve(monitor.store.definition().origin())?
        .checked_add(monitor.store.byte_len() as u64 + 8192)
        .ok_or_else(exhausted)
}
fn active_original<S: EffectJournalStorage, N: BuildSource>(
    state: &mut State<S, N>,
    spec: &MonitorDefinition,
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
        .clock_config
        .as_ref()
        .ok_or_else(|| denied("clock configuration absent"))?
        .matches(spec)?;
    let monitor = state
        .completion
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
    if source.furniture_generation != state.binding.generation()
        || state
            .clock
            .as_ref()
            .is_some_and(|clock| &source.operations != clock.operations())
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
    spec: &MonitorDefinition,
    c: &OperationContext,
    ticks: u32,
) -> Result<()> {
    if u64::from(ticks) > c.budget.max_game_ticks
        || ticks > spec.goal().timing().max_gap
        || state.clock.as_ref().is_none_or(|owner| !owner.is_control())
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
    let native = native_context(&future, &state.clock_grants)?;
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
    let store = &state.completion.as_ref().ok_or_else(exhausted)?.store;
    let source = completion::source_reserve(store.definition().origin())?;
    // A reopened monitor may need two read-intent frames. Every source guard,
    // both complete journal checks, publication and decode are prepaid before
    // the clock can synchronize its Intent or DispatchStarted marker.
    source
        .checked_add(
            if state.room_clock_config.is_some() || state.room_clock.is_some() {
                room_clock::VIEW_BYTES
            } else {
                0
            },
        )
        .and_then(|bytes| {
            bytes.checked_mul(
                if state.room_clock_config.is_some() || state.room_clock.is_some() {
                    12
                } else {
                    10
                },
            )
        })
        .and_then(|bytes| bytes.checked_add(2 * (store.byte_len() as u64 + 8192)))
        .and_then(|bytes| bytes.checked_add(store.operation_reserve(false)))
        .and_then(|bytes| bytes.checked_add(store.operation_reserve(true)))
        .and_then(|bytes| bytes.checked_add(SOURCE_BYTES + DECODE_BYTES + 16_384))
        .ok_or_else(exhausted)
}

#[allow(clippy::too_many_arguments)]
fn guard<S: EffectJournalStorage, N: BuildSource, I: ClockIo>(
    state: &mut State<S, N>,
    spec: &MonitorDefinition,
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
    if state.room_clock_config.is_some() || state.room_clock.is_some() {
        let other = budget.take(room_clock::VIEW_BYTES, state.authority_tick)?;
        room_clock::ensure_for_sample(state, &other)?;
    }
    let mut monitor = state.completion.take().ok_or_else(exhausted)?;
    let outcome = (|| {
        let parent = monitor.parent.clone();
        let child = monitor.child.clone();
        let source_bytes = completion::source_reserve(spec.origin())?;
        let begin = budget.take(monitor.store.operation_reserve(false), state.authority_tick)?;
        {
            let mut admit =
                |definition: &MonitorDefinition,
                 progress: &dfmcp_adapter::construction_plan::Progress| {
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
                if state.room_clock_config.is_some() || state.room_clock.is_some() {
                    let other = budget.take(room_clock::VIEW_BYTES, current.anchor.tick.get())?;
                    room_clock::ensure_for_sample(state, &other)?;
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
        let sample = io.sample(spec, &read)?;
        // Retain the consumed source clock before later custody, runtime or
        // authority checks can refuse this completed acquisition. Its bounded
        // decode allowance was reserved before the native acquisition.
        let observed = LiveOperationsObservation::decode_profile(
            &sample.capture,
            sample.operations.generation,
            sample.operations.df_version.clone(),
            sample.operations.dfhack_version.clone(),
            OperationsProfile::PagedV1_4,
        )?;
        state.authority_tick = state.authority_tick.max(observed.jobs.tick().get());
        drop(observed);
        let history = budget.take(monitor.store.byte_len() as u64 + 8192, state.authority_tick)?;
        monitor.store.verify(&history)?;
        let publish_bytes = monitor
            .store
            .operation_reserve(true)
            .checked_add(8192)
            .ok_or_else(exhausted)?;
        let publish = budget.take(publish_bytes, state.authority_tick)?;
        let mut admit = |definition: &MonitorDefinition,
                         _: &dfmcp_adapter::construction_plan::Progress| {
            if definition.canonical_bytes() != spec.canonical_bytes() {
                return Err(denied("readiness names another original construction goal"));
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
            if state.room_clock_config.is_some() || state.room_clock.is_some() {
                let other = budget.take(room_clock::VIEW_BYTES, current.anchor.tick.get())?;
                room_clock::ensure_for_sample(state, &other)?;
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
                "readiness terminated the original construction goal; no new run is permitted",
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
    state.completion = Some(monitor);
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
        .clock_config
        .clone()
        .ok_or_else(|| denied("clock is not configured"))
}

fn perform<S: EffectJournalStorage, N: BuildSource, I: ClockIo>(
    state: &mut State<S, N>,
    c: &OperationContext,
    work: &mut Work,
    action: Action,
    io: &mut I,
) -> Result<Value> {
    match action {
        Action::Prepare(request) => {
            exact(state, request.monitor_id)?;
            let spec = state.clock.as_ref().ok_or_else(exhausted)?.spec().clone();
            let config = configuration(state)?;
            let audit = audit_bytes(state)?;
            let initial = work.take(c, audit)?;
            let guard_budget = work.take(c, readiness_reserve(state)?)?;
            let initial_clock = work.take(c, VIEW_BYTES)?;
            let connect = work.take(c, CONNECT_BYTES)?;
            let operation = work.take(c, OPERATION_BYTES)?;
            let final_check = work.take(c, VIEW_BYTES)?;
            if state.room_clock_config.is_some() || state.room_clock.is_some() {
                let other = work.take(c, room_clock::VIEW_BYTES)?;
                room_clock::ensure_for_sample(
                    state,
                    &narrowed(work, &other, state.authority_tick)?,
                )?;
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
                    "settle retained construction clock before another interval",
                ));
            }
            if before.runs.len()
                >= dfmcp_adapter::construction_plan::clock::MAX_RUNS
                    .min(spec.goal().timing().max_observations as usize)
            {
                return Err(exhausted());
            }
            let current = narrowed(work, &connect, state.authority_tick)?;
            let native = native_context(&current, &state.clock_grants)?;
            let mut source = io.connect(&config, &native, true)?;
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
                            "wrong clock prepare boundary",
                        ));
                    }
                    let reserved = budget.take().ok_or_else(exhausted)?;
                    guard(state, &spec, current, &reserved, work, audit, io)
                },
            );
            state.clock = Some(owner);
            retain_floor(state);
            let view = outcome?;
            retain_readiness(state, &view);
            let checked = inspect_current(state, work, &final_check)?;
            io.checkpoint(&work.current(c)?, true)?;
            Ok(
                json!({"ok":true,"construction_clock":display(&checked,0,8)?,
                "confirmation_required":true,"game_unpause_requested":false,"goal_sample_published":false,
                "readiness_acquisition_retained":true,"readiness_earns_stability_credit":false}),
            )
        }
        Action::Commit { id, seal } => {
            exact(state, id)?;
            let spec = state.clock.as_ref().ok_or_else(exhausted)?.spec().clone();
            let config = configuration(state)?;
            let audit = audit_bytes(state)?;
            let initial = work.take(c, audit)?;
            let initial_clock = work.take(c, VIEW_BYTES)?;
            let guard_budget = work.take(c, readiness_reserve(state)?)?;
            let connect = work.take(c, CONNECT_BYTES)?;
            let operation = work.take(c, OPERATION_BYTES)?;
            let final_check = work.take(c, VIEW_BYTES)?;
            if state.room_clock_config.is_some() || state.room_clock.is_some() {
                let other = work.take(c, room_clock::VIEW_BYTES)?;
                room_clock::ensure_for_sample(
                    state,
                    &narrowed(work, &other, state.authority_tick)?,
                )?;
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
            let native = native_context(&current, &state.clock_grants)?;
            let mut source = io.connect(&config, &native, true)?;
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
                            "wrong clock commit boundary",
                        ));
                    }
                    let reserved = budget.take().ok_or_else(exhausted)?;
                    guard(state, &spec, current, &reserved, work, audit, io)
                },
            );
            state.clock = Some(owner);
            retain_floor(state);
            let view = outcome?;
            retain_readiness(state, &view);
            let checked = inspect_current(state, work, &final_check)?;
            io.checkpoint(&work.current(c)?, true)?;
            Ok(
                json!({"ok":true,"construction_clock":display(&checked,0,8)?,
                "game_unpause_requested":true,"goal_sample_published":false,
                "readiness_acquisition_retained":true,"readiness_earns_stability_credit":false,
                "next_step":"reconcile_clock_then_acquire_a_separate_whole_plan_construction_sample"}),
            )
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
                let native = native_context(&current, &state.clock_grants)?;
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
                let native = native_context(&current, &state.clock_grants)?;
                Some(io.connect(&config, &native, cancel)?)
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
            Ok(
                json!({"ok":true,"scope":"clock","construction_clock":display(&checked,0,8)?,
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
                json!({"ok":true,"construction_clock":display(&view,offset,limit)?,"native_calls":0,"game_mutations":0}),
            )
        }
    }
}

pub(super) fn run_action<S: EffectJournalStorage, N: BuildSource, I: ClockIo>(
    state: &mut State<S, N>,
    mut c: OperationContext,
    op: &str,
    action: Action,
    started: Instant,
    io: &mut I,
) -> String {
    state.abandon();
    if let Some(other) = &mut state.room_clock {
        other.abandon_preparation();
    }
    state.clock_verified = false;
    let control = matches!(
        &action,
        Action::Prepare(_) | Action::Commit { .. } | Action::Cancel(_)
    );
    let may_have_effect = matches!(&action, Action::Commit { .. } | Action::Cancel(_));
    let failure = |cause: &dfmcp_core::DfmcpError| {
        let mut result = presentation::failure(cause);
        result["scope"] = json!("clock");
        result["effect_may_have_occurred"] = json!(may_have_effect);
        result["goal_sample_published"] = json!(false);
        if cause.code == ErrorCode::EffectIndeterminate {
            result["error"]["message"] = json!(
                "Retain and reconcile this exact construction clock run; do not repeat unpause or publish fresh completion proof while it is unsettled."
            );
        }
        result
    };
    let packet = |state: &State<S, N>, mut result: Value, c: &OperationContext| {
        let current = match state.disclosure_context(c) {
            Ok(current) => current,
            Err(cause) => return super::unbound(op, &cause),
        };
        if let Some(owner) = &state.clock {
            let disclosure = native_context(&current, &state.clock_grants)
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
    let advancement = matches!(&action, Action::Prepare(_) | Action::Commit { .. });
    let room_publication = room_monitor::Publication::reserve(state, &c, &mut work);
    let publication = work.take(&c, VIEW_BYTES);
    let outcome = match (&publication, &room_publication) {
        (_, Err(cause)) => Err(cause.clone()),
        (Ok(_), Ok(_)) => perform(state, &c, &mut work, action, io),
        (Err(cause), _) => Err(cause.clone()),
    };
    retain_floor(state);
    c.anchor.tick = GameTick(c.anchor.tick.get().max(state.authority_tick));
    let failed = outcome.is_err();
    let result = match outcome {
        Ok(result) => result,
        Err(cause) => {
            state.withdraw_monitor();
            state.abandon_all();
            state.clock_verified = false;
            failure(&cause)
        }
    };
    let rendered = packet(state, result.clone(), &c);
    let expected = state.clock.as_ref().map(Clock::recovery);
    let final_check = (|| {
        if rendered.len() as u64 > presentation::OUTPUT_BYTES {
            return Err(exhausted());
        }
        io.checkpoint(&work.current(&c)?, control)?;
        if let Some(reserved) = room_publication.as_ref().map_err(Clone::clone)? {
            let check = reserved.verify(state, expected, false, !advancement, |_| Ok(()));
            if advancement {
                check?;
                if room_clock::blocks_sampling(state) || room_clock::bootstrap_pending(state) {
                    return Err(error(
                        ErrorCode::EffectIndeterminate,
                        "retained room clock blocks furnishing clock publication",
                    ));
                }
            } else if check.is_err()
                && let Some(monitor) = &mut state.room_completion
            {
                monitor.verified = false;
                monitor.store_verified = false;
            }
        }
        if let Some(expected) = expected {
            let checked =
                inspect_current(state, &work, publication.as_ref().map_err(Clone::clone)?)?;
            if checked.recovery != expected {
                return Err(error(
                    ErrorCode::CorruptLedger,
                    "clock publication no longer belongs to verified history",
                ));
            }
        }
        work.current(&c)?;
        Ok(())
    })();
    match final_check {
        Ok(()) => {
            if failed {
                state.clock_verified = false;
            }
            let output = packet(state, result, &c);
            if output.len() as u64 > presentation::OUTPUT_BYTES || work.current(&c).is_err() {
                state.withdraw_monitor();
                state.abandon_all();
                state.clock_verified = false;
                packet(state, failure(&exhausted()), &c)
            } else {
                output
            }
        }
        Err(cause) => {
            state.withdraw_monitor();
            state.abandon_all();
            state.clock_verified = false;
            retain_floor(state);
            c.anchor.tick = GameTick(c.anchor.tick.get().max(state.authority_tick));
            packet(state, failure(&cause), &c)
        }
    }
}

#[cfg(test)]
#[path = "clock/tests.rs"]
mod tests;
