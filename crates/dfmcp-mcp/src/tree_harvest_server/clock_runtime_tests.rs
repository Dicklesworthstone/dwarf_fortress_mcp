//! Actual Tree Runtime and native TCP clients against joined protocol doubles.
//! Operator environments live in isolated children; custody is created only by
//! the production Runtime. These tests do not establish MCP or live-game behavior.
use super::*;
use std::path::Path;
use std::process::Command;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
const PHASE: &str = "TREE_CLOCK_RUNTIME_PHASE";
const CASE: &str = "TREE_CLOCK_RUNTIME_CASE";
const INPUT: &str = "TREE_CLOCK_RUNTIME_INPUT";
const OUTPUT: &str = "TREE_CLOCK_RUNTIME_OUTPUT";
const STAGE: &str = "TREE_CLOCK_RUNTIME_STAGE";

fn text(value: &Value) -> TestResult<&str> {
    value
        .as_str()
        .ok_or_else(|| "missing Tree fixture text".into())
}

fn stage(value: &str) -> TestResult {
    std::fs::write(std::env::var(STAGE)?, value)?;
    Ok(())
}

fn output(value: &Value) -> TestResult {
    std::fs::write(std::env::var(OUTPUT)?, serde_json::to_vec(value)?)?;
    Ok(())
}

fn open(mode: Mode) -> TestResult<(Runtime, Value)> {
    Ok(Runtime::open(
        Config::load()?,
        mode,
        SessionId::new(match mode {
            Mode::Control => 31_001,
            Mode::Offline => 31_002,
            Mode::Recover => 31_003,
        }),
        // Exercise the actual public clock-session ceiling, not a larger test allowance.
        WorkBudget {
            max_wall_millis: 60_000,
            max_bytes: 512 * 1024 * 1024,
            max_entities: 4096,
            max_actions: 1,
            max_output_tokens: 131_072,
            max_game_ticks: 1200,
        },
        Instant::now(),
    )?)
}

fn clock_request(view: &Value, game_ticks: u32) -> String {
    json!({"kind":"run_tree", "target_id":view["clock"]["target_id"],
        "game_ticks":game_ticks, "wall_millis":1000})
    .to_string()
}

fn assert_original(view: &Value, original: &Value, target: &Value, deadline: u64) {
    assert_eq!(view["original"]["key"], original["key"]);
    assert_eq!(view["original"]["plan_digest"], original["plan_digest"]);
    assert_eq!(view["original"]["deadline_tick"], deadline);
    assert_eq!(view["clock"]["target_id"], *target);
    assert_eq!(view["clock"]["deadline_tick"], deadline);
    assert_eq!(view["automatic_redispatch"], false);
}

fn rejected(runtime: &mut Runtime, action: Action, label: &str) -> TestResult<Value> {
    let cause = runtime
        .perform(action)
        .expect_err("unsafe Tree clock operation must refuse");
    Ok(
        json!({"case":label, "refused":true, "code":cause.code.as_str(),
        "reference":runtime.reference()}),
    )
}

fn control(case: &str) -> TestResult {
    let (mut runtime, opened) = open(Mode::Control)?;
    assert_eq!(opened["clock"]["enabled"], true);
    assert!(opened["original"].is_null());
    let start = opened["observation"]["tick"]
        .as_u64()
        .ok_or("initial Tree tick")?;
    let deadline = start + if case == "repeated" { 16 } else { 300 };
    let planned = runtime.perform(Action::Plan(
        json!({"kind":"fell_one_visible_tree",
            "expected_observation":opened["observation"]["witness"],
            "deadline_tick":deadline})
        .to_string(),
    ))?;
    let designated = runtime.perform(Action::Commit(
        text(&planned["original"]["review_seal"])?.into(),
        true,
    ))?;
    assert_eq!(designated["original"]["native_phase"], 2);
    assert_eq!(designated["original"]["fresh_supply_observed"], false);
    let original = designated["original"].clone();
    let mut sampled = runtime.perform(Action::Wait)?;
    assert_eq!(sampled["original"]["samples"], 1);
    assert_eq!(sampled["original"]["fresh_supply_observed"], false);
    let target = sampled["clock"]["target_id"].clone();
    assert_eq!(text(&target)?.len(), 64);
    assert_original(&sampled, &original, &target, deadline);

    if case == "capacity" {
        for index in 1..14 {
            stage(&format!("sample-{index}"))?;
            sampled = runtime.perform(Action::Wait)?;
        }
        assert_eq!(sampled["original"]["samples"], 14);
        let denied = rejected(
            &mut runtime,
            Action::ClockPlan(clock_request(&sampled, 10)),
            case,
        )?;
        assert_eq!(denied["code"], "budget_exceeded");
        return output(&denied);
    }

    stage("plan-clock")?;
    if case.starts_with("before_prepare_") {
        // Observe the changed original before any Run intent exists; retaining
        // that failed sample must then block clock preparation altogether.
        rejected(&mut runtime, Action::Wait, case)?;
    }
    if case.starts_with("before_prepare_") || case.starts_with("after_intent_") {
        return output(&rejected(
            &mut runtime,
            Action::ClockPlan(clock_request(&sampled, 10)),
            case,
        )?);
    }
    let prepared = runtime
        .perform(Action::ClockPlan(clock_request(&sampled, 10)))
        .map_err(|cause| format!("Tree ClockPlan in {case}: {cause}"))?;
    assert_original(&prepared, &original, &target, deadline);
    assert_eq!(prepared["clock"]["commit_available"], true);
    assert_eq!(prepared["original"]["fresh_supply_observed"], false);
    let seal = text(&prepared["clock"]["active_run"]["review_seal"])?.to_owned();
    assert_ne!(seal, text(&original["review_seal"])?);
    let first_key = prepared["clock"]["active_run"]["key"].clone();

    if case == "wrong_review" {
        rejected(
            &mut runtime,
            Action::ClockCommit("0".repeat(64), true),
            case,
        )?;
        let denied = rejected(&mut runtime, Action::ClockCommit(seal, true), case)?;
        assert!(denied["reference"]["clock"]["active_run"]["review_seal"].is_null());
        return output(&denied);
    }
    if case == "ack_drift" {
        let path = runtime.config.input.directory.join("tree-harvest.ack");
        let original_ack = std::fs::read(&path)?;
        let mut damaged = original_ack.clone();
        *damaged.last_mut().ok_or("empty ACK fixture")? ^= 1;
        std::fs::write(&path, damaged)?;
        let denied = rejected(&mut runtime, Action::ClockCommit(seal.clone(), true), case);
        std::fs::write(&path, original_ack)?;
        denied?;
        // Restoring bytes cannot restore the consumed in-memory clock review.
        return output(&rejected(
            &mut runtime,
            Action::ClockCommit(seal, true),
            case,
        )?);
    }

    stage("commit-clock")?;
    if case.starts_with("after_dispatch_") || case == "lost_reply" {
        let mut denied = rejected(&mut runtime, Action::ClockCommit(seal, true), case)?;
        denied["original"] = original;
        denied["target_id"] = target;
        denied["original_run_key"] = first_key;
        denied["deadline_tick"] = json!(deadline);
        return output(&denied);
    }

    let running = runtime
        .perform(Action::ClockCommit(seal, true))
        .map_err(|cause| format!("Tree ClockCommit in {case}: {cause}"))?;
    assert_original(&running, &original, &target, deadline);
    assert_eq!(running["clock"]["pending"], true);
    assert_eq!(running["clock"]["active_run"]["native_phase"], "running");
    assert_eq!(running["original"]["fresh_supply_observed"], false);
    let sample_count = running["original"]["samples"].clone();
    let pending = runtime.perform(Action::ClockWait)?;
    assert_eq!(pending["clock"]["pending"], true);
    assert_eq!(pending["original"]["samples"], sample_count);

    stage(if case == "repeated" {
        "finish-incomplete"
    } else if case == "stale_progress" {
        "finish-stale"
    } else {
        "finish-complete"
    })?;
    if case == "stale_progress" {
        let denied = rejected(&mut runtime, Action::ClockWait, case)?;
        assert!(
            denied["reference"]["consumed_tick_floor"]
                .as_u64()
                .is_some_and(|tick| tick >= start + 10),
            "stopped Run floor was forgotten: {denied}"
        );
        return output(&denied);
    }
    let mut complete = runtime
        .perform(Action::ClockWait)
        .map_err(|cause| format!("Tree stopped-interval observation in {case}: {cause}"))?;
    if case == "repeated" {
        assert_original(&complete, &original, &target, deadline);
        assert_eq!(complete["original"]["fresh_supply_observed"], false);
        assert_eq!(complete["clock"]["new_run_available"], true);
        stage("second-plan")?;
        let second = runtime.perform(Action::ClockPlan(clock_request(&complete, 6)))?;
        assert_ne!(second["clock"]["active_run"]["key"], first_key);
        assert_original(&second, &original, &target, deadline);
        runtime.perform(Action::ClockCommit(
            text(&second["clock"]["active_run"]["review_seal"])?.into(),
            true,
        ))?;
        stage("finish-second")?;
        complete = runtime.perform(Action::ClockWait)?;
    }
    assert_original(&complete, &original, &target, deadline);
    assert_eq!(complete["clock"]["pending"], false);
    assert_eq!(complete["original"]["fresh_supply_observed"], true);
    assert_eq!(complete["original"]["latest_progress"]["root_predicate"], 2);
    assert_eq!(
        complete["original"]["latest_progress"]["tick"],
        start + if case == "repeated" { 16 } else { 10 }
    );
    assert_eq!(complete["original"]["causal_item_provenance"], "unknown");
    let historical = runtime.perform(Action::Query)?;
    assert_eq!(historical["original"]["fresh_supply_observed"], false);
    output(&complete)
}

fn recovered(phase: &str) -> TestResult {
    if phase == "open_rejected" {
        assert!(open(Mode::Offline).is_err());
        return output(&json!({"phase":phase, "refused":true}));
    }
    let saved: Value = serde_json::from_slice(&std::fs::read(std::env::var(INPUT)?)?)?;
    let (mut runtime, opened) = open(if phase == "offline" {
        Mode::Offline
    } else {
        Mode::Recover
    })?;
    assert_eq!(opened["original"]["key"], saved["original"]["key"]);
    assert_eq!(opened["clock"]["target_id"], saved["target_id"]);
    assert_eq!(opened["clock"]["deadline_tick"], saved["deadline_tick"]);
    assert_eq!(opened["clock"]["commit_available"], false);
    assert_eq!(opened["original"]["fresh_supply_observed"], false);
    if phase == "offline" {
        let queried = runtime.perform(Action::Query)?;
        assert_eq!(
            queried["clock"]["active_run"]["key"],
            saved["original_run_key"]
        );
        assert_eq!(queried["original"]["fresh_supply_observed"], false);
        let waited = runtime.perform(Action::Wait)?;
        assert_eq!(waited["original"]["fresh_supply_observed"], false);
        let clock_wait = runtime.perform(Action::ClockWait);
        if let Ok(view) = clock_wait {
            assert_eq!(view["original"]["fresh_supply_observed"], false);
        }
        return output(&queried);
    }
    let complete = runtime.perform(Action::ClockWait)?;
    assert_eq!(complete["clock"]["pending"], false);
    assert_eq!(complete["original"]["fresh_supply_observed"], true);
    assert_eq!(complete["original"]["key"], saved["original"]["key"]);
    assert_eq!(complete["clock"]["target_id"], saved["target_id"]);
    assert!(complete["clock"]["active_run"].is_null());
    assert!(
        complete["clock"]["recent_runs"]
            .as_array()
            .is_some_and(|runs| runs
                .iter()
                .any(|run| run["key"] == saved["original_run_key"]
                    && run["native_phase"] == "stopped"
                    && run["stopped_with_pause"] == true))
    );
    assert_eq!(complete["clock"]["deadline_tick"], saved["deadline_tick"]);
    assert_eq!(complete["clock"]["new_run_available"], false);
    assert_eq!(complete["automatic_redispatch"], false);
    output(&complete)
}

fn campaign(kind: &str, test: &str) -> TestResult {
    if let Ok(phase) = std::env::var(PHASE) {
        let case = std::env::var(CASE)?;
        crate::run_with_runtime_cx(|_| async {
            match phase.as_str() {
                "control" => control(&case),
                "offline" | "recover" | "open_rejected" => recovered(&phase),
                _ => Err("unknown Tree clock fixture phase".into()),
            }
        })??;
        println!("tree-clock-runtime:{phase}:{case}:checked");
        return Ok(());
    }
    let full = format!("{}::{test}", module_path!());
    let (_, name) = full.split_once("::").ok_or("Tree clock test name")?;
    let driver =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/stdio/tree_clock_runtime_peer.py");
    let result = Command::new("python3")
        .arg("-B")
        .arg(driver)
        .arg(std::env::current_exe()?)
        .args([name, kind])
        .output()?;
    assert!(
        result.status.success(),
        "Tree clock Runtime campaign {kind}: {}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        String::from_utf8_lossy(&result.stdout)
            .contains(&format!("tree-clock-campaign:{kind}:checked"))
    );
    Ok(())
}

#[test]
fn native_tree_clock_preserves_original_deadline_and_requires_fresh_wood() -> TestResult {
    campaign(
        "progress",
        "native_tree_clock_preserves_original_deadline_and_requires_fresh_wood",
    )
}

#[test]
fn native_tree_clock_rechecks_durable_edges_and_recovers_without_redispatch() -> TestResult {
    campaign(
        "custody",
        "native_tree_clock_rechecks_durable_edges_and_recovers_without_redispatch",
    )
}
