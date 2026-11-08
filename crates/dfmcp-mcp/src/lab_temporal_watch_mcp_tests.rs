//! Foreground laboratory watch lifecycle through the actual public tool facade.

use super::*;
use dfmcp_world::{FactPresence, FactSource, Value as WorldValue};
use serde_json::Value;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Owner {
    id: String,
}
impl Drop for Owner {
    fn drop(&mut self) {
        if let Ok(id) = u128::from_str_radix(&self.id, 16) {
            sessions().remove(&SessionId::new(id));
        }
    }
}

fn parsed(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn open(scenario: &str) -> TestResult<Owner> {
    let result = parsed(&crate::agent_facade::fortress_open_session(
        Some(true),
        None,
        Some(vec![
            ("observe".into(), "read_only".into()),
            ("query".into(), "read_only".into()),
            ("plan".into(), "reversible".into()),
            ("configure_labor".into(), "reversible".into()),
            ("control_clock".into(), "reversible".into()),
            ("checkpoint".into(), "guarded".into()),
            ("restore".into(), "guarded".into()),
        ]),
        Some(60_000),
        Some(2_000),
        Some(8_192),
        Some(4 * 1024 * 1024),
        Some(16_384),
        Some(64),
        Some(scenario.into()),
        Some(false),
        Some(false),
    ))?;
    assert_eq!(result["ok"], true, "{result}");
    Ok(Owner {
        id: result["session_id"].as_str().ok_or("session ID")?.into(),
    })
}

fn snapshot(owner: &Owner) -> TestResult<WorldSnapshot> {
    Ok(resolve_session(Some(owner.id.clone()))?
        .lock()
        .map_err(|_| "session lock")?
        .adapter
        .snapshot()
        .clone())
}

fn query(owner: &Owner, query: Value) -> TestResult<Value> {
    parsed(&crate::agent_facade::fortress_query(
        Some(owner.id.clone()),
        Some(
            json!({
                "mode":"temporal", "request":{"schema":"dfmcp.query/1", "query":query}
            })
            .to_string(),
        ),
    ))
}

fn watch(owner: &Owner, key: &str, condition: Value, stable: u32) -> TestResult<Value> {
    query(
        owner,
        json!({"kind":"watch", "key":key, "condition":condition,
        "deadline_tick":snapshot(owner)?.tick.0 + 100,
        "poll_interval_ticks":1, "stable_observations":stable}),
    )
}

fn handle(result: &Value) -> TestResult<String> {
    assert_eq!(result["ok"], true, "{result}");
    Ok(result["record"]["watch"]
        .as_str()
        .ok_or("watch handle")?
        .into())
}

fn refusal(result: &Value, code: &str) {
    assert_eq!(result["ok"], false, "{result}");
    assert_eq!(result["error"]["code"], code, "{result}");
}

fn commit(owner: &Owner, plan: Value) -> TestResult<Value> {
    assert_eq!(plan["ok"], true, "{plan}");
    let result = parsed(&crate::agent_facade::fortress_commit(
        Some(owner.id.clone()),
        plan["plan_digest"].as_str().ok_or("plan digest")?.into(),
    ))?;
    assert_eq!(result["ok"], true, "{result}");
    Ok(result)
}

fn mutate(owner: &Owner, edit: impl FnOnce(&mut WorldSnapshot) -> TestResult) -> TestResult {
    let session = resolve_session(Some(owner.id.clone()))?;
    let mut guard = session.lock().map_err(|_| "session lock")?;
    let mut source = guard.adapter.snapshot().clone();
    edit(&mut source)?;
    source.cursor = source.cursor.checked_next().ok_or("cursor overflow")?;
    source.refresh_hash();
    guard.adapter.inject_snapshot(source)?;
    Ok(())
}

#[test]
fn lab_watch_public_stock_discovery_preserves_baselines_and_pending_game_work() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("starter_fortress")?;
    let source = snapshot(&owner)?;
    let ledger = crate::lab_world::starter::STOCK_LEDGER;
    let field = dfmcp_intent::effects::STOCK_DRINK_FIELD;
    let condition = json!({"op":"field", "entity_id":ledger.to_string(), "generation":1,
        "field":field, "comparison":"ge", "value":{"type":"u64", "value":100}});
    let first = watch(&owner, "drink-recovery", condition.clone(), 2)?;
    let watch_id = handle(&first)?;
    assert_eq!(first["record"]["status"], "waiting");
    assert_eq!(first["record"]["evaluation"]["condition"], "false");
    assert_eq!(snapshot(&owner)?.anchor(), source.anchor());
    let replayed = watch(&owner, "drink-recovery", condition, 2)?;
    assert_eq!(replayed["replayed"], true);
    assert_eq!(replayed["record"], first["record"]);
    refusal(
        &query(&owner, json!({"kind":"release_watch", "watch":watch_id}))?,
        "conflict",
    );
    let captured = query(
        &owner,
        json!({"kind":"capture", "key":"roster", "max_game_ticks":100,
        "select":{"kind":"entities", "kinds":["unit"], "fields":["alive"]}}),
    )?;
    assert_eq!(captured["ok"], true, "{captured}");
    let baseline = captured["captured"]["baseline"]
        .as_str()
        .ok_or("baseline")?;
    let plan = parsed(&crate::agent_facade::fortress_plan(
        Some(owner.id.clone()),
        Some("resume laboratory time".into()),
        Some(false),
        None,
        None,
    ))?;
    assert_eq!(plan["ok"], true, "{plan}");
    let listed = query(&owner, json!({"kind":"watches"}))?;
    let work = &listed["agent_turn"]["active_work"];
    assert_eq!(work["query_baselines"][0]["baseline"], baseline);
    assert_eq!(work["query_watches"][0]["watch"], watch_id);
    assert_eq!(work["condition_watches"][0]["watch"], watch_id);
    assert_eq!(work["pending_plans"][0]["plan_digest"], plan["plan_digest"]);
    let observed = parsed(&crate::agent_facade::fortress_observe(Some(
        owner.id.clone(),
    )))?;
    assert_eq!(observed["ok"], true, "{observed}");
    assert_eq!(
        observed["agent_turn"]["active_work"]["query_watches"][0]["watch"],
        watch_id
    );
    assert_eq!(
        observed["agent_turn"]["active_work"]["query_baselines"][0]["baseline"],
        baseline
    );
    let cancelled = query(&owner, json!({"kind":"cancel_watch", "watch":watch_id}))?;
    assert_eq!(cancelled["record"]["status"], "cancelled");
    assert_eq!(
        cancelled["agent_turn"]["active_work"]["condition_watches"],
        json!([])
    );
    let next = &cancelled["agent_turn"]["active_work"]["query_watches"][0]["next_step"];
    assert!(next["arguments"].get("query").is_none());
    let released = parsed(&crate::agent_facade::fortress_query(
        Some(owner.id.clone()),
        next["arguments"]["mode"].as_str().map(str::to_owned),
    ))?;
    assert_eq!(released["released"], true, "{released}");
    commit(&owner, plan)?;
    assert!(!snapshot(&owner)?.paused);
    Ok(())
}

#[test]
fn lab_watch_public_batch_detects_real_labor_commit_after_authorized_clock_advance() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("starter_fortress")?;
    let original = snapshot(&owner)?;
    let labor = watch(
        &owner,
        "one-miner",
        json!({"op":"entity_count", "scope":"observed_projection",
        "kind":"unit", "predicate":{"op":"field", "field":"labor.MINING", "comparison":"eq",
            "value":{"type":"bool", "value":true}}, "comparison":"ge", "value":1}),
        1,
    )?;
    let labor_id = handle(&labor)?;
    assert_eq!(labor["record"]["status"], "blocked_unknown");
    let tick = watch(
        &owner,
        "advanced-clock",
        json!({"op":"tick_at_least", "value":original.tick.0 + 1}),
        1,
    )?;
    let tick_id = handle(&tick)?;
    let plan = parsed(&crate::agent_facade::fortress_plan(
        Some(owner.id.clone()),
        None,
        None,
        Some(
            json!([{"action":{"kind":"set_labor", "units":["1001"],
            "labor":"MINING", "enabled":true}}])
            .to_string(),
        ),
        None,
    ))?;
    commit(&owner, plan)?;
    let resume = parsed(&crate::agent_facade::fortress_plan(
        Some(owner.id.clone()),
        Some("advance for explicit watch sample".into()),
        Some(false),
        None,
        None,
    ))?;
    commit(&owner, resume)?;
    let waited = parsed(&crate::agent_facade::fortress_wait(
        Some(owner.id.clone()),
        Some(1),
    ))?;
    assert_eq!(waited["ok"], true, "{waited}");
    let before_poll = snapshot(&owner)?.anchor();
    assert!(before_poll.tick > original.tick);
    let result = query(
        &owner,
        json!({"kind":"poll_watches", "watches":[labor_id,tick_id]}),
    )?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["all_satisfied"], true, "{result}");
    assert_eq!(result["sampled"], 2);
    assert_eq!(result["atomic_watch_publication"], true);
    assert_eq!(result["native_captures"], 0);
    assert_eq!(result["advances_game"], false);
    assert_eq!(result["mutation_authority"], false);
    assert_eq!(
        result["agent_turn"]["active_work"]["condition_watches"],
        json!([])
    );
    assert_eq!(
        result["agent_turn"]["active_work"]["query_watches"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
    assert_eq!(snapshot(&owner)?.anchor(), before_poll);
    assert_eq!(query(&owner, json!({"kind":"poll_watches"}))?["sampled"], 0);
    Ok(())
}

#[test]
fn lab_watch_public_small_output_refuses_the_whole_batch_without_losing_samples() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("empty")?;
    let future = snapshot(&owner)?.tick.0 + 1;
    let condition = json!({"op":"tick_at_least", "value":future});
    watch(&owner, "first", condition.clone(), 1)?;
    watch(&owner, "second", condition, 1)?;
    let before = query(&owner, json!({"kind":"watches"}))?;
    mutate(&owner, |source| {
        source.tick = GameTick(future);
        Ok(())
    })?;
    let session = resolve_session(Some(owner.id.clone()))?;
    let budget = session.lock().map_err(|_| "session lock")?.budget;
    session
        .lock()
        .map_err(|_| "session lock")?
        .budget
        .max_output_tokens = 128;
    refusal(
        &query(&owner, json!({"kind":"poll_watches"}))?,
        "budget_exceeded",
    );
    session.lock().map_err(|_| "session lock")?.budget = budget;
    let after = query(&owner, json!({"kind":"watches"}))?;
    for index in 0..2 {
        assert_eq!(
            after["records"][index]["evidence_digest"],
            before["records"][index]["evidence_digest"]
        );
        assert_eq!(after["records"][index]["status"], "waiting");
    }
    let retry = query(&owner, json!({"kind":"poll_watches"}))?;
    assert_eq!(retry["all_satisfied"], true, "{retry}");
    assert_eq!(retry["sampled"], 2);
    Ok(())
}

#[test]
fn lab_watch_public_source_loss_is_unknown_and_native_only_conditions_are_refused() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("starter_fortress")?;
    let condition = json!({"op":"field", "entity_id":"1001", "generation":1, "field":"alive",
        "comparison":"eq", "value":{"type":"bool", "value":true}});
    let created = watch(&owner, "alive", condition, 2)?;
    let id = handle(&created)?;
    assert_eq!(created["record"]["status"], "candidate");
    mutate(&owner, |source| {
        let fact = source
            .graph
            .entities
            .get_mut(&EntityId::new(1001))
            .ok_or("unit")?
            .fields
            .get_mut("alive")
            .ok_or("alive")?;
        fact.source = FactSource::Replay;
        fact.presence = Some(FactPresence::Known(WorldValue::Bool(true)));
        Ok(())
    })?;
    let lost = query(&owner, json!({"kind":"poll_watch", "watch":id}))?;
    assert_eq!(lost["record"]["status"], "blocked_unknown", "{lost}");
    assert_eq!(lost["record"]["stable_observations"], 0);
    refusal(
        &watch(
            &owner,
            "native",
            json!({"op":"item_quantity", "scope":"observed_projection",
        "quantity_unit":"stack_units", "predicate":{"op":"always"}, "comparison":"ge", "value":1}),
            1,
        )?,
        "invalid_request",
    );
    assert_eq!(
        query(&owner, json!({"kind":"watches"}))?["records"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    Ok(())
}

#[test]
fn lab_watch_public_current_authority_and_restore_epoch_fence_retained_work() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("empty")?;
    let id = handle(&watch(
        &owner,
        "after-tick",
        json!({"op":"tick_at_least", "value":50}),
        2,
    )?)?;
    let session = resolve_session(Some(owner.id.clone()))?;
    let grants = session.lock().map_err(|_| "session lock")?.grants.clone();
    session
        .lock()
        .map_err(|_| "session lock")?
        .grants
        .retain(|g| g.capability != Capability::Query);
    for query_input in [
        json!({"kind":"watches"}),
        json!({"kind":"poll_watches"}),
        json!({"kind":"poll_watch", "watch":id}),
        json!({"kind":"cancel_watch", "watch":id}),
        json!({"kind":"release_watch", "watch":id}),
    ] {
        refusal(&query(&owner, query_input)?, "capability_denied");
    }
    session.lock().map_err(|_| "session lock")?.grants = grants;
    assert_eq!(
        query(&owner, json!({"kind":"watches"}))?["records"][0]["status"],
        "waiting"
    );
    let checkpoint = parsed(&crate::agent_facade::fortress_checkpoint(
        Some(owner.id.clone()),
        Some("watch restore".into()),
    ))?;
    assert_eq!(checkpoint["ok"], true, "{checkpoint}");
    let restored = parsed(&crate::agent_facade::fortress_restore(
        Some(owner.id.clone()),
        checkpoint["checkpoint_id"]
            .as_str()
            .ok_or("checkpoint")?
            .into(),
    ))?;
    assert_eq!(restored["ok"], true, "{restored}");
    let result = query(&owner, json!({"kind":"poll_watch", "watch":id}))?;
    assert_eq!(result["record"]["status"], "invalidated", "{result}");
    assert_eq!(
        result["agent_turn"]["active_work"]["condition_watches"],
        json!([])
    );
    assert_eq!(
        result["agent_turn"]["active_work"]["query_watches"][0]["watch"],
        id
    );
    Ok(())
}

#[test]
fn lab_watch_public_small_result_keeps_large_source_admission_independent() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("starter_fortress")?;
    mutate(&owner, |source| {
        let entity = source
            .graph
            .entities
            .get_mut(&EntityId::new(1001))
            .ok_or("unit")?;
        let mut fact = entity.fields.get("alive").ok_or("alive")?.clone();
        fact.value = WorldValue::Text("x".repeat(70_000));
        fact.presence = Some(FactPresence::Known(fact.value.clone()));
        entity.fields.insert("unselected_notes".into(), fact);
        Ok(())
    })?;
    resolve_session(Some(owner.id.clone()))?
        .lock()
        .map_err(|_| "session lock")?
        .budget
        .max_output_tokens = 4096;
    let result = watch(
        &owner,
        "small",
        json!({"op":"field", "entity_id":"1001", "generation":1,
        "field":"alive", "comparison":"eq", "value":{"type":"bool", "value":true}}),
        2,
    )?;
    handle(&result)?;
    assert_eq!(result["record"]["status"], "candidate");
    assert!(
        result["source_work"]["canonical_bytes"]
            .as_u64()
            .is_some_and(|n| n > 65_536)
    );
    assert!(result.to_string().len() <= 16_384);
    assert!(!result.to_string().contains("unselected_notes"));
    Ok(())
}
