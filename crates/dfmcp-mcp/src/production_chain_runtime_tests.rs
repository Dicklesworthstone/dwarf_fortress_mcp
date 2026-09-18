//! Exercise the actual spatial/1.8 query path with an injected coherent source.
use super::super::super::super::*;
use super::{fixture, query};
use std::collections::VecDeque;

static SERIAL: Mutex<()> = Mutex::new(());
struct Script { values: VecDeque<LiveSpatialCitizenObservation>, calls: Arc<AtomicUsize>, fenced: bool }
impl Source for Script {
    fn read(&mut self, _: Duration) -> Result<LiveSpatialCitizenObservation> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.values.pop_front().ok_or_else(|| error(ErrorCode::AdapterUnavailable, "chain fixture exhausted"))
    }
    fn poisoned(&self) -> bool { self.fenced }
    fn fence(&mut self) { self.fenced = true; }
    fn pages(&self) -> u32 { 1 }
}
struct Registered { id: SessionId, calls: Arc<AtomicUsize> }
impl Registered { fn handle(&self) -> Option<String> { Some(self.id.to_string()) } }
impl Drop for Registered {
    fn drop(&mut self) { let _ = fortress_cancel(self.handle(), Some("session".into()), Some(true)); }
}
fn register(tokens: u32) -> Result<Registered> {
    let first = fixture::observation(3, 2, 7, false)?;
    let limits = CitizenSpatialLimits { spatial: SpatialLimits {
        operations: PagedOperationsLimits::default(), region: first.spatial().terrain().map.region }, citizens: 4096 };
    let mut state = LiveSpatialCitizenState::default(); state.publish(first)?;
    let anchor = state.snapshot().ok_or_else(|| error(ErrorCode::InvalidRequest, "chain snapshot absent"))?.anchor();
    let id = next_id()?; let calls = Arc::new(AtomicUsize::new(0));
    let session = Session { id, state, limits, journal: None, _watch_journal: None, request: 0, _slot: Slot::reserve()?,
        source: Box::new(Script { values: VecDeque::from([fixture::observation(4, 2, 8, false)?]), calls: calls.clone(), fenced: false }),
        budget: WorkBudget { max_entities: limits.entity_limit(), max_bytes: 1024 * 1024,
            max_output_tokens: tokens, max_wall_millis: 60_000, ..WorkBudget::default() },
        grants: [Capability::Query, Capability::Observe, Capability::Doctor].into_iter().map(|capability| CapabilityGrant {
            capability, scope: CapabilityScope { fortress_id: Some(anchor.fortress_id), ..CapabilityScope::default() },
            max_risk: RiskTier::ReadOnly, expires_at_tick: None, remaining_uses: None }).collect() };
    lock(&SESSIONS)?.insert(id, Arc::new(Mutex::new(session))); Ok(Registered { id, calls })
}
fn decode(raw: &str) -> Result<Value> {
    serde_json::from_str(raw).map_err(|_| error(ErrorCode::InvalidRequest, "chain response JSON"))
}
fn ask(s: &Registered, query: Value) -> Result<Value> { decode(&fortress_query(s.handle(), None, Some(query))) }
fn envelope(query: Value) -> Value { json!({"schema":"dfmcp.query/1","query":query}) }

#[test]
fn live_chain_query_exposes_dependencies_and_deficits_without_game_reads_or_writes() -> Result<()> {
    let _serial = lock(&SERIAL)?; let s = register(65_536)?;
    let out = ask(&s, query())?; assert_eq!(out["ok"], true, "{out}");
    assert_eq!(out["model_feasible"], false); assert_eq!(out["summary"]["modeled_steps"], 2);
    assert_eq!(out["summary"]["shortage_resources"], 1);
    assert_eq!(out["agent_turn"]["briefing"]["bridge_protocol"], "1.8");
    let schema = decode(&fortress_query(s.handle(), Some("schema".into()), None))?;
    assert_eq!(schema["ok"], true, "{schema}");
    assert!(schema["query_schema"]["$defs"]["query"]["oneOf"].as_array()
        .ok_or_else(|| error(ErrorCode::InvalidRequest, "chain schema"))?.iter()
        .any(|v| v["properties"]["kind"]["const"] == "production_chain"));
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    assert_eq!(decode(&fortress_commit(s.handle()))?["error"]["code"], "capability_denied");
    Ok(())
}

#[test]
fn paginated_chain_keeps_current_watch_evidence_and_complete_summary() -> Result<()> {
    let _serial = lock(&SERIAL)?; let s = register(4096)?;
    let watch = ask(&s, envelope(json!({"kind":"watch","key":"chain-review",
        "condition":{"op":"paused","value":true},"deadline_tick":105u64*403200+100,"stable_observations":2})))?;
    assert_eq!(watch["ok"], true, "{watch}");
    let before = ask(&s, envelope(json!({"kind":"watches"})))?;
    let mut q = query(); q["query"]["limit"] = json!(1);
    let first = ask(&s, q.clone())?; assert_eq!(first["ok"], true, "{first}");
    let mut rows = Vec::new();
    for _ in 0..8 {
        let raw = fortress_query(s.handle(), None, Some(q.clone())); assert!(raw.len() <= 16_384);
        let page = decode(&raw)?; assert_eq!(page["ok"], true, "{raw}");
        assert_eq!(page["summary"], first["summary"]); assert_eq!(page["model_digest"], first["model_digest"]);
        assert_eq!(page["agent_turn"]["active_work"]["obligations"][0]["stable_observations"], 1);
        rows.extend(page["rows"].as_array().ok_or_else(|| error(ErrorCode::InvalidRequest, "chain rows"))?.iter().cloned());
        if page["continuation"].is_null() { break; }
        q["query"]["continuation"] = page["continuation"].clone(); q["query"]["limit"] = json!(2);
    }
    assert_eq!(rows.len(), 8); assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    assert_eq!(ask(&s, envelope(json!({"kind":"watches"})))?["records"], before["records"]);
    assert_eq!(decode(&fortress_observe(s.handle()))?["ok"], true);
    q["query"]["continuation"] = first["continuation"].clone();
    assert_eq!(ask(&s, q)?["error"]["code"], "stale_anchor");
    let current = ask(&s, query())?; assert_eq!(current["ok"], true, "{current}");
    assert_eq!(current["model_feasible"], true); assert_eq!(current["observed_quotas_met"], false);
    Ok(())
}

#[test]
fn fenced_or_unauthorized_sources_cannot_be_used_for_chain_planning() -> Result<()> {
    let _serial = lock(&SERIAL)?; let s = register(4096)?;
    { let handle = resolve(s.handle())?; lock(&handle)?.source.fence(); }
    assert_eq!(ask(&s, query())?["error"]["code"], "adapter_unavailable");
    { let handle = resolve(s.handle())?; lock(&handle)?.grants.clear(); }
    assert_eq!(ask(&s, query())?["error"]["code"], "capability_denied");
    assert_eq!(s.calls.load(Ordering::SeqCst), 0); Ok(())
}

#[path = "production_chain_compare_runtime_tests.rs"]
mod comparisons;
