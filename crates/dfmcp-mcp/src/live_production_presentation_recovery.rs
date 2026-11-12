//! Keep the most recent complete prerequisite custody when a failed owner
//! cannot return its state. Retained values stay historical, never current proof.
use super::digest;
use serde_json::{Value, json};

const WORKSHOP: &[&str] = &["tick", "generation", "next_building_id"];
const MANAGER: &[&str] = &[
    "tick",
    "generation",
    "sequence",
    "next_event",
    "history_count",
    "group_next_position",
    "group_next_assignment",
    "civ_next_position",
    "civ_next_assignment",
];

fn complete_floor(value: &Value, fields: &[&str]) -> bool {
    value["fenced"].is_boolean() && fields.iter().all(|field| value[*field].is_u64())
}

pub(super) fn retain(retained: &mut Value, latest: &Value) {
    let mut recovery = latest["recovery"].clone();
    for (scope, seal, floor, fields) in [
        (
            "workshop_prerequisite",
            "workshop_prerequisite_digest",
            "workshop_prerequisite_floor",
            WORKSHOP,
        ),
        (
            "manager_prerequisite",
            "manager_prerequisite_digest",
            "manager_prerequisite_floor",
            MANAGER,
        ),
    ] {
        // These form one original association. Never combine a new digest with
        // an unavailable floor, or let a partial failure erase the known pair.
        if recovery.is_object()
            && !(digest(&recovery[seal]).is_some() && complete_floor(&recovery[floor], fields))
            && digest(&retained["recovery"][seal]).is_some()
            && complete_floor(&retained["recovery"][floor], fields)
        {
            recovery[seal] = retained["recovery"][seal].clone();
            recovery[floor] = retained["recovery"][floor].clone();
        }
        let checkpoint = &latest[scope]["checkpoint"];
        let complete = complete_floor(checkpoint, fields)
            && digest(&checkpoint["origin_witness"]).is_some()
            && checkpoint["origin_witness"] == latest[scope]["origin_witness"];
        let fenced =
            retained[scope]["source_fenced"] == true || latest[scope]["source_fenced"] == true;
        if complete || (!retained[scope].is_object() && latest[scope].is_object()) {
            retained[scope] = latest[scope].clone();
        } else if latest[scope]["status"] == "unavailable" && retained[scope].is_object() {
            retained[scope]["status"] = json!("unavailable");
        }
        if retained[scope].is_object() {
            retained[scope]["last_qualified"] = Value::Null;
            if fenced {
                if recovery[floor].is_object() {
                    recovery[floor]["fenced"] = json!(true);
                }
                retained[scope]["source_fenced"] = json!(true);
                if retained[scope]["checkpoint"].is_object() {
                    retained[scope]["checkpoint"]["fenced"] = json!(true);
                }
            }
        }
    }
    if !recovery.is_null() {
        retained["recovery"] = recovery;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_owner_keeps_both_exact_checkpoint_and_association_pairs() {
        let mut retained = json!({"recovery":{"frontier":{"head":"old"}}});
        for (scope, seal, floor, fields) in [
            (
                "workshop_prerequisite",
                "workshop_prerequisite_digest",
                "workshop_prerequisite_floor",
                WORKSHOP,
            ),
            (
                "manager_prerequisite",
                "manager_prerequisite_digest",
                "manager_prerequisite_floor",
                MANAGER,
            ),
        ] {
            let mut state = json!({"fenced":true});
            for field in fields {
                state[*field] = json!(41);
            }
            retained["recovery"][seal] = json!("a".repeat(64));
            retained["recovery"][floor] = state.clone();
            state["origin_witness"] = json!("b".repeat(64));
            retained[scope] = json!({"origin_witness":"b".repeat(64),"checkpoint":state,
                "association_digest":"a".repeat(64),"source_fenced":true,"last_qualified":{"status":"ready"}});
        }
        let original = retained.clone();
        let latest = json!({"recovery":{"frontier":{"head":"new"},
            "manager_prerequisite_digest":null,"manager_prerequisite_floor":null,
            "workshop_prerequisite_digest":null,"workshop_prerequisite_floor":null},
            "manager_prerequisite":{"status":"unavailable","current_state_proven":false},
            "workshop_prerequisite":{"status":"unavailable","current_state_proven":false}});
        retain(&mut retained, &latest);
        assert_eq!(retained["recovery"]["frontier"]["head"], "new");
        for scope in ["workshop_prerequisite", "manager_prerequisite"] {
            assert_eq!(retained[scope]["checkpoint"], original[scope]["checkpoint"]);
            assert_eq!(
                retained[scope]["association_digest"],
                original[scope]["association_digest"]
            );
            assert_eq!(retained[scope]["source_fenced"], true);
            assert_eq!(retained[scope]["status"], "unavailable");
            assert!(retained[scope]["last_qualified"].is_null());
            for suffix in ["_digest", "_floor"] {
                let key = format!("{scope}{suffix}");
                assert_eq!(retained["recovery"][&key], original["recovery"][&key]);
            }
        }
        retained["recovery"] = Value::Null;
        retain(
            &mut retained,
            &json!({"manager_prerequisite":{"status":"unavailable"}}),
        );
        assert_eq!(
            retained["manager_prerequisite"]["checkpoint"],
            original["manager_prerequisite"]["checkpoint"]
        );
    }

    #[test]
    fn complete_new_checkpoint_advances_without_restoring_readiness_or_erasing_fence() {
        let mut retained = json!({"manager_prerequisite":{"source_fenced":true}});
        let mut floor = json!({"fenced":true});
        for field in MANAGER {
            floor[*field] = json!(99);
        }
        let mut checkpoint = floor.clone();
        checkpoint["origin_witness"] = json!("b".repeat(64));
        let latest = json!({"recovery":{"manager_prerequisite_digest":"a".repeat(64),
            "manager_prerequisite_floor":floor},"manager_prerequisite":{"origin_witness":"b".repeat(64),
            "checkpoint":checkpoint,"source_fenced":false,"last_qualified":{"status":"ready"}}});
        retain(&mut retained, &latest);
        assert_eq!(retained["manager_prerequisite"]["checkpoint"], checkpoint);
        assert_eq!(retained["manager_prerequisite"]["source_fenced"], true);
        assert!(retained["manager_prerequisite"]["last_qualified"].is_null());
        assert_eq!(retained["recovery"], latest["recovery"]);
    }
}
