use super::*;

fn tasks(priorities: &[u32]) -> Vec<Task> {
    priorities.iter().enumerate().map(|(i, &priority)| Task { key: format!("t{i}"), priority }).collect()
}
fn demands(units: &[u64]) -> Vec<Demand> {
    units.iter().enumerate().map(|(i, &units)| Demand { key: format!("d{i}"), units }).collect()
}
fn supplies(masks: &[u32]) -> Vec<Supply> {
    masks.iter().enumerate().filter(|(_, mask)| **mask != 0)
        .map(|(i, &eligible)| Supply { id: i as u64 + 1, units: 1, eligible }).collect()
}
fn run(priorities: &[u32], units: &[u64], workers: &[u32], items: &[u32]) -> Result<Selection> {
    let task = tasks(priorities); let demand = demands(units);
    let owners: Vec<_> = (0..task.len()).collect();
    select(&task, Model { supplies: &supplies(workers), demands: &demand, owners: &owners },
        Model { supplies: &supplies(items), demands: &demand, owners: &owners }, flow::MAX_WORK, Duration::from_secs(60))
}

// Exhaustive one-unit assignments, independent of the production flow solver.
fn satisfiable(supply: &[u32], needed: &mut [u64], next: usize) -> bool {
    if needed.iter().all(|n| *n == 0) { return true; }
    if next == supply.len() { return false; }
    if satisfiable(supply, needed, next + 1) { return true; }
    for i in 0..needed.len() {
        if needed[i] != 0 && supply[next] & (1u32 << i) != 0 {
            needed[i] -= 1;
            let ok = satisfiable(supply, needed, next + 1);
            needed[i] += 1;
            if ok { return true; }
        }
    }
    false
}
fn oracle(priority: &[u32], units: &[u64], workers: &[u32], items: &[u32]) -> u16 {
    let mut best = (0u64, 0usize, Vec::<usize>::new(), 0u16);
    for mask in 1u16..(1u16 << priority.len()) {
        let indices: Vec<_> = (0..priority.len()).filter(|&i| mask & (1u16 << i) != 0).collect();
        let score: u64 = indices.iter().map(|&i| u64::from(priority[i])).sum();
        let better = score > best.0 || (score == best.0 && (indices.len() > best.1
            || (indices.len() == best.1 && indices < best.2)));
        if !better { continue; }
        let required: Vec<_> = (0..priority.len()).map(|i| if mask & (1u16 << i) != 0 { units[i] } else { 0 }).collect();
        if satisfiable(workers, &mut required.clone(), 0) && satisfiable(items, &mut required.clone(), 0) {
            best = (score, indices.len(), indices, mask);
        }
    }
    best.3
}
fn check_assignment(selection: &Selection, allocation: &Allocation, resources: &[Supply],
    demand: &[Demand], owners: &[usize]) {
    let mut used = std::collections::BTreeMap::<u64, u64>::new();
    for a in &allocation.assignments {
        assert!(selection.task_mask & (1u16 << owners[a.demand_index]) != 0);
        let supply = resources.iter().find(|s| s.id == a.supply_id).expect("assigned supply exists");
        assert!(supply.eligible & (1u32 << a.demand_index) != 0);
        *used.entry(a.supply_id).or_default() += a.units;
        assert!(used[&a.supply_id] <= supply.units);
    }
    for (i, d) in demand.iter().enumerate() {
        assert_eq!(allocation.allocated_by_demand[i], if selection.task_mask & (1u16 << owners[i]) != 0 { d.units } else { 0 });
    }
}

#[test]
fn crossed_partial_allocations_do_not_strand_the_supported_third_task() -> Result<()> {
    let selection = run(&[1, 1, 1], &[1, 1, 1], &[0b101], &[0b110])?;
    assert_eq!(selection.task_mask, 0b100);
    assert_eq!(selection.workers.allocated_by_demand, [0, 0, 1]);
    assert_eq!(selection.materials.allocated_by_demand, [0, 0, 1]);
    assert_eq!(selection.rejected.len(), 6);
    Ok(())
}

#[test]
fn priorities_and_complete_task_count_beat_greedy_slot_allocation() -> Result<()> {
    assert_eq!(run(&[3, 2, 2], &[2, 1, 1], &[7, 7], &[7, 7])?.task_mask, 0b110);
    assert_eq!(run(&[5, 2, 2], &[2, 1, 1], &[7, 7], &[7, 7])?.task_mask, 0b001);
    assert_eq!(run(&[2, 1, 1], &[2, 1, 1], &[7, 7], &[7, 7])?.task_mask, 0b110);
    assert_eq!(run(&[1, 1, 1], &[1, 1, 1], &[7], &[7])?.task_mask, 0b001);
    Ok(())
}

#[test]
fn exhaustive_three_task_two_worker_two_item_graphs_match_an_independent_oracle() -> Result<()> {
    for encoded in 0u32..4096 {
        let workers = [encoded & 7, (encoded >> 3) & 7];
        let items = [(encoded >> 6) & 7, (encoded >> 9) & 7];
        let priority = [1, 2, 2]; let units = [1, 1, 1];
        let selected = run(&priority, &units, &workers, &items)?;
        assert_eq!(selected.task_mask, oracle(&priority, &units, &workers, &items), "graph={encoded}");
        let demand = demands(&units); let owners = [0, 1, 2];
        check_assignment(&selected, &selected.workers, &supplies(&workers), &demand, &owners);
        check_assignment(&selected, &selected.materials, &supplies(&items), &demand, &owners);
        let mut seen = std::collections::BTreeSet::new();
        for rejection in &selected.rejected {
            assert!(seen.insert(rejection.task_mask));
            let model = if rejection.domain == Domain::Workers { &workers } else { &items };
            let cut = &rejection.shortage;
            let mask = cut.demand_indices.iter().fold(0u32, |bits, &i| bits | (1u32 << i));
            assert!(cut.demand_indices.iter().all(|&i| rejection.task_mask & (1u16 << i) != 0));
            assert_eq!(cut.required_units, cut.demand_indices.len() as u64);
            assert_eq!(cut.eligible_units, model.iter().filter(|&&bits| bits & mask != 0).count() as u64);
            assert_eq!(cut.deficit, cut.required_units - cut.eligible_units);
            assert!(cut.deficit > 0);
        }
    }
    Ok(())
}

#[test]
fn several_material_inputs_of_one_task_are_indivisible_as_a_set() -> Result<()> {
    let task = tasks(&[2, 1]); let wd = demands(&[1, 1]); let md = demands(&[1, 1, 1]);
    let ws = supplies(&[3, 3]); let ms = supplies(&[0b001, 0b100]);
    let selected = select(&task, Model { supplies: &ws, demands: &wd, owners: &[0, 1] },
        Model { supplies: &ms, demands: &md, owners: &[0, 0, 1] }, flow::MAX_WORK, Duration::from_secs(60))?;
    assert_eq!(selected.task_mask, 2);
    assert_eq!(selected.materials.allocated_by_demand, [0, 0, 1]);
    assert_eq!(selected.workers.allocated_by_demand, [0, 1]);
    assert!(selected.rejected.iter().any(|r| r.domain == Domain::Materials && r.shortage.demand_indices.contains(&1)));
    Ok(())
}

#[test]
fn eight_tasks_and_capacity_splitting_remain_deterministic() -> Result<()> {
    let task = tasks(&[1; 8]); let demand = demands(&[1; 8]); let owners: Vec<_> = (0..8).collect();
    let workers = supplies(&[255; 8]);
    let material = [Supply { id: 1, units: 5, eligible: 255 }];
    let calculate = || select(&task, Model { supplies: &workers, demands: &demand, owners: &owners },
        Model { supplies: &material, demands: &demand, owners: &owners }, flow::MAX_WORK, Duration::from_secs(60));
    let first = calculate()?; let second = calculate()?;
    assert_eq!(first, second); assert_eq!(first.task_mask, 0b0001_1111);
    assert_eq!(first.materials.allocated_units, 5);
    check_assignment(&first, &first.materials, &material, &demand, &owners);
    Ok(())
}

#[test]
fn empty_feasible_set_keeps_all_capacity_unassigned() -> Result<()> {
    let selected = run(&[1, 1], &[1, 1], &[1], &[2])?;
    assert_eq!(selected.task_mask, 0); assert_eq!(selected.priority, 0);
    assert!(selected.workers.assignments.is_empty()); assert!(selected.materials.assignments.is_empty());
    assert_eq!(selected.rejected.len(), 3); Ok(())
}

#[test]
fn malformed_ineligible_or_losing_input_cannot_hide_behind_subset_selection() {
    let task = tasks(&[1, 1]); let demand = demands(&[1, 1]); let ws = supplies(&[3]);
    let valid = Model { supplies: &ws, demands: &demand, owners: &[0, 1] };
    for bad in [vec![Supply { id: 1, units: 1, eligible: 4 }], vec![Supply { id: 0, units: 1, eligible: 3 }],
        vec![Supply { id: 1, units: 0, eligible: 3 }], vec![ws[0].clone(), ws[0].clone()]] {
        assert_eq!(select(&task, valid, Model { supplies: &bad, ..valid }, flow::MAX_WORK,
            Duration::from_secs(60)), Err(AllocationError::InvalidInput));
    }
    assert_eq!(select(&task, Model { owners: &[0, 0], ..valid }, valid, flow::MAX_WORK,
        Duration::from_secs(60)), Err(AllocationError::InvalidInput));
    let mut unordered = task.clone(); unordered.reverse();
    assert_eq!(select(&unordered, valid, valid, flow::MAX_WORK, Duration::from_secs(60)), Err(AllocationError::InvalidInput));
}

#[test]
fn work_time_and_arithmetic_exhaustion_never_return_a_partial_optimum() {
    let task = tasks(&[1, 1]); let demand = demands(&[1, 1]); let ws = supplies(&[3]);
    let model = Model { supplies: &ws, demands: &demand, owners: &[0, 1] };
    for work in [0, 1, 20, flow::MAX_WORK + 1] {
        assert_eq!(select(&task, model, model, work, Duration::from_secs(60)), Err(AllocationError::BudgetExceeded));
    }
    assert_eq!(select(&task, model, model, flow::MAX_WORK, Duration::ZERO), Err(AllocationError::BudgetExceeded));
    let huge = demands(&[u64::MAX, 1]);
    assert_eq!(select(&task, model, Model { demands: &huge, ..model }, flow::MAX_WORK,
        Duration::from_secs(60)), Err(AllocationError::ArithmeticOverflow));
}
