//! Operator-owned Linux backing for the furniture journal. No client-selected paths.
//!
//! The opened regular file remains exclusively locked through replay and every
//! operation. A directory descriptor pins opens; publication syncs BOTH file and
//! parent. Read-only recovery cannot create, write, flush, sync or truncate.
use std::path::Path;

use super::{BuildJournal, BuildMode, error};
use crate::build_placement::BuildBinding;
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
pub use linux::PrivateBuildFile;

#[cfg(not(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
)))]
pub struct PrivateBuildFile;

// A refusing implementation preserves the public return type on unsupported
// platforms without pretending that their filesystem custody was implemented.
#[cfg(not(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
)))]
mod unsupported {
    use super::PrivateBuildFile;
    use crate::control_effect_journal::EffectJournalStorage;
    use std::io::{self, Read, Seek, SeekFrom, Write};
    fn denied() -> io::Error {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "furniture private storage requires Linux x86_64/aarch64",
        )
    }
    impl Read for PrivateBuildFile {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(denied())
        }
    }
    impl Write for PrivateBuildFile {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(denied())
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(denied())
        }
    }
    impl Seek for PrivateBuildFile {
        fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
            Err(denied())
        }
    }
    impl EffectJournalStorage for PrivateBuildFile {
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
pub fn open_private_build(
    path: &Path,
    context: &OperationContext,
    mode: BuildMode,
    expected: Option<BuildBinding>,
) -> Result<BuildJournal<PrivateBuildFile>> {
    if context.cancellation_requested {
        return Err(error(
            ErrorCode::CancellationRequested,
            "furniture storage open cancelled",
        ));
    }
    context.budget.validate()?;
    if context.budget.max_wall_millis > 60_000 {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "furniture storage deadline exceeds profile bound",
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
            "furniture private journals require Linux x86_64/aarch64",
        ))
    }
}
