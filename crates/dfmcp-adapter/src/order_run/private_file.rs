//! Operator-selected private Linux custody; never accept a path in a tool request.
use super::FortressIdentity;
use super::journal::{OrderRunBinding, OrderRunJournal, OrderRunMode};
use super::rpc::authorize;
use crate::bounded_run::error;
use crate::control_effect_journal::EffectJournalStorage;
use dfmcp_core::{ErrorCode, OperationContext, Result};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

pub struct PrivateOrderRunFile {
    file: File,
    path: PathBuf,
    read_only: bool,
    extent: u64,
    #[cfg(unix)]
    identity: (u64, u64, u32, u64, u64),
}
fn denied() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "order-run journal custody changed",
    )
}
fn failed(_: io::Error) -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::CorruptLedger,
        "private conditional-run storage failed",
    )
}
impl PrivateOrderRunFile {
    fn writable(&self) -> io::Result<()> {
        if self.read_only {
            return Err(denied());
        }
        self.validate_identity()
    }
}
impl Read for PrivateOrderRunFile {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.file.read(out)
    }
}
impl Seek for PrivateOrderRunFile {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.file.seek(pos)
    }
}
impl Write for PrivateOrderRunFile {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.writable()?;
        if self.file.stream_position()? != self.extent {
            return Err(denied());
        }
        let n = self.file.write(data)?;
        self.extent += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.writable()?;
        self.file.flush()
    }
}
impl EffectJournalStorage for PrivateOrderRunFile {
    fn sync(&mut self) -> io::Result<()> {
        self.writable()?;
        self.file.sync_all()
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(denied())
    }
    fn validate_identity(&self) -> io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let parent = self.path.parent().ok_or_else(denied)?;
            if parent.canonicalize()? != parent {
                return Err(denied());
            }
            let dir = fs::symlink_metadata(parent)?;
            let named = fs::symlink_metadata(&self.path)?;
            let opened = self.file.metadata()?;
            if !dir.is_dir()
                || dir.mode() & 0o7777 != 0o700
                || !named.is_file()
                || named.mode() & 0o7777 != 0o600
                || named.nlink() != 1
                || named.uid() != dir.uid()
                || opened.mode() & 0o7777 != 0o600
                || opened.nlink() != 1
                || (
                    opened.dev(),
                    opened.ino(),
                    opened.uid(),
                    dir.dev(),
                    dir.ino(),
                ) != self.identity
                || (named.dev(), named.ino()) != (opened.dev(), opened.ino())
                || opened.len() != self.extent
            {
                return Err(denied());
            }
            Ok(())
        }
        #[cfg(not(unix))]
        {
            Err(denied())
        }
    }
}
/// Linux flag values are intentionally gated. Other platforms refuse custody.
pub fn open_private_order_journal(
    path: &Path,
    fortress: &FortressIdentity,
    c: &OperationContext,
    mode: OrderRunMode,
    binding: Option<OrderRunBinding>,
) -> Result<OrderRunJournal<PrivateOrderRunFile>> {
    authorize(c, fortress, mode == OrderRunMode::Control)?;
    #[cfg(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        const NOFOLLOW: i32 = 0x20000;
        const NONBLOCK: i32 = 0x800;
        const DIRECTORY: i32 = 0x10000;
        let refuse = || {
            error(
                ErrorCode::CapabilityDenied,
                "order-run journal requires normalized absolute path, owner-private 0700 parent and single-link 0600 file",
            )
        };
        if !path.is_absolute()
            || path.as_os_str().len() > 4096
            || path.file_name().is_none()
            || path
                .components()
                .any(|p| !matches!(p, Component::RootDir | Component::Normal(_)))
        {
            return Err(refuse());
        }
        let parent = path.parent().ok_or_else(refuse)?;
        if parent.canonicalize().map_err(failed)? != parent {
            return Err(refuse());
        }
        let dir = fs::symlink_metadata(parent).map_err(failed)?;
        let uid = fs::metadata("/proc/self").map_err(failed)?.uid();
        if !dir.is_dir() || dir.mode() & 0o7777 != 0o700 || (dir.uid() != 0 && dir.uid() != uid) {
            return Err(refuse());
        }
        let before = match fs::symlink_metadata(path) {
            Ok(m) => {
                if !m.is_file()
                    || m.mode() & 0o7777 != 0o600
                    || m.nlink() != 1
                    || m.uid() != dir.uid()
                {
                    return Err(refuse());
                }
                Some(m)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(failed(e)),
        };
        let created = before.is_none();
        if created && (mode != OrderRunMode::Control || binding.is_none()) {
            return Err(error(
                ErrorCode::InvalidRequest,
                "recovery requires existing conditional-run journal",
            ));
        }
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(mode != OrderRunMode::Offline)
            .custom_flags(NOFOLLOW | NONBLOCK);
        if created {
            options.create_new(true).mode(0o600);
        }
        let file = options.open(path).map_err(failed)?;
        file.try_lock().map_err(|_| {
            error(
                ErrorCode::Conflict,
                "conditional-run journal is already owned",
            )
        })?;
        let opened = file.metadata().map_err(failed)?;
        if before
            .as_ref()
            .is_some_and(|m| (m.dev(), m.ino()) != (opened.dev(), opened.ino()))
        {
            return Err(refuse());
        }
        let storage = PrivateOrderRunFile {
            file,
            path: path.to_owned(),
            read_only: mode == OrderRunMode::Offline,
            extent: opened.len(),
            identity: (
                opened.dev(),
                opened.ino(),
                opened.uid(),
                dir.dev(),
                dir.ino(),
            ),
        };
        storage.validate_identity().map_err(failed)?;
        if created {
            let directory = OpenOptions::new()
                .read(true)
                .custom_flags(NOFOLLOW | DIRECTORY)
                .open(parent)
                .map_err(failed)?;
            let actual = directory.metadata().map_err(failed)?;
            if (actual.dev(), actual.ino()) != (dir.dev(), dir.ino()) {
                return Err(refuse());
            }
            directory.sync_all().map_err(failed)?;
        }
        OrderRunJournal::open(storage, c, mode, fortress, binding, created)
    }
    #[cfg(not(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )))]
    {
        let _ = (path, binding);
        Err(error(
            ErrorCode::CapabilityDenied,
            "conditional-run journal custody supports Linux x86_64/aarch64 only",
        ))
    }
}
