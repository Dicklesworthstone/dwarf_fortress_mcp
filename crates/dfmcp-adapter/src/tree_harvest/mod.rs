#![forbid(unsafe_code)]
//! Closed tree/1.21 evidence. A root coordinate is a predicate, never a plant ID.
pub mod journal;
pub mod rpc;
#[cfg(test)]
pub(crate) mod tests;
use crate::bounded_run::{Reader, error, hash, require, validate_key};
pub use crate::order_run::FortressIdentity;
use dfmcp_core::{Digest32, ErrorCode, MapCoord, MapCuboid, Result};

pub const MAX_CAPTURE_BYTES: usize = 128 * 1024;
pub const MAX_RECORD_BYTES: usize = 384 * 1024;
pub const MAX_CELLS: usize = 2048;
pub const MAX_SAMPLES: u32 = 16;
pub(crate) fn number16(r: &mut Reader<'_>) -> Result<u16> {
    Ok(u16::from_be_bytes(r.array()?))
}
pub(crate) fn text(r: &mut Reader<'_>, max: usize) -> Result<String> {
    let n = usize::from(number16(r)?);
    require(n > 0 && n <= max, "invalid tree text bound")?;
    let value = std::str::from_utf8(r.take(n)?)
        .map_err(|_| error(ErrorCode::AdapterRejected, "invalid tree UTF-8"))?;
    require(!value.contains('\0'), "NUL in tree text")?;
    Ok(value.to_owned())
}
pub(crate) fn field(out: &mut Vec<u8>, s: &[u8]) {
    out.extend_from_slice(&(s.len() as u16).to_be_bytes());
    out.extend_from_slice(s);
}
pub(crate) fn blob(out: &mut Vec<u8>, s: &[u8]) {
    out.extend_from_slice(&(s.len() as u32).to_be_bytes());
    out.extend_from_slice(s);
}
pub(crate) fn read_blob<'a>(r: &mut Reader<'a>, max: usize) -> Result<&'a [u8]> {
    let n = r.u32()? as usize;
    require(n <= max, "tree blob bound")?;
    r.take(n)
}
fn coord(r: &mut Reader<'_>) -> Result<[u32; 3]> {
    let p = [r.u32()?, r.u32()?, r.u32()?];
    require(p.iter().all(|v| *v < 32768), "tree coordinate bound")?;
    Ok(p)
}
fn put_coord(out: &mut Vec<u8>, p: [u32; 3]) {
    for n in p {
        out.extend_from_slice(&n.to_be_bytes());
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    pub position: [u32; 3],
    pub presence: u8,
    pub tile_type: u32,
    pub designation_other: u32,
    pub occupancy: u32,
    pub block_other: u32,
    pub temperature: [u16; 2],
    pub dig: u8,
    pub liquid: u8,
    pub designated: bool,
}
impl Cell {
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        let position = coord(r)?;
        let presence = r.byte()?;
        require(presence <= 2, "invalid tree cell presence")?;
        let mut out = Self {
            position,
            presence,
            tile_type: 0,
            designation_other: 0,
            occupancy: 0,
            block_other: 0,
            temperature: [0; 2],
            dig: 0,
            liquid: 0,
            designated: false,
        };
        if presence == 2 {
            out.tile_type = r.u32()?;
            out.designation_other = r.u32()?;
            out.occupancy = r.u32()?;
            out.block_other = r.u32()?;
            out.temperature = [number16(r)?, number16(r)?];
            out.dig = r.byte()?;
            out.liquid = r.byte()?;
            out.designated = r.boolean()?;
            require(out.dig <= 7 && out.liquid <= 7, "invalid tree cell enum")?;
        }
        Ok(out)
    }
    fn append(&self, out: &mut Vec<u8>) {
        put_coord(out, self.position);
        out.push(self.presence);
        if self.presence == 2 {
            for n in [
                self.tile_type,
                self.designation_other,
                self.occupancy,
                self.block_other,
            ] {
                out.extend_from_slice(&n.to_be_bytes());
            }
            for n in self.temperature {
                out.extend_from_slice(&n.to_be_bytes());
            }
            out.extend_from_slice(&[self.dig, self.liquid, u8::from(self.designated)]);
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worker {
    pub id: u32,
    pub position: [u32; 3],
    pub facts: [bool; 7],
}
impl Worker {
    pub fn eligible(&self) -> bool {
        self.facts.iter().all(|v| *v)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tool {
    pub id: u32,
    pub flags: u32,
    pub flags2: u32,
    pub holder: u32,
    pub role: i32,
    pub position: [u32; 3],
    pub facts: [bool; 4],
}
impl Tool {
    pub fn offered(&self) -> bool {
        self.facts.iter().all(|v| *v)
            && self.flags2 == 0
            && self.flags & !((1 << 14) | (1 << 28) | (1 << 29)) == 0
            && self.role == 0
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capture {
    raw: Vec<u8>,
    pub generation: u64,
    pub sequence: u64,
    pub tick: u64,
    pub source: FortressIdentity,
    pub dimensions: [u32; 3],
    pub next_item: u32,
    pub root: [u32; 3],
    pub root_presence: u8,
    pub species: i32,
    pub size: [u32; 3],
    pub designation: [u32; 3],
    pub paused: bool,
    pub mature: bool,
    pub alive: bool,
    pub complete: bool,
    pub existing_job: bool,
    pub body: Vec<u16>,
    pub cells: Vec<Cell>,
    pub workers: Vec<Worker>,
    pub tools: Vec<Tool>,
}
impl Capture {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        require(bytes.len() <= MAX_CAPTURE_BYTES, "tree capture bound")?;
        let mut r = Reader(bytes);
        require(r.take(8)? == b"DFMTC021", "not tree/1.21 evidence")?;
        let generation = r.u64()?;
        let sequence = r.u64()?;
        let tick = r.u64()?;
        let site = r.u32()?;
        let dimensions = [r.u32()?, r.u32()?, r.u32()?];
        let next_item = r.u32()?;
        let source = FortressIdentity::new(&text(&mut r, 512)?, site)?;
        let root = coord(&mut r)?;
        let root_presence = r.byte()?;
        let species = r.u32()? as i32;
        let size = [r.u32()?, r.u32()?, r.u32()?];
        let designation = coord(&mut r)?;
        let paused = r.boolean()?;
        let mature = r.boolean()?;
        let alive = r.boolean()?;
        let complete = r.boolean()?;
        let existing_job = r.boolean()?;
        require(
            generation > 0
                && generation < u64::MAX
                && sequence < u64::MAX
                && tick <= crate::bounded_run::MAX_NATIVE_TICK
                && dimensions.iter().all(|d| (1..=32768).contains(d))
                && root_presence <= 2
                && next_item <= i32::MAX as u32
                && (0..3).all(|i| root[i] < dimensions[i]),
            "invalid tree source fields",
        )?;
        let n = usize::from(number16(&mut r)?);
        require(n <= MAX_CELLS, "tree body bound")?;
        let mut body = Vec::with_capacity(n);
        for _ in 0..n {
            body.push(number16(&mut r)?);
        }
        let n = usize::from(number16(&mut r)?);
        require(n <= MAX_CELLS, "tree cell bound")?;
        let mut cells = Vec::with_capacity(n);
        for _ in 0..n {
            cells.push(Cell::read(&mut r)?);
        }
        let n = r.byte()?;
        require(n <= 32, "tree worker bound")?;
        let mut workers = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let id = r.u32()?;
            let position = coord(&mut r)?;
            let mut facts = [false; 7];
            for v in &mut facts {
                *v = r.boolean()?;
            }
            require(
                id < i32::MAX as u32 && workers.last().is_none_or(|w: &Worker| w.id < id),
                "tree worker order",
            )?;
            workers.push(Worker {
                id,
                position,
                facts,
            });
        }
        let n = r.byte()?;
        require(n <= 8, "tree tool bound")?;
        let mut tools = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let id = r.u32()?;
            let flags = r.u32()?;
            let flags2 = r.u32()?;
            let holder = r.u32()?;
            let role = r.u32()? as i32;
            let position = coord(&mut r)?;
            let mut facts = [false; 4];
            for v in &mut facts {
                *v = r.boolean()?;
            }
            require(
                id < i32::MAX as u32
                    && holder < i32::MAX as u32
                    && (-1..=2).contains(&role)
                    && tools.last().is_none_or(|t: &Tool| t.id < id),
                "invalid tree tool",
            )?;
            tools.push(Tool {
                id,
                flags,
                flags2,
                holder,
                role,
                position,
                facts,
            });
        }
        r.finish()?;
        let out = Self {
            raw: bytes.to_vec(),
            generation,
            sequence,
            tick,
            source,
            dimensions,
            next_item,
            root,
            root_presence,
            species,
            size,
            designation,
            paused,
            mature,
            alive,
            complete,
            existing_job,
            body,
            cells,
            workers,
            tools,
        };
        require(out.encode() == bytes, "noncanonical tree capture")?;
        out.geometry()?;
        Ok(out)
    }
    fn geometry(&self) -> Result<()> {
        if !self.mature {
            return require(
                self.body.is_empty() && self.size == [0; 3],
                "non-tree body payload",
            );
        }
        require(
            self.root_presence == 2 && self.size.iter().all(|v| (1..=64).contains(v)),
            "invalid mature geometry",
        )?;
        let volume = self
            .size
            .iter()
            .try_fold(1u64, |a, b| a.checked_mul(u64::from(*b)))
            .ok_or_else(|| error(ErrorCode::AdapterRejected, "tree volume overflow"))?;
        require(volume <= MAX_CELLS as u64, "oversized tree geometry")?;
        if self.body.is_empty() {
            return require(
                !self.complete && self.cells.iter().any(|c| c.presence != 2),
                "unobserved tree body payload",
            );
        }
        require(volume as usize == self.body.len(), "incomplete tree body")?;
        let [w, h, z] = self.size;
        let x1 = self.root[0].checked_sub(w / 2).filter(|n| *n > 0);
        let y1 = self.root[1].checked_sub(h / 2).filter(|n| *n > 0);
        let (Some(x1), Some(y1)) = (x1, y1) else {
            return Err(error(ErrorCode::AdapterRejected, "tree bounds underflow"));
        };
        require(
            x1 + w < self.dimensions[0]
                && y1 + h < self.dimensions[1]
                && self.root[2] + z < self.dimensions[2]
                && u64::from(w + 2) * u64::from(h + 2) * u64::from(z) == self.cells.len() as u64,
            "tree capture domain differs",
        )?;
        let mut i = 0;
        for zz in self.root[2]..self.root[2] + z {
            for yy in y1 - 1..=y1 + h {
                for xx in x1 - 1..=x1 + w {
                    require(
                        self.cells
                            .get(i)
                            .is_some_and(|c| c.position == [xx, yy, zz]),
                        "tree cell order/domain",
                    )?;
                    i += 1;
                }
            }
        }
        require(
            self.cells.iter().any(|c| c.position == self.designation),
            "designation outside captured scope",
        )?;
        require(
            self.cells.iter().all(|c| c.presence == 2),
            "tree body requires complete visibility",
        )?;
        let mut xx = w / 2;
        let mut yy = h / 2;
        require(
            self.body[(yy * w + xx) as usize] & 1 != 0,
            "root is not an observed trunk",
        )?;
        while xx + 1 < w && yy + 1 < h {
            if self.body[(yy * w + xx + 1) as usize] & 1 != 0 {
                xx += 1;
            } else if self.body[((yy + 1) * w + xx) as usize] & 1 != 0 {
                yy += 1;
            } else {
                break;
            }
        }
        require(
            self.designation == [x1 + xx, y1 + yy, self.root[2]],
            "computed southeast designation differs",
        )?;
        require(
            self.workers
                .iter()
                .all(|w| (0..3).all(|i| w.position[i] < self.dimensions[i]))
                && self
                    .tools
                    .iter()
                    .all(|t| (0..3).all(|i| t.position[i] < self.dimensions[i])),
            "tree actor/tool position outside map",
        )
    }
    fn encode(&self) -> Vec<u8> {
        let mut out = b"DFMTC021".to_vec();
        for n in [self.generation, self.sequence, self.tick] {
            out.extend_from_slice(&n.to_be_bytes());
        }
        out.extend_from_slice(&self.source.site().to_be_bytes());
        put_coord(&mut out, self.dimensions);
        out.extend_from_slice(&self.next_item.to_be_bytes());
        field(&mut out, self.source.folder().as_bytes());
        put_coord(&mut out, self.root);
        out.push(self.root_presence);
        out.extend_from_slice(&self.species.to_be_bytes());
        put_coord(&mut out, self.size);
        put_coord(&mut out, self.designation);
        out.extend(
            [
                self.paused,
                self.mature,
                self.alive,
                self.complete,
                self.existing_job,
            ]
            .map(u8::from),
        );
        out.extend_from_slice(&(self.body.len() as u16).to_be_bytes());
        for n in &self.body {
            out.extend_from_slice(&n.to_be_bytes());
        }
        out.extend_from_slice(&(self.cells.len() as u16).to_be_bytes());
        for c in &self.cells {
            c.append(&mut out);
        }
        out.push(self.workers.len() as u8);
        for w in &self.workers {
            out.extend_from_slice(&w.id.to_be_bytes());
            put_coord(&mut out, w.position);
            out.extend(w.facts.map(u8::from));
        }
        out.push(self.tools.len() as u8);
        for t in &self.tools {
            for n in [t.id, t.flags, t.flags2, t.holder] {
                out.extend_from_slice(&n.to_be_bytes());
            }
            out.extend_from_slice(&t.role.to_be_bytes());
            put_coord(&mut out, t.position);
            out.extend(t.facts.map(u8::from));
        }
        out
    }
    pub fn bytes(&self) -> &[u8] {
        &self.raw
    }
    pub fn witness(&self) -> Digest32 {
        Digest32::of_bytes(&self.raw)
    }
    /// Direct designation scope: observed tree/halo plus the complete block
    /// whose scheduling flag markPlant writes. Clock exposure is separate.
    pub fn effect_scope(&self) -> Result<MapCuboid> {
        self.geometry()?;
        require(
            self.mature && !self.body.is_empty() && !self.cells.is_empty(),
            "tree effect geometry unestablished",
        )?;
        let mut lo = [u32::MAX; 3];
        let mut hi = [0; 3];
        for c in &self.cells {
            for i in 0..3 {
                lo[i] = lo[i].min(c.position[i]);
                hi[i] = hi[i].max(c.position[i]);
            }
        }
        for i in 0..2 {
            lo[i] = lo[i].min((self.designation[i] / 16) * 16);
            hi[i] = hi[i].max(((self.designation[i] / 16) * 16 + 15).min(self.dimensions[i] - 1));
        }
        lo[2] = lo[2].min(self.designation[2]);
        hi[2] = hi[2].max(self.designation[2]);
        MapCuboid::new(
            MapCoord::new(lo[0] as i32, lo[1] as i32, lo[2] as i32),
            MapCoord::new(hi[0] as i32, hi[1] as i32, hi[2] as i32),
        )
    }
    pub fn eligible(&self) -> bool {
        self.paused
            && self.root_presence == 2
            && self.mature
            && self.alive
            && self.complete
            && !self.existing_job
            && self.species >= 0
            && !self.body.is_empty()
            && !self.cells.is_empty()
            && self.sequence < u64::MAX - 1
            && self.cells.iter().all(|c| {
                c.presence == 2 && c.liquid == 0 && c.temperature.iter().all(|n| *n <= 10080)
            })
            && self
                .cells
                .iter()
                .any(|c| c.position == self.designation && c.dig == 0 && c.occupancy == 0)
            && self.workers.iter().any(Worker::eligible)
            && self.tools.iter().any(Tool::offered)
    }
    pub fn expected_after(&self) -> Result<Self> {
        require(self.eligible(), "tree offer ineligible")?;
        let mut out = self.clone();
        out.sequence += 1;
        for c in &mut out.cells {
            if c.position[2] == self.designation[2]
                && c.position[0] / 16 == self.designation[0] / 16
                && c.position[1] / 16 == self.designation[1] / 16
            {
                c.designated = true;
            }
            if c.position == self.designation {
                c.dig = 1;
            }
        }
        out.raw = out.encode();
        Ok(out)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    key: String,
    pub before: Capture,
    pub digest: Digest32,
    pub token: [u8; 16],
}
impl Plan {
    pub fn new(key: &str, before: Capture) -> Result<Self> {
        validate_key(key)?;
        require(
            before.encode() == before.raw,
            "tree evidence fields differ from canonical bytes",
        )?;
        before.geometry()?;
        require(before.eligible(), "tree offer ineligible")?;
        let mut bytes = Vec::new();
        put_coord(&mut bytes, before.root);
        bytes.extend_from_slice(before.witness().as_bytes());
        let digest = hash(b"dfmcp-tree-harvest-plan/1", &bytes);
        let mut bytes = Vec::new();
        field(&mut bytes, key.as_bytes());
        bytes.extend_from_slice(digest.as_bytes());
        let token = hash(b"dfmcp-tree-harvest-token/1", &bytes).as_bytes()[..16]
            .try_into()
            .map_err(|_| error(ErrorCode::InternalInvariantViolation, "tree token width"))?;
        Ok(Self {
            key: key.to_owned(),
            before,
            digest,
            token,
        })
    }
    pub fn key(&self) -> &str {
        &self.key
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut out = b"DFMTL021".to_vec();
        field(&mut out, self.key.as_bytes());
        blob(&mut out, self.before.bytes());
        out
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader(bytes);
        require(r.take(8)? == b"DFMTL021", "not tree plan")?;
        let key = text(&mut r, 128)?;
        let before = Capture::decode(read_blob(&mut r, MAX_CAPTURE_BYTES)?)?;
        r.finish()?;
        Self::new(&key, before)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    raw: Vec<u8>,
    pub plan: Plan,
    pub phase: u8,
    pub reason: u8,
    pub after: Option<Capture>,
    pub receipt: Digest32,
}
impl Record {
    pub fn decode(bytes: &[u8], plan: &Plan) -> Result<Self> {
        require(
            bytes.len() <= MAX_RECORD_BYTES && bytes.len() >= 32,
            "tree record bound",
        )?;
        let (body, checksum) = bytes.split_at(bytes.len() - 32);
        let receipt = hash(b"dfmcp-tree-harvest-receipt/1", body);
        require(receipt.as_bytes() == checksum, "tree receipt checksum")?;
        let mut r = Reader(body);
        require(
            r.take(8)? == b"DFMTR021" && text(&mut r, 128)? == plan.key(),
            "tree receipt identity",
        )?;
        require(
            r.take(32)? == plan.digest.as_bytes() && r.take(16)? == plan.token,
            "tree receipt plan/token",
        )?;
        require(
            Capture::decode(read_blob(&mut r, MAX_CAPTURE_BYTES)?)? == plan.before,
            "tree receipt original capture",
        )?;
        let phase = r.byte()?;
        let reason = r.byte()?;
        require(phase <= 4 && reason <= 5, "tree receipt phase")?;
        let after = if r.boolean()? {
            Some(Capture::decode(read_blob(&mut r, MAX_CAPTURE_BYTES)?)?)
        } else {
            None
        };
        let insertion = r.boolean()?;
        if insertion {
            require(
                r.take(8)? == b"DFMTI021"
                    && coord(&mut r)? == plan.before.root
                    && coord(&mut r)? == plan.before.designation
                    && r.u32()? as i32 == plan.before.species
                    && r.boolean()?,
                "tree designation proof differs",
            )?;
        }
        r.finish()?;
        match phase {
            0 => require(
                reason == 0 && after.is_none() && !insertion,
                "invalid prepared tree receipt",
            )?,
            1 => require(
                reason == 5 && after.is_none() && !insertion,
                "invalid indeterminate tree receipt",
            )?,
            2 => require(
                reason == 0 && insertion && after.as_ref() == Some(&plan.before.expected_after()?),
                "tree exact poststate differs",
            )?,
            3 => require(
                (1..=3).contains(&reason) && after.is_none() && !insertion,
                "invalid refused tree receipt",
            )?,
            _ => require(
                reason == 4 && after.is_none() && !insertion,
                "invalid cancelled tree receipt",
            )?,
        }
        Ok(Self {
            raw: bytes.to_vec(),
            plan: plan.clone(),
            phase,
            reason,
            after,
            receipt,
        })
    }
    pub fn bytes(&self) -> &[u8] {
        &self.raw
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wood {
    pub id: u32,
    pub position: [u32; 3],
    pub material: i32,
    pub material_index: i32,
    pub flags: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    raw: Vec<u8>,
    pub generation: u64,
    pub sequence: u64,
    pub tick: u64,
    pub source: FortressIdentity,
    pub dimensions: [u32; 3],
    pub root: [u32; 3],
    pub paused: bool,
    pub root_predicate: u8,
    pub species: i32,
    pub next_item: u32,
    pub nearby_felling_jobs: u16,
    pub wood: Vec<Wood>,
}
impl Progress {
    pub fn decode(bytes: &[u8], plan: &Plan) -> Result<Self> {
        require(bytes.len() <= 4096, "tree progress bound")?;
        let mut r = Reader(bytes);
        require(r.take(8)? == b"DFMTP021", "not tree progress")?;
        let generation = r.u64()?;
        let sequence = r.u64()?;
        let tick = r.u64()?;
        let site = r.u32()?;
        let dimensions = [r.u32()?, r.u32()?, r.u32()?];
        let source = FortressIdentity::new(&text(&mut r, 512)?, site)?;
        let root = coord(&mut r)?;
        let paused = r.boolean()?;
        let root_predicate = r.byte()?;
        let species = r.u32()? as i32;
        let next_item = r.u32()?;
        let nearby_felling_jobs = number16(&mut r)?;
        let n = number16(&mut r)?;
        require(
            generation > 0
                && generation < u64::MAX
                && sequence < u64::MAX
                && tick <= crate::bounded_run::MAX_NATIVE_TICK
                && root_predicate <= 3
                && root == plan.before.root
                && species == plan.before.species
                && n <= 64
                && dimensions == plan.before.dimensions
                && source == plan.before.source
                && next_item >= plan.before.next_item,
            "tree progress binding",
        )?;
        let mut wood = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let id = r.u32()?;
            let position = coord(&mut r)?;
            let material = r.u32()? as i32;
            let material_index = r.u32()? as i32;
            let flags = r.u32()?;
            require(
                id >= plan.before.next_item
                    && id < next_item
                    && material >= 0
                    && flags & !((1 << 0) | (1 << 28) | (1 << 29)) == 0
                    && flags & 1 == 1
                    && (0..3).all(|i| position[i] < dimensions[i])
                    && wood.last().is_none_or(|w: &Wood| w.id < id),
                "invalid new ground-WOOD observation",
            )?;
            wood.push(Wood {
                id,
                position,
                material,
                material_index,
                flags,
            });
        }
        r.finish()?;
        Ok(Self {
            raw: bytes.to_vec(),
            generation,
            sequence,
            tick,
            source,
            dimensions,
            root,
            paused,
            root_predicate,
            species,
            next_item,
            nearby_felling_jobs,
            wood,
        })
    }
    pub fn bytes(&self) -> &[u8] {
        &self.raw
    }
    pub fn supply_observed(&self) -> bool {
        self.paused
            && self.root_predicate == 2
            && self.nearby_felling_jobs == 0
            && !self.wood.is_empty()
    }
}
