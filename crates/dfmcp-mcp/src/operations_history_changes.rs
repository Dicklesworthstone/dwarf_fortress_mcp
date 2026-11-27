//! Exact selected changes between two verified operations journal endpoints.
//! Current authority and one deadline govern replay, comparison and publication.
//! No observation, baseline, watch or game effect is created by this operation.

use super::*;
use dfmcp_world::WorldSnapshot;
use std::time::Instant;

#[path = "spatial_query_source.rs"]
#[allow(dead_code, clippy::duplicate_mod)]
mod source;

#[path = "operations_furniture_arrivals.rs"]
mod furniture;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordRef {
    record: u64,
    record_digest: String,
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
    HistoricalFurnitureArrivals {
        from: RecordRef,
        to: RecordRef,
        furniture_kind: furniture::Kind,
        material: Option<[i32; 2]>,
    },
    HistoricalChanges {
        from: RecordRef,
        to: RecordRef,
        select: Value,
        limit: Option<u32>,
        continuation: Option<String>,
    },
}

fn invalid(text: &str) -> DfmcpError {
    error(ErrorCode::InvalidRequest, text)
}

fn bounded(text: &str) -> DfmcpError {
    error(ErrorCode::BudgetExceeded, text)
}

fn remaining(current: &OperationContext, started: Instant) -> Result<OperationContext> {
    current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    let elapsed = started.elapsed().as_millis();
    if elapsed >= u128::from(current.budget.max_wall_millis) {
        return Err(bounded(
            "operations comparison exhausted its foreground deadline",
        ));
    }
    let mut context = current.clone();
    context.budget.max_wall_millis -= elapsed as u64;
    Ok(context)
}

fn selected(journal: &Journal, requested: &RecordRef) -> Result<JournalEntry> {
    if !(1..=4096).contains(&requested.record) {
        return Err(invalid("historical record must be 1..4096"));
    }
    let digest = Digest32::from_hex(&requested.record_digest)
        .filter(|value| value.to_string() == requested.record_digest)
        .ok_or_else(|| invalid("record_digest must be canonical lowercase SHA-256"))?;
    let index = usize::try_from(requested.record - 1)
        .map_err(|_| invalid("historical record index cannot be represented"))?;
    let entry = journal
        .entries()
        .get(index)
        .ok_or_else(|| error(ErrorCode::CursorGap, "comparison record is not retained"))?;
    if entry.record_digest != digest {
        return Err(error(
            ErrorCode::StaleAnchor,
            "comparison record digest differs",
        ));
    }
    Ok(entry.clone())
}

/// Charge each complete endpoint before acquiring another one. A narrow
/// selection does not make unselected source records disappear from admission.
#[derive(Default)]
struct SourceAdmission {
    records: u64,
    canonical_bytes: u64,
    structural_bytes: usize,
    work: usize,
    endpoints: u32,
}

impl SourceAdmission {
    fn admit(
        &mut self,
        snapshot: &WorldSnapshot,
        current: &OperationContext,
        started: Instant,
    ) -> Result<()> {
        let mut context = remaining(current, started)?;
        if snapshot.fortress_id != current.anchor.fortress_id {
            return Err(error(
                ErrorCode::StaleAnchor,
                "comparison source fortress differs",
            ));
        }
        self.records = self.records.saturating_add(
            snapshot.graph.entities.len() as u64
                + snapshot.graph.edges.len() as u64
                + snapshot.graph.chunks.len() as u64
                + snapshot.graph.events.len() as u64,
        );
        if self.records > u64::from(current.budget.max_entities) {
            return Err(bounded(
                "comparison exceeds its complete-source record allowance",
            ));
        }
        context.budget.max_bytes = 16 * 1024 * 1024;
        let admitted = source::preflight(snapshot, &context, Instant::now())?;
        self.structural_bytes = self.structural_bytes.saturating_add(admitted.bytes);
        self.work = self.work.saturating_add(admitted.work);
        if self.structural_bytes > 16 * 1024 * 1024 || self.work > 4_000_000 {
            return Err(bounded(
                "comparison exceeds its shared source structure allowance",
            ));
        }
        self.canonical_bytes = self
            .canonical_bytes
            .saturating_add(snapshot.canonical_bytes().len() as u64);
        if self.canonical_bytes > current.budget.max_bytes {
            return Err(bounded(
                "comparison exceeds its complete canonical-source byte allowance",
            ));
        }
        self.endpoints += 1;
        remaining(current, started)?;
        Ok(())
    }

    fn metadata(&self) -> Value {
        json!({"records":self.records,"canonical_bytes":self.canonical_bytes,
            "structural_work_units":self.work,"distinct_endpoints":self.endpoints})
    }
}

fn render(view: &QueryResponseProjection, value: Value) -> Result<String> {
    let basis = value.get("basis").cloned().unwrap_or(Value::Null);
    let changes = if value["kind"] == "historical_furniture_arrivals" {
        json!([{"kind":"archived_furniture_arrivals","new_item_count":value["new_item_count"],
            "net_item_count_change":value["net_item_count_change"],"detail_path":"new_item_examples",
            "evidence":[value["comparison"]["from"]["record_digest"],value["comparison"]["to"]["record_digest"]]}])
    } else {
        json!([{"kind":"archived_endpoint_comparison",
        "change_count":value.get("change_count"),"returned":value.get("returned"),
        "detail_path":"changes","intermediate_history_proven":false,
        "evidence":[value.get("basis_result_digest"),value.get("target_result_digest")]}])
    };
    let mut packet: Value = serde_json::from_str(&finish_query(view, value)?).map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "comparison packet is invalid",
        )
    })?;
    packet["agent_turn"]["continuity"]["status"] = json!("partial");
    packet["agent_turn"]["continuity"]["basis"] = basis;
    packet["agent_turn"]["continuity"]["gap"] = json!({
        "reason":"historical_endpoint_comparison_not_continuous_or_current_game_state"});
    packet["agent_turn"]["changes"] = changes;
    let encoded = packet.to_string();
    if encoded.len() > view.maximum_bytes {
        return Err(bounded(
            "complete historical comparison exceeds its response allowance",
        ));
    }
    Ok(encoded)
}

pub(super) fn execute(
    session: &mut OperationsSession,
    current: &OperationContext,
    input: &Value,
) -> Result<String> {
    let started = Instant::now();
    remaining(current, started)?;
    semantic_query::validate_retained_input(input)?;
    if session.anchor()? != current.anchor {
        return Err(error(
            ErrorCode::StaleAnchor,
            "comparison requires the current session anchor",
        ));
    }
    let envelope: Envelope = serde_json::from_value(input.clone())
        .map_err(|_| invalid("invalid operations historical comparison envelope"))?;
    if envelope.schema != "dfmcp.query/1" {
        return Err(invalid("historical comparison requires dfmcp.query/1"));
    }
    if envelope
        .expected_anchor
        .as_ref()
        .is_some_and(|anchor| anchor != &anchor_json(current.anchor))
    {
        return Err(error(
            ErrorCode::StaleAnchor,
            "comparison expected_anchor differs from current session",
        ));
    }
    let (from, to) = match &envelope.query {
        Request::HistoricalChanges { from, to, select, limit, continuation } => {
            if limit.is_some_and(|value| value == 0 || value > 128 || value > current.budget.max_entities)
                || continuation.as_ref().is_some_and(|value| value.is_empty() || value.len() > 128)
            {
                return Err(invalid("historical comparison page arguments exceed their bounds"));
            }
            semantic_query::validate_entities_selection(select)?;
            (from, to)
        }
        Request::HistoricalFurnitureArrivals { from, to, material, .. } => {
            if material.is_some_and(|[kind, index]| kind < 0 || index < -1) {
                return Err(invalid("expected an exact native material type and index"));
            }
            (from, to)
        }
    };
    if from.record > to.record {
        return Err(invalid("comparison requires from.record <= to.record"));
    }
    let profile = session.state.profile();
    let journal = session.journal.as_mut().ok_or_else(|| {
        invalid("historical comparison requires the operator-configured operations journal")
    })?;
    if journal.profile() != profile
        || journal.state().snapshot().map(|snapshot| snapshot.anchor()) != Some(current.anchor)
    {
        return Err(error(
            ErrorCode::CorruptLedger,
            "comparison journal differs from its session profile or published anchor",
        ));
    }
    journal.validate_custody(&remaining(current, started)?)?;
    let first = selected(journal, from)?;
    let last = selected(journal, to)?;
    if first.anchor.cursor.epoch != last.anchor.cursor.epoch || last.anchor.tick < first.anchor.tick
    {
        return Err(error(
            ErrorCode::StaleAnchor,
            "comparison crosses an observation reset or regressed clock",
        ));
    }
    let archive_id = journal.id();
    let archive_head = journal.head();
    let profile_name = format!("operations/{}", profile.protocol());
    let mut metadata = json!({"historical":true,"live":false,"current_freshness_proven":false,
        "native_observations":0,"eligible_as_mutation_evidence":false,
        "comparison":{"journal_id":archive_id.to_string(),"journal_head":archive_head.to_string(),
            "profile":profile_name,"from":entry_json(&first),"to":entry_json(&last)},
        "current_live_anchor":anchor_json(current.anchor)});
    let binding = Digest32::of_bytes(
        &serde_json::to_vec(&json!({
        "domain":"dfmcp-operations-archived-comparison/1","metadata":metadata}))
        .map_err(|_| invalid("comparison binding cannot be encoded"))?,
    );
    let mut view = query_view(session, current)?;
    view.anchor = anchor_json(last.anchor);
    view.briefing = json!({"runtime":"unadmitted_development","bridge_protocol":session.state.profile().protocol(),
        "read_only":true,"historical":true,"live":false,"runtime_admitted":false,"mutation_admissible":false,
        "current_live_anchor":anchor_json(current.anchor),"live_source_fenced":session.source.poisoned(),
        "active_work_basis":"current_session_not_archived","query_source":"verified_operations_endpoint_comparison"});
    view.coverage = json!({"status":"partial","complete_domains":["selected_endpoint_projections"],
        "partial_domains":[],"omitted_domains":["current_game_state","events_between_endpoints"],
        "absence_proven":false,"temporal_coverage":"endpoint_comparison_only","current_freshness_proven":false});
    view.references = vec![
        json!({"kind":"verified_operations_comparison_endpoints","journal_id":archive_id.to_string(),
        "profile":profile_name,"from_digest":first.record_digest.to_string(),"to_digest":last.record_digest.to_string()}),
    ];

    let (select, limit, continuation) = match envelope.query {
        Request::HistoricalChanges { select, limit, continuation, .. } => (select, limit, continuation),
        Request::HistoricalFurnitureArrivals { furniture_kind, material, .. } => {
            let journal = session.journal.as_mut().ok_or_else(|| invalid("comparison journal disappeared"))?;
            let result = furniture::compare(
                journal, [&first, &last], furniture_kind, material, current, started,
            )?;
            view.coverage["complete_domains"] = json!(["furniture_rosters_at_selected_endpoints"]);
            return publish_comparison(journal, current, started, result, metadata, &view, (archive_id, archive_head, profile));
        }
    };

    // Reserve the entire packet and its repeated evidence before disk replay.
    let comparator_sample = json!({
        "schema":"dfmcp.query.result/1", "basis":anchor_json(first.anchor),
        "anchor":anchor_json(last.anchor), "kind":"historical_changes",
        "change_count":u64::MAX, "returned":u64::MAX, "truncated":true,
        "continuation":"x".repeat(128), "basis_result_digest":"f".repeat(64),
        "target_result_digest":"f".repeat(64)
    });
    let mut sample = metadata.clone();
    let sample_object = sample.as_object_mut().ok_or_else(|| {
        error(
            ErrorCode::InternalInvariantViolation,
            "comparison sample is not an object",
        )
    })?;
    let comparator_object = comparator_sample.as_object().ok_or_else(|| {
        error(
            ErrorCode::InternalInvariantViolation,
            "comparison core sample is not an object",
        )
    })?;
    sample_object.extend(
        comparator_object
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    sample["source_work"] = json!({"records":u64::MAX,"canonical_bytes":u64::MAX,
        "structural_work_units":u64::MAX,"distinct_endpoints":2});
    // The comparator budgets its complete result, including these shared fields.
    // Charge only the enclosing archive evidence/Agent Turn here, not those
    // same fields a second time. Final publication still checks exact bytes.
    let sample_bytes =
        semantic_query::publish_with_active_work(current, sample, |value| render(&view, value))?
            .len();
    let overhead = sample_bytes
        .checked_sub(comparator_sample.to_string().len())
        .ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "comparison reservation underflow",
            )
        })?;
    let output_bytes = view
        .maximum_bytes
        .checked_sub(overhead.saturating_add(64))
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            bounded("archive evidence leaves no historical comparison output allowance")
        })? as u64;

    let journal = session
        .journal
        .as_mut()
        .ok_or_else(|| invalid("comparison journal disappeared"))?;
    let before = journal.snapshot_at(
        first.number,
        first.record_digest,
        &remaining(current, started)?,
    )?;
    let mut source_admission = SourceAdmission::default();
    source_admission.admit(&before, current, started)?;
    let later = if first.number == last.number {
        None
    } else {
        let snapshot = journal.snapshot_at(
            last.number,
            last.record_digest,
            &remaining(current, started)?,
        )?;
        source_admission.admit(&snapshot, current, started)?;
        Some(snapshot)
    };
    let after = match &later {
        Some(snapshot) => snapshot,
        None => &before,
    };
    metadata["source_work"] = source_admission.metadata();
    let result = semantic_query::compare_endpoints_with_output_budget(
        &before,
        after,
        &remaining(current, started)?,
        &select,
        binding,
        limit,
        continuation.as_deref(),
        output_bytes,
    )?;
    publish_comparison(journal, current, started, result, metadata, &view, (archive_id, archive_head, profile))
}

fn publish_comparison(
    journal: &mut Journal,
    current: &OperationContext,
    started: Instant,
    mut result: Value,
    metadata: Value,
    view: &QueryResponseProjection,
    expected: (Digest32, Digest32, OperationsProfile),
) -> Result<String> {
    journal.validate_custody(&remaining(current, started)?)?;
    if (journal.id(), journal.head(), journal.profile()) != expected {
        return Err(error(ErrorCode::StaleAnchor, "comparison archive identity changed"));
    }
    let object = result.as_object_mut().ok_or_else(|| {
        error(
            ErrorCode::InternalInvariantViolation,
            "comparison result is not an object",
        )
    })?;
    let additions = metadata.as_object().ok_or_else(|| {
        error(
            ErrorCode::InternalInvariantViolation,
            "comparison metadata is not an object",
        )
    })?;
    object.extend(
        additions
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    remaining(current, started)?;
    let response =
        semantic_query::publish_with_active_work(current, result, |value| render(view, value))?;
    remaining(current, started)?;
    Ok(response)
}
