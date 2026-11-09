use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, EntityId, FortressId, GameTick, RequestId, SessionId,
    WorkBudget,
};
use dfmcp_world::WorldSnapshot;

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
    let denied = assessment(&context, Ok(intent.clone()));
    assert_eq!(denied["available"], false);
    assert_eq!(denied["risk"], "guarded");
    assert_eq!(denied["reversibility"], "not_compensable");
    assert_eq!(denied["requires_checkpoint"], true);
    assert_eq!(denied["requires_confirmation"], false);
    context.grants[1].max_risk = RiskTier::Guarded;
    assert_eq!(assessment(&context, Ok(intent.clone()))["available"], false);
    context
        .grants
        .push(grant(&snapshot, Capability::Checkpoint, RiskTier::Guarded));
    let before = context.clone();
    let allowed = assessment(&context, Ok(intent));
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
        let result = assessment(&restricted, Ok(intent.clone()));
        assert_eq!(result["available"], false, "case {case}: {result:?}");
    }
    let mut scoped = base;
    scoped.grants[1].scope.entity_ids = BTreeSet::from([EntityId::new(1001), EntityId::new(8001)]);
    assert_eq!(assessment(&scoped, Ok(intent))["available"], true);
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
    let denied = assessment(&context, Ok(intent.clone()));
    assert_eq!(denied["risk"], "guarded");
    assert_eq!(denied["requires_checkpoint"], true);
    assert_eq!(denied["available"], false);
    context.grants[1].max_risk = RiskTier::Guarded;
    assert_eq!(assessment(&context, Ok(intent.clone()))["available"], true);
    context.grants[1].scope.entity_ids = BTreeSet::from([
        EntityId::new(1001),
        EntityId::new(1002),
        EntityId::new(9001),
    ]);
    assert_eq!(
        assessment(&context, Ok(intent))["available"],
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
    let allowed = assessment(&context, Ok(intent));
    assert_eq!(allowed["available"], true, "{allowed:?}");
    assert_eq!(allowed["risk"], "reversible");
    assert_eq!(allowed["reversibility"], "not_compensable");
    assert_eq!(allowed["requires_checkpoint"], false);
    let administrator = OperationContext {
        grants: vec![grant(&snapshot, Capability::Admin, RiskTier::Guarded)],
        ..context
    };
    assert_eq!(
        assessment(&administrator, actions(&snapshot, RECRUIT))["available"],
        true
    );
    let labor = actions(
        &snapshot,
        r#"[{"action":{"kind":"set_labor","units":["1001"],"labor":"BREW","enabled":true}}]"#,
    )?;
    assert_eq!(
        assessment(&administrator, Ok(labor))["reversibility"],
        "reversible"
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
    qualify(&snapshot, &recovery, &context, &mut alerts);
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
    qualify(&snapshot, &recovery, &context, &mut alerts);
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
