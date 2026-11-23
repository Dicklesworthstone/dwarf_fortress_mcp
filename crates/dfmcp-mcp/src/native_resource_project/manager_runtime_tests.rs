//! Actual parent and Manager owners against joined native TCP protocol doubles.
//! Rust creates all custody; the peer only supplies native frames and injects
//! failures at explicit reply boundaries. This is not live-game evidence.
use super::*;
use std::path::Path;
use std::process::Command;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
const PHASE: &str = "RESOURCE_PROJECT_MANAGER_PHASE";
const CASE: &str = "RESOURCE_PROJECT_MANAGER_CASE";
const INPUT: &str = "RESOURCE_PROJECT_MANAGER_INPUT";
const OUTPUT: &str = "RESOURCE_PROJECT_MANAGER_OUTPUT";

fn text(value: &Value) -> TestResult<&str> {
    value
        .as_str()
        .ok_or_else(|| "missing Manager project fixture text".into())
}

fn output(value: &Value) -> TestResult {
    std::fs::write(std::env::var(OUTPUT)?, serde_json::to_vec(value)?)?;
    Ok(())
}

fn open(mode: Mode) -> TestResult<(Runtime, Result<Value>)> {
    let (config, workshop) = Config::load()?;
    Ok(Runtime::open(
        config,
        workshop,
        mode,
        SessionId::new(match mode {
            Mode::Control => 32_001,
            Mode::Offline => 32_002,
            Mode::Recover => 32_003,
        }),
        WorkBudget {
            max_wall_millis: 60_000,
            max_bytes: 4 * 1024 * 1024 * 1024,
            max_entities: 4096,
            max_actions: 5,
            max_output_tokens: 65_536,
            max_game_ticks: 0,
        },
        Instant::now(),
    )?)
}

fn advance(runtime: &mut Runtime) -> Result<Value> {
    runtime.perform(Action::Plan(
        json!({"kind":"advance_from_manager",
            "expected_head":runtime.reference()["parent_acknowledged"]["head"]})
        .to_string(),
    ))
}

fn assert_original(reference: &Value, original: &Value, deadline: u64) {
    let manager = &reference["child_reference"];
    assert_eq!(manager["original"]["key"], original["key"]);
    assert_eq!(manager["original"]["plan_digest"], original["plan_digest"]);
    assert_eq!(manager["original"]["deadline_tick"], deadline);
    assert_eq!(reference["original"]["goal"]["deadline_tick"], deadline);
    assert_eq!(reference["automatic_redispatch"], false);
    assert_eq!(reference["preparation_restored"], false);
    assert_eq!(reference["current_goal_available"], false);
    assert!(manager.get("original_review_seal").is_none());
}

fn control(case: &str) -> TestResult {
    let (mut runtime, opened) = open(Mode::Control)?;
    let opened = opened?;
    assert_eq!(opened["stage"], "manager");
    assert_eq!(opened["original_goal_proven"], false);
    let deadline = runtime.config.goal.deadline_tick();
    let observed = runtime.perform(Action::Observe(None))?;
    assert_eq!(observed["child"]["source_profile"], "manager/1.23");
    assert_eq!(observed["child"]["project_deadline_tick"], deadline);
    assert_eq!(observed["child"]["observation"]["unit_id"], 256);
    assert_eq!(observed["child"]["observation"]["histfig_id"], 361);
    assert!(
        observed["child"]["observation"]
            .get("native_capture_hex")
            .is_none()
    );
    assert_eq!(
        observed["child"]["observation"]["native_capture_commitment"]["raw_bytes_inline"],
        false
    );

    // The real parent rejects deadline replacement before its child can create
    // an original or send PrepareManager. Then a fresh observation is required.
    assert!(
        runtime
            .perform(Action::Plan(
                json!({
                    "kind":"appoint_vacant_manager",
                    "expected_observation":observed["child"]["observation"]["witness"],
                    "deadline_tick":deadline + 1,
                })
                .to_string()
            ))
            .is_err()
    );
    assert!(runtime.reference()["child_reference"]["original"].is_null());
    let observed = runtime.perform(Action::Observe(None))?;
    let planned = runtime.perform(Action::Plan(
        json!({
            "kind":"appoint_vacant_manager",
            "expected_observation":observed["child"]["observation"]["witness"],
            "deadline_tick":deadline,
        })
        .to_string(),
    ))?;
    let original = planned["child"]["original"].clone();
    assert_eq!(original["prepared_in_this_connection"], true);
    assert_eq!(original["deadline_tick"], deadline);
    assert_eq!(original["native_phase"], 0);
    let prepared = runtime.reference();
    assert_original(&prepared, &original, deadline);

    let committed = runtime.perform(Action::Commit {
        seal: text(&original["review_seal"])?.into(),
        confirmed: true,
        scope: Some("goal".into()),
    });
    if matches!(case, "lost_commit" | "parent_ack") {
        assert!(
            committed.is_err(),
            "injected {case} must not report successful parent publication"
        );
        let reference = runtime.reference();
        assert_original(&reference, &original, deadline);
        assert_eq!(reference["retained_stage"]["stage"], "manager");
        assert!(reference["retained_stage"]["checkpoint"]["manager_handoff"].is_null());
        assert!(reference["retained_stage"]["checkpoint"]["workshop"].is_null());
        if case == "lost_commit" {
            assert_eq!(reference["child_reference"]["uncertain_effect"], true);
            assert_eq!(reference["child_reference"]["dispatch_started"], true);
        } else {
            assert_eq!(reference["controller_fenced"], true);
            assert_eq!(reference["child_reference"]["native_phase"], 2);
        }
        return output(&json!({"reference":reference,"original":original,
            "prepared_reference":prepared,"deadline_tick":deadline}));
    }
    let committed = committed?;
    assert_eq!(committed["child"]["original"]["appointment_proved"], true);
    assert_eq!(committed["original_goal_proven"], false);
    let complete = runtime.perform(Action::Wait(Some("goal".into())))?;
    assert_eq!(complete["child"]["original"]["native_phase"], 2);
    assert_eq!(
        complete["child"]["original"]["production_manager_cache_proved"],
        true
    );
    assert_eq!(
        complete["child"]["original"]["prepared_in_this_connection"],
        false
    );
    assert_eq!(complete["original_goal_proven"], false);

    // An exact parent head is part of the public transition contract. Refusing
    // an old head must not re-open either child or manufacture a new review.
    assert!(
        runtime
            .perform(Action::Plan(
                json!({"kind":"advance_from_manager",
        "expected_head":Digest32::of_bytes(b"another parent head").to_hex()})
                .to_string()
            ))
            .is_err()
    );
    assert!(runtime.reference()["retained_stage"]["checkpoint"]["manager_handoff"].is_null());
    let advanced = advance(&mut runtime);
    let reference = runtime.reference();
    if case == "handoff_ack" {
        assert!(advanced.is_err());
        assert_eq!(reference["retained_stage"]["stage"], "manager");
        assert_eq!(reference["controller_fenced"], true);
        assert!(reference["retained_stage"]["checkpoint"]["manager_handoff"].is_null());
        assert!(reference["retained_stage"]["checkpoint"]["workshop"].is_null());
        assert_original(&reference, &original, deadline);
    } else {
        let checkpoint = &reference["retained_stage"]["checkpoint"];
        let handoff = &checkpoint["manager_handoff"];
        assert_eq!(handoff["schema"], "dfmcp.manager-project-handoff/1");
        assert_eq!(handoff["original_key"], original["key"]);
        assert_eq!(handoff["original_plan_digest"], original["plan_digest"]);
        assert_eq!(handoff["deadline_tick"], deadline);
        assert_eq!(
            handoff["acknowledged"],
            checkpoint["manager"]["acknowledged"]
        );
        assert_eq!(checkpoint["manager"]["original"]["deadline_tick"], deadline);
        if case == "stale_workshop" {
            assert!(
                advanced.is_err(),
                "older first Workshop source must not activate its stage"
            );
            assert_eq!(reference["retained_stage"]["stage"], "workshop_opening");
            assert!(checkpoint["workshop"].is_null());
            let retained = reference.clone();
            assert!(
                advance(&mut runtime).is_err(),
                "a retained opening marker must not reinitialize Workshop"
            );
            assert_eq!(
                runtime.reference()["retained_stage"],
                retained["retained_stage"]
            );
        } else {
            let advanced = advanced?;
            assert_eq!(advanced["stage"], "workshop");
            assert_eq!(advanced["child"]["advanced_to"], "workshop");
            assert_eq!(advanced["child"]["native_effect_dispatched"], false);
            assert_eq!(advanced["original_goal_proven"], false);
            assert!(checkpoint["tree"].is_null());
            assert!(checkpoint["tree_handoff"].is_null());
            assert!(checkpoint["production"].is_null());
            let frozen = checkpoint["manager"].clone();
            let frozen_handoff = handoff.clone();
            let inspected = runtime.perform(Action::Inspect)?;
            assert_eq!(inspected["original_goal_proven"], false);
            assert_eq!(
                runtime.reference()["retained_stage"]["checkpoint"]["manager"],
                frozen
            );
            assert_eq!(
                runtime.reference()["retained_stage"]["checkpoint"]["manager_handoff"],
                frozen_handoff
            );
        }
    }
    output(&json!({"reference":runtime.reference(),"original":original,
        "prepared_reference":prepared,"deadline_tick":deadline}))
}

fn recovered(phase: &str, case: &str) -> TestResult {
    let retained: Value = serde_json::from_slice(&std::fs::read(std::env::var(INPUT)?)?)?;
    let mode = if phase == "recover" {
        Mode::Recover
    } else {
        Mode::Offline
    };
    let opened = open(mode);
    if phase == "reject_opening" {
        if let Ok((runtime, result)) = opened {
            assert!(
                result.is_err(),
                "an unfinished Workshop opening accepted initialization on restart"
            );
            assert_eq!(
                runtime.reference()["retained_stage"]["stage"],
                "workshop_opening"
            );
            assert_eq!(
                runtime.reference()["retained_stage"]["checkpoint"]["manager_handoff"],
                retained["reference"]["retained_stage"]["checkpoint"]["manager_handoff"]
            );
        }
        return Ok(());
    }
    let (mut runtime, opened) = opened?;
    let opened = opened?;
    assert_eq!(opened["original_goal_proven"], false);
    assert_eq!(
        Some(runtime.config.goal.deadline_tick()),
        retained["deadline_tick"].as_u64()
    );
    let inspected = runtime.perform(Action::Inspect)?;
    assert_eq!(inspected["original_goal_proven"], false);
    if case == "success" {
        assert_eq!(inspected["stage"], "workshop");
        assert_eq!(
            runtime.reference()["retained_stage"]["checkpoint"]["manager"],
            retained["reference"]["retained_stage"]["checkpoint"]["manager"]
        );
        assert_eq!(
            runtime.reference()["retained_stage"]["checkpoint"]["manager_handoff"],
            retained["reference"]["retained_stage"]["checkpoint"]["manager_handoff"]
        );
    } else {
        let deadline = retained["deadline_tick"]
            .as_u64()
            .ok_or("retained deadline")?;
        assert_original(&runtime.reference(), &retained["original"], deadline);
        assert_eq!(
            inspected["child"]["original"]["prepared_in_this_connection"],
            false
        );
        assert_eq!(
            inspected["child"]["production_handoff_available"],
            phase == "recover" && case != "lost_commit"
        );
        if phase == "recover" {
            let reconciled = runtime.perform(Action::Wait(Some("goal".into())))?;
            assert_eq!(reconciled["child"]["original"]["appointment_proved"], true);
            assert_eq!(
                reconciled["child"]["original"]["production_manager_cache_proved"],
                true
            );
            assert_eq!(reconciled["child"]["original"]["native_phase"], 2);
            assert_eq!(reconciled["original_goal_proven"], false);
            assert_original(&runtime.reference(), &retained["original"], deadline);
            assert!(
                advance(&mut runtime).is_err(),
                "recovery cannot start a later child"
            );
            assert_eq!(runtime.reference()["retained_stage"]["stage"], "manager");
            assert!(
                runtime.reference()["retained_stage"]["checkpoint"]["manager_handoff"].is_null()
            );
        }
    }
    output(&json!({"reference":runtime.reference(),"view":inspected}))
}

#[test]
fn native_manager_prefix_transfers_exact_custody_and_recovers_without_reexecution() -> TestResult {
    if let Ok(phase) = std::env::var(PHASE) {
        let case = std::env::var(CASE)?;
        crate::run_with_runtime_cx(|_| async {
            match phase.as_str() {
                "control" => control(&case),
                "offline" | "recover" | "reject_opening" => recovered(&phase, &case),
                _ => Err("unknown parent Manager runtime fixture phase".into()),
            }
        })??;
        println!("resource-project-manager:{phase}:{case}:checked");
        return Ok(());
    }
    let full = format!(
        "{}::native_manager_prefix_transfers_exact_custody_and_recovers_without_reexecution",
        module_path!()
    );
    let (_, name) = full.split_once("::").ok_or("Manager project test name")?;
    let driver = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/stdio/resource_project_manager_runtime_peer.py");
    let result = Command::new("python3")
        .arg("-B")
        .arg(driver)
        .arg(std::env::current_exe()?)
        .arg(name)
        .output()?;
    assert!(
        result.status.success(),
        "Manager project campaign: {}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        String::from_utf8_lossy(&result.stdout)
            .contains("resource-project-manager-campaign:checked")
    );
    Ok(())
}
