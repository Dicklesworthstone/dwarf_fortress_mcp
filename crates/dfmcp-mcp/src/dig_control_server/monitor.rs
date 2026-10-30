//! Whole-original-blueprint sampled excavation, separate from native effects.
use super::{Action, COMPLETE_INVENTORY_BYTES, Query, State, Work, batch, retain_blueprint};
use super::{error, exhausted, presentation};
use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
use dfmcp_adapter::dig_blueprint::BatchDefinition;
use dfmcp_adapter::dig_blueprint::monitor::store::initial_recovery;
use dfmcp_adapter::dig_blueprint::monitor::store::{MonitorStore, Recovery};
use dfmcp_adapter::dig_blueprint::monitor::{MonitorSpec, Policy, SourcePin};
use dfmcp_adapter::dig_blueprint::monitor::{PREDICATE, Status};
use dfmcp_adapter::dig_designation::journal::session::DigSessionView;
use dfmcp_adapter::dig_designation::journal::{DigMode, DigRecord};
use dfmcp_adapter::dig_designation::rpc::DigSource;
use dfmcp_adapter::live_map::LiveMapObservation;
use dfmcp_core::GameTick;
use dfmcp_core::{Digest32, ErrorCode, OperationContext, Result};
use dfmcp_world::map_region::{Cell, Shape};
use serde::Deserialize;
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Instant;

/// Complete bounded history opening/replay, distinct from native journal work.
pub(super) const OPEN_BYTES: u64 = dfmcp_adapter::dig_blueprint::monitor::store::OPEN_RESERVE;
pub(super) const FINAL_BYTES: u64 = dfmcp_adapter::dig_blueprint::monitor::store::VERIFY_RESERVE;
pub(super) const MAX_ENTITIES: u32 = 2048;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Config {
    pub path: PathBuf,
    pub initialize: bool,
    pub source: SourcePin,
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
    monitor_id: String,
    head: String,
}
impl Config {
    pub fn parse(raw: &str) -> Result<Self> {
        if raw.is_empty() || raw.len() > 8192 {
            return Err(exhausted());
        }
        let input: ConfigInput = serde_json::from_str(raw).map_err(|_| {
            error(
                ErrorCode::InvalidRequest,
                "invalid closed blueprint monitor configuration",
            )
        })?;
        if input.path.len() > 4096
            || !input.path.starts_with('/')
            || input.path.contains('\0')
            || input.path[1..]
                .split('/')
                .any(|p| p.is_empty() || p == "." || p == "..")
            || input.initialize == input.recovery.is_some()
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "monitor initialization, path or recovery differs",
            ));
        }
        let endpoint: SocketAddr = input.source.endpoint.parse().map_err(|_| {
            error(
                ErrorCode::InvalidRequest,
                "monitor endpoint requires numeric loopback",
            )
        })?;
        if endpoint.to_string() != input.source.endpoint {
            return Err(error(
                ErrorCode::InvalidRequest,
                "monitor endpoint is not canonical",
            ));
        }
        let source = SourcePin::new(
            endpoint,
            input.source.generation,
            input.source.df_version,
            input.source.dfhack_version,
        )?;
        let recovery = input
            .recovery
            .map(|input| {
                let monitor_id = presentation::digest(&input.monitor_id)?;
                let head = presentation::digest(&input.head)?;
                if monitor_id == Digest32::ZERO || head == Digest32::ZERO {
                    return Err(error(
                        ErrorCode::InvalidRequest,
                        "reserved monitor recovery identity",
                    ));
                }
                Ok(Recovery { monitor_id, head })
            })
            .transpose()?;
        Ok(Self {
            path: input.path.into(),
            initialize: input.initialize,
            source,
            recovery,
        })
    }
    pub fn matches(&self, spec: &MonitorSpec) -> Result<()> {
        if &self.source != spec.source() {
            return Err(error(
                ErrorCode::StaleAnchor,
                "retained monitor differs from current independent source selection",
            ));
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub blueprint_id: String,
    deadline_tick: u64,
    stable_ticks: Option<u64>,
    required_samples: Option<u32>,
    max_gap_ticks: Option<u64>,
    max_samples: Option<u32>,
}
impl Request {
    pub fn parse(raw: &str) -> Result<Self> {
        if raw.len() > 2048 {
            return Err(exhausted());
        }
        let shape: Value = serde_json::from_str(raw).map_err(|_| {
            error(
                ErrorCode::InvalidRequest,
                "invalid blueprint monitor request",
            )
        })?;
        if shape
            .as_object()
            .is_some_and(|v| v.values().any(Value::is_null))
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "omit optional monitor values instead of null",
            ));
        }
        let input: Self = serde_json::from_str(raw).map_err(|_| {
            error(
                ErrorCode::InvalidRequest,
                "invalid closed blueprint monitor request",
            )
        })?;
        presentation::digest(&input.blueprint_id)?;
        input.policy()?;
        Ok(input)
    }
    pub fn policy(&self) -> Result<Policy> {
        Policy::new(
            self.deadline_tick,
            self.stable_ticks.unwrap_or(10),
            self.required_samples.unwrap_or(2),
            self.max_gap_ticks.unwrap_or(1200),
            self.max_samples.unwrap_or(128),
        )
    }
}

/// Injected foreground ports keep private custody and native I/O out of the
/// synchronous handler. They never provide a game mutation operation.
pub(super) trait MonitorIo<S: EffectJournalStorage> {
    fn create_history(
        &mut self,
        spec: MonitorSpec,
        c: &OperationContext,
    ) -> Result<MonitorStore<S>>;
    fn read_map(&mut self, spec: &MonitorSpec, c: &OperationContext) -> Result<LiveMapObservation>;
    fn checkpoint(&mut self, c: &OperationContext) -> Result<()>;
}
pub(super) struct Unavailable;
impl<S: EffectJournalStorage> MonitorIo<S> for Unavailable {
    fn create_history(&mut self, _: MonitorSpec, _: &OperationContext) -> Result<MonitorStore<S>> {
        Err(error(
            ErrorCode::CapabilityDenied,
            "no monitor custody configured",
        ))
    }
    fn read_map(&mut self, _: &MonitorSpec, _: &OperationContext) -> Result<LiveMapObservation> {
        Err(error(
            ErrorCode::CapabilityDenied,
            "no monitor source configured",
        ))
    }
    fn checkpoint(&mut self, c: &OperationContext) -> Result<()> {
        c.budget.validate()?;
        if c.cancellation_requested {
            return Err(error(
                ErrorCode::CancellationRequested,
                "monitor request cancelled",
            ));
        }
        Ok(())
    }
}

pub(super) fn recovery(value: Recovery) -> Value {
    json!({"monitor_id":value.monitor_id.to_string(),"head":value.head.to_string()})
}

/// Cached identities survive failed runtime or custody checks, but cached
/// terrain is never promoted to a current observation.
pub(super) fn reference<S: EffectJournalStorage>(store: &MonitorStore<S>) -> Value {
    json!({"monitor_id":store.spec().id().to_string(),
        "blueprint_id":store.spec().parent().id().to_string(),
        "journal_id":store.spec().parent().journal_id().to_string(),
        "recovery":recovery(store.recovery()),"history_verified":false,
        "historical_status":store.state().status().as_str(),
        "authority_tick_floor":store.high_tick(),"sampling_fenced":store.is_fenced(),
        "fresh_sample_acquired":false,"sampled_excavation_condition_proven":false,
        "shared_process_identity_proven":false,"mining_causality_proven":false})
}

pub(super) fn display<S: EffectJournalStorage>(
    store: &MonitorStore<S>,
    fresh: bool,
    originals_verified: bool,
    offset: usize,
    limit: usize,
) -> Result<Value> {
    let spec = store.spec();
    let progress = store.state();
    let policy = spec.policy();
    let targets = spec.parent().layout().targets();
    if offset >= targets.len() || !(1..=16).contains(&limit) {
        return Err(exhausted());
    }
    let count = progress.counts()?.map(|n| {
        json!({"matched":n.matched,"mismatched":n.mismatched,
        "hidden":n.hidden,"missing":n.missing,"wrong_shape":n.wrong_shape,
        "wet":n.wet,"designated":n.designated,"deficit_categories_may_overlap":true})
    });
    let rows = if let Some(sample) = progress.latest() {
        targets.iter().skip(offset).take(limit).map(|position| {
            let index = sample.map.region.index(*position).ok_or_else(|| error(ErrorCode::CorruptLedger,"retained monitor target outside capture"))?;
            Ok(match sample.map.cells.get(index).ok_or_else(|| error(ErrorCode::CorruptLedger,"retained monitor cell missing"))? {
                Cell::Unallocated => json!({"coordinate":position,"presence":"missing","matches":false}),
                Cell::Hidden => json!({"coordinate":position,"presence":"hidden","matches":false}),
                Cell::Visible(tile) => json!({"coordinate":position,"presence":"visible","shape":tile.shape.name(),
                    "liquid_depth":tile.liquid_depth,"magma":tile.magma,"dig_designation":tile.dig_designation,
                    "matches":tile.shape==Shape::Floor && tile.liquid_depth==0 && !tile.magma && tile.dig_designation==0}),
            })
        }).collect::<Result<Vec<_>>>()?
    } else {
        vec![]
    };
    // Invalid source captures retain the last accepted target page. That older
    // evidence must remain historical even when this request acquired new bytes.
    let fresh = fresh && progress.status() != Status::Invalidated;
    let satisfied = progress.status() == Status::Satisfied;
    let identity = json!({
        "monitor_id":spec.id().to_string(),"blueprint_id":spec.parent().id().to_string(),
        "journal_id":spec.parent().journal_id().to_string(),"history_head":store.head().to_string(),
        "recovery":recovery(store.recovery()),"history_verified":true,
        "original_designations_verified":originals_verified,"native_head":spec.native_head().to_string(),
        "designation_set_digest":spec.receipt_digest().to_string(),"predicate":PREDICATE,
        "status":progress.status().as_str(),"terminal":progress.status().terminal(),
        "sampling_fenced":store.is_fenced(),"attempts":progress.attempts(),
        "attempt_limit_reached":progress.attempts()>=policy.max_samples(),
        "start_tick":spec.start_tick(),"authority_tick_floor":store.high_tick(),
        "policy":{"deadline_tick":policy.deadline_tick(),"stable_ticks":policy.stable_ticks(),
            "required_samples":policy.required_samples(),"max_gap_ticks":policy.max_gap_ticks(),
            "max_samples":policy.max_samples()}
    });
    let evidence = json!({
        "source":{"protocol":"1.5","generation":spec.source().generation(),
            "df_version":spec.source().df_version(),"dfhack_version":spec.source().dfhack_version(),
            "dig_generation":spec.parent().binding().manifest().generation},
        "capture_region":{"origin":spec.region().origin,"size":spec.region().size},
        "target_count":targets.len(),"counts":count,"matching_samples":progress.streak(),
        "matching_since_tick":progress.since_tick(),
        "latest_sample_tick":progress.latest().map(|s|s.tick().get()),
        "first_satisfied_tick":progress.first_satisfied().map(|s|s.tick().get()),
        "historical_satisfaction_recorded":progress.first_satisfied().is_some(),
        "fresh_sample_acquired":fresh,"sampled_excavation_condition_proven":fresh && satisfied && originals_verified,
        "historical_evidence_only":!fresh,"shared_process_identity_proven":false,"mining_causality_proven":false,
        "continuous_stability_proven":false,"structural_safety_proven":false,"game_ticks_advanced":0,
        "target_page":{"offset":offset,"total":targets.len(),"tiles":rows,
            "next_offset":(offset+limit<targets.len()).then_some(offset+limit),
            "next_query":(offset+limit<targets.len()).then(||json!({"mode":"blueprint_monitor",
                "monitor_id":spec.id().to_string(),"expected_head":store.head().to_string(),
                "offset":offset+limit,"limit":limit}))},
        "query":{"mode":"blueprint_monitor","monitor_id":spec.id().to_string(),
            "expected_head":store.head().to_string(),"offset":0,"limit":16}
    });
    match (identity, evidence) {
        (Value::Object(mut identity), Value::Object(evidence)) => {
            identity.extend(evidence);
            Ok(Value::Object(identity))
        }
        _ => Err(error(
            ErrorCode::InternalInvariantViolation,
            "monitor presentation is not an object",
        )),
    }
}

pub(super) fn audit<
    S: EffectJournalStorage,
    N: DigSource,
    B: EffectJournalStorage,
    C: EffectJournalStorage,
>(
    state: &mut State<S, N, B, C>,
    c: &OperationContext,
) -> Result<(BatchDefinition, DigSessionView, Vec<DigRecord>)> {
    let started = Instant::now();
    let mut current = c.clone();
    current.budget.max_bytes = COMPLETE_INVENTORY_BYTES;
    let (view, records) = state.control.complete_inventory(&current)?;
    current.budget.max_wall_millis = current
        .budget
        .max_wall_millis
        .checked_sub(u64::try_from(started.elapsed().as_millis()).map_err(|_| exhausted())?)
        .filter(|n| *n > 0)
        .ok_or_else(exhausted)?;
    current.budget.max_bytes = batch::PUBLICATION_BYTES;
    let parent = state.blueprint.as_mut().ok_or_else(|| {
        error(
            ErrorCode::Conflict,
            "complete original blueprint is not retained",
        )
    })?;
    batch::verify(parent, &view, &records, &current)?;
    Ok((parent.definition().clone(), view, records))
}
fn exact<
    S: EffectJournalStorage,
    N: DigSource,
    B: EffectJournalStorage,
    C: EffectJournalStorage,
>(
    state: &mut State<S, N, B, C>,
    id: Digest32,
) -> Result<&mut MonitorStore<B>> {
    state
        .monitor
        .as_mut()
        .filter(|m| m.spec().id() == id)
        .ok_or_else(|| {
            error(
                ErrorCode::StaleAnchor,
                "exact original blueprint monitor is not retained",
            )
        })
}
fn narrowed(
    work: &Work,
    c: &OperationContext,
    reserved: &OperationContext,
    tick: u64,
) -> Result<OperationContext> {
    let mut out = reserved.clone();
    out.anchor.tick = GameTick(c.anchor.tick.get().max(tick));
    out.budget.max_wall_millis = work.current(&out)?.budget.max_wall_millis;
    Ok(out)
}

fn perform<S, N, B, C, M>(
    state: &mut State<S, N, B, C>,
    c: &OperationContext,
    work: &mut Work,
    action: Action,
    io: &mut M,
) -> Result<Value>
where
    S: EffectJournalStorage,
    N: DigSource,
    B: EffectJournalStorage,
    C: EffectJournalStorage,
    M: MonitorIo<B>,
{
    match action {
        Action::StartMonitor(request) => {
            let id = presentation::digest(&request.blueprint_id)?;
            let config = state.monitor_config.clone().ok_or_else(|| {
                error(
                    ErrorCode::CapabilityDenied,
                    "no original blueprint monitor configured",
                )
            })?;
            let initial = work.take(c, COMPLETE_INVENTORY_BYTES + batch::PUBLICATION_BYTES)?;
            let final_check = work.take(c, COMPLETE_INVENTORY_BYTES + batch::PUBLICATION_BYTES)?;
            let opening = work.take(c, OPEN_BYTES)?;
            let final_history = work.take(c, FINAL_BYTES)?;
            let (parent, view, records) = audit(state, &initial)?;
            if parent.id() != id {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "monitor request names another original blueprint",
                ));
            }
            let policy = request.policy()?;
            io.checkpoint(&work.current(c)?)?;
            if let Some(store) = state.monitor.as_mut() {
                if store.spec().parent() != &parent || store.spec().policy() != &policy {
                    return Err(error(
                        ErrorCode::Conflict,
                        "original monitor policy cannot be replaced or re-deadlined",
                    ));
                }
                config.matches(store.spec())?;
                store.verify(&narrowed(work, c, &opening, store.high_tick())?)?;
                store.spec().verify_original(&parent, &view, &records)?;
            } else {
                if !config.initialize
                    || state.monitor_recovery.is_some()
                    || state.control.mode() == DigMode::Offline
                {
                    return Err(error(
                        ErrorCode::CapabilityDenied,
                        "new monitor requires writable exact configured empty custody",
                    ));
                }
                let spec = MonitorSpec::new(
                    parent,
                    &view,
                    &records,
                    config.source,
                    policy,
                    c.anchor
                        .tick
                        .get()
                        .max(state.authority_tick)
                        .max(state.control.high_tick()),
                )?;
                spec.authorize(&work.current(c)?, true)?;
                state.monitor_recovery = Some(initial_recovery(&spec)?);
                let store =
                    io.create_history(spec, &narrowed(work, c, &opening, state.authority_tick)?)?;
                state.monitor_recovery = Some(store.recovery());
                state.monitor = Some(store);
            }
            let authority_tick = state.authority_tick;
            let (parent, view, records) =
                audit(state, &narrowed(work, c, &final_check, authority_tick)?)?;
            let store = state
                .monitor
                .as_mut()
                .ok_or_else(|| error(ErrorCode::CorruptLedger, "monitor publication missing"))?;
            store.spec().verify_original(&parent, &view, &records)?;
            store.verify(&narrowed(work, c, &final_history, store.high_tick())?)?;
            io.checkpoint(&work.current(c)?)?;
            Ok(
                json!({"ok":true,"blueprint_monitor":display(store,false,true,0,16)?,"native_calls":0,"game_mutations":0}),
            )
        }
        Action::SampleMonitor(id) => {
            let final_clock = if state.clock_config.is_some() {
                Some(work.take(c, super::clock::VIEW_BYTES)?)
            } else {
                None
            };
            if state.clock_config.is_some() {
                let reserved = work.take(c, super::clock::VIEW_BYTES)?;
                super::clock::inspect(state, &reserved)?;
                if super::clock::blocks_sampling(state) {
                    return Err(error(
                        ErrorCode::EffectIndeterminate,
                        "reconcile the original clock run before sampling excavation",
                    ));
                }
            }
            let initial = work.take(c, COMPLETE_INVENTORY_BYTES + batch::PUBLICATION_BYTES)?;
            let final_check = work.take(c, COMPLETE_INVENTORY_BYTES + batch::PUBLICATION_BYTES)?;
            let final_history = work.take(c, FINAL_BYTES)?;
            let initial_history = work.take(c, FINAL_BYTES)?;
            let read = work.take(c, dfmcp_adapter::dig_blueprint::monitor::io::READ_BYTES)?;
            let spec = exact(state, id)?.spec().clone();
            let operation = work.take(c, exact(state, id)?.operation_reserve())?;
            let (parent, view, records) = audit(state, &initial)?;
            spec.verify_original(&parent, &view, &records)?;
            let config = state.monitor_config.as_ref().ok_or_else(|| {
                error(
                    ErrorCode::CapabilityDenied,
                    "monitor source configuration absent",
                )
            })?;
            config.matches(&spec)?;
            let authority_tick = state.authority_tick;
            exact(state, id)?.verify(&narrowed(work, c, &initial_history, authority_tick)?)?;
            if exact(state, id)?.state().status().terminal() {
                io.checkpoint(&work.current(c)?)?;
                return Ok(
                    json!({"ok":true,"blueprint_monitor":display(exact(state,id)?,false,true,0,16)?,
                    "native_calls":0,"game_mutations":0,"replayed_locally":true}),
                );
            }
            if state.control.mode() == DigMode::Offline {
                return Err(error(
                    ErrorCode::CapabilityDenied,
                    "offline monitor inspection cannot sample",
                ));
            }
            io.checkpoint(&work.current(c)?)?;
            spec.authorize(&work.current(c)?, true)?;
            let attempt =
                exact(state, id)?.begin_attempt(&narrowed(work, c, &operation, authority_tick)?)?;
            let outcome: Result<()> = (|| {
                let raw = io.read_map(&spec, &narrowed(work, c, &read, state.authority_tick)?)?;
                // Persist the entire raw capture before newly learned clock,
                // source or runtime rejection can erase negative evidence.
                exact(state, id)?.capture(&attempt, raw, &operation)?;
                state.authority_tick = state.authority_tick.max(exact(state, id)?.high_tick());
                let current = narrowed(work, c, &operation, state.authority_tick)?;
                io.checkpoint(&current)?;
                spec.authorize(&current, true)?;
                let authority_tick = state.authority_tick;
                let (parent, view, records) =
                    audit(state, &narrowed(work, c, &final_check, authority_tick)?)?;
                spec.verify_original(&parent, &view, &records)?;
                if let Some(reserved) = &final_clock {
                    super::clock::inspect(
                        state,
                        &narrowed(work, c, reserved, state.authority_tick)?,
                    )?;
                    if super::clock::blocks_sampling(state) {
                        return Err(error(
                            ErrorCode::EffectIndeterminate,
                            "clock custody changed before excavation publication",
                        ));
                    }
                }
                let authority_tick = state.authority_tick;
                exact(state, id)?
                    .publish(&attempt, &narrowed(work, c, &operation, authority_tick)?)?;
                exact(state, id)?.verify(&narrowed(work, c, &final_history, authority_tick)?)?;
                io.checkpoint(&work.current(&current)?)?;
                Ok(())
            })();
            state.authority_tick = state.authority_tick.max(exact(state, id)?.high_tick());
            if let Err(cause) = outcome {
                // Both paths retain consumed attempts and the highest observed
                // tick. If authority/time is gone the store fences unfinished
                // custody, and reopening cannot resume its previous streak.
                if let Ok(current) = narrowed(work, c, &operation, state.authority_tick) {
                    let store = exact(state, id)?;
                    let _ = if cause.code == ErrorCode::StaleAnchor {
                        store.invalidate(&attempt, &current)
                    } else {
                        store.interrupt(&attempt, &current)
                    };
                }
                return Err(cause);
            }
            Ok(
                json!({"ok":true,"blueprint_monitor":display(exact(state,id)?,true,true,0,16)?,
                "native_calls":1,"game_mutations":0}),
            )
        }
        Action::CancelMonitor(id) => {
            let bytes = exact(state, id)?.operation_reserve();
            let operation = work.take(c, bytes)?;
            let final_history = work.take(c, FINAL_BYTES)?;
            io.checkpoint(&work.current(c)?)?;
            exact(state, id)?.cancel(&operation)?;
            let authority_tick = state.authority_tick;
            exact(state, id)?.verify(&narrowed(work, c, &final_history, authority_tick)?)?;
            io.checkpoint(&work.current(c)?)?;
            Ok(
                json!({"ok":true,"scope":"monitor","blueprint_monitor":display(exact(state,id)?,false,false,0,16)?,
                "native_calls":0,"game_mutations":0,"effects_cancelled":false}),
            )
        }
        Action::Query(Query::BlueprintMonitor {
            monitor_id,
            expected_head,
            offset,
            limit,
        }) => {
            let id = presentation::digest(&monitor_id)?;
            let check = work.take(c, FINAL_BYTES)?;
            io.checkpoint(&work.current(c)?)?;
            let store = exact(state, id)?;
            store.verify(&check)?;
            if expected_head
                .as_deref()
                .map(presentation::digest)
                .transpose()?
                .is_some_and(|head| head != store.head())
            {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "monitor page belongs to an older history head",
                ));
            }
            io.checkpoint(&work.current(c)?)?;
            Ok(
                json!({"ok":true,"blueprint_monitor":display(store,false,false,offset.unwrap_or(0),limit.unwrap_or(16))?,
                "native_calls":0,"game_mutations":0}),
            )
        }
        _ => Err(error(
            ErrorCode::InternalInvariantViolation,
            "wrong blueprint monitor action",
        )),
    }
}

pub(super) fn run_action<S, N, B, C, M>(
    state: &mut State<S, N, B, C>,
    mut c: OperationContext,
    op: &str,
    action: Action,
    started: Instant,
    io: &mut M,
) -> String
where
    S: EffectJournalStorage,
    N: DigSource,
    B: EffectJournalStorage,
    C: EffectJournalStorage,
    M: MonitorIo<B>,
{
    state.abandon();
    let mut work = match Work::new(&c, started) {
        Ok(work) => work,
        Err(cause) => {
            state.withdraw_monitor();
            return presentation::packet(
                op,
                retain_blueprint(state, presentation::failure(&cause)),
                Some(&c),
                Some(&state.binding),
                None,
                None,
                None,
            );
        }
    };
    let outcome = perform(state, &c, &mut work, action, io);
    if outcome.is_err() {
        state.withdraw_monitor();
    }
    if let Some(store) = &state.monitor {
        state.authority_tick = state.authority_tick.max(store.high_tick());
        state.monitor_recovery = Some(store.recovery());
    }
    c.anchor.tick = c.anchor.tick.max(GameTick(state.authority_tick));
    let result = retain_blueprint(
        state,
        match outcome {
            Ok(value) => value,
            Err(cause) => presentation::failure(&cause),
        },
    );
    let rendered = presentation::packet(
        op,
        result,
        Some(&c),
        Some(&state.binding),
        None,
        Some(&state.policy),
        None,
    );
    if rendered.len() as u64 > presentation::OUTPUT_BYTES || work.current(&c).is_err() {
        state.withdraw_monitor();
        presentation::packet(
            op,
            retain_blueprint(state, presentation::failure(&exhausted())),
            Some(&c),
            Some(&state.binding),
            None,
            None,
            None,
        )
    } else {
        rendered
    }
}
