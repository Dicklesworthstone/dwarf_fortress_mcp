//! Actual handler tests use a private on-disk journal and no bridge connection.
//! No environment mutation, native service, or network mock is involved.

use super::*;
use std::fs;
use std::os::unix::fs::DirBuilderExt;

static SERIAL: Mutex<()> = Mutex::new(());
static FILE_ID: AtomicUsize = AtomicUsize::new(0);

fn io_error(_: std::io::Error) -> DfmcpError { err(ErrorCode::CorruptLedger, "recovery test filesystem error") }
fn parse(raw: String) -> Result<Value> {
    serde_json::from_str(&raw).map_err(|_| err(ErrorCode::InternalInvariantViolation, "invalid handler JSON"))
}
fn context() -> OperationContext {
    OperationContext { session_id: SessionId::new(100), request_id: RequestId::new(1),
        anchor: coordinator_anchor(), budget: WorkBudget::default(), cancellation_requested: false,
        grants: vec![CapabilityGrant { capability: Capability::ControlClock, max_risk: RiskTier::Reversible,
            scope: CapabilityScope::default(), expires_at_tick: None, remaining_uses: None }] }
}
struct Fixture { directory: PathBuf, path: PathBuf }
impl Fixture {
    fn new() -> Result<Self> {
        let serial = FILE_ID.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().canonicalize().map_err(io_error)?
            .join(format!("dfmcp-mcp-control-recovery-{}-{serial}", std::process::id()));
        fs::DirBuilder::new().mode(0o700).create(&directory).map_err(io_error)?;
        let fixture = Self { path: directory.join("effects.bin"), directory };
        let mut journal = open_private_control_journal(&fixture.path, &context(), 7, EffectTailRecovery::Refuse)?;
        for key in ["started", "prepared", "applied"] {
            let plan = Digest32::of_bytes(key.as_bytes());
            journal.record_prepared(key.into(), plan, true, 10, 7, [1;16], &context())?;
            if key != "prepared" { journal.begin_commit(key, plan, 7, &context())?; }
            if key == "applied" {
                journal.record_reconciliation(key, plan, 7, true, true, true, 11,
                    Some(Digest32::of_bytes(b"receipt")), &context())?;
            }
        }
        drop(journal);
        Ok(fixture)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // Only paths underneath this test's exclusively created directory.
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_dir(&self.directory);
    }
}

#[test]
fn offline_recovery_tools_discover_evidence_without_a_bridge_or_mutations() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let fixture = Fixture::new()?;
    let original = fs::read(&fixture.path).map_err(io_error)?;
    let id = next_id()?;
    let session = configured_session(id, &fixture.path, EffectTailRecovery::Refuse,
        WorkBudget::default(), true, Slot::reserve()?)?;
    assert!(session.connection.is_none());
    assert!(session.journal.read_only());
    assert_eq!(session.grants.len(), 1);
    assert_eq!(session.grants[0].capability, Capability::Query);
    let handle = Arc::new(Mutex::new(session));
    lock(&SESSIONS)?.insert(id, handle.clone());
    let result = (|| -> Result<()> {
        let first = parse(fortress_query(Some(id.to_string()), None, Some(1), None, None, None))?;
        assert_eq!(first["result"]["total_effects"], 3);
        assert_eq!(first["result"]["effects"][0]["idempotency_key"], "applied");
        assert_eq!(first["result"]["reconciliation_required_effects"], 1);
        assert_eq!(first["result"]["current_freshness_proven"], false);
        assert_eq!(first["agent_turn"]["briefing"]["development_mutation_enabled"], false);
        let token = first["result"]["continuation"].as_str()
            .ok_or_else(||err(ErrorCode::InternalInvariantViolation, "missing continuation"))?.to_owned();
        let second = parse(fortress_query(Some(id.to_string()), None, Some(1), Some(token), None, None))?;
        assert_eq!(second["result"]["effects"][0]["idempotency_key"], "prepared");
        let explanation = parse(fortress_explain(Some(id.to_string()), "started".into(),
            Digest32::of_bytes(b"started").to_string()))?;
        assert_eq!(explanation["result"]["effect"]["state"], "commit_started");
        assert_eq!(explanation["result"]["reconciliation_performed"], false);
        assert_eq!(explanation["result"]["commit_permitted"], false);
        assert_eq!(explanation["result"]["effect"]["safe_to_retry_same_effect"], false);
        let planned = parse(fortress_plan(Some(id.to_string()), "prepared".into(),
            Digest32::of_bytes(b"prepared").to_string(), true, 10))?;
        assert_eq!(planned["result"]["error"]["code"], ErrorCode::CapabilityDenied.as_str());
        let committed = parse(fortress_commit(Some(id.to_string()), "prepared".into(),
            Digest32::of_bytes(b"prepared").to_string(), "01".repeat(16)))?;
        assert_eq!(committed["result"]["error"]["code"], ErrorCode::CapabilityDenied.as_str());
        let waited = parse(fortress_wait(Some(id.to_string()), vec!["started".into()], None, None, None))?;
        assert_eq!(waited["result"]["error"]["code"], ErrorCode::CapabilityDenied.as_str());
        assert_eq!(waited["result"]["error"]["mutation_dispatched"], false);
        let doctor = parse(fortress_doctor(Some(id.to_string())))?;
        assert_eq!(doctor["result"]["recovery_only"], true);
        assert_eq!(doctor["result"]["bridge_connection_present"], false);
        assert!(doctor["result"]["bridge_generation"].is_null());
        assert!(lock(&handle)?.connection.is_none());
        Ok(())
    })();
    lock(&SESSIONS)?.remove(&id);
    drop(handle);
    assert_eq!(fs::read(&fixture.path).map_err(io_error)?, original);
    result?;
    // Reopen under a fresh process-scoped session identity; no transcript/key is needed.
    let mut reopened = configured_session(next_id()?, &fixture.path, EffectTailRecovery::Refuse,
        WorkBudget::default(), true, Slot::reserve()?)?;
    let context = reopened.context()?;
    assert_eq!(reopened.journal.records(&context)?.len(), 3);
    assert_eq!(reopened.journal.lookup("started").map(|r| r.state), Some(DurablePauseState::CommitStarted));
    Ok(())
}

#[test]
fn recovery_session_rejects_repair_and_missing_journals_before_any_connection() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let fixture = Fixture::new()?;
    let original = fs::read(&fixture.path).map_err(io_error)?;
    assert!(matches!(configured_session(next_id()?, &fixture.path, EffectTailRecovery::TruncateIncomplete,
        WorkBudget::default(), true, Slot::reserve()?), Err(e) if e.code == ErrorCode::CapabilityDenied));
    let missing = fixture.directory.join("missing.bin");
    assert!(configured_session(next_id()?, &missing, EffectTailRecovery::Refuse,
        WorkBudget::default(), true, Slot::reserve()?).is_err());
    assert!(!missing.exists());
    assert_eq!(fs::read(&fixture.path).map_err(io_error)?, original);
    Ok(())
}

#[test]
fn recovery_session_cannot_dispatch_even_if_a_caller_injects_clock_grants() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let fixture = Fixture::new()?;
    let original = fs::read(&fixture.path).map_err(io_error)?;
    let mut session = configured_session(next_id()?, &fixture.path, EffectTailRecovery::Refuse,
        WorkBudget::default(), true, Slot::reserve()?)?;
    session.grants = context().grants;
    assert!(matches!(session.live(), Err(e) if e.code == ErrorCode::CapabilityDenied));
    let context = session.context()?;
    assert!(matches!(session.journal.begin_commit("prepared", Digest32::of_bytes(b"prepared"), 7, &context),
        Err(e) if e.code == ErrorCode::CapabilityDenied));
    assert!(matches!(reconciliation::wait(&mut session, context, vec!["started".into()], None, None, None),
        Err(e) if e.code == ErrorCode::CapabilityDenied));
    assert!(session.connection.is_none());
    assert_eq!(fs::read(&fixture.path).map_err(io_error)?, original);
    Ok(())
}
