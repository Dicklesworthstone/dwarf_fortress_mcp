//! Actual Linux file-backed coordinator tests. Native effects use explicit doubles.
use super::*;
use std::fs::DirBuilder;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt, symlink};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::dig_designation::{
    DigEffect, DigObservation, DigPhase, DigPlan, DigRegion,
    journal::{DigGuard, DigStage, DigState, private_file::open_private_dig},
    rpc::{DigManifest, DigPreparation, DigSource},
    tests::{fixture, plan},
};
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, GameTick, MapCoord, MapCuboid, ObservationCursor, RequestId,
    SessionId, StateAnchor, WorkBudget,
};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);
struct Directory(PathBuf);
impl Directory {
    fn new() -> Result<Self> {
        let base = std::env::temp_dir().canonicalize().map_err(failure)?;
        for _ in 0..100 {
            let serial = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!(
                "dfmcp-dig-rust-file-{}-{serial}",
                std::process::id()
            ));
            match DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(cause) if cause.kind() == io::ErrorKind::AlreadyExists => {}
                Err(cause) => return Err(failure(cause)),
            }
        }
        Err(error(
            ErrorCode::Conflict,
            "test directory namespace exhausted",
        ))
    }
    fn file(&self) -> PathBuf {
        self.0.join("dig.journal")
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn scope() -> MapCuboid {
    MapCuboid {
        min: MapCoord::new(0, 0, 0),
        max: MapCoord::new(63, 63, 7),
    }
}
fn context() -> Result<OperationContext> {
    let p = plan()?;
    Ok(OperationContext {
        session_id: SessionId::new(81),
        request_id: RequestId::new(1),
        anchor: StateAnchor {
            fortress_id: p.before().fortress_id(),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(p.before().tick()),
            state_hash: p.before().witness(),
        },
        budget: WorkBudget {
            max_wall_millis: 60_000,
            max_game_ticks: 0,
            max_entities: 1000,
            max_bytes: 1024 * 1024 * 1024,
            max_output_tokens: 65_536,
            max_actions: 1,
        },
        grants: [
            Capability::Query,
            Capability::Observe,
            Capability::Plan,
            Capability::Designate,
        ]
        .into_iter()
        .map(|capability| CapabilityGrant {
            capability,
            scope: CapabilityScope {
                fortress_id: Some(p.before().fortress_id()),
                map_area: Some(scope()),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::Guarded,
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
        cancellation_requested: false,
    })
}
fn binding() -> Result<DigBinding> {
    DigBinding::new(
        "127.0.0.1:5000".parse().map_err(|_| failure(denied()))?,
        DigManifest {
            generation: 7,
            df_version: "test-df".into(),
            dfhack_version: "test-dfhack".into(),
        },
        plan()?.before(),
        scope(),
    )
}
fn create(path: &Path) -> Result<DigJournal<PrivateDigFile>> {
    open_private_dig(path, &context()?, DigMode::Control, Some(binding()?))
}
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(failure)?;
    file.write_all(bytes).map_err(failure)
}
struct Guard {
    checks: Vec<DigStage>,
}
impl DigGuard for Guard {
    fn check(&mut self, stage: DigStage, _: &DigPlan, _: &OperationContext) -> Result<()> {
        self.checks.push(stage);
        Ok(()) // TEST DOUBLE: no real runtime/lease authority.
    }
}
struct Native {
    manifest: DigManifest,
    record: Option<DigEffect>,
    calls: Vec<DigStage>,
    commits: usize,
    lose_reply: bool,
}
impl Native {
    fn new() -> Result<Self> {
        Ok(Self {
            manifest: binding()?.manifest().clone(),
            record: None,
            calls: Vec::new(),
            commits: 0,
            lose_reply: false,
        })
    }
}
impl DigSource for Native {
    fn manifest(&self) -> &DigManifest {
        &self.manifest
    }
    fn endpoint(&self) -> Option<std::net::SocketAddr> {
        "127.0.0.1:5000".parse().ok()
    }
    fn observe(&mut self, region: DigRegion, _: &OperationContext) -> Result<DigObservation> {
        self.calls.push(DigStage::Observe);
        assert_eq!(region, plan()?.before().region());
        Ok(plan()?.before().clone())
    }
    fn prepare(&mut self, p: &DigPlan, _: &OperationContext) -> Result<DigPreparation> {
        self.calls.push(DigStage::Prepare);
        let raw = fixture("prepared")?;
        self.record = Some(DigEffect::decode(&raw, p)?);
        DigPreparation::decode(&raw, false, p)
    }
    fn commit(&mut self, p: &DigPlan, _: &OperationContext) -> Result<DigEffect> {
        self.calls.push(DigStage::Commit);
        self.commits += 1;
        let record = DigEffect::decode(&fixture("designated")?, p)?;
        self.record = Some(record.clone());
        if self.lose_reply {
            return Err(error(
                ErrorCode::EffectIndeterminate,
                "test lost native reply",
            ));
        }
        Ok(record)
    }
    fn query(&mut self, _: &DigPlan, _: &OperationContext) -> Result<Option<DigEffect>> {
        self.calls.push(DigStage::Query);
        Ok(self.record.clone())
    }
    fn cancel(&mut self, p: &DigPlan, _: &OperationContext) -> Result<DigEffect> {
        self.calls.push(DigStage::Cancel);
        let record = DigEffect::decode(&fixture("cancelled")?, p)?;
        self.record = Some(record.clone());
        Ok(record)
    }
}

#[test]
fn private_creation_reopen_and_query_only_offline_identity() -> Result<()> {
    let directory = Directory::new()?;
    let c = context()?;
    let j = create(&directory.file())?;
    let id = j.id();
    let head = j.head();
    assert_eq!(
        fs::metadata(directory.file()).map_err(failure)?.mode() & 0o7777,
        0o600
    );
    drop(j);
    let mut query = c.clone();
    query.grants.retain(|g| g.capability == Capability::Query);
    let mut reopened = open_private_dig(&directory.file(), &query, DigMode::Offline, None)?;
    assert_eq!(reopened.id(), id);
    assert_eq!(reopened.head(), head);
    assert_eq!(reopened.list(&query, 8, None)?.total_records, 0);
    assert!(reopened.storage.read_only);
    Ok(())
}
#[test]
fn missing_recovery_and_existing_empty_files_never_initialize() -> Result<()> {
    let d = Directory::new()?;
    let c = context()?;
    for mode in [DigMode::Recover, DigMode::Offline] {
        assert!(open_private_dig(&d.file(), &c, mode, Some(binding()?)).is_err());
        assert!(!d.file().exists());
    }
    write_private(&d.file(), b"")?;
    for mode in [DigMode::Control, DigMode::Recover, DigMode::Offline] {
        assert!(open_private_dig(&d.file(), &c, mode, Some(binding()?)).is_err());
        assert!(fs::read(d.file()).map_err(failure)?.is_empty());
    }
    Ok(())
}
#[test]
fn cancellation_missing_or_expired_query_and_invalid_paths_refuse_before_creation() -> Result<()> {
    let d = Directory::new()?;
    for case in 0..5 {
        let mut c = context()?;
        match case {
            0 => c.cancellation_requested = true,
            1 => c.grants.clear(),
            2 => {
                for grant in &mut c.grants {
                    grant.remaining_uses = Some(1);
                }
            }
            3 => {
                for grant in &mut c.grants {
                    grant.expires_at_tick = Some(GameTick(0));
                }
            }
            _ => c.budget.max_wall_millis = 60_001,
        }
        assert!(open_private_dig(&d.file(), &c, DigMode::Control, Some(binding()?)).is_err());
        assert!(!d.file().exists());
    }
    for suffix in [
        "//dig.journal",
        "/./dig.journal",
        "/../dig.journal",
        "/dig\0journal",
    ] {
        let path = PathBuf::from(format!("{}{suffix}", d.0.display()));
        assert!(create(&path).is_err());
    }
    assert!(create(Path::new("relative.journal")).is_err());
    Ok(())
}
#[test]
fn exclusive_lock_blocks_another_open_and_release_allows_recovery() -> Result<()> {
    let d = Directory::new()?;
    let c = context()?;
    let j = create(&d.file())?;
    for mode in [DigMode::Control, DigMode::Recover, DigMode::Offline] {
        assert!(open_private_dig(&d.file(), &c, mode, Some(binding()?)).is_err());
    }
    drop(j);
    assert!(open_private_dig(&d.file(), &c, DigMode::Recover, Some(binding()?)).is_ok());
    Ok(())
}
struct ProbeChild(std::process::Child);
impl Drop for ProbeChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn child_probe(path: &Path, locked: bool) -> Result<()> {
    let mut child = ProbeChild(
        Command::new(std::env::current_exe().map_err(failure)?)
            .args([
                "--exact",
                "dig_designation::journal::private_file::linux::tests::lock_probe",
                "--nocapture",
            ])
            .env("DFMCP_TEST_DIG_LOCK_PATH", path)
            .env(
                "DFMCP_TEST_DIG_LOCK_EXPECTED",
                if locked { "1" } else { "0" },
            )
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(failure)?,
    );
    let started = Instant::now();
    loop {
        match child.0.try_wait().map_err(failure)? {
            Some(status) => {
                assert!(status.success(), "bounded lock child failed");
                return Ok(());
            }
            None if started.elapsed() < Duration::from_secs(5) => {
                std::thread::sleep(Duration::from_millis(10))
            }
            None => {
                let _ = child.0.kill();
                let _ = child.0.wait();
                return Err(error(
                    ErrorCode::VerificationTimeout,
                    "lock child did not quiesce",
                ));
            }
        }
    }
}
#[test]
fn lock_probe() -> Result<()> {
    let Some(path) = std::env::var_os("DFMCP_TEST_DIG_LOCK_PATH") else {
        return Ok(());
    };
    let locked =
        std::env::var("DFMCP_TEST_DIG_LOCK_EXPECTED").map_err(|_| failure(denied()))? == "1";
    let result = open_private_dig(
        Path::new(&path),
        &context()?,
        DigMode::Offline,
        Some(binding()?),
    );
    assert_eq!(result.is_err(), locked);
    Ok(())
}
#[test]
fn exclusive_lock_is_enforced_in_another_process() -> Result<()> {
    let d = Directory::new()?;
    let j = create(&d.file())?;
    child_probe(&d.file(), true)?;
    drop(j);
    child_probe(&d.file(), false)
}
#[test]
fn file_modes_symlinks_hardlinks_and_parent_modes_refuse() -> Result<()> {
    let d = Directory::new()?;
    drop(create(&d.file())?);
    for mode in [0o400, 0o640, 0o660, 0o1600] {
        fs::set_permissions(d.file(), fs::Permissions::from_mode(mode)).map_err(failure)?;
        assert!(open_private_dig(&d.file(), &context()?, DigMode::Offline, None).is_err());
    }
    fs::set_permissions(d.file(), fs::Permissions::from_mode(0o600)).map_err(failure)?;
    let alias = d.0.join("alias");
    symlink(d.file(), &alias).map_err(failure)?;
    assert!(create(&alias).is_err());
    fs::remove_file(&alias).map_err(failure)?;
    fs::hard_link(d.file(), &alias).map_err(failure)?;
    assert!(create(&d.file()).is_err());
    fs::remove_file(&alias).map_err(failure)?;
    let directory_alias = d.0.join("directory-alias");
    symlink(&d.0, &directory_alias).map_err(failure)?;
    assert!(create(&directory_alias.join("dig.journal")).is_err());
    fs::set_permissions(&d.0, fs::Permissions::from_mode(0o750)).map_err(failure)?;
    assert!(create(&d.file()).is_err());
    fs::set_permissions(&d.0, fs::Permissions::from_mode(0o700)).map_err(failure)?;
    Ok(())
}
#[test]
fn offline_storage_cannot_write_flush_sync_or_truncate() -> Result<()> {
    let d = Directory::new()?;
    drop(create(&d.file())?);
    let raw = fs::read(d.file()).map_err(failure)?;
    let mut offline = open_private_dig(&d.file(), &context()?, DigMode::Offline, None)?;
    offline.storage.seek(SeekFrom::End(0)).map_err(failure)?;
    assert!(offline.storage.write_all(b"x").is_err());
    assert!(offline.storage.flush().is_err());
    assert!(offline.storage.sync().is_err());
    assert!(offline.storage.truncate(0).is_err());
    offline.list(&context()?, 8, None)?;
    assert_eq!(fs::read(d.file()).map_err(failure)?, raw);
    Ok(())
}
#[test]
fn nonappend_writes_tail_repair_and_oversized_files_are_refused() -> Result<()> {
    let d = Directory::new()?;
    let mut j = create(&d.file())?;
    let raw = fs::read(d.file()).map_err(failure)?;
    j.storage.seek(SeekFrom::Start(0)).map_err(failure)?;
    assert!(j.storage.write_all(b"bad").is_err());
    assert!(j.storage.truncate(0).is_err());
    assert_eq!(fs::read(d.file()).map_err(failure)?, raw);
    drop(j);
    let mut file = OpenOptions::new()
        .append(true)
        .open(d.file())
        .map_err(failure)?;
    file.write_all(b"DFMDJR01\0").map_err(failure)?;
    drop(file);
    let partial = fs::read(d.file()).map_err(failure)?;
    for mode in [DigMode::Control, DigMode::Recover, DigMode::Offline] {
        assert!(open_private_dig(&d.file(), &context()?, mode, Some(binding()?)).is_err());
        assert_eq!(fs::read(d.file()).map_err(failure)?, partial);
    }
    OpenOptions::new()
        .write(true)
        .open(d.file())
        .map_err(failure)?
        .set_len(super::super::super::MAX_JOURNAL_BYTES as u64 + 1)
        .map_err(failure)?;
    assert!(open_private_dig(&d.file(), &context()?, DigMode::Offline, None).is_err());
    Ok(())
}
#[test]
fn same_length_corruption_fences_live_journal() -> Result<()> {
    let d = Directory::new()?;
    let mut j = create(&d.file())?;
    let mut raw = fs::read(d.file()).map_err(failure)?;
    raw[12] ^= 1;
    fs::write(d.file(), &raw).map_err(failure)?;
    assert!(j.list(&context()?, 8, None).is_err());
    assert!(j.fenced());
    assert_eq!(fs::read(d.file()).map_err(failure)?, raw);
    Ok(())
}
#[test]
fn named_inode_parent_or_permissions_cannot_change_under_custody() -> Result<()> {
    let d = Directory::new()?;
    for case in 0..3 {
        let root = d.0.join(format!("case-{case}"));
        DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .map_err(failure)?;
        let path = root.join("dig.journal");
        let mut j = create(&path)?;
        match case {
            0 => {
                let raw = fs::read(&path).map_err(failure)?;
                fs::rename(&path, root.join("old")).map_err(failure)?;
                write_private(&path, &raw)?;
            }
            1 => {
                fs::rename(&root, d.0.join("moved")).map_err(failure)?;
                DirBuilder::new()
                    .mode(0o700)
                    .create(&root)
                    .map_err(failure)?;
            }
            _ => fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).map_err(failure)?,
        }
        assert!(j.list(&context()?, 8, None).is_err());
        assert!(j.fenced());
    }
    Ok(())
}
#[test]
fn persisted_preparation_cannot_commit_after_reopen_and_cancel_is_native() -> Result<()> {
    let d = Directory::new()?;
    let c = context()?;
    let p = plan()?;
    let mut j = create(&d.file())?;
    let mut native = Native::new()?;
    let mut guard = Guard { checks: Vec::new() };
    j.prepare(&mut native, &p, &c, &mut guard)?;
    drop(j);
    let mut recovered = open_private_dig(&d.file(), &c, DigMode::Control, Some(binding()?))?;
    assert!(!recovered.list(&c, 8, None)?.records[0].dispatchable);
    assert!(
        recovered
            .commit(&mut native, p.key(), p.digest(), &c, &mut guard)
            .is_err()
    );
    let cancelled = recovered.cancel(&mut native, p.key(), p.digest(), &c, &mut guard)?;
    assert_eq!(cancelled.state(), DigState::Terminal);
    assert_eq!(
        cancelled.effect().map(DigEffect::phase),
        Some(DigPhase::Refused)
    );
    assert!(native.calls.contains(&DigStage::Cancel));
    assert_eq!(native.commits, 0);
    Ok(())
}
#[test]
fn terminal_proof_reopens_offline_without_native_work() -> Result<()> {
    let d = Directory::new()?;
    let c = context()?;
    let p = plan()?;
    let mut j = create(&d.file())?;
    let mut native = Native::new()?;
    let mut guard = Guard { checks: Vec::new() };
    j.prepare(&mut native, &p, &c, &mut guard)?;
    let terminal = j.commit(&mut native, p.key(), p.digest(), &c, &mut guard)?;
    let head = j.head();
    drop(j);
    let raw = fs::read(d.file()).map_err(failure)?;
    let mut offline = open_private_dig(&d.file(), &c, DigMode::Offline, None)?;
    assert_eq!(offline.head(), head);
    assert_eq!(offline.get(p.key(), p.digest(), &c)?, terminal);
    let calls = native.calls.len();
    offline.reconcile(&mut native, p.key(), p.digest(), &c, &mut guard)?;
    assert_eq!(native.calls.len(), calls);
    assert_eq!(native.commits, 1);
    assert_eq!(fs::read(d.file()).map_err(failure)?, raw);
    Ok(())
}
#[test]
fn lost_reply_recovers_from_file_by_query_not_commit() -> Result<()> {
    let d = Directory::new()?;
    let c = context()?;
    let p = plan()?;
    let mut j = create(&d.file())?;
    let mut native = Native::new()?;
    let mut guard = Guard { checks: Vec::new() };
    j.prepare(&mut native, &p, &c, &mut guard)?;
    native.lose_reply = true;
    assert!(
        j.commit(&mut native, p.key(), p.digest(), &c, &mut guard)
            .is_err()
    );
    drop(j);
    let mut recovered = open_private_dig(&d.file(), &c, DigMode::Recover, Some(binding()?))?;
    assert_eq!(
        recovered.get(p.key(), p.digest(), &c)?.state(),
        DigState::DispatchStarted
    );
    assert!(
        recovered
            .commit(&mut native, p.key(), p.digest(), &c, &mut guard)
            .is_err()
    );
    let result = recovered.reconcile(&mut native, p.key(), p.digest(), &c, &mut guard)?;
    assert_eq!(
        result.effect().map(DigEffect::phase),
        Some(DigPhase::Designated)
    );
    assert_eq!(native.calls.last(), Some(&DigStage::Query));
    assert_eq!(native.commits, 1);
    Ok(())
}
#[test]
fn wrong_binding_or_narrow_query_cannot_reopen_or_rewrite_history() -> Result<()> {
    let d = Directory::new()?;
    let c = context()?;
    drop(create(&d.file())?);
    let raw = fs::read(d.file()).map_err(failure)?;
    let wrong = DigBinding::new(
        "127.0.0.1:5001".parse().map_err(|_| failure(denied()))?,
        binding()?.manifest().clone(),
        plan()?.before(),
        scope(),
    )?;
    assert!(open_private_dig(&d.file(), &c, DigMode::Recover, Some(wrong)).is_err());
    let mut narrow = c.clone();
    for grant in &mut narrow.grants {
        grant.scope.map_area = Some(plan()?.before().region().halo());
    }
    assert!(open_private_dig(&d.file(), &narrow, DigMode::Offline, None).is_err());
    assert_eq!(fs::read(d.file()).map_err(failure)?, raw);
    Ok(())
}

#[test]
fn file_and_parent_sync_failures_are_propagated_in_order() -> Result<()> {
    let d = Directory::new()?;
    let j = create(&d.file())?;
    for failed in [1, 2] {
        let mut calls = Vec::new();
        let result = j.storage.synchronize(|file| {
            calls.push(file.metadata()?.is_dir());
            if calls.len() == failed {
                return Err(io::Error::other("injected individual sync failure"));
            }
            file.sync_all()
        });
        assert!(result.is_err());
        assert_eq!(calls.as_slice(), &([false, true][..failed]));
    }
    j.storage.synchronize(File::sync_all).map_err(failure)?;
    Ok(())
}
