//! Finite inputs must lead to an explicitly reviewed, complete room pursuit.

use super::*;
use dfmcp_intent::HarvestResource;

fn open_resources(selector: &str, shared: bool, durable: bool) -> TestResult<String> {
    open_resources_with_budget(selector, shared, durable, 16_384)
}

fn open_resources_with_budget(
    selector: &str,
    shared: bool,
    durable: bool,
    max_output_tokens: u32,
) -> TestResult<String> {
    let opened = parsed(&facade::fortress_open_session(
        Some(false),
        Some(selector.to_owned()),
        Some(
            [
                ("observe", "read_only"),
                ("query", "read_only"),
                ("plan", "read_only"),
                ("control_clock", "reversible"),
                ("configure_production", "reversible"),
                ("configure_labor", "reversible"),
                ("designate", "guarded"),
                ("construct", "guarded"),
                ("checkpoint", "guarded"),
                ("restore", "guarded"),
            ]
            .into_iter()
            .map(|(capability, risk)| (capability.to_owned(), risk.to_owned()))
            .collect(),
        ),
        Some(60_000),
        Some(12_000),
        Some(8_192),
        Some(4 * 1024 * 1024),
        Some(max_output_tokens),
        Some(64),
        Some("starter_fortress_resources".to_owned()),
        Some(shared),
        Some(durable),
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    identity(&opened, "session_id")
}

fn original_room() -> Json {
    json!({
        "template":"provisioned_room",
        "layout":{"template":"bedroom_cluster", "origin":[2,6,10], "rooms":2, "room_size":[3,3]},
        "acquire_resources":{"assign_labor":false},
        "prerequisites":{"assign_labor":false,"workshops":[{
            "building":"workshop:Carpenters", "location":[0,0,10],
        }]},
    })
}

fn original_connected_room() -> Json {
    json!({
        "template":"connected_room", "room":original_room(),
        "access":{"entry":[0,2,10], "region":{"min":[0,0,10],"max":[12,12,10]}},
    })
}

/// This source fixture begins with no usable wood and no Carpenter workshop.
/// Actual harvest, manufacture, construction and placement use the real adapter.
fn require_finite_inputs(session: &str) -> TestResult<()> {
    require_finite_stock(session, true)
}

fn require_finite_stock(session: &str, remove_workshop: bool) -> TestResult<()> {
    in_session(session, |guard| {
        let mut snapshot = guard.adapter.snapshot().clone();
        let ledger = effects::stock_ledger(&snapshot)
            .ok_or_else(|| fixture_error("room stock ledger missing"))?;
        let record = snapshot
            .graph
            .entities
            .get_mut(&ledger)
            .ok_or_else(|| fixture_error("room stock ledger missing"))?;
        record
            .fields
            .get_mut(effects::STOCK_WOOD_FIELD)
            .ok_or_else(|| fixture_error("qualified wood fact missing"))?
            .value = WorldValue::U64(0);
        record.revision += 1;
        if remove_workshop {
            snapshot
                .graph
                .entities
                .remove(&crate::lab_world::starter::CARPENTERS);
        }
        snapshot.cursor = snapshot
            .cursor
            .checked_next()
            .ok_or_else(|| fixture_error("fixture cursor exhausted"))?;
        snapshot.refresh_hash();
        guard.adapter.inject_snapshot(snapshot)?;
        if guard.durable_scenario.is_some() {
            persist_durable_head(guard);
            durability_gate(guard)?;
        }
        Ok(())
    })
}

fn wood(session: &str) -> TestResult<u64> {
    in_session(session, |guard| {
        let snapshot = guard.adapter.snapshot();
        let ledger = effects::stock_ledger(snapshot)
            .and_then(|id| snapshot.graph.entities.get(&id))
            .ok_or_else(|| fixture_error("room stock ledger missing"))?;
        match ledger
            .fields
            .get(effects::STOCK_WOOD_FIELD)
            .and_then(|fact| laboratory_fact_value(fact, snapshot.tick))
        {
            Some(WorldValue::U64(stock)) => Ok(*stock),
            _ => Err(fixture_error("room wood stock is not qualified")),
        }
    })
}

fn plan_room(session: &str, request: &Json) -> TestResult<Json> {
    parsed(&facade::fortress_plan(
        Some(session.to_owned()),
        None,
        None,
        None,
        Some(request.to_string()),
    ))
}

fn pending_digest(session: &str) -> TestResult<Option<String>> {
    in_session(session, |guard| {
        Ok(guard.pending.as_ref().map(|pending| pending.digest.clone()))
    })
}

fn pending_plan(session: &str) -> TestResult<PreparedPlan> {
    in_session(session, |guard| {
        guard
            .pending
            .as_ref()
            .map(|pending| pending.plan.clone())
            .ok_or_else(|| fixture_error("resource room review missing"))
    })
}

fn assert_stage(proposal: &Json, stage: &str, continuation: bool) {
    assert_eq!(proposal["ok"], true, "{proposal}");
    assert_eq!(proposal["schema"], "dfmcp.resource-room/2");
    assert_eq!(proposal["version"], 2);
    assert!(
        crate::sealed_plan_review::validate(proposal).is_ok(),
        "{proposal}"
    );
    assert_eq!(
        proposal["kind"],
        if continuation {
            "resource_room_continuation"
        } else {
            "resource_room"
        }
    );
    assert_eq!(proposal["room"]["resource_workflow"]["stage"], stage);
    let goal_scope = if stage == "acquire_resources" {
        "resource_acquisition_stage"
    } else {
        "reviewed_room_pursuit"
    };
    assert_eq!(proposal["goal_scope"], goal_scope);
    assert_eq!(proposal["room"]["goal_scope"], goal_scope);
    assert!(proposal["room"]["resource_workflow"]["original_request"].is_object());
    let review = &proposal["agent_turn"]["active_work"]["pending_plans"][0];
    let reference = &review["steps_reference"];
    assert_eq!(reference["scope"], "same_response");
    assert_eq!(reference["json_pointer"], "/steps");
    assert_eq!(reference["complete"], true);
    assert_eq!(
        reference["predicate_encoding_pointer"],
        "/predicate_encoding"
    );
    assert_eq!(reference["plan_digest"], proposal["plan_digest"]);
    assert_eq!(
        review["step_count"].as_u64(),
        proposal["steps"].as_array().map(|steps| steps.len() as u64)
    );
}

fn only_wood_harvest(session: &str, units: u32) -> TestResult<()> {
    let plan = pending_plan(session)?;
    assert_eq!(plan.steps.len(), 1, "{plan:?}");
    assert!(matches!(
        plan.steps[0].action,
        Action::HarvestResource { resource: HarvestResource::Wood, amount, .. }
            if amount == units
    ));
    Ok(())
}

fn finish_stage(session: &str, digest: &str) -> TestResult<Json> {
    for _ in 0..256 {
        let observed = goal(session, digest)?;
        if observed["physical_quiescent"] == true {
            return Ok(observed);
        }
        wait(session, 100)?;
    }
    Err("resource room stage did not become physically quiet within 25,600 ticks".into())
}

fn complete_acquisition(session: &str, request: &Json) -> TestResult<(String, Json)> {
    complete_acquisition_units(session, request, 3)
}

fn complete_acquisition_units(
    session: &str,
    request: &Json,
    units: u32,
) -> TestResult<(String, Json)> {
    let before = effects_before(session)?;
    let acquisition = plan_room(session, request)?;
    assert_stage(&acquisition, "acquire_resources", false);
    only_wood_harvest(session, units)?;
    assert_eq!(effects_before(session)?, before);
    assert_eq!(wood(session)?, 0);
    let digest = identity(&acquisition, "plan_digest")?;
    commit_ok(session, &acquisition)?;
    let before = effects_before(session)?;
    let active = continuation(session, &digest)?;
    assert_eq!(active["error"]["code"], "preconditions_failed", "{active}");
    assert_eq!(effects_before(session)?, before);
    let finished = finish_stage(session, &digest)?;
    assert_eq!(finished["predicate_truth"], "true", "{finished}");
    assert_eq!(finished["semantic_room_goal"]["truth"], "false");
    assert_eq!(finished["goal_scope"], "resource_acquisition_stage");
    assert_eq!(
        finished["historical_achievement_scope"],
        "resource_acquisition_stage"
    );
    assert_eq!(finished["needs_replan"], true);
    assert!(finished["continuation_request"].is_object());
    assert_eq!(wood(session)?, u64::from(units));
    Ok((digest, finished))
}

#[test]
fn resource_room_observes_finite_wood_before_manufacturing_building_and_furnishing() -> TestResult {
    let _serial = serialized_durable_tests();
    let session = open_resources("9654101", false, false)?;
    require_finite_inputs(&session)?;
    let request = original_room();
    let (root, harvest_history) = complete_acquisition(&session, &request)?;
    let before = effects_before(&session)?;
    let room = continuation(&session, &root)?;
    assert_stage(&room, "room", true);
    assert_eq!(effects_before(&session)?, before);
    let original = &room["room"]["resource_workflow"]["original_request"];
    assert_eq!(original["planner"], "provisioned_room_resources_v1");
    assert_eq!(original["layout"], request["layout"]);
    assert_eq!(original["acquire_resources"], request["acquire_resources"]);
    assert_eq!(original["prerequisites"]["assign_labor"], false);
    let sealed = pending_plan(&session)?;
    assert!(
        sealed
            .steps
            .iter()
            .any(|step| matches!(step.action, Action::Build { .. }))
    );
    assert!(
        sealed
            .steps
            .iter()
            .any(|step| matches!(step.action, Action::CreateWorkOrder { .. }))
    );
    assert_eq!(
        sealed
            .steps
            .iter()
            .filter(|step| matches!(step.action, Action::PlaceFurniture { .. }))
            .count(),
        2
    );
    assert!(
        !sealed
            .steps
            .iter()
            .any(|step| matches!(step.action, Action::HarvestResource { .. }))
    );
    let child = identity(&room, "plan_digest")?;
    commit_ok(&session, &room)?;
    let completed = finish_stage(&session, &child)?;
    assert_eq!(completed["predicate_truth"], "true", "{completed}");
    assert_eq!(completed["semantic_room_goal"]["truth"], "true");
    assert_eq!(completed["goal_scope"], "reviewed_room_pursuit");
    in_session(&session, |guard| {
        for step in &sealed.steps {
            if let Action::PlaceFurniture {
                kind,
                item,
                location,
            } = step.action
            {
                effects::validate_furniture_completion(
                    guard.adapter.snapshot(),
                    effects::created_entity_id(&step.idempotency_key, 0),
                    kind,
                    item,
                    location,
                )?;
            }
        }
        Ok(())
    })?;
    let original_now = goal(&session, &root)?;
    assert_history_unchanged(&harvest_history, &original_now);
    assert_eq!(original_now["semantic_room_goal"]["truth"], "true");
    assert_eq!(original_now["needs_replan"], false);
    let before = effects_before(&session)?;
    let duplicate = continuation(&session, &root)?;
    assert_eq!(duplicate["error"]["code"], "invalid_intent", "{duplicate}");
    assert_eq!(effects_before(&session)?, before);
    Ok(())
}

#[test]
fn resource_connected_room_preserves_original_entrance_and_complete_access_goal() -> TestResult {
    let _serial = serialized_durable_tests();
    let session = open_resources("9654102", false, false)?;
    require_finite_inputs(&session)?;
    let request = original_connected_room();
    let (root, _) = complete_acquisition(&session, &request)?;
    let room = continuation(&session, &root)?;
    assert_stage(&room, "room", true);
    let original = &room["room"]["resource_workflow"]["original_request"];
    assert_eq!(original["planner"], "connected_room_resources_v1");
    assert_eq!(original["access"], request["access"]);
    assert_eq!(original["room"]["layout"], request["room"]["layout"]);
    assert_eq!(
        original["room"]["acquire_resources"],
        request["room"]["acquire_resources"]
    );
    let child = identity(&room, "plan_digest")?;
    commit_ok(&session, &room)?;
    let completed = finish_stage(&session, &child)?;
    assert_eq!(completed["predicate_truth"], "true", "{completed}");
    assert_eq!(completed["semantic_room_goal"]["truth"], "true");
    assert_eq!(
        goal(&session, &root)?["semantic_room_goal"]["truth"],
        "true"
    );
    Ok(())
}

#[test]
fn two_bedroom_resource_projects_with_existing_workshops_fit_8192_token_reviews() -> TestResult {
    let _serial = serialized_durable_tests();
    for connected in [false, true] {
        let session = open_resources_with_budget(
            if connected { "9654111" } else { "9654110" },
            false,
            false,
            8_192,
        )?;
        require_finite_stock(&session, false)?;
        let mut request = original_room();
        request
            .as_object_mut()
            .ok_or("room request is not an object")?
            .remove("prerequisites");
        if connected {
            request = json!({
                "template":"connected_room", "room":request,
                "access":{"entry":[0,2,10], "region":{"min":[0,0,10],"max":[12,12,10]}},
            });
        }
        let (root, harvest_history) = complete_acquisition_units(&session, &request, 2)?;
        assert_eq!(harvest_history["semantic_room_goal"]["truth"], "false");
        let before = effects_before(&session)?;
        let reviewed = continuation(&session, &root)?;
        assert_stage(&reviewed, "room", true);
        assert_eq!(effects_before(&session)?, before);
        assert!(reviewed.to_string().len() <= 8_192 * 4, "{reviewed}");
        let sealed = pending_plan(&session)?;
        assert_eq!(
            json!(crate::sealed_plan_review::expanded_steps(&reviewed)?),
            crate::lab_world::plan_steps_json(&sealed),
        );
        assert_eq!(
            sealed
                .steps
                .iter()
                .filter(|step| matches!(step.action, Action::PlaceFurniture { .. }))
                .count(),
            2,
        );
        assert!(
            sealed
                .steps
                .iter()
                .any(|step| matches!(step.action, Action::CreateWorkOrder { .. }))
        );
        assert!(sealed.steps.iter().all(|step| !matches!(
            step.action,
            Action::Build { .. } | Action::HarvestResource { .. }
        )));
        assert_eq!(reviewed["room"]["continuation"]["parent_plan_digest"], root);
        assert_eq!(reviewed["room"]["continuation"]["root_plan_digest"], root);
        commit_ok(&session, &reviewed)?;
        let completed = finish_stage(&session, &identity(&reviewed, "plan_digest")?)?;
        assert_eq!(completed["predicate_truth"], "true", "{completed}");
        assert_eq!(completed["semantic_room_goal"]["truth"], "true");
        let original_now = goal(&session, &root)?;
        assert_history_unchanged(&harvest_history, &original_now);
        assert_eq!(original_now["semantic_room_goal"]["truth"], "true");
        assert_eq!(original_now["needs_replan"], false);
    }
    Ok(())
}

#[test]
fn resource_room_reviews_preserve_complete_proof_or_pending_under_default_budget() -> TestResult {
    let _serial = serialized_durable_tests();
    for (index, (connected, missing_workshop)) in
        [(false, false), (true, false), (false, true), (true, true)]
            .into_iter()
            .enumerate()
    {
        let session = open_resources(&format!("965412{index}"), false, false)?;
        require_finite_stock(&session, missing_workshop)?;
        let mut request = original_room();
        if !missing_workshop {
            request
                .as_object_mut()
                .ok_or("room request is not an object")?
                .remove("prerequisites");
        }
        if connected {
            request = json!({
                "template":"connected_room", "room":request,
                "access":{"entry":[0,2,10], "region":{"min":[0,0,10],"max":[12,12,10]}},
            });
        }
        let units = if missing_workshop { 3 } else { 2 };
        let (root, harvest_history) = complete_acquisition_units(&session, &request, units)?;
        assert_eq!(harvest_history["semantic_room_goal"]["truth"], "false");
        let prior = continuation(&session, &root)?;
        assert_stage(&prior, "room", true);
        let retained = pending_digest(&session)?;
        let budget = in_session(&session, |guard| Ok(guard.budget))?;
        in_session(&session, |guard| {
            guard.budget.max_output_tokens = 4_096;
            Ok(())
        })?;
        let before = effects_before(&session)?;
        let attempted = continuation(&session, &root)?;
        assert_eq!(effects_before(&session)?, before);
        // Default-budget fit is not assumed: an unsuccessful complete review
        // must leave the last published proposal usable and the world intact.
        let reviewed = if attempted["ok"] == true {
            assert_stage(&attempted, "room", true);
            assert!(attempted.to_string().len() <= 4_096 * 4, "{attempted}");
            attempted
        } else {
            assert_eq!(attempted["error"]["code"], "budget_exceeded", "{attempted}");
            assert_eq!(pending_digest(&session)?, retained);
            prior
        };
        in_session(&session, |guard| {
            guard.budget = budget;
            Ok(())
        })?;
        let sealed = pending_plan(&session)?;
        assert_eq!(
            json!(crate::sealed_plan_review::expanded_steps(&reviewed)?),
            crate::lab_world::plan_steps_json(&sealed),
        );
        assert_eq!(
            sealed
                .steps
                .iter()
                .filter(|step| matches!(step.action, Action::PlaceFurniture { .. }))
                .count(),
            2,
        );
        assert_eq!(
            sealed
                .steps
                .iter()
                .filter(|step| matches!(step.action, Action::Build { .. }))
                .count(),
            usize::from(missing_workshop),
        );
        assert!(
            sealed
                .steps
                .iter()
                .any(|step| matches!(step.action, Action::CreateWorkOrder { .. }))
        );
        assert!(
            sealed
                .steps
                .iter()
                .all(|step| !matches!(step.action, Action::HarvestResource { .. }))
        );
        assert_eq!(reviewed["room"]["continuation"]["parent_plan_digest"], root);
        assert_eq!(reviewed["room"]["continuation"]["root_plan_digest"], root);
        commit_ok(&session, &reviewed)?;
        let completed = finish_stage(&session, &identity(&reviewed, "plan_digest")?)?;
        assert_eq!(completed["predicate_truth"], "true", "{completed}");
        assert_eq!(completed["semantic_room_goal"]["truth"], "true");
        let original_now = goal(&session, &root)?;
        assert_history_unchanged(&harvest_history, &original_now);
        assert_eq!(original_now["semantic_room_goal"]["truth"], "true");
        assert_eq!(original_now["needs_replan"], false);
    }
    Ok(())
}

#[test]
fn resource_room_authority_and_complete_publication_refusals_preserve_pending() -> TestResult {
    let _serial = serialized_durable_tests();
    let session = open_resources("9654103", false, false)?;
    require_finite_inputs(&session)?;
    let (root, _) = complete_acquisition(&session, &original_room())?;
    let room = continuation(&session, &root)?;
    assert_stage(&room, "room", true);
    let retained = pending_digest(&session)?;
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
        for refused in [continuation(&session, &root)?, commit(&session, &room)?] {
            assert_eq!(refused["error"]["code"], "capability_denied", "{refused}");
            assert_eq!(pending_digest(&session)?, retained);
            assert_eq!(effects_before(&session)?, before);
        }
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
    for (entities, tokens) in [
        (1, budget.max_output_tokens),
        (budget.max_entities, 1_024),
        (budget.max_entities, 600),
    ] {
        in_session(&session, |guard| {
            guard.budget.max_entities = entities;
            guard.budget.max_output_tokens = tokens;
            Ok(())
        })?;
        let before = effects_before(&session)?;
        let refused = continuation(&session, &root)?;
        assert_eq!(refused["error"]["code"], "budget_exceeded", "{refused}");
        assert_eq!(pending_digest(&session)?, retained);
        assert_eq!(effects_before(&session)?, before);
        in_session(&session, |guard| {
            guard.budget = budget;
            Ok(())
        })?;
    }
    commit_ok(&session, &room)?;
    assert_eq!(
        finish_stage(&session, &identity(&room, "plan_digest")?)?["predicate_truth"],
        "true"
    );
    Ok(())
}

#[test]
fn resource_room_stale_anchors_require_a_fresh_review_at_both_stages() -> TestResult {
    let _serial = serialized_durable_tests();
    let session = open_resources("9654104", false, false)?;
    require_finite_inputs(&session)?;
    let request = original_room();
    let stale_harvest = plan_room(&session, &request)?;
    assert_stage(&stale_harvest, "acquire_resources", false);
    wait(&session, 1)?;
    let before = effects_before(&session)?;
    let refused = commit(&session, &stale_harvest)?;
    assert_eq!(refused["error"]["code"], "stale_anchor", "{refused}");
    assert!(refused["rebased_plan"].is_null());
    assert_eq!(
        pending_digest(&session)?.as_deref(),
        stale_harvest["plan_digest"].as_str()
    );
    assert_eq!(effects_before(&session)?, before);
    let (root, _) = complete_acquisition(&session, &request)?;
    let stale_room = continuation(&session, &root)?;
    assert_stage(&stale_room, "room", true);
    wait(&session, 1)?;
    let before = effects_before(&session)?;
    let refused = commit(&session, &stale_room)?;
    assert_eq!(refused["error"]["code"], "stale_anchor", "{refused}");
    assert!(refused["rebased_plan"].is_null());
    assert_eq!(
        pending_digest(&session)?.as_deref(),
        stale_room["plan_digest"].as_str()
    );
    assert_eq!(effects_before(&session)?, before);
    let reviewed = continuation(&session, &root)?;
    assert_stage(&reviewed, "room", true);
    commit_ok(&session, &reviewed)?;
    assert_eq!(
        finish_stage(&session, &identity(&reviewed, "plan_digest")?)?["predicate_truth"],
        "true"
    );
    Ok(())
}

#[test]
fn shared_resource_room_candidates_cannot_duplicate_an_active_same_root_pursuit() -> TestResult {
    let _serial = serialized_durable_tests();
    let first = open_resources("9654105", true, false)?;
    require_finite_inputs(&first)?;
    let (root, _) = complete_acquisition(&first, &original_room())?;
    let second = open_resources("9654105", true, false)?;
    let chosen = continuation(&first, &root)?;
    let competing = continuation(&second, &root)?;
    assert_stage(&chosen, "room", true);
    assert_stage(&competing, "room", true);
    commit_ok(&first, &chosen)?;
    let before = effects_before(&second)?;
    for refused in [commit(&second, &competing)?, continuation(&second, &root)?] {
        assert_eq!(
            refused["error"]["code"], "preconditions_failed",
            "{refused}"
        );
        assert!(refused["rebased_plan"].is_null());
    }
    assert_eq!(
        pending_digest(&second)?.as_deref(),
        competing["plan_digest"].as_str()
    );
    assert_eq!(effects_before(&second)?, before);
    Ok(())
}

#[test]
fn completed_harvest_cannot_evict_an_unfinished_resource_room_at_goal_capacity() -> TestResult {
    let _serial = serialized_durable_tests();
    let session = open_resources("9654106", false, false)?;
    require_finite_inputs(&session)?;
    let (root, completed) = complete_acquisition(&session, &original_room())?;
    assert_eq!(completed["historical_achievement"], "verified");
    // Every filler is a real admitted then canceled pursuit. Internal handlers
    // isolate capacity from the size of a facade packet containing 64 roots.
    for index in 1..dfmcp_lab::durable::MAX_OBJECTIVES_PER_FORTRESS {
        let proposal = parsed(&plan_request(
            Some(session.clone()),
            Some(format!("retain unfinished furniture production {index}")),
            None,
            None,
            Some(
                json!({"template":"production", "quotas":[{"item":"BED","minimum":26}],
                "acquire_resources":{"assign_labor":false},
                "prerequisites":original_room()["prerequisites"]})
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
        Some("capacity must retain unfinished rooms".to_owned()),
        Some(true),
        None,
        None,
        None,
    ))?;
    assert_eq!(candidate["ok"], true, "{candidate}");
    let retained = pending_digest(&session)?;
    let before = effects_before(&session)?;
    let refused = parsed(&fortress_commit(
        Some(session.clone()),
        identity(&candidate, "plan_digest")?,
    ))?;
    assert_eq!(refused["error"]["code"], "budget_exceeded", "{refused}");
    assert_eq!(pending_digest(&session)?, retained);
    assert_eq!(effects_before(&session)?, before);
    let still_required = goal(&session, &root)?;
    assert_eq!(still_required["predicate_truth"], "true");
    assert_eq!(still_required["semantic_room_goal"]["truth"], "false");
    Ok(())
}

#[test]
fn canceled_resource_room_reopens_and_uses_only_observed_residual_inputs() -> TestResult {
    let _serial = serialized_durable_tests();
    let directory = StateDir::new();
    let first = open_resources("9654107", false, true)?;
    require_finite_inputs(&first)?;
    let acquisition = plan_room(&first, &original_room())?;
    assert_stage(&acquisition, "acquire_resources", false);
    only_wood_harvest(&first, 3)?;
    let root = identity(&acquisition, "plan_digest")?;
    commit_ok(&first, &acquisition)?;
    for _ in 0..100 {
        if wood(&first)? > 0 {
            break;
        }
        wait(&first, 10)?;
    }
    let observed = wood(&first)?;
    assert!(observed > 0 && observed < 3);
    let stopped = parsed(&cancel_in_scope(
        Some(first.clone()),
        Some("stop_future_steps".to_owned()),
        Some("plan".to_owned()),
    ))?;
    assert_eq!(stopped["ok"], true, "{stopped}");
    let history = finish_stage(&first, &root)?;
    assert_eq!(history["predicate_truth"], "false");
    assert_eq!(wood(&first)?, observed);
    with_durable_store(|store| store.compact())?;
    simulate_durable_restart(Some(directory.0.clone()));
    let second = open_resources("9654107", false, true)?;
    assert_eq!(wood(&second)?, observed);
    assert_history_unchanged(&history, &goal(&second, &root)?);
    let residual = continuation(&second, &root)?;
    assert_stage(&residual, "acquire_resources", true);
    only_wood_harvest(&second, 3 - u32::try_from(observed)?)?;
    let residual_digest = identity(&residual, "plan_digest")?;
    commit_ok(&second, &residual)?;
    assert_eq!(
        finish_stage(&second, &residual_digest)?["predicate_truth"],
        "true"
    );
    assert_eq!(wood(&second)?, 3);
    let room = continuation(&second, &residual_digest)?;
    assert_stage(&room, "room", true);
    let child = identity(&room, "plan_digest")?;
    commit_ok(&second, &room)?;
    let completed = finish_stage(&second, &child)?;
    assert_eq!(completed["predicate_truth"], "true", "{completed}");
    let original = goal(&second, &root)?;
    assert_eq!(original["predicate_truth"], "false");
    assert_eq!(original["semantic_room_goal"]["truth"], "true");
    assert_history_unchanged(&history, &original);
    with_durable_store(|store| store.compact())?;
    simulate_durable_restart(Some(directory.0.clone()));
    let third = open_resources("9654107", false, true)?;
    assert_eq!(goal(&third, &child)?["semantic_room_goal"]["truth"], "true");
    assert_history_unchanged(&history, &goal(&third, &root)?);
    let duplicate = continuation(&third, &root)?;
    assert_eq!(duplicate["error"]["code"], "invalid_intent", "{duplicate}");
    Ok(())
}

#[test]
fn resource_room_comparison_cannot_equate_different_rooms_with_the_same_wood_stage() -> TestResult {
    let _serial = serialized_durable_tests();
    let session = open_resources("9654108", false, false)?;
    require_finite_inputs(&session)?;
    let original = original_room();
    let retained = plan_room(&session, &original)?;
    assert_stage(&retained, "acquire_resources", false);
    let mut shifted = original.clone();
    shifted["layout"]["origin"] = json!([3, 6, 10]);
    let before = effects_before(&session)?;
    let compared = plan_room(
        &session,
        &json!({
            "template":"compare_candidates", "version":1, "horizon_ticks":100,
            "candidates":[
                {"key":"original", "blueprint":original},
                {"key":"shifted", "blueprint":shifted},
            ],
        }),
    )?;
    assert_eq!(compared["ok"], false, "{compared}");
    assert_eq!(compared["error"]["code"], "invalid_request", "{compared}");
    assert!(compared["error"]["message"].as_str().is_some_and(
        |message| message.contains("original")
            || message.contains("objective")
            || message.contains("room")
    ));
    assert_eq!(
        pending_digest(&session)?.as_deref(),
        retained["plan_digest"].as_str()
    );
    assert_eq!(effects_before(&session)?, before);
    Ok(())
}

#[test]
fn resource_room_comparison_preserves_complete_room_and_limits_forecasts_to_reviewed_stage()
-> TestResult {
    let _serial = serialized_durable_tests();
    let session = open_resources("9654109", false, false)?;
    require_finite_inputs(&session)?;
    let request = original_room();
    let retained = plan_room(&session, &request)?;
    assert_stage(&retained, "acquire_resources", false);
    let expected_source = &retained["room"]["resource_workflow"]["original_request"];
    let before = effects_before(&session)?;
    let compared = plan_room(
        &session,
        &json!({
            "template":"compare_candidates", "version":1, "horizon_ticks":1_200,
            "candidates":[
                {"key":"first", "blueprint":request.clone()},
                {"key":"second", "blueprint":request},
            ],
        }),
    )?;
    assert_eq!(compared["ok"], true, "{compared}");
    assert_eq!(compared["schema"], "dfmcp.plan-comparison/1");
    assert_eq!(&compared["semantic_room_request"], expected_source);
    assert_eq!(
        compared["semantic_room_request"]["planner"],
        "provisioned_room_resources_v1"
    );
    assert_eq!(compared["goal_scope"], "resource_acquisition_stage");
    for field in ["forecast_scope", "recommendation_scope"] {
        assert_eq!(compared[field], "current_reviewed_stage_only");
    }
    assert_eq!(
        compared["method"],
        "same_original_room_request_current_stage_on_discarded_laboratory_forks"
    );
    let rows = compared["candidates"]
        .as_array()
        .ok_or("resource room comparison candidates missing")?;
    assert_eq!(rows.len(), 2);
    for row in rows {
        assert_eq!(row["status"], "evaluated", "{row}");
        assert_eq!(row["goal_scope"], "resource_acquisition_stage");
        assert_eq!(row["forecast_scope"], "current_reviewed_stage_only");
    }
    assert_eq!(compared["pending_plan_changed"], false);
    assert_eq!(compared["effect_evidence"], false);
    assert_eq!(
        pending_digest(&session)?.as_deref(),
        retained["plan_digest"].as_str()
    );
    assert_eq!(effects_before(&session)?, before);
    assert_eq!(wood(&session)?, 0);
    Ok(())
}
