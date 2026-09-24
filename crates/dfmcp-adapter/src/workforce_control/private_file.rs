//! Operator-owned Linux storage. Directory descriptors pin every open; no paths
//! accepted from tool arguments, no symlink following, overwrite or tail repair.
use super::{
    error,
    journal::{WorkforceBinding, WorkforceJournal, WorkforceMode},
    rpc::authorize,
};
use crate::control_effect_journal::EffectJournalStorage;
use dfmcp_core::{ErrorCode, OperationContext, Result};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub struct PrivateWorkforceFile {
    file: File,
    directory: File,
    path: PathBuf,
    read_only: bool,
    extent: u64,
    #[cfg(unix)]
    identity: (u64, u64, u32, u64, u64),
}
fn denied() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "workforce journal custody changed",
    )
}
fn failure(_: io::Error) -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::CorruptLedger,
        "workforce journal private custody or I/O failed",
    )
}
impl PrivateWorkforceFile {
    fn writable(&self) -> io::Result<()> {
        if self.read_only {
            return Err(denied());
        }
        self.validate_identity()
    }
}
impl Read for PrivateWorkforceFile {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.file.read(out)
    }
}
impl Seek for PrivateWorkforceFile {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.file.seek(pos)
    }
}
impl Write for PrivateWorkforceFile {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writable()?;
        if self.file.stream_position()? != self.extent {
            return Err(denied());
        }
        let n = self.file.write(bytes)?;
        self.extent += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.writable()?;
        self.file.flush()
    }
}
impl EffectJournalStorage for PrivateWorkforceFile {
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
            use std::os::unix::{fs::MetadataExt, io::AsRawFd};
            let parent = self.path.parent().ok_or_else(denied)?;
            if parent.canonicalize()? != parent {
                return Err(denied());
            }
            let dir = self.directory.metadata()?;
            let named_dir = fs::symlink_metadata(parent)?;
            let local = PathBuf::from(format!("/proc/self/fd/{}", self.directory.as_raw_fd()))
                .join(self.path.file_name().ok_or_else(denied)?);
            let named = fs::symlink_metadata(local)?;
            let opened = self.file.metadata()?;
            if !dir.is_dir()
                || dir.mode() & 0o7777 != 0o700
                || !named_dir.is_dir()
                || (dir.dev(), dir.ino()) != (named_dir.dev(), named_dir.ino())
                || !named.is_file()
                || !opened.is_file()
                || named.mode() & 0o7777 != 0o600
                || opened.mode() & 0o7777 != 0o600
                || opened.nlink() != 1
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
            Err(denied())
        }
    }
}
pub fn open_private_workforce(
    path: &Path,
    context: &OperationContext,
    mode: WorkforceMode,
    expected: Option<WorkforceBinding>,
) -> Result<WorkforceJournal<PrivateWorkforceFile>> {
    authorize(
        context,
        context.anchor.fortress_id,
        context.anchor.tick.get(),
        mode == WorkforceMode::Control,
    )?;
    #[cfg(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))]
    {
        use std::os::unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, OpenOptionsExt},
            io::AsRawFd,
        };
        const NOFOLLOW: i32 = 0x20000;
        const NONBLOCK: i32 = 0x800;
        const DIRECTORY: i32 = 0x10000;
        let raw = path.as_os_str().as_bytes();
        if !path.is_absolute()
            || raw.len() > 4096
            || raw.len() < 2
            || raw[1..]
                .split(|b| *b == b'/')
                .any(|part| part.is_empty() || part == b"." || part == b"..")
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "workforce journal path must be normalized absolute",
            ));
        }
        let parent = path.parent().ok_or_else(|| failure(denied()))?;
        let mut directory = OpenOptions::new()
            .read(true)
            .custom_flags(NOFOLLOW | DIRECTORY)
            .open("/")
            .map_err(failure)?;
        for component in parent.iter().skip(1) {
            let child =
                PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd())).join(component);
            directory = OpenOptions::new()
                .read(true)
                .custom_flags(NOFOLLOW | DIRECTORY)
                .open(child)
                .map_err(failure)?;
        }
        let dir = directory.metadata().map_err(failure)?;
        let uid = fs::metadata("/proc/self").map_err(failure)?.uid();
        if dir.mode() & 0o7777 != 0o700 || (dir.uid() != 0 && dir.uid() != uid) {
            return Err(failure(denied()));
        }
        let leaf = PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()))
            .join(path.file_name().ok_or_else(|| failure(denied()))?);
        let before = match fs::symlink_metadata(&leaf) {
            Ok(m) => {
                if !m.is_file()
                    || m.mode() & 0o7777 != 0o600
                    || m.nlink() != 1
                    || m.uid() != dir.uid()
                {
                    return Err(failure(denied()));
                }
                Some(m)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(failure(e)),
        };
        let created = before.is_none();
        if created && (mode != WorkforceMode::Control || expected.is_none()) {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "workforce recovery cannot create a journal",
            ));
        }
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(mode != WorkforceMode::Offline)
            .custom_flags(NOFOLLOW | NONBLOCK);
        if created {
            options.create_new(true).mode(0o600);
        }
        let file = options.open(&leaf).map_err(failure)?;
        file.try_lock().map_err(|_| {
            error(
                ErrorCode::Conflict,
                "workforce journal already owned or lock unavailable",
            )
        })?;
        let opened = file.metadata().map_err(failure)?;
        if before
            .as_ref()
            .is_some_and(|m| (m.dev(), m.ino()) != (opened.dev(), opened.ino()))
        {
            return Err(failure(denied()));
        }
        let storage = PrivateWorkforceFile {
            file,
            directory,
            path: path.to_owned(),
            read_only: mode == WorkforceMode::Offline,
            extent: opened.len(),
            identity: (
                opened.dev(),
                opened.ino(),
                opened.uid(),
                dir.dev(),
                dir.ino(),
            ),
        };
        storage.validate_identity().map_err(failure)?;
        let initialize = if created {
            let binding = expected.clone().ok_or_else(|| failure(denied()))?;
            let mut nonce = [0; 32];
            File::open("/dev/urandom")
                .and_then(|mut f| f.read_exact(&mut nonce))
                .map_err(failure)?;
            storage.directory.sync_all().map_err(failure)?;
            Some((binding, nonce))
        } else {
            None
        };
        let mut journal = WorkforceJournal::open(storage, context, mode, initialize)?;
        if let Some(expected) = expected {
            if journal.view(context)?.binding != expected {
                return Err(error(
                    ErrorCode::Conflict,
                    "workforce journal binds another exact source",
                ));
            }
        }
        Ok(journal)
    }
    #[cfg(not(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )))]
    {
        let _ = (path, expected);
        Err(error(
            ErrorCode::CapabilityDenied,
            "workforce private journals require Linux x86_64/aarch64",
        ))
    }
}
