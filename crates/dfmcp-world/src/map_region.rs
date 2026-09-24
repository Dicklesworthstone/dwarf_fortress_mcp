#![forbid(unsafe_code)]

//! Read-only cognition over an already-authorized, bounded terrain observation.
//! A candidate route is not a DF unit path, safety proof, or global reachability claim.

use std::collections::VecDeque;

pub const MAX_MAP_TILES: usize = 16_384;
pub const MAX_MAP_SIDE: u32 = 128;
pub const MAX_ROUTE_WORK: u64 = 1_000_000;
pub const ROUTE_POLICY: &str = "observed-dry-cardinal-floor-stairs/1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub origin: [u32; 3],
    pub size: [u32; 3],
}

impl Region {
    pub fn volume(self) -> Result<usize, MapError> {
        let mut volume = 1usize;
        for axis in 0..3 {
            if self.size[axis] == 0
                || self.size[axis] > MAX_MAP_SIDE
                || self.origin[axis]
                    .checked_add(self.size[axis])
                    .is_none_or(|end| end > 32_768)
            {
                return Err(MapError::InvalidRegion);
            }
            volume = volume
                .checked_mul(self.size[axis] as usize)
                .ok_or(MapError::BudgetExceeded)?;
        }
        if volume > MAX_MAP_TILES {
            return Err(MapError::BudgetExceeded);
        }
        Ok(volume)
    }

    pub fn index(self, position: [u32; 3]) -> Option<usize> {
        self.volume().ok()?;
        let mut relative = [0usize; 3];
        for axis in 0..3 {
            let value = position[axis].checked_sub(self.origin[axis])?;
            if value >= self.size[axis] {
                return None;
            }
            relative[axis] = value as usize;
        }
        Some(
            (relative[2] * self.size[1] as usize + relative[1]) * self.size[0] as usize
                + relative[0],
        )
    }

    pub fn position(self, index: usize) -> Option<[u32; 3]> {
        if index >= self.volume().ok()? {
            return None;
        }
        let x = self.size[0] as usize;
        let y = self.size[1] as usize;
        Some([
            self.origin[0] + (index % x) as u32,
            self.origin[1] + ((index / x) % y) as u32,
            self.origin[2] + (index / (x * y)) as u32,
        ])
    }
}

/// Semantic tags, not native enum ordinals. Unknown shapes never become floors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Shape {
    Other = 0,
    Empty = 1,
    Wall = 2,
    Floor = 3,
    Ramp = 4,
    RampTop = 5,
    StairUp = 6,
    StairDown = 7,
    StairUpDown = 8,
}

impl Shape {
    pub fn from_tag(tag: u8) -> Result<Self, MapError> {
        match tag {
            0 => Ok(Self::Other),
            1 => Ok(Self::Empty),
            2 => Ok(Self::Wall),
            3 => Ok(Self::Floor),
            4 => Ok(Self::Ramp),
            5 => Ok(Self::RampTop),
            6 => Ok(Self::StairUp),
            7 => Ok(Self::StairDown),
            8 => Ok(Self::StairUpDown),
            _ => Err(MapError::InvalidTile),
        }
    }
    pub const fn name(self) -> &'static str {
        match self {
            Self::Other => "other",
            Self::Empty => "empty",
            Self::Wall => "wall",
            Self::Floor => "floor",
            Self::Ramp => "ramp",
            Self::RampTop => "ramp_top",
            Self::StairUp => "stair_up",
            Self::StairDown => "stair_down",
            Self::StairUpDown => "stair_up_down",
        }
    }
    const fn supported(self) -> bool {
        matches!(
            self,
            Self::Floor | Self::StairUp | Self::StairDown | Self::StairUpDown
        )
    }
    const fn up(self) -> bool {
        matches!(self, Self::StairUp | Self::StairUpDown)
    }
    const fn down(self) -> bool {
        matches!(self, Self::StairDown | Self::StairUpDown)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tile {
    pub native_tiletype: u32,
    pub shape: Shape,
    pub liquid_depth: u8,
    pub magma: bool,
    pub traffic: u8,
    pub dig_designation: u8,
    pub building_occupancy: u8,
    /// bit 0: standing unit; bit 1: grounded unit. No unit identity is inferred.
    pub unit_occupancy: u8,
    pub walkable_region: u32,
    pub temperature_1: u16,
    pub temperature_2: u16,
}

impl Tile {
    pub fn validate(self) -> Result<(), MapError> {
        if self.native_tiletype > i32::MAX as u32
            || self.liquid_depth > 7
            || self.traffic > 3
            || self.dig_designation > 7
            || self.building_occupancy > 7
            || self.unit_occupancy > 3
        {
            return Err(MapError::InvalidTile);
        }
        Ok(())
    }
    pub const fn candidate(self) -> bool {
        self.shape.supported()
            && self.liquid_depth == 0
            && self.building_occupancy == 0
            && self.unit_occupancy == 0
            && self.walkable_region != 0
    }
}

/// Hidden cells carry no terrain/occupancy attributes, even in the wire payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell {
    Unallocated,
    Hidden,
    Visible(Tile),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapRegion {
    pub region: Region,
    pub cells: Vec<Cell>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapError {
    InvalidRegion,
    InvalidTile,
    OutsideRegion,
    BudgetExceeded,
}

impl MapRegion {
    pub fn validate(&self) -> Result<(), MapError> {
        if self.cells.len() != self.region.volume()? {
            return Err(MapError::InvalidRegion);
        }
        for cell in &self.cells {
            if let Cell::Visible(tile) = cell {
                tile.validate()?;
            }
        }
        Ok(())
    }

    pub fn candidate(&self, index: usize) -> bool {
        matches!(self.cells.get(index), Some(Cell::Visible(tile)) if tile.candidate())
    }

    /// Six-neighbor, unit-weight BFS. Stable flattened tile index breaks ties.
    /// Stair travel requires complementary shapes on both observed endpoints.
    /// No ramps, diagonals, doors, swimming, flight, digging, or hidden tiles.
    pub fn route(
        &self,
        start: [u32; 3],
        goal: [u32; 3],
        maximum_work: u64,
    ) -> Result<Route, MapError> {
        self.validate()?;
        if maximum_work == 0 || maximum_work > MAX_ROUTE_WORK {
            return Err(MapError::BudgetExceeded);
        }
        let source = self.region.index(start).ok_or(MapError::OutsideRegion)?;
        let target = self.region.index(goal).ok_or(MapError::OutsideRegion)?;
        let mut result = Route {
            path: Vec::new(),
            visited_tiles: 0,
            work_units: 0,
            touched_region_boundary: false,
            endpoint_excluded: !self.candidate(source) || !self.candidate(target),
        };
        if result.endpoint_excluded {
            return Ok(result);
        }
        let mut parents = vec![usize::MAX; self.cells.len()];
        parents[source] = source;
        let mut queue = VecDeque::from([source]);
        while let Some(current) = queue.pop_front() {
            result.charge(maximum_work)?;
            result.visited_tiles += 1;
            let position = self
                .region
                .position(current)
                .ok_or(MapError::InvalidRegion)?;
            for (axis, coordinate) in position.iter().enumerate() {
                let r = *coordinate - self.region.origin[axis];
                if r == 0 || r + 1 == self.region.size[axis] {
                    result.touched_region_boundary = true;
                }
            }
            if current == target {
                let mut node = target;
                for _ in 0..self.cells.len() {
                    result
                        .path
                        .push(self.region.position(node).ok_or(MapError::InvalidRegion)?);
                    if node == source {
                        result.path.reverse();
                        return Ok(result);
                    }
                    node = parents[node];
                }
                return Err(MapError::InvalidRegion);
            }
            let mut neighbors = Vec::with_capacity(6);
            for axis in 0..3 {
                for increase in [false, true] {
                    let mut next = position;
                    let value = if increase {
                        next[axis].checked_add(1)
                    } else {
                        next[axis].checked_sub(1)
                    };
                    if let Some(value) = value {
                        next[axis] = value;
                        if let Some(index) = self.region.index(next) {
                            neighbors.push((index, axis, increase));
                        }
                    }
                }
            }
            neighbors.sort_unstable_by_key(|entry| entry.0);
            for (next, axis, increase) in neighbors {
                result.charge(maximum_work)?;
                if parents[next] != usize::MAX || !self.candidate(next) {
                    continue;
                }
                if axis == 2 {
                    let (Cell::Visible(a), Cell::Visible(b)) =
                        (self.cells[current], self.cells[next])
                    else {
                        continue;
                    };
                    if !(if increase {
                        a.shape.up() && b.shape.down()
                    } else {
                        a.shape.down() && b.shape.up()
                    }) {
                        continue;
                    }
                }
                parents[next] = current;
                queue.push_back(next);
            }
        }
        Ok(result)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    pub path: Vec<[u32; 3]>,
    pub visited_tiles: u32,
    pub work_units: u64,
    pub touched_region_boundary: bool,
    pub endpoint_excluded: bool,
}
impl Route {
    fn charge(&mut self, maximum: u64) -> Result<(), MapError> {
        self.work_units += 1;
        if self.work_units > maximum {
            return Err(MapError::BudgetExceeded);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn floor() -> Cell {
        Cell::Visible(Tile {
            native_tiletype: 1,
            shape: Shape::Floor,
            liquid_depth: 0,
            magma: false,
            traffic: 0,
            dig_designation: 0,
            building_occupancy: 0,
            unit_occupancy: 0,
            walkable_region: 1,
            temperature_1: 10015,
            temperature_2: 10015,
        })
    }
    fn grid(size: [u32; 3]) -> MapRegion {
        let region = Region {
            origin: [0; 3],
            size,
        };
        MapRegion {
            region,
            cells: vec![floor(); region.volume().unwrap_or(0)],
        }
    }
    #[test]
    fn stable_shortest_route_and_zero_distance() -> Result<(), MapError> {
        let map = grid([3, 3, 1]);
        let path = map.route([0, 0, 0], [2, 2, 0], 1000)?;
        assert_eq!(
            path.path,
            vec![[0, 0, 0], [1, 0, 0], [2, 0, 0], [2, 1, 0], [2, 2, 0]]
        );
        assert_eq!(map.route([1, 1, 0], [1, 1, 0], 10)?.path, vec![[1, 1, 0]]);
        Ok(())
    }
    #[test]
    fn hidden_and_unallocated_cannot_complete_a_corridor() -> Result<(), MapError> {
        for cell in [Cell::Hidden, Cell::Unallocated] {
            let mut map = grid([3, 1, 1]);
            map.cells[1] = cell;
            assert!(map.route([0, 0, 0], [2, 0, 0], 100)?.path.is_empty());
        }
        Ok(())
    }
    #[test]
    fn vertical_edges_require_two_complementary_stairs() -> Result<(), MapError> {
        let mut map = grid([1, 1, 2]);
        assert!(map.route([0, 0, 0], [0, 0, 1], 100)?.path.is_empty());
        if let Cell::Visible(tile) = &mut map.cells[0] {
            tile.shape = Shape::StairUp;
        }
        assert!(map.route([0, 0, 0], [0, 0, 1], 100)?.path.is_empty());
        if let Cell::Visible(tile) = &mut map.cells[1] {
            tile.shape = Shape::StairDown;
        }
        assert_eq!(map.route([0, 0, 0], [0, 0, 1], 100)?.path.len(), 2);
        assert_eq!(map.route([0, 0, 1], [0, 0, 0], 100)?.path.len(), 2);
        Ok(())
    }
    #[test]
    fn liquids_occupancy_unknown_walkability_and_ramps_are_excluded() -> Result<(), MapError> {
        for which in 0..5 {
            let mut map = grid([2, 1, 1]);
            if let Cell::Visible(t) = &mut map.cells[1] {
                match which {
                    0 => t.liquid_depth = 1,
                    1 => t.building_occupancy = 1,
                    2 => t.unit_occupancy = 2,
                    3 => t.walkable_region = 0,
                    _ => t.shape = Shape::Ramp,
                }
            }
            assert!(map.route([0, 0, 0], [1, 0, 0], 100)?.endpoint_excluded);
        }
        Ok(())
    }
    #[test]
    fn malformed_shapes_and_work_exhaustion_are_errors_not_negative_paths() {
        let map = grid([4, 4, 1]);
        assert_eq!(
            map.route([0, 0, 0], [3, 3, 0], 1),
            Err(MapError::BudgetExceeded)
        );
        assert_eq!(
            map.route([0, 0, 0], [4, 3, 0], 100),
            Err(MapError::OutsideRegion)
        );
        assert!(
            Region {
                origin: [u32::MAX, 0, 0],
                size: [1, 1, 1]
            }
            .volume()
            .is_err()
        );
        assert!(
            Region {
                origin: [0; 3],
                size: [128; 3]
            }
            .volume()
            .is_err()
        );
        assert!(Shape::from_tag(9).is_err());
    }
    #[test]
    fn exhaustive_three_by_three_obstacles_match_distance_relaxation() -> Result<(), MapError> {
        for mask in 0..512u32 {
            let mut map = grid([3, 3, 1]);
            for i in 0..9 {
                if mask & (1 << i) != 0 {
                    map.cells[i] = Cell::Hidden;
                }
            }
            let mut d = [99usize; 9];
            if map.candidate(0) {
                d[0] = 0;
            }
            for _ in 0..9 {
                for a in 0..9 {
                    for b in 0..9 {
                        let ax = a % 3;
                        let ay = a / 3;
                        let bx = b % 3;
                        let by = b / 3;
                        if map.candidate(a)
                            && map.candidate(b)
                            && ax.abs_diff(bx) + ay.abs_diff(by) == 1
                        {
                            d[b] = d[b].min(d[a] + 1);
                        }
                    }
                }
            }
            let result = map.route([0, 0, 0], [2, 2, 0], 1000)?;
            assert_eq!(
                result.path.len().checked_sub(1),
                if d[8] < 99 { Some(d[8]) } else { None },
                "mask={mask}"
            );
        }
        Ok(())
    }
}
