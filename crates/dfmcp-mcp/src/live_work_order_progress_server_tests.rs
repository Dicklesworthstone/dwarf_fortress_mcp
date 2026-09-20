use super::*;

fn maximal() -> Result<ProgressObservation> {
    fn u32(out:&mut Vec<u8>,n:u32){out.extend_from_slice(&n.to_be_bytes());}
    fn text(out:&mut Vec<u8>,value:&[u8]){out.extend_from_slice(&(value.len() as u16).to_be_bytes());out.extend_from_slice(value);}
    let mut b=b"DFMWP012".to_vec();for n in [7u64,1,10]{b.extend_from_slice(&n.to_be_bytes());}
    for n in [1u32,32,32]{u32(&mut b,n);}b.push(1);text(&mut b,&vec![1;512]);u32(&mut b,32);
    for id in 0..32u32{
        u32(&mut b,id);b.push(1);u32(&mut b,1);b.push(0);
        for n in [0u32,0,u32::MAX,1,u32::MAX,1,u32::MAX,u32::MAX,4096,4096]{u32(&mut b,n);}
        text(&mut b,&vec![1;128]);text(&mut b,&vec![1;128]);
    }
    ProgressObservation::decode(&b,&(0..32).collect::<Vec<_>>())
}
fn context()->OperationContext{
    OperationContext{session_id:SessionId::new(1),request_id:RequestId::new(1),
        anchor:StateAnchor{fortress_id:FortressId::new(1),cursor:ObservationCursor::ORIGIN,tick:GameTick(0),state_hash:Digest32::ZERO},
        budget:WorkBudget{max_bytes:MAX_BYTES,max_output_tokens:65_536,max_entities:4096,..WorkBudget::default()},
        grants:vec![],cancellation_requested:false}
}
#[test]
fn exact_development_gate_refuses_other_profiles_and_admission(){
    let keys=ENVIRONMENT.iter().map(|s|(*s).to_owned()).collect::<Vec<_>>();
    assert!(environment_contract(Some("1"),&keys,false).is_ok());
    for opt in [None,Some("0"),Some("true"),Some("01"),Some("1 ")]{assert!(environment_contract(opt,&keys,false).is_err());}
    assert!(environment_contract(Some("1"),&keys,true).is_err());
    for key in ["DFMCP_ADMITTED_BRIDGE_PROTOCOL","DFMCP_ADMISSION_TICKET","DFMCP_WORK_ORDERS_ALLOW_PRODUCTION","DFMCP_JOB_CONTROL_TOKEN","DFMCP_ALLOW_UNADMITTED_WORK_ORDER_PROGRESS_V1_11","DFMCP_ALLOW_UNADMITTED_ORDER_PROGRESS_V1_11"]{
        let mut keys=keys.clone();keys.push(key.to_owned());assert!(environment_contract(Some("1"),&keys,false).is_err());
    }
}
#[test]
fn selected_ids_and_profile_session_handles_are_bounded()->Result<()> {
    assert_eq!(normalized_ids(vec![8,3])?,vec![3,8]);
    assert!(normalized_ids(vec![3,3]).is_err());assert!(normalized_ids(vec![0;33]).is_err());
    let id=SessionId::new((1u128<<127)|FAMILY|1);assert_eq!(parse_session(&id.to_string())?,id);
    let other=SessionId::new((1u128<<127)|(11u128<<57)|1);assert!(parse_session(&other.to_string()).is_err());
    assert!(parse_session(&"f".repeat(1024)).is_err());Ok(())
}
#[test]
fn complete_maximal_observation_and_envelope_fit_reservation()->Result<()> {
    let o=maximal()?;let c=context();let comparison=progress::compare(None,&o)?;
    let out=packet("fortress.observe",json!({"ok":true,"observation":observation_json(&o),"comparison":comparison_json(&comparison)}),Some(&c),Some(&o),Some(&comparison));
    assert!(out.len() as u64<=BASE_RESERVE+ROW_RESERVE*32);
    let value:Value=serde_json::from_str(&out).map_err(|_|error(ErrorCode::InvalidRequest,"test JSON"))?;
    assert_eq!(value["result"]["observation"]["rows"].as_array().map(Vec::len),Some(32));
    assert_eq!(value["agent_turn"]["briefing"]["runtime_admitted"],false);
    assert_eq!(value["agent_turn"]["anchor"]["canonical_world_anchor"],false);Ok(())
}
#[test]
fn output_and_work_budget_refusal_precede_any_capture()->Result<()> {
    let c=context();let work=reserve(c.clone(),32)?;assert_eq!(work.budget.max_bytes,c.budget.max_bytes-BASE_RESERVE-ROW_RESERVE*32);
    let mut small=c.clone();small.budget.max_output_tokens=1;assert!(reserve(small,1).is_err());
    let mut small=c;small.budget.max_bytes=BASE_RESERVE+ROW_RESERVE;assert!(reserve(small,1).is_err());
    assert!(reserve(context(),33).is_err());Ok(())
}
#[test]
fn failed_or_absent_capture_never_claims_complete_presence_or_other_work()->Result<()> {
    let out=unbound("fortress.observe",&error(ErrorCode::AdapterUnavailable,"lost response"));
    let value:Value=serde_json::from_str(&out).map_err(|_|error(ErrorCode::InvalidRequest,"test JSON"))?;
    assert_eq!(value["result"]["error"]["game_mutation_dispatched"],false);
    assert_eq!(value["agent_turn"]["continuity"]["status"],"indeterminate");
    assert_eq!(value["agent_turn"]["coverage"]["complete_domains"],json!([]));
    assert!(value["agent_turn"]["anchor"].is_null());
    assert!(value["agent_turn"]["briefing"].get("admission").is_none());Ok(())
}
#[test]
fn inherited_runtime_restrictions_remain_effective()->std::result::Result<(),Box<dyn std::error::Error>> {
    assert!(runtime_io().is_err());
    crate::run_with_runtime_cx(|cx|async move {
        runtime_io()?;
        {
            let _guard=cx.restrict::<fastmcp_rust::asupersync::cx::cap::None>().set_current_restricted();
            assert!(runtime_io().is_err());
        }
        runtime_io()?;
        cx.cancel_with(fastmcp_rust::asupersync::types::CancelKind::User,Some("progress test cancellation"));
        assert!(runtime_io().is_err());
        Ok::<_,DfmcpError>(())
    })??;Ok(())
}
