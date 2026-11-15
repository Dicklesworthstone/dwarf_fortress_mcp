use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{
    DfmcpError, Digest32, EdgeId, EntityId, ErrorCode, FortressId, GameTick, ObservationCursor,
    Result,
};
use dfmcp_world::graph_query::{
    DependencySchedule, DependencyScheduleQuery, GraphBudget, ScheduledDependency,
    analyze_dependencies, schedule_dependencies, schedule_dependencies_with_check,
};
use dfmcp_world::{
    EdgeKind, EdgeRecord, EntityKind, EntityRecord, Fact, FactPresence, FactSource, Value,
    WorldGraph, WorldSnapshot,
};

fn scope() -> Digest32 {
    Digest32::of_bytes(b"authorized-schedule-fixture")
}

fn query(targets: &[u64]) -> DependencyScheduleQuery {
    DependencyScheduleQuery {
        edge_kinds: vec![EdgeKind::Requires],
        duration_field: "duration".to_owned(),
        targets: targets.iter().copied().map(EntityId::new).collect(),
    }
}

fn world(durations: &[u64], edges: &[(u64, u64)]) -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    for (offset, &duration) in durations.iter().enumerate() {
        let id = EntityId::new(offset as u64 + 1);
        graph.entities.insert(
            id,
            EntityRecord {
                id,
                generation: 1,
                revision: 1,
                kind: EntityKind::Job,
                label: format!("job-{id}"),
                fields: BTreeMap::from([(
                    "duration".to_owned(),
                    Fact::known(
                        Value::U64(duration),
                        GameTick(10),
                        FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                        Digest32::ZERO,
                    ),
                )]),
            },
        );
    }
    for (offset, &(from, to)) in edges.iter().enumerate() {
        let id = EdgeId::new(offset as u128 + 1);
        graph.edges.insert(
            id,
            EdgeRecord {
                id,
                revision: 1,
                kind: EdgeKind::Requires,
                from: EntityId::new(from),
                to: EntityId::new(to),
                fields: BTreeMap::new(),
            },
        );
    }
    WorldSnapshot::new(
        FortressId::new(1),
        GameTick(10),
        ObservationCursor::ORIGIN,
        true,
        graph,
    )
}

fn run(snapshot: &WorldSnapshot, targets: &[u64]) -> Result<DependencySchedule> {
    schedule_dependencies(snapshot, scope(), &query(targets), GraphBudget::default())
}

fn ids(values: &[EntityId]) -> Vec<u64> {
    values.iter().map(|id| id.get()).collect()
}

fn missing() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::InternalInvariantViolation,
        "missing schedule fixture",
    )
}

fn entry(result: &DependencySchedule, id: u64) -> Result<&ScheduledDependency> {
    result
        .entries
        .iter()
        .find(|entry| entry.entity_id == EntityId::new(id))
        .ok_or_else(missing)
}

fn fact_mut(snapshot: &mut WorldSnapshot, id: u64) -> Result<&mut Fact> {
    snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(id))
        .and_then(|entity| entity.fields.get_mut("duration"))
        .ok_or_else(missing)
}

#[test]
fn duration_bottleneck_beats_hop_count_and_exposes_float() -> Result<()> {
    let snapshot = world(&[1, 1, 1, 9, 1, 2], &[(1, 2), (2, 3), (1, 4), (5, 1)]);
    let legacy = analyze_dependencies(
        &snapshot,
        scope(),
        &[EdgeKind::Requires],
        GraphBudget::default(),
    )?;
    assert_eq!(
        legacy.critical_chain.as_deref().map(ids),
        Some(vec![5, 1, 2, 3])
    );
    let result = run(&snapshot, &[])?;
    assert_eq!(result.makespan, 11);
    assert_eq!(ids(&result.critical_chain), vec![4, 1, 5]);
    assert_eq!(ids(&result.dependency_order), vec![3, 2, 4, 1, 5, 6]);
    assert_eq!(entry(&result, 2)?.earliest_start, 1);
    assert_eq!(entry(&result, 2)?.earliest_finish, 2);
    assert_eq!(entry(&result, 2)?.latest_start, 8);
    assert_eq!(entry(&result, 2)?.latest_finish, 9);
    assert_eq!(entry(&result, 2)?.total_slack, 7);
    assert_eq!(entry(&result, 6)?.total_slack, 9);
    assert_eq!(result.witness.source_anchor, snapshot.anchor());
    assert_eq!(result.witness.authorization_scope_digest, scope());
    assert!(
        result
            .entries
            .iter()
            .all(|entry| entry.generation == 1 && entry.revision == 1)
    );
    Ok(())
}

#[test]
fn equal_finish_ties_and_parallel_edges_have_canonical_decisions() -> Result<()> {
    let snapshot = world(&[2, 3, 3, 1], &[(1, 2), (1, 3), (2, 4), (3, 4)]);
    let result = run(&snapshot, &[])?;
    assert_eq!(result.makespan, 6);
    assert_eq!(ids(&result.critical_chain), vec![4, 2, 1]);
    assert_eq!(
        result
            .parallel_fronts
            .iter()
            .map(|front| (front.earliest_start, ids(&front.entities)))
            .collect::<Vec<_>>(),
        vec![(0, vec![4]), (1, vec![2, 3]), (4, vec![1])]
    );
    let parallel = run(
        &world(&[2, 3, 3, 1], &[(1, 2), (1, 3), (2, 4), (3, 4), (1, 2)]),
        &[],
    )?;
    assert_eq!(result.entries, parallel.entries);
    assert_eq!(result.critical_chain, parallel.critical_chain);
    assert_eq!(result.parallel_fronts, parallel.parallel_fronts);
    assert_ne!(
        result.witness.projection_digest,
        parallel.witness.projection_digest
    );
    Ok(())
}

#[test]
fn selected_goals_include_every_prerequisite_without_unrelated_durations() -> Result<()> {
    let mut snapshot = world(&[2, 3, 0, 0, 4], &[(1, 2), (3, 4), (4, 3), (5, 1)]);
    for id in [3, 4, 5] {
        snapshot
            .graph
            .entities
            .get_mut(&EntityId::new(id))
            .ok_or_else(missing)?
            .fields
            .clear();
    }
    snapshot.refresh_hash();
    let result = run(&snapshot, &[1])?;
    assert_eq!(result.targets, vec![EntityId::new(1)]);
    assert_eq!(
        result
            .entries
            .iter()
            .map(|entry| entry.entity_id.get())
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(result.makespan, 5);
    assert_eq!(ids(&result.critical_chain), vec![2, 1]);
    assert_eq!(result.witness.scanned_vertices, 5);
    assert_eq!(result.witness.scanned_edges, 4);
    assert_eq!(run(&snapshot, &[1, 2, 1])?, run(&snapshot, &[2, 1])?);
    assert!(
        matches!(run(&snapshot, &[5]), Err(error) if error.code == ErrorCode::PreconditionsFailed)
    );
    snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(2))
        .ok_or_else(missing)?
        .fields
        .clear();
    snapshot.refresh_hash();
    assert!(
        matches!(run(&snapshot, &[1]), Err(error) if error.code == ErrorCode::PreconditionsFailed)
    );
    Ok(())
}

#[test]
fn cycles_fail_only_when_in_the_selected_prerequisite_closure() -> Result<()> {
    let snapshot = world(&[1, 2, 3, 4], &[(1, 2), (2, 1), (3, 1)]);
    for targets in [&[][..], &[1][..], &[3][..]] {
        assert!(matches!(run(&snapshot, targets), Err(error) if error.code == ErrorCode::Conflict));
    }
    assert_eq!(run(&snapshot, &[4])?.makespan, 4);
    assert!(
        matches!(run(&world(&[0], &[(1, 1)]), &[]), Err(error) if error.code == ErrorCode::Conflict)
    );
    Ok(())
}

#[test]
fn zero_duration_dependencies_share_offsets_but_keep_precedence_order() -> Result<()> {
    let result = run(&world(&[0, 0, 0, 0], &[(1, 2), (2, 3)]), &[])?;
    assert_eq!(result.makespan, 0);
    assert_eq!(ids(&result.critical_chain), vec![3, 2, 1]);
    assert_eq!(ids(&result.dependency_order), vec![3, 2, 1, 4]);
    assert_eq!(result.parallel_fronts.len(), 1);
    assert_eq!(ids(&result.parallel_fronts[0].entities), vec![1, 2, 3, 4]);
    assert!(result.entries.iter().all(|entry| entry.total_slack == 0));
    let empty = run(&world(&[], &[]), &[])?;
    assert_eq!(empty.makespan, 0);
    assert!(empty.entries.is_empty());
    assert!(empty.critical_chain.is_empty());
    assert!(empty.parallel_fronts.is_empty());
    Ok(())
}

#[test]
fn accepts_current_registered_source_values_and_nonnegative_signed_durations() -> Result<()> {
    let mut snapshot = world(&[2, 0, 3], &[(1, 2), (1, 3)]);
    let signed = fact_mut(&mut snapshot, 1)?;
    signed.value = Value::I64(2);
    signed.presence = Some(FactPresence::Known(Value::I64(2)));
    fact_mut(&mut snapshot, 2)?.source =
        FactSource::Derived("dfmcp.reference-effects/1".to_owned());
    let native = fact_mut(&mut snapshot, 3)?;
    native.source = FactSource::DfhackField("test.expected_duration".to_owned());
    native.source_digest = Digest32::of_bytes(b"independently-admitted-test-source");
    native.observed_at = GameTick(9);
    snapshot.refresh_hash();
    assert_eq!(run(&snapshot, &[])?.makespan, 5);
    Ok(())
}

#[test]
fn refuses_unknown_inconsistent_asserted_replayed_and_unregistered_duration_evidence() -> Result<()>
{
    let base = world(&[2], &[]);
    for presence in [
        FactPresence::Absent,
        FactPresence::Unknown("not measured".to_owned()),
        FactPresence::Unsupported("old version".to_owned()),
        FactPresence::Omitted("projection".to_owned()),
        FactPresence::Redacted("scope".to_owned()),
        FactPresence::Stale(base.anchor()),
        FactPresence::Known(Value::U64(99)),
    ] {
        let mut snapshot = base.clone();
        fact_mut(&mut snapshot, 1)?.presence = Some(presence);
        snapshot.refresh_hash();
        assert!(
            matches!(run(&snapshot, &[]), Err(error) if error.code == ErrorCode::PreconditionsFailed)
        );
    }
    for source in [
        FactSource::AgentAssertion("agent".to_owned()),
        FactSource::Replay,
        FactSource::Derived("unregistered-formula".to_owned()),
        FactSource::DfhackField("duration".to_owned()),
    ] {
        let mut snapshot = base.clone();
        fact_mut(&mut snapshot, 1)?.source = source;
        snapshot.refresh_hash();
        assert!(
            matches!(run(&snapshot, &[]), Err(error) if error.code == ErrorCode::PreconditionsFailed)
        );
    }
    for name in [
        "".to_owned(),
        " ".to_owned(),
        "bad\nfield".to_owned(),
        "x".repeat(257),
    ] {
        let mut snapshot = base.clone();
        let fact = fact_mut(&mut snapshot, 1)?;
        fact.source = FactSource::DfhackField(name);
        fact.source_digest = Digest32::of_bytes(b"some-source");
        snapshot.refresh_hash();
        assert!(
            matches!(run(&snapshot, &[]), Err(error) if error.code == ErrorCode::PreconditionsFailed)
        );
    }
    for source in [
        FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
        FactSource::DfhackField("duration".to_owned()),
    ] {
        let mut snapshot = base.clone();
        let fact = fact_mut(&mut snapshot, 1)?;
        fact.source = source;
        fact.source_digest = Digest32::of_bytes(b"different-source");
        fact.observed_at = GameTick(11);
        snapshot.refresh_hash();
        assert!(
            matches!(run(&snapshot, &[]), Err(error) if error.code == ErrorCode::PreconditionsFailed)
        );
    }
    let mut changed_producer = base.clone();
    fact_mut(&mut changed_producer, 1)?.source_digest =
        Digest32::of_bytes(b"not-the-laboratory-producer");
    changed_producer.refresh_hash();
    assert!(
        matches!(run(&changed_producer, &[]), Err(error) if error.code == ErrorCode::PreconditionsFailed)
    );
    Ok(())
}

#[test]
fn refuses_negative_nonnumeric_and_overflow_without_saturating_schedule() -> Result<()> {
    for value in [
        Value::I64(-1),
        Value::Null,
        Value::Bool(false),
        Value::Text("3".to_owned()),
        Value::Fixed { units: 3, scale: 1 },
        Value::List(vec![Value::U64(3)]),
    ] {
        let mut snapshot = world(&[3], &[]);
        fact_mut(&mut snapshot, 1)?.value = value;
        snapshot.refresh_hash();
        assert!(
            matches!(run(&snapshot, &[]), Err(error) if error.code == ErrorCode::PreconditionsFailed)
        );
    }
    let largest = run(&world(&[u64::MAX, 0], &[(2, 1)]), &[])?;
    assert_eq!(largest.makespan, u64::MAX);
    assert_eq!(ids(&largest.critical_chain), vec![1, 2]);
    assert!(
        matches!(run(&world(&[u64::MAX, 1], &[(2, 1)]), &[]), Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn witness_binds_exact_source_durations_fields_targets_scope_and_budget() -> Result<()> {
    let base = world(&[3, 2], &[(1, 2)]);
    let result = run(&base, &[])?;
    let mut changed = base.clone();
    fact_mut(&mut changed, 2)?.value = Value::U64(4);
    changed.refresh_hash();
    let duration_change = run(&changed, &[])?;
    assert_ne!(
        duration_change.witness.projection_digest,
        result.witness.projection_digest
    );
    let mut provenance = base.clone();
    fact_mut(&mut provenance, 2)?.observed_at = GameTick(9);
    provenance.refresh_hash();
    let provenance_change = run(&provenance, &[])?;
    assert_eq!(provenance_change.entries, result.entries);
    assert_ne!(
        provenance_change.witness.decision_digest,
        result.witness.decision_digest
    );
    let mut alias = base.clone();
    for entity in alias.graph.entities.values_mut() {
        let fact = entity.fields.get("duration").ok_or_else(missing)?.clone();
        entity.fields.insert("other_duration".to_owned(), fact);
    }
    alias.refresh_hash();
    let old_field = run(&alias, &[])?;
    let new_field = schedule_dependencies(
        &alias,
        scope(),
        &DependencyScheduleQuery {
            duration_field: "other_duration".to_owned(),
            ..query(&[])
        },
        GraphBudget::default(),
    )?;
    assert_eq!(old_field.entries, new_field.entries);
    assert_ne!(
        old_field.witness.projection_digest,
        new_field.witness.projection_digest
    );
    let rescope = schedule_dependencies(
        &base,
        Digest32::of_bytes(b"narrow-scope"),
        &query(&[]),
        GraphBudget::default(),
    )?;
    assert_eq!(rescope.entries, result.entries);
    assert_ne!(
        rescope.witness.decision_digest,
        result.witness.decision_digest
    );
    let targeted = run(&base, &[1])?;
    assert_eq!(targeted.entries, result.entries);
    assert_ne!(
        targeted.witness.projection_digest,
        result.witness.projection_digest
    );
    let different_budget = schedule_dependencies(
        &base,
        scope(),
        &query(&[]),
        GraphBudget {
            max_work: GraphBudget::default().max_work + 1,
            ..GraphBudget::default()
        },
    )?;
    assert_eq!(different_budget.entries, result.entries);
    assert_ne!(
        different_budget.witness.decision_digest,
        result.witness.decision_digest
    );
    let mut later = base.clone();
    later.tick = GameTick(11);
    later.cursor.sequence += 1;
    later.refresh_hash();
    assert_ne!(
        run(&later, &[])?.witness.decision_digest,
        result.witness.decision_digest
    );
    Ok(())
}

#[test]
fn selector_normalization_and_builtin_custom_relation_identity_are_distinct() -> Result<()> {
    let base = world(&[3, 2], &[(1, 2)]);
    let original = run(&base, &[])?;
    let duplicate = schedule_dependencies(
        &base,
        scope(),
        &DependencyScheduleQuery {
            edge_kinds: vec![EdgeKind::Requires, EdgeKind::Requires],
            ..query(&[])
        },
        GraphBudget::default(),
    )?;
    assert_eq!(original, duplicate);
    let custom = schedule_dependencies(
        &base,
        scope(),
        &DependencyScheduleQuery {
            edge_kinds: vec![EdgeKind::Custom("requires".to_owned())],
            ..query(&[])
        },
        GraphBudget::default(),
    )?;
    assert_eq!(custom.makespan, 3);
    assert_ne!(
        custom.witness.projection_digest,
        original.witness.projection_digest
    );
    let reordered = WorldSnapshot::new(
        base.fortress_id,
        base.tick,
        base.cursor,
        base.paused,
        WorldGraph {
            entities: base
                .graph
                .entities
                .iter()
                .rev()
                .map(|(id, value)| (*id, value.clone()))
                .collect(),
            edges: base
                .graph
                .edges
                .iter()
                .rev()
                .map(|(id, value)| (*id, value.clone()))
                .collect(),
            ..WorldGraph::default()
        },
    );
    assert_eq!(original, run(&reordered, &[])?);
    Ok(())
}

#[test]
fn shape_and_invalid_canonical_source_fail_before_an_answer() -> Result<()> {
    let base = world(&[1, 1], &[(1, 2)]);
    for field in [
        "".to_owned(),
        " ".to_owned(),
        "duration\0".to_owned(),
        "x".repeat(129),
    ] {
        assert!(
            matches!(schedule_dependencies(&base, scope(), &DependencyScheduleQuery {
            duration_field: field, ..query(&[])
        }, GraphBudget::default()), Err(error) if error.code == ErrorCode::InvalidRequest)
        );
    }
    assert!(
        matches!(schedule_dependencies(&base, scope(), &DependencyScheduleQuery {
        edge_kinds: Vec::new(), ..query(&[])
    }, GraphBudget::default()), Err(error) if error.code == ErrorCode::InvalidRequest)
    );
    for target in [0, 99] {
        assert!(
            matches!(run(&base, &[target]), Err(error) if error.code == ErrorCode::InvalidRequest)
        );
    }
    assert!(matches!(run(&base, &[1; 65]), Err(error) if error.code == ErrorCode::BudgetExceeded));
    let mut invalid = base.clone();
    invalid.paused = false;
    assert!(
        matches!(run(&invalid, &[]), Err(error) if error.code == ErrorCode::InternalInvariantViolation)
    );
    let dangling = world(&[1, 1], &[(1, 99)]);
    assert!(
        matches!(run(&dangling, &[2]), Err(error) if error.code == ErrorCode::InternalInvariantViolation)
    );
    let mut alias = base.clone();
    let entity = alias
        .graph
        .entities
        .remove(&EntityId::new(2))
        .ok_or_else(missing)?;
    alias.graph.entities.insert(EntityId::new(99), entity);
    alias.refresh_hash();
    assert!(
        matches!(run(&alias, &[]), Err(error) if error.code == ErrorCode::InternalInvariantViolation)
    );
    Ok(())
}

#[test]
fn independent_work_input_and_frontier_limits_return_no_partial_schedule() -> Result<()> {
    let base = world(&[1, 2, 3, 4], &[(1, 2), (1, 3), (1, 4)]);
    for budget in [
        GraphBudget {
            max_vertices: 3,
            ..GraphBudget::default()
        },
        GraphBudget {
            max_edges: 2,
            ..GraphBudget::default()
        },
        GraphBudget {
            max_frontier: 2,
            ..GraphBudget::default()
        },
        GraphBudget {
            max_work: 1,
            ..GraphBudget::default()
        },
    ] {
        assert!(
            matches!(schedule_dependencies(&base, scope(), &query(&[1]), budget),
            Err(error) if error.code == ErrorCode::BudgetExceeded)
        );
    }
    // The requested closure is small, but cannot evade source admission bounds.
    assert!(
        matches!(schedule_dependencies(&base, scope(), &query(&[2]), GraphBudget {
        max_vertices: 1, ..GraphBudget::default()
    }), Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    let expected = run(&base, &[])?;
    let exact = GraphBudget {
        max_work: expected.witness.work_units,
        ..GraphBudget::default()
    };
    let allowed = schedule_dependencies(&base, scope(), &query(&[]), exact)?;
    assert_eq!(expected.entries, allowed.entries);
    assert_eq!(expected.witness.work_units, allowed.witness.work_units);
    assert!(
        matches!(schedule_dependencies(&base, scope(), &query(&[]), GraphBudget {
        max_work: exact.max_work - 1, ..exact
    }), Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    let zero_chain = world(&[0, 0], &[(1, 2)]);
    assert!(
        matches!(schedule_dependencies(&zero_chain, scope(), &query(&[]), GraphBudget {
        max_frontier: 1, ..GraphBudget::default()
    }), Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn cancellation_before_after_index_mid_analysis_and_at_publication_is_honored() -> Result<()> {
    let small = world(&[1, 1], &[(1, 2)]);
    let large = world(
        &vec![1; 1_000],
        &(2..=1_000).map(|id| (id, id - 1)).collect::<Vec<_>>(),
    );
    let original = large.canonical_bytes();
    for (snapshot, stop) in [
        (&small, 1),
        (&small, 2),
        (&small, 3),
        (&large, 3),
        (&large, 12),
    ] {
        let mut calls = 0;
        let outcome = schedule_dependencies_with_check(
            snapshot,
            scope(),
            &query(&[]),
            GraphBudget::default(),
            || {
                calls += 1;
                if calls == stop {
                    Err(DfmcpError::new(
                        ErrorCode::CancellationRequested,
                        "caller cancelled",
                    ))
                } else {
                    Ok(())
                }
            },
        );
        assert!(matches!(outcome, Err(error) if error.code == ErrorCode::CancellationRequested));
        assert_eq!(calls, stop);
    }
    assert_eq!(original, large.canonical_bytes());
    Ok(())
}

#[test]
fn long_chain_is_iterative_and_all_offsets_are_relative_to_model_start() -> Result<()> {
    let count = 4_000;
    let snapshot = world(
        &vec![2; count],
        &(2..=count as u64)
            .map(|id| (id, id - 1))
            .collect::<Vec<_>>(),
    );
    let result = run(&snapshot, &[count as u64])?;
    assert_eq!(result.makespan, count as u64 * 2);
    assert_eq!(result.critical_chain.len(), count);
    assert_eq!(result.entries.len(), count);
    assert_eq!(entry(&result, 1)?.earliest_start, 0);
    assert_eq!(
        entry(&result, count as u64)?.earliest_finish,
        count as u64 * 2
    );
    assert!(result.witness.work_units < 32 * count as u64);
    Ok(())
}

// Independent small-graph oracle: enumerate all directed simple paths. This is
// intentionally not a topological forward/backward dynamic program.
fn path_duration(
    node: usize,
    durations: &[u64],
    edges: &[(u64, u64)],
    selected: &[bool],
    reverse: bool,
) -> u64 {
    let next = edges
        .iter()
        .filter_map(|&(from, to)| {
            let (from, to) = if reverse { (to, from) } else { (from, to) };
            (from as usize == node + 1 && selected[to as usize - 1]).then_some(to as usize - 1)
        })
        .map(|next| path_duration(next, durations, edges, selected, reverse))
        .max()
        .map_or(0, |value| value);
    durations[node] + next
}

fn assert_path_oracle(
    result: &DependencySchedule,
    durations: &[u64],
    edges: &[(u64, u64)],
    selected: &[bool],
) -> Result<()> {
    let makespan = (0..durations.len())
        .filter(|&node| selected[node])
        .map(|node| path_duration(node, durations, edges, selected, false))
        .max()
        .map_or(0, |value| value);
    assert_eq!(result.makespan, makespan);
    assert_eq!(
        result.entries.len(),
        selected.iter().filter(|&&yes| yes).count()
    );
    for actual in &result.entries {
        let node = actual.entity_id.get() as usize - 1;
        let finish = path_duration(node, durations, edges, selected, false);
        let latest = makespan - path_duration(node, durations, edges, selected, true);
        assert_eq!(actual.earliest_finish, finish);
        assert_eq!(actual.earliest_start, finish - durations[node]);
        assert_eq!(actual.latest_start, latest);
        assert_eq!(actual.latest_finish, latest + durations[node]);
        assert_eq!(actual.total_slack, latest - (finish - durations[node]));
    }
    let ranks: BTreeMap<_, _> = result
        .dependency_order
        .iter()
        .enumerate()
        .map(|(rank, id)| (id.get(), rank))
        .collect();
    for &(dependent, prerequisite) in edges {
        if selected[dependent as usize - 1] {
            assert!(ranks[&prerequisite] < ranks[&dependent]);
        }
    }
    assert_eq!(
        result
            .critical_chain
            .iter()
            .map(|id| durations[id.get() as usize - 1])
            .sum::<u64>(),
        makespan
    );
    for pair in result.critical_chain.windows(2) {
        assert!(edges.contains(&(pair[1].get(), pair[0].get())));
    }
    for id in &result.critical_chain {
        assert_eq!(entry(result, id.get())?.total_slack, 0);
    }
    let mut seen = BTreeSet::new();
    for front in &result.parallel_fronts {
        assert!(front.entities.windows(2).all(|pair| pair[0] < pair[1]));
        for id in &front.entities {
            assert!(seen.insert(*id));
            assert_eq!(
                front.earliest_start,
                entry(result, id.get())?.earliest_start
            );
        }
    }
    assert_eq!(seen.len(), result.entries.len());
    Ok(())
}

#[test]
fn exhaustive_four_vertex_dags_and_duration_vectors_match_path_oracle() -> Result<()> {
    let possible = [(2, 1), (3, 1), (3, 2), (4, 1), (4, 2), (4, 3)];
    for mask in 0_u64..64 {
        let edges: Vec<_> = possible
            .iter()
            .copied()
            .enumerate()
            .filter_map(|(bit, edge)| (mask & (1 << bit) != 0).then_some(edge))
            .collect();
        for code in 0_u64..81 {
            let mut remaining = code;
            let mut durations = Vec::new();
            for _ in 0..4 {
                durations.push(remaining % 3);
                remaining /= 3;
            }
            let snapshot = world(&durations, &edges);
            let result = run(&snapshot, &[])?;
            assert_path_oracle(&result, &durations, &edges, &[true; 4])?;
        }
    }
    Ok(())
}

#[test]
#[allow(clippy::needless_range_loop)]
fn exhaustive_cyclic_graphs_and_target_sets_match_independent_reachability() -> Result<()> {
    let durations = [0, 2, 5];
    for mask in 0_u64..512 {
        let mut edges = Vec::new();
        let mut reach = [[false; 3]; 3];
        for from in 0..3 {
            reach[from][from] = true;
            for to in 0..3 {
                if mask & (1 << (from * 3 + to)) != 0 {
                    reach[from][to] = true;
                    edges.push((from as u64 + 1, to as u64 + 1));
                }
            }
        }
        for via in 0..3 {
            for from in 0..3 {
                for to in 0..3 {
                    reach[from][to] |= reach[from][via] && reach[via][to];
                }
            }
        }
        let snapshot = world(&durations, &edges);
        for targets_mask in 0_u64..8 {
            let targets: Vec<_> = (0..3)
                .filter(|node| targets_mask & (1 << node) != 0)
                .map(|node| node as u64 + 1)
                .collect();
            let selected: Vec<_> = (0..3)
                .map(|node| {
                    targets.is_empty()
                        || targets
                            .iter()
                            .any(|target| reach[*target as usize - 1][node])
                })
                .collect();
            let cycle = (0..3).any(|node| {
                selected[node]
                    && (mask & (1 << (node * 3 + node)) != 0
                        || (0..3)
                            .any(|other| node != other && reach[node][other] && reach[other][node]))
            });
            let result = run(&snapshot, &targets);
            if cycle {
                assert!(matches!(result, Err(error) if error.code == ErrorCode::Conflict));
            } else {
                assert_path_oracle(&result?, &durations, &edges, &selected)?;
            }
        }
    }
    Ok(())
}
