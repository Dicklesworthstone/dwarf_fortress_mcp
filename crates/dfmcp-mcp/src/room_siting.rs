//! Choose a concrete original room from a bounded grid of possible origins.
//! Selection is performed once, before the ordinary room source is sealed.

use super::*;
use dfmcp_world::terrain::region_tiles;

const MAX_ORIGINS: u32 = 16;
const MAX_GEOMETRY_WORK: u64 = 131_072;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    template: String,
    version: u8,
    origin_grid: [u8; 2],
    #[serde(deserialize_with = "ProvisionedRoomRequest::deserialize_current")]
    room: ProvisionedRoomRequest,
}

pub(crate) fn is_request(raw: &str) -> bool {
    #[derive(Deserialize)]
    struct Header {
        template: String,
    }
    raw.len() <= MAX_REQUEST_BYTES
        && serde_json::from_str::<Header>(raw)
            .is_ok_and(|header| header.template == "site_provisioned_room")
}

impl Request {
    pub(crate) fn parse(raw: &str) -> Result<Self> {
        if raw.len() > MAX_REQUEST_BYTES {
            return Err(bounds("room site request exceeds 65536 bytes"));
        }
        let request: Self = serde_json::from_str(raw)
            .map_err(|error| invalid(format!("invalid room site request: {error}")))?;
        let [width, height] = request.origin_grid;
        if request.template != "site_provisioned_room" || request.version != 1 {
            return Err(invalid(
                "room siting requires site_provisioned_room version 1",
            ));
        }
        if width == 0 || height == 0 || u32::from(width) * u32::from(height) > MAX_ORIGINS {
            return Err(bounds(
                "room siting requires one to sixteen possible origins",
            ));
        }
        Ok(request)
    }

    /// Only the room origin moves. Workshop sites, labor choices and finite
    /// resource permissions always retain their original absolute meaning.
    fn room_at(&self, dx: u8, dy: u8) -> Result<ProvisionedRoomRequest> {
        let mut room = self.room.clone();
        let origin = match &mut room.layout {
            RoomLayout::BedroomCluster { origin, .. }
            | RoomLayout::OfficeCluster { origin, .. }
            | RoomLayout::Dormitory { origin, .. }
            | RoomLayout::DiningPairs { origin, .. }
            | RoomLayout::DiningHall { origin, .. } => origin,
        };
        origin[0] = origin[0]
            .checked_add(i32::from(dx))
            .ok_or_else(|| bounds("site x overflow"))?;
        origin[1] = origin[1]
            .checked_add(i32::from(dy))
            .ok_or_else(|| bounds("site y overflow"))?;
        Ok(room)
    }

    pub(crate) fn select<T>(
        &self,
        snapshot: &WorldSnapshot,
        check_budget: impl Fn() -> Result<()>,
        mut prepare: impl FnMut(&ProvisionedRoomRequest) -> Result<T>,
    ) -> Result<(T, Json)> {
        check_budget()?;
        let footprints = laboratory_building_footprints(snapshot)?;
        let mut work = 0_u64;
        let mut ranked = Vec::new();
        let [width, height] = self.origin_grid;
        for dy in 0..height {
            for dx in 0..width {
                check_budget()?;
                let room = self.room_at(dx, dy)?;
                let (origin, template) = room.layout.geometry();
                let layout = BlueprintPlanner.layout(origin, template)?;
                let regions = layout.excavations().len() + layout.office_walls().len();
                work =
                    work.saturating_add((regions as u64).saturating_mul(footprints.len() as u64));
                work = work.saturating_add(layout.tile_count());
                for wall in layout.office_walls() {
                    work = work.saturating_add(
                        wall.tile_count()
                            .ok_or_else(|| bounds("site wall area overflow"))?,
                    );
                }
                if work > MAX_GEOMETRY_WORK {
                    return Err(bounds(
                        "room site search exceeds its shared geometry work budget",
                    ));
                }
                if let Some(excavation_tiles) = excavation_cost(snapshot, &layout, &footprints) {
                    ranked.push((excavation_tiles, dy, dx));
                }
            }
        }
        // Ties prefer the lowest y and then the lowest x at the same z.
        ranked.sort_unstable();
        let mut first_refusal = None;
        let eligible = ranked.len();
        for (index, (cost, dy, dx)) in ranked.into_iter().enumerate() {
            check_budget()?;
            let room = self.room_at(dx, dy)?;
            match prepare(&room) {
                Ok(result) => {
                    let original: Json = serde_json::from_str(&room.canonical_json())
                        .map_err(|_| invariant("selected room lost its canonical source"))?;
                    let site = json!({
                        "policy":"fewest_excavation_tiles_then_y_x_v1",
                        "first_origin": self.room.layout.canonical_value()["origin"],
                        "origin_grid": self.origin_grid,
                        "origins_examined": u32::from(width) * u32::from(height),
                        "geometry_candidates": eligible,
                        "plans_examined": index + 1,
                        "tiles_requiring_excavation": cost,
                        "selected_room": original,
                    });
                    return Ok((result, site));
                }
                Err(error)
                    if matches!(
                        error.code,
                        ErrorCode::InvalidRequest
                            | ErrorCode::InvalidIntent
                            | ErrorCode::PreconditionsFailed
                    ) =>
                {
                    first_refusal.get_or_insert(error);
                }
                Err(error) => return Err(error),
            }
        }
        let error = precondition("no eligible room site in the requested origin grid");
        Err(match first_refusal {
            Some(cause) => error.with_detail("first_plan_refusal", cause.message),
            None => error,
        })
    }
}

fn excavation_cost(
    snapshot: &WorldSnapshot,
    layout: &BlueprintLayout,
    footprints: &BTreeMap<EntityId, MapCuboid>,
) -> Option<u64> {
    if layout
        .excavations()
        .iter()
        .map(|part| part.area)
        .chain(layout.office_walls().iter().copied())
        .any(|area| footprints.values().any(|building| overlap(*building, area)))
    {
        return None;
    }
    for wall in layout.office_walls() {
        if region_tiles(*wall)
            .any(|tile| snapshot.tile_code_at(tile) != Some(dfmcp_world::tile_codes::SOLID_WALL))
        {
            return None;
        }
    }
    let mut cost = 0;
    for part in layout.excavations() {
        let target = effects::dig_target_tile_code(part.mode);
        for tile in region_tiles(part.area) {
            let source = snapshot.tile_code_at(tile)?;
            if !effects::dig_transition_allowed(part.mode, source) {
                return None;
            }
            cost += u64::from(source != target);
        }
    }
    Some(cost)
}
