//! Explicitly unadmitted jobs/1.2 MCP runtime. This observes the native job list;
//! it neither merges independently timed citizen snapshots nor controls the game.

#[path = "query_response.rs"]
mod query_response;
#[path = "semantic_query.rs"]
mod semantic_query;

use crate::agent_turn::{
    AgentPhase, AgentTurnBuilder, ContinuityStatus, ObservationProfile, empty_active_work,
};
use dfmcp_adapter::live_jobs::{
    JobPublication, LiveJobObservation, LiveJobsState, MAX_JOB_FRAME_BYTES, MAX_JOBS,
};
use dfmcp_adapter::live_jobs_rpc::{DeadlineStream, JobsRpcClient};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, DfmcpError, ErrorCode, OperationContext,
    RequestId, Result, RiskTier, SessionId, StateAnchor, WorkBudget,
};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use query_response::QueryResponseProjection;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::Duration;

const OPT_IN: &str = "DFMCP_ALLOW_UNADMITTED_JOBS_V1_2";
const MAX_SESSIONS: usize = 16;
const FAMILY: u128 = 1u128 << 60;
static NEXT: LazyLock<Mutex<u128>> = LazyLock::new(|| Mutex::new(1));
static SLOTS: AtomicUsize = AtomicUsize::new(0);
static SESSIONS: LazyLock<Mutex<BTreeMap<SessionId, Arc<Mutex<JobSession>>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

fn error(code: ErrorCode, text: &str) -> DfmcpError {
    DfmcpError::new(code, text)
}
fn lock<T>(value: &Mutex<T>) -> Result<MutexGuard<'_, T>> {
    value.lock().map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "jobs session mutex is poisoned",
        )
    })
}
struct SessionSlot;
impl SessionSlot {
    fn reserve() -> Result<Self> {
        SLOTS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                (value < MAX_SESSIONS).then_some(value + 1)
            })
            .map_err(|_| {
                error(
                    ErrorCode::BudgetExceeded,
                    "jobs runtime session capacity reached",
                )
            })?;
        Ok(Self)
    }
}
impl Drop for SessionSlot {
    fn drop(&mut self) {
        SLOTS.fetch_sub(1, Ordering::AcqRel);
    }
}

trait JobSource: Send {
    fn read(&mut self, timeout: Duration) -> Result<LiveJobObservation>;
    fn poisoned(&self) -> bool;
    fn fence(&mut self);
}
impl JobSource for JobsRpcClient<DeadlineStream> {
    fn read(&mut self, timeout: Duration) -> Result<LiveJobObservation> {
        self.refresh(timeout)
    }
    fn poisoned(&self) -> bool {
        JobsRpcClient::poisoned(self)
    }
    fn fence(&mut self) {
        JobsRpcClient::fence(self);
    }
}
struct JobSession {
    id: SessionId,
    source: Box<dyn JobSource>,
    state: LiveJobsState,
    budget: WorkBudget,
    grants: Vec<CapabilityGrant>,
    request: u128,
    _slot: SessionSlot,
}
impl JobSession {
    fn anchor(&self) -> Result<StateAnchor> {
        self.state
            .snapshot()
            .map(|snapshot| snapshot.anchor())
            .ok_or_else(|| {
                error(
                    ErrorCode::InternalInvariantViolation,
                    "jobs session has no published snapshot",
                )
            })
    }
    fn context(&mut self) -> Result<OperationContext> {
        self.request = self
            .request
            .checked_add(1)
            .ok_or_else(|| error(ErrorCode::BudgetExceeded, "jobs request counter exhausted"))?;
        Ok(OperationContext {
            session_id: self.id,
            request_id: RequestId::new(self.request),
            anchor: self.anchor()?,
            budget: self.budget,
            grants: self.grants.clone(),
            cancellation_requested: false,
        })
    }
    fn refresh(&mut self, context: &OperationContext) -> Result<JobPublication> {
        context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        if context.anchor != self.anchor()? {
            return Err(error(
                ErrorCode::StaleAnchor,
                "jobs refresh context is stale",
            ));
        }
        if self.source.poisoned() {
            return Err(error(
                ErrorCode::AdapterUnavailable,
                "jobs source is fenced; reopen the session",
            ));
        }
        let result = self
            .source
            .read(Duration::from_millis(context.budget.max_wall_millis))
            .and_then(|observation| {
                if observation.jobs.len().saturating_add(1) > context.budget.max_entities as usize
                    || observation.encode_payload()?.len() as u64 > context.budget.max_bytes
                {
                    return Err(error(
                        ErrorCode::BudgetExceeded,
                        "job observation exceeds session budget",
                    ));
                }
                self.state.publish(observation)
            });
        if result.is_err() {
            self.source.fence();
        }
        result
    }
}
fn next_id() -> Result<SessionId> {
    let mut counter = lock(&NEXT)?;
    if *counter >= FAMILY {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "jobs session IDs exhausted",
        ));
    }
    let id = SessionId::new((1u128 << 127) | FAMILY | *counter);
    *counter += 1;
    Ok(id)
}
fn resolve(id: Option<String>) -> Result<Arc<Mutex<JobSession>>> {
    let raw = id.ok_or_else(|| {
        error(
            ErrorCode::InvalidRequest,
            "open a jobs session and supply session_id",
        )
    })?;
    if raw.len() != 32 || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(error(
            ErrorCode::InvalidRequest,
            "invalid jobs session handle",
        ));
    }
    let value = u128::from_str_radix(&raw, 16)
        .map_err(|_| error(ErrorCode::InvalidRequest, "invalid jobs session handle"))?;
    let id = SessionId::new(value);
    // Parsing may not mint a process-scoped identity from an untrusted raw ID.
    if value != id.get()
        || !id.is_process_scoped_live()
        || (id.get() & ((1u128 << 62) - 1)) >> 60 != 1
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "session belongs to a different runtime or is not an encoded handle",
        ));
    }
    lock(&SESSIONS)?
        .get(&id)
        .cloned()
        .ok_or_else(|| error(ErrorCode::SessionNotFound, "jobs session not found"))
}
fn allowed_environment(name: &str) -> bool {
    !name.starts_with("DFMCP_")
        || matches!(
            name,
            "DFMCP_ALLOW_UNADMITTED_JOBS_V1_2" | "DFMCP_JOBS_TOKEN" | "DFMCP_JOBS_ENDPOINT"
        )
}
fn validate_environment() -> Result<()> {
    if std::env::var(OPT_IN).ok().as_deref() != Some("1")
        || std::env::vars_os().any(|(key, _)| !allowed_environment(&key.to_string_lossy()))
        || crate::admission::current_admission_provenance().is_some()
    {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "jobs development requires DFMCP_ALLOW_UNADMITTED_JOBS_V1_2=1 and refuses other DFMCP/admission environment state",
        ));
    }
    Ok(())
}
fn anchor_json(anchor: StateAnchor) -> Value {
    json!({"fortress_id":anchor.fortress_id.to_string(),"epoch":anchor.cursor.epoch,
        "sequence":anchor.cursor.sequence,"game_tick":anchor.tick.0,"state_hash":anchor.state_hash.to_string()})
}
fn coverage() -> Value {
    json!({"status":"partial","complete_domains":["fortress.current_job_roster"],
        "partial_domains":[{"domain":"fortress.jobs","reason":"flags, native references, positions and counts only; blockers and completion are not established"}],
        "omitted_domains":["fortress.citizens","fortress.announcements","fortress.items","fortress.buildings",
            "fortress.map","fortress.military","fortress.history"],"continuation":null})
}
fn briefing(session: &JobSession) -> Value {
    let observation = session.state.observation();
    json!({"runtime":"unadmitted_development","bridge_protocol":"1.2","observation_profile":"jobs-only",
        "read_only":true,"live":true,"mutation_admissible":false,"runtime_admitted":false,
        "compatibility_admitted":false,"source_poisoned":session.source.poisoned(),
        "paused":observation.map(|value|value.paused),
        "job_count":observation.map(|value|value.jobs.len()),
        "suspended_job_count":observation.map(|value|value.jobs.iter().filter(|job|job.suspended).count()),
        "unassigned_job_count":observation.map(|value|value.jobs.iter().filter(|job|job.worker_native_id.is_none()).count()),
        "note":"suspended or unassigned does not by itself establish a production failure"})
}
fn packet_limit(budget: WorkBudget) -> usize {
    budget
        .max_bytes
        .min(u64::from(budget.max_output_tokens) * 4) as usize
}
fn tool_packet(
    session: Option<&JobSession>,
    context: Option<&OperationContext>,
    operation: &str,
    mut payload: Value,
) -> Result<String> {
    let mut work = empty_active_work();
    if let Some(value) = payload
        .as_object_mut()
        .and_then(|object| object.remove("_condition_watch_work"))
    {
        work["obligations"] = value;
    }
    let mut builder = AgentTurnBuilder::new(
        operation,
        if operation == "fortress.observe" {
            AgentPhase::Orient
        } else {
            AgentPhase::Inspect
        },
    )
    .profile(ObservationProfile::Briefing)
    .active_work(work);
    let mut limit = 8192;
    if let (Some(session), Some(context)) = (session, context) {
        limit = packet_limit(session.budget);
        let anchor = session.anchor()?;
        let reset = payload.get("reset").and_then(Value::as_bool) == Some(true);
        let continuity = if session.source.poisoned() {
            ContinuityStatus::Stale
        } else if reset {
            ContinuityStatus::Reset
        } else if payload.get("kind").and_then(Value::as_str) == Some("heartbeat") {
            ContinuityStatus::Heartbeat
        } else {
            ContinuityStatus::Continuous
        };
        payload["session_id"] = json!(session.id.to_string());
        payload["anchor"] = anchor_json(anchor);
        builder = builder
            .session_id(session.id.to_string())
            .request_id(context.request_id.to_string())
            .anchor(anchor_json(anchor))
            .briefing(briefing(session))
            .coverage(coverage())
            .continuity(
                continuity,
                Some(anchor_json(context.anchor)),
                None,
                reset.then(|| "jobs_bridge_clock_or_identity_horizon_reset".to_owned()),
            );
    } else {
        builder = builder
            .briefing(
                json!({"runtime":"unadmitted_development","observation_profile":"jobs-only",
            "runtime_admitted":false,"mutation_admissible":false,"fortress_loaded":false}),
            )
            .coverage(
                json!({"status":"unknown","complete_domains":[],"partial_domains":[],
                "omitted_domains":["fortress.jobs"],"continuation":null}),
            );
    }
    let encoded = builder.attach(payload);
    if encoded.len() > limit {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "complete jobs response exceeds output budget",
        ));
    }
    Ok(encoded)
}
fn error_packet(
    session: Option<&JobSession>,
    context: Option<&OperationContext>,
    operation: &str,
    failure: &DfmcpError,
) -> String {
    let payload = json!({"ok":false,"error":{"code":failure.code.as_str(),"message":failure.message,
        "operation":operation,"retryable":false,"next_step":"refresh or reopen only after correcting the stated failure"}});
    match tool_packet(session, context, operation, payload) {
        Ok(text) => text,
        Err(_) => AgentTurnBuilder::new(operation, AgentPhase::Inspect)
            .briefing(json!({"runtime_admitted":false,"mutation_admissible":false}))
            .attach(json!({"ok":false,"error":{"code":"budget_exceeded","message":"response budget exhausted"}})),
    }
}
fn with_session<F>(id: Option<String>, operation: &str, capability: Capability, body: F) -> String
where
    F: FnOnce(&mut JobSession, OperationContext) -> Result<String>,
{
    let handle = match resolve(id) {
        Ok(value) => value,
        Err(failure) => return error_packet(None, None, operation, &failure),
    };
    let mut session = match lock(&handle) {
        Ok(value) => value,
        Err(failure) => return error_packet(None, None, operation, &failure),
    };
    let context = match session.context() {
        Ok(value) => value,
        Err(failure) => return error_packet(None, None, operation, &failure),
    };
    let result = context
        .authorize(capability, RiskTier::ReadOnly, &[], None)
        .and_then(|()| body(&mut session, context.clone()));
    match result {
        Ok(text) => text,
        Err(failure) => error_packet(Some(&session), Some(&context), operation, &failure),
    }
}
fn read_capabilities(requested: Option<Vec<String>>) -> Result<Vec<Capability>> {
    let requested = requested.unwrap_or_else(|| {
        vec![
            "observe".to_owned(),
            "query".to_owned(),
            "doctor".to_owned(),
        ]
    });
    if requested.is_empty() || requested.len() > 3 {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "request 1..3 read-only capabilities",
        ));
    }
    let mut capabilities = Vec::new();
    for name in requested {
        let capability = match name.as_str() {
            "observe" => Capability::Observe,
            "query" => Capability::Query,
            "doctor" => Capability::Doctor,
            _ => {
                return Err(error(
                    ErrorCode::CapabilityDenied,
                    "jobs runtime cannot grant that capability",
                ));
            }
        };
        if capabilities.contains(&capability) {
            return Err(error(ErrorCode::InvalidRequest, "duplicate capability"));
        }
        capabilities.push(capability);
    }
    Ok(capabilities)
}

#[tool(
    description = "Open an explicitly unadmitted, authenticated jobs-only DFHack session. Token and numeric loopback endpoint come only from process configuration. No mutation authority exists."
)]
pub fn fortress_open_session(
    max_jobs: Option<u32>,
    max_output_tokens: Option<u32>,
    max_bytes: Option<u64>,
    max_wall_millis: Option<u64>,
    requested_capabilities: Option<Vec<String>>,
) -> String {
    let result = (|| -> Result<String> {
        validate_environment()?;
        let capabilities = read_capabilities(requested_capabilities)?;
        let maximum = max_jobs.unwrap_or(1024);
        let budget = WorkBudget {
            max_wall_millis: max_wall_millis.unwrap_or(5000),
            max_game_ticks: 1_000_000,
            max_entities: maximum.saturating_add(1),
            max_bytes: max_bytes.unwrap_or(MAX_JOB_FRAME_BYTES as u64),
            max_output_tokens: max_output_tokens.unwrap_or(8192),
            max_actions: 1,
        };
        if maximum == 0
            || maximum as usize > MAX_JOBS
            || !(1..=60_000).contains(&budget.max_wall_millis)
            || !(8192..=MAX_JOB_FRAME_BYTES as u64).contains(&budget.max_bytes)
            || !(2048..=65_536).contains(&budget.max_output_tokens)
        {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "jobs session budgets exceed their supported bounds",
            ));
        }
        budget.validate()?;
        let slot = SessionSlot::reserve()?;
        let id = next_id()?;
        let endpoint =
            std::env::var("DFMCP_JOBS_ENDPOINT").unwrap_or_else(|_| "127.0.0.1:5000".to_owned());
        let endpoint = dfmcp_adapter::parse_loopback_endpoint(&endpoint)?;
        let token = std::env::var("DFMCP_JOBS_TOKEN").map_err(|_| {
            error(
                ErrorCode::CapabilityDenied,
                "DFMCP_JOBS_TOKEN must be configured",
            )
        })?;
        let mut source = JobsRpcClient::connect(
            endpoint,
            token.into_bytes(),
            id.get().to_be_bytes().to_vec(),
            Duration::from_millis(budget.max_wall_millis),
            maximum,
            budget.max_bytes as usize,
        )?;
        let mut state = LiveJobsState::default();
        state.publish(source.refresh(Duration::from_millis(budget.max_wall_millis))?)?;
        let fortress = state
            .snapshot()
            .ok_or_else(|| {
                error(
                    ErrorCode::InternalInvariantViolation,
                    "jobs bootstrap lost snapshot",
                )
            })?
            .fortress_id;
        let grants = capabilities
            .iter()
            .map(|capability| CapabilityGrant {
                capability: *capability,
                scope: CapabilityScope {
                    fortress_id: Some(fortress),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect();
        let mut session = JobSession {
            id,
            source: Box::new(source),
            state,
            budget,
            grants,
            request: 0,
            _slot: slot,
        };
        let context = session.context()?;
        let output = tool_packet(
            Some(&session),
            Some(&context),
            "fortress.open_session",
            json!({"ok":true,
            "granted_capabilities":capabilities.iter().map(|capability|capability.as_str()).collect::<Vec<_>>(),
            "query_help":{"mode":"schema"},"max_jobs":maximum,"max_output_tokens":budget.max_output_tokens,
            "maximum_bytes":budget.max_bytes,"max_wall_millis":budget.max_wall_millis}),
        )?;
        let mut registry = lock(&SESSIONS)?;
        if registry.contains_key(&id) {
            return Err(error(
                ErrorCode::InternalInvariantViolation,
                "jobs session ID collision",
            ));
        }
        registry.insert(id, Arc::new(Mutex::new(session)));
        Ok(output)
    })();
    match result {
        Ok(text) => text,
        Err(failure) => error_packet(None, None, "fortress.open_session", &failure),
    }
}
fn observe(id: Option<String>, operation: &str) -> String {
    with_session(id, operation, Capability::Observe, |session, context| {
        let outcome = session.refresh(&context)?;
        let payload = json!({"ok":true,
            "kind":if outcome==JobPublication::Heartbeat {"heartbeat"} else {"snapshot"},
            "reset":outcome==JobPublication::Reset,"game_clock_controlled":false});
        let mut current = context.clone();
        current.anchor = session.anchor()?;
        if current
            .authorize(Capability::Query, RiskTier::ReadOnly, &[], None)
            .is_ok()
        {
            semantic_query::publish_with_active_work(&current, payload, |result| {
                tool_packet(Some(session), Some(&context), operation, result)
            })
        } else {
            tool_packet(Some(session), Some(&context), operation, payload)
        }
    })
}
#[tool(
    description = "Read and atomically publish one complete bounded job roster. A disappearing job is not proof of completion. Never unpauses the game."
)]
pub fn fortress_observe(session_id: Option<String>) -> String {
    observe(session_id, "fortress.observe")
}
#[tool(
    description = "Perform one bounded jobs observation, without controlling time or claiming completion. For conditions use query kind await_watch."
)]
pub fn fortress_wait(session_id: Option<String>) -> String {
    observe(session_id, "fortress.wait")
}

fn query_view(session: &JobSession, context: &OperationContext) -> Result<QueryResponseProjection> {
    let source = session
        .state
        .observation()
        .ok_or_else(|| error(ErrorCode::InternalInvariantViolation, "jobs source missing"))?;
    Ok(QueryResponseProjection {
        session_id: session.id.to_string(),
        request_id: context.request_id.to_string(),
        anchor: anchor_json(session.anchor()?),
        briefing: briefing(session),
        attention: Vec::new(),
        affordances: Vec::new(),
        uncertainty: vec![
            json!({"domain":"fortress.jobs.blockers","epistemic_state":"unknown",
            "reason":"no material-availability, path, building-completion or labor-eligibility observation"}),
        ],
        coverage: coverage(),
        budget: json!({"admitted":{"max_bytes":session.budget.max_bytes,
            "max_output_tokens":session.budget.max_output_tokens}}),
        references: vec![
            json!({"kind":"jobs_observation","digest":source.source_digest()?.to_string()}),
        ],
        maximum_bytes: packet_limit(session.budget),
    })
}
fn finish_query(view: &QueryResponseProjection, result: Value) -> Result<String> {
    let encoded = view.finish(result)?;
    let mut payload: Value = serde_json::from_str(&encoded).map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "query response cannot be decoded",
        )
    })?;
    payload["agent_turn"]["turn_id"] = json!(format!("jobs-turn-{}", view.request_id));
    let encoded = payload.to_string();
    if encoded.len() > view.maximum_bytes {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "jobs query packet overflow",
        ));
    }
    Ok(encoded)
}
#[tool(
    description = "Query current job facts, aggregates, search, baselines, or condition watches. Use mode=schema for the shared query envelope. This profile observes jobs only, not citizens or inventory."
)]
pub fn fortress_query(
    session_id: Option<String>,
    mode: Option<String>,
    query: Option<Value>,
) -> String {
    with_session(
        session_id,
        "fortress.query",
        Capability::Query,
        |session, mut context| {
            if mode.is_some() && query.is_some() {
                return Err(error(
                    ErrorCode::InvalidRequest,
                    "do not mix mode and query",
                ));
            }
            let schema = mode.as_deref() == Some("schema");
            if mode.is_some() && !schema && mode.as_deref() != Some("summary") {
                return Err(error(
                    ErrorCode::InvalidRequest,
                    "jobs mode must be summary or schema",
                ));
            }
            let mut input = query.unwrap_or_else(||json!({"schema":"dfmcp.query/1","query":{"kind":"aggregate","kinds":["job"],"group_by":{"kind":"field","field":"suspended"}}}));
            let kind = input["query"]["kind"].as_str().unwrap_or("");
            let local = matches!(
                kind,
                "watches" | "cancel_watch" | "release_watch" | "baselines" | "release_baseline"
            );
            if session.source.poisoned() && !local && !schema {
                return Err(error(
                    ErrorCode::AdapterUnavailable,
                    "jobs source fenced; reopen or manage retained local records",
                ));
            }
            let mut refreshed = None;
            if !schema && kind == "await_watch" {
                let snapshot = session.state.snapshot().ok_or_else(|| {
                    error(
                        ErrorCode::InternalInvariantViolation,
                        "jobs snapshot missing",
                    )
                })?;
                let needed = semantic_query::prepare_await(snapshot, &context, &input)?;
                if needed {
                    let basis = context.anchor;
                    let outcome = session.refresh(&context)?;
                    context.anchor = session.anchor()?;
                    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
                    refreshed = Some(
                        json!({"basis":anchor_json(basis),"reset":outcome==JobPublication::Reset,
                    "kind":if outcome==JobPublication::Heartbeat {"heartbeat"} else {"snapshot"}}),
                    );
                }
                if let Some(object) = input.as_object_mut() {
                    object.remove("expected_anchor");
                }
                input["query"]["kind"] = json!("poll_watch");
            }
            let view = query_view(session, &context)?;
            let mut narrowed = context.clone();
            narrowed.budget.max_bytes = view.result_byte_budget()? as u64;
            if schema {
                let schema: Value =
                    serde_json::from_str(include_str!("../../../schemas/mcp_query_v1.json"))
                        .map_err(|_| {
                            error(
                                ErrorCode::InternalInvariantViolation,
                                "invalid embedded query schema",
                            )
                        })?;
                return semantic_query::publish_with_active_work(
                    &context,
                    json!({"schema":schema,
                "profile":"jobs-only","mode":"schema","source_stale":session.source.poisoned(),
                "example":{"schema":"dfmcp.query/1","query":{"kind":"entities","kinds":["job"],
                    "fields":["type_key","suspended","worker_assigned","holder_native_id"],"limit":4}},
                "truncated":false,"continuation":null}),
                    |result| finish_query(&view, result),
                );
            }
            let snapshot = session.state.snapshot().ok_or_else(|| {
                error(
                    ErrorCode::InternalInvariantViolation,
                    "jobs snapshot missing",
                )
            })?;
            semantic_query::execute_with_publisher(snapshot, &narrowed, &input, |mut result| {
                result["source_stale"] = json!(session.source.poisoned());
                if let Some(refresh) = refreshed {
                    result["observation_refresh"] = refresh;
                }
                finish_query(&view, result)
            })
        },
    )
}
#[tool(
    description = "Explain jobs-profile coverage and provenance. Inspect a specific generation with fortress.query kind inspect."
)]
pub fn fortress_explain(session_id: Option<String>) -> String {
    with_session(
        session_id,
        "fortress.explain",
        Capability::Query,
        |session, context| {
            tool_packet(
                Some(session),
                Some(&context),
                "fortress.explain",
                json!({"ok":true,
            "source":"DFHack native global job list, one suspended RPC",
            "unknown":["why suspended","material availability","reachability","completed successfully"],
            "native_references":"worker_native_id and holder_native_id are not handles into other runtime snapshots",
            "fields":["native_job_id","job_type","type_key","reaction","suspended","repeating","position",
                "worker_assigned","worker_native_id","holder_native_id","attached_item_count","required_item_filter_count","completion_timer_raw"]}),
            )
        },
    )
}
#[tool(
    description = "Report the jobs source connection, current anchor and explicit unadmitted coverage; performs no network repair or game effect."
)]
pub fn fortress_doctor(session_id: Option<String>) -> String {
    with_session(
        session_id,
        "fortress.doctor",
        Capability::Doctor,
        |session, context| {
            tool_packet(
                Some(session),
                Some(&context),
                "fortress.doctor",
                json!({"ok":true,
            "status":if session.source.poisoned(){"source_fenced"}else{"read_only_unadmitted"},
            "native_qualified":false,"live_qualified":false,"production_admitted":false}),
            )
        },
    )
}
fn no_effect(id: Option<String>, operation: &str) -> String {
    with_session(id, operation, Capability::Query, |_, _| {
        Err(error(
            ErrorCode::CapabilityDenied,
            "jobs/1.2 is read-only; no prepare, commit, cancellation, checkpoint or restore effect exists",
        ))
    })
}
#[tool(description = "Unavailable: jobs/1.2 has no game mutation or live planning authority.")]
pub fn fortress_plan(session_id: Option<String>) -> String {
    no_effect(session_id, "fortress.plan")
}
#[tool(description = "Unavailable: no job changes are committed by this read-only profile.")]
pub fn fortress_commit(session_id: Option<String>) -> String {
    no_effect(session_id, "fortress.commit")
}
#[tool(
    description = "Unavailable for game actions. Cancel a foreground condition using query kind cancel_watch."
)]
pub fn fortress_cancel(session_id: Option<String>) -> String {
    no_effect(session_id, "fortress.cancel")
}
#[tool(description = "Unavailable: no game or save checkpoint is created.")]
pub fn fortress_checkpoint(session_id: Option<String>) -> String {
    no_effect(session_id, "fortress.checkpoint")
}
#[tool(description = "Unavailable: no game or save state is restored.")]
pub fn fortress_restore(session_id: Option<String>) -> String {
    no_effect(session_id, "fortress.restore")
}

/// Public entry is independently opt-in gated even when invoked as a library.
pub fn run_stdio() {
    if let Err(failure) = validate_environment() {
        eprintln!("{failure}");
        std::process::exit(1);
    }
    let server=ServerBuilder::new("dfmcp-live-jobs-dev",env!("CARGO_PKG_VERSION"))
        .tool(FortressOpenSession).tool(FortressObserve).tool(FortressQuery).tool(FortressPlan)
        .tool(FortressCommit).tool(FortressWait).tool(FortressCancel).tool(FortressCheckpoint)
        .tool(FortressRestore).tool(FortressExplain).tool(FortressDoctor).request_timeout(60)
        .instructions("Explicitly unadmitted jobs-only read profile. Open a session first. Query suspended jobs, group counts by type or holder, capture changes, and register foreground conditions. Missing jobs do not prove completion. No citizen, inventory, map, or game mutation authority exists. Endpoint and token are operator process configuration.")
        .build();
    crate::run_modern_stdio(server);
}

#[cfg(test)]
mod tests {
    use super::*;
    use dfmcp_adapter::live_jobs::LiveJob;
    use dfmcp_core::MapCoord;
    use std::collections::VecDeque;
    struct Script {
        values: VecDeque<LiveJobObservation>,
        poisoned: bool,
    }
    impl JobSource for Script {
        fn read(&mut self, _: Duration) -> Result<LiveJobObservation> {
            self.values
                .pop_front()
                .ok_or_else(|| error(ErrorCode::AdapterUnavailable, "fixture exhausted"))
        }
        fn poisoned(&self) -> bool {
            self.poisoned
        }
        fn fence(&mut self) {
            self.poisoned = true;
        }
    }
    fn observation(suspended: bool, tick: u32) -> LiveJobObservation {
        LiveJobObservation {
            bridge_generation: 7,
            df_version: "test-df".to_owned(),
            dfhack_version: "test-dfhack".to_owned(),
            year: 105,
            year_tick: tick,
            paused: true,
            site_id: 1,
            world_folder: "region1".to_owned(),
            next_job_id: 4,
            jobs: vec![LiveJob {
                native_id: 0,
                job_type: 5,
                type_key: "Dig".to_owned(),
                reaction: String::new(),
                suspended,
                repeating: false,
                position: MapCoord::new(1, 2, 3),
                worker_native_id: None,
                holder_native_id: Some(4),
                completion_timer: -1,
                attached_item_count: 0,
                required_item_filter_count: 1,
            }],
        }
    }
    fn register() -> Result<SessionId> {
        let id = next_id()?;
        let mut state = LiveJobsState::default();
        state.publish(observation(true, 1))?;
        let fortress = state
            .snapshot()
            .ok_or_else(|| error(ErrorCode::InternalInvariantViolation, "fixture"))?
            .fortress_id;
        let session = JobSession {
            id,
            source: Box::new(Script {
                values: VecDeque::from([observation(false, 2)]),
                poisoned: false,
            }),
            state,
            budget: WorkBudget {
                max_bytes: 32768,
                max_output_tokens: 8192,
                ..WorkBudget::default()
            },
            grants: [Capability::Observe, Capability::Query, Capability::Doctor]
                .into_iter()
                .map(|capability| CapabilityGrant {
                    capability,
                    scope: CapabilityScope {
                        fortress_id: Some(fortress),
                        ..CapabilityScope::default()
                    },
                    max_risk: RiskTier::ReadOnly,
                    expires_at_tick: None,
                    remaining_uses: None,
                })
                .collect(),
            request: 0,
            _slot: SessionSlot::reserve()?,
        };
        lock(&SESSIONS)?.insert(id, Arc::new(Mutex::new(session)));
        Ok(id)
    }
    fn decode(text: &str) -> Result<Value> {
        serde_json::from_str(text)
            .map_err(|_| error(ErrorCode::InternalInvariantViolation, "fixture JSON"))
    }
    #[test]
    fn actual_query_handlers_filter_refresh_and_keep_effects_unavailable() -> Result<()> {
        let id = register()?;
        let handle = Some(id.to_string());
        let input = json!({"schema":"dfmcp.query/1","query":{"kind":"entities","kinds":["job"],"fields":["suspended"],
            "where":{"op":"compare","field":"suspended","comparison":"eq","value":{"type":"bool","value":true}}}});
        let before = decode(&fortress_query(handle.clone(), None, Some(input.clone())))?;
        assert_eq!(before["matched"], 1);
        assert_eq!(before["agent_turn"]["briefing"]["runtime_admitted"], false);
        assert_eq!(decode(&fortress_observe(handle.clone()))?["ok"], true);
        assert_eq!(
            decode(&fortress_query(handle.clone(), None, Some(input)))?["matched"],
            0
        );
        assert_eq!(
            decode(&fortress_commit(handle.clone()))?["error"]["code"],
            "capability_denied"
        );
        assert_eq!(decode(&fortress_observe(handle.clone()))?["ok"], false);
        assert_eq!(
            decode(&fortress_query(handle.clone(), None, None))?["ok"],
            false
        );
        assert_eq!(decode(&fortress_doctor(handle))?["status"], "source_fenced");
        lock(&SESSIONS)?.remove(&id);
        Ok(())
    }
    #[test]
    fn baseline_changes_use_the_actual_native_job_projection() -> Result<()> {
        let id = register()?;
        let handle = Some(id.to_string());
        let capture = decode(&fortress_query(
            handle.clone(),
            None,
            Some(json!({"schema":"dfmcp.query/1","query":{
            "kind":"capture","key":"suspended-jobs","max_game_ticks":100,"select":{"kind":"entities","kinds":["job"],"fields":["suspended"]}}})),
        ))?;
        assert_eq!(capture["ok"], true);
        assert_eq!(decode(&fortress_wait(handle.clone()))?["ok"], true);
        let changes = decode(&fortress_query(
            handle,
            None,
            Some(json!({"schema":"dfmcp.query/1","query":{
            "kind":"changes","baseline":capture["captured"]["baseline"]}})),
        ))?;
        assert_eq!(changes["ok"], true);
        assert_eq!(changes["change_count"], 1);
        assert_eq!(changes["agent_turn"]["continuity"]["status"], "partial");
        lock(&SESSIONS)?.remove(&id);
        Ok(())
    }
    #[test]
    fn await_watch_consumes_one_job_observation_and_terminal_retry_skips_io() -> Result<()> {
        let id = register()?;
        let handle = Some(id.to_string());
        let deadline = 105u64 * 403_200 + 50;
        let created = decode(&fortress_query(
            handle.clone(),
            None,
            Some(json!({"schema":"dfmcp.query/1","query":{
            "kind":"watch","key":"resume-dig","label":"Observe unsuspended job",
            "condition":{"op":"field","entity_id":"2","generation":1,"field":"suspended",
                "comparison":"eq","value":{"type":"bool","value":false}},
            "deadline_tick":deadline,"poll_interval_ticks":1,"stable_observations":1}})),
        ))?;
        assert_eq!(created["ok"], true);
        assert_eq!(created["record"]["terminal"], false);
        let request = json!({"schema":"dfmcp.query/1","query":{"kind":"await_watch","watch":created["record"]["watch"]}});
        let satisfied = decode(&fortress_query(handle.clone(), None, Some(request.clone())))?;
        assert_eq!(satisfied["ok"], true);
        assert_eq!(satisfied["record"]["terminal"], true);
        assert_eq!(satisfied["observation_refresh"]["kind"], "snapshot");
        let repeated = decode(&fortress_query(handle.clone(), None, Some(request)))?;
        assert_eq!(repeated["ok"], true);
        assert_eq!(repeated["record"], satisfied["record"]);
        assert!(repeated["observation_refresh"].is_null());
        assert_eq!(
            decode(&fortress_doctor(handle))?["status"],
            "read_only_unadmitted"
        );
        lock(&SESSIONS)?.remove(&id);
        Ok(())
    }
    #[test]
    fn local_watch_cancellation_remains_available_after_source_failure() -> Result<()> {
        let id = register()?;
        let handle = Some(id.to_string());
        let created = decode(&fortress_query(
            handle.clone(),
            None,
            Some(json!({"schema":"dfmcp.query/1","query":{
            "kind":"watch","key":"pending-clock","label":"Wait for later tick",
            "condition":{"op":"tick_at_least","value":105u64*403_200+20},
            "deadline_tick":105u64*403_200+50,"poll_interval_ticks":1,"stable_observations":1}})),
        ))?;
        assert_eq!(created["ok"], true);
        let observed = decode(&fortress_observe(handle.clone()))?;
        assert_eq!(observed["ok"], true);
        assert!(
            observed["agent_turn"]["active_work"]["obligations"]
                .as_array()
                .is_some_and(|work| !work.is_empty())
        );
        assert_eq!(decode(&fortress_observe(handle.clone()))?["ok"], false);
        let cancelled = decode(&fortress_query(
            handle,
            None,
            Some(json!({"schema":"dfmcp.query/1","query":{
            "kind":"cancel_watch","watch":created["record"]["watch"]}})),
        ))?;
        assert_eq!(cancelled["ok"], true);
        assert_eq!(cancelled["record"]["terminal"], true);
        assert_eq!(cancelled["agent_turn"]["continuity"]["status"], "stale");
        lock(&SESSIONS)?.remove(&id);
        Ok(())
    }
    #[test]
    fn development_and_capability_boundaries_do_not_widen() {
        assert!(!allowed_environment("DFMCP_ADMISSION_TICKET"));
        assert!(!allowed_environment("DFMCP_ADMITTED_BRIDGE_PROTOCOL"));
        assert!(allowed_environment("DFMCP_JOBS_TOKEN"));
        assert!(read_capabilities(Some(vec!["control_clock".to_owned()])).is_err());
        assert!(resolve(Some("11000000000000000000000000000001".to_owned())).is_err());
        let raw_alias = format!("{:032x}", (1u128 << 127) | FAMILY | 1);
        assert!(
            matches!(resolve(Some(raw_alias)),Err(failure) if failure.code==ErrorCode::InvalidRequest)
        );
    }
    #[test]
    fn unbound_error_does_not_claim_complete_job_coverage() -> Result<()> {
        let result = decode(&fortress_query(None, None, None))?;
        assert_eq!(result["ok"], false);
        assert_eq!(
            result["agent_turn"]["coverage"]["complete_domains"],
            json!([])
        );
        assert_eq!(result["agent_turn"]["briefing"]["fortress_loaded"], false);
        Ok(())
    }
}
