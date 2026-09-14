//! Operator-configured private local storage. No path is accepted from MCP.
//! Ancestors and the owning account/root are trusted; this is not a hostile-host
//! sandbox. Pre/post-open identity checks reject ordinary replacement and links.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use dfmcp_core::{Capability, DfmcpError, ErrorCode, OperationContext, Result, RiskTier};
use super::{JournalLimits, JournalStorage, OperationsJournal, TailRecovery, storage_error};

pub struct PrivateJournalFile {
    file: File,
    path: PathBuf,
    #[cfg(unix)]
    identity: (u64,u64,u32,u64,u64),
}
impl Read for PrivateJournalFile {
    fn read(&mut self,out:&mut [u8])->io::Result<usize>{self.file.read(out)}
}
impl Write for PrivateJournalFile {
    fn write(&mut self,bytes:&[u8])->io::Result<usize>{self.file.write(bytes)}
    fn flush(&mut self)->io::Result<()>{self.file.flush()}
}
impl Seek for PrivateJournalFile {
    fn seek(&mut self,from:SeekFrom)->io::Result<u64>{self.file.seek(from)}
}
impl JournalStorage for PrivateJournalFile {
    fn sync(&mut self)->io::Result<()>{self.validate_identity()?;self.file.sync_all()}
    fn truncate(&mut self,length:u64)->io::Result<()>{self.validate_identity()?;self.file.set_len(length)}
    fn validate_identity(&self)->io::Result<()> {
        #[cfg(unix)] {
            use std::os::unix::fs::MetadataExt;
            let denied=||io::Error::new(io::ErrorKind::PermissionDenied,"private journal custody changed");
            let parent=self.path.parent().ok_or_else(denied)?;
            if parent.canonicalize()?!=parent{return Err(denied());}
            let dir=fs::symlink_metadata(parent)?;
            let named=fs::symlink_metadata(&self.path)?;let opened=self.file.metadata()?;
            if !dir.is_dir() || dir.mode()&0o7777!=0o700 || !named.is_file()
                || named.mode()&0o7777!=0o600 || named.nlink()!=1 || named.uid()!=dir.uid()
                || (opened.dev(),opened.ino(),opened.uid(),dir.dev(),dir.ino())!=self.identity
                || (named.dev(),named.ino())!=(opened.dev(),opened.ino()) {
                return Err(denied());
            }
            Ok(())
        }
        #[cfg(not(unix))] { Err(io::Error::new(io::ErrorKind::Unsupported,"private journals require Unix custody checks")) }
    }
}

pub fn open_private_journal(path:&Path,context:&OperationContext,limits:JournalLimits,
    recovery:TailRecovery)->Result<OperationsJournal<PrivateJournalFile>> {
    context.authorize(Capability::Observe,RiskTier::ReadOnly,&[],None)?;
    context.authorize(Capability::Query,RiskTier::ReadOnly,&[],None)?;
    limits.validate()?;
    #[cfg(unix)] {
        use std::os::unix::fs::{MetadataExt,OpenOptionsExt};
        let denied=||DfmcpError::new(ErrorCode::CapabilityDenied,
            "journal requires an absolute normalized file path in an existing private 0700 directory and a single-link 0600 regular file");
        if !path.is_absolute() || path.as_os_str().len()>4096
            || path.components().any(|c|!matches!(c,Component::RootDir|Component::Normal(_)))
            || path.file_name().is_none() {return Err(denied());}
        let parent=path.parent().ok_or_else(denied)?;
        if parent.canonicalize().map_err(storage_error)?!=parent{return Err(denied());}
        let dir=fs::symlink_metadata(parent).map_err(storage_error)?;
        if !dir.is_dir() || dir.mode()&0o7777!=0o700 {return Err(denied());}
        let before=match fs::symlink_metadata(path) {
            Ok(meta)=>{
                if !meta.is_file() || meta.mode()&0o7777!=0o600 || meta.nlink()!=1 || meta.uid()!=dir.uid(){return Err(denied());}
                Some(meta)
            }
            Err(e) if e.kind()==io::ErrorKind::NotFound=>None,
            Err(e)=>return Err(storage_error(e)),
        };
        let created=before.is_none();
        let mut options=OpenOptions::new();options.read(true).write(true);
        if created {options.create_new(true).mode(0o600);}
        let file=options.open(path).map_err(storage_error)?;
        file.try_lock().map_err(|_|DfmcpError::new(ErrorCode::Conflict,"operations journal already has a writer or cannot be exclusively locked"))?;
        let opened=file.metadata().map_err(storage_error)?;
        if before.as_ref().is_some_and(|meta|(meta.dev(),meta.ino())!=(opened.dev(),opened.ino())) {return Err(denied());}
        let storage=PrivateJournalFile{identity:(opened.dev(),opened.ino(),opened.uid(),dir.dev(),dir.ino()),file,path:path.to_owned()};
        storage.validate_identity().map_err(storage_error)?;
        // Ensure the newly created directory entry precedes any acknowledged append.
        if created {File::open(parent).and_then(|dir|dir.sync_all()).map_err(storage_error)?;}
        OperationsJournal::open(storage,context,limits,created,recovery)
    }
    #[cfg(not(unix))] {
        let _=(path,limits,recovery);
        Err(DfmcpError::new(ErrorCode::CapabilityDenied,"private operations journals are currently supported on Unix only"))
    }
}
