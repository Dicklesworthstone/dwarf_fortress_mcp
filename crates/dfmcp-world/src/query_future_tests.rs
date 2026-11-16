//! Source-tick eligibility for ordinary field comparisons (df-lab-structured-query-f7q.6).
use std::collections::BTreeMap;

use dfmcp_core::{Digest32, EntityId, FortressId, GameTick, ObservationCursor, Result};

use super::{
    CompareOp, Predicate, PredicateTruth, QueryOrder, WorldQuery, evaluate, evaluate_truth_for,
    execute_bounded_query, execute_query,
};
use crate::{EntityKind, EntityRecord, Fact, FactPresence, FactSource, Value, WorldGraph, WorldSnapshot};

const SUBJECT: EntityId = EntityId::new(1);

fn source(observed_at: u64, explicit_known: bool) -> WorldSnapshot {
    let mut fact = Fact::known(
        Value::Bool(true),
        GameTick(observed_at),
        FactSource::Replay,
        Digest32::of_bytes(b"source-time-fixture"),
    );
    fact.presence = explicit_known.then_some(FactPresence::Known(Value::Bool(true)));
    let mut graph = WorldGraph::default();
    graph.entities.insert(
        SUBJECT,
        EntityRecord {
            id: SUBJECT,
            generation: 1,
            revision: 1,
            kind: EntityKind::Unit,
            label: "subject".to_owned(),
            fields: BTreeMap::from([("available".to_owned(), fact)]),
        },
    );
    WorldSnapshot::new(
        FortressId::new(1),
        GameTick(10),
        ObservationCursor::ORIGIN,
        true,
        graph,
    )
}

fn comparison(entity_id: EntityId) -> Predicate {
    Predicate::FieldCompare {
        entity_id,
        field: "available".to_owned(),
        op: CompareOp::Eq,
        value: Value::Bool(true),
    }
}

fn query(predicate: Predicate) -> WorldQuery {
    WorldQuery {
        kinds: vec![EntityKind::Unit],
        predicate: Some(predicate),
        order: QueryOrder::EntityIdAscending,
        limit: 10,
        continuation: None,
    }
}

#[test]
fn field_comparisons_admit_past_and_equal_ticks_but_not_future_known_values() -> Result<()> {
    for explicit_known in [false, true] {
        for observed_at in [9, 10, 11] {
            let snapshot = source(observed_at, explicit_known);
            let future = observed_at > snapshot.tick.0;
            let predicate = comparison(EntityId::NIL);
            assert_eq!(
                evaluate_truth_for(&snapshot, SUBJECT, &predicate),
                if future { PredicateTruth::Unknown } else { PredicateTruth::True }
            );
            assert_eq!(
                evaluate_truth_for(&snapshot, SUBJECT, &Predicate::Not(Box::new(predicate.clone()))),
                if future { PredicateTruth::Unknown } else { PredicateTruth::False }
            );
            assert_eq!(evaluate(&snapshot, &comparison(SUBJECT)), !future);
            assert!(!evaluate(
                &snapshot,
                &Predicate::Not(Box::new(comparison(SUBJECT)))
            ));
            assert_eq!(
                execute_query(&snapshot, &query(predicate.clone()), 10)?.matched,
                u64::from(!future)
            );
            assert_eq!(
                execute_bounded_query(&snapshot, &query(predicate), 10, Some(16_384))?.matched,
                u64::from(!future)
            );
            assert_eq!(
                execute_query(
                    &snapshot,
                    &query(Predicate::Not(Box::new(comparison(EntityId::NIL)))),
                    10,
                )?.matched,
                0
            );
        }
    }
    Ok(())
}

#[test]
fn future_comparisons_preserve_three_valued_boolean_short_circuits() {
    let snapshot = source(11, true);
    let future = comparison(EntityId::NIL);
    for (predicate, expected) in [
        (Predicate::All(vec![Predicate::True, future.clone()]), PredicateTruth::Unknown),
        (Predicate::Any(vec![Predicate::False, future.clone()]), PredicateTruth::Unknown),
        (Predicate::All(vec![Predicate::False, future.clone()]), PredicateTruth::False),
        (Predicate::Any(vec![Predicate::True, future.clone()]), PredicateTruth::True),
        (Predicate::Not(Box::new(future)), PredicateTruth::Unknown),
    ] {
        assert_eq!(evaluate_truth_for(&snapshot, SUBJECT, &predicate), expected);
    }
}

#[test]
fn later_world_does_not_promote_the_same_fact_in_an_exact_retained_source() -> Result<()> {
    let retained = source(11, true);
    let original = retained.anchor();
    let mut later = retained.clone();
    later.tick = GameTick(11);
    later.cursor.sequence += 1;
    later.refresh_hash();
    let selection = query(comparison(EntityId::NIL));
    assert_eq!(execute_query(&later, &selection, 10)?.matched, 1);
    assert_eq!(execute_query(&retained, &selection, 10)?.matched, 0);
    assert_eq!(
        evaluate_truth_for(&retained, SUBJECT, &comparison(EntityId::NIL)),
        PredicateTruth::Unknown
    );
    assert_eq!(retained.anchor(), original);
    assert!(retained.hash_is_valid());
    Ok(())
}
