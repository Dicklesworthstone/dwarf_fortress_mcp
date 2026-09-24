#![forbid(unsafe_code)]
//! Explicitly unadmitted job-control/1.9 MCP runtime. No production runner edge.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};

use crate::job_control_session::JobControlSession;
use dfmcp_adapter::job_suspension::coordinator::{
    PrivateJobJournalFile, open_private_job_journal, open_private_job_reconciliation,
    open_private_job_recovery,
};
use dfmcp_adapter::job_suspension::rpc::{DeadlineTcpStream, JobControlRpcClient};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, DfmcpError, ErrorCode, FortressId, GameTick,
    ObservationCursor, OperationContext, RequestId, Result, RiskTier, SessionId, StateAnchor,
    WorkBudget,
};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use serde_json::{Value, json};

#[path = "job_control_presentation.rs"]
mod presentation;
use presentation::{
    BASE_RESERVE, Continuations, Filter, MAX_PAGE, RECORD_RESERVE, TurnView, digest, error,
    failure, observation_json, packet, record_json, summary_json,
};

const FAMILY: u128 = 9u128 << 57;
const SEQUENCE_LIMIT: u64 = 1u64 << 57;
const MAX_BYTES: u64 = 65 * 1024 * 1024;
static NEXT: AtomicU64 = AtomicU64::new(1);
static SESSION: Mutex<Option<RuntimeSession>> = Mutex::new(None);
type Control = JobControlSession<PrivateJobJournalFile, JobControlRpcClient<DeadlineTcpStream>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Offline,
    Reconcile,
    Control,
}
impl Mode {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "offline" => Ok(Self::Offline),
            "reconcile" => Ok(Self::Reconcile),
            "control" => Ok(Self::Control),
            _ => Err(error(
                ErrorCode::InvalidRequest,
                "job session mode must be offline, reconcile, or control",
            )),
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::Reconcile => "reconcile",
            Self::Control => "control",
        }
    }
}

struct RuntimeSession {
    id: SessionId,
    request: u128,
    anchor: StateAnchor,
    budget: WorkBudget,
    grants: Vec<CapabilityGrant>,
    mode: Mode,
    control: Control,
    continuations: Continuations,
}
impl RuntimeSession {
    fn context(&mut self) -> Result<OperationContext> {
        self.request = self
            .request
            .checked_add(1)
            .ok_or_else(|| error(ErrorCode::BudgetExceeded, "job request IDs exhausted"))?;
        Ok(OperationContext {
            session_id: self.id,
            request_id: RequestId::new(self.request),
            anchor: self.anchor,
            budget: self.budget,
            grants: self
                .grants
                .iter()
                .filter(|grant| {
                    grant.capability != Capability::ConfigureProduction
                        || std::env::var("DFMCP_JOB_CONTROL_ALLOW_PRODUCTION")
                            .ok()
                            .as_deref()
                            == Some("1")
                })
                .cloned()
                .collect(),
            cancellation_requested: asupersync::Cx::current()
                .is_some_and(|cx| cx.checkpoint().is_err()),
        })
    }
}

fn runtime_io() -> Result<()> {
    let cx = asupersync::Cx::current().ok_or_else(|| {
        error(
            ErrorCode::CapabilityDenied,
            "job MCP I/O requires its owned runtime context",
        )
    })?;
    cx.checkpoint().map_err(|_| {
        error(
            ErrorCode::CancellationRequested,
            "job runtime request is cancelled",
        )
    })?;
    if cx.io().is_none() {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "inherited runtime context does not permit job MCP I/O",
        ));
    }
    Ok(())
}

fn session_lock() -> Result<MutexGuard<'static, Option<RuntimeSession>>> {
    match SESSION.try_lock() {
        Ok(guard) => Ok(guard),
        Err(TryLockError::WouldBlock) => Err(error(
            ErrorCode::BudgetExceeded,
            "job session is serving another bounded foreground operation; retry a read later",
        )),
        Err(TryLockError::Poisoned(_)) => Err(error(
            ErrorCode::InternalInvariantViolation,
            "job session lock is poisoned; restart and recover the durable journal",
        )),
    }
}
fn next_id() -> Result<SessionId> {
    let number = NEXT
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            (n < SEQUENCE_LIMIT).then_some(n + 1)
        })
        .map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "job session identities exhausted",
            )
        })?;
    Ok(SessionId::new((1u128 << 127) | FAMILY | u128::from(number)))
}
fn session_id(raw: &str) -> Result<SessionId> {
    if raw.len() != 32
        || !raw
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "invalid job-control session ID",
        ));
    }
    let value = u128::from_str_radix(raw, 16)
        .map_err(|_| error(ErrorCode::InvalidRequest, "invalid session ID"))?;
    let id = SessionId::new(value);
    if id.get() != value || !id.is_process_scoped_live() || (value & ((1u128 << 62) - 1)) >> 57 != 9
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "not a job-control/1.9 session ID",
        ));
    }
    Ok(id)
}

const ALLOWED_ENVIRONMENT: [&str; 6] = [
    "DFMCP_ALLOW_UNADMITTED_JOB_CONTROL_V1_9",
    "DFMCP_JOB_CONTROL_TOKEN",
    "DFMCP_JOB_CONTROL_ENDPOINT",
    "DFMCP_JOB_CONTROL_JOURNAL",
    "DFMCP_JOB_CONTROL_FORTRESS_ID",
    "DFMCP_JOB_CONTROL_ALLOW_PRODUCTION",
];
fn environment_contract(
    opt_in: Option<&str>,
    production: Option<&str>,
    keys: &[String],
    admitted: bool,
) -> Result<()> {
    if opt_in != Some("1")
        || admitted
        || production.is_some_and(|value| value != "1")
        || keys
            .iter()
            .any(|key| key.starts_with("DFMCP_") && !ALLOWED_ENVIRONMENT.contains(&key.as_str()))
    {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "job-control/1.9 requires exact development opt-in and refuses admission or other DFMCP environment state",
        ));
    }
    Ok(())
}
fn validate_environment() -> Result<()> {
    let keys: Vec<String> = std::env::vars_os()
        .map(|(key, _)| key.to_string_lossy().into_owned())
        .collect();
    let opt_in = std::env::var("DFMCP_ALLOW_UNADMITTED_JOB_CONTROL_V1_9").ok();
    let production = std::env::var("DFMCP_JOB_CONTROL_ALLOW_PRODUCTION");
    if matches!(production, Err(std::env::VarError::NotUnicode(_))) {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "job production opt-in must be absent or exactly 1",
        ));
    }
    environment_contract(
        opt_in.as_deref(),
        production.ok().as_deref(),
        &keys,
        crate::admission::current_admission_provenance().is_some(),
    )
}
fn configured(name: &str, maximum: usize) -> Result<String> {
    let value = std::env::var(name).map_err(|_| {
        error(
            ErrorCode::CapabilityDenied,
            "required operator job-control configuration is absent or not UTF-8",
        )
    })?;
    if value.is_empty() || value.len() > maximum || value.contains('\0') {
        return Err(error(
            ErrorCode::InvalidRequest,
            "operator job-control configuration exceeds its bound",
        ));
    }
    Ok(value)
}

#[derive(Default)]
struct Limits {
    wall: Option<u64>,
    bytes: Option<u64>,
    tokens: Option<u32>,
}
fn narrowed(
    mut context: OperationContext,
    limits: &Limits,
    rows: usize,
) -> Result<(OperationContext, OperationContext)> {
    if let Some(value) = limits.wall {
        context.budget.max_wall_millis = value.min(context.budget.max_wall_millis);
    }
    if let Some(value) = limits.bytes {
        context.budget.max_bytes = value.min(context.budget.max_bytes);
    }
    if let Some(value) = limits.tokens {
        context.budget.max_output_tokens = value.min(context.budget.max_output_tokens);
    }
    context.budget.validate()?;
    let reserve = BASE_RESERVE + RECORD_RESERVE * rows as u64;
    if rows > MAX_PAGE
        || context.budget.max_bytes <= reserve
        || u64::from(context.budget.max_output_tokens) * 4 < reserve
    {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "complete job response and recovery warnings do not fit; no native operation was started",
        ));
    }
    let mut work = context.clone();
    work.budget.max_bytes -= reserve;
    Ok((context, work))
}
fn remaining_context(
    mut context: OperationContext,
    started: Instant,
    total_millis: u64,
) -> Result<OperationContext> {
    let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    context.budget.max_wall_millis = total_millis
        .checked_sub(elapsed)
        .map(|left| left.min(context.budget.max_wall_millis))
        .filter(|left| *left > 0)
        .ok_or_else(|| {
            error(
                ErrorCode::BudgetExceeded,
                "job request deadline expired before native work",
            )
        })?;
    Ok(context)
}
fn unbound(operation: &str, cause: &DfmcpError) -> String {
    packet(
        operation,
        failure(cause, operation),
        TurnView {
            context: None,
            mode: "unopened",
            summary: None,
            selected: None,
        },
    )
}
fn render(
    operation: &str,
    result: Value,
    session: &RuntimeSession,
    context: &OperationContext,
) -> String {
    let mut current = context.clone();
    current.anchor = session.anchor;
    let checked = session.control.summary(&current);
    let result = match &checked {
        Ok(_) => result,
        Err(cause) => json!({"ok":false, "retained_operation_result":result,
            "post_operation_error":failure(cause, operation), "journal_health_unknown":true}),
    };
    let summary = checked.ok();
    let selected = session.control.selected(&current).ok().flatten();
    let out = packet(
        operation,
        result,
        TurnView {
            context: Some(context),
            mode: session.mode.name(),
            summary: summary.as_ref(),
            selected,
        },
    );
    let limit = context
        .budget
        .max_bytes
        .min(u64::from(context.budget.max_output_tokens) * 4);
    if out.len() as u64 <= limit {
        return out;
    }
    // Never truncate JSON or lose an ambiguous-effect warning after dispatch.
    let cause = error(
        if operation == "fortress.commit" {
            ErrorCode::EffectIndeterminate
        } else {
            ErrorCode::BudgetExceeded
        },
        "job result exceeded its reserved envelope; inspect durable evidence before any further control",
    );
    packet(
        operation,
        failure(&cause, operation),
        TurnView {
            context: Some(context),
            mode: session.mode.name(),
            summary: summary.as_ref(),
            selected: None,
        },
    )
}
fn with_session<F>(raw: String, operation: &str, limits: Limits, rows: usize, body: F) -> String
where
    F: FnOnce(&mut RuntimeSession, OperationContext) -> Result<Value>,
{
    let started = Instant::now();
    let result = (|| -> Result<String> {
        runtime_io()?;
        validate_environment()?;
        let id = session_id(&raw)?;
        let mut guard = session_lock()?;
        let session = guard.as_mut().filter(|s| s.id == id).ok_or_else(|| {
            error(
                ErrorCode::SessionNotFound,
                "job session is absent or closed; open a new session",
            )
        })?;
        let initial = session.context()?;
        let (display, work) = match narrowed(initial.clone(), &limits, rows) {
            Ok(value) => value,
            Err(cause) => {
                return Ok(render(
                    operation,
                    failure(&cause, operation),
                    session,
                    &initial,
                ));
            }
        };
        // Access is checked before any cached response or native operation.
        let outcome = session
            .control
            .summary(&work)
            .and_then(|_| remaining_context(work, started, display.budget.max_wall_millis))
            .and_then(|context| body(session, context));
        let result = outcome.unwrap_or_else(|cause| failure(&cause, operation));
        Ok(render(operation, result, session, &display))
    })();
    result.unwrap_or_else(|cause| unbound(operation, &cause))
}

fn operator_grants(
    mode: Mode,
    fortress: FortressId,
    production_enabled: bool,
) -> Result<Vec<CapabilityGrant>> {
    if mode == Mode::Control && !production_enabled {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "control mode additionally requires operator DFMCP_JOB_CONTROL_ALLOW_PRODUCTION=1",
        ));
    }
    let mut grants = vec![CapabilityGrant {
        capability: Capability::Query,
        scope: CapabilityScope {
            fortress_id: Some(fortress),
            ..CapabilityScope::default()
        },
        max_risk: RiskTier::ReadOnly,
        expires_at_tick: None,
        remaining_uses: None,
    }];
    if mode == Mode::Control {
        grants.push(CapabilityGrant {
            capability: Capability::ConfigureProduction,
            scope: CapabilityScope {
                fortress_id: Some(fortress),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::Reversible,
            expires_at_tick: None,
            remaining_uses: None,
        });
    }
    Ok(grants)
}

#[tool(
    description = "Open isolated, unadmitted job-control/1.9. Default offline mode opens an existing private journal with Query authority and no DFHack credentials or connection. Reconcile mode queries native receipts but cannot mutate jobs. Control mode additionally requires an operator production opt-in. Journal, fortress, endpoint and credentials are operator configuration, never tool arguments."
)]
pub fn fortress_open_session(
    mode: Option<String>,
    max_wall_millis: Option<u64>,
    max_bytes: Option<u64>,
    max_output_tokens: Option<u32>,
) -> String {
    let result = (|| -> Result<String> {
        runtime_io()?;
        validate_environment()?;
        let started = Instant::now();
        let mode = Mode::parse(mode.as_deref().unwrap_or("offline"))?;
        let mut guard = session_lock()?;
        if guard.is_some() {
            return Err(error(
                ErrorCode::Conflict,
                "close the retained job session before opening another",
            ));
        }
        let fortress_text = configured("DFMCP_JOB_CONTROL_FORTRESS_ID", 20)?;
        let fortress_number = fortress_text.parse::<u64>().map_err(|_| {
            error(
                ErrorCode::InvalidRequest,
                "operator fortress ID must be canonical nonzero decimal",
            )
        })?;
        if fortress_number == 0 || fortress_number.to_string() != fortress_text {
            return Err(error(
                ErrorCode::InvalidRequest,
                "operator fortress ID must be canonical nonzero decimal",
            ));
        }
        let fortress = FortressId::new(fortress_number);
        let grants = operator_grants(
            mode,
            fortress,
            std::env::var("DFMCP_JOB_CONTROL_ALLOW_PRODUCTION")
                .ok()
                .as_deref()
                == Some("1"),
        )?;
        let path = PathBuf::from(configured("DFMCP_JOB_CONTROL_JOURNAL", 4096)?);
        let budget = WorkBudget {
            max_wall_millis: max_wall_millis.unwrap_or(5000),
            max_bytes: max_bytes.unwrap_or(MAX_BYTES),
            max_output_tokens: max_output_tokens.unwrap_or(32_768),
            max_entities: 4096,
            max_actions: 1,
            max_game_ticks: 0,
        };
        if budget.max_wall_millis > 60_000
            || budget.max_bytes > MAX_BYTES
            || budget.max_output_tokens > 65_536
        {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "job session limits exceed 60000ms, 65MiB, or 65536 output-token proxy units",
            ));
        }
        let id = next_id()?;
        let anchor = StateAnchor {
            fortress_id: fortress,
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(0),
            state_hash: dfmcp_core::Digest32::ZERO,
        };
        let context = OperationContext {
            session_id: id,
            request_id: RequestId::new(1),
            anchor,
            budget,
            grants: grants.clone(),
            cancellation_requested: asupersync::Cx::current()
                .is_some_and(|cx| cx.checkpoint().is_err()),
        };
        let (display, mut work) = narrowed(context, &Limits::default(), 1)?;
        work = remaining_context(work, started, budget.max_wall_millis)?;
        work.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        // Offline is deliberately BEFORE endpoint/token reads or connection construction.
        let (journal, source) = if mode == Mode::Offline {
            (open_private_job_recovery(&path, &work)?, None)
        } else {
            let endpoint_text = match std::env::var("DFMCP_JOB_CONTROL_ENDPOINT") {
                Ok(value) if value.len() <= 128 => value,
                Err(std::env::VarError::NotPresent) => "127.0.0.1:5000".to_owned(),
                _ => {
                    return Err(error(
                        ErrorCode::InvalidRequest,
                        "job endpoint must be bounded UTF-8 numeric loopback",
                    ));
                }
            };
            let endpoint = dfmcp_adapter::parse_loopback_endpoint(&endpoint_text)?;
            let token = configured("DFMCP_JOB_CONTROL_TOKEN", 256)?.into_bytes();
            work.budget.max_bytes = work
                .budget
                .max_bytes
                .checked_sub(278_528)
                .filter(|left| *left > 0)
                .ok_or_else(|| {
                    error(
                        ErrorCode::BudgetExceeded,
                        "job bootstrap RPC and journal do not fit",
                    )
                })?;
            let source = JobControlRpcClient::connect(
                endpoint,
                token,
                id.get().to_be_bytes().to_vec(),
                Duration::from_millis(work.budget.max_wall_millis),
            )?;
            work = remaining_context(work, started, budget.max_wall_millis)?;
            let journal = if mode == Mode::Control {
                open_private_job_journal(&path, &work)?
            } else {
                open_private_job_reconciliation(&path, &work)?
            };
            (journal, Some(source))
        };
        let control = JobControlSession::new(journal, source, &work)?;
        let session = RuntimeSession {
            id,
            request: 1,
            anchor,
            budget,
            grants,
            mode,
            control,
            continuations: Continuations::default(),
        };
        let summary = session.control.summary(&work)?;
        let out = packet(
            "fortress.open_session",
            json!({"ok":true,"session_id":id.to_string(),
            "mode":mode.name(),"bridge_connection_present":mode != Mode::Offline,
            "capabilities":if mode == Mode::Control {json!(["query","configure_production"])} else {json!(["query"])},
            "runtime_admitted":false,"durable_job_journal":summary_json(&summary),
            "next_step":{"tool":"fortress.query","state":"pending","limit":8},
            "close":{"tool":"fortress.cancel","scope":"session"}}),
            TurnView {
                context: Some(&display),
                mode: mode.name(),
                summary: Some(&summary),
                selected: None,
            },
        );
        if out.len() as u64
            > display
                .budget
                .max_bytes
                .min(u64::from(display.budget.max_output_tokens) * 4)
        {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "complete job bootstrap response does not fit; custody was not published",
            ));
        }
        if started.elapsed() >= Duration::from_millis(budget.max_wall_millis) {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "job session opening exceeded its budget; custody was not published",
            ));
        }
        *guard = Some(session);
        Ok(out)
    })();
    result.unwrap_or_else(|cause| unbound("fortress.open_session", &cause))
}

#[tool(
    description = "Acquire one exact native job observation in a connected session. Native IDs are not canonical entity IDs. Failed refresh invalidates the old selection. Use returned witness in fortress.plan; no mutation occurs."
)]
pub fn fortress_observe(session_id: String, native_job_id: u32) -> String {
    with_session(
        session_id,
        "fortress.observe",
        Limits::default(),
        1,
        |session, context| {
            let o = session.control.observe(native_job_id, &context)?;
            session.anchor.tick = GameTick(o.tick());
            session.anchor.state_hash = o.witness();
            Ok(json!({"ok":true,"observation":observation_json(&o)}))
        },
    )
}

#[tool(
    description = "Discover complete durable job records, including terminal outcomes, without native calls. State is all, pending, or reconciliation_required. Limit 1..16, default 8. A bounded 64-record scan may return fewer matches with continuation. Continuations bind this session, exact journal head, filter and limit; restart after journal changes. Output/byte limits only narrow session bounds."
)]
pub fn fortress_query(
    session_id: String,
    state: Option<String>,
    limit: Option<u32>,
    continuation: Option<String>,
    max_bytes: Option<u64>,
    max_output_tokens: Option<u32>,
) -> String {
    let limit = limit.unwrap_or(8) as usize;
    if !(1..=MAX_PAGE).contains(&limit) {
        return unbound(
            "fortress.query",
            &error(ErrorCode::InvalidRequest, "job page limit must be 1..16"),
        );
    }
    with_session(
        session_id,
        "fortress.query",
        Limits {
            bytes: max_bytes,
            tokens: max_output_tokens,
            wall: None,
        },
        limit,
        |session, context| {
            let filter = Filter::parse(state.as_deref().unwrap_or("all"))?;
            let summary = session.control.summary(&context)?;
            let after = continuation
                .as_deref()
                .map(|token| {
                    session
                        .continuations
                        .resolve(token, session.id, &summary, filter, limit)
                })
                .transpose()?;
            let scan_limit = 64usize
                .min((context.budget.max_bytes / 2048) as usize)
                .min(context.budget.max_entities as usize);
            if scan_limit == 0 {
                return Err(error(
                    ErrorCode::BudgetExceeded,
                    "job discovery cannot fit one complete record",
                ));
            }
            let page = session.control.records_page(
                summary.head,
                after.as_deref(),
                scan_limit,
                &context,
            )?;
            let mut records = Vec::new();
            let mut consumed = 0;
            for record in &page.records {
                consumed += 1;
                if filter.matches(record.state()) {
                    records.push(record_json(record));
                }
                if records.len() == limit {
                    break;
                }
            }
            let more = consumed < page.records.len() || page.next_after.is_some();
            let next = if more {
                page.records.get(consumed.saturating_sub(1)).map(|last| {
                    session.continuations.issue(
                        session.id,
                        &summary,
                        last.plan().key().to_owned(),
                        filter,
                        limit,
                    )
                })
            } else {
                None
            };
            Ok(
                json!({"ok":true,"records":records,"state":filter.name(),"scanned_records":consumed,
            "matching_records_in_journal":filter.total(&summary),
            "complete_matching_set_in_this_response":records.len() == filter.total(&summary),
            "continuation":next,"durable_job_journal":summary_json(&summary)}),
            )
        },
    )
}

#[tool(
    description = "Seal and durably prepare suspension intent from the currently selected native job. Requires ConfigureProduction, a stable idempotency key and exact observed witness. The server constructs the plan digest/token. It does not mutate the game or renew TTL on replay. Any unresolved dispatch blocks new keys."
)]
pub fn fortress_plan(
    session_id: String,
    idempotency_key: String,
    native_job_id: u32,
    suspended: bool,
    expected_witness: String,
) -> String {
    with_session(
        session_id,
        "fortress.plan",
        Limits::default(),
        1,
        |session, context| {
            let witness = digest(&expected_witness)?;
            let record = session.control.plan(
                &idempotency_key,
                native_job_id,
                suspended,
                witness,
                &context,
            )?;
            Ok(json!({"ok":true,"effect":record_json(&record),"game_mutation_dispatched":false}))
        },
    )
}

#[tool(
    description = "Commit an exact durably prepared job-suspension plan once. Requires its key, server-produced plan digest, exact expected witness, current selection and ConfigureProduction. The journal syncs dispatch intent before the setter. Terminal replay does not dispatch. Unknown outcomes require receipt reconciliation, never retry or a new key."
)]
pub fn fortress_commit(
    session_id: String,
    idempotency_key: String,
    plan_digest: String,
    expected_witness: String,
) -> String {
    with_session(
        session_id,
        "fortress.commit",
        Limits::default(),
        1,
        |session, context| {
            let record = session.control.commit(
                &idempotency_key,
                digest(&plan_digest)?,
                digest(&expected_witness)?,
                &context,
            )?;
            Ok(json!({"ok":true,"effect":record_json(&record),
            "operation_acknowledged":true,"production_goal_completion_proven":false}))
        },
    )
}

#[tool(
    description = "Perform one bounded Query-authorized native receipt reconciliation for an exact retained key and plan digest. No polling loop, reconnect, prepare or setter retry. Prepared and terminal records return stored evidence without native calls. Offline unresolved records require reopening in reconcile mode; absence never proves no effect."
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
        |session, context| {
            let record =
                session
                    .control
                    .reconcile(&idempotency_key, digest(&plan_digest)?, &context)?;
            Ok(json!({"ok":true,"effect":record_json(&record),"game_mutation_dispatched":false}))
        },
    )
}

#[tool(
    description = "Inspect one exact durable plan and receipt without native calls or journal changes. Retained evidence is not current game state. This is available in offline mode and does not require mutation authority."
)]
pub fn fortress_explain(
    session_id: String,
    idempotency_key: String,
    plan_digest: String,
) -> String {
    with_session(
        session_id,
        "fortress.explain",
        Limits::default(),
        1,
        |session, context| {
            let record =
                session
                    .control
                    .record(&idempotency_key, digest(&plan_digest)?, &context)?;
            Ok(json!({"ok":true,"effect":record_json(&record),"native_calls":0}))
        },
    )
}

fn close(raw: String) -> String {
    let result = (|| -> Result<String> {
        let id = session_id(&raw)?;
        let mut guard = session_lock()?;
        let session = guard.as_mut().filter(|s| s.id == id).ok_or_else(|| {
            error(
                ErrorCode::SessionNotFound,
                "job session is absent or already closed",
            )
        })?;
        let mut context = session.context()?;
        context.cancellation_requested = false; // Explicit resource drain, never a game effect.
        let out = packet(
            "fortress.cancel",
            json!({"ok":true,"scope":"session","closed":true,
            "effects_cancelled":false,"journal_changed":false,"native_calls":0,
            "next_step":{"tool":"fortress.open_session","mode":"offline"}}),
            TurnView {
                context: Some(&context),
                mode: session.mode.name(),
                summary: None,
                selected: None,
            },
        );
        // Drop native connection and private journal while still holding the
        // sole session slot. Another opener cannot race custody release.
        drop(guard.take());
        Ok(out)
    })();
    result.unwrap_or_else(|cause| unbound("fortress.cancel", &cause))
}
#[tool(
    description = "Cancel a still-prepared local effect by exact key/digest with scope=effect, or release this session with scope=session and no effect identity. Effect cancellation requires ConfigureProduction and never undoes a setter. Session close works even when custody is fenced and never cancels effects, repairs bytes, or erases recovery evidence."
)]
pub fn fortress_cancel(
    session_id: String,
    scope: String,
    idempotency_key: Option<String>,
    plan_digest: Option<String>,
) -> String {
    match (scope.as_str(), idempotency_key, plan_digest) {
        ("session", None, None) => close(session_id),
        ("effect", Some(key), Some(digest_text)) => with_session(
            session_id,
            "fortress.cancel",
            Limits::default(),
            1,
            |session, context| {
                let record = session
                    .control
                    .cancel(&key, digest(&digest_text)?, &context)?;
                Ok(
                    json!({"ok":true,"scope":"effect","effect":record_json(&record),"native_calls":0}),
                )
            },
        ),
        _ => unbound(
            "fortress.cancel",
            &error(
                ErrorCode::InvalidRequest,
                "cancel requires scope=effect with key and digest, or scope=session without either identity",
            ),
        ),
    }
}

fn denied(session_id: String, operation: &str) -> String {
    with_session(session_id, operation, Limits::default(), 0, |_, _| {
        Err(error(
            ErrorCode::CapabilityDenied,
            "job-control/1.9 does not implement checkpoint, restore, or other game mutation families",
        ))
    })
}
#[tool(description = "Unavailable in the isolated job-control/1.9 profile.")]
pub fn fortress_checkpoint(session_id: String) -> String {
    denied(session_id, "fortress.checkpoint")
}
#[tool(description = "Unavailable in the isolated job-control/1.9 profile.")]
pub fn fortress_restore(session_id: String) -> String {
    denied(session_id, "fortress.restore")
}
#[tool(
    description = "Inspect custody-checked journal health and recovery counts without native calls. This is diagnosis, not live-state freshness, production qualification or admission."
)]
pub fn fortress_doctor(session_id: String) -> String {
    with_session(
        session_id,
        "fortress.doctor",
        Limits::default(),
        0,
        |session, context| {
            Ok(
                json!({"ok":true,"runtime_admitted":false,"bridge_connection_present":session.control.has_source(&context)?,
            "durable_job_journal":summary_json(&session.control.summary(&context)?),"native_calls":0}),
            )
        },
    )
}

pub fn run_stdio() {
    if let Err(cause) = validate_environment() {
        eprintln!("{cause}");
        std::process::exit(1);
    }
    let server = ServerBuilder::new("dfmcp-live-job-control-dev", env!("CARGO_PKG_VERSION"))
        .tool(FortressOpenSession).tool(FortressObserve).tool(FortressQuery).tool(FortressPlan)
        .tool(FortressCommit).tool(FortressWait).tool(FortressCancel).tool(FortressCheckpoint)
        .tool(FortressRestore).tool(FortressExplain).tool(FortressDoctor)
        .instructions("Unadmitted job-control/1.9. Open defaults to offline inspection. Discover pending work first. Connected Query-only reconciliation never grants ConfigureProduction. Operator-enabled control changes only suspension of an existing eligible idle job while paused. Observe a selected native job, plan using its exact witness, then commit the server-sealed digest once. Applied means verified suspension readback, not production goal completion. Unknown outcomes block new dispatches and require receipt queries, not retries or new keys. Native connection errors require explicit close and reopen; no automatic reconnect or setter retry exists. Session close releases custody but never deletes or cancels effects. Checkpoint and restore are unavailable.")
        .build();
    crate::run_modern_stdio(server);
}

#[cfg(test)]
#[path = "live_job_control_server_tests.rs"]
mod tests;
