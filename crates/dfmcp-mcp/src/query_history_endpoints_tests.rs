use super::*;
use dfmcp_core::{CapabilityGrant, CapabilityScope, EntityId, FortressId, GameTick,
    ObservationCursor, RequestId, WorkBudget};
use dfmcp_world::{EntityKind, EntityRecord, Fact, FactPresence, FactSource,
    Value as WorldValue, WorldGraph};

fn world(tick: u64, sequence: u64, values: &[(u64, u32, i64)]) -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    for &(id, generation, value) in values {
        let fact = Fact::known(WorldValue::I64(value), GameTick(tick),
            FactSource::DfhackField("unit.test_value".into()), Digest32::of_bytes(&tick.to_be_bytes()));
        graph.entities.insert(EntityId::new(id), EntityRecord { id:EntityId::new(id), generation,
            revision:sequence + 1, kind:EntityKind::Unit, label:format!("unit-{id}"),
            fields:BTreeMap::from([("value".into(), fact)]) });
    }
    WorldSnapshot::new(FortressId::new(1), GameTick(tick),
        ObservationCursor { epoch:0, sequence }, true, graph)
}
fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext { session_id:SessionId::new(781), request_id:RequestId::new(1), anchor:snapshot.anchor(),
        budget:WorkBudget { max_entities:1000, max_bytes:262144, max_output_tokens:65536,
            max_wall_millis:60000, ..WorkBudget::default() },
        grants:vec![CapabilityGrant { capability:Capability::Query,
            scope:CapabilityScope { fortress_id:Some(snapshot.fortress_id), ..CapabilityScope::default() },
            max_risk:RiskTier::ReadOnly, expires_at_tick:None, remaining_uses:None }],
        cancellation_requested:false }
}
fn selection() -> Value { json!({"kind":"entities","kinds":["unit"],"fields":["value"]}) }
fn compare(before: &WorldSnapshot, after: &WorldSnapshot) -> Result<Value> {
    compare_endpoints(before, after, &context(after), &selection(), Digest32::of_bytes(b"archive-pair"), None, None)
}

#[test]
fn additions_departures_changes_and_recycled_ids_are_separate_ordered_records() -> Result<()> {
    let before = world(1,0,&[(1,1,10),(2,1,20),(4,1,40)]);
    let after = world(2,1,&[(1,1,11),(3,1,30),(4,2,40)]);
    let result = compare(&before,&after)?;
    assert_eq!(result["change_count"],5);
    assert_eq!(result["change_counts"],json!({"changed_in_result":1,"entered_result":2,"left_result":2}));
    let rows = result["changes"].as_array().ok_or_else(||invalid("test changes absent"))?;
    assert_eq!(rows.iter().map(|r|(r["entity_id"].as_str(),r["generation"].as_u64(),r["kind"].as_str())).collect::<Vec<_>>(),
        vec![(Some("1"),Some(1),Some("changed_in_result")),(Some("2"),Some(1),Some("left_result")),
            (Some("3"),Some(1),Some("entered_result")),(Some("4"),Some(1),Some("left_result")),
            (Some("4"),Some(2),Some("entered_result"))]);
    assert_eq!(result["baseline_created"],false);
    assert_eq!(result["coverage"]["intermediate_observations_evaluated"],false);
    Ok(())
}

#[test]
fn observation_bookkeeping_is_disclosed_without_manufacturing_value_changes() -> Result<()> {
    let before=world(1,0,&[(1,1,7),(2,1,8)]);
    let after=world(2,1,&[(1,1,7),(2,1,8)]);
    let result=compare(&before,&after)?;
    assert_eq!(result["change_count"],0);assert_eq!(result["provenance_only_refreshes"],2);
    assert_eq!(result["anchor_advanced"],true);assert_eq!(result["continuation"],Value::Null);
    let same=compare(&before,&before)?;
    assert_eq!(same["change_count"],0);assert_eq!(same["provenance_only_refreshes"],0);
    assert_eq!(same["basis_result_digest"],same["target_result_digest"]);
    Ok(())
}

#[test]
fn changed_presence_or_source_is_not_hidden_by_equal_underlying_values() -> Result<()> {
    let before=world(1,0,&[(1,1,7)]);
    for case in 0..5 {
        let mut after=world(2,1,&[(1,1,7)]);
        let fact=after.graph.entities.get_mut(&EntityId::new(1)).and_then(|e|e.fields.get_mut("value"))
            .ok_or_else(||invalid("test fact absent"))?;
        match case {
            0=>fact.presence=Some(FactPresence::Unknown("not captured".into())),
            1=>fact.presence=Some(FactPresence::Absent),
            2=>fact.presence=Some(FactPresence::Omitted("budget".into())),
            3=>fact.presence=Some(FactPresence::Redacted("hidden".into())),
            _=>fact.source=FactSource::Derived("model".into()),
        }
        after.refresh_hash();
        assert_eq!(compare(&before,&after)?["change_count"],1,"case={case}");
    }
    Ok(())
}

#[test]
fn leaving_a_filtered_selection_is_not_labeled_deletion() -> Result<()> {
    let before=world(1,0,&[(1,1,10),(2,1,0)]);
    let after=world(2,1,&[(1,1,0),(2,1,10)]);
    let mut select=selection();
    select["where"]=json!({"op":"compare","field":"value","comparison":"gt","value":{"type":"i64","value":5}});
    let result=compare_endpoints(&before,&after,&context(&after),&select,Digest32::ZERO,None,None)?;
    assert_eq!(result["matched_before"],1);assert_eq!(result["matched_after"],1);
    assert_eq!(result["changes"][0]["kind"],"left_result");
    assert_eq!(result["changes"][1]["kind"],"entered_result");
    assert_eq!(result["coverage"]["absence_proven"],false);
    Ok(())
}

#[test]
fn epoch_changes_regressions_and_same_cursor_forks_fail_instead_of_reporting_mass_changes() -> Result<()> {
    let before=world(10,5,&[(1,1,7)]);
    for case in 0..5 {
        let mut after=world(11,6,&[(1,1,8)]);
        match case {0=>after.cursor.epoch=1,1=>after.cursor.sequence=4,
            2=>after.tick=GameTick(9),3=>after.cursor=before.cursor,
            _=>after.fortress_id=FortressId::new(2)}
        after.refresh_hash();
        assert!(matches!(compare(&before,&after),Err(e)if e.code==ErrorCode::StaleAnchor));
    }
    Ok(())
}

#[test]
fn continuations_bind_record_pair_session_and_selection_but_not_page_width() -> Result<()> {
    let before=world(1,0,&[(1,1,1),(2,1,2),(3,1,3)]);
    let after=world(2,1,&[(1,1,4),(2,1,5),(3,1,6)]);
    let c=context(&after);let binding=Digest32::of_bytes(b"pair");
    let first=compare_endpoints(&before,&after,&c,&selection(),binding,Some(1),None)?;
    let token=first["continuation"].as_str().ok_or_else(||invalid("test continuation absent"))?;
    let rest=compare_endpoints(&before,&after,&c,&selection(),binding,Some(128),Some(token))?;
    assert_eq!(rest["returned"],2);assert_eq!(rest["changes"][0]["entity_id"],"2");
    let mut other=c.clone();other.session_id=SessionId::new(782);
    assert!(compare_endpoints(&before,&after,&other,&selection(),binding,None,Some(token)).is_err());
    assert!(compare_endpoints(&before,&after,&c,&selection(),Digest32::ZERO,None,Some(token)).is_err());
    let mut select=selection();select["order"]=json!("id_descending");
    assert!(compare_endpoints(&before,&after,&c,&select,binding,None,Some(token)).is_err());
    Ok(())
}

#[test]
fn complete_selections_must_fit_retention_bounds_without_silent_partial_comparison() -> Result<()> {
    let empty=world(1,0,&[]);
    let values:Vec<_>=(1..=257).map(|id|(id,1,id as i64)).collect();
    let many=world(2,1,&values);
    assert!(matches!(compare(&empty,&many),Err(e)if e.code==ErrorCode::BudgetExceeded));
    Ok(())
}

#[test]
fn current_authority_cannot_be_revived_by_selecting_older_authorized_ticks() -> Result<()> {
    let before=world(1,0,&[(1,1,1)]);let after=world(2,1,&[(1,1,2)]);
    for case in 0..4 {
        let mut c=context(&after);c.anchor.tick=GameTick(100);
        match case {0=>c.grants.clear(),1=>c.cancellation_requested=true,
            2=>c.grants[0].expires_at_tick=Some(GameTick(50)),_=>c.grants[0].remaining_uses=Some(0)}
        assert!(compare_endpoints(&before,&after,&c,&selection(),Digest32::ZERO,None,None).is_err());
    }
    Ok(())
}

#[test]
fn tiny_response_or_invalid_selector_never_returns_an_empty_progress_page() -> Result<()> {
    let before=world(1,0,&[(1,1,1)]);let after=world(2,1,&[(1,1,2)]);
    let mut c=context(&after);c.budget.max_bytes=1;
    assert!(matches!(compare_endpoints(&before,&after,&c,&selection(),Digest32::ZERO,None,None),Err(e)if e.code==ErrorCode::BudgetExceeded));
    let c=context(&after);let mut select=selection();select["limit"]=json!(1);
    assert!(compare_endpoints(&before,&after,&c,&select,Digest32::ZERO,None,None).is_err());
    assert!(compare_endpoints(&before,&after,&c,&selection(),Digest32::ZERO,Some(0),None).is_err());
    assert!(compare_endpoints(&before,&after,&c,&selection(),Digest32::ZERO,Some(129),None).is_err());
    Ok(())
}
