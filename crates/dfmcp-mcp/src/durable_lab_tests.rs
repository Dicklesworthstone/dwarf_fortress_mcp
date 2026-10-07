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
    assert_eq!(terrain(&id(&third, "session_id")?)?, at_checkpoint);
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
