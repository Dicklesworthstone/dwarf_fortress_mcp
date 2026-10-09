//! Crash-durable laboratory fortresses through the eleven tools: work and
//! checkpoints survive a simulated server restart, the resumed world enters a
//! new observation epoch, and the pre-restart session is fenced.
use super::*;

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn parsed(raw: &str) -> std::result::Result<Value, Box<dyn std::error::Error>> {
    Ok(serde_json::from_str(raw)?)
}

fn caps() -> Vec<(String, String)> {
    [
        ("observe", "read_only"),
        ("query", "read_only"),
        ("plan", "reversible"),
        ("control_clock", "reversible"),
        ("checkpoint", "guarded"),
        ("restore", "guarded"),
        ("doctor", "read_only"),
        ("designate", "guarded"),
    ]
    .iter()
    .map(|(c, r)| ((*c).to_owned(), (*r).to_owned()))
    .collect()
}

fn open_durable(
    selector: &str,
    scenario: Option<&str>,
) -> std::result::Result<Value, Box<dyn std::error::Error>> {
    parsed(&fortress_open_session(
        Some(false),
        Some(selector.to_owned()),
        Some(caps()),
        None,
        Some(100_000),
        None,
        None,
        Some(8_192),
        None,
        scenario.map(str::to_owned),
        None,
        Some(true),
    ))
}

fn terrain(session: &str) -> std::result::Result<Value, Box<dyn std::error::Error>> {
    let rows = parsed(&fortress_query(
        Some(session.to_owned()),
        Some(r#"{"mode":"terrain","min":[0,3,10],"max":[7,5,10]}"#.to_owned()),
    ))?;
    assert_eq!(rows["ok"], true, "{rows}");
    Ok(rows["levels"][0]["rows"].clone())
}

fn id(value: &Value, field: &str) -> std::result::Result<String, Box<dyn std::error::Error>> {
    Ok(value[field]
        .as_str()
        .ok_or_else(|| format!("{field} missing in {value}"))?
        .to_owned())
}

/// Durable tests share the process-wide store; run them one at a time.
static DURABLE_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serialized() -> std::sync::MutexGuard<'static, ()> {
    match DURABLE_TESTS.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

struct StateDir(std::path::PathBuf);

impl Drop for StateDir {
    fn drop(&mut self) {
        crate::server::simulate_durable_restart(None);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn durable_fortress_survives_restart_with_work_and_checkpoints() -> TestResult {
    let _serial = serialized();
    let dir = StateDir(
        std::env::temp_dir().join(format!("dfmcp-durable-lab-mcp-{}", std::process::id())),
    );
    let _ = std::fs::remove_dir_all(&dir.0);
    crate::server::simulate_durable_restart(Some(dir.0.clone()));

    // A fresh durable fortress.
    let opened = open_durable("880011", Some("starter_fortress"))?;
    assert_eq!(opened["ok"], true, "{opened}");
    assert_eq!(opened["durable"]["resumed"], false);
    let first = id(&opened, "session_id")?;

    // Designate a long excavation and let only part of it happen.
    let planned = parsed(&fortress_plan(
        Some(first.clone()),
        Some("long hall".to_owned()),
        None,
        Some(
            r#"[{"action":{"kind":"designate_dig","min":[0,3,10],"max":[7,5,10],"mode":"mine"}},
                {"action":{"kind":"designate_dig","min":[0,6,10],"max":[1,6,10],"mode":"mine"},"depends_on":[0]}]"#
                .to_owned(),
        ),
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let digest = id(&planned, "plan_digest")?;
    let committed = parsed(&fortress_commit(Some(first.clone()), digest))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let waited = parsed(&fortress_wait(Some(first.clone()), Some(20)))?;
    assert_eq!(waited["ok"], true, "{waited}");
    let checkpoint = parsed(&fortress_checkpoint(
        Some(first.clone()),
        Some("early dig".to_owned()),
    ))?;
    assert_eq!(checkpoint["ok"], true, "{checkpoint}");
    assert_eq!(checkpoint["durable"], true, "{checkpoint}");
    let checkpoint_id = id(&checkpoint, "checkpoint_id")?;
    let at_checkpoint = terrain(&first)?;
    let waited = parsed(&fortress_wait(Some(first.clone()), Some(40)))?;
    assert_eq!(waited["ok"], true, "{waited}");
    let before_crash = terrain(&first)?;
    assert_ne!(before_crash, at_checkpoint, "excavation should progress");
    let doctor = parsed(&fortress_doctor(Some(first.clone())))?;
    assert_eq!(doctor["durability"]["durable"], true, "{doctor}");
    assert_eq!(
        doctor["durability"]["persisted_is_current"], true,
        "{doctor}"
    );
    let crash_anchor = doctor["current_anchor"].clone();

    // The process dies; a new one opens the same store.
    crate::server::simulate_durable_restart(Some(dir.0.clone()));

    // A different scenario cannot silently replace the durable world.
    let wrong = open_durable("880011", Some("empty"))?;
    assert_eq!(wrong["ok"], false, "{wrong}");

    let resumed = open_durable("880011", None)?;
    assert_eq!(resumed["ok"], true, "{resumed}");
    assert_eq!(resumed["durable"]["resumed"], true, "{resumed}");
    assert_eq!(resumed["scenario"], "starter_fortress");
    assert_eq!(resumed["durable"]["restorable_checkpoints"], 1);
    assert_eq!(
        resumed["durable"]["recovered_from_anchor"]["state_hash"],
        crash_anchor["state_hash"]
    );
    let new_epoch = resumed["anchor"]["epoch"].as_u64().ok_or("epoch")?;
    let old_epoch = crash_anchor["epoch"].as_u64().ok_or("old epoch")?;
    assert_eq!(new_epoch, old_epoch + 1, "{resumed}");
    let second = id(&resumed, "session_id")?;

    // The commit made before the crash was recompiled from its recorded
    // request against the world it was sealed on, reproducing its digest:
    // the dispatched excavation is carried, the deferred step never ran.
    let commits = &resumed["durable"]["recovered_commits"];
    assert_eq!(commits.as_array().map(Vec::len), Some(1), "{resumed}");
    assert_eq!(commits[0]["plan_digest"], planned["plan_digest"]);
    assert_eq!(commits[0]["status"], "carried");
    assert_eq!(commits[0]["steps"][0]["state"], "dispatched");
    assert_eq!(commits[0]["steps"][1]["state"], "not_dispatched");
    let carried = &resumed["durable"]["carried_obligations"];
    assert_eq!(carried.as_array().map(Vec::len), Some(1), "{resumed}");
    assert_eq!(carried[0]["action"], "designate_dig");

    // The world is exactly what was persisted.
    assert_eq!(terrain(&second)?, before_crash);

    // The pre-restart session is fenced rather than allowed to interleave.
    let fenced = parsed(&fortress_observe(Some(first.clone())))?;
    assert_eq!(fenced["ok"], false, "{fenced}");
    assert_eq!(fenced["error"]["code"], "conflict", "{fenced}");

    // The designation lives in the world, so excavation continues after
    // the restart even though no action handle was carried over.
    for _ in 0..20 {
        let waited = parsed(&fortress_wait(Some(second.clone()), Some(25)))?;
        assert_eq!(waited["ok"], true, "{waited}");
    }
    // The carried obligation was proven by observation and retired.
    let doctor = parsed(&fortress_doctor(Some(second.clone())))?;
    assert_eq!(
        doctor["durability"]["carried_obligations"][0]["state"], "verified",
        "{doctor}"
    );
    let finished = terrain(&second)?;
    assert_ne!(finished, before_crash);
    assert_eq!(finished[0], "........", "{finished}");

    // A checkpoint taken before the crash restores in the new process.
    let restored = parsed(&fortress_restore(Some(second.clone()), checkpoint_id))?;
    assert_eq!(restored["ok"], true, "{restored}");
    assert_eq!(terrain(&second)?, at_checkpoint);
    assert_eq!(restored["untracked_work"]["quiescent"], false, "{restored}");
    assert_eq!(
        restored["untracked_work"]["items"][0]["work_state"]["state"],
        "active"
    );

    // And the restore itself is durable.
    crate::server::simulate_durable_restart(Some(dir.0.clone()));
    let third = open_durable("880011", None)?;
    assert_eq!(third["ok"], true, "{third}");
    assert_eq!(
        third["durable"]["recovered_commits"]
            .as_array()
            .map(Vec::len),
        Some(0),
        "finished and abandoned commits are retired: {third}"
    );
    assert_eq!(third["untracked_work"]["quiescent"], false, "{third}");
    assert_eq!(
        third["untracked_work"]["items"][0]["work_state"]["state"],
        "active"
    );
    let third_session = id(&third, "session_id")?;
    assert_eq!(terrain(&third_session)?, at_checkpoint);

    // Restoring invalidated the original handles and retired their journal,
    // but the restored designation still owns its region in the real world.
    let overlapping = plan_one_tile(&third_session)?;
    let refused = parsed(&fortress_commit(
        Some(third_session.clone()),
        id(&overlapping, "plan_digest")?,
    ))?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "conflict", "{refused}");
    let completed = parsed(&fortress_wait(Some(third_session.clone()), Some(250)))?;
    assert_eq!(completed["ok"], true, "{completed}");
    assert_eq!(
        completed["untracked_work"]["quiescent"], true,
        "{completed}"
    );
    assert_eq!(terrain(&third_session)?, finished);
    // The finished hall is already floor; the released lease no longer blocks
    // work beside it whose hazard halo reaches into the hall.
    let now_available = parsed(&fortress_plan(
        Some(third_session.clone()),
        None,
        None,
        Some(
            r#"[{"action":{"kind":"designate_dig","min":[3,6,10],"max":[3,6,10],"mode":"mine"}}]"#
                .to_owned(),
        ),
        None,
    ))?;
    assert_eq!(now_available["ok"], true, "{now_available}");
    let committed = parsed(&fortress_commit(
        Some(third_session),
        id(&now_available, "plan_digest")?,
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    Ok(())
}

/// One durable commit has two journal boundaries: the commit record (before
/// any effect), then the atomic world and progress frontier. Crashing after each
/// must recover honestly: never a verified step without its effect.
#[test]
fn crashes_at_each_commit_boundary_recover_without_false_success() -> TestResult {
    let _serial = serialized();
    let dir =
        StateDir(std::env::temp_dir().join(format!("dfmcp-durable-crash-{}", std::process::id())));
    let dig =
        r#"[{"action":{"kind":"designate_dig","min":[2,3,10],"max":[3,3,10],"mode":"mine"}}]"#;
    let mut outcomes = Vec::new();
    for (case, budget) in [("after_commit_record", 1), ("after_atomic_frontier", 2)] {
        let _ = std::fs::remove_dir_all(&dir.0);
        crate::server::simulate_durable_restart(Some(dir.0.clone()));
        let selector = format!("8803{budget}");
        let opened = open_durable(&selector, Some("starter_fortress"))?;
        let session = id(&opened, "session_id")?;
        let planned = parsed(&fortress_plan(
            Some(session.clone()),
            None,
            None,
            Some(dig.to_owned()),
            None,
        ))?;
        let digest = id(&planned, "plan_digest")?;
        crate::server::inject_durable_crash_after(budget);
        let _ = parsed(&fortress_commit(Some(session), digest.clone()))?;

        // The process dies; a new one resumes from whatever reached disk.
        crate::server::simulate_durable_restart(Some(dir.0.clone()));
        let resumed = open_durable(&selector, None)?;
        assert_eq!(resumed["ok"], true, "{case}: {resumed}");
        let second = id(&resumed, "session_id")?;
        let designations = parsed(&fortress_query(
            Some(second.clone()),
            Some(r#"{"mode":"entities","kind":"dig_designation"}"#.to_owned()),
        ))?;
        let world_has_effect = designations["total"].as_u64().unwrap_or(0) > 0;
        let commits = resumed["durable"]["recovered_commits"].clone();
        let step_state = commits[0]["steps"][0]["state"].as_str().map(str::to_owned);
        assert_eq!(
            commits[0]["plan_digest"],
            digest.as_str(),
            "{case}: {resumed}"
        );
        // Observe completion on its declared cadence before the deadline,
        // then advance beyond it to check that terminal proof remains stable.
        for _ in 0..20 {
            parsed(&fortress_wait(Some(second.clone()), Some(5)))?;
        }
        for _ in 0..10 {
            parsed(&fortress_wait(Some(second.clone()), Some(100)))?;
        }
        let doctor = parsed(&fortress_doctor(Some(second)))?;
        let carried = doctor["durability"]["carried_obligations"][0]["state"]
            .as_str()
            .map(str::to_owned);
        if carried.as_deref() == Some("verified") {
            assert!(world_has_effect, "{case}: verified without the effect");
        }
        outcomes.push((case, world_has_effect, step_state, carried));
    }
    assert_eq!(
        outcomes,
        vec![
            // Nothing past the commit record survived: the plan never ran.
            (
                "after_commit_record",
                false,
                Some("not_dispatched".to_owned()),
                None
            ),
            // The step and its world reached disk together.
            (
                "after_atomic_frontier",
                true,
                Some("dispatched".to_owned()),
                Some("verified".to_owned())
            ),
        ]
    );
    Ok(())
}

#[test]
fn immediate_verified_step_and_temporal_effect_recover_together() -> TestResult {
    let _serial = serialized();
    let dir = StateDir(std::env::temp_dir().join(format!(
        "dfmcp-durable-mixed-frontier-{}",
        std::process::id()
    )));
    for budget in [1, 2] {
        let _ = std::fs::remove_dir_all(&dir.0);
        crate::server::simulate_durable_restart(Some(dir.0.clone()));
        let selector = format!("8898{budget}");
        let opened = open_durable(&selector, Some("starter_fortress"))?;
        let first = id(&opened, "session_id")?;
        let planned = parsed(&fortress_plan(
            Some(first.clone()),
            None,
            None,
            Some(
                r#"[{"action":{"kind":"pause","paused":true}},
                    {"action":{"kind":"designate_dig","min":[2,3,10],"max":[3,3,10],"mode":"mine"}}]"#
                    .to_owned(),
            ),
            None,
        ))?;
        assert_eq!(planned["ok"], true, "{planned}");
        crate::server::inject_durable_crash_after(budget);
        parsed(&fortress_commit(Some(first), id(&planned, "plan_digest")?))?;

        crate::server::simulate_durable_restart(Some(dir.0.clone()));
        let resumed = open_durable(&selector, None)?;
        assert_eq!(resumed["ok"], true, "{resumed}");
        let second = id(&resumed, "session_id")?;
        let observed = parsed(&fortress_observe(Some(second.clone())))?;
        let designations = parsed(&fortress_query(
            Some(second),
            Some(r#"{"mode":"entities","kind":"dig_designation"}"#.to_owned()),
        ))?;
        let steps = &resumed["durable"]["recovered_commits"][0]["steps"];
        if budget == 1 {
            assert_eq!(observed["paused"], false, "{observed}");
            assert_eq!(designations["total"], 0, "{designations}");
            assert_eq!(steps[0]["state"], "not_dispatched", "{resumed}");
            assert_eq!(steps[1]["state"], "not_dispatched", "{resumed}");
        } else {
            assert_eq!(observed["paused"], true, "{observed}");
            assert_eq!(designations["total"], 1, "{designations}");
            assert_eq!(steps[0]["state"], "verified", "{resumed}");
            assert_eq!(steps[1]["state"], "dispatched", "{resumed}");
        }
    }
    Ok(())
}

#[test]
fn a_shared_durable_fortress_resumes_for_every_member() -> TestResult {
    let _serial = serialized();
    let dir =
        StateDir(std::env::temp_dir().join(format!("dfmcp-durable-shared-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    crate::server::simulate_durable_restart(Some(dir.0.clone()));
    let open_shared_durable = |scenario: Option<&str>| {
        parsed(&fortress_open_session(
            Some(false),
            Some("880077".to_owned()),
            Some(caps()),
            None,
            Some(100_000),
            None,
            None,
            Some(8_192),
            None,
            scenario.map(str::to_owned),
            Some(true),
            Some(true),
        ))
    };
    let a = open_shared_durable(Some("starter_fortress"))?;
    assert_eq!(a["ok"], true, "{a}");
    let b = open_shared_durable(None)?;
    assert_eq!(b["ok"], true, "{b}");
    assert_eq!(b["shared_world"]["members"], 2);
    // A process-local member cannot join a durable shared fortress.
    let local = parsed(&fortress_open_session(
        Some(false),
        Some("880077".to_owned()),
        Some(caps()),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(true),
        Some(false),
    ))?;
    assert_eq!(local["ok"], false, "{local}");

    let (a, b) = (id(&a, "session_id")?, id(&b, "session_id")?);
    let planned = parsed(&fortress_plan(
        Some(a.clone()),
        None,
        None,
        Some(
            r#"[{"action":{"kind":"designate_dig","min":[2,3,10],"max":[3,3,10],"mode":"mine"}}]"#
                .to_owned(),
        ),
        None,
    ))?;
    let committed = parsed(&fortress_commit(Some(a), id(&planned, "plan_digest")?))?;
    assert_eq!(committed["ok"], true, "{committed}");
    // B's wait moves the shared clock; the excavation completes.
    for _ in 0..3 {
        parsed(&fortress_wait(Some(b.clone()), Some(20)))?;
    }
    let before = terrain(&b)?;

    crate::server::simulate_durable_restart(Some(dir.0.clone()));
    let a2 = open_shared_durable(None)?;
    assert_eq!(a2["ok"], true, "{a2}");
    assert_eq!(a2["durable"]["resumed"], true, "{a2}");
    let b2 = open_shared_durable(None)?;
    assert_eq!(b2["shared_world"]["joined_existing"], true, "{b2}");
    assert_eq!(terrain(&id(&b2, "session_id")?)?, before);
    assert_eq!(terrain(&id(&a2, "session_id")?)?, before);
    Ok(())
}

fn open_shared_durable(
    selector: &str,
    scenario: Option<&str>,
) -> std::result::Result<Value, Box<dyn std::error::Error>> {
    parsed(&fortress_open_session(
        Some(false),
        Some(selector.to_owned()),
        Some(caps()),
        None,
        Some(100_000),
        None,
        None,
        Some(8_192),
        None,
        scenario.map(str::to_owned),
        Some(true),
        Some(true),
    ))
}

fn plan_one_tile(session: &str) -> std::result::Result<Value, Box<dyn std::error::Error>> {
    let planned = parsed(&fortress_plan(
        Some(session.to_owned()),
        None,
        None,
        Some(
            r#"[{"action":{"kind":"designate_dig","min":[3,5,10],"max":[3,5,10],"mode":"mine"}}]"#
                .to_owned(),
        ),
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    Ok(planned)
}

#[test]
fn recovered_proof_waits_for_fresh_cadence_after_the_world_completes() -> TestResult {
    let _serial = serialized();
    let dir = StateDir(std::env::temp_dir().join(format!(
        "dfmcp-durable-proof-cadence-{}",
        std::process::id()
    )));
    let _ = std::fs::remove_dir_all(&dir.0);
    crate::server::simulate_durable_restart(Some(dir.0.clone()));
    let opened = open_durable("881001", Some("starter_fortress"))?;
    let first = id(&opened, "session_id")?;
    let planned = plan_one_tile(&first)?;
    assert_eq!(planned["steps"][0]["obligation"]["poll_interval_ticks"], 10);
    let committed = parsed(&fortress_commit(
        Some(first.clone()),
        id(&planned, "plan_digest")?,
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let before = parsed(&fortress_wait(Some(first), Some(9)))?;
    assert_eq!(before["game_tick"], 10, "{before}");

    crate::server::simulate_durable_restart(Some(dir.0.clone()));
    let resumed = open_durable("881001", None)?;
    let second = id(&resumed, "session_id")?;
    let early = parsed(&fortress_wait(Some(second.clone()), Some(1)))?;
    assert_eq!(early["ok"], true, "{early}");
    assert_eq!(early["game_tick"], 11, "{early}");
    assert_eq!(
        early["carried_obligations"][0]["state"], "dispatched",
        "{early}"
    );
    assert_eq!(
        early["carried_obligations"][0]["stability"]["consecutive_observations"],
        0
    );
    let rows = terrain(&second)?;
    assert_eq!(
        rows[2].as_str().and_then(|row| row.chars().nth(3)),
        Some('.'),
        "the world goal is already true before the next proof sample: {rows}",
    );
    // Repeated reads at the same game time are not fresh cadence samples.
    for _ in 0..3 {
        let doctor = parsed(&fortress_doctor(Some(second.clone())))?;
        assert_eq!(
            doctor["durability"]["carried_obligations"][0]["state"], "dispatched",
            "{doctor}"
        );
    }
    let eligible = parsed(&fortress_wait(Some(second), Some(9)))?;
    assert_eq!(eligible["game_tick"], 20, "{eligible}");
    assert_eq!(
        eligible["carried_obligations"][0]["state"], "verified",
        "{eligible}"
    );
    assert_eq!(
        eligible["carried_obligations"][0]["proof_anchor"]["game_tick"],
        20
    );
    Ok(())
}

#[test]
fn recovered_proof_keeps_its_original_deadline() -> TestResult {
    let _serial = serialized();
    let dir = StateDir(std::env::temp_dir().join(format!(
        "dfmcp-durable-proof-deadline-{}",
        std::process::id()
    )));
    for late in [false, true] {
        let _ = std::fs::remove_dir_all(&dir.0);
        crate::server::simulate_durable_restart(Some(dir.0.clone()));
        let selector = if late { "881012" } else { "881011" };
        let opened = open_durable(selector, Some("starter_fortress"))?;
        let first = id(&opened, "session_id")?;
        let planned = plan_one_tile(&first)?;
        let deadline = planned["steps"][0]["obligation"]["deadline_tick"]
            .as_u64()
            .ok_or("sealed deadline missing")?;
        let committed = parsed(&fortress_commit(Some(first), id(&planned, "plan_digest")?))?;
        assert_eq!(committed["ok"], true, "{committed}");
        crate::server::simulate_durable_restart(Some(dir.0.clone()));
        let resumed = open_durable(selector, None)?;
        let second = id(&resumed, "session_id")?;
        let tick = resumed["anchor"]["game_tick"]
            .as_u64()
            .ok_or("resume tick missing")?;
        let waited = parsed(&fortress_wait(
            Some(second),
            Some(deadline - tick + u64::from(late)),
        ))?;
        assert_eq!(waited["ok"], true, "{waited}");
        let proof = &waited["carried_obligations"][0];
        assert_eq!(proof["deadline_tick"], deadline, "{waited}");
        assert_eq!(
            proof["state"],
            if late { "failed" } else { "verified" },
            "{waited}"
        );
        if late {
            assert!(
                proof["failure_reason"]
                    .as_str()
                    .is_some_and(|reason| reason.contains("deadline")),
                "{waited}"
            );
        }
    }
    Ok(())
}

#[test]
fn current_observe_authority_is_required_for_carried_proof() -> TestResult {
    let _serial = serialized();
    let dir = StateDir(std::env::temp_dir().join(format!(
        "dfmcp-durable-proof-authority-{}",
        std::process::id()
    )));
    let _ = std::fs::remove_dir_all(&dir.0);
    crate::server::simulate_durable_restart(Some(dir.0.clone()));
    let opened = open_durable("881021", Some("starter_fortress"))?;
    let first = id(&opened, "session_id")?;
    let planned = plan_one_tile(&first)?;
    parsed(&fortress_commit(Some(first), id(&planned, "plan_digest")?))?;
    crate::server::simulate_durable_restart(Some(dir.0.clone()));
    let resumed = parsed(&fortress_open_session(
        Some(false),
        Some("881021".to_owned()),
        Some(vec![("doctor".to_owned(), "read_only".to_owned())]),
        None,
        Some(100_000),
        None,
        None,
        Some(8_192),
        None,
        None,
        None,
        Some(true),
    ))?;
    assert_eq!(resumed["ok"], true, "{resumed}");
    let second = id(&resumed, "session_id")?;
    let handle = crate::server::lookup_session_str(&second)?;
    // Inject autonomous world progress in the test shell. Doctor must not
    // confer Observe authority when the common persistence hook runs.
    crate::server::with_session(
        &handle,
        || {
            Err(dfmcp_core::DfmcpError::new(
                dfmcp_core::ErrorCode::InternalInvariantViolation,
                "test session unavailable",
            ))
        },
        |guard| guard.adapter.advance_ticks(20),
    )?;
    let doctor = parsed(&fortress_doctor(Some(second.clone())))?;
    assert_eq!(doctor["ok"], true, "{doctor}");
    let proof = &doctor["durability"]["carried_obligations"][0];
    assert_eq!(proof["state"], "dispatched", "{doctor}");
    assert!(
        proof["observation_error"]
            .as_str()
            .is_some_and(|error| error.contains("capability_denied")),
        "{doctor}"
    );
    let denied = parsed(&fortress_wait(Some(second), Some(0)))?;
    assert_eq!(denied["ok"], false, "{denied}");
    assert_eq!(denied["error"]["code"], "capability_denied", "{denied}");
    Ok(())
}

#[test]
fn shared_members_cannot_bypass_an_unpublished_frontier_or_recover_twice() -> TestResult {
    let _serial = serialized();
    let dir = StateDir(std::env::temp_dir().join(format!(
        "dfmcp-durable-shared-frontier-{}",
        std::process::id()
    )));
    let _ = std::fs::remove_dir_all(&dir.0);
    crate::server::simulate_durable_restart(Some(dir.0.clone()));
    let a = open_shared_durable("881031", Some("starter_fortress"))?;
    let b = open_shared_durable("881031", None)?;
    assert_eq!(a["ok"], true, "{a}");
    assert_eq!(b["ok"], true, "{b}");
    assert_eq!(b["durable"]["resumed"], false, "{b}");
    assert_eq!(b["durable"]["joined_existing"], true, "{b}");
    assert_eq!(
        a["anchor"], b["anchor"],
        "a join must not create a recovery epoch"
    );
    let (a, b) = (id(&a, "session_id")?, id(&b, "session_id")?);
    let planned = plan_one_tile(&a)?;
    crate::server::inject_durable_crash_after(1);
    parsed(&fortress_commit(
        Some(a.clone()),
        id(&planned, "plan_digest")?,
    ))?;
    let blocked = parsed(&fortress_wait(Some(b.clone()), Some(1)))?;
    assert_eq!(
        blocked["ok"], false,
        "a peer must see the same save fault: {blocked}"
    );
    assert_eq!(blocked["error"]["code"], "adapter_unavailable", "{blocked}");
    let failed_join = open_shared_durable("881031", None)?;
    assert_eq!(failed_join["ok"], false, "{failed_join}");
    crate::server::inject_durable_crash_after(100_000);
    let c = open_shared_durable("881031", None)?;
    assert_eq!(c["ok"], true, "{c}");
    assert_eq!(
        c["shared_world"]["members"], 3,
        "failed joins must not leave unreachable members: {c}"
    );
    assert_eq!(c["durable"]["resumed"], false, "{c}");
    assert_eq!(
        c["anchor"]["game_tick"], 1,
        "the blocked peer did not advance time: {c}"
    );
    let before = parsed(&fortress_doctor(Some(b)))?;
    assert_eq!(
        before["durability"]["persisted_is_current"], true,
        "{before}"
    );

    crate::server::simulate_durable_restart(Some(dir.0.clone()));
    let resumed = open_shared_durable("881031", None)?;
    assert_eq!(resumed["ok"], true, "{resumed}");
    assert_eq!(
        resumed["durable"]["recovered_commits"][0]["steps"][0]["state"], "dispatched",
        "{resumed}"
    );
    let observed = parsed(&fortress_query(
        Some(id(&resumed, "session_id")?),
        Some(r#"{"mode":"entities","kind":"dig_designation"}"#.to_owned()),
    ))?;
    assert_eq!(
        observed["total"], 1,
        "a peer's retry must publish the originating plan with its world: {observed}"
    );
    let stale = parsed(&fortress_observe(Some(a)))?;
    assert_eq!(stale["ok"], false, "{stale}");
    assert_eq!(stale["error"]["code"], "conflict", "{stale}");
    Ok(())
}

#[test]
fn durable_admission_fences_ownership_modes_and_already_resolved_sessions() -> TestResult {
    let _serial = serialized();
    let dir = StateDir(
        std::env::temp_dir().join(format!("dfmcp-durable-ownership-{}", std::process::id())),
    );
    let _ = std::fs::remove_dir_all(&dir.0);
    crate::server::simulate_durable_restart(Some(dir.0.clone()));
    let first = open_durable("881041", Some("starter_fortress"))?;
    let old_handle = crate::server::resolve_session(Some(id(&first, "session_id")?))?;
    let shared = open_shared_durable("881041", None)?;
    assert_eq!(shared["ok"], false, "{shared}");
    assert_eq!(shared["error"]["code"], "conflict", "{shared}");
    let replacement = open_durable("881041", None)?;
    assert_eq!(replacement["ok"], true, "{replacement}");
    let entered = crate::server::with_session(&old_handle, || false, |_| true);
    assert!(
        !entered,
        "a handle resolved before replacement must be fenced before its body runs"
    );
    let running_shared = open_shared_durable("881042", Some("starter_fortress"))?;
    assert_eq!(running_shared["ok"], true, "{running_shared}");
    let private = open_durable("881042", None)?;
    assert_eq!(private["ok"], false, "{private}");
    assert_eq!(private["error"]["code"], "conflict", "{private}");
    Ok(())
}

#[test]
fn restore_world_and_commit_retirement_share_one_crash_boundary() -> TestResult {
    let _serial = serialized();
    let dir = StateDir(std::env::temp_dir().join(format!(
        "dfmcp-durable-restore-frontier-{}",
        std::process::id()
    )));
    for budget in [0, 1] {
        let _ = std::fs::remove_dir_all(&dir.0);
        crate::server::simulate_durable_restart(Some(dir.0.clone()));
        let selector = format!("88105{budget}");
        let opened = open_durable(&selector, Some("starter_fortress"))?;
        let first = id(&opened, "session_id")?;
        let checkpoint = parsed(&fortress_checkpoint(
            Some(first.clone()),
            Some("before excavation".to_owned()),
        ))?;
        assert_eq!(checkpoint["ok"], true, "{checkpoint}");
        let planned = plan_one_tile(&first)?;
        let committed = parsed(&fortress_commit(
            Some(first.clone()),
            id(&planned, "plan_digest")?,
        ))?;
        assert_eq!(committed["ok"], true, "{committed}");
        crate::server::inject_durable_crash_after(budget);
        parsed(&fortress_restore(
            Some(first),
            id(&checkpoint, "checkpoint_id")?,
        ))?;
        crate::server::simulate_durable_restart(Some(dir.0.clone()));
        let resumed = open_durable(&selector, None)?;
        assert_eq!(resumed["ok"], true, "{resumed}");
        let designations = parsed(&fortress_query(
            Some(id(&resumed, "session_id")?),
            Some(r#"{"mode":"entities","kind":"dig_designation"}"#.to_owned()),
        ))?;
        let commits = &resumed["durable"]["recovered_commits"];
        if budget == 0 {
            assert_eq!(designations["total"], 1, "{designations}");
            assert_eq!(commits[0]["steps"][0]["state"], "dispatched", "{resumed}");
        } else {
            assert_eq!(designations["total"], 0, "{designations}");
            assert_eq!(commits.as_array().map(Vec::len), Some(0), "{resumed}");
        }
    }
    Ok(())
}

#[test]
fn legacy_world_ahead_of_its_step_record_remains_indeterminate() -> TestResult {
    let _serial = serialized();
    let dir = StateDir(std::env::temp_dir().join(format!(
        "dfmcp-durable-legacy-frontier-{}",
        std::process::id()
    )));
    for (case, legacy_state) in [
        None,
        Some("not_dispatched"),
        Some("abandoned"),
        Some("verified"),
    ]
    .into_iter()
    .enumerate()
    {
        let _ = std::fs::remove_dir_all(&dir.0);
        crate::server::simulate_durable_restart(Some(dir.0.clone()));
        let selector = format!("88106{case}");
        let opened = open_durable(&selector, Some("starter_fortress"))?;
        let first = id(&opened, "session_id")?;
        let planned = plan_one_tile(&first)?;
        let digest = dfmcp_core::Digest32::from_hex(&id(&planned, "plan_digest")?)
            .ok_or("invalid sealed digest")?;
        let step = u32::try_from(planned["steps"][0]["step"].as_u64().ok_or("step missing")?)?;
        crate::server::inject_durable_crash_after(1);
        parsed(&fortress_commit(Some(first.clone()), digest.to_hex()))?;
        let handle = crate::server::lookup_session_str(&first)?;
        let changed_world = handle
            .lock()
            .map_err(|_| "test session poisoned")?
            .adapter
            .snapshot()
            .clone();

        // Reproduce the older peer-save API: the world contains the effect,
        // while its originating P has no anchored step frontier.
        crate::server::simulate_durable_restart(Some(dir.0.clone()));
        {
            let mut legacy = dfmcp_lab::durable::DurableLabStore::open(&dir.0)?;
            legacy.persist_head("starter_fortress", &changed_world)?;
            if let Some(state) = legacy_state {
                legacy.persist_step(changed_world.fortress_id, digest, step, state)?;
            }
        }
        let resumed = open_durable(&selector, None)?;
        assert_eq!(resumed["ok"], true, "{resumed}");
        let recovered = &resumed["durable"]["recovered_commits"][0]["steps"][0];
        assert_eq!(
            recovered["state"], "indeterminate",
            "legacy {legacy_state:?}: {resumed}"
        );
        assert_eq!(recovered["blind_retry_allowed"], false, "{resumed}");
        assert!(recovered["recorded_anchor"].is_null(), "{resumed}");
        let waited = parsed(&fortress_wait(
            Some(id(&resumed, "session_id")?),
            Some(1_000),
        ))?;
        assert_eq!(
            waited["carried_obligations"][0]["state"], "indeterminate",
            "{waited}"
        );

        crate::server::simulate_durable_restart(Some(dir.0.clone()));
        let again = open_durable(&selector, None)?;
        assert_eq!(again["ok"], true, "{again}");
        assert_eq!(
            again["durable"]["recovered_commits"]
                .as_array()
                .map(Vec::len),
            Some(1),
            "unresolved legacy evidence must not be retired: {again}"
        );
        assert_eq!(
            again["durable"]["carried_obligations"][0]["state"], "indeterminate",
            "{again}"
        );
    }
    Ok(())
}

fn open_durable_production(
    selector: &str,
    scenario: Option<&str>,
) -> std::result::Result<Value, Box<dyn std::error::Error>> {
    let mut capabilities = caps();
    capabilities.push(("configure_production".to_owned(), "reversible".to_owned()));
    parsed(&fortress_open_session(
        Some(false),
        Some(selector.to_owned()),
        Some(capabilities),
        None,
        Some(100_000),
        None,
        None,
        Some(8_192),
        None,
        scenario.map(str::to_owned),
        None,
        Some(true),
    ))
}

fn plan_durable_order(
    session: &str,
    name: &str,
    amount: u32,
    conditions: Value,
) -> std::result::Result<Value, Box<dyn std::error::Error>> {
    let planned = parsed(&fortress_plan(
        Some(session.to_owned()),
        None,
        None,
        Some(
            json!([{"action": {
                "kind": "create_work_order", "name": name,
                "job_token": "MAKE_TEST_ITEM", "amount": amount,
                "conditions": conditions,
            }}])
            .to_string(),
        ),
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    Ok(planned)
}

#[test]
fn durable_failed_work_survives_two_restarts_until_real_completion() -> TestResult {
    let _serial = serialized();
    let dir = StateDir(std::env::temp_dir().join(format!(
        "dfmcp-durable-failed-physical-work-{}",
        std::process::id()
    )));
    let _ = std::fs::remove_dir_all(&dir.0);
    crate::server::simulate_durable_restart(Some(dir.0.clone()));
    let opened = open_durable_production("881071", Some("empty"))?;
    assert_eq!(opened["ok"], true, "{opened}");
    let mut session = id(&opened, "session_id")?;
    let planned = plan_durable_order(
        &session,
        "downstream",
        1,
        json!([{"kind": "completed_order", "order_name": "upstream"}]),
    )?;
    let digest = id(&planned, "plan_digest")?;
    let deadline = planned["steps"][0]["obligation"]["deadline_tick"]
        .as_u64()
        .ok_or("sealed deadline missing")?;
    let tick = opened["anchor"]["game_tick"]
        .as_u64()
        .ok_or("initial game tick missing")?;
    let committed = parsed(&fortress_commit(Some(session.clone()), digest.clone()))?;
    assert_eq!(committed["ok"], true, "{committed}");

    // The prerequisite does not exist, so real work remains active even
    // after the sealed goal's deadline makes its proof permanently Failed.
    let failed = parsed(&fortress_wait(
        Some(session.clone()),
        Some(deadline - tick + 1),
    ))?;
    assert_eq!(failed["ok"], true, "{failed}");
    assert_eq!(failed["commit_state"], "Failed", "{failed}");
    assert_eq!(failed["work_state"]["state"], "active", "{failed}");
    assert_eq!(failed["open_actions_remaining"], 1, "{failed}");
    let original_proof = failed["observed_anchor"].clone();

    for restart in 0..2 {
        crate::server::simulate_durable_restart(Some(dir.0.clone()));
        let resumed = open_durable_production("881071", None)?;
        assert_eq!(resumed["ok"], true, "restart {restart}: {resumed}");
        let commits = &resumed["durable"]["recovered_commits"];
        assert_eq!(commits.as_array().map(Vec::len), Some(1), "{resumed}");
        assert_eq!(commits[0]["plan_digest"], digest, "{resumed}");
        assert_eq!(commits[0]["status"], "carried", "{resumed}");
        let retained = &resumed["durable"]["carried_obligations"][0];
        assert_eq!(retained["state"], "failed", "{resumed}");
        assert_eq!(retained["work_state"]["state"], "active", "{resumed}");
        assert_eq!(retained["work_state"]["quiescent"], false, "{resumed}");
        assert_eq!(retained["proof_anchor"], original_proof, "{resumed}");
        session = id(&resumed, "session_id")?;
        let still_blocked = parsed(&fortress_wait(Some(session.clone()), Some(10)))?;
        assert_eq!(still_blocked["ok"], true, "{still_blocked}");
        let retained = &still_blocked["carried_obligations"][0];
        assert_eq!(retained["state"], "failed", "{still_blocked}");
        assert_eq!(retained["work_state"]["state"], "active", "{still_blocked}");
        assert_eq!(retained["proof_anchor"], original_proof, "{still_blocked}");
    }

    // Satisfy the real prerequisite through a new authorized action. A later
    // physical completion may retire the journal, but cannot rewrite failure.
    let upstream = plan_durable_order(&session, "upstream", 1, json!([]))?;
    let committed = parsed(&fortress_commit(
        Some(session.clone()),
        id(&upstream, "plan_digest")?,
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let upstream_done = parsed(&fortress_wait(Some(session.clone()), Some(50)))?;
    assert_eq!(upstream_done["ok"], true, "{upstream_done}");
    let finished = parsed(&fortress_wait(Some(session), Some(50)))?;
    assert_eq!(finished["ok"], true, "{finished}");
    let retained = &finished["carried_obligations"][0];
    assert_eq!(retained["state"], "failed", "{finished}");
    assert_eq!(retained["work_state"]["state"], "quiescent", "{finished}");
    assert_eq!(retained["work_state"]["quiescent"], true, "{finished}");
    assert_eq!(retained["proof_anchor"], original_proof, "{finished}");

    crate::server::simulate_durable_restart(Some(dir.0.clone()));
    let settled = open_durable_production("881071", None)?;
    assert_eq!(settled["ok"], true, "{settled}");
    assert_eq!(
        settled["durable"]["recovered_commits"]
            .as_array()
            .map(Vec::len),
        Some(0),
        "only observed quiescence permits retirement: {settled}"
    );
    Ok(())
}

#[test]
fn durable_recorded_verified_work_is_retained_until_observed_quiescent() -> TestResult {
    let _serial = serialized();
    let dir = StateDir(std::env::temp_dir().join(format!(
        "dfmcp-durable-verified-physical-work-{}",
        std::process::id()
    )));
    let _ = std::fs::remove_dir_all(&dir.0);
    crate::server::simulate_durable_restart(Some(dir.0.clone()));
    let opened = open_durable_production("881072", Some("empty"))?;
    assert_eq!(opened["ok"], true, "{opened}");
    let mut session = id(&opened, "session_id")?;
    let planned = plan_durable_order(&session, "recorded early proof", 2, json!([]))?;
    let digest_text = id(&planned, "plan_digest")?;
    let digest = dfmcp_core::Digest32::from_hex(&digest_text).ok_or("invalid plan digest")?;
    let step = u32::try_from(planned["steps"][0]["step"].as_u64().ok_or("step missing")?)?;
    let committed = parsed(&fortress_commit(Some(session.clone()), digest_text.clone()))?;
    assert_eq!(committed["ok"], true, "{committed}");
    assert_eq!(committed["actions"][0]["work_state"]["state"], "active");
    let handle = crate::server::lookup_session_str(&session)?;
    let active_world = handle
        .lock()
        .map_err(|_| "test session poisoned")?
        .adapter
        .snapshot()
        .clone();
    let original_proof = crate::server::anchor_json(&active_world.anchor());

    // The public JSON compiler gives orders completion goals. To exercise
    // recovery of an already-recorded early Verified outcome, publish an
    // explicit historical frontier through the real atomic store API. Its
    // action and still-active effect came from an actual public-tool commit;
    // this fixture tests recovery, not creation of a new proof.
    crate::server::simulate_durable_restart(Some(dir.0.clone()));
    {
        let mut store = dfmcp_lab::durable::DurableLabStore::open(&dir.0)?;
        store.persist_progress(
            "empty",
            &active_world,
            &[dfmcp_lab::durable::DurableStepUpdate {
                plan_digest: digest,
                step,
                state: "verified".to_owned(),
            }],
            &[],
        )?;
    }

    for restart in 0..2 {
        crate::server::simulate_durable_restart(Some(dir.0.clone()));
        let resumed = open_durable_production("881072", None)?;
        assert_eq!(resumed["ok"], true, "restart {restart}: {resumed}");
        let commits = &resumed["durable"]["recovered_commits"];
        assert_eq!(commits.as_array().map(Vec::len), Some(1), "{resumed}");
        assert_eq!(commits[0]["plan_digest"], digest_text, "{resumed}");
        let retained = &resumed["durable"]["carried_obligations"][0];
        assert_eq!(retained["state"], "verified", "{resumed}");
        assert_eq!(retained["work_state"]["state"], "active", "{resumed}");
        assert_eq!(retained["work_state"]["quiescent"], false, "{resumed}");
        assert_eq!(retained["proof_anchor"], original_proof, "{resumed}");
        session = id(&resumed, "session_id")?;
    }

    let finished = parsed(&fortress_wait(Some(session), Some(100)))?;
    assert_eq!(finished["ok"], true, "{finished}");
    let retained = &finished["carried_obligations"][0];
    assert_eq!(retained["state"], "verified", "{finished}");
    assert_eq!(retained["work_state"]["state"], "quiescent", "{finished}");
    assert_eq!(retained["work_state"]["quiescent"], true, "{finished}");
    assert_eq!(retained["proof_anchor"], original_proof, "{finished}");
    crate::server::simulate_durable_restart(Some(dir.0.clone()));
    let settled = open_durable_production("881072", None)?;
    assert_eq!(settled["ok"], true, "{settled}");
    assert_eq!(
        settled["durable"]["recovered_commits"]
            .as_array()
            .map(Vec::len),
        Some(0),
        "verified physical work must finish before retirement: {settled}"
    );
    Ok(())
}
