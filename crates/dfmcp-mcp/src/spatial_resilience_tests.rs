//! Actual spatial codecs, canonical publication and structured query dispatcher.
//! These tests do not connect to DFHack or assert native movement semantics.
use super::*;
use dfmcp_adapter::live_operations::OperationsProfile;
use dfmcp_adapter::live_spatial::{
    LiveSpatialObservation, LiveSpatialState,
    citizens::{LiveSpatialCitizenObservation, LiveSpatialCitizenState},
};
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, EntityId, GameTick, RequestId, SessionId, WorkBudget,
};
use dfmcp_world::map_region::{Cell, Region, Shape, Tile};
use std::collections::BTreeSet;

fn floor() -> Cell {
    Cell::Visible(Tile {
        native_tiletype: 1,
        shape: Shape::Floor,
        liquid_depth: 0,
        magma: false,
        traffic: 0,
        dig_designation: 0,
        building_occupancy: 0,
        unit_occupancy: 0,
        walkable_region: 1,
        temperature_1: 10015,
        temperature_2: 10015,
    })
}
fn part(out: &mut Vec<u8>, payload: &[u8]) {
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
}
fn payload(size: [u32; 3], hidden: &[usize], tick: u32) -> Result<Vec<u8>> {
    let raw = include_str!("../../dfmcp-adapter/tests/fixtures/spatial_v1_6.hex").trim();
    let bytes = (0..raw.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&raw[i..i + 2], 16).map_err(|_| invalid("fixture hex")))
        .collect::<Result<Vec<_>>>()?;
    let first = LiveSpatialObservation::decode_payload(&bytes, 7, "df".into(), "dfhack".into())?;
    let mut operations = first.operations().clone();
    let mut terrain = first.terrain().clone();
    operations.jobs.year_tick = tick;
    terrain.year_tick = tick;
    let region = Region {
        origin: [0, 0, 5],
        size,
    };
    for (axis, dimension) in terrain.map_dimensions.iter_mut().enumerate() {
        *dimension = (*dimension).max(region.origin[axis] + region.size[axis]);
    }
    let mut cells = vec![floor(); region.volume().map_err(map_error)?];
    for &i in hidden {
        *cells
            .get_mut(i)
            .ok_or_else(|| invalid("fixture hidden cell"))? = Cell::Hidden;
    }
    terrain.map = MapRegion { region, cells };
    let mut bytes = b"DFMS1600".to_vec();
    part(
        &mut bytes,
        &operations.encode_profile(OperationsProfile::PagedV1_4)?,
    );
    part(&mut bytes, &terrain.encode_payload()?);
    Ok(bytes)
}
fn context(anchor: dfmcp_core::StateAnchor) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(981543),
        request_id: RequestId::new(1),
        anchor,
        budget: WorkBudget {
            max_entities: 100_000,
            max_bytes: 32768,
            max_output_tokens: 8192,
            max_wall_millis: 60000,
            ..WorkBudget::default()
        },
        cancellation_requested: false,
        grants: vec![CapabilityGrant {
            capability: Capability::Query,
            scope: CapabilityScope::default(),
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        }],
    }
}
fn fixture(
    size: [u32; 3],
    hidden: &[usize],
    tick: u32,
) -> Result<(LiveSpatialState, OperationContext)> {
    let mut state = LiveSpatialState::default();
    state.publish(LiveSpatialObservation::decode_payload(
        &payload(size, hidden, tick)?,
        7,
        "df".into(),
        "dfhack".into(),
    )?)?;
    let c = context(
        state
            .snapshot()
            .ok_or_else(|| invalid("fixture snapshot"))?
            .anchor(),
    );
    Ok((state, c))
}
fn input(goal: [u32; 3], mode: &str) -> Value {
    json!({"schema":"dfmcp.query/1","query":{"kind":"map_route_resilience",
        "start":[0,0,5],"goal":goal,"disjointness":mode}})
}
fn call<S: SpatialStateView + OperationsStateView>(
    s: &S,
    c: &OperationContext,
    input: &Value,
) -> Result<Value> {
    super::super::execute(s, c, input)
}

#[test]
fn compact_default_cut_has_full_checked_certificate_without_route_dump() -> Result<()> {
    let (s, c) = fixture([5, 1, 1], &[], 3)?;
    let before = s.source_digest()?;
    let request = input([4, 0, 5], "internal_tiles");
    let result = call(&s, &c, &request)?;
    assert_eq!(result["section"], "cut");
    assert_eq!(result["summary"]["maximum_disjoint_paths"], 1);
    assert_eq!(result["certificate"]["pure_internal_tile_cut_size"], 1);
    assert_eq!(
        result["rows"],
        json!([{"row_kind":"minimum_cut_tile","position":[1,0,5]}])
    );
    assert_eq!(result["analysis_complete"], true);
    assert_eq!(result["certificate_verified"], true);
    assert_eq!(result["truncated"], false);
    assert_eq!(result["native_captures"], 0);
    for key in [
        "mutation_dispatched",
        "safety_proven",
        "unit_path_proven",
        "global_unreachability_proven",
        "commit_compatible",
        "reservation_created",
    ] {
        assert_eq!(result[key], false, "{key}");
    }
    assert_eq!(call(&s, &c, &request)?, result);
    assert_eq!(s.source_digest()?, before);
    assert_eq!(
        s.snapshot().ok_or_else(|| invalid("snapshot"))?.anchor(),
        c.anchor
    );
    Ok(())
}

#[test]
fn edge_and_tile_modes_distinguish_shared_articulation_failure() -> Result<()> {
    let (s, c) = fixture([3, 3, 1], &[2, 6], 3)?;
    let edges = call(&s, &c, &input([2, 2, 5], "edges"))?;
    let tiles = call(&s, &c, &input([2, 2, 5], "internal_tiles"))?;
    assert_eq!(edges["summary"]["maximum_disjoint_paths"], 2);
    assert_eq!(edges["summary"]["internal_tiles_may_be_shared"], true);
    assert_eq!(tiles["summary"]["maximum_disjoint_paths"], 1);
    assert_eq!(tiles["summary"]["internal_tiles_may_be_shared"], false);
    assert_eq!(tiles["rows"][0]["position"], json!([1, 1, 5]));
    assert_ne!(edges["structural_digest"], tiles["structural_digest"]);
    Ok(())
}

#[test]
fn adjacent_endpoints_expose_mixed_separator_without_claiming_pure_vertex_cut() -> Result<()> {
    let (s, c) = fixture([2, 2, 1], &[], 3)?;
    for mode in ["edges", "internal_tiles"] {
        let result = call(&s, &c, &input([1, 0, 5], mode))?;
        assert_eq!(result["summary"]["maximum_disjoint_paths"], 2);
        assert_eq!(result["certificate"]["contains_direct_endpoint_edge"], true);
        assert_eq!(
            result["certificate"]["mixed_internal_tile_and_direct_edge_cut"],
            mode == "internal_tiles"
        );
        assert!(result["certificate"]["pure_internal_tile_cut_size"].is_null());
        let rows = result["rows"].as_array().ok_or_else(|| invalid("rows"))?;
        assert_eq!(
            rows.iter()
                .filter(|row| row["direct_endpoint_edge"] == true)
                .count(),
            1
        );
        assert_eq!(result["certificate"]["endpoint_deletion_permitted"], false);
    }
    Ok(())
}

#[test]
fn excluded_endpoints_and_disconnected_candidates_have_different_epistemic_results() -> Result<()> {
    for (hidden, excluded) in [(vec![0], true), (vec![2], false)] {
        let (s, c) = fixture([5, 1, 1], &hidden, 3)?;
        let result = call(&s, &c, &input([4, 0, 5], "internal_tiles"))?;
        if excluded {
            assert_eq!(result["status"], "endpoint_excluded");
            assert!(result["summary"]["maximum_disjoint_paths"].is_null());
            assert!(result["certificate"]["packing_size"].is_null());
            assert_eq!(result["endpoint_classification"]["start"], "hidden");
        } else {
            assert_eq!(result["status"], "no_route_in_observed_model");
            assert_eq!(result["summary"]["maximum_disjoint_paths"], 0);
        }
        assert_eq!(result["analysis_complete"], !excluded);
        assert_eq!(result["certificate_verified"], !excluded);
        assert_eq!(result["rows"], json!([]));
        assert_eq!(result["global_unreachability_proven"], false);
    }
    Ok(())
}

#[test]
fn path_pages_reassemble_exact_witness_without_skips_or_partial_analysis() -> Result<()> {
    let (s, c) = fixture([3, 3, 1], &[], 3)?;
    let mut request = input([2, 2, 5], "internal_tiles");
    request["query"]["section"] = json!("all");
    request["query"]["limit"] = json!(128);
    let full = call(&s, &c, &request)?;
    request["query"]["limit"] = json!(1);
    let mut rows = Vec::new();
    for _ in 0..32 {
        let page = call(&s, &c, &request)?;
        assert_eq!(page["analysis_digest"], full["analysis_digest"]);
        assert_eq!(page["structural_digest"], full["structural_digest"]);
        assert_eq!(page["analysis_complete"], true);
        rows.extend(
            page["rows"]
                .as_array()
                .ok_or_else(|| invalid("rows"))?
                .iter()
                .cloned(),
        );
        if page["truncated"] == false {
            assert!(page["continuation"].is_null());
            break;
        }
        request["query"]["continuation"] = page["continuation"].clone();
    }
    assert_eq!(json!(rows), full["rows"]);
    assert_eq!(full["total_rows"], 12);
    Ok(())
}

#[test]
fn continuation_binds_source_session_endpoints_mode_section_and_work_policy() -> Result<()> {
    let (s, c) = fixture([3, 3, 1], &[], 3)?;
    let mut request = input([2, 2, 5], "internal_tiles");
    request["query"]["section"] = json!("all");
    request["query"]["limit"] = json!(1);
    let first = call(&s, &c, &request)?;
    request["query"]["continuation"] = first["continuation"].clone();
    request["query"]["limit"] = json!(3);
    assert_eq!(call(&s, &c, &request)?["returned"], 3);
    for (key, value) in [
        ("start", json!([1, 0, 5])),
        ("goal", json!([1, 2, 5])),
        ("disjointness", json!("edges")),
        ("section", json!("paths")),
        ("max_work", json!(999999)),
    ] {
        let mut changed = request.clone();
        changed["query"][key] = value;
        assert!(
            matches!(call(&s, &c, &changed), Err(e) if e.code == ErrorCode::StaleAnchor),
            "{key}"
        );
    }
    let mut other = c.clone();
    other.session_id = SessionId::new(981544);
    assert!(matches!(call(&s, &other, &request), Err(e) if e.code == ErrorCode::StaleAnchor));
    let (later, later_context) = fixture([3, 3, 1], &[], 4)?;
    assert!(
        matches!(call(&later, &later_context, &request), Err(e) if e.code == ErrorCode::StaleAnchor)
    );
    Ok(())
}

#[test]
fn long_routes_page_positions_instead_of_requiring_one_oversize_path_row() -> Result<()> {
    let (s, mut c) = fixture([128, 1, 1], &[], 3)?;
    c.budget.max_bytes = 8192;
    let mut request = input([127, 0, 5], "internal_tiles");
    let compact = call(&s, &c, &request)?;
    assert_eq!(compact["total_rows"], 1);
    request["query"]["section"] = json!("paths");
    request["query"]["limit"] = json!(1);
    let first = call(&s, &c, &request)?;
    assert_eq!(first["total_rows"], 128);
    assert_eq!(first["truncated"], true);
    assert_eq!(first["structural_digest"], compact["structural_digest"]);
    assert_eq!(
        first["rows"][0],
        json!({"row_kind":"path_position","path":0,"ordinal":0,"position":[0,0,5],"path_positions":128})
    );
    request["query"]["continuation"] = first["continuation"].clone();
    let next = call(&s, &c, &request)?;
    assert_eq!(next["rows"][0]["ordinal"], 1);
    assert_eq!(next["rows"][0]["position"], json!([1, 0, 5]));
    Ok(())
}

#[test]
fn query_authority_is_never_recovered_from_capture_or_continuation() -> Result<()> {
    let (s, c) = fixture([3, 3, 1], &[], 3)?;
    let request = input([2, 2, 5], "internal_tiles");
    for change in 0..5 {
        let mut denied = c.clone();
        match change {
            0 => denied.grants.clear(),
            1 => denied.cancellation_requested = true,
            2 => denied.grants[0].expires_at_tick = Some(GameTick(0)),
            3 => denied.grants[0].remaining_uses = Some(1),
            _ => denied.grants[0].scope.entity_ids = BTreeSet::from([EntityId::new(1)]),
        }
        assert!(
            call(&s, &denied, &request).is_err(),
            "authority case {change}"
        );
    }
    Ok(())
}

#[test]
fn malformed_query_coordinates_modes_and_cursor_shapes_fail_closed() -> Result<()> {
    let (s, c) = fixture([3, 3, 1], &[], 3)?;
    for (key, value) in [
        ("start", json!([-1, 0, 5])),
        ("start", json!([0, 0])),
        ("start", json!([0, 0, 5, 0])),
        ("goal", json!([3, 2, 5])),
        ("goal", json!([0, 0, 5])),
        ("disjointness", json!("shortest")),
        ("disjointness", Value::Null),
        ("section", json!("unsafe")),
        ("limit", json!(0)),
        ("limit", json!(129)),
        ("max_work", json!(0)),
        ("max_work", json!(1000001)),
        ("continuation", json!("sp1:01:abcd")),
        ("continuation", json!("x".repeat(129))),
        ("unregistered", json!(true)),
    ] {
        let mut request = input([2, 2, 5], "internal_tiles");
        request["query"][key] = value;
        assert!(call(&s, &c, &request).is_err(), "{key}");
    }
    Ok(())
}

#[test]
fn complete_expected_anchor_and_context_anchor_are_both_enforced() -> Result<()> {
    let (s, c) = fixture([3, 3, 1], &[], 3)?;
    let mut request = input([2, 2, 5], "internal_tiles");
    request["expected_anchor"] = anchor_json(c.anchor);
    call(&s, &c, &request)?;
    let keys: Vec<_> = request["expected_anchor"]
        .as_object()
        .ok_or_else(|| invalid("anchor"))?
        .keys()
        .cloned()
        .collect();
    assert!(!keys.is_empty());
    for key in keys {
        let mut changed = request.clone();
        changed["expected_anchor"]
            .as_object_mut()
            .ok_or_else(|| invalid("anchor"))?
            .remove(&key);
        assert!(
            matches!(call(&s, &c, &changed), Err(e) if e.code == ErrorCode::StaleAnchor),
            "{key}"
        );
    }
    let mut changed = c.clone();
    changed.anchor.cursor.sequence += 1;
    assert!(matches!(call(&s, &changed, &request), Err(e) if e.code == ErrorCode::StaleAnchor));
    Ok(())
}

#[test]
fn work_wall_scan_and_final_output_budgets_fail_without_altering_source() -> Result<()> {
    let (s, c) = fixture([3, 3, 1], &[], 3)?;
    let request = input([2, 2, 5], "internal_tiles");
    let before = s.source_digest()?;
    for change in 0..5 {
        let mut narrowed = c.clone();
        let mut input = request.clone();
        match change {
            0 => narrowed.budget.max_wall_millis = 0,
            1 => narrowed.budget.max_bytes = 1,
            2 => narrowed.budget.max_output_tokens = 1,
            3 => narrowed.budget.max_entities = 0,
            _ => input["query"]["max_work"] = json!(1),
        }
        let expected = if change == 0 || change == 3 {
            // Core budget validation rejects zero dimensions before the query
            // can classify a nonzero, exhausted work allowance.
            ErrorCode::InvalidRequest
        } else {
            ErrorCode::BudgetExceeded
        };
        assert!(
            matches!(call(&s, &narrowed, &input), Err(e) if e.code == expected),
            "budget case {change}"
        );
    }
    let mut scan_limited = c.clone();
    scan_limited.budget.max_entities = 1;
    assert!(
        matches!(call(&s, &scan_limited, &request), Err(e) if e.code == ErrorCode::BudgetExceeded)
    );
    assert_eq!(s.source_digest()?, before);
    assert_eq!(call(&s, &c, &request)?["certificate_verified"], true);
    Ok(())
}

#[test]
fn registered_query_schema_and_dispatch_selection_include_the_new_kind() -> Result<()> {
    let request = input([1, 0, 5], "edges");
    assert!(super::super::handles(&request));
    let schema = super::super::schema()?;
    let variants = schema["$defs"]["query"]["oneOf"]
        .as_array()
        .ok_or_else(|| invalid("schema variants"))?;
    assert_eq!(
        variants
            .iter()
            .filter(|variant| variant["properties"]["kind"]["const"] == "map_route_resilience")
            .count(),
        1
    );
    let _: super::super::Envelope =
        serde_json::from_value(request).map_err(|_| invalid("request decode"))?;
    Ok(())
}

#[test]
fn citizen_inclusive_capture_uses_the_same_checked_read_path_without_refresh() -> Result<()> {
    let mut bytes = b"DFMS1800".to_vec();
    part(&mut bytes, &payload([3, 3, 1], &[], 3)?);
    let mut citizens = b"DFMC1800".to_vec();
    citizens.extend_from_slice(&0u32.to_be_bytes());
    part(&mut bytes, &citizens);
    let mut state = LiveSpatialCitizenState::default();
    state.publish(LiveSpatialCitizenObservation::decode_payload(
        &bytes,
        7,
        "df".into(),
        "dfhack".into(),
    )?)?;
    let c = context(
        state
            .snapshot()
            .ok_or_else(|| invalid("citizen snapshot"))?
            .anchor(),
    );
    let source = state.source_digest()?;
    let result = call(&state, &c, &input([2, 2, 5], "internal_tiles"))?;
    assert_eq!(result["summary"]["maximum_disjoint_paths"], 2);
    assert_eq!(result["source_digest"], source.to_string());
    assert_eq!(result["anchor"], anchor_json(c.anchor));
    assert_eq!(result["native_captures"], 0);
    assert_eq!(state.source_digest()?, source);
    Ok(())
}
