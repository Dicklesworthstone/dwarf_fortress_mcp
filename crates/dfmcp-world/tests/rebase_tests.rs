#![forbid(unsafe_code)]

//! Integration tests for WP-WOR-04 Semantic Rebase and Conflict Certificate Generation.

use std::collections::BTreeMap;
use std::error::Error;

use dfmcp_core::{Digest32, EntityId, FortressId, GameTick, ObservationCursor, PlanId};
use dfmcp_world::{
    ConflictKind, EntityKind, EntityRecord, Fact, FactSource, RebaseOutcome, SemanticRebaseEngine,
    Value, WitnessSet, WorldChange, WorldGraph, WorldSnapshot,
};

fn sample_entity(id: EntityId, generation: u32, rev: u64, label: &str) -> EntityRecord {
    let mut fields = BTreeMap::new();
    fields.insert(
        "name".to_owned(),
        Fact::known(
            Value::Text(label.to_owned()),
            GameTick(100),
            FactSource::Derived("UnitScanner".to_owned()),
            Digest32::ZERO,
        ),
    );
    EntityRecord {
        id,
        generation,
        revision: rev,
        kind: EntityKind::Unit,
        label: label.to_owned(),
        fields,
    }
}

fn sample_snapshot(entities: Vec<EntityRecord>, tick: u64, seq: u64) -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    for entity in entities {
        graph.entities.insert(entity.id, entity);
    }
    WorldSnapshot::new(
        FortressId::new(42),
        GameTick(tick),
        ObservationCursor {
            epoch: 1,
            sequence: seq,
        },
        true,
        graph,
    )
}

#[test]
fn test_clean_rebase_no_conflicts() -> Result<(), Box<dyn Error>> {
    let engine = SemanticRebaseEngine::new();
    let e1 = sample_entity(EntityId::new(1), 1, 1, "Miner 1");
    let base = sample_snapshot(vec![e1.clone()], 100, 1);
    let target = sample_snapshot(vec![e1], 105, 2);

    let e2 = sample_entity(EntityId::new(2), 1, 1, "Mason 2");
    let changes = vec![WorldChange::UpsertEntity(e2.clone())];

    let outcome = engine.rebase_changes(&base, &target, &changes, PlanId::new(100));
    assert!(matches!(outcome, RebaseOutcome::Clean { .. }));
    if let RebaseOutcome::Clean {
        rebased_anchor,
        rebased_changes,
    } = outcome
    {
        assert_eq!(rebased_anchor, target.anchor());
        assert_eq!(rebased_changes.len(), 1);
        if let WorldChange::UpsertEntity(rec) = &rebased_changes[0] {
            assert_eq!(rec.id, EntityId::new(2));
        }
    }
    Ok(())
}

#[test]
fn test_aba_generation_mismatch_conflict() -> Result<(), Box<dyn Error>> {
    let engine = SemanticRebaseEngine::new();
    let e1_base = sample_entity(EntityId::new(1), 1, 1, "Miner 1");
    let base = sample_snapshot(vec![e1_base], 100, 1);

    // Target has advanced generation (Entity 1 was removed and recreated with gen 2)
    let e1_target = sample_entity(EntityId::new(1), 2, 1, "Miner 1 New");
    let target = sample_snapshot(vec![e1_target], 105, 2);

    let e1_change = sample_entity(EntityId::new(1), 1, 2, "Miner 1 Mod");
    let changes = vec![WorldChange::UpsertEntity(e1_change)];

    let outcome = engine.rebase_changes(&base, &target, &changes, PlanId::new(100));
    assert!(matches!(outcome, RebaseOutcome::Conflicted(_)));
    if let RebaseOutcome::Conflicted(cert) = outcome {
        assert_eq!(cert.plan_id, PlanId::new(100));
        assert_eq!(cert.conflict_kind, ConflictKind::EntityUnavailable);
        assert!(cert.diagnosis.contains("ABA generation mismatch"));
    }
    Ok(())
}

#[test]
fn test_witness_validation_with_phantom_detection() -> Result<(), Box<dyn Error>> {
    let engine = SemanticRebaseEngine::new();
    let e1 = sample_entity(EntityId::new(1), 1, 1, "Miner 1");
    let base = sample_snapshot(vec![e1.clone()], 100, 1);

    let mut witness = WitnessSet::new();
    witness.add_positive_entity(EntityId::new(1), 1, 1);
    witness.add_negative_entity(EntityId::new(99));

    // Target violates negative witness by inserting entity 99
    let e99 = sample_entity(EntityId::new(99), 1, 1, "Phantom Intruder");
    let target = sample_snapshot(vec![e1, e99], 105, 2);

    let changes = vec![WorldChange::RemoveEntity {
        id: EntityId::new(1),
        expected_generation: 1,
        expected_revision: 1,
    }];

    let outcome = engine.rebase_with_witness(&base, &target, &witness, &changes, PlanId::new(200));
    assert!(matches!(outcome, RebaseOutcome::Conflicted(_)));
    if let RebaseOutcome::Conflicted(cert) = outcome {
        assert_eq!(cert.plan_id, PlanId::new(200));
        assert!(matches!(
            cert.conflict_kind,
            ConflictKind::PreconditionViolated { .. }
        ));
        assert!(cert.diagnosis.contains("witnessed absent entity 99"));
    }
    Ok(())
}

#[test]
fn test_three_way_merge_concurrent_independent_edits() -> Result<(), Box<dyn Error>> {
    let engine = SemanticRebaseEngine::new();
    let e1 = sample_entity(EntityId::new(1), 1, 1, "Miner 1");
    let base = sample_snapshot(vec![e1.clone()], 100, 1);

    // Ours adds entity 2
    let e2 = sample_entity(EntityId::new(2), 1, 1, "Mason 2");
    let ours = sample_snapshot(vec![e1.clone(), e2.clone()], 105, 2);

    // Theirs adds entity 3
    let e3 = sample_entity(EntityId::new(3), 1, 1, "Carpenter 3");
    let theirs = sample_snapshot(vec![e1.clone(), e3.clone()], 106, 3);

    let merged_res = engine.three_way_merge(&base, &ours, &theirs, PlanId::new(300));
    assert!(merged_res.is_ok());
    if let Ok(merged) = merged_res {
        assert_eq!(merged.graph.entities.len(), 3);
        assert!(merged.graph.entities.contains_key(&EntityId::new(1)));
        assert!(merged.graph.entities.contains_key(&EntityId::new(2)));
        assert!(merged.graph.entities.contains_key(&EntityId::new(3)));
    }
    Ok(())
}
