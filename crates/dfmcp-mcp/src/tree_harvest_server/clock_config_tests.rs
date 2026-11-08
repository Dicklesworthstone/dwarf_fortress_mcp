use super::*;

fn configured() -> Value {
    json!({
        "schema":"dfmcp.tree-harvest-mcp-development/1",
        "directory":"/tmp/dfmcp-tree-clock-consumer",
        "endpoint":"127.0.0.1:5000","initialize":true,"acknowledged":null,
        "world_folder":"region1","site_id":7,"df_version":"53.16",
        "dfhack_version":"53.16-r3","generation":3,"recovery_generation":null,
        "map_dimensions":[48,48,4],"root":[20,20,2],
        "read_scope":[0,0,0,47,47,3],"scope":[0,0,0,47,47,3],"protected":[],
        "authority_expires_tick":20000,"lease_ticks":1200,
        "checkpoint":"disposable-fortress-no-checkpoint",
        "clock":{"initialize":true,"generation":9,"recovery_generation":null,"recovery":null}
    })
}
fn run_request() -> Value {
    json!({"kind":"run_tree","target_id":"a".repeat(64),"game_ticks":1200,"wall_millis":60000})
}

#[test]
fn tree_clock_configuration_preserves_independent_generation_and_legacy_default() -> Result<()> {
    let value = configured();
    let config = TreeConfig::parse(&value.to_string())?;
    let clock = config.clock.as_ref().ok_or_else(exhausted)?;
    assert_eq!(config.binding.generation, 3);
    assert_eq!(clock.binding.manifest().generation, 9);
    assert_eq!(clock.recovery_generation, 9);
    assert_eq!(clock.binding.endpoint(), config.binding.endpoint);
    assert_eq!(clock.binding.manifest().df_version, config.input.df_version);
    assert_eq!(
        clock.binding.manifest().dfhack_version,
        config.input.dfhack_version
    );
    assert!(clock.control(Mode::Control));
    assert!(!clock.control(Mode::Recover));
    assert!(!clock.control(Mode::Offline));
    assert!(config.has_clock());
    let mut legacy = value;
    legacy
        .as_object_mut()
        .ok_or_else(exhausted)?
        .remove("clock");
    let legacy = TreeConfig::parse(&legacy.to_string())?;
    assert!(!legacy.has_clock());
    assert_eq!(legacy.input.root, [20, 20, 2]);
    Ok(())
}

#[test]
fn tree_clock_configuration_is_closed_and_inherits_the_original_deadline_only() -> Result<()> {
    for key in [
        "endpoint",
        "directory",
        "df_version",
        "token",
        "native_command",
        "tree_generation",
        "fortress_id",
        "root",
        "scope",
        "pause",
        "controller",
        "deadline_tick",
    ] {
        let mut value = configured();
        value["clock"][key] = json!("untrusted-selector");
        assert!(TreeConfig::parse(&value.to_string()).is_err(), "{key}");
    }
    for key in ["initialize", "generation"] {
        let mut value = configured();
        value["clock"]
            .as_object_mut()
            .ok_or_else(exhausted)?
            .remove(key);
        assert!(
            TreeConfig::parse(&value.to_string()).is_err(),
            "missing {key}"
        );
    }
    // Even a plausible finite replacement deadline is forbidden; the Tree
    // Original sealed before designation is the only clock horizon.
    let mut value = configured();
    value["clock"]["deadline_tick"] = json!(19000);
    assert!(TreeConfig::parse(&value.to_string()).is_err());
    Ok(())
}

#[test]
fn tree_clock_generation_pins_refuse_reserved_or_regressed_incarnations() -> Result<()> {
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
            TreeConfig::parse(&value.to_string()).is_err(),
            "{key}={bad}"
        );
    }
    for generation in [9, 12] {
        let mut value = configured();
        value["clock"]["recovery_generation"] = json!(generation);
        let config = TreeConfig::parse(&value.to_string())?;
        assert_eq!(
            config
                .clock
                .as_ref()
                .ok_or_else(exhausted)?
                .recovery_generation,
            generation
        );
        assert_eq!(
            config
                .clock
                .as_ref()
                .ok_or_else(exhausted)?
                .binding
                .manifest()
                .generation,
            9
        );
    }
    let mut equal = configured();
    equal["clock"]["generation"] = json!(3);
    assert!(TreeConfig::parse(&equal.to_string()).is_ok());
    Ok(())
}

#[test]
fn tree_clock_recovery_requires_exact_nonzero_pin_and_never_controls() -> Result<()> {
    let pin = json!({"journal_id":"c".repeat(64),"head":"d".repeat(64)});
    let mut missing = configured();
    missing["clock"]["initialize"] = json!(false);
    assert!(TreeConfig::parse(&missing.to_string()).is_err());
    let mut conflicting = configured();
    conflicting["clock"]["recovery"] = pin.clone();
    assert!(TreeConfig::parse(&conflicting.to_string()).is_err());
    let mut value = missing;
    value["initialize"] = json!(false);
    value["acknowledged"] = json!({"journal_id":"a".repeat(64),"head":"b".repeat(64),"frames":4});
    value["clock"]["recovery"] = pin;
    value["clock"]["recovery_generation"] = json!(12);
    let config = TreeConfig::parse(&value.to_string())?;
    let clock = config.clock.as_ref().ok_or_else(exhausted)?;
    assert_eq!(
        clock.recovery.ok_or_else(exhausted)?.journal_id,
        digest(&"c".repeat(64))?
    );
    assert_eq!(
        clock.recovery.ok_or_else(exhausted)?.head,
        digest(&"d".repeat(64))?
    );
    for mode in [Mode::Control, Mode::Recover, Mode::Offline] {
        assert!(!clock.control(mode));
    }
    for field in ["journal_id", "head"] {
        for bad in [
            "0".repeat(64),
            "A".repeat(64),
            "c".repeat(63),
            "x".repeat(64),
        ] {
            let mut invalid = value.clone();
            invalid["clock"]["recovery"][field] = json!(bad);
            assert!(TreeConfig::parse(&invalid.to_string()).is_err(), "{field}");
        }
        let mut omitted = value.clone();
        omitted["clock"]["recovery"]
            .as_object_mut()
            .ok_or_else(exhausted)?
            .remove(field);
        assert!(TreeConfig::parse(&omitted.to_string()).is_err());
    }
    value["clock"]["recovery"]["frames"] = json!(4);
    assert!(TreeConfig::parse(&value.to_string()).is_err());
    Ok(())
}

#[test]
fn tree_clock_authority_is_separate_from_original_designation_and_read_scope() -> Result<()> {
    let mut value = configured();
    value["initialize"] = json!(false);
    value["acknowledged"] = json!({"journal_id":"a".repeat(64),"head":"b".repeat(64),"frames":4});
    let config = TreeConfig::parse(&value.to_string())?;
    let mut budget = dfmcp_core::WorkBudget::default();
    budget.max_game_ticks = 1200;
    let context = super::super::context(
        &config,
        Mode::Control,
        dfmcp_core::SessionId::new(17),
        budget,
        1,
        100,
        Digest32::ZERO,
    );
    context.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
    context.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
    assert!(
        context
            .authorize(
                Capability::Designate,
                RiskTier::Irreversible,
                &[],
                Some(config.scope)
            )
            .is_err()
    );
    context.authorize(
        Capability::Query,
        RiskTier::ReadOnly,
        &[],
        Some(config.binding.read_scope()?),
    )?;
    let outside = dfmcp_core::MapCuboid::new(
        dfmcp_core::MapCoord::new(0, 0, 0),
        dfmcp_core::MapCoord::new(48, 48, 4),
    )?;
    assert!(
        context
            .authorize(Capability::Query, RiskTier::ReadOnly, &[], Some(outside))
            .is_err()
    );
    let native = native_context(&context, &native_grants(&config, true));
    assert_eq!(native.anchor.fortress_id, FortressId::NIL);
    assert_ne!(native.anchor.fortress_id, context.anchor.fortress_id);
    native.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
    let mut after_authority = native;
    after_authority.anchor.tick = GameTick(20001);
    after_authority.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    assert!(
        after_authority
            .authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)
            .is_err()
    );
    Ok(())
}

#[test]
fn tree_run_request_requires_explicit_finite_limits_and_exact_original_target() -> Result<()> {
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
        let mut value = value.clone();
        value[key] = json!(bad);
        assert!(request(&value.to_string()).is_err(), "{key}={bad}");
    }
    for bad in [
        "0".repeat(64),
        "A".repeat(64),
        "a".repeat(63),
        "x".repeat(64),
    ] {
        let mut value = value.clone();
        value["target_id"] = json!(bad);
        assert!(request(&value.to_string()).is_err());
    }
    for key in ["kind", "target_id", "game_ticks", "wall_millis"] {
        let mut value = value.clone();
        value.as_object_mut().ok_or_else(exhausted)?.remove(key);
        assert!(request(&value.to_string()).is_err(), "missing {key}");
    }
    Ok(())
}

#[test]
fn tree_run_request_cannot_supply_native_commands_or_replace_original_horizon() -> Result<()> {
    for key in [
        "deadline_tick",
        "endpoint",
        "root",
        "generation",
        "key",
        "plan_digest",
        "native_command",
        "pause",
        "repeat",
    ] {
        let mut value = run_request();
        value[key] = json!("untrusted");
        assert!(request(&value.to_string()).is_err(), "{key}");
    }
    assert!(!is_request(
        &json!({"kind":"fell_one_visible_tree"}).to_string()
    )?);
    assert!(!is_request(&json!({"kind":"run_workshop"}).to_string())?);
    assert!(request(&json!({"kind":"run_workshop"}).to_string()).is_err());
    assert!(is_request("{").is_err());
    assert!(is_request(&" ".repeat(2049)).is_err());
    Ok(())
}
