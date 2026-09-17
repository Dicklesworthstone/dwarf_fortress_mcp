//! Deterministic excavation geometry, not terrain or construction authority.

use dfmcp_core::{DfmcpError, ErrorCode, MapCoord, MapCuboid, Result};

use super::{BlueprintTemplate, checked_coord_offset, inclusive_span, invalid, rectangle};
use crate::DigMode;

pub const LAYOUT_POLICY: &str = "dfmcp.blueprint-layout/1";
pub const MAX_BLUEPRINT_ACTIONS: usize = 64;
pub const MAX_BLUEPRINT_TILES: u64 = 16_384;
pub const MAX_BLUEPRINT_ROOMS: u32 = 24;

/// Why a disjoint part of the layout is excavated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExcavationRole {
    Room,
    Doorway,
    Corridor,
    Perimeter,
}

/// An excavation proposal. A doorway is an opening, not a placed door.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlueprintExcavation {
    pub area: MapCuboid,
    pub mode: DigMode,
    pub role: ExcavationRole,
}

/// Bounded, nonoverlapping geometry. No observation, reservation or effect occurs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlueprintLayout {
    summary: String,
    excavations: Vec<BlueprintExcavation>,
    tile_count: u64,
    access_point: Option<MapCoord>,
    reserved_crossing: Option<MapCuboid>,
}

impl BlueprintLayout {
    #[must_use]
    pub fn summary(&self) -> &str {
        &self.summary
    }

    #[must_use]
    pub fn excavations(&self) -> &[BlueprintExcavation] {
        &self.excavations
    }

    #[must_use]
    pub const fn tile_count(&self) -> u64 {
        self.tile_count
    }

    /// Proposed corridor endpoint; connectivity to the existing fort is unproved.
    #[must_use]
    pub const fn access_point(&self) -> Option<MapCoord> {
        self.access_point
    }

    /// Unexcavated north-edge crossing. This does not construct a drawbridge.
    #[must_use]
    pub const fn reserved_crossing(&self) -> Option<MapCuboid> {
        self.reserved_crossing
    }

    pub fn generate(origin: MapCoord, template: BlueprintTemplate) -> Result<Self> {
        let mut layout = Self {
            summary: String::new(),
            excavations: Vec::new(),
            tile_count: 0,
            access_point: None,
            reserved_crossing: None,
        };
        match template {
            BlueprintTemplate::BedroomCluster { rooms_count, room_size } => {
                layout.cluster(origin, rooms_count, room_size, 4)?;
                layout.summary = format!("excavate {rooms_count} bedroom units");
            }
            BlueprintTemplate::WorkshopHub { bays_count } => {
                layout.cluster(origin, bays_count, (5, 5), MAX_BLUEPRINT_ROOMS)?;
                layout.summary = format!("excavate {bays_count} workshop bays");
            }
            BlueprintTemplate::DiningHall { width, height } => {
                layout.push(rectangle(origin, width, height)?, DigMode::Mine, ExcavationRole::Room)?;
                layout.summary = format!("excavate grand dining hall ({width}x{height})");
            }
            BlueprintTemplate::StockpileVault { width, height, category } => {
                if category.trim().is_empty() || category.len() > 128 || category.contains('\0') {
                    return Err(invalid("stockpile category must contain 1..128 UTF-8 bytes without NUL"));
                }
                layout.push(rectangle(origin, width, height)?, DigMode::Mine, ExcavationRole::Room)?;
                layout.summary = format!("stockpile vault ({category})");
            }
            BlueprintTemplate::DefensiveMoat { perimeter_cuboid, drawbridge_span } => {
                layout.moat(perimeter_cuboid, drawbridge_span)?;
                layout.summary = "excavate defensive moat".to_owned();
            }
        }
        layout.excavations.sort_by_key(|part| {
            let a = part.area;
            (a.min.z, a.min.y, a.min.x, a.max.z, a.max.y, a.max.x)
        });
        Ok(layout)
    }

    fn push(&mut self, area: MapCuboid, mode: DigMode, role: ExcavationRole) -> Result<()> {
        let count = area.tile_count().ok_or_else(|| bounds("blueprint tile count overflow"))?;
        let total = self.tile_count.checked_add(count).ok_or_else(|| bounds("blueprint tile count overflow"))?;
        if self.excavations.len() >= MAX_BLUEPRINT_ACTIONS || total > MAX_BLUEPRINT_TILES {
            return Err(bounds("blueprint exceeds 64 excavation parts or 16384 tiles"));
        }
        if self.excavations.iter().any(|part| overlaps(part.area, area)) {
            return Err(DfmcpError::new(ErrorCode::InternalInvariantViolation, "blueprint parts overlap"));
        }
        self.tile_count = total;
        self.excavations.push(BlueprintExcavation { area, mode, role });
        Ok(())
    }

    /// Rooms have one separating wall, a south doorway and a corridor per row.
    /// A west spine joins rows without opening any other room wall.
    fn cluster(&mut self, origin: MapCoord, count: u32, size: (u8, u8), columns: u32) -> Result<()> {
        if count == 0 || count > MAX_BLUEPRINT_ROOMS {
            return Err(bounds("blueprint room/bay count must be 1..24"));
        }
        super::validate_dimensions(size.0, size.1)?;
        let width = i64::from(size.0);
        let height = i64::from(size.1);
        let rows = (count - 1) / columns + 1;
        for row in 0..rows {
            let in_row = (count - row * columns).min(columns);
            let row_y = i64::from(row) * (height + 3);
            for column in 0..in_row {
                let room = offset(origin, i64::from(column) * (width + 1), row_y)?;
                self.push(rectangle(room, size.0, size.1)?, DigMode::Mine, ExcavationRole::Room)?;
                let doorway = offset(room, (width - 1) / 2, height)?;
                self.push(MapCuboid::new(doorway, doorway)?, DigMode::Mine, ExcavationRole::Doorway)?;
            }
            let left = offset(origin, -1, row_y + height + 1)?;
            let right = offset(origin, i64::from(in_row) * (width + 1) - 2, row_y + height + 1)?;
            self.push(MapCuboid::new(left, right)?, DigMode::Mine, ExcavationRole::Corridor)?;
        }
        let entrance = offset(origin, -2, height + 1)?;
        let end = offset(origin, -2, i64::from(rows - 1) * (height + 3) + height + 1)?;
        self.push(MapCuboid::new(entrance, end)?, DigMode::Mine, ExcavationRole::Corridor)?;
        self.access_point = Some(entrance);
        Ok(())
    }

    fn moat(&mut self, area: MapCuboid, gap: u8) -> Result<()> {
        if area.min.x > area.max.x || area.min.y > area.max.y || area.min.z != area.max.z {
            return Err(invalid("moat requires an ordered single-level perimeter"));
        }
        let width = inclusive_span(area.min.x, area.max.x);
        let height = inclusive_span(area.min.y, area.max.y);
        if width < 3 || height < 3 || u64::from(gap) > width - 2 {
            return Err(invalid("moat must enclose an interior and its crossing must preserve both north corners"));
        }
        // Compute the perimeter budget before constructing or scanning any strip.
        let perimeter = 2 * width + 2 * height - 4 - u64::from(gap);
        if perimeter > MAX_BLUEPRINT_TILES {
            return Err(bounds("moat exceeds the blueprint tile budget"));
        }
        let north_end = MapCoord { x: area.max.x, ..area.min };
        if gap == 0 {
            self.push(MapCuboid::new(area.min, north_end)?, DigMode::Channel, ExcavationRole::Perimeter)?;
        } else {
            // Center on the north edge; an odd spare tile goes on the east side.
            let start = offset(area.min, 1 + ((width - 2 - u64::from(gap)) / 2) as i64, 0)?;
            let end = offset(start, i64::from(gap) - 1, 0)?;
            self.reserved_crossing = Some(MapCuboid::new(start, end)?);
            self.push(MapCuboid::new(area.min, offset(start, -1, 0)?)?, DigMode::Channel, ExcavationRole::Perimeter)?;
            self.push(MapCuboid::new(offset(end, 1, 0)?, north_end)?, DigMode::Channel, ExcavationRole::Perimeter)?;
        }
        self.push(MapCuboid::new(
            offset(area.min, 0, 1)?, MapCoord { x: area.min.x, y: area.max.y - 1, z: area.min.z },
        )?, DigMode::Channel, ExcavationRole::Perimeter)?;
        self.push(MapCuboid::new(
            offset(north_end, 0, 1)?, MapCoord { y: area.max.y - 1, ..area.max },
        )?, DigMode::Channel, ExcavationRole::Perimeter)?;
        self.push(MapCuboid::new(MapCoord { x: area.min.x, ..area.max }, area.max)?, DigMode::Channel, ExcavationRole::Perimeter)?;
        Ok(())
    }
}

fn bounds(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

fn offset(origin: MapCoord, dx: i64, dy: i64) -> Result<MapCoord> {
    checked_coord_offset(origin, dx, dy, 0).ok_or_else(|| invalid("blueprint coordinate arithmetic overflow"))
}

fn overlaps(a: MapCuboid, b: MapCuboid) -> bool {
    a.min.x <= b.max.x && b.min.x <= a.max.x
        && a.min.y <= b.max.y && b.min.y <= a.max.y
        && a.min.z <= b.max.z && b.min.z <= a.max.z
}
