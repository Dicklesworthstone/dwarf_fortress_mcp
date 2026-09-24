//! Explicitly unadmitted map/1.5 runtime. One fixed region per session; no
//! separately timed operation snapshots are merged into this terrain observation.
#[path = "map_queries.rs"]
mod map_queries;
#[path = "query_response.rs"]
mod query_response;
#[path = "semantic_query.rs"]
mod semantic_query;

use crate::agent_turn::{AgentPhase, AgentTurnBuilder, ContinuityStatus, empty_active_work};
use dfmcp_adapter::live_jobs::JobPublication;
use dfmcp_adapter::live_jobs_rpc::{DeadlineStream, operations::map::MapRpcClient};
use dfmcp_adapter::live_map::{LiveMapObservation, LiveMapState, MAX_MAP_BYTES, map_error};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, DfmcpError, ErrorCode, OperationContext,
    RequestId, Result, RiskTier, SessionId, StateAnchor, WorkBudget,
};
use dfmcp_world::map_region::{Cell, Region};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use query_response::QueryResponseProjection;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::Duration;

const FAMILY: u128 = 1u128 << 59;
static NEXT: Mutex<u128> = Mutex::new(1);
static SLOTS: AtomicUsize = AtomicUsize::new(0);
static SESSIONS: LazyLock<Mutex<BTreeMap<SessionId, Arc<Mutex<MapSession>>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
fn error(code: ErrorCode, text: &str) -> DfmcpError {
    DfmcpError::new(code, text)
}
fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>> {
    mutex
        .lock()
        .map_err(|_| error(ErrorCode::InternalInvariantViolation, "map mutex poisoned"))
}
struct Slot;
impl Slot {
    fn reserve() -> Result<Self> {
        SLOTS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 2).then_some(n + 1)
            })
            .map_err(|_| {
                error(
                    ErrorCode::BudgetExceeded,
                    "map runtime has two retained sessions",
                )
            })?;
        Ok(Self)
    }
}
impl Drop for Slot {
    fn drop(&mut self) {
        SLOTS.fetch_sub(1, Ordering::AcqRel);
    }
}
trait MapSource: Send {
    fn read(&mut self, timeout: Duration) -> Result<LiveMapObservation>;
    fn poisoned(&self) -> bool;
    fn fence(&mut self);
}
impl MapSource for MapRpcClient<DeadlineStream> {
    fn read(&mut self, t: Duration) -> Result<LiveMapObservation> {
        self.refresh(t)
    }
    fn poisoned(&self) -> bool {
        MapRpcClient::poisoned(self)
    }
    fn fence(&mut self) {
        MapRpcClient::fence(self);
    }
}
struct MapSession {
    id: SessionId,
    source: Box<dyn MapSource>,
    state: LiveMapState,
    budget: WorkBudget,
    grants: Vec<CapabilityGrant>,
    request: u128,
    _slot: Slot,
}
impl MapSession {
    fn anchor(&self) -> Result<StateAnchor> {
        self.state.snapshot().map(|v| v.anchor()).ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "map snapshot missing",
            )
        })
    }
    fn context(&mut self) -> Result<OperationContext> {
        self.request = self
            .request
            .checked_add(1)
            .ok_or_else(|| error(ErrorCode::BudgetExceeded, "map request IDs exhausted"))?;
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
            return Err(error(ErrorCode::StaleAnchor, "map refresh anchor changed"));
        }
        if self.source.poisoned() {
            return Err(error(
                ErrorCode::AdapterUnavailable,
                "map source fenced; reopen session",
            ));
        }
        let result = self
            .source
            .read(Duration::from_millis(context.budget.max_wall_millis))
            .and_then(|v| {
                if v.map.cells.len().saturating_add(1) > context.budget.max_entities as usize {
                    return Err(error(ErrorCode::BudgetExceeded, "map tile budget exceeded"));
                }
                self.state.publish(v)
            });
        if result.is_err() {
            self.source.fence();
        }
        result
    }
}
fn next_id() -> Result<SessionId> {
    let mut n = lock(&NEXT)?;
    if *n >= FAMILY {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "map session IDs exhausted",
        ));
    }
    let id = SessionId::new((1u128 << 127) | FAMILY | *n);
    *n += 1;
    Ok(id)
}
fn resolve(raw: Option<String>) -> Result<Arc<Mutex<MapSession>>> {
    let raw = raw.ok_or_else(|| {
        error(
            ErrorCode::InvalidRequest,
            "open a map session and supply session_id",
        )
    })?;
    if raw.len() != 32 || !raw.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(error(
            ErrorCode::InvalidRequest,
            "invalid map session handle",
        ));
    }
    let value = u128::from_str_radix(&raw, 16)
        .map_err(|_| error(ErrorCode::InvalidRequest, "invalid session encoding"))?;
    let id = SessionId::new(value);
    if id.get() != value || !id.is_process_scoped_live() || (value & ((1u128 << 62) - 1)) >> 59 != 1
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "not an encoded map session",
        ));
    }
    lock(&SESSIONS)?
        .get(&id)
        .cloned()
        .ok_or_else(|| error(ErrorCode::SessionNotFound, "map session not found"))
}
fn allowed_environment(name: &str) -> bool {
    !name.starts_with("DFMCP_")
        || matches!(
            name,
            "DFMCP_ALLOW_UNADMITTED_MAP_V1_5" | "DFMCP_MAP_TOKEN" | "DFMCP_MAP_ENDPOINT"
        )
}
fn validate_environment() -> Result<()> {
    if std::env::var("DFMCP_ALLOW_UNADMITTED_MAP_V1_5")
        .ok()
        .as_deref()
        != Some("1")
        || std::env::vars_os().any(|(k, _)| !allowed_environment(&k.to_string_lossy()))
        || crate::admission::current_admission_provenance().is_some()
    {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "map/1.5 requires its exact development opt-in and refuses other DFMCP/admission settings",
        ));
    }
    Ok(())
}
fn anchor_json(a: StateAnchor) -> Value {
    json!({"fortress_id":a.fortress_id.to_string(),"epoch":a.cursor.epoch,"sequence":a.cursor.sequence,
    "game_tick":a.tick.0,"state_hash":a.state_hash.to_string()})
}
fn briefing(session: &MapSession) -> Value {
    let v = session.state.observation();
    json!({"runtime":"unadmitted_development","bridge_protocol":"1.5","observation_profile":"map-region",
        "runtime_admitted":false,"mutation_admissible":false,"read_only":true,"source_poisoned":session.source.poisoned(),
        "region":v.map(|v|json!({"origin":v.map.region.origin,"size":v.map.region.size})),
        "visible_tiles":v.map(|v|v.map.cells.iter().filter(|c|matches!(c,Cell::Visible(_))).count()),
        "hidden_tiles":v.map(|v|v.map.cells.iter().filter(|c|**c==Cell::Hidden).count()),
        "unallocated_tiles":v.map(|v|v.map.cells.iter().filter(|c|**c==Cell::Unallocated).count())})
}
fn coverage() -> Value {
    json!({"status":"partial","complete_domains":["requested_region.cell_presence"],
    "partial_domains":[{"domain":"fortress.map","reason":"one bounded region; hidden attributes redacted and unallocated blocks unknown"}],
    "omitted_domains":["outside_region","unit_path_rules","jobs","inventory","citizens","terrain_history"],"continuation":null})
}
fn packet(
    session: Option<&MapSession>,
    context: Option<&OperationContext>,
    operation: &str,
    mut value: Value,
) -> Result<String> {
    let mut work = empty_active_work();
    if let Some(v) = value
        .as_object_mut()
        .and_then(|m| m.remove("_condition_watch_work"))
    {
        work["obligations"] = v;
    }
    let mut builder = AgentTurnBuilder::new(operation, AgentPhase::Inspect).active_work(work);
    let mut maximum = 8192;
    if let (Some(s), Some(c)) = (session, context) {
        let a = s.anchor()?;
        maximum = s
            .budget
            .max_bytes
            .min(u64::from(s.budget.max_output_tokens) * 4) as usize;
        value["session_id"] = json!(s.id.to_string());
        value["anchor"] = anchor_json(a);
        let reset = value["reset"] == true;
        builder = builder
            .session_id(s.id.to_string())
            .request_id(c.request_id.to_string())
            .anchor(anchor_json(a))
            .briefing(briefing(s))
            .coverage(coverage())
            .continuity(
                if s.source.poisoned() {
                    ContinuityStatus::Stale
                } else if reset {
                    ContinuityStatus::Reset
                } else if value["kind"] == "heartbeat" {
                    ContinuityStatus::Heartbeat
                } else {
                    ContinuityStatus::Continuous
                },
                Some(anchor_json(c.anchor)),
                None,
                reset.then(|| "map_clock_generation_or_dimensions_reset".to_owned()),
            );
    } else {
        builder=builder.briefing(json!({"runtime_admitted":false,"mutation_admissible":false,"observation_profile":"map-region"}));
    }
    let out = builder.attach(value);
    if out.len() > maximum {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "complete map packet exceeds budget",
        ));
    }
    Ok(out)
}
fn failure(
    session: Option<&MapSession>,
    context: Option<&OperationContext>,
    operation: &str,
    e: &DfmcpError,
) -> String {
    let value = json!({"ok":false,"error":{"code":e.code.as_str(),"message":e.message,"operation":operation,"mutation_dispatched":false}});
    let result = match (session, context) {
        (Some(s), Some(c))
            if c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)
                .is_ok() =>
        {
            semantic_query::publish_with_active_work(c, value, |v| {
                packet(Some(s), Some(c), operation, v)
            })
        }
        _ => packet(None, None, operation, value),
    };
    result.unwrap_or_else(|_|AgentTurnBuilder::new(operation,AgentPhase::Inspect).attach(json!({"ok":false,
        "error":{"code":"budget_exceeded","message":"required response could not fit; retained work is unchanged"}})))
}
fn with_session<F>(id: Option<String>, operation: &str, capability: Capability, body: F) -> String
where
    F: FnOnce(&mut MapSession, OperationContext) -> Result<String>,
{
    let handle = match resolve(id) {
        Ok(v) => v,
        Err(e) => return failure(None, None, operation, &e),
    };
    let mut s = match lock(&handle) {
        Ok(v) => v,
        Err(e) => return failure(None, None, operation, &e),
    };
    let c = match s.context() {
        Ok(v) => v,
        Err(e) => return failure(None, None, operation, &e),
    };
    if let Err(e) = c.authorize(capability, RiskTier::ReadOnly, &[], None) {
        return failure(None, None, operation, &e);
    }
    match body(&mut s, c.clone()) {
        Ok(v) => v,
        Err(e) => {
            let mut current = c;
            if let Ok(a) = s.anchor() {
                current.anchor = a;
            }
            failure(Some(&s), Some(&current), operation, &e)
        }
    }
}
fn capabilities(input: Option<Vec<String>>) -> Result<Vec<Capability>> {
    let input = input.unwrap_or_else(|| {
        vec![
            "observe".to_owned(),
            "query".to_owned(),
            "doctor".to_owned(),
        ]
    });
    if input.is_empty() || input.len() > 3 {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "request one to three read capabilities",
        ));
    }
    let mut out = Vec::new();
    for n in input {
        let c = match n.as_str() {
            "observe" => Capability::Observe,
            "query" => Capability::Query,
            "doctor" => Capability::Doctor,
            _ => {
                return Err(error(
                    ErrorCode::CapabilityDenied,
                    "map profile cannot grant that capability",
                ));
            }
        };
        if out.contains(&c) {
            return Err(error(ErrorCode::InvalidRequest, "duplicate capability"));
        }
        out.push(c);
    }
    Ok(out)
}
#[tool(
    description = "Open an unadmitted read-only map/1.5 session for exactly one region: {origin:[x,y,z],size:[width,height,depth]}. At most 16384 tiles, each side at most 128. Hidden attributes are redacted. No game effects."
)]
pub fn fortress_open_session(
    region: Value,
    max_output_tokens: Option<u32>,
    max_wall_millis: Option<u64>,
    requested_capabilities: Option<Vec<String>>,
) -> String {
    let result = (|| -> Result<String> {
        validate_environment()?;
        let region = map_queries::parse_region(&region)?;
        let caps = capabilities(requested_capabilities)?;
        let budget = WorkBudget {
            max_entities: region.volume().map_err(map_error)? as u32 + 1,
            max_bytes: MAX_MAP_BYTES as u64,
            max_output_tokens: max_output_tokens.unwrap_or(8192),
            max_wall_millis: max_wall_millis.unwrap_or(5000),
            ..WorkBudget::default()
        };
        budget.validate()?;
        if !(2048..=65536).contains(&budget.max_output_tokens)
            || !(1..=60000).contains(&budget.max_wall_millis)
        {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "map session budget outside bounds",
            ));
        }
        let slot = Slot::reserve()?;
        let id = next_id()?;
        let endpoint = dfmcp_adapter::parse_loopback_endpoint(
            &std::env::var("DFMCP_MAP_ENDPOINT").unwrap_or_else(|_| "127.0.0.1:5000".to_owned()),
        )?;
        let token = std::env::var("DFMCP_MAP_TOKEN").map_err(|_| {
            error(
                ErrorCode::CapabilityDenied,
                "DFMCP_MAP_TOKEN must be configured",
            )
        })?;
        let timeout = Duration::from_millis(budget.max_wall_millis);
        let mut source = MapRpcClient::connect(
            endpoint,
            token.into_bytes(),
            id.get().to_be_bytes().to_vec(),
            region,
            MAX_MAP_BYTES,
            timeout,
        )?;
        let mut state = LiveMapState::default();
        state.publish(source.refresh(timeout)?)?;
        let fortress = state
            .snapshot()
            .ok_or_else(|| {
                error(
                    ErrorCode::InternalInvariantViolation,
                    "map bootstrap lost snapshot",
                )
            })?
            .fortress_id;
        let grants = caps
            .iter()
            .map(|c| CapabilityGrant {
                capability: *c,
                scope: CapabilityScope {
                    fortress_id: Some(fortress),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect();
        let mut s = MapSession {
            id,
            source: Box::new(source),
            state,
            budget,
            grants,
            request: 0,
            _slot: slot,
        };
        let c = s.context()?;
        let out = packet(
            Some(&s),
            Some(&c),
            "fortress.open_session",
            json!({"ok":true,"granted_capabilities":caps.iter().map(|c|c.as_str()).collect::<Vec<_>>(),
            "schema_discovery":{"tool":"fortress.query","arguments":{"session_id":id.to_string(),"mode":"schema"}}}),
        )?;
        let mut registry = lock(&SESSIONS)?;
        if registry.contains_key(&id) {
            return Err(error(
                ErrorCode::InternalInvariantViolation,
                "map session collision",
            ));
        }
        registry.insert(id, Arc::new(Mutex::new(s)));
        Ok(out)
    })();
    match result {
        Ok(v) => v,
        Err(e) => failure(None, None, "fortress.open_session", &e),
    }
}
fn observe(id: Option<String>, operation: &str) -> String {
    with_session(id, operation, Capability::Observe, |s, c| {
        let outcome = s.refresh(&c)?;
        let mut target = c.clone();
        target.anchor = s.anchor()?;
        target.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        let value = json!({"ok":true,"kind":if outcome==JobPublication::Heartbeat{"heartbeat"}else{"snapshot"},"reset":outcome==JobPublication::Reset,
        "native_observations":1,"game_clock_controlled":false});
        if target
            .authorize(Capability::Query, RiskTier::ReadOnly, &[], None)
            .is_ok()
        {
            semantic_query::publish_with_active_work(&target, value, |v| {
                packet(Some(s), Some(&c), operation, v)
            })
        } else {
            packet(Some(s), Some(&c), operation, value)
        }
    })
}
#[tool(
    description = "Refresh the same bounded map region once. No unpause, reveal, block allocation or movement is performed."
)]
pub fn fortress_observe(session_id: Option<String>) -> String {
    observe(session_id, "fortress.observe")
}
#[tool(
    description = "Read one new terrain observation, without controlling game time. Use query await_watch for a foreground condition."
)]
pub fn fortress_wait(session_id: Option<String>) -> String {
    observe(session_id, "fortress.wait")
}
fn view(s: &MapSession, c: &OperationContext) -> Result<QueryResponseProjection> {
    let source = s
        .state
        .observation()
        .ok_or_else(|| error(ErrorCode::InternalInvariantViolation, "map source missing"))?;
    Ok(QueryResponseProjection {
        session_id: s.id.to_string(),
        request_id: c.request_id.to_string(),
        anchor: anchor_json(s.anchor()?),
        briefing: briefing(s),
        attention: Vec::new(),
        affordances: Vec::new(),
        coverage: coverage(),
        uncertainty: vec![
            json!({"domain":"unit_navigation","epistemic_state":"unknown",
            "reason":"candidate geometry does not model doors, ramps, diagonals, unit abilities, temperatures or outside-region paths"}),
        ],
        budget: json!({"admitted":{"max_bytes":s.budget.max_bytes,"max_output_tokens":s.budget.max_output_tokens}}),
        references: vec![
            json!({"kind":"map_observation","digest":source.source_digest()?.to_string()}),
        ],
        maximum_bytes: s
            .budget
            .max_bytes
            .min(u64::from(s.budget.max_output_tokens) * 4) as usize,
    })
}
fn finish(v: &QueryResponseProjection, value: Value) -> Result<String> {
    let raw = v.finish(value)?;
    let mut value: Value = serde_json::from_str(&raw).map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "map query packet decode",
        )
    })?;
    value["agent_turn"]["turn_id"] = json!(format!("map-turn-{}", v.request_id));
    let raw = value.to_string();
    if raw.len() > v.maximum_bytes {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "map result exceeds budget",
        ));
    }
    Ok(raw)
}
#[tool(
    description = "Query map tile_feature facts, summaries, baselines and watches. Modes: summary, tiles, schema. Structured map_route returns a paged candidate route within the observed dry cardinal floor/stair model; never a unit path or global-unreachability proof."
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
        |s, mut c| {
            if mode.is_some() && query.is_some() {
                return Err(error(
                    ErrorCode::InvalidRequest,
                    "do not combine mode and query",
                ));
            }
            let schema = mode.as_deref() == Some("schema");
            let mut input = match query {
                Some(v) => v,
                None => match mode.as_deref() {
                    None | Some("summary" | "schema") => {
                        json!({"schema":"dfmcp.query/1","query":{"kind":"aggregate","kinds":["tile_feature"],"group_by":{"kind":"field","field":"visibility"}}})
                    }
                    Some("tiles") => {
                        json!({"schema":"dfmcp.query/1","query":{"kind":"entities","kinds":["tile_feature"],"fields":["position","visibility","shape","liquid_depth"],"limit":1}})
                    }
                    _ => {
                        return Err(error(
                            ErrorCode::InvalidRequest,
                            "map mode must be summary, tiles or schema",
                        ));
                    }
                },
            };
            let kind = input["query"]["kind"].as_str().unwrap_or("");
            let local = matches!(
                kind,
                "watches" | "cancel_watch" | "release_watch" | "baselines" | "release_baseline"
            );
            if s.source.poisoned() && !local && !schema {
                return Err(error(
                    ErrorCode::AdapterUnavailable,
                    "map source fenced; only local record management remains",
                ));
            }
            let mut refresh = None;
            if !schema && kind == "await_watch" {
                let snapshot = s.state.snapshot().ok_or_else(|| {
                    error(
                        ErrorCode::InternalInvariantViolation,
                        "map snapshot missing",
                    )
                })?;
                if semantic_query::prepare_await(snapshot, &c, &input)? {
                    let basis = c.anchor;
                    let outcome = s.refresh(&c)?;
                    c.anchor = s.anchor()?;
                    c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
                    c.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
                    refresh = Some(
                        json!({"basis":anchor_json(basis),"reset":outcome==JobPublication::Reset,"native_observations":1,
                    "kind":if outcome==JobPublication::Heartbeat{"heartbeat"}else{"snapshot"}}),
                    );
                }
                if let Some(o) = input.as_object_mut() {
                    o.remove("expected_anchor");
                }
                input["query"]["kind"] = json!("poll_watch");
            }
            let v = view(s, &c)?;
            let mut narrowed = c.clone();
            narrowed.budget.max_bytes = v.result_byte_budget()? as u64;
            if schema {
                return semantic_query::publish_with_active_work(
                    &c,
                    json!({"mode":"schema","query_schema":map_queries::schema()?,
            "profile":"map/1.5","source_stale":s.source.poisoned(),"truncated":false,"continuation":null}),
                    |r| finish(&v, r),
                );
            }
            if input["query"]["kind"] == "map_route" {
                let rc = semantic_query::result_context(&narrowed)?;
                let result = map_queries::route(s, &rc, &input)?;
                return semantic_query::publish_with_active_work(&narrowed, result, |r| {
                    finish(&v, r)
                });
            }
            let snapshot = s.state.snapshot().ok_or_else(|| {
                error(
                    ErrorCode::InternalInvariantViolation,
                    "map snapshot missing",
                )
            })?;
            semantic_query::execute_with_publisher(snapshot, &narrowed, &input, |mut r| {
                r["source_stale"] = json!(s.source.poisoned());
                if let Some(refresh) = refresh {
                    r["observation_refresh"] = refresh;
                }
                finish(&v, r)
            })
        },
    )
}
#[tool(description = "Explain map coverage, redaction and route limitations without native I/O.")]
pub fn fortress_explain(session_id: Option<String>) -> String {
    with_session(session_id, "fortress.explain", Capability::Query, |s, c| {
        semantic_query::publish_with_active_work(
            &c,
            json!({"ok":true,"source":"one suspended native read of a fixed region",
        "hidden":"attributes absent from wire and Redacted in canonical facts","unallocated":"Unknown, not empty or walkable",
        "route":"dry, unoccupied cardinal floors and complementary stairs only; no safety or unit-path proof",
        "joined_to_operations":false}),
            |r| packet(Some(s), Some(&c), "fortress.explain", r),
        )
    })
}
#[tool(description = "Inspect map connection health without reconnecting or modifying the game.")]
pub fn fortress_doctor(session_id: Option<String>) -> String {
    with_session(session_id, "fortress.doctor", Capability::Doctor, |s, c| {
        packet(
            Some(s),
            Some(&c),
            "fortress.doctor",
            json!({"ok":true,"status":if s.source.poisoned(){"source_fenced"}else{"read_only_unadmitted"},"production_admitted":false}),
        )
    })
}
fn no_effect(id: Option<String>, op: &str) -> String {
    with_session(id, op, Capability::Query, |_, _| {
        Err(error(
            ErrorCode::CapabilityDenied,
            "map profile has no movement, designation, reveal, clock or save mutation capability",
        ))
    })
}
#[tool(description = "Unavailable: candidate map routes are not executable game plans.")]
pub fn fortress_plan(session_id: Option<String>) -> String {
    no_effect(session_id, "fortress.plan")
}
#[tool(description = "Unavailable: no movement or terrain mutation is dispatched.")]
pub fn fortress_commit(session_id: Option<String>) -> String {
    no_effect(session_id, "fortress.commit")
}
#[tool(description = "Unavailable for game actions; query cancel_watch cancels a local watch.")]
pub fn fortress_cancel(session_id: Option<String>) -> String {
    no_effect(session_id, "fortress.cancel")
}
#[tool(description = "Unavailable: map observations do not create game checkpoints.")]
pub fn fortress_checkpoint(session_id: Option<String>) -> String {
    no_effect(session_id, "fortress.checkpoint")
}
#[tool(description = "Unavailable: map observations cannot restore game state.")]
pub fn fortress_restore(session_id: Option<String>) -> String {
    no_effect(session_id, "fortress.restore")
}
pub fn run_stdio() {
    if let Err(e) = validate_environment() {
        eprintln!("{e}");
        std::process::exit(1);
    }
    let server=ServerBuilder::new("dfmcp-live-map-dev",env!("CARGO_PKG_VERSION"))
        .tool(FortressOpenSession).tool(FortressObserve).tool(FortressQuery).tool(FortressPlan).tool(FortressCommit)
        .tool(FortressWait).tool(FortressCancel).tool(FortressCheckpoint).tool(FortressRestore).tool(FortressExplain).tool(FortressDoctor)
        .request_timeout(60).instructions("Unadmitted read-only map/1.5. Open a fixed bounded region first. Hidden terrain is redacted. Query tile_feature facts, capture changes and await foreground conditions. map_route is only a dry cardinal floor/stair candidate within the observed region, not DF unit pathfinding or a safety guarantee. Never join this anchor with separately timed operation snapshots. No game effects or production admission.").build();
    crate::run_modern_stdio(server);
}
#[cfg(test)]
#[path = "live_map_server_tests.rs"]
mod tests;
