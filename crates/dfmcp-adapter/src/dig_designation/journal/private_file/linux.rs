use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::{
    ffi::OsStrExt,
    fs::{MetadataExt, OpenOptionsExt},
    io::AsRawFd,
};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::{DigBinding, DigJournal, DigMode, error};
use crate::control_effect_journal::EffectJournalStorage;
use dfmcp_core::{Capability, ErrorCode, OperationContext, Result, RiskTier};

const NOFOLLOW: i32 = 0x20000;
const NONBLOCK: i32 = 0x800;
const DIRECTORY: i32 = 0x10000;
const APPEND: i32 = 0x400;

fn denied() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "dig journal private custody changed",
    )
}
fn failure(_: io::Error) -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::CorruptLedger,
        "dig journal private custody or I/O failed; no repair performed",
    )
}
fn remaining(context: &OperationContext, deadline: Instant) -> Result<OperationContext> {
    let left = deadline
        .checked_duration_since(Instant::now())
        .ok_or_else(|| {
            error(
                ErrorCode::BudgetExceeded,
                "dig storage open deadline exhausted",
            )
        })?;
    let millis = left.as_millis() as u64;
    if millis == 0 {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "dig storage open deadline exhausted",
        ));
    }
    let mut current = context.clone();
    current.budget.max_wall_millis = millis.min(context.budget.max_wall_millis);
    Ok(current)
}
fn descriptor_leaf(directory: &File, name: &std::ffi::OsStr) -> PathBuf {
    PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd())).join(name)
}
fn directory_ok(metadata: &Metadata, uid: u32) -> bool {
    metadata.is_dir()
        && metadata.mode() & 0o7777 == 0o700
        && (metadata.uid() == 0 || metadata.uid() == uid)
}
fn file_ok(metadata: &Metadata, owner: u32) -> bool {
    metadata.is_file()
        && metadata.mode() & 0o7777 == 0o600
        && metadata.nlink() == 1
        && metadata.uid() == owner
}

/// Safe Rust uses an already opened directory via Linux procfd for relative
/// no-follow opens. The live pathname, pinned descriptors and extent are checked
/// at every storage boundary. This does not defend a hostile same-UID process.
pub struct PrivateDigFile {
    file: File,
    directory: File,
    path: PathBuf,
    read_only: bool,
    extent: u64,
    file_identity: (u64, u64, u32),
    directory_identity: (u64, u64, u32),
}
impl PrivateDigFile {
    fn writable(&self) -> io::Result<()> {
        if self.read_only {
            return Err(denied());
        }
        self.validate_identity()
    }
    fn synchronize(&self, mut sync: impl FnMut(&File) -> io::Result<()>) -> io::Result<()> {
        self.writable()?;
        sync(&self.file)?;
        sync(&self.directory)?;
        self.validate_identity()
    }
}
impl Read for PrivateDigFile {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.validate_identity()?;
        let n = self.file.read(out)?;
        self.validate_identity()?;
        Ok(n)
    }
}
impl Seek for PrivateDigFile {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.validate_identity()?;
        self.file.seek(position)
    }
}
impl Write for PrivateDigFile {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writable()?;
        if self.file.stream_position()? != self.extent
            || bytes.len() as u64 > super::super::MAX_JOURNAL_BYTES as u64 - self.extent
        {
            return Err(denied());
        }
        let n = self.file.write(bytes)?;
        self.extent += n as u64;
        self.validate_identity()?;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.writable()?;
        self.file.flush()
    }
}
impl EffectJournalStorage for PrivateDigFile {
    fn sync(&mut self) -> io::Result<()> {
        self.synchronize(File::sync_all)
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(denied())
    }
    fn validate_identity(&self) -> io::Result<()> {
        let parent = self.path.parent().ok_or_else(denied)?;
        if parent.canonicalize()? != parent {
            return Err(denied());
        }
        let dir = self.directory.metadata()?;
        let named_dir = fs::symlink_metadata(parent)?;
        let named = fs::symlink_metadata(descriptor_leaf(
            &self.directory,
            self.path.file_name().ok_or_else(denied)?,
        ))?;
        let opened = self.file.metadata()?;
        if !directory_ok(&dir, self.directory_identity.2)
            || !directory_ok(&named_dir, self.directory_identity.2)
            || (dir.dev(), dir.ino(), dir.uid()) != self.directory_identity
            || (named_dir.dev(), named_dir.ino(), named_dir.uid()) != self.directory_identity
            || !file_ok(&opened, dir.uid())
            || !file_ok(&named, dir.uid())
            || (opened.dev(), opened.ino(), opened.uid()) != self.file_identity
            || (named.dev(), named.ino(), named.uid()) != self.file_identity
            || opened.len() != self.extent
            || named.len() != self.extent
            || self.extent > super::super::MAX_JOURNAL_BYTES as u64
        {
            return Err(denied());
        }
        Ok(())
    }
}

pub(super) fn open(
    path: &Path,
    context: &OperationContext,
    mode: DigMode,
    expected: Option<DigBinding>,
) -> Result<DigJournal<PrivateDigFile>> {
    let deadline = Instant::now()
        .checked_add(Duration::from_millis(context.budget.max_wall_millis))
        .ok_or_else(|| error(ErrorCode::BudgetExceeded, "dig storage deadline overflow"))?;
    // Fail before opening any path when the caller lacks a current Query grant.
    // Unbound offline open cannot know the persisted scope yet; test candidate
    // scopes without broadening them, then the journal authorizes the real scope.
    if let Some(binding) = &expected {
        binding.authorize(context, context.anchor.tick.get())?;
    } else if !context.grants.iter().any(|grant| {
        matches!(grant.capability, Capability::Query | Capability::Admin)
            && !grant.is_limited()
            && grant.allows(
                Capability::Query,
                RiskTier::ReadOnly,
                context.anchor.tick,
                context.anchor.fortress_id,
                &[],
                grant.scope.map_area,
            )
    }) {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "dig storage open requires current Query authority",
        ));
    }
    let raw = path.as_os_str().as_bytes();
    if !path.is_absolute()
        || raw.len() < 2
        || raw.len() > 4096
        || raw[1..]
            .split(|b| *b == b'/')
            .any(|part| part.is_empty() || part == b"." || part == b".." || part.contains(&0))
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "dig journal path must be normalized absolute",
        ));
    }
    let parent = path.parent().ok_or_else(|| failure(denied()))?;
    let mut directory = OpenOptions::new()
        .read(true)
        .custom_flags(NOFOLLOW | DIRECTORY)
        .open("/")
        .map_err(failure)?;
    for component in parent.iter().skip(1) {
        remaining(context, deadline)?;
        let child = descriptor_leaf(&directory, component);
        directory = OpenOptions::new()
            .read(true)
            .custom_flags(NOFOLLOW | DIRECTORY)
            .open(child)
            .map_err(failure)?;
    }
    let dir = directory.metadata().map_err(failure)?;
    let uid = fs::metadata("/proc/self").map_err(failure)?.uid();
    if !directory_ok(&dir, uid) {
        return Err(failure(denied()));
    }
    let leaf = descriptor_leaf(
        &directory,
        path.file_name().ok_or_else(|| failure(denied()))?,
    );
    let before = match fs::symlink_metadata(&leaf) {
        Ok(metadata) => {
            if !file_ok(&metadata, dir.uid()) {
                return Err(failure(denied()));
            }
            Some(metadata)
        }
        Err(cause) if cause.kind() == io::ErrorKind::NotFound => None,
        Err(cause) => return Err(failure(cause)),
    };
    let created = before.is_none();
    if created && (mode != DigMode::Control || expected.is_none()) {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "dig recovery cannot create a journal",
        ));
    }
    remaining(context, deadline)?;
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(mode != DigMode::Offline)
        .custom_flags(NOFOLLOW | NONBLOCK | if mode == DigMode::Offline { 0 } else { APPEND });
    if created {
        options.create_new(true).mode(0o600);
    }
    let file = options.open(&leaf).map_err(failure)?;
    file.try_lock().map_err(|_| {
        error(
            ErrorCode::Conflict,
            "dig journal already owned or exclusive locking unavailable",
        )
    })?;
    let opened = file.metadata().map_err(failure)?;
    if before
        .as_ref()
        .is_some_and(|m| (m.dev(), m.ino()) != (opened.dev(), opened.ino()))
    {
        return Err(failure(denied()));
    }
    let storage = PrivateDigFile {
        file,
        directory,
        path: path.to_owned(),
        read_only: mode == DigMode::Offline,
        extent: opened.len(),
        file_identity: (opened.dev(), opened.ino(), opened.uid()),
        directory_identity: (dir.dev(), dir.ino(), dir.uid()),
    };
    storage.validate_identity().map_err(failure)?;
    let nonce = if created {
        let mut nonce = [0; 32];
        File::open("/dev/urandom")
            .and_then(|mut source| source.read_exact(&mut nonce))
            .map_err(failure)?;
        Some(nonce)
    } else {
        None
    };
    // Header publication (including the directory entry) is synchronized by the
    // same storage.sync used for every append. Empty existing files are refused.
    DigJournal::open(
        storage,
        &remaining(context, deadline)?,
        mode,
        expected,
        nonce,
    )
}

#[cfg(test)]
mod tests;
