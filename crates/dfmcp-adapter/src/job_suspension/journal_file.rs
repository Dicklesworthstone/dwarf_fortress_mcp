//! Operator-selected private custody. No MCP request may select this path.
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

use super::{
    EffectJournalStorage, JobControlJournal, OperationContext, Result, authorize, conflict, fail,
    io_error,
};
use dfmcp_core::ErrorCode;

pub struct PrivateJobJournalFile {
    file: File,
    path: PathBuf,
    read_only: bool,
    #[cfg(unix)]
    identity: (u64, u64, u32, u64, u64),
}
impl PrivateJobJournalFile {
    fn writable(&self) -> io::Result<()> {
        if self.read_only {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "read-only job recovery file",
            ));
        }
        Ok(())
    }
}
impl Read for PrivateJobJournalFile {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.file.read(out)
    }
}
impl Seek for PrivateJobJournalFile {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.file.seek(from)
    }
}
impl Write for PrivateJobJournalFile {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.writable()?;
        self.file.write(data)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.writable()?;
        self.file.flush()
    }
}
impl EffectJournalStorage for PrivateJobJournalFile {
    fn sync(&mut self) -> io::Result<()> {
        self.writable()?;
        self.validate_identity()?;
        self.file.sync_all()
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "job journals never truncate or erase recovery evidence",
        ))
    }
    fn validate_identity(&self) -> io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let denied = || {
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "job journal custody changed",
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
                "private job journals require Unix custody checks",
            ))
        }
    }
}

pub fn open_private_job_journal(
    path: &Path,
    context: &OperationContext,
) -> Result<JobControlJournal<PrivateJobJournalFile>> {
    let (storage, created) = open_storage(path, context, false, true)?;
    JobControlJournal::open(storage, context, created)
}
pub fn open_private_job_recovery(
    path: &Path,
    context: &OperationContext,
) -> Result<JobControlJournal<PrivateJobJournalFile>> {
    let (storage, _) = open_storage(path, context, true, false)?;
    JobControlJournal::open_read_only(storage, context)
}
/// Query-only recovery can retain new reconciliation evidence in an EXISTING
/// journal. No file creation, native dispatch, grant synthesis or tail repair.
pub fn open_private_job_reconciliation(
    path: &Path,
    context: &OperationContext,
) -> Result<JobControlJournal<PrivateJobJournalFile>> {
    let (storage, _) = open_storage(path, context, false, false)?;
    JobControlJournal::open_for_reconciliation(storage, context)
}
fn open_storage(
    path: &Path,
    context: &OperationContext,
    read_only: bool,
    allow_create: bool,
) -> Result<(PrivateJobJournalFile, bool)> {
    authorize(context, context.anchor.fortress_id, allow_create)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let denied = || {
            fail(
                ErrorCode::CapabilityDenied,
                "job journal requires an absolute normalized path, private 0700 parent, and single-link 0600 regular file",
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
        if parent.canonicalize().map_err(io_error)? != parent {
            return Err(denied());
        }
        let dir = fs::symlink_metadata(parent).map_err(io_error)?;
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
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(io_error(e)),
        };
        let created = before.is_none();
        if created && !allow_create {
            return Err(fail(
                ErrorCode::InvalidRequest,
                "job recovery requires an existing journal",
            ));
        }
        let mut options = OpenOptions::new();
        options.read(true).write(!read_only);
        if created {
            options.create_new(true).mode(0o600);
        }
        let file = options.open(path).map_err(io_error)?;
        file.try_lock().map_err(|_| {
            conflict("job journal already has an owner or cannot be exclusively locked")
        })?;
        let opened = file.metadata().map_err(io_error)?;
        if before
            .as_ref()
            .is_some_and(|m| (m.dev(), m.ino()) != (opened.dev(), opened.ino()))
        {
            return Err(denied());
        }
        let storage = PrivateJobJournalFile {
            file,
            path: path.to_owned(),
            read_only,
            identity: (
                opened.dev(),
                opened.ino(),
                opened.uid(),
                dir.dev(),
                dir.ino(),
            ),
        };
        storage.validate_identity().map_err(io_error)?;
        if created {
            File::open(parent)
                .and_then(|dir| dir.sync_all())
                .map_err(io_error)?;
        }
        Ok((storage, created))
    }
    #[cfg(not(unix))]
    {
        let _ = (path, read_only);
        Err(fail(
            ErrorCode::CapabilityDenied,
            "private job journals are currently Unix-only",
        ))
    }
}
