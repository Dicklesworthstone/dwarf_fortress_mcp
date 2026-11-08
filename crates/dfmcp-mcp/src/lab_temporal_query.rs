//! Session-owned, foreground query baselines over the laboratory's current world.
//! The caller owns the session throughout execution and supplies a pure complete
//! Agent Turn publisher. No retained state changes before that publication fits.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use dfmcp_core::{
    Capability, DfmcpError, ErrorCode, OperationContext, Result, RiskTier, SessionId,
};
use dfmcp_world::WorldSnapshot;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const MAX_REQUEST_BYTES: usize = 65_536;
const MAX_BASELINES: usize = 8;
const MAX_BASELINE_DESCRIPTOR_BYTES: usize = 2_048;

#[derive(Deserialize)]
struct Selector {
    mode: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Temporal,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    mode: Mode,
    request: Envelope,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    schema: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_anchor: Option<Value>,
    query: Query,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum SelectionKind {
    Entities,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    kind: SelectionKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    kinds: Vec<String>,
    #[serde(rename = "where", skip_serializing_if = "Option::is_none")]
    predicate: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    fields: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    order: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Query {
    Capture {
        key: String,
        select: Selection,
        max_game_ticks: u64,
    },
    Changes {
        baseline: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        limit: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        continuation: Option<String>,
    },
    Baselines {},
    ReleaseBaseline {
        baseline: String,
    },
}

impl Request {
    pub(super) fn kind(&self) -> &'static str {
        match self.request.query {
            Query::Capture { .. } => "capture",
            Query::Changes { .. } => "changes",
            Query::Baselines {} => "baselines",
            Query::ReleaseBaseline { .. } => "release_baseline",
        }
    }
}

fn invalid(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn exhausted(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

fn invariant(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InternalInvariantViolation, message)
}

/// Decode the original bytes at the wrapper, envelope and operation boundaries;
/// a Value round trip must not conceal duplicate fields on a mutating request.
/// Historical selectors and all watch/refresh operations are deliberately absent.
pub(super) fn parse(raw: &str) -> Result<Option<Request>> {
    if raw.len() > MAX_REQUEST_BYTES {
        return Err(exhausted(
            "laboratory temporal query exceeds its request byte bound",
        ));
    }
    if !raw.trim_start().starts_with('{') {
        return Ok(None);
    }
    let selector: Selector =
        serde_json::from_str(raw).map_err(|_| invalid("query requires one unambiguous mode"))?;
    if selector.mode != "temporal" {
        return Ok(None);
    }
    let request: Request = serde_json::from_str(raw)
        .map_err(|error| invalid(format!("invalid laboratory temporal query: {error}")))?;
    let Mode::Temporal = request.mode;
    if request.request.schema != "dfmcp.query/1" {
        return Err(invalid("temporal request schema must be dfmcp.query/1"));
    }
    Ok(Some(request))
}

/// One guard belongs to each laboratory session. It neither owns a game source
/// nor persists monitoring intent. Dropping the session frees only that session's
/// private query allocations, using the established poison-tolerant teardown.
pub(super) struct SessionGuard {
    session: SessionId,
}

impl SessionGuard {
    pub(super) const fn new(session: SessionId) -> Self {
        Self { session }
    }
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        let _ = super::semantic_query::release_session_resources(self.session, true, |_| {
            Ok(String::new())
        });
    }
}

fn remaining(current: &OperationContext, started: Instant) -> Result<OperationContext> {
    current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    let elapsed = started.elapsed();
    if elapsed >= Duration::from_millis(current.budget.max_wall_millis) {
        return Err(exhausted(
            "temporal query exhausted its shared wall-time budget",
        ));
    }
    let mut context = current.clone();
    context.budget.max_wall_millis -= elapsed.as_millis() as u64;
    Ok(context)
}

type Catalog = BTreeMap<String, Value>;

fn catalog(snapshot: &WorldSnapshot, current: &OperationContext) -> Result<Catalog> {
    let mut result = Catalog::new();
    super::semantic_query::execute_laboratory_baseline_with_publisher(
        snapshot,
        current,
        &json!({"schema":"dfmcp.query/1","query":{"kind":"baselines"}}),
        |value| {
            let records = value["baselines"]
                .as_array()
                .filter(|records| records.len() <= MAX_BASELINES)
                .ok_or_else(|| invariant("baseline engine returned an invalid catalog"))?;
            for record in records {
                let handle = record["baseline"]
                    .as_str()
                    .ok_or_else(|| invariant("baseline metadata has no handle"))?;
                if result.insert(handle.to_owned(), record.clone()).is_some() {
                    return Err(invariant("baseline catalog repeats a retained handle"));
                }
            }
            Ok(String::new())
        },
    )?;
    Ok(result)
}

fn next_step(session: SessionId, query: Value) -> Value {
    json!({"tool":"fortress.query","arguments":{
        "session_id":session.to_string(),
        "mode":json!({"mode":"temporal", "request":{
            "schema":"dfmcp.query/1", "query":query
        }}).to_string()
    }})
}

fn work(catalog: &Catalog, session: SessionId) -> Result<Value> {
    if catalog.len() > MAX_BASELINES {
        return Err(invariant("baseline catalog exceeds its session bound"));
    }
    let mut records = Vec::with_capacity(catalog.len());
    for (handle, value) in catalog {
        let mut record = value.clone();
        record["kind"] = json!("query_baseline");
        record["game_effect"] = json!("none");
        record["durable"] = json!(false);
        record["mutation_authority"] = json!(false);
        let release = next_step(
            session,
            json!({"kind":"release_baseline","baseline":handle}),
        );
        record["next_step"] = if record["status"] == "retained" {
            next_step(session, json!({"kind":"changes","baseline":handle}))
        } else {
            release.clone()
        };
        record["release_step"] = release;
        if record.to_string().len() > MAX_BASELINE_DESCRIPTOR_BYTES {
            return Err(invariant(
                "baseline descriptor exceeds its publication reserve",
            ));
        }
        records.push(record);
    }
    Ok(json!(records))
}

/// Reconstruct retained query work from the session's own store, never from a
/// presentation cache. This does not sample, advance, release or renew a baseline.
pub(super) fn retained_work(snapshot: &WorldSnapshot, current: &OperationContext) -> Result<Value> {
    let started = Instant::now();
    super::query_budget::admit_sources(&[snapshot], current)?;
    let records = catalog(snapshot, &remaining(current, started)?)?;
    let result = work(&records, current.session_id)?;
    remaining(current, started)?;
    Ok(result)
}

fn candidate_catalog(before: &Catalog, payload: &Value) -> Result<Catalog> {
    let mut candidate = before.clone();
    match payload["kind"].as_str() {
        Some("capture") => {
            let captured = payload["captured"].clone();
            let handle = captured["baseline"]
                .as_str()
                .ok_or_else(|| invariant("capture publication has no baseline handle"))?;
            candidate.insert(handle.to_owned(), captured);
        }
        Some("release_baseline") => {
            let handle = payload["baseline"]
                .as_str()
                .ok_or_else(|| invariant("release publication has no baseline handle"))?;
            candidate.remove(handle);
        }
        Some("changes" | "baselines") => {}
        _ => return Err(invariant("unexpected laboratory temporal query result")),
    }
    Ok(candidate)
}

fn decorate(
    mut payload: Value,
    current: &OperationContext,
    source_work: &Value,
    baseline_work: Value,
) -> Value {
    payload["ok"] = json!(true);
    payload["mode"] = json!("temporal");
    payload["session_id"] = json!(current.session_id.to_string());
    payload["game_tick"] = json!(current.anchor.tick.get());
    payload["current_anchor"] = super::anchor_json(&current.anchor);
    payload["evidence_scope"] = json!("laboratory_reference_world");
    payload["source_work"] = source_work.clone();
    payload["execution"] = json!("foreground_only");
    payload["native_captures"] = json!(0);
    payload["advances_game"] = json!(false);
    payload["durable"] = json!(false);
    payload["mutation_authority"] = json!(false);
    payload["_query_baseline_work"] = baseline_work;
    payload
}

/// A publisher may add presentation metadata, but must return the accepted full
/// success unchanged. In particular an Ok budget-refusal, silently clipped page,
/// omitted work catalog or a mismatched anchor cannot commit a capture/release.
fn check_publication(encoded: &str, expected: &Value, current: &OperationContext) -> Result<()> {
    let maximum = current
        .budget
        .max_bytes
        .min(u64::from(current.budget.max_output_tokens).saturating_mul(4));
    if encoded.len() as u64 > maximum {
        return Err(exhausted(
            "complete temporal query and Agent Turn exceed the response budget",
        ));
    }
    let actual: Value = serde_json::from_str(encoded)
        .map_err(|_| invariant("temporal publisher did not return a JSON Agent Turn"))?;
    if actual["ok"] != true
        || actual["agent_turn"]["session_id"] != current.session_id.to_string()
        || actual["agent_turn"]["request_id"] != current.request_id.to_string()
        || actual["agent_turn"]["anchor"] != super::anchor_json(&current.anchor)
        || actual["agent_turn"]["active_work"]["query_baselines"]
            != expected["_query_baseline_work"]
    {
        return Err(invariant(
            "temporal publisher refused or changed required session state",
        ));
    }
    for (key, value) in expected
        .as_object()
        .ok_or_else(|| invariant("temporal result must be an object"))?
    {
        if !key.starts_with('_') && actual.get(key) != Some(value) {
            return Err(invariant(
                "temporal publisher changed a complete result field",
            ));
        }
    }
    Ok(())
}

/// `result_bytes` is the remaining allowance after the caller has reserved its
/// complete laboratory Agent Turn, including pending game work. The publisher
/// runs before the existing baseline engine changes its root. It must not lock
/// the session or query this engine recursively.
pub(super) fn execute<F>(
    snapshot: &WorldSnapshot,
    current: &OperationContext,
    request: &Request,
    result_bytes: usize,
    publish: F,
) -> Result<String>
where
    F: FnOnce(Value) -> Result<String>,
{
    let started = Instant::now();
    let source_work = super::query_budget::admit_sources(&[snapshot], current)?;
    let before = catalog(snapshot, &remaining(current, started)?)?;
    let baseline_work = work(&before, current.session_id)?;
    let reserve = decorate(json!({}), current, &source_work, baseline_work)
        .to_string()
        .len()
        .saturating_add(if request.kind() == "capture" {
            MAX_BASELINE_DESCRIPTOR_BYTES
        } else {
            0
        });
    let mut context = remaining(current, started)?;
    // Acquisition reuses the semantic query engine, which rechecks the complete
    // canonical source on each bounded page. Reserve output separately so an
    // admitted large world can still yield a small, complete selected baseline.
    let result_tokens = result_bytes
        .checked_sub(reserve)
        .map(|bytes| bytes / 4)
        .filter(|tokens| *tokens > 0)
        .ok_or_else(|| {
            exhausted("retained query work leaves no room for a complete temporal result")
        })?;
    context.budget.max_output_tokens = context.budget.max_output_tokens.min(
        u32::try_from(result_tokens)
            .map_err(|_| exhausted("temporal result budget cannot be represented"))?,
    );
    let input = serde_json::to_value(&request.request)
        .map_err(|_| invariant("typed temporal request cannot be encoded"))?;
    super::semantic_query::execute_laboratory_baseline_with_publisher(
        snapshot,
        &context,
        &input,
        |value| {
            let candidate = candidate_catalog(&before, &value)?;
            let payload = decorate(
                value,
                current,
                &source_work,
                work(&candidate, current.session_id)?,
            );
            if payload.to_string().len() > result_bytes {
                return Err(exhausted(
                    "complete temporal result exceeds its reserved byte budget",
                ));
            }
            let encoded = publish(payload.clone())?;
            check_publication(&encoded, &payload, current)?;
            // No fallible work after the engine commits its candidate root. The
            // deadline includes source admission, acquisition and full rendering.
            remaining(current, started)?;
            Ok(encoded)
        },
    )
}

#[cfg(test)]
#[path = "lab_temporal_query_tests.rs"]
mod tests;
