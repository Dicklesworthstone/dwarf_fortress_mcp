//! Real host acquisition/compiler/custody/monitor paths. The baseline is read
//! through the same bounded operations source used by subsequent goal samples.
use super::*;
use dfmcp_adapter::semantic_work_orders::ObservedInventoryIncreaseObjective;
use dfmcp_world::Predicate;

fn request(seal: Digest32, deadline: u64, increase: u64, samples: u32) -> Result<Request> {
    Request::parse(&json!({
        "expected_observation":seal.to_hex(), "recipe":"wooden_bed", "amount":5,
        "deadline_tick":deadline, "poll_interval_ticks":10, "stable_samples":samples,
        "objective":{"kind":"observed_inventory_increase", "item_type":"BED",
            "increase_stack_units":increase}
    }).to_string())
}

fn observe(runtime: &mut Runtime<Resources>) -> Result<(Digest32, u64)> {
    let c = runtime.context(false)?;
    let view = runtime.observe(&c)?;
    Ok((view.observation.expect("observation").0, runtime.anchor().tick.get() + 100))
}

fn retire(resources: &Resources) -> Result<()> {
    let queue = resources.native.borrow().queue.clone();
    let mut bytes = queue.canonical_bytes().to_vec();
    let offset = 43 + queue.world_folder().len();
    bytes[offset..offset + 4]
        .copy_from_slice(&((queue.order_ids().len() - 1) as u32).to_be_bytes());
    bytes.truncate(bytes.len() - 4);
    resources.native.borrow_mut().queue = WorkOrderObservation::decode(&bytes)?;
    Ok(())
}

#[test]
fn increase_request_is_closed_recipe_bound_and_requires_server_resolution() -> Result<()> {
    let base = json!({"expected_observation":"11".repeat(32), "recipe":"wooden_bed",
        "amount":5, "deadline_tick":1000,
        "objective":{"kind":"observed_inventory_increase", "item_type":"BED", "increase_stack_units":1}});
    for (field, value) in [
        ("increase_stack_units", json!(0)),
        ("increase_stack_units", json!(1_000_001)),
        ("increase_stack_units", json!(-1)),
        ("increase_stack_units", json!(u64::MAX)),
        ("item_type", json!("DOOR")),
        ("item_type", json!("bed")),
        ("baseline_stack_units", json!(0)),
        ("baseline_anchor", json!({})),
        ("baseline_source_digest", json!("11".repeat(32))),
        ("source_generation", json!(601)),
        ("baseline_source_counters", json!([0,0,0])),
        ("target_stack_units", json!(1)),
    ] {
        let mut bad = base.clone();
        bad["objective"][field] = value;
        assert!(Request::parse(&bad.to_string()).is_err(), "accepted {field}");
    }
    let mut maximum = base;
    maximum["objective"]["increase_stack_units"] = json!(1_000_000);
    Request::parse(&maximum.to_string())?;
    let resources = Resources::new()?;
    let mut runtime = start(&resources)?;
    let (seal, deadline) = observe(&mut runtime)?;
    let c = runtime.context(false)?;
    let basis = runtime.basis.as_ref().expect("basis");
    let policy = resources.pins.citizens(&basis.capsule, &basis.projection, &c)?;
    assert!(request(seal, deadline, 1, 1)?.compile(
        &PredicateEvidence::scoped(&basis.projection.snapshot, policy)?, &c
    ).is_err());
    assert_eq!(resources.operations_acquisitions.get(), 0);
    assert!(resources.archive.data.borrow().bytes.is_empty());
    Ok(())
}

#[test]
fn sealed_increase_uses_existing_stock_once_and_survives_retirement_and_recovery() -> Result<()> {
    let resources = Resources::new()?;
    resources.item_count.set(2);
    let mut runtime = start(&resources)?;
    let (seal, deadline) = observe(&mut runtime)?;
    let c = runtime.context(false)?;
    let prepared = runtime.plan(request(seal, deadline, 1, 2)?, &c)?;
    let original = prepared.original.as_ref().expect("original").clone();
    let goal = ObservedInventoryIncreaseObjective::from_plan(&original).expect("increase");
    assert_eq!(goal.baseline_stack_units(), 2);
    assert_eq!(goal.increase_stack_units(), 1);
    assert_eq!(goal.target_stack_units(), 3);
    assert_eq!(goal.baseline_anchor(), original.anchor);
    assert_eq!(goal.source_generation(), resources.pins.operations_generation);
    assert_eq!(goal.baseline_source_counters(), [100,100,100]);
    assert_ne!(goal.baseline_source_digest(), Digest32::ZERO);
    assert!(matches!(original.terminal_condition, Predicate::EntityQuantityIncrease { .. }));
    assert_eq!(resources.operations_acquisitions.get(), 1);

    // Repetition binds the original census even if the current stock changed.
    resources.item_count.set(17);
    let c = runtime.context(false)?;
    assert_eq!(runtime.plan(request(seal, deadline, 1, 2)?, &c)?.original, Some(original.clone()));
    let c = runtime.context(false)?;
    assert!(runtime.plan(request(seal, deadline, 2, 2)?, &c).is_err());
    assert_eq!(resources.operations_acquisitions.get(), 1);
    assert_eq!(resources.native.borrow().prepares, 1);
    resources.item_count.set(2);
    let c = runtime.context(false)?;
    runtime.commit(prepared.review.as_ref().expect("review").seal(), true, &c)?;
    resources.advance(10, 0)?;
    retire(&resources)?;
    let c = runtime.context(false)?;
    let unchanged = runtime.wait(&c)?;
    assert_eq!(unchanged.goal.as_ref().expect("goal").current_goal, PredicateTruth::False);
    assert!(!unchanged.goal.as_ref().expect("goal").receipt_linked);

    resources.item_count.set(3);
    resources.advance(10, 0)?;
    let c = runtime.context(false)?;
    let first = runtime.wait(&c)?;
    assert_eq!(first.goal.as_ref().expect("goal").current_goal, PredicateTruth::True);
    assert!(!first.goal.as_ref().expect("goal").current_goal_proven());
    let c = runtime.context(false)?;
    assert!(!runtime.wait(&c)?.goal.as_ref().expect("goal").current_goal_proven());
    resources.advance(10, 0)?;
    let c = runtime.context(false)?;
    let fulfilled = runtime.wait(&c)?;
    assert!(fulfilled.goal.as_ref().expect("goal").current_goal_proven());
    assert!(!fulfilled.goal.as_ref().expect("goal").receipt_linked);
    assert_eq!(fulfilled.to_json()["current_goal"]["semantic_completion_proven"], false);
    assert_eq!(fulfilled.to_json()["inventory_objective"]["baseline_stack_units"], 2);
    assert_eq!(fulfilled.to_json()["inventory_objective"]["production_attribution"], "unknown");

    resources.item_count.set(1);
    resources.advance(10, 0)?;
    let c = runtime.context(false)?;
    let consumed = runtime.wait(&c)?;
    assert_eq!(consumed.goal.as_ref().expect("goal").current_goal, PredicateTruth::False);
    assert_eq!(consumed.first_satisfied_anchor, fulfilled.first_satisfied_anchor);
    let acquired = resources.operations_acquisitions.get();
    let writes = resources.native.borrow().commits;
    let recovery = resources.recover(&consumed);
    drop(runtime);
    let mut reopened = Runtime::from_shell(recovery, Mode::Offline, SessionId::new(11), budget())?;
    let c = reopened.context(false)?;
    let historical = reopened.view(&c)?;
    assert_eq!(historical.original, Some(original));
    assert_eq!(historical.first_satisfied_anchor, fulfilled.first_satisfied_anchor);
    assert!(historical.goal.is_none());
    assert_eq!(historical.to_json()["inventory_objective"]["baseline_stack_units"], 2);
    assert_eq!(historical.to_json()["inventory_objective"]["proven_at_anchor"], false);
    assert_eq!(resources.operations_acquisitions.get(), acquired);
    assert_eq!(resources.native.borrow().commits, writes);
    Ok(())
}

#[test]
fn baseline_refuses_bad_sources_and_budget_before_original_or_native_preparation() -> Result<()> {
    for mode in 0..5 {
        let resources = Resources::new()?;
        resources.item_count.set(1);
        let mut runtime = start(&resources)?;
        let (seal, deadline) = observe(&mut runtime)?;
        let mut capture = operations(&resources.native.borrow().queue, true, false);
        match mode {
            0 => capture.jobs.bridge_generation += 1,
            1 => capture.jobs.paused = false,
            2 => capture.jobs.year_tick += 1,
            3 => capture.jobs.df_version = "different".into(),
            _ => {}
        }
        *resources.operations_override.borrow_mut() = Some(capture);
        let mut c = runtime.context(false)?;
        if mode == 4 {
            c.budget.max_bytes = inventory_baseline::BASELINE_BYTES;
        }
        assert!(runtime.plan(request(seal, deadline, 1, 1)?, &c).is_err());
        assert!(runtime.original.is_none());
        assert!(resources.archive.data.borrow().bytes.is_empty());
        assert_eq!(resources.native.borrow().prepares, 0);
        assert_eq!(resources.native.borrow().commits, 0);
        if mode == 4 {
            assert_eq!(resources.operations_acquisitions.get(), 0);
        }
    }
    Ok(())
}

#[test]
fn failed_future_baseline_keeps_authority_floor_without_relabeling_original_anchor() -> Result<()> {
    let resources = Resources::new()?;
    let mut runtime = start(&resources)?;
    let (seal, deadline) = observe(&mut runtime)?;
    let original = runtime.anchor();
    let mut capture = operations(&resources.native.borrow().queue, false, false);
    capture.jobs.year_tick += 10;
    *resources.operations_override.borrow_mut() = Some(capture);
    let c = runtime.context(false)?;
    assert!(runtime.plan(request(seal, deadline, 1, 1)?, &c).is_err());
    assert_eq!(runtime.anchor(), original);
    assert_eq!(runtime.inventory_baseline_tick_floor, GameTick(original.tick.get() + 10));
    assert_eq!(resources.operations_acquisitions.get(), 1);
    *resources.operations_override.borrow_mut() = None;
    let c = runtime.context(false)?;
    assert!(runtime.plan(request(seal, deadline, 1, 1)?, &c).is_err());
    assert_eq!(resources.operations_acquisitions.get(), 1);

    for objective in [
        None,
        Some(request::Objective::ObservedInventoryMinimum {
            item_type:"BED".into(), minimum_stack_units:1,
        }),
    ] {
        let mut replacement = request(seal, deadline, 1, 1)?;
        replacement.objective = objective;
        let c = runtime.context(false)?;
        assert!(runtime.plan(replacement, &c).is_err());
    }
    assert!(resources.archive.data.borrow().bytes.is_empty());
    assert_eq!(resources.native.borrow().prepares, 0);
    assert_eq!(resources.operations_acquisitions.get(), 1);

    let acquired = resources.citizen_acquisitions.get();
    let mut expired = runtime.context(false)?;
    for grant in &mut expired.grants {
        if grant.capability == Capability::Observe {
            grant.expires_at_tick = Some(original.tick);
        }
    }
    assert!(runtime.plan(request(seal, deadline, 1, 1)?, &expired).is_err());
    assert!(runtime.observe(&expired).is_err());
    assert_eq!(resources.citizen_acquisitions.get(), acquired);
    assert_eq!(resources.operations_acquisitions.get(), 1);
    resources.advance(10, 5)?;
    let (fresh, deadline) = observe(&mut runtime)?;
    let c = runtime.context(false)?;
    runtime.plan(request(fresh, deadline, 1, 1)?, &c)?;
    assert_eq!(resources.operations_acquisitions.get(), 2);
    Ok(())
}

#[test]
fn all_five_compilers_share_one_qualified_census_and_seal_each_original_leaf() -> Result<()> {
    use crate::live_production_runtime::{
        ExistingLaborPortfolioRequest, ExistingLaborRequest, PortfolioRequest,
        Request as CombinedRequest,
    };
    use dfmcp_adapter::live_production::ExistingLaborRequirement;
    let resources = Resources::new()?;
    resources.item_count.set(3);
    let mut runtime = start(&resources)?;
    let (seal, deadline) = observe(&mut runtime)?;
    let mut c = runtime.context(false)?;
    c.budget.max_actions = 5;
    let basis = runtime.basis.as_ref().expect("basis");
    let policy = resources.pins.citizens(&basis.capsule, &basis.projection, &c)?;
    let evidence = PredicateEvidence::scoped(&basis.projection.snapshot, policy)?;
    let unit = basis.projection.snapshot.graph.entities.values()
        .find(|entity| entity.kind == EntityKind::Unit).expect("citizen").id;
    let bed = request::Objective::ObservedInventoryIncrease {
        item_type:"BED".into(), increase_stack_units:2,
    };
    let door = request::Objective::ObservedInventoryIncrease {
        item_type:"DOOR".into(), increase_stack_units:1,
    };
    let mut floor = c.anchor.tick;
    let baselines = inventory_baseline::acquire(&resources, &[&bed, &door], &mut floor, &c)?;
    assert_eq!(resources.operations_acquisitions.get(), 1);
    resources.item_count.set(9);
    let standalone = request(seal, deadline, 2, 2)?
        .compile_with_inventory(&evidence, &baselines, &c)?;
    let mut common = json!({"expected_observation":seal.to_hex(), "units":[unit.get().to_string()],
        "labor":"CARPENTER", "enabled":true, "recipe":"wooden_bed", "amount":5,
        "deadline_tick":deadline, "poll_interval_ticks":10, "stable_samples":2,
        "objective":{"kind":"observed_inventory_increase","item_type":"BED","increase_stack_units":2}});
    let assignment = CombinedRequest::parse(&common.to_string())?
        .compile_with_inventory(&evidence, &baselines, &c)?;
    common.as_object_mut().expect("object").remove("enabled");
    common["kind"] = json!("existing_labor");
    let requirement = ExistingLaborRequirement::new("CARPENTER".into(), vec![(unit, 900)])?;
    let existing = ExistingLaborRequest::parse(&common.to_string())?
        .compile_with_inventory(&evidence, &requirement, &baselines, &c)?;

    let mut portfolio = json!({"kind":"portfolio", "expected_observation":seal.to_hex(),
        "units":[unit.get().to_string()],"labor":"CARPENTER","enabled":true,
        "orders":[
            {"recipe":"wooden_door","amount":1,"objective":{"kind":"observed_inventory_increase","item_type":"DOOR","increase_stack_units":1}},
            {"recipe":"wooden_bed","amount":5,"objective":{"kind":"observed_inventory_increase","item_type":"BED","increase_stack_units":2}}
        ], "deadline_tick":deadline,"poll_interval_ticks":10,"stable_samples":2});
    let assigned_portfolio = PortfolioRequest::parse(&portfolio.to_string())?
        .compile_with_inventory(&evidence, &baselines, &c)?;
    portfolio["orders"].as_array_mut().expect("orders").reverse();
    assert_eq!(assigned_portfolio, PortfolioRequest::parse(&portfolio.to_string())?
        .compile_with_inventory(&evidence, &baselines, &c)?);
    portfolio.as_object_mut().expect("object").remove("enabled");
    portfolio["kind"] = json!("existing_labor_portfolio");
    let existing_portfolio = ExistingLaborPortfolioRequest::parse(&portfolio.to_string())?
        .compile_with_inventory(&evidence, &requirement, &baselines, &c)?;
    let source = baselines.get("BED", 2, c.anchor)?.baseline_source_digest();
    for (plan, count) in [
        (&standalone,1), (&assignment,1), (&existing,1),
        (&assigned_portfolio,2), (&existing_portfolio,2),
    ] {
        let goals: Vec<_> = plan.steps.iter().filter_map(|step|
            ObservedInventoryIncreaseObjective::from_selected_plan(plan, step.id)).collect();
        assert_eq!(goals.len(), count);
        for goal in goals {
            assert_eq!(goal.baseline_anchor(), c.anchor);
            assert_eq!(goal.baseline_source_digest(), source);
            assert_eq!(goal.baseline_stack_units(), if goal.item_type() == "BED" {3} else {0});
        }
        let retained = inventory_baseline::InventoryBaselines::from_original(plan)?;
        assert_eq!(retained.get("BED", 2, c.anchor)?.baseline_stack_units(), 3);
    }
    assert_eq!(resources.operations_acquisitions.get(), 1);
    let changed = inventory_baseline::acquire(&resources, &[&bed], &mut floor, &c)?;
    let replacement = request(seal, deadline, 2, 2)?
        .compile_with_inventory(&evidence, &changed, &c)?;
    assert_ne!(replacement.intent_id, standalone.intent_id);
    assert_ne!(replacement.steps[0].idempotency_key, standalone.steps[0].idempotency_key);
    assert_eq!(resources.native.borrow().prepares, 0);
    Ok(())
}

#[test]
fn increase_goal_first_sample_rejects_original_source_counter_or_generation_regression() -> Result<()> {
    for counter in [false, true] {
        let resources = Resources::new()?;
        resources.item_count.set(1);
        let mut runtime = start(&resources)?;
        let (seal, deadline) = observe(&mut runtime)?;
        let c = runtime.context(false)?;
        let prepared = runtime.plan(request(seal, deadline, 1, 1)?, &c)?;
        let c = runtime.context(false)?;
        runtime.commit(prepared.review.as_ref().expect("review").seal(), true, &c)?;
        resources.item_count.set(2);
        resources.advance(10, 0)?;
        let mut raw = operations(&resources.native.borrow().queue, true, false);
        if counter {
            raw.next_item_id = 99;
        } else {
            raw.jobs.bridge_generation += 1;
        }
        *resources.operations_override.borrow_mut() = Some(raw);
        let c = runtime.context(false)?;
        assert!(runtime.wait(&c).is_err());
        assert!(runtime.monitor.as_ref().is_none_or(|monitor| monitor.latest().is_none()));
        assert_eq!(resources.native.borrow().commits, 1);
        assert_eq!(resources.operations_acquisitions.get(), 2);
    }
    Ok(())
}

#[test]
fn public_agent_turn_keeps_inventory_increase_separate_from_order_completion() -> Result<()> {
    let resources = Resources::new()?;
    resources.item_count.set(2);
    let mut runtime = start(&resources)?;
    let (seal, deadline) = observe(&mut runtime)?;
    let c = runtime.context(false)?;
    let prepared = runtime.plan(request(seal, deadline, 1, 1)?, &c)?;
    let c = runtime.context(false)?;
    runtime.commit(prepared.review.as_ref().expect("review").seal(), true, &c)?;
    resources.item_count.set(3);
    resources.advance(10, 0)?;
    retire(&resources)?;
    let c = runtime.context(false)?;
    let complete = runtime.wait(&c)?;
    let packet = crate::live_semantic_work_orders_server::test_packet_for_view(
        &complete, runtime.session_id(), budget(),
    );
    let packet: Value = serde_json::from_str(&packet).expect("public packet");
    let briefing = &packet["agent_turn"]["briefing"];
    assert_eq!(briefing["inventory_objective"]["kind"], "observed_inventory_increase");
    assert_eq!(briefing["inventory_objective"]["baseline_stack_units"], 2);
    assert_eq!(briefing["inventory_objective"]["target_stack_units"], 3);
    assert_eq!(briefing["inventory_objective"]["production_attribution"], "unknown");
    assert_eq!(briefing["inventory_objective_proven_at_anchor"], true);
    assert_eq!(complete.to_json()["current_goal"]["semantic_completion_proven"], false);
    assert!(packet["agent_turn"]["recommendations"].as_array().expect("recommendations")
        .iter().all(|row| row["tool"] != "fortress_wait" && row["tool"] != "fortress.wait"));
    Ok(())
}
