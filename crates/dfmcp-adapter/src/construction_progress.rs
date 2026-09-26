//! Furniture construction conditions over ONE privately published operations view.
//! This joins observed building stages, job holders and exact item holders; it
//! never joins independently acquired captures or interprets job disappearance
//! alone as completion. No effect receipt, terrain safety or causality is proved.

use crate::live_jobs::LiveJob;
use crate::live_operations::{LiveItem, building_entity_id, item_entity_id};
use crate::operations_analysis::{AnalysisHandle, OperationsStateView};
use dfmcp_core::{
    Capability, DfmcpError, Digest32, EntityId, ErrorCode, MapCoord, OperationContext, Result,
    RiskTier, StateAnchor,
};
use dfmcp_world::{EntityKind, WorldSnapshot};
use std::collections::BTreeMap;
use std::time::Instant;

pub const POLICY: &str = "dfmcp.furniture-construction-condition/1";
pub const MAX_TARGETS: usize = 32;
pub const MAX_JOB_EXAMPLES: usize = 8;
pub const MAX_WORK: u64 = 1_000_000;

/// An optional expectation is a comparison, not proof of a placement receipt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub building_native_id: u32,
    pub expected_generation: Option<u32>,
    pub expected_type: Option<String>,
    pub item_native_id: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Missing,
    IdentityMismatch,
    Unsupported,
    RemovalPending,
    Suspended,
    NoConstructionJob,
    Pending,
    ItemUnverified,
    SatisfiedAtObservation,
}
impl Status {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::IdentityMismatch => "identity_mismatch",
            Self::Unsupported => "unsupported",
            Self::RemovalPending => "removal_pending",
            Self::Suspended => "suspended",
            Self::NoConstructionJob => "no_construction_job",
            Self::Pending => "pending",
            Self::ItemUnverified => "item_unverified",
            Self::SatisfiedAtObservation => "satisfied_at_observation",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobEvidence {
    pub native_id: u32,
    pub handle: AnalysisHandle,
    pub type_key: String,
    pub suspended: bool,
    /// A native ID from this capture, not a citizen-generation handle.
    pub worker_native_id: Option<u32>,
    pub completion_timer: i32,
    pub attached_items: u32,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Jobs {
    pub construction: u32,
    pub suspended_construction: u32,
    pub construction_with_worker: u32,
    pub removal: u32,
    pub other: u32,
    pub examples: Vec<JobEvidence>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemEvidence {
    pub native_id: u32,
    pub handle: Option<AnalysisHandle>,
    pub type_key: Option<String>,
    pub flags: Option<u32>,
    pub container_native_id: Option<u32>,
    pub holder_building_native_id: Option<u32>,
    pub attached_jobs: u32,
    pub attached_construction_jobs: u32,
    /// Known only when this exact selected item is in the complete item roster.
    pub installed_condition: Option<bool>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub target: Target,
    pub handle: Option<AnalysisHandle>,
    pub type_key: Option<String>,
    pub min: Option<MapCoord>,
    pub max: Option<MapCoord>,
    pub stage: Option<i32>,
    pub maximum_stage: Option<i32>,
    pub stage_complete: Option<bool>,
    pub jobs: Jobs,
    pub item: Option<ItemEvidence>,
    pub status: Status,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub anchor: StateAnchor,
    pub source_digest: Digest32,
    pub rows: Vec<Row>,
    pub work_used: u64,
}

fn error(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message)
}
fn invalid(message: &str) -> DfmcpError {
    error(ErrorCode::InvalidRequest, message)
}
fn bounded(message: &str) -> DfmcpError {
    error(ErrorCode::BudgetExceeded, message)
}
struct Work<'a> {
    context: &'a OperationContext,
    start: Instant,
    used: u64,
    limit: u64,
}
impl Work<'_> {
    fn charge(&mut self, count: u64) -> Result<()> {
        self.used = self.used.checked_add(count).ok_or_else(|| bounded("construction work overflow"))?;
        if self.used > self.limit {
            return Err(bounded("construction analysis exhausted its work allowance"));
        }
        if self.start.elapsed().as_millis() >= u128::from(self.context.budget.max_wall_millis) {
            return Err(bounded("construction analysis exhausted its foreground deadline"));
        }
        self.context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)
    }
}
fn handle(snapshot: &WorldSnapshot, id: EntityId, kind: EntityKind) -> Result<AnalysisHandle> {
    let value = snapshot.graph.entities.get(&id).ok_or_else(|| {
        error(ErrorCode::InternalInvariantViolation, "construction endpoint has no canonical identity")
    })?;
    if value.kind != kind || value.generation == 0 {
        return Err(error(ErrorCode::InternalInvariantViolation, "invalid construction endpoint identity"));
    }
    Ok(AnalysisHandle { entity_id: id, generation: value.generation, revision: value.revision })
}
fn item_kind(building: &str) -> Option<&'static str> {
    match building {
        "Bed" => Some("BED"),
        "Chair" => Some("CHAIR"),
        "Table" => Some("TABLE"),
        _ => None,
    }
}

/// Exact observation-local condition, deliberately not an inference from absence.
/// Public for deterministic policy testing; callers cannot use it as authority.
pub fn installed_item(item: &LiveItem, building: u32, kind: &str, attached_jobs: u32) -> bool {
    item_kind(kind) == Some(item.type_key.as_str())
        && item.holder_building_native_id == Some(building)
        && item.container_native_id.is_none()
        && item.flags & (1 << 8) != 0
        && item.flags & ((1 << 1) | (1 << 3) | (1 << 6) | (1 << 7)) == 0
        && attached_jobs == 0
}

fn job_evidence(job: &LiveJob, snapshot: &WorldSnapshot) -> Result<JobEvidence> {
    Ok(JobEvidence {
        native_id: job.native_id,
        handle: handle(snapshot, EntityId::new(u64::from(job.native_id) + 2), EntityKind::Job)?,
        type_key: job.type_key.clone(),
        suspended: job.suspended,
        worker_native_id: job.worker_native_id,
        completion_timer: job.completion_timer,
        attached_items: job.attached_item_count,
    })
}

/// Select at most 32 targets from the same complete, validated source. Native
/// IDs come from this capture; optional expectations never authenticate external
/// receipts. All selected rows are analyzed before any page can be rendered.
pub fn analyze<S: OperationsStateView + ?Sized>(
    state: &S,
    context: &OperationContext,
    targets: &[Target],
    max_work: u64,
) -> Result<Report> {
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if targets.is_empty() || targets.len() > MAX_TARGETS {
        return Err(invalid("construction selection requires 1..32 targets"));
    }
    if max_work == 0 || max_work > MAX_WORK {
        return Err(bounded("construction work allowance must be 1..1000000"));
    }
    let mut work = Work { context, start: Instant::now(), used: 0, limit: max_work };
    let mut selected = BTreeMap::new();
    let mut item_ids = BTreeMap::new();
    for target in targets {
        work.charge(1)?;
        if target.building_native_id >= i32::MAX as u32
            || target.item_native_id.is_some_and(|id| id >= i32::MAX as u32)
            || target.expected_generation == Some(0)
            || target.expected_type.as_ref().is_some_and(|kind| item_kind(kind).is_none())
            || selected.insert(target.building_native_id, target).is_some()
        {
            return Err(invalid("duplicate or invalid construction target or expectation"));
        }
        if let Some(id) = target.item_native_id {
            if item_ids.insert(id, target.building_native_id).is_some() {
                return Err(invalid("one exact item cannot be requested for multiple furniture targets"));
            }
        }
    }
    let observation = state.operations_observation().ok_or_else(|| invalid("no coherent operations capture"))?;
    let snapshot = state.operations_snapshot().ok_or_else(|| invalid("no canonical operations projection"))?;
    if snapshot.anchor() != context.anchor {
        return Err(error(ErrorCode::StaleAnchor, "construction context names another capture"));
    }
    if snapshot.graph.entities.len() > context.budget.max_entities as usize {
        return Err(bounded("construction analysis exceeds session entity scan allowance"));
    }
    // Account for both complete validation/hash traversals, then each join visit.
    // The sealed view excludes client-built or partially published observations.
    work.charge(2 * (snapshot.graph.entities.len() + snapshot.graph.edges.len()) as u64)?;
    if !snapshot.hash_is_valid() {
        return Err(error(ErrorCode::InternalInvariantViolation, "invalid construction source hash"));
    }
    let source_digest = state.operations_source_digest()?;
    work.charge(1)?;
    let mut buildings = BTreeMap::new();
    for building in &observation.buildings {
        work.charge(1)?;
        if selected.contains_key(&building.native_id) {
            buildings.insert(building.native_id, building);
        }
    }
    let mut jobs = BTreeMap::<u32, Jobs>::new();
    let mut construction_ids = BTreeMap::new();
    for job in &observation.jobs.jobs {
        work.charge(1)?;
        if let Some(holder) = job.holder_native_id.filter(|id| selected.contains_key(id)) {
            let group = jobs.entry(holder).or_default();
            match job.type_key.as_str() {
                "ConstructBuilding" => {
                    group.construction += 1;
                    group.suspended_construction += u32::from(job.suspended);
                    group.construction_with_worker += u32::from(job.worker_native_id.is_some());
                    construction_ids.insert(job.native_id, holder);
                }
                "DestroyBuilding" => group.removal += 1,
                _ => group.other += 1,
            }
            if group.examples.len() < MAX_JOB_EXAMPLES {
                group.examples.push(job_evidence(job, snapshot)?);
            }
        }
    }
    // Attachments are canonically ordered by job then item. Retain only the
    // last job per selected item: duplicate roles must not inflate job counts.
    let mut attachments = BTreeMap::<u32, (u32, u32, u32)>::new();
    for attachment in &observation.attachments {
        work.charge(1)?;
        if let Some(building) = item_ids.get(&attachment.item_native_id) {
            let entry = attachments.entry(attachment.item_native_id).or_insert((u32::MAX, 0, 0));
            if entry.0 != attachment.job_native_id {
                entry.0 = attachment.job_native_id;
                entry.1 += 1;
                entry.2 += u32::from(construction_ids.get(&attachment.job_native_id) == Some(building));
            }
        }
    }
    let mut items = BTreeMap::new();
    for item in &observation.items {
        work.charge(1)?;
        if item_ids.contains_key(&item.native_id) {
            items.insert(item.native_id, item);
        }
    }
    let mut rows = Vec::with_capacity(selected.len());
    for (id, target) in selected {
        work.charge(1)?;
        let building = buildings.get(&id).copied();
        let identity = building.map(|_| handle(snapshot, building_entity_id(id), EntityKind::Building)).transpose()?;
        let group = jobs.remove(&id).unwrap_or_default();
        let item = target.item_native_id.map(|item_id| -> Result<ItemEvidence> {
            let observed = items.get(&item_id).copied();
            let (_, attached_jobs, attached_construction_jobs) =
                attachments.get(&item_id).copied().unwrap_or_default();
            Ok(ItemEvidence {
                native_id: item_id,
                handle: observed.map(|_| handle(snapshot, item_entity_id(item_id), EntityKind::Item)).transpose()?,
                type_key: observed.map(|v| v.type_key.clone()),
                flags: observed.map(|v| v.flags),
                container_native_id: observed.and_then(|v| v.container_native_id),
                holder_building_native_id: observed.and_then(|v| v.holder_building_native_id),
                attached_jobs,
                attached_construction_jobs,
                installed_condition: observed.map(|v| building.is_some_and(|b| installed_item(v, id, &b.type_key, attached_jobs))),
            })
        }).transpose()?;
        let stage_complete = building.map(|b| b.max_build_stage > 0 && b.build_stage == b.max_build_stage);
        let status = match building {
            None => Status::Missing,
            Some(b) if target.expected_generation.is_some_and(|g| identity.as_ref().is_none_or(|h| h.generation != g))
                || target.expected_type.as_ref().is_some_and(|kind| kind != &b.type_key) => Status::IdentityMismatch,
            Some(b) if item_kind(&b.type_key).is_none() || b.max_build_stage <= 0
                || b.max_build_stage > 32 => Status::Unsupported,
            Some(_) if group.removal != 0 => Status::RemovalPending,
            Some(_) if stage_complete != Some(true) || group.construction != 0 => {
                if group.construction == 0 { Status::NoConstructionJob }
                else if group.suspended_construction == group.construction { Status::Suspended }
                else { Status::Pending }
            }
            Some(_) if item.as_ref().is_some_and(|v| v.installed_condition != Some(true)) => Status::ItemUnverified,
            Some(_) => Status::SatisfiedAtObservation,
        };
        rows.push(Row {
            target: target.clone(), handle: identity,
            type_key: building.map(|b| b.type_key.clone()),
            min: building.map(|b| MapCoord::new(b.x1, b.y1, b.z)),
            max: building.map(|b| MapCoord::new(b.x2, b.y2, b.z)),
            stage: building.map(|b| b.build_stage),
            maximum_stage: building.map(|b| b.max_build_stage),
            stage_complete, jobs: group, item, status,
        });
    }
    work.charge(1)?;
    Ok(Report { anchor: context.anchor, source_digest, rows, work_used: work.used })
}

#[cfg(test)]
#[path = "construction_progress_tests.rs"]
mod tests;
