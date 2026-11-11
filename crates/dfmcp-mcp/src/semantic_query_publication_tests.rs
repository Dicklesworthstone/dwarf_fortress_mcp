//! Publication regressions: small pages may read a larger authorized source.
use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, EdgeId, EntityId, FortressId, GameTick, MapCoord,
    ObservationCursor, RequestId, SessionId, WorkBudget,
};
use dfmcp_world::{
    EdgeKind, EdgeRecord, EntityKind, EntityRecord, Fact, FactSource, Value as WorldValue,
    WorldGraph,
};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;
static NEXT_SESSION: AtomicU64 = AtomicU64::new(30_000_000);

fn known(value: WorldValue) -> Fact {
    Fact::known(
        value,
        GameTick(10),
        FactSource::DfhackField("fixture".to_owned()),
        Digest32::of_bytes(b"query-publication-fixture"),
    )
}

fn fixture() -> (WorldSnapshot, OperationContext) {
    let mut graph = WorldGraph::default();
    for id in 1..=5 {
        let mut fields = BTreeMap::from([
            (
                "position".to_owned(),
                known(WorldValue::Coord(MapCoord {
                    x: id as i32,
                    y: 0,
                    z: 0,
                })),
            ),
            ("rank".to_owned(), known(WorldValue::U64(id))),
        ]);
        if id == 1 {
            fields.insert(
                "source_only_payload".to_owned(),
                known(WorldValue::Text("x".repeat(65_536))),
            );
        }
        graph.entities.insert(
            EntityId::new(id),
            EntityRecord {
                id: EntityId::new(id),
                generation: 1,
                revision: 1,
                kind: EntityKind::Unit,
                label: format!("unit {id}"),
                fields,
            },
        );
        let edge = EdgeRecord {
            id: EdgeId::new(u128::from(id)),
            revision: 1,
            kind: EdgeKind::AssignedTo,
            from: EntityId::new(id),
            to: EntityId::new(100),
            fields: BTreeMap::new(),
        };
        graph.edges.insert(edge.id, edge);
    }
    graph.entities.insert(
        EntityId::new(100),
        EntityRecord {
            id: EntityId::new(100),
            generation: 1,
            revision: 1,
            kind: EntityKind::Building,
            label: "workshop".to_owned(),
            fields: BTreeMap::new(),
        },
    );
    let snapshot = WorldSnapshot::new(
        FortressId::new(42),
        GameTick(10),
        ObservationCursor {
            epoch: 2,
            sequence: 8,
        },
        true,
        graph,
    );
    let context = OperationContext {
        session_id: SessionId::new(u128::from(NEXT_SESSION.fetch_add(1, Ordering::Relaxed))),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget {
            max_output_tokens: 2048,
            max_wall_millis: 60_000,
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

fn requests() -> [Value; 3] {
    [
        json!({"schema":"dfmcp.query/1","query":{"kind":"spatial_entities",
            "kinds":["unit"],"region":{"min":[0,0,0],"max":[10,0,0]},
            "geometry":{"kind":"point","field":"position"},"limit":1}}),
        json!({"schema":"dfmcp.query/1","query":{"kind":"related_entities",
            "kinds":["unit"],"relation":{"edge_kinds":["assigned_to"],
            "target_kinds":["building"]},"limit":1}}),
        json!({"schema":"dfmcp.query/1","query":{"kind":"ranked_entities",
            "kinds":["unit"],"order_by":[{"field":"rank","numeric_type":"u64",
            "direction":"ascending"}],"limit":1}}),
    ]
}

fn publish(
    snapshot: &WorldSnapshot,
    context: &OperationContext,
    input: &Value,
) -> std::result::Result<Value, Box<dyn std::error::Error>> {
    let raw = execute_with_publisher(snapshot, context, input, |value| Ok(value.to_string()))?;
    let maximum = context
        .budget
        .max_bytes
        .min(u64::from(context.budget.max_output_tokens).saturating_mul(4));
    assert!(raw.len() as u64 <= maximum);
    Ok(serde_json::from_str(&raw)?)
}

#[test]
fn published_spatial_related_and_ranked_pages_keep_complete_source_budget() -> TestResult {
    let (snapshot, context) = fixture();
    for mut input in requests() {
        let first = publish(&snapshot, &context, &input)?;
        assert_eq!(first["matched"], 5);
        assert_eq!(first["returned"], 1);
        assert_eq!(first["rows"][0]["entity_id"], "1");
        assert!(
            first["source_cost"]["bounded_bytes"]
                .as_u64()
                .ok_or("source bytes")?
                > 8192
        );
        assert!(!first["continuation"].is_null());
        assert_eq!(first["_condition_watch_work"], json!([]));
        input["query"]["continuation"] = first["continuation"].clone();
        let second = publish(&snapshot, &context, &input)?;
        assert_eq!(second["rows"][0]["entity_id"], "2");
        assert_eq!(second["query_digest"], first["query_digest"]);
    }
    Ok(())
}

#[test]
fn active_watch_reservation_preserves_source_and_page_continuation() -> TestResult {
    let (snapshot, context) = fixture();
    let create = json!({"schema":"dfmcp.query/1","query":{"kind":"watch",
        "key":"wait-for-unpause","condition":{"op":"paused","value":false},
        "deadline_tick":100}});
    let watch = publish(&snapshot, &context, &create)?;
    assert!(
        !watch["_condition_watch_work"]
            .as_array()
            .ok_or("active watches")?
            .is_empty()
    );
    for mut input in requests() {
        let first = publish(&snapshot, &context, &input)?;
        assert_eq!(first["rows"][0]["entity_id"], "1");
        assert_eq!(
            first["_condition_watch_work"],
            watch["_condition_watch_work"]
        );
        input["query"]["continuation"] = first["continuation"].clone();
        let second = publish(&snapshot, &context, &input)?;
        assert_eq!(second["rows"][0]["entity_id"], "2");
        assert_eq!(
            second["_condition_watch_work"],
            watch["_condition_watch_work"]
        );
    }
    release_session_resources(context.session_id, true, |value| Ok(value.to_string()))?;
    Ok(())
}

#[test]
fn source_and_selected_output_overflow_never_reach_publication() -> TestResult {
    let (snapshot, context) = fixture();
    for mut input in requests() {
        let called = Cell::new(false);
        let mut small_source = context.clone();
        small_source.budget.max_bytes = 8192;
        assert!(
            matches!(execute_with_publisher(&snapshot, &small_source, &input, |value| {
            called.set(true);
            Ok(value.to_string())
        }), Err(error) if error.code == ErrorCode::BudgetExceeded)
        );
        assert!(!called.get());

        input["query"]["fields"] = json!(["source_only_payload"]);
        assert!(
            matches!(execute_with_publisher(&snapshot, &context, &input, |value| {
            called.set(true);
            Ok(value.to_string())
        }), Err(error) if error.code == ErrorCode::BudgetExceeded)
        );
        assert!(!called.get());
    }
    Ok(())
}

#[test]
fn projected_entity_pages_reserve_active_work_without_narrowing_source_admission() -> TestResult {
    let (snapshot, context) = fixture();
    let create = json!({"schema":"dfmcp.query/1","query":{"kind":"watch",
        "key":"entity-page-watch","condition":{"op":"paused","value":false},
        "deadline_tick":100}});
    let watch = publish(&snapshot, &context, &create)?;
    let mut input = json!({"schema":"dfmcp.query/1","query":{"kind":"entities",
        "kinds":["unit"],"fields":["rank"],"limit":1}});
    let first = publish(&snapshot, &context, &input)?;
    assert_eq!(first["matched"], 5);
    assert_eq!(first["rows"][0]["entity_id"], "1");
    assert_eq!(
        first["_condition_watch_work"],
        watch["_condition_watch_work"]
    );
    input["query"]["continuation"] = first["continuation"].clone();
    let second = publish(&snapshot, &context, &input)?;
    assert_eq!(second["rows"][0]["entity_id"], "2");
    assert_eq!(
        second["_condition_watch_work"],
        watch["_condition_watch_work"]
    );
    let called = Cell::new(false);
    let mut small_source = context.clone();
    small_source.budget.max_bytes = 8192;
    assert!(
        matches!(execute_with_publisher(&snapshot, &small_source, &input, |value| {
        called.set(true); Ok(value.to_string())
    }), Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    assert!(!called.get());
    input["query"]
        .as_object_mut()
        .ok_or("query missing")?
        .remove("continuation");
    input["query"]["fields"] = json!(["source_only_payload"]);
    assert!(
        matches!(execute_with_publisher(&snapshot, &context, &input, |value| {
        called.set(true); Ok(value.to_string())
    }), Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    assert!(!called.get());
    release_session_resources(context.session_id, true, |value| Ok(value.to_string()))?;
    Ok(())
}

fn compound_aggregates() -> Vec<Value> {
    let relation = json!({"edge_kinds":["assigned_to"],"target_kinds":["building"]});
    let spatial = json!({"region":{"min":[0,0,0],"max":[10,0,0]},
        "geometry":{"kind":"point","field":"position"}});
    [
        json!({"kind":"aggregate","kinds":["unit"],"relation":relation}),
        json!({"kind":"aggregate","kinds":["unit"],"spatial":spatial}),
        json!({"kind":"aggregate","kinds":["unit"],"relation":relation,"spatial":spatial}),
    ]
    .into_iter()
    .map(|query| json!({"schema":"dfmcp.query/1","query":query}))
    .collect()
}

#[test]
fn compound_counts_publish_under_default_output_without_shrinking_source_authority() -> TestResult {
    let (snapshot, context) = fixture();
    assert!(snapshot.canonical_bytes().len() > 65_536);
    for input in compound_aggregates() {
        let page = publish(&snapshot, &context, &input)?;
        assert_eq!(page["matched"], 5);
        assert_eq!(page["selection"]["matched"], 5);
        assert!(
            page["source_cost"]["bounded_bytes"]
                .as_u64()
                .ok_or("source bytes")?
                > 8192
        );
        assert_eq!(page["coverage"]["absence_proven"], false);
    }
    Ok(())
}

#[test]
fn enclosing_response_reservation_keeps_the_original_source_for_every_projected_family()
-> TestResult {
    let (snapshot, context) = fixture();
    let mut inputs = requests().to_vec();
    inputs.extend(compound_aggregates());
    inputs.push(json!({"schema":"dfmcp.query/1","query":{"kind":"entities",
        "kinds":["unit"],"fields":["rank"],"limit":1}}));
    inputs.push(json!({"schema":"dfmcp.query/1","query":{"kind":"search",
        "kinds":["unit"],"text":"unit","limit":1}}));
    for mut input in inputs {
        // This is the allowance remaining after a live runtime reserves its
        // complete Agent Turn, before the shared publisher reserves watches.
        let output_bytes = 8192;
        let expected = execute(&snapshot, &context, &input)?;
        let raw =
            execute_with_publisher_budget(&snapshot, &context, &input, output_bytes, |value| {
                Ok(value.to_string())
            })?;
        assert!(raw.len() as u64 <= output_bytes);
        let first: Value = serde_json::from_str(&raw)?;
        assert_eq!(first["matched"], 5);
        assert_eq!(first["query_digest"], expected["query_digest"]);
        if first["continuation"].is_string() {
            input["query"]["continuation"] = first["continuation"].clone();
            let raw = execute_with_publisher_budget(
                &snapshot,
                &context,
                &input,
                output_bytes,
                |value| Ok(value.to_string()),
            )?;
            let second: Value = serde_json::from_str(&raw)?;
            assert_eq!(second["rows"][0]["entity_id"], "2");
            assert_eq!(second["query_digest"], first["query_digest"]);
        }
    }
    Ok(())
}

#[test]
fn compound_output_reserves_active_watches_and_retains_their_exact_evidence() -> TestResult {
    let (snapshot, context) = fixture();
    let watch = publish(
        &snapshot,
        &context,
        &json!({"schema":"dfmcp.query/1","query":{
        "kind":"watch","key":"compound-count-watch","condition":{"op":"paused","value":false},
        "deadline_tick":100}}),
    )?;
    for input in compound_aggregates() {
        let raw = execute_with_publisher_budget(&snapshot, &context, &input, 8192, |value| {
            Ok(value.to_string())
        })?;
        assert!(raw.len() <= 8192);
        let page: Value = serde_json::from_str(&raw)?;
        assert_eq!(page["matched"], 5);
        assert_eq!(
            page["_condition_watch_work"],
            watch["_condition_watch_work"]
        );
    }
    release_session_resources(context.session_id, true, |value| Ok(value.to_string()))?;
    Ok(())
}

#[test]
fn independent_output_budget_cannot_expand_sources_grants_or_publish_partial_aggregates()
-> TestResult {
    let (snapshot, context) = fixture();
    for input in compound_aggregates() {
        let called = Cell::new(false);
        let publisher = |value: Value| {
            called.set(true);
            Ok(value.to_string())
        };
        let mut source_limited = context.clone();
        source_limited.budget.max_bytes = 8192;
        assert!(
            matches!(execute_with_publisher_budget(&snapshot, &source_limited, &input,
            u64::MAX, publisher), Err(error) if error.code == ErrorCode::BudgetExceeded)
        );
        assert!(!called.get());
        let mut record_limited = context.clone();
        record_limited.budget.max_entities = 1;
        assert!(
            matches!(execute_with_publisher_budget(&snapshot, &record_limited, &input,
            u64::MAX, publisher), Err(error) if error.code == ErrorCode::BudgetExceeded)
        );
        assert!(!called.get());
        for output_bytes in [0, 1, 128] {
            assert!(
                matches!(execute_with_publisher_budget(&snapshot, &context, &input,
                output_bytes, publisher), Err(error) if error.code == ErrorCode::BudgetExceeded)
            );
            assert!(!called.get());
        }
        let mut expired = context.clone();
        expired.grants[0].expires_at_tick = Some(GameTick(9));
        assert!(
            matches!(execute_with_publisher_budget(&snapshot, &expired, &input,
            u64::MAX, publisher), Err(error) if error.code == ErrorCode::CapabilityDenied)
        );
        assert!(!called.get());
        let mut cancelled = context.clone();
        cancelled.cancellation_requested = true;
        assert!(
            execute_with_publisher_budget(&snapshot, &cancelled, &input, 8192, publisher).is_err()
        );
        assert!(!called.get());
    }
    Ok(())
}
