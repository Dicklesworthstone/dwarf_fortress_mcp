//! Foreground, session-owned predicates over published observations.
//! No timers, bridge calls, game effects, or detached work live here. A caller
//! explicitly refreshes observations and polls a watch. Publication occurs only
//! after the complete response, including active work, has been rendered.

use std::collections::BTreeMap;
use std::sync::{LazyLock, Mutex, MutexGuard};

use dfmcp_core::{Capability, DfmcpError, Digest32, EntityId, ErrorCode, OperationContext,
    Result, RiskTier, SessionId, StateAnchor};
use dfmcp_world::{FactPresence, FactSource, Value as WorldValue, WorldSnapshot};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const MAX_PER_SESSION: usize = 8;
const MAX_TOTAL: usize = 128;
const MAX_INPUT_BYTES: usize = 32_768;
const MAX_INPUT_NODES: usize = 1_024;
const MAX_CONDITIONS: usize = 64;
const MAX_CONDITION_DEPTH: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case", deny_unknown_fields)]
enum Literal {
    Null,
    Bool(bool),
    I64(i64),
    U64(u64),
    Text(String),
    Fixed { units: i64, scale: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Comparison { Eq, Ne, Lt, Le, Gt, Ge }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Condition {
    Field { entity_id: String, generation: u32, field: String, comparison: Comparison, value: Literal },
    Paused { value: bool },
    TickAtLeast { value: u64 },
    All { args: Vec<Condition> },
    Any { args: Vec<Condition> },
    Not { arg: Box<Condition> },
}

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
    Watch {
        key: String,
        label: Option<String>,
        condition: Condition,
        failure_condition: Option<Condition>,
        deadline_tick: u64,
        poll_interval_ticks: Option<u64>,
        stable_observations: Option<u32>,
    },
    PollWatch { watch: String },
    Watches,
    CancelWatch { watch: String },
    ReleaseWatch { watch: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Definition {
    key: String,
    label: String,
    condition: Condition,
    failure_condition: Option<Condition>,
    deadline_tick: u64,
    poll_interval_ticks: u64,
    stable_observations: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Truth { True, False, Unknown }

impl Truth {
    fn from_bool(value: bool) -> Self { if value { Self::True } else { Self::False } }
    fn text(self) -> &'static str {
        match self { Self::True => "true", Self::False => "false", Self::Unknown => "unknown" }
    }
    fn not(self) -> Self {
        match self { Self::True => Self::False, Self::False => Self::True, Self::Unknown => Self::Unknown }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status { Waiting, Candidate, BlockedUnknown, Satisfied, Failed, Expired, Invalidated, Cancelled }

impl Status {
    fn text(self) -> &'static str {
        match self {
            Self::Waiting => "waiting", Self::Candidate => "candidate",
            Self::BlockedUnknown => "blocked_unknown", Self::Satisfied => "satisfied",
            Self::Failed => "failed", Self::Expired => "expired",
            Self::Invalidated => "invalidated", Self::Cancelled => "cancelled",
        }
    }
    fn terminal(self) -> bool {
        matches!(self, Self::Satisfied | Self::Failed | Self::Expired | Self::Invalidated | Self::Cancelled)
    }
}

#[derive(Clone)]
struct Watch {
    handle: String,
    definition: Definition,
    created_at: StateAnchor,
    last_seen: StateAnchor,
    last_sample_tick: Option<u64>,
    streak: u32,
    samples: u64,
    status: Status,
    evaluation: Value,
    evidence_digest: Digest32,
}

#[derive(Default)]
struct Store {
    serial: u64,
    entries: BTreeMap<(SessionId, String), Watch>,
}

static WATCHES: LazyLock<Mutex<Store>> = LazyLock::new(|| Mutex::new(Store::default()));

fn failure(code: ErrorCode, message: impl Into<String>) -> DfmcpError { DfmcpError::new(code, message) }
fn invalid(message: &str) -> DfmcpError { failure(ErrorCode::InvalidRequest, message) }
fn bounded(message: &str) -> DfmcpError { failure(ErrorCode::BudgetExceeded, message) }
fn lock(store: &Mutex<Store>) -> Result<MutexGuard<'_, Store>> {
    store.lock().map_err(|_| failure(ErrorCode::InternalInvariantViolation, "condition-watch store is poisoned"))
}
fn anchor(value: StateAnchor) -> Value { super::core_query::anchor_json(value) }
fn digest(value: &Value) -> Result<Digest32> {
    serde_json::to_vec(value).map(|bytes| Digest32::of_bytes(&bytes))
        .map_err(|_| failure(ErrorCode::InternalInvariantViolation, "watch evidence cannot be encoded"))
}
fn positive_id(value: &str) -> Result<EntityId> {
    if value.is_empty() || value.len() > 20 || value.starts_with('0')
        || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid("watch entity IDs must be canonical positive decimal u64 strings"));
    }
    value.parse::<u64>().map(EntityId::new).map_err(|_| invalid("watch entity ID exceeds u64"))
}
fn name(value: &str, maximum: usize) -> Result<()> {
    if value.is_empty() || value.len() > maximum || value.contains('\0') {
        return Err(invalid("watch key, label, or field name violates its byte bound"));
    }
    Ok(())
}
fn validate_handle(value: &str) -> Result<()> {
    let Some(hash) = value.strip_prefix("watch:") else { return Err(invalid("invalid watch handle")); };
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
        return Err(invalid("invalid watch handle"));
    }
    Ok(())
}

fn validate_input(input: &Value) -> Result<()> {
    let mut pending = vec![(input, 0usize)];
    let mut nodes = 0usize;
    let mut bytes = 0usize;
    while let Some((value, depth)) = pending.pop() {
        nodes += 1;
        bytes = bytes.saturating_add(32);
        if depth > 24 || nodes > MAX_INPUT_NODES { return Err(bounded("watch input exceeds depth/node bounds")); }
        match value {
            Value::String(text) => bytes = bytes.saturating_add(text.len()),
            Value::Array(values) => {
                if values.len().saturating_add(pending.len()).saturating_add(nodes) > MAX_INPUT_NODES {
                    return Err(bounded("watch input exceeds its node bound"));
                }
                pending.extend(values.iter().map(|value| (value, depth + 1)));
            }
            Value::Object(values) => {
                if values.len().saturating_add(pending.len()).saturating_add(nodes) > MAX_INPUT_NODES {
                    return Err(bounded("watch input exceeds its node bound"));
                }
                for (key, value) in values {
                    bytes = bytes.saturating_add(key.len());
                    pending.push((value, depth + 1));
                }
            }
            _ => {}
        }
        if bytes > MAX_INPUT_BYTES { return Err(bounded("watch input exceeds its aggregate byte bound")); }
    }
    Ok(())
}

fn validate_definition(definition: &Definition) -> Result<()> {
    name(&definition.key, 64)?;
    name(&definition.label, 256)?;
    if !(1..=64).contains(&definition.stable_observations)
        || !(1..=1_000_000).contains(&definition.poll_interval_ticks) {
        return Err(invalid("watch stability must be 1..64 and cadence 1..1000000 game ticks"));
    }
    let mut pending = vec![(&definition.condition, 1usize)];
    if let Some(condition) = &definition.failure_condition { pending.push((condition, 1)); }
    let mut nodes = 0;
    while let Some((condition, depth)) = pending.pop() {
        nodes += 1;
        if nodes > MAX_CONDITIONS || depth > MAX_CONDITION_DEPTH {
            return Err(bounded("watch conditions exceed their shared depth/node budget"));
        }
        match condition {
            Condition::Field { entity_id, generation, field, value, .. } => {
                positive_id(entity_id)?;
                if *generation == 0 { return Err(invalid("watch entity generation must be positive")); }
                name(field, 128)?;
                if let Literal::Text(text) = value
                    && (text.len() > 1_024 || text.contains('\0')) {
                    return Err(invalid("watch text literal exceeds its byte bound or contains NUL"));
                }
            }
            Condition::All { args } | Condition::Any { args } => {
                if args.is_empty() || args.len().saturating_add(nodes).saturating_add(pending.len()) > MAX_CONDITIONS {
                    return Err(bounded("watch boolean groups must be nonempty and bounded"));
                }
                pending.extend(args.iter().map(|child| (child, depth + 1)));
            }
            Condition::Not { arg } => pending.push((arg, depth + 1)),
            Condition::Paused { .. } | Condition::TickAtLeast { .. } => {}
        }
    }
    Ok(())
}

fn compare(actual: &WorldValue, operation: Comparison, expected: &Literal) -> Truth {
    use std::cmp::Ordering;
    let order = match (actual, expected) {
        (WorldValue::Null, Literal::Null) => Some(Ordering::Equal),
        (WorldValue::Bool(left), Literal::Bool(right)) => Some(left.cmp(right)),
        (WorldValue::I64(left), Literal::I64(right)) => Some(left.cmp(right)),
        (WorldValue::U64(left), Literal::U64(right)) => Some(left.cmp(right)),
        (WorldValue::Text(left), Literal::Text(right)) => Some(left.cmp(right)),
        (WorldValue::Fixed { units: left, scale: ls }, Literal::Fixed { units: right, scale: rs }) if ls == rs => Some(left.cmp(right)),
        _ => None,
    };
    let Some(order) = order else { return Truth::Unknown; };
    Truth::from_bool(match operation {
        Comparison::Eq => order.is_eq(), Comparison::Ne => !order.is_eq(),
        Comparison::Lt => order.is_lt(), Comparison::Le => !order.is_gt(),
        Comparison::Gt => order.is_gt(), Comparison::Ge => !order.is_lt(),
    })
}

#[derive(Default)]
struct Probe {
    facts: Vec<Value>,
    invalid_generation: bool,
}

impl Probe {
    fn evaluate(&mut self, condition: &Condition, snapshot: &WorldSnapshot) -> Result<Truth> {
        match condition {
            Condition::All { args } | Condition::Any { args } => {
                let all = matches!(condition, Condition::All { .. });
                let mut decisive = false;
                let mut unknown = false;
                // Visit every bounded leaf, including after a boolean decision,
                // so a recycled identity cannot hide behind short-circuiting.
                for child in args {
                    match self.evaluate(child, snapshot)? {
                        Truth::False if all => decisive = true,
                        Truth::True if !all => decisive = true,
                        Truth::Unknown => unknown = true,
                        _ => {}
                    }
                }
                Ok(if decisive { Truth::from_bool(!all) }
                    else if unknown { Truth::Unknown } else { Truth::from_bool(all) })
            }
            Condition::Not { arg } => Ok(self.evaluate(arg, snapshot)?.not()),
            Condition::Paused { value } => {
                let truth = Truth::from_bool(snapshot.paused == *value);
                self.facts.push(json!({"field":"canonical.paused","truth":truth.text(),
                    "source_digest":snapshot.state_hash.to_string()}));
                Ok(truth)
            }
            Condition::TickAtLeast { value } => {
                let truth = Truth::from_bool(snapshot.tick.0 >= *value);
                self.facts.push(json!({"field":"canonical.game_tick","truth":truth.text(),
                    "source_digest":snapshot.state_hash.to_string()}));
                Ok(truth)
            }
            Condition::Field { entity_id, generation, field, comparison, value } => {
                let id = positive_id(entity_id)?;
                let entity = snapshot.graph.entities.get(&id);
                let fact = entity.and_then(|entity| entity.fields.get(field));
                let mut reason = None;
                let truth = match entity {
                    None => { reason = Some("entity_not_observed"); Truth::Unknown }
                    Some(entity) if entity.generation != *generation => {
                        self.invalid_generation = true;
                        reason = Some("entity_generation_changed");
                        Truth::Unknown
                    }
                    Some(_) => match fact {
                        None => { reason = Some("field_not_observed"); Truth::Unknown }
                        Some(fact) => {
                            let known = match &fact.presence {
                                None => true,
                                Some(FactPresence::Known(value)) => value == &fact.value,
                                _ => false,
                            };
                            if !known { reason = Some("field_not_consistently_known"); Truth::Unknown }
                            else if !matches!(&fact.source, FactSource::DfhackField(_)) {
                                reason = Some("source_is_not_an_observed_dfhack_field"); Truth::Unknown
                            } else if fact.source_digest == Digest32::ZERO {
                                reason = Some("source_digest_not_established"); Truth::Unknown
                            } else if fact.observed_at != snapshot.tick {
                                reason = Some("field_is_not_observed_at_this_game_tick"); Truth::Unknown
                            } else {
                                let truth = compare(&fact.value, *comparison, value);
                                if truth == Truth::Unknown { reason = Some("incompatible_value_type_or_scale"); }
                                truth
                            }
                        }
                    },
                };
                self.facts.push(json!({"entity_id":entity_id,"generation":generation,
                    "field":field,"truth":truth.text(),"reason":reason,
                    "revision":entity.map(|entity|entity.revision),
                    "source_digest":fact.map(|fact|fact.source_digest.to_string()),
                    "observed_at_game_tick":fact.map(|fact|fact.observed_at.0)}));
                Ok(truth)
            }
        }
    }
}

impl Watch {
    fn seal(&mut self) -> Result<()> {
        self.evidence_digest = digest(&json!({"domain":"dfmcp-condition-watch-evidence-v1",
            "watch":self.handle,"prior_digest":self.evidence_digest.to_string(),
            "anchor":anchor(self.last_seen),"status":self.status.text(),
            "streak":self.streak,"samples":self.samples,"evaluation":self.evaluation}))?;
        Ok(())
    }

    fn advance(&mut self, snapshot: &WorldSnapshot, initial: bool) -> Result<()> {
        if self.status.terminal() { return Ok(()); }
        let current = snapshot.anchor();
        let previous = self.last_seen;
        let incompatible = current.fortress_id != self.created_at.fortress_id
            || current.cursor.epoch != self.created_at.cursor.epoch
            || current.tick < previous.tick || current.cursor.sequence < previous.cursor.sequence
            || (current.cursor == previous.cursor && current != previous);
        if incompatible {
            self.status = Status::Invalidated;
            self.streak = 0;
            self.last_seen = current;
            self.evaluation = json!({"reason":"fortress_epoch_clock_or_cursor_identity_changed",
                "prior_anchor":anchor(previous)});
            return self.seal();
        }
        if !initial && current == previous { return Ok(()); }
        self.last_seen = current;
        if current.tick.0 > self.definition.deadline_tick {
            self.status = Status::Expired;
            self.streak = 0;
            self.evaluation = json!({"reason":"game_tick_deadline_passed"});
            return self.seal();
        }
        let gap = !initial && current.cursor.sequence > previous.cursor.sequence.saturating_add(1);
        if gap { self.streak = 0; }
        let mut probe = Probe::default();
        let truth = probe.evaluate(&self.definition.condition, snapshot)?;
        let failure_truth = match &self.definition.failure_condition {
            Some(condition) => probe.evaluate(condition, snapshot)?,
            None => Truth::False,
        };
        let due = self.last_sample_tick.is_none_or(|last| {
            current.tick.0.checked_sub(last).is_some_and(|elapsed| elapsed >= self.definition.poll_interval_ticks)
        });
        self.evaluation = json!({"condition":truth.text(),"failure_condition":failure_truth.text(),
            "sample_due":due,"skipped_observations_reset_streak":gap,
            "continuous_between_observations":false,"facts":probe.facts});
        if probe.invalid_generation {
            self.status = Status::Invalidated;
            self.streak = 0;
        } else if failure_truth == Truth::True {
            self.status = Status::Failed;
            self.streak = 0;
        } else {
            if due {
                self.last_sample_tick = Some(current.tick.0);
                self.samples = self.samples.checked_add(1)
                    .ok_or_else(|| bounded("watch sample counter exhausted"))?;
            }
            if truth == Truth::Unknown || failure_truth == Truth::Unknown {
                self.status = Status::BlockedUnknown;
                self.streak = 0;
            } else if truth == Truth::False {
                self.status = Status::Waiting;
                self.streak = 0;
            } else {
                if due { self.streak += 1; }
                self.status = if self.streak >= self.definition.stable_observations {
                    Status::Satisfied
                } else { Status::Candidate };
            }
            if !self.status.terminal() && current.tick.0 == self.definition.deadline_tick {
                self.status = Status::Expired;
                self.evaluation["reason"] = json!("deadline_reached_without_stable_completion");
            }
        }
        self.seal()
    }

    fn summary(&self, current: StateAnchor) -> Value {
        json!({"watch":self.handle,"key":self.definition.key,"label":self.definition.label,
            "status":self.status.text(),"terminal":self.status.terminal(),
            "stable_observations":self.streak,"required_stable_observations":self.definition.stable_observations,
            "deadline_tick":self.definition.deadline_tick,
            "last_evaluated_anchor":anchor(self.last_seen),
            "evaluation_current":self.last_seen==current,
            "needs_poll":!self.status.terminal() && self.last_seen!=current,
            "next_sample_tick":self.last_sample_tick.and_then(|tick|tick.checked_add(self.definition.poll_interval_ticks)),
            "evidence_digest":self.evidence_digest.to_string()})
    }

    fn detail(&self, current: StateAnchor) -> Value {
        let mut result = self.summary(current);
        result["definition"] = json!(self.definition);
        result["created_at"] = anchor(self.created_at);
        result["sample_count"] = json!(self.samples);
        result["evaluation"] = self.evaluation.clone();
        result
    }
}

fn active_work(store: &Store, context: &OperationContext) -> Vec<Value> {
    store.entries.iter().filter(|((session, _), watch)| *session == context.session_id && !watch.status.terminal())
        .map(|(_, watch)| {
            let mut value = watch.summary(context.anchor);
            value["obligation_id"] = json!(watch.handle);
            value["kind"] = json!("foreground_observation_watch");
            value["game_effect"] = json!("none");
            value["next_step"] = json!({"tool":"fortress.query","arguments":{
                "session_id":context.session_id.to_string(),"query":{"schema":"dfmcp.query/1",
                    "query":{"kind":"poll_watch","watch":watch.handle}}}});
            value
        }).collect()
}

fn payload(context: &OperationContext, kind: &str) -> Value {
    json!({"schema":"dfmcp.query.result/1","kind":kind,"anchor":anchor(context.anchor),
        "truncated":false,"continuation":null,"coverage":{"domain":"explicit_observed_predicates",
            "absence_proven":false,"continuous_between_observations":false},
        "execution":"foreground_only","advances_game":false,"durable":false})
}

fn authorize(snapshot: &WorldSnapshot, context: &OperationContext) -> Result<()> {
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if snapshot.anchor() != context.anchor {
        return Err(failure(ErrorCode::StaleAnchor,"watch context does not name the supplied snapshot"));
    }
    if snapshot.graph.entities.len() > context.budget.max_entities as usize {
        return Err(bounded("watch input exceeds the session's entity budget"));
    }
    if !snapshot.hash_is_valid() {
        return Err(failure(ErrorCode::InternalInvariantViolation,"watch snapshot hash is invalid"));
    }
    Ok(())
}

fn publish_work<F>(store: &Store, context: &OperationContext, mut value: Value, publish: F) -> Result<String>
where F: FnOnce(Value) -> Result<String> {
    value["_condition_watch_work"] = json!(active_work(store, context));
    let size = serde_json::to_vec(&value).map_err(|_| invalid("watch result cannot be encoded"))?.len();
    let maximum = context.budget.max_bytes.min(u64::from(context.budget.max_output_tokens).saturating_mul(4));
    if size as u64 > maximum { return Err(bounded("watch result and active-work summary exceed the output budget")); }
    publish(value)
}

/// Pure queries retain the current watch projection without sampling it.
pub fn with_active_work<F>(context: &OperationContext, value: Value, publish: F) -> Result<String>
where F: FnOnce(Value) -> Result<String> {
    let store = lock(&WATCHES)?;
    publish_work(&store, context, value, publish)
}

pub fn execute<F>(snapshot: &WorldSnapshot, context: &OperationContext, input: &Value, publish: F) -> Result<String>
where F: FnOnce(Value) -> Result<String> {
    execute_in(&WATCHES, snapshot, context, input, publish)
}

fn execute_in<F>(storage: &Mutex<Store>, snapshot: &WorldSnapshot, context: &OperationContext,
    input: &Value, publish: F) -> Result<String>
where F: FnOnce(Value) -> Result<String> {
    authorize(snapshot, context)?;
    validate_input(input)?;
    if input["query"]["kind"] == "watches"
        && input["query"].as_object().is_none_or(|query| query.len() != 1) {
        return Err(invalid("watches does not accept additional arguments"));
    }
    let envelope: Envelope = serde_json::from_value(input.clone())
        .map_err(|_| invalid("invalid condition-watch request"))?;
    if envelope.schema != "dfmcp.query/1" { return Err(invalid("watch schema must be dfmcp.query/1")); }
    if envelope.expected_anchor.as_ref().is_some_and(|value| value != &anchor(context.anchor)) {
        return Err(failure(ErrorCode::StaleAnchor,"watch expected_anchor differs from the complete current anchor"));
    }
    let mut store = lock(storage)?;
    match envelope.query {
        Request::Watch { key, label, condition, failure_condition, deadline_tick, poll_interval_ticks, stable_observations } => {
            let definition = Definition { label:label.unwrap_or_else(||key.clone()),key,condition,failure_condition,
                deadline_tick,poll_interval_ticks:poll_interval_ticks.unwrap_or(1),stable_observations:stable_observations.unwrap_or(2) };
            validate_definition(&definition)?;
            if let Some((_, prior)) = store.entries.iter().find(|((session, _), watch)| {
                *session==context.session_id && watch.definition.key==definition.key
            }) {
                if prior.definition != definition { return Err(failure(ErrorCode::Conflict,"watch key already identifies another definition")); }
                let mut value = payload(context,"watch");
                value["record"] = prior.detail(context.anchor);
                value["replayed"] = json!(true);
                return publish_work(&store,context,value,publish);
            }
            if deadline_tick <= context.anchor.tick.0
                || deadline_tick.checked_sub(context.anchor.tick.0).is_none_or(|ticks|ticks>context.budget.max_game_ticks) {
                return Err(invalid("new watch deadline must be future and within the negotiated game-tick horizon"));
            }
            let count = store.entries.keys().filter(|(session,_)|*session==context.session_id).count();
            if count>=MAX_PER_SESSION || store.entries.len()>=MAX_TOTAL {
                return Err(bounded("watch retention is full; release a terminal watch before registering more"));
            }
            let serial = store.serial.checked_add(1).ok_or_else(||bounded("watch identity space exhausted"))?;
            let identity = digest(&json!({"domain":"dfmcp-condition-watch-v1","session":context.session_id.to_string(),
                "serial":serial,"anchor":anchor(context.anchor),"definition":definition}))?;
            let mut watch = Watch {handle:format!("watch:{identity}"),definition,created_at:context.anchor,
                last_seen:context.anchor,last_sample_tick:None,streak:0,samples:0,status:Status::Waiting,
                evaluation:Value::Null,evidence_digest:identity};
            watch.advance(snapshot,true)?;
            let key = (context.session_id,watch.handle.clone());
            let mut value = payload(context,"watch");
            value["record"] = watch.detail(context.anchor);
            value["replayed"] = json!(false);
            // Build a bounded candidate store. The caller's pure publisher must
            // accept the full response before the authoritative root is swapped.
            let mut candidate = Store {serial,entries:store.entries.clone()};
            candidate.entries.insert(key,watch);
            let encoded = publish_work(&candidate,context,value,publish)?;
            *store = candidate;
            Ok(encoded)
        }
        Request::Watches => {
            let mut value = payload(context,"watches");
            value["records"] = json!(store.entries.iter().filter(|((session,_) ,_)|*session==context.session_id)
                .map(|(_,watch)|watch.summary(context.anchor)).collect::<Vec<_>>());
            value["maximum_per_session"] = json!(MAX_PER_SESSION);
            publish_work(&store,context,value,publish)
        }
        Request::PollWatch { watch } | Request::CancelWatch { watch } | Request::ReleaseWatch { watch } => {
            validate_handle(&watch)?;
            let key = (context.session_id,watch.clone());
            let kind = input["query"]["kind"].as_str().ok_or_else(||invalid("watch operation missing"))?;
            let mut candidate = Store {serial:store.serial,entries:store.entries.clone()};
            let mut value = payload(context,kind);
            if kind=="release_watch" {
                if candidate.entries.get(&key).is_some_and(|watch|!watch.status.terminal()) {
                    return Err(failure(ErrorCode::Conflict,"cancel a nonterminal watch before releasing it"));
                }
                value["released"] = json!(candidate.entries.remove(&key).is_some());
                value["watch"] = json!(watch);
            } else {
                let record = candidate.entries.get_mut(&key)
                    .ok_or_else(||invalid("watch is not retained by this session"))?;
                if kind=="cancel_watch" && !record.status.terminal() {
                    record.status = Status::Cancelled;
                    record.last_seen = context.anchor;
                    record.evaluation = json!({"reason":"foreground_watch_cancelled_without_game_effect"});
                    record.seal()?;
                } else if kind=="poll_watch" { record.advance(snapshot,false)?; }
                value["record"] = record.detail(context.anchor);
            }
            let encoded = publish_work(&candidate,context,value,publish)?;
            *store = candidate;
            Ok(encoded)
        }
    }
}

#[cfg(test)]
#[path = "query_watch_tests.rs"]
mod tests;
