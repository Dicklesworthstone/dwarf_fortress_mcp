//! Existing-session negotiation must survive public presentation and replay.

use super::*;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn parsed(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn renegotiate(session: &str, tokens: u32) -> TestResult<Value> {
    parsed(&fortress_open_session_with_session(
        Some(session.to_owned()),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(tokens),
        None,
        None,
        None,
        None,
    ))
}

#[test]
fn output_negotiation_preserves_pending_review_and_replays_in_the_same_session() -> TestResult {
    let _serial = crate::test_serial();
    let opened = parsed(&fortress_open_session(
        Some(true),
        Some("7855301".to_owned()),
        None,
        None,
        None,
        None,
        None,
        Some(4_096),
        None,
        None,
        None,
        None,
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    let session = opened["session_id"].as_str().ok_or("session missing")?;
    let planned = parsed(&fortress_plan(
        Some(session.to_owned()),
        Some("resume after review".to_owned()),
        Some(false),
        None,
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let digest = planned["plan_digest"].as_str().ok_or("plan missing")?;
    let before = orientation_sessions()
        .get(session)
        .cloned()
        .ok_or("orientation missing")?;

    let accepted = renegotiate(session, 8_192)?;
    assert_eq!(accepted["ok"], true, "{accepted}");
    assert_eq!(accepted["kind"], "output_budget_renegotiation");
    assert_eq!(accepted["session_id"], session);
    assert_eq!(accepted["budget"]["max_output_tokens"], 8_192);
    assert_eq!(accepted["previous_budget"], opened["budget"]);
    assert_eq!(
        accepted["agent_turn"]["anchor"],
        planned["agent_turn"]["anchor"]
    );
    assert_ne!(accepted["agent_turn"]["continuity"]["status"], "bootstrap");
    assert!(accepted.to_string().len() <= 8_192 * 4);
    assert_eq!(
        accepted["agent_turn"]["active_work"]["pending_plans"][0]["plan_digest"],
        digest
    );
    let after = orientation_sessions()
        .get(session)
        .cloned()
        .ok_or("orientation missing")?;
    assert_eq!(after.pending_plan_digest, before.pending_plan_digest);
    assert_eq!(after.pending_plan_steps, before.pending_plan_steps);
    assert_eq!(after.pending_forecast, before.pending_forecast);
    assert_eq!(after.grants, before.grants);
    assert_eq!(after.budget["max_output_tokens"], 8_192);

    // A narrowing that cannot publish the complete receipt leaves the accepted
    // allowance and reviewed work intact, including their presentation hints.
    let refused = renegotiate(session, 1)?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "budget_exceeded");
    let later = parsed(&fortress_observe(Some(session.to_owned())))?;
    assert_eq!(later["ok"], true, "{later}");
    assert_eq!(
        later["agent_turn"]["active_work"]["pending_plans"][0]["plan_digest"],
        digest
    );
    assert_eq!(
        later["agent_turn"]["budget"]["admitted"]["max_output_tokens"],
        8_192
    );
    let committed = parsed(&fortress_commit(
        Some(session.to_owned()),
        digest.to_owned(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");

    let bundle = crate::server::replay_bundle_for(session)?;
    assert_eq!(bundle["replayable"], true, "{bundle}");
    assert_eq!(bundle["calls"][2]["arguments"]["existing_session"], true);
    assert!(bundle["calls"][2]["arguments"].get("session_id").is_none());
    let report = crate::replay::replay_bundle(&bundle);
    assert_eq!(report["ok"], true, "{report}");
    let replayed = report["replay_session_id"]
        .as_str()
        .ok_or("replay session missing")?;
    assert_eq!(crate::server::replay_bundle_for(replayed)?, bundle);
    Ok(())
}

#[test]
fn existing_session_requires_an_explicit_output_only_request() -> TestResult {
    let _serial = crate::test_serial();
    let opened = parsed(&fortress_open_session(
        Some(true),
        Some("7855302".to_owned()),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    ))?;
    let session = opened["session_id"].as_str().ok_or("session missing")?;
    for (tokens, tick_budget, paused) in [
        (None, None, None),
        (Some(8_192), Some(20_000), None),
        (Some(8_192), None, Some(false)),
    ] {
        let refused = parsed(&fortress_open_session_with_session(
            Some(session.to_owned()),
            paused,
            None,
            None,
            None,
            tick_budget,
            None,
            None,
            tokens,
            None,
            None,
            None,
            None,
        ))?;
        assert_eq!(refused["ok"], false, "{refused}");
        assert_eq!(refused["error"]["code"], "invalid_request");
    }
    let observed = parsed(&fortress_observe(Some(session.to_owned())))?;
    assert_eq!(
        observed["agent_turn"]["anchor"],
        opened["agent_turn"]["anchor"]
    );
    assert_eq!(
        observed["agent_turn"]["budget"]["admitted"],
        opened["budget"]
    );
    let report = crate::replay::replay_bundle(&crate::server::replay_bundle_for(session)?);
    assert_eq!(report["ok"], true, "{report}");
    Ok(())
}

#[test]
fn concurrent_budget_receipts_cannot_roll_back_authoritative_presentation() -> TestResult {
    let _serial = crate::test_serial();
    let mut opened = Value::Null;
    // Session IDs are monotonic hex. Choose one with letters so uppercase
    // normalization is exercised regardless of other tests' allocation order.
    for _ in 0..16 {
        opened = parsed(&fortress_open_session(
            Some(true),
            Some("7855303".to_owned()),
            None,
            None,
            None,
            None,
            None,
            Some(4_096),
            None,
            None,
            None,
            None,
        ))?;
        if opened["session_id"]
            .as_str()
            .is_some_and(|id| id.bytes().any(|b| (b'a'..=b'f').contains(&b)))
        {
            break;
        }
    }
    let session = opened["session_id"].as_str().ok_or("session missing")?;
    assert_ne!(session, session.to_ascii_uppercase());
    // Hold both published replies outside the facade to reproduce a delayed
    // first cache write after the later, narrower owner transaction.
    let first = crate::server::session_output_budget::renegotiate(
        session.to_owned(),
        8_192,
        std::time::Instant::now(),
    )?;
    let second = crate::server::session_output_budget::renegotiate(
        session.to_owned(),
        4_096,
        std::time::Instant::now(),
    )?;
    let mut later_anchor = opened["agent_turn"]["anchor"].clone();
    later_anchor["sequence"] = json!(100);
    orientation_sessions()
        .get_mut(session)
        .ok_or("cache missing")?
        .anchor = Some(later_anchor.clone());
    refresh_published_output_budget_orientation(&second);
    refresh_published_output_budget_orientation(&first);
    assert_eq!(
        orientation_sessions()
            .get(session)
            .ok_or("cache missing")?
            .anchor,
        Some(later_anchor)
    );
    assert_eq!(
        orientation_sessions()
            .get(session)
            .ok_or("cache missing")?
            .budget["max_output_tokens"],
        4_096
    );

    // Even before the newer receipt reaches presentation, ordinary rendering
    // must take the current owner allowance instead of a stale cached value.
    orientation_sessions()
        .get_mut(session)
        .ok_or("cache missing")?
        .budget["max_output_tokens"] = json!(8_192);
    let observed_raw = fortress_observe(Some(session.to_owned()));
    let observed = parsed(&observed_raw)?;
    assert_eq!(observed["ok"], true, "{observed}");
    assert_eq!(
        observed["agent_turn"]["budget"]["admitted"]["max_output_tokens"],
        4_096
    );
    assert!(observed_raw.len() <= 4_096 * 4);

    let accepted = renegotiate(&session.to_ascii_uppercase(), 8_192)?;
    assert_eq!(accepted["ok"], true, "{accepted}");
    assert_eq!(accepted["session_id"], session);
    assert_eq!(
        orientation_sessions()
            .get(session)
            .ok_or("cache missing")?
            .budget["max_output_tokens"],
        8_192
    );
    Ok(())
}

#[test]
fn reordered_output_budget_transitions_cannot_claim_replay_equality() -> TestResult {
    let _serial = crate::test_serial();
    let opened = parsed(&fortress_open_session(
        Some(true),
        Some("7855304".to_owned()),
        None,
        None,
        None,
        None,
        None,
        Some(4_096),
        None,
        None,
        None,
        None,
    ))?;
    let session = opened["session_id"].as_str().ok_or("session missing")?;
    assert_eq!(renegotiate(session, 8_192)?["ok"], true);
    assert_eq!(renegotiate(session, 4_096)?["ok"], true);
    let mut bundle = crate::server::replay_bundle_for(session)?;
    assert_eq!(
        bundle["calls"][1]["arguments"]["previous_max_output_tokens"],
        4_096
    );
    assert_eq!(
        bundle["calls"][1]["arguments"]["admitted_max_output_tokens"],
        8_192
    );
    // Simulate two post-owner recorder calls arriving in the opposite order.
    // Their success flags and canonical anchors are identical.
    let calls = bundle["calls"].as_array_mut().ok_or("calls missing")?;
    calls.swap(1, 2);
    calls[1]["seq"] = json!(1);
    calls[2]["seq"] = json!(2);
    let mut bytes = b"dfmcp-replay-bundle-calls/1\0".to_vec();
    bytes.extend_from_slice(bundle["calls"].to_string().as_bytes());
    bundle["calls_digest"] = json!(dfmcp_core::Digest32::of_bytes(&bytes).to_hex());
    let report = crate::replay::replay_bundle(&bundle);
    assert_eq!(report["ok"], false, "{report}");
    assert_eq!(report["first_divergence"]["seq"], 1);
    assert_eq!(
        report["first_divergence"]["field"],
        "previous_max_output_tokens"
    );
    assert_eq!(report["first_divergence"]["expected"], 8_192);
    assert_eq!(report["first_divergence"]["observed"], 4_096);
    Ok(())
}

#[test]
fn wire_keeps_one_opening_tool_with_an_optional_existing_session_selector() {
    let definition = FortressOpenSession.definition();
    assert_eq!(definition.name, "fortress_open_session");
    assert!(
        definition.input_schema["properties"]
            .get("session_id")
            .is_some()
    );
    assert!(
        definition.input_schema["properties"]
            .get("max_output_tokens")
            .is_some()
    );
    assert!(
        !definition.input_schema["required"]
            .as_array()
            .is_some_and(|fields| fields.iter().any(|field| field == "session_id"))
    );
}
