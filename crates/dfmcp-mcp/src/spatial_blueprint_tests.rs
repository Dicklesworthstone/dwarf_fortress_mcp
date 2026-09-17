use super::*;
use super::super::{execute as query, handles, schema};
use dfmcp_adapter::live_spatial::{LiveSpatialObservation, LiveSpatialState};
use dfmcp_core::{Capability, CapabilityGrant, CapabilityScope, ErrorCode, RequestId, RiskTier, SessionId, WorkBudget};
use dfmcp_world::map_region::{MapRegion, Region, Shape, Tile};

fn terrain() -> Result<LiveMapObservation> {
    let region = Region { origin: [0, 0, 4], size: [12, 12, 3] };
    let tile = Cell::Visible(Tile { native_tiletype: 2, shape: Shape::Wall, liquid_depth: 0,
        magma: false, traffic: 0, dig_designation: 0, building_occupancy: 0,
        unit_occupancy: 0, walkable_region: 0, temperature_1: 10015, temperature_2: 10015 });
    Ok(LiveMapObservation { bridge_generation: 7, df_version: "df".to_owned(),
        dfhack_version: "dfhack".to_owned(), year: 105, year_tick: 100, paused: true,
        site_id: 1, world_folder: "fixture".to_owned(), map_dimensions: [32, 32, 8],
        map: MapRegion { region, cells: vec![tile; region.volume().map_err(|_| invalid("test region"))?] } })
}

fn hall(origin: MapCoord) -> Result<BlueprintLayout> {
    BlueprintPlanner.layout(origin, BlueprintTemplate::DiningHall { width: 3, height: 3 })
}

#[test]
fn visible_attributes_never_fill_in_hidden_or_unallocated_tiles() -> Result<()> {
    let mut map = terrain()?;
    let hidden = map.map.region.index([4, 2, 5]).ok_or_else(|| invalid("hidden fixture cell"))?;
    let unallocated = map.map.region.index([5, 2, 5]).ok_or_else(|| invalid("unallocated fixture cell"))?;
    let wet = map.map.region.index([6, 2, 5]).ok_or_else(|| invalid("wet fixture cell"))?;
    map.map.cells[hidden] = Cell::Hidden;
    map.map.cells[unallocated] = Cell::Unallocated;
    if let Cell::Visible(tile) = &mut map.map.cells[wet] {
        tile.liquid_depth = 7; tile.magma = true; tile.building_occupancy = 1;
        tile.unit_occupancy = 2; tile.dig_designation = 1;
    }
    let (summary, rows) = analyze(&map, &hall(MapCoord { x: 4, y: 2, z: 5 })?, MAX_WORK)?;
    assert_eq!(summary["excavated_tiles"], 9);
    assert_eq!(summary["footprint"]["visible"], 7);
    assert_eq!(summary["footprint"]["hidden"], 1);
    assert_eq!(summary["footprint"]["unallocated"], 1);
    assert_eq!(summary["footprint"]["all_positions_visible"], false);
    assert_eq!(summary["footprint"]["visible_attributes"], json!({"liquid_tiles":1,"magma_liquid_tiles":1,
        "building_occupied_tiles":1,"unit_occupied_tiles":1,"existing_designation_tiles":1,"shapes":{"wall":7}}));
    assert_eq!(summary["halo"]["unique_positions"], 75);
    assert_eq!(summary["halo"]["coverage"]["visible"], 73);
    assert_eq!(rows[0]["coverage"], summary["footprint"]);
    Ok(())
}

#[test]
fn uncaptured_positions_and_map_edges_are_explicit_not_safe_empty_space() -> Result<()> {
    let map = terrain()?;
    let (summary, _) = analyze(&map, &hall(MapCoord { x: 20, y: 20, z: 5 })?, MAX_WORK)?;
    assert_eq!(summary["footprint"]["outside_capture"], 9);
    assert_eq!(summary["footprint"]["outside_map"], 0);
    assert_eq!(summary["footprint"]["visible"], 0);
    assert_eq!(summary["footprint"]["visible_attributes"]["shapes"], json!({}));
    let layout = BlueprintPlanner.layout(MapCoord { x: 0, y: 0, z: 5 }, BlueprintTemplate::WorkshopHub { bays_count: 1 })?;
    let (summary, _) = analyze(&map, &layout, MAX_WORK)?;
    assert_eq!(summary["footprint"]["outside_map"], 2);
    assert_eq!(summary["footprint"]["all_positions_visible"], false);
    Ok(())
}

#[test]
fn halo_requires_observations_above_and_below_the_footprint() -> Result<()> {
    let mut map = terrain()?;
    map.map.region.origin[2] = 5;
    map.map.region.size[2] = 1;
    map.map.cells.truncate(144);
    let (summary, _) = analyze(&map, &hall(MapCoord { x: 4, y: 2, z: 5 })?, MAX_WORK)?;
    assert_eq!(summary["footprint"]["all_positions_visible"], true);
    assert_eq!(summary["halo"]["coverage"]["all_positions_visible"], false);
    assert_eq!(summary["halo"]["coverage"]["outside_capture"], 50);
    Ok(())
}

#[test]
fn crossing_is_measured_but_never_counted_as_excavation_or_a_built_bridge() -> Result<()> {
    let layout = BlueprintPlanner.layout(MapCoord { x: 2, y: 2, z: 5 }, BlueprintTemplate::DefensiveMoat {
        perimeter_cuboid: MapCuboid::new(MapCoord { x: 2, y: 2, z: 5 }, MapCoord { x: 8, y: 8, z: 5 })?,
        drawbridge_span: 3,
    })?;
    let (summary, _) = analyze(&terrain()?, &layout, MAX_WORK)?;
    assert_eq!(summary["excavated_tiles"], 21);
    assert_eq!(summary["reserved_crossing"]["coverage"]["visible"], 3);
    assert_eq!(summary["reserved_crossing"]["bridge_constructed"], false);
    assert_eq!(summary["reserved_crossing"]["reservation_created"], false);
    Ok(())
}

#[test]
fn complete_work_bound_is_checked_before_scanning_and_corruption_is_not_absence() -> Result<()> {
    let map = terrain()?;
    let layout = hall(MapCoord { x: 4, y: 2, z: 5 })?;
    let (summary, _) = analyze(&map, &layout, MAX_WORK)?;
    let cost = summary["scan_work_upper_bound"].as_u64().ok_or_else(|| invalid("test work bound"))?;
    assert_eq!(analyze(&map, &layout, cost)?.0, summary);
    for bound in [0, 1, cost - 1, MAX_WORK + 1] {
        assert!(matches!(analyze(&map, &layout, bound), Err(e) if e.code == ErrorCode::BudgetExceeded));
    }
    let mut corrupt = map;
    corrupt.map.cells.pop();
    assert!(analyze(&corrupt, &layout, MAX_WORK).is_err());
    Ok(())
}

fn state() -> Result<(LiveSpatialState, OperationContext)> {
    let text = include_str!("../../dfmcp-adapter/tests/fixtures/spatial_v1_6.hex").trim();
    let bytes = (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i+2], 16)
        .map_err(|_| invalid("spatial test fixture"))).collect::<Result<Vec<_>>>()?;
    let mut state = LiveSpatialState::default();
    state.publish(LiveSpatialObservation::decode_payload(&bytes, 7, "df".to_owned(), "dfhack".to_owned())?)?;
    let anchor = state.snapshot().ok_or_else(|| invalid("test snapshot"))?.anchor();
    let c = OperationContext { session_id: SessionId::new(1), request_id: RequestId::new(2), anchor,
        budget: WorkBudget { max_entities: 100_000, max_bytes: 100_000, max_output_tokens: 25_000, ..WorkBudget::default() },
        grants: vec![CapabilityGrant { capability: Capability::Query, scope: CapabilityScope::default(),
            max_risk: RiskTier::ReadOnly, expires_at_tick: None, remaining_uses: None }], cancellation_requested: false };
    Ok((state, c))
}

fn request(limit: u32) -> Value {
    json!({"schema":"dfmcp.query/1","query":{"kind":"blueprint_layout","origin":[2,0,5],
        "template":{"kind":"bedroom_cluster","rooms_count":4,"room_size":[3,3]},"limit":limit}})
}

#[test]
fn shared_spatial_dispatch_returns_anchor_bound_preview_not_a_prepared_plan() -> Result<()> {
    let (state, c) = state()?;
    let input = request(128);
    assert!(handles(&input));
    let out = query(&state, &c, &input)?;
    assert_eq!(out["kind"], "blueprint_layout");
    assert_eq!(out["source_digest"], state.source_digest()?.to_string());
    assert_eq!(out["total_rows"], 10);
    assert_eq!(out["returned"], 10);
    for flag in ["safety_proven","unit_path_proven","plan_created","commit_compatible",
        "reservation_created","completion_proven","excavation_eligibility_proven"] { assert_eq!(out[flag], false); }
    assert!(schema()?["$defs"]["query"]["oneOf"].as_array().ok_or_else(|| invalid("test schema"))?
        .iter().any(|v| v["properties"]["kind"]["const"] == "blueprint_layout"));
    Ok(())
}

#[test]
fn pages_can_change_width_but_not_session_layout_or_work_policy() -> Result<()> {
    let (state, c) = state()?;
    let full = query(&state, &c, &request(128))?;
    let first = query(&state, &c, &request(1))?;
    let mut next = request(128);
    next["query"]["continuation"] = first["continuation"].clone();
    let tail = query(&state, &c, &next)?;
    assert_eq!(tail["analysis_digest"], first["analysis_digest"]);
    assert_eq!(tail["rows"], json!(&full["rows"].as_array().ok_or_else(|| invalid("test rows"))?[1..]));
    assert_eq!(tail["summary"], first["summary"]);
    assert_eq!(tail, query(&state, &c, &next)?);
    for field in ["origin","template","max_work"] {
        let mut changed = next.clone();
        changed["query"][field] = match field {
            "origin" => json!([3,0,5]),
            "template" => json!({"kind":"bedroom_cluster","rooms_count":3,"room_size":[3,3]}),
            _ => json!(MAX_WORK - 1),
        };
        assert!(matches!(query(&state, &c, &changed), Err(e) if e.code == ErrorCode::StaleAnchor));
    }
    let mut other = c.clone(); other.session_id = SessionId::new(99);
    assert!(matches!(query(&state, &other, &next), Err(e) if e.code == ErrorCode::StaleAnchor));
    Ok(())
}

#[test]
fn authority_staleness_output_limits_and_mutation_fields_fail_closed() -> Result<()> {
    let (state, c) = state()?;
    let input = request(1);
    let mut denied = c.clone(); denied.grants.clear();
    assert!(matches!(query(&state, &denied, &input), Err(e) if e.code == ErrorCode::CapabilityDenied));
    let mut stale = c.clone(); stale.anchor.tick.0 += 1;
    assert!(matches!(query(&state, &stale, &input), Err(e) if e.code == ErrorCode::StaleAnchor));
    let mut tiny = c.clone(); tiny.budget.max_bytes = 1;
    assert!(matches!(query(&state, &tiny, &input), Err(e) if e.code == ErrorCode::BudgetExceeded));
    for limit in [0, 129] { assert!(query(&state, &c, &request(limit)).is_err()); }
    for field in ["commit","raw_lua","override_environmental_hazards"] {
        let mut bad = input.clone(); bad["query"][field] = json!(true);
        assert!(matches!(query(&state, &c, &bad), Err(e) if e.code == ErrorCode::InvalidRequest));
    }
    let mut bad = input.clone(); bad["query"]["template"]["raw_lua"] = json!("not executable");
    assert!(query(&state, &c, &bad).is_err());
    let mut bad = input; bad["query"]["origin"] = json!([32768,0,5]);
    assert!(query(&state, &c, &bad).is_err());
    Ok(())
}
