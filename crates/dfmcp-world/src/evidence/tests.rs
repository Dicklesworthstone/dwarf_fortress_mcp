use super::*;

use dfmcp_core::{FortressId, ObservationCursor};

use crate::{
    ChunkCoord, CompareOp, CompletenessProfile, EdgeRecord, EntityRecord, FactPresence, MapChunk,
    ProfiledSnapshot, ProjectionProvenance, TerrainRun, WorldGraph,
};

const UNIT: EntityId = EntityId::new(1);
const BUILDING: EntityId = EntityId::new(2);
const EDGE: EdgeId = EdgeId::new(1);

fn source_digest() -> Digest32 {
    Digest32::of_bytes(b"exact admitted observation contents")
}

fn observed_fact(value: Value) -> Fact {
    Fact::known(
        value,
        GameTick(8),
        FactSource::DfhackField("unit.ready".to_owned()),
        source_digest(),
    )
}

fn snapshot_with(fact: Fact) -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    graph.entities.insert(
        UNIT,
        EntityRecord {
            id: UNIT,
            generation: 1,
            revision: 1,
            kind: EntityKind::Unit,
            label: "Urist".to_owned(),
            fields: BTreeMap::from([("ready".to_owned(), fact)]),
        },
    );
    graph.entities.insert(
        BUILDING,
        EntityRecord {
            id: BUILDING,
            generation: 1,
            revision: 1,
            kind: EntityKind::Building,
            label: "Workshop".to_owned(),
            fields: BTreeMap::new(),
        },
    );
    graph.edges.insert(
        EDGE,
        EdgeRecord {
            id: EDGE,
            revision: 1,
            kind: EdgeKind::AssignedTo,
            from: UNIT,
            to: BUILDING,
            fields: BTreeMap::new(),
        },
    );
    let coord = ChunkCoord { x: 0, y: 0, z: 0 };
    graph.chunks.insert(
        coord,
        MapChunk {
            coord,
            revision: 1,
            width: 16,
            height: 16,
            terrain_runs: vec![
                TerrainRun { tile_code: 1, length: 1 },
                TerrainRun { tile_code: 2, length: 255 },
            ],
            sparse_overlays: BTreeMap::new(),
        },
    );
    WorldSnapshot::new(
        FortressId::new(1),
        GameTick(10),
        ObservationCursor { epoch: 3, sequence: 11 },
        true,
        graph,
    )
}

fn field(value: Value) -> Predicate {
    Predicate::FieldCompare {
        entity_id: UNIT,
        field: "ready".to_owned(),
        op: CompareOp::Eq,
        value,
    }
}

fn source_policy(snapshot: &WorldSnapshot) -> EvidencePolicy {
    let mut policy = EvidencePolicy::at(snapshot.anchor());
    policy.all_entities = EvidenceCoverage::Observed;
    policy.sources.insert(EvidenceSource::Observed {
        field: "unit.ready".to_owned(),
        source_digest: source_digest(),
    });
    policy
}

fn region(min_x: i32, max_x: i32) -> MapCuboid {
    MapCuboid {
        min: MapCoord::new(min_x, 0, 0),
        max: MapCoord::new(max_x, 0, 0),
    }
}

fn assert_unknown_logic(evidence: &PredicateEvidence<'_>, leaf: Predicate) -> Result<()> {
    for predicate in [
        leaf.clone(),
        Predicate::Not(Box::new(leaf.clone())),
        Predicate::All(vec![Predicate::True, leaf.clone()]),
        Predicate::Any(vec![Predicate::False, leaf]),
    ] {
        assert_eq!(evidence.evaluate(&predicate)?, PredicateTruth::Unknown);
        assert!(!evidence.establishes(&predicate)?);
    }
    Ok(())
}

#[test]
fn default_scope_cannot_promote_canonical_contents_into_authority() -> Result<()> {
    let snapshot = snapshot_with(observed_fact(Value::Bool(true)));
    let evidence = PredicateEvidence::untrusted(&snapshot)?;
    for predicate in [
        field(Value::Bool(true)),
        field(Value::Bool(false)),
        Predicate::EntityExists(UNIT),
        Predicate::EntityExists(EntityId::new(99)),
        Predicate::EntityKind { entity_id: UNIT, kind: EntityKind::Unit },
        Predicate::EdgeExists { edge_id: EDGE, kind: None },
        Predicate::EdgeExists { edge_id: EdgeId::new(99), kind: None },
        Predicate::Paused(true),
        Predicate::Paused(false),
        Predicate::RegionTerrain { area: region(0, 0), tile_code: 1 },
        Predicate::RegionTerrain { area: region(0, 0), tile_code: 2 },
    ] {
        assert_unknown_logic(&evidence, predicate)?;
    }
    assert_eq!(evidence.snapshot().anchor(), snapshot.anchor());
    assert!(evidence.establishes(&Predicate::True)?);
    assert!(!evidence.establishes(&Predicate::False)?);
    // Inspection of supplied facts remains a separate, unchanged operation.
    assert!(crate::evaluate(&snapshot, &field(Value::Bool(true))));
    Ok(())
}

#[test]
fn fields_require_both_identity_and_exact_source_grants() -> Result<()> {
    let snapshot = snapshot_with(observed_fact(Value::Bool(true)));
    let mut policy = source_policy(&snapshot);
    policy.all_entities = EvidenceCoverage::Unknown;
    assert_unknown_logic(&PredicateEvidence::scoped(&snapshot, policy)?, field(Value::Bool(true)))?;
    let mut policy = source_policy(&snapshot);
    policy.sources.clear();
    assert_unknown_logic(&PredicateEvidence::scoped(&snapshot, policy)?, field(Value::Bool(true)))?;

    let evidence = PredicateEvidence::scoped(&snapshot, source_policy(&snapshot))?;
    assert_eq!(evidence.evaluate(&field(Value::Bool(true)))?, PredicateTruth::True);
    assert_eq!(evidence.evaluate(&field(Value::Bool(false)))?, PredicateTruth::False);
    assert!(evidence.establishes(&Predicate::Not(Box::new(field(Value::Bool(false)))))?);
    let incomparable = Predicate::FieldCompare {
        entity_id: UNIT,
        field: "ready".to_owned(),
        op: CompareOp::Lt,
        value: Value::U64(0),
    };
    assert_unknown_logic(&evidence, incomparable)?;
    // Known disjunction and conjunction semantics still short-circuit unknowns.
    assert_eq!(evidence.evaluate(&Predicate::Any(vec![
        Predicate::EntityExists(EntityId::new(99)), field(Value::Bool(true)),
    ]))?, PredicateTruth::True);
    assert_eq!(evidence.evaluate(&Predicate::All(vec![
        Predicate::EntityExists(EntityId::new(99)), field(Value::Bool(false)),
    ]))?, PredicateTruth::False);
    Ok(())
}

#[test]
fn general_source_grants_reject_variant_name_digest_and_future_forgery() -> Result<()> {
    for (source, digest, observed_at) in [
        (FactSource::DfhackField("unit.other".to_owned()), source_digest(), GameTick(8)),
        (FactSource::DfhackField("unit.ready".to_owned()), Digest32::ZERO, GameTick(8)),
        (FactSource::DfhackField("unit.ready".to_owned()), Digest32::of_bytes(b"wrong"), GameTick(8)),
        (FactSource::Derived("unit.ready".to_owned()), source_digest(), GameTick(8)),
        (FactSource::AgentAssertion("unit.ready".to_owned()), source_digest(), GameTick(8)),
        (FactSource::Replay, source_digest(), GameTick(8)),
        (FactSource::DfhackField("unit.ready".to_owned()), source_digest(), GameTick(11)),
    ] {
        let snapshot = snapshot_with(Fact::known(Value::Bool(true), observed_at, source, digest));
        let evidence = PredicateEvidence::scoped(&snapshot, source_policy(&snapshot))?;
        assert_unknown_logic(&evidence, field(Value::Bool(true)))?;
        assert_unknown_logic(&evidence, field(Value::Bool(false)))?;
        assert!(crate::evaluate(&snapshot, &field(Value::Bool(true))));
    }
    Ok(())
}

#[test]
fn derived_grant_is_exact_and_cannot_authorize_an_assertion_or_replay() -> Result<()> {
    for (source, expected) in [
        (FactSource::Derived("registered.formula/1".to_owned()), PredicateTruth::True),
        (FactSource::Derived("registered.formula/2".to_owned()), PredicateTruth::Unknown),
        (FactSource::AgentAssertion("registered.formula/1".to_owned()), PredicateTruth::Unknown),
        (FactSource::DfhackField("registered.formula/1".to_owned()), PredicateTruth::Unknown),
        (FactSource::Replay, PredicateTruth::Unknown),
    ] {
        let snapshot = snapshot_with(Fact::known(Value::Bool(true), GameTick(10), source, source_digest()));
        let mut policy = EvidencePolicy::at(snapshot.anchor());
        policy.all_entities = EvidenceCoverage::Observed;
        policy.sources.insert(EvidenceSource::CertifiedDerived {
            derivation: "registered.formula/1".to_owned(), source_digest: source_digest(),
        });
        let evidence = PredicateEvidence::scoped(&snapshot, policy)?;
        assert_eq!(evidence.evaluate(&field(Value::Bool(true)))?, expected);
    }
    Ok(())
}

#[test]
fn laboratory_inputs_require_registered_current_consistent_known_facts() -> Result<()> {
    for name in [LAB_SCENARIO, LAB_EFFECTS] {
        let fact = Fact::known(Value::Bool(true), GameTick(8), FactSource::Derived(name.to_owned()), Digest32::ZERO);
        assert_eq!(laboratory_fact_value(&fact, GameTick(10)), Some(&Value::Bool(true)));
        assert!(lab_fact_is_eligible(&fact, GameTick(10)));
        let snapshot = snapshot_with(fact.clone());
        let evidence = PredicateEvidence::laboratory(&snapshot)?;
        assert!(evidence.establishes(&field(Value::Bool(true)))?);
        assert_unknown_logic(&PredicateEvidence::untrusted(&snapshot)?, field(Value::Bool(true)))?;

        for presence in [
            FactPresence::Absent,
            FactPresence::Unknown("not observed".to_owned()),
            FactPresence::Unsupported("unsupported".to_owned()),
            FactPresence::Omitted("research-full".to_owned()),
            FactPresence::Redacted("policy".to_owned()),
            FactPresence::Stale(snapshot.anchor()),
            FactPresence::Known(Value::Bool(false)),
        ] {
            let mut unavailable = fact.clone();
            unavailable.presence = Some(presence);
            assert!(!lab_fact_is_eligible(&unavailable, GameTick(10)));
            let snapshot = snapshot_with(unavailable);
            assert_unknown_logic(&PredicateEvidence::laboratory(&snapshot)?, field(Value::Bool(true)))?;
        }
        let mut consistent = fact;
        consistent.presence = Some(FactPresence::Known(Value::Bool(true)));
        assert!(lab_fact_is_eligible(&consistent, GameTick(10)));
        consistent.observed_at = GameTick(11);
        assert!(!lab_fact_is_eligible(&consistent, GameTick(10)));
    }
    Ok(())
}

#[test]
fn laboratory_does_not_accept_arbitrary_source_labels_or_same_value_assertions() -> Result<()> {
    for (source, digest) in [
        (FactSource::Derived("test".to_owned()), Digest32::ZERO),
        (FactSource::Derived(format!("{LAB_EFFECTS}.forged")), Digest32::ZERO),
        (FactSource::Derived(LAB_EFFECTS.to_owned()), source_digest()),
        (FactSource::AgentAssertion(LAB_EFFECTS.to_owned()), Digest32::ZERO),
        (FactSource::DfhackField(LAB_EFFECTS.to_owned()), Digest32::ZERO),
        (FactSource::DfhackField("unit.ready".to_owned()), source_digest()),
        (FactSource::Replay, Digest32::ZERO),
    ] {
        let fact = Fact::known(Value::Bool(true), GameTick(8), source, digest);
        assert!(!lab_fact_is_eligible(&fact, GameTick(10)));
        let snapshot = snapshot_with(fact);
        let evidence = PredicateEvidence::laboratory(&snapshot)?;
        assert_unknown_logic(&evidence, field(Value::Bool(true)))?;
        assert_unknown_logic(&evidence, field(Value::Bool(false)))?;
    }
    Ok(())
}

#[test]
fn complete_kind_and_observed_membership_do_not_imply_global_entity_absence() -> Result<()> {
    let snapshot = snapshot_with(observed_fact(Value::Bool(true)));
    let missing = EntityId::new(99);
    let missing_unit = Predicate::EntityKind { entity_id: missing, kind: EntityKind::Unit };
    let mut policy = EvidencePolicy::at(snapshot.anchor());
    policy.entity_kinds.insert(EntityKind::Unit, EvidenceCoverage::Observed);
    let evidence = PredicateEvidence::scoped(&snapshot, policy.clone())?;
    assert!(evidence.establishes(&Predicate::EntityExists(UNIT))?);
    assert_unknown_logic(&evidence, Predicate::EntityExists(BUILDING))?;
    assert_unknown_logic(&evidence, Predicate::EntityExists(missing))?;
    assert_unknown_logic(&evidence, missing_unit.clone())?;
    assert_eq!(evidence.evaluate(&Predicate::EntityKind { entity_id: UNIT, kind: EntityKind::Building })?, PredicateTruth::False);

    policy.entity_kinds.insert(EntityKind::Unit, EvidenceCoverage::Complete);
    let evidence = PredicateEvidence::scoped(&snapshot, policy.clone())?;
    assert_eq!(evidence.evaluate(&missing_unit)?, PredicateTruth::False);
    assert!(evidence.establishes(&Predicate::Not(Box::new(missing_unit)))?);
    assert_unknown_logic(&evidence, Predicate::EntityExists(missing))?;
    assert_unknown_logic(&evidence, Predicate::EntityKind { entity_id: missing, kind: EntityKind::Building })?;
    policy.all_entities = EvidenceCoverage::Complete;
    let evidence = PredicateEvidence::scoped(&snapshot, policy)?;
    assert_eq!(evidence.evaluate(&Predicate::EntityExists(missing))?, PredicateTruth::False);
    assert_unknown_logic(&evidence, Predicate::EntityExists(EntityId::NIL))?;
    Ok(())
}

#[test]
fn relations_need_observed_endpoints_and_exact_absence_domains() -> Result<()> {
    let snapshot = snapshot_with(observed_fact(Value::Bool(true)));
    let missing = EdgeId::new(99);
    let qualified_missing = Predicate::EdgeExists { edge_id: missing, kind: Some(EdgeKind::AssignedTo) };
    let mut policy = EvidencePolicy::at(snapshot.anchor());
    policy.edge_kinds.insert(EdgeKind::AssignedTo, EvidenceCoverage::Observed);
    let evidence = PredicateEvidence::scoped(&snapshot, policy.clone())?;
    assert_unknown_logic(&evidence, Predicate::EdgeExists { edge_id: EDGE, kind: None })?;
    policy.all_entities = EvidenceCoverage::Observed;
    let evidence = PredicateEvidence::scoped(&snapshot, policy.clone())?;
    assert!(evidence.establishes(&Predicate::EdgeExists { edge_id: EDGE, kind: None })?);
    assert_eq!(evidence.evaluate(&Predicate::EdgeExists { edge_id: EDGE, kind: Some(EdgeKind::MemberOf) })?, PredicateTruth::False);
    assert_unknown_logic(&evidence, qualified_missing.clone())?;
    policy.edge_kinds.insert(EdgeKind::AssignedTo, EvidenceCoverage::Complete);
    let evidence = PredicateEvidence::scoped(&snapshot, policy.clone())?;
    assert_eq!(evidence.evaluate(&qualified_missing)?, PredicateTruth::False);
    assert_unknown_logic(&evidence, Predicate::EdgeExists { edge_id: missing, kind: None })?;
    assert_unknown_logic(&evidence, Predicate::EdgeExists { edge_id: missing, kind: Some(EdgeKind::MemberOf) })?;
    policy.all_edges = EvidenceCoverage::Complete;
    let evidence = PredicateEvidence::scoped(&snapshot, policy)?;
    assert_eq!(evidence.evaluate(&Predicate::EdgeExists { edge_id: missing, kind: None })?, PredicateTruth::False);
    assert_unknown_logic(&evidence, Predicate::EdgeExists { edge_id: EdgeId::NIL, kind: None })?;

    let mut dangling = snapshot.clone();
    dangling.graph.entities.remove(&BUILDING);
    dangling.refresh_hash();
    assert_unknown_logic(&PredicateEvidence::laboratory(&dangling)?, Predicate::EdgeExists { edge_id: EDGE, kind: None })?;
    Ok(())
}

#[test]
fn terrain_requires_covered_tiles_and_retains_partial_counterexample_semantics() -> Result<()> {
    let snapshot = snapshot_with(observed_fact(Value::Bool(true)));
    let both_floor = Predicate::RegionTerrain { area: region(0, 1), tile_code: 1 };
    let mut policy = EvidencePolicy::at(snapshot.anchor());
    policy.terrain_regions.push(region(0, 0));
    let evidence = PredicateEvidence::scoped(&snapshot, policy.clone())?;
    assert!(evidence.establishes(&Predicate::RegionTerrain { area: region(0, 0), tile_code: 1 })?);
    assert_unknown_logic(&evidence, both_floor.clone())?;

    policy.terrain_regions = vec![region(1, 1)];
    let evidence = PredicateEvidence::scoped(&snapshot, policy)?;
    assert_eq!(evidence.evaluate(&both_floor)?, PredicateTruth::False);
    assert!(evidence.establishes(&Predicate::Not(Box::new(both_floor)))?);
    let laboratory = PredicateEvidence::laboratory(&snapshot)?;
    assert_unknown_logic(&laboratory, Predicate::RegionTerrain { area: region(16, 16), tile_code: 1 })?;
    assert!(laboratory.establishes(&Predicate::Paused(true))?);

    let mut malformed = snapshot.clone();
    if let Some(chunk) = malformed.graph.chunks.values_mut().next() {
        chunk.width = 1;
    }
    malformed.refresh_hash();
    assert_unknown_logic(&PredicateEvidence::laboratory(&malformed)?, Predicate::RegionTerrain { area: region(0, 0), tile_code: 1 })?;
    Ok(())
}

#[test]
fn known_null_is_evidence_but_absent_or_inconsistent_presence_is_not() -> Result<()> {
    let known_null = snapshot_with(observed_fact(Value::Null));
    assert!(PredicateEvidence::scoped(&known_null, source_policy(&known_null))?.establishes(&field(Value::Null))?);
    let mut absent = observed_fact(Value::Null);
    absent.presence = Some(FactPresence::Absent);
    let snapshot = snapshot_with(absent);
    assert_unknown_logic(&PredicateEvidence::scoped(&snapshot, source_policy(&snapshot))?, field(Value::Null))?;
    let mut inconsistent = observed_fact(Value::Bool(true));
    inconsistent.presence = Some(FactPresence::Known(Value::Bool(false)));
    let snapshot = snapshot_with(inconsistent);
    assert_unknown_logic(&PredicateEvidence::scoped(&snapshot, source_policy(&snapshot))?, field(Value::Bool(true)))?;
    assert_unknown_logic(&PredicateEvidence::scoped(&snapshot, source_policy(&snapshot))?, field(Value::Bool(false)))?;
    Ok(())
}

#[test]
fn scope_rejects_changed_anchors_hashes_and_unhashed_lookup_key_aliases() -> Result<()> {
    let snapshot = snapshot_with(observed_fact(Value::Bool(true)));
    assert!(matches!(PredicateEvidence::scoped(&snapshot, EvidencePolicy::default()), Err(error) if error.code == ErrorCode::StaleAnchor));
    for index in 0..5 {
        let mut policy = source_policy(&snapshot);
        let mut anchor = snapshot.anchor();
        match index {
            0 => anchor.fortress_id = FortressId::new(2),
            1 => anchor.cursor.epoch += 1,
            2 => anchor.cursor.sequence += 1,
            3 => anchor.tick = GameTick(11),
            _ => anchor.state_hash = Digest32::of_bytes(b"different source"),
        }
        policy.anchor = Some(anchor);
        assert!(matches!(PredicateEvidence::scoped(&snapshot, policy), Err(error) if error.code == ErrorCode::StaleAnchor));
    }
    let mut corrupt = snapshot.clone();
    corrupt.paused = false;
    assert!(matches!(PredicateEvidence::untrusted(&corrupt), Err(error) if error.code == ErrorCode::ChecksumMismatch));
    let mut alias = snapshot.clone();
    if let Some(record) = alias.graph.entities.remove(&UNIT) {
        alias.graph.entities.insert(EntityId::new(99), record);
    }
    alias.refresh_hash();
    assert!(matches!(PredicateEvidence::laboratory(&alias), Err(error) if error.code == ErrorCode::InternalInvariantViolation));
    Ok(())
}

#[test]
fn profile_names_manifests_and_projected_bytes_do_not_issue_evidence_scopes() -> Result<()> {
    let snapshot = snapshot_with(observed_fact(Value::Bool(true)));
    for profile in CompletenessProfile::ALL {
        let projected = ProfiledSnapshot::project(
            &snapshot,
            profile,
            ProjectionProvenance {
                source_schema: "dfmcp-world-snapshot-v1".to_owned(),
                source_manifest: source_digest(),
            },
            BTreeMap::new(),
        )?;
        let evidence = PredicateEvidence::untrusted(projected.snapshot())?;
        assert_unknown_logic(&evidence, field(Value::Bool(true)))?;
        assert_unknown_logic(&evidence, Predicate::EntityExists(EntityId::new(99)))?;
        if projected.snapshot().anchor() != snapshot.anchor() {
            assert!(matches!(PredicateEvidence::scoped(projected.snapshot(), source_policy(&snapshot)), Err(error) if error.code == ErrorCode::StaleAnchor));
        }
    }
    Ok(())
}

#[test]
fn invalid_or_oversized_policies_and_predicate_branches_fail_before_evaluation() -> Result<()> {
    let snapshot = snapshot_with(observed_fact(Value::Bool(true)));
    for source in [
        EvidenceSource::Observed { field: "unit.ready".to_owned(), source_digest: Digest32::ZERO },
        EvidenceSource::CertifiedDerived { derivation: LAB_EFFECTS.to_owned(), source_digest: Digest32::ZERO },
        EvidenceSource::Observed { field: String::new(), source_digest: source_digest() },
        EvidenceSource::Observed { field: "unit\0ready".to_owned(), source_digest: source_digest() },
        EvidenceSource::Observed { field: "a".repeat(MAX_EVIDENCE_SOURCE_BYTES + 1), source_digest: source_digest() },
    ] {
        let mut policy = EvidencePolicy::at(snapshot.anchor());
        policy.sources.insert(source);
        assert!(matches!(PredicateEvidence::scoped(&snapshot, policy), Err(error) if error.code == ErrorCode::InvalidRequest));
    }
    let mut too_many = source_policy(&snapshot);
    too_many.sources = (0..=MAX_EVIDENCE_SOURCES).map(|index| EvidenceSource::Observed {
        field: format!("source.{index}"), source_digest: source_digest(),
    }).collect();
    assert!(matches!(PredicateEvidence::scoped(&snapshot, too_many), Err(error) if error.code == ErrorCode::BudgetExceeded));
    let mut too_many = EvidencePolicy::at(snapshot.anchor());
    too_many.entity_kinds = (0..=MAX_EVIDENCE_KINDS).map(|index| (
        EntityKind::Other(format!("kind.{index}")), EvidenceCoverage::Observed,
    )).collect();
    assert!(matches!(PredicateEvidence::scoped(&snapshot, too_many), Err(error) if error.code == ErrorCode::BudgetExceeded));
    let mut too_many = EvidencePolicy::at(snapshot.anchor());
    too_many.terrain_regions = vec![region(0, 0); MAX_EVIDENCE_TERRAIN_REGIONS + 1];
    assert!(matches!(PredicateEvidence::scoped(&snapshot, too_many), Err(error) if error.code == ErrorCode::BudgetExceeded));
    let mut invalid_area = EvidencePolicy::at(snapshot.anchor());
    invalid_area.terrain_regions.push(region(2, 1));
    assert!(PredicateEvidence::scoped(&snapshot, invalid_area).is_err());

    let evidence = PredicateEvidence::laboratory(&snapshot)?;
    let mut deep = field(Value::Bool(true));
    for _ in 0..65 {
        deep = Predicate::Not(Box::new(deep));
    }
    for invalid in [
        deep,
        Predicate::RegionTerrain { area: region(0, 65_536), tile_code: 1 },
    ] {
        // Even an otherwise trivially true branch cannot bypass shape budgets.
        assert!(matches!(evidence.evaluate(&Predicate::Any(vec![Predicate::True, invalid])), Err(error) if error.code == ErrorCode::BudgetExceeded));
    }
    Ok(())
}

#[test]
fn canonical_kind_aliases_preserve_authoritative_truth_and_coverage() -> Result<()> {
    let canonical = snapshot_with(observed_fact(Value::Bool(true)));
    let mut aliases = canonical.clone();
    if let Some(unit) = aliases.graph.entities.get_mut(&UNIT) {
        unit.kind = EntityKind::Other("unit".to_owned());
    }
    if let Some(building) = aliases.graph.entities.get_mut(&BUILDING) {
        building.kind = EntityKind::Other("building".to_owned());
    }
    if let Some(edge) = aliases.graph.edges.get_mut(&EDGE) {
        edge.kind = EdgeKind::Custom("assigned_to".to_owned());
    }
    aliases.refresh_hash();
    assert_eq!(aliases.canonical_bytes(), canonical.canonical_bytes());
    assert_eq!(aliases.anchor(), canonical.anchor());

    for snapshot in [&canonical, &aliases] {
        for grant_aliases in [false, true] {
            let mut policy = source_policy(snapshot);
            policy.all_entities = EvidenceCoverage::Unknown;
            policy.entity_kinds.insert(
                if grant_aliases { EntityKind::Other("unit".to_owned()) } else { EntityKind::Unit },
                EvidenceCoverage::Complete,
            );
            policy.entity_kinds.insert(
                if grant_aliases { EntityKind::Other("building".to_owned()) } else { EntityKind::Building },
                EvidenceCoverage::Observed,
            );
            policy.edge_kinds.insert(
                if grant_aliases { EdgeKind::Custom("assigned_to".to_owned()) } else { EdgeKind::AssignedTo },
                EvidenceCoverage::Complete,
            );
            let scoped = PredicateEvidence::scoped(snapshot, policy)?;
            let laboratory = PredicateEvidence::laboratory(snapshot)?;
            for evidence in [&scoped, &laboratory] {
                for positive in [
                    Predicate::EntityKind { entity_id: UNIT, kind: EntityKind::Unit },
                    Predicate::EntityKind { entity_id: UNIT, kind: EntityKind::Other("unit".to_owned()) },
                    Predicate::EdgeExists { edge_id: EDGE, kind: Some(EdgeKind::AssignedTo) },
                    Predicate::EdgeExists { edge_id: EDGE, kind: Some(EdgeKind::Custom("assigned_to".to_owned())) },
                ] {
                    assert_eq!(evidence.evaluate(&positive)?, PredicateTruth::True);
                    assert_eq!(evidence.evaluate(&Predicate::Not(Box::new(positive)))?, PredicateTruth::False);
                }
                assert_eq!(evidence.evaluate(&Predicate::EntityKind { entity_id: EntityId::new(99), kind: EntityKind::Unit })?, PredicateTruth::False);
                assert_eq!(evidence.evaluate(&Predicate::EdgeExists { edge_id: EdgeId::new(99), kind: Some(EdgeKind::AssignedTo) })?, PredicateTruth::False);
            }
            assert!(scoped.establishes(&field(Value::Bool(true)))?);
            // Complete unit and relation scopes retain their original limits.
            assert_unknown_logic(&scoped, Predicate::EntityExists(EntityId::new(99)))?;
            assert_unknown_logic(&scoped, Predicate::EdgeExists { edge_id: EdgeId::new(99), kind: None })?;
        }
    }
    Ok(())
}

#[test]
fn duplicate_canonical_policy_grants_are_rejected_before_interpreting_coverage() -> Result<()> {
    let snapshot = snapshot_with(observed_fact(Value::Bool(true)));
    for coverage in [EvidenceCoverage::Unknown, EvidenceCoverage::Observed, EvidenceCoverage::Complete] {
        let mut entities = EvidencePolicy::at(snapshot.anchor());
        entities.entity_kinds.insert(EntityKind::Unit, EvidenceCoverage::Observed);
        entities.entity_kinds.insert(EntityKind::Other("unit".to_owned()), coverage);
        assert!(matches!(PredicateEvidence::scoped(&snapshot, entities), Err(error) if error.code == ErrorCode::InvalidRequest && error.message.contains("duplicate canonical")));

        let mut edges = EvidencePolicy::at(snapshot.anchor());
        edges.edge_kinds.insert(EdgeKind::AssignedTo, EvidenceCoverage::Observed);
        edges.edge_kinds.insert(EdgeKind::Custom("assigned_to".to_owned()), coverage);
        assert!(matches!(PredicateEvidence::scoped(&snapshot, edges), Err(error) if error.code == ErrorCode::InvalidRequest && error.message.contains("duplicate canonical")));
    }
    Ok(())
}
