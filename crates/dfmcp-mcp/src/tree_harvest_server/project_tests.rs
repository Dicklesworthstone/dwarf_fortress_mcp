//! Actual native codecs and real private original custody. These tests exercise
//! historical transfer/policy boundaries, not live-game or MCP admission.
use super::*;
use dfmcp_adapter::tree_harvest::Record;
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
    let vectors: Value = serde_json::from_str(include_str!(
        "../../../../tests/native/tree_harvest/vectors.json"
    ))
    .map_err(|_| invalid("native vectors"))?;
    let raw = vectors[name]
        .as_str()
        .ok_or_else(|| invalid("native vector missing"))?;
    raw.as_bytes()
        .chunks_exact(2)
        .map(|p| {
            let text = std::str::from_utf8(p).map_err(|_| invalid("vector UTF-8"))?;
            u8::from_str_radix(text, 16).map_err(|_| invalid("vector hex"))
        })
        .collect()
}
fn configured(capture: &Capture, directory: &Path) -> Value {
    let dims = capture.dimensions;
    json!({"schema":"dfmcp.tree-harvest-mcp-development/1","directory":directory,
        "endpoint":"127.0.0.1:5000","initialize":true,"acknowledged":null,
        "world_folder":capture.source.folder(),"site_id":capture.source.site(),
        "df_version":"test-df","dfhack_version":"test-dfhack","generation":capture.generation,
        "recovery_generation":null,"map_dimensions":dims,"root":capture.root,
        "read_scope":[0,0,0,dims[0]-1,dims[1]-1,dims[2]-1],
        "scope":[0,0,0,dims[0]-1,dims[1]-1,dims[2]-1],"protected":[],
        "authority_expires_tick":capture.tick+20000,"lease_ticks":1200,
        "checkpoint":"disposable-fortress-no-checkpoint"})
}
fn project_config(value: Value, deadline: u64) -> Result<Config> {
    let mut config = Config::parse(&value.to_string())?;
    config.parent_check = Some(Arc::new(|| Ok(())));
    config.with_project_deadline(deadline)
}
fn fixture() -> Result<(Runtime, Directory, OperationContext, Progress)> {
    fixture_with_original(true)
}
fn fixture_with_original(
    with_original: bool,
) -> Result<(Runtime, Directory, OperationContext, Progress)> {
    let capture = Capture::decode(&vector("capture")?)?;
    let directory = Directory(std::env::temp_dir().join(format!(
        "dfmcp-tree-project-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory.0)
        .map_err(|_| invalid("private test directory"))?;
    let config = project_config(configured(&capture, &directory.0), capture.tick + 12000)?;
    let budget = WorkBudget {
        max_wall_millis: 60000,
        max_bytes: 512 * 1024 * 1024,
        ..WorkBudget::CONSERVATIVE_DEFAULT
    };
    let context = context(
        &config,
        Mode::Control,
        SessionId::new(71),
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
        capture.root,
        true,
        None,
        [7; 32],
    )?;
    let plan = Plan::new("tree-001", capture.clone())?;
    let original = Original::new(plan.clone(), capture.tick + 12000)?;
    let mut raw = vector("progress_supply")?;
    raw[24..32].copy_from_slice(&(capture.tick + 10).to_be_bytes());
    let progress = Progress::decode(&raw, &plan)?;
    if with_original {
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
            |_, _| Record::decode(&vector("designated")?, &plan),
        )?;
        journal.begin_sample(&context, &mut acks)?;
        journal.sample(progress.clone(), &context, &mut acks)?;
    }
    let runtime = Runtime {
        config,
        session: context.session_id,
        mode: Mode::Control,
        budget,
        journal,
        acks,
        source: None,
        observation: Some(capture.clone()),
        leases: LeaseManager::new(),
        lease: None,
        next_request: 2,
        high_tick: if with_original {
            progress.tick
        } else {
            capture.tick
        },
        high_sequence: if with_original {
            progress.sequence
        } else {
            capture.sequence
        },
        clock: None,
        project: State::from_observation(Some(&capture)),
    };
    Ok((runtime, directory, context, progress))
}
fn descriptor(
    origin: &dfmcp_adapter::tree_harvest::journal::TreeMaterialOrigin,
    directory: &Path,
    progress: &Progress,
) -> Value {
    let b = origin.binding();
    json!({"schema":"dfmcp.tree-project-handoff/1","directory":directory,
        "binding":{"endpoint":b.endpoint.to_string(),"world_folder":b.source.folder(),"site_id":b.source.site(),
            "df_version":b.df_version,"dfhack_version":b.dfhack_version,"generation":b.generation,
            "map_dimensions":b.dimensions,"scope":[b.scope.min.x,b.scope.min.y,b.scope.min.z,b.scope.max.x,b.scope.max.y,b.scope.max.z]},
        "root":origin.original().plan.before.root,"acknowledged":pin_json(origin.pin()),
        "original_key":origin.original().plan.key(),"original_plan_digest":origin.original().plan.digest.to_hex(),
        "deadline_tick":origin.original().deadline_tick,"origin_witness":origin.witness().to_hex(),
        "historical_supply_observed_tick":origin.achieved_tick(),"progress_hex":hex(progress.bytes()),
        "high_tick":progress.tick,"high_generation":progress.generation,
        "high_sequence":progress.sequence,"high_next_item_id":progress.next_item})
}

#[test]
fn project_tree_policy_keeps_root_source_deadline_and_excludes_only_recovery_pins() -> Result<()> {
    let capture = Capture::decode(&vector("capture")?)?;
    let value = configured(&capture, Path::new("/tmp/tree-project-policy"));
    let deadline = capture.tick + 12000;
    let initial = project_config(value.clone(), deadline)?;
    let mut recovery = value.clone();
    recovery["initialize"] = json!(false);
    recovery["acknowledged"] =
        json!({"journal_id":"a".repeat(64),"head":"b".repeat(64),"frames":6});
    recovery["recovery_generation"] = json!(capture.generation + 1);
    assert_eq!(
        initial.project_identity(),
        project_config(recovery, deadline)?.project_identity()
    );
    for (name, changed) in [
        ("root", json!([14, 15, 2])),
        ("authority_expires_tick", json!(capture.tick + 21000)),
        ("endpoint", json!("127.0.0.1:5001")),
        ("lease_ticks", json!(1199)),
    ] {
        let mut changed_value = value.clone();
        changed_value[name] = changed;
        assert_ne!(
            initial.project_identity(),
            project_config(changed_value, deadline)?.project_identity()
        );
    }
    assert!(initial.clone().with_project_deadline(deadline + 1).is_err());
    assert!(initial.project_effect_deadline(deadline).is_err());
    assert!(initial.project_effect_deadline(deadline - 1).is_ok());
    Ok(())
}

#[test]
fn project_tree_checkpoint_exports_real_completed_custody_without_offline_authority() -> Result<()>
{
    let (mut runtime, directory, context, progress) = fixture()?;
    let before = std::fs::read(directory.0.join("tree-harvest.journal"))
        .map_err(|_| invalid("read journal"))?;
    let origin = runtime
        .journal
        .material_checkpoint(&mut runtime.acks, &context)?;
    assert_eq!(origin.achieved_tick(), Some(progress.tick));
    assert!(
        runtime
            .journal
            .material_origin(&mut runtime.acks, &context)
            .is_err()
    );
    let value = descriptor(&origin, &directory.0, &progress);
    let handoff = TreeProjectHandoff::from_json(&value)?;
    handoff.verify_origin(&origin)?;
    assert_eq!(handoff.to_json(), value);
    assert!(value.to_string().len() < 16 * 1024);
    assert_eq!(
        std::fs::read(directory.0.join("tree-harvest.journal"))
            .map_err(|_| invalid("read journal"))?,
        before
    );
    assert!(!runtime.journal.dispatchable());
    Ok(())
}

#[test]
fn project_tree_handoff_rejects_custody_substitution_and_noncanonical_progress() -> Result<()> {
    let (mut runtime, directory, context, progress) = fixture()?;
    let origin = runtime
        .journal
        .material_checkpoint(&mut runtime.acks, &context)?;
    let value = descriptor(&origin, &directory.0, &progress);
    for (field, changed) in [
        ("origin_witness", json!("a".repeat(64))),
        ("original_key", json!("replacement-key")),
        ("high_tick", json!(progress.tick + 1)),
        ("high_sequence", json!(progress.sequence + 1)),
        ("high_next_item_id", json!(progress.next_item + 1)),
        ("historical_supply_observed_tick", json!(progress.tick - 1)),
    ] {
        let mut altered = value.clone();
        altered[field] = changed;
        let handoff = TreeProjectHandoff::from_json(&altered)?;
        assert!(handoff.verify_origin(&origin).is_err(), "{field}");
    }
    for (field, changed) in [
        ("approved", json!(true)),
        ("progress_hex", json!("AB")),
        ("directory", json!("/tmp/../replacement")),
        ("high_generation", json!(progress.generation + 1)),
        ("high_tick", value["deadline_tick"].clone()),
    ] {
        let mut altered = value.clone();
        altered[field] = changed;
        assert!(TreeProjectHandoff::from_json(&altered).is_err(), "{field}");
    }
    Ok(())
}

#[test]
fn project_tree_recovery_checks_exact_ack_and_retains_descendant_floors() -> Result<()> {
    let (mut runtime, _directory, _context, progress) = fixture()?;
    let mut config = runtime.config.clone();
    config.input.initialize = false;
    config.external = runtime.acks.latest();
    let reference = runtime.project_recovery_reference();
    config.project_recovery_pins(&reference)?;
    let mut altered = reference.clone();
    altered["acknowledged"]["head"] = json!("c".repeat(64));
    assert!(config.project_recovery_pins(&altered).is_err());
    runtime.high_tick = progress.tick + 4;
    runtime.retain_frontier(Frontier {
        high_tick: progress.tick,
        high_generation: progress.generation,
        high_sequence: progress.sequence + 2,
        high_next_item_id: progress.next_item + 3,
        fenced: true,
    })?;
    assert_eq!(runtime.high_tick, progress.tick + 4);
    assert_eq!(runtime.high_sequence, progress.sequence + 2);
    assert!(runtime.project.fenced);
    runtime.retain_frontier(Frontier {
        high_tick: 0,
        high_generation: 0,
        high_sequence: 0,
        high_next_item_id: 0,
        fenced: false,
    })?;
    assert!(runtime.project.fenced);
    assert_eq!(runtime.project.high_item, progress.next_item + 3);
    assert!(reference.get("original_review_seal").is_none());
    assert_eq!(reference["original"]["currently_verified"], false);
    Ok(())
}

#[test]
fn project_tree_empty_parent_checkpoint_binds_recovered_descendant_deadline() -> Result<()> {
    for exact_deadline in [false, true] {
        let (mut runtime, directory, initial, _progress) = fixture_with_original(false)?;
        let parent_reference = runtime.project_recovery_reference();
        assert!(parent_reference["original"].is_null());
        let parent_pin = pin(&parent_reference["acknowledged"])?;
        assert_eq!(parent_pin.frames, 0);
        runtime.validate_project_original_policy()?;

        let capture = runtime
            .observation
            .as_ref()
            .ok_or_else(|| invalid("fixture capture missing"))?
            .clone();
        let deadline = runtime
            .config
            .project_deadline
            .ok_or_else(|| invalid("fixture project deadline missing"))?;
        let original = Original::new(
            Plan::new("tree-001", capture.clone())?,
            deadline - u64::from(!exact_deadline),
        )?;
        runtime
            .journal
            .intent(original.clone(), &initial, &mut runtime.acks)?;
        let descendant_pin = runtime.journal.pin();
        assert!(descendant_pin.frames > parent_pin.frames);
        let mut config = runtime.config.clone();
        config.input.initialize = false;
        config.external = Some(parent_pin);
        config.project_recovery_pins(&parent_reference)?;
        let budget = runtime.budget;
        drop(runtime);

        // Reopen the actual independently acknowledged private-file descendant
        // from the older parent pin. No replacement journal or source is used.
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
            capture.root,
            false,
            Some(parent_pin),
            [0; 32],
        )?;
        assert_eq!(journal.pin(), descendant_pin);
        assert_eq!(journal.original.as_ref(), Some(&original));
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
            clock: None,
            project: State::default(),
        };
        // The same production state-restoration body runs after the host's
        // separately checked current-environment and exact recovery-pin gates.
        let result = reopened.retain_project_recovery_state(&parent_reference);
        assert_eq!(result.is_ok(), exact_deadline);
        assert_eq!(
            reopened.validate_project_original_policy().is_ok(),
            exact_deadline
        );
        assert_eq!(reopened.project.high_generation, capture.generation);
        assert_eq!(reopened.project.high_item, capture.next_item);
        assert!(reopened.high_tick >= capture.tick);
        assert!(reopened.high_sequence >= capture.sequence);
        assert!(reopened.observation.is_none());
        assert!(reopened.source.is_none());
        assert!(!reopened.journal.dispatchable());
        let retained = reopened.project_recovery_reference();
        assert_eq!(
            retained["original"]["deadline_tick"],
            original.deadline_tick
        );
        assert_eq!(retained["original"]["currently_verified"], false);
        assert_eq!(retained["acknowledged"], pin_json(descendant_pin));
        assert!(retained.get("original_review_seal").is_none());
        assert_eq!(parent_reference["acknowledged"], pin_json(parent_pin));
    }
    Ok(())
}

#[test]
fn project_tree_compact_review_preserves_action_semantics_and_honest_custody() -> Result<()> {
    let (runtime, _directory, _context, _progress) = fixture()?;
    let capture = Capture::decode(&vector("capture")?)?;
    for (retained, owner) in [
        (false, "current_tree_owner"),
        (true, "original_tree_journal"),
    ] {
        let view = runtime.capture_view(&capture, retained);
        assert!(view.get("native_capture_hex").is_none());
        assert_eq!(view["native_capture_commitment"]["retained_by"], owner);
        assert_eq!(
            view["native_capture_commitment"]["sha256"],
            Digest32::of_bytes(capture.bytes()).to_hex()
        );
        assert_eq!(
            view["native_capture_commitment"]["byte_length"],
            capture.bytes().len()
        );
        assert_eq!(view["approval"]["root"], json!(capture.root));
        assert_eq!(
            view["approval"]["workers"][0]["woodcutting_labor_enabled"],
            capture.workers[0].facts[3]
        );
        assert_eq!(
            view["approval"]["tools"][0]["equipment_or_scheduling_proven"],
            false
        );
        assert_eq!(view["approval"]["future_falling_safety_proven"], false);
        assert!(view.to_string().len() < 32 * 1024);
    }
    let mut runtime = runtime;
    runtime.config.parent_check = None;
    assert_eq!(
        runtime.capture_view(&capture, false),
        capture_json(&capture)
    );
    Ok(())
}

#[test]
fn project_tree_unacknowledged_opening_cannot_adopt_original_history() -> Result<()> {
    let (empty, _directory, _context, _progress) = fixture_with_original(false)?;
    assert!(!empty.project_has_original_history());
    let (mut original, _directory, _context, _progress) = fixture()?;
    assert!(original.project_has_original_history());
    original.journal.original = None;
    assert!(
        original.project_has_original_history(),
        "mutable public cache cannot erase retained frames"
    );
    Ok(())
}

#[test]
fn project_tree_budget_narrows_every_allowance_without_renewal() -> Result<()> {
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
    let narrowed = narrow(cap, remaining, Instant::now())?;
    assert!(narrowed.max_wall_millis <= remaining.max_wall_millis);
    assert_eq!(narrowed.max_bytes, 1024);
    assert_eq!(narrowed.max_entities, 4);
    assert_eq!(narrowed.max_actions, 1);
    assert_eq!(narrowed.max_output_tokens, 256);
    assert_eq!(narrowed.max_game_ticks, 0);
    Ok(())
}

#[test]
fn parent_authority_floor_withdraws_tree_offer_and_rejects_older_native_capture() -> Result<()> {
    let (mut runtime, directory, _context, _) = fixture_with_original(false)?;
    let capture = runtime
        .observation
        .clone()
        .ok_or_else(|| invalid("fixture observation"))?;
    let files = ["tree-harvest.journal", "tree-harvest.ack"];
    let before = files
        .iter()
        .map(|name| std::fs::read(directory.0.join(name)))
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(|_| invalid("read original custody"))?;
    runtime.retain_project_authority_tick(capture.tick)?;
    assert_eq!(
        runtime.observation.as_ref().map(Capture::witness),
        Some(capture.witness())
    );
    let parent_tick = capture.tick + 100;
    runtime.retain_project_authority_tick(parent_tick)?;
    assert!(runtime.observation.is_none());
    assert!(runtime.source.is_none());
    assert!(!runtime.journal.dispatchable());
    assert_eq!(runtime.high_tick, parent_tick);
    assert_eq!(
        runtime.project_recovery_reference()["project_source_frontier"]["high_tick"],
        parent_tick
    );
    assert!(runtime.project_recovery_reference()["original"].is_null());
    runtime.retain_project_authority_tick(capture.tick)?;
    assert_eq!(runtime.high_tick, parent_tick);
    assert!(runtime.consume(&capture).is_err());
    assert!(runtime.project.fenced);
    assert_eq!(runtime.high_tick, parent_tick);
    assert_eq!(Capture::decode(capture.bytes())?.tick, capture.tick);
    assert!(
        runtime
            .retain_project_authority_tick(MAX_NATIVE_TICK + 1)
            .is_err()
    );
    assert_eq!(runtime.high_tick, parent_tick);
    for (name, expected) in files.into_iter().zip(before) {
        assert_eq!(
            std::fs::read(directory.0.join(name))
                .map_err(|_| invalid("read retained custody"))?,
            expected
        );
    }
    Ok(())
}

#[test]
fn parent_authority_floor_preserves_tree_original_and_cannot_renew_expired_designation()
-> Result<()> {
    let (mut runtime, _directory, _context, _) = fixture()?;
    let before = runtime.project_recovery_reference();
    let parent_tick = runtime
        .config
        .project_deadline
        .ok_or_else(|| invalid("deadline"))?
        + 1;
    runtime.retain_project_authority_tick(parent_tick)?;
    let after = runtime.project_recovery_reference();
    assert_eq!(after["original"], before["original"]);
    assert_eq!(after["acknowledged"], before["acknowledged"]);
    assert_eq!(after["project_source_frontier"]["high_tick"], parent_tick);
    let authority = context(
        &runtime.config,
        runtime.mode,
        runtime.session,
        runtime.budget,
        9,
        runtime.high_tick,
        Digest32::ZERO,
    );
    assert!(
        authority
            .authorize(
                Capability::Designate,
                RiskTier::Irreversible,
                &[],
                Some(runtime.config.scope),
            )
            .is_err()
    );
    assert!(
        authority
            .authorize(
                Capability::Query,
                RiskTier::ReadOnly,
                &[],
                Some(runtime.config.scope),
            )
            .is_ok()
    );
    runtime.config.parent_check = None;
    assert!(
        runtime
            .retain_project_authority_tick(parent_tick + 1)
            .is_err()
    );
    assert_eq!(runtime.high_tick, parent_tick);
    Ok(())
}

#[test]
fn resource_tree_open_rejects_unbound_policy_or_invalid_floor_before_io() -> Result<()> {
    let capture = Capture::decode(&vector("capture")?)?;
    let policy = configured(&capture, Path::new("/tmp/tree-project-floor-not-opened"));
    let bound = project_config(policy.clone(), capture.tick + 12000)?;
    let standalone = Config::parse(&policy.to_string())?;
    for (config, floor) in [(standalone, capture.tick), (bound, MAX_NATIVE_TICK + 1)] {
        let result = Runtime::open_for_resource_project(
            config,
            Mode::Control,
            SessionId::new(71),
            WorkBudget::CONSERVATIVE_DEFAULT,
            Instant::now(),
            floor,
        );
        assert_eq!(
            result.err().map(|error| error.code),
            Some(ErrorCode::CapabilityDenied)
        );
    }
    Ok(())
}
