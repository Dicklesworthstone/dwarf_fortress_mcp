//! Scenario opt-in and current-frontier projection regressions.

use super::*;
use dfmcp_world::FactPresence;

fn source() -> Result<WorldSnapshot> {
    scenario_snapshot("starter_fortress_workforce", FortressId::new(984), false)
}

#[test]
fn workforce_scenario_is_additive_and_bound_to_the_actual_fortress() -> Result<()> {
    for fortress in [FortressId::new(1), FortressId::new(984)] {
        let legacy = scenario_snapshot("starter_fortress", fortress, false)?;
        let finite = scenario_snapshot("starter_fortress_workforce", fortress, false)?;
        assert_eq!(effects::laboratory_workforce_model(&legacy)?, None);
        assert_eq!(
            effects::laboratory_workforce_model(&finite)?,
            Some(effects::LABORATORY_WORKFORCE_MODEL)
        );
        assert!(workforce_json(&legacy)?.is_none());
        assert!(briefing(&legacy).get("workforce").is_none());
        assert_ne!(legacy.state_hash, finite.state_hash);
        let settings = effects::fortress_settings_entity_id(fortress);
        assert!(finite.graph.entities.contains_key(&settings));
        for unit in finite
            .graph
            .entities
            .values()
            .filter(|entity| entity.kind == EntityKind::Unit)
        {
            assert_eq!(
                unit.fields
                    .get("alive")
                    .and_then(|fact| laboratory_fact_value(fact, finite.tick)),
                Some(&Value::Bool(true))
            );
            for labor in ["MINING", "CONSTRUCTION", "BREW", "COOK", "CARPENTRY"] {
                assert!(matches!(
                    unit.fields
                        .get(&format!("labor.{labor}"))
                        .and_then(|fact| laboratory_fact_value(fact, finite.tick)),
                    Some(Value::Bool(_))
                ));
            }
        }
        // Removing the explicit opt-in additions recovers the exact old
        // canonical world, including its geometry, inventory and timestamps.
        let mut original = finite;
        original.graph.entities.remove(&settings);
        for unit in legacy
            .graph
            .entities
            .values()
            .filter(|entity| entity.kind == EntityKind::Unit)
        {
            original.graph.entities.insert(unit.id, unit.clone());
        }
        original.refresh_hash();
        assert_eq!(original.canonical_bytes(), legacy.canonical_bytes());
        assert_eq!(original.state_hash, legacy.state_hash);
    }
    Ok(())
}

#[test]
fn unavailable_model_never_renders_a_retained_value_as_current_knowledge() -> Result<()> {
    for variant in 0..3 {
        let mut snapshot = source()?;
        let settings = effects::fortress_settings_entity_id(snapshot.fortress_id);
        let fact = snapshot
            .graph
            .entities
            .get_mut(&settings)
            .and_then(|settings| {
                settings
                    .fields
                    .get_mut(effects::LABORATORY_WORKFORCE_MODEL_FIELD)
            })
            .ok_or_else(|| invalid("workforce model fixture is missing"))?;
        match variant {
            0 => fact.presence = Some(FactPresence::Unknown("not observed".to_owned())),
            1 => fact.source = FactSource::AgentAssertion("unsupported claim".to_owned()),
            _ => fact.observed_at = GameTick(2),
        }
        snapshot.refresh_hash();
        assert!(workforce_json(&snapshot).is_err());
        let view = briefing(&snapshot);
        assert_eq!(view["workforce"]["available"], false);
        assert_eq!(view["workforce"]["epistemic_state"], "unknown");
        assert!(view["workforce"].get("model").is_none());
        assert!(view["workforce"].get("assignments").is_none());
    }
    Ok(())
}

#[test]
fn bounded_briefing_does_not_hide_a_later_actions_workforce_blocker() -> Result<()> {
    let mut snapshot = source()?;
    for index in 0..MAX_BRIEFING_ITEMS + 3 {
        let y = i32::try_from(index).map_err(|_| invalid("fixture index overflow"))? + 4;
        let at = MapCoord::new(10, y, starter::LEVEL_Z);
        effects::apply_effect(
            &mut snapshot,
            &Action::DesignateDig {
                area: MapCuboid::new(at, at)?,
                mode: DigMode::Mine,
            },
            &format!("workforce-projection-{index}"),
        )?;
    }
    snapshot.refresh_hash();
    let report = effects::laboratory_workforce_report(&snapshot)?
        .ok_or_else(|| invalid("workforce report fixture is missing"))?;
    assert!(report.blocked.len() > MAX_BRIEFING_ITEMS);
    let last = report
        .blocked
        .last()
        .ok_or_else(|| invalid("blocked activity fixture is missing"))?;
    let view =
        workforce_json(&snapshot)?.ok_or_else(|| invalid("workforce view fixture is missing"))?;
    assert_eq!(
        view["blocked"].as_array().map(Vec::len),
        Some(MAX_BRIEFING_ITEMS)
    );
    assert_eq!(
        view["blocked_omitted"],
        report.blocked.len() - MAX_BRIEFING_ITEMS
    );
    assert_eq!(
        view["source_anchor"]["state_hash"],
        snapshot.state_hash.to_hex()
    );
    let action = workforce_action_json(&snapshot, last.work)?
        .ok_or_else(|| invalid("action workforce fixture is missing"))?;
    assert_eq!(action["state"], "blocked");
    assert_eq!(action["blocker"]["work_entity_id"], last.work.to_string());
    assert_eq!(action["blocker"]["labor"], "MINING");
    assert_eq!(action["blocker"]["reason"], last.reason);
    Ok(())
}
