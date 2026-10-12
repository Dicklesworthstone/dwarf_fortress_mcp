//! Real native projection and the existing watch evaluator/store, not a second monitor.
use super::*;
use dfmcp_adapter::live_spatial::{SpatialStateView, citizens::LiveSpatialCitizenState};
use dfmcp_adapter::shelter_analysis::assess;
use dfmcp_core::SessionId;
use dfmcp_world::{FactPresence, PredicateTruth};

#[path = "../../dfmcp-adapter/tests/support/shelter_fixture.rs"]
#[allow(dead_code)]
mod fixture;

const ADULT: u16 = 0x11f;
fn request(ids: &[u32]) -> Value {
    json!({"op":"shelter_set",
        "citizens":ids.iter().map(|id| json!({"entity_id":citizen_entity_id(*id).to_string(),"generation":1})).collect::<Vec<_>>(),
        "shelters":[{"min":[0,0,5],"max":[1,1,5]}]})
}
fn condition(ids: &[u32]) -> Result<Condition> {
    serde_json::from_value(request(ids)).map_err(|_| invalid("test shelter condition"))
}
fn snapshot(state: &LiveSpatialCitizenState) -> Result<&WorldSnapshot> {
    state.snapshot().ok_or_else(|| invalid("test shelter snapshot"))
}
fn context(state: &LiveSpatialCitizenState) -> Result<OperationContext> {
    let mut c = fixture::context(state)?;
    c.session_id = SessionId::new(981_421);
    c.budget.max_bytes = 1024 * 1024;
    c.budget.max_game_ticks = 1000;
    Ok(c)
}
fn query(storage: &Mutex<Store>, state: &LiveSpatialCitizenState, query: Value) -> Result<Value> {
    let input = json!({"schema":"dfmcp.query/1","query":query});
    let text = execute_in(storage, snapshot(state)?, &context(state)?, &input, |value| Ok(value.to_string()))?;
    serde_json::from_str(&text).map_err(|_| invalid("test watch response"))
}
fn registration(state: &LiveSpatialCitizenState, ids: &[u32]) -> Result<Value> {
    Ok(json!({"kind":"watch","key":"shelter","condition":request(ids),
        "deadline_tick":snapshot(state)?.tick.0 + 100,
        "poll_interval_ticks":10,"stable_observations":2}))
}
fn handle(value: &Value) -> Result<String> {
    value["record"]["watch"].as_str().map(str::to_owned).ok_or_else(|| invalid("test watch handle"))
}
fn poll(storage: &Mutex<Store>, state: &LiveSpatialCitizenState, watch: &str) -> Result<Value> {
    query(storage, state, json!({"kind":"poll_watch","watch":watch}))
}
fn evaluate_at(condition: &Condition, snapshot: &WorldSnapshot) -> Result<(Truth, Probe)> {
    let mut probe = Probe::default();
    let truth = probe.evaluate(condition, snapshot)?;
    Ok((truth, probe))
}

#[test]
fn shelter_watch_matches_native_assessment_across_flags_and_three_dimensional_positions() -> Result<()> {
    for flags in [ADULT, 0x05f, 0x09f, ADULT & !1, ADULT & !4, ADULT & !8] {
        for position in [[0,0,5], [1,1,5], [2,0,5], [0,0,4], [0,0,6], [-1,0,5], [32768,0,5]] {
            let state = fixture::state(&[(10, position, flags)])?;
            let native = assess(&state, &context(&state)?, &fixture::selection(&[10])?, 1_000_000)?;
            let (truth, probe) = evaluate_at(&condition(&[10])?, snapshot(&state)?)?;
            let expected = match native.arrival_truth {
                PredicateTruth::True => Truth::True,
                PredicateTruth::False => Truth::False,
                PredicateTruth::Unknown => Truth::Unknown,
            };
            assert_eq!(truth, expected, "flags={flags} position={position:?}");
            assert_eq!(probe.facts[0]["records"][0]["status"], native.rows[0].status.as_str());
            assert_eq!(probe.facts[0]["selection_digest"], native.selection.digest().to_string());
        }
    }
    let state = fixture::state(&[(10, [0,0,5], ADULT)])?;
    let (truth, probe) = evaluate_at(&condition(&[10,11])?, snapshot(&state)?)?;
    assert_eq!(truth, Truth::Unknown);
    assert_eq!(probe.facts[0]["selected"], 2);
    assert_eq!(probe.facts[0]["unresolved"], 1);
    Ok(())
}

#[test]
fn separately_arrived_citizens_never_complete_the_joint_watch() -> Result<()> {
    let storage = Mutex::new(Store::default());
    let mut state = fixture::state(&[(10,[0,0,5],ADULT),(11,[2,0,5],ADULT)])?;
    let first = query(&storage, &state, registration(&state, &[10,11])?)?;
    assert_eq!(first["record"]["status"], "waiting");
    let watch = handle(&first)?;
    state.publish(fixture::observation(&[(10,[2,0,5],ADULT),(11,[0,0,5],ADULT)],10,7)?)?;
    assert_eq!(poll(&storage,&state,&watch)?["record"]["stable_observations"],0);
    state.publish(fixture::observation(&[(10,[0,0,5],ADULT),(11,[0,0,5],ADULT)],20,7)?)?;
    let candidate = poll(&storage,&state,&watch)?;
    assert_eq!(candidate["record"]["status"],"candidate");
    assert_eq!(candidate["record"]["stable_observations"],1);
    assert_eq!(poll(&storage,&state,&watch)?["record"],candidate["record"]);
    state.publish(fixture::observation(&[(10,[1,0,5],ADULT),(11,[0,0,5],ADULT)],20,7)?)?;
    assert_eq!(poll(&storage,&state,&watch)?["record"]["stable_observations"],1);
    state.publish(fixture::observation(&[(10,[0,0,5],ADULT),(11,[0,0,5],ADULT)],30,7)?)?;
    let done = poll(&storage,&state,&watch)?;
    assert_eq!(done["record"]["status"],"satisfied");
    let receipt = done["record"]["evidence_digest"].clone();
    state.publish(fixture::observation(&[(10,[2,0,5],ADULT),(11,[0,0,5],ADULT)],40,7)?)?;
    let history = poll(&storage,&state,&watch)?;
    assert_eq!(history["record"]["status"],"satisfied");
    assert_eq!(history["record"]["evidence_digest"],receipt);
    assert_eq!(history["record"]["evaluation_current"],false);
    assert_eq!(evaluate_at(&condition(&[10,11])?,snapshot(&state)?)?.0,Truth::False);
    Ok(())
}

#[test]
fn missing_positions_cursor_gaps_and_recycled_incarnations_do_not_keep_stability() -> Result<()> {
    let storage = Mutex::new(Store::default());
    let mut state = fixture::state(&[(10,[0,0,5],ADULT)])?;
    let watch = handle(&query(&storage,&state,registration(&state,&[10])?)?)?;
    state.publish(fixture::observation(&[(10,[-1,0,5],ADULT)],10,7)?)?;
    let unknown = poll(&storage,&state,&watch)?;
    assert_eq!(unknown["record"]["status"],"blocked_unknown");
    assert_eq!(unknown["record"]["stable_observations"],0);
    state.publish(fixture::observation(&[(10,[0,0,5],ADULT)],20,7)?)?;
    assert_eq!(poll(&storage,&state,&watch)?["record"]["stable_observations"],1);
    state.publish(fixture::observation(&[(10,[2,0,5],ADULT)],30,7)?)?;
    state.publish(fixture::observation(&[(10,[0,0,5],ADULT)],40,7)?)?;
    let gap = poll(&storage,&state,&watch)?;
    assert_eq!(gap["record"]["evaluation"]["skipped_observations_reset_streak"],true);
    assert_eq!(gap["record"]["stable_observations"],1);
    state.publish(fixture::observation(&[],50,7)?)?;
    assert_eq!(poll(&storage,&state,&watch)?["record"]["status"],"blocked_unknown");
    state.publish(fixture::observation(&[(10,[0,0,5],ADULT)],60,7)?)?;
    assert_eq!(poll(&storage,&state,&watch)?["record"]["status"],"invalidated");
    Ok(())
}

#[test]
fn false_and_unknown_cannot_hide_a_changed_generation_in_a_boolean_group() -> Result<()> {
    let mut state = fixture::state(&[(10,[2,0,5],ADULT),(11,[0,0,5],ADULT)])?;
    state.publish(fixture::observation(&[(10,[2,0,5],ADULT)],10,7)?)?;
    state.publish(fixture::observation(&[(10,[2,0,5],ADULT),(11,[0,0,5],ADULT)],20,7)?)?;
    let (_, probe) = evaluate_at(&Condition::Any { args:vec![
        Condition::Paused {value:snapshot(&state)?.paused}, condition(&[10,11])?,
    ]},snapshot(&state)?)?;
    assert!(probe.invalid_generation);
    assert_eq!(probe.facts[1]["records"].as_array().map(Vec::len),Some(2));
    Ok(())
}

#[test]
fn unqualified_stale_and_cross_capture_fields_remain_unknown() -> Result<()> {
    let state = fixture::state(&[(10,[0,0,5],ADULT)])?;
    for field in ["position","alive","active","visible","citizen","resident","native_unit_id"] {
        for case in 0..6 {
            let mut world = snapshot(&state)?.clone();
            let record = world.graph.entities.get_mut(&citizen_entity_id(10)).ok_or_else(|| invalid("test unit"))?;
            let fact = record.fields.get_mut(field).ok_or_else(|| invalid("test field"))?;
            match case {
                0 => fact.source = FactSource::AgentAssertion("not an observation".into()),
                1 => fact.source_digest = Digest32::of_bytes(b"different capture"),
                2 => fact.observed_at = GameTick(world.tick.0 + 1),
                3 => fact.presence = Some(FactPresence::Unknown("unavailable".into())),
                4 => { fact.value = WorldValue::Text("wrong type".into()); fact.presence = None; }
                _ => fact.source = FactSource::DfhackField("different.native.field".into()),
            }
            world.refresh_hash();
            assert_eq!(evaluate_at(&condition(&[10])?,&world)?.0,Truth::Unknown,"{field} mutation {case}");
        }
    }
    for field in ["observation_profile","strict_citizen_count","map_dimensions"] {
        let mut world = snapshot(&state)?.clone();
        world.graph.entities.get_mut(&EntityId::new(1)).ok_or_else(|| invalid("test root"))?.fields.remove(field);
        world.refresh_hash();
        assert_eq!(evaluate_at(&condition(&[10])?,&world)?.0,Truth::Unknown);
    }
    Ok(())
}

#[test]
fn closed_canonical_conditions_round_trip_and_reject_partial_or_ambiguous_goals() -> Result<()> {
    let original = condition(&[10,11])?;
    let encoded = serde_json::to_value(&original).map_err(|_| invalid("test encoding"))?;
    assert_eq!(encoded,request(&[10,11]));
    let restored: Condition = serde_json::from_value(encoded).map_err(|_| invalid("test decode"))?;
    assert_eq!(restored,original);
    let state = fixture::state(&[(10,[0,0,5],ADULT),(11,[0,0,5],ADULT)])?;
    for invalid_goal in [request(&[]),request(&[10,10]),request(&[11,10])] {
        let storage = Mutex::new(Store::default());
        let mut input = registration(&state,&[10])?;
        input["condition"] = invalid_goal;
        assert!(query(&storage,&state,input).is_err());
        assert!(lock(&storage)?.entries.is_empty());
    }
    let mut forged = request(&[10]); forged["source_policy"] = json!("trusted");
    assert!(serde_json::from_value::<Condition>(forged).is_err());
    let legacy = json!({"op":"field","entity_id":"1","generation":1,"field":"paused",
        "comparison":"eq","value":{"type":"bool","value":true}});
    let legacy_condition: Condition = serde_json::from_value(legacy.clone()).map_err(|_| invalid("test legacy"))?;
    assert_eq!(serde_json::to_value(legacy_condition).map_err(|_| invalid("test legacy encode"))?,legacy);
    Ok(())
}

#[test]
fn failed_full_publication_does_not_register_or_advance_a_shelter_watch() -> Result<()> {
    let storage = Mutex::new(Store::default());
    let mut state = fixture::state(&[(10,[0,0,5],ADULT)])?;
    let input = json!({"schema":"dfmcp.query/1","query":registration(&state,&[10])?});
    assert!(execute_in(&storage,snapshot(&state)?,&context(&state)?,&input,|_| Err(bounded("test full output refusal"))).is_err());
    assert!(lock(&storage)?.entries.is_empty());
    let first = query(&storage,&state,registration(&state,&[10])?)?;
    let watch = handle(&first)?;
    state.publish(fixture::observation(&[(10,[0,0,5],ADULT)],10,7)?)?;
    let input = json!({"schema":"dfmcp.query/1","query":{"kind":"poll_watch","watch":watch}});
    assert!(execute_in(&storage,snapshot(&state)?,&context(&state)?,&input,|_| Err(bounded("test full output refusal"))).is_err());
    let retained = lock(&storage)?;
    let row = retained.entries.values().next().ok_or_else(|| invalid("test retained watch"))?;
    assert_eq!(row.streak,1);
    assert_eq!(row.evidence_digest.to_string(),first["record"]["evidence_digest"]);
    drop(retained);
    assert_eq!(poll(&storage,&state,&watch)?["record"]["status"],"satisfied");
    Ok(())
}

#[test]
fn cancellation_deadline_and_query_authority_remain_owned_by_the_existing_engine() -> Result<()> {
    for cancel in [true,false] {
        let storage = Mutex::new(Store::default());
        let mut state = fixture::state(&[(10,[2,0,5],ADULT)])?;
        let watch = handle(&query(&storage,&state,registration(&state,&[10])?)?)?;
        if cancel {
            let before = snapshot(&state)?.anchor();
            assert_eq!(query(&storage,&state,json!({"kind":"cancel_watch","watch":watch}))?["record"]["status"],"cancelled");
            assert_eq!(snapshot(&state)?.anchor(),before);
        } else {
            state.publish(fixture::observation(&[(10,[2,0,5],ADULT)],100,7)?)?;
            assert_eq!(poll(&storage,&state,&watch)?["record"]["status"],"expired");
        }
        let mut denied = context(&state)?; denied.grants.clear();
        let input = json!({"schema":"dfmcp.query/1","query":{"kind":"poll_watch","watch":watch}});
        assert!(matches!(execute_in(&storage,snapshot(&state)?,&denied,&input,|v| Ok(v.to_string())),Err(e) if e.code == ErrorCode::CapabilityDenied));
    }
    Ok(())
}

#[test]
fn maximum_group_retains_every_target_and_geometry_under_one_condition() -> Result<()> {
    let rows:Vec<_> = (10..138).map(|id| (id,[0,0,5],ADULT)).collect();
    let state = fixture::state(&rows)?;
    let mut value = request(&(10..138).collect::<Vec<_>>());
    value["shelters"] = json!((0..16).map(|x| json!({"min":[x,0,5],"max":[x,0,5]})).collect::<Vec<_>>());
    let c:Condition = serde_json::from_value(value.clone()).map_err(|_| invalid("test maximum condition"))?;
    let (truth,probe) = evaluate_at(&c,snapshot(&state)?)?;
    assert_eq!(truth,Truth::True);
    assert_eq!(probe.facts[0]["selected"],128);
    assert_eq!(probe.facts[0]["records"].as_array().map(Vec::len),Some(128));
    let mut q = registration(&state,&[10])?; q["condition"] = value;
    let storage = Mutex::new(Store::default());
    assert_eq!(query(&storage,&state,q)?["record"]["status"],"candidate");
    Ok(())
}
