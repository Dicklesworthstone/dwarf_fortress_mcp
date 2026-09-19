#![forbid(unsafe_code)]

//! Exact witnesses and receipts for the isolated, unadmitted job-control/1.9
//! profile. These values describe one native job, not a canonical world snapshot.
//! A plan is not authority to dispatch: an authorized durable coordinator must
//! record dispatch intent before invoking the native setter.

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, Result};

pub const MAX_OBSERVATION_BYTES: usize = 1024;
pub const MAX_EFFECT_BYTES: usize = 322;
pub const MAX_IDEMPOTENCY_KEY_BYTES: usize = 128;
pub const MAX_NATIVE_TICK: u64 = u32::MAX as u64 * 403_200 + 403_199;

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::AdapterRejected, message)
}

fn require(condition: bool, message: &str) -> Result<()> {
    if condition { Ok(()) } else { Err(invalid(message)) }
}

struct Reader<'a> { remaining: &'a [u8] }
impl<'a> Reader<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        let value = self.remaining.get(..length)
            .ok_or_else(|| invalid("truncated job-control record"))?;
        self.remaining = &self.remaining[length..];
        Ok(value)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.take(N)?.try_into().map_err(|_| invalid("invalid job-control field width"))
    }
    fn byte(&mut self) -> Result<u8> { Ok(self.array::<1>()?[0]) }
    fn boolean(&mut self) -> Result<bool> {
        match self.byte()? {
            0 => Ok(false), 1 => Ok(true),
            _ => Err(invalid("noncanonical job-control boolean")),
        }
    }
    fn u32(&mut self) -> Result<u32> { Ok(u32::from_be_bytes(self.array()?)) }
    fn i32(&mut self) -> Result<i32> { Ok(i32::from_be_bytes(self.array()?)) }
    fn u64(&mut self) -> Result<u64> { Ok(u64::from_be_bytes(self.array()?)) }
    fn digest(&mut self) -> Result<Digest32> { Ok(Digest32::from_bytes(self.array()?)) }
    fn text(&mut self, maximum: usize, empty: bool) -> Result<String> {
        let length = usize::from(u16::from_be_bytes(self.array()?));
        require(length <= maximum && (empty || length > 0), "invalid job-control text bound")?;
        let value = std::str::from_utf8(self.take(length)?)
            .map_err(|_| invalid("job-control text is not UTF-8"))?;
        require(!value.contains('\0'), "job-control text contains NUL")?;
        Ok(value.to_owned())
    }
    fn finish(self) -> Result<()> {
        require(self.remaining.is_empty(), "trailing job-control record bytes")
    }
}

fn identity(generation: u64, sequence: u64, tick: u64, job: u32) -> Result<()> {
    require(generation != 0 && generation != u64::MAX && sequence != u64::MAX,
        "invalid job-control incarnation or sequence")?;
    require(tick <= MAX_NATIVE_TICK && job <= i32::MAX as u32,
        "job-control tick or job ID out of range")
}

/// Validate before any transport write. Native keys use a closed ASCII grammar.
pub fn validate_key(key: &str) -> Result<()> {
    if key.is_empty() || key.len() > MAX_IDEMPOTENCY_KEY_BYTES
        || !key.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(DfmcpError::new(ErrorCode::InvalidRequest,
            "job-control key must be 1..128 ASCII letters, digits, '.', '_' or '-'"));
    }
    Ok(())
}

fn put_text(out: &mut Vec<u8>, text: &str) {
    // All callers have already checked the native 128-byte key bound.
    out.extend_from_slice(&(text.len() as u16).to_be_bytes());
    out.extend_from_slice(text.as_bytes());
}

fn plan_hash(job: u32, desired: bool, witness: Digest32) -> Digest32 {
    let mut bytes = b"dfmcp-job-suspension-plan/1\0".to_vec();
    bytes.extend_from_slice(&job.to_be_bytes());
    bytes.push(u8::from(desired));
    bytes.extend_from_slice(witness.as_bytes());
    Digest32::of_bytes(&bytes)
}

fn token_hash(generation: u64, key: &str, plan: Digest32) -> [u8; 16] {
    let mut bytes = b"dfmcp-job-suspension-token/1\0".to_vec();
    bytes.extend_from_slice(&generation.to_be_bytes());
    put_text(&mut bytes, key);
    bytes.extend_from_slice(plan.as_bytes());
    let digest = Digest32::of_bytes(&bytes);
    let mut token = [0; 16];
    token.copy_from_slice(&digest.as_bytes()[..16]);
    token
}

/// Immutable, fully decoded, selected-job observation. `witness` hashes exactly
/// `canonical_bytes`, including unknown native type numbers and negative positions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobObservation {
    bytes: Vec<u8>,
    generation: u64,
    sequence: u64,
    tick: u64,
    job: u32,
    next_job: u32,
    site: u32,
    job_type: i32,
    holder: i32,
    holder_type: i32,
    worker: i32,
    position: [i32; 3],
    timer: i32,
    attachments: u32,
    filters: u32,
    flags: u8,
    folder: String,
    type_key: String,
    reaction: String,
}

impl JobObservation {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        require(bytes.len() <= MAX_OBSERVATION_BYTES, "job observation exceeds 1024 bytes")?;
        let mut r = Reader { remaining: bytes };
        require(r.take(8)? == b"DFMJS019", "not a job-control/1.9 observation")?;
        let generation = r.u64()?;
        let sequence = r.u64()?;
        let tick = r.u64()?;
        let job = r.u32()?;
        let next_job = r.u32()?;
        let site = r.u32()?;
        identity(generation, sequence, tick, job)?;
        require(job < next_job && next_job <= i32::MAX as u32 && site <= i32::MAX as u32,
            "invalid native job horizon or fortress site")?;
        let job_type = r.i32()?;
        let holder = r.i32()?;
        let holder_type = r.i32()?;
        let worker = r.i32()?;
        let position = [r.i32()?, r.i32()?, r.i32()?];
        let timer = r.i32()?;
        let attachments = r.u32()?;
        let filters = r.u32()?;
        let flags = r.byte()?;
        require(job_type >= 0 && holder >= -1 && holder_type >= -1 && worker >= -1 && timer >= -1,
            "invalid native job scalar")?;
        require(attachments <= 65_536 && filters <= 4096 && flags & !63 == 0,
            "invalid job-control counts or reserved flags")?;
        require(if holder == -1 { holder_type == -1 && flags & (8 | 32) == 0 }
            else { holder_type >= 0 }, "inconsistent native job holder evidence")?;
        let folder = r.text(512, false)?;
        let type_key = r.text(128, false)?;
        let reaction = r.text(128, true)?;
        r.finish()?;
        Ok(Self { bytes: bytes.to_vec(), generation, sequence, tick, job, next_job, site,
            job_type, holder, holder_type, worker, position, timer, attachments, filters,
            flags, folder, type_key, reaction })
    }

    pub fn canonical_bytes(&self) -> &[u8] { &self.bytes }
    pub fn witness(&self) -> Digest32 { Digest32::of_bytes(&self.bytes) }
    pub fn generation(&self) -> u64 { self.generation }
    pub fn sequence(&self) -> u64 { self.sequence }
    pub fn tick(&self) -> u64 { self.tick }
    pub fn job_id(&self) -> u32 { self.job }
    pub fn next_job_id(&self) -> u32 { self.next_job }
    pub fn site_id(&self) -> u32 { self.site }
    pub fn job_type(&self) -> i32 { self.job_type }
    pub fn holder_id(&self) -> Option<u32> { u32::try_from(self.holder).ok() }
    pub fn holder_type(&self) -> Option<u32> { u32::try_from(self.holder_type).ok() }
    pub fn worker_id(&self) -> Option<u32> { u32::try_from(self.worker).ok() }
    pub fn position(&self) -> [i32; 3] { self.position }
    pub fn completion_timer(&self) -> i32 { self.timer }
    pub fn attachment_count(&self) -> u32 { self.attachments }
    pub fn filter_count(&self) -> u32 { self.filters }
    pub fn suspended(&self) -> bool { self.flags & 1 != 0 }
    pub fn repeating(&self) -> bool { self.flags & 2 != 0 }
    pub fn paused(&self) -> bool { self.flags & 4 != 0 }
    pub fn holder_complete(&self) -> bool { self.flags & 8 != 0 }
    pub fn supported(&self) -> bool { self.flags & 16 != 0 }
    pub fn production_holder(&self) -> bool { self.flags & 32 != 0 }
    pub fn world_folder(&self) -> &str { &self.folder }
    pub fn type_key(&self) -> &str { &self.type_key }
    pub fn reaction(&self) -> &str { &self.reaction }

    /// Eligibility is evidence, not a grant, reservation, or future guarantee.
    pub fn eligible(&self) -> bool {
        self.paused() && self.holder >= 0 && self.production_holder()
            && self.holder_complete() && self.supported() && self.worker == -1 && self.timer == -1
    }

    fn expected_after_witness(&self, suspended: bool) -> Result<Digest32> {
        let sequence = self.sequence.checked_add(1)
            .filter(|s| *s != u64::MAX).ok_or_else(|| invalid("job sequence exhausted"))?;
        // Frozen DFMJS019 layout: sequence at 16, flags at 84. The native
        // transaction permits only these two fields to change before readback.
        let mut bytes = self.bytes.clone();
        bytes[16..24].copy_from_slice(&sequence.to_be_bytes());
        bytes[84] = (bytes[84] & !1) | u8::from(suspended);
        Ok(Digest32::of_bytes(&bytes))
    }
}

/// Sealed native intent. A prepare replay cannot change its key, target, desired
/// setting, witness, incarnation, or token, and cannot renew its native lifetime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SuspensionPlan {
    observation: JobObservation,
    key: String,
    desired: bool,
    digest: Digest32,
    token: [u8; 16],
}
impl SuspensionPlan {
    pub fn new(observation: JobObservation, key: &str, desired: bool) -> Result<Self> {
        validate_key(key)?;
        if !observation.eligible() {
            return Err(DfmcpError::new(ErrorCode::CapabilityDenied,
                "job suspension requires a paused, idle, supported job at a completed workshop or furnace"));
        }
        let digest = plan_hash(observation.job, desired, observation.witness());
        let token = token_hash(observation.generation, key, digest);
        Ok(Self { observation, key: key.to_owned(), desired, digest, token })
    }
    pub fn observation(&self) -> &JobObservation { &self.observation }
    pub fn key(&self) -> &str { &self.key }
    pub fn desired(&self) -> bool { self.desired }
    pub fn digest(&self) -> Digest32 { self.digest }
    pub fn prepare_token(&self) -> &[u8; 16] { &self.token }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum SuspensionState { Prepared = 0, Unknown = 1, Applied = 2, NotApplied = 3, Refused = 4 }
impl SuspensionState {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Applied | Self::NotApplied | Self::Refused)
    }
}

/// A record is returned only after binding its complete identity and readback
/// to a caller-held sealed plan. Checksums are not signatures or game authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SuspensionEffect {
    bytes: Vec<u8>,
    state: SuspensionState,
    after: Option<bool>,
    after_witness: Option<Digest32>,
    receipt: Option<Digest32>,
}
impl SuspensionEffect {
    pub fn decode(bytes: &[u8], plan: &SuspensionPlan) -> Result<Self> {
        require(bytes.len() <= MAX_EFFECT_BYTES, "job effect exceeds 322 bytes")?;
        let mut r = Reader { remaining: bytes };
        require(r.take(8)? == b"DFMJSE19", "not a job-control/1.9 effect")?;
        let generation = r.u64()?;
        let sequence = r.u64()?;
        let tick = r.u64()?;
        let job = r.u32()?;
        let desired = r.boolean()?;
        let witness = r.digest()?;
        let digest = r.digest()?;
        let token = r.array::<16>()?;
        let state = match r.byte()? {
            0 => SuspensionState::Prepared, 1 => SuspensionState::Unknown,
            2 => SuspensionState::Applied, 3 => SuspensionState::NotApplied,
            4 => SuspensionState::Refused,
            _ => return Err(invalid("unknown job suspension state")),
        };
        let after_known = r.boolean()?;
        let after = r.boolean()?;
        let after_tick = r.u64()?;
        let after_witness = r.digest()?;
        let receipt = r.digest()?;
        let key = r.text(MAX_IDEMPOTENCY_KEY_BYTES, false)?;
        r.finish()?;
        let before = &plan.observation;
        require(generation == before.generation && sequence == before.sequence && tick == before.tick
            && job == before.job && desired == plan.desired && witness == before.witness()
            && digest == plan.digest && token == plan.token && key == plan.key,
            "job effect does not match its sealed preparation")?;
        let observed = matches!(state, SuspensionState::Applied | SuspensionState::NotApplied);
        require(after_known == observed, "job outcome and observation presence disagree")?;
        if observed {
            require(after_tick == tick && after_witness == before.expected_after_witness(after)?,
                "job readback is not the exact controlled observation transition")?;
            require((after == desired) == (state == SuspensionState::Applied),
                "job effect outcome contradicts readback")?;
        } else {
            require(!after && after_tick == 0 && after_witness == Digest32::ZERO,
                "absent job readback has noncanonical backing data")?;
        }
        if state.terminal() {
            let mut proof = b"dfmcp-job-suspension-receipt/1\0".to_vec();
            proof.extend_from_slice(&generation.to_be_bytes());
            put_text(&mut proof, &key);
            proof.extend_from_slice(digest.as_bytes());
            proof.extend_from_slice(&token);
            proof.extend_from_slice(&[state as u8, u8::from(after_known), u8::from(after)]);
            proof.extend_from_slice(&after_tick.to_be_bytes());
            proof.extend_from_slice(after_witness.as_bytes());
            require(receipt == Digest32::of_bytes(&proof), "job suspension receipt mismatch")?;
        } else {
            require(receipt == Digest32::ZERO, "nonterminal job effect carries a receipt")?;
        }
        Ok(Self { bytes: bytes.to_vec(), state, after: observed.then_some(after),
            after_witness: observed.then_some(after_witness), receipt: state.terminal().then_some(receipt) })
    }
    pub fn canonical_bytes(&self) -> &[u8] { &self.bytes }
    pub fn state(&self) -> SuspensionState { self.state }
    pub fn observed_suspended(&self) -> Option<bool> { self.after }
    pub fn after_witness(&self) -> Option<Digest32> { self.after_witness }
    pub fn receipt(&self) -> Option<Digest32> { self.receipt }
}

#[cfg(test)]
#[path = "job_suspension/tests.rs"]
mod tests;
