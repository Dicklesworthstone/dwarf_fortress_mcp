use super::*;
use crate::dig_designation::journal::private_file::open_private_dig_exact;
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
            "dfmcp-mining-blueprint-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        let parent = path.join("custody");
        fs::create_dir(&parent)?;
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o700))?;
        Ok(Self(path))
    }
    fn parent(&self) -> PathBuf {
        self.0.join("custody")
    }
    fn file(&self) -> PathBuf {
        self.parent().join("blueprint.journal")
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn private_create(path: &Path) -> Result<PrivateBlueprintStore> {
    open_private_blueprint_store(
        path,
        &context()?,
        DigMode::Control,
        Some(definition()?),
        true,
        None,
    )
}
fn private_reopen(path: &Path, mode: DigMode, pin: Recovery) -> Result<PrivateBlueprintStore> {
    open_private_blueprint_store(path, &query_context()?, mode, None, false, Some(pin))
}

#[test]
fn private_parent_lock_is_exclusive_and_offline_replay_preserves_stopped_bytes() -> TestResult {
    let directory = Directory::new()?;
    let path = directory.file();
    let mut owner = private_create(&path)?;
    let initial = owner.recovery();
    assert!(private_reopen(&path, DigMode::Offline, initial).is_err());
    owner.stop(&query_context()?)?;
    let stopped = owner.recovery();
    drop(owner);
    let before = fs::read(&path)?;
    let mut offline = private_reopen(&path, DigMode::Offline, initial)?;
    offline.verify(&query_context()?)?;
    assert!(offline.durable_stopped());
    assert_eq!(offline.recovery(), stopped);
    assert!(offline.stop(&query_context()?).is_err());
    drop(offline);
    assert_eq!(fs::read(&path)?, before);
    assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o7777, 0o600);
    Ok(())
}

#[test]
fn lazy_storage_never_recreates_recovery_or_overwrites_existing_initialization() -> TestResult {
    let directory = Directory::new()?;
    let path = directory.file();
    let pin = initial_recovery(&definition()?)?;
    for mode in [DigMode::Control, DigMode::Recover, DigMode::Offline] {
        assert!(private_reopen(&path, mode, pin).is_err());
        assert!(!path.exists());
    }
    assert!(
        open_private_blueprint_storage(
            &path,
            &query_context()?,
            DigMode::Control,
            true,
            binding(5000)?.scope()
        )
        .is_err()
    );
    assert!(!path.exists());
    let (raw, c) = open_private_blueprint_storage(
        &path,
        &context()?,
        DigMode::Control,
        true,
        binding(5000)?.scope(),
    )?;
    assert_eq!(fs::metadata(&path)?.len(), 0);
    drop(raw);
    assert!(private_create(&path).is_err());
    assert_eq!(fs::metadata(&path)?.len(), 0);
    assert!(private_reopen(&path, DigMode::Control, pin).is_err());
    assert_eq!(fs::metadata(&path)?.len(), 0);
    fs::remove_file(&path)?;
    let (raw, current) =
        open_private_blueprint_storage(&path, &c, DigMode::Control, true, binding(5000)?.scope())?;
    let mut parent = BatchStore::open(
        raw,
        &current,
        DigMode::Control,
        Some(definition()?),
        true,
        None,
    )?;
    parent.verify(&context()?)?;
    assert_eq!(parent.recovery(), pin);
    Ok(())
}

#[test]
fn blueprint_and_native_journal_share_private_parent_without_changing_native_bytes() -> TestResult {
    let directory = Directory::new()?;
    let native_path = directory.parent().join("dig.journal");
    let mut c = context()?;
    c.budget.max_bytes = 1024 * 1024 * 1024;
    for capability in [Capability::Observe, Capability::Designate] {
        let mut grant = c.grants[1].clone();
        grant.capability = capability;
        c.grants.push(grant);
    }
    let mut native = open_private_dig_exact(
        &native_path,
        &c,
        DigMode::Control,
        Some(binding(5000)?),
        true,
    )?;
    let journal_id = native.list(&c, 1, None)?.journal_id;
    let original_native_bytes = fs::read(&native_path)?;
    let plan = definition()?;
    let linked = BatchDefinition::new(
        plan.layout().clone(),
        plan.binding().clone(),
        journal_id,
        capture()?,
    )?;
    let mut parent = open_private_blueprint_store(
        &directory.file(),
        &context()?,
        DigMode::Control,
        Some(linked),
        true,
        None,
    )?;
    parent.stop(&query_context()?)?;
    let pin = parent.recovery();
    assert_eq!(parent.definition().journal_id(), journal_id);
    assert_eq!(native.list(&c, 1, None)?.journal_id, journal_id);
    assert_eq!(fs::read(&native_path)?, original_native_bytes);
    drop(parent);
    drop(native);
    assert!(private_reopen(&native_path, DigMode::Offline, pin).is_err());
    assert_eq!(fs::read(&native_path)?, original_native_bytes);
    Ok(())
}

#[test]
fn file_parent_replacement_removal_and_hardlinks_fence_open_custody() -> TestResult {
    for fault in 0..4 {
        let directory = Directory::new()?;
        let path = directory.file();
        let mut owner = private_create(&path)?;
        let bytes = fs::read(&path)?;
        match fault {
            0 => fs::remove_file(&path)?,
            1 => {
                fs::rename(&path, directory.parent().join("old"))?;
                fs::write(&path, &bytes)?;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
            }
            2 => fs::hard_link(&path, directory.parent().join("alias"))?,
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
        assert!(owner.stop(&query_context()?).is_err());
    }
    Ok(())
}

#[test]
fn symlinks_modes_oversize_and_stopped_parent_rollback_are_refused() -> TestResult {
    let directory = Directory::new()?;
    let path = directory.file();
    let mut owner = private_create(&path)?;
    let unstopped = fs::read(&path)?;
    owner.stop(&query_context()?)?;
    let pin = owner.recovery();
    drop(owner);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640))?;
    assert!(private_reopen(&path, DigMode::Offline, pin).is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    let parent_alias = directory.0.join("alias");
    symlink(directory.parent(), &parent_alias)?;
    assert!(
        private_reopen(
            &parent_alias.join("blueprint.journal"),
            DigMode::Offline,
            pin
        )
        .is_err()
    );
    fs::write(&path, &unstopped)?;
    assert!(private_reopen(&path, DigMode::Recover, pin).is_err());
    assert_eq!(fs::read(&path)?, unstopped);
    fs::write(&path, vec![0; MAX_STORE_BYTES + 1])?;
    assert!(private_reopen(&path, DigMode::Offline, pin).is_err());
    assert_eq!(fs::metadata(&path)?.len(), (MAX_STORE_BYTES + 1) as u64);
    fs::remove_file(&path)?;
    let target = directory.parent().join("target");
    fs::write(&target, b"preserved")?;
    symlink(&target, &path)?;
    assert!(private_create(&path).is_err());
    assert_eq!(fs::read(&target)?, b"preserved");
    fs::remove_file(&path)?;
    fs::set_permissions(directory.parent(), fs::Permissions::from_mode(0o750))?;
    assert!(private_create(&path).is_err());
    assert!(!path.exists());
    Ok(())
}
