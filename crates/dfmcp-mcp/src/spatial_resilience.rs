//! Endpoint-specific independent routes and minimum separators on one capture.
//! Uses existing spatial publication and pagination; no bridge reads or effects.
use super::*;
use dfmcp_world::map_connectivity::classify;
use dfmcp_world::map_region::{MAX_ROUTE_WORK, MapError, MapRegion};
use dfmcp_world::map_resilience::{
    self as model, CutElement, Disjointness, RESILIENCE_POLICY, RouteResilience,
};
use serde::Serialize;
use std::time::Instant;

const QUERY_POLICY: &str = "dfmcp.map-route-resilience-query/1";

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Edges,
    InternalTiles,
}
impl Mode {
    const fn model(self) -> Disjointness {
        match self {
            Self::Edges => Disjointness::Edges,
            Self::InternalTiles => Disjointness::InternalTiles,
        }
    }
}
#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Section {
    Cut,
    Paths,
    All,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    start: [u32; 3],
    goal: [u32; 3],
    disjointness: Mode,
    section: Option<Section>,
    limit: Option<u32>,
    continuation: Option<String>,
    max_work: Option<u64>,
}
fn guard(context: &OperationContext, started: Instant) -> Result<()> {
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if started.elapsed().as_millis() >= u128::from(context.budget.max_wall_millis) {
        return Err(budget("route resilience exhausted its analysis/rendering deadline"));
    }
    Ok(())
}
fn position(map: &MapRegion, index: usize) -> Result<[u32; 3]> {
    map.region.position(index)
        .ok_or_else(|| invalid("route witness index outside the captured map"))
}
fn source_guard<S: SpatialStateView + OperationsStateView>(
    state: &S,
    context: &OperationContext,
    source: Digest32,
) -> Result<()> {
    let snapshot = state.snapshot().ok_or_else(|| invalid("spatial snapshot absent"))?;
    if snapshot.anchor() != context.anchor || state.source_digest()? != source {
        return Err(DfmcpError::new(ErrorCode::StaleAnchor, "route resilience source changed"));
    }
    if !snapshot.hash_is_valid() {
        return Err(DfmcpError::new(ErrorCode::InternalInvariantViolation, "invalid spatial snapshot hash"));
    }
    Ok(())
}

/// One canonical commitment covers the complete packing and cut, even when the
/// response asks only for the compact cut view or one page of route positions.
fn structural_digest(
    map: &MapRegion,
    report: &RouteResilience,
    source: Digest32,
    context: &OperationContext,
    maximum_work: u64,
    started: Instant,
) -> Result<Digest32> {
    let mut bytes = b"dfmcp-route-resilience-structure/1\0".to_vec();
    bytes.extend_from_slice(RESILIENCE_POLICY.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(ROUTE_POLICY.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(source.as_bytes());
    bytes.extend_from_slice(&context.session_id.get().to_be_bytes());
    bytes.extend_from_slice(&context.anchor.fortress_id.get().to_be_bytes());
    bytes.extend_from_slice(&context.anchor.cursor.epoch.to_be_bytes());
    bytes.extend_from_slice(&context.anchor.cursor.sequence.to_be_bytes());
    bytes.extend_from_slice(&context.anchor.tick.0.to_be_bytes());
    bytes.extend_from_slice(context.anchor.state_hash.as_bytes());
    let put = |bytes: &mut Vec<u8>, n: u64| bytes.extend_from_slice(&n.to_be_bytes());
    for n in map.region.origin.into_iter().chain(map.region.size)
        .chain(report.start).chain(report.goal)
    {
        put(&mut bytes, u64::from(n));
    }
    bytes.push(match report.disjointness { Disjointness::Edges => 0, Disjointness::InternalTiles => 1 });
    bytes.push(u8::from(report.endpoint_excluded));
    for n in [maximum_work, report.work_units, u64::from(report.candidate_tiles),
        u64::from(report.candidate_edges), report.minimum_cut.len() as u64]
    {
        put(&mut bytes, n);
    }
    for cut in &report.minimum_cut {
        guard(context, started)?;
        match *cut {
            CutElement::Tile(tile) => { bytes.push(0); put(&mut bytes, tile as u64); }
            CutElement::Edge([a, b]) => {
                bytes.push(1); put(&mut bytes, a as u64); put(&mut bytes, b as u64);
            }
        }
    }
    put(&mut bytes, report.paths.len() as u64);
    for path in &report.paths {
        put(&mut bytes, path.len() as u64);
        for (i, &tile) in path.iter().enumerate() {
            if i.is_multiple_of(128) { guard(context, started)?; }
            put(&mut bytes, tile as u64);
        }
    }
    guard(context, started)?;
    Ok(Digest32::of_bytes(&bytes))
}

pub(super) fn execute<S: SpatialStateView + OperationsStateView>(
    state: &S,
    context: &OperationContext,
    source: Digest32,
    request: Request,
) -> Result<Value> {
    let started = Instant::now();
    guard(context, started)?;
    source_guard(state, context, source)?;
    if request.start == request.goal {
        return Err(invalid("route resilience requires distinct endpoints"));
    }
    let limit = request.limit.unwrap_or(16);
    let maximum_work = request.max_work.unwrap_or(MAX_ROUTE_WORK);
    if !(1..=128).contains(&limit) || context.budget.max_entities == 0
        || maximum_work == 0 || maximum_work > MAX_ROUTE_WORK
        || request.continuation.as_ref().is_some_and(|value| value.len() > 128)
    {
        return Err(budget("route resilience page/work arguments exceed bounds"));
    }
    let section = request.section.unwrap_or(Section::Cut);
    let mode = request.disjointness.model();
    let map = &state.spatial_observation()
        .ok_or_else(|| invalid("route resilience requires a spatial capture"))?
        .terrain().map;
    let mut interrupted = None;
    let attempt = model::analyze_with_check(map, request.start, request.goal, mode, maximum_work, || {
        match guard(context, started) {
            Ok(()) => Ok(()),
            Err(error) => { interrupted = Some(error); Err(MapError::BudgetExceeded) }
        }
    });
    let report = match attempt {
        Ok(report) => report,
        Err(error) => return Err(match interrupted { Some(error) => error, None => map_error(error) }),
    };
    let structure = structural_digest(map, &report, source, context, maximum_work, started)?;
    let id = identity(context, source, json!({"kind":"map_route_resilience",
        "policy":QUERY_POLICY,"structure":structure.to_string(),"section":section,
        "start":request.start,"goal":request.goal,"disjointness":request.disjointness,
        "max_work":maximum_work}));
    let established = !report.endpoint_excluded;
    let count = established.then_some(report.paths.len());
    let source_tile = map.region.index(request.start).ok_or_else(|| invalid("start outside map"))?;
    let target_tile = map.region.index(request.goal).ok_or_else(|| invalid("goal outside map"))?;
    let direct_edge = [source_tile.min(target_tile), source_tile.max(target_tile)];
    let direct_cut = report.minimum_cut.contains(&CutElement::Edge(direct_edge));
    let mixed = mode == Disjointness::InternalTiles && direct_cut;
    let mut out = base(context, source, "map_route_resilience");
    out["query_policy"] = json!(QUERY_POLICY);
    out["resilience_policy"] = json!(RESILIENCE_POLICY);
    out["structural_digest"] = json!(structure.to_string());
    out["analysis_complete"] = json!(established);
    out["certificate_verified"] = json!(established);
    out["native_captures"] = json!(0);
    out["mutation_dispatched"] = json!(false);
    out["region"] = json!({"origin":map.region.origin,"size":map.region.size});
    out["start"] = json!(request.start);
    out["goal"] = json!(request.goal);
    out["endpoint_classification"] = json!({
        "start":classify(map.cells[map.region.index(request.start).ok_or_else(|| invalid("start outside map"))?]).name(),
        "goal":classify(map.cells[map.region.index(request.goal).ok_or_else(|| invalid("goal outside map"))?]).name()});
    out["disjointness"] = json!(request.disjointness);
    out["section"] = json!(section);
    out["status"] = json!(if !established { "endpoint_excluded" }
        else if report.paths.is_empty() { "no_route_in_observed_model" }
        else if report.paths.len() == 1 { "single_candidate_route" }
        else { "redundant_candidate_routes" });
    out["summary"] = json!({"maximum_disjoint_paths":count,"minimum_separator_elements":count,
        "candidate_tiles":report.candidate_tiles,"candidate_edges":report.candidate_edges,
        "path_steps":report.paths.iter().map(|path|path.len().checked_sub(1)).collect::<Vec<_>>(),
        "structural_work_units":report.work_units,"structural_work_limit":maximum_work,
        "internal_tiles_may_be_shared":mode==Disjointness::Edges});
    out["certificate"] = json!({"kind":"feasible_packing_equal_separating_cut",
        "verified_in_observed_model":established,"packing_size":count,"separator_size":count,
        "contains_direct_endpoint_edge":direct_cut,
        "mixed_internal_tile_and_direct_edge_cut":mixed,
        "pure_internal_tile_cut_size":if established && mode==Disjointness::InternalTiles && !mixed { count } else { None },
        "endpoint_deletion_permitted":false,"complete_witness_digest":structure.to_string()});
    out["coverage"] = json!({"domain":"captured_candidate_tile_graph",
        "outside_region_paths":"unknown","native_unit_rules":"not_modeled",
        "native_throughput_measured":false,"endpoint_failure_protection":false,
        "evacuation_safety_proven":false,"continuous_history":false});
    out["interpretation"] = json!(
        "Exact route packing and separator only in this captured dry cardinal floor/stair graph. Edge-disjoint paths may share tiles. Internal-tile mode shares endpoints; adjacent endpoints require a direct-edge plus internal-tile separator, not a pure tile cut. Counts are not native throughput, global absence, evacuation safety or permission to alter terrain."
    );
    out["order"] = json!("minimum_cut_tiles_then_edges_then_lexicographic_paths_and_position_ordinals");
    let cut_rows = if matches!(section, Section::Cut | Section::All) { report.minimum_cut.len() } else { 0 };
    let path_rows = if matches!(section, Section::Paths | Section::All) {
        report.paths.iter().map(Vec::len).sum::<usize>()
    } else { 0 };
    let result = paginate(out, cut_rows + path_rows, request.continuation.as_deref(),
        limit.min(context.budget.max_entities), id, context, |mut row| {
            guard(context, started)?;
            if row < cut_rows {
                return match report.minimum_cut[row] {
                    CutElement::Tile(tile) => Ok(json!({"row_kind":"minimum_cut_tile","position":position(map,tile)?})),
                    CutElement::Edge([a,b]) => Ok(json!({"row_kind":"minimum_cut_edge",
                        "endpoints":[position(map,a)?,position(map,b)?],"direct_endpoint_edge":direct_edge==[a,b]})),
                };
            }
            row -= cut_rows;
            for (index, path) in report.paths.iter().enumerate() {
                if row < path.len() {
                    return Ok(json!({"row_kind":"path_position","path":index,"ordinal":row,
                        "position":position(map,path[row])?,"path_positions":path.len()}));
                }
                row -= path.len();
            }
            Err(invalid("route resilience page index invalid"))
        })?;
    guard(context, started)?;
    source_guard(state, context, source)?;
    guard(context, started)?;
    Ok(result)
}

#[cfg(test)]
#[path = "spatial_resilience_tests.rs"]
mod tests;
