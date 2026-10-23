//! Bounded entrance-connected room pursuits in the laboratory movement model.
//! Routing proposes exact mining masks; current access remains a terminal fact.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, IntentId, MapCoord, MapCuboid, Result};
use dfmcp_intent::blueprint::{BlueprintLayout, ExcavationRole, HazardAssessment};
use dfmcp_intent::{Action, BlueprintPlanner, Intent, RequestedAction, StaticPlanner};
use dfmcp_world::{
    EntityKind, Predicate, WorldSnapshot, laboratory_building_footprints, tile_codes,
};
use serde::Deserialize;
use serde_json::{Value as Json, json};

use super::room_provisioning::{ProvisionedRoomRequest, RoomAccessComposition};
use crate::lab_world::spatial_index;

const PLANNER: &str = "connected_room_v1";
const RESIDUAL_PLANNER: &str = "connected_room_residual_v1";
const ROUTING_POLICY: &str = "laboratory-room-mining-cardinal/1";
const MAX_REQUEST_BYTES: usize = 64 * 1_024;
const MAX_CELLS: u64 = 1_024;
const MAX_SOURCE_CHUNKS: usize = 4_096;
const MAX_WORK: u64 = 12_000_000;
const MAX_STEPS: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConnectedRoomRequest {
    room: ProvisionedRoomRequest,
    entry: MapCoord,
    area: MapCuboid,
}

pub(crate) fn is_connected_room(raw: &str) -> bool {
    if raw.len() > MAX_REQUEST_BYTES {
        return false;
    }
    #[derive(Deserialize)]
    struct Header {
        template: String,
    }
    serde_json::from_str::<Header>(raw).is_ok_and(|header| header.template == "connected_room")
}

impl ConnectedRoomRequest {
    pub(crate) fn parse_current(raw: &str) -> Result<Self> {
        Self::parse_inner(raw, true)
    }

    pub(crate) fn parse(raw: &str) -> Result<Self> {
        Self::parse_inner(raw, false)
    }

    fn parse_inner(raw: &str, current: bool) -> Result<Self> {
        if raw.len() > MAX_REQUEST_BYTES {
            return Err(bounds("connected room source exceeds 65536 bytes"));
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Region {
            min: [i32; 3],
            max: [i32; 3],
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Access {
            entry: [i32; 3],
            region: Region,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Input {
            template: String,
            planner: Option<String>,
            room: Json,
            access: Access,
        }
        let input: Input = serde_json::from_str(raw)
            .map_err(|error| invalid(format!("invalid connected room source: {error}")))?;
        if input.template != "connected_room"
            || !match input.planner.as_deref() {
                Some(PLANNER) => true,
                None => current,
                _ => false,
            }
        {
            return Err(invalid(
                "connected_room requires the fixed connected_room_v1 planner",
            ));
        }
        let room = if current {
            ProvisionedRoomRequest::parse_current(&input.room.to_string())?
        } else {
            ProvisionedRoomRequest::parse(&input.room.to_string())?
        };
        let request = Self {
            room,
            entry: coordinate(input.access.entry),
            area: MapCuboid::new(
                coordinate(input.access.region.min),
                coordinate(input.access.region.max),
            )?,
        };
        let (layout, targets) = request.room.access_geometry()?;
        request.validate_geometry(&layout, &targets)?;
        if request.canonical_json().len() > MAX_REQUEST_BYTES {
            return Err(bounds(
                "canonical connected room source exceeds 65536 bytes",
            ));
        }
        Ok(request)
    }

    pub(crate) fn canonical_json(&self) -> String {
        // Both components are generated JSON, so embedding the room preserves
        // its exact canonical generation without a fallible parse round trip.
        format!(
            "{{\"access\":{},\"planner\":\"{PLANNER}\",\"room\":{},\"template\":\"connected_room\"}}",
            json!({
                "entry":position(self.entry),
                "region":{"min":position(self.area.min),"max":position(self.area.max)},
            }),
            self.room.canonical_json(),
        )
    }

    pub(crate) fn compile(
        &self,
        id: IntentId,
        snapshot: &WorldSnapshot,
        summary: &str,
    ) -> Result<(Intent, Json)> {
        self.compile_inner(id, snapshot, summary, false)
    }

    pub(crate) fn compile_residual(
        &self,
        id: IntentId,
        snapshot: &WorldSnapshot,
        summary: &str,
    ) -> Result<(Intent, Json)> {
        self.compile_inner(id, snapshot, summary, true)
    }

    fn compile_inner(
        &self,
        id: IntentId,
        snapshot: &WorldSnapshot,
        summary: &str,
        residual: bool,
    ) -> Result<(Intent, Json)> {
        let source_work = self.validate_source(snapshot)?;
        let build_access =
            |layout: &BlueprintLayout, targets: &[MapCoord], production: &[RequestedAction]| {
                self.build_access(snapshot, layout, targets, production, source_work)
            };
        let (mut intent, mut analysis) =
            self.room
                .compile_with_access(id, snapshot, residual, &build_access)?;
        let mut source = if residual {
            b"dfmcp-connected-room-residual-source/1\0".to_vec()
        } else {
            b"dfmcp-connected-room-source/1\0".to_vec()
        };
        source.extend_from_slice(self.canonical_json().as_bytes());
        let digest = Digest32::of_bytes(&source);
        let label = if summary.is_empty() {
            "connect and provision the original room"
        } else {
            summary
        };
        // This complete wrapper digest includes the original room and every
        // access choice. One suffix leaves space for a continuation lineage seal.
        intent.summary = format!("{label} [connected room source {}]", digest.to_hex());
        if intent.summary.len() > StaticPlanner::default().policy.max_string_bytes
            || intent.summary.contains('\0')
        {
            return Err(invalid(
                "connected room summary plus source seal exceeds the planner bound",
            ));
        }
        analysis["template"] = json!("connected_room");
        analysis["room_planner"] = analysis["planner"].clone();
        analysis["planner"] = json!(if residual { RESIDUAL_PLANNER } else { PLANNER });
        analysis["source_binding"]["connected_room_source_digest"] = json!(digest.to_hex());
        analysis["source_binding"]["access_request_preserved"] = json!(true);
        analysis["goal"] = json!(
            "every original room terrain region and furnishing, plus current entrance-to-furnishing approach access"
        );
        Ok((intent, analysis))
    }

    fn validate_geometry(&self, layout: &BlueprintLayout, targets: &[MapCoord]) -> Result<()> {
        if self.area.min.z != self.area.max.z
            || self
                .area
                .tile_count()
                .is_none_or(|count| count == 0 || count > MAX_CELLS)
        {
            return Err(bounds(
                "connected room access requires at most 1024 cells on one level",
            ));
        }
        if !self.area.contains(self.entry)
            || targets.is_empty()
            || targets.len() > 16
            || targets.iter().any(|target| !self.area.contains(*target))
            || layout.excavations().iter().any(|part| {
                !self.area.contains(part.area.min) || !self.area.contains(part.area.max)
            })
        {
            return Err(invalid(
                "the access domain must contain the entry and the complete original room geometry",
            ));
        }
        if targets.iter().copied().collect::<BTreeSet<_>>().len() != targets.len() {
            return Err(invalid(
                "connected room furnishing targets must be distinct",
            ));
        }
        Ok(())
    }

    fn validate_source(&self, snapshot: &WorldSnapshot) -> Result<u64> {
        // The shared room compiler constructs a full spatial index before the
        // access hook. Bound malformed encodings before that existing scan;
        // zero-length runs must not turn a 256-tile chunk into unbounded work.
        if snapshot.graph.chunks.len() > MAX_SOURCE_CHUNKS {
            return Err(bounds("connected room source exceeds 4096 terrain chunks"));
        }
        if snapshot.graph.entities.len()
            > dfmcp_world::laboratory_access::MAX_LABORATORY_ACCESS_ENTITIES
        {
            return Err(bounds("connected room source exceeds 65536 entities"));
        }
        for (key, chunk) in &snapshot.graph.chunks {
            if chunk.coord != *key
                || chunk.width != 16
                || chunk.height != 16
                || chunk.terrain_runs.len() > 256
                || chunk.sparse_overlays.len() > 256
                || chunk.terrain_runs.iter().any(|run| run.length == 0)
                || chunk.encoded_tile_count() != Some(256)
                || chunk.sparse_overlays.keys().any(|offset| *offset >= 256)
            {
                return Err(precondition(
                    "connected room source contains malformed bounded terrain chunks",
                ));
            }
        }
        // Six run passes plus two spatial-index materializations, one complete
        // qualified occupancy census and bounded route tile lookups. These are
        // conservative structural units, not elapsed time or CPU instructions.
        Ok(snapshot.graph.chunks.len() as u64 * (6 * 256 + 2 * 256)
            + snapshot.graph.entities.len() as u64 * 32
            + (self
                .area
                .tile_count()
                .ok_or_else(|| bounds("access area overflow"))?
                + 1)
                * 512)
    }

    fn build_access(
        &self,
        snapshot: &WorldSnapshot,
        layout: &BlueprintLayout,
        targets: &[MapCoord],
        production: &[RequestedAction],
        source_work: u64,
    ) -> Result<RoomAccessComposition> {
        self.validate_geometry(layout, targets)?;
        let mut work = Work::default();
        work.charge(source_work)?;
        let mut occupied = BTreeSet::new();
        for (building, area) in &laboratory_building_footprints(snapshot)? {
            // The shared access census recognizes canonical kind names, while
            // older room excavation/retention code requires the typed variant.
            // Such an alias may block a route, but cannot authorize mining or
            // placement over a building the underlying room compiler ignores.
            if snapshot
                .graph
                .entities
                .get(building)
                .is_some_and(|record| record.kind != EntityKind::Building)
            {
                let requested_areas = layout.excavations().iter().map(|part| part.area).chain(
                    production.iter().filter_map(|step| match step.action {
                        Action::Build { footprint, .. } => Some(footprint),
                        _ => None,
                    }),
                );
                for requested in requested_areas {
                    work.charge(1)?;
                    if requested.min.x <= area.max.x
                        && requested.max.x >= area.min.x
                        && requested.min.y <= area.max.y
                        && requested.max.y >= area.min.y
                        && requested.min.z <= area.max.z
                        && requested.max.z >= area.min.z
                    {
                        return Err(precondition(
                            "an aliased building overlaps requested room or production geometry without typed occupancy custody",
                        ));
                    }
                }
            }
            project_occupancy(self.area, *area, &mut occupied, &mut work)?;
        }
        let mut planned_workshops = 0_usize;
        for step in production {
            if let Action::Build { footprint, .. } = step.action {
                project_occupancy(self.area, footprint, &mut occupied, &mut work)?;
                planned_workshops += 1;
            }
        }
        let target_set: BTreeSet<_> = targets.iter().copied().collect();
        if snapshot.tile_code_at(self.entry) != Some(tile_codes::FLOOR)
            || occupied.contains(&self.entry)
            || target_set.contains(&self.entry)
        {
            return Err(precondition(
                "connected room entry must be observed unoccupied floor outside furnishing targets",
            ));
        }
        let planned: BTreeSet<_> = layout
            .excavations()
            .iter()
            .flat_map(|part| dfmcp_world::terrain::region_tiles(part.area))
            .collect();
        let walls = protected_room_walls(layout, &planned)?;
        if walls.contains(&self.entry) {
            return Err(precondition(
                "connected room entry occupies a protected bedroom wall",
            ));
        }
        let dock = room_endpoint(layout, &planned, &target_set, &occupied)?;
        let internal =
            intended_circulation(self.area, dock, &planned, &target_set, &occupied, &mut work)?;
        if targets.iter().any(|target| {
            !neighbors(self.area, *target)
                .iter()
                .any(|at| internal.contains(at))
        }) {
            return Err(precondition(
                "the original furniture layout has no connected cardinal approach to every furnishing",
            ));
        }
        let inside = planned.contains(&self.entry);
        if inside && !internal.contains(&self.entry) {
            return Err(precondition(
                "the selected entry is disconnected within the intended room circulation",
            ));
        }
        let spatial = spatial_index(snapshot)?;
        let mut costs = BTreeMap::new();
        let mut observed = BTreeMap::new();
        for at in dfmcp_world::terrain::region_tiles(self.area) {
            work.charge(1)?;
            let tile = snapshot.tile_code_at(at).ok_or_else(|| {
                precondition("the bounded connected room routing domain has unobserved terrain")
            })?;
            observed.insert(at, tile);
            if planned.contains(&at) && !matches!(tile, tile_codes::FLOOR | tile_codes::SOLID_WALL)
            {
                return Err(precondition(
                    "connected room geometry requires observed floor or ordinary wall",
                ));
            }
            if occupied.contains(&at) || target_set.contains(&at) || walls.contains(&at) {
                continue;
            }
            if inside {
                if internal.contains(&at) {
                    costs.insert(at, 0);
                }
                continue;
            }
            if planned.contains(&at) && at != dock {
                continue;
            }
            match tile {
                tile_codes::FLOOR => {
                    costs.insert(at, 0);
                }
                tile_codes::SOLID_WALL if at == dock => {
                    costs.insert(at, 0);
                }
                tile_codes::SOLID_WALL => {
                    work.charge(27)?;
                    let singleton = MapCuboid::new(at, at)?;
                    if BlueprintPlanner.assess_hazards(&singleton, &spatial)
                        == HazardAssessment::Safe
                    {
                        costs.insert(at, 1);
                    }
                }
                _ => {}
            }
        }
        let path = minimum_path(self.area, self.entry, dock, &costs, &mut work)?;
        let missing: BTreeSet<_> = path
            .iter()
            .copied()
            .filter(|at| !planned.contains(at) && observed.get(at) == Some(&tile_codes::SOLID_WALL))
            .collect();
        let excavations = partition_mask(&missing)?;
        for area in &excavations {
            require_safe_area(*area, &spatial)?;
        }
        let mut targets = targets.to_vec();
        targets.sort_unstable();
        let terminal = Predicate::LaboratoryRoomAccess {
            area: self.area,
            entry: self.entry,
            targets,
        };
        terminal.validate_shape()?;
        let analysis = json!({
            "policy":ROUTING_POLICY,
            "entry":position(self.entry), "room_endpoint":position(dock),
            "region":{"min":position(self.area.min),"max":position(self.area.max)},
            "route":path.iter().copied().map(position).collect::<Vec<_>>(),
            "new_corridor_tiles":missing.len(), "corridor_parts":excavations.len(),
            "protected_bedroom_wall_tiles":walls.len(), "planned_workshops_avoided":planned_workshops,
            "work_units":work.used,
            "selection_policy":"fewest new wall tiles, then shortest cardinal path, then lexicographic (z,y,x) path",
            "current_access_is_a_terminal_requirement":true,
            "native_pathfinding_proven":false, "entry_global_fortress_membership_proven":false,
        });
        Ok(RoomAccessComposition {
            excavations,
            terminal,
            analysis,
        })
    }
}

#[derive(Default)]
struct Work {
    used: u64,
}

impl Work {
    fn charge(&mut self, amount: u64) -> Result<()> {
        self.used = self
            .used
            .checked_add(amount)
            .ok_or_else(|| bounds("connected room work overflow"))?;
        if self.used > MAX_WORK {
            return Err(bounds(
                "connected room routing exceeded its bounded work allowance",
            ));
        }
        Ok(())
    }
}

fn project_occupancy(
    domain: MapCuboid,
    footprint: MapCuboid,
    occupied: &mut BTreeSet<MapCoord>,
    work: &mut Work,
) -> Result<()> {
    work.charge(1)?;
    if footprint.max.x < domain.min.x
        || footprint.min.x > domain.max.x
        || footprint.max.y < domain.min.y
        || footprint.min.y > domain.max.y
        || footprint.max.z < domain.min.z
        || footprint.min.z > domain.max.z
    {
        return Ok(());
    }
    let clipped = MapCuboid::new(
        MapCoord::new(
            footprint.min.x.max(domain.min.x),
            footprint.min.y.max(domain.min.y),
            domain.min.z,
        ),
        MapCoord::new(
            footprint.max.x.min(domain.max.x),
            footprint.max.y.min(domain.max.y),
            domain.max.z,
        ),
    )?;
    for at in dfmcp_world::terrain::region_tiles(clipped) {
        work.charge(1)?;
        occupied.insert(at);
    }
    Ok(())
}

fn protected_room_walls(
    layout: &BlueprintLayout,
    planned: &BTreeSet<MapCoord>,
) -> Result<BTreeSet<MapCoord>> {
    let mut walls = BTreeSet::new();
    // Only bedroom geometry promises separating walls. Its doorway/corridor
    // cells are explicit planned floor and remain openings in this ring.
    if layout.access_point().is_none() {
        return Ok(walls);
    }
    for room in layout
        .excavations()
        .iter()
        .filter(|part| part.role == ExcavationRole::Room)
    {
        let a = room.area;
        let min = MapCoord::new(
            a.min
                .x
                .checked_sub(1)
                .ok_or_else(|| invalid("room wall coordinate overflow"))?,
            a.min
                .y
                .checked_sub(1)
                .ok_or_else(|| invalid("room wall coordinate overflow"))?,
            a.min.z,
        );
        let max = MapCoord::new(
            a.max
                .x
                .checked_add(1)
                .ok_or_else(|| invalid("room wall coordinate overflow"))?,
            a.max
                .y
                .checked_add(1)
                .ok_or_else(|| invalid("room wall coordinate overflow"))?,
            a.max.z,
        );
        for at in dfmcp_world::terrain::region_tiles(MapCuboid::new(min, max)?) {
            if !planned.contains(&at) {
                walls.insert(at);
            }
        }
    }
    Ok(walls)
}

fn room_endpoint(
    layout: &BlueprintLayout,
    planned: &BTreeSet<MapCoord>,
    targets: &BTreeSet<MapCoord>,
    occupied: &BTreeSet<MapCoord>,
) -> Result<MapCoord> {
    let point = if let Some(point) = layout.access_point() {
        Some(point)
    } else {
        let room = layout
            .excavations()
            .iter()
            .find(|part| part.role == ExcavationRole::Room)
            .ok_or_else(|| invalid("connected room layout has no room"))?;
        planned
            .iter()
            .copied()
            .filter(|at| {
                (at.x == room.area.min.x
                    || at.x == room.area.max.x
                    || at.y == room.area.min.y
                    || at.y == room.area.max.y)
                    && !targets.contains(at)
                    && !occupied.contains(at)
            })
            .min_by_key(|at| order(*at))
    }
    .ok_or_else(|| precondition("the room has no free perimeter entrance"))?;
    if !planned.contains(&point) || targets.contains(&point) || occupied.contains(&point) {
        return Err(precondition(
            "the intended room entrance is occupied or outside its floor geometry",
        ));
    }
    Ok(point)
}

fn intended_circulation(
    area: MapCuboid,
    entry: MapCoord,
    planned: &BTreeSet<MapCoord>,
    targets: &BTreeSet<MapCoord>,
    occupied: &BTreeSet<MapCoord>,
    work: &mut Work,
) -> Result<BTreeSet<MapCoord>> {
    let mut reached = BTreeSet::from([entry]);
    let mut queue = VecDeque::from([entry]);
    while let Some(at) = queue.pop_front() {
        work.charge(1)?;
        for next in neighbors(area, at) {
            work.charge(1)?;
            if planned.contains(&next)
                && !targets.contains(&next)
                && !occupied.contains(&next)
                && reached.insert(next)
            {
                queue.push_back(next);
            }
        }
    }
    Ok(reached)
}

fn neighbors(area: MapCuboid, at: MapCoord) -> Vec<MapCoord> {
    let mut adjacent = Vec::with_capacity(4);
    for (dx, dy) in [(-1, 0), (0, -1), (0, 1), (1, 0)] {
        let (Some(x), Some(y)) = (at.x.checked_add(dx), at.y.checked_add(dy)) else {
            continue;
        };
        let next = MapCoord::new(x, y, at.z);
        if area.contains(next) {
            adjacent.push(next);
        }
    }
    adjacent.sort_unstable_by_key(|at| order(*at));
    adjacent
}

/// Reverse lexicographic-cost Dijkstra followed by canonical reconstruction.
/// A forward step pays the entered cell's excavation cost. Reverse relaxation
/// therefore pays the current cell, preserving the exact directed cost model.
fn minimum_path(
    area: MapCuboid,
    entry: MapCoord,
    endpoint: MapCoord,
    costs: &BTreeMap<MapCoord, u32>,
    work: &mut Work,
) -> Result<Vec<MapCoord>> {
    if !costs.contains_key(&entry) || !costs.contains_key(&endpoint) {
        return Err(precondition(
            "the required entry or room endpoint is unavailable to the route",
        ));
    }
    let mut distance = BTreeMap::from([(endpoint, (0_u32, 0_u32))]);
    let mut frontier = BTreeSet::from([(0_u32, 0_u32, order(endpoint))]);
    while let Some((walls, length, (z, y, x))) = frontier.pop_first() {
        work.charge(1)?;
        let at = MapCoord::new(x, y, z);
        if distance.get(&at) != Some(&(walls, length)) {
            continue;
        }
        let entered_cost = costs
            .get(&at)
            .copied()
            .ok_or_else(|| invalid("routing frontier escaped the eligible domain"))?;
        for previous in neighbors(area, at) {
            work.charge(1)?;
            if !costs.contains_key(&previous) {
                continue;
            }
            let candidate = (
                walls
                    .checked_add(entered_cost)
                    .ok_or_else(|| bounds("route cost overflow"))?,
                length
                    .checked_add(1)
                    .ok_or_else(|| bounds("route length overflow"))?,
            );
            if distance
                .get(&previous)
                .is_none_or(|prior| candidate < *prior)
            {
                if let Some(prior) = distance.insert(previous, candidate) {
                    frontier.remove(&(prior.0, prior.1, order(previous)));
                }
                frontier.insert((candidate.0, candidate.1, order(previous)));
            }
        }
    }
    if !distance.contains_key(&entry) {
        return Err(precondition(
            "no eligible corridor reaches the intended room entrance within the bounded domain",
        ));
    }
    let mut path = vec![entry];
    let mut at = entry;
    while at != endpoint {
        work.charge(1)?;
        let remaining = distance
            .get(&at)
            .copied()
            .ok_or_else(|| invalid("route distance missing"))?;
        let next = neighbors(area, at)
            .into_iter()
            .find(|next| match (distance.get(next), costs.get(next)) {
                (Some((walls, length)), Some(cost)) => {
                    walls.checked_add(*cost) == Some(remaining.0)
                        && length.checked_add(1) == Some(remaining.1)
                }
                _ => false,
            })
            .ok_or_else(|| invalid("minimum route has no canonical successor"))?;
        path.push(next);
        if path.len() > MAX_CELLS as usize {
            return Err(bounds("minimum corridor path exceeds its domain bound"));
        }
        at = next;
    }
    Ok(path)
}

/// Merge identical contiguous horizontal runs on successive rows. Every emitted
/// rectangle contains selected wall cells only; no bounding-box overdig occurs.
fn partition_mask(mask: &BTreeSet<MapCoord>) -> Result<Vec<MapCuboid>> {
    let mut rows = BTreeMap::<(i32, i32), BTreeSet<i32>>::new();
    for at in mask {
        rows.entry((at.z, at.y)).or_default().insert(at.x);
    }
    let mut result = Vec::new();
    let mut open = BTreeMap::<(i32, i32), MapCuboid>::new();
    for ((z, y), xs) in rows {
        let mut runs = Vec::new();
        let mut current: Option<(i32, i32)> = None;
        for x in xs {
            match current {
                Some((start, end)) if end.checked_add(1) == Some(x) => current = Some((start, x)),
                Some(run) => {
                    runs.push(run);
                    current = Some((x, x));
                }
                None => current = Some((x, x)),
            }
        }
        if let Some(run) = current {
            runs.push(run);
        }
        let mut next = BTreeMap::new();
        for (min_x, max_x) in runs {
            let prior = open.remove(&(min_x, max_x));
            let area = match prior {
                Some(prior) if prior.max.z == z && prior.max.y.checked_add(1) == Some(y) => {
                    MapCuboid::new(prior.min, MapCoord::new(max_x, y, z))?
                }
                prior => {
                    if let Some(prior) = prior {
                        result.push(prior);
                    }
                    MapCuboid::new(MapCoord::new(min_x, y, z), MapCoord::new(max_x, y, z))?
                }
            };
            next.insert((min_x, max_x), area);
        }
        result.extend(open.into_values());
        open = next;
    }
    result.extend(open.into_values());
    result.sort_unstable_by_key(|area| (order(area.min), order(area.max)));
    if result.len() > MAX_STEPS {
        return Err(bounds(
            "connected room corridor exceeds 64 exact excavation parts",
        ));
    }
    Ok(result)
}

fn require_safe_area(
    area: MapCuboid,
    spatial: &dfmcp_world::spatial_index::ChunkSpatialIndex,
) -> Result<()> {
    match BlueprintPlanner.assess_hazards(&area, spatial) {
        HazardAssessment::Safe => Ok(()),
        HazardAssessment::MagmaProximity { .. } => Err(precondition(
            "connected room corridor approaches a magma hazard",
        )),
        HazardAssessment::UnsupportedCaveInRisk { .. } => Err(precondition(
            "connected room corridor exceeds the supported excavation span",
        )),
        HazardAssessment::IncompleteKnowledge { .. } => Err(precondition(
            "connected room corridor lacks a complete hazard halo",
        )),
    }
}

fn coordinate(at: [i32; 3]) -> MapCoord {
    MapCoord::new(at[0], at[1], at[2])
}
fn position(at: MapCoord) -> [i32; 3] {
    [at.x, at.y, at.z]
}
fn order(at: MapCoord) -> (i32, i32, i32) {
    (at.z, at.y, at.x)
}

fn invalid(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}
fn precondition(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::PreconditionsFailed, message)
}
fn bounds(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

#[cfg(test)]
#[path = "connected_room_tests.rs"]
mod tests;
