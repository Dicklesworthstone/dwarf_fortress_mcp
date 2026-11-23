//! Exact native codec vectors and pure host-policy checks; no live admission.
use super::*;

fn configured() -> Value {
    json!({"schema":"dfmcp.workshop-mcp-development/1",
        "directory":"/tmp/dfmcp-project-workshop","endpoint":"127.0.0.1:5000",
        "initialize":true,"acknowledged":null,"world_folder":"region1","site_id":7,
        "df_version":"53.16","dfhack_version":"53.16-r3","generation":7,
        "recovery_generation":null,"map_dimensions":[64,64,4],"item_id":42,
        "center":[15,15,2],"scope":[0,0,0,63,63,3],"protected":[],
        "authority_expires_tick":20000,"lease_ticks":1200,
        "checkpoint":"disposable-fortress-no-checkpoint"})
}
fn config(value: Value) -> Result<Config> {
    let mut config = Config::parse(&value.to_string())?;
    config.parent_check = Some(Arc::new(|| Ok(())));
    config.with_project_deadline(19000)
}
fn vector(name: &str) -> Result<Vec<u8>> {
    let vectors: Value = serde_json::from_str(include_str!(
        "../../../../tests/native/workshop/vectors.json"
    ))
    .map_err(|_| invalid("native vector JSON"))?;
    bytes(
        vectors[name].as_str().ok_or_else(|| invalid("native vector absent"))?,
        6144,
    )
}
fn handoff() -> Result<WorkshopProjectHandoff> {
    let record = WorkshopRecord::decode(&vector("placed")?)?;
    let progress = WorkshopProgress::decode(&vector("progress_complete")?)?;
    let binding = WorkshopBinding::from_source_pins(
        "127.0.0.1:5000".parse().map_err(|_| invalid("endpoint"))?,
        record.plan().before().fortress().clone(),
        record.plan().before().generation(),
        record.plan().before().dimensions(),
        "53.16",
        "53.16-r3",
    )?;
    let acknowledged = WorkshopPin {
        journal_id: Digest32::of_bytes(b"component original journal"),
        head: Digest32::of_bytes(b"component exact acknowledged head"),
        frames: 4,
    };
    let after = record.after().ok_or_else(|| invalid("Placed after capture"))?;
    let origin_high_tick = after.tick();
    let origin_high_building = after.next_building_id();
    let mut canonical = b"dfmcp-workshop-production-origin/1\0".to_vec();
    let binding_bytes = binding.encode();
    canonical.extend_from_slice(&(binding_bytes.len() as u32).to_be_bytes());
    canonical.extend_from_slice(&binding_bytes);
    canonical.extend_from_slice(acknowledged.journal_id.as_bytes());
    canonical.extend_from_slice(acknowledged.head.as_bytes());
    canonical.extend_from_slice(&acknowledged.frames.to_be_bytes());
    canonical.extend_from_slice(&(record.canonical_bytes().len() as u32).to_be_bytes());
    canonical.extend_from_slice(record.canonical_bytes());
    canonical.extend_from_slice(&origin_high_tick.to_be_bytes());
    canonical.extend_from_slice(&origin_high_building.to_be_bytes());
    Ok(WorkshopProjectHandoff {
        directory: PathBuf::from("/tmp/dfmcp-project-workshop"),
        binding,
        acknowledged,
        record,
        origin_witness: Digest32::of_bytes(&canonical),
        origin_high_tick,
        origin_high_building,
        high_tick: progress.tick(),
        high_generation: progress.generation(),
        high_building: progress.next_building_id(),
        progress,
        project_deadline_tick: 19000,
        authority_expires_tick: 20000,
        clock_deadline_tick: None,
    })
}

#[test]
fn project_policy_keeps_original_material_source_authority_and_deadline() -> Result<()> {
    let original = config(configured())?;
    let identity = original.project_identity();
    let mut recovery = configured();
    recovery["initialize"] = json!(false);
    recovery["acknowledged"] = json!({
        "journal_id":"a".repeat(64),"head":"b".repeat(64),"frames":4
    });
    recovery["recovery_generation"] = json!(8);
    assert_eq!(identity, config(recovery)?.project_identity());
    for (field, changed) in [
        ("directory", json!("/tmp/another-workshop")),
        ("item_id", json!(43)),
        ("generation", json!(8)),
        ("dfhack_version", json!("53.16-r4")),
        ("authority_expires_tick", json!(21000)),
        ("lease_ticks", json!(1199)),
        ("checkpoint", json!("required")),
    ] {
        let mut value = configured();
        value[field] = changed;
        assert_ne!(identity, config(value)?.project_identity(), "{field}");
    }
    assert_eq!(original.project_binding()?.generation(), 7);
    assert_eq!(original.project_directory(), Path::new("/tmp/dfmcp-project-workshop"));
    assert!(original.with_project_deadline(19001).is_err());
    Ok(())
}

#[test]
fn project_deadline_cannot_shorten_reconciliation_authority_or_renew_clock() -> Result<()> {
    let standalone = Config::parse(&configured().to_string())?;
    assert!(standalone.with_project_deadline(19000).is_err());
    let bound = config(configured())?;
    assert!(bound.project_effect_deadline(18999).is_ok());
    assert!(bound.project_effect_deadline(19000).is_err());
    let grants = Runtime::grants(&bound, true, true);
    for grant in &grants {
        assert_eq!(
            grant.expires_at_tick,
            Some(GameTick(if grant.capability == Capability::Construct {
                19000
            } else {
                20000
            }))
        );
    }
    assert!(grants.iter().any(|grant| grant.capability == Capability::ControlClock));
    let mut value = configured();
    value["clock"] = json!({
        "initialize":true,"generation":9,"recovery_generation":null,
        "deadline_tick":19000,"recovery":null
    });
    let clock = config(value.clone())?;
    assert_eq!(clock.clock_deadline(), Some(19000));
    value["clock"]["deadline_tick"] = json!(18000);
    assert!(config(value).is_err());
    Ok(())
}

#[test]
fn parent_remaining_budget_narrows_every_dimension_without_restarting_wall_time() -> Result<()> {
    let ceiling = WorkBudget {
        max_wall_millis: 60000,
        max_game_ticks: 1200,
        max_entities: 128,
        max_bytes: 512 * 1024 * 1024,
        max_output_tokens: 10000,
        max_actions: 1,
    };
    let remaining = WorkBudget {
        max_wall_millis: 49,
        max_game_ticks: 0,
        max_entities: 3,
        max_bytes: 1234,
        max_output_tokens: 77,
        max_actions: 8,
    };
    let actual = narrow(ceiling, remaining)?;
    assert_eq!(actual, WorkBudget { max_actions: 1, ..remaining });
    assert_eq!(narrow(remaining, ceiling)?, actual);
    assert!(narrow(ceiling, WorkBudget { max_wall_millis: 0, ..remaining }).is_err());
    assert!(narrow(ceiling, WorkBudget { max_actions: 0, ..remaining }).is_err());
    let elapsed = Instant::now().checked_sub(Duration::from_millis(100))
        .ok_or_else(|| invalid("component monotone clock"))?;
    assert!(narrow_current(ceiling, remaining, elapsed).is_err());
    Ok(())
}

#[test]
fn project_handoff_roundtrip_retains_exact_original_and_consumed_floors() -> Result<()> {
    let original = handoff()?;
    original.validate()?;
    let json = original.to_json();
    assert!(json.to_string().len() < HANDOFF_BYTES);
    let restored = WorkshopProjectHandoff::from_json(&json)?;
    assert_eq!(restored.to_json(), json);
    assert_eq!(restored.original_key(), "workshop-001");
    assert_eq!(restored.original_plan_digest(), original.record.plan().digest());
    assert_eq!(restored.origin_witness(), original.origin_witness());
    assert_eq!(restored.high_tick(), original.progress.tick());
    assert_eq!(restored.high_generation(), original.binding.generation());
    assert_eq!(restored.high_next_building_id(), original.progress.next_building_id());
    assert_eq!(restored.project_deadline_tick(), 19000);
    assert_eq!(restored.authority_expires_tick(), 20000);
    assert_eq!(restored.clock_deadline_tick(), None);
    assert!(!json.to_string().contains("current_proof"));
    Ok(())
}

#[test]
fn project_handoff_refuses_rebound_custody_source_and_unqualified_completion() -> Result<()> {
    let original = handoff()?.to_json();
    for (field, changed) in [
        ("directory", json!("/tmp/../other")),
        ("origin_witness", json!("a".repeat(64))),
        ("high_tick", json!(1)),
        ("high_generation", json!(8)),
        ("high_next_building_id", json!(1)),
        ("project_deadline_tick", json!(1)),
        ("authority_expires_tick", json!(100)),
        ("clock_deadline_tick", json!(18000)),
        ("record_hex", json!(hex(&vector("prepared")?))),
        ("progress_hex", json!(hex(&vector("progress_stage0")?))),
    ] {
        let mut changed_value = original.clone();
        changed_value[field] = changed;
        assert!(WorkshopProjectHandoff::from_json(&changed_value).is_err(), "{field}");
    }
    let mut rebound = original.clone();
    rebound["acknowledged"]["head"] = json!("a".repeat(64));
    assert!(WorkshopProjectHandoff::from_json(&rebound).is_err());
    let mut flag = original;
    flag["currently_complete"] = json!(true);
    assert!(WorkshopProjectHandoff::from_json(&flag).is_err());
    Ok(())
}

#[test]
fn recovered_parent_frontier_merges_verified_descendants_without_erasing_fences() -> Result<()> {
    let mut state = State {
        high_generation: 7,
        high_building: 140,
        fenced: true,
    };
    let tick = state.retain(
        17000,
        FrontierInput {
            high_tick: 16000,
            high_generation: 7,
            high_next_building_id: 130,
            fenced: false,
        },
        7,
    )?;
    assert_eq!(tick, 17000);
    assert_eq!((state.high_generation, state.high_building), (7, 140));
    assert!(state.fenced);
    let tick = state.retain(
        tick,
        FrontierInput {
            high_tick: 18000,
            high_generation: 8,
            high_next_building_id: 150,
            fenced: false,
        },
        7,
    )?;
    assert_eq!(tick, 18000);
    assert_eq!((state.high_generation, state.high_building), (8, 150));
    assert!(state.fenced);
    let before = state.reference(tick);
    assert!(state.retain(
        tick,
        FrontierInput {
            high_tick: u64::MAX,
            high_generation: 9,
            high_next_building_id: 151,
            fenced: false,
        },
        7,
    ).is_err());
    assert_eq!(state.reference(tick), before);
    Ok(())
}

#[test]
fn restart_selectors_must_match_parent_ack_and_clock_checkpoint() -> Result<()> {
    let pin = json!({"journal_id":"a".repeat(64),"head":"b".repeat(64),"frames":4});
    let clock_pin = json!({"journal_id":"c".repeat(64),"head":"d".repeat(64)});
    let mut value = configured();
    value["initialize"] = json!(false);
    value["acknowledged"] = pin.clone();
    let reference = json!({"acknowledged":pin,"clock":null});
    let original = config(value.clone())?;
    original.project_recovery_pins(&reference)?;
    for field in ["journal_id", "head", "frames"] {
        let mut changed = reference.clone();
        changed["acknowledged"][field] = if field == "frames" {
            json!(3)
        } else {
            json!("e".repeat(64))
        };
        assert!(original.project_recovery_pins(&changed).is_err(), "{field}");
    }
    value["clock"] = json!({
        "initialize":false,"generation":9,"recovery_generation":9,
        "deadline_tick":19000,"recovery":clock_pin
    });
    let with_clock = config(value.clone())?;
    let mut clock_reference = reference.clone();
    clock_reference["clock"] = json!({
        "enabled":true,"recovery":clock_pin,"advancement_fenced":true
    });
    with_clock.project_recovery_pins(&clock_reference)?;
    assert!(with_clock.project_recovery_pins(&reference).is_err());
    assert!(original.project_recovery_pins(&clock_reference).is_err());
    let mut older_clock = clock_reference.clone();
    older_clock["clock"]["recovery"]["head"] = json!("e".repeat(64));
    assert!(with_clock.project_recovery_pins(&older_clock).is_err());
    value["clock"]["initialize"] = json!(true);
    value["clock"]["recovery"] = Value::Null;
    assert!(config(value)
        .and_then(|configured| configured.project_recovery_pins(&clock_reference))
        .is_err());
    Ok(())
}

#[test]
fn parent_floor_withdraws_workshop_offer_without_relabeling_native_anchor_or_custody()
-> Result<()> {
    let fixture = super::super::production_handoff_tests::completed_original_fixture()?;
    let config = config(fixture.configuration())?;
    let observed = WorkshopCapture::decode(&vector("capture")?)?;
    let observed_anchor = anchor(&observed);
    let budget = WorkBudget {
        max_wall_millis: 60000,
        max_bytes: 512 * 1024 * 1024,
        ..WorkBudget::CONSERVATIVE_DEFAULT
    };
    let session = SessionId::new(713);
    let initial = OperationContext {
        session_id: session,
        request_id: RequestId::new(1),
        anchor: observed_anchor,
        budget,
        grants: Runtime::grants(&config, false, false),
        cancellation_requested: false,
    };
    let acks = open_private_acknowledgements(
        &config.input.directory.join("workshop.ack"),
        &initial,
        WorkshopMode::Offline,
        false,
        config.pin,
    )?;
    let journal = open_private_workshop(
        &config.input.directory.join("workshop.journal"),
        &initial,
        WorkshopMode::Offline,
        None,
        [0; 32],
        acks.latest(),
    )?;
    let authority_tick = journal.high_tick().max(observed.tick());
    let guard = Guard {
        config: config.clone(),
        selection: Some(observed.selection()),
        acks: Arc::new(Mutex::new(acks)),
        leases: LeaseManager::new(),
        lease: None,
        session,
    };
    let mut runtime = Runtime {
        session,
        mode: WorkshopMode::Offline,
        budget,
        config,
        journal,
        guard,
        source: None,
        observation: Some(observed.clone()),
        tree_material: None,
        clock: None,
        anchor: Some(observed_anchor),
        authority_tick,
        next_request: 2,
        project: State::default(),
    };
    let reference = runtime.project_recovery_reference();
    let files = ["workshop.journal", "workshop.ack"];
    let before = files
        .iter()
        .map(|name| std::fs::read(runtime.config.input.directory.join(name)))
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(|_| invalid("read original Workshop custody"))?;
    runtime.retain_project_authority_tick(authority_tick)?;
    assert_eq!(
        runtime.observation.as_ref().map(WorkshopCapture::witness),
        Some(observed.witness())
    );
    let parent_tick = authority_tick + 100;
    runtime.retain_project_authority_tick(parent_tick)?;
    assert!(runtime.observation.is_none());
    assert!(runtime.source.is_none());
    assert_eq!(runtime.anchor, Some(observed_anchor));
    let current = runtime.context()?;
    assert_eq!(current.anchor.tick.get(), parent_tick);
    assert_eq!(current.anchor.state_hash, observed.witness());
    assert_eq!(runtime.anchor, Some(observed_anchor));
    let after = runtime.project_recovery_reference();
    assert_eq!(after["original"], reference["original"]);
    assert_eq!(after["acknowledged"], reference["acknowledged"]);
    assert_eq!(after["project_source_frontier"]["high_tick"], parent_tick);
    runtime.retain_project_authority_tick(authority_tick)?;
    assert_eq!(runtime.authority_tick, parent_tick);
    assert!(
        runtime
            .retain_project_authority_tick(dfmcp_adapter::bounded_run::MAX_NATIVE_TICK + 1)
            .is_err()
    );
    assert_eq!(runtime.authority_tick, parent_tick);
    runtime.retain_project_authority_tick(runtime.config.input.authority_expires_tick + 1)?;
    assert!(
        runtime
            .context()?
            .authorize(Capability::Query, RiskTier::ReadOnly, &[], None)
            .is_err()
    );
    assert_eq!(runtime.anchor, Some(observed_anchor));
    for (name, expected) in files.into_iter().zip(before) {
        assert_eq!(
            std::fs::read(runtime.config.input.directory.join(name))
                .map_err(|_| invalid("read retained Workshop custody"))?,
            expected
        );
    }
    Ok(())
}

#[test]
fn resource_workshop_open_rejects_unbound_policy_or_invalid_floor_before_io() -> Result<()> {
    let standalone = Config::parse(&configured().to_string())?;
    let bound = config(configured())?;
    for (config, floor) in [
        (standalone, 100),
        (bound, dfmcp_adapter::bounded_run::MAX_NATIVE_TICK + 1),
    ] {
        let result = Runtime::open_for_resource_project(
            config,
            WorkshopMode::Control,
            SessionId::new(713),
            WorkBudget::CONSERVATIVE_DEFAULT,
            floor,
        );
        assert_eq!(
            result.err().map(|error| error.code),
            Some(ErrorCode::CapabilityDenied)
        );
    }
    Ok(())
}
