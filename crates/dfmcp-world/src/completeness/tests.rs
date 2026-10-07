use super::*;
use crate::{
    ChunkCoord, EdgeKind, EdgeRecord, EntityRecord, FactSource, MapChunk, TerrainRun, WorldChange,
    WorldEvent, WorldEventKind, WorldGraph,
};
use dfmcp_core::{EdgeId, EntityId, EventId};

fn provenance() -> ProjectionProvenance {
    ProjectionProvenance {
        source_schema: "dfmcp-laboratory-world-v1".to_owned(),
        source_manifest: Digest32::of_bytes(b"fixture-manifest-v1"),
    }
}

fn known(value: Value) -> Fact {
    Fact::known(
        value,
        GameTick(7),
        FactSource::Replay,
        Digest32::of_bytes(b"fixture-source"),
    )
}

fn presence_fields() -> BTreeMap<String, Fact> {
    let old = StateAnchor {
        fortress_id: FortressId::new(7),
        cursor: ObservationCursor {
            epoch: 1,
            sequence: 0,
        },
        tick: GameTick(3),
        state_hash: Digest32::of_bytes(b"older-source"),
    };
    let mut fields = BTreeMap::from([
        ("legacy_null".to_owned(), known(Value::Null)),
        ("amount".to_owned(), known(Value::U64(3))),
        (
            "future.optional".to_owned(),
            known(Value::Object(BTreeMap::from([
                ("opaque".to_owned(), Value::Bytes(vec![0xff, 0, 0x7f])),
                (
                    "sequence".to_owned(),
                    Value::List(vec![Value::Null, Value::Text("modded".to_owned())]),
                ),
            ]))),
        ),
    ]);
    for (name, presence) in [
        ("known_null", FactPresence::Known(Value::Null)),
        ("known_number", FactPresence::Known(Value::U64(4))),
        ("absent", FactPresence::Absent),
        ("unknown", FactPresence::Unknown("not observed".to_owned())),
        (
            "unsupported",
            FactPresence::Unsupported("missing capability".to_owned()),
        ),
        (
            "omitted",
            FactPresence::Omitted("upstream projection".to_owned()),
        ),
        ("redacted", FactPresence::Redacted("private".to_owned())),
        ("stale", FactPresence::Stale(old)),
    ] {
        fields.insert(
            name.to_owned(),
            Fact::with_presence(
                presence,
                GameTick(7),
                FactSource::Replay,
                Digest32::of_bytes(b"fixture-source"),
            ),
        );
    }
    fields
}

fn entity(id: u64, kind: EntityKind) -> EntityRecord {
    EntityRecord {
        id: EntityId::new(id),
        generation: 1,
        revision: 1,
        kind,
        label: format!("entity-{id}"),
        fields: presence_fields(),
    }
}

fn source() -> WorldSnapshot {
    let entities = [
        entity(1, EntityKind::Unit),
        entity(2, EntityKind::HistoricalFigure),
        entity(3, EntityKind::Other("unregistered.mod_kind".to_owned())),
        entity(10, EntityKind::Fortress),
    ]
    .into_iter()
    .map(|entity| (entity.id, entity))
    .collect();
    let edge = |id, to| EdgeRecord {
        id: EdgeId::new(id),
        revision: 1,
        kind: EdgeKind::Custom("unregistered.mod_relation".to_owned()),
        from: EntityId::new(1),
        to: EntityId::new(to),
        fields: presence_fields(),
    };
    let chunk = MapChunk {
        coord: ChunkCoord { x: 0, y: 0, z: 0 },
        revision: 1,
        width: 2,
        height: 2,
        terrain_runs: vec![TerrainRun {
            tile_code: 7,
            length: 4,
        }],
        sparse_overlays: BTreeMap::from([(
            2,
            BTreeMap::from([("mod.overlay".to_owned(), Value::Bytes(vec![0, 0xff]))]),
        )]),
    };
    let event = WorldEvent {
        id: EventId::new(1),
        tick: GameTick(7),
        kind: WorldEventKind::Other("unregistered.mod_event".to_owned()),
        subject: Some(EntityId::new(2)),
        summary: "retained event".to_owned(),
        fields: BTreeMap::from([("mod.event".to_owned(), Value::Text("retained".to_owned()))]),
    };
    WorldSnapshot::new(
        FortressId::new(7),
        GameTick(7),
        ObservationCursor {
            epoch: 1,
            sequence: 1,
        },
        true,
        WorldGraph {
            entities,
            edges: [edge(7, 2), edge(8, 10)]
                .into_iter()
                .map(|edge| (edge.id, edge))
                .collect(),
            chunks: BTreeMap::from([(chunk.coord, chunk)]),
            events: BTreeMap::from([(event.id, event)]),
        },
    )
}

fn projected(source: &WorldSnapshot, profile: CompletenessProfile) -> Result<ProfiledSnapshot> {
    ProfiledSnapshot::project(
        source,
        profile,
        provenance(),
        BTreeMap::from([("optional.mod_payload".to_owned(), vec![0xff, 0, 0x80])]),
    )
}

fn successor(base: &WorldSnapshot) -> WorldSnapshot {
    let mut next = base.clone();
    next.cursor.sequence += 1;
    next.tick = GameTick(8);
    next.graph.entities.remove(&EntityId::new(2));
    let unit = next.graph.entities.get_mut(&EntityId::new(1)).unwrap();
    unit.revision += 1;
    unit.fields
        .insert("amount".to_owned(), known(Value::U64(9)));
    next.graph.edges.remove(&EdgeId::new(7));
    let new_edge = EdgeRecord {
        id: EdgeId::new(9),
        revision: 1,
        kind: EdgeKind::AssignedTo,
        from: EntityId::new(1),
        to: EntityId::new(10),
        fields: presence_fields(),
    };
    next.graph.edges.insert(new_edge.id, new_edge);
    let mut chunk = next.graph.chunks.pop_first().unwrap().1;
    chunk.coord.x = 1;
    next.graph.chunks.insert(chunk.coord, chunk);
    let event = WorldEvent {
        id: EventId::new(2),
        tick: next.tick,
        kind: WorldEventKind::UnitChanged,
        subject: Some(EntityId::new(1)),
        summary: "unit changed".to_owned(),
        fields: BTreeMap::from([("after".to_owned(), Value::U64(9))]),
    };
    next.graph.events.insert(event.id, event);
    next.refresh_hash();
    next
}

#[test]
fn all_five_profiles_round_trip_and_preserve_presence_without_claiming_completeness() -> Result<()>
{
    let source = source();
    let original_bytes = source.canonical_bytes();
    for profile in CompletenessProfile::ALL {
        let projection = projected(&source, profile)?;
        assert_eq!(CompletenessProfile::parse(profile.as_str())?, profile);
        assert_eq!(projection.profile(), profile);
        assert_eq!(projection.source_anchor(), source.anchor());
        assert!(projection.snapshot().hash_is_valid());
        assert_eq!(
            projection.snapshot().graph.entities.len(),
            source.graph.entities.len()
        );
        assert_eq!(
            projection.snapshot().graph.edges.len(),
            source.graph.edges.len()
        );
        for (id, original) in &source.graph.entities {
            let delivered = &projection.snapshot().graph.entities[id];
            assert_eq!(
                (&delivered.id, &delivered.kind, &delivered.label),
                (&original.id, &original.kind, &original.label)
            );
            for (field, fact) in &delivered.fields {
                if profile.includes_entity_fields(&original.kind) {
                    assert_eq!(fact, &original.fields[field]);
                } else {
                    assert_eq!(
                        fact.presence,
                        Some(FactPresence::Omitted(profile.as_str().to_owned()))
                    );
                    assert_eq!(fact.value, Value::Null);
                    assert_eq!(fact.known_value(), None);
                    assert_eq!(fact.source_digest, original.fields[field].source_digest);
                }
            }
        }
        for edge in projection.snapshot().graph.edges.values() {
            let included = [edge.from, edge.to]
                .iter()
                .all(|id| profile.includes_entity_fields(&source.graph.entities[id].kind));
            if included {
                assert_eq!(edge, &source.graph.edges[&edge.id]);
            } else {
                assert!(edge.fields.values().all(|fact| fact.presence
                    == Some(FactPresence::Omitted(profile.as_str().to_owned()))));
            }
        }
        assert_eq!(
            !projection.snapshot().graph.chunks.is_empty(),
            profile.includes_map_chunks()
        );
        assert_eq!(
            !projection.snapshot().graph.events.is_empty(),
            profile.includes_events()
        );
        if profile == CompletenessProfile::ResearchFull {
            assert_eq!(projection.snapshot().anchor(), source.anchor());
        } else {
            assert_ne!(projection.snapshot().state_hash, source.state_hash);
        }
        let bytes = projection.canonical_bytes()?;
        assert_eq!(projection.digest(), Digest32::of_bytes(&bytes));
        assert_eq!(ProfiledSnapshot::from_canonical_bytes(&bytes)?, projection);
        projection.verify_against_source(&source)?;
    }
    assert_eq!(source.canonical_bytes(), original_bytes);
    assert!(CompletenessProfile::parse("briefing").is_err());
    Ok(())
}

#[test]
fn known_null_absent_and_all_unavailable_states_stay_distinct() -> Result<()> {
    let projection = projected(&source(), CompletenessProfile::ResearchFull)?;
    let fields = &projection.snapshot().graph.entities[&EntityId::new(1)].fields;
    assert_eq!(fields["legacy_null"].known_value(), Some(&Value::Null));
    assert_eq!(fields["known_null"].known_value(), Some(&Value::Null));
    assert_ne!(
        fields["known_null"].canonical_bytes(),
        fields["absent"].canonical_bytes()
    );
    for field in [
        "absent",
        "unknown",
        "unsupported",
        "omitted",
        "redacted",
        "stale",
    ] {
        assert_eq!(fields[field].known_value(), None, "{field}");
    }
    assert_eq!(
        projection.extensions()["optional.mod_payload"],
        vec![0xff, 0, 0x80]
    );
    assert_eq!(
        projection.snapshot().graph.entities[&EntityId::new(3)].kind,
        EntityKind::Other("unregistered.mod_kind".to_owned())
    );
    assert_eq!(
        fields["future.optional"].value,
        presence_fields()["future.optional"].value
    );
    Ok(())
}

#[test]
fn canonical_kind_aliases_have_identical_projection_policy_and_bytes() -> Result<()> {
    let original = source();
    let mut alias = original.clone();
    alias
        .graph
        .entities
        .get_mut(&EntityId::new(1))
        .unwrap()
        .kind = EntityKind::Other("unit".to_owned());
    alias.refresh_hash();
    assert_eq!(alias.state_hash, original.state_hash);
    for profile in CompletenessProfile::ALL {
        assert_eq!(
            profile.includes_entity_fields(&EntityKind::Other("unit".to_owned())),
            profile.includes_entity_fields(&EntityKind::Unit)
        );
        assert_eq!(
            projected(&alias, profile)?.canonical_bytes()?,
            projected(&original, profile)?.canonical_bytes()?
        );
    }
    Ok(())
}

#[test]
fn profile_capsules_round_trip_apply_and_decode_every_delta_change() -> Result<()> {
    let source = source();
    let next = successor(&source);
    let delta = diff_snapshots(&source, &next)?;
    let tags: std::collections::BTreeSet<_> = delta
        .changes
        .iter()
        .map(|change| match change {
            WorldChange::UpsertEntity(_) => 0,
            WorldChange::RemoveEntity { .. } => 1,
            WorldChange::UpsertEdge(_) => 2,
            WorldChange::RemoveEdge { .. } => 3,
            WorldChange::UpsertMapChunk(_) => 4,
            WorldChange::RemoveMapChunk { .. } => 5,
            WorldChange::AppendEvent(_) => 6,
        })
        .collect();
    assert_eq!(tags, (0..7).collect());
    assert_eq!(delta.changes.len(), 7);
    let encoded_delta = delta.canonical_bytes();
    assert_eq!(
        crate::StateDelta::from_canonical_bytes(&encoded_delta)?,
        delta
    );
    for profile in CompletenessProfile::ALL {
        let base = projected(&source, profile)?;
        let target = projected(&next, profile)?;
        let capsule = ProfiledObservationCapsule::between(&base, &target, GameTick(8))?;
        assert_eq!(capsule.apply(&base)?, target);
        assert_eq!(capsule.capsule().basis_anchor, base.snapshot().anchor());
        assert_eq!(
            capsule.capsule().successor_anchor,
            target.snapshot().anchor()
        );
        let bytes = capsule.canonical_bytes()?;
        assert_eq!(capsule.digest(), Digest32::of_bytes(&bytes));
        assert_eq!(
            ProfiledObservationCapsule::from_canonical_bytes(&bytes, &base)?,
            capsule
        );
        assert!(ProfiledObservationCapsule::between(&base, &target, GameTick(7)).is_err());
    }
    Ok(())
}

#[test]
fn exact_profile_source_manifest_extensions_and_epoch_are_required_for_deltas() -> Result<()> {
    let source = source();
    let next = successor(&source);
    let base = projected(&source, CompletenessProfile::Operations)?;
    let target = projected(&next, CompletenessProfile::Operations)?;
    let capsule = ProfiledObservationCapsule::between(&base, &target, GameTick(8))?;
    let mut hidden_change = source.clone();
    hidden_change
        .graph
        .entities
        .get_mut(&EntityId::new(2))
        .unwrap()
        .fields
        .insert("amount".to_owned(), known(Value::U64(50)));
    hidden_change.refresh_hash();
    let other_source = projected(&hidden_change, CompletenessProfile::Operations)?;
    assert_eq!(other_source.snapshot().anchor(), base.snapshot().anchor());
    assert_ne!(other_source.source_anchor(), base.source_anchor());
    assert!(capsule.apply(&other_source).is_err());
    assert!(
        ProfiledObservationCapsule::from_canonical_bytes(
            &capsule.canonical_bytes()?,
            &other_source
        )
        .is_err()
    );
    for index in 0..3 {
        let mut provenance = provenance();
        let mut extensions = base.extensions().clone();
        match index {
            0 => provenance.source_manifest = Digest32::of_bytes(b"changed-manifest"),
            1 => provenance.source_schema.push_str("-changed"),
            _ => {
                extensions.insert("optional.new".to_owned(), vec![7]);
            }
        }
        let incompatible = ProfiledSnapshot::project(
            &next,
            base.profile(),
            provenance.clone(),
            extensions.clone(),
        )?;
        assert!(ProfiledObservationCapsule::between(&base, &incompatible, GameTick(8)).is_err());
        let incompatible_base =
            ProfiledSnapshot::project(&source, base.profile(), provenance, extensions)?;
        assert!(capsule.apply(&incompatible_base).is_err());
    }
    let empty = WorldSnapshot::new(
        FortressId::new(7),
        GameTick(7),
        ObservationCursor {
            epoch: 1,
            sequence: 1,
        },
        true,
        WorldGraph::default(),
    );
    let mut advanced = empty.clone();
    advanced.tick = GameTick(8);
    advanced.cursor.sequence += 1;
    advanced.refresh_hash();
    let control = projected(&empty, CompletenessProfile::ControlMinimum)?;
    let full = projected(&empty, CompletenessProfile::ResearchFull)?;
    assert_eq!(control.snapshot().anchor(), full.snapshot().anchor());
    assert_ne!(control.digest(), full.digest());
    assert!(
        ProfiledObservationCapsule::between(
            &control,
            &projected(&advanced, CompletenessProfile::ResearchFull)?,
            GameTick(8)
        )
        .is_err()
    );
    let mut restored = next.clone();
    restored.cursor = ObservationCursor {
        epoch: 2,
        sequence: 1,
    };
    restored.refresh_hash();
    assert!(
        ProfiledObservationCapsule::between(
            &base,
            &projected(&restored, base.profile())?,
            GameTick(8)
        )
        .is_err()
    );
    Ok(())
}

/// Build intentionally untrusted bytes without granting access to the public
/// wrapper's private fields; this exercises decoder policy and source checks.
fn unchecked_snapshot_bytes(
    profile: CompletenessProfile,
    source_anchor: StateAnchor,
    snapshot: &WorldSnapshot,
) -> Vec<u8> {
    let mut bytes = Vec::new();
    put_str(&mut bytes, SNAPSHOT_DOMAIN);
    encode_identity(&mut bytes, profile, &provenance(), &BTreeMap::new());
    put_anchor(&mut bytes, source_anchor);
    put_bytes(&mut bytes, &snapshot.canonical_bytes());
    bytes
}

#[test]
fn inconsistent_facts_and_forged_projection_policy_are_rejected() -> Result<()> {
    let source = source();
    for kind in 0..5 {
        let mut invalid_source = source.clone();
        let fact = invalid_source
            .graph
            .entities
            .get_mut(&EntityId::new(1))
            .unwrap()
            .fields
            .get_mut("amount")
            .unwrap();
        match kind {
            0 => fact.presence = Some(FactPresence::Known(Value::U64(99))),
            1 => fact.presence = Some(FactPresence::Unknown("unavailable".to_owned())),
            2 => fact.observed_at = GameTick(8),
            3 => {
                *fact = Fact::with_presence(
                    FactPresence::Stale(StateAnchor {
                        fortress_id: FortressId::new(8),
                        ..source.anchor()
                    }),
                    GameTick(7),
                    FactSource::Replay,
                    Digest32::ZERO,
                );
            }
            _ => {
                invalid_source
                    .graph
                    .edges
                    .get_mut(&EdgeId::new(7))
                    .unwrap()
                    .to = EntityId::new(999)
            }
        }
        invalid_source.refresh_hash();
        assert!(projected(&invalid_source, CompletenessProfile::ResearchFull).is_err());
        assert!(
            ProfiledSnapshot::from_canonical_bytes(&unchecked_snapshot_bytes(
                CompletenessProfile::ResearchFull,
                invalid_source.anchor(),
                &invalid_source
            ))
            .is_err()
        );
    }
    let mut broken_hash = source.clone();
    broken_hash.state_hash = Digest32::ZERO;
    assert!(projected(&broken_hash, CompletenessProfile::ResearchFull).is_err());
    assert!(
        ProfiledSnapshot::from_canonical_bytes(&unchecked_snapshot_bytes(
            CompletenessProfile::ControlMinimum,
            source.anchor(),
            &source
        ))
        .is_err()
    );
    let mut bad_anchor = source.anchor();
    bad_anchor.cursor.sequence += 1;
    assert!(
        ProfiledSnapshot::from_canonical_bytes(&unchecked_snapshot_bytes(
            CompletenessProfile::ResearchFull,
            bad_anchor,
            &source
        ))
        .is_err()
    );
    Ok(())
}

#[test]
fn source_authenticity_requires_independently_retained_source() -> Result<()> {
    let original = source();
    let mut changed = original.clone();
    changed
        .graph
        .entities
        .get_mut(&EntityId::new(1))
        .unwrap()
        .fields
        .insert("amount".to_owned(), known(Value::U64(100)));
    changed.refresh_hash();
    assert!(
        ProfiledSnapshot::from_canonical_bytes(&unchecked_snapshot_bytes(
            CompletenessProfile::ResearchFull,
            original.anchor(),
            &changed,
        ))
        .is_err()
    );
    let changed_projection = projected(&changed, CompletenessProfile::Operations)?;
    // Integrity protects the received bytes. Without the canonical source,
    // the decoder cannot authenticate a same-identity source-hash claim.
    let claim = ProfiledSnapshot::from_canonical_bytes(&unchecked_snapshot_bytes(
        CompletenessProfile::Operations,
        original.anchor(),
        changed_projection.snapshot(),
    ))?;
    assert!(claim.snapshot().hash_is_valid());
    assert!(claim.verify_against_source(&original).is_err());
    assert!(claim.verify_against_source(&changed).is_err());
    Ok(())
}

#[test]
fn malformed_or_oversized_envelopes_and_deltas_refuse_before_use() -> Result<()> {
    let source = source();
    let profile = projected(&source, CompletenessProfile::ResearchFull)?;
    let bytes = profile.canonical_bytes()?;
    for end in 0..bytes.len() {
        assert!(ProfiledSnapshot::from_canonical_bytes(&bytes[..end]).is_err());
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(ProfiledSnapshot::from_canonical_bytes(&trailing).is_err());
    let mut bad_version = bytes.clone();
    bad_version[8] ^= 1;
    assert!(ProfiledSnapshot::from_canonical_bytes(&bad_version).is_err());
    let capsule = ProfiledObservationCapsule::between(
        &profile,
        &projected(&successor(&source), profile.profile())?,
        GameTick(8),
    )?;
    let capsule_bytes = capsule.canonical_bytes()?;
    for end in 0..capsule_bytes.len() {
        assert!(
            ProfiledObservationCapsule::from_canonical_bytes(&capsule_bytes[..end], &profile)
                .is_err()
        );
    }
    let delta_bytes = capsule.capsule().delta.canonical_bytes();
    for end in 0..delta_bytes.len() {
        assert!(crate::StateDelta::from_canonical_bytes(&delta_bytes[..end]).is_err());
    }
    let mut delta_trailing = delta_bytes;
    delta_trailing.push(0);
    assert!(crate::StateDelta::from_canonical_bytes(&delta_trailing).is_err());
    let mut bad_provenance = provenance();
    bad_provenance.source_manifest = Digest32::ZERO;
    assert!(
        ProfiledSnapshot::project(&source, profile.profile(), bad_provenance, BTreeMap::new())
            .is_err()
    );
    let mut bad_provenance = provenance();
    bad_provenance.source_schema = "x".repeat(257);
    assert!(
        ProfiledSnapshot::project(&source, profile.profile(), bad_provenance, BTreeMap::new())
            .is_err()
    );
    assert!(
        ProfiledSnapshot::project(
            &source,
            profile.profile(),
            provenance(),
            BTreeMap::from([(
                "optional.large".to_owned(),
                vec![0; MAX_EXTENSION_BYTES + 1]
            )]),
        )
        .is_err()
    );
    let mut deep = source.clone();
    let mut value = Value::Null;
    for _ in 0..crate::canonical_decode::MAX_VALUE_DEPTH + 2 {
        value = Value::List(vec![value]);
    }
    deep.graph
        .entities
        .get_mut(&EntityId::new(1))
        .unwrap()
        .fields
        .insert("deep".to_owned(), known(value));
    // The budget check runs before canonical hashing/encoding or cloning.
    assert_eq!(
        projected(&deep, profile.profile()).unwrap_err().code,
        ErrorCode::BudgetExceeded
    );
    let mut oversized = source.clone();
    oversized
        .graph
        .entities
        .get_mut(&EntityId::new(1))
        .unwrap()
        .label = "x".repeat(MAX_PROFILE_BYTES);
    assert_eq!(
        projected(&oversized, profile.profile()).unwrap_err().code,
        ErrorCode::BudgetExceeded
    );
    let oversized_frame = vec![0; MAX_PROFILE_BYTES + 1];
    assert_eq!(
        ProfiledSnapshot::from_canonical_bytes(&oversized_frame)
            .unwrap_err()
            .code,
        ErrorCode::BudgetExceeded
    );
    Ok(())
}
