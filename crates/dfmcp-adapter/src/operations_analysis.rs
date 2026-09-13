#![forbid(unsafe_code)]

//! Authority-checked cognition over the sealed, coherent operations projection.
//! Diagnostic observations are not causal blocker proofs. Allocation concerns
//! caller-declared stack-unit requests under a deliberately conservative policy,
//! not DF's full job requirement language or a reservation of game resources.

use std::collections::{BTreeMap, BTreeSet};
use dfmcp_core::{Capability, DfmcpError, Digest32, EntityId, ErrorCode,
    OperationContext, Result, RiskTier, StateAnchor};
use dfmcp_world::{EntityKind, WorldSnapshot};
use dfmcp_world::inventory_allocation::{self as flow, Allocation, AllocationError, Demand, Supply};
use crate::live_operations::{LiveItem, LiveOperationsObservation, LiveOperationsState,
    building_entity_id, item_entity_id};

pub const ANALYSIS_POLICY: &str = "dfmcp.operations-analysis/1";
pub const SUPPLY_POLICY: &str = "conservative-unattached-stack-units/1";
pub const MAX_ANALYSIS_WORK: u64 = flow::MAX_WORK;
const DISALLOWED_FLAGS: u32 = 0x1bf;
const ATTACHED: u32 = 1 << 9;
const BUILDING_HELD: u32 = 1 << 10;
const EXAMPLES: usize = 8;
pub const DIAGNOSTIC_FLAGS: [(u32, &str); 5] = [
    (8, "removed"), (16, "rotten"), (1, "forbidden"), (4, "dump"), (32, "trader"),
];

fn invalid(message: &str) -> DfmcpError { DfmcpError::new(ErrorCode::InvalidRequest, message) }
fn invariant(message: &str) -> DfmcpError { DfmcpError::new(ErrorCode::InternalInvariantViolation, message) }
fn exhausted(message: &str) -> DfmcpError { DfmcpError::new(ErrorCode::BudgetExceeded, message) }

struct Work { used: u64, maximum: u64 }
impl Work {
    fn new(maximum: u64) -> Result<Self> {
        if maximum == 0 || maximum > MAX_ANALYSIS_WORK { return Err(exhausted("invalid analysis work bound")); }
        Ok(Self { used: 0, maximum })
    }
    fn charge(&mut self, units: u64) -> Result<()> {
        self.used = self.used.checked_add(units).ok_or_else(|| exhausted("analysis counter overflow"))?;
        if self.used > self.maximum { return Err(exhausted("operations analysis exhausted its work budget")); }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AnalysisHandle {
    pub entity_id: EntityId,
    pub generation: u32,
    pub revision: u64,
}
fn handle(snapshot: &WorldSnapshot, id: EntityId) -> Result<AnalysisHandle> {
    let entity = snapshot.graph.entities.get(&id).ok_or_else(|| invariant("analysis endpoint has no canonical entity"))?;
    Ok(AnalysisHandle { entity_id: id, generation: entity.generation, revision: entity.revision })
}
fn source<'a>(state: &'a LiveOperationsState, context: &OperationContext)
    -> Result<(&'a LiveOperationsObservation, &'a WorldSnapshot, Digest32)> {
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    let observation = state.observation().ok_or_else(|| invalid("no coherent operations observation is published"))?;
    let snapshot = state.snapshot().ok_or_else(|| invariant("operations projection missing"))?;
    if snapshot.anchor() != context.anchor { return Err(DfmcpError::new(ErrorCode::StaleAnchor, "analysis context names another observation")); }
    if snapshot.graph.entities.len() > context.budget.max_entities as usize {
        return Err(exhausted("operations analysis exceeds the session scan budget"));
    }
    if !snapshot.hash_is_valid() { return Err(invariant("operations analysis source hash is invalid")); }
    Ok((observation, snapshot, observation.source_digest()?))
}

/// Computed once per request, not once per job or demand. Chains are iterative
/// and memoized so a deeply nested inventory cannot cause quadratic ancestry work.
struct InventoryIndex {
    ordinal: BTreeMap<u32, usize>,
    parent: Vec<Option<usize>>,
    inherited: Vec<u32>,
    attachment_jobs: BTreeMap<u32, BTreeSet<u32>>,
}
impl InventoryIndex {
    fn build(observation: &LiveOperationsObservation, work: &mut Work) -> Result<Self> {
        let ordinal: BTreeMap<_, _> = observation.items.iter().enumerate().map(|(i, item)| (item.native_id, i)).collect();
        work.charge(observation.items.len() as u64)?;
        let mut attachment_jobs = BTreeMap::<u32, BTreeSet<u32>>::new();
        for attachment in &observation.attachments {
            work.charge(1)?;
            attachment_jobs.entry(attachment.item_native_id).or_default().insert(attachment.job_native_id);
        }
        let mut parent = Vec::with_capacity(observation.items.len());
        let mut direct = Vec::with_capacity(observation.items.len());
        for item in &observation.items {
            work.charge(1)?;
            let ancestor = item.container_native_id.map(|id| ordinal.get(&id).copied()
                .ok_or_else(|| invariant("analysis encountered an unobserved container"))).transpose()?;
            parent.push(ancestor);
            let mut flags = item.flags & DISALLOWED_FLAGS;
            if attachment_jobs.contains_key(&item.native_id) { flags |= ATTACHED; }
            if item.holder_building_native_id.is_some() { flags |= BUILDING_HELD; }
            direct.push(flags);
        }
        let mut inherited = vec![0u32; direct.len()];
        let mut color = vec![0u8; direct.len()];
        for start in 0..direct.len() {
            if color[start] == 2 { continue; }
            let mut path = Vec::new();
            let mut current = Some(start);
            let mut flags = 0;
            while let Some(node) = current {
                work.charge(1)?;
                match color[node] {
                    1 => return Err(invariant("analysis encountered a containment cycle")),
                    2 => { flags = inherited[node]; break; }
                    _ => {}
                }
                color[node] = 1;
                path.push(node);
                current = parent[node];
            }
            for node in path.into_iter().rev() {
                work.charge(1)?;
                flags |= direct[node];
                inherited[node] = flags;
                color[node] = 2;
            }
        }
        Ok(Self { ordinal, parent, inherited, attachment_jobs })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterialDemand {
    pub key: String,
    pub units: u64,
    pub item_types: Vec<String>,
    pub subtype: Option<i32>,
    pub material_type: Option<i32>,
    pub material_index: Option<i32>,
}
impl MaterialDemand {
    fn matches(&self, item: &LiveItem) -> bool {
        self.item_types.binary_search(&item.type_key).is_ok()
            && self.subtype.is_none_or(|v| v == item.subtype)
            && self.material_type.is_none_or(|v| v == item.material_type)
            && self.material_index.is_none_or(|v| v == item.material_index)
    }
}
fn normalized_demands(input: &[MaterialDemand]) -> Result<Vec<MaterialDemand>> {
    if input.is_empty() || input.len() > flow::MAX_DEMANDS { return Err(exhausted("request 1..32 material demands")); }
    for demand in input {
        if demand.key.is_empty() || demand.key.len() > 64 || demand.units == 0
            || !demand.key.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            || demand.item_types.is_empty() || demand.item_types.len() > 8
            || demand.item_types.iter().any(|v| v.is_empty() || v.len() > 128 || v.contains('\0'))
            || demand.subtype.is_some_and(|v| v < -1) || demand.material_type.is_some_and(|v| v < -1)
            || demand.material_index.is_some_and(|v| v < -1)
            || (demand.material_index.is_some() && demand.material_type.is_none()) {
            return Err(invalid("invalid material-demand key, units, type selectors, or raw material pair"));
        }
    }
    let mut demands = input.to_vec();
    demands.sort_by(|a, b| a.key.cmp(&b.key));
    if demands.windows(2).any(|pair| pair[0].key == pair[1].key) { return Err(invalid("material-demand keys must be unique")); }
    for demand in &mut demands { demand.item_types.sort(); demand.item_types.dedup(); }
    Ok(demands)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InventoryAnalysis {
    pub anchor: StateAnchor,
    pub source_digest: Digest32,
    pub demands: Vec<MaterialDemand>,
    pub allocation: Allocation,
    pub candidate_items: u64,
    pub candidate_stack_units: u64,
    /// One primary policy reason per excluded item; categories are disjoint.
    pub excluded_items: BTreeMap<&'static str, u64>,
    pub unmatched_items: u64,
    pub work_units: u64,
}

fn exclusion(flags: u32) -> Option<&'static str> {
    [(8, "removed_in_chain"), (1, "forbidden_in_chain"), (16, "rotten_in_chain"),
        (32, "trader_in_chain"), (ATTACHED, "attached_to_job_in_chain"), (2, "in_job_in_chain"),
        (4, "dump_in_chain"), (128, "in_inventory_in_chain"), (256, "in_building_in_chain"),
        (BUILDING_HELD, "building_holder_in_chain")]
        .into_iter().find_map(|(bit, reason)| (flags & bit != 0).then_some(reason))
}

pub fn plan_inventory(state: &LiveOperationsState, context: &OperationContext,
    requested: &[MaterialDemand], max_work: u64) -> Result<InventoryAnalysis> {
    let (observation, snapshot, source_digest) = source(state, context)?;
    let demands = normalized_demands(requested)?;
    let mut work = Work::new(max_work)?;
    let inventory = InventoryIndex::build(observation, &mut work)?;
    let mut supplies = Vec::new();
    let mut excluded_items = BTreeMap::new();
    let mut unmatched_items = 0;
    let mut candidate_stack_units = 0u64;
    for (i, item) in observation.items.iter().enumerate() {
        work.charge(1)?;
        let reason = if item.stack_size == 0 { Some("zero_stack_size") } else { exclusion(inventory.inherited[i]) };
        if let Some(reason) = reason { *excluded_items.entry(reason).or_default() += 1; continue; }
        let mut eligible = 0u32;
        for (d, demand) in demands.iter().enumerate() {
            work.charge(1)?;
            if demand.matches(item) { eligible |= 1u32 << d; }
        }
        if eligible == 0 { unmatched_items += 1; continue; }
        let units = u64::from(item.stack_size);
        candidate_stack_units = candidate_stack_units.checked_add(units).ok_or_else(|| exhausted("inventory unit sum overflow"))?;
        supplies.push(Supply { id: item_entity_id(item.native_id).get(), units, eligible });
    }
    let model: Vec<_> = demands.iter().map(|d| Demand { key: d.key.clone(), units: d.units }).collect();
    let allocation = flow::allocate(&supplies, &model, work.maximum.saturating_sub(work.used)).map_err(|failure| {
        match failure {
            AllocationError::BudgetExceeded => exhausted("inventory allocation exhausted its shape or operation budget"),
            AllocationError::ArithmeticOverflow => exhausted("inventory allocation exceeds the exact integer domain"),
            AllocationError::InvalidInput => invalid("invalid normalized inventory allocation model"),
            AllocationError::InvariantViolation => invariant("inventory allocation certificate failed"),
        }
    })?;
    work.charge(allocation.work_units)?;
    Ok(InventoryAnalysis { anchor: snapshot.anchor(), source_digest, demands, allocation,
        candidate_items: supplies.len() as u64, candidate_stack_units, excluded_items, unmatched_items, work_units: work.used })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobDiagnosis {
    pub job: AnalysisHandle,
    pub native_job_id: u32,
    pub type_key: String,
    pub suspended: bool,
    pub worker_assigned: bool,
    pub holder: Option<AnalysisHandle>,
    pub holder_stage: Option<(i32, i32)>,
    pub attachment_records: u32,
    pub distinct_attached_items: u32,
    pub required_filter_count: u32,
    pub filters_without_indexed_attachments: u32,
    pub direct_item_flags: BTreeMap<&'static str, u32>,
    /// Count attached descendants affected by a container flag, not containers.
    pub container_item_flags: BTreeMap<&'static str, u32>,
    pub shared_attached_items: u32,
    pub zero_size_attached_items: u32,
    pub affected_item_count: u32,
    pub affected_item_examples: Vec<AnalysisHandle>,
    pub findings: Vec<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionDiagnosis {
    pub anchor: StateAnchor,
    pub source_digest: Digest32,
    pub jobs_considered: u64,
    pub jobs_with_findings: u64,
    pub finding_counts: BTreeMap<&'static str, u64>,
    pub rows: Vec<JobDiagnosis>,
    pub work_units: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiagnosisScope {
    /// Canonical ID and expected generation; raw native IDs are not accepted.
    pub job: Option<(EntityId, u32)>,
    pub holder: Option<(EntityId, u32)>,
    pub include_clear_jobs: bool,
}
fn check_focus(snapshot: &WorldSnapshot, focus: Option<(EntityId, u32)>, kind: EntityKind) -> Result<()> {
    if let Some((id, generation)) = focus {
        let entity = snapshot.graph.entities.get(&id).ok_or_else(|| invalid("diagnostic focus is not observed"))?;
        if entity.kind != kind { return Err(invalid("diagnostic focus has the wrong entity kind")); }
        if entity.generation != generation { return Err(DfmcpError::new(ErrorCode::Conflict, "diagnostic focus generation changed")); }
    }
    Ok(())
}

pub fn diagnose_production(state: &LiveOperationsState, context: &OperationContext,
    scope: DiagnosisScope, max_work: u64) -> Result<ProductionDiagnosis> {
    let (observation, snapshot, source_digest) = source(state, context)?;
    check_focus(snapshot, scope.job, EntityKind::Job)?;
    check_focus(snapshot, scope.holder, EntityKind::Building)?;
    let mut work = Work::new(max_work)?;
    let inventory = InventoryIndex::build(observation, &mut work)?;
    let buildings: BTreeMap<_, _> = observation.buildings.iter().map(|v| (v.native_id, v)).collect();
    work.charge(observation.buildings.len() as u64)?;
    let mut by_job = BTreeMap::<u32, Vec<_>>::new();
    for attachment in &observation.attachments {
        work.charge(1)?;
        by_job.entry(attachment.job_native_id).or_default().push(attachment);
    }
    let mut rows = Vec::new();
    let mut jobs_considered = 0;
    let mut jobs_with_findings = 0;
    let mut finding_counts = BTreeMap::new();
    for job in &observation.jobs.jobs {
        work.charge(1)?;
        let job_id = EntityId::new(u64::from(job.native_id) + 2);
        let holder_id = job.holder_native_id.map(building_entity_id);
        if scope.job.is_some_and(|(id, _)| id != job_id)
            || scope.holder.is_some_and(|(id, _)| Some(id) != holder_id) { continue; }
        jobs_considered += 1;
        let holder = holder_id.map(|id| handle(snapshot, id)).transpose()?;
        let holder_stage = job.holder_native_id.map(|id| buildings.get(&id)
            .map(|b| (b.build_stage, b.max_build_stage)).ok_or_else(|| invariant("diagnosis holder not observed"))).transpose()?;
        let mut items = BTreeSet::new();
        let mut filters = BTreeSet::new();
        for attachment in by_job.get(&job.native_id).into_iter().flatten() {
            work.charge(1)?;
            items.insert(attachment.item_native_id);
            if attachment.filter_index >= 0 { filters.insert(attachment.filter_index as u32); }
        }
        let mut direct_item_flags = BTreeMap::new();
        let mut container_item_flags = BTreeMap::new();
        let mut shared_attached_items = 0;
        let mut zero_size_attached_items = 0;
        let mut affected_item_count = 0;
        let mut affected_item_examples = Vec::new();
        for id in &items {
            work.charge(1)?;
            let i = *inventory.ordinal.get(id).ok_or_else(|| invariant("diagnosis attachment target not observed"))?;
            let item = &observation.items[i];
            let ancestor_flags = inventory.parent[i].map_or(0, |p| inventory.inherited[p]);
            let mut affected = false;
            for (bit, name) in DIAGNOSTIC_FLAGS {
                work.charge(1)?;
                if item.flags & bit != 0 { *direct_item_flags.entry(name).or_default() += 1; affected = true; }
                if ancestor_flags & bit != 0 { *container_item_flags.entry(name).or_default() += 1; affected = true; }
            }
            if inventory.attachment_jobs.get(id).is_some_and(|jobs| jobs.len() > 1) { shared_attached_items += 1; affected = true; }
            if item.stack_size == 0 { zero_size_attached_items += 1; affected = true; }
            if affected {
                affected_item_count += 1;
                if affected_item_examples.len() < EXAMPLES { affected_item_examples.push(handle(snapshot, item_entity_id(*id))?); }
            }
        }
        let uncovered = job.required_item_filter_count.checked_sub(filters.len() as u32)
            .ok_or_else(|| invariant("diagnosis filter index count exceeds native filter count"))?;
        let mut findings = Vec::new();
        if job.suspended { findings.push("job_suspended"); }
        if job.worker_native_id.is_none() { findings.push("no_worker_assigned"); }
        if holder_stage.is_some_and(|(stage, maximum)| stage < maximum) { findings.push("holder_under_construction"); }
        if uncovered > 0 { findings.push("filters_without_indexed_attachments"); }
        if !direct_item_flags.is_empty() { findings.push("attached_item_flags"); }
        if !container_item_flags.is_empty() { findings.push("container_item_flags"); }
        if shared_attached_items > 0 { findings.push("shared_attachment"); }
        if zero_size_attached_items > 0 { findings.push("zero_stack_size_attachment"); }
        if !findings.is_empty() { jobs_with_findings += 1; }
        for code in &findings { *finding_counts.entry(*code).or_default() += 1; }
        if findings.is_empty() && !scope.include_clear_jobs { continue; }
        rows.push(JobDiagnosis { job: handle(snapshot, job_id)?, native_job_id: job.native_id,
            type_key: job.type_key.clone(), suspended: job.suspended, worker_assigned: job.worker_native_id.is_some(),
            holder, holder_stage, attachment_records: job.attached_item_count, distinct_attached_items: items.len() as u32,
            required_filter_count: job.required_item_filter_count, filters_without_indexed_attachments: uncovered,
            direct_item_flags, container_item_flags, shared_attached_items, zero_size_attached_items,
            affected_item_count, affected_item_examples, findings });
    }
    // This is a reproducible inspection order, not a causal or probabilistic score.
    rows.sort_by_key(|row| {
        let flags = |name| row.direct_item_flags.contains_key(name) || row.container_item_flags.contains_key(name);
        (!flags("removed"), !flags("rotten"), !flags("forbidden"), !row.suspended,
            !row.holder_stage.is_some_and(|(stage, maximum)| stage < maximum), row.native_job_id)
    });
    Ok(ProductionDiagnosis { anchor: snapshot.anchor(), source_digest, jobs_considered,
        jobs_with_findings, finding_counts, rows, work_units: work.used })
}
