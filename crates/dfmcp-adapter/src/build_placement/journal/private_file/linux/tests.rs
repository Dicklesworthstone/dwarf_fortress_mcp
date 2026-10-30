use super::*;
use std::os::unix::fs::DirBuilderExt;
use std::sync::atomic::{AtomicU64, Ordering};

#[test]
fn dropped_owner_releases_lock_even_when_duplicate_descriptor_survives() -> Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Directory(PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let base = std::env::temp_dir().canonicalize().map_err(failure)?;
    let path = base.join(format!(
        "dfmcp-build-duplicate-lock-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .map_err(failure)?;
    let dir = Directory(path);
    let journal = dir.0.join("placement.journal");
    let context = crate::build_placement::journal::tests::context()?;
    let (owner, created, _) = open_storage(&journal, &context, BuildMode::Control, true, 4096)?;
    assert!(created);
    let inherited = owner.file.try_clone().map_err(failure)?;
    assert!(open_storage(&journal, &context, BuildMode::Offline, false, 4096).is_err());
    drop(owner);
    let (reopened, _, _) = open_storage(&journal, &context, BuildMode::Offline, false, 4096)?;
    drop(reopened);
    drop(inherited);
    Ok(())
}
