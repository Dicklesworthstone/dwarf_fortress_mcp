//! Reuse the real private-journal/actual-handler fixture from quantity timelines.
//! These tests never run DFHack and do not retroactively advance saved watches.
use super::*;

fn predicate(threshold: u64) -> Value {
    json!({"kind":"condition_evaluation","condition":{"op":"all","args":[
        {"op":"entity_count","scope":"observed_projection","kind":"unit","predicate":{"op":"always"},"comparison":"ge","value":1},
        {"op":"item_quantity","scope":"observed_projection","quantity_unit":"stack_units","predicate":{"op":"always"},"comparison":"ge","value":threshold}]},
        "failure_condition":{"op":"paused","value":false}})
}
fn stock(s: &Registered) -> Result<u64> {
    ok(ask(s, measure())?)["quantity"]["quantity_min"]
        .as_u64()
        .ok_or_else(|| invalid("stock fixture"))
}
fn condition_timeline(s: &Registered, measurement: Value) -> Result<Value> {
    let mut query = timeline(s)?;
    query["measurement"] = measurement;
    Ok(query)
}

#[test]
fn compound_condition_history_matches_each_exact_inspection_without_changing_work() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(&files, &[(4, 7), (5, 12)])?;
    let measurement = predicate(stock(&s)?);
    watch(&s)?;
    let initial = ok(ask(&s, measurement.clone())?);
    assert_eq!(initial["evaluation"]["status"], "condition_met");
    capture(&s)?;
    capture(&s)?;
    let before = ok(ask(&s, json!({"kind":"watches"}))?);
    let observations = fs::read(&files.observations).map_err(io_error)?;
    let watches = fs::read(&files.watches).map_err(io_error)?;
    let current = ok(ask(&s, measurement.clone())?);
    let result = ok(ask(&s, condition_timeline(&s, measurement.clone())?)?);
    let rows = result["rows"]
        .as_array()
        .ok_or_else(|| invalid("condition rows"))?;
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows.iter()
            .map(|r| r["evaluation"]["status"].as_str().unwrap_or(""))
            .collect::<Vec<_>>(),
        ["condition_met", "condition_not_met", "condition_met"]
    );
    for row in rows {
        let record = &row["record"];
        let exact = ok(ask(
            &s,
            json!({"kind":"historical_query","record":record["record"],
            "record_digest":record["record_digest"],"query":measurement.clone()}),
        )?);
        assert_eq!(row["evaluation"], exact["evaluation"]);
        assert_eq!(row["evidence_digest"], exact["evidence_digest"]);
        assert_eq!(row["predicate_digest"], exact["predicate_digest"]);
        assert_eq!(row["evaluation"]["watch_completion_proven"], false);
    }
    assert!(rows[0]["change_from_previous"].is_null());
    assert_eq!(
        rows[1]["change_from_previous"]["status"],
        "evaluation_classification_changed"
    );
    assert_eq!(ok(ask(&s, measurement)?)["anchor"], current["anchor"]);
    assert_eq!(
        ok(ask(&s, json!({"kind":"watches"}))?)["records"],
        before["records"]
    );
    assert_eq!(
        fs::read(&files.observations).map_err(io_error)?,
        observations
    );
    assert_eq!(fs::read(&files.watches).map_err(io_error)?, watches);
    assert_eq!(s.calls.load(Ordering::SeqCst), 2);
    Ok(())
}

#[test]
fn paged_condition_transitions_match_one_page_and_bind_the_failure_guard() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(&files, &[(4, 7), (5, 12)])?;
    let measurement = predicate(stock(&s)?);
    watch(&s)?;
    capture(&s)?;
    capture(&s)?;
    let mut query = condition_timeline(&s, measurement)?;
    let complete = ok(ask(&s, query.clone())?);
    {
        let h = resolve(s.handle())?;
        lock(&h)?.budget.max_output_tokens = 4096;
    }
    query["limit"] = json!(1);
    let mut rows = Vec::new();
    let mut pages = 0;
    let mut first_token = Value::Null;
    loop {
        let raw = fortress_query(
            s.handle(),
            None,
            Some(json!({"schema":"dfmcp.query/1","query":query.clone()})),
        );
        assert!(raw.len() <= 16384);
        let result = ok(decode(&raw)?);
        let samples = result["rows"]
            .as_array()
            .ok_or_else(|| invalid("paged conditions"))?;
        assert!(!samples.is_empty());
        rows.extend(samples.iter().cloned());
        pages += 1;
        assert!(pages <= 3);
        assert_eq!(
            result["agent_turn"]["coverage"]["continuation"],
            result["continuation"]
        );
        if result["continuation"].is_null() {
            break;
        }
        if first_token.is_null() {
            first_token = result["continuation"].clone();
        }
        query["continuation"] = result["continuation"].clone();
        query["limit"] = json!(2);
    }
    assert_eq!(Value::Array(rows), complete["rows"]);
    assert!(pages > 1);
    query["continuation"] = first_token;
    query["measurement"]["failure_condition"] = json!({"op":"paused","value":true});
    assert_eq!(ask(&s, query)?["error"]["code"], "stale_anchor");
    Ok(())
}

#[test]
fn missing_and_recycled_entities_never_become_successful_historical_samples() -> Result<()> {
    use dfmcp_adapter::live_spatial::citizens::citizen_entity_id;
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(&files, &[])?;
    let measurement = json!({"kind":"condition_evaluation","condition":{"op":"field",
        "entity_id":citizen_entity_id(10).to_string(),"generation":1,"field":"active",
        "comparison":"eq","value":{"type":"bool","value":true}}});
    {
        let h = resolve(s.handle())?;
        let mut session = lock(&h)?;
        session.source = Box::new(Script {
            calls: s.calls.clone(),
            fenced: false,
            values: vec![
                fixture::observation(4, 0, 0, true)?,
                fixture::observation(5, 1, 10, false)?,
            ]
            .into(),
        });
    }
    capture(&s)?;
    capture(&s)?;
    let result = ok(ask(&s, condition_timeline(&s, measurement)?)?);
    assert_eq!(result["rows"][0]["evaluation"]["status"], "condition_met");
    assert_eq!(result["rows"][1]["evaluation"]["status"], "blocked_unknown");
    assert_eq!(
        result["rows"][2]["evaluation"]["status"],
        "invalidated_reference"
    );
    assert_eq!(
        result["rows"][2]["evaluation"]["eligible_success_sample"],
        false
    );
    Ok(())
}

#[test]
fn failed_source_and_offline_reopening_preserve_condition_evidence_not_current_claims() -> Result<()>
{
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(&files, &[(4, 7)])?;
    let measurement = predicate(stock(&s)?);
    capture(&s)?;
    let query = condition_timeline(&s, measurement.clone())?;
    let original = ok(ask(&s, query.clone())?);
    let (limits, budget) = {
        let h = resolve(s.handle())?;
        let mut session = lock(&h)?;
        session.source.fence();
        (session.limits, session.budget)
    };
    assert_eq!(
        ask(&s, measurement.clone())?["error"]["code"],
        "adapter_unavailable"
    );
    assert_eq!(ok(ask(&s, query.clone())?)["rows"], original["rows"]);
    drop(s);
    let before = fs::read(&files.observations).map_err(io_error)?;
    let watch_bytes = fs::read(&files.watches).map_err(io_error)?;
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
    let result = ok(ask(&s, query)?);
    assert_eq!(result["rows"], original["rows"]);
    assert_eq!(result["archive_only"], true);
    assert_eq!(result["current_freshness_proven"], false);
    let latest = ok(ask(&s, measurement)?);
    assert_eq!(latest["historical"], true);
    assert_eq!(latest["evaluation"], result["rows"][1]["evaluation"]);
    assert_eq!(latest["bridge_connection_present"], false);
    assert_eq!(
        ask(&s, json!({"kind":"watch","key":"not-permitted"}))?["error"]["code"],
        "capability_denied"
    );
    assert_eq!(fs::read(&files.observations).map_err(io_error)?, before);
    assert_eq!(fs::read(&files.watches).map_err(io_error)?, watch_bytes);
    Ok(())
}

#[test]
fn reset_boundaries_and_unknown_guards_never_establish_continuous_completion() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(&files, &[(3, 7), (2, 99)])?;
    capture(&s)?;
    capture(&s)?;
    let measurement = json!({"kind":"condition_evaluation","condition":{"op":"paused","value":true},
        "failure_condition":{"op":"field","entity_id":"999999","generation":1,"field":"missing",
            "comparison":"eq","value":{"type":"bool","value":true}}});
    let result = ok(ask(&s, condition_timeline(&s, measurement)?)?);
    assert_eq!(result["rows"][0]["evaluation"]["status"], "blocked_unknown");
    assert_eq!(
        result["rows"][1]["change_from_previous"]["status"],
        "same_evaluation_classification"
    );
    assert_eq!(
        result["rows"][1]["change_from_previous"]["elapsed_game_ticks"],
        0
    );
    assert_eq!(
        result["rows"][2]["change_from_previous"]["status"],
        "epoch_or_clock_discontinuity"
    );
    assert!(
        result["rows"][2]["change_from_previous"]
            .get("condition_truth")
            .is_none()
    );
    Ok(())
}

#[test]
fn malformed_conditions_output_refusal_and_current_expiry_leave_saved_work_untouched() -> Result<()>
{
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(&files, &[(4, 7)])?;
    let measurement = predicate(stock(&s)?);
    watch(&s)?;
    capture(&s)?;
    let query = condition_timeline(&s, measurement)?;
    let observations = fs::read(&files.observations).map_err(io_error)?;
    let watches = fs::read(&files.watches).map_err(io_error)?;
    let mut bad = query.clone();
    bad["measurement"]["condition"] = json!({"op":"all","args":[]});
    assert_eq!(ask(&s, bad)?["ok"], false);
    {
        let h = resolve(s.handle())?;
        lock(&h)?.budget.max_output_tokens = 1;
    }
    assert_eq!(ask(&s, query.clone())?["error"]["code"], "budget_exceeded");
    {
        let h = resolve(s.handle())?;
        let mut session = lock(&h)?;
        session.budget.max_output_tokens = 65536;
        for g in &mut session.grants {
            g.expires_at_tick = Some(GameTick(105u64 * 403200 + 3));
        }
    }
    assert_eq!(ask(&s, query)?["error"]["code"], "capability_denied");
    assert_eq!(
        fs::read(&files.observations).map_err(io_error)?,
        observations
    );
    assert_eq!(fs::read(&files.watches).map_err(io_error)?, watches);
    assert_eq!(s.calls.load(Ordering::SeqCst), 1);
    Ok(())
}
