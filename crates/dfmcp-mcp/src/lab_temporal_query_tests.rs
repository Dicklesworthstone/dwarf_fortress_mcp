//! Actual shared baseline engine with a complete laboratory packet publisher.
//! Owning implementation bead: df-lab-structured-query-f7q.

use super::*;
use crate::agent_turn::{AgentPhase, AgentTurnBuilder, empty_active_work};
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, Digest32, EntityId, FortressId, GameTick, ObservationCursor,
    RequestId, WorkBudget,
};
use dfmcp_world::{
    EntityKind, EntityRecord, Fact, FactPresence, FactSource, Value as WorldValue, WorldGraph,
};
use std::sync::atomic::{AtomicU64, Ordering};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
static NEXT_SESSION: AtomicU64 = AtomicU64::new(940_000_000);

struct Fixture {
    snapshot: WorldSnapshot,
    context: OperationContext,
    _guard: SessionGuard,
}

fn known(value: WorldValue, tick: GameTick) -> Fact {
    Fact::known(
        value,
        tick,
        FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
        Digest32::ZERO,
    )
}

fn fixture() -> Fixture {
    let tick = GameTick(10);
    let mut graph = WorldGraph::default();
    for id in 1..=4 {
        graph.entities.insert(
            EntityId::new(id),
            EntityRecord {
                id: EntityId::new(id),
                generation: 1,
                revision: 1,
                kind: EntityKind::Unit,
                label: format!("Urist {id}"),
                fields: BTreeMap::from([
                    ("alive".to_owned(), known(WorldValue::Bool(true), tick)),
                    ("stock".to_owned(), known(WorldValue::U64(id), tick)),
                ]),
            },
        );
    }
    let snapshot = WorldSnapshot::new(
        FortressId::new(741),
        tick,
        ObservationCursor {
            epoch: 1,
            sequence: 1,
        },
        true,
        graph,
    );
    let session = SessionId::new(u128::from(NEXT_SESSION.fetch_add(1, Ordering::Relaxed)));
    let context = OperationContext {
        session_id: session,
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget {
            max_wall_millis: 60_000,
            max_entities: 128,
            max_bytes: 1024 * 1024,
            max_output_tokens: 65_536,
            max_game_ticks: 1000,
            max_actions: 1,
        },
        grants: vec![CapabilityGrant {
            capability: Capability::Query,
            scope: CapabilityScope {
                fortress_id: Some(snapshot.fortress_id),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        }],
        cancellation_requested: false,
    };
    Fixture {
        snapshot,
        context,
        _guard: SessionGuard::new(session),
    }
}

fn request(query: Value) -> TestResult<Request> {
    parse(
        &json!({"mode":"temporal", "request":{
            "schema":"dfmcp.query/1", "query":query
        }})
        .to_string(),
    )?
    .ok_or_else(|| "temporal request was not selected".into())
}

fn capture(key: &str) -> Value {
    json!({"kind":"capture", "key":key, "max_game_ticks":100,
        "select":{"kind":"entities", "kinds":["unit"], "fields":["alive","stock"]}})
}

fn render(mut value: Value, context: &OperationContext) -> Result<String> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| invariant("test payload"))?;
    let mut active = empty_active_work();
    active["query_baselines"] = object
        .remove("_query_baseline_work")
        .ok_or_else(|| invariant("test baseline catalog"))?;
    active["query_watches"] = object
        .remove("_query_watch_work")
        .ok_or_else(|| invariant("test retained watch catalog"))?;
    active["condition_watches"] = object
        .remove("_condition_watch_work")
        .ok_or_else(|| invariant("test active watch catalog"))?;
    Ok(AgentTurnBuilder::new("fortress.query", AgentPhase::Inspect)
        .session_id(context.session_id.to_string())
        .request_id(context.request_id.to_string())
        .anchor(super::super::anchor_json(&context.anchor))
        .active_work(active)
        .attach(value))
}

fn run(fixture: &Fixture, query: Value) -> TestResult<Value> {
    let request = request(query)?;
    let text = execute(
        &fixture.snapshot,
        &fixture.context,
        &request,
        64 * 1024,
        |value| render(value, &fixture.context),
    )?;
    Ok(serde_json::from_str(&text)?)
}

fn handle(value: &Value) -> TestResult<String> {
    Ok(value["captured"]["baseline"]
        .as_str()
        .ok_or("missing handle")?
        .to_owned())
}

fn update(fixture: &mut Fixture) -> TestResult {
    fixture.snapshot.tick = fixture
        .snapshot
        .tick
        .checked_add(1)
        .ok_or("tick overflow")?;
    fixture.snapshot.cursor = fixture
        .snapshot
        .cursor
        .checked_next()
        .ok_or("cursor overflow")?;
    fixture.snapshot.refresh_hash();
    fixture.context.anchor = fixture.snapshot.anchor();
    Ok(())
}

#[test]
fn temporal_wrapper_is_closed_and_never_selects_history_or_native_refresh() -> TestResult {
    for query in [
        capture("a"),
        json!({"kind":"changes","baseline":"qb1:a"}),
        json!({"kind":"baselines"}),
        json!({"kind":"release_baseline","baseline":"qb1:a"}),
    ] {
        request(query)?;
    }
    assert!(parse("summary")?.is_none());
    assert!(parse(r#"{"mode":"entities"}"#)?.is_none());
    for raw in [
        r#"{"mode":"temporal","mode":"temporal","request":{"schema":"dfmcp.query/1","query":{"kind":"baselines"}}}"#,
        r#"{"mode":"temporal","at":"abc","request":{"schema":"dfmcp.query/1","query":{"kind":"baselines"}}}"#,
        r#"{"mode":"temporal","request":{"schema":"dfmcp.query/1","schema":"dfmcp.query/1","query":{"kind":"baselines"}}}"#,
        r#"{"mode":"temporal","request":{"schema":"wrong","query":{"kind":"baselines"}}}"#,
        r#"{"mode":"temporal","request":{"schema":"dfmcp.query/1","query":{"kind":"baselines","kind":"baselines"}}}"#,
        r#"{"mode":"temporal","request":{"schema":"dfmcp.query/1","query":{"kind":"baselines","limit":1}}}"#,
        r#"{"mode":"temporal","request":{"schema":"dfmcp.query/1","query":{"kind":"capture","key":"a","key":"b","max_game_ticks":5,"select":{}}}}"#,
        r#"{"mode":"temporal","request":{"schema":"dfmcp.query/1","query":{"kind":"capture","key":"a","max_game_ticks":5,"select":{"kind":"entities","fields":[],"fields":["stock"]}}}}"#,
        r#"{"mode":"temporal","request":{"schema":"dfmcp.query/1","query":{"kind":"capture","key":"a","max_game_ticks":5,"select":{"kind":"entities","limit":1}}}}"#,
        r#"{"mode":"temporal","request":{"schema":"dfmcp.query/1","query":{"kind":"await_watch","watch":"x"}}}"#,
        r#"{"mode":"temporal","request":{"schema":"dfmcp.query/1","query":{"kind":"watch"}}}"#,
    ] {
        assert!(
            matches!(parse(raw), Err(error) if error.code == ErrorCode::InvalidRequest),
            "{raw}"
        );
    }
    assert!(
        matches!(parse(&"x".repeat(MAX_REQUEST_BYTES + 1)), Err(error)
        if error.code == ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn capture_replay_changes_and_release_keep_complete_resumable_work() -> TestResult {
    let mut fixture = fixture();
    let original = fixture.snapshot.anchor();
    let captured = run(&fixture, capture("starting-roster"))?;
    let baseline = handle(&captured)?;
    assert_eq!(captured["captured"]["rows"], 4);
    assert_eq!(captured["native_captures"], 0);
    assert_eq!(captured["advances_game"], false);
    assert_eq!(captured["durable"], false);
    assert_eq!(fixture.snapshot.anchor(), original);
    let work = &captured["agent_turn"]["active_work"]["query_baselines"];
    assert_eq!(work.as_array().map(Vec::len), Some(1));
    assert_eq!(work[0]["baseline"], baseline);
    let next = parse(
        work[0]["next_step"]["arguments"]["mode"]
            .as_str()
            .ok_or("next step")?,
    )?
    .ok_or("missing next request")?;
    assert_eq!(next.kind(), "changes");
    let repeat = run(&fixture, capture("starting-roster"))?;
    assert_eq!(repeat["reused"], true);
    assert_eq!(handle(&repeat)?, baseline);
    fixture
        .snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(1))
        .ok_or("unit")?
        .fields
        .insert("stock".to_owned(), known(WorldValue::U64(10), GameTick(11)));
    update(&mut fixture)?;
    let result = run(&fixture, json!({"kind":"changes","baseline":baseline}))?;
    assert_eq!(result["change_count"], 1);
    assert_eq!(result["changes"][0]["kind"], "changed_in_result");
    assert_eq!(result["changes"][0]["entity_id"], "1");
    assert_eq!(
        result["agent_turn"]["active_work"]["query_baselines"][0]["basis"],
        super::super::anchor_json(&original)
    );
    let listed = run(&fixture, json!({"kind":"baselines"}))?;
    assert_eq!(listed["baselines"].as_array().map(Vec::len), Some(1));
    let released = run(
        &fixture,
        json!({"kind":"release_baseline","baseline":baseline}),
    )?;
    assert_eq!(released["released"], true);
    assert_eq!(
        released["agent_turn"]["active_work"]["query_baselines"],
        json!([])
    );
    assert_eq!(
        run(
            &fixture,
            json!({"kind":"release_baseline","baseline":baseline})
        )?["released"],
        false
    );
    Ok(())
}

#[test]
fn declined_malformed_or_clipped_publication_never_creates_a_baseline() -> TestResult {
    let fixture = fixture();
    let input = request(capture("atomic"))?;
    assert!(
        matches!(execute(&fixture.snapshot, &fixture.context, &input, 64 * 1024, |_| {
        Err(exhausted("test render refused"))
    }), Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    assert_eq!(
        run(&fixture, json!({"kind":"baselines"}))?["baselines"],
        json!([])
    );
    for case in 0..6 {
        let result = execute(
            &fixture.snapshot,
            &fixture.context,
            &input,
            64 * 1024,
            |mut value| {
                if case == 0 {
                    return Ok(json!({"ok":false,"error":{"code":"budget_exceeded"}}).to_string());
                }
                if case == 1 {
                    value["captured"]["rows"] = json!(0);
                }
                let mut output: Value = serde_json::from_str(&render(value, &fixture.context)?)
                    .map_err(|_| invariant("test render"))?;
                if case == 2 {
                    output["agent_turn"]["active_work"]["query_baselines"] = json!([]);
                }
                if case == 3 {
                    output["agent_turn"]["anchor"]["sequence"] = json!(9);
                }
                if case == 4 {
                    output["agent_turn"]["session_id"] = json!("another-session");
                }
                if case == 5 {
                    output["agent_turn"]["request_id"] = json!("another-request");
                }
                Ok(output.to_string())
            },
        );
        assert!(
            matches!(result, Err(error) if error.code == ErrorCode::InternalInvariantViolation)
        );
        assert_eq!(
            run(&fixture, json!({"kind":"baselines"}))?["baselines"],
            json!([])
        );
    }
    assert_eq!(run(&fixture, capture("atomic"))?["reused"], false);
    Ok(())
}

#[test]
fn failed_release_or_whole_packet_budget_preserves_the_previous_baseline() -> TestResult {
    let fixture = fixture();
    let baseline = handle(&run(&fixture, capture("keep"))?)?;
    let input = request(json!({"kind":"release_baseline","baseline":baseline}))?;
    assert!(
        execute(
            &fixture.snapshot,
            &fixture.context,
            &input,
            64 * 1024,
            |_| { Err(exhausted("test refused release")) }
        )
        .is_err()
    );
    assert_eq!(
        run(&fixture, json!({"kind":"baselines"}))?["baselines"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    let input = request(capture("too-large"))?;
    assert!(
        matches!(execute(&fixture.snapshot, &fixture.context, &input, 64 * 1024, |value| {
        let mut output: Value = serde_json::from_str(&render(value, &fixture.context)?)
            .map_err(|_| invariant("test render"))?;
        output["excess"] = json!("x".repeat(300_000));
        Ok(output.to_string())
    }), Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    let catalog = retained_work(&fixture.snapshot, &fixture.context)?;
    assert_eq!(catalog.as_array().map(Vec::len), Some(1));
    assert_eq!(catalog[0]["baseline"], baseline);
    Ok(())
}

#[test]
fn epoch_change_and_expiry_are_discoverable_and_never_renew_retained_rows() -> TestResult {
    let mut fixture = fixture();
    let baseline = handle(&run(&fixture, capture("before-restore"))?)?;
    fixture.snapshot.cursor.epoch += 1;
    update(&mut fixture)?;
    let listed = run(&fixture, json!({"kind":"baselines"}))?;
    assert_eq!(listed["baselines"][0]["status"], "invalidated_epoch");
    let release =
        listed["agent_turn"]["active_work"]["query_baselines"][0]["next_step"]["arguments"]["mode"]
            .as_str()
            .ok_or("release next step")?;
    assert_eq!(
        parse(release)?.ok_or("missing temporal release")?.kind(),
        "release_baseline"
    );
    let changes = request(json!({"kind":"changes","baseline":baseline}))?;
    assert!(
        matches!(execute(&fixture.snapshot,&fixture.context,&changes,64 * 1024,|value| render(value,&fixture.context)),
        Err(error) if error.code == ErrorCode::StaleAnchor)
    );
    run(
        &fixture,
        json!({"kind":"release_baseline","baseline":baseline}),
    )?;
    let expired = handle(&run(&fixture, capture("before-expiry"))?)?;
    fixture.snapshot.tick = GameTick(111);
    update(&mut fixture)?;
    let listed = run(&fixture, json!({"kind":"baselines"}))?;
    assert_eq!(listed["baselines"][0]["status"], "expired");
    assert_eq!(listed["baselines"][0]["baseline"], expired);
    Ok(())
}

#[test]
fn session_scope_and_guard_cleanup_do_not_affect_another_owner() -> TestResult {
    let first = fixture();
    let second = fixture();
    let baseline = handle(&run(&first, capture("private"))?)?;
    let other = handle(&run(&second, capture("other"))?)?;
    let input = request(json!({"kind":"changes","baseline":baseline}))?;
    assert!(
        matches!(execute(&second.snapshot,&second.context,&input,64 * 1024,|value| render(value,&second.context)),
        Err(error) if error.code == ErrorCode::InvalidRequest)
    );
    assert_eq!(
        run(
            &second,
            json!({"kind":"release_baseline","baseline":baseline})
        )?["released"],
        false
    );
    let old_snapshot = first.snapshot.clone();
    let old_context = first.context.clone();
    drop(first);
    assert_eq!(retained_work(&old_snapshot, &old_context)?, json!([]));
    assert_eq!(
        retained_work(&second.snapshot, &second.context)?[0]["baseline"],
        other
    );
    Ok(())
}

#[test]
fn current_authority_cancellation_and_full_source_budget_fence_every_operation() -> TestResult {
    let mut fixture = fixture();
    let baseline = handle(&run(&fixture, capture("authorized"))?)?;
    let saved = fixture.context.clone();
    for case in 0..4 {
        fixture.context = saved.clone();
        match case {
            0 => fixture.context.grants.clear(),
            1 => fixture.context.grants[0].expires_at_tick = Some(GameTick(9)),
            2 => fixture.context.cancellation_requested = true,
            _ => fixture.context.budget.max_entities = 1,
        }
        for query in [
            json!({"kind":"baselines"}),
            capture("blocked"),
            json!({"kind":"changes","baseline":baseline}),
            json!({"kind":"release_baseline","baseline":baseline}),
        ] {
            let input = request(query)?;
            assert!(
                execute(
                    &fixture.snapshot,
                    &fixture.context,
                    &input,
                    64 * 1024,
                    |value| render(value, &fixture.context)
                )
                .is_err()
            );
        }
    }
    fixture.context = saved;
    assert_eq!(
        retained_work(&fixture.snapshot, &fixture.context)?
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    Ok(())
}

#[test]
fn paged_changes_preserve_unknowns_generations_and_exact_target_identity() -> TestResult {
    let mut fixture = fixture();
    let baseline = handle(&run(&fixture, capture("compare"))?)?;
    let unit = fixture
        .snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(1))
        .ok_or("unit")?;
    unit.generation = 2;
    let unit = fixture
        .snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(2))
        .ok_or("unit")?;
    unit.fields.get_mut("alive").ok_or("alive")?.presence =
        Some(FactPresence::Unknown("not captured".into()));
    fixture
        .snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(3))
        .ok_or("unit")?
        .label = "Changed Urist".into();
    update(&mut fixture)?;
    let query = json!({"kind":"changes","baseline":baseline,"limit":1});
    let first = run(&fixture, query.clone())?;
    assert_eq!(first["change_count"], 4);
    let mut changes = Vec::new();
    let mut page = first.clone();
    for _ in 0..4 {
        let rows = page["changes"].as_array().ok_or("changes")?;
        assert_eq!(rows.len(), 1);
        changes.extend(rows.iter().cloned());
        if page["continuation"].is_null() {
            break;
        }
        let mut next = query.clone();
        next["continuation"] = page["continuation"].clone();
        page = run(&fixture, next)?;
    }
    assert_eq!(changes.len(), 4);
    assert_eq!(changes[0]["kind"], "left_result");
    assert_eq!(changes[1]["kind"], "entered_result");
    assert_eq!(
        changes[2]["after"]["fields"]["alive"]["presence"],
        "unknown"
    );
    let mut stale = query;
    stale["continuation"] = first["continuation"].clone();
    update(&mut fixture)?;
    let stale = request(stale)?;
    assert!(
        matches!(execute(&fixture.snapshot,&fixture.context,&stale,64 * 1024,|value| render(value,&fixture.context)),
        Err(error) if error.code == ErrorCode::StaleAnchor)
    );
    Ok(())
}

#[test]
fn all_eight_retained_baselines_are_visible_and_capacity_reclaims_explicitly() -> TestResult {
    let fixture = fixture();
    let mut first = String::new();
    for n in 0..MAX_BASELINES {
        let result = run(&fixture, capture(&format!("slot-{n}")))?;
        if n == 0 {
            first = handle(&result)?;
        }
        assert_eq!(
            result["agent_turn"]["active_work"]["query_baselines"]
                .as_array()
                .map(Vec::len),
            Some(n + 1)
        );
    }
    let input = request(capture("overflow"))?;
    assert!(
        matches!(execute(&fixture.snapshot,&fixture.context,&input,64 * 1024,|value| render(value,&fixture.context)),
        Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    run(
        &fixture,
        json!({"kind":"release_baseline","baseline":first}),
    )?;
    assert_eq!(run(&fixture, capture("replacement"))?["reused"], false);
    assert_eq!(
        retained_work(&fixture.snapshot, &fixture.context)?
            .as_array()
            .map(Vec::len),
        Some(MAX_BASELINES)
    );
    Ok(())
}

#[test]
fn final_wall_refusal_cannot_publish_a_captured_root() -> TestResult {
    let mut fixture = fixture();
    fixture.context.budget.max_wall_millis = 10;
    let input = request(capture("late"))?;
    let result = execute(
        &fixture.snapshot,
        &fixture.context,
        &input,
        64 * 1024,
        |value| {
            let start = Instant::now();
            while start.elapsed() < Duration::from_millis(20) {
                std::hint::spin_loop();
            }
            render(value, &fixture.context)
        },
    );
    assert!(matches!(result, Err(error) if error.code == ErrorCode::BudgetExceeded));
    fixture.context.budget.max_wall_millis = 60_000;
    assert_eq!(
        retained_work(&fixture.snapshot, &fixture.context)?,
        json!([])
    );
    Ok(())
}

fn stock_condition(minimum: u64) -> Value {
    json!({"op":"field", "entity_id":"1", "generation":1, "field":"stock",
        "comparison":"ge", "value":{"type":"u64", "value":minimum}})
}

fn watch(key: &str, condition: Value) -> Value {
    json!({"kind":"watch", "key":key, "condition":condition,
        "deadline_tick":100, "poll_interval_ticks":1, "stable_observations":2})
}

fn watch_handle(value: &Value) -> TestResult<String> {
    Ok(value["record"]["watch"]
        .as_str()
        .ok_or("watch handle")?
        .to_owned())
}

#[test]
fn watch_wrapper_accepts_only_the_lab_condition_lifecycle_and_preserves_duplicate_checks()
-> TestResult {
    for query in [
        watch("stock", stock_condition(2)),
        watch(
            "units",
            json!({"op":"entity_count", "scope":"observed_projection",
            "kind":"unit", "predicate":{"op":"field", "field":"alive",
            "comparison":"eq", "value":{"type":"bool", "value":true}},
            "comparison":"ge", "value":4}),
        ),
        json!({"kind":"poll_watch", "watch":"watch:placeholder"}),
        json!({"kind":"poll_watches"}),
        json!({"kind":"poll_watches", "watches":["watch:placeholder"]}),
        json!({"kind":"watches"}),
        json!({"kind":"cancel_watch", "watch":"watch:placeholder"}),
        json!({"kind":"release_watch", "watch":"watch:placeholder"}),
    ] {
        request(query)?;
    }
    for condition in [
        json!({"op":"item_quantity", "scope":"observed_projection", "quantity_unit":"stack_units",
            "predicate":{"op":"always"}, "comparison":"ge", "value":1}),
        json!({"op":"furniture_set", "targets":[], "test":"complete"}),
        json!({"op":"terrain_count", "areas":[], "predicate":{"op":"always"}, "comparison":"ge", "value":1}),
        json!({"op":"entity_count", "scope":"observed_projection", "kind":"unit",
            "predicate":{"op":"related", "entity_id":"1", "generation":1,
                "relation":"contains", "direction":"outgoing"}, "comparison":"ge", "value":1}),
    ] {
        assert!(request(watch("unsupported", condition)).is_err());
    }
    for raw in [
        r#"{"mode":"temporal","request":{"schema":"dfmcp.query/1","query":{"kind":"watch","key":"duplicate","condition":{"op":"paused","value":true,"value":false},"deadline_tick":100}}}"#,
        r#"{"mode":"temporal","request":{"schema":"dfmcp.query/1","query":{"kind":"watch","key":"duplicate","condition":{"op":"field","entity_id":"1","generation":1,"field":"stock","comparison":"ge","value":{"type":"u64","value":1,"value":2}},"deadline_tick":100}}}"#,
        r#"{"mode":"temporal","request":{"schema":"dfmcp.query/1","query":{"kind":"watches","limit":1}}}"#,
        r#"{"mode":"temporal","request":{"schema":"dfmcp.query/1","query":{"kind":"poll_watches","watches":[],"watches":null}}}"#,
        r#"{"mode":"temporal","request":{"schema":"dfmcp.query/1","query":{"kind":"await_watches"}}}"#,
    ] {
        assert!(
            matches!(parse(raw), Err(error) if error.code == ErrorCode::InvalidRequest),
            "{raw}"
        );
    }
    Ok(())
}

#[test]
fn stock_watch_samples_explicitly_and_preserves_both_baseline_and_watch_catalogs() -> TestResult {
    let mut fixture = fixture();
    let baseline = handle(&run(&fixture, capture("before"))?)?;
    let created = run(&fixture, watch("stock-growth", stock_condition(2)))?;
    let watch = watch_handle(&created)?;
    assert_eq!(created["record"]["status"], "waiting");
    assert_eq!(created["record"]["sample_count"], 1);
    assert_eq!(
        created["agent_turn"]["active_work"]["query_baselines"][0]["baseline"],
        baseline
    );
    let descriptor = &created["agent_turn"]["active_work"]["condition_watches"][0];
    assert_eq!(descriptor["watch"], watch);
    assert!(descriptor["next_step"]["arguments"].get("query").is_none());
    assert_eq!(
        parse(
            descriptor["next_step"]["arguments"]["mode"]
                .as_str()
                .ok_or("mode")?
        )?
        .ok_or("next request")?
        .kind(),
        "poll_watch"
    );
    fixture
        .snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(1))
        .ok_or("unit")?
        .fields
        .insert("stock".into(), known(WorldValue::U64(5), GameTick(11)));
    update(&mut fixture)?;
    let listed = run(&fixture, json!({"kind":"baselines"}))?;
    assert_eq!(
        listed["agent_turn"]["active_work"]["condition_watches"][0]["status"],
        "waiting"
    );
    assert_eq!(
        listed["agent_turn"]["active_work"]["condition_watches"][0]["needs_poll"],
        true
    );
    let source = fixture.snapshot.anchor();
    let sampled = run(&fixture, json!({"kind":"poll_watch", "watch":watch}))?;
    assert_eq!(sampled["record"]["status"], "candidate");
    assert_eq!(sampled["record"]["stable_observations"], 1);
    let repeated = run(&fixture, json!({"kind":"poll_watch", "watch":watch}))?;
    assert_eq!(repeated["record"], sampled["record"]);
    assert_eq!(fixture.snapshot.anchor(), source);
    update(&mut fixture)?;
    let terminal = run(&fixture, json!({"kind":"poll_watch", "watch":watch}))?;
    assert_eq!(terminal["record"]["status"], "satisfied");
    assert_eq!(
        terminal["agent_turn"]["active_work"]["condition_watches"],
        json!([])
    );
    assert_eq!(
        terminal["agent_turn"]["active_work"]["query_watches"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    let release =
        terminal["agent_turn"]["active_work"]["query_watches"][0]["next_step"]["arguments"]["mode"]
            .as_str()
            .ok_or("release mode")?;
    assert_eq!(
        parse(release)?.ok_or("release request")?.kind(),
        "release_watch"
    );
    assert_eq!(
        run(&fixture, json!({"kind":"release_watch", "watch":watch}))?["released"],
        true
    );
    assert_eq!(
        retained_watch_work(&fixture.context)?["query_watches"],
        json!([])
    );
    assert_eq!(
        retained_work(&fixture.snapshot, &fixture.context)?[0]["baseline"],
        baseline
    );
    Ok(())
}

#[test]
fn source_policy_unknowns_and_failure_guards_cannot_become_stable_lab_success() -> TestResult {
    for case in 0..6 {
        let mut fixture = fixture();
        let fact = fixture
            .snapshot
            .graph
            .entities
            .get_mut(&EntityId::new(1))
            .ok_or("unit")?
            .fields
            .get_mut("stock")
            .ok_or("stock")?;
        match case {
            0 => fact.source = FactSource::Replay,
            1 => fact.source = FactSource::Derived("unregistered-model/1".into()),
            2 => fact.source_digest = Digest32::of_bytes(b"untrusted-producer"),
            3 => fact.source = FactSource::DfhackField("stock".into()),
            4 => fact.observed_at = GameTick(99),
            _ => fact.presence = Some(FactPresence::Redacted("private".into())),
        }
        fixture.snapshot.refresh_hash();
        fixture.context.anchor = fixture.snapshot.anchor();
        let result = run(&fixture, watch("unknown", stock_condition(1)))?;
        assert_eq!(
            result["record"]["status"], "blocked_unknown",
            "case {case}: {result}"
        );
        assert_eq!(result["record"]["stable_observations"], 0);
    }
    let mut fixture = fixture();
    let mut input = watch("guarded", json!({"op":"paused", "value":true}));
    input["failure_condition"] = stock_condition(10);
    let watch = watch_handle(&run(&fixture, input)?)?;
    fixture
        .snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(1))
        .ok_or("unit")?
        .fields
        .get_mut("stock")
        .ok_or("stock")?
        .presence = Some(FactPresence::Unknown("lost".into()));
    update(&mut fixture)?;
    let result = run(&fixture, json!({"kind":"poll_watch", "watch":watch}))?;
    assert_eq!(result["record"]["status"], "blocked_unknown");
    assert_eq!(result["record"]["stable_observations"], 0);
    assert_eq!(
        result["record"]["evaluation"]["failure_condition"],
        "unknown"
    );
    Ok(())
}

#[test]
fn discarded_or_clipped_watch_publication_cannot_change_either_registry() -> TestResult {
    let fixture = fixture();
    let baseline = handle(&run(&fixture, capture("retained"))?)?;
    let input = request(watch("atomic", stock_condition(2)))?;
    for case in 0..4 {
        let result = execute(
            &fixture.snapshot,
            &fixture.context,
            &input,
            64 * 1024,
            |value| {
                if case == 0 {
                    return Err(exhausted("publisher rejected watch"));
                }
                let mut output: Value = serde_json::from_str(&render(value, &fixture.context)?)
                    .map_err(|_| invariant("test output"))?;
                let key = match case {
                    1 => "query_baselines",
                    2 => "query_watches",
                    _ => "condition_watches",
                };
                output["agent_turn"]["active_work"][key] = json!([]);
                Ok(output.to_string())
            },
        );
        assert!(result.is_err());
        assert_eq!(
            retained_watch_work(&fixture.context)?["query_watches"],
            json!([])
        );
        assert_eq!(
            retained_work(&fixture.snapshot, &fixture.context)?[0]["baseline"],
            baseline
        );
    }
    let created = run(&fixture, watch("atomic", stock_condition(2)))?;
    let watch = watch_handle(&created)?;
    let input = request(json!({"kind":"cancel_watch", "watch":watch}))?;
    assert!(
        execute(
            &fixture.snapshot,
            &fixture.context,
            &input,
            64 * 1024,
            |_| Err(exhausted("cancel refused"))
        )
        .is_err()
    );
    assert_eq!(
        run(&fixture, json!({"kind":"poll_watch", "watch":watch}))?["record"]["status"],
        "waiting"
    );
    run(&fixture, json!({"kind":"cancel_watch", "watch":watch}))?;
    let input = request(json!({"kind":"release_watch", "watch":watch}))?;
    assert!(
        execute(
            &fixture.snapshot,
            &fixture.context,
            &input,
            64 * 1024,
            |_| Err(exhausted("release refused"))
        )
        .is_err()
    );
    assert_eq!(
        retained_watch_work(&fixture.context)?["query_watches"][0]["status"],
        "cancelled"
    );
    let input = request(json!({"kind":"release_baseline", "baseline":baseline}))?;
    assert!(
        execute(
            &fixture.snapshot,
            &fixture.context,
            &input,
            64 * 1024,
            |value| {
                let mut output: Value = serde_json::from_str(&render(value, &fixture.context)?)
                    .map_err(|_| invariant("test output"))?;
                output["agent_turn"]["active_work"]["query_watches"] = json!([]);
                Ok(output.to_string())
            }
        )
        .is_err()
    );
    assert_eq!(
        retained_work(&fixture.snapshot, &fixture.context)?[0]["baseline"],
        baseline
    );
    Ok(())
}

#[test]
fn batched_watch_publication_is_all_or_none_and_unchanged_sources_are_not_resampled() -> TestResult
{
    let mut fixture = fixture();
    let first = watch_handle(&run(
        &fixture,
        watch("first", json!({"op":"tick_at_least", "value":11})),
    )?)?;
    let second = watch_handle(&run(
        &fixture,
        watch("second", json!({"op":"tick_at_least", "value":11})),
    )?)?;
    let prior = retained_watch_work(&fixture.context)?;
    update(&mut fixture)?;
    let query = json!({"kind":"poll_watches", "watches":[first,second]});
    let input = request(query.clone())?;
    assert!(
        execute(
            &fixture.snapshot,
            &fixture.context,
            &input,
            64 * 1024,
            |_| Err(exhausted("batch publication refused"))
        )
        .is_err()
    );
    let unchanged = retained_watch_work(&fixture.context)?;
    for (before, after) in prior["query_watches"]
        .as_array()
        .ok_or("prior")?
        .iter()
        .zip(unchanged["query_watches"].as_array().ok_or("after")?)
    {
        assert_eq!(before["evidence_digest"], after["evidence_digest"]);
        assert_eq!(after["status"], "waiting");
    }
    let sampled = run(&fixture, query.clone())?;
    assert_eq!(sampled["sampled"], 2);
    assert_eq!(sampled["atomic_watch_publication"], true);
    assert_eq!(sampled["records"].as_array().map(Vec::len), Some(2));
    assert_eq!(run(&fixture, query.clone())?["sampled"], 0);
    update(&mut fixture)?;
    let finished = run(&fixture, query)?;
    assert_eq!(finished["all_satisfied"], true);
    assert_eq!(
        finished["agent_turn"]["active_work"]["condition_watches"],
        json!([])
    );
    assert_eq!(
        finished["agent_turn"]["active_work"]["query_watches"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
    Ok(())
}

#[test]
fn watch_session_cleanup_and_epoch_changes_preserve_other_owners() -> TestResult {
    let first = fixture();
    let mut second = fixture();
    let private = watch_handle(&run(&first, watch("private", stock_condition(2)))?)?;
    let other = watch_handle(&run(&second, watch("other", stock_condition(2)))?)?;
    assert!(run(&second, json!({"kind":"poll_watch", "watch":private})).is_err());
    let old_context = first.context.clone();
    drop(first);
    assert_eq!(
        retained_watch_work(&old_context)?["query_watches"],
        json!([])
    );
    assert_eq!(
        retained_watch_work(&second.context)?["query_watches"][0]["watch"],
        other
    );
    second.snapshot.cursor.epoch += 1;
    update(&mut second)?;
    let result = run(&second, json!({"kind":"poll_watch", "watch":other}))?;
    assert_eq!(result["record"]["status"], "invalidated");
    assert_eq!(
        result["agent_turn"]["active_work"]["condition_watches"],
        json!([])
    );
    Ok(())
}
