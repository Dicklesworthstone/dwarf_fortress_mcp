//! Closed terrain transitions for reference excavation.
//!
//! Excavation removes or shapes observed material; it cannot create support
//! inside empty space, a chasm, a tree or an unsupported/hazardous tile. This
//! coarse laboratory model retains generic stairs and abstract construction
//! removal. It does not establish native stair direction, construction origin,
//! material refunds, surrounding hazard clearance or structural support.

use super::*;

/// Whether one observed canonical source cell supports the registered
/// laboratory excavation. Already-correct target cells are harmless no-ops.
/// Unknown numeric codes are refused rather than treated as generic rock.
#[must_use]
pub const fn dig_transition_allowed(mode: DigMode, source: u32) -> bool {
    if source == dig_target_tile_code(mode) {
        return true;
    }
    match mode {
        DigMode::Mine | DigMode::RemoveConstruction => matches!(
            source,
            tile_codes::SOLID_WALL
                | tile_codes::FORTIFICATION
                | tile_codes::STAIR
                | tile_codes::RAMP
        ),
        DigMode::Channel => matches!(
            source,
            tile_codes::SOLID_WALL
                | tile_codes::FLOOR
                | tile_codes::FORTIFICATION
                | tile_codes::STAIR
                | tile_codes::RAMP
        ),
        DigMode::DownStair => matches!(source, tile_codes::SOLID_WALL | tile_codes::FLOOR),
        DigMode::UpStair | DigMode::UpDownStair | DigMode::Ramp => source == tile_codes::SOLID_WALL,
    }
}

/// Read-only, allocation-free preflight over the complete bounded region.
/// The effect boundary repeats it against current terrain at dispatch and
/// progress; a successful preflight grants no future mutation authority.
pub fn validate_dig_terrain(
    snapshot: &WorldSnapshot,
    area: MapCuboid,
    mode: DigMode,
) -> Result<()> {
    validate_region(area)?;
    for coord in region_tiles(area) {
        validate_tile(snapshot, coord, mode)?;
    }
    Ok(())
}

pub(super) fn validate_tile(
    snapshot: &WorldSnapshot,
    coord: MapCoord,
    mode: DigMode,
) -> Result<u32> {
    let source = snapshot.tile_code_at(coord).ok_or_else(|| {
        precondition(format!(
            "excavation region includes unobserved terrain at {coord:?}"
        ))
    })?;
    validate_source(coord, mode, source)?;
    Ok(source)
}

pub(super) fn validate_source(coord: MapCoord, mode: DigMode, source: u32) -> Result<()> {
    if !dig_transition_allowed(mode, source) {
        return Err(precondition(format!(
            "laboratory {} excavation cannot transform terrain code {source} at {coord:?}; an eligible observed source is required",
            dig_mode_name(mode),
        )));
    }
    Ok(())
}
