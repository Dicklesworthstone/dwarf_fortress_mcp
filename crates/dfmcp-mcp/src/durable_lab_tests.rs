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

struct StateDir(std::path::PathBuf);

impl Drop for StateDir {
    fn drop(&mut self) {
        crate::server::simulate_durable_restart(None);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn durable_fortress_survives_restart_with_work_and_checkpoints() -> TestResult {
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
            r#"[{"action":{"kind":"designate_dig","min":[0,3,10],"max":[7,5,10],"mode":"mine"}}]"#
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
    assert_eq!(terrain(&id(&third, "session_id")?)?, at_checkpoint);
    Ok(())
}
