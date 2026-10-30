use super::*;
use dfmcp_core::{CapabilityScope, RequestId, SessionId, WorkBudget};

fn configuration() -> Value {
    json!({
        "path":"/private/furniture/clock","initialize":true,
        "source":{"endpoint":"127.0.0.1:5000","generation":41,
            "df_version":"test-df","dfhack_version":"test-dfhack"}
    })
}

#[test]
fn clock_configuration_requires_closed_source_and_exact_recovery() -> Result<()> {
    let value = configuration();
    let parsed = Config::parse(&value.to_string())?;
    assert!(parsed.initialize);
    assert_eq!(parsed.binding.manifest().generation, 41);
    assert!(parsed.recovery.is_none());
    let id = Digest32::of_bytes(b"clock-journal").to_string();
    let head = Digest32::of_bytes(b"clock-head").to_string();
    let mut reopened = value.clone();
    reopened["initialize"] = json!(false);
    reopened["recovery"] = json!({"journal_id":id,"head":head});
    assert_eq!(
        Config::parse(&reopened.to_string())?.recovery,
        Some(Recovery {
            journal_id: presentation::digest(&id)?,
            head: presentation::digest(&head)?,
        })
    );
    for (field, bad) in [
        ("initialize", json!(false)),
        ("initialize", json!("true")),
        ("initialize", Value::Null),
        ("path", json!("relative/clock")),
        ("path", json!("/private/../clock")),
        ("path", json!("/private//clock")),
        ("path", json!("/private/clock/")),
        ("recovery", Value::Null),
        ("recovery", json!({"journal_id":id,"head":head})),
        ("clock_authorized", json!(true)),
    ] {
        let mut changed = value.clone();
        changed[field] = bad;
        assert!(
            Config::parse(&changed.to_string()).is_err(),
            "accepted {field}"
        );
    }
    for (field, bad) in [
        ("endpoint", json!("localhost:5000")),
        ("endpoint", json!("192.0.2.1:5000")),
        ("endpoint", json!("127.0.0.1:0")),
        ("generation", json!(0)),
        ("generation", json!(u64::MAX)),
        ("generation", json!(41.0)),
        ("df_version", json!("")),
        ("dfhack_version", json!("bad\nsoftware")),
        ("token", json!("unexpected-secret")),
    ] {
        let mut changed = value.clone();
        changed["source"][field] = bad;
        assert!(
            Config::parse(&changed.to_string()).is_err(),
            "accepted source {field}"
        );
    }
    for field in ["journal_id", "head"] {
        let mut changed = reopened.clone();
        changed["recovery"][field] = json!(Digest32::ZERO.to_string());
        assert!(Config::parse(&changed.to_string()).is_err());
    }
    let duplicate = value.to_string().replacen(
        "\"initialize\":true",
        "\"initialize\":true,\"initialize\":true",
        1,
    );
    assert!(Config::parse(&duplicate).is_err());
    Ok(())
}

#[test]
fn reviewed_run_request_has_only_fixed_identity_and_bounded_duration() -> Result<()> {
    let id = Digest32::of_bytes(b"whole-original-furnishing-monitor").to_string();
    let value = json!({"monitor_id":id,"game_ticks":1,"wall_millis":1});
    assert_eq!(
        Request::parse(&value.to_string())?.typed().monitor_id,
        presentation::digest(&id)?
    );
    for (ticks, millis) in [(1, 1), (1200, 60_000)] {
        let mut changed = value.clone();
        changed["game_ticks"] = json!(ticks);
        changed["wall_millis"] = json!(millis);
        Request::parse(&changed.to_string())?;
    }
    for (field, bad) in [
        ("monitor_id", json!(Digest32::ZERO.to_string())),
        ("monitor_id", json!("not-a-digest")),
        ("game_ticks", json!(0)),
        ("game_ticks", json!(1201)),
        ("game_ticks", json!(1.0)),
        ("game_ticks", json!(true)),
        ("wall_millis", json!(0)),
        ("wall_millis", json!(60_001)),
        ("wall_millis", Value::Null),
        ("deadline_tick", json!(100_000)),
        ("idempotency_key", json!("replacement")),
        ("confirmed", json!(true)),
        ("path", json!("/another/clock")),
    ] {
        let mut changed = value.clone();
        changed[field] = bad;
        assert!(
            Request::parse(&changed.to_string()).is_err(),
            "accepted {field}"
        );
    }
    let duplicate =
        value
            .to_string()
            .replacen("\"game_ticks\":1", "\"game_ticks\":1,\"game_ticks\":1", 1);
    assert!(Request::parse(&duplicate).is_err());
    assert!(Request::parse(&format!("{}{}", " ".repeat(2048), value)).is_err());
    Ok(())
}

fn grant(fortress: FortressId) -> CapabilityGrant {
    CapabilityGrant {
        capability: Capability::Query,
        max_risk: RiskTier::ReadOnly,
        scope: CapabilityScope {
            fortress_id: Some(fortress),
            ..CapabilityScope::default()
        },
        expires_at_tick: None,
        remaining_uses: None,
    }
}

#[test]
fn clock_native_context_preserves_independent_grants_and_expiry() -> Result<()> {
    let fortress = FortressId::new(7);
    let c = OperationContext {
        session_id: SessionId::new(9),
        request_id: RequestId::new(11),
        anchor: StateAnchor {
            fortress_id: fortress,
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(123),
            state_hash: Digest32::of_bytes(b"operations"),
        },
        budget: WorkBudget::CONSERVATIVE_DEFAULT,
        cancellation_requested: false,
        grants: vec![grant(fortress)],
    };
    assert!(native_context(&c, &c.grants).is_err());
    assert!(native_context(&c, &[]).is_err());
    let mut source = grant(FortressId::NIL);
    let native = native_context(&c, std::slice::from_ref(&source))?;
    assert_eq!(native.session_id, c.session_id);
    assert_eq!(native.request_id, c.request_id);
    assert_eq!(native.anchor.tick, c.anchor.tick);
    assert_eq!(native.anchor.fortress_id, FortressId::NIL);
    assert_eq!(native.anchor.cursor, ObservationCursor::ORIGIN);
    assert_eq!(native.anchor.state_hash, Digest32::ZERO);
    assert_eq!(native.grants, vec![source.clone()]);
    assert!(
        native
            .authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)
            .is_err()
    );
    source.expires_at_tick = Some(GameTick(122));
    assert!(native_context(&c, &[source]).is_err());
    let mut cancelled = c.clone();
    cancelled.cancellation_requested = true;
    assert!(native_context(&cancelled, &native.grants).is_err());
    Ok(())
}
