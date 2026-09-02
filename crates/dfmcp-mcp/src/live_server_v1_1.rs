//! Agent-oriented development MCP server for protocol-1.1 retained announcements.
//!
//! This module preserves the frozen eleven-tool waist while exercising the
//! isolated protocol-1.1 bridge, transactional citizen-plus-announcement
//! publication, canonical projection, and read-only `GameAdapter`. It is
//! deliberately *not* an admitted runtime. The separately named binary requires
//! explicit operator opt-in and rejects every production-admission environment
//! marker so execution cannot be confused with compatibility or artifact
//! admission.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::agent_turn::{
    AgentPhase, AgentTurnBuilder, ContinuityStatus, ObservationProfile,
    RecoveryClass, empty_active_work, recommendation, recovery_guidance,
    uncertainty,
};
use dfmcp_adapter::{
    AuthenticatedLiveSourceV1_1, BridgeCredentialsV1_1, GameAdapter,
    InterestSet, LiveConnectionConfig, LiveReadAdapterV1_1,
    LiveReadBootstrapConfigV1_1, ObservationPayload, ObservationRequest,
    PrimedLiveSourceV1_1, Projection, QueryRequest,
    bootstrap_live_read_adapter_v1_1, build_live_announcement_briefing,
    connect_authenticated_live_source_v1_1, parse_loopback_endpoint,
    summarize_live_announcement_change, DEFAULT_LIVE_ANNOUNCEMENT_PAGE_SIZE,
    DEFAULT_MAX_LIVE_ANNOUNCEMENTS, MAX_ANNOUNCEMENTS_PER_BATCH,
    MAX_CAPSULE_CITIZENS, MAX_V1_1_CITIZENS_PER_PAGE,
};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, DfmcpError, Digest32,
    EntityId, ErrorCode, FortressId, ObservationCursor, OperationContext,
    RequestId, Result, RiskTier, SessionId, StateAnchor, WorkBudget,
};
use dfmcp_world::{
    EntityKind, Fact, FactPresence, QueryOrder, Value as WorldValue,
    WorldQuery,
};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use serde_json::{Map as JsonMap, Value as JsonValue, json};

const DEVELOPMENT_OPT_IN: &str = "DFMCP_ALLOW_UNADMITTED_LIVE_V1_1";
const LIVE_IMPLEMENTATION_PHASE: &str =
    "bridge_r1_retained_announcements_unadmitted_development";
const MAX_LIVE_MCP_SESSIONS: usize = 32;
const MAX_CAPABILITY_REQUESTS: usize = 8;
const MAX_CAPABILITY_NAME_BYTES: usize = 64;
const MAX_RISK_NAME_BYTES: usize = 32;
const MAX_FORTRESS_SELECTOR_BYTES: usize = 20;
const MAX_MODE_BYTES: usize = 32;
const MAX_ENTITY_ID_BYTES: usize = 20;
const U128_HEX_ID_BYTES: usize = 32;
const SESSION_NAMESPACE_MASK: u128 = 0xffu128 << 120;
const SESSION_NAMESPACE_PREFIX: u128 = 0x11u128 << 120;
const MIN_RESPONSE_BYTES: u64 = 8 * 1024;
const MIN_RESPONSE_TOKENS: u32 = 2 * 1024;
const LIVE_BUDGET_CEILING: WorkBudget = WorkBudget {
    max_wall_millis: 60_000,
    max_game_ticks: 1_000_000,
    max_entities: 100_513,
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
    ("fortress.items", "protocol 1.1 does not observe items"),
    ("fortress.jobs", "protocol 1.1 does not observe jobs"),
    ("fortress.map", "protocol 1.1 does not observe map state"),
    ("fortress.economy", "protocol 1.1 does not observe economy state"),
    (
        "fortress.welfare",
        "protocol 1.1 does not observe detailed welfare state",
    ),
    (
        "fortress.military",
        "protocol 1.1 does not observe military state",
    ),
    (
        "fortress.history",
        "the retained announcement suffix is not complete fortress history",
    ),
];
const FORBIDDEN_ADMISSION_ENVIRONMENT: [&str; 7] = [
    "DFMCP_ADMISSION_TICKET",
    "DFMCP_COMPATIBILITY_ENTRY_ID",
    "DFMCP_COMPATIBILITY_DECISION_DIGEST",
    "DFMCP_COMPATIBILITY_REGISTRY_DIGEST",
    "DFMCP_COMPATIBILITY_FLOOR_",
    "DFMCP_SERVER_RECEIPT_DIGEST",
    "DFMCP_ADMITTED_LAUNCH_DIGEST",
];

type LiveMcpSourceV1_1 =
    PrimedLiveSourceV1_1<AuthenticatedLiveSourceV1_1>;
type LiveMcpAdapterV1_1 = LiveReadAdapterV1_1<LiveMcpSourceV1_1>;

struct LiveSessionV1_1 {
    session_id: SessionId,
    adapter: LiveMcpAdapterV1_1,
    grants: Vec<CapabilityGrant>,
    budget: WorkBudget,
    next_request_id: u128,
}

impl LiveSessionV1_1 {
    fn current_anchor(&self) -> Result<StateAnchor> {
        self.adapter.current_anchor().ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "registered protocol-1.1 session has no canonical anchor",
            )
        })
    }

    fn next_context(&mut self) -> Result<(RequestId, OperationContext)> {
        let next = self.next_request_id.checked_add(1).ok_or_else(|| {
            error(
                ErrorCode::BudgetExceeded,
                "protocol-1.1 session request identifier space is exhausted",
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

static LIVE_SESSIONS_V1_1: LazyLock<
    Mutex<BTreeMap<SessionId, Arc<Mutex<LiveSessionV1_1>>>>,
> = LazyLock::new(|| Mutex::new(BTreeMap::new()));
static NEXT_LIVE_SESSION_ID_V1_1: LazyLock<Mutex<u128>> =
    LazyLock::new(|| Mutex::new(SESSION_NAMESPACE_PREFIX | 1));

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

fn development_opt_in_value(value: Option<&str>) -> Result<()> {
    if value != Some("1") {
        return Err(error(
            ErrorCode::CapabilityDenied,
            format!(
                "{DEVELOPMENT_OPT_IN}=1 is required for the explicitly unadmitted protocol-1.1 development server"
            ),
        ));
    }
    Ok(())
}

fn forbidden_admission_environment_name(name: &str) -> bool {
    FORBIDDEN_ADMISSION_ENVIRONMENT.iter().any(|candidate| {
        name == *candidate
            || (candidate.ends_with('_') && name.starts_with(candidate))
    })
}

fn validate_development_runtime_environment() -> Result<()> {
    let opt_in = match env::var(DEVELOPMENT_OPT_IN) {
        Ok(value) => value,
        Err(env::VarError::NotPresent) => String::new(),
        Err(env::VarError::NotUnicode(_)) => {
            return Err(error(
                ErrorCode::InvalidRequest,
                format!("{DEVELOPMENT_OPT_IN} must be valid UTF-8"),
            ));
        }
    };
    development_opt_in_value((!opt_in.is_empty()).then_some(opt_in.as_str()))?;
    let mut forbidden = env::vars_os()
        .filter_map(|(name, _)| name.into_string().ok())
        .filter(|name| forbidden_admission_environment_name(name))
        .collect::<Vec<_>>();
    forbidden.sort();
    forbidden.dedup();
    if !forbidden.is_empty() {
        return Err(error(
            ErrorCode::CapabilityDenied,
            format!(
                "unadmitted protocol-1.1 development runtime refuses production admission environment: {}",
                forbidden.join(", ")
            ),
        ));
    }
    Ok(())
}

fn sessions() -> Result<MutexGuard<'static, BTreeMap<SessionId, Arc<Mutex<LiveSessionV1_1>>>>> {
    LIVE_SESSIONS_V1_1.lock().map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "protocol-1.1 session registry mutex is poisoned",
        )
    })
}

fn next_session_id() -> Result<SessionId> {
    let mut counter = NEXT_LIVE_SESSION_ID_V1_1.lock().map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "protocol-1.1 session identifier mutex is poisoned",
        )
    })?;
    let value = *counter;
    if value & SESSION_NAMESPACE_MASK != SESSION_NAMESPACE_PREFIX {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "protocol-1.1 session identifier namespace is exhausted",
        ));
    }
    *counter = value.checked_add(1).ok_or_else(|| {
        error(
            ErrorCode::BudgetExceeded,
            "protocol-1.1 session identifier space is exhausted",
        )
    })?;
    Ok(SessionId::new(value))
}

fn parse_session_id(value: &str) -> Result<SessionId> {
    if value.len() != U128_HEX_ID_BYTES
        || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "session_id must be the 32-character hexadecimal identifier returned by fortress.open_session",
        ));
    }
    let parsed = u128::from_str_radix(value, 16).map_err(|_| {
        error(
            ErrorCode::InvalidRequest,
            "session_id is not a valid hexadecimal u128 identifier",
        )
    })?;
    if parsed & SESSION_NAMESPACE_MASK != SESSION_NAMESPACE_PREFIX {
        return Err(error(
            ErrorCode::InvalidRequest,
            "session_id belongs to another runtime generation; open a protocol-1.1 session",
        ));
    }
    Ok(SessionId::new(parsed))
}

fn resolve_session(value: Option<String>) -> Result<Arc<Mutex<LiveSessionV1_1>>> {
    let raw = value.ok_or_else(|| {
        error(
            ErrorCode::InvalidRequest,
            "session_id is required; call fortress.open_session first",
        )
    })?;
    let session_id = parse_session_id(&raw)?;
    sessions()?.get(&session_id).cloned().ok_or_else(|| {
        error(
            ErrorCode::SessionNotFound,
            "no protocol-1.1 session has the supplied session_id; call fortress.open_session",
        )
    })
}

fn lock_session(
    session: &Arc<Mutex<LiveSessionV1_1>>,
) -> Result<MutexGuard<'_, LiveSessionV1_1>> {
    session.lock().map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "protocol-1.1 session mutex is poisoned; the session cannot be used safely",
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
            .map(|(capability, risk)| {
                ((*capability).to_owned(), (*risk).to_owned())
            })
            .collect(),
    };
    if raw.len() > MAX_CAPABILITY_REQUESTS {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "requested capability count exceeds the protocol-1.1 session bound",
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
                "protocol 1.1 grants only read_only risk",
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
                        "capability {capability:?} is unavailable in the protocol-1.1 development server"
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
            .unwrap_or(WorkBudget::CONSERVATIVE_DEFAULT.max_wall_millis),
        max_game_ticks: max_game_ticks
            .unwrap_or(WorkBudget::CONSERVATIVE_DEFAULT.max_game_ticks),
        max_entities: max_entities
            .unwrap_or(WorkBudget::CONSERVATIVE_DEFAULT.max_entities),
        max_bytes: max_bytes.unwrap_or(WorkBudget::CONSERVATIVE_DEFAULT.max_bytes),
        max_output_tokens: max_output_tokens
            .unwrap_or(WorkBudget::CONSERVATIVE_DEFAULT.max_output_tokens),
        max_actions: max_actions
            .unwrap_or(WorkBudget::CONSERVATIVE_DEFAULT.max_actions),
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
            "requested budget exceeds the protocol-1.1 server ceiling",
        ));
    }
    if budget.max_bytes < MIN_RESPONSE_BYTES
        || budget.max_output_tokens < MIN_RESPONSE_TOKENS
    {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "protocol-1.1 session budget is too small for the mandatory Agent Turn spine",
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

fn env_i32(name: &str, default: i32, minimum: i32, maximum: i32) -> Result<i32> {
    let value = match env::var(name) {
        Ok(raw) => raw.parse::<i32>().map_err(|_| {
            error(
                ErrorCode::InvalidRequest,
                format!("{name} must be a decimal i32"),
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
    bytes.extend_from_slice(b"dfmcp-live-mcp-nonce-v1-1\0");
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
            "DFMCP_BRIDGE_TOKEN is required in the development server environment",
        )),
        Err(env::VarError::NotUnicode(_)) => Err(error(
            ErrorCode::InvalidRequest,
            "DFMCP_BRIDGE_TOKEN must be valid UTF-8",
        )),
    }
}

fn live_source(session_id: SessionId) -> Result<AuthenticatedLiveSourceV1_1> {
    let endpoint = bridge_endpoint()?;
    let address = parse_loopback_endpoint(&endpoint)?;
    let credentials = BridgeCredentialsV1_1::new(
        bridge_token()?,
        bridge_nonce(session_id, &endpoint)?,
    )?;
    let connect_millis = env_u64("DFMCP_BRIDGE_CONNECT_MILLIS", 2_000, 1, 60_000)?;
    let read_millis = env_u64("DFMCP_BRIDGE_READ_MILLIS", 5_000, 1, 60_000)?;
    let write_millis = env_u64("DFMCP_BRIDGE_WRITE_MILLIS", 5_000, 1, 60_000)?;
    connect_authenticated_live_source_v1_1(
        &LiveConnectionConfig {
            endpoint: address,
            connect_timeout: Duration::from_millis(connect_millis),
            read_timeout: Duration::from_millis(read_millis),
            write_timeout: Duration::from_millis(write_millis),
            client_name: "dwarf-fortress-mcp-live-v1-1-development".to_owned(),
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

fn announcement_state_json(session: &LiveSessionV1_1) -> JsonValue {
    let Some(capsule) = session.adapter.last_capsule() else {
        return JsonValue::Null;
    };
    let coverage = &capsule.announcement_batch.coverage;
    json!({
        "source_digest": capsule.announcement_batch.content_digest.to_string(),
        "requested_after_id": coverage.requested_after_id,
        "next_after_id": coverage.next_after_id,
        "oldest_available_id": coverage.oldest_available_id,
        "latest_available_id": coverage.latest_available_id,
        "returned": coverage.returned,
        "complete_through_latest": coverage.complete_through_latest,
        "gap_before_retained_window": coverage.has_gap(),
        "complete_history": false,
    })
}

fn coverage_json(session: &LiveSessionV1_1) -> JsonValue {
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

fn announcement_briefing_json(session: &LiveSessionV1_1) -> JsonValue {
    let Some(capsule) = session.adapter.last_capsule() else {
        return JsonValue::Null;
    };
    match build_live_announcement_briefing(&capsule.announcement_batch) {
        Ok(briefing) => json!({
            "source_digest": briefing.source_digest.to_string(),
            "bridge_generation": briefing.bridge_generation,
            "requested_after_id": briefing.requested_after_id,
            "next_after_id": briefing.next_after_id,
            "oldest_available_id": briefing.oldest_available_id,
            "latest_available_id": briefing.latest_available_id,
            "returned": briefing.returned,
            "complete_through_latest": briefing.complete_through_latest,
            "gap_before_retained_window": briefing.gap_before_retained_window,
            "complete_history": briefing.complete_history,
            "records_truncated_for_briefing": briefing.records_truncated_for_briefing,
            "latest_records": briefing.latest_records.iter().map(|record| json!({
                "report_id": record.report_id,
                "report_type": record.report_type,
                "text": record.text,
                "year": record.year,
                "year_tick": record.year_tick,
                "repeat_count": record.repeat_count,
                "continuation": record.continuation,
                "unconscious": record.unconscious,
                "announcement": record.announcement,
            })).collect::<Vec<_>>(),
        }),
        Err(failure) => json!({
            "status": "invalid",
            "error": failure.message,
            "complete_history": false,
        }),
    }
}

fn briefing_json(session: &LiveSessionV1_1) -> JsonValue {
    let capsule = session.adapter.last_capsule();
    let identity = session.adapter.identity();
    json!({
        "implementation_phase": LIVE_IMPLEMENTATION_PHASE,
        "runtime": "unadmitted_development",
        "compatibility_admitted": false,
        "server_artifact_qualified": false,
        "runtime_admitted": false,
        "adapter": identity.name,
        "compatibility": format!("{:?}", identity.compatibility),
        "bridge_protocol": identity.bridge_protocol_version,
        "live": true,
        "read_only": true,
        "fortress_loaded": capsule.is_some(),
        "world_name": capsule.map(|value| value.base.world_name.clone()),
        "world_folder": capsule.map(|value| value.base.world_folder.clone()),
        "site_id": capsule.map(|value| value.base.site_id),
        "paused": capsule.map(|value| value.base.paused),
        "calendar_year": capsule.map(|value| value.base.current_year),
        "year_tick": capsule.map(|value| value.base.current_year_tick),
        "citizen_count": capsule.map(|value| value.base.citizen_coverage.total),
        "citizen_names_observed": capsule.map(|value| value.base.names_included),
        "announcements": announcement_briefing_json(session),
        "mutation_admissible": false,
        "source_poisoned": session.source_poisoned(),
        "highest_unresolved_uncertainty": "the retained announcement suffix is not complete fortress history and this development process is not compatibility-admitted",
    })
}

fn announcement_attention(session: &LiveSessionV1_1) -> Vec<JsonValue> {
    let Some(capsule) = session.adapter.last_capsule() else {
        return Vec::new();
    };
    let Ok(briefing) = build_live_announcement_briefing(&capsule.announcement_batch) else {
        return vec![json!({
            "attention_id": "live.announcements.invalid_briefing",
            "category": "control_plane",
            "severity": "high",
            "urgency": "now",
            "confidence": {"epistemic_state": "observed", "value": 1.0},
            "finding": "canonical announcement briefing could not be derived",
            "likely_consequence_if_ignored": "announcement orientation may be incomplete",
            "evidence": [capsule.announcement_batch.content_digest.to_string()],
        })];
    };
    briefing
        .attention
        .iter()
        .map(|item| {
            json!({
                "attention_id": item.attention_id,
                "category": item.category,
                "severity": item.severity.as_str(),
                "urgency": if item.severity.as_str() == "high" { "now" } else { "persistent" },
                "confidence": {"epistemic_state": "certified_derived", "value": 1.0},
                "finding": item.finding,
                "likely_consequence_if_ignored": if item.category == "continuity" {
                    "older announcement history may be unavailable"
                } else {
                    "the agent may miss newly retained fortress information"
                },
                "report_ids": item.report_ids,
                "score_components": item.score_components,
                "evidence": [item.source_digest.to_string()],
            })
        })
        .collect()
}

fn affordances_json(session: &LiveSessionV1_1) -> Vec<JsonValue> {
    [
        (Capability::Observe, "observe-live-v1-1", "fortress.observe", "observe"),
        (Capability::Query, "query-live-v1-1", "fortress.query", "query"),
        (Capability::Query, "explain-live-v1-1", "fortress.explain", "explain"),
        (Capability::Observe, "wait-live-v1-1", "fortress.wait", "wait"),
        (Capability::Doctor, "doctor-live-v1-1", "fortress.doctor", "doctor"),
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
                json!("the protocol-1.1 source is poisoned; open a fresh session")
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

fn uncertainties_json(session: &LiveSessionV1_1) -> Vec<JsonValue> {
    let mut values = OMITTED_LIVE_DOMAINS
        .iter()
        .map(|(domain, reason)| {
            uncertainty(
                format!("omitted-{domain}"),
                "unknown",
                format!("{domain} is not completely observed"),
                *reason,
                None,
                json!({}),
            )
        })
        .collect::<Vec<_>>();
    values.push(uncertainty(
        "protocol-1-1-development-unadmitted",
        "observed",
        "this process is an explicitly unadmitted development runtime",
        "successful reads do not establish compatibility, server qualification, floor acceptance, or runtime admission",
        None,
        json!({}),
    ));
    values.push(uncertainty(
        "live-mutation-unavailable",
        "observed",
        "protocol 1.1 has no mutation methods",
        "this session cannot change Dwarf Fortress state",
        None,
        json!({}),
    ));
    if session.source_poisoned() {
        values.push(uncertainty(
            "live-source-poisoned",
            "stale",
            "the protocol-1.1 source is permanently fenced after a failed call",
            session
                .source_poison_reason()
                .unwrap_or("the exact failure reason is unavailable"),
            Some("fortress.open_session"),
            json!({}),
        ));
    }
    values
}

fn references_json(session: &LiveSessionV1_1) -> Vec<JsonValue> {
    let Some(capsule) = session.adapter.last_capsule() else {
        return Vec::new();
    };
    vec![
        json!({
            "kind": "protocol_1_1_observation_capsule",
            "digest": capsule.content_digest.to_string(),
        }),
        json!({
            "kind": "protocol_1_1_announcement_batch",
            "digest": capsule.announcement_batch.content_digest.to_string(),
        }),
    ]
}

fn response_byte_limit(session: &LiveSessionV1_1) -> usize {
    let byte_budget = usize::try_from(session.budget.max_bytes).unwrap_or(usize::MAX);
    let token_budget = usize::try_from(session.budget.max_output_tokens)
        .ok()
        .and_then(|tokens| tokens.checked_mul(4))
        .unwrap_or(usize::MAX);
    byte_budget.min(token_budget)
}

fn attach_turn(
    session: &LiveSessionV1_1,
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
        .turn_id(format!("live-v1-1-turn-{request_id}"))
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
    let encoded = builder.attach(payload);
    if encoded.len() <= response_byte_limit(session) {
        return encoded;
    }
    AgentTurnBuilder::new(operation, phase)
        .session_id(session.session_id.to_string())
        .turn_id(format!("live-v1-1-turn-{request_id}"))
        .request_id(request_id.to_string())
        .continuity(continuity, basis.map(anchor_json), None, reset_reason)
        .profile(ObservationProfile::Pulse)
        .briefing(json!({
            "implementation_phase": LIVE_IMPLEMENTATION_PHASE,
            "runtime": "unadmitted_development",
            "read_only": true,
            "response_reduced": true,
        }))
        .active_work(empty_active_work())
        .budget(budget_json(session.budget))
        .attach(json!({
            "ok": false,
            "error": {
                "operation": operation,
                "code": ErrorCode::BudgetExceeded.as_str(),
                "message": "final protocol-1.1 Agent Turn exceeded the negotiated response budget",
                "retryable": false,
                "details": [],
                "recovery": recovery_guidance(
                    RecoveryClass::NeverUnchanged,
                    None,
                    "open a new development session with a larger output budget or request a narrower query",
                    json!({}),
                ),
            },
        }))
}

fn recovery_class(code: ErrorCode) -> RecoveryClass {
    match code {
        ErrorCode::CursorGap
        | ErrorCode::StaleAnchor
        | ErrorCode::PreconditionsFailed => RecoveryClass::RefreshAndRetry,
        ErrorCode::AdapterUnavailable | ErrorCode::AdapterFailure => {
            RecoveryClass::Backoff
        }
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
            "runtime": "unadmitted_development",
            "compatibility_admitted": false,
            "server_artifact_qualified": false,
            "runtime_admitted": false,
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
                "open-protocol-1-1-development-session",
                "fortress.open_session",
                "establish an authenticated canonical citizen-plus-announcement anchor",
                "high",
                "high",
                "read_only",
                "not_applicable",
                false,
                json!({}),
            )]
        })
        .uncertainty(vec![uncertainty(
            "protocol-1-1-session-unavailable",
            "unknown",
            "no protocol-1.1 development session is available",
            "no live fortress or announcement fact is established by this response",
            Some("fortress.open_session"),
            json!({}),
        )])
        .attach(error_payload(operation, failure))
}

fn session_error(
    session: &LiveSessionV1_1,
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
            "replace-poisoned-protocol-1-1-session",
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
            "attention_id": "protocol-1-1-runtime-error",
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

fn validate_bootstrap_budget(
    adapter: &LiveMcpAdapterV1_1,
    budget: WorkBudget,
) -> Result<()> {
    let capsule = adapter.last_capsule().ok_or_else(|| {
        error(
            ErrorCode::InternalInvariantViolation,
            "protocol-1.1 bootstrap returned no source capsule",
        )
    })?;
    let canonical_bytes = u64::try_from(capsule.canonical_bytes.len()).map_err(|_| {
        error(
            ErrorCode::BudgetExceeded,
            "protocol-1.1 capsule size cannot be represented in the negotiated budget",
        )
    })?;
    if canonical_bytes > budget.max_bytes {
        return Err(error(
            ErrorCode::BudgetExceeded,
            format!(
                "protocol-1.1 capsule requires {canonical_bytes} bytes, exceeding the negotiated {}-byte ceiling",
                budget.max_bytes
            ),
        ));
    }
    let projection = adapter.current_projection().ok_or_else(|| {
        error(
            ErrorCode::InternalInvariantViolation,
            "protocol-1.1 bootstrap returned no canonical projection",
        )
    })?;
    let entities = u32::try_from(projection.snapshot.graph.entities.len()).map_err(|_| {
        error(
            ErrorCode::BudgetExceeded,
            "protocol-1.1 projection entity count cannot be represented in u32",
        )
    })?;
    if entities > budget.max_entities {
        return Err(error(
            ErrorCode::BudgetExceeded,
            format!(
                "protocol-1.1 projection contains {entities} entities, exceeding the negotiated {}-entity ceiling",
                budget.max_entities
            ),
        ));
    }
    Ok(())
}

fn query_kinds(mode: &str) -> Result<Vec<EntityKind>> {
    match mode {
        "summary" => Ok(vec![EntityKind::Fortress]),
        "citizens" => Ok(vec![EntityKind::Unit]),
        "announcements" => Ok(vec![EntityKind::Announcement]),
        "all" => Ok(Vec::new()),
        _ => Err(error(
            ErrorCode::InvalidRequest,
            "query mode must be summary, citizens, announcements, or all",
        )),
    }
}

#[tool(
    description = "Open an explicitly unadmitted protocol-1.1 development session over the authenticated read-only DFHack bridge. Endpoint and bearer token are process configuration, never MCP arguments."
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
        Err(failure) => {
            return unbound_error(operation, AgentPhase::Bootstrap, &failure);
        }
    };
    let capabilities = match parse_requested_capabilities(requested_capabilities) {
        Ok(value) => value,
        Err(failure) => {
            return unbound_error(operation, AgentPhase::Bootstrap, &failure);
        }
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
        Err(failure) => {
            return unbound_error(operation, AgentPhase::Bootstrap, &failure);
        }
    };
    if sessions().is_ok_and(|registry| registry.len() >= MAX_LIVE_MCP_SESSIONS) {
        return unbound_error(
            operation,
            AgentPhase::Bootstrap,
            &error(
                ErrorCode::BudgetExceeded,
                "protocol-1.1 development server reached its explicit session bound",
            ),
        );
    }
    let session_id = match next_session_id() {
        Ok(value) => value,
        Err(failure) => {
            return unbound_error(operation, AgentPhase::Bootstrap, &failure);
        }
    };
    let source = match live_source(session_id) {
        Ok(value) => value,
        Err(failure) => {
            return unbound_error(operation, AgentPhase::Bootstrap, &failure);
        }
    };
    let hard_citizens = match u32::try_from(MAX_CAPSULE_CITIZENS) {
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
    let environment_max_citizens = match env_u32(
        "DFMCP_BRIDGE_MAX_CITIZENS",
        u32::try_from(MAX_CAPSULE_CITIZENS).unwrap_or(u32::MAX),
        0,
        hard_citizens,
    ) {
        Ok(value) => value,
        Err(failure) => {
            return unbound_error(operation, AgentPhase::Bootstrap, &failure);
        }
    };
    let max_citizens =
        environment_max_citizens.min(budget.max_entities.saturating_sub(1));
    let citizen_page_size = match env_u32(
        "DFMCP_BRIDGE_PAGE_SIZE",
        MAX_V1_1_CITIZENS_PER_PAGE,
        1,
        MAX_V1_1_CITIZENS_PER_PAGE,
    ) {
        Ok(value) => value,
        Err(failure) => {
            return unbound_error(operation, AgentPhase::Bootstrap, &failure);
        }
    };
    let include_names = match env_bool("DFMCP_BRIDGE_INCLUDE_NAMES", true) {
        Ok(value) => value,
        Err(failure) => {
            return unbound_error(operation, AgentPhase::Bootstrap, &failure);
        }
    };
    let announcement_after_id = match env_i32(
        "DFMCP_BRIDGE_ANNOUNCEMENT_AFTER_ID",
        -1,
        -1,
        i32::MAX,
    ) {
        Ok(value) => value,
        Err(failure) => {
            return unbound_error(operation, AgentPhase::Bootstrap, &failure);
        }
    };
    let hard_announcements = match u32::try_from(MAX_ANNOUNCEMENTS_PER_BATCH) {
        Ok(value) => value,
        Err(_) => {
            return unbound_error(
                operation,
                AgentPhase::Bootstrap,
                &error(
                    ErrorCode::InternalInvariantViolation,
                    "announcement ceiling does not fit u32",
                ),
            );
        }
    };
    let announcement_page_size = match env_u32(
        "DFMCP_BRIDGE_ANNOUNCEMENT_PAGE_SIZE",
        DEFAULT_LIVE_ANNOUNCEMENT_PAGE_SIZE,
        1,
        hard_announcements,
    ) {
        Ok(value) => value,
        Err(failure) => {
            return unbound_error(operation, AgentPhase::Bootstrap, &failure);
        }
    };
    let max_total_announcements = match env_u32(
        "DFMCP_BRIDGE_MAX_ANNOUNCEMENTS",
        DEFAULT_MAX_LIVE_ANNOUNCEMENTS,
        1,
        hard_announcements,
    ) {
        Ok(value) => value,
        Err(failure) => {
            return unbound_error(operation, AgentPhase::Bootstrap, &failure);
        }
    };
    let adapter = match bootstrap_live_read_adapter_v1_1(
        source,
        LiveReadBootstrapConfigV1_1 {
            citizen_page_size,
            max_citizens,
            include_names,
            announcement_after_id,
            announcement_page_size,
            max_total_announcements,
            initial_epoch: 0,
        },
    ) {
        Ok(value) => value,
        Err(failure) => {
            return unbound_error(operation, AgentPhase::Bootstrap, &failure);
        }
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
                    "protocol-1.1 bootstrap returned no canonical anchor",
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
                "fortress_selector does not match the authenticated protocol-1.1 fortress identity",
            ),
        );
    }
    if let Some(expected) = paused {
        let matches = adapter
            .last_capsule()
            .is_some_and(|capsule| capsule.base.paused == expected);
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
    let session = Arc::new(Mutex::new(LiveSessionV1_1 {
        session_id,
        adapter,
        grants,
        budget,
        next_request_id: 1,
    }));
    {
        let mut registry = match sessions() {
            Ok(value) => value,
            Err(failure) => {
                return unbound_error(operation, AgentPhase::Bootstrap, &failure);
            }
        };
        if registry.len() >= MAX_LIVE_MCP_SESSIONS {
            return unbound_error(
                operation,
                AgentPhase::Bootstrap,
                &error(
                    ErrorCode::BudgetExceeded,
                    "protocol-1.1 development server reached its explicit session bound",
                ),
            );
        }
        if registry.insert(session_id, Arc::clone(&session)).is_some() {
            return unbound_error(
                operation,
                AgentPhase::Bootstrap,
                &error(
                    ErrorCode::InternalInvariantViolation,
                    "fresh protocol-1.1 session identifier collided with an existing session",
                ),
            );
        }
    }
    let guard = match lock_session(&session) {
        Ok(value) => value,
        Err(failure) => {
            return unbound_error(operation, AgentPhase::Bootstrap, &failure);
        }
    };
    let capsule = match guard.adapter.last_capsule() {
        Some(value) => value,
        None => {
            return unbound_error(
                operation,
                AgentPhase::Bootstrap,
                &error(
                    ErrorCode::InternalInvariantViolation,
                    "registered protocol-1.1 session lost its source capsule",
                ),
            );
        }
    };
    let request_id = RequestId::new(1);
    let mut attention = announcement_attention(&guard);
    attention.insert(
        0,
        json!({
            "attention_id": "protocol-1-1-unadmitted-development-runtime",
            "category": "admission",
            "severity": "high",
            "urgency": "persistent",
            "confidence": {"epistemic_state": "observed", "value": 1.0},
            "finding": "this protocol-1.1 MCP process is explicitly unadmitted development evidence",
            "likely_consequence_if_ignored": "a successful read could be misrepresented as compatibility or release evidence",
            "evidence": [],
        }),
    );
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
            "kind": "protocol_1_1_development_session_opened",
            "subject": {"fortress_id": anchor.fortress_id.to_string()},
            "epistemic_state": "observed",
            "invalidates": [],
            "evidence": [capsule.content_digest.to_string()],
        })],
        attention,
        vec![recommendation(
            "observe-protocol-1-1-pulse",
            "fortress.observe",
            "refresh the combined citizen-plus-announcement anchor",
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
            "mode": "authenticated_live_read_only_unadmitted_development",
            "compatibility_admitted": false,
            "runtime_admitted": false,
            "adapter": guard.adapter.identity().name,
            "fortress_id": anchor.fortress_id.to_string(),
            "anchor": anchor_json(anchor),
            "paused": capsule.base.paused,
            "granted_capabilities": capabilities.iter().map(|value| value.as_str()).collect::<Vec<_>>(),
            "budget": budget_json(budget)["admitted"].clone(),
            "bridge": {
                "protocol": "1.1",
                "version": capsule.base.bridge.bridge_version,
                "generation": capsule.base.bridge.bridge_generation,
                "dfhack_version": capsule.base.bridge.dfhack_version,
                "dwarf_fortress_version": capsule.base.bridge.df_version,
                "supported_methods": capsule.base.bridge.supported_methods,
                "mutation_methods": [],
            },
            "announcements": announcement_state_json(&guard),
        }),
    )
}

#[tool(
    description = "Refresh an explicitly unadmitted protocol-1.1 read-only session. Returns a heartbeat or a complete citizen-plus-retained-announcement snapshot."
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
    let prior_batch = guard
        .adapter
        .last_capsule()
        .map(|capsule| capsule.announcement_batch.clone());
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
    let reset = current.cursor.epoch != prior.cursor.epoch;
    let (kind, continuity, reset_reason, mut changes) = match &frame.payload {
        ObservationPayload::Heartbeat(_) => (
            "heartbeat",
            ContinuityStatus::Heartbeat,
            None,
            Vec::new(),
        ),
        ObservationPayload::Snapshot(_) if reset => (
            "snapshot",
            ContinuityStatus::Reset,
            Some("bridge_generation_or_game_clock_reset".to_owned()),
            vec![json!({
                "kind": "observation_epoch_reset",
                "subject": {"fortress_id": current.fortress_id.to_string()},
                "epistemic_state": "observed",
                "invalidates": ["all_prior_protocol_1_1_continuations", "all_prior_protocol_1_1_recommendations"],
                "evidence": frame.evidence.iter().map(|value| value.digest.to_string()).collect::<Vec<_>>(),
            })],
        ),
        ObservationPayload::Snapshot(_) => (
            "snapshot",
            ContinuityStatus::Continuous,
            None,
            vec![json!({
                "kind": "protocol_1_1_state_advanced",
                "subject": {"fortress_id": current.fortress_id.to_string()},
                "epistemic_state": "observed",
                "invalidates": ["prior_protocol_1_1_recommendations"],
                "evidence": frame.evidence.iter().map(|value| value.digest.to_string()).collect::<Vec<_>>(),
            })],
        ),
        ObservationPayload::Delta(_) => {
            let failure = error(
                ErrorCode::InternalInvariantViolation,
                "protocol-1.1 adapter unexpectedly returned a delta payload",
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
    if !reset {
        if let (Some(basis), Some(target)) = (
            prior_batch.as_ref(),
            guard
                .adapter
                .last_capsule()
                .map(|capsule| &capsule.announcement_batch),
        ) {
            match summarize_live_announcement_change(basis, target) {
                Ok(summary) if !summary.heartbeat => changes.push(json!({
                    "kind": "retained_announcements_advanced",
                    "epistemic_state": "certified_derived",
                    "basis_digest": summary.basis_digest.to_string(),
                    "target_digest": summary.target_digest.to_string(),
                    "added_report_ids": summary.added_report_ids,
                    "ids_truncated": summary.ids_truncated,
                    "cursor_advanced": summary.cursor_advanced,
                    "retained_window_gap_introduced": summary.retained_window_gap_introduced,
                    "continuation_required": summary.continuation_required,
                    "invalidates": ["prior_announcement_attention"],
                    "evidence": [summary.target_digest.to_string()],
                })),
                Ok(_) => {}
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
            }
        }
    }
    let attention = announcement_attention(&guard);
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
        attention,
        Vec::new(),
        json!({
            "ok": true,
            "session_id": guard.session_id.to_string(),
            "request_id": request_id.to_string(),
            "kind": kind,
            "anchor": anchor_json(current),
            "paused": capsule.map(|value| value.base.paused),
            "calendar_year": capsule.map(|value| value.base.current_year),
            "year_tick": capsule.map(|value| value.base.current_year_tick),
            "citizens": capsule.map(|value| value.base.citizen_coverage.total),
            "announcements": announcement_state_json(&guard),
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
    description = "Query the current protocol-1.1 canonical snapshot. mode is summary, citizens, announcements, or all. Querying never refreshes state."
)]
pub fn fortress_query(session_id: Option<String>, mode: Option<String>) -> String {
    let operation = "fortress.query";
    let mode = mode.unwrap_or_else(|| "summary".to_owned());
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
    let kinds = match query_kinds(&mode) {
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
        announcement_attention(&guard),
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
            "{operation} cannot succeed: protocol 1.1 is read-only and registers no mutation methods"
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
            "attention_id": "protocol-1-1-mutation-unavailable",
            "category": "capability",
            "severity": "medium",
            "urgency": "persistent",
            "confidence": {"epistemic_state": "observed", "value": 1.0},
            "finding": "the protocol-1.1 bridge has no mutation method set",
            "likely_consequence_if_ignored": "repeated mutation attempts will continue to fail unchanged",
            "evidence": [],
        })],
        Vec::new(),
        error_payload(operation, &failure),
    )
}

#[tool(description = "Unavailable in read-only protocol 1.1; fails closed without preparing an effect.")]
pub fn fortress_plan(
    session_id: Option<String>,
    _summary: Option<String>,
    _paused_target: Option<bool>,
) -> String {
    read_only_tool_error(session_id, "fortress.plan", AgentPhase::Propose)
}

#[tool(description = "Unavailable in read-only protocol 1.1; fails closed without committing an effect.")]
pub fn fortress_commit(session_id: Option<String>, _plan_digest: String) -> String {
    read_only_tool_error(session_id, "fortress.commit", AgentPhase::Commit)
}

#[tool(description = "Report that protocol 1.1 has no active mutation work, then recommend a read-only observation only when useful.")]
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
            "observe-after-empty-protocol-1-1-wait",
            "fortress.observe",
            "no mutation work exists; refresh combined state only when new information is valuable",
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
        announcement_attention(&guard),
        recommendations,
        json!({
            "ok": true,
            "session_id": guard.session_id.to_string(),
            "request_id": request_id.to_string(),
            "active_work": [],
            "terminal": true,
            "summary": "protocol 1.1 has no mutation work to poll",
            "anchor": anchor_json(anchor),
            "announcements": announcement_state_json(&guard),
        }),
    )
}

#[tool(description = "Unavailable in read-only protocol 1.1; fails closed without cancellation effects.")]
pub fn fortress_cancel(session_id: Option<String>, _mode: Option<String>) -> String {
    read_only_tool_error(session_id, "fortress.cancel", AgentPhase::Reconcile)
}

#[tool(description = "Unavailable in read-only protocol 1.1; no game/save checkpoint is created.")]
pub fn fortress_checkpoint(session_id: Option<String>, _label: Option<String>) -> String {
    read_only_tool_error(session_id, "fortress.checkpoint", AgentPhase::Commit)
}

#[tool(description = "Unavailable in read-only protocol 1.1; no restore or epoch mutation is attempted.")]
pub fn fortress_restore(session_id: Option<String>, _checkpoint_id: String) -> String {
    read_only_tool_error(session_id, "fortress.restore", AgentPhase::Reconcile)
}

#[tool(
    description = "Explain the protocol-1.1 adapter, retained-announcement coverage, or one canonical fortress, citizen, or announcement entity."
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
                "protocol-1.1 session lost its canonical projection",
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
                        "canonical entity ID is not present at the current protocol-1.1 anchor",
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
            "kind": "protocol_1_1_development_session",
            "projection_schema": projection.receipt.schema(),
            "source_capsule_digest": projection.receipt.source_capsule_digest().to_string(),
            "source_citizen_capsule_digest": projection.receipt.source_citizen_capsule_digest().to_string(),
            "source_announcement_batch_digest": projection.receipt.source_announcement_batch_digest().to_string(),
            "bridge_generation": projection.receipt.source_bridge_generation(),
            "anchor": anchor_json(anchor),
            "coverage": coverage_json(&guard),
            "announcements": announcement_state_json(&guard),
            "authority": {
                "observe": guard.has_grant(Capability::Observe),
                "query": guard.has_grant(Capability::Query),
                "doctor": guard.has_grant(Capability::Doctor),
                "mutation": false,
                "compatibility_admitted": false,
                "runtime_admitted": false,
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
        announcement_attention(&guard),
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
    description = "Diagnose an explicitly unadmitted protocol-1.1 development session, including exact versions, retained-window coverage, and permanent source-fence state."
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
    let mut attention = announcement_attention(&guard);
    if poisoned {
        attention.insert(
            0,
            json!({
                "attention_id": "replace-poisoned-protocol-1-1-session",
                "category": "continuity",
                "severity": "critical",
                "urgency": "now",
                "confidence": {"epistemic_state": "observed", "value": 1.0},
                "finding": "the protocol-1.1 source is permanently fenced after a failed call",
                "likely_consequence_if_ignored": "no later observation can safely use this transport",
                "evidence": [],
            }),
        );
    }
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
                "open-replacement-protocol-1-1-session",
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
            "status": if poisoned { "degraded" } else { "read_only_development" },
            "runtime": "unadmitted_development",
            "compatibility_admitted": false,
            "server_artifact_qualified": false,
            "runtime_admitted": false,
            "adapter": health.identity.name,
            "bridge_protocol": health.identity.bridge_protocol_version,
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
            "announcements": announcement_state_json(&guard),
        }),
    )
}

/// Run the explicitly unadmitted protocol-1.1 development MCP server on stdio.
pub fn run_live_v1_1_development_stdio() {
    if let Err(failure) = validate_development_runtime_environment() {
        eprintln!("protocol-1.1 development startup: FAIL: {failure}");
        std::process::exit(1);
    }
    ServerBuilder::new(
        "dwarf-fortress-mcp-live-v1-1-development",
        env!("CARGO_PKG_VERSION"),
    )
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
        "Explicitly unadmitted Dwarf Fortress protocol-1.1 development server. Set \
         DFMCP_ALLOW_UNADMITTED_LIVE_V1_1=1 to start it. This runtime cannot consume \
         production admission state and proves no compatibility. Endpoint and bearer token \
         are process configuration, never tool arguments. Call fortress.open_session first. \
         The frozen eleven-tool waist is preserved; Handshake and ReadObservation are the only \
         bridge methods; all mutation-stage tools fail closed. Query modes are summary, \
         citizens, announcements, and all. Every result carries an Agent Turn with exact anchor, \
         retained-window coverage, explicit historical uncertainty, authority, and recovery.",
    )
    .build()
    .run_stdio();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn development_gate_requires_exact_opt_in() {
        assert!(development_opt_in_value(Some("1")).is_ok());
        assert!(development_opt_in_value(None).is_err());
        assert!(development_opt_in_value(Some("true")).is_err());
        assert!(development_opt_in_value(Some("01")).is_err());
    }

    #[test]
    fn production_admission_environment_is_rejected_by_name() {
        for name in [
            "DFMCP_ADMISSION_TICKET",
            "DFMCP_COMPATIBILITY_ENTRY_ID",
            "DFMCP_COMPATIBILITY_DECISION_DIGEST",
            "DFMCP_COMPATIBILITY_REGISTRY_DIGEST",
            "DFMCP_COMPATIBILITY_FLOOR_DIGEST",
            "DFMCP_COMPATIBILITY_FLOOR_SEQUENCE",
            "DFMCP_SERVER_RECEIPT_DIGEST",
            "DFMCP_ADMITTED_LAUNCH_DIGEST",
        ] {
            assert!(forbidden_admission_environment_name(name));
        }
        assert!(!forbidden_admission_environment_name(DEVELOPMENT_OPT_IN));
        assert!(!forbidden_admission_environment_name("DFMCP_BRIDGE_TOKEN"));
    }

    #[test]
    fn protocol_1_1_session_ids_are_generation_namespaced() -> Result<()> {
        let session = SessionId::new(SESSION_NAMESPACE_PREFIX | 17);
        assert_eq!(parse_session_id(&session.to_string())?, session);
        assert!(parse_session_id("80000000000000000000000000000011").is_err());
        assert!(parse_session_id("11").is_err());
        assert!(parse_session_id("00000000000000000000000000000000").is_err());
        Ok(())
    }

    #[test]
    fn query_modes_are_explicit_and_announcement_aware() -> Result<()> {
        assert_eq!(query_kinds("summary")?, vec![EntityKind::Fortress]);
        assert_eq!(query_kinds("citizens")?, vec![EntityKind::Unit]);
        assert_eq!(
            query_kinds("announcements")?,
            vec![EntityKind::Announcement]
        );
        assert!(query_kinds("all")?.is_empty());
        assert!(query_kinds("events").is_err());
        Ok(())
    }

    #[test]
    fn response_budget_reserves_mandatory_agent_turn_space() {
        assert!(requested_budget(None, None, None, None, None, None).is_ok());
        assert!(
            requested_budget(None, None, None, Some(MIN_RESPONSE_BYTES - 1), None, None)
                .is_err()
        );
        assert!(
            requested_budget(
                None,
                None,
                None,
                None,
                Some(MIN_RESPONSE_TOKENS - 1),
                None,
            )
            .is_err()
        );
    }

    #[test]
    fn unbound_error_never_implies_runtime_admission() -> Result<()> {
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
        assert_eq!(value["agent_turn"]["briefing"]["runtime_admitted"], false);
        assert_eq!(
            value["agent_turn"]["briefing"]["compatibility_admitted"],
            false
        );
        assert_eq!(value["agent_turn"]["operation"], "fortress.observe");
        Ok(())
    }

    #[test]
    fn world_values_have_deterministic_machine_projections() {
        assert_eq!(world_value_json(&WorldValue::Bool(true)), json!(true));
        assert_eq!(
            world_value_json(&WorldValue::Bytes(vec![0, 1, 254, 255])),
            json!({
                "encoding": "hex",
                "hex": "0001feff",
                "byte_length": 4,
            })
        );
    }
}
