//! Use the production suite's coherent captures, real private journals and
//! shared serial guard. These tests call the actual MCP query/cancel handlers.
use super::*;
use dfmcp_adapter::live_operations::OperationsProfile;

fn predicate() -> Value {
    json!({"op":"field","field":"type_key","comparison":"eq","value":{"type":"text","value":"item_type_3"}})
}
fn quantity() -> Value {
    json!({"kind":"item_quantity","scope":"observed_projection","quantity_unit":"stack_units","predicate":predicate()})
}
fn watch_quantity(key: &str, stable: u32) -> Value {
    json!({"kind":"watch","key":key,"condition":{"op":"item_quantity","scope":"observed_projection",
        "quantity_unit":"stack_units","predicate":predicate(),"comparison":"ge","value":7},
        "deadline_tick":105u64*403200+100,"stable_observations":stable})
}
fn close(s: &Registered) -> Result<()> {
    ok(decode(&fortress_cancel(
        s.handle(),
        Some("session".into()),
        None,
    ))?)?;
    Ok(())
}

/// Repartition only the selected item stacks in the established fixture.
/// Retain its citizens and same-capture operations/terrain identity.
fn partition(tick: u32, units: &[u32]) -> Result<LiveSpatialCitizenObservation> {
    let base = fixture::observation(tick, 1, 7, false)?;
    let mut operations = base.spatial().operations().clone();
    let prototype = operations
        .items
        .iter()
        .find(|item| item.native_id == 32)
        .cloned()
        .ok_or_else(|| error(ErrorCode::InvalidRequest, "quantity fixture item"))?;
    operations.items.retain(|item| item.native_id != 32);
    for (index, units) in units.iter().enumerate() {
        let mut item = prototype.clone();
        item.native_id = 100 + index as u32;
        item.stack_size = *units;
        operations.items.push(item);
    }
    fn part(out: &mut Vec<u8>, bytes: &[u8]) {
        out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        out.extend_from_slice(bytes);
    }
    let mut spatial = b"DFMS1600".to_vec();
    part(
        &mut spatial,
        &operations.encode_profile(OperationsProfile::PagedV1_4)?,
    );
    part(&mut spatial, &base.spatial().terrain().encode_payload()?);
    let original = base.encode_payload()?;
    let length = original
        .get(8..12)
        .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
        .map(u32::from_be_bytes)
        .ok_or_else(|| error(ErrorCode::InvalidRequest, "quantity fixture framing"))?
        as usize;
    let citizens = original
        .get(12 + length..)
        .ok_or_else(|| error(ErrorCode::InvalidRequest, "quantity fixture citizens"))?;
    let mut combined = b"DFMS1800".to_vec();
    part(&mut combined, &spatial);
    combined.extend_from_slice(citizens);
    LiveSpatialCitizenObservation::decode_payload(&combined, 7, "df".into(), "dfhack".into())
}

#[test]
fn quantity_inspection_is_complete_and_does_not_sample_or_persist_watch_progress() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(&files, 1, vec![])?;
    let created = ok(ask(&s, watch_quantity("stock", 3))?)?;
    let records = ok(ask(&s, json!({"kind":"watches"}))?)?["records"].clone();
    let bytes = fs::read(&files.watches).map_err(io_error)?;
    {
        let handle = resolve(s.handle())?;
        lock(&handle)?.budget.max_output_tokens = 4096;
    }
    let raw = fortress_query(
        s.handle(),
        None,
        Some(json!({"schema":"dfmcp.query/1","query":quantity()})),
    );
    assert!(raw.len() <= 16384);
    let result = ok(decode(&raw)?)?;
    assert_eq!(result["quantity"]["quantity_min"], 7);
    assert_eq!(result["quantity"]["quantity_max"], 7);
    assert_eq!(result["quantity"]["matched_records_min"], 1);
    assert_eq!(result["quantity"]["quantity_exact"], true);
    assert_eq!(result["watch_registered"], false);
    assert_eq!(result["watch_evaluated"], false);
    assert_eq!(result["quantity"]["usable_supply_proven"], false);
    assert_eq!(
        result["quantity"]["quantity_min"],
        created["record"]["evaluation"]["facts"][0]["quantity_min"]
    );
    assert_eq!(ok(ask(&s, json!({"kind":"watches"}))?)?["records"], records);
    assert_eq!(fs::read(&files.watches).map_err(io_error)?, bytes);
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    close(&s)
}

#[test]
fn splitting_and_merging_stacks_preserves_quantity_stability_in_a_shared_capture_batch()
-> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(&files, 1, vec![partition(4, &[3, 4])?, partition(5, &[7])?])?;
    ok(ask(&s, watch_quantity("units", 3))?)?;
    ok(ask(
        &s,
        json!({"kind":"watch","key":"records","condition":{"op":"entity_count",
        "scope":"observed_projection","kind":"item","predicate":predicate(),"comparison":"ge","value":1},
        "deadline_tick":105u64*403200+100,"stable_observations":3}),
    )?)?;
    for (step, stacks) in [(1, 2), (2, 1)] {
        let result = ok(ask(&s, json!({"kind":"await_watches"}))?)?;
        assert_eq!(result["native_captures"], 1);
        assert_eq!(result["all_satisfied"], step == 2);
        for record in result["records"]
            .as_array()
            .ok_or_else(|| error(ErrorCode::InvalidRequest, "batch records"))?
        {
            assert_eq!(record["stable_observations"], step + 1);
        }
        let measured = ok(ask(&s, quantity())?)?;
        assert_eq!(measured["quantity"]["quantity_min"], 7);
        assert_eq!(measured["quantity"]["matched_records_min"], stacks);
    }
    assert_eq!(s.calls.load(Ordering::SeqCst), 2);
    close(&s)
}

#[test]
fn quantity_definitions_recover_without_counting_restart_as_a_successful_sample() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(&files, 1, vec![])?;
    let old = ok(ask(&s, watch_quantity("stock", 2))?)?["record"]["watch"].clone();
    let before = fs::read(&files.watches).map_err(io_error)?;
    close(&s)?;
    assert_eq!(fs::read(&files.watches).map_err(io_error)?, before);
    let next = register(
        &files,
        1,
        vec![
            fixture::observation(4, 1, 7, false)?,
            fixture::observation(5, 1, 7, false)?,
        ],
    )?;
    let records = ok(ask(&next, json!({"kind":"watches"}))?)?["records"].clone();
    assert_ne!(records[0]["watch"], old);
    assert_eq!(records[0]["stable_observations"], 0);
    assert_eq!(records[0]["fresh_observation_required"], true);
    let record = ok(ask(
        &next,
        json!({"kind":"poll_watch","watch":records[0]["watch"]}),
    )?)?;
    assert_eq!(
        record["record"]["definition"]["condition"]["op"],
        "item_quantity"
    );
    ok(ask(&next, quantity())?)?;
    assert_eq!(
        ok(ask(&next, json!({"kind":"watches"}))?)?["records"],
        records
    );
    for step in 1..=2 {
        let result = ok(ask(&next, json!({"kind":"await_watches"}))?)?;
        assert_eq!(result["records"][0]["stable_observations"], step);
        assert_eq!(result["all_satisfied"], step == 2);
    }
    assert_eq!(next.calls.load(Ordering::SeqCst), 2);
    close(&next)
}

#[test]
fn historical_and_offline_quantity_queries_remain_on_the_exact_record() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(&files, 1, vec![fixture::observation(4, 1, 12, false)?])?;
    let first = first_record(&s)?;
    retain_watch(&s)?;
    ok(decode(&fortress_observe(s.handle()))?)?;
    let before = fs::read(&files.watches).map_err(io_error)?;
    let current = ok(ask(&s, quantity())?)?;
    assert_eq!(current["quantity"]["quantity_min"], 12);
    let old = ok(ask(&s, historical(&first, quantity()))?)?;
    assert_eq!(old["quantity"]["quantity_min"], 7);
    assert_eq!(old["anchor"], first["anchor"]);
    assert_eq!(old["historical"], true);
    assert_eq!(old["watch_evaluated"], false);
    assert_eq!(ok(ask(&s, quantity())?)?["anchor"], current["anchor"]);
    assert_eq!(fs::read(&files.watches).map_err(io_error)?, before);
    let (limits, budget) = {
        let handle = resolve(s.handle())?;
        let session = lock(&handle)?;
        (session.limits, session.budget)
    };
    close(&s)?;
    let observation_bytes = fs::read(&files.observations).map_err(io_error)?;
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
    let offline = Registered {
        id,
        calls: Arc::new(AtomicUsize::new(0)),
    };
    let latest = ok(ask(&offline, quantity())?)?;
    assert_eq!(latest["quantity"]["quantity_min"], 12);
    assert_eq!(latest["current_freshness_proven"], false);
    assert_eq!(latest["bridge_connection_present"], false);
    assert_eq!(
        ok(ask(&offline, historical(&first, quantity()))?)?["quantity"]["quantity_min"],
        7
    );
    assert_eq!(
        ask(&offline, watch_quantity("no-monitoring", 2))?["ok"],
        false
    );
    close(&offline)?;
    assert_eq!(
        fs::read(&files.observations).map_err(io_error)?,
        observation_bytes
    );
    assert_eq!(fs::read(&files.watches).map_err(io_error)?, before);
    Ok(())
}

#[test]
fn quantity_schema_is_composed_once_and_rejected_results_leave_monitoring_intact() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(&files, 1, vec![])?;
    let schema = ok(decode(&fortress_query(
        s.handle(),
        Some("schema".into()),
        None,
    ))?)?;
    let definitions = &schema["query_schema"]["$defs"];
    let conditions = definitions["watch_condition"]["oneOf"]
        .as_array()
        .ok_or_else(|| error(ErrorCode::InvalidRequest, "conditions"))?;
    for op in ["entity_count", "item_quantity"] {
        assert_eq!(
            conditions
                .iter()
                .filter(|v| v["properties"]["op"]["const"] == op)
                .count(),
            1
        );
    }
    let queries = definitions["query"]["oneOf"]
        .as_array()
        .ok_or_else(|| error(ErrorCode::InvalidRequest, "queries"))?;
    assert_eq!(
        queries
            .iter()
            .filter(|v| v["properties"]["kind"]["const"] == "item_quantity")
            .count(),
        1
    );
    ok(ask(&s, watch_quantity("stock", 3))?)?;
    let bytes = fs::read(&files.watches).map_err(io_error)?;
    {
        let handle = resolve(s.handle())?;
        lock(&handle)?.budget.max_output_tokens = 1;
    }
    assert_eq!(ask(&s, quantity())?["error"]["code"], "budget_exceeded");
    {
        let handle = resolve(s.handle())?;
        lock(&handle)?.budget.max_output_tokens = 65536;
    }
    let mut wrong = quantity();
    wrong["quantity_unit"] = json!("food_portions");
    assert_eq!(ask(&s, wrong)?["error"]["code"], "invalid_request");
    assert_eq!(fs::read(&files.watches).map_err(io_error)?, bytes);
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    close(&s)
}
