//! Reuse the existing real-handler/private-journal fixture and its serial lock.
use super::*;

fn reserve(key: &str, units: u64) -> Value {
    json!({"key":key,"units":units,"item_types":["item_type_3"]})
}
fn reserved(units: u64) -> Value {
    let mut q = request();
    q["reserves"] = json!([reserve("buffer", units)]);
    q
}

#[test]
fn protected_stock_is_not_reported_as_production_consumption_or_a_reservation() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let f = Files::new()?;
    let s = register(&f, vec![])?;
    retain_watch(&s)?;
    let watched = ok(ask(&s, json!({"kind":"watches"}))?);
    let before = fs::read(&f.observations).map_err(io_error)?;
    let watch_bytes = fs::read(&f.watches).map_err(io_error)?;
    let out = ok(ask(&s, reserved(1))?);
    assert_eq!(out["selected_task_keys"], json!(["b"]));
    assert_eq!(out["assigned_stack_units"], 1);
    assert_eq!(out["reserve_constraints"]["protected_stack_units"], 1);
    assert_eq!(out["reserve_constraints"]["satisfied"], true);
    assert_eq!(out["selected_set_model_feasible"], true);
    assert_eq!(out["reservations_created"], false);
    let rows = out["rows"].as_array().expect("rows");
    let protected = rows
        .iter()
        .find(|r| r["row_kind"] == "reserve_assignment")
        .expect("reserve row");
    assert_eq!(protected["reserve_key"], "buffer");
    assert!(protected["task_key"].is_null());
    assert_eq!(protected["consumed_by_selected_tasks"], false);
    assert_eq!(protected["reservation_created"], false);
    let total: u64 = rows
        .iter()
        .filter(|r| r.get("item").is_some())
        .map(|r| r["units"].as_u64().unwrap())
        .sum();
    assert_eq!(total, 2);
    assert_eq!(
        ok(ask(&s, json!({"kind":"watches"}))?)["records"],
        watched["records"]
    );
    assert_eq!(fs::read(&f.observations).map_err(io_error)?, before);
    assert_eq!(fs::read(&f.watches).map_err(io_error)?, watch_bytes);
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn infeasible_reserves_withhold_partial_allocations_and_the_empty_optimum_claim() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let f = Files::new()?;
    let s = register(&f, vec![])?;
    let mut q = request();
    q["reserves"] = json!([reserve("emergency", 2), reserve("winter", 2)]);
    let out = ok(ask(&s, q)?);
    assert_eq!(out["optimization"]["status"], "infeasible_hard_reserves");
    assert_eq!(out["selected_set_model_feasible"], false);
    assert_eq!(out["selected_tasks"], 0);
    assert_eq!(out["assigned_stack_units"], 0);
    assert_eq!(out["assigned_workers"], 0);
    assert_eq!(out["reserve_constraints"]["protected_stack_units"], 0);
    assert_eq!(out["reserve_constraints"]["support_units"], 2);
    assert_eq!(
        out["reserve_constraints"]["partial_support_is_diagnostic_only"],
        true
    );
    assert_eq!(out["total_rows"], 1);
    assert_eq!(out["rows"][0]["row_kind"], "reserve_shortfall");
    assert_eq!(
        out["rows"][0]["reserve_keys"],
        json!(["emergency", "winter"])
    );
    assert_eq!(out["rows"][0]["deficit"], 2);
    assert_eq!(out["rows"][0]["all_task_sets_excluded"], true);
    assert!(out["continuation"].is_null());
    Ok(())
}

#[test]
fn reserve_pages_share_one_solution_and_cursors_bind_the_normalized_floors() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let f = Files::new()?;
    let s = register(&f, vec![])?;
    retain_watch(&s)?;
    {
        let h = resolve(s.handle())?;
        lock(&h)?.budget.max_output_tokens = 8192;
    }
    let mut q = reserved(1);
    q["limit"] = json!(1);
    let mut token = Value::Null;
    let mut evidence = Value::Null;
    let mut consumed = 0;
    let mut protected = 0;
    let mut pages = 0;
    loop {
        let raw = fortress_query(
            s.handle(),
            None,
            Some(json!({"schema":"dfmcp.query/1","query":q.clone()})),
        );
        assert!(raw.len() <= 32768);
        let page = ok(decode(&raw)?);
        assert_eq!(page["selected_task_mask"], 2);
        assert_eq!(page["assigned_stack_units"], 1);
        if evidence.is_null() {
            evidence = page["optimization"]["exclusion_evidence_digest"].clone();
        }
        assert_eq!(page["optimization"]["exclusion_evidence_digest"], evidence);
        for r in page["rows"].as_array().expect("rows") {
            match r["row_kind"].as_str() {
                Some("reserve_assignment") => protected += r["units"].as_u64().unwrap(),
                Some("material_assignment") => consumed += r["units"].as_u64().unwrap(),
                _ => {}
            }
        }
        pages += 1;
        assert!(pages <= 8);
        if page["continuation"].is_null() {
            break;
        }
        if token.is_null() {
            token = page["continuation"].clone();
        }
        q["continuation"] = page["continuation"].clone();
        q["limit"] = json!(2);
    }
    assert_eq!((consumed, protected), (1, 1));
    assert!(pages > 1);
    let mut changed = reserved(2);
    changed["continuation"] = token.clone();
    assert_eq!(ask(&s, changed)?["error"]["code"], "stale_anchor");
    let mut equivalent = reserved(1);
    equivalent["continuation"] = token;
    equivalent["reserves"][0]["item_types"] = json!(["item_type_3", "item_type_3"]);
    ok(ask(&s, equivalent)?);
    Ok(())
}

#[test]
fn historical_and_offline_reserve_routes_remain_on_the_exact_capture() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let f = Files::new()?;
    let s = register(&f, vec![fixture::observation(4, 2, 5, false)?])?;
    retain_watch(&s)?;
    let first = ok(ask(&s, json!({"kind":"history","limit":1}))?)["rows"][0].clone();
    ok(decode(&fortress_observe(s.handle()))?);
    assert_eq!(
        ok(ask(&s, reserved(1))?)["selected_task_keys"],
        json!(["b", "c"])
    );
    let out = ok(ask(&s, historical(&first, reserved(1)))?);
    assert_eq!(out["selected_task_keys"], json!(["b"]));
    assert_eq!(out["historical"], true);
    let route = out["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|r| r["row_kind"] == "reserve_assignment")
        .expect("reserve route")["route_query"]
        .clone();
    assert_eq!(route["query"]["record_digest"], first["record_digest"]);
    assert_eq!(
        ok(decode(&fortress_query(s.handle(), None, Some(route)))?)["anchor"],
        first["anchor"]
    );
    let (limits, budget) = {
        let h = resolve(s.handle())?;
        let guard = lock(&h)?;
        (guard.limits, guard.budget)
    };
    drop(s);
    let archive_bytes = fs::read(&f.observations).map_err(io_error)?;
    let watch_bytes = fs::read(&f.watches).map_err(io_error)?;
    let session = archive::open(
        next_id()?,
        Slot::reserve()?,
        &f.observations,
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
    let old = ok(ask(&s, historical(&first, reserved(1)))?);
    assert_eq!(old["selected_task_keys"], json!(["b"]));
    assert_eq!(old["archive_only"], true);
    assert_eq!(old["current_freshness_proven"], false);
    assert_eq!(old["native_captures"], 0);
    assert_eq!(fs::read(&f.observations).map_err(io_error)?, archive_bytes);
    assert_eq!(fs::read(&f.watches).map_err(io_error)?, watch_bytes);
    Ok(())
}

#[test]
fn schema_and_no_reserve_defaults_preserve_existing_query_identity() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let f = Files::new()?;
    let s = register(&f, vec![])?;
    let original = ok(ask(&s, request())?);
    for empty in [Value::Null, json!([])] {
        let mut q = request();
        q["reserves"] = empty;
        let out = ok(ask(&s, q)?);
        assert_eq!(out["model_digest"], original["model_digest"]);
        assert_eq!(
            out["optimization"]["exclusion_evidence_digest"],
            original["optimization"]["exclusion_evidence_digest"]
        );
        assert_eq!(out["rows"], original["rows"]);
    }
    let schema = ok(decode(&fortress_query(
        s.handle(),
        Some("schema".into()),
        None,
    ))?);
    let variant = schema["query_schema"]["$defs"]["query"]["oneOf"]
        .as_array()
        .expect("variants")
        .iter()
        .find(|v| v["properties"]["kind"]["const"] == "production_portfolio")
        .expect("portfolio");
    assert_eq!(variant["properties"]["reserves"]["maxItems"], 8);
    assert_eq!(
        variant["properties"]["reserves"]["items"],
        variant["properties"]["tasks"]["items"]["properties"]["materials"]["items"]
    );
    Ok(())
}

#[test]
fn reserve_input_budget_and_custody_failures_never_write_or_capture() -> Result<()> {
    use std::io::Write;
    let _serial = lock(&SERIAL)?;
    let f = Files::new()?;
    let s = register(&f, vec![])?;
    let original = fs::read(&f.observations).map_err(io_error)?;
    let watched = fs::read(&f.watches).map_err(io_error)?;
    for bad in [
        json!([reserve("x", 0)]),
        json!([reserve("x", 1), reserve("x", 1)]),
        json!(vec![reserve("x", 1); 9]),
    ] {
        let mut q = request();
        q["reserves"] = bad;
        assert_eq!(ask(&s, q)?["ok"], false);
    }
    {
        let h = resolve(s.handle())?;
        lock(&h)?.budget.max_output_tokens = 1;
    }
    assert_eq!(ask(&s, reserved(1))?["error"]["code"], "budget_exceeded");
    {
        let h = resolve(s.handle())?;
        lock(&h)?.budget.max_output_tokens = 65536;
    }
    assert_eq!(fs::read(&f.observations).map_err(io_error)?, original);
    fs::OpenOptions::new()
        .append(true)
        .open(&f.observations)
        .and_then(|mut out| out.write_all(b"x"))
        .map_err(io_error)?;
    assert_eq!(ask(&s, reserved(1))?["error"]["code"], "corrupt_ledger");
    assert_eq!(
        fs::read(&f.observations).map_err(io_error)?,
        [original, b"x".to_vec()].concat()
    );
    assert_eq!(fs::read(&f.watches).map_err(io_error)?, watched);
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    Ok(())
}
