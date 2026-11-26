//! Current spatial exclusions for reference excavation and construction.
//!
//! These are effect-time checks, not new plan predicates or archive semantics.
//! Already-correct terrain may remain beneath installed furniture while a room
//! repair excavates other cells. A changed tile may never belong to a building.

use super::*;

pub(super) fn capture_buildings(
    snapshot: &WorldSnapshot,
    budget: &mut EffectAdvanceBudget,
) -> Result<BTreeMap<EntityId, MapCuboid>> {
    budget.charge(snapshot.graph.entities.len() as u64)?;
    dfmcp_world::laboratory_building_footprints(snapshot)
}

/// Charge the complete clipped geometry before allocating its difference grid.
/// Its expanded dimensions contain at most eight times the requested tiles;
/// initialization, three prefix passes, extraction and lookup fit this bound.
fn mask(
    area: MapCuboid,
    buildings: &BTreeMap<EntityId, MapCuboid>,
    budget: &mut EffectAdvanceBudget,
) -> Result<Option<Vec<bool>>> {
    let tiles = validate_region(area)?;
    budget.charge(buildings.len() as u64)?;
    if !buildings
        .values()
        .any(|footprint| occupancy::intersects(area, *footprint))
    {
        return Ok(None);
    }
    budget.charge(tiles * 40 + buildings.len() as u64 * 16)?;
    dfmcp_world::laboratory_access::laboratory_building_occupancy_mask(area, buildings).map(Some)
}

pub(super) fn validate_dispatch(
    snapshot: &WorldSnapshot,
    area: MapCuboid,
    target: u32,
) -> Result<()> {
    let mut budget = EffectAdvanceBudget::new(EffectAdvanceLimits::default(), 0)?;
    let buildings = capture_buildings(snapshot, &mut budget)?;
    let Some(occupied) = mask(area, &buildings, &mut budget)? else {
        return Ok(());
    };
    for (coord, occupied) in region_tiles(area).zip(occupied) {
        if occupied && snapshot.tile_code_at(coord) != Some(target) {
            return Err(precondition(format!(
                "excavation would change terrain beneath a building at {coord:?}"
            )));
        }
    }
    Ok(())
}

/// Called once per active region when building the causal timeline, before
/// physical work starts. Buildings keep their footprint throughout an advance:
/// completing construction does not free ground, and no new action dispatches
/// inside that advance. A later foreground slice captures current state again.
pub(super) fn validate_pending(
    area: MapCuboid,
    pending: &BTreeSet<(i32, i32, i32)>,
    buildings: &BTreeMap<EntityId, MapCuboid>,
    budget: &mut EffectAdvanceBudget,
) -> Result<()> {
    let Some(occupied) = mask(area, buildings, budget)? else {
        return Ok(());
    };
    let width = usize::try_from(i64::from(area.max.x) - i64::from(area.min.x) + 1)
        .map_err(|_| advance_budget_error("excavation occupancy width exceeds its bound"))?;
    let height = usize::try_from(i64::from(area.max.y) - i64::from(area.min.y) + 1)
        .map_err(|_| advance_budget_error("excavation occupancy height exceeds its bound"))?;
    for &(z, y, x) in pending {
        let coord = MapCoord::new(x, y, z);
        if !area.contains(coord) {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "pending excavation coordinate lies outside its admitted region",
            ));
        }
        let x = usize::try_from(i64::from(x) - i64::from(area.min.x))
            .map_err(|_| advance_budget_error("excavation occupancy offset exceeds its bound"))?;
        let y = usize::try_from(i64::from(y) - i64::from(area.min.y))
            .map_err(|_| advance_budget_error("excavation occupancy offset exceeds its bound"))?;
        let z = usize::try_from(i64::from(z) - i64::from(area.min.z))
            .map_err(|_| advance_budget_error("excavation occupancy offset exceeds its bound"))?;
        if occupied[(z * height + y) * width + x] {
            return Err(precondition(format!(
                "active excavation would change terrain beneath a building at {coord:?}"
            )));
        }
    }
    Ok(())
}

/// A new building cannot claim floor that an unfinished non-floor designation
/// still owns. Mining around an already open footprint is compatible: its
/// target is FLOOR, so those occupied cells cannot change. Missing current
/// status or geometry never establishes that an original dig is harmless.
pub(super) fn require_clear_for_construction(
    snapshot: &WorldSnapshot,
    footprint: MapCuboid,
) -> Result<()> {
    if snapshot.graph.entities.len() > MAX_EFFECT_ADVANCE_PRODUCTION_ENTITIES {
        return Err(advance_budget_error(
            "excavation exclusion exceeds its canonical entity bound",
        ));
    }
    for (id, record) in &snapshot.graph.entities {
        if record.kind.as_str() != DIG_DESIGNATION_KIND {
            continue;
        }
        if *id != record.id || *id == EntityId::NIL || record.generation == 0 {
            return Err(precondition(
                "excavation exclusion has an invalid original entity identity",
            ));
        }
        match field_text(record, STATUS_FIELD, snapshot.tick) {
            Some(STATUS_CANCELLED | STATUS_COMPLETE) => continue,
            Some(STATUS_ACTIVE) => {}
            _ => {
                return Err(precondition(
                    "excavation exclusion requires a source-qualified original lifecycle",
                ));
            }
        }
        let area = designation_area(record, snapshot.tick)?;
        validate_region(area)?;
        if !occupancy::intersects(area, footprint) {
            continue;
        }
        if field_u64(record, "target_tile_code", snapshot.tick)? != u64::from(tile_codes::FLOOR) {
            return Err(precondition(
                "construction footprint overlaps unfinished terrain-changing excavation",
            ));
        }
    }
    Ok(())
}
