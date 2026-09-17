//! Install a bounded monitoring set at one anchor and publish it once. Existing
//! keys are exact-definition replays, never implicit edits or extra samples.
use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistrationEnvelope {
    schema: String,
    expected_anchor: Option<Value>,
    query: RegistrationRequest,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum RegistrationRequest {
    RegisterWatches { watches: Vec<Value> },
}

// Decode through the existing single-watch request so field types, unknown-field
// refusal and condition operators cannot drift into an alternative watch dialect.
fn definition(input: Value) -> Result<Definition> {
    let mut object = input.as_object().cloned()
        .ok_or_else(|| invalid("each watch registration must be an object"))?;
    if object.contains_key("kind") {
        return Err(invalid("watch definitions inside register_watches omit kind"));
    }
    object.insert("kind".into(), json!("watch"));
    match serde_json::from_value::<super::super::Request>(Value::Object(object))
        .map_err(|_| invalid("invalid watch in registration set"))? {
        super::super::Request::Watch { key, label, condition, failure_condition,
            deadline_tick, poll_interval_ticks, stable_observations } => {
            let result = Definition {
                label: label.unwrap_or_else(|| key.clone()), key, condition, failure_condition,
                deadline_tick, poll_interval_ticks: poll_interval_ticks.unwrap_or(1),
                stable_observations: stable_observations.unwrap_or(2),
            };
            validate_definition(&result)?;
            Ok(result)
        }
        _ => Err(invalid("registration set accepts watch definitions only")),
    }
}

pub(crate) fn execute<F>(snapshot: &WorldSnapshot, context: &OperationContext,
    input: &Value, publisher: F) -> Result<String>
where F: FnOnce(Value) -> Result<String> {
    execute_in(&WATCHES, snapshot, context, input, publisher)
}

fn execute_in<F>(storage: &Mutex<Store>, snapshot: &WorldSnapshot,
    context: &OperationContext, input: &Value, publisher: F) -> Result<String>
where F: FnOnce(Value) -> Result<String> {
    let mut work = counts::EvaluationBudget::new(context.budget.max_wall_millis);
    authorize(snapshot, context)?;
    // These are aggregate limits for the ENTIRE set, not renewed per definition.
    validate_input(input)?;
    let envelope: RegistrationEnvelope = serde_json::from_value(input.clone())
        .map_err(|_| invalid("invalid register_watches envelope"))?;
    if envelope.schema != "dfmcp.query/1" {
        return Err(invalid("register_watches requires dfmcp.query/1"));
    }
    if envelope.expected_anchor.as_ref().is_some_and(|a| a != &anchor(context.anchor)) {
        return Err(failure(ErrorCode::StaleAnchor, "registration set names another observation"));
    }
    let RegistrationRequest::RegisterWatches { watches } = envelope.query;
    if watches.is_empty() || watches.len() > MAX_PER_SESSION {
        return Err(invalid("register_watches accepts one to eight unique watch keys"));
    }
    let mut definitions = BTreeMap::new();
    for input in watches {
        let value = definition(input)?;
        if definitions.insert(value.key.clone(), value).is_some() {
            return Err(invalid("registration set contains a duplicate watch key"));
        }
    }
    let configuration = digest(&json!({"domain":"dfmcp-watch-registration-set/1",
        "definitions":definitions.values().collect::<Vec<_>>()}))?;
    let mut store = lock(storage)?;
    work.check()?;
    let existing: BTreeMap<_, _> = store.entries.iter()
        .filter(|((session, _), _)| *session == context.session_id)
        .map(|((_, handle), watch)| (watch.definition.key.clone(), handle.clone())).collect();
    let mut new_count = 0usize;
    // Validate ALL identities and deadlines before even evaluating the first new
    // watch. Expired/recovered/terminal existing keys replay without renewal.
    for (key, definition) in &definitions {
        if let Some(handle) = existing.get(key) {
            if record(&store, context.session_id, handle)?.definition != *definition {
                return Err(failure(ErrorCode::Conflict,
                    "a registration key already names another definition; no watches were installed"));
            }
        } else {
            if definition.deadline_tick <= context.anchor.tick.0
                || definition.deadline_tick.checked_sub(context.anchor.tick.0)
                    .is_none_or(|ticks| ticks > context.budget.max_game_ticks) {
                return Err(invalid("every new watch deadline must be future and within the session horizon"));
            }
            new_count += 1;
        }
    }
    if existing.len().saturating_add(new_count) > MAX_PER_SESSION
        || store.entries.len().saturating_add(new_count) > MAX_TOTAL {
        return Err(bounded("complete monitoring set does not fit watch retention; nothing was registered"));
    }
    store.serial.checked_add(new_count as u64)
        .ok_or_else(|| bounded("watch identity space exhausted"))?;
    let mut candidate = Store { serial: store.serial, entries: store.entries.clone() };
    let mut rows = Vec::with_capacity(definitions.len());
    let mut pending = Vec::new();
    for (key, definition) in definitions {
        work.check()?;
        let replayed = existing.contains_key(&key);
        let handle = match existing.get(&key) {
            Some(handle) => handle.clone(),
            None => {
                candidate.serial += 1; // Complete arithmetic bound checked above.
                let identity = digest(&json!({"domain":"dfmcp-condition-watch-v1",
                    "session":context.session_id.to_string(),"serial":candidate.serial,
                    "anchor":anchor(context.anchor),"definition":definition}))?;
                let handle = format!("watch:{identity}");
                let mut watch = Watch { handle: handle.clone(), definition,
                    created_at: context.anchor, last_seen: context.anchor,
                    last_sample_tick: None, streak: 0, samples: 0, status: Status::Waiting,
                    evaluation: Value::Null, evidence_digest: identity, recovery: None };
                watch.advance_bounded(snapshot, true, &mut work)?;
                if candidate.entries.insert((context.session_id, handle.clone()), watch).is_some() {
                    return Err(failure(ErrorCode::InternalInvariantViolation, "watch identity collision"));
                }
                handle
            }
        };
        let watch = record(&candidate, context.session_id, &handle)?;
        if !watch.status.terminal() { pending.push(handle); }
        let mut row = watch.summary(context.anchor);
        row["replayed"] = json!(replayed);
        row["sample_count"] = json!(watch.samples);
        rows.push(row);
    }
    let mut value = payload(context, "register_watches");
    value["registered"] = json!(rows.len());
    value["created"] = json!(new_count);
    value["replayed"] = json!(rows.len() - new_count);
    value["configuration_digest"] = json!(configuration.to_string());
    value["records"] = json!(rows);
    value["atomic_watch_publication"] = json!(true);
    value["only_new_watches_sampled"] = json!(true);
    value["native_captures"] = json!(0);
    value["game_effect_success_proven"] = json!(false);
    value["detail_query_kind"] = json!("poll_watch");
    value["next_step"] = if pending.is_empty() { Value::Null } else {
        json!({"tool":"fortress.query","arguments":{"session_id":context.session_id.to_string(),
            "query":{"schema":"dfmcp.query/1","query":{"kind":"await_watches","watches":pending}}}})
    };
    // Reuse the batch publisher: one existing-format checkpoint for the entire
    // candidate set. Pure output refusal and failed sync cannot expose a subset.
    let encoded = publish(&candidate, context, value, |value| {
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        work.check()?;
        let encoded = publisher(value)?;
        if encoded.len() as u64 > context.budget.max_bytes.min(u64::from(context.budget.max_output_tokens) * 4) {
            return Err(bounded("complete monitoring-set acknowledgement exceeds the output budget"));
        }
        work.check()?;
        Ok(encoded)
    })?;
    // Publication has synced. No fallible work may hide it from the caller.
    *store = candidate;
    Ok(encoded)
}

#[cfg(test)]
#[path = "query_watch_registration_tests.rs"]
mod tests;
