use std::collections::BTreeMap;

use dfmcp_core::{Digest32, EntityId, ErrorCode, FortressId, GameTick, ObservationCursor, Result};
use dfmcp_world::query_page::{execute_bounded_query, execute_query};
use dfmcp_world::{
    CompareOp, EntityKind, EntityRecord, Fact, FactSource, Predicate, QueryOrder, WorldGraph,
    WorldQuery, WorldSnapshot, Value,
};

fn snapshot(count: u64) -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    for id in 1..=count {
        graph.entities.insert(EntityId::new(id), EntityRecord {
            id: EntityId::new(id), generation: 1, revision: 1,
            kind: EntityKind::Unit, label: format!("citizen-{id:04}"),
            fields: BTreeMap::from([("stress".to_owned(), Fact::known(
                Value::U64(id), GameTick(1), FactSource::Replay, Digest32::ZERO,
            ))]),
        });
    }
    WorldSnapshot::new(FortressId::new(1), GameTick(1), ObservationCursor::ORIGIN, true, graph)
}

fn query() -> WorldQuery {
    WorldQuery {
        kinds: vec![EntityKind::Unit], predicate: None,
        order: QueryOrder::EntityIdAscending, limit: 2, continuation: None,
    }
}

#[test]
fn every_order_paginates_without_missing_or_repeating_rows() -> Result<()> {
    let snapshot = snapshot(17);
    for order in [QueryOrder::EntityIdAscending, QueryOrder::EntityIdDescending,
        QueryOrder::RevisionDescending, QueryOrder::LabelAscending] {
        let mut query = query();
        query.order = order;
        let mut rows = Vec::new();
        let mut pages = 0;
        loop {
            let page = execute_query(&snapshot, &query, 100)?;
            assert_eq!(page.matched, 17);
            assert_eq!(page.truncated, page.continuation.is_some());
            rows.extend(page.entities.iter().map(|row| row.id.get()));
            pages += 1;
            assert!(pages <= 17);
            query.continuation = page.continuation;
            if query.continuation.is_none() { break; }
            query.limit = 3;
        }
        let expected = if order == QueryOrder::EntityIdDescending {
            (1..=17).rev().collect::<Vec<_>>()
        } else { (1..=17).collect::<Vec<_>>() };
        assert_eq!(rows, expected);
    }
    Ok(())
}

#[test]
fn changing_filters_or_order_cannot_reuse_a_page() -> Result<()> {
    let snapshot = snapshot(7);
    let mut original = query();
    original.continuation = execute_query(&snapshot, &original, 100)?.continuation;
    for change in 0..3 {
        let mut changed = original.clone();
        match change {
            0 => changed.order = QueryOrder::EntityIdDescending,
            1 => changed.kinds = vec![EntityKind::Job],
            _ => changed.predicate = Some(Predicate::FieldCompare {
                entity_id: EntityId::NIL, field: "stress".to_owned(),
                op: CompareOp::Gt, value: Value::U64(3),
            }),
        }
        assert!(matches!(execute_query(&snapshot, &changed, 100),
            Err(error) if error.code == ErrorCode::StaleAnchor));
    }
    Ok(())
}

#[test]
fn every_anchor_component_is_bound() -> Result<()> {
    let original = snapshot(7);
    let mut query = query();
    query.continuation = execute_query(&original, &query, 100)?.continuation;
    for change in 0..5 {
        let mut changed = original.clone();
        match change {
            0 => changed.fortress_id = FortressId::new(2),
            1 => changed.cursor.epoch += 1,
            2 => changed.cursor.sequence += 1,
            3 => changed.tick = GameTick(2),
            _ => changed.paused = false,
        }
        changed.refresh_hash();
        assert!(matches!(execute_query(&changed, &query, 100),
            Err(error) if error.code == ErrorCode::StaleAnchor));
    }
    Ok(())
}

#[test]
fn a_fork_with_the_same_cursor_is_not_the_same_result_set() -> Result<()> {
    let original = snapshot(7);
    let mut query = query();
    query.continuation = execute_query(&original, &query, 100)?.continuation;
    let mut fork = original.clone();
    fork.graph.entities.remove(&EntityId::new(1));
    fork.refresh_hash();
    assert_eq!(fork.cursor, original.cursor);
    assert!(matches!(execute_query(&fork, &query, 100),
        Err(error) if error.code == ErrorCode::StaleAnchor));
    Ok(())
}

#[test]
fn cursor_edits_and_legacy_offsets_fail_closed() -> Result<()> {
    let snapshot = snapshot(7);
    let original = execute_query(&snapshot, &query(), 100)?.continuation;
    let Some(original) = original else {
        return Err(dfmcp_core::DfmcpError::new(ErrorCode::InternalInvariantViolation,
            "fixture must produce a continuation"));
    };
    let mut query = query();
    query.continuation = Some(original.replacen("q1:2:", "q1:3:", 1));
    assert!(matches!(execute_query(&snapshot, &query, 100),
        Err(error) if error.code == ErrorCode::StaleAnchor));
    for token in ["cont:1:0:0:2", "offset:2", "q1:0:bad", "q1:+2:bad", "q1:2:bad:extra"] {
        query.continuation = Some(token.to_owned());
        assert!(execute_query(&snapshot, &query, 100).is_err());
    }
    query.continuation = Some("q1:".repeat(100));
    assert!(matches!(execute_query(&snapshot, &query, 100),
        Err(error) if error.code == ErrorCode::BudgetExceeded));
    Ok(())
}

#[test]
fn kind_selection_is_a_set_but_custom_names_do_not_alias_builtins() -> Result<()> {
    let snapshot = snapshot(7);
    let mut query = query();
    query.kinds = vec![EntityKind::Job, EntityKind::Unit];
    query.continuation = execute_query(&snapshot, &query, 100)?.continuation;
    query.kinds = vec![EntityKind::Unit, EntityKind::Job, EntityKind::Unit];
    assert_eq!(execute_query(&snapshot, &query, 100)?.entities[0].id.get(), 3);
    query.kinds = vec![EntityKind::Other("unit".to_owned()), EntityKind::Job];
    assert!(matches!(execute_query(&snapshot, &query, 100),
        Err(error) if error.code == ErrorCode::StaleAnchor));
    Ok(())
}

#[test]
fn byte_budget_produces_progressing_whole_row_pages() -> Result<()> {
    let snapshot = snapshot(7);
    let bytes = snapshot.graph.entities[&EntityId::new(1)].canonical_bytes().len();
    let mut query = query();
    let page = execute_bounded_query(&snapshot, &query, 100, Some(bytes))?;
    assert_eq!(page.entities.len(), 1);
    query.continuation = page.continuation;
    let next = execute_bounded_query(&snapshot, &query, 100, Some(bytes * 2))?;
    assert_eq!(next.entities.iter().map(|row| row.id.get()).collect::<Vec<_>>(), vec![2, 3]);
    assert!(matches!(execute_bounded_query(&snapshot, &query, 100, Some(1)),
        Err(error) if error.code == ErrorCode::BudgetExceeded));
    Ok(())
}

#[test]
fn invalid_snapshot_hash_is_not_a_query_anchor() {
    let mut snapshot = snapshot(3);
    snapshot.paused = false;
    assert!(matches!(execute_query(&snapshot, &query(), 100),
        Err(error) if error.code == ErrorCode::InternalInvariantViolation));
}

#[test]
fn cartesian_scan_predicate_work_is_rejected_before_evaluation() {
    let snapshot = snapshot(1500);
    let mut query = query();
    query.predicate = Some(Predicate::All(vec![Predicate::True; 3000]));
    assert!(matches!(execute_query(&snapshot, &query, 100),
        Err(error) if error.code == ErrorCode::BudgetExceeded));
}

#[test]
fn aggregate_query_bytes_and_nested_kind_names_are_bounded() {
    let snapshot = snapshot(1);
    let mut query = query();
    query.predicate = Some(Predicate::All((0..5).map(|_| Predicate::FieldCompare {
        entity_id: EntityId::NIL, field: "stress".to_owned(), op: CompareOp::Eq,
        value: Value::Text("x".repeat(65536)),
    }).collect()));
    assert!(matches!(execute_query(&snapshot, &query, 100),
        Err(error) if error.code == ErrorCode::BudgetExceeded));
    query.predicate = Some(Predicate::EntityKind {
        entity_id: EntityId::NIL, kind: EntityKind::Other("x".repeat(129)),
    });
    assert!(matches!(execute_query(&snapshot, &query, 100),
        Err(error) if error.code == ErrorCode::BudgetExceeded));
}
