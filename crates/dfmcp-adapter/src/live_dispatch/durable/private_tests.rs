use super::*;
use std::fs;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Directory(PathBuf);
impl Directory {
    fn new() -> io::Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-durable-live-dispatch-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        let custody = path.join("custody");
        fs::create_dir(&custody)?;
        fs::set_permissions(&custody, fs::Permissions::from_mode(0o700))?;
        Ok(Self(path))
    }
    fn path(&self) -> PathBuf {
        self.0.join("custody").join("frontier.journal")
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn private_creation_lock_and_query_only_reopen_preserve_the_complete_original() -> TestResult {
    let directory = Directory::new()?;
    let path = directory.path();
    let mut c = context();
    c.session_id = SessionId::new((1_u128 << 101) + 18);
    c.request_id = RequestId::new((1_u128 << 99) + 1);
    let original = plan(16);
    original.validate_structure()?;
    let pin = initial_recovery(&original, &c)?;
    let mut owner = open_private_coordinator(
        &path,
        &c,
        BuildMode::Control,
        Some(original.clone()),
        pin,
        true,
    )?;
    assert_eq!(owner.original_plan(), &original);
    assert_eq!(owner.inspect(&c)?.entries.len(), 16);
    assert!(open_private_coordinator(&path, &c, BuildMode::Offline, None, pin, false).is_err());
    assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o7777, 0o600);
    drop(owner);
    let before = fs::read(&path)?;
    let mut query = c.clone();
    query.session_id = SessionId::new(29);
    query
        .grants
        .retain(|grant| grant.capability == Capability::Query);
    query.budget.max_actions = 1;
    let mut recovered =
        open_private_coordinator(&path, &query, BuildMode::Recover, None, pin, false)?;
    assert_eq!(recovered.original_plan(), &original);
    assert!(recovered.inspect(&query)?.review.is_none());
    drop(recovered);
    assert_eq!(fs::read(&path)?, before);
    assert!(
        open_private_coordinator(&path, &c, BuildMode::Control, Some(original), pin, true).is_err()
    );
    assert_eq!(fs::read(&path)?, before);
    Ok(())
}

#[test]
fn private_missing_recovery_and_expired_pinned_floor_never_create_a_file() -> TestResult {
    let directory = Directory::new()?;
    let path = directory.path();
    let original = plan(2);
    let c = context();
    let pin = initial_recovery(&original, &c)?;
    for mode in [BuildMode::Control, BuildMode::Recover, BuildMode::Offline] {
        assert!(open_private_coordinator(&path, &c, mode, None, pin, false).is_err());
        assert!(!path.exists());
    }
    let mut expired = c.clone();
    for grant in &mut expired.grants {
        if grant.capability == Capability::Query {
            grant.expires_at_tick = Some(GameTick(200));
        }
    }
    let future = Recovery {
        authority_tick_floor: 300,
        ..pin
    };
    assert!(
        open_private_coordinator(
            &path,
            &expired,
            BuildMode::Control,
            Some(original),
            future,
            true,
        )
        .is_err()
    );
    assert!(!path.exists());
    Ok(())
}

#[test]
fn private_file_or_parent_substitution_fences_current_custody() -> TestResult {
    for parent in [false, true] {
        let directory = Directory::new()?;
        let path = directory.path();
        let c = context();
        let original = plan(2);
        let pin = initial_recovery(&original, &c)?;
        let mut owner =
            open_private_coordinator(&path, &c, BuildMode::Control, Some(original), pin, true)?;
        let bytes = fs::read(&path)?;
        if parent {
            let custody = directory.0.join("custody");
            fs::rename(&custody, directory.0.join("preserved-parent"))?;
            fs::create_dir(&custody)?;
            fs::set_permissions(&custody, fs::Permissions::from_mode(0o700))?;
        } else {
            fs::rename(&path, directory.0.join("custody").join("preserved-file"))?;
        }
        let mut replacement = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        replacement.write_all(&bytes)?;
        drop(replacement);
        assert!(owner.inspect(&c).is_err());
        assert!(owner.is_fenced());
    }
    Ok(())
}

#[test]
fn private_oversize_and_symlink_custody_are_refused_without_repair() -> TestResult {
    let directory = Directory::new()?;
    let path = directory.path();
    let c = context();
    let pin = initial_recovery(&plan(2), &c)?;
    let large = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    large.set_len((MAX_STORE_BYTES + 1) as u64)?;
    drop(large);
    assert!(open_private_coordinator(&path, &c, BuildMode::Offline, None, pin, false).is_err());
    assert_eq!(fs::metadata(&path)?.len(), (MAX_STORE_BYTES + 1) as u64);
    fs::remove_file(&path)?;
    let target = directory.0.join("custody").join("preserved");
    fs::write(&target, b"untouched")?;
    symlink(&target, &path)?;
    assert!(
        open_private_coordinator(&path, &c, BuildMode::Control, Some(plan(2)), pin, true).is_err()
    );
    assert_eq!(fs::read(&target)?, b"untouched");
    Ok(())
}
