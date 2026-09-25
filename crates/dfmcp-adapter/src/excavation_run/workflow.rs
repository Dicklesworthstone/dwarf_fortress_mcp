//! Concrete foreground lifecycle: private Rust journal + native Rust TCP source.
//!
//! Runtime-selected directory and nonce only; no Python subprocess or tool-selected
//! path. The caller owns the region, capability grants and cancellation signal.
use super::*;
use super::coordinator::{ExcavationBinding, ExcavationRunSource, authorize_start};
use super::private_file::{open_private_excavation, create_private_excavation};
use super::rpc::{ExcavationCancellation, ExcavationRpc};
use dfmcp_core::OperationContext;
use std::path::Path;
use std::time::{Duration, Instant};

const RPC_RESERVE: u64 = 272 * 1024;
struct Budget<'a> {
    original: &'a OperationContext,
    cancellation: ExcavationCancellation,
    deadline: Instant,
    bytes: u64,
}
impl<'a> Budget<'a> {
    fn new(context: &'a OperationContext, cancellation: ExcavationCancellation) -> Result<Self> {
        context.budget.validate()?;
        require(context.budget.max_wall_millis <= 60000, "excavation workflow deadline exceeds bound")?;
        let deadline = Instant::now().checked_add(Duration::from_millis(context.budget.max_wall_millis))
            .ok_or_else(|| error(ErrorCode::BudgetExceeded, "excavation workflow deadline overflow"))?;
        let out = Self { original: context, cancellation, deadline, bytes: context.budget.max_bytes };
        out.context()?;
        Ok(out)
    }
    fn context(&self) -> Result<OperationContext> {
        if self.original.cancellation_requested || self.cancellation.is_cancelled() {
            return Err(error(ErrorCode::CancellationRequested, "excavation workflow cancelled; retain journal"));
        }
        let left = self.deadline.checked_duration_since(Instant::now())
            .ok_or_else(|| error(ErrorCode::BudgetExceeded, "excavation workflow deadline exhausted"))?;
        let mut current = self.original.clone();
        current.budget.max_wall_millis = u64::try_from(left.as_millis())
            .map_err(|_| error(ErrorCode::BudgetExceeded, "excavation workflow time overflow"))?;
        current.budget.max_bytes = self.bytes;
        current.budget.validate()?;
        Ok(current)
    }
    fn charge(&mut self, amount: u64) -> Result<()> {
        self.context()?;
        self.bytes = self.bytes.checked_sub(amount)
            .ok_or_else(|| error(ErrorCode::BudgetExceeded, "excavation workflow byte allowance exhausted"))?;
        Ok(())
    }
    fn reserve_connect(&self, control: bool) -> Result<()> {
        let reserve = (if control { 8 } else { 7 }) * RPC_RESERVE + 24;
        // Leave at least one subsequent native exchange. The coordinator checks
        // its complete start/recovery storage reservation before mutating anything.
        if self.bytes < reserve + RPC_RESERVE {
            return Err(error(ErrorCode::BudgetExceeded, "excavation workflow cannot afford connection and operation"));
        }
        Ok(())
    }
}

/// Exclusively initialize a NEW dedicated store, without native contact. The
/// binding should come from a control observation. It is evidence, not authority;
/// every later start must establish that binding again over its own connection.
pub fn initialize(directory: &Path, binding: ExcavationBinding, context: &OperationContext) -> Result<()> {
    let _owner = create_private_excavation(directory, binding, context)?;
    Ok(())
}

pub struct ConfirmedExcavationStart {
    pub plan: ExcavationRunPlan,
    pub confirmed_digest: Digest32,
    /// A fresh nonce from the supervising runtime, not a stored receipt.
    pub nonce: [u8; 32],
}

/// Existing store only. Validate the journal and pending-work fence BEFORE
/// opening a socket, then use one connection and the actual durable coordinator.
pub fn start(directory: &Path, request: ConfirmedExcavationStart, context: &OperationContext,
    cancellation: ExcavationCancellation) -> Result<ExcavationRunRecord>
{
    let mut budget = Budget::new(context, cancellation.clone())?;
    let mut owner = open_private_excavation(directory, request.plan.before().fortress(), &budget.context()?)?;
    budget.charge(owner.storage_bytes() as u64)?;
    let current = budget.context()?;
    authorize_start(&current, owner.binding(), &request.plan)?;
    if request.confirmed_digest != request.plan.digest() || owner.pending_count() != 0
        || owner.entry(request.plan.key()).is_some()
    { return Err(error(ErrorCode::Conflict, "unconfirmed, reused or unresolved excavation work")); }
    budget.reserve_connect(true)?;
    let mut source = ExcavationRpc::connect_control(request.plan.before().fortress().clone(),
        request.plan.before().region(), request.nonce, &budget.context()?, cancellation)?;
    budget.charge(8 * RPC_RESERVE + 24)?;
    let result = owner.start(&mut source, request.plan, request.confirmed_digest, &budget.context()?);
    source.fence();
    result
}

#[derive(Clone, Copy)]
pub enum RecoveryAction { Query, Cancel }
pub struct ExcavationRecovery<'a> {
    pub fortress: &'a FortressIdentity,
    pub key: &'a str,
    pub action: RecoveryAction,
    pub nonce: [u8; 32],
}

/// One explicit recovery step; never prepare, commit, repair or replay unpause.
/// Existing terminal history returns without environment reads or native contact.
pub fn recover(directory: &Path, request: ExcavationRecovery<'_>, context: &OperationContext,
    cancellation: ExcavationCancellation) -> Result<Option<ExcavationRunRecord>>
{
    validate_key(request.key)?;
    let mut budget = Budget::new(context, cancellation.clone())?;
    let mut owner = open_private_excavation(directory, request.fortress, &budget.context()?)?;
    budget.charge(owner.storage_bytes() as u64)?;
    let entry = owner.entry(request.key).ok_or_else(|| error(ErrorCode::InvalidRequest, "excavation key not in this journal"))?;
    if entry.native().is_some_and(ExcavationRunRecord::terminal) {
        budget.context()?;
        return Ok(entry.native().cloned());
    }
    let region = entry.plan().before().region();
    budget.reserve_connect(false)?;
    let mut source = ExcavationRpc::connect_recovery(owner.binding(), region, request.nonce, &budget.context()?, cancellation)?;
    budget.charge(7 * RPC_RESERVE + 24)?;
    let result = owner.recover(&mut source, request.key, matches!(request.action, RecoveryAction::Cancel), &budget.context()?);
    source.fence();
    result
}
