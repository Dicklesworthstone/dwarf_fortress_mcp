use super::*;
use dfmcp_core::FortressId;
use super::super::tests::Fixture;

fn summary() -> CreationSummary {
    CreationSummary { fortress_id:FortressId::new(7),journal_id:Digest32::of_bytes(b"journal"),
        head:Digest32::of_bytes(b"head"),records:6,prepared:2,unresolved:1,terminal:3,
        transitions:10,retained_bytes:4096,mode:JournalMode::Reconcile }
}

#[test]
fn recipes_digests_and_state_filters_are_closed_and_canonical() -> Result<()> {
    for (text,expected) in [("wooden_bed",WorkOrderRecipe::WoodenBed),
        ("wooden_door",WorkOrderRecipe::WoodenDoor),("wooden_table",WorkOrderRecipe::WoodenTable),
        ("wooden_chair",WorkOrderRecipe::WoodenChair)]
    { assert_eq!(recipe(text)?,expected); }
    for text in ["WoodenBed","1","wooden_bed ","custom_reaction","lua"] { assert!(recipe(text).is_err()); }
    assert_eq!(digest(&Digest32::ZERO.to_string())?,Digest32::ZERO);
    for text in ["0".repeat(63),"0".repeat(65),"A".repeat(64),"é".repeat(32)] { assert!(digest(&text).is_err()); }
    let s = summary();
    assert_eq!(Filter::All.total(&s),6); assert_eq!(Filter::Pending.total(&s),3); assert_eq!(Filter::Unresolved.total(&s),1);
    assert!(!Filter::Pending.matches(CreationState::CancelledBeforeDispatch));
    assert!(Filter::Unresolved.matches(CreationState::DispatchStarted));
    assert!(Filter::parse("not_applied").is_err());
    Ok(())
}

#[test]
fn complete_record_projection_preserves_exact_evidence_without_claiming_production() -> Result<()> {
    let mut fixture = Fixture::new()?; let record = fixture.prepare("evidence")?;
    let value = record_json(&record);
    assert_eq!(value["original_observation"]["order_ids"],json!([2,6]));
    assert_eq!(value["plan_digest"],record.plan().digest().to_string());
    assert_eq!(value["state"],"prepared");
    assert!(value["created_order_id"].is_null()); assert!(value["receipt"].is_null());
    assert_eq!(value["production_goal_completion_proven"],false);
    assert_eq!(value["safe_to_retry_insertion"],false);
    assert!((value.to_string().len() as u64) < RECORD_RESERVE);
    Ok(())
}

#[test]
fn issued_cursor_binds_every_identity_and_retains_bounded_replay_history() -> Result<()> {
    let s = summary(); let session = SessionId::new(7); let mut cursors = Continuations::default();
    let token = cursors.issue(session,&s,"key".into(),Filter::Pending,2)?;
    assert_eq!(cursors.resolve(&token,session,&s,Filter::Pending,2)?,"key");
    assert_eq!(cursors.resolve(&token,session,&s,Filter::Pending,2)?,"key");
    let mut other = s.clone(); other.head = Digest32::ZERO;
    assert!(cursors.resolve(&token,session,&other,Filter::Pending,2).is_err());
    other = s.clone(); other.journal_id = Digest32::ZERO;
    assert!(cursors.resolve(&token,session,&other,Filter::Pending,2).is_err());
    assert!(cursors.resolve(&token,SessionId::new(8),&s,Filter::Pending,2).is_err());
    assert!(cursors.resolve(&token,session,&s,Filter::All,2).is_err());
    assert!(cursors.resolve(&token,session,&s,Filter::Pending,1).is_err());
    assert!(cursors.resolve(&Digest32::ZERO.to_string(),session,&s,Filter::Pending,2).is_err());
    for n in 0..64 { cursors.issue(session,&s,format!("key-{n}"),Filter::All,2)?; }
    assert_eq!(cursors.issued.len(),64);
    assert!(cursors.resolve(&token,session,&s,Filter::Pending,2).is_err());
    cursors.serial = u64::MAX;
    assert!(cursors.issue(session,&s,"end".into(),Filter::All,2).is_err());
    Ok(())
}

#[test]
fn common_turn_exposes_unfinished_work_and_never_inherits_admission_or_empty_world_claims() -> Result<()> {
    let s = summary();
    let text = packet("fortress.query",json!({"ok":true}),TurnView {
        context:None,mode:Some(JournalMode::Reconcile),summary:Some(&s),selected:None });
    let value:Value = serde_json::from_str(&text).map_err(|_|error(ErrorCode::InvalidRequest,"JSON"))?;
    assert_eq!(value["agent_turn"]["schema"],"dfmcp.agent_turn/1");
    assert_eq!(value["agent_turn"]["continuity"]["status"],"indeterminate");
    assert_eq!(value["agent_turn"]["active_work"]["prepared_count"],2);
    assert_eq!(value["agent_turn"]["active_work"]["unresolved_count"],1);
    assert_eq!(value["agent_turn"]["affordances"][0]["enabled"],false);
    assert!(value["agent_turn"]["anchor"].is_null());
    assert!(value["agent_turn"]["briefing"].get("admission").is_none());
    assert_eq!(value["agent_turn"]["coverage"]["complete_domains"],json!([]));
    assert!((text.len() as u64) < BASE_RESERVE);
    Ok(())
}

#[test]
fn unknown_custody_errors_retain_uncertainty_instead_of_proving_no_effect() -> Result<()> {
    for code in [ErrorCode::CorruptLedger,ErrorCode::EffectIndeterminate] {
        let cause = error(code,"recovery required");
        let result = failure(&cause,"fortress.commit");
        assert_ne!(result["error"]["effect_may_have_occurred"],json!(false));
        let text = packet("fortress.commit",result,TurnView { context:None,mode:None,summary:None,selected:None });
        let value:Value = serde_json::from_str(&text).map_err(|_|error(ErrorCode::InvalidRequest,"JSON"))?;
        assert_eq!(value["agent_turn"]["active_work"]["state_known"],false);
        assert_eq!(value["agent_turn"]["affordances"][0]["enabled"],false);
        assert!((text.len() as u64) < BASE_RESERVE);
    }
    Ok(())
}
