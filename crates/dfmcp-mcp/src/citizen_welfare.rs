//! Welfare indicators over one complete spatial/1.8 strict-citizen roster.
//! Stress categories, availability and assignments are observed indicators;
//! neither this report nor absence from its filtered rows proves good health.

use std::collections::BTreeMap;
use std::time::Instant;

use dfmcp_adapter::live_spatial::{SpatialStateView, citizens::LiveSpatialCitizenState};
use dfmcp_core::{
    Capability, DfmcpError, Digest32, EntityId, ErrorCode, OperationContext, Result, RiskTier,
    StateAnchor,
};
use dfmcp_world::{
    EntityKind, EntityRecord, Fact, FactPresence, FactSource, Value as W, WorldSnapshot,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    Page, anchor_json, budget, invalid, invariant, page_identity, paginate, validate_shape,
};

const POLICY: &str = "dfmcp.citizen-welfare/1";
const MAX_WORK: u64 = 4_000_000;
const MAX_ENTITIES: usize = 131_072;
const MAX_CITIZENS: usize = 4_096;
const MAX_JOB_EXAMPLES: usize = 4;
const STATUS_FIELDS: [&str; 9] = [
    "alive",
    "sane",
    "active",
    "visible",
    "adult",
    "child",
    "baby",
    "job_available_preserve_social",
    "job_available_interrupt_social",
];
const INSPECT_FIELDS: [&str; 12] = [
    "name",
    "stress_category",
    "position",
    "alive",
    "sane",
    "active",
    "visible",
    "adult",
    "child",
    "baby",
    "job_available_preserve_social",
    "job_available_interrupt_social",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    schema: String,
    expected_anchor: Option<Value>,
    query: Request,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    kind: String,
    max_stress_category: Option<u8>,
    include_unestablished: Option<bool>,
    limit: Option<u32>,
    continuation: Option<String>,
    max_work: Option<u64>,
}

struct Work<'a> {
    context: &'a OperationContext,
    started: Instant,
    maximum: u64,
    used: u64,
}

impl<'a> Work<'a> {
    fn new(context: &'a OperationContext, maximum: u64) -> Result<Self> {
        if maximum == 0 || maximum > MAX_WORK {
            return Err(budget("citizen welfare max_work must be 1..4000000"));
        }
        let mut work = Self {
            context,
            started: Instant::now(),
            maximum,
            used: 0,
        };
        work.charge(0)?;
        Ok(work)
    }

    fn charge(&mut self, units: u64) -> Result<()> {
        self.used = self
            .used
            .checked_add(units)
            .ok_or_else(|| budget("citizen welfare work overflow"))?;
        if self.used > self.maximum
            || self.started.elapsed().as_millis() >= u128::from(self.context.budget.max_wall_millis)
        {
            return Err(budget(
                "citizen welfare exhausted its work or wall allowance",
            ));
        }
        self.context
            .authorize(Capability::Query, RiskTier::ReadOnly, &[], None)
    }
}

fn source_matches(fact: &Fact, path: &str, anchor: StateAnchor, source: Digest32) -> bool {
    fact.observed_at == anchor.tick
        && fact.source_digest == source
        && matches!(&fact.source, FactSource::DfhackField(actual) if actual == path)
}

fn native<'a>(
    entity: &'a EntityRecord,
    name: &str,
    path: &str,
    anchor: StateAnchor,
    source: Digest32,
) -> Option<&'a W> {
    let fact = entity.fields.get(name)?;
    if !source_matches(fact, path, anchor, source) {
        return None;
    }
    match &fact.presence {
        None => Some(&fact.value),
        Some(FactPresence::Known(value)) if value == &fact.value => Some(value),
        _ => None,
    }
}

fn citizen_field<'a>(
    entity: &'a EntityRecord,
    name: &str,
    anchor: StateAnchor,
    source: Digest32,
) -> Option<&'a W> {
    let suffix = match name {
        "stress_category" => "citizen.stress_category",
        "name" => "citizen.name",
        "position" => "citizen.position",
        "job_available_preserve_social" | "job_available_interrupt_social" => {
            "citizen.job_available"
        }
        other => other,
    };
    native(
        entity,
        name,
        &format!("spatial/1.8.{suffix}"),
        anchor,
        source,
    )
}

fn boolean(
    entity: &EntityRecord,
    name: &str,
    anchor: StateAnchor,
    source: Digest32,
) -> Option<bool> {
    match citizen_field(entity, name, anchor, source) {
        Some(W::Bool(value)) => Some(*value),
        _ => None,
    }
}

fn stress_category(entity: &EntityRecord, anchor: StateAnchor, source: Digest32) -> Option<u8> {
    match citizen_field(entity, "stress_category", anchor, source) {
        Some(W::I64(value)) if (0..=6).contains(value) => Some(*value as u8),
        _ => None,
    }
}

fn handle(entity: &EntityRecord) -> Value {
    json!({"entity_id":entity.id.to_string(),"generation":entity.generation,"revision":entity.revision})
}

#[derive(Default)]
struct Jobs {
    count: usize,
    examples: Vec<Value>,
}

struct Report<'a> {
    snapshot: &'a WorldSnapshot,
    source: Digest32,
    selected: Vec<(&'a EntityRecord, Option<u8>)>,
    categories: [u64; 7],
    unknown_stress: usize,
    citizens: usize,
    available_preserving_social: usize,
    available_interrupting_social: usize,
    unknown_availability: usize,
    jobs: BTreeMap<EntityId, Jobs>,
    unresolved_job_workers: usize,
}

fn assess<'a>(
    snapshot: &'a WorldSnapshot,
    source: Digest32,
    maximum_category: u8,
    include_unestablished: bool,
    work: &mut Work<'_>,
) -> Result<Report<'a>> {
    if snapshot.anchor() != work.context.anchor {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "citizen welfare snapshot differs",
        ));
    }
    if snapshot.graph.entities.len() > MAX_ENTITIES
        || snapshot.graph.entities.len() > work.context.budget.max_entities as usize
    {
        return Err(budget("citizen welfare exceeds the entity-scan allowance"));
    }
    if !snapshot.hash_is_valid() {
        return Err(invariant("citizen welfare snapshot hash is invalid"));
    }
    work.charge(0)?;
    let anchor = snapshot.anchor();
    let mut report = Report {
        snapshot,
        source,
        selected: Vec::new(),
        categories: [0; 7],
        unknown_stress: 0,
        citizens: 0,
        available_preserving_social: 0,
        available_interrupting_social: 0,
        unknown_availability: 0,
        jobs: BTreeMap::new(),
        unresolved_job_workers: 0,
    };
    for (id, entity) in &snapshot.graph.entities {
        work.charge(1)?;
        if *id != entity.id {
            return Err(invariant("citizen welfare entity identity differs"));
        }
        if entity.kind == EntityKind::Unit {
            report.citizens += 1;
            if report.citizens > MAX_CITIZENS {
                return Err(budget("citizen welfare exceeds the complete roster bound"));
            }
            work.charge(4)?;
            let stress = stress_category(entity, anchor, source);
            if let Some(category) = stress {
                report.categories[usize::from(category)] += 1;
            } else {
                report.unknown_stress += 1;
            }
            if stress.is_some_and(|category| category <= maximum_category)
                || (stress.is_none() && include_unestablished)
            {
                report.selected.push((entity, stress));
            }
            match (
                boolean(entity, "job_available_preserve_social", anchor, source),
                boolean(entity, "job_available_interrupt_social", anchor, source),
            ) {
                (Some(true), Some(true)) => report.available_preserving_social += 1,
                (Some(false), Some(true)) => report.available_interrupting_social += 1,
                (Some(false), Some(false)) => {}
                // The preserve-social policy cannot admit a worker rejected
                // by the otherwise identical interrupt-social policy.
                _ => report.unknown_availability += 1,
            }
        } else if entity.kind == EntityKind::Job {
            work.charge(3)?;
            let worker = native(
                entity,
                "worker_entity",
                "spatial/1.8.Job.getWorker",
                anchor,
                source,
            );
            let strict = native(
                entity,
                "worker_is_strict_citizen",
                "spatial/1.8.Job.getWorker",
                anchor,
                source,
            );
            match (worker, strict) {
                (Some(W::Entity(worker)), Some(W::Bool(true)))
                    if snapshot
                        .graph
                        .entities
                        .get(worker)
                        .is_some_and(|u| u.kind == EntityKind::Unit) =>
                {
                    let jobs = report.jobs.entry(*worker).or_default();
                    jobs.count += 1;
                    if jobs.examples.len() < MAX_JOB_EXAMPLES {
                        jobs.examples.push(handle(entity));
                    }
                }
                // Outside-roster workers and native absence cannot be charged
                // to an arbitrary citizen. Their exact false membership fact
                // still establishes that they are not strict-citizen jobs.
                (None, Some(W::Bool(false)))
                    if entity.fields.get("worker_entity").is_some_and(|fact| {
                        source_matches(fact, "spatial/1.8.Job.getWorker", anchor, source)
                            && matches!(
                                &fact.presence,
                                Some(FactPresence::Absent | FactPresence::Unknown(_))
                            )
                    }) => {}
                _ => report.unresolved_job_workers += 1,
            }
        }
    }
    // At most 4096 citizens; charge the bounded sort comparisons conservatively.
    work.charge((report.selected.len() as u64).saturating_mul(13))?;
    report
        .selected
        .sort_by_key(|(entity, stress)| (stress.unwrap_or(7), entity.id));
    Ok(report)
}

impl Report<'_> {
    fn summary(&self) -> Value {
        json!({"strict_citizens":self.citizens,"stress_categories":self.categories,
            "stress_unestablished":self.unknown_stress,
            "elevated_stress_category_0_to_2":self.categories[..3].iter().sum::<u64>(),
            "selected_citizens":self.selected.len(),
            "job_available_preserving_social":self.available_preserving_social,
            "job_available_only_interrupting_social":self.available_interrupting_social,
            "job_availability_unestablished":self.unknown_availability,
            "unresolved_job_worker_links":self.unresolved_job_workers})
    }

    fn row(&self, index: usize) -> Result<Value> {
        let (entity, stress) = self
            .selected
            .get(index)
            .ok_or_else(|| invariant("citizen welfare row is absent"))?;
        let anchor = self.snapshot.anchor();
        let mut observed = serde_json::Map::new();
        let mut unknown = Vec::new();
        for field in STATUS_FIELDS {
            let value = boolean(entity, field, anchor, self.source);
            if value.is_none() {
                unknown.push(field);
            }
            observed.insert(field.to_owned(), json!(value));
        }
        if stress.is_none() {
            unknown.push("stress_category");
        }
        let name = match citizen_field(entity, "name", anchor, self.source) {
            Some(W::Text(value)) if value.len() <= 256 => Some(value),
            _ => {
                unknown.push("name");
                None
            }
        };
        let position = match citizen_field(entity, "position", anchor, self.source) {
            Some(W::Coord(p)) => Some([p.x, p.y, p.z]),
            _ => {
                unknown.push("position");
                None
            }
        };
        let jobs = self.jobs.get(&entity.id);
        let count = jobs.map_or(0, |jobs| jobs.count);
        let examples = jobs.map_or_else(Vec::new, |jobs| jobs.examples.clone());
        let availability = match (
            boolean(entity, "job_available_preserve_social", anchor, self.source),
            boolean(
                entity,
                "job_available_interrupt_social",
                anchor,
                self.source,
            ),
        ) {
            (Some(true), Some(true)) => "available_preserving_social",
            (Some(false), Some(true)) => "available_only_interrupting_social",
            (Some(false), Some(false)) => "unavailable",
            _ => "unknown",
        };
        Ok(
            json!({"citizen":handle(entity),"name":name,"position":position,
            "stress_category":stress,"stress_evidence":if stress.is_some(){"observed"}else{"unknown"},
            "observed_status":observed,"job_availability":availability,"unestablished_fields":unknown,
            "current_job_assignments":{"observed_count":count,"examples":examples,
                "omitted_examples":count.saturating_sub(MAX_JOB_EXAMPLES),
                "no_assignment_proven":count==0 && self.unresolved_job_workers==0},
            "inspect_query":{"schema":"dfmcp.query/1","expected_anchor":anchor_json(anchor),
                "query":{"kind":"inspect","entity_id":entity.id.to_string(),
                    "generation":entity.generation,"fields":INSPECT_FIELDS}}}),
        )
    }
}

pub(super) fn execute(
    state: &LiveSpatialCitizenState,
    context: &OperationContext,
    input: &Value,
) -> Result<Value> {
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    validate_shape(input)?;
    let envelope: Envelope = serde_json::from_value(input.clone())
        .map_err(|_| invalid("invalid citizen_welfare query fields"))?;
    if envelope.schema != "dfmcp.query/1" || envelope.query.kind != "citizen_welfare" {
        return Err(invalid("citizen_welfare requires dfmcp.query/1"));
    }
    let request = envelope.query;
    let maximum_category = request.max_stress_category.unwrap_or(2);
    if maximum_category > 6 {
        return Err(invalid("stress category must be 0..6"));
    }
    let include_unestablished = request.include_unestablished.unwrap_or(true);
    let limit = request.limit.unwrap_or(8);
    if !(1..=128).contains(&limit) {
        return Err(budget("citizen welfare limit must be 1..128"));
    }
    if envelope
        .expected_anchor
        .as_ref()
        .is_some_and(|anchor| *anchor != anchor_json(context.anchor))
    {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "citizen welfare expected anchor differs",
        ));
    }
    let maximum_work = request.max_work.unwrap_or(MAX_WORK);
    let mut work = Work::new(context, maximum_work)?;
    let snapshot = state
        .snapshot()
        .ok_or_else(|| invariant("citizen welfare snapshot absent"))?;
    let source = state.source_digest()?;
    let report = assess(
        snapshot,
        source,
        maximum_category,
        include_unestablished,
        &mut work,
    )?;
    let original = state
        .observation_full()
        .ok_or_else(|| invariant("citizen welfare source absent"))?;
    if report.citizens != original.citizens().len() {
        return Err(invariant("citizen welfare canonical roster is incomplete"));
    }
    let model = Digest32::of_bytes(
        json!({"domain":POLICY,"anchor":anchor_json(context.anchor),
        "source":source.to_string(),"max_stress_category":maximum_category,
        "include_unestablished":include_unestablished})
        .to_string()
        .as_bytes(),
    );
    // Charge every candidate row, including rows deferred by byte pagination,
    // so a narrow page cannot hide an unbounded disclosure/model pass.
    work.charge((report.selected.len() as u64).saturating_mul(16))?;
    let out = json!({"schema":"dfmcp.query.result/1","kind":"citizen_welfare",
        "anchor":anchor_json(context.anchor),"source_digest":source.to_string(),
        "source_profile":"spatial/1.8","policy":POLICY,"model_digest":model.to_string(),
        "selection":{"max_stress_category":maximum_category,"include_unestablished":include_unestablished},
        "summary":report.summary(),"work_units":work.used,
        "ordering":"stress_category_ascending_unknown_last_then_entity_id",
        "coverage":{"strict_citizen_roster":"complete_at_capture","stress_category_order":"0_highest_to_6_lowest",
            "needs":"unknown","physical_health":"unknown","hunger":"unknown","thirst":"unknown","injuries":"unknown",
            "raw_stress_value":"not_observed","soul_presence":"not_observed","other_population":"not_observed"},
        "interpretation":"Stress categories and current availability/assignments are indicators, not causes, health diagnoses or labor permission. Category 3 may be DFHack's no-soul fallback. Strict roster excludes residents, visitors, dead and insane units; no whole-population wellbeing claim.",
        "all_clear_proven":false,"native_captures":0,"mutation_dispatched":false});
    let out = paginate(
        out,
        report.selected.len(),
        context,
        Page {
            limit,
            continuation: request.continuation.as_deref(),
            prefix: "cw1",
            identity: page_identity(context, model, maximum_work),
        },
        |index| report.row(index),
    )?;
    work.charge(0)?;
    Ok(out)
}

/// A welfare drill-down from an archive must inspect that same immutable record,
/// even after the live session advances. The historical wrapper fits the space
/// already reserved for the complete current-anchor wrapper.
pub(super) fn pin_historical(out: &mut Value, record: u64, digest: Digest32) -> Result<()> {
    if out.get("kind").and_then(Value::as_str) != Some("citizen_welfare") {
        return Ok(());
    }
    if let Some(rows) = out.get_mut("rows").and_then(Value::as_array_mut) {
        for row in rows {
            let inspect = row
                .get("inspect_query")
                .ok_or_else(|| invariant("welfare drill-down absent"))?;
            let query = inspect
                .get("query")
                .filter(|q| q["kind"] == "inspect")
                .ok_or_else(|| invariant("welfare drill-down is not an inspection"))?;
            let wrapped = json!({"schema":"dfmcp.query/1","query":{"kind":"historical_query",
                "record":record,"record_digest":digest.to_string(),"query":query}});
            if wrapped.to_string().len() > inspect.to_string().len() {
                return Err(invariant(
                    "historical welfare inspection exceeds its reserved row",
                ));
            }
            row["inspect_query"] = wrapped;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "citizen_welfare_tests.rs"]
mod tests;
