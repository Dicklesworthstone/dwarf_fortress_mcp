//! Actual child publication and restart against the existing native TCP peers.
//! Project cases inject faults at the parent ACK; ordinary cases use the public
//! portfolio planners without a parent binding or ACK. The parent ledger is
//! tested separately. These are component tests, not live-game admission.
use super::*;
use std::path::Path;
use std::process::Command;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
const PHASE: &str = "PROJECT_PUBLICATION_RUNTIME_PHASE";
const CASE: &str = "PROJECT_PUBLICATION_RUNTIME_CASE";
const INPUT: &str = "PROJECT_PUBLICATION_RUNTIME_INPUT";
const OUTPUT: &str = "PROJECT_PUBLICATION_RUNTIME_OUTPUT";
const STAGE: &str = "PROJECT_PUBLICATION_RUNTIME_STAGE";
const CLOCKS: &str = "PROJECT_PUBLICATION_RUNTIME_CLOCKS";
const ONE_ORDER: &str = "PROJECT_PUBLICATION_RUNTIME_ONE_ORDER";
const ASSIGN_LABOR: &str = "PROJECT_PUBLICATION_RUNTIME_ASSIGN_LABOR";
const ORDER_COUNT: &str = "PROJECT_PUBLICATION_RUNTIME_ORDER_COUNT";
const FOREIGN_CLOCK: &[u8] = b"foreign clock bytes must never be adopted or overwritten";
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
        max_game_ticks: if std::env::var(CLOCKS).is_ok() { 1200 } else { 0 },
    }
}

fn expected_order_steps() -> Vec<u32> {
    let first = u32::from(std::env::var(ASSIGN_LABOR).as_deref() != Ok("0"));
    let fallback = if std::env::var(ONE_ORDER).as_deref() == Ok("1") { 1 } else { 2 };
    let count = std::env::var(ORDER_COUNT)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(fallback);
    assert!((1..=4).contains(&count));
    (first..first + count).collect()
}

fn publication_case(requested: &str) -> (bool, bool, &str) {
    let portfolio = requested.starts_with("portfolio_");
    let case = requested.strip_prefix("portfolio_").unwrap_or(requested);
    match case.strip_prefix("reopened_") {
        Some(case) => (portfolio, true, case),
        None if case == "reopened" => (portfolio, true, "published"),
        None => (portfolio, false, case),
    }
}

fn foreign_clock_file(path: &Path) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|_| invalid("cannot inject foreign project clock"))?;
    file.write_all(FOREIGN_CLOCK)
        .map_err(|_| invalid("cannot write foreign project clock"))
}

fn check_portfolio_clocks(reference: &Value, order: bool, inventory: bool) -> TestResult {
    let slots = reference["recovery"]["portfolio"].as_array().ok_or("missing original slots")?;
    assert_eq!(slots.len(), expected_order_steps().len());
    assert_eq!(reference["clock"].is_object(), order);
    assert_eq!(reference["inventory_clock"].is_object(), inventory);
    if !order && !inventory {
        assert!(reference.get("portfolio_clocks").is_none());
        for slot in slots {
            assert!(slot["clock_journal_id"].is_null());
            assert!(slot["inventory_clock_journal_id"].is_null());
        }
        return Ok(());
    }
    let clocks = reference["portfolio_clocks"].as_array().ok_or("missing complete clock pins")?;
    assert_eq!(clocks.len(), slots.len());
    assert_eq!(reference["clock"], clocks[0]["clock"]);
    assert_eq!(reference["inventory_clock"], clocks[0]["inventory_clock"]);
    let mut ids = std::collections::BTreeSet::new();
    for (slot, clocks) in slots.iter().zip(clocks) {
        assert_eq!(slot["step_id"], clocks["step_id"]);
        for (family, field, configured) in [
            ("clock", "clock_journal_id", order),
            ("inventory_clock", "inventory_clock_journal_id", inventory),
        ] {
            let pin = &clocks[family];
            assert_eq!(pin.is_object(), configured);
            assert_eq!(slot[field], pin["journal_id"]);
            if configured {
                let id = pin["journal_id"].as_str().ok_or("missing clock identity")?;
                assert!(ids.insert(id.to_owned()), "two original clocks share a journal");
                assert_eq!(pin["head"], pin["journal_id"], "publication advanced a clock");
                assert_eq!(pin["high_source_sequence"], 0);
            }
        }
    }
    Ok(())
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
    let scalar_request = runtime
        .stock_target_no_work
        .as_ref()
        .map(|watch| watch.request.clone());
    let portfolio_request = runtime
        .stock_targets_no_work
        .as_ref()
        .map(|watch| watch.request.clone());
    assert_eq!(scalar_request.is_some(), !goal.is_portfolio());
    assert_eq!(portfolio_request.is_some(), goal.is_portfolio());
    let original_watch = runtime.no_work_goal_json();
    let witness = original_watch["goal_witness"].clone();
    assert!(witness.is_string());
    if goal.is_portfolio() {
        assert_eq!(original_watch["targets"].as_array().map(Vec::len), Some(3));
    }
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
    assert_eq!(
        runtime
            .stock_target_no_work
            .as_ref()
            .map(|watch| &watch.request),
        scalar_request.as_ref()
    );
    assert_eq!(
        runtime
            .stock_targets_no_work
            .as_ref()
            .map(|watch| &watch.request),
        portfolio_request.as_ref()
    );
    let retained_watch = runtime.no_work_goal_json();
    assert_eq!(retained_watch["goal_witness"], witness);
    assert_eq!(retained_watch["targets"], original_watch["targets"]);
    assert!(retained_watch["sample"].is_null());
    if let Some(watch) = &runtime.stock_target_no_work {
        assert!(watch.monitor.latest().is_none());
    }
    if let Some(watch) = &runtime.stock_targets_no_work {
        assert!(watch.monitor.latest().is_none());
    }
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

fn control(requested_case: &str) -> TestResult {
    let (portfolio, reopening, case) = publication_case(requested_case);
    let retained: Option<Value> = if reopening {
        Some(serde_json::from_slice(&std::fs::read(std::env::var(
            INPUT,
        )?)?)?)
    } else {
        None
    };
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
    let mut runtime = if let Some(retained) = &retained {
        let goal: ProjectStockGoal = serde_json::from_value(retained["goal"].clone())?;
        assert!(goal.is_portfolio());
        assert!(retained["actual"]["recovery"].is_null());
        let origin = workforce::digest(
            retained["actual"]["workshop_prerequisite"]["checkpoint"]["origin_witness"]
                .as_str()
                .ok_or("missing pre-plan Workshop origin")?,
        )?;
        let mut config = Config::from_environment(|| Ok(()))?;
        assert!(!config.initialize && config.recovery.is_none());
        bind_project(&mut config, &goal, origin);
        config.validate_project_recovery(&retained["actual"])?;
        let runtime = Runtime::open_for_resource_project(
            config,
            Mode::Control,
            SessionId::new(30_003),
            budget(),
            retained["actual"]["observation_tick_floor"]
                .as_u64()
                .ok_or("missing pre-plan parent floor")?,
        )?;
        assert_eq!(
            runtime.project_recovery_reference()["native_custody"],
            retained["actual"]["native_custody"]
        );
        assert!(runtime.original.is_none());
        assert!(
            runtime.stock_targets_no_work.is_none(),
            "reopen restored a process-local watch"
        );
        runtime
    } else {
        Runtime::open(
            Config::from_environment(|| Ok(()))?,
            Mode::Control,
            SessionId::new(30_001),
            budget(),
        )?
    };
    let c = runtime.context(false)?;
    let observed = runtime.observe(ObserveRequest::default(), &c)?.value;
    let goal: ProjectStockGoal = if let Some(retained) = &retained {
        serde_json::from_value(retained["goal"].clone())?
    } else {
        let mut value = json!({
            "deadline_tick":runtime.anchor.tick.get() + 300,
            "poll_interval_ticks":5,"stable_samples":2,
            "assign_labor":std::env::var(ASSIGN_LABOR).as_deref() != Ok("0"),
        });
        if portfolio {
            value["targets"] = json!([
                {"recipe":"wooden_bed","minimum_stack_units":6,"max_order_amount":100},
                {"recipe":"wooden_chair","minimum_stack_units":2,"max_order_amount":10},
                {"recipe":"wooden_door","minimum_stack_units":1,"max_order_amount":10}
            ]);
        } else {
            value["recipe"] = json!("wooden_bed");
            value["minimum_stack_units"] = json!(6);
            value["max_order_amount"] = json!(100);
        }
        serde_json::from_value(value)?
    };
    let origin = runtime
        .workshop
        .as_ref()
        .ok_or("missing Workshop fixture")?
        .witness()?;
    let before_binding = runtime.project_recovery_reference();
    if !reopening {
        assert!(
            runtime
                .retain_project_authority_tick(runtime.anchor.tick.get() + 1)
                .is_err()
        );
        assert_eq!(runtime.project_recovery_reference(), before_binding);
    }
    bind_project(&mut runtime.config, &goal, origin);
    let mut request = serde_json::to_value(&goal)?;
    request["kind"] = json!(if portfolio {
        "stock_targets"
    } else {
        "stock_target"
    });
    request["expected_observation"] = observed["observation"]["seal"].clone();
    request["units"] = json!(["43"]);
    request["labor"] = json!("CARPENTER");
    let prior = runtime.project_recovery_reference();
    assert!(prior["recovery"].is_null());
    let directory = runtime.config.directory.clone();
    let order_config: Value = serde_json::from_str(&std::env::var("DFMCP_SEMANTIC_WORK_ORDER_CONFIG")?)?;
    let order_directory = Path::new(order_config["directory"].as_str().ok_or("missing order directory")?);
    let order_clock = runtime.config.clock.is_some();
    let inventory_clock = runtime.config.inventory_clock.is_some();
    let collision_path = order_directory.join(if inventory_clock {
        "portfolio-2-inventory-clock.journal"
    } else {
        "portfolio-2-clock.journal"
    });
    if case == "existing_slot" {
        assert!(portfolio && (order_clock || inventory_clock));
        foreign_clock_file(&collision_path)?;
    }
    let before_plan = custody_bytes()?;
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
        for slot in 2..=4 {
            for family in ["clock", "inventory-clock"] {
                assert!(
                    !order_directory.join(format!("portfolio-{slot}-{family}.journal")).exists(),
                    "child wrote an additional clock before parent ACK"
                );
            }
        }
        intent.validate_checkpoint(&prior)?;
        let intended = intent.to_json();
        if portfolio {
            check_portfolio_clocks(&intended["recovery_reference"], order_clock, inventory_clock)
                .map_err(|_| invalid("prospective clock publication fixture"))?;
            assert_eq!(intended["recovery_reference"]["clock"], prior["clock"]);
            assert_eq!(intended["recovery_reference"]["inventory_clock"], prior["inventory_clock"]);
        }
        recorded = Some(intended);
        if case == "ack_refusal" {
            return Err(denied("injected parent ACK refusal"));
        }
        if case == "slot_collision" {
            foreign_clock_file(&collision_path)?;
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
    let result = if portfolio {
        runtime.project_plan_stock_targets_recorded(
            &goal,
            StockTargetsRequest::parse(&request.to_string())?,
            &c,
            &mut record,
        )
    } else {
        runtime.project_plan_stock_target_recorded(
            &goal,
            StockTargetRequest::parse(&request.to_string())?,
            &c,
            &mut record,
        )
    };
    match case {
        "no_work" | "no_work_floor" => {
            let view = result?.value;
            assert_eq!(calls, 0);
            assert!(recorded.is_none());
            assert!(view["recovery"].is_null());
            let target_view = if portfolio {
                &view["stock_targets"]
            } else {
                &view["stock_target"]
            };
            assert_eq!(target_view["work_required"], false);
            assert!(!runtime.project_status(&goal)?.original_goal_proven);
            if portfolio {
                assert_eq!(target_view["targets"].as_array().map(Vec::len), Some(3));
                assert_eq!(
                    view["no_work_stock_goal"]["targets"]
                        .as_array()
                        .map(Vec::len),
                    Some(3)
                );
                assert!(runtime.stock_targets_no_work.is_some());
                assert!(runtime.portfolio.is_none());
            }
            for name in ORIGINAL_FILES {
                assert!(!directory.join(name).exists());
            }
            if case == "no_work_floor" {
                no_work_parent_floor(&mut runtime, &goal)?;
            }
        }
        "existing_slot" | "missing_authority" => {
            assert!(portfolio && (order_clock || inventory_clock));
            assert!(result.is_err());
            assert_eq!(calls, 0, "invalid new-slot publication reached parent ACK");
            assert!(recorded.is_none());
            assert!(!runtime.initialization_failed);
            assert!(runtime.original.is_none());
            assert_eq!(custody_bytes()?, before_plan);
        }
        "slot_collision" => {
            assert!(result.is_err());
            assert_eq!(calls, 1);
            assert!(runtime.initialization_failed);
            assert!(directory.join("original.plan").is_file());
            assert!(directory.join("execution.journal").is_file());
            if order_clock && inventory_clock {
                assert!(
                    order_directory.join("portfolio-2-clock.journal").is_file(),
                    "fixture must fail after creating the first additional header"
                );
            }
            assert_eq!(std::fs::read(&collision_path)?, FOREIGN_CLOCK);
        }
        "ack_refusal" => {
            assert!(result.is_err());
            assert_eq!(calls, 1);
            assert!(!runtime.initialization_failed);
            assert_eq!(custody_bytes()?, before_plan, "ACK refusal wrote child custody");
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
            if portfolio {
                let layout = runtime
                    .layout
                    .as_ref()
                    .ok_or("missing original portfolio layout")?;
                assert_eq!(
                    layout
                        .order_steps()
                        .iter()
                        .map(|step| step.get())
                        .collect::<Vec<_>>(),
                    expected_order_steps()
                );
                assert_eq!(
                    layout.inventory_minima().len(),
                    3,
                    "already-met door target was dropped"
                );
                let reference = runtime.project_recovery_reference();
                assert_eq!(
                    reference["recovery"]["portfolio"].as_array().map(Vec::len),
                    Some(expected_order_steps().len())
                );
                assert_eq!(
                    recorded.as_ref().ok_or("missing portfolio intent")?["goal"]["targets"]
                        .as_array()
                        .map(Vec::len),
                    Some(3)
                );
                check_portfolio_clocks(&reference, order_clock, inventory_clock)?;
                let selected = *expected_order_steps().last().ok_or("missing original selector")?;
                let c = runtime.context(false)?;
                runtime.select_order(Some(selected), &c)?;
                assert_eq!(
                    runtime.project_recovery_reference(),
                    reference,
                    "selecting an inactive leaf changed original custody"
                );
                runtime.validate_project_publication(&goal, &intent)?;
                assert!(!runtime.project_status(&goal)?.original_goal_proven);
                for entry in reference["recovery"]["portfolio"]
                    .as_array()
                    .ok_or("missing slots")?
                {
                    assert_eq!(entry["goal_history_present"], false);
                    assert_eq!(entry["clock_journal_id"].is_string(), order_clock);
                    assert_eq!(entry["inventory_clock_journal_id"].is_string(), inventory_clock);
                }
            }
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

fn recover(requested_case: &str) -> TestResult {
    let (_, _, case) = publication_case(requested_case);
    let retained: Value = serde_json::from_slice(&std::fs::read(std::env::var(INPUT)?)?)?;
    let goal: ProjectStockGoal = serde_json::from_value(retained["goal"].clone())?;
    let intent = match ProjectProductionIntent::from_json(&retained["intent"]) {
        Ok(intent) => intent,
        Err(_) if !matches!(case, "published" | "published_floor") => return Ok(()),
        Err(cause) => return Err(cause.into()),
    };
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
        })
        .and_then(|runtime| {
            runtime.validate_project_publication(&goal, &intent)?;
            Ok(runtime)
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
    if goal.is_portfolio() {
        assert_eq!(
            reference["recovery"]["portfolio"].as_array().map(Vec::len),
            Some(expected_order_steps().len())
        );
        assert_eq!(
            runtime
                .layout
                .as_ref()
                .ok_or("missing recovered layout")?
                .inventory_minima()
                .len(),
            3
        );
        check_portfolio_clocks(
            &reference,
            runtime.config.clock.is_some(),
            runtime.config.inventory_clock.is_some(),
        )?;
        assert_eq!(reference["portfolio_clocks"], expected["portfolio_clocks"]);
        let selected = *expected_order_steps().last().ok_or("missing recovered selector")?;
        let c = runtime.context(false)?;
        runtime.select_order(Some(selected), &c)?;
        assert_eq!(runtime.project_recovery_reference(), reference);
        runtime.validate_project_publication(&goal, &intent)?;
    }
    let c = runtime.context(false)?;
    let view = runtime.inspect(&c)?.value;
    assert!(view["review"].is_null());
    assert_eq!(view["original_goal_proven"], false);
    assert_eq!(view["automatic_redispatch"], false);
    output(&reference)
}


fn ordinary(phase: &str, case: &str) -> TestResult {
    let config = Config::from_environment(|| Ok(()))?;
    assert!(config.project_binding.is_none(), "ordinary host borrowed a parent ACK");
    let before_open = custody_bytes()?;
    let mode = if phase == "ordinary-recover" { Mode::Offline } else { Mode::Control };
    assert_eq!(config.initialize, phase == "ordinary-open");
    assert_eq!(config.recovery.is_some(), phase == "ordinary-recover");
    let opened = Runtime::open(config, mode, SessionId::new(31_001), budget());
    if phase == "ordinary-recover" && case != "published" {
        assert!(opened.is_err(), "ordinary recovery accepted {case}");
        assert_eq!(custody_bytes()?, before_open, "recovery recreated original custody");
        return Ok(());
    }
    let mut runtime = opened?;
    if phase == "ordinary-open" {
        let c = runtime.context(false)?;
        runtime.observe(ObserveRequest::default(), &c)?;
        assert!(runtime.original.is_none());
        return output(&json!({
            "intent": null, "actual": runtime.project_recovery_reference(),
            "workforce_journal_id": runtime.workforce_journal.to_hex(),
        }));
    }
    let retained: Value = serde_json::from_slice(&std::fs::read(std::env::var(INPUT)?)?)?;
    assert_eq!(runtime.workforce_journal.to_hex(), retained["workforce_journal_id"]);
    assert_eq!(
        runtime.project_recovery_reference()["native_custody"],
        retained["actual"]["native_custody"]
    );
    if phase == "ordinary-recover" {
        let reference = runtime.project_recovery_reference();
        assert_eq!(reference["recovery"], retained["actual"]["recovery"]);
        assert_eq!(reference["portfolio_clocks"], retained["actual"]["portfolio_clocks"]);
        check_portfolio_clocks(
            &reference,
            runtime.config.clock.is_some(),
            runtime.config.inventory_clock.is_some(),
        )?;
        let selected = *expected_order_steps().last().ok_or("missing recovered selector")?;
        let c = runtime.context(false)?;
        runtime.select_order(Some(selected), &c)?;
        let c = runtime.context(false)?;
        let view = runtime.inspect(&c)?.value;
        assert!(view["review"].is_null());
        assert_eq!(view["original_goal_proven"], false);
        assert_eq!(view["automatic_redispatch"], false);
        assert_eq!(runtime.project_recovery_reference(), reference);
        assert_eq!(custody_bytes()?, before_open, "ordinary recovery rewrote custody");
        return output(&reference);
    }
    assert_eq!(phase, "ordinary-plan");
    assert!(runtime.original.is_none() && runtime.frontier.is_none());
    assert!(retained["actual"]["recovery"].is_null());
    assert_eq!(custody_bytes()?, before_open, "pre-plan reopen rewrote native custody");
    let c = runtime.context(false)?;
    let observed = runtime.observe(ObserveRequest::default(), &c)?.value;
    let assign_labor = std::env::var(ASSIGN_LABOR).as_deref() != Ok("0");
    let orders = ["wooden_bed", "wooden_chair", "wooden_door", "wooden_table"]
        .into_iter()
        .take(expected_order_steps().len())
        .map(|recipe| json!({"recipe": recipe, "amount": 1}))
        .collect::<Vec<_>>();
    let mut request = json!({
        "kind": if assign_labor { "portfolio" } else { "existing_labor_portfolio" },
        "expected_observation": observed["observation"]["seal"],
        "units": ["43"], "labor": "CARPENTER", "orders": orders,
        "deadline_tick": runtime.anchor.tick.get() + 300,
        "poll_interval_ticks": 5, "stable_samples": 2,
    });
    if assign_labor {
        request["enabled"] = json!(true);
    }
    if case == "existing_slot" {
        let configured: Value = serde_json::from_str(
            &std::env::var("DFMCP_SEMANTIC_WORK_ORDER_CONFIG")?,
        )?;
        let directory = Path::new(configured["directory"].as_str().ok_or("missing order directory")?);
        foreign_clock_file(&directory.join(if runtime.config.inventory_clock.is_some() {
            "portfolio-2-inventory-clock.journal"
        } else {
            "portfolio-2-clock.journal"
        }))?;
    }
    let before_plan = custody_bytes()?;
    let mut c = runtime.context(false)?;
    if case == "missing_context_authority" {
        c.grants.retain(|grant| grant.capability != Capability::ControlClock);
    }
    let result = if assign_labor {
        runtime.plan_portfolio(PortfolioRequest::parse(&request.to_string())?, &c)
    } else {
        runtime.plan_existing_labor_portfolio(
            ExistingLaborPortfolioRequest::parse(&request.to_string())?,
            &c,
        )
    };
    if matches!(case, "missing_authority" | "missing_context_authority" | "existing_slot") {
        assert!(result.is_err(), "ordinary publication accepted {case}");
        assert!(!runtime.initialization_failed, "preflight wrote a partial original");
        assert!(runtime.original.is_none() && runtime.frontier.is_none());
        assert_eq!(custody_bytes()?, before_plan, "refusal wrote original custody");
        return output(&json!({"refused": case}));
    }
    result?;
    assert!(!runtime.initialization_failed);
    let reference = runtime.project_recovery_reference();
    check_portfolio_clocks(
        &reference,
        runtime.config.clock.is_some(),
        runtime.config.inventory_clock.is_some(),
    )?;
    assert_eq!(
        runtime.layout.as_ref().ok_or("missing ordinary layout")?
            .order_steps().iter().map(|step| step.get()).collect::<Vec<_>>(),
        expected_order_steps()
    );
    for (path, bytes) in before_plan {
        assert_eq!(std::fs::read(path)?, bytes, "publication changed pre-plan custody");
    }
    let selected = *expected_order_steps().last().ok_or("missing ordinary selector")?;
    let c = runtime.context(false)?;
    runtime.select_order(Some(selected), &c)?;
    assert_eq!(runtime.project_recovery_reference(), reference);
    let before_repeat = custody_bytes()?;
    let c = runtime.context(false)?;
    let repeated = if assign_labor {
        runtime.plan_portfolio(PortfolioRequest::parse(&request.to_string())?, &c)
    } else {
        runtime.plan_existing_labor_portfolio(
            ExistingLaborPortfolioRequest::parse(&request.to_string())?,
            &c,
        )
    };
    assert!(repeated.is_err(), "ordinary original was replanned");
    assert_eq!(custody_bytes()?, before_repeat);
    output(&json!({
        "intent": null, "actual": reference,
        "workforce_journal_id": runtime.workforce_journal.to_hex(),
    }))
}

#[test]
fn recorded_project_publication_recovers_exact_original_without_reexecution() -> TestResult {
    if let Ok(phase) = std::env::var(PHASE) {
        let case = std::env::var(CASE)?;
        match phase.as_str() {
            "control" => control(&case)?,
            "recover" => recover(&case)?,
            "ordinary-open" | "ordinary-plan" | "ordinary-recover" => ordinary(&phase, &case)?,
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
