use super::*;
use serde_json::json;

fn fixture() -> Environment {
    let mut values = BTreeMap::new();
    for key in REQUIRED_OPT_INS {
        values.insert(key.to_owned(), "1".to_owned());
    }
    values.insert(
        CONFIG.to_owned(),
        json!({
            "schema":"dfmcp.workshop-production-development/1"
        })
        .to_string(),
    );
    values.insert(
        WORKSHOP_CONFIG.to_owned(),
        json!({
            "schema":"dfmcp.workshop-mcp-development/1",
            "directory":"/tmp/staged-workshop","endpoint":"127.0.0.1:5001",
            "initialize":true,"world_folder":"region1","site_id":7,
            "df_version":"df","dfhack_version":"hack","generation":30,
            "map_dimensions":[64,64,4],"item_id":2,"center":[3,3,0],
            "scope":[0,0,0,63,63,3],"authority_expires_tick":1000,
            "lease_ticks":20,"checkpoint":"disposable-fortress-no-checkpoint"
        })
        .to_string(),
    );
    values.insert(
        PRODUCTION_CONFIG.to_owned(),
        json!({
            "schema":"dfmcp.combined-production-development/1",
            "directory":"/tmp/staged-production","initialize":true
        })
        .to_string(),
    );
    values.insert(
        WORKFORCE_CONFIG.to_owned(),
        json!({
            "schema":"dfmcp.semantic-workforce-development/1",
            "directory":"/tmp/staged-workforce","endpoint":"127.0.0.1:5000","initialize":true,
            "sources":{"kind":"operator-development-source-acceptance/1",
                "world_folder":"region1","site_id":7,"df_version":"df","dfhack_version":"hack",
                "citizen_bridge_version":"bridge","citizen_generation":10,"workforce_generation":20}
        })
        .to_string(),
    );
    values.insert(
        ORDER_CONFIG.to_owned(),
        json!({
            "schema":"dfmcp.semantic-work-order-development/1",
            "directory":"/tmp/staged-orders","endpoint":"127.0.0.1:5000","initialize":true,
            "sources":{"kind":"operator-development-source-acceptance/1",
                "world_folder":"region1","site_id":7,"df_version":"df","dfhack_version":"hack",
                "citizen_bridge_version":"bridge","citizen_generation":10,
                "work_order_generation":21,"progress_generation":22,"operations_generation":23}
        })
        .to_string(),
    );
    Environment { values }
}

fn edit(environment: &mut Environment, key: &str, change: impl FnOnce(&mut Value)) -> Result<()> {
    let mut value: Value =
        serde_json::from_str(environment.required(key)?).map_err(|_| invalid("fixture JSON"))?;
    change(&mut value);
    environment.values.insert(key.to_owned(), value.to_string());
    Ok(())
}

fn reopened(stage: Stage) -> Result<Environment> {
    let mut environment = fixture();
    let hash = Digest32::of_bytes(b"staged recovery fixture").to_hex();
    for key in [
        WORKSHOP_CONFIG,
        PRODUCTION_CONFIG,
        WORKFORCE_CONFIG,
        ORDER_CONFIG,
    ] {
        edit(&mut environment, key, |value| {
            value["initialize"] = json!(false)
        })?;
    }
    edit(&mut environment, WORKSHOP_CONFIG, |value| {
        value["acknowledged"] = json!({"journal_id":hash,"head":hash,"frames":7});
    })?;
    edit(&mut environment, WORKFORCE_CONFIG, |value| {
        value["journal_id"] = json!(hash);
    })?;
    edit(&mut environment, CONFIG, |value| {
        value["recovery"] = json!({"stage":stage.as_str(),"authority_tick_floor":120});
        if stage != Stage::Workshop {
            value["recovery"]["handoff_digest"] = json!(hash);
        }
    })?;
    Ok(environment)
}

#[test]
fn closed_stages_require_exact_nonnull_handoff_and_bounded_floor() -> Result<()> {
    assert!(parse(r#"{"schema":"dfmcp.workshop-production-development/1"}"#)?.is_none());
    for stage in [Stage::Workshop, Stage::Transition, Stage::Production] {
        let environment = reopened(stage)?;
        assert_eq!(
            parse(environment.required(CONFIG)?)?.map(|r| r.stage),
            Some(stage)
        );
    }
    let good: Value = serde_json::from_str(reopened(Stage::Production)?.required(CONFIG)?)
        .map_err(|_| invalid("fixture"))?;
    for (field, value) in [
        ("stage", json!("done")),
        ("handoff_digest", Value::Null),
        ("handoff_digest", json!("00".repeat(32))),
        ("handoff_digest", json!("AA".repeat(32))),
        ("authority_tick_floor", json!(MAX_NATIVE_TICK + 1)),
        ("authority_tick_floor", Value::Null),
        ("original_key", json!("caller-key")),
    ] {
        let mut bad = good.clone();
        bad["recovery"][field] = value;
        assert!(parse(&bad.to_string()).is_err(), "{field}");
    }
    for field in ["stage", "handoff_digest", "authority_tick_floor"] {
        let mut bad = good.clone();
        bad["recovery"]
            .as_object_mut()
            .ok_or_else(|| invalid("fixture"))?
            .remove(field);
        assert!(parse(&bad.to_string()).is_err(), "missing {field}");
    }
    for recovery in [Value::Null, json!({}), json!("production")] {
        let bad = json!({"schema":"dfmcp.workshop-production-development/1","recovery":recovery});
        assert!(parse(&bad.to_string()).is_err());
    }
    let mut bad = good;
    bad["recovery"]["stage"] = json!("workshop");
    assert!(parse(&bad.to_string()).is_err());
    bad["recovery"]
        .as_object_mut()
        .ok_or_else(|| invalid("fixture"))?
        .remove("handoff_digest");
    bad["endpoint"] = json!("127.0.0.1:5000");
    assert!(parse(&bad.to_string()).is_err());
    assert!(parse(r#"{"schema":"dfmcp.workshop-production-development/1","schema":"dfmcp.workshop-production-development/1"}"#).is_err());
    Ok(())
}

#[test]
fn all_sources_are_checked_without_equating_independent_generations() -> Result<()> {
    let environment = fixture();
    assert!(environment.validate()?.is_none());
    for key in [WORKFORCE_CONFIG, ORDER_CONFIG] {
        for (field, changed) in [
            ("world_folder", json!("other")),
            ("site_id", json!(8)),
            ("df_version", json!("other")),
            ("dfhack_version", json!("other")),
            ("citizen_bridge_version", json!("other")),
            ("citizen_generation", json!(11)),
        ] {
            let mut bad = Environment {
                values: environment.values.clone(),
            };
            edit(&mut bad, key, |value| value["sources"][field] = changed)?;
            assert!(bad.validate().is_err(), "{key} {field}");
        }
    }
    let mut bad = Environment {
        values: environment.values.clone(),
    };
    edit(&mut bad, ORDER_CONFIG, |value| {
        value["endpoint"] = json!("127.0.0.1:5002")
    })?;
    assert!(bad.validate().is_err());
    Ok(())
}

#[test]
fn every_owned_directory_must_be_disjoint_and_nonnested() -> Result<()> {
    for key in [PRODUCTION_CONFIG, WORKFORCE_CONFIG, ORDER_CONFIG] {
        for directory in ["/tmp/staged-workshop", "/tmp/staged-workshop/child", "/tmp"] {
            let mut bad = fixture();
            edit(&mut bad, key, |value| value["directory"] = json!(directory))?;
            assert!(bad.validate().is_err(), "{key} {directory}");
        }
    }
    assert!(disjoint_directories(&[
        Path::new("/tmp/one"),
        Path::new("/tmp/one-more")
    ]));
    assert!(!disjoint_directories(&[
        Path::new("/tmp/one"),
        Path::new("/tmp/one/child")
    ]));
    Ok(())
}

#[test]
fn recovery_never_reinitializes_a_child_or_rewinds_an_original_into_workshop() -> Result<()> {
    for stage in [Stage::Workshop, Stage::Transition, Stage::Production] {
        let environment = reopened(stage)?;
        assert_eq!(environment.validate()?.map(|r| r.stage), Some(stage));
        for key in [
            WORKSHOP_CONFIG,
            PRODUCTION_CONFIG,
            WORKFORCE_CONFIG,
            ORDER_CONFIG,
        ] {
            let mut bad = Environment {
                values: environment.values.clone(),
            };
            edit(&mut bad, key, |value| value["initialize"] = json!(true))?;
            assert!(bad.validate().is_err(), "{key} {stage:?}");
        }
    }
    for stage in [Stage::Workshop, Stage::Transition] {
        let mut bad = reopened(stage)?;
        let hash = Digest32::of_bytes(b"combined original").to_hex();
        edit(&mut bad, PRODUCTION_CONFIG, |value| {
            value["recovery"] = json!({"plan_digest":hash,"object_digest":hash,"identity_digest":hash,
                "anchor":{"fortress_id":"1","epoch":0,"sequence":1,"tick":120,"state_hash":hash},
                "frontier":{"journal_id":hash,"head":hash,"authority_tick_floor":120}});
        })?;
        assert!(bad.validate().is_err());
    }
    Ok(())
}

#[test]
fn consumed_production_floors_cannot_exceed_parent_checkpoint() -> Result<()> {
    let mut environment = reopened(Stage::Transition)?;
    let hash = Digest32::of_bytes(b"retained donor floor").to_hex();
    edit(&mut environment, PRODUCTION_CONFIG, |value| {
        value["workshop_prerequisite_checkpoint"] = json!({
            "origin_witness":hash,"tick":121,"generation":30,"next_building_id":91,"fenced":false
        });
    })?;
    assert!(environment.validate().is_err());
    edit(&mut environment, CONFIG, |value| {
        value["recovery"]["authority_tick_floor"] = json!(121)
    })?;
    assert_eq!(
        environment.validate()?.map(|r| r.authority_tick_floor),
        Some(121)
    );
    edit(&mut environment, CONFIG, |value| {
        value["recovery"]["stage"] = json!("workshop");
        value["recovery"]
            .as_object_mut()
            .map(|r| r.remove("handoff_digest"));
    })?;
    assert!(environment.validate().is_err());
    Ok(())
}

#[test]
fn union_allowlist_keeps_optional_credentials_and_write_grants_independent() -> Result<()> {
    let environment = fixture();
    assert!(environment.validate()?.is_none());
    for key in REQUIRED_OPT_INS {
        let mut bad = Environment {
            values: environment.values.clone(),
        };
        bad.values.remove(key);
        assert!(bad.validate().is_err(), "missing {key}");
        bad.values.insert(key.to_owned(), "true".to_owned());
        assert!(bad.validate().is_err(), "invalid {key}");
    }
    for key in PERMISSIONS {
        let mut bad = Environment {
            values: environment.values.clone(),
        };
        bad.values.insert(key.to_owned(), "0".to_owned());
        assert!(bad.validate().is_err(), "{key}");
    }
    for key in [
        "DFMCP_ADMITTED_BRIDGE_PROTOCOL",
        "DFMCP_ALLOW_UNADMITTED_TREE_HARVEST",
        "DFMCP_TREE_HARVEST_TOKEN",
        "DFMCP_RUN_TOKEN",
        "DFMCP_COMBINED_PRODUCTION_MANAGER_TOKEN",
        "DFMCP_SEMANTIC_WORKFORCE_CLOCK_TOKEN",
        "DFMCP_SEMANTIC_WORK_ORDER_CLOCK_TOKEN",
        "DFMCP_SEMANTIC_WORK_ORDER_INVENTORY_CLOCK_TOKEN",
    ] {
        let mut bad = Environment {
            values: environment.values.clone(),
        };
        bad.values.insert(key.to_owned(), "x".repeat(32));
        assert!(bad.validate().is_err(), "{key}");
    }
    let mut donor_reader = environment;
    donor_reader.values.insert(
        "DFMCP_COMBINED_PRODUCTION_WORKSHOP_TOKEN".to_owned(),
        "x".repeat(32),
    );
    assert!(
        donor_reader.validate()?.is_none(),
        "the future verified donor has its separate read credential"
    );
    assert!(
        !donor_reader
            .values
            .contains_key("DFMCP_WORKSHOP_ALLOW_BUILD")
    );
    Ok(())
}

#[test]
fn environment_pin_covers_raw_child_bytes_optins_and_optional_authority() -> Result<()> {
    let environment = fixture();
    let original = environment.digest();
    for key in CONFIGURATIONS {
        let mut changed = Environment {
            values: environment.values.clone(),
        };
        changed
            .values
            .get_mut(key)
            .ok_or_else(|| invalid("fixture"))?
            .push(' ');
        assert!(
            changed.validate()?.is_none(),
            "formatting is valid configuration"
        );
        assert_ne!(
            changed.digest(),
            original,
            "exact raw {key} is still pinned"
        );
    }
    for key in REQUIRED_OPT_INS {
        let mut changed = Environment {
            values: environment.values.clone(),
        };
        changed.values.remove(key);
        assert_ne!(changed.digest(), original);
    }
    for key in [
        "DFMCP_WORKSHOP_ALLOW_BUILD",
        "DFMCP_WORKSHOP_TOKEN",
        "DFMCP_COMBINED_PRODUCTION_WORKSHOP_TOKEN",
    ] {
        let mut changed = Environment {
            values: environment.values.clone(),
        };
        changed.values.insert(key.to_owned(), "1".to_owned());
        assert_ne!(changed.digest(), original);
        changed
            .values
            .insert(key.to_owned(), "different".to_owned());
        assert_ne!(changed.digest(), original);
    }
    Ok(())
}

#[test]
fn workshop_clock_cannot_be_added_or_reinitialized_during_parent_recovery() -> Result<()> {
    let mut environment = fixture();
    edit(&mut environment, WORKSHOP_CONFIG, |value| {
        value["clock"] = json!({"initialize":true,"generation":41,"deadline_tick":900});
    })?;
    assert!(
        environment.validate().is_err(),
        "independent clock opt-in is required"
    );
    environment
        .values
        .insert(WORKSHOP_CLOCK_OPT_IN.to_owned(), "1".to_owned());
    assert!(environment.validate()?.is_none());
    let mut recovered = reopened(Stage::Workshop)?;
    edit(&mut recovered, WORKSHOP_CONFIG, |value| {
        value["clock"] = json!({"initialize":true,"generation":41,"deadline_tick":900});
    })?;
    recovered
        .values
        .insert(WORKSHOP_CLOCK_OPT_IN.to_owned(), "1".to_owned());
    assert!(recovered.validate().is_err());
    let hash = Digest32::of_bytes(b"retained workshop clock").to_hex();
    edit(&mut recovered, WORKSHOP_CONFIG, |value| {
        value["clock"] = json!({"initialize":false,"generation":41,"deadline_tick":900,
            "recovery":{"journal_id":hash,"head":hash}});
    })?;
    assert!(recovered.validate()?.is_some());
    Ok(())
}

#[test]
fn caller_supplied_workshop_donor_never_enters_parent_configuration() -> Result<()> {
    for donor in [Value::Null, json!({}), json!({"directory":"/tmp/foreign"})] {
        let mut bad = fixture();
        edit(&mut bad, PRODUCTION_CONFIG, |value| {
            value["workshop_prerequisite"] = donor
        })?;
        assert!(bad.validate().is_err());
    }
    Ok(())
}

#[test]
fn tree_material_and_manager_donors_join_the_same_disjoint_custody_check() -> Result<()> {
    let mut environment = fixture();
    let hash = Digest32::of_bytes(b"optional donor").to_hex();
    edit(&mut environment, WORKSHOP_CONFIG, |value| {
        value["schema"] = json!("dfmcp.workshop-tree-material-mcp-development/1");
        value.as_object_mut().map(|object| object.remove("item_id"));
        value["tree_material_source"] = json!({
            "directory":"/tmp/staged-tree","endpoint":"127.0.0.1:5003",
            "generation":40,"root":[10,10,0],"scope":[0,0,0,63,63,3],
            "read_scope":[0,0,0,63,63,3],
            "acknowledged":{"journal_id":hash,"head":hash,"frames":7}
        });
    })?;
    environment
        .values
        .insert(TREE_OPT_IN.to_owned(), "1".to_owned());
    edit(&mut environment, PRODUCTION_CONFIG, |value| {
        value["manager_prerequisite"] = json!({
            "directory":"/tmp/staged-manager","endpoint":"127.0.0.1:5004",
            "world_folder":"region1","site_id":7,"df_version":"df","dfhack_version":"hack",
            "generation":50,"map_dimensions":[64,64,4],"unit_id":42,"histfig_id":142,
            "acknowledged":{"journal_id":hash,"head":hash,"frames":7},
            "original_key":"original-manager.1","original_plan_digest":hash
        });
    })?;
    assert!(environment.validate()?.is_none());
    for directory in [
        "/tmp/staged-workforce",
        "/tmp/staged-orders/child",
        "/tmp/staged-manager",
    ] {
        let mut bad = Environment {
            values: environment.values.clone(),
        };
        edit(&mut bad, WORKSHOP_CONFIG, |value| {
            value["tree_material_source"]["directory"] = json!(directory);
        })?;
        assert!(bad.validate().is_err(), "Tree {directory}");
    }
    let mut bad = environment;
    edit(&mut bad, PRODUCTION_CONFIG, |value| {
        value["manager_prerequisite"]["directory"] = json!("/tmp/staged-tree/child");
    })?;
    assert!(bad.validate().is_err());
    Ok(())
}
