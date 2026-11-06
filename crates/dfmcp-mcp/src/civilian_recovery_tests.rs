//! The actual owner monitor and source codec over canonical laboratory worlds.

use super::*;
use dfmcp_adapter::GameAdapter;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, FortressId, GameTick, OperationContext,
    RequestId, SessionId, WorkBudget,
};
use dfmcp_intent::{Action, effects};
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::{Fact, FactPresence, FactSource, PredicateEvidence, Value as WorldValue};

fn protected() -> Result<WorldSnapshot> {
    let mut snapshot =
        crate::lab_world::scenario_snapshot("starter_fortress", FortressId::new(982), false)?;
    let civilians = laboratory_civilian_census(&snapshot, crate::lab_world::starter::INNER_BURROW)?
        .civilians
        .iter()
        .map(|identity| identity.id)
        .collect();
    effects::apply_effect(
        &mut snapshot,
        &Action::SetStandingOrder {
            key: "CIVILIAN_BURROW_RESTRICTION".to_owned(),
            value: "ACTIVE".to_owned(),
        },
        "test-protect-restriction",
    )?;
    effects::apply_effect(
        &mut snapshot,
        &Action::SetBurrowMembership {
            units: civilians,
            burrow: crate::lab_world::starter::INNER_BURROW,
            assigned: true,
        },
        "test-protect-civilians",
    )?;
    snapshot.refresh_hash();
    Ok(snapshot)
}

fn next(snapshot: &mut WorldSnapshot, ticks: u64) {
    snapshot.tick.0 += ticks;
    snapshot.cursor = snapshot.cursor.next();
    snapshot.refresh_hash();
}

fn ready() -> Result<(WorldSnapshot, CivilianRecovery)> {
    let mut snapshot = protected()?;
    let mut recovery = CivilianRecovery::default();
    recovery.observe(&snapshot);
    next(&mut snapshot, 100);
    recovery.observe(&snapshot);
    assert!(
        recovery
            .release_source(&snapshot, crate::lab_world::starter::INNER_BURROW)
            .is_ok()
    );
    Ok((snapshot, recovery))
}

fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(98_200),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget {
            max_entities: 8_192,
            max_actions: 64,
            max_bytes: 16 * 1024 * 1024,
            max_wall_millis: 60_000,
            ..WorkBudget::default()
        },
        cancellation_requested: false,
        grants: [
            (Capability::Observe, RiskTier::ReadOnly),
            (Capability::Plan, RiskTier::ReadOnly),
            (Capability::ConfigureLogistics, RiskTier::Guarded),
            (Capability::Checkpoint, RiskTier::Guarded),
        ]
        .into_iter()
        .map(|(capability, max_risk)| CapabilityGrant {
            capability,
            max_risk,
            scope: CapabilityScope {
                fortress_id: Some(snapshot.fortress_id),
                ..CapabilityScope::default()
            },
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
    }
}

#[test]
fn duplicates_do_not_clear_and_one_hundred_consecutive_ticks_do() -> Result<()> {
    let mut snapshot = protected()?;
    let mut recovery = CivilianRecovery::default();
    for _ in 0..100 {
        recovery.observe(&snapshot);
    }
    assert_eq!(recovery.projection()["burrows"][0]["clear_samples"], 1);
    assert!(
        recovery
            .release_source(&snapshot, crate::lab_world::starter::INNER_BURROW)
            .is_err()
    );
    assert!(
        !crate::lab_world::world_alerts(&snapshot)
            .iter()
            .any(|alert| alert["alert"] == "all_clear")
    );
    next(&mut snapshot, 99);
    recovery.observe(&snapshot);
    assert!(
        recovery
            .release_source(&snapshot, crate::lab_world::starter::INNER_BURROW)
            .is_err()
    );
    next(&mut snapshot, 1);
    recovery.observe(&snapshot);
    let source = recovery.release_source(&snapshot, crate::lab_world::starter::INNER_BURROW)?;
    assert_eq!(source.clear_since_tick, 1);
    assert_eq!(source.clear_samples, 3);
    let alerts = recovery.alerts();
    assert_eq!(alerts[0]["alert"], "all_clear");
    assert!(alerts[0]["remedy"]["arguments"].get("actions").is_none());
    assert!(is_release_request(
        alerts[0]["remedy"]["arguments"]["blueprint"]
            .as_str()
            .unwrap_or_default()
    ));
    Ok(())
}

#[test]
fn gaps_same_tick_effects_new_epoch_and_restart_rebuild_clearance() -> Result<()> {
    for variation in 0..4 {
        let (mut snapshot, mut recovery) = ready()?;
        match variation {
            0 => {
                next(&mut snapshot, 101);
            }
            1 => {
                next(&mut snapshot, 0);
            }
            2 => {
                snapshot.cursor = snapshot.cursor.reset_epoch();
                snapshot.refresh_hash();
            }
            _ => {
                recovery = CivilianRecovery::default();
            }
        }
        recovery.observe(&snapshot);
        assert!(
            !recovery.projection()["burrows"][0]["release_ready"]
                .as_bool()
                .unwrap_or(true)
        );
        next(&mut snapshot, 100);
        recovery.observe(&snapshot);
        assert!(
            recovery
                .release_source(&snapshot, crate::lab_world::starter::INNER_BURROW)
                .is_ok()
        );
    }
    Ok(())
}

#[test]
fn changed_roster_generation_or_unknown_life_invalidates_owned_clearance() -> Result<()> {
    for variation in 0..3 {
        let (mut snapshot, mut recovery) = ready()?;
        let first = EntityId::new(crate::lab_world::starter::FIRST_DWARF);
        let mut unit = snapshot
            .graph
            .entities
            .get(&first)
            .cloned()
            .ok_or_else(|| invalid("unit"))?;
        match variation {
            0 => {
                unit.id = EntityId::new(1_100);
            }
            1 => {
                unit.generation += 1;
            }
            _ => {
                unit.fields
                    .get_mut("alive")
                    .ok_or_else(|| invalid("life"))?
                    .presence = Some(FactPresence::Unknown(
                    "test observation is incomplete".to_owned(),
                ));
            }
        }
        snapshot.graph.entities.insert(unit.id, unit);
        next(&mut snapshot, 100);
        recovery.observe(&snapshot);
        assert!(
            recovery
                .release_source(&snapshot, crate::lab_world::starter::INNER_BURROW)
                .is_err()
        );
    }
    Ok(())
}

#[test]
fn new_hostile_and_unknown_threat_reset_the_window() -> Result<()> {
    let (mut snapshot, mut recovery) = ready()?;
    let raid =
        crate::lab_world::scenario_snapshot("besieged_fortress", snapshot.fortress_id, false)?;
    let mut hostile = raid.graph.entities[&crate::lab_world::starter::RAIDER].clone();
    snapshot.graph.entities.insert(hostile.id, hostile.clone());
    next(&mut snapshot, 100);
    recovery.observe(&snapshot);
    assert_eq!(recovery.projection()["burrows"][0]["threat"], "present");
    assert_eq!(recovery.projection()["burrows"][0]["clear_samples"], 0);
    hostile
        .fields
        .get_mut(effects::THREAT_STATUS_FIELD)
        .ok_or_else(|| invalid("status"))?
        .presence = Some(FactPresence::Unknown(
        "test observation is incomplete".to_owned(),
    ));
    snapshot.graph.entities.insert(hostile.id, hostile);
    next(&mut snapshot, 100);
    recovery.observe(&snapshot);
    assert_eq!(recovery.projection()["burrows"][0]["threat"], "unknown");
    assert!(
        recovery
            .release_source(&snapshot, crate::lab_world::starter::INNER_BURROW)
            .is_err()
    );
    Ok(())
}

#[test]
fn historical_source_reproduces_the_seal_but_cannot_restore_owned_admission() -> Result<()> {
    let (snapshot, recovery) = ready()?;
    let source = recovery.release_source(&snapshot, crate::lab_world::starter::INNER_BURROW)?;
    let raw = source.canonical_json();
    let (intent, _) = source.compile(IntentId::new(98_201), &snapshot, "release civilians")?;
    let original =
        StaticPlanner::default().prepare_laboratory(&snapshot, &intent, &context(&snapshot))?;
    let (replayed, _) =
        ReleaseSource::parse(&raw)?.compile(intent.id, &snapshot, "release civilians")?;
    let reconstructed =
        StaticPlanner::default().prepare_laboratory(&snapshot, &replayed, &context(&snapshot))?;
    assert_eq!(original.digest, reconstructed.digest);
    recovery.validate_release(&snapshot, &raw)?;
    let mut restarted = CivilianRecovery::default();
    restarted.observe(&snapshot);
    assert!(restarted.validate_release(&snapshot, &raw).is_err());
    let mut changed = snapshot.clone();
    next(&mut changed, 1);
    assert_eq!(
        recovery
            .validate_release(&changed, &raw)
            .err()
            .map(|error| error.code),
        Some(ErrorCode::StaleAnchor)
    );
    let mut forged = source.clone();
    forged.clear_samples += 1;
    let (different_intent, _) = forged.compile(intent.id, &snapshot, "release civilians")?;
    let different = StaticPlanner::default().prepare_laboratory(
        &snapshot,
        &different_intent,
        &context(&snapshot),
    )?;
    assert_ne!(different.digest, original.digest);
    assert!(
        recovery
            .validate_release(&snapshot, &forged.canonical_json())
            .is_err()
    );
    Ok(())
}

#[test]
fn reviewed_source_executes_both_guarded_effects_and_proves_the_current_configuration() -> Result<()>
{
    let (snapshot, mut recovery) = ready()?;
    let source = recovery.release_source(&snapshot, crate::lab_world::starter::INNER_BURROW)?;
    let (intent, _) = source.compile(IntentId::new(98_202), &snapshot, "release civilians")?;
    let plan =
        StaticPlanner::default().prepare_laboratory(&snapshot, &intent, &context(&snapshot))?;
    let mut adapter = MemoryAdapter::new(snapshot);
    let prepared = adapter.prepare(&plan, &context(adapter.snapshot()))?;
    let receipt = adapter.commit(&plan, &prepared, &context(adapter.snapshot()))?;
    for _ in 0..4 {
        for action in &receipt.actions {
            adapter.poll_action(action.action_id, &context(adapter.snapshot()))?;
        }
    }
    assert_eq!(
        PredicateEvidence::laboratory(adapter.snapshot())?.evaluate(&plan.terminal_condition)?,
        PredicateTruth::True
    );
    recovery.observe(adapter.snapshot());
    assert_eq!(
        recovery.projection()["burrows"][0]["released_configuration"],
        "true"
    );
    assert!(
        !recovery
            .alerts()
            .iter()
            .any(|alert| alert["alert"] == "all_clear")
    );
    Ok(())
}

#[test]
fn public_forms_cannot_supply_clearance_or_choose_historical_monitor_state() -> Result<()> {
    for raw in [
        r#"{"template":"civilian_release","burrow":"3001","clear_samples":100}"#,
        r#"{"template":"civilian_release","burrow":"3001","burrow":"3002"}"#,
        r#"{"template":"civilian_release","burrow":"03001"}"#,
        r#"{"template":"civilian_release","burrow":"0"}"#,
    ] {
        assert!(parse_release_request(raw).is_err());
    }
    assert!(is_recovery_query(r#"{"mode":"civilian_recovery","at":"historical"}"#).is_err());
    assert!(
        is_recovery_query(r#"{"mode":"civilian_recovery","mode":"summary"}"#)
            .is_ok_and(|value| !value)
    );
    assert!(is_recovery_query("civilian_recovery")?);
    assert!(is_recovery_query(r#"{"mode":"civilian_recovery"}"#)?);
    assert!(!is_recovery_query("summary")?);
    Ok(())
}

#[test]
fn admitted_reference_producer_refresh_does_not_destroy_continuity() -> Result<()> {
    let mut snapshot = protected()?;
    let mut recovery = CivilianRecovery::default();
    recovery.observe(&snapshot);
    for record in snapshot
        .graph
        .entities
        .values_mut()
        .filter(|record| record.kind == EntityKind::Unit)
    {
        record.fields.insert(
            "alive".to_owned(),
            Fact::known(
                WorldValue::Bool(true),
                GameTick(101),
                FactSource::Derived("dfmcp.reference-effects/1".to_owned()),
                Digest32::ZERO,
            ),
        );
    }
    next(&mut snapshot, 100);
    recovery.observe(&snapshot);
    assert!(
        recovery
            .release_source(&snapshot, crate::lab_world::starter::INNER_BURROW)
            .is_ok()
    );
    Ok(())
}

#[test]
fn protection_advice_is_actionable_and_stops_after_the_configuration_is_complete() -> Result<()> {
    let mut snapshot =
        crate::lab_world::scenario_snapshot("besieged_fortress", FortressId::new(982), false)?;
    let mut recovery = CivilianRecovery::default();
    recovery.observe(&snapshot);
    assert_eq!(
        recovery.alerts()[0]["remedy"]["arguments"]["paused_target"],
        true
    );
    snapshot.paused = true;
    next(&mut snapshot, 0);
    recovery.observe(&snapshot);
    let alerts = recovery.alerts();
    let request: Value = serde_json::from_str(
        alerts[0]["remedy"]["arguments"]["actions"]
            .as_str()
            .unwrap_or_default(),
    )
    .map_err(|error| invalid(error.to_string()))?;
    assert_eq!(request["workflow"], "civilian_protection_v1");
    let raid = snapshot.graph.entities[&crate::lab_world::starter::RAIDER].clone();
    let mut protected = protected()?;
    protected.graph.entities.insert(raid.id, raid);
    next(&mut protected, 1);
    recovery.observe(&protected);
    let alerts = recovery.alerts();
    assert_eq!(alerts[0]["alert"], "civilian_protected");
    assert!(alerts[0].get("remedy").is_none());
    Ok(())
}
