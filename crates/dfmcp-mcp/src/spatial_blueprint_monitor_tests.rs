use super::*;
use dfmcp_core::{CapabilityGrant, CapabilityScope, FortressId, GameTick, MapCoord, MapCuboid,
    ObservationCursor, RequestId, SessionId, StateAnchor, WorkBudget};
use dfmcp_intent::blueprint::{BlueprintPlanner, BlueprintTemplate};
use std::collections::BTreeSet;

fn context() -> OperationContext {
    OperationContext { session_id: SessionId::new(3), request_id: RequestId::new(4),
        anchor: StateAnchor { fortress_id: FortressId::new(1), tick: GameTick(100),
            cursor: ObservationCursor::ORIGIN, state_hash: Digest32::of_bytes(b"capture") },
        budget: WorkBudget { max_game_ticks: 200, ..WorkBudget::default() },
        grants: vec![CapabilityGrant { capability: Capability::Query, scope: CapabilityScope::default(),
            max_risk: RiskTier::ReadOnly, expires_at_tick: None, remaining_uses: None }], cancellation_requested: false }
}
fn options() -> Options {
    Options { key: "excavation".into(), deadline_tick: 200, poll_interval_ticks: None, stable_observations: None }
}
fn origin() -> MapCoord { MapCoord::new(10, 10, 5) }
fn hall() -> Result<BlueprintLayout> {
    BlueprintPlanner.layout(origin(), BlueprintTemplate::DiningHall { width: 3, height: 3 })
}

#[test]
fn proposal_covers_every_part_and_preserves_explicit_anchor_and_registration() -> Result<()> {
    let layout = BlueprintPlanner.layout(origin(), BlueprintTemplate::BedroomCluster { rooms_count: 20, room_size: (3, 3) })?;
    let result = proposal(&layout, [128, 128, 16], &context(), options())?;
    let watch = &result["watch_request"]["query"];
    assert_eq!(watch["condition"]["areas"], json!(layout.excavations().iter().map(|p| area_json(p.area)).collect::<Vec<_>>()));
    assert_eq!(watch["condition"]["value"], layout.tile_count());
    assert_eq!(watch["condition"]["predicate"]["value"]["value"], "floor");
    assert_eq!(watch["stable_observations"], 2); assert_eq!(watch["poll_interval_ticks"], 1);
    assert_eq!(result["watch_request"]["expected_anchor"], super::super::super::anchor_json(context().anchor));
    assert_eq!(result["watch_registered"], false);
    assert_eq!(result, proposal(&layout, [128, 128, 16], &context(), options())?);
    Ok(())
}

#[test]
fn moat_monitor_excludes_crossing_and_enclosed_interior() -> Result<()> {
    let layout = BlueprintPlanner.layout(origin(), BlueprintTemplate::DefensiveMoat {
        perimeter_cuboid: MapCuboid::new(origin(), MapCoord::new(16, 16, 5))?, drawbridge_span: 3 })?;
    let result = proposal(&layout, [128, 128, 16], &context(), options())?;
    assert_eq!(result["target_shapes"], json!(["empty", "ramp_top"]));
    let mut points = BTreeSet::new();
    for area in result["watch_request"]["query"]["condition"]["areas"].as_array().ok_or_else(|| invalid("test mask"))? {
        let low: [i32; 3] = serde_json::from_value(area["min"].clone()).map_err(|_| invalid("test minimum"))?;
        let high: [i32; 3] = serde_json::from_value(area["max"].clone()).map_err(|_| invalid("test maximum"))?;
        for y in low[1]..=high[1] { for x in low[0]..=high[0] { assert!(points.insert([x, y, low[2]])); } }
    }
    assert_eq!(points.len(), 21); assert_eq!(result["requested_tiles"], 21);
    for x in 12..=14 { assert!(!points.contains(&[x, 10, 5])); }
    for y in 11..=15 { for x in 11..=15 { assert!(!points.contains(&[x, y, 5])); } }
    Ok(())
}

#[test]
fn proposal_identity_covers_session_deadline_cadence_stability_and_key() -> Result<()> {
    let base = proposal(&hall()?, [128, 128, 16], &context(), options())?;
    for case in 0..5 {
        let mut changed = options(); let mut c = context();
        match case { 0 => changed.key = "another".into(), 1 => changed.deadline_tick += 1,
            2 => changed.poll_interval_ticks = Some(2), 3 => changed.stable_observations = Some(3),
            _ => c.session_id = SessionId::new(99) }
        assert_ne!(proposal(&hall()?, [128, 128, 16], &c, changed)?["proposal_digest"], base["proposal_digest"]);
    }
    Ok(())
}

#[test]
fn invalid_monitor_options_and_out_of_map_parts_are_refused_without_clamping() -> Result<()> {
    for case in 0..8 {
        let mut changed = options();
        match case { 0 => changed.key.clear(), 1 => changed.key = "x".repeat(65),
            2 => changed.key = "bad\0key".into(), 3 => changed.deadline_tick = 100,
            4 => changed.deadline_tick = 301, 5 => changed.poll_interval_ticks = Some(0),
            6 => changed.stable_observations = Some(65), _ => changed.stable_observations = Some(0) }
        assert!(proposal(&hall()?, [128, 128, 16], &context(), changed).is_err());
    }
    let layout = BlueprintPlanner.layout(MapCoord::new(0, 0, 5), BlueprintTemplate::WorkshopHub { bays_count: 1 })?;
    assert!(proposal(&layout, [128, 128, 16], &context(), options()).is_err());
    assert!(proposal(&hall()?, [12, 128, 16], &context(), options()).is_err());
    let mut denied = context(); denied.grants.clear();
    assert!(proposal(&hall()?, [128, 128, 16], &denied, options()).is_err());
    assert!(serde_json::from_value::<Options>(json!({"key":"x","deadline_tick":200,"commit":true})).is_err());
    Ok(())
}
