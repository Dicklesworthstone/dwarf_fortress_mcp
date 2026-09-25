//! Operator-owned Linux journal custody for the concrete excavation coordinator.
//!
//! One dedicated directory, one fixed journal name, no repair or replacement.
//! Paths are runtime configuration, never native payloads or MCP tool arguments.
use super::coordinator::{ExcavationBinding, ExcavationCoordinator, ExcavationEntry};
use super::{FortressIdentity, Result, error};
use dfmcp_core::{Capability, ErrorCode, OperationContext, RiskTier};
use std::path::Path;
use std::time::{Duration, Instant};

pub const JOURNAL_NAME: &str = "excavation-run-v1_18.journal";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode { Create, Recover, Inspect }

#[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")))]
mod linux;
#[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")))]
pub use linux::PrivateExcavationFile;
#[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")))]
use linux as platform;

#[cfg(not(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64"))))]
mod unsupported {
    use crate::control_effect_journal::EffectJournalStorage;
    use std::io::{self, Read, Seek, SeekFrom, Write};
    pub struct PrivateExcavationFile;
    pub(super) fn open(_: &std::path::Path, _: &dfmcp_core::OperationContext, _: super::Mode)
        -> super::Result<PrivateExcavationFile>
    {
        Err(super::error(dfmcp_core::ErrorCode::CapabilityDenied,
            "private excavation storage requires Linux x86_64/aarch64"))
    }
    fn refusal() -> io::Error { io::Error::new(io::ErrorKind::Unsupported, "private excavation journals require Linux x86_64/aarch64") }
    impl Read for PrivateExcavationFile {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> { Err(refusal()) }
    }
    impl Write for PrivateExcavationFile {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> { Err(refusal()) }
        fn flush(&mut self) -> io::Result<()> { Err(refusal()) }
    }
    impl Seek for PrivateExcavationFile {
        fn seek(&mut self, _: SeekFrom) -> io::Result<u64> { Err(refusal()) }
    }
    impl EffectJournalStorage for PrivateExcavationFile {
        fn sync(&mut self) -> io::Result<()> { Err(refusal()) }
        fn truncate(&mut self, _: u64) -> io::Result<()> { Err(refusal()) }
        fn validate_identity(&self) -> io::Result<()> { Err(refusal()) }
    }
}
#[cfg(not(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64"))))]
pub use unsupported::PrivateExcavationFile;
#[cfg(not(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64"))))]
use unsupported as platform;

fn authorize(context: &OperationContext, fortress: &FortressIdentity, mode: Mode) -> Result<()> {
    if context.anchor.fortress_id != fortress.fortress_id() {
        return Err(error(ErrorCode::CapabilityDenied, "excavation storage belongs to another fortress"));
    }
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if mode == Mode::Create {
        context.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
    }
    if context.budget.max_wall_millis > 60000 {
        return Err(error(ErrorCode::BudgetExceeded, "excavation storage deadline exceeds profile bound"));
    }
    Ok(())
}
fn open_storage(directory: &Path, fortress: &FortressIdentity, context: &OperationContext,
    mode: Mode) -> Result<(PrivateExcavationFile, OperationContext)>
{
    authorize(context, fortress, mode)?;
    let start = Instant::now();
    let storage = platform::open(directory, context, mode)?;
    authorize(context, fortress, mode)?;
    let remaining = Duration::from_millis(context.budget.max_wall_millis).checked_sub(start.elapsed())
        .ok_or_else(|| error(ErrorCode::BudgetExceeded, "excavation storage open exhausted deadline"))?;
    let mut narrowed = context.clone();
    narrowed.budget.max_wall_millis = u64::try_from(remaining.as_millis()).map_err(|_| {
        error(ErrorCode::BudgetExceeded, "excavation storage deadline overflow")
    })?;
    narrowed.budget.validate()?;
    Ok((storage, narrowed))
}

/// Exclusively create the fixed journal in an existing, empty private directory.
/// An old empty file is a conflict, never a new journal. The directory and file
/// remain locked in the returned coordinator until it is dropped.
pub fn create_private_excavation(directory: &Path, binding: ExcavationBinding,
    context: &OperationContext) -> Result<ExcavationCoordinator<PrivateExcavationFile>>
{
    let (storage, narrowed) = open_storage(directory, binding.fortress(), context, Mode::Create)?;
    ExcavationCoordinator::create(storage, binding, &narrowed)
}

/// Open only an existing nonempty journal for query/cancel recovery. This does
/// not read credentials or contact the game. Reopening cannot create dispatch
/// permission. Starting new work still requires the coordinator's current grants.
pub fn open_private_excavation(directory: &Path, fortress: &FortressIdentity,
    context: &OperationContext) -> Result<ExcavationCoordinator<PrivateExcavationFile>>
{
    let (storage, narrowed) = open_storage(directory, fortress, context, Mode::Recover)?;
    ExcavationCoordinator::open(storage, fortress, &narrowed)
}

/// Immutable historical inventory, never a handle to a writable coordinator.
/// Its scope is this one journal at the completed inspection, not all controllers.
#[derive(Clone, Debug)]
pub struct ExcavationArchive {
    binding: ExcavationBinding,
    entries: Vec<ExcavationEntry>,
}
impl ExcavationArchive {
    pub fn binding(&self) -> &ExcavationBinding { &self.binding }
    pub fn entries(&self) -> &[ExcavationEntry] { &self.entries }
    pub fn pending_count(&self) -> usize { self.entries.iter().filter(|e| e.unresolved()).count() }
}

/// Strictly read-only offline inspection: no native source, create, write,
/// flush, sync or truncate. The archive cannot subsequently be used to dispatch.
pub fn inspect_private_excavation(directory: &Path, fortress: &FortressIdentity,
    context: &OperationContext) -> Result<ExcavationArchive>
{
    let (storage, narrowed) = open_storage(directory, fortress, context, Mode::Inspect)?;
    let owner = ExcavationCoordinator::open(storage, fortress, &narrowed)?;
    Ok(ExcavationArchive { binding: owner.binding().clone(), entries: owner.entries().cloned().collect() })
}
