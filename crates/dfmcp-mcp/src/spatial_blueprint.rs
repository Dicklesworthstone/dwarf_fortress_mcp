//! Read-only blueprint geometry and coverage over one coherent spatial capture.
use std::collections::{BTreeMap, BTreeSet};

use dfmcp_adapter::live_map::LiveMapObservation;
use dfmcp_adapter::live_spatial::SpatialStateView;
use dfmcp_core::{Digest32, MapCoord, MapCuboid, OperationContext, Result};
use dfmcp_intent::blueprint::{BlueprintLayout, BlueprintPlanner, BlueprintTemplate, ExcavationRole};
use dfmcp_intent::blueprint::layout::LAYOUT_POLICY;
use dfmcp_intent::DigMode;
use dfmcp_world::map_region::Cell;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{base, budget, identity, invalid, paginate};

const MAX_WORK: u64 = 262_144;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    origin: [u32; 3],
    template: Template,
    limit: Option<u32>,
    continuation: Option<String>,
    max_work: Option<u64>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Template {
    BedroomCluster { rooms_count: u32, room_size: [u8; 2] },
    DiningHall { width: u8, height: u8 },
    WorkshopHub { bays_count: u32 },
    StockpileVault { width: u8, height: u8, category: String },
    DefensiveMoat { min: [u32; 3], max: [u32; 3], drawbridge_span: u8 },
}

fn coord(p: [u32; 3]) -> Result<MapCoord> {
    if p.iter().any(|v| *v >= 32_768) {
        return Err(invalid("blueprint input coordinates must be 0..32767"));
    }
    Ok(MapCoord { x: p[0] as i32, y: p[1] as i32, z: p[2] as i32 })
}

impl Template {
    fn into_template(self) -> Result<BlueprintTemplate> {
        Ok(match self {
            Self::BedroomCluster { rooms_count, room_size } => BlueprintTemplate::BedroomCluster {
                rooms_count, room_size: (room_size[0], room_size[1]),
            },
            Self::DiningHall { width, height } => BlueprintTemplate::DiningHall { width, height },
            Self::WorkshopHub { bays_count } => BlueprintTemplate::WorkshopHub { bays_count },
            Self::StockpileVault { width, height, category } => BlueprintTemplate::StockpileVault { width, height, category },
            Self::DefensiveMoat { min, max, drawbridge_span } => BlueprintTemplate::DefensiveMoat {
                perimeter_cuboid: MapCuboid::new(coord(min)?, coord(max)?)?, drawbridge_span,
            },
        })
    }
}

fn position(p: MapCoord) -> [i32; 3] { [p.x, p.y, p.z] }
fn area_json(a: MapCuboid) -> Value { json!({"min":position(a.min),"max":position(a.max)}) }

#[derive(Default)]
struct Coverage {
    visible: u64,
    hidden: u64,
    unallocated: u64,
    outside_capture: u64,
    outside_map: u64,
    liquid: u64,
    magma_liquid: u64,
    building_occupied: u64,
    unit_occupied: u64,
    designated: u64,
    shapes: BTreeMap<&'static str, u64>,
}

impl Coverage {
    fn observe(&mut self, map: &LiveMapObservation, p: [i32; 3]) -> Result<()> {
        if (0..3).any(|axis| p[axis] < 0 || p[axis] as u32 >= map.map_dimensions[axis]) {
            self.outside_map += 1;
            return Ok(());
        }
        let p = [p[0] as u32, p[1] as u32, p[2] as u32];
        let Some(index) = map.map.region.index(p) else {
            self.outside_capture += 1;
            return Ok(());
        };
        match map.map.cells.get(index).ok_or_else(|| invalid("blueprint capture cell absent"))? {
            Cell::Hidden => self.hidden += 1,
            Cell::Unallocated => self.unallocated += 1,
            Cell::Visible(tile) => {
                self.visible += 1;
                self.liquid += u64::from(tile.liquid_depth > 0);
                self.magma_liquid += u64::from(tile.liquid_depth > 0 && tile.magma);
                self.building_occupied += u64::from(tile.building_occupancy > 0);
                self.unit_occupied += u64::from(tile.unit_occupancy > 0);
                self.designated += u64::from(tile.dig_designation > 0);
                *self.shapes.entry(tile.shape.name()).or_default() += 1;
            }
        }
        Ok(())
    }

    fn complete(&self) -> bool {
        self.hidden + self.unallocated + self.outside_capture + self.outside_map == 0
    }

    fn json(&self) -> Value {
        json!({"visible":self.visible,"hidden":self.hidden,"unallocated":self.unallocated,
            "outside_capture":self.outside_capture,"outside_map":self.outside_map,
            "all_positions_visible":self.complete(),
            "visible_attributes":{"liquid_tiles":self.liquid,"magma_liquid_tiles":self.magma_liquid,
                "building_occupied_tiles":self.building_occupied,"unit_occupied_tiles":self.unit_occupied,
                "existing_designation_tiles":self.designated,"shapes":self.shapes}})
    }
}

fn halo(a: MapCuboid) -> Result<MapCuboid> {
    let min = MapCoord { x: a.min.x.checked_sub(1).ok_or_else(|| budget("blueprint halo overflow"))?,
        y: a.min.y.checked_sub(1).ok_or_else(|| budget("blueprint halo overflow"))?,
        z: a.min.z.checked_sub(1).ok_or_else(|| budget("blueprint halo overflow"))? };
    let max = MapCoord { x: a.max.x.checked_add(1).ok_or_else(|| budget("blueprint halo overflow"))?,
        y: a.max.y.checked_add(1).ok_or_else(|| budget("blueprint halo overflow"))?,
        z: a.max.z.checked_add(1).ok_or_else(|| budget("blueprint halo overflow"))? };
    MapCuboid::new(min, max)
}

fn visit(a: MapCuboid, mut f: impl FnMut([i32; 3]) -> Result<()>) -> Result<()> {
    for z in a.min.z..=a.max.z {
        for y in a.min.y..=a.max.y {
            for x in a.min.x..=a.max.x { f([x, y, z])?; }
        }
    }
    Ok(())
}

fn analyze(map: &LiveMapObservation, layout: &BlueprintLayout, maximum_work: u64) -> Result<(Value, Vec<Value>)> {
    if maximum_work == 0 || maximum_work > MAX_WORK {
        return Err(budget("blueprint max_work must be 1..262144"));
    }
    // Conservative visit bound: validation + row/total reads + halo insertions and
    // reads. Repeated halo positions are charged even though reads deduplicate.
    let mut cost = map.map.cells.len() as u64 + 2 * layout.tile_count();
    let mut halos = Vec::new();
    for part in layout.excavations() {
        let expanded = halo(part.area)?;
        let count = expanded.tile_count().ok_or_else(|| budget("blueprint halo size overflow"))?;
        cost = count.checked_mul(2).and_then(|n| cost.checked_add(n))
            .ok_or_else(|| budget("blueprint scan cost overflow"))?;
        halos.push(expanded);
    }
    if let Some(crossing) = layout.reserved_crossing() {
        cost = cost.checked_add(crossing.tile_count().ok_or_else(|| budget("crossing size overflow"))?)
            .ok_or_else(|| budget("blueprint scan cost overflow"))?;
    }
    if cost > maximum_work { return Err(budget("blueprint terrain analysis exceeds max_work")); }
    map.validate()?;
    let mut footprint = Coverage::default();
    let mut rows = Vec::with_capacity(layout.excavations().len());
    for (index, part) in layout.excavations().iter().enumerate() {
        let mut coverage = Coverage::default();
        visit(part.area, |p| { coverage.observe(map, p)?; footprint.observe(map, p) })?;
        let mode = match part.mode {
            DigMode::Mine => "mine", DigMode::Channel => "channel",
            _ => return Err(invalid("unsupported blueprint excavation mode")),
        };
        let role = match part.role {
            ExcavationRole::Room => "room", ExcavationRole::Doorway => "doorway",
            ExcavationRole::Corridor => "corridor", ExcavationRole::Perimeter => "perimeter",
        };
        rows.push(json!({"part_index":index,"role":role,"dig_mode":mode,
            "area":area_json(part.area),"tile_count":part.area.tile_count(),"coverage":coverage.json()}));
    }
    let mut positions = BTreeSet::new();
    for area in halos { visit(area, |p| { positions.insert(p); Ok(()) })?; }
    let mut surrounding = Coverage::default();
    for p in &positions { surrounding.observe(map, *p)?; }
    let crossing = if let Some(area) = layout.reserved_crossing() {
        let mut coverage = Coverage::default();
        visit(area, |p| coverage.observe(map, p))?;
        json!({"area":area_json(area),"coverage":coverage.json(),"bridge_constructed":false,"reservation_created":false})
    } else { Value::Null };
    let summary = json!({"label":layout.summary(),"excavated_tiles":layout.tile_count(),
        "part_count":layout.excavations().len(),"footprint":footprint.json(),
        "halo":{"unique_positions":positions.len(),"coverage":surrounding.json()},
        "access_point":layout.access_point().map(position),"reserved_crossing":crossing,
        "scan_work_upper_bound":cost});
    Ok((summary, rows))
}

pub(super) fn execute<T: SpatialStateView>(state: &T, c: &OperationContext, source: Digest32, request: Request) -> Result<Value> {
    let origin = coord(request.origin)?;
    let layout = BlueprintPlanner.layout(origin, request.template.into_template()?)?;
    let maximum = request.max_work.unwrap_or(MAX_WORK);
    let map = state.spatial_observation().ok_or_else(|| invalid("spatial source absent"))?.terrain();
    let (summary, rows) = analyze(map, &layout, maximum)?;
    let id = identity(c, source, json!({"kind":"blueprint_layout","policy":LAYOUT_POLICY,
        "origin":request.origin,"max_work":maximum,"summary":summary,"parts":rows}));
    let mut out = base(c, source, "blueprint_layout");
    out["layout_policy"] = json!(LAYOUT_POLICY);
    out["summary"] = summary;
    out["status"] = json!("geometry_preview_only");
    out["plan_created"] = json!(false);
    out["excavation_eligibility_proven"] = json!(false);
    out["completion_proven"] = json!(false);
    out["unknown_safety_domains"] = json!(["aquifers","water_pressure","structural_support",
        "protected_areas","native_designation_eligibility","unit_access"]);
    out["interpretation"] = json!("Disjoint proposed geometry and visible attributes from this capture, not excavation permission, a safe site, a built room/bridge, or proof of completed work. Hidden and uncaptured positions remain unknown; a corridor endpoint is not an existing-fort connection.");
    paginate(out, rows.len(), request.continuation.as_deref(), request.limit.unwrap_or(8), id, c,
        |index| Ok(rows[index].clone()))
}

#[cfg(test)]
#[path = "spatial_blueprint_tests.rs"]
mod tests;
