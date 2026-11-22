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

fn bind_project(config: &mut Config, goal: &ProjectStockGoal, origin: Digest32) {
    config.project_binding = Some(config::ProjectProductionBinding {
        workshop_origin: origin,
        deadline_tick: goal.deadline_tick,
        authority_expires_tick: goal.deadline_tick,
    });
}

fn control(case: &str) -> TestResult {
    let mut runtime = Runtime::open(
        Config::from_environment(|| Ok(()))?,
        Mode::Control,
        SessionId::new(30_001),
        budget(),
    )?;
    let c = runtime.context(false)?;
    let observed = runtime.observe(ObserveRequest::default(), &c)?.value;
    let goal = ProjectStockGoal {
        recipe: "wooden_bed".into(),
        minimum_stack_units: 6,
        max_order_amount: 100,
        deadline_tick: runtime.anchor.tick.get() + 300,
        poll_interval_ticks: 5,
        stable_samples: 2,
        assign_labor: true,
    };
    let origin = runtime
        .workshop
        .as_ref()
        .ok_or("missing Workshop fixture")?
        .witness()?;
    bind_project(&mut runtime.config, &goal, origin);
    let request = StockTargetRequest::parse(
        &json!({
            "kind":"stock_target", "expected_observation":observed["observation"]["seal"],
            "units":["43"], "labor":"CARPENTER", "recipe":goal.recipe,
            "minimum_stack_units":goal.minimum_stack_units,"max_order_amount":goal.max_order_amount,
            "deadline_tick":goal.deadline_tick,"poll_interval_ticks":goal.poll_interval_ticks,
            "stable_samples":goal.stable_samples,"assign_labor":goal.assign_labor,
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
        "no_work" => {
            let view = result?.value;
            assert_eq!(calls, 0);
            assert!(recorded.is_none());
            assert!(view["recovery"].is_null());
            assert_eq!(view["stock_target"]["work_required"], false);
            for name in ORIGINAL_FILES {
                assert!(!directory.join(name).exists());
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
        "published" | "advanced" => {
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
    let expected = intent.to_json()["recovery_reference"].clone();
    let recovered = config
        .recover_project_publication(&retained["prior"], &intent)
        .and_then(|()| Runtime::open(config, Mode::Offline, SessionId::new(30_002), budget()));
    if case != "published" {
        assert!(
            recovered.is_err(),
            "unrecoverable {case} publication was accepted"
        );
        return Ok(());
    }
    let mut runtime = recovered?;
    runtime.validate_project_publication(&goal, &intent)?;
    let reference = runtime.project_recovery_reference();
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
