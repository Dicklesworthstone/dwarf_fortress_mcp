//! Opt-in multi-level routing in the laboratory's paired-stair model.
//!
//! A new vertical connection excavates ordinary rock into generic laboratory
//! stairs on both levels. Existing floors and the original room's floor masks
//! are never converted into shafts. Native directional stairs are not proved.

use super::*;
use dfmcp_intent::DigMode;

const POLICY: &str = "laboratory-room-mining-paired-stairs/1";

pub(super) fn build(
    request: &ConnectedRoomRequest,
    snapshot: &WorldSnapshot,
    layout: &BlueprintLayout,
    targets: &[MapCoord],
    production: &[RequestedAction],
    source_work: u64,
) -> Result<RoomAccessComposition> {
    request.validate_geometry(layout, targets)?;
    let mut work = Work::default();
    work.charge(source_work)?;
    let mut occupied = BTreeSet::new();
    for (building, footprint) in laboratory_building_footprints(snapshot)? {
        if snapshot
            .graph
            .entities
            .get(&building)
            .is_some_and(|record| record.kind != EntityKind::Building)
        {
            for area in layout.excavations().iter().map(|part| part.area).chain(
                production.iter().filter_map(|step| match step.action {
                    Action::Build { footprint, .. } => Some(footprint),
                    _ => None,
                }),
            ) {
                work.charge(1)?;
                if area.min.x <= footprint.max.x
                    && area.max.x >= footprint.min.x
                    && area.min.y <= footprint.max.y
                    && area.max.y >= footprint.min.y
                    && area.min.z <= footprint.max.z
                    && area.max.z >= footprint.min.z
                {
                    return Err(precondition(
                        "an aliased building overlaps requested room or production geometry without typed occupancy custody",
                    ));
                }
            }
        }
        project_occupancy(request.area, footprint, &mut occupied, &mut work)?;
    }
    let mut planned_workshops = 0_usize;
    for step in production {
        if let Action::Build { footprint, .. } = step.action {
            project_occupancy(request.area, footprint, &mut occupied, &mut work)?;
            planned_workshops += 1;
        }
    }
    let target_set: BTreeSet<_> = targets.iter().copied().collect();
    if !matches!(
        snapshot.tile_code_at(request.entry),
        Some(tile_codes::FLOOR | tile_codes::STAIR)
    ) || occupied.contains(&request.entry)
        || target_set.contains(&request.entry)
    {
        return Err(precondition(
            "paired-stair room entry must be observed unoccupied floor or stair outside furnishing targets",
        ));
    }
    let planned: BTreeSet<_> = layout
        .excavations()
        .iter()
        .flat_map(|part| dfmcp_world::terrain::region_tiles(part.area))
        .collect();
    let walls = protected_room_walls(layout, &planned)?;
    if walls.contains(&request.entry) {
        return Err(precondition(
            "connected room entry occupies a protected bedroom wall",
        ));
    }
    let dock = room_endpoint(layout, &planned, &target_set, &occupied)?;
    let internal = intended_circulation(
        request.area,
        dock,
        &planned,
        &target_set,
        &occupied,
        &mut work,
    )?;
    if targets.iter().any(|target| {
        !neighbors(request.area, *target)
            .iter()
            .any(|at| internal.contains(at))
    }) {
        return Err(precondition(
            "the original furniture layout has no connected cardinal approach to every furnishing",
        ));
    }
    let inside = planned.contains(&request.entry);
    if inside && !internal.contains(&request.entry) {
        return Err(precondition(
            "the selected entry is disconnected within the intended room circulation",
        ));
    }

    let spatial = spatial_index(snapshot)?;
    let mut costs = BTreeMap::new();
    let mut vertical = BTreeSet::new();
    let mut observed = BTreeMap::new();
    for at in dfmcp_world::terrain::region_tiles(request.area) {
        work.charge(1)?;
        let tile = snapshot.tile_code_at(at).ok_or_else(|| {
            precondition("the bounded connected room routing domain has unobserved terrain")
        })?;
        observed.insert(at, tile);
        if planned.contains(&at) && !matches!(tile, tile_codes::FLOOR | tile_codes::SOLID_WALL) {
            return Err(precondition(
                "connected room floor geometry requires observed floor or ordinary wall",
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
            tile_codes::STAIR => {
                costs.insert(at, 0);
                vertical.insert(at);
            }
            tile_codes::SOLID_WALL if at == dock => {
                costs.insert(at, 0);
            }
            tile_codes::SOLID_WALL => {
                work.charge(27)?;
                if BlueprintPlanner.assess_hazards(&MapCuboid::new(at, at)?, &spatial)
                    == HazardAssessment::Safe
                {
                    costs.insert(at, 1);
                    // All ordinary rock costs one new tile whether it is mined
                    // into a corridor or excavated as part of a stair shaft.
                    vertical.insert(at);
                }
            }
            _ => {}
        }
    }
    let path = minimum_path_using(request.entry, dock, &costs, &mut work, |at| {
        adjacent(request.area, at, &vertical)
    })?;
    let mut shaft = BTreeSet::new();
    for pair in path.windows(2) {
        if pair[0].z != pair[1].z {
            shaft.extend(pair.iter().copied());
        }
    }
    let missing: BTreeSet<_> = path
        .iter()
        .copied()
        .filter(|at| !planned.contains(at) && observed.get(at) == Some(&tile_codes::SOLID_WALL))
        .collect();
    let new_stairs: BTreeSet<_> = missing.intersection(&shaft).copied().collect();
    let new_corridor: BTreeSet<_> = missing.difference(&shaft).copied().collect();
    let corridors = partition_mask(&new_corridor)?;
    let stairs = partition_mask(&new_stairs)?;
    if corridors.len().saturating_add(stairs.len()) > MAX_STEPS {
        return Err(bounds(
            "connected room route exceeds 64 exact excavation parts",
        ));
    }
    let corridor_parts = corridors.len();
    let stair_parts = stairs.len();
    let mut excavations = Vec::with_capacity(corridor_parts + stair_parts);
    for (parts, mode) in [(corridors, DigMode::Mine), (stairs, DigMode::UpDownStair)] {
        for area in parts {
            require_safe_area(area, &spatial)?;
            excavations.push(RoomAccessExcavation { area, mode });
        }
    }
    let mut targets = targets.to_vec();
    targets.sort_unstable();
    let terminal = Predicate::LaboratoryRoomAccessPairedStairs {
        area: request.area,
        entry: request.entry,
        targets,
    };
    terminal.validate_shape()?;
    let analysis = json!({
        "policy":POLICY,
        "entry":position(request.entry), "room_endpoint":position(dock),
        "region":{"min":position(request.area.min),"max":position(request.area.max)},
        "route":path.iter().copied().map(position).collect::<Vec<_>>(),
        "new_corridor_tiles":new_corridor.len(), "corridor_parts":corridor_parts,
        "new_stair_tiles":new_stairs.len(), "stair_parts":stair_parts,
        "vertical_edges":path.windows(2).filter(|pair| pair[0].z != pair[1].z).count(),
        "stair_model":"paired adjacent generic laboratory stairs, bidirectional",
        "shaft_policy":"ordinary rock or existing stairs outside original room floor masks",
        "protected_bedroom_wall_tiles":walls.len(), "planned_workshops_avoided":planned_workshops,
        "work_units":work.used,
        "selection_policy":"fewest new wall tiles, then shortest cardinal/paired-stair path, then lexicographic (z,y,x) path",
        "current_access_is_a_terminal_requirement":true,
        "native_pathfinding_proven":false, "native_directional_stairs_proven":false,
        "entry_global_fortress_membership_proven":false,
    });
    Ok(RoomAccessComposition {
        excavations,
        terminal,
        analysis,
    })
}

/// Horizontal movement uses eligible cells. A vertical edge requires both
/// endpoints to be existing stairs or safe shaft rock outside room floor masks.
pub(super) fn adjacent(
    area: MapCuboid,
    at: MapCoord,
    vertical: &BTreeSet<MapCoord>,
) -> Vec<MapCoord> {
    let mut result = neighbors(area, at);
    if vertical.contains(&at) {
        for dz in [-1, 1] {
            if let Some(z) = at.z.checked_add(dz) {
                let next = MapCoord::new(at.x, at.y, z);
                if area.contains(next) && vertical.contains(&next) {
                    result.push(next);
                }
            }
        }
    }
    result.sort_unstable_by_key(|at| order(*at));
    result
}
