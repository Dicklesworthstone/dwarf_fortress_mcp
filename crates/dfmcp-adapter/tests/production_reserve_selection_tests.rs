//! Public solver tests; no DFHack, files, or fabricated native eligibility.
use dfmcp_adapter::workforce_analysis::portfolio::selection::{self, Model, RESERVE_OWNER, Task};
use dfmcp_world::inventory_allocation::{AllocationError, Demand, Supply};
use std::time::Duration;

fn demand(key: &str, units: u64) -> Demand {
    Demand {
        key: key.into(),
        units,
    }
}
fn supply(id: u64, units: u64, eligible: u32) -> Supply {
    Supply {
        id,
        units,
        eligible,
    }
}
fn task(key: &str, priority: u32) -> Task {
    Task {
        key: key.into(),
        priority,
    }
}
fn solve(
    tasks: &[Task],
    workers: &[Supply],
    materials: &[Supply],
    demands: &[Demand],
    owners: &[usize],
) -> Result<selection::Selection, AllocationError> {
    let worker_demands: Vec<_> = tasks.iter().map(|t| demand(&t.key, 1)).collect();
    let worker_owners: Vec<_> = (0..tasks.len()).collect();
    selection::select(
        tasks,
        Model {
            supplies: workers,
            demands: &worker_demands,
            owners: &worker_owners,
        },
        Model {
            supplies: materials,
            demands,
            owners,
        },
        10_000_000,
        Duration::from_secs(60),
    )
}

#[test]
fn reserves_win_over_even_the_highest_priority_production() {
    let out = solve(
        &[task("a", 1_000_000)],
        &[supply(1, 1, 1)],
        &[supply(10, 5, 3)],
        &[demand("a", 4), demand("reserve", 2)],
        &[0, RESERVE_OWNER],
    )
    .unwrap();
    assert_eq!(out.task_mask, 0);
    assert_eq!(out.materials.allocated_by_demand, [0, 2]);
    assert!(out.materials.shortage.is_none());
    assert!(out.workers.assignments.is_empty());
    assert_eq!(out.rejected.len(), 1);
    assert_eq!(out.rejected[0].shortage.deficit, 1);
    assert_eq!(out.rejected[0].shortage.demand_indices, [0, 1]);
}

#[test]
fn reserve_support_is_jointly_rerouted_not_greedily_subtracted() {
    let out = solve(
        &[task("a", 1)],
        &[supply(1, 1, 1)],
        &[supply(10, 1, 3), supply(11, 1, 2)],
        &[demand("a", 1), demand("reserve", 1)],
        &[0, RESERVE_OWNER],
    )
    .unwrap();
    assert_eq!(out.task_mask, 1);
    assert_eq!(out.materials.allocated_by_demand, [1, 1]);
    assert_eq!(
        out.materials
            .assignments
            .iter()
            .filter(|a| a.demand_index == 0)
            .map(|a| a.supply_id)
            .collect::<Vec<_>>(),
        [10]
    );
    assert_eq!(
        out.materials
            .assignments
            .iter()
            .filter(|a| a.demand_index == 1)
            .map(|a| a.supply_id)
            .collect::<Vec<_>>(),
        [11]
    );
}

#[test]
fn infeasible_overlapping_reserves_are_not_a_feasible_empty_portfolio() {
    let out = solve(
        &[task("a", 9)],
        &[supply(1, 1, 1)],
        &[supply(10, 1, 1), supply(11, 1, 6)],
        &[demand("a", 1), demand("r1", 1), demand("r2", 1)],
        &[0, RESERVE_OWNER, RESERVE_OWNER],
    )
    .unwrap();
    assert_eq!(out.task_mask, 0);
    assert_eq!(out.priority, 0);
    assert!(out.workers.assignments.is_empty());
    let cut = out.materials.shortage.as_ref().unwrap();
    assert_eq!(cut.demand_indices, [1, 2]);
    assert_eq!(
        (cut.required_units, cut.eligible_units, cut.deficit),
        (2, 1, 1)
    );
    assert_eq!(out.materials.allocated_by_demand[0], 0);
    assert!(out.rejected.is_empty());
}

#[test]
fn reserves_remain_supported_when_no_worker_is_available() {
    let out = solve(
        &[task("a", 1)],
        &[],
        &[supply(10, 7, 3)],
        &[demand("a", 1), demand("reserve", 3)],
        &[0, RESERVE_OWNER],
    )
    .unwrap();
    assert_eq!(out.task_mask, 0);
    assert_eq!(out.materials.requested_units, 3);
    assert_eq!(out.materials.allocated_units, 3);
    assert_eq!(out.materials.allocated_by_demand, [0, 3]);
    assert!(out.materials.shortage.is_none());
}

#[test]
fn joint_task_priority_is_optimized_after_protecting_the_floor() {
    let tasks = [task("a", 3), task("b", 2), task("c", 2)];
    let out = solve(
        &tasks,
        &[supply(1, 1, 7), supply(2, 1, 7)],
        &[supply(10, 3, 15)],
        &[
            demand("a", 2),
            demand("b", 1),
            demand("c", 1),
            demand("reserve", 1),
        ],
        &[0, 1, 2, RESERVE_OWNER],
    )
    .unwrap();
    assert_eq!(out.task_mask, 6);
    assert_eq!(out.priority, 4);
    assert_eq!(out.materials.allocated_by_demand, [0, 1, 1, 1]);
    assert_eq!(
        out.materials
            .assignments
            .iter()
            .map(|a| a.units)
            .sum::<u64>(),
        3
    );
}

#[test]
fn invalid_models_cannot_hide_behind_reserve_shortfall() {
    let tasks = [task("a", 1)];
    let demands = [demand("a", 1), demand("reserve", 1)];
    assert!(matches!(
        solve(
            &tasks,
            &[supply(1, 1, 1)],
            &[supply(10, 1, 8)],
            &demands,
            &[0, RESERVE_OWNER]
        ),
        Err(AllocationError::InvalidInput)
    ));
    let worker_demands = [demand("a", 1)];
    assert!(matches!(
        selection::select(
            &tasks,
            Model {
                supplies: &[],
                demands: &worker_demands,
                owners: &[RESERVE_OWNER]
            },
            Model {
                supplies: &[],
                demands: &demands,
                owners: &[0, RESERVE_OWNER]
            },
            10_000_000,
            Duration::from_secs(60)
        ),
        Err(AllocationError::InvalidInput)
    ));
    assert!(matches!(
        solve(&tasks, &[], &[], &demands, &[RESERVE_OWNER, RESERVE_OWNER]),
        Err(AllocationError::InvalidInput)
    ));
    assert!(matches!(
        solve(
            &tasks,
            &[supply(1, 1, 1)],
            &[],
            &[demand("a", u64::MAX), demand("reserve", 1)],
            &[0, RESERVE_OWNER]
        ),
        Err(AllocationError::ArithmeticOverflow)
    ));
}

// Independent exhaustive unit assignments: no flow or subset implementation is reused.
fn can_fill(masks: &[u32], needed: u32, index: usize) -> bool {
    if needed == 0 {
        return true;
    }
    if index == masks.len() {
        return false;
    }
    if can_fill(masks, needed, index + 1) {
        return true;
    }
    (0..3).any(|bit| {
        needed & (1 << bit) != 0
            && masks[index] & (1 << bit) != 0
            && can_fill(masks, needed & !(1 << bit), index + 1)
    })
}

#[test]
fn all_small_worker_material_graphs_match_independent_reserve_assignment_oracle() {
    let tasks = [task("a", 2), task("b", 1)];
    for worker_code in 0u32..16 {
        let wm = [worker_code & 3, (worker_code >> 2) & 3];
        let workers: Vec<_> = wm
            .iter()
            .enumerate()
            .filter(|(_, m)| **m != 0)
            .map(|(i, m)| supply(i as u64 + 1, 1, *m))
            .collect();
        for material_code in 0u32..512 {
            let mm = [
                material_code & 7,
                (material_code >> 3) & 7,
                (material_code >> 6) & 7,
            ];
            let materials: Vec<_> = mm
                .iter()
                .enumerate()
                .filter(|(_, m)| **m != 0)
                .map(|(i, m)| supply(i as u64 + 10, 1, *m))
                .collect();
            let out = solve(
                &tasks,
                &workers,
                &materials,
                &[demand("a", 1), demand("b", 1), demand("reserve", 1)],
                &[0, 1, RESERVE_OWNER],
            )
            .unwrap();
            let floor_possible = can_fill(&mm, 4, 0);
            assert_eq!(out.materials.shortage.is_none(), floor_possible);
            let expected = [3u32, 1, 2, 0]
                .into_iter()
                .find(|mask| can_fill(&wm, *mask, 0) && can_fill(&mm, *mask | 4, 0));
            assert_eq!(out.task_mask, u16::try_from(expected.unwrap_or(0)).unwrap());
            if expected.is_some() {
                assert_eq!(out.materials.allocated_by_demand[2], 1);
                for supply in &materials {
                    assert!(
                        out.materials
                            .assignments
                            .iter()
                            .filter(|a| a.supply_id == supply.id)
                            .map(|a| a.units)
                            .sum::<u64>()
                            <= supply.units
                    );
                }
            }
        }
    }
}
