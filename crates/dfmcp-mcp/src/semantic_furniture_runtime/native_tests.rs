use super::*;
use dfmcp_adapter::live_spatial::LiveSpatialObservation;
use dfmcp_world::{EntityKind, EvidenceCoverage, EvidenceSource, FactSource};
use std::fs;
use std::net::TcpListener;
use std::sync::atomic::{AtomicU64, AtomicUsize};

fn hex(raw: &str) -> Result<Vec<u8>> {
    if !raw.len().is_multiple_of(2) || !raw.is_ascii() {
        return Err(invalid("fixture hex extent"));
    }
    raw.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).map_err(|_| invalid("fixture hex text"))?;
            u8::from_str_radix(text, 16).map_err(|_| invalid("fixture hex byte"))
        })
        .collect()
}
fn capture() -> Result<BuildCapture> {
    let raw: Value = serde_json::from_str(include_str!(
        "../../../../bridge/common/tests/fixtures/build_placement_v1_19.json"
    ))
    .map_err(|_| invalid("fixture JSON"))?;
    BuildCapture::decode(&hex(raw["capture"]
        .as_str()
        .ok_or_else(|| invalid("fixture capture"))?)?)
}
fn fixture() -> Result<Value> {
    let capture = capture()?;
    Ok(json!({
        "schema":"dfmcp.semantic-furniture-development/1",
        "directory":"/tmp/dfmcp-semantic-furniture-test",
        "endpoint":"127.0.0.1:5000",
        "initialize":true,
        "sources":{
            "kind":"operator-development-source-acceptance/1",
            "world_folder":capture.folder(),"site_id":capture.site(),
            "df_version":"df","dfhack_version":"dfhack",
            "spatial_generation":801,"furniture_generation":capture.generation(),
            "operations_generation":902,"map_dimensions":capture.dimensions()
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
            Capability::Construct,
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
    assert_eq!(config.sources.operations_generation, 902);
    assert_eq!(config.sources.furniture_generation, capture()?.generation());
    assert_ne!(
        config.sources.spatial_generation,
        config.sources.furniture_generation
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
    for field in [
        "spatial_generation",
        "furniture_generation",
        "operations_generation",
    ] {
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
fn scope_contains_entire_target_halo_and_protected_cells() -> Result<()> {
    let mut raw = fixture()?;
    raw["scope"] = json!([10, 10, 2, 20, 20, 2]);
    let config = Config::parse(&raw.to_string())?;
    config.selection(BuildSelection::new(BuildKind::Bed, 42, [15, 15, 2])?)?;
    for target in [
        [10, 15, 2],
        [20, 15, 2],
        [15, 10, 2],
        [15, 20, 2],
        [15, 15, 1],
        [15, 15, 3],
    ] {
        assert!(
            config
                .selection(BuildSelection::new(BuildKind::Bed, 42, target)?)
                .is_err()
        );
    }
    raw["protected"] = json!([[16, 16, 2, 16, 16, 2]]);
    let config = Config::parse(&raw.to_string())?;
    assert!(
        config
            .selection(BuildSelection::new(BuildKind::Bed, 42, [15, 15, 2])?)
            .is_err(),
        "the protected halo corner must fence placement"
    );
    config.selection(BuildSelection::new(BuildKind::Chair, 42, [12, 12, 2])?)?;
    Ok(())
}

#[test]
fn native_source_pins_do_not_alias_spatial_or_operations_generations() -> Result<()> {
    let config = Config::parse(&fixture()?.to_string())?;
    let capture = capture()?;
    let binding = BuildBinding::new(config.endpoint, "df", "dfhack", &capture)?;
    config.binding(&binding)?;
    config.sources.capture(&capture)?;
    for generation in [
        config.sources.spatial_generation,
        config.sources.operations_generation,
    ] {
        let mut wrong = config.sources.clone();
        wrong.furniture_generation = generation;
        assert!(wrong.binding(&binding).is_err());
        assert!(wrong.capture(&capture).is_err());
    }
    let other_version = BuildBinding::new(config.endpoint, "different df", "dfhack", &capture)?;
    assert!(config.binding(&other_version).is_err());
    let other_endpoint = BuildBinding::new(
        SocketAddr::from(([127, 0, 0, 1], 5001)),
        "df",
        "dfhack",
        &capture,
    )?;
    assert!(config.binding(&other_endpoint).is_err());
    let mut wrong = config.sources.clone();
    wrong.world_folder = "other fortress".to_owned();
    assert!(wrong.binding(&binding).is_err());
    Ok(())
}

fn spatial() -> Result<(LiveSpatialCitizenState, SourceAcceptance, Region)> {
    let base = LiveSpatialObservation::decode_payload(
        &hex(include_str!("../../../dfmcp-adapter/tests/fixtures/spatial_v1_6.hex").trim())?,
        801,
        "df".to_owned(),
        "dfhack".to_owned(),
    )?;
    let citizens = [b"DFMC1800".as_slice(), &0u32.to_be_bytes()].concat();
    let mut payload = b"DFMS1800".to_vec();
    for part in [&base.encode_payload()?, &citizens] {
        payload.extend_from_slice(
            &u32::try_from(part.len())
                .map_err(|_| invalid("fixture extent"))?
                .to_be_bytes(),
        );
        payload.extend_from_slice(part);
    }
    let observed = LiveSpatialCitizenObservation::decode_payload(
        &payload,
        801,
        "df".to_owned(),
        "dfhack".to_owned(),
    )?;
    let terrain = observed.spatial().terrain();
    let sources = SourceAcceptance {
        kind: super::super::policy::ACCEPTANCE_KIND.to_owned(),
        world_folder: terrain.world_folder.clone(),
        site_id: terrain.site_id,
        df_version: "df".to_owned(),
        dfhack_version: "dfhack".to_owned(),
        spatial_generation: 801,
        furniture_generation: 41,
        operations_generation: 902,
        map_dimensions: terrain.map_dimensions,
    };
    let region = terrain.map.region;
    let mut state = LiveSpatialCitizenState::default();
    state.publish(observed)?;
    Ok((state, sources, region))
}

#[test]
fn spatial_policy_qualifies_exact_item_facts_without_broadening_other_domains() -> Result<()> {
    let (state, sources, region) = spatial()?;
    let snapshot = state
        .snapshot()
        .ok_or_else(|| invalid("fixture snapshot"))?;
    let c = context(snapshot.anchor());
    let policy = sources.spatial(&state, region, &c)?;
    assert!(policy.paused);
    assert_eq!(policy.all_entities, EvidenceCoverage::Unknown);
    assert_eq!(policy.all_edges, EvidenceCoverage::Unknown);
    assert_eq!(policy.entity_kinds.len(), 1);
    assert_eq!(
        policy.entity_kinds.get(&EntityKind::Item),
        Some(&EvidenceCoverage::Observed)
    );
    assert!(policy.terrain_regions.is_empty());
    let mut covered = 0;
    for item in snapshot
        .graph
        .entities
        .values()
        .filter(|entity| entity.kind == EntityKind::Item)
    {
        for fact in item.fields.values() {
            if let FactSource::DfhackField(field) = &fact.source {
                assert!(
                    policy.sources.contains(&EvidenceSource::Observed {
                        field: field.clone(),
                        source_digest: fact.source_digest,
                    }),
                    "missing exact item source {field}"
                );
                covered += 1;
            }
        }
    }
    assert!(
        covered >= 19,
        "actual fixture must exercise the complete item projection"
    );
    let mut denied_context = c.clone();
    denied_context
        .grants
        .retain(|grant| grant.capability != Capability::Observe);
    assert!(sources.spatial(&state, region, &denied_context).is_err());
    denied_context = c.clone();
    denied_context.cancellation_requested = true;
    assert!(sources.spatial(&state, region, &denied_context).is_err());
    denied_context = c;
    denied_context.anchor.state_hash = Digest32::of_bytes(b"substituted publication");
    assert!(sources.spatial(&state, region, &denied_context).is_err());
    let mut substituted = sources;
    substituted.spatial_generation = 41;
    assert!(
        substituted
            .observation(
                state
                    .observation_full()
                    .ok_or_else(|| invalid("fixture observation"))?,
                region
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
            "dfmcp-semantic-furniture-native-{}-{}",
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
    fs::write(directory.0.join("furniture.journal"), b"journal")?;
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
    let path = directory.0.join("furniture.journal");
    let c = native_context()?;
    let binding = BuildBinding::new(
        SocketAddr::from(([127, 0, 0, 1], 5000)),
        "df",
        "dfhack",
        &capture()?,
    )?;
    assert!(
        open_private_build_exact(&path, &c, BuildMode::Control, Some(binding.clone()), false)
            .is_err()
    );
    assert!(
        !path.exists(),
        "reopen with a supplied binding must not create custody"
    );
    let mut owner =
        open_private_build_exact(&path, &c, BuildMode::Control, Some(binding.clone()), true)?;
    let id = owner.inventory(&c)?.journal_id;
    assert!(
        open_private_build_exact(&path, &c, BuildMode::Offline, None, false).is_err(),
        "existing exclusive descriptor remains the only owner"
    );
    drop(owner);
    let bytes = fs::read(&path)?;
    assert!(
        open_private_build_exact(&path, &c, BuildMode::Control, Some(binding.clone()), true)
            .is_err()
    );
    assert_eq!(fs::read(&path)?, bytes);
    let mut reopened = open_private_build_exact(&path, &c, BuildMode::Control, None, false)?;
    assert_eq!(reopened.inventory(&c)?.journal_id, id);
    drop(reopened);
    fs::remove_file(&path)?;
    fs::write(&path, [])?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    assert!(open_private_build_exact(&path, &c, BuildMode::Control, Some(binding), true).is_err());
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
        "runtime checkpoint rejected furniture access",
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
    let binding = BuildBinding::new(config.endpoint, "df", "dfhack", &capture()?)?;
    let shell = NativeShell {
        config: Arc::new(config),
        mode: Mode::Control,
        lease: Arc::new(Mutex::new(None)),
        spatial_fenced: AtomicBool::new(false),
    };
    runtime_rejected(shell.check_environment())?;
    runtime_rejected(shell.open_control(Mode::Control, &c))?;
    runtime_rejected(shell.native(&binding, capture()?.selection(), &c, Duration::from_secs(1)))?;
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
