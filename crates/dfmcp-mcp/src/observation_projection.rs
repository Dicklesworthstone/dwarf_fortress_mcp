//! Presence-preserving laboratory projections and response coverage.
//!
//! A displayed null is not evidence of absence. Completeness describes the
//! returned projection, separately from the internal laboratory source.

use std::collections::BTreeMap;

use dfmcp_core::{Result, StateAnchor};
use dfmcp_world::{EntityRecord, Fact, FactPresence, PredicateTruth, Value};
use serde_json::{Value as Json, json};

pub(crate) fn anchor_json(anchor: &StateAnchor) -> Json {
    json!({
        "fortress_id": anchor.fortress_id.to_string(),
        "epoch": anchor.cursor.epoch,
        "sequence": anchor.cursor.sequence,
        "game_tick": anchor.tick.0,
        "state_hash": anchor.state_hash.to_hex(),
    })
}

/// Goal success is a derived claim about the authorized reference model.
/// A readable compatibility value is insufficient; unknown and invalid
/// evidence never produce an achieved goal or an observed truth claim.
pub(crate) fn laboratory_objective_evidence(truth: Result<PredicateTruth>) -> Json {
    let (truth, error) = match truth {
        Ok(truth) => (truth, None),
        Err(error) => (PredicateTruth::Unknown, Some(error)),
    };
    let mut result = json!({
        "status": if truth == PredicateTruth::True { "achieved" } else { "not_yet_observed" },
        "epistemic_state": if truth == PredicateTruth::Unknown { "unknown" } else { "certified_derived" },
        "predicate_truth": match truth {
            PredicateTruth::True => "true",
            PredicateTruth::False => "false",
            PredicateTruth::Unknown => "unknown",
        },
        "evidence_scope": "laboratory_reference_world",
    });
    if let Some(error) = error {
        result["observation_error"] = json!({
            "code": error.code.as_str(),
            "message": error.message,
        });
    }
    result
}

const JSON_SAFE_INTEGER: u64 = (1 << 53) - 1;

pub(crate) fn value_json(value: &Value) -> Json {
    match value {
        Value::Null => Json::Null,
        Value::Bool(value) => json!(value),
        Value::I64(value) if value.unsigned_abs() <= JSON_SAFE_INTEGER => json!(value),
        Value::I64(value) => json!(value.to_string()),
        Value::U64(value) if *value <= JSON_SAFE_INTEGER => json!(value),
        Value::U64(value) => json!(value.to_string()),
        Value::Fixed { units, scale } => json!({"units": units.to_string(), "scale": scale}),
        Value::Text(value) => json!(value),
        Value::Entity(id) => json!({"entity_id": id.get().to_string()}),
        Value::Coord(c) => json!([c.x, c.y, c.z]),
        Value::Bytes(bytes) => json!({"bytes": bytes.len()}),
        Value::List(values) => Json::Array(values.iter().map(value_json).collect()),
        Value::Object(values) => Json::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), value_json(value)))
                .collect(),
        ),
    }
}

pub(crate) fn fact_value(fact: &Fact) -> Json {
    fact.known_value().map_or(Json::Null, value_json)
}

/// Presence of the displayed value, which may summarize a known binary payload.
pub(crate) fn value_presence(value: &Value) -> Json {
    fn contains_bytes(value: &Value) -> bool {
        match value {
            Value::Bytes(_) => true,
            Value::List(values) => values.iter().any(contains_bytes),
            Value::Object(values) => values.values().any(contains_bytes),
            _ => false,
        }
    }
    if contains_bytes(value) {
        json!({"state": "omitted", "source_state": "known",
            "projection": "binary_payload_summarized",
            "reason": "binary payloads are represented by their byte lengths in this JSON rendering"})
    } else {
        json!({"state": "known"})
    }
}

pub(crate) fn presence_json(fact: &Fact) -> Json {
    match fact.presence.as_ref() {
        None => value_presence(&fact.value),
        Some(FactPresence::Known(value)) if value == &fact.value => value_presence(value),
        Some(FactPresence::Known(_)) => json!({
            "state": "unknown", "reason": "inconsistent known-value representation",
        }),
        Some(FactPresence::Absent) => json!({"state": "absent"}),
        Some(FactPresence::Unknown(reason)) => json!({"state": "unknown", "reason": reason}),
        Some(FactPresence::Unsupported(manifest)) => {
            json!({"state": "unsupported", "manifest": manifest})
        }
        Some(FactPresence::Omitted(profile)) => json!({"state": "omitted", "projection": profile}),
        Some(FactPresence::Redacted(policy)) => json!({"state": "redacted", "policy": policy}),
        Some(FactPresence::Stale(anchor)) => {
            json!({"state": "stale", "last_anchor": anchor_json(anchor)})
        }
    }
}

pub(crate) fn entity_json(entity: &EntityRecord) -> Json {
    json!({
        "entity_id": entity.id.get().to_string(),
        "kind": entity.kind.as_str(),
        "label": entity.label,
        "generation": entity.generation,
        "revision": entity.revision,
        "fields": entity.fields.iter()
            .map(|(name, fact)| (name.clone(), fact_value(fact)))
            .collect::<BTreeMap<_, _>>(),
        "field_presence": entity.fields.iter()
            .map(|(name, fact)| (name.clone(), presence_json(fact)))
            .collect::<BTreeMap<_, _>>(),
    })
}

/// A summary or failed read cannot certify the complete world just because
/// the server internally holds a complete laboratory model.
pub(crate) fn coverage(payload: &Json, current_observation_anchor: Option<&Json>) -> Json {
    let observed = payload.get("ok").and_then(Json::as_bool) == Some(true)
        && current_observation_anchor.is_some_and(Json::is_object);
    let mut complete = Vec::new();
    if observed {
        complete.push("laboratory.anchor");
        if payload.get("paused").is_some_and(Json::is_boolean) {
            complete.push("laboratory.pause_state");
        }
    }
    let mut covered = payload
        .get("observation_coverage")
        .filter(|value| observed && value.is_object())
        .cloned()
        .unwrap_or_else(|| {
            json!({
                "status": if observed { "complete_for_named_projection" } else { "partial" },
                "complete_domains": complete,
                "partial_domains": [],
                "absence_proof_scope": [],
                "continuation": payload.get("continuation"),
            })
        });
    covered["anchor"] = if observed {
        current_observation_anchor.cloned().unwrap_or(Json::Null)
    } else {
        Json::Null
    };
    let mut omitted = covered
        .get("omitted_domains")
        .and_then(Json::as_array)
        .cloned()
        .unwrap_or_default();
    omitted.extend([
        json!({"domain": "live_dwarf_fortress", "reason": "this laboratory session has no live observation source"}),
        json!({"domain": "dwarf_fortress_behaviour", "reason": "reference action semantics and calibrated laboratory rates"}),
    ]);
    covered["omitted_domains"] = json!(omitted);
    if !observed {
        covered["reason"] =
            json!("this response did not establish a successful anchored observation");
    }
    covered
}

#[cfg(test)]
mod tests {
    use super::*;
    use dfmcp_core::{Digest32, EntityId, FortressId, GameTick, ObservationCursor};
    use dfmcp_world::{EntityKind, FactSource};

    fn anchor() -> StateAnchor {
        StateAnchor {
            fortress_id: FortressId::new(7),
            cursor: ObservationCursor {
                epoch: 2,
                sequence: 9,
            },
            tick: GameTick(20),
            state_hash: Digest32::ZERO,
        }
    }

    #[test]
    fn every_presence_survives_without_exposing_unavailable_values() {
        for (presence, expected) in [
            (FactPresence::Known(Value::Null), "known"),
            (FactPresence::Absent, "absent"),
            (FactPresence::Unknown("missing read".into()), "unknown"),
            (
                FactPresence::Unsupported("manifest/1".into()),
                "unsupported",
            ),
            (FactPresence::Omitted("spatial".into()), "omitted"),
            (FactPresence::Redacted("policy/1".into()), "redacted"),
            (FactPresence::Stale(anchor()), "stale"),
        ] {
            let mut fact =
                Fact::with_presence(presence, GameTick(20), FactSource::Replay, Digest32::ZERO);
            if expected != "known" {
                fact.value = Value::Text("unavailable retained value".into());
            }
            assert!(fact_value(&fact).is_null());
            assert_eq!(presence_json(&fact)["state"], expected);
            assert!(
                !presence_json(&fact)
                    .to_string()
                    .contains("unavailable retained value")
            );
        }
    }

    #[test]
    fn legacy_known_and_unknown_optional_field_names_keep_their_values() {
        let fact = Fact::known(
            Value::List(vec![Value::U64(1), Value::Null]),
            GameTick(20),
            FactSource::AgentAssertion("untrusted input".into()),
            Digest32::ZERO,
        );
        let entity = EntityRecord {
            id: EntityId::new(4),
            generation: 2,
            revision: 3,
            kind: EntityKind::Unit,
            label: "Urist".into(),
            fields: BTreeMap::from([("future.optional".into(), fact)]),
        };
        let projected = entity_json(&entity);
        assert_eq!(projected["fields"]["future.optional"], json!([1, null]));
        assert_eq!(
            projected["field_presence"]["future.optional"]["state"],
            "known"
        );
    }

    #[test]
    fn inconsistent_known_value_is_not_presented_as_fact() {
        let mut fact = Fact::with_presence(
            FactPresence::Known(Value::Bool(true)),
            GameTick(20),
            FactSource::Replay,
            Digest32::ZERO,
        );
        fact.value = Value::Bool(false);
        assert!(fact_value(&fact).is_null());
        assert_eq!(presence_json(&fact)["state"], "unknown");
    }

    #[test]
    fn failed_or_unanchored_responses_certify_no_domain() {
        for payload in [
            json!({"ok": false, "anchor": anchor_json(&anchor())}),
            json!({"ok": true}),
        ] {
            let covered = coverage(&payload, payload.get("anchor"));
            assert_eq!(covered["status"], "partial");
            assert_eq!(covered["complete_domains"], json!([]));
            assert_eq!(covered["absence_proof_scope"], json!([]));
        }
    }

    #[test]
    fn query_coverage_keeps_scope_and_does_not_claim_the_whole_world() {
        let payload = json!({"ok": true, "anchor": anchor_json(&anchor()),
            "observation_coverage": {"status": "partial", "complete_domains": [],
                "partial_domains": [{"domain": "laboratory.entities", "unknown_filter_rows": 2}],
                "absence_proof_scope": [], "continuation": {"offset": 5}}});
        let covered = coverage(&payload, payload.get("anchor"));
        assert_eq!(covered["continuation"]["offset"], 5);
        assert_eq!(covered["partial_domains"][0]["unknown_filter_rows"], 2);
        assert!(!covered.to_string().contains("laboratory.world_model"));
    }
    #[test]
    fn nested_binary_summaries_are_not_reported_as_complete_known_values() {
        let value = Value::Object(BTreeMap::from([(
            "future".into(),
            Value::List(vec![Value::Null, Value::Bytes(vec![7, 8, 9])]),
        )]));
        let fact = Fact::known(value, GameTick(20), FactSource::Replay, Digest32::ZERO);
        assert_eq!(fact_value(&fact), json!({"future": [null, {"bytes": 3}]}));
        assert_eq!(presence_json(&fact)["state"], "omitted");
        assert_eq!(presence_json(&fact)["source_state"], "known");
        assert_eq!(
            presence_json(&fact)["projection"],
            "binary_payload_summarized"
        );
    }
    #[test]
    fn objective_projection_distinguishes_established_false_unknown_and_invalid() {
        for (truth, status, epistemic, predicate_truth) in [
            (PredicateTruth::True, "achieved", "certified_derived", "true"),
            (PredicateTruth::False, "not_yet_observed", "certified_derived", "false"),
            (PredicateTruth::Unknown, "not_yet_observed", "unknown", "unknown"),
        ] {
            let shown = laboratory_objective_evidence(Ok(truth));
            assert_eq!(shown["status"], status);
            assert_eq!(shown["epistemic_state"], epistemic);
            assert_eq!(shown["predicate_truth"], predicate_truth);
            assert_eq!(shown["evidence_scope"], "laboratory_reference_world");
            assert!(shown.get("observation_error").is_none());
        }
        let shown = laboratory_objective_evidence(Err(dfmcp_core::DfmcpError::new(
            dfmcp_core::ErrorCode::ChecksumMismatch,
            "goal observation hash mismatch",
        )));
        assert_eq!(shown["status"], "not_yet_observed");
        assert_eq!(shown["epistemic_state"], "unknown");
        assert_eq!(shown["predicate_truth"], "unknown");
        assert_eq!(shown["observation_error"]["code"], dfmcp_core::ErrorCode::ChecksumMismatch.as_str());
    }

    #[test]
    fn an_asserted_goal_never_becomes_achieved_through_projection() -> Result<()> {
        use dfmcp_world::{CompareOp, Predicate, PredicateEvidence, WorldGraph, WorldSnapshot};
        let id = EntityId::new(1);
        let make = |source| {
            let entity = EntityRecord {
                id, generation: 1, revision: 1, kind: EntityKind::Unit,
                label: "goal subject".to_owned(),
                fields: BTreeMap::from([("ready".to_owned(), Fact::known(
                    Value::Bool(true), GameTick(20), source, Digest32::ZERO,
                ))]),
            };
            WorldSnapshot::new(FortressId::new(7), GameTick(20), ObservationCursor::ORIGIN,
                true, WorldGraph { entities: BTreeMap::from([(id, entity)]), ..WorldGraph::default() })
        };
        let predicate = Predicate::FieldCompare { entity_id: id, field: "ready".to_owned(),
            op: CompareOp::Eq, value: Value::Bool(true) };
        for source in [FactSource::AgentAssertion("claimed ready".to_owned()),
            FactSource::Replay, FactSource::Derived("unregistered".to_owned())] {
            let snapshot = make(source);
            assert!(dfmcp_world::evaluate(&snapshot, &predicate));
            let evidence = PredicateEvidence::laboratory(&snapshot)?;
            let shown = laboratory_objective_evidence(evidence.evaluate(&predicate));
            assert_eq!(shown["status"], "not_yet_observed");
            assert_eq!(shown["epistemic_state"], "unknown");
        }
        let snapshot = make(FactSource::Derived("dfmcp.lab-scenario/1".to_owned()));
        let evidence = PredicateEvidence::laboratory(&snapshot)?;
        assert_eq!(laboratory_objective_evidence(evidence.evaluate(&predicate))["status"], "achieved");
        Ok(())
    }

}
