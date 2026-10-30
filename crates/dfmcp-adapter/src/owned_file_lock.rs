//! Release a successfully acquired local lock when its owner leaves scope.
//!
//! Acquisition remains at each authorized call site. This wrapper neither
//! opens nor locks a file. Releasing is cleanup of an already-owned local
//! resource; a failed unlock does not prove release or storage durability.

use std::fs::File;
use std::ops::{Deref, DerefMut};

pub(crate) struct OwnedFileLock(File);

impl OwnedFileLock {
    /// Transfer a descriptor whose exclusive lock has already been acquired.
    pub(crate) fn from_locked(file: File) -> Self {
        Self(file)
    }
}
impl Deref for OwnedFileLock {
    type Target = File;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl DerefMut for OwnedFileLock {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl Drop for OwnedFileLock {
    fn drop(&mut self) {
        // A duplicate of the same open-file description may outlive this owner
        // across fork/exec. Closing alone would retain its lock until that copy
        // closes. Unlock only the resource already owned by this guard.
        let _ = self.0.unlock();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TempFile(std::path::PathBuf);
    impl TempFile {
        fn new() -> io::Result<Self> {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "dfmcp-owned-lock-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            File::options().write(true).create_new(true).open(&path)?;
            Ok(Self(path))
        }
        fn open(&self) -> io::Result<File> {
            File::options().read(true).write(true).open(&self.0)
        }
    }
    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn duplicate_survives_owner_but_cannot_release_a_fresh_owner() -> io::Result<()> {
        let path = TempFile::new()?;
        let file = path.open()?;
        file.try_lock()?;
        let owner = OwnedFileLock::from_locked(file);
        let inherited = owner.try_clone()?;
        let contender = path.open()?;
        assert!(contender.try_lock().is_err());
        drop(owner);
        contender.try_lock()?;
        let fresh = OwnedFileLock::from_locked(contender);
        drop(inherited);
        let second = path.open()?;
        assert!(second.try_lock().is_err());
        drop(fresh);
        second.try_lock()?;
        Ok(())
    }

    #[test]
    fn early_return_releases_an_acquired_lock_with_a_surviving_duplicate() -> io::Result<()> {
        fn fail_after_acquire(file: File, path: &TempFile) -> io::Result<()> {
            file.try_lock()?;
            let _owner = OwnedFileLock::from_locked(file);
            assert!(path.open()?.try_lock().is_err());
            Err(io::Error::other("post-acquisition validation failed"))
        }
        let path = TempFile::new()?;
        let file = path.open()?;
        let inherited = file.try_clone()?;
        assert!(fail_after_acquire(file, &path).is_err());
        let second = path.open()?;
        second.try_lock()?;
        drop(inherited);
        Ok(())
    }

    #[test]
    fn dropping_an_unlocked_read_only_descriptor_does_not_unlock_an_owner() -> io::Result<()> {
        let path = TempFile::new()?;
        let owner = path.open()?;
        owner.try_lock()?;
        let owner = OwnedFileLock::from_locked(owner);
        let unlocked = File::open(&path.0)?;
        drop(unlocked);
        assert!(path.open()?.try_lock().is_err());
        drop(owner);
        path.open()?.try_lock()?;
        Ok(())
    }
}
