//! Baseline lifecycle through the actual default laboratory tool facade.

use super::*;
use dfmcp_world::{
    EntityKind, EntityRecord, Fact, FactPresence, FactSource, Value as WorldValue, WorldGraph,
};
use serde_json::Value;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct TestSession {
    id: String,
}

impl Drop for TestSession {
    fn drop(&mut self) {
        if let Ok(number) = u128::from_str_radix(&self.id, 16) {
            sessions().remove(&SessionId::new(number));
        }
    }
}

fn parsed(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn open(scenario: &str) -> TestResult<TestSession> {
    let value = parsed(&crate::agent_facade::fortress_open_session(
        Some(true),
        None,
        Some(vec![
            ("observe".to_owned(), "read_only".to_owned()),
            ("query".to_owned(), "read_only".to_owned()),
            ("plan".to_owned(), "reversible".to_owned()),
            ("configure_labor".to_owned(), "reversible".to_owned()),
            ("control_clock".to_owned(), "reversible".to_owned()),
            ("checkpoint".to_owned(), "guarded".to_owned()),
            ("restore".to_owned(), "guarded".to_owned()),
        ]),
        Some(60_000),
        Some(2_000),
        Some(8_192),
        Some(4 * 1024 * 1024),
        Some(16_384),
        Some(64),
        Some(scenario.to_owned()),
        Some(false),
        Some(false),
    ))?;
    assert_eq!(value["ok"], true, "{value}");
    Ok(TestSession {
        id: value["session_id"]
            .as_str()
            .ok_or("session ID missing")?
            .to_owned(),
    })
}

fn known(value: WorldValue, tick: GameTick) -> Fact {
    Fact::known(
        value,
        tick,
        FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
        Digest32::of_bytes(b"temporal-public-fixture"),
    )
}

fn fixture() -> TestResult<TestSession> {
    let owner = open("empty")?;
    let session = resolve_session(Some(owner.id.clone()))?;
    let mut guard = session.lock().map_err(|_| "session poisoned")?;
    let mut snapshot = guard.adapter.snapshot().clone();
    snapshot.tick = GameTick(10);
    snapshot.cursor = snapshot.cursor.checked_next().ok_or("cursor exhausted")?;
    snapshot.graph = WorldGraph::default();
    for number in 1..=3 {
        snapshot.graph.entities.insert(
            EntityId::new(number),
            EntityRecord {
                id: EntityId::new(number),
                generation: 1,
                revision: 1,
                kind: EntityKind::Unit,
                label: format!("Urist {number}"),
                fields: BTreeMap::from([(
                    "stock".to_owned(),
                    known(WorldValue::U64(number), snapshot.tick),
                )]),
            },
        );
    }
    snapshot.refresh_hash();
    guard.adapter.inject_snapshot(snapshot)?;
    drop(guard);
    Ok(owner)
}

fn snapshot(id: &str) -> TestResult<WorldSnapshot> {
    Ok(resolve_session(Some(id.to_owned()))?
        .lock()
        .map_err(|_| "session poisoned")?
        .adapter
        .snapshot()
        .clone())
}

fn mutate(
    id: &str,
    edit: impl FnOnce(&mut WorldSnapshot) -> TestResult,
) -> TestResult<WorldSnapshot> {
    let session = resolve_session(Some(id.to_owned()))?;
    let mut guard = session.lock().map_err(|_| "session poisoned")?;
    remember_version(&mut guard);
    let mut current = guard.adapter.snapshot().clone();
    current.tick = GameTick(current.tick.0 + 1);
    current.cursor = current.cursor.checked_next().ok_or("cursor exhausted")?;
    edit(&mut current)?;
    current.refresh_hash();
    guard.adapter.inject_snapshot(current.clone())?;
    Ok(current)
}

fn envelope(query: Value) -> Value {
    json!({"mode":"temporal","request":{"schema":"dfmcp.query/1","query":query}})
}

fn call(id: &str, input: Value) -> TestResult<Value> {
    parsed(&crate::agent_facade::fortress_query(
        Some(id.to_owned()),
        Some(input.to_string()),
    ))
}

fn query(id: &str, input: Value) -> TestResult<Value> {
    call(id, envelope(input))
}

fn capture(key: &str, fields: &[&str]) -> Value {
    json!({"kind":"capture","key":key,"max_game_ticks":100,
        "select":{"kind":"entities","kinds":["unit"],"fields":fields}})
}

fn handle(value: &Value) -> TestResult<String> {
    assert_eq!(value["ok"], true, "{value}");
    Ok(value["captured"]["baseline"]
        .as_str()
        .ok_or("baseline handle missing")?
        .to_owned())
}

fn refusal(value: &Value, code: &str) {
    assert_eq!(value["ok"], false, "{value}");
    assert_eq!(value["error"]["code"], code, "{value}");
    assert!(value.get("captured").is_none(), "{value}");
    assert!(value.get("changes").is_none(), "{value}");
    assert!(
        value.get("continuation").is_none_or(Value::is_null),
        "{value}"
    );
    if value["output_budget"]["tier"] == "refused" {
        assert_eq!(value["error"]["code"], "budget_exceeded", "{value}");
    } else {
        assert!(value["agent_turn"].is_object(), "{value}");
    }
}

#[test]
fn lab_temporal_public_capture_plan_commit_changes_release_keeps_queries_effect_free() -> TestResult
{
    let _serial = crate::test_serial();
    let owner = open("starter_fortress")?;
    let before = snapshot(&owner.id)?;
    let current = before
        .graph
        .entities
        .get(&EntityId::new(1001))
        .and_then(|entity| entity.fields.get("labor.MINING"))
        .is_some_and(|fact| fact.value == WorldValue::Bool(true));
    let captured = query(&owner.id, capture("labor-before", &["labor.MINING"]))?;
    let baseline = handle(&captured)?;
    assert_eq!(captured["execution"], "foreground_only");
    assert_eq!(captured["durable"], false);
    assert_eq!(captured["native_captures"], 0);
    assert_eq!(snapshot(&owner.id)?.anchor(), before.anchor());
    let planned = parsed(&crate::agent_facade::fortress_plan(
        Some(owner.id.clone()),
        None,
        None,
        Some(
            json!([{"action":{"kind":"set_labor","units":["1001"],
            "labor":"MINING","enabled":!current}}])
            .to_string(),
        ),
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let committed = parsed(&crate::agent_facade::fortress_commit(
        Some(owner.id.clone()),
        planned["plan_digest"]
            .as_str()
            .ok_or("plan digest missing")?
            .to_owned(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let after = snapshot(&owner.id)?;
    let changes = query(&owner.id, json!({"kind":"changes","baseline":baseline}))?;
    assert_eq!(changes["ok"], true, "{changes}");
    assert_eq!(changes["change_count"], 1, "{changes}");
    assert_eq!(changes["changes"][0]["entity_id"], "1001");
    assert_eq!(changes["changes"][0]["kind"], "changed_in_result");
    assert_eq!(
        changes["changes"][0]["after"]["fields"]["labor.MINING"]["value"]["value"],
        !current
    );
    assert_eq!(changes["basis"], anchor_json(&before.anchor()));
    assert_eq!(
        changes["agent_turn"]["anchor"],
        anchor_json(&after.anchor())
    );
    assert_eq!(changes["agent_turn"]["continuity"]["status"], "partial");
    assert_eq!(
        changes["agent_turn"]["continuity"]["basis"],
        changes["basis"]
    );
    assert_eq!(
        changes["agent_turn"]["changes"][0]["epistemic_state"],
        "inferred"
    );
    assert_eq!(changes["baseline_advanced"], false);
    assert_eq!(changes["mutation_authority"], false);
    let released = query(
        &owner.id,
        json!({"kind":"release_baseline","baseline":baseline}),
    )?;
    assert_eq!(released["released"], true, "{released}");
    assert_eq!(
        released["agent_turn"]["active_work"]["query_baselines"],
        json!([])
    );
    assert_eq!(
        query(
            &owner.id,
            json!({"kind":"release_baseline","baseline":baseline})
        )?["released"],
        false
    );
    assert_eq!(snapshot(&owner.id)?.anchor(), after.anchor());
    Ok(())
}

#[test]
fn lab_temporal_public_catalog_recovers_handle_and_capture_keys_never_replace_state() -> TestResult
{
    let _serial = crate::test_serial();
    let owner = fixture()?;
    let request = capture("recover-after-context-loss", &["stock"]);
    let captured = query(&owner.id, request.clone())?;
    let baseline = handle(&captured)?;
    let repeated = query(&owner.id, request.clone())?;
    assert_eq!(repeated["reused"], true);
    assert_eq!(handle(&repeated)?, baseline);
    let listed = query(&owner.id, json!({"kind":"baselines"}))?;
    assert_eq!(listed["baselines"].as_array().map(Vec::len), Some(1));
    assert_eq!(listed["baselines"][0]["baseline"], baseline);
    let next = &listed["agent_turn"]["active_work"]["query_baselines"][0]["next_step"];
    assert_eq!(next["tool"], "fortress.query");
    assert_eq!(next["arguments"]["session_id"], owner.id);
    let resumed = parsed(&crate::agent_facade::fortress_query(
        next["arguments"]["session_id"].as_str().map(str::to_owned),
        next["arguments"]["mode"].as_str().map(str::to_owned),
    ))?;
    assert_eq!(resumed["ok"], true, "{resumed}");
    assert_eq!(resumed["change_count"], 0);
    assert_eq!(resumed["agent_turn"]["continuity"]["status"], "heartbeat");
    let mut conflicting = request.clone();
    conflicting["select"]["fields"] = json!([]);
    refusal(&query(&owner.id, conflicting)?, "conflict");
    let old = snapshot(&owner.id)?;
    mutate(&owner.id, |_| Ok(()))?;
    refusal(&query(&owner.id, request)?, "conflict");
    let mut stale = envelope(capture("must-not-capture-wrong-target", &["stock"]));
    stale["request"]["expected_anchor"] = anchor_json(&old.anchor());
    refusal(&call(&owner.id, stale)?, "stale_anchor");
    let listed = query(&owner.id, json!({"kind":"baselines"}))?;
    assert_eq!(listed["baselines"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        listed["baselines"][0]["basis"],
        captured["captured"]["basis"]
    );
    Ok(())
}

#[test]
fn lab_temporal_public_pages_preserve_generations_unknowns_and_target_identity() -> TestResult {
    let _serial = crate::test_serial();
    let owner = fixture()?;
    let baseline = handle(&query(&owner.id, capture("pages", &["stock"]))?)?;
    let target = mutate(&owner.id, |snapshot| {
        snapshot
            .graph
            .entities
            .get_mut(&EntityId::new(1))
            .ok_or("unit missing")?
            .generation = 2;
        let field = snapshot
            .graph
            .entities
            .get_mut(&EntityId::new(2))
            .ok_or("unit missing")?
            .fields
            .get_mut("stock")
            .ok_or("stock missing")?;
        field.presence = Some(FactPresence::Unknown("not captured".to_owned()));
        snapshot
            .graph
            .entities
            .get_mut(&EntityId::new(3))
            .ok_or("unit missing")?
            .label = "Changed Urist".to_owned();
        Ok(())
    })?;
    let request = json!({"kind":"changes","baseline":baseline,"limit":1});
    let first = query(&owner.id, request.clone())?;
    assert_eq!(first["ok"], true, "{first}");
    assert_eq!(first["change_count"], 4, "{first}");
    assert_eq!(first["source_work"]["records"], 3);
    let mut rows = Vec::new();
    let mut page = first.clone();
    for _ in 0..4 {
        let selected = page["changes"].as_array().ok_or("changes missing")?;
        assert_eq!(selected.len(), 1);
        rows.extend(selected.iter().cloned());
        if page["continuation"].is_null() {
            break;
        }
        let mut next = request.clone();
        next["continuation"] = page["continuation"].clone();
        page = query(&owner.id, next.clone())?;
        assert_eq!(page["ok"], true, "{page}");
        assert_eq!(page["changes"], query(&owner.id, next)?["changes"]);
    }
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0]["kind"], "left_result");
    assert_eq!(rows[0]["generation"], 1);
    assert_eq!(rows[1]["kind"], "entered_result");
    assert_eq!(rows[1]["generation"], 2);
    assert_eq!(rows[2]["after"]["fields"]["stock"]["presence"], "unknown");
    assert!(rows[2]["after"]["fields"]["stock"]["value"].is_null());
    assert_eq!(snapshot(&owner.id)?.anchor(), target.anchor());
    mutate(&owner.id, |_| Ok(()))?;
    let mut stale = request;
    stale["continuation"] = first["continuation"].clone();
    refusal(&query(&owner.id, stale)?, "stale_anchor");
    Ok(())
}

#[test]
fn lab_temporal_public_session_ownership_and_drop_reclaim_only_private_baselines() -> TestResult {
    let _serial = crate::test_serial();
    let first = fixture()?;
    let second = fixture()?;
    let baseline = handle(&query(&first.id, capture("private", &["stock"]))?)?;
    let other = handle(&query(&second.id, capture("other", &["stock"]))?)?;
    refusal(
        &query(&second.id, json!({"kind":"changes","baseline":baseline}))?,
        "invalid_request",
    );
    assert_eq!(
        query(
            &second.id,
            json!({"kind":"release_baseline","baseline":baseline})
        )?["released"],
        false
    );
    assert_eq!(
        query(&first.id, json!({"kind":"baselines"}))?["baselines"][0]["baseline"],
        baseline
    );
    let retained = resolve_session(Some(first.id.clone()))?;
    let (source, context) = {
        let guard = retained.lock().map_err(|_| "session poisoned")?;
        (guard.adapter.snapshot().clone(), read_context(&guard))
    };
    drop(first);
    drop(retained);
    assert_eq!(temporal_query::retained_work(&source, &context)?, json!([]));
    assert_eq!(
        query(&second.id, json!({"kind":"baselines"}))?["baselines"][0]["baseline"],
        other
    );
    Ok(())
}

#[test]
fn lab_temporal_public_current_query_grants_and_source_budget_fence_every_operation() -> TestResult
{
    let _serial = crate::test_serial();
    let owner = fixture()?;
    let baseline = handle(&query(&owner.id, capture("authorized", &["stock"]))?)?;
    let session = resolve_session(Some(owner.id.clone()))?;
    let (grants, budget) = {
        let guard = session.lock().map_err(|_| "session poisoned")?;
        (guard.grants.clone(), guard.budget)
    };
    for case in 0..5 {
        {
            let mut guard = session.lock().map_err(|_| "session poisoned")?;
            guard.grants = grants.clone();
            guard.budget = budget;
            let query_grant = guard
                .grants
                .iter_mut()
                .find(|grant| grant.capability == Capability::Query)
                .ok_or("query grant missing")?;
            match case {
                0 => query_grant.capability = Capability::Observe,
                1 => query_grant.expires_at_tick = Some(GameTick(9)),
                2 => query_grant.remaining_uses = Some(0),
                3 => {
                    query_grant.scope.entity_ids.insert(EntityId::new(1));
                }
                _ => guard.budget.max_entities = 1,
            }
        }
        for input in [
            json!({"kind":"baselines"}),
            capture("forbidden", &["stock"]),
            json!({"kind":"changes","baseline":baseline}),
            json!({"kind":"release_baseline","baseline":baseline}),
        ] {
            refusal(
                &query(&owner.id, input)?,
                if case == 4 {
                    "budget_exceeded"
                } else {
                    "capability_denied"
                },
            );
        }
    }
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        guard.grants = grants;
        guard.budget = budget;
    }
    let listed = query(&owner.id, json!({"kind":"baselines"}))?;
    assert_eq!(listed["baselines"].as_array().map(Vec::len), Some(1));
    assert_eq!(listed["baselines"][0]["baseline"], baseline);
    Ok(())
}

#[test]
fn lab_temporal_public_packet_preserves_pending_plan_and_its_commit_authority() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("starter_fortress")?;
    let planned = parsed(&crate::agent_facade::fortress_plan(
        Some(owner.id.clone()),
        Some("resume the reviewed laboratory world".to_owned()),
        Some(false),
        None,
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let digest = planned["plan_digest"]
        .as_str()
        .ok_or("plan digest missing")?
        .to_owned();
    let captured = query(&owner.id, capture("pending-plan", &["labor.MINING"]))?;
    let baseline = handle(&captured)?;
    for result in [
        captured,
        query(&owner.id, json!({"kind":"baselines"}))?,
        query(&owner.id, json!({"kind":"changes","baseline":baseline}))?,
    ] {
        assert_eq!(result["ok"], true, "{result}");
        let original = &planned["agent_turn"]["active_work"]["pending_plans"][0];
        let pending = &result["agent_turn"]["active_work"]["pending_plans"][0];
        for key in ["plan_digest", "state"] {
            assert_eq!(pending[key], original[key]);
        }
        assert_eq!(
            pending["minimum_safe_next_step"]["tool"],
            original["minimum_safe_next_step"]["tool"]
        );
        assert_eq!(
            pending["minimum_safe_next_step"]["arguments"]["plan_digest"],
            original["minimum_safe_next_step"]["arguments"]["plan_digest"]
        );
        assert_eq!(
            result["agent_turn"]["active_work"]["pending_plans"][0]["plan_digest"],
            digest
        );
    }
    let committed = parsed(&crate::agent_facade::fortress_commit(
        Some(owner.id.clone()),
        digest,
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    assert!(!snapshot(&owner.id)?.paused);
    Ok(())
}

#[test]
fn lab_temporal_public_packet_budget_refusals_neither_capture_nor_release() -> TestResult {
    let _serial = crate::test_serial();
    let owner = fixture()?;
    let session = resolve_session(Some(owner.id.clone()))?;
    let budget = session.lock().map_err(|_| "session poisoned")?.budget;
    session
        .lock()
        .map_err(|_| "session poisoned")?
        .budget
        .max_output_tokens = 128;
    refusal(
        &query(&owner.id, capture("must-not-be-hidden", &["stock"]))?,
        "budget_exceeded",
    );
    session.lock().map_err(|_| "session poisoned")?.budget = budget;
    assert_eq!(
        query(&owner.id, json!({"kind":"baselines"}))?["baselines"],
        json!([])
    );
    let captured = query(&owner.id, capture("must-survive-refusal", &["stock"]))?;
    let baseline = handle(&captured)?;
    session
        .lock()
        .map_err(|_| "session poisoned")?
        .budget
        .max_output_tokens = 128;
    refusal(
        &query(
            &owner.id,
            json!({"kind":"release_baseline","baseline":baseline}),
        )?,
        "budget_exceeded",
    );
    session.lock().map_err(|_| "session poisoned")?.budget = budget;
    let listed = query(&owner.id, json!({"kind":"baselines"}))?;
    assert_eq!(listed["baselines"].as_array().map(Vec::len), Some(1));
    assert_eq!(listed["baselines"][0]["baseline"], baseline);
    assert_eq!(
        query(&owner.id, json!({"kind":"changes","baseline":baseline}))?["change_count"],
        0
    );
    Ok(())
}

#[test]
fn lab_temporal_public_checkpoint_restore_and_deadline_leave_discoverable_release_work()
-> TestResult {
    let _serial = crate::test_serial();
    let owner = fixture()?;
    let baseline = handle(&query(&owner.id, capture("before-restore", &["stock"]))?)?;
    let checkpoint = parsed(&crate::agent_facade::fortress_checkpoint(
        Some(owner.id.clone()),
        Some("temporal-restore".to_owned()),
    ))?;
    assert_eq!(checkpoint["ok"], true, "{checkpoint}");
    let restored = parsed(&crate::agent_facade::fortress_restore(
        Some(owner.id.clone()),
        checkpoint["checkpoint_id"]
            .as_str()
            .ok_or("checkpoint ID missing")?
            .to_owned(),
    ))?;
    assert_eq!(restored["ok"], true, "{restored}");
    let listed = query(&owner.id, json!({"kind":"baselines"}))?;
    assert_eq!(listed["ok"], true, "{listed}");
    assert_eq!(listed["baselines"][0]["status"], "invalidated_epoch");
    refusal(
        &query(&owner.id, json!({"kind":"changes","baseline":baseline}))?,
        "stale_anchor",
    );
    let release_mode =
        listed["agent_turn"]["active_work"]["query_baselines"][0]["next_step"]["arguments"]["mode"]
            .as_str()
            .ok_or("release mode missing")?;
    let released = parsed(&crate::agent_facade::fortress_query(
        Some(owner.id.clone()),
        Some(release_mode.to_owned()),
    ))?;
    assert_eq!(released["released"], true, "{released}");
    let captured = query(&owner.id, capture("before-deadline", &["stock"]))?;
    let deadline = captured["captured"]["expires_at_game_tick"]
        .as_u64()
        .ok_or("deadline missing")?;
    let baseline = handle(&captured)?;
    mutate(&owner.id, |snapshot| {
        snapshot.tick = GameTick(deadline);
        Ok(())
    })?;
    let listed = query(&owner.id, json!({"kind":"baselines"}))?;
    assert_eq!(listed["baselines"][0]["status"], "expired");
    refusal(
        &query(&owner.id, json!({"kind":"changes","baseline":baseline}))?,
        "stale_anchor",
    );
    assert_eq!(
        query(
            &owner.id,
            json!({"kind":"release_baseline","baseline":baseline})
        )?["released"],
        true
    );
    Ok(())
}

#[test]
fn lab_temporal_public_small_selection_keeps_large_source_budget_independent_of_output()
-> TestResult {
    let _serial = crate::test_serial();
    let owner = fixture()?;
    mutate(&owner.id, |snapshot| {
        for entity in snapshot.graph.entities.values_mut() {
            entity.fields.insert(
                "unselected_notes".to_owned(),
                known(WorldValue::Text("x".repeat(32_768)), snapshot.tick),
            );
        }
        Ok(())
    })?;
    let session = resolve_session(Some(owner.id.clone()))?;
    session
        .lock()
        .map_err(|_| "session poisoned")?
        .budget
        .max_output_tokens = 4_096;
    let captured = query(&owner.id, capture("large-source-small-result", &["stock"]))?;
    let baseline = handle(&captured)?;
    assert!(
        captured["source_work"]["canonical_bytes"]
            .as_u64()
            .is_some_and(|bytes| bytes > 65_536)
    );
    assert!(captured.to_string().len() <= 16_384);
    mutate(&owner.id, |snapshot| {
        snapshot
            .graph
            .entities
            .get_mut(&EntityId::new(1))
            .ok_or("unit missing")?
            .fields
            .insert(
                "stock".to_owned(),
                known(WorldValue::U64(99), snapshot.tick),
            );
        Ok(())
    })?;
    let changes = query(
        &owner.id,
        json!({"kind":"changes","baseline":baseline,"limit":1}),
    )?;
    assert_eq!(changes["ok"], true, "{changes}");
    assert_eq!(changes["change_count"], 1);
    assert_eq!(
        changes["changes"][0]["after"]["fields"]["stock"]["value"]["value"],
        99
    );
    assert!(changes.to_string().len() <= 16_384);
    assert!(!changes.to_string().contains("unselected_notes"));
    Ok(())
}

#[test]
fn lab_temporal_public_capture_marks_process_local_handle_transcripts_not_replayable() -> TestResult
{
    let _serial = crate::test_serial();
    let owner = open("starter_fortress")?;
    let session = resolve_session(Some(owner.id.clone()))?;
    {
        let guard = session.lock().map_err(|_| "session poisoned")?;
        assert_eq!(
            crate::replay::bundle_json(&guard.replay)["replayable"],
            true
        );
    }
    let baseline = handle(&query(
        &owner.id,
        capture("nonreplayable-handle", &["labor.MINING"]),
    )?)?;
    {
        let guard = session.lock().map_err(|_| "session poisoned")?;
        let bundle = crate::replay::bundle_json(&guard.replay);
        assert_eq!(bundle["replayable"], false, "{bundle}");
        let reason = bundle["not_replayable_reason"]
            .as_str()
            .ok_or("replay reason missing")?;
        assert!(reason.contains("query baselines"), "{reason}");
        assert!(reason.contains("process-local session handles"), "{reason}");
    }
    assert_eq!(
        query(
            &owner.id,
            json!({"kind":"release_baseline","baseline":baseline})
        )?["released"],
        true
    );
    let guard = session.lock().map_err(|_| "session poisoned")?;
    assert_eq!(
        crate::replay::bundle_json(&guard.replay)["replayable"],
        false
    );
    Ok(())
}

#[test]
fn lab_temporal_public_elapsed_admission_budget_cannot_capture_or_change_replayability()
-> TestResult {
    let _serial = crate::test_serial();
    let owner = fixture()?;
    let before = snapshot(&owner.id)?;
    let started = std::time::Instant::now()
        .checked_sub(std::time::Duration::from_secs(61))
        .ok_or("test start instant cannot be represented")?;
    let publication = fortress_query_publication(
        Some(owner.id.clone()),
        Some(envelope(capture("admission-budget-expired", &["stock"])).to_string()),
        started,
    );
    let QueryPublication::Pending(raw) = publication else {
        return Err("expired admission published a temporal success".into());
    };
    let failed = parsed(&raw)?;
    assert_eq!(failed["ok"], false, "{failed}");
    assert_eq!(failed["error"]["code"], "budget_exceeded", "{failed}");
    assert!(failed.get("captured").is_none());
    assert_eq!(
        query(&owner.id, json!({"kind":"baselines"}))?["baselines"],
        json!([])
    );
    assert_eq!(snapshot(&owner.id)?.anchor(), before.anchor());
    let session = resolve_session(Some(owner.id.clone()))?;
    let guard = session.lock().map_err(|_| "session poisoned")?;
    assert_eq!(
        crate::replay::bundle_json(&guard.replay)["replayable"],
        true
    );
    Ok(())
}
