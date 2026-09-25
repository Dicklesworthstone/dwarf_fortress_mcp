//! Foreground excavation sessions over the existing durable 1.18 coordinator.
//!
//! A review is local and disposable. Only a fresh, consumed review can enter
//! `start`; recovered native preparation is never dispatch permission. The
//! backend owns each complete storage/native operation. No polling or task is
//! created here. Beads: df-dfhack-bridge-plane-c-pic.4/.5,
//! df-action-coordinator-exec-ero.4.
use super::coordinator::{ExcavationBinding, ExcavationEntry, MAX_ENTRIES};
use super::rpc::ExcavationCancellation;
use super::*;
use dfmcp_core::{Capability, OperationContext, RiskTier, SessionId};
use std::time::{Duration, Instant};

pub mod native;

pub const MAX_SESSION_BYTES: u64 = 64 * 1024 * 1024;
pub const VIEW_BYTES: u64 = 8 * 1024 * 1024;
pub const RESPONSE_BYTES: u64 = 32 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExcavationMode {
    Offline,
    Recover,
    Control,
}

/// A verified retained-state projection, NOT a canonical world snapshot or a
/// digest of the journal's physical frames. It grants no dispatch permission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExcavationInventory {
    binding: ExcavationBinding,
    entries: Vec<ExcavationEntry>,
    digest: Digest32,
    high_tick: u64,
}
impl ExcavationInventory {
    pub fn new(binding: ExcavationBinding, entries: Vec<ExcavationEntry>) -> Result<Self> {
        require(entries.len() <= MAX_ENTRIES, "excavation inventory exceeds 256 entries")?;
        let mut bytes = b"dfmcp-excavation-inventory/1\0".to_vec();
        fn field(out: &mut Vec<u8>, value: &[u8]) -> Result<()> {
            let length = u32::try_from(value.len()).map_err(|_| exhausted())?;
            out.extend_from_slice(&length.to_be_bytes());
            out.extend_from_slice(value);
            Ok(())
        }
        field(&mut bytes, binding.endpoint().to_string().as_bytes())?;
        field(&mut bytes, binding.fortress().folder().as_bytes())?;
        bytes.extend_from_slice(&binding.fortress().site().to_be_bytes());
        bytes.extend_from_slice(&binding.generation().to_be_bytes());
        for value in binding.dimensions() {
            bytes.extend_from_slice(&value.to_be_bytes());
        }
        field(&mut bytes, binding.df_version().as_bytes())?;
        field(&mut bytes, binding.dfhack_version().as_bytes())?;
        bytes.extend_from_slice(&(entries.len() as u32).to_be_bytes());
        let mut previous: Option<&str> = None;
        let mut high_tick = 0;
        for entry in &entries {
            let plan = entry.plan();
            if previous.is_some_and(|key| key >= plan.key())
                || plan.before().fortress() != binding.fortress()
                || plan.before().generation() != binding.generation()
                || plan.before().dimensions() != binding.dimensions()
            {
                return Err(corrupt());
            }
            previous = Some(plan.key());
            field(&mut bytes, &plan.canonical_bytes())?;
            bytes.push(u8::from(entry.dispatch_started()));
            bytes.push(u8::from(entry.cancel_requested()));
            high_tick = high_tick.max(plan.before().tick());
            if let Some(native) = entry.native() {
                require(native.plan() == plan, "inventory native plan differs")?;
                bytes.push(1);
                field(&mut bytes, native.canonical_bytes())?;
                high_tick = high_tick.max(native.last_capture_tick());
                if let Some(tick) = native.observed_tick() {
                    high_tick = high_tick.max(tick);
                }
            } else {
                bytes.push(0);
            }
        }
        require(bytes.len() <= 2 * 1024 * 1024, "inventory projection exceeds bound")?;
        Ok(Self { binding, entries, digest: Digest32::of_bytes(&bytes), high_tick })
    }
    pub fn binding(&self) -> &ExcavationBinding { &self.binding }
    pub fn entries(&self) -> &[ExcavationEntry] { &self.entries }
    pub fn digest(&self) -> Digest32 { self.digest }
    pub fn high_tick(&self) -> u64 { self.high_tick }
    pub fn pending_count(&self) -> usize {
        self.entries.iter().filter(|entry| entry.unresolved()).count()
    }
    pub fn entry(&self, key: &str) -> Option<&ExcavationEntry> {
        self.entries.iter().find(|entry| entry.plan().key() == key)
    }
    /// Within one session, retained plans cannot disappear, regress or change.
    /// A consistent rollback before a new process starts is not detected here.
    pub fn validate_successor(&self, next: &Self) -> Result<()> {
        if self.binding != next.binding || next.high_tick < self.high_tick {
            return Err(corrupt());
        }
        for old in &self.entries {
            let new = next.entry(old.plan().key()).ok_or_else(corrupt)?;
            if old.plan() != new.plan()
                || (old.dispatch_started() && !new.dispatch_started())
                || (old.cancel_requested() && !new.cancel_requested())
            {
                return Err(corrupt());
            }
            if let Some(before) = old.native() {
                before.validate_successor(new.native().ok_or_else(corrupt)?)?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct ExcavationObservation {
    pub binding: ExcavationBinding,
    pub capture: ExcavationCapture,
}

fn validate_observation(observed: &ExcavationObservation, inventory: &ExcavationInventory,
    region: ExcavationRegion, high_tick: u64) -> Result<()>
{
    let binding = inventory.binding();
    if &observed.binding != binding || observed.capture.region() != region
        || observed.capture.tick() < high_tick
        || observed.capture.fortress() != binding.fortress()
        || observed.capture.generation() != binding.generation()
        || observed.capture.dimensions() != binding.dimensions()
    { return Err(stale()); }
    Ok(())
}

/// Supplied by the supervising runtime. Cancellation must also be checked
/// inside native I/O; the returned handle must retain the parent's restrictions.
pub trait ExcavationSessionGuard {
    fn checkpoint(&self) -> Result<()>;
    fn allow_start(&self) -> Result<()>;
    fn cancellation(&self) -> ExcavationCancellation;
}

/// Trusted effect shell. Every mutating operation MUST re-open and verify the
/// exact expected inventory under exclusive custody before making a connection.
/// A read-only inspection must never create, sync, truncate or repair a journal.
pub trait ExcavationSessionBackend {
    fn fortress(&self) -> &FortressIdentity;
    fn region(&self) -> ExcavationRegion;
    fn inspect(&mut self, context: &OperationContext,
        guard: &dyn ExcavationSessionGuard) -> Result<ExcavationInventory>;
    fn initialize(&mut self, context: &OperationContext,
        guard: &dyn ExcavationSessionGuard) -> Result<ExcavationObservation>;
    fn observe(&mut self, expected: &ExcavationInventory, context: &OperationContext,
        guard: &dyn ExcavationSessionGuard) -> Result<ExcavationObservation>;
    fn start(&mut self, expected: &ExcavationInventory, plan: ExcavationRunPlan,
        context: &OperationContext, guard: &dyn ExcavationSessionGuard) -> Result<ExcavationRunRecord>;
    fn recover(&mut self, expected: &ExcavationInventory, key: &str, cancel: bool,
        context: &OperationContext, guard: &dyn ExcavationSessionGuard) -> Result<Option<ExcavationRunRecord>>;
}

#[derive(Clone, Debug)]
pub enum ExcavationCommand {
    Observe,
    Plan { key: String, witness: Digest32, spec: ExcavationRunSpec },
    Commit { key: String, digest: Digest32, confirmed: bool },
    Wait { key: String, digest: Digest32 },
    CancelEffect { key: String, digest: Digest32 },
    CancelPlan { key: String, digest: Digest32 },
    Inventory,
    Explain { key: String, digest: Digest32 },
    Release { for_recovery: bool },
}
#[derive(Debug)]
pub enum ExcavationOutcome {
    Observation(ExcavationCapture),
    Plan(ExcavationRunPlan),
    Effect { key: String, native_record_found: Option<bool> },
    PlanCancelled,
    Inventory,
    Released,
}
#[derive(Clone, Debug)]
pub struct ExcavationAttempt {
    pub key: String,
    pub digest: Digest32,
}
#[derive(Debug)]
pub struct ExcavationTurn {
    pub outcome: Result<ExcavationOutcome>,
    /// None means unverified, never an empty native-effect inventory.
    pub inventory: Option<ExcavationInventory>,
    pub historical_prior: Option<ExcavationInventory>,
    pub plan: Option<ExcavationRunPlan>,
    pub uncertain_attempt: Option<ExcavationAttempt>,
    pub native_operation_attempted: bool,
    pub released: bool,
}

fn exhausted() -> dfmcp_core::DfmcpError {
    error(ErrorCode::BudgetExceeded, "complete excavation request and response exceed their allowance")
}
fn corrupt() -> dfmcp_core::DfmcpError {
    error(ErrorCode::CorruptLedger, "retained excavation inventory changed or regressed; preserve journal")
}
fn denied() -> dfmcp_core::DfmcpError {
    error(ErrorCode::CapabilityDenied, "excavation session mode, owner or authority refused")
}
fn stale() -> dfmcp_core::DfmcpError {
    error(ErrorCode::StaleAnchor, "excavation observation or review changed; observe and review again")
}
fn query(context: &OperationContext, fortress: &FortressIdentity, tick: u64) -> Result<OperationContext> {
    if context.anchor.fortress_id != fortress.fortress_id() { return Err(denied()); }
    let mut current = context.clone();
    current.anchor.tick = dfmcp_core::GameTick(current.anchor.tick.get().max(tick));
    current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    Ok(current)
}

struct Call {
    context: OperationContext,
    deadline: Instant,
    child_bytes: u64,
}
impl Call {
    fn new(context: OperationContext, started: Instant) -> Result<Self> {
        context.budget.validate()?;
        if context.budget.max_wall_millis > 60000
            || context.budget.max_bytes > MAX_SESSION_BYTES
            || context.budget.max_game_ticks > 1200
            || u64::from(context.budget.max_output_tokens) * 4 < RESPONSE_BYTES
        { return Err(exhausted()); }
        let child_bytes = context.budget.max_bytes.checked_sub(2 * VIEW_BYTES + RESPONSE_BYTES)
            .filter(|n| *n > 0).ok_or_else(exhausted)?;
        let deadline = started.checked_add(Duration::from_millis(context.budget.max_wall_millis))
            .ok_or_else(exhausted)?;
        let call = Self { context, deadline, child_bytes };
        call.context(child_bytes, 0)?;
        Ok(call)
    }
    fn context(&self, bytes: u64, tick: u64) -> Result<OperationContext> {
        let remaining = self.deadline.checked_duration_since(Instant::now())
            .filter(|time| *time >= Duration::from_millis(1)).ok_or_else(exhausted)?;
        if self.context.cancellation_requested {
            return Err(error(ErrorCode::CancellationRequested, "excavation request cancelled"));
        }
        let mut current = self.context.clone();
        current.budget.max_wall_millis = u64::try_from(remaining.as_millis()).map_err(|_| exhausted())?;
        current.budget.max_bytes = bytes;
        current.anchor.tick = dfmcp_core::GameTick(current.anchor.tick.get().max(tick));
        Ok(current)
    }
    fn view(&self, tick: u64) -> Result<OperationContext> { self.context(VIEW_BYTES, tick) }
    fn work(&self, tick: u64) -> Result<OperationContext> { self.context(self.child_bytes, tick) }
}

/// One caller-owned session. Dropping it drops only local review/cache state;
/// the durable coordinator and native bounded-stop owner remain authoritative.
pub struct ExcavationSession<B> {
    backend: B,
    mode: ExcavationMode,
    owner: SessionId,
    last_request: u128,
    inventory: ExcavationInventory,
    selected: Option<ExcavationCapture>,
    review: Option<(Digest32, ExcavationRunPlan)>,
    uncertain_attempt: Option<ExcavationAttempt>,
    high_tick: u64,
    fenced: bool,
    released: bool,
}
impl<B: ExcavationSessionBackend> ExcavationSession<B> {
    pub fn open(mut backend: B, mode: ExcavationMode, initialize: bool,
        context: &OperationContext, started: Instant, guard: &dyn ExcavationSessionGuard) -> Result<Self>
    {
        guard.checkpoint()?;
        let context = query(context, backend.fortress(), 0)?;
        let call = Call::new(context.clone(), started)?;
        if mode == ExcavationMode::Control {
            context.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
            context.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
            guard.allow_start()?;
        }
        let observed = if initialize {
            if mode != ExcavationMode::Control { return Err(denied()); }
            context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
            Some(backend.initialize(&call.work(0)?, guard)?)
        } else { None };
        let inventory = backend.inspect(&call.view(0)?, guard)?;
        let mut high_tick = inventory.high_tick();
        if inventory.binding().fortress() != backend.fortress() { return Err(denied()); }
        let selected = if let Some(observed) = observed {
            validate_observation(&observed, &inventory, backend.region(), high_tick)?;
            high_tick = high_tick.max(observed.capture.tick());
            Some(observed.capture)
        } else { None };
        query(&call.view(high_tick)?, backend.fortress(), high_tick)?;
        guard.checkpoint()?;
        Ok(Self { backend, mode, owner: context.session_id, last_request: context.request_id.get(),
            inventory, selected, review: None, uncertain_attempt: None, high_tick,
            fenced: false, released: false })
    }
    pub fn mode(&self) -> ExcavationMode { self.mode }
    pub fn inventory(&self) -> &ExcavationInventory { &self.inventory }
    pub fn selected(&self) -> Option<&ExcavationCapture> { self.selected.as_ref() }
    pub fn plan(&self) -> Option<&ExcavationRunPlan> { self.review.as_ref().map(|(_, plan)| plan) }
    pub fn high_tick(&self) -> u64 { self.high_tick }
    pub fn is_released(&self) -> bool { self.released }
    pub fn is_fenced(&self) -> bool { self.fenced }

    fn accept(&mut self, next: ExcavationInventory) -> Result<()> {
        if let Err(cause) = self.inventory.validate_successor(&next) {
            self.fenced = true;
            self.selected = None;
            self.review = None;
            return Err(cause);
        }
        self.high_tick = self.high_tick.max(next.high_tick());
        self.inventory = next;
        Ok(())
    }
    fn exact(&self, key: &str, digest: Digest32) -> Result<&ExcavationEntry> {
        let entry = self.inventory.entry(key).ok_or_else(|| {
            error(ErrorCode::InvalidRequest, "excavation key is not in this journal")
        })?;
        if entry.plan().digest() != digest { return Err(stale()); }
        Ok(entry)
    }
    fn control(&self, context: &OperationContext) -> Result<()> {
        if self.mode != ExcavationMode::Control || self.fenced { return Err(denied()); }
        context.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)
    }
    fn authorize_plan(&self, plan: &ExcavationRunPlan, context: &OperationContext) -> Result<()> {
        self.control(context)?;
        super::coordinator::authorize_start(context, self.inventory.binding(), plan)
    }
    fn perform(&mut self, command: ExcavationCommand, call: &Call,
        guard: &dyn ExcavationSessionGuard, native_attempted: &mut bool) -> Result<ExcavationOutcome>
    {
        let context = query(&call.work(self.high_tick)?, self.backend.fortress(), self.high_tick)?;
        guard.checkpoint()?;
        match command {
            ExcavationCommand::Observe => {
                if self.mode != ExcavationMode::Control { return Err(denied()); }
                context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
                self.selected = None;
                self.review = None;
                *native_attempted = true;
                let observed = self.backend.observe(&self.inventory, &context, guard)?;
                validate_observation(&observed, &self.inventory, self.backend.region(), self.high_tick)?;
                self.high_tick = self.high_tick.max(observed.capture.tick());
                query(&context, self.backend.fortress(), self.high_tick)?;
                self.selected = Some(observed.capture.clone());
                Ok(ExcavationOutcome::Observation(observed.capture))
            }
            ExcavationCommand::Plan { key, witness, spec } => {
                self.control(&context)?;
                guard.allow_start()?;
                if self.inventory.pending_count() != 0 || self.uncertain_attempt.is_some()
                    || self.inventory.entry(&key).is_some() {
                    return Err(error(ErrorCode::Conflict, "retained or unresolved excavation work blocks a fresh plan"));
                }
                let before = self.selected.as_ref().filter(|capture| capture.witness() == witness)
                    .ok_or_else(stale)?.clone();
                let plan = ExcavationRunPlan::new(&key, spec, before)?;
                self.authorize_plan(&plan, &context)?;
                if self.review.as_ref().is_some_and(|(_, old)| old != &plan) {
                    return Err(error(ErrorCode::Conflict, "cancel the outstanding local review before replacing it"));
                }
                self.review = Some((self.inventory.digest(), plan.clone()));
                Ok(ExcavationOutcome::Plan(plan))
            }
            ExcavationCommand::Commit { key, digest, confirmed } => {
                self.control(&context)?;
                if !confirmed { return Err(denied()); }
                // A duplicate is a historical lookup, never a reconstructed permit.
                if self.inventory.entry(&key).is_some() {
                    self.exact(&key, digest)?;
                    return Ok(ExcavationOutcome::Effect { key, native_record_found: None });
                }
                if self.uncertain_attempt.is_some() || self.inventory.pending_count() != 0 {
                    return Err(error(ErrorCode::EffectIndeterminate, "recover unresolved work instead of committing"));
                }
                let (root, plan) = self.review.as_ref().ok_or_else(stale)?;
                if *root != self.inventory.digest() || plan.key() != key || plan.digest() != digest {
                    return Err(stale());
                }
                self.authorize_plan(plan, &context)?;
                guard.allow_start()?;
                let plan = plan.clone();
                // Consume BEFORE the effect shell. An error cannot restore this review.
                self.review = None;
                self.selected = None;
                self.uncertain_attempt = Some(ExcavationAttempt { key: key.clone(), digest });
                *native_attempted = true;
                let record = self.backend.start(&self.inventory, plan.clone(), &context, guard)?;
                if record.plan() != &plan { return Err(corrupt()); }
                Ok(ExcavationOutcome::Effect { key, native_record_found: Some(true) })
            }
            ExcavationCommand::Wait { key, digest }
            | ExcavationCommand::CancelEffect { key, digest } => {
                // The variant is split by execute; see recover_effect below.
                self.exact(&key, digest)?;
                Err(error(ErrorCode::InternalInvariantViolation, "unrouted excavation recovery request"))
            }
            ExcavationCommand::CancelPlan { key, digest } => {
                if self.mode != ExcavationMode::Control { return Err(denied()); }
                let (_, plan) = self.review.as_ref().ok_or_else(stale)?;
                if plan.key() != key || plan.digest() != digest { return Err(stale()); }
                self.review = None;
                self.selected = None;
                Ok(ExcavationOutcome::PlanCancelled)
            }
            ExcavationCommand::Inventory => Ok(ExcavationOutcome::Inventory),
            ExcavationCommand::Explain { key, digest } => {
                self.exact(&key, digest)?;
                Ok(ExcavationOutcome::Effect { key, native_record_found: None })
            }
            ExcavationCommand::Release { .. } => {
                if self.inventory.pending_count() != 0 || self.uncertain_attempt.is_some() {
                    return Err(error(ErrorCode::CancellationIncomplete,
                        "unresolved excavation work requires explicit release for recovery"));
                }
                self.review = None;
                self.selected = None;
                self.released = true;
                Ok(ExcavationOutcome::Released)
            }
        }
    }
    fn recover_effect(&mut self, key: String, digest: Digest32, cancel: bool,
        call: &Call, guard: &dyn ExcavationSessionGuard, attempted: &mut bool) -> Result<ExcavationOutcome>
    {
        let context = query(&call.work(self.high_tick)?, self.backend.fortress(), self.high_tick)?;
        if cancel { self.control(&context)?; }
        let entry = self.exact(&key, digest)?;
        // SourceLost is terminal historical evidence but remains unresolved.
        if entry.native().is_some_and(ExcavationRunRecord::terminal) {
            return Ok(ExcavationOutcome::Effect { key, native_record_found: None });
        }
        if self.mode == ExcavationMode::Offline { return Err(denied()); }
        guard.checkpoint()?;
        self.review = None;
        self.selected = None;
        *attempted = true;
        let result = self.backend.recover(&self.inventory, &key, cancel, &context, guard)?;
        Ok(ExcavationOutcome::Effect { key, native_record_found: Some(result.is_some()) })
    }
    /// One foreground turn. Both inventory reads and the single child operation
    /// share the request's original wall deadline and disjoint byte reservations.
    /// Forced release deliberately bypasses broken storage; it never cancels a run.
    pub fn execute(&mut self, command: ExcavationCommand, context: &OperationContext,
        started: Instant, guard: &dyn ExcavationSessionGuard) -> ExcavationTurn
    {
        let mut verified = false;
        let mut authorized = false;
        let mut native_attempted = false;
        let outcome = (|| {
            if self.released || context.session_id != self.owner
                || context.request_id.get() <= self.last_request { return Err(denied()); }
            self.last_request = context.request_id.get();
            match &command {
                ExcavationCommand::Plan { key, .. } | ExcavationCommand::Commit { key, .. }
                | ExcavationCommand::Wait { key, .. } | ExcavationCommand::CancelEffect { key, .. }
                | ExcavationCommand::CancelPlan { key, .. } | ExcavationCommand::Explain { key, .. } => validate_key(key)?,
                _ => {}
            }
            guard.checkpoint()?;
            let current = query(context, self.backend.fortress(), self.high_tick)?;
            authorized = true;
            let call = Call::new(current, started)?;
            if matches!(&command, ExcavationCommand::Release { for_recovery: true }) {
                self.review = None;
                self.selected = None;
                self.released = true;
                return Ok(ExcavationOutcome::Released);
            }
            if self.fenced { return Err(corrupt()); }
            let before = self.backend.inspect(&call.view(self.high_tick)?, guard)?;
            self.accept(before)?;
            query(&call.view(self.high_tick)?, self.backend.fortress(), self.high_tick)?;
            let result = match command {
                ExcavationCommand::Wait { key, digest } =>
                    self.recover_effect(key, digest, false, &call, guard, &mut native_attempted),
                ExcavationCommand::CancelEffect { key, digest } =>
                    self.recover_effect(key, digest, true, &call, guard, &mut native_attempted),
                other => self.perform(other, &call, guard, &mut native_attempted),
            };
            let after = self.backend.inspect(&call.view(self.high_tick)?, guard)?;
            self.accept(after)?;
            query(&call.view(self.high_tick)?, self.backend.fortress(), self.high_tick)?;
            guard.checkpoint()?;
            verified = true;
            if self.uncertain_attempt.as_ref().is_some_and(|attempt|
                self.inventory.entry(&attempt.key).is_some_and(|entry| entry.plan().digest() == attempt.digest)) {
                self.uncertain_attempt = None;
            }
            if self.review.as_ref().is_some_and(|(root, plan)|
                *root != self.inventory.digest() || plan.before().tick() != self.high_tick) {
                self.review = None;
                return Err(stale());
            }
            if let Ok(ExcavationOutcome::Effect { key, .. }) = &result {
                if self.inventory.entry(key).is_none() { return Err(corrupt()); }
            }
            result
        })();
        // A newer evidence floor may have exhausted authority. Do not return
        // prior rows merely because the request was authorized before acquisition.
        authorized &= query(context, self.backend.fortress(), self.high_tick).is_ok();
        if !verified && native_attempted {
            self.selected = None;
            self.review = None;
        }
        ExcavationTurn {
            outcome,
            inventory: (verified && authorized).then(|| self.inventory.clone()),
            historical_prior: (!verified && authorized).then(|| self.inventory.clone()),
            plan: if authorized { self.plan().cloned() } else { None },
            uncertain_attempt: if authorized { self.uncertain_attempt.clone() } else { None },
            native_operation_attempted: native_attempted,
            released: self.released,
        }
    }
}

#[cfg(test)]
mod tests;
