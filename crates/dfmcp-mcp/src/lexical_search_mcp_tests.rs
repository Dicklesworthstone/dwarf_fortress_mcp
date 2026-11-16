#![forbid(unsafe_code)]

//! Public laboratory lexical retrieval keeps the canonical presence boundary.
//! Owning roadmap bead: dfmcp-reality-wiring-proof-4bf.9 (not closed by this slice).

use std::collections::BTreeMap;

use dfmcp_core::{Capability, Digest32, EntityId, EventId, GameTick};
use dfmcp_world::{
    EntityKind, EntityRecord, Fact, FactPresence, FactSource, Value as WorldValue, WorldEvent,
    WorldEventKind, WorldSnapshot,
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
            let mut fact =
                Fact::with_presence(presence, snapshot.tick, FactSource::Replay, Digest32::ZERO);
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

#[test]
fn public_lexical_search_accepts_long_announcements_without_losing_their_suffix() -> TestResult {
    let _serial = crate::test_serial();
    let (session_id, _) = open_fixture()?;
    let session = crate::server::resolve_session(Some(session_id.clone()))?;
    let snapshot = {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        let mut snapshot = guard.adapter.snapshot().clone();
        let event = WorldEvent {
            id: EventId::new(96_102),
            tick: snapshot.tick,
            kind: WorldEventKind::Announcement,
            subject: None,
            summary: format!("{}🪨 tailannouncementmarker", "岩".repeat(85)),
            fields: BTreeMap::new(),
        };
        snapshot.graph.events.insert(event.id, event);
        snapshot.cursor = snapshot.cursor.checked_next().ok_or("cursor exhausted")?;
        snapshot.refresh_hash();
        guard.adapter.inject_snapshot(snapshot.clone())?;
        snapshot
    };
    let expected_anchor = crate::observation_projection::anchor_json(&snapshot.anchor());
    let result = search(&session_id, "tailannouncementmarker")?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["returned"], 1);
    assert_eq!(result["hits"][0]["event_id"], "96102");
    assert_eq!(result["hits"][0]["entity_id"], Value::Null);
    assert_eq!(
        result["hits"][0]["title"],
        format!("{}...", "岩".repeat(84))
    );
    assert_eq!(result["anchor"], expected_anchor);
    assert_eq!(result["agent_turn"]["anchor"], expected_anchor);
    assert_eq!(
        result["agent_turn"]["coverage"]["absence_proof_scope"],
        json!([])
    );

    // One long event must not disable unrelated entity retrieval.
    let unrelated = search(&session_id, "needle")?;
    assert_eq!(unrelated["ok"], true, "{unrelated}");
    assert_eq!(unrelated["returned"], 1);
    assert_eq!(unrelated["hits"][0]["entity_id"], SEARCH_ENTITY.to_string());
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        assert_eq!(guard.adapter.snapshot(), &snapshot);
        guard
            .grants
            .retain(|grant| grant.capability != Capability::Query);
    }
    let denied = search(&session_id, "tailannouncementmarker")?;
    assert_eq!(denied["ok"], false, "{denied}");
    assert_eq!(denied["error"]["code"], "capability_denied");
    assert!(denied.get("hits").is_none());
    Ok(())
}

#[test]
fn public_lexical_search_uses_selected_source_time_and_preserves_current_authority() -> TestResult {
    let _serial = crate::test_serial();
    let (session_id, initial) = open_fixture()?;
    let baseline_hits = search(&session_id, "needle")?["hits"].clone();
    let future_tick = GameTick(initial.tick.0.checked_add(1).ok_or("tick exhausted")?);
    let session = crate::server::resolve_session(Some(session_id.clone()))?;
    let earlier = {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        let mut snapshot = guard.adapter.snapshot().clone();
        let entity = snapshot
            .graph
            .entities
            .get_mut(&EntityId::new(SEARCH_ENTITY))
            .ok_or("search entity missing")?;
        entity.fields.insert(
            "futurelegacyfield".to_owned(),
            Fact::known(
                WorldValue::Text("needle futurelegacymarker".to_owned()),
                future_tick,
                FactSource::Replay,
                Digest32::ZERO,
            ),
        );
        entity.fields.insert(
            "futureexplicitfield".to_owned(),
            Fact::with_presence(
                FactPresence::Known(WorldValue::Text("futureexplicitmarker".to_owned())),
                future_tick,
                FactSource::Replay,
                Digest32::ZERO,
            ),
        );
        let event = WorldEvent {
            id: EventId::new(96_103),
            tick: future_tick,
            kind: WorldEventKind::Announcement,
            subject: Some(entity.id),
            summary: "needle futureeventmarker".to_owned(),
            fields: BTreeMap::from([(
                "futureeventfield".to_owned(),
                WorldValue::Text("futureeventpayload".to_owned()),
            )]),
        };
        snapshot.graph.events.insert(event.id, event);
        snapshot.cursor = snapshot.cursor.checked_next().ok_or("cursor exhausted")?;
        snapshot.refresh_hash();
        guard.adapter.inject_snapshot(snapshot.clone())?;
        guard.history.record(&snapshot);
        snapshot
    };
    let earlier_anchor = crate::observation_projection::anchor_json(&earlier.anchor());
    let future_markers = [
        "futurelegacyfield",
        "futurelegacymarker",
        "futureexplicitfield",
        "futureexplicitmarker",
        "futureeventmarker",
        "futureeventfield",
        "futureeventpayload",
    ];
    for marker in future_markers {
        let result = search(&session_id, marker)?;
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["hits"], json!([]), "{result}");
        assert_eq!(result["anchor"], earlier_anchor);
        assert_eq!(result["agent_turn"]["anchor"], earlier_anchor);
        assert_eq!(
            result["agent_turn"]["coverage"]["absence_proof_scope"],
            json!([])
        );
    }
    assert_eq!(search(&session_id, "needle")?["hits"], baseline_hits);

    let later = {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        assert_eq!(guard.adapter.snapshot(), &earlier);
        let mut snapshot = earlier.clone();
        snapshot.tick = future_tick;
        snapshot.cursor = snapshot.cursor.checked_next().ok_or("cursor exhausted")?;
        snapshot.refresh_hash();
        guard.adapter.inject_snapshot(snapshot.clone())?;
        snapshot
    };
    for marker in future_markers {
        let result = search(&session_id, marker)?;
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["returned"], 1, "{result}");
    }

    // A later current world must not make a retained earlier source eligible
    // for facts or reports that were still in that source's future.
    let historical_mode = |text| {
        json!({"mode":"search","text":text,"limit":5,"at":earlier.state_hash.to_hex()})
            .to_string()
    };
    for marker in future_markers {
        let result = parsed(&fortress_query(
            Some(session_id.clone()),
            Some(historical_mode(marker)),
        ))?;
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["hits"], json!([]), "{result}");
        assert_eq!(result["historical"], true);
        assert_eq!(result["anchor"], earlier_anchor);
        assert_eq!(
            result["current_anchor"],
            crate::observation_projection::anchor_json(&later.anchor())
        );
    }
    let historical_visible = parsed(&fortress_query(
        Some(session_id.clone()),
        Some(historical_mode("needle")),
    ))?;
    assert_eq!(historical_visible["ok"], true, "{historical_visible}");
    assert_eq!(historical_visible["hits"], baseline_hits);
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        assert_eq!(guard.adapter.snapshot(), &later);
        guard
            .grants
            .retain(|grant| grant.capability != Capability::Query);
    }
    let denied = parsed(&fortress_query(
        Some(session_id),
        Some(historical_mode("needle")),
    ))?;
    assert_eq!(denied["ok"], false, "{denied}");
    assert_eq!(denied["error"]["code"], "capability_denied");
    assert!(denied.get("hits").is_none());
    Ok(())
}
