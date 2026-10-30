#![forbid(unsafe_code)]
//! Explicitly unadmitted mining control. All native effects use DigSession and
//! its durable coordinator; recovery-only MCP remains a separate sealed profile.
use dfmcp_adapter::bounded_run::private_file::PrivateRunFile;
use dfmcp_adapter::build_placement::journal::private_file::PrivateBuildFile;
use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
use dfmcp_adapter::dig_blueprint::clock::{Clock as BlueprintClock, Recovery as ClockRecovery};
use dfmcp_adapter::dig_blueprint::store::{
    BatchStore, Recovery as BlueprintRecovery, initial_recovery, open_private_blueprint_storage,
};
use dfmcp_adapter::dig_blueprint::{BatchDefinition, Layout};
use dfmcp_adapter::dig_control_policy::{DigControlPolicy, DigReview, PolicyDigGuard};
use dfmcp_adapter::dig_designation::journal::private_file::{
    PrivateDigFile, open_private_dig, open_private_dig_exact,
};
use dfmcp_adapter::dig_designation::journal::session::{
    COMPLETE_INVENTORY_BYTES, DigSession, DigSessionGuard, DigSessionView, SOURCE_RESERVATION_BYTES,
};
use dfmcp_adapter::dig_designation::journal::{
    DigBinding, DigCursor, DigMode, DigRecord, DigState, MAX_FRAME_BYTES, MAX_JOURNAL_BYTES,
};
use dfmcp_adapter::dig_designation::rpc::{DigRpcClient, DigSource, DigTcpStream, RPC_BYTES};
use dfmcp_adapter::dig_designation::{DigObservation, DigPlan, DigRegion, MAX_NATIVE_TICK};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, Digest32, ErrorCode, GameTick, LeaseManager,
    ObservationCursor, OperationContext, RequestId, Result, RiskTier, SessionId, StateAnchor,
    WorkBudget,
};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::{
    Arc, Mutex, MutexGuard, TryLockError,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

mod batch;
mod clock;
mod clock_recovery;
mod monitor;
mod monitor_recovery;
mod presentation;
mod runtime;
use presentation::{OUTPUT_BYTES, digest, failure, inventory, packet};
use runtime::{Config, RequestControl};
const MAX_WORK_BYTES: u64 = 1024 * 1024 * 1024;
const VIEW_BYTES: u64 = MAX_JOURNAL_BYTES as u64 + 4096;
const LOCAL_BYTES: u64 = VIEW_BYTES + 32 * 1024;
const OPEN_BYTES: u64 = 4 * VIEW_BYTES;
const FAMILY: u128 = 18u128 << 57;
static NEXT: AtomicU64 = AtomicU64::new(1);
type Native = DigRpcClient<DigTcpStream>;
enum Entry {
    Native {
        state: State<PrivateDigFile, Native, PrivateBuildFile, PrivateRunFile>,
        config: Config,
    },
    Monitor {
        state: monitor_recovery::RecoveryState<PrivateBuildFile>,
        config: Config,
    },
    ClockRecovery {
        state: clock_recovery::RecoveryState<PrivateBuildFile, PrivateRunFile>,
        config: Config,
    },
}
impl Entry {
    fn id(&self) -> SessionId {
        match self {
            Self::Native { state, .. } => state.id,
            Self::Monitor { state, .. } => state.id(),
            Self::ClockRecovery { state, .. } => state.id(),
        }
    }
    fn config(&self) -> &Config {
        match self {
            Self::Native { config, .. }
            | Self::Monitor { config, .. }
            | Self::ClockRecovery { config, .. } => config,
        }
    }
    fn blueprint_reference(&self) -> Option<Value> {
        match self {
            Self::Native { state, .. } => state.blueprint_reference(),
            Self::Monitor { .. } | Self::ClockRecovery { .. } => None,
        }
    }
    fn monitor_reference(&self) -> Option<Value> {
        match self {
            Self::Native { state, .. } => state.monitor_reference(),
            Self::Monitor { state, .. } => Some(state.reference()),
            Self::ClockRecovery { state, .. } => Some(state.monitor_reference()),
        }
    }
    fn clock_reference(&self) -> Option<Value> {
        match self {
            Self::Native { state, .. } => state.clock_reference(),
            Self::Monitor { .. } => None,
            Self::ClockRecovery { state, .. } => Some(state.clock_reference()),
        }
    }
    fn abandon(&mut self) {
        if let Self::Native { state, .. } = self {
            state.abandon();
            if let Some(clock) = state.clock.as_mut() {
                clock.abandon_preparation();
            }
        }
    }
}
static SESSION: Mutex<Option<Entry>> = Mutex::new(None);
fn error(code: ErrorCode, text: &str) -> dfmcp_core::DfmcpError {
    dfmcp_core::DfmcpError::new(code, text)
}
fn exhausted() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "complete mining work and response exceed the allowance",
    )
}
fn unbound(op: &str, e: &dfmcp_core::DfmcpError) -> String {
    let mut result = failure(e);
    result["effect_may_have_occurred"] = json!(matches!(op, "fortress.commit" | "fortress.cancel"));
    packet(op, result, None, None, None, None, None)
}
fn key(raw: &str) -> Result<()> {
    if raw.is_empty()
        || raw.len() > 128
        || !raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return Err(error(ErrorCode::InvalidRequest, "invalid mining key"));
    }
    Ok(())
}
#[derive(Clone, Default)]
struct Cursors {
    serial: u64,
    entries: VecDeque<(String, DigCursor)>,
}
impl Cursors {
    fn issue(&mut self, cursor: DigCursor, c: &OperationContext, head: Digest32) -> Result<String> {
        self.serial = self.serial.checked_add(1).ok_or_else(exhausted)?;
        let mut raw = b"dfmcp-dig-control-cursor/1\0".to_vec();
        raw.extend_from_slice(&c.session_id.get().to_be_bytes());
        raw.extend_from_slice(head.as_bytes());
        raw.extend_from_slice(&self.serial.to_be_bytes());
        let token = Digest32::of_bytes(&raw).to_string();
        if self.entries.len() == 64 {
            self.entries.pop_front();
        }
        self.entries.push_back((token.clone(), cursor));
        Ok(token)
    }
    fn resolve(&self, token: &str) -> Result<DigCursor> {
        digest(token)?;
        self.entries
            .iter()
            .find(|(t, _)| t == token)
            .map(|(_, c)| c.clone())
            .ok_or_else(|| error(ErrorCode::StaleAnchor, "mining continuation expired"))
    }
}
struct Review {
    key: String,
    plan: Digest32,
    witness: Digest32,
    hidden: bool,
    value: DigReview,
    blueprint_seal: Option<Digest32>,
}
struct State<S: EffectJournalStorage, N, B: EffectJournalStorage = S, C: EffectJournalStorage = B> {
    id: SessionId,
    request: u128,
    budget: WorkBudget,
    grants: Vec<CapabilityGrant>,
    authority_tick: u64,
    control: DigSession<S, N>,
    binding: DigBinding,
    policy: DigControlPolicy,
    leases: LeaseManager,
    review: Option<Review>,
    cursors: Cursors,
    blueprint: Option<BatchStore<B>>,
    blueprint_storage: Option<B>,
    blueprint_recovery: Option<BlueprintRecovery>,
    blueprint_required: bool,
    monitor: Option<dfmcp_adapter::dig_blueprint::monitor::store::MonitorStore<B>>,
    monitor_recovery: Option<dfmcp_adapter::dig_blueprint::monitor::store::Recovery>,
    monitor_config: Option<monitor::Config>,
    worker_acknowledgement: Option<Arc<AtomicBool>>,
    clock: Option<BlueprintClock<C>>,
    clock_config: Option<clock::Config>,
    clock_recovery: Option<ClockRecovery>,
    clock_grants: Vec<CapabilityGrant>,
}
impl<S: EffectJournalStorage, N: DigSource, B: EffectJournalStorage, C: EffectJournalStorage>
    State<S, N, B, C>
{
    fn new(mut control: DigSession<S, N>, c: &OperationContext, config: &Config) -> Result<Self> {
        if control.mode() != DigMode::Control && config.blueprint.is_none() {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "control requires an explicitly opened Control journal",
            ));
        }
        let view = control.view(c)?;
        let binding = control.binding().clone();
        config.matches(&binding)?;
        let authority_tick = c.anchor.tick.get().max(control.high_tick());
        let mut leases = LeaseManager::new();
        let lease = leases.acquire_spatial_lease(
            c.session_id,
            binding.scope(),
            true,
            GameTick(authority_tick),
            1200,
        )?;
        let policy = DigControlPolicy::new(
            binding.clone(),
            view.journal_id,
            c.session_id,
            lease,
            config.protected.clone(),
            config.checkpoint,
        )?;
        Ok(Self {
            id: c.session_id,
            request: c.request_id.get(),
            budget: c.budget,
            grants: c.grants.clone(),
            authority_tick,
            control,
            binding,
            policy,
            leases,
            review: None,
            cursors: Cursors::default(),
            blueprint: None,
            blueprint_storage: None,
            blueprint_recovery: None,
            blueprint_required: config.blueprint.is_some(),
            monitor: None,
            monitor_recovery: None,
            monitor_config: config.monitor.clone(),
            worker_acknowledgement: None,
            clock: None,
            clock_config: config.clock.clone(),
            clock_recovery: None,
            clock_grants: Vec::new(),
        })
    }
    fn context(&mut self, write_enabled: bool, wall: Option<u64>) -> Result<OperationContext> {
        self.context_with_clock(write_enabled, false, wall)
    }
    fn context_with_clock(
        &mut self,
        write_enabled: bool,
        clock_enabled: bool,
        wall: Option<u64>,
    ) -> Result<OperationContext> {
        self.withdraw_unacknowledged();
        self.request = self.request.checked_add(1).ok_or_else(exhausted)?;
        let mut budget = self.budget;
        if let Some(wall) = wall {
            budget.max_wall_millis = budget.max_wall_millis.min(wall);
        }
        let mut c = context(
            self.id,
            RequestId::new(self.request),
            &self.binding,
            self.authority_tick
                .max(self.control.high_tick())
                .max(self.clock.as_ref().map_or(0, BlueprintClock::high_tick)),
            budget,
            false,
        );
        c.grants = self
            .grants
            .iter()
            .filter(|g| {
                let control = self.control.mode() == DigMode::Control;
                match g.capability {
                    Capability::Plan => control && (write_enabled || clock_enabled),
                    Capability::Designate => control && write_enabled,
                    Capability::ControlClock => control && clock_enabled,
                    _ => true,
                }
            })
            .cloned()
            .collect();
        Ok(c)
    }
    fn abandon(&mut self) {
        self.review = None;
        self.control.abandon_preparation();
    }
    fn withdraw_monitor(&mut self) {
        if let Some(store) = self.monitor.as_mut() {
            self.authority_tick = self.authority_tick.max(store.high_tick());
            self.monitor_recovery = Some(store.recovery());
            if store.withdraw_unpublished().is_err() {
                // Preserve exact recovery pins, but never keep a sampler whose
                // unfinished continuity could not be conservatively withdrawn.
                self.monitor = None;
            }
        }
    }
    fn withdraw_unacknowledged(&mut self) {
        if self
            .worker_acknowledgement
            .take()
            .is_some_and(|ack| !ack.load(Ordering::Acquire))
        {
            self.abandon();
            if let Some(clock) = self.clock.as_mut() {
                clock.abandon_preparation();
            }
            self.withdraw_monitor();
        }
    }
    fn seal(&self) -> Option<Digest32> {
        if self.control.has_preparation_connection() {
            self.review
                .as_ref()
                .map(|r| r.blueprint_seal.map_or_else(|| r.value.seal(), |s| s))
        } else {
            None
        }
    }
    fn blueprint_reference(&self) -> Option<Value> {
        self.blueprint
            .as_ref()
            .map(batch::reference)
            .or_else(|| {
                self.blueprint_recovery.map(|r| {
                    json!({"recovery":batch::recovery(r),"inventory_verified":false,
                "journal_id":self.policy.journal_id().to_string(),
                "publication_unverified":true,"excavation_completion_proven":false,
                "retry_commit_permitted":false})
                })
            })
            .or_else(|| {
                self.blueprint_required.then(|| {
                    json!({"journal_id":self.policy.journal_id().to_string(),
            "creation_pending":self.blueprint_storage.is_some(),"inventory_verified":false,
            "excavation_completion_proven":false,"retry_commit_permitted":false})
                })
            })
    }
    fn monitor_reference(&self) -> Option<Value> {
        self.monitor.as_ref().map(monitor::reference).or_else(|| {
            self.monitor_recovery.map(|r| {
                json!({"monitor_id":r.monitor_id.to_string(),
                "recovery":monitor::recovery(r),"history_verified":false,
                "publication_unverified":true,"fresh_sample_acquired":false,
                "sampled_excavation_condition_proven":false,
                "shared_process_identity_proven":false,"mining_causality_proven":false})
            })
        })
    }
    fn clock_reference(&self) -> Option<Value> {
        let floor = self.authority_tick.max(self.control.high_tick()).max(
            self.monitor
                .as_ref()
                .map_or(0, |monitor| monitor.high_tick()),
        );
        self.clock
            .as_ref()
            .map(|owner| {
                let mut reference = clock::reference(owner);
                reference["authority_tick_floor"] = json!(floor.max(owner.high_tick()));
                reference
            })
            .or_else(|| {
                self.clock_recovery.map(|r| {
                    json!({"recovery":clock::recovery(r),"history_verified":false,
                "publication_unverified":true,"current_pause_proven":false,"authority_tick_floor":floor,
                "sampled_excavation_condition_proven":false,"retry_commit_permitted":false})
                })
            })
            .or_else(|| {
                self.clock_config.as_ref().map(|config| {
                    json!({"creation_pending":config.initialize,"history_verified":false,"authority_tick_floor":floor,
                "current_pause_proven":false,"sampled_excavation_condition_proven":false,
                "retry_commit_permitted":false})
                })
            })
    }
}
fn grants(
    fortress: dfmcp_core::FortressId,
    scope: dfmcp_core::MapCuboid,
    write: bool,
) -> Vec<CapabilityGrant> {
    [
        Capability::Query,
        Capability::Observe,
        Capability::Plan,
        Capability::Designate,
    ]
    .into_iter()
    .filter(|v| write || matches!(v, Capability::Query | Capability::Observe))
    .map(|capability| CapabilityGrant {
        capability,
        scope: CapabilityScope {
            fortress_id: Some(fortress),
            map_area: Some(scope),
            ..CapabilityScope::default()
        },
        max_risk: if matches!(capability, Capability::Plan | Capability::Designate) {
            RiskTier::Guarded
        } else {
            RiskTier::ReadOnly
        },
        expires_at_tick: None,
        remaining_uses: None,
    })
    .collect()
}
fn add_clock_grants(c: &mut OperationContext, config: &Config, mode: DigMode, enabled: bool) {
    if !enabled || config.clock.is_none() || mode != DigMode::Control {
        return;
    }
    if !c.grants.iter().any(|g| g.capability == Capability::Plan) {
        c.grants.push(CapabilityGrant {
            capability: Capability::Plan,
            scope: CapabilityScope {
                fortress_id: Some(config.fortress()),
                map_area: Some(config.scope),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::Guarded,
            expires_at_tick: None,
            remaining_uses: None,
        });
    }
    c.grants.push(CapabilityGrant {
        capability: Capability::ControlClock,
        scope: CapabilityScope {
            fortress_id: Some(config.fortress()),
            ..CapabilityScope::default()
        },
        max_risk: RiskTier::Guarded,
        expires_at_tick: None,
        remaining_uses: None,
    });
}
fn context(
    id: SessionId,
    request_id: RequestId,
    binding: &DigBinding,
    tick: u64,
    budget: WorkBudget,
    write: bool,
) -> OperationContext {
    OperationContext {
        session_id: id,
        request_id,
        budget,
        cancellation_requested: false,
        anchor: StateAnchor {
            fortress_id: binding.fortress_id(),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(tick),
            state_hash: Digest32::ZERO,
        },
        grants: grants(binding.fortress_id(), binding.scope(), write),
    }
}
struct Work {
    deadline: Instant,
    bytes: u64,
}
impl Work {
    fn new(c: &OperationContext, started: Instant) -> Result<Self> {
        c.budget.validate()?;
        if c.budget.max_wall_millis > 60_000
            || c.budget.max_bytes > MAX_WORK_BYTES
            || c.budget.max_output_tokens > 65_536
            || u64::from(c.budget.max_output_tokens) * 4 < OUTPUT_BYTES
        {
            return Err(exhausted());
        }
        let bytes = c
            .budget
            .max_bytes
            .checked_sub(OUTPUT_BYTES + 2 * VIEW_BYTES)
            .ok_or_else(exhausted)?;
        let deadline = started
            .checked_add(Duration::from_millis(c.budget.max_wall_millis))
            .ok_or_else(exhausted)?;
        let out = Self { deadline, bytes };
        out.current(c)?;
        Ok(out)
    }
    fn current(&self, c: &OperationContext) -> Result<OperationContext> {
        let left = self
            .deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(exhausted)?
            .as_millis();
        if left == 0 {
            return Err(exhausted());
        }
        let mut out = c.clone();
        out.budget.max_wall_millis = u64::try_from(left).map_err(|_| exhausted())?;
        out.budget.max_bytes = self.bytes;
        Ok(out)
    }
    fn take(&mut self, c: &OperationContext, bytes: u64) -> Result<OperationContext> {
        let mut out = self.current(c)?;
        self.bytes = self.bytes.checked_sub(bytes).ok_or_else(exhausted)?;
        out.budget.max_bytes = bytes;
        Ok(out)
    }
    fn view(&self, c: &OperationContext) -> Result<OperationContext> {
        let mut out = self.current(c)?;
        out.budget.max_bytes = VIEW_BYTES;
        Ok(out)
    }
    fn operation(&mut self, c: &OperationContext, v: &DigSessionView) -> Result<OperationContext> {
        self.take(
            c,
            44 * (v.byte_len + 2 * MAX_FRAME_BYTES) as u64
                + SOURCE_RESERVATION_BYTES
                + 6 * RPC_BYTES
                + 64 * 1024,
        )
    }
}
#[derive(Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
enum Query {
    BlueprintClock {
        monitor_id: String,
        expected_head: Option<String>,
        offset: Option<usize>,
        limit: Option<usize>,
    },
    BlueprintMonitor {
        monitor_id: String,
        expected_head: Option<String>,
        offset: Option<usize>,
        limit: Option<usize>,
    },
    Blueprint {
        blueprint_id: String,
        expected_head: Option<String>,
        expected_parent_head: Option<String>,
        offset: Option<usize>,
        limit: Option<usize>,
    },
    BlueprintLayout {
        blueprint_id: String,
    },
    Records {
        limit: Option<usize>,
        continuation: Option<String>,
    },
    SelectionTiles {
        witness: String,
        offset: usize,
        limit: Option<usize>,
    },
    PlanTiles {
        idempotency_key: String,
        plan_digest: String,
        offset: usize,
        limit: Option<usize>,
    },
    Schema {},
}
impl Query {
    fn parse(raw: &str) -> Result<Self> {
        if raw.len() > 2048 {
            return Err(error(
                ErrorCode::InvalidRequest,
                "mining query exceeds 2 KiB",
            ));
        }
        let shape: Value = serde_json::from_str(raw)
            .map_err(|_| error(ErrorCode::InvalidRequest, "invalid closed mining query"))?;
        if shape
            .as_object()
            .is_some_and(|m| m.values().any(Value::is_null))
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "omit optional query fields instead of supplying null",
            ));
        }
        // Parse the original again: converting from Value would hide duplicate fields.
        let value: Self = serde_json::from_str(raw)
            .map_err(|_| error(ErrorCode::InvalidRequest, "invalid closed mining query"))?;
        match &value {
            Self::BlueprintClock {
                monitor_id,
                expected_head,
                offset,
                limit,
            } => {
                digest(monitor_id)?;
                if let Some(head) = expected_head {
                    digest(head)?;
                }
                if offset.map_or(0, |n| n) >= 64 || !(1..=8).contains(&limit.map_or(8, |n| n)) {
                    return Err(exhausted());
                }
            }
            Self::BlueprintMonitor {
                monitor_id,
                expected_head,
                offset,
                limit,
            } => {
                digest(monitor_id)?;
                if let Some(head) = expected_head {
                    digest(head)?;
                }
                if offset.map_or(0, |n| n) >= 512 || !(1..=16).contains(&limit.map_or(16, |n| n)) {
                    return Err(exhausted());
                }
            }
            Self::Blueprint {
                blueprint_id,
                expected_head,
                expected_parent_head,
                offset,
                limit,
            } => {
                digest(blueprint_id)?;
                if let Some(head) = expected_head {
                    digest(head)?;
                }
                if let Some(head) = expected_parent_head {
                    digest(head)?;
                }
                if offset.map_or(0, |n| n) >= 128 || !(1..=16).contains(&limit.map_or(8, |n| n)) {
                    return Err(exhausted());
                }
            }
            Self::BlueprintLayout { blueprint_id } => {
                digest(blueprint_id)?;
            }
            Self::Records {
                limit,
                continuation,
            } => {
                if !(1..=8).contains(&limit.map_or(8, |n| n)) {
                    return Err(exhausted());
                }
                if let Some(v) = continuation {
                    digest(v)?;
                }
            }
            Self::SelectionTiles {
                witness,
                offset,
                limit,
            } => {
                digest(witness)?;
                tile_bounds(*offset, *limit)?;
            }
            Self::PlanTiles {
                idempotency_key,
                plan_digest,
                offset,
                limit,
            } => {
                key(idempotency_key)?;
                digest(plan_digest)?;
                tile_bounds(*offset, *limit)?;
            }
            Self::Schema {} => {}
        }
        Ok(value)
    }
}
fn tile_bounds(offset: usize, limit: Option<usize>) -> Result<()> {
    if offset >= 300 || !(1..=16).contains(&limit.map_or(16, |n| n)) {
        return Err(exhausted());
    }
    Ok(())
}
enum Action {
    PrepareClock(clock::Request),
    CommitClock {
        monitor_id: Digest32,
        seal: Digest32,
    },
    WaitClock(Digest32),
    CancelClock(Digest32),
    StartMonitor(monitor::Request),
    SampleMonitor(Digest32),
    CancelMonitor(Digest32),
    Blueprint(Layout),
    StopBlueprint(Digest32),
    Observe(DigRegion),
    Plan {
        key: String,
        witness: Digest32,
        hidden: bool,
    },
    Commit {
        key: String,
        plan: Digest32,
        seal: Digest32,
    },
    Wait(String, Digest32),
    Cancel(String, Digest32),
    Explain(String, Digest32),
    Query(Query),
    Inventory,
    Denied,
}
#[allow(clippy::too_many_arguments)]
fn perform<S, N, B, C, G, F>(
    state: &mut State<S, N, B, C>,
    c: &OperationContext,
    work: &mut Work,
    before: &DigSessionView,
    cursors: &mut Cursors,
    action: Action,
    runtime: &mut G,
    connect: F,
) -> Result<Value>
where
    S: EffectJournalStorage,
    N: DigSource,
    B: EffectJournalStorage,
    C: EffectJournalStorage,
    G: DigSessionGuard,
    F: FnOnce(&DigBinding, DigRegion, &OperationContext) -> Result<N>,
{
    let advancing = matches!(
        &action,
        Action::Observe(_) | Action::Plan { .. } | Action::Commit { .. }
    );
    if advancing && state.blueprint_required && state.blueprint.is_none() {
        return Err(error(
            ErrorCode::Conflict,
            "publish the configured original blueprint before any child action",
        ));
    }
    let original_inventory = if advancing && state.blueprint.is_some() {
        let inventory = state
            .control
            .complete_inventory(&work.take(c, COMPLETE_INVENTORY_BYTES)?)?;
        let parent = state
            .blueprint
            .as_mut()
            .ok_or_else(|| error(ErrorCode::CorruptLedger, "blueprint custody absent"))?;
        batch::verify(
            parent,
            &inventory.0,
            &inventory.1,
            &work.take(c, batch::PUBLICATION_BYTES)?,
        )?;
        // Pay for every additional parent check before contacting the native source.
        work.take(c, batch::GUARD_BYTES)?;
        Some(inventory)
    } else {
        None
    };
    match action {
        Action::PrepareClock(_)
        | Action::CommitClock { .. }
        | Action::WaitClock(_)
        | Action::CancelClock(_)
        | Action::Query(Query::BlueprintClock { .. })
        | Action::StartMonitor(_)
        | Action::SampleMonitor(_)
        | Action::CancelMonitor(_)
        | Action::Query(Query::BlueprintMonitor { .. }) => Err(error(
            ErrorCode::InternalInvariantViolation,
            "monitor action bypassed its read-only owner",
        )),
        Action::Blueprint(layout) => {
            if let Some(parent) = state.blueprint.as_mut() {
                if parent.definition().layout() != &layout {
                    return Err(error(
                        ErrorCode::Conflict,
                        "the original blueprint cannot be replaced",
                    ));
                }
                let (view, records) = state
                    .control
                    .complete_inventory(&work.take(c, COMPLETE_INVENTORY_BYTES)?)?;
                let progress = batch::verify(
                    parent,
                    &view,
                    &records,
                    &work.take(c, batch::PUBLICATION_BYTES)?,
                )?;
                return Ok(
                    json!({"ok":true,"blueprint":batch::display(parent,&progress,&view,0,8)?,"replayed_locally":true,"native_calls":0}),
                );
            }
            state.abandon();
            if !state.blueprint_required
                || state.blueprint_storage.is_none()
                || state.control.mode() != DigMode::Control
                || before.total_records != 0
                || state.blueprint_recovery.is_some()
            {
                return Err(error(
                    ErrorCode::Conflict,
                    "a new blueprint requires its configured empty parent and empty original native journal",
                ));
            }
            batch::policy(&layout, &state.policy, c)?;
            let mut publication = work.take(c, batch::PUBLICATION_BYTES)?;
            // Native observation is read-only. No child preparation occurs in this call.
            let region = *layout.regions().first().ok_or_else(exhausted)?;
            let mut guard = PolicyDigGuard::new(&state.policy, &state.leases, runtime, None);
            let capture =
                state
                    .control
                    .observe(region, &work.operation(c, before)?, connect, &mut guard)?;
            let definition =
                BatchDefinition::new(layout, state.binding.clone(), before.journal_id, capture)?;
            publication.budget.max_wall_millis = work.current(c)?.budget.max_wall_millis;
            publication.anchor.tick = publication
                .anchor
                .tick
                .max(GameTick(state.control.high_tick()));
            let recovery = initial_recovery(&definition)?;
            state.blueprint_recovery = Some(recovery);
            let storage = state.blueprint_storage.take().ok_or_else(|| {
                error(
                    ErrorCode::CorruptLedger,
                    "original blueprint publication storage is absent",
                )
            })?;
            state.blueprint = Some(BatchStore::open(
                storage,
                &publication,
                DigMode::Control,
                Some(definition),
                true,
                None,
            )?);
            let parent = state.blueprint.as_ref().ok_or_else(|| {
                error(
                    ErrorCode::CorruptLedger,
                    "original blueprint publication missing",
                )
            })?;
            let progress = parent.definition().audit(before, &[], false)?;
            Ok(
                json!({"ok":true,"blueprint":batch::display(parent,&progress,before,0,8)?,
                "observation":state.control.selected().map(presentation::observation),
                "game_mutation_dispatched":false,"native_preparation_performed":false}),
            )
        }
        Action::StopBlueprint(expected) => {
            state.abandon();
            let parent = state
                .blueprint
                .as_mut()
                .filter(|p| p.definition().id() == expected)
                .ok_or_else(|| {
                    error(
                        ErrorCode::StaleAnchor,
                        "exact original blueprint is not retained",
                    )
                })?;
            parent.stop(&work.take(c, batch::PUBLICATION_BYTES)?)?;
            state.blueprint_recovery = Some(parent.recovery());
            Ok(
                json!({"ok":true,"blueprint":batch::reference(parent),"stopped":true,
                "native_effect_cancelled":false,"designation_undone":false,"history_erased":false}),
            )
        }
        Action::Observe(region) => {
            state.abandon();
            let next = if let (Some(parent), Some((view, records))) =
                (&state.blueprint, &original_inventory)
            {
                let progress = parent.definition().audit(view, records, parent.stopped())?;
                Some(parent.definition().key(progress.next_index.ok_or_else(|| {
                    error(
                        ErrorCode::Conflict,
                        "recover the current original blueprint step before observing another",
                    )
                })?)?)
            } else {
                None
            };
            let mut project = batch::Guard {
                parent: &mut state.blueprint,
                inventory: original_inventory.as_ref(),
                key: next.as_deref(),
                runtime,
                bytes: batch::GUARD_BYTES,
            };
            let mut guard = PolicyDigGuard::new(&state.policy, &state.leases, &mut project, None);
            let value =
                state
                    .control
                    .observe(region, &work.operation(c, before)?, connect, &mut guard)?;
            Ok(
                json!({"ok":true,"observation":presentation::observation(&value),"game_mutation_dispatched":false}),
            )
        }
        Action::Plan {
            key,
            witness,
            hidden,
        } => {
            if let Some(review) = &state.review {
                if review.key != key || review.witness != witness || review.hidden != hidden {
                    return Err(error(
                        ErrorCode::Conflict,
                        "another exact mining review is pending",
                    ));
                }
                let value = state
                    .control
                    .get(&key, review.plan, &work.take(c, LOCAL_BYTES)?)?;
                state.policy.evaluate(value.plan(), c, &state.leases)?;
                return Ok(
                    json!({"ok":true,"plan":presentation::record(&value),"review_seal":state.seal().map(|s|s.to_string()),"replayed_locally":true}),
                );
            }
            let capture = state
                .control
                .selected()
                .filter(|v| v.witness() == witness)
                .ok_or_else(|| {
                    error(
                        ErrorCode::StaleAnchor,
                        "planning needs this session's exact selected observation",
                    )
                })?;
            let plan = DigPlan::new(&key, hidden, capture.clone())?;
            state.policy.evaluate(&plan, c, &state.leases)?;
            if let (Some(parent), Some((view, records))) = (&state.blueprint, &original_inventory) {
                if hidden {
                    return Err(error(
                        ErrorCode::CapabilityDenied,
                        "blueprints require visible complete neighbor context",
                    ));
                }
                parent.definition().validate_next(
                    view,
                    records,
                    parent.stopped(),
                    &key,
                    capture.region(),
                    Some(capture),
                )?;
            }
            let mut project = batch::Guard {
                parent: &mut state.blueprint,
                inventory: original_inventory.as_ref(),
                key: Some(&key),
                runtime,
                bytes: batch::GUARD_BYTES,
            };
            let mut guard = PolicyDigGuard::new(&state.policy, &state.leases, &mut project, None);
            let value = state.control.prepare(
                &key,
                hidden,
                witness,
                &work.operation(c, before)?,
                &mut guard,
            )?;
            if value.state() == DigState::Prepared && state.control.has_preparation_connection() {
                let review = state
                    .policy
                    .review(value.plan(), &work.current(c)?, &state.leases)?;
                let blueprint_seal = if let Some(parent) = state.blueprint.as_ref() {
                    let prepared = state.control.view(&work.take(c, LOCAL_BYTES)?)?;
                    Some(batch::seal(
                        parent,
                        value.plan(),
                        prepared.head,
                        review.seal(),
                    ))
                } else {
                    None
                };
                state.review = Some(Review {
                    key,
                    plan: value.plan().digest(),
                    witness,
                    hidden,
                    value: review,
                    blueprint_seal,
                });
            }
            Ok(
                json!({"ok":true,"plan":presentation::record(&value),"review_seal":state.seal().map(|s|s.to_string()),"game_mutation_dispatched":false}),
            )
        }
        Action::Commit { key, plan, seal } => {
            let old = state.control.get(&key, plan, &work.take(c, LOCAL_BYTES)?)?;
            if old.state().terminal() {
                return Ok(
                    json!({"ok":true,"effect":presentation::record(&old),"native_calls":0,"historical_replay":true}),
                );
            }
            if !state.review.as_ref().is_some_and(|r| {
                r.key == key
                    && r.plan == plan
                    && r.blueprint_seal.map_or_else(|| r.value.seal(), |s| s) == seal
            }) {
                return Err(error(
                    ErrorCode::CapabilityDenied,
                    "exact local review must be confirmed",
                ));
            }
            state
                .policy
                .evaluate(old.plan(), &work.current(c)?, &state.leases)?;
            let review = state
                .review
                .take()
                .ok_or_else(|| error(ErrorCode::CapabilityDenied, "review unavailable"))?;
            if let Some(parent) = state.blueprint.as_ref() {
                if review.blueprint_seal
                    != Some(batch::seal(
                        parent,
                        old.plan(),
                        before.head,
                        review.value.seal(),
                    ))
                {
                    return Err(error(
                        ErrorCode::StaleAnchor,
                        "original blueprint prefix or review changed",
                    ));
                }
            }
            let policy_seal = review.value.seal();
            let confirmed = review.value.confirm(policy_seal)?;
            let mut project = batch::Guard {
                parent: &mut state.blueprint,
                inventory: original_inventory.as_ref(),
                key: Some(&key),
                runtime,
                bytes: batch::GUARD_BYTES,
            };
            let mut guard =
                PolicyDigGuard::new(&state.policy, &state.leases, &mut project, Some(&confirmed));
            let result = state
                .control
                .commit(&key, plan, &work.operation(c, before)?, &mut guard);
            state.abandon();
            Ok(
                json!({"ok":true,"effect":presentation::record(&result?),"excavation_completion_proven":false}),
            )
        }
        Action::Wait(key, plan) => {
            state.abandon();
            let mut guard = PolicyDigGuard::new(&state.policy, &state.leases, runtime, None);
            let result = state.control.reconcile(
                &key,
                plan,
                &work.operation(c, before)?,
                connect,
                &mut guard,
            )?;
            Ok(json!({"ok":true,"effect":presentation::record(&result),"commit_retried":false}))
        }
        Action::Cancel(key, plan) => {
            state.abandon();
            let mut guard = PolicyDigGuard::new(&state.policy, &state.leases, runtime, None);
            let result = state.control.cancel(
                &key,
                plan,
                &work.operation(c, before)?,
                connect,
                &mut guard,
            )?;
            Ok(json!({"ok":true,"effect":presentation::record(&result),"designation_undone":false}))
        }
        Action::Explain(key, plan) => {
            let value = state.control.get(&key, plan, &work.take(c, LOCAL_BYTES)?)?;
            Ok(json!({"ok":true,"record":presentation::record(&value),"native_calls":0}))
        }
        Action::Query(Query::Records {
            limit,
            continuation,
        }) => {
            let cursor = continuation
                .as_deref()
                .map(|s| cursors.resolve(s))
                .transpose()?;
            let page = state.control.list(
                &work.take(c, LOCAL_BYTES)?,
                limit.map_or(8, |n| n),
                cursor.as_ref(),
            )?;
            let next = page
                .continuation
                .map(|v| cursors.issue(v, c, page.head))
                .transpose()?;
            Ok(
                json!({"ok":true,"records":page.records.iter().map(presentation::summary).collect::<Vec<_>>(),
                "total_records":page.total_records,"unsettled_records":page.unsettled_records,"continuation":next,"native_calls":0}),
            )
        }
        Action::Query(Query::Blueprint {
            blueprint_id,
            expected_head,
            expected_parent_head,
            offset,
            limit,
        }) => {
            let expected = digest(&blueprint_id)?;
            let parent = state
                .blueprint
                .as_mut()
                .filter(|p| p.definition().id() == expected)
                .ok_or_else(|| {
                    error(
                        ErrorCode::StaleAnchor,
                        "original blueprint identity differs",
                    )
                })?;
            let (view, records) = state
                .control
                .complete_inventory(&work.take(c, COMPLETE_INVENTORY_BYTES)?)?;
            if expected_head
                .as_deref()
                .map(digest)
                .transpose()?
                .is_some_and(|head| head != view.head)
            {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "blueprint page belongs to an older journal head",
                ));
            }
            let progress = batch::verify(
                parent,
                &view,
                &records,
                &work.take(c, batch::PUBLICATION_BYTES)?,
            )?;
            if expected_parent_head
                .as_deref()
                .map(digest)
                .transpose()?
                .is_some_and(|head| head != parent.head())
            {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "blueprint page belongs to an older parent stop state",
                ));
            }
            Ok(
                json!({"ok":true,"blueprint":batch::display(parent,&progress,&view,offset.map_or(0,|n|n),limit.map_or(8,|n|n))?,"native_calls":0}),
            )
        }
        Action::Query(Query::BlueprintLayout { blueprint_id }) => {
            let expected = digest(&blueprint_id)?;
            let parent = state
                .blueprint
                .as_mut()
                .filter(|p| p.definition().id() == expected)
                .ok_or_else(|| {
                    error(
                        ErrorCode::StaleAnchor,
                        "original blueprint identity differs",
                    )
                })?;
            parent.verify(&work.take(c, batch::PUBLICATION_BYTES)?)?;
            let layout = parent.definition().layout();
            let blueprint: Value =
                serde_json::from_slice(layout.canonical_bytes()).map_err(|_| {
                    error(
                        ErrorCode::InternalInvariantViolation,
                        "canonical blueprint cannot be displayed",
                    )
                })?;
            Ok(
                json!({"ok":true,"layout":batch::preview(layout),"definition":blueprint,
                "blueprint_id":parent.definition().id().to_string(),"recovery":batch::recovery(parent.recovery()),"native_calls":0}),
            )
        }
        Action::Query(Query::SelectionTiles {
            witness,
            offset,
            limit,
        }) => {
            let wanted = digest(&witness)?;
            let selected = state
                .control
                .selected()
                .filter(|v| v.witness() == wanted)
                .ok_or_else(|| {
                    error(
                        ErrorCode::StaleAnchor,
                        "selection witness no longer retained",
                    )
                })?;
            let limit = limit.map_or(16, |n| n);
            let mut page = presentation::tile_page(selected, offset, limit)?;
            page["next_query"]=page["next_offset"].as_u64().map_or(Value::Null,|offset|
                json!({"mode":"selection_tiles","witness":witness,"offset":offset,"limit":limit}));
            Ok(json!({"ok":true,"page":page,"native_calls":0}))
        }
        Action::Query(Query::PlanTiles {
            idempotency_key,
            plan_digest,
            offset,
            limit,
        }) => {
            let value = state.control.get(
                &idempotency_key,
                digest(&plan_digest)?,
                &work.take(c, LOCAL_BYTES)?,
            )?;
            let limit = limit.map_or(16, |n| n);
            let mut page = presentation::tile_page(value.plan().before(), offset, limit)?;
            page["next_query"]=page["next_offset"].as_u64().map_or(Value::Null,|offset|
                json!({"mode":"plan_tiles","idempotency_key":idempotency_key,"plan_digest":plan_digest,"offset":offset,"limit":limit}));
            Ok(json!({"ok":true,"page":page,"native_calls":0}))
        }
        Action::Query(Query::Schema {}) => Ok(
            json!({"ok":true,"query_schema":serde_json::from_str::<Value>(include_str!("../../../schemas/dig_control_query.json"))
            .map_err(|_|error(ErrorCode::InternalInvariantViolation,"invalid embedded mining query schema"))?,"native_calls":0}),
        ),
        Action::Inventory => Ok(
            json!({"ok":true,"journal":inventory(before),"native_calls":0,"live_health_checked":false}),
        ),
        Action::Denied => Err(error(
            ErrorCode::CapabilityDenied,
            "mining journal is not a game checkpoint or restore",
        )),
    }
}

fn run_action<S, N, B, C, G, F>(
    state: &mut State<S, N, B, C>,
    c: OperationContext,
    op: &str,
    action: Result<Action>,
    started: Instant,
    runtime: &mut G,
    connect: F,
) -> String
where
    S: EffectJournalStorage,
    N: DigSource,
    B: EffectJournalStorage,
    C: EffectJournalStorage,
    G: DigSessionGuard,
    F: FnOnce(&DigBinding, DigRegion, &OperationContext) -> Result<N>,
{
    run_action_with_monitor(
        state,
        c,
        op,
        action,
        started,
        runtime,
        connect,
        &mut monitor::Unavailable,
    )
}
#[allow(clippy::too_many_arguments)]
fn run_action_with_monitor<S, N, B, C, G, F, M>(
    state: &mut State<S, N, B, C>,
    c: OperationContext,
    op: &str,
    action: Result<Action>,
    started: Instant,
    runtime: &mut G,
    connect: F,
    io: &mut M,
) -> String
where
    S: EffectJournalStorage,
    N: DigSource,
    B: EffectJournalStorage,
    C: EffectJournalStorage,
    G: DigSessionGuard,
    F: FnOnce(&DigBinding, DigRegion, &OperationContext) -> Result<N>,
    M: monitor::MonitorIo<B>,
{
    if action.is_err() {
        state.withdraw_monitor();
    }
    match action {
        Ok(
            action @ (Action::StartMonitor(_)
            | Action::SampleMonitor(_)
            | Action::CancelMonitor(_)
            | Action::Query(Query::BlueprintMonitor { .. })),
        ) => monitor::run_action(state, c, op, action, started, io),
        action => run_native_action(state, c, op, action, started, runtime, connect),
    }
}
#[allow(clippy::too_many_arguments)]
fn run_action_with_clock<S, N, B, C, G, F, M, I>(
    state: &mut State<S, N, B, C>,
    c: OperationContext,
    op: &str,
    action: Result<Action>,
    started: Instant,
    runtime: &mut G,
    connect: F,
    map: &mut M,
    clock_io: &mut I,
) -> String
where
    S: EffectJournalStorage,
    N: DigSource,
    B: EffectJournalStorage,
    C: EffectJournalStorage,
    G: DigSessionGuard,
    F: FnOnce(&DigBinding, DigRegion, &OperationContext) -> Result<N>,
    M: monitor::MonitorIo<B>,
    I: clock::ClockIo<C>,
{
    match action {
        Ok(
            action @ (Action::PrepareClock(_)
            | Action::CommitClock { .. }
            | Action::WaitClock(_)
            | Action::CancelClock(_)
            | Action::Query(Query::BlueprintClock { .. })),
        ) => clock::run_action(state, c, op, action, started, map, clock_io),
        Ok(action @ Action::SampleMonitor(_)) if state.clock_config.is_some() => {
            let mut c = c;
            if let Err(cause) = clock::ensure_for_sample(state, &mut c, started, clock_io) {
                state.abandon();
                state.withdraw_monitor();
                if let Some(clock) = state.clock.as_mut() {
                    clock.abandon_preparation();
                }
                return packet(
                    op,
                    retain_blueprint(state, failure(&cause)),
                    Some(&c),
                    Some(&state.binding),
                    None,
                    Some(&state.policy),
                    None,
                );
            }
            monitor::run_action(state, c, op, action, started, map)
        }
        action => run_action_with_monitor(state, c, op, action, started, runtime, connect, map),
    }
}
#[allow(clippy::too_many_arguments)]
fn run_native_action<S, N, B, C, G, F>(
    state: &mut State<S, N, B, C>,
    c: OperationContext,
    op: &str,
    action: Result<Action>,
    started: Instant,
    runtime: &mut G,
    connect: F,
) -> String
where
    S: EffectJournalStorage,
    N: DigSource,
    B: EffectJournalStorage,
    C: EffectJournalStorage,
    G: DigSessionGuard,
    F: FnOnce(&DigBinding, DigRegion, &OperationContext) -> Result<N>,
{
    let operation = action.as_ref().ok().and_then(|action| {
        let identity = match action {
            Action::Plan {key,witness,hidden} => state.control.selected().filter(|c|c.witness()==*witness)
                .and_then(|c|DigPlan::new(key,*hidden,c.clone()).ok())
                .map(|p|(p.key().to_owned(),p.digest())),
            Action::Commit {key,plan,..} | Action::Wait(key,plan) | Action::Cancel(key,plan)
                | Action::Explain(key,plan) => Some((key.clone(),*plan)),
            _ => None,
        };
        identity.map(|(key,plan)|json!({"idempotency_key":key,"plan_digest":plan.to_string(),
            "pre_operation_identity_only":true,"native_dispatch_proven":false,"retry_commit_permitted":false}))
    });
    let recovery_only = matches!(
        &action,
        Ok(Action::Wait(..)
            | Action::Cancel(..)
            | Action::Explain(..)
            | Action::Query(
                Query::Records { .. } | Query::PlanTiles { .. } | Query::Schema { .. }
            )
            | Action::Inventory)
    );
    if op == "fortress.observe" {
        state.abandon();
    }
    let mut work = match Work::new(&c, started) {
        Ok(w) => w,
        Err(e) => {
            return packet(
                op,
                retain_blueprint(state, failure(&e)),
                Some(&c),
                None,
                None,
                None,
                None,
            );
        }
    };
    let final_blueprint = if state.blueprint_required {
        match work.take(&c, COMPLETE_INVENTORY_BYTES + batch::PUBLICATION_BYTES) {
            Ok(reserved) => Some(reserved),
            Err(e) => {
                state.abandon();
                return packet(
                    op,
                    retain_blueprint(state, failure(&e)),
                    Some(&c),
                    Some(&state.binding),
                    None,
                    None,
                    None,
                );
            }
        }
    } else {
        None
    };
    let before = match work.view(&c).and_then(|c| state.control.view(&c)) {
        Ok(v) => v,
        Err(e) => {
            state.abandon();
            return packet(
                op,
                retain_blueprint(state, failure(&e)),
                Some(&c),
                Some(&state.binding),
                None,
                Some(&state.policy),
                None,
            );
        }
    };
    let mut staged = state.cursors.clone();
    let outcome = (|| {
        let current = work.current(&c)?;
        perform(
            state,
            &current,
            &mut work,
            &before,
            &mut staged,
            action?,
            runtime,
            connect,
        )
    })();
    if outcome.is_err()
        && matches!(
            op,
            "fortress.observe"
                | "fortress.plan"
                | "fortress.commit"
                | "fortress.wait"
                | "fortress.cancel"
        )
    {
        state.abandon();
    }
    let mut display = c.clone();
    display.anchor.tick = display.anchor.tick.max(GameTick(
        state.authority_tick.max(state.control.high_tick()),
    ));
    let after = work.view(&display).and_then(|c| state.control.view(&c));
    let (mut result, view) = match after {
        Ok(v) => (
            match outcome {
                Ok(v) => v,
                Err(e) => failure(&e),
            },
            Some(v),
        ),
        Err(e) => {
            state.abandon();
            let mut failed = failure(&e);
            failed["post_operation_inventory_unverified"] = json!(true);
            failed["historical_pending_before_request"] =
                json!(before.pending.as_ref().map(presentation::summary));
            (failed, None)
        }
    };
    if let (Some(parent), Some(mut reserved)) = (state.blueprint.as_mut(), final_blueprint) {
        let project = (|| {
            reserved.anchor = display.anchor;
            reserved.budget.max_wall_millis = work.current(&display)?.budget.max_wall_millis;
            reserved.budget.max_bytes = COMPLETE_INVENTORY_BYTES;
            let (full, records) = state.control.complete_inventory(&reserved)?;
            reserved.budget.max_wall_millis = work.current(&display)?.budget.max_wall_millis;
            reserved.budget.max_bytes = batch::PUBLICATION_BYTES;
            let progress = batch::verify(parent, &full, &records, &reserved)?;
            batch::display(parent, &progress, &full, 0, 8)
        })();
        match project {
            Ok(mut project) => {
                let enabled = json!(
                    state.control.mode() == DigMode::Control
                        && display
                            .authorize(
                                Capability::Plan,
                                RiskTier::Guarded,
                                &[],
                                Some(state.binding.scope())
                            )
                            .is_ok()
                        && display
                            .authorize(
                                Capability::Designate,
                                RiskTier::Guarded,
                                &[],
                                Some(state.binding.scope())
                            )
                            .is_ok()
                );
                project["new_designation_enabled"] = enabled.clone();
                if result
                    .get("blueprint")
                    .is_none_or(|b| b.get("inventory_verified") != Some(&Value::Bool(true)))
                {
                    result["blueprint"] = project;
                }
                result["blueprint"]["new_designation_enabled"] = enabled;
            }
            Err(e) => {
                if recovery_only && result["ok"] == true {
                    result["blueprint_error"] = failure(&e)["error"].clone();
                    result["blueprint_inventory_verified"] = json!(false);
                    if let Some(object) = result.as_object_mut() {
                        object.remove("blueprint");
                    }
                } else {
                    result = failure(&e);
                }
                state.abandon();
            }
        }
    }
    result = retain_blueprint(state, result);
    if result["ok"] != true {
        if let Some(operation) = operation.as_ref() {
            result["operation"] = operation.clone();
        }
        result["effect_may_have_occurred"] =
            json!(matches!(op, "fortress.commit" | "fortress.cancel"));
    }
    let rendered = packet(
        op,
        result,
        Some(&display),
        Some(&state.binding),
        view.as_ref(),
        Some(&state.policy),
        state.seal(),
    );
    if rendered.len() as u64 > OUTPUT_BYTES || work.current(&display).is_err() {
        state.abandon();
        let mut failed = failure(&error(
            ErrorCode::EffectIndeterminate,
            "response exceeded reservation; recover the journal",
        ));
        if let Some(operation) = operation {
            failed["operation"] = operation;
        }
        return packet(
            op,
            retain_blueprint(state, failed),
            Some(&display),
            Some(&state.binding),
            view.as_ref(),
            None,
            None,
        );
    }
    if view.is_some() {
        state.cursors = staged;
    }
    rendered
}
fn retain_blueprint<
    S: EffectJournalStorage,
    N: DigSource,
    B: EffectJournalStorage,
    C: EffectJournalStorage,
>(
    state: &State<S, N, B, C>,
    mut result: Value,
) -> Value {
    if result.get("blueprint").is_none() {
        if let Some(reference) = state.blueprint_reference() {
            result["blueprint"] = reference;
        }
    }
    if result.get("blueprint_monitor").is_none() {
        if let Some(reference) = state.monitor_reference() {
            result["blueprint_monitor"] = reference;
        }
    }
    if result.get("blueprint_clock").is_none() {
        if let Some(reference) = state.clock_reference() {
            result["blueprint_clock"] = reference;
        }
    }
    result
}
fn lock() -> Result<MutexGuard<'static, Option<Entry>>> {
    match SESSION.try_lock() {
        Ok(v) => Ok(v),
        Err(TryLockError::WouldBlock) => Err(error(
            ErrorCode::Conflict,
            "mining session is already serving a request",
        )),
        Err(TryLockError::Poisoned(_)) => Err(error(
            ErrorCode::InternalInvariantViolation,
            "mining session poisoned; restart for recovery",
        )),
    }
}
fn session_id(raw: &str) -> Result<SessionId> {
    if raw.len() != 32
        || !raw
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "invalid control session ID",
        ));
    }
    let n = u128::from_str_radix(raw, 16)
        .map_err(|_| error(ErrorCode::InvalidRequest, "invalid control session ID"))?;
    let id = SessionId::new(n);
    if id.get() != n || !id.is_process_scoped_live() || (n & ((1u128 << 62) - 1)) >> 57 != 18 {
        return Err(error(
            ErrorCode::InvalidRequest,
            "wrong mining session family",
        ));
    }
    Ok(id)
}
async fn with_session(
    raw: String,
    op: &'static str,
    wall: Option<u64>,
    action: Result<Action>,
) -> String {
    let native_effect_requested = matches!(
        &action,
        Ok(Action::Commit { .. }
            | Action::Cancel(..)
            | Action::CommitClock { .. }
            | Action::CancelClock(_))
    );
    runtime::owned(op, move |control: RequestControl| {
        let mut retained = None;
        let mut retained_monitor = None;
        let mut retained_clock = None;
        let mut native_handoff = None;
        let mut monitor_only = false;
        let mut clock_profile = None;
        let result = (|| {
            let id = session_id(&raw)?;
            let mut locked = lock()?;
            let entry = locked
                .as_mut()
                .filter(|v| v.id() == id)
                .ok_or_else(|| error(ErrorCode::SessionNotFound, "mining session absent"))?;
            if let Entry::Native { state, .. } = entry {
                state.withdraw_unacknowledged();
            }
            retained = entry.blueprint_reference();
            retained_monitor = entry.monitor_reference();
            retained_clock = entry.clock_reference();
            monitor_only = matches!(entry, Entry::Monitor { .. });
            if let Entry::ClockRecovery { state, .. } = entry {
                clock_profile = Some(state.mode().as_str());
            }
            if op == "fortress.observe" {
                entry.abandon();
            }
            let config = entry.config().clone();
            if let Err(cause) = runtime::boundary(&control, &config, false) {
                entry.abandon();
                if let Entry::Native { state, .. } = entry {
                    state.withdraw_monitor();
                }
                retained_monitor = entry.monitor_reference();
                retained_clock = entry.clock_reference();
                return Err(cause);
            }
            let rendered = match entry {
                Entry::Native { state, .. } => {
                    let c = match (|| {
                        state.context_with_clock(
                            runtime::enabled()?,
                            runtime::clock_enabled()?,
                            wall,
                        )
                    })() {
                        Ok(c) => c,
                        Err(cause) => {
                            state.withdraw_monitor();
                            state.abandon();
                            if let Some(clock) = state.clock.as_mut() {
                                clock.abandon_preparation();
                            }
                            retained_monitor = state.monitor_reference();
                            retained_clock = state.clock_reference();
                            return Err(cause);
                        }
                    };
                    let binding = state.binding.clone();
                    let mut guard = runtime::Guard {
                        control: &control,
                        config: &config,
                        binding: &binding,
                    };
                    let mut map_io = runtime::MonitorIo {
                        control: &control,
                        config: &config,
                        mode: state.control.mode(),
                    };
                    let mut clock_io = runtime::ClockIo {
                        control: &control,
                        config: &config,
                        mode: state.control.mode(),
                        opening_recovery: None,
                    };
                    run_action_with_clock(
                        state,
                        c,
                        op,
                        action,
                        control.started,
                        &mut guard,
                        |b, r, c| {
                            config.matches(b)?;
                            runtime::connect(&control, &config, r, c)
                        },
                        &mut map_io,
                        &mut clock_io,
                    )
                }
                Entry::Monitor { state, .. } => {
                    let c = state.context(wall)?;
                    state.run_action(c, op, action, control.started, |_| {
                        runtime::boundary(&control, &config, false)
                    })
                }
                Entry::ClockRecovery { state, .. } => {
                    let mut c = state.context(wall)?;
                    if !runtime::clock_enabled()? {
                        c.grants
                            .retain(|grant| grant.capability == Capability::Query);
                    }
                    let mut io = runtime::ClockIo {
                        control: &control,
                        config: &config,
                        mode: if state.mode() == clock_recovery::Mode::Offline {
                            DigMode::Offline
                        } else {
                            DigMode::Control
                        },
                        opening_recovery: None,
                    };
                    state.run_action(c, op, action, control.started, &mut io)
                }
            };
            retained = entry.blueprint_reference();
            retained_monitor = entry.monitor_reference();
            retained_clock = entry.clock_reference();
            native_handoff = presentation::retained_handoff(&rendered);
            if let Err(e) = runtime::boundary(&control, &config, false) {
                entry.abandon();
                if let Entry::Native { state, .. } = entry {
                    state.withdraw_monitor();
                }
                retained_monitor = entry.monitor_reference();
                retained_clock = entry.clock_reference();
                return Err(e);
            }
            if let Entry::Native { state, .. } = entry {
                state.worker_acknowledgement = Some(control.acknowledgement());
            }
            Ok(rendered)
        })();
        match result {
            Ok(value) => value,
            Err(e) => {
                let mut result = failure(&e);
                result["effect_may_have_occurred"] =
                    json!(native_effect_requested && !monitor_only);
                if monitor_only {
                    result["monitor_only"] = json!(true);
                }
                if let Some(profile) = clock_profile {
                    result["clock_recovery_only"] = json!(true);
                    result["profile"] = json!(profile);
                }
                if let Some(reference) = retained {
                    result["blueprint"] = reference;
                }
                if let Some(reference) = retained_monitor {
                    result["blueprint_monitor"] = reference;
                }
                if let Some(reference) = retained_clock {
                    result["blueprint_clock"] = reference;
                }
                if let Some(handoff) = native_handoff {
                    result["native_handoff"] = handoff;
                }
                packet(op, result, None, None, None, None, None)
            }
        }
    })
    .await
}

/// Carry a retained conservative clock floor into the new native owner and
/// host lease. The unchanged grants remain subject to their original expiry.
fn context_with_blueprint_floor<B: EffectJournalStorage>(
    c: &OperationContext,
    parent: Option<&BatchStore<B>>,
) -> OperationContext {
    let mut current = c.clone();
    if let Some(parent) = parent {
        current.anchor.tick = current
            .anchor
            .tick
            .max(GameTick(parent.definition().bootstrap().tick()));
    }
    current
}

/// A new Control owner needs a current clock from the exact retained source
/// before issuing its new lease. This read grants no permit and is not retained
/// as a planning observation. Offline and reconcile owners do not need it.
fn current_blueprint_context<N: DigSource>(
    c: &OperationContext,
    parent: &BatchDefinition,
    records: &[DigRecord],
    source: &N,
    region: DigRegion,
    capture: &DigObservation,
) -> Result<OperationContext> {
    let binding = parent.binding();
    let floor_tick = records.iter().fold(
        c.anchor.tick.get().max(parent.bootstrap().tick()),
        |tick, record| tick.max(record.plan().before().tick()),
    );
    let floor_sequence = records
        .iter()
        .fold(parent.bootstrap().sequence(), |sequence, record| {
            let designated = record
                .effect()
                .is_some_and(|e| e.phase().terminal() && e.designated_count().is_some());
            sequence.max(
                record
                    .plan()
                    .before()
                    .sequence()
                    .saturating_add(u64::from(designated)),
            )
        });
    if c.anchor.fortress_id != binding.fortress_id()
        || source.endpoint() != Some(binding.endpoint())
        || source.manifest() != binding.manifest()
        || !parent.layout().regions().contains(&region)
        || capture.region() != region
        || capture.dimensions() != parent.bootstrap().dimensions()
        || capture.tick() < floor_tick
        || capture.tick() > MAX_NATIVE_TICK
        || capture.sequence() < floor_sequence
        || DigBinding::new(
            binding.endpoint(),
            source.manifest().clone(),
            capture,
            binding.scope(),
        )? != *binding
    {
        return Err(error(
            ErrorCode::StaleAnchor,
            "current blueprint source differs from the retained clock or identity",
        ));
    }
    let mut current = c.clone();
    current.anchor.tick = GameTick(capture.tick());
    current.authorize(
        Capability::Query,
        RiskTier::ReadOnly,
        &[],
        Some(binding.scope()),
    )?;
    current.authorize(
        Capability::Observe,
        RiskTier::ReadOnly,
        &[],
        Some(region.halo()),
    )?;
    Ok(current)
}
fn opening_mode(raw: Option<&str>, config: &Config) -> Result<DigMode> {
    if config.monitor.is_some() && config.blueprint.is_none() {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "combined mining requires its original blueprint custody",
        ));
    }
    let mode = match raw {
        None | Some("control") => DigMode::Control,
        Some("offline") => DigMode::Offline,
        Some("reconcile") => DigMode::Recover,
        _ => {
            return Err(error(
                ErrorCode::InvalidRequest,
                "mode must be control, offline or reconcile",
            ));
        }
    };
    if mode != DigMode::Control && config.blueprint.as_ref().is_none_or(|p| p.initialize) {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "offline/reconcile requires the exact retained blueprint configuration",
        ));
    }
    Ok(mode)
}
fn opening_selection(
    raw: Option<&str>,
    region: Option<&str>,
    config: &Config,
) -> Result<(DigMode, Option<DigRegion>, bool)> {
    let recovery = match raw {
        Some("monitor-offline") => Some(DigMode::Offline),
        Some("monitor-recovery") => Some(DigMode::Recover),
        _ => None,
    };
    if let Some(mode) = recovery {
        if config.clock.is_some() {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "monitor-only recovery cannot open or conceal configured clock custody",
            ));
        }
        if region.is_some() {
            return Err(error(
                ErrorCode::InvalidRequest,
                "monitor-only recovery takes no native bootstrap region",
            ));
        }
        let selected = config.monitor.as_ref().ok_or_else(|| {
            error(
                ErrorCode::CapabilityDenied,
                "monitor recovery is not configured",
            )
        })?;
        if selected.initialize || selected.recovery.is_none() {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "monitor-only opening requires exact existing history",
            ));
        }
        Ok((mode, None, true))
    } else {
        let mode = opening_mode(raw, config)?;
        let region = parse_region(region.ok_or_else(|| {
            error(
                ErrorCode::InvalidRequest,
                "native mining opening requires a region",
            )
        })?)?;
        config.region(region)?;
        Ok((mode, Some(region), false))
    }
}

fn clock_recovery_selection(
    raw: Option<&str>,
    region: Option<&str>,
    config: &Config,
    game_ticks: Option<u64>,
) -> Result<Option<clock_recovery::Mode>> {
    let mode = match raw {
        Some("clock-offline") => clock_recovery::Mode::Offline,
        Some("clock-recovery") => clock_recovery::Mode::Recovery,
        _ => return Ok(None),
    };
    if region.is_some() || game_ticks.is_some_and(|ticks| ticks != 0) {
        return Err(error(
            ErrorCode::InvalidRequest,
            "independent clock recovery takes no bootstrap region or future game ticks",
        ));
    }
    let monitor = config.monitor.as_ref().ok_or_else(|| {
        error(
            ErrorCode::CapabilityDenied,
            "clock recovery requires its original monitor configuration",
        )
    })?;
    let clock = config.clock.as_ref().ok_or_else(|| {
        error(
            ErrorCode::CapabilityDenied,
            "clock recovery requires exact existing run custody",
        )
    })?;
    if monitor.initialize
        || monitor.recovery.is_none()
        || clock.initialize
        || clock.recovery.is_none()
    {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "independent recovery never initializes replacement history",
        ));
    }
    Ok(Some(mode))
}

/// Fresh opening grants for the two independent identities. Recovery never
/// issues observation, planning, designation or future advancement authority.
fn clock_recovery_authority(
    config: &Config,
    mode: clock_recovery::Mode,
    enabled: bool,
) -> (Vec<CapabilityGrant>, Vec<CapabilityGrant>) {
    let control = mode == clock_recovery::Mode::Recovery && enabled;
    let mut canonical = grants(config.fortress(), config.scope, false);
    canonical.retain(|grant| grant.capability == Capability::Query);
    if control {
        canonical.push(CapabilityGrant {
            capability: Capability::ControlClock,
            scope: CapabilityScope {
                fortress_id: Some(config.fortress()),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::Guarded,
            expires_at_tick: None,
            remaining_uses: None,
        });
    }
    let native = runtime::clock_grants(control)
        .into_iter()
        .filter(|grant| {
            matches!(
                grant.capability,
                Capability::Query | Capability::ControlClock
            )
        })
        .collect();
    (canonical, native)
}
#[tool(
    name = "fortress.open_session",
    description = "Open combined mining control or retained blueprint offline/reconcile with a region. Select clock-offline or clock-recovery without region and with zero game-tick allowance to recover exact existing monitor and clock histories without original dig/parent custody. Clock recovery can query the original run and, with separately retained clock authority, request its safety pause; it cannot plan or sample. Monitor-only modes remain available without configured clock custody. Paths/source pins are operator configuration."
)]
pub async fn fortress_open_session(
    region: Option<String>,
    mode: Option<String>,
    max_wall_millis: Option<u64>,
    max_bytes: Option<u64>,
    max_output_tokens: Option<u32>,
    max_game_ticks: Option<u64>,
) -> String {
    let requested_monitor_only = matches!(
        mode.as_deref(),
        Some("monitor-offline" | "monitor-recovery")
    );
    let requested_clock_only = matches!(mode.as_deref(), Some("clock-offline" | "clock-recovery"));
    runtime::owned("fortress.open_session", move |control| {
        let mut retained=None;
        let mut retained_monitor=None;
        let mut retained_clock=None;
        let result=(|| {
            let config=runtime::configuration()?;
            runtime::boundary(&control,&config,false)?;
            let recovery_mode=clock_recovery_selection(mode.as_deref(),region.as_deref(),&config,max_game_ticks)?;
            let (mode,region,monitor_only)=if let Some(recovery)=recovery_mode {
                (if recovery==clock_recovery::Mode::Offline {DigMode::Offline}else{DigMode::Control},None,false)
            }else{opening_selection(mode.as_deref(),region.as_deref(),&config)?};
            let game_ticks=max_game_ticks.map_or(0,|ticks|ticks);
            if game_ticks>1200 || (game_ticks>0 && (mode!=DigMode::Control || config.clock.is_none() || !runtime::clock_enabled()?)) {
                return Err(error(ErrorCode::CapabilityDenied,"positive clock work requires configured control, explicit clock opt-in and at most1200 ticks"));
            }
            let budget=WorkBudget {max_wall_millis:max_wall_millis.map_or(10_000,|n|n),
                max_bytes:max_bytes.map_or(MAX_WORK_BYTES,|n|n),max_output_tokens:max_output_tokens.map_or(8192,|n|n),
                max_entities:if config.monitor.is_some(){monitor::MAX_ENTITIES}else{300},max_actions:1,max_game_ticks:game_ticks};
            let mut locked=lock()?;
            if locked.is_some() {return Err(error(ErrorCode::Conflict,"release the current mining session first"));}
            let seq=NEXT.try_update(Ordering::AcqRel,Ordering::Acquire,|n|(n<(1u64<<57)).then_some(n+1)).map_err(|_|exhausted())?;
            let id=SessionId::new((1u128<<127)|FAMILY|u128::from(seq));
            let write=recovery_mode.is_none() && mode==DigMode::Control && runtime::enabled()?;
            let mut c=OperationContext {session_id:id,request_id:RequestId::new(1),budget,cancellation_requested:false,
                anchor:StateAnchor {fortress_id:config.fortress(),cursor:ObservationCursor::ORIGIN,tick:GameTick(0),state_hash:Digest32::ZERO},
                grants:grants(config.fortress(),config.scope,write)};
            let mut recovery_native_grants=vec![];
            if let Some(recovery)=recovery_mode {
                (c.grants,recovery_native_grants)=clock_recovery_authority(&config,recovery,runtime::clock_enabled()?);
            }else{
                if mode==DigMode::Offline || monitor_only {c.grants.retain(|g|g.capability==Capability::Query);}
                add_clock_grants(&mut c,&config,mode,runtime::clock_enabled()?);
            }
            let mut work=Work::new(&c,control.started)?;
            if let Some(recovery)=recovery_mode {
                use clock::ClockIo as _;
                let monitor_config=config.monitor.as_ref().ok_or_else(||error(ErrorCode::CapabilityDenied,"original monitor configuration absent"))?;
                let clock_config=config.clock.as_ref().ok_or_else(||error(ErrorCode::CapabilityDenied,"original clock configuration absent"))?;
                retained_monitor=monitor_config.recovery.map(monitor::recovery);
                retained_clock=clock_config.recovery.map(clock::recovery);
                // Reserve the whole opening before either private file is touched.
                let monitor_open=work.take(&c,monitor::OPEN_BYTES)?;
                let mut clock_open=work.take(&c,clock::OPEN_BYTES)?;
                let mut constructor=work.take(&c,clock_recovery::INSPECT_BYTES)?;
                let mut inspection=work.take(&c,clock_recovery::INSPECT_BYTES)?;
                let mut publication=work.take(&c,clock_recovery::INSPECT_BYTES)?;
                let store=dfmcp_adapter::dig_blueprint::monitor::store::open_private_monitor_store(
                    &monitor_config.path,&monitor_open,DigMode::Offline,None,false,monitor_config.recovery,config.scope)?;
                config.matches(store.spec().parent().binding())?;
                monitor_config.matches(store.spec())?;
                c.anchor.tick=c.anchor.tick.max(GameTick(store.high_tick()));
                retained_monitor=Some(monitor::reference(&store));
                if let Some(reference)=retained_monitor.as_mut(){reference["authority_tick_floor"]=json!(c.anchor.tick.get());}
                retained_clock=clock_config.recovery.map(|pin|json!({
                    "recovery":clock::recovery(pin),"authority_tick_floor":c.anchor.tick.get()}));
                clock_open.anchor=c.anchor;
                clock_open.budget.max_wall_millis=work.current(&c)?.budget.max_wall_millis;
                let native=clock::native_context(&clock_open,&recovery_native_grants)?;
                let mut io=runtime::ClockIo{control:&control,config:&config,mode,opening_recovery:None};
                let opened=io.open(store.spec(),&clock_open,&native);
                if let Some(pin)=io.opening_recovery(){retained_clock=Some(json!({
                    "recovery":clock::recovery(pin),"authority_tick_floor":c.anchor.tick.get()}));}
                let owner=opened?;
                c.anchor.tick=c.anchor.tick.max(GameTick(owner.high_tick()));
                let mut reference=clock::reference(&owner);
                reference["authority_tick_floor"]=json!(c.anchor.tick.get());
                retained_clock=Some(reference);
                // Current opt-in may have disappeared after initial grant issue.
                // The actual private owner's mode bounds this session forever.
                if owner.mode()!=dfmcp_adapter::bounded_run::journal::RunMode::Control {
                    c.grants.retain(|grant|grant.capability==Capability::Query);
                    recovery_native_grants.retain(|grant|grant.capability==Capability::Query);
                }
                constructor.anchor=c.anchor;
                constructor.grants=c.grants.clone();
                constructor.budget.max_wall_millis=work.current(&c)?.budget.max_wall_millis;
                let native=clock::native_context(&constructor,&recovery_native_grants)?;
                let mut state=clock_recovery::RecoveryState::new(store,owner,&constructor,&native,&config,recovery)?;
                state.set_budget(budget);
                c.anchor.tick=GameTick(state.high_tick());
                inspection.anchor=c.anchor;
                inspection.grants=c.grants.clone();
                inspection.budget.max_wall_millis=work.current(&c)?.budget.max_wall_millis;
                let native=clock::native_context(&inspection,&recovery_native_grants)?;
                let inspected=state.inspect(&inspection,&native);
                c.anchor.tick=c.anchor.tick.max(GameTick(state.high_tick()));
                retained_monitor=Some(state.monitor_reference());
                retained_clock=Some(state.clock_reference());
                let mut result=inspected?;
                result["session_id"]=json!(id.to_string());
                result["mode"]=json!(recovery.as_str());
                result["capabilities"]=json!(c.grants.iter().map(|grant|grant.capability.as_str()).collect::<Vec<_>>());
                result["native_capabilities"]=json!(recovery_native_grants.iter().map(|grant|grant.capability.as_str()).collect::<Vec<_>>());
                result["max_game_ticks"]=json!(0);
                let expected_monitor=result["blueprint_monitor"]["recovery"].clone();
                let expected_clock=result["blueprint_clock"]["recovery"].clone();
                let published_tick=c.anchor.tick;
                let rendered=packet("fortress.open_session",result,Some(&c),Some(state.binding()),None,None,None);
                if rendered.len() as u64>OUTPUT_BYTES{return Err(exhausted());}
                work.current(&c)?;
                runtime::boundary(&control,&config,false)?;
                publication.anchor=c.anchor;
                publication.grants=c.grants.clone();
                publication.budget.max_wall_millis=work.current(&c)?.budget.max_wall_millis;
                let native=clock::native_context(&publication,&recovery_native_grants)?;
                let final_check=state.inspect(&publication,&native);
                c.anchor.tick=c.anchor.tick.max(GameTick(state.high_tick()));
                retained_monitor=Some(state.monitor_reference());
                retained_clock=Some(state.clock_reference());
                let final_check=final_check?;
                if final_check["blueprint_monitor"]["recovery"]!=expected_monitor
                    || final_check["blueprint_clock"]["recovery"]!=expected_clock
                    || c.anchor.tick!=published_tick {
                    return Err(error(ErrorCode::StaleAnchor,"original recovered histories changed before publication"));
                }
                work.current(&c)?;
                *locked=Some(Entry::ClockRecovery{state,config});
                return Ok(rendered);
            }
            if monitor_only {
                let selected=config.monitor.as_ref().ok_or_else(||error(ErrorCode::CapabilityDenied,"monitor recovery configuration absent"))?;
                retained_monitor=selected.recovery.map(monitor::recovery);
                let opening=work.take(&c,monitor::OPEN_BYTES)?;
                let mut constructor=work.take(&c,2*monitor::FINAL_BYTES)?;
                let mut inspection=work.take(&c,monitor::FINAL_BYTES)?;
                let store=dfmcp_adapter::dig_blueprint::monitor::store::open_private_monitor_store(
                    &selected.path,&opening,mode,None,false,selected.recovery,config.scope)?;
                retained_monitor=Some(monitor::recovery(store.recovery()));
                c.anchor.tick=c.anchor.tick.max(GameTick(store.high_tick()));
                constructor.anchor=c.anchor;
                constructor.budget.max_wall_millis=work.current(&c)?.budget.max_wall_millis;
                let mut state=monitor_recovery::RecoveryState::new(store,&constructor,&config,mode)?;
                state.set_budget(budget);
                inspection.anchor=c.anchor;
                inspection.budget.max_wall_millis=work.current(&c)?.budget.max_wall_millis;
                let evidence=state.inspect(&inspection)?;
                let result=json!({"ok":true,"session_id":id.to_string(),"monitor_only":true,
                    "mode":if mode==DigMode::Offline{"monitor-offline"}else{"monitor-recovery"},
                    "capabilities":["query"],"blueprint_monitor":evidence,"native_calls":0,
                    "native_custody_opened":false,"blueprint_custody_opened":false,"game_mutations":0});
                let rendered=packet("fortress.open_session",result,Some(&c),Some(state.binding()),None,None,None);
                if rendered.len() as u64>OUTPUT_BYTES {return Err(exhausted());}
                work.current(&c)?;
                runtime::boundary(&control,&config,false)?;
                *locked=Some(Entry::Monitor{state,config});
                return Ok(rendered);
            }
            let region=region.ok_or_else(||error(ErrorCode::InternalInvariantViolation,"native opening lost its selected region"))?;
            let reopen_blueprint=config.blueprint.as_ref().is_some_and(|p|!p.initialize);
            // Reject predictable initialization failures before exclusive file
            // creation. An operator can then correct configuration and retry.
            if config.blueprint.as_ref().is_some_and(|p|p.initialize) {
                c.authorize(Capability::Plan,RiskTier::Guarded,&[],Some(config.scope))?;
                c.authorize(Capability::Designate,RiskTier::Guarded,&[],Some(config.scope))?;
                if config.checkpoint!=dfmcp_adapter::dig_control_policy::DigCheckpointPolicy::DisposableFortress {
                    return Err(error(ErrorCode::CheckpointRequired,"blueprint initialization requires the explicit operator checkpoint exception"));
                }
                runtime::boundary(&control,&config,true)?;
            }
            let mut parent_open=if config.blueprint.is_some() {Some(work.take(&c,batch::PUBLICATION_BYTES)?)} else {None};
            let journal=if reopen_blueprint {
                // Exact existing custody can be inspected without any native source.
                open_private_dig_exact(&config.path,&work.take(&c,OPEN_BYTES)?,mode,None,false)?
            } else {
                let mut source=runtime::connect(&control,&config,region,&work.take(&c,SOURCE_RESERVATION_BYTES)?)?;
                runtime::boundary(&control,&config,false)?;
                let before=source.observe(region,&work.take(&c,RPC_BYTES)?)?;
                if before.region()!=region || before.fortress_id()!=config.fortress()
                    || before.folder()!=config.folder || before.site()!=config.site || before.tick()>MAX_NATIVE_TICK {
                    return Err(error(ErrorCode::StaleAnchor,"bootstrap source differs from configured fortress"));
                }
                c.anchor.tick=GameTick(before.tick());
                let binding=DigBinding::new(config.endpoint,source.manifest().clone(),&before,config.scope)?;
                drop(source);
                runtime::boundary(&control,&config,false)?;
                if config.blueprint.is_some() {
                    open_private_dig_exact(&config.path,&work.take(&c,OPEN_BYTES)?,mode,Some(binding),true)?
                } else {open_private_dig(&config.path,&work.take(&c,OPEN_BYTES)?,mode,Some(binding))?}
            };
            let mut parent_storage=None;
            let mut parent_owner=None;
            if let Some(parent)=&config.blueprint {
                runtime::boundary(&control,&config,false)?;
                retained=parent.recovery.map(batch::recovery);
                let mut reserved=parent_open.take().ok_or_else(exhausted)?;
                reserved.anchor=c.anchor;
                reserved.budget.max_wall_millis=work.current(&c)?.budget.max_wall_millis;
                let (storage,remaining)=open_private_blueprint_storage(&parent.path,&reserved,mode,parent.initialize,config.scope)?;
                if parent.initialize {
                    parent_storage=Some(storage);
                } else {
                    let owned=BatchStore::open(storage,&remaining,mode,None,false,parent.recovery)?;
                    config.matches(owned.definition().binding())?;
                    retained=Some(batch::recovery(owned.recovery()));
                    parent_owner=Some(owned);
                }
            }
            // Parent bootstrap is a conservative clock floor, including when no
            // child intent exists yet. It supplies neither a grant nor a permit.
            c=context_with_blueprint_floor(&c,parent_owner.as_ref());
            let mut monitor_owner=None;
            if let Some(selected)=config.monitor.as_ref().filter(|p|!p.initialize) {
                retained_monitor=selected.recovery.map(monitor::recovery);
                let owned=dfmcp_adapter::dig_blueprint::monitor::store::open_private_monitor_store(
                    &selected.path,&work.take(&c,monitor::OPEN_BYTES)?,mode,None,false,selected.recovery,config.scope)?;
                selected.matches(owned.spec())?;
                c.anchor.tick=c.anchor.tick.max(GameTick(owned.high_tick()));
                owned.spec().authorize(&work.current(&c)?,false)?;
                retained_monitor=Some(monitor::recovery(owned.recovery()));
                monitor_owner=Some(owned);
            }
            let mut session=DigSession::<_,Native>::new(journal,&work.view(&c)?)?;
            c.anchor.tick=c.anchor.tick.max(GameTick(session.high_tick()));
            if let Some(owned)=parent_owner.as_mut() {
                if owned.definition().binding()!=session.binding() {
                    return Err(error(ErrorCode::CorruptLedger,"blueprint belongs to another native source or scope"));
                }
                let (view,records)=session.complete_inventory(&work.take(&c,COMPLETE_INVENTORY_BYTES)?)?;
                batch::verify(owned,&view,&records,&work.take(&c,batch::PUBLICATION_BYTES)?)?;
                if let Some(proof)=monitor_owner.as_ref() {proof.spec().verify_original(owned.definition(),&view,&records)?;}
                // A complete original monitor permits clock recovery even
                // while the game runs; no further child can be designated.
                if mode==DigMode::Control && !(config.clock.is_some() && monitor_owner.is_some()) {
                    let first=owned.definition().bootstrap().region();
                    let mut source=runtime::connect(&control,&config,first,&work.take(&c,SOURCE_RESERVATION_BYTES)?)?;
                    runtime::boundary(&control,&config,false)?;
                    let before=source.observe(first,&work.take(&c,RPC_BYTES)?)?;
                    c=current_blueprint_context(&c,owned.definition(),&records,&source,first,&before)?;
                    session.retain_read_floor(&before,&work.current(&c)?)?;
                    drop(source);
                    runtime::boundary(&control,&config,false)?;
                }
            }
            let mut state:State<_,_,PrivateBuildFile,PrivateRunFile>=State::new(session,&work.take(&c,LOCAL_BYTES)?,&config)?;
            state.budget=budget;
            state.grants=c.grants.clone();
            state.clock_grants=runtime::clock_grants(mode==DigMode::Control && config.clock.is_some() && runtime::clock_enabled()?);
            state.blueprint_storage=parent_storage;
            if let Some(owned)=monitor_owner {
                state.monitor_recovery=Some(owned.recovery());
                state.monitor=Some(owned);
            }
            if let Some(owned)=parent_owner {
                state.blueprint_recovery=Some(owned.recovery());
                state.blueprint=Some(owned);
            }
            // Acquire exact custody as soon as its monitor exists. An existing
            // journal mislabeled initialize:true must never bypass sampling.
            if let Some(selected)=config.clock.as_ref().filter(|clock|!clock.initialize || state.monitor.is_some()) {
                use clock::ClockIo as _;
                retained_clock=selected.recovery.map(clock::recovery);
                let spec=state.monitor.as_ref().ok_or_else(||error(ErrorCode::CorruptLedger,"clock recovery requires its exact original monitor"))?.spec().clone();
                let opening=work.take(&c,clock::OPEN_BYTES)?;
                let n=clock::native_context(&opening,&state.clock_grants)?;
                let mut io=runtime::ClockIo{control:&control,config:&config,mode,opening_recovery:None};
                let opened=io.open(&spec,&opening,&n);
                if let Some(recovery)=io.opening_recovery() {retained_clock=Some(clock::recovery(recovery));}
                let owned=opened?;
                state.authority_tick=state.authority_tick.max(owned.high_tick());
                c.anchor.tick=c.anchor.tick.max(GameTick(owned.high_tick()));
                retained_clock=Some(clock::recovery(owned.recovery()));
                state.clock_recovery=Some(owned.recovery());
                state.clock=Some(owned);
            }
            if state.blueprint_storage.is_some() && state.control.view(&work.view(&c)?)?.total_records!=0 {
                return Err(error(ErrorCode::Conflict,"a new blueprint requires a new empty native journal"));
            }
            let view=state.control.view(&work.view(&c)?)?;
            let mut result=json!({"ok":true,"session_id":id.to_string(),"journal":inventory(&view),
                "mode":match mode {DigMode::Control=>"control",DigMode::Recover=>"reconcile",DigMode::Offline=>"offline"},
                "capabilities":c.grants.iter().map(|g|g.capability.as_str()).collect::<Vec<_>>(),
                "planning_observation_retained":false,"native_designation_dispatched":false,
                "blueprint_configured":state.blueprint_required,"blueprint_creation_pending":state.blueprint_storage.is_some(),
                "blueprint_monitor_configured":state.monitor_config.is_some(),
                "blueprint_clock_configured":state.clock_config.is_some(),
                "clock_control_enabled":state.clock.as_ref().map_or(mode==DigMode::Control && runtime::clock_enabled()?,BlueprintClock::is_control),
                "max_game_ticks":budget.max_game_ticks});
            if let Some(parent)=state.blueprint.as_mut() {
                let (full,records)=state.control.complete_inventory(&work.take(&c,COMPLETE_INVENTORY_BYTES)?)?;
                let progress=batch::verify(parent,&full,&records,&work.take(&c,batch::PUBLICATION_BYTES)?)?;
                result["blueprint"]=batch::display(parent,&progress,&full,0,8)?;
            }
            if let Some(proof)=state.monitor.as_mut() {
                proof.verify(&work.take(&c,monitor::FINAL_BYTES)?)?;
                result["blueprint_monitor"]=monitor::display(proof,false,true,0,16)?;
            }
            if let Some(reference)=state.clock_reference() {result["blueprint_clock"]=reference;}
            let rendered=packet("fortress.open_session",result,Some(&c),Some(&state.binding),Some(&view),Some(&state.policy),None);
            if rendered.len() as u64>OUTPUT_BYTES {return Err(exhausted());}
            work.current(&c)?;
            runtime::boundary(&control,&config,false)?;
            *locked=Some(Entry::Native {state,config});
            Ok(rendered)
        })();
        match result {
            Ok(v)=>v,
            Err(e)=>{
                let mut result=failure(&e);
                if requested_monitor_only {result["monitor_only"]=json!(true);}
                if requested_clock_only {result["clock_recovery_only"]=json!(true);}
                if let Some(recovery)=retained {result["blueprint"]=json!({"recovery":recovery,"inventory_verified":false});}
                if let Some(reference)=retained_monitor {result["blueprint_monitor"]=presentation::opening_monitor_reference(reference);}
                if let Some(reference)=retained_clock {result["blueprint_clock"]=presentation::opening_clock_reference(reference);}
                packet("fortress.open_session",result,None,None,None,None,None)
            }
        }
    }).await
}

fn parse_region(raw: &str) -> Result<DigRegion> {
    if raw.len() > 128 {
        return Err(error(ErrorCode::InvalidRequest, "region exceeds its bound"));
    }
    let [x, y, z, w, h]: [u32; 5] = serde_json::from_str(raw).map_err(|_| {
        error(
            ErrorCode::InvalidRequest,
            "region must be [x,y,z,width,height]",
        )
    })?;
    DigRegion::new(x, y, z, w, h)
}
#[tool(
    name = "fortress.observe",
    description = "Retain one exact bounded mining observation and its native connection. region is [x,y,z,width,height], width/height 1..8. A new observation abandons old local commit permission, never the durable obligation. No game mutation."
)]
pub async fn fortress_observe(session_id: String, region: String) -> String {
    with_session(
        session_id,
        "fortress.observe",
        None,
        parse_region(&region).map(Action::Observe),
    )
    .await
}
#[tool(
    name = "fortress.plan",
    description = "Pass blueprint alone to retain a complete configured sparse floor project. After every original child is terminal Designated, pass monitor alone as closed JSON with blueprint_id, deadline_tick and optional stability/sample bounds to register read-only whole-blueprint completion monitoring. Otherwise pass the complete idempotency_key, observation_witness and allow_hidden_neighbors review request. No designation is dispatched during plan. Pass clock alone as closed JSON with monitor_id, game_ticks and wall_millis to prepare one separately reviewed finite clock interval."
)]
pub async fn fortress_plan(
    session_id: String,
    idempotency_key: Option<String>,
    observation_witness: Option<String>,
    allow_hidden_neighbors: Option<bool>,
    blueprint: Option<String>,
    monitor: Option<String>,
    clock: Option<String>,
) -> String {
    let action = plan_with_clock_action(
        idempotency_key,
        observation_witness,
        allow_hidden_neighbors,
        blueprint,
        monitor,
        clock,
    );
    with_session(session_id, "fortress.plan", None, action).await
}
fn plan_with_clock_action(
    id: Option<String>,
    observed: Option<String>,
    hidden: Option<bool>,
    blueprint: Option<String>,
    monitor: Option<String>,
    clock: Option<String>,
) -> Result<Action> {
    if let Some(raw) = clock {
        if id.is_some()
            || observed.is_some()
            || hidden.is_some()
            || blueprint.is_some()
            || monitor.is_some()
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "supply the clock request alone",
            ));
        }
        clock::Request::parse(&raw).map(Action::PrepareClock)
    } else {
        plan_with_monitor_action(id, observed, hidden, blueprint, monitor)
    }
}
fn plan_with_monitor_action(
    id: Option<String>,
    observed: Option<String>,
    hidden: Option<bool>,
    blueprint: Option<String>,
    monitor: Option<String>,
) -> Result<Action> {
    if let Some(raw) = monitor {
        if id.is_some() || observed.is_some() || hidden.is_some() || blueprint.is_some() {
            return Err(error(
                ErrorCode::InvalidRequest,
                "supply the monitor request alone",
            ));
        }
        monitor::Request::parse(&raw).map(Action::StartMonitor)
    } else {
        plan_action(id, observed, hidden, blueprint)
    }
}
fn plan_action(
    idempotency_key: Option<String>,
    observation_witness: Option<String>,
    hidden: Option<bool>,
    blueprint: Option<String>,
) -> Result<Action> {
    match (idempotency_key, observation_witness, hidden, blueprint) {
        (None, None, None, Some(raw)) => batch::parse(&raw).map(Action::Blueprint),
        (Some(id), Some(observed), Some(hidden), None) => {
            key(&id)?;
            Ok(Action::Plan {
                key: id,
                witness: digest(&observed)?,
                hidden,
            })
        }
        _ => Err(error(
            ErrorCode::InvalidRequest,
            "supply either the complete blueprint alone or one complete child review request",
        )),
    }
}
#[tool(
    name = "fortress.commit",
    description = "Confirm the exact reviewed plan digest and review seal for one native commit attempt on the original preparation connection. Current policy, leases, capabilities, source and complete terrain witness are rechecked. Lost replies require exact-record recovery, never retry. For clock advancement use scope=clock, monitor_id and the separately returned clock review_seal, omitting native key/digest. Neither path proves completed excavation."
)]
pub async fn fortress_commit(
    session_id: String,
    idempotency_key: Option<String>,
    plan_digest: Option<String>,
    review_seal: String,
    scope: Option<String>,
    monitor_id: Option<String>,
) -> String {
    let action = commit_action(
        scope.as_deref(),
        idempotency_key,
        plan_digest,
        review_seal,
        monitor_id,
    );
    with_session(session_id, "fortress.commit", None, action).await
}
fn commit_action(
    scope: Option<&str>,
    key_value: Option<String>,
    plan: Option<String>,
    seal: String,
    monitor: Option<String>,
) -> Result<Action> {
    match (scope.unwrap_or("effect"), key_value, plan, monitor) {
        ("effect", Some(key_value), Some(plan), None) => {
            key(&key_value)?;
            Ok(Action::Commit {
                key: key_value,
                plan: digest(&plan)?,
                seal: digest(&seal)?,
            })
        }
        ("clock", None, None, Some(monitor)) => Ok(Action::CommitClock {
            monitor_id: digest(&monitor)?,
            seal: digest(&seal)?,
        }),
        _ => Err(error(
            ErrorCode::InvalidRequest,
            "commit requires one complete native effect or clock review",
        )),
    }
}
#[tool(
    name = "fortress.query",
    description = "Bounded local records, selection_tiles, plan_tiles, blueprint, blueprint_layout, blueprint_monitor, blueprint_clock or schema query. query is closed JSON. blueprint_monitor requires monitor_id, accepts expected_head, offset and limit, and shows durable historical progress and selected tile deficits. Query never acquires fresh terrain or proves current completion."
)]
pub async fn fortress_query(session_id: String, query: String) -> String {
    with_session(
        session_id,
        "fortress.query",
        None,
        Query::parse(&query).map(Action::Query),
    )
    .await
}
fn effect_action(
    key_value: String,
    raw: String,
    make: impl FnOnce(String, Digest32) -> Action,
) -> Result<Action> {
    key(&key_value)?;
    Ok(make(key_value, digest(&raw)?))
}
#[tool(
    name = "fortress.wait",
    description = "Pass monitor_id alone to acquire one paused whole-blueprint map capture, retain it durably and advance its fixed sampled stability goal. Or pass idempotency_key and plan_digest to reconcile one original native designation outcome. Terminal monitors/effects return historical evidence locally. Use scope=clock with monitor_id to reconcile one retained run before sampling. No commit replay, polling loop or game-clock advance."
)]
pub async fn fortress_wait(
    session_id: String,
    idempotency_key: Option<String>,
    plan_digest: Option<String>,
    monitor_id: Option<String>,
    max_wall_millis: Option<u64>,
    scope: Option<String>,
) -> String {
    with_session(
        session_id,
        "fortress.wait",
        max_wall_millis,
        wait_with_clock_action(scope.as_deref(), idempotency_key, plan_digest, monitor_id),
    )
    .await
}
fn wait_with_clock_action(
    scope: Option<&str>,
    key: Option<String>,
    plan: Option<String>,
    monitor: Option<String>,
) -> Result<Action> {
    match scope {
        None => wait_action(key, plan, monitor),
        Some("clock") if key.is_none() && plan.is_none() => monitor
            .as_deref()
            .ok_or_else(|| error(ErrorCode::InvalidRequest, "clock wait requires monitor_id"))
            .and_then(digest)
            .map(Action::WaitClock),
        _ => Err(error(
            ErrorCode::InvalidRequest,
            "wait scope and identity disagree",
        )),
    }
}
fn wait_action(
    key: Option<String>,
    plan: Option<String>,
    monitor: Option<String>,
) -> Result<Action> {
    match (key, plan, monitor) {
        (Some(key), Some(plan), None) => effect_action(key, plan, Action::Wait),
        (None, None, Some(id)) => digest(&id).map(Action::SampleMonitor),
        _ => Err(error(
            ErrorCode::InvalidRequest,
            "wait requires one exact native operation or monitor identity",
        )),
    }
}
#[tool(
    name = "fortress.explain",
    description = "Inspect the exact retained mining plan, witness and native receipt without a new game observation or mutation."
)]
pub async fn fortress_explain(
    session_id: String,
    idempotency_key: String,
    plan_digest: String,
) -> String {
    with_session(
        session_id,
        "fortress.explain",
        None,
        effect_action(idempotency_key, plan_digest, Action::Explain),
    )
    .await
}
async fn close(raw: String, release: bool) -> String {
    runtime::owned("fortress.cancel", move |control| {
        let mut retained = None;
        let mut retained_monitor = None;
        let mut retained_clock = None;
        let mut monitor_only = false;
        let mut clock_profile = None;
        let result = (|| {
            control.checkpoint()?;
            let id = session_id(&raw)?;
            let mut locked = lock()?;
            let entry = locked
                .as_mut()
                .filter(|e| e.id() == id)
                .ok_or_else(|| error(ErrorCode::SessionNotFound, "mining session absent"))?;
            retained = entry.blueprint_reference();
            retained_monitor = entry.monitor_reference();
            retained_clock = entry.clock_reference();
            if let Entry::Monitor { state, config } = entry {
                monitor_only = true;
                let c = state.context(None)?;
                let output = state.close(c.clone(), release, control.started, |_| {
                    if release {
                        control.checkpoint()
                    } else {
                        runtime::boundary(&control, config, false)
                    }
                });
                retained_monitor = Some(state.reference());
                if output.len() as u64 > OUTPUT_BYTES {
                    return Err(exhausted());
                }
                let value: Value = serde_json::from_str(&output).map_err(|_| {
                    error(
                        ErrorCode::InternalInvariantViolation,
                        "invalid recovery close response",
                    )
                })?;
                Work::new(&c, control.started)?.current(&c)?;
                if value["result"]["ok"] == true {
                    drop(locked.take());
                }
                return Ok(output);
            }
            if let Entry::ClockRecovery { state, config } = entry {
                clock_profile = Some(state.mode().as_str());
                let c = state.context(None)?;
                let output = state.close(c.clone(), release, control.started, |_| {
                    if release {
                        control.checkpoint()
                    } else {
                        runtime::boundary(&control, config, false)
                    }
                });
                retained_monitor = Some(state.monitor_reference());
                retained_clock = Some(state.clock_reference());
                if output.len() as u64 > OUTPUT_BYTES {
                    return Err(exhausted());
                }
                let value: Value = serde_json::from_str(&output).map_err(|_| {
                    error(
                        ErrorCode::InternalInvariantViolation,
                        "invalid clock recovery close response",
                    )
                })?;
                Work::new(&c, control.started)?.current(&c)?;
                if value["result"]["ok"] == true {
                    drop(locked.take());
                }
                return Ok(output);
            }
            let Entry::Native { state, config } = entry else {
                return Err(error(
                    ErrorCode::InternalInvariantViolation,
                    "unknown mining session owner",
                ));
            };
            state.abandon();
            if let Some(clock) = state.clock.as_mut() {
                clock.abandon_preparation();
            }
            let c = state.context(false, None)?;
            let mut work = Work::new(&c, control.started)?;
            let view = if release {
                None
            } else {
                runtime::boundary(&control, config, false)?;
                if state.clock_config.is_some() {
                    let reserved = work.take(&c, clock::VIEW_BYTES)?;
                    clock::inspect(state, &reserved)?;
                    retained_clock = state.clock_reference();
                    if clock::blocks_sampling(state) {
                        return Err(error(
                            ErrorCode::EffectIndeterminate,
                            "unsettled clock requires explicit recovery release",
                        ));
                    }
                }
                Some(state.control.view(&work.view(&c)?)?)
            };
            if !release && view.as_ref().is_some_and(|v| v.pending.is_some()) {
                return Err(error(
                    ErrorCode::EffectIndeterminate,
                    "unsettled work requires explicit recovery release",
                ));
            }
            let result = retain_blueprint(
                state,
                json!({"ok":true,"scope":"session","closed":true,"release_for_recovery":release,
                "effects_cancelled":false,"history_erased":false,"native_quiescence_proven":false}),
            );
            let output = packet(
                "fortress.cancel",
                result,
                Some(&c),
                Some(&state.binding),
                view.as_ref(),
                None,
                None,
            );
            if output.len() as u64 > OUTPUT_BYTES {
                return Err(exhausted());
            }
            work.current(&c)?;
            if release {
                control.checkpoint()?;
            } else {
                runtime::boundary(&control, config, false)?;
            }
            drop(locked.take());
            Ok(output)
        })();
        match result {
            Ok(v) => v,
            Err(e) => {
                let mut result = failure(&e);
                result["effect_may_have_occurred"] = json!(false);
                if monitor_only {
                    result["monitor_only"] = json!(true);
                }
                if let Some(profile) = clock_profile {
                    result["clock_recovery_only"] = json!(true);
                    result["profile"] = json!(profile);
                }
                if let Some(reference) = retained {
                    result["blueprint"] = reference;
                }
                if let Some(reference) = retained_monitor {
                    result["blueprint_monitor"] = reference;
                }
                if let Some(reference) = retained_clock {
                    result["blueprint_clock"] = reference;
                }
                packet("fortress.cancel", result, None, None, None, None, None)
            }
        }
    })
    .await
}
#[tool(
    name = "fortress.cancel",
    description = "scope=clock with monitor_id retires a run or requests its safety pause. scope=monitor with monitor_id stops only the read-only excavation monitor and preserves its history. scope=blueprint with blueprint_id permanently stops original child advancement. scope=effect retires an exact native preparation. scope=session releases custody, with release_for_recovery=true for unresolved effects. None of these undo excavation or erase history."
)]
pub async fn fortress_cancel(
    session_id: String,
    scope: String,
    idempotency_key: Option<String>,
    plan_digest: Option<String>,
    release_for_recovery: Option<bool>,
    blueprint_id: Option<String>,
    monitor_id: Option<String>,
) -> String {
    let release = release_for_recovery.is_some_and(|v| v);
    match (
        scope.as_str(),
        idempotency_key,
        plan_digest,
        blueprint_id,
        monitor_id,
    ) {
        ("session", None, None, None, None) => close(session_id, release).await,
        ("clock", None, None, None, Some(id)) if !release => {
            with_session(
                session_id,
                "fortress.cancel",
                None,
                digest(&id).map(Action::CancelClock),
            )
            .await
        }
        ("monitor", None, None, None, Some(id)) if !release => {
            with_session(
                session_id,
                "fortress.cancel",
                None,
                digest(&id).map(Action::CancelMonitor),
            )
            .await
        }
        ("blueprint", None, None, Some(id), None) if !release => {
            with_session(
                session_id,
                "fortress.cancel",
                None,
                digest(&id).map(Action::StopBlueprint),
            )
            .await
        }
        ("effect", Some(key), Some(plan), None, None) if !release => {
            with_session(
                session_id,
                "fortress.cancel",
                None,
                effect_action(key, plan, Action::Cancel),
            )
            .await
        }
        _ => unbound(
            "fortress.cancel",
            &error(
                ErrorCode::InvalidRequest,
                "cancel scope and identity disagree",
            ),
        ),
    }
}
#[tool(
    name = "fortress.checkpoint",
    description = "Unavailable: the mining journal is not a verified game save. Required-checkpoint policy refuses new designation."
)]
pub async fn fortress_checkpoint(session_id: String) -> String {
    with_session(session_id, "fortress.checkpoint", None, Ok(Action::Denied)).await
}
#[tool(
    name = "fortress.restore",
    description = "Unavailable: journal replay does not restore terrain or undo mining."
)]
pub async fn fortress_restore(session_id: String) -> String {
    with_session(session_id, "fortress.restore", None, Ok(Action::Denied)).await
}
#[tool(
    name = "fortress.doctor",
    description = "Verify local mining coordination inventory; this is not live-game health, structural safety or production admission."
)]
pub async fn fortress_doctor(session_id: String) -> String {
    with_session(session_id, "fortress.doctor", None, Ok(Action::Inventory)).await
}
pub fn run_stdio() {
    if let Err(e) = runtime::configuration() {
        eprintln!("{e}");
        std::process::exit(1);
    }
    let server=ServerBuilder::new("dfmcp-dig-control-dev",env!("CARGO_PKG_VERSION"))
        .tool(FortressOpenSession).tool(FortressObserve).tool(FortressQuery).tool(FortressPlan).tool(FortressCommit)
        .tool(FortressWait).tool(FortressCancel).tool(FortressCheckpoint).tool(FortressRestore).tool(FortressExplain).tool(FortressDoctor)
        .instructions("Unadmitted development mining control. Default policy requires an unavailable game checkpoint. Explicit operator disposable-fortress policy permits consideration. Optional operator blueprint custody retains one complete sparse floor project: plan with blueprint alone, then observe/prepare/review/confirm each original next region. Only terminal Designated receipts unlock further children and whole-blueprint monitoring. Register a configured monitor through plan, sample it through wait with monitor_id, inspect its historical progress through query, and stop it with cancel scope=monitor. One independent map capture covers the whole original mask; completion requires visible dry floors with cleared designations over the fixed shared sampled stability window. Source pins do not prove shared process identity or mining causality. Never replace uncertain work or extend the original deadline. Blueprint stop preserves native recovery and does not stop read-only monitoring. Offline/reconcile reopening restores no commit permit. Optional independent run/1.13 custody permits one explicitly budgeted and separately reviewed finite interval: plan with clock alone, commit with scope=clock and monitor_id, reconcile with wait scope=clock, or request a safety pause with cancel scope=clock. Pending or unresolved intervals block fresh completion sampling. Retain exact recovery heads; restart restores no commit permit. No automatic run chaining, verified checkpoint or restore.")
        .build();
    crate::run_modern_stdio(server);
}
#[cfg(test)]
mod batch_tests;
#[cfg(test)]
mod clock_recovery_routing_tests;
#[cfg(test)]
mod tests;
