#![forbid(unsafe_code)]

//! Deterministic capacitated matching for explicitly declared stack-unit demands.
//! This solver knows nothing about DF job eligibility, access, or mutation. Its
//! certificate proves only the supplied bipartite allocation model. It can be
//! tested independently with `rustc --edition=2024 --test inventory_allocation.rs`.

use std::collections::{BTreeMap, VecDeque};

pub const MAX_SUPPLIES: usize = 32_768;
pub const MAX_DEMANDS: usize = 32;
pub const MAX_WORK: u64 = 10_000_000;
const MAX_GROUPS: usize = 1_024;
const MAX_ARCS: usize = 65_536;
const MAX_ASSIGNMENTS: usize = 65_536;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Supply {
    pub id: u64,
    pub units: u64,
    /// Bit i admits this supply to demand i. Input demand order is canonical.
    pub eligible: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Demand {
    pub key: String,
    pub units: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assignment {
    pub supply_id: u64,
    pub demand_index: usize,
    pub units: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortage {
    /// A Hall-type deficient subset, not independently additive shortages.
    pub demand_indices: Vec<usize>,
    pub required_units: u64,
    pub eligible_units: u64,
    pub deficit: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Allocation {
    /// Ordered by demand index, then stable supply ID.
    pub assignments: Vec<Assignment>,
    pub allocated_by_demand: Vec<u64>,
    pub requested_units: u64,
    pub allocated_units: u64,
    /// Equals allocated_units: primal flow and dual cut agree.
    pub cut_capacity: u64,
    pub shortage: Option<Shortage>,
    pub work_units: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AllocationError {
    InvalidInput,
    BudgetExceeded,
    ArithmeticOverflow,
    InvariantViolation,
}

type Result<T> = std::result::Result<T, AllocationError>;

struct Work {
    used: u64,
    maximum: u64,
}
impl Work {
    fn charge(&mut self, units: u64) -> Result<()> {
        self.used = self
            .used
            .checked_add(units)
            .ok_or(AllocationError::ArithmeticOverflow)?;
        if self.used > self.maximum {
            return Err(AllocationError::BudgetExceeded);
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Arc {
    to: usize,
    reverse: usize,
    remaining: u64,
    original: u64,
}

fn add_arc(graph: &mut [Vec<Arc>], from: usize, to: usize, capacity: u64) -> usize {
    let index = graph[from].len();
    let reverse = graph[to].len();
    graph[from].push(Arc {
        to,
        reverse,
        remaining: capacity,
        original: capacity,
    });
    graph[to].push(Arc {
        to: from,
        reverse: index,
        remaining: 0,
        original: 0,
    });
    index
}

fn sum(left: u64, right: u64) -> Result<u64> {
    left.checked_add(right)
        .ok_or(AllocationError::ArithmeticOverflow)
}

/// Maximum total allocated units, integral and deterministic, with residual
/// rerouting rather than greedy demand-by-demand consumption. Equal-eligibility
/// supplies are grouped before flow, then expanded without double-counting.
/// All IDs and keys must already be strictly increasing. No partial result is
/// returned on a work, shape, arithmetic, or certificate failure.
pub fn allocate(supplies: &[Supply], demands: &[Demand], max_work: u64) -> Result<Allocation> {
    if supplies.len() > MAX_SUPPLIES
        || demands.is_empty()
        || demands.len() > MAX_DEMANDS
        || max_work == 0
        || max_work > MAX_WORK
    {
        return Err(AllocationError::BudgetExceeded);
    }
    let mut work = Work {
        used: 0,
        maximum: max_work,
    };
    let mask_limit = if demands.len() == 32 {
        u32::MAX
    } else {
        (1u32 << demands.len()) - 1
    };
    let mut requested = 0u64;
    for (index, demand) in demands.iter().enumerate() {
        work.charge(1)?;
        if demand.key.is_empty()
            || demand.key.len() > 64
            || demand.units == 0
            || !demand
                .key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            || (index > 0 && demands[index - 1].key >= demand.key)
        {
            return Err(AllocationError::InvalidInput);
        }
        requested = sum(requested, demand.units)?;
    }
    let mut grouped: BTreeMap<u32, (u64, Vec<&Supply>)> = BTreeMap::new();
    for (index, supply) in supplies.iter().enumerate() {
        work.charge(1)?;
        if supply.id == 0
            || supply.units == 0
            || supply.eligible == 0
            || supply.eligible & !mask_limit != 0
            || (index > 0 && supplies[index - 1].id >= supply.id)
        {
            return Err(AllocationError::InvalidInput);
        }
        if !grouped.contains_key(&supply.eligible) && grouped.len() >= MAX_GROUPS {
            return Err(AllocationError::BudgetExceeded);
        }
        let entry = grouped.entry(supply.eligible).or_default();
        entry.0 = sum(entry.0, supply.units)?;
        entry.1.push(supply);
    }
    let groups: Vec<_> = grouped.into_iter().collect();
    let source = 0;
    let demand_start = 1 + groups.len();
    let sink = demand_start + demands.len();
    let mut graph = vec![Vec::new(); sink + 1];
    let mut links = vec![Vec::new(); groups.len()];
    let mut arcs = 0usize;
    for (i, (mask, (units, _))) in groups.iter().enumerate() {
        work.charge(1)?;
        add_arc(&mut graph, source, i + 1, *units);
        arcs += 2;
        for d in 0..demands.len() {
            work.charge(1)?;
            if mask & (1u32 << d) != 0 {
                if arcs + 2 > MAX_ARCS {
                    return Err(AllocationError::BudgetExceeded);
                }
                // The total request is a sound finite infinity for this model.
                let edge = add_arc(&mut graph, i + 1, demand_start + d, requested);
                links[i].push((d, edge));
                arcs += 2;
            }
        }
    }
    for (d, demand) in demands.iter().enumerate() {
        if arcs + 2 > MAX_ARCS {
            return Err(AllocationError::BudgetExceeded);
        }
        add_arc(&mut graph, demand_start + d, sink, demand.units);
        arcs += 2;
    }
    let mut allocated = 0u64;
    let reached = loop {
        let mut parents = vec![None; graph.len()];
        parents[source] = Some((source, 0));
        let mut queue = VecDeque::from([source]);
        while let Some(node) = queue.pop_front() {
            work.charge(1)?;
            for (edge, arc) in graph[node].iter().enumerate() {
                work.charge(1)?;
                if arc.remaining > 0 && parents[arc.to].is_none() {
                    parents[arc.to] = Some((node, edge));
                    queue.push_back(arc.to);
                }
            }
            if parents[sink].is_some() {
                break;
            }
        }
        if parents[sink].is_none() {
            break parents.iter().map(Option::is_some).collect::<Vec<_>>();
        }
        let mut node = sink;
        let mut amount = u64::MAX;
        while node != source {
            work.charge(1)?;
            let (from, edge) = parents[node].ok_or(AllocationError::InvariantViolation)?;
            amount = amount.min(graph[from][edge].remaining);
            node = from;
        }
        if amount == 0 {
            return Err(AllocationError::InvariantViolation);
        }
        node = sink;
        while node != source {
            work.charge(1)?;
            let (from, edge) = parents[node].ok_or(AllocationError::InvariantViolation)?;
            let arc = graph[from][edge];
            graph[from][edge].remaining -= amount;
            graph[node][arc.reverse].remaining = sum(graph[node][arc.reverse].remaining, amount)?;
            node = from;
        }
        allocated = sum(allocated, amount)?;
        if allocated > requested {
            return Err(AllocationError::InvariantViolation);
        }
    };
    let mut cut_capacity = 0u64;
    for (node, neighbors) in graph.iter().enumerate() {
        for arc in neighbors {
            work.charge(1)?;
            if reached[node] && !reached[arc.to] {
                cut_capacity = sum(cut_capacity, arc.original)?;
            }
        }
    }
    if cut_capacity != allocated {
        return Err(AllocationError::InvariantViolation);
    }
    let shortage = if allocated < requested {
        let demand_indices: Vec<_> = (0..demands.len())
            .filter(|d| !reached[demand_start + d])
            .collect();
        let mut required_units = 0;
        for &d in &demand_indices {
            required_units = sum(required_units, demands[d].units)?;
        }
        let subset = demand_indices
            .iter()
            .fold(0u32, |mask, d| mask | (1u32 << d));
        let mut eligible_units = 0;
        for (mask, (units, _)) in &groups {
            work.charge(1)?;
            if mask & subset != 0 {
                eligible_units = sum(eligible_units, *units)?;
            }
        }
        let deficit = required_units
            .checked_sub(eligible_units)
            .ok_or(AllocationError::InvariantViolation)?;
        if deficit != requested - allocated || deficit == 0 {
            return Err(AllocationError::InvariantViolation);
        }
        Some(Shortage {
            demand_indices,
            required_units,
            eligible_units,
            deficit,
        })
    } else {
        None
    };
    let mut assignments = Vec::new();
    let mut allocated_by_demand = vec![0u64; demands.len()];
    for (g, (_, (_, members))) in groups.iter().enumerate() {
        let mut member = 0;
        let mut used = 0u64;
        for &(demand_index, edge) in &links[g] {
            let arc = graph[g + 1][edge];
            let mut remaining = arc.original - arc.remaining;
            while remaining > 0 {
                work.charge(1)?;
                let supply = members
                    .get(member)
                    .ok_or(AllocationError::InvariantViolation)?;
                let amount = remaining.min(supply.units - used);
                if amount == 0 || assignments.len() >= MAX_ASSIGNMENTS {
                    return Err(AllocationError::BudgetExceeded);
                }
                assignments.push(Assignment {
                    supply_id: supply.id,
                    demand_index,
                    units: amount,
                });
                allocated_by_demand[demand_index] = sum(allocated_by_demand[demand_index], amount)?;
                used += amount;
                remaining -= amount;
                if used == supply.units {
                    member += 1;
                    used = 0;
                }
            }
        }
    }
    assignments.sort_by_key(|a| (a.demand_index, a.supply_id));
    let mut verified_total = 0;
    for (d, units) in allocated_by_demand.iter().enumerate() {
        if *units > demands[d].units {
            return Err(AllocationError::InvariantViolation);
        }
        verified_total = sum(verified_total, *units)?;
    }
    if verified_total != allocated {
        return Err(AllocationError::InvariantViolation);
    }
    Ok(Allocation {
        assignments,
        allocated_by_demand,
        requested_units: requested,
        allocated_units: allocated,
        cut_capacity,
        shortage,
        work_units: work.used,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demands(units: &[u64]) -> Vec<Demand> {
        units
            .iter()
            .enumerate()
            .map(|(i, &units)| Demand {
                key: format!("d{i:02}"),
                units,
            })
            .collect()
    }
    fn verify(supplies: &[Supply], demands: &[Demand], result: &Allocation) {
        let mut used = BTreeMap::<u64, u64>::new();
        let mut fulfilled = vec![0; demands.len()];
        for assignment in &result.assignments {
            let supply = supplies.iter().find(|s| s.id == assignment.supply_id);
            assert!(supply.is_some_and(|s| s.eligible & (1 << assignment.demand_index) != 0));
            assert!(assignment.units > 0);
            *used.entry(assignment.supply_id).or_default() += assignment.units;
            fulfilled[assignment.demand_index] += assignment.units;
        }
        for supply in supplies {
            assert!(used.get(&supply.id).copied().unwrap_or(0) <= supply.units);
        }
        assert_eq!(fulfilled, result.allocated_by_demand);
        assert_eq!(fulfilled.iter().sum::<u64>(), result.allocated_units);
        assert_eq!(result.cut_capacity, result.allocated_units);
        for (d, units) in demands.iter().zip(&fulfilled) {
            assert!(*units <= d.units);
        }
        if let Some(cut) = &result.shortage {
            let mask = cut
                .demand_indices
                .iter()
                .fold(0, |mask, &d| mask | (1u32 << d));
            assert_eq!(
                cut.required_units,
                cut.demand_indices
                    .iter()
                    .map(|&d| demands[d].units)
                    .sum::<u64>()
            );
            assert_eq!(
                cut.eligible_units,
                supplies
                    .iter()
                    .filter(|s| s.eligible & mask != 0)
                    .map(|s| s.units)
                    .sum::<u64>()
            );
            assert_eq!(cut.required_units - cut.eligible_units, cut.deficit);
            assert_eq!(cut.deficit, result.requested_units - result.allocated_units);
        } else {
            assert_eq!(result.requested_units, result.allocated_units);
        }
    }
    #[test]
    fn residual_rerouting_finds_the_assignment_greedy_consumption_misses() -> Result<()> {
        let supplies = vec![
            Supply {
                id: 1,
                units: 1,
                eligible: 3,
            },
            Supply {
                id: 2,
                units: 1,
                eligible: 4,
            },
            Supply {
                id: 3,
                units: 1,
                eligible: 5,
            },
        ];
        let needs = demands(&[1, 1, 1]);
        let result = allocate(&supplies, &needs, MAX_WORK)?;
        assert_eq!(result.allocated_units, 3);
        verify(&supplies, &needs, &result);
        Ok(())
    }
    #[test]
    fn grouped_stacks_split_without_double_counting() -> Result<()> {
        let supplies = vec![
            Supply {
                id: 4,
                units: 3,
                eligible: 3,
            },
            Supply {
                id: 9,
                units: 5,
                eligible: 3,
            },
        ];
        let needs = demands(&[5, 3]);
        let result = allocate(&supplies, &needs, MAX_WORK)?;
        assert_eq!(
            result.assignments,
            vec![
                Assignment {
                    supply_id: 4,
                    demand_index: 0,
                    units: 3
                },
                Assignment {
                    supply_id: 9,
                    demand_index: 0,
                    units: 2
                },
                Assignment {
                    supply_id: 9,
                    demand_index: 1,
                    units: 3
                }
            ]
        );
        verify(&supplies, &needs, &result);
        assert_eq!(result, allocate(&supplies, &needs, MAX_WORK)?);
        Ok(())
    }
    #[test]
    fn joint_shortage_is_not_an_independent_per_demand_availability_claim() -> Result<()> {
        let supplies = vec![Supply {
            id: 1,
            units: 5,
            eligible: 3,
        }];
        let needs = demands(&[4, 4]);
        let result = allocate(&supplies, &needs, MAX_WORK)?;
        assert_eq!(result.allocated_units, 5);
        assert_eq!(result.shortage.as_ref().map(|s| s.deficit), Some(3));
        verify(&supplies, &needs, &result);
        Ok(())
    }
    #[test]
    fn absent_supply_and_disconnected_demand_get_exact_certificates() -> Result<()> {
        for supplies in [
            Vec::new(),
            vec![Supply {
                id: 1,
                units: 10,
                eligible: 1,
            }],
        ] {
            let needs = demands(&[1, 5]);
            let result = allocate(&supplies, &needs, MAX_WORK)?;
            verify(&supplies, &needs, &result);
        }
        Ok(())
    }
    #[test]
    fn highest_mask_bit_and_large_integral_counts_do_not_round() -> Result<()> {
        let needs = demands(&[1; 32]);
        let supplies = vec![Supply {
            id: 1,
            units: 1,
            eligible: 1 << 31,
        }];
        verify(&supplies, &needs, &allocate(&supplies, &needs, MAX_WORK)?);
        let needs = demands(&[9_007_199_254_740_993]);
        let supplies = vec![Supply {
            id: 1,
            units: 9_007_199_254_740_993,
            eligible: 1,
        }];
        let result = allocate(&supplies, &needs, MAX_WORK)?;
        assert_eq!(result.allocated_units, 9_007_199_254_740_993);
        Ok(())
    }
    #[test]
    fn malformed_overflowing_and_exhausted_inputs_return_no_partial_solution() {
        let needs = demands(&[1]);
        for supply in [
            Supply {
                id: 0,
                units: 1,
                eligible: 1,
            },
            Supply {
                id: 1,
                units: 0,
                eligible: 1,
            },
            Supply {
                id: 1,
                units: 1,
                eligible: 2,
            },
            Supply {
                id: 1,
                units: 1,
                eligible: 0,
            },
        ] {
            assert_eq!(
                allocate(&[supply], &needs, MAX_WORK),
                Err(AllocationError::InvalidInput)
            );
        }
        assert_eq!(
            allocate(&[], &demands(&[u64::MAX, 1]), MAX_WORK),
            Err(AllocationError::ArithmeticOverflow)
        );
        assert_eq!(
            allocate(&[], &needs, 1),
            Err(AllocationError::BudgetExceeded)
        );
        let repeated = vec![
            Supply {
                id: 1,
                units: 1,
                eligible: 1
            };
            2
        ];
        assert_eq!(
            allocate(&repeated, &needs, MAX_WORK),
            Err(AllocationError::InvalidInput)
        );
        assert!(allocate(&[], &demands(&[1; 33]), MAX_WORK).is_err());
    }
    fn brute_force(units: &[u32], remaining: &mut [u64], index: usize) -> u64 {
        if index == units.len() {
            return 0;
        }
        let mut best = brute_force(units, remaining, index + 1);
        for d in 0..remaining.len() {
            if units[index] & (1 << d) != 0 && remaining[d] > 0 {
                remaining[d] -= 1;
                best = best.max(1 + brute_force(units, remaining, index + 1));
                remaining[d] += 1;
            }
        }
        best
    }
    #[test]
    fn exhaustive_small_models_match_independent_enumeration_and_cut_checks() -> Result<()> {
        for a in 1..8u32 {
            for b in 1..8u32 {
                for x in 1..=2u64 {
                    for y in 1..=2u64 {
                        for demand_bits in 0..8 {
                            let needs = demands(
                                &(0..3)
                                    .map(|d| 1 + ((demand_bits >> d) & 1))
                                    .collect::<Vec<u64>>(),
                            );
                            let supplies = vec![
                                Supply {
                                    id: 1,
                                    units: x,
                                    eligible: a,
                                },
                                Supply {
                                    id: 2,
                                    units: y,
                                    eligible: b,
                                },
                            ];
                            let mut units = vec![a; x as usize];
                            units.extend(vec![b; y as usize]);
                            let mut remaining = needs.iter().map(|d| d.units).collect::<Vec<_>>();
                            let expected = brute_force(&units, &mut remaining, 0);
                            let result = allocate(&supplies, &needs, MAX_WORK)?;
                            assert_eq!(
                                result.allocated_units, expected,
                                "masks {a}/{b}, units {x}/{y}, demands {demand_bits}"
                            );
                            verify(&supplies, &needs, &result);
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
