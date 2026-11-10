//! Finite inputs and an absolute reserve through the actual laboratory facade.

use super::*;

fn reserve_request(quotas: Json, until: u64) -> Json {
    let mut source = request(quotas);
    source["reserve_until_tick"] = json!(until);
    source
}

fn reserve_beds(minimum: u32, until: u64) -> Json {
    reserve_request(json!([{"item":"BED","minimum":minimum}]), until)
}

fn assert_reserve_stage(proposal: &Json, stage: &str, child: bool, until: u64) {
    assert_eq!(proposal["ok"], true, "{proposal}");
    assert_eq!(proposal["schema"], "dfmcp.resource-production/1");
    assert_eq!(
        proposal["kind"],
        if child {
            "resource_production_continuation"
        } else {
            "resource_production"
        }
    );
    let workflow = &proposal["production"]["resource_workflow"];
    assert_eq!(workflow["schema"], "dfmcp.production-resource-workflow/2");
    assert_eq!(workflow["stage"], stage, "{proposal}");
    assert_eq!(workflow["planner"], "resource_reserve_v1");
    assert_eq!(
        workflow["original_request"]["planner"],
        "resource_reserve_v1"
    );
    assert_eq!(workflow["original_request"]["reserve_until_tick"], until);
    assert_eq!(
        workflow["original_request"]["acquire_resources"]["assign_labor"],
        false
    );
    assert_eq!(workflow["automatic_continuation"], false);
    let consumption = &proposal["production"]["consumption"];
    assert_eq!(consumption["reserve_until_tick"], until);
    assert_eq!(consumption["requested_tick_is_action_deadline"], false);
    assert_eq!(consumption["continuous_maintenance_proven"], false);
    complete_reference(proposal);
}

fn assert_reserve_lineage(proposal: &Json, parent: &str, root: &str, until: u64) {
    let lineage = &proposal["production"]["continuation"];
    assert_eq!(lineage["parent_plan_digest"], parent);
    assert_eq!(lineage["root_plan_digest"], root);
    assert_eq!(lineage["source_schema"], "dfmcp.production-continuation/5");
    assert_eq!(lineage["planner"], "resource_reserve_v1");
    assert_eq!(lineage["reserve_until_tick"], until);
    assert_eq!(lineage["original_absolute_horizon_preserved"], true);
    assert_eq!(lineage["original_resource_policy_preserved"], true);
    assert_eq!(lineage["requires_observed_inputs_for_production"], true);
    assert_eq!(lineage["requires_explicit_commit"], true);
    assert_eq!(lineage["next_stage_dispatched"], false);
}

fn current_tick(session: &str) -> TestResult<u64> {
    owned(session, |session| Ok(session.adapter.snapshot().tick.0))
}

fn finish_at(session: &str, until: u64) -> TestResult<Json> {
    let remaining = until
        .checked_sub(current_tick(session)?)
        .ok_or("reserve tick was passed")?;
    wait(session, remaining)
}

fn stop_plan(session: &str) -> TestResult<Json> {
    let result = parsed(&facade::fortress_cancel(
        Some(session.to_owned()),
        Some("stop_future_steps".to_owned()),
        Some("plan".to_owned()),
    ))?;
    assert_eq!(result["ok"], true, "{result}");
    Ok(result)
}

#[test]
fn public_resource_reserve_observes_plant_yield_then_produces_for_the_same_future_tick()
-> TestResult {
    let _serial = serialized_durable_tests();
    let session = open("9643101", false, false, Some(16_384))?;
    wait(&session, 1_100)?;
    let quotas = json!([{"item":"DRINK","minimum":140},{"item":"FOOD","minimum":65}]);
    let before = effects_before(&session)?;
    let acquisition = plan(&session, reserve_request(quotas, 4_801), None)?;
    assert_reserve_stage(&acquisition, "acquire_resources", false, 4_801);
    assert_eq!(effects_before(&session)?, before);
    only_harvest(&session, HarvestResource::Plant, 10)?;
    let original_plan = prepared(&session)?;
    let root = text(&acquisition, "plan_digest")?;
    commit_ok(&session, &acquisition)?;
    let active = continuation(&session, &root, None)?;
    assert_eq!(active["error"]["code"], "preconditions_failed", "{active}");
    wait(&session, 500)?;
    assert_eq!(stock(&session, effects::STOCK_PLANT_FIELD)?, 30);
    assert_eq!(stock(&session, effects::STOCK_DRINK_FIELD)?, 33);
    assert_eq!(stock(&session, effects::STOCK_FOOD_FIELD)?, 60);
    assert_eq!(goal(&session, &root)?["predicate_truth"], "false");

    let before = effects_before(&session)?;
    let production = continuation(&session, &root, None)?;
    assert_reserve_stage(&production, "production", true, 4_801);
    assert_reserve_lineage(&production, &root, &root, 4_801);
    assert_eq!(effects_before(&session)?, before);
    assert_eq!(
        prepared(&session)?.terminal_condition,
        original_plan.terminal_condition
    );
    assert!(
        prepared(&session)?
            .steps
            .iter()
            .all(|step| matches!(step.action, Action::CreateWorkOrder { .. }))
    );
    assert_eq!(
        production["production"]["consumption"]["source_tick"],
        1_601
    );
    assert_eq!(
        production["forecast"]["reserve_forecast"]["predicted_tick"],
        4_801
    );
    assert_eq!(
        production["forecast"]["reserve_forecast"]["predicted_goal_truth"],
        "true"
    );
    let child = text(&production, "plan_digest")?;
    commit_ok(&session, &production)?;
    wait(&session, 1_500)?;
    assert_eq!(goal(&session, &child)?["predicate_truth"], "true");
    finish_at(&session, 4_801)?;
    assert!(stock(&session, effects::STOCK_DRINK_FIELD)? >= 140);
    assert!(stock(&session, effects::STOCK_FOOD_FIELD)? >= 65);
    assert_eq!(goal(&session, &root)?["predicate_truth"], "true");
    Ok(())
}

#[test]
fn current_true_resource_reserve_discovers_continuation_and_does_not_mistake_siblings_for_funding()
-> TestResult {
    let _serial = serialized_durable_tests();
    let session = open("9643102", false, false, Some(16_384))?;
    let quotas = json!([{"item":"DRINK","minimum":40},{"item":"FOOD","minimum":60}]);
    let acquisition = plan(&session, reserve_request(quotas, 12_001), None)?;
    assert_reserve_stage(&acquisition, "acquire_resources", false, 12_001);
    only_harvest(&session, HarvestResource::Plant, 1)?;
    let root = text(&acquisition, "plan_digest")?;
    commit_ok(&session, &acquisition)?;
    wait(&session, 100)?;
    assert_eq!(stock(&session, effects::STOCK_PLANT_FIELD)?, 21);
    assert_eq!(stock(&session, effects::STOCK_DRINK_FIELD)?, 40);
    assert_eq!(stock(&session, effects::STOCK_FOOD_FIELD)?, 60);
    let original = goal(&session, &root)?;
    assert_eq!(original["predicate_truth"], "true", "{original}");
    assert_eq!(original["semantic_output_goal"]["predicate_truth"], "true");
    assert_eq!(original["resource_reserve"]["status"], "work_required");
    assert_eq!(original["resource_reserve"]["epistemic"], "predicted");
    assert_eq!(original["needs_replan"], true);
    assert!(!original["continuation_request"].is_null(), "{original}");

    let production = continuation(&session, &root, None)?;
    assert_reserve_stage(&production, "production", true, 12_001);
    assert_reserve_lineage(&production, &root, &root, 12_001);
    assert_eq!(production["production"]["consumption"]["source_tick"], 101);
    let interrupted = text(&production, "plan_digest")?;
    commit_ok(&session, &production)?;
    stop_plan(&session)?;
    // The completed acquisition is a currently true sibling. Its current
    // quota proof does not establish the remaining future output reserve.
    let resumed = continuation(&session, &interrupted, None)?;
    assert_reserve_stage(&resumed, "production", true, 12_001);
    assert_reserve_lineage(&resumed, &interrupted, &root, 12_001);
    commit_ok(&session, &resumed)?;
    wait(&session, 1_500)?;
    let funded = goal(&session, &root)?;
    assert_eq!(funded["semantic_output_goal"]["predicate_truth"], "true");
    assert_eq!(
        funded["resource_reserve"]["status"],
        "funded_from_current_stock"
    );
    assert_eq!(
        funded["resource_reserve"]["continuous_maintenance_proven"],
        false
    );
    assert_eq!(funded["needs_replan"], false);
    assert!(funded["continuation_request"].is_null());
    let before = effects_before(&session)?;
    let satisfied = continuation(&session, &root, None)?;
    assert_eq!(satisfied["error"]["code"], "invalid_intent", "{satisfied}");
    assert_eq!(effects_before(&session)?, before);
    finish_at(&session, 12_001)?;
    assert!(stock(&session, effects::STOCK_DRINK_FIELD)? >= 40);
    assert!(stock(&session, effects::STOCK_FOOD_FIELD)? >= 60);
    Ok(())
}

#[test]
fn resource_reserve_default_budget_keeps_complete_review_and_refusals_preserve_pending()
-> TestResult {
    let _serial = serialized_durable_tests();
    let session = open("9643103", false, false, None)?;
    let acquisition = plan(&session, reserve_beds(22, 6_001), None)?;
    assert_reserve_stage(&acquisition, "acquire_resources", false, 6_001);
    assert!(acquisition.to_string().len() <= 4 * 4_096);
    let root = text(&acquisition, "plan_digest")?;
    commit_ok(&session, &acquisition)?;
    wait(&session, 100)?;
    let production = continuation(&session, &root, None)?;
    assert_reserve_stage(&production, "production", true, 6_001);
    assert_reserve_lineage(&production, &root, &root, 6_001);
    assert!(production.to_string().len() <= 4 * 4_096);
    let retained = pending(&session)?;
    let budget = owned(&session, |session| Ok(session.budget))?;
    for source_bound in [true, false] {
        owned(&session, |session| {
            if source_bound {
                session.budget.max_entities = 1;
            } else {
                session.budget.max_output_tokens = 600;
            }
            Ok(())
        })?;
        let before = effects_before(&session)?;
        let refused = continuation(&session, &root, None)?;
        assert_eq!(refused["error"]["code"], "budget_exceeded", "{refused}");
        assert_eq!(pending(&session)?, retained);
        assert_eq!(effects_before(&session)?, before);
        owned(&session, |session| {
            session.budget = budget;
            Ok(())
        })?;
    }
    for capability in [Capability::Observe, Capability::Query, Capability::Plan] {
        owned(&session, |session| {
            for grant in &mut session.grants {
                if grant.capability == capability {
                    grant.expires_at_tick = Some(GameTick(0));
                }
            }
            Ok(())
        })?;
        let before = effects_before(&session)?;
        for refused in [
            continuation(&session, &root, None)?,
            commit(&session, &production)?,
        ] {
            assert_eq!(refused["error"]["code"], "capability_denied", "{refused}");
        }
        assert_eq!(pending(&session)?, retained);
        assert_eq!(effects_before(&session)?, before);
        owned(&session, |session| {
            for grant in &mut session.grants {
                if grant.capability == capability {
                    grant.expires_at_tick = None;
                }
            }
            Ok(())
        })?;
    }
    commit_ok(&session, &production)?;
    wait(&session, 3_000)?;
    assert_eq!(stock(&session, effects::STOCK_BED_FIELD)?, 22);
    Ok(())
}

#[test]
fn canceled_resource_reserve_recovers_exact_policy_and_produces_after_residual_acquisition()
-> TestResult {
    let _serial = serialized_durable_tests();
    let directory = StateDir::new("reserve-reopen");
    let first = open("9643104", false, true, Some(16_384))?;
    let acquisition = plan(
        &first,
        reserve_beds(26, 6_001),
        Some("reserve twenty-six beds"),
    )?;
    assert_reserve_stage(&acquisition, "acquire_resources", false, 6_001);
    only_harvest(&first, HarvestResource::Wood, 6)?;
    let root = text(&acquisition, "plan_digest")?;
    commit_ok(&first, &acquisition)?;
    wait(&first, 100)?;
    stop_plan(&first)?;
    assert_eq!(stock(&first, effects::STOCK_WOOD_FIELD)?, 22);
    let history = goal(&first, &root)?;
    assert_eq!(history["predicate_truth"], "false");
    assert_eq!(history["physical_quiescent"], true);
    with_durable_store(|store| store.compact())?;
    simulate_durable_restart(Some(directory.0.clone()));
    let second = open("9643104", false, true, Some(16_384))?;
    unchanged_history(&history, &goal(&second, &root)?);
    let residual = continuation(&second, &root, None)?;
    assert_reserve_stage(&residual, "acquire_resources", true, 6_001);
    assert_reserve_lineage(&residual, &root, &root, 6_001);
    only_harvest(&second, HarvestResource::Wood, 4)?;
    let parent = text(&residual, "plan_digest")?;
    commit_ok(&second, &residual)?;
    wait(&second, 200)?;
    let production = continuation(&second, &parent, None)?;
    assert_reserve_stage(&production, "production", true, 6_001);
    assert_reserve_lineage(&production, &parent, &root, 6_001);
    let (source, expected, snapshot, context) = owned(&second, |session| {
        let pending = session
            .pending
            .as_ref()
            .ok_or_else(|| fixture("pending reserve missing"))?;
        Ok((
            pending.source.clone(),
            pending.plan.clone(),
            session.adapter.snapshot().clone(),
            read_context(session),
        ))
    })?;
    let restored = PlanSource::from_durable(&source.durable());
    let (intent, _) = restored.compile(expected.intent_id, &snapshot)?;
    assert_eq!(
        StaticPlanner::default().prepare_laboratory(&snapshot, &intent, &context)?,
        expected
    );
    let PlanSource::ProductionContinuation { raw, .. } = source else {
        return Err("private reserve continuation missing".into());
    };
    let archived = parsed(&raw)?;
    assert_eq!(archived["schema"], "dfmcp.production-continuation/5");
    assert_eq!(
        archived["production"],
        history["original_source"]["request"]
    );
    let before = effects_before(&second)?;
    let refused = plan(&second, archived, None)?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(effects_before(&second)?, before);
    assert_eq!(
        pending(&second)?.as_deref(),
        production["plan_digest"].as_str()
    );
    commit_ok(&second, &production)?;
    wait(&second, 3_000)?;
    assert_eq!(stock(&second, effects::STOCK_BED_FIELD)?, 26);
    let original = goal(&second, &root)?;
    assert_eq!(original["predicate_truth"], "false");
    assert_eq!(original["semantic_output_goal"]["predicate_truth"], "true");
    assert_eq!(
        original["resource_reserve"]["status"],
        "funded_from_current_stock"
    );
    assert_eq!(original["needs_replan"], false);
    unchanged_history(&history, &original);
    Ok(())
}

#[test]
fn stale_and_expired_resource_reserve_preserve_pending_and_never_extend_original_tick() -> TestResult
{
    let _serial = serialized_durable_tests();
    let session = open("9643105", false, false, Some(16_384))?;
    let acquisition = plan(&session, reserve_beds(22, 201), Some("same original date"))?;
    assert_reserve_stage(&acquisition, "acquire_resources", false, 201);
    wait(&session, 1)?;
    let before = effects_before(&session)?;
    let stale = commit(&session, &acquisition)?;
    assert_eq!(stale["error"]["code"], "stale_anchor", "{stale}");
    assert_eq!(
        stale["rebase"]["method"],
        "fresh_resource_production_required"
    );
    assert!(stale["rebased_plan"].is_null());
    assert_eq!(effects_before(&session)?, before);
    assert_eq!(
        pending(&session)?.as_deref(),
        acquisition["plan_digest"].as_str()
    );
    let source = parsed(&text(&stale["next_step"]["arguments"], "blueprint")?)?;
    assert_eq!(source["planner"], "resource_reserve_v1");
    assert_eq!(source["reserve_until_tick"], 201);
    let reviewed = plan(&session, source, Some("same original date"))?;
    assert_reserve_stage(&reviewed, "acquire_resources", false, 201);
    let root = text(&reviewed, "plan_digest")?;
    commit_ok(&session, &reviewed)?;
    wait(&session, 100)?;
    let production = continuation(&session, &root, None)?;
    assert_reserve_stage(&production, "production", true, 201);
    assert_reserve_lineage(&production, &root, &root, 201);
    let retained = pending(&session)?;
    finish_at(&session, 201)?;
    let expired = goal(&session, &root)?;
    assert_eq!(
        expired["resource_reserve"]["status"], "expired",
        "{expired}"
    );
    assert_eq!(expired["resource_reserve"]["new_request_required"], true);
    assert_eq!(expired["resource_reserve"]["remaining_horizon_ticks"], 0);
    assert_eq!(expired["needs_replan"], false);
    assert!(expired["continuation_request"].is_null());
    let before = effects_before(&session)?;
    for refused in [
        continuation(&session, &root, None)?,
        plan(&session, reserve_beds(22, 201), None)?,
    ] {
        assert_eq!(refused["ok"], false, "{refused}");
        assert_eq!(pending(&session)?, retained);
        assert_eq!(effects_before(&session)?, before);
    }
    Ok(())
}

#[test]
fn shared_current_true_resource_reserve_rechecks_same_root_work_before_commit() -> TestResult {
    let _serial = serialized_durable_tests();
    let first = open("9643106", true, false, Some(16_384))?;
    let acquisition = plan(
        &first,
        reserve_request(
            json!([{"item":"DRINK","minimum":40},{"item":"FOOD","minimum":60}]),
            12_001,
        ),
        None,
    )?;
    assert_reserve_stage(&acquisition, "acquire_resources", false, 12_001);
    let root = text(&acquisition, "plan_digest")?;
    commit_ok(&first, &acquisition)?;
    wait(&first, 100)?;
    let second = open("9643106", true, false, Some(16_384))?;
    let chosen = continuation(&first, &root, Some("first future output pursuit"))?;
    let peer = continuation(&second, &root, Some("peer future output pursuit"))?;
    assert_reserve_stage(&chosen, "production", true, 12_001);
    assert_reserve_stage(&peer, "production", true, 12_001);
    commit_ok(&first, &chosen)?;
    let before = effects_before(&second)?;
    for refused in [continuation(&second, &root, None)?, commit(&second, &peer)?] {
        assert_eq!(
            refused["error"]["code"], "preconditions_failed",
            "{refused}"
        );
        assert!(refused["rebased_plan"].is_null());
    }
    assert_eq!(pending(&second)?.as_deref(), peer["plan_digest"].as_str());
    assert_eq!(effects_before(&second)?, before);
    Ok(())
}

#[test]
fn current_true_resource_reserve_cannot_be_evicted_while_future_work_or_its_evidence_is_missing()
-> TestResult {
    let _serial = serialized_durable_tests();
    let session = open("9643107", false, false, Some(16_384))?;
    let acquisition = plan(
        &session,
        reserve_request(
            json!([{"item":"DRINK","minimum":40},{"item":"FOOD","minimum":60}]),
            12_001,
        ),
        None,
    )?;
    let root = text(&acquisition, "plan_digest")?;
    commit_ok(&session, &acquisition)?;
    wait(&session, 100)?;
    let proven = goal(&session, &root)?;
    assert_eq!(proven["predicate_truth"], "true", "{proven}");
    assert_eq!(proven["historical_achievement"], "verified");
    assert_eq!(proven["physical_quiescent"], true);
    assert_eq!(proven["resource_reserve"]["status"], "work_required");

    // Match the established objective-capacity fixtures: use the internal
    // request/commit boundary so 64 retained roots do not measure facade packet
    // compression. Every slot still comes from an actual admitted sealed goal.
    // The verified current-True reserve is the sole ordinary eviction candidate.
    for index in 1..dfmcp_lab::durable::MAX_OBJECTIVES_PER_FORTRESS {
        let proposal = parsed(&plan_request(
            Some(session.clone()),
            Some(format!("retain unfinished bed goal {index}")),
            None,
            None,
            Some(reserve_beds(26, 12_001).to_string()),
            None,
        ))?;
        assert_eq!(proposal["ok"], true, "{proposal}");
        only_harvest(&session, HarvestResource::Wood, 6)?;
        let committed = parsed(&fortress_commit(
            Some(session.clone()),
            text(&proposal, "plan_digest")?,
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
        Some("capacity must retain reserve".to_owned()),
        None,
        None,
        Some(reserve_beds(22, 12_001).to_string()),
        None,
    ))?;
    assert_eq!(candidate["ok"], true, "{candidate}");
    let retained = pending(&session)?;
    let before = effects_before(&session)?;
    let refused = parsed(&fortress_commit(
        Some(session.clone()),
        text(&candidate, "plan_digest")?,
    ))?;
    assert_eq!(refused["error"]["code"], "budget_exceeded", "{refused}");
    assert_eq!(pending(&session)?, retained);
    assert_eq!(effects_before(&session)?, before);

    owned(&session, |session| {
        let mut snapshot = session.adapter.snapshot().clone();
        let ledger =
            effects::stock_ledger(&snapshot).ok_or_else(|| fixture("stock ledger missing"))?;
        let record = snapshot
            .graph
            .entities
            .get_mut(&ledger)
            .ok_or_else(|| fixture("stock ledger missing"))?;
        record.fields.insert(
            effects::METABOLISM_TICKS_FIELD.to_owned(),
            Fact::known(
                WorldValue::U64(100),
                snapshot.tick,
                FactSource::AgentAssertion("unknown future consumption phase".to_owned()),
                Digest32::ZERO,
            ),
        );
        record.revision += 1;
        snapshot.cursor = snapshot
            .cursor
            .checked_next()
            .ok_or_else(|| fixture("cursor exhausted"))?;
        snapshot.refresh_hash();
        session.adapter.inject_snapshot(snapshot)?;
        let candidate = session
            .pending
            .as_ref()
            .ok_or_else(|| fixture("candidate missing"))?;
        assert_eq!(
            objectives::objective_evictions(session, candidate.plan.digest)
                .err()
                .map(|error| error.code),
            Some(ErrorCode::BudgetExceeded)
        );
        Ok(())
    })?;
    let unknown = goal(&session, &root)?;
    assert_eq!(unknown["semantic_output_goal"]["predicate_truth"], "true");
    assert_eq!(unknown["resource_reserve"]["status"], "unknown");
    assert_eq!(unknown["needs_replan"], false);
    assert!(unknown["continuation_request"].is_null());
    assert_eq!(pending(&session)?, retained);
    Ok(())
}
