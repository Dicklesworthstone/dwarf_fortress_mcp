//! Session-owned, foreground query baselines and condition watches over the
//! laboratory's current world.
//! The caller owns the session throughout execution and supplies a pure complete
//! Agent Turn publisher. No retained state changes before that publication fits.

use super::query_condition::Condition;
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
const MAX_WATCHES: usize = 8;
const MAX_WATCH_DESCRIPTOR_BYTES: usize = 4_096;

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
    Watch {
        key: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        condition: Condition,
        #[serde(skip_serializing_if = "Option::is_none")]
        failure_condition: Option<Condition>,
        deadline_tick: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        poll_interval_ticks: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        stable_observations: Option<u32>,
    },
    PollWatch {
        watch: String,
    },
    PollWatches {
        #[serde(skip_serializing_if = "Option::is_none")]
        watches: Option<Vec<String>>,
    },
    Watches {},
    CancelWatch {
        watch: String,
    },
    ReleaseWatch {
        watch: String,
    },
}

impl Request {
    pub(super) fn kind(&self) -> &'static str {
        match self.request.query {
            Query::Capture { .. } => "capture",
            Query::Changes { .. } => "changes",
            Query::Baselines {} => "baselines",
            Query::ReleaseBaseline { .. } => "release_baseline",
            Query::Watch { .. } => "watch",
            Query::PollWatch { .. } => "poll_watch",
            Query::PollWatches { .. } => "poll_watches",
            Query::Watches {} => "watches",
            Query::CancelWatch { .. } => "cancel_watch",
            Query::ReleaseWatch { .. } => "release_watch",
        }
    }

    fn is_watch(&self) -> bool {
        matches!(
            self.request.query,
            Query::Watch { .. }
                | Query::PollWatch { .. }
                | Query::PollWatches { .. }
                | Query::Watches {}
                | Query::CancelWatch { .. }
                | Query::ReleaseWatch { .. }
        )
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
/// Historical selectors and observation-refresh operations are absent. The
/// condition grammar is decoded directly, so duplicate nested fields cannot
/// disappear before the engine validates a stateful definition.
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

fn decode_catalog(value: &Value) -> Result<Catalog> {
    let records = value
        .as_array()
        .filter(|records| records.len() <= MAX_BASELINES)
        .ok_or_else(|| invariant("baseline engine returned an invalid catalog"))?;
    let mut result = Catalog::new();
    for record in records {
        let handle = record["baseline"]
            .as_str()
            .ok_or_else(|| invariant("baseline metadata has no handle"))?;
        if result.insert(handle.to_owned(), record.clone()).is_some() {
            return Err(invariant("baseline catalog repeats a retained handle"));
        }
    }
    Ok(result)
}

fn catalog(snapshot: &WorldSnapshot, current: &OperationContext) -> Result<Catalog> {
    let mut result = Catalog::new();
    super::semantic_query::execute_laboratory_baseline_with_publisher(
        snapshot,
        current,
        &json!({"schema":"dfmcp.query/1","query":{"kind":"baselines"}}),
        |value| {
            result = decode_catalog(&value["baselines"])?;
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

#[derive(Clone)]
struct WatchWork {
    records: Value,
    active: Value,
}

impl WatchWork {
    fn empty() -> Self {
        Self {
            records: json!([]),
            active: json!([]),
        }
    }

    fn from_catalog(records: &Value, active: &Value, current: &OperationContext) -> Result<Self> {
        fn descriptors(input: &Value, current: &OperationContext, active: bool) -> Result<Value> {
            let session = current.session_id;
            let records = input
                .as_array()
                .filter(|rows| rows.len() <= MAX_WATCHES)
                .ok_or_else(|| invariant("watch engine returned an invalid bounded catalog"))?;
            let mut seen = std::collections::BTreeSet::new();
            let mut output = Vec::with_capacity(records.len());
            for value in records {
                let handle = value["watch"]
                    .as_str()
                    .ok_or_else(|| invariant("watch descriptor has no handle"))?;
                if !seen.insert(handle) {
                    return Err(invariant("watch catalog repeats a retained handle"));
                }
                let terminal = value["terminal"]
                    .as_bool()
                    .ok_or_else(|| invariant("watch descriptor has no terminal state"))?;
                if active && terminal {
                    return Err(invariant("active watch catalog contains terminal work"));
                }
                let mut record = value.clone();
                record["kind"] = json!("foreground_observation_watch");
                record["game_effect"] = json!("none");
                record["durable"] = json!(false);
                record["mutation_authority"] = json!(false);
                record["next_step"] = next_step(
                    session,
                    json!({
                        "kind":if terminal { "release_watch" } else { "poll_watch" },
                        "watch":handle,
                    }),
                );
                if !terminal {
                    record["cancel_step"] =
                        next_step(session, json!({"kind":"cancel_watch", "watch":handle}));
                    if let Some(step) = watch_wait_step(current, &record) {
                        record["wait_step"] = step;
                    }
                }
                if record.to_string().len() > MAX_WATCH_DESCRIPTOR_BYTES {
                    return Err(invariant("watch descriptor exceeds its publication bound"));
                }
                output.push(record);
            }
            Ok(json!(output))
        }
        let retained = records
            .as_array()
            .ok_or_else(|| invariant("watch catalog is not an array"))?;
        let running = active
            .as_array()
            .ok_or_else(|| invariant("active watch catalog is not an array"))?;
        let expected: std::collections::BTreeSet<_> = retained
            .iter()
            .filter(|record| record["terminal"] == false)
            .map(|record| record["watch"].as_str())
            .collect();
        let actual: std::collections::BTreeSet<_> = running
            .iter()
            .map(|record| record["watch"].as_str())
            .collect();
        if expected != actual {
            return Err(invariant(
                "watch catalogs disagree about active retained work",
            ));
        }
        for record in running {
            let summary = retained
                .iter()
                .find(|summary| summary["watch"] == record["watch"])
                .ok_or_else(|| invariant("active watch is absent from retained catalog"))?;
            if summary.as_object().is_none_or(|fields| {
                fields
                    .iter()
                    .any(|(key, value)| record.get(key) != Some(value))
            }) {
                return Err(invariant(
                    "watch catalogs disagree about a retained summary",
                ));
            }
        }
        Ok(Self {
            records: descriptors(records, current, false)?,
            active: descriptors(active, current, true)?,
        })
    }
}

/// Offer bounded clock progression only while the complete proposed interval
/// has current Observe, Query and ControlClock authority. This is an affordance,
/// never an unpause request or a replacement for admission when it is invoked.
fn watch_wait_step(current: &OperationContext, record: &Value) -> Option<Value> {
    let mut ticks = current.budget.max_game_ticks.min(100);
    for key in ["next_sample_tick", "deadline_tick"] {
        if let Some(target) = record[key].as_u64().filter(|target| *target > current.anchor.tick.0) {
            ticks = ticks.min(target - current.anchor.tick.0);
        }
    }
    if ticks == 0 {
        return None;
    }
    let mut end = current.clone();
    end.anchor.tick = dfmcp_core::GameTick(current.anchor.tick.0.checked_add(ticks)?);
    for context in [current, &end] {
        context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None).ok()?;
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None).ok()?;
        context.authorize(Capability::ControlClock, RiskTier::Reversible, &[], None).ok()?;
    }
    Some(json!({"tool":"fortress.wait","arguments":{
        "session_id":current.session_id.to_string(),
        "watch_id":record["watch"],"max_game_ticks":ticks},
        "unpauses":false,"authority_rechecked_on_use":true}))
}

fn watch_catalog(current: &OperationContext) -> Result<WatchWork> {
    let catalog = super::semantic_query::laboratory_watch_work(current)?;
    WatchWork::from_catalog(
        &catalog["records"],
        &catalog["active_work"],
        current,
    )
}

/// Convert a candidate catalog without re-locking the retained watch store.
/// Foreground waits call this while their publisher owns the candidate root.
pub(super) fn candidate_watch_work(payload: &Value, current: &OperationContext) -> Result<Value> {
    let work = WatchWork::from_catalog(
        &payload["_query_watch_work"],
        &payload["_condition_watch_work"],
        current,
    )?;
    Ok(json!({"query_watches":work.records,"condition_watches":work.active}))
}

/// Reconstruct both retained and active watch work without sampling. Call while
/// owning the laboratory session and outside either temporal engine publisher.
pub(super) fn retained_watch_work(current: &OperationContext) -> Result<Value> {
    let started = Instant::now();
    let work = watch_catalog(&remaining(current, started)?)?;
    remaining(current, started)?;
    Ok(json!({"query_watches":work.records, "condition_watches":work.active}))
}

/// Read retained descriptors under current authority without acquiring rows or
/// rescanning the source. The caller supplies the owned current session anchor.
pub(super) fn retained_baseline_work(current: &OperationContext) -> Result<Value> {
    let started = Instant::now();
    let catalog = super::semantic_query::laboratory_baseline_work(&remaining(current, started)?)?;
    let result = work(&decode_catalog(&catalog)?, current.session_id)?;
    remaining(current, started)?;
    Ok(result)
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
    watch_work: WatchWork,
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
    payload["_query_watch_work"] = watch_work.records;
    payload["_condition_watch_work"] = watch_work.active;
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
        || actual["agent_turn"]["active_work"]["query_watches"] != expected["_query_watch_work"]
        || actual["agent_turn"]["active_work"]["condition_watches"]
            != expected["_condition_watch_work"]
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
/// runs before the selected temporal engine changes its root. It must not lock
/// the session or query either temporal engine recursively.
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
    // The watch engine supplies and bounds both candidate watch catalogs in its
    // own output. Baseline operations instead read that catalog once, outside
    // History's publisher, and reserve it without acquiring WATCHES recursively.
    let watches = if request.is_watch() {
        WatchWork::empty()
    } else {
        watch_catalog(&remaining(current, started)?)?
    };
    let reserve = decorate(
        json!({}),
        current,
        &source_work,
        baseline_work,
        watches.clone(),
    )
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
    let publication = |value: Value| {
        let candidate = if request.is_watch() {
            before.clone()
        } else {
            candidate_catalog(&before, &value)?
        };
        let watch_work = if request.is_watch() {
            WatchWork::from_catalog(
                &value["_query_watch_work"],
                &value["_condition_watch_work"],
                current,
            )?
        } else {
            watches
        };
        let payload = decorate(
            value,
            current,
            &source_work,
            work(&candidate, current.session_id)?,
            watch_work,
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
    };
    if request.is_watch() {
        super::semantic_query::execute_laboratory_watch_with_publisher(
            snapshot,
            &context,
            &input,
            publication,
        )
    } else {
        super::semantic_query::execute_laboratory_baseline_with_publisher(
            snapshot,
            &context,
            &input,
            publication,
        )
    }
}

#[cfg(test)]
#[path = "lab_temporal_query_tests.rs"]
mod tests;
