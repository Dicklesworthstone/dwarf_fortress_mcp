//! Source-qualified citizen indicators at two retained spatial/1.8 endpoints.
//! Category movement is not a measured raw-stress change, continuous trend,
//! health diagnosis or explanation of why a citizen left the strict roster.
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use dfmcp_adapter::live_spatial::{
    SpatialStateView,
    citizens::{LiveSpatialCitizenState, citizen_entity_id},
};
use dfmcp_core::{
    Capability, DfmcpError, Digest32, EntityId, ErrorCode, OperationContext, Result, RiskTier,
    StateAnchor,
};
use dfmcp_world::{
    EntityKind, EntityRecord, Fact, FactPresence, FactSource, Value as W, WorldSnapshot,
};
use serde::Deserialize;
use serde_json::{Value, json};

pub(super) const POLICY: &str = "dfmcp.historical-citizen-welfare/1";
pub(super) const MAX_WORK: u64 = 4_000_000;
const MAX_CITIZENS: usize = 4_096;
const MAX_ENTITIES: usize = 131_072;
const MAX_EDGES: usize = 262_144;
const STATUS_FIELDS: [&str; 7] = [
    "alive", "sane", "active", "visible", "adult", "child", "baby",
];

fn invalid(text: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, text)
}
fn bounded(text: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, text)
}
fn invariant(text: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InternalInvariantViolation, text)
}
pub(super) fn anchor_json(a: StateAnchor) -> Value {
    json!({"fortress_id":a.fortress_id.to_string(),"epoch":a.cursor.epoch,"sequence":a.cursor.sequence,
        "game_tick":a.tick.0,"state_hash":a.state_hash.to_string()})
}

struct Work<'a> {
    context: &'a OperationContext,
    started: Instant,
    maximum: u64,
    used: u64,
}
impl<'a> Work<'a> {
    fn new(context: &'a OperationContext, maximum: u64) -> Result<Self> {
        if !(1..=MAX_WORK).contains(&maximum) {
            return Err(bounded(
                "citizen welfare history max_work must be 1..4000000",
            ));
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
            .ok_or_else(|| bounded("citizen welfare history work overflow"))?;
        if self.used > self.maximum
            || self.started.elapsed().as_millis() >= u128::from(self.context.budget.max_wall_millis)
        {
            return Err(bounded(
                "citizen welfare history exhausted work or wall allowance",
            ));
        }
        self.context
            .authorize(Capability::Query, RiskTier::ReadOnly, &[], None)
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Focus {
    pub entity_id: String,
    pub generation: u32,
}
impl Focus {
    pub fn validate(&self) -> Result<()> {
        if self.entity_id.is_empty()
            || self.entity_id.len() > 20
            || self.entity_id.starts_with('0')
            || !self.entity_id.bytes().all(|b| b.is_ascii_digit())
            || self.entity_id.parse::<u64>().is_err()
        {
            return Err(invalid(
                "citizen focus requires a canonical positive decimal u64 entity_id",
            ));
        }
        Ok(())
    }
    pub fn json(&self) -> Value {
        json!({"entity_id":self.entity_id,"generation":self.generation})
    }
}

struct Citizen {
    id: EntityId,
    generation: u32,
    revision: u64,
    native_id: Option<u32>,
    name: Option<String>,
    category: Option<u8>,
    availability: Option<&'static str>,
    status: BTreeMap<&'static str, Option<bool>>,
}
impl Citizen {
    fn json(&self) -> Value {
        let mut missing: Vec<&str> = self
            .status
            .iter()
            .filter_map(|(name, value)| value.is_none().then_some(*name))
            .collect();
        for (name, known) in [
            ("native_unit_id", self.native_id.is_some()),
            ("name", self.name.is_some()),
            ("stress_category", self.category.is_some()),
            ("job_availability", self.availability.is_some()),
        ] {
            if !known {
                missing.push(name);
            }
        }
        json!({"citizen":{"entity_id":self.id.to_string(),"generation":self.generation,"revision":self.revision},
            "native_unit_id":self.native_id,"name":self.name,"stress_category":self.category,
            "stress_evidence":if self.category.is_some(){"observed_category"}else{"unestablished"},
            "category_may_be_no_soul_fallback":self.category.map(|v|v==3),
            "job_availability":self.availability,"observed_status":self.status,"unestablished_fields":missing})
    }
}

/// Only the original native roster supplies a completeness witness. A test
/// endpoint without that witness may compare positive facts, never absence.
struct Endpoint<'a> {
    snapshot: &'a WorldSnapshot,
    source: Digest32,
    roster: Option<BTreeSet<EntityId>>,
}
impl<'a> Endpoint<'a> {
    fn state(state: &'a LiveSpatialCitizenState) -> Result<Self> {
        let snapshot = state
            .snapshot()
            .ok_or_else(|| invariant("welfare history snapshot absent"))?;
        let original = state
            .observation_full()
            .ok_or_else(|| invariant("welfare history source absent"))?;
        Ok(Self {
            snapshot,
            source: state.source_digest()?,
            roster: Some(
                original
                    .citizens()
                    .iter()
                    .map(|c| citizen_entity_id(c.native_id))
                    .collect(),
            ),
        })
    }
    fn native<'b>(&self, entity: &'b EntityRecord, key: &str, suffix: &str) -> Option<&'b W> {
        let fact: &Fact = entity.fields.get(key)?;
        if fact.observed_at != self.snapshot.anchor().tick
            || fact.source_digest != self.source
            || !matches!(&fact.source, FactSource::DfhackField(path) if path.strip_prefix("spatial/1.8.")==Some(suffix))
        {
            return None;
        }
        match &fact.presence {
            None => Some(&fact.value),
            Some(FactPresence::Known(v)) if v == &fact.value => Some(v),
            _ => None,
        }
    }
    fn boolean(&self, entity: &EntityRecord, key: &str, suffix: &str) -> Option<bool> {
        match self.native(entity, key, suffix) {
            Some(W::Bool(v)) => Some(*v),
            _ => None,
        }
    }
    fn collect(&self, work: &mut Work<'_>) -> Result<BTreeMap<EntityId, Citizen>> {
        let graph = &self.snapshot.graph;
        if graph.entities.len() > MAX_ENTITIES
            || graph.edges.len() > MAX_EDGES
            || graph.entities.len() > work.context.budget.max_entities as usize
            || self.roster.as_ref().is_some_and(|r| r.len() > MAX_CITIZENS)
        {
            return Err(bounded(
                "citizen welfare history exceeds the complete scan bound",
            ));
        }
        work.charge(graph.entities.len() as u64 + graph.edges.len() as u64)?;
        if !self.snapshot.hash_is_valid() {
            return Err(invariant("welfare history snapshot hash invalid"));
        }
        let mut result = BTreeMap::new();
        for (id, entity) in &graph.entities {
            work.charge(1)?;
            if *id != entity.id {
                return Err(invariant("welfare history entity identity differs"));
            }
            if entity.kind != EntityKind::Unit {
                continue;
            }
            if result.len() == MAX_CITIZENS {
                return Err(bounded("welfare history citizen bound exceeded"));
            }
            work.charge(16)?;
            let native_id = match self.native(entity, "native_unit_id", "citizen.unit_id") {
                Some(W::U64(v)) => u32::try_from(*v)
                    .ok()
                    .filter(|v| *v <= i32::MAX as u32 && citizen_entity_id(*v) == *id),
                _ => None,
            };
            let name = match self.native(entity, "name", "citizen.name") {
                Some(W::Text(v)) if v.len() <= 256 => Some(v.clone()),
                _ => None,
            };
            let category = match self.native(entity, "stress_category", "citizen.stress_category") {
                Some(W::I64(v)) if (0..=6).contains(v) => Some(*v as u8),
                _ => None,
            };
            let availability = match (
                self.boolean(
                    entity,
                    "job_available_preserve_social",
                    "citizen.job_available",
                ),
                self.boolean(
                    entity,
                    "job_available_interrupt_social",
                    "citizen.job_available",
                ),
            ) {
                (Some(true), Some(true)) => Some("available_preserving_social"),
                (Some(false), Some(true)) => Some("available_only_interrupting_social"),
                (Some(false), Some(false)) => Some("unavailable"),
                _ => None,
            };
            let status = STATUS_FIELDS
                .into_iter()
                .map(|name| (name, self.boolean(entity, name, name)))
                .collect();
            result.insert(
                *id,
                Citizen {
                    id: *id,
                    generation: entity.generation,
                    revision: entity.revision,
                    native_id,
                    name,
                    category,
                    availability,
                    status,
                },
            );
        }
        if self
            .roster
            .as_ref()
            .is_some_and(|r| !r.iter().copied().eq(result.keys().copied()))
        {
            return Err(invariant(
                "welfare history canonical and native citizen rosters differ",
            ));
        }
        work.charge(0)?;
        Ok(result)
    }
}

fn difference<T: PartialEq>(before: &Option<T>, after: &Option<T>) -> &'static str {
    match (before, after) {
        (Some(a), Some(b)) if a == b => "unchanged",
        (Some(_), Some(_)) => "changed",
        _ => "unestablished",
    }
}
fn category_change(before: &Citizen, after: &Citizen) -> &'static str {
    if before.native_id.is_none() || before.native_id != after.native_id {
        return "unestablished";
    }
    match (before.category, after.category) {
        (Some(a), Some(b)) if a > b => "toward_higher_stress_category",
        (Some(a), Some(b)) if a < b => "toward_lower_stress_category",
        (Some(_), Some(_)) => "unchanged",
        _ => "unestablished",
    }
}
fn elevated_change(before: &Citizen, after: &Citizen) -> &'static str {
    if before.native_id.is_none() || before.native_id != after.native_id {
        return "unestablished";
    }
    match (
        before.category.map(|v| v <= 2),
        after.category.map(|v| v <= 2),
    ) {
        (Some(false), Some(true)) => "entered_elevated_category",
        (Some(true), Some(false)) => "left_elevated_category",
        (Some(true), Some(true)) => "elevated_at_both_endpoints",
        (Some(false), Some(false)) => "outside_elevated_categories_at_both_endpoints",
        _ => "unestablished",
    }
}
fn indicators(before: &Citizen, after: &Citizen) -> BTreeMap<&'static str, &'static str> {
    let identity = before.native_id.is_some() && before.native_id == after.native_id;
    let mut result = BTreeMap::from([
        ("stress_category", category_change(before, after)),
        (
            "job_availability",
            if identity {
                difference(&before.availability, &after.availability)
            } else {
                "unestablished"
            },
        ),
    ]);
    for name in STATUS_FIELDS {
        result.insert(
            name,
            if identity {
                difference(
                    &before.status.get(name).copied().flatten(),
                    &after.status.get(name).copied().flatten(),
                )
            } else {
                "unestablished"
            },
        );
    }
    result
}
fn changed(values: &BTreeMap<&str, &str>) -> bool {
    values.values().any(|v| {
        matches!(
            *v,
            "changed" | "toward_higher_stress_category" | "toward_lower_stress_category"
        )
    })
}
fn unchanged(values: &BTreeMap<&str, &str>) -> bool {
    values.values().all(|v| *v == "unchanged")
}

pub(super) struct Report {
    before: BTreeMap<EntityId, Citizen>,
    after: BTreeMap<EntityId, Citizen>,
    selected: Vec<EntityId>,
    before_complete: bool,
    after_complete: bool,
    pub basis: StateAnchor,
    pub anchor: StateAnchor,
    pub before_source: Digest32,
    pub after_source: Digest32,
    pub summary: Value,
    pub work_units: u64,
}
impl Report {
    pub fn len(&self) -> usize {
        self.selected.len()
    }
    fn identity_status(&self, id: EntityId) -> &'static str {
        match (self.before.get(&id), self.after.get(&id)) {
            (Some(a), Some(b)) if a.generation == b.generation => "same_generation",
            (Some(_), Some(_)) => "generation_replaced",
            (None, Some(_)) if self.before_complete => "newly_observed",
            (Some(_), None) if self.after_complete => "no_longer_observed",
            _ => "roster_absence_unestablished",
        }
    }
    pub fn row(&self, index: usize, from: &Value, to: &Value) -> Result<Value> {
        let id = *self
            .selected
            .get(index)
            .ok_or_else(|| invariant("welfare history row index out of bounds"))?;
        let before = self.before.get(&id);
        let after = self.after.get(&id);
        let status = self.identity_status(id);
        let delta = match (before, after) {
            (Some(a), Some(b)) if status == "same_generation" => Some(indicators(a, b)),
            _ => None,
        };
        let elevated = match (before, after) {
            (Some(a), Some(b)) if status == "same_generation" => elevated_change(a, b),
            _ => "not_same_citizen_generation",
        };
        Ok(json!({"entity_id":id.to_string(),"identity_status":status,
            "before":before.map(Citizen::json),"after":after.map(Citizen::json),"indicators":delta,
            "elevated_category_transition":elevated,
            "observed_indicator_change":delta.as_ref().is_some_and(changed),
            "all_compared_indicators_unchanged":delta.as_ref().is_some_and(unchanged),
            "inspect_before":before.map(|c|inspect(c,from)),"inspect_after":after.map(|c|inspect(c,to))}))
    }
}
fn inspect(citizen: &Citizen, entry: &Value) -> Value {
    json!({"schema":"dfmcp.query/1","query":{"kind":"historical_query","record":entry["record"],"record_digest":entry["record_digest"],
        "query":{"kind":"inspect","entity_id":citizen.id.to_string(),"generation":citizen.generation,
            "fields":["native_unit_id","name","stress_category","alive","sane","active","visible","adult","child","baby",
                "job_available_preserve_social","job_available_interrupt_social"]}}})
}
fn population(citizens: &BTreeMap<EntityId, Citizen>) -> Value {
    let mut categories = [0u64; 7];
    let mut unknown = 0u64;
    let mut availability = BTreeMap::from([
        ("available_preserving_social", 0u64),
        ("available_only_interrupting_social", 0),
        ("unavailable", 0),
        ("unestablished", 0),
    ]);
    for citizen in citizens.values() {
        if let Some(category) = citizen.category {
            categories[usize::from(category)] += 1;
        } else {
            unknown += 1;
        }
        *availability
            .entry(citizen.availability.unwrap_or("unestablished"))
            .or_default() += 1;
    }
    json!({"strict_citizens":citizens.len(),"stress_categories":categories,"stress_unestablished":unknown,
        "elevated_stress_category_0_to_2":categories[..3].iter().sum::<u64>(),"job_availability":availability})
}

fn assess(
    before: Endpoint<'_>,
    after: Endpoint<'_>,
    context: &OperationContext,
    focus: Option<&Focus>,
    include_unchanged: bool,
    maximum: u64,
) -> Result<Report> {
    let mut work = Work::new(context, maximum)?;
    let basis = before.snapshot.anchor();
    let anchor = after.snapshot.anchor();
    if basis.fortress_id != anchor.fortress_id
        || context.anchor.fortress_id != basis.fortress_id
        || basis.cursor.epoch != anchor.cursor.epoch
        || anchor.tick < basis.tick
        || anchor.cursor.sequence < basis.cursor.sequence
    {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "welfare history crosses fortress, epoch or clock/sequence reset",
        ));
    }
    let mut report = Report {
        before: before.collect(&mut work)?,
        after: after.collect(&mut work)?,
        selected: Vec::new(),
        before_complete: before.roster.is_some(),
        after_complete: after.roster.is_some(),
        basis,
        anchor,
        before_source: before.source,
        after_source: after.source,
        summary: Value::Null,
        work_units: 0,
    };
    let focus_id = if let Some(focus) = focus {
        focus.validate()?;
        let found = report
            .before
            .iter()
            .find(|(id, _)| id.to_string() == focus.entity_id)
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::StaleAnchor,
                    "focused citizen absent from basis record",
                )
            })?;
        if found.1.generation != focus.generation {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "focused citizen generation differs from basis record",
            ));
        }
        Some(*found.0)
    } else {
        None
    };
    let ids: BTreeSet<_> = report
        .before
        .keys()
        .chain(report.after.keys())
        .copied()
        .collect();
    let mut counts = BTreeMap::from([
        ("same_generation", 0u64),
        ("generation_replaced", 0),
        ("newly_observed", 0),
        ("no_longer_observed", 0),
        ("roster_absence_unestablished", 0),
    ]);
    let mut categories = BTreeMap::from([
        ("toward_higher_stress_category", 0u64),
        ("toward_lower_stress_category", 0),
        ("unchanged", 0),
        ("unestablished", 0),
    ]);
    let mut transitions = BTreeMap::from([
        ("entered_elevated_category", 0u64),
        ("left_elevated_category", 0),
        ("elevated_at_both_endpoints", 0),
        ("outside_elevated_categories_at_both_endpoints", 0),
        ("unestablished", 0),
    ]);
    let mut indicator_changes = 0u64;
    let mut indicator_unchanged = 0u64;
    let mut indicator_unknown = 0u64;
    for id in ids {
        work.charge(24)?;
        let status = report.identity_status(id);
        *counts.entry(status).or_default() += 1;
        let mut stable = false;
        if let (Some(a), Some(b)) = (report.before.get(&id), report.after.get(&id))
            && status == "same_generation"
        {
            let delta = indicators(a, b);
            stable = unchanged(&delta);
            indicator_changes += u64::from(changed(&delta));
            indicator_unchanged += u64::from(stable);
            indicator_unknown += u64::from(delta.values().any(|v| *v == "unestablished"));
            *categories.entry(category_change(a, b)).or_default() += 1;
            *transitions.entry(elevated_change(a, b)).or_default() += 1;
        }

        if focus_id.is_none_or(|selected| selected == id) && (include_unchanged || !stable) {
            report.selected.push(id);
        }
    }
    work.charge((report.before.len() + report.after.len()) as u64)?;
    report.summary = json!({"before":population(&report.before),"after":population(&report.after),"identity_counts":counts,
        "same_generation_category_changes":categories,"same_generation_elevated_category_transitions":transitions,
        "same_generation_with_observed_indicator_change":indicator_changes,"same_generation_with_all_indicators_unchanged":indicator_unchanged,
        "same_generation_with_unestablished_indicators":indicator_unknown,"selected_rows":report.selected.len(),
        "scope":"entire_endpoint_strict_citizen_union_before_focus_and_unchanged_filter"});
    work.charge(0)?;
    report.work_units = work.used;
    Ok(report)
}

pub(super) fn compare(
    before: &LiveSpatialCitizenState,
    after: &LiveSpatialCitizenState,
    context: &OperationContext,
    focus: Option<&Focus>,
    include_unchanged: bool,
    maximum: u64,
) -> Result<Report> {
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    let original = before
        .observation_full()
        .ok_or_else(|| invariant("welfare basis source absent"))?
        .spatial()
        .operations();
    let target = after
        .observation_full()
        .ok_or_else(|| invariant("welfare target source absent"))?
        .spatial()
        .operations();
    if original.jobs.world_folder != target.jobs.world_folder
        || original.jobs.site_id != target.jobs.site_id
        || original.jobs.df_version != target.jobs.df_version
        || original.jobs.dfhack_version != target.jobs.dfhack_version
        || original.jobs.bridge_generation != target.jobs.bridge_generation
    {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "welfare history endpoints differ in world, site, software or source generation",
        ));
    }
    assess(
        Endpoint::state(before)?,
        Endpoint::state(after)?,
        context,
        focus,
        include_unchanged,
        maximum,
    )
}

pub(super) struct Page<'a> {
    pub context: &'a OperationContext,
    pub from: &'a Value,
    pub to: &'a Value,
    pub binding: Digest32,
    pub selection: Value,
    pub maximum_work: u64,
    pub limit: u32,
    pub continuation: Option<&'a str>,
}
fn token(offset: usize, identity: Digest32) -> String {
    format!("hcw1:{offset}:{}",Digest32::of_bytes(json!({"domain":"dfmcp.citizen-welfare-history-page/1","offset":offset,"identity":identity.to_string()}).to_string().as_bytes()))
}
pub(super) fn paginate(report: &Report, page: Page<'_>) -> Result<Value> {
    let context = page.context;
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if !(1..=128).contains(&page.limit) || page.limit > context.budget.max_entities {
        return Err(bounded(
            "citizen welfare history limit exceeds page/entity bound",
        ));
    }
    let model=Digest32::of_bytes(json!({"policy":POLICY,"basis":anchor_json(report.basis),"anchor":anchor_json(report.anchor),
        "before_source":report.before_source.to_string(),"after_source":report.after_source.to_string(),
        "archive_binding":page.binding.to_string(),"selection":page.selection}).to_string().as_bytes());
    let identity = Digest32::of_bytes(
        json!({"policy":POLICY,"model":model.to_string(),"session":context.session_id.to_string(),
        "authority_anchor":anchor_json(context.anchor),"max_work":page.maximum_work})
        .to_string()
        .as_bytes(),
    );
    let mut offset = 0usize;
    if let Some(raw) = page.continuation {
        if raw.len() > 128 {
            return Err(bounded(
                "citizen welfare history continuation exceeds 128 bytes",
            ));
        }
        let parts: Vec<_> = raw.split(':').collect();
        if parts.len() != 3
            || parts[0] != "hcw1"
            || parts[1].is_empty()
            || parts[1].starts_with('0')
            || !parts[1].bytes().all(|v| v.is_ascii_digit())
        {
            return Err(invalid("invalid citizen welfare history continuation"));
        }
        offset = parts[1]
            .parse()
            .map_err(|_| invalid("citizen welfare history continuation offset overflow"))?;
        if raw != token(offset, identity) {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "citizen welfare history continuation names another session, archive, endpoint, selection or allowance",
            ));
        }
        if offset >= report.len() {
            return Err(DfmcpError::new(
                ErrorCode::CursorGap,
                "citizen welfare history continuation is past the result set",
            ));
        }
    }
    let maximum = context
        .budget
        .max_bytes
        .min(u64::from(context.budget.max_output_tokens) * 4);
    let mut out = json!({"schema":"dfmcp.query.result/1","kind":"historical_citizen_welfare","policy":POLICY,
        "basis":anchor_json(report.basis),"anchor":anchor_json(report.anchor),"model_digest":model.to_string(),
        "basis_source_digest":report.before_source.to_string(),"target_source_digest":report.after_source.to_string(),
        "source_profile":"spatial/1.8","elapsed_game_ticks":report.anchor.tick.0-report.basis.tick.0,
        "selection":page.selection,"summary":report.summary,"work_units":report.work_units,
        "coverage":{"strict_citizens_before":if report.before_complete{"complete_at_capture"}else{"unknown"},
            "strict_citizens_after":if report.after_complete{"complete_at_capture"}else{"unknown"},"temporal":"selected_endpoints_only",
            "intermediate_events":"unknown","physical_health":"unknown","needs":"unknown",
            "raw_stress_value":"not_observed","soul_presence":"not_observed","other_population":"not_observed"},
        "ordering":"canonical_citizen_entity_id_ascending","all_clear_proven":false,"health_change_proven":false,
        "raw_stress_change_proven":false,"continuous_trend_proven":false,"cause_proven":false,
        "current_freshness_proven":false,"native_captures":0,"mutation_dispatched":false,
        "interpretation":"Category movement compares only the observed 0-highest to 6-lowest ordering. Category 3 may be DFHack's no-soul fallback; raw stress and soul presence were not measured. Roster disappearance does not establish death, migration or insanity. Two endpoints cannot prove a continuous trend, cause, current wellbeing or health change. Availability is not labor permission.",
        "rows":[],"returned":0,"offset":offset,"truncated":false,"continuation":null});
    if out.to_string().len() as u64 > maximum {
        return Err(bounded(
            "citizen welfare history metadata exceeds result budget",
        ));
    }
    let mut rows = Vec::new();
    let mut work = Work::new(context, page.maximum_work)?;
    work.charge(report.work_units)?;
    while offset + rows.len() < report.len() && rows.len() < page.limit as usize {
        work.charge(32)?;
        rows.push(report.row(offset + rows.len(), page.from, page.to)?);
        let next = offset + rows.len();
        out["rows"] = json!(rows);
        out["returned"] = json!(rows.len());
        out["truncated"] = json!(next < report.len());
        out["continuation"] = if next < report.len() {
            json!(token(next, identity))
        } else {
            Value::Null
        };
        out["work_units"] = json!(work.used);
        if out.to_string().len() as u64 > maximum {
            rows.pop();
            if rows.is_empty() {
                return Err(bounded(
                    "one complete citizen welfare history row exceeds result budget",
                ));
            }
            break;
        }
    }
    let next = offset + rows.len();
    out["rows"] = json!(rows);
    out["returned"] = json!(rows.len());
    out["truncated"] = json!(next < report.len());
    out["continuation"] = if next < report.len() {
        json!(token(next, identity))
    } else {
        Value::Null
    };
    out["work_units"] = json!(work.used);
    if out.to_string().len() as u64 > maximum {
        return Err(bounded(
            "complete citizen welfare history page exceeds result budget",
        ));
    }
    work.charge(0)?;
    Ok(out)
}

#[cfg(test)]
#[path = "citizen_welfare_progress_tests.rs"]
mod tests;
