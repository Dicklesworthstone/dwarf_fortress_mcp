//! Foreground selection of an exact original commit, independent of recency.
use super::*;

fn selected_scope(commit: &Json) -> TestResult<String> {
    Ok(format!(
        "plan:{}",
        commit["plan_digest"]
            .as_str()
            .ok_or("plan digest missing")?
    ))
}

#[test]
fn selected_middle_plan_leaves_oldest_latest_and_pending_untouched() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("73640")?;
    let mut plans = Vec::new();
    for name in ["keep oldest", "stop middle", "keep latest"] {
        let committed = plan_and_commit(&session, &order(name, "BREW_DRINK", 20, json!([])))?;
        plans.push((committed, last_effect(&session)?));
    }
    let pending = plan(
        &session,
        &order("keep pending review", "BREW_DRINK", 1, json!([])),
    )?;
    let originals = plans
        .iter()
        .map(|(_, (id, _))| receipt(&session, *id))
        .collect::<TestResult<Vec<_>>>()?;
    let before = snapshot(&session)?;
    in_session(&session, |guard| {
        guard.budget.max_actions = 1;
        Ok(())
    })?;
    let scope = selected_scope(&plans[1].0)?;
    let stopped = cancel(&session, "stop_future_steps", &scope)?;
    assert_certificate(&stopped);
    assert_eq!(stopped["scope"], scope);
    assert_eq!(stopped["plan_digest"], plans[1].0["plan_digest"]);
    assert_eq!(stopped["selection_scope"], "exact_original_commit");
    assert_eq!(stopped["descendant_plans_included"], false);
    assert_eq!(stopped["drain_progress"]["actions_total"], 1);
    assert_eq!(stopped["steps"][0]["action_id"], plans[1].1.0.to_string());
    assert_eq!(snapshot(&session)?.tick, before.tick);
    for index in [0, 2] {
        assert_eq!(receipt(&session, plans[index].1.0)?, originals[index]);
        assert_ne!(
            fields(&session, "work_order", plans[index].1.1)?["status"],
            "cancelled"
        );
    }
    in_session(&session, |guard| {
        assert_eq!(guard.last_action, Some(plans[2].1.0));
        assert_eq!(guard.last_plan_actions, vec![plans[2].1.0]);
        assert_eq!(
            guard.pending.as_ref().map(|entry| entry.digest.as_str()),
            pending["plan_digest"].as_str()
        );
        assert!(guard.open_actions.contains(&plans[0].1.0));
        assert!(guard.open_actions.contains(&plans[2].1.0));
        assert!(!guard.open_actions.contains(&plans[1].1.0));
        Ok(())
    })?;
    let settled = snapshot(&session)?;
    let repeated = cancel(&session, "stop_future_steps", &scope)?;
    assert_certificate(&repeated);
    assert_eq!(snapshot(&session)?, settled);
    Ok(())
}

#[test]
fn selected_plan_preflights_every_original_capability_before_stopping_any_work() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("73641")?;
    let committed = plan_and_commit(
        &session,
        r#"[
        {"action":{"kind":"designate_dig","min":[1,3,10],"max":[10,5,10],"mode":"mine"}},
        {"action":{"kind":"create_work_order","name":"must remain active","job_token":"BREW_DRINK","amount":20}}
    ]"#,
    )?;
    let (ids, grants) = in_session(&session, |guard| {
        let grants = guard.grants.clone();
        guard
            .grants
            .retain(|grant| grant.capability != Capability::Designate);
        Ok((guard.last_plan_actions.clone(), grants))
    })?;
    let before = snapshot(&session)?;
    let originals = ids
        .iter()
        .map(|id| receipt(&session, *id))
        .collect::<TestResult<Vec<_>>>()?;
    let denied = cancel(&session, "stop_future_steps", &selected_scope(&committed)?)?;
    assert_eq!(denied["ok"], false, "{denied}");
    assert_eq!(denied["error"]["code"], "capability_denied");
    assert!(denied["finalize_certificate"].is_null());
    assert_eq!(snapshot(&session)?, before);
    for (id, original) in ids.iter().zip(originals) {
        assert_eq!(receipt(&session, *id)?, original);
    }
    in_session(&session, |guard| {
        guard.grants = grants;
        Ok(())
    })?;
    assert_certificate(&cancel(
        &session,
        "stop_future_steps",
        &selected_scope(&committed)?,
    )?);
    Ok(())
}

#[test]
fn selected_original_failure_stops_work_without_rewriting_historical_proof() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("73642")?;
    set_brewer(&session, false)?;
    let committed = plan_and_commit(
        &session,
        &order("failed original", "BREW_DRINK", 1, json!([])),
    )?;
    let (original_id, entity) = last_effect(&session)?;
    wait(&session, 200)?;
    let historical = receipt(&session, original_id)?;
    assert_eq!(historical.state, CommitState::Failed);
    let latest = plan_and_commit(
        &session,
        &order("unrelated later work", "BREW_DRINK", 20, json!([])),
    )?;
    let latest_id = last_effect(&session)?.0;
    let later_receipt = receipt(&session, latest_id)?;
    let result = cancel(&session, "stop_future_steps", &selected_scope(&committed)?)?;
    assert_certificate(&result);
    assert_eq!(result["steps"][0]["before"], "Failed");
    assert_eq!(result["steps"][0]["after"], "Failed");
    assert_eq!(receipt(&session, original_id)?, historical);
    assert_eq!(receipt(&session, latest_id)?, later_receipt);
    assert_eq!(
        fields(&session, "work_order", entity)?["status"],
        "cancelled"
    );
    assert_ne!(result["plan_digest"], latest["plan_digest"]);
    Ok(())
}

#[test]
fn pending_foreign_and_lost_commit_custody_never_fall_back_to_latest() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open_session("73643", true, Some("starter_fortress"), true)?;
    let other = open_session("73643", true, None, true)?;
    let original = plan_and_commit(&owner, &order("owned only", "BREW_DRINK", 20, json!([])))?;
    plan_and_commit(
        &other,
        &order("other session work", "BREW_DRINK", 20, json!([])),
    )?;
    let scope = selected_scope(&original)?;
    let before = snapshot(&owner)?;
    let foreign = cancel(&other, "stop_future_steps", &scope)?;
    assert_eq!(foreign["ok"], false, "{foreign}");
    assert_eq!(foreign["error"]["code"], "conflict");
    assert_eq!(snapshot(&owner)?, before);
    let pending = plan(&owner, &order("not committed", "BREW_DRINK", 1, json!([])))?;
    let denied = cancel(&owner, "stop_future_steps", &selected_scope(&pending)?)?;
    assert_eq!(denied["error"]["code"], "conflict", "{denied}");
    assert_eq!(snapshot(&owner)?, before);
    // Restore/restart intentionally clears process-local commit custody. An
    // adapter receipt or retained original source alone cannot replace it.
    in_session(&owner, |guard| {
        guard.commit_receipts.clear();
        Ok(())
    })?;
    let lost = cancel(&owner, "stop_future_steps", &scope)?;
    assert_eq!(lost["error"]["code"], "conflict", "{lost}");
    assert_eq!(snapshot(&owner)?, before);
    Ok(())
}

#[test]
fn complete_original_receipt_is_required_before_any_selected_effect() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("73644")?;
    let committed = plan_and_commit(
        &session,
        r#"[
        {"action":{"kind":"create_work_order","name":"original first","job_token":"BREW_DRINK","amount":20}},
        {"action":{"kind":"create_work_order","name":"original second","job_token":"BREW_DRINK","amount":20}}
    ]"#,
    )?;
    let digest = committed["plan_digest"]
        .as_str()
        .ok_or("digest missing")?
        .to_owned();
    let original = in_session(&session, |guard| {
        guard
            .commit_receipts
            .get(&digest)
            .cloned()
            .ok_or_else(|| fixture_error("receipt missing"))
    })?;
    let before = snapshot(&session)?;
    for defect in [
        "missing",
        "duplicate",
        "reordered",
        "wrong_step",
        "wrong_owner",
    ] {
        let mut value = parsed(&original)?;
        match defect {
            "missing" => {
                value["actions"]
                    .as_array_mut()
                    .ok_or("actions missing")?
                    .pop();
            }
            "duplicate" => value["actions"][1] = value["actions"][0].clone(),
            "reordered" => value["actions"]
                .as_array_mut()
                .ok_or("actions missing")?
                .reverse(),
            "wrong_step" => value["actions"][1]["step"] = json!(999),
            "wrong_owner" => value["session_id"] = json!("0"),
            _ => return Err("unknown receipt defect".into()),
        }
        in_session(&session, |guard| {
            guard
                .commit_receipts
                .insert(digest.clone(), value.to_string());
            Ok(())
        })?;
        let result = cancel(&session, "stop_future_steps", &format!("plan:{digest}"))?;
        assert_eq!(result["error"]["code"], "conflict", "{defect}: {result}");
        assert_eq!(snapshot(&session)?, before);
    }
    in_session(&session, |guard| {
        guard.commit_receipts.insert(digest.clone(), original);
        Ok(())
    })?;
    assert_certificate(&cancel(
        &session,
        "stop_future_steps",
        &format!("plan:{digest}"),
    )?);
    Ok(())
}

#[test]
fn alias_preserves_requested_identity_and_discloses_actual_original() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("73645")?;
    let committed = plan_and_commit(
        &session,
        &order("rebased receipt", "BREW_DRINK", 20, json!([])),
    )?;
    let actual = committed["plan_digest"]
        .as_str()
        .ok_or("digest missing")?
        .to_owned();
    let alias = Digest32::of_bytes(b"reviewed pre-rebase selector").to_hex();
    in_session(&session, |guard| {
        let raw = guard
            .commit_receipts
            .get(&actual)
            .cloned()
            .ok_or_else(|| fixture_error("original receipt missing"))?;
        guard.commit_receipts.insert(alias.clone(), raw);
        Ok(())
    })?;
    let result = cancel(&session, "stop_future_steps", &format!("plan:{alias}"))?;
    assert_certificate(&result);
    assert_eq!(result["requested_plan_digest"], alias);
    assert_eq!(result["plan_digest"], actual);
    assert_eq!(result["scope"], format!("plan:{alias}"));
    Ok(())
}

#[test]
fn selector_and_source_budgets_refuse_before_work_changes() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("73646")?;
    let original = plan_and_commit(
        &session,
        &order("bounded original", "BREW_DRINK", 20, json!([])),
    )?;
    let before = snapshot(&session)?;
    for scope in [
        "plan:abc".to_owned(),
        format!("plan:{}", "A".repeat(64)),
        format!("plan:{}~steps", "a".repeat(64)),
        format!("plan:{}", "0".repeat(100_000)),
    ] {
        let result = cancel(&session, "stop_future_steps", &scope)?;
        assert_eq!(result["error"]["code"], "invalid_request", "{result}");
        assert_eq!(snapshot(&session)?, before);
    }
    let budget = in_session(&session, |guard| {
        let budget = guard.budget;
        guard.budget.max_bytes = 64;
        Ok(budget)
    })?;
    let result = parsed(&cancel_in_scope(
        Some(session.clone()),
        Some("stop_future_steps".to_owned()),
        Some(selected_scope(&original)?),
    ))?;
    assert_eq!(result["error"]["code"], "budget_exceeded", "{result}");
    assert_eq!(snapshot(&session)?, before);
    in_session(&session, |guard| {
        guard.budget = budget;
        Ok(())
    })?;
    Ok(())
}

#[test]
fn explanation_offers_only_current_owned_cancellation_without_changing_state() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("73647")?;
    let original = plan_and_commit(
        &session,
        &order("explain selected stop", "BREW_DRINK", 20, json!([])),
    )?;
    let digest = Digest32::from_hex(original["plan_digest"].as_str().ok_or("digest missing")?)
        .ok_or("invalid digest")?;
    let before = snapshot(&session)?;
    let offer = in_session(&session, |guard| {
        let request_id = guard.next_request_id;
        let offered = plan_cancellation::offer(guard, digest);
        assert_eq!(guard.next_request_id, request_id);
        Ok(offered)
    })?;
    assert_eq!(offer["available"], true, "{offer}");
    assert_eq!(
        offer["request"]["arguments"]["scope"],
        selected_scope(&original)?
    );
    assert_eq!(offer["request"]["arguments"]["mode"], "stop_future_steps");
    assert_eq!(snapshot(&session)?, before);
    in_session(&session, |guard| {
        guard
            .grants
            .retain(|grant| grant.capability != Capability::ConfigureProduction);
        let denied = plan_cancellation::offer(guard, digest);
        assert_eq!(denied["available"], false, "{denied}");
        assert_eq!(denied["code"], "capability_denied");
        Ok(())
    })?;
    assert_eq!(snapshot(&session)?, before);
    Ok(())
}
