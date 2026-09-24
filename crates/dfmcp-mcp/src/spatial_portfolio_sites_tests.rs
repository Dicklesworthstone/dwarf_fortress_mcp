//! Actual query dispatcher, real private journals and coherent injected captures.
//! No native game or transport qualification is implied by these fixtures.
use super::*;
#[path = "../../dfmcp-adapter/tests/support/production_site_spatial.rs"]
mod site_fixture;

const WEST: [u32; 3] = [0, 0, 5];
const EAST: [u32; 3] = [4, 0, 5];
fn register_sites(
    files: &Files,
    first: LiveSpatialCitizenObservation,
    next: Vec<LiveSpatialCitizenObservation>,
) -> Result<Registered> {
    let limits = CitizenSpatialLimits {
        spatial: SpatialLimits {
            operations: PagedOperationsLimits::default(),
            region: first.spatial().terrain().map.region,
        },
        citizens: 4096,
    };
    let mut state = LiveSpatialCitizenState::default();
    state.publish(first)?;
    let anchor = state
        .snapshot()
        .ok_or_else(|| error(ErrorCode::InvalidRequest, "site fixture state"))?
        .anchor();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut session = Session {
        id: next_id()?,
        source: Box::new(Script {
            values: next.into(),
            calls: calls.clone(),
            fenced: false,
        }),
        state,
        limits,
        journal: None,
        budget: WorkBudget {
            max_entities: limits.entity_limit(),
            max_bytes: 1024 * 1024,
            max_output_tokens: 65536,
            max_wall_millis: 60000,
            ..WorkBudget::default()
        },
        grants: [Capability::Query, Capability::Observe, Capability::Doctor]
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
        _watch_journal: None,
        _slot: Slot::reserve()?,
    };
    let mut c = session.context()?;
    history::attach(&mut session, &files.observations, TailRecovery::Refuse, &c)?;
    c.anchor = session.anchor()?;
    durable_watches::finish_open(&mut session, &c, Some(&files.watches), json!({"ok":true}))?;
    let id = session.id;
    lock(&SESSIONS)?.insert(id, Arc::new(Mutex::new(session)));
    Ok(Registered { id, calls })
}
fn request() -> Value {
    let mut east = task("b", 2, 1, 1);
    east["origin"] = json!(EAST);
    json!({"kind":"production_portfolio","origin":WEST,"quantity_unit":"stack_units",
        "tasks":[task("a",1,1,1),east],"limit":128})
}
fn reserve(q: &mut Value, units: u64) {
    q["reserves"] = json!([{"key":"buffer","units":units,"item_types":["item_type_3"]}]);
}

#[test]
fn site_assignments_and_drilldowns_use_their_own_capture_bound_origins() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register_sites(&files, site_fixture::observation(3, 1, 1, true)?, vec![])?;
    retain_watch(&s)?;
    let watches = fs::read(&files.watches).map_err(io_error)?;
    let archive = fs::read(&files.observations).map_err(io_error)?;
    let result = ok(ask(&s, request())?);
    assert_eq!(result["selected_task_mask"], 3);
    assert_eq!(result["supply_model"]["candidate_stacks"], 2);
    assert_eq!(result["supply_model"]["shared_capacity_across_sites"], true);
    assert_eq!(
        result["agent_turn"]["active_work"]["obligations"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    for row in result["rows"].as_array().expect("rows") {
        let expected = if row["task_key"] == "a" {
            json!(WEST)
        } else {
            json!(EAST)
        };
        assert_eq!(row["origin"], expected);
        assert_eq!(row["route_query"]["query"]["start"], expected);
        let route = ok(decode(&fortress_query(
            s.handle(),
            None,
            Some(row["route_query"].clone()),
        ))?);
        assert_eq!(route["anchor"], result["anchor"]);
        assert_eq!(route["status"], "candidate_found");
    }
    assert_eq!(fs::read(&files.watches).map_err(io_error)?, watches);
    assert_eq!(fs::read(&files.observations).map_err(io_error)?, archive);
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn pages_bind_task_locations_and_never_duplicate_assignments() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register_sites(&files, site_fixture::observation(3, 1, 1, true)?, vec![])?;
    retain_watch(&s)?;
    {
        let h = resolve(s.handle())?;
        lock(&h)?.budget.max_output_tokens = 4096;
    }
    let mut q = request();
    q["limit"] = json!(1);
    let mut ids = BTreeSet::new();
    let mut first_token = Value::Null;
    for _ in 0..4 {
        let raw = fortress_query(
            s.handle(),
            None,
            Some(json!({"schema":"dfmcp.query/1","query":q.clone()})),
        );
        assert!(raw.len() <= 16384);
        let page = ok(decode(&raw)?);
        assert_eq!(page["selected_task_mask"], 3);
        let row = &page["rows"][0];
        assert!(ids.insert((row["row_kind"].to_string(), row["task_key"].to_string())));
        if first_token.is_null() {
            first_token = page["continuation"].clone();
        }
        if page["continuation"].is_null() {
            break;
        }
        q["continuation"] = page["continuation"].clone();
    }
    assert_eq!(ids.len(), 4);
    assert!(first_token.is_string());
    let mut changed = request();
    changed["continuation"] = first_token;
    changed["tasks"][1]["origin"] = json!([3, 0, 5]);
    assert_eq!(ask(&s, changed)?["error"]["code"], "stale_anchor");
    Ok(())
}

#[test]
fn site_reserves_cannot_be_supported_by_disconnected_remote_stock() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register_sites(&files, site_fixture::observation(3, 1, 10, true)?, vec![])?;
    let mut q = request();
    reserve(&mut q, 1);
    let result = ok(ask(&s, q.clone())?);
    assert_eq!(result["selected_task_mask"], 2);
    assert_eq!(result["assigned_stack_units"], 1);
    let protected = result["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|r| r["row_kind"] == "reserve_assignment")
        .expect("reserve");
    assert_eq!(protected["origin"], json!(WEST));
    assert_eq!(protected["route_query"]["query"]["start"], json!(WEST));
    reserve(&mut q, 2);
    let failure = ok(ask(&s, q)?);
    assert_eq!(
        failure["optimization"]["status"],
        "infeasible_hard_reserves"
    );
    assert_eq!(failure["rows"][0]["deficit"], 1);
    assert_eq!(failure["selected_set_model_feasible"], false);
    Ok(())
}

#[test]
fn historical_site_routes_stay_on_the_selected_record_and_leave_watches_unchanged() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register_sites(
        &files,
        site_fixture::observation(3, 1, 1, true)?,
        vec![site_fixture::observation(4, 1, 0, true)?],
    )?;
    retain_watch(&s)?;
    let entry = ok(ask(&s, json!({"kind":"history","limit":1}))?)["rows"][0].clone();
    ok(decode(&fortress_observe(s.handle()))?);
    let current = ok(ask(&s, request())?);
    assert_eq!(current["selected_task_mask"], 1);
    let before = fs::read(&files.watches).map_err(io_error)?;
    let old = ok(ask(&s, historical(&entry, request()))?);
    assert_eq!(old["selected_task_mask"], 3);
    for row in old["rows"].as_array().expect("rows") {
        let q = &row["route_query"];
        assert_eq!(q["query"]["record"], entry["record"]);
        let expected = if row["task_key"] == "a" {
            json!(WEST)
        } else {
            json!(EAST)
        };
        assert_eq!(q["query"]["query"]["start"], expected);
        let route = ok(decode(&fortress_query(s.handle(), None, Some(q.clone())))?);
        assert_eq!(route["historical"], true);
        assert_eq!(route["anchor"], entry["anchor"]);
    }
    assert_eq!(ok(ask(&s, request())?)["anchor"], current["anchor"]);
    assert_eq!(fs::read(&files.watches).map_err(io_error)?, before);
    assert_eq!(s.calls.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn offline_site_planning_and_schema_discovery_require_no_bridge() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register_sites(&files, site_fixture::observation(3, 1, 1, true)?, vec![])?;
    let (limits, budget) = {
        let h = resolve(s.handle())?;
        let session = lock(&h)?;
        (session.limits, session.budget)
    };
    drop(s);
    let before = fs::read(&files.observations).map_err(io_error)?;
    let session = archive::open(
        next_id()?,
        Slot::reserve()?,
        &files.observations,
        limits,
        budget,
        &[Capability::Query],
    )?;
    let id = session.id;
    lock(&SESSIONS)?.insert(id, Arc::new(Mutex::new(session)));
    let s = Registered {
        id,
        calls: Arc::new(AtomicUsize::new(0)),
    };
    let result = ok(ask(&s, request())?);
    assert_eq!(result["selected_task_mask"], 3);
    assert_eq!(result["historical"], true);
    assert_eq!(result["bridge_connection_present"], false);
    assert_eq!(result["native_captures"], 0);
    let schema = ok(decode(&fortress_query(
        s.handle(),
        Some("schema".into()),
        None,
    ))?);
    let portfolio = schema["query_schema"]["$defs"]["query"]["oneOf"]
        .as_array()
        .expect("variants")
        .iter()
        .find(|v| v["properties"]["kind"]["const"] == "production_portfolio")
        .expect("portfolio schema");
    assert!(portfolio["properties"]["tasks"]["items"]["properties"]["origin"].is_object());
    assert_eq!(fs::read(&files.observations).map_err(io_error)?, before);
    Ok(())
}

#[test]
fn default_site_spellings_keep_model_and_continuation_identity() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register_sites(&files, site_fixture::observation(3, 1, 1, true)?, vec![])?;
    let mut q = request();
    q["tasks"][1]
        .as_object_mut()
        .expect("task")
        .remove("origin");
    q["limit"] = json!(1);
    let prior = ok(ask(&s, q.clone())?);
    for origin in [Value::Null, json!(WEST)] {
        q["tasks"][1]["origin"] = origin;
        let result = ok(ask(&s, q.clone())?);
        assert_eq!(result["model_digest"], prior["model_digest"]);
        assert_eq!(result["continuation"], prior["continuation"]);
    }
    Ok(())
}

#[test]
fn invalid_or_excluded_sites_and_refused_output_do_not_publish_or_capture() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register_sites(&files, site_fixture::observation(3, 1, 1, true)?, vec![])?;
    let archive = fs::read(&files.observations).map_err(io_error)?;
    let watches = fs::read(&files.watches).map_err(io_error)?;
    for origin in [
        json!([]),
        json!([4, 0]),
        json!([-1, 0, 5]),
        json!([2, 0, 5]),
        json!([32768, 0, 5]),
        json!([99, 0, 5]),
    ] {
        let mut q = request();
        q["tasks"][1]["origin"] = origin;
        assert_eq!(ask(&s, q)?["ok"], false);
    }
    {
        let h = resolve(s.handle())?;
        lock(&h)?.budget.max_output_tokens = 1;
    }
    assert_eq!(ask(&s, request())?["error"]["code"], "budget_exceeded");
    {
        let h = resolve(s.handle())?;
        let mut session = lock(&h)?;
        session.budget.max_output_tokens = 65536;
        session.grants.clear();
    }
    assert_eq!(ask(&s, request())?["error"]["code"], "capability_denied");
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    assert_eq!(fs::read(&files.observations).map_err(io_error)?, archive);
    assert_eq!(fs::read(&files.watches).map_err(io_error)?, watches);
    Ok(())
}
