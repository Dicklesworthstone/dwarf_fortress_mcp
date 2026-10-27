use super::*;
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};

fn fixture() -> Value {
    json!({
        "schema":"dfmcp.semantic-workforce-development/1",
        "directory":"/tmp/dfmcp-semantic-workforce-test",
        "endpoint":"127.0.0.1:5000",
        "initialize":true,
        "sources":{
            "kind":"operator-development-source-acceptance/1",
            "world_folder":"region1","site_id":7,
            "df_version":"df","dfhack_version":"dfhack",
            "citizen_bridge_version":"citizens","citizen_generation":10,
            "workforce_generation":20
        }
    })
}
#[test]
fn closed_config_preserves_independent_sources_and_exact_recovery() -> Result<()> {
    let mut raw = fixture();
    let config = Config::parse(&raw.to_string())?;
    assert_eq!(config.sources.citizen_generation, 10);
    assert_eq!(config.sources.workforce_generation, 20);
    assert!(config.runtime_check.is_none());
    let binding = config.binding()?;
    assert_eq!(binding.manifest().generation, 20);
    assert_eq!(binding.endpoint().to_string(), "127.0.0.1:5000");
    assert!(
        config
            .sources
            .native_manifest(&WorkforceManifest {
                generation: 10,
                df_version: "df".into(),
                dfhack_version: "dfhack".into(),
            })
            .is_err()
    );
    let journal = Digest32::of_bytes(b"externally retained original journal").to_string();
    raw["journal_id"] = json!(journal);
    assert!(
        Config::parse(&raw.to_string()).is_err(),
        "initialize must not adopt custody"
    );
    raw["initialize"] = json!(false);
    let config = Config::parse(&raw.to_string())?;
    assert!(config.journal_id.is_some());
    raw["recovery"] = json!({
        "journal_id":journal,
        "plan_digest":Digest32::of_bytes(b"original plan").to_string(),
        "object_digest":Digest32::of_bytes(b"original archive").to_string(),
        "identity_digest":Digest32::of_bytes(b"original citizen capsule").to_string(),
        "anchor":{
            "fortress_id":config.sources.fortress().get().to_string(),
            "epoch":0,"sequence":1,"tick":50,
            "state_hash":Digest32::of_bytes(b"original anchor").to_string()
        },
        "history_present":true
    });
    let recovered = Config::parse(&raw.to_string())?;
    assert!(
        recovered
            .recovery
            .is_some_and(|value| value.history_present)
    );
    raw["recovery"]["journal_id"] = json!(Digest32::of_bytes(b"different journal").to_string());
    assert!(Config::parse(&raw.to_string()).is_err());
    raw["recovery"]["journal_id"] = raw["journal_id"].clone();
    raw["recovery"]["anchor"]["fortress_id"] = json!("1");
    assert!(
        Config::parse(&raw.to_string()).is_err(),
        "external fortress cannot be replaced"
    );
    raw["recovery"] = Value::Null;
    raw.as_object_mut()
        .expect("fixture object")
        .remove("journal_id");
    assert!(
        Config::parse(&raw.to_string()).is_err(),
        "reopen needs external journal identity"
    );
    Ok(())
}

#[test]
fn operator_config_refuses_unbounded_unknown_and_client_authority_fields() {
    for (field, value) in [
        ("runtime_check", json!("client callback")),
        ("production_admitted", json!(true)),
        ("credential", json!("client credential")),
        ("directory", json!("relative/path")),
        ("endpoint", json!("192.0.2.1:5000")),
        ("endpoint", json!("127.0.0.1:0")),
        ("schema", json!("dfmcp.semantic-work-order-development/1")),
    ] {
        let mut raw = fixture();
        raw[field] = value;
        assert!(Config::parse(&raw.to_string()).is_err(), "refused {field}");
    }
    for field in ["citizen_generation", "workforce_generation"] {
        for generation in [0, u64::MAX] {
            let mut raw = fixture();
            raw["sources"][field] = json!(generation);
            assert!(Config::parse(&raw.to_string()).is_err());
        }
    }
    let mut raw = fixture();
    raw["sources"]["field_grants"] = json!(["all"]);
    assert!(Config::parse(&raw.to_string()).is_err());
    assert!(Config::parse("").is_err());
    assert!(Config::parse(&" ".repeat(MAX_CONFIG_BYTES + 1)).is_err());
}

#[cfg(unix)]
#[test]
fn original_custody_cannot_be_hidden_by_omitting_recovery_or_history() -> Result<()> {
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("fixture clock")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "dfmcp-workforce-custody-preflight-{}-{unique}",
        std::process::id(),
    ));
    fs::create_dir(&directory).expect("fixture directory");
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).expect("fixture mode");
    let mut raw = fixture();
    raw["directory"] = json!(directory);
    let mut config = Config::parse(&raw.to_string())?;
    config.original_custody()?;
    fs::write(config.path("original.capsule"), b"partial identity").expect("fixture bytes");
    assert!(
        config.original_custody().is_err(),
        "orphan identity cannot be hidden"
    );
    let digest = Digest32::of_bytes(b"external original handoff");
    config.recovery = Some(Recovery {
        seal: PlanArchiveSeal {
            plan_digest: digest,
            object_digest: digest,
        },
        identity_digest: digest,
        anchor: StateAnchor {
            fortress_id: config.sources.fortress(),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(0),
            state_hash: digest,
        },
        history_present: false,
    });
    assert!(
        config.original_custody().is_err(),
        "exact recovery requires both originals"
    );
    fs::write(config.path("original.plan"), b"plan bytes").expect("fixture bytes");
    config.original_custody()?; // Actual store still must validate exact bytes/seals.
    fs::write(config.path("goal.history"), b"retained history").expect("fixture bytes");
    assert!(
        config.original_custody().is_err(),
        "history flag cannot hide an existing history"
    );
    config
        .recovery
        .as_mut()
        .expect("fixture recovery")
        .history_present = true;
    config.original_custody()?;
    config.recovery = None;
    assert!(
        config.original_custody().is_err(),
        "full original history cannot be hidden"
    );
    let linked = directory.join("linked");
    fs::create_dir(&linked).expect("fixture link directory");
    symlink(
        directory.join("original.capsule"),
        linked.join("original.capsule"),
    )
    .expect("fixture link");
    config.directory = linked;
    assert!(
        config.original_custody().is_err(),
        "symbolic links are not original custody"
    );
    Ok(())
}

static CHECKS: AtomicUsize = AtomicUsize::new(0);
fn reject_runtime() -> Result<()> {
    CHECKS.fetch_add(1, Ordering::SeqCst);
    Err(DfmcpError::new(
        ErrorCode::CancellationRequested,
        "installed runtime checkpoint rejected workforce access",
    ))
}
fn rejected<T>(result: Result<T>) {
    let cause = match result {
        Ok(_) => panic!("installed runtime checkpoint was bypassed"),
        Err(cause) => cause,
    };
    assert_eq!(cause.code, ErrorCode::CancellationRequested);
    assert_eq!(
        cause.message,
        "installed runtime checkpoint rejected workforce access"
    );
}
#[test]
fn runtime_checkpoint_precedes_configuration_credentials_custody_and_native_io() -> Result<()> {
    CHECKS.store(0, Ordering::SeqCst);
    let listener = TcpListener::bind("127.0.0.1:0").expect("fixture listener");
    listener.set_nonblocking(true).expect("nonblocking fixture");
    let directory = std::env::temp_dir().join(format!(
        "dfmcp-workforce-runtime-check-{}-{}",
        std::process::id(),
        listener.local_addr().expect("fixture address").port(),
    ));
    assert!(!directory.exists());
    let mut raw = fixture();
    raw["directory"] = json!(directory);
    raw["endpoint"] = json!(listener.local_addr().expect("fixture address").to_string());
    let config = Config::parse(&raw.to_string())?;
    let c = OperationContext {
        session_id: SessionId::new(901),
        request_id: RequestId::new(902),
        anchor: StateAnchor {
            fortress_id: config.sources.fortress(),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(0),
            state_hash: Digest32::ZERO,
        },
        budget: WorkBudget {
            max_bytes: MAX_BYTES,
            max_entities: 8192,
            max_actions: 1,
            max_wall_millis: 5000,
            max_output_tokens: MAX_OUTPUT_TOKENS,
            max_game_ticks: 0,
        },
        grants: vec![],
        cancellation_requested: false,
    };
    rejected(NativeRuntime::open_with_runtime_check(
        config.clone(),
        Mode::Control,
        c.session_id,
        c.budget,
        reject_runtime,
    ));
    let mut installed = config;
    installed.runtime_check = Some(reject_runtime);
    let binding = installed.binding()?;
    let shell = NativeShell {
        config: Arc::new(installed.clone()),
    };
    rejected(installed.current());
    rejected(shell.check_environment());
    rejected(shell.open_control(Mode::Control, &c));
    rejected(shell.native(&binding, &c));
    rejected(shell.acquire(&c));
    rejected(shell.recover_identity(Digest32::of_bytes(b"identity"), &c));
    assert_eq!(CHECKS.load(Ordering::SeqCst), 7);
    assert!(
        !directory.exists(),
        "runtime checkpoint rejected before custody creation"
    );
    assert!(matches!(listener.accept(), Err(cause)
        if cause.kind() == std::io::ErrorKind::WouldBlock));
    Ok(())
}
