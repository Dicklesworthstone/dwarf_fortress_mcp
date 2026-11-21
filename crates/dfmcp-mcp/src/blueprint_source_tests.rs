//! New blueprint intake, durable compiler identity and actual facade goal proof.
//! Owning bead: df-lab-shared-workforce-8n4.

use super::*;
use crate::agent_facade as facade;
use crate::server::{PlanSource, original_goal_observation, resolve_session};
use dfmcp_core::FortressId;
use dfmcp_world::PredicateTruth;
use serde_json::{Value, json};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn parsed(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn room() -> Value {
    json!({"template":"dining_hall","origin":[2,6,10],"width":3,"height":3})
}

#[test]
fn current_intake_versions_every_ordinary_template_and_is_idempotent() -> TestResult {
    for request in [
        room(),
        json!({"template":"bedroom_cluster","origin":[2,6,10],"rooms":2,"room_size":[3,3]}),
        json!({"template":"workshop_hub","origin":[2,6,10],"bays":2}),
        json!({"template":"stockpile_vault","origin":[2,6,10],"width":3,"height":3,"category":"wood"}),
    ] {
        let canonical = canonical_current(&request.to_string())?;
        assert_eq!(canonical_current(&canonical)?, canonical);
        let mut retained = parsed(&canonical)?;
        assert_eq!(retained["planner"], CURRENT_PLANNER);
        retained
            .as_object_mut()
            .ok_or("source object missing")?
            .remove("planner");
        assert_eq!(retained, request);
    }
    Ok(())
}

#[test]
fn ordinary_source_parser_refuses_downgrades_duplicates_null_and_input_growth() -> TestResult {
    for marker in [
        json!("furnished_blueprint_v1"),
        json!("future"),
        Value::Null,
        json!(7),
    ] {
        let mut request = room();
        request["planner"] = marker;
        assert!(canonical_current(&request.to_string()).is_err());
    }
    for raw in [
        r#"{"template":"dining_hall","template":"workshop_hub","origin":[2,6,10],"width":3,"height":3}"#,
        r#"{"template":"dining_hall","origin":[2,6,10],"width":3,"width":1,"height":3}"#,
        r#"{"template":"dining_hall","origin":[2,6,10],"width":3,"height":3,"planner":"furnished_blueprint_v2","planner":"furnished_blueprint_v2"}"#,
        r#"{"template":"dining_hall","origin":[2,6,10],"width":3,"height":3,"deadline_tick":1000}"#,
        r#"{"template":"dining_hall","origin":[2,6,10,11],"width":3,"height":3}"#,
    ] {
        assert!(canonical_current(raw).is_err(), "{raw}");
    }
    let huge = format!(
        "{}{}",
        " ".repeat(crate::lab_world::MAX_QUERY_JSON_BYTES),
        room()
    );
    assert!(
        matches!(canonical_current(&huge), Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn durable_ordinary_sources_keep_their_original_compiler_and_exact_goal() -> TestResult {
    let snapshot =
        crate::lab_world::scenario_snapshot("starter_fortress", FortressId::new(459_102), false)?;
    let raw = room().to_string();
    let current_raw = canonical_current(&raw)?;
    let legacy = PlanSource::Blueprint {
        summary: "A hall".to_owned(),
        raw,
    };
    let current = PlanSource::Blueprint {
        summary: "A hall".to_owned(),
        raw: current_raw.clone(),
    };
    let old_intent = legacy.intent(IntentId::new(501), &snapshot)?;
    let new_intent = current.intent(IntentId::new(501), &snapshot)?;
    assert_ne!(old_intent.terminal_condition, new_intent.terminal_condition);
    assert!(old_intent.requested_actions[0].obligation.is_some());
    assert!(new_intent.requested_actions[0].obligation.is_none());
    for (source, expected) in [(&legacy, old_intent), (&current, new_intent)] {
        let durable = source.durable();
        let reopened = PlanSource::from_durable(&durable);
        assert_eq!(reopened.intent(IntentId::new(501), &snapshot)?, expected);
        assert_eq!(reopened.durable(), durable);
    }
    let mut wrong = parsed(&current_raw)?;
    wrong["planner"] = json!("unknown_compiler");
    assert!(compile(IntentId::new(501), &snapshot, &wrong.to_string()).is_err());
    Ok(())
}

#[test]
fn public_ordinary_blueprint_wait_does_not_achieve_goal_before_furniture() -> TestResult {
    let _serial = crate::test_serial();
    let opened = parsed(&facade::fortress_open_session(
        Some(false),
        None,
        Some(
            [
                ("observe", "read_only"),
                ("query", "read_only"),
                ("plan", "read_only"),
                ("control_clock", "reversible"),
                ("checkpoint", "guarded"),
                ("designate", "guarded"),
                ("construct", "guarded"),
            ]
            .into_iter()
            .map(|(c, r)| (c.to_owned(), r.to_owned()))
            .collect(),
        ),
        Some(30_000),
        Some(2_000),
        None,
        None,
        Some(8_192),
        Some(64),
        Some("starter_fortress".to_owned()),
        Some(false),
        Some(false),
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    let session_id = opened["session_id"]
        .as_str()
        .ok_or("session missing")?
        .to_owned();
    let planned = parsed(&facade::fortress_plan(
        Some(session_id.clone()),
        None,
        None,
        None,
        Some(room().to_string()),
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let digest = planned["plan_digest"]
        .as_str()
        .ok_or("plan digest missing")?
        .to_owned();
    let session = resolve_session(Some(session_id.clone()))?;
    {
        let guard = session.lock().map_err(|_| "session poisoned")?;
        let PlanSource::Blueprint { raw, .. } =
            &guard.pending.as_ref().ok_or("pending missing")?.source
        else {
            return Err("ordinary blueprint source missing".into());
        };
        assert_eq!(parsed(raw)?["planner"], CURRENT_PLANNER);
    }
    let committed = parsed(&facade::fortress_commit(
        Some(session_id.clone()),
        digest.clone(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let excavated = parsed(&facade::fortress_wait(Some(session_id.clone()), Some(90)))?;
    assert_eq!(excavated["ok"], true, "{excavated}");
    {
        let guard = session.lock().map_err(|_| "session poisoned")?;
        let (truth, details) = original_goal_observation(&guard, &digest)?;
        assert_ne!(truth, PredicateTruth::True, "{details}");
        assert!(details["achieved_tick"].is_null(), "{details}");
    }
    let furnished = parsed(&facade::fortress_wait(Some(session_id), Some(600)))?;
    assert_eq!(furnished["ok"], true, "{furnished}");
    let guard = session.lock().map_err(|_| "session poisoned")?;
    let (truth, details) = original_goal_observation(&guard, &digest)?;
    assert_eq!(truth, PredicateTruth::True, "{details}");
    assert!(
        details["achieved_tick"]
            .as_u64()
            .is_some_and(|tick| tick >= 590),
        "{details}"
    );
    Ok(())
}
