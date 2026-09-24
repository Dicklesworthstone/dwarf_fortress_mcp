//! Operator-owned Linux backing for the dig journal. No client-selected paths.
//!
//! The opened regular file remains exclusively locked through replay and every
//! operation. A directory descriptor pins opens; publication syncs BOTH file and
//! parent. Read-only recovery cannot create, write, flush, sync or truncate.
use std::path::Path;

use super::{DigBinding, DigJournal, DigMode, error};
use dfmcp_core::{ErrorCode, OperationContext, Result};

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod linux;
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
pub use linux::PrivateDigFile;

#[cfg(not(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
)))]
pub struct PrivateDigFile;

// A refusing implementation preserves the public return type on unsupported
// platforms without pretending that their filesystem custody was implemented.
#[cfg(not(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
)))]
mod unsupported {
    use super::PrivateDigFile;
    use crate::control_effect_journal::EffectJournalStorage;
    use std::io::{self, Read, Seek, SeekFrom, Write};
    fn denied() -> io::Error {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "dig private storage requires Linux x86_64/aarch64",
        )
    }
    impl Read for PrivateDigFile {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(denied())
        }
    }
    impl Write for PrivateDigFile {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(denied())
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(denied())
        }
    }
    impl Seek for PrivateDigFile {
        fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
            Err(denied())
        }
    }
    impl EffectJournalStorage for PrivateDigFile {
        fn sync(&mut self) -> io::Result<()> {
            Err(denied())
        }
        fn truncate(&mut self, _: u64) -> io::Result<()> {
            Err(denied())
        }
        fn validate_identity(&self) -> io::Result<()> {
            Err(denied())
        }
    }
}

/// Create ONLY in Control mode with an exact operator binding, or reopen a
/// private existing journal. No native I/O or capability grant is performed.
/// Recover/Offline missing or empty files are errors, never new journals.
pub fn open_private_dig(
    path: &Path,
    context: &OperationContext,
    mode: DigMode,
    expected: Option<DigBinding>,
) -> Result<DigJournal<PrivateDigFile>> {
    if context.cancellation_requested {
        return Err(error(
            ErrorCode::CancellationRequested,
            "dig storage open cancelled",
        ));
    }
    context.budget.validate()?;
    if context.budget.max_wall_millis > 60_000 {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "dig storage deadline exceeds profile bound",
        ));
    }
    #[cfg(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))]
    {
        linux::open(path, context, mode, expected)
    }
    #[cfg(not(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )))]
    {
        let _ = (path, mode, expected);
        Err(error(
            ErrorCode::CapabilityDenied,
            "dig private journals require Linux x86_64/aarch64",
        ))
    }
}
