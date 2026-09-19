//! Request-owned retrospective monitoring. Reuse the actual foreground Watch
//! transition machine, but never register a watch or read/write either store.
use super::*;

pub(crate) const MAX_REPLAY_RECORDS: u64 = 32;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    condition: Condition,
    failure_condition: Option<Condition>,
    deadline_tick: u64,
    poll_interval_ticks: Option<u64>,
    stable_observations: Option<u32>,
}

pub(crate) struct Replay {
    watch: Watch,
    first_record: u64,
    last_record: u64,
    visited: u64,
    evaluated: u64,
    failed: bool,
    first_terminal_record: Option<u64>,
    changes: Vec<Value>,
    identity: Digest32,
    budget: counts::EvaluationBudget,
}
impl Replay {
    /// `binding` is supplied by verified history, never as an MCP authority.
    /// It binds the archive incarnation and both exact endpoint record digests.
    pub(crate) fn new(input: &Value, first: StateAnchor, first_record: u64,
        last_record: u64, binding: Digest32, context: &OperationContext) -> Result<Self> {
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if first.fortress_id != context.anchor.fortress_id || binding == Digest32::ZERO {
            return Err(failure(ErrorCode::StaleAnchor, "historical monitor names another observation universe"));
        }
        if first_record == 0 || last_record.checked_sub(first_record)
            .is_none_or(|n| n >= MAX_REPLAY_RECORDS) {
            return Err(bounded("historical monitor replays one to 32 consecutive retained observations"));
        }
        validate_input(input)?;
        let input: Input = serde_json::from_value(input.clone())
            .map_err(|_| invalid("invalid historical monitor definition"))?;
        let definition = Definition { key: "historical-replay".into(), label: "Historical monitor replay".into(),
            condition: input.condition, failure_condition: input.failure_condition,
            deadline_tick: input.deadline_tick, poll_interval_ticks: input.poll_interval_ticks.unwrap_or(1),
            stable_observations: input.stable_observations.unwrap_or(2) };
        validate_definition(&definition)?;
        if definition.deadline_tick <= first.tick.0 {
            return Err(invalid("historical monitor deadline must follow its first selected capture"));
        }
        let identity = digest(&json!({"domain":"dfmcp-historical-watch-replay/1",
            "binding":binding.to_string(),"first_record":first_record,"last_record":last_record,
            "basis":anchor(first),"definition":definition}))?;
        let watch = Watch { handle:format!("replay:{identity}"), definition, created_at:first,
            last_seen:first, last_sample_tick:None, streak:0, samples:0, status:Status::Waiting,
            evaluation:Value::Null, evidence_digest:identity, recovery:None };
        Ok(Self { watch, first_record, last_record, visited:0, evaluated:0, failed:false,
            first_terminal_record:None, changes:Vec::new(), identity,
            budget:counts::EvaluationBudget::new(context.budget.max_wall_millis) })
    }

    pub(crate) fn advance(&mut self, record: u64, snapshot: &WorldSnapshot,
        context: &OperationContext) -> Result<()> {
        if self.failed { return Err(invalid("historical monitor replay is already failed")); }
        let result = self.advance_inner(record, snapshot, context);
        if result.is_err() { self.failed = true; }
        result
    }

    fn advance_inner(&mut self, record: u64, snapshot: &WorldSnapshot,
        context: &OperationContext) -> Result<()> {
        // Never substitute an archived tick for the authority context.
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        self.budget.charge()?;
        if self.first_record.checked_add(self.visited) != Some(record) || record > self.last_record {
            return Err(invalid("historical monitor observations must be complete and consecutive"));
        }
        if snapshot.fortress_id != context.anchor.fortress_id || snapshot.fortress_id != self.watch.created_at.fortress_id {
            return Err(failure(ErrorCode::StaleAnchor, "historical monitor snapshot differs from its bound fortress"));
        }
        if snapshot.graph.entities.len() > context.budget.max_entities as usize {
            return Err(bounded("historical monitor snapshot exceeds its current entity allowance"));
        }
        if !snapshot.hash_is_valid() {
            return Err(failure(ErrorCode::CorruptLedger, "historical monitor snapshot hash is invalid"));
        }
        let initial = self.visited == 0;
        if initial && snapshot.anchor() != self.watch.created_at {
            return Err(failure(ErrorCode::StaleAnchor, "historical monitor first capture differs from its bound basis"));
        }
        self.visited += 1;
        if self.watch.status.terminal() { return Ok(()); }
        let before = self.watch.status;
        self.watch.advance_bounded(snapshot, initial, &mut self.budget)?;
        self.evaluated += 1;
        if initial || self.watch.status != before {
            self.changes.push(json!({"record":record,"anchor":anchor(self.watch.last_seen),
                "status":self.watch.status.text(),"stable_observations":self.watch.streak,
                "sample_count":self.watch.samples,"evidence_digest":self.watch.evidence_digest.to_string()}));
        }
        if self.watch.status.terminal() { self.first_terminal_record = Some(record); }
        self.budget.check()
    }

    /// No retained handle is returned. The replay identifier cannot poll, cancel,
    /// release, recover, prepare or commit live work. It is evidence identity only.
    pub(crate) fn finish(self, details: bool, context: &OperationContext) -> Result<Value> {
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        self.budget.check()?;
        if self.failed || self.visited != self.last_record - self.first_record + 1 {
            return Err(invalid("historical monitor cannot publish an incomplete replay"));
        }
        let proof = digest(&json!({"domain":"dfmcp-historical-watch-replay-result/1",
            "identity":self.identity.to_string(),"terminal_record":self.first_terminal_record,
            "visited":self.visited,"evaluated":self.evaluated,
            "monitor_evidence":self.watch.evidence_digest.to_string()}))?;
        let mut value = json!({"schema":"dfmcp.query.result/1","kind":"historical_watch_replay",
            "replay_id":self.identity.to_string(),"evidence_digest":proof.to_string(),
            "monitor_evidence_digest":self.watch.evidence_digest.to_string(),
            "status":self.watch.status.text(),"terminal":self.watch.status.terminal(),
            "first_terminal_record":self.first_terminal_record,
            "basis":anchor(self.watch.created_at),"last_evaluated_anchor":anchor(self.watch.last_seen),
            "records_verified":self.visited,"records_evaluated":self.evaluated,
            "records_after_terminal":self.visited-self.evaluated,
            "sample_count":self.watch.samples,"stable_observations":self.watch.streak,
            "required_stable_observations":self.watch.definition.stable_observations,
            "poll_interval_ticks":self.watch.definition.poll_interval_ticks,
            "deadline_tick":self.watch.definition.deadline_tick,
            "condition_truth":self.watch.evaluation.get("condition"),
            "failure_condition_truth":self.watch.evaluation.get("failure_condition"),
            "reason":self.watch.evaluation.get("reason"),
            "status_change_count":self.changes.len(),"detail":if details {"evidence"} else {"summary"},
            "status_changes_complete":details,"truncated":false,"continuation":null,
            "historical":true,"live":false,"current_freshness_proven":false,
            "watch_registered":false,"watch_evaluated":false,"native_captures":0,
            "mutation_dispatched":false,"original_watch_activity_proven":false,
            "continuous_between_observations":false,"game_action_completion_proven":false,
            "interpretation":"Retrospective execution of the foreground monitor over the selected archived samples. The first sample acts as registration; normal cadence, stability, failure, deadline and identity rules apply. This neither proves an actual watch was active then nor creates, advances or authorizes live work."});
        if details {
            value["status_changes"] = json!(self.changes);
            value["evaluation"] = self.watch.evaluation;
        }
        let maximum = context.budget.max_bytes.min(u64::from(context.budget.max_output_tokens) * 4);
        if value.to_string().len() as u64 > maximum {
            return Err(bounded("complete historical monitor evidence exceeds the response budget"));
        }
        self.budget.check()?;
        Ok(value)
    }
}

#[cfg(test)]
#[path = "query_watch_replay_tests.rs"]
mod tests;
