//! Exact archived job endpoints through the existing query surface. Replay,
//! comparison and packet construction share the foreground request deadline.
use super::*;

#[path = "job_progress.rs"]
mod model;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProgressEnvelope {
    schema: String,
    expected_anchor: Option<Value>,
    query: ProgressRequest,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ProgressRequest {
    HistoricalJobProgress {
        from: RecordRef,
        to: RecordRef,
        job: Option<model::Focus>,
        include_unchanged: Option<bool>,
        limit: Option<u32>,
        continuation: Option<String>,
        max_work: Option<u64>,
    },
}

pub(in super::super::super) fn handles(input: &Value) -> bool {
    input
        .get("query")
        .and_then(|v| v.get("kind"))
        .and_then(Value::as_str)
        == Some("historical_job_progress")
}
pub(in super::super::super) fn schema() -> Result<Value> {
    serde_json::from_str(include_str!(
        "../../../schemas/mcp_historical_job_progress_v1.json"
    ))
    .map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "historical job progress schema invalid",
        )
    })
}

fn render_progress(
    session: &Session,
    context: &OperationContext,
    target: &JournalEntry,
    projection: Option<&QueryResponseProjection>,
    value: Value,
) -> Result<String> {
    let basis = value.get("basis").cloned().unwrap_or(Value::Null);
    let cursor = value.get("continuation").cloned().unwrap_or(Value::Null);
    let summary = json!({"kind":"archived_job_progress","summary":value.get("summary"),"returned":value.get("returned"),
        "detail_path":"rows","intermediate_history_proven":false,"completion_proven":false,"stall_proven":false,
        "evidence":[value.get("basis_source_digest"),value.get("target_source_digest")]});
    let raw = match projection {
        Some(projection) => finish(projection, value)?,
        None => archive::packet(session, context, "fortress.query", Some(target), value)?,
    };
    let mut packet: Value = serde_json::from_str(&raw).map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "job progress packet invalid",
        )
    })?;
    packet["agent_turn"]["continuity"]["status"] = json!("partial");
    packet["agent_turn"]["continuity"]["basis"] = basis;
    packet["agent_turn"]["continuity"]["gap"] =
        json!({"reason":"historical_job_endpoints_do_not_prove_continuous_or_current_state"});
    packet["agent_turn"]["coverage"]["continuation"] = cursor;
    packet["agent_turn"]["changes"] = json!([summary]);
    let encoded = packet.to_string();
    if encoded.len() as u64
        > context
            .budget
            .max_bytes
            .min(u64::from(context.budget.max_output_tokens) * 4)
    {
        return Err(bounded(
            "complete historical job progress packet exceeds output budget",
        ));
    }
    Ok(encoded)
}

fn replay_state(
    journal: &mut history::Journal,
    entry: &JournalEntry,
    context: &OperationContext,
    limits: CitizenSpatialLimits,
) -> Result<LiveSpatialCitizenState> {
    let state = journal.state_at(entry.number, entry.record_digest, context)?;
    archive::validate_limits(
        state.observation_full().ok_or_else(|| {
            error(
                ErrorCode::CorruptLedger,
                "job progress archived source absent",
            )
        })?,
        limits,
    )?;
    if state.snapshot().map(WorldSnapshot::anchor) != Some(entry.anchor)
        || state.source_digest()? != entry.source_digest
    {
        return Err(error(
            ErrorCode::CorruptLedger,
            "job progress replay differs from retained endpoint witness",
        ));
    }
    Ok(state)
}

pub(in super::super::super) fn execute(
    session: &mut Session,
    context: &OperationContext,
    input: &Value,
) -> Result<String> {
    let started = Instant::now();
    remaining(context, started)?;
    shape(input)?;
    if session.anchor()? != context.anchor {
        return Err(error(
            ErrorCode::StaleAnchor,
            "job progress context differs from current session",
        ));
    }
    let envelope: ProgressEnvelope = serde_json::from_value(input.clone())
        .map_err(|_| invalid("invalid historical job progress envelope"))?;
    if envelope.schema != "dfmcp.query/1" {
        return Err(invalid("historical job progress requires dfmcp.query/1"));
    }
    if envelope
        .expected_anchor
        .as_ref()
        .is_some_and(|v| v != &anchor_json(context.anchor))
    {
        return Err(error(
            ErrorCode::StaleAnchor,
            "job progress expected_anchor differs from current session",
        ));
    }
    let ProgressRequest::HistoricalJobProgress {
        from,
        to,
        job,
        include_unchanged,
        limit,
        continuation,
        max_work,
    } = envelope.query;
    let limit = limit.unwrap_or(4);
    let maximum = max_work.unwrap_or(model::MAX_WORK);
    if from.record > to.record {
        return Err(invalid(
            "historical job progress requires from.record <= to.record",
        ));
    }
    if !(1..=128).contains(&limit)
        || limit > context.budget.max_entities
        || !(1..=model::MAX_WORK).contains(&maximum)
        || continuation.as_ref().is_some_and(|v| v.len() > 128)
    {
        return Err(bounded(
            "historical job progress page or work arguments exceed bounds",
        ));
    }
    if let Some(job) = &job {
        job.validate()?;
    }
    let include_unchanged = include_unchanged.unwrap_or(true);
    let selection =
        json!({"job":job.as_ref().map(model::Focus::json),"include_unchanged":include_unchanged});
    let journal = session.journal.as_mut().ok_or_else(|| {
        invalid("historical job progress requires the configured spatial/1.8 journal")
    })?;
    journal.validate_custody(context)?;
    if journal.state().snapshot().map(WorldSnapshot::anchor) != Some(context.anchor) {
        return Err(error(
            ErrorCode::StaleAnchor,
            "job progress archive head differs from current session",
        ));
    }
    let first = selected(journal, &from)?;
    let last = selected(journal, &to)?;
    if first.anchor.fortress_id != last.anchor.fortress_id
        || first.anchor.cursor.epoch != last.anchor.cursor.epoch
        || last.anchor.tick < first.anchor.tick
    {
        return Err(error(
            ErrorCode::StaleAnchor,
            "job progress crosses fortress, observation reset or regressed clock",
        ));
    }
    let archive_id = journal.id();
    let archive_head = journal.head();
    let from_json = archive::entry_json(&first);
    let to_json = archive::entry_json(&last);
    let metadata = json!({"historical":true,"live":false,"current_freshness_proven":false,"native_captures":0,
        "comparison":{"journal_id":archive_id.to_string(),"journal_head":archive_head.to_string(),"from":from_json,"to":to_json},
        "current_session_anchor":anchor_json(context.anchor)});
    let binding = Digest32::of_bytes(metadata.to_string().as_bytes());
    let mut projection = if session.source.archive_only() {
        None
    } else {
        Some(view(session, context)?)
    };
    if let Some(p) = projection.as_mut() {
        p.anchor = anchor_json(last.anchor);
        p.briefing = json!({"runtime":"unadmitted_development","bridge_protocol":"1.8","historical":true,"read_only":true,
            "live":false,"runtime_admitted":false,"mutation_admissible":false,"current_live_anchor":anchor_json(context.anchor),
            "live_source_fenced":session.source.poisoned(),"active_work_basis":"current_session_not_archived"});
        p.coverage = json!({"status":"partial","complete_domains":["selected_endpoint_job_rosters"],
            "omitted_domains":["current_game_state","events_between_endpoints","native_blocker_reasons"],"absence_proven":false,
            "temporal_coverage":"endpoint_comparison_only","current_freshness_proven":false,"continuation":null});
        p.references = vec![
            json!({"kind":"verified_job_progress_endpoints","journal_id":archive_id.to_string(),
            "from_digest":first.record_digest.to_string(),"to_digest":last.record_digest.to_string()}),
        ];
    }
    // Reserve the complete Agent Turn and both record witnesses before replay.
    // A bounded synthetic summary also reserves its duplicate in agent_turn.
    let mut sample = metadata.clone();
    sample["kind"] = json!("historical_job_progress");
    sample["basis"] = anchor_json(first.anchor);
    sample["anchor"] = anchor_json(last.anchor);
    sample["returned"] = json!(128);
    sample["continuation"] = json!("x".repeat(128));
    sample["basis_source_digest"] = json!("f".repeat(64));
    sample["target_source_digest"] = json!("f".repeat(64));
    sample["summary"] = json!({"reserved":"x".repeat(1800)});
    let overhead = render_progress(session, context, &last, projection.as_ref(), sample.clone())?
        .len()
        .saturating_sub(sample.to_string().len())
        .saturating_add(metadata.to_string().len())
        .saturating_add(256) as u64;
    let mut comparison = remaining(context, started)?;
    comparison.budget.max_bytes = context
        .budget
        .max_bytes
        .min(u64::from(context.budget.max_output_tokens) * 4)
        .checked_sub(overhead + 64)
        .filter(|v| *v > 0)
        .ok_or_else(|| bounded("job progress archive evidence leaves no result budget"))?;
    if projection.is_some() {
        comparison = semantic_query::result_context(&comparison)?;
    }
    let mut read_context = history::replay_context(session, &remaining(context, started)?);
    let limits = session.limits;
    let journal = session
        .journal
        .as_mut()
        .ok_or_else(|| invalid("job progress journal disappeared"))?;
    let before = replay_state(journal, &first, &read_context, limits)?;
    read_context.budget.max_wall_millis = remaining(context, started)?.budget.max_wall_millis;
    let after = replay_state(journal, &last, &read_context, limits)?;
    comparison.budget.max_wall_millis = remaining(context, started)?.budget.max_wall_millis;
    let report = model::compare(
        &before,
        &after,
        &comparison,
        job.as_ref(),
        include_unchanged,
        maximum,
    )?;
    comparison.budget.max_wall_millis = remaining(context, started)?.budget.max_wall_millis;
    let mut value = model::paginate(
        &report,
        model::Page {
            context: &comparison,
            from: &from_json,
            to: &to_json,
            binding,
            selection,
            maximum_work: maximum,
            limit,
            continuation: continuation.as_deref(),
        },
    )?;
    journal.validate_custody(context)?;
    if journal.id() != archive_id || journal.head() != archive_head {
        return Err(error(
            ErrorCode::StaleAnchor,
            "job progress archive head changed",
        ));
    }
    let result = value.as_object_mut().ok_or_else(|| {
        error(
            ErrorCode::InternalInvariantViolation,
            "job progress result is not object",
        )
    })?;
    if let Some(metadata) = metadata.as_object() {
        result.extend(metadata.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    remaining(context, started)?;
    if projection.is_some() {
        semantic_query::publish_with_active_work(context, value, |value| {
            let packet = render_progress(session, context, &last, projection.as_ref(), value)?;
            remaining(context, started)?;
            Ok(packet)
        })
    } else {
        let packet = render_progress(session, context, &last, None, value)?;
        remaining(context, started)?;
        Ok(packet)
    }
}
