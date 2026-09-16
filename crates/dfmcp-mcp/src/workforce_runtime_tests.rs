//! Actual spatial/1.8 handler tests; the only observation source is injected.
use super::super::super::*;
use std::collections::{BTreeSet, VecDeque};
use dfmcp_adapter::live_operations::OperationsProfile;
use dfmcp_adapter::live_spatial::LiveSpatialObservation;
use dfmcp_world::map_region::{Cell, MapRegion, Shape, Tile};

static SERIAL: Mutex<()> = Mutex::new(());
struct Script { values: VecDeque<LiveSpatialCitizenObservation>, calls: Arc<AtomicUsize>, fenced: bool }
impl Source for Script {
    fn read(&mut self, _: Duration) -> Result<LiveSpatialCitizenObservation> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.values.pop_front().ok_or_else(|| error(ErrorCode::AdapterUnavailable, "workforce fixture exhausted"))
    }
    fn poisoned(&self) -> bool { self.fenced }
    fn fence(&mut self) { self.fenced = true; }
    fn pages(&self) -> u32 { 1 }
}
fn put(out: &mut Vec<u8>, n: u32) { out.extend_from_slice(&n.to_be_bytes()); }
fn text(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&(text.len() as u16).to_be_bytes()); out.extend_from_slice(text.as_bytes());
}
fn part(out: &mut Vec<u8>, bytes: &[u8]) { put(out, bytes.len() as u32); out.extend_from_slice(bytes); }
fn observation(tick: u32, workers: u32) -> Result<LiveSpatialCitizenObservation> {
    let hex = include_str!("../../dfmcp-adapter/tests/fixtures/spatial_v1_6.hex").trim();
    let bytes = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i+2], 16)
        .map_err(|_| error(ErrorCode::InternalInvariantViolation, "workforce fixture hex"))).collect::<Result<Vec<_>>>()?;
    let original = LiveSpatialObservation::decode_payload(&bytes, 7, "df".into(), "dfhack".into())?;
    let mut operations = original.operations().clone(); let mut terrain = original.terrain().clone();
    operations.jobs.jobs.clear(); operations.attachments.clear(); operations.items.clear(); operations.buildings.clear();
    operations.jobs.year_tick = tick; terrain.year_tick = tick; operations.jobs.paused = true; terrain.paused = true;
    let tile = Tile { native_tiletype: 1, shape: Shape::Floor, liquid_depth: 0, magma: false,
        traffic: 0, dig_designation: 0, building_occupancy: 0, unit_occupancy: 0,
        walkable_region: 1, temperature_1: 10015, temperature_2: 10015 };
    terrain.map = MapRegion { region: Region { origin: [0,0,5], size: [3,3,1] }, cells: vec![Cell::Visible(tile); 9] };
    let mut spatial = b"DFMS1600".to_vec();
    part(&mut spatial, &operations.encode_profile(OperationsProfile::PagedV1_4)?); part(&mut spatial, &terrain.encode_payload()?);
    let mut citizens = b"DFMC1800".to_vec(); put(&mut citizens, workers);
    for i in 0..workers {
        put(&mut citizens, 10+i); text(&mut citizens, "Urist"); text(&mut citizens, "DWARF");
        for n in [0,1,0,5] { put(&mut citizens, n); }
        citizens.extend_from_slice(&0x11fu16.to_be_bytes()); put(&mut citizens, 6); citizens.extend_from_slice(&[1,1]);
        let skills: &[&str] = if i == 0 { &["CARPENTRY", "MINING"] } else { &["CARPENTRY"] };
        citizens.extend_from_slice(&(skills.len() as u16).to_be_bytes());
        for (id, skill) in skills.iter().enumerate() {
            put(&mut citizens, id as u32); text(&mut citizens, skill);
            for n in [5,5,1] { put(&mut citizens, n); }
        }
    }
    let mut all = b"DFMS1800".to_vec(); part(&mut all, &spatial); part(&mut all, &citizens);
    LiveSpatialCitizenObservation::decode_payload(&all, 7, "df".into(), "dfhack".into())
}
struct Registered { id: SessionId, calls: Arc<AtomicUsize> }
impl Registered { fn handle(&self) -> Option<String> { Some(self.id.to_string()) } }
impl Drop for Registered {
    fn drop(&mut self) { if let Ok(mut sessions) = SESSIONS.lock() { sessions.remove(&self.id); } }
}
fn register(workers: u32, tokens: u32) -> Result<Registered> {
    let first = observation(3, workers)?; let region = first.spatial().terrain().map.region;
    let mut state = LiveSpatialCitizenState::default(); state.publish(first)?;
    let anchor = state.snapshot().ok_or_else(|| error(ErrorCode::InternalInvariantViolation, "fixture state"))?.anchor();
    let limits = CitizenSpatialLimits { spatial: SpatialLimits { operations: PagedOperationsLimits::default(), region }, citizens: 4096 };
    let calls = Arc::new(AtomicUsize::new(0)); let id = next_id()?;
    let session = Session { id, source: Box::new(Script { values: VecDeque::from([observation(4, workers)?]), calls: calls.clone(), fenced: false }),
        state, limits, journal: None, request: 0, _watch_journal: None, _slot: Slot::reserve()?,
        budget: WorkBudget { max_entities: limits.entity_limit(), max_bytes: 1024*1024, max_output_tokens: tokens,
            max_wall_millis: 60_000, ..WorkBudget::default() },
        grants: [Capability::Observe, Capability::Query, Capability::Doctor].into_iter().map(|capability| CapabilityGrant {
            capability, scope: CapabilityScope { fortress_id: Some(anchor.fortress_id), ..CapabilityScope::default() },
            max_risk: RiskTier::ReadOnly, expires_at_tick: None, remaining_uses: None }).collect() };
    lock(&SESSIONS)?.insert(id, Arc::new(Mutex::new(session))); Ok(Registered { id, calls })
}
fn decode(text: &str) -> Result<Value> { serde_json::from_str(text).map_err(|_| error(ErrorCode::InternalInvariantViolation, "handler JSON")) }
fn ask(session: &Registered, query: Value) -> Result<Value> {
    decode(&fortress_query(session.handle(), None, Some(json!({"schema":"dfmcp.query/1","query":query}))))
}
fn demand(key: &str, skill: &str, workers: u32) -> Value {
    json!({"key":key,"workers":workers,"target":[0,0,5],"skill_key":skill,"min_effective_skill":1})
}
fn planned(workers: u32) -> Value { json!({"kind":"workforce_plan","demands":[demand("wood", "CARPENTRY", workers)],"limit":128}) }

#[test]
fn registered_queries_discover_skills_plan_without_conflicts_and_drill_into_routes() -> Result<()> {
    let _serial = lock(&SERIAL)?; let s = register(2, 65_536)?;
    let schema = decode(&fortress_query(s.handle(), Some("schema".into()), None))?;
    assert_eq!(schema["ok"], true, "{schema}");
    let variants = schema["query_schema"]["$defs"]["query"]["oneOf"].as_array()
        .ok_or_else(|| error(ErrorCode::InternalInvariantViolation, "query variants"))?;
    for name in ["workforce_candidates", "workforce_plan"] {
        assert!(variants.iter().any(|v| v["properties"]["kind"]["const"] == name));
    }
    let candidates = ask(&s, json!({"kind":"workforce_candidates","target":[0,0,5],"skill_key":"MINING","min_effective_skill":1}))?;
    assert_eq!(candidates["ok"], true, "{candidates}"); assert_eq!(candidates["total_rows"], 1);
    let query = json!({"kind":"workforce_plan","demands":[demand("a", "CARPENTRY", 1), demand("b", "MINING", 1)]});
    let result = ask(&s, query)?;
    assert_eq!(result["ok"], true, "{result}"); assert_eq!(result["assigned_workers"], 2);
    assert_eq!(result["model_feasible"], true); assert_eq!(result["cut_capacity"], 2);
    assert_ne!(result["rows"][0]["citizen"]["entity_id"], result["rows"][1]["citizen"]["entity_id"]);
    let route = decode(&fortress_query(s.handle(), None, Some(result["rows"][0]["route_query"].clone())))?;
    assert_eq!(route["ok"], true, "{route}"); assert_eq!(route["anchor"], result["anchor"]);
    assert_eq!(route["unit_path_proven"], false); assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    assert_eq!(decode(&fortress_plan(s.handle()))?["error"]["code"], "capability_denied");
    assert_eq!(decode(&fortress_commit(s.handle()))?["error"]["code"], "capability_denied");
    Ok(())
}

#[test]
fn workforce_pages_fit_8192_bytes_keep_watches_and_reject_stale_capture_tokens() -> Result<()> {
    let _serial = lock(&SERIAL)?; let s = register(40, 2048)?;
    let w = ask(&s, json!({"kind":"watch","key":"workforce-review","condition":{"op":"paused","value":true},
        "deadline_tick":105u64*403200+100,"poll_interval_ticks":1,"stable_observations":2}))?;
    assert_eq!(w["ok"], true, "{w}");
    let mut query = planned(40); let mut ids = BTreeSet::new(); let mut first_token = Value::Null; let mut pages = 0;
    loop {
        let raw = fortress_query(s.handle(), None, Some(json!({"schema":"dfmcp.query/1","query":query.clone()})));
        assert!(raw.len() <= 8192); let page = decode(&raw)?; assert_eq!(page["ok"], true, "{page}");
        assert_eq!(page["assigned_workers"], 40); assert_eq!(page["model_feasible"], true);
        assert_eq!(page["agent_turn"]["active_work"]["obligations"].as_array().map(Vec::len), Some(1));
        assert_eq!(page["agent_turn"]["active_work"]["obligations"][0]["stable_observations"], 1);
        for row in page["rows"].as_array().ok_or_else(|| error(ErrorCode::InternalInvariantViolation, "workforce rows"))? {
            assert!(ids.insert(row["citizen"]["entity_id"].as_str().ok_or_else(|| error(ErrorCode::InternalInvariantViolation, "worker ID"))?.to_owned()));
        }
        pages += 1; assert!(pages <= 40);
        if first_token.is_null() { first_token = page["continuation"].clone(); }
        if page["continuation"].is_null() { break; }
        query["continuation"] = page["continuation"].clone(); query["limit"] = json!(3);
    }
    assert!(pages > 1); assert_eq!(ids.len(), 40); assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    assert_eq!(decode(&fortress_observe(s.handle()))?["ok"], true);
    query["continuation"] = first_token;
    assert_eq!(ask(&s, query)?["error"]["code"], "stale_anchor"); assert_eq!(s.calls.load(Ordering::SeqCst), 1);
    assert_eq!(ask(&s, json!({"kind":"cancel_watch","watch":w["record"]["watch"]}))?["ok"], true);
    assert_eq!(ask(&s, json!({"kind":"release_watch","watch":w["record"]["watch"]}))?["ok"], true);
    Ok(())
}

#[test]
fn workforce_query_preserves_authority_fencing_and_invalid_input_refusal() -> Result<()> {
    let _serial = lock(&SERIAL)?; let s = register(2, 8192)?;
    let mut invalid = planned(1); invalid["dispatch"] = json!(true);
    assert_eq!(ask(&s, invalid)?["ok"], false);
    let mut too_much = planned(129); too_much["max_work"] = json!(1);
    assert_eq!(ask(&s, too_much)?["ok"], false);
    { let handle = resolve(s.handle())?; lock(&handle)?.source.fence(); }
    assert_eq!(ask(&s, planned(1))?["error"]["code"], "adapter_unavailable");
    { let handle = resolve(s.handle())?; lock(&handle)?.grants.retain(|g| g.capability != Capability::Query); }
    let denied = ask(&s, planned(1))?;
    assert_eq!(denied["error"]["code"], "capability_denied");
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    Ok(())
}
