#![forbid(unsafe_code)]

//! Read-only workforce allocation over one coherent spatial/1.8 capture.
//! Each citizen supplies one indivisible worker slot. Feasibility is conditional
//! on observed skill/readiness and the declared terrain model, never native labor
//! eligibility, a reservation, a schedule, or permission to assign a game job.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;
use dfmcp_core::{Capability, DfmcpError, Digest32, ErrorCode, OperationContext, Result, RiskTier, StateAnchor};
use dfmcp_world::inventory_allocation::{self as flow, Allocation, Demand, Supply};
use dfmcp_world::map_reachability::Reachability;
use dfmcp_world::map_region::{Cell, MapRegion, MAX_ROUTE_WORK};
use crate::live_map::map_error;
use crate::live_spatial::{SpatialStateView, citizens::{LiveSpatialCitizenState, citizen_entity_id}};

#[path = "production_portfolio.rs"]
pub mod portfolio;

#[path = "workforce_quality.rs"]
pub mod quality;

pub const MAX_WORKFORCE_DEMANDS: usize = 16;
pub const MAX_WORKER_SLOTS: u32 = 128;
pub const WORKFORCE_POLICY: &str = "coherent-observed-skill-readiness-dry-occupied-endpoint/1";

fn invalid(text: &str) -> DfmcpError { DfmcpError::new(ErrorCode::InvalidRequest, text) }
fn exhausted(text: &str) -> DfmcpError { DfmcpError::new(ErrorCode::BudgetExceeded, text) }
fn invariant(text: &str) -> DfmcpError { DfmcpError::new(ErrorCode::InternalInvariantViolation, text) }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkforceDemand {
    pub key: String,
    pub workers: u32,
    pub target: [u32; 3],
    pub skill_key: String,
    pub min_effective_skill: i32,
    pub preserve_social: bool,
    pub adults_only: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkforceCandidate {
    pub citizen_index: usize,
    pub entity_id: u64,
    pub generation: u32,
    pub revision: u64,
    pub nominal: i32,
    pub effective: i32,
    pub experience: i32,
    pub steps: u32,
    pub approach: [u32; 3],
    pub endpoint_step: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkforceAnalysis {
    pub anchor: StateAnchor,
    pub source_digest: Digest32,
    pub demands: Vec<WorkforceDemand>,
    /// Candidate rows have a deterministic preference order, not allocation priority.
    pub candidates: Vec<Vec<WorkforceCandidate>>,
    pub classifications: Vec<BTreeMap<&'static str, u64>>,
    pub skill_key_observed: Vec<bool>,
    pub reachable_tiles: Vec<u32>,
    pub touched_region_boundary: Vec<bool>,
    pub work_units: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkforcePlan {
    pub analysis: WorkforceAnalysis,
    /// Supply IDs are citizen entity IDs, and every supply has exactly one unit.
    pub allocation: Allocation,
    pub work_units: u64,
}

struct Work { used: u64, maximum: u64, started: Instant, wall_millis: u64 }
impl Work {
    fn new(context: &OperationContext, maximum: u64) -> Result<Self> {
        if maximum == 0 || maximum > flow::MAX_WORK { return Err(exhausted("invalid workforce work allowance")); }
        Ok(Self { used: 0, maximum, started: Instant::now(), wall_millis: context.budget.max_wall_millis })
    }
    fn charge(&mut self, units: u64) -> Result<()> {
        self.used = self.used.checked_add(units).ok_or_else(|| exhausted("workforce work counter overflow"))?;
        if self.used > self.maximum || self.started.elapsed().as_millis() > u128::from(self.wall_millis) {
            return Err(exhausted("workforce analysis exceeded its work or cooperative wall-time allowance"));
        }
        Ok(())
    }
    fn remaining(&self) -> u64 { self.maximum.saturating_sub(self.used) }
}

fn normalize(requested: &[WorkforceDemand]) -> Result<Vec<WorkforceDemand>> {
    if requested.is_empty() || requested.len() > MAX_WORKFORCE_DEMANDS {
        return Err(exhausted("request 1..16 workforce demands"));
    }
    let mut total = 0u32;
    for demand in requested {
        if demand.key.is_empty() || demand.key.len() > 64
            || !demand.key.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            || demand.workers == 0 || demand.workers > MAX_WORKER_SLOTS
            || demand.skill_key.is_empty() || demand.skill_key.len() > 96
            || demand.skill_key.chars().any(char::is_control) || demand.min_effective_skill < 0
            || demand.target.iter().any(|&n| n >= 32_768) {
            return Err(invalid("invalid workforce key, worker count, target or skill selector"));
        }
        total = total.checked_add(demand.workers).ok_or_else(|| exhausted("worker count overflow"))?;
    }
    if total > MAX_WORKER_SLOTS { return Err(exhausted("workforce request exceeds 128 simultaneous worker slots")); }
    let mut result = requested.to_vec();
    result.sort_by(|a, b| a.key.cmp(&b.key));
    if result.windows(2).any(|pair| pair[0].key == pair[1].key) { return Err(invalid("duplicate workforce demand key")); }
    Ok(result)
}

/// Model only one horizontal exit from an observed otherwise-candidate tile.
/// A neighboring floor cannot turn hidden terrain, a wall, liquid, building
/// occupancy or unknown walkability at the citizen's position into a route.
fn approach(map: &MapRegion, field: &Reachability, position: [i32; 3]) -> Option<([u32; 3], u32, bool)> {
    if position.iter().any(|&n| n < 0) { return None; }
    let p = [position[0] as u32, position[1] as u32, position[2] as u32];
    let index = map.region.index(p)?;
    if let Some(distance) = field.distance_to(p) { return Some((p, distance, false)); }
    let Cell::Visible(mut tile) = *map.cells.get(index)? else { return None; };
    if tile.unit_occupancy == 0 { return None; }
    tile.unit_occupancy = 0;
    if !tile.candidate() { return None; }
    let mut best: Option<([u32; 3], u32, bool)> = None;
    for axis in 0..2 {
        for up in [false, true] {
            let mut next = p;
            next[axis] = match if up { p[axis].checked_add(1) } else { p[axis].checked_sub(1) } {
                Some(value) => value, None => continue,
            };
            let Some(distance) = field.distance_to(next).and_then(|n| n.checked_add(1)) else { continue; };
            if best.is_none_or(|(prior, steps, _)| (distance, next) < (steps, prior)) {
                best = Some((next, distance, true));
            }
        }
    }
    best
}

fn analyze_inner(state: &LiveSpatialCitizenState, context: &OperationContext,
    requested: &[WorkforceDemand], work: &mut Work) -> Result<WorkforceAnalysis> {
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    let demands = normalize(requested)?;
    let snapshot = state.snapshot().ok_or_else(|| invalid("workforce snapshot is absent"))?;
    if snapshot.anchor() != context.anchor { return Err(DfmcpError::new(ErrorCode::StaleAnchor, "workforce context names another capture")); }
    if snapshot.graph.entities.len() > context.budget.max_entities as usize { return Err(exhausted("workforce capture exceeds entity allowance")); }
    if !snapshot.hash_is_valid() { return Err(invariant("workforce snapshot hash is invalid")); }
    let observation = state.observation_full().ok_or_else(|| invariant("coherent workforce source is absent"))?;
    let map = &observation.spatial().terrain().map;
    for demand in &demands {
        let index = map.region.index(demand.target).ok_or_else(|| invalid("workforce target lies outside the captured region"))?;
        if !map.candidate(index) { return Err(invalid("workforce target must be an observed candidate tile, not an occupied workshop or hidden tile")); }
    }
    let mut known_skills = BTreeSet::new();
    for citizen in observation.citizens() {
        work.charge(1)?;
        for skill in &citizen.skills { work.charge(1)?; known_skills.insert(skill.key.as_str()); }
    }
    let mut fields = BTreeMap::new();
    let mut result = WorkforceAnalysis { anchor: snapshot.anchor(), source_digest: state.source_digest()?, demands,
        candidates: Vec::new(), classifications: Vec::new(), skill_key_observed: Vec::new(),
        reachable_tiles: Vec::new(), touched_region_boundary: Vec::new(), work_units: 0 };
    for demand in &result.demands {
        if !fields.contains_key(&demand.target) {
            let field = Reachability::compute(map, &[demand.target], work.remaining().min(MAX_ROUTE_WORK)).map_err(map_error)?;
            work.charge(field.work_units)?;
            fields.insert(demand.target, field);
        }
        let field = fields.get(&demand.target).ok_or_else(|| invariant("workforce route field lost"))?;
        let key_known = known_skills.contains(demand.skill_key.as_str());
        let mut counts = BTreeMap::new();
        let mut rows = Vec::new();
        for (citizen_index, citizen) in observation.citizens().iter().enumerate() {
            work.charge(1)?;
            let reason = if !citizen.alive() || !citizen.sane() || !citizen.active() { Some("inactive_or_unavailable_state") }
                else if demand.adults_only && !citizen.adult() { Some("not_adult") }
                else if demand.preserve_social && !citizen.job_available_preserve_social { Some("not_job_available_preserve_social") }
                else if !demand.preserve_social && !citizen.job_available_interrupt_social { Some("not_job_available_interrupt_social") }
                else if !key_known { Some("skill_key_not_observed") } else { None };
            if let Some(reason) = reason { *counts.entry(reason).or_insert(0) += 1; continue; }
            // Sparse zero defaults are used only for a key represented elsewhere
            // in this capture. An unknown/typo key must not recruit every novice.
            let (mut nominal, mut effective, mut experience) = (0, 0, 0);
            for skill in &citizen.skills {
                work.charge(1)?;
                if skill.key == demand.skill_key { nominal = skill.nominal; effective = skill.effective; experience = skill.experience; break; }
            }
            if effective < demand.min_effective_skill { *counts.entry("below_minimum_effective_skill").or_insert(0) += 1; continue; }
            work.charge(5)?;
            let Some((approach, steps, endpoint_step)) = approach(map, field, [citizen.position.x, citizen.position.y, citizen.position.z]) else {
                *counts.entry("no_candidate_approach_in_observed_model").or_insert(0) += 1; continue;
            };
            work.charge(1)?;
            let id = citizen_entity_id(citizen.native_id);
            let entity = snapshot.graph.entities.get(&id).ok_or_else(|| invariant("workforce citizen entity is absent"))?;
            rows.push(WorkforceCandidate { citizen_index, entity_id: id.get(), generation: entity.generation,
                revision: entity.revision, nominal, effective, experience, steps, approach, endpoint_step });
            *counts.entry("candidate").or_insert(0) += 1;
        }
        // Charge a conservative comparison allowance before deterministic sorting.
        work.charge((rows.len() as u64).saturating_mul(32))?;
        rows.sort_by(|a, b| b.effective.cmp(&a.effective).then_with(|| b.nominal.cmp(&a.nominal))
            .then_with(|| a.steps.cmp(&b.steps)).then_with(|| a.entity_id.cmp(&b.entity_id)));
        result.candidates.push(rows);
        result.classifications.push(counts);
        result.skill_key_observed.push(key_known);
        result.reachable_tiles.push(field.visited_tiles);
        result.touched_region_boundary.push(field.touched_region_boundary);
    }
    work.charge(0)?;
    result.work_units = work.used;
    Ok(result)
}

pub fn analyze(state: &LiveSpatialCitizenState, context: &OperationContext,
    requested: &[WorkforceDemand], maximum_work: u64) -> Result<WorkforceAnalysis> {
    let mut work = Work::new(context, maximum_work)?;
    analyze_inner(state, context, requested, &mut work)
}

pub fn plan(state: &LiveSpatialCitizenState, context: &OperationContext,
    requested: &[WorkforceDemand], maximum_work: u64) -> Result<WorkforcePlan> {
    let mut work = Work::new(context, maximum_work)?;
    let analysis = analyze_inner(state, context, requested, &mut work)?;
    let mut eligibility = BTreeMap::<u64, u32>::new();
    for (demand, rows) in analysis.candidates.iter().enumerate() {
        for row in rows { work.charge(1)?; *eligibility.entry(row.entity_id).or_default() |= 1u32 << demand; }
    }
    let supplies: Vec<_> = eligibility.into_iter().map(|(id, eligible)| Supply { id, units: 1, eligible }).collect();
    let demands: Vec<_> = analysis.demands.iter().map(|d| Demand { key: d.key.clone(), units: u64::from(d.workers) }).collect();
    let allocation = flow::allocate(&supplies, &demands, work.remaining()).map_err(|error| match error {
        flow::AllocationError::InvalidInput | flow::AllocationError::InvariantViolation => invariant("workforce allocation model or certificate disagrees"),
        _ => exhausted("workforce allocation exceeded bounded work or arithmetic"),
    })?;
    work.charge(allocation.work_units)?;
    let mut assigned = BTreeSet::new();
    for assignment in &allocation.assignments {
        if assignment.units != 1 || !assigned.insert(assignment.supply_id) { return Err(invariant("workforce allocation double-booked a citizen")); }
    }
    Ok(WorkforcePlan { analysis, allocation, work_units: work.used })
}

#[cfg(test)]
#[path = "workforce_analysis_tests.rs"]
mod tests;