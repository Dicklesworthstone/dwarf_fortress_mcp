//! Finite, shared production capacity for the two registered laboratory recipes.
//!
//! A completed workshop and a living labor-enabled unit each service one order
//! during an interval. These assignments reserve neither game resources nor
//! native workers; they are decisions inside the reference simulation only.

use std::cmp::Reverse;
use std::collections::VecDeque;

use super::*;

const FAMILIES: [(&str, &str); 2] = [
    ("workshop:Still", "BREW"),
    ("workshop:Kitchen", "COOK"),
];

fn family(job: &str) -> Option<usize> {
    match job {
        "BREW_DRINK" => Some(0),
        "PREPARE_MEAL" | "COOK_MEAL" => Some(1),
        _ => None,
    }
}

pub(super) fn requirements(job: &str) -> Option<(&'static str, &'static str)> {
    family(job).map(|index| FAMILIES[index])
}

pub(super) struct Selection {
    pub(super) ready: Vec<usize>,
    pub(super) waiting: BTreeMap<usize, String>,
}

struct Candidate {
    order: usize,
    family: usize,
    partial: u64,
}

struct Capacity {
    workers: usize,
    eligible: [Vec<usize>; FAMILIES.len()],
    workshops: [usize; FAMILIES.len()],
}

impl Capacity {
    fn capture(snapshot: &WorldSnapshot, budget: &mut EffectAdvanceBudget) -> Result<Self> {
        if snapshot.graph.entities.len() > MAX_EFFECT_ADVANCE_PRODUCTION_ENTITIES {
            return Err(advance_budget_error(
                "production capacity exceeds its explicit canonical entity bound",
            ));
        }
        // Reserve every fixed family/field visit before building the bounded
        // eligibility lists. Both lists refer to one shared set of unit IDs.
        budget.charge(snapshot.graph.entities.len() as u64 * 8 + 1)?;
        let mut capacity = Self {
            workers: 0,
            eligible: [Vec::new(), Vec::new()],
            workshops: [0, 0],
        };
        for record in snapshot.graph.entities.values() {
            if record.kind == EntityKind::Unit && is_alive(record, snapshot.tick) {
                let eligibility = FAMILIES.map(|(_, labor)| {
                    field_value(record, &format!("{LABOR_FIELD_PREFIX}{labor}"), snapshot.tick)
                        == Some(&Value::Bool(true))
                });
                if eligibility.iter().any(|eligible| *eligible) {
                    for (index, eligible) in eligibility.into_iter().enumerate() {
                        if eligible {
                            capacity.eligible[index].push(capacity.workers);
                        }
                    }
                    capacity.workers += 1;
                }
            } else if record.kind == EntityKind::Building
                && field_text(record, CONSTRUCTION_STAGE_FIELD, snapshot.tick)
                    == Some(STAGE_COMPLETE)
            {
                for (index, (workshop, _)) in FAMILIES.iter().enumerate() {
                    if field_text(record, "building_kind", snapshot.tick) == Some(*workshop) {
                        capacity.workshops[index] += 1;
                    }
                }
            }
        }
        Ok(capacity)
    }
}

/// Add one candidate while retaining every earlier accepted order. An iterative
/// augmenting path may move a flexible worker so a specialist-only assignment
/// is not stranded by a greedy first match. Workshop slots are per family;
/// the registered families have distinct workshop kinds and identical worker
/// eligibility for every order within a family.
fn augment(
    root: usize,
    candidates: &[Candidate],
    capacity: &Capacity,
    owner: &mut [Option<usize>],
    assigned: &mut [Option<usize>],
    budget: &mut EffectAdvanceBudget,
) -> Result<bool> {
    budget.charge(capacity.workers as u64 * 2 + candidates.len() as u64 + 1)?;
    let mut seen = vec![false; capacity.workers];
    let mut parent = vec![None; capacity.workers];
    let mut queue = VecDeque::from([root]);
    while let Some(candidate) = queue.pop_front() {
        budget.charge(1)?;
        for worker in &capacity.eligible[candidates[candidate].family] {
            budget.charge(1)?;
            if seen[*worker] {
                continue;
            }
            seen[*worker] = true;
            parent[*worker] = Some(candidate);
            if let Some(previous) = owner[*worker] {
                queue.push_back(previous);
                continue;
            }
            let mut available = *worker;
            loop {
                budget.charge(1)?;
                let next = parent[available].ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::InternalInvariantViolation,
                        "production capacity lost an augmenting-path predecessor",
                    )
                })?;
                let previous = assigned[next].replace(available);
                owner[available] = Some(next);
                if let Some(previous) = previous {
                    available = previous;
                } else {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

/// Select a maximum-cardinality set of eligible production orders, preferring
/// earned partial work and then canonical entity order. A unit or a workshop
/// cannot appear twice. Conditions have already been evaluated at this exact
/// source boundary; an ineligible order consumes no service capacity.
pub(super) fn select(
    snapshot: &WorldSnapshot,
    orders: &[TimelineOrder],
    ready: Vec<usize>,
    budget: &mut EffectAdvanceBudget,
) -> Result<Selection> {
    budget.charge(ready.len() as u64 * 4 + 1)?;
    if snapshot.graph.entities.len() > MAX_EFFECT_ADVANCE_PRODUCTION_ENTITIES
        && ready.iter().any(|index| family(&orders[*index].job).is_some())
    {
        return Err(advance_budget_error(
            "production capacity exceeds its explicit canonical entity bound",
        ));
    }
    let mut candidates = Vec::new();
    let mut selected = Vec::new();
    for index in ready {
        if let Some(family) = family(&orders[index].job) {
            candidates.push(Candidate {
                order: index,
                family,
                partial: field_u64(
                    entity(snapshot, orders[index].id)?,
                    "work_ticks",
                    snapshot.tick,
                )?,
            });
        } else {
            // No workshop or labor requirement is registered for this job.
            // Preserve its existing deliberately abstract reference behavior.
            selected.push(index);
        }
    }
    if candidates.is_empty() {
        return Ok(Selection {
            ready: selected,
            waiting: BTreeMap::new(),
        });
    }
    let capacity = Capacity::capture(snapshot, budget)?;
    let depth = usize::BITS - candidates.len().leading_zeros();
    budget.charge(candidates.len() as u64 * (u64::from(depth) + 1))?;
    candidates.sort_by_key(|candidate| {
        (Reverse(candidate.partial), orders[candidate.order].id)
    });
    budget.charge(capacity.workers as u64 + candidates.len() as u64 + 1)?;
    let mut owner = vec![None; capacity.workers];
    let mut assigned = vec![None; candidates.len()];
    let mut used_workshops = [0; FAMILIES.len()];
    for (index, candidate) in candidates.iter().enumerate() {
        budget.charge(1)?;
        if used_workshops[candidate.family] < capacity.workshops[candidate.family]
            && augment(
                index,
                &candidates,
                &capacity,
                &mut owner,
                &mut assigned,
                budget,
            )?
        {
            used_workshops[candidate.family] += 1;
        }
    }
    let mut waiting = BTreeMap::new();
    for (index, candidate) in candidates.iter().enumerate() {
        budget.charge(1)?;
        if assigned[index].is_some() {
            selected.push(candidate.order);
        } else {
            let (workshop, labor) = FAMILIES[candidate.family];
            let reason = if used_workshops[candidate.family] == capacity.workshops[candidate.family] {
                format!(
                    "production capacity busy: every established {workshop} is assigned to another work order"
                )
            } else {
                format!(
                    "production capacity busy: living units with {labor} enabled are assigned to other work orders"
                )
            };
            waiting.insert(candidate.order, reason);
        }
    }
    // Capacity preference decides who earns time; completion ties still settle
    // by canonical entity order, independent of assignment-search traversal.
    let depth = usize::BITS - selected.len().leading_zeros();
    budget.charge(selected.len() as u64 * (u64::from(depth) + 1))?;
    selected.sort_unstable();
    Ok(Selection {
        ready: selected,
        waiting,
    })
}
