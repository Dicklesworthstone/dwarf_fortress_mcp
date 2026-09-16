use super::*;
use dfmcp_core::{CapabilityGrant, CapabilityScope, FortressId, GameTick, ObservationCursor, RequestId, WorkBudget};
use dfmcp_world::{Fact, WorldGraph};

fn fixture(rows: &[(Option<bool>, Option<u64>)]) -> (WorldSnapshot, OperationContext) {
    let tick = GameTick(3);
    let known = |value| Fact::known(value, tick, FactSource::DfhackField("item.test".into()), Digest32::of_bytes(b"quantity fixture"));
    let unknown = || Fact::with_presence(FactPresence::Omitted("not observed".into()), tick, FactSource::Replay, Digest32::ZERO);
    let mut graph = WorldGraph::default();
    for (i, (keep, units)) in rows.iter().enumerate() {
        let id = EntityId::new(100 + i as u64);
        graph.entities.insert(id, EntityRecord { id, generation:1, revision:1, kind:EntityKind::Item,
            label:"stack".into(), fields:BTreeMap::from([
                ("keep".into(), keep.map_or_else(unknown, |value| known(WorldValue::Bool(value)))),
                ("stack_size".into(), units.map_or_else(unknown, |value| known(WorldValue::U64(value)))),
            ]) });
    }
    let snapshot = WorldSnapshot::new(FortressId::new(991), tick, ObservationCursor::ORIGIN, true, graph);
    let context = OperationContext { session_id:SessionId::new(912_002), request_id:RequestId::new(1), anchor:snapshot.anchor(),
        budget:WorkBudget { max_entities:100_000, max_bytes:1024*1024, max_output_tokens:65536, max_wall_millis:60000, ..WorkBudget::default() },
        grants:vec![CapabilityGrant { capability:Capability::Query, scope:CapabilityScope::default(),
            max_risk:RiskTier::ReadOnly, expires_at_tick:None, remaining_uses:None }], cancellation_requested:false };
    (snapshot, context)
}
fn selector() -> Predicate {
    Predicate::Field { field:"keep".into(), comparison:Comparison::Eq, value:Literal::Bool(true) }
}
fn scan(snapshot: &WorldSnapshot) -> Result<Measurement> {
    measure(snapshot, &selector(), &mut EvaluationBudget::new(60000))
}
fn request() -> Value {
    json!({"schema":"dfmcp.query/1","query":{"kind":"item_quantity","scope":"observed_projection",
        "quantity_unit":"stack_units","predicate":selector()}})
}
fn condition(comparison: Comparison, value: u64) -> Condition {
    Condition::ItemQuantity { scope:Scope::ObservedProjection, quantity_unit:QuantityUnit::StackUnits,
        predicate:selector(), comparison, value }
}
fn register(condition: Condition, failure: Option<Condition>) -> Value {
    json!({"schema":"dfmcp.query/1","query":{"kind":"watch","key":"quantity","condition":condition,
        "failure_condition":failure,"deadline_tick":30,"stable_observations":1}})
}

#[test]
fn quantity_sums_units_not_records_and_excludes_other_entity_kinds() -> Result<()> {
    let (mut snapshot, _) = fixture(&[(Some(true),Some(2)),(Some(true),Some(50)),(Some(false),Some(999))]);
    let mut other = snapshot.graph.entities[&EntityId::new(100)].clone();
    other.id = EntityId::new(200); other.kind = EntityKind::Building;
    snapshot.graph.entities.insert(other.id,other); snapshot.refresh_hash();
    let result = scan(&snapshot)?;
    assert_eq!(result.population,3); assert_eq!(result.matched,2);
    assert_eq!(result.bounds,Bounds {lower:52,upper:Some(52)});
    Ok(())
}

#[test]
fn uncertain_membership_and_unknown_quantity_have_different_sound_bounds() -> Result<()> {
    let (snapshot, _) = fixture(&[(Some(true),Some(5)),(None,Some(9)),(Some(false),None)]);
    let bounded = scan(&snapshot)?;
    assert_eq!(bounded.bounds,Bounds {lower:5,upper:Some(14)});
    assert_eq!(bounded.unknown_quantity,0);
    assert_eq!(bounded.bounds.compare(Comparison::Ge,5),Truth::True);
    assert_eq!(bounded.bounds.compare(Comparison::Ge,15),Truth::False);
    assert_eq!(bounded.bounds.compare(Comparison::Ne,5),Truth::Unknown);
    let (snapshot, _) = fixture(&[(Some(true),Some(5)),(Some(true),None)]);
    let unbounded = scan(&snapshot)?;
    assert_eq!(unbounded.bounds,Bounds {lower:5,upper:None});
    assert_eq!(unbounded.bounds.compare(Comparison::Eq,4),Truth::False);
    assert_eq!(unbounded.bounds.compare(Comparison::Ne,4),Truth::True);
    assert_eq!(unbounded.bounds.compare(Comparison::Le,u64::MAX),Truth::Unknown);
    assert_eq!(unbounded.bounds.compare(Comparison::Ge,5),Truth::True);
    Ok(())
}

#[test]
fn zero_sized_unknown_membership_does_not_make_the_quantity_unknown() -> Result<()> {
    let (snapshot, _) = fixture(&[(None,Some(0)),(Some(false),None)]);
    let result = scan(&snapshot)?;
    assert_eq!(result.bounds,Bounds::default());
    assert_eq!(result.unknown_membership,1); assert_eq!(result.unknown_quantity,0);
    assert_eq!(result.bounds.compare(Comparison::Eq,0),Truth::True);
    let (empty, _) = fixture(&[]);
    assert_eq!(scan(&empty)?.bounds,Bounds::default());
    Ok(())
}

#[test]
fn unavailable_or_wrongly_typed_quantities_never_leak_their_backing_values() -> Result<()> {
    let (base, _) = fixture(&[(Some(true),Some(7))]);
    for case in 0..9 {
        let mut snapshot = base.clone();
        let fact = snapshot.graph.entities.get_mut(&EntityId::new(100)).ok_or_else(||invalid("fixture item"))?
            .fields.get_mut("stack_size").ok_or_else(||invalid("fixture quantity"))?;
        match case {
            0 => fact.presence=Some(FactPresence::Redacted("hidden".into())),
            1 => fact.presence=Some(FactPresence::Absent),
            2 => fact.presence=Some(FactPresence::Known(WorldValue::U64(999))),
            3 => fact.observed_at=GameTick(2),
            4 => fact.source=FactSource::Replay,
            5 => fact.source_digest=Digest32::ZERO,
            6 => {fact.presence=None;fact.value=WorldValue::I64(-7);},
            7 => {fact.presence=None;fact.value=WorldValue::Text("7".into());},
            _ => {fact.presence=None;fact.value=WorldValue::I64(7);},
        }
        snapshot.refresh_hash();
        let report = scan(&snapshot)?;
        assert_eq!(report.bounds,Bounds {lower:0,upper:None},"case={case}");
        assert_eq!(report.unknown_quantity,1);
    }
    Ok(())
}

#[test]
fn all_finite_interval_comparisons_match_enumerated_possible_counts() {
    for lower in 0..12 {
        for upper in lower..12 {
            for target in 0..14 {
                for comparison in [Comparison::Eq,Comparison::Ne,Comparison::Lt,Comparison::Le,Comparison::Gt,Comparison::Ge] {
                    let values:Vec<_>=(lower..=upper).map(|n| match comparison {
                        Comparison::Eq=>n==target,Comparison::Ne=>n!=target,Comparison::Lt=>n<target,
                        Comparison::Le=>n<=target,Comparison::Gt=>n>target,Comparison::Ge=>n>=target,
                    }).collect();
                    let expected=if values.iter().all(|v|*v){Truth::True}
                        else if values.iter().all(|v|!*v){Truth::False}else{Truth::Unknown};
                    assert_eq!(Bounds {lower,upper:Some(upper)}.compare(comparison,target),expected);
                }
            }
        }
    }
}

#[test]
fn overflow_and_work_exhaustion_emit_no_partial_quantity_evidence() -> Result<()> {
    for rows in [[(Some(true),Some(u64::MAX)),(Some(true),Some(1))],
        [(None,Some(u64::MAX)),(None,Some(1))]] {
        let (snapshot, _) = fixture(&rows); let mut probe=Probe::default();
        assert!(matches!(probe.evaluate(&condition(Comparison::Ge,1),&snapshot),Err(e) if e.code==ErrorCode::BudgetExceeded));
        assert!(probe.facts.is_empty());
    }
    let (snapshot, _) = fixture(&[(Some(true),Some(9))]);
    let mut budget=EvaluationBudget::new(60000);budget.used=MAX_EVALUATION_WORK-1;
    let mut probe=Probe::default();
    assert!(matches!(evaluate(&mut probe,&snapshot,&selector(),Comparison::Ge,1,&mut budget),Err(e) if e.code==ErrorCode::BudgetExceeded));
    assert!(probe.facts.is_empty());
    Ok(())
}

#[test]
fn inspection_and_watch_use_identical_measurements_without_registry_changes() -> Result<()> {
    let (snapshot, context) = fixture(&[(Some(true),Some(6)),(None,Some(4))]);
    let inspected=query(&snapshot,&context,&request())?;
    let mut probe=Probe::default();assert_eq!(probe.evaluate(&condition(Comparison::Ge,8),&snapshot)?,Truth::Unknown);
    let mut measured=probe.facts.remove(0);
    for key in ["op","comparison","threshold","truth"] {measured.as_object_mut().ok_or_else(||invalid("fact"))?.remove(key);}
    assert_eq!(measured,inspected["quantity"]);
    assert_eq!(inspected["watch_registered"],false); assert_eq!(inspected["watch_evaluated"],false);
    assert_eq!(query(&snapshot,&context,&request())?,inspected);
    Ok(())
}

#[test]
fn query_refusals_preserve_authority_identity_and_complete_output_bounds() -> Result<()> {
    let (snapshot, context) = fixture(&[(Some(true),Some(6))]);
    for case in 0..5 {
        let mut denied=context.clone();
        match case {0=>denied.grants.clear(),1=>denied.cancellation_requested=true,
            2=>denied.budget.max_entities=0,3=>denied.budget.max_bytes=1,_=>denied.budget.max_wall_millis=0}
        assert!(query(&snapshot,&denied,&request()).is_err());
    }
    for change in [json!({"quantity_unit":"portions"}),json!({"scope":"complete_world"}),json!({"limit":1})] {
        let mut input=request();input["query"].as_object_mut().ok_or_else(||invalid("query"))?
            .extend(change.as_object().ok_or_else(||invalid("change"))?.clone());
        assert!(query(&snapshot,&context,&input).is_err());
    }
    let mut stale=request();stale["expected_anchor"]=json!({});
    assert!(matches!(query(&snapshot,&context,&stale),Err(e) if e.code==ErrorCode::StaleAnchor));
    Ok(())
}

#[test]
fn quantity_failure_guard_and_render_errors_do_not_publish_a_false_success() -> Result<()> {
    let (snapshot, context) = fixture(&[(Some(true),Some(1)),(None,Some(4))]);
    let store=Mutex::new(Store::default());
    let input=register(Condition::Paused {value:true},Some(condition(Comparison::Lt,3)));
    let result=execute_in(&store,&snapshot,&context,&input,|v|Ok(v.to_string()))?;
    let result:Value=serde_json::from_str(&result).map_err(|_|invalid("response"))?;
    assert_eq!(result["record"]["status"],"blocked_unknown");
    let rejected=Mutex::new(Store::default());
    assert!(execute_in(&rejected,&snapshot,&context,&input,|_|Err(bounded("injected output refusal"))).is_err());
    assert!(lock(&rejected)?.entries.is_empty());
    Ok(())
}

#[test]
fn quantity_and_count_predicates_share_the_same_definition_budget() -> Result<()> {
    let leaf=selector();
    let mut definition=Definition {key:"bounds".into(),label:"bounds".into(),
        condition:Condition::All {args:vec![condition(Comparison::Ge,1),condition(Comparison::Ge,2)]},
        failure_condition:None,deadline_tick:30,poll_interval_ticks:1,stable_observations:2};
    validate_definition(&definition)?;
    definition.condition=Condition::ItemQuantity {scope:Scope::ObservedProjection,quantity_unit:QuantityUnit::StackUnits,
        predicate:Predicate::All {args:vec![leaf;63]},comparison:Comparison::Ge,value:1};
    assert!(matches!(validate_definition(&definition),Err(e) if e.code==ErrorCode::BudgetExceeded));
    Ok(())
}
