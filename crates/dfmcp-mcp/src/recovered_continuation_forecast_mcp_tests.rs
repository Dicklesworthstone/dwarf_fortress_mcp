//! Full custody admission for recovered and mixed committed project chains.

use super::*;

fn continue_beds(owner: &Owner, root: &str) -> TestResult<String> {
    commit(
        owner,
        None,
        Some(json!({"template":"continue_goal","plan_digest":root})),
    )
}

fn settle_output(owner: &Owner, digest: &str) -> TestResult {
    let result = parsed(&facade::fortress_wait_for_plan(
        Some(owner.0.clone()),
        Some(2_000),
        Some(digest.to_owned()),
    ))?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["selected_plan_wait"]["actions_verified"], true);
    assert_eq!(result["selected_plan_wait"]["physical_quiescent"], true);
    assert_eq!(result["selected_plan_wait"]["goal_truth"], "true");
    Ok(())
}

#[test]
fn recovered_continuation_chain_survives_reopen_compaction_and_queries_without_old_handles()
-> TestResult {
    let _serial = crate::test_serial();
    let _durable = server::serialized_durable_tests();
    let directory = StateDir::new("continued-complete");
    let first = open_durable("8161505")?;
    let root = original(&first)?;
    settle(&first, &root)?;
    let child = continue_beds(&first, &root)?;
    settle_output(&first, &child)?;
    directory.restart();
    drop(first);
    for _ in 0..2 {
        let owner = open_durable("8161505")?;
        plan(&owner, Some(order("retain pending review across chain forecast", 1)), None)?;
        let before = state(&owner)?;
        let result = forecast(&owner, &root, 0)?;
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["schema"], "dfmcp.lab-original-forecast/3");
        assert_eq!(result["prediction"]["all_complete"], true, "{result}");
        assert_eq!(result["prediction"]["forecast_work"]["action_polls"], 0);
        assert_eq!(result["prediction"]["forecast_work"]["hypothetical_stages"], 0);
        let retained = &result["prediction"]["originals"][0]["retained_stages"][0];
        assert_eq!(retained["actual_plan_digest"], child);
        assert_eq!(retained["custody"], "recovered_verified_original");
        assert_eq!(retained["selection_origin"], "durable_recovery");
        assert_eq!(retained["dispatch_handles_restored"], false);
        assert_eq!(retained["recommitted_by_forecast"], false);
        assert!(retained["steps"].as_array().is_some_and(|steps| {
            !steps.is_empty() && steps.iter().all(|step| step["action_id"].is_null())
        }));
        assert_eq!(result["next_step"]["arguments"]["plan_digests"], json!([child]));
        assert!(result["source_work"]["recovered_original_source_witnesses"]
            .as_array().is_some_and(|sources| sources.len() == 2
                && sources.iter().all(|source| source["query_archive_reads"] == 0)));
        assert_eq!(state(&owner)?, before);

        let budget = view(&owner, |session| Ok(session.budget))?;
        let steps = result["source_work"]["verified_recovered_original_steps"]
            .as_u64().ok_or("recovered chain step count missing")?;
        edit(&owner, |session| {
            session.budget.max_actions = u32::try_from(steps - 1)?;
            Ok(())
        })?;
        let before = state(&owner)?;
        assert_eq!(forecast(&owner, &root, 0)?["ok"], false);
        assert_eq!(state(&owner)?, before);
        edit(&owner, |session| {
            session.budget = budget;
            session.commit_receipts.insert(child.clone(), "{}".to_owned());
            Ok(())
        })?;
        let before = state(&owner)?;
        assert_eq!(forecast(&owner, &root, 0)?["ok"], false);
        assert_eq!(state(&owner)?, before);
        edit(&owner, |session| {
            session.commit_receipts.remove(&child);
            Ok(())
        })?;
        assert_eq!(forecast(&owner, &root, 0)?["ok"], true);
        directory.restart();
        drop(owner);
        dfmcp_lab::durable::DurableLabStore::open(&directory.0)?.compact()?;
    }
    Ok(())
}

#[test]
fn recovered_root_forecasts_live_and_recovered_tips_without_restoring_dispatch() -> TestResult {
    let _serial = crate::test_serial();
    let _durable = server::serialized_durable_tests();
    let directory = StateDir::new("continued-mixed");
    let first = open_durable("8161506")?;
    let root = original(&first)?;
    settle(&first, &root)?;
    directory.restart();
    drop(first);
    let owner = open_durable("8161506")?;
    let child = continue_beds(&owner, &root)?;
    let before = state(&owner)?;
    for horizon in [0, 4_000] {
        let result = forecast(&owner, &root, horizon)?;
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["prediction"]["all_complete"], horizon > 0, "{result}");
        assert_eq!(result["prediction"]["forecast_work"]["hypothetical_stages"], 0);
        let retained = &result["prediction"]["originals"][0]["retained_stages"][0];
        assert_eq!(retained["actual_plan_digest"], child);
        assert_eq!(retained["custody"], "already_committed_original");
        assert_eq!(retained["recommitted_by_forecast"], false);
        assert!(retained["steps"].as_array().is_some_and(|steps| {
            !steps.is_empty() && steps.iter().all(|step| step["action_id"].is_string())
        }));
        assert_eq!(state(&owner)?, before);
    }
    directory.restart();
    drop(owner);
    let owner = open_durable("8161506")?;
    let before = state(&owner)?;
    let mut predicted_completion = None;
    for horizon in [0, 4_000] {
        let result = forecast(&owner, &root, horizon)?;
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["schema"], "dfmcp.lab-original-forecast/4");
        assert_eq!(result["prediction"]["all_complete"], horizon > 0, "{result}");
        assert_eq!(result["prediction"]["forecast_work"]["hypothetical_stages"], 0);
        let retained = &result["prediction"]["originals"][0]["retained_stages"][0];
        assert_eq!(retained["actual_plan_digest"], child);
        assert_eq!(retained["custody"], "recovered_observation_original");
        assert_eq!(retained["dispatch_handles_restored"], false);
        assert_eq!(retained["recommitted_by_forecast"], false);
        assert!(retained["steps"].as_array().is_some_and(|steps| {
            !steps.is_empty() && steps.iter().all(|step| {
                step["action_id"].is_null()
                    && step["dispatch_handle_restored"] == false
            })
        }));
        if horizon > 0 {
            predicted_completion = result["prediction"]["predicted_frontier_tick"].as_u64();
        }
        assert_eq!(state(&owner)?, before);
    }
    settle_output(&owner, &child)?;
    assert_eq!(
        Some(view(&owner, |session| Ok(session.adapter.snapshot().tick.0))?),
        predicted_completion,
    );
    let before = state(&owner)?;
    let result = forecast(&owner, &root, 0)?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["prediction"]["all_complete"], true, "{result}");
    assert_eq!(result["prediction"]["originals"][0]["retained_stages"][0]["custody"],
        "recovered_verified_original");
    assert_eq!(state(&owner)?, before);
    Ok(())
}
