//! Real session drains over complete current and recovered original custody.
//!
//! These tests exercise public effects, complete refusal, partial durable
//! publication, and scope preservation. They do not depend on coordinator layout.
use super::*;

fn drain_session(session: &str, mode: &str) -> TestResult<Value> {
    payload(&facade::fortress_cancel(
        Some(session.to_owned()),
        Some(mode.to_owned()),
        Some("session".into()),
    ))
}

fn named_order(name: &str) -> Value {
    let mut order = blocked_order();
    order["action"]["name"] = json!(name);
    order
}

fn assert_session_quiet(value: &Value) {
    assert_eq!(value["ok"], true, "{value}");
    assert_eq!(value["scope"], "session", "{value}");
    assert_eq!(value["selection_scope"], "session_originals", "{value}");
    assert_eq!(value["drain_progress"]["remaining_nonterminal"], 0, "{value}");
    assert_eq!(value["drain_progress"]["remaining_work"], 0, "{value}");
    assert_eq!(value["drain_progress"]["remaining_carried"], 0, "{value}");
    assert_eq!(value["drain_progress"]["quiescent"], true, "{value}");
    assert_eq!(value["publication_confirmed"], true, "{value}");
    assert_eq!(value["durability_pending"], false, "{value}");
    assert_eq!(value["selected_plans_digest"].as_str().map(str::len), Some(64));
    assert_eq!(value["finalize_certificate"]["scope"], "session");
    assert_eq!(
        value["finalize_certificate"]["selected_plans_digest"],
        value["selected_plans_digest"]
    );
    assert_eq!(
        value["finalize_certificate"]["digest"].as_str().map(str::len),
        Some(64)
    );
}

fn view<T>(session: &str, body: impl FnOnce(&LabSession) -> TestResult<T>) -> TestResult<T> {
    let handle = server::lookup_session_str(session)?;
    server::with_session_read(&handle, |error| Err(error.into()), body)
}

fn open_shared(selector: &str) -> TestResult<String> {
    open_variant(selector, true, true)
}

fn open_variant(selector: &str, shared: bool, durable: bool) -> TestResult<String> {
    let opened = payload(&facade::fortress_open_session(
        Some(false),
        Some(selector.to_owned()),
        Some(
            [
                ("observe", "read_only"),
                ("query", "read_only"),
                ("plan", "guarded"),
                ("designate", "guarded"),
                ("configure_production", "reversible"),
                ("control_clock", "reversible"),
            ]
            .into_iter()
            .map(|(capability, risk)| (capability.to_owned(), risk.to_owned()))
            .collect(),
        ),
        Some(60_000),
        Some(2_000),
        Some(8_192),
        Some(4 * 1024 * 1024),
        Some(16_384),
        Some(64),
        Some("starter_fortress".into()),
        Some(shared),
        Some(durable),
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    field(&opened, "session_id")
}

#[test]
fn session_drains_multiple_recovered_and_live_originals_then_reopens_without_losing_partial_progress()
-> TestResult {
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new("session-mixed");
    let first = open("7864031")?;
    let (dig_digest, _) = commit(&first, json!([dig()]))?;
    assert_eq!(wait(&first, 20)?["ok"], true);
    let (order_digest, _) = commit(&first, json!([named_order("second recovered original")]))?;
    directory.restart();
    let session = open("7864031")?;
    let (live_digest, _) = commit(&session, json!([named_order("current original")]))?;
    let pending = plan(&session, json!([{"action":{"kind":"pause","paused":true}}]))?;
    let latest = inspect(&session, |session| {
        Ok((session.last_action, session.last_plan_actions.clone()))
    })?;
    let before = world(&session)?;
    let stopped = drain_session(&session, "stop_future_steps")?;
    assert_session_quiet(&stopped);
    assert_eq!(stopped["drain_progress"]["original_plans_total"], 3);
    assert_eq!(stopped["drain_progress"]["live_actions_total"], 1);
    assert_eq!(stopped["drain_progress"]["recovered_steps_total"], 2);
    assert_eq!(stopped["drain_progress"]["actions_total"], 3);
    assert_eq!(stopped["drain_progress"]["drained"], 3);
    assert_eq!(stopped["drain_progress"]["cancelled"], 3);
    let after = world(&session)?;
    assert_eq!(after.tick, before.tick);
    assert_eq!(after.graph.chunks, before.graph.chunks);
    inspect(&session, |session| {
        assert!(!session.closed, "ordinary session drain must preserve the session");
        assert!(session.open_actions.is_empty());
        assert!(session.carried.iter().all(CarriedStep::is_final));
        assert_eq!(session.last_action, latest.0);
        assert_eq!(session.last_plan_actions, latest.1);
        assert_eq!(
            session.pending.as_ref().map(|pending| pending.digest.as_str()),
            pending["plan_digest"].as_str()
        );
        Ok(())
    })?;
    directory.restart();
    let reopened = open("7864031")?;
    for digest in [&dig_digest, &order_digest, &live_digest] {
        let observed = selected_wait(&reopened, digest)?;
        assert_eq!(observed["ok"], true, "{observed}");
        assert_eq!(observed["selected_plan_wait"]["stop_reason"], "cancelled");
        assert_eq!(observed["selected_plan_wait"]["physical_quiescent"], true);
    }
    assert_eq!(world(&reopened)?.tick, after.tick);
    Ok(())
}

#[test]
fn session_preflights_every_original_before_stopping_newer_authorized_live_work() -> TestResult {
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new("session-preflight");
    let first = open("7864032")?;
    commit(&first, json!([dig()]))?;
    directory.restart();
    let session = open("7864032")?;
    commit(&session, json!([named_order("newer independently authorized work")]))?;
    let before = world(&session)?;
    let (grants, budget) = inspect(&session, |session| {
        Ok((session.grants.clone(), session.budget))
    })?;
    for missing in [Capability::Designate, Capability::Query] {
        inspect(&session, |session| {
            session.grants = grants.clone();
            session.grants.retain(|grant| grant.capability != missing);
            Ok(())
        })?;
        let refused = drain_session(&session, "stop_future_steps")?;
        assert_eq!(refused["error"]["code"], "capability_denied", "{refused}");
        assert!(refused["finalize_certificate"].is_null());
        assert_eq!(world(&session)?, before);
    }
    inspect(&session, |session| {
        session.grants = grants;
        Ok(())
    })?;
    for narrowed in [
        WorkBudget { max_actions: 1, ..budget },
        WorkBudget { max_entities: 1, ..budget },
        WorkBudget { max_bytes: 6_000, ..budget },
        WorkBudget { max_output_tokens: 1_499, ..budget },
    ] {
        inspect(&session, |session| {
            session.budget = narrowed;
            Ok(())
        })?;
        let refused = drain_session(&session, "stop_future_steps")?;
        assert_eq!(refused["error"]["code"], "budget_exceeded", "{refused}");
        assert!(refused["finalize_certificate"].is_null());
        assert_eq!(world(&session)?, before);
    }
    inspect(&session, |session| {
        session.budget = budget;
        Ok(())
    })?;
    let refused = drain_session(&session, "compensate_reversible")?;
    assert_eq!(refused["error"]["code"], "conflict", "{refused}");
    assert_eq!(world(&session)?, before);
    assert_session_quiet(&drain_session(&session, "stop_future_steps")?);
    Ok(())
}

#[test]
fn session_emergency_admits_one_pause_and_the_complete_mixed_drain_before_any_effect() -> TestResult {
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new("session-emergency");
    let first = open("7864033")?;
    commit(&first, json!([dig()]))?;
    directory.restart();
    let session = open("7864033")?;
    commit(&session, json!([named_order("live emergency cleanup")]))?;
    let before = world(&session)?;
    assert!(!before.paused);
    inspect(&session, |session| {
        session.budget.max_actions = 2;
        Ok(())
    })?;
    let refused = drain_session(&session, "emergency_pause_and_drain")?;
    assert_eq!(refused["error"]["code"], "budget_exceeded", "{refused}");
    assert_eq!(world(&session)?, before);
    inspect(&session, |session| {
        session.budget.max_actions = 3;
        Ok(())
    })?;
    let stopped = drain_session(&session, "emergency_pause_and_drain")?;
    assert_session_quiet(&stopped);
    assert_eq!(stopped["drain_progress"]["actions_total"], 2);
    assert_eq!(stopped["drain_progress"]["drained"], 2);
    assert_eq!(stopped["emergency_pause"]["changed"], true);
    assert_eq!(stopped["emergency_pause"]["publication_confirmed"], true);
    assert_eq!(stopped["emergency_pause"]["game_ticks_advanced"], 0);
    assert_eq!(
        stopped["emergency_pause"]["before_anchor"],
        server::anchor_json(&before.anchor())
    );
    assert_eq!(stopped["paused"], true);
    assert_eq!(world(&session)?.tick, before.tick);
    inspect(&session, |session| {
        let drain = session.carried.iter().find_map(|row| row.recovered_drain.as_ref())
            .ok_or("recovered drain receipt missing")?;
        assert!(drain.before_anchor.cursor.sequence > before.cursor.sequence);
        assert_eq!(drain.before_anchor.tick, before.tick);
        assert!(session.leases.unpause_consent.is_empty());
        Ok(())
    })?;
    Ok(())
}

#[test]
fn session_halts_after_each_failed_stop_publication_then_retries_and_reopens() -> TestResult {
    let _serial = server::serialized_durable_tests();
    for (selector, mixed) in [("7864034", false), ("7864035", true)] {
        let directory = StateDir::new(if mixed { "session-live-append" } else { "session-recovered-append" });
        let first = open(selector)?;
        let (first_digest, _) = commit(&first, json!([dig()]))?;
        let recovered_second = if mixed {
            None
        } else {
            Some(commit(&first, json!([named_order("second recovered append frontier")]))?.0)
        };
        directory.restart();
        let session = open(selector)?;
        let second_digest = if let Some(digest) = recovered_second {
            digest
        } else {
            commit(&session, json!([named_order("live first append frontier")]))?.0
        };
        let before = world(&session)?;
        server::inject_durable_crash_after(0);
        let partial = drain_session(&session, "stop_future_steps")?;
        assert_eq!(partial["ok"], false, "{partial}");
        assert_eq!(partial["error"]["code"], "adapter_unavailable");
        assert_eq!(partial["drain_progress"]["actions_total"], 2);
        assert_eq!(partial["drain_progress"]["drained"], 1);
        assert_eq!(partial["drain_progress"]["remaining_work"], 1);
        assert_eq!(partial["drain_progress"]["quiescent"], false);
        assert_eq!(partial["durability_pending"], true);
        assert_eq!(partial["publication_confirmed"], false);
        assert!(partial["finalize_certificate"].is_null());
        inspect(&session, |session| {
            let older = session.carried.iter()
                .find(|row| row.plan_digest.to_hex() == first_digest)
                .ok_or("older recovered original missing")?;
            assert_eq!(older.state, "dispatched");
            assert!(!older.work_state.is_quiescent());
            if mixed {
                let live = session.last_action.ok_or("newest live action missing")?;
                assert!(server::action_fully_drained(&session.adapter, live));
            } else {
                let newer = session.carried.iter()
                    .find(|row| row.plan_digest.to_hex() == second_digest)
                    .ok_or("newer recovered original missing")?;
                assert_eq!(newer.state, "cancelled");
                assert!(newer.work_state.is_quiescent());
            }
            Ok(())
        })?;
        let frontier = world(&session)?;
        assert_ne!(frontier, before);
        assert_eq!(frontier.tick, before.tick);
        let denied = drain_session(&session, "stop_future_steps")?;
        assert_eq!(denied["error"]["code"], "adapter_unavailable", "{denied}");
        assert_eq!(world(&session)?, frontier);
        server::inject_durable_crash_after(100_000);
        let completed = drain_session(&session, "stop_future_steps")?;
        assert_session_quiet(&completed);
        assert_eq!(completed["drain_progress"]["drained"], 1);
        directory.restart();
        let reopened = open(selector)?;
        for digest in [&first_digest, &second_digest] {
            let observed = selected_wait(&reopened, digest)?;
            assert_eq!(observed["ok"], true, "{observed}");
            assert_eq!(observed["selected_plan_wait"]["stop_reason"], "cancelled");
            assert_eq!(observed["selected_plan_wait"]["physical_quiescent"], true);
        }
        assert_eq!(world(&reopened)?.tick, before.tick);
    }
    Ok(())
}

#[test]
fn session_drains_accepted_recovery_and_own_work_without_stopping_another_live_member() -> TestResult {
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new("session-shared");
    let first = open_shared("7864036")?;
    commit(&first, json!([named_order("accepted recovered world custody")]))?;
    directory.restart();
    let owner = open_shared("7864036")?;
    let other = open_shared("7864036")?;
    commit(&owner, json!([named_order("owned live original")]))?;
    commit(&other, json!([named_order("foreign live original")]))?;
    let foreign = inspect(&other, |session| {
        session.last_action.ok_or_else(|| "foreign action missing".into())
    })?;
    let (foreign_receipt, foreign_entity, before_tick) = view(&other, |session| {
        let receipt = session.adapter.action_receipt(foreign).cloned()
            .ok_or("foreign receipt missing")?;
        let step = session.adapter.action_step(foreign).ok_or("foreign step missing")?;
        let entity = dfmcp_intent::effects::created_entity_id(&step.idempotency_key, 0);
        let record = session.adapter.snapshot().graph.entities.get(&entity).cloned()
            .ok_or("foreign physical work missing")?;
        Ok((receipt, (entity, record), session.adapter.snapshot().tick))
    })?;
    let stopped = drain_session(&owner, "stop_future_steps")?;
    assert_session_quiet(&stopped);
    assert_eq!(stopped["drain_progress"]["original_plans_total"], 2);
    assert_eq!(stopped["drain_progress"]["live_actions_total"], 1);
    assert_eq!(stopped["drain_progress"]["recovered_steps_total"], 1);
    assert_eq!(stopped["drain_progress"]["drained"], 2);
    view(&other, |session| {
        assert!(!session.closed);
        assert_eq!(session.adapter.action_receipt(foreign), Some(&foreign_receipt));
        assert_eq!(
            session.adapter.snapshot().graph.entities.get(&foreign_entity.0),
            Some(&foreign_entity.1)
        );
        assert!(!server::action_fully_drained(&session.adapter, foreign));
        assert_eq!(session.adapter.snapshot().tick, before_tick);
        Ok(())
    })?;
    assert_session_quiet(&drain_session(&other, "stop_future_steps")?);
    Ok(())
}

#[test]
fn session_refuses_unknown_recovered_work_before_stopping_a_newer_live_original() -> TestResult {
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new("session-unknown");
    let first = open("7864037")?;
    commit(&first, json!([dig()]))?;
    directory.restart();
    let session = open("7864037")?;
    commit(&session, json!([named_order("leave live work alone on unknown recovery")]))?;
    inspect(&session, |session| {
        let mut snapshot = session.adapter.snapshot().clone();
        let step = session.carried.iter().find_map(|row| row.work_step.as_ref())
            .ok_or("recovered work missing")?;
        let entity = dfmcp_intent::effects::created_entity_id(&step.idempotency_key, 0);
        snapshot.graph.entities.get_mut(&entity).ok_or("designation missing")?
            .fields.remove("status");
        snapshot.cursor = snapshot.cursor.checked_next().ok_or("cursor exhausted")?;
        snapshot.refresh_hash();
        session.adapter.inject_snapshot(snapshot)?;
        Ok(())
    })?;
    let before = world(&session)?;
    let refused = drain_session(&session, "stop_future_steps")?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert!(refused["finalize_certificate"].is_null());
    assert_ne!(refused["drain_progress"]["quiescent"], true);
    assert_eq!(world(&session)?, before);
    Ok(())
}

#[test]
fn session_emergency_still_pauses_untracked_work_without_claiming_a_complete_drain() -> TestResult {
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new("session-untracked");
    let first = open("7864038")?;
    commit(&first, json!([dig()]))?;
    directory.restart();
    let session = open("7864038")?;
    inspect(&session, |session| {
        // A lost recovered owner is not permission to stop its physical work.
        // It must also not disable the independent authorized emergency brake.
        session.carried.clear();
        Ok(())
    })?;
    let before = world(&session)?;
    let stopped = drain_session(&session, "emergency_pause_and_drain")?;
    assert_eq!(stopped["paused"], true, "{stopped}");
    assert_eq!(stopped["emergency_pause"]["publication_confirmed"], true);
    assert_eq!(stopped["emergency_pause"]["changed"], true);
    assert_eq!(stopped["drain_progress"]["drained"], 0);
    assert_eq!(stopped["drain_progress"]["untracked_work_quiescent"], false);
    assert_eq!(stopped["drain_progress"]["quiescent"], false);
    assert!(stopped["finalize_certificate"].is_null());
    let after = world(&session)?;
    assert_eq!(after.tick, before.tick);
    assert_eq!(after.graph.entities, before.graph.entities);
    assert_eq!(after.graph.chunks, before.graph.chunks);
    inspect(&session, |session| {
        assert_eq!(server::untracked_work_json(session)["quiescent"], false);
        Ok(())
    })?;
    Ok(())
}

#[test]
fn minimum_session_output_keeps_full_selection_and_partial_progress_identity() -> TestResult {
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new("session-minimum-output");
    let first = open("7864039")?;
    for name in ["first retained original", "second retained original", "third retained original"] {
        commit(&first, json!([named_order(name)]))?;
    }
    directory.restart();
    let session = open("7864039")?;
    commit(&session, json!([named_order("current retained original")]))?;
    inspect(&session, |session| {
        session.budget.max_output_tokens = 1_500;
        Ok(())
    })?;
    server::inject_durable_crash_after(0);
    let raw = facade::fortress_cancel(
        Some(session.clone()), Some("stop_future_steps".into()), Some("session".into()),
    );
    assert!(raw.len() <= 6_000, "{} bytes: {raw}", raw.len());
    let partial = payload(&raw)?;
    assert_eq!(partial["ok"], false, "{partial}");
    assert_eq!(partial["scope"], "session");
    assert_eq!(partial["selection_scope"], "session_originals");
    assert_eq!(partial["selected_plans_digest"].as_str().map(str::len), Some(64));
    assert_eq!(partial["drain_progress"]["original_plans_total"], 4);
    assert_eq!(partial["drain_progress"]["live_actions_total"], 1);
    assert_eq!(partial["drain_progress"]["recovered_steps_total"], 3);
    assert_eq!(partial["drain_progress"]["actions_total"], 4);
    assert_eq!(partial["drain_progress"]["drained"], 1);
    assert_eq!(partial["drain_progress"]["remaining_work"], 3);
    assert_eq!(partial["drain_progress"]["quiescent"], false);
    assert_eq!(partial["publication_confirmed"], false);
    assert_eq!(partial["durability_pending"], true);
    assert_eq!(partial["error"]["code"], "adapter_unavailable");
    assert!(partial["finalize_certificate"].is_null());
    assert_eq!(partial["output_budget"]["max_output_tokens"], 1_500);
    assert_ne!(partial["output_budget"]["tier"], "full");
    assert_eq!(partial["agent_turn"]["anchor"], partial["observed_anchor"]);
    server::inject_durable_crash_after(100_000);
    let raw = facade::fortress_cancel(
        Some(session.clone()), Some("stop_future_steps".into()), Some("session".into()),
    );
    assert!(raw.len() <= 6_000, "{} bytes: {raw}", raw.len());
    let completed = payload(&raw)?;
    assert_session_quiet(&completed);
    assert_eq!(completed["drain_progress"]["drained"], 3);
    Ok(())
}

#[test]
fn session_reconstructs_complete_live_custody_when_volatile_open_lists_are_missing() -> TestResult {
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new("session-missing-open-list");
    let first = open("7864040")?;
    commit(&first, json!([named_order("retained recovered original")]))?;
    directory.restart();
    let session = open("7864040")?;
    commit(&session, json!([named_order("retained live original")]))?;
    let live = inspect(&session, |session| {
        let live = session.last_action.ok_or("live action missing")?;
        session.open_actions.clear();
        session.last_plan_actions.clear();
        session.last_action = None;
        Ok(live)
    })?;
    let before = world(&session)?;
    let stopped = drain_session(&session, "stop_future_steps")?;
    assert_session_quiet(&stopped);
    assert_eq!(stopped["drain_progress"]["original_plans_total"], 2);
    assert_eq!(stopped["drain_progress"]["live_actions_total"], 1);
    assert_eq!(stopped["drain_progress"]["recovered_steps_total"], 1);
    assert_eq!(stopped["drain_progress"]["drained"], 2);
    assert_eq!(world(&session)?.tick, before.tick);
    inspect(&session, |session| {
        assert!(server::action_fully_drained(&session.adapter, live));
        assert!(session.carried.iter().all(CarriedStep::is_final));
        Ok(())
    })?;
    Ok(())
}

#[test]
fn session_published_success_matches_the_final_owner_frontier_at_append_boundaries() -> TestResult {
    let _serial = server::serialized_durable_tests();
    let mut saw_success = false;
    let mut saw_write_failure = false;
    // Sweep real append cutoffs without assuming how many journal records one
    // original consumes. In particular, an implicit write after the response
    // must not turn a reported successful publication into a hidden fault.
    for append_allowance in (0..=12).chain(std::iter::once(100_000)) {
        let directory = StateDir::new("session-final-publication");
        let first = open("7864041")?;
        commit(&first, json!([named_order("recovered publication boundary")]))?;
        directory.restart();
        let session = open("7864041")?;
        commit(&session, json!([named_order("live publication boundary")]))?;
        server::inject_durable_crash_after(append_allowance);
        let result = drain_session(&session, "stop_future_steps")?;
        let (fault, current, durable) = inspect(&session, |session| {
            let durable = server::with_durable_store(|store| {
                Ok(store.head(session.fortress_id).map(|head| head.anchor))
            })?;
            Ok((session.durability_fault.clone(), session.adapter.snapshot().anchor(), durable))
        })?;
        if result["publication_confirmed"] == true {
            assert!(fault.is_none(), "append allowance {append_allowance}: {result}; {fault:?}");
            assert_eq!(durable, Some(current), "append allowance {append_allowance}: {result}");
        }
        if result["finalize_certificate"].is_object() {
            saw_success = true;
            assert_session_quiet(&result);
            assert!(fault.is_none(), "certificate preceded a failed trailing publication");
            assert_eq!(durable, Some(current));
            assert_eq!(result["finalize_certificate"]["anchor"], server::anchor_json(&current));
        }
        if result["error"]["code"] == "adapter_unavailable" {
            saw_write_failure = true;
            assert!(result["finalize_certificate"].is_null());
        }
    }
    assert!(saw_success, "the high-allowance control must reach a complete drain");
    assert!(saw_write_failure, "the zero-append boundary must exercise publication failure");
    Ok(())
}

#[test]
fn contradictory_foreign_lease_cannot_hide_an_owned_original_with_lost_receipt_custody()
-> TestResult {
    let _serial = server::serialized_durable_tests();
    let _directory = StateDir::new("session-contradictory-owner");
    let owner = open_variant("7864042", true, false)?;
    let other = open_variant("7864042", true, false)?;
    commit(&owner, json!([dig()]))?;
    let other_id = server::parse_session_id_arg(&other)?;
    let handle = server::lookup_session_str(&owner)?;
    server::with_session(
        &handle,
        || -> TestResult { Err("shared fixture unavailable".into()) },
        |session| {
            let action = session.last_action.ok_or("owned action missing")?;
            session.commit_receipts.clear();
            session.open_actions.clear();
            session.last_plan_actions.clear();
            session.last_action = None;
            // The immutable original still names this caller. A contradictory
            // derived lease label must not turn it into positively foreign work.
            session.leases.by_action.insert(action, (other_id, Vec::new()));
            Ok(())
        },
    )?;
    let before = view(&owner, |session| Ok(session.adapter.snapshot().clone()))?;
    let result = drain_session(&owner, "stop_future_steps")?;
    assert_ne!(result["drain_progress"]["quiescent"], true, "{result}");
    assert!(result["finalize_certificate"].is_null(), "{result}");
    assert_eq!(view(&owner, |session| Ok(session.adapter.snapshot().clone()))?, before);
    Ok(())
}

#[test]
fn session_preflights_compensation_authority_for_older_live_original_before_newer_stop()
-> TestResult {
    let _serial = server::serialized_durable_tests();
    let _directory = StateDir::new("session-compensation-authority");
    let session = open_variant("7864043", false, false)?;
    // The typed planner can retain a compensation that the closed public action
    // grammar does not expose. Commit and cancel still use the real facade.
    let digest = inspect(&session, |session| {
        let raw = json!([dig()]).to_string();
        let summary = "original excavation with independent pause compensation".to_owned();
        let mut intent = server::semantic_intent(
            dfmcp_core::IntentId::new(7864043),
            session.adapter.snapshot(),
            summary.clone(),
            &raw,
        )?;
        intent.requested_actions[0].compensation = Some(Action::Pause { paused: true });
        let prepared = dfmcp_intent::StaticPlanner::default().prepare_laboratory(
            session.adapter.snapshot(), &intent, &server::read_context(session),
        )?;
        assert_eq!(prepared.steps[0].compensation, Some(Action::Pause { paused: true }));
        let digest = prepared.digest.to_hex();
        session.pending = Some(server::PendingPlan {
            plan: prepared,
            digest: digest.clone(),
            source: server::PlanSource::Actions { summary, raw },
        });
        Ok(digest)
    })?;
    let committed = payload(&facade::fortress_commit(Some(session.clone()), digest))?;
    assert_eq!(committed["ok"], true, "{committed}");
    commit(&session, json!([named_order("newer work with independent stop authority")]))?;
    inspect(&session, |session| {
        session.grants.retain(|grant| grant.capability != Capability::ControlClock);
        Ok(())
    })?;
    let before = world(&session)?;
    let refused = drain_session(&session, "compensate_reversible")?;
    assert_eq!(refused["error"]["code"], "capability_denied", "{refused}");
    assert!(refused["finalize_certificate"].is_null());
    assert_eq!(world(&session)?, before);
    assert_session_quiet(&drain_session(&session, "stop_future_steps")?);
    Ok(())
}

#[test]
fn session_preflights_live_spatial_reservations_before_newer_stops_or_emergency_pause()
-> TestResult {
    let _serial = server::serialized_durable_tests();
    let _directory = StateDir::new("session-live-reservation");
    let session = open_variant("7864044", false, false)?;
    commit(&session, json!([dig()]))?;
    commit(&session, json!([named_order("newer work behind a conflicting live reservation")]))?;
    let previous = inspect(&session, |session| {
        let area = session.open_actions.iter().find_map(|id| {
            session.adapter.action_step(*id).and_then(|step| step.action.scope().map_area)
        }).ok_or("live excavation scope missing")?;
        let previous = session.leases.manager.clone();
        // A current foreign reservation may exist after an older local lease
        // was released. Complete original receipts remain the caller's custody.
        session.leases.manager.release_session_leases(session.session_id);
        session.leases.manager.acquire_spatial_lease(
            SessionId::new(999), area, true, session.adapter.snapshot().tick, 100,
        )?;
        Ok(previous)
    })?;
    let before = world(&session)?;
    let leases_before = inspect(&session, |session| Ok(format!("{:?}", session.leases.manager)))?;
    for mode in ["stop_future_steps", "emergency_pause_and_drain"] {
        let refused = drain_session(&session, mode)?;
        assert_eq!(refused["ok"], false, "{refused}");
        assert_eq!(refused["error"]["code"], "conflict", "{refused}");
        assert!(refused["finalize_certificate"].is_null());
        assert!(refused["emergency_pause"].is_null());
        assert_eq!(world(&session)?, before);
        assert_eq!(
            inspect(&session, |session| Ok(format!("{:?}", session.leases.manager)))?,
            leases_before,
        );
    }
    inspect(&session, |session| {
        session.leases.manager = previous;
        Ok(())
    })?;
    let stopped = drain_session(&session, "stop_future_steps")?;
    assert_session_quiet(&stopped);
    assert_eq!(stopped["drain_progress"]["drained"], 2);
    Ok(())
}

#[test]
fn session_refuses_foreign_owner_of_selected_live_action_with_original_receipt_retained()
-> TestResult {
    let _serial = server::serialized_durable_tests();
    let _directory = StateDir::new("session-live-owner-conflict");
    let owner = open_variant("7864045", true, false)?;
    let other = open_variant("7864045", true, false)?;
    commit(&owner, json!([dig()]))?;
    let action = inspect(&owner, |session| session.last_action.ok_or_else(|| "owned action missing".into()))?;
    commit(&owner, json!([named_order("newer owned work must stay active")]))?;
    let other_id = server::parse_session_id_arg(&other)?;
    let handle = server::lookup_session_str(&owner)?;
    let previous = server::with_session(
        &handle,
        || -> TestResult<_> { Err("shared fixture unavailable".into()) },
        |session| {
            let previous = session.leases.by_action.get(&action).cloned()
                .ok_or("selected ownership missing")?;
            session.leases.by_action.insert(action, (other_id, previous.1.clone()));
            Ok(previous)
        },
    )?;
    let before = view(&owner, |session| Ok(session.adapter.snapshot().clone()))?;
    for mode in ["stop_future_steps", "emergency_pause_and_drain"] {
        let refused = drain_session(&owner, mode)?;
        assert_eq!(refused["ok"], false, "{refused}");
        assert_eq!(refused["error"]["code"], "conflict", "{refused}");
        assert!(refused["emergency_pause"].is_null());
        assert!(refused["finalize_certificate"].is_null());
        assert_eq!(view(&owner, |session| Ok(session.adapter.snapshot().clone()))?, before);
    }
    server::with_session(
        &handle,
        || -> TestResult { Err("shared fixture unavailable".into()) },
        |session| {
            session.leases.by_action.insert(action, previous);
            Ok(())
        },
    )?;
    assert_session_quiet(&drain_session(&owner, "stop_future_steps")?);
    Ok(())
}

#[test]
fn session_preflights_distinct_compensation_entity_before_stopping_newer_original()
-> TestResult {
    let _serial = server::serialized_durable_tests();
    let _directory = StateDir::new("session-inverse-ownership");
    let session = open_variant("7864046", false, false)?;
    let unit = dfmcp_core::EntityId::new(1003);
    let digest = inspect(&session, |session| {
        let mut grant = session.grants.iter().find(|grant| grant.capability == Capability::Designate)
            .cloned().ok_or("fixture grant missing")?;
        grant.capability = Capability::ConfigureLabor;
        grant.max_risk = RiskTier::Reversible;
        session.grants.push(grant);
        let raw = json!([dig()]).to_string();
        let summary = "excavation with an independently scoped labor inverse".to_owned();
        let mut intent = server::semantic_intent(
            dfmcp_core::IntentId::new(7864046),
            session.adapter.snapshot(), summary.clone(), &raw,
        )?;
        intent.requested_actions[0].compensation = Some(Action::SetLabor {
            units: vec![unit], labor: "BREW".to_owned(), enabled: false,
        });
        let prepared = dfmcp_intent::StaticPlanner::default().prepare_laboratory(
            session.adapter.snapshot(), &intent, &server::read_context(session),
        )?;
        let digest = prepared.digest.to_hex();
        session.pending = Some(server::PendingPlan {
            plan: prepared, digest: digest.clone(),
            source: server::PlanSource::Actions { summary, raw },
        });
        Ok(digest)
    })?;
    let committed = payload(&facade::fortress_commit(Some(session.clone()), digest))?;
    assert_eq!(committed["ok"], true, "{committed}");
    commit(&session, json!([named_order("newer original before independent inverse")]))?;
    inspect(&session, |session| {
        session.leases.manager.acquire_entity_lease(
            SessionId::new(999), unit, true, session.adapter.snapshot().tick, 100,
        )?;
        Ok(())
    })?;
    let before = world(&session)?;
    let refused = drain_session(&session, "compensate_reversible")?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "conflict", "{refused}");
    assert!(refused["finalize_certificate"].is_null());
    assert_eq!(world(&session)?, before);
    // An ordinary stop never applies that inverse and does not need its entity.
    assert_session_quiet(&drain_session(&session, "stop_future_steps")?);
    Ok(())
}

#[test]
fn session_preserves_overlapping_foreign_physical_work_without_live_derived_leases()
-> TestResult {
    let _serial = server::serialized_durable_tests();
    let _directory = StateDir::new("session-foreign-missing-leases");
    let owner = open_variant("7864047", true, false)?;
    let other = open_variant("7864047", true, false)?;
    commit(&other, json!([dig()]))?;
    let foreign_id = inspect(&other, |session| {
        session.last_action.ok_or_else(|| "foreign action missing".into())
    })?;
    let foreign_owner = server::parse_session_id_arg(&other)?;
    let handle = server::lookup_session_str(&owner)?;
    server::with_session(
        &handle,
        || -> TestResult { Err("shared fixture unavailable".into()) },
        |session| {
            // Keep the real immutable original and active physical work, but
            // simulate loss of its derived lease index and reservations.
            session.leases.by_action.remove(&foreign_id);
            session.leases.manager.release_session_leases(foreign_owner);
            Ok(())
        },
    )?;
    // The legacy designation engine supports overlapping observations. Missing
    // derived admission custody permits this setup; cancellation must still use
    // the positively known foreign original before stopping either project.
    commit(&owner, json!([dig()]))?;
    commit(&owner, json!([named_order("newer work behind overlapping foreign excavation")]))?;
    let before = view(&owner, |session| Ok(session.adapter.snapshot().clone()))?;
    let foreign_receipt = view(&owner, |session| {
        session.adapter.action_receipt(foreign_id).cloned()
            .ok_or_else(|| "foreign original receipt missing".into())
    })?;
    for retain_expired_owner in [false, true] {
        if retain_expired_owner {
            server::with_session(
                &handle,
                || -> TestResult { Err("shared fixture unavailable".into()) },
                |session| {
                    // This is also the state after reservation expiry/pruning:
                    // original ownership remains but no active lease protects it.
                    session.leases.by_action.insert(foreign_id, (foreign_owner, Vec::new()));
                    Ok(())
                },
            )?;
        }
        let leases_before = view(&owner, |session| Ok(format!("{:?}", session.leases.manager)))?;
        for mode in ["stop_future_steps", "emergency_pause_and_drain"] {
            let refused = drain_session(&owner, mode)?;
            assert_eq!(refused["ok"], false, "{refused}");
            assert_eq!(refused["error"]["code"], "conflict", "{refused}");
            assert!(refused["emergency_pause"].is_null());
            assert!(refused["finalize_certificate"].is_null());
            view(&owner, |session| {
                assert_eq!(session.adapter.snapshot(), &before);
                assert_eq!(session.adapter.action_receipt(foreign_id), Some(&foreign_receipt));
                assert!(!server::action_fully_drained(&session.adapter, foreign_id));
                assert_eq!(format!("{:?}", session.leases.manager), leases_before);
                Ok(())
            })?;
        }
    }
    Ok(())
}
