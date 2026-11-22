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
        ProjectStockGoal {
            recipe: "wooden_bed".into(),
            minimum_stack_units: 7,
            max_order_amount: 100,
            deadline_tick: 300,
            poll_interval_ticks: 5,
            stable_samples: 2,
            assign_labor: true,
        },
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
    let mut goal = intent.goal.clone();
    goal.minimum_stack_units += 1;
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
