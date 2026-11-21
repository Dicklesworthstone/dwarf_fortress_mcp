//! Actual modern stdio coverage for bounded original-set cancellation.
//! Owning bead: df-cancellation-progress-certificates-vvw.

use super::*;

fn commit_dig(
    client: &mut StdioClient,
    session: &str,
    tile: [i32; 3],
) -> TestResult<(String, String)> {
    let plan = client.tool(
        "fortress_plan",
        json!({
            "session_id": session,
            "summary": "independently owned excavation for selected cleanup",
            "actions": json!([{"action": {
                "kind": "designate_dig", "min": tile, "max": tile, "mode": "mine"
            }}]).to_string()
        }),
    )?;
    let entity = plan["steps"][0]["creates_entity_id"]
        .as_str()
        .ok_or("designation identity missing")?
        .to_owned();
    let committed = client.tool(
        "fortress_commit",
        json!({"session_id": session, "plan_digest": plan_digest(&plan)?}),
    )?;
    Ok((plan_digest(&committed)?.to_owned(), entity))
}

#[test]
fn selected_original_set_stops_only_requested_work_over_modern_stdio() -> TestResult {
    let mut client = StdioClient::spawn(false)?;
    client.discover()?;
    let session = client.open("93801", true)?;
    let first = commit_dig(&mut client, &session, [3, 5, 10])?;
    let keep = commit_dig(&mut client, &session, [4, 5, 10])?;
    let last = commit_dig(&mut client, &session, [5, 5, 10])?;
    let before = client.entities(&session, "dig_designation")?;
    let original_kept = find_entity(&before, &keep.1)
        .ok_or("unselected designation missing")?
        .clone();
    let scope = format!("plans:{},{}", last.0, first.0);
    let result = client.tool(
        "fortress_cancel",
        json!({"session_id": session, "scope": scope, "mode": "stop_future_steps"}),
    )?;
    assert_eq!(result["drain_progress"]["original_plans_total"], 2);
    assert_eq!(result["drain_progress"]["quiescent"], true, "{result}");
    assert!(result["finalize_certificate"].is_object(), "{result}");
    assert_eq!(
        result["selected_plans_digest"].as_str().map(str::len),
        Some(64)
    );
    let after = client.entities(&session, "dig_designation")?;
    assert_eq!(after["anchor"]["game_tick"], before["anchor"]["game_tick"]);
    assert_eq!(find_entity(&after, &keep.1), Some(&original_kept));
    for selected in [&first, &last] {
        assert_eq!(
            find_entity(&after, &selected.1).ok_or("selected designation missing")?["fields"]["status"],
            "cancelled"
        );
    }

    let repeated = client.tool(
        "fortress_cancel",
        json!({"session_id": session, "scope": scope, "mode": "stop_future_steps"}),
    )?;
    assert_eq!(repeated["drain_progress"]["quiescent"], true);
    assert_eq!(
        repeated["selected_plans_digest"],
        result["selected_plans_digest"]
    );
    let settled = client.entities(&session, "dig_designation")?;
    assert_eq!(settled["anchor"], after["anchor"]);
    assert_eq!(settled["rows"], after["rows"]);
    Ok(())
}

#[test]
fn invalid_member_refuses_the_whole_selected_stop_over_modern_stdio() -> TestResult {
    let mut client = StdioClient::spawn(false)?;
    client.discover()?;
    let session = client.open("93802", true)?;
    let first = commit_dig(&mut client, &session, [3, 5, 10])?;
    let second = commit_dig(&mut client, &session, [4, 5, 10])?;
    let before = client.entities(&session, "dig_designation")?;
    for scope in [
        format!("plans:{},{}", first.0, "0".repeat(64)),
        format!("plans:{},{}", first.0, first.0),
        format!("plans:{},{}", first.0, "A".repeat(64)),
        format!("plans:{},{} ", first.0, second.0),
        "plans:".to_owned(),
    ] {
        let refused = client.rpc(
            "tools/call",
            json!({"name": "fortress_cancel", "arguments": {
                "session_id": session, "scope": scope, "mode": "stop_future_steps"
            }}),
        )?;
        // Ordinary fortress refusals can use the semantic Agent Turn envelope;
        // protocol-level refusal is also valid and must not produce an effect.
        let semantic = refused["result"]
            .get("structuredContent")
            .cloned()
            .or_else(|| {
                refused["result"]["content"][0]["text"]
                    .as_str()
                    .and_then(|text| serde_json::from_str::<Value>(text).ok())
            });
        assert!(
            refused["error"].is_object()
                || refused["result"]["isError"] == true
                || semantic
                    .as_ref()
                    .is_some_and(|payload| payload["ok"] == false),
            "selected-set refusal was lost: {refused}"
        );
        let unchanged = client.entities(&session, "dig_designation")?;
        assert_eq!(unchanged["anchor"], before["anchor"]);
        assert_eq!(unchanged["rows"], before["rows"]);
    }
    Ok(())
}
