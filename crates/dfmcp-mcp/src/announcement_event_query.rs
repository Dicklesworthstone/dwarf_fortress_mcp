//! Read-only event views over the existing protocol-1.1 announcement entities.
//!
//! This deliberately does not insert WorldEvents, change the canonical world
//! projection, or assign new hashes to old capsules. Retention metadata is not
//! encoded by an Announcement entity; only the owning live runtime can attach
//! its exact already-published batch bounds to the result.

use super::*;
use dfmcp_world::{EntityKind, EntityRecord, Fact, FactSource};

use super::super::core_query;

#[path = "spatial_query_source.rs"]
#[allow(clippy::duplicate_mod)]
mod source;

const PROJECTION: &str = "dfmcp.announcement-event-window/1";
const IDENTITY_FIELDS: [&str; 3] = ["report_id", "year", "year_tick"];

pub(super) struct Window<'a> {
    pub(super) selection: &'a Selection,
    pub(super) limit: usize,
    pub(super) continuation: Option<&'a str>,
    pub(super) started: Instant,
    pub(super) max_output_bytes: u64,
    pub(super) fits: &'a dyn Fn(&Value) -> Result<bool>,
}

struct Report<'a> {
    entity: &'a EntityRecord,
    report_id: i32,
    tick: u64,
    event_id: EventId,
    batch: Digest32,
    observed_at: dfmcp_core::GameTick,
}

fn native_fact<'a>(
    entity: &'a EntityRecord,
    name: &str,
    tick: dfmcp_core::GameTick,
) -> Option<&'a Fact> {
    let fact = entity.fields.get(name)?;
    if fact.observed_at > tick
        || fact.source_digest == Digest32::ZERO
        || !matches!(&fact.source, FactSource::DfhackField(source)
            if source == &format!("dfmcp_bridge_v1_1.ReadObservation.announcement.{name}"))
    {
        return None;
    }
    fact.known_value()?;
    Some(fact)
}

fn integer(fact: &Fact) -> Option<i32> {
    match fact.known_value()? {
        WorldValue::I64(value) => i32::try_from(*value).ok(),
        _ => None,
    }
}

fn report<'a>(entity: &'a EntityRecord, snapshot: &WorldSnapshot) -> Option<Report<'a>> {
    let id = native_fact(entity, "report_id", snapshot.tick)?;
    let year = native_fact(entity, "year", snapshot.tick)?;
    let year_tick = native_fact(entity, "year_tick", snapshot.tick)?;
    if entity.generation == 0
        || year.source_digest != id.source_digest
        || year_tick.source_digest != id.source_digest
        || year.observed_at != id.observed_at
        || year_tick.observed_at != id.observed_at
    {
        return None;
    }
    let report_id = integer(id)?;
    if dfmcp_adapter::announcement_entity_id_to_report_id(entity.id) != Some(report_id) {
        return None;
    }
    let tick = dfmcp_adapter::DwarfFortressClock {
        year: u32::try_from(integer(year)?).ok()?,
        year_tick: u32::try_from(integer(year_tick)?).ok()?,
    }
    .absolute_tick()
    .ok()?;
    if tick > id.observed_at || tick > snapshot.tick {
        return None;
    }
    // Collision-free within the selected source: all 64 entity-ID bits and
    // all 32 generation bits survive. The anchor identifies the fortress and
    // epoch; the separate query domain prevents aliasing canonical WorldEvents.
    let event_id =
        EventId::new((u128::from(entity.generation) << 64) | u128::from(entity.id.get()));
    Some(Report {
        entity,
        report_id,
        tick: tick.0,
        event_id,
        batch: id.source_digest,
        observed_at: id.observed_at,
    })
}

impl Report<'_> {
    fn native(&self, name: &str) -> Option<&Fact> {
        native_fact(self.entity, name, self.observed_at).filter(|fact| {
            fact.source_digest == self.batch && fact.observed_at == self.observed_at
        })
    }

    fn field(&self, name: &str) -> Value {
        match self.entity.fields.get(name) {
            Some(fact) => {
                let mut value = core_query::fact_json(fact);
                if fact.known_value().is_some() && self.native(name).is_none() {
                    value["presence"] = json!("unknown");
                    value["epistemic_state"] = json!("unknown");
                    value["value"] = Value::Null;
                    value["reason"] = json!(
                        "field does not share this report's protocol source, batch and observation tick"
                    );
                }
                value
            }
            None => json!({"presence":"unknown", "epistemic_state":"unknown", "value":null,
                "reason":"field is not present in the retained announcement projection"}),
        }
    }

    fn text(&self) -> Option<&str> {
        match self.native("text")?.known_value()? {
            WorldValue::Text(text) => Some(text),
            _ => None,
        }
    }

    fn row(&self, fields: &BTreeSet<String>) -> Value {
        let projected: BTreeMap<_, _> = fields
            .iter()
            .map(|name| (name, self.field(name)))
            .collect();
        let identity: BTreeMap<_, _> = IDENTITY_FIELDS
            .iter()
            .map(|name| (*name, self.field(name)))
            .collect();
        json!({"event_id":self.event_id.to_string(), "tick":self.tick,
            "kind":"announcement", "subject":null, "summary":self.text(),
            "summary_presence":if self.text().is_some() {"known"} else {"unknown"},
            "entity_id":self.entity.id.to_string(), "generation":self.entity.generation,
            "revision":self.entity.revision, "report_id":self.report_id,
            "source_batch_digest":self.batch.to_string(),
            "identity_evidence":identity, "fields":projected,
            "epistemic_state":"inferred", "taint":"untrusted_source_text"})
    }
}

fn page(
    base: &Value,
    rows: &[Value],
    matching: &[Report<'_>],
    start: usize,
    identity: Digest32,
) -> Value {
    let end = start + rows.len();
    let truncated = end < matching.len();
    let continuation = if truncated {
        end.checked_sub(1)
            .and_then(|offset| matching.get(offset))
            .map(|report| cursor(identity, (report.tick, report.event_id)))
    } else {
        None
    };
    let mut result = base.clone();
    result["rows"] = json!(rows);
    result["returned"] = json!(rows.len());
    result["truncated"] = json!(truncated);
    result["continuation"] = json!(continuation);
    result["coverage"]["page_complete"] = json!(start == 0 && end == matching.len());
    result
}

pub(super) fn execute(
    snapshot: &WorldSnapshot,
    context: &OperationContext,
    request: Window<'_>,
) -> Result<Value> {
    let spec = request.selection;
    let mut work = source::preflight(snapshot, context, request.started)?;
    if !snapshot.hash_is_valid() {
        return Err(DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "announcement query canonical snapshot hash is invalid",
        ));
    }
    work.check()?;
    let identity = identity(snapshot, context, spec);
    let after = parse_cursor(request.continuation, identity)?;
    let maximum = usize::try_from(
        context.budget.max_bytes.min(request.max_output_bytes).min(
            u64::from(context.budget.max_output_tokens).saturating_mul(4),
        ),
    )
    .map_err(|_| exhausted("announcement query output bound cannot be represented"))?;
    let mut matching = Vec::new();
    let mut retained = 0usize;
    let mut unestablished = 0usize;
    let mut unknown_subjects = 0usize;
    let mut batches = BTreeSet::new();
    let (mut min_tick, mut max_tick) = (None, None);
    for entity in snapshot.graph.entities.values() {
        work.charge(0, 1)?;
        // Canonical encoding uses kind names, so a reserved-name alias must
        // produce the same read view under an identical canonical hash.
        if entity.kind.as_str() != EntityKind::Announcement.as_str() {
            continue;
        }
        retained += 1;
        work.charge(0, IDENTITY_FIELDS.len() * 4)?;
        let Some(report) = report(entity, snapshot) else {
            unestablished += 1;
            continue;
        };
        batches.insert(report.batch.to_string());
        min_tick = Some(min_tick.map_or(report.tick, |tick: u64| tick.min(report.tick)));
        max_tick = Some(max_tick.map_or(report.tick, |tick: u64| tick.max(report.tick)));
        if report.tick < spec.from_tick
            || report.tick > spec.through_tick
            || (!spec.kinds.is_empty() && !spec.kinds.contains("announcement"))
        {
            continue;
        }
        // Native report text does not identify a canonical subject. In
        // particular, a name in its text cannot become an entity-ID join.
        if !spec.subjects.is_empty() {
            unknown_subjects += 1;
            continue;
        }
        matching.push(report);
    }
    work.charge(
        0,
        matching.len().saturating_mul(
            usize::try_from(matching.len().max(1).ilog2()).map_or(usize::MAX, |n| n + 1),
        ),
    )?;
    matching.sort_by_key(|report| (report.tick, report.event_id));
    if spec.order == Order::TickDescending {
        matching.reverse();
    }
    let start = match after {
        None => 0,
        Some(position) => matching
            .iter()
            .position(|report| (report.tick, report.event_id) == position)
            .map(|offset| offset + 1)
            .filter(|offset| *offset < matching.len())
            .ok_or_else(|| stale("announcement continuation does not name a resumable report"))?,
    };
    let anchor = core_query::anchor_json(snapshot.anchor());
    let base = json!({"schema":"dfmcp.query.result/1", "kind":"events",
        "source":"announcements", "projection":PROJECTION, "anchor":anchor,
        "query_digest":identity.to_string(), "matched":matching.len(),
        "order":if spec.order == Order::TickAscending {"tick_ascending"} else {"tick_descending"},
        "provenance":{"kind":"canonical_announcement_entity_view", "source_anchor":anchor,
            "record_identity":"announcement_entity_id_and_generation",
            "source_batch_digests":batches, "source_authenticity":"not_established",
            "field_provenance":"retained_canonical_facts", "taint":"untrusted_source_text"},
        "coverage":{"domain":"retained_announcement_entity_projection",
            "from_tick":spec.from_tick, "through_tick":spec.through_tick, "kinds":spec.kinds,
            "subjects":spec.subjects.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "fields":spec.fields, "retained_records":retained, "retained_min_tick":min_tick,
            "retained_max_tick":max_tick, "unestablished_identity_or_clock":unestablished,
            "unestablished_subject_selection":unknown_subjects,
            "selection_complete_within_projection":unestablished == 0 && unknown_subjects == 0,
            "native_retained_window":null, "retained_suffix_complete":"unknown",
            "history_complete":false, "absence_proven":false,
            "eligible_as_mutation_evidence":false,
            "note":"announcement entities alone do not encode retained-window bounds or gaps; only the owning runtime can attach its exact published batch; reports do not identify subjects, causal events, complete history or production admission"},
        "work":{"source_bytes":work.bytes,"source_work":work.work,"scanned_entities":snapshot.graph.entities.len()}});
    let output = core_query::PageBudget {
        maximum_bytes: maximum,
        fits: request.fits,
    };
    let fields = spec.fields.iter().cloned().collect::<Vec<_>>();
    let mandatory = IDENTITY_FIELDS
        .iter()
        .copied()
        .chain(["text"])
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let reaches_terminal = matching.len().saturating_sub(start) <= request.limit;
    let mut rows = Vec::new();
    let mut accepted = None;
    for report in matching.iter().skip(start).take(request.limit) {
        if !source::render_fits(report.entity, &fields, &mandatory, maximum, &mut work)? {
            break;
        }
        work.charge(0, fields.len().saturating_add(mandatory.len()))?;
        rows.push(report.row(&spec.fields));
        let candidate = page(&base, &rows, &matching, start, identity);
        if output.accepts(&candidate)? {
            accepted = Some(candidate);
        } else if !output.terminal_may_fit(&candidate, reaches_terminal)? {
            break;
        }
        work.check()?;
    }
    let result = accepted
        .or_else(|| (start == matching.len()).then(|| page(&base, &[], &matching, start, identity)))
        .ok_or_else(|| exhausted("one complete announcement row and its envelope exceed the output budget"))?;
    if !output.accepts(&result)? {
        return Err(exhausted("announcement event metadata exceeds the output budget"));
    }
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    work.check()?;
    Ok(result)
}

#[cfg(test)]
#[path = "announcement_event_query_tests.rs"]
mod tests;
