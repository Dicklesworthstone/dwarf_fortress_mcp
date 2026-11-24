//! Real stock-target portfolio Runtime over independently encoded TCP doubles.
//! The source qualification, immutable originals, private custody and native
//! clients are production owners; these tests are not live-game qualification.
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
    value
        .as_str()
        .ok_or_else(|| "missing stock-targets fixture text".into())
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
pub(super) fn custody_bytes() -> TestResult<std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>>
{
    let mut files = std::collections::BTreeMap::new();
    for name in [
        "DFMCP_COMBINED_PRODUCTION_CONFIG",
        "DFMCP_SEMANTIC_WORKFORCE_CONFIG",
        "DFMCP_SEMANTIC_WORK_ORDER_CONFIG",
    ] {
        let config: Value = serde_json::from_str(&std::env::var(name)?)?;
        let directory = config["directory"]
            .as_str()
            .ok_or("missing custody directory")?;
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                files.insert(entry.path(), std::fs::read(entry.path())?);
            }
        }
    }
    Ok(files)
}
fn open(mode: Mode) -> TestResult<Runtime> {
    let session = match mode {
        Mode::Control => 30_001,
        Mode::Offline => 30_002,
        Mode::Reconcile => 30_003,
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
            max_game_ticks: if std::env::var(CASE).is_ok_and(|case| case.ends_with("parent_clock"))
            {
                5
            } else {
                0
            },
        },
    )?)
}
fn inspect(runtime: &mut Runtime) -> TestResult<Value> {
    let context = runtime.context(false)?;
    Ok(runtime.inspect(&context)?.value)
}
fn prepare(runtime: &mut Runtime, plan: Digest32) -> TestResult<Value> {
    let context = runtime.context(false)?;
    Ok(runtime
        .prepare_next(plan, &context)
        .map_err(|cause| format!("stock-targets Prepare: {cause}"))?
        .value)
}
fn commit(runtime: &mut Runtime, prepared: &Value) -> TestResult<Value> {
    let seal = digest(&prepared["review"]["paired_seal"])?;
    let context = runtime.context(false)?;
    Ok(runtime
        .commit(seal, true, &context)
        .map_err(|cause| format!("stock-targets Commit: {cause}"))?
        .value)
}
fn request(runtime: &Runtime, observed: &Value, case: &str) -> TestResult<StockTargetsRequest> {
    let assignment = case.starts_with("assignment_");
    let case = case.strip_prefix("assignment_").unwrap_or(case);
    let mut value = json!({
        "kind":"stock_targets", "expected_observation":observed["observation"]["seal"],
        "units":["43"], "labor":"MINE",
        "targets":[
            {"recipe":"wooden_door","minimum_stack_units":3},
            {"recipe":"wooden_bed","minimum_stack_units":6}
        ],
        "deadline_tick":runtime.anchor().tick.get() + 300,
        "poll_interval_ticks":5, "stable_samples":2,
    });
    if case == "cap" {
        value["targets"][1]["max_order_amount"] = json!(2);
    }
    if assignment {
        value["assign_labor"] = true.into();
    }
    if case == "four" {
        let targets = value["targets"]
            .as_array_mut()
            .ok_or("target fixture array")?;
        targets.extend([
            json!({"recipe":"wooden_chair","minimum_stack_units":1}),
            json!({"recipe":"wooden_table","minimum_stack_units":1}),
        ]);
    }
    Ok(StockTargetsRequest::parse(&value.to_string())?)
}
fn assert_same(current: &Value, original: &Value) {
    assert_eq!(current["original_plan"], original["original_plan"]);
    for field in [
        "schema",
        "status",
        "targets",
        "baseline_anchor",
        "source_digest",
        "source_generation",
        "source_counters",
        "item_rows",
        "assessment_witness",
        "work_required",
        "mutation_planned",
        "all_target_minima_retained",
        "labor_assignment_requested",
        "labor_assignment_planned",
        "worker_prerequisite_checked",
    ] {
        assert_eq!(
            current["stock_targets"][field], original["stock_targets"][field],
            "original target vector changed: {field}"
        );
    }
    assert_eq!(current["automatic_redispatch"], false);
}
fn assert_report(view: &Value, case: &str) -> TestResult {
    let assignment = case.starts_with("assignment_");
    let case = case.strip_prefix("assignment_").unwrap_or(case);
    let report = &view["stock_targets"];
    assert_eq!(report["schema"], "dfmcp.stock-targets/1");
    assert_eq!(
        report["status"],
        if case == "already" {
            "already_satisfied"
        } else {
            "planned"
        }
    );
    assert_eq!(report["current_inventory_proven"], false);
    assert_eq!(report["fresh_for_current_call"], false);
    assert_eq!(report["labor_assignment_requested"], assignment);
    assert_eq!(
        report["labor_assignment_planned"],
        assignment && case != "already"
    );
    assert_eq!(report["all_target_minima_retained"], true);
    assert_ne!(digest(&report["assessment_witness"])?, Digest32::ZERO);
    assert_ne!(digest(&report["source_digest"])?, Digest32::ZERO);
    assert_eq!(report["source_generation"], 71);
    let rows = report["targets"]
        .as_array()
        .ok_or("complete target vector")?;
    assert_eq!(rows.len(), if case == "four" { 4 } else { 2 });
    if case == "four" {
        for (index, (kind, baseline, target)) in [
            ("BED", 2, 6),
            ("CHAIR", 0, 1),
            ("DOOR", 1, 3),
            ("TABLE", 0, 1),
        ]
        .into_iter()
        .enumerate()
        {
            let row = &rows[index];
            assert_eq!(row["item_type"], kind);
            assert_eq!(row["baseline_stack_units"], baseline);
            assert_eq!(row["target_stack_units"], target);
            assert_eq!(row["amount"], target - baseline);
            assert_eq!(row["order_step_id"], index + usize::from(assignment));
            assert_eq!(row["labor_assignment_requested"], assignment);
            assert_eq!(row["labor_assignment_planned"], assignment);
        }
        return Ok(());
    }
    assert_eq!(rows[0]["recipe"], "wooden_bed");
    assert_eq!(rows[1]["recipe"], "wooden_door");
    assert_eq!(rows[0]["target_stack_units"], 6);
    assert_eq!(rows[1]["target_stack_units"], 3);
    assert_eq!(
        rows[0]["baseline_stack_units"],
        if case == "already" { 6 } else { 2 }
    );
    assert_eq!(
        rows[1]["baseline_stack_units"],
        if case.starts_with("multiple") { 1 } else { 3 }
    );
    assert_eq!(rows[0]["amount"], if case == "already" { 0 } else { 4 });
    assert_eq!(
        rows[1]["amount"],
        if case.starts_with("multiple") { 2 } else { 0 }
    );
    if case == "already" {
        assert!(rows[0]["order_step_id"].is_null());
    } else {
        assert_eq!(rows[0]["order_step_id"], u32::from(assignment));
    }
    if case.starts_with("multiple") {
        assert_eq!(rows[1]["order_step_id"], 1 + u32::from(assignment));
    } else {
        assert!(rows[1]["order_step_id"].is_null());
    }
    for row in rows {
        assert_eq!(row["source_digest"], report["source_digest"]);
        assert_eq!(row["baseline_anchor"], report["baseline_anchor"]);
        assert_ne!(digest(&row["assessment_witness"])?, Digest32::ZERO);
        assert_eq!(row["labor_assignment_requested"], assignment);
        assert_eq!(
            row["labor_assignment_planned"],
            assignment && row["amount"] != 0
        );
    }
    Ok(())
}

fn no_work_watch(runtime: &mut Runtime, planned: &Value) -> TestResult {
    let original = runtime
        .stock_targets_no_work
        .as_ref()
        .ok_or("no-work portfolio owner absent")?
        .request
        .clone();
    let start_tick = runtime.anchor.tick.get();
    let witness = planned["no_work_stock_goal"]["goal_witness"].clone();
    let files = custody_bytes()?;
    assert_eq!(
        planned["no_work_stock_goal"]["schema"],
        "dfmcp.stock-targets-watch/1"
    );
    assert_eq!(
        planned["no_work_stock_goal"]["deadline_tick"],
        original.deadline_tick
    );
    assert!(planned["no_work_stock_goal"]["sample"].is_null());
    let context = runtime.context(false)?;
    let repeated = runtime
        .plan_stock_targets(original.clone(), &context)?
        .value;
    assert_eq!(
        repeated["no_work_stock_goal"],
        planned["no_work_stock_goal"]
    );
    for change in 0..3 {
        let mut changed = original.clone();
        match change {
            0 => changed.deadline_tick += 1,
            1 => changed.targets[0].minimum_stack_units += 1,
            _ => changed.assign_labor = !changed.assign_labor,
        }
        let context = runtime.context(false)?;
        assert!(runtime.plan_stock_targets(changed, &context).is_err());
        assert_eq!(
            runtime
                .stock_targets_no_work
                .as_ref()
                .ok_or("lost original")?
                .request,
            original
        );
    }
    for (index, (phase, count, proven)) in [
        ("", 0, false),
        ("goal_first", 1, false),
        ("goal_first", 1, false),
        ("goal_second", 2, true),
    ]
    .into_iter()
    .enumerate()
    {
        stage(phase)?;
        let context = runtime.context(false)?;
        let sampled = if index % 2 == 0 {
            runtime.wait(&context)?
        } else {
            runtime.observe(ObserveRequest::default(), &context)?
        }
        .value;
        assert_same(&sampled, planned);
        assert_eq!(sampled["original_goal_proven"], proven);
        assert_eq!(sampled["current_state_proven"], false);
        let goal = &sampled["no_work_stock_goal"];
        assert_eq!(goal["goal_witness"], witness);
        assert_eq!(goal["deadline_tick"], original.deadline_tick);
        assert_eq!(goal["original_goal_proven"], proven);
        assert_eq!(goal["current_inventory_proven"], proven);
        assert_eq!(goal["sample"]["stable_samples_observed"], count);
        assert_eq!(goal["sample"]["current_truth"], "true");
        assert_eq!(goal["sample"]["anchor"], sampled["anchor"]);
        let targets = goal["sample"]["targets"]
            .as_array()
            .ok_or("current target census")?;
        assert_eq!(targets.len(), 2);
        for (index, (kind, units)) in [("BED", 6), ("DOOR", 3)].into_iter().enumerate() {
            assert_eq!(targets[index]["item_type"], kind);
            assert_eq!(targets[index]["observed_stack_units"], units);
            assert_eq!(targets[index]["current_truth"], "true");
        }
        for key in [
            "native_effect_dispatched",
            "labor_assignment_planned",
            "worker_prerequisite_checked",
        ] {
            assert_eq!(goal[key], false);
        }
        assert!(sampled["original_plan"].is_null());
        assert!(sampled["frontier"].is_null());
        assert!(sampled["review"].is_null());
    }
    let current = inspect(runtime)?;
    let context = runtime.context(false)?;
    let repeated = runtime
        .plan_stock_targets(original.clone(), &context)?
        .value;
    assert_eq!(
        repeated["no_work_stock_goal"],
        current["no_work_stock_goal"]
    );

    // Exercise the actual response-size refusal and the Runtime withdrawal
    // hook used by live_production_server::render. This component fixture does
    // not substitute for MCP transport or final authority-boundary evidence.
    stage("watch_publication")?;
    let context = runtime.context(false)?;
    let mut unpublished = runtime.wait(&context)?.value;
    assert_eq!(unpublished["original_goal_proven"], true);
    unpublished["fixture_output_overflow"] = json!("x".repeat(70_000));
    let retained = runtime.opening_recovery_reference();
    let (packet, overflow) = crate::live_production_server::presentation::packet(
        "fortress.wait",
        Some(runtime.session_id()),
        Some(runtime.anchor()),
        Some(runtime.mode().as_str()),
        Some(context.request_id),
        Some(context.budget),
        unpublished,
        Some(&retained),
        true,
        false,
    );
    assert!(overflow);
    let refused: Value = serde_json::from_str(&packet)?;
    assert_eq!(refused["original_goal_proven"], false);
    assert_eq!(refused["error"]["code"], "budget_exceeded");
    runtime.abandon_preparation();
    assert_eq!(runtime.authority_tick_floor().get(), start_tick + 15);
    assert_eq!(
        runtime
            .stock_targets_no_work
            .as_ref()
            .ok_or("lost original")?
            .monitor
            .high_tick()
            .get(),
        start_tick + 15
    );
    let withdrawn = inspect(runtime)?;
    assert_same(&withdrawn, planned);
    assert_eq!(withdrawn["original_goal_proven"], false);
    assert!(withdrawn["no_work_stock_goal"]["sample"].is_null());
    assert_eq!(withdrawn["no_work_stock_goal"]["goal_witness"], witness);
    assert_eq!(custody_bytes()?, files);

    stage("watch_consumed")?;
    let context = runtime.context(false)?;
    let consumed = runtime.wait(&context)?.value;
    assert_same(&consumed, planned);
    assert_eq!(consumed["original_goal_proven"], false);
    let sample = &consumed["no_work_stock_goal"]["sample"];
    assert_eq!(sample["current_truth"], "false");
    assert_eq!(sample["targets"][0]["observed_stack_units"], 6);
    assert_eq!(sample["targets"][1]["observed_stack_units"], 0);
    assert_eq!(sample["achieved_at_tick"], start_tick + 10);
    stage("watch_restored")?;
    let context = runtime.context(false)?;
    let restored = runtime.observe(ObserveRequest::default(), &context)?.value;
    assert_same(&restored, planned);
    assert_eq!(restored["original_goal_proven"], true);
    assert_eq!(
        restored["no_work_stock_goal"]["sample"]["achieved_at_tick"],
        start_tick + 10
    );

    // A decoded future operations census cannot be published against an older
    // citizen anchor, and its consumed tick cannot later be rolled back.
    let anchor = runtime.anchor;
    for phase in ["watch_source_ahead", "watch_source_behind"] {
        stage(phase)?;
        let context = runtime.context(false)?;
        assert!(runtime.wait(&context).is_err());
        assert_eq!(runtime.anchor, anchor);
        assert_eq!(runtime.authority_tick_floor().get(), start_tick + 50);
        let retained = inspect(runtime)?;
        assert_same(&retained, planned);
        assert_eq!(retained["original_goal_proven"], false);
        assert!(retained["no_work_stock_goal"]["sample"].is_null());
        assert_eq!(retained["no_work_stock_goal"]["source_fenced"], false);
        assert_eq!(retained["no_work_stock_goal"]["goal_witness"], witness);
        assert_eq!(
            retained["no_work_stock_goal"]["consumed_tick_floor"],
            start_tick + 50
        );
    }
    stage("watch_source_current")?;
    let context = runtime.context(false)?;
    let current = runtime.observe(ObserveRequest::default(), &context)?.value;
    assert_same(&current, planned);
    assert_eq!(current["original_goal_proven"], true);
    assert_eq!(current["anchor"]["tick"], start_tick + 50);
    assert_eq!(
        current["no_work_stock_goal"]["sample"]["achieved_at_tick"],
        start_tick + 10
    );
    assert_eq!(
        current["no_work_stock_goal"]["deadline_tick"],
        original.deadline_tick
    );
    let recovery = runtime.opening_recovery_reference();
    assert_eq!(recovery["original_goal_proven"], false);
    assert_eq!(
        recovery["no_work_stock_goal"]["original_goal_proven"],
        false
    );
    assert!(recovery["no_work_stock_goal"]["sample"].is_null());
    assert_eq!(custody_bytes()?, files);

    let context = runtime.context(false)?;
    let cancelled = runtime.cancel(&context)?.value;
    assert_same(&cancelled, planned);
    assert_eq!(cancelled["no_work_stock_goal"]["cancelled"], true);
    assert_eq!(cancelled["original_goal_proven"], false);
    assert!(cancelled["no_work_stock_goal"]["sample"].is_null());
    let context = runtime.context(false)?;
    let waiting = runtime.wait(&context)?.value;
    assert_eq!(
        waiting["no_work_stock_goal"],
        cancelled["no_work_stock_goal"]
    );

    // Cancellation permits an explicit fresh observation and new request. It
    // cannot lend the new portfolio the earlier owner's temporal achievement.
    stage("watch_new_plan")?;
    let context = runtime.context(false)?;
    let observed = runtime.observe(ObserveRequest::default(), &context)?.value;
    let mut replacement = original;
    replacement.expected_observation = text(&observed["observation"]["seal"])?.into();
    replacement.deadline_tick = runtime.anchor.tick.get() + 300;
    replacement.targets[0].minimum_stack_units = 5;
    let deadline = replacement.deadline_tick;
    let context = runtime.context(false)?;
    let replanned = runtime.plan_stock_targets(replacement, &context)?.value;
    assert_ne!(replanned["no_work_stock_goal"]["goal_witness"], witness);
    assert_eq!(replanned["no_work_stock_goal"]["deadline_tick"], deadline);
    assert_eq!(
        replanned["no_work_stock_goal"]["targets"][0]["target_stack_units"],
        5
    );
    assert_eq!(replanned["no_work_stock_goal"]["cancelled"], false);
    assert_eq!(replanned["original_goal_proven"], false);
    assert!(replanned["no_work_stock_goal"]["sample"].is_null());
    assert!(runtime.original.is_none());
    assert!(runtime.frontier.is_none());
    assert!(runtime.portfolio.is_none());
    assert_eq!(custody_bytes()?, files);
    output(&replanned)
}

fn control(case: &str) -> TestResult {
    if case
        .strip_prefix("assignment_")
        .unwrap_or(case)
        .starts_with("continue")
    {
        return continuation_control(case);
    }
    if let Some(behavior) = case.strip_prefix("assignment_") {
        return assignment_control(case, behavior);
    }
    let mut runtime = open(Mode::Control)?;
    let before_no_work = (case == "already").then(custody_bytes).transpose()?;
    let context = runtime.context(false)?;
    let observed = runtime.observe(ObserveRequest::default(), &context)?.value;
    let request = request(&runtime, &observed, case)?;
    if case == "already" {
        // Plan remains available through the independently retained labor
        // owner; the no-work path needs no production effect authority.
        runtime.opening_orders = false;
    }
    let context = runtime.context(false)?;
    let planned = runtime.plan_stock_targets(request, &context);
    if case == "cap" {
        assert!(planned.is_err());
        assert!(runtime.original.is_none());
        assert!(!runtime.config.path("original.plan").exists());
        return output(&inspect(&mut runtime)?);
    }
    let planned = planned
        .map_err(|cause| format!("stock-targets Plan: {cause}"))?
        .value;
    assert_report(&planned, case)?;
    assert!(planned["review"].is_null());
    assert_eq!(planned["original_goal_proven"], false);
    assert_eq!(planned["current_state_proven"], false);
    if case == "already" {
        assert!(runtime.original.is_none());
        assert!(runtime.portfolio.is_none());
        assert!(planned["original_plan"].is_null());
        assert!(planned["frontier"].is_null());
        assert!(!runtime.config.path("original.plan").exists());
        assert_eq!(
            planned["stock_targets"]["worker_prerequisite_checked"],
            false
        );
        assert_eq!(Some(custody_bytes()?), before_no_work);
        return no_work_watch(&mut runtime, &planned);
    }
    // Even one positive leaf is an original portfolio: all met targets keep
    // their current proof and complete-parent temporal requirements.
    assert!(runtime.portfolio.is_some());
    assert_eq!(
        planned["stock_targets"]["baseline_anchor"],
        observed["anchor"]
    );
    let steps = planned["original_plan"]["steps"]
        .as_array()
        .ok_or("original steps")?;
    assert_eq!(
        steps.len(),
        if case.starts_with("multiple") { 2 } else { 1 }
    );
    assert_eq!(steps[0]["action"]["amount"], 4);
    for (index, step) in steps.iter().enumerate() {
        assert_eq!(step["step_id"], index as u64);
        assert_eq!(step["depends_on"], json!([]));
        assert_eq!(step["action"]["kind"], "create_work_order");
    }
    let original = digest(&planned["original_plan"]["plan_digest"])?;
    if case == "before_stock" {
        stage("before_prepare")?;
        assert!(prepare(&mut runtime, original).is_err());
        let retained = inspect(&mut runtime)?;
        assert_same(&retained, &planned);
        assert_eq!(retained["stock_targets"]["revalidation_failed"], true);
        assert!(retained["review"].is_null());
        return output(&retained);
    }
    let prepared = prepare(&mut runtime, original)?;
    assert_eq!(prepared["review"]["step_id"], 0);
    assert_eq!(prepared["review"]["idempotency_key"], steps[0]["key"]);
    let committed = commit(&mut runtime, &prepared);
    if matches!(case, "after_stock" | "lost") {
        assert!(committed.is_err());
        let retained = inspect(&mut runtime)?;
        assert_same(&retained, &planned);
        assert_eq!(retained["frontier"]["needs_reconciliation"], true);
        assert_eq!(retained["frontier"]["all_applied"], false);
        assert!(retained["review"].is_null());
        assert_eq!(retained["original_goal_proven"], false);
        if case == "after_stock" {
            assert_eq!(retained["stock_targets"]["revalidation_failed"], true);
        }
        assert!(commit(&mut runtime, &prepared).is_err());
        return output(&inspect(&mut runtime)?);
    }
    let committed = committed?;
    assert_same(&committed, &planned);
    if case.starts_with("multiple") {
        assert_eq!(committed["frontier"]["all_applied"], false);
        assert_eq!(committed["frontier"]["entries"][0]["phase"], "applied");
        assert_eq!(committed["frontier"]["next_ready"], 1);
        stage("before_second_prepare")?;
        let second = prepare(&mut runtime, original);
        if case != "multiple" {
            assert!(
                second.is_err(),
                "changed receipt prefix admitted a second order"
            );
            let retained = inspect(&mut runtime)?;
            assert_same(&retained, &planned);
            assert_eq!(retained["frontier"]["entries"][0]["phase"], "applied");
            assert_eq!(retained["stock_targets"]["revalidation_failed"], true);
            return output(&retained);
        }
        let second = second?;
        assert_eq!(second["review"]["step_id"], 1);
        assert_eq!(second["review"]["idempotency_key"], steps[1]["key"]);
        let complete = commit(&mut runtime, &second)?;
        assert_same(&complete, &planned);
        assert_eq!(complete["frontier"]["all_applied"], true);
        assert_eq!(complete["original_goal_proven"], false);
        assert_eq!(
            complete["frontier"]["entries"][0]["receipt_digest"],
            committed["frontier"]["entries"][0]["receipt_digest"]
        );
        return output(&complete);
    }
    assert_eq!(committed["frontier"]["all_applied"], true);
    assert_eq!(committed["original_goal_proven"], false);
    sample_goal(&mut runtime, &planned, case, 0)
}

fn continuation_request(witness: &Value) -> TestResult<StockGoalContinuationRequest> {
    Ok(StockGoalContinuationRequest::parse(
        &json!({
            "kind":"continue_stock_goal", "goal_witness":witness,
        })
        .to_string(),
    )?)
}

fn continuation_control(case: &str) -> TestResult {
    let mut runtime = open(Mode::Control)?;
    let context = runtime.context(false)?;
    let observed = runtime.observe(ObserveRequest::default(), &context)?.value;
    let original = request(&runtime, &observed, case)?;
    let context = runtime.context(false)?;
    let watched = runtime
        .plan_stock_targets(original.clone(), &context)?
        .value;
    let witness = watched["no_work_stock_goal"]["goal_witness"].clone();
    assert_eq!(watched["stock_targets"]["status"], "already_satisfied");
    assert!(runtime.original.is_none());
    let files = custody_bytes()?;
    let start = runtime.anchor.tick.get();
    stage("continue_all_met")?;
    let context = runtime.context(false)?;
    assert!(
        runtime
            .continue_stock_goal(continuation_request(&witness)?, &context)
            .is_err()
    );
    let retained = runtime
        .stock_targets_no_work
        .as_ref()
        .ok_or("retained vector watch")?;
    assert_eq!(retained.request, original);
    assert_eq!(retained.monitor.witness().to_hex(), text(&witness)?);
    assert!(retained.monitor.latest().is_none());
    assert_eq!(runtime.authority_tick_floor().get(), start + 5);
    assert_eq!(custody_bytes()?, files);
    stage("continue_deficit")?;
    let context = runtime.context(false)?;
    let false_sample = runtime.wait(&context)?.value;
    assert_eq!(
        false_sample["no_work_stock_goal"]["sample"]["current_truth"],
        "false"
    );
    assert_eq!(
        false_sample["no_work_stock_goal"]["sample"]["targets"][0]["observed_stack_units"],
        6
    );
    assert_eq!(
        false_sample["no_work_stock_goal"]["sample"]["targets"][1]["observed_stack_units"],
        0
    );
    assert_eq!(
        false_sample["no_work_stock_goal"]["continuation_request"]["goal_witness"],
        witness
    );
    stage("continue_fresh")?;
    let context = runtime.context(false)?;
    let continued = runtime
        .continue_stock_goal(continuation_request(&witness)?, &context)?
        .value;
    assert!(runtime.stock_targets_no_work.is_none());
    assert!(continued["no_work_stock_goal"].is_null());
    assert!(runtime.portfolio.is_some());
    assert_eq!(
        continued["stock_goal_continuation"]["original_watch_witness"],
        witness
    );
    assert_eq!(
        continued["stock_targets"]["targets"][0]["recipe"],
        "wooden_bed"
    );
    assert_eq!(
        continued["stock_targets"]["targets"][0]["baseline_stack_units"],
        2
    );
    assert_eq!(continued["stock_targets"]["targets"][0]["amount"], 4);
    assert_eq!(
        continued["stock_targets"]["targets"][1]["target_stack_units"],
        3
    );
    let plan = runtime
        .original
        .as_ref()
        .ok_or("continued vector original")?;
    assert_eq!(
        prerequisite_binding::requirements(plan)?
            .stock_watch
            .map(Digest32::to_hex),
        Some(text(&witness)?.to_owned())
    );
    assert_eq!(
        runtime
            .layout
            .as_ref()
            .ok_or("continued layout")?
            .inventory_minima()
            .len(),
        2
    );
    let multiple = case.ends_with("multiple");
    let steps = continued["original_plan"]["steps"]
        .as_array()
        .ok_or("continued vector steps")?;
    assert_eq!(
        steps.len(),
        usize::from(original.assign_labor) + if multiple { 2 } else { 1 }
    );
    for step in steps
        .iter()
        .filter(|step| step["action"]["kind"] == "create_work_order")
    {
        assert_eq!(step["obligation"]["deadline_tick"], original.deadline_tick);
        assert_eq!(
            step["obligation"]["poll_interval_ticks"],
            original.poll_interval_ticks
        );
        assert_eq!(
            step["obligation"]["stable_samples"],
            original.stable_samples
        );
    }
    let digest = plan.digest;
    let mut committed = continued.clone();
    for step in steps {
        let prepared = prepare(&mut runtime, digest)?;
        assert_eq!(prepared["review"]["idempotency_key"], step["key"]);
        committed = commit(&mut runtime, &prepared)?;
        assert_same(&committed, &continued);
    }
    assert_eq!(committed["frontier"]["all_applied"], true);
    assert_eq!(committed["original_goal_proven"], false);
    output(&committed)
}

fn continuation_offline() -> TestResult {
    let retained: Value = serde_json::from_str(&std::fs::read_to_string(std::env::var(INPUT)?)?)?;
    let mut runtime = open(Mode::Offline)?;
    let reopened = inspect(&mut runtime)?;
    assert_same(&reopened, &retained);
    assert_eq!(
        reopened["stock_goal_continuation"],
        retained["stock_goal_continuation"]
    );
    assert!(runtime.stock_targets_no_work.is_none());
    assert!(reopened["no_work_stock_goal"].is_null());
    assert_eq!(reopened["original_goal_proven"], false);
    let context = runtime.context(false)?;
    assert!(
        runtime
            .continue_stock_goal(
                continuation_request(
                    &retained["stock_goal_continuation"]["original_watch_witness"]
                )?,
                &context
            )
            .is_err()
    );
    output(&reopened)
}

#[test]
fn original_stock_portfolio_continuation_preserves_all_minima_and_original_timing() -> TestResult {
    if let Ok(phase) = std::env::var(PHASE) {
        let case = std::env::var(CASE)?;
        match phase.as_str() {
            "control" => continuation_control(&case)?,
            "continuation_offline" => continuation_offline()?,
            _ => return Err("unknown stock portfolio continuation phase".into()),
        }
        println!("combined-stock-target-runtime:{phase}:{case}:checked");
        return Ok(());
    }
    let full = format!(
        "{}::original_stock_portfolio_continuation_preserves_all_minima_and_original_timing",
        module_path!()
    );
    let (_, name) = full
        .split_once("::")
        .ok_or("stock portfolio continuation test name")?;
    let driver = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/stdio/combined_stock_targets_runtime_peer.py");
    let result = Command::new("python3")
        .arg("-B")
        .arg(driver)
        .arg(std::env::current_exe()?)
        .args([name, "continuation"])
        .output()?;
    assert!(
        result.status.success(),
        "stock portfolio continuation campaign: {}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        String::from_utf8_lossy(&result.stdout)
            .contains("combined-stock-targets-campaign:continuation:checked")
    );
    Ok(())
}

fn sample_goal(runtime: &mut Runtime, planned: &Value, case: &str, order_step: u32) -> TestResult {
    let stages: &[&str] = if matches!(case, "parent_regression" | "parent_clock") {
        &["goal_first", "goal_second", "goal_third"]
    } else {
        &["goal_first", "goal_second"]
    };
    for (position, name) in stages.iter().enumerate() {
        stage(name)?;
        let context = runtime.context(false)?;
        let sampled = runtime
            .wait(&context)
            .map_err(|cause| format!("stock-targets {name} Wait: {cause}"))?
            .value;
        assert_same(&sampled, &planned);
        assert_eq!(
            sampled["original_goal_proven"],
            position + 1 == stages.len()
        );
        let parent = &sampled["current_parent_goal"];
        let requirements = parent["inventory_requirements"]
            .as_array()
            .ok_or("complete current inventory requirements")?;
        assert_eq!(requirements.len(), 2);
        let door = requirements
            .iter()
            .find(|row| row["item_type"] == "DOOR")
            .ok_or("original met DOOR target")?;
        assert_eq!(door["minimum_stack_units"], 3);
        if matches!(case, "parent_regression" | "parent_clock") {
            assert_eq!(
                door["current_truth"],
                if position == 0 { "false" } else { "true" }
            );
            assert_eq!(
                parent["parent_stability"]["accepted_samples"],
                position as u64
            );
        }
        assert_eq!(sampled["current_state_proven"], false);
        assert_eq!(sampled["stock_targets"]["current_inventory_proven"], false);
        if matches!(case, "parent_regression" | "parent_clock") && position == 1 {
            assert_eq!(
                sampled["original_goal_proven"], false,
                "a met target's loss and regain bypassed all-target stability"
            );
        }
        if case == "parent_clock" && position == 1 {
            assert!(runtime.inventory_clock_control_enabled());
            let context = runtime.context(false)?;
            let selected = runtime.inspect_order(Some(order_step), &context)?.value;
            let request = orders::inventory_clock::RunRequest::parse(
                &json!({
                    "kind":"inventory_run",
                    "original_paired_seal":selected["inventory_clock"]["original_paired_seal"],
                    "game_ticks":5,"wall_millis":1000
                })
                .to_string(),
            )?;
            let context = runtime.context(false)?;
            let reviewed = runtime.plan_inventory_run(request, &context)?.value;
            assert_eq!(reviewed["original_goal_proven"], false);
            let seal = digest(&reviewed["inventory_clock"]["active_run"]["review_seal"])?;
            let context = runtime.context(false)?;
            let advanced = runtime.commit_inventory_run(seal, true, &context)?.value;
            assert_same(&advanced, &planned);
            assert_eq!(advanced["original_goal_proven"], false);
        }
        if position + 1 == stages.len() {
            output(&sampled)?;
        }
    }
    Ok(())
}
fn recover(phase: &str) -> TestResult {
    let case = std::env::var(CASE)?;
    let order_step = usize::from(case.starts_with("assignment_"));
    let retained: Value = serde_json::from_str(&std::fs::read_to_string(std::env::var(INPUT)?)?)?;
    let mode = if phase == "offline" {
        Mode::Offline
    } else {
        Mode::Reconcile
    };
    let mut runtime = open(mode)?;
    let reopened = inspect(&mut runtime)?;
    assert_same(&reopened, &retained);
    assert_report(&reopened, &case)?;
    assert!(runtime.portfolio.is_some());
    assert_eq!(reopened["frontier"]["needs_reconciliation"], true);
    assert!(reopened["review"].is_null());
    assert_eq!(reopened["original_goal_proven"], false);
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
    assert_same(&recovered, &retained);
    assert_eq!(recovered["frontier"]["all_applied"], true);
    assert_eq!(recovered["frontier"]["needs_reconciliation"], false);
    assert_eq!(
        recovered["frontier"]["entries"][order_step]["idempotency_key"],
        retained["original_plan"]["steps"][order_step]["key"]
    );
    output(&recovered)
}

#[test]
fn stock_targets_runtime_cases() -> TestResult {
    if let Ok(phase) = std::env::var(PHASE) {
        let case = std::env::var(CASE)?;
        match phase.as_str() {
            "control" => control(&case)?,
            "offline" | "reconcile" => recover(&phase)?,
            _ => return Err("unknown stock-targets fixture phase".into()),
        }
        println!("combined-stock-target-runtime:{phase}:{case}:checked");
        return Ok(());
    }
    let full = format!("{}::stock_targets_runtime_cases", module_path!());
    let (_, name) = full.split_once("::").ok_or("stock-targets test name")?;
    let driver = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/stdio/combined_stock_targets_runtime_peer.py");
    let result = Command::new("python3")
        .arg("-B")
        .arg(driver)
        .arg(std::env::current_exe()?)
        .args([name, "all"])
        .output()?;
    assert!(
        result.status.success(),
        "stock-targets Runtime campaign: {}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        String::from_utf8_lossy(&result.stdout)
            .contains("combined-stock-targets-campaign:all:checked")
    );
    Ok(())
}

fn assignment_control(case: &str, behavior: &str) -> TestResult {
    let mut runtime = open(Mode::Control)?;
    let before_no_work = (behavior == "already").then(custody_bytes).transpose()?;
    let context = runtime.context(false)?;
    let observed = runtime.observe(ObserveRequest::default(), &context)?.value;
    let request = request(&runtime, &observed, case)?;
    if matches!(behavior, "already" | "denied") {
        // The already-met vector must return before requiring any assignment
        // capability; a positive vector must refuse under the same narrowing.
        runtime.opening_labor = false;
    }
    let mut context = runtime.context(false)?;
    if behavior == "budget" {
        context.budget.max_actions = 1;
    }
    let planned = runtime.plan_stock_targets(request, &context);
    if matches!(behavior, "cap" | "denied" | "budget") {
        assert!(planned.is_err(), "assignment portfolio bypassed {behavior}");
        assert!(runtime.original.is_none());
        assert!(!runtime.config.path("original.plan").exists());
        return output(&inspect(&mut runtime)?);
    }
    let planned = planned
        .map_err(|cause| format!("assigned stock-targets Plan: {cause}"))?
        .value;
    assert_report(&planned, case)?;
    assert!(planned["review"].is_null());
    assert_eq!(planned["original_goal_proven"], false);
    assert_eq!(
        planned["stock_targets"]["worker_prerequisite_checked"],
        false
    );
    if behavior == "already" {
        assert!(runtime.original.is_none());
        assert!(runtime.portfolio.is_none());
        assert!(planned["original_plan"].is_null());
        assert!(planned["frontier"].is_null());
        assert!(!runtime.config.path("original.plan").exists());
        assert_eq!(Some(custody_bytes()?), before_no_work);
        return no_work_watch(&mut runtime, &planned);
    }
    assert!(runtime.portfolio.is_some());
    let steps = planned["original_plan"]["steps"]
        .as_array()
        .ok_or("complete original steps")?;
    let order_count = if behavior == "four" {
        4
    } else if behavior.starts_with("multiple") {
        2
    } else {
        1
    };
    assert_eq!(steps.len(), order_count + 1);
    assert_eq!(steps[0]["step_id"], 0);
    assert_eq!(steps[0]["action"]["kind"], "set_labor");
    assert_eq!(steps[0]["action"]["enabled"], true);
    for (index, step) in steps.iter().enumerate().skip(1) {
        assert_eq!(step["step_id"], index);
        assert_eq!(step["action"]["kind"], "create_work_order");
        assert_eq!(step["depends_on"], json!([0]));
    }
    let original = digest(&planned["original_plan"]["plan_digest"])?;
    let labor_review = prepare(&mut runtime, original)?;
    assert_eq!(labor_review["review"]["step_id"], 0);
    assert_eq!(labor_review["review"]["idempotency_key"], steps[0]["key"]);
    assert!(labor_review["stock_targets"]["last_check_witness"].is_null());
    let labor_committed = commit(&mut runtime, &labor_review)?;
    assert_same(&labor_committed, &planned);
    assert_eq!(
        labor_committed["frontier"]["entries"][0]["phase"],
        "applied"
    );
    assert_eq!(labor_committed["frontier"]["all_applied"], false);
    assert_eq!(labor_committed["frontier"]["next_ready"], 1);
    assert!(labor_committed["stock_targets"]["last_check_witness"].is_null());
    if behavior == "before_stock" {
        stage("before_prepare")?;
        assert!(prepare(&mut runtime, original).is_err());
        let retained = inspect(&mut runtime)?;
        assert_same(&retained, &planned);
        assert_eq!(
            retained["frontier"]["entries"][0]["receipt_digest"],
            labor_committed["frontier"]["entries"][0]["receipt_digest"]
        );
        assert_eq!(retained["stock_targets"]["revalidation_failed"], true);
        return output(&retained);
    }
    let mut previous = labor_committed;
    for (index, step) in steps.iter().enumerate().skip(1) {
        if index > 1 {
            stage("before_second_prepare")?;
        }
        let prepared = prepare(&mut runtime, original);
        if index == 2 && matches!(behavior, "multiple_extra" | "multiple_retired") {
            assert!(
                prepared.is_err(),
                "changed original order prefix admitted the next assignment leaf"
            );
            let retained = inspect(&mut runtime)?;
            assert_same(&retained, &planned);
            assert_eq!(retained["stock_targets"]["revalidation_failed"], true);
            for prior in 0..index {
                assert_eq!(retained["frontier"]["entries"][prior]["phase"], "applied");
                assert_eq!(
                    retained["frontier"]["entries"][prior]["receipt_digest"],
                    previous["frontier"]["entries"][prior]["receipt_digest"]
                );
            }
            return output(&retained);
        }
        let prepared = prepared?;
        assert_eq!(prepared["review"]["step_id"], index);
        assert_eq!(prepared["review"]["idempotency_key"], step["key"]);
        assert_eq!(prepared["stock_targets"]["last_check_order_step_id"], index);
        let committed = commit(&mut runtime, &prepared);
        if matches!(behavior, "after_stock" | "lost") {
            assert!(committed.is_err());
            let retained = inspect(&mut runtime)?;
            assert_same(&retained, &planned);
            assert_eq!(retained["frontier"]["needs_reconciliation"], true);
            assert_eq!(retained["frontier"]["all_applied"], false);
            assert_eq!(retained["frontier"]["entries"][0]["phase"], "applied");
            assert!(retained["review"].is_null());
            if behavior == "after_stock" {
                assert_eq!(retained["stock_targets"]["revalidation_failed"], true);
            }
            assert!(commit(&mut runtime, &prepared).is_err());
            return output(&inspect(&mut runtime)?);
        }
        let committed = committed?;
        assert_same(&committed, &planned);
        for prior in 0..index {
            assert_eq!(committed["frontier"]["entries"][prior]["phase"], "applied");
            assert_eq!(
                committed["frontier"]["entries"][prior]["receipt_digest"],
                previous["frontier"]["entries"][prior]["receipt_digest"]
            );
        }
        assert_eq!(committed["frontier"]["all_applied"], index == order_count);
        assert_eq!(committed["original_goal_proven"], false);
        previous = committed;
    }
    if order_count > 1 {
        return output(&previous);
    }
    sample_goal(&mut runtime, &planned, behavior, 1)
}

#[test]
fn stock_targets_assignment_runtime_cases() -> TestResult {
    if let Ok(phase) = std::env::var(PHASE) {
        let case = std::env::var(CASE)?;
        match phase.as_str() {
            "control" => control(&case)?,
            "offline" | "reconcile" => recover(&phase)?,
            _ => return Err("unknown assigned stock-targets fixture phase".into()),
        }
        println!("combined-stock-target-runtime:{phase}:{case}:checked");
        return Ok(());
    }
    let full = format!("{}::stock_targets_assignment_runtime_cases", module_path!());
    let (_, name) = full
        .split_once("::")
        .ok_or("assigned stock-targets test name")?;
    let driver = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/stdio/combined_stock_targets_runtime_peer.py");
    let result = Command::new("python3")
        .arg("-B")
        .arg(driver)
        .arg(std::env::current_exe()?)
        .args([name, "assignment"])
        .output()?;
    assert!(
        result.status.success(),
        "assigned stock-targets Runtime campaign: {}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        String::from_utf8_lossy(&result.stdout)
            .contains("combined-stock-targets-campaign:assignment:checked")
    );
    Ok(())
}
