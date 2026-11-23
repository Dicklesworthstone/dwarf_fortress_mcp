//! Recovery selector and source-frontier mechanics. These values are never
//! admitted as native effect evidence or current goal proof.
use super::*;
use serde_json::json;

fn identity(label: &str) -> Digest32 {
    Digest32::of_bytes(label.as_bytes())
}

fn recovery() -> Recovery {
    Recovery {
        seal: PlanArchiveSeal {
            plan_digest: identity("mechanical original plan"),
            object_digest: identity("mechanical archive object"),
        },
        anchor: StateAnchor {
            fortress_id: FortressId::new(7),
            cursor: ObservationCursor {
                epoch: 3,
                sequence: 5,
            },
            tick: GameTick(100),
            state_hash: identity("mechanical original state"),
        },
        identity_digest: identity("mechanical original citizen capsule"),
        frontier: durable::Recovery {
            journal_id: identity("mechanical original frontier"),
            head: identity("parent acknowledged frontier head"),
            authority_tick_floor: 150,
        },
        goal_history_present: true,
        clock_journal_id: Some(identity("mechanical order clock")),
        inventory_clock_journal_id: Some(identity("mechanical inventory clock")),
        portfolio: None,
        workshop_prerequisite_digest: None,
        workshop_prerequisite_floor: None,
        manager_prerequisite_digest: None,
        manager_prerequisite_floor: None,
        input_supply_digest: None,
        input_supply_floor: None,
    }
}

fn original_input(recovery: Recovery) -> Result<RecoveryInput> {
    serde_json::from_value(json!({
        "plan_digest": recovery.seal.plan_digest.to_hex(),
        "object_digest": recovery.seal.object_digest.to_hex(),
        "identity_digest": recovery.identity_digest.to_hex(),
        "anchor": {
            "fortress_id": recovery.anchor.fortress_id.get().to_string(),
            "epoch": recovery.anchor.cursor.epoch,
            "sequence": recovery.anchor.cursor.sequence,
            "tick": recovery.anchor.tick.get(),
            "state_hash": recovery.anchor.state_hash.to_hex(),
        },
        "frontier": {
            "journal_id": recovery.frontier.journal_id.to_hex(),
            "head": recovery.frontier.head.to_hex(),
            "authority_tick_floor": recovery.frontier.authority_tick_floor,
        },
        "goal_history_present": recovery.goal_history_present,
        "clock_journal_id": recovery.clock_journal_id.map(|id| id.to_hex()),
        "inventory_clock_journal_id": recovery.inventory_clock_journal_id.map(|id| id.to_hex()),
        "portfolio": recovery.portfolio.map(|slots| slots.into_iter().flatten().map(|slot| json!({
            "step_id": slot.step_id, "goal_history_present": slot.goal_history_present,
            "clock_journal_id": slot.clock_journal_id.map(|id| id.to_hex()),
            "inventory_clock_journal_id": slot.inventory_clock_journal_id.map(|id| id.to_hex()),
        })).collect::<Vec<_>>()),
    }))
    .map_err(|_| invalid("test recovery input"))
}

#[test]
fn parent_original_pin_cannot_be_replaced_by_older_configured_frontier() -> Result<()> {
    let current = recovery();
    let prior = original_input(current)?;
    original(current, &prior, 150)?;
    let mut older = current;
    older.frontier.head = identity("older valid frontier head");
    assert!(original(older, &prior, 150).is_err());
    older = current;
    older.frontier.journal_id = identity("other frontier custodian");
    assert!(original(older, &prior, 150).is_err());
    older = current;
    older.identity_digest = identity("other citizen capsule");
    assert!(original(older, &prior, 150).is_err());
    older = current;
    older.anchor.cursor.sequence += 1;
    assert!(original(older, &prior, 150).is_err());
    older = current;
    older.anchor.tick = GameTick(101);
    assert!(original(older, &prior, 150).is_err());
    older = current;
    older.goal_history_present = false;
    assert!(original(older, &prior, 150).is_err());
    older = current;
    older.frontier.authority_tick_floor = 149;
    assert!(original(older, &prior, 150).is_err());
    let mut later = current;
    later.frontier.authority_tick_floor = 170;
    original(later, &prior, 150)?;
    Ok(())
}

#[test]
fn all_manager_allocators_and_sticky_source_loss_survive_parent_recovery() -> Result<()> {
    let floor = ManagerRecoveryFloor {
        tick: 150,
        generation: 97,
        sequence: 200,
        next_event: 30,
        history_count: 31,
        group_next_position: 32,
        group_next_assignment: 33,
        civ_next_position: 34,
        civ_next_assignment: 35,
        fenced: true,
    };
    manager_floor_at_least(floor, floor)?;
    for older in [
        ManagerRecoveryFloor { tick: 149, ..floor },
        ManagerRecoveryFloor {
            generation: 96,
            ..floor
        },
        ManagerRecoveryFloor {
            sequence: 199,
            ..floor
        },
        ManagerRecoveryFloor {
            next_event: 29,
            ..floor
        },
        ManagerRecoveryFloor {
            history_count: 30,
            ..floor
        },
        ManagerRecoveryFloor {
            group_next_position: 31,
            ..floor
        },
        ManagerRecoveryFloor {
            group_next_assignment: 32,
            ..floor
        },
        ManagerRecoveryFloor {
            civ_next_position: 33,
            ..floor
        },
        ManagerRecoveryFloor {
            civ_next_assignment: 34,
            ..floor
        },
        ManagerRecoveryFloor {
            fenced: false,
            ..floor
        },
    ] {
        assert!(manager_floor_at_least(older, floor).is_err());
    }
    manager_floor_at_least(
        ManagerRecoveryFloor {
            tick: 151,
            sequence: 201,
            ..floor
        },
        floor,
    )?;
    Ok(())
}

#[test]
fn workshop_and_input_recovery_cannot_reset_floors_before_planning() -> Result<()> {
    let workshop = WorkshopRecoveryFloor {
        tick: 150,
        generation: 97,
        next_building_id: 200,
        fenced: true,
    };
    workshop_floor_at_least(workshop, workshop)?;
    for older in [
        WorkshopRecoveryFloor {
            tick: 149,
            ..workshop
        },
        WorkshopRecoveryFloor {
            generation: 96,
            ..workshop
        },
        WorkshopRecoveryFloor {
            next_building_id: 199,
            ..workshop
        },
        WorkshopRecoveryFloor {
            fenced: false,
            ..workshop
        },
    ] {
        assert!(workshop_floor_at_least(older, workshop).is_err());
    }
    let input = InputFloor {
        tick: 150,
        generation: 61,
        next_job_id: 31,
        next_building_id: 41,
        next_item_id: 51,
        fenced: true,
    };
    input_floor_at_least(input, input)?;
    for older in [
        InputFloor { tick: 149, ..input },
        InputFloor {
            generation: 60,
            ..input
        },
        InputFloor {
            next_job_id: 30,
            ..input
        },
        InputFloor {
            next_building_id: 40,
            ..input
        },
        InputFloor {
            next_item_id: 50,
            ..input
        },
        InputFloor {
            fenced: false,
            ..input
        },
    ] {
        assert!(input_floor_at_least(older, input).is_err());
    }
    assert!(optional_floor(Some(input), None, input_floor_at_least).is_err());
    assert!(optional_floor(None, Some(input), input_floor_at_least).is_err());
    Ok(())
}

#[test]
fn parent_clock_pin_keeps_both_exact_custody_and_rejected_capture_sequence() -> Result<()> {
    let id = identity("original clock");
    let prior = ClockPin {
        journal_id: id.to_hex(),
        head: identity("acknowledged clock dispatch").to_hex(),
        authority_tick_floor: 150,
        high_source_sequence: 401,
    };
    let config = ClockConfig {
        generation: 71,
        journal_id: Some(id),
    };
    let pin =
        clock_pin(Some(config), Some(&prior))?.ok_or_else(|| invalid("test clock pin absent"))?;
    assert_eq!(pin.journal_id, id);
    assert_eq!(pin.head, digest(&prior.head)?);
    assert_eq!(pin.authority_tick_floor, 150);
    assert_eq!(pin.high_source_sequence, 401);
    assert!(clock_pin(Some(config), None).is_err());
    assert!(clock_pin(None, Some(&prior)).is_err());
    assert!(
        clock_pin(
            Some(ClockConfig {
                journal_id: Some(identity("replacement clock")),
                ..config
            }),
            Some(&prior)
        )
        .is_err()
    );
    let invalid_head = ClockPin {
        head: Digest32::ZERO.to_hex(),
        ..prior
    };
    assert!(clock_pin(Some(config), Some(&invalid_head)).is_err());
    Ok(())
}

#[test]
fn compact_parent_reference_rejects_diagnostics_unknown_fields_and_restored_proof() -> Result<()> {
    let reference = json!({
        "schema": SCHEMA,
        "recovery": null,
        "native_custody": {
            "workforce_journal_id": identity("workforce owner").to_hex(),
            "order_journal_id": identity("order owner").to_hex(),
        },
        "observation_tick_floor": 150,
        "clock": null, "inventory_clock": null,
        "workshop_prerequisite": null, "manager_prerequisite": null, "input_supply": null,
        "original_goal_proven": false,
        "preparation_restored": false,
    });
    retained(&reference)?;
    for field in ["original_goal_proven", "preparation_restored"] {
        let mut changed = reference.clone();
        changed[field] = json!(true);
        assert!(retained(&changed).is_err());
    }
    let mut changed = reference.clone();
    changed["original_plan"] = json!({"predicates":"not part of a compact recovery selector"});
    assert!(retained(&changed).is_err());
    changed = reference.clone();
    changed["native_custody"]["order_head_override"] = json!("unknown");
    assert!(retained(&changed).is_err());
    changed = reference;
    changed["schema"] = json!("x".repeat(MAX_REFERENCE_BYTES));
    assert!(retained(&changed).is_err());
    Ok(())
}

fn publication_fixture() -> Result<(ProjectProductionIntent, Value, Value)> {
    let workshop = json!({"origin_witness":identity("project Workshop").to_hex(),
        "tick":150,"generation":97,"next_building_id":200,"fenced":false});
    let manager = json!({"origin_witness":identity("project Manager").to_hex(),
        "tick":150,"generation":98,"sequence":201,"next_event":30,"history_count":31,
        "group_next_position":32,"group_next_assignment":33,
        "civ_next_position":34,"civ_next_assignment":35,"fenced":false});
    let input = json!({"origin_witness":identity("project compiled input binding").to_hex(),
        "tick":150,"generation":71,"next_job_id":100,"next_building_id":200,
        "next_item_id":300,"fenced":false});
    let floor = |checkpoint: &Value| -> Value {
        let mut checkpoint = checkpoint.clone();
        if let Some(object) = checkpoint.as_object_mut() {
            object.remove("origin_witness");
        }
        checkpoint
    };
    let clock_id = identity("project preopened clock").to_hex();
    let reference = json!({
        "schema":SCHEMA,
        "native_custody":{"workforce_journal_id":identity("project workforce").to_hex(),
            "order_journal_id":identity("project orders").to_hex()},
        "observation_tick_floor":150,
        "clock":{"journal_id":clock_id,"head":identity("clock initial header").to_hex(),
            "authority_tick_floor":150,"high_source_sequence":0},
        "inventory_clock":null,
        "workshop_prerequisite":{"checkpoint":workshop},
        "manager_prerequisite":{"checkpoint":manager},
        "input_supply":{"checkpoint":input},
        "original_goal_proven":false,"preparation_restored":false,
        "recovery":{
            "plan_digest":identity("project exact original plan").to_hex(),
            "object_digest":identity("project exact archive object").to_hex(),
            "identity_digest":identity("project exact citizen capsule").to_hex(),
            "anchor":{"fortress_id":"7","epoch":3,"sequence":5,"tick":100,
                "state_hash":identity("project original state").to_hex()},
            "frontier":{"journal_id":identity("project initial execution header").to_hex(),
                "head":identity("project initial execution header").to_hex(),"authority_tick_floor":150},
            "goal_history_present":false,"clock_journal_id":clock_id,"inventory_clock_journal_id":null,
            "workshop_prerequisite_digest":identity("project exact Workshop association").to_hex(),
            "workshop_prerequisite_floor":floor(&workshop),
            "manager_prerequisite_digest":identity("project exact Manager association").to_hex(),
            "manager_prerequisite_floor":floor(&manager),
            "input_supply_digest":input["origin_witness"],"input_supply_floor":floor(&input),
        },
    });
    let policy = identity("project independent input policy").to_hex();
    let intent = ProjectProductionIntent::new(
        serde_json::from_value(json!({
            "recipe": "wooden_bed",
            "minimum_stack_units": 7,
            "max_order_amount": 100,
            "deadline_tick": 300,
            "poll_interval_ticks": 5,
            "stable_samples": 2,
            "assign_labor": true,
        }))
        .map_err(|_| invalid("test scalar project goal"))?,
        reference.clone(),
        Some(policy.clone()),
    )?;
    let mut preplan = reference.clone();
    preplan["recovery"] = Value::Null;
    preplan["input_supply"]["checkpoint"]["origin_witness"] = json!(policy);
    Ok((intent, preplan, reference))
}

#[test]
fn exact_publication_intent_is_closed_and_never_retains_execution_or_current_proof() -> Result<()> {
    let (intent, preplan, planned) = publication_fixture()?;
    ProjectProductionIntent::from_json(&intent.to_json())?.validate_checkpoint(&preplan)?;
    intent.validate_checkpoint(&planned)?;
    assert_eq!(intent.authority_tick_floor(), 150);
    let raw = intent.to_json();
    let edits: [fn(&mut Value); 11] = [
        |v| v["permit"] = json!(true),
        |v| v["schema"] = json!("dfmcp.project-production-intent/2"),
        |v| v["recovery_reference"]["original_goal_proven"] = json!(true),
        |v| v["recovery_reference"]["preparation_restored"] = json!(true),
        |v| v["recovery_reference"]["recovery"]["goal_history_present"] = json!(true),
        |v| {
            v["recovery_reference"]["recovery"]["frontier"]["head"] =
                json!(identity("advanced frontier").to_hex())
        },
        |v| {
            v["recovery_reference"]["clock"]["journal_id"] =
                json!(identity("replacement clock").to_hex())
        },
        |v| v["recovery_reference"]["workshop_prerequisite"] = Value::Null,
        |v| {
            v["recovery_reference"]["recovery"]["workshop_prerequisite_floor"]["next_building_id"] =
                json!(201)
        },
        |v| v["input_policy_witness"] = Value::Null,
        |v| v["goal"]["deadline_tick"] = json!(100),
    ];
    for edit in edits {
        let mut changed = raw.clone();
        edit(&mut changed);
        assert!(ProjectProductionIntent::from_json(&changed).is_err());
    }
    let mut goal = intent.to_json()["goal"].clone();
    goal["minimum_stack_units"] = json!(8);
    let goal = serde_json::from_value(goal).map_err(|_| invalid("test changed scalar goal"))?;
    assert!(intent.validate_goal(&goal).is_err());
    let mut oversized = raw;
    oversized["schema"] = json!("x".repeat(MAX_REFERENCE_BYTES));
    assert!(ProjectProductionIntent::from_json(&oversized).is_err());
    Ok(())
}

#[test]
fn publication_recovery_joins_rejected_source_floors_without_changing_any_original_pin()
-> Result<()> {
    let (intent, mut prior, expected) = publication_fixture()?;
    prior["observation_tick_floor"] = json!(170);
    for source in [
        "workshop_prerequisite",
        "manager_prerequisite",
        "input_supply",
    ] {
        prior[source]["checkpoint"]["tick"] = json!(170);
        prior[source]["checkpoint"]["fenced"] = json!(true);
    }
    prior["manager_prerequisite"]["checkpoint"]["sequence"] = json!(250);
    prior["input_supply"]["checkpoint"]["next_item_id"] = json!(350);
    prior["clock"]["authority_tick_floor"] = json!(170);
    prior["clock"]["high_source_sequence"] = json!(7);
    intent.validate_checkpoint(&prior)?;
    let recovered = publication_reference(&prior, &intent)?;
    let recovery = reference_recovery(&recovered)?;
    validate_reference_sources(&retained(&recovered)?, recovery)?;
    assert_eq!(
        recovered["recovery"]["frontier"]["head"],
        expected["recovery"]["frontier"]["head"]
    );
    for field in [
        "plan_digest",
        "object_digest",
        "identity_digest",
        "anchor",
        "workshop_prerequisite_digest",
        "manager_prerequisite_digest",
        "input_supply_digest",
    ] {
        assert_eq!(recovered["recovery"][field], expected["recovery"][field]);
    }
    assert_eq!(recovered["observation_tick_floor"], 170);
    assert_eq!(recovery.frontier.authority_tick_floor, 170);
    assert!(
        recovery
            .workshop_prerequisite_floor
            .is_some_and(|floor| floor.fenced && floor.tick == 170)
    );
    assert!(
        recovery
            .manager_prerequisite_floor
            .is_some_and(|floor| floor.fenced && floor.sequence == 250)
    );
    assert!(
        recovery
            .input_supply_floor
            .is_some_and(|floor| floor.fenced && floor.next_item_id == 350)
    );
    assert_eq!(recovered["clock"]["high_source_sequence"], 7);
    assert_eq!(
        recovered["input_supply"]["checkpoint"]["origin_witness"],
        expected["recovery"]["input_supply_digest"]
    );
    assert!(!recovery.goal_history_present);
    intent.validate_initial_frontier(recovery.frontier)?;
    Ok(())
}

#[test]
fn publication_intent_rejects_foreign_custody_sources_originals_and_advanced_adoption() -> Result<()>
{
    let (intent, preplan, planned) = publication_fixture()?;
    for field in ["workforce_journal_id", "order_journal_id"] {
        let mut changed = preplan.clone();
        changed["native_custody"][field] = json!(identity("foreign native owner").to_hex());
        assert!(intent.validate_checkpoint(&changed).is_err());
    }
    for source in [
        "workshop_prerequisite",
        "manager_prerequisite",
        "input_supply",
    ] {
        for reference in [&preplan, &planned] {
            let mut changed = reference.clone();
            changed[source]["checkpoint"]["origin_witness"] =
                json!(identity("foreign source").to_hex());
            assert!(intent.validate_checkpoint(&changed).is_err());
        }
    }
    for field in [
        "plan_digest",
        "object_digest",
        "identity_digest",
        "workshop_prerequisite_digest",
        "manager_prerequisite_digest",
        "input_supply_digest",
    ] {
        let mut changed = planned.clone();
        changed["recovery"][field] = json!(identity("foreign original").to_hex());
        assert!(intent.validate_checkpoint(&changed).is_err());
    }
    let mut progressed = planned.clone();
    progressed["recovery"]["frontier"]["head"] =
        json!(identity("later legitimate execution").to_hex());
    // Metadata continuity allows acknowledged descendants; the actual durable
    // coordinator proves their ancestry. Adoption from the pre-plan slot is
    // stricter and cannot acknowledge this advanced execution as a new plan.
    intent.validate_checkpoint(&progressed)?;
    assert!(
        intent
            .validate_initial_frontier(reference_recovery(&progressed)?.frontier)
            .is_err()
    );
    let mut regressed = planned;
    regressed["manager_prerequisite"]["checkpoint"]["sequence"] = json!(200);
    regressed["recovery"]["manager_prerequisite_floor"]["sequence"] = json!(200);
    assert!(intent.validate_checkpoint(&regressed).is_err());
    Ok(())
}

/// Mechanical selectors exercise custody only. The actual compiler/archive
/// tests separately prove which of the four exact minima need an order.
fn reference_input(reference: &Value) -> Result<RecoveryInput> {
    serde_json::from_value(reference["recovery"].clone())
        .map_err(|_| invalid("test retained original input"))
}

fn portfolio_fixture(
    assign_labor: bool,
    order_count: usize,
) -> Result<(ProjectProductionIntent, Value, Value)> {
    let (scalar, _, mut reference) = publication_fixture()?;
    reference["clock"] = Value::Null;
    reference["recovery"]["clock_journal_id"] = Value::Null;
    reference["recovery"]["portfolio"] = json!(
        (0..order_count)
            .map(|index| json!({
                "step_id": u32::from(assign_labor) + index as u32,
                "goal_history_present": false,
                "clock_journal_id": null,
                "inventory_clock_journal_id": null,
            }))
            .collect::<Vec<_>>()
    );
    let goal = serde_json::from_value(json!({
        "targets": [
            {"recipe": "wooden_bed", "minimum_stack_units": 7, "max_order_amount": 10},
            {"recipe": "wooden_chair", "minimum_stack_units": 8, "max_order_amount": 11},
            {"recipe": "wooden_door", "minimum_stack_units": 9, "max_order_amount": 12},
            {"recipe": "wooden_table", "minimum_stack_units": 10, "max_order_amount": 13}
        ],
        "deadline_tick": 300,
        "poll_interval_ticks": 5,
        "stable_samples": 2,
        "assign_labor": assign_labor,
    }))
    .map_err(|_| invalid("test portfolio project goal"))?;
    let intent =
        ProjectProductionIntent::new(goal, reference.clone(), scalar.input_policy_witness.clone())?;
    let mut preplan = reference.clone();
    preplan["recovery"] = Value::Null;
    preplan["input_supply"]["checkpoint"]["origin_witness"] = json!(scalar.input_policy_witness);
    Ok((intent, preplan, reference))
}

#[test]
fn clockless_portfolio_intent_retains_all_targets_and_every_original_order() -> Result<()> {
    for assign_labor in [false, true] {
        for order_count in 1..=4 {
            let (intent, preplan, planned) = portfolio_fixture(assign_labor, order_count)?;
            let encoded = intent.to_json();
            let parsed = ProjectProductionIntent::from_json(&encoded)?;
            assert_eq!(parsed.to_json(), encoded);
            assert_eq!(encoded["goal"]["targets"].as_array().map(Vec::len), Some(4));
            parsed.validate_goal(&intent.goal)?;
            parsed.validate_checkpoint(&preplan)?;
            parsed.validate_checkpoint(&planned)?;
            let recovered = reference_recovery(&planned)?;
            parsed.validate_pending_checkpoint(&planned)?;
            assert_eq!(
                recovered
                    .portfolio
                    .map(|slots| slots.iter().flatten().count()),
                Some(order_count)
            );
            assert_eq!(
                recovered
                    .portfolio
                    .and_then(|slots| slots[0])
                    .map(|slot| slot.step_id),
                Some(u32::from(assign_labor))
            );
            assert!(recovered.clock_journal_id.is_none());
            assert!(recovered.inventory_clock_journal_id.is_none());
            assert_eq!(publication_reference(&preplan, &parsed)?, planned);
        }
    }
    Ok(())
}

#[test]
fn portfolio_intent_rejects_shape_changes_leaf_history_and_any_clock_owner() -> Result<()> {
    let (intent, _, planned) = portfolio_fixture(true, 4)?;
    let raw = intent.to_json();
    let edits: [fn(&mut Value); 14] = [
        |v| v["recovery_reference"]["recovery"]["portfolio"] = Value::Null,
        |v| v["recovery_reference"]["recovery"]["portfolio"] = json!([]),
        |v| v["recovery_reference"]["recovery"]["portfolio"][0]["step_id"] = json!(0),
        |v| v["recovery_reference"]["recovery"]["portfolio"][1]["step_id"] = json!(1),
        |v| v["recovery_reference"]["recovery"]["portfolio"][3]["step_id"] = json!(5),
        |v| {
            v["recovery_reference"]["recovery"]["portfolio"][3]["goal_history_present"] =
                json!(true)
        },
        |v| v["recovery_reference"]["recovery"]["goal_history_present"] = json!(true),
        |v| {
            v["recovery_reference"]["recovery"]["portfolio"][2]["clock_journal_id"] =
                json!(identity("unretained inactive clock").to_hex())
        },
        |v| {
            v["recovery_reference"]["recovery"]["portfolio"][2]["inventory_clock_journal_id"] =
                json!(identity("unretained inactive inventory clock").to_hex())
        },
        |v| {
            v["recovery_reference"]["recovery"]["clock_journal_id"] =
                json!(identity("unretained selected clock").to_hex())
        },
        |v| {
            v["recovery_reference"]["clock"] = json!({
                "journal_id": identity("added clock").to_hex(),
                "head": identity("added clock").to_hex(),
                "authority_tick_floor": 150, "high_source_sequence": 0,
            })
        },
        |v| {
            v["recovery_reference"]["inventory_clock"] = json!({
                "journal_id": identity("added inventory clock").to_hex(),
                "head": identity("added inventory clock").to_hex(),
                "authority_tick_floor": 150, "high_source_sequence": 0,
            })
        },
        |v| v["recovery_reference"]["recovery"]["portfolio"][1]["permit"] = json!(true),
        |v| v["goal"]["assign_labor"] = json!(false),
    ];
    for (index, edit) in edits.into_iter().enumerate() {
        let mut changed = raw.clone();
        edit(&mut changed);
        assert!(
            ProjectProductionIntent::from_json(&changed).is_err(),
            "{index}"
        );
    }
    // A scalar intent cannot adopt even a correctly formed clockless vector.
    let (scalar, _, _) = publication_fixture()?;
    let mut mixed = scalar.to_json();
    mixed["recovery_reference"] = planned;
    assert!(ProjectProductionIntent::from_json(&mixed).is_err());
    Ok(())
}

#[test]
fn portfolio_recovery_keeps_each_leaf_history_monotone_and_exact_slot_identity() -> Result<()> {
    let (_, _, planned) = portfolio_fixture(false, 4)?;
    let mut reference = planned;
    reference["recovery"]["portfolio"][1]["goal_history_present"] = json!(true);
    reference["recovery"]["portfolio"][3]["goal_history_present"] = json!(true);
    let current = reference_recovery(&reference)?;
    let prior = reference_input(&reference)?;
    original(current, &prior, 150)?;
    let mut progressed = current;
    if let Some(slots) = progressed.portfolio.as_mut()
        && let Some(first) = slots[0].as_mut()
    {
        first.goal_history_present = true;
    }
    progressed.goal_history_present = true;
    progressed.frontier.authority_tick_floor += 1;
    original(progressed, &prior, 150)?;
    for index in [1, 3] {
        let mut changed = current;
        if let Some(slots) = changed.portfolio.as_mut()
            && let Some(slot) = slots[index].as_mut()
        {
            slot.goal_history_present = false;
        }
        assert!(original(changed, &prior, 150).is_err(), "{index}");
    }
    for index in 0..4 {
        let mut missing = current;
        if let Some(slots) = missing.portfolio.as_mut() {
            slots[index] = None;
        }
        assert!(original(missing, &prior, 150).is_err(), "{index}");
        let mut replaced = current;
        if let Some(slots) = replaced.portfolio.as_mut()
            && let Some(slot) = slots[index].as_mut()
        {
            slot.step_id += 1;
        }
        assert!(original(replaced, &prior, 150).is_err(), "{index}");
    }
    let mut discarded = current;
    discarded.portfolio = None;
    assert!(original(discarded, &prior, 150).is_err());
    let mut scalar = reference_input(&reference)?;
    scalar.portfolio = None;
    assert!(original(current, &scalar, 150).is_err());
    let mut contradictory = current;
    contradictory.goal_history_present = true;
    assert!(original(contradictory, &prior, 150).is_err());
    Ok(())
}

#[test]
fn portfolio_recovery_cannot_hide_clock_custody_in_an_inactive_leaf() -> Result<()> {
    let (_, _, planned) = portfolio_fixture(true, 4)?;
    let current = reference_recovery(&planned)?;
    let prior = reference_input(&planned)?;
    for index in 0..4 {
        for inventory in [false, true] {
            let mut changed = current;
            if let Some(slots) = changed.portfolio.as_mut()
                && let Some(slot) = slots[index].as_mut()
            {
                if inventory {
                    slot.inventory_clock_journal_id = Some(identity("hidden inventory clock"));
                } else {
                    slot.clock_journal_id = Some(identity("hidden order clock"));
                }
            }
            assert!(
                original(changed, &prior, 150).is_err(),
                "{index}/{inventory}"
            );
        }
    }
    for clock in ["clock", "inventory_clock"] {
        let mut changed = planned.clone();
        changed[clock] = json!({
            "journal_id": identity("added parent clock").to_hex(),
            "head": identity("added parent clock").to_hex(),
            "authority_tick_floor": 150, "high_source_sequence": 0,
        });
        assert!(reference_recovery(&changed).is_err());
    }
    Ok(())
}

#[test]
fn pending_portfolio_publication_rejects_inactive_history_and_advanced_frontier() -> Result<()> {
    let (intent, _, planned) = portfolio_fixture(false, 4)?;
    let current = reference_recovery(&planned)?;
    intent.validate_pending_checkpoint(&planned)?;
    for index in 0..4 {
        let mut changed = current;
        if let Some(slots) = changed.portfolio.as_mut()
            && let Some(slot) = slots[index].as_mut()
        {
            slot.goal_history_present = true;
        }
        if index == 0 {
            changed.goal_history_present = true;
        }
        assert!(
            intent.validate_pending_recovery(changed).is_err(),
            "{index}"
        );
    }
    let mut advanced = current;
    advanced.frontier.head = identity("native prepare already advanced the frontier");
    assert!(intent.validate_pending_recovery(advanced).is_err());
    let mut published = planned;
    published["recovery"]["frontier"]["head"] = json!(advanced.frontier.head.to_hex());
    published["recovery"]["portfolio"][3]["goal_history_present"] = json!(true);
    // Once acknowledged as published, genuine descendants and per-leaf
    // historical custody are valid; each durable child proves its own ancestry.
    intent.validate_checkpoint(&published)?;
    Ok(())
}

#[test]
fn portfolio_publication_joins_floors_without_dropping_targets_or_inactive_slots() -> Result<()> {
    let (intent, mut preplan, planned) = portfolio_fixture(true, 4)?;
    preplan["observation_tick_floor"] = json!(170);
    preplan["workshop_prerequisite"]["checkpoint"]["tick"] = json!(170);
    preplan["workshop_prerequisite"]["checkpoint"]["next_building_id"] = json!(210);
    preplan["workshop_prerequisite"]["checkpoint"]["fenced"] = json!(true);
    let reference = publication_reference(&preplan, &intent)?;
    assert_eq!(
        reference["recovery"]["portfolio"],
        planned["recovery"]["portfolio"]
    );
    let recovered = reference_recovery(&reference)?;
    validate_reference_sources(&retained(&reference)?, recovered)?;
    intent.validate_pending_recovery(recovered)?;
    assert_eq!(recovered.frontier.authority_tick_floor, 170);
    assert!(recovered.workshop_prerequisite_floor.is_some_and(|floor| {
        floor.tick == 170 && floor.next_building_id == 210 && floor.fenced
    }));
    let mut changed = intent.to_json()["goal"].clone();
    changed["targets"][3]["minimum_stack_units"] = json!(11);
    let changed =
        serde_json::from_value(changed).map_err(|_| invalid("test changed vector goal"))?;
    assert!(intent.validate_goal(&changed).is_err());
    let mut dropped = intent.to_json()["goal"].clone();
    dropped["targets"] = json!([
        {"recipe": "wooden_bed", "minimum_stack_units": 7, "max_order_amount": 10}
    ]);
    let dropped =
        serde_json::from_value(dropped).map_err(|_| invalid("test dropped vector goal"))?;
    assert!(intent.validate_goal(&dropped).is_err());
    Ok(())
}

#[test]
fn scalar_publication_goal_serialization_stays_untagged_and_clock_pinned() -> Result<()> {
    let (intent, _, planned) = publication_fixture()?;
    assert_eq!(
        serde_json::to_string(&intent.goal).map_err(|_| invalid("test scalar goal encoding"))?,
        "{\"recipe\":\"wooden_bed\",\"minimum_stack_units\":7,\"max_order_amount\":100,\"deadline_tick\":300,\"poll_interval_ticks\":5,\"stable_samples\":2,\"assign_labor\":true}"
    );
    intent.validate_pending_checkpoint(&planned)?;
    let mut without_clock = planned;
    without_clock["clock"] = Value::Null;
    without_clock["recovery"]["clock_journal_id"] = Value::Null;
    assert!(intent.validate_checkpoint(&without_clock).is_err());
    Ok(())
}

fn clocked_portfolio_fixture(
    assign_labor: bool,
    count: usize,
    order_clock: bool,
    inventory_clock: bool,
) -> Result<(ProjectProductionIntent, Value, Value)> {
    let (original, _, mut reference) = portfolio_fixture(assign_labor, count)?;
    let slots = (0..count)
        .map(|index| {
            let pin = |family: &str| {
                let id = identity(&format!("original portfolio {index} {family}")).to_hex();
                json!({
                    "journal_id": id, "head": id,
                    "authority_tick_floor": 150, "high_source_sequence": 0,
                })
            };
            json!({
                "step_id": u32::from(assign_labor) + index as u32,
                "clock": order_clock.then(|| pin("order")),
                "inventory_clock": inventory_clock.then(|| pin("inventory")),
            })
        })
        .collect::<Vec<_>>();
    reference["portfolio_clocks"] = json!(slots);
    for index in 0..count {
        for (pin, id) in [
            ("clock", "clock_journal_id"),
            ("inventory_clock", "inventory_clock_journal_id"),
        ] {
            reference["recovery"]["portfolio"][index][id] = slots[index][pin]["journal_id"].clone();
        }
    }
    for (pin, id) in [
        ("clock", "clock_journal_id"),
        ("inventory_clock", "inventory_clock_journal_id"),
    ] {
        reference[pin] = slots[0][pin].clone();
        reference["recovery"][id] = slots[0][pin]["journal_id"].clone();
    }
    let intent = ProjectProductionIntent::new(
        original.goal,
        reference.clone(),
        original.input_policy_witness.clone(),
    )?;
    let mut preplan = reference.clone();
    preplan["recovery"] = Value::Null;
    preplan
        .as_object_mut()
        .ok_or_else(|| invalid("test preplan object"))?
        .remove("portfolio_clocks");
    preplan["input_supply"]["checkpoint"]["origin_witness"] =
        json!(original.input_policy_witness);
    Ok((intent, preplan, reference))
}

fn checked_reference(reference: &Value) -> Result<()> {
    validate_reference_sources(&retained(reference)?, reference_recovery(reference)?)
}

#[test]
fn portfolio_clock_reference_keeps_every_slot_and_each_independent_family() -> Result<()> {
    for assign in [false, true] {
        for count in 1..=4 {
            for (order, inventory) in [(true, false), (false, true), (true, true)] {
                let (intent, preplan, planned) =
                    clocked_portfolio_fixture(assign, count, order, inventory)?;
                checked_reference(&planned)?;
                intent.validate_checkpoint(&preplan)?;
                intent.validate_pending_checkpoint(&planned)?;
                let slots = portfolio_clock_pins(
                    &retained(&planned)?,
                    reference_recovery(&planned)?,
                )?
                .ok_or_else(|| invalid("test complete clock vector absent"))?;
                assert_eq!(slots.iter().flatten().count(), count);
                for (index, slot) in slots.into_iter().flatten().enumerate() {
                    assert_eq!(slot.step_id, u32::from(assign) + index as u32);
                    assert_eq!(slot.clock.is_some(), order);
                    assert_eq!(slot.inventory_clock.is_some(), inventory);
                }
                assert_eq!(publication_reference(&preplan, &intent)?, planned);
            }
        }
    }
    Ok(())
}

#[test]
fn portfolio_clock_reference_rejects_incomplete_or_substituted_inactive_custody() -> Result<()> {
    let (_, _, planned) = clocked_portfolio_fixture(false, 4, true, true)?;
    let edits: [fn(&mut Value); 12] = [
        |v| v["portfolio_clocks"] = Value::Null,
        |v| v["portfolio_clocks"] = json!([]),
        |v| v["portfolio_clocks"] = json!([v["portfolio_clocks"][0].clone()]),
        |v| v["portfolio_clocks"][3]["step_id"] = json!(2),
        |v| v["portfolio_clocks"][2]["clock"] = Value::Null,
        |v| v["portfolio_clocks"][3]["inventory_clock"] = Value::Null,
        |v| v["portfolio_clocks"][2]["clock"]["head"] = json!(Digest32::ZERO.to_hex()),
        |v| v["portfolio_clocks"][3]["clock"]["journal_id"] =
            json!(identity("replacement inactive owner").to_hex()),
        |v| v["portfolio_clocks"][3]["clock"]["authority_tick_floor"] = json!(151),
        |v| v["portfolio_clocks"][1]["inventory_clock"]["high_source_sequence"] = json!(-1),
        |v| v["portfolio_clocks"][2]["permit"] = json!(true),
        |v| v["clock"] = v["portfolio_clocks"][1]["clock"].clone(),
    ];
    for (index, edit) in edits.into_iter().enumerate() {
        let mut changed = planned.clone();
        edit(&mut changed);
        assert!(checked_reference(&changed).is_err(), "{index}");
    }
    // Even internally matching metadata cannot reuse one journal for two
    // original orders or reinterpret it as the independent clock family.
    for family in ["clock", "inventory_clock"] {
        let mut changed = planned.clone();
        changed["portfolio_clocks"][3][family] = changed["portfolio_clocks"][2]["clock"].clone();
        let id_field = if family == "clock" {
            "clock_journal_id"
        } else {
            "inventory_clock_journal_id"
        };
        changed["recovery"]["portfolio"][3][id_field] =
            changed["portfolio_clocks"][2]["clock"]["journal_id"].clone();
        assert!(checked_reference(&changed).is_err());
    }
    let mut scalar = planned;
    scalar["recovery"]["portfolio"] = Value::Null;
    assert!(checked_reference(&scalar).is_err());
    Ok(())
}

#[test]
fn every_portfolio_clock_retains_source_floors_and_pending_publication_rejects_runs()
-> Result<()> {
    let (intent, _, planned) = clocked_portfolio_fixture(true, 4, true, true)?;
    for index in 0..4 {
        for family in ["clock", "inventory_clock"] {
            let mut descendant = planned.clone();
            descendant["observation_tick_floor"] = json!(170);
            descendant["recovery"]["frontier"]["authority_tick_floor"] = json!(170);
            descendant["portfolio_clocks"][index][family]["authority_tick_floor"] = json!(170);
            descendant["portfolio_clocks"][index][family]["high_source_sequence"] = json!(9);
            descendant["portfolio_clocks"][index][family]["head"] =
                json!(identity("authentic later clock head").to_hex());
            if index == 0 {
                descendant[family] = descendant["portfolio_clocks"][0][family].clone();
            }
            intent.validate_checkpoint(&descendant)?;
            assert!(intent.validate_pending_checkpoint(&descendant).is_err());
            let prior = retained(&descendant)?;
            for field in ["authority_tick_floor", "high_source_sequence"] {
                let mut regressed = descendant.clone();
                regressed["portfolio_clocks"][index][family][field] = json!(0);
                if index == 0 {
                    regressed[family] = regressed["portfolio_clocks"][0][family].clone();
                }
                assert!(
                    compare_portfolio_clock_sources(&retained(&regressed)?, &prior, true).is_err(),
                    "{index}/{family}/{field}"
                );
            }
            let mut new_intent = intent.to_json();
            new_intent["recovery_reference"] = descendant;
            assert!(ProjectProductionIntent::from_json(&new_intent).is_err());
        }
    }
    Ok(())
}

#[test]
fn portfolio_publication_joins_preplan_clock_floors_without_replacing_other_slots() -> Result<()> {
    let (intent, mut preplan, planned) = clocked_portfolio_fixture(false, 4, true, true)?;
    preplan["observation_tick_floor"] = json!(170);
    for family in ["clock", "inventory_clock"] {
        preplan[family]["authority_tick_floor"] = json!(170);
        preplan[family]["high_source_sequence"] = json!(11);
    }
    let joined = publication_reference(&preplan, &intent)?;
    checked_reference(&joined)?;
    intent.validate_pending_checkpoint(&joined)?;
    for family in ["clock", "inventory_clock"] {
        assert_eq!(joined["portfolio_clocks"][0][family], joined[family]);
        for index in 1..4 {
            assert_eq!(
                joined["portfolio_clocks"][index][family],
                planned["portfolio_clocks"][index][family]
            );
        }
    }
    preplan["portfolio_clocks"] = planned["portfolio_clocks"].clone();
    assert!(intent.validate_checkpoint(&preplan).is_err());
    Ok(())
}

#[test]
fn clockless_and_scalar_recovery_do_not_gain_an_empty_clock_vector() -> Result<()> {
    for (_, _, mut reference) in [publication_fixture()?, portfolio_fixture(false, 4)?] {
        checked_reference(&reference)?;
        assert!(reference.get("portfolio_clocks").is_none());
        reference["portfolio_clocks"] = json!([]);
        assert!(checked_reference(&reference).is_err());
    }
    Ok(())
}
