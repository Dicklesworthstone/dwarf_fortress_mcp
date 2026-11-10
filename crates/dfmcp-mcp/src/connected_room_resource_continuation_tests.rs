//! Complete access and acquisition choices remain bound across room stages.

use super::*;

fn request(stairs: bool) -> Result<ConnectedRoomRequest> {
    ConnectedRoomRequest::parse_current(
        &json!({
            "template":"connected_room",
            "planner":if stairs {"connected_room_stairs_resources_v1"} else {"connected_room_resources_v1"},
            "room":{
                "template":"provisioned_room",
                "layout":{"template":"bedroom_cluster","origin":[2,6,if stairs {9} else {10}],"rooms":2,"room_size":[3,3]},
                "acquire_resources":{"assign_labor":true},
                "prerequisites":{"assign_labor":false,"workshops":[{
                    "building":"workshop:Carpenters","location":[1,0,10],
                }]},
            },
            "access":{"entry":[0,2,10],"region":{"min":[0,0,if stairs {9} else {10}],"max":[12,12,10]}},
        })
        .to_string(),
    )
}

fn source(stairs: bool) -> Result<ConnectedRoomContinuation> {
    let root = Digest32::of_bytes(b"resource-aware connected room");
    ConnectedRoomContinuation::new(root, root, request(stairs)?)
}

#[test]
fn cardinal_and_stair_resource_envelopes_retain_every_original_choice() -> Result<()> {
    for stairs in [false, true] {
        let first = source(stairs)?;
        let raw = first.canonical_json();
        let value: Value =
            serde_json::from_str(&raw).map_err(|error| invalid(error.to_string()))?;
        assert_eq!(value["schema"], RESOURCE_SOURCE_SCHEMA);
        assert_eq!(
            value["planner"],
            if stairs {
                "connected_room_stairs_resources_residual_v1"
            } else {
                "connected_room_resources_residual_v1"
            }
        );
        assert_eq!(
            value["connected_room"]["planner"],
            if stairs {
                "connected_room_stairs_resources_v1"
            } else {
                "connected_room_resources_v1"
            }
        );
        assert_eq!(
            value["connected_room"]["room"]["planner"],
            "provisioned_room_resources_v1"
        );
        assert_eq!(
            value["connected_room"]["room"]["acquire_resources"]["assign_labor"],
            true
        );
        assert_eq!(
            value["connected_room"]["room"]["prerequisites"]["assign_labor"],
            false
        );
        assert_eq!(
            value["connected_room"]["access"]["entry"],
            json!([0, 2, 10])
        );
        assert_eq!(ConnectedRoomContinuation::parse(&raw)?, first);
        let mut bytes = RESOURCE_SOURCE_DOMAIN.to_vec();
        bytes.extend_from_slice(raw.as_bytes());
        assert_eq!(first.source_digest(), Digest32::of_bytes(&bytes));
        let mut next = first.clone();
        for ordinal in 0u64..16 {
            next = next.next(Digest32::of_bytes(&ordinal.to_be_bytes()))?;
            next = ConnectedRoomContinuation::parse(&next.canonical_json())?;
            assert_eq!(next.root, first.root);
            assert_eq!(next.request, first.request);
            assert_eq!(next.canonical_json().len(), raw.len());
        }
        let lineage = next.lineage_json();
        assert_eq!(lineage["access_requirement_preserved"], true);
        assert_eq!(lineage["original_resource_policy_preserved"], true);
        assert_eq!(lineage["stage_completion_is_project_completion"], false);
        assert_eq!(lineage["replacement_work_dispatched"], false);
    }
    Ok(())
}

#[test]
fn connected_resource_source_cannot_cross_old_schema_or_access_generations() -> Result<()> {
    for stairs in [false, true] {
        let raw = source(stairs)?.canonical_json();
        let value: Value =
            serde_json::from_str(&raw).map_err(|error| invalid(error.to_string()))?;
        for (field, replacement) in [
            ("schema", json!(SOURCE_SCHEMA)),
            ("schema", json!("dfmcp.connected-room-continuation/3")),
            ("planner", json!(PLANNER)),
            (
                "planner",
                json!(if stairs {
                    "connected_room_resources_residual_v1"
                } else {
                    "connected_room_stairs_resources_residual_v1"
                }),
            ),
            ("work_quiescent", json!(true)),
        ] {
            let mut changed = value.clone();
            changed[field] = replacement;
            assert!(ConnectedRoomContinuation::parse(&changed.to_string()).is_err());
        }
        for marker in ["connected_room_v1", "connected_room_stairs_v1"] {
            let mut changed = value.clone();
            changed["connected_room"]["planner"] = json!(marker);
            assert!(ConnectedRoomContinuation::parse(&changed.to_string()).is_err());
        }
        for field in ["planner", "acquire_resources"] {
            let mut changed = value.clone();
            changed["connected_room"]["room"]
                .as_object_mut()
                .ok_or_else(|| invalid("fixture room missing"))?
                .remove(field);
            assert!(ConnectedRoomContinuation::parse(&changed.to_string()).is_err());
        }
        let duplicate = raw.replace(
            "\"acquire_resources\":{\"assign_labor\":true}",
            "\"acquire_resources\":{\"assign_labor\":false,\"assign_labor\":true}",
        );
        assert_ne!(duplicate, raw);
        assert!(ConnectedRoomContinuation::parse(&duplicate).is_err());
    }
    Ok(())
}

#[test]
fn connected_resource_seal_binds_unused_permissions_and_access_choices() -> Result<()> {
    let original = source(false)?;
    let value: Value = serde_json::from_str(&original.canonical_json())
        .map_err(|error| invalid(error.to_string()))?;
    for variant in 0..3 {
        let mut changed = value.clone();
        match variant {
            0 => changed["connected_room"]["access"]["entry"] = json!([1, 2, 10]),
            1 => changed["connected_room"]["access"]["region"]["max"] = json!([13, 12, 10]),
            _ => {
                changed["connected_room"]["room"]["acquire_resources"]["assign_labor"] =
                    json!(false)
            }
        }
        let changed = ConnectedRoomContinuation::parse(&changed.to_string())?;
        assert_ne!(changed.source_digest(), original.source_digest());
        assert!(!changed.same_original_request(&original));
    }
    Ok(())
}
