//! Closed operator pins and externally retained Manager rejection custody.
use super::*;
use dfmcp_adapter::manager_appointment::ManagerProtocol;
use serde_json::{Value, json};

fn configuration() -> Value {
    json!({"schema":"dfmcp.combined-production-development/1",
    "directory":"/tmp/combined","initialize":true,
    "manager_prerequisite":{
        "directory":"/tmp/completed-manager","endpoint":"127.0.0.1:5002",
        "world_folder":"region1","site_id":7,"df_version":"df","dfhack_version":"hack",
        "generation":34,"map_dimensions":[64,64,4],"unit_id":42,"histfig_id":142,
        "acknowledged":{"journal_id":Digest32::of_bytes(b"manager donor journal").to_hex(),
            "head":Digest32::of_bytes(b"manager donor head").to_hex(),"frames":5},
        "original_key":"original-manager.1",
        "original_plan_digest":Digest32::of_bytes(b"original manager plan").to_hex()
    }})
}

fn floor() -> Value {
    json!({"tick":130,"generation":34,"sequence":8,"next_event":102,
        "history_count":42,"group_next_position":15,"group_next_assignment":20,
        "civ_next_position":12,"civ_next_assignment":16,"fenced":false})
}

fn recovery() -> Value {
    let hash = Digest32::of_bytes(b"original combined custody").to_hex();
    json!({"plan_digest":hash,"object_digest":hash,"identity_digest":hash,
        "anchor":{"fortress_id":"1","epoch":0,"sequence":2,"tick":120,"state_hash":hash},
        "frontier":{"journal_id":hash,"head":hash,"authority_tick_floor":130},
        "manager_prerequisite_digest":Digest32::of_bytes(b"original manager association").to_hex(),
        "manager_prerequisite_floor":floor()})
}

#[test]
fn manager_pins_select_only_cache_bearing_original_and_complete_read_scope() -> Result<()> {
    let raw = configuration();
    let parsed = parse(&raw.to_string())?;
    assert!(parsed.1 && parsed.2.is_none() && parsed.5.is_none() && parsed.6.is_none());
    assert!(parsed.8.is_none());
    let manager = parsed.7.ok_or_else(|| invalid("manager fixture"))?;
    assert_eq!(manager.directory, Path::new("/tmp/completed-manager"));
    assert_eq!(manager.binding.protocol, ManagerProtocol::V1_23);
    assert_eq!(manager.binding.endpoint.to_string(), "127.0.0.1:5002");
    assert_eq!(manager.binding.source.folder(), "region1");
    assert_eq!(manager.binding.source.site(), 7);
    assert_eq!(manager.binding.generation, 34);
    assert_eq!(manager.binding.selection.unit, 42);
    assert_eq!(manager.binding.selection.figure, 142);
    assert_eq!(manager.binding.scope, manager.binding.read_scope()?);
    assert_eq!(manager.acknowledged.frames, 5);
    assert_eq!(manager.original_key, "original-manager.1");
    for field in [
        "directory",
        "endpoint",
        "world_folder",
        "site_id",
        "df_version",
        "dfhack_version",
        "generation",
        "map_dimensions",
        "unit_id",
        "histfig_id",
        "acknowledged",
        "original_key",
        "original_plan_digest",
    ] {
        let mut bad = raw.clone();
        bad["manager_prerequisite"]
            .as_object_mut()
            .ok_or_else(|| invalid("fixture"))?
            .remove(field);
        assert!(parse(&bad.to_string()).is_err(), "missing {field}");
    }
    for field in [
        "protocol",
        "method",
        "role",
        "read_scope",
        "scope",
        "initialize",
        "token",
    ] {
        let mut bad = raw.clone();
        bad["manager_prerequisite"][field] = json!("unreviewed");
        assert!(parse(&bad.to_string()).is_err(), "extra {field}");
    }
    for (field, value) in [
        ("endpoint", json!("localhost:5002")),
        ("endpoint", json!("127.0.0.1:0")),
        ("generation", json!(0)),
        ("generation", json!(u64::MAX)),
        ("map_dimensions", json!([0, 64, 4])),
        ("map_dimensions", json!([32769, 64, 4])),
        ("unit_id", json!(i32::MAX)),
        ("histfig_id", json!(-1)),
        ("world_folder", json!("region\n1")),
        ("original_plan_digest", json!("0".repeat(64))),
    ] {
        let mut bad = raw.clone();
        bad["manager_prerequisite"][field] = value;
        assert!(parse(&bad.to_string()).is_err(), "invalid {field}");
    }
    for frames in [0, 17, u32::MAX] {
        let mut bad = raw.clone();
        bad["manager_prerequisite"]["acknowledged"]["frames"] = json!(frames);
        assert!(parse(&bad.to_string()).is_err());
    }
    Ok(())
}

#[test]
fn manager_and_workshop_require_disjoint_donors_but_preserve_independent_incarnations() -> Result<()>
{
    let mut raw = configuration();
    let mut workshop = raw["manager_prerequisite"].clone();
    let fields = workshop.as_object_mut().ok_or_else(|| invalid("fixture"))?;
    fields.remove("unit_id");
    fields.remove("histfig_id");
    workshop["directory"] = json!("/tmp/completed-workshop");
    workshop["generation"] = json!(30);
    raw["workshop_prerequisite"] = workshop;
    let parsed = parse(&raw.to_string())?;
    assert_eq!(
        parsed
            .5
            .ok_or_else(|| invalid("workshop"))?
            .binding
            .generation(),
        30
    );
    assert_eq!(
        parsed
            .7
            .ok_or_else(|| invalid("manager"))?
            .binding
            .generation,
        34
    );
    for path in [
        "/tmp/combined",
        "/tmp/combined/nested",
        "/tmp",
        "relative-manager",
        "/tmp/../manager",
        "/tmp//manager",
        "/tmp/manager/",
        "/tmp/manager\n",
        "/tmp/completed-workshop",
        "/tmp/completed-workshop/nested",
    ] {
        let mut bad = raw.clone();
        bad["manager_prerequisite"]["directory"] = json!(path);
        assert!(parse(&bad.to_string()).is_err(), "directory {path:?}");
    }
    Ok(())
}

#[test]
fn manager_recovery_requires_full_exact_association_and_legal_consumed_frontier() -> Result<()> {
    let mut raw = configuration();
    raw["initialize"] = json!(false);
    raw["recovery"] = recovery();
    let retained = parse(&raw.to_string())?
        .2
        .ok_or_else(|| invalid("recovery"))?;
    assert!(retained.manager_prerequisite_digest.is_some());
    assert_eq!(
        retained
            .manager_prerequisite_floor
            .ok_or_else(|| invalid("floor"))?
            .tick,
        130
    );
    for field in ["manager_prerequisite_digest", "manager_prerequisite_floor"] {
        let mut bad = raw.clone();
        bad["recovery"]
            .as_object_mut()
            .ok_or_else(|| invalid("fixture"))?
            .remove(field);
        assert!(parse(&bad.to_string()).is_err(), "missing {field}");
        let mut bad = raw.clone();
        bad["recovery"][field] = Value::Null;
        assert!(parse(&bad.to_string()).is_err(), "null {field}");
    }
    for field in [
        "tick",
        "generation",
        "sequence",
        "next_event",
        "history_count",
        "group_next_position",
        "group_next_assignment",
        "civ_next_position",
        "civ_next_assignment",
        "fenced",
    ] {
        let mut bad = raw.clone();
        bad["recovery"]["manager_prerequisite_floor"]
            .as_object_mut()
            .ok_or_else(|| invalid("fixture"))?
            .remove(field);
        assert!(parse(&bad.to_string()).is_err(), "missing {field}");
    }
    for (field, value) in [
        ("tick", json!(131)),
        ("generation", json!(33)),
        ("generation", json!(35)),
        ("sequence", json!(u64::MAX)),
        ("next_event", json!(i32::MAX)),
        ("history_count", json!(65537)),
        ("group_next_position", json!(u32::MAX)),
        ("group_next_assignment", json!(u32::MAX)),
        ("civ_next_position", json!(u32::MAX)),
        ("civ_next_assignment", json!(u32::MAX)),
        ("fenced", json!("false")),
        ("extra", json!(true)),
    ] {
        let mut bad = raw.clone();
        bad["recovery"]["manager_prerequisite_floor"][field] = value;
        assert!(parse(&bad.to_string()).is_err(), "invalid {field}");
    }
    raw["recovery"]["manager_prerequisite_floor"]["generation"] = json!(35);
    raw["recovery"]["manager_prerequisite_floor"]["fenced"] = json!(true);
    assert!(parse(&raw.to_string())?.2.is_some());
    raw.as_object_mut()
        .ok_or_else(|| invalid("fixture"))?
        .remove("manager_prerequisite");
    assert!(
        parse(&raw.to_string()).is_err(),
        "cannot drop original manager policy"
    );
    Ok(())
}

#[test]
fn manager_preplan_checkpoint_is_closed_source_bound_and_separate_from_original_recovery()
-> Result<()> {
    let mut raw = configuration();
    let mut checkpoint = floor();
    checkpoint["origin_witness"] = json!(Digest32::of_bytes(b"exact manager origin").to_hex());
    raw["manager_prerequisite_checkpoint"] = checkpoint;
    let retained = parse(&raw.to_string())?
        .8
        .ok_or_else(|| invalid("checkpoint"))?;
    assert_eq!(
        retained.origin_witness,
        Digest32::of_bytes(b"exact manager origin")
    );
    assert_eq!(retained.floor.tick, 130);
    for field in [
        "origin_witness",
        "tick",
        "next_event",
        "civ_next_assignment",
        "fenced",
    ] {
        let mut bad = raw.clone();
        bad["manager_prerequisite_checkpoint"]
            .as_object_mut()
            .ok_or_else(|| invalid("fixture"))?
            .remove(field);
        assert!(parse(&bad.to_string()).is_err());
    }
    for checkpoint in [Value::Null, json!({}), json!("checkpoint")] {
        let mut bad = raw.clone();
        bad["manager_prerequisite_checkpoint"] = checkpoint;
        assert!(parse(&bad.to_string()).is_err());
    }
    let mut missing = raw.clone();
    missing
        .as_object_mut()
        .ok_or_else(|| invalid("fixture"))?
        .remove("manager_prerequisite");
    assert!(parse(&missing.to_string()).is_err());
    raw["initialize"] = json!(false);
    raw["recovery"] = recovery();
    assert!(
        parse(&raw.to_string()).is_err(),
        "original and preplan floors cannot compete"
    );
    Ok(())
}

#[test]
fn manager_environment_accepts_only_independent_configured_read_credential() {
    for manager in [false, true] {
        for workshop in [false, true] {
            for clock in [false, true] {
                for inventory_clock in [false, true] {
                    assert_eq!(
                        allowed_key(
                            manager_config::TOKEN,
                            clock,
                            inventory_clock,
                            workshop,
                            manager
                        ),
                        manager
                    );
                    assert_eq!(
                        allowed_key(
                            workshop_config::TOKEN,
                            clock,
                            inventory_clock,
                            workshop,
                            manager
                        ),
                        workshop
                    );
                    for foreign in [
                        "DFMCP_MANAGER_TOKEN",
                        "DFMCP_MANAGER_V1_23_TOKEN",
                        "DFMCP_MANAGER_V1_23_ALLOW_APPOINT",
                        "DFMCP_MANAGER_V1_23_MCP_CONFIG",
                        "DFMCP_ALLOW_UNADMITTED_MANAGER_V1_23",
                        "DFMCP_COMBINED_PRODUCTION_MANAGER_ALLOW_APPOINT",
                        "DFMCP_ADMITTED_BRIDGE_PROTOCOL",
                    ] {
                        assert!(
                            !allowed_key(foreign, clock, inventory_clock, workshop, manager),
                            "{foreign}"
                        );
                    }
                }
            }
        }
    }
    for length in [32, 256] {
        assert!(ManagerConfig::validate_token(&"x".repeat(length)).is_ok());
    }
    for token in [
        String::new(),
        "x".repeat(31),
        "x".repeat(257),
        format!("{}\0", "x".repeat(32)),
    ] {
        assert!(ManagerConfig::validate_token(&token).is_err());
    }
}
