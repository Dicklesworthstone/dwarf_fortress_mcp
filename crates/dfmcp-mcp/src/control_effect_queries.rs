//! Bounded read-only discovery of durable pause effects. This is coordinator
//! evidence, not a fresh observation of the game and never permission to retry.

use dfmcp_adapter::control_effect_journal::{ControlEffectJournal, DurablePauseState, EffectJournalStorage};
use dfmcp_core::{DfmcpError, Digest32, ErrorCode, OperationContext, Result, SessionId};
use serde_json::{Value, json};

use super::{journal_json, packet, record_json, state_name};

pub(super) struct EffectQuery<'a> {
    pub state: &'a str,
    pub limit: u32,
    pub continuation: Option<&'a str>,
    pub max_bytes: Option<u64>,
    pub max_output_tokens: Option<u32>,
}

fn invalid(message: &str) -> DfmcpError { DfmcpError::new(ErrorCode::InvalidRequest, message) }
fn exhausted() -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded,
        "effect query budget cannot fit a complete record and required Agent Turn; increase the budget")
}
fn matches_state(filter: &str, state: DurablePauseState) -> bool {
    match filter {
        "all" => true,
        "nonterminal" => !state.terminal(),
        "reconciliation_required" => state.reconciliation_required(),
        name => name == state_name(state),
    }
}

struct PageIdentity<'a> {
    session: SessionId,
    journal: Digest32,
    head: Digest32,
    filter: &'a str,
}
impl PageIdentity<'_> {
    fn token(&self, offset: u32) -> String {
        let mut bytes = b"dfmcp-control-effect-page/1\0".to_vec();
        bytes.extend_from_slice(&self.session.get().to_be_bytes());
        bytes.extend_from_slice(self.journal.as_bytes());
        bytes.extend_from_slice(self.head.as_bytes());
        bytes.extend_from_slice(&(self.filter.len() as u32).to_be_bytes());
        bytes.extend_from_slice(self.filter.as_bytes());
        bytes.extend_from_slice(&offset.to_be_bytes());
        format!("ce1:{offset:08x}:{}", Digest32::of_bytes(&bytes))
    }

    fn offset(&self, raw: Option<&str>, total: usize) -> Result<usize> {
        let Some(raw) = raw else { return Ok(0); };
        // Fixed ASCII framing makes every subsequent slice bounded and UTF-8 safe.
        if raw.len() != 77 || !raw.is_ascii() || !raw.starts_with("ce1:")
            || raw.as_bytes()[12] != b':'
            || !raw[4..12].bytes().chain(raw[13..].bytes())
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
            return Err(invalid("invalid control effect continuation"));
        }
        let offset = u32::from_str_radix(&raw[4..12], 16)
            .map_err(|_| invalid("invalid control effect continuation offset"))?;
        if offset == 0 || offset as usize >= total || self.token(offset) != raw {
            return Err(DfmcpError::new(ErrorCode::Conflict,
                "effect continuation does not match this session, journal head, filter or result set; restart the listing"));
        }
        Ok(offset as usize)
    }
}

pub(super) fn effects<S: EffectJournalStorage>(journal: &mut ControlEffectJournal<S>,
    context: &OperationContext, query: EffectQuery<'_>) -> Result<Value> {
    if !matches!(query.state, "all" | "nonterminal" | "reconciliation_required" | "prepared"
        | "commit_started" | "indeterminate" | "verified_applied" | "verified_not_applied") {
        return Err(invalid("unknown durable effect state filter"));
    }
    if !(1..=128).contains(&query.limit) || query.max_bytes == Some(0)
        || query.max_output_tokens == Some(0) {
        return Err(invalid("effect limit must be 1..128 and response budgets must be nonzero"));
    }
    let started = std::time::Instant::now();
    let recovery_only = journal.read_only();
    let identity = PageIdentity { session: context.session_id, journal: journal.id(),
        head: journal.head(), filter: query.state };
    let metadata = journal_json(journal);
    let records = journal.records(context)?;
    let mut counts = [0usize; 5];
    let mut matching = 0usize;
    for record in records.clone() {
        if started.elapsed().as_millis() >= u128::from(context.budget.max_wall_millis) {
            return Err(DfmcpError::new(ErrorCode::BudgetExceeded, "effect enumeration exceeded its work deadline"));
        }
        let index = match record.state {
            DurablePauseState::Prepared => 0,
            DurablePauseState::CommitStarted => 1,
            DurablePauseState::Indeterminate => 2,
            DurablePauseState::VerifiedApplied => 3,
            DurablePauseState::VerifiedNotApplied => 4,
        };
        counts[index] += 1;
        matching += usize::from(matches_state(query.state, record.state));
    }
    let offset = identity.offset(query.continuation, matching)?;
    let token_limit = query.max_output_tokens.unwrap_or(context.budget.max_output_tokens)
        .min(context.budget.max_output_tokens);
    let byte_limit = query.max_bytes.unwrap_or(context.budget.max_bytes)
        .min(context.budget.max_bytes).min(u64::from(token_limit) * 4);
    let limit = query.limit.min(context.budget.max_entities) as usize;
    let make_payload = |rows: &[Value]| {
        let next = offset + rows.len();
        json!({"schema":"dfmcp.control_effects/1", "ok":true,
            "session_id":context.session_id.to_string(),
            "source":"durable_control_journal", "recovery_only":recovery_only,
            "current_freshness_proven":false, "mutation_dispatched":false,
            "reconciliation_performed":false, "commit_permitted":false,
            "filter":query.state, "order":"idempotency_key_ascending",
            "total_effects":records.len(), "matched_effects":matching,
            "state_counts":{"prepared":counts[0], "commit_started":counts[1],
                "indeterminate":counts[2], "verified_applied":counts[3], "verified_not_applied":counts[4]},
            "reconciliation_required_effects":counts[1] + counts[2],
            "returned_effects":rows.len(), "effects":rows,
            "truncated":next < matching,
            "continuation":(next < matching).then(|| identity.token(next as u32)),
            "durable_effect_journal":metadata,
            "response_budget":{"max_bytes":byte_limit, "max_output_tokens":token_limit,
                "token_estimate":"ceil_utf8_bytes_div_4", "includes_agent_turn":true}})
    };
    let fits = |value: &Value| packet("fortress.query", value.clone(), recovery_only).len() as u64 <= byte_limit;
    let mut rows = Vec::new();
    let mut payload = make_payload(&rows);
    if !fits(&payload) { return Err(exhausted()); }
    for record in records.clone().filter(|r| matches_state(query.state, r.state)).skip(offset).take(limit) {
        if started.elapsed().as_millis() >= u128::from(context.budget.max_wall_millis) {
            return Err(DfmcpError::new(ErrorCode::BudgetExceeded, "effect rendering exceeded its work deadline"));
        }
        rows.push(record_json(record));
        let candidate = make_payload(&rows);
        if !fits(&candidate) { rows.pop(); break; }
        payload = candidate;
    }
    if rows.is_empty() && offset < matching { return Err(exhausted()); }
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
    use dfmcp_adapter::control_effect_journal::EffectTailRecovery;
    use dfmcp_core::{Capability, CapabilityGrant, CapabilityScope, FortressId, GameTick,
        ObservationCursor, RequestId, RiskTier, StateAnchor, WorkBudget};

    #[derive(Default)]
    struct Memory(Cursor<Vec<u8>>);
    impl Read for Memory { fn read(&mut self, b: &mut [u8]) -> io::Result<usize> { self.0.read(b) } }
    impl Write for Memory {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> { self.0.write(b) }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }
    impl Seek for Memory { fn seek(&mut self, p: SeekFrom) -> io::Result<u64> { self.0.seek(p) } }
    impl EffectJournalStorage for Memory {
        fn sync(&mut self) -> io::Result<()> { Ok(()) }
        fn truncate(&mut self, n: u64) -> io::Result<()> { self.0.get_mut().truncate(n as usize); Ok(()) }
    }
    fn context() -> OperationContext {
        OperationContext { session_id: SessionId::new(91), request_id: RequestId::new(1),
            anchor: StateAnchor { fortress_id: FortressId::NIL, cursor: ObservationCursor::ORIGIN,
                tick: GameTick(0), state_hash: Digest32::ZERO },
            budget: WorkBudget::default(), cancellation_requested: false,
            grants: vec![CapabilityGrant { capability: Capability::ControlClock, max_risk: RiskTier::Reversible,
                scope: CapabilityScope::default(), expires_at_tick: None, remaining_uses: None }] }
    }
    fn query(state: &str, limit: u32) -> EffectQuery<'_> {
        EffectQuery { state, limit, continuation: None, max_bytes: None, max_output_tokens: None }
    }
    fn fixture() -> Result<ControlEffectJournal<Memory>> {
        let mut j = ControlEffectJournal::open(Memory::default(), &context(), true, 7, EffectTailRecovery::Refuse)?;
        for key in ["c-indeterminate", "b-started", "a-prepared", "d-applied", "e-not-applied"] {
            let plan = Digest32::of_bytes(key.as_bytes());
            j.record_prepared(key.into(), plan, true, 10, 7, [1; 16], &context())?;
            if key != "a-prepared" { j.begin_commit(key, plan, 7, &context())?; }
            if key == "c-indeterminate" { j.mark_indeterminate(key, plan, &context())?; }
            if key == "d-applied" || key == "e-not-applied" {
                let applied = key == "d-applied";
                j.record_reconciliation(key, plan, 7, true, applied, applied, 11,
                    applied.then(|| Digest32::of_bytes(b"receipt")), &context())?;
            }
        }
        Ok(j)
    }

    #[test]
    fn pages_are_sorted_replayable_and_bound_to_the_full_packet_budget() -> Result<()> {
        let mut j = fixture()?;
        let head = j.head();
        let first = effects(&mut j, &context(), query("all", 1))?;
        assert_eq!(first["effects"][0]["idempotency_key"], "a-prepared");
        assert_eq!(first["reconciliation_required_effects"], 2);
        assert_eq!(first["total_effects"], 5);
        assert_eq!(effects(&mut j, &context(), query("all", 1))?, first);
        let token = first["continuation"].as_str().ok_or_else(|| invalid("missing test continuation"))?;
        let next = EffectQuery { continuation: Some(token), ..query("all", 2) };
        let second = effects(&mut j, &context(), next)?;
        assert_eq!(second["effects"][0]["idempotency_key"], "b-started");
        assert_eq!(second["effects"][1]["idempotency_key"], "c-indeterminate");
        assert!(packet("fortress.query", second.clone(), false).len() as u64
            <= second["response_budget"]["max_bytes"].as_u64().ok_or_else(exhausted)?);
        assert_eq!(j.head(), head);
        Ok(())
    }

    #[test]
    fn continuations_reject_other_sessions_filters_heads_and_tampering() -> Result<()> {
        let mut j = fixture()?;
        let first = effects(&mut j, &context(), query("all", 1))?;
        let token = first["continuation"].as_str().ok_or_else(|| invalid("missing test continuation"))?;
        let mut other = context(); other.session_id = SessionId::new(92);
        assert!(matches!(effects(&mut j, &other, EffectQuery { continuation: Some(token), ..query("all", 1) }),
            Err(e) if e.code == ErrorCode::Conflict));
        assert!(matches!(effects(&mut j, &context(), EffectQuery { continuation: Some(token), ..query("nonterminal", 1) }),
            Err(e) if e.code == ErrorCode::Conflict));
        let changed = format!("ce1:00000002:{}", &token[13..]);
        assert!(effects(&mut j, &context(), EffectQuery { continuation: Some(&changed), ..query("all", 1) }).is_err());
        j.record_prepared("f-new".into(), Digest32::of_bytes(b"new"), true, 10, 7, [1;16], &context())?;
        assert!(matches!(effects(&mut j, &context(), EffectQuery { continuation: Some(token), ..query("all", 1) }),
            Err(e) if e.code == ErrorCode::Conflict));
        Ok(())
    }

    #[test]
    fn filters_counts_and_empty_results_preserve_uncertainty() -> Result<()> {
        let mut j = fixture()?;
        for (state, expected) in [("all",5), ("nonterminal",3), ("reconciliation_required",2),
            ("prepared",1), ("commit_started",1), ("indeterminate",1), ("verified_applied",1), ("verified_not_applied",1)] {
            let value = effects(&mut j, &context(), query(state, 128))?;
            assert_eq!(value["matched_effects"], expected);
            assert_eq!(value["current_freshness_proven"], false);
            assert_eq!(value["commit_permitted"], false);
            assert_eq!(value["mutation_dispatched"], false);
        }
        let mut empty = ControlEffectJournal::open(Memory::default(), &context(), true, 7, EffectTailRecovery::Refuse)?;
        let value = effects(&mut empty, &context(), query("all", 1))?;
        assert_eq!(value["effects"], json!([]));
        assert_eq!(value["truncated"], false);
        assert!(value["continuation"].is_null());
        Ok(())
    }

    #[test]
    fn zero_progress_tiny_budgets_invalid_input_and_lost_authority_are_refused() -> Result<()> {
        let mut j = fixture()?;
        let head = j.head();
        for bytes in [1, 32, 128] {
            assert!(matches!(effects(&mut j, &context(), EffectQuery { max_bytes: Some(bytes), ..query("all", 1) }),
                Err(e) if e.code == ErrorCode::BudgetExceeded));
        }
        assert!(effects(&mut j, &context(), query("unknown", 1)).is_err());
        assert!(effects(&mut j, &context(), query("all", 0)).is_err());
        assert!(effects(&mut j, &context(), query("all", 129)).is_err());
        let mut denied = context(); denied.grants.clear();
        assert!(matches!(effects(&mut j, &denied, query("all", 1)), Err(e) if e.code == ErrorCode::CapabilityDenied));
        denied = context(); denied.cancellation_requested = true;
        assert!(matches!(effects(&mut j, &denied, query("all", 1)), Err(e) if e.code == ErrorCode::CancellationRequested));
        assert_eq!(j.head(), head);
        Ok(())
    }
}
