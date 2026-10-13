//! Rooted directed bottlenecks in an already-authorized observed projection.
//! Dominance is about these selected relation edges, not game walkability,
//! production feasibility, causal necessity, or complete-world absence.

use super::{
    DfmcpError, Digest32, EdgeKind, EntityId, ErrorCode, GraphBudget, GraphDirection,
    GraphWitness, Index, MAX_VERTICES, Result, WorldSnapshot, budget_error, invariant,
    put_bytes, put_str, put_u64,
};

pub const DOMINANCE_POLICY: &str = "observed-directed-dominators-rpo/1";
const NONE: usize = usize::MAX;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DominanceQuery {
    pub root: EntityId,
    /// Explicit relation semantics are mandatory; an empty selection is rejected.
    pub edge_kinds: Vec<EdgeKind>,
    /// Outgoing follows stored edges; Incoming reverses them. Undirected is rejected.
    pub direction: GraphDirection,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DominatorRecord {
    pub entity_id: EntityId,
    pub generation: u32,
    pub revision: u64,
    /// None only for the root. This is a dominator-tree link, NOT a graph edge.
    pub immediate_dominator: Option<EntityId>,
    /// Other currently reachable vertices made unreachable by deleting this vertex.
    /// Excludes the deleted vertex itself; deleting the root loses all other reachability.
    pub dominated_descendants: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DominanceAnalysis {
    pub root: EntityId,
    pub direction: GraphDirection,
    /// Reachable vertices, ordered by stable entity identity (not DFS order).
    pub vertices: Vec<DominatorRecord>,
    /// Present in the supplied projection, but unreachable from this root.
    /// No dominator is assigned to these vertices, including unreachable cycles.
    pub unreachable: Vec<EntityId>,
    pub fixed_point_passes: u64,
    pub intersection_steps: u64,
    pub witness: GraphWitness,
}

impl DominanceAnalysis {
    /// Root-to-target sequence of unavoidable vertices, including both endpoints.
    /// This is NOT a traversable path: adjacent dominators need not share an edge.
    /// None means the target was not reachable in this analysis, not world absence.
    pub fn dominator_chain_to(&self, target: EntityId) -> Result<Option<Vec<EntityId>>> {
        if self.vertices.len() > MAX_VERTICES {
            return Err(budget_error("dominator chain exceeds the vertex bound"));
        }
        let records: std::collections::BTreeMap<_, _> = self
            .vertices
            .iter()
            .map(|record| (record.entity_id, record))
            .collect();
        if records.len() != self.vertices.len() {
            return Err(invariant("dominator witness contains duplicate vertices"));
        }
        let Some(mut current) = records.get(&target).copied() else {
            return Ok(None);
        };
        let mut chain = Vec::new();
        for _ in 0..records.len() {
            chain.push(current.entity_id);
            match current.immediate_dominator {
                None if current.entity_id == self.root => {
                    chain.reverse();
                    return Ok(Some(chain));
                }
                Some(parent) if current.entity_id != self.root => {
                    current = records.get(&parent).copied().ok_or_else(|| {
                        invariant("dominator witness references an absent parent")
                    })?;
                }
                _ => return Err(invariant("dominator witness has an inconsistent root")),
            }
        }
        Err(invariant("dominator witness contains a parent cycle"))
    }
}

struct Run<F> {
    index: Index,
    check: F,
    next_check: u64,
    intersection_steps: u64,
}

impl<F: FnMut() -> Result<()>> Run<F> {
    fn charge(&mut self) -> Result<()> {
        self.index.charge(1)?;
        if self.index.work >= self.next_check {
            (self.check)()?;
            self.next_check = self.index.work + 128;
        }
        Ok(())
    }

    fn arc(&mut self) -> Result<()> {
        self.charge()?;
        self.index.examined_arcs += 1;
        Ok(())
    }

    fn intersect(
        &mut self,
        mut left: usize,
        mut right: usize,
        parents: &[usize],
        rank: &[usize],
    ) -> Result<usize> {
        while left != right {
            self.charge()?;
            self.intersection_steps += 1;
            let cursor = if rank[left] > rank[right] {
                &mut left
            } else {
                &mut right
            };
            let parent = parents[*cursor];
            if parent == NONE || rank[parent] >= rank[*cursor] {
                return Err(invariant("dominator intersection does not approach the root"));
            }
            *cursor = parent;
        }
        Ok(left)
    }
}

pub fn analyze_dominators(
    snapshot: &WorldSnapshot,
    scope_digest: Digest32,
    query: &DominanceQuery,
    budget: GraphBudget,
) -> Result<DominanceAnalysis> {
    analyze_dominators_with_check(snapshot, scope_digest, query, budget, || Ok(()))
}

/// Iterative reverse-postorder immediate dominators, with linear auxiliary space.
/// Work is bounded across DFS, fixed-point passes, every intersection ascent,
/// impact accumulation, and result encoding. Exhaustion returns NO partial tree.
/// The callback runs before/after shared index construction, every 128 analysis
/// work units, and before return. Shared snapshot hashing/index construction is
/// count-bounded but is not itself cooperatively interrupted by this callback.
/// Callers must apply authorization before supplying the snapshot.
pub fn analyze_dominators_with_check<F: FnMut() -> Result<()>>(
    snapshot: &WorldSnapshot,
    scope_digest: Digest32,
    query: &DominanceQuery,
    budget: GraphBudget,
    mut check: F,
) -> Result<DominanceAnalysis> {
    check()?;
    if query.edge_kinds.is_empty() || query.direction == GraphDirection::Undirected {
        return Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "dominators require explicit edge kinds and outgoing or incoming direction",
        ));
    }
    let mut index = Index::build(snapshot, &query.edge_kinds, budget)?;
    check()?;
    let root = index.ordinals.get(&query.root).copied().ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::InvalidRequest,
            "dominator root is not present in the authorized projection",
        )
    })?;
    if query.direction == GraphDirection::Incoming {
        std::mem::swap(&mut index.outgoing, &mut index.incoming);
    }
    let n = index.ids.len();
    let next_check = index.work + 128;
    let mut run = Run {
        index,
        check,
        next_check,
        intersection_steps: 0,
    };
    let mut seen = vec![false; n];
    let mut order = Vec::new();
    let mut stack = vec![(root, 0usize)];
    run.index.frontier(stack.len())?;
    seen[root] = true;
    while let Some((node, next)) = stack.last_mut() {
        run.charge()?;
        if let Some(arc) = run.index.outgoing[*node].get(*next).copied() {
            *next += 1;
            run.arc()?;
            if !seen[arc.target] {
                run.index.frontier(stack.len() + 1)?;
                seen[arc.target] = true;
                stack.push((arc.target, 0));
            }
        } else {
            order.push(*node);
            stack.pop();
        }
    }
    order.reverse();
    let mut rank = vec![NONE; n];
    for (position, &node) in order.iter().enumerate() {
        run.charge()?;
        rank[node] = position;
    }
    let mut parents = vec![NONE; n];
    parents[root] = root;
    let mut passes = 0u64;
    let mut trace = Digest32::of_bytes(DOMINANCE_POLICY.as_bytes());
    loop {
        run.charge()?;
        passes += 1;
        let mut changed = false;
        for &node in order.iter().skip(1) {
            run.charge()?;
            let mut parent = NONE;
            for offset in 0..run.index.incoming[node].len() {
                run.arc()?;
                let predecessor = run.index.incoming[node][offset].target;
                if parents[predecessor] == NONE {
                    continue;
                }
                parent = if parent == NONE {
                    predecessor
                } else {
                    run.intersect(parent, predecessor, &parents, &rank)?
                };
            }
            if parent == NONE || rank[parent] >= rank[node] {
                return Err(invariant("reachable vertex has no preceding dominator"));
            }
            if parents[node] != parent {
                // Fold each actual parent decision into a fixed-size transcript.
                // No O(passes * vertices) retained decision log.
                let mut decision = Vec::new();
                put_bytes(&mut decision, trace.as_bytes());
                put_u64(&mut decision, passes);
                put_u64(&mut decision, run.index.ids[node].get());
                put_u64(
                    &mut decision,
                    if parents[node] == NONE {
                        0
                    } else {
                        run.index.ids[parents[node]].get()
                    },
                );
                put_u64(&mut decision, run.index.ids[parent].get());
                trace = Digest32::of_bytes(&decision);
                parents[node] = parent;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // Every strict dominator precedes its children in reverse postorder.
    // Accumulate each subtree once, rather than walking every ancestor chain.
    let mut sizes = vec![1u64; n];
    for &node in order.iter().skip(1).rev() {
        run.charge()?;
        let parent = parents[node];
        sizes[parent] = sizes[parent]
            .checked_add(sizes[node])
            .ok_or_else(|| invariant("dominator impact count overflow"))?;
    }
    let mut decisions = Vec::new();
    put_str(&mut decisions, DOMINANCE_POLICY);
    let anchor = snapshot.anchor();
    put_str(&mut decisions, &anchor.fortress_id.to_string());
    put_u64(&mut decisions, anchor.cursor.epoch);
    put_u64(&mut decisions, anchor.cursor.sequence);
    put_u64(&mut decisions, anchor.tick.0);
    put_bytes(&mut decisions, anchor.state_hash.as_bytes());
    put_u64(&mut decisions, query.root.get());
    decisions.push(u8::from(query.direction == GraphDirection::Incoming));
    put_u64(&mut decisions, budget.max_vertices as u64);
    put_u64(&mut decisions, budget.max_edges as u64);
    put_u64(&mut decisions, budget.max_frontier as u64);
    put_u64(&mut decisions, budget.max_work);
    put_u64(&mut decisions, passes);
    put_u64(&mut decisions, run.intersection_steps);
    put_bytes(&mut decisions, trace.as_bytes());
    put_u64(&mut decisions, order.len() as u64);
    for &node in &order {
        run.charge()?;
        put_u64(&mut decisions, run.index.ids[node].get());
    }
    let mut vertices = Vec::new();
    let mut unreachable = Vec::new();
    for node in 0..n {
        run.charge()?;
        let id = run.index.ids[node];
        put_u64(&mut decisions, id.get());
        decisions.push(u8::from(seen[node]));
        if !seen[node] {
            unreachable.push(id);
            continue;
        }
        let immediate_dominator = (node != root).then(|| run.index.ids[parents[node]]);
        let dominated_descendants = sizes[node] - 1;
        put_u64(&mut decisions, immediate_dominator.map_or(0, EntityId::get));
        put_u64(&mut decisions, dominated_descendants);
        let (generation, revision) = run.index.versions[node];
        vertices.push(DominatorRecord {
            entity_id: id,
            generation,
            revision,
            immediate_dominator,
            dominated_descendants,
        });
    }
    let witness = run.index.witness(snapshot, scope_digest, &decisions);
    (run.check)()?;
    Ok(DominanceAnalysis {
        root: query.root,
        direction: query.direction,
        vertices,
        unreachable,
        fixed_point_passes: passes,
        intersection_steps: run.intersection_steps,
        witness,
    })
}
