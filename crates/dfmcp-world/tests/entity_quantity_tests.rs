//! Whole-domain inventory predicates, without production or source admission.
use std::collections::BTreeMap;

use dfmcp_core::{
    DfmcpError, Digest32, EntityId, ErrorCode, FortressId, GameTick, ObservationCursor, Result,
};
use dfmcp_world::{
    CompareOp, EntityKind, EntityRecord, EvidenceCoverage, EvidencePolicy, EvidenceSource, Fact,
    FactPresence, FactSource, Predicate, PredicateEvidence, PredicateTruth, QueryOrder, Value,
    WorldGraph, WorldQuery, WorldSnapshot, evaluate, evaluate_truth_for, execute_query,
};

fn digest() -> Digest32 {
    Digest32::of_bytes(b"complete inventory observation")
}

fn fact(field: &str, value: Value) -> Fact {
    Fact::known(
        value,
        GameTick(10),
        FactSource::DfhackField(format!("item.{field}")),
        digest(),
    )
}

fn snapshot(rows: &[(u64, &str, u64)]) -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    for (id, kind, units) in rows {
        let id = EntityId::new(*id);
        graph.entities.insert(
            id,
            EntityRecord {
                id,
                generation: 1,
                revision: 1,
                kind: EntityKind::Item,
                label: format!("item-{id}"),
                fields: BTreeMap::from([
                    (
                        "type_key".into(),
                        fact("type_key", Value::Text((*kind).into())),
                    ),
                    ("stack_size".into(), fact("stack_size", Value::U64(*units))),
                ]),
            },
        );
    }
    WorldSnapshot::new(
        FortressId::new(1),
        GameTick(10),
        ObservationCursor {
            epoch: 2,
            sequence: 3,
        },
        true,
        graph,
    )
}

fn policy(snapshot: &WorldSnapshot) -> EvidencePolicy {
    let mut policy = EvidencePolicy::at(snapshot.anchor());
    policy
        .entity_kinds
        .insert(EntityKind::Item, EvidenceCoverage::Complete);
    for field in ["type_key", "stack_size"] {
        policy.sources.insert(EvidenceSource::Observed {
            field: format!("item.{field}"),
            source_digest: digest(),
        });
    }
    policy
}

fn quantity(field: Option<&str>, op: CompareOp, value: u64) -> Predicate {
    Predicate::EntityQuantity {
        kind: EntityKind::Item,
        filter_field: "type_key".into(),
        filter_value: Value::Text("BED".into()),
        quantity_field: field.map(str::to_owned),
        op,
        value,
    }
}

fn change_fact(snapshot: &mut WorldSnapshot, id: u64, field: &str, fact: Fact) -> Result<()> {
    let row = snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(id))
        .ok_or_else(|| {
            DfmcpError::new(ErrorCode::InternalInvariantViolation, "fixture row missing")
        })?;
    row.fields.insert(field.into(), fact);
    snapshot.refresh_hash();
    Ok(())
}

fn unknown(evidence: &PredicateEvidence<'_>, predicate: &Predicate) -> Result<()> {
    for predicate in [
        predicate.clone(),
        Predicate::Not(Box::new(predicate.clone())),
        Predicate::All(vec![Predicate::True, predicate.clone()]),
        Predicate::Any(vec![Predicate::False, predicate.clone()]),
    ] {
        assert_eq!(evidence.evaluate(&predicate)?, PredicateTruth::Unknown);
        assert!(!evidence.establishes(&predicate)?);
    }
    Ok(())
}

#[test]
fn exact_record_count_and_stack_sum_have_distinct_meanings() -> Result<()> {
    let snapshot = snapshot(&[(1, "BED", 2), (2, "DOOR", 99), (3, "BED", 3)]);
    let evidence = PredicateEvidence::scoped(&snapshot, policy(&snapshot))?;
    for (field, total) in [(None, 2), (Some("stack_size"), 5)] {
        for (op, threshold, expected) in [
            (CompareOp::Eq, total, PredicateTruth::True),
            (CompareOp::Ne, total, PredicateTruth::False),
            (CompareOp::Lt, total + 1, PredicateTruth::True),
            (CompareOp::Le, total - 1, PredicateTruth::False),
            (CompareOp::Gt, total - 1, PredicateTruth::True),
            (CompareOp::Ge, total + 1, PredicateTruth::False),
        ] {
            let predicate = quantity(field, op, threshold);
            assert_eq!(evidence.evaluate(&predicate)?, expected);
            assert_eq!(
                evaluate(&snapshot, &predicate),
                expected == PredicateTruth::True
            );
        }
    }
    Ok(())
}

#[test]
fn complete_domain_is_required_even_for_positive_subtotals_and_empty_inventory() -> Result<()> {
    for snapshot in [snapshot(&[]), snapshot(&[(1, "BED", 5)])] {
        for coverage in [EvidenceCoverage::Unknown, EvidenceCoverage::Observed] {
            let mut partial = policy(&snapshot);
            partial.entity_kinds.insert(EntityKind::Item, coverage);
            let evidence = PredicateEvidence::scoped(&snapshot, partial)?;
            unknown(&evidence, &quantity(Some("stack_size"), CompareOp::Ge, 1))?;
            unknown(&evidence, &quantity(None, CompareOp::Eq, 0))?;
        }
        let mut complete = policy(&snapshot);
        complete.entity_kinds.clear();
        complete.all_entities = EvidenceCoverage::Complete;
        assert_eq!(
            PredicateEvidence::scoped(&snapshot, complete)?.evaluate(&quantity(
                None,
                CompareOp::Eq,
                0
            ))?,
            if snapshot.graph.entities.is_empty() {
                PredicateTruth::True
            } else {
                PredicateTruth::False
            }
        );
    }
    let empty = snapshot(&[]);
    let mut complete = policy(&empty);
    complete.sources.clear();
    assert!(
        PredicateEvidence::scoped(&empty, complete)?.establishes(&quantity(
            Some("stack_size"),
            CompareOp::Eq,
            0
        ))?
    );
    Ok(())
}

#[test]
fn every_unavailable_or_contradictory_input_keeps_the_full_quantity_unknown() -> Result<()> {
    let base = snapshot(&[(1, "BED", 5), (2, "BED", 1)]);
    let goal = quantity(Some("stack_size"), CompareOp::Ge, 1);
    for field in ["type_key", "stack_size"] {
        for presence in [
            FactPresence::Absent,
            FactPresence::Unknown("not read".into()),
            FactPresence::Unsupported("manifest".into()),
            FactPresence::Omitted("projection".into()),
            FactPresence::Redacted("policy".into()),
            FactPresence::Stale(base.anchor()),
            FactPresence::Known(Value::Bool(false)),
        ] {
            let mut current = base.clone();
            let mut unavailable = fact(
                field,
                if field == "type_key" {
                    Value::Text("BED".into())
                } else {
                    Value::U64(1)
                },
            );
            // Retained compatibility values must not satisfy a subtotal proof.
            unavailable.presence = Some(presence);
            change_fact(&mut current, 2, field, unavailable)?;
            unknown(
                &PredicateEvidence::scoped(&current, policy(&current))?,
                &goal,
            )?;
            assert_eq!(
                evaluate_truth_for(&current, EntityId::new(1), &goal),
                PredicateTruth::Unknown
            );
        }
        let mut current = base.clone();
        if let Some(row) = current.graph.entities.get_mut(&EntityId::new(2)) {
            row.fields.remove(field);
        }
        current.refresh_hash();
        unknown(
            &PredicateEvidence::scoped(&current, policy(&current))?,
            &goal,
        )?;
    }
    let mut explicit = base;
    for row in explicit.graph.entities.values_mut() {
        for fact in row.fields.values_mut() {
            fact.presence = Some(FactPresence::Known(fact.value.clone()));
        }
    }
    explicit.refresh_hash();
    assert!(PredicateEvidence::scoped(&explicit, policy(&explicit))?.establishes(&goal)?);
    Ok(())
}

#[test]
fn each_selector_and_matching_quantity_needs_independent_source_qualification() -> Result<()> {
    let base = snapshot(&[(1, "BED", 5), (2, "BED", 1)]);
    let goal = quantity(Some("stack_size"), CompareOp::Ge, 1);
    for field in ["type_key", "stack_size"] {
        for source in [
            FactSource::Replay,
            FactSource::AgentAssertion("agent".into()),
            FactSource::Derived("unregistered".into()),
            FactSource::DfhackField("another.field".into()),
        ] {
            let mut current = base.clone();
            let mut changed = fact(
                field,
                if field == "type_key" {
                    Value::Text("BED".into())
                } else {
                    Value::U64(1)
                },
            );
            changed.source = source;
            change_fact(&mut current, 2, field, changed)?;
            unknown(
                &PredicateEvidence::scoped(&current, policy(&current))?,
                &goal,
            )?;
        }
        for future in [false, true] {
            let mut current = base.clone();
            let mut changed = fact(
                field,
                if field == "type_key" {
                    Value::Text("BED".into())
                } else {
                    Value::U64(1)
                },
            );
            if future {
                changed.observed_at = GameTick(11);
            } else {
                changed.source_digest = Digest32::of_bytes(b"another observation");
            }
            change_fact(&mut current, 2, field, changed)?;
            unknown(
                &PredicateEvidence::scoped(&current, policy(&current))?,
                &goal,
            )?;
        }
        let mut narrowed = policy(&base);
        narrowed.sources.retain(|source| {
            !matches!(source,
            EvidenceSource::Observed { field: name, .. } if name == &format!("item.{field}"))
        });
        unknown(&PredicateEvidence::scoped(&base, narrowed)?, &goal)?;
    }
    Ok(())
}

#[test]
fn nonmatching_quantities_are_irrelevant_but_their_selectors_are_required() -> Result<()> {
    let mut current = snapshot(&[(1, "BED", 3), (2, "DOOR", 99)]);
    let goal = quantity(Some("stack_size"), CompareOp::Eq, 3);
    let mut unavailable = fact("stack_size", Value::U64(99));
    unavailable.presence = Some(FactPresence::Redacted("policy".into()));
    change_fact(&mut current, 2, "stack_size", unavailable)?;
    assert!(PredicateEvidence::scoped(&current, policy(&current))?.establishes(&goal)?);
    let mut selector = fact("type_key", Value::Text("DOOR".into()));
    selector.presence = Some(FactPresence::Unknown("not observed".into()));
    change_fact(&mut current, 2, "type_key", selector)?;
    unknown(
        &PredicateEvidence::scoped(&current, policy(&current))?,
        &goal,
    )?;
    Ok(())
}

#[test]
fn aggregate_inputs_retain_the_existing_certified_and_laboratory_source_boundaries() -> Result<()> {
    let goal = quantity(Some("stack_size"), CompareOp::Eq, 2);
    let mut current = snapshot(&[(1, "BED", 2)]);
    for row in current.graph.entities.values_mut() {
        for fact in row.fields.values_mut() {
            fact.source = FactSource::Derived("registered.inventory/1".into());
        }
    }
    current.refresh_hash();
    unknown(
        &PredicateEvidence::scoped(&current, policy(&current))?,
        &goal,
    )?;
    let mut certified = policy(&current);
    certified.sources.clear();
    certified.sources.insert(EvidenceSource::CertifiedDerived {
        derivation: "registered.inventory/1".into(),
        source_digest: digest(),
    });
    assert!(PredicateEvidence::scoped(&current, certified)?.establishes(&goal)?);
    unknown(&PredicateEvidence::laboratory(&current)?, &goal)?;
    for row in current.graph.entities.values_mut() {
        for fact in row.fields.values_mut() {
            fact.source = FactSource::Derived("dfmcp.lab-scenario/1".into());
            fact.source_digest = Digest32::ZERO;
        }
    }
    current.refresh_hash();
    assert!(PredicateEvidence::laboratory(&current)?.establishes(&goal)?);
    unknown(
        &PredicateEvidence::scoped(&current, policy(&current))?,
        &goal,
    )?;
    Ok(())
}

#[test]
fn sums_never_coerce_invalid_numbers_or_wrap_overflow_and_zero_remains_known() -> Result<()> {
    let goal = quantity(Some("stack_size"), CompareOp::Ge, 1);
    for invalid in [
        Value::Null,
        Value::I64(-1),
        Value::I64(1),
        Value::Text("1".into()),
    ] {
        let mut current = snapshot(&[(1, "BED", 4), (2, "BED", 1)]);
        change_fact(&mut current, 2, "stack_size", fact("stack_size", invalid))?;
        unknown(
            &PredicateEvidence::scoped(&current, policy(&current))?,
            &goal,
        )?;
    }
    let overflow = snapshot(&[(1, "BED", u64::MAX), (2, "BED", 1)]);
    let evidence = PredicateEvidence::scoped(&overflow, policy(&overflow))?;
    unknown(&evidence, &goal)?;
    assert!(evidence.establishes(&quantity(None, CompareOp::Eq, 2))?);
    let zero = snapshot(&[(1, "BED", 0)]);
    let evidence = PredicateEvidence::scoped(&zero, policy(&zero))?;
    assert!(evidence.establishes(&quantity(Some("stack_size"), CompareOp::Eq, 0))?);
    assert!(evidence.establishes(&quantity(None, CompareOp::Eq, 1))?);
    Ok(())
}

#[test]
fn canonical_kind_aliases_cannot_change_aggregate_truth_for_identical_world_bytes() -> Result<()> {
    let original = snapshot(&[(1, "BED", 2)]);
    let mut alias = original.clone();
    if let Some(row) = alias.graph.entities.get_mut(&EntityId::new(1)) {
        row.kind = EntityKind::Other("item".into());
    }
    alias.refresh_hash();
    assert_eq!(alias.state_hash, original.state_hash);
    let goal = quantity(Some("stack_size"), CompareOp::Eq, 2);
    let mut alias_goal = goal.clone();
    if let Predicate::EntityQuantity { kind, .. } = &mut alias_goal {
        *kind = EntityKind::Other("item".into());
    }
    assert_eq!(goal.canonical_bytes(), alias_goal.canonical_bytes());
    for current in [&original, &alias] {
        for predicate in [&goal, &alias_goal] {
            assert!(PredicateEvidence::scoped(current, policy(current))?.establishes(predicate)?);
            assert!(evaluate(current, predicate));
        }
    }
    Ok(())
}

#[test]
fn aggregate_encoding_appends_tag_twelve_and_boolean_normalization_remains_deterministic() {
    let predicate = quantity(Some("stack_size"), CompareOp::Ge, 3);
    let encoded = predicate
        .canonical_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    assert_eq!(
        encoded,
        concat!(
            "0c00000000000000046974656d0000000000000008747970655f6b6579",
            "05000000000000000342454401000000000000000a737461636b5f73697a65",
            "050000000000000003"
        )
    );
    assert_eq!(Predicate::Paused(false).canonical_bytes(), vec![6, 0]);
    assert_eq!(
        Predicate::All(vec![
            predicate.clone(),
            Predicate::Paused(true),
            predicate.clone()
        ])
        .canonical_bytes(),
        Predicate::All(vec![Predicate::Paused(true), predicate]).canonical_bytes()
    );
}

#[test]
fn aggregate_inputs_and_total_scan_work_are_validated_before_short_circuiting() -> Result<()> {
    let rows = (1..=1200).map(|id| (id, "BED", 1)).collect::<Vec<_>>();
    let current = snapshot(&rows);
    let evidence = PredicateEvidence::scoped(&current, policy(&current))?;
    let predicate = quantity(Some("stack_size"), CompareOp::Ge, 1);
    let oversized = Predicate::Any(vec![
        Predicate::True,
        Predicate::All(vec![predicate.clone(); 1200]),
    ]);
    assert!(
        matches!(evidence.evaluate(&oversized), Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    let query = WorldQuery {
        kinds: vec![EntityKind::Item],
        predicate: Some(predicate),
        order: QueryOrder::EntityIdAscending,
        limit: 1,
        continuation: None,
    };
    assert!(
        matches!(execute_query(&current, &query, 10), Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    for which in 0..6 {
        let mut invalid = quantity(Some("stack_size"), CompareOp::Ge, 1);
        if let Predicate::EntityQuantity {
            kind,
            filter_field,
            filter_value,
            quantity_field,
            ..
        } = &mut invalid
        {
            match which {
                0 => *kind = EntityKind::Other("x".repeat(129)),
                1 => *filter_field = "x".repeat(257),
                2 => *quantity_field = Some("x".repeat(257)),
                3 => *filter_field = String::new(),
                4 => *quantity_field = Some("stack\nsize".into()),
                _ => *filter_value = Value::Text("x".repeat(65537)),
            }
        }
        assert!(
            evidence
                .evaluate(&Predicate::Any(vec![Predicate::True, invalid]))
                .is_err()
        );
    }
    Ok(())
}

#[test]
fn query_continuations_bind_every_aggregate_component() -> Result<()> {
    let current = snapshot(&[(1, "BED", 2), (2, "DOOR", 1), (3, "BED", 3)]);
    let mut query = WorldQuery {
        kinds: vec![EntityKind::Item],
        predicate: Some(quantity(Some("stack_size"), CompareOp::Ge, 1)),
        order: QueryOrder::EntityIdAscending,
        limit: 1,
        continuation: None,
    };
    let first = execute_query(&current, &query, 10)?;
    assert_eq!(first.matched, 3);
    assert_eq!(first.entities[0].id, EntityId::new(1));
    query.continuation = first.continuation;
    assert!(query.continuation.is_some());
    assert_eq!(
        execute_query(&current, &query, 10)?.entities[0].id,
        EntityId::new(2)
    );
    for which in 0..7 {
        let mut changed = query.clone();
        if let Some(Predicate::EntityQuantity {
            kind,
            filter_field,
            filter_value,
            quantity_field,
            op,
            value,
        }) = &mut changed.predicate
        {
            match which {
                0 => *kind = EntityKind::Building,
                1 => *kind = EntityKind::Other("item".into()),
                2 => *filter_field = "subtype".into(),
                3 => *filter_value = Value::Text("DOOR".into()),
                4 => *quantity_field = None,
                5 => *op = CompareOp::Eq,
                _ => *value = 2,
            }
        }
        assert!(
            matches!(execute_query(&current, &changed, 10), Err(error) if error.code == ErrorCode::StaleAnchor)
        );
    }
    Ok(())
}

fn increase(original: &WorldSnapshot, baseline: u64, units: u64) -> Predicate {
    Predicate::EntityQuantityIncrease {
        kind: EntityKind::Item,
        filter_field: "type_key".into(),
        filter_value: Value::Text("BED".into()),
        quantity_field: Some("stack_size".into()),
        baseline,
        increase: units,
        baseline_anchor: original.anchor(),
        baseline_source_digest: digest(),
        source_generation: 7,
        baseline_source_counters: [11, 12, 13],
    }
}

fn later(rows: &[(u64, &str, u64)]) -> WorldSnapshot {
    let mut current = snapshot(rows);
    current.tick = GameTick(11);
    current.cursor.sequence += 1;
    current.refresh_hash();
    current
}

#[test]
fn increase_uses_sealed_stack_total_and_measures_net_growth_not_item_creation() -> Result<()> {
    let original = snapshot(&[(1, "BED", 2), (2, "DOOR", 99), (3, "BED", 3)]);
    let evidence = PredicateEvidence::scoped(&original, policy(&original))?;
    let baseline = evidence.entity_quantity_total(
        &EntityKind::Item, "type_key", &Value::Text("BED".into()), Some("stack_size"),
    )?;
    assert_eq!(baseline, Some(5));
    assert_eq!(evidence.entity_quantity_total(
        &EntityKind::Item, "type_key", &Value::Text("BED".into()), None,
    )?, Some(2));
    let goal = increase(&original, 5, 3);
    assert_eq!(evidence.evaluate(&goal)?, PredicateTruth::False);
    // Replacing every ID without growing stock is not an increase. Increasing
    // stack units on the same two IDs is enough; causal production is unknown.
    for (rows, expected) in [
        (vec![(4, "BED", 2), (5, "BED", 3)], PredicateTruth::False),
        (vec![(1, "BED", 1), (3, "BED", 3)], PredicateTruth::False),
        (vec![(1, "BED", 4), (3, "BED", 3)], PredicateTruth::False),
        (vec![(1, "BED", 4), (3, "BED", 4)], PredicateTruth::True),
    ] {
        let current = later(&rows);
        assert_eq!(PredicateEvidence::scoped(&current, policy(&current))?.evaluate(&goal)?, expected);
        assert_eq!(evaluate(&current, &goal), expected == PredicateTruth::True);
    }
    Ok(())
}

#[test]
fn qualified_baseline_measurement_distinguishes_zero_from_unknown() -> Result<()> {
    let selector = Value::Text("BED".into());
    let empty = snapshot(&[]);
    let evidence = PredicateEvidence::scoped(&empty, policy(&empty))?;
    assert_eq!(evidence.entity_quantity_total(
        &EntityKind::Item, "type_key", &selector, Some("stack_size"),
    )?, Some(0));
    for coverage in [EvidenceCoverage::Unknown, EvidenceCoverage::Observed] {
        let mut partial = policy(&empty);
        partial.entity_kinds.insert(EntityKind::Item, coverage);
        assert_eq!(PredicateEvidence::scoped(&empty, partial)?.entity_quantity_total(
            &EntityKind::Item, "type_key", &selector, Some("stack_size"),
        )?, None);
    }
    let base = snapshot(&[(1, "BED", 50), (2, "BED", 1)]);
    for field in ["type_key", "stack_size"] {
        let mut invalid = base.clone();
        let mut bad = fact(
            field,
            if field == "type_key" { selector.clone() } else { Value::U64(1) },
        );
        bad.source = FactSource::AgentAssertion("untrusted".into());
        change_fact(&mut invalid, 2, field, bad)?;
        assert_eq!(PredicateEvidence::scoped(&invalid, policy(&invalid))?.entity_quantity_total(
            &EntityKind::Item, "type_key", &selector, Some("stack_size"),
        )?, None);
    }
    let overflow = snapshot(&[(1, "BED", u64::MAX), (2, "BED", 1)]);
    assert_eq!(PredicateEvidence::scoped(&overflow, policy(&overflow))?.entity_quantity_total(
        &EntityKind::Item, "type_key", &selector, Some("stack_size"),
    )?, None);
    assert!(evidence.entity_quantity_total(&EntityKind::Item, "", &selector, None).is_err());
    Ok(())
}

#[test]
fn increase_never_accepts_a_different_fortress_epoch_or_earlier_frontier() -> Result<()> {
    let original = snapshot(&[(1, "BED", 5)]);
    let goal = increase(&original, 5, 1);
    let current = later(&[(1, "BED", 6)]);
    assert!(PredicateEvidence::scoped(&current, policy(&current))?.establishes(&goal)?);
    for which in 0..5 {
        let mut invalid = current.clone();
        match which {
            0 => invalid.fortress_id = FortressId::new(2),
            1 => invalid.cursor.epoch += 1,
            2 => invalid.cursor.sequence = original.cursor.sequence - 1,
            3 => invalid.tick = GameTick(original.tick.0 - 1),
            _ => { invalid.cursor = original.cursor; invalid.tick = original.tick; }
        }
        invalid.refresh_hash();
        unknown(&PredicateEvidence::scoped(&invalid, policy(&invalid))?, &goal)?;
        assert_eq!(
            evaluate_truth_for(&invalid, EntityId::new(1), &goal),
            PredicateTruth::Unknown,
        );
    }
    Ok(())
}

#[test]
fn increase_still_needs_current_complete_qualified_inputs_after_reaching_target() -> Result<()> {
    let original = snapshot(&[(1, "BED", 5)]);
    let goal = increase(&original, 5, 1);
    let mut current = later(&[(1, "BED", 50), (2, "BED", 1)]);
    let mut partial = policy(&current);
    partial.entity_kinds.insert(EntityKind::Item, EvidenceCoverage::Observed);
    unknown(&PredicateEvidence::scoped(&current, partial)?, &goal)?;
    let mut fact = fact("stack_size", Value::U64(1));
    fact.source_digest = Digest32::of_bytes(b"not independently accepted");
    change_fact(&mut current, 2, "stack_size", fact)?;
    unknown(&PredicateEvidence::scoped(&current, policy(&current))?, &goal)?;
    let overflow = later(&[(1, "BED", u64::MAX), (2, "BED", 1)]);
    unknown(&PredicateEvidence::scoped(&overflow, policy(&overflow))?, &goal)?;
    Ok(())
}

#[test]
fn increase_rejects_invalid_baselines_and_bounds_work_before_boolean_shortcuts() -> Result<()> {
    let original = snapshot(&[(1, "BED", 5)]);
    let evidence = PredicateEvidence::scoped(&original, policy(&original))?;
    for which in 0..6 {
        let mut goal = increase(&original, 5, 1);
        if let Predicate::EntityQuantityIncrease {
            baseline, increase, baseline_anchor, baseline_source_digest, source_generation, ..
        } = &mut goal {
            match which {
                0 => *increase = 0,
                1 => *baseline = u64::MAX,
                2 => baseline_anchor.fortress_id = FortressId::NIL,
                3 => baseline_anchor.state_hash = Digest32::ZERO,
                4 => *baseline_source_digest = Digest32::ZERO,
                _ => *source_generation = 0,
            }
        }
        assert!(matches!(
            evidence.evaluate(&Predicate::Any(vec![Predicate::True, goal])),
            Err(error) if error.code == ErrorCode::InvalidRequest
        ));
    }
    let rows = (1..=1200).map(|id| (id, "BED", 1)).collect::<Vec<_>>();
    let current = later(&rows);
    let excessive = Predicate::Any(vec![
        Predicate::True,
        Predicate::All(vec![increase(&original, 5, 1); 1200]),
    ]);
    assert!(matches!(
        PredicateEvidence::scoped(&current, policy(&current))?.evaluate(&excessive),
        Err(error) if error.code == ErrorCode::BudgetExceeded
    ));
    Ok(())
}

#[test]
fn increase_identity_and_query_continuations_seal_every_baseline_component() -> Result<()> {
    let original = snapshot(&[(1, "BED", 2)]);
    let current = later(&[(1, "BED", 3), (2, "BED", 1), (3, "DOOR", 1)]);
    let goal = increase(&original, 2, 1);
    assert_eq!(goal.canonical_bytes()[0], 15);
    let mut query = WorldQuery {
        kinds: vec![EntityKind::Item],
        predicate: Some(goal.clone()),
        order: QueryOrder::EntityIdAscending,
        limit: 1,
        continuation: None,
    };
    query.continuation = execute_query(&current, &query, 10)?.continuation;
    assert!(query.continuation.is_some());
    assert_eq!(execute_query(&current, &query, 10)?.entities[0].id, EntityId::new(2));
    for which in 0..16 {
        let mut changed = goal.clone();
        if let Predicate::EntityQuantityIncrease {
            kind, filter_field, filter_value, quantity_field, baseline, increase,
            baseline_anchor, baseline_source_digest, source_generation, baseline_source_counters,
        } = &mut changed {
            match which {
                0 => *kind = EntityKind::Building,
                1 => *filter_field = "other".into(),
                2 => *filter_value = Value::Text("DOOR".into()),
                3 => *quantity_field = None,
                4 => *baseline += 1,
                5 => *increase += 1,
                6 => baseline_anchor.fortress_id = FortressId::new(2),
                7 => baseline_anchor.cursor.epoch += 1,
                8 => baseline_anchor.cursor.sequence += 1,
                9 => baseline_anchor.tick.0 += 1,
                10 => baseline_anchor.state_hash = Digest32::of_bytes(b"different original"),
                11 => *baseline_source_digest = Digest32::of_bytes(b"different source"),
                12 => *source_generation += 1,
                13 => baseline_source_counters[0] += 1,
                14 => baseline_source_counters[1] += 1,
                _ => baseline_source_counters[2] += 1,
            }
        }
        assert_ne!(changed.canonical_bytes(), goal.canonical_bytes());
        let mut changed_query = query.clone();
        changed_query.predicate = Some(changed);
        assert!(matches!(
            execute_query(&current, &changed_query, 10),
            Err(error) if error.code == ErrorCode::StaleAnchor
        ));
    }
    assert_eq!(
        Predicate::All(vec![goal.clone(), Predicate::Paused(true), goal.clone()]).canonical_bytes(),
        Predicate::All(vec![Predicate::Paused(true), goal]).canonical_bytes(),
    );
    Ok(())
}
