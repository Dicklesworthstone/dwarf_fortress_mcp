//! Typed clock eligibility on the existing real production/native peer campaign.
//! The clock itself is exercised separately. These calls qualify stock through
//! the actual child and retain no order, review, native effect, or current proof.
use super::*;
use dfmcp_adapter::semantic_work_orders::OperationsGoalSource;

fn sample_count(runtime: &Runtime) -> TestResult<u64> {
    runtime.no_work_goal_json()["sample"]["stable_samples_observed"]
        .as_u64()
        .ok_or_else(|| "missing qualified stock-watch sample count".into())
}

pub(super) fn initial_sample(runtime: &mut Runtime, goal: &ProjectStockGoal) -> TestResult {
    let files = custody_bytes()?;
    let anchor = runtime.anchor;
    assert!(runtime.original.is_none());
    assert!(runtime.project_recovery_reference()["recovery"].is_null());
    assert!(runtime.project_stock_clock_basis(goal).is_err());
    let retained_witness = runtime.project_stock_clock_review_witness(goal)?;
    let c = runtime.context(false)?;
    runtime.wait(&c)?;
    let first = runtime.project_stock_clock_basis(goal)?;
    assert_eq!(first.anchor.tick, anchor.tick);
    assert_eq!(
        first.next_sample_tick,
        anchor.tick.get() + goal.poll_interval_ticks()
    );
    assert_eq!(sample_count(runtime)?, 0);
    assert_eq!(first.goal_witness, retained_witness);
    assert!(runtime.no_work_goal_json()["sample"]["last_counted_tick"].is_null());
    assert!(!runtime.project_status(goal)?.original_goal_proven);

    // The initial complete True sample justifies the first bounded interval;
    // another census at the same tick cannot spend that interval or add credit.
    let c = runtime.context(false)?;
    runtime.wait(&c)?;
    let repeated = runtime.project_stock_clock_basis(goal)?;
    assert_eq!(repeated.anchor.tick, first.anchor.tick);
    assert_eq!(repeated.goal_witness, first.goal_witness);
    assert_eq!(repeated.next_sample_tick, first.next_sample_tick);
    assert_eq!(sample_count(runtime)?, 0);

    // These are actual eligibility branches on an authentic qualified watch.
    // Restoring the fixture flags cannot restore a source or grant in production.
    runtime.fenced = true;
    assert!(runtime.project_stock_clock_basis(goal).is_err());
    runtime.fenced = false;
    runtime.initialization_failed = true;
    assert!(runtime.project_stock_clock_basis(goal).is_err());
    runtime.initialization_failed = false;
    let current_anchor = runtime.anchor;
    runtime.anchor.tick = GameTick(goal.deadline_tick());
    assert!(runtime.project_stock_clock_basis(goal).is_err());
    runtime.anchor = current_anchor;
    let prior_floor = runtime.observation_tick_floor;
    runtime.observation_tick_floor = GameTick(current_anchor.tick.get() + 1);
    assert!(runtime.project_stock_clock_basis(goal).is_err());
    runtime.observation_tick_floor = prior_floor;
    if let Some(watch) = &mut runtime.stock_target_no_work {
        watch.cancelled = true;
    } else if let Some(watch) = &mut runtime.stock_targets_no_work {
        watch.cancelled = true;
    }
    assert!(runtime.project_stock_clock_basis(goal).is_err());
    if let Some(watch) = &mut runtime.stock_target_no_work {
        watch.cancelled = false;
    } else if let Some(watch) = &mut runtime.stock_targets_no_work {
        watch.cancelled = false;
    }
    assert!(runtime.project_stock_clock_basis(goal).is_ok());

    // Successful preparation withholds its current sample. The same live watch
    // can still name an exact re-review, whose Plan must requalify stock.
    runtime.retain_project_stock_clock_tick(runtime.anchor.tick.get())?;
    assert!(runtime.no_work_goal_json()["sample"].is_null());
    assert_eq!(
        runtime.project_stock_clock_review_witness(goal)?,
        retained_witness
    );
    assert!(runtime.project_stock_clock_basis(goal).is_err());
    let c = runtime.context(false)?;
    runtime.wait(&c)?;

    let c = runtime.context(false)?;
    if runtime.opening_inventory_clock && runtime.config.orders.inventory_clock_enabled() {
        let refreshed = runtime.project_sample_stock_clock(goal, &c)?;
        assert_eq!(refreshed.anchor.tick, anchor.tick);
        assert_eq!(refreshed.goal_witness, first.goal_witness);
        assert_eq!(sample_count(runtime)?, 0);
    } else {
        assert!(runtime.project_sample_stock_clock(goal, &c).is_err());
    }
    assert!(runtime.original.is_none());
    assert!(runtime.project_recovery_reference()["recovery"].is_null());
    assert!(!runtime.project_status(goal)?.original_goal_proven);
    assert_eq!(
        runtime.no_work_goal_json()["native_effect_dispatched"],
        false
    );
    assert_eq!(
        files,
        custody_bytes()?,
        "stock qualification wrote native custody"
    );
    Ok(())
}

pub(super) fn before_interval(runtime: &mut Runtime, goal: &ProjectStockGoal) -> TestResult {
    let files = custody_bytes()?;
    let basis = runtime.project_stock_clock_basis(goal)?;
    let anchor = runtime.anchor;
    let count = sample_count(runtime)?;
    assert!(count < u64::from(goal.stable_samples()));
    runtime.retain_project_stock_clock_tick(basis.next_sample_tick)?;
    assert_eq!(
        runtime.anchor, anchor,
        "a clock floor became an observation"
    );
    assert_eq!(runtime.authority_tick_floor().get(), basis.next_sample_tick);
    let withheld = runtime.no_work_goal_json();
    assert!(withheld["sample"].is_null());
    assert_eq!(withheld["original_goal_proven"], false);
    assert!(runtime.project_stock_clock_basis(goal).is_err());
    assert_eq!(
        files,
        custody_bytes()?,
        "clock floor published a child effect"
    );
    Ok(())
}

pub(super) fn after_interval(
    runtime: &mut Runtime,
    goal: &ProjectStockGoal,
    complete: bool,
) -> TestResult {
    assert_eq!(sample_count(runtime)?, if complete { 2 } else { 1 });
    assert_eq!(runtime.project_status(goal)?.original_goal_proven, complete);
    if complete {
        assert!(runtime.project_stock_clock_basis(goal).is_err());
        assert!(runtime.project_stock_clock_review_witness(goal).is_err());
    } else {
        let first = runtime.project_stock_clock_basis(goal)?;
        let c = runtime.context(false)?;
        runtime.wait(&c)?;
        let repeated = runtime.project_stock_clock_basis(goal)?;
        assert_eq!(first.goal_witness, repeated.goal_witness);
        assert_eq!(first.anchor.tick, repeated.anchor.tick);
        assert_eq!(first.next_sample_tick, repeated.next_sample_tick);
        assert_eq!(
            sample_count(runtime)?,
            1,
            "same-tick refresh advanced stability"
        );
    }
    assert!(runtime.original.is_none());
    assert!(runtime.project_recovery_reference()["recovery"].is_null());
    Ok(())
}

pub(super) fn false_and_fenced_source(
    runtime: &mut Runtime,
    goal: &ProjectStockGoal,
) -> TestResult {
    let files = custody_bytes()?;
    stage("continue_deficit")?;
    let c = runtime.context(false)?;
    runtime.wait(&c)?;
    assert_eq!(
        runtime.no_work_goal_json()["sample"]["current_truth"],
        "false"
    );
    assert!(!runtime.project_status(goal)?.original_goal_proven);
    assert!(runtime.project_stock_clock_basis(goal).is_err());
    assert!(runtime.project_stock_clock_review_witness(goal).is_err());

    // Feed the authentic captured operations through the real monitor with one
    // changed source incarnation. It must fence instead of becoming clock proof.
    let c = runtime.context(false)?;
    let mut work = Work::new(&c)?;
    let mut source = runtime.config.orders.operations(&work.take(16 * MIB)?)?;
    let mut capture = source.read_operations(&work.take(16 * MIB)?)?;
    drop(source);
    capture.jobs.bridge_generation = capture
        .jobs
        .bridge_generation
        .checked_add(1)
        .ok_or("fixture source generation exhausted")?;
    if let Some(watch) = &mut runtime.stock_target_no_work {
        assert!(watch.monitor.note_capture(&capture).is_err());
        assert!(watch.monitor.fenced());
    } else if let Some(watch) = &mut runtime.stock_targets_no_work {
        assert!(watch.monitor.note_capture(&capture).is_err());
        assert!(watch.monitor.fenced());
    } else {
        return Err("the original no-work watch disappeared".into());
    }
    assert_eq!(runtime.no_work_goal_json()["source_fenced"], true);
    assert!(runtime.no_work_goal_json()["sample"].is_null());
    assert!(runtime.project_stock_clock_basis(goal).is_err());
    assert!(runtime.project_stock_clock_review_witness(goal).is_err());
    assert!(!runtime.project_status(goal)?.original_goal_proven);
    assert!(runtime.original.is_none());
    assert!(runtime.project_recovery_reference()["recovery"].is_null());
    assert_eq!(
        files,
        custody_bytes()?,
        "refusing a stock clock changed custody"
    );
    // The same peer is reused by existing restart cases; do not leave a
    // test-only stock override active for their subsequent planning phase.
    stage("")?;
    Ok(())
}

pub(super) fn published_original(runtime: &Runtime, goal: &ProjectStockGoal) -> TestResult {
    assert!(runtime.original.is_some());
    assert!(!runtime.project_has_stock_watch(goal)?);
    assert!(runtime.project_stock_clock_basis(goal).is_err());
    assert!(runtime.project_stock_clock_review_witness(goal).is_err());
    Ok(())
}
