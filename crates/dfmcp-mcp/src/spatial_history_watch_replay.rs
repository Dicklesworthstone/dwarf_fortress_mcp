//! Retrospective sampled monitoring, never a retained live Watch. The archive
//! reader verifies the whole selected prefix before any result is returned.
use super::*;

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
    HistoricalWatchReplay {
        from: RecordRef,
        to: RecordRef,
        definition: Value,
        detail: Option<Detail>,
    },
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Detail {
    Summary,
    Evidence,
}

pub(in super::super::super) fn handles(input: &Value) -> bool {
    input
        .get("query")
        .and_then(|q| q.get("kind"))
        .and_then(Value::as_str)
        == Some("historical_watch_replay")
}
pub(in super::super::super) fn schema() -> Result<Value> {
    serde_json::from_str(include_str!(
        "../../../schemas/mcp_historical_watch_replay_v1.json"
    ))
    .map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "historical watch replay schema invalid",
        )
    })
}
fn render(
    session: &Session,
    context: &OperationContext,
    last: &JournalEntry,
    projection: Option<&QueryResponseProjection>,
    value: Value,
) -> Result<String> {
    let basis = value.get("basis").cloned().unwrap_or(Value::Null);
    let raw = match projection {
        Some(projection) => finish(projection, value)?,
        None => archive::packet(session, context, "fortress.query", Some(last), value)?,
    };
    let mut packet: Value = serde_json::from_str(&raw).map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "historical monitor packet is not JSON",
        )
    })?;
    packet["agent_turn"]["continuity"]["status"] = json!("partial");
    packet["agent_turn"]["continuity"]["basis"] = basis;
    packet["agent_turn"]["continuity"]["gap"] =
        json!({"reason":"retrospective_monitor_not_original_watch_activity_or_continuous_history"});
    let encoded = packet.to_string();
    if encoded.len() as u64
        > context
            .budget
            .max_bytes
            .min(u64::from(context.budget.max_output_tokens) * 4)
    {
        return Err(bounded(
            "complete historical monitor packet exceeds the response budget",
        ));
    }
    Ok(encoded)
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
            "historical monitor context differs from the current session",
        ));
    }
    let envelope: Envelope = serde_json::from_value(input.clone())
        .map_err(|_| invalid("invalid historical watch replay envelope"))?;
    if envelope.schema != "dfmcp.query/1" {
        return Err(invalid("historical monitor requires dfmcp.query/1"));
    }
    if envelope
        .expected_anchor
        .as_ref()
        .is_some_and(|a| a != &anchor_json(context.anchor))
    {
        return Err(error(
            ErrorCode::StaleAnchor,
            "historical monitor expected_anchor differs",
        ));
    }
    let Request::HistoricalWatchReplay {
        from,
        to,
        definition,
        detail,
    } = envelope.query;
    if to.record.checked_sub(from.record).is_none_or(|n| n >= 32) {
        return Err(bounded(
            "historical monitor requires one to 32 consecutive records",
        ));
    }
    let journal = session
        .journal
        .as_mut()
        .ok_or_else(|| invalid("historical monitor requires the configured observation archive"))?;
    journal.validate_custody(context)?;
    if journal.state().snapshot().map(|s| s.anchor()) != Some(context.anchor) {
        return Err(error(
            ErrorCode::CorruptLedger,
            "historical monitor archive differs from the current session root",
        ));
    }
    let first = selected(journal, &from)?;
    let last = selected(journal, &to)?;
    let archive_id = journal.id();
    let archive_head = journal.head();
    // Incidental session IDs and later appends do not alter this evidence identity.
    // Endpoint record digests already bind their complete predecessor chains.
    let binding=Digest32::of_bytes(json!({"domain":"dfmcp-historical-monitor-range/1",
        "journal":archive_id.to_string(),"from_record":first.number,"from_digest":first.record_digest.to_string(),
        "to_record":last.number,"to_digest":last.record_digest.to_string()}).to_string().as_bytes());
    let requested: Vec<_> = journal
        .entries()
        .iter()
        .filter(|e| (first.number..=last.number).contains(&e.number))
        .map(|e| (e.number, e.record_digest))
        .collect();
    if requested.len() as u64 != last.number - first.number + 1 {
        return Err(error(
            ErrorCode::CorruptLedger,
            "historical monitor range has missing record identities",
        ));
    }
    let metadata = json!({"ok":true,"anchor":anchor_json(last.anchor),"basis":anchor_json(first.anchor),
        "current_session_anchor":anchor_json(context.anchor),
        "range":{"journal_id":archive_id.to_string(),"journal_head":archive_head.to_string(),
            "from":archive::entry_json(&first),"to":archive::entry_json(&last)},
        "replay":{"prefix_verified_through_record":last.number,"selected_records":requested.len(),
            "starting_state":"registration_at_first_selected_capture","all_selected_records_verified":true}});
    let mut projection = if session.source.archive_only() {
        None
    } else {
        Some(view(session, context)?)
    };
    if let Some(p) = projection.as_mut() {
        p.anchor = anchor_json(last.anchor);
        p.briefing = json!({"runtime":"unadmitted_development","bridge_protocol":"1.8","historical":true,
            "live":false,"read_only":true,"runtime_admitted":false,"mutation_admissible":false,
            "live_source_fenced":session.source.poisoned(),"current_session_anchor":anchor_json(context.anchor),
            "active_work_basis":"current_session_not_replayed"});
        p.coverage = json!({"status":"partial","complete_domains":["declared_monitor_over_selected_archived_samples"],
            "omitted_domains":["current_game_state","original_watch_activity","events_between_captures","game_effect_outcomes"],
            "temporal_coverage":"retained_samples_only","continuous_between_observations":false,
            "current_freshness_proven":false,"continuation":null});
        p.references = vec![
            json!({"kind":"historical_monitor_range","journal_id":archive_id.to_string(),
            "from_digest":first.record_digest.to_string(),"to_digest":last.record_digest.to_string()}),
        ];
    }
    // Reserve fixed range witnesses and the actual live/archive presentation.
    // Variable monitor evidence is checked whole after replay, never truncated.
    let overhead = render(
        session,
        context,
        &last,
        projection.as_ref(),
        metadata.clone(),
    )?
    .len() as u64
        + 128;
    let mut result_context = remaining(context, started)?;
    result_context.budget.max_bytes = context
        .budget
        .max_bytes
        .min(u64::from(context.budget.max_output_tokens) * 4)
        .checked_sub(overhead)
        .filter(|n| *n > 0)
        .ok_or_else(|| bounded("historical monitor witnesses leave no result budget"))?;
    if projection.is_some() {
        result_context = semantic_query::result_context(&result_context)?;
    }
    let mut monitor = semantic_query::HistoricalWatchReplay::new(
        &definition,
        first.anchor,
        first.number,
        last.number,
        binding,
        &remaining(context, started)?,
    )?;
    let replay_context = history::replay_context(session, &remaining(context, started)?);
    let limits = session.limits;
    let journal = session
        .journal
        .as_mut()
        .ok_or_else(|| invalid("historical monitor archive disappeared"))?;
    journal.project_records(&requested, &replay_context, |entry, state| {
        archive::validate_limits(
            state
                .observation_full()
                .ok_or_else(|| error(ErrorCode::CorruptLedger, "replay source absent"))?,
            limits,
        )?;
        let snapshot = state
            .snapshot()
            .ok_or_else(|| error(ErrorCode::CorruptLedger, "replay snapshot absent"))?;
        monitor.advance(entry.number, snapshot, &remaining(context, started)?)
    })?;
    journal.validate_custody(context)?;
    if journal.id() != archive_id || journal.head() != archive_head {
        return Err(error(
            ErrorCode::StaleAnchor,
            "historical monitor archive changed",
        ));
    }
    result_context.budget.max_wall_millis = remaining(context, started)?.budget.max_wall_millis;
    let mut value = monitor.finish(matches!(detail, Some(Detail::Evidence)), &result_context)?;
    let out = value.as_object_mut().ok_or_else(|| {
        error(
            ErrorCode::InternalInvariantViolation,
            "monitor result is not an object",
        )
    })?;
    if let Some(metadata) = metadata.as_object() {
        out.extend(metadata.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    if projection.is_some() {
        semantic_query::publish_with_active_work(context, value, |v| {
            let out = render(session, context, &last, projection.as_ref(), v)?;
            remaining(context, started)?;
            Ok(out)
        })
    } else {
        let out = render(session, context, &last, None, value)?;
        remaining(context, started)?;
        Ok(out)
    }
}

#[cfg(all(test, unix))]
#[path = "spatial_history_watch_replay_tests.rs"]
mod tests;
