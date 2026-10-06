//! The agent-facing laboratory loop for non-pause actions: open a scenario,
//! inspect entities and terrain, plan dependent semantic steps, commit, let
//! game time pass with `fortress.wait`, and observe proven completion.
use super::*;

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn parsed(raw: &str) -> std::result::Result<Value, Box<dyn std::error::Error>> {
    Ok(serde_json::from_str(raw)?)
}

fn caps(extra: &[(&str, &str)]) -> Vec<(String, String)> {
    [
        ("observe", "read_only"),
        ("query", "read_only"),
        ("plan", "reversible"),
        ("control_clock", "reversible"),
        ("checkpoint", "guarded"),
        ("doctor", "read_only"),
    ]
    .iter()
    .chain(extra)
    .map(|(c, r)| ((*c).to_owned(), (*r).to_owned()))
    .collect()
}

fn open(
    selector: &str,
    paused: bool,
    extra: &[(&str, &str)],
) -> std::result::Result<String, Box<dyn std::error::Error>> {
    let opened = parsed(&fortress_open_session(
        Some(paused),
        Some(selector.to_owned()),
        Some(caps(extra)),
        None,
        Some(2_000),
        None,
        None,
        Some(8_192),
        None,
        Some("starter_fortress".to_owned()),
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    assert_eq!(opened["scenario"], "starter_fortress");
    Ok(opened["session_id"]
        .as_str()
        .ok_or("session_id missing")?
        .to_owned())
}

const ALL_EFFECTS: [(&str, &str); 4] = [
    ("designate", "guarded"),
    ("construct", "guarded"),
    ("configure_labor", "reversible"),
    ("configure_production", "reversible"),
];

const WORKSHOP_PLAN: &str = r#"[
  {"action":{"kind":"designate_dig","min":[0,3,10],"max":[4,5,10],"mode":"mine"}},
  {"action":{"kind":"build","building":"workshop:Still","location":[2,4,10],"min":[1,3,10],"max":[3,5,10]},"depends_on":[0]},
  {"action":{"kind":"create_work_order","name":"brew","job_token":"BREW_DRINK","amount":2},"depends_on":[1]},
  {"action":{"kind":"set_labor","units":["1003"],"labor":"BREW","enabled":true}}
]"#;

#[test]
fn agent_digs_builds_and_brews_through_the_eleven_tools() -> TestResult {
    let session = open("72001", false, &ALL_EFFECTS)?;

    let units = parsed(&fortress_query(
        Some(session.clone()),
        Some(r#"{"mode":"entities","kind":"unit"}"#.to_owned()),
    ))?;
    assert_eq!(units["ok"], true, "{units}");
    assert_eq!(units["total"], 7);
    let before = parsed(&fortress_query(
        Some(session.clone()),
        Some(r#"{"mode":"terrain","min":[0,2,10],"max":[4,5,10]}"#.to_owned()),
    ))?;
    assert_eq!(before["levels"][0]["rows"][0], ".....");
    assert_eq!(before["levels"][0]["rows"][1], "#####");

    let planned = parsed(&fortress_plan(
        Some(session.clone()),
        Some("brewery".to_owned()),
        None,
        Some(WORKSHOP_PLAN.to_owned()),
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    assert_eq!(planned["max_risk"], "guarded");
    assert_eq!(planned["steps"].as_array().map(Vec::len), Some(4));
    assert_eq!(
        planned["steps"][0]["postconditions"][0]["region_terrain"]["tile"],
        "floor"
    );
    let building = planned["steps"][1]["creates_entity_id"]
        .as_str()
        .ok_or("build step names no created entity")?
        .to_owned();
    assert_eq!(planned["steps"][3]["compensable"], true);
    let digest = planned["plan_digest"].as_str().ok_or("digest")?.to_owned();

    let committed = parsed(&fortress_commit(Some(session.clone()), digest.clone()))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let states: Vec<&str> = committed["actions"]
        .as_array()
        .ok_or("actions")?
        .iter()
        .filter_map(|a| a["state"].as_str())
        .collect();
    assert_eq!(
        states,
        [
            "AppliedAwaitingVerification",
            "Prepared",
            "Prepared",
            "Verified"
        ]
    );
    // The Agent Turn carries every action and each open obligation with its
    // deadline, and recommends letting bounded game time pass.
    let turn = &committed["agent_turn"];
    assert_eq!(
        turn["active_work"]["actions"].as_array().map(Vec::len),
        Some(4)
    );
    assert_eq!(
        turn["active_work"]["obligations"].as_array().map(Vec::len),
        Some(3)
    );
    assert!(turn["active_work"]["obligations"][0]["deadline_tick"].is_u64());
    assert_eq!(turn["recommendations"][0]["tool"], "fortress.wait");
    assert_eq!(
        turn["recommendations"][0]["arguments"]["max_game_ticks"],
        100
    );

    let mut finished = false;
    for _ in 0..40 {
        let waited = parsed(&fortress_wait(Some(session.clone()), Some(100)))?;
        assert_eq!(waited["ok"], true, "{waited}");
        assert_eq!(waited["advanced_game_ticks"], 100);
        let states: Vec<&str> = waited["polled_actions"]
            .as_array()
            .ok_or("polled_actions")?
            .iter()
            .filter_map(|a| a["state"].as_str())
            .collect();
        assert!(!states.contains(&"Failed"), "{waited}");
        if states.iter().all(|state| *state == "Verified") {
            finished = true;
            break;
        }
    }
    assert!(finished, "plan never verified");
    let settled = parsed(&fortress_observe(Some(session.clone())))?;
    assert_eq!(
        settled["agent_turn"]["active_work"]["obligations"],
        json!([])
    );

    let after = parsed(&fortress_query(
        Some(session.clone()),
        Some(r#"{"mode":"terrain","min":[0,2,10],"max":[4,5,10]}"#.to_owned()),
    ))?;
    for row in 1..=3 {
        assert_eq!(after["levels"][0]["rows"][row], ".....");
    }
    let buildings = parsed(&fortress_query(
        Some(session.clone()),
        Some(r#"{"mode":"entities","kind":"building"}"#.to_owned()),
    ))?;
    assert_eq!(buildings["total"], 1);
    assert_eq!(buildings["rows"][0]["entity_id"], building);
    assert_eq!(
        buildings["rows"][0]["fields"]["construction_stage"],
        "complete"
    );

    // Replaying the commit returns the original receipt, not a second effect.
    let replay = parsed(&fortress_commit(Some(session), digest))?;
    assert_eq!(replay["actions"], committed["actions"]);
    Ok(())
}

#[test]
fn paused_fortress_makes_no_progress_and_says_why() -> TestResult {
    let session = open("72002", true, &ALL_EFFECTS)?;
    let planned = parsed(&fortress_plan(
        Some(session.clone()),
        None,
        None,
        Some(
            r#"[{"action":{"kind":"designate_dig","min":[0,3,10],"max":[1,3,10],"mode":"mine"}}]"#
                .to_owned(),
        ),
    ))?;
    let digest = planned["plan_digest"].as_str().ok_or("digest")?.to_owned();
    let committed = parsed(&fortress_commit(Some(session.clone()), digest))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let waited = parsed(&fortress_wait(Some(session), Some(500)))?;
    assert_eq!(waited["advanced_game_ticks"], 0);
    assert!(waited["blocked"].is_string());
    assert_eq!(
        waited["polled_actions"][0]["state"],
        "AppliedAwaitingVerification"
    );
    let turn = &waited["agent_turn"];
    assert_eq!(
        turn["active_work"]["obligations"][0]["blocked_by_pause"],
        true
    );
    assert_eq!(turn["recommendations"][0]["tool"], "fortress.plan");
    assert_eq!(
        turn["recommendations"][0]["arguments"]["paused_target"],
        false
    );
    Ok(())
}

#[test]
fn effect_plans_need_their_own_negotiated_authority() -> TestResult {
    // Only labor authority: the dig step cannot be committed.
    let session = open("72003", false, &[("configure_labor", "reversible")])?;
    let planned = parsed(&fortress_plan(
        Some(session.clone()),
        None,
        None,
        Some(
            r#"[{"action":{"kind":"designate_dig","min":[0,3,10],"max":[1,3,10],"mode":"mine"}}]"#
                .to_owned(),
        ),
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    // The commit affordance is disabled because the plan needs `designate`.
    let commit_affordance = planned["agent_turn"]["affordances"]
        .as_array()
        .ok_or("affordances")?
        .iter()
        .find(|a| a["affordance_id"] == "commit-pending-plan")
        .cloned()
        .ok_or("no commit affordance")?;
    assert_eq!(commit_affordance["enabled"], false);
    let digest = planned["plan_digest"].as_str().ok_or("digest")?.to_owned();
    let denied = parsed(&fortress_commit(Some(session.clone()), digest))?;
    assert_eq!(denied["ok"], false, "{denied}");
    assert_eq!(denied["error"]["code"], "capability_denied");
    let terrain = parsed(&fortress_query(
        Some(session),
        Some(r#"{"mode":"terrain","min":[0,3,10],"max":[1,3,10]}"#.to_owned()),
    ))?;
    assert_eq!(terrain["levels"][0]["rows"][0], "##");
    Ok(())
}

#[test]
fn already_satisfied_and_malformed_requests_are_refused_before_sealing() -> TestResult {
    let session = open("72004", false, &ALL_EFFECTS)?;
    // The hall is already floor: an excavation there has nothing to do.
    let noop = parsed(&fortress_plan(
        Some(session.clone()),
        None,
        None,
        Some(
            r#"[{"action":{"kind":"designate_dig","min":[0,0,10],"max":[2,0,10],"mode":"mine"}}]"#
                .to_owned(),
        ),
    ))?;
    assert_eq!(noop["ok"], false, "{noop}");
    let malformed = parsed(&fortress_plan(
        Some(session),
        None,
        None,
        Some(r#"[{"action":{"kind":"designate_dig","min":[0,0,10]}}]"#.to_owned()),
    ))?;
    assert_eq!(malformed["ok"], false, "{malformed}");
    Ok(())
}

#[test]
fn plan_scope_cancellation_drains_dependents_and_certifies_quiescence() -> TestResult {
    let session = open("72005", false, &ALL_EFFECTS)?;
    let planned = parsed(&fortress_plan(
        Some(session.clone()),
        None,
        None,
        Some(WORKSHOP_PLAN.to_owned()),
    ))?;
    let digest = planned["plan_digest"].as_str().ok_or("digest")?.to_owned();
    let committed = parsed(&fortress_commit(Some(session.clone()), digest))?;
    assert_eq!(committed["ok"], true, "{committed}");
    // Dig part of the room (15 tiles at 10 ticks each), then change course.
    let waited = parsed(&fortress_wait(Some(session.clone()), Some(50)))?;
    assert_eq!(waited["ok"], true, "{waited}");

    let drained = parsed(&fortress_cancel(
        Some(session.clone()),
        Some("stop_future_steps".to_owned()),
        Some("plan".to_owned()),
    ))?;
    assert_eq!(drained["ok"], true, "{drained}");
    let progress = &drained["drain_progress"];
    assert_eq!(progress["actions_total"], 4);
    assert_eq!(progress["already_terminal"], 1); // the labor change verified at commit
    assert_eq!(progress["cancelled"], 3);
    assert_eq!(progress["remaining_nonterminal"], 0);
    assert_eq!(progress["quiescent"], true);
    assert!(drained["finalize_certificate"]["digest"].is_string());
    let after: Vec<&str> = drained["steps"]
        .as_array()
        .ok_or("steps")?
        .iter()
        .filter_map(|step| step["after"].as_str())
        .collect();
    assert_eq!(after, ["Cancelled", "Cancelled", "Cancelled", "Verified"]);
    assert_eq!(
        drained["agent_turn"]["active_work"]["obligations"],
        json!([])
    );

    // Excavated tiles stay excavated, nothing progresses any more, and the
    // dependent building was never created.
    let before_more_time = parsed(&fortress_query(
        Some(session.clone()),
        Some(r#"{"mode":"terrain","min":[0,3,10],"max":[4,5,10]}"#.to_owned()),
    ))?;
    assert_eq!(before_more_time["levels"][0]["rows"][0], ".....");
    assert_eq!(before_more_time["counts"]["floor"], 5);
    let later = parsed(&fortress_wait(Some(session.clone()), Some(500)))?;
    assert_eq!(later["ok"], true, "{later}");
    let after_more_time = parsed(&fortress_query(
        Some(session.clone()),
        Some(r#"{"mode":"terrain","min":[0,3,10],"max":[4,5,10]}"#.to_owned()),
    ))?;
    assert_eq!(after_more_time["counts"], before_more_time["counts"]);
    let buildings = parsed(&fortress_query(
        Some(session.clone()),
        Some(r#"{"mode":"entities","kind":"building"}"#.to_owned()),
    ))?;
    assert_eq!(buildings["total"], 0);

    // Draining an already quiescent plan is an idempotent no-op.
    let again = parsed(&fortress_cancel(
        Some(session),
        Some("stop_future_steps".to_owned()),
        Some("plan".to_owned()),
    ))?;
    assert_eq!(again["drain_progress"]["already_terminal"], 4);
    assert_eq!(again["drain_progress"]["quiescent"], true);
    Ok(())
}

#[test]
fn a_later_plan_does_not_strand_an_earlier_plans_deferred_steps() -> TestResult {
    let session = open("72006", false, &ALL_EFFECTS)?;
    let first = parsed(&fortress_plan(
        Some(session.clone()),
        None,
        None,
        Some(
            r#"[{"action":{"kind":"designate_dig","min":[0,3,10],"max":[2,3,10],"mode":"mine"}},
                {"action":{"kind":"build","building":"furniture:Bed","location":[1,3,10],"min":[1,3,10],"max":[1,3,10]},"depends_on":[0]}]"#
                .to_owned(),
        ),
    ))?;
    let building = first["steps"][1]["creates_entity_id"]
        .as_str()
        .ok_or("created entity")?
        .to_owned();
    let digest = first["plan_digest"].as_str().ok_or("digest")?.to_owned();
    assert_eq!(
        parsed(&fortress_commit(Some(session.clone()), digest))?["ok"],
        true
    );
    // A second, unrelated plan becomes the "last" plan.
    let second = parsed(&fortress_plan(
        Some(session.clone()),
        None,
        None,
        Some(
            r#"[{"action":{"kind":"set_labor","units":["1001"],"labor":"MINE","enabled":true}}]"#
                .to_owned(),
        ),
    ))?;
    let digest = second["plan_digest"].as_str().ok_or("digest")?.to_owned();
    let committed = parsed(&fortress_commit(Some(session.clone()), digest))?;
    assert_eq!(committed["ok"], true, "{committed}");
    // The first plan's obligations are still visible as active work.
    assert_eq!(
        committed["agent_turn"]["active_work"]["obligations"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
    let mut remaining = None;
    for _ in 0..20 {
        let waited = parsed(&fortress_wait(Some(session.clone()), Some(100)))?;
        remaining = waited["open_actions_remaining"].as_u64();
        if remaining == Some(0) {
            break;
        }
    }
    assert_eq!(remaining, Some(0));
    let buildings = parsed(&fortress_query(
        Some(session),
        Some(r#"{"mode":"entities","kind":"building"}"#.to_owned()),
    ))?;
    assert_eq!(buildings["rows"][0]["entity_id"], building);
    assert_eq!(
        buildings["rows"][0]["fields"]["construction_stage"],
        "complete"
    );
    Ok(())
}

#[test]
fn handoff_packet_lets_a_fresh_agent_resume_open_work_without_the_transcript() -> TestResult {
    let session = open("72007", false, &ALL_EFFECTS)?;
    let planned = parsed(&fortress_plan(
        Some(session.clone()),
        None,
        None,
        Some(WORKSHOP_PLAN.to_owned()),
    ))?;
    let digest = planned["plan_digest"].as_str().ok_or("digest")?.to_owned();
    let committed = parsed(&fortress_commit(Some(session.clone()), digest.clone()))?;
    assert_eq!(committed["ok"], true, "{committed}");
    // A new plan is left pending, too.
    let pending = parsed(&fortress_plan(
        Some(session.clone()),
        None,
        Some(true),
        None,
    ))?;
    assert_eq!(pending["ok"], true, "{pending}");

    let uri = format!("df://session/{session}/handoff");
    let contents =
        crate::resources::session_handoff(&session, &uri).map_err(|error| format!("{error:?}"))?;
    let packet: Value = serde_json::from_str(contents[0].text.as_deref().ok_or("text")?)?;
    assert_eq!(packet["schema"], "dfmcp.lab-handoff/1");
    assert_eq!(
        packet["pending_plan"]["plan_digest"],
        pending["plan_digest"]
    );
    assert_eq!(packet["open_actions"].as_array().map(Vec::len), Some(3));
    assert!(packet["open_actions"][0]["obligation"]["deadline_tick"].is_u64());
    assert_eq!(packet["committed_plan_digests"][0], digest);
    let tools: Vec<&str> = packet["resume_protocol"]
        .as_array()
        .ok_or("resume_protocol")?
        .iter()
        .filter_map(|step| step["tool"].as_str())
        .collect();
    assert_eq!(
        tools,
        ["fortress.observe", "fortress.commit", "fortress.wait"]
    );
    // Reading the packet never polls or dispatches: states are unchanged.
    let again =
        crate::resources::session_handoff(&session, &uri).map_err(|error| format!("{error:?}"))?;
    let again: Value = serde_json::from_str(again[0].text.as_deref().ok_or("text")?)?;
    assert_eq!(again["open_actions"], packet["open_actions"]);
    assert_eq!(again["anchor"], packet["anchor"]);
    Ok(())
}
