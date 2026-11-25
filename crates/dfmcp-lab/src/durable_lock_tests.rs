//! Store ownership must survive journal inode replacement during compaction.

use super::*;
use dfmcp_world::WorldGraph;

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-store-lock-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id(),
        ));
        let _ = fs::remove_dir_all(&path);
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn snapshot(tick: u64) -> WorldSnapshot {
    WorldSnapshot::new(
        FortressId::new(7),
        GameTick(tick),
        ObservationCursor {
            epoch: 1,
            sequence: tick,
        },
        false,
        WorldGraph::default(),
    )
}

fn open_journal(root: &Path) -> Result<File> {
    private_options()
        .read(true)
        .append(true)
        .open(root.join("journal"))
        .map_err(|e| io("open test journal", &e))
}

#[test]
fn compaction_keeps_store_exclusive_when_the_replaced_journal_unlocks() -> Result<()> {
    let dir = TempDir::new("compaction");
    let mut store = DurableLabStore::open(&dir.0)?;
    store.persist_head("empty", &snapshot(1))?;
    // Model an opener paused between opening the journal and locking it.
    // This descriptor continues naming the old inode after compaction.
    let stale_journal = open_journal(&dir.0)?;
    assert!(stale_journal.try_lock().is_err());
    let store_lock = private_options()
        .read(true)
        .write(true)
        .open(dir.0.join("store.lock"))
        .map_err(|e| io("open test store lock", &e))?;
    assert!(store_lock.try_lock().is_err());
    store.persist_head("empty", &snapshot(2))?;
    store.compact_with_directory_sync(|root| {
        // The vulnerable interleaving is deterministic: rename has published
        // the replacement and dropped the old journal lock.
        stale_journal
            .try_lock()
            .map_err(|e| corrupt(format!("old journal remained locked: {e}")))?;
        assert!(store_lock.try_lock().is_err());
        let second = DurableLabStore::open(root);
        assert!(matches!(second, Err(ref e) if e.code == ErrorCode::Conflict));
        sync_dir(root)
    })?;
    // Ownership must remain on the same persistent file for every later
    // compaction, even though a stale journal descriptor is now locked.
    store.compact()?;
    assert!(store_lock.try_lock().is_err());
    let latest = snapshot(3);
    store.persist_head("empty", &latest)?;
    drop(store);
    store_lock
        .try_lock()
        .map_err(|e| corrupt(format!("store ownership was not released: {e}")))?;
    drop(store_lock);
    // An obsolete journal lock cannot obscure the actual published head.
    let recovered = DurableLabStore::open(&dir.0)?;
    assert_eq!(
        recovered.head(latest.fortress_id).map(|head| head.anchor),
        Some(latest.anchor())
    );
    assert_eq!(recovered.load_snapshot(latest.state_hash)?, latest);
    Ok(())
}

#[test]
fn store_lock_is_acquired_before_inspecting_or_creating_store_contents() -> Result<()> {
    let dir = TempDir::new("before-inspection");
    private_dir(&dir.0)?;
    let held = private_options()
        .read(true)
        .write(true)
        .create_new(true)
        .open(dir.0.join("store.lock"))
        .map_err(|e| io("create test store lock", &e))?;
    held.try_lock()
        .map_err(|e| corrupt(format!("cannot take test store lock: {e}")))?;
    fs::create_dir(dir.0.join("journal")).map_err(|e| io("create invalid test journal", &e))?;
    let second = DurableLabStore::open(&dir.0);
    assert!(matches!(second, Err(ref e) if e.code == ErrorCode::Conflict));
    assert!(!dir.0.join("objects").exists());
    drop(held);
    let invalid = DurableLabStore::open(&dir.0);
    assert!(matches!(invalid, Err(ref e) if e.code == ErrorCode::CorruptLedger));
    Ok(())
}

#[test]
fn legacy_journal_locks_remain_exclusive_across_upgrade_and_compaction() -> Result<()> {
    let dir = TempDir::new("legacy");
    let latest = snapshot(10);
    {
        let mut store = DurableLabStore::open(&dir.0)?;
        store.persist_head("empty", &latest)?;
    }
    // A pre-upgrade store has the same journal and no store-wide lock file.
    fs::remove_file(dir.0.join("store.lock")).map_err(|e| io("remove test store lock", &e))?;
    let journal_path = dir.0.join("journal");
    let bytes = fs::read(&journal_path).map_err(|e| io("read legacy journal", &e))?;
    let legacy = open_journal(&dir.0)?;
    legacy
        .try_lock()
        .map_err(|e| corrupt(format!("cannot take legacy journal lock: {e}")))?;
    let second = DurableLabStore::open(&dir.0);
    assert!(matches!(second, Err(ref e) if e.code == ErrorCode::Conflict));
    assert_eq!(
        fs::read(&journal_path).map_err(|e| io("read legacy journal", &e))?,
        bytes
    );
    drop(legacy);
    // Failure at the legacy journal lock releases the new store lock too.
    let mut store = DurableLabStore::open(&dir.0)?;
    assert_eq!(
        store.head(latest.fortress_id).map(|head| head.anchor),
        Some(latest.anchor())
    );
    assert_eq!(
        fs::read(&journal_path).map_err(|e| io("read upgraded journal", &e))?,
        bytes
    );
    for _ in 0..2 {
        let legacy = open_journal(&dir.0)?;
        assert!(legacy.try_lock().is_err());
        store.compact()?;
    }
    Ok(())
}

#[test]
fn store_lock_refuses_directories_and_symbolic_links() -> Result<()> {
    let dir = TempDir::new("directory");
    private_dir(&dir.0)?;
    fs::create_dir(dir.0.join("store.lock")).map_err(|e| io("create test lock directory", &e))?;
    let result = DurableLabStore::open(&dir.0);
    assert!(matches!(result, Err(ref e) if e.code == ErrorCode::CorruptLedger));
    assert!(!dir.0.join("objects").exists());
    #[cfg(unix)]
    {
        let dir = TempDir::new("symlink");
        private_dir(&dir.0)?;
        std::os::unix::fs::symlink("/etc/hostname", dir.0.join("store.lock"))
            .map_err(|e| io("symlink test lock", &e))?;
        let result = DurableLabStore::open(&dir.0);
        assert!(matches!(result, Err(ref e) if e.code == ErrorCode::CorruptLedger));
        assert!(!dir.0.join("objects").exists());
    }
    Ok(())
}
