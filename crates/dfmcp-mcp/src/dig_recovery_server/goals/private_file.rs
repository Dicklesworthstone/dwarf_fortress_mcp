//! Read-only snapshots of operator-selected monitor journals. Never create or repair.
use std::path::Path;
use dfmcp_core::Result;
use super::denied;

#[cfg(all(target_os="linux", any(target_arch="x86_64", target_arch="aarch64")))]
mod linux {
    use super::*;
    use super::super::MAX_BYTES;
    use std::fs::{self, File, Metadata, OpenOptions};
    use std::io::{Read, Seek, SeekFrom};
    use std::os::unix::{ffi::OsStrExt, fs::{MetadataExt, OpenOptionsExt}, io::AsRawFd};
    use std::path::PathBuf;

    const NOFOLLOW:i32=0x20000;
    const NONBLOCK:i32=0x800;
    const DIRECTORY:i32=0x10000;
    pub(super) struct ReadOnly {
        file:File, parent:File, path:PathBuf,
        pub raw:Vec<u8>,
        pub identity:(u64,u64,u64,u64),
    }
    fn leaf(parent:&File,name:&std::ffi::OsStr)->PathBuf {
        PathBuf::from(format!("/proc/self/fd/{}",parent.as_raw_fd())).join(name)
    }
    fn stamp(m:&Metadata)->(u64,i64,i64,i64,i64) {
        (m.len(),m.mtime(),m.mtime_nsec(),m.ctime(),m.ctime_nsec())
    }
    impl ReadOnly {
        fn custody(&self)->Result<()> {
            let parent=self.path.parent().ok_or_else(denied)?;
            if parent.canonicalize().map_err(|_|denied())?!=parent{return Err(denied());}
            let p=self.parent.metadata().map_err(|_|denied())?;
            let named_parent=fs::symlink_metadata(parent).map_err(|_|denied())?;
            let f=self.file.metadata().map_err(|_|denied())?;
            let named=fs::symlink_metadata(leaf(&self.parent,self.path.file_name().ok_or_else(denied)?)).map_err(|_|denied())?;
            let uid=fs::metadata("/proc/self").map_err(|_|denied())?.uid();
            for info in [&p,&named_parent] {
                if !info.is_dir()||info.mode()&0o7777!=0o700||(info.uid()!=0&&info.uid()!=uid)
                    ||(info.dev(),info.ino())!=(self.identity.0,self.identity.1){return Err(denied());}
            }
            for info in [&f,&named] {
                if !info.is_file()||info.mode()&0o7777!=0o600||info.nlink()!=1||info.uid()!=p.uid()
                    ||(info.dev(),info.ino())!=(self.identity.2,self.identity.3)
                    ||info.len()>MAX_BYTES as u64{return Err(denied());}
            }
            Ok(())
        }
        fn read(&mut self,check:&mut impl FnMut()->Result<()>)->Result<Vec<u8>> {
            check()?;self.custody()?;
            let before=self.file.metadata().map_err(|_|denied())?;
            if before.len()==0{return Err(denied());}
            self.file.seek(SeekFrom::Start(0)).map_err(|_|denied())?;
            let mut bytes=Vec::with_capacity(before.len() as usize);
            let mut part=[0;32768];
            loop {
                check()?;
                let n=self.file.read(&mut part).map_err(|_|denied())?;
                if n==0{break;}
                if bytes.len()+n>MAX_BYTES{return Err(denied());}
                bytes.extend_from_slice(&part[..n]);
            }
            let after=self.file.metadata().map_err(|_|denied())?;
            if stamp(&before)!=stamp(&after)||bytes.len()!=before.len() as usize{return Err(denied());}
            self.custody()?;check()?;Ok(bytes)
        }
        pub fn verify(&mut self,check:&mut impl FnMut()->Result<()>)->Result<()> {
            if self.read(check)?!=self.raw{return Err(denied());}Ok(())
        }
        pub fn open(path:&Path,check:&mut impl FnMut()->Result<()>)->Result<Self> {
            check()?;
            let raw=path.as_os_str().as_bytes();
            if !path.is_absolute()||raw.len()<2||raw.len()>4096||raw[1..].split(|b|*b==b'/')
                .any(|s|s.is_empty()||s==b"."||s==b".."||s.contains(&0)){return Err(denied());}
            let parent_path=path.parent().ok_or_else(denied)?;
            let mut parent=OpenOptions::new().read(true).custom_flags(NOFOLLOW|DIRECTORY).open("/").map_err(|_|denied())?;
            for component in parent_path.iter().skip(1) {
                check()?;
                parent=OpenOptions::new().read(true).custom_flags(NOFOLLOW|DIRECTORY)
                    .open(leaf(&parent,component)).map_err(|_|denied())?;
            }
            let file=OpenOptions::new().read(true).custom_flags(NOFOLLOW|NONBLOCK)
                .open(leaf(&parent,path.file_name().ok_or_else(denied)?)).map_err(|_|denied())?;
            file.try_lock().map_err(|_|denied())?;
            let p=parent.metadata().map_err(|_|denied())?;let f=file.metadata().map_err(|_|denied())?;
            let mut snapshot=Self{file,parent,path:path.to_owned(),raw:Vec::new(),identity:(p.dev(),p.ino(),f.dev(),f.ino())};
            snapshot.raw=snapshot.read(check)?;Ok(snapshot)
        }
    }
}

pub(super) struct Snapshot {
    #[cfg(all(target_os="linux", any(target_arch="x86_64", target_arch="aarch64")))]
    inner:linux::ReadOnly,
}
impl Snapshot {
    pub fn open(path:&Path,check:&mut impl FnMut()->Result<()>)->Result<Self> {
        #[cfg(all(target_os="linux", any(target_arch="x86_64", target_arch="aarch64")))]
        {Ok(Self{inner:linux::ReadOnly::open(path,check)?})}
        #[cfg(not(all(target_os="linux", any(target_arch="x86_64", target_arch="aarch64"))))]
        {let _=(path,check);Err(denied())}
    }
    pub fn bytes(&self)->&[u8] {
        #[cfg(all(target_os="linux", any(target_arch="x86_64", target_arch="aarch64")))]
        {&self.inner.raw}
        #[cfg(not(all(target_os="linux", any(target_arch="x86_64", target_arch="aarch64"))))]
        {&[]}
    }
    pub fn identity(&self)->(u64,u64,u64,u64) {
        #[cfg(all(target_os="linux", any(target_arch="x86_64", target_arch="aarch64")))]
        {self.inner.identity}
        #[cfg(not(all(target_os="linux", any(target_arch="x86_64", target_arch="aarch64"))))]
        {(0,0,0,0)}
    }
    pub fn verify(&mut self,check:&mut impl FnMut()->Result<()>)->Result<()> {
        #[cfg(all(target_os="linux", any(target_arch="x86_64", target_arch="aarch64")))]
        {self.inner.verify(check)}
        #[cfg(not(all(target_os="linux", any(target_arch="x86_64", target_arch="aarch64"))))]
        {let _=check;Err(denied())}
    }
}
