//! Immutable declared room geometry travels with the complete allocated request.
//! Intake observes inventory only; it does not establish any terrain condition.
use super::{Config, allocation, batch, error};
use dfmcp_adapter::furniture_batch::FurniturePlan;
use dfmcp_adapter::furniture_handoff::FurnitureRequest;
use dfmcp_adapter::room_intent::{RoomIntent, TerrainRequirements};
use dfmcp_core::{ErrorCode, Result};
use dfmcp_world::map_region::Region;
use serde::Deserialize;
use serde_json::{Value, json};

pub(super) const SCHEMA: &str = "dfmcp.room-construction-request/1";
const MAX_INPUT_BYTES: usize = 128 * 1024;

fn invalid() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::InvalidRequest,
        "invalid bounded room construction request",
    )
}

pub(super) fn exclusive(inputs: [bool; 4]) -> Result<()> {
    if inputs.into_iter().filter(|present| *present).count() > 1 {
        return Err(error(
            ErrorCode::InvalidRequest,
            "furniture and room intent inputs are mutually exclusive",
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegionInput {
    origin: [u32; 3],
    size: [u32; 3],
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TerrainInput {
    region: RegionInput,
    floors: Vec<[u32; 3]>,
    walls: Vec<[u32; 3]>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    schema: String,
    terrain: TerrainInput,
    // The exact raw value is passed to FurnitureRequest's existing strict
    // decoder below. Ignoring it here cannot erase duplicate nested fields or
    // normalize a floating-point identifier before that decoder sees it.
    furniture: serde::de::IgnoredAny,
}

fn shape(raw: &[u8]) -> Result<()> {
    if raw.is_empty() || raw.len() > MAX_INPUT_BYTES || raw.contains(&0) {
        return Err(invalid());
    }
    let (mut depth, mut quoted, mut escaped) = (0usize, false, false);
    for &byte in raw {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
        } else {
            match byte {
                b'"' => quoted = true,
                b'{' | b'[' => {
                    depth += 1;
                    if depth > 16 {
                        return Err(invalid());
                    }
                }
                b'}' | b']' => depth = depth.checked_sub(1).ok_or_else(invalid)?,
                _ => {}
            }
        }
    }
    if depth != 0 || quoted {
        return Err(invalid());
    }
    Ok(())
}

fn whitespace(raw: &[u8], at: &mut usize) {
    while raw.get(*at).is_some_and(u8::is_ascii_whitespace) {
        *at += 1;
    }
}
fn string_end(raw: &[u8], start: usize) -> Result<usize> {
    if raw.get(start) != Some(&b'"') {
        return Err(invalid());
    }
    let mut escaped = false;
    for (index, &byte) in raw.iter().enumerate().skip(start + 1) {
        if escaped {
            escaped = false;
        } else if byte == b'\\' {
            escaped = true;
        } else if byte == b'"' {
            return Ok(index + 1);
        }
    }
    Err(invalid())
}
fn value_end(raw: &[u8], start: usize) -> Result<usize> {
    let mut at = start;
    let mut depth = 0usize;
    while let Some(&byte) = raw.get(at) {
        match byte {
            b'"' => {
                at = string_end(raw, at)?;
                continue;
            }
            b'{' | b'[' => depth += 1,
            b'}' | b']' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    return Ok(at + 1);
                }
            }
            b',' | b'}' | b']' if depth == 0 => return Ok(at),
            _ => {}
        }
        at += 1;
    }
    Err(invalid())
}
/// The enclosing closed object has already passed serde syntax and duplicate
/// checks. Locate its original nested bytes without reparsing them into Value.
fn furniture_bytes(raw: &[u8]) -> Result<&[u8]> {
    let mut at = 0;
    whitespace(raw, &mut at);
    if raw.get(at) != Some(&b'{') {
        return Err(invalid());
    }
    at += 1;
    loop {
        whitespace(raw, &mut at);
        let end = string_end(raw, at)?;
        let name: String = serde_json::from_slice(&raw[at..end]).map_err(|_| invalid())?;
        at = end;
        whitespace(raw, &mut at);
        if raw.get(at) != Some(&b':') {
            return Err(invalid());
        }
        at += 1;
        whitespace(raw, &mut at);
        let end = value_end(raw, at)?;
        if name == "furniture" {
            return Ok(&raw[at..end]);
        }
        at = end;
        whitespace(raw, &mut at);
        if raw.get(at) != Some(&b',') {
            return Err(invalid());
        }
        at += 1;
    }
}

pub(super) fn parse(raw: &str) -> Result<RoomIntent> {
    shape(raw.as_bytes())?;
    let input: Input = serde_json::from_str(raw).map_err(|_| invalid())?;
    if input.schema != SCHEMA {
        return Err(invalid());
    }
    let _ = input.furniture;
    let furniture = FurnitureRequest::decode(furniture_bytes(raw.as_bytes())?)?;
    let terrain = TerrainRequirements::new(
        Region {
            origin: input.terrain.region.origin,
            size: input.terrain.region.size,
        },
        input.terrain.floors,
        input.terrain.walls,
    )?;
    RoomIntent::new(furniture, terrain)
}

pub(super) fn validate(room: &RoomIntent, config: &Config) -> Result<()> {
    allocation::validate_request(room.furniture(), config)?;
    let region = room.terrain().region();
    let low = [config.scope.min.x, config.scope.min.y, config.scope.min.z];
    let high = [config.scope.max.x, config.scope.max.y, config.scope.max.z];
    if (0..3).any(|axis| {
        i64::from(region.origin[axis]) < i64::from(low[axis])
            || i64::from(region.origin[axis]) + i64::from(region.size[axis]) - 1
                > i64::from(high[axis])
    }) {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "complete room terrain region exceeds operator scope",
        ));
    }
    Ok(())
}

pub(super) fn validate_dimensions(room: &RoomIntent, dimensions: [u32; 3]) -> Result<()> {
    let region = room.terrain().region();
    if (0..3).any(|axis| {
        u64::from(region.origin[axis]) + u64::from(region.size[axis]) > u64::from(dimensions[axis])
    }) {
        return Err(error(
            ErrorCode::StaleAnchor,
            "complete room terrain region exceeds the observed map",
        ));
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[usize::from(byte >> 4)] as char);
        out.push(DIGITS[usize::from(byte & 15)] as char);
    }
    out
}

pub(super) fn display(room: &RoomIntent) -> Value {
    let request = room.furniture();
    let slots = request
        .request()
        .slots
        .iter()
        .map(|slot| {
            let mut row = json!({"name":slot.name,"kind":slot.kind.as_str(),"target":slot.target,
            "after":slot.after,"max_distance":slot.max_distance});
            if let Some(material) = slot.material {
                row["material"] = json!([material.0, material.1]);
            }
            if let Some(subtype) = slot.subtype {
                row["subtype"] = json!(subtype);
            }
            row
        })
        .collect::<Vec<_>>();
    let terrain = room.terrain();
    let region = terrain.region();
    json!({"schema":"dfmcp.room-intent/1","room_intent_digest":room.digest().to_string(),
        "furniture_request_digest":request.digest().to_string(),
        "furniture_request":{"schema":"dfmcp.furniture-request/1","world_folder":request.folder(),
            "site":request.site(),"slots":slots,"excluded_items":request.request().excluded_items},
        "terrain":{"region":{"origin":region.origin,"size":region.size},
            "mask_encoding":"x-fastest-row-major-lsb0-hex","floor_mask":hex(terrain.floor_mask()),
            "wall_mask":hex(terrain.wall_mask()),"floor_tiles":terrain.floors().len(),"wall_tiles":terrain.walls().len()},
        "complete_declared_room_intent_retained":true,"terrain_observed":false,
        "room_condition_sampled":false,"room_completion_proven":false,
        "room_assignment_created":false,"native_pathfinding_proven":false})
}

pub(super) fn reserve_output(plan: &FurniturePlan, room: Option<&RoomIntent>) -> Result<()> {
    let extra = room.map_or(0, |room| display(room).to_string().len() + 32);
    batch::reserve_output_with_extra(plan, extra)
}
