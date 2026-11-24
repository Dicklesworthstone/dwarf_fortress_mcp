//! Pure policy and recovery-floor mechanics; no native completion is minted.
use super::*;
use serde_json::{Value, json};

fn policies() -> [Value; 3] {
    let common = json!({
        "kind":"operator-development-source-acceptance/1", "world_folder":"region1",
        "site_id":4, "df_version":"0.51.11", "dfhack_version":"0.51.11-r1",
        "citizen_bridge_version":"citizen-roster/1.0", "citizen_generation":23
    });
    let mut workforce = common.clone();
    workforce["workforce_generation"] = json!(31);
    let mut orders = common;
    orders["work_order_generation"] = json!(41);
    orders["progress_generation"] = json!(51);
    orders["operations_generation"] = json!(61);
    [
        json!({"schema":"dfmcp.combined-production-development/1", "directory":"/tmp/project-production", "initialize":true,
            "clock":{"generation":71}, "inventory_clock":{"generation":81},
            "input_supply":{"schema":"dfmcp.furniture-input-supply/1", "reserve_wood_stack_units":2}}),
        json!({"schema":"dfmcp.semantic-workforce-development/1", "directory":"/tmp/project-workers", "endpoint":"127.0.0.1:5000",
            "initialize":true, "sources":workforce}),
        json!({"schema":"dfmcp.semantic-work-order-development/1", "directory":"/tmp/project-orders", "endpoint":"127.0.0.1:5000",
            "initialize":true, "sources":orders, "clock":{"generation":71}, "inventory_clock":{"generation":81}}),
    ]
}

fn raw(values: &[Value; 3]) -> [String; 3] {
    values.each_ref().map(Value::to_string)
}

#[test]
fn project_policy_restart_keeps_sources_but_separately_checks_exact_config_bytes() -> Result<()> {
    let original = policies();
    let initial = raw(&original);
    parse_with_workshop(&initial[0], None, true)?;
    workforce::native::Config::parse(&initial[1])?;
    orders::native::Config::parse(&initial[2])?;
    let expected = project_policy_identity(&initial)?;
    let exact = initial
        .each_ref()
        .map(|value| Digest32::of_bytes(value.as_bytes()));
    let mut reopened = original;
    let id = Digest32::of_bytes(b"externally retained restart identity").to_hex();
    for source in &mut reopened {
        source["initialize"] = false.into();
    }
    reopened[1]["journal_id"] = json!(id);
    for index in [0, 2] {
        reopened[index]["clock"]["journal_id"] = json!(id);
        reopened[index]["inventory_clock"]["journal_id"] = json!(id);
    }
    // These fields are validated against parent-held custody by the opening
    // owner. Policy normalization may not confuse them with source changes.
    reopened[0]["recovery"] = json!({"retained_external_checkpoint":"mechanics only"});
    reopened[0]["workshop_prerequisite_checkpoint"] = json!({"tick":130});
    reopened[0]["manager_prerequisite_checkpoint"] = json!({"tick":130});
    reopened[0]["input_supply_checkpoint"] = json!({"tick":130});
    let reopened_raw = raw(&reopened);
    assert_eq!(project_policy_identity(&reopened_raw)?, expected);
    assert!(
        !matches_digests(&reopened_raw, exact),
        "live configuration cannot be changed in place"
    );
    let encode = |value: &Value| {
        serde_json::to_string_pretty(value)
            .map_err(|_| super::invalid("test policy serialization failed"))
    };
    let formatted = [encode(&reopened[0])?, encode(&reopened[1])?, encode(&reopened[2])?];
    assert_eq!(project_policy_identity(&formatted)?, expected);
    Ok(())
}

#[test]
fn project_policy_identity_retains_all_nonrecovery_source_and_input_fields() -> Result<()> {
    let original = policies();
    let expected = project_policy_identity(&raw(&original))?;
    for index in 0..3 {
        let mut changed = original.clone();
        changed[index]["directory"] = json!(format!("/tmp/other-child-{index}"));
        assert_ne!(project_policy_identity(&raw(&changed))?, expected);
    }
    for index in [1, 2] {
        for (field, value) in [
            ("world_folder", json!("region2")),
            ("site_id", json!(5)),
            ("df_version", json!("changed-df")),
            ("dfhack_version", json!("changed-dfhack")),
            ("citizen_bridge_version", json!("changed-citizen-bridge")),
            ("citizen_generation", json!(24)),
        ] {
            let mut changed = original.clone();
            changed[index]["sources"][field] = value;
            assert_ne!(
                project_policy_identity(&raw(&changed))?,
                expected,
                "source {index}/{field}"
            );
        }
        let mut changed = original.clone();
        changed[index]["endpoint"] = json!("127.0.0.1:5001");
        assert_ne!(project_policy_identity(&raw(&changed))?, expected);
    }
    for (index, field) in [
        (1, "workforce_generation"),
        (2, "work_order_generation"),
        (2, "progress_generation"),
        (2, "operations_generation"),
    ] {
        let mut changed = original.clone();
        changed[index]["sources"][field] = json!(99);
        assert_ne!(project_policy_identity(&raw(&changed))?, expected, "{field}");
    }
    for clock in ["clock", "inventory_clock"] {
        for index in [0, 2] {
            let mut changed = original.clone();
            changed[index][clock]["generation"] = json!(99);
            assert_ne!(project_policy_identity(&raw(&changed))?, expected);
        }
    }
    let mut changed = original;
    changed[0]["input_supply"]["reserve_wood_stack_units"] = json!(3);
    assert_ne!(project_policy_identity(&raw(&changed))?, expected);
    Ok(())
}

#[test]
fn project_policy_cannot_import_a_caller_selected_workshop_donor() -> Result<()> {
    let mut value = policies()[0].clone();
    let id = Digest32::of_bytes(b"independent donor pins").to_hex();
    value["workshop_prerequisite"] = json!({
        "directory":"/tmp/other-workshop", "endpoint":"127.0.0.1:5000",
        "world_folder":"region1", "site_id":4, "df_version":"0.51.11", "dfhack_version":"0.51.11-r1",
        "generation":97, "map_dimensions":[48,48,8],
        "acknowledged":{"journal_id":id, "head":id, "frames":2},
        "original_key":"workshop-original", "original_plan_digest":id
    });
    assert!(
        parse(&value.to_string())?.5.is_some(),
        "ordinary donor configuration remains supported"
    );
    assert!(parse_with_workshop(&value.to_string(), None, true).is_err());
    Ok(())
}

#[test]
fn project_handoff_floor_cannot_regress_tick_generation_or_allocator() -> Result<()> {
    let seed = WorkshopRecoveryFloor {
        tick: 130,
        generation: 97,
        next_building_id: 201,
        fenced: false,
    };
    require_project_floor(seed, seed)?;
    for lower in [
        WorkshopRecoveryFloor { tick: 129, ..seed },
        WorkshopRecoveryFloor { generation: 96, ..seed },
        WorkshopRecoveryFloor { next_building_id: 200, ..seed },
    ] {
        assert!(require_project_floor(lower, seed).is_err());
    }
    let later = WorkshopRecoveryFloor {
        tick: 150,
        generation: 98,
        next_building_id: 202,
        fenced: true,
    };
    require_project_floor(later, seed)?;
    later.validate(seed.generation, 150)?;
    assert!(
        WorkshopRecoveryFloor {
            fenced: false,
            ..later
        }
        .validate(seed.generation, 150)
        .is_err()
    );
    Ok(())
}

#[test]
fn workshop_preflight_joins_common_fortress_without_equating_family_generations() -> Result<()> {
    let values = policies();
    let workforce = serde_json::from_value::<workforce::policy::SourceAcceptance>(
        values[1]["sources"].clone(),
    )
    .map_err(|_| super::invalid("test workforce pins decoding failed"))?;
    let orders =
        serde_json::from_value::<orders::policy::SourceAcceptance>(values[2]["sources"].clone())
            .map_err(|_| super::invalid("test order pins decoding failed"))?;
    let policy = ProjectProductionPolicy {
        identity: project_policy_identity(&raw(&values))?,
        directory: "/tmp/project-production".into(),
        workforce_directory: "/tmp/project-workers".into(),
        order_directory: "/tmp/project-orders".into(),
        initialize: true,
        workforce,
        orders,
    };
    let binding = |folder: &str, site: u32, df: &str, dfhack: &str, generation| {
        WorkshopBinding::from_source_pins(
            "127.0.0.1:5002".parse().map_err(|_| super::invalid("test loopback address decoding failed"))?,
            dfmcp_adapter::workshop_construction::FortressIdentity::new(folder, site)?,
            generation,
            [48, 48, 8],
            df,
            dfhack,
        )
    };
    policy.matches_workshop(&binding("region1", 4, "0.51.11", "0.51.11-r1", 97)?)?;
    for different in [
        binding("region2", 4, "0.51.11", "0.51.11-r1", 97)?,
        binding("region1", 5, "0.51.11", "0.51.11-r1", 97)?,
        binding("region1", 4, "changed-df", "0.51.11-r1", 97)?,
        binding("region1", 4, "0.51.11", "changed-dfhack", 97)?,
    ] {
        assert!(policy.matches_workshop(&different).is_err());
    }
    Ok(())
}

fn acknowledged_portfolio_policies(
    first: u32,
    clock: bool,
    inventory_clock: bool,
) -> Result<[Value; 3]> {
    let mut values = policies();
    for value in &mut values {
        value["initialize"] = json!(false);
    }
    values[0]
        .as_object_mut()
        .ok_or_else(|| invalid("test production policy is not an object"))?
        .remove("input_supply");
    values[1]["journal_id"] = json!(Digest32::of_bytes(b"retained workforce journal").to_hex());
    let id = |family: &str, step: u32| {
        Digest32::of_bytes(format!("retained project {family} {step}").as_bytes()).to_hex()
    };
    for (family, enabled, generation) in [
        ("clock", clock, 71),
        ("inventory_clock", inventory_clock, 81),
    ] {
        for index in [0, 2] {
            values[index][family] = if enabled {
                json!({"generation":generation, "journal_id":id(family, first)})
            } else {
                Value::Null
            };
        }
    }
    let workforce =
        serde_json::from_value::<workforce::policy::SourceAcceptance>(values[1]["sources"].clone())
            .map_err(|_| invalid("test workforce source decoding failed"))?;
    let seal = Digest32::of_bytes(b"externally acknowledged project original").to_hex();
    let slots: Vec<_> = (0..4)
        .map(|index| {
            let step = first + index;
            json!({
                "step_id":step, "goal_history_present":index % 2 == 1,
                "clock_journal_id":clock.then(|| id("clock", step)),
                "inventory_clock_journal_id":inventory_clock.then(|| id("inventory_clock", step)),
            })
        })
        .collect();
    values[0]["recovery"] = json!({
        "plan_digest":seal, "object_digest":seal, "identity_digest":seal,
        "anchor":{"fortress_id":workforce.fortress().get().to_string(),
            "epoch":0, "sequence":2, "tick":100, "state_hash":seal},
        "frontier":{"journal_id":seal, "head":seal, "authority_tick_floor":150},
        "goal_history_present":false,
        "clock_journal_id":slots[0]["clock_journal_id"],
        "inventory_clock_journal_id":slots[0]["inventory_clock_journal_id"],
        "portfolio":slots,
    });
    Ok(values)
}

#[test]
fn project_configuration_reopens_acknowledged_clocked_portfolios_without_effect_authority() -> Result<()>
{
    const CHILD: &str = "PROJECT_CONFIG_PORTFOLIO_REOPEN_CASE";
    const CHECKED: &str = "project-config-portfolio-reopen-checked";
    if let Ok(case) = std::env::var(CHILD) {
        let refuse_parent = case == "parent-refusal";
        let parent_guard: Arc<dyn Fn() -> Result<()> + Send + Sync> = Arc::new(move || {
            if refuse_parent {
                Err(denied("test parent authority withdrawn"))
            } else {
                Ok(())
            }
        });
        let runtime_check: fn() -> Result<()> = if case == "runtime-refusal" {
            || Err(denied("test runtime cancelled"))
        } else {
            || Ok(())
        };
        // Exercise the configuration entry point itself: parsing the recovery
        // alone previously passed while this later seam refused every clocked
        // portfolio. No live owner, journal or native source is opened here.
        let configured =
            Config::project_configuration(parent_guard, runtime_check, None, None, false);
        if case == "accepted" {
            let (config, _) = configured?;
            config.current()?;
            assert!(!config.initialize);
            let recovery = config
                .recovery
                .ok_or_else(|| invalid("test acknowledged recovery missing"))?;
            let slots = recovery
                .portfolio
                .ok_or_else(|| invalid("test acknowledged portfolio missing"))?;
            assert_eq!(slots.into_iter().flatten().count(), 4);
            assert_eq!(
                recovery.clock_journal_id,
                config.clock.and_then(|clock| clock.journal_id)
            );
            assert_eq!(
                recovery.inventory_clock_journal_id,
                config.inventory_clock.and_then(|clock| clock.journal_id)
            );
            assert!(!workforce::Shell::labor_enabled(&config.workforce));
            assert!(!orders::Shell::production_enabled(&config.orders));
            assert!(!orders::Shell::clock_enabled(&config.orders));
            assert!(!orders::Shell::inventory_clock_enabled(&config.orders));
        } else {
            assert!(configured.is_err(), "invalid reopen accepted: {case}");
        }
        println!("{CHECKED}");
        return Ok(());
    }

    let full_name = format!(
        "{}::project_configuration_reopens_acknowledged_clocked_portfolios_without_effect_authority",
        module_path!()
    );
    let (_, test_name) = full_name
        .split_once("::")
        .ok_or_else(|| invalid("test project configuration name missing"))?;
    let executable =
        std::env::current_exe().map_err(|_| invalid("test executable path unavailable"))?;
    let run = |case: &str, values: [Value; 3]| -> Result<()> {
        let raw = raw(&values);
        let mut command = std::process::Command::new(&executable);
        command.args(["--exact", test_name, "--nocapture"]);
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("DFMCP_") {
                command.env_remove(key);
            }
        }
        command
            .env(OPT_IN, "1")
            .env("DFMCP_ALLOW_UNADMITTED_SEMANTIC_WORKFORCE", "1")
            .env("DFMCP_ALLOW_UNADMITTED_SEMANTIC_WORK_ORDERS", "1")
            .env(CONFIG, &raw[0])
            .env(WORKFORCE_CONFIG, &raw[1])
            .env(ORDER_CONFIG, &raw[2])
            .env(CHILD, case);
        if case == "missing-opt-in" {
            command.env_remove(OPT_IN);
        }
        if case == "invalid-effect" {
            command.env("DFMCP_SEMANTIC_WORK_ORDER_ALLOW_CLOCK", "yes");
        }
        let output = command
            .output()
            .map_err(|_| invalid("test project configuration subprocess failed"))?;
        assert!(
            output.status.success(),
            "project configuration {case}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains(CHECKED));
        Ok(())
    };
    for first in [0, 1] {
        for (clock, inventory_clock) in [(true, false), (false, true), (true, true)] {
            run(
                "accepted",
                acknowledged_portfolio_policies(first, clock, inventory_clock)?,
            )?;
        }
    }
    let retained = acknowledged_portfolio_policies(1, true, true)?;
    for case in [
        "first-alias",
        "native-identity",
        "inactive-family",
        "missing-opt-in",
        "invalid-effect",
        "parent-refusal",
        "runtime-refusal",
    ] {
        let mut invalid = retained.clone();
        match case {
            "first-alias" => {
                invalid[0]["recovery"]["clock_journal_id"] =
                    json!(Digest32::of_bytes(b"foreign first-slot alias").to_hex());
            }
            "native-identity" => {
                invalid[2]["clock"]["journal_id"] =
                    json!(Digest32::of_bytes(b"foreign native clock").to_hex());
            }
            "inactive-family" => {
                invalid[0]["recovery"]["portfolio"][3]["inventory_clock_journal_id"] = Value::Null;
            }
            _ => {}
        }
        run(case, invalid)?;
    }
    Ok(())
}
