use super::super::tests::{context, observation, plan, raw_record};
use super::*;
use std::io::{Cursor, Read, Seek, Write};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct Storage {
    bytes: Arc<Mutex<Cursor<Vec<u8>>>>,
    fail_sync: Arc<Mutex<bool>>,
}
impl Read for Storage {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.bytes
            .lock()
            .map_err(|_| io::Error::other("poison"))?
            .read(out)
    }
}
impl Seek for Storage {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.bytes
            .lock()
            .map_err(|_| io::Error::other("poison"))?
            .seek(pos)
    }
}
impl Write for Storage {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes
            .lock()
            .map_err(|_| io::Error::other("poison"))?
            .write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl EffectJournalStorage for Storage {
    fn sync(&mut self) -> io::Result<()> {
        if *self
            .fail_sync
            .lock()
            .map_err(|_| io::Error::other("poison"))?
        {
            Err(io::Error::other("sync"))
        } else {
            Ok(())
        }
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("no repair"))
    }
}
struct Source {
    manifest: RunManifest,
    record: Option<RunRecord>,
    commits: usize,
    prepares: usize,
    queries: usize,
    cancels: usize,
    lose_commit: bool,
    absent: bool,
    wrong_endpoint: bool,
}
impl Source {
    fn new() -> Self {
        Self {
            manifest: RunManifest {
                generation: 41,
                df_version: "fake-df".into(),
                dfhack_version: "fake-dfhack".into(),
            },
            record: None,
            commits: 0,
            prepares: 0,
            queries: 0,
            cancels: 0,
            lose_commit: false,
            absent: false,
            wrong_endpoint: false,
        }
    }
}
impl RunSource for Source {
    fn manifest(&self) -> &RunManifest {
        &self.manifest
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        Some(SocketAddr::from((
            [127, 0, 0, 1],
            if self.wrong_endpoint { 5001 } else { 5000 },
        )))
    }
    fn observe(&mut self, _: &OperationContext) -> Result<RunObservation> {
        observation()
    }
    fn prepare(&mut self, p: &RunPlan, _: &OperationContext) -> Result<RunRecord> {
        self.prepares += 1;
        let r = RunRecord::decode(&raw_record(p, 0, 0, false, false, None))?;
        self.record = Some(r.clone());
        Ok(r)
    }
    fn commit(&mut self, p: &RunPlan, _: &OperationContext) -> Result<RunRecord> {
        self.commits += 1;
        let r = RunRecord::decode(&raw_record(p, 1, 0, true, false, Some(100)))?;
        self.record = Some(r.clone());
        if self.lose_commit {
            Err(error(ErrorCode::AdapterUnavailable, "lost reply"))
        } else {
            Ok(r)
        }
    }
    fn query(&mut self, _: &RunPlan, _: &OperationContext) -> Result<Option<RunRecord>> {
        self.queries += 1;
        Ok(if self.absent {
            None
        } else {
            self.record.clone()
        })
    }
    fn cancel(&mut self, p: &RunPlan, _: &OperationContext) -> Result<RunRecord> {
        self.cancels += 1;
        let r = RunRecord::decode(&raw_record(p, 3, 3, true, true, Some(103)))?;
        self.record = Some(r.clone());
        Ok(r)
    }
}
fn setup() -> Result<(RunJournal<Storage>, Storage, Source, OperationContext)> {
    let storage = Storage::default();
    let source = Source::new();
    let mut c = context();
    c.budget.max_bytes = 16 * 1024 * 1024;
    c.anchor.state_hash = observation()?.witness();
    let b = RunBinding::new(
        SocketAddr::from(([127, 0, 0, 1], 5000)),
        source.manifest.clone(),
    )?;
    let journal = RunJournal::open(storage.clone(), &c, RunMode::Control, Some(b), true)?;
    Ok((journal, storage, source, c))
}
fn reopen(storage: Storage, c: &OperationContext, mode: RunMode) -> Result<RunJournal<Storage>> {
    RunJournal::open(storage, c, mode, None, false)
}
#[test]
fn prepare_commit_restart_and_terminal_receipt_round_trip() -> Result<()> {
    let (mut j, storage, mut source, c) = setup()?;
    let p = plan()?;
    assert_eq!(
        j.prepare(&mut source, p.clone(), &c)?.state(),
        RunState::Prepared
    );
    assert_eq!(
        j.commit(&mut source, p.key(), p.digest(), &c)?.state(),
        RunState::Tracking
    );
    assert_eq!(source.commits, 1);
    assert!(j.commit(&mut source, p.key(), p.digest(), &c).is_err());
    assert_eq!(source.commits, 1);
    source.record = Some(RunRecord::decode(&raw_record(
        &p,
        3,
        1,
        true,
        true,
        Some(110),
    ))?);
    let mut recovered = reopen(storage.clone(), &c, RunMode::Recover)?;
    let terminal = recovered.reconcile(&mut source, p.key(), p.digest(), &c)?;
    assert!(terminal.native().is_some_and(RunRecord::pause_verified));
    let mut offline = reopen(storage, &c, RunMode::Offline)?;
    assert_eq!(offline.records(&c)?, vec![terminal]);
    Ok(())
}
#[test]
fn lost_commit_and_absent_query_never_enable_redispatch() -> Result<()> {
    let (mut j, storage, mut source, c) = setup()?;
    let p = plan()?;
    source.lose_commit = true;
    j.prepare(&mut source, p.clone(), &c)?;
    assert_eq!(
        j.commit(&mut source, p.key(), p.digest(), &c)
            .err()
            .map(|e| e.code),
        Some(ErrorCode::EffectIndeterminate)
    );
    let mut recovered = reopen(storage, &c, RunMode::Control)?;
    assert!(
        recovered
            .commit(&mut source, p.key(), p.digest(), &c)
            .is_err()
    );
    assert_eq!(source.commits, 1);
    source.absent = true;
    assert!(
        recovered
            .reconcile(&mut source, p.key(), p.digest(), &c)
            .is_err()
    );
    assert!(recovered.records(&c)?[0].unresolved());
    let other = RunPlan::new("other", p.spec(), observation()?)?;
    assert!(recovered.prepare(&mut source, other, &c).is_err());
    assert_eq!(source.prepares, 1);
    Ok(())
}
#[test]
fn crash_after_marker_before_dispatch_stays_uncertain_even_when_native_is_prepared() -> Result<()> {
    let (mut j, storage, mut source, c) = setup()?;
    let p = plan()?;
    let mut r = j.prepare(&mut source, p.clone(), &c)?;
    r.state = RunState::DispatchStarted;
    j.append(r, &mut Allowance::new(&c)?)?;
    let mut recovered = reopen(storage, &c, RunMode::Control)?;
    let r = recovered.reconcile(&mut source, p.key(), p.digest(), &c)?;
    assert_eq!(r.state(), RunState::Tracking);
    assert_eq!(r.native().map(RunRecord::phase), Some(RunPhase::Prepared));
    assert!(
        recovered
            .commit(&mut source, p.key(), p.digest(), &c)
            .is_err()
    );
    assert_eq!(source.commits, 0);
    Ok(())
}
#[test]
fn sync_failure_before_dispatch_fences_without_native_commit() -> Result<()> {
    let (mut j, storage, mut source, c) = setup()?;
    let p = plan()?;
    j.prepare(&mut source, p.clone(), &c)?;
    *storage.fail_sync.lock().map_err(|_| corrupt("test lock"))? = true;
    assert!(j.commit(&mut source, p.key(), p.digest(), &c).is_err());
    assert_eq!(source.commits, 0);
    assert!(j.fenced());
    *storage.fail_sync.lock().map_err(|_| corrupt("test lock"))? = false;
    let mut recovered = reopen(storage, &c, RunMode::Control)?;
    assert!(recovered.records(&c)?[0].unresolved());
    assert!(
        recovered
            .commit(&mut source, p.key(), p.digest(), &c)
            .is_err()
    );
    Ok(())
}
#[test]
fn cancellation_before_and_after_dispatch_are_distinct() -> Result<()> {
    let (mut j, _, mut source, c) = setup()?;
    let p = plan()?;
    j.prepare(&mut source, p.clone(), &c)?;
    let r = j.cancel(None, p.key(), p.digest(), &c)?;
    assert_eq!(r.state(), RunState::CancelledBeforeDispatch);
    assert!(r.native().is_none());
    j.commit(&mut source, p.key(), p.digest(), &c)?;
    assert_eq!(source.commits, 0);
    let next = RunPlan::new("next", p.spec(), observation()?)?;
    j.prepare(&mut source, next.clone(), &c)?;
    j.commit(&mut source, next.key(), next.digest(), &c)?;
    let r = j.cancel(Some(&mut source), next.key(), next.digest(), &c)?;
    assert!(r.native().is_some_and(RunRecord::pause_verified));
    assert_eq!(source.cancels, 1);
    Ok(())
}
#[test]
fn offline_and_recover_modes_cannot_gain_control_from_injected_grants() -> Result<()> {
    let (mut j, storage, mut source, c) = setup()?;
    let p = plan()?;
    j.prepare(&mut source, p.clone(), &c)?;
    for mode in [RunMode::Offline, RunMode::Recover] {
        let mut recovered = reopen(storage.clone(), &c, mode)?;
        assert!(
            recovered
                .commit(&mut source, p.key(), p.digest(), &c)
                .is_err()
        );
        assert!(
            recovered
                .cancel(Some(&mut source), p.key(), p.digest(), &c)
                .is_err()
        );
    }
    assert_eq!(source.commits, 0);
    assert_eq!(source.cancels, 0);
    Ok(())
}
#[test]
fn same_length_corruption_and_torn_tail_never_repair() -> Result<()> {
    let (mut j, storage, mut source, c) = setup()?;
    let p = plan()?;
    j.prepare(&mut source, p, &c)?;
    let original = storage
        .bytes
        .lock()
        .map_err(|_| corrupt("test lock"))?
        .get_ref()
        .clone();
    let mut changed = original.clone();
    let n = changed.len();
    changed[n - 1] ^= 1;
    *storage.bytes.lock().map_err(|_| corrupt("test lock"))? = Cursor::new(changed.clone());
    assert!(j.records(&c).is_err());
    assert!(j.fenced());
    assert!(reopen(storage.clone(), &c, RunMode::Offline).is_err());
    assert_eq!(
        *storage
            .bytes
            .lock()
            .map_err(|_| corrupt("test lock"))?
            .get_ref(),
        changed
    );
    for cut in [1, 2, 31, 40] {
        *storage.bytes.lock().map_err(|_| corrupt("test lock"))? =
            Cursor::new(original[..original.len() - cut].to_vec());
        assert!(reopen(storage.clone(), &c, RunMode::Control).is_err());
    }
    Ok(())
}
#[test]
fn source_endpoint_software_and_budget_fences_precede_native_preparation() -> Result<()> {
    let (mut j, _, mut source, c) = setup()?;
    let p = plan()?;
    source.wrong_endpoint = true;
    assert!(j.prepare(&mut source, p.clone(), &c).is_err());
    source.wrong_endpoint = false;
    source.manifest.generation = 42;
    assert!(j.prepare(&mut source, p.clone(), &c).is_err());
    source.manifest.generation = 41;
    let mut small = c.clone();
    small.budget.max_bytes = 10000;
    assert!(j.prepare(&mut source, p.clone(), &small).is_err());
    let mut other = c.clone();
    other.session_id = SessionId::new(2);
    assert!(j.prepare(&mut source, p, &other).is_err());
    assert_eq!(source.prepares, 0);
    Ok(())
}
#[test]
fn illegal_durable_and_native_regressions_are_rejected() -> Result<()> {
    let p = plan()?;
    let old = DurableRun {
        plan: p.clone(),
        state: RunState::DispatchStarted,
        native: Some(RunRecord::decode(&raw_record(
            &p, 0, 0, false, false, None,
        ))?),
    };
    let mut next = old.clone();
    next.state = RunState::Prepared;
    assert!(transition(Some(&old), &next).is_err());
    let old = DurableRun {
        plan: p.clone(),
        state: RunState::Tracking,
        native: Some(RunRecord::decode(&raw_record(
            &p,
            2,
            3,
            true,
            false,
            Some(100),
        ))?),
    };
    next.state = RunState::Tracking;
    next.native = Some(RunRecord::decode(&raw_record(
        &p,
        1,
        0,
        true,
        false,
        Some(100),
    ))?);
    assert!(transition(Some(&old), &next).is_err());
    Ok(())
}
