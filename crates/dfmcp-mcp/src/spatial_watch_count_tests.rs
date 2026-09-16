//! These tests share their parent's real private-journal and injected-capture fixtures.
use super::*;

fn count_condition(kind: &str, comparison: &str, value: u64, predicate: Value) -> Value {
    json!({"op":"entity_count","scope":"observed_projection","kind":kind,
        "predicate":predicate,"comparison":comparison,"value":value})
}
fn register_count(s: &Registered, key: &str, condition: Value) -> Result<Value> {
    let result=ask(s,json!({"kind":"watch","key":key,"condition":condition,
        "deadline_tick":105u64*403200+100,"poll_interval_ticks":1,"stable_observations":2}))?;
    require_success(&result)?;Ok(result)
}
fn without_jobs(tick: u32) -> Result<LiveSpatialCitizenObservation> {
    let original=observation(tick)?;
    let mut operations=original.spatial().operations().clone();
    operations.jobs.jobs.clear();operations.attachments.clear();
    let mut spatial=b"DFMS1600".to_vec();
    part(&mut spatial,&operations.encode_profile(OperationsProfile::PagedV1_4)?);
    part(&mut spatial,&original.spatial().terrain().encode_payload()?);
    let mut citizens=b"DFMC1800".to_vec();citizens.extend_from_slice(&0u32.to_be_bytes());
    let mut combined=b"DFMS1800".to_vec();part(&mut combined,&spatial);part(&mut combined,&citizens);
    LiveSpatialCitizenObservation::decode_payload(&combined,7,"df".into(),"dfhack".into())
}

#[test]
fn population_watch_tracks_a_changing_roster_without_entity_handles() -> Result<()> {
    let _serial=lock(&SERIAL)?;let files=Files::new()?;let s=register(&files,3,&[],false)?;
    assert!(!observation(3)?.spatial().operations().jobs.jobs.is_empty());
    let registered=register_count(&s,"queue-empty",count_condition("job","eq",0,json!({"op":"always"})))?;
    assert_eq!(registered["record"]["status"],"waiting");
    {let handle=resolve(s.handle())?;let mut session=lock(&handle)?;
        session.source=Box::new(Script {values:VecDeque::from([without_jobs(4)?,without_jobs(5)?]),
            calls:s.calls.clone(),fenced:false,corrupt:None});}
    let before=checkpoint(&ask(&s,json!({"kind":"watches"}))?)?;
    for step in 1..=2 {
        let result=ask(&s,json!({"kind":"await_watches"}))?;require_success(&result)?;
        assert_eq!(result["native_captures"],1);assert_eq!(result["sampled"],1);
        assert_eq!(result["all_satisfied"],step==2);assert_eq!(checkpoint(&result)?,before+step);
    }
    let detail=ask(&s,json!({"kind":"poll_watch","watch":registered["record"]["watch"]}))?;
    require_success(&detail)?;
    assert_eq!(detail["record"]["evaluation"]["facts"][0]["matched_min"],0);
    assert_eq!(detail["record"]["evaluation"]["facts"][0]["complete_world_count_proven"],false);
    assert_eq!(s.calls.load(Ordering::SeqCst),2);Ok(())
}

#[test]
fn durable_population_definitions_recover_but_restart_does_not_count_as_a_sample() -> Result<()> {
    let _serial=lock(&SERIAL)?;let files=Files::new()?;let s=register(&files,3,&[],false)?;
    let initial=register_count(&s,"jobs-observed",count_condition("job","ge",0,json!({"op":"always"})))?;
    watch(&s,"legacy-paused",2)?;
    let old_handle=initial["record"]["watch"].clone();drop(s);
    let s=register(&files,4,&[5,6],false)?;
    let list=ask(&s,json!({"kind":"watches"}))?;require_success(&list)?;
    let recovered=list["records"].as_array().and_then(|rows|rows.iter().find(|r|r["key"]=="jobs-observed"))
        .ok_or_else(||error(ErrorCode::InvalidRequest,"recovered count watch absent"))?;
    assert_ne!(recovered["watch"],old_handle);assert_eq!(recovered["stable_observations"],0);
    let detail=ask(&s,json!({"kind":"poll_watch","watch":recovered["watch"]}))?;require_success(&detail)?;
    assert_eq!(detail["record"]["definition"]["condition"]["op"],"entity_count");
    assert_eq!(detail["record"]["fresh_observation_required"],true);
    assert_eq!(detail["record"]["stable_observations"],0);
    for step in 1..=2 {
        let result=ask(&s,json!({"kind":"await_watches"}))?;require_success(&result)?;
        assert_eq!(result["sampled"],2);assert_eq!(result["all_satisfied"],step==2);
    }
    assert_eq!(s.calls.load(Ordering::SeqCst),2);Ok(())
}

#[test]
fn missing_native_fields_do_not_prove_a_negated_population_goal() -> Result<()> {
    let _serial=lock(&SERIAL)?;let files=Files::new()?;let s=register(&files,3,&[4],false)?;
    assert!(!observation(3)?.spatial().operations().jobs.jobs.is_empty());
    let condition=count_condition("job","eq",0,json!({"op":"not","arg":{
        "op":"field","field":"not_observed_by_profile","comparison":"eq","value":{"type":"bool","value":true}}}));
    let registered=register_count(&s,"unknown-readiness",condition)?;
    assert_eq!(registered["record"]["status"],"blocked_unknown");
    let result=ask(&s,json!({"kind":"await_watches"}))?;require_success(&result)?;
    assert_eq!(result["all_satisfied"],false);assert_eq!(result["records"][0]["status"],"blocked_unknown");
    let detail=ask(&s,json!({"kind":"poll_watch","watch":registered["record"]["watch"]}))?;
    assert_eq!(detail["record"]["evaluation"]["facts"][0]["matched_min"],0);
    assert!(detail["record"]["evaluation"]["facts"][0]["matched_max"].as_u64().is_some_and(|n|n>0));
    Ok(())
}

#[test]
fn count_schema_is_discoverable_and_refused_registration_leaves_no_checkpoint() -> Result<()> {
    let _serial=lock(&SERIAL)?;let files=Files::new()?;let s=register(&files,3,&[],false)?;
    let schema=decode(&fortress_query(s.handle(),Some("schema".into()),None))?;require_success(&schema)?;
    let conditions=schema["query_schema"]["$defs"]["watch_condition"]["oneOf"].as_array()
        .ok_or_else(||error(ErrorCode::InvalidRequest,"condition schema absent"))?;
    assert!(conditions.iter().any(|v|v["properties"]["op"]["const"]=="entity_count"));
    let before=fs::read(&files.watches).map_err(io_error)?;
    let mut condition=count_condition("job","ge",1,json!({"op":"always"}));
    condition["scope"]=json!("complete_world");
    assert_eq!(ask(&s,json!({"kind":"watch","key":"bad-scope","condition":condition,
        "deadline_tick":105u64*403200+100}))?["ok"],false);
    {let handle=resolve(s.handle())?;let mut session=lock(&handle)?;session.budget.max_output_tokens=1;}
    assert_eq!(ask(&s,json!({"kind":"watch","key":"cannot-fit","condition":
        count_condition("job","ge",1,json!({"op":"always"})),"deadline_tick":105u64*403200+100}))?["ok"],false);
    assert_eq!(fs::read(&files.watches).map_err(io_error)?,before);
    assert_eq!(s.calls.load(Ordering::SeqCst),0);Ok(())
}
