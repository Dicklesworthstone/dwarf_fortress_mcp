use super::*;

use std::collections::BTreeMap;

use crate::{
    EntityKind, EvidenceCoverage, EvidencePolicy, EvidenceSource, Fact, FactPresence, FactSource,
    Predicate, PredicateEvidence, QueryOrder, WorldGraph, WorldQuery,
};
use dfmcp_core::ObservationCursor;

const BURROW: EntityId = EntityId::new(20);
const FIRST: EntityId = EntityId::new(11);
const SECOND: EntityId = EntityId::new(12);
const CREATURE: EntityId = EntityId::new(30);

fn exact(id: EntityId) -> LaboratoryEntityIdentity {
    LaboratoryEntityIdentity { id, generation: 1 }
}

fn fact(value: Value) -> Fact {
    Fact::known(
        value,
        GameTick(10),
        FactSource::Derived("dfmcp.lab-scenario/1".into()),
        Digest32::ZERO,
    )
}

fn record(id: EntityId, kind: EntityKind) -> EntityRecord {
    EntityRecord {
        id,
        generation: 1,
        revision: 1,
        kind,
        label: String::new(),
        fields: BTreeMap::new(),
    }
}

fn world() -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    for id in [FIRST, SECOND] {
        let mut unit = record(id, EntityKind::Unit);
        unit.fields.insert("alive".into(), fact(Value::Bool(true)));
        unit.fields
            .insert("burrow.20".into(), fact(Value::Bool(true)));
        graph.entities.insert(id, unit);
    }
    graph
        .entities
        .insert(BURROW, record(BURROW, EntityKind::Burrow));
    let mut creature = record(CREATURE, EntityKind::Creature);
    creature
        .fields
        .insert("hostile".into(), fact(Value::Bool(true)));
    creature
        .fields
        .insert("threat_status".into(), fact(Value::Text("slain".into())));
    graph.entities.insert(CREATURE, creature);
    let settings = laboratory_fortress_settings_entity_id(FortressId::new(7));
    let mut settings_record = record(settings, EntityKind::Other("fortress_settings".into()));
    settings_record.fields.insert(
        "standing_order.CIVILIAN_BURROW_RESTRICTION".into(),
        fact(Value::Text("ACTIVE".into())),
    );
    graph.entities.insert(settings, settings_record);
    WorldSnapshot::new(
        FortressId::new(7),
        GameTick(10),
        ObservationCursor::ORIGIN,
        false,
        graph,
    )
}

fn set(world: &mut WorldSnapshot, id: EntityId, name: &str, value: Fact) -> Result<()> {
    world
        .graph
        .entities
        .get_mut(&id)
        .ok_or_else(|| invalid("missing fixture entity"))?
        .fields
        .insert(name.into(), value);
    world.refresh_hash();
    Ok(())
}

fn proof() -> Predicate {
    Predicate::LaboratoryCivilianReleaseSafety {
        burrow: exact(BURROW),
        civilians: vec![exact(SECOND), exact(FIRST)],
    }
}

fn truth(world: &WorldSnapshot) -> Result<PredicateTruth> {
    PredicateEvidence::laboratory(world)?.evaluate(&proof())
}

#[test]
fn exact_complete_roster_and_registered_clear_threats_establish_current_safety() -> Result<()> {
    let world = world();
    let census = laboratory_civilian_census(&world, BURROW)?;
    assert_eq!(census.burrow, Some(exact(BURROW)));
    assert_eq!(census.civilians, vec![exact(FIRST), exact(SECOND)]);
    assert!(census.complete_civilian_roster);
    assert_eq!(census.threat, LaboratoryThreatAssessment::Clear);
    assert_eq!(census.protected, PredicateTruth::True);
    assert_eq!(census.released, PredicateTruth::False);
    assert_eq!(census.inspected_entities, 5);
    assert_eq!(truth(&world)?, PredicateTruth::True);
    assert!(crate::evaluate(&world, &proof()));
    Ok(())
}

#[test]
fn every_creature_is_inspected_and_present_dominates_unknown() -> Result<()> {
    for (status, expected) in [
        ("approaching", LaboratoryThreatAssessment::Present),
        ("attacking", LaboratoryThreatAssessment::Present),
        ("slain", LaboratoryThreatAssessment::Clear),
        ("fled", LaboratoryThreatAssessment::Unknown),
    ] {
        let mut world = world();
        set(
            &mut world,
            CREATURE,
            "threat_status",
            fact(Value::Text(status.into())),
        )?;
        assert_eq!(laboratory_civilian_census(&world, BURROW)?.threat, expected);
        if expected != LaboratoryThreatAssessment::Clear {
            assert_ne!(truth(&world)?, PredicateTruth::True);
        }
    }
    let mut world = world();
    world.graph.entities.insert(
        EntityId::new(31),
        record(EntityId::new(31), EntityKind::Creature),
    );
    world.refresh_hash();
    assert_eq!(truth(&world)?, PredicateTruth::Unknown);
    set(
        &mut world,
        CREATURE,
        "threat_status",
        fact(Value::Text("attacking".into())),
    )?;
    assert_eq!(truth(&world)?, PredicateTruth::False);
    set(&mut world, CREATURE, "hostile", fact(Value::Bool(false)))?;
    assert_eq!(truth(&world)?, PredicateTruth::Unknown);
    set(
        &mut world,
        EntityId::new(31),
        "hostile",
        fact(Value::Bool(false)),
    )?;
    assert_eq!(truth(&world)?, PredicateTruth::True);
    Ok(())
}

#[test]
fn unqualified_unknown_future_and_malformed_life_roles_and_threats_never_clear() -> Result<()> {
    for (id, field_name, valid) in [
        (FIRST, "alive", Value::Bool(true)),
        (FIRST, "squad", Value::Null),
        (CREATURE, "hostile", Value::Bool(true)),
        (CREATURE, "threat_status", Value::Text("slain".into())),
    ] {
        for corruption in 0..9 {
            let mut world = world();
            let mut value = fact(valid.clone());
            match corruption {
                0 => value.source = FactSource::Replay,
                1 => value.source = FactSource::AgentAssertion("clear".into()),
                2 => value.source = FactSource::DfhackField(field_name.into()),
                3 => value.source = FactSource::Derived("unregistered-model/1".into()),
                4 => value.source_digest = Digest32::of_bytes(b"unadmitted producer"),
                5 => value.observed_at = GameTick(11),
                6 => value.presence = Some(FactPresence::Unknown("missing".into())),
                7 => value.presence = Some(FactPresence::Known(Value::Bool(false))),
                _ => value.value = Value::U64(1),
            }
            set(&mut world, id, field_name, value)?;
            assert_eq!(
                truth(&world)?,
                PredicateTruth::Unknown,
                "{field_name} corruption {corruption}"
            );
        }
    }
    Ok(())
}

#[test]
fn absent_and_null_roles_are_civilian_but_unknown_or_dangling_military_roles_are_not_complete()
-> Result<()> {
    let mut world = world();
    set(&mut world, SECOND, "squad", fact(Value::Null))?;
    assert_eq!(truth(&world)?, PredicateTruth::True);
    set(
        &mut world,
        SECOND,
        "squad",
        fact(Value::Entity(EntityId::new(40))),
    )?;
    assert_eq!(truth(&world)?, PredicateTruth::Unknown);
    world.graph.entities.insert(
        EntityId::new(40),
        record(EntityId::new(40), EntityKind::Squad),
    );
    world.refresh_hash();
    let census = laboratory_civilian_census(&world, BURROW)?;
    assert!(census.complete_civilian_roster);
    assert_eq!(census.civilians, vec![exact(FIRST)]);
    assert_eq!(truth(&world)?, PredicateTruth::False);
    assert_eq!(
        laboratory_civilian_release_safety_truth(&world, exact(BURROW), &[exact(FIRST)])?,
        PredicateTruth::True
    );
    set(&mut world, SECOND, "alive", fact(Value::Bool(false)))?;
    set(
        &mut world,
        SECOND,
        "squad",
        fact(Value::Text("unknown former role".into())),
    )?;
    assert!(laboratory_civilian_census(&world, BURROW)?.complete_civilian_roster);
    Ok(())
}

#[test]
fn identities_roster_and_roles_reset_scope_but_ordinary_progress_and_configuration_do_not()
-> Result<()> {
    let original = world();
    let before = laboratory_civilian_census(&original, BURROW)?;
    let mut changed = original.clone();
    changed.tick = GameTick(110);
    changed.cursor.sequence += 1;
    changed.paused = true;
    for record in changed.graph.entities.values_mut() {
        record.revision += 1;
        for fact in record.fields.values_mut() {
            fact.source = FactSource::Derived("dfmcp.reference-effects/1".into());
            fact.observed_at = GameTick(110);
        }
    }
    for id in [FIRST, SECOND] {
        set(&mut changed, id, "burrow.20", fact(Value::Bool(false)))?;
    }
    let settings = laboratory_fortress_settings_entity_id(changed.fortress_id);
    set(
        &mut changed,
        settings,
        "standing_order.CIVILIAN_BURROW_RESTRICTION",
        fact(Value::Text("INACTIVE".into())),
    )?;
    let after = laboratory_civilian_census(&changed, BURROW)?;
    assert_eq!(before.source_scope, after.source_scope);
    assert_eq!(after.released, PredicateTruth::True);
    assert_eq!(after.protected, PredicateTruth::False);
    for id in [FIRST, BURROW, CREATURE] {
        let mut changed = original.clone();
        changed
            .graph
            .entities
            .get_mut(&id)
            .ok_or_else(|| invalid("fixture missing"))?
            .generation += 1;
        changed.refresh_hash();
        assert_ne!(
            before.source_scope,
            laboratory_civilian_census(&changed, BURROW)?.source_scope
        );
        if id != CREATURE {
            assert_eq!(truth(&changed)?, PredicateTruth::False);
        }
    }
    let mut changed = original.clone();
    set(&mut changed, FIRST, "alive", fact(Value::Bool(false)))?;
    assert_ne!(
        before.source_scope,
        laboratory_civilian_census(&changed, BURROW)?.source_scope
    );
    assert_eq!(truth(&changed)?, PredicateTruth::False);
    Ok(())
}

#[test]
fn partial_protection_is_retained_and_missing_configuration_never_proves_release() -> Result<()> {
    let mut world = world();
    let settings = laboratory_fortress_settings_entity_id(world.fortress_id);
    world.graph.entities.remove(&settings);
    for id in [FIRST, SECOND] {
        world
            .graph
            .entities
            .get_mut(&id)
            .ok_or_else(|| invalid("fixture missing"))?
            .fields
            .remove("burrow.20");
    }
    world.refresh_hash();
    let census = laboratory_civilian_census(&world, BURROW)?;
    assert_eq!(
        (census.protected, census.released),
        (PredicateTruth::Unknown, PredicateTruth::Unknown)
    );
    // Configuration uncertainty does not manufacture a threat, but it also
    // cannot confirm the FSM is already peacefully released.
    assert_eq!(truth(&world)?, PredicateTruth::True);
    set(&mut world, FIRST, "burrow.20", fact(Value::Bool(true)))?;
    let census = laboratory_civilian_census(&world, BURROW)?;
    assert_eq!(
        (census.protected, census.released),
        (PredicateTruth::True, PredicateTruth::False)
    );
    Ok(())
}

#[test]
fn complete_protection_requires_all_memberships_and_active_order_without_pause() -> Result<()> {
    let mut world = world();
    assert!(!world.paused);
    let census = laboratory_civilian_census(&world, BURROW)?;
    assert_eq!(census.protected, PredicateTruth::True);
    assert_eq!(census.protection_complete, PredicateTruth::True);

    set(&mut world, SECOND, "burrow.20", fact(Value::Bool(false)))?;
    let partial = laboratory_civilian_census(&world, BURROW)?;
    assert_eq!(partial.protected, PredicateTruth::True);
    assert_eq!(partial.protection_complete, PredicateTruth::False);

    world
        .graph
        .entities
        .get_mut(&SECOND)
        .ok_or_else(|| invalid("fixture missing"))?
        .fields
        .remove("burrow.20");
    world.refresh_hash();
    let unknown = laboratory_civilian_census(&world, BURROW)?;
    assert_eq!(unknown.protected, PredicateTruth::True);
    assert_eq!(unknown.protection_complete, PredicateTruth::Unknown);

    set(&mut world, SECOND, "burrow.20", fact(Value::Bool(true)))?;
    let settings = laboratory_fortress_settings_entity_id(world.fortress_id);
    set(
        &mut world,
        settings,
        "standing_order.CIVILIAN_BURROW_RESTRICTION",
        fact(Value::Text("INACTIVE".into())),
    )?;
    let inactive = laboratory_civilian_census(&world, BURROW)?;
    assert_eq!(inactive.protected, PredicateTruth::True);
    assert_eq!(inactive.protection_complete, PredicateTruth::False);
    Ok(())
}

#[test]
fn laboratory_policy_is_distinct_from_native_or_untrusted_evidence() -> Result<()> {
    let world = world();
    assert_eq!(
        PredicateEvidence::untrusted(&world)?.evaluate(&proof())?,
        PredicateTruth::Unknown
    );
    let mut policy = EvidencePolicy::at(world.anchor());
    policy.all_entities = EvidenceCoverage::Complete;
    policy.sources.insert(EvidenceSource::CertifiedDerived {
        derivation: "native-civilian-observer/1".into(),
        source_digest: Digest32::of_bytes(b"live"),
    });
    assert_eq!(
        PredicateEvidence::scoped(&world, policy)?.evaluate(&proof())?,
        PredicateTruth::Unknown
    );
    let mut aliases = world.clone();
    for record in aliases.graph.entities.values_mut() {
        record.kind = EntityKind::Other(record.kind.as_str().to_owned());
    }
    aliases.refresh_hash();
    assert_eq!(aliases.state_hash, world.state_hash);
    assert_eq!(
        laboratory_civilian_census(&aliases, BURROW)?,
        laboratory_civilian_census(&world, BURROW)?
    );
    Ok(())
}

#[test]
fn bounds_identities_and_global_work_are_checked_before_boolean_short_circuits() -> Result<()> {
    for civilians in [
        vec![],
        vec![exact(FIRST); 2],
        vec![exact(EntityId::NIL)],
        vec![exact(BURROW)],
        vec![exact(FIRST); 65],
    ] {
        let predicate = Predicate::LaboratoryCivilianReleaseSafety {
            burrow: exact(BURROW),
            civilians,
        };
        assert!(predicate.validate_shape().is_err());
        assert!(
            Predicate::Any(vec![Predicate::True, predicate])
                .validate_shape()
                .is_err()
        );
    }
    let invalid_generation = Predicate::LaboratoryCivilianReleaseSafety {
        burrow: LaboratoryEntityIdentity {
            id: BURROW,
            generation: 0,
        },
        civilians: vec![exact(FIRST)],
    };
    assert!(invalid_generation.validate_shape().is_err());
    assert!(Predicate::All(vec![proof(); 17]).validate_shape().is_err());
    let mut world = world();
    for id in 100..163 {
        let mut unit = record(EntityId::new(id), EntityKind::Unit);
        unit.fields.insert("alive".into(), fact(Value::Bool(true)));
        world.graph.entities.insert(unit.id, unit);
    }
    world.refresh_hash();
    let census = laboratory_civilian_census(&world, BURROW)?;
    assert_eq!(census.civilians.len(), 64);
    assert!(!census.complete_civilian_roster);
    assert_eq!(truth(&world)?, PredicateTruth::Unknown);
    for id in 200..600 {
        world.graph.entities.insert(
            EntityId::new(id),
            record(EntityId::new(id), EntityKind::Item),
        );
    }
    world.refresh_hash();
    let query = WorldQuery {
        kinds: vec![],
        predicate: Some(proof()),
        order: QueryOrder::EntityIdAscending,
        limit: 1,
        continuation: None,
    };
    assert!(
        matches!(crate::execute_query(&world, &query, 10), Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn canonical_tag_fourteen_binds_generations_and_query_continuation_roster() -> Result<()> {
    let predicate = proof();
    let mut expected = vec![14];
    expected.extend_from_slice(&BURROW.get().to_be_bytes());
    expected.extend_from_slice(&1u32.to_be_bytes());
    expected.extend_from_slice(&2u64.to_be_bytes());
    for id in [FIRST, SECOND] {
        expected.extend_from_slice(&id.get().to_be_bytes());
        expected.extend_from_slice(&1u32.to_be_bytes());
    }
    assert_eq!(predicate.canonical_bytes(), expected);
    assert_eq!(Predicate::Paused(true).canonical_bytes(), vec![6, 1]);
    let world = world();
    let mut query = WorldQuery {
        kinds: vec![EntityKind::Unit],
        predicate: Some(predicate),
        order: QueryOrder::EntityIdAscending,
        limit: 1,
        continuation: None,
    };
    let first = crate::execute_query(&world, &query, 10)?;
    query.continuation = first.continuation;
    assert!(query.continuation.is_some());
    assert_eq!(crate::execute_query(&world, &query, 10)?.entities.len(), 1);
    query.predicate = Some(Predicate::LaboratoryCivilianReleaseSafety {
        burrow: exact(BURROW),
        civilians: vec![exact(FIRST)],
    });
    assert!(crate::execute_query(&world, &query, 10).is_err());
    Ok(())
}
