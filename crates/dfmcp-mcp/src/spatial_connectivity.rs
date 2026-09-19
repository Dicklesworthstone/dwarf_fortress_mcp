//! Whole-region connectivity and disruption assessment on one coherent capture.
//! No new native reads, terrain edits, watches, baselines or reservations.
use super::*;
use std::collections::{BTreeMap, BTreeSet};
use std::cmp::Reverse;
use std::time::Instant;
use dfmcp_world::map_connectivity::{self as model, Classification, Connectivity, CONNECTIVITY_POLICY};
use dfmcp_world::map_region::{MapError, MapRegion};
use serde::Serialize;

const QUERY_POLICY: &str = "dfmcp.map-connectivity-query/1";

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Landmark { key: String, position: [u32; 3] }
#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Section { All, Components, Bottlenecks, Bridges, Landmarks }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    #[serde(default)] landmarks: Vec<Landmark>,
    section: Option<Section>, limit: Option<u32>, continuation: Option<String>, max_work: Option<u64>,
}
fn position(map: &MapRegion, index: usize) -> Result<[u32; 3]> {
    map.region.position(index).ok_or_else(||invalid("connectivity tile index outside the observed region"))
}
fn representative(map: &MapRegion, report: &Connectivity, component: usize) -> Result<[u32; 3]> {
    let component = report.components.get(component).ok_or_else(||invalid("connectivity component missing"))?;
    position(map, component.representative)
}
fn time(context: &OperationContext, started: Instant) -> Result<()> {
    if started.elapsed().as_millis() >= u128::from(context.budget.max_wall_millis) {
        return Err(budget("connectivity exhausted its shared analysis/rendering deadline"));
    }
    Ok(())
}
fn normalize(mut landmarks: Vec<Landmark>) -> Result<Vec<Landmark>> {
    if landmarks.len() > 128 { return Err(budget("connectivity accepts at most 128 landmarks")); }
    for landmark in &landmarks {
        if landmark.key.is_empty() || landmark.key.len() > 64
            || !landmark.key.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            || landmark.position.iter().any(|&n| n >= 32_768) {
            return Err(invalid("landmarks require bounded ASCII keys and coordinates in 0..32767"));
        }
    }
    landmarks.sort_unstable_by(|a,b| a.key.cmp(&b.key));
    if landmarks.windows(2).any(|pair| pair[0].key == pair[1].key) {
        return Err(invalid("connectivity landmark keys must be unique"));
    }
    Ok(landmarks)
}

/// Binds the complete structural result, not only the returned page. Fixed-width
/// big-endian fields and explicit list lengths avoid incidental JSON ordering.
/// At most O(tiles + edges) bytes are retained; no full row set is materialized.
fn structural_digest(map: &MapRegion, report: &Connectivity, source: Digest32,
    context: &OperationContext, started: Instant) -> Result<Digest32> {
    let mut bytes = b"dfmcp-terrain-connectivity-structure/1\0".to_vec();
    bytes.extend_from_slice(CONNECTIVITY_POLICY.as_bytes()); bytes.push(0);
    bytes.extend_from_slice(ROUTE_POLICY.as_bytes()); bytes.push(0);
    bytes.extend_from_slice(source.as_bytes()); bytes.extend_from_slice(context.anchor.state_hash.as_bytes());
    let put = |out: &mut Vec<u8>, n: u64| out.extend_from_slice(&n.to_be_bytes());
    for n in map.region.origin.into_iter().chain(map.region.size) { put(&mut bytes, u64::from(n)); }
    put(&mut bytes, report.component_of.len() as u64);
    for (i, component) in report.component_of.iter().enumerate() {
        if i % 128 == 0 { time(context, started)?; }
        // An explicit presence tag avoids reserving a magic valid component ID.
        bytes.push(u8::from(component.is_some()));
        if let Some(component) = component { put(&mut bytes, *component as u64); }
    }
    put(&mut bytes, report.components.len() as u64);
    for c in &report.components {
        time(context, started)?;
        for n in [c.representative as u64, u64::from(c.tiles), u64::from(c.edges)] { put(&mut bytes, n); }
        for n in c.min.into_iter().chain(c.max) { put(&mut bytes, u64::from(n)); }
        bytes.push(u8::from(c.touches_region_boundary));
    }
    put(&mut bytes, report.bottlenecks.len() as u64);
    for b in &report.bottlenecks {
        time(context, started)?;
        for n in [b.tile as u64, b.component as u64, b.separated_tile_pairs, b.partition_sizes.len() as u64] { put(&mut bytes, n); }
        for n in &b.partition_sizes { put(&mut bytes, u64::from(*n)); }
    }
    put(&mut bytes, report.bridges.len() as u64);
    for b in &report.bridges {
        time(context, started)?;
        for n in [b.endpoints[0] as u64, b.endpoints[1] as u64, b.component as u64,
            u64::from(b.side_sizes[0]), u64::from(b.side_sizes[1])] { put(&mut bytes, n); }
    }
    Ok(Digest32::of_bytes(&bytes))
}

pub(super) fn execute<S: SpatialStateView>(state: &S, context: &OperationContext,
    source: Digest32, request: Request) -> Result<Value> {
    let started = Instant::now(); time(context, started)?;
    let landmarks = normalize(request.landmarks)?;
    let section = request.section.unwrap_or(Section::All);
    let limit = request.limit.unwrap_or(8);
    if !(1..=128).contains(&limit) || context.budget.max_entities == 0
        || request.continuation.as_ref().is_some_and(|c| c.len() > 128) {
        return Err(budget("connectivity page arguments exceed bounds"));
    }
    let max_work = request.max_work.unwrap_or(1_000_000);
    let map = &state.spatial_observation().ok_or_else(||invalid("connectivity requires a spatial capture"))?.terrain().map;
    let report = model::analyze_with_check(map, max_work, || {
        if started.elapsed().as_millis() >= u128::from(context.budget.max_wall_millis) {
            Err(MapError::BudgetExceeded)
        } else { Ok(()) }
    }).map_err(map_error)?;
    let structural_digest = structural_digest(map, &report, source, context, started)?;
    let mut classes = BTreeMap::<&str,u32>::new();
    for cell in &map.cells { *classes.entry(model::classify(*cell).name()).or_default() += 1; }
    let mut established = 0usize;
    let mut landmark_components = BTreeSet::new();
    for landmark in &landmarks {
        if let Some(component) = map.region.index(landmark.position).and_then(|i|report.component_of[i]) {
            established += 1; landmark_components.insert(component);
        }
    }
    let connected = if landmarks.is_empty() || established != landmarks.len() { None }
        else { Some(landmark_components.len() == 1) };
    let mut bottlenecks: Vec<_> = (0..report.bottlenecks.len()).collect();
    bottlenecks.sort_unstable_by_key(|&i| (Reverse(report.bottlenecks[i].separated_tile_pairs), report.bottlenecks[i].tile));
    let mut bridges: Vec<_> = (0..report.bridges.len()).collect();
    bridges.sort_unstable_by_key(|&i| {
        let b = &report.bridges[i];
        (Reverse(u64::from(b.side_sizes[0]) * u64::from(b.side_sizes[1])), b.endpoints)
    });
    let id = identity(context, source, json!({"kind":"map_connectivity","policy":QUERY_POLICY,
        "structure":structural_digest.to_string(),"landmarks":landmarks,"section":section,"max_work":max_work}));
    let mut out = base(context, source, "map_connectivity");
    out["connectivity_policy"] = json!(CONNECTIVITY_POLICY); out["query_policy"] = json!(QUERY_POLICY);
    out["structural_digest"] = json!(structural_digest.to_string()); out["analysis_complete"] = json!(true);
    out["native_captures"] = json!(0); out["mutation_dispatched"] = json!(false);
    out["region"] = json!({"origin":map.region.origin,"size":map.region.size});
    out["summary"] = json!({"candidate_tiles":report.candidate_tiles,"model_edges":report.edges,
        "components":report.components.len(),"bottlenecks":report.bottlenecks.len(),"graph_bridges":report.bridges.len(),
        "tile_classification":classes,"structural_work_units":report.work_units});
    out["landmark_summary"] = json!({"requested":landmarks.len(),"established":established,
        "unestablished":landmarks.len()-established,"distinct_observed_components":landmark_components.len(),
        "all_connected_in_observed_model":connected});
    out["coverage"] = json!({"domain":"captured_candidate_tile_graph","continuous_history":false,
        "outside_region_paths":"unknown","native_unit_rules":"not_modeled","global_connectivity_proven":false,
        "evacuation_safety_proven":false,"bridge_means":"graph_cut_edge_not_a_DF_bridge_building"});
    out["interpretation"] = json!("Components and cuts are exact only in the captured dry cardinal floor/stair model. Occupancy, hidden cells and unsupported movement can omit real routes. Removing a tile or edge is a graph counterfactual, not a terrain edit, diagnosis, safe demolition or evacuation guarantee.");
    out["order"] = json!("landmarks_by_key_then_bottlenecks_by_separated_pairs_desc_then_bridges_by_separated_pairs_desc_then_components_by_representative");
    let counts = [
        if matches!(section,Section::All|Section::Landmarks) {landmarks.len()} else {0},
        if matches!(section,Section::All|Section::Bottlenecks) {bottlenecks.len()} else {0},
        if matches!(section,Section::All|Section::Bridges) {bridges.len()} else {0},
        if matches!(section,Section::All|Section::Components) {report.components.len()} else {0},
    ];
    let result = paginate(out, counts.iter().sum(), request.continuation.as_deref(),
        limit.min(context.budget.max_entities), id, context, |mut index| {
            time(context, started)?;
            if index < counts[0] {
                let l = &landmarks[index]; let cell = map.region.index(l.position);
                let component = cell.and_then(|i|report.component_of[i]);
                let classification = cell.map(|i|model::classify(map.cells[i]));
                return Ok(json!({"row_kind":"landmark","key":l.key,"position":l.position,
                    "status":if classification==Some(Classification::Candidate) {"candidate"} else {"unestablished"},
                    "reason":classification.map_or("outside_observed_region",Classification::name),
                    "component_representative":component.map(|c|representative(map,&report,c)).transpose()?}));
            }
            index -= counts[0];
            if index < counts[1] {
                let b = &report.bottlenecks[bottlenecks[index]];
                return Ok(json!({"row_kind":"bottleneck","position":position(map,b.tile)?,
                    "component_representative":representative(map,&report,b.component)?,
                    "remaining_partition_sizes":b.partition_sizes,"separated_tile_pairs":b.separated_tile_pairs,
                    "removed_candidate_tiles":1,"native_disconnection_proven":false}));
            }
            index -= counts[1];
            if index < counts[2] {
                let b = &report.bridges[bridges[index]];
                return Ok(json!({"row_kind":"graph_bridge","endpoints":[position(map,b.endpoints[0])?,position(map,b.endpoints[1])?],
                    "component_representative":representative(map,&report,b.component)?,"side_sizes":b.side_sizes,
                    "separated_tile_pairs":u64::from(b.side_sizes[0])*u64::from(b.side_sizes[1]),
                    "removed_candidate_edges":1,"native_disconnection_proven":false}));
            }
            index -= counts[2];
            let c = report.components.get(index).ok_or_else(||invalid("connectivity page index invalid"))?;
            Ok(json!({"row_kind":"component","representative_position":position(map,c.representative)?,
                "candidate_tiles":c.tiles,"model_edges":c.edges,"min":c.min,"max":c.max,
                "touches_region_boundary":c.touches_region_boundary}))
        })?;
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    time(context, started)?;
    Ok(result)
}

#[cfg(test)]
#[path = "spatial_connectivity_tests.rs"]
mod tests;
