use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, Digest32, EntityId, FortressId, GameTick, RequestId,
    SessionId, WorkBudget,
};
use dfmcp_world::{Fact, FactSource, Value as WorldValue, WorldSnapshot};

#[test]
fn retained_advice_refuses_foreign_context_and_competing_plan_sources_before_lookup()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let _serial = crate::server::serialized_durable_tests();
    let opened: Value = serde_json::from_str(&crate::server::open_session_in_scenario(
        Some(false),
        Some("98788".to_owned()),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some("starter_fortress".to_owned()),
        Some(false),
        Some(false),
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    let handle =
        crate::server::lookup_session_str(opened["session_id"].as_str().ok_or("session missing")?)?;
    let owner = handle.lock().map_err(|_| "session poisoned")?;
    for case in 0..4 {
        let mut context = crate::server::read_context(&owner);
        let mut remedy = json!({
            "tool":"fortress.plan", "goal_scope":"joint_supply_recovery",
            "arguments":{"blueprint":json!({
                "template":"continue_goal", "plan_digest":Digest32::ZERO.to_hex(),
            }).to_string()},
        });
        let expected = match case {
            0 => {
                context.session_id = SessionId::new(u128::MAX);
                assert_ne!(context.session_id, owner.session_id);
                ErrorCode::StaleAnchor
            }
            1 => {
                context.anchor.cursor.sequence += 1;
                ErrorCode::StaleAnchor
            }
            2 => {
                remedy["tool"] = json!("fortress.commit");
                ErrorCode::InvalidRequest
            }
            _ => {
                remedy["arguments"]["actions"] = json!([]);
                ErrorCode::InvalidRequest
            }
        };
        let unchanged = remedy.clone();
        let error = lower_owned(&owner, &context, &mut remedy)
            .expect_err("invalid context or source must be rejected before parent lookup");
        assert_eq!(error.code, expected, "case {case}: {error}");
        assert_eq!(remedy, unchanged);
    }
    Ok(())
}

fn snapshot() -> Result<WorldSnapshot> {
    crate::lab_world::scenario_snapshot("starter_fortress", FortressId::new(98_620), true)
}

fn grant(snapshot: &WorldSnapshot, capability: Capability, max_risk: RiskTier) -> CapabilityGrant {
    CapabilityGrant {
        capability,
        max_risk,
        scope: CapabilityScope {
            fortress_id: Some(snapshot.fortress_id),
            ..CapabilityScope::default()
        },
        expires_at_tick: None,
        remaining_uses: None,
    }
}

fn context(snapshot: &WorldSnapshot, grants: &[(Capability, RiskTier)]) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget::default(),
        grants: grants
            .iter()
            .map(|(capability, risk)| grant(snapshot, *capability, *risk))
            .collect(),
        cancellation_requested: false,
    }
}

fn actions(snapshot: &WorldSnapshot, raw: &str) -> Result<Intent> {
    PlanSource::Actions {
        summary: "review alert".to_owned(),
        raw: raw.to_owned(),
    }
    .compile(IntentId::new(1), snapshot)
    .map(|(intent, _)| intent)
}

const RECRUIT: &str = r#"[{"action":{"kind":"assign_squad","units":["1001"],"squad":"8001"}}]"#;

#[test]
fn military_advice_retains_guarded_risk_and_requires_real_checkpoint_authority() -> Result<()> {
    let snapshot = snapshot()?;
    let intent = actions(&snapshot, RECRUIT)?;
    let mut context = context(
        &snapshot,
        &[
            (Capability::Plan, RiskTier::ReadOnly),
            (Capability::ConfigureMilitary, RiskTier::Reversible),
        ],
    );
    let denied = assessment(&snapshot, &context, Ok(intent.clone()));
    assert_eq!(denied["available"], false);
    assert_eq!(denied["risk"], "guarded");
    assert_eq!(denied["reversibility"], "not_compensable");
    assert_eq!(denied["requires_checkpoint"], true);
    assert_eq!(denied["requires_confirmation"], false);
    context.grants[1].max_risk = RiskTier::Guarded;
    assert_eq!(
        assessment(&snapshot, &context, Ok(intent.clone()))["available"],
        false
    );
    context
        .grants
        .push(grant(&snapshot, Capability::Checkpoint, RiskTier::Guarded));
    let before = context.clone();
    let allowed = assessment(&snapshot, &context, Ok(intent));
    assert_eq!(allowed["available"], true, "{allowed:?}");
    assert_eq!(
        context, before,
        "advice must not consume grants or mutate the request"
    );
    Ok(())
}

#[test]
fn current_expiry_fortress_entity_scope_and_use_limits_are_not_capability_names() -> Result<()> {
    let snapshot = snapshot()?;
    let intent = actions(&snapshot, RECRUIT)?;
    let base = context(
        &snapshot,
        &[
            (Capability::Plan, RiskTier::ReadOnly),
            (Capability::ConfigureMilitary, RiskTier::Guarded),
            (Capability::Checkpoint, RiskTier::Guarded),
        ],
    );
    for case in 0..5 {
        let mut restricted = base.clone();
        let military = &mut restricted.grants[1];
        match case {
            0 => military.expires_at_tick = Some(GameTick(0)),
            1 => military.scope.fortress_id = Some(FortressId::new(98_621)),
            2 => military.scope.entity_ids = BTreeSet::from([EntityId::new(1001)]),
            3 => military.remaining_uses = Some(0),
            _ => military.remaining_uses = Some(1),
        }
        let result = assessment(&snapshot, &restricted, Ok(intent.clone()));
        assert_eq!(result["available"], false, "case {case}: {result:?}");
    }
    let mut scoped = base;
    scoped.grants[1].scope.entity_ids = BTreeSet::from([EntityId::new(1001), EntityId::new(8001)]);
    assert_eq!(
        assessment(&snapshot, &scoped, Ok(intent))["available"],
        true
    );
    Ok(())
}

#[test]
fn civilian_workflow_requires_guarded_global_logistics() -> Result<()> {
    let snapshot = snapshot()?;
    let intent = actions(
        &snapshot,
        r#"{"workflow":"civilian_protection_v1","burrow":"9001","civilians":["1001","1002"]}"#,
    )?;
    let mut context = context(
        &snapshot,
        &[
            (Capability::Plan, RiskTier::ReadOnly),
            (Capability::ConfigureLogistics, RiskTier::Reversible),
            (Capability::Checkpoint, RiskTier::Guarded),
        ],
    );
    let denied = assessment(&snapshot, &context, Ok(intent.clone()));
    assert_eq!(denied["risk"], "guarded");
    assert_eq!(denied["requires_checkpoint"], true);
    assert_eq!(denied["available"], false);
    context.grants[1].max_risk = RiskTier::Guarded;
    assert_eq!(
        assessment(&snapshot, &context, Ok(intent.clone()))["available"],
        true
    );
    context.grants[1].scope.entity_ids = BTreeSet::from([
        EntityId::new(1001),
        EntityId::new(1002),
        EntityId::new(9001),
    ]);
    assert_eq!(
        assessment(&snapshot, &context, Ok(intent))["available"],
        false,
        "named membership authority does not cover the global standing order"
    );
    Ok(())
}

#[test]
fn production_uses_current_lowering_and_admin_authority_is_respected() -> Result<()> {
    let snapshot = snapshot()?;
    let request = crate::lab_world::ProductionRequest::parse_current(
        r#"{"template":"production","quotas":[{"item":"DRINK","minimum":200}]}"#,
    )?;
    let (intent, _) = PlanSource::Production {
        summary: "review supply remedy".to_owned(),
        raw: request.canonical_json(),
    }
    .compile(IntentId::new(1), &snapshot)?;
    let context = context(
        &snapshot,
        &[
            (Capability::Plan, RiskTier::ReadOnly),
            (Capability::ConfigureProduction, RiskTier::Reversible),
        ],
    );
    let allowed = assessment(&snapshot, &context, Ok(intent));
    assert_eq!(allowed["available"], true, "{allowed:?}");
    assert_eq!(allowed["risk"], "reversible");
    assert_eq!(allowed["reversibility"], "not_compensable");
    assert_eq!(allowed["requires_checkpoint"], false);
    let administrator = OperationContext {
        grants: vec![grant(&snapshot, Capability::Admin, RiskTier::Guarded)],
        ..context
    };
    assert_eq!(
        assessment(&snapshot, &administrator, actions(&snapshot, RECRUIT))["available"],
        true
    );
    let labor = actions(
        &snapshot,
        r#"[{"action":{"kind":"set_labor","units":["1001"],"labor":"BREW","enabled":true}}]"#,
    )?;
    assert_eq!(
        assessment(&snapshot, &administrator, Ok(labor))["reversibility"],
        "reversible"
    );
    let unknown_prior = actions(
        &snapshot,
        r#"[{"action":{"kind":"set_labor","units":["1001"],"labor":"UNOBSERVED_LABOR","enabled":true}}]"#,
    )?;
    assert_eq!(
        assessment(&snapshot, &administrator, Ok(unknown_prior))["reversibility"],
        "not_compensable",
        "an unobserved prior setting cannot support a restoration claim"
    );
    Ok(())
}

#[test]
fn world_remedy_intake_uses_actual_lowering_without_mutating_its_sources() -> Result<()> {
    let snapshot =
        crate::lab_world::scenario_snapshot("besieged_fortress", FortressId::new(98_622), true)?;
    let context = context(
        &snapshot,
        &[
            (Capability::Plan, RiskTier::ReadOnly),
            (Capability::ConfigureMilitary, RiskTier::Guarded),
            (Capability::Checkpoint, RiskTier::Guarded),
        ],
    );
    let mut recovery = civilian_recovery::CivilianRecovery::default();
    recovery.observe(&snapshot);
    let before = (snapshot.clone(), context.clone(), recovery.projection());
    let mut alerts = crate::lab_world::world_alerts(&snapshot);
    let original = alerts
        .iter()
        .find(|alert| alert["alert"] == "hostile")
        .ok_or_else(|| invalid("scenario has no hostile alert"))?["remedy"]["arguments"]
        .clone();
    qualify(&snapshot, &recovery, &context, &mut alerts, None);
    let remedy = &alerts
        .iter()
        .find(|alert| alert["alert"] == "hostile")
        .ok_or_else(|| invalid("qualification lost the hostile alert"))?["remedy"];
    assert_eq!(remedy["available"], true, "{remedy}");
    assert_eq!(remedy["arguments"], original);
    assert_eq!(remedy["risk"], "guarded");
    assert_eq!(remedy["requires_checkpoint"], true);
    assert_eq!(remedy["reversibility"], "not_compensable");
    assert_eq!(snapshot, before.0);
    assert_eq!(context, before.1);
    assert_eq!(recovery.projection(), before.2);
    Ok(())
}

#[test]
fn stale_authority_cannot_qualify_a_remedy_from_another_anchor() -> Result<()> {
    let snapshot =
        crate::lab_world::scenario_snapshot("besieged_fortress", FortressId::new(98_623), true)?;
    let mut context = context(&snapshot, &[(Capability::Admin, RiskTier::Guarded)]);
    context.anchor.tick = GameTick(snapshot.tick.0 + 1);
    let mut recovery = civilian_recovery::CivilianRecovery::default();
    recovery.observe(&snapshot);
    let before = (snapshot.clone(), context.clone(), recovery.projection());
    // The anchor mismatch must win even before parsing an invalid source.
    assert_eq!(
        lower(&snapshot, &recovery, &context, &Value::Null).map_err(|error| error.code),
        Err(ErrorCode::StaleAnchor)
    );
    let mut alerts = crate::lab_world::world_alerts(&snapshot);
    qualify(&snapshot, &recovery, &context, &mut alerts, None);
    let remedy = &alerts
        .iter()
        .find(|alert| alert["alert"] == "hostile")
        .ok_or_else(|| invalid("qualification lost the hostile alert"))?["remedy"];
    assert_eq!(remedy["available"], false, "{remedy}");
    assert!(remedy["risk"].is_null());
    assert!(
        remedy["unavailable_reason"]
            .as_str()
            .is_some_and(|why| why.contains("anchor"))
    );
    assert_eq!(snapshot, before.0);
    assert_eq!(context, before.1);
    assert_eq!(recovery.projection(), before.2);
    Ok(())
}

#[test]
fn resource_supply_advice_checks_query_source_and_shared_time_before_publication() -> Result<()> {
    let mut snapshot = crate::lab_world::scenario_snapshot(
        "starter_fortress_resources",
        FortressId::new(98_624),
        false,
    )?;
    let tick = snapshot.tick;
    let ledger = snapshot
        .graph
        .entities
        .get_mut(&crate::lab_world::starter::STOCK_LEDGER)
        .ok_or_else(|| invalid("stock fixture missing"))?;
    for field in [
        dfmcp_intent::effects::STOCK_DRINK_FIELD,
        dfmcp_intent::effects::STOCK_PLANT_FIELD,
    ] {
        ledger.fields.insert(
            field.to_owned(),
            Fact::known(
                WorldValue::U64(0),
                tick,
                FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                Digest32::ZERO,
            ),
        );
    }
    snapshot.refresh_hash();
    let mut base = context(
        &snapshot,
        &[
            (Capability::Observe, RiskTier::ReadOnly),
            (Capability::Query, RiskTier::ReadOnly),
            (Capability::Plan, RiskTier::ReadOnly),
            (Capability::Designate, RiskTier::Guarded),
            (Capability::Checkpoint, RiskTier::Guarded),
            (Capability::ConfigureProduction, RiskTier::Reversible),
        ],
    );
    base.budget.max_wall_millis = 60_000;
    base.budget.max_bytes = 4 * 1024 * 1024;
    base.budget.max_entities = 8_192;
    let mut recovery = civilian_recovery::CivilianRecovery::default();
    recovery.observe(&snapshot);
    let original = crate::lab_world::world_alerts(&snapshot);
    let before = snapshot.clone();
    for case in 0..6 {
        let mut current = base.clone();
        match case {
            0 => {}
            1 => current
                .grants
                .retain(|grant| grant.capability != Capability::Query),
            2 => current.grants[1].expires_at_tick = Some(GameTick(0)),
            3 => current.budget.max_bytes = 1,
            4 => current.budget.max_entities = 1,
            _ => current.budget.max_wall_millis = 0,
        }
        let unchanged = current.clone();
        let mut alerts = original.clone();
        qualify(&snapshot, &recovery, &current, &mut alerts, None);
        let remedy = &alerts
            .iter()
            .find(|alert| alert["alert"] == "drink_supply")
            .ok_or_else(|| invalid("drink alert missing"))?["remedy"];
        assert_eq!(remedy["available"], case == 0, "case {case}: {remedy}");
        assert_eq!(remedy["availability_scope"], "current_reviewed_stage_only");
        if case > 0 {
            assert!(remedy["unavailable_reason"].is_string(), "{remedy}");
        } else {
            assert_eq!(remedy["risk"], "guarded");
            assert_eq!(remedy["requires_checkpoint"], true);
        }
        assert_eq!(current, unchanged);
        assert_eq!(snapshot, before);
    }
    Ok(())
}
