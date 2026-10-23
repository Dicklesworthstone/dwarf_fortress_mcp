//! The private room envelope preserves the complete source and exact lineage.

use super::*;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, FortressId, OperationContext, RequestId,
    RiskTier, SessionId, WorkBudget,
};

fn request() -> Result<ConnectedRoomRequest> {
    ConnectedRoomRequest::parse_current(&json!({
        "template":"connected_room",
        "planner":"connected_room_v1",
        "room":{
            "template":"provisioned_room", "planner":"provisioned_room_v1",
            "layout":{"template":"bedroom_cluster", "origin":[2,6,10], "rooms":2, "room_size":[3,3]},
            "prerequisites":{"assign_labor":true, "workshops":[{
                "building":"workshop:Carpenters", "location":[1,0,10],
            }]},
        },
        "access":{"entry":[0,2,10],"region":{"min":[0,0,10],"max":[12,12,10]}},
    }).to_string())
}

fn source() -> Result<ConnectedRoomContinuation> {
    let root = Digest32::of_bytes(b"original room");
    ConnectedRoomContinuation::new(root, root, request()?)
}

fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget::default(),
        cancellation_requested: false,
        grants: vec![CapabilityGrant {
            capability: Capability::Plan,
            max_risk: RiskTier::ReadOnly,
            scope: CapabilityScope {
                fortress_id: Some(snapshot.fortress_id),
                ..CapabilityScope::default()
            },
            expires_at_tick: None,
            remaining_uses: None,
        }],
    }
}

#[test]
fn canonical_connected_room_envelope_retains_original_compiler_and_unused_setup() -> Result<()> {
    let source = source()?;
    let raw = source.canonical_json();
    let retained: Value = serde_json::from_str(&raw).map_err(|error| invalid(error.to_string()))?;
    assert_eq!(retained["schema"], SOURCE_SCHEMA);
    assert_eq!(retained["planner"], PLANNER);
    assert_eq!(retained["connected_room"]["planner"], "connected_room_v1");
    assert_eq!(
        retained["connected_room"]["access"]["entry"],
        json!([0, 2, 10])
    );
    assert_eq!(
        retained["connected_room"]["access"]["region"],
        json!({"min":[0,0,10],"max":[12,12,10]})
    );
    assert_eq!(
        retained["connected_room"]["room"]["planner"],
        "provisioned_room_v1"
    );
    assert_eq!(
        retained["connected_room"]["room"]["prerequisites"]["assign_labor"],
        true
    );
    assert_eq!(
        retained["connected_room"]["room"]["prerequisites"]["workshops"][0]["location"],
        json!([1, 0, 10])
    );
    assert_eq!(ConnectedRoomContinuation::parse(&raw)?, source);
    assert_eq!(
        ConnectedRoomContinuation::parse(&raw)?.canonical_json(),
        raw
    );
    let mut bytes = SOURCE_DOMAIN.to_vec();
    bytes.extend_from_slice(raw.as_bytes());
    assert_eq!(source.source_digest(), Digest32::of_bytes(&bytes));
    Ok(())
}

#[test]
fn connected_room_lineage_is_flat_bounded_and_cannot_rewrite_original_request() -> Result<()> {
    let first = source()?;
    let mut next = first.clone();
    for index in 0..32 {
        next = next.next(Digest32::of_bytes(&[index]))?;
        assert_eq!(next.root, first.root);
        assert_eq!(next.request, first.request);
        assert_eq!(next.canonical_json().len(), first.canonical_json().len());
        assert_eq!(
            ConnectedRoomContinuation::parse(&next.canonical_json())?,
            next
        );
    }
    assert!(!first.same_original_request(&next));
    assert!(next.same_original_request(&ConnectedRoomContinuation::parse(&next.canonical_json())?));
    let lineage = next.lineage_json();
    assert_eq!(lineage["semantic_request_preserved"], true);
    assert_eq!(lineage["access_requirement_preserved"], true);
    assert_eq!(lineage["historical_exact_goal_unchanged"], true);
    assert_eq!(
        lineage["completion_proves"],
        "this_reviewed_access_and_room_pursuit_only"
    );
    assert_eq!(lineage["replacement_work_dispatched"], false);
    Ok(())
}

#[test]
fn connected_room_source_rejects_unknown_generations_injected_authority_and_missing_v1_marker()
-> Result<()> {
    let source = source()?;
    let raw: Value = serde_json::from_str(&source.canonical_json())
        .map_err(|error| invalid(error.to_string()))?;
    for (field, replacement) in [
        ("schema", json!("dfmcp.connected-room-continuation/2")),
        ("planner", json!("another_residual_compiler")),
        ("parent_plan_digest", json!(Digest32::ZERO.to_hex())),
        ("root_plan_digest", json!("not_a_digest")),
        ("work_quiescent", json!(true)),
        ("authority", json!(["construct"])),
    ] {
        let mut changed = raw.clone();
        changed[field] = replacement;
        assert!(ConnectedRoomContinuation::parse(&changed.to_string()).is_err());
    }
    for marker in [Value::Null, json!(PLANNER)] {
        let mut changed = raw.clone();
        changed["connected_room"]["planner"] = marker.clone();
        assert!(ConnectedRoomContinuation::parse(&changed.to_string()).is_err());
        let mut changed = raw.clone();
        changed["connected_room"]["room"]["planner"] = marker;
        assert!(ConnectedRoomContinuation::parse(&changed.to_string()).is_err());
    }
    assert!(ConnectedRoomContinuation::parse(&"x".repeat(MAX_ACTIONS_JSON_BYTES + 1)).is_err());
    assert!(ConnectedRoomContinuation::new(Digest32::ZERO, source.root, request()?).is_err());
    assert!(ConnectedRoomContinuation::new(source.parent, Digest32::ZERO, request()?).is_err());
    Ok(())
}

#[test]
fn connected_room_continuation_seal_binds_lineage_and_distinguishes_original_compilation()
-> Result<()> {
    let snapshot =
        crate::lab_world::scenario_snapshot("starter_fortress", FortressId::new(963_201), false)?;
    let source = source()?;
    let next = source.next(Digest32::of_bytes(b"another room parent"))?;
    let id = IntentId::new(81);
    let (first, analysis) = source.compile(id, &snapshot, "complete both rooms")?;
    let (other, _) = next.compile(id, &snapshot, "complete both rooms")?;
    assert_eq!(first.requested_actions, other.requested_actions);
    assert_eq!(first.terminal_condition, other.terminal_condition);
    assert_ne!(first.summary, other.summary);
    let planner = StaticPlanner::default();
    let first_plan = planner.prepare_laboratory(&snapshot, &first, &context(&snapshot))?;
    let other_plan = planner.prepare_laboratory(&snapshot, &other, &context(&snapshot))?;
    assert_ne!(first_plan.digest, other_plan.digest);
    let (replayed, replayed_analysis) = ConnectedRoomContinuation::parse(&source.canonical_json())?
        .compile(id, &snapshot, "complete both rooms")?;
    assert_eq!(replayed, first);
    assert_eq!(replayed_analysis, analysis);
    let (original, _) = source
        .request
        .compile(id, &snapshot, "complete both rooms")?;
    assert_ne!(first.summary, original.summary);
    assert_eq!(analysis["continuation"], source.lineage_json());
    assert_eq!(snapshot.anchor(), first.anchor);
    Ok(())
}

#[test]
fn connected_room_source_seal_retains_every_access_choice() -> Result<()> {
    let source = source()?;
    let raw: Value = serde_json::from_str(&source.canonical_json())
        .map_err(|error| invalid(error.to_string()))?;
    for access in [
        json!({"entry":[1,2,10],"region":{"min":[0,0,10],"max":[12,12,10]}}),
        json!({"entry":[0,2,10],"region":{"min":[0,0,10],"max":[13,12,10]}}),
    ] {
        let mut changed = raw.clone();
        changed["connected_room"]["access"] = access;
        let changed = ConnectedRoomContinuation::parse(&changed.to_string())?;
        assert_ne!(source.source_digest(), changed.source_digest());
        assert!(!source.same_original_request(&changed));
    }
    let snapshot =
        crate::lab_world::scenario_snapshot("starter_fortress", FortressId::new(963_205), false)?;
    assert!(
        source
            .compile(IntentId::new(82), &snapshot, &"s".repeat(256))
            .is_err()
    );
    Ok(())
}
