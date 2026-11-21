//! Set admission remains live-only; durable effects retain incremental custody.
use super::*;

fn work_order(name: &str) -> Value {
    json!({"action":{"kind":"create_work_order","name":name,"job_token":"BREW_DRINK","amount":20}})
}

fn stop_set(session: &str, digests: &[&str]) -> TestResult<Value> {
    payload(&facade::fortress_cancel(
        Some(session.to_owned()),
        Some("stop_future_steps".to_owned()),
        Some(format!("plans:{}", digests.join(","))),
    ))
}

#[test]
fn recovered_member_refuses_the_entire_set_before_live_work_changes() -> TestResult {
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new("selected-set-recovery-refusal");
    let first = open("74667")?;
    let (recovered, _) = commit(&first, json!([dig()]))?;
    directory.restart();
    let session = open("74667")?;
    let (live, _) = commit(
        &session,
        json!([work_order("live original beside recovered selection")]),
    )?;
    let before = world(&session)?;
    let denied = stop_set(&session, &[&live, &recovered])?;
    assert_eq!(denied["error"]["code"], "conflict", "{denied}");
    assert_eq!(denied["error"]["effect_may_have_occurred"], false);
    assert_eq!(denied["selection_resolved"], false);
    assert_eq!(
        denied["requested_plan_digests"].as_array().map(Vec::len),
        Some(2)
    );
    assert!(denied["finalize_certificate"].is_null());
    assert_eq!(world(&session)?, before);
    // Exact recovery remains available independently; no live authority is restored.
    assert_quiet(&stop(&session, &recovered, "stop_future_steps")?);
    let stopped = stop_set(&session, &[&live])?;
    assert_eq!(stopped["drain_progress"]["quiescent"], true, "{stopped}");
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
