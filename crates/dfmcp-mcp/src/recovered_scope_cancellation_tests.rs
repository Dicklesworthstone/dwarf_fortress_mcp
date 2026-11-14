//! Real public-facade oldest selection over recovered and new original work.
use super::*;

fn oldest(session: &str, mode: &str) -> TestResult<Value> {
    payload(&facade::fortress_cancel(
        Some(session.to_owned()), Some(mode.to_owned()), Some("oldest_open_plan".into()),
    ))
}

fn named_order(name: &str) -> Value {
    let mut order = blocked_order();
    order["action"]["name"] = json!(name);
    order
}

#[test]
fn oldest_drains_recovered_originals_before_new_live_work_without_consuming_pending_review() -> TestResult {
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new("oldest-mixed");
    let first = open("7864101")?;
    let (first_digest, _) = commit(&first, json!([dig()]))?;
    let (second_digest, _) = commit(&first, json!([named_order("second recovered")]))?;
    directory.restart();
    let session = open("7864101")?;
    let (live_digest, _) = commit(&session, json!([named_order("new live original")]))?;
    let pending = plan(&session, json!([{"action":{"kind":"pause","paused":true}}]))?;
    let before = world(&session)?;
    inspect(&session, |session| {
        session.carried.reverse();
        session.budget.max_actions = 1;
        Ok(())
    })?;
    for digest in [&first_digest, &second_digest] {
        let stopped = oldest(&session, "stop_future_steps")?;
        assert_quiet(&stopped);
        assert_eq!(stopped["scope"], "oldest_open_plan");
        assert_eq!(stopped["plan_digest"].as_str(), Some(digest.as_str()));
        assert_eq!(stopped["finalize_certificate"]["scope"], "oldest_open_plan");
        assert_eq!(stopped["finalize_certificate"]["plan_digest"], stopped["plan_digest"]);
        assert_eq!(stopped["finalize_certificate"]["schema"], "dfmcp-recovered-plan-drain-certificate/2");
        assert_eq!(stopped["drain_progress"]["drained"], 1);
        inspect(&session, |session| {
            let live = session.last_action.ok_or("new live action disappeared")?;
            assert!(!server::action_fully_drained(&session.adapter, live));
            assert_eq!(session.pending.as_ref().map(|p| p.digest.as_str()), pending["plan_digest"].as_str());
            Ok(())
        })?;
    }
    let stopped = oldest(&session, "stop_future_steps")?;
    assert_eq!(stopped["ok"], true, "{stopped}");
    assert_eq!(stopped["scope"], "oldest_open_plan");
    assert_eq!(stopped["plan_digest"], live_digest);
    assert_eq!(stopped["selection_origin"], "current_process");
    assert_eq!(stopped["drain_progress"]["quiescent"], true);
    assert_eq!(stopped["finalize_certificate"]["plan_digest"], stopped["plan_digest"]);
    assert_eq!(stopped["finalize_certificate"]["scope"], "oldest_open_plan");
    let mut material = stopped["finalize_certificate"].clone();
    let expected = material.as_object_mut().ok_or("certificate missing")?.remove("digest");
    material.as_object_mut().ok_or("certificate missing")?.remove("statement");
    assert_eq!(expected, Some(json!(Digest32::of_bytes(material.to_string().as_bytes()).to_hex())));
    assert_eq!(oldest(&session, "stop_future_steps")?["error"]["code"], "conflict");
    assert_eq!(world(&session)?.tick, before.tick);
    inspect(&session, |session| {
        assert_eq!(session.pending.as_ref().map(|p| p.digest.as_str()), pending["plan_digest"].as_str());
        Ok(())
    })?;
    Ok(())
}

#[test]
fn oldest_uses_fresh_physical_work_and_preserves_original_failed_proof() -> TestResult {
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new("oldest-failed");
    let first = open("7864102")?;
    let (first_digest, planned) = commit(&first, json!([named_order("expired oldest")]))?;
    let deadline = planned["steps"][0]["obligation"]["deadline_tick"].as_u64().ok_or("missing deadline")?;
    let failed = wait(&first, deadline - world(&first)?.tick.0 + 1)?;
    assert_eq!(failed["commit_state"], "Failed", "{failed}");
    let (second_digest, _) = commit(&first, json!([dig()]))?;
    directory.restart();
    let session = open("7864102")?;
    let proof = inspect(&session, |session| {
        let row = session.carried.iter_mut().find(|row| row.plan_digest.to_hex() == first_digest)
            .ok_or("missing oldest")?;
        let step = row.work_step.as_ref().ok_or("missing work")?;
        // Simulate stale cached quiet. Selection still reads the exact current
        // original physical entity before deciding that this plan can be skipped.
        row.work_state = EffectWorkState::Quiescent {
            entity_id: Some(dfmcp_intent::effects::created_entity_id(&step.idempotency_key, 0)),
        };
        Ok(row.proof_anchor)
    })?;
    let stopped = oldest(&session, "stop_future_steps")?;
    assert_quiet(&stopped);
    assert_eq!(stopped["plan_digest"], first_digest);
    assert_eq!(stopped["drain_progress"]["terminal_work_stopped"], 1);
    assert_eq!(stopped["drain_progress"]["cancelled"], 0);
    inspect(&session, |session| {
        let row = session.carried.iter_mut().find(|row| row.plan_digest.to_hex() == first_digest)
            .ok_or("missing stopped oldest")?;
        assert_eq!(row.state, "failed");
        assert_eq!(row.proof_anchor, proof);
        let step = row.work_step.as_ref().ok_or("missing work")?;
        // The opposite stale cache must not keep selecting an already-quiet
        // terminal original instead of allowing the next original to drain.
        row.work_state = EffectWorkState::Active {
            entity_id: dfmcp_intent::effects::created_entity_id(&step.idempotency_key, 0),
        };
        Ok(())
    })?;
    let next = oldest(&session, "stop_future_steps")?;
    assert_quiet(&next);
    assert_eq!(next["plan_digest"], second_digest);
    Ok(())
}

#[test]
fn oldest_never_skips_unavailable_or_malformed_earlier_recovered_custody() -> TestResult {
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new("oldest-custody");
    let first = open("7864103")?;
    let (first_digest, _) = commit(&first, json!([dig()]))?;
    let (second_digest, _) = commit(&first, json!([named_order("later recoverable")]))?;
    directory.restart();
    let session = open("7864103")?;
    let before = world(&session)?;
    let source = inspect(&session, |session| {
        let source = session.objectives.clone();
        session.objectives.retain(|goal| goal.plan_digest.to_hex() != first_digest);
        Ok(source)
    })?;
    let refused = oldest(&session, "stop_future_steps")?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(world(&session)?, before);
    inspect(&session, |session| { session.objectives = source; Ok(()) })?;
    assert_quiet(&stop(&session, &first_digest, "stop_future_steps")?);
    let quiet_frontier = world(&session)?;
    let original = inspect(&session, |session| {
        let row = session.carried.iter_mut().find(|row| row.plan_digest.to_hex() == first_digest)
            .ok_or("missing original")?;
        let original = row.clone();
        row.proof = Predicate::False;
        Ok(original)
    })?;
    let refused = oldest(&session, "stop_future_steps")?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "corrupt_ledger");
    assert_eq!(world(&session)?, quiet_frontier);
    inspect(&session, |session| {
        *session.carried.iter_mut().find(|row| row.plan_digest.to_hex() == first_digest)
            .ok_or("missing original")? = original;
        Ok(())
    })?;
    let stopped = oldest(&session, "stop_future_steps")?;
    assert_quiet(&stopped);
    assert_eq!(stopped["plan_digest"], second_digest);
    Ok(())
}

#[test]
fn oldest_preflights_authority_source_output_and_shared_deadline_before_effects() -> TestResult {
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new("oldest-bounds");
    let first = open("7864104")?;
    let (digest, _) = commit(&first, json!([dig()]))?;
    directory.restart();
    let session = open("7864104")?;
    let before = world(&session)?;
    let (grants, budget) = inspect(&session, |session| {
        let original = (session.grants.clone(), session.budget);
        session.grants.retain(|grant| grant.capability != Capability::Designate);
        Ok(original)
    })?;
    assert_eq!(oldest(&session, "stop_future_steps")?["error"]["code"], "capability_denied");
    inspect(&session, |session| { session.grants = grants; Ok(()) })?;
    for narrowed in [
        WorkBudget { max_entities: 1, ..budget },
        WorkBudget { max_bytes: 6_000, ..budget },
        WorkBudget { max_output_tokens: 1_499, ..budget },
    ] {
        inspect(&session, |session| { session.budget = narrowed; Ok(()) })?;
        let refused = oldest(&session, "stop_future_steps")?;
        assert_eq!(refused["error"]["code"], "budget_exceeded", "{refused}");
        assert_eq!(world(&session)?, before);
    }
    inspect(&session, |session| { session.budget = budget; Ok(()) })?;
    assert_eq!(oldest(&session, "compensate_reversible")?["error"]["code"], "conflict");
    assert_eq!(world(&session)?, before);
    assert_quiet(&stop(&session, &digest, "stop_future_steps")?);
    let (live, _) = commit(&session, json!([named_order("new live under shared deadline")]))?;
    let after = world(&session)?;
    inspect(&session, |session| {
        let expired = Instant::now().checked_sub(Duration::from_secs(120)).ok_or("clock underflow")?;
        let refused: Value = serde_json::from_str(&server::drain_oldest_live_at(
            session, CancelMode::StopFutureSteps, expired, 0, 0,
        ))?;
        assert_eq!(refused["error"]["code"], "budget_exceeded", "{refused}");
        assert_eq!(session.adapter.snapshot(), &after);
        let refused: Value = serde_json::from_str(&server::drain_oldest_live_at(
            session, CancelMode::StopFutureSteps, Instant::now(), budget.max_bytes, 0,
        ))?;
        assert_eq!(refused["error"]["code"], "budget_exceeded", "{refused}");
        assert_eq!(session.adapter.snapshot(), &after);
        Ok(())
    })?;
    assert_eq!(oldest(&session, "stop_future_steps")?["plan_digest"], live);
    Ok(())
}

#[test]
fn oldest_skips_withdrawn_quiet_original_and_retains_bounded_emergency_evidence() -> TestResult {
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new("oldest-withdrawn");
    let first = open("7864105")?;
    let (first_digest, _) = commit(&first, json!([dig()]))?;
    let (second_digest, _) = commit(&first, json!([named_order("after withdrawal")]))?;
    directory.restart();
    let session = open("7864105")?;
    assert_quiet(&stop(&session, &first_digest, "stop_future_steps")?);
    let withdrawn = payload(&facade::fortress_cancel(Some(session.clone()),
        Some("withdraw_goal".into()), Some(format!("goal:{first_digest}"))))?;
    assert_eq!(withdrawn["ok"], true, "{withdrawn}");
    inspect(&session, |session| { session.budget.max_output_tokens = 1_500; Ok(()) })?;
    let raw = facade::fortress_cancel(Some(session.clone()),
        Some("emergency_pause_and_drain".into()), Some("oldest_open_plan".into()));
    assert!(raw.len() <= 6_000, "{}: {raw}", raw.len());
    let stopped = payload(&raw)?;
    assert_quiet(&stopped);
    assert_eq!(stopped["plan_digest"], second_digest);
    assert_eq!(stopped["scope"], "oldest_open_plan");
    assert_eq!(stopped["finalize_certificate"]["scope"], "oldest_open_plan");
    assert_eq!(stopped["emergency_pause"]["publication_confirmed"], true);
    assert_eq!(stopped["paused"], true);
    Ok(())
}

#[test]
fn oldest_failed_publication_reports_partial_exact_original_and_stops_before_newer_work() -> TestResult {
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new("oldest-publication");
    let first = open("7864106")?;
    let (first_digest, _) = commit(&first, json!([dig(), named_order("older second step")]))?;
    let (second_digest, _) = commit(&first, json!([named_order("newer original")]))?;
    directory.restart();
    let session = open("7864106")?;
    server::inject_durable_crash_after(0);
    let refused = oldest(&session, "stop_future_steps")?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["scope"], "oldest_open_plan");
    assert_eq!(refused["plan_digest"], first_digest);
    assert_eq!(refused["error"]["code"], "adapter_unavailable");
    assert_eq!(refused["drain_progress"]["drained"], 1);
    assert_eq!(refused["drain_progress"]["remaining_work"], 1);
    assert_eq!(refused["drain_progress"]["quiescent"], false);
    assert!(refused["finalize_certificate"].is_null());
    assert_eq!(refused["publication_confirmed"], false);
    inspect(&session, |session| {
        let later = session.carried.iter().find(|row| row.plan_digest.to_hex() == second_digest)
            .ok_or("missing later original")?;
        assert_eq!(later.state, "dispatched");
        assert!(!later.work_state.is_quiescent());
        Ok(())
    })?;
    Ok(())
}
