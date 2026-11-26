//! Actual native-client/private-custody restart of original no-order watches.
//! The parent journal supplies these references in production and is exercised
//! independently by stock_watch_recovery_tests.
use super::*;

pub(super) fn retain(runtime: &mut Runtime, goal: &ProjectStockGoal, case: &str) -> TestResult {
    let initial = runtime.project_recovery_reference();
    assert!(initial["stock_watch"].is_object());
    let files = custody_bytes()?;
    if case == "watch_fenced" {
        stock_clock_guards::false_and_fenced_source(runtime, goal)?;
    } else {
        stage("first_sample")?;
        let c = runtime.context(false)?;
        let view = runtime.wait(&c)?.value;
        assert_eq!(
            view["no_work_stock_goal"]["sample"]["stable_samples_observed"],
            1
        );
        assert!(!runtime.project_status(goal)?.original_goal_proven);
        if case == "watch_achieved" {
            stage("second_sample")?;
            let c = runtime.context(false)?;
            runtime.wait(&c)?;
            assert!(runtime.project_status(goal)?.original_goal_proven);
        } else if case == "watch_cancelled" {
            let c = runtime.context(false)?;
            runtime.cancel(&c)?;
            assert_eq!(runtime.no_work_goal_json()["cancelled"], true);
        }
    }
    let current = runtime.project_recovery_reference();
    crate::live_production_runtime::project_stock_watch::verify_reference_continuity(
        &initial, &current,
    )?;
    assert_eq!(
        initial["stock_watch"]["request"],
        current["stock_watch"]["request"]
    );
    assert_eq!(
        files,
        custody_bytes()?,
        "watch sampling wrote native effect custody"
    );
    Ok(())
}

pub(super) fn recover(phase: &str, requested_case: &str) -> TestResult {
    let (_, _, case) = publication_case(requested_case);
    let retained: Value = serde_json::from_slice(&std::fs::read(std::env::var(INPUT)?)?)?;
    let goal: ProjectStockGoal = serde_json::from_value(retained["goal"].clone())?;
    let prior = &retained["actual"];
    let origin = workforce::digest(
        prior["workshop_prerequisite"]["checkpoint"]["origin_witness"]
            .as_str()
            .ok_or("missing original Workshop donor")?,
    )?;
    let mode = if phase == "watch-resume" {
        Mode::Control
    } else {
        Mode::Offline
    };
    let mut config = Config::from_environment(|| Ok(()))?;
    assert!(!config.initialize && config.recovery.is_none());
    bind_project(&mut config, &goal, origin);
    let files = custody_bytes()?;
    let opened = config.validate_project_recovery(prior).and_then(|()| {
        Runtime::open_for_resource_project(
            config,
            mode,
            SessionId::new(30_004),
            budget(),
            prior["observation_tick_floor"]
                .as_u64()
                .ok_or_else(|| invalid("missing watch floor"))?,
        )
    });
    if phase == "watch-reject" {
        assert!(opened.is_err(), "invalid retained watch custody opened");
        assert_eq!(files, custody_bytes()?);
        return Ok(());
    }
    let mut runtime = opened?;
    assert_eq!(
        files,
        custody_bytes()?,
        "watch reopen changed private custody"
    );
    let mut expected_watch = prior["stock_watch"].clone();
    if case == "watch_parent_stopped" {
        // The parent journal regression separately retains a terminal ACK
        // while this child checkpoint is still uncancelled (the crash window).
        // Apply its local retirement in both Offline and Control owners.
        assert_eq!(runtime.no_work_goal_json()["cancelled"], false);
        let mut wrong = serde_json::to_value(&goal)?;
        if goal.is_portfolio() {
            wrong["targets"][0]["minimum_stack_units"] = json!(7);
        } else {
            wrong["minimum_stack_units"] = json!(7);
        }
        let wrong: ProjectStockGoal = serde_json::from_value(wrong)?;
        assert!(
            runtime
                .retire_project_stock_watch_after_stop(&wrong)
                .is_err()
        );
        assert_eq!(runtime.no_work_goal_json()["cancelled"], false);
        runtime.retire_project_stock_watch_after_stop(&goal)?;
        let stopped = runtime.project_recovery_reference();
        runtime.retire_project_stock_watch_after_stop(&goal)?;
        assert_eq!(runtime.project_recovery_reference(), stopped);
        expected_watch["cancelled"] = json!(true);
    }
    let reopened = runtime.project_recovery_reference();
    assert_eq!(reopened["stock_watch"], expected_watch);
    assert_eq!(reopened["native_custody"], prior["native_custody"]);
    assert!(runtime.original.is_none());
    assert!(!runtime.project_status(&goal)?.original_goal_proven);
    let c = runtime.context(false)?;
    let inspected = runtime.inspect(&c)?.value;
    assert!(inspected["review"].is_null());
    assert!(inspected["no_work_stock_goal"]["sample"].is_null());
    assert_eq!(
        inspected["no_work_stock_goal"]["original_goal_proven"],
        false
    );
    let witness = inspected["no_work_stock_goal"]["goal_witness"].clone();
    assert_eq!(
        inspected["no_work_stock_goal"]["deadline_tick"],
        goal.deadline_tick()
    );
    if phase == "watch-recover" {
        if case == "watch_parent_stopped" {
            let c = runtime.context(false)?;
            runtime.wait(&c)?;
            let c = runtime.context(false)?;
            let view = runtime.observe(ObserveRequest::default(), &c)?.value;
            assert_eq!(view["no_work_stock_goal"]["cancelled"], true);
            assert!(view["no_work_stock_goal"]["sample"].is_null());
            assert!(view["no_work_stock_goal"]["continuation_request"].is_null());
            assert_eq!(view["original_goal_proven"], false);
            assert!(runtime.project_stock_clock_review_witness(&goal).is_err());
        }
        assert_eq!(files, custody_bytes()?);
        return output(&reopened);
    }

    match case {
        "watch_restart" => {
            for (stage_name, count, proven) in
                [("second_sample", 1, false), ("third_sample", 2, true)]
            {
                stage(stage_name)?;
                let c = runtime.context(false)?;
                let view = runtime.wait(&c)?.value;
                assert_eq!(
                    view["no_work_stock_goal"]["sample"]["stable_samples_observed"],
                    count
                );
                assert_eq!(view["no_work_stock_goal"]["goal_witness"], witness);
                assert_eq!(runtime.project_status(&goal)?.original_goal_proven, proven);
            }
            assert!(runtime.project_stock_clock_review_witness(&goal).is_err());
        }
        "watch_achieved" => {
            let achievement =
                inspected["no_work_stock_goal"]["historical_achieved_at_tick"].clone();
            assert!(achievement.is_u64());
            stage("continue_deficit")?;
            let c = runtime.context(false)?;
            let view = runtime.wait(&c)?.value;
            assert_eq!(
                view["no_work_stock_goal"]["sample"]["current_truth"],
                "false"
            );
            assert_eq!(
                view["no_work_stock_goal"]["historical_achieved_at_tick"],
                achievement
            );
            assert!(!runtime.project_status(&goal)?.original_goal_proven);
        }
        "watch_cancelled" | "watch_parent_stopped" => {
            assert_eq!(inspected["no_work_stock_goal"]["cancelled"], true);
            let c = runtime.context(false)?;
            runtime.wait(&c)?;
            let c = runtime.context(false)?;
            runtime.observe(ObserveRequest::default(), &c)?;
            let request = prior["stock_watch"]["request"].to_string();
            let c = runtime.context(false)?;
            if goal.is_portfolio() {
                assert!(
                    runtime
                        .project_plan_stock_targets_recorded(
                            &goal,
                            StockTargetsRequest::parse(&request)?,
                            &c,
                            &mut |_, _| Err(denied("cancelled watch must not publish")),
                        )
                        .is_err()
                );
            } else {
                assert!(
                    runtime
                        .project_plan_stock_target(&goal, StockTargetRequest::parse(&request)?, &c,)
                        .is_err()
                );
            }
        }
        "watch_fenced" => {
            assert_eq!(inspected["no_work_stock_goal"]["source_fenced"], true);
            let c = runtime.context(false)?;
            assert!(runtime.wait(&c).is_err());
        }
        "watch_continue" => {
            assert_eq!(
                json!(runtime.project_stock_clock_review_witness(&goal)?.to_hex()),
                witness
            );
            stage("continue_deficit")?;
            let mut intent = None;
            let prior = runtime.project_recovery_reference();
            let original_watch = prior["stock_watch"].clone();
            let directory = runtime.config.directory.clone();
            let c = runtime.context(false)?;
            let view = runtime
                .project_continue_stock_goal_recorded(
                    &goal,
                    StockGoalContinuationRequest::parse(
                        &json!({
                            "kind":"continue_stock_goal", "goal_witness":witness,
                        })
                        .to_string(),
                    )?,
                    &c,
                    &mut |publication, _| {
                        for name in ORIGINAL_FILES {
                            assert!(
                                !directory.join(name).exists(),
                                "new original preceded its parent ACK"
                            );
                        }
                        publication.validate_checkpoint(&prior)?;
                        intent = Some(publication.to_json());
                        Ok(())
                    },
                )?
                .value;
            assert!(view["original_plan"].is_object());
            assert!(view["review"].is_null());
            assert!(runtime.no_work_goal_json().is_null());
            assert_eq!(
                runtime.stock_continuation_json()["original_watch_witness"],
                witness
            );
            assert_eq!(
                runtime.project_recovery_reference()["stock_watch"]["request"],
                original_watch["request"]
            );
            let positive = runtime.project_recovery_reference();
            runtime.retire_project_stock_watch_after_stop(&goal)?;
            assert_eq!(
                runtime.project_recovery_reference(),
                positive,
                "root stop rewrote positive original or predecessor"
            );
            return output(&json!({
                "intent":intent, "prior":prior, "goal":goal,
                "actual":runtime.project_recovery_reference(),
                "workforce_journal_id":runtime.workforce_journal.to_hex(),
            }));
        }
        _ => return Err("unknown stock-watch recovery fixture".into()),
    }
    let c = runtime.context(false)?;
    assert!(
        runtime
            .project_continue_stock_goal_recorded(
                &goal,
                StockGoalContinuationRequest::parse(
                    &json!({
                        "kind":"continue_stock_goal", "goal_witness":witness,
                    })
                    .to_string()
                )?,
                &c,
                &mut |_, _| Err(denied("terminal watch must not publish")),
            )
            .is_err()
    );
    assert!(runtime.original.is_none());
    assert_eq!(
        files,
        custody_bytes()?,
        "resumed watch wrote native effect custody"
    );
    output(&runtime.project_recovery_reference())
}
