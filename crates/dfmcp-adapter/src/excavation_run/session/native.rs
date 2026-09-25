//! Actual private-file/RPC effect shell. Every effect revalidates the expected
//! inventory while the original directory and journal are exclusively locked.
use super::*;
use crate::control_effect_journal::EffectJournalStorage;
use crate::excavation_run::coordinator::{ExcavationCoordinator, ExcavationDispatch, ExcavationRunSource};
use crate::excavation_run::private_file::{
    create_private_excavation, inspect_private_excavation, open_private_excavation,
};
use crate::excavation_run::rpc::ExcavationRpc;
use std::path::PathBuf;

const RPC_BYTES: u64 = 272 * 1024;

/// Trusted operator configuration, never an MCP-selected path or native method.
#[derive(Clone, Debug)]
pub struct PrivateExcavationBackend {
    directory: PathBuf,
    fortress: FortressIdentity,
    region: ExcavationRegion,
}
impl PrivateExcavationBackend {
    pub fn new(directory: PathBuf, fortress: FortressIdentity, region: ExcavationRegion) -> Result<Self> {
        let raw = directory.to_str().ok_or_else(denied)?;
        if !directory.is_absolute() || raw.len() > 4096 || raw.contains('\0')
            || raw[1..].split('/').any(|part| part.is_empty() || part == "." || part == "..")
        { return Err(denied()); }
        Ok(Self { directory, fortress, region })
    }
}

struct Budget<'a> {
    context: &'a OperationContext,
    guard: &'a dyn ExcavationSessionGuard,
    deadline: Instant,
    bytes: u64,
}
impl<'a> Budget<'a> {
    fn new(context: &'a OperationContext, guard: &'a dyn ExcavationSessionGuard) -> Result<Self> {
        context.budget.validate()?;
        if context.budget.max_wall_millis > 60000 { return Err(exhausted()); }
        let deadline = Instant::now().checked_add(Duration::from_millis(context.budget.max_wall_millis))
            .ok_or_else(exhausted)?;
        let out = Self { context, guard, deadline, bytes: context.budget.max_bytes };
        out.current()?;
        Ok(out)
    }
    fn current(&self) -> Result<OperationContext> {
        self.guard.checkpoint()?;
        if self.context.cancellation_requested || self.guard.cancellation().is_cancelled() {
            return Err(error(ErrorCode::CancellationRequested, "excavation effect shell cancelled"));
        }
        let time = self.deadline.checked_duration_since(Instant::now())
            .filter(|time| *time >= Duration::from_millis(1)).ok_or_else(exhausted)?;
        let mut current = self.context.clone();
        current.budget.max_wall_millis = u64::try_from(time.as_millis()).map_err(|_| exhausted())?;
        current.budget.max_bytes = self.bytes;
        Ok(current)
    }
    fn charge(&mut self, bytes: u64) -> Result<()> {
        self.current()?;
        self.bytes = self.bytes.checked_sub(bytes).filter(|n| *n > 0).ok_or_else(exhausted)?;
        Ok(())
    }
    fn connect(&mut self, control: bool) -> Result<OperationContext> {
        let current = self.current()?;
        self.charge((if control { 8 } else { 7 }) * RPC_BYTES + 24)?;
        Ok(current)
    }
}
fn nonce(context: &OperationContext) -> [u8; 32] {
    // Session IDs are process-incarnation scoped in the MCP runtime, and requests
    // strictly increase. This nonce is a correlation identity, not a credential.
    let mut value = [0; 32];
    value[..16].copy_from_slice(&context.session_id.get().to_be_bytes());
    value[16..].copy_from_slice(&context.request_id.get().to_be_bytes());
    value
}
fn expected<S: EffectJournalStorage>(owner: &ExcavationCoordinator<S>, view: &ExcavationInventory) -> Result<()> {
    let actual = ExcavationInventory::new(owner.binding().clone(), owner.entries().cloned().collect())?;
    if &actual != view { return Err(stale()); }
    Ok(())
}

/// The coordinator calls these checks after its write/sync boundary and before
/// dispatch. An expired/cancelled runtime cannot spend a durable dispatch marker.
struct Checked<'a> {
    source: ExcavationRpc,
    guard: &'a dyn ExcavationSessionGuard,
}
impl ExcavationRunSource for Checked<'_> {
    fn binding(&self) -> &ExcavationBinding { self.source.binding() }
    fn fence(&mut self) { self.source.fence(); }
    fn observe(&mut self, region: ExcavationRegion, c: &OperationContext, t: Duration) -> Result<ExcavationCapture> {
        self.guard.checkpoint()?;
        self.source.observe(region, c, t)
    }
    fn prepare(&mut self, plan: &ExcavationRunPlan, c: &OperationContext, t: Duration) -> Result<ExcavationRunRecord> {
        self.guard.checkpoint()?;
        self.guard.allow_start()?;
        self.source.prepare(plan, c, t)
    }
    fn commit(&mut self, permit: ExcavationDispatch<'_>, c: &OperationContext, t: Duration) -> Result<ExcavationRunRecord> {
        self.guard.checkpoint()?;
        self.guard.allow_start()?;
        self.source.commit(permit, c, t)
    }
    fn query(&mut self, plan: &ExcavationRunPlan, c: &OperationContext, t: Duration) -> Result<Option<ExcavationRunRecord>> {
        self.guard.checkpoint()?;
        self.source.query(plan, c, t)
    }
    fn cancel(&mut self, plan: &ExcavationRunPlan, c: &OperationContext, t: Duration) -> Result<ExcavationRunRecord> {
        self.guard.checkpoint()?;
        // Cancellation still requires the caller's Clock grant. Revocation of
        // the separate unpause opt-in must not prevent an authorized safety stop.
        self.source.cancel(plan, c, t)
    }
}
impl ExcavationSessionBackend for PrivateExcavationBackend {
    fn fortress(&self) -> &FortressIdentity { &self.fortress }
    fn region(&self) -> ExcavationRegion { self.region }
    fn inspect(&mut self, c: &OperationContext, guard: &dyn ExcavationSessionGuard) -> Result<ExcavationInventory> {
        let work = Budget::new(c, guard)?;
        query(c, &self.fortress, 0)?;
        let archive = inspect_private_excavation(&self.directory, &self.fortress, &work.current()?)?;
        let view = ExcavationInventory::new(archive.binding().clone(), archive.entries().to_vec())?;
        query(&work.current()?, &self.fortress, view.high_tick())?;
        Ok(view)
    }
    fn initialize(&mut self, c: &OperationContext, guard: &dyn ExcavationSessionGuard) -> Result<ExcavationObservation> {
        let mut work = Budget::new(c, guard)?;
        guard.allow_start()?;
        c.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        c.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
        let source = ExcavationRpc::connect_control(self.fortress.clone(), self.region,
            nonce(c), &work.connect(true)?, guard.cancellation())?;
        let capture = source.initial_capture().ok_or_else(stale)?.clone();
        let binding = source.binding().clone();
        drop(source); // No second underlying read and no native preparation.
        guard.allow_start()?;
        let current = query(&work.current()?, &self.fortress, capture.tick())?;
        current.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        // EXCLUSIVE creation only: existing files, including empty or corrupt
        // files, are never initialized, repaired or substituted.
        let _owner = create_private_excavation(&self.directory, binding.clone(), &current)?;
        work.current()?;
        Ok(ExcavationObservation { binding, capture })
    }
    fn observe(&mut self, view: &ExcavationInventory, c: &OperationContext,
        guard: &dyn ExcavationSessionGuard) -> Result<ExcavationObservation>
    {
        let mut work = Budget::new(c, guard)?;
        query(c, &self.fortress, view.high_tick())?;
        c.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        let source = ExcavationRpc::connect_control(self.fortress.clone(), self.region,
            nonce(c), &work.connect(true)?, guard.cancellation())?;
        let capture = source.initial_capture().ok_or_else(stale)?.clone();
        let binding = source.binding().clone();
        drop(source);
        if &binding != view.binding() { return Err(stale()); }
        query(&work.current()?, &self.fortress, capture.tick())?
            .authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        Ok(ExcavationObservation { binding, capture })
    }
    fn start(&mut self, view: &ExcavationInventory, plan: ExcavationRunPlan,
        c: &OperationContext, guard: &dyn ExcavationSessionGuard) -> Result<ExcavationRunRecord>
    {
        let mut work = Budget::new(c, guard)?;
        guard.allow_start()?;
        if plan.before().region() != self.region { return Err(denied()); }
        let mut owner = open_private_excavation(&self.directory, &self.fortress, &work.current()?)?;
        work.charge(VIEW_BYTES)?;
        expected(&owner, view)?; // Under the SAME lock retained through commit.
        super::super::coordinator::authorize_start(&work.current()?, owner.binding(), &plan)?;
        guard.allow_start()?;
        let source = ExcavationRpc::connect_control(self.fortress.clone(), self.region,
            nonce(c), &work.connect(true)?, guard.cancellation())?;
        let mut checked = Checked { source, guard };
        let digest = plan.digest();
        let result = owner.start(&mut checked, plan, digest, &work.current()?);
        checked.fence();
        result
    }
    fn recover(&mut self, view: &ExcavationInventory, key: &str, cancel: bool,
        c: &OperationContext, guard: &dyn ExcavationSessionGuard) -> Result<Option<ExcavationRunRecord>>
    {
        let mut work = Budget::new(c, guard)?;
        query(c, &self.fortress, view.high_tick())?;
        if cancel { c.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?; }
        let mut owner = open_private_excavation(&self.directory, &self.fortress, &work.current()?)?;
        work.charge(VIEW_BYTES)?;
        expected(&owner, view)?;
        let entry = owner.entry(key).ok_or_else(stale)?;
        if entry.native().is_some_and(ExcavationRunRecord::terminal) {
            return Ok(entry.native().cloned());
        }
        let region = entry.plan().before().region();
        let source = ExcavationRpc::connect_recovery(owner.binding(), region,
            nonce(c), &work.connect(false)?, guard.cancellation())?;
        let mut checked = Checked { source, guard };
        let result = owner.recover(&mut checked, key, cancel, &work.current()?);
        checked.fence();
        result
    }
}
