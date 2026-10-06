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

    let mut finished = false;
    for _ in 0..40 {
        let waited = parsed(&fortress_wait(Some(session.clone()), Some(100)))?;
        assert_eq!(waited["ok"], true, "{waited}");
        assert_eq!(waited["advanced_game_ticks"], 100);
        let states: Vec<&str> = waited["plan_actions"]
            .as_array()
            .ok_or("plan_actions")?
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
        waited["plan_actions"][0]["state"],
        "AppliedAwaitingVerification"
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
