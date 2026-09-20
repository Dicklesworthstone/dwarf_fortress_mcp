//! A distinct operator-owned file: watch intent is not inserted into old archives.
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use dfmcp_core::{ErrorCode, OperationContext, Result};
use super::{ArchiveMode, JournalStorage, ProgressArchive, WatchBook, authorize, error, storage_error};

pub struct PrivateWatchFile {
    file: File, path: PathBuf, offline: bool,
    #[cfg(unix)]
    identity: (u64, u64, u32, u64, u64),
}
impl PrivateWatchFile {
    fn writable(&self) -> io::Result<()> {
        if self.offline { Err(io::Error::new(io::ErrorKind::PermissionDenied, "offline watch book is read-only")) } else { Ok(()) }
    }
}
impl Read for PrivateWatchFile { fn read(&mut self, out: &mut [u8]) -> io::Result<usize> { self.file.read(out) } }
impl Seek for PrivateWatchFile { fn seek(&mut self, from: SeekFrom) -> io::Result<u64> { self.file.seek(from) } }
impl Write for PrivateWatchFile {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> { self.writable()?; self.file.write(bytes) }
    fn flush(&mut self) -> io::Result<()> { self.writable()?; self.file.flush() }
}
impl JournalStorage for PrivateWatchFile {
    fn sync(&mut self) -> io::Result<()> { self.writable()?; self.validate_identity()?; self.file.sync_all() }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "watch intent cannot be truncated or repaired"))
    }
    fn validate_identity(&self) -> io::Result<()> {
        #[cfg(unix)] {
            use std::os::unix::fs::MetadataExt;
            let denied = || io::Error::new(io::ErrorKind::PermissionDenied, "watch book custody changed");
            let parent = self.path.parent().ok_or_else(denied)?;
            if parent.canonicalize()? != parent { return Err(denied()); }
            let dir = fs::symlink_metadata(parent)?;
            let named = fs::symlink_metadata(&self.path)?; let opened = self.file.metadata()?;
            if !dir.is_dir() || dir.mode() & 0o7777 != 0o700 || !named.is_file() || named.mode() & 0o7777 != 0o600
                || named.nlink() != 1 || named.uid() != dir.uid()
                || (opened.dev(), opened.ino(), opened.uid(), dir.dev(), dir.ino()) != self.identity
                || (named.dev(), named.ino()) != (opened.dev(), opened.ino()) { return Err(denied()); }
            Ok(())
        }
        #[cfg(not(unix))] { Err(io::Error::new(io::ErrorKind::Unsupported, "private watch files require Unix custody")) }
    }
}

pub fn open_watch_book<A: JournalStorage>(path: &Path, mode: ArchiveMode,
    archive: &mut ProgressArchive<A>, c: &OperationContext) -> Result<WatchBook<PrivateWatchFile>>
{
    let start = std::time::Instant::now();
    let summary = archive.summary(c)?;
    authorize(mode, &summary, c, mode == ArchiveMode::Live)?;
    #[cfg(unix)] {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let denied = || error(ErrorCode::CapabilityDenied,
            "watch book requires a normalized absolute path, real private 0700 parent and single-link 0600 regular file");
        if !path.is_absolute() || path.as_os_str().len() > 4096 || path.file_name().is_none()
            || path.components().any(|c| !matches!(c, Component::RootDir | Component::Normal(_))) { return Err(denied()); }
        let parent = path.parent().ok_or_else(denied)?;
        if parent.canonicalize().map_err(storage_error)? != parent { return Err(denied()); }
        let dir = fs::symlink_metadata(parent).map_err(storage_error)?;
        if !dir.is_dir() || dir.mode() & 0o7777 != 0o700 { return Err(denied()); }
        let before = match fs::symlink_metadata(path) {
            Ok(m) => {
                if !m.is_file() || m.mode() & 0o7777 != 0o600 || m.nlink() != 1 || m.uid() != dir.uid() { return Err(denied()); }
                Some(m)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(storage_error(e)),
        };
        let created = before.is_none();
        if created && mode == ArchiveMode::Offline {
            return Err(error(ErrorCode::InvalidRequest, "offline recovery requires an existing watch book"));
        }
        let mut options = OpenOptions::new(); options.read(true).write(mode == ArchiveMode::Live);
        if created { options.create_new(true).mode(0o600); }
        let file = options.open(path).map_err(storage_error)?;
        file.try_lock().map_err(|_| error(ErrorCode::Conflict, "watch book already has an owner or cannot be locked"))?;
        let opened = file.metadata().map_err(storage_error)?;
        if before.as_ref().is_some_and(|m| (m.dev(), m.ino()) != (opened.dev(), opened.ino())) { return Err(denied()); }
        let storage = PrivateWatchFile { file, path: path.to_owned(), offline: mode == ArchiveMode::Offline,
            identity: (opened.dev(), opened.ino(), opened.uid(), dir.dev(), dir.ino()) };
        storage.validate_identity().map_err(storage_error)?;
        if created { File::open(parent).and_then(|f| f.sync_all()).map_err(storage_error)?; }
        let mut remaining = c.clone();
        let elapsed = u64::try_from(start.elapsed().as_millis()).map_err(|_| super::exhausted())?;
        remaining.budget.max_wall_millis = c.budget.max_wall_millis.checked_sub(elapsed)
            .filter(|n| *n > 0).ok_or_else(super::exhausted)?;
        WatchBook::open(storage, mode, created, archive, &remaining)
    }
    #[cfg(not(unix))] {
        let _ = (path, start);
        Err(error(ErrorCode::CapabilityDenied, "private watch books currently require Unix custody"))
    }
}
