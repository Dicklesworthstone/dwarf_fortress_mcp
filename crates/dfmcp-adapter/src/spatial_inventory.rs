#![forbid(unsafe_code)]

//! Declared stack-unit allocation inside a single observed terrain component.
//! Candidate routes are not unit navigation, material eligibility or reservations.

use std::collections::{BTreeMap, BTreeSet};
use dfmcp_core::{Capability, DfmcpError, Digest32, EntityId, ErrorCode, OperationContext, Result, RiskTier, StateAnchor};
use dfmcp_world::inventory_allocation::{self as flow, Allocation, Demand, Supply};
use dfmcp_world::map_reachability::Reachability;
use dfmcp_world::map_region::MAX_ROUTE_WORK;
use crate::live_map::map_error;
use crate::live_operations::item_entity_id;
use crate::live_spatial::LiveSpatialState;
use crate::operations_analysis::MaterialDemand;

pub const SPATIAL_SUPPLY_POLICY: &str = "observed-route-unattached-ground-root-stack-units/1";
fn invalid(text: &str) -> DfmcpError { DfmcpError::new(ErrorCode::InvalidRequest, text) }
fn exhausted(text: &str) -> DfmcpError { DfmcpError::new(ErrorCode::BudgetExceeded, text) }
fn invariant(text: &str) -> DfmcpError { DfmcpError::new(ErrorCode::InternalInvariantViolation, text) }
struct Work { used: u64, maximum: u64 }
impl Work {
    fn charge(&mut self, n: u64) -> Result<()> {
        self.used = self.used.checked_add(n).ok_or_else(||exhausted("spatial work overflow"))?;
        if self.used > self.maximum { return Err(exhausted("spatial inventory work exhausted")); }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocatedSupply {
    pub item_id: EntityId,
    pub generation: u32,
    pub outermost_item_id: EntityId,
    pub outermost_generation: u32,
    pub position: [u32; 3],
    pub candidate_steps: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpatialInventory {
    pub anchor: StateAnchor,
    pub source_digest: Digest32,
    pub origin: [u32; 3],
    pub demands: Vec<MaterialDemand>,
    pub allocation: Allocation,
    pub locations: BTreeMap<u64, LocatedSupply>,
    /// One disjoint primary classification for each observed item.
    pub item_counts: BTreeMap<&'static str, u64>,
    pub reachable_tiles: u32,
    pub touched_region_boundary: bool,
    pub work_units: u64,
}
fn normalize(requested: &[MaterialDemand]) -> Result<Vec<MaterialDemand>> {
    if requested.is_empty() || requested.len() > flow::MAX_DEMANDS { return Err(exhausted("request 1..32 declared demands")); }
    for d in requested {
        if d.key.is_empty() || d.key.len() > 64 || d.units == 0
            || !d.key.bytes().all(|b|b.is_ascii_alphanumeric() || b"._-".contains(&b))
            || d.item_types.is_empty() || d.item_types.len() > 8
            || d.item_types.iter().any(|s|s.is_empty() || s.len() > 128 || s.contains('\0'))
            || d.subtype.is_some_and(|n|n < -1) || d.material_type.is_some_and(|n|n < -1)
            || d.material_index.is_some_and(|n|n < -1) || (d.material_index.is_some() && d.material_type.is_none())
        { return Err(invalid("invalid spatial demand key, units, types or material pair")); }
    }
    let mut demands = requested.to_vec();
    demands.sort_by(|a,b|a.key.cmp(&b.key));
    if demands.windows(2).any(|p|p[0].key == p[1].key) { return Err(invalid("duplicate spatial demand key")); }
    for d in &mut demands { d.item_types.sort(); d.item_types.dedup(); }
    Ok(demands)
}

pub fn plan(state: &LiveSpatialState, context: &OperationContext, origin: [u32; 3],
    requested: &[MaterialDemand], maximum_work: u64) -> Result<SpatialInventory> {
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if maximum_work == 0 || maximum_work > flow::MAX_WORK { return Err(exhausted("invalid spatial work budget")); }
    let demands = normalize(requested)?;
    let snapshot = state.snapshot().ok_or_else(||invalid("spatial snapshot is absent"))?;
    if snapshot.anchor() != context.anchor { return Err(DfmcpError::new(ErrorCode::StaleAnchor,"spatial plan uses another observation")); }
    if snapshot.graph.entities.len() > context.budget.max_entities as usize { return Err(exhausted("spatial scan exceeds session budget")); }
    if !snapshot.hash_is_valid() { return Err(invariant("invalid spatial snapshot hash")); }
    let observation = state.observation().ok_or_else(||invariant("spatial observation absent"))?;
    let map = &observation.terrain().map;
    let field = Reachability::compute(map, &[origin], maximum_work.min(MAX_ROUTE_WORK)).map_err(map_error)?;
    let mut work = Work { used: field.work_units, maximum: maximum_work };
    let operations = observation.operations();
    let items = &operations.items;
    work.charge(items.len() as u64)?;
    let ordinal: BTreeMap<_,_> = items.iter().enumerate().map(|(i,item)|(item.native_id,i)).collect();
    let mut attached = BTreeSet::new();
    for a in &operations.attachments { work.charge(1)?; attached.insert(a.item_native_id); }
    let mut parents = Vec::with_capacity(items.len());
    let mut flags = Vec::with_capacity(items.len());
    for item in items {
        work.charge(1)?;
        parents.push(item.container_native_id.map(|id|ordinal.get(&id).copied()
            .ok_or_else(||invariant("unobserved spatial container"))).transpose()?);
        flags.push((item.flags & 0x1bf) | if attached.contains(&item.native_id) {512} else {0}
            | if item.holder_building_native_id.is_some() {1024} else {0});
    }
    let mut color = vec![0u8; items.len()];
    let mut roots = vec![0usize; items.len()];
    for start in 0..items.len() {
        if color[start] == 2 { continue; }
        let mut current = start;
        let mut chain = Vec::new();
        let mut inherited = 0;
        let root;
        loop {
            work.charge(1)?;
            if color[current] == 1 { return Err(invariant("cyclic spatial containment")); }
            if color[current] == 2 { root = roots[current]; inherited = flags[current]; break; }
            color[current] = 1; chain.push(current);
            match parents[current] { Some(parent) => current = parent, None => { root = current; break; } }
        }
        for i in chain.into_iter().rev() {
            work.charge(1)?;
            inherited |= flags[i]; flags[i] = inherited; roots[i] = root; color[i] = 2;
        }
    }
    let mut counts = BTreeMap::new();
    let mut locations = BTreeMap::new();
    let mut supplies = Vec::new();
    for (i,item) in items.iter().enumerate() {
        work.charge(1)?;
        let root = &items[roots[i]];
        let p = root.raw_position;
        let position = [p.x as u32,p.y as u32,p.z as u32];
        let reason = if flags[i] != 0 { Some("excluded_item_or_container_policy") }
            else if item.stack_size == 0 { Some("zero_stack_size") }
            else if root.flags & 64 == 0 || p.x < 0 || p.y < 0 || p.z < 0 { Some("unestablished_ground_location") }
            else if map.region.index(position).is_none() { Some("outside_observed_region") }
            else if field.distance_to(position).is_none() { Some("no_candidate_route_in_observed_model") }
            else { None };
        if let Some(reason) = reason { *counts.entry(reason).or_insert(0u64) += 1; continue; }
        let mut eligible = 0u32;
        for (d,demand) in demands.iter().enumerate() {
            work.charge(1)?;
            if demand.item_types.binary_search(&item.type_key).is_ok()
                && demand.subtype.is_none_or(|n|n == item.subtype)
                && demand.material_type.is_none_or(|n|n == item.material_type)
                && demand.material_index.is_none_or(|n|n == item.material_index)
            { eligible |= 1u32 << d; }
        }
        if eligible == 0 { *counts.entry("no_declared_demand_match").or_insert(0) += 1; continue; }
        if supplies.len() >= flow::MAX_SUPPLIES { return Err(exhausted("too many spatially eligible stacks for bounded allocation")); }
        let id = item_entity_id(item.native_id); let root_id = item_entity_id(root.native_id);
        let entity = snapshot.graph.entities.get(&id).ok_or_else(||invariant("spatial item lacks entity"))?;
        let root_entity = snapshot.graph.entities.get(&root_id).ok_or_else(||invariant("spatial container lacks entity"))?;
        let steps = field.distance_to(position).ok_or_else(||invariant("spatial candidate distance lost"))?;
        locations.insert(id.get(),LocatedSupply {item_id:id,generation:entity.generation,outermost_item_id:root_id,
            outermost_generation:root_entity.generation,position,candidate_steps:steps});
        supplies.push(Supply {id:id.get(),units:u64::from(item.stack_size),eligible});
        *counts.entry("candidate_supply").or_insert(0) += 1;
    }
    let model: Vec<_> = demands.iter().map(|d|Demand {key:d.key.clone(),units:d.units}).collect();
    let allocation = flow::allocate(&supplies,&model,maximum_work.saturating_sub(work.used)).map_err(|e|
        DfmcpError::new(if e == flow::AllocationError::InvariantViolation {ErrorCode::InternalInvariantViolation}
            else {ErrorCode::BudgetExceeded},"spatial allocation exceeded bounds or failed certificate validation"))?;
    work.charge(allocation.work_units)?;
    Ok(SpatialInventory {anchor:snapshot.anchor(),source_digest:state.source_digest()?,origin,demands,allocation,
        locations,item_counts:counts,reachable_tiles:field.visited_tiles,
        touched_region_boundary:field.touched_region_boundary,work_units:work.used})
}
