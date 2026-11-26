//! One current, bounded building census for reference spatial effects.
//!
//! This check runs at dispatch or progress against the actual world. It does
//! not change an action's canonical bytes or reinterpret an archived plan.
//! In particular, a future build can still be prepared before its prerequisite
//! excavation establishes open floor.

use super::*;

/// Require the whole requested cuboid to be free of current building occupancy.
/// Only the exact advancing building may exclude itself; callers establishing
/// a new effect must pass `None`.
///
/// The shared world census bounds all entities before allocation, validates
/// canonical identity aliases and source-qualified lifecycle/extent facts, and
/// proves cancelled furniture's release shape before dropping its footprint.
/// Unknown occupancy anywhere cannot certify vacant ground. Specialized
/// material custody checks remain independent and may refuse more cases.
pub(super) fn require_vacant(
    snapshot: &WorldSnapshot,
    footprint: MapCuboid,
    own_building: Option<EntityId>,
) -> Result<()> {
    let occupied = dfmcp_world::laboratory_building_footprints(snapshot)?;
    for (building, area) in occupied {
        if Some(building) != own_building && intersects(footprint, area) {
            return Err(precondition(format!(
                "requested construction footprint overlaps occupied building {}",
                building.get(),
            )));
        }
    }
    excavation_occupancy::require_clear_for_construction(snapshot, footprint)
}

/// Admit each active construction once, before any timeline event can run.
/// The caller shares one complete qualified census with excavation. No event
/// creates a building or releases its footprint, and excavation admission
/// prevents terrain changes beneath it during the admitted interval.
pub(super) fn validate_progress(
    snapshot: &WorldSnapshot,
    building: &EntityRecord,
    occupied: &BTreeMap<EntityId, MapCuboid>,
    budget: &mut EffectAdvanceBudget,
) -> Result<()> {
    let footprint = occupied
        .get(&building.id)
        .copied()
        .ok_or_else(|| precondition("active construction has no qualified current footprint"))?;
    let tiles = validate_region(footprint)?;
    // Match the laboratory access allowance: each tile can inspect all 256
    // canonical RLE runs, with room for lookup and validation overhead. The
    // validated footprint bound makes this arithmetic finite before any scan.
    budget.charge(tiles * 320 + occupied.len() as u64)?;
    let position = coord_field(building, "position", snapshot.tick)
        .ok_or_else(|| precondition("active construction requires a qualified current position"))?;
    if !footprint.contains(position) {
        return Err(precondition(
            "active construction position lies outside its current footprint",
        ));
    }
    for (other, area) in occupied {
        if *other != building.id && intersects(footprint, *area) {
            return Err(precondition(format!(
                "active construction footprint overlaps occupied building {}",
                other.get(),
            )));
        }
    }
    for coord in region_tiles(footprint) {
        if dfmcp_world::laboratory_access::laboratory_bounded_tile_code(snapshot, coord)
            != Some(tile_codes::FLOOR)
        {
            return Err(precondition(format!(
                "active construction requires observed open floor at {coord:?}"
            )));
        }
    }
    Ok(())
}

pub(super) fn intersects(left: MapCuboid, right: MapCuboid) -> bool {
    left.min.x <= right.max.x
        && right.min.x <= left.max.x
        && left.min.y <= right.max.y
        && right.min.y <= left.max.y
        && left.min.z <= right.max.z
        && right.min.z <= left.max.z
}
