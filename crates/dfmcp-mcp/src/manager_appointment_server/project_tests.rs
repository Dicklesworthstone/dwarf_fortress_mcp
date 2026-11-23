//! Exact native codecs and private Manager journal/ACK custody. These tests
//! exercise project ownership and recovery, not live-game admission.
use super::*;
use dfmcp_adapter::manager_appointment::Record;
use std::os::unix::fs::DirBuilderExt;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(1);
struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn vector(name: &str) -> Result<Vec<u8>> {
    let value: Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/manager_v1_23_component.json"
    ))
    .map_err(|_| invalid("Manager component vector JSON"))?;
    let raw = value[name]
        .as_str()
        .ok_or_else(|| invalid("Manager component vector absent"))?;
    raw.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let raw = std::str::from_utf8(pair).map_err(|_| invalid("Manager vector UTF-8"))?;
            u8::from_str_radix(raw, 16).map_err(|_| invalid("Manager vector hex"))
        })
        .collect()
}
fn configured(capture: &Capture, directory: &Path) -> Value {
    let d = capture.dimensions;
    json!({"schema":"dfmcp.manager-appointment-mcp-development/2","directory":directory,
        "endpoint":"127.0.0.1:5000","initialize":true,"acknowledged":null,
        "world_folder":capture.source.folder(),"site_id":capture.source.site(),
        "df_version":"test-df","dfhack_version":"test-dfhack","generation":capture.generation,
        "map_dimensions":d,"unit_id":capture.selection.unit,"histfig_id":capture.selection.figure,
        "read_scope":[0,0,0,d[0]-1,d[1]-1,d[2]-1],"scope":[0,0,0,d[0]-1,d[1]-1,d[2]-1],
        "authority_expires_tick":capture.tick+20000,"lease_ticks":1200,
        "checkpoint":"disposable-fortress-no-checkpoint"})
}
fn project_config(value: Value, deadline: u64) -> Result<Config> {
    let mut config = Config::parse_protocol(&value.to_string(), ManagerProtocol::V1_23)?;
    config.parent_check = Some(Arc::new(|| Ok(())));
    config.with_project_deadline(deadline)
}
fn fixture(original: bool) -> Result<(Runtime, Directory, OperationContext)> {
    let capture = Capture::decode(&vector("capture")?)?;
    let directory = Directory(std::env::temp_dir().join(format!(
        "dfmcp-manager-project-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory.0)
        .map_err(|_| invalid("Manager private test directory"))?;
    let config = project_config(configured(&capture, &directory.0), capture.tick + 12000)?;
    let budget = WorkBudget {
        max_wall_millis: 60000,
        max_bytes: 512 * 1024 * 1024,
        ..WorkBudget::CONSERVATIVE_DEFAULT
    };
    let context = context(
        &config,
        Mode::Control,
        SessionId::new(81),
        budget,
        1,
        capture.tick,
        capture.witness(),
    );
    let (mut journal, mut acks) = journal::open_private(
        &directory.0,
        &context,
        Mode::Control,
        &config.binding,
        config.binding.selection,
        true,
        None,
        [7; 32],
    )?;
    if original {
        let plan = Plan::new("manager-001", capture.clone())?;
        let original = Original::new(plan.clone(), capture.tick + 12000)?;
        journal.intent(original.clone(), &context, &mut acks)?;
        journal.prepared(
            Record::decode(&vector("prepared")?, &plan)?,
            &context,
            &mut acks,
        )?;
        journal.dispatch(
            original.seal(),
            &context,
            &mut acks,
            &mut |_| Ok(()),
            |_, _| Record::decode(&vector("assigned")?, &plan),
        )?;
    }
    let handoff_floor = journal
        .native
        .as_ref()
        .and_then(|record| record.after.as_ref())
        .map(capture_floor);
    let runtime = Runtime {
        config,
        session: context.session_id,
        mode: Mode::Control,
        budget,
        high_tick: journal.high_tick.max(capture.tick),
        high_sequence: journal.high_sequence.max(capture.sequence),
        journal,
        acks,
        source: None,
        observation: Some(capture),
        leases: LeaseManager::new(),
        lease: None,
        next_request: 2,
        handoff_floor,
        project_fenced: false,
    };
    Ok((runtime, directory, context))
}
fn descriptor(origin: &ManagerProductionOrigin, directory: &Path) -> Result<ManagerProjectHandoff> {
    let historical = origin
        .historical_record()
        .after
        .as_ref()
        .ok_or_else(|| invalid("Manager poststate absent"))?;
    Ok(ManagerProjectHandoff {
        directory: directory.to_path_buf(),
        binding: origin.binding().clone(),
        acknowledged: origin.pin(),
        original_key: origin.plan().key().to_owned(),
        original_plan_digest: origin.plan().digest,
        deadline_tick: origin.original_deadline_tick(),
        origin_witness: origin.witness(),
        origin_descriptor: origin.canonical_bytes().to_vec(),
        original_seal: origin.original_seal(),
        original_receipt: origin.receipt(),
        historical_appointment_tick: historical.tick,
        floor: origin.floor(),
    })
}
fn offline(mut runtime: Runtime, context: &OperationContext) -> Result<Runtime> {
    let pin = runtime.journal.pin();
    let config = runtime.config.clone();
    let budget = runtime.budget;
    runtime.withdraw();
    drop(runtime);
    let (journal, acks) = journal::open_private(
        &config.input.directory,
        context,
        Mode::Offline,
        &config.binding,
        config.binding.selection,
        false,
        Some(pin),
        [0; 32],
    )?;
    let handoff_floor = journal
        .native
        .as_ref()
        .and_then(|record| record.after.as_ref())
        .map(capture_floor);
    Ok(Runtime {
        config,
        session: context.session_id,
        mode: Mode::Offline,
        budget,
        high_tick: journal.high_tick,
        high_sequence: journal.high_sequence,
        journal,
        acks,
        source: None,
        observation: None,
        leases: LeaseManager::new(),
        lease: None,
        next_request: 2,
        handoff_floor,
        project_fenced: false,
    })
}

#[test]
fn manager_project_policy_is_explicitly_v1_23_and_stable_across_only_custody_selectors()
-> Result<()> {
    let capture = Capture::decode(&vector("capture")?)?;
    let value = configured(&capture, Path::new("/private/manager-project-policy"));
    assert!(Config::parse_protocol(&value.to_string(), ManagerProtocol::V1_22).is_err());
    let deadline = capture.tick + 12000;
    let initial = project_config(value.clone(), deadline)?;
    let mut recovery = value.clone();
    recovery["initialize"] = json!(false);
    recovery["acknowledged"] =
        json!({"journal_id":"a".repeat(64),"head":"b".repeat(64),"frames":3});
    recovery["production_handoff_floor"] = production_handoff::floor_json(capture_floor(&capture));
    assert_eq!(
        initial.project_identity(),
        project_config(recovery, deadline)?.project_identity()
    );
    for (name, changed) in [
        ("unit_id", json!(capture.selection.unit + 1)),
        ("histfig_id", json!(capture.selection.figure + 1)),
        ("generation", json!(capture.generation + 1)),
        ("endpoint", json!("127.0.0.1:5001")),
        ("lease_ticks", json!(1199)),
        ("authority_expires_tick", json!(capture.tick + 21000)),
    ] {
        let mut altered = value.clone();
        altered[name] = changed;
        assert_ne!(
            initial.project_identity(),
            project_config(altered, deadline)?.project_identity(),
            "{name}"
        );
    }
    assert!(initial.clone().with_project_deadline(deadline + 1).is_err());
    assert!(initial.project_effect_deadline(deadline).is_err());
    assert!(initial.project_effect_deadline(deadline - 1).is_ok());
    assert_eq!(initial.project_binding().protocol, ManagerProtocol::V1_23);
    Ok(())
}

#[test]
fn manager_project_handoff_matches_real_acknowledged_offline_custody_without_writes() -> Result<()>
{
    let (runtime, directory, context) = fixture(true)?;
    let reference = runtime.project_recovery_reference();
    let journal_name = runtime.config.binding.protocol.journal_name();
    let ack_name = runtime.config.binding.protocol.ack_name();
    let before = (
        std::fs::read(directory.0.join(journal_name)).map_err(|_| invalid("read journal"))?,
        std::fs::read(directory.0.join(ack_name)).map_err(|_| invalid("read ACK"))?,
    );
    let mut runtime = offline(runtime, &context)?;
    let original = runtime
        .journal
        .original
        .as_ref()
        .ok_or_else(|| invalid("original absent"))?
        .clone();
    let origin = runtime.journal.production_origin(
        &mut runtime.acks,
        original.plan.key(),
        original.plan.digest,
        &context,
    )?;
    let expected = descriptor(&origin, &directory.0)?;
    let value = expected.to_json();
    let handoff = ManagerProjectHandoff::from_json(&value)?;
    handoff.verify_origin(&origin)?;
    runtime.config.validate_project_handoff(&handoff)?;
    handoff.validate_project_checkpoint(&reference)?;
    assert_eq!(value, handoff.to_json());
    assert!(value.to_string().len() < HANDOFF_BYTES);
    let config = handoff.production_configuration();
    assert_eq!(
        config["manager_prerequisite"]["original_plan_digest"],
        original.plan.digest.to_hex()
    );
    assert_eq!(
        config["manager_prerequisite_checkpoint"]["origin_witness"],
        origin.witness().to_hex()
    );
    assert_eq!(
        std::fs::read(directory.0.join(journal_name)).map_err(|_| invalid("read journal"))?,
        before.0
    );
    assert_eq!(
        std::fs::read(directory.0.join(ack_name)).map_err(|_| invalid("read ACK"))?,
        before.1
    );
    assert!(!runtime.journal.dispatchable());
    Ok(())
}

#[test]
fn manager_project_handoff_rejects_substitution_noncanonical_bytes_and_fenced_source() -> Result<()>
{
    let (runtime, directory, context) = fixture(true)?;
    let mut runtime = offline(runtime, &context)?;
    let original = runtime
        .journal
        .original
        .as_ref()
        .ok_or_else(|| invalid("original absent"))?
        .clone();
    let origin = runtime.journal.production_origin(
        &mut runtime.acks,
        original.plan.key(),
        original.plan.digest,
        &context,
    )?;
    let value = descriptor(&origin, &directory.0)?.to_json();
    for (path, changed) in [
        ("/original_key", json!("another-original")),
        ("/original_plan_digest", json!("a".repeat(64))),
        ("/original_review_seal", json!("b".repeat(64))),
        ("/original_receipt", json!("c".repeat(64))),
        ("/deadline_tick", json!(origin.original_deadline_tick() + 1)),
        (
            "/historical_appointment_tick",
            json!(origin.high_tick() - 1),
        ),
    ] {
        let mut altered = value.clone();
        *altered
            .pointer_mut(path)
            .ok_or_else(|| invalid("test pointer"))? = changed;
        assert!(
            ManagerProjectHandoff::from_json(&altered)?
                .verify_origin(&origin)
                .is_err(),
            "{path}"
        );
    }
    for (path, changed) in [
        ("/origin_witness", json!("a".repeat(64))),
        ("/origin_descriptor_hex", json!("AB")),
        ("/directory", json!("/private/../manager")),
        ("/floor/fenced", json!(true)),
        ("/floor/generation", json!(origin.binding().generation + 1)),
        ("/floor/tick", json!(origin.original_deadline_tick())),
        ("/acknowledged/frames", json!(0)),
    ] {
        let mut altered = value.clone();
        *altered
            .pointer_mut(path)
            .ok_or_else(|| invalid("test pointer"))? = changed;
        assert!(
            ManagerProjectHandoff::from_json(&altered).is_err(),
            "{path}"
        );
    }
    let mut extra = value;
    extra["production_ready"] = json!(true);
    assert!(ManagerProjectHandoff::from_json(&extra).is_err());
    Ok(())
}

#[test]
fn manager_project_handoff_requires_exact_parent_checkpoint_and_every_consumed_floor() -> Result<()>
{
    let (runtime, directory, context) = fixture(true)?;
    let reference = runtime.project_recovery_reference();
    let mut runtime = offline(runtime, &context)?;
    let original = runtime
        .journal
        .original
        .as_ref()
        .ok_or_else(|| invalid("original absent"))?
        .clone();
    let origin = runtime.journal.production_origin(
        &mut runtime.acks,
        original.plan.key(),
        original.plan.digest,
        &context,
    )?;
    let handoff = descriptor(&origin, &directory.0)?;
    for (path, changed) in [
        ("/acknowledged/head", json!("a".repeat(64))),
        ("/journal_pin_unverified/frames", json!(1)),
        ("/original/deadline_tick", json!(original.deadline_tick + 1)),
        (
            "/source_identity/unit_id",
            json!(origin.binding().selection.unit + 1),
        ),
        ("/source_identity/protocol", json!("1.22")),
        ("/native_phase", json!(0)),
        ("/uncertain_effect", json!(true)),
        ("/project_source_frontier/high_tick", json!(0)),
        ("/project_source_frontier/fenced", json!(true)),
        ("/production_handoff_floor/history_count", json!(0)),
        ("/production_handoff_floor/sequence", json!(0)),
    ] {
        let mut altered = reference.clone();
        *altered
            .pointer_mut(path)
            .ok_or_else(|| invalid("test pointer"))? = changed;
        assert!(
            handoff.validate_project_checkpoint(&altered).is_err(),
            "{path}"
        );
    }
    Ok(())
}

#[test]
fn manager_project_recovery_retains_monotone_floors_and_never_restores_preparation() -> Result<()> {
    let (mut runtime, _directory, _context) = fixture(true)?;
    let mut reference = runtime.project_recovery_reference();
    runtime.config.input.initialize = false;
    runtime.config.external = runtime.acks.latest();
    runtime.config.project_recovery_pins(&reference)?;
    let mut wrong = reference.clone();
    wrong["acknowledged"]["head"] = json!("a".repeat(64));
    assert!(runtime.config.project_recovery_pins(&wrong).is_err());
    let old = runtime.project_floor();
    reference["project_source_frontier"]["high_tick"] = json!(old.tick + 10);
    reference["project_source_frontier"]["high_sequence"] = json!(old.sequence + 5);
    reference["project_source_frontier"]["next_event"] = json!(old.next_event + 2);
    reference["project_source_frontier"]["fenced"] = json!(true);
    runtime.retain_project_recovery_state(&reference)?;
    assert_eq!(runtime.high_tick, old.tick + 10);
    assert_eq!(runtime.high_sequence, old.sequence + 5);
    assert!(runtime.project_floor().fenced);
    assert!(runtime.observation.is_none() && runtime.source.is_none());
    assert!(!runtime.journal.dispatchable());
    reference["project_source_frontier"]["high_tick"] = json!(old.tick);
    reference["project_source_frontier"]["high_sequence"] = json!(old.sequence);
    reference["project_source_frontier"]["fenced"] = json!(false);
    runtime.retain_project_recovery_state(&reference)?;
    assert_eq!(runtime.project_floor().tick, old.tick + 10);
    assert!(runtime.project_floor().fenced);
    let retained = runtime.project_recovery_reference();
    assert!(retained.get("original_review_seal").is_none());
    assert_eq!(retained["original"]["currently_verified"], false);
    Ok(())
}

#[test]
fn manager_project_recovered_descendant_must_keep_deadline_even_after_empty_checkpoint()
-> Result<()> {
    for exact in [false, true] {
        let (mut runtime, directory, initial) = fixture(false)?;
        let reference = runtime.project_recovery_reference();
        let parent_pin = parse_pin(&reference["acknowledged"], true)?;
        assert_eq!(parent_pin.frames, 0);
        let capture = runtime
            .observation
            .as_ref()
            .ok_or_else(|| invalid("initial capture absent"))?
            .clone();
        let deadline = runtime
            .config
            .project_deadline
            .ok_or_else(|| invalid("project deadline absent"))?;
        let original = Original::new(
            Plan::new("manager-001", capture.clone())?,
            deadline - u64::from(!exact),
        )?;
        runtime
            .journal
            .intent(original.clone(), &initial, &mut runtime.acks)?;
        let descendant = runtime.journal.pin();
        let mut config = runtime.config.clone();
        config.input.initialize = false;
        config.external = Some(parent_pin);
        config.project_recovery_pins(&reference)?;
        let budget = runtime.budget;
        drop(runtime);
        let recovery = context(
            &config,
            Mode::Recover,
            initial.session_id,
            budget,
            2,
            capture.tick,
            capture.witness(),
        );
        let (journal, acks) = journal::open_private(
            &directory.0,
            &recovery,
            Mode::Recover,
            &config.binding,
            config.binding.selection,
            false,
            Some(parent_pin),
            [0; 32],
        )?;
        assert_eq!(journal.pin(), descendant);
        let mut reopened = Runtime {
            config,
            session: recovery.session_id,
            mode: Mode::Recover,
            budget,
            high_tick: journal.high_tick,
            high_sequence: journal.high_sequence,
            journal,
            acks,
            source: None,
            observation: None,
            leases: LeaseManager::new(),
            lease: None,
            next_request: 3,
            handoff_floor: None,
            project_fenced: false,
        };
        assert_eq!(
            reopened.retain_project_recovery_state(&reference).is_ok(),
            exact
        );
        assert_eq!(reopened.validate_project_original_policy().is_ok(), exact);
        assert_eq!(
            reopened.project_recovery_reference()["original"]["deadline_tick"],
            original.deadline_tick
        );
        assert!(reopened.project_has_original_history());
        assert!(!reopened.journal.dispatchable());
    }
    Ok(())
}

#[test]
fn manager_project_compact_reviews_keep_complete_effect_terms_and_honest_capture_commitments()
-> Result<()> {
    let (runtime, _directory, _context) = fixture(true)?;
    let capture = Capture::decode(&vector("capture")?)?;
    let view = runtime.project_capture_view(&capture, "original_manager_journal");
    assert!(view.get("native_capture_hex").is_none());
    assert_eq!(
        view["native_capture_commitment"]["byte_length"],
        capture.bytes().len()
    );
    assert_eq!(
        view["native_capture_commitment"]["sha256"],
        Digest32::of_bytes(capture.bytes()).to_hex()
    );
    assert_eq!(view["native_capture_commitment"]["raw_bytes_inline"], false);
    assert_eq!(view["approval"]["risk"], "irreversible");
    assert_eq!(view["approval"]["unit_id"], capture.selection.unit);
    assert_eq!(view["approval"]["histfig_id"], capture.selection.figure);
    assert_eq!(
        view["group_role_buckets"],
        json!(capture.group.role_buckets)
    );
    assert_eq!(view["civ_role_buckets"], json!(capture.civ.role_buckets));
    assert_eq!(
        view["approval"]["project_deadline_tick"],
        json!(runtime.config.project_deadline)
    );
    assert!(view.to_string().len() < 32 * 1024);
    let projected = runtime.project_view(runtime.view(false));
    assert!(
        projected["original"]["original_source"]
            .get("native_capture_hex")
            .is_none()
    );
    assert!(
        projected["original"]["after"]
            .get("native_capture_hex")
            .is_none()
    );
    Ok(())
}

#[test]
fn manager_project_guidance_preserves_native_horizon_and_expired_frontier() -> Result<()> {
    let (mut runtime, _directory, _context) = fixture(false)?;
    let capture = runtime
        .observation
        .as_ref()
        .ok_or_else(|| invalid("Manager offer absent"))?;
    let native_deadline = capture.tick + journal::MAX_HORIZON_TICKS;
    let root_deadline = native_deadline + 1;
    runtime.config.project_deadline = Some(root_deadline);
    let view = runtime.project_view(runtime.view(false));
    assert_eq!(view["project_deadline_tick"], root_deadline);
    assert_eq!(view["recommended_deadline_tick"], native_deadline);
    assert!(
        view["recommended_deadline_tick"]
            .as_u64()
            .is_some_and(|tick| tick < root_deadline)
    );
    runtime.config.project_deadline = Some(native_deadline - 1);
    let view = runtime.project_view(runtime.view(false));
    assert_eq!(view["recommended_deadline_tick"], native_deadline - 1);
    runtime.raise_project_tick_floor(native_deadline - 1)?;
    let view = runtime.project_view(runtime.view(false));
    assert!(view["recommended_deadline_tick"].is_null());
    assert_eq!(view["production_handoff_available"], false);
    Ok(())
}

#[test]
fn manager_project_rejection_floor_reaches_authority_and_cannot_enable_fresh_writes() -> Result<()>
{
    let (mut runtime, _directory, _context) = fixture(false)?;
    let deadline = runtime
        .config
        .project_deadline
        .ok_or_else(|| invalid("project deadline absent"))?;
    runtime.raise_project_tick_floor(deadline)?;
    assert!(
        runtime
            .config
            .project_effect_deadline(runtime.project_floor().tick)
            .is_err()
    );
    let old = runtime
        .observation
        .as_ref()
        .ok_or_else(|| invalid("old capture absent"))?
        .clone();
    assert!(runtime.retain_project_capture(&old).is_err());
    assert!(runtime.project_floor().fenced);
    assert_eq!(runtime.project_floor().tick, deadline);
    assert!(
        runtime
            .raise_project_tick_floor(MAX_NATIVE_TICK + 1)
            .is_err()
    );
    Ok(())
}

#[test]
fn manager_project_narrows_all_foreground_allowances_and_never_advances_time() -> Result<()> {
    let cap = WorkBudget {
        max_wall_millis: 60000,
        max_bytes: 512 * 1024 * 1024,
        max_entities: 128,
        max_actions: 1,
        max_output_tokens: 32768,
        max_game_ticks: 1200,
    };
    let remaining = WorkBudget {
        max_wall_millis: 1000,
        max_bytes: 1024,
        max_entities: 4,
        max_actions: 1,
        max_output_tokens: 256,
        max_game_ticks: 0,
    };
    let budget = narrow(cap, remaining, Instant::now())?;
    assert!(budget.max_wall_millis <= 1000);
    assert_eq!(budget.max_bytes, 1024);
    assert_eq!(budget.max_entities, 4);
    assert_eq!(budget.max_actions, 1);
    assert_eq!(budget.max_output_tokens, 256);
    assert_eq!(budget.max_game_ticks, 0);
    assert!(
        narrow(
            cap,
            WorkBudget {
                max_actions: 0,
                ..remaining
            },
            Instant::now()
        )
        .is_err()
    );
    Ok(())
}
