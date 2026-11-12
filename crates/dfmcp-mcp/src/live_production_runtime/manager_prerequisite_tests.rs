//! Real production Runtime, acknowledged Manager 1.23 custody and native TCP fixtures.
//! Child processes isolate operator configuration without global environment
//! mutation. No MCP server, live game or production admission is simulated.
use super::*;
use dfmcp_adapter::build_placement::rpc::BuildCancellation;
use dfmcp_adapter::manager_appointment::journal::{Original as ManagerOriginal, open_private};
use dfmcp_adapter::manager_appointment::rpc::{Binding as ManagerBinding, ManagerRpc};
use dfmcp_adapter::manager_appointment::{
    FortressIdentity, ManagerProtocol, Plan as ManagerPlan, Selection,
};
use dfmcp_adapter::workshop_construction::journal::WorkshopMode;
use dfmcp_core::{MapCoord, MapCuboid};
use std::path::Path;
use std::process::Command;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
const PHASE: &str = "COMBINED_MANAGER_RUNTIME_PHASE";
const CASE: &str = "COMBINED_MANAGER_RUNTIME_CASE";
const INPUT: &str = "COMBINED_MANAGER_RUNTIME_INPUT";
const OUTPUT: &str = "COMBINED_MANAGER_RUNTIME_OUTPUT";

fn text(value: &Value) -> TestResult<&str> {
    value.as_str().ok_or_else(|| "missing fixture text".into())
}
fn number(value: &Value) -> TestResult<u64> {
    value
        .as_u64()
        .ok_or_else(|| "missing fixture number".into())
}
fn input() -> TestResult<Value> {
    Ok(serde_json::from_str(&std::fs::read_to_string(
        std::env::var(INPUT)?,
    )?)?)
}
fn write_output(value: &Value) -> TestResult {
    std::fs::write(std::env::var(OUTPUT)?, serde_json::to_vec(value)?)?;
    Ok(())
}
fn digest_value(value: &Value) -> TestResult<Digest32> {
    Ok(workforce::digest(text(value)?)?)
}

/// The donor is created by the actual Manager client and durable owners. Its
/// independent acknowledgement covers DispatchStarted before CommitManager.
fn seed_donor() -> TestResult {
    let mut value = input()?;
    let directory = Path::new(text(&value["directory"])?).to_owned();
    let fortress = FortressIdentity::new(
        text(&value["world_folder"])?,
        u32::try_from(number(&value["site_id"])?)?,
    )?;
    let fortress_id = fortress.fortress_id();
    let dimensions: [u32; 3] = serde_json::from_value(value["map_dimensions"].clone())?;
    let selection = Selection::new(
        u32::try_from(number(&value["unit_id"])?)?,
        u32::try_from(number(&value["histfig_id"])?)?,
    )?;
    let binding = ManagerBinding {
        protocol: ManagerProtocol::V1_23,
        endpoint: text(&value["endpoint"])?.parse()?,
        source: fortress,
        generation: number(&value["generation"])?,
        dimensions,
        selection,
        scope: MapCuboid::new(
            MapCoord::new(0, 0, 0),
            MapCoord::new(
                (dimensions[0] - 1) as i32,
                (dimensions[1] - 1) as i32,
                (dimensions[2] - 1) as i32,
            ),
        )?,
        df_version: text(&value["df_version"])?.into(),
        dfhack_version: text(&value["dfhack_version"])?.into(),
    };
    let context = OperationContext {
        session_id: SessionId::new(29_001),
        request_id: RequestId::new(1),
        anchor: StateAnchor {
            fortress_id,
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(number(&value["capture_tick"])?),
            state_hash: Digest32::ZERO,
        },
        budget: WorkBudget {
            max_wall_millis: 60_000,
            max_bytes: 512 * MIB,
            ..WorkBudget::CONSERVATIVE_DEFAULT
        },
        grants: [
            Capability::Query,
            Capability::Observe,
            Capability::Plan,
            Capability::ConfigureProduction,
        ]
        .into_iter()
        .map(|capability| CapabilityGrant {
            capability,
            max_risk: RiskTier::Irreversible,
            scope: CapabilityScope {
                fortress_id: Some(fortress_id),
                ..CapabilityScope::default()
            },
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
        cancellation_requested: false,
    };
    let (mut journal, mut acknowledgements) = open_private(
        &directory,
        &context,
        WorkshopMode::Control,
        &binding,
        selection,
        true,
        None,
        [91; 32],
    )?;
    let mut source = ManagerRpc::connect(
        binding.clone(),
        vec![b't'; 32],
        [92; 32],
        &context,
        BuildCancellation::default(),
        Box::new(|_| Ok(())),
    )?;
    let capture = source.observe(selection, &context)?;
    binding.capture(&capture)?;
    let plan = ManagerPlan::new("combined-manager-donor-1", capture)?;
    let original = ManagerOriginal::new(plan.clone(), context.anchor.tick.get() + 300)?;
    journal.intent(original.clone(), &context, &mut acknowledgements)?;
    let prepared = source.prepare(&plan, &context)?;
    journal.prepared(prepared, &context, &mut acknowledgements)?;
    source.preflight_commit(&plan, &context)?;
    let assigned = journal.dispatch(
        original.seal(),
        &context,
        &mut acknowledgements,
        &mut |_| Ok(()),
        |permit, current| source.commit(permit, current),
    )?;
    assert!(assigned.complete());
    assert_eq!(
        assigned
            .after
            .as_ref()
            .ok_or("manager after capture")?
            .group
            .role_buckets[4],
        vec![600]
    );
    let pin = journal.pin();
    assert_eq!(acknowledgements.latest(), Some(pin));
    value
        .as_object_mut()
        .ok_or("fixture object")?
        .remove("capture_tick");
    value["acknowledged"] = json!({
        "journal_id":pin.journal_id.to_hex(), "head":pin.head.to_hex(), "frames":pin.frames
    });
    value["original_key"] = json!(plan.key());
    value["original_plan_digest"] = json!(plan.digest.to_hex());
    write_output(&value)
}

fn open_runtime(mode: Mode) -> TestResult<Runtime> {
    let session = match (mode, std::env::var(PHASE).ok().as_deref()) {
        (Mode::Control, Some("checkpoint")) => 29_005,
        (Mode::Control, _) => 29_002,
        (Mode::Offline, _) => 29_003,
        (Mode::Reconcile, _) => 29_004,
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
fn inspect_runtime(runtime: &mut Runtime) -> TestResult<Value> {
    let context = runtime.context(false)?;
    Ok(runtime.inspect(&context)?.value)
}
fn prepare(runtime: &mut Runtime, plan: Digest32) -> TestResult<Value> {
    let context = runtime.context(false)?;
    Ok(runtime.prepare_next(plan, &context)?.value)
}
fn commit(runtime: &mut Runtime, prepared: &Value) -> TestResult<Value> {
    let seal = digest_value(&prepared["review"]["paired_seal"])?;
    let context = runtime.context(false)?;
    Ok(runtime.commit(seal, true, &context)?.value)
}

fn original_request(runtime: &Runtime, observed: &Value) -> TestResult<Request> {
    Ok(Request::parse(
        &json!({
            "expected_observation":observed["observation"]["seal"],
            "units":["43"], "labor":"MINE", "enabled":true,
            "recipe":"wooden_bed", "amount":1,
            "deadline_tick":runtime.anchor().tick.get() + 300,
            "poll_interval_ticks":5, "stable_samples":1
        })
        .to_string(),
    )?)
}

fn control(case: &str) -> TestResult {
    let mut runtime = open_runtime(Mode::Control)?;
    let context = runtime.context(false)?;
    let observed = runtime.observe(ObserveRequest::default(), &context)?.value;
    if case == "all_met" {
        let request = StockTargetsRequest::parse(
            &json!({
                "kind":"stock_targets", "expected_observation":observed["observation"]["seal"],
                "units":["43"], "labor":"MINE", "assign_labor":true,
                "targets":[{"recipe":"wooden_bed", "minimum_stack_units":1}],
                "deadline_tick":runtime.anchor().tick.get() + 300,
                "poll_interval_ticks":5, "stable_samples":1
            })
            .to_string(),
        )?;
        // The separately configured clock owner preserves Plan. Neither
        // production nor labor effect authority remains, and the time budget
        // is zero. No clock operation is requested or dispatched.
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
        assert_eq!(context.budget.max_game_ticks, 0);
        let planned = runtime.plan_stock_targets(request, &context)?.value;
        assert!(runtime.original.is_none());
        assert!(planned["original_plan"].is_null());
        assert!(planned["frontier"].is_null());
        assert!(planned["review"].is_null());
        assert_eq!(planned["stock_targets"]["status"], "already_satisfied");
        assert!(!runtime.config.path("original.plan").exists());
        assert!(!runtime.config.path("original.manager").exists());
        assert!(planned["manager_prerequisite"]["last_qualified"].is_null());
        return write_output(&planned);
    }
    let request = original_request(&runtime, &observed)?;
    let context = runtime.context(false)?;
    let planned = runtime.plan(request, &context);
    if matches!(
        case,
        "cache" | "identity" | "generation" | "future" | "past" | "dual_manager" | "dual_workshop"
    ) {
        let error = match planned {
            Ok(_) => return Err("unqualified manager unexpectedly allowed a plan".into()),
            Err(error) => error,
        };
        assert!(!error.message.is_empty());
        assert!(runtime.original.is_none());
        assert!(!runtime.config.path("original.manager").exists());
        let retained = inspect_runtime(&mut runtime)?;
        let checkpoint = &retained["manager_prerequisite"]["checkpoint"];
        assert_eq!(
            checkpoint["origin_witness"],
            retained["manager_prerequisite"]["origin_witness"]
        );
        assert!(number(&checkpoint["tick"])? >= runtime.anchor().tick.get());
        assert!(number(&checkpoint["next_event"])? > 0);
        if case == "generation" {
            assert_eq!(checkpoint["fenced"], true);
            assert!(number(&checkpoint["generation"])? > number(&input()?["generation"])?);
        }
        if case == "future" {
            assert!(number(&checkpoint["tick"])? > runtime.anchor().tick.get());
        }
        assert!(retained["manager_prerequisite"]["last_qualified"].is_null());
        return write_output(&retained);
    }
    let planned = planned?.value;
    let plan = digest_value(&planned["original_plan"]["plan_digest"])?;
    let association = digest_value(&planned["recovery"]["manager_prerequisite_digest"])?;
    assert_ne!(association, Digest32::ZERO);
    assert!(runtime.config.path("original.manager").is_file());
    if case.starts_with("dual") {
        assert_ne!(
            digest_value(&planned["recovery"]["workshop_prerequisite_digest"])?,
            Digest32::ZERO
        );
        assert!(runtime.config.path("original.workshop").is_file());
    }
    let labor = prepare(&mut runtime, plan)?;
    assert_eq!(labor["review"]["step_id"], 0);
    let applied = commit(&mut runtime, &labor);
    if case == "custody_late" {
        // The independent ACK was corrupted after the native labor writer.
        // Either its post-write guard or the next boundary must reject it.
        assert!(prepare(&mut runtime, plan).is_err());
        return write_output(&planned);
    }
    applied?;
    if matches!(case, "before_prepare" | "dual_late") {
        assert!(prepare(&mut runtime, plan).is_err());
        let refused = inspect_runtime(&mut runtime)?;
        assert_eq!(refused["frontier"]["all_applied"], false);
        assert_eq!(refused["original_plan"], planned["original_plan"]);
        let last = &refused["manager_prerequisite"]["last_qualified"];
        assert!(last.is_null() || last["fresh_for_current_call"] == false);
        return write_output(&refused);
    }
    let order = prepare(&mut runtime, plan)?;
    assert_eq!(order["review"]["step_id"], 1);
    let committed = commit(&mut runtime, &order);
    if matches!(case, "after_prepare" | "post_marker" | "lost" | "dual_lost") {
        assert!(committed.is_err());
        let retained = inspect_runtime(&mut runtime)?;
        assert_eq!(retained["frontier"]["needs_reconciliation"], true);
        assert_eq!(retained["frontier"]["all_applied"], false);
        if case == "post_marker" {
            assert_eq!(
                retained["frontier"]["entries"][1]["phase"],
                "dispatch_started"
            );
            assert_eq!(
                retained["frontier"]["entries"][1]["dispatch_attempted"],
                true
            );
        }
        assert_eq!(retained["original_plan"], planned["original_plan"]);
        assert_eq!(
            retained["recovery"]["manager_prerequisite_digest"],
            planned["recovery"]["manager_prerequisite_digest"]
        );
        return write_output(&retained);
    }
    assert!(matches!(case, "success" | "dual"));
    let committed = committed?;
    assert_eq!(committed["frontier"]["all_applied"], true);
    assert_eq!(committed["original_goal_proven"], false);
    assert_eq!(committed["automatic_redispatch"], false);
    assert_eq!(committed["original_plan"], planned["original_plan"]);
    assert_eq!(
        committed["recovery"]["manager_prerequisite_digest"],
        planned["recovery"]["manager_prerequisite_digest"]
    );
    if case == "dual" {
        let context = runtime.context(false)?;
        let waited = runtime.wait(&context)?.value;
        assert_eq!(waited["frontier"]["all_applied"], true);
        assert_eq!(waited["frontier"]["needs_reconciliation"], false);
        assert_eq!(waited["original_plan"], planned["original_plan"]);
        assert!(waited["current_parent_goal"].is_object());
        assert_eq!(waited["original_goal_proven"], false);
        return write_output(&waited);
    }
    write_output(&committed)
}

fn checkpoint_reopen() -> TestResult {
    let retained = input()?;
    let checkpoint = &retained["manager_prerequisite"]["checkpoint"];
    assert_eq!(checkpoint["fenced"], true);
    let mut runtime = open_runtime(Mode::Control)?;
    let reopened = inspect_runtime(&mut runtime)?;
    assert_eq!(reopened["manager_prerequisite"]["checkpoint"], *checkpoint);
    assert!(reopened["original_plan"].is_null());
    let context = runtime.context(false)?;
    let observed = runtime.observe(ObserveRequest::default(), &context)?.value;
    let request = original_request(&runtime, &observed)?;
    let context = runtime.context(false)?;
    let error = match runtime.plan(request, &context) {
        Ok(_) => return Err("a fenced manager checkpoint regained planning authority".into()),
        Err(error) => error,
    };
    assert_eq!(error.code, dfmcp_core::ErrorCode::CapabilityDenied);
    assert!(error.message.contains("fenced"));
    assert!(runtime.original.is_none());
    assert_eq!(
        inspect_runtime(&mut runtime)?["manager_prerequisite"]["checkpoint"],
        *checkpoint
    );
    Ok(())
}

fn offline_or_reconcile(phase: &str) -> TestResult {
    let retained = input()?;
    let mode = if phase == "offline" {
        Mode::Offline
    } else {
        Mode::Reconcile
    };
    let mut runtime =
        open_runtime(mode).map_err(|cause| format!("{phase} open failed: {cause}"))?;
    let reopened = inspect_runtime(&mut runtime)
        .map_err(|cause| format!("{phase} initial inspection failed: {cause}"))?;
    assert_eq!(reopened["original_plan"], retained["original_plan"]);
    assert_eq!(reopened["frontier"]["needs_reconciliation"], true);
    assert_eq!(reopened["original_goal_proven"], false);
    assert!(reopened["review"].is_null());
    assert!(reopened["manager_prerequisite"]["last_qualified"].is_null());
    assert_eq!(
        reopened["recovery"]["manager_prerequisite_digest"],
        retained["recovery"]["manager_prerequisite_digest"]
    );
    if phase == "offline" {
        assert_eq!(reopened["labor_control_enabled"], false);
        assert_eq!(reopened["production_control_enabled"], false);
        let context = runtime.context(false)?;
        assert!(
            runtime
                .observe(ObserveRequest::default(), &context)
                .is_err()
        );
        return write_output(&reopened);
    }
    let context = runtime.context(false)?;
    let recovered = match runtime.wait(&context) {
        Ok(view) => view.value,
        Err(cause) => {
            let state = match inspect_runtime(&mut runtime) {
                Ok(view) => json!({
                    "all_applied":view["frontier"]["all_applied"],
                    "needs_reconciliation":view["frontier"]["needs_reconciliation"],
                    "goal_history_present":view["recovery"]["goal_history_present"]
                }),
                Err(inspection) => json!({"inspection_error":inspection.to_string()}),
            };
            return Err(format!("{phase} wait failed: {cause}; retained={state}").into());
        }
    };
    assert_eq!(recovered["frontier"]["all_applied"], true);
    assert_eq!(recovered["frontier"]["needs_reconciliation"], false);
    assert_eq!(recovered["frontier"]["entries"][1]["phase"], "applied");
    assert_eq!(recovered["original_plan"], retained["original_plan"]);
    assert_eq!(
        recovered["frontier"]["entries"][1]["idempotency_key"],
        retained["original_plan"]["steps"][1]["key"]
    );
    assert_eq!(recovered["original_goal_proven"], false);
    assert_eq!(recovered["automatic_redispatch"], false);
    write_output(&recovered)
}

fn campaign(kind: &str, test: &str) -> TestResult {
    if let Ok(phase) = std::env::var(PHASE) {
        let case = std::env::var(CASE)?;
        match phase.as_str() {
            "seed" => seed_donor()?,
            "control" => control(&case)?,
            "checkpoint" => checkpoint_reopen()?,
            "offline" | "reconcile" => offline_or_reconcile(&phase)?,
            "reject" => {
                if case == "drop-all" {
                    // Prove this reaches sealed-plan recovery, rather than
                    // passing merely because a partial config is malformed.
                    Config::from_environment(|| Ok(()))?;
                }
                let cause = match open_runtime(Mode::Offline) {
                    Ok(_) => return Err("incomplete manager recovery was accepted".into()),
                    Err(cause) => cause,
                };
                if case == "drop-all" {
                    assert!(cause.to_string().contains("manager"));
                }
            }
            _ => return Err("unknown manager runtime fixture phase".into()),
        }
        println!("combined-manager-runtime:{phase}:{case}:checked");
        return Ok(());
    }
    let full = format!("{}::{test}", module_path!());
    let (_, name) = full.split_once("::").ok_or("runtime test name")?;
    let driver = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/stdio/combined_manager_prerequisite_runtime_peer.py");
    let output = Command::new("python3")
        .arg("-B")
        .arg(driver)
        .arg(std::env::current_exe()?)
        .args([name, kind])
        .output()?;
    assert!(
        output.status.success(),
        "manager Runtime campaign {kind}: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains(&format!("combined-manager-campaign:{kind}:checked"))
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
    Ok(())
}

#[test]
fn native_manager_prerequisite_qualifies_and_rechecks_each_order_boundary() -> TestResult {
    campaign(
        "gates",
        "native_manager_prerequisite_qualifies_and_rechecks_each_order_boundary",
    )
}

#[test]
fn native_manager_prerequisite_preserves_original_recovery_and_offline_custody() -> TestResult {
    campaign(
        "recovery",
        "native_manager_prerequisite_preserves_original_recovery_and_offline_custody",
    )
}
