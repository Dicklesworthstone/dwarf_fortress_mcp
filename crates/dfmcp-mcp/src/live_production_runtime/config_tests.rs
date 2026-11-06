use super::*;
use serde_json::{Value, json};

#[test]
fn closed_configuration_and_recovery_are_exact() -> Result<()> {
    let raw = json!({"schema":"dfmcp.combined-production-development/1", "directory":"/tmp/combined", "initialize":true});
    let (directory, initialize, recovery, clock) = parse(&raw.to_string())?;
    assert_eq!(directory, Path::new("/tmp/combined"));
    assert!(initialize && recovery.is_none());
    assert!(clock.is_none());
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
    let (_, _, value, _) = parse(&recovered.to_string())?;
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
    assert!(allowed_key("DFMCP_SEMANTIC_WORKFORCE_ALLOW_LABOR"));
    assert!(allowed_key("DFMCP_SEMANTIC_WORK_ORDER_ALLOW_PRODUCTION"));
    assert!(allowed_key("DFMCP_SEMANTIC_WORK_ORDER_ALLOW_CLOCK"));
    assert!(allowed_key("DFMCP_SEMANTIC_WORK_ORDER_CLOCK_TOKEN"));
    assert!(!allowed_key(
        "DFMCP_SEMANTIC_WORK_ORDER_ALLOW_INVENTORY_CLOCK"
    ));
    assert!(!allowed_key(
        "DFMCP_SEMANTIC_WORK_ORDER_INVENTORY_CLOCK_TOKEN"
    ));
    assert!(!allowed_key("DFMCP_ADMITTED_BRIDGE_PROTOCOL"));
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
