//! Agent-oriented read-only MCP server over the authenticated DFHack bridge.
//!
//! This module preserves the frozen eleven-tool waist. Six tools are useful in
//! bridge protocol V1 (`open_session`, `observe`, `query`, `wait`, `explain`,
//! and `doctor`). The five mutation-stage tools remain registered so clients
//! never have to discover a different protocol shape, but they fail closed and
//! cannot reach a bridge effect path. Connection secrets and endpoints come
//! only from the process environment, never MCP arguments.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::agent_turn::{
    AgentPhase, AgentTurnBuilder, ContinuityStatus, ObservationProfile, RecoveryClass,
    empty_active_work, recommendation, recovery_guidance, uncertainty,
};
use dfmcp_adapter::{
    AuthenticatedLiveSource, BridgeCredentials, GameAdapter, InterestSet, LiveConnectionConfig,
    LiveReadAdapter, LiveReadBootstrapConfig, ObservationPayload, ObservationRequest,
    PrimedLiveSource, Projection, QueryRequest, bootstrap_live_read_adapter,
    connect_authenticated_live_source, parse_loopback_endpoint,
};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, DfmcpError, Digest32, EntityId, ErrorCode,
    FortressId, GameTick, ObservationCursor, OperationContext, RequestId, Result, RiskTier,
    SessionId, StateAnchor, WorkBudget,
};
use dfmcp_world::{
    EntityKind, Fact, FactPresence, FactSource, QueryOrder, Value as WorldValue, WorldQuery,
};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use serde_json::{Map as JsonMap, Value as JsonValue, json};

const MAX_LIVE_MCP_SESSIONS: usize = 32;
const MAX_CAPABILITY_REQUESTS: usize = 8;
const MAX_CAPABILITY_NAME_BYTES: usize = 64;
const MAX_RISK_NAME_BYTES: usize = 32;
const MAX_FORTRESS_SELECTOR_BYTES: usize = 20;
const MAX_MODE_BYTES: usize = 32;
const MAX_ENTITY_ID_BYTES: usize = 20;
const U128_HEX_ID_BYTES: usize = 32;
const LIVE_IMPLEMENTATION_PHASE: &str = "bridge_r0_authenticated_read_only";
const LIVE_BUDGET_CEILING: WorkBudget = WorkBudget {
    max_wall_millis: 60_000,
    max_game_ticks: 1_000_000,
    max_entities: 100_001,
    max_bytes: 64 * 1024 * 1024,
    max_output_tokens: 65_536,
    max_actions: 64,
};

const DEFAULT_CAPABILITIES: [(&str, &str); 3] = [
    ("observe", "read_only"),
    ("query", "read_only"),
    ("doctor", "read_only"),
];

const OMITTED_LIVE_DOMAINS: [(&str, &str); 7] = [
    ("fortress.items", "bridge protocol V1 does not observe items"),
    ("fortress.jobs", "bridge protocol V1 does not observe jobs"),
    ("fortress.map", "bridge protocol V1 does not observe map state"),
    ("fortress.economy", "bridge protocol V1 does not observe economy state"),
    (
        "fortress.welfare",
        "bridge protocol V1 does not observe detailed welfare state",
    ),
    (
        "fortress.military",
        "bridge protocol V1 does not observe military state",
    ),
    (
        "fortress.history",
        "bridge protocol V1 does not observe historical state",
    ),
];

type LiveMcpSource = PrimedLiveSource<AuthenticatedLiveSource>;
type LiveMcpAdapter = LiveReadAdapter<LiveMcpSource>;

struct LiveSession {
    session_id: SessionId,
    adapter: LiveMcpAdapter,
    grants: Vec<CapabilityGrant>,
    budget: WorkBudget,
    next_request_id: u128,
}

impl LiveSession {
    fn current_anchor(&self) -> Result<StateAnchor> {
        self.adapter.current_anchor().ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "registered live session has no canonical anchor",
            )
        })
    }

    fn next_context(&mut self) -> Result<(RequestId, OperationContext)> {
        let next = self.next_request_id.checked_add(1).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "live session request identifier space is exhausted",
            )
        })?;
        self.next_request_id = next;
        let request_id = RequestId::new(next);
        Ok((
            request_id,
            OperationContext {
                session_id: self.session_id,
                request_id,
                anchor: self.current_anchor()?,
                budget: self.budget,
                grants: self.grants.clone(),
                cancellation_requested: false,
            },
        ))
    }

    fn has_grant(&self, capability: Capability) -> bool {
        self.grants
            .iter()
            .any(|grant| grant.capability == capability)
    }

    fn source_poisoned(&self) -> bool {
        self.adapter.source().source().is_poisoned()
    }

    fn source_poison_reason(&self) -> Option<&str> {
        self.adapter.source().source().poisoned_reason()
    }
}

static LIVE_SESSIONS: LazyLock<Mutex<BTreeMap<SessionId, Arc<Mutex<LiveSession>>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
static NEXT_LIVE_SESSION_ID: LazyLock<Mutex<u128>> =
    LazyLock::new(|| Mutex::new(1u128 << 127));

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

fn sessions() -> Result<MutexGuard<'static, BTreeMap<SessionId, Arc<Mutex<LiveSession>>>>> {
    LIVE_SESSIONS.lock().map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "live session registry mutex is poisoned",
        )
    })
}

fn next_session_id() -> Result<SessionId> {
    let mut counter = NEXT_LIVE_SESSION_ID.lock().map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "live session identifier mutex is poisoned",
        )
    })?;
    let value = *counter;
    if value == 0 {
        return Err(error(
            ErrorCode::InternalInvariantViolation,
            "live session identifier zero is reserved",
        ));
    }
    *counter = value.checked_add(1).ok_or_else(|| {
        error(
            ErrorCode::BudgetExceeded,
            "live session identifier space is exhausted",
        )
    })?;
    Ok(SessionId::new(value))
}

fn parse_session_id(value: &str) -> Result<SessionId> {
    if value.len() != U128_HEX_ID_BYTES || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(error(
            ErrorCode::InvalidRequest,
            "session_id must be the 32-character hexadecimal identifier returned by fortress_open_session",
        ));
    }
    let parsed = u128::from_str_radix(value, 16).map_err(|_| {
        error(
            ErrorCode::InvalidRequest,
            "session_id is not a valid hexadecimal u128 identifier",
        )
    })?;
    if parsed == 0 {
        return Err(error(
            ErrorCode::InvalidRequest,
            "session_id zero is reserved",
        ));
    }
    Ok(SessionId::new(parsed))
}

fn resolve_session(value: Option<String>) -> Result<Arc<Mutex<LiveSession>>> {
    let raw = value.ok_or_else(|| {
        error(
            ErrorCode::InvalidRequest,
            "session_id is required; call fortress_open_session first",
        )
    })?;
    let session_id = parse_session_id(&raw)?;
    sessions()?.get(&session_id).cloned().ok_or_else(|| {
        error(
            ErrorCode::SessionNotFound,
            "no live session has the supplied session_id; call fortress_open_session",
        )
    })
}

fn lock_session(session: &Arc<Mutex<LiveSession>>) -> Result<MutexGuard<'_, LiveSession>> {
    session.lock().map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "live session mutex is poisoned; the session cannot be used safely",
        )
    })
}

fn parse_requested_capabilities(
    requested: Option<Vec<(String, String)>>,
) -> Result<Vec<Capability>> {
    let raw = match requested {
        Some(value) => value,
        None => DEFAULT_CAPABILITIES
            .iter()
            .map(|(capability, risk)| ((*capability).to_owned(), (*risk).to_owned()))
            .collect(),
    };
    if raw.len() > MAX_CAPABILITY_REQUESTS {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "requested capability count exceeds the live-session bound",
        ));
    }
    let mut capabilities = Vec::with_capacity(raw.len());
    let mut seen = BTreeSet::new();
    for (capability, risk) in raw {
        if capability.is_empty()
            || capability.len() > MAX_CAPABILITY_NAME_BYTES
            || risk.is_empty()
            || risk.len() > MAX_RISK_NAME_BYTES
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "capability or risk name violates its byte bound",
            ));
        }
        if risk != "read_only" {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "the live bridge V1 grants only read_only risk",
            ));
        }
        let parsed = match capability.as_str() {
            "observe" => Capability::Observe,
            "query" => Capability::Query,
            "doctor" => Capability::Doctor,
            _ => {
                return Err(error(
                    ErrorCode::CapabilityDenied,
                    format!(
                        "capability {capability:?} is unavailable in the live read-only server"
                    ),
                ));
            }
        };
        if !seen.insert(parsed) {
            return Err(error(
                ErrorCode::InvalidRequest,
                format!("capability {capability:?} was requested more than once"),
            ));
        }
        capabilities.push(parsed);
    }
    Ok(capabilities)
}

fn parse_optional_selector(value: Option<String>) -> Result<Option<FortressId>> {
    let Some(raw) = value else {
        return Ok(None);
    };
    if raw.is_empty() || raw.len() > MAX_FORTRESS_SELECTOR_BYTES {
        return Err(error(
            ErrorCode::InvalidRequest,
            "fortress_selector must be a bounded decimal u64",
        ));
    }
    let parsed = raw.parse::<u64>().map_err(|_| {
        error(
            ErrorCode::InvalidRequest,
            "fortress_selector must be a decimal u64",
        )
    })?;
    if parsed == 0 {
        return Err(error(
            ErrorCode::InvalidRequest,
            "fortress_selector zero is reserved",
        ));
    }
    Ok(Some(FortressId::new(parsed)))
}

fn requested_budget(
    max_wall_millis: Option<u64>,
    max_game_ticks: Option<u64>,
    max_entities: Option<u32>,
    max_bytes: Option<u64>,
    max_output_tokens: Option<u32>,
    max_actions: Option<u32>,
) -> Result<WorkBudget> {
    let budget = WorkBudget {
        max_wall_millis: max_wall_millis
            .map_or(WorkBudget::CONSERVATIVE_DEFAULT.max_wall_millis, |value| value),
        max_game_ticks: max_game_ticks
            .map_or(WorkBudget::CONSERVATIVE_DEFAULT.max_game_ticks, |value| value),
        max_entities: max_entities
            .map_or(WorkBudget::CONSERVATIVE_DEFAULT.max_entities, |value| value),
        max_bytes: max_bytes.map_or(WorkBudget::CONSERVATIVE_DEFAULT.max_bytes, |value| value),
        max_output_tokens: max_output_tokens.map_or(
            WorkBudget::CONSERVATIVE_DEFAULT.max_output_tokens,
            |value| value,
        ),
        max_actions: max_actions
            .map_or(WorkBudget::CONSERVATIVE_DEFAULT.max_actions, |value| value),
    };
    budget.validate()?;
    if budget.max_wall_millis > LIVE_BUDGET_CEILING.max_wall_millis
        || budget.max_game_ticks > LIVE_BUDGET_CEILING.max_game_ticks
        || budget.max_entities > LIVE_BUDGET_CEILING.max_entities
        || budget.max_bytes > LIVE_BUDGET_CEILING.max_bytes
        || budget.max_output_tokens > LIVE_BUDGET_CEILING.max_output_tokens
        || budget.max_actions > LIVE_BUDGET_CEILING.max_actions
    {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "requested budget exceeds the live read-only server ceiling",
        ));
    }
    Ok(budget)
}

fn env_u64(name: &str, default: u64, minimum: u64, maximum: u64) -> Result<u64> {
    let value = match env::var(name) {
        Ok(raw) => raw.parse::<u64>().map_err(|_| {
            error(
                ErrorCode::InvalidRequest,
                format!("{name} must be a decimal u64"),
            )
        })?,
        Err(env::VarError::NotPresent) => default,
        Err(env::VarError::NotUnicode(_)) => {
            return Err(error(
                ErrorCode::InvalidRequest,
                format!("{name} must be valid UTF-8"),
            ));
        }
    };
    if value < minimum || value > maximum {
        return Err(error(
            ErrorCode::InvalidRequest,
            format!("{name} must be in {minimum}..={maximum}, got {value}"),
        ));
    }
    Ok(value)
}

fn env_u32(name: &str, default: u32, minimum: u32, maximum: u32) -> Result<u32> {
    let value = env_u64(
        name,
        u64::from(default),
        u64::from(minimum),
        u64::from(maximum),
    )?;
    u32::try_from(value).map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            format!("validated {name} does not fit u32"),
        )
    })
}

fn env_bool(name: &str, default: bool) -> Result<bool> {
    match env::var(name) {
        Ok(raw) => match raw.as_str() {
            "1" | "true" | "TRUE" | "yes" | "YES" => Ok(true),
            "0" | "false" | "FALSE" | "no" | "NO" => Ok(false),
            _ => Err(error(
                ErrorCode::InvalidRequest,
                format!("{name} must be true/false or 1/0"),
            )),
        },
        Err(env::VarError::NotPresent) => Ok(default),
        Err(env::VarError::NotUnicode(_)) => Err(error(
            ErrorCode::InvalidRequest,
            format!("{name} must be valid UTF-8"),
        )),
    }
}

fn bridge_nonce(session_id: SessionId, endpoint: &str) -> Result<Vec<u8>> {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "system clock precedes the Unix epoch",
        )
    })?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"dfmcp-live-mcp-nonce-v1\0");
    bytes.extend_from_slice(&session_id.get().to_be_bytes());
    bytes.extend_from_slice(&elapsed.as_nanos().to_be_bytes());
    bytes.extend_from_slice(&std::process::id().to_be_bytes());
    bytes.extend_from_slice(endpoint.as_bytes());
    bytes.extend_from_slice(env!("CARGO_PKG_VERSION").as_bytes());
    Ok(Digest32::of_bytes(&bytes).as_bytes().to_vec())
}

fn bridge_endpoint() -> Result<String> {
    match env::var("DFMCP_BRIDGE_ENDPOINT") {
        Ok(value) => Ok(value),
        Err(env::VarError::NotPresent) => Ok("127.0.0.1:5000".to_owned()),
        Err(env::VarError::NotUnicode(_)) => Err(error(
            ErrorCode::InvalidRequest,
            "DFMCP_BRIDGE_ENDPOINT must be valid UTF-8",
        )),
    }
}

fn bridge_token() -> Result<Vec<u8>> {
    match env::var("DFMCP_BRIDGE_TOKEN") {
        Ok(value) => Ok(value.into_bytes()),
        Err(env::VarError::NotPresent) => Err(error(
            ErrorCode::CapabilityDenied,
            "DFMCP_BRIDGE_TOKEN is required in the MCP server environment",
        )),
        Err(env::VarError::NotUnicode(_)) => Err(error(
            ErrorCode::InvalidRequest,
            "DFMCP_BRIDGE_TOKEN must be valid UTF-8",
        )),
    }
}

fn live_source(session_id: SessionId) -> Result<AuthenticatedLiveSource> {
    let endpoint = bridge_endpoint()?;
    let address = parse_loopback_endpoint(&endpoint)?;
    let credentials = BridgeCredentials::new(
        bridge_token()?,
        bridge_nonce(session_id, &endpoint)?,
    )?;
    let connect_millis = env_u64("DFMCP_BRIDGE_CONNECT_MILLIS", 2_000, 1, 60_000)?;
    let read_millis = env_u64("DFMCP_BRIDGE_READ_MILLIS", 5_000, 1, 60_000)?;
    let write_millis = env_u64("DFMCP_BRIDGE_WRITE_MILLIS", 5_000, 1, 60_000)?;
    connect_authenticated_live_source(
        &LiveConnectionConfig {
            endpoint: address,
            connect_timeout: Duration::from_millis(connect_millis),
            read_timeout: Duration::from_millis(read_millis),
            write_timeout: Duration::from_millis(write_millis),
            client_name: "dwarf-fortress-mcp-live".to_owned(),
            client_version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        credentials,
    )
}

fn capability_grants(
    fortress_id: FortressId,
    capabilities: &[Capability],
) -> Vec<CapabilityGrant> {
    capabilities
        .iter()
        .copied()
        .map(|capability| CapabilityGrant {
            capability,
            scope: CapabilityScope {
                fortress_id: Some(fortress_id),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect()
}

fn anchor_json(anchor: StateAnchor) -> JsonValue {
    json!({
        "fortress_id": anchor.fortress_id.to_string(),
        "epoch": anchor.cursor.epoch,
        "sequence": anchor.cursor.sequence,
        "game_tick": anchor.tick.get(),
        "state_hash": anchor.state_hash.to_string(),
    })
}

fn budget_json(budget: WorkBudget) -> JsonValue {
    json!({
        "requested": {
            "max_wall_millis": budget.max_wall_millis,
            "max_game_ticks": budget.max_game_ticks,
            "max_entities": budget.max_entities,
            "max_bytes": budget.max_bytes,
            "max_output_tokens": budget.max_output_tokens,
            "max_actions": budget.max_actions,
        },
        "admitted": {
            "max_wall_millis": budget.max_wall_millis,
            "max_game_ticks": budget.max_game_ticks,
            "max_entities": budget.max_entities,
            "max_bytes": budget.max_bytes,
            "max_output_tokens": budget.max_output_tokens,
            "max_actions": budget.max_actions,
        },
        "consumed": {},
        "remaining": null,
        "soft_stop_reason": null,
        "hard_stop_reason": null,
    })
}

fn coverage_json(session: &LiveSession) -> JsonValue {
    let Some(projection) = session.adapter.current_projection() else {
        return json!({
            "status": "unknown",
            "complete_domains": [],
            "partial_domains": [],
            "omitted_domains": [],
            "unknown_domains": [],
            "continuation": null,
        });
    };
    let coverage = projection.receipt.coverage();
    let mut complete = Vec::new();
    let mut partial = Vec::new();
    let mut omitted = Vec::new();
    let mut unknown = Vec::new();
    for domain in coverage.domains.values() {
        let item = json!({
            "domain": domain.domain,
            "reason": domain.reason,
        });
        match domain.status.as_str() {
            "complete" => complete.push(item),
            "partial" => partial.push(item),
            "omitted" => omitted.push(item),
            _ => unknown.push(item),
        }
    }
    json!({
        "status": "complete_for_named_projection",
        "anchor": coverage.anchor.map(anchor_json),
        "complete_domains": complete,
        "partial_domains": partial,
        "omitted_domains": omitted,
        "unknown_domains": unknown,
        "continuation": coverage.continuation,
    })
}

fn briefing_json(session: &LiveSession) -> JsonValue {
    let capsule = session.adapter.last_capsule();
    let identity = session.adapter.identity();
    json!({
        "implementation_phase": LIVE_IMPLEMENTATION_PHASE,
        "adapter": identity.name,
        "compatibility": format!("{:?}", identity.compatibility),
        "live": true,
        "read_only": true,
        "fortress_loaded": capsule.is_some(),
        "world_name": capsule.map(|value| value.world_name.clone()),
        "world_folder": capsule.map(|value| value.world_folder.clone()),
        "site_id": capsule.map(|value| value.site_id),
        "paused": capsule.map(|value| value.paused),
        "calendar_year": capsule.map(|value| value.current_year),
        "year_tick": capsule.map(|value| value.current_year_tick),
        "citizen_count": capsule.map(|value| value.citizen_coverage.total),
        "citizen_names_observed": capsule.map(|value| value.names_included),
        "mutation_admissible": false,
        "source_poisoned": session.source_poisoned(),
        "highest_unresolved_uncertainty": "bridge protocol V1 omits items, jobs, map, economy, welfare, military, and history",
    })
}

fn affordances_json(session: &LiveSession) -> Vec<JsonValue> {
    [
        (Capability::Observe, "observe-live", "fortress.observe", "observe"),
        (Capability::Query, "query-live", "fortress.query", "query"),
        (Capability::Query, "explain-live", "fortress.explain", "explain"),
        (Capability::Observe, "wait-live", "fortress.wait", "wait"),
        (Capability::Doctor, "doctor-live", "fortress.doctor", "doctor"),
    ]
    .into_iter()
    .map(|(capability, id, tool, family)| {
        let enabled = session.has_grant(capability) && !session.source_poisoned();
        json!({
            "affordance_id": id,
            "tool": tool,
            "intent_family": family,
            "risk": "read_only",
            "reversibility": "not_applicable",
            "enabled": enabled,
            "disabled_reason": if enabled {
                JsonValue::Null
            } else if session.source_poisoned() {
                json!("the live source is poisoned; open a fresh session")
            } else {
                json!(format!("{} capability is not granted", capability.as_str()))
            },
            "known_preconditions": [],
            "unverified_preconditions": [],
            "checkpoint_policy": "not_applicable",
            "confirmation_policy": "not_applicable",
            "estimated_cost": {
                "actions": 0,
                "bridge_bytes": null,
                "wall_millis": null,
                "game_ticks": 0,
            },
            "arguments": {},
        })
    })
    .collect()
}

fn uncertainties_json(session: &LiveSession) -> Vec<JsonValue> {
    let mut values = OMITTED_LIVE_DOMAINS
        .iter()
        .map(|(domain, reason)| {
            uncertainty(
                format!("omitted-{domain}"),
                "unknown",
                format!("{domain} is not observed"),
                *reason,
                None,
                json!({}),
            )
        })
        .collect::<Vec<_>>();
    values.push(uncertainty(
        "live-mutation-unavailable",
        "unknown",
        "bridge protocol V1 has no mutation methods",
        "this session cannot change Dwarf Fortress state",
        None,
        json!({}),
    ));
    if session.source_poisoned() {
        let reason = match session.source_poison_reason() {
            Some(value) => value,
            None => "the exact failure reason is unavailable",
        };
        values.push(uncertainty(
            "live-source-poisoned",
            "stale",
            "the negotiated live source failed and is permanently fenced",
            reason,
            Some("fortress.open_session"),
            json!({}),
        ));
    }
    values
}

fn references_json(session: &LiveSession) -> Vec<JsonValue> {
    let id = session.session_id.to_string();
    let mut values = vec![
        json!({"kind": "resource", "uri": format!("df://session/{id}/summary")}),
        json!({"kind": "resource", "uri": format!("df://session/{id}/capabilities")}),
    ];
    if let Ok(anchor) = session.current_anchor() {
        values.push(json!({
            "kind": "resource",
            "uri": format!("df://fortress/{}/anchor", anchor.fortress_id),
        }));
    }
    values
}

#[allow(clippy::too_many_arguments)]
fn attach_turn(
    session: &LiveSession,
    operation: &str,
    phase: AgentPhase,
    profile: ObservationProfile,
    request_id: RequestId,
    continuity: ContinuityStatus,
    basis: Option<StateAnchor>,
    reset_reason: Option<String>,
    changes: Vec<JsonValue>,
    attention: Vec<JsonValue>,
    recommendations: Vec<JsonValue>,
    payload: JsonValue,
) -> String {
    let mut builder = AgentTurnBuilder::new(operation, phase)
        .session_id(session.session_id.to_string())
        .turn_id(format!("live-turn-{request_id}"))
        .request_id(request_id.to_string())
        .continuity(continuity, basis.map(anchor_json), None, reset_reason)
        .profile(profile)
        .briefing(briefing_json(session))
        .changes(changes)
        .attention(attention)
        .active_work(empty_active_work())
        .affordances(affordances_json(session))
        .recommendations(recommendations)
        .uncertainty(uncertainties_json(session))
        .coverage(coverage_json(session))
        .budget(budget_json(session.budget))
        .references(references_json(session));
    if let Ok(anchor) = session.current_anchor() {
        builder = builder.anchor(anchor_json(anchor));
    }
    builder.attach(payload)
}

fn recovery_class(code: ErrorCode) -> RecoveryClass {
    match code {
        ErrorCode::CursorGap | ErrorCode::StaleAnchor | ErrorCode::PreconditionsFailed => {
            RecoveryClass::RefreshAndRetry
        }
        ErrorCode::AdapterUnavailable | ErrorCode::AdapterFailure => RecoveryClass::Backoff,
        ErrorCode::EffectIndeterminate => RecoveryClass::ReconciliationRequired,
        ErrorCode::VersionMismatch | ErrorCode::CompatibilityUnknown => {
            RecoveryClass::OperatorActionRequired
        }
        ErrorCode::CapabilityDenied
        | ErrorCode::AdapterRejected
        | ErrorCode::InvalidRequest
        | ErrorCode::BudgetExceeded => RecoveryClass::NeverUnchanged,
        _ => RecoveryClass::OperatorActionRequired,
    }
}

fn error_payload(operation: &str, failure: &DfmcpError) -> JsonValue {
    let class = recovery_class(failure.code);
    let next = match class {
        RecoveryClass::RefreshAndRetry => Some("fortress.observe"),
        RecoveryClass::Backoff => Some("fortress.open_session"),
        RecoveryClass::ReconciliationRequired => Some("fortress.wait"),
        _ => None,
    };
    json!({
        "ok": false,
        "error": {
            "operation": operation,
            "code": failure.code.as_str(),
            "message": failure.message,
            "retryable": failure.retryable,
            "details": failure.details,
            "recovery": recovery_guidance(
                class,
                next,
                "follow the minimum safe protocol step before retrying",
                json!({}),
            ),
        },
    })
}

fn unbound_error(operation: &str, phase: AgentPhase, failure: &DfmcpError) -> String {
    AgentTurnBuilder::new(operation, phase)
        .profile(ObservationProfile::Briefing)
        .continuity(ContinuityStatus::Bootstrap, None, None, None)
        .briefing(json!({
            "implementation_phase": LIVE_IMPLEMENTATION_PHASE,
            "live": true,
            "read_only": true,
            "fortress_loaded": false,
            "mutation_admissible": false,
        }))
        .active_work(empty_active_work())
        .recommendations(if operation == "fortress.open_session" {
            Vec::new()
        } else {
            vec![recommendation(
                "open-live-session",
                "fortress.open_session",
                "establish an authenticated canonical live anchor",
                "high",
                "high",
                "read_only",
                "not_applicable",
                false,
                json!({}),
            )]
        })
        .uncertainty(vec![uncertainty(
            "live-session-unavailable",
            "unknown",
            "no authenticated live session is available",
            "no live fortress fact is established by this response",
            Some("fortress.open_session"),
            json!({}),
        )])
        .attach(error_payload(operation, failure))
}

fn session_error(
    session: &LiveSession,
    operation: &str,
    phase: AgentPhase,
    profile: ObservationProfile,
    request_id: RequestId,
    basis: StateAnchor,
    continuity: ContinuityStatus,
    failure: &DfmcpError,
) -> String {
    let recommendations = if session.source_poisoned() {
        vec![recommendation(
            "replace-poisoned-live-session",
            "fortress.open_session",
            "the existing DFHack stream is permanently fenced after failure",
            "high",
            "high",
            "read_only",
            "not_applicable",
            false,
            json!({}),
        )]
    } else {
        Vec::new()
    };
    attach_turn(
        session,
        operation,
        phase,
        profile,
        request_id,
        continuity,
        Some(basis),
        None,
        Vec::new(),
        vec![json!({
            "attention_id": "live-protocol-error",
            "category": "control_plane",
            "severity": "high",
            "urgency": "now",
            "confidence": {"epistemic_state": "observed", "value": 1.0},
            "finding": failure.message,
            "likely_consequence_if_ignored": if continuity == ContinuityStatus::Continuous {
                "the request failed, but the prior canonical anchor remains valid"
            } else {
                "the agent may act from stale or incomplete live state"
            },
            "evidence": [],
        })],
        recommendations,
        error_payload(operation, failure),
    )
}

fn hex_bytes(value: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(value.len().saturating_mul(2));
    for byte in value {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn world_value_json(value: &WorldValue) -> JsonValue {
    match value {
        WorldValue::Null => JsonValue::Null,
        WorldValue::Bool(value) => json!(value),
        WorldValue::I64(value) => json!(value),
        WorldValue::U64(value) => json!(value),
        WorldValue::Fixed { units, scale } => json!({"units": units, "scale": scale}),
        WorldValue::Text(value) => json!(value),
        WorldValue::Entity(value) => json!({"entity_id": value.to_string()}),
        WorldValue::Coord(value) => json!({"x": value.x, "y": value.y, "z": value.z}),
        WorldValue::Bytes(value) => json!({
            "encoding": "hex",
            "hex": hex_bytes(value),
            "byte_length": value.len(),
        }),
        WorldValue::List(values) => {
            JsonValue::Array(values.iter().map(world_value_json).collect())
        }
        WorldValue::Object(values) => {
            let mut object = JsonMap::new();
            for (key, value) in values {
                object.insert(key.clone(), world_value_json(value));
            }
            JsonValue::Object(object)
        }
    }
}

fn fact_json(fact: &Fact) -> JsonValue {
    let (presence, epistemic_state, reason, stale_anchor) = match fact.presence.as_ref() {
        None | Some(FactPresence::Known(_)) => ("known", "observed", None, None),
        Some(FactPresence::Absent) => ("absent", "observed", None, None),
        Some(FactPresence::Unknown(value)) => {
            ("unknown", "unknown", Some(value.clone()), None)
        }
        Some(FactPresence::Unsupported(value)) => {
            ("unsupported", "unknown", Some(value.clone()), None)
        }
        Some(FactPresence::Omitted(value)) => {
            ("omitted", "unknown", Some(value.clone()), None)
        }
        Some(FactPresence::Redacted(value)) => {
            ("redacted", "unknown", Some(value.clone()), None)
        }
        Some(FactPresence::Stale(anchor)) => {
            ("stale", "stale", None, Some(anchor_json(*anchor)))
        }
    };
    json!({
        "value": world_value_json(&fact.value),
        "presence": presence,
        "reason": reason,
        "stale_anchor": stale_anchor,
        "observed_at_game_tick": fact.observed_at.get(),
        "source": format!("{:?}", fact.source),
        "source_digest": fact.source_digest.to_string(),
        "epistemic_state": epistemic_state,
    })
}

fn parse_entity_id(value: &str) -> Result<EntityId> {
    if value.is_empty() || value.len() > MAX_ENTITY_ID_BYTES {
        return Err(error(
            ErrorCode::InvalidRequest,
            "entity_id must be a bounded decimal u64",
        ));
    }
    let parsed = value.parse::<u64>().map_err(|_| {
        error(
            ErrorCode::InvalidRequest,
            "entity_id must be a decimal u64",
        )
    })?;
    if parsed == 0 {
        return Err(error(
            ErrorCode::InvalidRequest,
            "entity_id zero is reserved",
        ));
    }
    Ok(EntityId::new(parsed))
}

fn validate_bootstrap_budget(adapter: &LiveMcpAdapter, budget: WorkBudget) -> Result<()> {
    let capsule = adapter.last_capsule().ok_or_else(|| {
        error(
            ErrorCode::InternalInvariantViolation,
            "live bootstrap returned no source capsule",
        )
    })?;
    let canonical_bytes = u64::try_from(capsule.canonical_bytes.len()).map_err(|_| {
        error(
            ErrorCode::BudgetExceeded,
            "live capsule size cannot be represented in the negotiated budget",
        )
    })?;
    if canonical_bytes > budget.max_bytes {
        return Err(error(
            ErrorCode::BudgetExceeded,
            format!(
                "live capsule requires {canonical_bytes} bytes, exceeding the negotiated {}-byte ceiling",
                budget.max_bytes
            ),
        ));
    }
    let projection = adapter.current_projection().ok_or_else(|| {
        error(
            ErrorCode::InternalInvariantViolation,
            "live bootstrap returned no canonical projection",
        )
    })?;
    let entities = u32::try_from(projection.snapshot.graph.entities.len()).map_err(|_| {
        error(
            ErrorCode::BudgetExceeded,
            "live projection entity count cannot be represented in u32",
        )
    })?;
    if entities > budget.max_entities {
        return Err(error(
            ErrorCode::BudgetExceeded,
            format!(
                "live projection contains {entities} entities, exceeding the negotiated {}-entity ceiling",
                budget.max_entities
            ),
        ));
    }
    Ok(())
}

#[tool(
    description = "Open an authenticated read-only live Dwarf Fortress session. Endpoint and bearer token are process configuration, never MCP arguments. Only observe, query, and doctor capabilities are admitted."
)]
#[allow(clippy::too_many_arguments)]
pub fn fortress_open_session(
    paused: Option<bool>,
    fortress_selector: Option<String>,
    requested_capabilities: Option<Vec<(String, String)>>,
    max_wall_millis: Option<u64>,
    max_game_ticks: Option<u64>,
    max_entities: Option<u32>,
    max_bytes: Option<u64>,
    max_output_tokens: Option<u32>,
    max_actions: Option<u32>,
) -> String {
    let operation = "fortress.open_session";
    let selector = match parse_optional_selector(fortress_selector) {
        Ok(value) => value,
        Err(failure) => return unbound_error(operation, AgentPhase::Bootstrap, &failure),
    };
    let capabilities = match parse_requested_capabilities(requested_capabilities) {
        Ok(value) => value,
        Err(failure) => return unbound_error(operation, AgentPhase::Bootstrap, &failure),
    };
    let budget = match requested_budget(
        max_wall_millis,
        max_game_ticks,
        max_entities,
        max_bytes,
        max_output_tokens,
        max_actions,
    ) {
        Ok(value) => value,
        Err(failure) => return unbound_error(operation, AgentPhase::Bootstrap, &failure),
    };
    if sessions().is_ok_and(|registry| registry.len() >= MAX_LIVE_MCP_SESSIONS) {
        return unbound_error(
            operation,
            AgentPhase::Bootstrap,
            &error(
                ErrorCode::BudgetExceeded,
                "live MCP server reached its explicit session bound",
            ),
        );
    }
    let session_id = match next_session_id() {
        Ok(value) => value,
        Err(failure) => return unbound_error(operation, AgentPhase::Bootstrap, &failure),
    };
    let source = match live_source(session_id) {
        Ok(value) => value,
        Err(failure) => return unbound_error(operation, AgentPhase::Bootstrap, &failure),
    };
    let hard_citizens = match u32::try_from(dfmcp_adapter::MAX_CAPSULE_CITIZENS) {
        Ok(value) => value,
        Err(_) => {
            return unbound_error(
                operation,
                AgentPhase::Bootstrap,
                &error(
                    ErrorCode::InternalInvariantViolation,
                    "capsule citizen ceiling does not fit u32",
                ),
            );
        }
    };
    let environment_max = match env_u32(
        "DFMCP_BRIDGE_MAX_CITIZENS",
        dfmcp_adapter::DEFAULT_MAX_LIVE_CITIZENS,
        0,
        hard_citizens,
    ) {
        Ok(value) => value,
        Err(failure) => return unbound_error(operation, AgentPhase::Bootstrap, &failure),
    };
    let max_citizens = environment_max.min(budget.max_entities.saturating_sub(1));
    let page_size = match env_u32(
        "DFMCP_BRIDGE_PAGE_SIZE",
        dfmcp_adapter::MAX_CITIZENS_PER_PAGE,
        1,
        dfmcp_adapter::MAX_CITIZENS_PER_PAGE,
    ) {
        Ok(value) => value,
        Err(failure) => return unbound_error(operation, AgentPhase::Bootstrap, &failure),
    };
    let include_names = match env_bool("DFMCP_BRIDGE_INCLUDE_NAMES", true) {
        Ok(value) => value,
        Err(failure) => return unbound_error(operation, AgentPhase::Bootstrap, &failure),
    };
    let adapter = match bootstrap_live_read_adapter(
        source,
        LiveReadBootstrapConfig {
            page_size,
            max_citizens,
            include_names,
            initial_epoch: 0,
        },
    ) {
        Ok(value) => value,
        Err(failure) => return unbound_error(operation, AgentPhase::Bootstrap, &failure),
    };
    if let Err(failure) = validate_bootstrap_budget(&adapter, budget) {
        return unbound_error(operation, AgentPhase::Bootstrap, &failure);
    }
    let anchor = match adapter.current_anchor() {
        Some(value) => value,
        None => {
            return unbound_error(
                operation,
                AgentPhase::Bootstrap,
                &error(
                    ErrorCode::InternalInvariantViolation,
                    "live bootstrap returned no canonical anchor",
                ),
            );
        }
    };
    if selector.is_some_and(|expected| expected != anchor.fortress_id) {
        return unbound_error(
            operation,
            AgentPhase::Bootstrap,
            &error(
                ErrorCode::StaleAnchor,
                "fortress_selector does not match the authenticated live fortress identity",
            ),
        );
    }
    if let Some(expected) = paused {
        let matches = adapter
            .last_capsule()
            .is_some_and(|capsule| capsule.paused == expected);
        if !matches {
            return unbound_error(
                operation,
                AgentPhase::Bootstrap,
                &error(
                    ErrorCode::PreconditionsFailed,
                    "live fortress pause state does not match the requested assertion",
                ),
            );
        }
    }
    let grants = capability_grants(anchor.fortress_id, &capabilities);
    let session = Arc::new(Mutex::new(LiveSession {
        session_id,
        adapter,
        grants,
        budget,
        next_request_id: 1,
    }));
    {
        let mut registry = match sessions() {
            Ok(value) => value,
            Err(failure) => return unbound_error(operation, AgentPhase::Bootstrap, &failure),
        };
        if registry.len() >= MAX_LIVE_MCP_SESSIONS {
            return unbound_error(
                operation,
                AgentPhase::Bootstrap,
                &error(
                    ErrorCode::BudgetExceeded,
                    "live MCP server reached its explicit session bound",
                ),
            );
        }
        if registry.insert(session_id, Arc::clone(&session)).is_some() {
            return unbound_error(
                operation,
                AgentPhase::Bootstrap,
                &error(
                    ErrorCode::InternalInvariantViolation,
                    "fresh live session identifier collided with an existing session",
                ),
            );
        }
    }
    let guard = match lock_session(&session) {
        Ok(value) => value,
        Err(failure) => return unbound_error(operation, AgentPhase::Bootstrap, &failure),
    };
    let capsule = match guard.adapter.last_capsule() {
        Some(value) => value,
        None => {
            return unbound_error(
                operation,
                AgentPhase::Bootstrap,
                &error(
                    ErrorCode::InternalInvariantViolation,
                    "registered live session lost its source capsule",
                ),
            );
        }
    };
    let request_id = RequestId::new(1);
    attach_turn(
        &guard,
        operation,
        AgentPhase::Bootstrap,
        ObservationProfile::Briefing,
        request_id,
        ContinuityStatus::Bootstrap,
        None,
        None,
        vec![json!({
            "kind": "live_session_opened",
            "subject": {"fortress_id": anchor.fortress_id.to_string()},
            "epistemic_state": "observed",
            "invalidates": [],
            "evidence": [capsule.content_digest.to_string()],
        })],
        vec![json!({
            "attention_id": "live-read-only-posture",
            "category": "capability",
            "severity": "medium",
            "urgency": "persistent",
            "confidence": {"epistemic_state": "observed", "value": 1.0},
            "finding": "the authenticated bridge protocol exposes no mutation methods",
            "likely_consequence_if_ignored": "mutation attempts will fail closed",
            "evidence": [],
        })],
        vec![recommendation(
            "observe-live-pulse",
            "fortress.observe",
            "refresh the canonical live anchor and detect meaningful change",
            "high",
            "high",
            "read_only",
            "not_applicable",
            false,
            json!({"session_id": session_id.to_string()}),
        )],
        json!({
            "ok": true,
            "session_id": session_id.to_string(),
            "mode": "authenticated_live_read_only",
            "adapter": guard.adapter.identity().name,
            "compatibility": format!("{:?}", guard.adapter.identity().compatibility),
            "fortress_id": anchor.fortress_id.to_string(),
            "anchor": anchor_json(anchor),
            "paused": capsule.paused,
            "granted_capabilities": capabilities.iter().map(|value| value.as_str()).collect::<Vec<_>>(),
            "budget": budget_json(budget)["admitted"].clone(),
            "bridge": {
                "version": capsule.bridge.bridge_version,
                "generation": capsule.bridge.bridge_generation,
                "dfhack_version": capsule.bridge.dfhack_version,
                "dwarf_fortress_version": capsule.bridge.df_version,
                "supported_methods": capsule.bridge.supported_methods,
                "mutation_methods": [],
            },
        }),
    )
}

#[tool(
    description = "Refresh an authenticated live read-only session. Returns a heartbeat when the complete semantic state is unchanged, or a full canonical snapshot summary when it advanced or reset."
)]
pub fn fortress_observe(session_id: Option<String>) -> String {
    let operation = "fortress.observe";
    let session = match resolve_session(session_id) {
        Ok(value) => value,
        Err(failure) => return unbound_error(operation, AgentPhase::Orient, &failure),
    };
    let mut guard = match lock_session(&session) {
        Ok(value) => value,
        Err(failure) => return unbound_error(operation, AgentPhase::Orient, &failure),
    };
    let prior = match guard.current_anchor() {
        Ok(value) => value,
        Err(failure) => return unbound_error(operation, AgentPhase::Orient, &failure),
    };
    let (request_id, context) = match guard.next_context() {
        Ok(value) => value,
        Err(failure) => {
            return session_error(
                &guard,
                operation,
                AgentPhase::Orient,
                ObservationProfile::Pulse,
                RequestId::NIL,
                prior,
                ContinuityStatus::Continuous,
                &failure,
            );
        }
    };
    let request = ObservationRequest {
        since: Some(prior.cursor),
        projection: Projection::Full,
        interest: InterestSet::default(),
        max_entities: guard.budget.max_entities,
        max_bytes: guard.budget.max_bytes,
        max_output_tokens: guard.budget.max_output_tokens,
        continuation: None,
    };
    let frame = match guard.adapter.observe(&request, &context) {
        Ok(value) => value,
        Err(failure) => {
            return session_error(
                &guard,
                operation,
                AgentPhase::Orient,
                ObservationProfile::Pulse,
                request_id,
                prior,
                ContinuityStatus::Stale,
                &failure,
            );
        }
    };
    let current = match guard.current_anchor() {
        Ok(value) => value,
        Err(failure) => {
            return session_error(
                &guard,
                operation,
                AgentPhase::Orient,
                ObservationProfile::Pulse,
                request_id,
                prior,
                ContinuityStatus::Indeterminate,
                &failure,
            );
        }
    };
    let (kind, continuity, reset_reason, changes) = match &frame.payload {
        ObservationPayload::Heartbeat(_) => (
            "heartbeat",
            ContinuityStatus::Heartbeat,
            None,
            Vec::new(),
        ),
        ObservationPayload::Snapshot(_) if current.cursor.epoch != prior.cursor.epoch => (
            "snapshot",
            ContinuityStatus::Reset,
            Some("bridge_generation_or_game_clock_reset".to_owned()),
            vec![json!({
                "kind": "observation_epoch_reset",
                "subject": {"fortress_id": current.fortress_id.to_string()},
                "epistemic_state": "observed",
                "invalidates": ["all_prior_live_continuations", "all_prior_live_recommendations"],
                "evidence": frame.evidence.iter().map(|value| value.digest.to_string()).collect::<Vec<_>>(),
            })],
        ),
        ObservationPayload::Snapshot(_) => (
            "snapshot",
            ContinuityStatus::Continuous,
            None,
            vec![json!({
                "kind": "live_state_advanced",
                "subject": {"fortress_id": current.fortress_id.to_string()},
                "epistemic_state": "observed",
                "invalidates": ["prior_live_recommendations"],
                "evidence": frame.evidence.iter().map(|value| value.digest.to_string()).collect::<Vec<_>>(),
            })],
        ),
        ObservationPayload::Delta(_) => {
            let failure = error(
                ErrorCode::InternalInvariantViolation,
                "live bridge V1 unexpectedly returned a delta payload",
            );
            return session_error(
                &guard,
                operation,
                AgentPhase::Orient,
                ObservationProfile::Pulse,
                request_id,
                prior,
                ContinuityStatus::Indeterminate,
                &failure,
            );
        }
    };
    let capsule = guard.adapter.last_capsule();
    attach_turn(
        &guard,
        operation,
        AgentPhase::Orient,
        ObservationProfile::Pulse,
        request_id,
        continuity,
        Some(prior),
        reset_reason,
        changes,
        Vec::new(),
        Vec::new(),
        json!({
            "ok": true,
            "session_id": guard.session_id.to_string(),
            "request_id": request_id.to_string(),
            "kind": kind,
            "anchor": anchor_json(current),
            "paused": capsule.map(|value| value.paused),
            "calendar_year": capsule.map(|value| value.current_year),
            "year_tick": capsule.map(|value| value.current_year_tick),
            "citizens": capsule.map(|value| value.citizen_coverage.total),
            "warnings": frame.warnings,
            "evidence": frame.evidence.iter().map(|value| json!({
                "evidence_id": value.id.to_string(),
                "digest": value.digest.to_string(),
                "summary": value.summary,
            })).collect::<Vec<_>>(),
        }),
    )
}

#[tool(
    description = "Query the current canonical live snapshot. mode is summary, citizens, or all. Querying never refreshes state; call fortress_observe first when freshness matters."
)]
pub fn fortress_query(session_id: Option<String>, mode: Option<String>) -> String {
    let operation = "fortress.query";
    let mode = match mode {
        Some(value) => value,
        None => "summary".to_owned(),
    };
    if mode.is_empty() || mode.len() > MAX_MODE_BYTES {
        return unbound_error(
            operation,
            AgentPhase::Inspect,
            &error(
                ErrorCode::InvalidRequest,
                "query mode violates its byte bound",
            ),
        );
    }
    let session = match resolve_session(session_id) {
        Ok(value) => value,
        Err(failure) => return unbound_error(operation, AgentPhase::Inspect, &failure),
    };
    let mut guard = match lock_session(&session) {
        Ok(value) => value,
        Err(failure) => return unbound_error(operation, AgentPhase::Inspect, &failure),
    };
    let anchor = match guard.current_anchor() {
        Ok(value) => value,
        Err(failure) => return unbound_error(operation, AgentPhase::Inspect, &failure),
    };
    let (request_id, context) = match guard.next_context() {
        Ok(value) => value,
        Err(failure) => {
            return session_error(
                &guard,
                operation,
                AgentPhase::Inspect,
                ObservationProfile::Tactical,
                RequestId::NIL,
                anchor,
                ContinuityStatus::Continuous,
                &failure,
            );
        }
    };
    let kinds = match mode.as_str() {
        "summary" => vec![EntityKind::Fortress],
        "citizens" => vec![EntityKind::Unit],
        "all" => Vec::new(),
        _ => {
            return session_error(
                &guard,
                operation,
                AgentPhase::Inspect,
                ObservationProfile::Tactical,
                request_id,
                anchor,
                ContinuityStatus::Continuous,
                &error(
                    ErrorCode::InvalidRequest,
                    "query mode must be summary, citizens, or all",
                ),
            );
        }
    };
    let response = match guard.adapter.query(
        &QueryRequest {
            anchor,
            query: WorldQuery {
                kinds,
                predicate: None,
                order: QueryOrder::EntityIdAscending,
                limit: guard.budget.max_entities,
                continuation: None,
            },
            max_output_tokens: guard.budget.max_output_tokens,
            continuation: None,
        },
        &context,
    ) {
        Ok(value) => value,
        Err(failure) => {
            return session_error(
                &guard,
                operation,
                AgentPhase::Inspect,
                ObservationProfile::Tactical,
                request_id,
                anchor,
                ContinuityStatus::Continuous,
                &failure,
            );
        }
    };
    attach_turn(
        &guard,
        operation,
        AgentPhase::Inspect,
        ObservationProfile::Tactical,
        request_id,
        ContinuityStatus::Continuous,
        Some(anchor),
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        json!({
            "ok": true,
            "session_id": guard.session_id.to_string(),
            "request_id": request_id.to_string(),
            "mode": mode,
            "anchor": anchor_json(response.anchor),
            "matched": response.matched,
            "returned": response.rows.len(),
            "truncated": response.truncated,
            "continuation": response.continuation,
            "rows": response.rows.iter().map(|row| json!({
                "entity_id": row.entity_id.to_string(),
                "revision": row.revision,
                "fields": row.fields,
                "evidence": row.evidence.iter().map(|value| value.digest.to_string()).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "score_ledger": response.score_ledger,
        }),
    )
}

fn read_only_tool_error(
    session_id: Option<String>,
    operation: &str,
    phase: AgentPhase,
) -> String {
    let session = match resolve_session(session_id) {
        Ok(value) => value,
        Err(failure) => return unbound_error(operation, phase, &failure),
    };
    let mut guard = match lock_session(&session) {
        Ok(value) => value,
        Err(failure) => return unbound_error(operation, phase, &failure),
    };
    let anchor = match guard.current_anchor() {
        Ok(value) => value,
        Err(failure) => return unbound_error(operation, phase, &failure),
    };
    let (request_id, _) = match guard.next_context() {
        Ok(value) => value,
        Err(failure) => {
            return session_error(
                &guard,
                operation,
                phase,
                ObservationProfile::Tactical,
                RequestId::NIL,
                anchor,
                ContinuityStatus::Continuous,
                &failure,
            );
        }
    };
    let failure = error(
        ErrorCode::CapabilityDenied,
        format!(
            "{operation} cannot succeed: authenticated bridge protocol V1 is read-only and registers no mutation methods"
        ),
    );
    attach_turn(
        &guard,
        operation,
        phase,
        ObservationProfile::Tactical,
        request_id,
        ContinuityStatus::Continuous,
        Some(anchor),
        None,
        Vec::new(),
        vec![json!({
            "attention_id": "mutation-unavailable",
            "category": "capability",
            "severity": "medium",
            "urgency": "persistent",
            "confidence": {"epistemic_state": "observed", "value": 1.0},
            "finding": "the live bridge has no mutation method set",
            "likely_consequence_if_ignored": "repeated mutation attempts will continue to fail unchanged",
            "evidence": [],
        })],
        Vec::new(),
        error_payload(operation, &failure),
    )
}

#[tool(description = "Unavailable in authenticated live bridge protocol V1; fails closed without preparing an effect.")]
pub fn fortress_plan(
    session_id: Option<String>,
    _summary: Option<String>,
    _paused_target: Option<bool>,
) -> String {
    read_only_tool_error(session_id, "fortress.plan", AgentPhase::Propose)
}

#[tool(description = "Unavailable in authenticated live bridge protocol V1; fails closed without committing an effect.")]
pub fn fortress_commit(session_id: Option<String>, _plan_digest: String) -> String {
    read_only_tool_error(session_id, "fortress.commit", AgentPhase::Commit)
}

#[tool(description = "Report that the authenticated read-only session has no active mutation work, then suggest a live observation pulse when useful.")]
pub fn fortress_wait(session_id: Option<String>) -> String {
    let operation = "fortress.wait";
    let session = match resolve_session(session_id) {
        Ok(value) => value,
        Err(failure) => return unbound_error(operation, AgentPhase::Verify, &failure),
    };
    let mut guard = match lock_session(&session) {
        Ok(value) => value,
        Err(failure) => return unbound_error(operation, AgentPhase::Verify, &failure),
    };
    let anchor = match guard.current_anchor() {
        Ok(value) => value,
        Err(failure) => return unbound_error(operation, AgentPhase::Verify, &failure),
    };
    let (request_id, _) = match guard.next_context() {
        Ok(value) => value,
        Err(failure) => {
            return session_error(
                &guard,
                operation,
                AgentPhase::Verify,
                ObservationProfile::Pulse,
                RequestId::NIL,
                anchor,
                ContinuityStatus::Continuous,
                &failure,
            );
        }
    };
    let recommendations = if guard.has_grant(Capability::Observe) && !guard.source_poisoned() {
        vec![recommendation(
            "observe-after-empty-wait",
            "fortress.observe",
            "no mutation work exists; refresh live state only when new information is valuable",
            "medium",
            "medium",
            "read_only",
            "not_applicable",
            false,
            json!({"session_id": guard.session_id.to_string()}),
        )]
    } else {
        Vec::new()
    };
    attach_turn(
        &guard,
        operation,
        AgentPhase::Verify,
        ObservationProfile::Pulse,
        request_id,
        ContinuityStatus::Continuous,
        Some(anchor),
        None,
        Vec::new(),
        Vec::new(),
        recommendations,
        json!({
            "ok": true,
            "session_id": guard.session_id.to_string(),
            "request_id": request_id.to_string(),
            "active_work": [],
            "terminal": true,
            "summary": "bridge protocol V1 has no mutation work to poll",
            "anchor": anchor_json(anchor),
        }),
    )
}

#[tool(description = "Unavailable in authenticated live bridge protocol V1; fails closed without cancellation effects.")]
pub fn fortress_cancel(session_id: Option<String>, _mode: Option<String>) -> String {
    read_only_tool_error(session_id, "fortress.cancel", AgentPhase::Reconcile)
}

#[tool(description = "Unavailable in authenticated live bridge protocol V1; no game/save checkpoint is created.")]
pub fn fortress_checkpoint(session_id: Option<String>, _label: Option<String>) -> String {
    read_only_tool_error(session_id, "fortress.checkpoint", AgentPhase::Commit)
}

#[tool(description = "Unavailable in authenticated live bridge protocol V1; no restore or epoch mutation is attempted.")]
pub fn fortress_restore(session_id: Option<String>, _checkpoint_id: String) -> String {
    read_only_tool_error(session_id, "fortress.restore", AgentPhase::Reconcile)
}

#[tool(
    description = "Explain the current live adapter, coverage, or one canonical entity with field-level source digests. entity_id is the canonical decimal entity ID."
)]
pub fn fortress_explain(session_id: Option<String>, entity_id: Option<String>) -> String {
    let operation = "fortress.explain";
    let session = match resolve_session(session_id) {
        Ok(value) => value,
        Err(failure) => return unbound_error(operation, AgentPhase::Inspect, &failure),
    };
    let mut guard = match lock_session(&session) {
        Ok(value) => value,
        Err(failure) => return unbound_error(operation, AgentPhase::Inspect, &failure),
    };
    let anchor = match guard.current_anchor() {
        Ok(value) => value,
        Err(failure) => return unbound_error(operation, AgentPhase::Inspect, &failure),
    };
    let (request_id, context) = match guard.next_context() {
        Ok(value) => value,
        Err(failure) => {
            return session_error(
                &guard,
                operation,
                AgentPhase::Inspect,
                ObservationProfile::Forensic,
                RequestId::NIL,
                anchor,
                ContinuityStatus::Continuous,
                &failure,
            );
        }
    };
    if let Err(failure) = context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None) {
        return session_error(
            &guard,
            operation,
            AgentPhase::Inspect,
            ObservationProfile::Forensic,
            request_id,
            anchor,
            ContinuityStatus::Continuous,
            &failure,
        );
    }
    let Some(projection) = guard.adapter.current_projection() else {
        return session_error(
            &guard,
            operation,
            AgentPhase::Inspect,
            ObservationProfile::Forensic,
            request_id,
            anchor,
            ContinuityStatus::Indeterminate,
            &error(
                ErrorCode::InternalInvariantViolation,
                "live session lost its canonical projection",
            ),
        );
    };
    let explanation = match entity_id {
        Some(raw) => {
            let parsed = match parse_entity_id(&raw) {
                Ok(value) => value,
                Err(failure) => {
                    return session_error(
                        &guard,
                        operation,
                        AgentPhase::Inspect,
                        ObservationProfile::Forensic,
                        request_id,
                        anchor,
                        ContinuityStatus::Continuous,
                        &failure,
                    );
                }
            };
            let Some(entity) = projection.snapshot.graph.entities.get(&parsed) else {
                return session_error(
                    &guard,
                    operation,
                    AgentPhase::Inspect,
                    ObservationProfile::Forensic,
                    request_id,
                    anchor,
                    ContinuityStatus::Continuous,
                    &error(
                        ErrorCode::InvalidRequest,
                        "canonical entity ID is not present at the current anchor",
                    ),
                );
            };
            json!({
                "kind": "entity",
                "entity_id": entity.id.to_string(),
                "entity_kind": entity.kind.as_str(),
                "label": entity.label,
                "generation": entity.generation,
                "revision": entity.revision,
                "fields": entity.fields.iter().map(|(key, fact)| (key.clone(), fact_json(fact))).collect::<BTreeMap<_, _>>(),
            })
        }
        None => json!({
            "kind": "live_session",
            "projection_schema": projection.receipt.schema(),
            "source_capsule_digest": projection.receipt.source_capsule_digest().to_string(),
            "bridge_generation": projection.receipt.source_bridge_generation(),
            "anchor": anchor_json(anchor),
            "coverage": coverage_json(&guard),
            "authority": {
                "observe": guard.has_grant(Capability::Observe),
                "query": guard.has_grant(Capability::Query),
                "doctor": guard.has_grant(Capability::Doctor),
                "mutation": false,
            },
        }),
    };
    attach_turn(
        &guard,
        operation,
        AgentPhase::Inspect,
        ObservationProfile::Forensic,
        request_id,
        ContinuityStatus::Continuous,
        Some(anchor),
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        json!({
            "ok": true,
            "session_id": guard.session_id.to_string(),
            "request_id": request_id.to_string(),
            "explanation": explanation,
        }),
    )
}

#[tool(
    description = "Diagnose the authenticated live read-only session, including exact versions, current anchor, coverage, and permanent source-fence state."
)]
pub fn fortress_doctor(session_id: Option<String>) -> String {
    let operation = "fortress.doctor";
    let session = match resolve_session(session_id) {
        Ok(value) => value,
        Err(failure) => return unbound_error(operation, AgentPhase::Inspect, &failure),
    };
    let mut guard = match lock_session(&session) {
        Ok(value) => value,
        Err(failure) => return unbound_error(operation, AgentPhase::Inspect, &failure),
    };
    let anchor = match guard.current_anchor() {
        Ok(value) => value,
        Err(failure) => return unbound_error(operation, AgentPhase::Inspect, &failure),
    };
    let (request_id, context) = match guard.next_context() {
        Ok(value) => value,
        Err(failure) => {
            return session_error(
                &guard,
                operation,
                AgentPhase::Inspect,
                ObservationProfile::Forensic,
                RequestId::NIL,
                anchor,
                ContinuityStatus::Continuous,
                &failure,
            );
        }
    };
    let health = match guard.adapter.health(&context) {
        Ok(value) => value,
        Err(failure) => {
            return session_error(
                &guard,
                operation,
                AgentPhase::Inspect,
                ObservationProfile::Forensic,
                request_id,
                anchor,
                ContinuityStatus::Continuous,
                &failure,
            );
        }
    };
    let poisoned = guard.source_poisoned();
    let attention = if poisoned {
        vec![json!({
            "attention_id": "replace-poisoned-live-session",
            "category": "continuity",
            "severity": "critical",
            "urgency": "now",
            "confidence": {"epistemic_state": "observed", "value": 1.0},
            "finding": "the live source is permanently fenced after a failed call",
            "likely_consequence_if_ignored": "no later observation can safely use this transport",
            "evidence": [],
        })]
    } else {
        Vec::new()
    };
    attach_turn(
        &guard,
        operation,
        AgentPhase::Inspect,
        ObservationProfile::Forensic,
        request_id,
        ContinuityStatus::Continuous,
        Some(anchor),
        None,
        Vec::new(),
        attention,
        if poisoned {
            vec![recommendation(
                "open-replacement-live-session",
                "fortress.open_session",
                "replace the permanently fenced DFHack transport",
                "high",
                "high",
                "read_only",
                "not_applicable",
                false,
                json!({}),
            )]
        } else {
            Vec::new()
        },
        json!({
            "ok": true,
            "session_id": guard.session_id.to_string(),
            "request_id": request_id.to_string(),
            "status": if poisoned { "degraded" } else { "read_only" },
            "adapter": health.identity.name,
            "compatibility": format!("{:?}", health.identity.compatibility),
            "dwarf_fortress_version": health.identity.dwarf_fortress_version,
            "dfhack_version": health.identity.dfhack_version,
            "fortress_loaded": health.fortress_loaded,
            "paused": health.paused,
            "current_anchor": health.current_anchor.map(anchor_json),
            "warnings": health.warnings,
            "source_poisoned": poisoned,
            "source_poison_reason": guard.source_poison_reason(),
            "coverage": coverage_json(&guard),
        }),
    )
}

/// Run the authenticated, read-only, modern MCP 2026-07-28 server on stdio.
pub fn run_live_stdio() {
    ServerBuilder::new("dwarf-fortress-mcp-live", env!("CARGO_PKG_VERSION"))
        .tool(FortressOpenSession)
        .tool(FortressObserve)
        .tool(FortressQuery)
        .tool(FortressPlan)
        .tool(FortressCommit)
        .tool(FortressWait)
        .tool(FortressCancel)
        .tool(FortressCheckpoint)
        .tool(FortressRestore)
        .tool(FortressExplain)
        .tool(FortressDoctor)
        .request_timeout(60)
        .instructions(
            "Authenticated Dwarf Fortress live read-only control plane. The endpoint and bearer \
             token are process configuration and never tool arguments. Call fortress_open_session \
             first. The server preserves the frozen eleven-tool waist, but bridge protocol V1 \
             registers only Handshake and ReadObservation; all mutation-stage tools fail closed. \
             Every result includes an agent_turn packet with an exact canonical anchor, coverage, \
             omissions, authority, continuity, and the minimum safe next step.",
        )
        .build()
        .run_stdio();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_parser_admits_only_read_only_live_capabilities() -> Result<()> {
        let parsed = parse_requested_capabilities(Some(vec![
            ("observe".to_owned(), "read_only".to_owned()),
            ("query".to_owned(), "read_only".to_owned()),
            ("doctor".to_owned(), "read_only".to_owned()),
        ]))?;
        assert_eq!(
            parsed,
            vec![Capability::Observe, Capability::Query, Capability::Doctor]
        );
        assert!(
            parse_requested_capabilities(Some(vec![(
                "control_clock".to_owned(),
                "reversible".to_owned(),
            )]))
            .is_err()
        );
        assert!(
            parse_requested_capabilities(Some(vec![
                ("observe".to_owned(), "read_only".to_owned()),
                ("observe".to_owned(), "read_only".to_owned()),
            ]))
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn live_budget_is_bounded() {
        assert!(requested_budget(None, None, None, None, None, None).is_ok());
        assert!(
            requested_budget(Some(60_001), None, None, None, None, None).is_err()
        );
        assert!(
            requested_budget(None, None, Some(100_002), None, None, None).is_err()
        );
    }

    #[test]
    fn live_session_ids_round_trip_and_are_namespaced() -> Result<()> {
        let session = SessionId::new((1u128 << 127) + 17);
        assert_eq!(parse_session_id(&session.to_string())?, session);
        assert!(parse_session_id("17").is_err());
        assert!(parse_session_id("00000000000000000000000000000000").is_err());
        Ok(())
    }

    #[test]
    fn unbound_errors_preserve_the_agent_turn_spine() -> Result<()> {
        let encoded = unbound_error(
            "fortress.observe",
            AgentPhase::Orient,
            &error(ErrorCode::SessionNotFound, "missing"),
        );
        let value: JsonValue = serde_json::from_str(&encoded).map_err(|source| {
            error(
                ErrorCode::InternalInvariantViolation,
                format!("test JSON decode failed: {source}"),
            )
        })?;
        assert_eq!(value["ok"], false);
        assert_eq!(value["agent_turn"]["operation"], "fortress.observe");
        assert_eq!(value["agent_turn"]["continuity"]["status"], "bootstrap");
        Ok(())
    }

    #[test]
    fn world_values_have_deterministic_machine_projections() {
        assert_eq!(world_value_json(&WorldValue::Bool(true)), json!(true));
        assert_eq!(
            world_value_json(&WorldValue::Coord(dfmcp_core::MapCoord::new(1, 2, 3))),
            json!({"x": 1, "y": 2, "z": 3})
        );
        assert_eq!(
            world_value_json(&WorldValue::Bytes(vec![0, 1, 254, 255])),
            json!({
                "encoding": "hex",
                "hex": "0001feff",
                "byte_length": 4,
            })
        );
    }

    #[test]
    fn omitted_facts_are_unknown_not_a_fake_epistemic_class() {
        let fact = Fact::with_presence(
            FactPresence::Omitted("not requested".to_owned()),
            GameTick::new(7),
            FactSource::DfhackField("citizen.name".to_owned()),
            Digest32::of_bytes(b"source"),
        );
        let projected = fact_json(&fact);
        assert_eq!(projected["presence"], "omitted");
        assert_eq!(projected["epistemic_state"], "unknown");
        assert_eq!(projected["reason"], "not requested");
    }

    #[test]
    fn session_error_continuity_is_explicit_at_call_sites() {
        assert_eq!(recovery_class(ErrorCode::InvalidRequest), RecoveryClass::NeverUnchanged);
        assert_eq!(recovery_class(ErrorCode::CursorGap), RecoveryClass::RefreshAndRetry);
        assert_eq!(recovery_class(ErrorCode::AdapterFailure), RecoveryClass::Backoff);
    }
}
