use super::*;

fn demand(key: &str, workers: u32, priority: u16) -> Demand {
    Demand {
        key: key.into(),
        workers,
        priority,
    }
}
fn candidate(
    worker_id: u64,
    demand_index: usize,
    effective: i32,
    nominal: i32,
    steps: u32,
) -> Candidate {
    Candidate {
        worker_id,
        demand_index,
        effective,
        nominal,
        steps,
    }
}
fn solve(demands: &[Demand], candidates: &[Candidate]) -> Allocation {
    let out = allocate(demands, candidates, MAX_WORK).expect("bounded test model");
    assert!(verify(demands, candidates, &out, MAX_WORK).is_ok());
    out
}

#[test]
fn reroutes_instead_of_greedily_spending_the_best_worker() {
    let d = [demand("a", 1, 0), demand("b", 1, 0)];
    let c = [
        candidate(1, 0, 20, 20, 1),
        candidate(2, 0, 19, 19, 1),
        candidate(1, 1, 20, 20, 1),
    ];
    let out = solve(&d, &c);
    assert_eq!(out.assigned_workers, 2);
    assert_eq!(
        out.assignments,
        [
            Assignment {
                demand_index: 0,
                worker_id: 2
            },
            Assignment {
                demand_index: 1,
                worker_id: 1
            }
        ]
    );
    assert_eq!(out.cost, Cost([0, -39, -39, 2]));
}

#[test]
fn cardinality_dominates_priority_and_priority_dominates_skill() {
    let d = [demand("a", 1, 0), demand("b", 1, 1000)];
    let c = [
        candidate(1, 0, i32::MAX, i32::MAX, 0),
        candidate(1, 1, 0, 0, u32::MAX),
    ];
    let out = solve(&d, &c);
    assert_eq!(out.assignments[0].demand_index, 1);
    assert_eq!(out.shortage.as_ref().expect("deficient").deficit, 1);
    let c = [
        c[0].clone(),
        candidate(2, 0, 0, i32::MIN, u32::MAX),
        c[1].clone(),
    ];
    let out = solve(&d, &c);
    assert_eq!(out.assigned_workers, 2);
    assert_eq!(out.cost, Cost([-1000, 0, 2_147_483_648, 8_589_934_590]));
}

#[test]
fn quality_objectives_are_exact_and_lexicographic() {
    let d = [demand("a", 1, 0)];
    let c = [
        candidate(1, 0, 2, 0, u32::MAX),
        candidate(2, 0, 1, i32::MAX, 0),
    ];
    assert_eq!(solve(&d, &c).assignments[0].worker_id, 1);
    let c = [candidate(1, 0, 2, 2, u32::MAX), candidate(2, 0, 2, 1, 0)];
    assert_eq!(solve(&d, &c).assignments[0].worker_id, 1);
    let c = [candidate(1, 0, 2, 2, 9), candidate(2, 0, 2, 2, 1)];
    assert_eq!(solve(&d, &c).assignments[0].worker_id, 2);
}

#[test]
fn multiple_slots_empty_rosters_and_hall_deficits() {
    let d = [demand("a", 2, 1), demand("b", 2, 2)];
    let empty = solve(&d, &[]);
    assert_eq!(
        empty.shortage,
        Some(Shortage {
            demand_indices: vec![0, 1],
            required_workers: 4,
            eligible_workers: 0,
            deficit: 4
        })
    );
    let c = [
        candidate(1, 0, 1, 1, 0),
        candidate(2, 0, 1, 1, 0),
        candidate(1, 1, 1, 1, 0),
        candidate(2, 1, 1, 1, 0),
    ];
    let out = solve(&d, &c);
    assert_eq!(out.allocated_by_demand, [0, 2]);
    assert_eq!(
        out.shortage,
        Some(Shortage {
            demand_indices: vec![0, 1],
            required_workers: 4,
            eligible_workers: 2,
            deficit: 2
        })
    );
}

#[test]
fn canonical_ties_and_budget_refusal_are_reproducible() {
    let d = [demand("a", 1, 0), demand("b", 1, 0)];
    let c = [
        candidate(1, 0, 1, 1, 0),
        candidate(2, 0, 1, 1, 0),
        candidate(1, 1, 1, 1, 0),
        candidate(2, 1, 1, 1, 0),
    ];
    let out = solve(&d, &c);
    assert_eq!(out, solve(&d, &c));
    assert_eq!(out.assignments[0].worker_id, 1);
    assert_eq!(allocate(&d, &c, out.work_units), Ok(out.clone()));
    assert_eq!(
        allocate(&d, &c, out.work_units - 1),
        Err(Error::BudgetExceeded)
    );
    assert_eq!(allocate(&d, &c, 0), Err(Error::BudgetExceeded));
    assert_eq!(allocate(&d, &c, MAX_WORK + 1), Err(Error::BudgetExceeded));
}

#[test]
fn invalid_models_are_rejected_before_solving() {
    let d = [demand("a", 1, 0)];
    for c in [
        candidate(0, 0, 1, 1, 0),
        candidate(1, 1, 1, 1, 0),
        candidate(1, 0, -1, 1, 0),
    ] {
        assert_eq!(allocate(&d, &[c], MAX_WORK), Err(Error::InvalidInput));
    }
    let c = candidate(1, 0, 1, 1, 0);
    assert_eq!(
        allocate(&d, &[c.clone(), c], MAX_WORK),
        Err(Error::InvalidInput)
    );
    for d in [demand("", 1, 0), demand("a", 0, 0), demand("a", 1, 1001)] {
        assert_eq!(allocate(&[d], &[], MAX_WORK), Err(Error::InvalidInput));
    }
    assert_eq!(
        allocate(&[demand("a", 128, 0), demand("b", 1, 0)], &[], MAX_WORK),
        Err(Error::BudgetExceeded)
    );
    assert_eq!(
        allocate(&[demand("b", 1, 0), demand("a", 1, 0)], &[], MAX_WORK),
        Err(Error::InvalidInput)
    );
}

#[test]
fn independent_verifier_rejects_tampered_assignments_costs_and_witnesses() {
    let d = [demand("a", 1, 0)];
    let c = [candidate(1, 0, 10, 10, 0), candidate(2, 0, 1, 1, 0)];
    let out = solve(&d, &c);
    let mut bad = out.clone();
    bad.assignments[0].worker_id = 2;
    bad.cost = Cost([0, -1, -1, 0]);
    assert!(verify(&d, &c, &bad, MAX_WORK).is_err());
    let mut bad = out.clone();
    bad.cost.0[1] += 1;
    assert!(verify(&d, &c, &bad, MAX_WORK).is_err());
    let mut bad = out.clone();
    bad.witness.potentials[2].0[0] = i64::MAX;
    assert!(verify(&d, &c, &bad, MAX_WORK).is_err());
    let mut bad = out.clone();
    bad.witness.source_side[0] = false;
    assert!(verify(&d, &c, &bad, MAX_WORK).is_err());
    let mut bad = out.clone();
    bad.allocated_by_demand[0] = 0;
    assert!(verify(&d, &c, &bad, MAX_WORK).is_err());
    let mut changed = c.clone();
    changed[1].effective = 100;
    assert!(verify(&d, &changed, &out, MAX_WORK).is_err());
}

#[test]
fn every_three_worker_graph_matches_an_exhaustive_quality_oracle() {
    for graph in 0u32..512 {
        for priorities in 0u32..8 {
            let d: Vec<_> = (0..3)
                .map(|i| demand(&format!("d{i}"), 1, ((priorities >> i) & 1) as u16))
                .collect();
            let mut c = Vec::new();
            for role in 0..3 {
                for worker in 0..3 {
                    if graph & (1 << (role * 3 + worker)) != 0 {
                        c.push(candidate(
                            worker + 1,
                            role as usize,
                            ((worker * 7 + role * 3) % 11) as i32,
                            ((worker * 5 + role) % 7) as i32,
                            ((worker + role * 3) % 5) as u32,
                        ));
                    }
                }
            }
            // Each worker independently chooses unassigned or one of three roles.
            let mut best = (0i32, Cost::default());
            for code in 0u32..64 {
                let mut value = code;
                let mut counts = [0u32; 3];
                let mut assigned = 0;
                let mut total = Cost::default();
                let mut valid = true;
                for worker in 1..=3 {
                    let choice = value % 4;
                    value /= 4;
                    if choice == 0 {
                        continue;
                    }
                    let role = (choice - 1) as usize;
                    counts[role] += 1;
                    let Some(row) = c
                        .iter()
                        .find(|r| r.worker_id == worker && r.demand_index == role)
                    else {
                        valid = false;
                        break;
                    };
                    if counts[role] > d[role].workers {
                        valid = false;
                        break;
                    }
                    assigned += 1;
                    total = total.add(cost(&d, row)).expect("small oracle");
                }
                if valid {
                    best = best.min((-assigned, total));
                }
            }
            let out = solve(&d, &c);
            assert_eq!(
                (-(out.assigned_workers as i32), out.cost),
                best,
                "graph={graph}, priorities={priorities}"
            );
        }
    }
}
