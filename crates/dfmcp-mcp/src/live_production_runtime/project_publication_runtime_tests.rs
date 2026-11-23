//! Actual child publication and restart against the existing native TCP peers.
//! The parent ledger is tested separately; its ACK callback is the injected
//! fault boundary here. These are component tests, not live-game admission.
use super::*;
use std::path::Path;
use std::process::Command;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
const PHASE: &str = "PROJECT_PUBLICATION_RUNTIME_PHASE";
const CASE: &str = "PROJECT_PUBLICATION_RUNTIME_CASE";
const INPUT: &str = "PROJECT_PUBLICATION_RUNTIME_INPUT";
const OUTPUT: &str = "PROJECT_PUBLICATION_RUNTIME_OUTPUT";
const STAGE: &str = "PROJECT_PUBLICATION_RUNTIME_STAGE";
const ORIGINAL_FILES: &[&str] = &[
    "original.plan",
    "original.citizens",
    "original.workshop",
    "original.manager",
    "execution.journal",
];

fn budget() -> WorkBudget {
    WorkBudget {
        max_wall_millis: 60_000,
        max_bytes: 8 * 1024 * MIB,
        max_entities: 8192,
        max_actions: 5,
        max_output_tokens: MAX_OUTPUT_TOKENS,
        max_game_ticks: 0,
    }
}

fn output(value: &Value) -> TestResult {
    std::fs::write(std::env::var(OUTPUT)?, serde_json::to_vec(value)?)?;
    Ok(())
}

fn custody_bytes() -> TestResult<std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>> {
    let mut files = std::collections::BTreeMap::new();
    for name in [
        "DFMCP_COMBINED_PRODUCTION_CONFIG",
        "DFMCP_SEMANTIC_WORKFORCE_CONFIG",
        "DFMCP_SEMANTIC_WORK_ORDER_CONFIG",
    ] {
        let config: Value = serde_json::from_str(&std::env::var(name)?)?;
        let directory = config["directory"]
            .as_str()
            .ok_or("missing test custody directory")?;
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                files.insert(entry.path(), std::fs::read(entry.path())?);
            }
        }
    }
    Ok(files)
}

fn stage(name: &str) -> TestResult {
    std::fs::write(std::env::var(STAGE)?, name)?;
    Ok(())
}

fn no_work_parent_floor(runtime: &mut Runtime, goal: &ProjectStockGoal) -> TestResult {
    let start_tick = runtime.anchor.tick.get();
    let original = runtime
        .stock_target_no_work
        .as_ref()
        .ok_or("missing stock watch")?
        .request
        .clone();
    let witness = runtime
        .stock_target_no_work
        .as_ref()
        .ok_or("missing stock watch")?
        .monitor
        .witness();
    for (phase, proved) in [("first_sample", false), ("second_sample", true)] {
        stage(phase)?;
        let c = runtime.context(false)?;
        let sample = runtime.wait(&c)?.value;
        assert_eq!(sample["original_goal_proven"], proved);
        assert_eq!(runtime.project_status(goal)?.original_goal_proven, proved);
    }
    let anchor = runtime.anchor;
    let files = custody_bytes()?;
    let floor = start_tick + 20;
    runtime.retain_project_authority_tick(floor)?;
    assert_eq!(
        runtime.anchor, anchor,
        "a rejection floor is not an observation"
    );
    assert_eq!(runtime.authority_tick_floor().get(), floor);
    assert!(!runtime.project_status(goal)?.original_goal_proven);
    let retained = runtime
        .stock_target_no_work
        .as_ref()
        .ok_or("lost original stock watch")?;
    assert_eq!(retained.request, original);
    assert_eq!(retained.monitor.witness(), witness);
    assert!(retained.monitor.latest().is_none());
    assert!(runtime.basis.is_none());
    assert_eq!(
        files,
        custody_bytes()?,
        "raising a floor wrote private custody"
    );
    assert!(runtime.retain_project_authority_tick(u64::MAX).is_err());
    assert_eq!(runtime.authority_tick_floor().get(), floor);

    // The peer remains at the last successful tick. It must not be accepted
    // beneath the newer parent floor even though its stock is still sufficient.
    let c = runtime.context(false)?;
    assert!(runtime.wait(&c).is_err());
    assert_eq!(runtime.anchor, anchor);
    assert_eq!(runtime.authority_tick_floor().get(), floor);
    assert!(!runtime.project_status(goal)?.original_goal_proven);
    assert_eq!(files, custody_bytes()?);

    stage("at_parent_floor")?;
    let c = runtime.context(false)?;
    let current = runtime.wait(&c)?.value;
    assert_eq!(current["original_goal_proven"], true);
    assert_eq!(runtime.anchor.tick.get(), floor);
    assert_eq!(
        current["no_work_stock_goal"]["sample"]["achieved_at_tick"],
        start_tick + 10
    );
    assert_eq!(
        current["no_work_stock_goal"]["deadline_tick"],
        goal.deadline_tick()
    );
    let sample = runtime.no_work_goal_json();
    runtime.retain_project_authority_tick(floor)?;
    runtime.retain_project_authority_tick(start_tick)?;
    assert_eq!(
        runtime.no_work_goal_json(),
        sample,
        "an equal or older floor interrupted current proof"
    );
    assert_eq!(runtime.authority_tick_floor().get(), floor);
    assert_eq!(files, custody_bytes()?);
    assert!(runtime.original.is_none());
    assert!(runtime.frontier.is_none());
    Ok(())
}

fn prepared_parent_floor(runtime: &mut Runtime) -> TestResult {
    let original = runtime.original.clone().ok_or("missing original")?;
    let c = runtime.context(false)?;
    let prepared = runtime.prepare_next(original.digest, &c)?.value;
    let seal = workforce::digest(
        prepared["review"]["paired_seal"]
            .as_str()
            .ok_or("missing review")?,
    )?;
    let floor = runtime.parent_authority_tick_floor();
    let anchor = runtime.anchor;
    let files = custody_bytes()?;
    runtime.retain_project_authority_tick(floor)?;
    runtime.retain_project_authority_tick(floor.saturating_sub(1))?;
    let c = runtime.context(false)?;
    assert_eq!(runtime.inspect(&c)?.value["review"], prepared["review"]);
    runtime.retain_project_authority_tick(floor + 1)?;
    assert_eq!(runtime.anchor, anchor);
    assert_eq!(runtime.original.as_ref(), Some(&original));
    assert!(
        runtime.basis.is_some(),
        "original citizen identity must remain available"
    );
    assert_eq!(files, custody_bytes()?);
    let c = runtime.context(false)?;
    let inspected = runtime.inspect(&c)?.value;
    assert!(inspected["review"].is_null());
    assert_eq!(inspected["original_goal_proven"], false);
    let c = runtime.context(false)?;
    assert!(runtime.commit(seal, true, &c).is_err());
    assert_eq!(runtime.original.as_ref(), Some(&original));
    assert_eq!(runtime.anchor, anchor);
    assert_eq!(runtime.authority_tick_floor().get(), floor + 1);
    assert_eq!(
        files,
        custody_bytes()?,
        "floor withdrawal or stale confirmation changed custody"
    );
    Ok(())
}

fn bind_project(config: &mut Config, goal: &ProjectStockGoal, origin: Digest32) {
    config.project_binding = Some(config::ProjectProductionBinding {
        workshop_origin: origin,
        deadline_tick: goal.deadline_tick(),
        authority_expires_tick: goal.deadline_tick(),
    });
}

fn control(case: &str) -> TestResult {
    let unopened = custody_bytes()?;
    assert!(
        Runtime::open_for_resource_project(
            Config::from_environment(|| Ok(()))?,
            Mode::Control,
            SessionId::new(30_000),
            budget(),
            0,
        )
        .is_err(),
        "a standalone owner cannot use the resource-project opening"
    );
    assert_eq!(custody_bytes()?, unopened);
    let mut runtime = Runtime::open(
        Config::from_environment(|| Ok(()))?,
        Mode::Control,
        SessionId::new(30_001),
        budget(),
    )?;
    let c = runtime.context(false)?;
    let observed = runtime.observe(ObserveRequest::default(), &c)?.value;
    let scalar_goal = project_goal::ProjectSingleStockGoal {
        recipe: "wooden_bed".into(),
        minimum_stack_units: 6,
        max_order_amount: 100,
        deadline_tick: runtime.anchor.tick.get() + 300,
        poll_interval_ticks: 5,
        stable_samples: 2,
        assign_labor: true,
    };
    let goal = ProjectStockGoal::Single(scalar_goal.clone());
    let origin = runtime
        .workshop
        .as_ref()
        .ok_or("missing Workshop fixture")?
        .witness()?;
    let before_binding = runtime.project_recovery_reference();
    assert!(
        runtime
            .retain_project_authority_tick(runtime.anchor.tick.get() + 1)
            .is_err()
    );
    assert_eq!(runtime.project_recovery_reference(), before_binding);
    bind_project(&mut runtime.config, &goal, origin);
    let request = StockTargetRequest::parse(
        &json!({
            "kind":"stock_target", "expected_observation":observed["observation"]["seal"],
            "units":["43"], "labor":"CARPENTER", "recipe":scalar_goal.recipe,
            "minimum_stack_units":scalar_goal.minimum_stack_units,"max_order_amount":scalar_goal.max_order_amount,
            "deadline_tick":goal.deadline_tick(),"poll_interval_ticks":goal.poll_interval_ticks(),
            "stable_samples":goal.stable_samples(),"assign_labor":goal.assign_labor(),
        })
        .to_string(),
    )?;
    let prior = runtime.project_recovery_reference();
    assert!(prior["recovery"].is_null());
    let directory = runtime.config.directory.clone();
    let mut recorded = None;
    let mut calls = 0_u32;
    let mut record = |intent: &ProjectProductionIntent, _: &OperationContext| -> Result<()> {
        calls += 1;
        for name in ORIGINAL_FILES {
            assert!(
                !directory.join(name).exists(),
                "child wrote {name} before parent ACK"
            );
        }
        intent.validate_checkpoint(&prior)?;
        recorded = Some(intent.to_json());
        if case == "ack_refusal" {
            return Err(denied("injected parent ACK refusal"));
        }
        if case == "partial" {
            // A successful parent ACK followed by a competing/incomplete child
            // capsule publication. Production must neither overwrite nor fix it.
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(directory.join("original.citizens"))
                .map_err(|_| invalid("cannot inject partial publication"))?;
        }
        Ok(())
    };
    let c = runtime.context(false)?;
    let result = runtime.project_plan_stock_target_recorded(&goal, request, &c, &mut record);
    match case {
        "no_work" | "no_work_floor" => {
            let view = result?.value;
            assert_eq!(calls, 0);
            assert!(recorded.is_none());
            assert!(view["recovery"].is_null());
            assert_eq!(view["stock_target"]["work_required"], false);
            for name in ORIGINAL_FILES {
                assert!(!directory.join(name).exists());
            }
            if case == "no_work_floor" {
                no_work_parent_floor(&mut runtime, &goal)?;
            }
        }
        "ack_refusal" => {
            assert!(result.is_err());
            assert_eq!(calls, 1);
            assert!(!runtime.initialization_failed);
            for name in ORIGINAL_FILES {
                assert!(!directory.join(name).exists());
            }
        }
        "partial" => {
            assert!(result.is_err());
            assert_eq!(calls, 1);
            assert!(runtime.initialization_failed);
            assert!(directory.join("original.plan").is_file());
            assert!(!directory.join("execution.journal").exists());
        }
        "published" | "advanced" | "prepared_floor" => {
            result?;
            assert_eq!(calls, 1);
            let intent =
                ProjectProductionIntent::from_json(recorded.as_ref().ok_or("missing intent")?)?;
            runtime.validate_project_publication(&goal, &intent)?;
            assert_eq!(
                runtime.project_recovery_reference(),
                intent.to_json()["recovery_reference"]
            );
            if case == "advanced" {
                let plan = runtime.original.as_ref().ok_or("missing original")?.digest;
                let c = runtime.context(false)?;
                runtime.prepare_next(plan, &c)?;
            }
            if case == "prepared_floor" {
                prepared_parent_floor(&mut runtime)?;
            }
        }
        _ => return Err("unknown project publication case".into()),
    }
    output(&json!({
        "intent": recorded,"prior":prior,"goal":goal,
        "actual":runtime.project_recovery_reference(),
        "workforce_journal_id":runtime.workforce_journal.to_hex(),
    }))
}

fn recover(case: &str) -> TestResult {
    let retained: Value = serde_json::from_slice(&std::fs::read(std::env::var(INPUT)?)?)?;
    let goal: ProjectStockGoal = serde_json::from_value(retained["goal"].clone())?;
    let intent = ProjectProductionIntent::from_json(&retained["intent"])?;
    let origin = workforce::digest(retained["intent"]["recovery_reference"]
        ["workshop_prerequisite"]["checkpoint"]["origin_witness"].as_str().ok_or("missing origin")?)?;
    let mut config = Config::from_environment(|| Ok(()))?;
    assert!(!config.initialize);
    assert!(
        config.recovery.is_none(),
        "fixture must exercise lost selector derivation"
    );
    bind_project(&mut config, &goal, origin);
    let mut expected = intent.to_json()["recovery_reference"].clone();
    let parent_floor = if case == "published_floor" {
        expected["observation_tick_floor"]
            .as_u64()
            .ok_or("missing original floor")?
            + 20
    } else {
        0
    };
    let recovered = config
        .recover_project_publication(&retained["prior"], &intent)
        .and_then(|()| {
            Runtime::open_for_resource_project(
                config,
                Mode::Offline,
                SessionId::new(30_002),
                budget(),
                parent_floor,
            )
        });
    if !matches!(case, "published" | "published_floor") {
        assert!(
            recovered.is_err(),
            "unrecoverable {case} publication was accepted"
        );
        return Ok(());
    }
    let mut runtime = recovered?;
    runtime.validate_project_publication(&goal, &intent)?;
    let reference = runtime.project_recovery_reference();
    if case == "published_floor" {
        expected["recovery"]["frontier"]["authority_tick_floor"] = json!(parent_floor);
        assert_eq!(reference["observation_tick_floor"], parent_floor);
        assert_eq!(runtime.authority_tick_floor().get(), parent_floor);
        assert_eq!(
            anchor_json(runtime.anchor),
            retained["intent"]["recovery_reference"]["recovery"]["anchor"]
        );
        assert!(runtime.anchor.tick.get() < parent_floor);
        let files = custody_bytes()?;
        assert!(runtime.retain_project_authority_tick(u64::MAX).is_err());
        runtime.retain_project_authority_tick(parent_floor.saturating_sub(1))?;
        assert_eq!(runtime.project_recovery_reference(), reference);
        assert_eq!(custody_bytes()?, files);
    }
    assert_eq!(reference["recovery"], expected["recovery"]);
    assert_eq!(reference["native_custody"], expected["native_custody"]);
    assert!(!runtime.project_status(&goal)?.original_goal_proven);
    let c = runtime.context(false)?;
    let view = runtime.inspect(&c)?.value;
    assert!(view["review"].is_null());
    assert_eq!(view["original_goal_proven"], false);
    assert_eq!(view["automatic_redispatch"], false);
    output(&reference)
}

#[test]
fn recorded_project_publication_recovers_exact_original_without_reexecution() -> TestResult {
    if let Ok(phase) = std::env::var(PHASE) {
        let case = std::env::var(CASE)?;
        match phase.as_str() {
            "control" => control(&case)?,
            "recover" => recover(&case)?,
            _ => return Err("unknown project publication phase".into()),
        }
        println!("project-publication-runtime:{phase}:{case}:checked");
        return Ok(());
    }
    let full = format!(
        "{}::recorded_project_publication_recovers_exact_original_without_reexecution",
        module_path!()
    );
    let (_, name) = full.split_once("::").ok_or("runtime test name")?;
    let driver = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/stdio/project_publication_runtime_peer.py");
    let result = Command::new("python3")
        .arg("-B")
        .arg(driver)
        .arg(std::env::current_exe()?)
        .arg(name)
        .output()?;
    assert!(
        result.status.success(),
        "project publication campaign: {}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        String::from_utf8_lossy(&result.stdout).contains("project-publication-campaign:checked")
    );
    Ok(())
}
