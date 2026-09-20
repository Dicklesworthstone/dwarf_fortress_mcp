//! Operator-only Linux journal custody. No MCP argument is used as a file path.
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use dfmcp_core::{ErrorCode, OperationContext, Result};
use crate::control_effect_journal::EffectJournalStorage;
use super::error;
use super::journal::{RunBinding, RunJournal, RunMode};
use super::rpc::authorize;

pub struct PrivateRunFile {
    file:File, path:PathBuf, read_only:bool, extent:u64,
    #[cfg(unix)]
    identity:(u64,u64,u32,u64,u64),
}
fn io_denied() -> io::Error {io::Error::new(io::ErrorKind::PermissionDenied,"bounded-run journal custody changed")}
fn storage_error(_:io::Error) -> dfmcp_core::DfmcpError {error(ErrorCode::CorruptLedger,"bounded-run journal I/O or custody failed")}
impl PrivateRunFile {
    fn writable(&self) -> io::Result<()> {
        if self.read_only {return Err(io_denied());} self.validate_identity()
    }
}
impl Read for PrivateRunFile {fn read(&mut self,out:&mut [u8]) -> io::Result<usize> {self.file.read(out)}}
impl Seek for PrivateRunFile {fn seek(&mut self,pos:SeekFrom) -> io::Result<u64> {self.file.seek(pos)}}
impl Write for PrivateRunFile {
    fn write(&mut self,bytes:&[u8]) -> io::Result<usize> {
        self.writable()?;
        if self.file.stream_position()?!=self.extent {return Err(io_denied());}
        let n=self.file.write(bytes)?; self.extent+=n as u64; Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {self.writable()?;self.file.flush()}
}
impl EffectJournalStorage for PrivateRunFile {
    fn sync(&mut self) -> io::Result<()> {self.writable()?;self.file.sync_all()}
    fn truncate(&mut self,_:u64) -> io::Result<()> {Err(io_denied())}
    fn validate_identity(&self) -> io::Result<()> {
        #[cfg(unix)] {
            use std::os::unix::fs::MetadataExt;
            let parent=self.path.parent().ok_or_else(io_denied)?;
            if parent.canonicalize()?!=parent {return Err(io_denied());}
            let dir=fs::symlink_metadata(parent)?; let named=fs::symlink_metadata(&self.path)?; let opened=self.file.metadata()?;
            if !dir.is_dir() || dir.mode()&0o7777!=0o700 || !named.is_file() || named.mode()&0o7777!=0o600
                || named.nlink()!=1 || named.uid()!=dir.uid()
                || (opened.dev(),opened.ino(),opened.uid(),dir.dev(),dir.ino())!=self.identity
                || (named.dev(),named.ino())!=(opened.dev(),opened.ino()) || opened.len()!=self.extent
            {return Err(io_denied());} Ok(())
        }
        #[cfg(not(unix))] {Err(io_denied())}
    }
}

/// Supports Linux x86_64/aarch64 only. Explicit no-follow/nonblocking flags reject
/// final-component links and avoid a FIFO substitution hanging before validation.
/// Other targets fail closed rather than assuming platform flag equivalence.
pub fn open_private_run_journal(path:&Path,context:&OperationContext,mode:RunMode,
    binding:Option<RunBinding>) -> Result<RunJournal<PrivateRunFile>>
{
    authorize(context,mode==RunMode::Control)?;
    #[cfg(all(target_os="linux",any(target_arch="x86_64",target_arch="aarch64")))] {
        use std::os::unix::fs::{MetadataExt,OpenOptionsExt};
        const NOFOLLOW:i32=0x20000;
        const NONBLOCK:i32=0x800;
        const DIRECTORY:i32=0x10000;
        let denied=||error(ErrorCode::CapabilityDenied,"run journal requires absolute normalized path, private 0700 parent and single-link 0600 regular file");
        if !path.is_absolute() || path.as_os_str().len()>4096 || path.file_name().is_none()
            || path.components().any(|c|!matches!(c,Component::RootDir|Component::Normal(_)))
        {return Err(denied());}
        let parent=path.parent().ok_or_else(denied)?;
        if parent.canonicalize().map_err(storage_error)?!=parent {return Err(denied());}
        let dir=fs::symlink_metadata(parent).map_err(storage_error)?;
        // The kernel's /proc/self ownership gives a conservative owner check;
        // non-dumpable processes may be refused rather than widening custody.
        let uid=fs::metadata("/proc/self").map_err(storage_error)?.uid();
        if !dir.is_dir() || dir.mode()&0o7777!=0o700 || (dir.uid()!=0 && dir.uid()!=uid) {return Err(denied());}
        let before=match fs::symlink_metadata(path) {
            Ok(m)=>{
                if !m.is_file() || m.mode()&0o7777!=0o600 || m.nlink()!=1 || m.uid()!=dir.uid() {return Err(denied());}
                Some(m)
            }
            Err(e) if e.kind()==io::ErrorKind::NotFound=>None,
            Err(e)=>return Err(storage_error(e)),
        };
        let created=before.is_none();
        if created && mode!=RunMode::Control {return Err(error(ErrorCode::InvalidRequest,"run recovery requires an existing journal"));}
        let mut options=OpenOptions::new(); options.read(true).write(mode!=RunMode::Offline).custom_flags(NOFOLLOW|NONBLOCK);
        if created {options.create_new(true).mode(0o600);}
        let file=options.open(path).map_err(storage_error)?;
        file.try_lock().map_err(|_|error(ErrorCode::Conflict,"run journal already owned or file locking unavailable"))?;
        let opened=file.metadata().map_err(storage_error)?;
        if before.as_ref().is_some_and(|m|(m.dev(),m.ino())!=(opened.dev(),opened.ino())) {return Err(denied());}
        let storage=PrivateRunFile {file,path:path.to_owned(),read_only:mode==RunMode::Offline,extent:opened.len(),
            identity:(opened.dev(),opened.ino(),opened.uid(),dir.dev(),dir.ino())};
        storage.validate_identity().map_err(storage_error)?;
        if created {
            // Publication of the filename is durable before any native prepare.
            let directory=OpenOptions::new().read(true).custom_flags(NOFOLLOW|DIRECTORY).open(parent).map_err(storage_error)?;
            let actual=directory.metadata().map_err(storage_error)?;
            if (actual.dev(),actual.ino())!=(dir.dev(),dir.ino()) {return Err(denied());}
            directory.sync_all().map_err(storage_error)?;
        }
        RunJournal::open(storage,context,mode,binding,created)
    }
    #[cfg(not(all(target_os="linux",any(target_arch="x86_64",target_arch="aarch64"))))] {
        let _=(path,binding);
        Err(error(ErrorCode::CapabilityDenied,"run journal custody currently supports Linux x86_64/aarch64 only"))
    }
}
