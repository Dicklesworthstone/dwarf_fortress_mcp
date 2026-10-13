#![forbid(unsafe_code)]

//! Exact route redundancy in the bounded, observed floor/stair model.
//! Unit capacities count independent paths, NOT dwarves per tick or safe exits.
//! A feasible path packing and an equally sized separating cut are checked
//! independently before publication. No partial packing is a negative answer.

use std::collections::{BTreeSet, VecDeque};

use crate::map_region::{Cell, MAX_ROUTE_WORK, MapError, MapRegion};

pub const RESILIENCE_POLICY: &str = "observed-route-disjoint-paths-unit-flow/1";
const MAX_PATHS: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disjointness {
    Edges,
    InternalTiles,
}
impl Disjointness {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Edges => "edges",
            Self::InternalTiles => "internal_tiles",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CutElement {
    /// Never one of the two endpoints.
    Tile(usize),
    /// Ascending, undirected flattened tile indices. In InternalTiles mode,
    /// this is permitted ONLY for the direct start-goal edge: no internal
    /// tile can cut that path without deleting an endpoint.
    Edge([usize; 2]),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteResilience {
    pub start: [u32; 3],
    pub goal: [u32; 3],
    pub disjointness: Disjointness,
    /// Excluded endpoints establish neither zero redundancy nor a cut.
    pub endpoint_excluded: bool,
    /// Simple paths, each including both endpoints, in lexicographic order.
    pub paths: Vec<Vec<usize>>,
    /// Canonical minimum separator in the declared model.
    pub minimum_cut: Vec<CutElement>,
    pub candidate_tiles: u32,
    pub candidate_edges: u32,
    /// Includes projection, residual searches, extraction and certificate checks.
    pub work_units: u64,
}

struct Work<F> {
    used: u64,
    maximum: u64,
    check: F,
}
impl<F: FnMut() -> Result<(), MapError>> Work<F> {
    fn charge(&mut self) -> Result<(), MapError> {
        if self.used >= self.maximum {
            return Err(MapError::BudgetExceeded);
        }
        self.used += 1;
        if self.used == 1 || self.used.is_multiple_of(128) {
            (self.check)()?;
        }
        Ok(())
    }
}

struct TerrainGraph {
    adjacent: Vec<Vec<usize>>,
    candidates: Vec<bool>,
    edges: u32,
}
impl TerrainGraph {
    fn build<F: FnMut() -> Result<(), MapError>>(
        map: &MapRegion,
        work: &mut Work<F>,
    ) -> Result<Self, MapError> {
        let count = map.region.volume()?;
        if count != map.cells.len() {
            return Err(MapError::InvalidRegion);
        }
        let mut candidates = Vec::with_capacity(count);
        for cell in &map.cells {
            work.charge()?;
            if let Cell::Visible(tile) = cell {
                tile.validate()?;
            }
            candidates.push(matches!(cell, Cell::Visible(tile) if tile.candidate()));
        }
        let mut adjacent = vec![Vec::new(); count];
        let mut edges = 0u32;
        for current in 0..count {
            work.charge()?;
            if !candidates[current] {
                continue;
            }
            let position = map.region.position(current).ok_or(MapError::InvalidRegion)?;
            for axis in 0..3 {
                for increase in [false, true] {
                    work.charge()?;
                    let coordinate = if increase {
                        position[axis].checked_add(1)
                    } else {
                        position[axis].checked_sub(1)
                    };
                    let Some(coordinate) = coordinate else {
                        continue;
                    };
                    let mut next = position;
                    next[axis] = coordinate;
                    let Some(target) = map.region.index(next) else {
                        continue;
                    };
                    if !candidates[target] {
                        continue;
                    }
                    if axis == 2 {
                        let (Cell::Visible(a), Cell::Visible(b)) =
                            (map.cells[current], map.cells[target])
                        else {
                            return Err(MapError::InvalidTile);
                        };
                        if !(if increase {
                            a.shape.up() && b.shape.down()
                        } else {
                            a.shape.down() && b.shape.up()
                        }) {
                            continue;
                        }
                    }
                    adjacent[current].push(target);
                    if current < target {
                        edges += 1;
                    }
                }
            }
            adjacent[current].sort_unstable();
        }
        Ok(Self { adjacent, candidates, edges })
    }
}

#[derive(Clone, Copy)]
enum Meaning {
    Tile(usize),
    Step(usize, usize),
}
struct Link {
    from: usize,
    to: usize,
    flow: bool,
    meaning: Meaning,
}
#[derive(Clone, Copy)]
struct Arc {
    target: usize,
    link: usize,
    reverse: bool,
}
struct Network {
    adjacent: Vec<Vec<Arc>>,
    links: Vec<Link>,
    opposing: Vec<(usize, usize)>,
    source: usize,
    target: usize,
}
impl Network {
    fn add(&mut self, from: usize, to: usize, meaning: Meaning) -> usize {
        let link = self.links.len();
        self.links.push(Link { from, to, flow: false, meaning });
        self.adjacent[from].push(Arc { target: to, link, reverse: false });
        self.adjacent[to].push(Arc { target: from, link, reverse: true });
        link
    }

    fn build<F: FnMut() -> Result<(), MapError>>(
        graph: &TerrainGraph,
        source: usize,
        target: usize,
        mode: Disjointness,
        work: &mut Work<F>,
    ) -> Result<Self, MapError> {
        let tiles = mode == Disjointness::InternalTiles;
        let entrance = |tile| if tiles { tile * 2 } else { tile };
        let exit = |tile| if tiles { tile * 2 + 1 } else { tile };
        let mut network = Self {
            adjacent: vec![Vec::new(); graph.adjacent.len() * if tiles { 2 } else { 1 }],
            links: Vec::new(),
            opposing: Vec::new(),
            source: exit(source),
            target: entrance(target),
        };
        for (a, adjacent) in graph.adjacent.iter().enumerate() {
            work.charge()?;
            if tiles && graph.candidates[a] && a != source && a != target {
                network.add(entrance(a), exit(a), Meaning::Tile(a));
            }
            for &b in adjacent {
                work.charge()?;
                if tiles {
                    // Returning into source or leaving sink is never required
                    // for a simple source-sink path. No endpoint split is added.
                    if a != target && b != source {
                        network.add(exit(a), entrance(b), Meaning::Step(a, b));
                    }
                } else if a < b {
                    let forward = network.add(a, b, Meaning::Step(a, b));
                    let reverse = network.add(b, a, Meaning::Step(b, a));
                    network.opposing.push((forward, reverse));
                }
            }
        }
        for adjacent in &mut network.adjacent {
            work.charge()?;
            adjacent.sort_unstable_by_key(|arc| (arc.target, arc.link, arc.reverse));
        }
        Ok(network)
    }

    /// A full search also supplies the source side of the residual minimum cut.
    fn search<F: FnMut() -> Result<(), MapError>>(
        &self,
        positive: Option<&[bool]>,
        work: &mut Work<F>,
    ) -> Result<Vec<Option<Arc>>, MapError> {
        let mut parent = vec![None; self.adjacent.len()];
        // The root's sentinel is never interpreted as an edge.
        parent[self.source] = Some(Arc { target: self.source, link: 0, reverse: false });
        let mut queue = VecDeque::from([self.source]);
        while let Some(current) = queue.pop_front() {
            work.charge()?;
            for &arc in &self.adjacent[current] {
                work.charge()?;
                if parent[arc.target].is_some() {
                    continue;
                }
                let eligible = match positive {
                    Some(flow) => !arc.reverse && flow[arc.link],
                    None => self.links[arc.link].flow == arc.reverse,
                };
                if eligible {
                    parent[arc.target] = Some(arc);
                    if arc.target == self.target {
                        return Ok(parent);
                    }
                    queue.push_back(arc.target);
                }
            }
        }
        Ok(parent)
    }

    fn path<F: FnMut() -> Result<(), MapError>>(
        &self,
        parent: &[Option<Arc>],
        work: &mut Work<F>,
    ) -> Result<Vec<Arc>, MapError> {
        let mut path = Vec::new();
        let mut current = self.target;
        for _ in 0..self.adjacent.len() {
            work.charge()?;
            if current == self.source {
                path.reverse();
                return Ok(path);
            }
            let arc = parent[current].ok_or(MapError::InvalidRegion)?;
            let link = &self.links[arc.link];
            current = if arc.reverse { link.to } else { link.from };
            path.push(arc);
        }
        Err(MapError::InvalidRegion)
    }
}

fn edge(a: usize, b: usize) -> [usize; 2] {
    [a.min(b), a.max(b)]
}

/// Feasible disjoint paths give a lower bound. A separating set of equal
/// cardinality gives the upper bound. This check does not trust residual state.
fn verify<F: FnMut() -> Result<(), MapError>>(
    graph: &TerrainGraph,
    source: usize,
    target: usize,
    report: &RouteResilience,
    work: &mut Work<F>,
) -> Result<(), MapError> {
    let excluded = !graph.candidates[source] || !graph.candidates[target];
    if report.endpoint_excluded != excluded
        || report.paths.len() > MAX_PATHS
        || report.minimum_cut.len() != report.paths.len()
        // Bound each caller-supplied path before lexicographic comparisons.
        || report.paths.iter().any(|path| path.len() < 2 || path.len() > graph.adjacent.len())
        || report.minimum_cut.windows(2).any(|pair| pair[0] >= pair[1])
        || report.paths.windows(2).any(|pair| pair[0] >= pair[1])
        || report.candidate_tiles != graph.candidates.iter().filter(|&&v| v).count() as u32
        || report.candidate_edges != graph.edges
    {
        return Err(MapError::InvalidRegion);
    }
    if excluded {
        return if report.paths.is_empty() { Ok(()) } else { Err(MapError::InvalidRegion) };
    }
    let mut used_edges = BTreeSet::new();
    let mut used_tiles = BTreeSet::new();
    for path in &report.paths {
        work.charge()?;
        if path.len() < 2 || path.len() > graph.adjacent.len()
            || path.first() != Some(&source) || path.last() != Some(&target)
        {
            return Err(MapError::InvalidRegion);
        }
        let mut seen = BTreeSet::new();
        for (i, &tile) in path.iter().enumerate() {
            work.charge()?;
            if tile >= graph.adjacent.len() || !graph.candidates[tile] || !seen.insert(tile) {
                return Err(MapError::InvalidRegion);
            }
            if i > 0 {
                let previous = path[i - 1];
                if graph.adjacent[previous].binary_search(&tile).is_err()
                    || !used_edges.insert(edge(previous, tile))
                {
                    return Err(MapError::InvalidRegion);
                }
            }
            if report.disjointness == Disjointness::InternalTiles
                && tile != source && tile != target && !used_tiles.insert(tile)
            {
                return Err(MapError::InvalidRegion);
            }
        }
    }
    let mut removed_tiles = BTreeSet::new();
    let mut removed_edges = BTreeSet::new();
    for cut in &report.minimum_cut {
        work.charge()?;
        match *cut {
            CutElement::Tile(tile) => {
                if report.disjointness != Disjointness::InternalTiles
                    || tile == source || tile == target
                    || tile >= graph.adjacent.len() || !graph.candidates[tile]
                {
                    return Err(MapError::InvalidRegion);
                }
                removed_tiles.insert(tile);
            }
            CutElement::Edge([a, b]) => {
                if a >= b || b >= graph.adjacent.len()
                    || graph.adjacent[a].binary_search(&b).is_err()
                    || (report.disjointness == Disjointness::InternalTiles
                        && [a, b] != edge(source, target))
                {
                    return Err(MapError::InvalidRegion);
                }
                removed_edges.insert([a, b]);
            }
        }
    }
    let mut seen = vec![false; graph.adjacent.len()];
    seen[source] = true;
    let mut queue = VecDeque::from([source]);
    while let Some(current) = queue.pop_front() {
        work.charge()?;
        if current == target {
            return Err(MapError::InvalidRegion);
        }
        for &next in &graph.adjacent[current] {
            work.charge()?;
            if !seen[next] && !removed_tiles.contains(&next)
                && !removed_edges.contains(&edge(current, next))
            {
                seen[next] = true;
                queue.push_back(next);
            }
        }
    }
    Ok(())
}

pub fn analyze(
    map: &MapRegion,
    start: [u32; 3],
    goal: [u32; 3],
    mode: Disjointness,
    maximum_work: u64,
) -> Result<RouteResilience, MapError> {
    analyze_with_check(map, start, goal, mode, maximum_work, || Ok(()))
}

pub fn analyze_with_check<F: FnMut() -> Result<(), MapError>>(
    map: &MapRegion,
    start: [u32; 3],
    goal: [u32; 3],
    mode: Disjointness,
    maximum_work: u64,
    mut check: F,
) -> Result<RouteResilience, MapError> {
    if maximum_work == 0 || maximum_work > MAX_ROUTE_WORK {
        return Err(MapError::BudgetExceeded);
    }
    check()?;
    let source = map.region.index(start).ok_or(MapError::OutsideRegion)?;
    let target = map.region.index(goal).ok_or(MapError::OutsideRegion)?;
    if source == target {
        return Err(MapError::InvalidRegion);
    }
    let mut work = Work { used: 0, maximum: maximum_work, check };
    let graph = TerrainGraph::build(map, &mut work)?;
    let mut report = RouteResilience {
        start, goal, disjointness: mode,
        endpoint_excluded: !graph.candidates[source] || !graph.candidates[target],
        paths: Vec::new(), minimum_cut: Vec::new(),
        candidate_tiles: graph.candidates.iter().filter(|&&v| v).count() as u32,
        candidate_edges: graph.edges, work_units: 0,
    };
    if !report.endpoint_excluded {
        let mut net = Network::build(&graph, source, target, mode, &mut work)?;
        let mut units = 0usize;
        loop {
            let parent = net.search(None, &mut work)?;
            if parent[net.target].is_none() {
                break;
            }
            if units == MAX_PATHS {
                return Err(MapError::InvalidRegion);
            }
            for arc in net.path(&parent, &mut work)? {
                work.charge()?;
                net.links[arc.link].flow = !arc.reverse;
            }
            units += 1;
        }
        // Opposite directed flows on one undirected edge are a circulation,
        // not two independent routes. Remove them before path extraction.
        for &(a, b) in &net.opposing {
            work.charge()?;
            if net.links[a].flow && net.links[b].flow {
                net.links[a].flow = false;
                net.links[b].flow = false;
            }
        }
        let residual = net.search(None, &mut work)?;
        if residual[net.target].is_some() {
            return Err(MapError::InvalidRegion);
        }
        let mut cuts = BTreeSet::new();
        for link in &net.links {
            work.charge()?;
            if residual[link.from].is_none() || residual[link.to].is_some() {
                continue;
            }
            let cut = match link.meaning {
                Meaning::Tile(tile) => CutElement::Tile(tile),
                Meaning::Step(a, b) if mode == Disjointness::Edges
                    || edge(a, b) == edge(source, target) => CutElement::Edge(edge(a, b)),
                Meaning::Step(a, b) => {
                    // A unit-capacity movement cut can be moved onto either
                    // internal endpoint. Packing + separator verification below
                    // proves that deduplication has not lowered its cardinality.
                    let tile = [a, b].into_iter()
                        .filter(|&tile| tile != source && tile != target)
                        .min().ok_or(MapError::InvalidRegion)?;
                    CutElement::Tile(tile)
                }
            };
            cuts.insert(cut);
        }
        report.minimum_cut = cuts.into_iter().collect();
        let mut remaining: Vec<_> = net.links.iter().map(|link| link.flow).collect();
        for _ in 0..units {
            let parent = net.search(Some(&remaining), &mut work)?;
            if parent[net.target].is_none() {
                return Err(MapError::InvalidRegion);
            }
            let mut path = vec![source];
            for arc in net.path(&parent, &mut work)? {
                work.charge()?;
                remaining[arc.link] = false;
                if let Meaning::Step(_, target) = net.links[arc.link].meaning {
                    path.push(target);
                }
            }
            report.paths.push(path);
        }
        report.paths.sort();
    }
    verify(&graph, source, target, &report, &mut work)?;
    (work.check)()?;
    report.work_units = work.used;
    Ok(report)
}

/// Recheck packing and separator against the supplied map. This validates
/// mathematical optimality in the declared model, not native game feasibility,
/// source custody, an execution transcript or the reported work counter.
pub fn verify_certificate(
    map: &MapRegion,
    report: &RouteResilience,
    maximum_work: u64,
) -> Result<(), MapError> {
    if maximum_work == 0 || maximum_work > MAX_ROUTE_WORK {
        return Err(MapError::BudgetExceeded);
    }
    let source = map.region.index(report.start).ok_or(MapError::OutsideRegion)?;
    let target = map.region.index(report.goal).ok_or(MapError::OutsideRegion)?;
    if source == target {
        return Err(MapError::InvalidRegion);
    }
    let mut work = Work { used: 0, maximum: maximum_work, check: || Ok(()) };
    let graph = TerrainGraph::build(map, &mut work)?;
    verify(&graph, source, target, report, &mut work)
}
