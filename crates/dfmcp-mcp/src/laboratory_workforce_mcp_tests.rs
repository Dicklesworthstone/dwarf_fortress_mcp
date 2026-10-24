//! Public-facade authority and stale-plan regressions for the opt-in model.
//! These require the complete MCP facade; the focused semantic harness does
//! not establish that this transport and presentation surface executes.

use super::*;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn parsed(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn identity(value: &Value, field: &str) -> TestResult<String> {
    Ok(value[field]
        .as_str()
        .ok_or_else(|| format!("missing {field}: {value}"))?
        .to_owned())
}

fn open(selector: &str, observe: bool) -> TestResult<Value> {
    let mut capabilities: Vec<(String, String)> = [
        ("query", "read_only"),
        ("plan", "read_only"),
        ("control_clock", "reversible"),
        ("designate", "guarded"),
        ("checkpoint", "guarded"),
    ]
    .into_iter()
    .map(|(capability, risk)| (capability.to_owned(), risk.to_owned()))
    .collect();
    if observe {
        capabilities.push(("observe".to_owned(), "read_only".to_owned()));
    }
    let opened = parsed(&fortress_open_session(
        Some(false),
        Some(selector.to_owned()),
        Some(capabilities),
        None,
        Some(2_000),
        None,
        None,
        Some(8_192),
        None,
        Some("starter_fortress_workforce".to_owned()),
        None,
        None,
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    Ok(opened)
}

#[test]
fn workforce_orientation_requires_current_observe_and_does_not_cache_an_old_report() -> TestResult {
    let _serial = crate::server::serialized_durable_tests();
    let unauthorized = open("984401", false)?;
    assert!(unauthorized.get("workforce").is_none());
    assert!(
        unauthorized["agent_turn"]["briefing"]
            .get("workforce")
            .is_none()
    );
    let denied = parsed(&fortress_observe(Some(identity(
        &unauthorized,
        "session_id",
    )?)))?;
    assert_eq!(denied["ok"], false, "{denied}");
    assert!(denied["agent_turn"]["briefing"].get("workforce").is_none());

    let opened = open("984402", true)?;
    let session = identity(&opened, "session_id")?;
    assert_eq!(
        opened["workforce"]["model"],
        dfmcp_intent::effects::LABORATORY_WORKFORCE_MODEL
    );
    assert_eq!(opened["workforce"]["source_anchor"], opened["anchor"]);
    assert_eq!(
        opened["agent_turn"]["briefing"]["workforce"],
        opened["workforce"]
    );
    let waited = parsed(&fortress_wait(Some(session.clone()), Some(1)))?;
    assert_eq!(waited["ok"], true, "{waited}");
    assert!(waited["agent_turn"]["briefing"].get("workforce").is_none());
    let observed = parsed(&fortress_observe(Some(session)))?;
    assert_eq!(observed["ok"], true, "{observed}");
    assert_eq!(
        observed["world"]["workforce"]["source_anchor"],
        observed["anchor"]
    );
    assert_ne!(observed["anchor"], opened["anchor"]);
    assert_eq!(
        observed["agent_turn"]["briefing"]["workforce"],
        observed["world"]["workforce"]
    );
    Ok(())
}

#[test]
fn stale_workforce_plan_requires_reviewed_source_replay_before_dispatch() -> TestResult {
    let _serial = crate::server::serialized_durable_tests();
    let opened = open("984403", true)?;
    let session = identity(&opened, "session_id")?;
    let planned = parsed(&fortress_plan(
        Some(session.clone()),
        Some("excavate with the current finite workforce".to_owned()),
        None,
        Some(
            r#"[{"action":{"kind":"designate_dig","min":[12,3,10],"max":[12,3,10],"mode":"mine"}}]"#
                .to_owned(),
        ),
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let digest = identity(&planned, "plan_digest")?;
    let waited = parsed(&fortress_wait(Some(session.clone()), Some(1)))?;
    assert_eq!(waited["ok"], true, "{waited}");
    let stale = parsed(&fortress_commit(Some(session.clone()), digest.clone()))?;
    assert_eq!(stale["ok"], false, "{stale}");
    assert_eq!(stale["error"]["code"], "stale_anchor");
    assert_eq!(stale["witness_check"]["accepted"], false);
    assert!(
        stale["witness_check"]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("shared workforce")),
        "{stale}"
    );
    assert_eq!(stale["rebase"]["method"], "intent_replay");
    let reviewed = identity(&stale["rebased_plan"], "plan_digest")?;
    assert_ne!(reviewed, digest);
    let observed = parsed(&fortress_observe(Some(session.clone())))?;
    assert_eq!(observed["world"]["workforce"]["active_activities"], 0);
    let terrain = parsed(&fortress_query(
        Some(session.clone()),
        Some(r#"{"mode":"terrain","min":[12,3,10],"max":[12,3,10]}"#.to_owned()),
    ))?;
    assert_eq!(terrain["levels"][0]["rows"][0], "#");
    let committed = parsed(&fortress_commit(Some(session), reviewed))?;
    assert_eq!(committed["ok"], true, "{committed}");
    Ok(())
}
