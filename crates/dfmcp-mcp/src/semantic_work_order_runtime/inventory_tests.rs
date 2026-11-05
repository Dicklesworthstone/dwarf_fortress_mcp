//! Execute the original compiler, journals, monitor and replay with native
//! fixture responses. Inventory proof never supplies missing order causation.
use super::*;

fn inventory_request(seal: Digest32, deadline: u64, minimum: u64, samples: u32) -> Result<Request> {
    Request::parse(
        &json!({
            "expected_observation":seal.to_hex(), "recipe":"wooden_bed", "amount":5,
            "deadline_tick":deadline, "poll_interval_ticks":10, "stable_samples":samples,
            "objective":{"kind":"observed_inventory_minimum", "item_type":"BED",
                "minimum_stack_units":minimum}
        })
        .to_string(),
    )
}

fn prepare_inventory(runtime: &mut Runtime<Resources>, minimum: u64, samples: u32) -> Result<View> {
    let c = runtime.context(false)?;
    let observed = runtime.observe(&c)?;
    let request = inventory_request(
        observed.observation.expect("seal").0,
        runtime.anchor().tick.get() + 100,
        minimum,
        samples,
    )?;
    let c = runtime.context(false)?;
    runtime.plan(request, &c)
}

fn retire_order(resources: &Resources) -> Result<()> {
    let queue = resources.native.borrow().queue.clone();
    let mut bytes = queue.canonical_bytes().to_vec();
    let count_offset = 43 + queue.world_folder().len();
    let count = queue.order_ids().len();
    assert_eq!(queue.order_ids().last(), Some(&(queue.next_order_id() - 1)));
    bytes[count_offset..count_offset + 4].copy_from_slice(&((count - 1) as u32).to_be_bytes());
    bytes.truncate(bytes.len() - 4);
    resources.native.borrow_mut().queue = WorkOrderObservation::decode(&bytes)?;
    Ok(())
}

#[test]
fn inventory_request_is_explicit_closed_recipe_bound_and_immutable() -> Result<()> {
    let base = json!({"expected_observation":"11".repeat(32), "recipe":"wooden_bed",
        "amount":5,"deadline_tick":100,
        "objective":{"kind":"observed_inventory_minimum","item_type":"BED","minimum_stack_units":1}});
    for objective in [
        json!({"kind":"observed_inventory_minimum","item_type":"BED","minimum_stack_units":0}),
        json!({"kind":"observed_inventory_minimum","item_type":"BED","minimum_stack_units":1000001}),
        json!({"kind":"observed_inventory_minimum","item_type":"DOOR","minimum_stack_units":1}),
        json!({"kind":"observed_inventory_minimum","item_type":"bed","minimum_stack_units":1}),
        json!({"kind":"observed_inventory_minimum","item_type":"BED","minimum_stack_units":1,"newly_produced":true}),
        json!({"kind":"produced_inventory","item_type":"BED","minimum_stack_units":1}),
    ] {
        let mut invalid = base.clone();
        invalid["objective"] = objective;
        assert!(Request::parse(&invalid.to_string()).is_err());
    }
    let resources = Resources::new()?;
    let mut runtime = start(&resources)?;
    let prepared = prepare_inventory(&mut runtime, 1, 2)?;
    let original = prepared.original.as_ref().expect("original");
    let objective = ObservedInventoryObjective::from_plan(original).expect("explicit inventory");
    assert_eq!(objective.item_type, "BED");
    assert_eq!(objective.min_stack_units, 1);
    let seal = prepared.observation.expect("observation").0;
    let deadline = original.steps[0]
        .obligation
        .as_ref()
        .expect("obligation")
        .deadline_tick
        .get();
    let c = runtime.context(false)?;
    let repeated = runtime.plan(inventory_request(seal, deadline, 1, 2)?, &c)?;
    assert_eq!(repeated.original, prepared.original);
    let c = runtime.context(false)?;
    assert!(
        runtime
            .plan(inventory_request(seal, deadline, 2, 2)?, &c)
            .is_err()
    );
    assert_eq!(resources.native.borrow().prepares, 1);
    assert_eq!(resources.native.borrow().commits, 0);
    Ok(())
}

#[test]
fn inventory_goal_survives_retirement_and_recovers_without_redispatch() -> Result<()> {
    let resources = Resources::new()?;
    let mut runtime = start(&resources)?;
    let prepared = prepare_inventory(&mut runtime, 1, 2)?;
    let c = runtime.context(false)?;
    runtime.commit(prepared.review.as_ref().expect("review").seal(), true, &c)?;
    resources.item_count.set(1);
    resources.advance(10, 1)?;
    retire_order(&resources)?;
    let c = runtime.context(false)?;
    let first = runtime.wait(&c)?;
    assert_eq!(
        first.to_json()["inventory_objective"]["current_truth"],
        "true"
    );
    assert_eq!(
        first.to_json()["inventory_objective"]["proven_at_anchor"],
        false
    );
    let c = runtime.context(false)?;
    let same_tick = runtime.wait(&c)?;
    assert_eq!(
        same_tick.to_json()["inventory_objective"]["proven_at_anchor"],
        false
    );
    resources.advance(10, 1)?;
    let c = runtime.context(false)?;
    let done = runtime.wait(&c)?;
    let value = done.to_json();
    assert_eq!(value["inventory_objective"]["proven_at_anchor"], true);
    assert_eq!(
        value["inventory_objective"]["production_attribution"],
        "unknown"
    );
    assert_eq!(
        value["inventory_objective"]["material_classification"],
        "unknown"
    );
    assert_eq!(value["current_goal"]["receipt_linked"], false);
    assert_eq!(value["current_goal"]["semantic_completion_proven"], false);
    assert!(matches!(
        done.goal.as_ref().expect("goal").physical_work,
        dfmcp_adapter::semantic_work_orders::WorkOrderPhysicalState::Unknown { .. }
    ));
    assert!(done.first_satisfied_anchor.is_some());
    assert_eq!(done.original, prepared.original);
    let packet = crate::live_semantic_work_orders_server::test_packet_for_view(
        &done,
        runtime.session_id(),
        budget(),
    );
    let packet: Value = serde_json::from_str(&packet).expect("packet");
    assert_eq!(
        packet["agent_turn"]["briefing"]["inventory_objective_proven_at_anchor"],
        true
    );
    assert_eq!(
        packet["agent_turn"]["briefing"]["semantic_completion_proven_at_anchor"],
        false
    );
    assert!(
        packet["agent_turn"]["recommendations"]
            .as_array()
            .expect("next")
            .is_empty()
    );
    let handoff = resources.recover(&done);
    let history = resources.history.data.borrow().bytes.clone();
    drop(runtime);
    let mut offline =
        Runtime::from_shell(handoff.clone(), Mode::Offline, SessionId::new(51), budget())?;
    let c = offline.context(false)?;
    let reopened = offline.inspect(&c)?;
    assert_eq!(reopened.original, done.original);
    assert_eq!(reopened.first_satisfied_anchor, done.first_satisfied_anchor);
    assert!(reopened.goal.is_none());
    assert_eq!(
        reopened.to_json()["inventory_objective"]["proven_at_anchor"],
        false
    );
    assert_eq!(resources.history.data.borrow().bytes, history);
    drop(offline);
    let mut online = Runtime::from_shell(handoff, Mode::Reconcile, SessionId::new(52), budget())?;
    resources.item_count.set(0);
    resources.advance(10, 1)?;
    let c = online.context(false)?;
    let consumed = online.wait(&c)?;
    assert_eq!(
        consumed.to_json()["inventory_objective"]["current_truth"],
        "false"
    );
    assert_eq!(
        consumed.to_json()["inventory_objective"]["proven_at_anchor"],
        false
    );
    assert_eq!(consumed.first_satisfied_anchor, done.first_satisfied_anchor);
    assert_eq!(resources.native.borrow().commits, 1);
    Ok(())
}

#[test]
fn inventory_proof_counts_full_roster_stack_units_and_preexisting_items() -> Result<()> {
    let resources = Resources::new()?;
    let mut items = Vec::new();
    let row = operations(&resources.native.borrow().queue, true, false)
        .items
        .remove(0);
    for n in 0..20 {
        let mut item = row.clone();
        item.native_id = 2 + n;
        item.stack_size = 2;
        items.push(item);
    }
    *resources.items_override.borrow_mut() = Some(items);
    let mut runtime = start(&resources)?;
    let prepared = prepare_inventory(&mut runtime, 35, 1)?;
    let c = runtime.context(false)?;
    runtime.commit(prepared.review.as_ref().expect("review").seal(), true, &c)?;
    resources.advance(10, 5)?;
    let c = runtime.context(false)?;
    let sample = runtime.wait(&c)?.to_json();
    assert_eq!(sample["item_observations"]["returned"], 16);
    assert_eq!(sample["item_observations"]["omitted"], 4);
    assert_eq!(sample["inventory_objective"]["proven_at_anchor"], true);
    assert_eq!(
        sample["inventory_objective"]["preexisting_items_included"],
        true
    );
    assert_eq!(sample["current_goal"]["semantic_completion_proven"], false);
    assert_eq!(resources.remaining.get(), 5);
    assert_eq!(resources.native.borrow().commits, 1);
    Ok(())
}

#[test]
fn inventory_goal_rechecks_authority_source_and_original_deadline() -> Result<()> {
    let resources = Resources::new()?;
    let mut runtime = start(&resources)?;
    let prepared = prepare_inventory(&mut runtime, 1, 2)?;
    let c = runtime.context(false)?;
    runtime.commit(prepared.review.as_ref().expect("review").seal(), true, &c)?;
    resources.item_count.set(1);
    resources.advance(10, 4)?;
    let mut denied = runtime.context(false)?;
    denied
        .grants
        .retain(|grant| grant.capability != Capability::Observe);
    assert!(runtime.wait(&denied).is_err());
    let c = runtime.context(false)?;
    let first = runtime.wait(&c)?;
    assert!(!first.goal.as_ref().expect("sample").current_goal_proven());
    resources.changed_operations_generation.set(true);
    resources.advance(10, 3)?;
    let c = runtime.context(false)?;
    assert!(runtime.wait(&c).is_err());
    assert!(
        runtime
            .monitor
            .as_ref()
            .expect("monitor")
            .latest()
            .is_none()
    );
    assert_eq!(resources.native.borrow().commits, 1);

    let resources = Resources::new()?;
    let mut runtime = start(&resources)?;
    let prepared = prepare_inventory(&mut runtime, 1, 1)?;
    let c = runtime.context(false)?;
    runtime.commit(prepared.review.as_ref().expect("review").seal(), true, &c)?;
    resources.item_count.set(1);
    resources.advance(101, 4)?;
    let c = runtime.context(false)?;
    let expired = runtime.wait(&c)?;
    assert_eq!(
        expired.to_json()["inventory_objective"]["current_truth"],
        "true"
    );
    assert_eq!(
        expired.to_json()["inventory_objective"]["proven_at_anchor"],
        false
    );
    assert!(expired.first_satisfied_anchor.is_none());
    assert_eq!(expired.original, prepared.original);
    Ok(())
}

#[test]
fn denied_wait_withdraws_inventory_sample_and_interrupts_the_unfinished_streak() -> Result<()> {
    let resources = Resources::new()?;
    let mut runtime = start(&resources)?;
    let prepared = prepare_inventory(&mut runtime, 1, 2)?;
    let c = runtime.context(false)?;
    runtime.commit(prepared.review.as_ref().expect("review").seal(), true, &c)?;
    resources.item_count.set(1);
    resources.advance(10, 4)?;
    let c = runtime.context(false)?;
    let first = runtime.wait(&c)?;
    assert!(!first.goal.as_ref().expect("sample").current_goal_proven());
    let history = resources.history.data.borrow().bytes.clone();
    let mut denied = runtime.context(false)?;
    denied
        .grants
        .retain(|grant| grant.capability != Capability::Observe);
    assert!(runtime.wait(&denied).is_err());
    assert_eq!(resources.history.data.borrow().bytes, history);
    let c = runtime.context(false)?;
    let withdrawn = runtime.inspect(&c)?;
    assert!(withdrawn.goal.is_none());
    assert_eq!(
        withdrawn.to_json()["inventory_objective"]["proven_at_anchor"],
        false
    );
    resources.advance(10, 3)?;
    let c = runtime.context(false)?;
    let restarted = runtime.wait(&c)?;
    assert!(
        !restarted
            .goal
            .as_ref()
            .expect("sample")
            .current_goal_proven()
    );
    resources.advance(10, 2)?;
    let c = runtime.context(false)?;
    let done = runtime.wait(&c)?;
    assert!(done.goal.as_ref().expect("sample").current_goal_proven());
    let historical = done.first_satisfied_anchor;
    let mut denied = runtime.context(false)?;
    denied.budget.max_bytes = 1;
    assert!(runtime.wait(&denied).is_err());
    let c = runtime.context(false)?;
    let withdrawn = runtime.inspect(&c)?;
    assert!(withdrawn.goal.is_none());
    assert_eq!(withdrawn.first_satisfied_anchor, historical);
    assert_eq!(resources.native.borrow().commits, 1);
    Ok(())
}

#[test]
fn cancelled_preparation_cannot_turn_preexisting_inventory_into_goal_proof() -> Result<()> {
    let resources = Resources::new()?;
    resources.item_count.set(1);
    let mut runtime = start(&resources)?;
    let prepared = prepare_inventory(&mut runtime, 1, 1)?;
    let c = runtime.context(false)?;
    runtime.cancel(&c)?;
    resources.advance(10, 5)?;
    let c = runtime.context(false)?;
    let cancelled = runtime.wait(&c)?;
    assert_eq!(cancelled.original, prepared.original);
    assert!(cancelled.goal.is_none());
    assert_eq!(
        cancelled.to_json()["inventory_objective"]["proven_at_anchor"],
        false
    );
    assert_eq!(resources.native.borrow().commits, 0);
    Ok(())
}

#[test]
fn inventory_truth_cannot_reconcile_an_uncertain_insertion_or_authorize_redispatch() -> Result<()> {
    let resources = Resources::new()?;
    let mut runtime = start(&resources)?;
    let prepared = prepare_inventory(&mut runtime, 1, 1)?;
    let review = prepared.review.as_ref().expect("review");
    resources.native.borrow_mut().commit_state = WorkOrderState::Unknown;
    let c = runtime.context(false)?;
    let _ = runtime.commit(review.seal(), true, &c);
    let c = runtime.context(false)?;
    assert!(
        runtime
            .inspect(&c)?
            .native
            .as_ref()
            .expect("native")
            .state()
            .unresolved()
    );
    // The game may have changed despite the unverified receipt. Observed stock
    // and an apparent matching order still cannot supply the missing receipt.
    resources.native.borrow_mut().queue = after(review.native_plan())?;
    resources.item_count.set(1);
    resources.advance(10, 4)?;
    let c = runtime.context(false)?;
    let observed = runtime.wait(&c)?;
    let value = observed.to_json();
    assert_eq!(value["inventory_objective"]["proven_at_anchor"], true);
    assert_eq!(value["insertion"]["verified"], false);
    assert_eq!(value["current_goal"]["receipt_linked"], false);
    assert_eq!(value["current_goal"]["semantic_completion_proven"], false);
    assert!(
        observed
            .native
            .as_ref()
            .expect("native")
            .state()
            .unresolved()
    );
    let packet = crate::live_semantic_work_orders_server::test_packet_for_view(
        &observed,
        runtime.session_id(),
        budget(),
    );
    let packet: Value = serde_json::from_str(&packet).expect("packet");
    assert!(
        !packet["agent_turn"]["active_work"]["indeterminate_effects"]
            .as_array()
            .expect("uncertainty")
            .is_empty()
    );
    assert!(
        !packet["agent_turn"]["recommendations"]
            .as_array()
            .expect("next")
            .is_empty()
    );
    let c = runtime.context(false)?;
    let _ = runtime.commit(review.seal(), true, &c);
    assert_eq!(resources.native.borrow().commits, 1);
    Ok(())
}
