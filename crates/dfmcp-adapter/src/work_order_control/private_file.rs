//! Operator-selected private custody, never a path accepted from an MCP request.
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

use super::{EffectJournalStorage, JournalMode, WorkOrderJournal, authorize, error, storage_error};
use dfmcp_core::{ErrorCode, OperationContext, Result};

pub struct PrivateWorkOrderFile {
    file: File,
    path: PathBuf,
    read_only: bool,
    extent: u64,
    #[cfg(unix)]
    identity: (u64, u64, u32, u64, u64),
}
impl PrivateWorkOrderFile {
    fn writable(&self) -> io::Result<()> {
        if self.read_only {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "offline creation recovery is read-only",
            ));
        }
        self.validate_identity()
    }
}
impl Read for PrivateWorkOrderFile {
    fn read(&mut self, data: &mut [u8]) -> io::Result<usize> {
        self.file.read(data)
    }
}
impl Seek for PrivateWorkOrderFile {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.file.seek(position)
    }
}
impl Write for PrivateWorkOrderFile {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.writable()?;
        if self.file.stream_position()? != self.extent {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "creation storage is append-only",
            ));
        }
        let size = self.file.write(data)?;
        self.extent += size as u64;
        Ok(size)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.writable()?;
        self.file.flush()
    }
}
impl EffectJournalStorage for PrivateWorkOrderFile {
    fn sync(&mut self) -> io::Result<()> {
        self.writable()?;
        self.file.sync_all()
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "creation evidence is never truncated",
        ))
    }
    fn validate_identity(&self) -> io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let denied = || {
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "creation journal custody changed",
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
                || opened.len() != self.extent
            {
                return Err(denied());
            }
            Ok(())
        }
        #[cfg(not(unix))]
        {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "private creation journals require Unix",
            ))
        }
    }
}

/// Control can create new storage. Reconcile opens existing writable storage
/// with Query only. Offline opens existing storage with a read-only descriptor.
/// These modes are immutable; later grants never promote a recovery journal.
pub fn open_private_work_orders(
    path: &Path,
    context: &OperationContext,
    mode: JournalMode,
) -> Result<WorkOrderJournal<PrivateWorkOrderFile>> {
    authorize(context, context.anchor.fortress_id, false)?;
    if mode == JournalMode::Control {
        authorize(context, context.anchor.fortress_id, true)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let denied = || {
            error(
                ErrorCode::CapabilityDenied,
                "creation journal requires normalized absolute path, private 0700 parent and single-link 0600 regular file",
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
            Ok(meta) => {
                if !meta.is_file()
                    || meta.mode() & 0o7777 != 0o600
                    || meta.nlink() != 1
                    || meta.uid() != dir.uid()
                {
                    return Err(denied());
                }
                Some(meta)
            }
            Err(cause) if cause.kind() == io::ErrorKind::NotFound => None,
            Err(cause) => return Err(storage_error(cause)),
        };
        let created = before.is_none();
        if created && mode != JournalMode::Control {
            return Err(error(
                ErrorCode::InvalidRequest,
                "creation recovery requires an existing journal",
            ));
        }
        let mut options = OpenOptions::new();
        options.read(true).write(mode != JournalMode::Offline);
        if created {
            options.create_new(true).mode(0o600);
        }
        let file = options.open(path).map_err(storage_error)?;
        file.try_lock().map_err(|_| {
            error(
                ErrorCode::Conflict,
                "creation journal already owned or lock unavailable",
            )
        })?;
        let opened = file.metadata().map_err(storage_error)?;
        if before
            .as_ref()
            .is_some_and(|meta| (meta.dev(), meta.ino()) != (opened.dev(), opened.ino()))
        {
            return Err(denied());
        }
        let storage = PrivateWorkOrderFile {
            file,
            path: path.to_owned(),
            read_only: mode == JournalMode::Offline,
            extent: opened.len(),
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
                .and_then(|dir| dir.sync_all())
                .map_err(storage_error)?;
        }
        WorkOrderJournal::open(storage, context, mode, created)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(error(
            ErrorCode::CapabilityDenied,
            "private work-order journals are currently Unix-only",
        ))
    }
}
