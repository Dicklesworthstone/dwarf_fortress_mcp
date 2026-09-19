#![forbid(unsafe_code)]

//! Connected areas and single-point failures in the existing observed route
//! model. A cut in this graph is NOT a native path, evacuation or safety proof.
//! Iterative low-link DFS is bounded even on a 16,384-tile single corridor.
use crate::map_region::{Cell, MapError, MapRegion, Shape, MAX_ROUTE_WORK};

pub const CONNECTIVITY_POLICY: &str = "observed-route-components-lowlink/1";
const NONE: usize = usize::MAX;
const DEGREE: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Classification {
    Candidate, Hidden, Unallocated, UnsupportedShape, Liquid, BuildingOccupied,
    UnitOccupied, WalkabilityUnestablished,
}
impl Classification {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Candidate => "candidate", Self::Hidden => "hidden",
            Self::Unallocated => "unallocated", Self::UnsupportedShape => "unsupported_shape",
            Self::Liquid => "liquid", Self::BuildingOccupied => "building_occupied",
            Self::UnitOccupied => "unit_occupied", Self::WalkabilityUnestablished => "walkability_unestablished",
        }
    }
}
/// Ordered exclusion reasons, using exactly Tile::candidate's current policy.
/// Hidden/unallocated cells have no backing attributes to inspect or reveal.
pub fn classify(cell: Cell) -> Classification {
    match cell {
        Cell::Hidden => Classification::Hidden,
        Cell::Unallocated => Classification::Unallocated,
        Cell::Visible(tile) if tile.candidate() => Classification::Candidate,
        Cell::Visible(tile) => {
            if !matches!(tile.shape, Shape::Floor | Shape::StairUp | Shape::StairDown | Shape::StairUpDown) {
                Classification::UnsupportedShape
            } else if tile.liquid_depth != 0 { Classification::Liquid }
            else if tile.building_occupancy != 0 { Classification::BuildingOccupied }
            else if tile.unit_occupancy != 0 { Classification::UnitOccupied }
            else { Classification::WalkabilityUnestablished }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Component {
    /// Least flattened tile index in this component, not a durable entity ID.
    pub representative: usize,
    pub tiles: u32,
    pub edges: u32,
    pub min: [u32; 3],
    pub max: [u32; 3],
    pub touches_region_boundary: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bottleneck {
    pub tile: usize,
    pub component: usize,
    /// Sizes after deleting this vertex, descending. The deleted tile is absent.
    pub partition_sizes: Vec<u32>,
    /// Unordered pairs connected before deletion but separated afterwards.
    pub separated_tile_pairs: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bridge {
    /// Canonical ascending pair of flattened endpoint indices.
    pub endpoints: [usize; 2],
    pub component: usize,
    /// Size of the side containing each corresponding endpoint after edge removal.
    pub side_sizes: [u32; 2],
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connectivity {
    pub components: Vec<Component>,
    pub bottlenecks: Vec<Bottleneck>,
    pub bridges: Vec<Bridge>,
    /// Component ordinal for each candidate; excluded cells have no component.
    pub component_of: Vec<Option<usize>>,
    pub candidate_tiles: u32,
    pub edges: u32,
    /// Structural work units, not CPU instructions or an elapsed-time estimate.
    pub work_units: u64,
}

struct Work<F> { used: u64, maximum: u64, check: F }
impl<F: FnMut() -> Result<(), MapError>> Work<F> {
    fn charge(&mut self) -> Result<(), MapError> {
        if self.used >= self.maximum { return Err(MapError::BudgetExceeded); }
        self.used += 1;
        if self.used == 1 || self.used % 128 == 0 { (self.check)()?; }
        Ok(())
    }
}

/// Candidate adjacency is the same six-neighbor, dry floor/complementary-stair
/// graph as MapRegion::route. Differential tests compare it with that public
/// route oracle rather than assuming two independently edited policies agree.
fn neighbors<F: FnMut() -> Result<(), MapError>>(map: &MapRegion, index: usize,
    work: &mut Work<F>) -> Result<[usize; DEGREE], MapError> {
    let mut out = [NONE; DEGREE];
    if !map.candidate(index) { return Ok(out); }
    let position = map.region.position(index).ok_or(MapError::InvalidRegion)?;
    let mut length = 0;
    for axis in 0..3 {
        for increase in [false, true] {
            work.charge()?;
            let coordinate = if increase { position[axis].checked_add(1) } else { position[axis].checked_sub(1) };
            let Some(coordinate) = coordinate else { continue; };
            let mut next = position; next[axis] = coordinate;
            let Some(target) = map.region.index(next) else { continue; };
            if !map.candidate(target) { continue; }
            if axis == 2 {
                let (Cell::Visible(a), Cell::Visible(b)) = (map.cells[index], map.cells[target]) else { continue; };
                let up = |s| matches!(s, Shape::StairUp | Shape::StairUpDown);
                let down = |s| matches!(s, Shape::StairDown | Shape::StairUpDown);
                if !(if increase { up(a.shape) && down(b.shape) } else { down(a.shape) && up(b.shape) }) { continue; }
            }
            out[length] = target; length += 1;
        }
    }
    out[..length].sort_unstable();
    Ok(out)
}

pub fn analyze(map: &MapRegion, maximum_work: u64) -> Result<Connectivity, MapError> {
    analyze_with_check(map, maximum_work, || Ok(()))
}

/// The caller may inject a cooperative deadline check; failure returns no partial
/// analysis. No recursion, I/O, mutable map, cached generation or detached work.
pub fn analyze_with_check<F: FnMut() -> Result<(), MapError>>(map: &MapRegion,
    maximum_work: u64, mut check: F) -> Result<Connectivity, MapError> {
    if maximum_work == 0 || maximum_work > MAX_ROUTE_WORK { return Err(MapError::BudgetExceeded); }
    check()?;
    map.validate()?;
    let n = map.cells.len();
    let mut work = Work { used: 0, maximum: maximum_work, check };
    let mut adjacent = Vec::with_capacity(n);
    let mut candidate_tiles = 0u32;
    let mut arcs = 0u32;
    for index in 0..n {
        work.charge()?;
        let row = neighbors(map, index, &mut work)?;
        candidate_tiles += u32::from(map.candidate(index));
        arcs += row.iter().filter(|&&v| v != NONE).count() as u32;
        adjacent.push(row);
    }
    let mut result = Connectivity { components: Vec::new(), bottlenecks: Vec::new(), bridges: Vec::new(),
        component_of: vec![None; n], candidate_tiles, edges: arcs / 2, work_units: 0 };
    let mut discovered = vec![0usize; n];
    let mut low = vec![0usize; n];
    let mut parent = vec![NONE; n];
    let mut subtree = vec![0u32; n];
    let mut split_sizes = vec![[0u32; DEGREE]; n];
    let mut split_count = vec![0usize; n];
    let mut bridge_child = vec![false; n];
    let mut timer = 0usize;
    for root in 0..n {
        work.charge()?;
        if !map.candidate(root) || discovered[root] != 0 { continue; }
        let component = result.components.len();
        let position = map.region.position(root).ok_or(MapError::InvalidRegion)?;
        result.components.push(Component { representative: root, tiles: 0, edges: 0,
            min: position, max: position, touches_region_boundary: false });
        // Each frame holds the next neighbor offset. A vertex is discovered and
        // pushed once, and each geometric adjacency is visited at most once.
        let mut stack = vec![(root, 0usize)];
        timer += 1; discovered[root] = timer; low[root] = timer; subtree[root] = 1;
        result.component_of[root] = Some(component);
        while let Some((node, next_edge)) = stack.last().copied() {
            work.charge()?;
            if next_edge < DEGREE && adjacent[node][next_edge] != NONE {
                let target = adjacent[node][next_edge];
                let frame = stack.last_mut().ok_or(MapError::InvalidRegion)?;
                frame.1 += 1;
                if discovered[target] == 0 {
                    timer += 1; discovered[target] = timer; low[target] = timer;
                    subtree[target] = 1; parent[target] = node;
                    result.component_of[target] = Some(component);
                    stack.push((target, 0));
                } else if target != parent[node] { low[node] = low[node].min(discovered[target]); }
            } else {
                stack.pop();
                let p = parent[node];
                if p != NONE {
                    subtree[p] += subtree[node]; low[p] = low[p].min(low[node]);
                    if low[node] >= discovered[p] {
                        let next = split_count[p];
                        if next >= DEGREE { return Err(MapError::InvalidRegion); }
                        split_sizes[p][next] = subtree[node]; split_count[p] += 1;
                    }
                    bridge_child[node] = low[node] > discovered[p];
                }
            }
        }
        result.components[component].tiles = subtree[root];
    }
    for node in 0..n {
        work.charge()?;
        let Some(component) = result.component_of[node] else { continue; };
        let position = map.region.position(node).ok_or(MapError::InvalidRegion)?;
        let c = &mut result.components[component];
        c.edges += adjacent[node].iter().filter(|&&target| target != NONE && node < target).count() as u32;
        for (axis, coordinate) in position.iter().copied().enumerate() {
            c.min[axis] = c.min[axis].min(coordinate); c.max[axis] = c.max[axis].max(coordinate);
            let relative = coordinate - map.region.origin[axis];
            c.touches_region_boundary |= relative == 0 || relative + 1 == map.region.size[axis];
        }
        let mut parts = split_sizes[node][..split_count[node]].to_vec();
        let separated: u32 = parts.iter().sum();
        let remainder = c.tiles.checked_sub(1).and_then(|v| v.checked_sub(separated))
            .ok_or(MapError::InvalidRegion)?;
        if remainder > 0 { parts.push(remainder); }
        if parts.len() > 1 {
            parts.sort_unstable_by(|a, b| b.cmp(a));
            let mut pairs = 0u64; let mut prior = 0u64;
            for &size in &parts { pairs += prior * u64::from(size); prior += u64::from(size); }
            result.bottlenecks.push(Bottleneck { tile: node, component,
                partition_sizes: parts, separated_tile_pairs: pairs });
        }
        if bridge_child[node] {
            let p = parent[node]; let sizes = [subtree[node], c.tiles - subtree[node]];
            result.bridges.push(if node < p { Bridge { endpoints: [node, p], component, side_sizes: sizes } }
                else { Bridge { endpoints: [p, node], component, side_sizes: [sizes[1], sizes[0]] } });
        }
    }
    result.bridges.sort_unstable_by_key(|edge| edge.endpoints);
    (work.check)()?;
    result.work_units = work.used;
    Ok(result)
}

#[cfg(test)]
#[path = "map_connectivity_tests.rs"]
mod tests;
