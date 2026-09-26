use super::*;
use dfmcp_core::{CapabilityGrant, CapabilityScope, EdgeId, FortressId, GameTick,
    ObservationCursor, RequestId, WorkBudget};
use dfmcp_world::{EdgeKind, EdgeRecord, EntityRecord, Fact, WorldGraph};

fn targets(n: u32, items: bool) -> Vec<Target> {
    (0..n).map(|i| Target { building_native_id: 10 + i, building_generation: 1,
        kind: Kind::Bed, max_stage: 3, item_native_id: items.then_some(100 + i),
        item_generation: items.then_some(1) }).collect()
}
fn fact(value: WorldValue, tick: u64) -> Fact {
    Fact::known(value, GameTick(tick), FactSource::DfhackField("operations/1.4.test".into()),
        Digest32::of_bytes(b"coherent construction test"))
}
fn entity(id: EntityId, kind: EntityKind, fields: Vec<(&str, WorldValue)>, tick: u64) -> EntityRecord {
    EntityRecord { id, generation: 1, revision: tick, kind, label: "test".into(),
        fields: fields.into_iter().map(|(k,v)| (k.into(),fact(v,tick))).collect() }
}
fn edge(graph: &mut WorldGraph, from: EntityId, to: EntityId, kind: EdgeKind, tick: u64) {
    let id = EdgeId::new(graph.edges.len() as u128 + 1);
    graph.edges.insert(id, EdgeRecord { id, from, to, kind, revision: tick,
        fields: BTreeMap::from([("relation".into(),fact(WorldValue::Text("test".into()),tick))]) });
}
fn snapshot(targets: &[Target], tick: u64) -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    for t in targets {
        let bid = building_entity_id(t.building_native_id);
        graph.entities.insert(bid, entity(bid, EntityKind::Building, vec![
            ("type_key",WorldValue::Text(t.kind.names().0.into())),
            ("build_stage",WorldValue::I64(i64::from(t.max_stage))),
            ("max_build_stage",WorldValue::I64(i64::from(t.max_stage))),
        ],tick));
        if let Some(native) = t.item_native_id {
            let id = item_entity_id(native);
            graph.entities.insert(id, entity(id,EntityKind::Item,vec![
                ("native_item_id",WorldValue::U64(u64::from(native))),
                ("type_key",WorldValue::Text(t.kind.names().1.into())),
                ("in_building",WorldValue::Bool(true)),("in_job",WorldValue::Bool(false)),
                ("removed",WorldValue::Bool(false)),("on_ground",WorldValue::Bool(false)),
                ("in_inventory",WorldValue::Bool(false)),
            ],tick));
            edge(&mut graph,id,bid,EdgeKind::ContainedIn,tick);
        }
    }
    WorldSnapshot::new(FortressId::new(1),GameTick(tick),
        ObservationCursor { epoch: 0,sequence: tick - 100 },true,graph)
}
fn rehash(s: WorldSnapshot) -> WorldSnapshot {
    WorldSnapshot::new(s.fortress_id,s.tick,s.cursor,s.paused,s.graph)
}
fn set_field(s: &mut WorldSnapshot, id: EntityId, key: &str, value: WorldValue) -> Result<()> {
    let e = s.graph.entities.get_mut(&id).ok_or_else(|| invalid("test entity missing"))?;
    e.fields.insert(key.into(),fact(value,s.tick.0));
    Ok(())
}
fn check(s: &WorldSnapshot, targets: &[Target], test: Test) -> Result<(Truth, Probe)> {
    let mut p = Probe::default();
    let truth = evaluate(&mut p,s,targets,test,&mut counts::EvaluationBudget::new(60_000))?;
    Ok((truth,p))
}
fn context(s: &WorldSnapshot) -> OperationContext {
    OperationContext { session_id:SessionId::new(887),request_id:RequestId::new(1),anchor:s.anchor(),
        budget:WorkBudget {max_wall_millis:60_000,max_entities:5000,max_bytes:131_072,
            max_output_tokens:32_768,max_game_ticks:1000,..WorkBudget::default()},
        grants:vec![CapabilityGrant { capability:Capability::Query,scope:CapabilityScope::default(),
            max_risk:RiskTier::ReadOnly,expires_at_tick:None,remaining_uses:None }],cancellation_requested:false }
}
fn definition(targets: &[Target]) -> Definition {
    Definition {key:"furnished-plan".into(),label:"All furniture".into(),
        condition:Condition::FurnitureSet {targets:targets.to_vec(),test:Test::AllComplete},
        failure_condition:Some(Condition::FurnitureSet {targets:targets.to_vec(),test:Test::AnyRemoval}),
        deadline_tick:110,poll_interval_ticks:1,stable_observations:2 }
}
fn request(targets: &[Target]) -> Value {
    let d = definition(targets);
    let mut q = json!(d);
    q["kind"] = json!("watch");
    json!({"schema":"dfmcp.query/1","query":q})
}
fn call(store: &Mutex<Store>, s: &WorldSnapshot, input: Value) -> Result<Value> {
    let text = execute_in(store,s,&context(s),&input,|v|Ok(v.to_string()))?;
    serde_json::from_str(&text).map_err(|_|invalid("test response"))
}

#[test]
fn singleton_expansion_matches_existing_construction_fixture() -> Result<()> {
    let fixture:Value=serde_json::from_str(include_str!("../tests/fixtures/construction_condition_v1.json"))
        .map_err(|_|invalid("test fixture"))?;
    let mut target=targets(1,true).remove(0);target.item_native_id=Some(20);
    assert_eq!(recipe(&target,Test::AllComplete),fixture["condition"]);
    assert_eq!(recipe(&target,Test::AnyRemoval),fixture["failure_condition"]);
    let expanded=Definition { condition:serde_json::from_value(recipe(&target,Test::AllComplete))
        .map_err(|_|invalid("test recipe"))?,failure_condition:Some(serde_json::from_value(
        recipe(&target,Test::AnyRemoval)).map_err(|_|invalid("test failure recipe"))?),..definition(&[target]) };
    validate_definition(&expanded)
}

#[test]
fn target_bounds_and_closed_schema_do_not_weaken_the_goal() -> Result<()> {
    assert!(validate(&[]).is_err());assert!(validate(&targets(33,false)).is_err());
    for case in 0..8 {
        let mut t=targets(2,true);
        match case {0=>t[1].building_native_id=t[0].building_native_id,1=>t.reverse(),
            2=>t[1].item_native_id=t[0].item_native_id,3=>t[0].building_generation=0,
            4=>t[0].max_stage=0,5=>t[0].max_stage=33,6=>t[0].item_generation=None,
            _=>t[0].item_native_id=Some(i32::MAX as u32)}
        assert!(validate(&t).is_err());
    }
    let mut extra=json!(targets(1,true)[0]);extra["skip_missing"]=json!(true);
    assert!(serde_json::from_value::<Target>(extra).is_err());
    let valid=definition(&targets(32,true));validate_definition(&valid)?;
    let mut too_many=valid.clone();too_many.condition=Condition::All {args:vec![
        valid.condition.clone(),valid.condition.clone()]};
    assert!(validate_definition(&too_many).is_err());
    validate_input(&request(&targets(32,true)))?;
    let bytes=serde_json::to_vec(&valid).map_err(|_|invalid("test encode"))?;
    assert_eq!(valid,serde_json::from_slice::<Definition>(&bytes).map_err(|_|invalid("test replay"))?);
    Ok(())
}

#[test]
fn thirty_two_targets_have_one_shared_complete_evaluation() -> Result<()> {
    let mut t=targets(32,true);t[10].kind=Kind::Chair;t[20].kind=Kind::Table;
    let s=snapshot(&t,100);let (truth,p)=check(&s,&t,Test::AllComplete)?;
    assert_eq!(truth,Truth::True);assert!(!p.invalid_generation);
    assert_eq!(p.facts.len(),1);assert_eq!(p.facts[0]["selected"],32);
    assert_eq!(p.facts[0]["records"].as_array().map(Vec::len),Some(32));
    assert_eq!(check(&s,&t,Test::AnyRemoval)?.0,Truth::False);
    Ok(())
}

#[test]
fn every_selected_item_flag_is_checked_not_just_the_first_building() -> Result<()> {
    let t=targets(32,true);
    for index in [0,15,31] {
        for flags in 0..32 {
            let mut s=snapshot(&t,100);let id=item_entity_id(100+index);
            for (bit,key) in ["in_building","in_job","removed","on_ground","in_inventory"].iter().enumerate() {
                set_field(&mut s,id,key,WorldValue::Bool(flags&(1<<bit)!=0))?;
            }
            assert_eq!(check(&rehash(s),&t,Test::AllComplete)?.0,Truth::from_bool(flags==1));
        }
    }
    Ok(())
}

#[test]
fn recycled_late_reference_cannot_hide_after_a_decisive_target() -> Result<()> {
    let t=targets(32,true);let mut s=snapshot(&t,100);
    set_field(&mut s,building_entity_id(10),"build_stage",WorldValue::I64(0))?;
    s.graph.entities.get_mut(&item_entity_id(131)).ok_or_else(||invalid("test item"))?.generation=2;
    let (truth,p)=check(&rehash(s.clone()),&t,Test::AllComplete)?;
    assert_eq!(truth,Truth::False);assert!(p.invalid_generation);
    assert!(check(&rehash(s),&t,Test::AnyRemoval)?.1.invalid_generation);
    Ok(())
}

#[test]
fn missing_or_unestablished_roots_and_stale_fields_never_satisfy() -> Result<()> {
    let t=targets(2,true);
    for case in 0..5 {
        let mut s=snapshot(&t,100);let id=item_entity_id(101);
        match case {0=>{s.graph.entities.remove(&id);s.graph.edges.clear();},
            1=>s.graph.entities.get_mut(&id).ok_or_else(||invalid("test item"))?.revision=0,
            2=>s.graph.entities.get_mut(&id).ok_or_else(||invalid("test item"))?.kind=EntityKind::Job,
            3=>{let f=s.graph.entities.get_mut(&id).ok_or_else(||invalid("test item"))?
                .fields.get_mut("in_job").ok_or_else(||invalid("test field"))?;f.source_digest=Digest32::ZERO;},
            _=>{let f=s.graph.entities.get_mut(&id).ok_or_else(||invalid("test item"))?
                .fields.get_mut("in_job").ok_or_else(||invalid("test field"))?;f.observed_at=GameTick(99);}}
        assert_ne!(check(&rehash(s),&t,Test::AllComplete)?.0,Truth::True);
    }
    Ok(())
}

#[test]
fn a_late_removal_job_fails_the_entire_plan() -> Result<()> {
    let t=targets(32,true);let mut s=snapshot(&t,100);let job=EntityId::new(52);
    s.graph.entities.insert(job,entity(job,EntityKind::Job,vec![
        ("type_key",WorldValue::Text("DestroyBuilding".into()))],100));
    edge(&mut s.graph,job,building_entity_id(41),EdgeKind::ContainedIn,100);
    let s=rehash(s);
    assert_eq!(check(&s,&t,Test::AnyRemoval)?.0,Truth::True);
    let result=call(&Mutex::new(Store::default()),&s,request(&t))?;
    assert_eq!(result["record"]["status"],"failed");
    Ok(())
}

#[test]
fn whole_plan_not_independent_historical_successes_and_paused_repeats() -> Result<()> {
    let t=targets(32,true);let store=Mutex::new(Store::default());let mut s=snapshot(&t,100);
    for id in 26..42 {set_field(&mut s,building_entity_id(id),"build_stage",WorldValue::I64(0))?;}
    let first=call(&store,&rehash(s),request(&t))?;let handle=first["record"]["watch"].clone();
    let poll=json!({"schema":"dfmcp.query/1","query":{"kind":"poll_watch","watch":handle}});
    let mut s=snapshot(&t,101);
    for id in 10..26 {set_field(&mut s,building_entity_id(id),"build_stage",WorldValue::I64(0))?;}
    assert_eq!(call(&store,&rehash(s),poll.clone())?["record"]["status"],"waiting");
    let s=snapshot(&t,102);let candidate=call(&store,&s,poll.clone())?;
    assert_eq!(candidate["record"]["status"],"candidate");
    assert_eq!(candidate["record"],call(&store,&s,poll.clone())?["record"]);
    assert_eq!(call(&store,&snapshot(&t,103),poll)?["record"]["status"],"satisfied");
    assert_eq!(lock(&store)?.entries.len(),1);
    Ok(())
}

#[test]
fn failed_output_publication_and_shared_budget_leave_no_registration() -> Result<()> {
    let t=targets(32,true);let s=snapshot(&t,100);let store=Mutex::new(Store::default());
    assert!(execute_in(&store,&s,&context(&s),&request(&t),|_|Err(bounded("test output failure"))).is_err());
    assert!(lock(&store)?.entries.is_empty());
    let mut c=context(&s);c.budget.max_bytes=64;
    assert!(execute_in(&store,&s,&c,&request(&t),|v|Ok(v.to_string())).is_err());
    assert!(lock(&store)?.entries.is_empty());
    assert!(evaluate(&mut Probe::default(),&s,&t,Test::AllComplete,
        &mut counts::EvaluationBudget::new(0)).is_err());
    c=context(&s);c.grants.clear();
    assert!(execute_in(&store,&s,&c,&request(&t),|v|Ok(v.to_string())).is_err());
    Ok(())
}

#[test]
fn singleton_inspection_retains_full_predicate_trace_without_expanding_set_pages() -> Result<()> {
    let t=targets(1,true);let s=snapshot(&t,100);
    let (_,p)=check(&s,&t,Test::AllComplete)?;
    let row=&p.facts[0]["records"][0];
    assert!(row["facts"].as_array().is_some_and(|facts|facts.len()>8));
    let t=targets(2,true);let (_,p)=check(&snapshot(&t,100),&t,Test::AllComplete)?;
    assert!(p.facts[0]["records"][0].get("facts").is_none());
    Ok(())
}
