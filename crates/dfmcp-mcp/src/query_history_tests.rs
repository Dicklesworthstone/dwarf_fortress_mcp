use super::execute_with_publisher;
use dfmcp_core::{
    Capability, DfmcpError, Digest32, ErrorCode, OperationContext, Result, RiskTier, SessionId,
};
use dfmcp_world::WorldSnapshot;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_PER_SESSION: usize = 8;
static NEXT_SESSION: AtomicU64 = AtomicU64::new(10_000_000);
fn invariant(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InternalInvariantViolation, message)
}
fn exhausted(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}
fn encode(value: &Value) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|_| invariant("test encoding"))
}
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, EntityId, FortressId, GameTick, ObservationCursor, RequestId,
    WorkBudget,
};
use dfmcp_world::{
    EntityKind, EntityRecord, Fact, FactPresence, FactSource, Value as WorldValue, WorldGraph,
};

fn fixture(count: u64) -> (WorldSnapshot, OperationContext) {
    let mut graph = WorldGraph::default();
    for id in 1..=count {
        graph.entities.insert(
            EntityId::new(id),
            EntityRecord {
                id: EntityId::new(id),
                generation: 1,
                revision: 1,
                kind: EntityKind::Unit,
                label: format!("citizen-{id}"),
                fields: BTreeMap::from([(
                    "sane".to_owned(),
                    Fact::known(
                        WorldValue::Bool(true),
                        GameTick(1),
                        FactSource::DfhackField("unit.sane".to_owned()),
                        Digest32::of_bytes(b"source"),
                    ),
                )]),
            },
        );
    }
    let snapshot = WorldSnapshot::new(
        FortressId::new(1),
        GameTick(1),
        ObservationCursor::ORIGIN,
        true,
        graph,
    );
    let context = OperationContext {
        session_id: SessionId::new(u128::from(NEXT_SESSION.fetch_add(2, Ordering::Relaxed))),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget {
            max_entities: 1024,
            max_bytes: 262144,
            max_output_tokens: 65536,
            max_game_ticks: 1000,
            ..WorkBudget::default()
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
    (snapshot, context)
}

fn capture(key: &str) -> Value {
    json!({"kind":"capture", "key":key, "max_game_ticks":100,
    "select":{"kind":"entities", "kinds":["unit"], "fields":["sane"]}})
}
fn run(world: &WorldSnapshot, context: &OperationContext, request: Value) -> Result<Value> {
    let input = json!({"schema":"dfmcp.query/1", "query":request});
    let output = execute_with_publisher(world, context, &input, |payload| {
        String::from_utf8(encode(&payload)?).map_err(|_| invariant("test encoding"))
    })?;
    serde_json::from_str(&output).map_err(|_| invariant("test response"))
}
fn retained(world: &WorldSnapshot, context: &OperationContext) -> Result<usize> {
    let result = run(world, context, json!({"kind":"baselines"}))?;
    result["baselines"]
        .as_array()
        .map(Vec::len)
        .ok_or_else(|| invariant("test baselines"))
}
fn handle(result: &Value) -> Result<String> {
    result["captured"]["baseline"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| invariant("test baseline"))
}
fn advance(world: &mut WorldSnapshot, ctx: &mut OperationContext) {
    world.cursor.sequence += 1;
    world.tick.0 += 1;
    world.refresh_hash();
    ctx.anchor = world.anchor();
}

#[test]
fn capture_finishes_multiple_pages_and_is_idempotent_at_the_same_anchor() -> Result<()> {
    let (world, ctx) = fixture(25);
    let first = run(&world, &ctx, capture("citizens"))?;
    assert_eq!(first["captured"]["rows"], 25);
    let again = run(&world, &ctx, capture("citizens"))?;
    assert_eq!(handle(&first)?, handle(&again)?);
    assert_eq!(again["reused"], true);
    assert_eq!(retained(&world, &ctx)?, 1);
    let page = run(
        &world,
        &ctx,
        json!({"kind":"changes", "baseline":handle(&first)?}),
    )?;
    assert_eq!(page["change_count"], 0);
    assert_eq!(page["anchor_advanced"], false);
    Ok(())
}

#[test]
fn field_changes_filtered_departures_and_generation_reuse_are_not_confused() -> Result<()> {
    let (mut world, mut ctx) = fixture(3);
    let id = handle(&run(&world, &ctx, capture("citizens"))?)?;
    world.graph.entities.remove(&EntityId::new(1));
    world
        .graph
        .entities
        .get_mut(&EntityId::new(2))
        .ok_or_else(|| invariant("fixture"))?
        .generation = 2;
    world
        .graph
        .entities
        .get_mut(&EntityId::new(3))
        .ok_or_else(|| invariant("fixture"))?
        .fields
        .insert(
            "sane".to_owned(),
            Fact::known(
                WorldValue::Bool(false),
                GameTick(2),
                FactSource::DfhackField("unit.sane".to_owned()),
                Digest32::ZERO,
            ),
        );
    advance(&mut world, &mut ctx);
    let result = run(&world, &ctx, json!({"kind":"changes", "baseline":id}))?;
    assert_eq!(result["change_count"], 4);
    assert_eq!(result["changes"][0]["kind"], "left_result");
    assert_eq!(result["changes"][1]["generation"], 1);
    assert_eq!(result["changes"][1]["kind"], "left_result");
    assert_eq!(result["changes"][2]["generation"], 2);
    assert_eq!(result["changes"][2]["kind"], "entered_result");
    assert_eq!(result["changes"][3]["kind"], "changed_in_result");
    assert_eq!(result["coverage"]["absence_proven"], false);
    assert_eq!(result["baseline_advanced"], false);
    Ok(())
}

#[test]
fn leaving_a_filter_is_not_a_deletion_and_unknown_is_a_material_change() -> Result<()> {
    let (mut world, mut ctx) = fixture(2);
    let mut selection = capture("sane");
    selection["select"]["where"] = json!({"op":"compare", "field":"sane", "comparison":"eq",
        "value":{"type":"bool", "value":true}});
    let id = handle(&run(&world, &ctx, selection)?)?;
    let all = handle(&run(&world, &ctx, capture("all"))?)?;
    world
        .graph
        .entities
        .get_mut(&EntityId::new(1))
        .ok_or_else(|| invariant("fixture"))?
        .fields
        .insert(
            "sane".to_owned(),
            Fact::with_presence(
                FactPresence::Unknown("read unavailable".to_owned()),
                GameTick(2),
                FactSource::Replay,
                Digest32::ZERO,
            ),
        );
    advance(&mut world, &mut ctx);
    let filtered = run(&world, &ctx, json!({"kind":"changes", "baseline":id}))?;
    assert_eq!(filtered["changes"][0]["kind"], "left_result");
    assert_eq!(world.graph.entities.len(), 2);
    let unfiltered = run(&world, &ctx, json!({"kind":"changes", "baseline":all}))?;
    assert_eq!(
        unfiltered["changes"][0]["after"]["fields"]["sane"]["presence"],
        "unknown"
    );
    Ok(())
}

#[test]
fn provenance_refreshes_do_not_flood_the_change_stream() -> Result<()> {
    let (mut world, mut ctx) = fixture(2);
    let id = handle(&run(&world, &ctx, capture("citizens"))?)?;
    let entity = world
        .graph
        .entities
        .get_mut(&EntityId::new(1))
        .ok_or_else(|| invariant("fixture"))?;
    entity.revision += 1;
    let fact = entity
        .fields
        .get_mut("sane")
        .ok_or_else(|| invariant("fixture"))?;
    fact.observed_at = GameTick(2);
    fact.source_digest = Digest32::of_bytes(b"new observation");
    advance(&mut world, &mut ctx);
    let result = run(&world, &ctx, json!({"kind":"changes", "baseline":id}))?;
    assert_eq!(result["change_count"], 0);
    assert_eq!(result["provenance_only_refreshes"], 1);
    assert_eq!(result["anchor_advanced"], true);
    Ok(())
}

#[test]
fn pages_and_retries_never_consume_the_baseline() -> Result<()> {
    let (mut world, mut ctx) = fixture(25);
    let id = handle(&run(&world, &ctx, capture("citizens"))?)?;
    for entity in world.graph.entities.values_mut() {
        entity.label.push_str(" renamed");
    }
    advance(&mut world, &mut ctx);
    let mut input = json!({"kind":"changes", "baseline":id, "limit":1});
    let mut ids = Vec::new();
    loop {
        let page = run(&world, &ctx, input.clone())?;
        assert_eq!(page, run(&world, &ctx, input.clone())?);
        ids.push(page["changes"][0]["entity_id"].clone());
        assert!(ids.len() <= 25);
        if page["continuation"].is_null() {
            break;
        }
        input["continuation"] = page["continuation"].clone();
    }
    assert_eq!(
        ids,
        (1..=25).map(|id| json!(id.to_string())).collect::<Vec<_>>()
    );
    assert_eq!(retained(&world, &ctx)?, 1);
    let old_page = run(
        &world,
        &ctx,
        json!({"kind":"changes", "baseline":id, "limit":1}),
    )?;
    advance(&mut world, &mut ctx);
    assert!(
        matches!(run(&world, &ctx, json!({"kind":"changes", "baseline":id,
        "continuation":old_page["continuation"]})), Err(error) if error.code == ErrorCode::StaleAnchor)
    );
    Ok(())
}

#[test]
fn session_restore_deadline_and_authority_boundaries_are_enforced() -> Result<()> {
    let (world, ctx) = fixture(2);
    let id = handle(&run(&world, &ctx, capture("citizens"))?)?;
    let input = json!({"kind":"changes", "baseline":id});
    for case in 0..6 {
        let mut changed = world.clone();
        let mut context = ctx.clone();
        match case {
            0 => context.session_id = SessionId::new(ctx.session_id.get() + 1),
            1 => changed.cursor.epoch += 1,
            2 => {
                changed.tick = GameTick(101);
                changed.cursor.sequence += 1;
            }
            3 => context.grants.clear(),
            4 => context.cancellation_requested = true,
            _ => changed.paused = false,
        }
        changed.refresh_hash();
        context.anchor = changed.anchor();
        assert!(run(&changed, &context, input.clone()).is_err());
    }
    let mut other = ctx.clone();
    other.session_id = SessionId::new(ctx.session_id.get() + 1);
    assert_eq!(
        run(&world, &other, json!({"kind":"baselines"}))?["baselines"],
        json!([])
    );
    assert_eq!(
        run(
            &world,
            &other,
            json!({"kind":"release_baseline", "baseline":id})
        )?["released"],
        false
    );
    assert_eq!(retained(&world, &ctx)?, 1);
    Ok(())
}

#[test]
fn failed_publication_does_not_create_or_release_state() -> Result<()> {
    let (world, ctx) = fixture(2);
    let input = json!({"schema":"dfmcp.query/1", "query":capture("citizens")});
    let result = execute_with_publisher(&world, &ctx, &input, |_| {
        Err(exhausted("packet cannot fit"))
    });
    assert!(result.is_err());
    assert_eq!(retained(&world, &ctx)?, 0);
    let id = handle(&run(&world, &ctx, capture("citizens"))?)?;
    let input =
        json!({"schema":"dfmcp.query/1", "query":{"kind":"release_baseline", "baseline":id}});
    assert!(
        execute_with_publisher(&world, &ctx, &input, |_| Err(exhausted(
            "packet cannot fit"
        )))
        .is_err()
    );
    assert_eq!(retained(&world, &ctx)?, 1);
    Ok(())
}

#[test]
fn capacity_explicit_release_and_fresh_handles_are_bounded() -> Result<()> {
    let (world, ctx) = fixture(0);
    let mut first = String::new();
    for i in 0..MAX_PER_SESSION {
        let id = handle(&run(&world, &ctx, capture(&format!("baseline-{i}")))?)?;
        if i == 0 {
            first = id;
        }
    }
    assert!(
        matches!(run(&world, &ctx, capture("overflow")), Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    run(
        &world,
        &ctx,
        json!({"kind":"release_baseline", "baseline":first}),
    )?;
    let next = handle(&run(&world, &ctx, capture("baseline-0"))?)?;
    assert_ne!(first, next);
    assert!(run(&world, &ctx, json!({"kind":"changes", "baseline":first})).is_err());
    Ok(())
}

#[test]
fn partial_capture_and_forged_controls_never_become_a_baseline() -> Result<()> {
    let (world, ctx) = fixture(257);
    assert!(run(&world, &ctx, capture("too-many")).is_err());
    assert_eq!(retained(&world, &ctx)?, 0);
    let (world, ctx) = fixture(1);
    for field in ["continuation", "limit", "arbitrary_command"] {
        let mut input = capture("invalid");
        input["select"][field] = json!(1);
        assert!(run(&world, &ctx, input).is_err());
    }
    let id = handle(&run(&world, &ctx, capture("good"))?)?;
    assert!(
        run(
            &world,
            &ctx,
            json!({"kind":"changes", "baseline":id,
        "continuation":"qh1:0:fake"})
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn baseline_changes_survive_the_complete_agent_turn_budget() -> Result<()> {
    use super::super::query_response::QueryResponseProjection;
    let (mut world, mut ctx) = fixture(25);
    let mut view = QueryResponseProjection {
        session_id: ctx.session_id.to_string(),
        request_id: ctx.request_id.to_string(),
        anchor: json!({"fortress_id":"1", "epoch":0, "sequence":0, "game_tick":1,
            "state_hash":world.state_hash.to_string()}),
        briefing: json!({"read_only":true, "runtime_admitted":false, "mutation_admissible":false}),
        attention: vec![json!({"finding":"preserve this warning"})],
        affordances: vec![],
        uncertainty: vec![json!({"domain":"history", "epistemic_state":"unknown"})],
        coverage: json!({"status":"partial", "partial_domains":[{"domain":"fortress.history"}]}),
        budget: json!({"admitted":{"max_bytes":8192}}),
        references: vec![],
        maximum_bytes: 8192,
    };
    ctx.budget.max_output_tokens = 2048;
    ctx.budget.max_bytes = view.result_byte_budget()? as u64;
    let capture = json!({"schema":"dfmcp.query/1", "query":capture("packet-budget")});
    let encoded = execute_with_publisher(&world, &ctx, &capture, |payload| view.finish(payload))?;
    assert!(encoded.len() <= 8192);
    let captured: Value = serde_json::from_str(&encoded).map_err(|_| invariant("test response"))?;
    let id = handle(&captured)?;
    for entity in world.graph.entities.values_mut() {
        entity.label.push_str(" renamed");
    }
    advance(&mut world, &mut ctx);
    view.anchor = json!({"fortress_id":"1", "epoch":0, "sequence":1, "game_tick":2,
        "state_hash":world.state_hash.to_string()});
    ctx.budget.max_bytes = view.result_byte_budget()? as u64;
    let mut request =
        json!({"schema":"dfmcp.query/1", "query":{"kind":"changes", "baseline":id, "limit":128}});
    let (mut seen, mut pages) = (Vec::new(), 0usize);
    loop {
        let encoded =
            execute_with_publisher(&world, &ctx, &request, |payload| view.finish(payload))?;
        assert!(encoded.len() <= 8192);
        let page: Value = serde_json::from_str(&encoded).map_err(|_| invariant("test response"))?;
        assert_eq!(page["ok"], true);
        assert_eq!(page["agent_turn"]["continuity"]["status"], "partial");
        assert_eq!(
            page["agent_turn"]["continuity"]["basis"],
            captured["captured"]["basis"]
        );
        assert_eq!(page["agent_turn"]["briefing"]["runtime_admitted"], false);
        assert_eq!(
            page["agent_turn"]["attention"][0]["finding"],
            "preserve this warning"
        );
        assert_eq!(page["agent_turn"]["uncertainty"][0]["domain"], "history");
        assert_eq!(page["agent_turn"]["changes"][0]["change_count"], 25);
        let rows = page["changes"]
            .as_array()
            .ok_or_else(|| invariant("test changes"))?;
        assert!(!rows.is_empty());
        seen.extend(rows.iter().map(|row| row["entity_id"].clone()));
        pages += 1;
        assert!(pages <= 25);
        if page["continuation"].is_null() {
            break;
        }
        request["query"]["continuation"] = page["continuation"].clone();
    }
    assert!(pages > 1);
    assert_eq!(
        seen,
        (1..=25).map(|id| json!(id.to_string())).collect::<Vec<_>>()
    );
    let input =
        json!({"schema":"dfmcp.query/1", "query":{"kind":"release_baseline", "baseline":id}});
    execute_with_publisher(&world, &ctx, &input, |payload| view.finish(payload))?;
    assert_eq!(retained(&world, &ctx)?, 0);
    Ok(())
}
