#![forbid(unsafe_code)]
//! Selected current order counters, never creation receipts or produced goods.
//! Protocol 1.11 is independently incarnated and read-only. Native IDs do not
//! prove correlation to another profile's historical creation across a restore.

pub mod rpc;
pub mod session;
use crate::work_orders::{MAX_NATIVE_TICK, WorkOrderRecipe, WorkOrderSpec};
use dfmcp_core::{DfmcpError, Digest32, ErrorCode, FortressId, Result};

pub const MAX_PROGRESS_BYTES: usize = 1024;
pub const MAX_WATCHES: usize = 8;

pub(crate) fn error(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message)
}
fn invalid() -> DfmcpError {
    error(
        ErrorCode::AdapterRejected,
        "malformed order-progress/1.11 observation",
    )
}
struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let out = self.0.get(..n).ok_or_else(invalid)?;
        self.0 = &self.0[n..];
        Ok(out)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.take(N)?.try_into().map_err(|_| invalid())
    }
    fn byte(&mut self) -> Result<u8> {
        Ok(self.array::<1>()?[0])
    }
    fn boolean(&mut self) -> Result<bool> {
        match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(invalid()),
        }
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.array()?))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.array()?))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.array()?))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderCounters {
    job_type: i32,
    remaining: u32,
    total: u32,
    status: u32,
    frequency: i32,
    template: Option<WorkOrderSpec>,
}
impl OrderCounters {
    pub fn native_job_type(&self) -> i32 {
        self.job_type
    }
    pub fn remaining(&self) -> u32 {
        self.remaining
    }
    pub fn total(&self) -> u32 {
        self.total
    }
    pub fn raw_status(&self) -> u32 {
        self.status
    }
    pub fn native_frequency(&self) -> i32 {
        self.frequency
    }
    pub fn validated(&self) -> bool {
        self.status & 1 != 0
    }
    pub fn active(&self) -> bool {
        self.status & 2 != 0
    }
    pub fn template(&self) -> Option<WorkOrderSpec> {
        self.template
    }
    pub fn classification(&self) -> &'static str {
        if self.template.is_none() {
            "unsupported_configuration"
        } else if self.remaining == 0 {
            "counter_zero"
        } else if !self.validated() {
            "awaiting_validation"
        } else if self.active() {
            "active"
        } else {
            "inactive"
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderProgress {
    bytes: Vec<u8>,
    generation: u64,
    sequence: u64,
    tick: u64,
    order: u32,
    horizon: u32,
    site: u32,
    paused: bool,
    folder: String,
    counters: Option<OrderCounters>,
}
impl OrderProgress {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_PROGRESS_BYTES {
            return Err(invalid());
        }
        let mut r = Reader(bytes);
        if r.take(8)? != b"DFMOP011" {
            return Err(invalid());
        }
        let generation = r.u64()?;
        let sequence = r.u64()?;
        let tick = r.u64()?;
        let order = r.u32()?;
        let horizon = r.u32()?;
        let site = r.u32()?;
        let paused = r.boolean()?;
        let present = r.boolean()?;
        let len = usize::from(u16::from_be_bytes(r.array()?));
        if generation == 0
            || generation == u64::MAX
            || sequence == 0
            || sequence == u64::MAX
            || tick > MAX_NATIVE_TICK
            || [order, horizon, site].iter().any(|n| *n > i32::MAX as u32)
            || !(1..=512).contains(&len)
        {
            return Err(invalid());
        }
        let folder = std::str::from_utf8(r.take(len)?)
            .map_err(|_| invalid())?
            .to_owned();
        if folder.contains('\0') {
            return Err(invalid());
        }
        let counters = if present {
            let job_type = r.i32()?;
            let remaining = r.u32()?;
            let total = r.u32()?;
            let status = r.u32()?;
            let frequency = r.i32()?;
            let recipe = r.byte()?;
            if order >= horizon
                || job_type < 0
                || frequency < -1
                || remaining > i16::MAX as u32
                || total > i16::MAX as u32
                || recipe > 4
            {
                return Err(invalid());
            }
            let template = if recipe == 0 {
                None
            } else {
                if remaining > total || frequency != 0 || status & !3 != 0 {
                    return Err(invalid());
                }
                Some(
                    WorkOrderSpec::new(WorkOrderRecipe::from_code(u32::from(recipe))?, total)
                        .map_err(|_| invalid())?,
                )
            };
            Some(OrderCounters {
                job_type,
                remaining,
                total,
                status,
                frequency,
                template,
            })
        } else {
            None
        };
        if !r.0.is_empty() {
            return Err(invalid());
        }
        Ok(Self {
            bytes: bytes.to_vec(),
            generation,
            sequence,
            tick,
            order,
            horizon,
            site,
            paused,
            folder,
            counters,
        })
    }
    pub fn canonical_bytes(&self) -> &[u8] {
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
    pub fn tick(&self) -> u64 {
        self.tick
    }
    pub fn order_id(&self) -> u32 {
        self.order
    }
    pub fn next_order_id(&self) -> u32 {
        self.horizon
    }
    pub fn site_id(&self) -> u32 {
        self.site
    }
    pub fn world_folder(&self) -> &str {
        &self.folder
    }
    pub fn paused(&self) -> bool {
        self.paused
    }
    pub fn counters(&self) -> Option<&OrderCounters> {
        self.counters.as_ref()
    }
    pub fn classification(&self) -> &'static str {
        self.counters
            .as_ref()
            .map_or("missing", OrderCounters::classification)
    }
    pub fn fortress_id(&self) -> FortressId {
        let mut data = b"dfmcp-live-fortress-id-v1\0".to_vec();
        data.extend_from_slice(self.folder.as_bytes());
        data.push(0);
        data.extend_from_slice(&self.site.to_be_bytes());
        let hash = Digest32::of_bytes(&data);
        let mut bytes = [0; 8];
        bytes.copy_from_slice(&hash.as_bytes()[..8]);
        FortressId::new(u64::from_be_bytes(bytes) | 1)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgressState {
    Watching,
    ZeroCandidate,
    CounterZeroStable,
    Missing,
    Changed,
    Expired,
    Cancelled,
}
impl ProgressState {
    pub fn terminal(self) -> bool {
        !matches!(self, Self::Watching | Self::ZeroCandidate)
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Watching => "watching",
            Self::ZeroCandidate => "zero_candidate",
            Self::CounterZeroStable => "counter_zero_stable",
            Self::Missing => "missing_not_completed",
            Self::Changed => "configuration_or_counter_changed",
            Self::Expired => "expired",
            Self::Cancelled => "cancelled",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgressWatch {
    key: String,
    baseline: OrderProgress,
    last: OrderProgress,
    previous: Option<OrderProgress>,
    deadline: u64,
    interval: u64,
    required: u32,
    zero_samples: u32,
    credited_tick: u64,
    state: ProgressState,
}
impl ProgressWatch {
    pub fn new(
        key: &str,
        baseline: OrderProgress,
        deadline: u64,
        interval: u64,
        required: u32,
    ) -> Result<Self> {
        crate::work_orders::validate_key(key)?;
        if baseline
            .counters()
            .and_then(OrderCounters::template)
            .is_none()
        {
            return Err(error(
                ErrorCode::PreconditionsFailed,
                "watch requires a present verified finite furniture template",
            ));
        }
        if deadline <= baseline.tick
            || deadline > MAX_NATIVE_TICK
            || interval == 0
            || interval > 403_200
            || !(2..=32).contains(&required)
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "watch needs a future game deadline, 1..403200 tick interval and 2..32 stable samples",
            ));
        }
        Ok(Self {
            key: key.to_owned(),
            last: baseline.clone(),
            previous: None,
            credited_tick: baseline.tick,
            baseline,
            deadline,
            interval,
            required,
            zero_samples: 0,
            state: ProgressState::Watching,
        })
    }
    pub fn key(&self) -> &str {
        &self.key
    }
    pub fn baseline(&self) -> &OrderProgress {
        &self.baseline
    }
    pub fn last(&self) -> &OrderProgress {
        &self.last
    }
    pub fn previous(&self) -> Option<&OrderProgress> {
        self.previous.as_ref()
    }
    pub fn state(&self) -> ProgressState {
        self.state
    }
    pub fn deadline(&self) -> u64 {
        self.deadline
    }
    pub fn interval(&self) -> u64 {
        self.interval
    }
    pub fn required_samples(&self) -> u32 {
        self.required
    }
    pub fn zero_samples(&self) -> u32 {
        self.zero_samples
    }
    /// Pure transition. Never infer completed goods from counters, disappearance,
    /// or endpoint subtraction. A terminal sample is historical, not fresh state.
    pub fn sampled(&self, next: OrderProgress) -> Result<Self> {
        if self.state.terminal() {
            return Ok(self.clone());
        }
        if next.order != self.baseline.order
            || next.generation != self.baseline.generation
            || next.folder != self.baseline.folder
            || next.site != self.baseline.site
            || next.sequence <= self.last.sequence
            || next.tick < self.last.tick
            || next.horizon < self.last.horizon
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "progress observation crossed identity, sequence, clock or allocation horizon; re-observe",
            ));
        }
        let mut out = self.clone();
        out.previous = Some(self.last.clone());
        out.last = next;
        if out.last.tick >= self.deadline {
            out.state = ProgressState::Expired;
            return Ok(out);
        }
        let Some(current) = out.last.counters() else {
            out.state = ProgressState::Missing;
            out.zero_samples = 0;
            return Ok(out);
        };
        let original = self.baseline.counters().ok_or_else(invalid)?;
        let previous = self.last.counters().ok_or_else(invalid)?;
        if current.template != original.template
            || current.job_type != original.job_type
            || current.remaining > previous.remaining
        {
            out.state = ProgressState::Changed;
            out.zero_samples = 0;
            return Ok(out);
        }
        if current.remaining != 0 {
            out.zero_samples = 0;
            out.state = ProgressState::Watching;
            out.credited_tick = out.last.tick;
        } else {
            // Same-tick repetitions and samples faster than the declared cadence
            // do not build temporal evidence. The baseline itself is not a sample.
            if out.last.tick.saturating_sub(out.credited_tick) >= out.interval {
                out.zero_samples += 1;
                out.credited_tick = out.last.tick;
            }
            out.state = if out.zero_samples >= out.required {
                ProgressState::CounterZeroStable
            } else {
                ProgressState::ZeroCandidate
            };
        }
        Ok(out)
    }
    pub fn cancelled(&self) -> Self {
        let mut out = self.clone();
        if !out.state.terminal() {
            out.state = ProgressState::Cancelled;
        }
        out
    }
}

#[cfg(test)]
#[path = "order_progress/tests.rs"]
mod tests;
