//! Bounded Agent Turn presentation; retained views are not fresh evidence.
use crate::agent_turn::{AgentPhase, AgentTurnBuilder, ContinuityStatus, recommendation};
use dfmcp_core::{RequestId, SessionId, StateAnchor, WorkBudget};
use serde_json::{Value, json};

pub(super) const RESPONSE_RESERVE: u64 = 128 * 1024;
const TEXT_LIMIT: usize = 60_000;

pub(super) fn compact(view: &Value) -> Value {
    let entries: Vec<Value> = view["frontier"]["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .take(16)
        .map(|entry| {
            json!({"step_id": entry["step_id"].as_u64(),
            "family": bounded(&entry["family"], 64), "phase": bounded(&entry["phase"], 32),
            "idempotency_key": bounded(&entry["idempotency_key"], 128),
            "dispatch_attempted": entry["dispatch_attempted"].as_bool(),
            "preparation_digest": bounded(&entry["preparation_digest"], 64),
            "receipt_digest": bounded(&entry["receipt_digest"], 64)})
        })
        .collect();
    let r = &view["recovery"];
    json!({"recovery":{"plan_digest":bounded(&r["plan_digest"],64),
        "object_digest":bounded(&r["object_digest"],64), "identity_digest":bounded(&r["identity_digest"],64),
        "anchor":compact_anchor(&r["anchor"]),
        "frontier":{"journal_id":bounded(&r["frontier"]["journal_id"],64),
            "head":bounded(&r["frontier"]["head"],64),"authority_tick_floor":r["frontier"]["authority_tick_floor"].as_u64()}},
        "frontier":{"entries":entries,"next_ready":view["frontier"]["next_ready"].as_u64(),"needs_reconciliation":view["frontier"]["needs_reconciliation"].as_bool(),
            "all_applied":view["frontier"]["all_applied"].as_bool(),"review":null},
        "original_goal_proven":false,"current_state_proven":false,"details_omitted":true})
}
fn bounded(value: &Value, max: usize) -> Option<&str> {
    value
        .as_str()
        .filter(|s| s.len() <= max && !s.chars().any(char::is_control))
}
fn compact_anchor(value: &Value) -> Value {
    json!({"fortress_id":bounded(&value["fortress_id"],20),"epoch":value["epoch"].as_u64(),
        "sequence":value["sequence"].as_u64(),"tick":value["tick"].as_u64(),
        "state_hash":bounded(&value["state_hash"],64)})
}
fn anchor_value(a: StateAnchor) -> Value {
    json!({"fortress_id":a.fortress_id.get().to_string(),"epoch":a.cursor.epoch,
        "sequence":a.cursor.sequence,"tick":a.tick.get(),"state_hash":a.state_hash.to_hex()})
}

#[allow(clippy::too_many_arguments)]
fn encode(
    operation: &str,
    session: Option<SessionId>,
    anchor: Option<StateAnchor>,
    mode: Option<&str>,
    request: Option<RequestId>,
    budget: Option<WorkBudget>,
    mut result: Value,
    retained: Option<&Value>,
    verified: bool,
    uncertain: bool,
) -> String {
    let phase = match operation {
        "fortress.open_session" => AgentPhase::Bootstrap,
        "fortress.observe" => AgentPhase::Orient,
        "fortress.plan" => AgentPhase::Propose,
        "fortress.commit" => AgentPhase::Commit,
        "fortress.wait" => AgentPhase::Verify,
        "fortress.cancel" => AgentPhase::Reconcile,
        _ => AgentPhase::Inspect,
    };
    let view = retained.unwrap_or(&Value::Null);
    let entries = view["frontier"]["entries"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let review = if verified {
        view["review"].clone()
    } else {
        Value::Null
    };
    let recommendation = if session.is_none() || result["closed"] == true {
        None
    } else if uncertain {
        Some((
            "fortress_wait",
            "Reconcile the original uncertain key before any new dispatch.",
            false,
            json!({}),
        ))
    } else if !verified {
        None
    } else if !review.is_null() {
        Some((
            "fortress_commit",
            "Confirm the exact current paired review.",
            true,
            json!({"paired_seal":review["paired_seal"],"confirmed":true}),
        ))
    } else if view["frontier"]["next_ready"].is_u64()
        && view["original_plan"]["plan_digest"].is_string()
    {
        Some((
            "fortress_plan",
            "Prepare the next dependency-satisfied original step.",
            false,
            json!({"request":json!({"kind":"step", "original_plan_digest":view["original_plan"]["plan_digest"]}).to_string()}),
        ))
    } else if view["original_plan"].is_null() && view["observation"].is_null() {
        Some((
            "fortress_observe",
            "Acquire the paused canonical observation before planning.",
            false,
            json!({}),
        ))
    } else if view["original_plan"].is_null() {
        Some((
            "fortress_plan",
            "Compile one original labor and finite-order plan against this observation.",
            false,
            json!({}),
        ))
    } else {
        None
    };
    let recommendations = recommendation
        .into_iter()
        .map(|(tool, reason, confirm, mut arguments)| {
            if let Some(id) = session {
                arguments["session_id"] = json!(id.to_string());
            }
            recommendation_fn(tool, reason, confirm, arguments)
        })
        .collect();
    let mut turn = AgentTurnBuilder::new(operation, phase)
        .continuity(if uncertain { ContinuityStatus::Indeterminate } else if verified { ContinuityStatus::Partial } else if retained.is_some() { ContinuityStatus::Stale } else { ContinuityStatus::Bootstrap }, None, None, None)
        .briefing(json!({"runtime":"unadmitted_combined_production_development", "runtime_admitted":false,"mode":mode,
            "retained_state_currently_verified":verified,"current_state_proven":false,"original_goal_proven":false}))
        .active_work(json!({"pending_plans":[],"actions":entries,"obligations":[],"cancellation_drains":[],
            "indeterminate_effects":if uncertain {vec![json!({"state":"unknown","reconcile_original_key":true})]} else {vec![]},
            "publications":[],"confirmations":if review.is_null(){vec![]} else {vec![review]}}))
        .recommendations(recommendations)
        .coverage(json!({"status":"partial","complete_domains":[],"partial_domains":["retained_original_plan"],
            "omitted_domains":["current_goal_completion","physical_output","other_controllers"],"continuation":null}));
    if let Some(id) = session {
        turn = turn.session_id(id.to_string());
    }
    if let Some(id) = request {
        turn = turn.request_id(id.to_string());
    }
    if let Some(a) = anchor {
        turn = turn.anchor(anchor_value(a));
    }
    if let Some(b) = budget {
        turn = turn.budget(json!({"requested":null,"admitted":{"max_wall_millis":b.max_wall_millis,
        "max_bytes":b.max_bytes,"max_output_tokens":b.max_output_tokens,"max_game_ticks":b.max_game_ticks},
        "consumed":{},"remaining":null,"soft_stop_reason":null,"hard_stop_reason":null}));
    }
    let mut agent_turn = turn.build();
    if let Some(briefing) = agent_turn["briefing"].as_object_mut() {
        briefing.remove("admission");
    }
    if let Some(object) = result.as_object_mut() {
        object.insert("agent_turn".into(), agent_turn);
    }
    result.to_string()
}
fn recommendation_fn(tool: &str, reason: &str, confirm: bool, arguments: Value) -> Value {
    recommendation(
        tool,
        tool,
        reason,
        "advance the retained original plan",
        "resolve the next bounded step",
        if confirm { "guarded" } else { "read_only" },
        "not_assessed",
        confirm,
        arguments,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn packet(
    operation: &str,
    session: Option<SessionId>,
    anchor: Option<StateAnchor>,
    mode: Option<&str>,
    request: Option<RequestId>,
    budget: Option<WorkBudget>,
    result: Value,
    retained: Option<&Value>,
    verified: bool,
    uncertain: bool,
) -> (String, bool) {
    let full = encode(
        operation, session, anchor, mode, request, budget, result, retained, verified, uncertain,
    );
    if serde_json::to_string(&full).is_ok_and(|s| s.len() <= TEXT_LIMIT) {
        return (full, false);
    }
    let fallback = json!({"ok":false,"error":{"code":"budget_exceeded","message":"complete response exceeds output limit; local review withdrawn","retryable":false},
        "output_limit_bytes":TEXT_LIMIT,"retained_reference":retained.map(compact),"retained_state_currently_verified":false,
        "automatic_redispatch":false,"current_state_proven":false,"original_goal_proven":false});
    let compacted = retained.map(compact);
    (
        encode(
            operation,
            session,
            anchor,
            mode,
            request,
            budget,
            fallback,
            compacted.as_ref(),
            false,
            uncertain,
        ),
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recommendations_follow_observation_and_numeric_next_step_without_claiming_goal() {
        let original = "a".repeat(64);
        let session = SessionId::new((1u128 << 127) | (24u128 << 57) | 1);
        let mut view = json!({"observation":null,"original_plan":null,"frontier":null});
        let call = |view: &Value, verified| {
            let (raw, overflow) = packet(
                "fortress.query",
                Some(session),
                None,
                None,
                None,
                None,
                json!({"ok":true}),
                Some(view),
                verified,
                false,
            );
            assert!(!overflow);
            serde_json::from_str::<Value>(&raw).expect("packet JSON")
        };
        assert_eq!(
            call(&view, true)["agent_turn"]["recommendations"][0]["tool"],
            "fortress_observe"
        );
        view["observation"] = json!({"seal":"b".repeat(64)});
        assert_eq!(
            call(&view, true)["agent_turn"]["recommendations"][0]["tool"],
            "fortress_plan"
        );
        view["original_plan"] = json!({"plan_digest":original});
        view["frontier"] = json!({"next_ready":0,"review":null,"entries":[]});
        let response = call(&view, true);
        let recommendation = &response["agent_turn"]["recommendations"][0];
        assert_eq!(recommendation["tool"], "fortress_plan");
        assert_eq!(
            recommendation["arguments"]["session_id"],
            session.to_string()
        );
        let request: Value = serde_json::from_str(
            recommendation["arguments"]["request"]
                .as_str()
                .expect("request"),
        )
        .expect("request JSON");
        assert_eq!(request["kind"], "step");
        assert_eq!(request["original_plan_digest"], original);
        assert!(
            call(&view, false)["agent_turn"]["recommendations"]
                .as_array()
                .expect("recommendations")
                .is_empty()
        );
        view["frontier"]["next_ready"] = Value::Null;
        view["frontier"]["all_applied"] = json!(true);
        assert!(
            call(&view, true)["agent_turn"]["recommendations"]
                .as_array()
                .expect("recommendations")
                .is_empty()
        );
        assert_eq!(
            call(&view, true)["agent_turn"]["briefing"]["original_goal_proven"],
            false
        );
        view["review"] = json!({"paired_seal":"c".repeat(64),"native_review":{"detail_index":1}});
        let reviewed = call(&view, true);
        assert_eq!(
            reviewed["agent_turn"]["active_work"]["confirmations"][0]["native_review"]["detail_index"],
            1
        );
        assert_eq!(
            reviewed["agent_turn"]["recommendations"][0]["tool"],
            "fortress_commit"
        );
        let (closed, _) = packet(
            "fortress.cancel",
            Some(session),
            None,
            None,
            None,
            None,
            json!({"ok":true,"closed":true}),
            Some(&view),
            true,
            true,
        );
        let closed: Value = serde_json::from_str(&closed).expect("closed packet");
        assert!(
            closed["agent_turn"]["recommendations"]
                .as_array()
                .expect("recommendations")
                .is_empty()
        );
        let (unbound, _) = packet(
            "fortress.query",
            None,
            None,
            None,
            None,
            None,
            json!({"ok":false}),
            Some(&view),
            true,
            true,
        );
        let unbound: Value = serde_json::from_str(&unbound).expect("unbound packet");
        assert!(
            unbound["agent_turn"]["recommendations"]
                .as_array()
                .expect("recommendations")
                .is_empty()
        );
    }
    #[test]
    fn overflow_withdraws_review_and_keeps_bounded_uncertainty() {
        let retained = json!({"frontier":{"entries":[{"step_id":0, "idempotency_key":"original-key", "family":"workforce",
            "phase":"indeterminate", "dispatch_attempted":true}],"needs_reconciliation":true,
            "review":{"paired_seal":"b".repeat(64)}},
            "recovery":{"plan_digest":"c".repeat(64),"object_digest":"d".repeat(64),
                "identity_digest":"e".repeat(64),"anchor":{"fortress_id":"1","epoch":0,"sequence":1,"tick":10,"state_hash":"f".repeat(64)},
                "frontier":{"journal_id":"1".repeat(64),"head":"2".repeat(64),"authority_tick_floor":10}}});
        let (raw, withdrawn) = packet(
            "fortress.plan",
            None,
            None,
            None,
            None,
            None,
            json!({"ok":true,"payload":"x".repeat(70_000)}),
            Some(&retained),
            true,
            true,
        );
        assert!(withdrawn);
        assert!(serde_json::to_string(&raw).is_ok_and(|s| s.len() <= TEXT_LIMIT));
        let value: Value = serde_json::from_str(&raw).expect("packet JSON");
        assert_eq!(
            value["retained_reference"]["frontier"]["entries"][0]["phase"],
            "indeterminate"
        );
        assert_eq!(
            value["retained_reference"]["frontier"]["entries"][0]["step_id"],
            0
        );
        assert_eq!(
            value["retained_reference"]["frontier"]["entries"][0]["idempotency_key"],
            "original-key"
        );
        assert_eq!(
            value["retained_reference"]["frontier"]["needs_reconciliation"],
            true
        );
        assert!(value["retained_reference"]["frontier"]["review"].is_null());
        assert_eq!(
            value["retained_reference"]["recovery"]["frontier"]["authority_tick_floor"],
            10
        );
        assert_eq!(value["original_goal_proven"], false);
    }
}
