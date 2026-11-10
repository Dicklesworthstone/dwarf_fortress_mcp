//! Presentation continuity for atomically published resource plans. These
//! fixtures exercise cache/projection behavior; authority remains server-owned.

use super::*;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

const PLAN_DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn register_session(id: &str) -> String {
    let id = id.to_owned();
    let mut opened = json!({
        "ok":true, "session_id":id, "paused":false,
        "granted_capabilities":["observe","query","plan","construct","checkpoint"],
        "budget":{"max_game_ticks":1_000},
        "anchor":{"fortress_id":"7","epoch":1,"sequence":3,"game_tick":100,"state_hash":"fixture"},
    });
    presentation_state("fortress.open_session", &mut opened, Some(&id));
    id
}

fn registered_state(id: &str) -> TestResult<SessionOrientation> {
    orientation_sessions()
        .get(id)
        .cloned()
        .ok_or_else(|| "orientation session is missing".into())
}

fn full_terminal() -> Value {
    json!({"all":[{"entity_exists":"207"},{"paused":false}]})
}

fn shared_plan(id: &str, kind: &str) -> Value {
    json!({
        "ok":true, "session_id":id, "kind":kind,
        "schema":if kind.starts_with("resource_room") {
            "dfmcp.resource-room/2"
        } else {
            "dfmcp.resource-production/2"
        },
        "version":2, "plan_digest":PLAN_DIGEST,
        "required_capabilities":["construct","checkpoint"],
        "predicate_encoding":{
            "schema":"dfmcp.plan-predicate-references/1", "scope":"same_response",
            "plan_digest":PLAN_DIGEST, "values_pointer":"/predicates", "complete":true,
            "projection":"readable_predicate_json", "exact_typed_contract":"retained_plan_archive",
            "order":"terminal_then_steps_postconditions_then_obligation_children_first",
        },
        "predicates":[
            {"entity_exists":"207"}, {"paused":false},
            {"all":[{"$ref":"/predicates/0"},{"$ref":"/predicates/1"}]},
        ],
        "terminal_condition":{"$ref":"/predicates/2"},
        "steps":[{
            "step":0, "kind":"place_furniture", "creates_entity_id":"207",
            "postconditions":[{"$ref":"/predicates/0"}],
            "obligation":{
                "terminal":{"$ref":"/predicates/2"},
                "deadline_tick":1_000, "poll_interval_ticks":7,
            },
        }],
        "forecast":{
            "available":true, "resolution_ticks":10,
            "steps":[{"step":0,"predicted_state":"Verified","predicted_terminal_tick":200}],
        },
    })
}

fn seed_old_pending(id: &str) -> TestResult {
    let mut registry = orientation_sessions();
    let state = registry
        .get_mut(id)
        .ok_or("orientation session is missing")?;
    cache_pending_plan(
        &json!({
            "plan_digest":"old-plan", "required_capabilities":["designate"],
            "steps":[{"step":0,"kind":"old_step"}], "forecast":{"available":false},
        }),
        state,
    );
    state.pending_resource_review = Some((
        "old-plan".to_owned(),
        json!({
            "tool":"fortress.plan", "arguments":{"blueprint":"old request"},
        }),
    ));
    Ok(())
}

#[test]
fn published_resource_plans_refresh_complete_pending_orientation() -> TestResult {
    let _serial = crate::test_serial();
    for kind in [
        "resource_production",
        "resource_production_continuation",
        "resource_room",
        "resource_room_continuation",
        "select_candidate",
    ] {
        let id = register_session(&format!("published-orientation-{kind}"));
        seed_old_pending(&id)?;
        let payload = shared_plan(&id, kind);
        let raw = serde_json::to_string(&payload)?;
        refresh_published_plan_orientation(&raw);
        let state = registered_state(&id)?;
        assert_eq!(state.pending_plan_digest.as_deref(), Some(PLAN_DIGEST));
        assert!(state.pending_resource_review.is_none());
        assert_eq!(state.pending_plan_capabilities, ["construct", "checkpoint"]);
        assert_eq!(state.pending_forecast, payload["forecast"]);
        assert_eq!(
            state.pending_plan_steps[0]["postconditions"],
            json!([{"entity_exists":"207"}])
        );
        assert_eq!(
            state.pending_plan_steps[0]["obligation"]["terminal"],
            full_terminal()
        );
        let work = active_work(&state);
        assert_eq!(work["pending_plans"][0]["state"], "prepared");
        assert_eq!(
            work["pending_plans"][0]["minimum_safe_next_step"]["arguments"]["plan_digest"],
            PLAN_DIGEST
        );
        assert!(affordances(&state).iter().any(|value| {
            value["affordance_id"] == "commit-pending-plan"
                && value["enabled"] == true
                && value["arguments"]["plan_digest"] == PLAN_DIGEST
        }));
        // Refresh takes an immutable view of already-published bytes.
        assert_eq!(serde_json::from_str::<Value>(&raw)?, payload);
    }
    Ok(())
}

#[test]
fn published_resource_steps_remain_self_contained_after_commit_and_observe() -> TestResult {
    let _serial = crate::test_serial();
    let id = register_session("published-orientation-commit");
    refresh_published_plan_orientation(&shared_plan(&id, "resource_room_continuation").to_string());
    let mut commit = json!({
        "ok":true, "session_id":id,
        "observed_anchor":{"fortress_id":"7","epoch":1,"sequence":4,"game_tick":101,"state_hash":"applied"},
        "actions":[{"action_id":"91","step":0,"state":"AppliedAwaitingVerification","work_state":{"quiescent":false}}],
    });
    presentation_state("fortress.commit", &mut commit, Some(&id));
    let mut observed = json!({"ok":true,"session_id":id,"game_tick":102});
    let (state, _, _) = presentation_state("fortress.observe", &mut observed, Some(&id));
    assert!(state.pending_plan_digest.is_none());
    assert!(state.pending_plan_steps.is_empty());
    assert_eq!(state.plan_actions.len(), 1);
    assert_eq!(
        state.plan_actions[0].predicted,
        Some(("Verified".to_owned(), Some(200)))
    );
    let work = active_work(&state);
    assert_eq!(work["actions"][0]["kind"], "place_furniture");
    assert_eq!(work["obligations"][0]["terminal"], full_terminal());
    assert_eq!(work["obligations"][0]["deadline_tick"], 1_000);
    assert_eq!(
        work["obligations"][0]["next_step"]["arguments"]["max_game_ticks"],
        70
    );
    assert!(!work.to_string().contains("\"$ref\""));
    Ok(())
}

#[test]
fn stale_room_review_survives_observation_and_disables_old_commit() -> TestResult {
    let _serial = crate::test_serial();
    let id = register_session("published-orientation-stale-room");
    refresh_published_plan_orientation(&shared_plan(&id, "resource_room").to_string());
    let next = json!({
        "tool":"fortress.plan", "arguments":{
            "blueprint":"{\"template\":\"provisioned_room\",\"acquire_resources\":{\"assign_labor\":false}}",
        },
    });
    let mut refusal = json!({
        "ok":false, "error":{"code":"stale_anchor"},
        "rebase":{"method":"fresh_resource_room_required","pending_plan_preserved":true,"from_digest":PLAN_DIGEST},
        "next_step":next,
    });
    assert_eq!(resource_replan_step(&refusal), Some(&next));
    presentation_state("fortress.commit", &mut refusal, Some(&id));
    let mut observed = json!({"ok":true,"session_id":id,"game_tick":103});
    let (state, _, _) = presentation_state("fortress.observe", &mut observed, Some(&id));
    assert_eq!(state.resource_review_step(), Some(&next));
    let work = active_work(&state);
    assert_eq!(work["pending_plans"][0]["state"], "review_required");
    assert_eq!(work["pending_plans"][0]["minimum_safe_next_step"], next);
    assert!(affordances(&state).iter().any(|value| {
        value["affordance_id"] == "commit-pending-plan" && value["enabled"] == false
    }));
    // A different stale digest cannot redirect this retained proposal.
    refusal["rebase"]["from_digest"] = json!("unrelated-digest");
    refusal["next_step"]["arguments"]["blueprint"] = json!("unrelated request");
    presentation_state("fortress.commit", &mut refusal, Some(&id));
    assert_eq!(registered_state(&id)?.resource_review_step(), Some(&next));
    Ok(())
}

#[test]
fn comparisons_and_refusals_preserve_prior_pending_orientation() -> TestResult {
    let _serial = crate::test_serial();
    let id = register_session("published-orientation-no-replacement");
    seed_old_pending(&id)?;
    let before = registered_state(&id)?;
    let mut comparison = shared_plan(&id, "compare_candidates");
    refresh_published_plan_orientation(&comparison.to_string());
    comparison["kind"] = json!("resource_production");
    comparison["ok"] = json!(false);
    refresh_published_plan_orientation(&comparison.to_string());
    let after = registered_state(&id)?;
    assert_eq!(after.turn_sequence, before.turn_sequence);
    assert_eq!(after.pending_plan_digest, before.pending_plan_digest);
    assert_eq!(
        after.pending_resource_review,
        before.pending_resource_review
    );
    assert_eq!(
        after.pending_plan_capabilities,
        before.pending_plan_capabilities
    );
    assert_eq!(after.pending_plan_steps, before.pending_plan_steps);
    assert_eq!(after.pending_forecast, before.pending_forecast);
    Ok(())
}

#[test]
fn incomplete_published_proof_discards_stale_pending_hints() -> TestResult {
    let _serial = crate::test_serial();
    let id = register_session("published-orientation-invalid-proof");
    for corruption in [
        "dangling_reference",
        "wrong_digest",
        "wrong_version",
        "missing_table",
    ] {
        seed_old_pending(&id)?;
        let mut payload = shared_plan(&id, "select_candidate");
        match corruption {
            "dangling_reference" => {
                payload["steps"][0]["obligation"]["terminal"] = json!({"$ref":"/predicates/99"})
            }
            "wrong_digest" => payload["predicate_encoding"]["plan_digest"] = json!("unrelated"),
            "wrong_version" => payload["version"] = json!(1),
            "missing_table" => {
                payload
                    .as_object_mut()
                    .ok_or("payload is not an object")?
                    .remove("predicates");
            }
            _ => unreachable!(),
        }
        let raw = payload.to_string();
        refresh_published_plan_orientation(&raw);
        let state = registered_state(&id)?;
        assert!(state.pending_plan_digest.is_none(), "{corruption}");
        assert!(state.pending_resource_review.is_none(), "{corruption}");
        assert!(state.pending_plan_capabilities.is_empty(), "{corruption}");
        assert!(state.pending_plan_steps.is_empty(), "{corruption}");
        assert!(state.pending_forecast.is_null(), "{corruption}");
        assert!(
            !affordances(&state)
                .iter()
                .any(|value| value["affordance_id"] == "commit-pending-plan")
        );
        assert_eq!(serde_json::from_str::<Value>(&raw)?, payload);
    }
    Ok(())
}
