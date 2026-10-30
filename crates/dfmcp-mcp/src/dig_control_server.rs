#![forbid(unsafe_code)]
//! Explicitly unadmitted mining control. All native effects use DigSession and
//! its durable coordinator; recovery-only MCP remains a separate sealed profile.
use dfmcp_adapter::build_placement::journal::private_file::PrivateBuildFile;
use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
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
        state: State<PrivateDigFile, Native, PrivateBuildFile>,
        config: Config,
    },
    Monitor {
        state: monitor_recovery::RecoveryState<PrivateBuildFile>,
        config: Config,
    },
}
impl Entry {
    fn id(&self) -> SessionId {
        match self {
            Self::Native { state, .. } => state.id,
            Self::Monitor { state, .. } => state.id(),
        }
    }
    fn config(&self) -> &Config {
        match self {
            Self::Native { config, .. } | Self::Monitor { config, .. } => config,
        }
    }
    fn blueprint_reference(&self) -> Option<Value> {
        match self {
            Self::Native { state, .. } => state.blueprint_reference(),
            Self::Monitor { .. } => None,
        }
    }
    fn monitor_reference(&self) -> Option<Value> {
        match self {
            Self::Native { state, .. } => state.monitor_reference(),
            Self::Monitor { state, .. } => Some(state.reference()),
        }
    }
    fn abandon(&mut self) {
        if let Self::Native { state, .. } = self {
            state.abandon();
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
struct State<S, N, B = S> {
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
}
impl<S: EffectJournalStorage, N: DigSource, B: EffectJournalStorage> State<S, N, B> {
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
        })
    }
    fn context(&mut self, write_enabled: bool, wall: Option<u64>) -> Result<OperationContext> {
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
            self.authority_tick.max(self.control.high_tick()),
            budget,
            false,
        );
        c.grants = self
            .grants
            .iter()
            .filter(|g| {
                (write_enabled && self.control.mode() == DigMode::Control)
                    || !matches!(g.capability, Capability::Plan | Capability::Designate)
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
fn perform<S, N, B, G, F>(
    state: &mut State<S, N, B>,
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
        Action::StartMonitor(_)
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

fn run_action<S, N, B, G, F>(
    state: &mut State<S, N, B>,
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
fn run_action_with_monitor<S, N, B, G, F, M>(
    state: &mut State<S, N, B>,
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
fn run_native_action<S, N, B, G, F>(
    state: &mut State<S, N, B>,
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
fn retain_blueprint<S: EffectJournalStorage, N: DigSource, B: EffectJournalStorage>(
    state: &State<S, N, B>,
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
    let native_effect_requested = matches!(&action, Ok(Action::Commit { .. } | Action::Cancel(..)));
    runtime::owned(op, move |control: RequestControl| {
        let mut retained = None;
        let mut retained_monitor = None;
        let mut native_handoff = None;
        let mut monitor_only = false;
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
            monitor_only = matches!(entry, Entry::Monitor { .. });
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
                return Err(cause);
            }
            let rendered = match entry {
                Entry::Native { state, .. } => {
                    let c =
                        match runtime::enabled().and_then(|enabled| state.context(enabled, wall)) {
                            Ok(c) => c,
                            Err(cause) => {
                                state.withdraw_monitor();
                                retained_monitor = state.monitor_reference();
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
                    run_action_with_monitor(
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
                    )
                }
                Entry::Monitor { state, .. } => {
                    let c = state.context(wall)?;
                    state.run_action(c, op, action, control.started, |_| {
                        runtime::boundary(&control, &config, false)
                    })
                }
            };
            retained = entry.blueprint_reference();
            retained_monitor = entry.monitor_reference();
            native_handoff = presentation::retained_handoff(&rendered);
            if let Err(e) = runtime::boundary(&control, &config, false) {
                entry.abandon();
                if let Entry::Native { state, .. } = entry {
                    state.withdraw_monitor();
                }
                retained_monitor = entry.monitor_reference();
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
                if let Some(reference) = retained {
                    result["blueprint"] = reference;
                }
                if let Some(reference) = retained_monitor {
                    result["blueprint_monitor"] = reference;
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
#[tool(
    name = "fortress.open_session",
    description = "Open mining control or retained blueprint offline/reconcile with region=[x,y,z,width,height]. For an intact retained monitor without native or parent custody, select monitor-offline (inspection) or monitor-recovery (inspection/local stop) and omit region. Monitor-only modes have Query authority only and never contact the game. Paths/source pins are operator configuration."
)]
pub async fn fortress_open_session(
    region: Option<String>,
    mode: Option<String>,
    max_wall_millis: Option<u64>,
    max_bytes: Option<u64>,
    max_output_tokens: Option<u32>,
) -> String {
    let requested_monitor_only = matches!(
        mode.as_deref(),
        Some("monitor-offline" | "monitor-recovery")
    );
    runtime::owned("fortress.open_session", move |control| {
        let mut retained=None;
        let mut retained_monitor=None;
        let result=(|| {
            let config=runtime::configuration()?;
            runtime::boundary(&control,&config,false)?;
            let (mode,region,monitor_only)=opening_selection(mode.as_deref(),region.as_deref(),&config)?;
            let budget=WorkBudget {max_wall_millis:max_wall_millis.map_or(10_000,|n|n),
                max_bytes:max_bytes.map_or(MAX_WORK_BYTES,|n|n),max_output_tokens:max_output_tokens.map_or(8192,|n|n),
                max_entities:if config.monitor.is_some(){monitor::MAX_ENTITIES}else{300},max_actions:1,max_game_ticks:0};
            let mut locked=lock()?;
            if locked.is_some() {return Err(error(ErrorCode::Conflict,"release the current mining session first"));}
            let seq=NEXT.try_update(Ordering::AcqRel,Ordering::Acquire,|n|(n<(1u64<<57)).then_some(n+1)).map_err(|_|exhausted())?;
            let id=SessionId::new((1u128<<127)|FAMILY|u128::from(seq));
            let write=mode==DigMode::Control && runtime::enabled()?;
            let mut c=OperationContext {session_id:id,request_id:RequestId::new(1),budget,cancellation_requested:false,
                anchor:StateAnchor {fortress_id:config.fortress(),cursor:ObservationCursor::ORIGIN,tick:GameTick(0),state_hash:Digest32::ZERO},
                grants:grants(config.fortress(),config.scope,write)};
            if mode==DigMode::Offline || monitor_only {c.grants.retain(|g|g.capability==Capability::Query);}
            let mut work=Work::new(&c,control.started)?;
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
                if mode==DigMode::Control {
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
            let mut state:State<_,_,PrivateBuildFile>=State::new(session,&work.take(&c,LOCAL_BYTES)?,&config)?;
            state.budget=budget;
            state.grants=c.grants.clone();
            state.blueprint_storage=parent_storage;
            if let Some(owned)=monitor_owner {
                state.monitor_recovery=Some(owned.recovery());
                state.monitor=Some(owned);
            }
            if let Some(owned)=parent_owner {
                state.blueprint_recovery=Some(owned.recovery());
                state.blueprint=Some(owned);
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
                "blueprint_monitor_configured":state.monitor_config.is_some()});
            if let Some(parent)=state.blueprint.as_mut() {
                let (full,records)=state.control.complete_inventory(&work.take(&c,COMPLETE_INVENTORY_BYTES)?)?;
                let progress=batch::verify(parent,&full,&records,&work.take(&c,batch::PUBLICATION_BYTES)?)?;
                result["blueprint"]=batch::display(parent,&progress,&full,0,8)?;
            }
            if let Some(proof)=state.monitor.as_mut() {
                proof.verify(&work.take(&c,monitor::FINAL_BYTES)?)?;
                result["blueprint_monitor"]=monitor::display(proof,false,true,0,16)?;
            }
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
                if let Some(recovery)=retained {result["blueprint"]=json!({"recovery":recovery,"inventory_verified":false});}
                if let Some(recovery)=retained_monitor {result["blueprint_monitor"]=json!({"recovery":recovery,"history_verified":false,
                    "fresh_sample_acquired":false,"sampled_excavation_condition_proven":false});}
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
    description = "Pass blueprint alone to retain a complete configured sparse floor project. After every original child is terminal Designated, pass monitor alone as closed JSON with blueprint_id, deadline_tick and optional stability/sample bounds to register read-only whole-blueprint completion monitoring. Otherwise pass the complete idempotency_key, observation_witness and allow_hidden_neighbors review request. No designation is dispatched during plan."
)]
pub async fn fortress_plan(
    session_id: String,
    idempotency_key: Option<String>,
    observation_witness: Option<String>,
    allow_hidden_neighbors: Option<bool>,
    blueprint: Option<String>,
    monitor: Option<String>,
) -> String {
    let action = plan_with_monitor_action(
        idempotency_key,
        observation_witness,
        allow_hidden_neighbors,
        blueprint,
        monitor,
    );
    with_session(session_id, "fortress.plan", None, action).await
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
    description = "Confirm the exact reviewed plan digest and review seal for one native commit attempt on the original preparation connection. Current policy, leases, capabilities, source and complete terrain witness are rechecked. Lost replies require exact-record recovery, never retry. Success proves designation configuration, not completed excavation."
)]
pub async fn fortress_commit(
    session_id: String,
    idempotency_key: String,
    plan_digest: String,
    review_seal: String,
) -> String {
    let action = key(&idempotency_key)
        .and_then(|_| Ok((digest(&plan_digest)?, digest(&review_seal)?)))
        .map(|(plan, seal)| Action::Commit {
            key: idempotency_key,
            plan,
            seal,
        });
    with_session(session_id, "fortress.commit", None, action).await
}
#[tool(
    name = "fortress.query",
    description = "Bounded local records, selection_tiles, plan_tiles, blueprint, blueprint_layout, blueprint_monitor or schema query. query is closed JSON. blueprint_monitor requires monitor_id, accepts expected_head, offset and limit, and shows durable historical progress and selected tile deficits. Query never acquires fresh terrain or proves current completion."
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
    description = "Pass monitor_id alone to acquire one paused whole-blueprint map capture, retain it durably and advance its fixed sampled stability goal. Or pass idempotency_key and plan_digest to reconcile one original native designation outcome. Terminal monitors/effects return historical evidence locally. No commit replay, polling loop or game-clock advance."
)]
pub async fn fortress_wait(
    session_id: String,
    idempotency_key: Option<String>,
    plan_digest: Option<String>,
    monitor_id: Option<String>,
    max_wall_millis: Option<u64>,
) -> String {
    with_session(
        session_id,
        "fortress.wait",
        max_wall_millis,
        wait_action(idempotency_key, plan_digest, monitor_id),
    )
    .await
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
        let mut monitor_only = false;
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
            let Entry::Native { state, config } = entry else {
                return Err(error(
                    ErrorCode::InternalInvariantViolation,
                    "unknown mining session owner",
                ));
            };
            let c = state.context(false, None)?;
            let view = if release {
                None
            } else {
                runtime::boundary(&control, config, false)?;
                let work = Work::new(&c, control.started)?;
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
                if let Some(reference) = retained {
                    result["blueprint"] = reference;
                }
                if let Some(reference) = retained_monitor {
                    result["blueprint_monitor"] = reference;
                }
                packet("fortress.cancel", result, None, None, None, None, None)
            }
        }
    })
    .await
}
#[tool(
    name = "fortress.cancel",
    description = "scope=monitor with monitor_id stops only the read-only excavation monitor and preserves its history. scope=blueprint with blueprint_id permanently stops original child advancement. scope=effect retires an exact native preparation. scope=session releases custody, with release_for_recovery=true for unresolved effects. None of these undo excavation or erase history."
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
        .instructions("Unadmitted development mining control. Default policy requires an unavailable game checkpoint. Explicit operator disposable-fortress policy permits consideration. Optional operator blueprint custody retains one complete sparse floor project: plan with blueprint alone, then observe/prepare/review/confirm each original next region. Only terminal Designated receipts unlock further children and whole-blueprint monitoring. Register a configured monitor through plan, sample it through wait with monitor_id, inspect its historical progress through query, and stop it with cancel scope=monitor. One independent map capture covers the whole original mask; completion requires visible dry floors with cleared designations over the fixed shared sampled stability window. Source pins do not prove shared process identity or mining causality. Never replace uncertain work or extend the original deadline. Blueprint stop preserves native recovery and does not stop read-only monitoring. Offline/reconcile reopening restores no commit permit. No automatic game-clock advancement, checkpoint or restore.")
        .build();
    crate::run_modern_stdio(server);
}
#[cfg(test)]
mod batch_tests;
#[cfg(test)]
mod tests;
