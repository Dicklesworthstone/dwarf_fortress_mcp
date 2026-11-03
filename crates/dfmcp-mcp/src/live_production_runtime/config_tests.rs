use super::*;
use serde_json::json;

#[test]
fn closed_configuration_and_recovery_are_exact() -> Result<()> {
    let raw = json!({"schema":"dfmcp.combined-production-development/1", "directory":"/tmp/combined", "initialize":true});
    let (directory, initialize, recovery) = parse(&raw.to_string())?;
    assert_eq!(directory, Path::new("/tmp/combined"));
    assert!(initialize && recovery.is_none());
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
    let (_, _, value) = parse(&recovered.to_string())?;
    assert_eq!(
        value
            .ok_or_else(|| invalid("missing test recovery"))?
            .frontier
            .authority_tick_floor,
        4
    );
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
    assert!(!allowed_key("DFMCP_SEMANTIC_WORK_ORDER_ALLOW_CLOCK"));
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
