//! Pure full-packet publication for laboratory query-baseline transactions.
//! All session facts are supplied while the caller owns its session. Rendering
//! acquires no lock, reads no source and updates no presentation cache.

use crate::agent_turn::{
    AgentPhase, AgentTurnBuilder, ContinuityStatus, ObservationProfile, uncertainty,
};
use dfmcp_core::{DfmcpError, ErrorCode, Result, WorkBudget};
use serde_json::{Value, json};

pub(super) struct TemporalResponseProjection {
    pub session_id: String,
    pub request_id: String,
    pub anchor: Value,
    pub paused: bool,
    pub budget: WorkBudget,
    pub active_work: Value,
    pub objectives: Value,
}

fn invariant(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InternalInvariantViolation, message)
}

fn exhausted(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

impl TemporalResponseProjection {
    fn maximum_bytes(&self) -> Result<usize> {
        self.budget.validate()?;
        usize::try_from(
            self.budget
                .max_bytes
                .min(u64::from(self.budget.max_output_tokens).saturating_mul(4)),
        )
        .map_err(|_| exhausted("temporal response budget cannot be represented"))
    }

    /// Reserve the complete current game-work packet and largest comparison
    /// metadata before the baseline engine fills a page. The helper separately
    /// reserves the complete candidate baseline catalog, including a new capture.
    pub(super) fn result_byte_budget(&self) -> Result<usize> {
        if !self.active_work.is_object() || !self.anchor.is_object() {
            return Err(invariant(
                "laboratory temporal presentation state is invalid",
            ));
        }
        let sample = json!({
            "kind":"changes", "basis":self.anchor,
            "baseline":format!("qb1:{}", "x".repeat(64)),
            "anchor_advanced":true, "change_count":u64::MAX,
            "returned":u64::MAX, "basis_result_digest":"x".repeat(64),
            "target_result_digest":"x".repeat(64),
            "truncated":true, "continuation":"x".repeat(128),
            "_query_baseline_work":[],
        });
        let reserve = self
            .render(sample)?
            .len()
            .checked_add(512)
            .ok_or_else(|| exhausted("temporal Agent Turn reservation overflow"))?;
        self.maximum_bytes()?
            .checked_sub(reserve)
            .filter(|bytes| *bytes > 0)
            .ok_or_else(|| {
                exhausted(
                    "required laboratory game work and Agent Turn leave no temporal result budget",
                )
            })
    }

    /// Return the complete success or refuse it. No row/field clipping or other
    /// fallible publication step is permitted after the baseline root changes.
    pub(super) fn finish(&self, payload: Value) -> Result<String> {
        if !payload.is_object()
            || payload["anchor"] != self.anchor
            || payload["current_anchor"] != self.anchor
            || payload["session_id"] != self.session_id
            || payload["mode"] != "temporal"
            || payload["ok"] != true
        {
            return Err(invariant(
                "temporal result differs from its complete session anchor",
            ));
        }
        if payload
            .get("_condition_watch_work")
            .is_some_and(|work| work.as_array().is_none_or(|work| !work.is_empty()))
        {
            return Err(invariant(
                "laboratory baseline publication cannot omit condition-watch work",
            ));
        }
        let output = self.render(payload)?;
        if output.len() > self.maximum_bytes()? {
            return Err(exhausted(
                "complete temporal query and current game work exceed the response budget",
            ));
        }
        Ok(output)
    }

    fn render(&self, mut payload: Value) -> Result<String> {
        let baseline_work = payload
            .as_object_mut()
            .and_then(|object| object.remove("_query_baseline_work"))
            .filter(|work| work.as_array().is_some_and(|work| work.len() <= 8))
            .ok_or_else(|| {
                invariant("temporal publication requires the complete bounded baseline catalog")
            })?;
        let mut active_work = self.active_work.clone();
        if !active_work.is_object() {
            return Err(invariant("current laboratory game work is not an object"));
        }
        active_work["query_baselines"] = baseline_work;
        if let Some(object) = payload.as_object_mut() {
            object.remove("_condition_watch_work");
        }
        let comparison = payload["kind"] == "changes";
        let interval = comparison && payload["anchor_advanced"] == true;
        let truncated = payload["truncated"] == true;
        let continuity = if interval || truncated {
            ContinuityStatus::Partial
        } else {
            ContinuityStatus::Heartbeat
        };
        let basis = if comparison {
            payload["basis"].clone()
        } else {
            self.anchor.clone()
        };
        let changes = if comparison {
            vec![
                json!({"kind":"query_endpoint_comparison", "baseline":payload["baseline"],
                "basis":payload["basis"], "target":payload["anchor"],
                "change_count":payload["change_count"], "returned":payload["returned"],
                "epistemic_state":"inferred",
                "evidence":[payload["basis_result_digest"],payload["target_result_digest"]],
                "detail_path":"changes", "intermediate_history_proven":false}),
            ]
        } else {
            Vec::new()
        };
        let list = json!({"tool":"fortress.query","arguments":{
            "session_id":self.session_id,
            "mode":json!({"mode":"temporal","request":{
                "schema":"dfmcp.query/1", "query":{"kind":"baselines"}
            }}).to_string()
        }});
        let maximum = self.maximum_bytes()?;
        payload["request_id"] = json!(self.request_id);
        payload["query_help"] = list.clone();
        payload["query_budget"] = json!({
            "maximum_response_bytes":maximum,
            "max_output_tokens":self.budget.max_output_tokens,
            "max_wall_millis":self.budget.max_wall_millis,
        });
        let admitted = json!({
            "max_wall_millis":self.budget.max_wall_millis,
            "max_game_ticks":self.budget.max_game_ticks,
            "max_entities":self.budget.max_entities,
            "max_bytes":self.budget.max_bytes,
            "max_output_tokens":self.budget.max_output_tokens,
            "max_actions":self.budget.max_actions,
            "maximum_response_bytes":maximum,
        });
        let coverage = json!({
            "status":"partial",
            "complete_domains":["laboratory.source_anchor","laboratory.session_query_baseline_catalog"],
            "partial_domains":[{"domain":"laboratory.selected_endpoint_projection",
                "reason":"only selected canonical endpoint rows are compared; unselected facts and intermediate history are not established"}],
            "omitted_domains":["live_dwarf_fortress_state"],
            "absence_proof_scope":[], "continuation":payload["continuation"],
            "query_coverage":payload["coverage"],
            "query_temporal_coverage":{"status":"endpoint_comparison_only",
                "intermediate_observations_retained":false,
                "continuous_between_observations":false,
                "eligible_as_mutation_evidence":false},
        });
        let packet = AgentTurnBuilder::new("fortress.query", AgentPhase::Inspect)
            .session_id(self.session_id.clone())
            .request_id(self.request_id.clone())
            .turn_id(format!("lab-temporal-turn-{}",self.request_id))
            .anchor(self.anchor.clone())
            .continuity(continuity, Some(basis), None, None)
            .profile(ObservationProfile::Tactical)
            .briefing(json!({
                "runtime":"laboratory_reference_world", "adapter":"memory",
                "paused":self.paused, "objectives":self.objectives,
                "query_execution":"foreground_only", "query_game_effect":"none",
                "baseline_storage":"bounded_process_local", "baselines_durable":false,
                "baseline_mutation_authority":false, "live_game_qualification":false,
            }))
            .changes(changes)
            .active_work(active_work)
            .affordances(vec![json!({
                "affordance_id":"query-baselines", "tool":"fortress.query",
                "intent_family":"query", "risk":"read_only",
                "reversibility":"not_applicable", "enabled":true,
                "disabled_reason":null, "known_preconditions":[],
                "unverified_preconditions":[],
                "checkpoint_policy":"registered_policy",
                "confirmation_policy":"registered_policy",
                "estimated_cost":{"actions":null,"bridge_bytes":null,
                    "wall_millis":null,"game_ticks":null},
                "arguments":list["arguments"],
            })])
            .uncertainty(vec![
                uncertainty(
                    "query-temporal-history",
                    "unknown",
                    "Retained selections compare endpoints; intermediate changes and continuous truth are not observed.",
                    "Endpoint equality does not establish an unchanged interval or mutation success.",
                    Some("fortress.query"),
                    list["arguments"].clone(),
                ),
                uncertainty(
                    "query-baseline-future-availability",
                    "unknown",
                    "Baseline availability after process restart or session disposal is not retained; restore invalidates the prior epoch.",
                    "These bounded process-local selections do not provide durable monitoring or cross-epoch history.",
                    Some("fortress.query"),
                    list["arguments"].clone(),
                ),
            ])
            .coverage(coverage)
            .budget(json!({"requested":null,"admitted":admitted,
                "consumed":{"canonical_source":payload["source_work"],
                    "native_captures":0,"game_ticks":0},
                "remaining":null,"soft_stop_reason":null,"hard_stop_reason":null}))
            .references(vec![json!({"kind":"laboratory_world",
                "state_hash":self.anchor["state_hash"],"anchor":self.anchor})])
            .attach(payload);
        Ok(packet)
    }
}

#[cfg(test)]
#[path = "lab_temporal_response_tests.rs"]
mod tests;
