#![cfg(unix)]
//! Real private-file recovery through public APIs; no bridge or environment mutation.
use dfmcp_adapter::live_operations::OperationsProfile;
use dfmcp_adapter::live_spatial::{LiveSpatialObservation, LiveSpatialState};
use dfmcp_adapter::operations_journal::{
    JournalLimits, Operations13, PrivateJournalFile, Spatial16, TailRecovery, open_profile_journal,
};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, DfmcpError, ErrorCode, FortressId, GameTick,
    OperationContext, RequestId, Result, RiskTier, SessionId, WorkBudget,
};
use std::fs;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);
fn io_error(_: std::io::Error) -> DfmcpError {
    DfmcpError::new(ErrorCode::CorruptLedger, "archive test filesystem error")
}
struct Fixture {
    directory: PathBuf,
    path: PathBuf,
}
impl Fixture {
    fn new() -> Result<Self> {
        let directory = std::env::temp_dir()
            .canonicalize()
            .map_err(io_error)?
            .join(format!(
                "dfmcp-archive-recovery-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .map_err(io_error)?;
        Ok(Self {
            path: directory.join("spatial.bin"),
            directory,
        })
    }
    fn populated(&self) -> Result<OperationContext> {
        let observation = observation(3)?;
        let c = context(&observation)?;
        let mut journal = open_profile_journal::<Spatial16>(
            &self.path,
            &c,
            JournalLimits::default(),
            TailRecovery::Refuse,
        )?;
        journal.append(observation, &c)?;
        journal.append(observation_at_next_tick()?, &c)?;
        Ok(c)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // Only this test's exclusively created paths are eligible for cleanup.
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_file(self.directory.join("alias.bin"));
        let _ = fs::remove_dir(&self.directory);
    }
}
fn observation(tick: u32) -> Result<LiveSpatialObservation> {
    let hex = include_str!("fixtures/spatial_v1_6.hex").trim();
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16)
                .map_err(|_| DfmcpError::new(ErrorCode::InvalidRequest, "fixture hex"))
        })
        .collect::<Result<Vec<_>>>()?;
    let base = LiveSpatialObservation::decode_payload(&bytes, 7, "df".into(), "dfhack".into())?;
    let mut op = base.operations().clone();
    let mut terrain = base.terrain().clone();
    op.jobs.year_tick = tick;
    terrain.year_tick = tick;
    let mut bytes = b"DFMS1600".to_vec();
    for part in [
        op.encode_profile(OperationsProfile::PagedV1_4)?,
        terrain.encode_payload()?,
    ] {
        bytes.extend_from_slice(&(part.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&part);
    }
    LiveSpatialObservation::decode_payload(&bytes, 7, "df".into(), "dfhack".into())
}
fn observation_at_next_tick() -> Result<LiveSpatialObservation> {
    observation(4)
}
fn context(observation: &LiveSpatialObservation) -> Result<OperationContext> {
    let mut state = LiveSpatialState::default();
    state.publish(observation.clone())?;
    let anchor = state
        .snapshot()
        .ok_or_else(|| DfmcpError::new(ErrorCode::InvalidRequest, "fixture snapshot"))?
        .anchor();
    Ok(OperationContext {
        session_id: SessionId::new(91_200),
        request_id: RequestId::new(1),
        anchor,
        budget: WorkBudget {
            max_bytes: 16 * 1024 * 1024,
            max_entities: 100_000,
            max_wall_millis: 60_000,
            ..WorkBudget::default()
        },
        grants: [Capability::Query, Capability::Observe]
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope::default(),
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect(),
        cancellation_requested: false,
    })
}
fn query_only(c: &OperationContext) -> OperationContext {
    let mut c = c.clone();
    c.grants.retain(|g| g.capability == Capability::Query);
    c
}

#[test]
fn archive_recovers_exact_state_and_history_with_query_only_authority() -> Result<()> {
    let f = Fixture::new()?;
    let c = f.populated()?;
    let before = fs::read(&f.path).map_err(io_error)?;
    let mut bootstrap = query_only(&c);
    bootstrap.anchor.fortress_id = FortressId::NIL;
    let mut archive = PrivateJournalFile::open_recovery::<Spatial16>(
        &f.path,
        &bootstrap,
        JournalLimits::default(),
    )?;
    assert!(archive.recovery_only());
    assert_eq!(archive.entries().len(), 2);
    assert_eq!(archive.repaired_tail_bytes(), 0);
    let latest = archive.state().snapshot().cloned();
    let entry = archive.entries()[0].clone();
    let historical = archive.state_at(entry.number, entry.record_digest, &query_only(&c))?;
    assert_eq!(
        historical.snapshot().map(|s| s.anchor()),
        Some(entry.anchor)
    );
    assert_eq!(archive.state().snapshot(), latest.as_ref());
    drop(archive);
    assert_eq!(fs::read(&f.path).map_err(io_error)?, before);
    Ok(())
}

#[test]
fn archive_never_initializes_or_repairs_missing_empty_or_incomplete_files() -> Result<()> {
    let f = Fixture::new()?;
    let c = query_only(&context(&observation(3)?)?);
    assert!(
        PrivateJournalFile::open_recovery::<Spatial16>(&f.path, &c, JournalLimits::default())
            .is_err()
    );
    assert!(!f.path.exists());
    let live = open_profile_journal::<Spatial16>(
        &f.path,
        &context(&observation(3)?)?,
        JournalLimits::default(),
        TailRecovery::Refuse,
    )?;
    drop(live);
    let header = fs::read(&f.path).map_err(io_error)?;
    // A valid header with no observations is readable at the adapter boundary.
    let archive =
        PrivateJournalFile::open_recovery::<Spatial16>(&f.path, &c, JournalLimits::default())?;
    assert!(archive.entries().is_empty());
    drop(archive);
    for bytes in [
        Vec::new(),
        header[..header.len() - 1].to_vec(),
        [header.clone(), b"DFM".to_vec()].concat(),
        [header, b"garbage".to_vec()].concat(),
    ] {
        fs::write(&f.path, &bytes).map_err(io_error)?;
        assert!(
            PrivateJournalFile::open_recovery::<Spatial16>(&f.path, &c, JournalLimits::default())
                .is_err()
        );
        assert_eq!(fs::read(&f.path).map_err(io_error)?, bytes);
    }
    Ok(())
}

#[test]
fn recovery_preserves_current_authority_time_identity_and_acquisition_bounds() -> Result<()> {
    let f = Fixture::new()?;
    let original = f.populated()?;
    let before = fs::read(&f.path).map_err(io_error)?;
    for case in 0..6 {
        let mut c = query_only(&original);
        match case {
            0 => c.grants.clear(),
            1 => c.cancellation_requested = true,
            2 => c.anchor.fortress_id = FortressId::new(original.anchor.fortress_id.get() ^ 1),
            3 => {
                c.grants[0].expires_at_tick = Some(c.anchor.tick);
                c.anchor.tick = GameTick(c.anchor.tick.0 + 100);
            }
            4 => c.budget.max_entities = 1,
            _ => c.grants[0].remaining_uses = Some(0),
        }
        assert!(
            PrivateJournalFile::open_recovery::<Spatial16>(&f.path, &c, JournalLimits::default())
                .is_err(),
            "case={case}"
        );
    }
    assert!(
        PrivateJournalFile::open_recovery::<Operations13>(
            &f.path,
            &original,
            JournalLimits::default()
        )
        .is_err()
    );
    assert_eq!(fs::read(&f.path).map_err(io_error)?, before);
    Ok(())
}

#[test]
fn recovery_descriptor_refuses_changed_appends_even_with_injected_observe_grant() -> Result<()> {
    let f = Fixture::new()?;
    let c = f.populated()?;
    let before = fs::read(&f.path).map_err(io_error)?;
    let mut archive = PrivateJournalFile::open_recovery::<Spatial16>(
        &f.path,
        &query_only(&c),
        JournalLimits::default(),
    )?;
    let anchor = archive.state().snapshot().map(|s| s.anchor());
    assert!(archive.append(observation(5)?, &c).is_err());
    assert_eq!(archive.state().snapshot().map(|s| s.anchor()), anchor);
    drop(archive);
    assert_eq!(fs::read(&f.path).map_err(io_error)?, before);
    Ok(())
}

#[test]
fn recovery_retains_exclusive_custody_and_rejects_links_and_permissive_files() -> Result<()> {
    let f = Fixture::new()?;
    let c = f.populated()?;
    let archive =
        PrivateJournalFile::open_recovery::<Spatial16>(&f.path, &c, JournalLimits::default())?;
    assert!(
        open_profile_journal::<Spatial16>(
            &f.path,
            &c,
            JournalLimits::default(),
            TailRecovery::Refuse
        )
        .is_err()
    );
    assert!(
        PrivateJournalFile::open_recovery::<Spatial16>(&f.path, &c, JournalLimits::default())
            .is_err()
    );
    drop(archive);
    let alias = f.directory.join("alias.bin");
    std::os::unix::fs::symlink(&f.path, &alias).map_err(io_error)?;
    assert!(
        PrivateJournalFile::open_recovery::<Spatial16>(&alias, &c, JournalLimits::default())
            .is_err()
    );
    fs::remove_file(&alias).map_err(io_error)?;
    fs::hard_link(&f.path, &alias).map_err(io_error)?;
    assert!(
        PrivateJournalFile::open_recovery::<Spatial16>(&f.path, &c, JournalLimits::default())
            .is_err()
    );
    fs::remove_file(&alias).map_err(io_error)?;
    fs::set_permissions(&f.path, fs::Permissions::from_mode(0o644)).map_err(io_error)?;
    assert!(
        PrivateJournalFile::open_recovery::<Spatial16>(&f.path, &c, JournalLimits::default())
            .is_err()
    );
    Ok(())
}

#[test]
fn cached_archive_access_rechecks_authority_and_fences_changed_custody() -> Result<()> {
    use std::io::Write;
    let f = Fixture::new()?;
    let c = f.populated()?;
    let mut archive =
        PrivateJournalFile::open_recovery::<Spatial16>(&f.path, &c, JournalLimits::default())?;
    let mut denied = c.clone();
    denied.grants.clear();
    assert!(archive.validate_custody(&denied).is_err());
    assert!(!archive.fenced());
    fs::OpenOptions::new()
        .append(true)
        .open(&f.path)
        .and_then(|mut f| f.write_all(b"x"))
        .map_err(io_error)?;
    assert!(matches!(archive.validate_custody(&c),Err(e)if e.code==ErrorCode::CorruptLedger));
    assert!(archive.fenced());
    Ok(())
}
