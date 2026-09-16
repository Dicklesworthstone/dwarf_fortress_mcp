//! Foreground, process-local query baselines. No watcher or bridge work is spawned.
//! Captures are immutable until explicitly released. Changes compare endpoints,
//! never claim a continuous event history, and never authorize game mutations.

#[path = "query_history_endpoints.rs"]
mod endpoints;
pub(super) use endpoints::compare_endpoints;

use std::collections::BTreeMap;
use std::sync::{LazyLock, Mutex, MutexGuard};
use std::hash::{BuildHasher, Hasher};

use dfmcp_core::{Capability, DfmcpError, Digest32, ErrorCode, OperationContext,
    Result, RiskTier, SessionId, StateAnchor};
use dfmcp_world::WorldSnapshot;
use serde::Deserialize;
use serde_json::{Value, json};

const MAX_BASELINES: usize = 128;
const MAX_PER_SESSION: usize = 8;
const MAX_ROWS: usize = 256;
const MAX_CAPTURE_BYTES: usize = 256 * 1024;
const MAX_PAGE_CALLS: usize = 64;
const MAX_SOURCE_ROW_VISITS: usize = 2_000_000;

type RowKey = (u64, u32);
type Rows = BTreeMap<RowKey, Value>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    schema: String,
    expected_anchor: Option<Value>,
    query: Request,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Capture { key: String, select: Value, max_game_ticks: u64 },
    Changes { baseline: String, limit: Option<u32>, continuation: Option<String> },
    Baselines,
    ReleaseBaseline { baseline: String },
}

#[derive(Clone)]
struct Baseline {
    id: String,
    key: String,
    anchor: StateAnchor,
    expires_at: u64,
    max_game_ticks: u64,
    selection: Value,
    rows: Rows,
    digest: Digest32,
    bytes: usize,
}

#[derive(Default)]
struct History {
    next_id: u64,
    entries: BTreeMap<(SessionId, String), Baseline>,
}

// A handle cannot alias a newly allocated baseline after a process restart.
// This incarnation discriminator is not a secret capability or an auth token.
static INCARNATION: LazyLock<(u64, u64)> = LazyLock::new(|| {
    let scope = || {
        let mut hash = std::collections::hash_map::RandomState::new().build_hasher();
        hash.write(b"dfmcp-query-baseline-store-v1");
        hash.finish()
    };
    (scope(), scope())
});

static HISTORY: LazyLock<Mutex<History>> = LazyLock::new(|| Mutex::new(History::default()));

fn failure(code: ErrorCode, message: &str) -> DfmcpError { DfmcpError::new(code, message) }
fn invalid(message: &str) -> DfmcpError { failure(ErrorCode::InvalidRequest, message) }
fn exhausted(message: &str) -> DfmcpError { failure(ErrorCode::BudgetExceeded, message) }
fn invariant(message: &str) -> DfmcpError { failure(ErrorCode::InternalInvariantViolation, message) }

fn lock(history: &Mutex<History>) -> Result<MutexGuard<'_, History>> {
    history.lock().map_err(|_| invariant("query baseline store is poisoned"))
}

fn encode(value: &Value) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|_| invariant("query baseline JSON cannot be encoded"))
}

fn anchor_json(anchor: StateAnchor) -> Value {
    json!({"fortress_id":anchor.fortress_id.to_string(), "epoch":anchor.cursor.epoch,
        "sequence":anchor.cursor.sequence, "game_tick":anchor.tick.0,
        "state_hash":anchor.state_hash.to_string()})
}

fn check_input(root: &Value) -> Result<()> {
    let mut pending = vec![(root, 0usize)];
    let (mut nodes, mut bytes) = (0usize, 0usize);
    while let Some((value, depth)) = pending.pop() {
        nodes += 1;
        bytes = bytes.saturating_add(32);
        if depth > 24 || nodes > 4096 { return Err(exhausted("history query exceeds its shape bound")); }
        match value {
            Value::String(text) => bytes = bytes.saturating_add(text.len()),
            Value::Array(values) => {
                if nodes.saturating_add(pending.len()).saturating_add(values.len()) > 4096 {
                    return Err(exhausted("history query exceeds its node bound"));
                }
                pending.extend(values.iter().map(|value| (value, depth + 1)));
            }
            Value::Object(values) => {
                if nodes.saturating_add(pending.len()).saturating_add(values.len()) > 4096 {
                    return Err(exhausted("history query exceeds its node bound"));
                }
                for (key, value) in values {
                    bytes = bytes.saturating_add(key.len());
                    pending.push((value, depth + 1));
                }
            }
            _ => {}
        }
        if bytes > 65_536 { return Err(exhausted("history query exceeds its input-byte bound")); }
    }
    Ok(())
}

fn validate_selection(selection: &Value) -> Result<()> {
    let object = selection.as_object().ok_or_else(|| invalid("capture select must be an entities query object"))?;
    if object.get("kind").and_then(Value::as_str) != Some("entities")
        || object.keys().any(|key| !matches!(key.as_str(), "kind" | "kinds" | "where" | "fields" | "order"))
    {
        return Err(invalid("capture select supports entities, kinds, where, fields, order only; pagination is owned by the capture"));
    }
    Ok(())
}

fn row_key(row: &Value) -> Result<RowKey> {
    let raw = row.get("entity_id").and_then(Value::as_str)
        .ok_or_else(|| invariant("captured row has no stable entity ID"))?;
    let id = raw.parse::<u64>().map_err(|_| invariant("captured row ID is not u64"))?;
    let generation = row.get("generation").and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| invariant("captured row has no generation"))?;
    if id == 0 || raw != id.to_string() { return Err(invariant("captured row ID is not canonical")); }
    Ok((id, generation))
}

/// Finish all pages against the same in-memory snapshot. Both total rows and
/// repeated scan work are bounded. A partial result is never captured as a set.
fn materialize(snapshot: &WorldSnapshot, context: &OperationContext, selection: &Value) -> Result<(Rows, usize)> {
    let started = std::time::Instant::now();
    let mut input = json!({"schema":"dfmcp.query/1", "query":selection});
    let mut page_limit = context.budget.max_entities.min(8);
    let mut rows = Rows::new();
    let (mut bytes, mut calls) = (0usize, 0usize);
    let mut expected_matched = None;
    loop {
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if started.elapsed().as_millis() >= u128::from(context.budget.max_wall_millis) {
            return Err(exhausted("query selection exhausted its foreground wall-time allowance"));
        }
        calls += 1;
        if calls > MAX_PAGE_CALLS
            || calls.saturating_mul(snapshot.graph.entities.len()) > MAX_SOURCE_ROW_VISITS
        {
            return Err(exhausted("query baseline acquisition exceeded its foreground page/scan budget; narrow the selection"));
        }
        input["query"]["limit"] = json!(page_limit);
        let result = match super::execute(snapshot, context, &input) {
            Ok(result) => result,
            Err(error) if error.code == ErrorCode::BudgetExceeded && page_limit > 1 => {
                page_limit /= 2;
                continue;
            }
            Err(error) => return Err(error),
        };
        if started.elapsed().as_millis() >= u128::from(context.budget.max_wall_millis) {
            return Err(exhausted("query selection exhausted its foreground wall-time allowance"));
        }
        if result.get("anchor") != Some(&anchor_json(snapshot.anchor())) {
            return Err(invariant("capture page changed the requested snapshot anchor"));
        }
        let matched = result.get("matched").and_then(Value::as_u64)
            .ok_or_else(|| invariant("capture page has no match count"))?;
        if matched > MAX_ROWS as u64 || expected_matched.is_some_and(|expected| expected != matched) {
            return Err(exhausted("baseline selection exceeds 256 rows or changed during acquisition"));
        }
        expected_matched = Some(matched);
        let page = result.get("rows").and_then(Value::as_array)
            .ok_or_else(|| invariant("capture page has no rows"))?;
        for row in page {
            bytes = bytes.checked_add(encode(row)?.len()).ok_or_else(|| exhausted("capture size overflow"))?;
            if bytes > MAX_CAPTURE_BYTES { return Err(exhausted("query baseline exceeds its retained-byte bound")); }
            if rows.insert(row_key(row)?, row.clone()).is_some() {
                return Err(invariant("query baseline acquisition repeated an entity generation"));
            }
        }
        match result.get("continuation").and_then(Value::as_str) {
            Some(token) => {
                if page.is_empty() || result.get("truncated") != Some(&json!(true)) {
                    return Err(invariant("capture pagination did not make explicit progress"));
                }
                input["query"]["continuation"] = json!(token);
            }
            None => {
                if result.get("truncated") != Some(&json!(false)) || rows.len() as u64 != matched {
                    return Err(invariant("query baseline acquisition is incomplete"));
                }
                return Ok((rows, bytes));
            }
        }
    }
}

fn rows_digest(rows: &Rows) -> Result<Digest32> {
    Ok(Digest32::of_bytes(&encode(&json!({"domain":"dfmcp-query-baseline-rows/1",
        "rows":rows.values().collect::<Vec<_>>()}))?))
}

/// Ignore only observation bookkeeping, not presence, epistemic class, source
/// kind, generation, label, or values. A provenance-only refresh is disclosed.
fn semantic_row(row: &Value) -> Value {
    let mut row = row.clone();
    if let Some(object) = row.as_object_mut() { object.remove("revision"); }
    if let Some(fields) = row.get_mut("fields").and_then(Value::as_object_mut) {
        for field in fields.values_mut() {
            if let Some(fact) = field.as_object_mut() {
                fact.remove("observed_at_game_tick");
                fact.remove("source_digest");
            }
        }
    }
    row
}

fn compare_rows(before: &Rows, after: &Rows) -> (Vec<Value>, usize) {
    let keys = before.keys().chain(after.keys()).copied().collect::<std::collections::BTreeSet<_>>();
    let mut changes = Vec::new();
    let mut refreshed = 0usize;
    for (id, generation) in keys {
        let old = before.get(&(id, generation));
        let new = after.get(&(id, generation));
        let kind = match (old, new) {
            (Some(old), Some(new)) if semantic_row(old) == semantic_row(new) => {
                if old != new { refreshed += 1; }
                continue;
            }
            (Some(_), Some(_)) => "changed_in_result",
            (None, Some(_)) => "entered_result",
            (Some(_), None) => "left_result",
            (None, None) => continue,
        };
        changes.push(json!({"kind":kind, "entity_id":id.to_string(), "generation":generation,
            "before":old, "after":new}));
    }
    (changes, refreshed)
}

fn check_baseline(baseline: &Baseline, current: StateAnchor) -> Result<()> {
    if baseline.anchor.fortress_id != current.fortress_id || baseline.anchor.cursor.epoch != current.cursor.epoch {
        return Err(failure(ErrorCode::StaleAnchor, "baseline belongs to another fortress or observation epoch; capture a new baseline"));
    }
    if current.cursor.sequence < baseline.anchor.cursor.sequence || current.tick < baseline.anchor.tick
        || (current.cursor == baseline.anchor.cursor && current != baseline.anchor)
    {
        return Err(failure(ErrorCode::StaleAnchor, "baseline cannot be compared with a regressed or forked anchor"));
    }
    if current.tick.0 >= baseline.expires_at {
        return Err(failure(ErrorCode::StaleAnchor, "query baseline reached its game-time deadline; capture a new baseline"));
    }
    Ok(())
}

fn base_payload(kind: &str, anchor: StateAnchor) -> Value {
    json!({"schema":"dfmcp.query.result/1", "kind":kind, "anchor":anchor_json(anchor),
        "truncated":false, "continuation":null,
        "coverage":{"domain":"selected_observed_projection", "absence_proven":false,
            "temporal_coverage":"endpoint_comparison_only", "intermediate_observations_retained":false},
        "storage":"bounded_process_local", "mutation_authority":false})
}

fn describe(baseline: &Baseline, current: StateAnchor) -> Value {
    json!({"baseline":baseline.id, "key":baseline.key, "basis":anchor_json(baseline.anchor),
        "expires_at_game_tick":baseline.expires_at, "rows":baseline.rows.len(),
        "retained_bytes":baseline.bytes, "source_result_digest":baseline.digest.to_string(),
        "status":if baseline.anchor.fortress_id != current.fortress_id || baseline.anchor.cursor.epoch != current.cursor.epoch {
            "invalidated_epoch"
        } else if current.tick.0 >= baseline.expires_at { "expired" } else { "retained" }})
}

fn cursor(offset: usize, identity: Digest32) -> Result<String> {
    let digest = Digest32::of_bytes(&encode(&json!({"domain":"dfmcp-query-changes-page/1",
        "identity":identity.to_string(), "offset":offset}))?);
    Ok(format!("qh1:{offset}:{digest}"))
}

fn offset(token: Option<&str>, identity: Digest32, total: usize) -> Result<usize> {
    let Some(token) = token else { return Ok(0); };
    if token.len() > 128 { return Err(invalid("changes continuation exceeds its byte bound")); }
    let mut parts = token.split(':');
    let (Some("qh1"), Some(raw), Some(_), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return Err(invalid("changes continuation has an invalid shape"));
    };
    let value = raw.parse::<usize>().map_err(|_| invalid("changes continuation offset is invalid"))?;
    if value == 0 || value >= total || raw != value.to_string() {
        return Err(failure(ErrorCode::CursorGap, "changes continuation makes no progress or exceeds the result horizon"));
    }
    if token != cursor(value, identity)? {
        return Err(failure(ErrorCode::StaleAnchor, "changes continuation belongs to a different baseline or target snapshot"));
    }
    Ok(value)
}

fn change_page(baseline: &Baseline, current: &WorldSnapshot, context: &OperationContext,
    limit: Option<u32>, continuation: Option<&str>) -> Result<Value> {
    check_baseline(baseline, current.anchor())?;
    let hard_limit = context.budget.max_entities.min(256);
    let limit = limit.unwrap_or(hard_limit.min(16));
    if limit == 0 || limit > hard_limit { return Err(invalid("changes limit exceeds the negotiated row limit")); }
    let (rows, _) = materialize(current, context, &baseline.selection)?;
    let result_digest = rows_digest(&rows)?;
    let identity = Digest32::of_bytes(&encode(&json!({"baseline":baseline.id,
        "target":anchor_json(current.anchor()), "result":result_digest.to_string()}))?);
    let (changes, refreshed) = compare_rows(&baseline.rows, &rows);
    let start = offset(continuation, identity, changes.len())?;
    let mut payload = base_payload("changes", current.anchor());
    payload["baseline"] = json!(baseline.id);
    payload["basis"] = anchor_json(baseline.anchor);
    payload["basis_result_digest"] = json!(baseline.digest.to_string());
    payload["target_result_digest"] = json!(result_digest.to_string());
    payload["matched_before"] = json!(baseline.rows.len());
    payload["matched_after"] = json!(rows.len());
    payload["change_count"] = json!(changes.len());
    payload["provenance_only_refreshes"] = json!(refreshed);
    payload["anchor_advanced"] = json!(baseline.anchor != current.anchor());
    payload["baseline_advanced"] = json!(false);
    payload["note"] = json!("left_result means no longer selected in this projection, not deleted or dead; intermediate changes are unknown");
    let maximum = usize::try_from(context.budget.max_bytes.min(u64::from(context.budget.max_output_tokens) * 4))
        .map_err(|_| exhausted("changes byte budget cannot be represented"))?;
    let mut end = start;
    payload["changes"] = json!([]);
    payload["returned"] = json!(0);
    for change in changes.iter().skip(start).take(limit as usize) {
        let mut candidate = payload.clone();
        let Some(array) = candidate["changes"].as_array_mut() else { return Err(invariant("change page lost its row array")); };
        array.push(change.clone());
        candidate["returned"] = json!(end + 1 - start);
        candidate["truncated"] = json!(end + 1 < changes.len());
        candidate["continuation"] = if end + 1 < changes.len() { json!(cursor(end + 1, identity)?) } else { Value::Null };
        if encode(&candidate)?.len() > maximum { break; }
        payload = candidate;
        end += 1;
    }
    if start < changes.len() && end == start {
        return Err(exhausted("one complete change cannot fit the result budget; increase the response budget or capture fewer fields"));
    }
    if encode(&payload)?.len() > maximum { return Err(exhausted("change-page metadata exceeds the response budget")); }
    Ok(payload)
}

fn validate_handle(handle: &str) -> Result<()> {
    if handle.len() != 68 || !handle.starts_with("qb1:")
        || !handle[4..].bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid("baseline must be a qb1 handle returned by capture or baselines"));
    }
    Ok(())
}

/// The publisher must validate/render the complete Agent Turn. A capture or
/// release becomes visible only after publication preparation succeeds.
pub(super) fn execute<F>(snapshot: &WorldSnapshot, context: &OperationContext,
    input: &Value, publish: F) -> Result<String>
where F: FnOnce(Value) -> Result<String> {
    execute_in(&HISTORY, snapshot, context, input, publish)
}

fn execute_in<F>(history: &Mutex<History>, snapshot: &WorldSnapshot, context: &OperationContext,
    input: &Value, publish: F) -> Result<String>
where F: FnOnce(Value) -> Result<String> {
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if context.anchor != snapshot.anchor() { return Err(failure(ErrorCode::StaleAnchor, "baseline query context has a different anchor")); }
    check_input(input)?;
    let envelope: Envelope = serde_json::from_value(input.clone())
        .map_err(|error| invalid(&format!("invalid baseline query: {error}")))?;
    if envelope.schema != "dfmcp.query/1" { return Err(invalid("baseline query schema must be dfmcp.query/1")); }
    if envelope.expected_anchor.as_ref().is_some_and(|anchor| anchor != &anchor_json(context.anchor)) {
        return Err(failure(ErrorCode::StaleAnchor, "expected_anchor does not match the complete current anchor"));
    }
    if snapshot.graph.entities.len() > context.budget.max_entities as usize {
        return Err(exhausted("baseline query exceeds the entity-scan budget"));
    }
    if !snapshot.hash_is_valid() { return Err(invariant("baseline query snapshot hash is invalid")); }
    match envelope.query {
        Request::Capture { key, select, max_game_ticks } => {
            if key.is_empty() || key.len() > 64 || !key.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte)) {
                return Err(invalid("capture key must be 1..64 ASCII letters, digits, dots, underscores, or hyphens"));
            }
            if max_game_ticks == 0 || max_game_ticks > context.budget.max_game_ticks {
                return Err(exhausted("capture lifetime must be positive and within the session game-tick budget"));
            }
            let expires_at = context.anchor.tick.0.checked_add(max_game_ticks)
                .ok_or_else(|| exhausted("capture deadline overflow"))?;
            validate_selection(&select)?;
            let (rows, bytes) = materialize(snapshot, context, &select)?;
            let digest = rows_digest(&rows)?;
            let mut store = lock(history)?;
            if let Some((_, existing)) = store.entries.iter().find(|((session, _), entry)|
                *session == context.session_id && entry.key == key) {
                if existing.anchor != context.anchor || existing.selection != select || existing.max_game_ticks != max_game_ticks {
                    return Err(failure(ErrorCode::Conflict, "capture key already names another baseline; list, use, or release it first"));
                }
                let mut payload = base_payload("capture", context.anchor);
                payload["captured"] = describe(existing, context.anchor);
                payload["reused"] = json!(true);
                return publish(payload);
            }
            let session_count = store.entries.keys().filter(|(session, _)| *session == context.session_id).count();
            if store.entries.len() >= MAX_BASELINES || session_count >= MAX_PER_SESSION {
                return Err(exhausted("query baseline capacity reached; explicitly release a retained baseline"));
            }
            let sequence = store.next_id.checked_add(1).ok_or_else(|| exhausted("query baseline ID space exhausted"))?;
            let id = format!("qb1:{}", Digest32::of_bytes(&encode(&json!({"domain":"dfmcp-query-baseline/1",
                "incarnation":*INCARNATION, "session":context.session_id.to_string(), "sequence":sequence, "key":key,
                "anchor":anchor_json(context.anchor), "selection":select, "result":digest.to_string()}))?));
            let baseline = Baseline { id: id.clone(), key, anchor:context.anchor, expires_at,
                max_game_ticks, selection:select, rows, digest, bytes };
            let mut payload = base_payload("capture", context.anchor);
            payload["captured"] = describe(&baseline, context.anchor);
            payload["reused"] = json!(false);
            let encoded = publish(payload)?;
            store.entries.insert((context.session_id, id), baseline);
            store.next_id = sequence;
            Ok(encoded)
        }
        Request::Changes { baseline, limit, continuation } => {
            validate_handle(&baseline)?;
            let baseline = lock(history)?.entries.get(&(context.session_id, baseline)).cloned()
                .ok_or_else(|| invalid("baseline is not retained by this session; list baselines or capture one"))?;
            publish(change_page(&baseline, snapshot, context, limit, continuation.as_deref())?)
        }
        Request::Baselines => {
            let baselines = lock(history)?.entries.iter().filter(|((session, _), _)| *session == context.session_id)
                .map(|(_, baseline)| describe(baseline, context.anchor)).collect::<Vec<_>>();
            let mut payload = base_payload("baselines", context.anchor);
            payload["baselines"] = json!(baselines);
            payload["maximum_per_session"] = json!(MAX_PER_SESSION);
            publish(payload)
        }
        Request::ReleaseBaseline { baseline } => {
            validate_handle(&baseline)?;
            let mut store = lock(history)?;
            let mut payload = base_payload("release_baseline", context.anchor);
            payload["baseline"] = json!(baseline);
            payload["released"] = json!(store.entries.contains_key(&(context.session_id, baseline.clone())));
            let encoded = publish(payload)?;
            store.entries.remove(&(context.session_id, baseline));
            Ok(encoded)
        }
    }
}
