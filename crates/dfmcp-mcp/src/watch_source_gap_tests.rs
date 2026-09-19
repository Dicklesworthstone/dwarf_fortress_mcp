use super::*;
use dfmcp_core::{CapabilityGrant, CapabilityScope, RequestId, WorkBudget};
use dfmcp_world::WorldGraph;

fn snapshot(tick: u64, epoch: u64) -> WorldSnapshot {
    WorldSnapshot::new(FortressId::new(71), GameTick(tick),
        ObservationCursor { epoch, sequence: tick }, true, WorldGraph::default())
}
fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext { session_id:SessionId::new(7_181_991), request_id:RequestId::new(1),
        anchor:snapshot.anchor(), budget:WorkBudget { max_wall_millis:60_000,
            max_bytes:1024*1024,max_output_tokens:65536,max_game_ticks:1000,
            ..WorkBudget::default() }, grants:[Capability::Query,Capability::Observe].into_iter()
            .map(|capability|CapabilityGrant {capability,scope:CapabilityScope::default(),
                max_risk:RiskTier::ReadOnly,expires_at_tick:None,remaining_uses:None}).collect(),
        cancellation_requested:false }
}
fn register(store: &Mutex<Store>, snapshot: &WorldSnapshot, stable: u32) -> Result<Value> {
    call(store, snapshot, json!({"schema":"dfmcp.query/1","query":{"kind":"watch",
        "key":"supply-goal","condition":{"op":"paused","value":true},
        "deadline_tick":100,"stable_observations":stable}}))
}
fn call(store: &Mutex<Store>, snapshot: &WorldSnapshot, input: Value) -> Result<Value> {
    let encoded = watches::execute_in(store,snapshot,&context(snapshot),&input,|v|Ok(v.to_string()))?;
    serde_json::from_str(&encoded).map_err(|_|corrupt("test JSON"))
}
fn poll(store: &Mutex<Store>, snapshot: &WorldSnapshot, handle: &Value) -> Result<Value> {
    call(store,snapshot,json!({"schema":"dfmcp.query/1","query":{"kind":"poll_watch","watch":handle}}))
}
fn evidence(store: &Mutex<Store>) -> Result<Vec<Digest32>> {
    Ok(watches::lock(store)?.entries.values().map(|w|w.evidence_digest).collect())
}

#[test]
fn interruption_preserves_intent_but_requires_a_new_success_streak() -> Result<()> {
    let store=Mutex::new(Store::default());let first=snapshot(1,1);
    let created=register(&store,&first,2)?;let handle=&created["record"]["watch"];
    assert_eq!(created["record"]["status"],"candidate");
    let info=interrupt_in(&store,&first,&context(&first),|v|Ok(v.to_string()))?;
    assert_eq!(info["changed_watches"],1);assert_eq!(info["samples_added"],0);
    let same=poll(&store,&first,handle)?;
    assert_eq!(same["record"]["status"],"blocked_unknown");
    assert_eq!(same["record"]["sample_count"],1);
    assert_eq!(same["record"]["stable_observations"],0);
    assert_eq!(same["record"]["definition"],created["record"]["definition"]);
    assert_eq!(same["record"]["created_at"],created["record"]["created_at"]);
    assert_eq!(same["record"]["evaluation"]["reason"],REASON);
    let next=poll(&store,&snapshot(2,1),handle)?;
    assert_eq!(next["record"]["status"],"candidate");
    assert_eq!(next["record"]["stable_observations"],1);
    assert_eq!(poll(&store,&snapshot(3,1),handle)?["record"]["status"],"satisfied");
    Ok(())
}

#[test]
fn repeated_failed_recovery_does_not_rewrite_the_same_gap() -> Result<()> {
    let store=Mutex::new(Store::default());let s=snapshot(1,1);register(&store,&s,2)?;
    interrupt_in(&store,&s,&context(&s),|v|Ok(v.to_string()))?;let previous=evidence(&store)?;
    let again=interrupt_in(&store,&s,&context(&s),|v|Ok(v.to_string()))?;
    assert_eq!(again["changed_watches"],0);assert_eq!(again["pending_watches"],1);
    assert_eq!(evidence(&store)?,previous);Ok(())
}

#[test]
fn terminal_outcomes_are_not_restarted_or_resealed() -> Result<()> {
    let store=Mutex::new(Store::default());let s=snapshot(1,1);register(&store,&s,1)?;
    let before=evidence(&store)?;
    let info=interrupt_in(&store,&s,&context(&s),|v|Ok(v.to_string()))?;
    assert_eq!(info["changed_watches"],0);assert_eq!(info["terminal_watches"],1);
    assert_eq!(info["pending_watches"],0);assert_eq!(before,evidence(&store)?);Ok(())
}

#[test]
fn expired_and_cross_epoch_intents_cannot_be_revived() -> Result<()> {
    for (target,status) in [(snapshot(100,1),"expired"),(snapshot(2,2),"invalidated"),
        (snapshot(0,1),"invalidated")] {
        let store=Mutex::new(Store::default());let s=snapshot(1,1);let created=register(&store,&s,2)?;
        interrupt_in(&store,&target,&context(&target),|v|Ok(v.to_string()))?;
        let record=poll(&store,&target,&created["record"]["watch"])?;
        assert_eq!(record["record"]["status"],status);
        assert_eq!(record["record"]["deadline_tick"],100);
    }
    Ok(())
}

#[test]
fn permission_cancellation_budget_and_output_refusal_leave_watches_unchanged() -> Result<()> {
    for case in 0..6 {
        let store=Mutex::new(Store::default());let s=snapshot(1,1);register(&store,&s,2)?;
        let before=evidence(&store)?;let mut c=context(&s);
        match case {
            0=>c.grants.retain(|g|g.capability!=Capability::Observe),
            1=>c.grants.retain(|g|g.capability!=Capability::Query),
            2=>c.cancellation_requested=true,
            3=>c.budget.max_bytes=1,
            4=>c.budget.max_wall_millis=0,
            _=>{},
        }
        assert!(interrupt_in(&store,&s,&c,|v|if case==5 {
            Err(bounded("injected publisher refusal"))
        } else {Ok(v.to_string())}).is_err(),"case={case}");
        assert_eq!(evidence(&store)?,before,"case={case}");
    }
    Ok(())
}

#[test]
fn source_gap_checkpoint_keeps_serialized_watch_identity_and_counters() -> Result<()> {
    let store=Mutex::new(Store::default());let s=snapshot(1,1);register(&store,&s,2)?;
    interrupt_in(&store,&s,&context(&s),|v|Ok(v.to_string()))?;
    let guard=watches::lock(&store)?;
    let saved=SavedSet::capture(&guard,&context(&s));drop(guard);
    let encoded=serde_json::to_vec(&saved).map_err(|_|corrupt("test saved set"))?;
    let decoded=decode_saved(&encoded,&BTreeMap::from([(s.anchor(),0)]))?;
    assert_eq!(saved,decoded);
    assert_eq!(decoded.watches[0].samples,1);assert_eq!(decoded.watches[0].streak,0);
    assert_eq!(decoded.watches[0].status,"blocked_unknown");Ok(())
}
