//! Real Linux private-file custody for durable read-only monitoring.
use super::super::{Recovery, initial_recovery, open_private_monitor_store};
use crate::dig_blueprint::monitor::tests::{capture, context, parts, spec as fixture_spec};
use crate::dig_designation::journal::DigMode;
use dfmcp_core::{Capability, Digest32};
use std::fs::{self, OpenOptions};
use std::io;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Directory(PathBuf);
impl Directory {
    fn new() -> io::Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-blueprint-monitor-{}-{}",
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
        self.parent().join("excavation.monitor")
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn private_initialize_is_exclusive_and_reopening_requires_exact_recovery() -> TestResult {
    let directory = Directory::new()?;
    let path = directory.file();
    let spec = fixture_spec(parts())?;
    let scope = spec.parent().binding().scope();
    let c = context(&spec);
    assert!(
        c.grants
            .iter()
            .all(|g| matches!(g.capability, Capability::Query | Capability::Observe))
    );
    let mut owner = open_private_monitor_store(
        &path,
        &c,
        DigMode::Recover,
        Some(spec.clone()),
        true,
        None,
        scope,
    )?;
    owner.verify(&c)?;
    let pin = owner.recovery();
    assert_eq!(pin, initial_recovery(&spec)?);
    let original = fs::read(&path)?;
    assert!(!original.is_empty());
    assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o7777, 0o600);
    assert!(
        open_private_monitor_store(&path, &c, DigMode::Offline, None, false, Some(pin), scope,)
            .is_err()
    );
    assert!(
        open_private_monitor_store(
            &path,
            &c,
            DigMode::Control,
            Some(spec.clone()),
            true,
            None,
            scope,
        )
        .is_err()
    );
    assert_eq!(fs::read(&path)?, original);
    drop(owner);

    for recovery in [
        None,
        Some(Recovery {
            monitor_id: Digest32::of_bytes(b"foreign monitor"),
            head: pin.head,
        }),
        Some(Recovery {
            monitor_id: pin.monitor_id,
            head: Digest32::of_bytes(b"foreign head"),
        }),
    ] {
        assert!(
            open_private_monitor_store(&path, &c, DigMode::Offline, None, false, recovery, scope,)
                .is_err()
        );
        assert_eq!(fs::read(&path)?, original);
    }
    let mut reopened =
        open_private_monitor_store(&path, &c, DigMode::Offline, None, false, Some(pin), scope)?;
    reopened.verify(&c)?;
    assert_eq!(reopened.recovery(), pin);
    assert_eq!(reopened.spec(), &spec);
    drop(reopened);
    assert_eq!(fs::read(&path)?, original);
    Ok(())
}

#[test]
fn private_recovery_never_creates_or_repairs_missing_or_empty_files() -> TestResult {
    let directory = Directory::new()?;
    let path = directory.file();
    let spec = fixture_spec(parts())?;
    let scope = spec.parent().binding().scope();
    let c = context(&spec);
    let pin = initial_recovery(&spec)?;
    for mode in [DigMode::Control, DigMode::Recover, DigMode::Offline] {
        assert!(
            open_private_monitor_store(&path, &c, mode, None, false, Some(pin), scope,).is_err()
        );
        assert!(!path.exists());
    }
    let mut denied = c.clone();
    denied.grants.retain(|g| g.capability == Capability::Query);
    let mut tiny = c.clone();
    tiny.budget.max_bytes = 1;
    for (current, mode) in [
        (&c, DigMode::Offline),
        (&denied, DigMode::Recover),
        (&tiny, DigMode::Control),
    ] {
        assert!(open_private_monitor_store(
            &path, current, mode, Some(spec.clone()), true, None, scope,
        ).is_err());
        assert!(!path.exists());
    }
    drop(
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?,
    );
    for mode in [DigMode::Control, DigMode::Recover] {
        assert!(
            open_private_monitor_store(&path, &c, mode, Some(spec.clone()), true, None, scope,)
                .is_err()
        );
        assert_eq!(fs::metadata(&path)?.len(), 0);
    }
    assert!(
        open_private_monitor_store(&path, &c, DigMode::Offline, None, false, Some(pin), scope,)
            .is_err()
    );
    assert_eq!(fs::metadata(&path)?.len(), 0);
    Ok(())
}

#[test]
fn actual_capture_replay_is_read_only_and_retains_the_learned_clock() -> TestResult {
    let directory = Directory::new()?;
    let path = directory.file();
    let spec = fixture_spec(parts())?;
    let scope = spec.parent().binding().scope();
    let c = context(&spec);
    let mut owner = open_private_monitor_store(
        &path,
        &c,
        DigMode::Recover,
        Some(spec.clone()),
        true,
        None,
        scope,
    )?;
    let attempt = owner.begin_attempt(&c)?;
    owner.capture(&attempt, capture(&spec, 1010)?, &c)?;
    owner.publish(&attempt, &c)?;
    let pin = owner.recovery();
    assert_eq!(owner.high_tick(), 1010);
    drop(owner);
    let original = fs::read(&path)?;
    let mut query = c.clone();
    query.grants.retain(|g| g.capability == Capability::Query);
    let mut offline = open_private_monitor_store(
        &path,
        &query,
        DigMode::Offline,
        None,
        false,
        Some(pin),
        scope,
    )?;
    offline.verify(&query)?;
    assert_eq!(offline.high_tick(), 1010);
    assert!(offline.begin_attempt(&query).is_err());
    assert!(offline.cancel(&query).is_err());
    drop(offline);
    assert_eq!(fs::read(&path)?, original);

    let mut expired = query;
    expired
        .grants
        .iter_mut()
        .for_each(|g| g.expires_at_tick = Some(dfmcp_core::GameTick(1009)));
    assert!(
        open_private_monitor_store(
            &path,
            &expired,
            DigMode::Offline,
            None,
            false,
            Some(pin),
            scope,
        )
        .is_err()
    );
    assert_eq!(fs::read(&path)?, original);
    let mut recovered =
        open_private_monitor_store(&path, &c, DigMode::Recover, None, false, Some(pin), scope)?;
    recovered.cancel(&c)?;
    let stopped = recovered.recovery();
    assert_ne!(stopped.head, pin.head);
    drop(recovered);
    let cancelled = fs::read(&path)?;
    let mut offline = open_private_monitor_store(
        &path,
        &c,
        DigMode::Offline,
        None,
        false,
        Some(stopped),
        scope,
    )?;
    offline.verify(&c)?;
    assert_eq!(offline.recovery(), stopped);
    drop(offline);
    assert_eq!(fs::read(&path)?, cancelled);
    Ok(())
}

#[test]
fn monitor_private_path_refuses_symlinks_and_public_file_permissions() -> TestResult {
    let directory = Directory::new()?;
    let path = directory.file();
    let spec = fixture_spec(parts())?;
    let scope = spec.parent().binding().scope();
    let c = context(&spec);
    let owner = open_private_monitor_store(
        &path,
        &c,
        DigMode::Recover,
        Some(spec.clone()),
        true,
        None,
        scope,
    )?;
    let pin = owner.recovery();
    drop(owner);
    let original = fs::read(&path)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640))?;
    assert!(
        open_private_monitor_store(&path, &c, DigMode::Offline, None, false, Some(pin), scope,)
            .is_err()
    );
    assert_eq!(fs::read(&path)?, original);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    let alias = directory.parent().join("alias");
    symlink(&path, &alias)?;
    assert!(
        open_private_monitor_store(&alias, &c, DigMode::Recover, None, false, Some(pin), scope,)
            .is_err()
    );
    assert_eq!(fs::read(&path)?, original);
    let parent_alias = directory.0.join("parent-alias");
    symlink(directory.parent(), &parent_alias)?;
    assert!(
        open_private_monitor_store(
            &parent_alias.join("excavation.monitor"),
            &c,
            DigMode::Offline,
            None,
            false,
            Some(pin),
            scope,
        )
        .is_err()
    );
    assert_eq!(fs::read(&path)?, original);
    Ok(())
}
