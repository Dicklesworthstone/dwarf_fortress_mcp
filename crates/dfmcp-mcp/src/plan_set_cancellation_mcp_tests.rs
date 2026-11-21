//! Explicit selected original sets cross the same real facade as single drains.
use super::*;

fn selection(commits: &[&Json]) -> TestResult<String> {
    Ok(format!(
        "plans:{}",
        commits
            .iter()
            .map(|commit| commit["plan_digest"].as_str().ok_or("plan digest missing"))
            .collect::<std::result::Result<Vec<_>, _>>()?
            .join(",")
    ))
}

#[test]
fn selected_set_preserves_unselected_pending_and_original_identity_on_repeat() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("74660")?;
    let mut commits = Vec::new();
    for name in ["selected older", "keep middle", "selected newer"] {
        let committed = plan_and_commit(&session, &order(name, "BREW_DRINK", 20, json!([])))?;
        commits.push((committed, last_effect(&session)?));
    }
    let pending = plan(
        &session,
        &order("pending review survives", "BREW_DRINK", 1, json!([])),
    )?;
    let before = snapshot(&session)?;
    let unselected = receipt(&session, commits[1].1.0)?;
    let selected_scope = selection(&[&commits[2].0, &commits[0].0])?;
    let stopped = cancel(&session, "stop_future_steps", &selected_scope)?;
    assert_certificate(&stopped);
    assert_eq!(stopped["selection_scope"], "exact_original_set");
    assert_eq!(stopped["drain_progress"]["original_plans_total"], 2);
    assert_eq!(stopped["drain_progress"]["actions_total"], 2);
    assert_eq!(stopped["drain_progress"]["drained"], 2);
    assert_eq!(stopped["descendant_plans_included"], false);
    assert_eq!(stopped["unselected_plans_included"], false);
    assert_eq!(
        stopped["finalize_certificate"]["selected_plans"],
        stopped["selected_plans"]
    );
    assert_eq!(
        stopped["finalize_certificate"]["selected_plans_digest"],
        stopped["selected_plans_digest"]
    );
    assert!(stopped["finalize_certificate"]["original_action_certificate"]["digest"].is_string());
    assert_eq!(snapshot(&session)?.tick, before.tick);
    assert_eq!(receipt(&session, commits[1].1.0)?, unselected);
    assert_ne!(
        fields(&session, "work_order", commits[1].1.1)?["status"],
        "cancelled"
    );
    in_session(&session, |guard| {
        assert_eq!(
            guard.pending.as_ref().map(|entry| entry.digest.as_str()),
            pending["plan_digest"].as_str()
        );
        assert_eq!(guard.last_action, Some(commits[2].1.0));
        assert_eq!(guard.last_plan_actions, vec![commits[2].1.0]);
        assert_eq!(guard.open_actions, vec![commits[1].1.0]);
        Ok(())
    })?;
    let settled = snapshot(&session)?;
    let repeated = cancel(
        &session,
        "stop_future_steps",
        &selection(&[&commits[0].0, &commits[2].0])?,
    )?;
    assert_certificate(&repeated);
    assert_eq!(
        repeated["selected_plans_digest"],
        stopped["selected_plans_digest"]
    );
    assert_eq!(repeated["selected_plans"], stopped["selected_plans"]);
    assert_eq!(repeated["scope"], stopped["scope"]);
    assert_eq!(repeated["drain_progress"]["drained"], 0);
    assert_eq!(repeated["drain_progress"]["already_terminal"], 2);
    assert_eq!(snapshot(&session)?, settled);
    Ok(())
}

#[test]
fn every_selected_capability_is_checked_before_stop_or_emergency_pause() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("74661")?;
    let older = plan_and_commit(
        &session,
        r#"[{"action":{"kind":"designate_dig","min":[1,3,10],"max":[10,5,10],"mode":"mine"}}]"#,
    )?;
    let newer = plan_and_commit(
        &session,
        &order("must not stop early", "BREW_DRINK", 20, json!([])),
    )?;
    let scope = selection(&[&newer, &older])?;
    in_session(&session, |guard| {
        guard
            .grants
            .retain(|grant| grant.capability != Capability::Designate);
        Ok(())
    })?;
    let before = snapshot(&session)?;
    for mode in ["stop_future_steps", "emergency_pause_and_drain"] {
        let denied = cancel(&session, mode, &scope)?;
        assert_eq!(denied["error"]["code"], "capability_denied", "{denied}");
        assert_eq!(denied["selection_resolved"], true);
        assert_eq!(
            denied["selected_plans"]["originals"]
                .as_array()
                .map(Vec::len),
            Some(2)
        );
        assert_eq!(denied["error"]["effect_may_have_occurred"], false);
        assert!(denied["finalize_certificate"].is_null());
        assert_eq!(snapshot(&session)?, before);
    }
    Ok(())
}

#[test]
fn all_inverse_authority_and_entity_custody_precede_any_selected_stop() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("74662")?;
    let unit = dfmcp_core::EntityId::new(1003);
    let digest = in_session(&session, |guard| {
        let raw =
            r#"[{"action":{"kind":"designate_dig","min":[1,3,10],"max":[10,5,10],"mode":"mine"}}]"#
                .to_owned();
        let summary = "excavation with independently scoped labor inverse".to_owned();
        let mut intent = semantic_intent(
            dfmcp_core::IntentId::new(74662),
            guard.adapter.snapshot(),
            summary.clone(),
            &raw,
        )?;
        intent.requested_actions[0].compensation = Some(dfmcp_intent::Action::SetLabor {
            units: vec![unit],
            labor: "BREW".to_owned(),
            enabled: false,
        });
        let prepared = dfmcp_intent::StaticPlanner::default().prepare_laboratory(
            guard.adapter.snapshot(),
            &intent,
            &read_context(guard),
        )?;
        let digest = prepared.digest.to_hex();
        guard.pending = Some(PendingPlan {
            plan: prepared,
            digest: digest.clone(),
            source: PlanSource::Actions { summary, raw },
        });
        Ok(digest)
    })?;
    let older = parsed(&facade::fortress_commit(Some(session.clone()), digest))?;
    assert_eq!(older["ok"], true, "{older}");
    let newer = plan_and_commit(
        &session,
        &order("later independent production", "BREW_DRINK", 20, json!([])),
    )?;
    let scope = selection(&[&newer, &older])?;
    let grants = in_session(&session, |guard| {
        let grants = guard.grants.clone();
        guard
            .grants
            .retain(|grant| grant.capability != Capability::ConfigureLabor);
        Ok(grants)
    })?;
    let before = snapshot(&session)?;
    let denied = cancel(&session, "compensate_reversible", &scope)?;
    assert_eq!(denied["error"]["code"], "capability_denied", "{denied}");
    assert_eq!(snapshot(&session)?, before);
    in_session(&session, |guard| {
        guard.grants = grants;
        guard.leases.manager.acquire_entity_lease(
            dfmcp_core::SessionId::new(999),
            unit,
            true,
            guard.adapter.snapshot().tick,
            100,
        )?;
        Ok(())
    })?;
    let blocked = cancel(&session, "compensate_reversible", &scope)?;
    assert_eq!(blocked["error"]["code"], "conflict", "{blocked}");
    assert_eq!(snapshot(&session)?, before);
    // No inverse is applied in stop mode, so its independent entity is irrelevant.
    assert_certificate(&cancel(&session, "stop_future_steps", &scope)?);
    Ok(())
}

#[test]
fn aliases_are_preserved_but_duplicate_actual_originals_and_bad_members_refuse() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("74663")?;
    let first = plan_and_commit(
        &session,
        &order("alias original", "BREW_DRINK", 20, json!([])),
    )?;
    let second = plan_and_commit(
        &session,
        &order("other selected original", "BREW_DRINK", 20, json!([])),
    )?;
    let actual = first["plan_digest"]
        .as_str()
        .ok_or("digest missing")?
        .to_owned();
    let alias = Digest32::of_bytes(b"selected-set pre-rebase identity").to_hex();
    in_session(&session, |guard| {
        let raw = guard
            .commit_receipts
            .get(&actual)
            .cloned()
            .ok_or_else(|| fixture_error("receipt missing"))?;
        guard.commit_receipts.insert(alias.clone(), raw);
        Ok(())
    })?;
    let before = snapshot(&session)?;
    let unknown = Digest32::of_bytes(b"unknown selected original").to_hex();
    for (scope, code) in [
        (format!("plans:{actual},{actual}"), "invalid_request"),
        (format!("plans:{actual},{alias}"), "invalid_request"),
        (format!("plans:{actual},{unknown}"), "conflict"),
        (format!("plans:{actual},"), "invalid_request"),
        (
            format!("plans:{actual},{}", "A".repeat(64)),
            "invalid_request",
        ),
        ("plans:".to_owned(), "invalid_request"),
        (
            format!(
                "plans:{}",
                (1..=9)
                    .map(|n| format!("{n:064x}"))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            "invalid_request",
        ),
    ] {
        let denied = cancel(&session, "emergency_pause_and_drain", &scope)?;
        assert_eq!(denied["error"]["code"], code, "{denied}");
        assert_eq!(snapshot(&session)?, before);
    }
    let scope = format!(
        "plans:{alias},{}",
        second["plan_digest"].as_str().ok_or("digest missing")?
    );
    let stopped = cancel(&session, "stop_future_steps", &scope)?;
    assert_certificate(&stopped);
    let identity = stopped["selected_plans"]["originals"]
        .as_array()
        .ok_or("originals missing")?
        .iter()
        .find(|row| row["requested_plan_digest"] == alias)
        .ok_or("alias missing")?;
    assert_eq!(identity["plan_digest"], actual);
    Ok(())
}

#[test]
fn selected_set_requires_aggregate_action_source_and_complete_output_allowance() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("74664")?;
    let first = plan_and_commit(
        &session,
        &order("aggregate first", "BREW_DRINK", 20, json!([])),
    )?;
    let second = plan_and_commit(
        &session,
        &order("aggregate second", "BREW_DRINK", 20, json!([])),
    )?;
    let scope = selection(&[&first, &second])?;
    let budget = in_session(&session, |guard| Ok(guard.budget))?;
    let before = snapshot(&session)?;
    for restricted in [
        dfmcp_core::WorkBudget {
            max_actions: 1,
            ..budget
        },
        dfmcp_core::WorkBudget {
            max_bytes: 64,
            ..budget
        },
        dfmcp_core::WorkBudget {
            max_output_tokens: 1_500,
            ..budget
        },
        dfmcp_core::WorkBudget {
            max_entities: 1,
            ..budget
        },
    ] {
        in_session(&session, |guard| {
            guard.budget = restricted;
            Ok(())
        })?;
        let denied = parsed(&cancel_in_scope(
            Some(session.clone()),
            Some("stop_future_steps".to_owned()),
            Some(scope.clone()),
        ))?;
        assert_eq!(denied["error"]["code"], "budget_exceeded", "{denied}");
        assert_eq!(denied["error"]["effect_may_have_occurred"], false);
        assert_eq!(
            denied["requested_plan_digests"].as_array().map(Vec::len),
            Some(2)
        );
        assert_eq!(snapshot(&session)?, before);
    }
    in_session(&session, |guard| {
        guard.budget = budget;
        Ok(())
    })?;
    assert_certificate(&cancel(&session, "stop_future_steps", &scope)?);
    Ok(())
}

#[test]
fn terminal_original_proof_survives_selected_set_physical_cleanup() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("74665")?;
    set_brewer(&session, false)?;
    let failed = plan_and_commit(
        &session,
        &order("failed selected original", "BREW_DRINK", 1, json!([])),
    )?;
    let failed_id = last_effect(&session)?.0;
    wait(&session, 200)?;
    let historical = receipt(&session, failed_id)?;
    assert_eq!(historical.state, CommitState::Failed);
    let newer = plan_and_commit(
        &session,
        &order("newer selected original", "BREW_DRINK", 20, json!([])),
    )?;
    let result = cancel(
        &session,
        "stop_future_steps",
        &selection(&[&failed, &newer])?,
    )?;
    assert_certificate(&result);
    assert_eq!(result["drain_progress"]["terminal_work_stopped"], 1);
    assert_eq!(receipt(&session, failed_id)?, historical);
    in_session(&session, |guard| {
        assert!(
            guard
                .objectives
                .iter()
                .all(|goal| goal.resource_goal(guard).withdrawal.is_none())
        );
        Ok(())
    })?;
    Ok(())
}

#[test]
fn foreign_or_lost_live_custody_never_cancels_the_other_selected_original() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open_session("74666", true, Some("starter_fortress"), true)?;
    let other = open_session("74666", true, None, true)?;
    let foreign = plan_and_commit(
        &owner,
        &order("foreign original", "BREW_DRINK", 20, json!([])),
    )?;
    let owned = plan_and_commit(
        &other,
        &order("must remain owned", "BREW_DRINK", 20, json!([])),
    )?;
    let before = snapshot(&other)?;
    let denied = cancel(
        &other,
        "stop_future_steps",
        &selection(&[&owned, &foreign])?,
    )?;
    assert_eq!(denied["error"]["code"], "conflict", "{denied}");
    assert_eq!(denied["selection_resolved"], false);
    assert!(denied["selected_plans"].is_null());
    assert_eq!(snapshot(&other)?, before);
    in_session(&other, |guard| {
        guard.commit_receipts.clear();
        Ok(())
    })?;
    let lost = cancel(&other, "stop_future_steps", &selection(&[&owned])?)?;
    assert_eq!(lost["error"]["code"], "conflict", "{lost}");
    assert_eq!(snapshot(&other)?, before);
    Ok(())
}

#[test]
fn selected_drain_refusal_and_reordered_repeat_replay_with_identical_bundle() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("74669")?;
    let first = plan_and_commit(
        &session,
        &order("replay first original", "BREW_DRINK", 20, json!([])),
    )?;
    let second = plan_and_commit(
        &session,
        &order("replay second original", "BREW_DRINK", 20, json!([])),
    )?;
    let unknown = Digest32::of_bytes(b"replayed missing set member").to_hex();
    let refused = cancel(
        &session,
        "stop_future_steps",
        &format!(
            "plans:{unknown},{}",
            first["plan_digest"].as_str().ok_or("digest missing")?
        ),
    )?;
    assert_eq!(refused["error"]["code"], "conflict", "{refused}");
    let scope = selection(&[&second, &first])?;
    assert_certificate(&cancel(&session, "stop_future_steps", &scope)?);
    assert_certificate(&cancel(
        &session,
        "stop_future_steps",
        &selection(&[&first, &second])?,
    )?);
    let bundle = replay_bundle_for(&session)?;
    assert_eq!(bundle["replayable"], true, "{bundle}");
    assert_eq!(bundle["calls"][6]["arguments"]["scope"], scope);
    let report = crate::replay::replay_bundle(&bundle);
    assert_eq!(report["ok"], true, "{report}");
    let replayed = report["replay_session_id"]
        .as_str()
        .ok_or("replay session missing")?;
    assert_ne!(replayed, session);
    assert_eq!(replay_bundle_for(replayed)?, bundle);
    Ok(())
}

#[test]
fn eight_originals_fit_and_keep_complete_identity_in_actual_facade_outcome() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("74670")?;
    let mut originals = Vec::new();
    for index in 0..8 {
        originals.push(plan_and_commit(
            &session,
            &order(
                &format!("bounded set original {index}"),
                "BREW_DRINK",
                20,
                json!([]),
            ),
        )?);
    }
    let scope = selection(&originals.iter().collect::<Vec<_>>())?;
    let stopped = cancel(&session, "stop_future_steps", &scope)?;
    assert_certificate(&stopped);
    assert_eq!(stopped["drain_progress"]["original_plans_total"], 8);
    assert_eq!(stopped["drain_progress"]["drained"], 8);
    assert_eq!(
        stopped["selected_plans"]["originals"]
            .as_array()
            .map(Vec::len),
        Some(8)
    );
    assert_eq!(
        stopped["finalize_certificate"]["selected_plans"],
        stopped["selected_plans"]
    );
    Ok(())
}
