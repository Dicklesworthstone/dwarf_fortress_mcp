use super::*;
use serde_json::{Value, json};

fn input_configuration() -> Value {
    json!({"schema":"dfmcp.combined-production-development/1", "directory":"/tmp/combined-inputs",
        "initialize":true,"input_supply":{"schema":"dfmcp.furniture-input-supply/1"}})
}

fn input_floor() -> Value {
    json!({"tick":130,"generation":27,"next_job_id":80,
        "next_building_id":90,"next_item_id":100,"fenced":false})
}

fn input_recovery() -> Value {
    let hash = Digest32::of_bytes(b"original conserved input custody").to_hex();
    json!({"plan_digest":hash,"object_digest":hash,"identity_digest":hash,
        "anchor":{"fortress_id":"1","epoch":0,"sequence":2,"tick":120,"state_hash":hash},
        "frontier":{"journal_id":hash,"head":hash,"authority_tick_floor":130},
        "input_supply_digest":Digest32::of_bytes(b"original input marker").to_hex(),
        "input_supply_floor":input_floor()})
}

#[test]
fn input_policy_is_opt_in_closed_and_bounded_without_native_selectors() -> Result<()> {
    let raw = input_configuration();
    assert_eq!(
        parse(&raw.to_string())?
            .9
            .map(|c| c.reserve_wood_stack_units),
        Some(0)
    );
    assert!(parse(&raw.to_string())?.10.is_none());
    for reserve in [0, 1, 1_000_000] {
        let mut valid = raw.clone();
        valid["input_supply"]["reserve_wood_stack_units"] = json!(reserve);
        assert_eq!(
            parse(&valid.to_string())?
                .9
                .map(|c| c.reserve_wood_stack_units),
            Some(reserve)
        );
    }
    for value in [Value::Null, json!({}), json!("policy")] {
        let mut bad = raw.clone();
        bad["input_supply"] = value;
        assert!(parse(&bad.to_string()).is_err());
    }
    for (field, value) in [
        ("schema", json!("dfmcp.furniture-input-supply/2")),
        ("reserve_wood_stack_units", json!(-1)),
        ("reserve_wood_stack_units", json!(1_000_001)),
        ("reserve_wood_stack_units", Value::Null),
        ("endpoint", json!("127.0.0.1:5000")),
        ("directory", json!("/tmp/foreign")),
        ("method", json!("ReadItems")),
        ("protocol", json!("operations/1.3")),
        ("generation", json!(27)),
        ("item_ids", json!([1, 2])),
        ("required_stack_units", json!(0)),
    ] {
        let mut bad = raw.clone();
        bad["input_supply"][field] = value;
        assert!(parse(&bad.to_string()).is_err(), "invalid {field}");
    }
    Ok(())
}

#[test]
fn input_recovery_requires_exact_marker_and_every_consumed_allocator_floor() -> Result<()> {
    let mut raw = input_configuration();
    raw["initialize"] = json!(false);
    raw["recovery"] = input_recovery();
    let recovery = parse(&raw.to_string())?
        .2
        .ok_or_else(|| invalid("recovery"))?;
    assert_eq!(
        recovery.input_supply_floor.map(|f| f.counters()),
        Some([80, 90, 100])
    );
    for field in ["input_supply_digest", "input_supply_floor"] {
        let mut bad = raw.clone();
        bad["recovery"]
            .as_object_mut()
            .ok_or_else(|| invalid("fixture"))?
            .remove(field);
        assert!(parse(&bad.to_string()).is_err(), "missing {field}");
        bad["recovery"][field] = Value::Null;
        assert!(parse(&bad.to_string()).is_err(), "null {field}");
    }
    for field in [
        "tick",
        "generation",
        "next_job_id",
        "next_building_id",
        "next_item_id",
        "fenced",
    ] {
        let mut bad = raw.clone();
        bad["recovery"]["input_supply_floor"]
            .as_object_mut()
            .ok_or_else(|| invalid("fixture"))?
            .remove(field);
        assert!(parse(&bad.to_string()).is_err(), "missing {field}");
    }
    for (field, value) in [
        ("tick", json!(131)),
        ("generation", json!(0)),
        ("generation", json!(u64::MAX)),
        ("next_job_id", json!(u32::MAX)),
        ("next_building_id", json!(u32::MAX)),
        ("next_item_id", json!(u32::MAX)),
        ("fenced", json!("false")),
        ("extra", json!(true)),
    ] {
        let mut bad = raw.clone();
        bad["recovery"]["input_supply_floor"][field] = value;
        assert!(parse(&bad.to_string()).is_err(), "invalid {field}");
    }
    raw.as_object_mut()
        .ok_or_else(|| invalid("fixture"))?
        .remove("input_supply");
    assert!(
        parse(&raw.to_string()).is_err(),
        "input policy cannot be stripped from recovery"
    );
    Ok(())
}

#[test]
fn input_preplan_checkpoint_cannot_compete_with_full_original_recovery() -> Result<()> {
    let mut raw = input_configuration();
    let mut checkpoint = input_floor();
    checkpoint["origin_witness"] = json!(Digest32::of_bytes(b"input source policy").to_hex());
    raw["input_supply_checkpoint"] = checkpoint;
    let retained = parse(&raw.to_string())?
        .10
        .ok_or_else(|| invalid("checkpoint"))?;
    assert_eq!(retained.floor.tick, 130);
    assert_eq!(retained.floor.counters(), [80, 90, 100]);
    for value in [Value::Null, json!({}), json!("checkpoint")] {
        let mut bad = raw.clone();
        bad["input_supply_checkpoint"] = value;
        assert!(parse(&bad.to_string()).is_err());
    }
    for field in [
        "origin_witness",
        "tick",
        "generation",
        "next_job_id",
        "next_building_id",
        "next_item_id",
        "fenced",
    ] {
        let mut bad = raw.clone();
        bad["input_supply_checkpoint"]
            .as_object_mut()
            .ok_or_else(|| invalid("fixture"))?
            .remove(field);
        assert!(parse(&bad.to_string()).is_err(), "missing {field}");
    }
    let mut missing = raw.clone();
    missing
        .as_object_mut()
        .ok_or_else(|| invalid("fixture"))?
        .remove("input_supply");
    assert!(parse(&missing.to_string()).is_err());
    raw["initialize"] = json!(false);
    raw["recovery"] = input_recovery();
    assert!(parse(&raw.to_string()).is_err());
    Ok(())
}

#[test]
fn portfolio_recovery_retains_actual_zero_or_one_based_order_ids() -> Result<()> {
    let hash = Digest32::of_bytes(b"retained portfolio").to_hex();
    let raw = json!({"schema":"dfmcp.combined-production-development/1", "directory":"/tmp/combined-portfolio",
        "initialize":false,"recovery":{"plan_digest":hash,"object_digest":hash,"identity_digest":hash,
        "anchor":{"fortress_id":"1","epoch":0,"sequence":2,"tick":3,"state_hash":hash},
        "frontier":{"journal_id":hash,"head":hash,"authority_tick_floor":4},
        "goal_history_present":true}});
    for first in [0, 1] {
        let mut valid = raw.clone();
        valid["recovery"]["portfolio"] = Value::Array((first..first+4).map(|step|
            json!({"step_id":step,"goal_history_present":step==first,"clock_journal_id":null,"inventory_clock_journal_id":null})).collect());
        let entries = parse(&valid.to_string())?
            .2
            .ok_or_else(|| invalid("recovery"))?
            .portfolio
            .ok_or_else(|| invalid("portfolio"))?;
        for (ordinal, entry) in entries.into_iter().enumerate() {
            assert_eq!(
                entry.ok_or_else(|| invalid("entry"))?.step_id,
                first + ordinal as u32
            );
        }
        for ids in [
            [first, first, first + 2, first + 3],
            [first, first + 2, first + 1, first + 3],
            [first, first + 1, first + 2, first + 4],
        ] {
            let mut bad = valid.clone();
            for (index, id) in ids.into_iter().enumerate() {
                bad["recovery"]["portfolio"][index]["step_id"] = id.into();
            }
            assert!(parse(&bad.to_string()).is_err());
        }
        let mut bad = valid.clone();
        bad["recovery"]["portfolio"][0]["goal_history_present"] = false.into();
        assert!(
            parse(&bad.to_string()).is_err(),
            "first ordinal slot must mirror the legacy history path"
        );
    }
    let mut bad = raw;
    bad["recovery"]["portfolio"] = json!([{"step_id":2,"goal_history_present":true}]);
    assert!(parse(&bad.to_string()).is_err());
    Ok(())
}

#[test]
fn closed_configuration_and_recovery_are_exact() -> Result<()> {
    let raw = json!({"schema":"dfmcp.combined-production-development/1", "directory":"/tmp/combined", "initialize":true});
    let (directory, initialize, recovery, clock, inventory_clock, workshop, checkpoint, _, _, _, _) =
        parse(&raw.to_string())?;
    assert_eq!(directory, Path::new("/tmp/combined"));
    assert!(initialize && recovery.is_none());
    assert!(clock.is_none());
    assert!(inventory_clock.is_none());
    assert!(workshop.is_none() && checkpoint.is_none());
    let mut bad = raw.clone();
    bad["extra"] = json!(true);
    assert!(parse(&bad.to_string()).is_err());
    bad = raw.clone();
    bad["directory"] = json!("/tmp/../escape");
    assert!(parse(&bad.to_string()).is_err());
    bad = raw.clone();
    bad["recovery"] = json!({});
    assert!(parse(&bad.to_string()).is_err());

    let hash = Digest32::of_bytes(b"retained").to_hex();
    let mut recovered = raw;
    recovered["initialize"] = json!(false);
    recovered["recovery"] = json!({
        "plan_digest":hash, "object_digest":hash, "identity_digest":hash,
        "anchor":{"fortress_id":"1", "epoch":0, "sequence":2, "tick":3, "state_hash":hash},
        "frontier":{"journal_id":hash, "head":hash, "authority_tick_floor":4}
    });
    let (_, _, value, _, _, _, _, _, _, _, _) = parse(&recovered.to_string())?;
    assert_eq!(
        value
            .ok_or_else(|| invalid("missing test recovery"))?
            .frontier
            .authority_tick_floor,
        4
    );
    assert!(
        !parse(&recovered.to_string())?
            .2
            .ok_or_else(|| invalid("recovery"))?
            .goal_history_present
    );
    recovered["recovery"]["goal_history_present"] = json!(true);
    assert!(
        parse(&recovered.to_string())?
            .2
            .ok_or_else(|| invalid("recovery"))?
            .goal_history_present
    );
    recovered["recovery"]["goal_history_present"] = json!("true");
    assert!(parse(&recovered.to_string()).is_err());
    recovered["recovery"]["goal_history_present"] = json!(true);
    recovered["recovery"]["anchor"]["fortress_id"] = json!("01");
    assert!(parse(&recovered.to_string()).is_err());
    Ok(())
}

#[test]
fn custody_and_environment_scope_refuse_substitution() {
    assert!(distinct_directories([
        Path::new("/tmp/a"),
        Path::new("/tmp/b"),
        Path::new("/tmp/c")
    ]));
    assert!(!distinct_directories([
        Path::new("/tmp/a"),
        Path::new("/tmp/a/nested"),
        Path::new("/tmp/c")
    ]));
    assert!(!distinct_directories([
        Path::new("/tmp/a"),
        Path::new("/tmp/b"),
        Path::new("/tmp/a")
    ]));
    assert!(allowed_key(
        "DFMCP_SEMANTIC_WORKFORCE_ALLOW_LABOR",
        false,
        false,
        false,
        false
    ));
    assert!(allowed_key(
        "DFMCP_SEMANTIC_WORK_ORDER_ALLOW_PRODUCTION",
        false,
        false,
        false,
        false
    ));
    for clock in [false, true] {
        for inventory_clock in [false, true] {
            for key in [
                "DFMCP_SEMANTIC_WORK_ORDER_ALLOW_CLOCK",
                "DFMCP_SEMANTIC_WORK_ORDER_CLOCK_TOKEN",
            ] {
                assert_eq!(
                    allowed_key(key, clock, inventory_clock, false, false),
                    clock,
                    "{key}"
                );
            }
            for key in [
                "DFMCP_SEMANTIC_WORK_ORDER_ALLOW_INVENTORY_CLOCK",
                "DFMCP_SEMANTIC_WORK_ORDER_INVENTORY_CLOCK_TOKEN",
            ] {
                assert_eq!(
                    allowed_key(key, clock, inventory_clock, false, false),
                    inventory_clock,
                    "{key}"
                );
            }
            for key in [
                "DFMCP_ADMITTED_BRIDGE_PROTOCOL",
                "DFMCP_RUN_TOKEN",
                "DFMCP_UNKNOWN",
            ] {
                assert!(
                    !allowed_key(key, clock, inventory_clock, false, false),
                    "{key}"
                );
            }
        }
    }
    let original = [
        "parent".to_string(),
        "workforce".to_string(),
        "orders".to_string(),
    ];
    let pins = original
        .each_ref()
        .map(|v| Digest32::of_bytes(v.as_bytes()));
    assert!(matches_digests(&original, pins));
    for index in 0..3 {
        let mut changed = original.clone();
        changed[index].push('!');
        assert!(!matches_digests(&changed, pins));
    }
}

#[test]
fn optional_clock_pins_and_parent_recovery_cannot_be_omitted_or_replaced() -> Result<()> {
    let mut raw = json!({"schema":"dfmcp.combined-production-development/1",
        "directory":"/tmp/combined-clock", "initialize":true,
        "clock":{"generation":50}});
    let configured = parse(&raw.to_string())?.3.ok_or_else(|| invalid("clock"))?;
    assert_eq!(configured.generation, 50);
    assert_eq!(configured.journal_id, None);
    for generation in [0, u64::MAX] {
        let mut bad = raw.clone();
        bad["clock"]["generation"] = json!(generation);
        assert!(parse(&bad.to_string()).is_err());
    }
    for extra in ["order_id", "endpoint", "token", "target_digest"] {
        let mut bad = raw.clone();
        bad["clock"][extra] = json!("caller-selected");
        assert!(parse(&bad.to_string()).is_err());
    }
    let identity = Digest32::of_bytes(b"retained independent clock");
    raw["clock"]["journal_id"] = json!(identity.to_hex());
    assert!(parse(&raw.to_string()).is_err());
    raw["initialize"] = json!(false);
    assert_eq!(
        parse(&raw.to_string())?
            .3
            .ok_or_else(|| invalid("clock"))?
            .journal_id,
        Some(identity)
    );
    let hash = Digest32::of_bytes(b"retained original").to_hex();
    raw["recovery"] = json!({
        "plan_digest":hash, "object_digest":hash, "identity_digest":hash,
        "anchor":{"fortress_id":"1", "epoch":0, "sequence":2, "tick":3, "state_hash":hash},
        "frontier":{"journal_id":hash, "head":hash, "authority_tick_floor":4}
    });
    assert!(
        parse(&raw.to_string()).is_err(),
        "clock recovery cannot be omitted"
    );
    raw["recovery"]["clock_journal_id"] = json!(identity.to_hex());
    assert_eq!(
        parse(&raw.to_string())?
            .2
            .ok_or_else(|| invalid("recovery"))?
            .clock_journal_id,
        Some(identity)
    );
    let mut bad = raw.clone();
    bad["recovery"]["clock_journal_id"] = json!(hash);
    assert!(parse(&bad.to_string()).is_err());
    bad = raw.clone();
    bad["clock"] = Value::Null;
    assert!(parse(&bad.to_string()).is_err());
    for journal in [Value::Null, json!("0".repeat(64)), json!("ab"), json!(5)] {
        let mut bad = raw.clone();
        bad["clock"]["journal_id"] = journal;
        assert!(parse(&bad.to_string()).is_err());
    }
    raw["clock"] = json!({"generation":50});
    assert!(
        parse(&raw.to_string()).is_err(),
        "reopening requires external clock identity"
    );
    Ok(())
}

#[test]
fn independent_inventory_clock_and_both_recovery_journals_remain_exact() -> Result<()> {
    let mut raw = json!({"schema":"dfmcp.combined-production-development/1",
        "directory":"/tmp/combined-inventory-clock", "initialize":true,
        "inventory_clock":{"generation":60}});
    let (_, _, _, order_clock, inventory_clock, _, _, _, _, _, _) = parse(&raw.to_string())?;
    assert!(order_clock.is_none());
    let configured = inventory_clock.ok_or_else(|| invalid("inventory clock"))?;
    assert_eq!(configured.generation, 60);
    assert_eq!(configured.journal_id, None);
    for invalid_clock in [
        json!({"generation":0}),
        json!({"generation":u64::MAX}),
        json!({"generation":60,"order_id":100}),
        json!({"generation":60,"journal_id":null}),
        json!({"generation":60,"journal_id":"ab".repeat(32)}),
    ] {
        let mut bad = raw.clone();
        bad["inventory_clock"] = invalid_clock;
        assert!(parse(&bad.to_string()).is_err());
    }
    let inventory_id = Digest32::of_bytes(b"independent inventory clock");
    let order_id = Digest32::of_bytes(b"independent order clock");
    raw["initialize"] = json!(false);
    assert!(
        parse(&raw.to_string()).is_err(),
        "reopening requires inventory identity"
    );
    raw["inventory_clock"]["journal_id"] = json!(inventory_id.to_hex());
    assert_eq!(
        parse(&raw.to_string())?
            .4
            .ok_or_else(|| invalid("inventory clock"))?
            .journal_id,
        Some(inventory_id)
    );
    let hash = Digest32::of_bytes(b"retained combined original").to_hex();
    raw["recovery"] = json!({
        "plan_digest":hash, "object_digest":hash, "identity_digest":hash,
        "anchor":{"fortress_id":"1", "epoch":0, "sequence":2, "tick":3, "state_hash":hash},
        "frontier":{"journal_id":hash, "head":hash, "authority_tick_floor":4}
    });
    assert!(
        parse(&raw.to_string()).is_err(),
        "inventory recovery cannot be omitted"
    );
    raw["recovery"]["inventory_clock_journal_id"] = json!(inventory_id.to_hex());
    let recovered = parse(&raw.to_string())?
        .2
        .ok_or_else(|| invalid("recovery"))?;
    assert_eq!(recovered.inventory_clock_journal_id, Some(inventory_id));
    assert_eq!(recovered.clock_journal_id, None);

    raw["clock"] = json!({"generation":50,"journal_id":order_id.to_hex()});
    assert!(
        parse(&raw.to_string()).is_err(),
        "order recovery remains independent"
    );
    raw["recovery"]["clock_journal_id"] = json!(order_id.to_hex());
    let (_, _, recovery, clock, inventory_clock, _, _, _, _, _, _) = parse(&raw.to_string())?;
    let recovered = recovery.ok_or_else(|| invalid("recovery"))?;
    assert_eq!(recovered.clock_journal_id, Some(order_id));
    assert_eq!(recovered.inventory_clock_journal_id, Some(inventory_id));
    assert_eq!(clock.ok_or_else(|| invalid("clock"))?.generation, 50);
    assert_eq!(
        inventory_clock
            .ok_or_else(|| invalid("inventory clock"))?
            .generation,
        60
    );
    for (configuration, recovery_field, other_id) in [
        ("clock", "clock_journal_id", inventory_id),
        ("inventory_clock", "inventory_clock_journal_id", order_id),
    ] {
        let mut bad = raw.clone();
        bad[configuration] = Value::Null;
        assert!(parse(&bad.to_string()).is_err(), "omitted {configuration}");
        for identity in [
            Value::Null,
            json!(other_id.to_hex()),
            json!("0".repeat(64)),
            json!(5),
        ] {
            let mut bad = raw.clone();
            bad["recovery"][recovery_field] = identity;
            assert!(
                parse(&bad.to_string()).is_err(),
                "substituted {recovery_field}"
            );
        }
        let mut bad = raw.clone();
        bad[configuration] = json!({"generation":60});
        assert!(
            parse(&bad.to_string()).is_err(),
            "missing journal for {configuration}"
        );
    }
    Ok(())
}

#[test]
fn citizen_identity_pins_must_match_across_families() -> Result<()> {
    let mut common = json!({"kind":"operator-development-source-acceptance/1", "world_folder":"region1", "site_id":7,
        "df_version":"df", "dfhack_version":"hack", "citizen_bridge_version":"bridge", "citizen_generation":10});
    common["workforce_generation"] = json!(20);
    let w: workforce::policy::SourceAcceptance =
        serde_json::from_value(common.clone()).map_err(|_| invalid("workforce test fixture"))?;
    common
        .as_object_mut()
        .ok_or_else(|| invalid("test object"))?
        .remove("workforce_generation");
    common["work_order_generation"] = json!(21);
    common["progress_generation"] = json!(22);
    common["operations_generation"] = json!(23);
    let o: orders::policy::SourceAcceptance =
        serde_json::from_value(common.clone()).map_err(|_| invalid("order test fixture"))?;
    assert!(matching_sources(&w, &o));
    for (field, value) in [
        ("world_folder", json!("other")),
        ("site_id", json!(8)),
        ("df_version", json!("other")),
        ("dfhack_version", json!("other")),
        ("citizen_bridge_version", json!("other")),
        ("citizen_generation", json!(11)),
    ] {
        let mut changed = common.clone();
        changed[field] = value;
        let o: orders::policy::SourceAcceptance =
            serde_json::from_value(changed).map_err(|_| invalid("order test fixture"))?;
        assert!(!matching_sources(&w, &o), "changed {field}");
    }
    Ok(())
}

fn workshop_configuration() -> Value {
    json!({
        "schema":"dfmcp.combined-production-development/1",
        "directory":"/tmp/combined-workshop",
        "initialize":true,
        "workshop_prerequisite": {
            "directory":"/tmp/completed-workshop",
            "endpoint":"127.0.0.1:5001",
            "world_folder":"region1",
            "site_id":7,
            "df_version":"df",
            "dfhack_version":"hack",
            "generation":30,
            "map_dimensions":[64,64,4],
            "acknowledged": {
                "journal_id":Digest32::of_bytes(b"workshop donor journal").to_hex(),
                "head":Digest32::of_bytes(b"workshop donor head").to_hex(),
                "frames":7
            },
            "original_key":"original-carpenters.1",
            "original_plan_digest":Digest32::of_bytes(b"original workshop plan").to_hex()
        }
    })
}

#[test]
fn optional_workshop_retains_exact_independent_source_and_original_pins() -> Result<()> {
    let raw = workshop_configuration();
    let (_, initialize, recovery, clock, inventory_clock, workshop, checkpoint, _, _, _, _) =
        parse(&raw.to_string())?;
    assert!(initialize && recovery.is_none() && clock.is_none() && inventory_clock.is_none());
    assert!(checkpoint.is_none());
    let workshop = workshop.ok_or_else(|| invalid("workshop config"))?;
    assert_eq!(workshop.directory, Path::new("/tmp/completed-workshop"));
    assert_eq!(workshop.binding.endpoint().to_string(), "127.0.0.1:5001");
    assert_eq!(workshop.binding.fortress().folder(), "region1");
    assert_eq!(workshop.binding.fortress().site(), 7);
    assert_eq!(workshop.binding.generation(), 30);
    assert_eq!(workshop.binding.dimensions(), [64, 64, 4]);
    assert_eq!(workshop.binding.df_version(), "df");
    assert_eq!(workshop.binding.dfhack_version(), "hack");
    assert_eq!(
        workshop.acknowledged.journal_id,
        Digest32::of_bytes(b"workshop donor journal")
    );
    assert_eq!(
        workshop.acknowledged.head,
        Digest32::of_bytes(b"workshop donor head")
    );
    assert_eq!(workshop.acknowledged.frames, 7);
    assert_eq!(workshop.original_key, "original-carpenters.1");
    assert_eq!(
        workshop.original_plan_digest,
        Digest32::of_bytes(b"original workshop plan")
    );
    let mut reopen_unplanned = raw;
    reopen_unplanned["initialize"] = json!(false);
    assert!(parse(&reopen_unplanned.to_string())?.5.is_some());
    Ok(())
}

#[test]
fn workshop_donor_requires_separate_normalized_custody() {
    let raw = workshop_configuration();
    for directory in [
        "relative",
        "/",
        "/tmp//workshop",
        "/tmp/./workshop",
        "/tmp/../workshop",
        "/tmp/workshop/",
        "/tmp/workshop\n",
        "/tmp/combined-workshop",
        "/tmp/combined-workshop/child",
        "/tmp",
    ] {
        let mut bad = raw.clone();
        bad["workshop_prerequisite"]["directory"] = json!(directory);
        assert!(parse(&bad.to_string()).is_err(), "{directory:?}");
    }
    for donor in [
        "/tmp/workforce",
        "/tmp/orders",
        "/tmp/orders/nested",
        "/tmp",
    ] {
        assert!(!distinct_directories([
            Path::new("/tmp/combined-workshop"),
            Path::new("/tmp/workforce"),
            Path::new("/tmp/orders"),
            Path::new(donor),
        ]));
    }
    assert!(distinct_directories([
        Path::new("/tmp/combined-workshop"),
        Path::new("/tmp/workforce"),
        Path::new("/tmp/orders"),
        Path::new("/tmp/completed-workshop"),
    ]));
}

#[test]
fn workshop_original_and_acknowledgement_are_required_closed_and_bounded() -> Result<()> {
    let raw = workshop_configuration();
    for field in [
        "directory",
        "endpoint",
        "world_folder",
        "site_id",
        "df_version",
        "dfhack_version",
        "generation",
        "map_dimensions",
        "acknowledged",
        "original_key",
        "original_plan_digest",
    ] {
        let mut bad = raw.clone();
        bad["workshop_prerequisite"]
            .as_object_mut()
            .ok_or_else(|| invalid("donor object"))?
            .remove(field);
        assert!(parse(&bad.to_string()).is_err(), "missing {field}");
    }
    for extra in [
        "initialize",
        "recovery_generation",
        "allow_build",
        "item_id",
        "building_id",
        "token",
    ] {
        let mut bad = raw.clone();
        bad["workshop_prerequisite"][extra] = json!(true);
        assert!(parse(&bad.to_string()).is_err(), "extra {extra}");
    }
    for key in ["", "../other", "space here", "é", &"x".repeat(129)] {
        let mut bad = raw.clone();
        bad["workshop_prerequisite"]["original_key"] = json!(key);
        assert!(parse(&bad.to_string()).is_err(), "invalid original key");
    }
    for frames in [
        0,
        dfmcp_adapter::workshop_construction::journal::MAX_FRAMES + 1,
    ] {
        let mut bad = raw.clone();
        bad["workshop_prerequisite"]["acknowledged"]["frames"] = json!(frames);
        assert!(parse(&bad.to_string()).is_err());
    }
    for field in ["journal_id", "head"] {
        for value in [
            Value::Null,
            json!("0".repeat(64)),
            json!("AB".repeat(32)),
            json!("ab"),
            json!(5),
        ] {
            let mut bad = raw.clone();
            bad["workshop_prerequisite"]["acknowledged"][field] = value;
            assert!(parse(&bad.to_string()).is_err(), "invalid {field}");
        }
    }
    for value in [
        Value::Null,
        json!("0".repeat(64)),
        json!("AB".repeat(32)),
        json!("ab"),
        json!(5),
    ] {
        let mut bad = raw.clone();
        bad["workshop_prerequisite"]["original_plan_digest"] = value;
        assert!(parse(&bad.to_string()).is_err());
    }
    let mut bad = raw;
    bad["workshop_prerequisite"]["acknowledged"]["completed"] = json!(true);
    assert!(parse(&bad.to_string()).is_err());
    Ok(())
}

#[test]
fn workshop_source_pins_reject_unbounded_or_noncanonical_values() {
    let raw = workshop_configuration();
    for (field, value) in [
        ("endpoint", json!("localhost:5001")),
        ("endpoint", json!("127.0.0.1:0")),
        ("endpoint", json!("127.0.0.1:05001")),
        ("endpoint", json!("192.0.2.1:5001")),
        ("endpoint", json!("[::1]:5001")),
        ("world_folder", json!("")),
        ("world_folder", json!("region\n1")),
        ("world_folder", json!("x".repeat(513))),
        ("site_id", json!(u32::MAX)),
        ("df_version", json!("")),
        ("df_version", json!("x".repeat(129))),
        ("dfhack_version", json!("hack\n")),
        ("generation", json!(0)),
        ("generation", json!(u64::MAX)),
        ("map_dimensions", json!([0, 64, 4])),
        ("map_dimensions", json!([64, 32769, 4])),
        ("map_dimensions", json!([64, 64])),
    ] {
        let mut bad = raw.clone();
        bad["workshop_prerequisite"][field] = value;
        assert!(parse(&bad.to_string()).is_err(), "invalid {field}");
    }
}

#[test]
fn workshop_recovery_requires_its_external_association_and_configuration() -> Result<()> {
    let mut raw = workshop_configuration();
    let hash = Digest32::of_bytes(b"retained original production").to_hex();
    let association = Digest32::of_bytes(b"retained workshop prerequisite association");
    raw["initialize"] = json!(false);
    raw["recovery"] = json!({
        "plan_digest":hash,"object_digest":hash,"identity_digest":hash,
        "anchor":{"fortress_id":"1","epoch":0,"sequence":2,"tick":3,"state_hash":hash},
        "frontier":{"journal_id":hash,"head":hash,"authority_tick_floor":4}
    });
    assert!(
        parse(&raw.to_string()).is_err(),
        "configured original cannot omit its association"
    );
    raw["recovery"]["workshop_prerequisite_digest"] = json!(association.to_hex());
    assert!(
        parse(&raw.to_string()).is_err(),
        "external consumed floors cannot be omitted"
    );
    raw["recovery"]["workshop_prerequisite_floor"] = json!({
        "tick":4,"generation":30,"next_building_id":21,"fenced":false
    });
    assert_eq!(
        parse(&raw.to_string())?
            .2
            .ok_or_else(|| invalid("recovery"))?
            .workshop_prerequisite_digest,
        Some(association)
    );
    let mut bad = raw.clone();
    bad["workshop_prerequisite"] = Value::Null;
    assert!(
        parse(&bad.to_string()).is_err(),
        "cannot hide configured donor on recovery"
    );
    bad = raw.clone();
    bad["initialize"] = json!(true);
    assert!(
        parse(&bad.to_string()).is_err(),
        "initialization cannot replace recovery"
    );
    for value in [
        Value::Null,
        json!("0".repeat(64)),
        json!("AB".repeat(32)),
        json!("ab"),
        json!(5),
    ] {
        let mut bad = raw.clone();
        bad["recovery"]["workshop_prerequisite_digest"] = value;
        assert!(
            parse(&bad.to_string()).is_err(),
            "invalid external association"
        );
    }
    // The complete raw configuration, including each donor pin and the external
    // association, is also retained by the unchanged current-environment guard.
    let original = [raw.to_string(), "workforce".into(), "orders".into()];
    let pins = original
        .each_ref()
        .map(|value| Digest32::of_bytes(value.as_bytes()));
    for (field, value) in [
        ("original_key", json!("replacement-carpenters")),
        ("original_plan_digest", json!(hash)),
        ("generation", json!(31)),
    ] {
        let mut changed = raw.clone();
        changed["workshop_prerequisite"][field] = value;
        let current = [
            changed.to_string(),
            original[1].clone(),
            original[2].clone(),
        ];
        assert!(!matches_digests(&current, pins), "changed {field}");
    }
    raw["recovery"]["workshop_prerequisite_digest"] = json!(hash);
    assert!(!matches_digests(
        &[raw.to_string(), original[1].clone(), original[2].clone()],
        pins
    ));
    Ok(())
}

#[test]
fn workshop_recovery_retains_consumed_floors_and_sticky_generation_loss() -> Result<()> {
    let mut raw = workshop_configuration();
    let hash = Digest32::of_bytes(b"retained workshop parent").to_hex();
    raw["initialize"] = json!(false);
    raw["recovery"] = json!({
        "plan_digest":hash,"object_digest":hash,"identity_digest":hash,
        "anchor":{"fortress_id":"1","epoch":0,"sequence":2,"tick":100,"state_hash":hash},
        "frontier":{"journal_id":hash,"head":hash,"authority_tick_floor":120},
        "workshop_prerequisite_digest":hash,
        "workshop_prerequisite_floor":{"tick":120,"generation":30,"next_building_id":21,"fenced":false}
    });
    let recovery = parse(&raw.to_string())?
        .2
        .ok_or_else(|| invalid("recovery"))?;
    assert_eq!(
        recovery.workshop_prerequisite_floor,
        Some(WorkshopRecoveryFloor {
            tick: 120,
            generation: 30,
            next_building_id: 21,
            fenced: false
        })
    );
    for (field, value) in [
        ("tick", json!(121)),
        (
            "tick",
            json!(dfmcp_adapter::bounded_run::MAX_NATIVE_TICK + 1),
        ),
        ("generation", json!(0)),
        ("generation", json!(29)),
        ("generation", json!(31)),
        ("generation", json!(u64::MAX)),
        ("next_building_id", json!(u32::MAX)),
        ("fenced", json!("false")),
        ("unknown", json!(true)),
    ] {
        let mut bad = raw.clone();
        bad["recovery"]["workshop_prerequisite_floor"][field] = value;
        assert!(parse(&bad.to_string()).is_err(), "invalid {field}");
    }
    for field in ["tick", "generation", "next_building_id", "fenced"] {
        let mut bad = raw.clone();
        bad["recovery"]["workshop_prerequisite_floor"]
            .as_object_mut()
            .ok_or_else(|| invalid("floor object"))?
            .remove(field);
        assert!(parse(&bad.to_string()).is_err(), "missing {field}");
    }
    let mut bad = raw.clone();
    bad["recovery"]["workshop_prerequisite_floor"] = Value::Null;
    assert!(parse(&bad.to_string()).is_err());
    bad = raw.clone();
    bad["recovery"]["workshop_prerequisite_digest"] = Value::Null;
    assert!(parse(&bad.to_string()).is_err());
    bad = raw.clone();
    bad["workshop_prerequisite"] = Value::Null;
    assert!(parse(&bad.to_string()).is_err());
    raw["recovery"]["workshop_prerequisite_floor"]["generation"] = json!(31);
    raw["recovery"]["workshop_prerequisite_floor"]["fenced"] = json!(true);
    assert!(
        parse(&raw.to_string())?
            .2
            .ok_or_else(|| invalid("fenced recovery"))?
            .workshop_prerequisite_floor
            .ok_or_else(|| invalid("fenced floor"))?
            .fenced
    );
    raw["recovery"]["workshop_prerequisite_floor"]["generation"] = json!(29);
    assert!(
        parse(&raw.to_string()).is_err(),
        "a fence cannot excuse generation rewind"
    );
    Ok(())
}

#[test]
fn workshop_preplan_checkpoint_keeps_exact_donor_and_cannot_replace_original_recovery() -> Result<()>
{
    let mut raw = workshop_configuration();
    let witness = Digest32::of_bytes(b"exact imported workshop origin");
    let checkpoint = json!({
        "origin_witness":witness.to_hex(),
        "tick":120,"generation":30,"next_building_id":21,"fenced":false
    });
    raw["workshop_prerequisite_checkpoint"] = checkpoint.clone();
    for initialize in [true, false] {
        raw["initialize"] = json!(initialize);
        assert_eq!(
            parse(&raw.to_string())?.6,
            Some(WorkshopPrerequisiteCheckpoint {
                origin_witness: witness,
                floor: WorkshopRecoveryFloor {
                    tick: 120,
                    generation: 30,
                    next_building_id: 21,
                    fenced: false,
                },
            })
        );
    }
    let mut missing = raw.clone();
    missing
        .as_object_mut()
        .ok_or_else(|| invalid("config object"))?
        .remove("workshop_prerequisite");
    assert!(
        parse(&missing.to_string()).is_err(),
        "checkpoint requires its configured donor"
    );

    let retained = Digest32::of_bytes(b"original production recovery").to_hex();
    raw["recovery"] = json!({
        "plan_digest":retained,"object_digest":retained,"identity_digest":retained,
        "anchor":{"fortress_id":"1","epoch":0,"sequence":2,"tick":100,"state_hash":retained},
        "frontier":{"journal_id":retained,"head":retained,"authority_tick_floor":120},
        "workshop_prerequisite_digest":retained,
        "workshop_prerequisite_floor":{"tick":120,"generation":30,"next_building_id":21,"fenced":false}
    });
    assert!(
        parse(&raw.to_string()).is_err(),
        "pre-plan checkpoint cannot compete with complete recovery"
    );
    raw.as_object_mut()
        .ok_or_else(|| invalid("config object"))?
        .remove("workshop_prerequisite_checkpoint");
    assert!(
        parse(&raw.to_string())?.2.is_some(),
        "complete original recovery remains valid without checkpoint"
    );

    // The existing exact environment guard also pins an accepted checkpoint.
    raw.as_object_mut()
        .ok_or_else(|| invalid("config object"))?
        .remove("recovery");
    raw["workshop_prerequisite_checkpoint"] = checkpoint;
    let original = [raw.to_string(), "workforce".into(), "orders".into()];
    let pins = original
        .each_ref()
        .map(|value| Digest32::of_bytes(value.as_bytes()));
    raw["workshop_prerequisite_checkpoint"]["tick"] = json!(121);
    assert!(!matches_digests(
        &[raw.to_string(), original[1].clone(), original[2].clone()],
        pins
    ));
    Ok(())
}

#[test]
fn workshop_preplan_checkpoint_rejects_unbounded_or_incomplete_custody() -> Result<()> {
    let mut raw = workshop_configuration();
    raw["workshop_prerequisite_checkpoint"] = json!({
        "origin_witness":Digest32::of_bytes(b"original workshop witness").to_hex(),
        "tick":120,"generation":30,"next_building_id":21,"fenced":false
    });
    for (field, value) in [
        ("origin_witness", json!("0".repeat(64))),
        ("origin_witness", json!("AB".repeat(32))),
        ("origin_witness", json!("ab")),
        ("tick", json!(-1)),
        (
            "tick",
            json!(dfmcp_adapter::bounded_run::MAX_NATIVE_TICK + 1),
        ),
        ("generation", json!(0)),
        ("generation", json!(29)),
        ("generation", json!(31)),
        ("generation", json!(u64::MAX)),
        ("next_building_id", json!(u32::MAX)),
        ("fenced", json!("false")),
        ("floor", json!({"tick":120})),
    ] {
        let mut bad = raw.clone();
        bad["workshop_prerequisite_checkpoint"][field] = value;
        assert!(
            parse(&bad.to_string()).is_err(),
            "invalid checkpoint {field}"
        );
    }
    for field in [
        "origin_witness",
        "tick",
        "generation",
        "next_building_id",
        "fenced",
    ] {
        let mut bad = raw.clone();
        bad["workshop_prerequisite_checkpoint"]
            .as_object_mut()
            .ok_or_else(|| invalid("checkpoint object"))?
            .remove(field);
        assert!(
            parse(&bad.to_string()).is_err(),
            "missing checkpoint {field}"
        );
    }
    let mut bad = raw.clone();
    bad["workshop_prerequisite_checkpoint"] = Value::Null;
    assert!(
        parse(&bad.to_string()).is_err(),
        "explicit null cannot hide a retained checkpoint"
    );
    raw["workshop_prerequisite_checkpoint"]["generation"] = json!(31);
    raw["workshop_prerequisite_checkpoint"]["fenced"] = json!(true);
    assert!(
        parse(&raw.to_string())?
            .6
            .ok_or_else(|| invalid("fenced checkpoint"))?
            .floor
            .fenced
    );
    raw["workshop_prerequisite_checkpoint"]["generation"] = json!(29);
    assert!(
        parse(&raw.to_string()).is_err(),
        "sticky loss cannot excuse generation rewind"
    );
    Ok(())
}

#[test]
fn workshop_environment_permits_only_configured_read_credential() {
    for clock in [false, true] {
        for inventory_clock in [false, true] {
            for workshop in [false, true] {
                assert_eq!(
                    allowed_key(
                        workshop_config::TOKEN,
                        clock,
                        inventory_clock,
                        workshop,
                        false
                    ),
                    workshop
                );
                for key in [
                    "DFMCP_WORKSHOP_TOKEN",
                    "DFMCP_WORKSHOP_ALLOW_BUILD",
                    "DFMCP_WORKSHOP_MCP_CONFIG",
                    "DFMCP_ALLOW_UNADMITTED_WORKSHOP_V1_20",
                    "DFMCP_COMBINED_PRODUCTION_WORKSHOP_ALLOW_BUILD",
                    "DFMCP_TREE_HARVEST_TOKEN",
                    "DFMCP_ADMITTED_BRIDGE_PROTOCOL",
                ] {
                    assert!(
                        !allowed_key(key, clock, inventory_clock, workshop, false),
                        "{key}"
                    );
                }
            }
        }
    }
    for length in [32, 256] {
        assert!(WorkshopConfig::validate_token(&"x".repeat(length)).is_ok());
    }
    for invalid in [
        String::new(),
        "x".repeat(31),
        "x".repeat(257),
        format!("{}\0", "x".repeat(32)),
    ] {
        assert!(WorkshopConfig::validate_token(&invalid).is_err());
    }
}

#[test]
fn workshop_joins_common_fortress_pins_without_equating_native_generations() -> Result<()> {
    let workshop = parse(&workshop_configuration().to_string())?
        .5
        .ok_or_else(|| invalid("workshop"))?;
    let common = json!({"kind":"operator-development-source-acceptance/1","world_folder":"region1","site_id":7,
        "df_version":"df","dfhack_version":"hack","citizen_bridge_version":"bridge","citizen_generation":10});
    let mut workforce_value = common.clone();
    workforce_value["workforce_generation"] = json!(20);
    let mut orders_value = common;
    orders_value["work_order_generation"] = json!(21);
    orders_value["progress_generation"] = json!(22);
    orders_value["operations_generation"] = json!(23);
    let workforce: workforce::policy::SourceAcceptance =
        serde_json::from_value(workforce_value.clone())
            .map_err(|_| invalid("workforce fixture"))?;
    let orders: orders::policy::SourceAcceptance =
        serde_json::from_value(orders_value.clone()).map_err(|_| invalid("orders fixture"))?;
    assert!(workshop.matching_sources(&workforce, &orders));
    for (field, value) in [
        ("world_folder", json!("other")),
        ("site_id", json!(8)),
        ("df_version", json!("other")),
        ("dfhack_version", json!("other")),
    ] {
        let mut changed = workforce_value.clone();
        changed[field] = value.clone();
        let changed: workforce::policy::SourceAcceptance =
            serde_json::from_value(changed).map_err(|_| invalid("changed workforce fixture"))?;
        assert!(
            !workshop.matching_sources(&changed, &orders),
            "workforce {field}"
        );
        let mut changed = orders_value.clone();
        changed[field] = value;
        let changed: orders::policy::SourceAcceptance =
            serde_json::from_value(changed).map_err(|_| invalid("changed orders fixture"))?;
        assert!(
            !workshop.matching_sources(&workforce, &changed),
            "orders {field}"
        );
    }
    Ok(())
}
