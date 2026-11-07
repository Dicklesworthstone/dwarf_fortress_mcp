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
