use super::*;
use dfmcp_core::{FortressId, GameTick, ObservationCursor, RequestId, StateAnchor, WorkBudget};

fn summary() -> JobJournalSummary {
    JobJournalSummary { fortress_id: FortressId::new(7), journal_id: Digest32::of_bytes(b"journal"),
        head: Digest32::of_bytes(b"head"), retained_bytes: 128, transitions: 3,
        records: 3, prepared: 1, unresolved: 1, terminal: 1, read_only: true }
}
fn context() -> OperationContext {
    OperationContext { session_id: SessionId::new(7), request_id: RequestId::new(8),
        anchor: StateAnchor { fortress_id: FortressId::new(7), cursor: ObservationCursor::ORIGIN,
            tick: GameTick(0), state_hash: Digest32::ZERO },
        budget: WorkBudget::default(), grants: Vec::new(), cancellation_requested: false }
}

#[test]
fn digest_parser_rejects_noncanonical_and_unbounded_inputs() -> Result<()> {
    assert_eq!(digest(&Digest32::ZERO.to_string())?, Digest32::ZERO);
    for value in ["", "abc", &"0".repeat(63), &"a".repeat(65), &"A".repeat(64), &"é".repeat(32)] {
        assert!(digest(value).is_err());
    }
    Ok(())
}

#[test]
fn cursors_bind_session_journal_head_filter_and_limit_and_support_replay() -> Result<()> {
    let mut cursors = Continuations::default(); let s = summary(); let id = SessionId::new(9);
    let token = cursors.issue(id, &s, "job-001".into(), Filter::Pending, 8);
    assert_eq!(token.len(), 64);
    assert_eq!(cursors.resolve(&token, id, &s, Filter::Pending, 8)?, "job-001");
    let next = cursors.issue(id, &s, "job-002".into(), Filter::Pending, 8);
    assert_ne!(next, token);
    assert_eq!(cursors.resolve(&token, id, &s, Filter::Pending, 8)?, "job-001");
    assert_eq!(cursors.issue(id, &s, "job-001".into(), Filter::Pending, 8), token);
    assert!(cursors.resolve(&token, SessionId::new(10), &s, Filter::Pending, 8).is_err());
    assert!(cursors.resolve(&token, id, &s, Filter::All, 8).is_err());
    assert!(cursors.resolve(&token, id, &s, Filter::Pending, 1).is_err());
    let mut changed = s.clone(); changed.journal_id = Digest32::ZERO;
    assert!(cursors.resolve(&token, id, &changed, Filter::Pending, 8).is_err());
    changed = s; changed.head = Digest32::ZERO;
    assert!(matches!(cursors.resolve(&token, id, &changed, Filter::Pending, 8),
        Err(e) if e.code == ErrorCode::StaleAnchor));
    Ok(())
}

#[test]
fn unissued_tokens_and_expired_cursors_never_become_offsets() -> Result<()> {
    let mut cursors = Continuations::default(); let s = summary(); let id = SessionId::new(9);
    assert!(cursors.resolve(&Digest32::ZERO.to_string(), id, &s, Filter::All, 8).is_err());
    let first = cursors.issue(id, &s, "job-000".into(), Filter::All, 8);
    for number in 1..=64 { cursors.issue(id, &s, format!("job-{number:03}"), Filter::All, 8); }
    assert_eq!(cursors.issued.len(), 64); assert_eq!(cursors.order.len(), 64);
    assert!(cursors.resolve(&first, id, &s, Filter::All, 8).is_err());
    Ok(())
}

#[test]
fn pending_filter_includes_preparations_but_excludes_cancelled_and_terminal_effects() -> Result<()> {
    assert!(Filter::Pending.matches(DurableJobState::Prepared));
    assert!(Filter::Pending.matches(DurableJobState::DispatchStarted));
    assert!(Filter::Reconciliation.matches(DurableJobState::Indeterminate));
    assert!(!Filter::Reconciliation.matches(DurableJobState::Prepared));
    for state in [DurableJobState::Applied, DurableJobState::NotApplied,
        DurableJobState::Refused, DurableJobState::CancelledBeforeDispatch] {
        assert!(!Filter::Pending.matches(state)); assert!(Filter::All.matches(state));
    }
    assert_eq!(Filter::Pending.total(&summary()), 2);
    assert!(Filter::parse("unknown-filter").is_err()); Ok(())
}

#[test]
fn error_turn_preserves_active_work_and_never_claims_admission_or_live_anchor() -> Result<()> {
    let c = context(); let s = summary();
    let out = packet("fortress.commit", failure(&error(ErrorCode::BudgetExceeded, "test"), "fortress.commit"),
        TurnView { context:Some(&c),mode:"control",summary:Some(&s),selected:None });
    let value: Value = serde_json::from_str(&out).map_err(|_|error(ErrorCode::InternalInvariantViolation,"invalid JSON"))?;
    assert_eq!(value["agent_turn"]["schema"], "dfmcp.agent_turn/1");
    assert_eq!(value["agent_turn"]["active_work"]["counts"]["reconciliation_required"], 1);
    assert_eq!(value["agent_turn"]["active_work"]["details_omitted"], true);
    assert_eq!(value["agent_turn"]["continuity"]["status"], "indeterminate");
    assert_eq!(value["agent_turn"]["briefing"]["runtime_admitted"], false);
    assert_eq!(value["agent_turn"]["briefing"]["development_mutation_enabled"], false);
    assert!(value["agent_turn"]["briefing"]["admission"].is_null());
    assert!(value["agent_turn"]["anchor"].is_null());
    assert_eq!(value["result"]["error"]["effect_may_have_occurred"], true);
    assert!((out.len() as u64) < BASE_RESERVE); Ok(())
}

#[test]
fn closing_turn_redirects_recovery_to_new_offline_session_without_empty_work_claim() -> Result<()> {
    let c = context();
    let out = packet("fortress.cancel", json!({"ok":true,"closed":true}),
        TurnView {context:Some(&c),mode:"offline",summary:None,selected:None});
    let value: Value = serde_json::from_str(&out).map_err(|_|error(ErrorCode::InternalInvariantViolation,"invalid JSON"))?;
    assert!(value["agent_turn"]["active_work"]["counts"].is_null());
    assert_eq!(value["agent_turn"]["active_work"]["details_omitted"], true);
    assert_eq!(value["agent_turn"]["active_work"]["discovery"]["tool"], "fortress.open_session");
    Ok(())
}
