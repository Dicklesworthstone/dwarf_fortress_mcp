//! Closed opt-in intake for complete selected peer projects.

use super::*;

fn request() -> Value {
    json!({
        "template":"compare_candidates", "version":5,
        "forecast_scope":"complete_project", "progression_scope":"owned_session",
        "competing_plan_digests":["22".repeat(32),"11".repeat(32)],
        "horizon_ticks":2000,
        "candidates":[
            {"key":"existing_labor","blueprint":{
                "template":"production","quotas":[{"item":"BED","minimum":24}],
                "acquire_resources":{"assign_labor":false},
            }},
            {"key":"allow_labor","blueprint":{
                "template":"production","quotas":[{"item":"BED","minimum":24}],
                "acquire_resources":{"assign_labor":true},
            }},
        ],
    })
}

#[test]
fn portfolio_intake_binds_a_canonical_bounded_selected_peer_set() -> Result<()> {
    let parsed = parse(&request().to_string())?.ok_or_else(|| invalid("missing request"))?;
    assert!(uses_owned_progression(&parsed));
    let peers = competing_projects(&parsed).ok_or_else(|| invalid("missing peers"))?;
    assert_eq!(
        peers
            .iter()
            .copied()
            .map(Digest32::to_hex)
            .collect::<Vec<_>>(),
        vec!["11".repeat(32), "22".repeat(32)],
    );
    for count in [1, 7] {
        let mut value = request();
        value["competing_plan_digests"] = json!(
            (1..=count)
                .map(|index| format!("{index:064x}"))
                .collect::<Vec<_>>()
        );
        let parsed = parse(&value.to_string())?.ok_or_else(|| invalid("missing request"))?;
        assert_eq!(
            competing_projects(&parsed).map(<[Digest32]>::len),
            Some(count)
        );
    }
    Ok(())
}

#[test]
fn portfolio_intake_refuses_ambiguous_or_unbounded_peer_identity() {
    for peers in [
        Value::Null,
        json!([]),
        json!(["11".repeat(32), "11".repeat(32)]),
        json!(["A".repeat(64)]),
        json!(["a".repeat(63)]),
        json!([format!("{}\n", "a".repeat(64))]),
        json!(["g".repeat(64)]),
        json!([17]),
        json!(
            (1..=8)
                .map(|index| format!("{index:064x}"))
                .collect::<Vec<_>>()
        ),
    ] {
        let mut value = request();
        value["competing_plan_digests"] = peers;
        assert!(parse(&value.to_string()).is_err(), "{value}");
    }
    let mut missing = request();
    missing
        .as_object_mut()
        .expect("object")
        .remove("competing_plan_digests");
    assert!(parse(&missing.to_string()).is_err());
    let duplicate = request().to_string().replacen(
        "\"competing_plan_digests\":",
        &format!(
            "\"competing_plan_digests\":[\"{}\"],\"competing_plan_digests\":",
            "33".repeat(32),
        ),
        1,
    );
    assert!(parse(&duplicate).is_err());
}

#[test]
fn selected_projects_cannot_leak_into_older_comparison_or_selection_generations() -> Result<()> {
    for version in 1..=4 {
        let mut value = request();
        value["version"] = json!(version);
        let object = value
            .as_object_mut()
            .ok_or_else(|| invalid("missing object"))?;
        object.remove("competing_plan_digests");
        match version {
            1 => {
                object.remove("forecast_scope");
                object.remove("progression_scope");
            }
            2 => {
                object.remove("progression_scope");
            }
            3 => {
                object.insert("forecast_scope".to_owned(), json!("current_stage"));
            }
            _ => {}
        }
        let parsed = parse(&value.to_string())?.ok_or_else(|| invalid("missing legacy request"))?;
        assert!(competing_projects(&parsed).is_none());
        for peers in [json!(["11".repeat(32)]), json!([]), Value::Null] {
            value["competing_plan_digests"] = peers;
            assert!(parse(&value.to_string()).is_err(), "{value}");
        }
    }
    let selection = json!({
        "template":"select_candidate", "version":1,
        "expected_anchor":{"fortress_id":"1","game_tick":0,"epoch":0,
            "sequence":0,"state_hash":"00".repeat(32)},
        "candidate":request()["candidates"][0], "candidate_digest":"00".repeat(32),
        "competing_plan_digests":["11".repeat(32)],
    });
    assert!(parse(&selection.to_string()).is_err());
    Ok(())
}

#[test]
fn portfolio_intake_keeps_complete_project_scope_and_ordinary_source_rules() {
    for (field, value) in [
        ("forecast_scope", json!("current_stage")),
        ("forecast_scope", Value::Null),
        ("progression_scope", json!("selected_projects")),
        ("progression_scope", Value::Null),
    ] {
        let mut altered = request();
        altered[field] = value;
        assert!(parse(&altered.to_string()).is_err(), "{altered}");
    }
    for candidate in [
        json!({"key":"pause", "paused_target":false}),
        json!({"key":"actions", "actions":[]}),
        json!({"key":"continued", "blueprint":{
            "template":"continue_goal", "plan_digest":"11".repeat(32),
        }}),
    ] {
        let mut altered = request();
        altered["candidates"][0] = candidate;
        assert!(parse(&altered.to_string()).is_err(), "{altered}");
    }
}
