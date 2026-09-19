#![forbid(unsafe_code)]
//! Sealed evidence for isolated, unadmitted work-orders/1.10.
//!
//! A queue observation establishes native IDs and the allocation horizon, not
//! existing order configurations, material availability, or production progress.
//! A Created receipt proves only the exact immediate insertion/template readback.
//! None of these values grants authority or replaces a durable coordinator.

pub mod rpc;

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, FortressId, Result};

pub const MAX_ORDERS: usize = 4096;
pub const MAX_OBSERVATION_BYTES: usize = 17 * 1024;
pub const MAX_EFFECT_BYTES: usize = 357;
pub const MAX_AMOUNT: u32 = 100;
pub const MAX_NATIVE_TICK: u64 = u32::MAX as u64 * 403_200 + 403_199;

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::AdapterRejected, message)
}
fn require(condition: bool, message: &str) -> Result<()> {
    if condition { Ok(()) } else { Err(invalid(message)) }
}

pub fn validate_key(key: &str) -> Result<()> {
    if key.is_empty() || key.len() > 128
        || !key.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(DfmcpError::new(ErrorCode::InvalidRequest,
            "work-order key must be 1..128 ASCII letters, digits, '.', '_' or '-'"));
    }
    Ok(())
}

struct Reader<'a> { remaining: &'a [u8] }
impl<'a> Reader<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        let out = self.remaining.get(..length).ok_or_else(|| invalid("truncated work-order record"))?;
        self.remaining = &self.remaining[length..];
        Ok(out)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.take(N)?.try_into().map_err(|_| invalid("invalid work-order field width"))
    }
    fn byte(&mut self) -> Result<u8> { Ok(self.array::<1>()?[0]) }
    fn boolean(&mut self) -> Result<bool> {
        match self.byte()? { 0 => Ok(false), 1 => Ok(true), _ => Err(invalid("noncanonical work-order boolean")) }
    }
    fn u32(&mut self) -> Result<u32> { Ok(u32::from_be_bytes(self.array()?)) }
    fn u64(&mut self) -> Result<u64> { Ok(u64::from_be_bytes(self.array()?)) }
    fn digest(&mut self) -> Result<Digest32> { Ok(Digest32::from_bytes(self.array()?)) }
    fn text(&mut self, maximum: usize) -> Result<String> {
        let length = usize::from(u16::from_be_bytes(self.array()?));
        require(length > 0 && length <= maximum, "invalid work-order text bound")?;
        let value = std::str::from_utf8(self.take(length)?).map_err(|_| invalid("invalid work-order UTF-8"))?;
        require(!value.contains('\0'), "NUL in work-order text")?;
        Ok(value.to_owned())
    }
    fn finish(self) -> Result<()> {
        require(self.remaining.is_empty(), "trailing work-order record bytes")
    }
}
fn put_text(out: &mut Vec<u8>, value: &str) {
    // Callers hold a decoded <=512-byte folder or a validated <=128-byte key.
    out.extend_from_slice(&(value.len() as u16).to_be_bytes());
    out.extend_from_slice(value.as_bytes());
}

/// Closed semantic catalog, NOT raw DF job-type numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum WorkOrderRecipe { WoodenBed = 1, WoodenDoor = 2, WoodenTable = 3, WoodenChair = 4 }
impl WorkOrderRecipe {
    pub fn from_code(value: u32) -> Result<Self> {
        match value {
            1 => Ok(Self::WoodenBed), 2 => Ok(Self::WoodenDoor),
            3 => Ok(Self::WoodenTable), 4 => Ok(Self::WoodenChair),
            _ => Err(DfmcpError::new(ErrorCode::InvalidRequest, "unknown work-order recipe")),
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WoodenBed => "wooden_bed", Self::WoodenDoor => "wooden_door",
            Self::WoodenTable => "wooden_table", Self::WoodenChair => "wooden_chair",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkOrderSpec { recipe: WorkOrderRecipe, amount: u32 }
impl WorkOrderSpec {
    pub fn new(recipe: WorkOrderRecipe, amount: u32) -> Result<Self> {
        if !(1..=MAX_AMOUNT).contains(&amount) {
            return Err(DfmcpError::new(ErrorCode::InvalidRequest, "work-order amount must be 1..100"));
        }
        Ok(Self { recipe, amount })
    }
    pub fn recipe(self) -> WorkOrderRecipe { self.recipe }
    pub fn amount(self) -> u32 { self.amount }
    fn append(self, out: &mut Vec<u8>) {
        out.push(self.recipe as u8); out.extend_from_slice(&self.amount.to_be_bytes());
    }
}

/// Immutable complete membership observation; it is not a canonical world snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkOrderObservation {
    bytes: Vec<u8>, generation: u64, sequence: u64, tick: u64,
    next_order: u32, site: u32, paused: bool, folder: String, ids: Vec<u32>,
}
impl WorkOrderObservation {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        require(bytes.len() <= MAX_OBSERVATION_BYTES, "work-order observation exceeds 17 KiB")?;
        let mut r = Reader { remaining: bytes };
        require(r.take(8)? == b"DFMWO010", "not a work-orders/1.10 observation")?;
        let generation = r.u64()?; let sequence = r.u64()?; let tick = r.u64()?;
        let next_order = r.u32()?; let site = r.u32()?; let paused = r.boolean()?;
        require(generation != 0 && generation != u64::MAX && sequence != u64::MAX,
            "invalid work-order incarnation or sequence")?;
        require(tick <= MAX_NATIVE_TICK && next_order <= i32::MAX as u32 && site <= i32::MAX as u32,
            "work-order clock or native ID out of range")?;
        let folder = r.text(512)?;
        let count = usize::try_from(r.u32()?).map_err(|_| invalid("work-order count overflow"))?;
        require(count <= MAX_ORDERS && count <= r.remaining.len() / 4, "invalid work-order membership bound")?;
        let mut ids = Vec::with_capacity(count);
        for _ in 0..count {
            let id = r.u32()?;
            require(id < next_order && ids.last().is_none_or(|previous| *previous < id),
                "work-order membership is not unique, sorted and below the allocation horizon")?;
            ids.push(id);
        }
        r.finish()?;
        Ok(Self { bytes: bytes.to_vec(), generation, sequence, tick, next_order, site, paused, folder, ids })
    }
    pub fn canonical_bytes(&self) -> &[u8] { &self.bytes }
    pub fn witness(&self) -> Digest32 { Digest32::of_bytes(&self.bytes) }
    pub fn generation(&self) -> u64 { self.generation }
    pub fn sequence(&self) -> u64 { self.sequence }
    pub fn tick(&self) -> u64 { self.tick }
    pub fn next_order_id(&self) -> u32 { self.next_order }
    pub fn site_id(&self) -> u32 { self.site }
    pub fn paused(&self) -> bool { self.paused }
    pub fn world_folder(&self) -> &str { &self.folder }
    pub fn order_ids(&self) -> &[u32] { &self.ids }
    pub fn eligible(&self) -> bool {
        self.paused && self.ids.len() < MAX_ORDERS && self.next_order < i32::MAX as u32
            && self.sequence < u64::MAX - 1
    }
    /// Same folder/site lineage domain as the other live profiles, not an EntityId.
    pub fn fortress_id(&self) -> FortressId {
        let mut data = b"dfmcp-live-fortress-id-v1\0".to_vec();
        data.extend_from_slice(self.folder.as_bytes()); data.push(0);
        data.extend_from_slice(&self.site.to_be_bytes());
        let digest = Digest32::of_bytes(&data); let mut bytes = [0; 8];
        bytes.copy_from_slice(&digest.as_bytes()[..8]);
        FortressId::new(u64::from_be_bytes(bytes) | 1)
    }
    fn expected_after_witness(&self) -> Result<Digest32> {
        require(self.eligible(), "ineligible work-order observation cannot form readback")?;
        // Reconstruct from decoded fields rather than trusting a self-hashed reply.
        let mut data = b"DFMWO010".to_vec();
        for value in [self.generation, self.sequence + 1, self.tick] { data.extend_from_slice(&value.to_be_bytes()); }
        data.extend_from_slice(&(self.next_order + 1).to_be_bytes());
        data.extend_from_slice(&self.site.to_be_bytes()); data.push(1);
        put_text(&mut data, &self.folder);
        data.extend_from_slice(&((self.ids.len() + 1) as u32).to_be_bytes());
        for id in self.ids.iter().copied().chain(std::iter::once(self.next_order)) { data.extend_from_slice(&id.to_be_bytes()); }
        Ok(Digest32::of_bytes(&data))
    }
}

/// Sealed finite intent, generated from retained observations, not raw plan bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkOrderPlan {
    observation: WorkOrderObservation, spec: WorkOrderSpec, key: String, digest: Digest32, token: [u8; 16],
}
impl WorkOrderPlan {
    pub fn new(observation: WorkOrderObservation, key: &str, spec: WorkOrderSpec) -> Result<Self> {
        validate_key(key)?;
        if !observation.eligible() {
            return Err(DfmcpError::new(ErrorCode::CapabilityDenied,
                "creation requires paused observation, spare queue capacity and unexhausted IDs/sequence"));
        }
        let mut data = b"dfmcp-work-order-plan/1\0".to_vec(); spec.append(&mut data);
        data.extend_from_slice(observation.witness().as_bytes()); let digest = Digest32::of_bytes(&data);
        let mut data = b"dfmcp-work-order-token/1\0".to_vec();
        data.extend_from_slice(&observation.generation.to_be_bytes()); put_text(&mut data, key);
        data.extend_from_slice(digest.as_bytes()); let token_hash = Digest32::of_bytes(&data);
        let mut token = [0; 16]; token.copy_from_slice(&token_hash.as_bytes()[..16]);
        Ok(Self { observation, spec, key: key.to_owned(), digest, token })
    }
    pub fn observation(&self) -> &WorkOrderObservation { &self.observation }
    pub fn spec(&self) -> WorkOrderSpec { self.spec }
    pub fn key(&self) -> &str { &self.key }
    pub fn digest(&self) -> Digest32 { self.digest }
    pub fn prepare_token(&self) -> &[u8; 16] { &self.token }
    fn configuration_witness(&self) -> Digest32 {
        let mut data = b"DFMWOC10".to_vec(); data.extend_from_slice(&self.observation.next_order.to_be_bytes());
        self.spec.append(&mut data); Digest32::of_bytes(&data)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum WorkOrderState { Prepared = 0, Unknown = 1, Created = 2, Refused = 4 }
impl WorkOrderState {
    pub fn terminal(self) -> bool { matches!(self, Self::Created | Self::Refused) }
}

/// Complete plan-bound native evidence. Hashes are checksums, not signatures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkOrderEffect {
    bytes: Vec<u8>, state: WorkOrderState, created_id: Option<u32>, after_tick: Option<u64>,
    after_witness: Option<Digest32>, configuration_witness: Option<Digest32>, receipt: Option<Digest32>,
}
impl WorkOrderEffect {
    pub fn decode(bytes: &[u8], plan: &WorkOrderPlan) -> Result<Self> {
        require(bytes.len() <= MAX_EFFECT_BYTES, "work-order effect exceeds 357 bytes")?;
        let mut r = Reader { remaining: bytes };
        require(r.take(8)? == b"DFMWOE10", "not a work-orders/1.10 effect")?;
        let generation = r.u64()?; let sequence = r.u64()?; let tick = r.u64()?; let id = r.u32()?;
        let recipe = WorkOrderRecipe::from_code(u32::from(r.byte()?))?;
        let spec = WorkOrderSpec::new(recipe, r.u32()?)?;
        let witness = r.digest()?; let digest = r.digest()?; let token = r.array::<16>()?;
        let state = match r.byte()? {
            0 => WorkOrderState::Prepared, 1 => WorkOrderState::Unknown,
            2 => WorkOrderState::Created, 4 => WorkOrderState::Refused,
            _ => return Err(invalid("unknown work-order outcome; absence is not not-applied")),
        };
        let after_known = r.boolean()?; let after_tick = r.u64()?;
        let after_witness = r.digest()?; let configuration_witness = r.digest()?; let receipt = r.digest()?;
        let key = r.text(128)?; r.finish()?;
        let before = &plan.observation;
        require(generation == before.generation && sequence == before.sequence && tick == before.tick
            && id == before.next_order && spec == plan.spec && witness == before.witness()
            && digest == plan.digest && token == plan.token && key == plan.key,
            "work-order effect differs from its complete sealed plan")?;
        let created = state == WorkOrderState::Created;
        require(after_known == created, "work-order outcome and readback presence disagree")?;
        if created {
            require(after_tick == tick && after_witness == before.expected_after_witness()?
                && configuration_witness == plan.configuration_witness(),
                "creation lacks exact controlled queue and template readback")?;
        } else {
            require(after_tick == 0 && after_witness == Digest32::ZERO && configuration_witness == Digest32::ZERO,
                "absent work-order readback has nonzero backing bytes")?;
        }
        if state.terminal() {
            let mut proof = b"dfmcp-work-order-receipt/1\0".to_vec(); proof.extend_from_slice(&generation.to_be_bytes());
            put_text(&mut proof, &key); proof.extend_from_slice(digest.as_bytes()); proof.extend_from_slice(&token);
            proof.extend_from_slice(&[state as u8, u8::from(after_known)]); proof.extend_from_slice(&after_tick.to_be_bytes());
            proof.extend_from_slice(after_witness.as_bytes()); proof.extend_from_slice(configuration_witness.as_bytes());
            require(receipt == Digest32::of_bytes(&proof), "work-order receipt checksum mismatch")?;
        } else {
            require(receipt == Digest32::ZERO, "nonterminal work-order effect carries a receipt")?;
        }
        Ok(Self { bytes: bytes.to_vec(), state, created_id: created.then_some(id), after_tick: created.then_some(after_tick),
            after_witness: created.then_some(after_witness), configuration_witness: created.then_some(configuration_witness),
            receipt: state.terminal().then_some(receipt) })
    }
    pub fn canonical_bytes(&self) -> &[u8] { &self.bytes }
    pub fn state(&self) -> WorkOrderState { self.state }
    pub fn created_order_id(&self) -> Option<u32> { self.created_id }
    pub fn observed_tick(&self) -> Option<u64> { self.after_tick }
    pub fn after_witness(&self) -> Option<Digest32> { self.after_witness }
    pub fn configuration_witness(&self) -> Option<Digest32> { self.configuration_witness }
    pub fn receipt(&self) -> Option<Digest32> { self.receipt }
}

#[cfg(test)]
#[path = "work_orders/tests.rs"]
mod tests;
