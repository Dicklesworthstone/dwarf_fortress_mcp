use super::*;
use dfmcp_core::{CapabilityGrant, CapabilityScope, FortressId, GameTick, ObservationCursor, RequestId, WorkBudget};
use dfmcp_world::{Fact, WorldGraph};

fn source() -> Digest32 { Digest32::of_bytes(b"count-watch-native-fixture") }
fn snapshot(rows: &[(u64, Option<bool>)], tick: u64) -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    for &(id, value) in rows {
        let fields = value.map(|v| ("ready".into(), Fact::known(WorldValue::Bool(v), GameTick(tick),
            FactSource::DfhackField("count-test.ready".into()), source()))).into_iter().collect();
        graph.entities.insert(EntityId::new(id), EntityRecord { id: EntityId::new(id), generation: 1,
            revision: tick, kind: EntityKind::Unit, label: String::new(), fields });
    }
    WorldSnapshot::new(FortressId::new(17), GameTick(tick), ObservationCursor { epoch: 1, sequence: tick }, true, graph)
}
fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext { session_id: SessionId::new(9_010_017), request_id: RequestId::new(1), anchor: snapshot.anchor(),
        budget: WorkBudget { max_entities: 1000, max_bytes: 1024*1024, max_output_tokens: 65536,
            max_wall_millis: 60_000, max_game_ticks: 1000, ..WorkBudget::default() },
        grants: vec![CapabilityGrant { capability: Capability::Query, scope: CapabilityScope::default(),
            max_risk: RiskTier::ReadOnly, expires_at_tick: None, remaining_uses: None }], cancellation_requested: false }
}
fn predicate() -> Predicate {
    Predicate::Field { field: "ready".into(), comparison: Comparison::Eq, value: Literal::Bool(true) }
}
fn condition(comparison: Comparison, value: u64) -> Condition {
    Condition::EntityCount { scope: Scope::ObservedProjection, kind: Kind::Unit, predicate: predicate(), comparison, value }
}
fn evaluate_count(snapshot: &WorldSnapshot, comparison: Comparison, value: u64) -> Result<(Truth, Value)> {
    let mut probe = Probe::default();
    let truth = probe.evaluate(&condition(comparison, value), snapshot)?;
    Ok((truth, probe.facts.remove(0)))
}
fn request(key: &str, condition: &Condition) -> Value {
    json!({"schema":"dfmcp.query/1","query":{"kind":"watch","key":key,"condition":condition,
        "deadline_tick":100,"poll_interval_ticks":1,"stable_observations":2}})
}
fn call(store: &Mutex<Store>, snapshot: &WorldSnapshot, input: Value) -> Result<Value> {
    let raw = execute_in(store, snapshot, &context(snapshot), &input, |value| Ok(value.to_string()))?;
    serde_json::from_str(&raw).map_err(|_| invalid("test JSON"))
}

#[test]
fn every_small_count_interval_matches_exhaustive_integer_completions() {
    for lower in 0..=8 {
        for upper in lower..=8 {
            for threshold in 0..=10 {
                for comparison in [Comparison::Eq, Comparison::Ne, Comparison::Lt, Comparison::Le, Comparison::Gt, Comparison::Ge] {
                    let values: Vec<_> = (lower..=upper).map(|n| match comparison {
                        Comparison::Eq => n==threshold, Comparison::Ne => n!=threshold,
                        Comparison::Lt => n<threshold, Comparison::Le => n<=threshold,
                        Comparison::Gt => n>threshold, Comparison::Ge => n>=threshold,
                    }).collect();
                    let expected = if values.iter().all(|v|*v) { Truth::True }
                        else if values.iter().all(|v|!*v) { Truth::False } else { Truth::Unknown };
                    assert_eq!(interval_truth(lower,upper,comparison,threshold),expected);
                }
            }
        }
    }
    assert_eq!(interval_truth(0,u64::MAX,Comparison::Ne,u64::MAX),Truth::Unknown);
    assert_eq!(interval_truth(u64::MAX,u64::MAX,Comparison::Ge,u64::MAX),Truth::True);
    assert_eq!(interval_truth(0,u64::MAX,Comparison::Gt,u64::MAX),Truth::False);
}

#[test]
fn missing_rows_widen_counts_without_becoming_false_or_true_under_negation() -> Result<()> {
    let s = snapshot(&[(1,Some(true)),(2,Some(false)),(3,None)],3);
    for (cmp,n,expected) in [(Comparison::Ge,1,Truth::True),(Comparison::Ge,2,Truth::Unknown),
        (Comparison::Ge,3,Truth::False),(Comparison::Eq,1,Truth::Unknown),(Comparison::Ne,1,Truth::Unknown)] {
        let (truth,evidence)=evaluate_count(&s,cmp,n)?;
        assert_eq!(truth,expected);assert_eq!(evidence["matched_min"],1);assert_eq!(evidence["matched_max"],2);
        assert_eq!(evidence["population"],3);assert_eq!(evidence["complete_world_count_proven"],false);
        assert_eq!(evidence["snapshot_hash"],s.state_hash.to_string());
    }
    let mut probe=Probe::default();
    assert_eq!(probe.evaluate(&Condition::Not {arg:Box::new(condition(Comparison::Eq,1))},&s)?,Truth::Unknown);
    let mut budget=EvaluationBudget::new(60_000);
    assert_eq!(row_truth(&Predicate::Not {arg:Box::new(predicate())},&s.graph.entities[&EntityId::new(3)],&s,&mut budget)?,Truth::Unknown);
    Ok(())
}

#[test]
fn every_non_native_or_inconsistent_field_remains_unestablished() -> Result<()> {
    let base=snapshot(&[(1,Some(true))],3);
    for case in 0..10 {
        let mut graph=base.graph.clone();
        let fact=graph.entities.get_mut(&EntityId::new(1)).and_then(|e|e.fields.get_mut("ready"))
            .ok_or_else(||invalid("test fact"))?;
        match case {
            0=>fact.presence=Some(FactPresence::Absent),
            1=>fact.presence=Some(FactPresence::Unknown("unknown".into())),
            2=>fact.presence=Some(FactPresence::Unsupported("unsupported".into())),
            3=>fact.presence=Some(FactPresence::Omitted("omitted".into())),
            4=>fact.presence=Some(FactPresence::Redacted("hidden".into())),
            5=>fact.presence=Some(FactPresence::Known(WorldValue::Bool(false))),
            6=>fact.source=FactSource::Derived("not observation".into()),
            7=>fact.source_digest=Digest32::ZERO,
            8=>fact.observed_at=GameTick(2),
            _=>{fact.value=WorldValue::U64(1);fact.presence=None;},
        }
        let s=WorldSnapshot::new(base.fortress_id,base.tick,base.cursor,true,graph);
        let (truth,evidence)=evaluate_count(&s,Comparison::Eq,0)?;
        assert_eq!(truth,Truth::Unknown,"case={case}");assert_eq!(evidence["matched_min"],0);
        assert_eq!(evidence["matched_max"],1);assert_eq!(evidence["unestablished"],1);
    }
    Ok(())
}

#[test]
fn dynamic_membership_can_change_while_count_stability_is_preserved() -> Result<()> {
    let store=Mutex::new(Store::default());let first=snapshot(&[(1,Some(true)),(2,Some(true))],3);
    let created=call(&store,&first,request("dynamic",&condition(Comparison::Ge,2)))?;
    assert_eq!(created["record"]["status"],"candidate");
    let second=snapshot(&[(2,Some(true)),(3,Some(true))],4);
    let result=call(&store,&second,json!({"schema":"dfmcp.query/1","query":{
        "kind":"poll_watch","watch":created["record"]["watch"]}}))?;
    assert_eq!(result["record"]["status"],"satisfied");
    assert_eq!(result["record"]["stable_observations"],2);
    assert_eq!(result["record"]["evaluation"]["facts"][0]["membership"],"dynamic_at_each_sample");
    let third=snapshot(&[],5);
    let replay=call(&store,&third,json!({"schema":"dfmcp.query/1","query":{
        "kind":"poll_watch","watch":created["record"]["watch"]}}))?;
    assert_eq!(replay["record"]["evidence_digest"],result["record"]["evidence_digest"]);
    assert_eq!(replay["record"]["evaluation_current"],false);Ok(())
}

#[test]
fn count_failure_guard_blocks_success_until_the_range_is_decisive() -> Result<()> {
    let store=Mutex::new(Store::default());let first=snapshot(&[(1,None)],3);
    let mut input=request("guard",&Condition::Paused {value:true});
    input["query"]["failure_condition"]=json!(condition(Comparison::Ge,1));
    let created=call(&store,&first,input)?;
    assert_eq!(created["record"]["status"],"blocked_unknown");
    let second=snapshot(&[(1,Some(true))],4);
    let result=call(&store,&second,json!({"schema":"dfmcp.query/1","query":{
        "kind":"poll_watch","watch":created["record"]["watch"]}}))?;
    assert_eq!(result["record"]["status"],"failed");Ok(())
}

#[test]
fn count_nodes_share_existing_definition_bounds_and_strict_deserialization() -> Result<()> {
    let store=Mutex::new(Store::default());let s=snapshot(&[],3);
    let mut input=request("shape",&condition(Comparison::Ge,1));
    input["query"]["condition"]["predicate"]=json!({"op":"always","unexpected":true});
    assert!(call(&store,&s,input).is_err());
    let mut input=request("shape",&condition(Comparison::Ge,1));
    input["query"]["condition"].as_object_mut().ok_or_else(||invalid("test condition"))?.remove("scope");
    assert!(call(&store,&s,input).is_err());
    let group=Predicate::All {args:vec![predicate();31]};
    let count=Condition::EntityCount {scope:Scope::ObservedProjection,kind:Kind::Unit,
        predicate:group,comparison:Comparison::Ge,value:1};
    let mut input=request("shape",&count);input["query"]["failure_condition"]=json!(count);
    assert!(matches!(call(&store,&s,input),Err(e)if e.code==ErrorCode::BudgetExceeded));
    assert!(lock(&store)?.entries.is_empty());Ok(())
}

#[test]
fn evaluation_work_is_shared_and_exhaustion_never_returns_a_partial_count() -> Result<()> {
    let s=snapshot(&[(1,Some(true)),(2,None)],3);let mut probe=Probe::default();
    let mut budget=EvaluationBudget::new(60_000);budget.used=MAX_EVALUATION_WORK-5;
    evaluate(&mut probe,&s,Kind::Unit,&predicate(),Comparison::Ge,1,&mut budget)?;
    let published_facts=probe.facts.clone();
    assert!(matches!(evaluate(&mut probe,&s,Kind::Unit,&predicate(),Comparison::Ge,1,&mut budget),
        Err(e)if e.code==ErrorCode::BudgetExceeded));
    assert_eq!(probe.facts,published_facts);
    let mut zero=EvaluationBudget::new(0);
    assert!(evaluate(&mut probe,&s,Kind::Unit,&predicate(),Comparison::Ge,1,&mut zero).is_err());Ok(())
}

#[test]
fn rejected_output_cannot_register_or_advance_an_aggregate_watch() -> Result<()> {
    let store=Mutex::new(Store::default());let s=snapshot(&[(1,Some(true))],3);
    assert!(execute_in(&store,&s,&context(&s),&request("render",&condition(Comparison::Ge,1)),
        |_|Err(bounded("injected output refusal"))).is_err());
    assert!(lock(&store)?.entries.is_empty());
    let created=call(&store,&s,request("render",&condition(Comparison::Ge,1)))?;
    let old=lock(&store)?.entries.values().next().map(|w|w.evidence_digest);
    let next=snapshot(&[(1,Some(true))],4);
    assert!(execute_in(&store,&next,&context(&next),&json!({"schema":"dfmcp.query/1","query":{
        "kind":"poll_watch","watch":created["record"]["watch"]}}),|_|Err(bounded("injected refusal"))).is_err());
    assert_eq!(lock(&store)?.entries.values().next().map(|w|w.evidence_digest),old);Ok(())
}

#[test]
fn empty_projection_is_explicitly_not_a_complete_world_absence_proof() -> Result<()> {
    let s=snapshot(&[],3);let (truth,evidence)=evaluate_count(&s,Comparison::Eq,0)?;
    assert_eq!(truth,Truth::True);assert_eq!(evidence["scope"],"observed_projection");
    assert_eq!(evidence["population"],0);assert_eq!(evidence["complete_world_count_proven"],false);Ok(())
}
