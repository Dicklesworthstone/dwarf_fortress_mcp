//! Actual durable reopen, original custody, query admission and read-only
//! publication for recovered complete-project forecasts.

use super::*;
use dfmcp_core::Capability;

#[path = "recovered_continuation_forecast_mcp_tests.rs"]
mod chains;

#[path = "recovered_active_forecast_mcp_tests.rs"]
mod active;

struct StateDir(std::path::PathBuf);
impl StateDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-recovered-project-forecast-{label}-{}",
            std::process::id(),
        ));
        let _ = std::fs::remove_dir_all(&path);
        server::simulate_durable_restart(Some(path.clone()));
        Self(path)
    }
    fn restart(&self) {
        server::simulate_durable_restart(Some(self.0.clone()));
    }
}
impl Drop for StateDir {
    fn drop(&mut self) {
        server::simulate_durable_restart(None);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn open_durable(selector: &str) -> TestResult<Owner> {
    let opened = parsed(&facade::fortress_open_session(
        Some(false),
        Some(selector.to_owned()),
        Some(
            [
                ("observe", "read_only"),
                ("query", "read_only"),
                ("plan", "guarded"),
                ("configure_production", "reversible"),
                ("configure_labor", "reversible"),
                ("designate", "guarded"),
                ("construct", "guarded"),
                ("checkpoint", "guarded"),
                ("control_clock", "reversible"),
            ]
            .into_iter()
            .map(|(capability, risk)| (capability.to_owned(), risk.to_owned()))
            .collect(),
        ),
        Some(60_000),
        Some(4_000),
        Some(32_768),
        Some(4 * 1024 * 1024),
        Some(32_768),
        Some(64),
        Some("starter_fortress_resources".to_owned()),
        Some(false),
        Some(true),
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    Ok(Owner(field(&opened, "session_id")?))
}

fn beds_request() -> Value {
    json!({"template":"production","quotas":[{"item":"BED","minimum":24}],
        "acquire_resources":{"assign_labor":false}})
}

fn original(owner: &Owner) -> TestResult<String> {
    commit(owner, None, Some(beds_request()))
}

fn settle(owner: &Owner, digest: &str) -> TestResult<Value> {
    let waited = parsed(&facade::fortress_wait_for_plan(
        Some(owner.0.clone()),
        Some(2_000),
        Some(digest.to_owned()),
    ))?;
    assert_eq!(waited["ok"], true, "{waited}");
    assert_eq!(
        waited["selected_plan_wait"]["actions_verified"], true,
        "{waited}"
    );
    assert_eq!(
        waited["selected_plan_wait"]["physical_quiescent"], true,
        "{waited}"
    );
    assert_eq!(
        waited["selected_plan_wait"]["goal_truth"], "false",
        "{waited}"
    );
    Ok(waited)
}

fn forecast(owner: &Owner, digest: &str, ticks: u64) -> TestResult<Value> {
    parsed(&facade::fortress_query(
        Some(owner.0.clone()),
        Some(
            json!({
                "mode":"forecast_originals","plan_digests":[digest],"horizon_ticks":ticks,
                "forecast_scope":"complete_project",
            })
            .to_string(),
        ),
    ))
}

fn state(owner: &Owner) -> TestResult<Value> {
    view(owner, |session| {
        Ok(json!({
            "adapter":format!("{:?}",session.adapter),
            "pending":session.pending.as_ref().map(|pending|pending.plan.digest.to_hex()),
            "receipts":session.commit_receipts,
            "actions":format!("{:?}",session.open_actions),
            "carried":format!("{:?}",session.carried),
            "objectives":format!("{:?}",session.objectives),
            "history":session.history.len(),
        }))
    })
}

#[test]
fn durable_recovered_harvest_forecasts_production_without_hydrating_or_changing_custody()
-> TestResult {
    let _serial = crate::test_serial();
    let _durable = server::serialized_durable_tests();
    let directory = StateDir::new("complete");
    let first = open_durable("8161501")?;
    let digest = original(&first)?;
    settle(&first, &digest)?;
    directory.restart();
    drop(first);
    for _ in 0..2 {
        let owner = open_durable("8161501")?;
        let old_stage = query(&owner, std::slice::from_ref(&digest), 0)?;
        assert_eq!(old_stage["ok"], false, "{old_stage}");
        let pending = plan(&owner, Some(order("preserve this pending review", 1)), None)?;
        let before = state(&owner)?;
        let result = forecast(&owner, &digest, 4_000)?;
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["schema"], "dfmcp.lab-original-forecast/3");
        assert_eq!(result["prediction"]["all_complete"], true, "{result}");
        let row = &result["prediction"]["originals"][0];
        assert_eq!(row["actual_plan_digest"], digest);
        assert_eq!(row["selection_origin"], "durable_recovery");
        assert_eq!(row["dispatch_handles_restored"], false);
        assert_eq!(row["predicted_actions_complete"], true);
        assert_eq!(row["future_stages"][0]["stage"], "production");
        assert!(row["steps"].as_array().is_some_and(
            |rows| !rows.is_empty() && rows.iter().all(|step| step["action_id"].is_null())
        ));
        assert!(
            result["source_work"]["recovered_original_source_witnesses"]
                .as_array()
                .is_some_and(|rows| rows.len() == 1
                    && rows[0]["query_archive_reads"] == 0
                    && rows[0]["canonical_bytes"]
                        .as_u64()
                        .is_some_and(|bytes| bytes > 0))
        );
        assert_eq!(
            result["prediction"]["forecast_work"]["actual_game_ticks_advanced"],
            0
        );
        assert_eq!(
            result["agent_turn"]["coverage"]["eligible_as_mutation_evidence"],
            false
        );
        assert_eq!(state(&owner)?, before);
        assert_eq!(before["pending"], pending);
        directory.restart();
        drop(owner);
        let mut store = dfmcp_lab::durable::DurableLabStore::open(&directory.0)?;
        store.compact()?;
    }
    Ok(())
}

#[test]
fn recovery_forecast_observes_unfinished_work_and_charges_archived_sources_and_steps() -> TestResult
{
    let _serial = crate::test_serial();
    let _durable = server::serialized_durable_tests();
    let directory = StateDir::new("bounds");
    let first = open_durable("8161502")?;
    let digest = original(&first)?;
    directory.restart();
    drop(first);
    let owner = open_durable("8161502")?;
    let before = state(&owner)?;
    let active = forecast(&owner, &digest, 4_000)?;
    assert_eq!(active["ok"], true, "{active}");
    assert_eq!(active["schema"], "dfmcp.lab-original-forecast/4");
    assert_eq!(active["prediction"]["all_complete"], true, "{active}");
    assert_eq!(active["prediction"]["forecast_work"]["actual_game_ticks_advanced"], 0);
    assert_eq!(state(&owner)?, before);
    settle(&owner, &digest)?;
    let zero = forecast(&owner, &digest, 0)?;
    assert_eq!(zero["ok"], true, "{zero}");
    assert_eq!(zero["prediction"]["all_complete"], false);
    let baseline = view(&owner, |session| {
        Ok((session.budget, session.grants.clone()))
    })?;
    let retained_records = zero["source_work"]["retained_source_records"]
        .as_u64()
        .ok_or("source records missing")?;
    let original_steps = zero["source_work"]["verified_recovered_original_steps"]
        .as_u64()
        .ok_or("recovered step count missing")?;
    for constraint in [
        "source_records",
        "cumulative_actions",
        "current_plan",
        "future_production",
        "output",
    ] {
        edit(&owner, |session| {
            session.budget = baseline.0;
            session.grants = baseline.1.clone();
            match constraint {
                "source_records" => session.budget.max_entities = u32::try_from(retained_records)?,
                "cumulative_actions" => session.budget.max_actions = u32::try_from(original_steps)?,
                "current_plan" => session
                    .grants
                    .retain(|grant| grant.capability != Capability::Plan),
                "future_production" => session
                    .grants
                    .retain(|grant| grant.capability != Capability::ConfigureProduction),
                _ => session.budget.max_output_tokens = 64,
            }
            Ok(())
        })?;
        let before = state(&owner)?;
        let refused = forecast(&owner, &digest, 4_000)?;
        assert_eq!(refused["ok"], false, "{constraint}: {refused}");
        assert_eq!(state(&owner)?, before, "{constraint}");
    }
    edit(&owner, |session| {
        session.budget = baseline.0;
        session.grants = baseline.1.clone();
        Ok(())
    })?;
    assert_eq!(forecast(&owner, &digest, 4_000)?["ok"], true);
    Ok(())
}

#[test]
fn recovery_forecast_never_uses_recovery_to_bypass_corrupt_live_receipts_or_replaced_ledger()
-> TestResult {
    let _serial = crate::test_serial();
    let _durable = server::serialized_durable_tests();
    let directory = StateDir::new("custody");
    let first = open_durable("8161503")?;
    let digest = original(&first)?;
    settle(&first, &digest)?;
    directory.restart();
    drop(first);
    let owner = open_durable("8161503")?;
    edit(&owner, |session| {
        session
            .commit_receipts
            .insert(digest.clone(), "{}".to_owned());
        Ok(())
    })?;
    let before = state(&owner)?;
    assert_eq!(forecast(&owner, &digest, 4_000)?["ok"], false);
    assert_eq!(state(&owner)?, before);
    edit(&owner, |session| {
        session.commit_receipts.remove(&digest);
        Ok(())
    })?;
    assert_eq!(forecast(&owner, &digest, 4_000)?["ok"], true);
    edit(&owner, |session| {
        let mut snapshot = session.adapter.snapshot().clone();
        let ledger =
            dfmcp_intent::effects::stock_ledger(&snapshot).ok_or("stock ledger missing")?;
        snapshot
            .graph
            .entities
            .get_mut(&ledger)
            .ok_or("stock ledger missing")?
            .generation += 1;
        snapshot.refresh_hash();
        session.adapter = dfmcp_lab::MemoryAdapter::new(snapshot);
        Ok(())
    })?;
    let before = state(&owner)?;
    assert_eq!(forecast(&owner, &digest, 4_000)?["ok"], false);
    assert_eq!(state(&owner)?, before);
    Ok(())
}

#[test]
fn recovered_reserve_forecast_requires_the_exact_retained_endpoint_without_query_hydration()
-> TestResult {
    let _serial = crate::test_serial();
    let _durable = server::serialized_durable_tests();
    let directory = StateDir::new("reserve");
    let first = open_durable("8161504")?;
    let digest = commit(
        &first,
        None,
        Some(json!({
            "template":"production", "reserve_until_tick":1_234,
            "quotas":[{"item":"DRINK","minimum":40},{"item":"FOOD","minimum":60}],
        })),
    )?;
    let observed = parsed(&facade::fortress_wait_for_plan(
        Some(first.0.clone()),
        Some(2_000),
        Some(digest.clone()),
    ))?;
    assert_eq!(observed["ok"], true, "{observed}");
    assert_eq!(
        observed["selected_plan_wait"]["stop_reason"], "complete",
        "{observed}"
    );
    assert_eq!(observed["game_tick"], 1_234);
    let endpoint = observed["selected_plan_wait"]["reserve_frontier"].clone();
    assert_eq!(endpoint["truth"], "true");
    assert_eq!(endpoint["durable"], true);
    let advanced = parsed(&facade::fortress_wait(Some(first.0.clone()), Some(1)))?;
    assert_eq!(advanced["ok"], true, "{advanced}");
    directory.restart();
    drop(first);

    let owner = open_durable("8161504")?;
    let original = server::selected_plan_wait::parse_digest(&digest)?;
    let retained = view(&owner, |session| {
        let selected = session
            .objectives
            .iter()
            .find(|goal| goal.plan_digest == original)
            .ok_or("recovered reserve original missing")?;
        let retained = selected
            .selected_wait_reserve_observation
            .ok_or("recovered reserve endpoint missing")?;
        assert!(session.history.get(&retained.0.state_hash).is_none());
        Ok(retained)
    })?;
    let before = state(&owner)?;
    let refused = forecast(&owner, &digest, 0)?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(state(&owner)?, before);
    let waited = parsed(&facade::fortress_wait_for_plan(
        Some(owner.0.clone()),
        Some(0),
        Some(digest.clone()),
    ))?;
    assert_eq!(waited["ok"], true, "{waited}");
    assert_eq!(waited["advanced_game_ticks"], 0);
    assert_eq!(
        waited["selected_plan_wait"]["reserve_frontier"]["observed_anchor"],
        endpoint["observed_anchor"]
    );
    view(&owner, |session| {
        let selected = session
            .objectives
            .iter()
            .find(|goal| goal.plan_digest == original)
            .ok_or("recovered reserve original missing")?;
        assert_eq!(selected.selected_wait_reserve_observation, Some(retained));
        assert_eq!(
            session
                .history
                .get(&retained.0.state_hash)
                .map(|source| source.anchor()),
            Some(retained.0)
        );
        Ok(())
    })?;
    let before = state(&owner)?;
    let result = forecast(&owner, &digest, 0)?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["prediction"]["all_complete"], true, "{result}");
    assert_eq!(
        result["prediction"]["originals"][0]["reserve_frontier_truth"],
        "true"
    );
    assert_eq!(state(&owner)?, before);
    Ok(())
}
