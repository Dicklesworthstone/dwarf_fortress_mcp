//! Closed query routing, original custody, publication and live-state isolation.

use super::*;
use super::super as server;
use crate::agent_facade as facade;
use dfmcp_core::{CommitState, SessionId};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Owner(String);
impl Drop for Owner {
    fn drop(&mut self) {
        if let Ok(id)=u128::from_str_radix(&self.0,16) {
            server::sessions().remove(&SessionId::new(id));
        }
    }
}

fn parsed(raw: &str) -> TestResult<Value> { Ok(serde_json::from_str(raw)?) }
fn field(value: &Value,key: &str) -> TestResult<String> {
    Ok(value[key].as_str().ok_or_else(||format!("missing {key}: {value}"))?.to_owned())
}

fn open(selector: &str, scenario: &str, paused: bool) -> TestResult<Owner> {
    let opened=parsed(&facade::fortress_open_session(
        Some(paused),Some(selector.to_owned()),
        Some([("observe","read_only"),("query","read_only"),("plan","reversible"),
            ("configure_production","reversible"),("configure_labor","reversible"),
            ("designate","guarded"),("checkpoint","guarded"),
            ("control_clock","reversible")].into_iter()
            .map(|(capability,risk)|(capability.to_owned(),risk.to_owned())).collect()),
        Some(60_000),Some(4_000),Some(32_768),Some(4*1024*1024),Some(32_768),
        Some(64),Some(scenario.to_owned()),Some(false),Some(false),
    ))?;
    assert_eq!(opened["ok"],true,"{opened}");
    Ok(Owner(field(&opened,"session_id")?))
}

fn view<T>(owner: &Owner, read: impl FnOnce(&LabSession)->TestResult<T>) -> TestResult<T> {
    let handle=server::resolve_session(Some(owner.0.clone()))?;
    server::with_session_read(&handle,|error|Err(error.into()),read)
}
fn edit<T>(owner: &Owner, write: impl FnOnce(&mut LabSession)->TestResult<T>) -> TestResult<T> {
    let handle=server::resolve_session(Some(owner.0.clone()))?;
    server::with_session(&handle,||Err("fixture session unavailable".into()),write)
}

fn plan(owner: &Owner, actions: Option<Value>, blueprint: Option<Value>) -> TestResult<String> {
    let result=parsed(&facade::fortress_plan(
        Some(owner.0.clone()),None,None,actions.map(|value|value.to_string()),
        blueprint.map(|value|value.to_string()),
    ))?;
    assert_eq!(result["ok"],true,"{result}");
    field(&result,"plan_digest")
}

fn commit(owner: &Owner, actions: Option<Value>, blueprint: Option<Value>) -> TestResult<String> {
    let digest=plan(owner,actions,blueprint)?;
    let result=parsed(&facade::fortress_commit(Some(owner.0.clone()),digest.clone()))?;
    assert_eq!(result["ok"],true,"{result}");
    field(&result,"plan_digest")
}

fn order(name: &str,amount: u32) -> Value {
    json!([{"action":{"kind":"create_work_order","name":name,"job_token":"MAKE_BED","amount":amount}}])
}

fn query(owner: &Owner,digests: &[String],ticks: u64) -> TestResult<Value> {
    parsed(&facade::fortress_query(Some(owner.0.clone()),Some(json!({
        "mode":"forecast_originals","plan_digests":digests,"horizon_ticks":ticks,
    }).to_string())))
}

#[test]
fn closed_original_forecast_grammar_rejects_ambiguous_or_unbounded_requests() {
    let digest="1".repeat(64);
    let valid=json!({"mode":"forecast_originals","plan_digests":[digest],"horizon_ticks":100}).to_string();
    assert!(matches!(parse(&valid),Ok(Some(_))));
    for raw in [
        r#"{"mode":"forecast_originals","plan_digests":[],"horizon_ticks":100}"#.to_owned(),
        r#"{"mode":"forecast_originals","plan_digests":null,"horizon_ticks":100}"#.to_owned(),
        r#"{"mode":"forecast_originals","plan_digests":["ABC"],"horizon_ticks":100}"#.to_owned(),
        format!(r#"{{"mode":"forecast_originals","mode":"history","plan_digests":["{digest}"],"horizon_ticks":100}}"#),
        format!(r#"{{"mode":"forecast_originals","plan_digests":["{digest}"],"horizon_ticks":100,"horizon_ticks":200}}"#),
        format!(r#"{{"mode":"forecast_originals","plan_digests":["{digest}"],"horizon_ticks":100,"at":"{digest}"}}"#),
        json!({"mode":"forecast_originals","plan_digests":vec![digest.clone();9],"horizon_ticks":100}).to_string(),
        json!({"mode":"forecast_originals","plan_digests":[digest.clone(),digest],"horizon_ticks":100}).to_string(),
    ] {
        assert!(parse(&raw).is_err(),"{raw}");
    }
    assert!(matches!(parse("summary"),Ok(None)));
    assert!(matches!(parse(r#"{"mode":"entities","limit":3}"#),Ok(None)));
}

#[test]
fn public_joint_forecast_preserves_pending_review_receipts_world_and_history() -> TestResult {
    let _serial=crate::test_serial();
    let owner=open("8161101","starter_fortress_provisions",false)?;
    let first=commit(&owner,Some(order("first pair",2)),None)?;
    let second=commit(&owner,Some(order("second pair",2)),None)?;
    let pending=plan(&owner,Some(order("keep this pending",1)),None)?;
    let before=view(&owner,|session|Ok((
        format!("{:?}",session.adapter),session.pending.as_ref().map(|pending|pending.plan.digest),
        session.commit_receipts.clone(),session.open_actions.clone(),
        session.history.len(),session.objectives.iter().map(|goal|goal.selected_wait_reserve_observation).collect::<Vec<_>>(),
    )))?;
    let result=query(&owner,&[second.clone(),first.clone()],110)?;
    assert_eq!(result["ok"],true,"{result}");
    assert_eq!(result["schema"],"dfmcp.lab-original-forecast/1");
    assert_eq!(result["prediction"]["all_complete"],false);
    assert_eq!(result["prediction"]["forecast_work"]["actual_game_ticks_advanced"],0);
    assert_eq!(result["agent_turn"]["anchor"],result["anchor"]);
    assert_eq!(result["agent_turn"]["continuity"]["status"],"partial");
    assert_eq!(result["agent_turn"]["coverage"]["eligible_as_mutation_evidence"],false);
    let actuals=result["prediction"]["originals"].as_array().ok_or("originals missing")?
        .iter().map(|row|field(row,"actual_plan_digest")).collect::<TestResult<Vec<_>>>()?;
    let mut expected=vec![first,second];expected.sort();assert_eq!(actuals,expected);
    let after=view(&owner,|session|Ok((
        format!("{:?}",session.adapter),session.pending.as_ref().map(|pending|pending.plan.digest),
        session.commit_receipts.clone(),session.open_actions.clone(),
        session.history.len(),session.objectives.iter().map(|goal|goal.selected_wait_reserve_observation).collect::<Vec<_>>(),
    )))?;
    assert_eq!(after,before);
    assert_eq!(after.1.map(|digest|digest.to_hex()),Some(pending));
    assert!(result["agent_turn"]["active_work"].is_object() || result["agent_turn"]["active_work"].is_array());
    Ok(())
}

#[test]
fn selected_original_aliases_preserve_request_identity_and_cannot_double_count() -> TestResult {
    let _serial=crate::test_serial();
    let owner=open("8161102","starter_fortress_provisions",false)?;
    let digest=commit(&owner,Some(order("alias original",1)),None)?;
    let alias=Digest32::of_bytes(b"forecast-requested-alias").to_hex();
    edit(&owner,|session| {
        let receipt=session.commit_receipts.get(&digest).ok_or("receipt missing")?.clone();
        session.commit_receipts.insert(alias.clone(),receipt);
        Ok(())
    })?;
    let result=query(&owner,std::slice::from_ref(&alias),100)?;
    assert_eq!(result["ok"],true,"{result}");
    assert_eq!(result["prediction"]["originals"][0]["requested_plan_digest"],alias);
    assert_eq!(result["prediction"]["originals"][0]["actual_plan_digest"],digest);
    let duplicate=query(&owner,&[alias,digest],100)?;
    assert_eq!(duplicate["ok"],false,"{duplicate}");
    assert_eq!(duplicate["error"]["code"],"invalid_request");
    Ok(())
}

#[test]
fn foreign_pending_and_missing_live_commit_custody_cannot_be_forecast() -> TestResult {
    let _serial=crate::test_serial();
    let owner=open("8161103","starter_fortress_provisions",false)?;
    let foreign=open("8161103","starter_fortress_provisions",false)?;
    let digest=commit(&owner,Some(order("owned",1)),None)?;
    assert_eq!(query(&foreign,std::slice::from_ref(&digest),100)?["ok"],false);
    let pending=plan(&owner,Some(order("pending",1)),None)?;
    assert_eq!(query(&owner,&[pending],100)?["ok"],false);
    let before=view(&owner,|session|Ok(format!("{:?}",session.adapter)))?;
    edit(&owner,|session| {
        session.commit_receipts.clear();
        Ok(())
    })?;
    assert_eq!(query(&owner,&[digest],100)?["ok"],false);
    assert_eq!(view(&owner,|session|Ok(format!("{:?}",session.adapter)))?,before);
    Ok(())
}

#[test]
fn current_plan_authority_and_complete_source_and_output_budgets_are_required() -> TestResult {
    let _serial=crate::test_serial();
    let owner=open("8161104","starter_fortress_provisions",false)?;
    let digest=commit(&owner,Some(order("admission",1)),None)?;
    let grants=edit(&owner,|session| {
        let grants=session.grants.clone();
        session.grants.retain(|grant|grant.capability!=Capability::Plan);
        Ok(grants)
    })?;
    let refused=query(&owner,std::slice::from_ref(&digest),100)?;
    assert_eq!(refused["ok"],false,"{refused}");
    assert_eq!(refused["error"]["code"],"capability_denied");
    edit(&owner,|session| {session.grants=grants;session.budget.max_entities=1;Ok(())})?;
    assert_eq!(query(&owner,std::slice::from_ref(&digest),100)?["error"]["code"],"budget_exceeded");
    edit(&owner,|session| {session.budget.max_entities=32_768;session.budget.max_output_tokens=600;Ok(())})?;
    let bounded=query(&owner,&[digest],100)?;
    assert_eq!(bounded["ok"],false,"{bounded}");
    assert!(bounded["prediction"].is_null(),"truncated predictions must never survive as successful forecasts");
    Ok(())
}

#[test]
fn resource_acquisition_success_does_not_complete_the_original_production_goal() -> TestResult {
    let _serial=crate::test_serial();
    let owner=open("8161105","starter_fortress_resources",false)?;
    let digest=commit(&owner,None,Some(json!({
        "template":"production","quotas":[{"item":"BED","minimum":24}],
        "acquire_resources":{"assign_labor":false},
    })))?;
    let result=query(&owner,&[digest],4_000)?;
    assert_eq!(result["ok"],true,"{result}");
    let original=&result["prediction"]["originals"][0];
    assert_eq!(original["predicted_actions_complete"],true,"{result}");
    assert_eq!(original["predicted_stage_truth"],"false");
    assert_eq!(original["predicted_goal_truth"],"false");
    assert_eq!(original["predicted_outcome"],"goal_unmet");
    assert_eq!(result["prediction"]["all_complete"],false);
    assert_eq!(view(&owner,|session|Ok(session.adapter.snapshot().tick.0))?,1);
    Ok(())
}

#[test]
fn exact_future_reserves_are_predicted_without_becoming_retained_endpoint_evidence() -> TestResult {
    let _serial=crate::test_serial();
    let owner=open("8161106","starter_fortress",false)?;
    let first=commit(&owner,None,Some(json!({
        "template":"production","quotas":[{"item":"DRINK","minimum":40}],
        "reserve_until_tick":1_234,
    })))?;
    let second=commit(&owner,None,Some(json!({
        "template":"production","quotas":[{"item":"FOOD","minimum":65}],
        "reserve_until_tick":1_511,
    })))?;
    let before=view(&owner,|session|Ok(format!("{:?}",session.adapter)))?;
    let result=query(&owner,&[first,second],2_000)?;
    assert_eq!(result["ok"],true,"{result}");
    assert_eq!(result["prediction"]["all_complete"],true,"{result}");
    assert_eq!(result["prediction"]["predicted_frontier_tick"],1_511);
    assert!(view(&owner,|session|Ok(session.objectives.iter().all(|goal|goal.selected_wait_reserve_observation.is_none())))?);
    assert_eq!(view(&owner,|session|Ok(format!("{:?}",session.adapter)))?,before);
    assert!(view(&owner,|session|Ok(session.open_actions.iter().any(|id|
        session.adapter.action_receipt(*id).is_some_and(|receipt|receipt.state!=CommitState::Verified)
    )))?);
    Ok(())
}


fn complete_query(owner: &Owner,digests: &[String],ticks:u64) -> TestResult<Value> {
    parsed(&facade::fortress_query(Some(owner.0.clone()),Some(json!({
        "mode":"forecast_originals","plan_digests":digests,"horizon_ticks":ticks,
        "forecast_scope":"complete_project",
    }).to_string())))
}

#[test]
fn complete_project_scope_is_closed_and_default_compatible() {
    let digest="1".repeat(64);
    for scope in ["current_stage","complete_project"] {
        assert!(matches!(parse(&json!({"mode":"forecast_originals","plan_digests":[digest],
            "horizon_ticks":100,"forecast_scope":scope}).to_string()),Ok(Some(_))));
    }
    for scope in [Value::Null,json!(true),json!({}),json!("all"),json!("complete")] {
        assert!(parse(&json!({"mode":"forecast_originals","plan_digests":[digest],
            "horizon_ticks":100,"forecast_scope":scope}).to_string()).is_err());
    }
    let duplicate=format!(r#"{{"mode":"forecast_originals","plan_digests":["{digest}"],"horizon_ticks":100,"forecast_scope":"current_stage","forecast_scope":"complete_project"}}"#);
    assert!(parse(&duplicate).is_err());
}

#[test]
fn public_complete_project_predicts_harvest_then_output_without_publishing_any_stage() -> TestResult {
    let _serial=crate::test_serial();
    let owner=open("8161201","starter_fortress_resources",false)?;
    let digest=commit(&owner,None,Some(json!({
        "template":"production","quotas":[{"item":"BED","minimum":24}],
        "acquire_resources":{"assign_labor":false},
    })))?;
    let pending=plan(&owner,Some(order("unchanged pending review",1)),None)?;
    let before=view(&owner,|session|Ok((format!("{:?}",session.adapter),
        session.commit_receipts.clone(),session.open_actions.clone(),session.history.len(),
        session.objectives.iter().map(|goal|goal.selected_wait_reserve_observation).collect::<Vec<_>>())))?;
    let default=query(&owner,std::slice::from_ref(&digest),4_000)?;
    assert_eq!(default["schema"],"dfmcp.lab-original-forecast/1");
    assert_eq!(default["prediction"]["originals"][0]["predicted_outcome"],"goal_unmet");
    let result=complete_query(&owner,std::slice::from_ref(&digest),4_000)?;
    assert_eq!(result["ok"],true,"{result}");
    assert_eq!(result["schema"],"dfmcp.lab-original-forecast/2");
    assert_eq!(result["prediction"]["all_complete"],true,"{result}");
    let original=&result["prediction"]["originals"][0];
    assert_eq!(original["actual_plan_digest"],digest);
    assert_eq!(original["predicted_actions_complete"],true);
    assert_eq!(original["predicted_project_actions_complete"],true);
    assert_eq!(original["future_stages"].as_array().map(Vec::len),Some(1));
    assert_eq!(original["future_stages"][0]["executable_review"],false);
    assert_eq!(original["future_stage_plans_are_executable"],false);
    assert_eq!(result["agent_turn"]["anchor"],result["anchor"]);
    assert_eq!(result["agent_turn"]["coverage"]["eligible_as_mutation_evidence"],false);
    assert_eq!(view(&owner,|session|Ok((format!("{:?}",session.adapter),
        session.commit_receipts.clone(),session.open_actions.clone(),session.history.len(),
        session.objectives.iter().map(|goal|goal.selected_wait_reserve_observation).collect::<Vec<_>>())))?,before);
    assert_eq!(view(&owner,|session|Ok(session.pending.as_ref().map(|value|value.plan.digest.to_hex())))?,Some(pending));
    Ok(())
}

#[test]
fn complete_project_rejects_unsupported_originals_missing_sources_and_clipped_output() -> TestResult {
    let _serial=crate::test_serial();
    let owner=open("8161202","starter_fortress_resources",false)?;
    let raw=commit(&owner,Some(order("ordinary raw order",1)),None)?;
    assert_eq!(complete_query(&owner,&[raw],4_000)?["error"]["code"],"invalid_request");
    let digest=commit(&owner,None,Some(json!({
        "template":"production","quotas":[{"item":"BED","minimum":24}],
        "acquire_resources":{"assign_labor":false},
    })))?;
    let before=view(&owner,|session|Ok(format!("{:?}",session.adapter)))?;
    edit(&owner,|session|{session.budget.max_output_tokens=600;Ok(())})?;
    let clipped=complete_query(&owner,std::slice::from_ref(&digest),4_000)?;
    assert_eq!(clipped["ok"],false,"{clipped}");
    assert!(clipped["prediction"].is_null());
    edit(&owner,|session|{
        session.budget.max_output_tokens=32_768;
        let plan=session.objectives.iter().find(|goal|goal.plan_digest.to_hex()==digest)
            .and_then(|goal|goal.resource_plan()).ok_or("original plan missing")?;
        let source=plan.anchor.state_hash;
        session.history.remove(&source);
        Ok(())
    })?;
    let missing=complete_query(&owner,&[digest],4_000)?;
    assert_eq!(missing["ok"],false,"{missing}");
    assert_eq!(view(&owner,|session|Ok(format!("{:?}",session.adapter)))?,before);
    Ok(())
}
