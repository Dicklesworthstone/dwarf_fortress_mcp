//! Real production Runtime, private donor custody and the native TCP fixtures.
//! Child processes isolate operator configuration without global environment
//! mutation. No MCP server, live game or production admission is simulated.
use super::*;
use dfmcp_adapter::build_placement::rpc::BuildCancellation;
use dfmcp_adapter::workshop_construction::acknowledgement::{
    WorkshopAcknowledgements, open_private_acknowledgements,
};
use dfmcp_adapter::workshop_construction::journal::{
    WorkshopGuard, WorkshopMode, WorkshopPin, WorkshopStage, open_private_workshop,
};
use dfmcp_adapter::workshop_construction::rpc::{WorkshopRpc, WorkshopSource};
use dfmcp_adapter::workshop_construction::{
    FortressIdentity, WorkshopBinding, WorkshopPlan, WorkshopSelection,
};
use std::path::Path;
use std::process::Command;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
const PHASE: &str = "COMBINED_WORKSHOP_RUNTIME_PHASE";
const CASE: &str = "COMBINED_WORKSHOP_RUNTIME_CASE";
const INPUT: &str = "COMBINED_WORKSHOP_RUNTIME_INPUT";
const OUTPUT: &str = "COMBINED_WORKSHOP_RUNTIME_OUTPUT";

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

struct SeedGuard {
    binding: WorkshopBinding,
    acknowledgements: WorkshopAcknowledgements<File>,
}
impl WorkshopGuard for SeedGuard {
    fn acknowledge(&mut self, pin: WorkshopPin, context: &OperationContext) -> Result<()> {
        self.acknowledgements.acknowledge(pin, context)
    }
    fn check(
        &mut self,
        stage: WorkshopStage,
        binding: &WorkshopBinding,
        selection: WorkshopSelection,
        _item_position: Option<[u32; 3]>,
        context: &OperationContext,
    ) -> Result<()> {
        if binding != &self.binding || selection != WorkshopSelection::new(42, [15, 15, 2])? {
            return Err(denied("workshop fixture selection or binding changed"));
        }
        dfmcp_adapter::workshop_construction::journal::authorize(
            context,
            binding,
            context.anchor.tick.get(),
            stage,
        )?;
        self.acknowledgements.verify(context)
    }
}

/// Create the donor with the actual native client and durable owners. In
/// particular the independent ACK covers DispatchStarted before CommitWorkshop.
fn seed_donor() -> TestResult {
    let mut value = input()?;
    let directory = Path::new(text(&value["directory"])?).to_owned();
    let fortress = FortressIdentity::new(
        text(&value["world_folder"])?,
        u32::try_from(number(&value["site_id"])?)?,
    )?;
    let fortress_id = fortress.fortress_id();
    let binding = WorkshopBinding::from_source_pins(
        text(&value["endpoint"])?.parse()?,
        fortress.clone(),
        number(&value["generation"])?,
        serde_json::from_value(value["map_dimensions"].clone())?,
        text(&value["df_version"])?,
        text(&value["dfhack_version"])?,
    )?;
    let context = OperationContext {
        session_id: SessionId::new(28_001),
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
            Capability::Construct,
        ]
        .into_iter()
        .map(|capability| CapabilityGrant {
            capability,
            max_risk: RiskTier::Guarded,
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
    let mut journal = open_private_workshop(
        &directory.join("workshop.journal"),
        &context,
        WorkshopMode::Control,
        Some(binding.clone()),
        [81; 32],
        None,
    )?;
    let mut guard = SeedGuard {
        binding: binding.clone(),
        acknowledgements: open_private_acknowledgements(
            &directory.join("workshop.ack"),
            &context,
            WorkshopMode::Control,
            true,
            None,
        )?,
    };
    guard.acknowledge(journal.pin(), &context)?;
    let mut source = WorkshopRpc::connect(
        binding.endpoint(),
        vec![b't'; 32],
        [82; 32],
        fortress,
        &context,
        BuildCancellation::default(),
        Box::new(|_| Ok(())),
    )?;
    let capture = source.observe(
        WorkshopSelection::new(42, [15, 15, 2])?,
        &context,
        Duration::from_secs(5),
    )?;
    assert_eq!(
        binding,
        WorkshopBinding::new(
            binding.endpoint(),
            binding.df_version(),
            binding.dfhack_version(),
            &capture
        )?
    );
    let plan = WorkshopPlan::new("combined-workshop-donor-1", capture)?;
    journal.prepare(&mut source, &plan, &context, &mut guard)?;
    let placed = journal.commit(&mut source, plan.key(), plan.digest(), &context, &mut guard)?;
    assert!(placed.placed());
    guard.acknowledge(journal.pin(), &context)?;
    let completed = journal.sample(&mut source, plan.key(), plan.digest(), &context, &mut guard)?;
    assert!(completed.goal_complete());
    let pin = journal.pin();
    guard.acknowledge(pin, &context)?;
    assert_eq!(guard.acknowledgements.latest(), Some(pin));
    value
        .as_object_mut()
        .ok_or("fixture object")?
        .remove("capture_tick");
    value["acknowledged"] = json!({
        "journal_id":pin.journal_id.to_hex(),
        "head":pin.head.to_hex(),
        "frames":pin.frames
    });
    value["original_key"] = json!(plan.key());
    value["original_plan_digest"] = json!(plan.digest().to_hex());
    write_output(&value)
}

fn open_runtime(mode: Mode) -> TestResult<Runtime> {
    let session = match (mode, std::env::var(PHASE).ok().as_deref()) {
        (Mode::Control, Some("checkpoint")) => 28_005,
        (Mode::Control, _) => 28_002,
        (Mode::Offline, _) => 28_003,
        (Mode::Reconcile, _) => 28_004,
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
    let request = original_request(&runtime, &observed)?;
    let context = runtime.context(false)?;
    let planned = runtime.plan(request, &context);
    if matches!(case, "unfinished" | "removed" | "generation") {
        let error = match planned {
            Ok(_) => return Err("unqualified workshop unexpectedly allowed a plan".into()),
            Err(error) => error,
        };
        assert_eq!(
            error.code,
            if case == "unfinished" {
                dfmcp_core::ErrorCode::Conflict
            } else {
                dfmcp_core::ErrorCode::StaleAnchor
            }
        );
        assert!(runtime.original.is_none());
        assert!(!runtime.config.path("original.workshop").exists());
        let retained = inspect_runtime(&mut runtime)?;
        let checkpoint = &retained["workshop_prerequisite"]["checkpoint"];
        assert_eq!(
            checkpoint["origin_witness"],
            retained["workshop_prerequisite"]["origin_witness"]
        );
        assert!(number(&checkpoint["tick"])? >= runtime.anchor().tick.get());
        assert!(number(&checkpoint["next_building_id"])? > 70);
        assert_eq!(checkpoint["fenced"], case != "unfinished");
        return write_output(&retained);
    }
    let planned = planned?.value;
    let plan = digest_value(&planned["original_plan"]["plan_digest"])?;
    let association = digest_value(&planned["recovery"]["workshop_prerequisite_digest"])?;
    assert_ne!(association, Digest32::ZERO);
    assert!(runtime.config.path("original.workshop").is_file());
    let labor = prepare(&mut runtime, plan)?;
    assert_eq!(labor["review"]["step_id"], 0);
    commit(&mut runtime, &labor)?;
    if case == "before_prepare" {
        assert!(prepare(&mut runtime, plan).is_err());
        assert!(
            !inspect_runtime(&mut runtime)?["frontier"]["all_applied"]
                .as_bool()
                .ok_or("frontier completion flag")?
        );
        return Ok(());
    }
    let order = prepare(&mut runtime, plan)?;
    assert_eq!(order["review"]["step_id"], 1);
    let committed = commit(&mut runtime, &order);
    if matches!(case, "after_prepare" | "lost") {
        assert!(committed.is_err());
        let retained = inspect_runtime(&mut runtime)?;
        assert_eq!(retained["frontier"]["needs_reconciliation"], true);
        assert_eq!(retained["frontier"]["all_applied"], false);
        assert_eq!(retained["original_plan"], planned["original_plan"]);
        assert_eq!(
            retained["recovery"]["workshop_prerequisite_digest"],
            planned["recovery"]["workshop_prerequisite_digest"]
        );
        return write_output(&retained);
    }
    assert_eq!(case, "success");
    let committed = committed?;
    assert_eq!(committed["frontier"]["all_applied"], true);
    assert_eq!(committed["original_goal_proven"], false);
    assert_eq!(committed["automatic_redispatch"], false);
    assert_eq!(committed["original_plan"], planned["original_plan"]);
    assert_eq!(
        committed["recovery"]["workshop_prerequisite_digest"],
        planned["recovery"]["workshop_prerequisite_digest"]
    );
    write_output(&committed)
}

fn checkpoint_reopen() -> TestResult {
    let retained = input()?;
    let checkpoint = &retained["workshop_prerequisite"]["checkpoint"];
    assert_eq!(checkpoint["fenced"], true);
    let mut runtime = open_runtime(Mode::Control)?;
    let reopened = inspect_runtime(&mut runtime)?;
    assert_eq!(reopened["workshop_prerequisite"]["checkpoint"], *checkpoint);
    assert!(reopened["original_plan"].is_null());
    let context = runtime.context(false)?;
    let observed = runtime.observe(ObserveRequest::default(), &context)?.value;
    let request = original_request(&runtime, &observed)?;
    let context = runtime.context(false)?;
    let error = match runtime.plan(request, &context) {
        Ok(_) => return Err("a fenced workshop checkpoint regained planning authority".into()),
        Err(error) => error,
    };
    assert_eq!(error.code, dfmcp_core::ErrorCode::CapabilityDenied);
    assert!(error.message.contains("fenced"));
    assert!(runtime.original.is_none());
    assert_eq!(
        inspect_runtime(&mut runtime)?["workshop_prerequisite"]["checkpoint"],
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
    assert_eq!(
        reopened["recovery"]["workshop_prerequisite_digest"],
        retained["recovery"]["workshop_prerequisite_digest"]
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
                    Ok(_) => return Err("incomplete workshop recovery was accepted".into()),
                    Err(cause) => cause,
                };
                if case == "drop-all" {
                    assert!(cause.to_string().contains("workshop"));
                }
            }
            _ => return Err("unknown workshop runtime fixture phase".into()),
        }
        println!("combined-workshop-runtime:{phase}:{case}:checked");
        return Ok(());
    }
    let full = format!("{}::{test}", module_path!());
    let (_, name) = full.split_once("::").ok_or("runtime test name")?;
    let driver = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/stdio/combined_workshop_runtime_peer.py");
    let output = Command::new("python3")
        .arg("-B")
        .arg(driver)
        .arg(std::env::current_exe()?)
        .args([name, kind])
        .output()?;
    assert!(
        output.status.success(),
        "workshop Runtime campaign {kind}: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains(&format!("combined-workshop-campaign:{kind}:checked"))
    );
    Ok(())
}

#[test]
fn native_workshop_prerequisite_qualifies_and_rechecks_each_order_boundary() -> TestResult {
    campaign(
        "gates",
        "native_workshop_prerequisite_qualifies_and_rechecks_each_order_boundary",
    )
}

#[test]
fn native_workshop_prerequisite_preserves_original_recovery_and_offline_custody() -> TestResult {
    campaign(
        "recovery",
        "native_workshop_prerequisite_preserves_original_recovery_and_offline_custody",
    )
}
