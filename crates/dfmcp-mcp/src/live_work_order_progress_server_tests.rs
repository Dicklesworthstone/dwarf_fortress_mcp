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

#[cfg(unix)]
mod archive_integration {
    use super::*;
    use std::os::unix::fs::DirBuilderExt;
    use dfmcp_adapter::work_order_progress::ProgressManifest;
    static PATH_SEQUENCE: AtomicU64 = AtomicU64::new(1);
    struct Fixture { root:PathBuf, context:OperationContext, entries:Vec<ProgressArchiveEntry> }
    impl Drop for Fixture {fn drop(&mut self){let _=std::fs::remove_dir_all(&self.root);}}
    impl Fixture {
        fn new() -> Result<Self> {
            let root=std::env::temp_dir().canonicalize().map_err(|_|error(ErrorCode::InvalidRequest,"test root"))?
                .join(format!("dfmcp-progress-mcp-{}-{}",std::process::id(),PATH_SEQUENCE.fetch_add(1,Ordering::Relaxed)));
            let mut builder=std::fs::DirBuilder::new();builder.mode(0o700);builder.create(&root)
                .map_err(|_|error(ErrorCode::InvalidRequest,"test directory"))?;
            let capture=maximal()?;let mut c=context();c.anchor.fortress_id=capture.fortress_id();
            c.grants=session_grants(c.anchor.fortress_id,false);
            let mut f=Self{root,context:c,entries:Vec::new()};
            let mut a=open_progress_archive(&f.root.join("history.bin"),ArchiveMode::Live,&f.context)?;
            let manifest=ProgressManifest{generation:7,df_version:"df".into(),dfhack_version:"dfhack".into()};
            for sequence in 1..=2u64 {
                let mut bytes=capture.canonical_bytes().to_vec();bytes[16..24].copy_from_slice(&sequence.to_be_bytes());
                bytes[24..32].copy_from_slice(&(10*sequence).to_be_bytes());
                let capture=ProgressObservation::decode(&bytes,&capture.ids())?;
                f.entries.push(a.append(&manifest,&capture,&f.context)?);
            }
            drop(a);Ok(f)
        }
        fn session(&self)->Result<RuntimeSession>{
            let mut c=self.context.clone();c.grants=session_grants(c.anchor.fortress_id,true);
            let a=open_progress_archive(&self.root.join("history.bin"),ArchiveMode::Offline,&c)?;
            offline_session(a,&c,c.budget)
        }
    }
    #[test]
    fn offline_bootstrap_has_no_source_and_retains_only_query_authority()->Result<()> {
        let f=Fixture::new()?;let mut s=f.session()?;
        assert!(s.reader.is_none());assert!(s.offline);assert_eq!(s.grants.len(),1);assert_eq!(s.grants[0].capability,Capability::Query);
        let c=s.context()?;let view=selected_result(&mut s,&c,false)?;
        assert!(view.historical);assert_eq!(view.capture.as_ref().map(ProgressObservation::tick),Some(20));
        assert_eq!(view.value["source_read_this_call"],false);
        let out=render_checked("fortress.query",view,&mut s,&c)?;
        let v:Value=serde_json::from_str(&out).map_err(|_|error(ErrorCode::InvalidRequest,"test JSON"))?;
        assert_eq!(v["agent_turn"]["continuity"]["status"],"stale");assert_eq!(v["agent_turn"]["anchor"]["kind"],"historical_native_order_progress");
        let before=std::fs::read(f.root.join("history.bin")).map_err(|_|error(ErrorCode::InvalidRequest,"test read"))?;
        let mut injected=c.clone();injected.grants=session_grants(c.anchor.fortress_id,false);
        assert!(refresh(&mut s,&[3,8],&injected).is_err());assert!(s.reader.is_none());
        assert_eq!(std::fs::read(f.root.join("history.bin")).map_err(|_|error(ErrorCode::InvalidRequest,"test read"))?,before);Ok(())
    }
    #[test]
    fn historical_navigation_does_not_replace_latest_selection_or_authority_tick()->Result<()> {
        let f=Fixture::new()?;let mut s=f.session()?;let c=s.context()?;
        let anchor=s.anchor;let ids=s.ids.clone();
        let archive=s.archive.as_mut().ok_or_else(||error(ErrorCode::InvalidRequest,"test archive"))?;
        let summary=archive.summary(&c)?;
        let answer=history::query(archive,&mut s.cursors,history::Request::Record{archive_id:summary.archive_id.to_string(),
            number:f.entries[0].number,record_digest:f.entries[0].record_digest.to_string()},&c)?;
        let projection=Projection{value:answer.value,capture:answer.capture,comparison:answer.comparison,historical:true};
        let out=render_checked("fortress.query",projection,&mut s,&c)?;
        let v:Value=serde_json::from_str(&out).map_err(|_|error(ErrorCode::InvalidRequest,"test JSON"))?;
        assert_eq!(v["agent_turn"]["anchor"]["game_tick"],10);assert_eq!(s.anchor,anchor);assert_eq!(s.ids,ids);
        assert_eq!(selected_result(&mut s,&c,false)?.capture.as_ref().map(ProgressObservation::tick),Some(20));
        let mut expired=c.clone();expired.anchor.tick=GameTick(0);
        for grant in &mut expired.grants{grant.expires_at_tick=Some(GameTick(19));}
        assert!(selected_result(&mut s,&expired,false).is_err());Ok(())
    }
    #[test]
    fn custody_loss_suppresses_even_a_previously_built_capture_and_blocks_bootstrap_publication()->Result<()> {
        let f=Fixture::new()?;let mut s=f.session()?;let c=s.context()?;
        let projection=selected_result(&mut s,&c,false)?;
        std::fs::rename(f.root.join("history.bin"),f.root.join("moved.bin")).map_err(|_|error(ErrorCode::InvalidRequest,"test rename"))?;
        let mut target=None;
        assert!(publish_session(s,projection,&c,Instant::now(),&mut target).is_err());assert!(target.is_none());Ok(())
    }
    #[test]
    fn output_and_expired_acknowledgement_do_not_publish_an_offline_session()->Result<()> {
        let f=Fixture::new()?;let mut s=f.session()?;let c=s.context()?;
        let projection=selected_result(&mut s,&c,false)?;let mut small=c.clone();small.budget.max_output_tokens=1;
        let mut target=None;assert!(publish_session(s,projection,&small,Instant::now(),&mut target).is_err());assert!(target.is_none());
        let mut s=f.session()?;let c=s.context()?;let projection=selected_result(&mut s,&c,false)?;
        let before=Instant::now().checked_sub(Duration::from_secs(61)).ok_or_else(||error(ErrorCode::InvalidRequest,"test clock"))?;
        assert!(publish_session(s,projection,&c,before,&mut target).is_err());assert!(target.is_none());Ok(())
    }
    #[test]
    fn maximal_history_record_and_metadata_page_fit_complete_response_reservation()->Result<()> {
        let f=Fixture::new()?;let mut s=f.session()?;let c=s.context()?;
        let archive=s.archive.as_mut().ok_or_else(||error(ErrorCode::InvalidRequest,"test archive"))?;let summary=archive.summary(&c)?;
        let answer=history::query(archive,&mut s.cursors,history::Request::Changes{archive_id:summary.archive_id.to_string(),
            before_number:1,before_digest:f.entries[0].record_digest.to_string(),after_number:2,after_digest:f.entries[1].record_digest.to_string()},&c)?;
        let projection=Projection{value:answer.value,capture:answer.capture,comparison:answer.comparison,historical:true};
        let out=render_checked("fortress.query",projection,&mut s,&c)?;assert!(out.len() as u64<=BASE_RESERVE+ROW_RESERVE*32);
        let row=history::entry_json(&f.entries[0],summary.archive_id);
        let page=Projection::plain(json!({"ok":true,"historical":true,"entries":vec![row;64],"continuation":"f".repeat(64)}),true);
        let out=render_checked("fortress.query",page,&mut s,&c)?;assert!(out.len() as u64<=BASE_RESERVE+ROW_RESERVE*32);Ok(())
    }
    #[test]
    fn releasing_offline_session_unlocks_custody_without_erasing_evidence()->Result<()> {
        let f=Fixture::new()?;let s=f.session()?;let before=std::fs::read(f.root.join("history.bin"))
            .map_err(|_|error(ErrorCode::InvalidRequest,"test read"))?;
        assert!(f.session().is_err());drop(s);
        let mut reopened=f.session()?;let c=reopened.context()?;
        assert!(selected_result(&mut reopened,&c,false)?.capture.is_some());
        assert_eq!(std::fs::read(f.root.join("history.bin")).map_err(|_|error(ErrorCode::InvalidRequest,"test read"))?,before);Ok(())
    }
}
