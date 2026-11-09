use super::*;
use crate::lab_world::{scenario_snapshot, starter};
use dfmcp_core::{Digest32, EntityId, FortressId, GameTick};
use dfmcp_world::{Fact, FactPresence, FactSource};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn lab_fact(value: Value) -> Fact {
    Fact::known(
        value,
        GameTick::new(1),
        FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
        Digest32::ZERO,
    )
}

fn source() -> dfmcp_core::Result<WorldSnapshot> {
    scenario_snapshot("starter_fortress", FortressId::new(98_601), false)
}

fn set(snapshot: &mut WorldSnapshot, id: EntityId, field: &str, fact: Fact) -> TestResult {
    snapshot
        .graph
        .entities
        .get_mut(&id)
        .ok_or("fixture entity missing")?
        .fields
        .insert(field.to_owned(), fact);
    snapshot.refresh_hash();
    Ok(())
}

fn drink(snapshot: &WorldSnapshot) -> std::result::Result<Json, Box<dyn std::error::Error>> {
    world_alerts(snapshot)
        .into_iter()
        .find(|alert| alert["alert"] == "drink_supply")
        .ok_or_else(|| "drink alert missing".into())
}

#[test]
fn unqualified_stock_never_leaks_retained_counts_or_remedies() -> TestResult {
    let template = source()?;
    let mut rejected = Vec::new();
    for presence in [
        FactPresence::Unknown("not established".to_owned()),
        FactPresence::Absent,
        FactPresence::Unsupported("not supported".to_owned()),
        FactPresence::Omitted("not projected".to_owned()),
        FactPresence::Redacted("private".to_owned()),
        FactPresence::Stale(template.anchor()),
        FactPresence::Known(Value::U64(99)),
    ] {
        let mut fact = lab_fact(Value::U64(0));
        fact.presence = Some(presence);
        rejected.push(fact);
    }
    for source in [
        FactSource::AgentAssertion("claim".to_owned()),
        FactSource::Replay,
        FactSource::DfhackField("stock".to_owned()),
        FactSource::Derived("unregistered".to_owned()),
    ] {
        let mut fact = lab_fact(Value::U64(0));
        fact.source = source;
        rejected.push(fact);
    }
    let mut future = lab_fact(Value::U64(0));
    future.observed_at = GameTick::new(template.tick.get() + 1);
    rejected.push(future);
    let mut wrong_digest = lab_fact(Value::U64(0));
    wrong_digest.source_digest = Digest32::from_bytes([1; 32]);
    rejected.push(wrong_digest);
    for fact in rejected {
        let mut snapshot = template.clone();
        set(
            &mut snapshot,
            starter::STOCK_LEDGER,
            effects::STOCK_DRINK_FIELD,
            fact,
        )?;
        let alert = drink(&snapshot)?;
        assert_eq!(alert["epistemic_state"], "unknown", "{alert}");
        for field in ["stock", "ticks_until_exhausted", "remedy"] {
            assert!(alert[field].is_null(), "{field}: {alert}");
        }
        assert!(
            !alert["finding"]
                .as_str()
                .is_some_and(|finding| finding.contains("no drink left"))
        );
    }
    Ok(())
}

#[test]
fn known_empty_stock_survives_unknown_population_without_a_guessed_quota() -> TestResult {
    let mut snapshot = source()?;
    set(
        &mut snapshot,
        starter::STOCK_LEDGER,
        effects::STOCK_DRINK_FIELD,
        lab_fact(Value::U64(0)),
    )?;
    let mut unavailable = lab_fact(Value::Bool(true));
    unavailable.source = FactSource::Replay;
    set(&mut snapshot, EntityId::new(1007), "alive", unavailable)?;
    let alert = drink(&snapshot)?;
    assert_eq!(alert["severity"], "critical");
    assert_eq!(alert["epistemic_state"], "certified_derived");
    assert_eq!(alert["stock"], 0);
    assert_eq!(alert["known_living_units"], 6);
    assert_eq!(alert["unknown_living_units"], 1);
    assert!(alert["living_units"].is_null());
    assert!(alert["ticks_until_exhausted"].is_null());
    assert!(alert["remedy"].is_null());
    Ok(())
}

#[test]
fn kind_aliases_cannot_supply_reference_workers_or_military_targets() -> TestResult {
    let mut snapshot = source()?;
    set(
        &mut snapshot,
        starter::STOCK_LEDGER,
        effects::STOCK_DRINK_FIELD,
        lab_fact(Value::U64(0)),
    )?;
    snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(1003))
        .ok_or("brewer missing")?
        .kind = EntityKind::Other("unit".to_owned());
    let alert = drink(&snapshot)?;
    assert_eq!(alert["unknown_living_units"], 1);
    assert!(alert["remedy"].is_null());

    let mut snapshot = scenario_snapshot("besieged_fortress", FortressId::new(98_605), false)?;
    snapshot
        .graph
        .entities
        .get_mut(&starter::MILITIA_SQUAD)
        .ok_or("squad missing")?
        .kind = EntityKind::Other("squad".to_owned());
    let alert = world_alerts(&snapshot)
        .into_iter()
        .find(|alert| alert["alert"] == "hostile")
        .ok_or("hostile missing")?;
    assert!(alert["remedy"].is_null());
    Ok(())
}

#[test]
fn missing_workshop_is_not_remedied_by_irrelevant_labor_assignment() -> TestResult {
    let mut snapshot = source()?;
    set(
        &mut snapshot,
        starter::STOCK_LEDGER,
        effects::STOCK_DRINK_FIELD,
        lab_fact(Value::U64(0)),
    )?;
    set(
        &mut snapshot,
        starter::STILL,
        effects::CONSTRUCTION_STAGE_FIELD,
        lab_fact(Value::Text("planned".to_owned())),
    )?;
    let alert = drink(&snapshot)?;
    assert!(
        alert["production_blocked_by"]
            .as_str()
            .is_some_and(|why| why.contains("workshop:Still"))
    );
    assert!(alert["remedy"].is_null(), "{alert}");
    Ok(())
}

#[test]
fn labor_remedy_selects_an_established_disabled_worker() -> TestResult {
    let mut snapshot = source()?;
    set(
        &mut snapshot,
        starter::STOCK_LEDGER,
        effects::STOCK_DRINK_FIELD,
        lab_fact(Value::U64(0)),
    )?;
    set(
        &mut snapshot,
        EntityId::new(1003),
        "labor.BREW",
        lab_fact(Value::Bool(false)),
    )?;
    let alert = drink(&snapshot)?;
    let actions: Json = serde_json::from_str(
        alert["remedy"]["arguments"]["actions"]
            .as_str()
            .ok_or("labor actions missing")?,
    )?;
    assert_eq!(actions[0]["action"]["units"], json!(["1003"]));
    assert_eq!(alert["remedy"]["requires"], "configure_labor");
    let mut unknown = lab_fact(Value::Bool(false));
    unknown.source = FactSource::AgentAssertion("claim".to_owned());
    set(&mut snapshot, EntityId::new(1003), "labor.BREW", unknown)?;
    assert!(drink(&snapshot)?["remedy"].is_null());
    Ok(())
}

#[test]
fn retained_deprivation_does_not_claim_refilled_stock_is_empty_or_plan_a_noop() -> TestResult {
    let mut snapshot = source()?;
    set(
        &mut snapshot,
        EntityId::new(1001),
        effects::NEED_DRINK_FIELD,
        lab_fact(Value::Text("thirsty".to_owned())),
    )?;
    let alert = drink(&snapshot)?;
    assert_eq!(alert["stock"], 40);
    assert_eq!(alert["known_deprived_units"], 1);
    assert!(alert["deprived_units"].is_null());
    assert!(alert["remedy"].is_null());
    assert!(
        !alert["finding"]
            .as_str()
            .is_some_and(|text| text.contains("out of drink"))
    );
    // A compatibility thirst value cannot become an observed deprivation count.
    let mut unknown = lab_fact(Value::Text("thirsty".to_owned()));
    unknown.presence = Some(FactPresence::Unknown("unavailable".to_owned()));
    set(
        &mut snapshot,
        EntityId::new(1001),
        effects::NEED_DRINK_FIELD,
        unknown,
    )?;
    set(
        &mut snapshot,
        starter::STOCK_LEDGER,
        effects::STOCK_DRINK_FIELD,
        lab_fact(Value::U64(1)),
    )?;
    assert_eq!(drink(&snapshot)?["known_deprived_units"], 0);
    // Very large stock must not overflow its optional whole-round estimate.
    set(
        &mut snapshot,
        starter::STOCK_LEDGER,
        effects::STOCK_DRINK_FIELD,
        lab_fact(Value::U64(u64::MAX)),
    )?;
    assert!(
        !world_alerts(&snapshot)
            .iter()
            .any(|alert| alert["alert"] == "drink_supply")
    );
    Ok(())
}

#[test]
fn threats_do_not_depend_on_economy_and_never_invent_arrival_or_defense_counts() -> TestResult {
    let mut snapshot = scenario_snapshot("besieged_fortress", FortressId::new(98_602), false)?;
    snapshot.graph.entities.remove(&starter::STOCK_LEDGER);
    let hostile_id = snapshot
        .graph
        .entities
        .values()
        .find(|entity| entity.kind == EntityKind::Creature)
        .ok_or("hostile missing")?
        .id;
    let mut unknown = lab_fact(Value::U64(0));
    unknown.source = FactSource::Replay;
    set(
        &mut snapshot,
        hostile_id,
        effects::ARRIVES_AT_FIELD,
        unknown,
    )?;
    let mut membership = lab_fact(Value::Null);
    membership.presence = Some(FactPresence::Unknown("unavailable".to_owned()));
    set(
        &mut snapshot,
        EntityId::new(1001),
        effects::SQUAD_FIELD,
        membership,
    )?;
    let alert = world_alerts(&snapshot)
        .into_iter()
        .find(|alert| alert["alert"] == "hostile")
        .ok_or("stockless world hid hostile")?;
    assert!(alert["arrives_at_tick"].is_null());
    assert!(alert["soldiers"].is_null());
    assert!(alert["remedy"].is_null());
    assert!(
        alert["finding"]
            .as_str()
            .is_some_and(|finding| finding.contains("arrival time is unknown"))
    );
    for unit in snapshot
        .graph
        .entities
        .values_mut()
        .filter(|entity| entity.kind == EntityKind::Unit)
    {
        unit.fields
            .insert("alive".to_owned(), lab_fact(Value::Bool(false)));
    }
    assert!(
        world_alerts(&snapshot)
            .iter()
            .any(|alert| alert["alert"] == "hostile")
    );
    Ok(())
}

#[test]
fn unavailable_hostility_and_status_are_uncertainty_not_attack_or_clearance() -> TestResult {
    let template = scenario_snapshot("besieged_fortress", FortressId::new(98_603), false)?;
    let hostile_id = template
        .graph
        .entities
        .values()
        .find(|entity| entity.kind == EntityKind::Creature)
        .ok_or("hostile missing")?
        .id;
    for field in [effects::HOSTILE_FIELD, effects::THREAT_STATUS_FIELD] {
        let mut snapshot = template.clone();
        let fact = snapshot
            .graph
            .entities
            .get_mut(&hostile_id)
            .ok_or("hostile missing")?
            .fields
            .get_mut(field)
            .ok_or("threat field missing")?;
        fact.source = FactSource::AgentAssertion("claim".to_owned());
        let alerts = world_alerts(&snapshot);
        assert!(!alerts.iter().any(|alert| alert["alert"] == "hostile"));
        let coverage = alerts
            .iter()
            .find(|alert| alert["alert"] == "threat_coverage")
            .ok_or("unknown threat silently omitted")?;
        assert_eq!(coverage["epistemic_state"], "unknown");
        assert_eq!(coverage["unknown_threats"], 1);
        assert_eq!(
            coverage["known_hostiles_with_unknown_status"],
            usize::from(field == effects::THREAT_STATUS_FIELD)
        );
        assert!(coverage["remedy"].is_null());
    }
    Ok(())
}

#[test]
fn bounded_threat_selection_keeps_critical_hazards_and_names_omissions() -> TestResult {
    let mut snapshot = scenario_snapshot("besieged_fortress", FortressId::new(98_604), false)?;
    let hostile = snapshot
        .graph
        .entities
        .values()
        .find(|entity| entity.kind == EntityKind::Creature)
        .ok_or("hostile missing")?
        .clone();
    for index in 1..=MAX_THREAT_ALERTS {
        let mut added = hostile.clone();
        added.id = EntityId::new(60_000 + index as u64);
        if index == MAX_THREAT_ALERTS {
            added.fields.insert(
                effects::THREAT_STATUS_FIELD.to_owned(),
                lab_fact(Value::Text(effects::THREAT_ATTACKING.to_owned())),
            );
        }
        snapshot.graph.entities.insert(added.id, added);
    }
    snapshot.refresh_hash();
    let alerts = world_alerts(&snapshot);
    let threats: Vec<_> = alerts
        .iter()
        .filter(|alert| alert["alert"] == "hostile")
        .collect();
    assert_eq!(threats.len(), MAX_THREAT_ALERTS);
    assert_eq!(threats[0]["severity"], "critical");
    assert_eq!(
        threats[0]["subject"],
        (60_000 + MAX_THREAT_ALERTS as u64).to_string()
    );
    assert_eq!(
        alerts
            .iter()
            .find(|alert| alert["alert"] == "threat_coverage")
            .ok_or("omission absent")?["omitted_active_threats"],
        1
    );
    assert_eq!(alerts, world_alerts(&snapshot));
    Ok(())
}
