//! Public current-stage forecasts survive ordinary durable recovery.
//! Raw excavation uses a source with terrain and no production ledger.

use super::*;

fn seed_bare_terrain(owner: &Owner) -> TestResult {
    edit(owner, |session| {
        let mut snapshot = session.adapter.snapshot().clone();
        assert!(snapshot.graph.entities.is_empty());
        let coord = dfmcp_world::ChunkCoord { x: 0, y: 0, z: 10 };
        snapshot.graph.chunks.insert(coord, dfmcp_world::terrain::uniform_chunk(
            coord, dfmcp_world::tile_codes::SOLID_WALL,
        ));
        snapshot.cursor = snapshot.cursor.checked_next().ok_or("fixture cursor overflow")?;
        snapshot.refresh_hash();
        assert!(crate::lab_world::ProductionGoalLedger::capture(&snapshot).is_err());
        session.adapter = dfmcp_lab::MemoryAdapter::new(snapshot);
        Ok(())
    })
}

fn raw_mine(owner: &Owner) -> TestResult<String> {
    commit(owner, Some(json!([{
        "action":{"kind":"designate_dig","min":[1,1,10],"max":[4,2,10],"mode":"mine"}
    }])), None)
}

fn files(path: &std::path::Path) -> TestResult<Vec<(std::path::PathBuf, Vec<u8>)>> {
    let mut rows = Vec::new();
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            rows.extend(files(&entry.path())?);
        } else {
            rows.push((entry.path(), std::fs::read(entry.path())?));
        }
    }
    rows.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(rows)
}

#[test]
fn raw_mine_without_a_stock_ledger_forecasts_after_reopen_and_matches_actual_wait()
-> TestResult {
    let _serial = crate::test_serial();
    let _durable = server::serialized_durable_tests();
    let directory = StateDir::new("current-stage-bare-mine");
    let first = open_durable_scenario("8161511", "empty")?;
    seed_bare_terrain(&first)?;
    let digest = raw_mine(&first)?;
    directory.restart();
    drop(first);

    let owner = open_durable_scenario("8161511", "empty")?;
    view(&owner, |session| {
        assert!(crate::lab_world::ProductionGoalLedger::capture(session.adapter.snapshot()).is_err());
        assert!(session.commit_receipts.is_empty());
        assert!(session.open_actions.is_empty());
        assert!(session.carried.iter().any(|row| row.state == "dispatched" && row.monitor.is_some()));
        Ok(())
    })?;
    let pending = plan(&owner, Some(json!([{"action":{"kind":"pause","paused":true}}])), None)?;
    let before = state(&owner)?;
    let disk = files(&directory.0)?;
    let zero = query(&owner, std::slice::from_ref(&digest), 0)?;
    assert_eq!(zero["ok"], true, "{zero}");
    assert_eq!(zero["schema"], "dfmcp.lab-original-forecast/5");
    assert_eq!(zero["forecast_scope"], "current_stage");
    assert_eq!(zero["prediction"]["forecast_work"]["recovered_observation_polls"], 0);
    assert_eq!(zero["source_work"]["recovered_original_source_witnesses"][0]
        ["production_ledger_required"], false);
    let predicted = query(&owner, std::slice::from_ref(&digest), 4_000)?;
    assert_eq!(predicted["ok"], true, "{predicted}");
    assert_eq!(predicted["prediction"]["all_complete"], true, "{predicted}");
    assert_eq!(predicted["prediction"]["forecast_work"]["deferred_dispatches"], 0);
    assert_eq!(predicted["prediction"]["forecast_work"]["action_polls"], 0);
    assert_eq!(predicted["prediction"]["forecast_work"]["actual_game_ticks_advanced"], 0);
    let row = &predicted["prediction"]["originals"][0];
    assert_eq!(row["actual_plan_digest"], digest);
    assert_eq!(row["selection_origin"], "durable_recovery");
    assert_eq!(row["dispatch_handles_restored"], false);
    assert!(row["future_stages"].is_null());
    assert!(row["steps"].as_array().is_some_and(|steps| !steps.is_empty()
        && steps.iter().all(|step| step["action_id"].is_null()
            && step["predicted_proof_is_canonical"] == false)));
    assert_eq!(predicted["agent_turn"]["coverage"]["eligible_as_mutation_evidence"], false);
    assert_eq!(state(&owner)?, before);
    assert_eq!(before["pending"], pending);
    assert_eq!(files(&directory.0)?, disk);
    let waited = parsed(&facade::fortress_wait_for_plan(
        Some(owner.0.clone()), Some(4_000), Some(digest),
    ))?;
    assert_eq!(waited["ok"], true, "{waited}");
    assert_eq!(waited["selected_plan_wait"]["stop_reason"], "complete", "{waited}");
    assert_eq!(waited["game_tick"], predicted["prediction"]["predicted_frontier_tick"]);
    Ok(())
}

#[test]
fn mixed_live_and_recovered_raw_orders_share_the_selected_action_allowance_and_real_frontier()
-> TestResult {
    let _serial = crate::test_serial();
    let _durable = server::serialized_durable_tests();
    let directory = StateDir::new("current-stage-mixed-orders");
    let first = open_durable("8161512")?;
    let recovered = commit(&first, Some(order("recovered raw beds", 4)), None)?;
    directory.restart();
    drop(first);
    let owner = open_durable("8161512")?;
    let live = commit(&owner, Some(json!([{"action":{
        "kind":"create_work_order","name":"live raw chairs","job_token":"MAKE_CHAIR","amount":4
    }}])), None)?;
    let selected = [recovered.clone(), live.clone()];
    let steps = view(&owner, |session| Ok(session.objectives.iter()
        .filter(|goal| selected.contains(&goal.plan_digest.to_hex()))
        .filter_map(|goal| goal.resource_plan()).map(|plan| plan.steps.len()).sum::<usize>()))?;
    edit(&owner, |session| {
        session.budget.max_actions = u32::try_from(steps)?;
        Ok(())
    })?;
    let before = state(&owner)?;
    let predicted = query(&owner, &selected, 4_000)?;
    assert_eq!(predicted["ok"], true, "{predicted}");
    assert_eq!(predicted["schema"], "dfmcp.lab-original-forecast/5");
    assert_eq!(predicted["prediction"]["all_complete"], true, "{predicted}");
    assert_eq!(predicted["prediction"]["forecast_work"]["competing_recovered_steps"], 0);
    let rows = predicted["prediction"]["originals"].as_array().ok_or("selected originals missing")?;
    let old = rows.iter().find(|row| row["actual_plan_digest"] == recovered).ok_or("recovered row missing")?;
    let new = rows.iter().find(|row| row["actual_plan_digest"] == live).ok_or("live row missing")?;
    assert_eq!(old["selection_origin"], "durable_recovery");
    assert!(old["steps"].as_array().is_some_and(|steps| steps.iter().all(|step| step["action_id"].is_null())));
    assert!(new["steps"].as_array().is_some_and(|steps| steps.iter().all(|step| step["action_id"].is_string())));
    assert_eq!(state(&owner)?, before);
    let waited = parsed(&facade::fortress_wait_for_plans(
        Some(owner.0.clone()), Some(4_000), selected.to_vec(), None,
    ))?;
    assert_eq!(waited["ok"], true, "{waited}");
    assert_eq!(waited["game_tick"], predicted["prediction"]["predicted_frontier_tick"]);
    Ok(())
}

#[test]
fn historical_raw_originals_charge_archived_sources_and_require_current_authority()
-> TestResult {
    let _serial = crate::test_serial();
    let _durable = server::serialized_durable_tests();
    let directory = StateDir::new("current-stage-history");
    let first = open_durable_scenario("8161513", "empty")?;
    let digest = commit(&first, Some(json!([{"action":{"kind":"pause","paused":true}}])), None)?;
    directory.restart();
    drop(first);
    let owner = open_durable_scenario("8161513", "empty")?;
    view(&owner, |session| {
        assert!(session.carried.is_empty());
        assert!(session.commit_receipts.is_empty());
        Ok(())
    })?;
    let before = state(&owner)?;
    let valid = query(&owner, std::slice::from_ref(&digest), 0)?;
    assert_eq!(valid["ok"], true, "{valid}");
    assert_eq!(valid["schema"], "dfmcp.lab-original-forecast/5");
    assert_eq!(valid["prediction"]["all_complete"], true);
    assert_eq!(valid["prediction"]["forecast_work"]["recovered_observation_polls"], 0);
    assert_eq!(valid["source_work"]["recovered_observation_steps"], 0);
    assert_eq!(valid["source_work"]["verified_recovered_original_steps"], 1);
    assert!(valid["source_work"]["additional_original_bytes"].as_u64().is_some_and(|n| n > 0));
    assert!(valid["source_work"]["fixed_structural_bytes"].as_u64().is_some());
    assert_eq!(state(&owner)?, before);
    let bytes = valid["source_work"]["inspected_canonical_and_custody_bytes"]
        .as_u64().ok_or("source byte accounting missing")?;
    let baseline = view(&owner, |session| Ok((session.budget, session.grants.clone())))?;
    for constraint in ["source_bytes", "actions", "observe", "query", "plan", "output"] {
        edit(&owner, |session| {
            session.budget = baseline.0;
            session.grants = baseline.1.clone();
            match constraint {
                "source_bytes" => session.budget.max_bytes = bytes - 1,
                "actions" => session.budget.max_actions = 0,
                "observe" => session.grants.retain(|grant| grant.capability != Capability::Observe),
                "query" => session.grants.retain(|grant| grant.capability != Capability::Query),
                "plan" => session.grants.retain(|grant| grant.capability != Capability::Plan),
                _ => session.budget.max_output_tokens = 64,
            }
            Ok(())
        })?;
        let before = state(&owner)?;
        assert_eq!(query(&owner, std::slice::from_ref(&digest), 0)?["ok"], false, "{constraint}");
        assert_eq!(state(&owner)?, before, "{constraint}");
    }
    edit(&owner, |session| {
        session.budget = baseline.0;
        session.grants = baseline.1.clone();
        session.commit_receipts.insert(digest.clone(), "{}".to_owned());
        Ok(())
    })?;
    let before = state(&owner)?;
    assert_eq!(query(&owner, std::slice::from_ref(&digest), 0)?["ok"], false);
    assert_eq!(state(&owner)?, before);
    Ok(())
}
