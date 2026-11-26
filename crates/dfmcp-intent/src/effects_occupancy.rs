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
