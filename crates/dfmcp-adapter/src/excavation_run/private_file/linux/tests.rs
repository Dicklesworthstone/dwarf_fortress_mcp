//! These tests exercise the actual Rust backend when run on supported Linux.
//! No qualification is inferred merely from their source being present.
use super::*;
use crate::excavation_run::{ExcavationRunPlan, ExcavationRunRecord, FortressIdentity};
use crate::excavation_run::coordinator::{ExcavationBinding, ExcavationDispatch, ExcavationRunSource};
use crate::excavation_run::private_file::{create_private_excavation, open_private_excavation, inspect_private_excavation};
use crate::excavation_run::tests::{plan, prepared, stopped};
use dfmcp_core::{Capability, CapabilityGrant, CapabilityScope, GameTick, ObservationCursor,
    RequestId, RiskTier, SessionId, StateAnchor, WorkBudget};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt, symlink};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn context() -> Result<OperationContext> {
    let p = plan()?; let id = p.before().fortress().fortress_id();
    Ok(OperationContext {
        session_id: SessionId::new(1), request_id: RequestId::new(2),
        anchor: StateAnchor { fortress_id: id, cursor: ObservationCursor::ORIGIN,
            tick: GameTick(p.before().tick()), state_hash: p.before().witness() },
        budget: WorkBudget { max_wall_millis: 5000, max_bytes: 32 * 1024 * 1024,
            ..WorkBudget::CONSERVATIVE_DEFAULT },
        grants: [Capability::Query, Capability::Plan, Capability::ControlClock].into_iter().map(|capability|
            CapabilityGrant { capability, scope: CapabilityScope { fortress_id: Some(id), ..CapabilityScope::default() },
                max_risk: RiskTier::Guarded, expires_at_tick: None, remaining_uses: None }).collect(),
        cancellation_requested: false,
    })
}
fn binding() -> Result<ExcavationBinding> {
    ExcavationBinding::new("127.0.0.1:5000".parse().unwrap(), "df", "dfhack", plan()?.before())
}
fn directory() -> PathBuf {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let base = std::env::temp_dir().canonicalize().unwrap();
    loop {
        let path = base.join(format!("dfmcp-rust-excavation-{}-{}", std::process::id(), SERIAL.fetch_add(1, Ordering::Relaxed)));
        match fs::DirBuilder::new().mode(0o700).create(&path) {
            Ok(()) => { fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap(); return path; }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => panic!("private test directory: {e}"),
        }
    }
    // Evidence directories deliberately retained; no automatic deletion of journals.
}
fn fixture() -> Vec<u8> {
    let value = include_str!("../../../../tests/fixtures/excavation_run_journal_v1_18.hex").trim();
    (0..value.len()).step_by(2).map(|n| u8::from_str_radix(&value[n..n + 2], 16).unwrap()).collect()
}
fn write_fixture(dir: &Path) {
    let mut f = OpenOptions::new().write(true).create_new(true).mode(0o600).open(dir.join(JOURNAL_NAME)).unwrap();
    f.write_all(&fixture()).unwrap(); f.sync_all().unwrap();
    File::open(dir).unwrap().sync_all().unwrap();
}
struct Native {
    binding: ExcavationBinding,
    calls: Vec<&'static str>,
    record: Option<ExcavationRunRecord>,
    lose_commit: bool,
    fault_path: Option<PathBuf>,
}
impl Native {
    fn new() -> Result<Self> { Ok(Self { binding: binding()?, calls: Vec::new(), record: None, lose_commit: false, fault_path: None }) }
}
impl ExcavationRunSource for Native {
    fn binding(&self) -> &ExcavationBinding { &self.binding }
    fn fence(&mut self) { self.calls.push("fence"); }
    fn observe(&mut self, _: crate::excavation_run::ExcavationRegion, _: &OperationContext,
        _: Duration) -> Result<crate::excavation_run::ExcavationCapture>
    { self.calls.push("observe"); Ok(plan()?.before().clone()) }
    fn prepare(&mut self, _: &ExcavationRunPlan, _: &OperationContext, _: Duration) -> Result<ExcavationRunRecord> {
        self.calls.push("prepare"); self.record = Some(prepared()?);
        if let Some(path) = &self.fault_path {
            fs::set_permissions(path.join(JOURNAL_NAME), fs::Permissions::from_mode(0o400)).unwrap();
        }
        Ok(prepared()?)
    }
    fn commit(&mut self, _: ExcavationDispatch<'_>, _: &OperationContext, _: Duration) -> Result<ExcavationRunRecord> {
        self.calls.push("commit"); self.record = Some(stopped()?);
        if self.lose_commit { return Err(error(ErrorCode::AdapterUnavailable, "lost reply")); }
        Ok(stopped()?)
    }
    fn query(&mut self, _: &ExcavationRunPlan, _: &OperationContext, _: Duration) -> Result<Option<ExcavationRunRecord>> {
        self.calls.push("query"); Ok(self.record.clone())
    }
    fn cancel(&mut self, _: &ExcavationRunPlan, _: &OperationContext, _: Duration) -> Result<ExcavationRunRecord> {
        self.calls.push("cancel"); Ok(stopped()?)
    }
}

#[test]
fn actual_private_backend_roundtrip_and_strict_offline_archive() -> Result<()> {
    let dir = directory(); let c = context()?; let p = plan()?;
    let mut native = Native::new()?;
    {
        let mut owner = create_private_excavation(&dir, binding()?, &c)?;
        assert_eq!(owner.pending_count(), 0);
        let result = owner.start(&mut native, p.clone(), p.digest(), &c)?;
        assert!(result.resolved());
    }
    let before = fs::read(dir.join(JOURNAL_NAME)).unwrap();
    let metadata = fs::metadata(dir.join(JOURNAL_NAME)).unwrap();
    let mut readonly = c; readonly.grants.retain(|g| g.capability == Capability::Query);
    let archive = inspect_private_excavation(&dir, p.before().fortress(), &readonly)?;
    assert_eq!(archive.pending_count(), 0); assert_eq!(archive.entries().len(), 1);
    assert!(archive.entries()[0].native().unwrap().historical_pause_verified());
    assert_eq!(fs::read(dir.join(JOURNAL_NAME)).unwrap(), before);
    assert_eq!(stamp(&fs::metadata(dir.join(JOURNAL_NAME)).unwrap()), stamp(&metadata));
    assert_eq!(native.calls, vec!["observe", "query", "prepare", "commit"]);
    Ok(())
}
#[test]
fn lost_reply_reopens_original_disk_journal_and_only_queries() -> Result<()> {
    let dir = directory(); let c = context()?; let p = plan()?; let mut native = Native::new()?;
    native.lose_commit = true;
    {
        let mut owner = create_private_excavation(&dir, binding()?, &c)?;
        assert!(owner.start(&mut native, p.clone(), p.digest(), &c).is_err());
        assert_eq!(owner.pending_count(), 1);
    }
    let mut owner = open_private_excavation(&dir, p.before().fortress(), &c)?;
    assert_eq!(owner.pending_count(), 1);
    assert!(owner.recover(&mut native, p.key(), false, &c)?.unwrap().resolved());
    assert_eq!(native.calls.iter().filter(|n| **n == "commit").count(), 1);
    assert_eq!(native.calls.last(), Some(&"query"));
    Ok(())
}
#[test]
fn fixed_name_and_unrelated_files_refuse_creation_and_recovery() -> Result<()> {
    let dir = directory(); let c = context()?; let p = plan()?;
    fs::write(dir.join("other-controller.journal"), b"unresolved").unwrap();
    assert!(create_private_excavation(&dir, binding()?, &c).is_err());
    assert!(!dir.join(JOURNAL_NAME).exists());
    assert!(open_private_excavation(&dir, p.before().fortress(), &c).is_err());
    Ok(())
}
#[test]
fn existing_empty_partial_and_corrupt_files_are_not_reinitialized() -> Result<()> {
    for bytes in [vec![], b"DFMEJ018".to_vec(), fixture()[..100].to_vec(), vec![0; 300]] {
        let dir = directory(); let c = context()?; let p = plan()?;
        let path = dir.join(JOURNAL_NAME); fs::write(&path, &bytes).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(create_private_excavation(&dir, binding()?, &c).is_err());
        assert!(open_private_excavation(&dir, p.before().fortress(), &c).is_err());
        assert!(inspect_private_excavation(&dir, p.before().fortress(), &c).is_err());
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    Ok(())
}
#[test]
fn private_modes_links_and_parent_symlinks_are_refused() -> Result<()> {
    let c = context()?; let p = plan()?;
    for mode in [0o755, 0o500, 0o777, 0o1700] {
        let dir = directory(); fs::set_permissions(&dir, fs::Permissions::from_mode(mode)).unwrap();
        assert!(create_private_excavation(&dir, binding()?, &c).is_err());
    }
    for mode in [0o644, 0o400, 0o1600] {
        let dir = directory(); write_fixture(&dir); fs::set_permissions(dir.join(JOURNAL_NAME), fs::Permissions::from_mode(mode)).unwrap();
        assert!(open_private_excavation(&dir, p.before().fortress(), &c).is_err());
    }
    let dir = directory(); write_fixture(&dir);
    let alias = directory().join("alias"); symlink(&dir, &alias).unwrap();
    assert!(open_private_excavation(&alias, p.before().fortress(), &c).is_err());
    let external = directory().join("hard"); fs::hard_link(dir.join(JOURNAL_NAME), external).unwrap();
    assert!(open_private_excavation(&dir, p.before().fortress(), &c).is_err());
    let other = directory(); symlink(dir.join(JOURNAL_NAME), other.join(JOURNAL_NAME)).unwrap();
    assert!(open_private_excavation(&other, p.before().fortress(), &c).is_err());
    Ok(())
}
#[test]
fn directory_and_file_locking_refuses_second_owner() -> Result<()> {
    let dir = directory(); let c = context()?; let p = plan()?;
    let owner = create_private_excavation(&dir, binding()?, &c)?;
    assert!(open_private_excavation(&dir, p.before().fortress(), &c).is_err());
    assert!(inspect_private_excavation(&dir, p.before().fortress(), &c).is_err());
    let raw = File::open(dir.join(JOURNAL_NAME)).unwrap(); assert!(raw.try_lock().is_err());
    drop(owner);
    open_private_excavation(&dir, p.before().fortress(), &c)?;
    Ok(())
}
#[test]
fn read_only_backend_refuses_every_write_sync_flush_and_truncate() -> Result<()> {
    let dir = directory(); write_fixture(&dir); let c = context()?;
    let mut file = open(&dir, &c, Mode::Inspect)?;
    assert!(file.write(b"bad").is_err()); assert!(file.flush().is_err());
    assert!(file.sync().is_err()); assert!(file.truncate(0).is_err());
    assert_eq!(fs::read(dir.join(JOURNAL_NAME)).unwrap(), fixture());
    Ok(())
}
#[test]
fn append_only_backend_cannot_overwrite_existing_bytes() -> Result<()> {
    let dir = directory(); write_fixture(&dir); let c = context()?;
    let mut file = open(&dir, &c, Mode::Recover)?;
    file.seek(SeekFrom::Start(0)).unwrap();
    assert!(file.write(b"bad").is_err()); assert!(file.fenced.get());
    assert_eq!(fs::read(dir.join(JOURNAL_NAME)).unwrap(), fixture());
    Ok(())
}
#[test]
fn directory_replacement_never_redirects_writes_to_replacement() -> Result<()> {
    let dir = directory(); write_fixture(&dir); let c = context()?;
    let mut file = open(&dir, &c, Mode::Recover)?;
    let renamed = dir.with_extension("retained"); fs::rename(&dir, &renamed).unwrap();
    fs::DirBuilder::new().mode(0o700).create(&dir).unwrap(); write_fixture(&dir);
    assert!(file.seek(SeekFrom::End(0)).is_err()); assert!(file.write(b"bad").is_err());
    assert_eq!(fs::read(dir.join(JOURNAL_NAME)).unwrap(), fixture());
    assert_eq!(fs::read(renamed.join(JOURNAL_NAME)).unwrap(), fixture());
    Ok(())
}
#[test]
fn same_size_external_rewrite_and_file_replacement_fence_owner() -> Result<()> {
    for replace in [false, true] {
        let dir = directory(); write_fixture(&dir); let c = context()?; let mut file = open(&dir, &c, Mode::Recover)?;
        std::thread::sleep(Duration::from_millis(5));
        if replace {
            fs::rename(dir.join(JOURNAL_NAME), directory().join("old")).unwrap(); write_fixture(&dir);
        } else {
            let mut bytes = fixture(); bytes[40] ^= 1; fs::write(dir.join(JOURNAL_NAME), bytes).unwrap();
        }
        assert!(file.validate_identity().is_err()); assert!(file.write(b"bad").is_err());
    }
    Ok(())
}
#[test]
fn custody_failure_after_prepare_stops_before_commit() -> Result<()> {
    let dir = directory(); let c = context()?; let p = plan()?; let mut native = Native::new()?;
    native.fault_path = Some(dir.clone());
    let mut owner = create_private_excavation(&dir, binding()?, &c)?;
    assert!(owner.start(&mut native, p.clone(), p.digest(), &c).is_err());
    assert!(owner.is_fenced()); assert!(!native.calls.contains(&"commit"));
    Ok(())
}
#[test]
fn cancelled_and_wrong_fortress_contexts_do_not_create_files() -> Result<()> {
    let mut c = context()?; let dir = directory(); c.cancellation_requested = true;
    assert!(create_private_excavation(&dir, binding()?, &c).is_err()); assert!(!dir.join(JOURNAL_NAME).exists());
    let dir = directory(); let wrong = FortressIdentity::new("another", 999)?;
    assert!(open_private_excavation(&dir, &wrong, &context()?).is_err()); assert!(!dir.join(JOURNAL_NAME).exists());
    Ok(())
}
#[test]
fn special_file_refuses_without_blocking_on_open() -> Result<()> {
    let dir = directory(); let c = context()?; let p = plan()?;
    let result = Command::new("mkfifo").arg("-m").arg("600").arg(dir.join(JOURNAL_NAME)).status().unwrap();
    assert!(result.success()); let start = Instant::now();
    assert!(inspect_private_excavation(&dir, p.before().fortress(), &c).is_err());
    assert!(start.elapsed() < Duration::from_secs(1));
    Ok(())
}
#[test]
fn missing_store_and_noncanonical_paths_are_not_created_or_followed() -> Result<()> {
    let c = context()?; let p = plan()?; let dir = directory();
    assert!(open_private_excavation(&dir, p.before().fortress(), &c).is_err());
    for bad in [PathBuf::from("relative"), PathBuf::from(format!("{}/.", dir.display())),
        PathBuf::from(format!("{}//", dir.display())), dir.join("missing/../other")]
    { assert!(create_private_excavation(&bad, binding()?, &c).is_err()); }
    assert!(fs::read_dir(dir).unwrap().next().is_none());
    Ok(())
}

#[test]
fn public_workflow_terminal_recovery_is_offline_for_query_and_cancel() -> Result<()> {
    use crate::excavation_run::workflow::{self, ExcavationRecovery, RecoveryAction};
    use crate::excavation_run::rpc::ExcavationCancellation;
    let dir = directory(); write_fixture(&dir); let p = plan()?;
    let mut c = context()?; c.grants.retain(|g| g.capability == Capability::Query);
    let original = fs::read(dir.join(JOURNAL_NAME)).unwrap();
    for action in [RecoveryAction::Query, RecoveryAction::Cancel] {
        let result = workflow::recover(&dir, ExcavationRecovery {
            fortress: p.before().fortress(), key: p.key(), action, nonce: [0; 32],
        }, &c, ExcavationCancellation::default())?;
        assert!(result.unwrap().resolved());
    }
    assert_eq!(fs::read(dir.join(JOURNAL_NAME)).unwrap(), original);
    Ok(())
}
#[test]
fn public_workflow_confirmation_and_pending_fences_precede_connection() -> Result<()> {
    use crate::excavation_run::workflow::{self, ConfirmedExcavationStart};
    use crate::excavation_run::rpc::ExcavationCancellation;
    let c = context()?; let p = plan()?; let dir = directory();
    workflow::initialize(&dir, binding()?, &c)?;
    let original = fs::read(dir.join(JOURNAL_NAME)).unwrap();
    let error = workflow::start(&dir, ConfirmedExcavationStart { plan: p.clone(),
        confirmed_digest: dfmcp_core::Digest32::ZERO, nonce: [0; 32] }, &c, ExcavationCancellation::default()).unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(fs::read(dir.join(JOURNAL_NAME)).unwrap(), original);
    let mut native = Native::new()?; native.lose_commit = true;
    {
        let mut owner = open_private_excavation(&dir, p.before().fortress(), &c)?;
        assert!(owner.start(&mut native, p.clone(), p.digest(), &c).is_err());
    }
    let another = ExcavationRunPlan::new("another", p.spec(), p.before().clone())?;
    let error = workflow::start(&dir, ConfirmedExcavationStart { confirmed_digest: another.digest(),
        plan: another, nonce: [0; 32] }, &c, ExcavationCancellation::default()).unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    Ok(())
}
#[test]
fn cancelled_workflow_does_not_open_or_create_storage() -> Result<()> {
    use crate::excavation_run::workflow::{self, ConfirmedExcavationStart};
    use crate::excavation_run::rpc::ExcavationCancellation;
    let p = plan()?; let c = context()?; let signal = ExcavationCancellation::default(); signal.cancel();
    let dir = directory();
    let error = workflow::start(&dir, ConfirmedExcavationStart { confirmed_digest: p.digest(),
        plan: p, nonce: [0; 32] }, &c, signal).unwrap_err();
    assert_eq!(error.code, ErrorCode::CancellationRequested);
    assert!(fs::read_dir(dir).unwrap().next().is_none());
    Ok(())
}
