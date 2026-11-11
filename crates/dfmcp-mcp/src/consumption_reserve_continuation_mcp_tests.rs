//! Foreground continuation of an unfinished original future reserve while
//! currently observed stock still meets the requested minima.

use super::*;

fn reserve(session: &str, until: u64) -> TestResult<Value> {
    parsed(&facade::fortress_plan(
        Some(session.to_owned()),
        None,
        None,
        None,
        Some(
            json!({
                "template":"production", "reserve_until_tick":until,
                "quotas":[{"item":"DRINK","minimum":40},{"item":"FOOD","minimum":60}],
            })
            .to_string(),
        ),
    ))
}

fn stop(session: &str) -> TestResult<Value> {
    let stopped = parsed(&facade::fortress_cancel(
        Some(session.to_owned()),
        Some("stop_future_steps".to_owned()),
        Some("plan".to_owned()),
    ))?;
    assert_eq!(stopped["ok"], true, "{stopped}");
    Ok(stopped)
}

fn pending(session: &str) -> TestResult<(PreparedPlan, dfmcp_lab::durable::DurablePlanSource)> {
    in_session(session, |guard| {
        let pending = guard
            .pending
            .as_ref()
            .ok_or_else(|| fixture_error("reserve pending missing"))?;
        Ok((pending.plan.clone(), pending.source.durable()))
    })
}

fn canceled_reserve(session: &str) -> TestResult<(String, Value)> {
    let original = reserve(session, 2401)?;
    assert_eq!(original["ok"], true, "{original}");
    let root = identity(&original, "plan_digest")?;
    commit_ok(session, &original)?;
    stop(session)?;
    wait(session, 1)?;
    let observation = goal(session, &root)?;
    assert_eq!(observation["predicate_truth"], "true", "{observation}");
    assert_eq!(observation["physical_quiescent"], true, "{observation}");
    assert_eq!(observation["production_reserve"]["status"], "work_required");
    assert_eq!(observation["production_reserve"]["epistemic"], "predicted");
    assert_eq!(
        observation["production_reserve"]["planner"],
        "consumption_reserve_v1"
    );
    assert_eq!(
        observation["production_reserve"]["reserve_until_tick"],
        2401
    );
    assert_eq!(observation["needs_replan"], true, "{observation}");
    assert!(
        !observation["continuation_request"].is_null(),
        "{observation}"
    );
    Ok((root, observation))
}

fn assert_review(session: &str, review: &Value, parent: &str, root: &str) -> TestResult {
    assert_eq!(review["ok"], true, "{review}");
    assert_eq!(review["schema"], "dfmcp.production-reserve/1");
    assert_eq!(review["kind"], "production_reserve_continuation");
    assert_lineage(&review["production"]["continuation"], parent, root);
    let lineage = &review["production"]["continuation"];
    assert_eq!(lineage["source_schema"], "dfmcp.production-continuation/6");
    assert_eq!(lineage["planner"], "consumption_reserve_v1");
    assert_eq!(lineage["reserve_until_tick"], 2401);
    assert_eq!(lineage["original_absolute_horizon_preserved"], true);
    assert_eq!(lineage["requires_original_stock_ledger"], true);
    assert_eq!(
        review["production"]["consumption"]["reserve_until_tick"],
        2401
    );
    assert_eq!(
        review["production"]["consumption"]["continuous_maintenance_proven"],
        false
    );
    let (plan, source) = pending(session)?;
    assert_eq!(
        json!(crate::sealed_plan_review::expanded_steps(review)?),
        crate::lab_world::plan_steps_json(&plan)
    );
    let dfmcp_lab::durable::DurablePlanSource::ProductionContinuation { raw, .. } = source else {
        return Err("ordinary reserve continuation source missing".into());
    };
    assert_eq!(parsed(&raw)?["schema"], "dfmcp.production-continuation/6");
    assert!(parsed(&raw)?["production"]["acquire_resources"].is_null());
    Ok(())
}

#[test]
fn ordinary_current_true_reserve_continues_within_default_budget_and_funds_original_tick()
-> TestResult {
    let _serial = serialized_durable_tests();
    let session = open("7861101", false, false)?;
    in_session(&session, |guard| {
        guard.budget.max_output_tokens = 4096;
        Ok(())
    })?;
    let (root, original_history) = canceled_reserve(&session)?;
    let before = effects_before(&session)?;
    let review = continuation(&session, &root, "finish original reserve")?;
    assert_review(&session, &review, &root, &root)?;
    assert!(
        review.to_string().len() <= 4096 * 4,
        "{} bytes",
        review.to_string().len()
    );
    assert_eq!(effects_before(&session)?, before);
    let child = identity(&review, "plan_digest")?;
    commit_ok(&session, &review)?;
    wait(&session, 300)?;
    let funded = goal(&session, &root)?;
    assert_eq!(funded["predicate_truth"], "true");
    assert_eq!(
        funded["production_reserve"]["status"],
        "funded_from_current_stock"
    );
    assert_eq!(
        funded["production_reserve"]["continuous_maintenance_proven"],
        false
    );
    assert_eq!(funded["needs_replan"], false);
    assert!(funded["continuation_request"].is_null());
    assert_history_unchanged(&original_history, &funded);
    assert_eq!(goal(&session, &child)?["predicate_truth"], "true");
    let before = effects_before(&session)?;
    let duplicate = continuation(&session, &root, "already funded")?;
    assert_eq!(duplicate["error"]["code"], "invalid_intent", "{duplicate}");
    assert_eq!(effects_before(&session)?, before);
    let remaining = in_session(&session, |guard| Ok(2401 - guard.adapter.snapshot().tick.0))?;
    wait(&session, remaining)?;
    let expired = goal(&session, &root)?;
    assert_eq!(expired["predicate_truth"], "true");
    assert_eq!(expired["production_reserve"]["status"], "expired");
    assert_eq!(expired["production_reserve"]["new_request_required"], true);
    assert_history_unchanged(&original_history, &expired);
    Ok(())
}

#[test]
fn ordinary_reserve_current_authority_source_and_output_refusals_preserve_review() -> TestResult {
    let _serial = serialized_durable_tests();
    let session = open("7861102", false, false)?;
    let (root, _) = canceled_reserve(&session)?;
    let review = continuation(&session, &root, "retained reviewed reserve")?;
    assert_review(&session, &review, &root, &root)?;
    let retained = pending(&session)?;
    for capability in [Capability::Observe, Capability::Query, Capability::Plan] {
        in_session(&session, |guard| {
            for grant in &mut guard.grants {
                if grant.capability == capability {
                    grant.expires_at_tick = Some(GameTick(0));
                }
            }
            Ok(())
        })?;
        let before = effects_before(&session)?;
        for denied in [
            continuation(&session, &root, "missing authority")?,
            commit(&session, &review)?,
        ] {
            assert_eq!(denied["error"]["code"], "capability_denied", "{denied}");
        }
        assert_eq!(pending(&session)?, retained);
        assert_eq!(effects_before(&session)?, before);
        in_session(&session, |guard| {
            for grant in &mut guard.grants {
                if grant.capability == capability {
                    grant.expires_at_tick = None;
                }
            }
            Ok(())
        })?;
    }
    let budget = in_session(&session, |guard| Ok(guard.budget))?;
    for (entities, tokens) in [(1, budget.max_output_tokens), (budget.max_entities, 600)] {
        in_session(&session, |guard| {
            guard.budget.max_entities = entities;
            guard.budget.max_output_tokens = tokens;
            Ok(())
        })?;
        let before = effects_before(&session)?;
        let refused = continuation(&session, &root, "budget cannot replace pending")?;
        assert_eq!(refused["error"]["code"], "budget_exceeded", "{refused}");
        assert_eq!(pending(&session)?, retained);
        assert_eq!(effects_before(&session)?, before);
        in_session(&session, |guard| {
            guard.budget = budget;
            Ok(())
        })?;
    }
    wait(&session, 1)?;
    let before = effects_before(&session)?;
    let stale = commit(&session, &review)?;
    assert_eq!(stale["error"]["code"], "stale_anchor", "{stale}");
    assert_eq!(
        stale["rebase"]["method"],
        "fresh_production_reserve_required"
    );
    assert!(stale["rebased_plan"].is_null());
    assert_eq!(pending(&session)?, retained);
    assert_eq!(effects_before(&session)?, before);
    let fresh = continuation(&session, &root, "review remaining original reserve")?;
    assert_review(&session, &fresh, &root, &root)?;
    commit_ok(&session, &fresh)?;
    wait(&session, 300)?;
    assert_eq!(
        goal(&session, &root)?["production_reserve"]["status"],
        "funded_from_current_stock"
    );
    Ok(())
}

#[test]
fn ordinary_reserve_same_root_siblings_block_active_work_but_not_remaining_reserve() -> TestResult {
    let _serial = serialized_durable_tests();
    let first = open("7861103", true, false)?;
    let (root, history) = canceled_reserve(&first)?;
    let second = open("7861103", true, false)?;
    let accepted = continuation(&first, &root, "first reserve review")?;
    let competing = continuation(&second, &root, "second reserve review")?;
    assert_review(&first, &accepted, &root, &root)?;
    assert_review(&second, &competing, &root, &root)?;
    let child = identity(&accepted, "plan_digest")?;
    commit_ok(&first, &accepted)?;
    let before = effects_before(&second)?;
    for denied in [
        commit(&second, &competing)?,
        continuation(&second, &root, "active sibling")?,
    ] {
        assert_eq!(denied["error"]["code"], "preconditions_failed", "{denied}");
    }
    assert_eq!(effects_before(&second)?, before);
    stop(&first)?;
    // Both historical current stock predicates are True; neither proves the
    // unchanged future tick funded after these production orders were stopped.
    let resumed = continuation(&second, &child, "finish child reserve")?;
    assert_review(&second, &resumed, &child, &root)?;
    commit_ok(&second, &resumed)?;
    wait(&second, 300)?;
    assert_eq!(
        goal(&second, &root)?["production_reserve"]["status"],
        "funded_from_current_stock"
    );
    assert_history_unchanged(&history, &goal(&second, &root)?);
    Ok(())
}

#[test]
fn ordinary_reserve_survives_two_durable_reopens_with_same_ledger_horizon_and_history() -> TestResult
{
    let _serial = serialized_durable_tests();
    let directory = StateDir::new("current-reserve");
    let first = open("7861104", false, true)?;
    let (root, original_history) = canceled_reserve(&first)?;
    with_durable_store(|store| store.compact())?;
    simulate_durable_restart(Some(directory.0.clone()));
    let second = open("7861104", false, true)?;
    let recovered = goal(&second, &root)?;
    assert_history_unchanged(&original_history, &recovered);
    assert_eq!(recovered["production_reserve"]["status"], "work_required");
    assert_ne!(
        recovered["sealed_anchor"]["epoch"],
        recovered["observed_anchor"]["epoch"]
    );
    let review = continuation(&second, &root, "reserve after first recovery")?;
    assert_review(&second, &review, &root, &root)?;
    let child = identity(&review, "plan_digest")?;
    commit_ok(&second, &review)?;
    wait(&second, 50)?;
    stop(&second)?;
    wait(&second, 1)?;
    let partial = goal(&second, &child)?;
    assert_eq!(partial["predicate_truth"], "true");
    assert_eq!(partial["production_reserve"]["status"], "work_required");
    with_durable_store(|store| store.compact())?;
    simulate_durable_restart(Some(directory.0.clone()));
    let third = open("7861104", false, true)?;
    assert_history_unchanged(&partial, &goal(&third, &child)?);
    assert_history_unchanged(&original_history, &goal(&third, &root)?);
    let resumed = continuation(&third, &child, "same reserve after second recovery")?;
    assert_review(&third, &resumed, &child, &root)?;
    commit_ok(&third, &resumed)?;
    wait(&third, 300)?;
    let funded = goal(&third, &root)?;
    assert_eq!(
        funded["production_reserve"]["status"],
        "funded_from_current_stock"
    );
    assert_history_unchanged(&original_history, &funded);
    assert_history_unchanged(&partial, &goal(&third, &child)?);
    Ok(())
}

#[test]
fn historical_reserve_candidate_cannot_import_current_true_eligibility() -> TestResult {
    let _serial = serialized_durable_tests();
    let session = open("7861105", false, false)?;
    let (root, _) = canceled_reserve(&session)?;
    let review = continuation(&session, &root, "new generation review")?;
    assert_review(&session, &review, &root, &root)?;
    let before = effects_before(&session)?;
    in_session(&session, |guard| {
        let pending = guard
            .pending
            .as_ref()
            .ok_or_else(|| fixture_error("pending missing"))?;
        let PlanSource::ProductionContinuation { summary, raw } = &pending.source else {
            return Err(fixture_error("continuation source missing"));
        };
        let mut old: Value =
            serde_json::from_str(raw).map_err(|error| fixture_error(&error.to_string()))?;
        old["schema"] = json!("dfmcp.production-continuation/3");
        let original = goal_continuation::ProductionContinuation::parse(&old.to_string())?;
        let legacy = PlanSource::ProductionContinuation {
            summary: summary.clone(),
            raw: original.canonical_json(),
        };
        let error = objectives::validate_continuation(guard, &legacy, None)
            .err()
            .ok_or_else(|| fixture_error("historical /3 imported current-True eligibility"))?;
        assert_eq!(error.code, ErrorCode::InvalidIntent);
        Ok(())
    })?;
    assert_eq!(effects_before(&session)?, before);
    commit_ok(&session, &review)?;
    Ok(())
}

#[test]
fn ordinary_reserve_replacement_ledger_is_unknown_and_cannot_continue() -> TestResult {
    let _serial = serialized_durable_tests();
    let session = open("7861106", false, false)?;
    let (root, history) = canceled_reserve(&session)?;
    let review = continuation(&session, &root, "retain the qualified ledger")?;
    assert_review(&session, &review, &root, &root)?;
    let retained = pending(&session)?;
    in_session(&session, |guard| {
        let mut changed = guard.adapter.snapshot().clone();
        changed
            .graph
            .entities
            .get_mut(&starter::STOCK_LEDGER)
            .ok_or_else(|| fixture_error("ledger fixture missing"))?
            .generation += 1;
        changed.cursor = changed
            .cursor
            .checked_next()
            .ok_or_else(|| fixture_error("cursor exhausted"))?;
        changed.refresh_hash();
        guard.adapter.inject_snapshot(changed)?;
        Ok(())
    })?;
    let observation = goal(&session, &root)?;
    assert_eq!(observation["predicate_truth"], "unknown");
    assert_eq!(observation["production_reserve"]["status"], "unknown");
    assert_eq!(observation["needs_replan"], false);
    assert_history_unchanged(&history, &observation);
    let before = effects_before(&session)?;
    for denied in [
        continuation(&session, &root, "cannot rebind ledger")?,
        commit(&session, &review)?,
    ] {
        assert_eq!(denied["error"]["code"], "preconditions_failed", "{denied}");
    }
    assert_eq!(effects_before(&session)?, before);
    assert_eq!(pending(&session)?, retained);
    Ok(())
}

#[test]
fn current_true_unfunded_reserve_retains_custody_at_goal_capacity() -> TestResult {
    let _serial = serialized_durable_tests();
    let session = open("7861107", false, false)?;
    let (root, history) = canceled_reserve(&session)?;
    assert_eq!(history["historical_achievement"], "verified");
    // Fill the book with distinct, actually admitted and canceled reserve
    // pursuits. Internal handlers isolate retention from facade packet size.
    for index in 1..dfmcp_lab::durable::MAX_OBJECTIVES_PER_FORTRESS {
        let proposal = parsed(&plan_request(
            Some(session.clone()),
            Some(format!("retain unfinished explicit reserve {index}")),
            None,
            None,
            Some(
                json!({
                    "template":"production", "reserve_until_tick":2401,
                    "quotas":[{"item":"DRINK","minimum":40},{"item":"FOOD","minimum":60}],
                })
                .to_string(),
            ),
            None,
        ))?;
        assert_eq!(proposal["ok"], true, "{proposal}");
        let committed = parsed(&fortress_commit(
            Some(session.clone()),
            identity(&proposal, "plan_digest")?,
        ))?;
        assert_eq!(committed["ok"], true, "{committed}");
        let stopped = parsed(&cancel_in_scope(
            Some(session.clone()),
            Some("stop_future_steps".to_owned()),
            Some("plan".to_owned()),
        ))?;
        assert_eq!(stopped["ok"], true, "{stopped}");
    }
    let candidate = parsed(&plan_request(
        Some(session.clone()),
        Some("retain the original future reserve".to_owned()),
        Some(true),
        None,
        None,
        None,
    ))?;
    assert_eq!(candidate["ok"], true, "{candidate}");
    let retained = pending(&session)?;
    let before = effects_before(&session)?;
    let refused = parsed(&fortress_commit(
        Some(session.clone()),
        identity(&candidate, "plan_digest")?,
    ))?;
    assert_eq!(refused["error"]["code"], "budget_exceeded", "{refused}");
    assert_eq!(pending(&session)?, retained);
    assert_eq!(effects_before(&session)?, before);
    let still_required = goal(&session, &root)?;
    assert_eq!(still_required["predicate_truth"], "true");
    assert_eq!(
        still_required["production_reserve"]["status"],
        "work_required"
    );
    assert_history_unchanged(&history, &still_required);
    Ok(())
}

#[test]
fn restore_abandons_an_unfinished_current_true_reserve_across_reopen() -> TestResult {
    let _serial = serialized_durable_tests();
    let directory = StateDir::new("reserve-abandoned");
    let session = open("7861108", false, true)?;
    let checkpoint = parsed(&fortress_checkpoint(
        Some(session.clone()),
        Some("before the original future reserve".to_owned()),
    ))?;
    assert_eq!(checkpoint["ok"], true, "{checkpoint}");
    let (root, history) = canceled_reserve(&session)?;
    let review = continuation(&session, &root, "review before restore")?;
    assert_review(&session, &review, &root, &root)?;
    let restored = parsed(&fortress_restore(
        Some(session.clone()),
        identity(&checkpoint, "checkpoint_id")?,
    ))?;
    assert_eq!(restored["ok"], true, "{restored}");
    simulate_durable_restart(Some(directory.0.clone()));
    let reopened = open("7861108", false, true)?;
    let abandoned = goal(&reopened, &root)?;
    assert_eq!(abandoned["status"], "abandoned");
    assert_eq!(abandoned["predicate_truth"], "unknown");
    assert_eq!(
        abandoned["first_satisfied_anchor"],
        history["first_satisfied_anchor"]
    );
    assert_eq!(abandoned["original_source"], history["original_source"]);
    assert_eq!(abandoned["sealed_anchor"], history["sealed_anchor"]);
    assert!(!abandoned["restore_abandoned_anchor"].is_null());
    assert!(abandoned["continuation_request"].is_null());
    let before = effects_before(&reopened)?;
    let refused = continuation(&reopened, &root, "restore cannot renew a reserve")?;
    assert_eq!(
        refused["error"]["code"], "preconditions_failed",
        "{refused}"
    );
    assert_eq!(effects_before(&reopened)?, before);
    Ok(())
}
