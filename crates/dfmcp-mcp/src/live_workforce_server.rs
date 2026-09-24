#![forbid(unsafe_code)]
//! Isolated workforce/1.17 presentation. Native effects remain in the typed journal.
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};

use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
use dfmcp_adapter::workforce_control::journal::{
    AssignmentRecord, MAX_BYTES as JOURNAL_BYTES, MAX_KEYS, WorkforceBinding, WorkforceMode,
    WorkforceView,
};
use dfmcp_adapter::workforce_control::private_file::{
    PrivateWorkforceFile, open_private_workforce,
};
use dfmcp_adapter::workforce_control::rpc::{
    CONNECT_BYTES, WorkforceRpcClient, WorkforceSource, authorize,
};
use dfmcp_adapter::workforce_control::{
    AssignmentSpec, MAX_EFFECT, MAX_PLAN, WorkforceCapture, fortress_id, validate_ids,
};
use dfmcp_adapter::workforce_session::WorkforceSession;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, Digest32, ErrorCode, FortressId, GameTick,
    ObservationCursor, OperationContext, RequestId, Result, RiskTier, SessionId, StateAnchor,
    WorkBudget,
};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use serde::Deserialize;
use serde_json::{Value, json};

mod native;
mod presentation;
use native::CheckedSource;
use presentation::{
    COMPACT_OUTPUT, Cursors, DETAIL_OUTPUT, Filter, MAX_PAGE, capture_summary, details_page,
    digest, failure, mode_name, packet, record_detail, record_summary, summary,
};

// Accounting allowances, not allocations. The journal may verify 64 MiB several
// times during a transaction. Reserve both views instead of charging clock-sized
// constants for arbitrarily large workforce configurations.
const MAX_WORK_BYTES: u64 = 1024 * 1024 * 1024;
const VIEW_BYTES: u64 =
    JOURNAL_BYTES as u64 + MAX_KEYS as u64 * (MAX_PLAN + MAX_EFFECT) as u64 + 4096;
const OPEN_BYTES: u64 = 3 * VIEW_BYTES;
const FAMILY: u128 = 17u128 << 57;
static NEXT: AtomicU64 = AtomicU64::new(1);
static SESSION: Mutex<Option<State<PrivateWorkforceFile>>> = Mutex::new(None);

fn error(code: ErrorCode, message: &str) -> dfmcp_core::DfmcpError {
    dfmcp_core::DfmcpError::new(code, message)
}
fn exhausted() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "complete workforce response and work exceed the admitted allowance",
    )
}
fn unbound(op: &str, e: &dfmcp_core::DfmcpError) -> String {
    packet(op, failure(e), None, None, None)
}

struct State<S> {
    id: SessionId,
    request: u128,
    budget: WorkBudget,
    grants: Vec<CapabilityGrant>,
    binding: WorkforceBinding,
    control: WorkforceSession<S>,
    // Presentation-only copy. The adapter session owns the authoritative
    // planning selection and all current-grant/clock-floor checks.
    selected: Option<WorkforceCapture>,
    cursors: Cursors,
}
impl<S: EffectJournalStorage> State<S> {
    fn context(&mut self, labor: bool, cancelled: bool) -> Result<OperationContext> {
        self.request = self.request.checked_add(1).ok_or_else(exhausted)?;
        Ok(OperationContext {
            session_id: self.id,
            request_id: RequestId::new(self.request),
            anchor: StateAnchor {
                fortress_id: self.binding.fortress(),
                cursor: ObservationCursor::ORIGIN,
                tick: GameTick(self.control.high_tick()),
                state_hash: Digest32::ZERO,
            },
            budget: self.budget,
            grants: self
                .grants
                .iter()
                .filter(|g| {
                    labor || !matches!(g.capability, Capability::Plan | Capability::ConfigureLabor)
                })
                .cloned()
                .collect(),
            cancellation_requested: cancelled,
        })
    }
    fn clear_selection(&mut self) {
        self.selected = None;
        self.control.clear_selection();
    }
}
#[derive(Default)]
struct Limits {
    wall: Option<u64>,
    bytes: Option<u64>,
    tokens: Option<u32>,
}
struct Work {
    deadline: Instant,
    bytes: u64,
}
impl Work {
    fn split(
        mut c: OperationContext,
        limits: Limits,
        output: u64,
    ) -> Result<(OperationContext, Self)> {
        if let Some(n) = limits.wall {
            c.budget.max_wall_millis = n.min(c.budget.max_wall_millis);
        }
        if let Some(n) = limits.bytes {
            c.budget.max_bytes = n.min(c.budget.max_bytes);
        }
        if let Some(n) = limits.tokens {
            c.budget.max_output_tokens = n.min(c.budget.max_output_tokens);
        }
        c.budget.validate()?;
        if output > u64::from(c.budget.max_output_tokens) * 4 {
            return Err(exhausted());
        }
        let bytes = c
            .budget
            .max_bytes
            .checked_sub(output + 2 * VIEW_BYTES)
            .filter(|n| *n > 0)
            .ok_or_else(exhausted)?;
        let deadline = Instant::now()
            .checked_add(Duration::from_millis(c.budget.max_wall_millis))
            .ok_or_else(exhausted)?;
        Ok((c, Self { deadline, bytes }))
    }
    fn context(&self, c: &OperationContext) -> Result<OperationContext> {
        let left = self
            .deadline
            .checked_duration_since(Instant::now())
            .filter(|d| *d >= Duration::from_millis(1))
            .ok_or_else(exhausted)?;
        let mut out = c.clone();
        out.budget.max_wall_millis = u64::try_from(left.as_millis()).map_err(|_| exhausted())?;
        out.budget.max_bytes = self.bytes;
        Ok(out)
    }
    fn view_context(&self, c: &OperationContext) -> Result<OperationContext> {
        let mut out = self.context(c)?;
        out.budget.max_bytes = VIEW_BYTES;
        Ok(out)
    }
    fn consume(&mut self, c: &OperationContext, bytes: u64) -> Result<OperationContext> {
        let mut out = self.context(c)?;
        self.bytes = self
            .bytes
            .checked_sub(bytes)
            .filter(|n| *n > 0)
            .ok_or_else(exhausted)?;
        out.budget.max_bytes = bytes;
        Ok(out)
    }
    fn connect<N, F: FnOnce(&OperationContext) -> Result<N>>(
        &mut self,
        c: &OperationContext,
        f: F,
    ) -> Result<N> {
        f(&self.consume(c, CONNECT_BYTES)?)
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    unit_ids: Vec<u32>,
}
impl Selection {
    fn parse(raw: &str) -> Result<Self> {
        if raw.len() > 2048 {
            return Err(error(
                ErrorCode::InvalidRequest,
                "workforce selection exceeds 2 KiB",
            ));
        }
        let value: Self = serde_json::from_str(raw).map_err(|_| {
            error(
                ErrorCode::InvalidRequest,
                "selection must be closed typed JSON",
            )
        })?;
        validate_ids(&value.unit_ids)?;
        Ok(value)
    }
}
#[derive(Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
enum Query {
    Records {
        state: Option<Filter>,
        limit: Option<u32>,
        continuation: Option<String>,
    },
    Details {
        witness: String,
        offset: u32,
        limit: Option<u32>,
    },
}
impl Query {
    fn parse(raw: &str) -> Result<Self> {
        if raw.len() > 2048 {
            return Err(error(
                ErrorCode::InvalidRequest,
                "workforce query exceeds 2 KiB",
            ));
        }
        let value: Self = serde_json::from_str(raw).map_err(|_| {
            error(
                ErrorCode::InvalidRequest,
                "unknown or malformed workforce query",
            )
        })?;
        let limit = match &value {
            Self::Records { limit, .. } | Self::Details { limit, .. } => limit.unwrap_or(4),
        };
        if !(1..=MAX_PAGE as u32).contains(&limit) {
            return Err(error(ErrorCode::InvalidRequest, "page limit must be 1..8"));
        }
        match &value {
            Self::Records {
                continuation: Some(token),
                ..
            } if token.len() != 64 => {
                return Err(error(
                    ErrorCode::InvalidRequest,
                    "invalid workforce continuation",
                ));
            }
            Self::Details {
                witness, offset, ..
            } => {
                digest(witness)?;
                if *offset >= 64 {
                    return Err(error(
                        ErrorCode::InvalidRequest,
                        "detail offset must be 0..63",
                    ));
                }
            }
            _ => {}
        }
        Ok(value)
    }
    fn output(&self) -> u64 {
        match self {
            Self::Records { .. } => COMPACT_OUTPUT,
            Self::Details { .. } => DETAIL_OUTPUT,
        }
    }
}
enum Action {
    Observe(Selection),
    Plan {
        key: String,
        spec: AssignmentSpec,
        witness: Digest32,
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
    Query(Query),
    Doctor,
    Unavailable,
}
fn known<'a>(view: &'a WorkforceView, key: &str, plan: Digest32) -> Result<&'a AssignmentRecord> {
    let value = view
        .records
        .iter()
        .find(|r| r.plan().key() == key)
        .ok_or_else(|| error(ErrorCode::InvalidRequest, "workforce key is not retained"))?;
    if value.plan().digest() != plan {
        return Err(error(
            ErrorCode::Conflict,
            "workforce key and digest differ",
        ));
    }
    Ok(value)
}
fn result_record(record: &AssignmentRecord) -> Value {
    json!({"ok":true,"effect":record_detail(record),"membership_undone":false,
        "current_state_proven":false,"jobs_completed_proven":false})
}

/// Shared actual handler path. The connection factory is not evaluated for local
/// history, permanent Unknown, terminal replay or pre-dispatch cancellation.
fn perform<S, N, F>(
    state: &mut State<S>,
    c: &OperationContext,
    work: &mut Work,
    view: &WorkforceView,
    action: Action,
    connect: F,
) -> Result<Value>
where
    S: EffectJournalStorage,
    N: WorkforceSource,
    F: FnOnce(&OperationContext) -> Result<N>,
{
    match action {
        Action::Observe(selection) => {
            state.clear_selection();
            let value =
                state
                    .control
                    .observe(&selection.unit_ids, &work.context(c)?, |_, current| {
                        connect(current)
                    })?;
            let result = capture_summary(&value);
            state.selected = Some(value);
            Ok(json!({"ok":true,"observation":result,"game_mutation_dispatched":false}))
        }
        Action::Plan { key, spec, witness } => {
            // Do not clear the adapter-owned precondition before it seals the
            // plan. The presentation copy cannot authorize an assignment.
            state.selected = None;
            let record =
                state
                    .control
                    .prepare(&key, spec, witness, &work.context(c)?, |_, current| {
                        connect(current)
                    })?;
            Ok(result_record(&record))
        }
        Action::Commit { key, plan, confirm } => {
            state.selected = None;
            let record =
                state
                    .control
                    .commit(&key, plan, confirm, &work.context(c)?, |_, current| {
                        connect(current)
                    })?;
            Ok(result_record(&record))
        }
        Action::Wait { key, plan } => {
            state.selected = None;
            let record = state
                .control
                .reconcile(&key, plan, &work.context(c)?, |_, current| connect(current))?;
            Ok(result_record(&record))
        }
        Action::Cancel { key, plan } => {
            state.selected = None;
            let record = state
                .control
                .cancel(&key, plan, &work.context(c)?, |_, current| connect(current))?;
            Ok(result_record(&record))
        }
        Action::Explain { key, plan } => Ok(result_record(known(view, &key, plan)?)),
        Action::Query(Query::Records {
            state: filter,
            limit,
            continuation,
        }) => {
            let filter = filter.unwrap_or(Filter::All);
            let limit = limit.unwrap_or(4) as usize;
            let rows = view
                .records
                .iter()
                .filter(|r| filter.matches(r))
                .collect::<Vec<_>>();
            let offset = continuation
                .as_deref()
                .map(|t| state.cursors.resolve(t, state.id, view, filter, limit))
                .transpose()?
                .unwrap_or(0);
            if offset > rows.len() {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "workforce continuation no longer addresses this page",
                ));
            }
            let end = (offset + limit).min(rows.len());
            let next = if end < rows.len() {
                Some(state.cursors.issue(state.id, view, filter, limit, end)?)
            } else {
                None
            };
            Ok(
                json!({"ok":true,"records":rows[offset..end].iter().map(|r| record_summary(r)).collect::<Vec<_>>(),
                "offset":offset,"matching_records":rows.len(),"continuation":next,"last_page":end==rows.len(),
                "complete_matching_set_in_this_response":offset==0 && end==rows.len(),"native_calls":0}),
            )
        }
        Action::Query(Query::Details {
            witness,
            offset,
            limit,
        }) => {
            let witness = digest(&witness)?;
            let selected = state
                .selected
                .as_ref()
                .filter(|v| v.witness() == witness)
                .ok_or_else(|| {
                    error(
                        ErrorCode::StaleAnchor,
                        "detail query must use the retained live selection witness",
                    )
                })?;
            Ok(
                json!({"ok":true,"selection":details_page(selected, offset as usize, limit.unwrap_or(4) as usize)?,"native_calls":0}),
            )
        }
        Action::Doctor => Ok(
            json!({"ok":true,"journal":summary(view),"native_calls":0,"live_health_checked":false}),
        ),
        Action::Unavailable => Err(error(
            ErrorCode::CapabilityDenied,
            "workforce journals are not game checkpoints or restores",
        )),
    }
}

struct Dispatch<'a> {
    operation: &'a str,
    limits: Limits,
    output: u64,
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
    N: WorkforceSource,
    F: FnOnce(&OperationContext) -> Result<N>,
{
    let Dispatch {
        operation,
        limits,
        output,
        action,
    } = request;
    // Even malformed/budget-refused refresh cannot retain a control-eligible old selection.
    if operation == "fortress.observe" {
        state.clear_selection();
    }
    let mode = state.control.mode();
    let (mut display, mut work) = match Work::split(context.clone(), limits, output) {
        Ok(v) => v,
        Err(e) => return packet(operation, failure(&e), Some(&context), Some(mode), None),
    };
    let before = match work
        .view_context(&display)
        .and_then(|c| state.control.view(&c))
    {
        Ok(v) => v,
        Err(e) => return packet(operation, failure(&e), Some(&display), Some(mode), None),
    };
    display.anchor.tick = GameTick(state.control.high_tick());
    // Recheck grants at the monotone evidence floor, not a coordination tick=0 sentinel.
    let outcome = authorize(
        &display,
        state.binding.fortress(),
        state.control.high_tick(),
        false,
    )
    .and_then(|_| work.context(&display))
    .and_then(|c| action.and_then(|a| perform(state, &c, &mut work, &before, a, connect)));
    let after = work.view_context(&display).and_then(|mut c| {
        c.anchor.tick = GameTick(state.control.high_tick());
        state.control.view(&c)
    });
    let (result, view) = match after {
        Ok(v) => (outcome.unwrap_or_else(|e| failure(&e)), Some(v)),
        Err(e) => {
            state.clear_selection();
            (
                json!({"ok":false,"post_operation_evidence_unavailable":true,
            "error":failure(&e),"effect_may_have_dispatched":matches!(operation,"fortress.commit"|"fortress.cancel"),"retry_commit":false}),
                None,
            )
        }
    };
    let rendered = packet(operation, result, Some(&display), Some(mode), view.as_ref());
    if rendered.len() as u64 <= output {
        return rendered;
    }
    let cause = error(
        if matches!(operation, "fortress.commit" | "fortress.cancel") {
            ErrorCode::EffectIndeterminate
        } else {
            ErrorCode::BudgetExceeded
        },
        "response exceeded reservation; recover retained evidence, never retry assignment",
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
    "DFMCP_ALLOW_UNADMITTED_WORKFORCE_V1_17",
    "DFMCP_WORKFORCE_WORLD_FOLDER",
    "DFMCP_WORKFORCE_SITE_ID",
    "DFMCP_WORKFORCE_JOURNAL",
    "DFMCP_WORKFORCE_ENDPOINT",
    "DFMCP_WORKFORCE_TOKEN",
    "DFMCP_WORKFORCE_ALLOW_LABOR",
];
fn labor_enabled() -> bool {
    std::env::var("DFMCP_WORKFORCE_ALLOW_LABOR").ok().as_deref() == Some("1")
}
fn environment_contract(
    opt_in: Option<&str>,
    labor: Option<&str>,
    names: &[String],
    admitted: bool,
) -> Result<()> {
    if opt_in != Some("1")
        || labor.is_some_and(|v| v != "1")
        || admitted
        || names
            .iter()
            .any(|n| n.starts_with("DFMCP_") && !ALLOWED.contains(&n.as_str()))
    {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "workforce development isolation or exact opt-in refused",
        ));
    }
    Ok(())
}
fn validate_environment() -> Result<()> {
    let opt = std::env::var("DFMCP_ALLOW_UNADMITTED_WORKFORCE_V1_17");
    let labor = std::env::var("DFMCP_WORKFORCE_ALLOW_LABOR");
    if matches!(labor, Err(std::env::VarError::NotUnicode(_))) {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "labor opt-in must be absent or exactly 1",
        ));
    }
    environment_contract(
        opt.ok().as_deref(),
        labor.ok().as_deref(),
        &std::env::vars_os()
            .map(|(k, _)| k.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        crate::admission::current_admission_provenance().is_some(),
    )
}
fn configured(name: &str, limit: usize) -> Result<String> {
    let value = std::env::var(name).map_err(|_| {
        error(
            ErrorCode::CapabilityDenied,
            "required workforce operator configuration is absent or not UTF-8",
        )
    })?;
    if value.is_empty() || value.len() > limit || value.contains('\0') {
        return Err(error(
            ErrorCode::InvalidRequest,
            "operator workforce value exceeds its bound",
        ));
    }
    Ok(value)
}
fn selected_fortress() -> Result<(String, u32)> {
    let folder = configured("DFMCP_WORKFORCE_WORLD_FOLDER", 512)?;
    let raw = configured("DFMCP_WORKFORCE_SITE_ID", 10)?;
    let site: u32 = raw.parse().map_err(|_| {
        error(
            ErrorCode::InvalidRequest,
            "site ID must be canonical nonnegative decimal",
        )
    })?;
    if site > i32::MAX as u32 || site.to_string() != raw {
        return Err(error(
            ErrorCode::InvalidRequest,
            "site ID is out of range or noncanonical",
        ));
    }
    Ok((folder, site))
}
fn runtime_io() -> Result<()> {
    let cx = asupersync::Cx::current().ok_or_else(|| {
        error(
            ErrorCode::CapabilityDenied,
            "workforce I/O requires an owned runtime context",
        )
    })?;
    cx.checkpoint().map_err(|_| {
        error(
            ErrorCode::CancellationRequested,
            "workforce request cancelled",
        )
    })?;
    if cx.io().is_none() {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "inherited runtime denies workforce I/O",
        ));
    }
    Ok(())
}
fn boundary(binding: Option<&WorkforceBinding>, write: bool) -> Result<()> {
    runtime_io()?;
    validate_environment()?;
    if write && !labor_enabled() {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "operator workforce authority was revoked",
        ));
    }
    if let Some(binding) = binding {
        let (folder, site) = selected_fortress()?;
        if binding.folder() != folder || binding.site() != site {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "configured workforce fortress changed",
            ));
        }
    }
    Ok(())
}
fn endpoint() -> Result<SocketAddr> {
    let value = match std::env::var("DFMCP_WORKFORCE_ENDPOINT") {
        Ok(v) if v.len() <= 128 => v,
        Err(std::env::VarError::NotPresent) => "127.0.0.1:5000".to_owned(),
        _ => {
            return Err(error(
                ErrorCode::InvalidRequest,
                "workforce endpoint must be bounded numeric loopback",
            ));
        }
    };
    dfmcp_adapter::parse_loopback_endpoint(&value)
}
fn grants(
    mode: WorkforceMode,
    fortress: FortressId,
    enabled: bool,
) -> Result<Vec<CapabilityGrant>> {
    if mode == WorkforceMode::Control && !enabled {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "control requires operator labor enablement",
        ));
    }
    let mut values = vec![CapabilityGrant {
        capability: Capability::Query,
        scope: CapabilityScope {
            fortress_id: Some(fortress),
            ..CapabilityScope::default()
        },
        max_risk: RiskTier::ReadOnly,
        expires_at_tick: None,
        remaining_uses: None,
    }];
    if mode == WorkforceMode::Control {
        for capability in [Capability::Plan, Capability::ConfigureLabor] {
            values.push(CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(fortress),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::Guarded,
                expires_at_tick: None,
                remaining_uses: None,
            });
        }
    }
    Ok(values)
}
fn parse_mode(raw: &str) -> Result<WorkforceMode> {
    match raw {
        "offline" => Ok(WorkforceMode::Offline),
        "recover" => Ok(WorkforceMode::Recover),
        "control" => Ok(WorkforceMode::Control),
        _ => Err(error(
            ErrorCode::InvalidRequest,
            "workforce mode must be offline, recover or control",
        )),
    }
}
fn lock() -> Result<MutexGuard<'static, Option<State<PrivateWorkforceFile>>>> {
    match SESSION.try_lock() {
        Ok(guard) => Ok(guard),
        Err(TryLockError::WouldBlock) => Err(error(
            ErrorCode::BudgetExceeded,
            "workforce session is already serving a bounded request",
        )),
        Err(TryLockError::Poisoned(_)) => Err(error(
            ErrorCode::InternalInvariantViolation,
            "workforce session poisoned; restart and recover journal",
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
            "invalid workforce session ID",
        ));
    }
    let n = u128::from_str_radix(raw, 16)
        .map_err(|_| error(ErrorCode::InvalidRequest, "invalid workforce session ID"))?;
    let id = SessionId::new(n);
    if id.get() != n || !id.is_process_scoped_live() || (n & ((1u128 << 62) - 1)) >> 57 != 17 {
        return Err(error(
            ErrorCode::InvalidRequest,
            "not a workforce/1.17 session ID",
        ));
    }
    Ok(id)
}
fn with_session(
    raw: String,
    operation: &str,
    limits: Limits,
    output: u64,
    action: Result<Action>,
) -> String {
    let result = (|| -> Result<String> {
        let id = session_id(&raw)?;
        let mut guard = lock()?;
        let state = guard
            .as_mut()
            .filter(|s| s.id == id)
            .ok_or_else(|| error(ErrorCode::SessionNotFound, "workforce session is absent"))?;
        if operation == "fortress.observe" {
            state.clear_selection();
        }
        boundary(Some(&state.binding), false)?;
        let context = state.context(labor_enabled(), false)?;
        let binding = state.binding.clone();
        Ok(run_action(
            state,
            context,
            Dispatch {
                operation,
                limits,
                output,
                action,
            },
            |c| {
                boundary(Some(&binding), false)?;
                if endpoint()? != binding.endpoint() {
                    return Err(error(
                        ErrorCode::CapabilityDenied,
                        "configured endpoint changed; reopen explicitly",
                    ));
                }
                let source = WorkforceRpcClient::connect(
                    binding.endpoint(),
                    configured("DFMCP_WORKFORCE_TOKEN", 256)?.into_bytes(),
                    c.request_id.get().to_be_bytes().to_vec(),
                    c,
                )?;
                Ok(CheckedSource {
                    source,
                    check: |write| boundary(Some(&binding), write),
                })
            },
        ))
    })();
    result.unwrap_or_else(|e| unbound(operation, &e))
}

#[tool(
    name = "fortress.open_session",
    description = "Open isolated workforce/1.17. Offline is the default and opens existing read-only custody without credentials or a native connection. Recover can query outcomes only. Control requires operator labor enablement. Fortress, path and endpoint are operator configuration, not arguments."
)]
pub fn fortress_open_session(
    mode: Option<String>,
    max_wall_millis: Option<u64>,
    max_bytes: Option<u64>,
    max_output_tokens: Option<u32>,
) -> String {
    let result = (|| -> Result<String> {
        boundary(None, false)?;
        let mode = parse_mode(mode.as_deref().unwrap_or("offline"))?;
        let (folder, site) = selected_fortress()?;
        let fortress = fortress_id(&folder, site);
        let authority = grants(mode, fortress, labor_enabled())?;
        let budget = WorkBudget {
            max_wall_millis: max_wall_millis.unwrap_or(10_000),
            max_bytes: max_bytes.unwrap_or(MAX_WORK_BYTES),
            max_output_tokens: max_output_tokens.unwrap_or(65_536),
            max_entities: 4192,
            max_actions: 1,
            max_game_ticks: 0,
        };
        if budget.max_wall_millis > 60_000
            || budget.max_bytes > MAX_WORK_BYTES
            || budget.max_output_tokens > 65_536
        {
            return Err(exhausted());
        }
        let mut guard = lock()?;
        if guard.is_some() {
            return Err(error(
                ErrorCode::Conflict,
                "close the current workforce session first",
            ));
        }
        let next = NEXT
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| {
                (v < (1u64 << 57)).then_some(v + 1)
            })
            .map_err(|_| exhausted())?;
        let id = SessionId::new((1u128 << 127) | FAMILY | u128::from(next));
        let c = OperationContext {
            session_id: id,
            request_id: RequestId::new(1),
            budget,
            grants: authority.clone(),
            cancellation_requested: false,
            anchor: StateAnchor {
                fortress_id: fortress,
                cursor: ObservationCursor::ORIGIN,
                tick: GameTick(0),
                state_hash: Digest32::ZERO,
            },
        };
        let (display, mut work) = Work::split(c, Limits::default(), COMPACT_OUTPUT)?;
        let path = PathBuf::from(configured("DFMCP_WORKFORCE_JOURNAL", 4096)?);
        // Offline/recover bootstrap consumes no transport credential. The exact
        // journal binding is verified first; online connections are request-local.
        let expected = if mode == WorkforceMode::Control {
            let address = endpoint()?;
            let token = configured("DFMCP_WORKFORCE_TOKEN", 256)?.into_bytes();
            let source = work.connect(&display, |c| {
                WorkforceRpcClient::connect(address, token, id.get().to_be_bytes().to_vec(), c)
            })?;
            Some(WorkforceBinding::new(
                address,
                source.manifest().clone(),
                folder.clone(),
                site,
            )?)
        } else {
            None
        };
        boundary(expected.as_ref(), mode == WorkforceMode::Control)?;
        let file_context = work.consume(&display, OPEN_BYTES)?;
        let journal = open_private_workforce(&path, &file_context, mode, expected)?;
        let (control, view) = WorkforceSession::new(journal, &work.view_context(&display)?)?;
        if view.binding.folder() != folder || view.binding.site() != site {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "journal is for a different exact fortress",
            ));
        }
        let state = State {
            id,
            request: 1,
            budget,
            grants: authority,
            binding: view.binding.clone(),
            control,
            selected: None,
            cursors: Cursors::default(),
        };
        let output = packet(
            "fortress.open_session",
            json!({"ok":true,"session_id":id.to_string(),"mode":mode_name(mode),
            "capabilities":state.grants.iter().map(|g| g.capability.as_str()).collect::<Vec<_>>(),
            "native_workforce_observed":false,"journal":summary(&view)}),
            Some(&display),
            Some(mode),
            Some(&view),
        );
        if output.len() as u64 > COMPACT_OUTPUT {
            return Err(exhausted());
        }
        work.context(&display)?;
        boundary(Some(&state.binding), mode == WorkforceMode::Control)?;
        *guard = Some(state);
        Ok(output)
    })();
    result.unwrap_or_else(|e| unbound("fortress.open_session", &e))
}
#[tool(
    name = "fortress.observe",
    description = "Capture 1..32 sorted unique citizen IDs and the complete bounded work-detail configuration. selection is closed JSON: {\"unit_ids\":[2,5]}. No labor changes. Failed refresh clears prior selection. Use the returned witness to inspect details and prepare an assignment."
)]
pub fn fortress_observe(session_id: String, selection: String) -> String {
    with_session(
        session_id,
        "fortress.observe",
        Limits::default(),
        COMPACT_OUTPUT,
        Selection::parse(&selection).map(Action::Observe),
    )
}
#[tool(
    name = "fortress.query",
    description = "Local verified discovery without native calls. query is closed JSON: records (state all/pending/unresolved/terminal, limit 1..8, continuation) or details (exact selection witness, offset 0..63, limit 1..8). Detail indices belong only to that captured configuration. Records survive restart; cached selection does not."
)]
pub fn fortress_query(
    session_id: String,
    query: String,
    max_bytes: Option<u64>,
    max_output_tokens: Option<u32>,
) -> String {
    let parsed = Query::parse(&query);
    let output = parsed.as_ref().map_or(COMPACT_OUTPUT, Query::output);
    with_session(
        session_id,
        "fortress.query",
        Limits {
            bytes: max_bytes,
            tokens: max_output_tokens,
            wall: None,
        },
        output,
        parsed.map(Action::Query),
    )
}
#[tool(
    name = "fortress.plan",
    description = "Prepare membership in one existing selected-only work detail for the retained observed citizens. Requires exact observation witness and operator-enabled ConfigureLabor/Plan authority. detail_index is capture-local, assigned adds/removes membership. Intent and native preparation are synced; no assignment is dispatched. Review the returned plan digest and labor columns before commit."
)]
pub fn fortress_plan(
    session_id: String,
    idempotency_key: String,
    expected_witness: String,
    detail_index: u32,
    assigned: bool,
) -> String {
    let action = (|| {
        Ok(Action::Plan {
            key: idempotency_key,
            spec: AssignmentSpec::new(detail_index, assigned)?,
            witness: digest(&expected_witness)?,
        })
    })();
    with_session(
        session_id,
        "fortress.plan",
        Limits::default(),
        DETAIL_OUTPUT,
        action,
    )
}
#[tool(
    name = "fortress.commit",
    description = "Confirm and commit one reviewed assignment plan once. Exact key/digest, confirm=true, current labor authority and identical paused source/workforce are required. Dispatch is synced before the setter. Applied proves immediate membership/labor readback, not productivity. Uncertain effects require recovery, never another commit or a new key."
)]
pub fn fortress_commit(
    session_id: String,
    idempotency_key: String,
    plan_digest: String,
    confirm: bool,
) -> String {
    let action = digest(&plan_digest).map(|plan| Action::Commit {
        key: idempotency_key,
        plan,
        confirm,
    });
    with_session(
        session_id,
        "fortress.commit",
        Limits::default(),
        DETAIL_OUTPUT,
        action,
    )
}
#[tool(
    name = "fortress.wait",
    description = "One QueryAssignment reconciliation for an unresolved attempt. No polling, capture, labor recomputation or assignment retry. Receipt sync precedes acknowledgement. Stored prepared/terminal/permanent-Unknown evidence returns locally. Missing native records remain uncertain; offline pending intent requires recover-mode reopen."
)]
pub fn fortress_wait(
    session_id: String,
    idempotency_key: String,
    plan_digest: String,
    max_wall_millis: Option<u64>,
) -> String {
    let action = digest(&plan_digest).map(|plan| Action::Wait {
        key: idempotency_key,
        plan,
    });
    with_session(
        session_id,
        "fortress.wait",
        Limits {
            wall: max_wall_millis,
            ..Limits::default()
        },
        DETAIL_OUTPUT,
        action,
    )
}
#[tool(
    name = "fortress.explain",
    description = "Inspect the exact retained plan, citizen identities, labor-column masks and historical receipt without contacting DFHack. Available offline. Neither stored Applied nor a self-hashed receipt proves current state or jobs completed."
)]
pub fn fortress_explain(
    session_id: String,
    idempotency_key: String,
    plan_digest: String,
) -> String {
    let action = digest(&plan_digest).map(|plan| Action::Explain {
        key: idempotency_key,
        plan,
    });
    with_session(
        session_id,
        "fortress.explain",
        Limits::default(),
        DETAIL_OUTPUT,
        action,
    )
}
fn close_allowed(view: Option<&WorkforceView>, release: bool) -> bool {
    release || view.is_some_and(|v| v.records.iter().all(|r| r.state().settled()))
}
fn close(raw: &str, release: bool) -> String {
    let result = (|| -> Result<String> {
        let id = session_id(raw)?;
        let mut guard = lock()?;
        let state = guard.as_mut().filter(|s| s.id == id).ok_or_else(|| {
            error(
                ErrorCode::SessionNotFound,
                "workforce session absent or already closed",
            )
        })?;
        let context = state.context(false, false)?;
        let mode = state.control.mode();
        let view = if release {
            None
        } else {
            boundary(Some(&state.binding), false)?;
            Some(state.control.view(&context)?)
        };
        if !close_allowed(view.as_ref(), release) {
            return Err(error(
                ErrorCode::EffectIndeterminate,
                "unsettled assignments require cancellation or explicit release_for_recovery",
            ));
        }
        let output = packet(
            "fortress.cancel",
            json!({"ok":true,"scope":"session","closed":true,
            "release_for_recovery":release,"effects_cancelled":false,"membership_undone":false,"quiescence_proven":false,
            "native_calls":0,"journal_changed":false}),
            Some(&context),
            Some(mode),
            view.as_ref(),
        );
        // Explicit recovery release works after fencing/revocation; it releases
        // only local custody, not obligations or native effects.
        drop(guard.take());
        Ok(output)
    })();
    result.unwrap_or_else(|e| unbound("fortress.cancel", &e))
}
#[tool(
    name = "fortress.cancel",
    description = "scope=effect with exact key/digest retires undispatched local intent or a surviving native preparation; never undoes membership or repairs Unknown. scope=session without effect identity releases only settled custody, unless release_for_recovery=true explicitly releases unresolved/fenced custody. No effect, history or membership is erased."
)]
pub fn fortress_cancel(
    session_id: String,
    scope: String,
    idempotency_key: Option<String>,
    plan_digest: Option<String>,
    release_for_recovery: Option<bool>,
) -> String {
    match (scope.as_str(), idempotency_key, plan_digest) {
        ("session", None, None) => close(&session_id, release_for_recovery.unwrap_or(false)),
        ("effect", Some(key), Some(raw)) if !release_for_recovery.unwrap_or(false) => {
            let action = digest(&raw).map(|plan| Action::Cancel { key, plan });
            with_session(
                session_id,
                "fortress.cancel",
                Limits::default(),
                DETAIL_OUTPUT,
                action,
            )
        }
        _ => unbound(
            "fortress.cancel",
            &error(
                ErrorCode::InvalidRequest,
                "cancel scope and effect identity/release flags disagree",
            ),
        ),
    }
}
#[tool(
    name = "fortress.checkpoint",
    description = "Unavailable: the workforce effect journal is not a game save or checkpoint."
)]
pub fn fortress_checkpoint(session_id: String) -> String {
    with_session(
        session_id,
        "fortress.checkpoint",
        Limits::default(),
        COMPACT_OUTPUT,
        Ok(Action::Unavailable),
    )
}
#[tool(
    name = "fortress.restore",
    description = "Unavailable: journal recovery never restores game state or reverses workforce membership."
)]
pub fn fortress_restore(session_id: String) -> String {
    with_session(
        session_id,
        "fortress.restore",
        Limits::default(),
        COMPACT_OUTPUT,
        Ok(Action::Unavailable),
    )
}
#[tool(
    name = "fortress.doctor",
    description = "Verify local workforce journal custody and report retained states without native calls. Not a live health, job-completion or admission check."
)]
pub fn fortress_doctor(session_id: String) -> String {
    with_session(
        session_id,
        "fortress.doctor",
        Limits::default(),
        COMPACT_OUTPUT,
        Ok(Action::Doctor),
    )
}
pub fn run_stdio() {
    if let Err(e) = validate_environment() {
        eprintln!("{e}");
        std::process::exit(1);
    }
    let server = ServerBuilder::new("dfmcp-live-workforce-dev", env!("CARGO_PKG_VERSION"))
        .tool(FortressOpenSession).tool(FortressObserve).tool(FortressQuery).tool(FortressPlan)
        .tool(FortressCommit).tool(FortressWait).tool(FortressCancel).tool(FortressCheckpoint)
        .tool(FortressRestore).tool(FortressExplain).tool(FortressDoctor)
        .instructions("Isolated unadmitted workforce/1.17. Discover unsettled records first. Observe exact citizens, inspect captured work details, prepare, review and commit once. Applied proves immediate membership and labor readback only. Recovery never retries assignment. Permanent Unknown blocks new control. Offline/recover/control modes are immutable. Paths, credentials and fortress selection are operator-owned. Other controllers are not globally fenced. No raw labor enum, work-detail creation, checkpoint, restore, job completion or productivity claim exists.")
        .build();
    crate::run_modern_stdio(server);
}
#[cfg(test)]
mod tests;
