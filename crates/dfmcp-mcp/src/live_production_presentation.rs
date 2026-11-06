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
        "current_parent_goal":null,
        "parent_goal_observation_fenced":view["parent_goal_observation_fenced"] == true,
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
    let parent_fenced = view["parent_goal_observation_fenced"] == true;
    // The joined sample has a different state hash from its inventory child.
    // Present it only at the exact composite anchor, linked to that child; a
    // retained receipt, inventory achievement or top-level flag cannot fill it.
    let candidate = &view["current_parent_goal"];
    let parent_goal = if verified
        && !uncertain
        && !parent_fenced
        && mode != Some("offline")
        && candidate["scope"] == "complete_original_labor_and_inventory_goal"
        && matches!(
            candidate["current_truth"].as_str(),
            Some("true" | "false" | "unknown")
        )
        && anchor.is_some_and(|a| candidate["anchor"] == anchor_value(a))
        && candidate["selected_anchor"].is_object()
        && candidate["selected_anchor"] == selected_goal["anchor"]
        && ["fortress_id", "epoch", "sequence", "tick"]
            .iter()
            .all(|&field| candidate["anchor"][field] == candidate["selected_anchor"][field])
        && candidate["original_goal_proven"]
            == (candidate["current_truth"] == "true" && selected_proven)
        && candidate["original_goal_proven"] == view["original_goal_proven"]
    {
        candidate.clone()
    } else {
        Value::Null
    };
    let parent_proven = parent_goal["original_goal_proven"] == true;
    let parent_truth = parent_goal["current_truth"].as_str();
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
    } else if !verified || parent_proven || parent_fenced {
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
            "Take one coherent sample of the original labor and inventory goal; additional stability samples require separately managed game-time progress.",
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
    let mut partial_domains = vec!["retained_original_plan"];
    if selected_goal.is_object() {
        partial_domains.push("selected_original_inventory_goal");
    }
    let mut omitted_domains = vec![
        "physical_output",
        "production_attribution",
        "other_controllers",
    ];
    if parent_goal.is_object() {
        partial_domains.push("current_parent_terminal_predicate");
    } else {
        omitted_domains.insert(0, "current_parent_goal");
    }
    let uncertainty = if parent_fenced {
        vec![json!({
            "scope":"complete_original_labor_and_inventory_goal",
            "epistemic_status":"unknown", "state":"observation_fenced",
            "retryable_in_session":false,
            "finding":"Current parent observation is fenced after a source-lineage failure.",
            "resolution":"Retain the original recovery reference and reconcile any uncertain effect under its original key. Inspect custody with fortress_doctor and resolve the source lineage before reopening; further goal waits in this session cannot clear this fence."
        })]
    } else {
        match parent_truth {
            Some("false") => vec![json!({
                "scope":"complete_original_labor_and_inventory_goal",
                "epistemic_status":"contradicted", "anchor":parent_goal["anchor"],
                "finding":"The complete original terminal predicate is false at this sample anchor.",
                "resolution":"Review the current parent predicate and original plan before choosing any corrective action; inventory polling alone does not establish labor truth.",
                "selected_inventory_goal_proven_at_anchor":selected_proven
            })],
            Some("unknown") => vec![json!({
                "scope":"complete_original_labor_and_inventory_goal",
                "epistemic_status":"unknown", "anchor":parent_goal["anchor"],
                "finding":"Current evidence does not establish the complete original terminal predicate.",
                "resolution":"Review source coverage and current authority before another bounded sample.",
                "selected_inventory_goal_proven_at_anchor":selected_proven
            })],
            None if selected_goal.is_object() => vec![json!({
                "scope":"complete_original_labor_and_inventory_goal",
                "epistemic_status":"unknown",
                "finding":"No current joined labor and inventory sample is published.",
                "resolution":"A fresh coherent parent sample is required; selected inventory history and historical labor receipts are insufficient."
            })],
            _ => Vec::new(),
        }
    };
    let mut turn = AgentTurnBuilder::new(operation, phase)
        .continuity(if uncertain { ContinuityStatus::Indeterminate } else if verified { ContinuityStatus::Partial } else if retained.is_some() { ContinuityStatus::Stale } else { ContinuityStatus::Bootstrap }, None, None, None)
        .briefing(json!({"runtime":"unadmitted_combined_production_development", "runtime_admitted":false,"mode":mode,
            "retained_state_currently_verified":verified,"current_state_proven":false,"original_goal_proven":parent_proven,
            "current_parent_goal_truth":parent_truth,
            "parent_goal_observation_fenced":parent_fenced,
            "selected_inventory_goal_proven_at_anchor":selected_proven}))
        .active_work(json!({"pending_plans":[],"actions":entries,
            "obligations":if selected_goal.is_null() {vec![]} else {vec![selected_goal]},
            "current_parent_goal":parent_goal,"cancellation_drains":[],
            "indeterminate_effects":if uncertain {vec![json!({"state":"unknown","reconcile_original_key":true})]} else {vec![]},
            "publications":[],"confirmations":if review.is_null(){vec![]} else {vec![review]}}))
        .recommendations(recommendations)
        .uncertainty(uncertainty)
        .coverage(json!({"status":"partial","complete_domains":[],"partial_domains":partial_domains,
            "omitted_domains":omitted_domains,"continuation":null}));
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
    use dfmcp_core::{Digest32, FortressId, GameTick, ObservationCursor};

    fn joined_goal_view(truth: &str, stable: bool) -> (StateAnchor, Value) {
        let anchor = StateAnchor {
            fortress_id: FortressId::new(7),
            tick: GameTick(130),
            cursor: ObservationCursor {
                epoch: 1,
                sequence: 5,
            },
            state_hash: Digest32::of_bytes(b"joined parent"),
        };
        let selected_anchor = StateAnchor {
            state_hash: Digest32::of_bytes(b"inventory child"),
            ..anchor
        };
        let proven = truth == "true" && stable;
        let view = json!({"anchor":anchor_value(anchor),
            "original_plan":{"plan_digest":"a".repeat(64)},
            "frontier":{"all_applied":true,"next_ready":null,"entries":[]},
            "review":null,"original_goal_proven":proven,
            "selected_inventory_goal":{"scope":"selected_original_order_step",
                "step_id":1,"kind":"observed_inventory_minimum","item_type":"BED",
                "minimum_stack_units":1,"current_truth":"true","proven_at_anchor":stable,
                "anchor":anchor_value(selected_anchor),
                "parent_terminal_truth":"unknown","original_goal_proven":false,
                "history":{"present":true,"fenced":false},"terminal_failure":false,
                "first_satisfied_anchor":if stable {anchor_value(selected_anchor)} else {Value::Null}},
            "current_parent_goal":{"scope":"complete_original_labor_and_inventory_goal",
                "anchor":anchor_value(anchor),"selected_anchor":anchor_value(selected_anchor),
                "current_truth":truth,"original_goal_proven":proven,
                "source_digest":"b".repeat(64),"workforce_source_digest":"c".repeat(64)}});
        (anchor, view)
    }

    fn joined_packet(
        anchor: StateAnchor,
        view: &Value,
        mode: &str,
        verified: bool,
        uncertain: bool,
    ) -> Value {
        let (raw, overflow) = packet(
            "fortress.wait",
            Some(SessionId::new((1u128 << 127) | (24u128 << 57) | 1)),
            Some(anchor),
            Some(mode),
            None,
            None,
            json!({"ok":verified}),
            Some(view),
            verified,
            uncertain,
        );
        assert!(!overflow);
        serde_json::from_str(&raw).expect("joined packet")
    }

    #[test]
    fn joined_parent_truth_requires_original_inventory_stability_and_preserves_both_anchors() {
        for stable in [false, true] {
            let (anchor, view) = joined_goal_view("true", stable);
            let value = joined_packet(anchor, &view, "control", true, false);
            let turn = &value["agent_turn"];
            assert_eq!(turn["briefing"]["original_goal_proven"], stable);
            assert_eq!(turn["briefing"]["current_parent_goal_truth"], "true");
            assert_eq!(turn["briefing"]["current_state_proven"], false);
            assert_eq!(turn["briefing"]["runtime_admitted"], false);
            assert_eq!(turn["anchor"], view["current_parent_goal"]["anchor"]);
            assert_eq!(
                turn["active_work"]["current_parent_goal"],
                view["current_parent_goal"]
            );
            assert_eq!(
                turn["active_work"]["obligations"][0]["anchor"],
                view["selected_inventory_goal"]["anchor"]
            );
            assert_ne!(
                turn["anchor"]["state_hash"],
                turn["active_work"]["obligations"][0]["anchor"]["state_hash"]
            );
            assert_eq!(turn["coverage"]["complete_domains"], json!([]));
            assert!(
                turn["coverage"]["partial_domains"]
                    .as_array()
                    .expect("domains")
                    .contains(&json!("current_parent_terminal_predicate"))
            );
            if stable {
                assert_eq!(turn["recommendations"], json!([]));
            } else {
                assert_eq!(turn["recommendations"][0]["tool"], "fortress_wait");
            }
        }
    }

    #[test]
    fn satisfied_inventory_with_false_or_unknown_parent_stops_inventory_only_polling() {
        for (truth, epistemic) in [("false", "contradicted"), ("unknown", "unknown")] {
            let (anchor, view) = joined_goal_view(truth, true);
            let value = joined_packet(anchor, &view, "control", true, false);
            let turn = &value["agent_turn"];
            assert_eq!(
                turn["briefing"]["selected_inventory_goal_proven_at_anchor"],
                true
            );
            assert_eq!(turn["briefing"]["original_goal_proven"], false);
            assert_eq!(turn["briefing"]["current_parent_goal_truth"], truth);
            assert_eq!(
                turn["active_work"]["current_parent_goal"]["current_truth"],
                truth
            );
            assert_eq!(turn["recommendations"], json!([]));
            assert_eq!(turn["uncertainty"][0]["epistemic_status"], epistemic);
            assert_eq!(
                turn["uncertainty"][0]["anchor"],
                view["current_parent_goal"]["anchor"]
            );
        }
    }

    #[test]
    fn missing_unverified_recovered_or_mismatched_parent_never_promotes_inventory_history() {
        let (anchor, original) = joined_goal_view("true", true);
        for case in 0..10 {
            let mut view = original.clone();
            let mut verified = true;
            let mut uncertain = false;
            let mut mode = "control";
            match case {
                0 => view["current_parent_goal"] = Value::Null,
                1 => verified = false,
                2 => uncertain = true,
                3 => mode = "offline",
                4 => view["current_parent_goal"]["anchor"]["state_hash"] = json!("d".repeat(64)),
                5 => view["current_parent_goal"]["selected_anchor"]["tick"] = json!(131),
                6 => view["selected_inventory_goal"]["proven_at_anchor"] = json!(false),
                7 => view["current_parent_goal"]["current_truth"] = json!("false"),
                8 => view["original_goal_proven"] = json!(false),
                _ => {
                    // Recovery can retain first achievement without a current
                    // sample. Replaying history never recreates the join.
                    view["current_parent_goal"] = Value::Null;
                    view["selected_inventory_goal"]["current_truth"] = Value::Null;
                    view["selected_inventory_goal"]["anchor"] = Value::Null;
                    view["selected_inventory_goal"]["proven_at_anchor"] = json!(false);
                    mode = "reconcile";
                }
            }
            let value = joined_packet(anchor, &view, mode, verified, uncertain);
            let turn = &value["agent_turn"];
            assert_eq!(
                turn["briefing"]["original_goal_proven"], false,
                "case {case}"
            );
            assert!(
                turn["briefing"]["current_parent_goal_truth"].is_null(),
                "case {case}"
            );
            assert!(
                turn["active_work"]["current_parent_goal"].is_null(),
                "case {case}"
            );
            assert!(
                turn["coverage"]["omitted_domains"]
                    .as_array()
                    .expect("omissions")
                    .contains(&json!("current_parent_goal"))
            );
        }
    }

    #[test]
    fn compact_and_overflow_withdraw_joined_proof_but_preserve_selected_achievement() {
        let (anchor, view) = joined_goal_view("true", true);
        let compacted = compact(&view);
        assert!(compacted["current_parent_goal"].is_null());
        assert_eq!(compacted["original_goal_proven"], false);
        assert_eq!(
            compacted["selected_inventory_goal"]["proven_at_anchor"],
            false
        );
        assert_eq!(
            compacted["selected_inventory_goal"]["first_satisfied_anchor"],
            view["selected_inventory_goal"]["first_satisfied_anchor"]
        );
        let (raw, overflow) = packet(
            "fortress.wait",
            Some(SessionId::new((1u128 << 127) | (24u128 << 57) | 1)),
            Some(anchor),
            Some("control"),
            None,
            None,
            json!({"ok":true,"payload":"x".repeat(70_000)}),
            Some(&view),
            true,
            false,
        );
        assert!(overflow);
        let value: Value = serde_json::from_str(&raw).expect("overflow packet");
        assert_eq!(value["original_goal_proven"], false);
        assert!(value["retained_reference"]["current_parent_goal"].is_null());
        assert_eq!(
            value["agent_turn"]["briefing"]["original_goal_proven"],
            false
        );
        assert!(value["agent_turn"]["active_work"]["current_parent_goal"].is_null());
        assert_eq!(
            value["retained_reference"]["selected_inventory_goal"]["first_satisfied_anchor"],
            view["selected_inventory_goal"]["first_satisfied_anchor"]
        );
        assert_eq!(value["agent_turn"]["recommendations"], json!([]));
    }
    #[test]
    fn fenced_parent_observation_preserves_inspection_guidance_without_goal_waits() {
        let (anchor, mut view) = joined_goal_view("true", true);
        view["parent_goal_observation_fenced"] = json!(true);
        for verified in [false, true] {
            let value = joined_packet(anchor, &view, "control", verified, false);
            let turn = &value["agent_turn"];
            assert_eq!(turn["briefing"]["parent_goal_observation_fenced"], true);
            assert_eq!(turn["briefing"]["original_goal_proven"], false);
            assert!(turn["active_work"]["current_parent_goal"].is_null());
            assert_eq!(turn["recommendations"], json!([]));
            assert_eq!(turn["uncertainty"][0]["state"], "observation_fenced");
            assert_eq!(turn["uncertainty"][0]["retryable_in_session"], false);
            assert!(
                turn["uncertainty"][0]["resolution"]
                    .as_str()
                    .expect("recovery guidance")
                    .contains("fortress_doctor")
            );
        }
        // Clearing current proof after a failure must not hide the fence.
        view["current_parent_goal"] = Value::Null;
        view["original_goal_proven"] = json!(false);
        view["selected_inventory_goal"]["proven_at_anchor"] = json!(false);
        assert_eq!(compact(&view)["parent_goal_observation_fenced"], true);
        let compacted = compact(&view);
        let turn = joined_packet(anchor, &compacted, "control", false, false);
        assert_eq!(
            turn["agent_turn"]["briefing"]["parent_goal_observation_fenced"],
            true
        );
        assert_eq!(
            turn["agent_turn"]["uncertainty"][0]["retryable_in_session"],
            false
        );
        assert_eq!(turn["agent_turn"]["recommendations"], json!([]));
        let (raw, overflow) = packet(
            "fortress.wait",
            Some(SessionId::new((1u128 << 127) | (24u128 << 57) | 1)),
            Some(anchor),
            Some("control"),
            None,
            None,
            json!({"ok":true,"payload":"x".repeat(70_000)}),
            Some(&view),
            true,
            false,
        );
        assert!(overflow);
        let value: Value = serde_json::from_str(&raw).expect("fenced overflow packet");
        assert_eq!(
            value["retained_reference"]["parent_goal_observation_fenced"],
            true
        );
        assert_eq!(
            value["agent_turn"]["briefing"]["parent_goal_observation_fenced"],
            true
        );
        assert_eq!(value["agent_turn"]["recommendations"], json!([]));
        // Uncertain effects still require same-key reconciliation. This is not
        // another ordinary parent acquisition and takes priority over the fence.
        let uncertain = joined_packet(anchor, &view, "reconcile", false, true);
        assert_eq!(
            uncertain["agent_turn"]["recommendations"][0]["tool"],
            "fortress_wait"
        );
        assert!(
            uncertain["agent_turn"]["recommendations"][0]["reason"]
                .as_str()
                .expect("reconciliation guidance")
                .contains("original uncertain key")
        );
    }

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
