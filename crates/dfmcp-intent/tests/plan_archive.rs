//! Original-plan recovery, not dispatch qualification.
use dfmcp_core::{
    Capability, Digest32, EdgeId, EntityId, ErrorCode, FortressId, GameTick, IntentId, MapCoord,
    MapCuboid, ObservationCursor, Result, RiskTier, StateAnchor, StepId,
};
use dfmcp_intent::plan_archive::{self as archive, ArchiveLimits, PlanArchiveSeal};
use dfmcp_intent::{
    Action, BuildingKind, DigMode, FurnitureKind, MaterialSelector, ObligationSpec, PlanStep,
    PreparedPlan, WorkOrderCondition, derive_step_idempotency_key,
};
use dfmcp_world::{CompareOp, EdgeKind, EntityKind, Predicate, Value};
use std::collections::{BTreeMap, BTreeSet};

fn anchor() -> StateAnchor {
    StateAnchor {
        fortress_id: FortressId::new(42),
        cursor: ObservationCursor {
            epoch: 3,
            sequence: 7,
        },
        tick: GameTick(100),
        state_hash: Digest32::of_bytes(b"archive-source"),
    }
}
fn make(actions: Vec<Action>, goal: Predicate, checkpoint: bool) -> PreparedPlan {
    let intent = IntentId::new(9);
    let mut maximum = RiskTier::ReadOnly;
    let mut capabilities = BTreeSet::new();
    let mut steps = Vec::new();
    for (i, action) in actions.into_iter().enumerate() {
        let action = action.normalized();
        let id = StepId::new(i as u32);
        let risk = action.risk();
        let required_capability = action.capability();
        maximum = maximum.max(risk);
        capabilities.insert(required_capability);
        let obligation = action.naturally_temporal().then(|| ObligationSpec {
            terminal: Predicate::EntityExists(EntityId::new(7)),
            failure: Some(Predicate::Not(Box::new(Predicate::EntityExists(
                EntityId::new(8),
            )))),
            deadline_tick: GameTick(180),
            poll_interval_ticks: 5,
            stable_for_observations: 2,
        });
        steps.push(PlanStep {
            id,
            idempotency_key: derive_step_idempotency_key(intent, anchor(), id, &action),
            action,
            preconditions: vec![Predicate::Paused(true)],
            postconditions: vec![goal.clone().normalized()],
            compensation: Some(Action::Pause { paused: true }),
            obligation,
            depends_on: if i == 0 {
                vec![]
            } else {
                vec![StepId::new(i as u32 - 1)]
            },
            risk,
            required_capability,
        });
    }
    if checkpoint {
        capabilities.insert(Capability::Checkpoint);
    }
    PreparedPlan::builder(intent, anchor(), "Restore the original pause plan", goal)
        .steps(steps)
        .max_risk(maximum)
        .required_capabilities(capabilities)
        .requires_checkpoint(checkpoint)
        .expires_at_tick(GameTick(200))
        .build()
}
fn pause() -> PreparedPlan {
    make(
        vec![Action::Pause { paused: false }],
        Predicate::Paused(false),
        false,
    )
}
fn roundtrip(plan: &PreparedPlan) -> Result<(Vec<u8>, PlanArchiveSeal)> {
    let (bytes, seal) = archive::encode(plan, ArchiveLimits::default())?;
    assert_eq!(seal.plan_digest, plan.digest);
    let recovered = archive::decode(&bytes, seal, ArchiveLimits::default())?;
    assert_eq!(&recovered, plan);
    assert_eq!(
        archive::encode(&recovered, ArchiveLimits::default())?,
        (bytes.clone(), seal)
    );
    Ok((bytes, seal))
}
fn area() -> MapCuboid {
    MapCuboid {
        min: MapCoord::new(1, 2, 3),
        max: MapCoord::new(2, 3, 3),
    }
}
fn resigned(bytes: &[u8], semantic: Digest32) -> (Vec<u8>, PlanArchiveSeal) {
    let mut bytes = bytes.to_vec();
    let end = bytes.len() - 32;
    let mut preimage = b"dfmcp-exact-plan-archive/1\0".to_vec();
    preimage.extend_from_slice(&bytes[..end]);
    let object_digest = Digest32::of_bytes(&preimage);
    bytes[end..].copy_from_slice(object_digest.as_bytes());
    (
        bytes,
        PlanArchiveSeal {
            plan_digest: semantic,
            object_digest,
        },
    )
}

#[test]
fn original_identity_expiry_compensation_and_goal_survive_exactly() -> Result<()> {
    let plan = pause();
    roundtrip(&plan)?;
    assert_eq!(plan.anchor, anchor());
    assert_eq!(plan.expires_at_tick, GameTick(200));
    Ok(())
}
#[test]
fn all_action_families_and_all_building_and_dig_variants_roundtrip() -> Result<()> {
    let mut actions = vec![
        Action::Pause { paused: false },
        Action::SetLabor {
            units: vec![EntityId::new(4), EntityId::new(2)],
            labor: "MINING".into(),
            enabled: true,
        },
        Action::CreateWorkOrder {
            name: "beds".into(),
            job_token: "CONSTRUCT_BED".into(),
            amount: 7,
            conditions: vec![
                WorkOrderCondition::CompletedOrder {
                    order_name: "timber".into(),
                },
                WorkOrderCondition::ItemCountBelow {
                    item_token: "BED".into(),
                    threshold: 20,
                },
                WorkOrderCondition::MaterialAvailable {
                    material_token: "WOOD".into(),
                    minimum: 7,
                },
            ],
        },
        Action::ConfigureStockpile {
            stockpile: EntityId::new(8),
            accepts: BTreeSet::from(["WOOD".into(), "STONE".into()]),
            max_bins: Some(0),
            max_barrels: None,
            max_wheelbarrows: Some(u32::MAX),
        },
        Action::AssignSquad {
            units: vec![EntityId::new(2)],
            squad: EntityId::new(3),
        },
        Action::SetBurrowMembership {
            units: vec![EntityId::new(2)],
            burrow: EntityId::new(4),
            assigned: false,
        },
        Action::SetStandingOrder {
            key: "gather".into(),
            value: "wood".into(),
        },
        Action::Extension {
            namespace: "registered".into(),
            name: "opaque".into(),
            parameters: BTreeMap::from([
                ("a".into(), "text is data".into()),
                ("b".into(), "λ".into()),
            ]),
        },
    ];
    for mode in [
        DigMode::Mine,
        DigMode::Channel,
        DigMode::UpStair,
        DigMode::DownStair,
        DigMode::UpDownStair,
        DigMode::Ramp,
        DigMode::RemoveConstruction,
    ] {
        actions.push(Action::DesignateDig { area: area(), mode });
    }
    for kind in [
        BuildingKind::Workshop("CARPENTER".into()),
        BuildingKind::Furnace("SMELTER".into()),
        BuildingKind::Furniture("BED".into()),
        BuildingKind::Construction("FLOOR".into()),
        BuildingKind::Trap("CAGE".into()),
        BuildingKind::FarmPlot,
        BuildingKind::Bridge,
        BuildingKind::Well,
        BuildingKind::Custom("opaque".into()),
    ] {
        actions.push(Action::Build {
            kind,
            location: area().min,
            footprint: area(),
            material: MaterialSelector {
                required_tokens: BTreeSet::from(["WOOD".into()]),
                forbidden_tokens: BTreeSet::from(["MAGMA".into()]),
                prefer_nearest: true,
                reserve_count: 3,
            },
        });
    }
    let plan = make(actions, Predicate::Paused(false), true);
    roundtrip(&plan)?;
    assert_eq!(plan.steps.len(), 24);
    assert!(plan.requires_checkpoint);
    Ok(())
}

#[test]
fn exact_item_placement_roundtrips_all_kinds_and_rejects_unknown_tags() -> Result<()> {
    for kind in [
        FurnitureKind::Bed,
        FurnitureKind::Door,
        FurnitureKind::Table,
        FurnitureKind::Chair,
    ] {
        let plan = make(
            vec![Action::PlaceFurniture {
                kind,
                item: EntityId::new(u64::MAX),
                location: MapCoord::new(-3, 7, 2),
            }],
            Predicate::Paused(false),
            false,
        );
        let (bytes, seal) = roundtrip(&plan)?;
        // Header, original identities/anchor, summary, goal, step count and ID.
        let action_offset = 12 + 16 + 16 + 64 + 4 + plan.summary.len() + 2 + 4 + 4;
        assert_eq!(bytes[action_offset], 10);
        let mut damaged = bytes;
        damaged[action_offset + 1] = 255;
        let (damaged, expected) = resigned(&damaged, seal.plan_digest);
        assert!(archive::decode(&damaged, expected, ArchiveLimits::default()).is_err());
    }
    Ok(())
}
#[test]
fn literal_variants_and_nested_boolean_predicates_roundtrip() -> Result<()> {
    let values = vec![
        Value::Null,
        Value::Bool(true),
        Value::I64(i64::MIN),
        Value::U64(u64::MAX),
        Value::Fixed {
            units: -31,
            scale: 100,
        },
        Value::Text("Urist λ".into()),
        Value::Entity(EntityId::new(33)),
        Value::Coord(MapCoord::new(-1, 2, 3)),
        Value::Bytes(vec![0, 255, 7]),
        Value::List(vec![Value::Null, Value::Bool(false)]),
        Value::Object(BTreeMap::from([(
            "a".into(),
            Value::List(vec![Value::U64(7)]),
        )])),
    ];
    for value in values {
        for op in [
            CompareOp::Eq,
            CompareOp::Ne,
            CompareOp::Lt,
            CompareOp::Le,
            CompareOp::Gt,
            CompareOp::Ge,
        ] {
            let p = Predicate::FieldCompare {
                entity_id: EntityId::new(7),
                field: "observed".into(),
                op,
                value: value.clone(),
            };
            let goal = Predicate::All(vec![
                p,
                Predicate::Any(vec![
                    Predicate::Paused(false),
                    Predicate::Not(Box::new(Predicate::EntityExists(EntityId::new(3)))),
                ]),
            ]);
            roundtrip(&make(vec![Action::Pause { paused: false }], goal, false))?;
        }
    }
    for goal in [
        Predicate::EntityExists(EntityId::new(1)),
        Predicate::RegionTerrain {
            area: area(),
            tile_code: 2,
        },
        Predicate::EdgeExists {
            edge_id: EdgeId::new(u128::MAX),
            kind: None,
        },
        Predicate::EdgeExists {
            edge_id: EdgeId::new(2),
            kind: Some(EdgeKind::Custom("requires".into())),
        },
    ] {
        roundtrip(&make(vec![Action::Pause { paused: false }], goal, false))?;
    }
    Ok(())
}

#[test]
fn laboratory_room_access_appends_a_lossless_predicate_tag() -> Result<()> {
    let region = MapCuboid {
        min: MapCoord::new(-4, -2, 3),
        max: MapCoord::new(1, 2, 3),
    };
    let access = Predicate::LaboratoryRoomAccess {
        area: region,
        entry: region.min,
        targets: vec![MapCoord::new(1, 2, 3), MapCoord::new(-1, 0, 3)],
    };
    let plan = make(vec![Action::Pause { paused: false }], access, false);
    let (bytes, _) = roundtrip(&plan)?;
    let predicate = 12 + 16 + 16 + 64 + 4 + plan.summary.len();
    assert_eq!(bytes[predicate], 11);
    assert_eq!(&bytes[predicate + 37..predicate + 41], &2u32.to_be_bytes());
    assert_eq!(
        plan.steps[0].postconditions,
        vec![plan.terminal_condition.normalized()]
    );
    Ok(())
}

#[test]
fn paired_stair_access_appends_tag_thirteen_and_preserves_original_obligation() -> Result<()> {
    let region = MapCuboid {
        min: MapCoord::new(-4, -2, 2),
        max: MapCoord::new(1, 2, 4),
    };
    let goal = Predicate::LaboratoryRoomAccessPairedStairs {
        area: region,
        entry: region.min,
        targets: vec![MapCoord::new(1, 2, 4), MapCoord::new(-1, 0, 3)],
    };
    let mut plan = make(
        vec![Action::DesignateDig {
            area: MapCuboid {
                min: MapCoord::new(0, 0, 2),
                max: MapCoord::new(0, 0, 4),
            },
            mode: DigMode::UpDownStair,
        }],
        goal.clone(),
        false,
    );
    if let Some(obligation) = &mut plan.steps[0].obligation {
        obligation.terminal = goal.normalized();
    }
    plan.digest = plan.compute_digest();
    plan.id = plan.expected_id();
    let (bytes, _) = roundtrip(&plan)?;
    let predicate = 12 + 16 + 16 + 64 + 4 + plan.summary.len();
    assert_eq!(bytes[predicate], 13);
    assert_eq!(&bytes[predicate + 37..predicate + 41], &2u32.to_be_bytes());
    assert_eq!(
        plan.steps[0].obligation.as_ref().map(|obligation| (
            obligation.deadline_tick,
            obligation.poll_interval_ticks,
            obligation.stable_for_observations,
        )),
        Some((GameTick(180), 5, 2)),
    );
    Ok(())
}

#[test]
fn paired_stair_archive_keeps_legacy_access_payload_and_policy_identity_distinct() -> Result<()> {
    let region = area();
    let targets = vec![MapCoord::new(1, 3, 3), MapCoord::new(2, 3, 3)];
    let legacy = make(
        vec![Action::Pause { paused: false }],
        Predicate::LaboratoryRoomAccess {
            area: region,
            entry: region.min,
            targets: targets.clone(),
        },
        false,
    );
    let paired = make(
        vec![Action::Pause { paused: false }],
        Predicate::LaboratoryRoomAccessPairedStairs {
            area: region,
            entry: region.min,
            targets: targets.clone(),
        },
        false,
    );
    let (legacy_bytes, legacy_seal) = roundtrip(&legacy)?;
    let (paired_bytes, paired_seal) = roundtrip(&paired)?;
    assert_ne!(legacy_seal.plan_digest, paired_seal.plan_digest);
    assert_ne!(legacy_seal.object_digest, paired_seal.object_digest);
    let predicate = 12 + 16 + 16 + 64 + 4 + legacy.summary.len();
    let mut expected = vec![11];
    for at in [region.min, region.max, region.min] {
        for component in [at.x, at.y, at.z] {
            expected.extend_from_slice(&component.to_be_bytes());
        }
    }
    expected.extend_from_slice(&2u32.to_be_bytes());
    for target in targets {
        for component in [target.x, target.y, target.z] {
            expected.extend_from_slice(&component.to_be_bytes());
        }
    }
    assert_eq!(
        &legacy_bytes[predicate..predicate + expected.len()],
        expected
    );
    expected[0] = 13;
    assert_eq!(
        &paired_bytes[predicate..predicate + expected.len()],
        expected
    );
    Ok(())
}

#[test]
fn paired_stair_archive_refuses_excessive_volume_or_invalid_targets() -> Result<()> {
    let access = Predicate::LaboratoryRoomAccessPairedStairs {
        area: MapCuboid {
            min: MapCoord::new(1, 2, 3),
            max: MapCoord::new(2, 3, 4),
        },
        entry: MapCoord::new(1, 2, 3),
        targets: vec![MapCoord::new(1, 3, 4), MapCoord::new(2, 3, 4)],
    };
    let plan = make(vec![Action::Pause { paused: false }], access, false);
    let (bytes, seal) = roundtrip(&plan)?;
    let predicate = 12 + 16 + 16 + 64 + 4 + plan.summary.len();
    for count in [0u32, 17, u32::MAX] {
        let mut changed = bytes.clone();
        changed[predicate + 37..predicate + 41].copy_from_slice(&count.to_be_bytes());
        let (changed, expected) = resigned(&changed, seal.plan_digest);
        assert!(archive::decode(&changed, expected, ArchiveLimits::default()).is_err());
    }
    for mutation in 0..3 {
        let mut changed = bytes.clone();
        let start = predicate + 41;
        match mutation {
            0 => {
                let first = changed[start..start + 12].to_vec();
                changed[start + 12..start + 24].copy_from_slice(&first);
            }
            1 => changed[start..start + 4].copy_from_slice(&i32::MAX.to_be_bytes()),
            _ => changed[predicate + 21..predicate + 25].copy_from_slice(&1024i32.to_be_bytes()),
        }
        let (changed, expected) = resigned(&changed, seal.plan_digest);
        assert!(archive::decode(&changed, expected, ArchiveLimits::default()).is_err());
    }
    Ok(())
}

#[test]
fn civilian_release_appends_tag_fourteen_and_roundtrips_exact_incarnations() -> Result<()> {
    let exact = |id, generation| dfmcp_world::LaboratoryEntityIdentity {
        id: EntityId::new(id),
        generation,
    };
    let goal = Predicate::LaboratoryCivilianReleaseSafety {
        burrow: exact(20, 5),
        civilians: vec![exact(11, 7), exact(12, 9)],
    };
    let plan = make(vec![Action::Pause { paused: false }], goal.clone(), false);
    let (bytes, _) = roundtrip(&plan)?;
    let predicate = 12 + 16 + 16 + 64 + 4 + plan.summary.len();
    let mut expected = vec![14];
    expected.extend_from_slice(&20u64.to_be_bytes());
    expected.extend_from_slice(&5u32.to_be_bytes());
    expected.extend_from_slice(&2u32.to_be_bytes());
    for (id, generation) in [(11u64, 7u32), (12, 9)] {
        expected.extend_from_slice(&id.to_be_bytes());
        expected.extend_from_slice(&generation.to_be_bytes());
    }
    assert_eq!(&bytes[predicate..predicate + expected.len()], expected);
    let changed = make(
        vec![Action::Pause { paused: false }],
        Predicate::LaboratoryCivilianReleaseSafety {
            burrow: exact(20, 6),
            civilians: vec![exact(11, 7), exact(12, 9)],
        },
        false,
    );
    assert_ne!(plan.digest, changed.digest);
    Ok(())
}

#[test]
fn civilian_release_archive_rejects_invalid_counts_duplicate_ids_and_zero_generations() -> Result<()>
{
    let exact = |id| dfmcp_world::LaboratoryEntityIdentity {
        id: EntityId::new(id),
        generation: 1,
    };
    let plan = make(
        vec![Action::Pause { paused: false }],
        Predicate::LaboratoryCivilianReleaseSafety {
            burrow: exact(20),
            civilians: vec![exact(11), exact(12)],
        },
        false,
    );
    let (bytes, seal) = roundtrip(&plan)?;
    let predicate = 12 + 16 + 16 + 64 + 4 + plan.summary.len();
    for mutation in 0..6 {
        let mut changed = bytes.clone();
        match mutation {
            0..=2 => {
                let count = [0u32, 65, u32::MAX][mutation];
                changed[predicate + 13..predicate + 17].copy_from_slice(&count.to_be_bytes());
            }
            3 => changed[predicate + 9..predicate + 13].copy_from_slice(&0u32.to_be_bytes()),
            4 => changed[predicate + 25..predicate + 29].copy_from_slice(&0u32.to_be_bytes()),
            _ => {
                let first = changed[predicate + 17..predicate + 25].to_vec();
                changed[predicate + 29..predicate + 37].copy_from_slice(&first);
            }
        }
        let (changed, expected) = resigned(&changed, seal.plan_digest);
        assert!(archive::decode(&changed, expected, ArchiveLimits::default()).is_err());
    }
    Ok(())
}

#[test]
fn inventory_quantity_roundtrips_exactly_with_original_obligation_and_tag_twelve() -> Result<()> {
    for quantity_field in [None, Some("stack_size".to_owned())] {
        for op in [
            CompareOp::Eq,
            CompareOp::Ne,
            CompareOp::Lt,
            CompareOp::Le,
            CompareOp::Gt,
            CompareOp::Ge,
        ] {
            let goal = Predicate::EntityQuantity {
                kind: EntityKind::Item,
                filter_field: "type_key".into(),
                filter_value: Value::Text("BED".into()),
                quantity_field: quantity_field.clone(),
                op,
                value: u64::MAX,
            };
            let mut plan = make(
                vec![Action::CreateWorkOrder {
                    name: "original inventory minimum".into(),
                    job_token: "CONSTRUCT_BED".into(),
                    amount: 3,
                    conditions: Vec::new(),
                }],
                goal.clone(),
                false,
            );
            if let Some(obligation) = &mut plan.steps[0].obligation {
                obligation.terminal = goal;
            }
            plan.digest = plan.compute_digest();
            plan.id = plan.expected_id();
            let (bytes, _) = roundtrip(&plan)?;
            let predicate = 12 + 16 + 16 + 64 + 4 + plan.summary.len();
            assert_eq!(bytes[predicate], 12);
            assert_eq!(
                plan.steps[0].obligation.as_ref().map(|o| (
                    o.deadline_tick,
                    o.poll_interval_ticks,
                    o.stable_for_observations
                )),
                Some((GameTick(180), 5, 2))
            );
        }
    }
    Ok(())
}

#[test]
fn inventory_quantity_archive_preserves_kind_aliases_and_rejects_malformed_optional_fields()
-> Result<()> {
    let goal = Predicate::EntityQuantity {
        kind: EntityKind::Item,
        filter_field: "type_key".into(),
        filter_value: Value::Text("BED".into()),
        quantity_field: Some("stack_size".into()),
        op: CompareOp::Ge,
        value: 1,
    };
    let ordinary = make(vec![Action::Pause { paused: false }], goal.clone(), false);
    let mut alias_goal = goal;
    if let Predicate::EntityQuantity { kind, .. } = &mut alias_goal {
        *kind = EntityKind::Other("item".into());
    }
    let alias = make(vec![Action::Pause { paused: false }], alias_goal, false);
    let (bytes, seal) = roundtrip(&ordinary)?;
    let (_, alias_seal) = roundtrip(&alias)?;
    assert_eq!(ordinary.digest, alias.digest);
    assert_ne!(seal.object_digest, alias_seal.object_digest);
    let predicate = 12 + 16 + 16 + 64 + 4 + ordinary.summary.len();
    // Tag, built-in kind, selector name, tagged text value, optional quantity.
    let optional = predicate + 1 + 1 + 4 + 8 + 1 + 4 + 3;
    assert_eq!(bytes[optional], 1);
    for (offset, patch) in [
        (optional, vec![2]),
        (optional + 1, u32::MAX.to_be_bytes().to_vec()),
        (optional + 1 + 4 + 10, vec![255]),
    ] {
        let mut changed = bytes.clone();
        changed[offset..offset + patch.len()].copy_from_slice(&patch);
        let (changed, expected) = resigned(&changed, seal.plan_digest);
        assert!(archive::decode(&changed, expected, ArchiveLimits::default()).is_err());
    }
    let mut oversized = ordinary;
    if let Predicate::EntityQuantity { quantity_field, .. } = &mut oversized.terminal_condition {
        *quantity_field = Some("x".repeat(257));
    }
    assert!(
        matches!(archive::encode(&oversized, ArchiveLimits::default()), Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn laboratory_room_access_archive_rejects_oversized_or_invalid_targets() -> Result<()> {
    let access = Predicate::LaboratoryRoomAccess {
        area: area(),
        entry: area().min,
        targets: vec![MapCoord::new(1, 3, 3), MapCoord::new(2, 3, 3)],
    };
    let plan = make(vec![Action::Pause { paused: false }], access, false);
    let (bytes, seal) = roundtrip(&plan)?;
    let predicate = 12 + 16 + 16 + 64 + 4 + plan.summary.len();
    for count in [0u32, 17, u32::MAX] {
        let mut changed = bytes.clone();
        changed[predicate + 37..predicate + 41].copy_from_slice(&count.to_be_bytes());
        let (changed, expected) = resigned(&changed, seal.plan_digest);
        assert!(archive::decode(&changed, expected, ArchiveLimits::default()).is_err());
    }
    for duplicate in [false, true] {
        let mut changed = bytes.clone();
        let start = predicate + 41;
        if duplicate {
            let first = changed[start..start + 12].to_vec();
            changed[start + 12..start + 24].copy_from_slice(&first);
        } else {
            changed[start..start + 4].copy_from_slice(&i32::MAX.to_be_bytes());
        }
        let (changed, expected) = resigned(&changed, seal.plan_digest);
        assert!(archive::decode(&changed, expected, ArchiveLimits::default()).is_err());
    }
    Ok(())
}

#[test]
fn reserved_kind_aliases_do_not_collapse_in_lossless_archive_identity() -> Result<()> {
    let ordinary = make(
        vec![Action::Pause { paused: false }],
        Predicate::EntityKind {
            entity_id: EntityId::new(1),
            kind: EntityKind::Unit,
        },
        false,
    );
    let custom = make(
        vec![Action::Pause { paused: false }],
        Predicate::EntityKind {
            entity_id: EntityId::new(1),
            kind: EntityKind::Other("unit".into()),
        },
        false,
    );
    // Existing semantic names collide. Do not change that old identity schema or
    // silently decode one Rust variant as another to hide the ambiguity.
    assert_eq!(ordinary.digest, custom.digest);
    let (a, sa) = roundtrip(&ordinary)?;
    let (b, sb) = roundtrip(&custom)?;
    assert_ne!(sa.object_digest, sb.object_digest);
    assert_ne!(a, b);
    assert!(archive::decode(&b, sa, ArchiveLimits::default()).is_err());
    Ok(())
}
#[test]
fn each_truncation_extension_and_single_byte_corruption_is_refused() -> Result<()> {
    let (bytes, seal) = roundtrip(&pause())?;
    for end in 0..bytes.len() {
        assert!(
            archive::decode(&bytes[..end], seal, ArchiveLimits::default()).is_err(),
            "cut {end}"
        );
    }
    for i in 0..bytes.len() {
        let mut damaged = bytes.clone();
        damaged[i] ^= 1;
        assert!(
            archive::decode(&damaged, seal, ArchiveLimits::default()).is_err(),
            "byte {i}"
        );
    }
    let mut trailing = bytes;
    trailing.push(0);
    assert!(archive::decode(&trailing, seal, ArchiveLimits::default()).is_err());
    Ok(())
}
#[test]
fn recomputed_checksum_cannot_replace_semantic_identity_or_validate_an_invalid_plan() -> Result<()>
{
    let (bytes, seal) = roundtrip(&pause())?;
    let mut changed_id = bytes.clone();
    changed_id[12] ^= 1;
    let (changed_id, new_seal) = resigned(&changed_id, seal.plan_digest);
    assert!(archive::decode(&changed_id, new_seal, ArchiveLimits::default()).is_err());
    let other = make(
        vec![Action::Pause { paused: true }],
        Predicate::Paused(true),
        false,
    );
    let (bytes, other_seal) = roundtrip(&other)?;
    assert!(
        archive::decode(
            &bytes,
            PlanArchiveSeal {
                plan_digest: seal.plan_digest,
                ..other_seal
            },
            ArchiveLimits::default()
        )
        .is_err()
    );
    Ok(())
}
#[test]
fn unknown_predicate_boolean_and_forged_lengths_refuse_after_checksum_verification() -> Result<()> {
    let plan = pause();
    let (bytes, seal) = roundtrip(&plan)?;
    let predicate = 12 + 16 + 16 + 64 + 4 + plan.summary.len();
    for (offset, patch) in [
        (predicate, vec![255]),
        (predicate + 1, vec![2]),
        (predicate + 2, u32::MAX.to_be_bytes().to_vec()),
        (12 + 96, u32::MAX.to_be_bytes().to_vec()),
    ] {
        let mut changed = bytes.clone();
        changed[offset..offset + patch.len()].copy_from_slice(&patch);
        let (changed, expected) = resigned(&changed, seal.plan_digest);
        assert!(
            archive::decode(&changed, expected, ArchiveLimits::default()).is_err(),
            "offset {offset}"
        );
    }
    Ok(())
}
#[test]
fn exact_byte_bounds_and_narrowed_structural_budgets_are_enforced() -> Result<()> {
    let plan = pause();
    let (bytes, seal) = roundtrip(&plan)?;
    let exact = ArchiveLimits {
        max_bytes: bytes.len(),
        ..ArchiveLimits::default()
    };
    assert_eq!(archive::encode(&plan, exact)?, (bytes.clone(), seal));
    assert_eq!(archive::decode(&bytes, seal, exact)?, plan);
    for limits in [
        ArchiveLimits {
            max_bytes: bytes.len() - 1,
            ..exact
        },
        ArchiveLimits {
            max_nodes: 1,
            ..exact
        },
        ArchiveLimits {
            max_steps: 0,
            ..exact
        },
        ArchiveLimits {
            max_depth: 0,
            ..exact
        },
    ] {
        assert!(archive::encode(&plan, limits).is_err());
        assert!(archive::decode(&bytes, seal, limits).is_err());
    }
    Ok(())
}
#[test]
fn oversized_action_text_and_collections_fail_before_legacy_validation() {
    let mut plan = pause();
    plan.steps[0].action = Action::SetLabor {
        units: vec![EntityId::new(2); 4097],
        labor: "MINING".into(),
        enabled: true,
    };
    assert!(
        matches!(archive::encode(&plan, ArchiveLimits::default()), Err(e) if e.code == ErrorCode::BudgetExceeded)
    );
    plan.steps[0].action = Action::SetStandingOrder {
        key: "x".into(),
        value: "v".repeat(65537),
    };
    assert!(
        matches!(archive::encode(&plan, ArchiveLimits::default()), Err(e) if e.code == ErrorCode::BudgetExceeded)
    );
}
#[test]
fn nesting_budget_is_shared_across_predicates_and_values() {
    let mut value = Value::Null;
    for _ in 0..65 {
        value = Value::List(vec![value]);
    }
    let mut plan = pause();
    plan.terminal_condition = Predicate::FieldCompare {
        entity_id: EntityId::new(1),
        field: "x".into(),
        op: CompareOp::Eq,
        value,
    };
    assert!(
        matches!(archive::encode(&plan, ArchiveLimits::default()), Err(e) if e.code == ErrorCode::BudgetExceeded)
    );
}
#[test]
fn changing_original_deadlines_keys_and_goals_requires_a_new_semantic_seal() {
    for which in 0..4 {
        let mut plan = pause();
        match which {
            0 => plan.expires_at_tick = GameTick(300),
            1 => plan.steps[0].idempotency_key = "0".repeat(64),
            2 => plan.terminal_condition = Predicate::Paused(true),
            _ => plan.anchor.cursor.epoch += 1,
        }
        assert!(archive::encode(&plan, ArchiveLimits::default()).is_err());
    }
}
#[test]
fn every_cooperative_encode_and_decode_abort_returns_no_result() -> Result<()> {
    let plan = pause();
    let (bytes, seal) = roundtrip(&plan)?;
    for decoding in [false, true] {
        let mut calls = 0;
        let mut count = || {
            calls += 1;
            Ok(())
        };
        if decoding {
            archive::decode_with_check(&bytes, seal, ArchiveLimits::default(), &mut count)?;
        } else {
            archive::encode_with_check(&plan, ArchiveLimits::default(), &mut count)?;
        }
        for stop in 1..=calls {
            let mut seen = 0;
            let check = || {
                seen += 1;
                if seen == stop {
                    Err(dfmcp_core::DfmcpError::new(
                        ErrorCode::CancellationRequested,
                        "test cancellation",
                    ))
                } else {
                    Ok(())
                }
            };
            let result = if decoding {
                archive::decode_with_check(&bytes, seal, ArchiveLimits::default(), check)
                    .map(|_| ())
            } else {
                archive::encode_with_check(&plan, ArchiveLimits::default(), check).map(|_| ())
            };
            assert!(matches!(result, Err(e) if e.code == ErrorCode::CancellationRequested));
        }
    }
    Ok(())
}
#[test]
fn independently_generated_pause_vector_matches_exact_archive_and_semantic_bytes() -> Result<()> {
    let raw = include_str!("fixtures/plan_archive_pause_v1.hex").trim();
    let expected = (0..raw.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&raw[i..i + 2], 16)
                .map_err(|_| dfmcp_core::DfmcpError::new(ErrorCode::InvalidPlan, "fixture hex"))
        })
        .collect::<Result<Vec<_>>>()?;
    let (bytes, seal) = roundtrip(&pause())?;
    assert_eq!(bytes, expected);
    assert_eq!(
        seal.plan_digest.to_hex(),
        include_str!("fixtures/plan_archive_pause_v1.plan-digest").trim()
    );
    Ok(())
}

#[test]
fn inventory_increase_archive_preserves_sealed_baseline_and_original_obligation() -> Result<()> {
    let goal = Predicate::EntityQuantityIncrease {
        kind: EntityKind::Item,
        filter_field: "type_key".into(),
        filter_value: Value::Text("BED".into()),
        quantity_field: Some("stack_size".into()),
        baseline: 42,
        increase: 3,
        baseline_anchor: anchor(),
        baseline_source_digest: Digest32::of_bytes(b"original complete inventory"),
        source_generation: 7,
        baseline_source_counters: [11, 12, 600],
    };
    let mut plan = make(
        vec![Action::CreateWorkOrder {
            name: "three beds".into(),
            job_token: "CONSTRUCT_BED".into(),
            amount: 3,
            conditions: vec![],
        }],
        goal.clone(),
        false,
    );
    if let Some(obligation) = &mut plan.steps[0].obligation {
        obligation.terminal = goal.clone();
    }
    plan.digest = plan.compute_digest();
    plan.id = plan.expected_id();
    let (bytes, seal) = roundtrip(&plan)?;
    let offset = 12 + 16 + 16 + 64 + 4 + plan.summary.len();
    assert_eq!(bytes[offset], 15);
    let restored = archive::decode(&bytes, seal, ArchiveLimits::default())?;
    assert_eq!(restored.anchor, anchor());
    assert_eq!(restored.terminal_condition, goal);
    assert_eq!(restored.steps[0].postconditions, vec![goal.clone()]);
    assert_eq!(restored.steps[0].idempotency_key, plan.steps[0].idempotency_key);
    let obligation = restored.steps[0].obligation.as_ref().ok_or_else(|| {
        dfmcp_core::DfmcpError::new(
            ErrorCode::InternalInvariantViolation, "missing fixture obligation",
        )
    })?;
    assert_eq!(obligation.terminal, goal);
    assert_eq!(
        (obligation.deadline_tick, obligation.poll_interval_ticks, obligation.stable_for_observations),
        (GameTick(180), 5, 2),
    );
    let mut unknown_tag = bytes;
    unknown_tag[offset] = 16;
    let (unknown_tag, expected) = resigned(&unknown_tag, seal.plan_digest);
    assert!(archive::decode(&unknown_tag, expected, ArchiveLimits::default()).is_err());
    Ok(())
}

#[test]
fn inventory_baseline_tampering_cannot_reuse_an_original_archive_seal() -> Result<()> {
    let goal = Predicate::EntityQuantityIncrease {
        kind: EntityKind::Item,
        filter_field: "type_key".into(),
        filter_value: Value::Text("BED".into()),
        quantity_field: None,
        baseline: 42,
        increase: 3,
        baseline_anchor: anchor(),
        baseline_source_digest: Digest32::of_bytes(b"original census"),
        source_generation: 7,
        baseline_source_counters: [11, 12, 600],
    };
    let original = make(vec![Action::Pause { paused: false }], goal.clone(), false);
    let (_, original_seal) = roundtrip(&original)?;
    for which in 0..6 {
        let mut changed = goal.clone();
        if let Predicate::EntityQuantityIncrease {
            baseline, increase, baseline_anchor, baseline_source_digest, source_generation,
            baseline_source_counters, ..
        } = &mut changed {
            match which {
                0 => *baseline += 1,
                1 => *increase += 1,
                2 => baseline_anchor.tick.0 += 1,
                3 => *baseline_source_digest = Digest32::of_bytes(b"replacement census"),
                4 => *source_generation += 1,
                _ => baseline_source_counters[2] += 1,
            }
        }
        let changed = make(vec![Action::Pause { paused: false }], changed, false);
        let (bytes, seal) = roundtrip(&changed)?;
        assert_ne!(seal.plan_digest, original_seal.plan_digest);
        assert_ne!(seal.object_digest, original_seal.object_digest);
        assert!(archive::decode(&bytes, original_seal, ArchiveLimits::default()).is_err());
    }
    Ok(())
}
