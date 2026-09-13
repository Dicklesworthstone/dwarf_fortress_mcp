use super::*;

// At most two new test sessions coexist, so these scenarios cannot consume the
// fixed production capacity while the five original handler tests run in parallel.
static SERIAL: Mutex<()> = Mutex::new(());

fn stock(session: &Registered, count: u32) -> Result<()> {
    let handle=resolve(session.handle())?;
    let mut guard=lock(&handle)?;
    let mut value=observation()?;
    value.jobs.jobs.clear();value.attachments.clear();
    let mut template=value.items[0].clone();
    template.type_key="BAR".to_owned();template.flags=64;template.stack_size=1;
    template.container_native_id=None;template.holder_building_native_id=None;
    value.items=(0..count).map(|i| {
        let mut item=template.clone();item.native_id=100+i;item
    }).collect();
    value.next_item_id=100+count;
    let mut state=LiveOperationsState::default();state.publish(value)?;
    guard.state=state;
    Ok(())
}
fn allocation(units:u64,limit:u32)->Value {
    json!({"kind":"inventory_plan","quantity_unit":"stack_units",
        "demands":[{"key":"metal","units":units,"item_types":["BAR"]}],"limit":limit})
}

#[test]
fn diagnostics_are_available_in_the_actual_tool_without_native_io()->Result<()> {
    let _serial=lock(&SERIAL)?;
    let session=full()?;
    let result=decode(&fortress_query(session.handle(),Some("production".to_owned()),None))?;
    assert_eq!(result["ok"],true);assert_eq!(result["kind"],"production_diagnosis");
    assert_eq!(result["summary"]["jobs_considered"],1);
    assert_eq!(result["rows"][0]["job"]["entity_id"],"9");
    assert_eq!(result["rows"][0]["holder"]["entity_id"],building_entity_id(20).to_string());
    assert_eq!(result["rows"][0]["holder_stage"],json!({"stage":1,"maximum":3}));
    assert_eq!(result["rows"][0]["blocker_proven"],false);
    assert_eq!(result["rows"][0]["job_ready_proven"],false);
    assert_eq!(result["source_digest"],observation()?.source_digest()?.to_string());
    assert_eq!(result["anchor"],result["agent_turn"]["anchor"]);
    assert_eq!(session.calls.load(Ordering::SeqCst),0);
    let focus=ask(&session,json!({"kind":"production_diagnosis",
        "holder":{"entity_id":building_entity_id(21).to_string(),"generation":1}}))?;
    assert_eq!(focus["ok"],true);assert_eq!(focus["summary"]["jobs_considered"],0);
    assert_eq!(focus["total_rows"],0);
    Ok(())
}

#[test]
fn discovered_operations_schema_extends_but_does_not_rewrite_other_profiles()->Result<()> {
    let _serial=lock(&SERIAL)?;
    let session=full()?;
    let result=decode(&fortress_query(session.handle(),Some("schema".to_owned()),None))?;
    assert_eq!(result["ok"],true);
    let original:Value=serde_json::from_str(include_str!("../../../schemas/mcp_query_v1.json"))
        .map_err(|_|error(ErrorCode::InternalInvariantViolation,"base schema"))?;
    let base=original["$defs"]["query"]["oneOf"].as_array()
        .ok_or_else(||error(ErrorCode::InternalInvariantViolation,"base variants"))?;
    let variants=result["query_schema"]["$defs"]["query"]["oneOf"].as_array()
        .ok_or_else(||error(ErrorCode::InternalInvariantViolation,"extended variants"))?;
    assert_eq!(base.len(),16);assert_eq!(variants.len(),18);
    assert_eq!(&variants[..16],base.as_slice());
    assert_eq!(variants[16]["properties"]["kind"]["const"],"production_diagnosis");
    assert_eq!(variants[17]["properties"]["kind"]["const"],"inventory_plan");
    assert!(result["query_schema"]["$defs"]["operations_focus"].is_object());
    assert_eq!(session.calls.load(Ordering::SeqCst),0);
    Ok(())
}

#[test]
fn shared_supply_shortage_is_exact_but_never_an_executable_plan()->Result<()> {
    let _serial=lock(&SERIAL)?;
    let session=full()?;stock(&session,3)?;
    let result=ask(&session,json!({"kind":"inventory_plan","quantity_unit":"stack_units",
        "demands":[{"key":"b","units":2,"item_types":["BAR"]},
            {"key":"a","units":2,"item_types":["BAR"]}]}))?;
    assert_eq!(result["ok"],true);assert_eq!(result["model_feasible"],false);
    assert_eq!(result["summary"]["requested_units"],4);
    assert_eq!(result["summary"]["allocated_units"],3);
    assert_eq!(result["certificate"]["flow_units"],result["certificate"]["cut_capacity"]);
    assert_eq!(result["certificate"]["shortage"]["demand_keys"],json!(["a","b"]));
    assert_eq!(result["certificate"]["shortage"]["deficit"],1);
    assert_eq!(result["reservation_created"],false);assert_eq!(result["commit_compatible"],false);
    assert_eq!(result["coverage"]["game_feasibility"],"unknown");
    assert!(result.get("plan_id").is_none());assert!(result.get("prepare_receipt").is_none());
    assert_eq!(decode(&fortress_commit(session.handle()))?["error"]["code"],"capability_denied");
    assert_eq!(session.calls.load(Ordering::SeqCst),0);
    Ok(())
}

#[test]
fn allocation_pages_fit_8192_bytes_and_keep_active_work_without_skipping_stacks()->Result<()> {
    let _serial=lock(&SERIAL)?;
    let session=register(2048,vec![Capability::Observe,Capability::Query,Capability::Doctor])?;
    stock(&session,40)?;
    let created=ask(&session,json!({"kind":"watch","key":"allocation-review","label":"Wait for review tick",
        "condition":{"op":"tick_at_least","value":105u64*403200+50},
        "deadline_tick":105u64*403200+100,"poll_interval_ticks":1,"stable_observations":1}))?;
    assert_eq!(created["ok"],true);
    let mut query=allocation(40,128);
    let mut ids=Vec::new();let mut pages=0;
    loop {
        let raw=fortress_query(session.handle(),None,Some(json!({"schema":"dfmcp.query/1","query":query})));
        assert!(raw.len()<=8192);let result=decode(&raw)?;
        assert_eq!(result["ok"],true,"{raw}");assert_eq!(result["summary"]["allocated_units"],40);
        assert_eq!(result["agent_turn"]["active_work"]["obligations"].as_array().map(Vec::len),Some(1));
        assert_eq!(result["agent_turn"]["briefing"]["runtime_admitted"],false);
        assert_eq!(result["agent_turn"]["uncertainty"][0]["epistemic_state"],"unknown");
        let rows=result["rows"].as_array().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"allocation rows"))?;
        assert!(!rows.is_empty());
        for row in rows {
            assert_eq!(row["units"],1);
            ids.push(row["item"]["entity_id"].as_str().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"item handle"))?.to_owned());
        }
        pages+=1;assert!(pages<=40);
        if result["continuation"].is_null(){break;}
        assert_eq!(result["agent_turn"]["continuity"]["status"],"partial");
        assert_eq!(result["continuation"],result["agent_turn"]["coverage"]["continuation"]);
        query["continuation"]=result["continuation"].clone();query["limit"]=json!(3);
    }
    assert!(pages>1);
    assert_eq!(ids,(100..140).map(|id|item_entity_id(id).to_string()).collect::<Vec<_>>());
    assert_eq!(session.calls.load(Ordering::SeqCst),0);
    assert_eq!(ask(&session,json!({"kind":"cancel_watch","watch":created["record"]["watch"]}))?["ok"],true);
    assert_eq!(ask(&session,json!({"kind":"release_watch","watch":created["record"]["watch"]}))?["ok"],true);
    Ok(())
}

#[test]
fn continuations_bind_session_query_and_snapshot_but_allow_normalized_type_order()->Result<()> {
    let _serial=lock(&SERIAL)?;
    let session=full()?;stock(&session,3)?;
    let mut initial=allocation(3,1);
    initial["demands"][0]["item_types"]=json!(["BAR","BOULDER"]);
    let first=ask(&session,initial.clone())?;
    assert_eq!(first["ok"],true);assert!(first["continuation"].is_string());
    let mut next=initial;next["continuation"]=first["continuation"].clone();next["limit"]=json!(2);
    next["demands"][0]["item_types"]=json!(["BOULDER","BAR","BAR"]);
    let page=ask(&session,next.clone())?;assert_eq!(page["ok"],true);assert_eq!(page["returned"],2);
    let other=full()?;stock(&other,3)?;
    assert_eq!(ask(&other,next.clone())?["error"]["code"],"stale_anchor");
    let mut changed=next.clone();changed["demands"][0]["units"]=json!(4);
    assert_eq!(ask(&session,changed)?["error"]["code"],"stale_anchor");
    assert_eq!(decode(&fortress_observe(session.handle()))?["ok"],true);
    assert_eq!(ask(&session,next)?["error"]["code"],"stale_anchor");
    assert_eq!(session.calls.load(Ordering::SeqCst),1);
    assert_eq!(other.calls.load(Ordering::SeqCst),0);
    Ok(())
}

#[test]
fn invalid_focus_envelopes_units_and_budgets_fail_before_any_io()->Result<()> {
    let _serial=lock(&SERIAL)?;
    let session=full()?;
    for query in [json!({"kind":"production_diagnosis","job":{"entity_id":"09","generation":1}}),
        json!({"kind":"production_diagnosis","job":{"entity_id":"9","generation":2}}),
        json!({"kind":"production_diagnosis","unexpected":true}),
        json!({"kind":"production_diagnosis","max_work":1}),
        json!({"kind":"production_diagnosis","limit":0}),
        json!({"kind":"production_diagnosis","continuation":format!("op1:0:{}","0".repeat(64))}),
        json!({"kind":"inventory_plan","demands":[{"key":"x","units":1,"item_types":["BAR"]}]}),
        json!({"kind":"inventory_plan","quantity_unit":"mass","demands":[]}),
        json!({"kind":"inventory_plan","quantity_unit":"stack_units","demands":[{"key":"x","units":1,"item_types":["BAR"],"material_index":1}]})] {
        assert_eq!(ask(&session,query)?["ok"],false);
    }
    let raw=fortress_query(session.handle(),None,Some(json!({"schema":"dfmcp.query/1",
        "expected_anchor":{},"query":{"kind":"production_diagnosis"}})));
    assert_eq!(decode(&raw)?["error"]["code"],"stale_anchor");
    assert_eq!(session.calls.load(Ordering::SeqCst),0);
    Ok(())
}

#[test]
fn new_analyses_reject_missing_authority_and_poisoned_sources()->Result<()> {
    let _serial=lock(&SERIAL)?;
    let denied=register(8192,vec![Capability::Doctor])?;
    for query in [json!({"kind":"production_diagnosis"}),allocation(1,1)] {
        let value=ask(&denied,query)?;
        assert_eq!(value["error"]["code"],"capability_denied");assert!(value.get("source_digest").is_none());
    }
    assert_eq!(denied.calls.load(Ordering::SeqCst),0);
    let session=full()?;
    assert_eq!(decode(&fortress_observe(session.handle()))?["ok"],true);
    assert_eq!(decode(&fortress_observe(session.handle()))?["ok"],false);
    for query in [json!({"kind":"production_diagnosis"}),allocation(1,1)] {
        let value=ask(&session,query)?;
        assert_eq!(value["error"]["code"],"adapter_unavailable");
        assert_eq!(value["agent_turn"]["continuity"]["status"],"stale");
    }
    assert_eq!(session.calls.load(Ordering::SeqCst),2);
    Ok(())
}
