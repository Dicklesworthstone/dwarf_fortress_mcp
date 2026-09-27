use super::*;
use crate::build_placement::journal::private_file::open_private_build;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Directory(PathBuf);
impl Directory {
    fn new() -> io::Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-furniture-parent-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        let private = path.join("custody");
        fs::create_dir(&private)?;
        fs::set_permissions(&private, fs::Permissions::from_mode(0o700))?;
        Ok(Self(path))
    }
    fn parent(&self) -> PathBuf {
        self.0.join("custody")
    }
    fn file(&self) -> PathBuf {
        self.parent().join("batch.journal")
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn batch_private_lock_is_exclusive_and_offline_replay_never_republishes() -> TestResult {
    let directory = Directory::new()?;
    let path = directory.file();
    let mut owner =
        open_private_batch(&path, &context()?, BuildMode::Control, Some(definition()?))?;
    assert!(open_private_batch(&path, &query_context()?, BuildMode::Offline, None).is_err());
    owner.stop(&query_context()?)?;
    drop(owner);
    let before = fs::read(&path)?;
    let mut offline = open_private_batch(&path, &query_context()?, BuildMode::Offline, None)?;
    offline.verify(&query_context()?)?;
    assert!(offline.durable_stopped());
    assert!(!offline.is_fenced());
    assert!(offline.stop(&query_context()?).is_err());
    drop(offline);
    assert_eq!(fs::read(&path)?, before);
    assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o7777, 0o600);
    Ok(())
}

#[test]
fn raw_storage_reuse_preserves_original_build_header_and_allows_same_parent() -> TestResult {
    let directory = Directory::new()?;
    let build_path = directory.parent().join("original-build.journal");
    let mut build_context = context()?;
    for capability in [Capability::Observe, Capability::Construct] {
        let mut grant = build_context.grants[1].clone();
        grant.capability = capability;
        build_context.grants.push(grant);
    }
    let binding = definition()?.binding().clone();
    let mut build = open_private_build(
        &build_path,
        &build_context,
        BuildMode::Control,
        Some(binding.clone()),
    )?;
    let original_id = build.inventory(&build_context)?.journal_id;
    let parent = BatchDefinition::new(definition()?.plan().clone(), binding.clone(), original_id)?;
    let mut batch = open_private_batch(
        &directory.file(),
        &context()?,
        BuildMode::Control,
        Some(parent),
    )?;
    assert_eq!(batch.definition().journal_id(), original_id);
    batch.verify(&query_context()?)?;
    assert_eq!(build.inventory(&build_context)?.journal_id, original_id);
    let bytes = fs::read(&build_path)?;
    drop(build);
    let mut offline = open_private_build(
        &build_path,
        &query_context()?,
        BuildMode::Offline,
        Some(binding),
    )?;
    assert_eq!(
        offline.inventory(&query_context()?)?.journal_id,
        original_id
    );
    assert_eq!(fs::read(&build_path)?, bytes);
    assert!(open_private_batch(&build_path, &query_context()?, BuildMode::Offline, None).is_err());
    drop(offline);
    // With the lock released, the incompatible unchanged build header remains
    // an error rather than an empty parent available for initialization.
    assert!(
        open_private_batch(
            &build_path,
            &context()?,
            BuildMode::Control,
            Some(definition()?)
        )
        .is_err()
    );
    assert_eq!(fs::read(&build_path)?, bytes);
    Ok(())
}

#[test]
fn missing_empty_symlink_and_modes_are_refused_without_initialization() -> TestResult {
    let directory = Directory::new()?;
    let path = directory.file();
    for mode in [BuildMode::Recover, BuildMode::Offline] {
        assert!(open_private_batch(&path, &context()?, mode, Some(definition()?)).is_err());
        assert!(!path.exists());
    }
    assert!(
        open_private_batch(
            &path,
            &query_context()?,
            BuildMode::Control,
            Some(definition()?)
        )
        .is_err()
    );
    assert!(!path.exists());
    assert!(open_private_batch(&path, &context()?, BuildMode::Control, None).is_err());
    assert!(!path.exists());
    fs::write(&path, [])?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    assert!(
        open_private_batch(&path, &context()?, BuildMode::Control, Some(definition()?)).is_err()
    );
    assert_eq!(fs::metadata(&path)?.len(), 0);
    fs::remove_file(&path)?;
    let target = directory.parent().join("target");
    fs::write(&target, b"preserved")?;
    symlink(&target, &path)?;
    assert!(
        open_private_batch(&path, &context()?, BuildMode::Control, Some(definition()?)).is_err()
    );
    assert_eq!(fs::read(target)?, b"preserved");
    fs::remove_file(&path)?;
    fs::set_permissions(directory.parent(), fs::Permissions::from_mode(0o750))?;
    assert!(
        open_private_batch(&path, &context()?, BuildMode::Control, Some(definition()?)).is_err()
    );
    assert!(!path.exists());
    Ok(())
}

#[test]
fn parent_replacement_and_file_removal_or_replacement_fence_the_open_owner() -> TestResult {
    for fault in 0..4 {
        let directory = Directory::new()?;
        let path = directory.file();
        let mut owner =
            open_private_batch(&path, &context()?, BuildMode::Control, Some(definition()?))?;
        let bytes = fs::read(&path)?;
        match fault {
            0 => {
                fs::remove_file(&path)?;
            }
            1 => {
                fs::rename(&path, directory.parent().join("old"))?;
                fs::write(&path, &bytes)?;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
            }
            2 => {
                fs::hard_link(&path, directory.parent().join("another-name"))?;
            }
            _ => {
                fs::rename(directory.parent(), directory.0.join("old-parent"))?;
                fs::create_dir(directory.parent())?;
                fs::set_permissions(directory.parent(), fs::Permissions::from_mode(0o700))?;
                fs::write(&path, &bytes)?;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
            }
        }
        assert!(
            owner.verify(&query_context()?).is_err(),
            "custody fault {fault}"
        );
        assert!(owner.is_fenced());
        assert!(owner.stopped());
        assert!(!owner.durable_stopped());
        assert!(owner.stop(&query_context()?).is_err());
    }
    Ok(())
}

#[test]
fn parent_symlink_wrong_file_mode_and_oversize_are_rejected() -> TestResult {
    let directory = Directory::new()?;
    let path = directory.file();
    drop(open_private_batch(
        &path,
        &context()?,
        BuildMode::Control,
        Some(definition()?),
    )?);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640))?;
    assert!(open_private_batch(&path, &query_context()?, BuildMode::Offline, None).is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    let alias = directory.0.join("alias");
    symlink(directory.parent(), &alias)?;
    assert!(
        open_private_batch(
            &alias.join("batch.journal"),
            &query_context()?,
            BuildMode::Offline,
            None
        )
        .is_err()
    );
    fs::write(&path, vec![0; MAX_STORE_BYTES + 1])?;
    assert!(open_private_batch(&path, &query_context()?, BuildMode::Offline, None).is_err());
    assert_eq!(fs::metadata(&path)?.len(), (MAX_STORE_BYTES + 1) as u64);
    Ok(())
}
