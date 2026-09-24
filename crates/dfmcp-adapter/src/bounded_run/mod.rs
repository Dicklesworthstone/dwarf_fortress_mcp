#![forbid(unsafe_code)]
//! Typed, sealed evidence for the isolated native run/1.13 development profile.
//!
//! Limits are native callback stop triggers, not exact-tick guarantees. A stopped
//! record proves a historical pause, not goal completion or present pause state.
//! This protocol observes a source incarnation, not a named fortress lineage.

pub mod journal;
pub mod private_file;
pub mod rpc;

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, Result};

pub const MAX_GAME_TICKS: u32 = 1200;
pub const MAX_WALL_MS: u32 = 60_000;
pub const MAX_RECORD_BYTES: usize = 274;
pub const MAX_NATIVE_TICK: u64 = u32::MAX as u64 * 403_200 + 403_199;

pub(crate) fn error(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message)
}
pub(crate) fn require(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(error(ErrorCode::AdapterRejected, message))
    }
}
pub fn validate_key(key: &str) -> Result<()> {
    if key.is_empty()
        || key.len() > 128
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "run key must be 1..128 ASCII letters, digits, '.', '_' or '-'",
        ));
    }
    Ok(())
}
pub(crate) fn hash(domain: &[u8], bytes: &[u8]) -> Digest32 {
    let mut input = domain.to_vec();
    input.push(0);
    input.extend_from_slice(bytes);
    Digest32::of_bytes(&input)
}

pub(crate) struct Reader<'a>(pub(crate) &'a [u8]);
impl<'a> Reader<'a> {
    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let bytes = self
            .0
            .get(..n)
            .ok_or_else(|| error(ErrorCode::AdapterRejected, "truncated bounded-run evidence"))?;
        self.0 = &self.0[n..];
        Ok(bytes)
    }
    pub(crate) fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.take(N)?.try_into().map_err(|_| {
            error(
                ErrorCode::AdapterRejected,
                "invalid bounded-run field width",
            )
        })
    }
    pub(crate) fn byte(&mut self) -> Result<u8> {
        Ok(self.array::<1>()?[0])
    }
    pub(crate) fn boolean(&mut self) -> Result<bool> {
        match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(error(ErrorCode::AdapterRejected, "noncanonical boolean")),
        }
    }
    pub(crate) fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.array()?))
    }
    pub(crate) fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.array()?))
    }
    pub(crate) fn finish(self) -> Result<()> {
        require(self.0.is_empty(), "trailing bounded-run evidence")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunSpec {
    ticks: u32,
    wall_ms: u32,
}
impl RunSpec {
    pub fn new(ticks: u32, wall_ms: u32) -> Result<Self> {
        if !(1..=MAX_GAME_TICKS).contains(&ticks) || !(1..=MAX_WALL_MS).contains(&wall_ms) {
            return Err(error(
                ErrorCode::InvalidRequest,
                "run requires 1..1200 ticks and 1..60000 milliseconds",
            ));
        }
        Ok(Self { ticks, wall_ms })
    }
    pub fn game_ticks(self) -> u32 {
        self.ticks
    }
    pub fn wall_ms(self) -> u32 {
        self.wall_ms
    }
    pub(crate) fn append(self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.ticks.to_be_bytes());
        out.extend_from_slice(&self.wall_ms.to_be_bytes());
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunObservation {
    bytes: [u8; 35],
    generation: u64,
    sequence: u64,
    tick: Option<u64>,
    loaded: bool,
    paused: bool,
}
impl RunObservation {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        require(
            bytes.len() == 35,
            "run observation must contain exactly 35 bytes",
        )?;
        let mut r = Reader(bytes);
        require(r.take(8)? == b"DFMRO013", "not a run/1.13 observation")?;
        let generation = r.u64()?;
        let sequence = r.u64()?;
        let tick = r.u64()?;
        let loaded = r.boolean()?;
        let valid = r.boolean()?;
        let paused = r.boolean()?;
        r.finish()?;
        require(
            generation > 0 && generation < u64::MAX,
            "invalid run source generation",
        )?;
        require(
            loaded || (!valid && !paused),
            "unloaded source contains live clock fields",
        )?;
        require(
            if valid {
                tick <= MAX_NATIVE_TICK
            } else {
                tick == 0
            },
            "invalid or fabricated game tick",
        )?;
        let bytes = bytes
            .try_into()
            .map_err(|_| error(ErrorCode::AdapterRejected, "invalid observation width"))?;
        Ok(Self {
            bytes,
            generation,
            sequence,
            tick: valid.then_some(tick),
            loaded,
            paused,
        })
    }
    pub fn canonical_bytes(&self) -> &[u8; 35] {
        &self.bytes
    }
    pub fn witness(&self) -> Digest32 {
        Digest32::of_bytes(&self.bytes)
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    pub fn tick(&self) -> Option<u64> {
        self.tick
    }
    pub fn loaded(&self) -> bool {
        self.loaded
    }
    pub fn paused(&self) -> bool {
        self.paused
    }
    pub fn eligible(&self) -> bool {
        self.loaded && self.tick.is_some() && self.paused && self.sequence != u64::MAX
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunPlan {
    key: String,
    spec: RunSpec,
    before: RunObservation,
    digest: Digest32,
    token: [u8; 16],
}
impl RunPlan {
    pub fn new(key: &str, spec: RunSpec, before: RunObservation) -> Result<Self> {
        validate_key(key)?;
        if !before.eligible() {
            return Err(error(
                ErrorCode::StaleAnchor,
                "run requires an eligible paused observation",
            ));
        }
        let mut bytes = Vec::new();
        spec.append(&mut bytes);
        bytes.extend_from_slice(before.canonical_bytes());
        let digest = hash(b"dfmcp-bounded-run-plan/1", &bytes);
        let mut bytes = (key.len() as u16).to_be_bytes().to_vec();
        bytes.extend_from_slice(key.as_bytes());
        bytes.extend_from_slice(digest.as_bytes());
        let full = hash(b"dfmcp-bounded-run-token/1", &bytes);
        let mut token = [0; 16];
        token.copy_from_slice(&full.as_bytes()[..16]);
        Ok(Self {
            key: key.to_owned(),
            spec,
            before,
            digest,
            token,
        })
    }
    pub fn key(&self) -> &str {
        &self.key
    }
    pub fn spec(&self) -> RunSpec {
        self.spec
    }
    pub fn before(&self) -> &RunObservation {
        &self.before
    }
    pub fn digest(&self) -> Digest32 {
        self.digest
    }
    pub fn token(&self) -> &[u8; 16] {
        &self.token
    }
    /// Reconstructible sealed intent for durable coordination; no ambient authority.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut bytes = (self.key.len() as u16).to_be_bytes().to_vec();
        bytes.extend_from_slice(self.key.as_bytes());
        self.spec.append(&mut bytes);
        bytes.extend_from_slice(self.before.canonical_bytes());
        bytes
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        require(bytes.len() <= 173, "run intent exceeds its byte bound")?;
        let mut r = Reader(bytes);
        let n = usize::from(u16::from_be_bytes(r.array()?));
        require((1..=128).contains(&n), "invalid run key width")?;
        let key = std::str::from_utf8(r.take(n)?)
            .map_err(|_| error(ErrorCode::AdapterRejected, "invalid run key UTF-8"))?;
        let spec = RunSpec::new(r.u32()?, r.u32()?)?;
        let before = RunObservation::decode(r.take(35)?)?;
        r.finish()?;
        Self::new(key, spec, before)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum RunPhase {
    Prepared = 0,
    Running = 1,
    Stopping = 2,
    Stopped = 3,
    Refused = 4,
    SourceLost = 5,
}
impl RunPhase {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Stopped | Self::Refused | Self::SourceLost)
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
            Self::Refused => "refused",
            Self::SourceLost => "source_lost",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum RunReason {
    None = 0,
    TickLimit = 1,
    WallLimit = 2,
    Cancelled = 3,
    ExternalPause = 4,
    NativeFailure = 5,
    ClockRegression = 6,
    SourceChanged = 7,
    Shutdown = 8,
    Stale = 9,
}
impl RunReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::TickLimit => "tick_limit",
            Self::WallLimit => "wall_limit",
            Self::Cancelled => "cancelled",
            Self::ExternalPause => "external_pause",
            Self::NativeFailure => "native_failure",
            Self::ClockRegression => "clock_regression",
            Self::SourceChanged => "source_changed",
            Self::Shutdown => "shutdown",
            Self::Stale => "stale",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunRecord {
    bytes: Vec<u8>,
    plan: RunPlan,
    phase: RunPhase,
    reason: RunReason,
    attempted: bool,
    verified: bool,
    observed_tick: Option<u64>,
    receipt: Digest32,
}
impl RunRecord {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        require(
            (147..=MAX_RECORD_BYTES).contains(&bytes.len()),
            "invalid run record size",
        )?;
        let mut r = Reader(bytes);
        require(r.take(8)? == b"DFMRE013", "not a run/1.13 record")?;
        let n = usize::from(u16::from_be_bytes(r.array()?));
        require((1..=128).contains(&n), "invalid record key width")?;
        let key = std::str::from_utf8(r.take(n)?)
            .map_err(|_| error(ErrorCode::AdapterRejected, "invalid run key UTF-8"))?;
        let spec = RunSpec::new(r.u32()?, r.u32()?)?;
        let before = RunObservation::decode(r.take(35)?)?;
        let plan = RunPlan::new(key, spec, before)?;
        require(
            r.take(32)? == plan.digest().as_bytes() && r.take(16)? == plan.token(),
            "run record plan/token mismatch",
        )?;
        let phase = match r.byte()? {
            0 => RunPhase::Prepared,
            1 => RunPhase::Running,
            2 => RunPhase::Stopping,
            3 => RunPhase::Stopped,
            4 => RunPhase::Refused,
            5 => RunPhase::SourceLost,
            _ => return Err(error(ErrorCode::AdapterRejected, "unknown run phase")),
        };
        let reason = match r.byte()? {
            0 => RunReason::None,
            1 => RunReason::TickLimit,
            2 => RunReason::WallLimit,
            3 => RunReason::Cancelled,
            4 => RunReason::ExternalPause,
            5 => RunReason::NativeFailure,
            6 => RunReason::ClockRegression,
            7 => RunReason::SourceChanged,
            8 => RunReason::Shutdown,
            9 => RunReason::Stale,
            _ => return Err(error(ErrorCode::AdapterRejected, "unknown run reason")),
        };
        let attempted = r.boolean()?;
        let verified = r.boolean()?;
        let known = r.boolean()?;
        let tick = r.u64()?;
        let receipt = Digest32::from_bytes(r.array()?);
        r.finish()?;
        require(
            receipt == hash(b"dfmcp-bounded-run-receipt/1", &bytes[..bytes.len() - 32]),
            "run receipt checksum mismatch",
        )?;
        require(
            if known {
                tick <= MAX_NATIVE_TICK
            } else {
                tick == 0
            },
            "fabricated run observation tick",
        )?;
        require(
            attempted
                == matches!(
                    phase,
                    RunPhase::Running
                        | RunPhase::Stopping
                        | RunPhase::Stopped
                        | RunPhase::SourceLost
                )
                && verified == (phase == RunPhase::Stopped),
            "run phase and effect flags disagree",
        )?;
        let valid = match phase {
            RunPhase::Prepared => reason == RunReason::None && !known,
            RunPhase::Running => {
                reason == RunReason::None && known && plan.before.tick.is_some_and(|t| tick >= t)
            }
            RunPhase::Stopping | RunPhase::Stopped => {
                matches!(
                    reason,
                    RunReason::TickLimit
                        | RunReason::WallLimit
                        | RunReason::Cancelled
                        | RunReason::NativeFailure
                        | RunReason::ClockRegression
                        | RunReason::Shutdown
                ) || (phase == RunPhase::Stopped && reason == RunReason::ExternalPause)
            }
            RunPhase::Refused => {
                matches!(
                    reason,
                    RunReason::Cancelled | RunReason::SourceChanged | RunReason::Stale
                ) && !known
            }
            RunPhase::SourceLost => reason == RunReason::SourceChanged && !known,
        };
        require(valid, "impossible run phase/reason/observation combination")?;
        Ok(Self {
            bytes: bytes.to_vec(),
            plan,
            phase,
            reason,
            attempted,
            verified,
            observed_tick: known.then_some(tick),
            receipt,
        })
    }
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn plan(&self) -> &RunPlan {
        &self.plan
    }
    pub fn phase(&self) -> RunPhase {
        self.phase
    }
    pub fn reason(&self) -> RunReason {
        self.reason
    }
    pub fn unpause_attempted(&self) -> bool {
        self.attempted
    }
    pub fn pause_verified(&self) -> bool {
        self.verified
    }
    pub fn observed_tick(&self) -> Option<u64> {
        self.observed_tick
    }
    pub fn observed_ticks_advanced(&self) -> Option<u64> {
        self.observed_tick?.checked_sub(self.plan.before.tick?)
    }
    pub fn observed_tick_overshoot(&self) -> Option<u64> {
        Some(
            self.observed_ticks_advanced()?
                .saturating_sub(u64::from(self.plan.spec.ticks)),
        )
    }
    pub fn receipt(&self) -> Digest32 {
        self.receipt
    }
}

#[cfg(test)]
mod tests;
