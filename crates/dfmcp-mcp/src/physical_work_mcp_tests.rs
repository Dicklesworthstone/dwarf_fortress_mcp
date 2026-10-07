//! Goal receipts are historical proofs; physical work has its own lifetime.
//! These regressions cross the agent-facing plan/commit/wait/cancel boundary.
use super::*;
use crate::agent_facade as facade;
use dfmcp_adapter::ActionReceipt;
use dfmcp_core::{MapCoord, MapCuboid, StepId};
use dfmcp_intent::{DigMode, ObligationSpec, derive_step_idempotency_key, effects};
use dfmcp_world::{Fact, FactSource, Value as WorldValue};
use serde_json::Value as Json;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn parsed(raw: &str) -> TestResult<Json> {
    Ok(serde_json::from_str(raw)?)
}

fn fixture_error(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InternalInvariantViolation, message)
}

fn in_session<T>(
    session_id: &str,
    body: impl FnOnce(&mut LabSession) -> Result<T>,
) -> TestResult<T> {
    let session = resolve_session(Some(session_id.to_owned()))?;
    Ok(with_session(
        &session,
        || Err(fixture_error("physical-work fixture session unavailable")),
        body,
    )?)
}

fn open_session(
    selector: &str,
    shared: bool,
    scenario: Option<&str>,
    clock: bool,
) -> TestResult<String> {
    let mut capabilities = vec![
        ("observe", "read_only"),
        ("query", "read_only"),
        ("plan", "reversible"),
        ("checkpoint", "guarded"),
        ("doctor", "read_only"),
        ("designate", "guarded"),
        ("configure_labor", "reversible"),
        ("configure_production", "reversible"),
    ];
    if clock {
        capabilities.push(("control_clock", "reversible"));
    }
    let opened = parsed(&facade::fortress_open_session(
        Some(false),
        Some(selector.to_owned()),
        Some(
            capabilities
                .into_iter()
                .map(|(capability, risk)| (capability.to_owned(), risk.to_owned()))
                .collect(),
        ),
        None,
        Some(2_000),
        None,
        None,
        Some(8_192),
        Some(16),
        scenario.map(str::to_owned),
        shared.then_some(true),
        None,
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    Ok(opened["session_id"]
        .as_str()
        .ok_or("session_id missing")?
        .to_owned())
}

fn open(selector: &str) -> TestResult<String> {
    open_session(selector, false, Some("starter_fortress"), true)
}

fn plan(session: &str, actions: &str) -> TestResult<Json> {
    let planned = parsed(&facade::fortress_plan(
        Some(session.to_owned()),
        Some("physical work regression".to_owned()),
        None,
        Some(actions.to_owned()),
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    Ok(planned)
}

fn commit(session: &str, planned: &Json) -> TestResult<Json> {
    parsed(&facade::fortress_commit(
        Some(session.to_owned()),
        planned["plan_digest"]
            .as_str()
            .ok_or("plan_digest missing")?
            .to_owned(),
    ))
}

fn plan_and_commit(session: &str, actions: &str) -> TestResult<Json> {
    let committed = commit(session, &plan(session, actions)?)?;
    assert_eq!(committed["ok"], true, "{committed}");
    Ok(committed)
}

fn wait(session: &str, ticks: u64) -> TestResult<Json> {
    let waited = parsed(&facade::fortress_wait(
        Some(session.to_owned()),
        Some(ticks),
    ))?;
    assert_eq!(waited["ok"], true, "{waited}");
    Ok(waited)
}

fn cancel(session: &str, mode: &str, scope: &str) -> TestResult<Json> {
    parsed(&facade::fortress_cancel(
        Some(session.to_owned()),
        Some(mode.to_owned()),
        Some(scope.to_owned()),
    ))
}

fn request_unpause(session: &str) -> TestResult<Json> {
    let planned = parsed(&facade::fortress_plan(
        Some(session.to_owned()),
        None,
        Some(false),
        None,
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    commit(session, &planned)
}

fn snapshot(session: &str) -> TestResult<WorldSnapshot> {
    in_session(session, |guard| Ok(guard.adapter.snapshot().clone()))
}

fn last_effect(session: &str) -> TestResult<(ActionId, EntityId)> {
    in_session(session, |guard| {
        let action = guard
            .last_action
            .ok_or_else(|| fixture_error("last action missing"))?;
        let step = guard
            .adapter
            .action_step(action)
            .ok_or_else(|| fixture_error("sealed step missing"))?;
        Ok((action, effects::created_entity_id(&step.idempotency_key, 0)))
    })
}

fn receipt(session: &str, action: ActionId) -> TestResult<ActionReceipt> {
    in_session(session, |guard| {
        guard
            .adapter
            .action_receipt(action)
            .cloned()
            .ok_or_else(|| fixture_error("action receipt missing"))
    })
}

fn fields(session: &str, kind: &str, entity: EntityId) -> TestResult<Json> {
    let queried = parsed(&facade::fortress_query(
        Some(session.to_owned()),
        Some(json!({"mode": "entities", "kind": kind}).to_string()),
    ))?;
    assert_eq!(queried["ok"], true, "{queried}");
    let id = entity.to_string();
    Ok(queried["rows"]
        .as_array()
        .ok_or("entity rows missing")?
        .iter()
        .find(|row| row["entity_id"].as_str() == Some(id.as_str()))
        .ok_or("expected entity missing")?["fields"]
        .clone())
}

fn drink(session: &str) -> TestResult<u64> {
    fields(session, effects::STOCK_LEDGER_KIND, EntityId::new(5001))?[effects::STOCK_DRINK_FIELD]
        .as_u64()
        .ok_or_else(|| "known drink stock missing".into())
}

fn order(name: &str, job: &str, amount: u32, conditions: Json) -> String {
    json!([{"action": {
        "kind": "create_work_order", "name": name, "job_token": job,
        "amount": amount, "conditions": conditions,
    }}])
    .to_string()
}

fn set_brewer(session: &str, enabled: bool) -> TestResult<Json> {
    plan_and_commit(
        session,
        &json!([{"action": {
            "kind": "set_labor", "units": ["1003"], "labor": "BREW", "enabled": enabled,
        }}])
        .to_string(),
    )
}

fn assert_certificate(drained: &Json) {
    assert_eq!(drained["ok"], true, "{drained}");
    assert_eq!(drained["drain_progress"]["quiescent"], true, "{drained}");
    assert_eq!(drained["drain_progress"]["remaining_work"], 0);
    assert_eq!(drained["drain_progress"]["remaining_nonterminal"], 0);
    assert_eq!(
        drained["finalize_certificate"]["digest"]
            .as_str()
            .map(str::len),
        Some(64),
        "{drained}"
    );
}

// The public action grammar chooses the reference completion proof. Core plans
// also support explicit proofs: here the designation's existence is sufficient
// for the caller's goal, while excavating its thirty tiles takes 300 ticks. Seal
// such a plan normally, then exercise the real MCP commit/clock/cancel handlers.
// This process-local fixture never rebases or journals the substitute raw source.
fn early_dig_plan(session: &str, intent_number: u128) -> TestResult<String> {
    in_session(session, |guard| {
        let snapshot = guard.adapter.snapshot();
        let area = MapCuboid::new(MapCoord::new(1, 3, 10), MapCoord::new(10, 5, 10))?;
        let action = Action::DesignateDig {
            area,
            mode: DigMode::Mine,
        };
        let id = IntentId::new(intent_number);
        let key = derive_step_idempotency_key(id, snapshot.anchor(), StepId::new(0), &action);
        let proof = Predicate::EntityExists(effects::created_entity_id(&key, 0));
        let summary = "prove designation acceptance before excavation completes".to_owned();
        let intent = Intent {
            id,
            anchor: snapshot.anchor(),
            summary: summary.clone(),
            terminal_condition: proof.clone(),
            constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
            requested_actions: vec![RequestedAction {
                action,
                preconditions: vec![],
                postconditions: vec![proof.clone()],
                compensation: None,
                obligation: Some(ObligationSpec {
                    terminal: proof,
                    failure: None,
                    deadline_tick: snapshot
                        .tick
                        .checked_add(10)
                        .ok_or_else(|| fixture_error("fixture deadline overflow"))?,
                    poll_interval_ticks: 1,
                    stable_for_observations: 1,
                }),
                depends_on: vec![],
            }],
        };
        let plan = StaticPlanner::default().prepare_laboratory(
            snapshot,
            &intent,
            &context_for(guard, guard.next_request_id),
        )?;
        let digest = plan.digest.to_hex();
        guard.pending = Some(PendingPlan {
            plan,
            digest: digest.clone(),
            source: PlanSource::Actions {
                summary,
                raw: r#"[{"action":{"kind":"designate_dig","min":[1,3,10],"max":[10,5,10],"mode":"mine"}}]"#.to_owned(),
            },
        });
        Ok(digest)
    })
}

#[test]
fn failed_goal_keeps_work_open_and_cleanup_preserves_its_receipt() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("73610")?;
    set_brewer(&session, false)?;
    let planned = plan(
        &session,
        &order("blocked brewer", "BREW_DRINK", 1, json!([])),
    )?;
    assert_eq!(planned["forecast"]["available"], true, "{planned}");
    assert_eq!(planned["forecast"]["predicted_complete"], false);
    let committed = commit(&session, &planned)?;
    assert_eq!(committed["ok"], true, "{committed}");
    let (action, entity) = last_effect(&session)?;
    let failed = wait(&session, 200)?;
    assert_eq!(failed["commit_state"], "Failed", "{failed}");
    assert_eq!(failed["work_state"]["state"], "active");
    assert_eq!(failed["open_actions_remaining"], 1);
    let original = receipt(&session, action)?;
    assert_eq!(original.state, CommitState::Failed);
    assert_eq!(
        fields(&session, "work_order", entity)?["amount_remaining"],
        1
    );

    // Revoking the original effect capability after commit must also revoke
    // cleanup authority. Restoring the same negotiated grants permits the stop.
    let grants = in_session(&session, |guard| {
        let original = guard.grants.clone();
        guard
            .grants
            .retain(|grant| grant.capability != Capability::ConfigureProduction);
        Ok(original)
    })?;
    let before = snapshot(&session)?;
    let denied = cancel(&session, "stop_future_steps", "plan")?;
    assert_eq!(denied["ok"], false, "{denied}");
    assert_eq!(denied["error"]["code"], "capability_denied");
    assert_eq!(snapshot(&session)?, before);
    assert_eq!(receipt(&session, action)?, original);
    in_session(&session, |guard| {
        guard.grants = grants;
        Ok(())
    })?;

    let drained = cancel(&session, "stop_future_steps", "plan")?;
    assert_certificate(&drained);
    assert_eq!(drained["drain_progress"]["already_terminal"], 1);
    assert_eq!(drained["drain_progress"]["terminal_work_stopped"], 1);
    assert_eq!(drained["steps"][0]["before"], "Failed");
    assert_eq!(drained["steps"][0]["after"], "Failed");
    assert_eq!(
        drained["steps"][0]["proof_receipt_digest"],
        original.adapter_receipt_digest.to_hex()
    );
    assert_eq!(
        drained["steps"][0]["physical_drain"]["proof_receipt_preserved"],
        true
    );
    assert_eq!(receipt(&session, action)?, original);

    let stock = drink(&session)?;
    set_brewer(&session, true)?;
    wait(&session, 100)?;
    let stopped = fields(&session, "work_order", entity)?;
    assert_eq!(stopped["status"], "cancelled");
    assert_eq!(stopped["amount_remaining"], 1);
    assert_eq!(
        drink(&session)?,
        stock,
        "stopped work must not revive when labor returns"
    );
    assert_eq!(receipt(&session, action)?, original);
    Ok(())
}

#[test]
fn verified_goal_keeps_shared_excavation_fenced_beyond_lease_expiry() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open_session("73611", true, Some("starter_fortress"), true)?;
    let other = open_session("73611", true, None, true)?;
    let digest = early_dig_plan(&owner, 73611)?;
    let committed = parsed(&facade::fortress_commit(Some(owner.clone()), digest))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let (action, designation) = last_effect(&owner)?;
    let early = wait(&owner, 1)?;
    assert_eq!(early["commit_state"], "Verified", "{early}");
    assert_eq!(early["work_state"]["state"], "active");
    assert_eq!(early["open_actions_remaining"], 1);
    let original = receipt(&owner, action)?;
    let expired = wait(&owner, 20)?;
    assert_eq!(expired["work_state"]["state"], "active", "{expired}");
    assert_eq!(
        fields(&owner, effects::DIG_DESIGNATION_KIND, designation)?["tiles_remaining"],
        28
    );
    let overlap =
        r#"[{"action":{"kind":"designate_dig","min":[10,5,10],"max":[10,5,10],"mode":"mine"}}]"#;
    let refused = commit(&other, &plan(&other, overlap)?)?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "conflict");
    assert!(
        refused["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("physical work")),
        "{refused}"
    );

    assert_certificate(&cancel(&owner, "stop_future_steps", "plan")?);
    assert_eq!(receipt(&owner, action)?, original);
    plan_and_commit(&other, overlap)?;
    wait(&other, 10)?;
    assert_eq!(
        fields(&owner, effects::DIG_DESIGNATION_KIND, designation)?["tiles_remaining"],
        28
    );
    assert_eq!(receipt(&owner, action)?, original);
    Ok(())
}

#[test]
fn session_drain_includes_older_work_after_a_new_plan_finishes() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("73612")?;
    plan_and_commit(
        &session,
        &order("older production", "BREW_DRINK", 20, json!([])),
    )?;
    let (older, entity) = last_effect(&session)?;
    let newer = plan_and_commit(
        &session,
        r#"[{"action":{"kind":"set_labor","units":["1001"],"labor":"MASON","enabled":true}}]"#,
    )?;
    assert_eq!(newer["actions"][0]["state"], "Verified", "{newer}");
    assert_certificate(&cancel(&session, "stop_future_steps", "plan")?);
    let still_running = in_session(&session, |guard| guard.adapter.action_work_state(older))?;
    assert!(matches!(still_running, EffectWorkState::Active { .. }));
    let drained = cancel(&session, "stop_future_steps", "session")?;
    assert_certificate(&drained);
    assert_eq!(drained["drain_progress"]["actions_total"], 1);
    assert_eq!(drained["steps"][0]["action_id"], older.to_string());
    let stock = drink(&session)?;
    wait(&session, 100)?;
    assert_eq!(drink(&session)?, stock);
    let stopped = fields(&session, "work_order", entity)?;
    assert_eq!(stopped["status"], "cancelled");
    assert_eq!(stopped["amount_remaining"], 20);
    Ok(())
}

#[test]
fn observation_discontinuity_cannot_turn_untrusted_status_into_a_drain_certificate() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("73613")?;
    plan_and_commit(
        &session,
        &order("unresolved ownership", "BREW_DRINK", 20, json!([])),
    )?;
    let (action, entity) = last_effect(&session)?;
    let discarded_handle = in_session(&session, |guard| {
        let mut injected = guard.adapter.snapshot().clone();
        let record = injected
            .graph
            .entities
            .get_mut(&entity)
            .ok_or_else(|| fixture_error("created order missing"))?;
        record.fields.insert(
            effects::STATUS_FIELD.to_owned(),
            Fact::known(
                WorldValue::Text(effects::STATUS_CANCELLED.to_owned()),
                injected.tick,
                FactSource::AgentAssertion("uncertified cancellation claim".to_owned()),
                Digest32::ZERO,
            ),
        );
        injected.cursor = injected
            .cursor
            .checked_next()
            .ok_or_else(|| fixture_error("fixture cursor overflow"))?;
        injected.refresh_hash();
        // Injection deliberately invalidates adapter handles. The MCP retains
        // the original plan IDs and must report this loss of ownership, rather
        // than adopting an entity whose compatibility status says cancelled.
        guard.adapter.inject_snapshot(injected)?;
        Ok(guard.adapter.action_receipt(action).is_none())
    })?;
    assert!(discarded_handle);
    let before = snapshot(&session)?;
    let refused = cancel(&session, "stop_future_steps", "plan")?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "conflict");
    assert!(refused["finalize_certificate"].is_null());
    assert_eq!(refused["drain_progress"]["quiescent"], false);
    assert!(
        refused["drain_progress"]["remaining_work"]
            .as_u64()
            .is_some_and(|remaining| remaining > 0),
        "{refused}"
    );
    let still_unresolved =
        in_session(&session, |guard| {
            Ok(guard.open_actions.contains(&action)
                && guard.adapter.action_work_state(action).is_err())
        })?;
    assert!(
        still_unresolved,
        "a refused cleanup must retain unresolved work"
    );
    assert_eq!(
        snapshot(&session)?,
        before,
        "unresolved ownership must refuse before mutation"
    );
    Ok(())
}

#[test]
fn conditional_forecast_and_actual_work_share_stock_gates_and_bounded_batches() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("73614")?;
    let conditions = json!([{"kind":"item_count_below", "item_token":"DRINK", "threshold":40}]);
    let planned = plan(
        &session,
        &order("wait for thirst", "BREW_DRINK", 1, conditions),
    )?;
    assert_eq!(planned["forecast"]["available"], true, "{planned}");
    assert_eq!(planned["forecast"]["predicted_complete"], false);
    assert_eq!(planned["forecast"]["steps"][0]["predicted_state"], "Failed");
    let committed = commit(&session, &planned)?;
    assert_eq!(committed["ok"], true, "{committed}");
    let (action, entity) = last_effect(&session)?;
    let blocked = wait(&session, 100)?;
    assert_eq!(blocked["work_state"]["state"], "active");
    assert_eq!(
        fields(&session, "work_order", entity)?["amount_remaining"],
        1
    );
    assert_eq!(drink(&session)?, 40);
    // Work is gated before metabolism in a bounded advance. At tick 1201,
    // seven dwarves consume seven drinks; that observation enables the next
    // advance, without banking the 1200 blocked ticks as productive work.
    let deadline = wait(&session, 1_100)?;
    assert_eq!(deadline["commit_state"], "Failed", "{deadline}");
    assert_eq!(deadline["work_state"]["state"], "active");
    assert_eq!(drink(&session)?, 33);
    let failed_proof = receipt(&session, action)?;
    let partial = wait(&session, 49)?;
    assert_eq!(partial["work_state"]["state"], "active");
    assert_eq!(drink(&session)?, 33, "blocked ticks must not be banked");
    assert_eq!(
        fields(&session, "work_order", entity)?["amount_remaining"],
        1
    );
    let released = wait(&session, 1)?;
    assert_eq!(released["commit_state"], "Failed", "{released}");
    assert_eq!(released["work_state"]["state"], "quiescent");
    assert_eq!(released["open_actions_remaining"], 0);
    assert_eq!(drink(&session)?, 38);
    assert_eq!(
        fields(&session, "work_order", entity)?["amount_remaining"],
        0
    );
    assert_eq!(receipt(&session, action)?, failed_proof);

    let capped = open("73615")?;
    let conditions = json!([{"kind":"item_count_below", "item_token":"DRINK", "threshold":46}]);
    let planned = plan(
        &capped,
        &order("bounded replenishment", "BREW_DRINK", 20, conditions),
    )?;
    assert_eq!(planned["forecast"]["available"], true, "{planned}");
    assert_eq!(planned["forecast"]["predicted_complete"], false);
    let committed = commit(&capped, &planned)?;
    assert_eq!(committed["ok"], true, "{committed}");
    let (_, entity) = last_effect(&capped)?;
    let waited = wait(&capped, 500)?;
    assert_eq!(waited["work_state"]["state"], "active");
    let bounded = fields(&capped, "work_order", entity)?;
    assert_eq!(bounded["amount_remaining"], 18);
    assert_eq!(bounded["work_ticks"], 0);
    assert!(
        bounded["blocked_by"]
            .as_str()
            .is_some_and(|reason| reason.contains("46")),
        "{bounded}"
    );
    assert_eq!(
        drink(&capped)?,
        50,
        "one indivisible unit may cross the threshold, but no further unit may run"
    );
    assert_certificate(&cancel(&capped, "stop_future_steps", "plan")?);
    Ok(())
}

#[test]
fn emergency_cleanup_of_terminal_work_pauses_without_rewriting_goal_proof() -> TestResult {
    let _serial = crate::test_serial();
    let session = open_session("73616", true, Some("starter_fortress"), true)?;
    let other = open_session("73616", true, None, true)?;
    let digest = early_dig_plan(&session, 73616)?;
    let committed = parsed(&facade::fortress_commit(Some(session.clone()), digest))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let (action, _) = last_effect(&session)?;
    let early = wait(&session, 1)?;
    assert_eq!(early["commit_state"], "Verified", "{early}");
    assert_eq!(early["work_state"]["state"], "active");
    let original = receipt(&session, action)?;
    let consent = request_unpause(&session)?;
    assert_eq!(consent["ok"], false, "{consent}");
    assert_eq!(consent["clock_consent"]["votes"], 1);
    let budget = in_session(&session, |guard| {
        let original = guard.budget;
        guard.budget.max_actions = 1;
        Ok(original)
    })?;
    let before = snapshot(&session)?;
    let denied = cancel(&session, "emergency_pause_and_drain", "plan")?;
    assert_eq!(denied["ok"], false, "{denied}");
    assert_eq!(denied["error"]["code"], "budget_exceeded");
    assert_eq!(
        snapshot(&session)?,
        before,
        "pause plus stop needs two action units"
    );
    assert_eq!(receipt(&session, action)?, original);
    in_session(&session, |guard| {
        assert!(guard.leases.unpause_consent.contains(&guard.session_id));
        guard.budget = budget;
        Ok(())
    })?;
    let drained = cancel(&session, "emergency_pause_and_drain", "plan")?;
    assert_certificate(&drained);
    assert_eq!(
        drained["agent_turn"]["briefing"]["paused"], true,
        "{drained}"
    );
    assert!(snapshot(&session)?.paused);
    assert_eq!(receipt(&session, action)?, original);
    in_session(&session, |guard| {
        assert!(guard.leases.unpause_consent.is_empty());
        Ok(())
    })?;
    let renewed = request_unpause(&other)?;
    assert_eq!(renewed["ok"], false, "{renewed}");
    assert_eq!(renewed["clock_consent"]["votes"], 1);
    assert!(snapshot(&session)?.paused);
    Ok(())
}

#[test]
fn partial_emergency_drain_refreshes_agent_work_and_revokes_previous_clock_consent() -> TestResult {
    let _serial = crate::test_serial();
    let session = open_session("73619", true, Some("starter_fortress"), true)?;
    let other = open_session("73619", true, None, true)?;
    plan_and_commit(
        &session,
        r#"[
            {"action":{"kind":"designate_dig","min":[1,3,10],"max":[10,5,10],"mode":"mine"}},
            {"action":{"kind":"create_work_order","name":"stop before denied digging","job_token":"BREW_DRINK","amount":20}}
        ]"#,
    )?;
    let (dig, order) = in_session(&session, |guard| {
        let ids = &guard.last_plan_actions;
        if ids.len() != 2 {
            return Err(fixture_error("expected two original actions"));
        }
        Ok((ids[0], ids[1]))
    })?;
    let original_dig = receipt(&session, dig)?;
    let consent = request_unpause(&session)?;
    assert_eq!(consent["ok"], false, "{consent}");
    assert_eq!(consent["clock_consent"]["votes"], 1);
    in_session(&session, |guard| {
        guard
            .grants
            .retain(|grant| grant.capability != Capability::Designate);
        Ok(())
    })?;
    let before = snapshot(&session)?;

    // Reverse-order cleanup stops production and pauses first. The original
    // digging grant has since been removed, so the next action must refuse.
    let refused = cancel(&session, "emergency_pause_and_drain", "plan")?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "capability_denied");
    assert_eq!(refused["drain_progress"]["cancelled"], 1);
    assert_eq!(refused["drain_progress"]["remaining_work"], 1);
    assert_eq!(refused["drain_progress"]["quiescent"], false);
    assert!(refused["finalize_certificate"].is_null());
    let current = snapshot(&session)?;
    assert!(current.paused);
    assert_ne!(current.anchor(), before.anchor());
    assert_eq!(refused["observed_anchor"], anchor_json(&current.anchor()));
    assert_eq!(refused["agent_turn"]["anchor"], refused["observed_anchor"]);
    assert_eq!(refused["agent_turn"]["briefing"]["paused"], true);
    assert_eq!(
        refused["agent_turn"]["active_work"]["cancellation_drains"][0]["drain_progress"],
        refused["drain_progress"]
    );
    assert_eq!(receipt(&session, dig)?, original_dig);
    assert_eq!(receipt(&session, order)?.state, CommitState::Cancelled);
    let visible = refused["agent_turn"]["active_work"]["actions"]
        .as_array()
        .ok_or("agent action rows missing")?;
    for (id, quiet) in [(dig, false), (order, true)] {
        let action = visible
            .iter()
            .find(|action| action["action_id"] == id.to_string())
            .ok_or("original action disappeared from partial drain")?;
        assert_eq!(action["work_state"]["quiescent"], quiet, "{refused}");
        assert_eq!(
            action["state"],
            format!("{:?}", receipt(&session, id)?.state),
            "{refused}"
        );
    }
    in_session(&session, |guard| {
        assert!(guard.leases.unpause_consent.is_empty());
        Ok(())
    })?;
    let renewed = request_unpause(&other)?;
    assert_eq!(renewed["ok"], false, "{renewed}");
    assert_eq!(renewed["clock_consent"]["votes"], 1);
    assert!(snapshot(&session)?.paused);
    Ok(())
}

#[test]
fn session_drain_refuses_the_aggregate_budget_before_stopping_any_order() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("73617")?;
    plan_and_commit(
        &session,
        &order("first retained batch", "BREW_DRINK", 20, json!([])),
    )?;
    let (first, _) = last_effect(&session)?;
    plan_and_commit(
        &session,
        &order("second retained batch", "PREPARE_MEAL", 20, json!([])),
    )?;
    let (second, _) = last_effect(&session)?;
    let originals = [receipt(&session, first)?, receipt(&session, second)?];
    let budget = in_session(&session, |guard| {
        let original = guard.budget;
        guard.budget.max_actions = 1;
        Ok(original)
    })?;
    let before = snapshot(&session)?;
    let denied = cancel(&session, "stop_future_steps", "session")?;
    assert_eq!(denied["ok"], false, "{denied}");
    assert_eq!(denied["error"]["code"], "budget_exceeded");
    assert_eq!(snapshot(&session)?, before);
    assert_eq!(receipt(&session, first)?, originals[0]);
    assert_eq!(receipt(&session, second)?, originals[1]);
    in_session(&session, |guard| {
        guard.budget = budget;
        Ok(())
    })?;
    let drained = cancel(&session, "stop_future_steps", "session")?;
    assert_certificate(&drained);
    assert_eq!(drained["drain_progress"]["actions_total"], 2);
    Ok(())
}

#[test]
fn older_original_plans_remain_drainable_when_session_work_exceeds_one_call_budget() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("73620")?;
    in_session(&session, |guard| {
        guard.budget.max_actions = 1;
        Ok(())
    })?;
    let mut plans = Vec::new();
    for name in ["oldest batch", "middle batch", "latest batch"] {
        let committed = plan_and_commit(&session, &order(name, "BREW_DRINK", 20, json!([])))?;
        let digest = committed["plan_digest"]
            .as_str()
            .ok_or("original plan digest missing")?
            .to_owned();
        plans.push((digest, last_effect(&session)?.0));
    }
    let originals = plans
        .iter()
        .map(|(_, action)| receipt(&session, *action))
        .collect::<TestResult<Vec<_>>>()?;
    let before = snapshot(&session)?;
    let refused = cancel(&session, "stop_future_steps", "session")?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "budget_exceeded");
    assert_eq!(snapshot(&session)?, before);
    for ((_, action), original) in plans.iter().zip(&originals) {
        assert_eq!(receipt(&session, *action)?, *original);
    }

    // The latest plan is selectable independently, but completing it does
    // not leave the earlier plans trapped behind an oversized session drain.
    let latest = cancel(&session, "stop_future_steps", "plan")?;
    assert_certificate(&latest);
    assert_eq!(latest["drain_progress"]["actions_total"], 1);
    assert_eq!(latest["steps"][0]["action_id"], plans[2].1.to_string());
    assert_eq!(
        cancel(&session, "stop_future_steps", "session")?["error"]["code"],
        "budget_exceeded"
    );
    for (digest, action) in &plans[..2] {
        let drained = cancel(&session, "stop_future_steps", "oldest_open_plan")?;
        assert_certificate(&drained);
        assert_eq!(drained["plan_digest"], *digest);
        assert_eq!(drained["drain_progress"]["actions_total"], 1);
        assert_eq!(drained["steps"][0]["action_id"], action.to_string());
        assert_eq!(receipt(&session, *action)?.state, CommitState::Cancelled);
    }
    let settled = cancel(&session, "stop_future_steps", "session")?;
    assert_certificate(&settled);
    assert_eq!(settled["drain_progress"]["actions_total"], 0);
    assert_eq!(settled["untracked_work"]["quiescent"], true);
    Ok(())
}

#[test]
fn ordinary_stop_needs_production_authority_but_no_clock_grant() -> TestResult {
    let _serial = crate::test_serial();
    let session = open_session("73618", false, Some("starter_fortress"), false)?;
    plan_and_commit(
        &session,
        &order("no clock grant", "BREW_DRINK", 20, json!([])),
    )?;
    let (action, entity) = last_effect(&session)?;
    let original = receipt(&session, action)?;
    let before = snapshot(&session)?;
    let denied_wait = parsed(&facade::fortress_wait(Some(session.clone()), Some(50)))?;
    assert_eq!(denied_wait["ok"], false, "{denied_wait}");
    assert_eq!(denied_wait["error"]["code"], "capability_denied");
    let denied_emergency = cancel(&session, "emergency_pause_and_drain", "plan")?;
    assert_eq!(denied_emergency["ok"], false, "{denied_emergency}");
    assert_eq!(denied_emergency["error"]["code"], "capability_denied");
    assert_eq!(snapshot(&session)?, before);
    assert_eq!(receipt(&session, action)?, original);
    assert_certificate(&cancel(&session, "stop_future_steps", "plan")?);
    assert_eq!(
        fields(&session, "work_order", entity)?["status"],
        "cancelled"
    );
    assert!(!snapshot(&session)?.paused);
    Ok(())
}
