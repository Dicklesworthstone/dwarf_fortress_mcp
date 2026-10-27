use super::*;
use dfmcp_core::{GameTick, RequestId, SessionId, WorkBudget};
use serde_json::{Value, json};

fn config() -> Value {
    json!({"schema":"dfmcp.semantic-workforce-development/1",
        "directory":"/tmp/semantic-workforce-clock-config", "endpoint":"127.0.0.1:5000", "initialize":true,
        "sources":{"kind":"operator-development-source-acceptance/1", "world_folder":"region1", "site_id":7,
            "df_version":"df", "dfhack_version":"dfhack", "citizen_bridge_version":"citizens",
            "citizen_generation":10, "workforce_generation":20}})
}
fn canonical(config: &Config) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(901),
        request_id: RequestId::new(902),
        anchor: StateAnchor {
            fortress_id: config.sources.fortress(),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(100),
            state_hash: Digest32::of_bytes(b"canonical-labor"),
        },
        budget: WorkBudget {
            max_bytes: 32 * MIB,
            max_entities: 8192,
            max_actions: 1,
            max_wall_millis: 5000,
            max_output_tokens: 32768,
            max_game_ticks: 1200,
        },
        grants: [
            Capability::Query,
            Capability::Observe,
            Capability::Plan,
            Capability::ConfigureLabor,
            Capability::ControlClock,
        ]
        .into_iter()
        .map(|capability| CapabilityGrant {
            capability,
            scope: CapabilityScope {
                fortress_id: Some(config.sources.fortress()),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::Guarded,
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
        cancellation_requested: false,
    }
}
#[test]
fn optional_clock_configuration_preserves_existing_configs_and_rejects_open_fields() -> Result<()> {
    let mut raw = config();
    assert!(Config::parse(&raw.to_string())?.clock.is_none());
    raw["clock"] = json!({"generation":30});
    assert!(Config::parse(&raw.to_string())?.clock.is_some());
    for clock in [
        json!({"generation":0}),
        json!({"generation":u64::MAX}),
        json!({"generation":30,"journal_id":null}),
        json!({"generation":30,"journal_id":"ab".repeat(32)}),
        json!({"generation":30,"fortress_id":"7"}),
        json!({"generation":30,"endpoint":"127.0.0.1:5001"}),
    ] {
        raw["clock"] = clock;
        assert!(Config::parse(&raw.to_string()).is_err());
    }
    Ok(())
}
#[test]
fn clock_recovery_and_original_handoff_require_the_same_external_identity() -> Result<()> {
    let mut raw = config();
    let parsed = Config::parse(&raw.to_string())?;
    raw["initialize"] = json!(false);
    raw["journal_id"] = json!("ab".repeat(32));
    raw["clock"] = json!({"generation":30});
    assert!(Config::parse(&raw.to_string()).is_err());
    raw["clock"]["journal_id"] = json!("cd".repeat(32));
    assert!(Config::parse(&raw.to_string()).is_ok());
    raw["recovery"] = json!({"journal_id":"ab".repeat(32),"plan_digest":"12".repeat(32),
        "object_digest":"34".repeat(32),"identity_digest":"56".repeat(32),
        "anchor":{"fortress_id":parsed.sources.fortress().get().to_string(),"epoch":0,"sequence":1,"tick":100,"state_hash":"78".repeat(32)}});
    assert!(Config::parse(&raw.to_string()).is_err());
    raw["recovery"]["clock_journal_id"] = json!("ef".repeat(32));
    assert!(Config::parse(&raw.to_string()).is_err());
    raw["recovery"]["clock_journal_id"] = raw["clock"]["journal_id"].clone();
    assert!(Config::parse(&raw.to_string()).is_ok());
    raw["clock"] = Value::Null;
    assert!(Config::parse(&raw.to_string()).is_err());
    Ok(())
}
#[test]
fn independent_source_context_preserves_allowances_and_cannot_assign_labor() -> Result<()> {
    let config = Config::parse(&config().to_string())?;
    let canonical = canonical(&config);
    let native = issue_context(&canonical, true)?;
    assert_eq!(native.anchor.fortress_id, FortressId::NIL);
    assert_eq!(native.anchor.state_hash, Digest32::ZERO);
    assert_eq!(native.budget, canonical.budget);
    assert_eq!(native.session_id, canonical.session_id);
    native.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
    assert!(
        native
            .authorize(Capability::ConfigureLabor, RiskTier::Guarded, &[], None)
            .is_err()
    );
    assert!(
        issue_context(&canonical, false)?
            .authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)
            .is_err()
    );
    let mut cancelled = canonical.clone();
    cancelled.cancellation_requested = true;
    assert!(issue_context(&cancelled, true)?.cancellation_requested);
    assert_eq!(canonical.anchor.fortress_id, config.sources.fortress());
    Ok(())
}
#[test]
fn exact_source_pins_do_not_conflate_independent_plugin_generations() -> Result<()> {
    let mut raw = config();
    raw["clock"] = json!({"generation":30});
    let config = Config::parse(&raw.to_string())?;
    let manifest = RunManifest {
        generation: 30,
        df_version: "df".into(),
        dfhack_version: "dfhack".into(),
    };
    identity(&config, &manifest, Some(config.endpoint))?;
    for generation in [10, 20, 29, 31] {
        assert!(
            identity(
                &config,
                &RunManifest {
                    generation,
                    ..manifest.clone()
                },
                Some(config.endpoint)
            )
            .is_err()
        );
    }
    assert!(identity(&config, &manifest, None).is_err());
    assert!(
        identity(
            &config,
            &RunManifest {
                df_version: "other".into(),
                ..manifest
            },
            Some(config.endpoint)
        )
        .is_err()
    );
    Ok(())
}
#[test]
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
fn exact_clock_file_creation_never_recreates_or_adopts_custody() -> Result<()> {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    let directory = std::env::temp_dir().join(format!(
        "dfmcp-clock-exact-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir(&directory).expect("test directory");
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).expect("test mode");
    let config = Config::parse(&config().to_string())?;
    let c = issue_context(&canonical(&config), true)?;
    let binding = RunBinding::new(
        config.endpoint,
        RunManifest {
            generation: 30,
            df_version: "df".into(),
            dfhack_version: "dfhack".into(),
        },
    )?;
    let path = directory.join("clock.journal");
    assert!(
        open_private_run_journal_exact(&path, &c, RunMode::Control, Some(binding.clone()), false)
            .is_err()
    );
    assert!(!path.exists());
    let journal =
        open_private_run_journal_exact(&path, &c, RunMode::Control, Some(binding.clone()), true)?;
    let id = journal.id();
    drop(journal);
    let bytes = fs::read(&path).expect("test journal");
    assert!(
        open_private_run_journal_exact(&path, &c, RunMode::Control, Some(binding.clone()), true)
            .is_err()
    );
    assert_eq!(fs::read(&path).expect("retained bytes"), bytes);
    let journal =
        open_private_run_journal_exact(&path, &c, RunMode::Offline, Some(binding), false)?;
    assert_eq!(journal.id(), id);
    drop(journal);
    fs::remove_file(&path).expect("test cleanup");
    assert!(open_private_run_journal_exact(&path, &c, RunMode::Control, None, false).is_err());
    assert!(!path.exists());
    fs::remove_dir(&directory).expect("test cleanup");
    Ok(())
}
fn reject_runtime() -> Result<()> {
    Err(dfmcp_core::DfmcpError::new(
        dfmcp_core::ErrorCode::CancellationRequested,
        "runtime clock checkpoint rejected",
    ))
}
#[test]
fn runtime_checkpoint_precedes_clock_custody_authority_and_native_contact() -> Result<()> {
    let mut raw = config();
    raw["clock"] = json!({"generation":30});
    let mut config = Config::parse(&raw.to_string())?;
    let c = canonical(&config);
    config.runtime_check = Some(reject_runtime);
    let config = Arc::new(config);
    assert_eq!(
        context(&config, &c, true).expect_err("checkpoint").code,
        dfmcp_core::ErrorCode::CancellationRequested
    );
    let native = issue_context(&c, true)?;
    let error = match open(&config, Mode::Control, &c) {
        Ok(_) => panic!("checkpoint bypass"),
        Err(e) => e,
    };
    assert_eq!(error.code, dfmcp_core::ErrorCode::CancellationRequested);
    let error = match connect(&config, &native) {
        Ok(_) => panic!("checkpoint bypass"),
        Err(e) => e,
    };
    assert_eq!(error.code, dfmcp_core::ErrorCode::CancellationRequested);
    Ok(())
}
