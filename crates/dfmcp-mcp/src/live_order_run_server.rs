#![forbid(unsafe_code)]
//! Unadmitted fortress-bound condition runs through the existing eleven-tool waist.
use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
use dfmcp_adapter::order_run::journal::{
    OrderRunBinding, OrderRunEntry, OrderRunJournal, OrderRunMode, OrderRunState, OrderRunView,
};
use dfmcp_adapter::order_run::private_file::{PrivateOrderRunFile, open_private_order_journal};
use dfmcp_adapter::order_run::rpc::{
    CONNECT_RESERVE_BYTES, OrderRunRpc, OrderRunSource, OrderRunTcp, authorize, authorize_plan,
};
use dfmcp_adapter::order_run::{
    FortressIdentity, OrderCapture, OrderPredicate, OrderRunPlan, OrderRunSpec,
};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, Digest32, ErrorCode, GameTick, ObservationCursor,
    OperationContext, RequestId, Result, RiskTier, SessionId, StateAnchor, WorkBudget,
};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use serde::Deserialize;
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};
mod native;
mod presentation;
use native::CheckedSource;
use presentation::{
    BASE_OUTPUT, Cursors, Filter, MAX_PAGE, ROW_OUTPUT, capture, digest, entry, failure, mode_name,
    packet, summary,
};

const MAX_BYTES: u64 = 64 * 1024 * 1024;
const VIEW_RESERVE: u64 = 3 * 1024 * 1024;
const FAMILY: u128 = 14u128 << 57;
static NEXT: AtomicU64 = AtomicU64::new(1);
static SESSION: Mutex<Option<State<PrivateOrderRunFile>>> = Mutex::new(None);
fn error(code: ErrorCode, message: &str) -> dfmcp_core::DfmcpError {
    dfmcp_core::DfmcpError::new(code, message)
}
fn budget_error() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "complete response and conditional-run work do not fit the bounded request",
    )
}
fn unbound(operation: &str, cause: &dfmcp_core::DfmcpError) -> String {
    packet(operation, failure(cause), None, None, None)
}

struct State<S> {
    id: SessionId,
    request: u128,
    budget: WorkBudget,
    grants: Vec<CapabilityGrant>,
    high_tick: u64,
    journal: OrderRunJournal<S>,
    selected: Option<OrderCapture>,
    cursors: Cursors,
}
impl<S: EffectJournalStorage> State<S> {
    fn context(&mut self, clock: bool, cancelled: bool) -> Result<OperationContext> {
        self.request = self.request.checked_add(1).ok_or_else(budget_error)?;
        Ok(OperationContext {
            session_id: self.id,
            request_id: RequestId::new(self.request),
            anchor: StateAnchor {
                fortress_id: self.journal.binding().fortress().fortress_id(),
                cursor: ObservationCursor::ORIGIN,
                tick: GameTick(self.high_tick),
                state_hash: Digest32::ZERO,
            },
            budget: self.budget,
            grants: self
                .grants
                .iter()
                .filter(|g| {
                    clock || !matches!(g.capability, Capability::ControlClock | Capability::Plan)
                })
                .cloned()
                .collect(),
            cancellation_requested: cancelled,
        })
    }
    fn control(&self, c: &OperationContext) -> Result<()> {
        if self.journal.mode() != OrderRunMode::Control {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "recovery mode cannot become conditional clock control",
            ));
        }
        authorize(c, self.journal.binding().fortress(), true)
    }
    fn advance_evidence_floor(&mut self, view: &OrderRunView) {
        for e in &view.entries {
            self.high_tick = self.high_tick.max(e.plan().before().tick());
            if let Some(tick) = e.native().and_then(|n| n.observed_tick()) {
                self.high_tick = self.high_tick.max(tick);
            }
        }
    }
}
#[derive(Default)]
struct Limits {
    wall: Option<u64>,
    bytes: Option<u64>,
    tokens: Option<u32>,
    ticks: Option<u64>,
}
struct Work {
    deadline: Instant,
    bytes: u64,
}
impl Work {
    fn split(
        mut c: OperationContext,
        limits: Limits,
        rows: usize,
    ) -> Result<(OperationContext, Self, u64)> {
        if let Some(n) = limits.wall {
            c.budget.max_wall_millis = n.min(c.budget.max_wall_millis);
        }
        if let Some(n) = limits.bytes {
            c.budget.max_bytes = n.min(c.budget.max_bytes);
        }
        if let Some(n) = limits.tokens {
            c.budget.max_output_tokens = n.min(c.budget.max_output_tokens);
        }
        if let Some(n) = limits.ticks {
            c.budget.max_game_ticks = n.min(c.budget.max_game_ticks);
        }
        c.budget.validate()?;
        if rows > MAX_PAGE {
            return Err(budget_error());
        }
        let output = BASE_OUTPUT + ROW_OUTPUT * rows as u64;
        if u64::from(c.budget.max_output_tokens) * 4 < output {
            return Err(budget_error());
        }
        let bytes = c
            .budget
            .max_bytes
            .checked_sub(output + 2 * VIEW_RESERVE)
            .filter(|n| *n > 0)
            .ok_or_else(budget_error)?;
        let deadline = Instant::now()
            .checked_add(Duration::from_millis(c.budget.max_wall_millis))
            .ok_or_else(budget_error)?;
        Ok((c, Self { deadline, bytes }, output))
    }
    fn context(&self, c: &OperationContext) -> Result<OperationContext> {
        let remaining = self
            .deadline
            .checked_duration_since(Instant::now())
            .filter(|t| *t >= Duration::from_millis(1))
            .ok_or_else(budget_error)?;
        let mut out = c.clone();
        out.budget.max_wall_millis =
            u64::try_from(remaining.as_millis()).map_err(|_| budget_error())?;
        out.budget.max_bytes = self.bytes;
        Ok(out)
    }
    fn view_context(&self, c: &OperationContext) -> Result<OperationContext> {
        let mut out = self.context(c)?;
        out.budget.max_bytes = VIEW_RESERVE;
        Ok(out)
    }
    fn connect<N, F>(&mut self, c: &OperationContext, f: F) -> Result<N>
    where
        F: FnOnce(&OperationContext) -> Result<N>,
    {
        let connect_context = self.context(c)?;
        self.bytes = self
            .bytes
            .checked_sub(CONNECT_RESERVE_BYTES)
            .filter(|n| *n > 0)
            .ok_or_else(budget_error)?;
        f(&connect_context)
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Condition {
    order_id: u32,
    predicate: String,
    game_ticks: u32,
    wall_millis: u32,
    threshold: Option<u32>,
    stable_samples: Option<u32>,
    interval_ticks: Option<u32>,
}
impl Condition {
    fn parse(raw: &str) -> Result<Self> {
        if raw.len() > 2048 {
            return Err(error(ErrorCode::InvalidRequest, "condition exceeds 2 KiB"));
        }
        let value: Self = serde_json::from_str(raw).map_err(|_| {
            error(
                ErrorCode::InvalidRequest,
                "condition must be closed typed JSON",
            )
        })?;
        value.spec()?;
        if value.order_id > i32::MAX as u32 {
            return Err(error(
                ErrorCode::InvalidRequest,
                "native order ID out of range",
            ));
        }
        Ok(value)
    }
    fn spec(&self) -> Result<OrderRunSpec> {
        let threshold = self.threshold.unwrap_or(0);
        let code = match self.predicate.as_str() {
            "approved" => 1,
            "active" => 2,
            "remaining_at_most" => 3,
            _ => {
                return Err(error(
                    ErrorCode::InvalidRequest,
                    "unknown conditional-run predicate",
                ));
            }
        };
        OrderRunSpec::new(
            self.game_ticks,
            self.wall_millis,
            OrderPredicate::from_code(code, threshold)?,
            self.stable_samples.unwrap_or(1),
            self.interval_ticks.unwrap_or(1),
        )
    }
}
enum Action {
    Observe(u32),
    Plan {
        key: String,
        witness: Digest32,
        condition: Condition,
    },
    Commit {
        key: String,
        plan: Digest32,
        confirm: bool,
    },
    Wait {
        key: String,
        plan: Digest32,
    },
    Cancel {
        key: String,
        plan: Digest32,
    },
    Explain {
        key: String,
        plan: Digest32,
    },
    Query {
        filter: Filter,
        limit: usize,
        continuation: Option<String>,
    },
    Doctor,
    Unavailable,
}
fn record(view: &OrderRunView, key: &str, plan: Digest32) -> Result<OrderRunEntry> {
    if key.is_empty() || key.len() > 128 {
        return Err(error(
            ErrorCode::InvalidRequest,
            "invalid conditional-run key",
        ));
    }
    let value = view
        .entries
        .iter()
        .find(|e| e.plan().key() == key)
        .ok_or_else(|| error(ErrorCode::InvalidRequest, "conditional-run key not found"))?;
    if value.plan().digest() != plan {
        return Err(error(
            ErrorCode::Conflict,
            "conditional-run key and digest differ",
        ));
    }
    Ok(value.clone())
}
fn terminal(e: &OrderRunEntry) -> bool {
    matches!(
        e.state(),
        OrderRunState::Terminal | OrderRunState::CancelledBeforeDispatch
    )
}
fn result_entry(e: &OrderRunEntry) -> Value {
    json!({"ok":true,"effect":entry(e),"goal_completion_proven":false,"current_pause_unproved":true})
}

/// Actual dispatcher with an injected connection factory for regression tests.
/// Local history and pre-dispatch cancellation never invoke that factory.
fn perform<S, N, F>(
    state: &mut State<S>,
    c: &OperationContext,
    work: &mut Work,
    view: &OrderRunView,
    action: Action,
    connect: F,
) -> Result<Value>
where
    S: EffectJournalStorage,
    N: OrderRunSource,
    F: FnOnce(&OperationContext) -> Result<N>,
{
    match action {
        Action::Observe(id) => {
            state.selected = None;
            if state.journal.mode() == OrderRunMode::Offline {
                return Err(error(
                    ErrorCode::CapabilityDenied,
                    "offline mode has no native observation",
                ));
            }
            if id > i32::MAX as u32 {
                return Err(error(
                    ErrorCode::InvalidRequest,
                    "native order ID out of range",
                ));
            }
            let mut source = work.connect(c, connect)?;
            let value = state.journal.observe(&mut source, id, &work.context(c)?)?;
            state.high_tick = state.high_tick.max(value.tick());
            state.selected = Some(value.clone());
            Ok(json!({"ok":true,"observation":capture(&value),"game_mutation_dispatched":false}))
        }
        Action::Plan {
            key,
            witness,
            condition,
        } => {
            state.control(c)?;
            let spec = condition.spec()?;
            if let Some(old) = view.entries.iter().find(|e| e.plan().key() == key) {
                if old.plan().before().witness() != witness
                    || old.plan().spec() != spec
                    || old.plan().before().order_id() != condition.order_id
                {
                    return Err(error(
                        ErrorCode::Conflict,
                        "conditional-run key already binds another observation or condition",
                    ));
                }
                authorize_plan(c, old.plan())?;
                return Ok(result_entry(old));
            }
            let before = state
                .selected
                .clone()
                .filter(|v| v.witness() == witness && v.order_id() == condition.order_id)
                .ok_or_else(|| {
                    error(
                        ErrorCode::StaleAnchor,
                        "observe the selected order and use its exact witness before planning",
                    )
                })?;
            let plan = OrderRunPlan::new(&key, spec, before)?;
            authorize_plan(c, &plan)?;
            state.selected = None;
            let mut source = work.connect(c, connect)?;
            let e = state
                .journal
                .prepare(&mut source, &plan, &work.context(c)?)?;
            Ok(result_entry(&e))
        }
        Action::Commit { key, plan, confirm } => {
            state.control(c)?;
            if !confirm {
                return Err(error(
                    ErrorCode::CapabilityDenied,
                    "commit requires explicit confirmation of the exact sealed plan",
                ));
            }
            let old = record(view, &key, plan)?;
            authorize_plan(c, old.plan())?;
            if old.settled() {
                return Ok(result_entry(&old));
            }
            if old.state() != OrderRunState::Prepared {
                return Err(error(
                    ErrorCode::EffectIndeterminate,
                    "run is not dispatchable; query or cancel instead",
                ));
            }
            state.selected = None;
            let mut source = work.connect(c, connect)?;
            let e = state
                .journal
                .commit(&mut source, &key, plan, &work.context(c)?)?;
            Ok(result_entry(&e))
        }
        Action::Wait { key, plan } => {
            let old = record(view, &key, plan)?;
            if terminal(&old) {
                return Ok(result_entry(&old));
            }
            if state.journal.mode() == OrderRunMode::Offline {
                return Err(error(
                    ErrorCode::CapabilityDenied,
                    "offline pending evidence requires explicit recover-mode reopen",
                ));
            }
            state.selected = None;
            let mut source = work.connect(c, connect)?;
            let e = state
                .journal
                .reconcile(&mut source, &key, plan, &work.context(c)?)?;
            Ok(result_entry(&e))
        }
        Action::Cancel { key, plan } => {
            state.control(c)?;
            let old = record(view, &key, plan)?;
            if terminal(&old) {
                return Ok(result_entry(&old));
            }
            state.selected = None;
            if matches!(old.state(), OrderRunState::Intent | OrderRunState::Prepared) {
                let e = state
                    .journal
                    .cancel::<N>(None, &key, plan, &work.context(c)?)?;
                return Ok(result_entry(&e));
            }
            let mut source = work.connect(c, connect)?;
            let e = state
                .journal
                .cancel(Some(&mut source), &key, plan, &work.context(c)?)?;
            Ok(result_entry(&e))
        }
        Action::Explain { key, plan } => Ok(result_entry(&record(view, &key, plan)?)),
        Action::Query {
            filter,
            limit,
            continuation,
        } => {
            if !(1..=MAX_PAGE).contains(&limit) {
                return Err(error(
                    ErrorCode::InvalidRequest,
                    "record page size must be 1..8",
                ));
            }
            let rows = view
                .entries
                .iter()
                .filter(|e| filter.matches(e))
                .collect::<Vec<_>>();
            let offset = continuation
                .as_deref()
                .map(|s| state.cursors.resolve(s, state.id, view, filter, limit))
                .transpose()?
                .unwrap_or(0);
            if offset > rows.len() {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "continuation offset is outside the retained selection",
                ));
            }
            let end = (offset + limit).min(rows.len());
            let next = if end < rows.len() {
                Some(state.cursors.issue(state.id, view, filter, limit, end)?)
            } else {
                None
            };
            Ok(
                json!({"ok":true,"records":rows[offset..end].iter().map(|e|entry(e)).collect::<Vec<_>>(),"state":filter.name(),
                "matching_records":rows.len(),"offset":offset,"continuation":next,"last_page":end==rows.len(),
                "complete_matching_set_in_this_response":offset==0&&end==rows.len(),"journal":summary(view),"native_calls":0}),
            )
        }
        Action::Doctor => Ok(
            json!({"ok":true,"journal":summary(view),"native_calls":0,"runtime_admitted":false,
            "world_state_health_checked":false,"current_pause_unproved":true}),
        ),
        Action::Unavailable => Err(error(
            ErrorCode::CapabilityDenied,
            "conditional runs do not implement game checkpoint or restore",
        )),
    }
}
struct Dispatch<'a> {
    operation: &'a str,
    limits: Limits,
    rows: usize,
    action: Result<Action>,
}
fn run_action<S, N, F>(
    state: &mut State<S>,
    context: OperationContext,
    request: Dispatch<'_>,
    connect: F,
) -> String
where
    S: EffectJournalStorage,
    N: OrderRunSource,
    F: FnOnce(&OperationContext) -> Result<N>,
{
    let Dispatch {
        operation,
        limits,
        rows,
        action,
    } = request;
    let mode = state.journal.mode();
    let (display, mut work, reserve) = match Work::split(context.clone(), limits, rows) {
        Ok(v) => v,
        Err(e) => return packet(operation, failure(&e), Some(&context), Some(mode), None),
    };
    let before = match work
        .view_context(&display)
        .and_then(|c| state.journal.view(&c))
    {
        Ok(v) => v,
        Err(e) => return packet(operation, failure(&e), Some(&display), Some(mode), None),
    };
    let outcome = work
        .context(&display)
        .and_then(|c| action.and_then(|a| perform(state, &c, &mut work, &before, a, connect)));
    let after = work
        .view_context(&display)
        .and_then(|c| state.journal.view(&c));
    let (result, view) = match after {
        Ok(v) => {
            state.advance_evidence_floor(&v);
            (outcome.unwrap_or_else(|e| failure(&e)), Some(v))
        }
        Err(e) => (
            json!({"ok":false,"post_operation_evidence_unavailable":true,"error":failure(&e),
            "effect_may_have_dispatched":matches!(operation,"fortress.commit"|"fortress.cancel"),"retry_commit":false}),
            None,
        ),
    };
    let output = packet(operation, result, Some(&display), Some(mode), view.as_ref());
    if output.len() as u64 <= reserve {
        return output;
    }
    let cause = error(
        if matches!(operation, "fortress.commit" | "fortress.cancel") {
            ErrorCode::EffectIndeterminate
        } else {
            ErrorCode::BudgetExceeded
        },
        "response exceeded reserved bound; inspect durable evidence before further control",
    );
    packet(
        operation,
        failure(&cause),
        Some(&display),
        Some(mode),
        view.as_ref(),
    )
}

const ALLOWED: [&str; 7] = [
    "DFMCP_ALLOW_UNADMITTED_ORDER_RUN_V1_14",
    "DFMCP_ORDER_RUN_TOKEN",
    "DFMCP_ORDER_RUN_ENDPOINT",
    "DFMCP_ORDER_RUN_JOURNAL",
    "DFMCP_ORDER_RUN_ALLOW_CLOCK",
    "DFMCP_ORDER_RUN_WORLD_FOLDER",
    "DFMCP_ORDER_RUN_SITE_ID",
];
fn environment_contract(
    opt: Option<&str>,
    clock: Option<&str>,
    keys: &[String],
    admitted: bool,
) -> Result<()> {
    if opt != Some("1")
        || clock.is_some_and(|s| s != "1")
        || admitted
        || keys
            .iter()
            .any(|s| s.starts_with("DFMCP_") && !ALLOWED.contains(&s.as_str()))
    {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "order-run/1.14 requires exact development opt-in and refuses admission or foreign DFMCP state",
        ));
    }
    Ok(())
}
fn clock_enabled() -> bool {
    std::env::var("DFMCP_ORDER_RUN_ALLOW_CLOCK").ok().as_deref() == Some("1")
}
fn validate_environment() -> Result<()> {
    let opt = std::env::var("DFMCP_ALLOW_UNADMITTED_ORDER_RUN_V1_14");
    let clock = std::env::var("DFMCP_ORDER_RUN_ALLOW_CLOCK");
    if matches!(clock, Err(std::env::VarError::NotUnicode(_))) {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "clock opt-in must be UTF-8",
        ));
    }
    environment_contract(
        opt.ok().as_deref(),
        clock.ok().as_deref(),
        &std::env::vars_os()
            .map(|(k, _)| k.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        crate::admission::current_admission_provenance().is_some(),
    )
}
fn runtime_io() -> Result<()> {
    let cx = asupersync::Cx::current().ok_or_else(|| {
        error(
            ErrorCode::CapabilityDenied,
            "conditional-run I/O requires owned runtime context",
        )
    })?;
    cx.checkpoint().map_err(|_| {
        error(
            ErrorCode::CancellationRequested,
            "conditional-run request cancelled",
        )
    })?;
    if cx.io().is_none() {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "inherited runtime context denies I/O",
        ));
    }
    Ok(())
}
fn native_gate(clock: bool) -> Result<()> {
    runtime_io()?;
    validate_environment()?;
    if clock && !clock_enabled() {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "operator conditional clock permission revoked",
        ));
    }
    Ok(())
}
fn configured(name: &str, max: usize) -> Result<String> {
    let raw = std::env::var(name).map_err(|_| {
        error(
            ErrorCode::CapabilityDenied,
            "required operator order-run configuration absent or not UTF-8",
        )
    })?;
    if raw.is_empty() || raw.len() > max || raw.contains('\0') {
        return Err(error(
            ErrorCode::InvalidRequest,
            "operator configuration exceeds its bound",
        ));
    }
    Ok(raw)
}
fn endpoint() -> Result<SocketAddr> {
    let raw = match std::env::var("DFMCP_ORDER_RUN_ENDPOINT") {
        Ok(v) => v,
        Err(std::env::VarError::NotPresent) => "127.0.0.1:5000".to_owned(),
        _ => {
            return Err(error(
                ErrorCode::InvalidRequest,
                "order-run endpoint must be numeric loopback",
            ));
        }
    };
    dfmcp_adapter::parse_loopback_endpoint(&raw)
}
fn bound_native_gate(clock: bool, fortress: &FortressIdentity) -> Result<()> {
    native_gate(clock)?;
    if &selected_fortress()? != fortress {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "operator fortress selection changed; reopen session",
        ));
    }
    Ok(())
}
fn selected_fortress() -> Result<FortressIdentity> {
    let folder = configured("DFMCP_ORDER_RUN_WORLD_FOLDER", 512)?;
    let raw = configured("DFMCP_ORDER_RUN_SITE_ID", 10)?;
    let site = raw.parse::<u32>().map_err(|_| {
        error(
            ErrorCode::InvalidRequest,
            "site ID must be canonical nonnegative decimal",
        )
    })?;
    if site.to_string() != raw {
        return Err(error(
            ErrorCode::InvalidRequest,
            "site ID must be canonical decimal",
        ));
    }
    FortressIdentity::new(&folder, site)
}
fn parse_mode(raw: &str) -> Result<OrderRunMode> {
    match raw {
        "control" => Ok(OrderRunMode::Control),
        "recover" => Ok(OrderRunMode::Recover),
        "offline" => Ok(OrderRunMode::Offline),
        _ => Err(error(
            ErrorCode::InvalidRequest,
            "mode must be offline, recover or control",
        )),
    }
}
fn grants(
    mode: OrderRunMode,
    fortress: &FortressIdentity,
    clock: bool,
) -> Result<Vec<CapabilityGrant>> {
    if mode == OrderRunMode::Control && !clock {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "control mode requires operator DFMCP_ORDER_RUN_ALLOW_CLOCK=1",
        ));
    }
    let caps = if mode == OrderRunMode::Control {
        vec![
            Capability::Query,
            Capability::Plan,
            Capability::ControlClock,
        ]
    } else {
        vec![Capability::Query]
    };
    Ok(caps
        .into_iter()
        .map(|capability| CapabilityGrant {
            capability,
            scope: CapabilityScope {
                fortress_id: Some(fortress.fortress_id()),
                ..CapabilityScope::default()
            },
            max_risk: if capability == Capability::Query {
                RiskTier::ReadOnly
            } else {
                RiskTier::Guarded
            },
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect())
}
type Native = CheckedSource<OrderRunRpc<OrderRunTcp>>;
fn connect_native(c: &OperationContext, binding: &OrderRunBinding) -> Result<Native> {
    bound_native_gate(false, binding.fortress())?;
    let endpoint = endpoint()?;
    if endpoint != binding.endpoint() {
        return Err(error(
            ErrorCode::StaleAnchor,
            "configured endpoint differs from bound journal",
        ));
    }
    let token = configured("DFMCP_ORDER_RUN_TOKEN", 256)?.into_bytes();
    let mut nonce = c.session_id.get().to_be_bytes().to_vec();
    nonce.extend_from_slice(&c.request_id.get().to_be_bytes());
    let inner = OrderRunRpc::connect(endpoint, token, nonce, binding.fortress().clone(), c)?;
    Ok(CheckedSource {
        inner,
        check: bound_native_gate,
    })
}
fn session_id(raw: &str) -> Result<SessionId> {
    if raw.len() != 32
        || !raw
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "invalid order-run session ID",
        ));
    }
    let number = u128::from_str_radix(raw, 16)
        .map_err(|_| error(ErrorCode::InvalidRequest, "invalid session ID"))?;
    let id = SessionId::new(number);
    if id.get() != number
        || !id.is_process_scoped_live()
        || (number & ((1u128 << 62) - 1)) >> 57 != 14
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "not an order-run session ID",
        ));
    }
    Ok(id)
}
fn session_lock() -> Result<MutexGuard<'static, Option<State<PrivateOrderRunFile>>>> {
    match SESSION.try_lock() {
        Ok(v) => Ok(v),
        Err(TryLockError::WouldBlock) => Err(error(
            ErrorCode::BudgetExceeded,
            "another bounded request owns this session",
        )),
        Err(TryLockError::Poisoned(_)) => Err(error(
            ErrorCode::InternalInvariantViolation,
            "conditional-run session poisoned; recover the journal",
        )),
    }
}
fn with_session<F: FnOnce() -> Result<Action>>(
    raw: String,
    operation: &str,
    limits: Limits,
    rows: usize,
    action: F,
) -> String {
    let result = (|| -> Result<String> {
        runtime_io()?;
        validate_environment()?;
        let id = session_id(&raw)?;
        let mut guard = session_lock()?;
        let state = guard.as_mut().filter(|s| s.id == id).ok_or_else(|| {
            error(
                ErrorCode::SessionNotFound,
                "conditional-run session absent or closed",
            )
        })?;
        let c = state.context(clock_enabled(), false)?;
        let binding = state.journal.binding().clone();
        Ok(run_action(
            state,
            c,
            Dispatch {
                operation,
                limits,
                rows,
                action: action(),
            },
            |c| connect_native(c, &binding),
        ))
    })();
    result.unwrap_or_else(|e| unbound(operation, &e))
}

#[tool(
    name = "fortress.open_session",
    description = "Open isolated fortress-bound order-run/1.14. Default offline opens an existing read-only binary Rust journal without endpoint/token reads. Recover permits receipt queries only; control requires separate operator clock permission. Fortress folder/site, journal path and credentials are operator configuration, never tool arguments."
)]
pub fn fortress_open_session(
    mode: Option<String>,
    max_wall_millis: Option<u64>,
    max_bytes: Option<u64>,
    max_output_tokens: Option<u32>,
    max_game_ticks: Option<u64>,
) -> String {
    let result = (|| -> Result<String> {
        runtime_io()?;
        validate_environment()?;
        let mode = parse_mode(mode.as_deref().unwrap_or("offline"))?;
        let mut guard = session_lock()?;
        if guard.is_some() {
            return Err(error(
                ErrorCode::Conflict,
                "release the retained conditional-run session first",
            ));
        }
        let fortress = selected_fortress()?;
        let grants = grants(mode, &fortress, clock_enabled())?;
        let path = PathBuf::from(configured("DFMCP_ORDER_RUN_JOURNAL", 4096)?);
        let budget = WorkBudget {
            max_wall_millis: max_wall_millis.unwrap_or(5000),
            max_bytes: max_bytes.unwrap_or(32 * 1024 * 1024),
            max_output_tokens: max_output_tokens.unwrap_or(65536),
            max_game_ticks: max_game_ticks.unwrap_or(1200),
            max_entities: 256,
            max_actions: 1,
        };
        budget.validate()?;
        if budget.max_wall_millis > 60000
            || budget.max_bytes > MAX_BYTES
            || budget.max_output_tokens > 65536
            || budget.max_game_ticks > 1200
        {
            return Err(budget_error());
        }
        let serial = NEXT
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < (1u64 << 57)).then_some(n + 1)
            })
            .map_err(|_| budget_error())?;
        let id = SessionId::new((1u128 << 127) | FAMILY | u128::from(serial));
        let c = OperationContext {
            session_id: id,
            request_id: RequestId::new(1),
            anchor: StateAnchor {
                fortress_id: fortress.fortress_id(),
                cursor: ObservationCursor::ORIGIN,
                tick: GameTick(0),
                state_hash: Digest32::ZERO,
            },
            budget,
            grants: grants.clone(),
            cancellation_requested: false,
        };
        let (display, mut work, reserve) = Work::split(c, Limits::default(), 0)?;
        // Recovery branches before any credential/endpoint read or native source construction.
        let binding = if mode == OrderRunMode::Control {
            native_gate(true)?;
            let native = work.connect(&display, |c| {
                OrderRunRpc::connect(
                    endpoint()?,
                    configured("DFMCP_ORDER_RUN_TOKEN", 256)?.into_bytes(),
                    id.get().to_be_bytes().to_vec(),
                    fortress.clone(),
                    c,
                )
            })?;
            Some(OrderRunBinding::from_source(&native)?)
        } else {
            None
        };
        native_gate(mode == OrderRunMode::Control)?;
        let journal =
            open_private_order_journal(&path, &fortress, &work.context(&display)?, mode, binding)?;
        let mut state = State {
            id,
            request: 1,
            budget,
            grants,
            high_tick: 0,
            journal,
            selected: None,
            cursors: Cursors::default(),
        };
        let view = state.journal.view(&work.view_context(&display)?)?;
        state.advance_evidence_floor(&view);
        let out = packet(
            "fortress.open_session",
            json!({"ok":true,"session_id":id.to_string(),"mode":mode_name(mode),"journal":summary(&view),
            "fortress_id":fortress.fortress_id().to_string(),"world_folder":fortress.folder(),"site_id":fortress.site(),
            "native_capture_acquired":false,"retained_native_connection":false,"capabilities":state.grants.iter().map(|g|g.capability.as_str()).collect::<Vec<_>>(),
            "next":{"tool":"fortress.query","arguments":{"session_id":id.to_string(),"state":"pending","limit":2}}}),
            Some(&display),
            Some(mode),
            Some(&view),
        );
        if out.len() as u64 > reserve {
            return Err(budget_error());
        }
        work.context(&display)?;
        native_gate(mode == OrderRunMode::Control)?;
        *guard = Some(state);
        Ok(out)
    })();
    result.unwrap_or_else(|e| unbound("fortress.open_session", &e))
}
#[tool(
    name = "fortress.observe",
    description = "Acquire one exact fortress-bound native order capture without advancing time. Failed refresh clears prior selection. Use observation.witness, not the coordination-root hash, for preparation."
)]
pub fn fortress_observe(session_id: String, native_order_id: u32) -> String {
    with_session(session_id, "fortress.observe", Limits::default(), 1, || {
        Ok(Action::Observe(native_order_id))
    })
}
#[tool(
    name = "fortress.plan",
    description = "Seal a finite conditional run from the selected exact witness. condition is closed JSON: order_id, predicate (approved/active/remaining_at_most), game_ticks, wall_millis, optional threshold, stable_samples, interval_ticks. Requires guarded clock and Plan authority; already-true goals are refused. Preparation never unpauses."
)]
pub fn fortress_plan(
    session_id: String,
    idempotency_key: String,
    expected_witness: String,
    condition: String,
) -> String {
    with_session(session_id, "fortress.plan", Limits::default(), 1, || {
        Ok(Action::Plan {
            key: idempotency_key,
            witness: digest(&expected_witness)?,
            condition: Condition::parse(&condition)?,
        })
    })
}
#[tool(
    name = "fortress.commit",
    description = "Commit the exact reviewed conditional-run plan once with confirm=true. Sync dispatch before unpause. Missing replies or records are unknown, never retry permission. Predicate evidence, historical pause and produced goods remain distinct."
)]
pub fn fortress_commit(
    session_id: String,
    idempotency_key: String,
    plan_digest: String,
    confirm: bool,
) -> String {
    with_session(session_id, "fortress.commit", Limits::default(), 1, || {
        Ok(Action::Commit {
            key: idempotency_key,
            plan: digest(&plan_digest)?,
            confirm,
        })
    })
}
#[tool(
    name = "fortress.wait",
    description = "Perform one receipt query and durably retain validated predicate/stop evidence. No polling, unpause, re-prepare, commit retry or budget extension. Stored terminal evidence needs no native connection; offline unresolved work requires recover-mode reopen."
)]
pub fn fortress_wait(
    session_id: String,
    idempotency_key: String,
    plan_digest: String,
    max_wall_millis: Option<u64>,
) -> String {
    with_session(
        session_id,
        "fortress.wait",
        Limits {
            wall: max_wall_millis,
            ..Limits::default()
        },
        1,
        || {
            Ok(Action::Wait {
                key: idempotency_key,
                plan: digest(&plan_digest)?,
            })
        },
    )
}
#[tool(
    name = "fortress.query",
    description = "Discover durable conditional-run records without native calls. Filter all/pending/unresolved/terminal; limit 1..8. Continuations bind session, exact journal head, filter and page size. Historical receipts do not prove current pause or goods produced."
)]
pub fn fortress_query(
    session_id: String,
    state: Option<String>,
    limit: Option<u32>,
    continuation: Option<String>,
    max_bytes: Option<u64>,
    max_output_tokens: Option<u32>,
) -> String {
    let limit = limit.unwrap_or(2) as usize;
    with_session(
        session_id,
        "fortress.query",
        Limits {
            bytes: max_bytes,
            tokens: max_output_tokens,
            ..Limits::default()
        },
        limit,
        || {
            Ok(Action::Query {
                filter: Filter::parse(state.as_deref().unwrap_or("all"))?,
                limit,
                continuation,
            })
        },
    )
}
#[tool(
    name = "fortress.explain",
    description = "Inspect one exact durable conditional-run plan and native receipt without contacting the game. Evidence remains historical."
)]
pub fn fortress_explain(
    session_id: String,
    idempotency_key: String,
    plan_digest: String,
) -> String {
    with_session(session_id, "fortress.explain", Limits::default(), 1, || {
        Ok(Action::Explain {
            key: idempotency_key,
            plan: digest(&plan_digest)?,
        })
    })
}
fn close(raw: String, release: bool) -> String {
    let result = (|| -> Result<String> {
        let id = session_id(&raw)?;
        let mut guard = session_lock()?;
        let state = guard
            .as_mut()
            .filter(|s| s.id == id)
            .ok_or_else(|| error(ErrorCode::SessionNotFound, "conditional-run session absent"))?;
        let mode = state.journal.mode();
        let c = if release {
            state.context(false, false).ok()
        } else {
            Some(state.context(false, false)?)
        };
        if !release {
            runtime_io()?;
            validate_environment()?;
            if state
                .journal
                .view(c.as_ref().ok_or_else(budget_error)?)?
                .entries
                .iter()
                .any(|e| !e.settled())
            {
                return Err(error(
                    ErrorCode::EffectIndeterminate,
                    "pending runs remain; cancel/reconcile or explicitly release_for_recovery",
                ));
            }
        }
        let out = packet(
            "fortress.cancel",
            json!({"ok":true,"scope":"session","closed":true,"released_for_recovery":release,
            "native_calls":0,"effects_cancelled":false,"journal_erased":false,"native_quiescence_proven":false}),
            c.as_ref(),
            Some(mode),
            None,
        );
        drop(guard.take());
        Ok(out)
    })();
    result.unwrap_or_else(|e| unbound("fortress.cancel", &e))
}
#[tool(
    name = "fortress.cancel",
    description = "scope=effect with exact key/digest retires undispatched intent or durably requests native safety pause; only that pause may repeat. scope=session without key/digest closes only settled custody unless release_for_recovery=true. Release never cancels game effects, erases evidence or proves native quiescence."
)]
pub fn fortress_cancel(
    session_id: String,
    scope: String,
    idempotency_key: Option<String>,
    plan_digest: Option<String>,
    release_for_recovery: Option<bool>,
) -> String {
    match (
        scope.as_str(),
        idempotency_key,
        plan_digest,
        release_for_recovery.unwrap_or(false),
    ) {
        ("session", None, None, release) => close(session_id, release),
        ("effect", Some(key), Some(plan), false) => {
            with_session(session_id, "fortress.cancel", Limits::default(), 1, || {
                Ok(Action::Cancel {
                    key,
                    plan: digest(&plan)?,
                })
            })
        }
        _ => unbound(
            "fortress.cancel",
            &error(
                ErrorCode::InvalidRequest,
                "cancel requires effect key/digest or session release, not both",
            ),
        ),
    }
}
#[tool(
    name = "fortress.doctor",
    description = "Inspect verified local journal counts without native calls. This is not live-world health, produced-goods proof, or compatibility admission."
)]
pub fn fortress_doctor(session_id: String) -> String {
    with_session(session_id, "fortress.doctor", Limits::default(), 0, || {
        Ok(Action::Doctor)
    })
}
#[tool(
    name = "fortress.checkpoint",
    description = "Unavailable in conditional-run development profile. An effect journal is not a game checkpoint."
)]
pub fn fortress_checkpoint(session_id: String) -> String {
    with_session(
        session_id,
        "fortress.checkpoint",
        Limits::default(),
        0,
        || Ok(Action::Unavailable),
    )
}
#[tool(
    name = "fortress.restore",
    description = "Unavailable in conditional-run development profile. Journal recovery never restores game state."
)]
pub fn fortress_restore(session_id: String) -> String {
    with_session(session_id, "fortress.restore", Limits::default(), 0, || {
        Ok(Action::Unavailable)
    })
}
pub fn run_stdio() {
    if let Err(e) = validate_environment() {
        eprintln!("{e}");
        std::process::exit(1);
    }
    let server=ServerBuilder::new("dfmcp-live-order-run-dev",env!("CARGO_PKG_VERSION"))
        .tool(FortressOpenSession).tool(FortressObserve).tool(FortressQuery).tool(FortressPlan).tool(FortressCommit)
        .tool(FortressWait).tool(FortressCancel).tool(FortressCheckpoint).tool(FortressRestore).tool(FortressExplain).tool(FortressDoctor)
        .instructions("Unadmitted order-run/1.14. Offline is the default; query retained work before new control. Operator selects exact fortress folder/site. Observe an order, seal an unmet finite condition, confirm the exact plan, commit once and reconcile by one-query waits. Native callback owns bounded stopping after client loss. Predicate samples and historical pause receipts never prove produced goods or present pause. Missing/uncertain results cannot be retried as unpause. Recover mode cannot become control; explicit session release preserves all evidence and never claims native quiescence. No other mutation, raw command, global clock lease, checkpoint or restore exists.").build();
    crate::run_modern_stdio(server);
}
#[cfg(test)]
mod tests;
