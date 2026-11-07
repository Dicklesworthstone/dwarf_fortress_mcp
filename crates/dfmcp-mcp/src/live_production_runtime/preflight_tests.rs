//! Actual combined Runtime and native TCP doubles, without an MCP transport.
//! The existing Python peer owns the documented native wire protocols. These
//! tests exercise production source acquisition, custody, inspection and plans;
//! they establish neither live-game evidence nor production admission.
use super::super::*;
use dfmcp_adapter::semantic_work_orders::ObservedInventoryIncreaseObjective;
use dfmcp_core::ErrorCode;
use std::path::Path;
use std::process::Command;

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;
const CHILD: &str = "COMBINED_RUNTIME_PREFLIGHT_TEST_CASE";

/// Reuse the native peer's environment in an isolated process, as the native
/// factory tests do, without modifying the test runner's global environment.
fn native_fixture(case: &str, test: &str) -> std::result::Result<bool, Box<dyn std::error::Error>> {
    if let Ok(selected) = std::env::var(CHILD) {
        assert_eq!(selected, case);
        return Ok(true);
    }
    let full_name = format!("{}::{test}", module_path!());
    let (_, name) = full_name.split_once("::").ok_or("runtime test name")?;
    let driver = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/stdio/combined_production_runtime_peer.py");
    let output = Command::new("python3")
        .arg(driver)
        .arg(std::env::current_exe()?)
        .args([name, case])
        .output()?;
    assert!(
        output.status.success(),
        "runtime native case {case}: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains(&format!("combined-runtime-preflight:{case}:checked"))
    );
    Ok(false)
}

fn runtime() -> Result<Runtime> {
    Runtime::open(
        Config::from_environment(|| Ok(()))?,
        Mode::Control,
        SessionId::new(27_001),
        WorkBudget {
            max_wall_millis: 60_000,
            max_bytes: 8 * 1024 * MIB,
            max_entities: 8192,
            max_actions: 5,
            max_output_tokens: MAX_OUTPUT_TOKENS,
            max_game_ticks: 0,
        },
    )
}

fn native_calls() -> std::result::Result<Vec<Value>, Box<dyn std::error::Error>> {
    let raw = std::fs::read_to_string(std::env::var("COMBINED_RUNTIME_NATIVE_LOG")?)?;
    raw.lines()
        .map(|line| serde_json::from_str(line).map_err(Into::into))
        .collect()
}

fn operations_reads(calls: &[Value]) -> usize {
    calls
        .iter()
        .filter(|call| {
            call["plugin"] == "dfmcp_operations_v1_3" && call["method"] == "ReadObservation"
        })
        .count()
}

fn no_native_effects(calls: &[Value]) {
    assert!(
        calls
            .iter()
            .all(|call| call["method"].as_str().is_some_and(|method| ![
                "Prepare", "Commit", "Cancel"
            ]
            .iter()
            .any(|prefix| method.starts_with(prefix)))),
        "preflight and original planning must not prepare or dispatch native effects"
    );
}

fn observe(runtime: &mut Runtime) -> Result<Value> {
    let context = runtime.context(false)?;
    Ok(runtime
        .observe(
            ObserveRequest::parse(r#"{"include_operations":true}"#)?,
            &context,
        )?
        .value)
}

fn inspect(runtime: &mut Runtime, source: &Value, query: Value) -> Result<Value> {
    let request = InspectionRequest::parse(
        &json!({"schema":"dfmcp.query/1",
        "expected_anchor":source["anchor"],"query":query})
        .to_string(),
    )?;
    let context = runtime.context(false)?;
    Ok(runtime.inspect_production(request, None, &context)?.value)
}

fn items() -> Value {
    json!({"kind":"entities","kinds":["item"],"fields":["type_key","stack_size"],"limit":1})
}

#[test]
fn native_preflight_inspection_is_read_only_and_growth_plan_acquires_its_own_baseline() -> TestResult
{
    const CASE: &str = "growth";
    if !native_fixture(
        CASE,
        "native_preflight_inspection_is_read_only_and_growth_plan_acquires_its_own_baseline",
    )? {
        return Ok(());
    }
    let mut runtime = runtime()?;
    let observed = observe(&mut runtime)?;
    let source = &observed["production_inspection_source"];
    assert_eq!(source["observation_phase"], "before_plan");
    assert_eq!(source["parent_anchor"], observed["anchor"]);
    assert_ne!(
        source["anchor"]["state_hash"],
        observed["anchor"]["state_hash"]
    );
    assert_eq!(source["anchor"]["tick"], observed["anchor"]["tick"]);
    assert!(source["order_step_id"].is_null());
    assert!(observed["original_plan"].is_null());
    assert_eq!(source["mutation_authority"], false);
    let before_query = native_calls()?;
    assert_eq!(operations_reads(&before_query), 1);

    // The peer has already changed its live inventory to eleven. Query must
    // keep reading the retained seven-unit source without contacting it.
    let report = inspect(&mut runtime, source, items())?;
    assert_eq!(report["production_inspection_source"], *source);
    let inspection = &report["production_inspection"];
    assert_eq!(inspection["anchor"], source["anchor"]);
    assert_eq!(
        inspection["rows"][0]["fields"]["stack_size"]["value"]["value"],
        7
    );
    assert_eq!(inspection["mutation_authority"], false);
    assert_eq!(inspection["commit_compatible"], false);
    let diagnosis = inspect(
        &mut runtime,
        source,
        json!({"kind":"production_diagnosis","limit":1}),
    )?;
    assert_eq!(
        diagnosis["production_inspection"]["anchor"],
        source["anchor"]
    );
    assert_eq!(native_calls()?, before_query, "Query made a native request");

    let request = Request::parse(&json!({
        "expected_observation":observed["observation"]["seal"],
        "units":["43"],"labor":"MINE","enabled":true,
        "recipe":"wooden_bed","amount":2,"deadline_tick":300,
        "poll_interval_ticks":5,"stable_samples":1,
        "objective":{"kind":"observed_inventory_increase","item_type":"BED","increase_stack_units":2}
    }).to_string())?;
    let context = runtime.context(false)?;
    let planned = runtime.plan(request, &context)?.value;
    assert!(!planned["original_plan"].is_null());
    assert!(planned["production_inspection_source"].is_null());
    assert!(planned["review"].is_null());
    assert_eq!(planned["original_goal_proven"], false);
    let original = runtime.original.as_ref().ok_or("original plan")?;
    let increase = original
        .steps
        .iter()
        .find_map(|step| ObservedInventoryIncreaseObjective::from_selected_plan(original, step.id))
        .ok_or("sealed inventory increase")?;
    assert_eq!(increase.baseline_stack_units(), 11);
    assert_eq!(increase.increase_stack_units(), 2);
    assert_eq!(increase.target_stack_units(), 13);
    assert_eq!(increase.baseline_anchor(), original.anchor);
    assert_ne!(
        increase.baseline_source_digest().to_hex(),
        source["source_digest"]
            .as_str()
            .ok_or("preflight source digest")?
    );
    let after_plan = native_calls()?;
    assert_eq!(
        operations_reads(&after_plan),
        2,
        "growth planning must acquire exactly one independent baseline"
    );
    no_native_effects(&after_plan);
    assert!(inspect(&mut runtime, source, items()).is_err());
    assert_eq!(native_calls()?, after_plan);
    println!("combined-runtime-preflight:{CASE}:checked");
    Ok(())
}

#[test]
fn native_preflight_refresh_and_default_observe_withdraw_stale_sources() -> TestResult {
    const CASE: &str = "refresh";
    if !native_fixture(
        CASE,
        "native_preflight_refresh_and_default_observe_withdraw_stale_sources",
    )? {
        return Ok(());
    }
    let mut runtime = runtime()?;
    let first = observe(&mut runtime)?;
    let first_source = &first["production_inspection_source"];
    let second = observe(&mut runtime)?;
    let second_source = &second["production_inspection_source"];
    assert_ne!(first_source["anchor"], second_source["anchor"]);
    assert_ne!(
        first_source["source_digest"],
        second_source["source_digest"]
    );
    let current = inspect(&mut runtime, second_source, items())?;
    assert_eq!(
        current["production_inspection"]["rows"][0]["fields"]["stack_size"]["value"]["value"],
        11
    );
    let before_refusal = native_calls()?;
    assert!(matches!(inspect(&mut runtime, first_source, items()),
        Err(cause) if cause.code == ErrorCode::StaleAnchor));
    // A failed query withdraws current visibility without erasing the source
    // publisher's identity history; only another Observe restores visibility.
    assert!(inspect(&mut runtime, second_source, items()).is_err());
    assert_eq!(native_calls()?, before_refusal);

    let fresh = observe(&mut runtime)?;
    assert_eq!(
        fresh["production_inspection_source"]["observation_phase"],
        "before_plan"
    );
    let before_default = native_calls()?;
    let context = runtime.context(false)?;
    let ordinary = runtime.observe(ObserveRequest::default(), &context)?.value;
    assert!(ordinary["production_inspection_source"].is_null());
    let after_default = native_calls()?;
    assert_eq!(
        operations_reads(&after_default),
        operations_reads(&before_default),
        "default Observe must not capture operations"
    );
    assert!(
        inspect(
            &mut runtime,
            &fresh["production_inspection_source"],
            items()
        )
        .is_err()
    );
    assert_eq!(native_calls()?, after_default);
    assert!(runtime.original.is_none());
    no_native_effects(&after_default);
    println!("combined-runtime-preflight:{CASE}:checked");
    Ok(())
}
