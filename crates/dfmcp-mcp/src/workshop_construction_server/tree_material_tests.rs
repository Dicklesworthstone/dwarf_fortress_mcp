use super::*;

fn configured() -> Value {
    json!({
        "schema":SCHEMA,"directory":"/tmp/dfmcp-workshop-tree-consumer",
        "endpoint":"127.0.0.1:5000","initialize":true,"acknowledged":null,
        "world_folder":"region1","site_id":7,"df_version":"53.16",
        "dfhack_version":"53.16-r3","generation":3,"recovery_generation":null,
        "map_dimensions":[48,48,4],"center":[20,20,2],
        "scope":[0,0,0,47,47,3],"protected":[],"authority_expires_tick":20000,
        "lease_ticks":1200,"checkpoint":"disposable-fortress-no-checkpoint",
        "tree_material_source":{
            "directory":"/tmp/dfmcp-tree-donor","endpoint":"127.0.0.1:5001",
            "generation":9,"root":[10,10,2],"scope":[0,0,0,31,31,3],
            "read_scope":[0,0,0,47,47,3],
            "acknowledged":{"journal_id":"a".repeat(64),"head":"b".repeat(64),"frames":4}
        }
    })
}

#[test]
fn workshop_tree_material_policy_is_closed_and_keeps_independent_source_pins() -> Result<()> {
    let value = configured();
    let config = WorkshopConfig::parse(&value.to_string())?;
    assert!(config.selection.is_none());
    let donor = config.tree_material.as_ref().ok_or_else(exhausted)?;
    assert_eq!(donor.binding.generation, 9);
    assert_eq!(config.input.generation, 3);
    assert_ne!(donor.binding.endpoint, config.endpoint);
    assert_eq!(donor.binding.source, config.identity);
    assert_eq!(donor.binding.dimensions, config.input.map_dimensions);
    assert_eq!(donor.pin.frames, 4);
    let selected = WorkshopSelection::new(42, config.input.center)?;
    assert!(host_candidate(&config, selected, [10, 10, 2]));
    assert!(!host_candidate(&config, selected, [20, 20, 2]));
    assert!(!host_candidate(&config, selected, [48, 10, 2]));
    let mut protected = value.clone();
    protected["protected"] = json!([[10, 10, 2, 10, 10, 2]]);
    let protected = WorkshopConfig::parse(&protected.to_string())?;
    assert!(!host_candidate(&protected, selected, [10, 10, 2]));
    let mut narrowed = value.clone();
    narrowed["scope"] = json!([0, 0, 0, 19, 47, 3]);
    let narrowed = WorkshopConfig::parse(&narrowed.to_string())?;
    assert!(!host_candidate(&narrowed, selected, [10, 10, 2]));

    for key in ["item_id", "native_command", "initialize", "recovery_generation"] {
        let mut changed = value.clone();
        changed["tree_material_source"][key] = json!(42);
        assert!(WorkshopConfig::parse(&changed.to_string()).is_err(), "{key}");
    }
    let mut changed = value.clone();
    changed["item_id"] = json!(42);
    assert!(WorkshopConfig::parse(&changed.to_string()).is_err());
    let mut changed = value.clone();
    changed["schema"] = json!("dfmcp.workshop-mcp-development/1");
    assert!(WorkshopConfig::parse(&changed.to_string()).is_err());
    let mut changed = value;
    changed.as_object_mut().ok_or_else(exhausted)?.remove("tree_material_source");
    assert!(WorkshopConfig::parse(&changed.to_string()).is_err());
    Ok(())
}

#[test]
fn workshop_tree_material_configuration_refuses_incomplete_scope_and_ambiguous_custody() {
    let cases = [
        ("read_scope", json!([0, 0, 0, 31, 47, 3])),
        ("scope", json!([0, 0, 0, 48, 47, 3])),
        ("root", json!([48, 10, 2])),
        ("generation", json!(0)),
        ("generation", json!(u64::MAX)),
        ("endpoint", json!("localhost:5001")),
        ("endpoint", json!("127.0.0.1:0")),
        ("directory", json!("/tmp/dfmcp-workshop-tree-consumer")),
        ("directory", json!("/tmp/../dfmcp-tree-donor")),
        ("directory", json!("relative/tree-donor")),
        ("acknowledged", json!({"journal_id":"a".repeat(64),"head":"b".repeat(64),"frames":0})),
    ];
    for (key, value) in cases {
        let mut input = configured();
        input["tree_material_source"][key] = value;
        assert!(WorkshopConfig::parse(&input.to_string()).is_err(), "{key}");
    }
}

#[test]
fn workshop_legacy_material_and_recovery_configuration_remain_distinct() -> Result<()> {
    let mut legacy = configured();
    legacy["schema"] = json!("dfmcp.workshop-mcp-development/1");
    legacy["item_id"] = json!(42);
    legacy.as_object_mut().ok_or_else(exhausted)?.remove("tree_material_source");
    let pinned = WorkshopConfig::parse(&legacy.to_string())?;
    assert_eq!(pinned.selection.ok_or_else(exhausted)?.item_id(), 42);
    assert!(pinned.tree_material.is_none());

    let mut recovery = configured();
    recovery["initialize"] = json!(false);
    recovery["acknowledged"] = json!({"journal_id":"c".repeat(64),"head":"d".repeat(64),"frames":3});
    let config = WorkshopConfig::parse(&recovery.to_string())?;
    assert!(!config.input.initialize);
    assert!(config.selection.is_none());
    assert!(config.pin.is_some());
    // Configuration parsing is offline: neither directory nor native peer needs
    // to exist. Runtime recovery obtains selection only from its original plan.
    Ok(())
}
