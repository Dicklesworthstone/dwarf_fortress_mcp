//! Linux-only descriptor-relative custody using the trusted /proc/self/fd mount.
//! No unsafe code, raw syscall FFI, descriptor cloning or path execution fallback.
use super::{JOURNAL_NAME, Mode};
use crate::control_effect_journal::EffectJournalStorage;
use crate::excavation_run::{Result, error};
use crate::excavation_run::coordinator::MAX_JOURNAL_BYTES;
use dfmcp_core::{ErrorCode, OperationContext};
use std::cell::Cell;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

const NOFOLLOW: i32 = 0x20000;
const NONBLOCK: i32 = 0x800;
const DIRECTORY: i32 = 0x10000;
const MAX_PATH: usize = 4096;
fn custody() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "private excavation journal custody lost; preserve files")
}
fn storage_error(_: io::Error) -> dfmcp_core::DfmcpError {
    error(ErrorCode::CorruptLedger, "private excavation journal open or custody check failed")
}
fn fd_path(file: &File) -> PathBuf { PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd())) }
fn identity(m: &Metadata) -> (u64, u64, u32) { (m.dev(), m.ino(), m.uid()) }
fn stamp(m: &Metadata) -> (i64, i64, i64, i64) { (m.mtime(), m.mtime_nsec(), m.ctime(), m.ctime_nsec()) }
fn normalized(path: &Path) -> io::Result<()> {
    let collected: PathBuf = path.components().collect();
    if !path.is_absolute() || path.as_os_str().len() > MAX_PATH || path.to_str().is_none()
        || path.as_os_str() != collected.as_os_str() || path.components().count() > 128
        || path.components().any(|p| !matches!(p, Component::RootDir | Component::Normal(_)))
    { return Err(custody()); }
    Ok(())
}
fn walk(path: &Path) -> io::Result<File> {
    normalized(path)?;
    let mut options = OpenOptions::new();
    options.read(true).custom_flags(NOFOLLOW | DIRECTORY | NONBLOCK);
    let mut directory = options.open("/")?;
    for part in path.components() {
        if let Component::Normal(name) = part {
            // The only followed symlink is the kernel's pinned descriptor entry.
            // The actual caller-supplied path component must not be a symlink.
            directory = options.open(fd_path(&directory).join(name))?;
        }
    }
    Ok(directory)
}
fn membership(directory: &File, expected_file: bool) -> io::Result<()> {
    let mut count = 0;
    for entry in fs::read_dir(fd_path(directory))? {
        let entry = entry?;
        if !expected_file || entry.file_name() != JOURNAL_NAME || count != 0 { return Err(custody()); }
        count += 1;
    }
    let expected_count = if expected_file { 1 } else { 0 };
    if count != expected_count { return Err(custody()); }
    Ok(())
}

/// The raw constructor is private. Returned only inside the typed coordinator;
/// fixed-directory custody cannot be bypassed by selecting a second filename.
pub struct PrivateExcavationFile {
    file: File,
    directory: File,
    path: PathBuf,
    file_id: (u64, u64, u32),
    directory_id: (u64, u64, u32),
    extent: u64,
    stamp: (i64, i64, i64, i64),
    read_only: bool,
    fenced: Cell<bool>,
}
impl PrivateExcavationFile {
    fn check(&self) -> io::Result<()> {
        if self.fenced.get() { return Err(custody()); }
        let named_directory = walk(&self.path)?;
        let directory_meta = self.directory.metadata()?;
        let named_meta = named_directory.metadata()?;
        if !directory_meta.is_dir() || directory_meta.mode() & 0o7777 != 0o700
            || directory_meta.nlink() == 0 || identity(&directory_meta) != self.directory_id
            || identity(&named_meta) != self.directory_id || named_meta.mode() & 0o7777 != 0o700
        { return Err(custody()); }
        membership(&self.directory, true)?;
        let named = fs::symlink_metadata(fd_path(&self.directory).join(JOURNAL_NAME))?;
        let opened = self.file.metadata()?;
        if !named.is_file() || !opened.is_file() || named.mode() & 0o7777 != 0o600
            || opened.mode() & 0o7777 != 0o600 || named.nlink() != 1 || opened.nlink() != 1
            || identity(&named) != self.file_id || identity(&opened) != self.file_id
            || opened.len() != self.extent || named.len() != self.extent
            || stamp(&opened) != self.stamp || stamp(&named) != self.stamp
        { return Err(custody()); }
        Ok(())
    }
    fn checked(&self) -> io::Result<()> {
        let result = self.check();
        if result.is_err() { self.fenced.set(true); }
        result
    }
    fn writable(&self) -> io::Result<()> {
        if self.read_only { return Err(custody()); }
        self.checked()
    }
    fn finish<T>(&self, result: io::Result<T>) -> io::Result<T> {
        if result.is_err() { self.fenced.set(true); }
        result
    }
}
impl Read for PrivateExcavationFile {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.checked()?;
        let result = self.file.read(out);
        let n = self.finish(result)?;
        self.checked()?;
        Ok(n)
    }
}
impl Seek for PrivateExcavationFile {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.checked()?;
        let result = self.file.seek(from);
        self.finish(result)
    }
}
impl Write for PrivateExcavationFile {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writable()?;
        let result = (|| {
            if self.file.stream_position()? != self.extent
                || bytes.len() as u64 > MAX_JOURNAL_BYTES as u64 - self.extent
            { return Err(custody()); }
            let n = self.file.write(bytes)?;
            if n == 0 && !bytes.is_empty() { return Err(io::Error::from(io::ErrorKind::WriteZero)); }
            self.extent += n as u64;
            let changed = self.file.metadata()?;
            if identity(&changed) != self.file_id || changed.len() != self.extent { return Err(custody()); }
            self.stamp = stamp(&changed);
            self.checked()?;
            Ok(n)
        })();
        self.finish(result)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.writable()?;
        let result = self.file.flush();
        self.finish(result)?;
        self.checked()
    }
}
impl EffectJournalStorage for PrivateExcavationFile {
    fn sync(&mut self) -> io::Result<()> {
        self.writable()?;
        let result = (|| {
            self.file.sync_all()?;
            self.directory.sync_all()?;
            self.checked()
        })();
        self.finish(result)
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> { Err(custody()) }
    fn validate_identity(&self) -> io::Result<()> { self.checked() }
}

pub(super) fn open(path: &Path, context: &OperationContext, mode: Mode) -> Result<PrivateExcavationFile> {
    let deadline = Instant::now().checked_add(Duration::from_millis(context.budget.max_wall_millis))
        .ok_or_else(|| error(ErrorCode::BudgetExceeded, "excavation storage deadline overflow"))?;
    let check = || -> Result<()> {
        if context.cancellation_requested {
            return Err(error(ErrorCode::CancellationRequested, "excavation storage open cancelled"));
        }
        if Instant::now() >= deadline {
            return Err(error(ErrorCode::BudgetExceeded, "excavation storage open deadline exhausted"));
        }
        Ok(())
    };
    check()?;
    let directory = walk(path).map_err(storage_error)?;
    let metadata = directory.metadata().map_err(storage_error)?;
    let uid = fs::metadata("/proc/self").map_err(storage_error)?.uid();
    if !metadata.is_dir() || metadata.mode() & 0o7777 != 0o700
        || metadata.nlink() == 0 || (metadata.uid() != 0 && metadata.uid() != uid)
    { return Err(storage_error(custody())); }
    directory.try_lock().map_err(|_| error(ErrorCode::Conflict, "excavation directory is already owned or locking failed"))?;
    membership(&directory, mode != Mode::Create).map_err(storage_error)?;
    check()?;
    // Rewalk immediately before opening; the locked original remains pinned.
    if identity(&walk(path).map_err(storage_error)?.metadata().map_err(storage_error)?) != identity(&metadata) {
        return Err(storage_error(custody()));
    }
    let named = fd_path(&directory).join(JOURNAL_NAME);
    let mut options = OpenOptions::new();
    options.read(true).write(mode != Mode::Inspect).custom_flags(NOFOLLOW | NONBLOCK);
    if mode == Mode::Create { options.create_new(true).mode(0o600); }
    let file = options.open(&named).map_err(storage_error)?;
    let meta = file.metadata().map_err(storage_error)?;
    if !meta.is_file() || meta.mode() & 0o7777 != 0o600 || meta.nlink() != 1
        || (meta.uid() != 0 && meta.uid() != uid) || meta.len() > MAX_JOURNAL_BYTES as u64
        || (mode == Mode::Create && meta.len() != 0) || (mode != Mode::Create && meta.len() == 0)
    { return Err(storage_error(custody())); }
    file.try_lock().map_err(|_| error(ErrorCode::Conflict, "excavation journal is already owned or locking failed"))?;
    let out = PrivateExcavationFile { file, directory, path: path.to_owned(), file_id: identity(&meta),
        directory_id: identity(&metadata), extent: meta.len(), stamp: stamp(&meta),
        read_only: mode == Mode::Inspect, fenced: Cell::new(false) };
    out.checked().map_err(storage_error)?;
    check()?;
    // No sync here: only coordinator publication syncs; inspection never does.
    Ok(out)
}

#[cfg(test)]
mod tests;
