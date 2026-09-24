//! Actual spatial/1.8 tools with the parent harness's coherent injected source.
use super::*;

fn preview() -> Value {
    json!({"kind":"blueprint_layout","origin":[0,0,5],
        "template":{"kind":"dining_hall","width":3,"height":3},
        "monitor":{"key":"floor-goal","deadline_tick":105u64*403200+100}})
}
fn submit(s: &Registered, request: Value) -> Result<Value> {
    decode(&fortress_query(s.handle(), None, Some(request)))
}
fn release(s: &Registered, watch: &Value) -> Result<()> {
    assert_eq!(
        ask(s, json!({"kind":"cancel_watch","watch":watch}))?["ok"],
        true
    );
    assert_eq!(
        ask(s, json!({"kind":"release_watch","watch":watch}))?["ok"],
        true
    );
    Ok(())
}

#[test]
fn preview_then_explicit_registration_and_wait_produce_stable_terrain_evidence() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let s = register(2, 2048)?;
    let p = ask(&s, preview())?;
    assert_eq!(p["ok"], true, "{p}");
    assert_eq!(p["monitoring"]["watch_registered"], false);
    assert_eq!(ask(&s, json!({"kind":"watches"}))?["records"], json!([]));
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    let created = submit(&s, p["monitoring"]["watch_request"].clone())?;
    assert_eq!(created["ok"], true, "{created}");
    assert_eq!(created["record"]["status"], "candidate");
    assert_eq!(
        created["record"]["evaluation"]["facts"][0]["requested_tiles"],
        9
    );
    let watch = created["record"]["watch"].clone();
    let same = ask(&s, json!({"kind":"poll_watch","watch":watch}))?;
    assert_eq!(same["record"]["sample_count"], 1);
    let done = ask(&s, json!({"kind":"await_watch","watch":watch}))?;
    assert_eq!(done["ok"], true, "{done}");
    assert_eq!(done["record"]["status"], "satisfied");
    assert_eq!(done["record"]["stable_observations"], 2);
    assert_eq!(s.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        done["record"]["evaluation"]["facts"][0]["mutation_cause_proven"],
        false
    );
    assert_eq!(
        decode(&fortress_commit(s.handle()))?["error"]["code"],
        "capability_denied"
    );
    release(&s, &watch)
}

#[test]
fn stale_preview_cannot_register_after_a_new_capture() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let s = register(2, 2048)?;
    let p = ask(&s, preview())?;
    assert_eq!(p["ok"], true, "{p}");
    assert_eq!(decode(&fortress_observe(s.handle()))?["ok"], true);
    let refused = submit(&s, p["monitoring"]["watch_request"].clone())?;
    assert_eq!(refused["error"]["code"], "stale_anchor");
    assert_eq!(ask(&s, json!({"kind":"watches"}))?["records"], json!([]));
    assert_eq!(s.calls.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn paged_preview_keeps_complete_monitor_and_existing_watch_without_sampling() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let s = register(2, 2048)?;
    let existing = ask(
        &s,
        json!({"kind":"watch","key":"existing","condition":{"op":"paused","value":true},
        "deadline_tick":105u64*403200+100,"stable_observations":2}),
    )?;
    assert_eq!(existing["ok"], true, "{existing}");
    let mut q = preview();
    q["origin"] = json!([2, 0, 5]);
    q["limit"] = json!(1);
    q["template"] = json!({"kind":"bedroom_cluster","rooms_count":1,"room_size":[1,1]});
    let first = ask(&s, q.clone())?;
    assert_eq!(first["ok"], true, "{first}");
    assert_eq!(first["total_rows"], 4);
    assert_eq!(
        first["monitoring"]["watch_request"]["query"]["condition"]["areas"]
            .as_array()
            .map(Vec::len),
        Some(4)
    );
    let mut indices = Vec::new();
    for _ in 0..4 {
        let raw = fortress_query(
            s.handle(),
            None,
            Some(json!({"schema":"dfmcp.query/1","query":q.clone()})),
        );
        assert!(raw.len() <= 8192);
        let page = decode(&raw)?;
        assert_eq!(page["ok"], true, "{raw}");
        assert_eq!(page["monitoring"], first["monitoring"]);
        assert_eq!(
            page["agent_turn"]["active_work"]["obligations"][0]["stable_observations"],
            1
        );
        for row in page["rows"]
            .as_array()
            .ok_or_else(|| error(ErrorCode::InvalidRequest, "test rows"))?
        {
            indices.push(row["part_index"].clone());
        }
        if page["continuation"].is_null() {
            break;
        }
        q["continuation"] = page["continuation"].clone();
        q["limit"] = json!(2);
    }
    assert_eq!(indices, vec![json!(0), json!(1), json!(2), json!(3)]);
    q["continuation"] = first["continuation"].clone();
    q["monitor"]["key"] = json!("different-goal");
    assert_eq!(ask(&s, q)?["error"]["code"], "stale_anchor");
    assert_eq!(
        ask(&s, json!({"kind":"watches"}))?["records"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    release(&s, &existing["record"]["watch"])
}

#[test]
fn invalid_or_unrenderable_proposals_do_not_register_or_dispatch() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let s = register(2, 2048)?;
    for field in ["commit", "raw_lua", "override_environmental_hazards"] {
        let mut q = preview();
        q["monitor"][field] = json!(true);
        assert_eq!(ask(&s, q)?["error"]["code"], "invalid_request");
    }
    let mut expired = preview();
    expired["monitor"]["deadline_tick"] = json!(1);
    assert_eq!(ask(&s, expired)?["error"]["code"], "invalid_request");
    {
        let handle = resolve(s.handle())?;
        lock(&handle)?.budget.max_bytes = 1;
    }
    assert_eq!(ask(&s, preview())?["ok"], false);
    {
        let handle = resolve(s.handle())?;
        lock(&handle)?.budget.max_bytes = 1024 * 1024;
    }
    assert_eq!(ask(&s, json!({"kind":"watches"}))?["records"], json!([]));
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    Ok(())
}
