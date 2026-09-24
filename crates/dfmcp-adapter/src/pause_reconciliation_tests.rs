use super::*;
use std::collections::VecDeque;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};

use crate::control_effect_journal::{DurablePauseState, EffectTailRecovery};
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, FortressId, GameTick, ObservationCursor, RequestId,
    SessionId, StateAnchor, WorkBudget,
};

#[derive(Default)]
struct Memory {
    bytes: Cursor<Vec<u8>>,
    fail_sync: bool,
}
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.bytes.read(out)
    }
}
impl Write for Memory {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Seek for Memory {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.bytes.seek(from)
    }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        if self.fail_sync {
            Err(io::Error::other("injected sync failure"))
        } else {
            Ok(())
        }
    }
    fn truncate(&mut self, length: u64) -> io::Result<()> {
        self.bytes.get_mut().truncate(length as usize);
        Ok(())
    }
}
fn context() -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(2),
        anchor: StateAnchor {
            fortress_id: FortressId::NIL,
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(10),
            state_hash: Digest32::ZERO,
        },
        budget: WorkBudget::default(),
        cancellation_requested: false,
        grants: vec![CapabilityGrant {
            capability: Capability::ControlClock,
            scope: CapabilityScope::default(),
            max_risk: RiskTier::Reversible,
            expires_at_tick: None,
            remaining_uses: None,
        }],
    }
}
fn fixture(keys: &[&str]) -> Result<ControlEffectJournal<Memory>> {
    let mut journal = ControlEffectJournal::open(
        Memory::default(),
        &context(),
        true,
        7,
        EffectTailRecovery::Refuse,
    )?;
    for key in keys {
        let plan = Digest32::of_bytes(b"plan");
        let seal = token_digest(key, plan, 7, 10, true);
        let mut token = [0; 16];
        token.copy_from_slice(&seal.as_bytes()[..16]);
        journal.record_prepared((*key).to_owned(), plan, true, 10, 7, token, &context())?;
        if *key != "prepared" {
            journal.begin_commit(key, plan, 7, &context())?;
        }
    }
    Ok(journal)
}
fn record<S: EffectJournalStorage>(
    journal: &ControlEffectJournal<S>,
    key: &str,
) -> Result<DurablePauseRecord> {
    journal
        .lookup(key)
        .cloned()
        .ok_or_else(|| rejected("test fixture missing"))
}
fn terminal(record: &DurablePauseRecord, applied: bool) -> PauseEffect {
    PauseEffect {
        bridge_generation: record.bridge_generation,
        known: true,
        applied,
        paused: if applied {
            record.desired_paused
        } else {
            !record.desired_paused
        },
        observed_tick: 11,
        prepare_token: record.prepare_token.to_vec(),
        receipt_digest: receipt_digest(record, 11).as_bytes().to_vec(),
    }
}
#[derive(Default)]
struct Source {
    replies: VecDeque<Result<PauseEffect>>,
    calls: Vec<String>,
    allowances: Vec<Duration>,
}
impl PauseReconciliationSource for Source {
    fn query_effect(
        &mut self,
        record: &DurablePauseRecord,
        remaining: Duration,
        _context: &OperationContext,
    ) -> Result<PauseEffect> {
        self.calls.push(record.idempotency_key.clone());
        self.allowances.push(remaining);
        self.replies
            .pop_front()
            .ok_or_else(|| rejected("unexpected bridge query"))?
    }
}
fn keys(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).to_owned()).collect()
}

#[test]
fn identity_vectors_match_independent_python_sha256() -> Result<()> {
    let journal = fixture(&["k"])?;
    let r = record(&journal, "k")?;
    let token = r
        .prepare_token
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect::<String>();
    assert_eq!(token, "c6c6cf2bcc44a56096f032b7b9c3d103");
    assert_eq!(
        receipt_digest(&r, 11).to_string(),
        "585db3220889e25732c55c031b1117cd4d57bd68bd7ccd0b41e187442d4266d6"
    );
    Ok(())
}

#[test]
fn prepare_checks_the_complete_native_seal() -> Result<()> {
    let journal = fixture(&["k"])?;
    let r = record(&journal, "k")?;
    let effect = PauseEffect {
        bridge_generation: 7,
        known: false,
        applied: false,
        paused: false,
        observed_tick: 0,
        prepare_token: r.prepare_token.to_vec(),
        receipt_digest: vec![],
    };
    assert_eq!(
        validate_prepare_reply("k", r.plan_digest, true, 10, &effect)?,
        r.prepare_token
    );
    for (key, plan, paused, tick) in [
        ("other", r.plan_digest, true, 10),
        ("k", Digest32::ZERO, true, 10),
        ("k", r.plan_digest, false, 10),
        ("k", r.plan_digest, true, 11),
    ] {
        assert!(validate_prepare_reply(key, plan, paused, tick, &effect).is_err());
    }
    let mut altered = effect;
    altered.bridge_generation += 1;
    assert!(validate_prepare_reply("k", r.plan_digest, true, 10, &altered).is_err());
    Ok(())
}

#[test]
fn prepared_native_record_never_proves_not_applied_after_lost_dispatch() -> Result<()> {
    let mut journal = fixture(&["k"])?;
    let r = record(&journal, "k")?;
    let mut effect = terminal(&r, false);
    effect.receipt_digest.clear();
    let recovered = reconcile_reply(&mut journal, "k", r.plan_digest, &effect, &context())?;
    assert_eq!(recovered.state, DurablePauseState::Indeterminate);
    assert!(!recovered.effect_known);
    assert!(recovered.receipt_digest.is_none());
    assert!(!recovered.safe_to_dispatch(7));
    let head = journal.head();
    reconcile_reply(&mut journal, "k", r.plan_digest, &effect, &context())?;
    assert_eq!(journal.head(), head);
    Ok(())
}

#[test]
fn receipt_verified_applied_and_actual_not_applied_are_terminal() -> Result<()> {
    for applied in [false, true] {
        let mut journal = fixture(&["k"])?;
        let r = record(&journal, "k")?;
        let updated = reconcile_reply(
            &mut journal,
            "k",
            r.plan_digest,
            &terminal(&r, applied),
            &context(),
        )?;
        assert!(updated.state.terminal());
        assert_eq!(updated.effect_applied, applied);
        assert_eq!(updated.observed_paused, Some(applied));
        assert!(updated.receipt_digest.is_some());
    }
    Ok(())
}

#[test]
fn wrong_receipt_token_tick_or_outcome_never_resolves() -> Result<()> {
    for case in 0..6 {
        let mut journal = fixture(&["k"])?;
        let r = record(&journal, "k")?;
        let mut effect = terminal(&r, true);
        match case {
            0 => effect.receipt_digest[0] ^= 1,
            1 => {
                effect.receipt_digest.pop();
            }
            2 => effect.prepare_token[0] ^= 1,
            3 => effect.observed_tick = 9,
            4 => effect.paused = false,
            _ => effect.receipt_digest.clear(),
        }
        let head = journal.head();
        assert!(reconcile_reply(&mut journal, "k", r.plan_digest, &effect, &context()).is_err());
        assert_eq!(journal.head(), head);
        assert_eq!(
            record(&journal, "k")?.state,
            DurablePauseState::CommitStarted
        );
    }
    Ok(())
}

#[test]
fn unknown_or_another_generation_stays_indeterminate() -> Result<()> {
    for changed_generation in [false, true] {
        let mut journal = fixture(&["k"])?;
        let r = record(&journal, "k")?;
        let mut effect = terminal(&r, true);
        if changed_generation {
            effect.bridge_generation += 1;
        } else {
            effect.known = false;
        }
        assert_eq!(
            reconcile_reply(&mut journal, "k", r.plan_digest, &effect, &context())?.state,
            DurablePauseState::Indeterminate
        );
    }
    Ok(())
}

#[test]
fn batch_is_sorted_and_skips_prepared_and_terminal_without_rpc() -> Result<()> {
    let mut journal = fixture(&["a", "b", "done", "prepared"])?;
    let a = record(&journal, "a")?;
    let b = record(&journal, "b")?;
    let done = record(&journal, "done")?;
    reconcile_reply(
        &mut journal,
        "done",
        done.plan_digest,
        &terminal(&done, true),
        &context(),
    )?;
    let mut source = Source {
        replies: VecDeque::from([Ok(terminal(&a, true)), Ok(terminal(&b, false))]),
        ..Source::default()
    };
    let batch = reconcile_batch_with_clock(
        &mut journal,
        &mut source,
        &keys(&["prepared", "b", "done", "a"]),
        &context(),
        || Duration::ZERO,
    )?;
    assert_eq!(source.calls, ["a", "b"]);
    assert!(batch.stopped.is_none());
    assert_eq!(
        batch
            .items
            .iter()
            .map(|v| v.record.idempotency_key.as_str())
            .collect::<Vec<_>>(),
        ["a", "b", "done", "prepared"]
    );
    assert!(!batch.items[3].queried);
    assert_eq!(batch.items[3].record.state, DurablePauseState::Prepared);
    Ok(())
}

#[test]
fn entire_selection_is_validated_before_any_query() -> Result<()> {
    let mut journal = fixture(&["a"])?;
    let mut source = Source::default();
    let head = journal.head();
    for selection in [
        vec![],
        keys(&["a", "missing"]),
        keys(&["a", "a"]),
        vec!["a".repeat(513)],
    ] {
        assert!(reconcile_batch(&mut journal, &mut source, &selection, &context()).is_err());
        assert!(source.calls.is_empty());
        assert_eq!(journal.head(), head);
    }
    let mut denied = context();
    denied.grants.clear();
    assert!(reconcile_batch(&mut journal, &mut source, &keys(&["a"]), &denied).is_err());
    let mut cancelled = context();
    cancelled.cancellation_requested = true;
    assert!(reconcile_batch(&mut journal, &mut source, &keys(&["a"]), &cancelled).is_err());
    assert!(source.calls.is_empty());
    Ok(())
}

#[test]
fn batch_preserves_completed_evidence_and_defers_after_transport_failure() -> Result<()> {
    let mut journal = fixture(&["a", "b", "c"])?;
    let a = record(&journal, "a")?;
    let mut source = Source {
        replies: VecDeque::from([
            Ok(terminal(&a, true)),
            Err(DfmcpError::new(ErrorCode::AdapterUnavailable, "injected")),
        ]),
        ..Source::default()
    };
    let batch = reconcile_batch_with_clock(
        &mut journal,
        &mut source,
        &keys(&["a", "b", "c"]),
        &context(),
        || Duration::ZERO,
    )?;
    assert_eq!(source.calls, ["a", "b"]);
    assert!(batch.items[0].record.state.terminal());
    assert_eq!(batch.items[1].error, Some(ErrorCode::AdapterUnavailable));
    assert!(batch.items[2].deferred);
    assert_eq!(
        record(&journal, "b")?.state,
        DurablePauseState::CommitStarted
    );
    assert_eq!(batch.stopped, Some(ErrorCode::AdapterUnavailable));
    Ok(())
}

#[test]
fn batch_deadline_is_shared_not_reset_for_each_effect() -> Result<()> {
    let mut journal = fixture(&["a", "b"])?;
    let a = record(&journal, "a")?;
    let mut source = Source {
        replies: VecDeque::from([Ok(terminal(&a, true))]),
        ..Source::default()
    };
    let mut calls = 0;
    let batch = reconcile_batch_with_clock(
        &mut journal,
        &mut source,
        &keys(&["a", "b"]),
        &context(),
        || {
            calls += 1;
            if calls == 1 {
                Duration::ZERO
            } else {
                Duration::from_secs(60)
            }
        },
    )?;
    assert_eq!(source.calls, ["a"]);
    assert!(batch.items[1].deferred);
    assert_eq!(batch.stopped, Some(ErrorCode::BudgetExceeded));
    Ok(())
}
