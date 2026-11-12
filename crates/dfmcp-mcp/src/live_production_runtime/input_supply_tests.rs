//! Actual combined Runtime and native RPC clients over independently encoded
//! operations, workforce and order peers. No MCP transport or game is modeled.
use super::*;
use std::path::Path;
use std::process::Command;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
const PHASE: &str = "COMBINED_INPUT_SUPPLY_PHASE";
const CASE: &str = "COMBINED_INPUT_SUPPLY_CASE";
const INPUT: &str = "COMBINED_INPUT_SUPPLY_INPUT";
const OUTPUT: &str = "COMBINED_INPUT_SUPPLY_OUTPUT";
const STAGE: &str = "COMBINED_INPUT_SUPPLY_STAGE";
// Match live_production_server::budgets for an 8 GiB public request: the
// wrapper retains opening inspection and the complete response before Runtime.
// Runtime::delegate separately pays its own 256 MiB output reservation.
const PUBLIC_WORK_BYTES: u64 = 8 * 1024 * MIB - 512 * MIB - 128 * 1024;

fn text(value: &Value) -> TestResult<&str> {
    value
        .as_str()
        .ok_or_else(|| "missing input-supply fixture text".into())
}
fn digest(value: &Value) -> TestResult<Digest32> {
    Ok(workforce::digest(text(value)?)?)
}
fn input() -> TestResult<Value> {
    Ok(serde_json::from_str(&std::fs::read_to_string(
        std::env::var(INPUT)?,
    )?)?)
}
fn output(value: &Value) -> TestResult {
    std::fs::write(std::env::var(OUTPUT)?, serde_json::to_vec(value)?)?;
    Ok(())
}
fn stage(value: &str) -> TestResult {
    std::fs::write(std::env::var(STAGE)?, value)?;
    Ok(())
}
fn open(mode: Mode) -> TestResult<Runtime> {
    Ok(Runtime::open(
        Config::from_environment(|| Ok(()))?,
        mode,
        SessionId::new(match mode {
            Mode::Control => 31_001,
            Mode::Offline => 31_002,
            Mode::Reconcile => 31_003,
        }),
        WorkBudget {
            max_wall_millis: 60_000,
            max_bytes: PUBLIC_WORK_BYTES,
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
fn prepare(runtime: &mut Runtime, plan: Digest32) -> TestResult<Value> {
    let context = runtime.context(false)?;
    Ok(runtime.prepare_next(plan, &context)?.value)
}
fn commit(runtime: &mut Runtime, view: &Value) -> TestResult<Value> {
    let seal = digest(&view["review"]["paired_seal"])?;
    let context = runtime.context(false)?;
    Ok(runtime.commit(seal, true, &context)?.value)
}

fn assignment(case: &str) -> bool {
    matches!(
        case,
        "legacy" | "portfolio" | "stock_targets" | "post_marker" | "dual" | "lost" | "dual_lost"
    )
}
fn single(case: &str) -> bool {
    matches!(
        case,
        "legacy" | "existing" | "stock_target" | "dual" | "lost" | "dual_lost"
    )
}

fn plan(runtime: &mut Runtime, observed: &Value, case: &str) -> TestResult<Value> {
    let mut request = json!({
        "expected_observation":observed["observation"]["seal"],
        "units":["43"], "labor":"MINE",
        "deadline_tick":runtime.anchor().tick.get() + 300,
        "poll_interval_ticks":5, "stable_samples":1
    });
    let context = runtime.context(false)?;
    let value = match case {
        "legacy" | "dual" | "lost" | "dual_lost" => {
            request["enabled"] = json!(true);
            request["recipe"] = json!("wooden_bed");
            request["amount"] = json!(2);
            runtime.plan(Request::parse(&request.to_string())?, &context)?
        }
        "existing" => {
            request["kind"] = json!("existing_labor");
            request["recipe"] = json!("wooden_bed");
            request["amount"] = json!(2);
            runtime
                .plan_existing_labor(ExistingLaborRequest::parse(&request.to_string())?, &context)?
        }
        "stock_target" => {
            request["kind"] = json!("stock_target");
            request["recipe"] = json!("wooden_bed");
            request["minimum_stack_units"] = json!(2);
            runtime.plan_stock_target(StockTargetRequest::parse(&request.to_string())?, &context)?
        }
        "stock_targets" | "all_met" | "all_met_fenced" | "positive_fenced"
        | "dual_four_targets" => {
            request["kind"] = json!("stock_targets");
            request["assign_labor"] = json!(case != "dual_four_targets");
            request["targets"] = json!([
                {"recipe":"wooden_bed","minimum_stack_units":1},
                {"recipe":"wooden_door","minimum_stack_units":1}
            ]);
            if case == "dual_four_targets" {
                request["targets"] = json!([
                    {"recipe":"wooden_bed","minimum_stack_units":1},
                    {"recipe":"wooden_chair","minimum_stack_units":1},
                    {"recipe":"wooden_door","minimum_stack_units":1},
                    {"recipe":"wooden_table","minimum_stack_units":1}
                ]);
            }
            runtime
                .plan_stock_targets(StockTargetsRequest::parse(&request.to_string())?, &context)?
        }
        _ => {
            request["orders"] = json!([
                {"recipe":"wooden_bed","amount":1},
                {"recipe":"wooden_door","amount":1}
            ]);
            if assignment(case) {
                request["kind"] = json!("portfolio");
                request["enabled"] = json!(true);
                runtime.plan_portfolio(PortfolioRequest::parse(&request.to_string())?, &context)?
            } else {
                request["kind"] = json!("existing_labor_portfolio");
                runtime.plan_existing_labor_portfolio(
                    ExistingLaborPortfolioRequest::parse(&request.to_string())?,
                    &context,
                )?
            }
        }
    };
    Ok(value.value)
}

fn assert_original(current: &Value, original: &Value) {
    assert_eq!(current["original_plan"], original["original_plan"]);
    assert_eq!(current["automatic_redispatch"], false);
    assert_eq!(current["original_goal_proven"], false);
    assert_eq!(
        current["input_supply"]["required_stack_units"],
        original["input_supply"]["required_stack_units"]
    );
    assert_eq!(current["input_supply"]["reserve_wood_stack_units"], 1);
    assert_eq!(
        current["input_supply"]["orders"],
        original["input_supply"]["orders"]
    );
    assert_eq!(
        current["recovery"]["input_supply_digest"],
        original["recovery"]["input_supply_digest"]
    );
}

fn control(case: &str) -> TestResult {
    let mut runtime = open(Mode::Control)?;
    let context = runtime.context(false)?;
    let observed = runtime.observe(ObserveRequest::default(), &context)?.value;
    if matches!(case, "all_met_fenced" | "positive_fenced") {
        // Restore the exact configured source-policy checkpoint with only its
        // sticky rejection fence advanced. This is not a supply assessment.
        let retained = inspect(&mut runtime)?;
        let mut floor = retained["input_supply"]["checkpoint"].clone();
        let origin_witness = digest(&floor["origin_witness"])?;
        floor
            .as_object_mut()
            .ok_or("missing input supply source checkpoint")?
            .remove("origin_witness");
        floor["fenced"] = json!(true);
        runtime
            .input_supply
            .as_ref()
            .ok_or("missing input supply owner")?
            .restore_checkpoint(
                input_config::InputCheckpoint {
                    origin_witness,
                    floor: serde_json::from_value(floor)?,
                },
                &context,
            )?;
    }
    if matches!(case, "all_met" | "all_met_fenced") {
        // A separate configured clock preserves Plan. No effect grant or clock
        // allowance is available, and no original work needs an input guard.
        runtime.opening_labor = false;
        runtime.opening_orders = false;
        let context = runtime.context(false)?;
        assert!(
            context
                .grants
                .iter()
                .any(|g| g.capability == Capability::Plan)
        );
        assert!(!context.grants.iter().any(|g| matches!(
            g.capability,
            Capability::ConfigureLabor | Capability::ConfigureProduction
        )));
    }
    let planned = plan(&mut runtime, &observed, case);
    if matches!(
        case,
        "shortage" | "reserve" | "ancestor" | "attachment" | "extra_queue" | "positive_fenced"
    ) {
        let error = planned
            .err()
            .ok_or("insufficient or competing supply produced an original")?;
        assert!(
            error.to_string().to_lowercase().contains("supply"),
            "{case}: {error}"
        );
        assert!(runtime.original.is_none());
        assert!(!runtime.config.path("original.plan").exists());
        let retained = inspect(&mut runtime)?;
        if case == "positive_fenced" {
            assert_eq!(retained["input_supply"]["source_fenced"], true);
            assert!(retained["input_supply"]["last_qualified"].is_null());
        } else if case != "extra_queue" {
            let required = if case == "shortage" { 2 } else { 3 };
            let eligible = match case {
                "shortage" => 1,
                "reserve" => 2,
                _ => 0,
            };
            assert_eq!(
                retained["input_supply"]["last_qualified"]["status"],
                "shortage"
            );
            assert_eq!(
                retained["input_supply"]["last_qualified"]["required_stack_units"],
                required
            );
            assert_eq!(
                retained["input_supply"]["last_qualified"]["eligible_stack_units"],
                eligible
            );
            assert_eq!(
                retained["input_supply"]["last_qualified"]["shortfall_stack_units"],
                required - eligible
            );
        }
        return output(&retained);
    }
    let planned = planned.map_err(|cause| format!("{case} Plan: {cause}"))?;
    if matches!(case, "all_met" | "all_met_fenced") {
        assert_eq!(planned["stock_targets"]["status"], "already_satisfied");
        assert!(runtime.original.is_none());
        assert!(planned["original_plan"].is_null());
        assert!(planned["frontier"].is_null());
        assert!(!runtime.config.path("original.plan").exists());
        if case == "all_met_fenced" {
            assert_eq!(planned["input_supply"]["source_fenced"], true);
            assert!(planned["input_supply"]["last_qualified"].is_null());
        }
        return output(&planned);
    }
    assert!(planned["input_supply"].is_object());
    assert_eq!(
        planned["input_supply"]["required_stack_units"],
        if case == "dual_four_targets" { 5 } else { 3 }
    );
    assert_eq!(planned["input_supply"]["reserve_wood_stack_units"], 1);
    assert_ne!(
        digest(&planned["recovery"]["input_supply_digest"])?,
        Digest32::ZERO
    );
    assert!(planned["recovery"]["input_supply_floor"].is_object());
    assert!(!planned["original_plan"].is_null());
    assert!(planned["review"].is_null());
    assert_eq!(planned["original_goal_proven"], false);
    let original = digest(&planned["original_plan"]["plan_digest"])?;
    if assignment(case) {
        let labor = prepare(&mut runtime, original)?;
        assert_eq!(labor["review"]["step_id"], 0);
        let applied = commit(&mut runtime, &labor)?;
        assert_original(&applied, &planned);
    }
    let order_step = u32::from(assignment(case));
    if matches!(case, "before_prepare" | "floor_regression") {
        stage("before_prepare")?;
        let error = prepare(&mut runtime, original)
            .err()
            .ok_or("changed input supply admitted Prepare")?;
        assert!(
            error.to_string().to_lowercase().contains("supply"),
            "{error}"
        );
        let retained = inspect(&mut runtime)?;
        assert_original(&retained, &planned);
        assert!(retained["review"].is_null());
        if case == "floor_regression" {
            assert_eq!(retained["recovery"]["input_supply_floor"]["fenced"], true);
            assert_eq!(
                retained["recovery"]["input_supply_floor"]["next_item_id"],
                100
            );
        }
        return output(&retained);
    }
    let review = prepare(&mut runtime, original)?;
    assert_eq!(review["review"]["step_id"], order_step);
    let committed = commit(&mut runtime, &review);
    if matches!(case, "post_marker" | "lost" | "dual_lost") {
        assert!(committed.is_err());
        let retained = inspect(&mut runtime)?;
        assert_original(&retained, &planned);
        assert_eq!(retained["frontier"]["needs_reconciliation"], true);
        assert_eq!(retained["frontier"]["all_applied"], false);
        assert_eq!(
            retained["frontier"]["entries"][order_step as usize]["dispatch_attempted"],
            true
        );
        if case == "post_marker" {
            assert_eq!(
                retained["frontier"]["entries"][order_step as usize]["phase"],
                "dispatch_started"
            );
        }
        assert!(retained["review"].is_null());
        assert!(commit(&mut runtime, &review).is_err());
        return output(&inspect(&mut runtime)?);
    }
    let committed = committed.map_err(|cause| format!("{case} first order: {cause}"))?;
    assert_original(&committed, &planned);
    if !single(case) {
        let count = if case == "dual_four_targets" { 4 } else { 2 };
        let mut complete = committed;
        for index in 1..count {
            if index == 1 {
                stage("before_second_prepare")?;
            }
            let next = prepare(&mut runtime, original);
            if matches!(case, "after_first" | "extra_after_first" | "retired_prefix") {
                assert!(
                    next.is_err(),
                    "original full demand or exact queue prefix was weakened"
                );
                let retained = inspect(&mut runtime)?;
                assert_original(&retained, &planned);
                assert_eq!(
                    retained["frontier"]["entries"][order_step as usize]["phase"],
                    "applied"
                );
                assert_eq!(retained["frontier"]["all_applied"], false);
                return output(&retained);
            }
            let next = next?;
            assert_eq!(next["review"]["step_id"], order_step + index);
            complete = commit(&mut runtime, &next)?;
            assert_original(&complete, &planned);
            assert_eq!(complete["frontier"]["all_applied"], index + 1 == count);
        }
        return output(&complete);
    }
    assert_eq!(committed["frontier"]["all_applied"], true);
    if case == "dual" {
        let context = runtime.context(false)?;
        let sampled = runtime.wait(&context)?.value;
        assert_original(&sampled, &planned);
        assert_eq!(sampled["frontier"]["all_applied"], true);
        assert!(sampled["current_parent_goal"].is_object());
        return output(&sampled);
    }
    output(&committed)
}

fn recover(phase: &str) -> TestResult {
    let retained = input()?;
    let mut runtime = open(if phase == "offline" {
        Mode::Offline
    } else {
        Mode::Reconcile
    })?;
    let reopened = inspect(&mut runtime)?;
    assert_original(&reopened, &retained);
    assert_eq!(reopened["frontier"]["needs_reconciliation"], true);
    assert!(reopened["review"].is_null());
    assert!(reopened["input_supply"].is_object());
    assert_eq!(
        reopened["recovery"]["input_supply_floor"],
        retained["recovery"]["input_supply_floor"]
    );
    assert!(reopened["input_supply"]["last_qualified"].is_null());
    if phase == "offline" {
        let context = runtime.context(false)?;
        assert!(
            runtime
                .observe(ObserveRequest::default(), &context)
                .is_err()
        );
        return output(&reopened);
    }
    let context = runtime.context(false)?;
    let recovered = runtime.wait(&context)?.value;
    assert_original(&recovered, &retained);
    assert_eq!(recovered["frontier"]["all_applied"], true);
    assert_eq!(recovered["frontier"]["needs_reconciliation"], false);
    assert_eq!(
        recovered["frontier"]["entries"][1]["idempotency_key"],
        retained["original_plan"]["steps"][1]["key"]
    );
    output(&recovered)
}

fn campaign(kind: &str, test: &str) -> TestResult {
    if let Ok(phase) = std::env::var(PHASE) {
        let case = std::env::var(CASE)?;
        match phase.as_str() {
            "control" => control(&case)?,
            "offline" | "reconcile" => recover(&phase)?,
            "reject" => {
                if case == "strip" {
                    Config::from_environment(|| Ok(()))?;
                }
                let error = open(Mode::Offline)
                    .err()
                    .ok_or("changed input-supply recovery was accepted")?;
                assert!(
                    error.to_string().to_lowercase().contains("supply"),
                    "{case}: {error}"
                );
            }
            _ => return Err("unknown input-supply test phase".into()),
        }
        println!("combined-input-supply-runtime:{phase}:{case}:checked");
        return Ok(());
    }
    let full = format!("{}::{test}", module_path!());
    let (_, name) = full.split_once("::").ok_or("input-supply test name")?;
    let driver = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/stdio/combined_input_supply_runtime_peer.py");
    let result = Command::new("python3")
        .arg("-B")
        .arg(driver)
        .arg(std::env::current_exe()?)
        .args([name, kind])
        .output()?;
    assert!(
        result.status.success(),
        "input-supply {kind}: {}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        String::from_utf8_lossy(&result.stdout)
            .contains(&format!("combined-input-supply-campaign:{kind}:checked"))
    );
    println!("{}", String::from_utf8_lossy(&result.stdout));
    Ok(())
}

#[test]
fn native_input_supply_guards_complete_original_demand_at_order_boundaries() -> TestResult {
    campaign(
        "gates",
        "native_input_supply_guards_complete_original_demand_at_order_boundaries",
    )
}

#[test]
fn native_input_supply_retains_original_recovery_without_redispatch() -> TestResult {
    campaign(
        "recovery",
        "native_input_supply_retains_original_recovery_without_redispatch",
    )
}
