//! Bounded access in the reference laboratory's registered movement models.
//!
//! This is not native unit pathfinding. Furnishing targets and every occupied
//! building footprint block traversal. Missing terrain can establish only a
//! potential route, and incomplete occupancy never establishes clear access.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use dfmcp_core::{DfmcpError, EntityId, ErrorCode, GameTick, MapCoord, MapCuboid, Result};

use crate::{
    EntityKind, EntityRecord, PredicateTruth, Value, WorldSnapshot, laboratory_fact_value,
    tile_codes,
};

pub const LABORATORY_ACCESS_POLICY: &str = "dfmcp.laboratory-room-access/1";
/// Opt-in movement policy with horizontal FLOOR/STAIR traversal and vertical
/// movement only between paired adjacent STAIR tiles. Generic laboratory stairs
/// are bidirectional in this model; native directional stairs are not inferred.
pub const LABORATORY_ACCESS_PAIRED_STAIRS_POLICY: &str =
    "dfmcp.laboratory-room-access-paired-stairs/1";
pub const MAX_LABORATORY_ACCESS_TILES: u64 = 1_024;
pub const MAX_LABORATORY_ACCESS_TARGETS: usize = 16;
pub const MAX_LABORATORY_ACCESS_ENTITIES: usize = 65_536;
/// Conservative work units include the complete occupancy census, bounded RLE
/// tile reads, rectangle rasterization and both cardinal graph traversals.
pub const MAX_PREDICATE_LABORATORY_ACCESS_WORK: u64 =
    16 * (32 * MAX_LABORATORY_ACCESS_ENTITIES as u64 + 320 * MAX_LABORATORY_ACCESS_TILES + 64 * 16);

fn unavailable(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::PreconditionsFailed, message)
}

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn bounded(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

pub(crate) fn validate_request(
    area: MapCuboid,
    entry: MapCoord,
    targets: &[MapCoord],
) -> Result<u64> {
    let tiles = crate::terrain::validate_region(area)?;
    if area.min.z != area.max.z {
        return Err(invalid(
            "laboratory room access requires a single-level area",
        ));
    }
    if tiles > MAX_LABORATORY_ACCESS_TILES
        || targets.is_empty()
        || targets.len() > MAX_LABORATORY_ACCESS_TARGETS
    {
        return Err(bounded(
            "laboratory room access requires at most 1024 tiles and 1..=16 targets",
        ));
    }
    if !area.contains(entry) || targets.iter().any(|target| !area.contains(*target)) {
        return Err(invalid(
            "laboratory room access entry and targets must lie inside its area",
        ));
    }
    if targets.iter().copied().collect::<BTreeSet<_>>().len() != targets.len() {
        return Err(invalid("laboratory room access targets must be distinct"));
    }
    Ok(tiles)
}

pub(crate) fn validate_paired_stairs_request(
    area: MapCuboid,
    entry: MapCoord,
    targets: &[MapCoord],
) -> Result<u64> {
    let tiles = crate::terrain::validate_region(area)?;
    if tiles > MAX_LABORATORY_ACCESS_TILES
        || targets.is_empty()
        || targets.len() > MAX_LABORATORY_ACCESS_TARGETS
    {
        return Err(bounded(
            "laboratory paired-stair room access requires at most 1024 tiles and 1..=16 targets",
        ));
    }
    if !area.contains(entry) || targets.iter().any(|target| !area.contains(*target)) {
        return Err(invalid(
            "laboratory paired-stair room access entry and targets must lie inside its area",
        ));
    }
    if targets.iter().copied().collect::<BTreeSet<_>>().len() != targets.len() {
        return Err(invalid(
            "laboratory paired-stair room access targets must be distinct",
        ));
    }
    Ok(tiles)
}

pub(crate) fn work_bound(area: MapCuboid, targets: usize, entities: usize) -> u64 {
    (entities as u64)
        .saturating_mul(32)
        .saturating_add(
            area.tile_count()
                .map_or(u64::MAX, |tiles| tiles)
                .saturating_mul(320),
        )
        .saturating_add((targets as u64).saturating_mul(64))
}

fn field<'a>(record: &'a EntityRecord, name: &str, tick: GameTick) -> Option<&'a Value> {
    record
        .fields
        .get(name)
        .and_then(|fact| laboratory_fact_value(fact, tick))
}

fn text<'a>(record: &'a EntityRecord, name: &str, tick: GameTick) -> Option<&'a str> {
    match field(record, name, tick) {
        Some(Value::Text(value)) => Some(value),
        _ => None,
    }
}

fn coord(record: &EntityRecord, name: &str, tick: GameTick) -> Option<MapCoord> {
    match field(record, name, tick) {
        Some(Value::Coord(value)) => Some(*value),
        _ => None,
    }
}

/// A released placement may no longer own its item, but its original building
/// identity and lifecycle must still be established before ignoring occupancy.
/// These are the furniture/1 footprint rules, not a new item-completion proof.
fn validate_placement(record: &EntityRecord, tick: GameTick) -> Result<()> {
    let position = coord(record, "position", tick)
        .ok_or_else(|| unavailable("furniture occupancy has no qualified position"))?;
    let progress = match field(record, "progress_ticks", tick) {
        Some(Value::U64(value)) => *value,
        _ => return Err(unavailable("furniture occupancy has no qualified progress")),
    };
    let stage = text(record, "construction_stage", tick);
    let valid_stage = match stage {
        Some("planned") => progress == 0,
        Some("under_construction" | "cancelled") => {
            progress < 500 && (stage != Some("under_construction") || progress > 0)
        }
        Some("complete") => progress == 500,
        _ => false,
    };
    if record.generation != 1
        || text(record, "placement_model", tick) != Some("furniture/1")
        || !matches!(
            text(record, "building_kind", tick),
            Some("furniture:Bed" | "furniture:Door" | "furniture:Table" | "furniture:Chair")
        )
        || !matches!(field(record, "source_item", tick), Some(Value::Entity(_)))
        || field(record, "source_item_generation", tick) != Some(&Value::U64(1))
        || coord(record, "footprint_min", tick) != Some(position)
        || coord(record, "footprint_max", tick) != Some(position)
        || field(record, "required_ticks", tick) != Some(&Value::U64(500))
        || !valid_stage
        || text(record, "attachment_state", tick)
            != Some(if stage == Some("cancelled") {
                "released"
            } else {
                "attached"
            })
    {
        return Err(unavailable(
            "furniture occupancy has no qualified original placement identity",
        ));
    }
    Ok(())
}

/// Complete qualified building census for the laboratory model. Canonical kind
/// names are used because Other("building") has the same canonical identity as
/// Building. A malformed or unqualified record anywhere refuses the census.
/// Generic cancelled buildings and valid cancelled/released furniture occupy
/// nothing; their historical records remain untouched.
pub fn laboratory_building_footprints(
    snapshot: &WorldSnapshot,
) -> Result<BTreeMap<EntityId, MapCuboid>> {
    if snapshot.graph.entities.len() > MAX_LABORATORY_ACCESS_ENTITIES {
        return Err(bounded(
            "laboratory occupancy exceeds its 65536-entity inspection bound",
        ));
    }
    let mut footprints = BTreeMap::new();
    for (id, record) in &snapshot.graph.entities {
        if *id != record.id {
            return Err(unavailable(
                "laboratory occupancy has mismatched canonical entity identities",
            ));
        }
        if record.kind.as_str() != EntityKind::Building.as_str() {
            continue;
        }
        if [
            "placement_model",
            "source_item",
            "source_item_generation",
            "attachment_state",
        ]
        .iter()
        .any(|name| record.fields.contains_key(*name))
        {
            validate_placement(record, snapshot.tick)?;
        }
        match text(record, "construction_stage", snapshot.tick) {
            Some("cancelled") => continue,
            Some("planned" | "under_construction" | "complete") => {}
            _ => return Err(unavailable("building occupancy has no qualified lifecycle")),
        }
        let (Some(min), Some(max)) = (
            coord(record, "footprint_min", snapshot.tick),
            coord(record, "footprint_max", snapshot.tick),
        ) else {
            return Err(unavailable(
                "building occupancy has no qualified complete footprint",
            ));
        };
        let area = MapCuboid::new(min, max)
            .map_err(|_| unavailable("building occupancy has an invalid footprint"))?;
        footprints.insert(*id, area);
    }
    Ok(footprints)
}

struct Grid {
    area: MapCuboid,
    width: usize,
    height: usize,
}

impl Grid {
    fn index(&self, at: MapCoord) -> Option<usize> {
        if !self.area.contains(at) {
            return None;
        }
        let x = usize::try_from(i64::from(at.x) - i64::from(self.area.min.x)).ok()?;
        let y = usize::try_from(i64::from(at.y) - i64::from(self.area.min.y)).ok()?;
        Some(y * self.width + x)
    }

    fn neighbors(&self, index: usize) -> impl Iterator<Item = usize> {
        let x = index % self.width;
        let y = index / self.width;
        [
            (x > 0).then(|| index - 1),
            (x + 1 < self.width).then_some(index + 1),
            (y > 0).then(|| index - self.width),
            (y + 1 < self.height).then_some(index + self.width),
        ]
        .into_iter()
        .flatten()
    }

    fn reached(&self, open: &[bool], entry: usize) -> Vec<bool> {
        let mut reached = vec![false; open.len()];
        let mut pending = VecDeque::new();
        if open[entry] {
            reached[entry] = true;
            pending.push_back(entry);
        }
        while let Some(current) = pending.pop_front() {
            for next in self.neighbors(current) {
                if open[next] && !reached[next] {
                    reached[next] = true;
                    pending.push_back(next);
                }
            }
        }
        reached
    }

    /// Difference rectangles avoid a buildings-times-area scan even when every
    /// bounded building footprint overlaps the complete requested region.
    fn occupied(&self, footprints: &BTreeMap<EntityId, MapCuboid>) -> Vec<bool> {
        let stride = self.width + 1;
        let mut difference = vec![0i64; stride * (self.height + 1)];
        for footprint in footprints.values() {
            if footprint.min.z > self.area.min.z || footprint.max.z < self.area.min.z {
                continue;
            }
            let min = MapCoord::new(
                footprint.min.x.max(self.area.min.x),
                footprint.min.y.max(self.area.min.y),
                self.area.min.z,
            );
            let max = MapCoord::new(
                footprint.max.x.min(self.area.max.x),
                footprint.max.y.min(self.area.max.y),
                self.area.min.z,
            );
            let (Some(start), Some(end)) = (self.index(min), self.index(max)) else {
                continue;
            };
            if min.x > max.x || min.y > max.y {
                continue;
            }
            let (left, top) = (start % self.width, start / self.width);
            let (right, bottom) = (end % self.width + 1, end / self.width + 1);
            difference[top * stride + left] += 1;
            difference[top * stride + right] -= 1;
            difference[bottom * stride + left] -= 1;
            difference[bottom * stride + right] += 1;
        }
        let mut occupied = vec![false; self.width * self.height];
        for y in 0..self.height {
            let mut row = 0;
            for x in 0..self.width {
                let index = y * stride + x;
                row += difference[index];
                difference[index] = row
                    + if y == 0 {
                        0
                    } else {
                        difference[index - stride]
                    };
                occupied[y * self.width + x] = difference[index] > 0;
            }
        }
        occupied
    }
}

fn bounded_tile_code(snapshot: &WorldSnapshot, at: MapCoord) -> Option<u32> {
    use crate::terrain::{TERRAIN_CHUNK_EDGE, TERRAIN_CHUNK_TILES, tile_slot};
    let slot = tile_slot(at);
    let chunk = snapshot.graph.chunks.get(&slot.chunk)?;
    if chunk.coord != slot.chunk
        || i32::from(chunk.width) != TERRAIN_CHUNK_EDGE
        || i32::from(chunk.height) != TERRAIN_CHUNK_EDGE
        || chunk.terrain_runs.len() > TERRAIN_CHUNK_TILES as usize
    {
        return None;
    }
    // Validate and select in one bounded pass. Zero-length runs must not make
    // an arbitrarily long malformed encoding look like a 256-tile chunk.
    let mut total = 0u32;
    let mut code = None;
    for run in &chunk.terrain_runs {
        let end = total.checked_add(run.length)?;
        if run.length == 0 || end > TERRAIN_CHUNK_TILES {
            return None;
        }
        if (total..end).contains(&slot.offset) {
            code = Some(run.tile_code);
        }
        total = end;
    }
    if total == TERRAIN_CHUNK_TILES {
        code
    } else {
        None
    }
}

/// Every furnishing needs a reachable cardinal approach from the selected
/// entry. True is a current laboratory FLOOR-model fact; False means no route
/// inside this exact area, not global inaccessibility. Unknown terrain supports
/// only potential paths. Invalid shapes/budgets are errors before allocations;
/// unavailable occupancy is Unknown rather than silently empty space.
pub fn laboratory_room_access_truth(
    snapshot: &WorldSnapshot,
    area: MapCuboid,
    entry: MapCoord,
    targets: &[MapCoord],
) -> Result<PredicateTruth> {
    let tiles = validate_request(area, entry, targets)?;
    let footprints = match laboratory_building_footprints(snapshot) {
        Ok(footprints) => footprints,
        Err(error) if error.code == ErrorCode::BudgetExceeded => return Err(error),
        Err(_) => return Ok(PredicateTruth::Unknown),
    };
    let grid = Grid {
        area,
        width: usize::try_from(i64::from(area.max.x) - i64::from(area.min.x) + 1)
            .map_err(|_| bounded("laboratory access width cannot be represented"))?,
        height: usize::try_from(i64::from(area.max.y) - i64::from(area.min.y) + 1)
            .map_err(|_| bounded("laboratory access height cannot be represented"))?,
    };
    let mut occupied = grid.occupied(&footprints);
    for target in targets {
        let index = grid
            .index(*target)
            .ok_or_else(|| invalid("access target left its area"))?;
        occupied[index] = true;
    }
    let mut clear = Vec::with_capacity(tiles as usize);
    let mut potential = Vec::with_capacity(tiles as usize);
    for (index, at) in crate::terrain::region_tiles(area).enumerate() {
        let terrain = bounded_tile_code(snapshot, at);
        clear.push(!occupied[index] && terrain == Some(tile_codes::FLOOR));
        potential.push(!occupied[index] && terrain.is_none_or(|code| code == tile_codes::FLOOR));
    }
    let entry = grid
        .index(entry)
        .ok_or_else(|| invalid("access entry left its area"))?;
    let clear = grid.reached(&clear, entry);
    let potential = grid.reached(&potential, entry);
    let mut result = PredicateTruth::True;
    for target in targets {
        let index = grid
            .index(*target)
            .ok_or_else(|| invalid("access target left its area"))?;
        if grid.neighbors(index).any(|next| clear[next]) {
            continue;
        }
        if !grid.neighbors(index).any(|next| potential[next]) {
            return Ok(PredicateTruth::False);
        }
        result = PredicateTruth::Unknown;
    }
    Ok(result)
}

/// Three-dimensional indexing stays separate from the original single-level
/// model so old predicates retain their exact FLOOR-only movement meaning.
struct PairedStairsGrid {
    area: MapCuboid,
    width: usize,
    height: usize,
    depth: usize,
}

impl PairedStairsGrid {
    fn new(area: MapCuboid) -> Result<Self> {
        let dimension = |min: i32, max: i32| {
            usize::try_from(i64::from(max) - i64::from(min) + 1)
                .map_err(|_| bounded("paired-stair access dimension cannot be represented"))
        };
        Ok(Self {
            area,
            width: dimension(area.min.x, area.max.x)?,
            height: dimension(area.min.y, area.max.y)?,
            depth: dimension(area.min.z, area.max.z)?,
        })
    }

    fn index(&self, at: MapCoord) -> Option<usize> {
        if !self.area.contains(at) {
            return None;
        }
        let x = usize::try_from(i64::from(at.x) - i64::from(self.area.min.x)).ok()?;
        let y = usize::try_from(i64::from(at.y) - i64::from(self.area.min.y)).ok()?;
        let z = usize::try_from(i64::from(at.z) - i64::from(self.area.min.z)).ok()?;
        Some((z * self.height + y) * self.width + x)
    }

    fn horizontal_neighbors(&self, index: usize) -> impl Iterator<Item = usize> {
        let x = index % self.width;
        let y = (index / self.width) % self.height;
        [
            (x > 0).then(|| index - 1),
            (x + 1 < self.width).then_some(index + 1),
            (y > 0).then(|| index - self.width),
            (y + 1 < self.height).then_some(index + self.width),
        ]
        .into_iter()
        .flatten()
    }

    fn reached(
        &self,
        terrain: &[Option<u32>],
        occupied: &[bool],
        entry: usize,
        potential: bool,
    ) -> Vec<bool> {
        let passable = |index: usize| {
            !occupied[index]
                && match terrain[index] {
                    Some(tile_codes::FLOOR | tile_codes::STAIR) => true,
                    None => potential,
                    _ => false,
                }
        };
        let stairs = |index: usize| {
            terrain[index] == Some(tile_codes::STAIR) || (potential && terrain[index].is_none())
        };
        let mut reached = vec![false; terrain.len()];
        let mut pending = VecDeque::new();
        if passable(entry) {
            reached[entry] = true;
            pending.push_back(entry);
        }
        let layer = self.width * self.height;
        while let Some(current) = pending.pop_front() {
            let z = current / layer;
            let vertical = [
                (z > 0).then(|| current - layer),
                (z + 1 < self.depth).then_some(current + layer),
            ];
            for next in self.horizontal_neighbors(current).chain(
                vertical
                    .into_iter()
                    .flatten()
                    .filter(|next| stairs(current) && stairs(*next)),
            ) {
                if !reached[next] && passable(next) {
                    reached[next] = true;
                    pending.push_back(next);
                }
            }
        }
        reached
    }

    /// A bounded three-dimensional difference array clips each footprint once.
    /// This avoids a buildings-times-levels scan when a thin region is tall.
    fn occupied(&self, footprints: &BTreeMap<EntityId, MapCuboid>) -> Vec<bool> {
        let stride = self.width + 1;
        let plane = stride * (self.height + 1);
        let mut difference = vec![0i64; plane * (self.depth + 1)];
        for footprint in footprints.values() {
            let min = MapCoord::new(
                footprint.min.x.max(self.area.min.x),
                footprint.min.y.max(self.area.min.y),
                footprint.min.z.max(self.area.min.z),
            );
            let max = MapCoord::new(
                footprint.max.x.min(self.area.max.x),
                footprint.max.y.min(self.area.max.y),
                footprint.max.z.min(self.area.max.z),
            );
            if min.x > max.x || min.y > max.y || min.z > max.z {
                continue;
            }
            let (Some(start), Some(end)) = (self.index(min), self.index(max)) else {
                continue;
            };
            let layer = self.width * self.height;
            let (left, top, front) = (
                start % self.width,
                (start / self.width) % self.height,
                start / layer,
            );
            let (right, bottom, back) = (
                end % self.width + 1,
                (end / self.width) % self.height + 1,
                end / layer + 1,
            );
            for (x, x_sign) in [(left, 1), (right, -1)] {
                for (y, y_sign) in [(top, 1), (bottom, -1)] {
                    for (z, z_sign) in [(front, 1), (back, -1)] {
                        difference[z * plane + y * stride + x] += x_sign * y_sign * z_sign;
                    }
                }
            }
        }
        for z in 0..=self.depth {
            for y in 0..=self.height {
                for x in 1..=self.width {
                    let index = z * plane + y * stride + x;
                    difference[index] += difference[index - 1];
                }
            }
        }
        for z in 0..=self.depth {
            for y in 1..=self.height {
                for x in 0..=self.width {
                    let index = z * plane + y * stride + x;
                    difference[index] += difference[index - stride];
                }
            }
        }
        for z in 1..=self.depth {
            for y in 0..=self.height {
                for x in 0..=self.width {
                    let index = z * plane + y * stride + x;
                    difference[index] += difference[index - plane];
                }
            }
        }
        let mut occupied = Vec::with_capacity(self.width * self.height * self.depth);
        for z in 0..self.depth {
            for y in 0..self.height {
                for x in 0..self.width {
                    occupied.push(difference[z * plane + y * stride + x] > 0);
                }
            }
        }
        occupied
    }
}

/// Current bounded access under the paired bidirectional laboratory stair model.
/// FLOOR and STAIR permit horizontal movement; changing levels requires STAIR
/// at both endpoints at the same (x,y). Every furnishing requires a horizontal
/// approach, and targets block traversal. Unknown terrain can establish only a
/// potential route. This never establishes native directional-stair pathfinding
/// or accessibility outside the selected region.
pub fn laboratory_room_access_paired_stairs_truth(
    snapshot: &WorldSnapshot,
    area: MapCuboid,
    entry: MapCoord,
    targets: &[MapCoord],
) -> Result<PredicateTruth> {
    let tiles = validate_paired_stairs_request(area, entry, targets)?;
    let footprints = match laboratory_building_footprints(snapshot) {
        Ok(footprints) => footprints,
        Err(error) if error.code == ErrorCode::BudgetExceeded => return Err(error),
        Err(_) => return Ok(PredicateTruth::Unknown),
    };
    let grid = PairedStairsGrid::new(area)?;
    let mut occupied = grid.occupied(&footprints);
    for target in targets {
        let index = grid
            .index(*target)
            .ok_or_else(|| invalid("paired-stair access target left its area"))?;
        occupied[index] = true;
    }
    let mut terrain = Vec::with_capacity(tiles as usize);
    for at in crate::terrain::region_tiles(area) {
        terrain.push(bounded_tile_code(snapshot, at).filter(|code| *code != tile_codes::UNKNOWN));
    }
    let entry = grid
        .index(entry)
        .ok_or_else(|| invalid("paired-stair access entry left its area"))?;
    let clear = grid.reached(&terrain, &occupied, entry, false);
    let potential = grid.reached(&terrain, &occupied, entry, true);
    let mut result = PredicateTruth::True;
    for target in targets {
        let index = grid
            .index(*target)
            .ok_or_else(|| invalid("paired-stair access target left its area"))?;
        if grid.horizontal_neighbors(index).any(|next| clear[next]) {
            continue;
        }
        if !grid.horizontal_neighbors(index).any(|next| potential[next]) {
            return Ok(PredicateTruth::False);
        }
        result = PredicateTruth::Unknown;
    }
    Ok(result)
}

#[cfg(test)]
#[path = "laboratory_access_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "laboratory_access_paired_stairs_tests.rs"]
mod paired_stairs_tests;
