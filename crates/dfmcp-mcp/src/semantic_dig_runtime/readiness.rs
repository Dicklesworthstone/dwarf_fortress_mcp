//! Independently authorized raw target qualification for finite mining time.
//! Pending designated walls remain outside the canonical floor proof. Readiness
//! never certifies reachability, structural safety, completion or current pause.
use super::*;
use dfmcp_intent::{Action, DigMode};
use dfmcp_world::map_region::{Cell, Shape};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClockReadiness {
    pub anchor: StateAnchor,
    pub source_digest: Digest32,
    pub eligible: bool,
    pub pending_tiles: u32,
    pub cleared_tiles: u32,
    pub reason: &'static str,
    pub blockers: Vec<&'static str>,
}
impl ClockReadiness {
    pub fn to_json(&self) -> Value {
        json!({"eligible":self.eligible,"anchor":anchor_json(self.anchor),
            "source_digest":self.source_digest.to_hex(),"pending_tiles":self.pending_tiles,
            "cleared_tiles":self.cleared_tiles,"reason":self.reason,"blockers":self.blockers,
            "policy":"operator-qualified-original-mine-advancement/1","original_goal_truth_upgraded":false})
    }
    pub(super) fn capture(
        original: &PreparedPlan,
        projection: &DigGoalProjection,
        source_authorized: bool,
    ) -> Result<Self> {
        let [step] = original.steps.as_slice() else {
            return Err(denied("clock terrain requires one original Mine target"));
        };
        let Action::DesignateDig {
            area,
            mode: DigMode::Mine,
        } = &step.action
        else {
            return Err(denied("clock terrain requires the original Mine rectangle"));
        };
        if area
            .tile_count()
            .is_none_or(|count| count == 0 || count > 64)
        {
            return Err(denied(
                "clock terrain exceeds the original bounded Mine target",
            ));
        }
        let mut out = Self {
            anchor: projection.anchor(),
            source_digest: projection.source_digest(),
            eligible: false,
            pending_tiles: 0,
            cleared_tiles: 0,
            reason: "target_not_qualified",
            blockers: Vec::new(),
        };
        if !source_authorized {
            out.reason = "source_not_authorized";
            out.blockers.push(out.reason);
            return Ok(out);
        }
        let raw = projection.raw().spatial().terrain();
        if !raw.paused {
            out.blockers.push("source_not_paused");
        }
        for tile in dfmcp_world::terrain::region_tiles(*area) {
            let position = [tile.x as u32, tile.y as u32, tile.z as u32];
            let cell = raw
                .map
                .region
                .index(position)
                .and_then(|index| raw.map.cells.get(index));
            let blocker = match cell {
                None | Some(Cell::Unallocated) => Some("target_unallocated"),
                Some(Cell::Hidden) => Some("target_hidden"),
                Some(Cell::Visible(tile)) if tile.liquid_depth > 0 || tile.magma => {
                    Some("target_wet")
                }
                Some(Cell::Visible(tile)) if tile.building_occupancy != 0 => {
                    Some("target_occupied_by_building")
                }
                Some(Cell::Visible(tile))
                    if tile.shape == Shape::Wall && tile.dig_designation == 1 =>
                {
                    out.pending_tiles += 1;
                    None
                }
                Some(Cell::Visible(tile))
                    if tile.shape == Shape::Floor && tile.dig_designation == 0 =>
                {
                    out.cleared_tiles += 1;
                    None
                }
                Some(Cell::Visible(_)) => Some("target_not_pending_mine_or_cleared_floor"),
            };
            if let Some(blocker) = blocker
                && !out.blockers.contains(&blocker)
            {
                out.blockers.push(blocker);
            }
        }
        out.eligible = out.blockers.is_empty();
        out.reason = if !out.eligible {
            "target_not_qualified"
        } else if out.pending_tiles > 0 {
            "original_mine_pending"
        } else {
            "original_floor_observed"
        };
        Ok(out)
    }
}
