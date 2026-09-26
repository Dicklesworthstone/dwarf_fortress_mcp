use super::*;
use dfmcp_adapter::live_jobs::LiveJobObservation;
use dfmcp_adapter::live_operations::{LiveItem, item_entity_id};
use dfmcp_core::MapCoord;
use std::collections::VecDeque;

struct Script {
    pending: VecDeque<LiveOperationsObservation>,
    calls: Arc<AtomicUsize>,
    fenced: bool,
    _slot: PagedSlot,
}
impl OperationsSource for Script {
    fn read(&mut self, _: Duration) -> Result<LiveOperationsObservation> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.pending.pop_front().ok_or_else(|| {
            error(
                ErrorCode::AdapterUnavailable,
                "scripted acquisition failure",
            )
        })
    }
    fn poisoned(&self) -> bool {
        self.fenced
    }
    fn fence(&mut self) {
        self.fenced = true;
    }
}
struct Registered {
    id: SessionId,
    calls: Arc<AtomicUsize>,
}
impl Drop for Registered {
    fn drop(&mut self) {
        if let Ok(mut registry) = SESSIONS.lock() {
            registry.remove(&self.id);
        }
    }
}
impl Registered {
    fn query(&self, query: Value) -> Result<Value> {
        parse(&super::super::fortress_query(
            Some(self.id.to_string()),
            None,
            Some(json!({"schema":"dfmcp.query/1","query":query})),
        ))
    }
}
fn parse(text: &str) -> Result<Value> {
    serde_json::from_str(text)
        .map_err(|_| error(ErrorCode::InternalInvariantViolation, "test JSON"))
}
fn register() -> Result<Registered> {
    let id = next_paged_id()?;
    let source = LiveOperationsObservation {
        jobs: LiveJobObservation {
            bridge_generation: 7,
            df_version: "df".to_owned(),
            dfhack_version: "dfhack".to_owned(),
            year: 105,
            year_tick: 3,
            paused: true,
            site_id: 1,
            world_folder: "region1".to_owned(),
            next_job_id: 0,
            jobs: Vec::new(),
        },
        next_building_id: 0,
        next_item_id: 40000,
        buildings: Vec::new(),
        attachments: Vec::new(),
        items: (0..40000)
            .map(|id| LiveItem {
                native_id: id,
                item_type: 1,
                type_key: "BAR".to_owned(),
                subtype: -1,
                material_type: 0,
                material_index: 1,
                stack_size: 1,
                raw_position: MapCoord::new(0, 0, 0),
                flags: 64,
                container_native_id: None,
                holder_building_native_id: None,
            })
            .collect(),
    };
    let mut next = source.clone();
    next.jobs.year_tick += 1;
    next.items[0].flags |= 1;
    let mut state = LiveOperationsState::with_profile(OperationsProfile::PagedV1_4);
    state.publish(source)?;
    let fortress = state
        .snapshot()
        .ok_or_else(|| error(ErrorCode::InternalInvariantViolation, "test snapshot"))?
        .fortress_id;
    let calls = Arc::new(AtomicUsize::new(0));
    let limits = OperationsLimits {
        jobs: 4096,
        buildings: 4096,
        items: 65536,
        payload_bytes: 16 * 1024 * 1024,
    };
    let session = OperationsSession {
        id,
        source: Box::new(Script {
            pending: VecDeque::from([next]),
            calls: Arc::clone(&calls),
            fenced: false,
            _slot: PagedSlot::reserve()?,
        }),
        state,
        journal: None,
        limits,
        budget: WorkBudget {
            max_entities: limits.entity_limit(),
            max_bytes: limits.payload_bytes as u64,
            max_output_tokens: 2048,
            ..WorkBudget::default()
        },
        grants: [Capability::Observe, Capability::Query, Capability::Doctor]
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(fortress),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect(),
        request: 0,
        _slot: SessionSlot::reserve()?,
    };
    lock(&SESSIONS)?.insert(id, Arc::new(Mutex::new(session)));
    Ok(Registered { id, calls })
}

// One session-heavy test avoids increasing concurrent production capacity.
#[test]
fn large_paged_projection_uses_existing_queries_baselines_and_watches() -> Result<()> {
    let session = register()?;
    let request = json!({"schema":"dfmcp.query/1","query":{"kind":"entities","kinds":["item"],
        "fields":["native_item_id","forbidden"],"limit":1}});
    let encoded = super::super::fortress_query(Some(session.id.to_string()), None, Some(request));
    assert!(encoded.len() <= 8192);
    let result = parse(&encoded)?;
    assert_eq!(result["ok"], true);
    assert_eq!(result["matched"], 40000);
    assert_eq!(result["agent_turn"]["briefing"]["bridge_protocol"], "1.4");
    assert_eq!(result["agent_turn"]["briefing"]["runtime_admitted"], false);
    assert_eq!(session.calls.load(Ordering::SeqCst), 0);
    let created = session.query(json!({"kind":"watch","key":"flag-change","label":"Observe forbidden flag",
        "condition":{"op":"field","entity_id":item_entity_id(0).to_string(),"generation":1,"field":"forbidden",
            "comparison":"eq","value":{"type":"bool","value":true}},
        "deadline_tick":105u64*403200+100,"poll_interval_ticks":1,"stable_observations":1}))?;
    assert_eq!(created["ok"], true);
    assert_eq!(created["record"]["terminal"], false);
    let captured = session.query(json!({"kind":"capture","key":"one-item","max_game_ticks":100,
        "select":{"kind":"entities","kinds":["item"],"fields":["forbidden"],
            "where":{"op":"compare","field":"native_item_id","comparison":"eq","value":{"type":"u64","value":0}}}}))?;
    assert_eq!(captured["ok"], true);
    let await_input = json!({"kind":"await_watch","watch":created["record"]["watch"]});
    let done = session.query(await_input.clone())?;
    assert_eq!(done["ok"], true);
    assert_eq!(done["record"]["terminal"], true);
    assert_eq!(done["agent_turn"]["briefing"]["bridge_protocol"], "1.4");
    assert_eq!(session.calls.load(Ordering::SeqCst), 1);
    assert_eq!(session.query(await_input)?["record"], done["record"]);
    assert_eq!(session.calls.load(Ordering::SeqCst), 1);
    let changed =
        session.query(json!({"kind":"changes","baseline":captured["captured"]["baseline"]}))?;
    assert_eq!(changed["ok"], true);
    assert_eq!(changed["change_count"], 1);
    assert_eq!(
        changed["changes"][0]["entity_id"],
        item_entity_id(0).to_string()
    );
    let diagnosis = session.query(json!({"kind":"production_diagnosis"}))?;
    assert_eq!(diagnosis["ok"], true);
    assert_eq!(diagnosis["summary"]["jobs_considered"], 0);
    let digest = lock(&*resolve(Some(session.id.to_string()))?)?
        .state
        .source_digest()?
        .to_string();
    assert_eq!(diagnosis["source_digest"], digest);
    // Discovery is checked with an adequate output budget, not by weakening it.
    lock(&*resolve(Some(session.id.to_string()))?)?
        .budget
        .max_output_tokens = 16384;
    let schema = parse(&super::super::fortress_query(
        Some(session.id.to_string()),
        Some("schema".to_owned()),
        None,
    ))?;
    assert_eq!(schema["ok"], true);
    assert_eq!(schema["profile"], "operations/1.4");
    assert_eq!(
        schema["query_schema"]["$defs"]["query"]["oneOf"]
            .as_array()
            .map(Vec::len),
        Some(18)
    );
    assert_eq!(session.query(json!({"kind":"history"}))?["ok"], false);
    assert_eq!(
        parse(&super::super::fortress_commit(Some(session.id.to_string())))?["error"]["code"],
        "capability_denied"
    );
    assert_eq!(
        session.query(json!({"kind":"release_watch","watch":created["record"]["watch"]}))?["ok"],
        true
    );
    assert_eq!(
        session.query(
            json!({"kind":"release_baseline","baseline":captured["captured"]["baseline"]})
        )?["ok"],
        true
    );
    let anchor = lock(&*resolve(Some(session.id.to_string()))?)?.anchor()?;
    let failure = parse(&super::super::fortress_observe(Some(
        session.id.to_string(),
    )))?;
    assert_eq!(failure["ok"], false);
    assert_eq!(failure["agent_turn"]["anchor"], anchor_json(anchor));
    assert_eq!(failure["agent_turn"]["continuity"]["status"], "stale");
    assert_eq!(session.query(json!({"kind":"entities"}))?["ok"], false);
    assert_eq!(session.calls.load(Ordering::SeqCst), 2);
    Ok(())
}

#[test]
fn paged_environment_and_handle_family_cannot_admit_old_authority() {
    for name in [
        "DFMCP_OPERATIONS_TOKEN",
        "DFMCP_OPERATIONS_JOURNAL",
        "DFMCP_OPERATIONS_JOURNAL_REPAIR",
        "DFMCP_ADMITTED_BRIDGE_PROTOCOL",
        "DFMCP_ADMISSION_TICKET",
        "DFMCP_ALLOW_UNADMITTED_OPERATIONS_V1_3",
    ] {
        assert!(!allowed_paged_environment(name));
    }
    assert!(allowed_paged_environment("DFMCP_OPERATIONS_PAGED_TOKEN"));
    assert!(allowed_paged_environment(
        "DFMCP_ALLOW_UNADMITTED_OPERATIONS_V1_4"
    ));
    assert!(resolve(Some(format!("{:032x}", (1u128 << 127) | PAGED_FAMILY | 1))).is_err());
}
