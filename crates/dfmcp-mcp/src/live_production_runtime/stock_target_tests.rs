//! Real stock-target planning and dispatch against the native TCP fixtures.
//! Isolated child processes supply operator environments; the production
//! Runtime, original archives, source qualification and native clients are real.
//! These are component regressions, not MCP transport or live-game evidence.
use super::*;
use std::path::Path;
use std::process::Command;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
const PHASE: &str = "COMBINED_STOCK_TARGET_RUNTIME_PHASE";
const CASE: &str = "COMBINED_STOCK_TARGET_RUNTIME_CASE";
const INPUT: &str = "COMBINED_STOCK_TARGET_RUNTIME_INPUT";
const OUTPUT: &str = "COMBINED_STOCK_TARGET_RUNTIME_OUTPUT";
const STAGE: &str = "COMBINED_STOCK_TARGET_RUNTIME_STAGE";

fn text(value: &Value) -> TestResult<&str> {
    value.as_str().ok_or_else(|| "missing fixture text".into())
}

fn digest(value: &Value) -> TestResult<Digest32> {
    Ok(workforce::digest(text(value)?)?)
}

fn output(value: &Value) -> TestResult {
    std::fs::write(std::env::var(OUTPUT)?, serde_json::to_vec(value)?)?;
    Ok(())
}

fn stage(name: &str) -> TestResult {
    std::fs::write(std::env::var(STAGE)?, name)?;
    Ok(())
}

fn open(mode: Mode) -> TestResult<Runtime> {
    let session = match mode {
        Mode::Control => 29_001,
        Mode::Offline => 29_002,
        Mode::Reconcile => 29_003,
    };
    Ok(Runtime::open(
        Config::from_environment(|| Ok(()))?,
        mode,
        SessionId::new(session),
        WorkBudget {
            max_wall_millis: 60_000,
            max_bytes: 8 * 1024 * MIB,
            max_entities: 8192,
            max_actions: 5,
            max_output_tokens: MAX_OUTPUT_TOKENS,
            max_game_ticks: 0,
        },
    )?)
}

fn inspect(runtime: &mut Runtime) -> TestResult<Value> {
    let context = runtime.context(false)?;
    Ok(runtime.inspect(&context)?.value)
}

fn request(runtime: &Runtime, observed: &Value, case: &str) -> TestResult<StockTargetRequest> {
    let mut value = json!({
        "kind":"stock_target", "expected_observation":observed["observation"]["seal"],
        "units":["43"], "labor":"MINE", "recipe":"wooden_bed",
        "minimum_stack_units":6, "deadline_tick":runtime.anchor().tick.get() + 300,
        "poll_interval_ticks":5, "stable_samples":2
    });
    if case == "cap" {
        value["max_order_amount"] = json!(2);
    }
    Ok(StockTargetRequest::parse(&value.to_string())?)
}

fn assert_report(view: &Value, baseline: u64, amount: u64, status: &str) -> TestResult {
    let report = &view["stock_target"];
    assert_eq!(report["status"], status);
    assert_eq!(report["recipe"], "wooden_bed");
    assert_eq!(report["item_type"], "BED");
    assert_eq!(report["baseline_stack_units"], baseline);
    assert_eq!(report["target_stack_units"], 6);
    assert_eq!(
        report["deficit_stack_units"],
        6_u64.saturating_sub(baseline)
    );
    assert_eq!(report["amount"], amount);
    assert_eq!(report["max_order_amount"], 100);
    assert_eq!(report["work_required"], amount > 0);
    assert_eq!(report["mutation_planned"], amount > 0);
    assert_eq!(report["fresh_for_current_call"], false);
    assert_eq!(report["current_inventory_proven"], false);
    assert_ne!(digest(&report["source_digest"])?, Digest32::ZERO);
    assert_ne!(digest(&report["assessment_witness"])?, Digest32::ZERO);
    assert_eq!(report["source_generation"], 71);
    assert_eq!(report["source_counters"].as_array().map(Vec::len), Some(3));
    Ok(())
}

fn assert_same_target(current: &Value, original: &Value) {
    for field in [
        "status",
        "recipe",
        "item_type",
        "baseline_stack_units",
        "target_stack_units",
        "deficit_stack_units",
        "amount",
        "max_order_amount",
        "baseline_anchor",
        "source_digest",
        "source_generation",
        "source_counters",
        "assessment_witness",
    ] {
        assert_eq!(
            current["stock_target"][field], original["stock_target"][field],
            "original stock target changed: {field}"
        );
    }
    assert_eq!(current["original_plan"], original["original_plan"]);
    assert_eq!(current["automatic_redispatch"], false);
}

fn prepare(runtime: &mut Runtime, plan: Digest32) -> TestResult<Value> {
    let context = runtime.context(false)?;
    Ok(runtime
        .prepare_next(plan, &context)
        .map_err(|cause| format!("stock-target Prepare: {cause}"))?
        .value)
}

fn commit(runtime: &mut Runtime, prepared: &Value) -> TestResult<Value> {
    let seal = digest(&prepared["review"]["paired_seal"])?;
    let context = runtime.context(false)?;
    Ok(runtime
        .commit(seal, true, &context)
        .map_err(|cause| format!("stock-target Commit: {cause}"))?
        .value)
}

fn control(case: &str) -> TestResult {
    let mut runtime = open(Mode::Control)?;
    let context = runtime.context(false)?;
    let observed = runtime
        .observe(
            if case == "fresh" {
                ObserveRequest::parse(r#"{"include_operations":true}"#)?
            } else {
                ObserveRequest::default()
            },
            &context,
        )?
        .value;
    assert!(!runtime.config.path("original.plan").exists());
    let requested = request(&runtime, &observed, case)?;
    let context = runtime.context(false)?;
    let planned = runtime.plan_stock_target(requested, &context);
    if matches!(
        case,
        "cap" | "generation" | "unpaused" | "incomplete" | "matching_job"
    ) {
        assert!(
            planned.is_err(),
            "invalid stock source or cap produced a plan"
        );
        assert!(runtime.original.is_none());
        assert!(!runtime.config.path("original.plan").exists());
        let retained = inspect(&mut runtime)?;
        assert!(retained["original_plan"].is_null());
        assert!(retained["review"].is_null());
        assert_eq!(retained["original_goal_proven"], false);
        return output(&retained);
    }
    let planned = planned
        .map_err(|cause| format!("stock-target Plan: {cause}"))?
        .value;
    if matches!(case, "already" | "above") {
        assert_report(
            &planned,
            if case == "already" { 6 } else { 9 },
            0,
            "already_satisfied",
        )?;
        assert!(runtime.original.is_none());
        assert!(planned["original_plan"].is_null());
        assert!(planned["frontier"].is_null());
        assert!(planned["review"].is_null());
        assert!(!runtime.config.path("original.plan").exists());
        assert_eq!(planned["original_goal_proven"], false);
        assert_eq!(planned["current_state_proven"], false);
        let retained = inspect(&mut runtime)?;
        assert_eq!(retained["stock_target"], planned["stock_target"]);
        return output(&retained);
    }

    let (baseline, amount) = if case == "fresh" { (4, 2) } else { (2, 4) };
    assert_report(&planned, baseline, amount, "planned")?;
    assert_eq!(
        planned["stock_target"]["baseline_anchor"],
        planned["original_plan"]["anchor"]
    );
    assert_eq!(planned["stock_target"]["revalidation_failed"], false);
    assert!(planned["production_inspection_source"].is_null());
    assert!(planned["review"].is_null());
    assert_eq!(planned["original_goal_proven"], false);
    let steps = planned["original_plan"]["steps"]
        .as_array()
        .ok_or("original steps")?;
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0]["step_id"], 0);
    assert_eq!(steps[0]["depends_on"], json!([]));
    assert_eq!(steps[0]["action"]["kind"], "create_work_order");
    assert_eq!(steps[0]["action"]["amount"], amount);
    assert_eq!(
        planned["stock_target"]["baseline_anchor"],
        observed["anchor"]
    );
    if case == "fresh" {
        assert_ne!(
            planned["stock_target"]["source_digest"],
            observed["production_inspection_source"]["source_digest"],
            "stock-target Plan reused the old advisory census"
        );
    }
    if case == "workshop" {
        assert_ne!(
            digest(&planned["recovery"]["workshop_prerequisite_digest"])?,
            Digest32::ZERO
        );
        assert!(runtime.config.path("original.workshop").is_file());
    }
    let original = digest(&planned["original_plan"]["plan_digest"])?;
    if matches!(case, "before_stock" | "before_queue" | "before_job") {
        stage("before_prepare")?;
        assert!(prepare(&mut runtime, original).is_err());
        let retained = inspect(&mut runtime)?;
        assert_same_target(&retained, &planned);
        assert!(retained["review"].is_null());
        assert_eq!(retained["frontier"]["all_applied"], false);
        assert_eq!(retained["stock_target"]["revalidation_failed"], true);
        return output(&retained);
    }

    let prepared = prepare(&mut runtime, original)?;
    assert_eq!(prepared["review"]["step_id"], 0);
    assert_eq!(prepared["review"]["idempotency_key"], steps[0]["key"]);
    let committed = commit(&mut runtime, &prepared);
    if matches!(case, "after_stock" | "after_queue" | "lost") {
        assert!(
            committed.is_err(),
            "late source change or lost insertion was reported settled"
        );
        let retained = inspect(&mut runtime)?;
        assert_same_target(&retained, &planned);
        assert_eq!(retained["frontier"]["needs_reconciliation"], true);
        assert_eq!(retained["frontier"]["all_applied"], false);
        assert!(retained["review"].is_null());
        assert_eq!(retained["original_goal_proven"], false);
        if case != "lost" {
            assert_eq!(retained["stock_target"]["revalidation_failed"], true);
        }
        assert!(
            commit(&mut runtime, &prepared).is_err(),
            "a withdrawn review dispatched again"
        );
        return output(&inspect(&mut runtime)?);
    }

    let committed = committed?;
    assert_same_target(&committed, &planned);
    assert_eq!(committed["frontier"]["all_applied"], true);
    assert_eq!(committed["original_goal_proven"], false);
    for (position, name) in ["goal_first", "goal_second"].into_iter().enumerate() {
        stage(name)?;
        let context = runtime.context(false)?;
        let sampled = runtime
            .wait(&context)
            .map_err(|cause| format!("stock-target {name} Wait: {cause}"))?
            .value;
        assert_same_target(&sampled, &planned);
        assert_eq!(
            sampled["selected_inventory_goal"]["kind"],
            "observed_inventory_increase"
        );
        assert_eq!(sampled["selected_inventory_goal"]["step_id"], 0);
        assert_eq!(
            sampled["selected_inventory_goal"]["baseline_stack_units"],
            baseline
        );
        assert_eq!(sampled["selected_inventory_goal"]["target_stack_units"], 6);
        assert_eq!(sampled["original_goal_proven"], position == 1);
        assert_eq!(sampled["current_state_proven"], false);
        assert_eq!(sampled["stock_target"]["current_inventory_proven"], false);
        if position == 1 {
            output(&sampled)?;
        }
    }
    Ok(())
}

fn recover(phase: &str) -> TestResult {
    let retained: Value = serde_json::from_str(&std::fs::read_to_string(std::env::var(INPUT)?)?)?;
    let mode = if phase == "offline" {
        Mode::Offline
    } else {
        Mode::Reconcile
    };
    let mut runtime = open(mode).map_err(|cause| format!("stock-target {phase} open: {cause}"))?;
    let reopened = inspect(&mut runtime)?;
    assert_same_target(&reopened, &retained);
    assert_report(&reopened, 2, 4, "planned")?;
    assert_eq!(reopened["frontier"]["needs_reconciliation"], true);
    assert_eq!(reopened["original_goal_proven"], false);
    assert!(reopened["review"].is_null());
    if phase == "offline" {
        assert_eq!(reopened["labor_control_enabled"], false);
        assert_eq!(reopened["production_control_enabled"], false);
        let context = runtime.context(false)?;
        assert!(
            runtime
                .observe(ObserveRequest::default(), &context)
                .is_err()
        );
        return output(&reopened);
    }
    let context = runtime.context(false)?;
    let recovered = runtime
        .wait(&context)
        .map_err(|cause| format!("stock-target original reconciliation: {cause}"))?
        .value;
    assert_same_target(&recovered, &retained);
    assert_report(&recovered, 2, 4, "planned")?;
    assert_eq!(recovered["frontier"]["all_applied"], true);
    assert_eq!(recovered["frontier"]["needs_reconciliation"], false);
    assert_eq!(recovered["frontier"]["entries"][0]["phase"], "applied");
    assert_eq!(
        recovered["frontier"]["entries"][0]["idempotency_key"],
        retained["original_plan"]["steps"][0]["key"]
    );
    output(&recovered)
}

fn campaign(kind: &str, test: &str) -> TestResult {
    if let Ok(phase) = std::env::var(PHASE) {
        let case = std::env::var(CASE)?;
        match phase.as_str() {
            "control" => control(&case)?,
            "offline" | "reconcile" => recover(&phase)?,
            _ => return Err("unknown stock-target fixture phase".into()),
        }
        println!("combined-stock-target-runtime:{phase}:{case}:checked");
        return Ok(());
    }
    let full = format!("{}::{test}", module_path!());
    let (_, name) = full.split_once("::").ok_or("stock-target test name")?;
    let driver = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/stdio/combined_stock_target_runtime_peer.py");
    let result = Command::new("python3")
        .arg("-B")
        .arg(driver)
        .arg(std::env::current_exe()?)
        .args([name, kind])
        .output()?;
    assert!(
        result.status.success(),
        "stock-target Runtime campaign {kind}: {}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        String::from_utf8_lossy(&result.stdout)
            .contains(&format!("combined-stock-target-campaign:{kind}:checked"))
    );
    Ok(())
}

#[test]
fn native_stock_target_uses_fresh_stock_without_unnecessary_or_unbounded_orders() -> TestResult {
    campaign(
        "planning",
        "native_stock_target_uses_fresh_stock_without_unnecessary_or_unbounded_orders",
    )
}

#[test]
fn native_stock_target_rechecks_dispatch_and_recovers_the_original_without_redispatch() -> TestResult
{
    campaign(
        "recovery",
        "native_stock_target_rechecks_dispatch_and_recovers_the_original_without_redispatch",
    )
}
