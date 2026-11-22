//! Exact selected sets span live and durable custody without adopting other work.
use super::*;

fn work_order(name: &str) -> Value {
    json!({"action":{"kind":"create_work_order","name":name,"job_token":"BREW_DRINK","amount":20}})
}

fn stop_set(session: &str, digests: &[&str]) -> TestResult<Value> {
    stop_set_mode(session, digests, "stop_future_steps")
}

fn stop_set_mode(session: &str, digests: &[&str], mode: &str) -> TestResult<Value> {
    payload(&facade::fortress_cancel(
        Some(session.to_owned()),
        Some(mode.to_owned()),
        Some(format!("plans:{}", digests.join(","))),
    ))
}

#[test]
fn mixed_set_admits_all_authority_and_budgets_before_live_stop_or_emergency_pause() -> TestResult {
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new("selected-mixed-admission");
    let first = open("74669")?;
    let (recovered, _) = commit(&first, json!([dig()]))?;
    directory.restart();
    let session = open("74669")?;
    let (live, _) = commit(&session, json!([work_order("mixed admission live")]))?;
    let before = world(&session)?;
    assert!(!before.paused);
    let (grants, budget) = inspect(&session, |session| {
        Ok((session.grants.clone(), session.budget))
    })?;
    for missing in [
        Capability::Designate,
        Capability::Query,
        Capability::ControlClock,
    ] {
        inspect(&session, |session| {
            session.grants = grants.clone();
            session.grants.retain(|grant| grant.capability != missing);
            Ok(())
        })?;
        let denied = stop_set_mode(&session, &[&live, &recovered], "emergency_pause_and_drain")?;
        assert_eq!(denied["error"]["code"], "capability_denied", "{denied}");
        assert_eq!(denied["error"]["effect_may_have_occurred"], false);
        assert!(denied["finalize_certificate"].is_null());
        assert_eq!(world(&session)?, before);
    }
    inspect(&session, |session| {
        session.grants = grants;
        Ok(())
    })?;
    for narrowed in [
        WorkBudget {
            max_actions: 2,
            ..budget
        },
        WorkBudget {
            max_entities: 1,
            ..budget
        },
        WorkBudget {
            max_bytes: 6_000,
            ..budget
        },
        WorkBudget {
            max_output_tokens: 2_000,
            ..budget
        },
    ] {
        inspect(&session, |session| {
            session.budget = narrowed;
            Ok(())
        })?;
        let denied = stop_set_mode(&session, &[&recovered, &live], "emergency_pause_and_drain")?;
        assert_eq!(denied["error"]["code"], "budget_exceeded", "{denied}");
        assert_eq!(denied["error"]["effect_may_have_occurred"], false);
        assert_eq!(world(&session)?, before);
    }
    inspect(&session, |session| {
        session.budget = budget;
        Ok(())
    })?;
    let denied = stop_set_mode(&session, &[&recovered, &live], "compensate_reversible")?;
    assert_eq!(denied["error"]["code"], "conflict", "{denied}");
    assert_eq!(world(&session)?, before);
    inspect(&session, |session| {
        session.budget.max_actions = 3;
        Ok(())
    })?;
    let stopped = stop_set_mode(&session, &[&recovered, &live], "emergency_pause_and_drain")?;
    assert_eq!(stopped["drain_progress"]["quiescent"], true, "{stopped}");
    assert_eq!(stopped["emergency_pause"]["changed"], true);
    assert_eq!(stopped["paused"], true);
    assert_eq!(stopped["drain_progress"]["drained"], 2);
    let results = stopped["original_results"]
        .as_array()
        .ok_or("missing original results")?;
    assert!(
        results
            .iter()
            .all(|value| value["emergency_pause"].is_null())
    );
    assert_eq!(world(&session)?.tick, before.tick);
    Ok(())
}

#[test]
fn mixed_set_rejects_missing_or_corrupt_selected_custody_and_duplicate_live_aliases() -> TestResult
{
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new("selected-mixed-custody");
    let first = open("74670")?;
    let (recovered, _) = commit(&first, json!([dig()]))?;
    directory.restart();
    let session = open("74670")?;
    let (live, _) = commit(&session, json!([work_order("selected custody live")]))?;
    let before = world(&session)?;
    let unknown = "0".repeat(64);
    let denied = stop_set(&session, &[&recovered, &live, &unknown])?;
    assert_eq!(denied["ok"], false, "{denied}");
    assert_eq!(denied["selection_resolved"], false);
    assert_eq!(denied["error"]["effect_may_have_occurred"], false);
    assert!(denied["drain_progress"]["actions_total"].is_null());
    assert_eq!(world(&session)?, before);
    let original = inspect(&session, |session| {
        let original = session
            .commit_receipts
            .get(&live)
            .cloned()
            .ok_or("live receipt missing")?;
        session
            .commit_receipts
            .insert(live.clone(), "not a receipt".into());
        Ok(original)
    })?;
    let denied = stop_set(&session, &[&recovered, &live])?;
    assert_eq!(denied["error"]["code"], "conflict", "{denied}");
    assert_eq!(world(&session)?, before);
    let alias = "a".repeat(64);
    inspect(&session, |session| {
        session
            .commit_receipts
            .insert(live.clone(), original.clone());
        session.commit_receipts.insert(alias.clone(), original);
        Ok(())
    })?;
    let denied = stop_set(&session, &[&recovered, &live, &alias])?;
    assert_eq!(denied["error"]["code"], "invalid_request", "{denied}");
    assert_eq!(world(&session)?, before);
    let stopped = stop_set(&session, &[&recovered, &alias])?;
    assert_eq!(stopped["drain_progress"]["quiescent"], true, "{stopped}");
    let identities = stopped["selected_plans"]["originals"]
        .as_array()
        .ok_or("selection missing")?;
    assert!(
        identities
            .iter()
            .any(|row| row["requested_plan_digest"] == alias && row["plan_digest"] == live)
    );
    Ok(())
}

#[test]
fn recovered_only_selection_drains_newest_first_and_preserves_unselected_originals() -> TestResult {
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new("selected-recovered-order");
    let first = open("74671")?;
    let (older, _) = commit(&first, json!([work_order("older recovered selected")]))?;
    let (keep, _) = commit(&first, json!([work_order("middle recovered unselected")]))?;
    let (newer, _) = commit(&first, json!([work_order("newer recovered selected")]))?;
    directory.restart();
    let session = open("74671")?;
    let stopped = stop_set(&session, &[&older, &newer])?;
    assert_eq!(stopped["drain_progress"]["quiescent"], true, "{stopped}");
    assert_eq!(stopped["drain_progress"]["live_actions_total"], 0);
    assert_eq!(stopped["drain_progress"]["recovered_steps_total"], 2);
    assert_eq!(stopped["original_results"][0]["plan_digest"], newer);
    assert_eq!(stopped["original_results"][1]["plan_digest"], older);
    inspect(&session, |session| {
        assert!(session.adapter.retained_action_ids().next().is_none());
        assert!(session.commit_receipts.is_empty());
        assert!(
            session
                .carried
                .iter()
                .any(|row| row.plan_digest.to_hex() == keep && !row.is_final())
        );
        Ok(())
    })?;
    Ok(())
}

#[test]
fn mixed_set_publication_failure_preserves_recovered_work_then_retry_finishes_selected_only()
-> TestResult {
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new("selected-mixed-publication");
    let first = open("74672")?;
    let (recovered, _) = commit(&first, json!([dig()]))?;
    let (keep, _) = commit(
        &first,
        json!([work_order("unselected across failed drain")]),
    )?;
    directory.restart();
    let session = open("74672")?;
    let (live, _) = commit(&session, json!([work_order("newest selected publication")]))?;
    server::inject_durable_crash_after(0);
    let partial = stop_set(&session, &[&recovered, &live])?;
    assert_eq!(partial["ok"], false, "{partial}");
    assert_eq!(partial["error"]["code"], "adapter_unavailable");
    assert_eq!(partial["error"]["effect_may_have_occurred"], true);
    assert_eq!(partial["drain_progress"]["drained"], 1);
    assert_eq!(partial["drain_progress"]["remaining_work"], 1);
    assert_eq!(partial["publication_confirmed"], false);
    assert!(partial["finalize_certificate"].is_null());
    inspect(&session, |session| {
        assert!(
            session
                .carried
                .iter()
                .any(|row| row.plan_digest.to_hex() == recovered && !row.is_final())
        );
        Ok(())
    })?;
    let frontier = world(&session)?;
    let denied = stop_set(&session, &[&live, &recovered])?;
    assert_eq!(denied["error"]["code"], "adapter_unavailable", "{denied}");
    assert_eq!(world(&session)?, frontier);
    server::inject_durable_crash_after(100_000);
    let finished = stop_set(&session, &[&live, &recovered])?;
    assert_eq!(finished["drain_progress"]["quiescent"], true, "{finished}");
    assert_eq!(finished["drain_progress"]["drained"], 1);
    assert_eq!(
        finished["selected_plans_digest"],
        partial["selected_plans_digest"]
    );
    assert_eq!(
        finished["finalize_certificate"]["selected_plans"],
        finished["selected_plans"]
    );
    inspect(&session, |session| {
        assert!(
            session
                .carried
                .iter()
                .any(|row| row.plan_digest.to_hex() == keep && !row.is_final())
        );
        Ok(())
    })?;
    Ok(())
}

#[test]
fn recovered_and_live_members_drain_together_preserving_unselected_work_and_pending_review()
-> TestResult {
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new("selected-set-recovery");
    let first = open("74667")?;
    let (recovered, _) = commit(&first, json!([dig()]))?;
    let (keep_recovered, _) = commit(&first, json!([work_order("unselected recovered producer")]))?;
    directory.restart();
    let session = open("74667")?;
    let (live, _) = commit(
        &session,
        json!([work_order("live original beside recovered selection")]),
    )?;
    let (keep_live, _) = commit(&session, json!([work_order("unselected current producer")]))?;
    let pending = plan(&session, json!([{"action":{"kind":"pause","paused":true}}]))?;
    let (kept_action, kept_receipt, latest) = inspect(&session, |session| {
        let id = session
            .last_action
            .ok_or("unselected live action missing")?;
        Ok((
            id,
            session
                .adapter
                .action_receipt(id)
                .cloned()
                .ok_or("receipt missing")?,
            session.last_plan_actions.clone(),
        ))
    })?;
    let before = world(&session)?;
    let stopped = stop_set(&session, &[&live, &recovered])?;
    assert_eq!(stopped["ok"], true, "{stopped}");
    assert_eq!(stopped["drain_progress"]["quiescent"], true, "{stopped}");
    assert_eq!(stopped["drain_progress"]["original_plans_total"], 2);
    assert_eq!(stopped["drain_progress"]["live_actions_total"], 1);
    assert_eq!(stopped["drain_progress"]["recovered_steps_total"], 1);
    assert_eq!(stopped["selection_scope"], "exact_original_set");
    assert_eq!(stopped["selection_resolved"], true);
    assert_eq!(stopped["dispatch_handles_restored"], false);
    assert_eq!(stopped["unselected_plans_included"], false);
    assert_eq!(
        stopped["selected_plans"]["schema"],
        "dfmcp-selected-originals-cancellation-selection/2"
    );
    assert_eq!(
        stopped["selected_plans"]["originals"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
    assert_eq!(
        stopped["finalize_certificate"]["selected_plans"],
        stopped["selected_plans"]
    );
    assert_eq!(
        stopped["finalize_certificate"]["schema"],
        "dfmcp-selected-originals-drain-certificate/2"
    );
    assert_eq!(stopped["original_results"][0]["plan_digest"], live);
    assert_eq!(stopped["original_results"][1]["plan_digest"], recovered);
    assert_eq!(world(&session)?.tick, before.tick);
    inspect(&session, |session| {
        assert_eq!(
            session.adapter.action_receipt(kept_action),
            Some(&kept_receipt)
        );
        assert!(!server::action_fully_drained(&session.adapter, kept_action));
        assert!(session.commit_receipts.contains_key(&keep_live));
        assert!(
            session
                .carried
                .iter()
                .any(|row| row.plan_digest.to_hex() == keep_recovered && !row.is_final())
        );
        assert_eq!(session.last_action, Some(kept_action));
        assert_eq!(session.last_plan_actions, latest);
        assert_eq!(
            session.pending.as_ref().map(|value| value.digest.as_str()),
            pending["plan_digest"].as_str()
        );
        Ok(())
    })?;
    let after = world(&session)?;
    let repeated = stop_set(&session, &[&recovered, &live])?;
    assert_eq!(repeated["drain_progress"]["quiescent"], true, "{repeated}");
    assert_eq!(repeated["drain_progress"]["drained"], 0);
    assert_eq!(world(&session)?, after);
    directory.restart();
    let reopened = open("74667")?;
    for digest in [&recovered, &live] {
        let current = selected_wait(&reopened, digest)?;
        assert_eq!(
            current["selected_plan_wait"]["stop_reason"], "cancelled",
            "{current}"
        );
        assert_eq!(current["selected_plan_wait"]["physical_quiescent"], true);
    }
    let repeated = stop_set(&reopened, &[&recovered, &live])?;
    assert_eq!(repeated["drain_progress"]["quiescent"], true, "{repeated}");
    assert_eq!(repeated["drain_progress"]["live_actions_total"], 0);
    assert_eq!(repeated["drain_progress"]["recovered_steps_total"], 2);
    assert_eq!(repeated["drain_progress"]["drained"], 0);
    Ok(())
}

#[test]
fn selected_set_publication_failure_stops_at_newest_frontier_and_retry_finishes_only_selected()
-> TestResult {
    let _serial = server::serialized_durable_tests();
    let _directory = StateDir::new("selected-set-publication");
    let session = open("74668")?;
    let (older, _) = commit(&session, json!([dig()]))?;
    let (newer, _) = commit(
        &session,
        json!([work_order("newer selected stop frontier")]),
    )?;
    let (unselected, _) = commit(&session, json!([work_order("keep unselected original")]))?;
    let unselected_id = inspect(&session, |session| {
        session
            .last_action
            .ok_or_else(|| "missing unselected action".into())
    })?;
    let unselected_receipt = inspect(&session, |session| {
        session
            .adapter
            .action_receipt(unselected_id)
            .cloned()
            .ok_or_else(|| "missing unselected receipt".into())
    })?;
    let before = world(&session)?;
    server::inject_durable_crash_after(0);
    let partial = stop_set(&session, &[&older, &newer])?;
    assert_eq!(partial["ok"], false, "{partial}");
    assert_eq!(partial["error"]["code"], "adapter_unavailable");
    assert_eq!(partial["error"]["effect_may_have_occurred"], true);
    assert_eq!(partial["drain_progress"]["original_plans_total"], 2);
    assert_eq!(partial["drain_progress"]["drained"], 1);
    assert_eq!(partial["drain_progress"]["remaining_work"], 1);
    assert_eq!(partial["publication_confirmed"], false);
    assert_eq!(partial["durability_pending"], true);
    assert!(partial["finalize_certificate"].is_null());
    inspect(&session, |session| {
        for id in session.adapter.retained_action_ids() {
            let original = session
                .adapter
                .action_plan_receipt(id)
                .ok_or("missing original receipt")?
                .plan_digest
                .to_hex();
            if original == newer {
                assert!(server::action_fully_drained(&session.adapter, id));
            } else if original == older {
                assert!(!server::action_fully_drained(&session.adapter, id));
            }
        }
        Ok(())
    })?;
    let frontier = world(&session)?;
    assert_ne!(frontier, before);
    assert_eq!(frontier.tick, before.tick);
    let denied = stop_set(&session, &[&newer, &older])?;
    assert_eq!(denied["error"]["code"], "adapter_unavailable", "{denied}");
    assert_eq!(world(&session)?, frontier);
    server::inject_durable_crash_after(100_000);
    let finished = stop_set(&session, &[&newer, &older])?;
    assert_eq!(finished["ok"], true, "{finished}");
    assert_eq!(finished["drain_progress"]["quiescent"], true);
    assert_eq!(finished["drain_progress"]["drained"], 1);
    assert_eq!(
        finished["selected_plans_digest"],
        partial["selected_plans_digest"]
    );
    assert!(finished["finalize_certificate"]["digest"].is_string());
    inspect(&session, |session| {
        assert_eq!(
            session.adapter.action_receipt(unselected_id),
            Some(&unselected_receipt)
        );
        assert!(!server::action_fully_drained(
            &session.adapter,
            unselected_id
        ));
        assert!(session.commit_receipts.contains_key(&unselected));
        Ok(())
    })?;
    Ok(())
}
