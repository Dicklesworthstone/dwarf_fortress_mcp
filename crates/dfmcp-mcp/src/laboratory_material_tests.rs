//! The material scenario extends, rather than reinterprets, earlier worlds.

use super::*;
use dfmcp_world::FactPresence;

fn source() -> Result<WorldSnapshot> {
    scenario_snapshot("starter_fortress_economy", FortressId::new(990), false)
}

#[test]
fn material_scenario_is_additive_and_bound_to_the_actual_fortress() -> Result<()> {
    for fortress in [FortressId::new(1), FortressId::new(990)] {
        let previous = scenario_snapshot("starter_fortress_workforce", fortress, false)?;
        let economy = scenario_snapshot("starter_fortress_economy", fortress, false)?;
        assert_eq!(effects::construction_material_model(&previous)?, None);
        assert!(construction_material_json(&previous)?.is_none());
        assert!(briefing(&previous).get("construction_materials").is_none());
        assert_eq!(
            effects::construction_material_model(&economy)?,
            Some(effects::LABORATORY_CONSTRUCTION_MATERIAL_MODEL)
        );
        assert_eq!(
            effects::laboratory_workforce_model(&economy)?,
            Some(effects::LABORATORY_WORKFORCE_MODEL)
        );
        let view = construction_material_json(&economy)?
            .ok_or_else(|| invalid("material projection missing"))?;
        assert_eq!(view["available_wood"], starter::OPENING_WOOD);
        assert_eq!(view["held_wood"], 0);
        assert_eq!(view["consumed_wood"], 0);
        assert_eq!(view["released_wood"], 0);
        assert_eq!(view["epistemic_state"], "certified_derived");
        assert_eq!(
            view["source_anchor"]["state_hash"],
            economy.state_hash.to_hex()
        );

        let mut normalized = economy;
        normalized
            .graph
            .entities
            .get_mut(&effects::fortress_settings_entity_id(fortress))
            .ok_or_else(|| invalid("material settings missing"))?
            .fields
            .remove(effects::LABORATORY_CONSTRUCTION_MATERIAL_MODEL_FIELD);
        normalized.refresh_hash();
        assert_eq!(normalized.canonical_bytes(), previous.canonical_bytes());
        assert_eq!(normalized.state_hash, previous.state_hash);
    }
    Ok(())
}

#[test]
fn unavailable_material_model_or_inventory_never_becomes_a_current_count() -> Result<()> {
    for field in [
        effects::LABORATORY_CONSTRUCTION_MATERIAL_MODEL_FIELD,
        effects::STOCK_WOOD_FIELD,
    ] {
        for variant in 0..3 {
            let mut snapshot = source()?;
            let id = if field == effects::STOCK_WOOD_FIELD {
                starter::STOCK_LEDGER
            } else {
                effects::fortress_settings_entity_id(snapshot.fortress_id)
            };
            let fact = snapshot
                .graph
                .entities
                .get_mut(&id)
                .and_then(|record| record.fields.get_mut(field))
                .ok_or_else(|| invalid("material fixture fact missing"))?;
            match variant {
                0 => fact.presence = Some(FactPresence::Unknown("not observed".to_owned())),
                1 => fact.source = FactSource::AgentAssertion("unverified stock".to_owned()),
                _ => fact.observed_at = GameTick(2),
            }
            snapshot.refresh_hash();
            assert!(construction_material_json(&snapshot).is_err());
            let view = briefing(&snapshot);
            assert_eq!(view["construction_materials"]["available"], false);
            assert_eq!(view["construction_materials"]["epistemic_state"], "unknown");
            assert!(
                view["construction_materials"]
                    .get("available_wood")
                    .is_none()
            );
            assert!(view["construction_materials"].get("allocations").is_none());
        }
    }
    Ok(())
}

#[test]
fn bounded_material_report_keeps_later_action_allocations_addressable() -> Result<()> {
    let mut snapshot = source()?;
    for index in 0..MAX_BRIEFING_ITEMS + 3 {
        let index = i32::try_from(index).map_err(|_| invalid("material fixture overflow"))?;
        let at = MapCoord::new(index % 7, index / 7, starter::LEVEL_Z);
        effects::apply_effect(
            &mut snapshot,
            &Action::Build {
                kind: BuildingKind::Workshop("Carpenters".to_owned()),
                location: at,
                footprint: MapCuboid::new(at, at)?,
                material: MaterialSelector::default(),
            },
            &format!("material-projection-{index}"),
        )?;
    }
    snapshot.refresh_hash();
    let report = effects::construction_material_report(&snapshot)?
        .ok_or_else(|| invalid("material report missing"))?;
    let last = report
        .allocations
        .last()
        .ok_or_else(|| invalid("allocation missing"))?;
    let view = construction_material_json(&snapshot)?
        .ok_or_else(|| invalid("material projection missing"))?;
    assert_eq!(
        view["allocations"].as_array().map(Vec::len),
        Some(MAX_BRIEFING_ITEMS)
    );
    assert_eq!(view["allocations_omitted"], 3);
    assert_eq!(view["available_wood"], 1);
    assert_eq!(view["held_wood"], 19);
    let action = construction_material_action_json(&snapshot, last.building)?
        .ok_or_else(|| invalid("individual allocation missing"))?;
    assert_eq!(
        action["allocation"]["building_entity_id"],
        last.building.to_string()
    );
    assert_eq!(action["allocation"]["state"], "held");
    assert_eq!(action["allocation"]["wood_units"], 1);
    assert!(construction_material_action_json(&snapshot, starter::CARPENTERS)?.is_none());
    Ok(())
}
