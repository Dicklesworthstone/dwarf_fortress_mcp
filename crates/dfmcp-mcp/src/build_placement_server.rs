#![forbid(unsafe_code)]
//! Isolated furniture/1.19 development control through the frozen eleven tools.
//! One session owns private journal custody and its original preparation source.
use dfmcp_adapter::build_placement::journal::private_file::{PrivateBuildFile, open_private_build};
use dfmcp_adapter::build_placement::journal::{BuildGuard, BuildInventory, BuildMode, BuildSource};
use dfmcp_adapter::build_placement::rpc::BuildRpc;
use dfmcp_adapter::build_placement::session::BuildSession;
use dfmcp_adapter::build_placement::{BuildBinding, BuildKind, BuildPlan, BuildSelection};
use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
use dfmcp_adapter::furniture_batch::store::open_private_batch;
use dfmcp_adapter::furniture_batch::{BatchDefinition, FurniturePlan};
use dfmcp_adapter::furniture_handoff::{FurnitureRequest, Handoff};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, Digest32, ErrorCode, GameTick, LeaseManager,
    ObservationCursor, OperationContext, RequestId, Result, RiskTier, SessionId, StateAnchor,
    WorkBudget,
};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{
    Mutex, MutexGuard, TryLockError,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};
mod allocation;
mod batch;
mod clock;
mod clock_recovery;
mod completion;
mod policy;
mod presentation;
mod room_clock;
mod room_intake;
mod room_monitor;
mod runtime;
mod runtime_room_clock;
use presentation::{OUTPUT_BYTES, digest, failure, inventory, packet};
use runtime::Config;

const FAMILY: u128 = 19u128 << 57;
const MAX_WORK_BYTES: u64 = 1024 * 1024 * 1024;
// Conservative admission reservations cover complete bounded journal replays,
// fsync/readback, one source lifecycle, and an intact final packet.
const VIEW_BYTES: u64 = 20 * 1024 * 1024;
const LOCAL_BYTES: u64 = 32 * 1024 * 1024;
const EFFECT_BYTES: u64 = 512 * 1024 * 1024;
const SOURCE_BYTES: u64 = 4 * 1024 * 1024;
static NEXT: AtomicU64 = AtomicU64::new(1);
static SESSION: Mutex<Option<Entry>> = Mutex::new(None);
// One process-wide session entry; variant size is irrelevant.
#[allow(clippy::large_enum_variant)]
enum Entry {
    Placement {
        state: State<PrivateBuildFile, BuildRpc>,
        config: Config,
    },
    Completion {
        state: completion::Recovery,
        config: Config,
    },
    ClockRecovery {
        state: clock_recovery::RecoveryState<
            PrivateBuildFile,
            dfmcp_adapter::bounded_run::private_file::PrivateRunFile,
        >,
        config: Config,
    },
}
impl Entry {
    fn id(&self) -> SessionId {
        match self {
            Self::Placement { state, .. } => state.id,
            Self::Completion { state, .. } => state.id,
            Self::ClockRecovery { state, .. } => state.id(),
        }
    }
}
fn error(code: ErrorCode, message: &str) -> dfmcp_core::DfmcpError {
    dfmcp_core::DfmcpError::new(code, message)
}
fn exhausted() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "complete furniture work and response exceed the allowance",
    )
}
fn unbound(op: &str, e: &dfmcp_core::DfmcpError) -> String {
    let mut result = failure(e);
    result["effect_may_have_occurred"] = json!(matches!(op, "fortress.commit" | "fortress.cancel"));
    packet(op, result, None, None, None, None, None, None, None)
}
fn key(raw: &str) -> Result<()> {
    if raw.is_empty()
        || raw.len() > 128
        || !raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "invalid furniture operation key",
        ));
    }
    Ok(())
}
fn parse_selection(raw: &str) -> Result<BuildSelection> {
    if raw.len() > 128 {
        return Err(error(
            ErrorCode::InvalidRequest,
            "furniture selection exceeds its bound",
        ));
    }
    let (kind, item, x, y, z): (String, u32, u32, u32, u32) =
        serde_json::from_str(raw).map_err(|_| {
            error(
                ErrorCode::InvalidRequest,
                "selection must be [kind,item_id,x,y,z]",
            )
        })?;
    let kind = match kind.as_str() {
        "bed" => BuildKind::Bed,
        "chair" => BuildKind::Chair,
        "table" => BuildKind::Table,
        _ => {
            return Err(error(
                ErrorCode::InvalidRequest,
                "only ordinary bed, chair, or table is supported",
            ));
        }
    };
    BuildSelection::new(kind, item, [x, y, z])
}
struct Review {
    key: String,
    plan: Digest32,
    witness: Digest32,
    seal: Digest32,
    head: Digest32,
}
struct State<S, N> {
    id: SessionId,
    request: u128,
    budget: WorkBudget,
    grants: Vec<CapabilityGrant>,
    control: BuildSession<S, N>,
    binding: BuildBinding,
    policy: policy::Policy,
    leases: LeaseManager,
    review: Option<Review>,
    historical: Option<BuildInventory>,
    pending_hint: Option<Value>,
    batch: Option<batch::Parent>,
    completion: Option<completion::Monitor>,
    room_completion: Option<room_monitor::Monitor>,
    room_recovery: Option<dfmcp_adapter::room_condition::store::Recovery>,
    clock: Option<
        dfmcp_adapter::construction_plan::clock::Clock<
            dfmcp_adapter::bounded_run::private_file::PrivateRunFile,
        >,
    >,
    clock_config: Option<clock::Config>,
    clock_recovery: Option<dfmcp_adapter::construction_plan::clock::Recovery>,
    clock_grants: Vec<CapabilityGrant>,
    clock_verified: bool,
    room_clock: Option<
        dfmcp_adapter::room_condition::clock::Clock<
            dfmcp_adapter::bounded_run::private_file::PrivateRunFile,
        >,
    >,
    room_clock_config: Option<room_clock::Config>,
    room_clock_recovery: Option<room_clock::Recovery>,
    room_clock_grants: Vec<CapabilityGrant>,
    room_clock_verified: bool,
    room_clock_bootstrap: bool,
    authority_tick: u64,
    worker_acknowledgement: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}
impl<S: EffectJournalStorage, N: BuildSource> State<S, N> {
    fn new(mut control: BuildSession<S, N>, c: &OperationContext, config: &Config) -> Result<Self> {
        let view = control.inventory(c)?;
        let binding = control.binding().clone();
        config.matches(&binding)?;
        let mut leases = LeaseManager::new();
        let mut current = c.clone();
        current.anchor.tick = GameTick(control.high_tick());
        let policy = policy::Policy::new(
            config.clone(),
            binding.clone(),
            view.journal_id,
            &current,
            &mut leases,
        )?;
        Ok(Self {
            id: c.session_id,
            request: c.request_id.get(),
            budget: c.budget,
            grants: c.grants.clone(),
            control,
            binding,
            policy,
            leases,
            review: None,
            historical: Some(view),
            pending_hint: None,
            batch: None,
            completion: None,
            room_completion: None,
            room_recovery: config.room_completion_recovery,
            clock: None,
            clock_config: config.clock.clone(),
            clock_recovery: config.clock.as_ref().and_then(|clock| clock.recovery),
            clock_grants: Vec::new(),
            clock_verified: false,
            room_clock: None,
            room_clock_config: config.room_clock.clone(),
            room_clock_recovery: config.room_clock.as_ref().and_then(|clock| clock.recovery),
            room_clock_grants: Vec::new(),
            room_clock_verified: false,
            room_clock_bootstrap: false,
            authority_tick: current.anchor.tick.get(),
            worker_acknowledgement: None,
        })
    }
    fn context(&mut self, write: bool, wall: Option<u64>) -> Result<OperationContext> {
        self.withdraw_unacknowledged();
        self.clock_verified = false;
        self.room_clock_verified = false;
        if let Some(monitor) = &mut self.completion {
            monitor.verified = false;
            monitor.store_verified = false;
        }
        if let Some(monitor) = &mut self.room_completion {
            monitor.verified = false;
            monitor.store_verified = false;
        }
        self.request = self.request.checked_add(1).ok_or_else(exhausted)?;
        let mut budget = self.budget;
        if let Some(w) = wall {
            budget.max_wall_millis = budget.max_wall_millis.min(w);
        }
        let mut c = context(
            self.id,
            RequestId::new(self.request),
            self.binding.fortress().fortress_id(),
            self.control
                .high_tick()
                .max(self.authority_tick)
                .max(self.room_floor())
                .max(
                    self.completion
                        .as_ref()
                        .and_then(|m| m.store.progress().last_tick)
                        .unwrap_or(0),
                ),
            budget,
            self.control.mode(),
            false,
        );
        c.grants = self
            .grants
            .iter()
            .filter(|g| write || !matches!(g.capability, Capability::Plan | Capability::Construct))
            .cloned()
            .collect();
        Ok(c)
    }
    fn room_floor(&self) -> u64 {
        self.room_completion
            .as_ref()
            .map_or(0, |m| m.store.authority_tick_floor())
    }
    fn abandon(&mut self) {
        self.review = None;
        self.control.abandon_preparation();
    }
    fn abandon_all(&mut self) {
        self.abandon();
        if let Some(clock) = &mut self.clock {
            clock.abandon_preparation();
        }
        if let Some(clock) = &mut self.room_clock {
            clock.abandon_preparation();
        }
    }
    fn withdraw_monitor(&mut self) {
        if let Some(monitor) = &mut self.room_completion {
            monitor.store.withdraw_unpublished();
            monitor.verified = false;
            monitor.store_verified = false;
        }
        if let Some(monitor) = &mut self.completion {
            monitor.store.withdraw_unpublished();
            monitor.verified = false;
            monitor.store_verified = false;
        }
    }
    fn withdraw_unacknowledged(&mut self) {
        if self
            .worker_acknowledgement
            .take()
            .is_some_and(|ack| !ack.load(Ordering::Acquire))
        {
            self.abandon_all();
            self.withdraw_monitor();
            self.clock_verified = false;
            self.room_clock_verified = false;
        }
    }
    fn seal(&self) -> Option<Digest32> {
        if self.control.has_preparation_connection() {
            self.review.as_ref().map(|v| v.seal)
        } else {
            None
        }
    }
    fn attach_batch(&self, result: &mut Value, view: Option<&BuildInventory>, verified: bool) {
        if let Some(parent) = &self.batch {
            result["batch"] = batch::display(parent, view, verified);
        }
        if let Some(monitor) = &self.completion {
            result["completion"] = completion::display(monitor, verified && monitor.verified);
            if clock::blocks_sampling(self) || room_clock::blocks_sampling(self) {
                result["completion"]["sampled_condition_satisfied"] = json!(false);
                result["completion"]["clock_history_blocks_sampling"] = json!(true);
            }
        }
        if let Some(monitor) = &self.room_completion {
            result["room_completion"] =
                room_monitor::display(monitor, verified && monitor.verified);
            let open = result["closed"] != true;
            result["room_completion"]["mode"] = json!(mode_name(self.control.mode()));
            result["room_completion"]["sampling_permitted"] = json!(
                open && self.control.mode() != BuildMode::Offline
                    && verified
                    && monitor.verified
                    && monitor.store_verified
                    && !monitor.store.is_fenced()
                    && !monitor.store.progress().terminal()
                    && monitor.store.progress().observations
                        < monitor.store.definition().goal().timing().max_observations
                    && self
                        .authority_tick
                        .max(self.control.high_tick())
                        .max(self.room_floor())
                        <= monitor.store.definition().goal().timing().deadline
                    && !(clock::blocks_sampling(self) || room_clock::blocks_sampling(self))
            );
            result["room_completion"]["local_cancel_permitted"] = json!(
                open && self.control.mode() != BuildMode::Offline
                    && monitor.store_verified
                    && !monitor.store.is_fenced()
                    && !monitor.store.progress().terminal()
            );
            if clock::blocks_sampling(self) || room_clock::blocks_sampling(self) {
                result["room_completion"]["sampled_condition_satisfied"] = json!(false);
                result["room_completion"]["clock_history_blocks_sampling"] = json!(true);
            }
        } else if let Some(pin) = self.room_recovery {
            result["room_completion"] = json!({"inventory_verified":false,"monitor_inventory_verified":false,
                "recovery":room_monitor::recovery(pin),"authority_tick_floor":self.authority_tick,
                "advancement_fenced":true,"sampled_condition_satisfied":false,
                "sampling_permitted":false,"local_cancel_permitted":false});
        }
        if result.get("construction_clock").is_none() {
            if let Some(clock) = &self.clock {
                result["construction_clock"] = clock::reference(clock);
                result["construction_clock"]["history_verified"] = json!(self.clock_verified);
            } else if self.clock_config.is_some() || self.clock_recovery.is_some() {
                result["construction_clock"] = json!({"history_verified":false,
                    "configured":self.clock_config.is_some(),"pending":true,
                    "recovery":self.clock_recovery.map(clock::recovery),
                    "fresh_goal_sampling_permitted":false,"retry_commit_permitted":false});
            }
        }
        if result.get("room_clock").is_none() {
            if let Some(owner) = &self.room_clock {
                result["room_clock"] = room_clock::reference(owner);
                result["room_clock"]["history_verified"] = json!(self.room_clock_verified);
            } else if self.room_clock_config.is_some() || self.room_clock_recovery.is_some() {
                let bootstrap = room_clock::bootstrap_pending(self);
                result["room_clock"] = json!({"history_verified":false,"configured":self.room_clock_config.is_some(),
                    "monitor_id":self.room_completion.as_ref().map(|m|m.store.definition().id().to_string()),
                    "pending":!bootstrap,"bootstrap_sample_required":bootstrap && self.room_completion.as_ref().is_none_or(|m|m.store.first_source().is_none()),
                    "initial_plan_required":bootstrap && self.room_completion.as_ref().is_some_and(|m|m.store.first_source().is_some()),
                    "recovery":self.room_clock_recovery.map(room_clock::recovery),
                    "fresh_goal_sampling_permitted":false,"retry_commit_permitted":false,
                    "sampled_joint_room_condition_proven":false});
            }
        }
        for name in ["construction_clock", "room_clock"] {
            if let Some(clock) = result.get_mut(name).filter(|v| v.is_object()) {
                // A decoded readiness capture can raise the shared floor before a
                // later original-custody failure prevents the clock owner seeing it.
                // Every handoff, including an already-rendered history page, retains
                // that consumed evidence without claiming a new successful sample.
                let floor = self
                    .authority_tick
                    .max(self.control.high_tick())
                    .max(self.room_floor())
                    .max(self.clock.as_ref().map_or(0, |owner| owner.high_tick()))
                    .max(
                        self.room_clock
                            .as_ref()
                            .map_or(0, |owner| owner.high_tick()),
                    )
                    .max(
                        self.completion
                            .as_ref()
                            .and_then(|monitor| monitor.store.progress().last_tick)
                            .map_or(0, |tick| tick),
                    )
                    .max(
                        clock
                            .get("authority_tick_floor")
                            .and_then(Value::as_u64)
                            .map_or(0, |tick| tick),
                    );
                clock["authority_tick_floor"] = json!(floor);
            }
        }
    }
    fn verify_batch(&mut self, c: &OperationContext, view: &BuildInventory) -> Result<()> {
        if let Some(parent) = &mut self.batch {
            batch::verify(parent, view, c)?;
        }
        Ok(())
    }
    fn disclosure_context(&self, c: &OperationContext) -> Result<OperationContext> {
        let mut current = c.clone();
        current.anchor.tick = GameTick(
            current
                .anchor
                .tick
                .get()
                .max(self.control.high_tick())
                .max(self.authority_tick)
                .max(self.room_floor())
                .max(
                    self.completion
                        .as_ref()
                        .and_then(|m| m.store.progress().last_tick)
                        .unwrap_or(0),
                ),
        );
        if current.session_id != self.id
            || current.anchor.fortress_id != self.binding.fortress().fortress_id()
        {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "furniture evidence belongs to another session or fortress",
            ));
        }
        // Cached evidence has the same disclosure boundary as a fresh journal
        // read. A failed custody or budget check cannot revive expired Query.
        current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if self.clock_config.is_some() || self.clock.is_some() || self.clock_recovery.is_some() {
            let native = clock::native_context(&current, &self.clock_grants)?;
            if let Some(owner) = &self.clock {
                owner.authorize_disclosure(&current, &native)?;
            }
        }
        if self.room_clock_config.is_some()
            || self.room_clock.is_some()
            || self.room_clock_recovery.is_some()
        {
            let native = clock::native_context(&current, &self.room_clock_grants)?;
            if let Some(owner) = &self.room_clock {
                owner.authorize_disclosure(&current, &native)?;
            }
        }
        Ok(current)
    }
    fn failed(
        &mut self,
        op: &str,
        c: Option<&OperationContext>,
        e: &dfmcp_core::DfmcpError,
    ) -> String {
        self.abandon_all();
        self.withdraw_monitor();
        self.clock_verified = false;
        self.room_clock_verified = false;
        let current = match c.map(|c| self.disclosure_context(c)) {
            Some(Ok(c)) => c,
            Some(Err(denied)) => return unbound(op, &denied),
            None => return unbound(op, e),
        };
        let mut result = failure(e);
        result["effect_may_have_occurred"] =
            json!(matches!(op, "fortress.commit" | "fortress.cancel"));
        result["pending_identity_hint"] = self.pending_hint.clone().unwrap_or(Value::Null);
        result["source_summary"] = self
            .control
            .native_summary()
            .map(presentation::source_summary)
            .unwrap_or(Value::Null);
        self.attach_batch(&mut result, self.historical.as_ref(), false);
        packet(
            op,
            result,
            Some(&current),
            Some(&self.binding),
            None,
            self.historical.as_ref(),
            Some(&self.policy),
            None,
            None,
        )
    }
    fn close_packet(
        &self,
        c: &OperationContext,
        view: Option<&BuildInventory>,
        release: bool,
    ) -> String {
        let mut result = json!({"ok":true,"scope":"session","closed":true,"release_for_recovery":release,
            "effects_cancelled":false,"history_erased":false,"native_quiescence_proven":false});
        match self.disclosure_context(c) {
            Ok(current) => {
                self.attach_batch(&mut result, view.or(self.historical.as_ref()), false);
                packet(
                    "fortress.cancel",
                    result,
                    Some(&current),
                    Some(&self.binding),
                    view,
                    self.historical.as_ref(),
                    None,
                    None,
                    None,
                )
            }
            // Explicit recovery release may relinquish owned custody even
            // after Query revocation. It publishes no retained fortress facts.
            Err(_) => packet(
                "fortress.cancel",
                result,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ),
        }
    }
}
fn grants(fortress: dfmcp_core::FortressId, mode: BuildMode, write: bool) -> Vec<CapabilityGrant> {
    [
        Capability::Query,
        Capability::Observe,
        Capability::Plan,
        Capability::Construct,
    ]
    .into_iter()
    .filter(|v| {
        *v == Capability::Query
            || (mode == BuildMode::Control && (*v == Capability::Observe || write))
    })
    .map(|capability| CapabilityGrant {
        capability,
        scope: CapabilityScope {
            fortress_id: Some(fortress),
            ..CapabilityScope::default()
        },
        max_risk: if matches!(capability, Capability::Plan | Capability::Construct) {
            RiskTier::Guarded
        } else {
            RiskTier::ReadOnly
        },
        expires_at_tick: None,
        remaining_uses: None,
    })
    .collect()
}
fn add_clock_grants(c: &mut OperationContext, config: &Config, enabled: bool) {
    if (config.clock.is_none() && config.room_clock.is_none())
        || config.mode != BuildMode::Control
        || !enabled
    {
        return;
    }
    for capability in [Capability::Plan, Capability::ControlClock] {
        if !c.grants.iter().any(|grant| grant.capability == capability) {
            c.grants.push(CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(config.fortress.fortress_id()),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::Guarded,
                expires_at_tick: None,
                remaining_uses: None,
            });
        }
    }
}
fn clock_recovery_authority(
    config: &Config,
    enabled: bool,
) -> (Vec<CapabilityGrant>, Vec<CapabilityGrant>) {
    let mut canonical = grants(config.fortress.fortress_id(), BuildMode::Recover, false);
    let control = config.clock_only && config.mode == BuildMode::Recover && enabled;
    if control {
        canonical.push(CapabilityGrant {
            capability: Capability::ControlClock,
            scope: CapabilityScope {
                fortress_id: Some(config.fortress.fortress_id()),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::Guarded,
            expires_at_tick: None,
            remaining_uses: None,
        });
    }
    let native = runtime::clock_grants(control)
        .into_iter()
        .filter(|grant| grant.capability != Capability::Plan)
        .collect();
    (canonical, native)
}
fn context(
    id: SessionId,
    request_id: RequestId,
    fortress: dfmcp_core::FortressId,
    tick: u64,
    budget: WorkBudget,
    mode: BuildMode,
    write: bool,
) -> OperationContext {
    OperationContext {
        session_id: id,
        request_id,
        budget,
        cancellation_requested: false,
        anchor: StateAnchor {
            fortress_id: fortress,
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(tick),
            state_hash: Digest32::ZERO,
        },
        grants: grants(fortress, mode, write),
    }
}
struct Work {
    deadline: Instant,
    bytes: u64,
}
impl Work {
    fn new(c: &OperationContext, started: Instant) -> Result<Self> {
        c.budget.validate()?;
        if c.budget.max_wall_millis > 60000
            || c.budget.max_game_ticks > 1200
            || c.budget.max_bytes > MAX_WORK_BYTES
            || c.budget.max_output_tokens > 65536
            || u64::from(c.budget.max_output_tokens) * 4 < OUTPUT_BYTES
        {
            return Err(exhausted());
        }
        let out = Self {
            deadline: started
                .checked_add(Duration::from_millis(c.budget.max_wall_millis))
                .ok_or_else(exhausted)?,
            bytes: c
                .budget
                .max_bytes
                .checked_sub(OUTPUT_BYTES + 2 * VIEW_BYTES)
                .ok_or_else(exhausted)?,
        };
        out.current(c)?;
        Ok(out)
    }
    fn current(&self, c: &OperationContext) -> Result<OperationContext> {
        let remaining = self
            .deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(exhausted)?
            .as_millis();
        if remaining == 0 {
            return Err(exhausted());
        }
        let mut out = c.clone();
        out.budget.max_wall_millis = u64::try_from(remaining).map_err(|_| exhausted())?;
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
}
#[derive(Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
enum Query {
    Records {
        limit: Option<usize>,
        offset: Option<usize>,
        head: Option<String>,
    },
    Get {
        idempotency_key: String,
        plan_digest: String,
    },
    Selection {
        witness: String,
    },
    Schema {},
    Batch {},
    Allocation {
        #[serde(default)]
        view: allocation::View,
    },
    Completion {},
    RoomCompletion {},
    RoomCompletionStart {
        deadline: u64,
        interval: Option<u32>,
        stable_samples: Option<u32>,
        stable_span: Option<u64>,
        max_gap: Option<u32>,
        max_observations: Option<u32>,
    },
    ConstructionClock {
        monitor_id: String,
        head: String,
        offset: Option<usize>,
        limit: Option<usize>,
    },
    RoomClock {
        monitor_id: String,
        head: String,
        offset: Option<usize>,
        limit: Option<usize>,
    },
    CompletionStart {
        deadline: u64,
        interval: Option<u32>,
        stable_samples: Option<u32>,
        stable_span: Option<u64>,
        max_gap: Option<u32>,
        max_observations: Option<u32>,
    },
}
impl Query {
    fn parse(raw: &str) -> Result<Self> {
        if raw.len() > 2048 {
            return Err(error(
                ErrorCode::InvalidRequest,
                "furniture query exceeds 2 KiB",
            ));
        }
        let shape: Value = serde_json::from_str(raw)
            .map_err(|_| error(ErrorCode::InvalidRequest, "invalid furniture query"))?;
        if shape
            .as_object()
            .is_some_and(|m| m.values().any(Value::is_null))
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "omit absent query fields instead of null",
            ));
        }
        // Parse original bytes, so duplicate fields cannot hide behind a Value map.
        let value: Self = serde_json::from_str(raw)
            .map_err(|_| error(ErrorCode::InvalidRequest, "invalid closed furniture query"))?;
        match &value {
            Self::Records {
                limit,
                offset,
                head,
            } => {
                if !(1..=8).contains(&limit.unwrap_or(8))
                    || offset.unwrap_or(0) > 256
                    || (offset.unwrap_or(0) > 0 && head.is_none())
                {
                    return Err(error(
                        ErrorCode::InvalidRequest,
                        "invalid furniture record page",
                    ));
                }
                if let Some(h) = head {
                    digest(h)?;
                }
            }
            Self::Get {
                idempotency_key,
                plan_digest,
            } => {
                key(idempotency_key)?;
                digest(plan_digest)?;
            }
            Self::Selection { witness } => {
                digest(witness)?;
            }
            Self::ConstructionClock {
                monitor_id,
                head,
                offset,
                limit,
            }
            | Self::RoomClock {
                monitor_id,
                head,
                offset,
                limit,
            } => {
                if digest(monitor_id)? == Digest32::ZERO
                    || digest(head)? == Digest32::ZERO
                    || offset.unwrap_or(0) >= 64
                    || !(1..=8).contains(&limit.unwrap_or(8))
                {
                    return Err(error(
                        ErrorCode::InvalidRequest,
                        "invalid construction clock page",
                    ));
                }
            }
            Self::Schema {}
            | Self::Batch {}
            | Self::Allocation { .. }
            | Self::Completion {}
            | Self::RoomCompletion {}
            | Self::RoomCompletionStart { .. }
            | Self::CompletionStart { .. } => {}
        }
        Ok(value)
    }
}
enum Action {
    Observe(BuildSelection),
    ObserveNext,
    Plan {
        key: String,
        witness: Digest32,
    },
    Commit {
        key: String,
        plan: Digest32,
        seal: Digest32,
    },
    Recover(String, Digest32, bool),
    Explain(String, Digest32),
    Query(Query),
    Inventory,
    StopBatch,
    Completion(completion::Action),
    RoomCompletion(room_monitor::Action),
    Clock(clock::Action),
    RoomClock(room_clock::Action),
    Denied,
}
fn schema() -> Value {
    json!({"schema":"dfmcp.build-placement-mcp-query/1","max_bytes":2048,"closed":true,
    "modes":{"records":{"limit":"optional integer 1..8","offset":"optional integer 0..256; head required when nonzero","head":"optional lowercase SHA-256 exact journal head"},
        "get":{"idempotency_key":"1..128 ASCII letters/digits/dot/underscore/hyphen","plan_digest":"lowercase SHA-256"},"selection":{"witness":"lowercase SHA-256"},"batch":{},"completion":{},"room_completion":{},
        "allocation":{"view":"optional request (default) or items; retained original evidence only"},
        "room_clock":{"monitor_id":"required original joint room monitor SHA-256","head":"required exact room clock journal head SHA-256","offset":"optional 0..63","limit":"optional 1..8"},
        "construction_clock":{"monitor_id":"required original completion monitor SHA-256","head":"required exact clock journal head SHA-256","offset":"optional 0..63","limit":"optional 1..8"},
        "room_completion_start":{"deadline":"required absolute game tick","interval":"optional 1..403200; default 1","stable_samples":"optional 2..64; default 2","stable_span":"optional 1..4032000; default 1","max_gap":"optional interval..4032000; default 1200","max_observations":"optional stable_samples..512; default 512"},
        "completion_start":{"deadline":"required absolute game tick","interval":"optional 1..403200; default 1","stable_samples":"optional 2..64; default 2","stable_span":"optional 1..4032000; default 1","max_gap":"optional interval..4032000; default 1200","max_observations":"optional stable_samples..512; default 512"},"schema":{}},
    "native_calls":0,"null_fields_allowed":false,"duplicate_fields_allowed":false})
}
#[allow(clippy::too_many_arguments)]
fn perform<S, N, G, F>(
    state: &mut State<S, N>,
    c: &OperationContext,
    work: &mut Work,
    before: &BuildInventory,
    action: Action,
    runtime: &mut G,
    connect: F,
) -> Result<Value>
where
    S: EffectJournalStorage,
    N: BuildSource,
    G: BuildGuard,
    F: FnOnce(BuildSelection, bool, &BuildBinding, &OperationContext, Duration) -> Result<N>,
{
    match action {
        Action::Observe(_) | Action::ObserveNext => {
            state.abandon();
            if state.batch.is_some() {
                state.verify_batch(&work.take(c, batch::GUARD_BYTES)?, before)?;
            }
            let next = state
                .batch
                .as_ref()
                .map(|p| batch::next(p, before))
                .transpose()?;
            let selection = match action {
                Action::Observe(selection) => {
                    if next
                        .as_ref()
                        .is_some_and(|(_, expected)| *expected != selection)
                    {
                        return Err(error(
                            ErrorCode::Conflict,
                            "selection differs from the original next batch step",
                        ));
                    }
                    selection
                }
                _ => next
                    .as_ref()
                    .map(|(_, selection)| *selection)
                    .ok_or_else(|| {
                        error(
                            ErrorCode::InvalidRequest,
                            "next requires an original furniture batch",
                        )
                    })?,
            };
            let guard_bytes = if state.batch.is_some() {
                work.take(c, batch::GUARD_BYTES)?.budget.max_bytes
            } else {
                0
            };
            let mut custody = batch::Guard {
                parent: &mut state.batch,
                view: before,
                key: next.as_ref().map(|(key, _)| key.as_str()),
                runtime,
                bytes: guard_bytes,
            };
            let mut guard = policy::Guard {
                policy: &state.policy,
                leases: &state.leases,
                runtime: &mut custody,
                confirmation: None,
            };
            let capture = state.control.observe(
                selection,
                &work.take(c, EFFECT_BYTES)?,
                |b, c, d| connect(selection, false, b, c, d),
                &mut guard,
            )?;
            if let (Some(parent), Some((key, _))) = (&state.batch, next) {
                parent.definition().validate_next(
                    before,
                    parent.stopped(),
                    &key,
                    selection,
                    Some(&capture),
                )?;
            }
            Ok(
                json!({"ok":true,"observation":presentation::observation(&capture),"game_mutation_dispatched":false,
                    "source_summary":state.control.native_summary().map(presentation::source_summary)}),
            )
        }
        Action::Plan { key, witness } => {
            if state.batch.is_some() {
                state.verify_batch(&work.take(c, batch::GUARD_BYTES)?, before)?;
            }
            if let Some(review) = &state.review {
                if review.key != key || review.witness != witness {
                    return Err(error(
                        ErrorCode::Conflict,
                        "another exact furniture review is pending",
                    ));
                }
                let old = state
                    .control
                    .get(&key, review.plan, &work.take(c, LOCAL_BYTES)?)?;
                state.policy.evaluate(old.plan(), c, &state.leases)?;
                if let Some(parent) = &state.batch {
                    parent.definition().validate_next(
                        before,
                        parent.stopped(),
                        &key,
                        old.plan().before().selection(),
                        Some(old.plan().before()),
                    )?;
                    if review.head != before.head {
                        return Err(error(
                            ErrorCode::StaleAnchor,
                            "reviewed batch journal head changed",
                        ));
                    }
                }
                return Ok(
                    json!({"ok":true,"plan":presentation::record(&old),"plan_digest":review.plan.to_string(),"review_seal":review.seal.to_string(),"replayed_locally":true}),
                );
            }
            let capture = state
                .control
                .selected()
                .filter(|v| v.witness() == witness)
                .ok_or_else(|| {
                    error(
                        ErrorCode::StaleAnchor,
                        "planning requires this session's exact selected observation",
                    )
                })?;
            let plan = BuildPlan::new(&key, capture.clone())?;
            state.policy.evaluate(&plan, c, &state.leases)?;
            if let Some(parent) = &state.batch {
                parent.definition().validate_next(
                    before,
                    parent.stopped(),
                    &key,
                    plan.before().selection(),
                    Some(plan.before()),
                )?;
            }
            state.pending_hint = Some(
                json!({"idempotency_key":key,"plan_digest":plan.digest().to_string(),"outcome":"unverified","retry_commit_permitted":false}),
            );
            let guard_bytes = if state.batch.is_some() {
                work.take(c, batch::GUARD_BYTES)?.budget.max_bytes
            } else {
                0
            };
            let mut custody = batch::Guard {
                parent: &mut state.batch,
                view: before,
                key: Some(&key),
                runtime,
                bytes: guard_bytes,
            };
            let mut guard = policy::Guard {
                policy: &state.policy,
                leases: &state.leases,
                runtime: &mut custody,
                confirmation: None,
            };
            let entry =
                state
                    .control
                    .prepare(&key, witness, &work.take(c, EFFECT_BYTES)?, &mut guard)?;
            if state.control.has_preparation_connection() {
                let head = if state.batch.is_some() {
                    let view = state.control.inventory(&work.take(c, LOCAL_BYTES)?)?;
                    state.verify_batch(&work.take(c, batch::GUARD_BYTES)?, &view)?;
                    view.head
                } else {
                    before.head
                };
                let policy_seal = state.policy.seal(entry.plan());
                let seal = state.batch.as_ref().map_or(policy_seal, |p| {
                    batch::seal(p, entry.plan(), head, policy_seal)
                });
                state.review = Some(Review {
                    key,
                    plan: entry.plan().digest(),
                    witness,
                    seal,
                    head,
                });
            }
            Ok(
                json!({"ok":true,"plan":presentation::record(&entry),"plan_digest":entry.plan().digest().to_string(),"review_seal":state.seal().map(|s|s.to_string()),"game_mutation_dispatched":false}),
            )
        }
        Action::Commit { key, plan, seal } => {
            let old = state.control.get(&key, plan, &work.take(c, LOCAL_BYTES)?)?;
            if !old.unresolved() {
                return Ok(
                    json!({"ok":true,"effect":presentation::record(&old),"native_calls":0,"historical_replay":true}),
                );
            }
            if state.batch.is_some() {
                state.verify_batch(&work.take(c, batch::GUARD_BYTES)?, before)?;
            }
            if !state
                .review
                .as_ref()
                .is_some_and(|r| r.key == key && r.plan == plan && r.seal == seal)
            {
                return Err(error(
                    ErrorCode::CapabilityDenied,
                    "exact local furniture review must be confirmed",
                ));
            }
            state
                .policy
                .evaluate(old.plan(), &work.current(c)?, &state.leases)?;
            let review = state.review.take().ok_or_else(|| {
                error(ErrorCode::CapabilityDenied, "furniture review unavailable")
            })?;
            let policy_seal = state.policy.seal(old.plan());
            if let Some(parent) = &state.batch {
                if review.head != before.head
                    || review.seal != batch::seal(parent, old.plan(), before.head, policy_seal)
                {
                    return Err(error(
                        ErrorCode::StaleAnchor,
                        "complete batch or prepared journal review changed",
                    ));
                }
                parent.definition().validate_next(
                    before,
                    parent.stopped(),
                    &key,
                    old.plan().before().selection(),
                    Some(old.plan().before()),
                )?;
            }
            let guard_bytes = if state.batch.is_some() {
                work.take(c, batch::GUARD_BYTES)?.budget.max_bytes
            } else {
                0
            };
            let mut custody = batch::Guard {
                parent: &mut state.batch,
                view: before,
                key: Some(&key),
                runtime,
                bytes: guard_bytes,
            };
            let mut guard = policy::Guard {
                policy: &state.policy,
                leases: &state.leases,
                runtime: &mut custody,
                confirmation: Some(policy_seal),
            };
            let result = state
                .control
                .commit(&key, plan, &work.take(c, EFFECT_BYTES)?, &mut guard);
            state.abandon();
            Ok(
                json!({"ok":true,"effect":presentation::record(&result?),"building_completion_proven":false,"retry_commit_permitted":false}),
            )
        }
        Action::Recover(key, plan, cancel) => {
            state.abandon();
            let old = state.control.get(&key, plan, &work.take(c, LOCAL_BYTES)?)?;
            let receipt_reconciliation = !cancel && old.can_reconcile_receipt();
            let selection = old.plan().before().selection();
            let mut guard = policy::Guard {
                policy: &state.policy,
                leases: &state.leases,
                runtime,
                confirmation: None,
            };
            let entry = state.control.recover(
                &key,
                plan,
                cancel,
                &work.take(c, EFFECT_BYTES)?,
                |b, c, d| connect(selection, true, b, c, d),
                &mut guard,
            )?;
            Ok(json!({"ok":true,"effect":presentation::record(&entry),
                    "receipt_reconciled":receipt_reconciliation && !entry.unresolved(),
                    "commit_retried":false,"building_undone":false}))
        }
        Action::Explain(key, plan) => {
            let entry = state.control.get(&key, plan, &work.take(c, LOCAL_BYTES)?)?;
            Ok(json!({"ok":true,"record":presentation::record(&entry),"native_calls":0}))
        }
        Action::Query(Query::Get {
            idempotency_key,
            plan_digest,
        }) => {
            let entry = state.control.get(
                &idempotency_key,
                digest(&plan_digest)?,
                &work.take(c, LOCAL_BYTES)?,
            )?;
            Ok(json!({"ok":true,"record":presentation::record(&entry),"native_calls":0}))
        }
        Action::Query(Query::Records {
            limit,
            offset,
            head,
        }) => {
            if let Some(head) = head
                && digest(&head)? != before.head
            {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "furniture journal continuation head changed",
                ));
            }
            let limit = limit.unwrap_or(8);
            let offset = offset.unwrap_or(0);
            let mut ordered = before.entries().iter().collect::<Vec<_>>();
            ordered.sort_by_key(|e| (!e.unresolved(), e.plan().key()));
            if offset > ordered.len() {
                return Err(error(
                    ErrorCode::InvalidRequest,
                    "record offset exceeds exact journal inventory",
                ));
            }
            let end = (offset + limit).min(ordered.len());
            let rows = ordered[offset..end]
                .iter()
                .map(|e| presentation::summary(e))
                .collect::<Vec<_>>();
            Ok(
                json!({"ok":true,"records":rows,"journal":inventory(before),"offset":offset,"next_query":if end<ordered.len(){json!({"mode":"records","limit":limit,"offset":end,"head":before.head.to_string()})}else{Value::Null},"native_calls":0}),
            )
        }
        Action::Query(Query::Selection { witness }) => {
            let witness = digest(&witness)?;
            let capture = state
                .control
                .selected()
                .filter(|v| v.witness() == witness)
                .ok_or_else(|| {
                    error(
                        ErrorCode::StaleAnchor,
                        "selected furniture observation is no longer retained",
                    )
                })?;
            Ok(json!({"ok":true,"observation":presentation::observation(capture),"native_calls":0}))
        }
        Action::Query(Query::Schema {}) => {
            Ok(json!({"ok":true,"query_schema":schema(),"native_calls":0}))
        }
        Action::Query(Query::Batch {}) => {
            if state.batch.is_none() {
                return Err(error(
                    ErrorCode::InvalidRequest,
                    "no furniture batch is configured",
                ));
            }
            state.verify_batch(&work.take(c, batch::GUARD_BYTES)?, before)?;
            Ok(json!({"ok":true,"native_calls":0}))
        }
        Action::Query(Query::Allocation { view }) => {
            state.verify_batch(&work.take(c, batch::GUARD_BYTES)?, before)?;
            let handoff = state
                .batch
                .as_ref()
                .and_then(|p| p.definition().handoff())
                .ok_or_else(|| {
                    error(
                        ErrorCode::InvalidRequest,
                        "this batch has no allocation origin",
                    )
                })?;
            Ok(json!({
                "ok":true,
                "allocation":allocation::display(handoff, view)?,
                "native_calls":0
            }))
        }
        Action::StopBatch => {
            state.abandon();
            let parent = state.batch.as_mut().ok_or_else(|| {
                error(
                    ErrorCode::InvalidRequest,
                    "no furniture batch is configured",
                )
            })?;
            batch::verify(parent, before, &work.take(c, batch::GUARD_BYTES)?)?;
            parent.stop(&work.take(c, batch::GUARD_BYTES)?)?;
            Ok(
                json!({"ok":true,"scope":"batch","stopped":true,"native_calls":0,
                "effects_cancelled":false,"building_undone":false,"original_key_recovery_preserved":true}),
            )
        }
        Action::Inventory => Ok(
            json!({"ok":true,"journal":inventory(before),"native_calls":0,"live_game_health_checked":false}),
        ),
        Action::Completion(_)
        | Action::RoomCompletion(_)
        | Action::Clock(_)
        | Action::RoomClock(_)
        | Action::Query(
            Query::Completion {}
            | Query::CompletionStart { .. }
            | Query::RoomCompletion {}
            | Query::RoomCompletionStart { .. }
            | Query::ConstructionClock { .. }
            | Query::RoomClock { .. },
        ) => Err(error(
            ErrorCode::InvalidRequest,
            "completion requires the original private batch session",
        )),
        Action::Denied => Err(error(
            ErrorCode::CapabilityDenied,
            "furniture journal is not a game checkpoint or restore",
        )),
    }
}
#[allow(clippy::too_many_arguments)]
fn run_action<S, N, G, F>(
    state: &mut State<S, N>,
    c: OperationContext,
    op: &str,
    action: Result<Action>,
    started: Instant,
    runtime: &mut G,
    connect: F,
) -> String
where
    S: EffectJournalStorage,
    N: BuildSource,
    G: BuildGuard,
    F: FnOnce(BuildSelection, bool, &BuildBinding, &OperationContext, Duration) -> Result<N>,
{
    if let Some(clock) = &mut state.clock {
        clock.abandon_preparation();
    }
    if let Some(clock) = &mut state.room_clock {
        clock.abandon_preparation();
    }
    if op == "fortress.observe" {
        state.abandon();
    }
    let mut work = match Work::new(&c, started) {
        Ok(v) => v,
        Err(e) => return state.failed(op, Some(&c), &e),
    };
    let before = match work.view(&c).and_then(|c| state.control.inventory(&c)) {
        Ok(v) => v,
        Err(e) => {
            state.abandon();
            return state.failed(op, Some(&c), &e);
        }
    };
    state.historical = Some(before.clone());
    let outcome = (|| {
        perform(
            state,
            &work.current(&c)?,
            &mut work,
            &before,
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
    let display = match state.disclosure_context(&c) {
        Ok(current) => current,
        Err(e) => {
            state.abandon();
            return unbound(op, &e);
        }
    };
    let after = work
        .view(&display)
        .and_then(|c| state.control.inventory(&c));
    let (mut result, verified) = match after {
        Ok(v) => {
            state.historical = Some(v.clone());
            state.pending_hint = None;
            (outcome.unwrap_or_else(|e| failure(&e)), Some(v))
        }
        Err(e) => {
            state.abandon();
            let mut f = failure(&e);
            f["post_operation_inventory_unverified"] = json!(true);
            f["pending_identity_hint"] = state.pending_hint.clone().unwrap_or(Value::Null);
            (f, None)
        }
    };
    if result["ok"] != true {
        result["effect_may_have_occurred"] =
            json!(matches!(op, "fortress.commit" | "fortress.cancel"));
    }
    result["source_summary"] = state
        .control
        .native_summary()
        .map(presentation::source_summary)
        .unwrap_or(Value::Null);
    if op == "fortress.plan"
        && result["error"]["code"] == ErrorCode::EffectIndeterminate.as_str()
        && verified
            .as_ref()
            .is_some_and(|v| v.head == before.head && v.pending().is_none())
        && state
            .control
            .native_summary()
            .is_some_and(|s| s.unresolved())
    {
        result["error"]["message"] = json!(
            "Retained native history blocks new preparation. No new local obligation was created; preserve and reconcile the journal that owns the unresolved native work."
        );
        result["new_local_obligation_created"] = json!(false);
        result["recovery_target"] = json!("journal_owning_unresolved_native_history");
    }
    let batch_verified = if state.batch.is_some() {
        match verified
            .as_ref()
            .ok_or_else(|| error(ErrorCode::CorruptLedger, "child inventory unavailable"))
            .and_then(|v| state.verify_batch(&work.view(&display)?, v))
        {
            Ok(()) => true,
            Err(e) => {
                state.abandon();
                result["batch_inventory_unverified"] = json!(true);
                if matches!(
                    op,
                    "fortress.observe" | "fortress.plan" | "fortress.commit" | "fortress.doctor"
                ) {
                    result["ok"] = json!(false);
                    result["error"] = failure(&e)["error"].clone();
                    result["review_seal"] = Value::Null;
                    result["retry_commit_permitted"] = json!(false);
                    result["effect_may_have_occurred"] = json!(op == "fortress.commit");
                }
                false
            }
        }
    } else {
        false
    };
    if let Some(allocation) = result.get_mut("allocation") {
        allocation["inventory_verified"] = json!(batch_verified);
    }
    if state.completion.is_some() {
        let checked = verified
            .as_ref()
            .ok_or_else(|| error(ErrorCode::CorruptLedger, "original inventory unavailable"))
            .and_then(|view| completion::verify(state, &display, &mut work, view, batch_verified));
        if let Err(e) = checked {
            result["completion_inventory_unverified"] = json!(true);
            if op == "fortress.doctor" {
                result["ok"] = json!(false);
                result["error"] = failure(&e)["error"].clone();
            }
        }
    }
    if state.room_completion.is_some() {
        let checked = verified
            .as_ref()
            .ok_or_else(|| {
                error(
                    ErrorCode::CorruptLedger,
                    "original room inventory unavailable",
                )
            })
            .and_then(|view| {
                room_monitor::verify(state, &display, &mut work, view, batch_verified)
            });
        if let Err(e) = checked {
            result["room_completion_inventory_unverified"] = json!(true);
            if op == "fortress.doctor" {
                result["ok"] = json!(false);
                result["error"] = failure(&e)["error"].clone();
            }
        }
    }
    if state.clock_config.is_some() || state.clock.is_some() {
        let checked = work
            .take(&display, clock::VIEW_BYTES)
            .and_then(|context| clock::inspect(state, &context));
        if let Err(e) = checked {
            state.withdraw_monitor();
            result["clock_inventory_unverified"] = json!(true);
            if op == "fortress.doctor" {
                result["ok"] = json!(false);
                result["error"] = failure(&e)["error"].clone();
            }
        }
    }
    if state.room_clock_config.is_some() || state.room_clock.is_some() {
        let checked = work
            .take(&display, room_clock::VIEW_BYTES)
            .and_then(|c| room_clock::inspect(state, &c));
        if let Err(e) = checked {
            state.withdraw_monitor();
            result["room_clock_inventory_unverified"] = json!(true);
            if op == "fortress.doctor" {
                result["ok"] = json!(false);
                result["error"] = failure(&e)["error"].clone();
            }
        }
    }
    state.attach_batch(
        &mut result,
        verified.as_ref().or(state.historical.as_ref()),
        batch_verified,
    );
    let rendered = packet(
        op,
        result,
        Some(&display),
        Some(&state.binding),
        verified.as_ref(),
        state.historical.as_ref(),
        Some(&state.policy),
        state.seal(),
        state.control.selected(),
    );
    if rendered.len() as u64 > OUTPUT_BYTES || work.current(&display).is_err() {
        state.abandon();
        return state.failed(
            op,
            Some(&display),
            &error(
                ErrorCode::EffectIndeterminate,
                "complete response unavailable; inspect original journal",
            ),
        );
    }
    rendered
}
fn lock() -> Result<MutexGuard<'static, Option<Entry>>> {
    match SESSION.try_lock() {
        Ok(v) => Ok(v),
        Err(TryLockError::WouldBlock) => Err(error(
            ErrorCode::Conflict,
            "furniture session already serving a request",
        )),
        Err(TryLockError::Poisoned(_)) => Err(error(
            ErrorCode::InternalInvariantViolation,
            "furniture session poisoned; restart for recovery",
        )),
    }
}
/// The caller reserves this replay before any effect. No runtime callback may
/// follow it inside the worker and still publish the previously rendered view.
fn verify_clock_handoff<S: EffectJournalStorage, N: BuildSource>(
    state: &mut State<S, N>,
    c: &OperationContext,
    started: Instant,
    expected: Option<dfmcp_adapter::construction_plan::clock::Recovery>,
) -> Result<()> {
    let mut current = state.disclosure_context(c)?;
    current.budget.max_bytes = clock::VIEW_BYTES;
    current.budget.max_wall_millis = c
        .budget
        .max_wall_millis
        .checked_sub(started.elapsed().as_millis().try_into().unwrap_or(u64::MAX))
        .filter(|remaining| *remaining > 0)
        .ok_or_else(exhausted)?;
    clock::inspect(state, &current)?;
    if state.clock_recovery != expected {
        return Err(error(
            ErrorCode::CorruptLedger,
            "construction clock changed at final response publication",
        ));
    }
    state.disclosure_context(&current)?;
    Ok(())
}

/// Opening publishes one exact original inventory, monitor and clock together.
/// These independent replays are prepaid before the clock file can be opened or
/// initialized, and retain the opening request's original absolute deadline.
struct OpeningPublication {
    deadline: Instant,
    source: OperationContext,
    monitor: OperationContext,
    clock: OperationContext,
    monitor_head: Digest32,
}
impl OpeningPublication {
    fn reserve<S: EffectJournalStorage, N: BuildSource>(
        state: &State<S, N>,
        c: &OperationContext,
        work: &mut Work,
        clock: OperationContext,
    ) -> Result<Self> {
        let monitor = state.completion.as_ref().ok_or_else(|| {
            error(
                ErrorCode::CorruptLedger,
                "construction opening lost its original monitor",
            )
        })?;
        let source = work.take(
            c,
            completion::source_reserve(monitor.store.definition().origin())?,
        )?;
        let bytes = (monitor.store.byte_len() as u64)
            .checked_add(8192)
            .ok_or_else(exhausted)?;
        Ok(Self {
            deadline: work.deadline,
            source,
            monitor: work.take(c, bytes)?,
            clock,
            monitor_head: monitor.store.head(),
        })
    }
    fn current<S: EffectJournalStorage, N: BuildSource>(
        &self,
        state: &State<S, N>,
        reserved: &OperationContext,
    ) -> Result<OperationContext> {
        let remaining: u64 = self
            .deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(exhausted)?
            .as_millis()
            .try_into()
            .map_err(|_| exhausted())?;
        if remaining == 0 {
            return Err(exhausted());
        }
        let mut current = reserved.clone();
        current.budget.max_wall_millis = current.budget.max_wall_millis.min(remaining);
        state.disclosure_context(&current)
    }
}
fn finalize_opening_publication<S: EffectJournalStorage, N: BuildSource>(
    state: &mut State<S, N>,
    reservation: &OpeningPublication,
    expected_clock: Option<dfmcp_adapter::construction_plan::clock::Recovery>,
    checkpoint: impl FnOnce(&OperationContext) -> Result<()>,
) -> Result<()> {
    checkpoint(&reservation.current(state, &reservation.source)?)?;
    let source = reservation.current(state, &reservation.source)?;
    let monitor = state.completion.as_ref().ok_or_else(|| {
        error(
            ErrorCode::CorruptLedger,
            "construction opening lost its original monitor",
        )
    })?;
    let view = completion::verify_source(
        &mut state.control,
        &mut state.batch,
        &state.binding,
        monitor.store.definition().origin(),
        (&monitor.parent, &monitor.child),
        &source,
    )?;
    state.authority_tick = state.authority_tick.max(state.control.high_tick());
    state.historical = Some(view);
    let current = reservation.current(state, &reservation.monitor)?;
    let monitor = state.completion.as_mut().ok_or_else(|| {
        error(
            ErrorCode::CorruptLedger,
            "construction opening lost its original monitor",
        )
    })?;
    monitor.store.verify(&current)?;
    if monitor.store.head() != reservation.monitor_head {
        return Err(error(
            ErrorCode::CorruptLedger,
            "construction monitor changed at opening publication",
        ));
    }
    let current = reservation.current(state, &reservation.clock)?;
    clock::inspect(state, &current)?;
    if state.clock_recovery != expected_clock {
        return Err(error(
            ErrorCode::CorruptLedger,
            "construction clock changed at opening publication",
        ));
    }
    reservation.current(state, &reservation.clock)?;
    Ok(())
}
fn session_id(raw: &str) -> Result<SessionId> {
    if raw.len() != 32
        || !raw
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "invalid furniture session ID",
        ));
    }
    let n = u128::from_str_radix(raw, 16)
        .map_err(|_| error(ErrorCode::InvalidRequest, "invalid furniture session ID"))?;
    let id = SessionId::new(n);
    if id.get() != n || !id.is_process_scoped_live() || (n & ((1u128 << 62) - 1)) >> 57 != 19 {
        return Err(error(
            ErrorCode::InvalidRequest,
            "wrong furniture session family",
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
    let monitor_action = matches!(
        &action,
        Ok(Action::Completion(_) | Action::RoomCompletion(_))
    );
    let read_only = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(monitor_action));
    let worker_read_only = read_only.clone();
    let output = runtime::owned(op, move |control| {
        let result = (|| {
            let id = session_id(&raw)?;
            let mut locked = lock()?;
            let entry = locked
                .as_mut()
                .filter(|e| e.id() == id)
                .ok_or_else(|| error(ErrorCode::SessionNotFound, "furniture session absent"))?;
            let (state, config) = match entry {
                Entry::Completion { state, config } => {
                    worker_read_only.store(true, Ordering::Release);
                    return Ok(completion::run_recovery(
                        state, config, &control, op, action, wall,
                    ));
                }
                Entry::ClockRecovery { state, config } => {
                    let mut c = state.context(wall)?;
                    let mut n = state.native_context(&c)?;
                    if !runtime::clock_enabled().unwrap_or(false) {
                        c.grants
                            .retain(|grant| grant.capability == Capability::Query);
                        n.grants
                            .retain(|grant| grant.capability == Capability::Query);
                    }
                    let mut io = runtime::ClockIo {
                        control: &control,
                        config,
                        opening_recovery: config.clock.as_ref().and_then(|value| value.recovery),
                    };
                    return Ok(state.run_action(c, n, op, action, control.started, &mut io));
                }
                Entry::Placement { state, config } => (state, config),
            };
            let placement_enabled = runtime::enabled().unwrap_or(false);
            let clock_enabled = runtime::clock_enabled().is_ok_and(|v| v);
            let room_clock_enabled = runtime::room_clock_enabled().is_ok_and(|v| v);
            let mut c = match state.context(
                placement_enabled || clock_enabled || room_clock_enabled,
                wall,
            ) {
                Ok(c) => c,
                Err(e) => return Ok(state.failed(op, None, &e)),
            };
            c.grants.retain(|grant| {
                (placement_enabled || grant.capability != Capability::Construct)
                    && (clock_enabled
                        || room_clock_enabled
                        || grant.capability != Capability::ControlClock)
            });
            state.worker_acknowledgement = Some(control.acknowledgement());
            if let Err(e) = runtime::boundary(&control, config, false) {
                state.abandon();
                return Ok(state.failed(op, Some(&c), &e));
            }
            let config = config.clone();
            let action = match action {
                Ok(Action::Clock(action)) => {
                    let mut io = runtime::ClockIo {
                        control: &control,
                        config: &config,
                        opening_recovery: state.clock_recovery,
                    };
                    return Ok(clock::run_action(
                        state,
                        c,
                        op,
                        action,
                        control.started,
                        &mut io,
                    ));
                }
                Ok(Action::RoomClock(action)) => {
                    let mut io = runtime_room_clock::Io {
                        control: &control,
                        config: &config,
                        opening_recovery: state.room_clock_recovery,
                        observed_floor: state.authority_tick,
                    };
                    return Ok(room_clock::run_action(
                        state,
                        c,
                        op,
                        action,
                        control.started,
                        &mut io,
                    ));
                }
                Ok(Action::RoomCompletion(action)) => {
                    let mut io = runtime::RoomIo {
                        control: &control,
                        config: &config,
                        observed_floor: state.authority_tick,
                    };
                    return Ok(room_monitor::run(
                        state,
                        &config,
                        c,
                        op,
                        action,
                        control.started,
                        &mut io,
                    ));
                }
                Ok(Action::Completion(action)) => {
                    state.abandon_all();
                    return Ok(completion::run(state, &config, &control, c, op, action));
                }
                other => other,
            };
            let mut guard = runtime::Guard {
                control: &control,
                config: &config,
            };
            let clock_handoff = state.clock_config.is_some() || state.clock.is_some();
            if clock_handoff {
                c.budget.max_bytes = match c.budget.max_bytes.checked_sub(clock::VIEW_BYTES) {
                    Some(bytes) => bytes,
                    None => return Ok(state.failed(op, Some(&c), &exhausted())),
                };
            }
            let room_publication = match (|| {
                let mut room_work = Work::new(&c, control.started)?;
                let room_before = room_work.bytes;
                let reserved = room_monitor::Publication::reserve(state, &c, &mut room_work)?;
                c.budget.max_bytes = c
                    .budget
                    .max_bytes
                    .checked_sub(room_before - room_work.bytes)
                    .ok_or_else(exhausted)?;
                Ok::<_, dfmcp_core::DfmcpError>(reserved)
            })() {
                Ok(reserved) => reserved,
                Err(e) => return Ok(state.failed(op, Some(&c), &e)),
            };
            let rendered = run_action(
                state,
                c.clone(),
                op,
                action,
                control.started,
                &mut guard,
                |selection, recovery, b, c, d| {
                    config.matches(b)?;
                    runtime::connect(
                        &config,
                        selection,
                        if recovery { Some(b) } else { None },
                        c,
                        d,
                    )
                },
            );
            let clock_recovery = state.clock_recovery;
            if let Err(e) = runtime::boundary(&control, &config, false) {
                state.abandon();
                return Ok(state.failed(op, Some(&c), &e));
            }
            if clock_handoff
                && let Err(e) = verify_clock_handoff(state, &c, control.started, clock_recovery)
            {
                return Ok(state.failed(op, Some(&c), &e));
            }

            if let Some(reservation) = room_publication {
                let expected = state.clock_recovery;
                if let Err(e) = reservation.verify(state, expected, false, false, |_| Ok(())) {
                    return Ok(state.failed(op, Some(&c), &e));
                }
            }
            Ok(rendered)
        })();
        match result {
            Ok(v) => v,
            Err(e) => unbound(op, &e),
        }
    })
    .await;
    if read_only.load(Ordering::Acquire) {
        completion::read_only_packet(output)
    } else {
        output
    }
}

/// Open only the retained construction monitor and run journal. Original
/// placement/parent paths remain configuration labels; this branch never opens
/// them, allocates furniture, acquires a lease or contacts a native source.
fn open_clock_recovery(
    config: &Config,
    request: &runtime::RequestControl,
    context: &OperationContext,
    native_grants: Vec<CapabilityGrant>,
    work: &mut Work,
) -> Result<(
    clock_recovery::RecoveryState<
        PrivateBuildFile,
        dfmcp_adapter::bounded_run::private_file::PrivateRunFile,
    >,
    String,
)> {
    use clock::ClockIo as _;
    use dfmcp_adapter::construction_plan::store::{MAX_STORE_BYTES, open_private_monitor};
    if !config.clock_only || config.completion_only || context.budget.max_game_ticks != 0 {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "clock recovery cannot acquire new work",
        ));
    }
    let mode = match config.mode {
        BuildMode::Offline => clock_recovery::Mode::Offline,
        BuildMode::Recover => clock_recovery::Mode::Recovery,
        BuildMode::Control => {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "clock recovery requires its restricted mode",
            ));
        }
    };
    let path = config.completion_path.as_ref().ok_or_else(exhausted)?;
    let pin = config.clock_monitor_recovery.ok_or_else(exhausted)?;
    let selected = config.clock.as_ref().ok_or_else(exhausted)?;
    if selected.initialize || selected.recovery.is_none() {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "clock recovery requires existing acknowledged custody",
        ));
    }
    let mut c = context.clone();
    c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    clock::native_context(&c, &native_grants)?;
    runtime::boundary(request, config, false)?;
    let metadata = std::fs::symlink_metadata(path).map_err(|_| {
        error(
            ErrorCode::CorruptLedger,
            "original construction monitor is unavailable",
        )
    })?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_STORE_BYTES as u64 {
        return Err(error(
            ErrorCode::CorruptLedger,
            "original construction monitor has invalid custody bounds",
        ));
    }
    let length = usize::try_from(metadata.len()).map_err(|_| exhausted())?;
    let inspect_bytes = clock_recovery::inspect_reserve(length)?;
    // Reserve every full-history check before either original file is opened.
    let opening = work.take(
        &c,
        metadata
            .len()
            .checked_mul(8)
            .and_then(|bytes| bytes.checked_add(2 * 1024 * 1024))
            .ok_or_else(exhausted)?,
    )?;
    let acknowledged = work.take(&c, metadata.len().checked_add(8192).ok_or_else(exhausted)?)?;
    let clock_open = work.take(&c, clock::OPEN_BYTES)?;
    let constructor = work.take(&c, inspect_bytes)?;
    let inspection = work.take(&c, inspect_bytes)?;
    let publication = work.take(&c, inspect_bytes)?;
    let narrow = |reserved: &OperationContext,
                  current: &OperationContext,
                  work: &Work|
     -> Result<OperationContext> {
        let mut out = reserved.clone();
        out.anchor = current.anchor;
        out.grants = current.grants.clone();
        out.budget.max_game_ticks = 0;
        out.budget.max_wall_millis = out
            .budget
            .max_wall_millis
            .min(work.current(current)?.budget.max_wall_millis);
        Ok(out)
    };
    let mut monitor =
        open_private_monitor(path, &narrow(&opening, &c, work)?, BuildMode::Offline, None)?;
    let definition = monitor.definition().clone();
    config.matches(definition.origin().definition().binding())?;
    selected.matches(&definition)?;
    let floor = definition
        .origin()
        .receipts()
        .iter()
        .flat_map(|record| {
            [
                record.plan().before().tick(),
                record.after().map_or(0, |after| after.tick()),
            ]
        })
        .chain(monitor.progress().last_tick)
        .max()
        .unwrap_or(0);
    c.anchor.tick = c.anchor.tick.max(GameTick(floor));
    monitor.verify_recovery(&narrow(&acknowledged, &c, work)?, pin)?;
    let operations = monitor
        .first_source()
        .ok_or_else(|| {
            error(
                ErrorCode::StaleAnchor,
                "clock recovery requires the original accepted operations source",
            )
        })?
        .operations
        .clone();
    let opening = narrow(&clock_open, &c, work)?;
    let native = clock::native_context(&opening, &native_grants)?;
    let mut io = runtime::ClockIo {
        control: request,
        config,
        opening_recovery: selected.recovery,
    };
    let owner = io.open(selected, &definition, &operations, &opening, &native)?;
    c.anchor.tick = c.anchor.tick.max(GameTick(owner.high_tick()));
    let mut native_grants = native_grants;
    if !owner.is_control() {
        c.grants
            .retain(|grant| grant.capability == Capability::Query);
        native_grants.retain(|grant| grant.capability == Capability::Query);
    }
    let constructor = narrow(&constructor, &c, work)?;
    let native = clock::native_context(&constructor, &native_grants)?;
    let mut state =
        clock_recovery::RecoveryState::new(monitor, owner, &constructor, &native, config, mode)?;
    state.set_budget(context.budget);
    c.anchor.tick = GameTick(state.high_tick());
    let inspection = narrow(&inspection, &c, work)?;
    let native = state.native_context(&inspection)?;
    let mut result = state.inspect(&inspection, &native)?;
    c.anchor.tick = GameTick(state.high_tick());
    result["session_opened"] = json!(true);
    result["session_id"] = json!(state.id().to_string());
    result["mode"] = json!(mode.as_str());
    result["capabilities"] = json!(
        c.grants
            .iter()
            .map(|grant| grant.capability.as_str())
            .collect::<Vec<_>>()
    );
    result["native_capabilities"] = json!(
        native_grants
            .iter()
            .map(|grant| grant.capability.as_str())
            .collect::<Vec<_>>()
    );
    let expected_monitor = state.monitor_reference()["recovery"].clone();
    let expected_clock = state.clock_reference()["recovery"].clone();
    let published_tick = state.high_tick();
    let native = state.native_context(&c)?;
    let output = state.reply("fortress.open_session", &c, &native, result);
    if output.len() as u64 > OUTPUT_BYTES {
        return Err(exhausted());
    }
    work.current(&c)?;
    runtime::boundary(request, config, false)?;
    let publication = narrow(&publication, &c, work)?;
    let native = state.native_context(&publication)?;
    state.inspect(&publication, &native)?;
    if state.monitor_reference()["recovery"] != expected_monitor
        || state.clock_reference()["recovery"] != expected_clock
        || state.high_tick() != published_tick
    {
        return Err(error(
            ErrorCode::StaleAnchor,
            "original recovered histories changed before publication",
        ));
    }
    work.current(&c)?;
    Ok((state, output))
}

#[tool(
    name = "fortress.open_session",
    description = "Open isolated furniture/1.19 custody. Use exactly one of selection JSON [kind,item_id,x,y,z], furniture_plan as complete dfmcp.furniture-plan/1 JSON, furniture_request as dfmcp.furniture-request/1 JSON, or room_request as dfmcp.room-construction-request/1 JSON with terrain region/floors/walls and the full nested furniture request. A request allocates all slots from one fresh operations/1.4 capture and seals its constraints and chosen items into operator-configured batch custody; room intake also retains every declared terrain requirement. Shortage creates no partial batch. Reopen retained custody without intent inputs or native bootstrap. Optional operator clock custody requires an existing completion monitor with an accepted sample; max_game_ticks defaults to zero and positive values require clock control. No client paths or policies. Operator clock-offline and clock-recover reopen only acknowledged monitor and clock histories without original placement files, native bootstrap or future game ticks. Opening never prepares, places, observes room terrain or advances time."
)]
#[expect(
    clippy::too_many_arguments,
    reason = "the frozen MCP schema requires flat tool arguments"
)]
pub async fn fortress_open_session(
    selection: Option<String>,
    max_wall_millis: Option<u64>,
    max_bytes: Option<u64>,
    max_output_tokens: Option<u32>,
    furniture_plan: Option<String>,
    furniture_request: Option<String>,
    max_game_ticks: Option<u64>,
    room_request: Option<String>,
) -> String {
    runtime::owned("fortress.open_session", move |control| {
        let result = (|| {
            let config = runtime::configuration()?;
            runtime::boundary(&control, &config, false)?;
            // Parse and bound complete intent before native contact or storage.
            room_intake::exclusive([selection.is_some(), furniture_plan.is_some(), furniture_request.is_some(), room_request.is_some()])?;
            if config.clock.is_some() && (selection.is_some() || furniture_plan.is_some() || furniture_request.is_some() || room_request.is_some()) {
                return Err(error(ErrorCode::InvalidRequest,
                    "construction clock opens only retained custody after its first accepted completion sample"));
            }
            let mut plan = furniture_plan.as_deref().map(|s|FurniturePlan::decode(s.as_bytes())).transpose()?;
            let room = room_request.as_deref().map(room_intake::parse).transpose()?;
            let requested = match &room {
                Some(room) => Some(room.furniture().clone()),
                None => furniture_request.as_deref().map(|s|FurnitureRequest::decode(s.as_bytes())).transpose()?,
            };
            let importing = plan.is_some() || requested.is_some();
            if let Some(requested) = &requested {
                if config.batch_path.is_none() || config.mode != BuildMode::Control || config.completion_only {
                    return Err(error(ErrorCode::InvalidRequest, "furniture request requires new Control batch custody"));
                }
                allocation::validate_request(requested, &config)?;
                if let Some(room) = &room { room_intake::validate(room, &config)?; }
            }
            let mut selected = if let Some(plan) = &plan {
                if config.batch_path.is_none() || config.mode != BuildMode::Control || selection.is_some() {
                    return Err(error(ErrorCode::InvalidRequest, "complete plan requires configured Control batch custody and no selection"));
                }
                batch::reserve_output(plan)?;
                for step in plan.steps() {
                    let halo = config.selection(step.selection)?;
                    if config.protected.iter().any(|region|dfmcp_core::cuboids_intersect(region,&halo)) {
                        return Err(error(ErrorCode::CapabilityDenied, "batch target intersects protected region"));
                    }
                }
                plan.ordered_steps().next().map(|step|step.selection)
            } else if config.batch_path.is_some() {
                if selection.is_some() {
                    return Err(error(ErrorCode::InvalidRequest, "reopening a batch derives selections from its retained plan"));
                }
                None
            } else {
                let selected = selection.as_deref().map(parse_selection).transpose()?;
                if (config.mode == BuildMode::Control) != selected.is_some() {
                    return Err(error(ErrorCode::InvalidRequest, "single control requires selection; recovery/offline must omit it"));
                }
                selected
            };
            let budget = WorkBudget { max_wall_millis:max_wall_millis.unwrap_or(10000),
                max_bytes:max_bytes.unwrap_or(MAX_WORK_BYTES),max_output_tokens:max_output_tokens.unwrap_or(16384),
                max_entities:73729,max_actions:1,max_game_ticks:max_game_ticks.unwrap_or(0) };
            if budget.max_game_ticks > 1200 || (budget.max_game_ticks > 0
                && (config.mode != BuildMode::Control
                    || !((config.clock.is_some() && runtime::clock_enabled()?)
                        || (config.room_clock.is_some() && runtime::room_clock_enabled()?)))) {
                return Err(error(ErrorCode::CapabilityDenied,"game ticks require explicit construction clock control"));
            }
            let mut locked = lock()?;
            if locked.is_some() { return Err(error(ErrorCode::Conflict, "release the current furniture session first")); }
            let seq = NEXT.try_update(Ordering::AcqRel,Ordering::Acquire,|v|(v<(1u64<<57)).then_some(v+1)).map_err(|_|exhausted())?;
            let id = SessionId::new((1u128<<127)|FAMILY|u128::from(seq));
            let mut c = context(id,RequestId::new(1),config.fortress.fortress_id(),0,budget,config.mode,runtime::enabled()?);
            let recovery_native_grants = if config.clock_only {
                let (canonical, native) = clock_recovery_authority(&config, runtime::clock_enabled()?);
                c.grants = canonical;
                native
            } else {
                add_clock_grants(&mut c, &config,
                    (config.clock.is_some() && runtime::clock_enabled()?)
                    || (config.room_clock.is_some() && runtime::room_clock_enabled()?));
                Vec::new()
            };
            let mut work = Work::new(&c,control.started)?;
            if config.clock_only {
                let (state, output) = open_clock_recovery(&config, &control, &c, recovery_native_grants, &mut work)?;
                *locked = Some(Entry::ClockRecovery { state, config });
                return Ok(output);
            }
            let mut opening_clock_replay = if config.clock.is_some() {
                // Reserve the final replay before bootstrap or any storage work.
                Some(work.take(&c, clock::VIEW_BYTES)?)
            } else { None };
            if config.completion_only {
                if selection.is_some() || plan.is_some() || requested.is_some() {
                    return Err(error(ErrorCode::InvalidRequest, "completion recovery reopens only its retained original monitor"));
                }
                let (state, output) = completion::open_recovery(&config, &control, &c, &mut work)?;
                *locked = Some(Entry::Completion { state, config });
                return Ok(output);
            }
            if importing {
                c.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
                c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
                // Import only creates a missing parent. Reopening derives all
                // intent from retained custody, so a substituted client plan
                // must never select even a bootstrap native observation.
                if let Some(path) = &config.batch_path {
                    match std::fs::symlink_metadata(path) {
                        Ok(_) => return Err(error(ErrorCode::Conflict,
                            "existing batch custody must be reopened without a supplied plan")),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
                        Err(_) => return Err(error(ErrorCode::CorruptLedger,
                            "batch parent existence cannot be established")),
                    }
                    work.current(&c)?;
                }
            }
            let mut handoff = None;
            if let Some(requested) = &requested {
                let observed = runtime::furniture_supply(&config, &work.take(&c, 20 * 1024 * 1024)?)?;
                runtime::boundary(&control, &config, false)?;
                let anchor = observed.snapshot().ok_or_else(|| error(ErrorCode::AdapterRejected,
                    "furniture supply lacks a complete operations snapshot"))?.anchor();
                c.anchor.tick = GameTick(c.anchor.tick.get().max(anchor.tick.get()));
                c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
                c.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
                let mut analysis_context = work.take(&c, 256 * 1024 * 1024)?;
                analysis_context.anchor = anchor;
                // Native I/O has finished, but CPU-bound allocation still belongs
                // to this joined request. Keep cancellation, inherited runtime
                // restrictions and the operator's Plan opt-in live throughout it.
                let outcome = Handoff::allocate_with_check(&observed, &analysis_context, config.endpoint,
                    requested, dfmcp_adapter::furniture_supply::MAX_WORK,
                    &mut || runtime::boundary(&control, &config, true))?;
                work.current(&c)?;
                runtime::boundary(&control, &config, false)?;
                if outcome.handoff.is_none() {
                    let result = allocation::shortage_result(&outcome, requested, room.as_ref())?;
                    let output = packet("fortress.open_session",result,Some(&c),None,None,None,None,None,None);
                    if output.len() as u64 > OUTPUT_BYTES { return Err(exhausted()); }
                    work.current(&c)?.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
                    runtime::boundary(&control, &config, false)?;
                    return Ok(output);
                }
                let allocated = outcome.handoff.ok_or_else(|| error(ErrorCode::InternalInvariantViolation,
                    "complete furniture allocation lost its handoff"))?;
                room_intake::reserve_output(allocated.plan(), room.as_ref())?;
                selected = allocated.plan().ordered_steps().next().map(|step|step.selection);
                plan = Some(allocated.plan().clone());
                handoff = Some(allocated);
            }
            let binding = if let Some(selection) = selected {
                config.selection(selection)?;
                let child = work.take(&c,SOURCE_BYTES)?;
                let source = runtime::connect(&config,selection,None,&child,Duration::from_millis(child.budget.max_wall_millis))?;
                let capture = source.initial_capture().ok_or_else(||error(ErrorCode::AdapterRejected,"furniture bootstrap capture missing"))?;
                if capture.fortress()!=&config.fortress || capture.selection()!=selection {
                    return Err(error(ErrorCode::StaleAnchor,"furniture bootstrap source differs from configuration"));
                }
                if let Some(handoff) = &handoff {
                    handoff.validate_capture(source.binding(), capture)?;
                }
                if let Some(room) = &room {
                    room_intake::validate_dimensions(room, source.binding().dimensions())?;
                }
                c.anchor.tick = GameTick(capture.tick());
                Some(source.binding().clone())
            } else { None };
            runtime::boundary(&control,&config,importing)?;
            let source_reserve = if config.completion_path.is_some() || config.room_completion_path.is_some() || handoff.is_some() {
                completion::open_reserve(&config.path, 16 * 1024 * 1024)?
            } else { EFFECT_BYTES };
            let mut journal = open_private_build(&config.path,&work.take(&c,source_reserve)?,config.mode,binding)?;
            config.matches(journal.binding())?;
            c.anchor.tick = GameTick(c.anchor.tick.get().max(journal.high_tick()));
            let mut retained_parent = None;
            if let Some(path) = &config.batch_path {
                let view = journal.inventory(&work.take(&c,LOCAL_BYTES)?)?;
                let expected = if let Some(plan) = plan {
                    if !view.entries().is_empty() {
                        return Err(error(ErrorCode::Conflict,"new batch requires an empty original journal; reopen retained work without a plan"));
                    }
                    Some(if let Some(handoff) = handoff {
                        if let Some(room) = room {
                            BatchDefinition::from_room_handoff(room,handoff,journal.binding().clone(),view.journal_id)?
                        } else {
                            BatchDefinition::from_handoff(handoff,journal.binding().clone(),view.journal_id)?
                        }
                    } else {
                        BatchDefinition::new(plan,journal.binding().clone(),view.journal_id)?
                    })
                } else { None };
                runtime::boundary(&control,&config,importing)?;
                let mut parent = open_private_batch(path,&work.take(&c,LOCAL_BYTES)?,config.mode,expected)?;
                batch::matches(parent.definition(),journal.binding())?;
                room_intake::reserve_output(parent.definition().plan(), parent.definition().room())?;
                if let Some(room) = parent.definition().room() { room_intake::validate(room, &config)?; }
                batch::verify(&mut parent,&view,&work.take(&c,batch::GUARD_BYTES)?)?;
                // An allocated batch may restart before its first child exists.
                // Its retained source tick must initialize the new session and
                // host lease, rather than creating an already-expired tick-zero
                // lease or requiring another inventory capture/reallocation.
                if let Some(handoff) = parent.definition().handoff() {
                    c.anchor.tick = GameTick(c.anchor.tick.get().max(handoff.source().anchor.tick.get()));
                    c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
                }
                retained_parent = Some(parent);
            }
            let session = BuildSession::<_,BuildRpc>::new(journal,&work.view(&c)?)?;
            let mut state = State::new(session,&work.take(&c,LOCAL_BYTES)?,&config)?;
            state.budget = budget;
            state.grants = c.grants.clone();
            state.clock_grants = runtime::clock_grants(config.mode == BuildMode::Control
                && config.clock.is_some() && runtime::clock_enabled()?);
            state.room_clock_grants=runtime::clock_grants(config.mode==BuildMode::Control
                && config.room_clock.is_some() && runtime::room_clock_enabled()?);
            state.batch = retained_parent;
            c = state.disclosure_context(&c)?;
            let view = state.control.inventory(&work.view(&c)?)?;
            state.verify_batch(&work.take(&c,batch::GUARD_BYTES)?,&view)?;
            completion::reopen(&mut state, &config, &c, &mut work, &view)?;
            if let Err(e) = room_monitor::reopen(&mut state, &config, &c, &mut work, &view) {
                return Ok(state.failed("fortress.open_session", Some(&c), &e));
            }
            let room_publication = room_monitor::Publication::reserve(&state, &c, &mut work)?;
            let mut opening_publication = None;
            if let Some(selected) = config.clock.as_ref() {
                use clock::ClockIo as _;
                let monitor = state.completion.as_ref().ok_or_else(|| error(ErrorCode::InvalidRequest,
                    "enable the construction clock only after an original completion monitor has its first accepted sample"))?;
                let definition = monitor.store.definition().clone();
                let operations = monitor.store.first_source()
                    .ok_or_else(|| error(ErrorCode::InvalidRequest,
                        "construction clock requires one accepted original completion sample"))?.operations.clone();
                c = state.disclosure_context(&c)?;
                opening_publication = Some(OpeningPublication::reserve(&state, &c, &mut work,
                    opening_clock_replay.take().ok_or_else(exhausted)?)?);
                let opening = work.take(&c, clock::OPEN_BYTES)?;
                let native = clock::native_context(&opening, &state.clock_grants)?;
                let mut io = runtime::ClockIo { control: &control, config: &config,
                    opening_recovery: selected.recovery };
                let opened = io.open(selected, &definition, &operations, &opening, &native);
                state.clock_recovery = io.opening_recovery();
                let owner = match opened {
                    Ok(owner) => owner,
                    Err(e) => return Ok(state.failed("fortress.open_session",Some(&c),&e)),
                };
                let agrees = owner.definition().canonical_bytes() == definition.canonical_bytes() && owner.operations() == &operations
                    && owner.binding() == &selected.binding;
                state.clock_recovery = Some(owner.recovery());
                state.authority_tick = state.authority_tick.max(owner.high_tick());
                state.clock = Some(owner);
                if !agrees { return Ok(state.failed("fortress.open_session",Some(&c),
                    &error(ErrorCode::StaleAnchor,"opened construction clock differs from configured origin"))); }
                let checked = work.take(&c, clock::VIEW_BYTES)
                    .and_then(|current| clock::inspect(&mut state, &current));
                if let Err(e) = checked {
                    return Ok(state.failed("fortress.open_session",Some(&c),&e));
                }
            }
            if config.room_clock.is_some() {
                let mut io=runtime_room_clock::Io {control:&control,config:&config,
                    opening_recovery:state.room_clock_recovery,observed_floor:state.authority_tick};
                if let Err(e)=room_clock::open(&mut state,&c,&mut work,&mut io) {
                    return Ok(state.failed("fortress.open_session",Some(&c),&e));
                }
            }
            c = match state.disclosure_context(&c) {
                Ok(current) => current,
                Err(e) => return Ok(state.failed("fortress.open_session",Some(&c),&e)),
            };
            state.worker_acknowledgement = Some(control.acknowledgement());
            let mut result = json!({"ok":true,"session_opened":true,"session_id":id.to_string(),"mode":mode_name(config.mode),"journal":inventory(&view),
                "capabilities":c.grants.iter().map(|g|g.capability.as_str()).collect::<Vec<_>>(),"planning_observation_retained":false,
                "native_preparation_dispatched":false,"game_mutation_dispatched":false});
            state.attach_batch(&mut result,Some(&view),true);
            let output = packet("fortress.open_session",result,Some(&c),Some(&state.binding),Some(&view),None,Some(&state.policy),None,None);
            if output.len() as u64>OUTPUT_BYTES {
                return Ok(state.failed("fortress.open_session",Some(&c),&exhausted()));
            }
            let finalized = if let Some(reservation) = opening_publication.as_ref() {
                let expected = state.clock_recovery;
                finalize_opening_publication(&mut state, reservation, expected,
                    |_| work.current(&c).and_then(|_| runtime::boundary(&control,&config,importing)))
            } else {
                work.current(&c).and_then(|_| runtime::boundary(&control,&config,importing))
            };
            if let Err(e) = finalized {
                return Ok(state.failed("fortress.open_session",Some(&c),&e));
            }
            if let Some(reservation) = room_publication {
                let expected = state.clock_recovery;
                if let Err(e) = reservation.verify(&mut state, expected, false, false, |_| Ok(())) {
                    return Ok(state.failed("fortress.open_session", Some(&c), &e));
                }
            }
            *locked = Some(Entry::Placement{state,config});
            Ok(output)
        })();
        match result { Ok(v)=>v,Err(e)=>unbound("fortress.open_session",&e) }
    }).await
}
fn mode_name(mode: BuildMode) -> &'static str {
    match mode {
        BuildMode::Control => "control",
        BuildMode::Recover => "recover",
        BuildMode::Offline => "offline",
    }
}
#[tool(
    name = "fortress.observe",
    description = "Capture an exact furniture selection, next for the original batch step, completion for one whole-plan construction sample, or room_completion for one joint original terrain-and-furnishings sample. Completion brackets all original receipts around one complete operations capture and durably records progress. Observation abandons local placement and clock review permissions, never pending work. Configured unresolved clock work prevents fresh completion samples."
)]
pub async fn fortress_observe(session_id: String, selection: String) -> String {
    with_session(
        session_id,
        "fortress.observe",
        None,
        if selection == "room_completion" {
            Ok(Action::RoomCompletion(room_monitor::Action::Sample))
        } else if selection == "completion" {
            Ok(Action::Completion(completion::Action::Sample))
        } else if selection == "next" {
            Ok(Action::ObserveNext)
        } else {
            parse_selection(&selection).map(Action::Observe)
        },
    )
    .await
}
#[tool(
    name = "fortress.plan",
    description = "Prepare one furniture placement from this session's exact observation witness under current scope, lease, protected regions and checkpoint policy. A batch requires its returned original step key; review also binds the complete parent plan and prepared journal head. Alternatively supply clock JSON {monitor_id,game_ticks,wall_millis} to prepare a finite whole-batch clock interval after fresh complete receipt-bracket readiness; scope=room_clock instead binds the exact joint room monitor and full map-and-furnishings readiness. Returns a separately confirmable review seal. Preparation never places a building or unpauses the game."
)]
pub async fn fortress_plan(
    session_id: String,
    idempotency_key: Option<String>,
    observation_witness: Option<String>,
    clock: Option<String>,
    scope: Option<String>,
) -> String {
    let action = plan_action(
        scope.as_deref(),
        idempotency_key,
        observation_witness,
        clock,
    );
    with_session(session_id, "fortress.plan", None, action).await
}
fn plan_action(
    scope: Option<&str>,
    idempotency_key: Option<String>,
    observation_witness: Option<String>,
    request: Option<String>,
) -> Result<Action> {
    match (scope, idempotency_key, observation_witness, request) {
        (None | Some("effect"), Some(key_value), Some(witness), None) => {
            key(&key_value)?;
            Ok(Action::Plan {
                key: key_value,
                witness: digest(&witness)?,
            })
        }
        (None | Some("clock"), None, None, Some(raw)) => clock::Request::parse(&raw)
            .map(|request| Action::Clock(clock::Action::Prepare(request))),
        (Some("room_clock"), None, None, Some(raw)) => room_clock::Request::parse(&raw)
            .map(|request| Action::RoomClock(room_clock::Action::Prepare(request))),
        _ => Err(error(
            ErrorCode::InvalidRequest,
            "choose one exact placement, furniture clock or room clock request",
        )),
    }
}

#[tool(
    name = "fortress.commit",
    description = "Confirm exact plan digest and review seal for one native placement attempt on the original preparation connection. Current policy and complete capture are revalidated after dispatch state is durable. scope=clock or room_clock instead requires the original monitor_id and its separate review_seal, with no placement key/digest; fresh whole-batch readiness precedes one finite unpause attempt. Uncertainty requires original-outcome recovery. Placement and clock receipts do not prove completed construction."
)]
pub async fn fortress_commit(
    session_id: String,
    idempotency_key: Option<String>,
    plan_digest: Option<String>,
    review_seal: String,
    scope: Option<String>,
    monitor_id: Option<String>,
) -> String {
    let action = (|| {
        let seal = digest(&review_seal)?;
        match (
            scope.as_deref().unwrap_or("effect"),
            idempotency_key,
            plan_digest,
            monitor_id,
        ) {
            ("effect", Some(key_value), Some(plan), None) => {
                key(&key_value)?;
                Ok(Action::Commit {
                    key: key_value,
                    plan: digest(&plan)?,
                    seal,
                })
            }
            ("clock", None, None, Some(id)) => Ok(Action::Clock(clock::Action::Commit {
                id: digest(&id)?,
                seal,
            })),
            ("room_clock", None, None, Some(id)) => {
                Ok(Action::RoomClock(room_clock::Action::Commit {
                    id: digest(&id)?,
                    seal,
                }))
            }
            _ => Err(error(
                ErrorCode::InvalidRequest,
                "commit scope and review identity disagree",
            )),
        }
    })();
    with_session(session_id, "fortress.commit", None, action).await
}

#[tool(
    name = "fortress.query",
    description = "Inspect local evidence with closed JSON modes batch/records/get/selection/schema/completion/room_completion/allocation/construction_clock/room_clock. allocation has view request (default) or items and returns original sealed constraints or selected-item evidence without reallocation. completion_start creates a fixed-deadline monitor from every original Placed receipt in a completed batch under Query authority. completion inspects retained progress and original custody. construction_clock requires monitor_id and exact head; offset and limit page original run history. Independent clock recovery permits historical batch/allocation/completion/clock/schema inspection only. No native calls or renewed commit permission."
)]
pub async fn fortress_query(session_id: String, query: String) -> String {
    with_session(
        session_id,
        "fortress.query",
        None,
        Query::parse(&query).and_then(|q| match q {
            Query::Get {
                idempotency_key,
                plan_digest,
            } => Ok(Action::Explain(idempotency_key, digest(&plan_digest)?)),
            Query::Completion {} => Ok(Action::Completion(completion::Action::Inspect)),
            Query::RoomCompletion {} => Ok(Action::RoomCompletion(room_monitor::Action::Inspect)),
            Query::RoomCompletionStart {
                deadline,
                interval,
                stable_samples,
                stable_span,
                max_gap,
                max_observations,
            } => Ok(Action::RoomCompletion(room_monitor::Action::Start(
                dfmcp_adapter::construction_plan::Timing {
                    deadline,
                    interval: interval.map_or(1, |v| v),
                    stable_samples: stable_samples.map_or(2, |v| v),
                    stable_span: stable_span.map_or(1, |v| v),
                    max_gap: max_gap.map_or(1200, |v| v),
                    max_observations: max_observations.map_or(512, |v| v),
                },
            ))),
            Query::RoomClock {
                monitor_id,
                head,
                offset,
                limit,
            } => Ok(Action::RoomClock(room_clock::Action::Inspect {
                id: digest(&monitor_id)?,
                head: digest(&head)?,
                offset: offset.map_or(0, |v| v),
                limit: limit.map_or(8, |v| v),
            })),
            Query::ConstructionClock {
                monitor_id,
                head,
                offset,
                limit,
            } => Ok(Action::Clock(clock::Action::Inspect {
                id: digest(&monitor_id)?,
                head: digest(&head)?,
                offset: offset.unwrap_or(0),
                limit: limit.unwrap_or(8),
            })),
            Query::CompletionStart {
                deadline,
                interval,
                stable_samples,
                stable_span,
                max_gap,
                max_observations,
            } => Ok(Action::Completion(completion::Action::Start(
                dfmcp_adapter::construction_plan::Timing {
                    deadline,
                    interval: interval.unwrap_or(1),
                    stable_samples: stable_samples.unwrap_or(2),
                    stable_span: stable_span.unwrap_or(1),
                    max_gap: max_gap.unwrap_or(1200),
                    max_observations: max_observations.unwrap_or(512),
                },
            ))),
            q => Ok(Action::Query(q)),
        }),
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
    description = "Reconcile one original outcome and synchronize validated evidence. Default scope=effect uses the original furniture key/digest; scope=clock or room_clock uses only monitor_id and queries its unresolved run at most once; scope=room_completion takes no effect identity and performs one joint room sample. Terminal history returns locally. No commit retry, new run, timer or construction-completion claim. Querying clears local commit permission. Independent clock-recover supports the original clock wait under Query authority; clock-offline never contacts the game."
)]
pub async fn fortress_wait(
    session_id: String,
    idempotency_key: Option<String>,
    plan_digest: Option<String>,
    max_wall_millis: Option<u64>,
    scope: Option<String>,
    monitor_id: Option<String>,
) -> String {
    let action = match (
        scope.as_deref().unwrap_or("effect"),
        idempotency_key,
        plan_digest,
        monitor_id,
    ) {
        ("effect", Some(k), Some(p), None) => {
            effect_action(k, p, |k, p| Action::Recover(k, p, false))
        }
        ("room_completion", None, None, None) => {
            Ok(Action::RoomCompletion(room_monitor::Action::Sample))
        }
        ("clock", None, None, Some(id)) => {
            digest(&id).map(|id| Action::Clock(clock::Action::Wait(id)))
        }
        ("room_clock", None, None, Some(id)) => {
            digest(&id).map(|id| Action::RoomClock(room_clock::Action::Wait(id)))
        }
        _ => Err(error(
            ErrorCode::InvalidRequest,
            "wait scope and original identity disagree",
        )),
    };
    with_session(session_id, "fortress.wait", max_wall_millis, action).await
}

#[tool(
    name = "fortress.explain",
    description = "Inspect the exact retained furniture plan, before-capture and native receipt as historical evidence. Offline operation; no new observation and no mutation."
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
    let output = runtime::owned("fortress.cancel", move |control| {
        let result = (|| {
            control.checkpoint()?;
            let id = session_id(&raw)?;
            let mut locked = lock()?;
            let entry = locked
                .as_mut()
                .filter(|e| e.id() == id)
                .ok_or_else(|| error(ErrorCode::SessionNotFound, "furniture session absent"))?;
            let (state, config) = match entry {
                Entry::Completion { state, config } => {
                    let output = completion::close_recovery(state, config, &control, release)?;
                    drop(locked.take());
                    return Ok(output);
                }
                Entry::ClockRecovery { state, config } => {
                    let c = state.context(None)?;
                    let n = state.native_context(&c)?;
                    let output = state.close(c.clone(), n, release, control.started, |_| {
                        if release { control.checkpoint() } else { runtime::boundary(&control, config, false) }
                    });
                    if output.len() as u64 > OUTPUT_BYTES { return Err(exhausted()); }
                    let value: Value = serde_json::from_str(&output).map_err(|_| error(
                        ErrorCode::InternalInvariantViolation, "invalid clock recovery close response"))?;
                    Work::new(&c, control.started)?.current(&c)?;
                    if value["result"]["ok"] == true && value["result"]["closed"] == true {
                        drop(locked.take());
                    }
                    return Ok(output);
                }
                Entry::Placement { state, config } => (state, config),
            };
            let c = state.context(false, None)?;
            state.abandon_all();
            let view = if release {
                None
            } else {
                if let Err(e) = runtime::boundary(&control, config, false) {
                    return Ok(state.failed("fortress.cancel", Some(&c), &e));
                }
                let mut work = Work::new(&c, control.started)?;
                if let Some(view) = clock::inspect(state, &work.take(&c, clock::VIEW_BYTES)?)?
                    && (view.pending || view.unresolved) {
                    return Ok(state.failed("fortress.cancel",Some(&c),
                        &error(ErrorCode::EffectIndeterminate,
                            "unsettled construction clock requires exact recovery or explicit release")));
                }
                if let Some(view)=room_clock::inspect(state,&work.take(&c,room_clock::VIEW_BYTES)?)?
                    && (view.pending || view.unresolved) {
                    return Ok(state.failed("fortress.cancel",Some(&c),&error(ErrorCode::EffectIndeterminate,
                        "unsettled room clock requires exact recovery or explicit release")));
                }
                let view = state.control.inventory(&work.view(&c)?)?;
                if state.room_completion.is_some() {
                    state.verify_batch(&work.take(&c, batch::GUARD_BYTES)?, &view)?;
                    room_monitor::verify(state, &c, &mut work, &view, true)?;
                    if state.room_completion.as_ref().is_some_and(|m| !m.store.progress().terminal()) {
                        return Ok(state.failed("fortress.cancel", Some(&c), &error(ErrorCode::EffectIndeterminate,
                            "active joint room monitor requires local cancellation or explicit recovery release")));
                    }
                }
                Some(view)
            };
            if !release && view.as_ref().is_some_and(|v| v.pending().is_some()) {
                return Ok(state.failed(
                    "fortress.cancel",
                    Some(&c),
                    &error(
                        ErrorCode::EffectIndeterminate,
                        "unresolved work requires explicit recovery release",
                    ),
                ));
            }
            let output = state.close_packet(&c, view.as_ref(), release);
            if output.len() as u64 > OUTPUT_BYTES {
                return Err(exhausted());
            }
            drop(locked.take());
            Ok(output)
        })();
        match result {
            Ok(v) => v,
            Err(e) => unbound("fortress.cancel", &e),
        }
    })
    .await;
    completion::read_only_packet(output)
}
#[tool(
    name = "fortress.cancel",
    description = "scope=room_completion cancels only the original joint room monitor. scope=completion cancels only the local completion monitor, preserving placement effects and evidence. scope=batch stops new batch steps under Query. scope=effect retires an exact prepared native record; cannot undo construction. scope=clock or room_clock with monitor_id retires the original preparation or requests its native safety pause under current clock authority, even after future-tick budgets expire. scope=session releases custody; release_for_recovery preserves unresolved identities. Independent clock recovery permits clock cancellation only with separately retained current clock authority, and explicit session release remains possible after Query loss with evidence redacted."
)]
pub async fn fortress_cancel(
    session_id: String,
    scope: String,
    idempotency_key: Option<String>,
    plan_digest: Option<String>,
    release_for_recovery: Option<bool>,
    monitor_id: Option<String>,
) -> String {
    let release = release_for_recovery.unwrap_or(false);
    match (scope.as_str(), idempotency_key, plan_digest, monitor_id) {
        ("session", None, None, None) => close(session_id, release).await,
        ("batch", None, None, None) if !release => {
            with_session(session_id, "fortress.cancel", None, Ok(Action::StopBatch)).await
        }
        ("room_completion", None, None, None) if !release => {
            with_session(
                session_id,
                "fortress.cancel",
                None,
                Ok(Action::RoomCompletion(room_monitor::Action::Cancel)),
            )
            .await
        }
        ("completion", None, None, None) if !release => {
            with_session(
                session_id,
                "fortress.cancel",
                None,
                Ok(Action::Completion(completion::Action::Cancel)),
            )
            .await
        }
        ("room_clock", None, None, Some(id)) if !release => {
            with_session(
                session_id,
                "fortress.cancel",
                None,
                digest(&id).map(|id| Action::RoomClock(room_clock::Action::Cancel(id))),
            )
            .await
        }
        ("clock", None, None, Some(id)) if !release => {
            with_session(
                session_id,
                "fortress.cancel",
                None,
                digest(&id).map(|id| Action::Clock(clock::Action::Cancel(id))),
            )
            .await
        }
        ("effect", Some(k), Some(p), None) if !release => {
            with_session(
                session_id,
                "fortress.cancel",
                None,
                effect_action(k, p, |k, p| Action::Recover(k, p, true)),
            )
            .await
        }
        _ => {
            with_session(
                session_id,
                "fortress.cancel",
                None,
                Err(error(
                    ErrorCode::InvalidRequest,
                    "cancel scope and identity disagree",
                )),
            )
            .await
        }
    }
}
#[tool(
    name = "fortress.checkpoint",
    description = "Unavailable: the furniture coordination journal is not a verified Dwarf Fortress game save. Default checkpoint policy refuses placement."
)]
pub async fn fortress_checkpoint(session_id: String) -> String {
    with_session(session_id, "fortress.checkpoint", None, Ok(Action::Denied)).await
}
#[tool(
    name = "fortress.restore",
    description = "Unavailable: replaying furniture journal evidence cannot restore game state or remove a constructed building."
)]
pub async fn fortress_restore(session_id: String) -> String {
    with_session(session_id, "fortress.restore", None, Ok(Action::Denied)).await
}
#[tool(
    name = "fortress.doctor",
    description = "Verify local furniture coordination custody and inventory. Independent clock recovery verifies only the retained monitor and clock histories. Does not inspect live fortress health, building completion, other controllers or production admission."
)]
pub async fn fortress_doctor(session_id: String) -> String {
    with_session(session_id, "fortress.doctor", None, Ok(Action::Inventory)).await
}
pub fn run_stdio() {
    if let Err(e) = runtime::configuration() {
        eprintln!("{e}");
        std::process::exit(1);
    }
    let server=ServerBuilder::new("dfmcp-build-placement-dev",env!("CARGO_PKG_VERSION"))
        .tool(FortressOpenSession).tool(FortressObserve).tool(FortressQuery).tool(FortressPlan).tool(FortressCommit)
        .tool(FortressWait).tool(FortressCancel).tool(FortressCheckpoint).tool(FortressRestore).tool(FortressExplain).tool(FortressDoctor)
        .instructions("Unadmitted furniture/1.19 development control. With operator batch custody, use open_session furniture_request (dfmcp.furniture-request/1 JSON) to allocate all requested slots from one fresh operations/1.4 capture, or import an exact furniture_plan (dfmcp.furniture-plan/1 JSON). A shortage creates no session or partial batch. Inspect query mode batch and observe selection next. Query mode allocation with view request or items inspects the immutable original constraints and selection. Review and commit exactly one returned original key at a time; only retained terminal Placed prefixes unlock more steps. Reopening a batch omits all intent inputs, performs no reallocation, and restores no permit. scope=batch cancellation permanently stops advancement while preserving original-key recovery. Single-selection control remains available without batch configuration. Default policy requires an unavailable game checkpoint; only explicit operator disposable-fortress policy permits placement. Query the original key after uncertainty; never retry commit. Placed proves historical stage-zero registration. Once every original step is Placed, completion_start fixes a whole-plan goal, observe selection completion acquires one sample, and query completion or local cancel inspects or stops the monitor. Optional operator construction-clock custody requires one accepted original completion sample before activation. If original placement or parent files are unavailable, operator clock-offline or clock-recover opens only the exact acknowledged monitor and run histories. These modes permit historical inspection, original clock reconciliation and separately authorized safety cancellation, with zero future game ticks and no new review or construction samples. Retain completion.recovery and construction_clock.recovery for both independent histories. Reopen with an explicit max_game_ticks allowance, plan with clock JSON, separately review commit(scope=clock,monitor_id,review_seal), then wait(scope=clock,monitor_id) for one reconciliation. Unsettled clock history blocks completion samples; readiness never counts as positive construction progress. cancel(scope=clock,monitor_id) requests original safety cancellation; query construction_clock requires its exact head. Preserve clock recovery across restart. No canonical world anchor, shared process identity, global controller fence, arbitrary command, verified checkpoint or restore.").build();
    crate::run_modern_stdio(server);
}
#[cfg(test)]
mod tests;

#[cfg(test)]
mod allocation_owner_tests;
