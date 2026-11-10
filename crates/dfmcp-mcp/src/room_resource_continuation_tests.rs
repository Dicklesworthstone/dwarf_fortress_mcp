//! Resource policy and room intent survive every separately reviewed stage.

use super::*;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, FortressId, OperationContext, RequestId,
    RiskTier, SessionId, WorkBudget,
};

fn request(assign_labor: bool) -> Result<ProvisionedRoomRequest> {
    ProvisionedRoomRequest::parse_current(
        &json!({
            "template":"provisioned_room",
            "layout":{"template":"bedroom_cluster","origin":[2,6,10],"rooms":2,"room_size":[3,3]},
            "acquire_resources":{"assign_labor":assign_labor},
            "prerequisites":{"assign_labor":false,"workshops":[{
                "building":"workshop:Carpenters","location":[1,0,10],
            }]},
        })
        .to_string(),
    )
}

fn source(assign_labor: bool) -> Result<RoomContinuation> {
    let root = Digest32::of_bytes(b"resource-aware original room");
    RoomContinuation::new(root, root, request(assign_labor)?)
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
fn resource_room_envelope_keeps_original_policy_setup_and_flat_lineage() -> Result<()> {
    let first = source(true)?;
    let raw = first.canonical_json();
    let value: Value = serde_json::from_str(&raw).map_err(|error| invalid(error.to_string()))?;
    assert_eq!(value["schema"], RESOURCE_SOURCE_SCHEMA);
    assert_eq!(value["planner"], "provisioned_room_resources_residual_v1");
    assert_eq!(value["room"]["planner"], "provisioned_room_resources_v1");
    assert_eq!(value["room"]["acquire_resources"]["assign_labor"], true);
    assert_eq!(value["room"]["prerequisites"]["assign_labor"], false);
    assert_eq!(
        value["room"]["prerequisites"]["workshops"][0]["location"],
        json!([1, 0, 10])
    );
    assert_eq!(RoomContinuation::parse(&raw)?, first);
    let mut bytes = RESOURCE_SOURCE_DOMAIN.to_vec();
    bytes.extend_from_slice(raw.as_bytes());
    assert_eq!(first.source_digest(), Digest32::of_bytes(&bytes));
    let mut next = first.clone();
    for ordinal in 0u64..16 {
        next = next.next(Digest32::of_bytes(&ordinal.to_be_bytes()))?;
        next = RoomContinuation::parse(&next.canonical_json())?;
        assert_eq!(next.root, first.root);
        assert_eq!(next.request, first.request);
        assert_eq!(next.canonical_json().len(), raw.len());
    }
    let lineage = next.lineage_json();
    assert_eq!(lineage["original_resource_policy_preserved"], true);
    assert_eq!(lineage["historical_exact_goal_unchanged"], true);
    assert_eq!(lineage["stage_completion_is_project_completion"], false);
    assert_eq!(lineage["replacement_work_dispatched"], false);
    Ok(())
}

#[test]
fn resource_room_archive_rejects_generation_crossovers_and_erased_permissions() -> Result<()> {
    let raw = source(false)?.canonical_json();
    let value: Value = serde_json::from_str(&raw).map_err(|error| invalid(error.to_string()))?;
    for (field, replacement) in [
        ("schema", json!(SOURCE_SCHEMA)),
        ("schema", json!("dfmcp.room-continuation/3")),
        ("planner", json!(PLANNER)),
        ("authority", json!(["harvest_resource"])),
    ] {
        let mut changed = value.clone();
        changed[field] = replacement;
        assert!(RoomContinuation::parse(&changed.to_string()).is_err());
    }
    for (field, replacement) in [
        ("planner", json!("provisioned_room_v1")),
        ("planner", Value::Null),
        ("acquire_resources", Value::Null),
        ("acquire_resources", json!({"assign_labor":null})),
        (
            "acquire_resources",
            json!({"assign_labor":false,"assume_supply":true}),
        ),
    ] {
        let mut changed = value.clone();
        changed["room"][field] = replacement;
        assert!(RoomContinuation::parse(&changed.to_string()).is_err());
    }
    for field in ["planner", "acquire_resources"] {
        let mut changed = value.clone();
        changed["room"]
            .as_object_mut()
            .ok_or_else(|| invalid("fixture room missing"))?
            .remove(field);
        assert!(RoomContinuation::parse(&changed.to_string()).is_err());
    }
    for duplicate in [
        raw.replace(
            "\"acquire_resources\":{\"assign_labor\":false}",
            "\"acquire_resources\":{\"assign_labor\":true,\"assign_labor\":false}",
        ),
        raw.replace(
            "\"acquire_resources\":{\"assign_labor\":false}",
            "\"acquire_resources\":{\"assign_labor\":true},\"acquire_resources\":{\"assign_labor\":false}",
        ),
    ] {
        assert_ne!(duplicate, raw);
        assert!(RoomContinuation::parse(&duplicate).is_err());
    }
    Ok(())
}

#[test]
fn unused_acquisition_permission_changes_room_seal_without_changing_physical_work() -> Result<()> {
    let snapshot = crate::lab_world::scenario_snapshot(
        "starter_fortress_resources",
        FortressId::new(963_301),
        false,
    )?;
    let first = source(false)?;
    let changed = source(true)?;
    assert_ne!(first.source_digest(), changed.source_digest());
    assert!(!first.same_original_request(&changed));
    let id = IntentId::new(91);
    let (first_intent, analysis) = first.compile(id, &snapshot, "furnish retained bedrooms")?;
    let (changed_intent, _) = changed.compile(id, &snapshot, "furnish retained bedrooms")?;
    assert_eq!(
        first_intent.requested_actions,
        changed_intent.requested_actions
    );
    assert_eq!(
        first_intent.terminal_condition,
        changed_intent.terminal_condition
    );
    assert_ne!(first_intent.summary, changed_intent.summary);
    let planner = StaticPlanner::default();
    let first_plan = planner.prepare_laboratory(&snapshot, &first_intent, &context(&snapshot))?;
    let changed_plan =
        planner.prepare_laboratory(&snapshot, &changed_intent, &context(&snapshot))?;
    assert_ne!(first_plan.digest, changed_plan.digest);
    assert_eq!(analysis["continuation"], first.lineage_json());
    assert_eq!(first.semantic_goal_truth(&snapshot)?, PredicateTruth::False);
    assert_eq!(
        RoomContinuation::parse(&first.canonical_json())?.compile(
            id,
            &snapshot,
            "furnish retained bedrooms",
        )?,
        (first_intent, analysis)
    );
    Ok(())
}
