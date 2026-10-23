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
    StateAnchor { fortress_id: FortressId::new(42), cursor: ObservationCursor { epoch: 3, sequence: 7 },
        tick: GameTick(100), state_hash: Digest32::of_bytes(b"archive-source") }
}
fn make(actions: Vec<Action>, goal: Predicate, checkpoint: bool) -> PreparedPlan {
    let intent = IntentId::new(9);
    let mut maximum = RiskTier::ReadOnly;
    let mut capabilities = BTreeSet::new();
    let mut steps = Vec::new();
    for (i, action) in actions.into_iter().enumerate() {
        let action = action.normalized();
        let id = StepId::new(i as u32);
        let risk = action.risk(); let required_capability = action.capability();
        maximum = maximum.max(risk); capabilities.insert(required_capability);
        let obligation = action.naturally_temporal().then(|| ObligationSpec {
            terminal: Predicate::EntityExists(EntityId::new(7)),
            failure: Some(Predicate::Not(Box::new(Predicate::EntityExists(EntityId::new(8))))),
            deadline_tick: GameTick(180), poll_interval_ticks: 5, stable_for_observations: 2,
        });
        steps.push(PlanStep { id,
            idempotency_key: derive_step_idempotency_key(intent, anchor(), id, &action),
            action, preconditions: vec![Predicate::Paused(true)],
            postconditions: vec![goal.clone().normalized()],
            compensation: Some(Action::Pause { paused: true }), obligation,
            depends_on: if i == 0 { vec![] } else { vec![StepId::new(i as u32 - 1)] },
            risk, required_capability });
    }
    if checkpoint { capabilities.insert(Capability::Checkpoint); }
    PreparedPlan::builder(intent, anchor(), "Restore the original pause plan", goal)
        .steps(steps).max_risk(maximum).required_capabilities(capabilities)
        .requires_checkpoint(checkpoint).expires_at_tick(GameTick(200)).build()
}
fn pause() -> PreparedPlan {
    make(vec![Action::Pause { paused: false }], Predicate::Paused(false), false)
}
fn roundtrip(plan: &PreparedPlan) -> Result<(Vec<u8>, PlanArchiveSeal)> {
    let (bytes, seal) = archive::encode(plan, ArchiveLimits::default())?;
    assert_eq!(seal.plan_digest, plan.digest);
    let recovered = archive::decode(&bytes, seal, ArchiveLimits::default())?;
    assert_eq!(&recovered, plan);
    assert_eq!(archive::encode(&recovered, ArchiveLimits::default())?, (bytes.clone(), seal));
    Ok((bytes, seal))
}
fn area() -> MapCuboid {
    MapCuboid { min: MapCoord::new(1, 2, 3), max: MapCoord::new(2, 3, 3) }
}
fn resigned(bytes: &[u8], semantic: Digest32) -> (Vec<u8>, PlanArchiveSeal) {
    let mut bytes = bytes.to_vec();
    let end = bytes.len() - 32;
    let mut preimage = b"dfmcp-exact-plan-archive/1\0".to_vec();
    preimage.extend_from_slice(&bytes[..end]);
    let object_digest = Digest32::of_bytes(&preimage);
    bytes[end..].copy_from_slice(object_digest.as_bytes());
    (bytes, PlanArchiveSeal { plan_digest: semantic, object_digest })
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
        Action::SetLabor { units: vec![EntityId::new(4), EntityId::new(2)], labor: "MINING".into(), enabled: true },
        Action::CreateWorkOrder { name: "beds".into(), job_token: "CONSTRUCT_BED".into(), amount: 7,
            conditions: vec![WorkOrderCondition::CompletedOrder { order_name: "timber".into() },
                WorkOrderCondition::ItemCountBelow { item_token: "BED".into(), threshold: 20 },
                WorkOrderCondition::MaterialAvailable { material_token: "WOOD".into(), minimum: 7 }] },
        Action::ConfigureStockpile { stockpile: EntityId::new(8), accepts: BTreeSet::from(["WOOD".into(), "STONE".into()]),
            max_bins: Some(0), max_barrels: None, max_wheelbarrows: Some(u32::MAX) },
        Action::AssignSquad { units: vec![EntityId::new(2)], squad: EntityId::new(3) },
        Action::SetBurrowMembership { units: vec![EntityId::new(2)], burrow: EntityId::new(4), assigned: false },
        Action::SetStandingOrder { key: "gather".into(), value: "wood".into() },
        Action::Extension { namespace: "registered".into(), name: "opaque".into(),
            parameters: BTreeMap::from([("a".into(), "text is data".into()), ("b".into(), "λ".into())]) },
    ];
    for mode in [DigMode::Mine, DigMode::Channel, DigMode::UpStair, DigMode::DownStair,
        DigMode::UpDownStair, DigMode::Ramp, DigMode::RemoveConstruction]
    { actions.push(Action::DesignateDig { area: area(), mode }); }
    for kind in [BuildingKind::Workshop("CARPENTER".into()), BuildingKind::Furnace("SMELTER".into()),
        BuildingKind::Furniture("BED".into()), BuildingKind::Construction("FLOOR".into()),
        BuildingKind::Trap("CAGE".into()), BuildingKind::FarmPlot, BuildingKind::Bridge,
        BuildingKind::Well, BuildingKind::Custom("opaque".into())]
    {
        actions.push(Action::Build { kind, location: area().min, footprint: area(), material: MaterialSelector {
            required_tokens: BTreeSet::from(["WOOD".into()]), forbidden_tokens: BTreeSet::from(["MAGMA".into()]),
            prefer_nearest: true, reserve_count: 3 } });
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
    let values = vec![Value::Null, Value::Bool(true), Value::I64(i64::MIN), Value::U64(u64::MAX),
        Value::Fixed { units: -31, scale: 100 }, Value::Text("Urist λ".into()),
        Value::Entity(EntityId::new(33)), Value::Coord(MapCoord::new(-1, 2, 3)),
        Value::Bytes(vec![0, 255, 7]), Value::List(vec![Value::Null, Value::Bool(false)]),
        Value::Object(BTreeMap::from([("a".into(), Value::List(vec![Value::U64(7)]))]))];
    for value in values {
        for op in [CompareOp::Eq, CompareOp::Ne, CompareOp::Lt, CompareOp::Le, CompareOp::Gt, CompareOp::Ge] {
            let p = Predicate::FieldCompare { entity_id: EntityId::new(7), field: "observed".into(), op, value: value.clone() };
            let goal = Predicate::All(vec![p, Predicate::Any(vec![Predicate::Paused(false),
                Predicate::Not(Box::new(Predicate::EntityExists(EntityId::new(3))))])]);
            roundtrip(&make(vec![Action::Pause { paused: false }], goal, false))?;
        }
    }
    for goal in [Predicate::EntityExists(EntityId::new(1)),
        Predicate::RegionTerrain { area: area(), tile_code: 2 },
        Predicate::EdgeExists { edge_id: EdgeId::new(u128::MAX), kind: None },
        Predicate::EdgeExists { edge_id: EdgeId::new(2), kind: Some(EdgeKind::Custom("requires".into())) }]
    { roundtrip(&make(vec![Action::Pause { paused: false }], goal, false))?; }
    Ok(())
}
#[test]
fn reserved_kind_aliases_do_not_collapse_in_lossless_archive_identity() -> Result<()> {
    let ordinary = make(vec![Action::Pause { paused: false }], Predicate::EntityKind {
        entity_id: EntityId::new(1), kind: EntityKind::Unit }, false);
    let custom = make(vec![Action::Pause { paused: false }], Predicate::EntityKind {
        entity_id: EntityId::new(1), kind: EntityKind::Other("unit".into()) }, false);
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
    for end in 0..bytes.len() { assert!(archive::decode(&bytes[..end], seal, ArchiveLimits::default()).is_err(), "cut {end}"); }
    for i in 0..bytes.len() {
        let mut damaged = bytes.clone(); damaged[i] ^= 1;
        assert!(archive::decode(&damaged, seal, ArchiveLimits::default()).is_err(), "byte {i}");
    }
    let mut trailing = bytes; trailing.push(0);
    assert!(archive::decode(&trailing, seal, ArchiveLimits::default()).is_err());
    Ok(())
}
#[test]
fn recomputed_checksum_cannot_replace_semantic_identity_or_validate_an_invalid_plan() -> Result<()> {
    let (bytes, seal) = roundtrip(&pause())?;
    let mut changed_id = bytes.clone(); changed_id[12] ^= 1;
    let (changed_id, new_seal) = resigned(&changed_id, seal.plan_digest);
    assert!(archive::decode(&changed_id, new_seal, ArchiveLimits::default()).is_err());
    let other = make(vec![Action::Pause { paused: true }], Predicate::Paused(true), false);
    let (bytes, other_seal) = roundtrip(&other)?;
    assert!(archive::decode(&bytes, PlanArchiveSeal { plan_digest: seal.plan_digest, ..other_seal }, ArchiveLimits::default()).is_err());
    Ok(())
}
#[test]
fn unknown_predicate_boolean_and_forged_lengths_refuse_after_checksum_verification() -> Result<()> {
    let plan = pause(); let (bytes, seal) = roundtrip(&plan)?;
    let predicate = 12 + 16 + 16 + 64 + 4 + plan.summary.len();
    for (offset, patch) in [(predicate, vec![255]), (predicate + 1, vec![2]),
        (predicate + 2, u32::MAX.to_be_bytes().to_vec()), (12 + 96, u32::MAX.to_be_bytes().to_vec())]
    {
        let mut changed = bytes.clone(); changed[offset..offset + patch.len()].copy_from_slice(&patch);
        let (changed, expected) = resigned(&changed, seal.plan_digest);
        assert!(archive::decode(&changed, expected, ArchiveLimits::default()).is_err(), "offset {offset}");
    }
    Ok(())
}
#[test]
fn exact_byte_bounds_and_narrowed_structural_budgets_are_enforced() -> Result<()> {
    let plan = pause(); let (bytes, seal) = roundtrip(&plan)?;
    let exact = ArchiveLimits { max_bytes: bytes.len(), ..ArchiveLimits::default() };
    assert_eq!(archive::encode(&plan, exact)?, (bytes.clone(), seal));
    assert_eq!(archive::decode(&bytes, seal, exact)?, plan);
    for limits in [ArchiveLimits { max_bytes: bytes.len() - 1, ..exact },
        ArchiveLimits { max_nodes: 1, ..exact }, ArchiveLimits { max_steps: 0, ..exact },
        ArchiveLimits { max_depth: 0, ..exact }]
    {
        assert!(archive::encode(&plan, limits).is_err());
        assert!(archive::decode(&bytes, seal, limits).is_err());
    }
    Ok(())
}
#[test]
fn oversized_action_text_and_collections_fail_before_legacy_validation() {
    let mut plan = pause();
    plan.steps[0].action = Action::SetLabor { units: vec![EntityId::new(2); 4097], labor: "MINING".into(), enabled: true };
    assert!(matches!(archive::encode(&plan, ArchiveLimits::default()), Err(e) if e.code == ErrorCode::BudgetExceeded));
    plan.steps[0].action = Action::SetStandingOrder { key: "x".into(), value: "v".repeat(65537) };
    assert!(matches!(archive::encode(&plan, ArchiveLimits::default()), Err(e) if e.code == ErrorCode::BudgetExceeded));
}
#[test]
fn nesting_budget_is_shared_across_predicates_and_values() {
    let mut value = Value::Null;
    for _ in 0..65 { value = Value::List(vec![value]); }
    let mut plan = pause();
    plan.terminal_condition = Predicate::FieldCompare { entity_id: EntityId::new(1), field: "x".into(), op: CompareOp::Eq, value };
    assert!(matches!(archive::encode(&plan, ArchiveLimits::default()), Err(e) if e.code == ErrorCode::BudgetExceeded));
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
    let plan = pause(); let (bytes, seal) = roundtrip(&plan)?;
    for decoding in [false, true] {
        let mut calls = 0;
        let mut count = || { calls += 1; Ok(()) };
        if decoding { archive::decode_with_check(&bytes, seal, ArchiveLimits::default(), &mut count)?; }
        else { archive::encode_with_check(&plan, ArchiveLimits::default(), &mut count)?; }
        for stop in 1..=calls {
            let mut seen = 0;
            let check = || { seen += 1; if seen == stop { Err(dfmcp_core::DfmcpError::new(ErrorCode::CancellationRequested, "test cancellation")) } else { Ok(()) } };
            let result = if decoding { archive::decode_with_check(&bytes, seal, ArchiveLimits::default(), check).map(|_| ()) }
                else { archive::encode_with_check(&plan, ArchiveLimits::default(), check).map(|_| ()) };
            assert!(matches!(result, Err(e) if e.code == ErrorCode::CancellationRequested));
        }
    }
    Ok(())
}
#[test]
fn independently_generated_pause_vector_matches_exact_archive_and_semantic_bytes() -> Result<()> {
    let raw = include_str!("fixtures/plan_archive_pause_v1.hex").trim();
    let expected = (0..raw.len()).step_by(2).map(|i| u8::from_str_radix(&raw[i..i+2],16)
        .map_err(|_| dfmcp_core::DfmcpError::new(ErrorCode::InvalidPlan,"fixture hex"))).collect::<Result<Vec<_>>>()?;
    let (bytes, seal) = roundtrip(&pause())?;
    assert_eq!(bytes, expected);
    assert_eq!(seal.plan_digest.to_hex(), include_str!("fixtures/plan_archive_pause_v1.plan-digest").trim());
    Ok(())
}
