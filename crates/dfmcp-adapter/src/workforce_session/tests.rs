use super::*;
use crate::workforce_control::rpc::WorkforceManifest;
use crate::workforce_control::{AssignmentEffect, MAX_EFFECT};
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, ObservationCursor, RequestId, StateAnchor, WorkBudget,
};
use std::cell::RefCell;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::rc::Rc;

fn unhex(value: &str) -> Result<Vec<u8>> {
    value
        .trim()
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let value = std::str::from_utf8(pair).map_err(|_| exhausted())?;
            u8::from_str_radix(value, 16).map_err(|_| exhausted())
        })
        .collect()
}
fn capture() -> Result<WorkforceCapture> {
    WorkforceCapture::decode(&unhex(include_str!(
        "../../../../tests/native/workforce/vectors/capture.hex"
    ))?)
}
fn context() -> Result<OperationContext> {
    let capture = capture()?;
    Ok(OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: StateAnchor {
            fortress_id: capture.fortress_id(),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(0),
            state_hash: Digest32::ZERO,
        },
        budget: WorkBudget {
            max_wall_millis: 10000,
            max_bytes: 128 * 1024 * 1024,
            max_output_tokens: 65536,
            max_entities: 8192,
            max_actions: 1,
            max_game_ticks: 0,
        },
        grants: [
            Capability::Query,
            Capability::Plan,
            Capability::ConfigureLabor,
        ]
        .into_iter()
        .map(|capability| CapabilityGrant {
            capability,
            scope: CapabilityScope {
                fortress_id: Some(capture.fortress_id()),
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
#[derive(Default)]
struct Memory {
    file: Cursor<Vec<u8>>,
    synced: Vec<u8>,
    fail_sync: bool,
    fail_write: bool,
}
#[derive(Clone, Default)]
struct Storage(Rc<RefCell<Memory>>);
impl Read for Storage {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.0.borrow_mut().file.read(out)
    }
}
impl Seek for Storage {
    fn seek(&mut self, at: SeekFrom) -> io::Result<u64> {
        self.0.borrow_mut().file.seek(at)
    }
}
impl Write for Storage {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut m = self.0.borrow_mut();
        if m.fail_write {
            return Err(io::Error::other("injected write failure"));
        }
        m.file.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl EffectJournalStorage for Storage {
    fn sync(&mut self) -> io::Result<()> {
        let mut m = self.0.borrow_mut();
        if m.fail_sync {
            return Err(io::Error::other("injected sync failure"));
        }
        m.synced = m.file.get_ref().clone();
        Ok(())
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("no repair"))
    }
}
fn effect(plan: &AssignmentPlan, phase: AssignmentPhase) -> Result<AssignmentEffect> {
    if phase == AssignmentPhase::Applied {
        return AssignmentEffect::decode(
            &unhex(include_str!(
                "../../../../tests/native/workforce/vectors/applied.hex"
            ))?,
            plan,
        );
    }
    let mut bytes = b"DFMWE017".to_vec();
    bytes.extend_from_slice(&(plan.key().len() as u16).to_be_bytes());
    bytes.extend_from_slice(plan.key().as_bytes());
    bytes.extend_from_slice(plan.digest().as_bytes());
    bytes.extend_from_slice(plan.token());
    bytes.extend_from_slice(&plan.spec().detail().to_be_bytes());
    bytes.push(u8::from(plan.spec().assigned()));
    bytes.extend_from_slice(plan.before().witness().as_bytes());
    for n in [
        plan.before().generation(),
        plan.before().sequence(),
        plan.before().tick(),
    ] {
        bytes.extend_from_slice(&n.to_be_bytes());
    }
    bytes.push(phase as u8);
    bytes.extend_from_slice(Digest32::ZERO.as_bytes());
    bytes.extend_from_slice(&(plan.before().labor_keys().len() as u16).to_be_bytes());
    bytes.extend_from_slice(&0u16.to_be_bytes());
    let receipt = crate::bounded_run::hash(b"dfmcp-workforce-receipt/1", &bytes);
    bytes.extend_from_slice(receipt.as_bytes());
    assert!(bytes.len() <= MAX_EFFECT);
    AssignmentEffect::decode(&bytes, plan)
}
struct NativeState {
    cap: WorkforceCapture,
    calls: [usize; 5],
    connections: usize,
    effect: Option<AssignmentEffect>,
    fail_read: bool,
    lose_prepare: bool,
    lose_commit: bool,
    missing: bool,
    unknown: bool,
}
#[derive(Clone)]
struct Native {
    binding: WorkforceBinding,
    data: Rc<RefCell<NativeState>>,
    storage: Storage,
}
impl Native {
    fn connect(&self, binding: &WorkforceBinding, c: &OperationContext) -> Result<Self> {
        assert_eq!(binding, &self.binding);
        assert_eq!(c.budget.max_bytes, CONNECT_BYTES);
        self.data.borrow_mut().connections += 1;
        Ok(self.clone())
    }
    fn synced_state(&self, plan: &AssignmentPlan) -> Result<AssignmentState> {
        let bytes = self.storage.0.borrow().synced.clone();
        let storage = Storage::default();
        storage.0.borrow_mut().file = Cursor::new(bytes);
        let ctx = context()?;
        let mut journal = WorkforceJournal::open(storage, &ctx, WorkforceMode::Offline, None)?;
        Ok(journal.get(plan.key(), plan.digest(), &ctx)?.state())
    }
}
impl WorkforceSource for Native {
    fn manifest(&self) -> &WorkforceManifest {
        self.binding.manifest()
    }
    fn endpoint(&self) -> Option<std::net::SocketAddr> {
        Some(self.binding.endpoint())
    }
    fn observe(&mut self, _: &[u32], _: &OperationContext) -> Result<WorkforceCapture> {
        let mut n = self.data.borrow_mut();
        n.calls[0] += 1;
        if n.fail_read {
            return Err(error(ErrorCode::AdapterRejected, "injected read failure"));
        }
        Ok(n.cap.clone())
    }
    fn prepare(&mut self, p: &AssignmentPlan, _: &OperationContext) -> Result<AssignmentEffect> {
        assert_eq!(self.synced_state(p)?, AssignmentState::Intent);
        let e = effect(p, AssignmentPhase::Prepared)?;
        let mut n = self.data.borrow_mut();
        n.calls[1] += 1;
        n.effect = Some(e.clone());
        if n.lose_prepare {
            return Err(error(ErrorCode::AdapterRejected, "lost prepare"));
        }
        Ok(e)
    }
    fn commit(&mut self, p: &AssignmentPlan, _: &OperationContext) -> Result<AssignmentEffect> {
        assert_eq!(self.synced_state(p)?, AssignmentState::DispatchStarted);
        let mut n = self.data.borrow_mut();
        n.calls[2] += 1;
        let e = effect(
            p,
            if n.unknown {
                AssignmentPhase::Unknown
            } else {
                AssignmentPhase::Applied
            },
        )?;
        n.effect = Some(e.clone());
        if n.lose_commit {
            return Err(error(ErrorCode::AdapterRejected, "lost commit"));
        }
        Ok(e)
    }
    fn query(
        &mut self,
        _: &AssignmentPlan,
        _: &OperationContext,
    ) -> Result<Option<AssignmentEffect>> {
        let mut n = self.data.borrow_mut();
        n.calls[3] += 1;
        Ok(if n.missing { None } else { n.effect.clone() })
    }
    fn cancel(&mut self, p: &AssignmentPlan, _: &OperationContext) -> Result<AssignmentEffect> {
        assert_eq!(self.synced_state(p)?, AssignmentState::CancelRequested);
        let e = effect(p, AssignmentPhase::Cancelled)?;
        let mut n = self.data.borrow_mut();
        n.calls[4] += 1;
        n.effect = Some(e.clone());
        Ok(e)
    }
}
fn never(_: &WorkforceBinding, _: &OperationContext) -> Result<Native> {
    Err(error(
        ErrorCode::InternalInvariantViolation,
        "native connection must not be constructed",
    ))
}
fn setup() -> Result<(WorkforceSession<Storage>, Native, OperationContext)> {
    let ctx = context()?;
    let cap = capture()?;
    let binding = WorkforceBinding::new(
        std::net::SocketAddr::from(([127, 0, 0, 1], 5000)),
        WorkforceManifest {
            generation: cap.generation(),
            df_version: "fake-df".into(),
            dfhack_version: "fake-dfhack".into(),
        },
        cap.folder().into(),
        cap.site(),
    )?;
    let storage = Storage::default();
    let journal = WorkforceJournal::open(
        storage.clone(),
        &ctx,
        WorkforceMode::Control,
        Some((binding.clone(), [1; 32])),
    )?;
    let (session, view) = WorkforceSession::new(journal, &ctx)?;
    assert!(view.records.is_empty());
    let native = Native {
        binding,
        storage,
        data: Rc::new(RefCell::new(NativeState {
            cap,
            calls: [0; 5],
            connections: 0,
            effect: None,
            fail_read: false,
            lose_prepare: false,
            lose_commit: false,
            missing: false,
            unknown: false,
        })),
    };
    Ok((session, native, ctx))
}
fn prepare(
    session: &mut WorkforceSession<Storage>,
    n: &Native,
    c: &OperationContext,
) -> Result<AssignmentRecord> {
    let cap = session.observe(&[2, 5], c, |b, c| n.connect(b, c))?;
    session.prepare(
        "assign",
        AssignmentSpec::new(0, true)?,
        cap.witness(),
        c,
        |b, c| n.connect(b, c),
    )
}
fn reopen(
    n: &Native,
    c: &OperationContext,
    mode: WorkforceMode,
) -> Result<WorkforceSession<Storage>> {
    let journal = WorkforceJournal::open(n.storage.clone(), c, mode, None)?;
    Ok(WorkforceSession::new(journal, c)?.0)
}

#[test]
fn lifecycle_exact_retries_and_offline_receipts_do_not_connect() -> Result<()> {
    let (mut s, n, c) = setup()?;
    let p = prepare(&mut s, &n, &c)?;
    assert_eq!(
        s.prepare(
            "assign",
            p.plan().spec(),
            p.plan().before().witness(),
            &c,
            never
        )?,
        p
    );
    assert!(
        s.commit("assign", p.plan().digest(), false, &c, never)
            .is_err()
    );
    let done = s.commit("assign", p.plan().digest(), true, &c, |b, c| {
        n.connect(b, c)
    })?;
    assert_eq!(done.state(), AssignmentState::Terminal);
    assert_eq!(n.data.borrow().calls[2], 1);
    assert_eq!(
        s.commit("assign", p.plan().digest(), true, &c, never)?,
        done
    );
    let mut q = c.clone();
    q.session_id = SessionId::new(2);
    q.grants.retain(|g| g.capability == Capability::Query);
    let mut offline = reopen(&n, &q, WorkforceMode::Offline)?;
    assert_eq!(
        offline.reconcile("assign", p.plan().digest(), &q, never)?,
        done
    );
    assert_eq!(offline.inspect("assign", p.plan().digest(), &q)?, done);
    assert!(offline.view(&c).is_err());
    Ok(())
}
#[test]
fn failed_or_invalid_refresh_invalidates_prior_selection() -> Result<()> {
    let (mut s, n, c) = setup()?;
    let cap = s.observe(&[2, 5], &c, |b, c| n.connect(b, c))?;
    n.data.borrow_mut().fail_read = true;
    assert!(s.observe(&[2, 5], &c, |b, c| n.connect(b, c)).is_err());
    assert!(
        s.prepare(
            "assign",
            AssignmentSpec::new(0, true)?,
            cap.witness(),
            &c,
            never
        )
        .is_err()
    );
    n.data.borrow_mut().fail_read = false;
    s.observe(&[2, 5], &c, |b, c| n.connect(b, c))?;
    assert!(s.observe(&[5, 2], &c, never).is_err());
    assert!(
        s.prepare(
            "assign",
            AssignmentSpec::new(0, true)?,
            cap.witness(),
            &c,
            never
        )
        .is_err()
    );
    Ok(())
}
#[test]
fn wrong_witness_duplicate_conflict_and_new_keys_refuse_before_connection() -> Result<()> {
    let (mut s, n, c) = setup()?;
    s.observe(&[2, 5], &c, |b, c| n.connect(b, c))?;
    assert!(
        s.prepare(
            "assign",
            AssignmentSpec::new(0, true)?,
            Digest32::ZERO,
            &c,
            never
        )
        .is_err()
    );
    let p = prepare(&mut s, &n, &c)?;
    assert!(
        s.prepare(
            "assign",
            AssignmentSpec::new(0, false)?,
            p.plan().before().witness(),
            &c,
            never
        )
        .is_err()
    );
    assert!(
        s.prepare(
            "other",
            p.plan().spec(),
            p.plan().before().witness(),
            &c,
            never
        )
        .is_err()
    );
    assert!(s.commit("assign", Digest32::ZERO, true, &c, never).is_err());
    Ok(())
}
#[test]
fn authority_is_current_scoped_unlimited_and_session_bound() -> Result<()> {
    let (mut s, n, c) = setup()?;
    let p = prepare(&mut s, &n, &c)?;
    for capability in [
        Capability::Query,
        Capability::Plan,
        Capability::ConfigureLabor,
    ] {
        let mut denied = c.clone();
        denied.grants.retain(|g| g.capability != capability);
        assert!(
            s.prepare(
                "assign",
                p.plan().spec(),
                p.plan().before().witness(),
                &denied,
                never
            )
            .is_err()
        );
    }
    let mut denied = c.clone();
    denied
        .grants
        .iter_mut()
        .for_each(|g| g.remaining_uses = Some(1));
    assert!(
        s.commit("assign", p.plan().digest(), true, &denied, never)
            .is_err()
    );
    denied = c.clone();
    denied.session_id = SessionId::new(99);
    assert!(s.view(&denied).is_err());
    denied = c.clone();
    denied.cancellation_requested = true;
    assert!(s.view(&denied).is_err());
    denied = c.clone();
    denied.anchor.fortress_id = dfmcp_core::FortressId::new(99);
    assert!(s.view(&denied).is_err());
    assert_eq!(n.data.borrow().calls[2], 0);
    Ok(())
}
#[test]
fn fixed_modes_cannot_be_promoted_with_injected_grants() -> Result<()> {
    let (mut s, n, c) = setup()?;
    let p = prepare(&mut s, &n, &c)?;
    for mode in [WorkforceMode::Offline, WorkforceMode::Recover] {
        let mut r = reopen(&n, &c, mode)?;
        assert!(
            r.prepare(
                "assign",
                p.plan().spec(),
                p.plan().before().witness(),
                &c,
                never
            )
            .is_err()
        );
        assert!(
            r.commit("assign", p.plan().digest(), true, &c, never)
                .is_err()
        );
        assert!(r.cancel("assign", p.plan().digest(), &c, never).is_err());
        if mode == WorkforceMode::Offline {
            assert!(r.observe(&[2, 5], &c, never).is_err());
        }
    }
    Ok(())
}
#[test]
fn lost_preparation_is_queried_without_renewing_or_restoring_grants() -> Result<()> {
    let (mut s, n, c) = setup()?;
    n.data.borrow_mut().lose_prepare = true;
    assert!(prepare(&mut s, &n, &c).is_err());
    let p = s.view(&c)?.records.remove(0);
    let mut q = c.clone();
    q.grants.retain(|g| g.capability == Capability::Query);
    let mut r = reopen(&n, &q, WorkforceMode::Recover)?;
    assert_eq!(
        r.reconcile("assign", p.plan().digest(), &q, |b, c| n.connect(b, c))?
            .state(),
        AssignmentState::Prepared
    );
    assert_eq!(n.data.borrow().calls[1], 1);
    assert_eq!(n.data.borrow().calls[3], 1);
    assert!(
        r.commit("assign", p.plan().digest(), true, &q, never)
            .is_err()
    );
    Ok(())
}
#[test]
fn lost_commit_and_absent_native_record_never_allow_redispatch() -> Result<()> {
    let (mut s, n, c) = setup()?;
    let p = prepare(&mut s, &n, &c)?;
    n.data.borrow_mut().lose_commit = true;
    assert!(
        s.commit("assign", p.plan().digest(), true, &c, |b, c| n
            .connect(b, c))
            .is_err()
    );
    let mut r = reopen(&n, &c, WorkforceMode::Control)?;
    assert!(
        r.commit("assign", p.plan().digest(), true, &c, never)
            .is_err()
    );
    n.data.borrow_mut().missing = true;
    assert!(
        r.reconcile("assign", p.plan().digest(), &c, |b, c| n.connect(b, c))
            .is_err()
    );
    assert_eq!(
        r.inspect("assign", p.plan().digest(), &c)?.state(),
        AssignmentState::DispatchStarted
    );
    n.data.borrow_mut().missing = false;
    assert_eq!(
        r.reconcile("assign", p.plan().digest(), &c, |b, c| n.connect(b, c))?
            .state(),
        AssignmentState::Terminal
    );
    assert_eq!(n.data.borrow().calls[2], 1);
    Ok(())
}
#[test]
fn permanent_unknown_is_local_unsettled_and_not_cancellable() -> Result<()> {
    let (mut s, n, c) = setup()?;
    let p = prepare(&mut s, &n, &c)?;
    n.data.borrow_mut().unknown = true;
    let unknown = s.commit("assign", p.plan().digest(), true, &c, |b, c| {
        n.connect(b, c)
    })?;
    assert!(!unknown.state().settled());
    assert_eq!(
        s.reconcile("assign", p.plan().digest(), &c, never)?,
        unknown
    );
    assert_eq!(s.cancel("assign", p.plan().digest(), &c, never)?, unknown);
    assert!(
        s.prepare(
            "other",
            p.plan().spec(),
            p.plan().before().witness(),
            &c,
            never
        )
        .is_err()
    );
    Ok(())
}
#[test]
fn local_cancel_and_crash_before_native_dispatch_have_distinct_retirement() -> Result<()> {
    let (mut s, n, c) = setup()?;
    let p = prepare(&mut s, &n, &c)?;
    assert_eq!(
        s.cancel("assign", p.plan().digest(), &c, never)?.state(),
        AssignmentState::CancelledBeforeDispatch
    );
    assert_eq!(n.data.borrow().calls[4], 0);
    let (mut s, n, c) = setup()?;
    let p = prepare(&mut s, &n, &c)?;
    n.storage.0.borrow_mut().fail_sync = true;
    assert!(
        s.commit("assign", p.plan().digest(), true, &c, |b, c| n
            .connect(b, c))
            .is_err()
    );
    assert_eq!(n.data.borrow().calls[2], 0);
    n.storage.0.borrow_mut().fail_sync = false;
    let mut r = reopen(&n, &c, WorkforceMode::Control)?;
    assert_eq!(
        r.reconcile("assign", p.plan().digest(), &c, |b, c| n.connect(b, c))?
            .state(),
        AssignmentState::Tracking
    );
    assert!(
        r.commit("assign", p.plan().digest(), true, &c, never)
            .is_err()
    );
    assert_eq!(
        r.cancel("assign", p.plan().digest(), &c, |b, c| n.connect(b, c))?
            .state(),
        AssignmentState::Terminal
    );
    assert_eq!(n.data.borrow().calls[4], 1);
    Ok(())
}
#[test]
fn changed_custody_refuses_before_connection_and_failed_intent_never_prepares() -> Result<()> {
    let (mut s, n, c) = setup()?;
    n.storage.0.borrow_mut().file.get_mut()[0] ^= 1;
    assert!(s.observe(&[2, 5], &c, never).is_err());
    assert_eq!(n.data.borrow().connections, 0);
    let (mut s, n, c) = setup()?;
    s.observe(&[2, 5], &c, |b, c| n.connect(b, c))?;
    n.storage.0.borrow_mut().fail_write = true;
    assert!(
        s.prepare(
            "assign",
            AssignmentSpec::new(0, true)?,
            capture()?.witness(),
            &c,
            |b, c| n.connect(b, c)
        )
        .is_err()
    );
    assert_eq!(n.data.borrow().calls[1], 0);
    Ok(())
}
#[test]
fn connection_budget_and_deadline_cannot_renew() -> Result<()> {
    let (mut s, n, mut c) = setup()?;
    c.budget.max_bytes = CONNECT_BYTES;
    assert!(s.observe(&[2, 5], &c, never).is_err());
    assert_eq!(n.data.borrow().connections, 0);
    let mut a = Allowance::new(context()?)?;
    a.deadline = Instant::now();
    assert!(a.current().is_err());
    assert!(a.connect(&n.binding, never).is_err());
    Ok(())
}
#[test]
fn observed_time_floor_prevents_regression_and_expired_grant_reuse() -> Result<()> {
    let (mut s, n, c) = setup()?;
    s.observe(&[2, 5], &c, |b, c| n.connect(b, c))?;
    let mut denied = c.clone();
    denied
        .grants
        .iter_mut()
        .for_each(|g| g.expires_at_tick = Some(GameTick(99)));
    assert!(
        s.prepare(
            "assign",
            AssignmentSpec::new(0, true)?,
            capture()?.witness(),
            &denied,
            never
        )
        .is_err()
    );
    let mut raw = capture()?.canonical_bytes().to_vec();
    raw[24..32].copy_from_slice(&99u64.to_be_bytes());
    n.data.borrow_mut().cap = WorkforceCapture::decode(&raw)?;
    assert!(s.observe(&[2, 5], &c, |b, c| n.connect(b, c)).is_err());
    assert_eq!(s.high_tick(), 100);
    assert!(
        s.prepare(
            "assign",
            AssignmentSpec::new(0, true)?,
            capture()?.witness(),
            &c,
            never
        )
        .is_err()
    );
    Ok(())
}
