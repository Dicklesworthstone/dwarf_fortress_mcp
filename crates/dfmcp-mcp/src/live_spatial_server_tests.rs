use super::*;
use dfmcp_adapter::live_map::{LiveMapObservation, tile_entity_id};
use dfmcp_adapter::live_operations::{
    LiveOperationsObservation, OperationsProfile, item_entity_id,
};
use dfmcp_world::map_region::Cell;
use std::collections::VecDeque;
static SERIAL: Mutex<()> = Mutex::new(());
struct Script {
    values: VecDeque<LiveSpatialObservation>,
    calls: Arc<AtomicUsize>,
    fenced: bool,
}
impl SpatialSource for Script {
    fn read(&mut self, _: Duration) -> Result<LiveSpatialObservation> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.values
            .pop_front()
            .ok_or_else(|| error(ErrorCode::AdapterUnavailable, "test source exhausted"))
    }
    fn poisoned(&self) -> bool {
        self.fenced
    }
    fn fence(&mut self) {
        self.fenced = true;
    }
    fn pages(&self) -> u32 {
        1
    }
}
struct Registered {
    id: SessionId,
    calls: Arc<AtomicUsize>,
}
impl Registered {
    fn handle(&self) -> Option<String> {
        Some(self.id.to_string())
    }
}
impl Drop for Registered {
    fn drop(&mut self) {
        if let Ok(mut sessions) = SESSIONS.lock() {
            sessions.remove(&self.id);
        }
    }
}
fn fixture() -> Result<LiveSpatialObservation> {
    let s = include_str!("../../dfmcp-adapter/tests/fixtures/spatial_v1_6.hex").trim();
    let bytes = (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16)
                .map_err(|_| error(ErrorCode::InternalInvariantViolation, "bad fixture"))
        })
        .collect::<Result<Vec<_>>>()?;
    LiveSpatialObservation::decode_payload(&bytes, 7, "df".to_owned(), "dfhack".to_owned())
}
fn compose(
    op: LiveOperationsObservation,
    map: LiveMapObservation,
) -> Result<LiveSpatialObservation> {
    let mut bytes = b"DFMS1600".to_vec();
    for p in [
        op.encode_profile(OperationsProfile::PagedV1_4)?,
        map.encode_payload()?,
    ] {
        bytes.extend_from_slice(&(p.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&p);
    }
    LiveSpatialObservation::decode_payload(&bytes, 7, "df".to_owned(), "dfhack".to_owned())
}
fn register(tokens: u32, caps: Vec<Capability>) -> Result<Registered> {
    let id = next_id()?;
    let slot = Slot::reserve()?;
    let first = fixture()?;
    let mut op = first.operations().clone();
    let mut map = first.terrain().clone();
    op.jobs.year_tick += 1;
    map.year_tick += 1;
    op.items[2].stack_size = 3;
    let index = map
        .map
        .region
        .index([1, 2, 5])
        .ok_or_else(|| error(ErrorCode::InvalidRequest, "fixture coordinate"))?;
    if let Cell::Visible(tile) = &mut map.map.cells[index] {
        tile.liquid_depth = 1;
    }
    let next = compose(op, map)?;
    let region = first.terrain().map.region;
    let mut state = LiveSpatialState::default();
    state.publish(first)?;
    let anchor = state
        .snapshot()
        .ok_or_else(|| error(ErrorCode::InvalidRequest, "fixture snapshot"))?
        .anchor();
    let calls = Arc::new(AtomicUsize::new(0));
    let limits = SpatialLimits {
        operations: PagedOperationsLimits::default(),
        region,
    };
    let session = SpatialSession {
        id,
        source: Box::new(Script {
            values: VecDeque::from([next]),
            calls: Arc::clone(&calls),
            fenced: false,
        }),
        state,
        journal: None,
        limits,
        budget: WorkBudget {
            max_entities: limits.entity_limit(),
            max_bytes: 1024 * 1024,
            max_output_tokens: tokens,
            ..WorkBudget::default()
        },
        grants: caps
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(anchor.fortress_id),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect(),
        request: 0,
        _slot: slot,
    };
    lock(&SESSIONS)?.insert(id, Arc::new(Mutex::new(session)));
    Ok(Registered { id, calls })
}
fn full() -> Result<Registered> {
    register(
        8192,
        vec![Capability::Query, Capability::Observe, Capability::Doctor],
    )
}
fn decode(s: &str) -> Result<Value> {
    serde_json::from_str(s).map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "invalid response JSON",
        )
    })
}
fn ask(s: &Registered, q: Value) -> Result<Value> {
    decode(&fortress_query(
        s.handle(),
        None,
        Some(json!({"schema":"dfmcp.query/1","query":q})),
    ))
}
fn allocation(units: u64, limit: u32) -> Value {
    json!({"kind":"spatial_inventory_plan","origin":[0,0,5],"quantity_unit":"stack_units",
    "demands":[{"key":"wood","units":units,"item_types":["item_type_3"]}],"limit":limit})
}
fn stock(s: &Registered, count: u32) -> Result<()> {
    let f = fixture()?;
    let mut op = f.operations().clone();
    op.jobs.jobs.clear();
    op.attachments.clear();
    let template = op.items[2].clone();
    op.items = (0..count)
        .map(|i| {
            let mut v = template.clone();
            v.native_id = 100 + i;
            v
        })
        .collect();
    op.next_item_id = 100 + count;
    let mut state = LiveSpatialState::default();
    state.publish(compose(op, f.terrain().clone())?)?;
    let handle = resolve(s.handle())?;
    lock(&handle)?.state = state;
    Ok(())
}
#[test]
fn one_handler_exposes_inventory_terrain_and_same_anchor_route_drill() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let s = full()?;
    for mode in ["summary", "items", "jobs", "buildings", "tiles"] {
        let v = decode(&fortress_query(s.handle(), Some(mode.to_owned()), None))?;
        assert_eq!(v["ok"], true, "{v}");
        assert_eq!(v["agent_turn"]["briefing"]["bridge_protocol"], "1.6");
    }
    let p = ask(&s, allocation(1, 8))?;
    assert_eq!(p["ok"], true, "{p}");
    assert_eq!(p["model_feasible"], true);
    assert_eq!(
        p["rows"][0]["item"]["entity_id"],
        item_entity_id(32).to_string()
    );
    assert_eq!(p["rows"][0]["candidate_steps"], 3);
    let route = decode(&fortress_query(
        s.handle(),
        None,
        Some(p["rows"][0]["route_query"].clone()),
    ))?;
    assert_eq!(route["ok"], true);
    assert_eq!(route["model_steps"], 3);
    assert_eq!(route["anchor"], p["anchor"]);
    assert_eq!(route["source_digest"], p["source_digest"]);
    assert_eq!(route["unit_path_proven"], false);
    let schema = decode(&fortress_query(s.handle(), Some("schema".to_owned()), None))?;
    assert_eq!(schema["ok"], true);
    assert_eq!(
        schema["query_schema"]["$defs"]["query"]["oneOf"]
            .as_array()
            .map(Vec::len),
        Some(21)
    );
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    Ok(())
}
#[test]
fn a_terrain_watch_and_inventory_baseline_advance_from_one_capture() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let s = full()?;
    let capture = ask(
        &s,
        json!({"kind":"capture","key":"wood","max_game_ticks":100,
        "select":{"kind":"entities","kinds":["item"],"fields":["stack_size"]}}),
    )?;
    assert_eq!(capture["ok"], true);
    let w = ask(
        &s,
        json!({"kind":"watch","key":"wet","label":"Observe water at supply tile",
        "condition":{"op":"field","entity_id":tile_entity_id([1,2,5])?.to_string(),"generation":1,
            "field":"liquid_depth","comparison":"ge","value":{"type":"u64","value":1}},
        "deadline_tick":105u64*403200+100,"poll_interval_ticks":1,"stable_observations":1}),
    )?;
    assert_eq!(w["ok"], true);
    assert_eq!(w["record"]["terminal"], false);
    let query = json!({"kind":"await_watch","watch":w["record"]["watch"]});
    let done = ask(&s, query.clone())?;
    assert_eq!(done["ok"], true);
    assert_eq!(done["record"]["terminal"], true);
    assert_eq!(done["observation_refresh"]["native_captures"], 1);
    assert_eq!(s.calls.load(Ordering::SeqCst), 1);
    assert_eq!(ask(&s, query)?["record"], done["record"]);
    assert_eq!(s.calls.load(Ordering::SeqCst), 1);
    let changed = ask(
        &s,
        json!({"kind":"changes","baseline":capture["captured"]["baseline"]}),
    )?;
    assert_eq!(changed["ok"], true);
    assert_eq!(changed["change_count"], 1);
    assert_eq!(changed["anchor"], done["anchor"]);
    let p = ask(&s, allocation(1, 8))?;
    assert_eq!(p["model_feasible"], false);
    assert_eq!(p["summary"]["allocated_units"], 0);
    assert_eq!(
        p["summary"]["item_classification"]["no_candidate_route_in_observed_model"],
        1
    );
    assert_eq!(p["global_unreachability_proven"], false);
    assert_eq!(
        ask(
            &s,
            json!({"kind":"release_watch","watch":w["record"]["watch"]})
        )?["ok"],
        true
    );
    assert_eq!(
        ask(
            &s,
            json!({"kind":"release_baseline","baseline":capture["captured"]["baseline"]})
        )?["ok"],
        true
    );
    Ok(())
}
#[test]
fn allocation_pages_keep_current_watch_and_fit_8192_bytes() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let s = register(2048, vec![Capability::Query, Capability::Observe])?;
    stock(&s, 40)?;
    let w = ask(
        &s,
        json!({"kind":"watch","key":"review","label":"Review future observation",
        "condition":{"op":"tick_at_least","value":105u64*403200+50},"deadline_tick":105u64*403200+100,
        "poll_interval_ticks":1,"stable_observations":1}),
    )?;
    assert_eq!(w["ok"], true, "{w}");
    let mut q = allocation(40, 128);
    let mut ids = Vec::new();
    let mut pages = 0;
    loop {
        let raw = fortress_query(
            s.handle(),
            None,
            Some(json!({"schema":"dfmcp.query/1","query":q})),
        );
        assert!(raw.len() <= 8192);
        let v = decode(&raw)?;
        assert_eq!(v["ok"], true, "{raw}");
        assert_eq!(v["summary"]["allocated_units"], 40);
        assert_eq!(
            v["agent_turn"]["active_work"]["obligations"]
                .as_array()
                .map(Vec::len),
            Some(1)
        );
        for row in v["rows"]
            .as_array()
            .ok_or_else(|| error(ErrorCode::InvalidRequest, "missing rows"))?
        {
            ids.push(
                row["item"]["entity_id"]
                    .as_str()
                    .ok_or_else(|| error(ErrorCode::InvalidRequest, "missing item"))?
                    .to_owned(),
            );
        }
        pages += 1;
        assert!(pages <= 40);
        if v["continuation"].is_null() {
            break;
        }
        assert_eq!(
            v["agent_turn"]["coverage"]["continuation"],
            v["continuation"]
        );
        q["continuation"] = v["continuation"].clone();
        q["limit"] = json!(3);
    }
    assert!(pages > 1);
    assert_eq!(
        ids,
        (100..140)
            .map(|i| item_entity_id(i).to_string())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        ask(
            &s,
            json!({"kind":"cancel_watch","watch":w["record"]["watch"]})
        )?["ok"],
        true
    );
    assert_eq!(
        ask(
            &s,
            json!({"kind":"release_watch","watch":w["record"]["watch"]})
        )?["ok"],
        true
    );
    Ok(())
}
#[test]
fn continuation_binds_current_capture_session_and_origin() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let s = full()?;
    let other = full()?;
    stock(&s, 3)?;
    stock(&other, 3)?;
    let first = ask(&s, allocation(3, 1))?;
    assert_eq!(first["ok"], true);
    assert!(first["continuation"].is_string());
    let mut next = allocation(3, 2);
    next["continuation"] = first["continuation"].clone();
    assert_eq!(ask(&s, next.clone())?["returned"], 2);
    assert_eq!(ask(&other, next.clone())?["error"]["code"], "stale_anchor");
    let mut changed = next.clone();
    changed["origin"] = json!([1, 0, 5]);
    assert_eq!(ask(&s, changed)?["error"]["code"], "stale_anchor");
    assert_eq!(decode(&fortress_observe(s.handle()))?["ok"], true);
    assert_eq!(ask(&s, next)?["error"]["code"], "stale_anchor");
    Ok(())
}
#[test]
fn source_failure_preserves_anchor_and_permits_local_cleanup() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let s = full()?;
    let w = ask(
        &s,
        json!({"kind":"watch","key":"future","label":"Future",
        "condition":{"op":"tick_at_least","value":105u64*403200+50},"deadline_tick":105u64*403200+100,
        "poll_interval_ticks":1,"stable_observations":1}),
    )?;
    assert_eq!(w["ok"], true);
    let good = decode(&fortress_observe(s.handle()))?;
    assert_eq!(good["ok"], true);
    let bad = decode(&fortress_observe(s.handle()))?;
    assert_eq!(bad["ok"], false);
    assert_eq!(bad["agent_turn"]["anchor"], good["anchor"]);
    assert_eq!(bad["agent_turn"]["continuity"]["status"], "stale");
    assert_eq!(
        ask(&s, allocation(1, 8))?["error"]["code"],
        "adapter_unavailable"
    );
    assert_eq!(ask(&s, json!({"kind":"watches"}))?["source_stale"], true);
    assert_eq!(
        ask(
            &s,
            json!({"kind":"cancel_watch","watch":w["record"]["watch"]})
        )?["ok"],
        true
    );
    assert_eq!(
        ask(
            &s,
            json!({"kind":"release_watch","watch":w["record"]["watch"]})
        )?["ok"],
        true
    );
    Ok(())
}
#[test]
fn authority_region_and_profile_rejections_do_not_touch_the_bridge() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let s = register(8192, vec![Capability::Doctor])?;
    let denied = ask(&s, allocation(1, 8))?;
    assert_eq!(denied["error"]["code"], "capability_denied");
    assert!(denied.get("anchor").is_none());
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    assert!(resolve(Some(format!("{:032x}", (1u128 << 127) | FAMILY | 1))).is_err());
    assert!(
        resolve(Some(
            SessionId::new((1u128 << 127) | (1u128 << 59) | 1).to_string()
        ))
        .is_err()
    );
    assert!(!allowed_environment("DFMCP_OPERATIONS_JOURNAL"));
    assert!(!allowed_environment("DFMCP_ADMITTED_BRIDGE_PROTOCOL"));
    assert!(!allowed_environment("DFMCP_MAP_TOKEN"));
    assert!(allowed_environment("DFMCP_SPATIAL_TOKEN"));
    assert!(capabilities(Some(vec!["control_clock".to_owned()])).is_err());
    for r in [
        json!({"origin":[0,0,0],"size":[128,128,128]}),
        json!({"origin":[0,0,0],"size":[0,1,1]}),
        json!({"origin":[0,0,0],"size":[1,1,1],"reveal":true}),
    ] {
        assert!(parse_region(&r).is_err());
    }
    let s = full()?;
    for q in [
        json!({"kind":"map_route","start":[0,0,5],"goal":[1,2,5],"max_work":0}),
        json!({"kind":"spatial_inventory_plan","origin":[0,0,5],"quantity_unit":"mass","demands":[]}),
    ] {
        assert_eq!(ask(&s, q)?["ok"], false);
    }
    assert_eq!(
        decode(&fortress_commit(s.handle()))?["error"]["code"],
        "capability_denied"
    );
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[cfg(unix)]
#[path = "spatial_history_tests.rs"]
mod history_cases;

#[test]
fn blueprint_preview_pages_use_one_capture_and_preserve_effect_refusal() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let s = register(2048, vec![Capability::Query, Capability::Observe])?;
    let mut q = json!({"kind":"blueprint_layout","origin":[2,0,5],
        "template":{"kind":"bedroom_cluster","rooms_count":4,"room_size":[3,3]},"limit":1});
    let first = ask(&s, q.clone())?;
    assert_eq!(first["ok"], true, "{first}");
    assert_eq!(first["total_rows"], 10);
    assert_eq!(first["status"], "geometry_preview_only");
    assert!(first["continuation"].is_string());
    let mut indices = Vec::new();
    let mut pages = 0;
    loop {
        let raw = fortress_query(
            s.handle(),
            None,
            Some(json!({"schema":"dfmcp.query/1","query":q.clone()})),
        );
        assert!(raw.len() <= 8192);
        let page = decode(&raw)?;
        assert_eq!(page["ok"], true, "{raw}");
        assert_eq!(page["anchor"], first["anchor"]);
        assert_eq!(page["summary"], first["summary"]);
        for key in [
            "safety_proven",
            "completion_proven",
            "commit_compatible",
            "plan_created",
            "reservation_created",
        ] {
            assert_eq!(page[key], false, "{key}");
        }
        for row in page["rows"]
            .as_array()
            .ok_or_else(|| error(ErrorCode::InvalidRequest, "blueprint rows absent"))?
        {
            indices.push(
                row["part_index"].as_u64().ok_or_else(|| {
                    error(ErrorCode::InvalidRequest, "blueprint part index absent")
                })?,
            );
        }
        pages += 1;
        assert!(pages <= 10);
        if page["continuation"].is_null() {
            break;
        }
        q["continuation"] = page["continuation"].clone();
        q["limit"] = json!(3);
    }
    assert_eq!(indices, (0..10u64).collect::<Vec<_>>());
    assert!(pages > 1);
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        decode(&fortress_commit(s.handle()))?["error"]["code"],
        "capability_denied"
    );
    assert_eq!(decode(&fortress_observe(s.handle()))?["ok"], true);
    q["continuation"] = first["continuation"].clone();
    assert_eq!(ask(&s, q)?["error"]["code"], "stale_anchor");
    assert_eq!(s.calls.load(Ordering::SeqCst), 1);
    Ok(())
}
