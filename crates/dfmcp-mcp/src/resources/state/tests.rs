//! `dfmcp-reality-wiring-proof-4bf.11`: the resource consumer, not a parallel renderer.

use super::*;
use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{Digest32, EntityId, FortressId, GameTick, ObservationCursor};
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::{EntityKind, EntityRecord, Fact, FactPresence, FactSource, WorldGraph};

fn open(fortress: u64, tokens: u32) -> String {
    let raw = crate::server::open_session_in_scenario(
        Some(true),
        Some(fortress.to_string()),
        None,
        None,
        None,
        None,
        None,
        Some(tokens),
        None,
        Some("starter_fortress".to_owned()),
        Some(false),
        Some(false),
    );
    let payload: Value = serde_json::from_str(&raw).expect("open JSON");
    assert_eq!(payload["ok"], true, "{payload}");
    payload["session_id"]
        .as_str()
        .expect("session id")
        .to_owned()
}

fn fixture(fortress: u64, tick: u64, count: u64) -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    for index in 1..=count {
        let unknown = Fact::with_presence(
            FactPresence::Unknown("not observed".to_owned()),
            GameTick(tick),
            FactSource::Replay,
            Digest32::ZERO,
        );
        let entity = EntityRecord {
            id: EntityId::new(index),
            generation: 1,
            revision: tick,
            kind: EntityKind::Unit,
            label: format!("citizen-{index:03}-{}", "Ѭ".repeat(200)),
            fields: BTreeMap::from([("uncertain".to_owned(), unknown)]),
        };
        graph.entities.insert(entity.id, entity);
    }
    WorldSnapshot::new(
        FortressId::new(fortress),
        GameTick(tick),
        ObservationCursor {
            epoch: 1,
            sequence: tick,
        },
        true,
        graph,
    )
}

fn install(session_id: &str, snapshot: WorldSnapshot) {
    let session = resolve_session(Some(session_id.to_owned())).expect("session");
    with_session(
        &session,
        || panic!("session unavailable"),
        |guard| {
            guard.adapter = MemoryAdapter::new(snapshot);
        },
    );
}

fn view(session_id: &str, view: &str) -> McpResult<Vec<ResourceContent>> {
    read(
        session_id,
        &format!("df://session/{session_id}/{view}"),
        view,
    )
}

fn payload(content: Vec<ResourceContent>) -> Value {
    assert_eq!(content.len(), 1);
    serde_json::from_str(content[0].text.as_deref().expect("JSON resource")).expect("resource JSON")
}

fn from_uri(session_id: &str, uri: &str) -> Vec<ResourceContent> {
    let prefix = format!("df://session/{session_id}/");
    read(
        session_id,
        uri,
        uri.strip_prefix(&prefix).expect("session prefix"),
    )
    .expect("resource read")
}

fn error_data(error: McpError, code: &str) -> Value {
    let data = error.data.expect("typed refusal");
    assert_eq!(data["error"]["code"], code);
    assert_eq!(data["error"]["effect_may_have_occurred"], false);
    assert_eq!(data["agent_turn"]["schema"], "dfmcp.agent_turn/1");
    assert_eq!(
        data["agent_turn"]["coverage"]["complete_domains"],
        json!([])
    );
    data
}

#[test]
fn address_parser_accepts_only_canonical_closed_resource_addresses() {
    let hash = "a".repeat(64);
    assert_eq!(
        address("state").expect("current"),
        Address {
            hash: None,
            page: None
        }
    );
    assert_eq!(
        address(&format!("state-{hash}")).expect("exact").hash,
        Some(hash.as_str())
    );
    for section in SECTIONS {
        assert_eq!(
            address(&format!("state-{hash}~{section}~12"))
                .expect("page")
                .page,
            Some((section, 12))
        );
    }
    for bad in ["", "state-", "states", "state-/x", "state-short"] {
        assert!(address(bad).is_err(), "{bad}");
    }
    for suffix in [
        "~entities",
        "~entities~",
        "~entities~-1",
        "~entities~+1",
        "~entities~01",
        "~memory~0",
        "~events~0~extra",
        "~chunks~184467440737095516160",
        "~entities~0?x=1",
    ] {
        assert!(
            address(&format!("state-{hash}{suffix}")).is_err(),
            "{suffix}"
        );
    }
    assert!(address(&format!("state-{}", "A".repeat(64))).is_err());
}

#[test]
fn manifest_and_every_section_use_the_existing_canonical_projection() {
    let id = open(390_001, 16_384);
    let manifest = payload(view(&id, "state").expect("manifest"));
    assert_eq!(manifest["scope"], "laboratory_reference_world");
    assert_eq!(manifest["game_mutation_dispatched"], false);
    assert_eq!(
        manifest["agent_turn"]["coverage"]["complete_domains"],
        json!([])
    );
    assert_eq!(manifest["sections"].as_array().map(Vec::len), Some(4));
    let session = resolve_session(Some(id.clone())).expect("session");
    let snapshot = session.lock().expect("lock").adapter.snapshot().clone();
    assert_eq!(manifest["anchor"], anchor_json(&snapshot.anchor()));
    assert_eq!(
        manifest["anchor_v2"],
        version_identity(&snapshot).expect("version")
    );
    for section in manifest["sections"].as_array().expect("sections") {
        let name = section["section"].as_str().expect("name");
        let uri = section["uri"].as_str().expect("URI");
        let read = payload(from_uri(&id, uri));
        let canonical = crate::lab_world::query(
            &snapshot,
            &json!({"mode":"observation",
            "completeness_profile":"research-full", "section":name, "limit":MAX_PAGE_RECORDS,
            "offset":0})
            .to_string(),
        )
        .expect("canonical projection");
        let returned = read["returned"].as_u64().expect("returned") as usize;
        assert_eq!(
            read["rows"],
            json!(&canonical["rows"].as_array().expect("rows")[..returned])
        );
        assert_eq!(read["source_anchor"], canonical["source_anchor"]);
        assert_eq!(read["profile_digest"], canonical["profile_digest"]);
        assert_eq!(read["total"], section["record_count"]);
    }
    assert_eq!(
        session.lock().expect("lock").adapter.snapshot().anchor(),
        snapshot.anchor()
    );
}

#[test]
fn pages_bound_escaped_bytes_and_keep_one_version_across_world_changes() {
    let id = open(390_002, 4_096);
    let source = fixture(390_002, 1, 47);
    let hash = source.state_hash.to_hex();
    install(&id, source);
    let mut uri = page_uri(&id, &hash, "entities", 0);
    let mut seen = BTreeSet::new();
    let mut pages = 0;
    loop {
        pages += 1;
        assert!(pages < 100);
        let contents = from_uri(&id, &uri);
        let text = contents[0].text.as_deref().expect("text");
        assert!(response_bytes(&uri, text) <= 4_096 * 4);
        assert!(!text.contains("must not escape an unknown fact"));
        let page = payload(contents);
        assert_eq!(page["canonical_uri"], uri);
        assert_eq!(page["anchor"]["state_hash"], hash);
        assert_eq!(page["agent_turn"]["anchor"], page["anchor"]);
        assert_eq!(page["complete_domain"], false);
        assert_eq!(page["absence_proven"], false);
        if pages == 1 {
            assert!(page["returned"].as_u64().expect("returned") < MAX_PAGE_RECORDS as u64);
            install(&id, fixture(390_002, 2, 48));
        } else {
            assert_eq!(page["historical"], true);
            assert_eq!(page["agent_turn"]["continuity"]["status"], "stale");
        }
        for row in page["rows"].as_array().expect("rows") {
            assert!(seen.insert(row["entity_id"].as_str().expect("entity id").to_owned()));
            assert_eq!(row["fields"]["uncertain"], Value::Null);
            assert_eq!(row["field_presence"]["uncertain"]["state"], "unknown");
        }
        match page["next"].as_str() {
            Some(next) => uri = next.to_owned(),
            None => {
                assert_eq!(page["complete"], true);
                break;
            }
        }
    }
    assert_eq!(seen.len(), 47);
    assert!(pages > 1);
    let retained = payload(view(&id, &format!("state-{hash}")).expect("old manifest"));
    assert_eq!(retained["sections"][0]["record_count"], 47);
    assert_eq!(retained["historical"], true);
}

#[test]
fn empty_records_prove_only_the_named_retained_domain_and_repeat_deterministically() {
    let id = open(390_003, 4_096);
    let snapshot = fixture(390_003, 1, 0);
    let hash = snapshot.state_hash.to_hex();
    install(&id, snapshot);
    let uri = page_uri(&id, &hash, "events", 0);
    let first = from_uri(&id, &uri);
    let second = from_uri(&id, &uri);
    assert_eq!(first[0].text, second[0].text);
    let page = payload(first);
    assert_eq!(page["absence_proven"], true);
    assert_eq!(page["complete_domain"], true);
    assert_eq!(
        page["observation_coverage"]["complete_domains"][0]["domain"],
        "laboratory.retained_event_records"
    );
    assert_eq!(
        page["agent_turn"]["uncertainty"][0]["domain"],
        "live_game_evidence"
    );
    let gap = error_data(
        view(&id, &format!("state-{hash}~events~1")).expect_err("offset past retained end"),
        "cursor_gap",
    );
    assert_eq!(gap["agent_turn"]["continuity"]["status"], "gap");
}

#[test]
fn foreign_and_unknown_versions_do_not_fall_back_to_the_current_world() {
    let id = open(390_004, 4_096);
    let other = open(390_005, 4_096);
    let other_hash = payload(view(&other, "state").expect("other manifest"))["state_hash"]
        .as_str()
        .expect("hash")
        .to_owned();
    for hash in [other_hash, "0".repeat(64)] {
        let data = error_data(
            view(&id, &format!("state-{hash}")).expect_err("unknown version"),
            "stale_anchor",
        );
        assert_eq!(data["agent_turn"]["continuity"]["status"], "gap");
        assert!(
            data["error"]["message"]
                .as_str()
                .expect("reason")
                .contains("never observed")
        );
    }
    assert!(read(&id, &format!("df://session/{other}/state"), "state").is_err());
    assert!(view("00000000000000000000000000000000ff", "state").is_err());
    for malformed in ["", "0", "not-a-session"] {
        assert!(view(malformed, "state").is_err());
    }
}

#[test]
fn reading_a_resource_does_not_pin_collected_versions() {
    let id = open(390_006, 4_096);
    let initial = fixture(390_006, 1, 1);
    let hash = initial.state_hash.to_hex();
    install(&id, initial);
    assert!(view(&id, &format!("state-{hash}")).is_ok());
    for tick in 2..=40 {
        install(&id, fixture(390_006, tick, 1));
    }
    let data = error_data(
        view(&id, &format!("state-{hash}")).expect_err("collected"),
        "stale_anchor",
    );
    assert!(
        data["error"]["message"]
            .as_str()
            .expect("reason")
            .contains("was collected")
    );
}

#[test]
fn historical_reads_recheck_revocation_and_expiry_against_current_tick() {
    for expired in [false, true] {
        let id = open(390_007, 4_096);
        let initial = fixture(390_007, 1, 1);
        let hash = initial.state_hash.to_hex();
        install(&id, initial);
        install(&id, fixture(390_007, 10, 1));
        let session = resolve_session(Some(id.clone())).expect("session");
        {
            let mut guard = session.lock().expect("lock");
            if expired {
                for grant in &mut guard.grants {
                    grant.expires_at_tick = Some(GameTick(2));
                }
            } else {
                guard.grants.clear();
            }
        }
        let data = error_data(
            view(&id, &format!("state-{hash}")).expect_err("denied"),
            "capability_denied",
        );
        assert_eq!(data["agent_turn"]["anchor"], Value::Null);
    }
}

#[test]
fn entity_scoped_grants_do_not_authorize_global_state_resources() {
    let id = open(390_008, 4_096);
    let session = resolve_session(Some(id.clone())).expect("session");
    for grant in &mut session.lock().expect("lock").grants {
        grant.scope.entity_ids.insert(EntityId::new(1));
    }
    let data = error_data(view(&id, "state").expect_err("denied"), "capability_denied");
    assert_eq!(data["agent_turn"]["anchor"], Value::Null);
}

#[test]
fn source_work_and_indivisible_metadata_have_explicit_budget_refusals() {
    let id = open(390_009, 4_096);
    let initial = fixture(390_009, 1, 2);
    let hash = initial.state_hash.to_hex();
    install(&id, initial);
    let session = resolve_session(Some(id.clone())).expect("session");
    session.lock().expect("lock").budget.max_entities = 1;
    error_data(
        view(&id, &format!("state-{hash}~entities~0")).expect_err("source budget"),
        "budget_exceeded",
    );
    session.lock().expect("lock").budget.max_output_tokens = 1;
    error_data(
        view(&id, "state").expect_err("metadata budget"),
        "budget_exceeded",
    );
}

#[test]
fn contradictory_unknown_facts_are_refused_without_revealing_retained_values() {
    let id = open(390_010, 4_096);
    let mut snapshot = fixture(390_010, 1, 1);
    snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(1))
        .expect("entity")
        .fields
        .get_mut("uncertain")
        .expect("unknown fact")
        .value = dfmcp_world::Value::Text("must not escape an unknown fact".to_owned());
    snapshot.refresh_hash();
    let hash = snapshot.state_hash.to_hex();
    install(&id, snapshot);
    let data = error_data(
        view(&id, &format!("state-{hash}~entities~0")).expect_err("canonical presence invariant"),
        "invalid_request",
    );
    assert!(!data.to_string().contains("must not escape an unknown fact"));
}
