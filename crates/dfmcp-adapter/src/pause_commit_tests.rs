use super::*;
use crate::control_effect_journal::EffectTailRecovery;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, FortressId, GameTick, ObservationCursor, RequestId,
    SessionId, StateAnchor, WorkBudget,
};
use std::cell::{Cell, RefCell};
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::rc::Rc;

#[derive(Clone, Default)]
struct Memory {
    bytes: Rc<RefCell<Cursor<Vec<u8>>>>,
    syncs: Rc<Cell<usize>>,
    fail_sync: Rc<Cell<Option<usize>>>,
    write_limit: Rc<Cell<Option<usize>>>,
}
impl Read for Memory {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        self.bytes.borrow_mut().read(b)
    }
}
impl Seek for Memory {
    fn seek(&mut self, p: SeekFrom) -> io::Result<u64> {
        self.bytes.borrow_mut().seek(p)
    }
}
impl Write for Memory {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        let n = match self.write_limit.get() {
            Some(0) => return Err(io::Error::other("injected partial write")),
            Some(limit) => {
                let n = limit.min(b.len());
                self.write_limit.set(Some(limit - n));
                n
            }
            None => b.len(),
        };
        self.bytes.borrow_mut().write(&b[..n])
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
            Err(io::Error::other("injected sync"))
        } else {
            Ok(())
        }
    }
    fn truncate(&mut self, n: u64) -> io::Result<()> {
        self.bytes.borrow_mut().get_mut().truncate(n as usize);
        Ok(())
    }
}
fn context() -> OperationContext {
    OperationContext {
        session_id: SessionId::new(51),
        request_id: RequestId::new(1),
        anchor: StateAnchor {
            fortress_id: FortressId::NIL,
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(10),
            state_hash: Digest32::ZERO,
        },
        budget: WorkBudget {
            max_wall_millis: 60000,
            ..WorkBudget::default()
        },
        cancellation_requested: false,
        grants: [Capability::ControlClock, Capability::Query]
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope::default(),
                max_risk: RiskTier::Reversible,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect(),
    }
}
fn reopen(storage: &Memory, read_only: bool) -> Result<ControlEffectJournal<Memory>> {
    let bytes = storage.bytes.borrow().get_ref().clone();
    let storage = Memory {
        bytes: Rc::new(RefCell::new(Cursor::new(bytes))),
        ..Memory::default()
    };
    if read_only {
        ControlEffectJournal::open_read_only(storage, &context())
    } else {
        ControlEffectJournal::open(storage, &context(), false, 7, EffectTailRecovery::Refuse)
    }
}
#[derive(Clone, Copy)]
enum Reply {
    Applied,
    NotApplied,
    Unknown,
    Invalid,
    Io,
}
struct Source {
    storage: Memory,
    reply: Reply,
    generation: u64,
    preflights: usize,
    commits: usize,
    fenced: bool,
    partial_terminal: bool,
    allowances: Vec<Duration>,
}
impl PauseCommitSource for Source {
    fn preflight(&mut self, remaining: Duration, _: &OperationContext) -> Result<u64> {
        self.preflights += 1;
        self.allowances.push(remaining);
        if self.fenced {
            return Err(DfmcpError::new(
                ErrorCode::AdapterUnavailable,
                "fenced test source",
            ));
        }
        Ok(self.generation)
    }
    fn commit_prepared(
        &mut self,
        record: &DurablePauseRecord,
        remaining: Duration,
        _: &OperationContext,
    ) -> Result<PauseEffect> {
        self.commits += 1;
        self.allowances.push(remaining);
        assert_eq!(
            self.storage.syncs.get(),
            3,
            "header, prepare and commit intent must be synced before RPC"
        );
        assert_eq!(
            reopen(&self.storage, true)?.lookup("key").map(|r| r.state),
            Some(DurablePauseState::CommitStarted)
        );
        assert_eq!(record.state, DurablePauseState::CommitStarted);
        if self.partial_terminal {
            self.storage.write_limit.set(Some(12));
        }
        if matches!(self.reply, Reply::Io) {
            return Err(DfmcpError::new(ErrorCode::AdapterUnavailable, "reply lost"));
        }
        let known = !matches!(self.reply, Reply::Unknown);
        let applied = matches!(self.reply, Reply::Applied | Reply::Invalid);
        Ok(PauseEffect {
            bridge_generation: self.generation,
            known,
            applied,
            paused: if applied {
                record.desired_paused
            } else {
                !record.desired_paused
            },
            observed_tick: 11,
            prepare_token: if known {
                record.prepare_token.to_vec()
            } else {
                Vec::new()
            },
            receipt_digest: match self.reply {
                Reply::Unknown => Vec::new(),
                Reply::Invalid => vec![9; 32],
                _ => receipt_digest(record, 11).as_bytes().to_vec(),
            },
        })
    }
    fn fence(&mut self) {
        self.fenced = true;
    }
}
fn fixture(reply: Reply) -> Result<(ControlEffectJournal<Memory>, Source, Digest32)> {
    let memory = Memory::default();
    let plan = Digest32::of_bytes(b"plan");
    let mut journal = ControlEffectJournal::open(
        memory.clone(),
        &context(),
        true,
        7,
        EffectTailRecovery::Refuse,
    )?;
    journal.record_prepared("key".into(), plan, true, 10, 7, [1; 16], &context())?;
    Ok((
        journal,
        Source {
            storage: memory,
            reply,
            generation: 7,
            preflights: 0,
            commits: 0,
            fenced: false,
            partial_terminal: false,
            allowances: Vec::new(),
        },
        plan,
    ))
}

#[test]
fn terminal_outcomes_require_durable_intent_and_never_dispatch_twice() -> Result<()> {
    for (reply, state) in [
        (Reply::Applied, DurablePauseState::VerifiedApplied),
        (Reply::NotApplied, DurablePauseState::VerifiedNotApplied),
    ] {
        let (mut journal, mut source, plan) = fixture(reply)?;
        let first = commit_once(&mut journal, &mut source, "key", plan, &[1; 16], &context())?;
        assert_eq!(first.record.state, state);
        assert!(first.dispatch_attempted);
        assert!(!first.replayed_terminal);
        source.fenced = true;
        // Historical evidence does not bypass the core budget or current grant.
        let mut invalid = context();
        invalid.budget.max_actions = 0;
        assert!(
            matches!(commit_once(&mut journal,&mut source,"key",plan,&[1;16],&invalid),Err(e)if e.code==ErrorCode::InvalidRequest)
        );
        let mut denied = context();
        denied.grants.clear();
        assert!(
            matches!(commit_once(&mut journal,&mut source,"key",plan,&[1;16],&denied),Err(e)if e.code==ErrorCode::CapabilityDenied)
        );
        let replay = commit_once(&mut journal, &mut source, "key", plan, &[1; 16], &context())?;
        assert!(replay.replayed_terminal);
        assert!(!replay.dispatch_attempted);
        assert_eq!(replay.record, first.record);
        assert_eq!((source.preflights, source.commits), (1, 1));
        assert_eq!(
            reopen(&source.storage, false)?.lookup("key"),
            Some(&first.record)
        );
    }
    Ok(())
}

#[test]
fn failed_intent_sync_never_calls_the_mutation_source() -> Result<()> {
    let (mut journal, mut source, plan) = fixture(Reply::Applied)?;
    source.storage.fail_sync.set(Some(3));
    assert!(
        matches!(commit_once(&mut journal,&mut source,"key",plan,&[1;16],&context()),Err(e)if e.code==ErrorCode::CorruptLedger)
    );
    assert_eq!(source.commits, 0);
    assert!(journal.fenced());
    assert_eq!(
        journal.lookup("key").map(|r| r.state),
        Some(DurablePauseState::Prepared)
    );
    let mut recovered = reopen(&source.storage, false)?;
    assert!(
        matches!(commit_once(&mut recovered,&mut source,"key",plan,&[1;16],&context()),Err(e)if e.code==ErrorCode::EffectIndeterminate)
    );
    assert_eq!(source.commits, 0);
    Ok(())
}

#[test]
fn lost_or_invalid_replies_fence_and_recover_as_unresolved_not_retryable() -> Result<()> {
    for reply in [Reply::Io, Reply::Invalid] {
        let (mut journal, mut source, plan) = fixture(reply)?;
        assert!(
            matches!(commit_once(&mut journal,&mut source,"key",plan,&[1;16],&context()),Err(e)if e.code==ErrorCode::EffectIndeterminate)
        );
        assert!(source.fenced);
        assert_eq!(source.commits, 1);
        let mut recovered = reopen(&source.storage, false)?;
        assert_eq!(
            recovered.lookup("key").map(|r| r.state),
            Some(DurablePauseState::Indeterminate)
        );
        assert!(
            matches!(commit_once(&mut recovered,&mut source,"key",plan,&[1;16],&context()),Err(e)if e.code==ErrorCode::EffectIndeterminate)
        );
        assert_eq!(source.commits, 1);
    }
    Ok(())
}

#[test]
fn unknown_native_evidence_does_not_become_a_failed_effect_receipt() -> Result<()> {
    let (mut journal, mut source, plan) = fixture(Reply::Unknown)?;
    let out = commit_once(&mut journal, &mut source, "key", plan, &[1; 16], &context())?;
    assert_eq!(out.record.state, DurablePauseState::Indeterminate);
    assert!(out.dispatch_attempted);
    assert!(out.record.receipt_digest.is_none());
    assert!(!out.record.effect_known);
    Ok(())
}

#[test]
fn terminal_sync_failure_is_not_acknowledged_but_complete_bytes_can_recover() -> Result<()> {
    let (mut journal, mut source, plan) = fixture(Reply::Applied)?;
    source.storage.fail_sync.set(Some(4));
    assert!(
        matches!(commit_once(&mut journal,&mut source,"key",plan,&[1;16],&context()),Err(e)if e.code==ErrorCode::EffectIndeterminate)
    );
    assert!(journal.fenced());
    assert!(source.fenced);
    assert_eq!(source.commits, 1);
    let mut recovered = reopen(&source.storage, false)?;
    let out = commit_once(
        &mut recovered,
        &mut source,
        "key",
        plan,
        &[1; 16],
        &context(),
    )?;
    assert!(out.replayed_terminal);
    assert_eq!(out.record.state, DurablePauseState::VerifiedApplied);
    assert_eq!(source.commits, 1);
    Ok(())
}

#[test]
fn partial_terminal_write_leaves_an_unacknowledged_attempt_and_refuses_replay() -> Result<()> {
    let (mut journal, mut source, plan) = fixture(Reply::Applied)?;
    source.partial_terminal = true;
    assert!(
        matches!(commit_once(&mut journal,&mut source,"key",plan,&[1;16],&context()),Err(e)if e.code==ErrorCode::EffectIndeterminate)
    );
    assert!(journal.fenced());
    assert_eq!(source.commits, 1);
    assert!(reopen(&source.storage, false).is_err());
    Ok(())
}

#[test]
fn deadline_is_shared_and_expiry_after_intent_never_makes_a_retryable_prepare() -> Result<()> {
    for after_intent in [false, true] {
        let (mut journal, mut source, plan) = fixture(Reply::Applied)?;
        let mut c = context();
        c.budget.max_wall_millis = 10;
        let times = if after_intent {
            vec![0, 2, 10]
        } else {
            vec![0, 10]
        };
        let mut times = times.into_iter();
        let out = commit_with_clock(&mut journal, &mut source, "key", plan, &[1; 16], &c, || {
            Duration::from_millis(times.next().unwrap_or(10))
        });
        let expected = if after_intent {
            ErrorCode::EffectIndeterminate
        } else {
            ErrorCode::BudgetExceeded
        };
        assert!(matches!(out,Err(e)if e.code==expected));
        assert_eq!(source.commits, 0);
        let state = if after_intent {
            DurablePauseState::Indeterminate
        } else {
            DurablePauseState::Prepared
        };
        assert_eq!(journal.lookup("key").map(|r| r.state), Some(state));
    }
    let (mut journal, mut source, plan) = fixture(Reply::Applied)?;
    let mut c = context();
    c.budget.max_wall_millis = 10;
    let mut times = [1, 3, 6].into_iter();
    commit_with_clock(&mut journal, &mut source, "key", plan, &[1; 16], &c, || {
        Duration::from_millis(times.next().unwrap_or(10))
    })?;
    assert_eq!(
        source.allowances,
        [Duration::from_millis(9), Duration::from_millis(4)]
    );
    Ok(())
}

#[test]
fn budgets_authority_and_identity_refusals_precede_source_calls_and_writes() -> Result<()> {
    for case in 0..6 {
        let (mut journal, mut source, plan) = fixture(Reply::Applied)?;
        let head = journal.head();
        let mut c = context();
        let mut digest = plan;
        let mut token = [1; 16];
        match case {
            0 => c.budget.max_actions = 0,
            1 => c.grants.clear(),
            2 => c.cancellation_requested = true,
            3 => digest = Digest32::ZERO,
            4 => token = [2; 16],
            _ => {
                journal.cancel_prepared("key", plan, &c)?;
            }
        }
        let head = if case == 5 { journal.head() } else { head };
        let expected = match case {
            0 => ErrorCode::InvalidRequest,
            1 => ErrorCode::CapabilityDenied,
            2 => ErrorCode::CancellationRequested,
            _ => ErrorCode::Conflict,
        };
        assert!(
            matches!(commit_once(&mut journal,&mut source,"key",digest,&token,&c),Err(e)if e.code==expected)
        );
        assert_eq!((source.preflights, source.commits), (0, 0));
        assert_eq!(journal.head(), head);
    }
    let (journal, mut source, plan) = fixture(Reply::Applied)?;
    let mut read_only = reopen(&source.storage, true)?;
    assert!(
        matches!(commit_once(&mut read_only,&mut source,"key",plan,&[1;16],&context()),Err(e)if e.code==ErrorCode::CapabilityDenied)
    );
    assert_eq!(source.commits, 0);
    drop(journal);
    Ok(())
}

#[test]
fn changed_generation_and_poisoned_connections_never_start_a_commit() -> Result<()> {
    for fenced in [false, true] {
        let (mut journal, mut source, plan) = fixture(Reply::Applied)?;
        source.generation = 8;
        source.fenced = fenced;
        let head = journal.head();
        assert!(commit_once(&mut journal, &mut source, "key", plan, &[1; 16], &context()).is_err());
        assert_eq!(source.commits, 0);
        assert_eq!(journal.head(), head);
    }
    Ok(())
}
