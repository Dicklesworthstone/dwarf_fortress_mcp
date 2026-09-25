use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ExcavationTrigger {
    None = 0,
    FloorObserved = 1,
    SourceChanged = 2,
    CaptureFailure = 3,
    Unobservable = 4,
    LiquidObserved = 5,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExcavationRunRecord {
    bytes: Vec<u8>,
    plan: ExcavationRunPlan,
    phase: RunPhase,
    reason: RunReason,
    trigger: ExcavationTrigger,
    observed_tick: Option<u64>,
    stable_samples: u32,
    first_stable_tick: u64,
    counted_tick: u64,
    last_tick: u64,
    sample: Option<ExcavationCapture>,
    receipt: Digest32,
}
impl ExcavationRunRecord {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        require(bytes.len() <= MAX_RECORD_BYTES, "excavation receipt exceeds 3 KiB")?;
        let mut r = Reader(bytes);
        require(r.take(8)? == b"DFMER018", "not an excavation-run/1.18 receipt")?;
        let key = text(field(&mut r, 128)?, 128)?;
        let spec = ExcavationRunSpec::read(&mut r)?;
        let before = ExcavationCapture::decode(field(&mut r, MAX_CAPTURE_BYTES)?)?;
        let plan = ExcavationRunPlan::new(&key, spec, before)?;
        require(r.take(32)? == plan.digest().as_bytes() && r.take(16)? == plan.token(),
            "excavation receipt plan/token mismatch")?;
        let phase = match r.byte()? {
            0 => RunPhase::Prepared, 1 => RunPhase::Running, 2 => RunPhase::Stopping,
            3 => RunPhase::Stopped, 4 => RunPhase::Refused, 5 => RunPhase::SourceLost,
            _ => return Err(error(ErrorCode::AdapterRejected, "unknown excavation phase")),
        };
        let reason = match r.byte()? {
            0 => RunReason::None, 1 => RunReason::TickLimit, 2 => RunReason::WallLimit,
            3 => RunReason::Cancelled, 4 => RunReason::ExternalPause, 5 => RunReason::NativeFailure,
            6 => RunReason::ClockRegression, 7 => RunReason::SourceChanged,
            8 => RunReason::Shutdown, 9 => RunReason::Stale,
            _ => return Err(error(ErrorCode::AdapterRejected, "unknown excavation reason")),
        };
        let attempted = r.boolean()?;
        let verified = r.boolean()?;
        let known = r.boolean()?;
        let tick = r.u64()?;
        require(if known { tick <= MAX_NATIVE_TICK } else { tick == 0 }, "fabricated tick")?;
        let trigger = match r.byte()? {
            0 => ExcavationTrigger::None, 1 => ExcavationTrigger::FloorObserved,
            2 => ExcavationTrigger::SourceChanged, 3 => ExcavationTrigger::CaptureFailure,
            4 => ExcavationTrigger::Unobservable, 5 => ExcavationTrigger::LiquidObserved,
            _ => return Err(error(ErrorCode::AdapterRejected, "unknown excavation trigger")),
        };
        let stable_samples = r.u32()?;
        let first_stable_tick = r.u64()?;
        let counted_tick = r.u64()?;
        let last_tick = r.u64()?;
        let sample = if r.boolean()? {
            Some(ExcavationCapture::decode(field(&mut r, MAX_CAPTURE_BYTES)?)?)
        } else { None };
        let receipt = Digest32::from_bytes(r.array()?);
        r.finish()?;
        require(receipt == hash(b"dfmcp-excavation-run-receipt/1", &bytes[..bytes.len() - 32]),
            "excavation receipt checksum mismatch")?;
        require(attempted == matches!(phase, RunPhase::Running | RunPhase::Stopping
            | RunPhase::Stopped | RunPhase::SourceLost) && verified == (phase == RunPhase::Stopped),
            "excavation phase/effect flags disagree")?;
        let allowed = match phase {
            RunPhase::Prepared => reason == RunReason::None && !known,
            RunPhase::Running => reason == RunReason::None && known && tick >= plan.before.tick(),
            RunPhase::Stopping | RunPhase::Stopped => matches!(reason, RunReason::TickLimit
                | RunReason::WallLimit | RunReason::Cancelled | RunReason::NativeFailure
                | RunReason::ClockRegression | RunReason::Shutdown)
                || (phase == RunPhase::Stopped && reason == RunReason::ExternalPause),
            RunPhase::Refused => !known && matches!(reason, RunReason::Cancelled
                | RunReason::SourceChanged | RunReason::Stale),
            RunPhase::SourceLost => !known && reason == RunReason::SourceChanged,
        };
        require(allowed, "impossible excavation phase/reason")?;
        let out = Self { bytes: bytes.to_vec(), plan, phase, reason, trigger,
            observed_tick: known.then_some(tick), stable_samples, first_stable_tick,
            counted_tick, last_tick, sample, receipt };
        out.validate_window()?;
        Ok(out)
    }
    fn validate_window(&self) -> Result<()> {
        let before = self.plan.before();
        let spec = self.plan.spec();
        require(self.stable_samples <= spec.samples() && before.tick() <= self.counted_tick
            && self.counted_tick <= self.last_tick, "invalid sample clock order")?;
        require(if self.stable_samples == 0 { self.first_stable_tick == 0 } else {
            before.tick() < self.first_stable_tick && self.first_stable_tick <= self.counted_tick
        }, "invalid stable window origin")?;
        if let Some(sample) = &self.sample {
            require(sample.region() == before.region() && sample.same_source(before)
                && sample.sequence() == before.sequence() + 1 && !sample.paused()
                && sample.tick() == self.last_tick
                && self.last_tick < before.tick() + u64::from(spec.clock().game_ticks()),
                "sample differs from planned source or horizon")?;
        } else {
            require(self.stable_samples == 0 && self.last_tick == before.tick()
                && self.counted_tick == before.tick(), "missing sample payload")?;
        }
        if matches!(self.phase, RunPhase::Prepared | RunPhase::Refused) {
            require(self.sample.is_none() && self.trigger == ExcavationTrigger::None,
                "predispatch record claims sampling")?;
        }
        if self.trigger != ExcavationTrigger::None {
            require(matches!(self.phase, RunPhase::Stopping | RunPhase::Stopped | RunPhase::SourceLost),
                "trigger without a stop")?;
        }
        let matches = self.sample.as_ref().is_some_and(ExcavationCapture::floor_observed);
        if self.stable_samples > 0 {
            require(matches && self.first_stable_tick >= before.tick() + u64::from(spec.interval())
                && self.counted_tick - self.first_stable_tick
                    >= u64::from(self.stable_samples - 1) * u64::from(spec.interval()),
                "impossible sampled stability")?;
        }
        match self.trigger {
            ExcavationTrigger::None => {}
            ExcavationTrigger::SourceChanged => require(self.phase == RunPhase::SourceLost,
                "source trigger without source loss")?,
            trigger => {
                require(self.phase == RunPhase::SourceLost || self.reason == RunReason::Cancelled,
                    "sample trigger did not request cancellation")?;
                if trigger == ExcavationTrigger::FloorObserved {
                    require(matches && self.stable_samples == spec.samples()
                        && self.counted_tick == self.last_tick
                        && self.counted_tick - self.first_stable_tick >= u64::from(spec.stable_ticks()),
                        "floor trigger lacks required sampled window")?;
                } else {
                    require(self.stable_samples == 0, "failed sampling retained a streak")?;
                    if trigger == ExcavationTrigger::Unobservable {
                        require(self.sample.as_ref().is_some_and(|s| s.cells().iter().any(|c|
                            matches!(c, ExcavationCell::Missing | ExcavationCell::Hidden))),
                            "unobservable trigger lacks witness")?;
                    } else if trigger == ExcavationTrigger::LiquidObserved {
                        require(self.sample.as_ref().is_some_and(|s|
                            s.cells().iter().all(|c| matches!(c, ExcavationCell::Visible { .. }))
                            && s.cells().iter().any(|c| matches!(c,
                                ExcavationCell::Visible { liquid, .. } if *liquid > 0))),
                            "liquid trigger lacks witness")?;
                    }
                }
            }
        }
        Ok(())
    }
    /// A skipped observation cannot be invented. Only contradictions are rejected.
    pub fn validate_successor(&self, next: &Self) -> Result<()> {
        require(self.plan == next.plan, "retained excavation plan changed")?;
        if self.terminal() { return require(self.bytes == next.bytes, "terminal receipt changed"); }
        require(match self.phase {
            RunPhase::Prepared => true,
            RunPhase::Running => !matches!(next.phase, RunPhase::Prepared | RunPhase::Refused),
            RunPhase::Stopping => matches!(next.phase, RunPhase::Stopping | RunPhase::Stopped | RunPhase::SourceLost),
            _ => false,
        }, "excavation phase regressed")?;
        require(self.trigger == ExcavationTrigger::None || self.trigger == next.trigger,
            "retained stop trigger changed")?;
        require(self.phase != RunPhase::Stopping || next.phase == RunPhase::SourceLost
            || self.reason == next.reason, "retained stop reason changed")?;
        require(next.last_tick >= self.last_tick, "retained sample regressed")?;
        if self.sample.is_some() && self.last_tick == next.last_tick {
            require(next.stable_samples <= self.stable_samples, "same-tick sample inflation")?;
        }
        if let Some(tick) = self.observed_tick {
            require(next.phase != RunPhase::Running || next.observed_tick.is_some_and(|n| n >= tick),
                "running clock regressed")?;
        }
        Ok(())
    }
    pub fn canonical_bytes(&self) -> &[u8] { &self.bytes }
    pub fn plan(&self) -> &ExcavationRunPlan { &self.plan }
    pub fn phase(&self) -> RunPhase { self.phase }
    pub fn reason(&self) -> RunReason { self.reason }
    pub fn trigger(&self) -> ExcavationTrigger { self.trigger }
    pub fn observed_tick(&self) -> Option<u64> { self.observed_tick }
    pub fn stable_samples(&self) -> u32 { self.stable_samples }
    pub fn first_stable_tick(&self) -> u64 { self.first_stable_tick }
    pub fn counted_tick(&self) -> u64 { self.counted_tick }
    pub fn last_capture_tick(&self) -> u64 { self.last_tick }
    pub fn sample(&self) -> Option<&ExcavationCapture> { self.sample.as_ref() }
    pub fn receipt(&self) -> Digest32 { self.receipt }
    pub fn terminal(&self) -> bool { self.phase.terminal() }
    pub fn resolved(&self) -> bool { matches!(self.phase, RunPhase::Stopped | RunPhase::Refused) }
    pub fn historical_pause_verified(&self) -> bool { self.phase == RunPhase::Stopped }
    pub fn sampled_floor_reported(&self) -> bool { self.trigger == ExcavationTrigger::FloorObserved }
}
