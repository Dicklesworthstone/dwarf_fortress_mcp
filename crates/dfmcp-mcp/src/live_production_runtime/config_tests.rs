use super::*;
use serde_json::{Value, json};

#[test]
fn closed_configuration_and_recovery_are_exact() -> Result<()> {
    let raw = json!({"schema":"dfmcp.combined-production-development/1", "directory":"/tmp/combined", "initialize":true});
    let (directory, initialize, recovery, clock, inventory_clock) = parse(&raw.to_string())?;
    assert_eq!(directory, Path::new("/tmp/combined"));
    assert!(initialize && recovery.is_none());
    assert!(clock.is_none());
    assert!(inventory_clock.is_none());
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
    let (_, _, value, _, _) = parse(&recovered.to_string())?;
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
        false
    ));
    assert!(allowed_key(
        "DFMCP_SEMANTIC_WORK_ORDER_ALLOW_PRODUCTION",
        false,
        false
    ));
    for clock in [false, true] {
        for inventory_clock in [false, true] {
            for key in [
                "DFMCP_SEMANTIC_WORK_ORDER_ALLOW_CLOCK",
                "DFMCP_SEMANTIC_WORK_ORDER_CLOCK_TOKEN",
            ] {
                assert_eq!(allowed_key(key, clock, inventory_clock), clock, "{key}");
            }
            for key in [
                "DFMCP_SEMANTIC_WORK_ORDER_ALLOW_INVENTORY_CLOCK",
                "DFMCP_SEMANTIC_WORK_ORDER_INVENTORY_CLOCK_TOKEN",
            ] {
                assert_eq!(
                    allowed_key(key, clock, inventory_clock),
                    inventory_clock,
                    "{key}"
                );
            }
            for key in [
                "DFMCP_ADMITTED_BRIDGE_PROTOCOL",
                "DFMCP_RUN_TOKEN",
                "DFMCP_UNKNOWN",
            ] {
                assert!(!allowed_key(key, clock, inventory_clock), "{key}");
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
    let (_, _, _, order_clock, inventory_clock) = parse(&raw.to_string())?;
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
    let (_, _, recovery, clock, inventory_clock) = parse(&raw.to_string())?;
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
