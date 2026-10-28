//! Observed job indicators at two coherent endpoints. A vanished job is not a
//! completed job; unchanged indicators are not proof of a stalled job. This
//! module deliberately has no transport, native service or mutation dependency.
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use dfmcp_adapter::live_operations::{building_entity_id, item_entity_id};
use dfmcp_adapter::live_spatial::{
    SpatialStateView,
    citizens::{LiveSpatialCitizenState, citizen_entity_id},
};
use dfmcp_core::{
    Capability, DfmcpError, Digest32, EntityId, ErrorCode, OperationContext, Result, RiskTier,
    StateAnchor,
};
use dfmcp_world::{
    EdgeKind, EdgeRecord, EntityKind, EntityRecord, Fact, FactPresence, FactSource, Value as W,
    WorldSnapshot,
};
use serde::Deserialize;
use serde_json::{Value, json};

pub(super) const POLICY: &str = "dfmcp.historical-job-progress/1";
pub(super) const MAX_WORK: u64 = 4_000_000;
const MAX_JOBS: usize = 4_096;
const MAX_ENTITIES: usize = 131_072;
const MAX_EDGES: usize = 262_144;
const EXAMPLES: usize = 4;

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
            return Err(bounded("job progress max_work must be 1..4000000"));
        }
        let mut value = Self {
            context,
            started: Instant::now(),
            maximum,
            used: 0,
        };
        value.charge(0)?;
        Ok(value)
    }
    fn charge(&mut self, units: u64) -> Result<()> {
        self.used = self
            .used
            .checked_add(units)
            .ok_or_else(|| bounded("job progress work overflow"))?;
        if self.used > self.maximum
            || self.started.elapsed().as_millis() >= u128::from(self.context.budget.max_wall_millis)
        {
            return Err(bounded("job progress exhausted work or wall allowance"));
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
                "job focus entity_id must be a canonical positive decimal u64 string",
            ));
        }
        Ok(())
    }
    pub fn json(&self) -> Value {
        json!({"entity_id":self.entity_id,"generation":self.generation})
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Identity {
    id: EntityId,
    generation: u32,
}
impl Identity {
    fn of(entity: &EntityRecord) -> Self {
        Self {
            id: entity.id,
            generation: entity.generation,
        }
    }
    fn json(self) -> Value {
        json!({"entity_id":self.id.to_string(),"generation":self.generation})
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Worker {
    Unassigned,
    Strict { native: u32, identity: Identity },
    OutsideRoster(u32),
    Unestablished,
}
impl Worker {
    fn json(&self) -> Value {
        match self {
            Self::Unassigned => json!({"status":"unassigned"}),
            Self::Strict { native, identity } => {
                json!({"status":"assigned_strict_citizen","native_id":native,"worker":identity.json()})
            }
            Self::OutsideRoster(native) => {
                json!({"status":"assigned_outside_strict_roster","native_id":native,"worker_identity":"unestablished"})
            }
            Self::Unestablished => {
                json!({"status":"unestablished","reason":"missing_unqualified_or_contradictory_assignment_evidence"})
            }
        }
    }
    fn qualified(&self) -> bool {
        !matches!(self, Self::Unestablished)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Holder {
    Absent,
    Observed {
        identity: Identity,
        stage: Option<(i64, i64)>,
    },
    Unestablished,
}
impl Holder {
    fn json(&self) -> Value {
        match self {
            Self::Absent => json!({"status":"absent"}),
            Self::Observed { identity, stage } => {
                json!({"status":"observed","building":identity.json(),
                "build_stage":stage.map(|v|v.0),"max_build_stage":stage.map(|v|v.1)})
            }
            Self::Unestablished => {
                json!({"status":"unestablished","reason":"missing_unqualified_or_contradictory_holder_evidence"})
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Attachment {
    item: Identity,
    role: i64,
    filter: i64,
}
impl Attachment {
    fn json(&self) -> Value {
        json!({"item":self.item.json(),"role":self.role,"filter_index":self.filter})
    }
}
#[derive(Clone)]
struct Attachments {
    count: Option<u64>,
    filters: Option<u64>,
    membership: Option<Digest32>,
    examples: Vec<Attachment>,
}
impl Attachments {
    fn json(&self) -> Value {
        json!({"status":if self.membership.is_some(){"complete_at_capture"}else{"unestablished"},
            "observed_attachment_count":self.count,"required_filter_count":self.filters,
            "membership_digest":self.membership.map(|v|v.to_string()),
            "examples":self.examples.iter().map(Attachment::json).collect::<Vec<_>>(),
            "examples_complete":self.membership.is_some() && self.count.is_some_and(|n|n<=EXAMPLES as u64),
            "readiness_proven":false})
    }
}

struct Job {
    identity: Identity,
    revision: u64,
    native_id: Option<u32>,
    job_type: Option<i64>,
    type_key: Option<String>,
    reaction: Option<String>,
    suspended: Option<bool>,
    repeating: Option<bool>,
    timer: Option<i64>,
    position: Option<[i32; 3]>,
    worker: Worker,
    holder: Holder,
    attachments: Attachments,
}
impl Job {
    fn json(&self) -> Value {
        json!({"handle":{"entity_id":self.identity.id.to_string(),"generation":self.identity.generation,"revision":self.revision},
            "native_job_id":self.native_id,"job_type":self.job_type,"type_key":self.type_key,"reaction":self.reaction,
            "suspended":self.suspended,"repeating":self.repeating,"completion_timer_raw":self.timer,"position":self.position,
            "worker_assignment":self.worker.json(),"holder":self.holder.json(),"attachments":self.attachments.json()})
    }
}

/// Completeness belongs to the native observation, never to a filtered query.
/// `None` is used by evidence-policy tests and cannot prove roster absence.
struct Endpoint<'a> {
    snapshot: &'a WorldSnapshot,
    source: Digest32,
    roster: Option<BTreeSet<EntityId>>,
}
impl<'a> Endpoint<'a> {
    fn state(state: &'a LiveSpatialCitizenState) -> Result<Self> {
        let snapshot = state
            .snapshot()
            .ok_or_else(|| invariant("job progress snapshot absent"))?;
        let original = state
            .observation_full()
            .ok_or_else(|| invariant("job progress native source absent"))?;
        Ok(Self {
            snapshot,
            source: state.source_digest()?,
            roster: Some(
                original
                    .spatial()
                    .operations()
                    .jobs
                    .jobs
                    .iter()
                    .map(|j| EntityId::new(u64::from(j.native_id) + 2))
                    .collect(),
            ),
        })
    }
    fn matches(&self, fact: &Fact, suffix: &str) -> bool {
        fact.observed_at == self.snapshot.anchor().tick
            && fact.source_digest == self.source
            && matches!(&fact.source, FactSource::DfhackField(path) if path.strip_prefix("spatial/1.8.")==Some(suffix))
    }
    fn native<'b>(
        &self,
        fields: &'b BTreeMap<String, Fact>,
        key: &str,
        suffix: &str,
    ) -> Option<&'b W> {
        let fact = fields.get(key)?;
        if !self.matches(fact, suffix) {
            return None;
        }
        match &fact.presence {
            None => Some(&fact.value),
            Some(FactPresence::Known(v)) if v == &fact.value => Some(v),
            _ => None,
        }
    }
    fn absent(&self, fields: &BTreeMap<String, Fact>, key: &str, suffix: &str) -> bool {
        fields.get(key).is_some_and(|f| {
            self.matches(f, suffix)
                && f.value == W::Null
                && f.presence == Some(FactPresence::Absent)
        })
    }
    fn unknown(&self, fields: &BTreeMap<String, Fact>, key: &str, suffix: &str) -> bool {
        fields.get(key).is_some_and(|f| {
            self.matches(f, suffix)
                && f.value == W::Null
                && matches!(f.presence, Some(FactPresence::Unknown(_)))
        })
    }
    fn unsigned(&self, e: &EntityRecord, key: &str, suffix: &str) -> Option<u64> {
        match self.native(&e.fields, key, suffix) {
            Some(W::U64(v)) => Some(*v),
            _ => None,
        }
    }
    fn signed(&self, e: &EntityRecord, key: &str, suffix: &str) -> Option<i64> {
        match self.native(&e.fields, key, suffix) {
            Some(W::I64(v)) if i32::try_from(*v).is_ok() => Some(*v),
            _ => None,
        }
    }
    fn boolean(&self, e: &EntityRecord, key: &str, suffix: &str) -> Option<bool> {
        match self.native(&e.fields, key, suffix) {
            Some(W::Bool(v)) => Some(*v),
            _ => None,
        }
    }
    fn text(&self, e: &EntityRecord, key: &str, suffix: &str) -> Option<String> {
        match self.native(&e.fields, key, suffix) {
            Some(W::Text(v)) if v.len() <= 128 => Some(v.clone()),
            _ => None,
        }
    }
    fn native_id(&self, e: &EntityRecord, key: &str, suffix: &str) -> Option<u32> {
        self.unsigned(e, key, suffix)
            .and_then(|v| u32::try_from(v).ok())
            .filter(|v| *v <= i32::MAX as u32)
    }
    fn worker(&self, job: &EntityRecord, edges: &[&EdgeRecord]) -> Worker {
        let assigned = self.boolean(job, "worker_assigned", "operations.Job.getWorker");
        let strict = self.boolean(job, "worker_is_strict_citizen", "Job.getWorker");
        if assigned == Some(false)
            && strict == Some(false)
            && edges.is_empty()
            && self.absent(
                &job.fields,
                "worker_native_id",
                "operations.worker_native_id",
            )
            && self.absent(&job.fields, "worker_entity", "Job.getWorker")
        {
            return Worker::Unassigned;
        }
        let Some(native) = self.native_id(job, "worker_native_id", "operations.worker_native_id")
        else {
            return Worker::Unestablished;
        };
        if assigned != Some(true) {
            return Worker::Unestablished;
        }
        let id = citizen_entity_id(native);
        if strict == Some(false)
            && edges.is_empty()
            && !self.snapshot.graph.entities.contains_key(&id)
            && self.unknown(&job.fields, "worker_entity", "Job.getWorker")
        {
            return Worker::OutsideRoster(native);
        }
        let Some(worker) = self.snapshot.graph.entities.get(&id) else {
            return Worker::Unestablished;
        };
        if strict != Some(true)
            || worker.kind != EntityKind::Unit
            || self.native_id(worker, "native_unit_id", "citizen.unit_id") != Some(native)
            || self.native(&job.fields, "worker_entity", "Job.getWorker") != Some(&W::Entity(id))
            || edges.len() != 1
            || edges[0].from != id
            || self.native(&edges[0].fields, "assignment_observed", "Job.getWorker")
                != Some(&W::Bool(true))
        {
            return Worker::Unestablished;
        }
        Worker::Strict {
            native,
            identity: Identity::of(worker),
        }
    }
    fn holder(&self, job: &EntityRecord, edges: &[&EdgeRecord]) -> Holder {
        if self.absent(
            &job.fields,
            "holder_native_id",
            "operations.holder_native_id",
        ) && edges.is_empty()
        {
            return Holder::Absent;
        }
        let Some(native) = self.native_id(job, "holder_native_id", "operations.holder_native_id")
        else {
            return Holder::Unestablished;
        };
        let id = building_entity_id(native);
        let Some(holder) = self.snapshot.graph.entities.get(&id) else {
            return Holder::Unestablished;
        };
        if holder.kind != EntityKind::Building
            || self.native_id(holder, "native_building_id", "operations.building.id")
                != Some(native)
            || edges.len() != 1
            || edges[0].to != id
            || self.native(&edges[0].fields, "relation", "operations.Job.getHolder")
                != Some(&W::Text("job_holder".into()))
        {
            return Holder::Unestablished;
        }
        let stage = self
            .signed(holder, "build_stage", "operations.building.getBuildStage")
            .zip(self.signed(
                holder,
                "max_build_stage",
                "operations.building.getMaxBuildStage",
            ))
            .filter(|(value, max)| *value >= 0 && value <= max);
        Holder::Observed {
            identity: Identity::of(holder),
            stage,
        }
    }
    fn attachments(
        &self,
        job: &EntityRecord,
        edges: &[&EdgeRecord],
        work: &mut Work<'_>,
    ) -> Result<Attachments> {
        let count = self
            .unsigned(job, "attached_item_count", "operations.job.items.size")
            .filter(|n| *n <= 65_536);
        let filters = self
            .unsigned(
                job,
                "required_item_filter_count",
                "operations.job.job_items.elements.size",
            )
            .filter(|n| *n <= 4096);
        let mut members = BTreeSet::new();
        let mut complete = count == Some(edges.len() as u64) && filters.is_some();
        for edge in edges {
            work.charge(8)?;
            let Some(item) = self.snapshot.graph.entities.get(&edge.to) else {
                complete = false;
                continue;
            };
            let native = self.native_id(item, "native_item_id", "operations.item.id");
            let role = match self.native(&edge.fields, "role", "operations.job.items.role") {
                Some(W::I64(v)) if *v >= 0 && i32::try_from(*v).is_ok() => Some(*v),
                _ => None,
            };
            let filter = match self.native(
                &edge.fields,
                "filter_index",
                "operations.job.items.job_item_idx",
            ) {
                Some(W::I64(v))
                    if *v == -1 || (*v >= 0 && filters.is_some_and(|n| (*v as u64) < n)) =>
                {
                    Some(*v)
                }
                _ => None,
            };
            match (native, role, filter) {
                (Some(native), Some(role), Some(filter))
                    if item.kind == EntityKind::Item && item_entity_id(native) == item.id =>
                {
                    if !members.insert(Attachment {
                        item: Identity::of(item),
                        role,
                        filter,
                    }) {
                        complete = false;
                    }
                }
                _ => complete = false,
            }
        }
        let membership = if complete {
            let encoded = members.iter().map(Attachment::json).collect::<Vec<_>>();
            Some(Digest32::of_bytes(
                json!({"domain":"dfmcp.job-attachment-membership/1","members":encoded})
                    .to_string()
                    .as_bytes(),
            ))
        } else {
            None
        };
        Ok(Attachments {
            count,
            filters,
            membership,
            examples: members.into_iter().take(EXAMPLES).collect(),
        })
    }
    fn collect(&self, work: &mut Work<'_>) -> Result<BTreeMap<EntityId, Job>> {
        let graph = &self.snapshot.graph;
        if graph.entities.len() > MAX_ENTITIES || graph.edges.len() > MAX_EDGES {
            return Err(bounded(
                "job progress graph exceeds complete spatial profile bounds",
            ));
        }
        work.charge((graph.entities.len() as u64).saturating_add(graph.edges.len() as u64))?;
        if !self.snapshot.hash_is_valid() {
            return Err(invariant("job progress snapshot hash invalid"));
        }
        let mut jobs = BTreeMap::new();
        for (id, e) in &graph.entities {
            work.charge(1)?;
            if *id != e.id {
                return Err(invariant("job progress entity map identity mismatch"));
            }
            if e.kind == EntityKind::Job {
                jobs.insert(*id, e);
            }
        }
        if jobs.len() > MAX_JOBS {
            return Err(bounded("job progress job roster exceeds 4096"));
        }
        if self.roster.as_ref().is_some_and(|ids| {
            ids.len() != jobs.len() || ids.iter().any(|id| !jobs.contains_key(id))
        }) {
            return Err(invariant(
                "job progress projection differs from complete native job roster",
            ));
        }
        let mut performers: BTreeMap<EntityId, Vec<&EdgeRecord>> = BTreeMap::new();
        let mut holders: BTreeMap<EntityId, Vec<&EdgeRecord>> = BTreeMap::new();
        let mut attachments: BTreeMap<EntityId, Vec<&EdgeRecord>> = BTreeMap::new();
        for (id, e) in &graph.edges {
            work.charge(1)?;
            if *id != e.id {
                return Err(invariant("job progress edge map identity mismatch"));
            }
            match e.kind {
                EdgeKind::Performs if jobs.contains_key(&e.to) => {
                    performers.entry(e.to).or_default().push(e)
                }
                EdgeKind::ContainedIn if jobs.contains_key(&e.from) => {
                    holders.entry(e.from).or_default().push(e)
                }
                EdgeKind::Uses if jobs.contains_key(&e.from) => {
                    attachments.entry(e.from).or_default().push(e)
                }
                _ => {}
            }
        }
        let mut result = BTreeMap::new();
        for (id, e) in jobs {
            work.charge(32)?;
            let native_id = self
                .native_id(e, "native_job_id", "operations.job.id")
                .filter(|n| EntityId::new(u64::from(*n) + 2) == id);
            let position = match self.native(&e.fields, "position", "operations.job.pos") {
                Some(W::Coord(p)) => Some([p.x, p.y, p.z]),
                _ => None,
            };
            result.insert(
                id,
                Job {
                    identity: Identity::of(e),
                    revision: e.revision,
                    native_id,
                    job_type: self
                        .signed(e, "job_type", "operations.job.type")
                        .filter(|v| *v >= 0),
                    type_key: self
                        .text(e, "type_key", "operations.job.type_key")
                        .filter(|v| !v.is_empty()),
                    reaction: self.text(e, "reaction", "operations.job.reaction_name"),
                    suspended: self.boolean(e, "suspended", "operations.job.flags.suspend"),
                    repeating: self.boolean(e, "repeating", "operations.job.flags.repeat"),
                    timer: self
                        .signed(e, "completion_timer_raw", "operations.job.completion_timer")
                        .filter(|v| *v >= -1),
                    position,
                    worker: self.worker(e, performers.get(&id).map_or(&[], Vec::as_slice)),
                    holder: self.holder(e, holders.get(&id).map_or(&[], Vec::as_slice)),
                    attachments: self.attachments(
                        e,
                        attachments.get(&id).map_or(&[], Vec::as_slice),
                        work,
                    )?,
                },
            );
        }
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
fn indicators(before: &Job, after: &Job) -> BTreeMap<&'static str, &'static str> {
    let worker = if !before.worker.qualified() || !after.worker.qualified() {
        "unestablished"
    } else if matches!((&before.worker,&after.worker),
            (Worker::OutsideRoster(a),Worker::OutsideRoster(b))
            | (Worker::Strict{native:a,..},Worker::OutsideRoster(b))
            | (Worker::OutsideRoster(a),Worker::Strict{native:b,..}) if a==b)
    {
        "unestablished"
    } else if before.worker == after.worker {
        "unchanged"
    } else {
        "changed"
    };
    let (holder, stage) = match (&before.holder, &after.holder) {
        (Holder::Absent, Holder::Absent) => ("unchanged", "not_applicable"),
        (
            Holder::Observed {
                identity: a,
                stage: sa,
            },
            Holder::Observed {
                identity: b,
                stage: sb,
            },
        ) if a == b => {
            let status = match (sa, sb) {
                (Some((a, ma)), Some((b, mb))) if ma == mb => {
                    if a < b {
                        "advanced"
                    } else if a > b {
                        "regressed"
                    } else {
                        "unchanged"
                    }
                }
                (Some(_), Some(_)) => "maximum_changed_not_comparable",
                _ => "unestablished",
            };
            ("unchanged", status)
        }
        (Holder::Unestablished, _) | (_, Holder::Unestablished) => {
            ("unestablished", "unestablished")
        }
        _ => ("changed", "holder_changed_not_comparable"),
    };
    BTreeMap::from([
        (
            "native_job_id",
            difference(&before.native_id, &after.native_id),
        ),
        (
            "suspension",
            difference(&before.suspended, &after.suspended),
        ),
        ("repeating", difference(&before.repeating, &after.repeating)),
        ("job_type", difference(&before.job_type, &after.job_type)),
        ("type_key", difference(&before.type_key, &after.type_key)),
        ("reaction", difference(&before.reaction, &after.reaction)),
        ("position", difference(&before.position, &after.position)),
        (
            "completion_timer_raw",
            difference(&before.timer, &after.timer),
        ),
        ("worker_assignment", worker),
        ("holder_identity", holder),
        ("holder_build_stage", stage),
        (
            "attachment_membership",
            difference(
                &before.attachments.membership,
                &after.attachments.membership,
            ),
        ),
        (
            "required_filter_count",
            difference(&before.attachments.filters, &after.attachments.filters),
        ),
    ])
}
fn changed(values: &BTreeMap<&str, &str>) -> bool {
    values.values().any(|v| {
        matches!(
            *v,
            "changed" | "advanced" | "regressed" | "maximum_changed_not_comparable"
        )
    })
}
fn unchanged(values: &BTreeMap<&str, &str>) -> bool {
    values
        .values()
        .all(|v| matches!(*v, "unchanged" | "not_applicable"))
}

pub(super) struct Report {
    before: BTreeMap<EntityId, Job>,
    after: BTreeMap<EntityId, Job>,
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
            (Some(a), Some(b)) if a.identity.generation == b.identity.generation => {
                "same_generation"
            }
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
            .ok_or_else(|| invariant("job progress row index out of bounds"))?;
        let before = self.before.get(&id);
        let after = self.after.get(&id);
        let status = self.identity_status(id);
        let delta = match (before, after) {
            (Some(a), Some(b)) if status == "same_generation" => Some(indicators(a, b)),
            _ => None,
        };
        let suspended_both = match (before, after) {
            (Some(a), Some(b)) if status == "same_generation" => {
                a.suspended.zip(b.suspended).map(|(a, b)| a && b)
            }
            _ => None,
        };
        let unassigned_both = match (before, after) {
            (Some(a), Some(b))
                if status == "same_generation" && a.worker.qualified() && b.worker.qualified() =>
            {
                Some(a.worker == Worker::Unassigned && b.worker == Worker::Unassigned)
            }
            _ => None,
        };
        Ok(json!({"entity_id":id.to_string(),"identity_status":status,
            "before":before.map(Job::json),"after":after.map(Job::json),
            "indicators":delta,"observed_indicator_change":delta.as_ref().is_some_and(changed),
            "all_compared_indicators_unchanged":delta.as_ref().is_some_and(unchanged),
            "suspended_at_both_endpoints":suspended_both,"unassigned_at_both_endpoints":unassigned_both,
            "inspect_before":before.map(|j|inspect(j,from)),"inspect_after":after.map(|j|inspect(j,to))}))
    }
}
fn inspect(job: &Job, entry: &Value) -> Value {
    json!({"schema":"dfmcp.query/1","query":{"kind":"historical_query","record":entry["record"],"record_digest":entry["record_digest"],
        "query":{"kind":"inspect","entity_id":job.identity.id.to_string(),"generation":job.identity.generation,
            "fields":["native_job_id","type_key","reaction","suspended","repeating","worker_assigned","worker_native_id","worker_entity","holder_native_id","completion_timer_raw","attached_item_count","required_item_filter_count"]}}})
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
            "job progress crosses fortress, epoch or clock/sequence reset",
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
                    "focused job is absent from the basis record",
                )
            })?;
        if found.1.identity.generation != focus.generation {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "focused job generation differs from the basis record",
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
    let mut counts: BTreeMap<&str, u64> = BTreeMap::from([
        ("same_generation", 0),
        ("generation_replaced", 0),
        ("newly_observed", 0),
        ("no_longer_observed", 0),
        ("roster_absence_unestablished", 0),
    ]);
    let mut indicators_changed = 0u64;
    let mut indicators_unchanged = 0u64;
    let mut indicators_unestablished = 0u64;
    let mut stage_advanced = 0u64;
    let mut stage_regressed = 0u64;
    let mut suspended_both = 0u64;
    let mut unassigned_both = 0u64;
    for id in ids {
        work.charge(20)?;
        let status = report.identity_status(id);
        *counts.entry(status).or_default() += 1;
        let mut stable = false;
        if let (Some(a), Some(b)) = (report.before.get(&id), report.after.get(&id)) {
            if status == "same_generation" {
                let delta = indicators(a, b);
                stable = unchanged(&delta);
                if changed(&delta) {
                    indicators_changed += 1;
                }
                if stable {
                    indicators_unchanged += 1;
                }
                if delta.values().any(|s| *s == "unestablished") {
                    indicators_unestablished += 1;
                }
                if delta.get("holder_build_stage") == Some(&"advanced") {
                    stage_advanced += 1;
                }
                if delta.get("holder_build_stage") == Some(&"regressed") {
                    stage_regressed += 1;
                }
                if a.suspended == Some(true) && b.suspended == Some(true) {
                    suspended_both += 1;
                }
                if a.worker == Worker::Unassigned && b.worker == Worker::Unassigned {
                    unassigned_both += 1;
                }
            }
        }
        if focus_id.is_none_or(|selected| selected == id) && (include_unchanged || !stable) {
            report.selected.push(id);
        }
    }
    report.summary = json!({"jobs_before":report.before.len(),"jobs_after":report.after.len(),"identity_counts":counts,
        "same_generation_with_observed_indicator_change":indicators_changed,"same_generation_with_all_indicators_unchanged":indicators_unchanged,
        "same_generation_with_unestablished_indicators":indicators_unestablished,"selected_rows":report.selected.len(),
        "same_holder_generation_stage_advanced":stage_advanced,"same_holder_generation_stage_regressed":stage_regressed,
        "same_generation_suspended_at_both_endpoints":suspended_both,"same_generation_unassigned_at_both_endpoints":unassigned_both,
        "scope":"entire_endpoint_job_union_before_focus_and_unchanged_filter"});
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
        .ok_or_else(|| invariant("basis source absent"))?
        .spatial()
        .operations();
    let target = after
        .observation_full()
        .ok_or_else(|| invariant("target source absent"))?
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
            "job progress endpoints differ in world, site, software or source generation",
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
    format!("hjp1:{offset}:{}",Digest32::of_bytes(json!({"domain":"dfmcp.job-progress-page/1","offset":offset,"identity":identity.to_string()}).to_string().as_bytes()))
}
pub(super) fn paginate(report: &Report, page: Page<'_>) -> Result<Value> {
    let context = page.context;
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if !(1..=128).contains(&page.limit) || page.limit > context.budget.max_entities {
        return Err(bounded("job progress limit exceeds page/entity bound"));
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
            return Err(bounded("job progress continuation exceeds 128 bytes"));
        }
        let parts: Vec<_> = raw.split(':').collect();
        if parts.len() != 3
            || parts[0] != "hjp1"
            || parts[1].is_empty()
            || parts[1].starts_with('0')
            || !parts[1].bytes().all(|v| v.is_ascii_digit())
        {
            return Err(invalid("invalid job progress continuation"));
        }
        offset = parts[1]
            .parse()
            .map_err(|_| invalid("job progress continuation offset overflow"))?;
        if raw != token(offset, identity) {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "job progress continuation names another session, archive, endpoint, selection or allowance",
            ));
        }
        if offset >= report.len() {
            return Err(DfmcpError::new(
                ErrorCode::CursorGap,
                "job progress continuation is past the result set",
            ));
        }
    }
    let maximum = context
        .budget
        .max_bytes
        .min(u64::from(context.budget.max_output_tokens) * 4);
    let mut out = json!({"schema":"dfmcp.query.result/1","kind":"historical_job_progress","policy":POLICY,
        "basis":anchor_json(report.basis),"anchor":anchor_json(report.anchor),"model_digest":model.to_string(),
        "basis_source_digest":report.before_source.to_string(),"target_source_digest":report.after_source.to_string(),
        "source_profile":"spatial/1.8","elapsed_game_ticks":report.anchor.tick.0-report.basis.tick.0,
        "selection":page.selection,"summary":report.summary,"work_units":report.work_units,
        "coverage":{"jobs_before":if report.before_complete{"complete_at_capture"}else{"unknown"},
            "jobs_after":if report.after_complete{"complete_at_capture"}else{"unknown"},"temporal":"selected_endpoints_only",
            "intermediate_events":"unknown","native_blocker_reasons":"not_observed"},
        "ordering":"canonical_job_entity_id_ascending","completion_proven":false,"stall_proven":false,"blocker_proven":false,
        "current_freshness_proven":false,"native_captures":0,"mutation_dispatched":false,
        "interpretation":"Only endpoint indicators are compared. Disappearance does not establish completion; unchanged endpoints do not establish a stall or continuous suspension. Raw timers are not an ETA or work quantity. Holder stages concern the same observed building generation, not job completion. Tick distance is not job age.",
        "rows":[],"returned":0,"offset":offset,"truncated":false,"continuation":null});
    if out.to_string().len() as u64 > maximum {
        return Err(bounded("job progress metadata exceeds result budget"));
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
                    "one complete job progress row exceeds result budget",
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
        return Err(bounded("complete job progress page exceeds result budget"));
    }
    work.charge(0)?;
    Ok(out)
}

#[cfg(test)]
#[path = "job_progress_tests.rs"]
mod tests;
