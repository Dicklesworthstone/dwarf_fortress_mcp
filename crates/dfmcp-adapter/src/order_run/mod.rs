#![forbid(unsafe_code)]
//! Fortress-bound sampled order conditions for native order-run/1.14.
//!
//! A predicate receipt, a pause readback and produced goods are different facts.
//! Native stability is a reported sampled count, not a complete sample trace.

pub mod rpc;

pub use crate::bounded_run::{RunPhase, RunReason, RunSpec};
use crate::bounded_run::{Reader, error, hash, require, validate_key};
use dfmcp_core::{Digest32, ErrorCode, FortressId, Result};

pub const MAX_CAPTURE_BYTES: usize = 573;
pub const MAX_PLAN_BYTES: usize = 604;
pub const MAX_INTENT_BYTES: usize = 736;
pub const MAX_RECORD_BYTES: usize = 1425;
pub const MAX_NATIVE_TICK: u64 = crate::bounded_run::MAX_NATIVE_TICK;

pub(crate) fn field<'a>(r: &mut Reader<'a>, maximum: usize) -> Result<&'a [u8]> {
    let size = usize::from(u16::from_be_bytes(r.array()?));
    require(size <= maximum, "order-run field exceeds its bound")?;
    r.take(size)
}
pub(crate) fn put_field(out: &mut Vec<u8>, bytes: &[u8]) {
    // All callers hold sealed fields smaller than 2 KiB.
    out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    out.extend_from_slice(bytes);
}
pub(crate) fn text(bytes: &[u8], maximum: usize) -> Result<String> {
    require(!bytes.is_empty() && bytes.len() <= maximum, "invalid order-run text length")?;
    let value = std::str::from_utf8(bytes)
        .map_err(|_| error(ErrorCode::AdapterRejected, "invalid order-run UTF-8"))?;
    require(!value.contains('\0'), "NUL in order-run text")?;
    Ok(value.to_owned())
}

/// Exact native folder/site identity. The numeric ID uses the existing live
/// lineage domain; exact folder/site bytes are still checked at every boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FortressIdentity { folder: String, site: u32 }
impl FortressIdentity {
    pub fn new(folder: &str, site: u32) -> Result<Self> {
        let folder = text(folder.as_bytes(), 512)?;
        require(site <= i32::MAX as u32, "native site ID out of range")?;
        Ok(Self { folder, site })
    }
    pub fn folder(&self) -> &str { &self.folder }
    pub fn site(&self) -> u32 { self.site }
    pub fn fortress_id(&self) -> FortressId {
        let mut bytes = b"dfmcp-live-fortress-id-v1\0".to_vec();
        bytes.extend_from_slice(self.folder.as_bytes()); bytes.push(0);
        bytes.extend_from_slice(&self.site.to_be_bytes());
        let digest = Digest32::of_bytes(&bytes);
        let mut id = [0; 8]; id.copy_from_slice(&digest.as_bytes()[..8]);
        FortressId::new(u64::from_be_bytes(id) | 1)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderCapture {
    bytes: Vec<u8>, fortress: FortressIdentity, generation: u64, sequence: u64, tick: u64,
    paused: bool, order_id: u32, horizon: u32, present: bool,
    recipe: u8, total: i32, remaining: i32, status: u32,
}
impl OrderCapture {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        require((62..=MAX_CAPTURE_BYTES).contains(&bytes.len()), "order-run capture size")?;
        let mut r = Reader(bytes);
        require(r.take(8)? == b"DFMOR014", "not an order-run/1.14 capture")?;
        let generation = r.u64()?; let sequence = r.u64()?; let tick = r.u64()?;
        let site = r.u32()?; let folder = text(field(&mut r, 512)?, 512)?;
        let fortress = FortressIdentity::new(&folder, site)?;
        let paused = r.boolean()?; let order_id = r.u32()?; let horizon = r.u32()?;
        let present = r.boolean()?; let recipe = r.byte()?;
        let total = i32::from_be_bytes(r.array()?); let remaining = i32::from_be_bytes(r.array()?);
        let status = r.u32()?; r.finish()?;
        require(generation > 0 && generation < u64::MAX && tick <= MAX_NATIVE_TICK,
            "invalid order-run incarnation/clock")?;
        require(order_id <= i32::MAX as u32 && horizon <= i32::MAX as u32 && recipe <= 4,
            "order-run native ID or recipe out of range")?;
        if present {
            require(order_id < horizon && i16::try_from(total).is_ok() && i16::try_from(remaining).is_ok(),
                "order-run counters or allocation horizon invalid")?;
            require(recipe == 0 || ((1..=100).contains(&total) && (0..=total).contains(&remaining) && status & !3 == 0),
                "recognized order-run template has invalid counters/status")?;
        } else {
            require(recipe == 0 && total == 0 && remaining == 0 && status == 0,
                "absent order-run target has backing fields")?;
        }
        Ok(Self { bytes: bytes.to_vec(), fortress, generation, sequence, tick, paused,
            order_id, horizon, present, recipe, total, remaining, status })
    }
    pub fn canonical_bytes(&self) -> &[u8] { &self.bytes }
    pub fn witness(&self) -> Digest32 { Digest32::of_bytes(&self.bytes) }
    pub fn fortress(&self) -> &FortressIdentity { &self.fortress }
    pub fn generation(&self) -> u64 { self.generation }
    pub fn sequence(&self) -> u64 { self.sequence }
    pub fn tick(&self) -> u64 { self.tick }
    pub fn paused(&self) -> bool { self.paused }
    pub fn order_id(&self) -> u32 { self.order_id }
    pub fn horizon(&self) -> u32 { self.horizon }
    pub fn present(&self) -> bool { self.present }
    pub fn recipe(&self) -> u8 { self.recipe }
    pub fn total(&self) -> i32 { self.total }
    pub fn remaining(&self) -> i32 { self.remaining }
    pub fn status(&self) -> u32 { self.status }
    pub fn eligible(&self) -> bool { self.paused && self.present && self.recipe != 0 && self.sequence != u64::MAX }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderPredicate { Approved, Active, RemainingAtMost(u32) }
impl OrderPredicate {
    pub fn from_code(code: u32, threshold: u32) -> Result<Self> {
        match (code, threshold) {
            (1, 0) => Ok(Self::Approved), (2, 0) => Ok(Self::Active),
            (3, 0..=100) => Ok(Self::RemainingAtMost(threshold)),
            _ => Err(error(ErrorCode::InvalidRequest, "unknown predicate or invalid threshold")),
        }
    }
    pub fn code(self) -> u8 { match self { Self::Approved => 1, Self::Active => 2, Self::RemainingAtMost(_) => 3 } }
    pub fn threshold(self) -> u32 { match self { Self::RemainingAtMost(n) => n, _ => 0 } }
    pub fn as_str(self) -> &'static str {
        match self { Self::Approved => "approved", Self::Active => "active", Self::RemainingAtMost(_) => "remaining_at_most" }
    }
    pub fn observed(self, value: &OrderCapture) -> bool {
        value.present && value.recipe != 0 && match self {
            Self::Approved => value.status & 1 != 0, Self::Active => value.status & 2 != 0,
            Self::RemainingAtMost(n) => i64::from(value.remaining) <= i64::from(n),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrderRunSpec { clock: RunSpec, predicate: OrderPredicate, samples: u32, interval: u32 }
impl OrderRunSpec {
    pub fn new(ticks: u32, wall_ms: u32, predicate: OrderPredicate, samples: u32, interval: u32) -> Result<Self> {
        let clock = RunSpec::new(ticks, wall_ms)?;
        OrderPredicate::from_code(u32::from(predicate.code()), predicate.threshold())?;
        if !(1..=16).contains(&samples) || !(1..=1200).contains(&interval)
            || u64::from(samples) * u64::from(interval) > u64::from(ticks)
        { return Err(error(ErrorCode::InvalidRequest, "sample cadence exceeds bounded run horizon")); }
        Ok(Self { clock, predicate, samples, interval })
    }
    pub fn game_ticks(self) -> u32 { self.clock.game_ticks() }
    pub fn wall_ms(self) -> u32 { self.clock.wall_ms() }
    pub fn predicate(self) -> OrderPredicate { self.predicate }
    pub fn samples(self) -> u32 { self.samples }
    pub fn interval(self) -> u32 { self.interval }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderRunPlan {
    key: String, spec: OrderRunSpec, before: OrderCapture, bytes: Vec<u8>, digest: Digest32, token: [u8; 16],
}
impl OrderRunPlan {
    pub fn new(key: &str, spec: OrderRunSpec, before: OrderCapture) -> Result<Self> {
        validate_key(key)?;
        if !before.eligible() || i64::from(spec.predicate.threshold()) > i64::from(before.total)
            || spec.predicate.observed(&before)
        { return Err(error(ErrorCode::StaleAnchor, "conditional run requires a paused recognized order and a not-yet-observed predicate")); }
        let mut bytes = b"DFMOP014".to_vec();
        for n in [spec.game_ticks(), spec.wall_ms()] { bytes.extend_from_slice(&n.to_be_bytes()); }
        bytes.push(spec.predicate.code());
        for n in [spec.predicate.threshold(), spec.samples, spec.interval] { bytes.extend_from_slice(&n.to_be_bytes()); }
        put_field(&mut bytes, before.canonical_bytes());
        let digest = hash(b"dfmcp-order-run-plan/1", &bytes);
        let mut keyed = Vec::new(); put_field(&mut keyed, key.as_bytes()); keyed.extend_from_slice(digest.as_bytes());
        let commitment = hash(b"dfmcp-order-run-token/1", &keyed);
        let mut token = [0; 16]; token.copy_from_slice(&commitment.as_bytes()[..16]);
        Ok(Self { key: key.to_owned(), spec, before, bytes, digest, token })
    }
    pub fn from_native(key: &str, bytes: &[u8]) -> Result<Self> {
        require((93..=MAX_PLAN_BYTES).contains(&bytes.len()), "order-run plan size")?;
        let mut r = Reader(bytes); require(r.take(8)? == b"DFMOP014", "not an order-run/1.14 plan")?;
        let ticks = r.u32()?; let wall = r.u32()?; let code = r.byte()?;
        let predicate = OrderPredicate::from_code(u32::from(code), r.u32()?)?;
        let spec = OrderRunSpec::new(ticks, wall, predicate, r.u32()?, r.u32()?)?;
        let before = OrderCapture::decode(field(&mut r, MAX_CAPTURE_BYTES)?)?; r.finish()?;
        let result = Self::new(key, spec, before)?;
        require(result.bytes == bytes, "noncanonical order-run plan")?; Ok(result)
    }
    pub fn key(&self) -> &str { &self.key }
    pub fn spec(&self) -> OrderRunSpec { self.spec }
    pub fn before(&self) -> &OrderCapture { &self.before }
    pub fn native_bytes(&self) -> &[u8] { &self.bytes }
    pub fn digest(&self) -> Digest32 { self.digest }
    pub fn token(&self) -> &[u8; 16] { &self.token }
    /// Keyed durable identity; deliberately distinct from the native plan bytes.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new(); put_field(&mut out, self.key.as_bytes()); put_field(&mut out, &self.bytes); out
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        require(bytes.len() <= MAX_INTENT_BYTES, "order-run intent size")?;
        let mut r = Reader(bytes); let key = text(field(&mut r, 128)?, 128)?;
        let plan = Self::from_native(&key, field(&mut r, MAX_PLAN_BYTES)?)?; r.finish()?; Ok(plan)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum OrderTrigger {
    None = 0, PredicateObserved = 1, TargetAbsent = 2, TargetChanged = 3,
    CounterRegression = 4, HorizonRegression = 5, SourceChanged = 6, NativeFailure = 7,
}
impl OrderTrigger {
    pub fn as_str(self) -> &'static str {
        match self { Self::None => "none", Self::PredicateObserved => "predicate_observed",
            Self::TargetAbsent => "target_absent", Self::TargetChanged => "target_changed",
            Self::CounterRegression => "counter_regression", Self::HorizonRegression => "horizon_regression",
            Self::SourceChanged => "source_changed", Self::NativeFailure => "native_failure" }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderRunRecord {
    bytes: Vec<u8>, plan: OrderRunPlan, phase: RunPhase, reason: RunReason, trigger: OrderTrigger,
    observed_tick: Option<u64>, count: u32, counted_tick: u64, sample: Option<OrderCapture>, receipt: Digest32,
}
impl OrderRunRecord {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        require((214..=MAX_RECORD_BYTES).contains(&bytes.len()), "order-run receipt size")?;
        let mut r = Reader(bytes); require(r.take(8)? == b"DFMOE014", "not an order-run/1.14 receipt")?;
        let key = text(field(&mut r, 128)?, 128)?;
        let plan = OrderRunPlan::from_native(&key, field(&mut r, MAX_PLAN_BYTES)?)?;
        require(r.take(32)? == plan.digest.as_bytes() && r.take(16)? == plan.token,
            "order-run receipt differs from its plan/token")?;
        let phase = match r.byte()? {
            0 => RunPhase::Prepared, 1 => RunPhase::Running, 2 => RunPhase::Stopping, 3 => RunPhase::Stopped,
            4 => RunPhase::Refused, 5 => RunPhase::SourceLost, _ => return Err(error(ErrorCode::AdapterRejected, "unknown clock phase")),
        };
        let reason = match r.byte()? {
            0 => RunReason::None, 1 => RunReason::TickLimit, 2 => RunReason::WallLimit, 3 => RunReason::Cancelled,
            4 => RunReason::ExternalPause, 5 => RunReason::NativeFailure, 6 => RunReason::ClockRegression,
            7 => RunReason::SourceChanged, 8 => RunReason::Shutdown, 9 => RunReason::Stale,
            _ => return Err(error(ErrorCode::AdapterRejected, "unknown clock reason")),
        };
        let trigger = match r.byte()? {
            0 => OrderTrigger::None, 1 => OrderTrigger::PredicateObserved, 2 => OrderTrigger::TargetAbsent,
            3 => OrderTrigger::TargetChanged, 4 => OrderTrigger::CounterRegression, 5 => OrderTrigger::HorizonRegression,
            6 => OrderTrigger::SourceChanged, 7 => OrderTrigger::NativeFailure,
            _ => return Err(error(ErrorCode::AdapterRejected, "unknown order trigger")),
        };
        let attempted = r.boolean()?; let verified = r.boolean()?; let known = r.boolean()?;
        let tick = r.u64()?; let count = r.u32()?; let counted_tick = r.u64()?;
        let sample_bytes = field(&mut r, MAX_CAPTURE_BYTES)?;
        let sample = if sample_bytes.is_empty() { None } else { Some(OrderCapture::decode(sample_bytes)?) };
        let receipt = Digest32::from_bytes(r.array()?); r.finish()?;
        require(receipt == hash(b"dfmcp-order-run-receipt/1", &bytes[..bytes.len()-32]), "order-run receipt checksum mismatch")?;
        require(attempted == matches!(phase, RunPhase::Running | RunPhase::Stopping | RunPhase::Stopped | RunPhase::SourceLost)
            && verified == (phase == RunPhase::Stopped), "impossible clock effect flags")?;
        require(if known { tick <= MAX_NATIVE_TICK } else { tick == 0 }, "unknown tick has nonzero backing")?;
        let before = plan.before(); let spec = plan.spec();
        require(count <= spec.samples && counted_tick <= MAX_NATIVE_TICK
            && counted_tick >= before.tick + u64::from(count) * u64::from(spec.interval), "invalid sampled stability count")?;
        if let Some(sample) = &sample {
            require(sample.fortress == before.fortress && sample.generation == before.generation
                && sample.order_id == before.order_id && sample.sequence == before.sequence + 1 && !sample.paused
                && before.tick <= counted_tick && counted_tick <= sample.tick
                && sample.tick < before.tick + u64::from(spec.game_ticks()), "sample outside exact sealed run")?;
        } else { require(count == 0 && counted_tick == before.tick, "stability without native sample")?; }
        let phase_valid = match phase {
            RunPhase::Prepared | RunPhase::Refused => !known && trigger == OrderTrigger::None && sample.is_none()
                && if phase == RunPhase::Prepared { reason == RunReason::None }
                    else { matches!(reason, RunReason::Cancelled | RunReason::SourceChanged | RunReason::Stale) },
            RunPhase::Running => reason == RunReason::None && trigger == OrderTrigger::None
                && known && tick >= before.tick && count < spec.samples,
            RunPhase::Stopping | RunPhase::Stopped => trigger != OrderTrigger::SourceChanged
                && (matches!(reason, RunReason::TickLimit | RunReason::WallLimit | RunReason::Cancelled
                    | RunReason::NativeFailure | RunReason::ClockRegression | RunReason::Shutdown)
                    || (phase == RunPhase::Stopped && reason == RunReason::ExternalPause)),
            RunPhase::SourceLost => reason == RunReason::SourceChanged && !known,
        };
        require(phase_valid, "impossible order-run phase/reason")?;
        if matches!(trigger, OrderTrigger::PredicateObserved | OrderTrigger::TargetAbsent | OrderTrigger::TargetChanged
            | OrderTrigger::CounterRegression | OrderTrigger::HorizonRegression)
        {
            require(matches!(phase, RunPhase::Stopping | RunPhase::Stopped | RunPhase::SourceLost)
                && (phase == RunPhase::SourceLost || reason == RunReason::Cancelled) && sample.is_some(),
                "order trigger without sample and safety-stop request")?;
        }
        if trigger == OrderTrigger::PredicateObserved {
            let s = sample.as_ref().ok_or_else(|| error(ErrorCode::AdapterRejected, "predicate has no sample"))?;
            require(count == spec.samples && counted_tick == s.tick && spec.predicate.observed(s)
                && s.recipe == before.recipe && s.total == before.total && s.remaining <= before.remaining
                && s.horizon >= before.horizon, "predicate sample does not prove reported condition")?;
        } else { require(count < spec.samples, "terminal stability count without predicate trigger")?; }
        match trigger {
            OrderTrigger::TargetAbsent => require(sample.as_ref().is_some_and(|s| !s.present), "absence trigger has a present target")?,
            OrderTrigger::TargetChanged => require(sample.as_ref().is_some_and(|s| s.present
                && (s.recipe == 0 || s.recipe != before.recipe || s.total != before.total)), "unchanged template trigger")?,
            OrderTrigger::SourceChanged => require(phase == RunPhase::SourceLost, "source trigger without source loss")?,
            OrderTrigger::NativeFailure => require(matches!(phase, RunPhase::Stopping | RunPhase::Stopped | RunPhase::SourceLost),
                "native failure trigger not stopping")?,
            _ => {}, // Regression claims need the missing previous sample; never infer it.
        }
        Ok(Self { bytes: bytes.to_vec(), plan, phase, reason, trigger, observed_tick: known.then_some(tick),
            count, counted_tick, sample, receipt })
    }
    pub fn canonical_bytes(&self) -> &[u8] { &self.bytes }
    pub fn plan(&self) -> &OrderRunPlan { &self.plan }
    pub fn phase(&self) -> RunPhase { self.phase }
    pub fn reason(&self) -> RunReason { self.reason }
    pub fn trigger(&self) -> OrderTrigger { self.trigger }
    pub fn unpause_attempted(&self) -> bool { !matches!(self.phase, RunPhase::Prepared | RunPhase::Refused) }
    pub fn pause_verified(&self) -> bool { self.phase == RunPhase::Stopped }
    pub fn observed_tick(&self) -> Option<u64> { self.observed_tick }
    pub fn reported_stable_samples(&self) -> u32 { self.count }
    pub fn counted_tick(&self) -> u64 { self.counted_tick }
    pub fn sample(&self) -> Option<&OrderCapture> { self.sample.as_ref() }
    pub fn receipt(&self) -> Digest32 { self.receipt }
    pub fn predicate_observed(&self) -> bool { self.trigger == OrderTrigger::PredicateObserved }
    pub fn observed_tick_overshoot(&self) -> Option<u64> {
        Some(self.observed_tick?.checked_sub(self.plan.before.tick)?.saturating_sub(u64::from(self.plan.spec.game_ticks())))
    }
    pub fn validate_successor(&self, next: &Self) -> Result<()> {
        require(self.plan == next.plan, "native order-run receipt changed its sealed plan")?;
        if self.phase.terminal() { return require(self == next, "native terminal receipt changed"); }
        if self.unpause_attempted() { require(next.unpause_attempted(), "native record regressed before dispatch")?; }
        if self.phase == RunPhase::Stopping {
            require(matches!(next.phase, RunPhase::Stopping | RunPhase::Stopped | RunPhase::SourceLost), "native stop regressed")?;
        }
        if self.trigger != OrderTrigger::None {
            require(self.trigger == next.trigger && self.sample == next.sample
                && self.count == next.count && self.counted_tick == next.counted_tick, "native trigger evidence changed")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
