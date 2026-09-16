//! Teardown must make the existing recovery paths usable without discarding
//! persisted evidence or consuming global capacity after repeated reopen cycles.
use super::*;

#[test]
fn archive_sessions_release_read_only_locks_without_loading_or_modifying_watches() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let writer = register(Some(&files), true, 3, &[])?;
    watch(&writer)?;
    let (limits, budget) = {
        let handle = resolve(writer.handle())?;
        let session = lock(&handle)?;
        (session.limits, session.budget)
    };
    successful(request_close(&writer, false)?);
    let observations = fs::read(&files.observations).map_err(io_error)?;
    let watches = fs::read(&files.watches).map_err(io_error)?;
    for _ in 0..4 {
        let id = next_id()?;
        let session = archive::open(id, Slot::reserve()?, &files.observations, limits, budget,
            &[Capability::Query])?;
        lock(&SESSIONS)?.insert(id, Arc::new(Mutex::new(session)));
        let reader = Registered { id, calls:Arc::new(AtomicUsize::new(0)), drops:Arc::new(AtomicUsize::new(0)) };
        let stale = resolve(reader.handle())?;
        let c = lock(&stale)?.context()?;
        assert!(PrivateJournalFile::open_recovery::<Spatial18>(&files.observations, &c, JournalLimits::default()).is_err());
        let result = successful(decode(&fortress_query(reader.handle(), Some("production".into()), None))?);
        assert_eq!(result["archive_only"], true);
        let closed = successful(request_close(&reader, false)?);
        assert_eq!(closed["process_local_resources"]["watch_journal_present"], false);
        assert_eq!(closed["process_local_resources"]["watch_handles_released"], 0);
        // Keep the old session Arc alive across reacquisition of the same file.
        let reopened = PrivateJournalFile::open_recovery::<Spatial18>(&files.observations, &c, JournalLimits::default())?;
        assert!(!reopened.entries().is_empty());
        assert!(matches!(lock(&stale)?.context(), Err(e) if e.code == ErrorCode::SessionNotFound));
        drop(reopened);
        assert_eq!(fs::read(&files.observations).map_err(io_error)?, observations);
        assert_eq!(fs::read(&files.watches).map_err(io_error)?, watches);
    }
    Ok(())
}

#[test]
fn closing_one_session_does_not_release_another_sessions_watches_or_baselines() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let first = register(None, false, 3, &[])?;
    let second = register(None, false, 3, &[])?;
    watch(&first)?; baseline(&first)?;
    watch(&second)?; baseline(&second)?;
    let watches = successful(ask(&second, json!({"kind":"watches"}))?)["records"].clone();
    let baselines = successful(ask(&second, json!({"kind":"baselines"}))?)["baselines"].clone();
    successful(request_close(&first, true)?);
    assert_eq!(successful(ask(&second, json!({"kind":"watches"}))?)["records"], watches);
    assert_eq!(successful(ask(&second, json!({"kind":"baselines"}))?)["baselines"], baselines);
    assert_eq!(second.drops.load(Ordering::SeqCst), 0);
    assert_eq!(request_close(&second, false)?["error"]["code"], "conflict");
    successful(request_close(&second, true)?);
    Ok(())
}

#[test]
fn local_watches_alone_require_consent_and_repeated_sessions_do_not_fill_global_stores() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    // More than the shared 128-entry watch/baseline limit; every iteration must
    // release the original records rather than merely hide its session handle.
    for _ in 0..130 {
        let session = register(None, false, 3, &[])?;
        watch(&session)?;
        assert_eq!(request_close(&session, false)?["error"]["code"], "conflict");
        baseline(&session)?;
        let result = successful(request_close(&session, true)?);
        assert_eq!(result["process_local_resources"]["process_local_baselines_discarded"], 1);
        assert_eq!(result["process_local_resources"]["process_local_watch_records_discarded"], 1);
        assert_eq!(session.calls.load(Ordering::SeqCst), 0);
        assert_eq!(session.drops.load(Ordering::SeqCst), 1);
    }
    assert_eq!(SLOTS.load(Ordering::SeqCst), 0);
    Ok(())
}
