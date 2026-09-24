//! Operator-only private custody. Offline descriptors cannot write or repair.
use super::{ArchiveMode, JournalStorage, ProgressArchive, error, storage_error};
use dfmcp_core::{Capability, ErrorCode, OperationContext, Result, RiskTier};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

pub struct PrivateProgressArchiveFile {
    file: File,
    path: PathBuf,
    offline: bool,
    #[cfg(unix)]
    identity: (u64, u64, u32, u64, u64),
}
impl PrivateProgressArchiveFile {
    fn writable(&self) -> io::Result<()> {
        if self.offline {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "offline progress archive is read-only",
            ))
        } else {
            Ok(())
        }
    }
}
impl Read for PrivateProgressArchiveFile {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.file.read(out)
    }
}
impl Seek for PrivateProgressArchiveFile {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.file.seek(from)
    }
}
impl Write for PrivateProgressArchiveFile {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writable()?;
        self.file.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.writable()?;
        self.file.flush()
    }
}
impl JournalStorage for PrivateProgressArchiveFile {
    fn sync(&mut self) -> io::Result<()> {
        self.writable()?;
        self.validate_identity()?;
        self.file.sync_all()
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "progress history cannot be truncated or repaired",
        ))
    }
    fn validate_identity(&self) -> io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let denied = || {
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "progress archive custody changed",
                )
            };
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
                || (
                    opened.dev(),
                    opened.ino(),
                    opened.uid(),
                    dir.dev(),
                    dir.ino(),
                ) != self.identity
                || (named.dev(), named.ino()) != (opened.dev(), opened.ino())
            {
                return Err(denied());
            }
            Ok(())
        }
        #[cfg(not(unix))]
        {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "private progress archives currently require Unix",
            ))
        }
    }
}

pub fn open_progress_archive(
    path: &Path,
    mode: ArchiveMode,
    c: &OperationContext,
) -> Result<ProgressArchive<PrivateProgressArchiveFile>> {
    c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if mode == ArchiveMode::Live {
        c.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let denied = || {
            error(
                ErrorCode::CapabilityDenied,
                "progress archive requires a normalized absolute path, real private 0700 parent and single-link 0600 regular file",
            )
        };
        if !path.is_absolute()
            || path.as_os_str().len() > 4096
            || path.file_name().is_none()
            || path
                .components()
                .any(|c| !matches!(c, Component::RootDir | Component::Normal(_)))
        {
            return Err(denied());
        }
        let parent = path.parent().ok_or_else(denied)?;
        if parent.canonicalize().map_err(storage_error)? != parent {
            return Err(denied());
        }
        let dir = fs::symlink_metadata(parent).map_err(storage_error)?;
        if !dir.is_dir() || dir.mode() & 0o7777 != 0o700 {
            return Err(denied());
        }
        let before = match fs::symlink_metadata(path) {
            Ok(m) => {
                if !m.is_file()
                    || m.mode() & 0o7777 != 0o600
                    || m.nlink() != 1
                    || m.uid() != dir.uid()
                {
                    return Err(denied());
                }
                Some(m)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(storage_error(e)),
        };
        let created = before.is_none();
        if created && mode == ArchiveMode::Offline {
            return Err(error(
                ErrorCode::InvalidRequest,
                "offline progress recovery requires an existing archive",
            ));
        }
        let mut options = OpenOptions::new();
        options.read(true).write(mode == ArchiveMode::Live);
        if created {
            options.create_new(true).mode(0o600);
        }
        let file = options.open(path).map_err(storage_error)?;
        file.try_lock().map_err(|_| {
            error(
                ErrorCode::Conflict,
                "progress archive already has an owner or cannot be locked",
            )
        })?;
        let opened = file.metadata().map_err(storage_error)?;
        if before
            .as_ref()
            .is_some_and(|m| (m.dev(), m.ino()) != (opened.dev(), opened.ino()))
        {
            return Err(denied());
        }
        let storage = PrivateProgressArchiveFile {
            file,
            path: path.to_owned(),
            offline: mode == ArchiveMode::Offline,
            identity: (
                opened.dev(),
                opened.ino(),
                opened.uid(),
                dir.dev(),
                dir.ino(),
            ),
        };
        storage.validate_identity().map_err(storage_error)?;
        if created {
            File::open(parent)
                .and_then(|f| f.sync_all())
                .map_err(storage_error)?;
        }
        ProgressArchive::open(storage, mode, created, c)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(error(
            ErrorCode::CapabilityDenied,
            "private progress archives currently require Unix custody",
        ))
    }
}
