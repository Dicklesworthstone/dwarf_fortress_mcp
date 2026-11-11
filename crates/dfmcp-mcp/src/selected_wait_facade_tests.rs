//! Targeted waiting through the actual public facade, replay and wire schema.

use super::*;
use serde_json::{Value, json};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn payload(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn open(selector: &str, output_tokens: u32) -> TestResult<String> {
    let opened = payload(&fortress_open_session(
        Some(false),
        Some(selector.to_owned()),
        Some(
            [
                ("observe", "read_only"),
                ("query", "read_only"),
                ("plan", "reversible"),
                ("control_clock", "reversible"),
                ("checkpoint", "guarded"),
                ("configure_production", "reversible"),
            ]
            .into_iter()
            .map(|(capability, risk)| (capability.to_owned(), risk.to_owned()))
            .collect(),
        ),
        Some(30_000),
        Some(2_000),
        None,
        None,
        Some(output_tokens),
        Some(64),
        Some("starter_fortress".to_owned()),
        Some(false),
        Some(false),
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    Ok(opened["session_id"]
        .as_str()
        .ok_or("session missing")?
        .to_owned())
}

fn production(session: &str) -> TestResult<String> {
    let planned = payload(&fortress_plan(
        Some(session.to_owned()),
        None,
        None,
        None,
        Some(
            json!({"template":"production", "quotas":[{"item":"DRINK", "minimum":50}]}).to_string(),
        ),
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let digest = planned["plan_digest"]
        .as_str()
        .ok_or("digest missing")?
        .to_owned();
    let committed = payload(&fortress_commit(Some(session.to_owned()), digest.clone()))?;
    assert_eq!(committed["ok"], true, "{committed}");
    Ok(digest)
}

#[test]
fn selected_wait_returns_before_consumption_undoes_the_original_goal() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7855201", 8_192)?;
    let digest = production(&session)?;
    let waited = payload(&fortress_wait_for_plan(
        Some(session.clone()),
        Some(2_000),
        Some(digest.clone()),
    ))?;
    assert_eq!(waited["ok"], true, "{waited}");
    let selected = &waited["selected_plan_wait"];
    assert_eq!(selected["requested_plan_digest"], digest);
    assert_eq!(selected["stop_reason"], "complete", "{waited}");
    assert_eq!(selected["goal_truth"], "true");
    assert_eq!(selected["actions_verified"], true);
    assert_eq!(selected["physical_quiescent"], true);
    assert_eq!(selected["complete"], true);
    let advanced = waited["advanced_game_ticks"]
        .as_u64()
        .ok_or("elapsed ticks missing")?;
    assert!(advanced > 0 && advanced < 1_200, "{waited}");
    assert_eq!(
        waited["wait_progress"]["remaining_game_ticks"],
        2_000 - advanced
    );
    assert_eq!(waited["agent_turn"]["anchor"], waited["observed_anchor"]);
    assert!(
        !waited["agent_turn"]["recommendations"]
            .as_array()
            .ok_or("recommendations missing")?
            .iter()
            .any(|item| item["recommendation_id"] == "advance-and-verify-work")
    );

    // Omitted selection retains the previous contract: elapsed time is useful
    // independently of whether this particular project has already finished.
    let later = payload(&fortress_wait(Some(session), Some(2_000)))?;
    assert_eq!(later["ok"], true, "{later}");
    assert_eq!(later["advanced_game_ticks"], 2_000);
    assert!(later.get("selected_plan_wait").is_none());
    Ok(())
}

#[test]
fn selected_wait_replays_its_exact_target_and_early_frontier() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7855202", 8_192)?;
    let digest = production(&session)?;
    let waited = payload(&fortress_wait_for_plan(
        Some(session.clone()),
        Some(2_000),
        Some(digest.clone()),
    ))?;
    assert_eq!(
        waited["selected_plan_wait"]["stop_reason"], "complete",
        "{waited}"
    );
    let bundle = crate::server::replay_bundle_for(&session)?;
    let last = bundle["calls"]
        .as_array()
        .and_then(|calls| calls.last())
        .ok_or("call missing")?;
    assert_eq!(last["tool"], "fortress.wait");
    assert_eq!(last["arguments"]["plan_digest"], digest);
    assert_eq!(last["arguments"]["max_game_ticks"], 2_000);
    let report = crate::replay::replay_bundle(&bundle);
    assert_eq!(report["ok"], true, "{report}");
    let replayed = report["replay_session_id"]
        .as_str()
        .ok_or("replay session missing")?;
    assert_eq!(crate::server::replay_bundle_for(replayed)?, bundle);
    Ok(())
}

#[test]
fn wire_definition_preserves_one_wait_name_with_optional_original_target() {
    let definition = FortressWait.definition();
    assert_eq!(definition.name, "fortress_wait");
    let properties = &definition.input_schema["properties"];
    assert!(properties.get("session_id").is_some());
    assert!(properties.get("max_game_ticks").is_some());
    assert!(properties.get("plan_digest").is_some());
    assert!(
        !definition.input_schema["required"]
            .as_array()
            .is_some_and(|fields| fields.iter().any(|field| field == "plan_digest"))
    );
}

#[test]
fn invalid_target_does_not_advance_or_replace_the_current_project() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7855203", 8_192)?;
    let digest = production(&session)?;
    let before = payload(&fortress_observe(Some(session.clone())))?;
    for target in ["not-a-digest".to_owned(), "ab".repeat(32)] {
        let refused = payload(&fortress_wait_for_plan(
            Some(session.clone()),
            Some(2_000),
            Some(target),
        ))?;
        assert_eq!(refused["ok"], false, "{refused}");
        let after = payload(&fortress_observe(Some(session.clone())))?;
        assert_eq!(
            after["agent_turn"]["anchor"],
            before["agent_turn"]["anchor"]
        );
    }
    let waited = payload(&fortress_wait_for_plan(
        Some(session),
        Some(2_000),
        Some(digest),
    ))?;
    assert_eq!(
        waited["selected_plan_wait"]["stop_reason"], "complete",
        "{waited}"
    );
    Ok(())
}

#[test]
fn selected_outcome_and_elapsed_work_survive_every_fitting_output_tier() -> TestResult {
    let selected = json!({
        "schema":"dfmcp.selected-plan-wait/1", "requested_plan_digest":"ab".repeat(32),
        "plan_digest":"ab".repeat(32), "stop_reason":"failed", "complete":false,
        "goal_truth":"false", "actions_verified":false, "physical_quiescent":false,
        "reserve_frontier":null, "selection_scope":"exact_original_commit",
        "descendant_plans_included":false,
    });
    let progress = json!({"requested_game_ticks":2000, "advanced_game_ticks":200, "remaining_game_ticks":1800});
    let original = json!({
        "ok":true, "session_id":"s", "selected_plan_wait":selected, "wait_progress":progress,
        "advanced_game_ticks":200, "game_tick":201, "paused":false,
        "diagnostic_detail":"x".repeat(6_000),
        "polled_actions":(0..30).map(|id| json!({"action_id":id, "message":"x".repeat(500)})).collect::<Vec<_>>(),
        "agent_turn":{"schema":"dfmcp.agent_turn/1", "operation":"fortress.wait", "anchor":{"game_tick":201},
            "continuity":{"status":"continuous"}, "active_work":{}, "recommendations":[], "coverage":{}},
    });
    let mut minimal_seen = false;
    for tokens in [20_000, 4_096, 2_000, 1_000, 500, 300, 200, 50] {
        let fitted = payload(&crate::output_budget::fit(&original.to_string(), tokens))?;
        if fitted["output_budget"]["tier"] == "refused" {
            assert_eq!(fitted["ok"], false);
        } else {
            assert_eq!(fitted["selected_plan_wait"], selected, "{fitted}");
            assert_eq!(fitted["wait_progress"], progress, "{fitted}");
            assert_eq!(fitted["advanced_game_ticks"], 200);
            minimal_seen |= fitted["output_budget"]["tier"] == "minimal";
        }
    }
    assert!(
        minimal_seen,
        "test must exercise the minimal selected outcome envelope"
    );
    Ok(())
}
