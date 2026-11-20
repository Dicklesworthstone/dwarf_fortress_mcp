use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{
    DfmcpError, Digest32, EdgeId, EntityId, ErrorCode, FortressId, GameTick, ObservationCursor,
    Result,
};
use dfmcp_world::graph_query::{
    CapacitySchedule, CapacityScheduledDependency, DependencyScheduleQuery, GraphBudget,
    schedule_dependencies, schedule_dependencies_with_capacity,
    schedule_dependencies_with_capacity_and_check,
};
use dfmcp_world::{
    EdgeKind, EdgeRecord, EntityKind, EntityRecord, Fact, FactPresence, FactSource, Value,
    WorldGraph, WorldSnapshot,
};

fn scope() -> Digest32 {
    Digest32::of_bytes(b"authorized-capacity-fixture")
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

fn run(snapshot: &WorldSnapshot, capacity: u32) -> Result<CapacitySchedule> {
    schedule_dependencies_with_capacity(
        snapshot,
        scope(),
        &query(&[]),
        GraphBudget::default(),
        capacity,
    )
}

fn missing() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::InternalInvariantViolation,
        "missing capacity fixture",
    )
}

fn entry(result: &CapacitySchedule, id: u64) -> Result<&CapacityScheduledDependency> {
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
fn finite_slots_expose_capacity_queue_delay_without_changing_the_baseline() -> Result<()> {
    let snapshot = world(&[3, 2, 4], &[]);
    let serial = run(&snapshot, 1)?;
    assert_eq!(serial.makespan, 9);
    assert_eq!(serial.baseline.makespan, 4);
    assert_eq!(
        serial.dispatch_order,
        vec![EntityId::new(3), EntityId::new(1), EntityId::new(2)]
    );
    assert_eq!(entry(&serial, 3)?.start, 0);
    assert_eq!(entry(&serial, 1)?.queue_delay, 4);
    assert_eq!(entry(&serial, 2)?.queue_delay, 7);
    assert!(serial.entries.iter().all(|entry| entry.slot == Some(0)));
    let parallel = run(&snapshot, 2)?;
    assert_eq!(parallel.makespan, 5);
    assert_eq!(entry(&parallel, 1)?.slot, Some(1));
    assert_eq!(entry(&parallel, 2)?.start, 3);
    assert_eq!(entry(&parallel, 2)?.slot, Some(1));
    let unlimited = schedule_dependencies(&snapshot, scope(), &query(&[]), GraphBudget::default())?;
    assert_eq!(serial.baseline, unlimited);
    assert_eq!(parallel.baseline, unlimited);
    let ample = run(&snapshot, 64)?;
    assert_eq!(ample.makespan, unlimited.makespan);
    assert!(ample.entries.iter().all(|entry| entry.queue_delay == 0));
    Ok(())
}

#[test]
fn longest_downstream_path_precedes_larger_standalone_duration() -> Result<()> {
    let snapshot = world(&[1, 5, 10], &[(3, 1)]);
    let result = run(&snapshot, 1)?;
    assert_eq!(
        result.dispatch_order,
        vec![EntityId::new(1), EntityId::new(3), EntityId::new(2)]
    );
    assert_eq!(entry(&result, 1)?.downstream_duration, 11);
    assert_eq!(entry(&result, 3)?.ready_at, 1);
    assert_eq!(entry(&result, 3)?.queue_delay, 0);
    assert_eq!(entry(&result, 2)?.queue_delay, 11);
    assert_eq!(result.makespan, 16);
    Ok(())
}

#[test]
fn every_simultaneous_finish_is_processed_before_new_dispatch() -> Result<()> {
    let snapshot = world(&[2, 2, 1, 10], &[(3, 1), (4, 1), (4, 2)]);
    let result = run(&snapshot, 2)?;
    assert_eq!(entry(&result, 1)?.slot, Some(0));
    assert_eq!(entry(&result, 2)?.slot, Some(1));
    assert_eq!(entry(&result, 4)?.start, 2);
    assert_eq!(entry(&result, 4)?.slot, Some(0));
    assert_eq!(entry(&result, 3)?.start, 2);
    assert_eq!(entry(&result, 3)?.slot, Some(1));
    assert_eq!(
        result.dispatch_order,
        vec![
            EntityId::new(1),
            EntityId::new(2),
            EntityId::new(4),
            EntityId::new(3)
        ]
    );
    Ok(())
}

#[test]
fn zero_duration_tasks_need_no_slot_and_keep_same_instant_precedence() -> Result<()> {
    let busy = run(&world(&[5, 0], &[]), 1)?;
    assert_eq!(entry(&busy, 1)?.slot, Some(0));
    assert_eq!(entry(&busy, 2)?.slot, None);
    assert_eq!(entry(&busy, 2)?.start, 0);
    assert_eq!(entry(&busy, 2)?.finish, 0);
    let result = run(&world(&[0, 0, 0, 0], &[(1, 2), (2, 3)]), 1)?;
    assert_eq!(result.makespan, 0);
    assert_eq!(
        result.dispatch_order,
        vec![
            EntityId::new(3),
            EntityId::new(2),
            EntityId::new(1),
            EntityId::new(4)
        ]
    );
    assert!(
        result
            .entries
            .iter()
            .all(|entry| entry.slot.is_none() && entry.finish == 0)
    );
    let empty = run(&world(&[], &[]), 64)?;
    assert_eq!(empty.makespan, 0);
    assert!(empty.entries.is_empty());
    assert!(empty.dispatch_order.is_empty());
    Ok(())
}

#[test]
fn queue_delay_uses_actual_prerequisite_completion_not_unlimited_offsets() -> Result<()> {
    let result = run(&world(&[2, 2, 1], &[(3, 2)]), 1)?;
    assert_eq!(
        result.dispatch_order,
        vec![EntityId::new(2), EntityId::new(1), EntityId::new(3)]
    );
    assert_eq!(entry(&result, 3)?.ready_at, 2);
    assert_eq!(entry(&result, 3)?.start, 4);
    assert_eq!(entry(&result, 3)?.queue_delay, 2);
    Ok(())
}

#[test]
fn selected_closure_ignores_unrelated_cycles_and_parallel_edges_do_not_add_demand() -> Result<()> {
    let mut snapshot = world(&[2, 3, 0, 0], &[(1, 2), (1, 2), (3, 4), (4, 3)]);
    for id in [3, 4] {
        snapshot
            .graph
            .entities
            .get_mut(&EntityId::new(id))
            .ok_or_else(missing)?
            .fields
            .clear();
    }
    snapshot.refresh_hash();
    let result = schedule_dependencies_with_capacity(
        &snapshot,
        scope(),
        &query(&[1]),
        GraphBudget::default(),
        2,
    )?;
    assert_eq!(result.makespan, 5);
    assert_eq!(result.entries.len(), 2);
    assert_eq!(entry(&result, 1)?.ready_at, 3);
    let single = run(&world(&[2, 3], &[(1, 2)]), 2)?;
    assert_eq!(single.entries, result.entries);
    assert_eq!(single.dispatch_order, result.dispatch_order);
    assert_ne!(
        single.witness.projection_digest,
        result.witness.projection_digest
    );
    assert!(
        matches!(run(&snapshot, 2), Err(error) if error.code == ErrorCode::PreconditionsFailed)
    );
    assert!(
        matches!(run(&world(&[1, 1], &[(1, 2), (2, 1)]), 2), Err(error) if error.code == ErrorCode::Conflict)
    );
    Ok(())
}

#[test]
fn source_qualification_and_exact_capacity_bounds_fail_closed() -> Result<()> {
    let base = world(&[2], &[]);
    for capacity in [0, 65, u32::MAX] {
        assert!(
            matches!(run(&base, capacity), Err(error) if error.code == ErrorCode::InvalidRequest)
        );
    }
    for presence in [
        FactPresence::Unknown("unknown duration".to_owned()),
        FactPresence::Redacted("no permission".to_owned()),
        FactPresence::Known(Value::U64(99)),
    ] {
        let mut snapshot = base.clone();
        fact_mut(&mut snapshot, 1)?.presence = Some(presence);
        snapshot.refresh_hash();
        assert!(
            matches!(run(&snapshot, 2), Err(error) if error.code == ErrorCode::PreconditionsFailed)
        );
    }
    let mut future = base.clone();
    fact_mut(&mut future, 1)?.observed_at = GameTick(11);
    future.refresh_hash();
    assert!(matches!(run(&future, 2), Err(error) if error.code == ErrorCode::PreconditionsFailed));
    let mut forged = base.clone();
    fact_mut(&mut forged, 1)?.source = FactSource::AgentAssertion("agent".to_owned());
    forged.refresh_hash();
    assert!(matches!(run(&forged, 2), Err(error) if error.code == ErrorCode::PreconditionsFailed));
    let mut corrupt = base;
    corrupt.paused = false;
    assert!(
        matches!(run(&corrupt, 2), Err(error) if error.code == ErrorCode::InternalInvariantViolation)
    );
    Ok(())
}

#[test]
fn finite_capacity_overflow_is_not_hidden_by_a_valid_unlimited_baseline() -> Result<()> {
    let snapshot = world(&[u64::MAX, 1], &[]);
    assert_eq!(run(&snapshot, 2)?.makespan, u64::MAX);
    assert!(matches!(run(&snapshot, 1), Err(error) if error.code == ErrorCode::BudgetExceeded));
    let zero = run(&world(&[u64::MAX, 0], &[(2, 1)]), 1)?;
    assert_eq!(entry(&zero, 2)?.start, u64::MAX);
    assert_eq!(entry(&zero, 2)?.slot, None);
    Ok(())
}

#[test]
fn combined_work_budget_charges_the_baseline_and_every_capacity_pass() -> Result<()> {
    let snapshot = world(&[2, 3, 1, 5], &[(4, 1), (4, 2), (3, 2)]);
    let expected = run(&snapshot, 2)?;
    assert!(expected.witness.work_units > expected.baseline.witness.work_units);
    assert!(expected.witness.examined_arcs > expected.baseline.witness.examined_arcs);
    assert_eq!(expected.witness.scanned_edges, 6);
    let budget = GraphBudget {
        max_work: expected.witness.work_units,
        ..GraphBudget::default()
    };
    let exact = schedule_dependencies_with_capacity(&snapshot, scope(), &query(&[]), budget, 2)?;
    assert_eq!(exact.entries, expected.entries);
    assert_eq!(exact.witness.work_units, expected.witness.work_units);
    for maximum in [expected.baseline.witness.work_units, budget.max_work - 1] {
        let narrow = GraphBudget {
            max_work: maximum,
            ..budget
        };
        assert!(
            matches!(schedule_dependencies_with_capacity(&snapshot, scope(), &query(&[]), narrow, 2), Err(error) if error.code == ErrorCode::BudgetExceeded)
        );
    }
    for narrow in [
        GraphBudget {
            max_vertices: 3,
            ..GraphBudget::default()
        },
        GraphBudget {
            max_edges: 2,
            ..GraphBudget::default()
        },
        GraphBudget {
            max_frontier: 1,
            ..GraphBudget::default()
        },
    ] {
        assert!(
            matches!(schedule_dependencies_with_capacity(&snapshot, scope(), &query(&[]), narrow, 2), Err(error) if error.code == ErrorCode::BudgetExceeded)
        );
    }
    Ok(())
}

#[test]
fn cancellation_is_checked_during_the_capacity_pass_and_before_publication() -> Result<()> {
    let durations = vec![1; 200];
    let snapshot = world(&durations, &[]);
    let mut baseline_checks = 0;
    dfmcp_world::graph_query::schedule_dependencies_with_check(
        &snapshot,
        scope(),
        &query(&[]),
        GraphBudget::default(),
        || {
            baseline_checks += 1;
            Ok(())
        },
    )?;
    let mut all_checks = 0;
    schedule_dependencies_with_capacity_and_check(
        &snapshot,
        scope(),
        &query(&[]),
        GraphBudget::default(),
        2,
        || {
            all_checks += 1;
            Ok(())
        },
    )?;
    assert!(all_checks > baseline_checks + 2);
    for fail_on in [1, baseline_checks + 2, all_checks] {
        let mut checks = 0;
        let result = schedule_dependencies_with_capacity_and_check(
            &snapshot,
            scope(),
            &query(&[]),
            GraphBudget::default(),
            2,
            || {
                checks += 1;
                if checks == fail_on {
                    Err(DfmcpError::new(
                        ErrorCode::CapabilityDenied,
                        "caller cancelled capacity analysis",
                    ))
                } else {
                    Ok(())
                }
            },
        );
        assert!(matches!(result, Err(error) if error.code == ErrorCode::CapabilityDenied));
        assert_eq!(checks, fail_on);
    }
    Ok(())
}

#[test]
fn witness_covers_capacity_scope_source_budget_and_canonical_decisions() -> Result<()> {
    let snapshot = world(&[3, 2], &[(2, 1)]);
    let one = run(&snapshot, 1)?;
    let two = run(&snapshot, 2)?;
    assert_eq!(one.entries, two.entries);
    assert_ne!(one.witness.projection_digest, two.witness.projection_digest);
    assert_ne!(one.witness.decision_digest, two.witness.decision_digest);
    assert_eq!(one.witness.source_anchor, snapshot.anchor());
    assert_eq!(one.witness.authorization_scope_digest, scope());
    let rescope = schedule_dependencies_with_capacity(
        &snapshot,
        Digest32::of_bytes(b"narrow"),
        &query(&[]),
        GraphBudget::default(),
        1,
    )?;
    assert_eq!(one.entries, rescope.entries);
    assert_ne!(one.witness.decision_digest, rescope.witness.decision_digest);
    let mut provenance = snapshot.clone();
    fact_mut(&mut provenance, 1)?.observed_at = GameTick(9);
    provenance.refresh_hash();
    assert_eq!(one.entries, run(&provenance, 1)?.entries);
    assert_ne!(
        one.witness.decision_digest,
        run(&provenance, 1)?.witness.decision_digest
    );
    let same = schedule_dependencies_with_capacity(
        &snapshot,
        scope(),
        &DependencyScheduleQuery {
            edge_kinds: vec![EdgeKind::Requires, EdgeKind::Requires],
            ..query(&[])
        },
        GraphBudget::default(),
        1,
    )?;
    assert_eq!(one, same);
    let budget = GraphBudget {
        max_work: GraphBudget::default().max_work + 1,
        ..GraphBudget::default()
    };
    let larger = schedule_dependencies_with_capacity(&snapshot, scope(), &query(&[]), budget, 1)?;
    assert_eq!(one.entries, larger.entries);
    assert_ne!(one.witness.decision_digest, larger.witness.decision_digest);
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OracleEntry {
    start: u64,
    finish: u64,
    ready_at: u64,
    slot: Option<u32>,
    downstream: u64,
}

// Exhaustive path enumeration on tiny DAGs, independent of the production
// baseline/adjacency passes and priority queues.
fn downstream(node: usize, durations: &[u64], edges: &[(u64, u64)]) -> u64 {
    let mut longest = 0;
    for &(dependent, prerequisite) in edges {
        if prerequisite as usize == node + 1 {
            longest = longest.max(downstream(dependent as usize - 1, durations, edges));
        }
    }
    durations[node] + longest
}

// Tick-by-tick exhaustive ready-set enumeration, with direct prerequisite and
// occupancy scans. It shares no scheduling implementation or graph helper.
fn oracle(
    durations: &[u64],
    edges: &[(u64, u64)],
    capacity: u32,
) -> Result<(Vec<OracleEntry>, Vec<EntityId>, u64)> {
    let n = durations.len();
    let mut result: Vec<Option<OracleEntry>> = vec![None; n];
    let mut done = vec![false; n];
    let mut order = Vec::new();
    let bound: u64 = durations.iter().sum();
    for at in 0..=bound {
        for node in 0..n {
            if result[node]
                .as_ref()
                .is_some_and(|entry| entry.finish == at)
            {
                done[node] = true;
            }
        }
        loop {
            let occupied: BTreeSet<_> = result
                .iter()
                .flatten()
                .filter(|entry| entry.start <= at && at < entry.finish)
                .filter_map(|entry| entry.slot)
                .collect();
            let slot = (0..capacity).find(|slot| !occupied.contains(slot));
            let chosen = (0..n)
                .filter(|&node| {
                    result[node].is_none()
                        && (durations[node] == 0 || slot.is_some())
                        && edges
                            .iter()
                            .filter(|(from, _)| *from as usize == node + 1)
                            .all(|(_, to)| done[*to as usize - 1])
                })
                .min_by_key(|&node| (std::cmp::Reverse(downstream(node, durations, edges)), node));
            let Some(node) = chosen else {
                break;
            };
            let ready_at = edges
                .iter()
                .filter(|(from, _)| *from as usize == node + 1)
                .map(|(_, to)| {
                    result[*to as usize - 1]
                        .as_ref()
                        .map_or(0, |entry| entry.finish)
                })
                .max()
                .unwrap_or(0);
            result[node] = Some(OracleEntry {
                start: at,
                finish: at + durations[node],
                ready_at,
                slot: if durations[node] == 0 { None } else { slot },
                downstream: downstream(node, durations, edges),
            });
            done[node] = durations[node] == 0;
            order.push(EntityId::new(node as u64 + 1));
        }
        if done.iter().all(|value| *value) {
            return Ok((result.into_iter().flatten().collect(), order, at));
        }
    }
    Err(DfmcpError::new(
        ErrorCode::InternalInvariantViolation,
        "independent finite DAG oracle failed to complete",
    ))
}

#[test]
fn matches_independent_tick_oracle_for_6912_weighted_dags_and_capacities() -> Result<()> {
    let possible = [(2, 1), (3, 1), (3, 2), (4, 1), (4, 2), (4, 3)];
    for mask in 0_u32..64 {
        let edges: Vec<_> = possible
            .iter()
            .enumerate()
            .filter(|(index, _)| mask & (1 << index) != 0)
            .map(|(_, edge)| *edge)
            .collect();
        for code in 0_u32..27 {
            let durations = [
                u64::from(code % 3),
                u64::from((code / 3) % 3),
                u64::from(code / 9),
                2,
            ];
            let snapshot = world(&durations, &edges);
            for capacity in 1..=4 {
                let actual = run(&snapshot, capacity)?;
                let (expected, order, makespan) = oracle(&durations, &edges, capacity)?;
                assert_eq!(
                    actual.dispatch_order, order,
                    "mask={mask} code={code} capacity={capacity}"
                );
                assert_eq!(actual.makespan, makespan);
                assert_eq!(actual.entries.len(), expected.len());
                for (entry, expected) in actual.entries.iter().zip(expected) {
                    assert_eq!(
                        (
                            entry.start,
                            entry.finish,
                            entry.ready_at,
                            entry.slot,
                            entry.downstream_duration
                        ),
                        (
                            expected.start,
                            expected.finish,
                            expected.ready_at,
                            expected.slot,
                            expected.downstream
                        )
                    );
                    assert_eq!(entry.queue_delay, entry.start - entry.ready_at);
                }
                let total: u64 = durations.iter().sum();
                assert!(actual.makespan >= actual.baseline.makespan);
                assert!(actual.makespan >= total.div_ceil(u64::from(capacity)));
                if capacity == 1 {
                    assert_eq!(actual.makespan, total);
                }
                if capacity == 4 {
                    assert_eq!(actual.makespan, actual.baseline.makespan);
                }
            }
        }
    }
    Ok(())
}

#[test]
fn long_chain_and_wide_frontier_use_events_without_tick_or_depth_iteration() -> Result<()> {
    let durations = vec![1_000_000; 4_000];
    let edges: Vec<_> = (2_u64..=4_000).map(|id| (id, id - 1)).collect();
    let result = run(&world(&durations, &edges), 64)?;
    assert_eq!(result.makespan, 4_000_000_000);
    assert!(
        result
            .entries
            .iter()
            .all(|entry| entry.queue_delay == 0 && entry.slot == Some(0))
    );
    assert!(result.witness.work_units < 50 * 4_000);
    let wide = run(&world(&vec![7; 130], &[]), 64)?;
    assert_eq!(wide.makespan, 21);
    assert_eq!(entry(&wide, 65)?.start, 7);
    assert_eq!(entry(&wide, 65)?.slot, Some(0));
    assert_eq!(entry(&wide, 129)?.start, 14);
    Ok(())
}
