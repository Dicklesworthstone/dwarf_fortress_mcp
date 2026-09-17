//! Read-only quantity and condition timelines. Each page projects one verified
//! prefix; no watch, baseline, bridge capture or current world is published.
use super::*;
#[path = "spatial_condition_series.rs"]
mod conditions;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SeriesEnvelope { schema: String, expected_anchor: Option<Value>, query: SeriesRequest }
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum SeriesRequest {
    HistoricalSeries { from: RecordRef, to: RecordRef, measurement: Value,
        limit: Option<u32>, continuation: Option<String> },
}

pub(in super::super::super) fn handles(input: &Value) -> bool {
    input.get("query").and_then(|q|q.get("kind")).and_then(Value::as_str) == Some("historical_series")
}
pub(in super::super::super) fn schema() -> Result<Value> {
    let mut schema: Value = serde_json::from_str(include_str!("../../../schemas/mcp_historical_series_v1.json"))
        .map_err(|_| error(ErrorCode::InternalInvariantViolation,"historical series schema invalid"))?;
    // Both measurement contracts reference the same watch predicate definitions
    // as current inspection. No parallel historical condition language exists.
    let quantity: Value = serde_json::from_str(include_str!("../../../schemas/mcp_item_quantity_v1.json"))
        .map_err(|_| error(ErrorCode::InternalInvariantViolation,"quantity schema invalid"))?;
    let condition: Value = serde_json::from_str(include_str!("../../../schemas/mcp_condition_evaluation_v1.json"))
        .map_err(|_| error(ErrorCode::InternalInvariantViolation,"condition schema invalid"))?;
    schema["properties"]["measurement"] = json!({"oneOf":[quantity["query"],condition]});
    Ok(schema)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct QuantityBounds { lower: u64, upper: Option<u64> }
impl QuantityBounds {
    fn read(value: &Value) -> Result<Self> {
        let bad = || error(ErrorCode::InternalInvariantViolation,"quantity evaluator returned invalid bounds");
        let quantity = value.get("quantity").ok_or_else(bad)?;
        let lower = quantity.get("quantity_min").and_then(Value::as_u64).ok_or_else(bad)?;
        let upper = match quantity.get("quantity_max") {
            Some(Value::Null) => None,
            Some(value) => Some(value.as_u64().ok_or_else(bad)?),
            None => return Err(bad()),
        };
        if upper.is_some_and(|n| n < lower) { return Err(bad()); }
        Ok(Self { lower, upper })
    }
    fn difference(self, previous: Self) -> (Option<i128>, Option<i128>) {
        (previous.upper.map(|n| i128::from(self.lower) - i128::from(n)),
            self.upper.map(|n| i128::from(n) - i128::from(previous.lower)))
    }
}
fn transition(previous: &(JournalEntry, Value), current: &(JournalEntry, Value)) -> Result<Value> {
    let before = &previous.0; let after = &current.0;
    if before.number.checked_add(1) != Some(after.number) {
        return Err(error(ErrorCode::InternalInvariantViolation,"timeline omitted an intermediate retained sample"));
    }
    let mut result = json!({"from_record":before.number,"from_record_digest":before.record_digest.to_string(),
        "continuous_between_observations":false,"causality_proven":false});
    if before.anchor.fortress_id != after.anchor.fortress_id
        || before.anchor.cursor.epoch != after.anchor.cursor.epoch || after.anchor.tick < before.anchor.tick {
        result["status"] = json!("epoch_or_clock_discontinuity");
        result["net_change"] = Value::Null; result["net_rate"] = Value::Null;
        return Ok(result);
    }
    if current.1.get("kind").and_then(Value::as_str)==Some("condition_evaluation") {
        return conditions::transition(&previous.1,&current.1,result,after.anchor.tick.0-before.anchor.tick.0);
    }
    let (lower, upper) = QuantityBounds::read(&current.1)?.difference(QuantityBounds::read(&previous.1)?);
    // Decimal strings preserve every signed difference of two u64 quantities.
    // Neither IEEE-754 rounding nor JSON i64 overflow is an acceptable shortcut.
    let interval = json!({"minimum":lower.map(|n|n.to_string()),"maximum":upper.map(|n|n.to_string()),
        "encoding":"signed_decimal_string","unit":"stack_units",
        "exact":lower.is_some() && lower==upper});
    let ticks = after.anchor.tick.0 - before.anchor.tick.0;
    result["status"] = json!(if lower.is_some_and(|n|n>0) {"definite_increase"}
        else if upper.is_some_and(|n|n<0) {"definite_decrease"}
        else if lower==Some(0) && upper==Some(0) {"unchanged_quantity"} else {"indeterminate_change"});
    result["net_change"] = interval;
    result["elapsed_game_ticks"] = json!(ticks);
    result["net_rate"] = if ticks==0 {Value::Null} else {json!({
        "numerator_minimum":lower.map(|n|n.to_string()),"numerator_maximum":upper.map(|n|n.to_string()),
        "denominator_game_ticks":ticks,"unit":"stack_units_per_game_tick",
        "interpretation":"endpoint_net_change_not_production_or_consumption_rate"})};
    result["rate_unavailable_reason"] = if ticks==0 {json!("same_game_tick")} else {Value::Null};
    Ok(result)
}
fn continuation(identity: Digest32, record: u64) -> String {
    let bytes = json!({"domain":"dfmcp-quantity-timeline-page/1","identity":identity.to_string(),"next_record":record});
    format!("hs1:{record}:{}",Digest32::of_bytes(bytes.to_string().as_bytes()))
}
fn start_record(raw: Option<&str>, identity: Digest32, first: u64, last: u64) -> Result<u64> {
    let Some(raw) = raw else { return Ok(first); };
    if raw.len()>128 { return Err(bounded("timeline continuation exceeds 128 bytes")); }
    let parts: Vec<_> = raw.split(':').collect();
    if parts.len()!=3 || parts[0]!="hs1" || parts[1].is_empty() || parts[1].starts_with('0')
        || !parts[1].bytes().all(|b|b.is_ascii_digit()) { return Err(invalid("invalid timeline continuation")); }
    let record = parts[1].parse::<u64>().map_err(|_|invalid("timeline continuation overflows"))?;
    if raw!=continuation(identity,record) {
        return Err(error(ErrorCode::StaleAnchor,"timeline continuation names another session, archive head, interval or measurement"));
    }
    if record<=first || record>last { return Err(error(ErrorCode::CursorGap,"timeline continuation is outside the selected interval")); }
    Ok(record)
}
fn render_series(session: &Session, context: &OperationContext, target: &JournalEntry,
    projection: Option<&QueryResponseProjection>, mut value: Value) -> Result<String> {
    let cursor = value.get("continuation").cloned().unwrap_or(Value::Null);
    value["anchor"] = anchor_json(target.anchor);
    let raw = match projection {
        Some(projection) => finish(projection,value)?,
        None => archive::packet(session,context,"fortress.query",Some(target),value)?,
    };
    let mut packet: Value = serde_json::from_str(&raw)
        .map_err(|_|error(ErrorCode::InternalInvariantViolation,"timeline packet is not JSON"))?;
    packet["agent_turn"]["continuity"]["status"] = json!("partial");
    packet["agent_turn"]["continuity"]["gap"] = json!({"reason":"retained_samples_not_continuous_or_current_state"});
    packet["agent_turn"]["coverage"]["continuation"] = cursor;
    let encoded = packet.to_string();
    if encoded.len() as u64 > context.budget.max_bytes.min(u64::from(context.budget.max_output_tokens)*4) {
        return Err(bounded("complete historical series packet exceeds output budget"));
    }
    Ok(encoded)
}

pub(in super::super::super) fn execute(session: &mut Session, context: &OperationContext, input: &Value) -> Result<String> {
    let started = Instant::now(); remaining(context,started)?; shape(input)?;
    if session.anchor()? != context.anchor { return Err(error(ErrorCode::StaleAnchor,"timeline context differs from current session")); }
    let parsed: SeriesEnvelope = serde_json::from_value(input.clone()).map_err(|_|invalid("invalid historical series envelope"))?;
    if parsed.schema!="dfmcp.query/1" { return Err(invalid("historical series requires dfmcp.query/1")); }
    if parsed.expected_anchor.as_ref().is_some_and(|a|a!=&anchor_json(context.anchor)) {
        return Err(error(ErrorCode::StaleAnchor,"timeline expected_anchor differs from current session"));
    }
    let SeriesRequest::HistoricalSeries { from, to, measurement, limit, continuation: raw } = parsed.query;
    let limit = limit.unwrap_or(8);
    if from.record>to.record || !(1..=32).contains(&limit) || limit>context.budget.max_entities {
        return Err(invalid("timeline requires increasing endpoints and a page limit of 1..32"));
    }
    let condition_measurement = match measurement.get("kind").and_then(Value::as_str) {
        Some("item_quantity") => false,
        Some("condition_evaluation") => true,
        _ => return Err(invalid("historical series permits stateless item_quantity or condition_evaluation only")),
    };
    let journal = session.journal.as_mut().ok_or_else(||invalid("historical series requires the configured spatial observation journal"))?;
    journal.validate_custody(context)?;
    if journal.state().snapshot().map(|s|s.anchor())!=Some(context.anchor) {
        return Err(error(ErrorCode::CorruptLedger,"timeline archive differs from the current session root"));
    }
    let first = selected(journal,&from)?; let last = selected(journal,&to)?;
    let id = journal.id(); let head = journal.head();
    let identity = Digest32::of_bytes(json!({"domain":"dfmcp-quantity-timeline/1","session":session.id.to_string(),
        "current_anchor":anchor_json(context.anchor),"journal":id.to_string(),"head":head.to_string(),
        "from":archive::entry_json(&first),"to":archive::entry_json(&last),"measurement":measurement}).to_string().as_bytes());
    let start = start_record(raw.as_deref(),identity,first.number,last.number)?;
    let end = last.number.min(start + u64::from(limit) - 1);
    let basis = if start>first.number {start-1} else {start};
    let requested: Vec<_> = journal.entries().iter().filter(|e| (basis..=end).contains(&e.number))
        .map(|e|(e.number,e.record_digest)).collect();
    let mut projection = if session.source.archive_only() {None} else {Some(view(session,context)?)};
    if let Some(p) = projection.as_mut() {
        p.anchor = anchor_json(last.anchor);
        p.briefing = json!({"runtime":"unadmitted_development","bridge_protocol":"1.8","historical":true,
            "read_only":true,"runtime_admitted":false,"mutation_admissible":false,"live":false,
            "current_session_anchor":anchor_json(context.anchor),"live_source_fenced":session.source.poisoned(),
            "active_work_basis":"current_session_not_archived"});
        p.coverage = json!({"status":"partial","complete_domains":[if condition_measurement {"returned_archived_condition_samples"} else {"returned_archived_quantity_samples"}],
            "omitted_domains":["current_game_state","events_between_captures","usable_supply"],
            "temporal_coverage":"retained_observation_samples_only","current_freshness_proven":false,"continuation":null});
        p.references = vec![json!({"kind":if condition_measurement {"archived_condition_series"} else {"archived_quantity_series"},"journal_id":id.to_string(),
            "from_digest":first.record_digest.to_string(),"to_digest":last.record_digest.to_string()})];
    }
    let mut out = json!({"ok":true,"schema":"dfmcp.query.result/1","kind":"historical_series",
        "historical":true,"live":false,"current_freshness_proven":false,"native_captures":0,
        "watch_evaluated":false,"watch_registered":false,"mutation_dispatched":false,
        "measurement":measurement,"series_digest":identity.to_string(),
        "range":{"journal_id":id.to_string(),"journal_head":head.to_string(),"from":archive::entry_json(&first),
            "to":archive::entry_json(&last),"retained_records":last.number-first.number+1},
        "current_session_anchor":anchor_json(context.anchor),"rows":[],"returned":0,"truncated":false,"continuation":null,
        "replay":{"prefix_verified_through_record":end,"projected_records":requested.len(),"preceding_sample_for_change":start>first.number},
        "interpretation":if condition_measurement {
            "Conditions and failure guards evaluated at retained captures, without creating or sampling watches. Classification changes do not prove continuous satisfaction, cadence, deadline compliance or stable goal completion."
        } else {"Dynamic selected stack quantities at retained captures. Net changes are not consumption, production, continuous stability or depletion forecasts."}});
    // Reserve all fixed evidence, the mode-specific Agent Turn and duplicated
    // continuation before replay. Keep a margin for varying anchor digit widths.
    let mut sample = out.clone(); sample["continuation"] = json!("x".repeat(128));
    let overhead = render_series(session,context,&last,projection.as_ref(),sample.clone())?.len()
        .saturating_sub(sample.to_string().len()) + 512;
    let mut result_context = remaining(context,started)?;
    result_context.budget.max_bytes = context.budget.max_bytes.min(u64::from(context.budget.max_output_tokens)*4)
        .checked_sub(overhead as u64).filter(|n|*n>0).ok_or_else(||bounded("timeline evidence leaves no result allowance"))?;
    if projection.is_some() { result_context = semantic_query::result_context(&result_context)?; }
    let maximum = result_context.budget.max_bytes;
    if sample.to_string().len() as u64 >= maximum { return Err(bounded("timeline metadata leaves no sample allowance")); }
    let replay_context = history::replay_context(session,&remaining(context,started)?);
    let limits = session.limits;
    let query = json!({"schema":"dfmcp.query/1","query":out["measurement"]});
    let journal = session.journal.as_mut().ok_or_else(||invalid("timeline archive disappeared"))?;
    let values = journal.project_records(&requested,&replay_context,|entry,state| {
        archive::validate_limits(state.observation_full().ok_or_else(||error(ErrorCode::CorruptLedger,"timeline source absent"))?,limits)?;
        let snapshot = state.snapshot().ok_or_else(||error(ErrorCode::CorruptLedger,"timeline snapshot absent"))?;
        let mut current = remaining(context,started)?; current.anchor = entry.anchor;
        // Internal measurement output is separate from the complete page budget;
        // every scan still has the existing one-million-unit work ceiling.
        current.budget.max_bytes = current.budget.max_bytes.min(16*1024);
        let value = semantic_query::execute(snapshot,&current,&query)?;
        if condition_measurement {conditions::validate(&value)?;} else {QuantityBounds::read(&value)?;}
        remaining(context,started)?;
        Ok(value)
    })?;
    journal.validate_custody(context)?;
    if journal.id()!=id || journal.head()!=head { return Err(error(ErrorCode::StaleAnchor,"timeline archive head changed")); }
    let mut returned = 0u64; let mut target = first.clone();
    for (index,current) in values.iter().enumerate() {
        if current.0.number < start { continue; }
        let change = if current.0.number==first.number {Value::Null} else {
            let previous = index.checked_sub(1).and_then(|n|values.get(n))
                .ok_or_else(||error(ErrorCode::InternalInvariantViolation,"timeline page lost preceding sample"))?;
            transition(previous,current)?
        };
        let row = if condition_measurement { conditions::row(&current.0,&current.1,change)? } else {
            json!({"record":archive::entry_json(&current.0),"quantity":current.1["quantity"],
                "evidence_digest":current.1["evidence_digest"],"change_from_previous":change})
        };
        let mut next = out.clone();
        next["rows"].as_array_mut().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"timeline rows absent"))?.push(row);
        next["returned"] = json!(returned+1); next["truncated"] = json!(current.0.number<last.number);
        next["continuation"] = if current.0.number<last.number {json!(continuation(identity,current.0.number+1))} else {Value::Null};
        if next.to_string().len() as u64 > maximum { break; }
        target = current.0.clone(); out = next; returned += 1;
    }
    if returned==0 { return Err(bounded("one whole timeline sample and its evidence cannot fit")); }
    if let Some(p) = projection.as_mut() { p.anchor = anchor_json(target.anchor); }
    remaining(context,started)?;
    if projection.is_some() {
        semantic_query::publish_with_active_work(context,out,|value| {
            let result = render_series(session,context,&target,projection.as_ref(),value)?;
            remaining(context,started)?; Ok(result)
        })
    } else {
        let result = render_series(session,context,&target,None,out)?;
        remaining(context,started)?; Ok(result)
    }
}

#[cfg(test)]
#[path = "spatial_history_series_math_tests.rs"]
mod math_tests;
