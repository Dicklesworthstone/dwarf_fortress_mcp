#![forbid(unsafe_code)]

//! One bounded flood for many inventory targets in the observed route model.
//! Unvisited means no candidate in this projection, never in-game inaccessibility.

use std::collections::{BTreeSet, VecDeque};
use crate::map_region::{Cell, MapError, MapRegion, Region, Shape, MAX_ROUTE_WORK};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reachability {
    region: Region,
    distance: Vec<Option<u32>>,
    parent: Vec<usize>,
    pub visited_tiles: u32,
    pub work_units: u64,
    pub touched_region_boundary: bool,
}
impl Reachability {
    /// All roots must be observed candidate tiles. Refuse excluded roots instead
    /// of turning an invalid starting point into a misleading supply shortage.
    pub fn compute(map: &MapRegion, roots: &[[u32; 3]], maximum_work: u64) -> Result<Self, MapError> {
        map.validate()?;
        if roots.is_empty() || roots.len() > 128 || maximum_work == 0 || maximum_work > MAX_ROUTE_WORK {
            return Err(MapError::BudgetExceeded);
        }
        let mut sources = BTreeSet::new();
        for root in roots {
            let index = map.region.index(*root).ok_or(MapError::OutsideRegion)?;
            if !map.candidate(index) { return Err(MapError::InvalidTile); }
            sources.insert(index);
        }
        let mut result = Self { region: map.region, distance: vec![None; map.cells.len()],
            parent: vec![usize::MAX; map.cells.len()], visited_tiles: 0,
            work_units: 0, touched_region_boundary: false };
        result.charge(map.cells.len() as u64, maximum_work)?;
        let mut queue = VecDeque::new();
        for source in sources {
            result.distance[source] = Some(0);
            result.parent[source] = source;
            queue.push_back(source);
        }
        while let Some(current) = queue.pop_front() {
            result.charge(1, maximum_work)?;
            result.visited_tiles += 1;
            let position = map.region.position(current).ok_or(MapError::InvalidRegion)?;
            let depth = result.distance[current].ok_or(MapError::InvalidRegion)?;
            let mut neighbors = Vec::with_capacity(6);
            for axis in 0..3 {
                let relative = position[axis] - map.region.origin[axis];
                if relative == 0 || relative + 1 == map.region.size[axis] {
                    result.touched_region_boundary = true;
                }
                for up in [false, true] {
                    let value = if up { position[axis].checked_add(1) } else { position[axis].checked_sub(1) };
                    if let Some(value) = value {
                        let mut next = position;
                        next[axis] = value;
                        if let Some(index) = map.region.index(next) { neighbors.push((index, axis, up)); }
                    }
                }
            }
            neighbors.sort_unstable_by_key(|entry| entry.0);
            for (next, axis, up) in neighbors {
                result.charge(1, maximum_work)?;
                if result.distance[next].is_some() || !map.candidate(next) { continue; }
                if axis == 2 {
                    let (Cell::Visible(a), Cell::Visible(b)) = (map.cells[current], map.cells[next]) else { continue; };
                    let ascends = |shape| matches!(shape, Shape::StairUp | Shape::StairUpDown);
                    let descends = |shape| matches!(shape, Shape::StairDown | Shape::StairUpDown);
                    if !(if up { ascends(a.shape) && descends(b.shape) } else { descends(a.shape) && ascends(b.shape) }) { continue; }
                }
                result.distance[next] = Some(depth.checked_add(1).ok_or(MapError::BudgetExceeded)?);
                result.parent[next] = current;
                queue.push_back(next);
            }
        }
        Ok(result)
    }
    fn charge(&mut self, amount: u64, maximum: u64) -> Result<(), MapError> {
        self.work_units = self.work_units.checked_add(amount).ok_or(MapError::BudgetExceeded)?;
        if self.work_units > maximum { return Err(MapError::BudgetExceeded); }
        Ok(())
    }
    pub fn distance_to(&self, target: [u32; 3]) -> Option<u32> {
        self.region.index(target).and_then(|i| self.distance[i])
    }
    pub fn path_to(&self, target: [u32; 3]) -> Result<Option<Vec<[u32; 3]>>, MapError> {
        let mut node = self.region.index(target).ok_or(MapError::OutsideRegion)?;
        if self.distance[node].is_none() { return Ok(None); }
        let mut path = Vec::new();
        for _ in 0..self.parent.len() {
            path.push(self.region.position(node).ok_or(MapError::InvalidRegion)?);
            if self.parent[node] == node { path.reverse(); return Ok(Some(path)); }
            node = *self.parent.get(node).ok_or(MapError::InvalidRegion)?;
        }
        Err(MapError::InvalidRegion)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map_region::Tile;
    fn grid(size: [u32; 3]) -> Result<MapRegion, MapError> {
        let region = Region { origin: [0; 3], size };
        let tile = Cell::Visible(Tile { native_tiletype: 1, shape: Shape::Floor,
            liquid_depth: 0, magma: false, traffic: 0, dig_designation: 0,
            building_occupancy: 0, unit_occupancy: 0, walkable_region: 1,
            temperature_1: 10015, temperature_2: 10015 });
        Ok(MapRegion { region, cells: vec![tile; region.volume()?] })
    }
    #[test]
    fn every_three_by_three_obstacle_map_matches_existing_routes() -> Result<(), MapError> {
        for mask in 0..512u32 {
            let mut map = grid([3, 3, 1])?;
            for i in 0..9 { if mask & (1 << i) != 0 { map.cells[i] = Cell::Hidden; } }
            if !map.candidate(0) {
                assert_eq!(Reachability::compute(&map, &[[0; 3]], 1000), Err(MapError::InvalidTile));
                continue;
            }
            let field = Reachability::compute(&map, &[[0; 3]], 1000)?;
            for i in 0..9 {
                let target = map.region.position(i).ok_or(MapError::InvalidRegion)?;
                let route = map.route([0; 3], target, 1000)?;
                assert_eq!(field.distance_to(target), route.path.len().checked_sub(1).map(|n| n as u32));
                assert_eq!(field.path_to(target)?.unwrap_or_default(), route.path);
            }
        }
        Ok(())
    }
    #[test]
    fn normalized_multi_source_ties_and_stairs() -> Result<(), MapError> {
        let mut map = grid([3, 1, 2])?;
        if let Cell::Visible(tile) = &mut map.cells[1] { tile.shape = Shape::StairUp; }
        if let Cell::Visible(tile) = &mut map.cells[4] { tile.shape = Shape::StairDown; }
        let a = Reachability::compute(&map, &[[2, 0, 0], [0, 0, 0], [2, 0, 0]], 1000)?;
        let b = Reachability::compute(&map, &[[0, 0, 0], [2, 0, 0]], 1000)?;
        assert_eq!(a, b);
        assert_eq!(a.distance_to([2, 0, 1]), Some(3));
        assert_eq!(a.path_to([1, 0, 1])?, Some(vec![[0, 0, 0], [1, 0, 0], [1, 0, 1]]));
        Ok(())
    }
    #[test]
    fn invalid_start_and_exhausted_work_are_not_empty_supply() -> Result<(), MapError> {
        let mut map = grid([3, 1, 1])?;
        assert_eq!(Reachability::compute(&map, &[[0; 3]], 1), Err(MapError::BudgetExceeded));
        assert_eq!(Reachability::compute(&map, &[[3, 0, 0]], 100), Err(MapError::OutsideRegion));
        map.cells[1] = Cell::Unallocated;
        let field = Reachability::compute(&map, &[[0; 3]], 100)?;
        assert_eq!(field.distance_to([2, 0, 0]), None);
        assert_eq!(field.path_to([2, 0, 0])?, None);
        assert_eq!(field.distance_to([3, 0, 0]), None);
        Ok(())
    }
}
