use super::*;

fn configured() -> Value {
    json!({
        "schema":"dfmcp.workshop-mcp-development/1",
        "directory":"/tmp/dfmcp-workshop-clock-consumer",
        "endpoint":"127.0.0.1:5000","initialize":true,"acknowledged":null,
        "world_folder":"region1","site_id":7,"df_version":"53.16",
        "dfhack_version":"53.16-r3","generation":3,"recovery_generation":null,
        "map_dimensions":[48,48,4],"item_id":42,"center":[20,20,2],
        "scope":[0,0,0,47,47,3],"protected":[],"authority_expires_tick":20000,
        "lease_ticks":1200,"checkpoint":"disposable-fortress-no-checkpoint",
        "clock":{
            "initialize":true,"generation":9,"recovery_generation":null,
            "deadline_tick":19000,"recovery":null
        }
    })
}

fn run_request() -> Value {
    json!({
        "kind":"run_workshop","target_id":"a".repeat(64),
        "game_ticks":1200,"wall_millis":60000
    })
}

#[test]
fn clock_configuration_preserves_independent_generation_and_legacy_default() -> Result<()> {
    let value = configured();
    let config = WorkshopConfig::parse(&value.to_string())?;
    let clock = config.clock.as_ref().ok_or_else(exhausted)?;
    assert_eq!(config.input.generation, 3);
    assert_eq!(clock.binding.manifest().generation, 9);
    assert_eq!(clock.recovery_generation, 9);
    assert_eq!(clock.binding.endpoint(), config.endpoint);
    assert_eq!(clock.binding.manifest().df_version, config.input.df_version);
    assert_eq!(
        clock.binding.manifest().dfhack_version,
        config.input.dfhack_version
    );
    assert_eq!(clock.deadline_tick, 19000);
    assert!(clock.control(WorkshopMode::Control));
    assert!(!clock.control(WorkshopMode::Recover));
    assert!(!clock.control(WorkshopMode::Offline));

    // The opt-in field does not change the legacy parser's required material.
    let mut legacy = value;
    legacy
        .as_object_mut()
        .ok_or_else(exhausted)?
        .remove("clock");
    let legacy = WorkshopConfig::parse(&legacy.to_string())?;
    assert!(legacy.clock.is_none());
    assert_eq!(legacy.selection.ok_or_else(exhausted)?.item_id(), 42);
    Ok(())
}

#[test]
fn clock_configuration_is_closed_and_cannot_select_another_native_source() -> Result<()> {
    for key in [
        "endpoint",
        "directory",
        "df_version",
        "token",
        "native_command",
        "workshop_generation",
        "fortress_id",
        "pause",
        "controller",
    ] {
        let mut value = configured();
        value["clock"][key] = json!("untrusted-selector");
        assert!(WorkshopConfig::parse(&value.to_string()).is_err(), "{key}");
    }
    for key in ["initialize", "generation", "deadline_tick"] {
        let mut value = configured();
        value["clock"]
            .as_object_mut()
            .ok_or_else(exhausted)?
            .remove(key);
        assert!(
            WorkshopConfig::parse(&value.to_string()).is_err(),
            "missing {key}"
        );
    }
    Ok(())
}

#[test]
fn clock_generation_pins_refuse_reserved_or_regressed_incarnations() -> Result<()> {
    for (key, bad) in [
        ("generation", 0),
        ("generation", u64::MAX),
        ("recovery_generation", 0),
        ("recovery_generation", 8),
        ("recovery_generation", u64::MAX),
    ] {
        let mut value = configured();
        value["clock"][key] = json!(bad);
        assert!(
            WorkshopConfig::parse(&value.to_string()).is_err(),
            "{key}={bad}"
        );
    }
    for generation in [9, 12] {
        let mut value = configured();
        value["clock"]["recovery_generation"] = json!(generation);
        let config = WorkshopConfig::parse(&value.to_string())?;
        let clock = config.clock.as_ref().ok_or_else(exhausted)?;
        assert_eq!(clock.binding.manifest().generation, 9);
        assert_eq!(clock.recovery_generation, generation);
    }
    // Independent incarnation pins may happen to share a number; equality is
    // neither required nor evidence of shared native process identity.
    let mut equal = configured();
    equal["clock"]["generation"] = json!(3);
    assert!(WorkshopConfig::parse(&equal.to_string()).is_ok());
    Ok(())
}

#[test]
fn explicit_clock_horizon_cannot_exceed_authority_or_native_tick_domain() -> Result<()> {
    for bad in [0, 20001, u64::MAX] {
        let mut value = configured();
        value["clock"]["deadline_tick"] = json!(bad);
        assert!(
            WorkshopConfig::parse(&value.to_string()).is_err(),
            "deadline {bad}"
        );
    }
    let mut at_authority = configured();
    at_authority["clock"]["deadline_tick"] = json!(20000);
    assert_eq!(
        WorkshopConfig::parse(&at_authority.to_string())?
            .clock
            .ok_or_else(exhausted)?
            .deadline_tick,
        20000
    );

    let native_max = dfmcp_adapter::bounded_run::MAX_NATIVE_TICK;
    let mut at_native_max = configured();
    at_native_max["authority_expires_tick"] = json!(u64::MAX);
    at_native_max["clock"]["deadline_tick"] = json!(native_max);
    assert_eq!(
        WorkshopConfig::parse(&at_native_max.to_string())?
            .clock
            .ok_or_else(exhausted)?
            .deadline_tick,
        native_max
    );
    at_native_max["clock"]["deadline_tick"] = json!(native_max + 1);
    assert!(WorkshopConfig::parse(&at_native_max.to_string()).is_err());
    Ok(())
}

#[test]
fn clock_recovery_requires_exact_closed_external_pin_and_never_controls() -> Result<()> {
    let pin = json!({"journal_id":"c".repeat(64),"head":"d".repeat(64)});
    let mut missing = configured();
    missing["clock"]["initialize"] = json!(false);
    assert!(WorkshopConfig::parse(&missing.to_string()).is_err());
    let mut conflicting = configured();
    conflicting["clock"]["recovery"] = pin.clone();
    assert!(WorkshopConfig::parse(&conflicting.to_string()).is_err());

    let mut value = missing;
    value["initialize"] = json!(false);
    value["acknowledged"] = json!({
        "journal_id":"a".repeat(64),"head":"b".repeat(64),"frames":4
    });
    value["clock"]["recovery"] = pin;
    value["clock"]["recovery_generation"] = json!(12);
    let parsed = WorkshopConfig::parse(&value.to_string())?;
    let clock = parsed.clock.as_ref().ok_or_else(exhausted)?;
    let recovery = clock.recovery.ok_or_else(exhausted)?;
    assert_eq!(recovery.journal_id, digest(&"c".repeat(64))?);
    assert_eq!(recovery.head, digest(&"d".repeat(64))?);
    assert_eq!(clock.recovery_generation, 12);
    for mode in [
        WorkshopMode::Control,
        WorkshopMode::Recover,
        WorkshopMode::Offline,
    ] {
        assert!(!clock.control(mode));
    }
    for field in ["journal_id", "head"] {
        for bad in [
            "0".repeat(64),
            "A".repeat(64),
            "c".repeat(63),
            "x".repeat(64),
        ] {
            let mut malformed = value.clone();
            malformed["clock"]["recovery"][field] = json!(bad);
            assert!(
                WorkshopConfig::parse(&malformed.to_string()).is_err(),
                "{field}"
            );
        }
        let mut omitted = value.clone();
        omitted["clock"]["recovery"]
            .as_object_mut()
            .ok_or_else(exhausted)?
            .remove(field);
        assert!(
            WorkshopConfig::parse(&omitted.to_string()).is_err(),
            "missing {field}"
        );
    }
    let mut foreign_pin = value;
    foreign_pin["clock"]["recovery"]["frames"] = json!(4);
    assert!(WorkshopConfig::parse(&foreign_pin.to_string()).is_err());
    Ok(())
}

#[test]
fn run_request_requires_explicit_finite_ticks_wall_and_exact_target() -> Result<()> {
    let value = run_request();
    assert!(is_request(&value.to_string())?);
    let parsed = request(&value.to_string())?;
    assert_eq!(parsed.target_id, digest(&"a".repeat(64))?);
    assert_eq!((parsed.game_ticks, parsed.wall_millis), (1200, 60000));
    for (key, bad) in [
        ("game_ticks", 0),
        ("game_ticks", 1201),
        ("game_ticks", u32::MAX),
        ("wall_millis", 0),
        ("wall_millis", 60001),
        ("wall_millis", u32::MAX),
    ] {
        let mut changed = value.clone();
        changed[key] = json!(bad);
        assert!(request(&changed.to_string()).is_err(), "{key}={bad}");
    }
    for key in ["kind", "target_id", "game_ticks", "wall_millis"] {
        let mut changed = value.clone();
        changed.as_object_mut().ok_or_else(exhausted)?.remove(key);
        assert!(request(&changed.to_string()).is_err(), "missing {key}");
    }
    for bad in ["0".repeat(64), "A".repeat(64), "a".repeat(63)] {
        let mut changed = value.clone();
        changed["target_id"] = json!(bad);
        assert!(request(&changed.to_string()).is_err());
    }
    let mut minimum = value;
    minimum["game_ticks"] = json!(1);
    minimum["wall_millis"] = json!(1);
    assert!(request(&minimum.to_string()).is_ok());
    Ok(())
}

#[test]
fn run_request_rejects_native_selectors_and_preserves_workshop_routing() -> Result<()> {
    for key in [
        "endpoint",
        "command",
        "native_command",
        "token",
        "generation",
        "deadline_tick",
        "fortress_id",
        "building_id",
        "pause",
        "recovery",
    ] {
        let mut changed = run_request();
        changed[key] = json!("untrusted-selector");
        assert!(request(&changed.to_string()).is_err(), "{key}");
    }
    let original = json!({
        "kind":"carpenters","expected_observation":"b".repeat(64),
        "material":"workshop-material:42:source"
    })
    .to_string();
    assert!(!is_request(&original)?);
    assert!(request(&original).is_err());
    for raw in ["{", "null", "[]"] {
        assert!(request(raw).is_err());
    }
    let oversized = " ".repeat(2049);
    assert!(is_request(&oversized).is_err());
    assert!(request(&oversized).is_err());
    Ok(())
}
