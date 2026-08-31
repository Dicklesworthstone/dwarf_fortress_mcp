#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::error::Error;

use dfmcp_core::{Digest32, EntityId, FortressId, GameTick, MapCoord, ObservationCursor};
use dfmcp_world::{
    CompareOp, EntityKind, EntityRecord, Fact, FactSource, Predicate, QueryOrder, Value,
    WorldGraph, WorldQuery, WorldSnapshot, execute_bounded_query, execute_query,
};

fn make_fact(val: Value) -> Fact {
    Fact::known(val, GameTick(1), FactSource::Replay, Digest32::ZERO)
}

fn make_test_snapshot() -> WorldSnapshot {
    let mut graph = WorldGraph::default();

    // Dwarf 1: stress=10, age=50, name="Urist", active=true, pos=(0,0,0)
    let mut f1 = BTreeMap::new();
    f1.insert("stress".to_owned(), make_fact(Value::I64(10)));
    f1.insert("age".to_owned(), make_fact(Value::U64(50)));
    f1.insert(
        "name".to_owned(),
        make_fact(Value::Text("Urist".to_owned())),
    );
    f1.insert("active".to_owned(), make_fact(Value::Bool(true)));
    f1.insert(
        "pos".to_owned(),
        make_fact(Value::Coord(MapCoord { x: 0, y: 0, z: 0 })),
    );
    f1.insert(
        "wealth".to_owned(),
        make_fact(Value::Fixed {
            units: 1000,
            scale: 2,
        }),
    );
    graph.entities.insert(
        EntityId::new(1),
        EntityRecord {
            id: EntityId::new(1),
            generation: 1,
            revision: 5,
            kind: EntityKind::Unit,
            label: "Alpha".to_owned(),
            fields: f1,
        },
    );

    // Dwarf 2: stress=20, age=30, name="Domas", active=false, pos=(10,10,0)
    let mut f2 = BTreeMap::new();
    f2.insert("stress".to_owned(), make_fact(Value::I64(20)));
    f2.insert("age".to_owned(), make_fact(Value::U64(30)));
    f2.insert(
        "name".to_owned(),
        make_fact(Value::Text("Domas".to_owned())),
    );
    f2.insert("active".to_owned(), make_fact(Value::Bool(false)));
    f2.insert(
        "pos".to_owned(),
        make_fact(Value::Coord(MapCoord { x: 10, y: 10, z: 0 })),
    );
    f2.insert(
        "wealth".to_owned(),
        make_fact(Value::Fixed {
            units: 2500,
            scale: 2,
        }),
    );
    graph.entities.insert(
        EntityId::new(2),
        EntityRecord {
            id: EntityId::new(2),
            generation: 1,
            revision: 5,
            kind: EntityKind::Unit,
            label: "Alpha".to_owned(), // identical label for tie-break testing
            fields: f2,
        },
    );

    // Dwarf 3: stress=30, age=70, name="Zulgar", active=true, pos=(5,5,0)
    let mut f3 = BTreeMap::new();
    f3.insert("stress".to_owned(), make_fact(Value::I64(30)));
    f3.insert("age".to_owned(), make_fact(Value::U64(70)));
    f3.insert(
        "name".to_owned(),
        make_fact(Value::Text("Zulgar".to_owned())),
    );
    f3.insert("active".to_owned(), make_fact(Value::Bool(true)));
    f3.insert(
        "pos".to_owned(),
        make_fact(Value::Coord(MapCoord { x: 5, y: 5, z: 0 })),
    );
    f3.insert(
        "wealth".to_owned(),
        make_fact(Value::Fixed {
            units: 500,
            scale: 2,
        }),
    );
    graph.entities.insert(
        EntityId::new(3),
        EntityRecord {
            id: EntityId::new(3),
            generation: 1,
            revision: 10,
            kind: EntityKind::Unit,
            label: "Zulgar".to_owned(),
            fields: f3,
        },
    );

    WorldSnapshot::new(
        FortressId::new(1),
        GameTick(100),
        ObservationCursor::ORIGIN,
        false,
        graph,
    )
}

/// TEST-006: Truth Table for CompareOp across all value types
#[test]
fn test_006_truth_table_compare_ops() -> Result<(), Box<dyn Error>> {
    let snapshot = make_test_snapshot();

    // 1. I64 Field Comparisons on 'stress' (10, 20, 30)
    let test_cases = vec![
        (CompareOp::Eq, Value::I64(20), vec![2]),
        (CompareOp::Ne, Value::I64(20), vec![1, 3]),
        (CompareOp::Lt, Value::I64(20), vec![1]),
        (CompareOp::Le, Value::I64(20), vec![1, 2]),
        (CompareOp::Gt, Value::I64(20), vec![3]),
        (CompareOp::Ge, Value::I64(20), vec![2, 3]),
    ];

    for (op, val, expected_ids) in test_cases {
        let q = WorldQuery {
            kinds: vec![EntityKind::Unit],
            predicate: Some(Predicate::FieldCompare {
                entity_id: EntityId::NIL,
                field: "stress".to_owned(),
                op,
                value: val,
            }),
            order: QueryOrder::EntityIdAscending,
            limit: 10,
            continuation: None,
        };
        let res = execute_query(&snapshot, &q, 100)?;
        let actual_ids: Vec<u64> = res.entities.iter().map(|e| e.id.get()).collect();
        assert_eq!(
            actual_ids, expected_ids,
            "Failed I64 truth table for op {:?}",
            op
        );
    }

    // 2. Text Comparisons on 'name'
    let q_text = WorldQuery {
        kinds: vec![EntityKind::Unit],
        predicate: Some(Predicate::FieldCompare {
            entity_id: EntityId::NIL,
            field: "name".to_owned(),
            op: CompareOp::Eq,
            value: Value::Text("Urist".to_owned()),
        }),
        order: QueryOrder::EntityIdAscending,
        limit: 10,
        continuation: None,
    };
    let res_text = execute_query(&snapshot, &q_text, 100)?;
    assert_eq!(res_text.entities.len(), 1);
    assert_eq!(res_text.entities[0].id.get(), 1);

    // 3. Bool Comparisons on 'active'
    let q_bool = WorldQuery {
        kinds: vec![EntityKind::Unit],
        predicate: Some(Predicate::FieldCompare {
            entity_id: EntityId::NIL,
            field: "active".to_owned(),
            op: CompareOp::Eq,
            value: Value::Bool(true),
        }),
        order: QueryOrder::EntityIdAscending,
        limit: 10,
        continuation: None,
    };
    let res_bool = execute_query(&snapshot, &q_bool, 100)?;
    assert_eq!(res_bool.entities.len(), 2);
    let bool_ids: Vec<u64> = res_bool.entities.iter().map(|e| e.id.get()).collect();
    assert_eq!(bool_ids, vec![1, 3]);

    // 4. Fixed scale comparisons on 'wealth'
    let q_fixed = WorldQuery {
        kinds: vec![EntityKind::Unit],
        predicate: Some(Predicate::FieldCompare {
            entity_id: EntityId::NIL,
            field: "wealth".to_owned(),
            op: CompareOp::Gt,
            value: Value::Fixed {
                units: 1000,
                scale: 2,
            },
        }),
        order: QueryOrder::EntityIdAscending,
        limit: 10,
        continuation: None,
    };
    let res_fixed = execute_query(&snapshot, &q_fixed, 100)?;
    assert_eq!(res_fixed.entities.len(), 1);
    assert_eq!(res_fixed.entities[0].id.get(), 2);

    Ok(())
}

/// TEST-006: Truth Table for Logical Connectives (All, Any, Not)
#[test]
fn test_006_truth_table_logical_connectives() -> Result<(), Box<dyn Error>> {
    let snapshot = make_test_snapshot();

    // ALL: active=true AND stress < 25 => only entity 1
    let q_all = WorldQuery {
        kinds: vec![EntityKind::Unit],
        predicate: Some(Predicate::All(vec![
            Predicate::FieldCompare {
                entity_id: EntityId::NIL,
                field: "active".to_owned(),
                op: CompareOp::Eq,
                value: Value::Bool(true),
            },
            Predicate::FieldCompare {
                entity_id: EntityId::NIL,
                field: "stress".to_owned(),
                op: CompareOp::Lt,
                value: Value::I64(25),
            },
        ])),
        order: QueryOrder::EntityIdAscending,
        limit: 10,
        continuation: None,
    };
    let res_all = execute_query(&snapshot, &q_all, 100)?;
    assert_eq!(res_all.entities.len(), 1);
    assert_eq!(res_all.entities[0].id.get(), 1);

    // ANY: stress == 10 OR stress == 30 => entities 1 and 3
    let q_any = WorldQuery {
        kinds: vec![EntityKind::Unit],
        predicate: Some(Predicate::Any(vec![
            Predicate::FieldCompare {
                entity_id: EntityId::NIL,
                field: "stress".to_owned(),
                op: CompareOp::Eq,
                value: Value::I64(10),
            },
            Predicate::FieldCompare {
                entity_id: EntityId::NIL,
                field: "stress".to_owned(),
                op: CompareOp::Eq,
                value: Value::I64(30),
            },
        ])),
        order: QueryOrder::EntityIdAscending,
        limit: 10,
        continuation: None,
    };
    let res_any = execute_query(&snapshot, &q_any, 100)?;
    assert_eq!(res_any.entities.len(), 2);
    let ids: Vec<u64> = res_any.entities.iter().map(|e| e.id.get()).collect();
    assert_eq!(ids, vec![1, 3]);

    // NOT: NOT (active == true) => entity 2
    let q_not = WorldQuery {
        kinds: vec![EntityKind::Unit],
        predicate: Some(Predicate::Not(Box::new(Predicate::FieldCompare {
            entity_id: EntityId::NIL,
            field: "active".to_owned(),
            op: CompareOp::Eq,
            value: Value::Bool(true),
        }))),
        order: QueryOrder::EntityIdAscending,
        limit: 10,
        continuation: None,
    };
    let res_not = execute_query(&snapshot, &q_not, 100)?;
    assert_eq!(res_not.entities.len(), 1);
    assert_eq!(res_not.entities[0].id.get(), 2);

    Ok(())
}

/// TEST-006: Deterministic Tie-Break Ordering Guarantees
#[test]
fn test_006_deterministic_tie_break_ordering() -> Result<(), Box<dyn Error>> {
    let snapshot = make_test_snapshot();

    // 1. LabelAscending tie-break: Entity 1 and Entity 2 both have label "Alpha" -> tie break by EntityId ascending
    let q_label = WorldQuery {
        kinds: vec![EntityKind::Unit],
        predicate: None,
        order: QueryOrder::LabelAscending,
        limit: 10,
        continuation: None,
    };
    let res_label = execute_query(&snapshot, &q_label, 100)?;
    let ids: Vec<u64> = res_label.entities.iter().map(|e| e.id.get()).collect();
    assert_eq!(ids, vec![1, 2, 3]);

    // 2. RevisionDescending tie-break: Entity 3 (rev 10), then Entity 1 & 2 (rev 5, tie break by EntityId asc: 1 then 2)
    let q_rev = WorldQuery {
        kinds: vec![EntityKind::Unit],
        predicate: None,
        order: QueryOrder::RevisionDescending,
        limit: 10,
        continuation: None,
    };
    let res_rev = execute_query(&snapshot, &q_rev, 100)?;
    let rev_ids: Vec<u64> = res_rev.entities.iter().map(|e| e.id.get()).collect();
    assert_eq!(rev_ids, vec![3, 1, 2]);

    Ok(())
}

/// Budget Enforcement: Entity limits, byte limits, and continuation tokens
#[test]
fn test_query_budget_enforcement_and_continuations() -> Result<(), Box<dyn Error>> {
    let snapshot = make_test_snapshot();

    // 1. Entity limit = 2 (out of 3 matches) -> truncated = true, continuation emitted
    let q_limit = WorldQuery {
        kinds: vec![EntityKind::Unit],
        predicate: None,
        order: QueryOrder::EntityIdAscending,
        limit: 2,
        continuation: None,
    };
    let res_limit = execute_query(&snapshot, &q_limit, 100)?;
    assert_eq!(res_limit.entities.len(), 2);
    assert_eq!(res_limit.matched, 3);
    assert!(res_limit.truncated);
    assert_eq!(res_limit.continuation.as_deref(), Some("cont:1:0:0:2"));

    // 2. Byte limit bounding: set byte limit small enough to fit only 1 entity
    let one_entity_bytes = snapshot.graph.entities[&EntityId::new(1)]
        .canonical_bytes()
        .len();
    let q_bounded = WorldQuery {
        kinds: vec![EntityKind::Unit],
        predicate: None,
        order: QueryOrder::EntityIdAscending,
        limit: 10,
        continuation: None,
    };
    let res_byte_bounded = execute_bounded_query(
        &snapshot,
        &q_bounded,
        100,
        Some(one_entity_bytes + 10), // only enough for 1 entity
    )?;
    assert_eq!(res_byte_bounded.entities.len(), 1);
    assert!(res_byte_bounded.truncated);
    assert_eq!(
        res_byte_bounded.continuation.as_deref(),
        Some("cont:1:0:0:1")
    );

    Ok(())
}

/// Static Plan Cost Estimation
#[test]
fn test_query_static_cost_estimation() {
    let snapshot = make_test_snapshot();

    let q = WorldQuery {
        kinds: vec![EntityKind::Unit],
        predicate: Some(Predicate::All(vec![
            Predicate::FieldCompare {
                entity_id: EntityId::NIL,
                field: "stress".to_owned(),
                op: CompareOp::Lt,
                value: Value::I64(20),
            },
            Predicate::FieldCompare {
                entity_id: EntityId::NIL,
                field: "active".to_owned(),
                op: CompareOp::Eq,
                value: Value::Bool(true),
            },
        ])),
        order: QueryOrder::EntityIdAscending,
        limit: 10,
        continuation: None,
    };

    let cost = q.estimate_cost(&snapshot);
    assert_eq!(cost.estimated_scanned_entities, 3);
    assert_eq!(cost.estimated_predicate_cost, 3); // All (1) + 2 comparisons (2) = 3
    assert_eq!(cost.estimated_total_cost, 9);
}
