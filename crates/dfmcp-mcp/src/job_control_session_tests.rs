use super::*;
use std::cell::{Cell, RefCell};
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::rc::Rc;

use dfmcp_adapter::job_suspension::rpc::JobControlManifest;
use dfmcp_adapter::job_suspension::{SuspensionEffect, SuspensionState};
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, GameTick, ObservationCursor, RequestId, StateAnchor,
    WorkBudget,
};

fn observation() -> Result<JobObservation> {
    let text =
        include_str!("../../dfmcp-adapter/tests/fixtures/job_suspension_observation_v1_9.hex");
    let bytes: Result<Vec<u8>> = text
        .trim()
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).map_err(|_| stale())?;
            u8::from_str_radix(text, 16).map_err(|_| stale())
        })
        .collect();
    JobObservation::decode(&bytes?)
}

fn context() -> Result<OperationContext> {
    let o = observation()?;
    Ok(OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(2),
        anchor: StateAnchor {
            fortress_id: job_fortress_id(&o),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(o.tick()),
            state_hash: o.witness(),
        },
        budget: WorkBudget::default(),
        cancellation_requested: false,
        grants: [Capability::Query, Capability::ConfigureProduction]
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope::default(),
                max_risk: RiskTier::Reversible,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect(),
    })
}

#[derive(Clone, Default)]
struct Memory {
    data: Rc<RefCell<Cursor<Vec<u8>>>>,
    syncs: Rc<Cell<usize>>,
    fail_sync: Rc<Cell<Option<usize>>>,
    trace: Rc<RefCell<Vec<&'static str>>>,
}
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.data.borrow_mut().read(out)
    }
}
impl Seek for Memory {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.data.borrow_mut().seek(from)
    }
}
impl Write for Memory {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.trace.borrow_mut().push("write");
        self.data.borrow_mut().write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        let count = self.syncs.get() + 1;
        self.syncs.set(count);
        if self.fail_sync.get() == Some(count) {
            return Err(io::Error::other("injected sync failure"));
        }
        self.trace.borrow_mut().push("sync");
        Ok(())
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("never erase evidence"))
    }
}

#[derive(Default)]
struct Calls {
    reads: usize,
    prepares: usize,
    commits: usize,
    queries: usize,
    fenced: bool,
    fail_read: bool,
    fail_commit: bool,
    query_state: Option<SuspensionState>,
}
struct Source {
    calls: Rc<RefCell<Calls>>,
    manifest: JobControlManifest,
    trace: Rc<RefCell<Vec<&'static str>>>,
}

fn text(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&(value.len() as u16).to_be_bytes());
    out.extend_from_slice(value.as_bytes());
}
fn effect(plan: &SuspensionPlan, state: SuspensionState) -> Result<SuspensionEffect> {
    let o = plan.observation();
    let mut bytes = b"DFMJSE19".to_vec();
    for value in [o.generation(), o.sequence(), o.tick()] {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    bytes.extend_from_slice(&o.job_id().to_be_bytes());
    bytes.push(u8::from(plan.desired()));
    bytes.extend_from_slice(o.witness().as_bytes());
    bytes.extend_from_slice(plan.digest().as_bytes());
    bytes.extend_from_slice(plan.prepare_token());
    let known = matches!(
        state,
        SuspensionState::Applied | SuspensionState::NotApplied
    );
    let after = known
        && if state == SuspensionState::Applied {
            plan.desired()
        } else {
            !plan.desired()
        };
    bytes.extend_from_slice(&[state as u8, u8::from(known), u8::from(after)]);
    bytes.extend_from_slice(&(if known { o.tick() } else { 0 }).to_be_bytes());
    let mut changed = o.canonical_bytes().to_vec();
    changed[16..24].copy_from_slice(&(o.sequence() + 1).to_be_bytes());
    changed[84] = (changed[84] & !1) | u8::from(after);
    let after_witness = if known {
        Digest32::of_bytes(&changed)
    } else {
        Digest32::ZERO
    };
    bytes.extend_from_slice(after_witness.as_bytes());
    let receipt = if state.terminal() {
        let mut proof = b"dfmcp-job-suspension-receipt/1\0".to_vec();
        proof.extend_from_slice(&o.generation().to_be_bytes());
        text(&mut proof, plan.key());
        proof.extend_from_slice(&bytes[69..160]);
        Digest32::of_bytes(&proof)
    } else {
        Digest32::ZERO
    };
    bytes.extend_from_slice(receipt.as_bytes());
    text(&mut bytes, plan.key());
    SuspensionEffect::decode(&bytes, plan)
}
impl JobSuspensionSource for Source {
    fn manifest(&self) -> &JobControlManifest {
        &self.manifest
    }
    fn fence(&mut self) {
        self.calls.borrow_mut().fenced = true;
    }
    fn prepare(&mut self, plan: &SuspensionPlan, _: Duration) -> Result<SuspensionEffect> {
        self.calls.borrow_mut().prepares += 1;
        effect(plan, SuspensionState::Prepared)
    }
    fn commit(
        &mut self,
        plan: &SuspensionPlan,
        _: &SuspensionEffect,
        _: Duration,
    ) -> Result<SuspensionEffect> {
        self.calls.borrow_mut().commits += 1;
        self.trace.borrow_mut().push("commit");
        if self.calls.borrow().fail_commit {
            return Err(error(ErrorCode::AdapterUnavailable, "lost reply"));
        }
        effect(plan, SuspensionState::Applied)
    }
    fn query(&mut self, plan: &SuspensionPlan, _: Duration) -> Result<Option<SuspensionEffect>> {
        self.calls.borrow_mut().queries += 1;
        self.calls
            .borrow()
            .query_state
            .map(|state| effect(plan, state))
            .transpose()
    }
}
impl SelectedJobSource for Source {
    fn read_job(&mut self, _: u32, _: Duration) -> Result<JobObservation> {
        self.calls.borrow_mut().reads += 1;
        if self.calls.borrow().fail_read {
            return Err(error(ErrorCode::AdapterUnavailable, "failed refresh"));
        }
        observation()
    }
}

type Session = JobControlSession<Memory, Source>;
fn source(memory: &Memory, calls: Rc<RefCell<Calls>>) -> Result<Source> {
    Ok(Source {
        calls,
        manifest: JobControlManifest {
            generation: observation()?.generation(),
            df_version: "df".into(),
            dfhack_version: "dfhack".into(),
        },
        trace: memory.trace.clone(),
    })
}
fn setup() -> Result<(Session, Memory, Rc<RefCell<Calls>>)> {
    let memory = Memory::default();
    let calls = Rc::new(RefCell::new(Calls {
        query_state: Some(SuspensionState::Applied),
        ..Calls::default()
    }));
    let c = context()?;
    let journal = JobControlJournal::open(memory.clone(), &c, true)?;
    let session = JobControlSession::new(journal, Some(source(&memory, calls.clone())?), &c)?;
    Ok((session, memory, calls))
}
fn prepare(session: &mut Session, key: &str, c: &OperationContext) -> Result<DurableJobRecord> {
    let o = observation()?;
    session.observe(o.job_id(), c)?;
    session.plan(key, o.job_id(), true, o.witness(), c)
}

#[test]
fn sealed_loop_syncs_before_setter_and_replays_without_native_calls() -> Result<()> {
    let (mut session, memory, calls) = setup()?;
    let c = context()?;
    let p = prepare(&mut session, "job-001", &c)?;
    memory.trace.borrow_mut().clear();
    let done = session.commit(
        p.plan().key(),
        p.plan().digest(),
        p.plan().observation().witness(),
        &c,
    )?;
    assert_eq!(done.state(), DurableJobState::Applied);
    assert_eq!(
        &*memory.trace.borrow(),
        &["write", "sync", "commit", "write", "sync"]
    );
    assert_eq!(
        session.commit(
            p.plan().key(),
            p.plan().digest(),
            p.plan().observation().witness(),
            &c
        )?,
        done
    );
    assert!(session.selected(&c)?.is_none());
    assert_eq!(calls.borrow().commits, 1);
    Ok(())
}

#[test]
fn plan_requires_selection_and_rejects_forged_intent_and_commit_seals() -> Result<()> {
    let (mut session, _, calls) = setup()?;
    let c = context()?;
    let o = observation()?;
    assert!(
        session
            .plan("job-001", o.job_id(), true, o.witness(), &c)
            .is_err()
    );
    let p = prepare(&mut session, "job-001", &c)?;
    assert!(
        session
            .plan("job-001", o.job_id(), false, o.witness(), &c)
            .is_err()
    );
    assert!(
        session
            .commit("job-001", Digest32::ZERO, o.witness(), &c)
            .is_err()
    );
    assert!(
        session
            .commit("job-001", p.plan().digest(), Digest32::ZERO, &c)
            .is_err()
    );
    assert_eq!(calls.borrow().prepares, 1);
    assert_eq!(calls.borrow().commits, 0);
    Ok(())
}

#[test]
fn failed_refresh_discards_previous_selection() -> Result<()> {
    let (mut session, _, calls) = setup()?;
    let c = context()?;
    let o = observation()?;
    session.observe(o.job_id(), &c)?;
    calls.borrow_mut().fail_read = true;
    assert!(session.observe(o.job_id(), &c).is_err());
    assert!(session.selected(&c)?.is_none());
    assert!(
        session
            .plan("new", o.job_id(), true, o.witness(), &c)
            .is_err()
    );
    assert_eq!(calls.borrow().prepares, 0);
    assert!(calls.borrow().fenced);
    Ok(())
}

#[test]
fn authority_session_anchor_and_action_budget_are_rechecked() -> Result<()> {
    let (mut session, _, calls) = setup()?;
    let c = context()?;
    let p = prepare(&mut session, "job-001", &c)?;
    let mut denied = c.clone();
    denied.grants.retain(|g| g.capability == Capability::Query);
    assert!(
        session
            .commit(
                p.plan().key(),
                p.plan().digest(),
                p.plan().observation().witness(),
                &denied
            )
            .is_err()
    );
    denied = c.clone();
    denied.session_id = SessionId::new(99);
    assert!(session.summary(&denied).is_err());
    denied = c.clone();
    denied.anchor.state_hash = Digest32::ZERO;
    assert!(
        session
            .commit(
                p.plan().key(),
                p.plan().digest(),
                p.plan().observation().witness(),
                &denied
            )
            .is_err()
    );
    denied = c.clone();
    denied.budget.max_actions = 0;
    assert!(
        session
            .cancel(p.plan().key(), p.plan().digest(), &denied)
            .is_err()
    );
    assert_eq!(calls.borrow().commits, 0);
    Ok(())
}

#[test]
fn cancellation_and_polling_never_dispatch_or_consume_preparations() -> Result<()> {
    let (mut session, _, calls) = setup()?;
    let c = context()?;
    let p = prepare(&mut session, "job-001", &c)?;
    assert_eq!(session.reconcile(p.plan().key(), p.plan().digest(), &c)?, p);
    let cancelled = session.cancel(p.plan().key(), p.plan().digest(), &c)?;
    assert_eq!(cancelled.state(), DurableJobState::CancelledBeforeDispatch);
    assert_eq!(
        session.commit(
            p.plan().key(),
            p.plan().digest(),
            p.plan().observation().witness(),
            &c
        )?,
        cancelled
    );
    assert_eq!(calls.borrow().commits, 0);
    assert_eq!(calls.borrow().queries, 0);
    Ok(())
}

#[test]
fn lost_reply_blocks_same_and_different_keys_until_query_only_recovery() -> Result<()> {
    let (mut session, memory, calls) = setup()?;
    let c = context()?;
    let p = prepare(&mut session, "job-001", &c)?;
    calls.borrow_mut().fail_commit = true;
    let unknown = session.commit(
        p.plan().key(),
        p.plan().digest(),
        p.plan().observation().witness(),
        &c,
    )?;
    assert_eq!(unknown.state(), DurableJobState::Indeterminate);
    let o = observation()?;
    session.observe(o.job_id(), &c)?;
    assert!(
        session
            .commit(p.plan().key(), p.plan().digest(), o.witness(), &c)
            .is_err()
    );
    assert!(
        session
            .plan("different-key", o.job_id(), true, o.witness(), &c)
            .is_err()
    );
    assert_eq!(calls.borrow().commits, 1);
    assert_eq!(calls.borrow().prepares, 1);
    drop(session);
    let mut query = c.clone();
    query.session_id = SessionId::new(3);
    query.grants.retain(|g| g.capability == Capability::Query);
    let journal = JobControlJournal::open_for_reconciliation(memory.clone(), &query)?;
    let mut recovered =
        JobControlSession::new(journal, Some(source(&memory, calls.clone())?), &query)?;
    let done = recovered.reconcile(p.plan().key(), p.plan().digest(), &query)?;
    assert_eq!(done.state(), DurableJobState::Applied);
    assert_eq!(calls.borrow().queries, 1);
    assert_eq!(calls.borrow().commits, 1);
    assert!(
        recovered
            .commit(p.plan().key(), p.plan().digest(), o.witness(), &query)
            .is_err()
    );
    Ok(())
}

#[test]
fn missing_native_receipt_remains_unknown_and_offline_recovery_cannot_dispatch() -> Result<()> {
    let (mut session, memory, calls) = setup()?;
    let c = context()?;
    let p = prepare(&mut session, "job-001", &c)?;
    calls.borrow_mut().fail_commit = true;
    calls.borrow_mut().query_state = None;
    session.commit(
        p.plan().key(),
        p.plan().digest(),
        p.plan().observation().witness(),
        &c,
    )?;
    assert_eq!(
        session
            .reconcile(p.plan().key(), p.plan().digest(), &c)?
            .state(),
        DurableJobState::Indeterminate
    );
    drop(session);
    let journal = JobControlJournal::open_read_only(memory, &c)?;
    let mut offline = JobControlSession::<_, Source>::new(journal, None, &c)?;
    assert_eq!(offline.summary(&c)?.unresolved, 1);
    assert!(
        offline
            .reconcile(p.plan().key(), p.plan().digest(), &c)
            .is_err()
    );
    assert!(
        offline
            .commit(
                p.plan().key(),
                p.plan().digest(),
                p.plan().observation().witness(),
                &c
            )
            .is_err()
    );
    assert_eq!(calls.borrow().commits, 1);
    Ok(())
}

#[test]
fn failed_dispatch_sync_is_reported_conservatively_and_never_calls_setter() -> Result<()> {
    let (mut session, memory, calls) = setup()?;
    let c = context()?;
    let p = prepare(&mut session, "job-001", &c)?;
    memory.fail_sync.set(Some(memory.syncs.get() + 1));
    let failure = session.commit(
        p.plan().key(),
        p.plan().digest(),
        p.plan().observation().witness(),
        &c,
    );
    assert!(matches!(failure, Err(e) if e.code == ErrorCode::EffectIndeterminate));
    assert_eq!(calls.borrow().commits, 0);
    Ok(())
}
