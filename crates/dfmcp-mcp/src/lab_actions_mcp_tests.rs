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
        None,
        None,
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
        None,
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
    // The dig touches x=0, which has no complete halo for dig/1.16; the
    // still and the brewing order have no live family; the labor maps onto
    // workforce/1.17. The plan says all of this before anything is committed.
    let routing = &planned["live_routing"];
    assert_eq!(routing["fully_routable"], false, "{routing}");
    assert_eq!(routing["steps"][0]["routable"], false);
    assert!(
        routing["steps"][0]["reason"]
            .as_str()
            .is_some_and(|r| r.contains("dig/1.16"))
    );
    assert_eq!(routing["steps"][1]["routable"], false);
    assert_eq!(routing["steps"][2]["routable"], false);
    assert_eq!(routing["steps"][3]["protocol"], "workforce/1.17");
    assert_eq!(
        routing["steps"][3]["dev_server"],
        "dfmcp-live-workforce-dev-server"
    );
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

    let first = parsed(&fortress_wait(Some(session.clone()), Some(1)))?;
    assert_eq!(
        first["agent_turn"]["briefing"]["objective_status"][0]["status"],
        "not_yet_observed"
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
    // The goal itself, not just the dispatch, is observed achieved.
    let objectives = &settled["agent_turn"]["briefing"]["objective_status"];
    assert_eq!(objectives[0]["summary"], "brewery", "{objectives}");
    assert_eq!(objectives[0]["status"], "achieved");

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
        None,
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
        None,
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
        None,
    ))?;
    assert_eq!(noop["ok"], false, "{noop}");
    let malformed = parsed(&fortress_plan(
        Some(session),
        None,
        None,
        Some(r#"[{"action":{"kind":"designate_dig","min":[0,0,10]}}]"#.to_owned()),
        None,
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
        None,
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
        None,
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
        None,
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
        None,
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
        None,
    ))?;
    assert_eq!(pending["ok"], true, "{pending}");

    let uri = format!("df://session/{session}/handoff");
    let contents =
        crate::resources::session_handoff(&session, &uri).map_err(|error| format!("{error:?}"))?;
    let packet: Value = serde_json::from_str(contents[0].text.as_deref().ok_or("text")?)?;
    assert_eq!(packet["schema"], "dfmcp.lab-handoff/1");
    assert_eq!(packet["durability"]["durable"], false);
    assert_eq!(packet["world_alerts"], json!([]));
    assert_eq!(
        packet["orientation"]["replay_bundle"],
        format!("df://session/{session}/replay")
    );
    assert!(
        packet["orientation"]["changes_since_oldest_retained"]["arguments"]["mode"].is_string()
    );
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

#[test]
fn restore_retires_open_work_so_later_waits_and_commits_still_function() -> TestResult {
    let mut caps = ALL_EFFECTS.to_vec();
    caps.push(("restore", "guarded"));
    let session = open("72008", false, &caps)?;
    let checkpoint = parsed(&fortress_checkpoint(
        Some(session.clone()),
        Some("before-digging".to_owned()),
    ))?;
    assert_eq!(checkpoint["ok"], true, "{checkpoint}");
    let planned = parsed(&fortress_plan(
        Some(session.clone()),
        None,
        None,
        Some(WORKSHOP_PLAN.to_owned()),
        None,
    ))?;
    let digest = planned["plan_digest"].as_str().ok_or("digest")?.to_owned();
    assert_eq!(
        parsed(&fortress_commit(Some(session.clone()), digest))?["ok"],
        true
    );
    let restored = parsed(&fortress_restore(
        Some(session.clone()),
        checkpoint["checkpoint_id"]
            .as_str()
            .ok_or("checkpoint id")?
            .to_owned(),
    ))?;
    assert_eq!(restored["ok"], true, "{restored}");
    assert_eq!(
        restored["agent_turn"]["active_work"]["obligations"],
        json!([])
    );
    // New work after the restore runs normally.
    let planned = parsed(&fortress_plan(
        Some(session.clone()),
        None,
        None,
        Some(
            r#"[{"action":{"kind":"designate_dig","min":[0,3,10],"max":[0,3,10],"mode":"mine"}}]"#
                .to_owned(),
        ),
        None,
    ))?;
    let digest = planned["plan_digest"].as_str().ok_or("digest")?.to_owned();
    assert_eq!(
        parsed(&fortress_commit(Some(session.clone()), digest))?["ok"],
        true
    );
    let waited = parsed(&fortress_wait(Some(session), Some(20)))?;
    assert_eq!(waited["ok"], true, "{waited}");
    assert_eq!(waited["open_actions_remaining"], 0);
    Ok(())
}

fn open_shared(
    selector: &str,
    scenario: Option<&str>,
    extra: &[(&str, &str)],
) -> std::result::Result<Value, Box<dyn std::error::Error>> {
    parsed(&fortress_open_session(
        Some(false),
        Some(selector.to_owned()),
        Some(caps(extra)),
        None,
        Some(2_000),
        None,
        None,
        Some(8_192),
        None,
        scenario.map(str::to_owned),
        Some(true),
        None,
    ))
}

fn plan_and_commit(
    session: &str,
    actions: &str,
) -> std::result::Result<Value, Box<dyn std::error::Error>> {
    let planned = parsed(&fortress_plan(
        Some(session.to_owned()),
        None,
        None,
        Some(actions.to_owned()),
        None,
    ))?;
    if planned["ok"] != true {
        return Ok(planned);
    }
    let digest = planned["plan_digest"].as_str().ok_or("digest")?.to_owned();
    parsed(&fortress_commit(Some(session.to_owned()), digest))
}

fn dig(min: [i32; 3], max: [i32; 3]) -> String {
    format!(
        r#"[{{"action":{{"kind":"designate_dig","min":[{},{},{}],"max":[{},{},{}],"mode":"mine"}}}}]"#,
        min[0], min[1], min[2], max[0], max[1], max[2]
    )
}

#[test]
fn agents_sharing_a_fortress_lease_regions_and_see_one_world() -> TestResult {
    let mut caps_a = ALL_EFFECTS.to_vec();
    caps_a.push(("restore", "guarded"));
    let a = open_shared("73001", Some("starter_fortress"), &caps_a)?;
    assert_eq!(a["ok"], true, "{a}");
    assert_eq!(a["shared_world"]["joined_existing"], false);
    let b = open_shared("73001", None, &ALL_EFFECTS)?;
    assert_eq!(b["ok"], true, "{b}");
    assert_eq!(b["shared_world"]["joined_existing"], true);
    assert_eq!(b["shared_world"]["members"], 2);
    assert_eq!(b["scenario"], "starter_fortress");
    assert_eq!(b["anchor"], a["anchor"]);
    let mismatched = open_shared("73001", Some("empty"), &ALL_EFFECTS)?;
    assert_eq!(mismatched["ok"], false, "{mismatched}");
    let (a, b) = (
        a["session_id"].as_str().ok_or("a")?.to_owned(),
        b["session_id"].as_str().ok_or("b")?.to_owned(),
    );

    // A leases and starts excavating a room.
    let a_dig = plan_and_commit(&a, &dig([0, 3, 10], [4, 5, 10]))?;
    assert_eq!(a_dig["ok"], true, "{a_dig}");
    // B sees A's designation in the one shared world.
    let designations = parsed(&fortress_query(
        Some(b.clone()),
        Some(r#"{"mode":"entities","kind":"dig_designation"}"#.to_owned()),
    ))?;
    assert_eq!(designations["total"], 1);
    // B cannot dig into A's leased region, and nothing changes.
    let overlap = plan_and_commit(&b, &dig([4, 5, 10], [6, 6, 10]))?;
    assert_eq!(overlap["ok"], false, "{overlap}");
    assert_eq!(overlap["error"]["code"], "conflict");
    assert!(
        overlap["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("lease")),
        "{overlap}"
    );
    // A disjoint region is fine.
    let b_dig = plan_and_commit(&b, &dig([6, 3, 10], [8, 3, 10]))?;
    assert_eq!(b_dig["ok"], true, "{b_dig}");

    // One clock: A's waits advance B's work too.
    for _ in 0..10 {
        let waited = parsed(&fortress_wait(Some(a.clone()), Some(100)))?;
        if waited["open_actions_remaining"] == 0 {
            break;
        }
    }
    let terrain = parsed(&fortress_query(
        Some(b.clone()),
        Some(r#"{"mode":"terrain","min":[0,3,10],"max":[8,3,10]}"#.to_owned()),
    ))?;
    assert_eq!(terrain["levels"][0]["rows"][0], ".....#...");
    let b_wait = parsed(&fortress_wait(Some(b.clone()), Some(10)))?;
    assert_eq!(b_wait["open_actions_remaining"], 0, "{b_wait}");

    // A's lease was released when its excavation verified: B may now work there.
    let reuse = plan_and_commit(&b, &dig([4, 6, 10], [4, 7, 10]))?;
    assert_eq!(reuse["ok"], true, "{reuse}");

    // Restore would rewrite B's world too, so A may not restore.
    let checkpoint = parsed(&fortress_checkpoint(Some(a.clone()), Some("x".to_owned())))?;
    let refused = parsed(&fortress_restore(
        Some(a),
        checkpoint["checkpoint_id"]
            .as_str()
            .ok_or("checkpoint")?
            .to_owned(),
    ))?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "conflict");
    Ok(())
}

#[test]
fn a_plan_made_stale_by_another_agent_is_replayed_not_committed_blind() -> TestResult {
    let a = open_shared("73002", Some("starter_fortress"), &ALL_EFFECTS)?;
    let b = open_shared("73002", None, &ALL_EFFECTS)?;
    let (a, b) = (
        a["session_id"].as_str().ok_or("a")?.to_owned(),
        b["session_id"].as_str().ok_or("b")?.to_owned(),
    );
    let planned = parsed(&fortress_plan(
        Some(a.clone()),
        None,
        None,
        Some(
            r#"[{"action":{"kind":"set_labor","units":["1001"],"labor":"MINE","enabled":true}}]"#
                .to_owned(),
        ),
        None,
    ))?;
    let stale_digest = planned["plan_digest"].as_str().ok_or("digest")?.to_owned();
    // B acts first, moving the shared anchor.
    let b_dig = plan_and_commit(&b, &dig([0, 3, 10], [0, 3, 10]))?;
    assert_eq!(b_dig["ok"], true, "{b_dig}");

    let stale = parsed(&fortress_commit(Some(a.clone()), stale_digest.clone()))?;
    assert_eq!(stale["ok"], false, "{stale}");
    assert_eq!(stale["error"]["code"], "stale_anchor");
    assert_eq!(stale["rebase"]["method"], "intent_replay");
    assert_eq!(stale["rebase"]["from_digest"], stale_digest);
    let rebased = stale["rebased_plan"]["plan_digest"]
        .as_str()
        .ok_or("rebased digest")?
        .to_owned();
    assert_ne!(rebased, stale_digest);
    assert_eq!(
        stale["agent_turn"]["recommendations"][0]["tool"],
        "fortress.commit"
    );
    assert_eq!(
        stale["agent_turn"]["recommendations"][0]["arguments"]["plan_digest"],
        rebased
    );
    assert_eq!(
        stale["agent_turn"]["active_work"]["pending_plans"][0]["plan_digest"],
        rebased
    );
    let committed = parsed(&fortress_commit(Some(a.clone()), rebased))?;
    assert_eq!(committed["ok"], true, "{committed}");
    assert_eq!(committed["actions"][0]["state"], "Verified");
    // The stale digest can never be committed later.
    let again = parsed(&fortress_commit(Some(a), stale_digest))?;
    assert_eq!(again["ok"], false, "{again}");
    Ok(())
}

#[test]
fn plan_forecasts_predict_completion_blocking_and_doomed_steps() -> TestResult {
    let session = open("72009", false, &ALL_EFFECTS)?;
    let planned = parsed(&fortress_plan(
        Some(session.clone()),
        None,
        None,
        Some(WORKSHOP_PLAN.to_owned()),
        None,
    ))?;
    let forecast = &planned["forecast"];
    assert_eq!(forecast["epistemic_state"], "predicted");
    assert_eq!(forecast["available"], true, "{forecast}");
    assert_eq!(forecast["predicted_complete"], true, "{forecast}");
    let predicted = forecast["predicted_completion_tick"]
        .as_u64()
        .ok_or("predicted completion tick")?;
    // Forecasting is side-effect free: nothing was dispatched.
    let designations = parsed(&fortress_query(
        Some(session.clone()),
        Some(r#"{"mode":"entities","kind":"dig_designation"}"#.to_owned()),
    ))?;
    assert_eq!(designations["total"], 0);

    // Reality (in 50-tick waits) agrees with the prediction to within a wait.
    let digest = planned["plan_digest"].as_str().ok_or("digest")?.to_owned();
    assert_eq!(
        parsed(&fortress_commit(Some(session.clone()), digest))?["ok"],
        true
    );
    let mut actual = None;
    for _ in 0..80 {
        let waited = parsed(&fortress_wait(Some(session.clone()), Some(50)))?;
        if waited["open_actions_remaining"] == 0 {
            actual = waited["game_tick"].as_u64();
            break;
        }
    }
    let actual = actual.ok_or("plan never completed")?;
    let resolution = forecast["resolution_ticks"].as_u64().ok_or("resolution")?;
    assert!(
        actual.abs_diff(predicted) <= 50 + resolution,
        "predicted {predicted} (resolution {resolution}), observed {actual}"
    );

    // A paused fortress: the forecast says the work is blocked.
    let paused = open("72010", true, &ALL_EFFECTS)?;
    let blocked = parsed(&fortress_plan(
        Some(paused),
        None,
        None,
        Some(dig([0, 3, 10], [1, 3, 10])),
        None,
    ))?;
    assert_eq!(blocked["forecast"]["blocked_by_pause"], true, "{blocked}");
    assert_eq!(blocked["forecast"]["predicted_complete"], false);

    // A step that would fail at commit is visible before committing.
    let doomed = parsed(&fortress_plan(
        Some(session),
        None,
        None,
        Some(dig([0, 3, 11], [0, 3, 12])),
        None,
    ))?;
    assert_eq!(doomed["ok"], true, "{doomed}");
    assert_eq!(doomed["forecast"]["available"], false);
    assert_eq!(doomed["forecast"]["reason"]["code"], "preconditions_failed");
    Ok(())
}

#[test]
fn an_observation_that_contradicts_the_forecast_emits_a_surprise_record() -> TestResult {
    // Planned while paused: the forecast predicts the dig cannot finish.
    let session = open("72011", true, &ALL_EFFECTS)?;
    let planned = parsed(&fortress_plan(
        Some(session.clone()),
        None,
        None,
        Some(dig([0, 3, 10], [1, 3, 10])),
        None,
    ))?;
    assert_eq!(planned["forecast"]["predicted_complete"], false);
    let digest = planned["plan_digest"].as_str().ok_or("digest")?.to_owned();
    assert_eq!(
        parsed(&fortress_commit(Some(session.clone()), digest))?["ok"],
        true
    );
    // The agent then changes the world the forecast assumed: it unpauses.
    let resume = parsed(&fortress_plan(
        Some(session.clone()),
        None,
        Some(false),
        None,
        None,
    ))?;
    let digest = resume["plan_digest"].as_str().ok_or("digest")?.to_owned();
    assert_eq!(
        parsed(&fortress_commit(Some(session.clone()), digest))?["ok"],
        true
    );
    let mut surprise = Value::Null;
    for _ in 0..5 {
        let waited = parsed(&fortress_wait(Some(session.clone()), Some(50)))?;
        if let Some(found) = waited["agent_turn"]["attention"]
            .as_array()
            .and_then(|items| items.iter().find(|item| item["category"] == "surprise"))
        {
            surprise = found["surprise"].clone();
            break;
        }
    }
    assert_eq!(surprise["kind"], "outcome_diverged", "{surprise}");
    assert_eq!(surprise["observed"]["state"], "Verified");
    assert_eq!(surprise["epistemic_state"], "observed");
    Ok(())
}

#[test]
fn a_stale_commit_is_recorded_as_a_surprise() -> TestResult {
    let a = open_shared("73003", Some("starter_fortress"), &ALL_EFFECTS)?;
    let b = open_shared("73003", None, &ALL_EFFECTS)?;
    let (a, b) = (
        a["session_id"].as_str().ok_or("a")?.to_owned(),
        b["session_id"].as_str().ok_or("b")?.to_owned(),
    );
    let planned = parsed(&fortress_plan(
        Some(a.clone()),
        None,
        None,
        Some(dig([0, 3, 10], [0, 3, 10])),
        None,
    ))?;
    let digest = planned["plan_digest"].as_str().ok_or("digest")?.to_owned();
    assert_eq!(
        plan_and_commit(&b, &dig([6, 3, 10], [6, 3, 10]))?["ok"],
        true
    );
    let stale = parsed(&fortress_commit(Some(a), digest))?;
    assert_eq!(stale["error"]["code"], "stale_anchor");
    assert!(stale["rebased_plan"]["forecast"]["available"].is_boolean());
    let surprise = stale["agent_turn"]["attention"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["category"] == "surprise"))
        .cloned()
        .ok_or("no surprise attention")?;
    assert_eq!(surprise["surprise"]["kind"], "anchor_moved_before_commit");
    Ok(())
}

#[test]
fn observe_briefs_the_starter_fortress_under_the_default_budget() -> TestResult {
    let opened = parsed(&fortress_open_session(
        Some(true),
        Some("72012".to_owned()),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some("starter_fortress".to_owned()),
        None,
        None,
    ))?;
    let session = opened["session_id"].as_str().ok_or("session")?.to_owned();
    let observed = parsed(&fortress_observe(Some(session)))?;
    assert_eq!(observed["ok"], true, "{observed}");
    assert_eq!(observed["world"]["counts_by_kind"]["unit"], 7);
    Ok(())
}

fn commit_pause(
    session: &str,
    paused: bool,
) -> std::result::Result<Value, Box<dyn std::error::Error>> {
    let planned = parsed(&fortress_plan(
        Some(session.to_owned()),
        None,
        Some(paused),
        None,
        None,
    ))?;
    if planned["ok"] != true {
        return Ok(planned);
    }
    let digest = planned["plan_digest"].as_str().ok_or("digest")?.to_owned();
    parsed(&fortress_commit(Some(session.to_owned()), digest))
}

#[test]
fn any_member_can_pause_a_shared_fortress_but_unpausing_needs_everyone() -> TestResult {
    let opened = parsed(&fortress_open_session(
        Some(true),
        Some("73004".to_owned()),
        Some(caps(&ALL_EFFECTS)),
        None,
        Some(2_000),
        None,
        None,
        Some(8_192),
        None,
        Some("starter_fortress".to_owned()),
        Some(true),
        None,
    ))?;
    assert_eq!(opened["paused"], true);
    let a = opened["session_id"].as_str().ok_or("a")?.to_owned();
    let b = open_shared("73004", None, &ALL_EFFECTS)?;
    let b = b["session_id"].as_str().ok_or("b")?.to_owned();

    // A alone cannot unpause: its consent is recorded, nothing is dispatched.
    let first = commit_pause(&a, false)?;
    assert_eq!(first["ok"], false, "{first}");
    assert_eq!(first["clock_consent"]["votes"], 1);
    assert_eq!(first["clock_consent"]["members"], 2);
    assert_eq!(
        first["agent_turn"]["recommendations"][0]["recommendation_id"],
        "await-unpause-consent"
    );
    assert_eq!(parsed(&fortress_observe(Some(a.clone())))?["paused"], true);
    // B's consent completes the vote and the shared fortress unpauses.
    let second = commit_pause(&b, false)?;
    assert_eq!(second["ok"], true, "{second}");
    assert_eq!(parsed(&fortress_observe(Some(a.clone())))?["paused"], false);
    // Either member may pull the emergency brake at once...
    assert_eq!(commit_pause(&a, true)?["ok"], true);
    assert_eq!(parsed(&fortress_observe(Some(b.clone())))?["paused"], true);
    // ...which clears consensus: B's renewed wish to unpause is not enough.
    let again = commit_pause(&b, false)?;
    assert_eq!(again["ok"], false, "{again}");
    assert_eq!(again["clock_consent"]["votes"], 1);
    Ok(())
}

#[test]
fn a_blueprint_objective_is_decomposed_dug_and_furnished() -> TestResult {
    let session = open("72101", false, &ALL_EFFECTS)?;
    let planned = parsed(&fortress_plan(
        Some(session.clone()),
        None,
        None,
        None,
        Some(
            r#"{"template":"bedroom_cluster","origin":[3,5,10],"rooms":4,"room_size":[3,3]}"#
                .to_owned(),
        ),
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let steps = planned["steps"].as_array().ok_or("steps")?;
    let beds: Vec<&Value> = steps
        .iter()
        .filter(|step| step["kind"] == "build")
        .collect();
    assert_eq!(beds.len(), 4, "{planned}");
    // Every bed waits for its room's excavation; the planner, not the agent,
    // derived those dependencies and their obligations.
    for bed in &beds {
        assert_eq!(bed["depends_on"].as_array().map(Vec::len), Some(1), "{bed}");
        assert!(bed["obligation"]["deadline_tick"].is_u64(), "{bed}");
    }
    assert_eq!(planned["forecast"]["available"], true, "{planned}");

    // Asking for both an objective and explicit actions is ambiguous.
    let both = parsed(&fortress_plan(
        Some(session.clone()),
        None,
        None,
        Some(dig([0, 3, 10], [0, 3, 10])),
        Some(r#"{"template":"dining_hall","origin":[3,5,10],"width":3,"height":3}"#.to_owned()),
    ))?;
    assert_eq!(both["ok"], false, "{both}");

    let digest = planned["plan_digest"].as_str().ok_or("digest")?.to_owned();
    let committed = parsed(&fortress_commit(Some(session.clone()), digest))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let mut settled = false;
    for _ in 0..19 {
        let waited = parsed(&fortress_wait(Some(session.clone()), Some(100)))?;
        assert_eq!(waited["ok"], true, "{waited}");
        if waited["open_actions_remaining"] == 0 {
            settled = true;
            break;
        }
    }
    assert!(settled, "blueprint work never settled");
    let buildings = parsed(&fortress_query(
        Some(session.clone()),
        Some(r#"{"mode":"entities","kind":"building"}"#.to_owned()),
    ))?;
    assert_eq!(buildings["total"], 4, "{buildings}");
    for row in buildings["rows"].as_array().ok_or("rows")? {
        assert_eq!(row["fields"]["construction_stage"], "complete", "{row}");
    }
    // The first bedroom is open floor now.
    let room = parsed(&fortress_query(
        Some(session),
        Some(r#"{"mode":"terrain","min":[3,5,10],"max":[5,7,10]}"#.to_owned()),
    ))?;
    assert_eq!(room["levels"][0]["rows"], json!(["...", "...", "..."]));
    Ok(())
}

#[test]
fn every_turn_reports_what_changed_in_the_world() -> TestResult {
    let session = open("72077", false, &ALL_EFFECTS)?;
    let planned = parsed(&fortress_plan(
        Some(session.clone()),
        None,
        None,
        Some(
            r#"[{"action":{"kind":"designate_dig","min":[2,3,10],"max":[4,4,10],"mode":"mine"}},
                {"action":{"kind":"set_labor","units":["1002"],"labor":"MINE","enabled":true}}]"#
                .to_owned(),
        ),
        None,
    ))?;
    let digest = planned["plan_digest"].as_str().ok_or("digest")?.to_owned();
    // Planning changes nothing in the world.
    let kinds = |turn: &Value| -> Vec<String> {
        turn["agent_turn"]["changes"]
            .as_array()
            .map(|changes| {
                changes
                    .iter()
                    .filter_map(|c| c["kind"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    assert_eq!(kinds(&planned), ["plan_prepared"]);

    let committed = parsed(&fortress_commit(Some(session.clone()), digest))?;
    let changes = committed["agent_turn"]["changes"]
        .as_array()
        .ok_or("changes")?
        .clone();
    let created = changes
        .iter()
        .find(|c| c["kind"] == "entity_created")
        .ok_or("no entity_created")?;
    assert_eq!(created["epistemic_state"], "observed");
    let dwarf = changes
        .iter()
        .find(|c| c["kind"] == "entity_changed" && c["subject"]["entity_id"] == "1002")
        .ok_or("labor change not reported")?;
    assert_eq!(dwarf["fields"][0]["field"], "labor.MINE");
    assert_eq!(dwarf["fields"][0]["after"], true);

    let waited = parsed(&fortress_wait(Some(session.clone()), Some(100)))?;
    let changes = waited["agent_turn"]["changes"]
        .as_array()
        .ok_or("changes")?
        .clone();
    let time = changes
        .iter()
        .find(|c| c["kind"] == "game_time_passed")
        .ok_or("no clock change")?;
    assert_eq!(
        time["subject"]["to_tick"].as_u64(),
        time["subject"]["from_tick"].as_u64().map(|t| t + 100)
    );
    let terrain = changes
        .iter()
        .find(|c| c["kind"] == "terrain_changed")
        .ok_or("no terrain change")?;
    assert_eq!(terrain["subject"]["z"], 10);
    assert_eq!(terrain["tiles_changed"], 6);
    assert_eq!(terrain["transitions"]["wall->floor"], 6);
    assert_eq!(terrain["bounding_box"]["min"], json!([2, 3, 10]));
    assert_eq!(terrain["bounding_box"]["max"], json!([4, 4, 10]));

    // Nothing changed since: an observe reports no world changes.
    let observed = parsed(&fortress_observe(Some(session.clone())))?;
    assert!(kinds(&observed).is_empty(), "{observed}");

    // Any retained version can be read exactly, and diffed against now.
    let before = committed["agent_turn"]["anchor"]["state_hash"]
        .as_str()
        .ok_or("anchor")?
        .to_owned();
    let past = parsed(&fortress_query(
        Some(session.clone()),
        Some(format!(
            r#"{{"mode":"terrain","min":[2,3,10],"max":[4,4,10],"at":"{before}"}}"#
        )),
    ))?;
    assert_eq!(past["ok"], true, "{past}");
    assert_eq!(past["historical"], true);
    assert_eq!(past["levels"][0]["rows"][0], "###");
    let since = parsed(&fortress_query(
        Some(session.clone()),
        Some(format!(r#"{{"mode":"changes","since":"{before}"}}"#)),
    ))?;
    assert_eq!(since["ok"], true, "{since}");
    assert!(
        since["changes"]
            .as_array()
            .is_some_and(|c| c.iter().any(|c| c["kind"] == "terrain_changed"))
    );
    let unknown = parsed(&fortress_query(
        Some(session),
        Some(format!(
            r#"{{"mode":"changes","since":"{}"}}"#,
            "ab".repeat(32)
        )),
    ))?;
    assert_eq!(unknown["ok"], false);
    Ok(())
}

#[test]
fn a_fortress_runs_dry_and_the_agent_brews_its_way_back() -> TestResult {
    let session = open("72088", false, &ALL_EFFECTS)?;
    let alert_of = |turn: &Value, id: &str| -> Option<Value> {
        turn["agent_turn"]["attention"]
            .as_array()?
            .iter()
            .find(|a| a["attention_id"] == id)
            .cloned()
    };
    let stock = |session: &str| -> std::result::Result<u64, Box<dyn std::error::Error>> {
        let rows = parsed(&fortress_query(
            Some(session.to_owned()),
            Some(r#"{"mode":"entities","kind":"stock_ledger"}"#.to_owned()),
        ))?;
        rows["rows"][0]["fields"]["stock.drink"]
            .as_u64()
            .ok_or_else(|| format!("no drink stock in {rows}").into())
    };
    assert_eq!(stock(&session)?, 40);
    let opening = parsed(&fortress_observe(Some(session.clone())))?;
    assert!(
        alert_of(&opening, "world-drink_supply").is_none(),
        "{opening}"
    );

    // Seven dwarves drink seven units every 1,200 ticks: 40 units last five
    // rounds, and the warning comes while there is still time to act.
    let mut warned = None;
    for _ in 0..60 {
        let waited = parsed(&fortress_wait(Some(session.clone()), Some(100)))?;
        if let Some(alert) = alert_of(&waited, "world-drink_supply") {
            warned = Some(alert);
            break;
        }
    }
    let warned = warned.ok_or("never warned about drink")?;
    assert_eq!(warned["severity"], "high", "{warned}");
    assert_eq!(warned["category"], "fortress_needs");
    assert_eq!(warned["remedy"]["tool"], "fortress.plan");

    // Ignore it until the barrels are empty and dwarves go thirsty.
    let mut thirsty = None;
    for _ in 0..60 {
        let waited = parsed(&fortress_wait(Some(session.clone()), Some(200)))?;
        if let Some(alert) = alert_of(&waited, "world-drink_supply")
            && alert["severity"] == "critical"
        {
            thirsty = Some(waited);
            break;
        }
    }
    let thirsty = thirsty.ok_or("never ran dry")?;
    assert_eq!(
        thirsty["agent_turn"]["recommendations"][0]["recommendation_id"], "remedy-drink_supply",
        "{thirsty}"
    );
    assert_eq!(stock(&session)?, 0, "{thirsty}");
    let units = parsed(&fortress_query(
        Some(session.clone()),
        Some(r#"{"mode":"entities","kind":"unit"}"#.to_owned()),
    ))?;
    assert!(
        units["rows"]
            .as_array()
            .is_some_and(|rows| rows.iter().any(|u| u["fields"]["need.drink"] == "thirsty")),
        "{units}"
    );

    // Follow the remedy: a brewing work order restocks and quenches.
    let remedy = alert_of(&thirsty, "world-drink_supply")
        .ok_or("alert vanished")?["remedy"]["arguments"]["actions"]
        .as_str()
        .ok_or("remedy actions")?
        .to_owned();
    let planned = parsed(&fortress_plan(
        Some(session.clone()),
        None,
        None,
        Some(remedy),
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let digest = planned["plan_digest"].as_str().ok_or("digest")?.to_owned();
    let committed = parsed(&fortress_commit(Some(session.clone()), digest))?;
    assert_eq!(committed["ok"], true, "{committed}");
    for _ in 0..20 {
        parsed(&fortress_wait(Some(session.clone()), Some(100)))?;
    }
    assert!(stock(&session)? > 0);
    let units = parsed(&fortress_query(
        Some(session.clone()),
        Some(r#"{"mode":"entities","kind":"unit"}"#.to_owned()),
    ))?;
    assert!(
        units["rows"].as_array().is_some_and(|rows| rows
            .iter()
            .all(|u| u["fields"]["need.drink"] == "satisfied")),
        "{units}"
    );
    Ok(())
}

fn open_besieged(selector: &str) -> std::result::Result<String, Box<dyn std::error::Error>> {
    let opened = parsed(&fortress_open_session(
        Some(false),
        Some(selector.to_owned()),
        Some(caps(&[
            ("configure_military", "guarded"),
            ("designate", "guarded"),
        ])),
        None,
        Some(20_000),
        None,
        None,
        Some(8_192),
        None,
        Some("besieged_fortress".to_owned()),
        None,
        None,
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    Ok(opened["session_id"]
        .as_str()
        .ok_or("session_id missing")?
        .to_owned())
}

fn dead_dwarves(session: &str) -> std::result::Result<usize, Box<dyn std::error::Error>> {
    let units = parsed(&fortress_query(
        Some(session.to_owned()),
        Some(r#"{"mode":"entities","kind":"unit"}"#.to_owned()),
    ))?;
    Ok(units["rows"]
        .as_array()
        .ok_or("rows")?
        .iter()
        .filter(|u| u["fields"]["alive"] == false)
        .count())
}

#[test]
fn an_ignored_raider_kills_and_a_mustered_squad_slays_it() -> TestResult {
    // Ignored: the raider arrives at tick 1,500 and kills exposed dwarves.
    let ignored = open_besieged("72101")?;
    let warned = parsed(&fortress_observe(Some(ignored.clone())))?;
    let alert = warned["world_alerts"][0].clone();
    assert_eq!(alert["alert"], "hostile", "{warned}");
    assert_eq!(alert["severity"], "high");
    for _ in 0..20 {
        parsed(&fortress_wait(Some(ignored.clone()), Some(100)))?;
    }
    assert!(dead_dwarves(&ignored)? >= 1);

    // Mustered: follow the remedy as soon as the raider attacks.
    let ready = open_besieged("72102")?;
    let mut attacked = None;
    for _ in 0..20 {
        let waited = parsed(&fortress_wait(Some(ready.clone()), Some(100)))?;
        if waited["world_alerts"]
            .as_array()
            .is_some_and(|alerts| alerts.iter().any(|a| a["severity"] == "critical"))
        {
            attacked = Some(waited);
            break;
        }
    }
    let attacked = attacked.ok_or("raider never attacked")?;
    let top = &attacked["agent_turn"]["recommendations"][0];
    assert_eq!(top["recommendation_id"], "remedy-hostile", "{attacked}");
    let actions = top["arguments"]["actions"]
        .as_str()
        .ok_or("remedy actions")?
        .to_owned();
    let planned = parsed(&fortress_plan(
        Some(ready.clone()),
        None,
        None,
        Some(actions),
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let digest = planned["plan_digest"].as_str().ok_or("digest")?.to_owned();
    let committed = parsed(&fortress_commit(Some(ready.clone()), digest))?;
    assert_eq!(committed["ok"], true, "{committed}");
    for _ in 0..5 {
        parsed(&fortress_wait(Some(ready.clone()), Some(100)))?;
    }
    let raider = parsed(&fortress_query(
        Some(ready.clone()),
        Some(r#"{"mode":"entities","kind":"creature"}"#.to_owned()),
    ))?;
    assert_eq!(
        raider["rows"][0]["fields"]["threat_status"], "slain",
        "{raider}"
    );
    assert_eq!(dead_dwarves(&ready)?, 0);
    let after = parsed(&fortress_observe(Some(ready)))?;
    assert!(
        !after["world_alerts"]
            .as_array()
            .is_some_and(|alerts| alerts.iter().any(|a| a["alert"] == "hostile")),
        "{after}"
    );
    Ok(())
}
