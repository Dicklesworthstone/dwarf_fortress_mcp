use super::*;
use dfmcp_world::{EvidenceCoverage, Predicate};
use std::fs;
use std::net::TcpListener;
use std::sync::atomic::{AtomicU64, AtomicUsize};

use super::super::tests::fixtures;
fn capture() -> Result<DigObservation> {
    fixtures::capture()
}
fn binding(config: &Config) -> Result<DigBinding> {
    let before = capture()?;
    DigBinding::new(
        config.endpoint,
        DigManifest {
            generation: before.generation(),
            df_version: "df".to_owned(),
            dfhack_version: "dfhack".to_owned(),
        },
        &before,
        config.scope,
    )
}
fn fixture() -> Result<Value> {
    let capture = capture()?;
    Ok(json!({
        "schema":"dfmcp.semantic-dig-development/1",
        "directory":"/tmp/dfmcp-semantic-dig-test",
        "endpoint":"127.0.0.1:5000",
        "initialize":true,
        "sources":{
            "kind":"operator-development-source-acceptance/1",
            "world_folder":capture.folder(),"site_id":capture.site(),
            "df_version":"df","dfhack_version":"dfhack",
            "spatial_generation":801,"dig_generation":capture.generation(),
            "map_dimensions":capture.dimensions()
        },
        "scope":[0,0,0,31,31,7],
        "protected":[],
        "checkpoint":"disposable-fortress-no-checkpoint"
    }))
}
fn recovered(raw: &mut Value, history: bool) -> Result<()> {
    let config = Config::parse(&raw.to_string())?;
    let journal = Digest32::of_bytes(b"external original journal").to_string();
    raw["initialize"] = json!(false);
    raw["journal_id"] = json!(journal);
    raw["recovery"] = json!({
        "journal_id":journal,
        "plan_digest":Digest32::of_bytes(b"original plan").to_string(),
        "object_digest":Digest32::of_bytes(b"original archive").to_string(),
        "anchor":{
            "fortress_id":config.sources.fortress().get().to_string(),
            "epoch":0,"sequence":1,"tick":capture()?.tick(),
            "state_hash":Digest32::of_bytes(b"original spatial anchor").to_string()
        },
        "history_present":history,
        "archive_publication_unknown":true,
        "paired_preparation_present":false
    });
    Ok(())
}
fn context(anchor: StateAnchor) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(901),
        request_id: RequestId::new(902),
        anchor,
        budget: WorkBudget {
            max_bytes: MAX_BYTES,
            max_entities: 65536,
            max_actions: 1,
            max_wall_millis: 60_000,
            max_output_tokens: MAX_OUTPUT_TOKENS,
            max_game_ticks: 0,
        },
        grants: [
            Capability::Query,
            Capability::Observe,
            Capability::Plan,
            Capability::Designate,
        ]
        .into_iter()
        .map(|capability| CapabilityGrant {
            capability,
            max_risk: RiskTier::Guarded,
            scope: CapabilityScope {
                fortress_id: Some(anchor.fortress_id),
                ..CapabilityScope::default()
            },
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
        cancellation_requested: false,
    }
}
fn native_context() -> Result<OperationContext> {
    let capture = capture()?;
    Ok(context(StateAnchor {
        fortress_id: capture.fortress_id(),
        cursor: ObservationCursor::ORIGIN,
        tick: GameTick(capture.tick()),
        state_hash: capture.witness(),
    }))
}

#[test]
fn closed_config_preserves_independent_sources_and_complete_recovery_handoff() -> Result<()> {
    let mut raw = fixture()?;
    let config = Config::parse(&raw.to_string())?;
    assert_eq!(config.sources.spatial_generation, 801);
    assert_eq!(config.sources.dig_generation, capture()?.generation());
    assert_ne!(
        config.sources.spatial_generation,
        config.sources.dig_generation
    );
    assert!(config.runtime_check.is_none());
    assert!(config.recovery.is_none());
    recovered(&mut raw, true)?;
    let recovery = Config::parse(&raw.to_string())?
        .recovery
        .ok_or_else(|| invalid("fixture recovery missing"))?;
    assert!(recovery.history_present);
    assert_eq!(recovery.anchor.fortress_id, config.sources.fortress());
    raw["recovery"]["journal_id"] = json!(Digest32::of_bytes(b"other journal").to_string());
    assert!(Config::parse(&raw.to_string()).is_err());
    raw["recovery"]["journal_id"] = raw["journal_id"].clone();
    raw["recovery"]["anchor"]["fortress_id"] = json!("0");
    assert!(Config::parse(&raw.to_string()).is_err());
    raw["recovery"]["anchor"]["fortress_id"] =
        json!(format!("0{}", config.sources.fortress().get()));
    assert!(Config::parse(&raw.to_string()).is_err());
    raw["recovery"] = Value::Null;
    raw["journal_id"] = Value::Null;
    assert!(
        Config::parse(&raw.to_string()).is_err(),
        "reopen requires external journal identity"
    );
    raw["initialize"] = json!(true);
    raw["journal_id"] = json!(Digest32::of_bytes(b"cannot adopt on initialization").to_string());
    assert!(Config::parse(&raw.to_string()).is_err());
    Ok(())
}

#[test]
fn operator_configuration_rejects_unknown_authority_and_unbounded_scopes() -> Result<()> {
    for (field, value) in [
        ("runtime_check", json!("client callback")),
        ("production_admitted", json!(true)),
        ("credential", json!("client credential")),
        ("directory", json!("relative")),
        ("directory", json!("/tmp//custody")),
        ("directory", json!("/tmp/../custody")),
        ("endpoint", json!("192.0.2.1:5000")),
        ("endpoint", json!("[::1]:5000")),
        ("endpoint", json!("127.0.0.1:0")),
        ("scope", json!([0, 0, 0, 63, 63, 7])),
        ("scope", json!([2, 0, 0, 1, 1, 1])),
        ("scope", json!([0, 0, 0, 64, 1, 1])),
        ("checkpoint", json!("already backed up")),
        ("schema", json!("dfmcp.semantic-workforce-development/1")),
    ] {
        let mut raw = fixture()?;
        raw[field] = value;
        assert!(Config::parse(&raw.to_string()).is_err(), "refused {field}");
    }
    for field in ["spatial_generation", "dig_generation"] {
        for generation in [0, u64::MAX] {
            let mut raw = fixture()?;
            raw["sources"][field] = json!(generation);
            assert!(Config::parse(&raw.to_string()).is_err());
        }
    }
    let mut raw = fixture()?;
    raw["sources"]["field_grants"] = json!(["all"]);
    assert!(Config::parse(&raw.to_string()).is_err());
    let mut raw = fixture()?;
    raw["protected"] = json!(vec![[0, 0, 0, 0, 0, 0]; 33]);
    assert!(Config::parse(&raw.to_string()).is_err());
    assert!(Config::parse("").is_err());
    assert!(Config::parse(&" ".repeat(MAX_CONFIG_BYTES + 1)).is_err());
    Ok(())
}

#[test]
fn scope_contains_complete_three_dimensional_halo_and_shared_block_writes() -> Result<()> {
    let mut raw = fixture()?;
    raw["scope"] = json!([0, 0, 1, 31, 31, 3]);
    let config = Config::parse(&raw.to_string())?;
    config.selection(capture()?.region())?;
    config.bootstrap_region()?;
    raw["scope"] = json!([14, 14, 1, 17, 17, 3]);
    let config = Config::parse(&raw.to_string())?;
    assert!(
        config.selection(capture()?.region()).is_err(),
        "the tiny target halo cannot authorize whole shared-block writes"
    );
    assert!(config.bootstrap_region().is_err());
    raw["scope"] = json!([0, 0, 2, 31, 31, 2]);
    let config = Config::parse(&raw.to_string())?;
    assert!(
        config.selection(capture()?.region()).is_err(),
        "vertical halo is mandatory"
    );
    Ok(())
}
#[test]
fn native_pins_remain_independent_and_validate_exact_scope_software_and_fortress() -> Result<()> {
    let config = Config::parse(&fixture()?.to_string())?;
    let before = capture()?;
    let retained = binding(&config)?;
    config.binding(&retained)?;
    config.sources.capture(&before)?;
    let mut wrong = config.sources.clone();
    wrong.dig_generation = config.sources.spatial_generation;
    assert!(wrong.binding(&retained).is_err());
    assert!(wrong.capture(&before).is_err());
    let mut wrong = config.sources.clone();
    wrong.world_folder = "other fortress".to_owned();
    assert!(wrong.binding(&retained).is_err());
    let bad = DigBinding::new(
        config.endpoint,
        DigManifest {
            generation: before.generation(),
            df_version: "different".to_owned(),
            dfhack_version: "dfhack".to_owned(),
        },
        &before,
        config.scope,
    )?;
    assert!(config.binding(&bad).is_err());
    let bad = DigBinding::new(
        SocketAddr::from(([127, 0, 0, 1], 5001)),
        retained.manifest().clone(),
        &before,
        config.scope,
    )?;
    assert!(config.binding(&bad).is_err());
    let bad = DigBinding::new(
        config.endpoint,
        retained.manifest().clone(),
        &before,
        MapCuboid::new(MapCoord::new(0, 0, 0), MapCoord::new(63, 63, 7))?,
    )?;
    assert!(config.binding(&bad).is_err());
    Ok(())
}
#[test]
fn terrain_policy_qualifies_only_exact_source_domains_and_unknown_stays_unknown() -> Result<()> {
    let f = fixtures::Fixture::new()?;
    let p = f.original_state;
    let raw = p.observation().spatial().terrain();
    let sources = SourceAcceptance {
        kind: super::super::policy::ACCEPTANCE_KIND.to_owned(),
        world_folder: raw.world_folder.clone(),
        site_id: raw.site_id,
        df_version: raw.df_version.clone(),
        dfhack_version: raw.dfhack_version.clone(),
        spatial_generation: raw.bridge_generation,
        dig_generation: f.capture.generation(),
        map_dimensions: raw.map_dimensions,
    };
    let c = context(p.anchor());
    let policy = sources.spatial(&p, raw.map.region, &c)?;
    assert!(policy.paused);
    assert!(policy.sources.is_empty());
    assert_eq!(policy.all_entities, EvidenceCoverage::Unknown);
    assert_eq!(policy.all_edges, EvidenceCoverage::Unknown);
    assert!(policy.entity_kinds.is_empty());
    assert!(!policy.terrain_regions.is_empty());
    assert!(p.evidence(policy)?.establishes(&Predicate::Paused(true))?);
    let mut denied_c = c.clone();
    denied_c
        .grants
        .retain(|g| g.capability != Capability::Observe);
    assert!(sources.spatial(&p, raw.map.region, &denied_c).is_err());
    denied_c = c.clone();
    denied_c.anchor.state_hash = Digest32::of_bytes(b"different source");
    assert!(sources.spatial(&p, raw.map.region, &denied_c).is_err());
    denied_c = c;
    denied_c.cancellation_requested = true;
    assert!(sources.spatial(&p, raw.map.region, &denied_c).is_err());
    let mut wrong = sources;
    wrong.spatial_generation = f.capture.generation();
    assert!(wrong.observation(p.observation(), raw.map.region).is_err());
    Ok(())
}

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);
struct Directory(PathBuf);
impl Directory {
    fn create() -> std::io::Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-semantic-dig-native-{}-{}",
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

#[cfg(unix)]
#[test]
fn original_custody_requires_exact_pair_and_visible_original_history()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;
    let directory = Directory::create()?;
    let mut raw = fixture()?;
    raw["directory"] = json!(directory.0);
    Config::parse(&raw.to_string())?.original_custody()?;
    fs::write(directory.0.join("dig.journal"), b"journal")?;
    assert!(Config::parse(&raw.to_string())?.original_custody().is_err());
    fs::write(directory.0.join("semantic.associations"), b"association")?;
    assert!(
        Config::parse(&raw.to_string())?.original_custody().is_err(),
        "initialize cannot adopt existing journal/association"
    );
    raw["initialize"] = json!(false);
    raw["journal_id"] = json!(Digest32::of_bytes(b"journal").to_string());
    Config::parse(&raw.to_string())?.original_custody()?;
    fs::write(directory.0.join("original.plan"), b"original")?;
    assert!(Config::parse(&raw.to_string())?.original_custody().is_err());
    raw["initialize"] = json!(true);
    raw["journal_id"] = Value::Null;
    recovered(&mut raw, false)?;
    Config::parse(&raw.to_string())?.original_custody()?;
    fs::write(directory.0.join("goal.history"), b"historical achievement")?;
    assert!(Config::parse(&raw.to_string())?.original_custody().is_err());
    raw["recovery"]["history_present"] = json!(true);
    Config::parse(&raw.to_string())?.original_custody()?;
    fs::remove_file(directory.0.join("original.plan"))?;
    assert!(
        Config::parse(&raw.to_string())?.original_custody().is_err(),
        "publication ambiguity does not allow a missing original archive"
    );
    symlink(
        directory.0.join("goal.history"),
        directory.0.join("original.plan"),
    )?;
    assert!(Config::parse(&raw.to_string())?.original_custody().is_err());
    Ok(())
}

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#[test]
fn exact_private_opener_cannot_adopt_existing_or_create_on_reopen()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::PermissionsExt;
    let directory = Directory::create()?;
    let path = directory.0.join("dig.journal");
    let c = native_context()?;
    let binding = binding(&Config::parse(&fixture()?.to_string())?)?;
    assert!(
        open_private_dig_exact(&path, &c, DigMode::Control, Some(binding.clone()), false).is_err()
    );
    assert!(
        !path.exists(),
        "reopen with a supplied binding must not create custody"
    );
    let owner = open_private_dig_exact(&path, &c, DigMode::Control, Some(binding.clone()), true)?;
    let id = owner.id();
    assert!(
        open_private_dig_exact(&path, &c, DigMode::Offline, None, false).is_err(),
        "existing exclusive descriptor remains the only owner"
    );
    drop(owner);
    let bytes = fs::read(&path)?;
    assert!(
        open_private_dig_exact(&path, &c, DigMode::Control, Some(binding.clone()), true).is_err()
    );
    assert_eq!(fs::read(&path)?, bytes);
    let reopened = open_private_dig_exact(&path, &c, DigMode::Control, None, false)?;
    assert_eq!(reopened.id(), id);
    drop(reopened);
    fs::remove_file(&path)?;
    fs::write(&path, [])?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    assert!(open_private_dig_exact(&path, &c, DigMode::Control, Some(binding), true).is_err());
    assert_eq!(
        fs::metadata(&path)?.len(),
        0,
        "existing empty file is never initialized or repaired"
    );
    Ok(())
}

static RUNTIME_CHECKS: AtomicUsize = AtomicUsize::new(0);
fn reject_runtime() -> Result<()> {
    RUNTIME_CHECKS.fetch_add(1, Ordering::SeqCst);
    Err(DfmcpError::new(
        ErrorCode::CancellationRequested,
        "runtime checkpoint rejected dig access",
    ))
}
fn runtime_rejected<T>(value: Result<T>) -> Result<()> {
    match value {
        Err(cause) if cause.code == ErrorCode::CancellationRequested => Ok(()),
        _ => Err(invalid("runtime checkpoint was bypassed")),
    }
}
#[test]
fn runtime_checkpoint_precedes_config_credentials_custody_and_native_io()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    RUNTIME_CHECKS.store(0, Ordering::SeqCst);
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let directory = Directory::create()?;
    let mut raw = fixture()?;
    raw["directory"] = json!(directory.0);
    raw["endpoint"] = json!(listener.local_addr()?.to_string());
    let mut config = Config::parse(&raw.to_string())?;
    let c = native_context()?;
    runtime_rejected(NativeRuntime::open_with_runtime_check(
        config.clone(),
        Mode::Control,
        c.session_id,
        c.budget,
        reject_runtime,
    ))?;
    config.runtime_check = Some(reject_runtime);
    let binding = binding(&config)?;
    let shell = NativeShell {
        config: Arc::new(config),
        mode: Mode::Control,
        lease: Arc::new(Mutex::new(None)),
        spatial_fenced: AtomicBool::new(false),
    };
    runtime_rejected(shell.check_environment())?;
    runtime_rejected(shell.open_control(Mode::Control, &c))?;
    runtime_rejected(shell.native(&binding, capture()?.region(), &c))?;
    runtime_rejected(shell.guard(Digest32::of_bytes(b"journal"), &c))?;
    runtime_rejected(shell.acquire(&c))?;
    runtime_rejected(shell.goal_source(&binding, &c))?;
    assert!(RUNTIME_CHECKS.load(Ordering::SeqCst) >= 7);
    assert_eq!(
        fs::read_dir(&directory.0)?.count(),
        0,
        "checkpoint rejects before custody creation"
    );
    assert!(matches!(listener.accept(),Err(cause) if cause.kind()==std::io::ErrorKind::WouldBlock));
    Ok(())
}

#[test]
fn concrete_default_evidence_reservation_fits_the_joined_runtime_envelope() -> Result<()> {
    let f = fixtures::Fixture::new()?;
    let mut config = Config::parse(&fixture()?.to_string())?;
    config.region = f
        .original_state
        .observation()
        .spatial()
        .terrain()
        .map
        .region;
    let required = evidence::reserve_bytes(&f.original_state, config.limits())?;
    assert!(
        required > 400 * MIB,
        "exercise the actual sixteen-MiB complete spatial payload bound"
    );
    assert!(
        required <= 430 * MIB,
        "joined default-budget tests must include the concrete native owner cost"
    );
    Ok(())
}
