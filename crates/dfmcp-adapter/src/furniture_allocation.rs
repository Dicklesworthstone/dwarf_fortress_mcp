//! Exact global furniture assignment over a caller-declared candidate model.
//!
//! One item satisfies one slot. Minimize total same-level Manhattan distance,
//! then the item-ID vector in lexical slot-name order. No partial executable
//! assignment, reservation, native eligibility or observation authority is created.
//! Beads: df-dfhack-bridge-plane-c-pic.3 / df-dfhack-bridge-plane-c-pic.4.

use dfmcp_core::{DfmcpError, ErrorCode, Result};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_SLOTS: usize = 32;
pub const MAX_CANDIDATES: usize = 65_536;
pub const MAX_EXCLUDED: usize = 4_096;
pub const MAX_DISTANCE: u32 = 65_532;
pub const MAX_ITEM_ID: u32 = 2_147_483_646;

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}
fn invariant(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InternalInvariantViolation, message)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Bed,
    Chair,
    Table,
}
impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bed => "bed",
            Self::Chair => "chair",
            Self::Table => "table",
        }
    }
    pub fn native_key(self) -> &'static str {
        match self {
            Self::Bed => "BED",
            Self::Chair => "CHAIR",
            Self::Table => "TABLE",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slot {
    pub name: String,
    pub kind: Kind,
    pub target: [u32; 3],
    pub after: Vec<String>,
    pub material: Option<(i32, i32)>,
    pub subtype: Option<i32>,
    pub max_distance: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub slots: Vec<Slot>,
    pub excluded_items: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub native_id: u32,
    pub kind: Kind,
    pub position: [u32; 3],
    pub material_type: i32,
    pub material_index: i32,
    pub subtype: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assignment {
    pub slot: String,
    pub item_id: u32,
    pub distance: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortage {
    pub slots: Vec<String>,
    pub candidate_items: Vec<u32>,
    pub missing: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Allocation {
    pub assignments: Vec<Assignment>,
    pub compatible_counts: Vec<(String, u32)>,
    pub maximum_assignable: usize,
    pub shortage: Option<Shortage>,
    pub total_distance: Option<u64>,
}

fn name_valid(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 48
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

/// Normalize independently of input order, while preserving every hard constraint.
/// Dependency order is separate from allocation's lexical slot objective.
pub fn normalize(requested: &Request, guard: &mut dyn FnMut() -> Result<()>) -> Result<Request> {
    guard()?;
    if requested.slots.is_empty()
        || requested.slots.len() > MAX_SLOTS
        || requested.excluded_items.len() > MAX_EXCLUDED
    {
        return Err(invalid(
            "furniture request exceeds slot or exclusion bounds",
        ));
    }
    let mut targets = BTreeSet::new();
    // Check all variable-length fields before cloning or sorting the request.
    for slot in &requested.slots {
        guard()?;
        if !name_valid(&slot.name)
            || !(1..=32_766).contains(&slot.target[0])
            || !(1..=32_766).contains(&slot.target[1])
            || slot.target[2] > 32_767
            || slot.max_distance > MAX_DISTANCE
            || slot
                .material
                .is_some_and(|(kind, index)| kind < 0 || index < -1)
            || slot.subtype.is_some_and(|value| value < -1)
            || slot.after.len() >= MAX_SLOTS
            || !targets.insert(slot.target)
        {
            return Err(invalid(
                "invalid or duplicate furniture target and constraints",
            ));
        }
        for dependency in &slot.after {
            guard()?;
            if !name_valid(dependency) || dependency == &slot.name {
                return Err(invalid("invalid or self-referencing furniture dependency"));
            }
        }
    }
    let mut request = requested.clone();
    for slot in &mut request.slots {
        guard()?;
        slot.after.sort();
        if slot.after.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(invalid("duplicate furniture dependency"));
        }
    }
    request.slots.sort_by(|a, b| a.name.cmp(&b.name));
    if request
        .slots
        .windows(2)
        .any(|pair| pair[0].name == pair[1].name)
    {
        return Err(invalid("duplicate furniture slot name"));
    }
    let names: BTreeSet<_> = request
        .slots
        .iter()
        .map(|slot| slot.name.as_str())
        .collect();
    for slot in &request.slots {
        for dependency in &slot.after {
            guard()?;
            if !names.contains(dependency.as_str()) {
                return Err(invalid("unresolved furniture dependency"));
            }
        }
    }
    let mut completed = BTreeSet::new();
    while completed.len() < request.slots.len() {
        let mut next = None;
        for slot in &request.slots {
            guard()?;
            if !completed.contains(slot.name.as_str())
                && slot
                    .after
                    .iter()
                    .all(|name| completed.contains(name.as_str()))
            {
                next = Some(slot.name.as_str());
                break;
            }
        }
        let Some(name) = next else {
            return Err(invalid("cyclic furniture dependencies"));
        };
        completed.insert(name);
    }
    for &identity in &request.excluded_items {
        guard()?;
        if identity > MAX_ITEM_ID {
            return Err(invalid("excluded furniture identity outside native bounds"));
        }
    }
    request.excluded_items.sort_unstable();
    if request
        .excluded_items
        .windows(2)
        .any(|pair| pair[0] == pair[1])
    {
        return Err(invalid("duplicate excluded furniture item"));
    }
    guard()?;
    Ok(request)
}

fn validate_candidate(candidate: &Candidate) -> Result<()> {
    if candidate.native_id > MAX_ITEM_ID
        || candidate.position.iter().any(|&value| value > 32_767)
        || candidate.material_type < 0
        || candidate.material_index < -1
        || candidate.subtype < -1
    {
        return Err(invalid(
            "invalid furniture candidate identity or attributes",
        ));
    }
    Ok(())
}

fn distance(slot: &Slot, candidate: &Candidate) -> Option<u32> {
    if slot.kind != candidate.kind
        || slot.target[2] != candidate.position[2]
        || slot
            .material
            .is_some_and(|value| value != (candidate.material_type, candidate.material_index))
        || slot.subtype.is_some_and(|value| value != candidate.subtype)
    {
        return None;
    }
    let distance = slot.target[0].abs_diff(candidate.position[0])
        + slot.target[1].abs_diff(candidate.position[1]);
    (distance <= slot.max_distance).then_some(distance)
}

type Edges = Vec<BTreeMap<u32, u32>>;

fn augment(
    row: usize,
    edges: &Edges,
    owners: &mut BTreeMap<u32, usize>,
    seen: &mut BTreeSet<u32>,
    guard: &mut dyn FnMut() -> Result<()>,
) -> Result<bool> {
    // Every recursive step traverses a distinct matched item; depth <= 32.
    for &identity in edges[row].keys() {
        guard()?;
        if !seen.insert(identity) {
            continue;
        }
        let prior = owners.get(&identity).copied();
        if match prior {
            None => true,
            Some(previous) => augment(previous, edges, owners, seen, guard)?,
        } {
            owners.insert(identity, row);
            return Ok(true);
        }
    }
    Ok(false)
}

fn maximum_matching(
    edges: &Edges,
    slots: &[Slot],
    counts: &[u32],
    guard: &mut dyn FnMut() -> Result<()>,
) -> Result<(usize, Option<Shortage>)> {
    let mut owners = BTreeMap::new();
    for row in 0..edges.len() {
        guard()?;
        augment(row, edges, &mut owners, &mut BTreeSet::new(), guard)?;
    }
    if owners.len() == edges.len() {
        return Ok((owners.len(), None));
    }
    let matched: BTreeSet<_> = owners.values().copied().collect();
    let mut rows: BTreeSet<_> = (0..edges.len())
        .filter(|row| !matched.contains(row))
        .collect();
    let mut queue: Vec<_> = rows.iter().copied().collect();
    let mut neighbors = BTreeSet::new();
    let mut cursor = 0;
    while cursor < queue.len() {
        guard()?;
        let row = queue[cursor];
        cursor += 1;
        for &identity in edges[row].keys() {
            guard()?;
            neighbors.insert(identity);
            let owner = owners
                .get(&identity)
                .ok_or_else(|| invariant("furniture maximum matching left an augmenting path"))?;
            if rows.insert(*owner) {
                queue.push(*owner);
            }
        }
    }
    if rows.len() <= neighbors.len()
        || rows
            .iter()
            .any(|&row| counts[row] as usize != edges[row].len())
    {
        return Err(invariant(
            "furniture shortage does not cover full candidate graph",
        ));
    }
    let missing = (rows.len() - neighbors.len()) as u32;
    Ok((
        owners.len(),
        Some(Shortage {
            slots: rows.iter().map(|&row| slots[row].name.clone()).collect(),
            candidate_items: neighbors.into_iter().collect(),
            missing,
        }),
    ))
}

/// Fixed-width ordered additive cost. The ID coordinates encode one selected
/// ID per lexical slot, so no huge scalar, float or external bigint is needed.
/// One guarded cost operation performs at most 33 checked integer operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LexCost {
    distance: i64,
    ids: [i64; MAX_SLOTS],
}
impl LexCost {
    const ZERO: Self = Self {
        distance: 0,
        ids: [0; MAX_SLOTS],
    };
    fn edge(distance: u32, identity: u32, row: usize) -> Self {
        let mut cost = Self::ZERO;
        cost.distance = i64::from(distance);
        cost.ids[row] = i64::from(identity);
        cost
    }
    fn combine(self, other: Self, subtract: bool) -> Result<Self> {
        let arithmetic = |left: i64, right: i64| {
            if subtract {
                left.checked_sub(right)
            } else {
                left.checked_add(right)
            }
            .ok_or_else(|| invariant("furniture lexicographic cost overflow"))
        };
        let mut out = Self::ZERO;
        out.distance = arithmetic(self.distance, other.distance)?;
        for (index, value) in out.ids.iter_mut().enumerate() {
            *value = arithmetic(self.ids[index], other.ids[index])?;
        }
        Ok(out)
    }
}
impl Ord for LexCost {
    fn cmp(&self, other: &Self) -> Ordering {
        self.distance
            .cmp(&other.distance)
            .then_with(|| self.ids.cmp(&other.ids))
    }
}
impl PartialOrd for LexCost {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn minimum_cost(edges: &Edges, guard: &mut dyn FnMut() -> Result<()>) -> Result<Vec<u32>> {
    let identities: Vec<_> = edges
        .iter()
        .flat_map(|row| row.keys().copied())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let (n, m) = (edges.len(), identities.len());
    if n > m || m > n * n {
        return Err(invariant("invalid reduced furniture assignment shape"));
    }
    let mut u = vec![LexCost::ZERO; n + 1];
    let mut v = vec![LexCost::ZERO; m + 1];
    let mut owners = vec![0usize; m + 1];
    let mut way = vec![0usize; m + 1];
    for row in 1..=n {
        guard()?;
        owners[0] = row;
        let mut column = 0;
        let mut minimum: Vec<Option<LexCost>> = vec![None; m + 1];
        let mut used = vec![false; m + 1];
        loop {
            guard()?;
            used[column] = true;
            let active = owners[column];
            let mut delta: Option<(LexCost, usize)> = None;
            for j in 1..=m {
                guard()?;
                if used[j] {
                    continue;
                }
                if let Some(&distance) = edges[active - 1].get(&identities[j - 1]) {
                    guard()?;
                    let cost = LexCost::edge(distance, identities[j - 1], active - 1)
                        .combine(u[active], true)?
                        .combine(v[j], true)?;
                    if minimum[j].is_none_or(|previous| cost < previous) {
                        minimum[j] = Some(cost);
                        way[j] = column;
                    }
                }
                if let Some(cost) = minimum[j]
                    && delta.is_none_or(|(best, _)| cost < best)
                {
                    delta = Some((cost, j));
                }
            }
            let Some((change, following)) = delta else {
                return Err(invariant("furniture cost solver has no augmenting column"));
            };
            for j in 0..=m {
                guard()?;
                if used[j] {
                    u[owners[j]] = u[owners[j]].combine(change, false)?;
                    v[j] = v[j].combine(change, true)?;
                } else if let Some(cost) = minimum[j] {
                    minimum[j] = Some(cost.combine(change, true)?);
                }
            }
            column = following;
            if owners[column] == 0 {
                break;
            }
        }
        while column != 0 {
            guard()?;
            let previous = way[column];
            owners[column] = owners[previous];
            column = previous;
        }
    }
    let mut chosen = vec![None; n];
    for column in 1..=m {
        guard()?;
        if owners[column] != 0 {
            chosen[owners[column] - 1] = Some(identities[column - 1]);
        }
    }
    chosen
        .into_iter()
        .enumerate()
        .map(|(row, identity)| {
            identity
                .filter(|value| edges[row].contains_key(value))
                .ok_or_else(|| invariant("furniture cost solver lost a complete assignment"))
        })
        .collect()
}

/// Allocate every slot or return a complete Hall shortage with no assignments.
///
/// For N slots, retain each slot's N best candidates by distance and ID. If an
/// optimum used a worse candidate, at least one of those N better candidates
/// would be unused by the other N-1 slots and improve its objective. A deficient
/// Hall set has fewer than N neighbors, so every member's list is untrimmed.
pub fn allocate(
    requested: &Request,
    candidates: &[Candidate],
    guard: &mut dyn FnMut() -> Result<()>,
) -> Result<Allocation> {
    let request = normalize(requested, guard)?;
    if candidates.len() > MAX_CANDIDATES {
        return Err(invalid("furniture candidate roster exceeds 65,536 items"));
    }
    let n = request.slots.len();
    let mut seen = BTreeSet::new();
    let mut counts = vec![0u32; n];
    let mut best = vec![BTreeSet::<(u32, u32)>::new(); n];
    for candidate in candidates {
        guard()?;
        validate_candidate(candidate)?;
        if !seen.insert(candidate.native_id) {
            return Err(invalid("duplicate furniture candidate item identity"));
        }
        if request
            .excluded_items
            .binary_search(&candidate.native_id)
            .is_ok()
        {
            continue;
        }
        for (row, slot) in request.slots.iter().enumerate() {
            guard()?;
            if let Some(distance) = distance(slot, candidate) {
                counts[row] = counts[row]
                    .checked_add(1)
                    .ok_or_else(|| invariant("furniture compatibility count overflow"))?;
                best[row].insert((distance, candidate.native_id));
                if best[row].len() > n {
                    let _ = best[row].pop_last();
                }
            }
        }
    }
    let edges: Edges = best
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|(distance, identity)| (identity, distance))
                .collect()
        })
        .collect();
    guard()?;
    let (maximum_assignable, shortage) = maximum_matching(&edges, &request.slots, &counts, guard)?;
    let mut result = Allocation {
        assignments: Vec::new(),
        compatible_counts: request
            .slots
            .iter()
            .zip(counts)
            .map(|(slot, count)| (slot.name.clone(), count))
            .collect(),
        maximum_assignable,
        shortage,
        total_distance: None,
    };
    if result.shortage.is_none() {
        let chosen = minimum_cost(&edges, guard)?;
        let mut total = 0u64;
        for (row, identity) in chosen.into_iter().enumerate() {
            guard()?;
            let distance = *edges[row]
                .get(&identity)
                .ok_or_else(|| invariant("chosen furniture edge missing"))?;
            total = total
                .checked_add(u64::from(distance))
                .ok_or_else(|| invariant("furniture total distance overflow"))?;
            result.assignments.push(Assignment {
                slot: request.slots[row].name.clone(),
                item_id: identity,
                distance,
            });
        }
        result.total_distance = Some(total);
    }
    guard()?;
    Ok(result)
}

#[cfg(test)]
#[path = "furniture_allocation_tests.rs"]
mod tests;
