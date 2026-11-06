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
    json!({"observation_tick_floor":view["observation_tick_floor"].as_u64(),
        "recovery":{"plan_digest":bounded(&r["plan_digest"],64),
        "object_digest":bounded(&r["object_digest"],64), "identity_digest":bounded(&r["identity_digest"],64),
        "goal_history_present":r["goal_history_present"].as_bool(),
        "anchor":compact_anchor(&r["anchor"]),
        "frontier":{"journal_id":bounded(&r["frontier"]["journal_id"],64),
            "head":bounded(&r["frontier"]["head"],64),"authority_tick_floor":r["frontier"]["authority_tick_floor"].as_u64()}},
        "frontier":{"entries":entries,"next_ready":view["frontier"]["next_ready"].as_u64(),"needs_reconciliation":view["frontier"]["needs_reconciliation"].as_bool(),
            "all_applied":view["frontier"]["all_applied"].as_bool(),"review":null},
        "selected_inventory_goal":compact_goal(&view["selected_inventory_goal"]),
        "original_goal_proven":false,"current_state_proven":false,"details_omitted":true})
}
fn compact_goal(value: &Value) -> Value {
    if !value.is_object() {
        return Value::Null;
    }
    json!({"scope":"selected_original_order_step",
        "step_id":value["step_id"].as_u64(),"kind":bounded(&value["kind"],64),
        "item_type":bounded(&value["item_type"],16),
        "minimum_stack_units":value["minimum_stack_units"].as_u64(),
        "first_satisfied_anchor":if value["first_satisfied_anchor"].is_object() {
            compact_anchor(&value["first_satisfied_anchor"])
        } else {Value::Null},
        "history":{"present":value["history"]["present"].as_bool(),
            "fenced":value["history"]["fenced"].as_bool()},
        "current_truth":null,"proven_at_anchor":false,"original_goal_proven":false,
        "production_attribution":"unknown"})
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
    let selected_goal = if verified && !uncertain {
        view["selected_inventory_goal"].clone()
    } else {
        Value::Null
    };
    let selected_proven = selected_goal["proven_at_anchor"] == true;
    let recommendation = if session.is_none() || result["closed"] == true {
        None
    } else if uncertain && mode == Some("offline") {
        Some((
            "fortress_cancel",
            "Retain the original recovery reference, close this offline session, and reopen in reconcile mode; closing does not cancel the effect.",
            false,
            json!({"scope":"session"}),
        ))
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
    } else if mode != Some("offline")
        && view["frontier"]["all_applied"] == true
        && selected_goal.is_object()
        && !selected_proven
        && selected_goal["terminal_failure"] != true
        && selected_goal["history"]["fenced"] != true
    {
        Some((
            "fortress_wait",
            "Take one fresh sample of the selected original inventory goal; additional stability samples require separately managed game-time progress.",
            false,
            json!({}),
        ))
    } else if view["original_plan"].is_null()
        && (view["observation"].is_null() || view["observation"]["workforce_choices"].is_null())
    {
        Some((
            "fortress_observe",
            "Acquire the paused canonical roster and advisory workforce choices before planning.",
            false,
            json!({}),
        ))
    } else if view["original_plan"].is_null()
        && mode == Some("control")
        && view["labor_control_enabled"] == true
        && view["production_control_enabled"] == true
        && view["observation"]["workforce_choices"]["options"]
            .as_array()
            .is_some_and(|v| !v.is_empty())
    {
        Some((
            "fortress_plan",
            "Compile one original labor and finite-order plan against this observation.",
            false,
            json!({}),
        ))
    } else {
        None
    };
    let mut recommendations = recommendation
        .into_iter()
        .map(|(tool, reason, confirm, mut arguments)| {
            if let Some(id) = session {
                arguments["session_id"] = json!(id.to_string());
            }
            recommendation_fn(tool, reason, confirm, arguments)
        })
        .collect::<Vec<_>>();
    // This suggestion requests a new capture; it never joins pages into a
    // canonical snapshot or carries mutation/preparation authority.
    if verified
        && !uncertain
        && mode != Some("offline")
        && view["original_plan"].is_null()
        && let Some(id) = session
        && view["observation"]["workforce_choices"]["next_observation_request"].is_object()
    {
        recommendations.push(recommendation_fn(
            "fortress_observe",
            "Inspect the next bounded labor choices for these same citizens with a fresh observation; use its new seal for planning.",
            false,
            json!({"session_id":id.to_string(),
                "request":view["observation"]["workforce_choices"]["next_observation_request"].to_string()}),
        ));
    }
    let mut turn = AgentTurnBuilder::new(operation, phase)
        .continuity(if uncertain { ContinuityStatus::Indeterminate } else if verified { ContinuityStatus::Partial } else if retained.is_some() { ContinuityStatus::Stale } else { ContinuityStatus::Bootstrap }, None, None, None)
        .briefing(json!({"runtime":"unadmitted_combined_production_development", "runtime_admitted":false,"mode":mode,
            "retained_state_currently_verified":verified,"current_state_proven":false,"original_goal_proven":false,
            "selected_inventory_goal_proven_at_anchor":selected_proven}))
        .active_work(json!({"pending_plans":[],"actions":entries,
            "obligations":if selected_goal.is_null() {vec![]} else {vec![selected_goal]},"cancellation_drains":[],
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
    fn selected_inventory_proof_guides_polling_without_promoting_parent_success() {
        let session = SessionId::new((1u128 << 127) | (24u128 << 57) | 1);
        let mut view = json!({"original_plan":{"plan_digest":"a".repeat(64)},
            "frontier":{"all_applied":true,"next_ready":null,"entries":[]},
            "review":null,"selected_inventory_goal":{"scope":"selected_original_order_step",
                "step_id":1,"kind":"observed_inventory_minimum","item_type":"BED",
                "minimum_stack_units":1,"current_truth":"true","proven_at_anchor":false,
                "history":{"present":true,"fenced":false},"terminal_failure":false}});
        let call = |view: &Value, mode: &str, verified: bool| {
            let (raw, overflow) = packet(
                "fortress.wait",
                Some(session),
                None,
                Some(mode),
                None,
                None,
                json!({"ok":verified}),
                Some(view),
                verified,
                false,
            );
            assert!(!overflow);
            serde_json::from_str::<Value>(&raw).expect("valid packet")
        };
        let pending = call(&view, "control", true);
        assert_eq!(
            pending["agent_turn"]["recommendations"][0]["tool"],
            "fortress_wait"
        );
        assert_eq!(
            pending["agent_turn"]["active_work"]["obligations"][0]["step_id"],
            1
        );
        view["selected_inventory_goal"]["proven_at_anchor"] = json!(true);
        let proved = call(&view, "control", true);
        assert_eq!(proved["agent_turn"]["recommendations"], json!([]));
        assert_eq!(
            proved["agent_turn"]["briefing"]["selected_inventory_goal_proven_at_anchor"],
            true
        );
        assert_eq!(
            proved["agent_turn"]["briefing"]["original_goal_proven"],
            false
        );
        assert_eq!(
            proved["agent_turn"]["briefing"]["current_state_proven"],
            false
        );
        let failed = call(&view, "control", false);
        assert_eq!(
            failed["agent_turn"]["briefing"]["selected_inventory_goal_proven_at_anchor"],
            false
        );
        assert_eq!(
            failed["agent_turn"]["active_work"]["obligations"],
            json!([])
        );
        view["selected_inventory_goal"]["proven_at_anchor"] = json!(false);
        assert_eq!(
            call(&view, "offline", true)["agent_turn"]["recommendations"],
            json!([])
        );
        for field in ["terminal_failure", "fenced"] {
            let mut stopped = view.clone();
            if field == "fenced" {
                stopped["selected_inventory_goal"]["history"][field] = json!(true);
            } else {
                stopped["selected_inventory_goal"][field] = json!(true);
            }
            assert_eq!(
                call(&stopped, "control", true)["agent_turn"]["recommendations"],
                json!([])
            );
        }
    }

    #[test]
    fn compact_inventory_recovery_retains_custody_and_withdraws_current_proof() {
        let view = json!({"recovery":{"goal_history_present":true},
            "selected_inventory_goal":{"scope":"selected_original_order_step","step_id":1,
                "kind":"observed_inventory_minimum","item_type":"BED","minimum_stack_units":1,
                "current_truth":"true","proven_at_anchor":true,
                "history":{"present":true,"fenced":false},
                "first_satisfied_anchor":{"fortress_id":"7","epoch":1,"sequence":5,
                    "tick":130,"state_hash":"f".repeat(64)}}});
        let retained = compact(&view);
        assert_eq!(retained["recovery"]["goal_history_present"], true);
        assert_eq!(
            retained["selected_inventory_goal"]["first_satisfied_anchor"]["tick"],
            130
        );
        assert_eq!(
            retained["selected_inventory_goal"]["current_truth"],
            Value::Null
        );
        assert_eq!(
            retained["selected_inventory_goal"]["proven_at_anchor"],
            false
        );
        assert_eq!(retained["original_goal_proven"], false);
    }

    #[test]
    fn later_choice_pages_are_explicit_fresh_reads_and_withdrawn_on_error() {
        let session = SessionId::new((1u128 << 127) | (24u128 << 57) | 1);
        let next = json!({"units":["43"],"option_offset":8});
        let view = json!({"original_plan":null,"labor_control_enabled":true,
            "production_control_enabled":true,"observation":{"seal":"a".repeat(64),
                "workforce_choices":{"options":[{"labor":"MINE"}],
                    "next_observation_request":next}}});
        for (verified, uncertain, expected) in
            [(true, false, 1), (false, false, 0), (true, true, 0)]
        {
            let (raw, overflow) = packet(
                "fortress.query",
                Some(session),
                None,
                Some("control"),
                None,
                None,
                json!({"ok":verified}),
                Some(&view),
                verified,
                uncertain,
            );
            assert!(!overflow);
            let response: Value = serde_json::from_str(&raw).expect("packet JSON");
            let suggestions = response["agent_turn"]["recommendations"]
                .as_array()
                .expect("recommendations");
            let pages = suggestions
                .iter()
                .filter(|item| item["tool"] == "fortress_observe")
                .collect::<Vec<_>>();
            assert_eq!(pages.len(), expected);
            if verified && !uncertain {
                assert_eq!(suggestions[0]["tool"], "fortress_plan");
                let arguments = &pages[0]["arguments"];
                assert_eq!(arguments["session_id"], session.to_string());
                let request: Value = serde_json::from_str(
                    arguments["request"]
                        .as_str()
                        .expect("closed observation request"),
                )
                .expect("observation request JSON");
                assert_eq!(request, next);
            }
        }
    }

    #[test]
    fn recommendations_follow_observation_and_numeric_next_step_without_claiming_goal() {
        let original = "a".repeat(64);
        let session = SessionId::new((1u128 << 127) | (24u128 << 57) | 1);
        let mut view = json!({"observation":null,"original_plan":null,"frontier":null,
            "labor_control_enabled":true,"production_control_enabled":true});
        let call = |view: &Value, verified| {
            let (raw, overflow) = packet(
                "fortress.query",
                Some(session),
                None,
                Some("control"),
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
        view["observation"] = json!({"seal":"b".repeat(64),
            "workforce_choices":{"options":[{"labor":"MINE","enable_candidates":["43"]}]}});
        assert_eq!(
            call(&view, true)["agent_turn"]["recommendations"][0]["tool"],
            "fortress_plan"
        );
        view["labor_control_enabled"] = json!(false);
        assert_eq!(
            call(&view, true)["agent_turn"]["recommendations"],
            json!([])
        );
        view["labor_control_enabled"] = json!(true);
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
    fn offline_uncertainty_offers_close_and_reconcile_not_a_nonprogressing_wait() {
        let session = SessionId::new((1u128 << 127) | (24u128 << 57) | 1);
        let view = json!({"frontier":{"needs_reconciliation":true,"entries":[{"step_id":0,"phase":"indeterminate"}]}});
        let (raw, overflow) = packet(
            "fortress.query",
            Some(session),
            None,
            Some("offline"),
            None,
            None,
            json!({"ok":true}),
            Some(&view),
            true,
            true,
        );
        assert!(!overflow);
        let value: Value = serde_json::from_str(&raw).expect("packet JSON");
        let next = &value["agent_turn"]["recommendations"][0];
        assert_eq!(next["tool"], "fortress_cancel");
        assert_eq!(next["arguments"]["scope"], "session");
        assert_eq!(next["arguments"]["session_id"], session.to_string());
        assert!(
            next["reason"]
                .as_str()
                .expect("reason")
                .contains("reconcile")
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
