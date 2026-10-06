//! Operator-owned Linux backing for the furniture journal. No client-selected paths.
//!
//! The opened regular file remains exclusively locked through replay and every
//! operation. A directory descriptor pins opens; publication syncs BOTH file and
//! parent. Read-only recovery cannot create, write, flush, sync or truncate.
use std::path::{Path, PathBuf};

use super::{BuildJournal, BuildMode, error};
use crate::build_placement::BuildBinding;
use dfmcp_core::{ErrorCode, OperationContext, Result};

/// Read-only identity of an already held private file and its parent. Persisting
/// this value lets a later owner reject a copied or substituted original journal.
/// The value is local custody evidence, not a signature or distributed fence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrivateFileIdentity {
    pub path: PathBuf,
    pub file_device: u64,
    pub file_inode: u64,
    pub file_owner: u32,
    pub directory_device: u64,
    pub directory_inode: u64,
    pub directory_owner: u32,
}
impl PrivateFileIdentity {
    /// Validate the bounded canonical representation without accessing a path.
    /// Actual source custody still requires the held file's private_identity.
    pub fn validate(&self) -> Result<()> {
        let path = self.path.to_str().ok_or_else(|| {
            error(
                ErrorCode::InvalidRequest,
                "private journal identity path must be UTF-8",
            )
        })?;
        if !self.path.is_absolute()
            || path.len() < 2
            || path.len() > 4096
            || path.as_bytes().contains(&0)
            || path[1..]
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
            || self.file_inode == 0
            || self.directory_inode == 0
            || self.file_owner != self.directory_owner
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "invalid private journal identity",
            ));
        }
        Ok(())
    }
}

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

#[cfg(not(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
)))]
impl PrivateBuildFile {
    pub fn private_identity(&self, context: &OperationContext) -> Result<PrivateFileIdentity> {
        context.authorize(
            dfmcp_core::Capability::Query,
            dfmcp_core::RiskTier::ReadOnly,
            &[],
            None,
        )?;
        Err(error(
            ErrorCode::CapabilityDenied,
            "private journal identities require Linux x86_64/aarch64",
        ))
    }
}

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
    validate_open(context)?;
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

fn validate_open(context: &OperationContext) -> Result<()> {
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
    Ok(())
}

/// Reuse the same descriptor-pinned custody for another bounded furniture
/// journal. The caller authenticates its own header and publication; this only
/// creates an empty file when explicitly allowed in Control mode. The returned
/// context retains the remaining foreground deadline after opening custody.
pub(crate) fn open_private_storage(
    path: &Path,
    context: &OperationContext,
    mode: BuildMode,
    allow_create: bool,
    maximum_bytes: usize,
) -> Result<(PrivateBuildFile, bool, OperationContext)> {
    validate_open(context)?;
    #[cfg(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))]
    {
        linux::open_storage(path, context, mode, allow_create, maximum_bytes)
    }
    #[cfg(not(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )))]
    {
        let _ = (path, mode, allow_create, maximum_bytes);
        Err(error(
            ErrorCode::CapabilityDenied,
            "furniture private journals require Linux x86_64/aarch64",
        ))
    }
}
