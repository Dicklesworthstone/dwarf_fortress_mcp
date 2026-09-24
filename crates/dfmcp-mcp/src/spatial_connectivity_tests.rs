use super::*;
use dfmcp_adapter::live_operations::OperationsProfile;
use dfmcp_adapter::live_spatial::{LiveSpatialObservation, LiveSpatialState};
use dfmcp_core::{CapabilityGrant, CapabilityScope, RequestId, SessionId, WorkBudget};
use dfmcp_world::map_region::{Cell, Region, Shape, Tile};

fn cells() -> Vec<Cell> {
    vec![
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
            temperature_2: 10015
        });
        5
    ]
}
fn fixture(cells: Vec<Cell>, tick: u32) -> Result<(LiveSpatialState, OperationContext)> {
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
    terrain.map = MapRegion {
        region: Region {
            origin: [0, 0, 5],
            size: [5, 1, 1],
        },
        cells,
    };
    let mut bytes = b"DFMS1600".to_vec();
    for part in [
        operations.encode_profile(OperationsProfile::PagedV1_4)?,
        terrain.encode_payload()?,
    ] {
        bytes.extend_from_slice(&(part.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&part);
    }
    let mut state = LiveSpatialState::default();
    state.publish(LiveSpatialObservation::decode_payload(
        &bytes,
        7,
        "df".into(),
        "dfhack".into(),
    )?)?;
    let anchor = state
        .snapshot()
        .ok_or_else(|| invalid("fixture snapshot"))?
        .anchor();
    let context = OperationContext {
        session_id: SessionId::new(987654),
        request_id: RequestId::new(1),
        anchor,
        budget: WorkBudget {
            max_entities: 100_000,
            max_bytes: 8192,
            max_output_tokens: 2048,
            max_wall_millis: 60_000,
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
    };
    Ok((state, context))
}
fn request(section: &str, landmarks: Value, limit: u32) -> Value {
    json!({"schema":"dfmcp.query/1","query":{"kind":"map_connectivity",
        "section":section,"landmarks":landmarks,"limit":limit}})
}
fn call(state: &LiveSpatialState, context: &OperationContext, input: &Value) -> Result<Value> {
    super::super::execute(state, context, input)
}

#[test]
fn live_query_ranks_highest_impact_bottlenecks_and_reports_full_counts() -> Result<()> {
    let (s, c) = fixture(cells(), 3)?;
    let out = call(&s, &c, &request("bottlenecks", json!([]), 128))?;
    assert_eq!(out["summary"]["components"], 1);
    assert_eq!(out["summary"]["candidate_tiles"], 5);
    assert_eq!(out["summary"]["graph_bridges"], 4);
    assert_eq!(out["summary"]["bottlenecks"], 3);
    assert_eq!(out["rows"][0]["position"], json!([2, 0, 5]));
    assert_eq!(out["rows"][0]["remaining_partition_sizes"], json!([2, 2]));
    assert_eq!(out["rows"][0]["separated_tile_pairs"], 4);
    assert_eq!(out["rows"][1]["position"], json!([1, 0, 5]));
    assert_eq!(out["analysis_complete"], true);
    assert_eq!(out["native_captures"], 0);
    assert_eq!(out["safety_proven"], false);
    assert_eq!(out["commit_compatible"], false);
    assert!(out["landmark_summary"]["all_connected_in_observed_model"].is_null());
    Ok(())
}

#[test]
fn named_landmarks_distinguish_disconnection_from_unknown_positions() -> Result<()> {
    let points = json!([{"key":"west","position":[0,0,5]},{"key":"east","position":[4,0,5]}]);
    let (s, c) = fixture(cells(), 3)?;
    let input = request("landmarks", points.clone(), 128);
    let out = call(&s, &c, &input)?;
    assert_eq!(
        out["landmark_summary"]["all_connected_in_observed_model"],
        true
    );
    assert_eq!(out["rows"][0]["key"], "east");
    let mut split = cells();
    split[2] = Cell::Hidden;
    let (s, c) = fixture(split, 3)?;
    let out = call(&s, &c, &input)?;
    assert_eq!(
        out["landmark_summary"]["all_connected_in_observed_model"],
        false
    );
    assert_eq!(out["global_unreachability_proven"], false);
    let mut input = request("landmarks", points, 128);
    input["query"]["landmarks"]
        .as_array_mut()
        .ok_or_else(|| invalid("points"))?
        .extend([
            json!({"key":"hidden","position":[2,0,5]}),
            json!({"key":"outside","position":[0,1,5]}),
        ]);
    let out = call(&s, &c, &input)?;
    assert!(out["landmark_summary"]["all_connected_in_observed_model"].is_null());
    assert_eq!(out["landmark_summary"]["unestablished"], 2);
    assert_eq!(out["rows"][1]["reason"], "hidden");
    assert_eq!(out["rows"][2]["reason"], "outside_observed_region");
    assert!(out["rows"][1]["component_representative"].is_null());
    Ok(())
}

#[test]
fn normalized_inputs_page_without_skips_and_bind_the_capture_and_section() -> Result<()> {
    let (s, c) = fixture(cells(), 3)?;
    let landmarks = json!([{"key":"z","position":[0,0,5]},{"key":"a","position":[4,0,5]}]);
    let mut input = request("all", landmarks, 1);
    let first = call(&s, &c, &input)?;
    let mut reordered = input.clone();
    reordered["query"]["landmarks"]
        .as_array_mut()
        .ok_or_else(|| invalid("points"))?
        .reverse();
    assert_eq!(call(&s, &c, &reordered)?, first);
    let mut rows = Vec::new();
    let mut page = first.clone();
    for _ in 0..16 {
        rows.extend(
            page["rows"]
                .as_array()
                .ok_or_else(|| invalid("rows"))?
                .iter()
                .cloned(),
        );
        if page["truncated"] == false {
            break;
        }
        input["query"]["continuation"] = page["continuation"].clone();
        page = call(&s, &c, &input)?;
        assert_eq!(page["structural_digest"], first["structural_digest"]);
        assert_eq!(page["analysis_digest"], first["analysis_digest"]);
    }
    assert_eq!(page["truncated"], false);
    assert_eq!(rows.len(), 10);
    input["query"]["limit"] = json!(128);
    input["query"]["continuation"] = Value::Null;
    assert_eq!(call(&s, &c, &input)?["rows"], json!(rows));
    input["query"]["continuation"] = first["continuation"].clone();
    input["query"]["section"] = json!("components");
    assert!(matches!(call(&s,&c,&input),Err(e)if e.code==ErrorCode::StaleAnchor));
    input["query"]["section"] = json!("all");
    let (next, nc) = fixture(cells(), 4)?;
    assert!(matches!(call(&next,&nc,&input),Err(e)if e.code==ErrorCode::StaleAnchor));
    Ok(())
}

#[test]
fn work_output_authority_and_expected_anchor_failures_do_not_mutate_state() -> Result<()> {
    let (s, c) = fixture(cells(), 3)?;
    let before = s.snapshot().cloned();
    let input = request("all", json!([]), 8);
    for case in 0..6 {
        let mut c = c.clone();
        let mut input = input.clone();
        match case {
            0 => c.grants.clear(),
            1 => c.cancellation_requested = true,
            2 => c.budget.max_wall_millis = 0,
            3 => c.budget.max_bytes = 64,
            4 => input["query"]["max_work"] = json!(1),
            _ => input["expected_anchor"] = json!({}),
        }
        assert!(call(&s, &c, &input).is_err());
        assert_eq!(s.snapshot(), before.as_ref());
    }
    Ok(())
}

#[test]
fn empty_candidate_graph_is_not_a_global_absence_or_safety_claim() -> Result<()> {
    let (s, c) = fixture(vec![Cell::Hidden; 5], 3)?;
    let out = call(&s, &c, &request("all", json!([]), 8))?;
    assert_eq!(out["summary"]["candidate_tiles"], 0);
    assert_eq!(out["summary"]["tile_classification"]["hidden"], 5);
    assert_eq!(out["rows"], json!([]));
    assert_eq!(out["analysis_complete"], true);
    assert_eq!(out["global_unreachability_proven"], false);
    assert_eq!(out["coverage"]["evacuation_safety_proven"], false);
    Ok(())
}

#[test]
fn schema_and_runtime_expose_a_closed_bounded_query() -> Result<()> {
    let (s, c) = fixture(cells(), 3)?;
    let schema = super::super::schema()?;
    let variants = schema["$defs"]["query"]["oneOf"]
        .as_array()
        .ok_or_else(|| invalid("variants"))?;
    assert_eq!(
        variants
            .iter()
            .filter(|v| v["properties"]["kind"]["const"] == "map_connectivity")
            .count(),
        1
    );
    let valid = request("all", json!([]), 8);
    for (field, value) in [
        ("unknown", json!(true)),
        ("section", json!("unsafe")),
        ("limit", json!(0)),
        ("limit", json!(129)),
        ("max_work", json!(1000001)),
        ("max_work", json!(0)),
        ("landmarks", Value::Null),
        (
            "landmarks",
            json!([{"key":"a","position":[0,0,5],"hidden":true}]),
        ),
        ("landmarks", json!([{"key":"a\n","position":[0,0,5]}])),
        ("landmarks", json!([{"key":"a","position":[32768,0,5]}])),
        (
            "landmarks",
            json!([{"key":"a","position":[0,0,5]},{"key":"a","position":[1,0,5]}]),
        ),
    ] {
        let mut input = valid.clone();
        input["query"][field] = value;
        assert!(call(&s, &c, &input).is_err(), "field={field}");
    }
    Ok(())
}
