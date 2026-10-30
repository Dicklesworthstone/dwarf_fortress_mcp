//! A complete declared room condition, retained before original furniture placement.
//!
//! This deliberately narrow Rust generation binds one floor/wall selection and
//! the full existing furniture request. It is not the Python RoomPlan format,
//! a room-layout compiler, observed terrain, or native room-assignment authority.
//! Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5.

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, Result};
use dfmcp_world::map_region::Region;

use crate::furniture_handoff::{FurnitureRequest, MAX_REQUEST_BYTES};

pub const MAX_ROOM_CELLS: usize = 4096;
pub const MAX_TERRAIN_BYTES: usize = 8 + 24 + 2 + 2 * (MAX_ROOM_CELLS / 8);
pub const MAX_ROOM_INTENT_BYTES: usize = 8 + 4 + MAX_REQUEST_BYTES + 4 + MAX_TERRAIN_BYTES;
const TERRAIN_MAGIC: &[u8; 8] = b"DFMRTN01";
const ROOM_MAGIC: &[u8; 8] = b"DFMRIN01";

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}
fn require(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(invalid(message))
    }
}
fn volume(region: Region) -> Result<usize> {
    let count = region
        .volume()
        .map_err(|_| invalid("room terrain region is invalid"))?;
    require(
        region.size[2] == 1 && count <= MAX_ROOM_CELLS,
        "room terrain requires one level and at most 4096 cells",
    )?;
    Ok(count)
}
fn field(out: &mut Vec<u8>, value: &[u8]) {
    out.extend_from_slice(&(value.len() as u32).to_be_bytes());
    out.extend_from_slice(value);
}

/// Immutable complete terrain requirements within one exact capture region.
///
/// A set bit selects a required cell. Bit `i` names `Region::position(i)`, with
/// x changing fastest, and uses bit `i % 8` in byte `i / 8`. Unused high bits
/// are zero. Unselected cells make no terrain-condition claim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerrainRequirements {
    region: Region,
    floors: Vec<[u32; 3]>,
    walls: Vec<[u32; 3]>,
    floor_mask: Vec<u8>,
    wall_mask: Vec<u8>,
    bytes: Vec<u8>,
}
impl TerrainRequirements {
    /// Input order is immaterial; repeated coordinates are invalid rather than
    /// silently removed. All raw cardinalities are bounded before projection.
    pub fn new(region: Region, floors: Vec<[u32; 3]>, walls: Vec<[u32; 3]>) -> Result<Self> {
        let count = volume(region)?;
        require(
            !floors.is_empty()
                && floors.len() <= count
                && walls.len() <= count
                && floors
                    .len()
                    .checked_add(walls.len())
                    .is_some_and(|n| n <= count),
            "room floor and wall selections exceed their complete region",
        )?;
        let mut floor_mask = vec![0u8; count.div_ceil(8)];
        let mut wall_mask = vec![0u8; count.div_ceil(8)];
        for (positions, mask) in [(&floors, &mut floor_mask), (&walls, &mut wall_mask)] {
            for &position in positions {
                let index = region
                    .index(position)
                    .ok_or_else(|| invalid("room terrain requirement lies outside its region"))?;
                let bit = 1u8 << (index % 8);
                require(
                    mask[index / 8] & bit == 0,
                    "duplicate room terrain requirement",
                )?;
                mask[index / 8] |= bit;
            }
        }
        Self::from_masks(region, floor_mask, wall_mask)
    }

    /// Decode a compact declared selection without accepting padding, aliases,
    /// overlapping meanings, or an empty floor condition.
    pub fn from_masks(region: Region, floor_mask: Vec<u8>, wall_mask: Vec<u8>) -> Result<Self> {
        let count = volume(region)?;
        let length = count.div_ceil(8);
        require(
            floor_mask.len() == length && wall_mask.len() == length,
            "room terrain masks must exactly cover the declared region",
        )?;
        require(
            floor_mask.iter().any(|byte| *byte != 0)
                && floor_mask
                    .iter()
                    .zip(&wall_mask)
                    .all(|(floor, wall)| floor & wall == 0),
            "room terrain requires nonempty floors disjoint from its walls",
        )?;
        if count % 8 != 0 {
            let permitted = (1u8 << (count % 8)) - 1;
            require(
                floor_mask[length - 1] & !permitted == 0 && wall_mask[length - 1] & !permitted == 0,
                "room terrain masks have nonzero unused bits",
            )?;
        }
        let mut floors = Vec::new();
        let mut walls = Vec::new();
        for index in 0..count {
            let bit = 1u8 << (index % 8);
            if floor_mask[index / 8] & bit != 0 {
                floors.push(
                    region
                        .position(index)
                        .ok_or_else(|| invalid("invalid room floor position"))?,
                );
            } else if wall_mask[index / 8] & bit != 0 {
                walls.push(
                    region
                        .position(index)
                        .ok_or_else(|| invalid("invalid room wall position"))?,
                );
            }
        }
        let mut bytes = TERRAIN_MAGIC.to_vec();
        for value in region.origin.into_iter().chain(region.size) {
            bytes.extend_from_slice(&value.to_be_bytes());
        }
        bytes.extend_from_slice(&(length as u16).to_be_bytes());
        bytes.extend_from_slice(&floor_mask);
        bytes.extend_from_slice(&wall_mask);
        Ok(Self {
            region,
            floors,
            walls,
            floor_mask,
            wall_mask,
            bytes,
        })
    }

    pub fn decode(raw: &[u8]) -> Result<Self> {
        require(
            raw.len() <= MAX_TERRAIN_BYTES,
            "room terrain bytes exceed bound",
        )?;
        let mut reader = Reader(raw);
        require(
            reader.take(8)? == TERRAIN_MAGIC,
            "unsupported room terrain generation",
        )?;
        let region = Region {
            origin: [reader.u32()?, reader.u32()?, reader.u32()?],
            size: [reader.u32()?, reader.u32()?, reader.u32()?],
        };
        let count = volume(region)?;
        let length = u16::from_be_bytes(reader.array()?) as usize;
        require(
            length == count.div_ceil(8),
            "noncanonical room terrain mask length",
        )?;
        let floors = reader.take(length)?.to_vec();
        let walls = reader.take(length)?.to_vec();
        require(reader.0.is_empty(), "trailing room terrain bytes")?;
        let value = Self::from_masks(region, floors, walls)?;
        require(
            value.canonical_bytes() == raw,
            "noncanonical room terrain bytes",
        )?;
        Ok(value)
    }

    pub fn region(&self) -> Region {
        self.region
    }
    /// Required coordinates in x-fastest row-major order.
    pub fn floors(&self) -> &[[u32; 3]] {
        &self.floors
    }
    /// Required coordinates in x-fastest row-major order.
    pub fn walls(&self) -> &[[u32; 3]] {
        &self.walls
    }
    pub fn floor_mask(&self) -> &[u8] {
        &self.floor_mask
    }
    pub fn wall_mask(&self) -> &[u8] {
        &self.wall_mask
    }
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Original furniture intent plus every declared floor and wall requirement.
/// Room-only changes alter this identity even when the selected furniture does
/// not change. Every requested furniture target must be a required floor cell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomIntent {
    furniture: FurnitureRequest,
    terrain: TerrainRequirements,
    bytes: Vec<u8>,
    digest: Digest32,
}
impl RoomIntent {
    pub fn new(furniture: FurnitureRequest, terrain: TerrainRequirements) -> Result<Self> {
        require(
            furniture.request().slots.iter().all(|slot| {
                terrain
                    .region
                    .index(slot.target)
                    .is_some_and(|index| terrain.floor_mask[index / 8] & (1u8 << (index % 8)) != 0)
            }),
            "every original furnishing target must be a required room floor",
        )?;
        let mut bytes = ROOM_MAGIC.to_vec();
        field(&mut bytes, furniture.canonical_bytes());
        field(&mut bytes, terrain.canonical_bytes());
        require(
            bytes.len() <= MAX_ROOM_INTENT_BYTES,
            "room intent exceeds bound",
        )?;
        let mut digest_input = b"dfmcp-room-intent-rust/1\0".to_vec();
        digest_input.extend_from_slice(&bytes);
        let digest = Digest32::of_bytes(&digest_input);
        Ok(Self {
            furniture,
            terrain,
            bytes,
            digest,
        })
    }
    pub fn decode(raw: &[u8]) -> Result<Self> {
        require(
            raw.len() <= MAX_ROOM_INTENT_BYTES,
            "room intent bytes exceed bound",
        )?;
        let mut reader = Reader(raw);
        require(
            reader.take(8)? == ROOM_MAGIC,
            "unsupported room intent generation",
        )?;
        let furniture = FurnitureRequest::decode(reader.field(MAX_REQUEST_BYTES)?)?;
        let terrain = TerrainRequirements::decode(reader.field(MAX_TERRAIN_BYTES)?)?;
        require(reader.0.is_empty(), "trailing room intent bytes")?;
        let value = Self::new(furniture, terrain)?;
        require(
            value.canonical_bytes() == raw,
            "noncanonical room intent bytes",
        )?;
        Ok(value)
    }
    pub fn furniture(&self) -> &FurnitureRequest {
        &self.furniture
    }
    pub fn terrain(&self) -> &TerrainRequirements {
        &self.terrain
    }
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn digest(&self) -> Digest32 {
        self.digest
    }
}

struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        require(length <= self.0.len(), "truncated room intent bytes")?;
        let (value, rest) = self.0.split_at(length);
        self.0 = rest;
        Ok(value)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.take(N)?
            .try_into()
            .map_err(|_| invalid("invalid room intent field"))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.array()?))
    }
    fn field(&mut self, maximum: usize) -> Result<&'a [u8]> {
        let length = self.u32()? as usize;
        require(length <= maximum, "room intent field exceeds bound")?;
        self.take(length)
    }
}

#[cfg(test)]
mod tests;
