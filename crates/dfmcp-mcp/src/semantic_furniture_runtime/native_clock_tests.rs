use super::*;
use dfmcp_core::{GameTick, RequestId, SessionId, WorkBudget};
use serde_json::{Value, json};
use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

fn config() -> Value {
    json!({"schema":"dfmcp.semantic-furniture-development/1",
        "directory":"/tmp/semantic-furniture-clock-config", "endpoint":"127.0.0.1:5000", "initialize":true,
        "sources":{"kind":"operator-development-source-acceptance/1", "world_folder":"region1", "site_id":7,
            "df_version":"df", "dfhack_version":"dfhack", "map_dimensions":[32,32,8],
            "spatial_generation":10, "furniture_generation":20, "operations_generation":30},
        "scope":[0,0,0,31,31,7], "checkpoint":"disposable-fortress-no-checkpoint"})
}
fn canonical(config: &Config) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(901),
        request_id: RequestId::new(902),
        anchor: StateAnchor {
            fortress_id: config.sources.fortress(),
            cursor: ObservationCursor {
                epoch: 3,
                sequence: 4,
            },
            tick: GameTick(100),
            state_hash: Digest32::of_bytes(b"canonical-original-construction"),
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
            Capability::Construct,
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
fn optional_clock_configuration_is_closed_and_requires_exact_incarnation() -> Result<()> {
    let mut raw = config();
    assert!(Config::parse(&raw.to_string())?.clock.is_none());
    raw["clock"] = json!({"generation":40});
    assert!(Config::parse(&raw.to_string())?.clock.is_some());
    for clock in [
        json!({"generation":0}),
        json!({"generation":u64::MAX}),
        json!({"generation":40,"journal_id":null}),
        json!({"generation":40,"journal_id":"ab".repeat(32)}),
        json!({"generation":40,"fortress_id":"7"}),
        json!({"generation":40,"endpoint":"127.0.0.1:5001"}),
        json!({"generation":40,"allow_control":true}),
    ] {
        raw["clock"] = clock;
        assert!(Config::parse(&raw.to_string()).is_err());
    }
    Ok(())
}
#[test]
fn clock_recovery_requires_the_original_external_journal_identity() -> Result<()> {
    let mut raw = config();
    let parsed = Config::parse(&raw.to_string())?;
    raw["initialize"] = json!(false);
    raw["journal_id"] = json!("ab".repeat(32));
    raw["clock"] = json!({"generation":40});
    assert!(Config::parse(&raw.to_string()).is_err());
    for id in ["00".repeat(32), "CD".repeat(32), "cd".repeat(31)] {
        raw["clock"]["journal_id"] = json!(id);
        assert!(Config::parse(&raw.to_string()).is_err());
    }
    raw["clock"]["journal_id"] = json!("cd".repeat(32));
    assert!(Config::parse(&raw.to_string()).is_ok());
    raw["recovery"] = json!({"journal_id":"ab".repeat(32),"plan_digest":"12".repeat(32),
        "object_digest":"34".repeat(32),
        "anchor":{"fortress_id":parsed.sources.fortress().get().to_string(),"epoch":0,"sequence":1,"tick":100,"state_hash":"78".repeat(32)}});
    assert!(Config::parse(&raw.to_string()).is_err());
    raw["recovery"]["clock_journal_id"] = Value::Null;
    assert!(Config::parse(&raw.to_string()).is_err());
    raw["recovery"]["clock_journal_id"] = json!("ef".repeat(32));
    assert!(Config::parse(&raw.to_string()).is_err());
    raw["recovery"]["clock_journal_id"] = raw["clock"]["journal_id"].clone();
    assert!(Config::parse(&raw.to_string()).is_ok());
    raw["clock"] = Value::Null;
    assert!(Config::parse(&raw.to_string()).is_err());
    raw["recovery"]["clock_journal_id"] = Value::Null;
    assert!(
        Config::parse(&raw.to_string()).is_ok(),
        "legacy no-clock original recovery remains valid"
    );
    Ok(())
}
#[test]
fn independent_source_context_preserves_allowances_without_construction_authority() -> Result<()> {
    let config = Config::parse(&config().to_string())?;
    let canonical = canonical(&config);
    let native = issue_context(&canonical, true)?;
    assert_eq!(native.anchor.fortress_id, FortressId::NIL);
    assert_eq!(native.anchor.state_hash, Digest32::ZERO);
    assert_eq!(native.anchor.cursor, ObservationCursor::ORIGIN);
    assert_eq!(native.anchor.tick, canonical.anchor.tick);
    assert_eq!(native.budget, canonical.budget);
    assert_eq!(native.session_id, canonical.session_id);
    assert_eq!(native.request_id, canonical.request_id);
    native.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
    for capability in [
        Capability::Construct,
        Capability::Observe,
        Capability::ConfigureLabor,
    ] {
        assert!(
            native
                .authorize(capability, RiskTier::Guarded, &[], None)
                .is_err()
        );
    }
    assert!(
        issue_context(&canonical, false)?
            .authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)
            .is_err()
    );
    let mut cancelled = canonical.clone();
    cancelled.cancellation_requested = true;
    let cancelled = issue_context(&cancelled, true)?;
    assert!(cancelled.cancellation_requested);
    assert!(
        cancelled
            .authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)
            .is_err()
    );
    assert_eq!(canonical.anchor.fortress_id, config.sources.fortress());
    Ok(())
}
#[test]
fn exact_clock_source_pins_keep_all_four_plugin_generations_independent() -> Result<()> {
    let mut raw = config();
    raw["clock"] = json!({"generation":40});
    let config = Config::parse(&raw.to_string())?;
    let manifest = RunManifest {
        generation: 40,
        df_version: "df".into(),
        dfhack_version: "dfhack".into(),
    };
    identity(&config, &manifest, Some(config.endpoint))?;
    for generation in [10, 20, 30, 39, 41] {
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
            &manifest,
            Some(SocketAddr::from(([127, 0, 0, 1], 5001)))
        )
        .is_err()
    );
    assert!(
        identity(
            &config,
            &RunManifest {
                df_version: "other".into(),
                ..manifest.clone()
            },
            Some(config.endpoint)
        )
        .is_err()
    );
    assert!(
        identity(
            &config,
            &RunManifest {
                dfhack_version: "other".into(),
                ..manifest
            },
            Some(config.endpoint)
        )
        .is_err()
    );
    Ok(())
}

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);
struct Directory(PathBuf);
impl Directory {
    fn create() -> std::io::Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-semantic-furniture-clock-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        }
        Ok(Self(path))
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
#[cfg(unix)]
fn clock_custody_preflight_rejects_hidden_missing_and_linked_originals()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;
    let directory = Directory::create()?;
    let mut raw = config();
    raw["directory"] = json!(directory.0);
    Config::parse(&raw.to_string())?.original_custody()?;
    fs::write(directory.0.join("clock.journal"), b"clock")?;
    assert!(Config::parse(&raw.to_string())?.original_custody().is_err());
    assert!(!directory.0.join("furniture.journal").exists());
    assert!(!directory.0.join("semantic.associations").exists());
    raw["clock"] = json!({"generation":40});
    assert!(
        Config::parse(&raw.to_string())?.original_custody().is_err(),
        "initialize cannot adopt existing clock custody"
    );
    fs::remove_file(directory.0.join("clock.journal"))?;
    Config::parse(&raw.to_string())?.original_custody()?;
    fs::write(directory.0.join("furniture.journal"), b"furniture")?;
    fs::write(directory.0.join("semantic.associations"), b"associations")?;
    raw["initialize"] = json!(false);
    raw["journal_id"] = json!("ab".repeat(32));
    raw["clock"]["journal_id"] = json!("cd".repeat(32));
    assert!(
        Config::parse(&raw.to_string())?.original_custody().is_err(),
        "clock reopening cannot recreate missing custody"
    );
    fs::write(directory.0.join("clock.journal"), b"clock")?;
    Config::parse(&raw.to_string())?.original_custody()?;
    fs::remove_file(directory.0.join("clock.journal"))?;
    symlink(
        directory.0.join("furniture.journal"),
        directory.0.join("clock.journal"),
    )?;
    assert!(Config::parse(&raw.to_string())?.original_custody().is_err());
    assert_eq!(
        fs::read(directory.0.join("furniture.journal"))?,
        b"furniture"
    );
    assert_eq!(
        fs::read(directory.0.join("semantic.associations"))?,
        b"associations"
    );
    Ok(())
}
#[test]
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
fn exact_clock_file_creation_never_recreates_or_adopts_custody()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let directory = Directory::create()?;
    let config = Config::parse(&config().to_string())?;
    let c = issue_context(&canonical(&config), true)?;
    let binding = RunBinding::new(
        config.endpoint,
        RunManifest {
            generation: 40,
            df_version: "df".into(),
            dfhack_version: "dfhack".into(),
        },
    )?;
    let path = directory.0.join("clock.journal");
    assert!(
        open_private_run_journal_exact(&path, &c, RunMode::Control, Some(binding.clone()), false)
            .is_err()
    );
    assert!(!path.exists());
    let journal =
        open_private_run_journal_exact(&path, &c, RunMode::Control, Some(binding.clone()), true)?;
    let id = journal.id();
    assert!(
        open_private_run_journal_exact(&path, &c, RunMode::Offline, None, false).is_err(),
        "only one descriptor may hold exact custody"
    );
    drop(journal);
    let bytes = fs::read(&path)?;
    assert!(
        open_private_run_journal_exact(&path, &c, RunMode::Control, Some(binding.clone()), true)
            .is_err()
    );
    assert_eq!(fs::read(&path)?, bytes);
    let wrong_binding = RunBinding::new(
        config.endpoint,
        RunManifest {
            generation: 41,
            df_version: "df".into(),
            dfhack_version: "dfhack".into(),
        },
    )?;
    assert!(
        open_private_run_journal_exact(&path, &c, RunMode::Offline, Some(wrong_binding), false)
            .is_err()
    );
    assert_eq!(fs::read(&path)?, bytes);
    let mut journal =
        open_private_run_journal_exact(&path, &c, RunMode::Offline, Some(binding), false)?;
    assert_eq!(journal.id(), id);
    assert!(journal.records(&c)?.is_empty());
    drop(journal);
    fs::remove_file(&path)?;
    assert!(open_private_run_journal_exact(&path, &c, RunMode::Control, None, false).is_err());
    assert!(!path.exists());
    Ok(())
}
fn reject_runtime() -> Result<()> {
    Err(dfmcp_core::DfmcpError::new(
        dfmcp_core::ErrorCode::CancellationRequested,
        "runtime clock checkpoint rejected",
    ))
}
#[test]
fn runtime_checkpoint_precedes_clock_custody_authority_and_native_contact()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let directory = Directory::create()?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let mut raw = config();
    raw["directory"] = json!(directory.0);
    raw["endpoint"] = json!(listener.local_addr()?.to_string());
    raw["clock"] = json!({"generation":40});
    let mut config = Config::parse(&raw.to_string())?;
    let c = canonical(&config);
    config.runtime_check = Some(reject_runtime);
    let config = Arc::new(config);
    let failure = context(&config, &c, true)
        .err()
        .ok_or_else(|| invalid("runtime checkpoint was bypassed by clock context"))?;
    assert_eq!(failure.code, dfmcp_core::ErrorCode::CancellationRequested);
    let failure = open(&config, Mode::Control, &c)
        .err()
        .ok_or_else(|| invalid("runtime checkpoint was bypassed by clock custody"))?;
    assert_eq!(failure.code, dfmcp_core::ErrorCode::CancellationRequested);
    let native = issue_context(&c, true)?;
    let failure = connect(&config, &native)
        .err()
        .ok_or_else(|| invalid("runtime checkpoint was bypassed by clock connection"))?;
    assert_eq!(failure.code, dfmcp_core::ErrorCode::CancellationRequested);
    assert_eq!(fs::read_dir(&directory.0)?.count(), 0);
    let failure = listener
        .accept()
        .err()
        .ok_or_else(|| invalid("clock contacted native after runtime cancellation"))?;
    assert_eq!(failure.kind(), std::io::ErrorKind::WouldBlock);
    Ok(())
}
