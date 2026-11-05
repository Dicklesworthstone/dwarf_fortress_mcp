#![forbid(unsafe_code)]

//! Public laboratory lexical retrieval keeps the canonical presence boundary.
//! Owning roadmap bead: dfmcp-reality-wiring-proof-4bf.9 (not closed by this slice).

use std::collections::BTreeMap;

use dfmcp_core::{Capability, Digest32, EntityId};
use dfmcp_world::{
    EntityKind, EntityRecord, Fact, FactPresence, FactSource, Value as WorldValue, WorldSnapshot,
};
use serde_json::{Value, json};

use crate::agent_facade::{fortress_open_session, fortress_query};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

const SEARCH_ENTITY: u64 = 96_101;
const HIDDEN_MARKERS: [&str; 8] = [
    "absentpayload",
    "unknownpayload",
    "unsupportedpayload",
    "omittedpayload",
    "redactedpayload",
    "stalepayload",
    "mismatchedpayload",
    "claimedpayload",
];

fn parsed(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn open_fixture() -> TestResult<(String, WorldSnapshot)> {
    let opened = parsed(&fortress_open_session(
        Some(true),
        None,
        Some(vec![
            ("observe".to_owned(), "read_only".to_owned()),
            ("query".to_owned(), "read_only".to_owned()),
        ]),
        None,
        None,
        None,
        None,
        Some(8_192),
        None,
        Some("empty".to_owned()),
        Some(false),
        Some(false),
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    let session_id = opened["session_id"]
        .as_str()
        .ok_or("session_id missing")?
        .to_owned();
    let session = crate::server::resolve_session(Some(session_id.clone()))?;
    let snapshot = {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        let mut snapshot = guard.adapter.snapshot().clone();
        let mut fields = BTreeMap::from([
            (
                "legacy_visible".to_owned(),
                Fact::known(
                    WorldValue::Text("legacymarker".to_owned()),
                    snapshot.tick,
                    FactSource::Replay,
                    Digest32::ZERO,
                ),
            ),
            (
                "explicit_visible".to_owned(),
                Fact::with_presence(
                    FactPresence::Known(WorldValue::Text("explicitmarker".to_owned())),
                    snapshot.tick,
                    FactSource::Replay,
                    Digest32::ZERO,
                ),
            ),
        ]);
        let presences = [
            FactPresence::Absent,
            FactPresence::Unknown("unavailable".to_owned()),
            FactPresence::Unsupported("unsupported".to_owned()),
            FactPresence::Omitted("projection".to_owned()),
            FactPresence::Redacted("policy".to_owned()),
            FactPresence::Stale(snapshot.anchor()),
            FactPresence::Known(WorldValue::Text(HIDDEN_MARKERS[7].to_owned())),
        ];
        for (index, presence) in presences.into_iter().enumerate() {
            let mut fact = Fact::with_presence(
                presence,
                snapshot.tick,
                FactSource::Replay,
                Digest32::ZERO,
            );
            fact.value = WorldValue::Text(HIDDEN_MARKERS[index].to_owned());
            fields.insert(format!("unavailable_{index}"), fact);
        }
        let id = EntityId::new(SEARCH_ENTITY);
        snapshot.graph.entities.insert(
            id,
            EntityRecord {
                id,
                kind: EntityKind::Other("searchsubject".to_owned()),
                generation: 1,
                revision: 1,
                label: "Needle specimen".to_owned(),
                fields,
            },
        );
        snapshot.cursor = snapshot.cursor.checked_next().ok_or("cursor exhausted")?;
        snapshot.refresh_hash();
        guard.adapter.inject_snapshot(snapshot.clone())?;
        snapshot
    };
    Ok((session_id, snapshot))
}

fn search(session_id: &str, text: &str) -> TestResult<Value> {
    parsed(&fortress_query(
        Some(session_id.to_owned()),
        Some(json!({"mode":"search","text":text,"limit":5}).to_string()),
    ))
}

#[test]
fn public_lexical_query_preserves_presence_kind_and_current_query_authority() -> TestResult {
    let _serial = crate::test_serial();
    let (session_id, snapshot) = open_fixture()?;
    let expected_anchor = crate::observation_projection::anchor_json(&snapshot.anchor());

    for marker in HIDDEN_MARKERS {
        let result = search(&session_id, marker)?;
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["returned"], 0, "{result}");
        assert_eq!(result["hits"], json!([]));
        assert_eq!(result["anchor"], expected_anchor);
        assert_eq!(result["agent_turn"]["anchor"], expected_anchor);
        assert_eq!(
            result["agent_turn"]["coverage"]["absence_proof_scope"],
            json!([])
        );
    }
    for text in ["needle", "legacymarker", "explicitmarker", "searchsubject"] {
        let result = search(&session_id, text)?;
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["returned"], 1, "{result}");
        assert_eq!(result["hits"][0]["entity_id"], SEARCH_ENTITY.to_string());
        assert_eq!(result["anchor"], expected_anchor);
        assert_eq!(result["agent_turn"]["operation"], "fortress.query");
        let visible_hits = result["hits"].to_string();
        assert!(
            HIDDEN_MARKERS
                .iter()
                .all(|marker| !visible_hits.contains(*marker)),
            "{visible_hits}"
        );
    }

    let session = crate::server::resolve_session(Some(session_id.clone()))?;
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        assert_eq!(guard.adapter.snapshot(), &snapshot);
        guard
            .grants
            .retain(|grant| grant.capability != Capability::Query);
    }
    let denied = search(&session_id, "legacymarker")?;
    assert_eq!(denied["ok"], false, "{denied}");
    assert_eq!(denied["error"]["code"], "capability_denied");
    assert!(denied.get("hits").is_none());
    Ok(())
}

#[test]
fn public_lexical_query_withdraws_a_fact_when_the_current_presence_changes() -> TestResult {
    let _serial = crate::test_serial();
    let (session_id, before) = open_fixture()?;
    assert_eq!(search(&session_id, "legacymarker")?["returned"], 1);
    let session = crate::server::resolve_session(Some(session_id.clone()))?;
    let after = {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        let mut snapshot = guard.adapter.snapshot().clone();
        let entity = snapshot
            .graph
            .entities
            .get_mut(&EntityId::new(SEARCH_ENTITY))
            .ok_or("search entity missing")?;
        entity
            .fields
            .get_mut("legacy_visible")
            .ok_or("search field missing")?
            .presence = Some(FactPresence::Redacted("policy/2".to_owned()));
        entity.revision += 1;
        snapshot.cursor = snapshot.cursor.checked_next().ok_or("cursor exhausted")?;
        snapshot.refresh_hash();
        guard.adapter.inject_snapshot(snapshot.clone())?;
        snapshot
    };

    let withdrawn = search(&session_id, "legacymarker")?;
    assert_eq!(withdrawn["ok"], true, "{withdrawn}");
    assert_eq!(withdrawn["returned"], 0);
    assert_eq!(withdrawn["hits"], json!([]));
    assert_eq!(
        withdrawn["anchor"],
        crate::observation_projection::anchor_json(&after.anchor())
    );
    assert_ne!(after.anchor(), before.anchor());
    let visible = search(&session_id, "needle")?;
    assert_eq!(visible["returned"], 1);
    assert!(!visible["hits"].to_string().contains("legacymarker"));
    assert_eq!(
        session
            .lock()
            .map_err(|_| "session poisoned")?
            .adapter
            .snapshot(),
        &after
    );
    Ok(())
}
