//! A real modern MCP client distinguishes a finished acquisition stage from
//! a predicted complete production project without advancing canonical time.
//! Owning bead: df-lab-joint-original-forecast-c2s.

use super::*;

#[test]
fn complete_original_project_forecast_crosses_the_resource_handoff_over_stdio() -> TestResult {
    let mut client = StdioClient::spawn(false)?;
    client.discover()?;
    let opened = client.tool(
        "fortress_open_session",
        json!({
            "fortress_selector": "93803", "paused": false,
            "scenario": "starter_fortress_resources",
            "max_wall_millis": 60_000, "max_game_ticks": 4_000,
            "max_entities": 32_768, "max_bytes": 4 * 1024 * 1024,
            "max_output_tokens": 32_768, "max_actions": 64,
            "requested_capabilities": [
                ["observe", "read_only"], ["query", "read_only"],
                ["plan", "reversible"], ["configure_production", "reversible"],
                ["configure_labor", "reversible"], ["designate", "guarded"],
                ["checkpoint", "guarded"], ["control_clock", "reversible"]
            ]
        }),
    )?;
    let session = opened["session_id"]
        .as_str()
        .ok_or("session identity missing")?;
    let plan = client.tool(
        "fortress_plan",
        json!({"session_id": session, "blueprint": json!({
            "template": "production", "quotas": [{"item": "BED", "minimum": 24}],
            "acquire_resources": {"assign_labor": false}
        }).to_string()}),
    )?;
    let committed = client.tool(
        "fortress_commit",
        json!({"session_id": session, "plan_digest": plan_digest(&plan)?}),
    )?;
    let digest = plan_digest(&committed)?;
    let before = client.query(session, json!({"mode": "summary"}))?;
    let current = client.query(
        session,
        json!({
            "mode": "forecast_originals", "plan_digests": [digest], "horizon_ticks": 4_000
        }),
    )?;
    assert_eq!(current["schema"], "dfmcp.lab-original-forecast/1");
    assert_eq!(current["prediction"]["all_complete"], false);
    // The original acquisition seal retains the full output predicate. The
    // existing /1 result stays unmet even after the harvest itself is Verified.
    assert_eq!(
        current["prediction"]["originals"][0]["predicted_outcome"],
        "goal_unmet"
    );

    let complete = client.query(
        session,
        json!({
            "mode": "forecast_originals", "plan_digests": [digest],
            "horizon_ticks": 4_000, "forecast_scope": "complete_project"
        }),
    )?;
    assert_eq!(complete["schema"], "dfmcp.lab-original-forecast/2");
    assert_eq!(complete["prediction"]["epistemic_state"], "predicted");
    assert_eq!(complete["prediction"]["all_complete"], true, "{complete}");
    let original = &complete["prediction"]["originals"][0];
    assert_eq!(original["actual_plan_digest"], digest);
    assert_eq!(original["predicted_project_actions_complete"], true);
    assert!(
        !original["future_stages"]
            .as_array()
            .ok_or("modeled stages missing")?
            .is_empty(),
        "{complete}"
    );
    assert_eq!(
        complete["prediction"]["forecast_work"]["actual_game_ticks_advanced"],
        0
    );
    assert_eq!(complete["agent_turn"]["anchor"], complete["anchor"]);
    assert_eq!(
        complete["agent_turn"]["coverage"]["eligible_as_mutation_evidence"],
        false
    );
    let after = client.query(session, json!({"mode": "summary"}))?;
    assert_eq!(after["anchor"], before["anchor"]);
    assert_eq!(complete["anchor"], before["anchor"]);
    Ok(())
}
