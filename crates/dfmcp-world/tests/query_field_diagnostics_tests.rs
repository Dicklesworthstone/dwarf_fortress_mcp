//! Source-bound query diagnostics are never game-state mutation evidence.

use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{Digest32, EntityId, ErrorCode, FortressId, GameTick, ObservationCursor, Result};
use dfmcp_world::{
    CompareOp, EntityKind, EntityRecord, EvidenceCoverage, EvidencePolicy, EvidenceSource, Fact,
    FactPresence, FactSource, FieldPresenceState, Predicate, PredicateEvidence, PredicateTruth,
    QueryOrder, Value, WorldGraph, WorldQuery, WorldSnapshot, evaluate_truth_for, execute_query,
};

const ALL_STATES: [FieldPresenceState; 10] = [
    FieldPresenceState::Known,
    FieldPresenceState::Absent,
    FieldPresenceState::Unknown,
    FieldPresenceState::Unsupported,
    FieldPresenceState::Omitted,
    FieldPresenceState::Redacted,
    FieldPresenceState::Stale,
    FieldPresenceState::Unobserved,
    FieldPresenceState::Contradicted,
    FieldPresenceState::Future,
];

fn fact(presence: Option<FactPresence>, observed_at: u64) -> Fact {
    Fact {
        value: Value::U64(7),
        observed_at: GameTick(observed_at),
        source: FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
        source_digest: Digest32::ZERO,
        presence,
    }
}

fn world(facts: Vec<Option<Fact>>, tick: u64) -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    for (index, fact) in facts.into_iter().enumerate() {
        let id = EntityId::new(index as u64 + 1);
        let mut fields = BTreeMap::new();
        if let Some(fact) = fact {
            fields.insert("stock".to_owned(), fact.clone());
            fields.insert("second".to_owned(), fact);
        }
        graph.entities.insert(
            id,
            EntityRecord {
                id,
                generation: 1,
                revision: 1,
                kind: EntityKind::Item,
                label: format!("item-{}", id.get()),
                fields,
            },
        );
    }
    WorldSnapshot::new(
        FortressId::new(1),
        GameTick(tick),
        ObservationCursor::ORIGIN,
        true,
        graph,
    )
}

fn presence(state: FieldPresenceState) -> Predicate {
    Predicate::FieldPresence {
        entity_id: EntityId::NIL,
        field: "stock".to_owned(),
        state,
    }
}

fn freshness(max_age_ticks: u64) -> Predicate {
    Predicate::FieldFreshness {
        entity_id: EntityId::NIL,
        field: "stock".to_owned(),
        max_age_ticks,
    }
}

fn query(predicate: Predicate) -> WorldQuery {
    WorldQuery {
        kinds: vec![EntityKind::Item],
        predicate: Some(predicate),
        order: QueryOrder::EntityIdAscending,
        limit: 2,
        continuation: None,
    }
}

#[test]
fn every_presence_category_is_disjoint_and_values_stay_unavailable() -> Result<()> {
    let source = world(vec![], 20);
    let cases = [
        (Some(fact(None, 20)), FieldPresenceState::Known),
        (
            Some(fact(Some(FactPresence::Known(Value::U64(7))), 19)),
            FieldPresenceState::Known,
        ),
        (
            Some(fact(Some(FactPresence::Absent), 19)),
            FieldPresenceState::Absent,
        ),
        (
            Some(fact(Some(FactPresence::Unknown("missing".to_owned())), 30)),
            FieldPresenceState::Unknown,
        ),
        (
            Some(fact(
                Some(FactPresence::Unsupported("version".to_owned())),
                30,
            )),
            FieldPresenceState::Unsupported,
        ),
        (
            Some(fact(
                Some(FactPresence::Omitted("projection".to_owned())),
                30,
            )),
            FieldPresenceState::Omitted,
        ),
        (
            Some(fact(Some(FactPresence::Redacted("policy".to_owned())), 30)),
            FieldPresenceState::Redacted,
        ),
        (
            Some(fact(Some(FactPresence::Stale(source.anchor())), 30)),
            FieldPresenceState::Stale,
        ),
        (None, FieldPresenceState::Unobserved),
        (
            Some(fact(Some(FactPresence::Known(Value::U64(8))), 30)),
            FieldPresenceState::Contradicted,
        ),
        (Some(fact(None, 21)), FieldPresenceState::Future),
        (
            Some(fact(Some(FactPresence::Known(Value::U64(7))), 21)),
            FieldPresenceState::Future,
        ),
        (
            Some(fact(Some(FactPresence::Absent), 21)),
            FieldPresenceState::Future,
        ),
    ];
    for (fact, expected) in cases {
        let snapshot = world(vec![fact], 20);
        for state in ALL_STATES {
            let expected_truth = if state == expected {
                PredicateTruth::True
            } else {
                PredicateTruth::False
            };
            assert_eq!(
                evaluate_truth_for(&snapshot, EntityId::new(1), &presence(state)),
                expected_truth,
                "expected {expected:?}, inspected {state:?}"
            );
            let result = execute_query(&snapshot, &query(presence(state)), 100)?;
            assert_eq!(result.matched, u64::from(state == expected));
        }
        let compare = Predicate::FieldCompare {
            entity_id: EntityId::NIL,
            field: "stock".to_owned(),
            op: CompareOp::Eq,
            value: Value::U64(7),
        };
        assert_eq!(
            evaluate_truth_for(&snapshot, EntityId::new(1), &compare),
            if expected == FieldPresenceState::Known {
                PredicateTruth::True
            } else {
                PredicateTruth::Unknown
            },
            "presence diagnostics must not change ordinary field comparison"
        );
    }
    // A legitimate known null is distinct from absence and missing metadata.
    let null = Fact::known(
        Value::Null,
        GameTick(20),
        FactSource::Replay,
        Digest32::ZERO,
    );
    assert_eq!(
        evaluate_truth_for(
            &world(vec![Some(null)], 20),
            EntityId::new(1),
            &presence(FieldPresenceState::Known)
        ),
        PredicateTruth::True
    );
    Ok(())
}

#[test]
fn missing_entities_remain_unknown_and_explicit_targets_do_not_follow_rows() {
    let snapshot = world(vec![Some(fact(None, 20)), None], 20);
    for state in ALL_STATES {
        let predicate = presence(state);
        assert_eq!(
            evaluate_truth_for(&snapshot, EntityId::new(99), &predicate),
            PredicateTruth::Unknown
        );
        assert_eq!(
            evaluate_truth_for(
                &snapshot,
                EntityId::new(99),
                &Predicate::Not(Box::new(predicate))
            ),
            PredicateTruth::Unknown
        );
    }
    let absolute = Predicate::FieldPresence {
        entity_id: EntityId::new(1),
        field: "stock".to_owned(),
        state: FieldPresenceState::Known,
    };
    assert_eq!(
        evaluate_truth_for(&snapshot, EntityId::new(2), &absolute),
        PredicateTruth::True
    );
    assert_eq!(
        evaluate_truth_for(
            &snapshot,
            EntityId::new(2),
            &presence(FieldPresenceState::Known)
        ),
        PredicateTruth::False
    );
}

#[test]
fn freshness_uses_inclusive_selected_source_age_without_underflow() {
    for (tick, observed_at, maximum, expected) in [
        (20, 20, 0, PredicateTruth::True),
        (20, 19, 0, PredicateTruth::False),
        (20, 15, 5, PredicateTruth::True),
        (20, 15, 4, PredicateTruth::False),
        (20, 21, u64::MAX, PredicateTruth::Unknown),
        (0, 1, u64::MAX, PredicateTruth::Unknown),
        (u64::MAX, 0, u64::MAX, PredicateTruth::True),
        (u64::MAX, 0, u64::MAX - 1, PredicateTruth::False),
        (u64::MAX, u64::MAX, 0, PredicateTruth::True),
    ] {
        let snapshot = world(vec![Some(fact(None, observed_at))], tick);
        let predicate = freshness(maximum);
        assert_eq!(
            evaluate_truth_for(&snapshot, EntityId::new(1), &predicate),
            expected
        );
        let negated = match expected {
            PredicateTruth::True => PredicateTruth::False,
            PredicateTruth::False => PredicateTruth::True,
            PredicateTruth::Unknown => PredicateTruth::Unknown,
        };
        assert_eq!(
            evaluate_truth_for(
                &snapshot,
                EntityId::new(1),
                &Predicate::Not(Box::new(predicate))
            ),
            negated
        );
    }
    let absent = world(vec![], 20).anchor();
    for unavailable in [
        None,
        Some(fact(Some(FactPresence::Absent), 20)),
        Some(fact(Some(FactPresence::Unknown("missing".to_owned())), 20)),
        Some(fact(
            Some(FactPresence::Unsupported("version".to_owned())),
            20,
        )),
        Some(fact(
            Some(FactPresence::Omitted("projection".to_owned())),
            20,
        )),
        Some(fact(Some(FactPresence::Redacted("policy".to_owned())), 20)),
        Some(fact(Some(FactPresence::Stale(absent)), 20)),
        Some(fact(Some(FactPresence::Known(Value::U64(8))), 20)),
    ] {
        let snapshot = world(vec![unavailable], 20);
        assert_eq!(
            evaluate_truth_for(&snapshot, EntityId::new(1), &freshness(u64::MAX)),
            PredicateTruth::Unknown
        );
        assert_eq!(
            evaluate_truth_for(
                &snapshot,
                EntityId::new(1),
                &Predicate::Not(Box::new(freshness(u64::MAX)))
            ),
            PredicateTruth::Unknown
        );
    }
}

#[test]
fn diagnostics_do_not_supply_laboratory_or_live_mutation_evidence() -> Result<()> {
    let mut snapshot = world(vec![Some(fact(None, 20))], 20);
    let mut predicates = Vec::new();
    for state in ALL_STATES {
        predicates.push(Predicate::FieldPresence {
            entity_id: EntityId::new(1),
            field: "stock".to_owned(),
            state,
        });
    }
    predicates.push(Predicate::FieldProvenance {
        entity_id: EntityId::new(1),
        field: "stock".to_owned(),
        source: dfmcp_world::FactProvenanceFilter {
            kind: dfmcp_world::FactSourceKind::Derived,
            name: None,
            digest: None,
        },
    });
    predicates.push(Predicate::FieldFreshness {
        entity_id: EntityId::new(1),
        field: "stock".to_owned(),
        max_age_ticks: 0,
    });
    for live in [false, true] {
        let mut policy = EvidencePolicy::at(snapshot.anchor());
        policy.all_entities = EvidenceCoverage::Complete;
        let evidence = if live {
            let digest = Digest32::of_bytes(b"admitted-source");
            for entity in snapshot.graph.entities.values_mut() {
                for field in entity.fields.values_mut() {
                    field.source = FactSource::DfhackField("stock".to_owned());
                    field.source_digest = digest;
                }
            }
            snapshot.refresh_hash();
            policy.anchor = Some(snapshot.anchor());
            policy.sources.insert(EvidenceSource::Observed {
                field: "stock".to_owned(),
                source_digest: digest,
            });
            PredicateEvidence::scoped(&snapshot, policy)?
        } else {
            PredicateEvidence::laboratory(&snapshot)?
        };
        for predicate in &predicates {
            assert_eq!(evidence.evaluate(predicate)?, PredicateTruth::Unknown);
            assert_eq!(
                evidence.evaluate(&Predicate::Not(Box::new(predicate.clone())))?,
                PredicateTruth::Unknown
            );
            assert!(!evidence.establishes(predicate)?);
        }
        assert!(evidence.establishes(&Predicate::FieldCompare {
            entity_id: EntityId::new(1),
            field: "stock".to_owned(),
            op: CompareOp::Eq,
            value: Value::U64(7),
        })?);
    }
    Ok(())
}

#[test]
fn diagnostic_cursors_bind_every_parameter_and_allow_page_width_changes() -> Result<()> {
    let snapshot = world(vec![Some(fact(None, 15)); 7], 20);
    for original in [presence(FieldPresenceState::Known), freshness(5)] {
        let mut request = query(original.clone());
        let first = execute_query(&snapshot, &request, 100)?;
        assert_eq!(first.matched, 7);
        assert!(first.continuation.is_some());
        request.continuation = first.continuation;
        request.limit = 3;
        let next = execute_query(&snapshot, &request, 100)?;
        assert_eq!(
            next.entities
                .iter()
                .map(|entity| entity.id.get())
                .collect::<Vec<_>>(),
            vec![3, 4, 5]
        );
        let mut variants = vec![presence(FieldPresenceState::Unknown), freshness(4)];
        let mut changed_field = original.clone();
        let mut changed_target = original.clone();
        for (predicate, target) in [(&mut changed_field, false), (&mut changed_target, true)] {
            match predicate {
                Predicate::FieldPresence {
                    entity_id, field, ..
                }
                | Predicate::FieldFreshness {
                    entity_id, field, ..
                } => {
                    if target {
                        *entity_id = EntityId::new(1);
                    } else {
                        *field = "second".to_owned();
                    }
                }
                _ => {}
            }
        }
        variants.extend([changed_field, changed_target]);
        for predicate in variants {
            let mut changed = request.clone();
            changed.predicate = Some(predicate);
            assert!(
                matches!(execute_query(&snapshot, &changed, 100), Err(error) if error.code == ErrorCode::StaleAnchor)
            );
        }
        let mut later = snapshot.clone();
        later.tick = GameTick(21);
        later.refresh_hash();
        assert!(
            matches!(execute_query(&later, &request, 100), Err(error) if error.code == ErrorCode::StaleAnchor)
        );
    }
    Ok(())
}

#[test]
fn diagnostic_identity_is_distinct_and_full_shape_is_checked_before_short_circuits() -> Result<()> {
    let identities = ALL_STATES
        .into_iter()
        .map(|state| presence(state).canonical_bytes())
        .collect::<BTreeSet<_>>();
    assert_eq!(identities.len(), ALL_STATES.len());
    assert_eq!(
        ALL_STATES
            .into_iter()
            .map(FieldPresenceState::as_str)
            .collect::<BTreeSet<_>>()
            .len(),
        ALL_STATES.len()
    );
    assert_ne!(
        freshness(0).canonical_bytes(),
        freshness(1).canonical_bytes()
    );
    assert!(!identities.contains(&freshness(0).canonical_bytes()));
    assert_eq!(Predicate::Paused(true).canonical_bytes(), vec![6, 1]);
    let snapshot = world(vec![Some(fact(None, 20))], 20);
    for field in [
        String::new(),
        "stock\0".to_owned(),
        "stock\n".to_owned(),
        "x".repeat(257),
    ] {
        for predicate in [
            Predicate::FieldPresence {
                entity_id: EntityId::NIL,
                field: field.clone(),
                state: FieldPresenceState::Known,
            },
            Predicate::FieldFreshness {
                entity_id: EntityId::NIL,
                field: field.clone(),
                max_age_ticks: 0,
            },
        ] {
            assert!(predicate.validate_shape().is_err());
            assert_eq!(
                evaluate_truth_for(&snapshot, EntityId::new(1), &predicate),
                PredicateTruth::Unknown
            );
            assert!(
                execute_query(
                    &snapshot,
                    &query(Predicate::All(vec![Predicate::False, predicate])),
                    100
                )
                .is_err()
            );
        }
    }
    let mut deep = freshness(0);
    for _ in 0..65 {
        deep = Predicate::Not(Box::new(deep));
    }
    assert!(matches!(deep.validate_shape(), Err(error) if error.code == ErrorCode::BudgetExceeded));
    let broad = Predicate::All(vec![freshness(0); 4_096]);
    assert!(
        matches!(broad.validate_shape(), Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    assert!(
        matches!(execute_query(&world(vec![Some(fact(None, 20)); 1_100], 20), &query(Predicate::All(vec![freshness(0); 4_000])), 100), Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn provenance_matches_one_eligible_source_record_and_binds_continuations() -> Result<()> {
    use dfmcp_world::{FactProvenanceFilter, FactSourceKind};

    let mut other_origin = fact(None, 20);
    other_origin.source = FactSource::AgentAssertion("dfmcp.lab-scenario/1".to_owned());
    let snapshot = world(
        vec![
            Some(fact(None, 20)),
            Some(fact(Some(FactPresence::Absent), 20)),
            Some(other_origin),
            Some(fact(Some(FactPresence::Redacted("private".to_owned())), 20)),
            Some(fact(Some(FactPresence::Known(Value::U64(8))), 20)),
            Some(fact(None, 21)),
            None,
        ],
        20,
    );
    let source = FactProvenanceFilter {
        kind: FactSourceKind::Derived,
        name: None,
        digest: None,
    };
    let select = |source| Predicate::FieldProvenance {
        entity_id: EntityId::NIL,
        field: "stock".to_owned(),
        source,
    };
    let predicate = select(source.clone());
    let negated = Predicate::Not(Box::new(predicate.clone()));
    for (index, (result, inverse)) in [
        (PredicateTruth::True, PredicateTruth::False),
        (PredicateTruth::True, PredicateTruth::False),
        (PredicateTruth::False, PredicateTruth::True),
        (PredicateTruth::Unknown, PredicateTruth::Unknown),
        (PredicateTruth::Unknown, PredicateTruth::Unknown),
        (PredicateTruth::Unknown, PredicateTruth::Unknown),
        (PredicateTruth::Unknown, PredicateTruth::Unknown),
    ]
    .into_iter()
    .enumerate()
    {
        let id = EntityId::new(index as u64 + 1);
        assert_eq!(evaluate_truth_for(&snapshot, id, &predicate), result);
        assert_eq!(evaluate_truth_for(&snapshot, id, &negated), inverse);
    }
    let mut request = query(predicate);
    request.limit = 1;
    request.continuation = execute_query(&snapshot, &request, 100)?.continuation;
    assert!(request.continuation.is_some());
    assert_eq!(execute_query(&snapshot, &request, 100)?.entities[0].id, EntityId::new(2));
    for changed in [
        FactProvenanceFilter { kind: FactSourceKind::AgentAssertion, ..source.clone() },
        FactProvenanceFilter { name: Some("dfmcp.lab-scenario/1".to_owned()), ..source.clone() },
        FactProvenanceFilter { digest: Some(Digest32::ZERO), ..source },
    ] {
        request.predicate = Some(select(changed));
        assert!(matches!(execute_query(&snapshot, &request, 100),
            Err(error) if error.code == ErrorCode::StaleAnchor));
    }
    Ok(())
}
