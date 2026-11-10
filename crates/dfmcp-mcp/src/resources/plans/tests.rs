//! Regression coverage for exact original-plan resource consumers.

use super::*;
use crate::server::{
    fortress_commit, fortress_open_session, fortress_plan, objectives_json, resource_replay_bundle,
    with_session,
};
use dfmcp_core::{EntityId, GameTick};
use dfmcp_intent::PreparedPlan;

fn open() -> String {
    let raw = fortress_open_session(
        Some(true),
        None,
        None,
        None,
        None,
        None,
        None,
        Some(16_384),
        None,
    );
    let value: Value = serde_json::from_str(&raw).expect("session JSON");
    assert_eq!(value["ok"], true, "{value}");
    value["session_id"].as_str().expect("session").to_owned()
}

fn plan(session: &str, summary: &str, paused: bool) -> Value {
    let raw = fortress_plan(
        Some(session.to_owned()),
        Some(summary.to_owned()),
        Some(paused),
    );
    let value: Value = serde_json::from_str(&raw).expect("plan JSON");
    assert_eq!(value["ok"], true, "{value}");
    value
}

fn commit(session: &str, prepared: &Value) {
    let raw = fortress_commit(
        Some(session.to_owned()),
        prepared["plan_digest"]
            .as_str()
            .expect("plan digest")
            .to_owned(),
    );
    let value: Value = serde_json::from_str(&raw).expect("commit JSON");
    assert_eq!(value["ok"], true, "{value}");
}

fn view(session: &str, name: &str) -> McpResult<Vec<ResourceContent>> {
    read(session, &format!("df://session/{session}/{name}"), name)
}

fn from_uri(session: &str, uri: &str) -> McpResult<Vec<ResourceContent>> {
    let prefix = format!("df://session/{session}/");
    read(
        session,
        uri,
        uri.strip_prefix(&prefix).expect("session prefix"),
    )
}

fn payload(contents: Vec<ResourceContent>) -> Value {
    assert_eq!(contents.len(), 1);
    serde_json::from_str(contents[0].text.as_deref().expect("JSON text")).expect("resource JSON")
}

fn error_data(error: McpError, code: &str) -> Value {
    let data = error.data.expect("structured error");
    assert_eq!(data["error"]["code"], code, "{data}");
    assert_eq!(data["error"]["effect_may_have_occurred"], false);
    assert_eq!(data["agent_turn"]["schema"], "dfmcp.agent_turn/1");
    assert_eq!(
        data["agent_turn"]["coverage"]["complete_domains"],
        json!([])
    );
    data
}

fn original(session: &str, digest: &str) -> PreparedPlan {
    let session = resolve_session(Some(session.to_owned())).expect("session");
    with_session_read(
        &session,
        |error| panic!("{error}"),
        |guard| {
            plan_resources::entries(guard)
                .find(|entry| entry.digest.to_hex() == digest)
                .and_then(|entry| entry.plan)
                .expect("exact plan")
                .clone()
        },
    )
}

fn fingerprint(session: &str) -> Value {
    let session = resolve_session(Some(session.to_owned())).expect("session");
    with_session_read(
        &session,
        |error| panic!("{error}"),
        |guard| {
            json!({
                "next_request_id": read_context(guard).request_id.to_string(),
                "anchor": anchor_json(&guard.adapter.snapshot().anchor()),
                "replay": resource_replay_bundle(guard),
                "objectives": objectives_json(guard),
            })
        },
    )
}

fn append_page(bytes: &mut Vec<u8>, page: &Value) {
    assert_eq!(page["offset"].as_u64(), Some(bytes.len() as u64));
    assert_eq!(page["encoding"], "hex");
    assert_eq!(page["format"], "DFMPAR01");
    let text = page["data_hex"].as_str().expect("hex");
    assert_eq!(text.len() % 2, 0);
    assert_eq!(
        page["returned_bytes"].as_u64(),
        Some((text.len() / 2) as u64)
    );
    for offset in (0..text.len()).step_by(2) {
        bytes.push(u8::from_str_radix(&text[offset..offset + 2], 16).expect("hex byte"));
    }
}

#[test]
fn only_canonical_closed_plan_addresses_are_accepted() {
    let a = "a".repeat(64);
    let b = "b".repeat(64);
    assert_eq!(
        address("plans").expect("catalog"),
        Address::Catalog {
            digest: None,
            offset: 0
        }
    );
    assert!(address(&format!("plans-{a}~0")).is_ok());
    assert!(address(&format!("plan-{a}~{b}")).is_ok());
    assert!(address(&format!("plan-{a}~{b}~archive~123")).is_ok());
    for bad in [
        String::new(),
        "plan".to_owned(),
        format!("plan-{a}"),
        format!("plan-{a}~{}", "B".repeat(64)),
        format!("plan-{a}~{b}~steps~0"),
        format!("plan-{a}~{b}~archive"),
        format!("plan-{a}~{b}~archive~-1"),
        format!("plan-{a}~{b}~archive~+1"),
        format!("plan-{a}~{b}~archive~01"),
        format!("plan-{a}~{b}~archive~0~extra"),
        format!("plan-{a}~{b}~archive~0?x=1"),
        format!("plans-{a}~01"),
        format!("plans-{a}~0~extra"),
        format!("plans-{a}~184467440737095516160"),
    ] {
        assert!(address(&bad).is_err(), "{bad}");
    }
}

#[test]
fn empty_catalog_is_not_a_goal_or_work_absence_proof() {
    let id = open();
    let result = payload(view(&id, "plans").expect("empty catalog"));
    assert_eq!(result["rows"], json!([]));
    assert_eq!(result["complete_catalog"], true);
    assert_eq!(result["coverage"]["complete_domains"], json!([]));
    assert_eq!(result["coverage"]["absence_proof_scope"], json!([]));
    assert_eq!(
        result["agent_turn"]["active_work"]["coverage"]["status"],
        "omitted"
    );
    assert_eq!(result["game_mutation_dispatched"], false);
}

#[test]
fn pending_manifest_and_archive_recover_the_exact_typed_plan_without_side_effects() {
    let id = open();
    let prepared = plan(&id, "Inspect an exact original plan", false);
    let digest = prepared["plan_digest"].as_str().expect("digest");
    let expected = original(&id, digest);
    let session = resolve_session(Some(id.clone())).expect("session");
    session.lock().expect("lock").budget.max_output_tokens = 4_096;
    let before = fingerprint(&id);
    let catalog = payload(view(&id, "plans").expect("catalog"));
    let row = &catalog["rows"][0];
    assert_eq!(row["retention"], "pending_plan");
    assert_eq!(row["plan_digest"], digest);
    let manifest = payload(from_uri(&id, row["uri"].as_str().expect("URI")).expect("manifest"));
    assert_eq!(manifest["goal_truth"], "not_evaluated");
    assert_eq!(manifest["dispatch_authority"], false);
    assert_eq!(manifest["inspection_projection"], true);
    assert_eq!(manifest["anchor"], anchor_json(&expected.anchor));
    assert_eq!(manifest["steps"], expected.steps.len());

    let seal = PlanArchiveSeal {
        plan_digest: expected.digest,
        object_digest: super::digest(row["object_digest"].as_str().expect("object"))
            .expect("digest"),
    };
    let mut uri = row["archive"].as_str().expect("archive").to_owned();
    let mut bytes = Vec::new();
    loop {
        let contents = from_uri(&id, &uri).expect("archive page");
        assert!(
            super::super::task_resource_bytes(&uri, contents[0].text.as_deref().expect("text"))
                <= 4_096 * 4
        );
        let page = payload(contents);
        assert_eq!(page["plan_digest"], digest);
        assert_eq!(page["object_digest"], seal.object_digest.to_hex());
        assert_eq!(page["agent_turn"]["anchor"], manifest["anchor"]);
        append_page(&mut bytes, &page);
        let Some(next) = page["next"].as_str() else {
            assert_eq!(page["complete"], true);
            break;
        };
        uri = next.to_owned();
    }
    assert_eq!(
        manifest["archive"]["sha256"],
        Digest32::of_bytes(&bytes).to_hex()
    );
    assert_eq!(
        plan_archive::decode(&bytes, seal, ArchiveLimits::default()).expect("lossless archive"),
        expected,
    );
    assert_eq!(
        fingerprint(&id),
        before,
        "resources cannot record a semantic turn or certify a goal"
    );
}

#[test]
fn retained_archive_pages_survive_new_pending_plans_and_world_changes() {
    let id = open();
    let prepared = plan(&id, &"archive history ".repeat(15), false);
    let digest = prepared["plan_digest"].as_str().expect("digest").to_owned();
    let expected = original(&id, &digest);
    let (expected_bytes, seal) =
        plan_archive::encode(&expected, ArchiveLimits::default()).expect("archive");
    commit(&id, &prepared);
    let session = resolve_session(Some(id.clone())).expect("session");
    let mut uri = archive_uri(&id, seal, 0);
    let full = from_uri(&id, &uri).expect("unclipped page");
    let wire_bytes =
        super::super::task_resource_bytes(&uri, full[0].text.as_deref().expect("text"));
    let semantic_budget = session.lock().expect("lock").budget;
    let mut page_budget = semantic_budget;
    page_budget.max_bytes = wire_bytes as u64 - 64;
    assert!(page_budget.max_bytes > expected_bytes.len() as u64);
    session.lock().expect("lock").budget = page_budget;
    let mut bytes = Vec::new();
    let mut page_count = 0;
    loop {
        let contents = from_uri(&id, &uri).expect("retained page");
        assert!(
            super::super::task_resource_bytes(&uri, contents[0].text.as_deref().expect("text"))
                as u64
                <= page_budget.max_bytes
        );
        let page = payload(contents);
        assert_eq!(page["anchor"], anchor_json(&expected.anchor));
        assert_eq!(page["historical"], true);
        assert_eq!(page["agent_turn"]["continuity"]["status"], "stale");
        assert_eq!(
            page["agent_turn"]["coverage"]["complete_domains"],
            json!([])
        );
        if page_count != 0 {
            assert_eq!(page["complete_object"], false);
        }
        append_page(&mut bytes, &page);
        if page_count == 0 {
            assert!(
                page["next"].is_string(),
                "wire bound must force a complete prefix"
            );
        }
        page_count += 1;
        let Some(next) = page["next"].as_str() else {
            break;
        };
        uri = next.to_owned();
        if page_count == 1 {
            session.lock().expect("lock").budget = semantic_budget;
            let replacement = plan(
                &id,
                "Change current world while inspecting its history",
                true,
            );
            commit(&id, &replacement);
            plan(&id, "A different pending plan", false);
            session.lock().expect("lock").budget = page_budget;
        }
    }
    assert!(page_count > 1);
    assert_eq!(bytes, expected_bytes);
    assert_eq!(
        plan_archive::decode(&bytes, seal, ArchiveLimits::default()).expect("decode"),
        expected
    );
}

#[test]
fn replaced_pending_foreign_and_wrong_object_references_never_fall_back() {
    let id = open();
    let prepared = plan(&id, "Original pending", false);
    let original_plan = original(&id, prepared["plan_digest"].as_str().expect("digest"));
    let (_, seal) =
        plan_archive::encode(&original_plan, ArchiveLimits::default()).expect("archive");
    let other = open();
    error_data(
        from_uri(&other, &plan_uri(&other, seal)).expect_err("private plan"),
        "stale_anchor",
    );
    let wrong = PlanArchiveSeal {
        object_digest: Digest32::of_bytes(b"wrong typed object"),
        ..seal
    };
    error_data(
        from_uri(&id, &plan_uri(&id, wrong)).expect_err("wrong object"),
        "stale_anchor",
    );
    error_data(
        from_uri(&id, &archive_uri(&id, seal, usize::MAX)).expect_err("offset"),
        "invalid_request",
    );
    let catalog = payload(view(&id, "plans").expect("catalog"));
    let pinned_catalog = catalog["canonical_uri"]
        .as_str()
        .expect("catalog URI")
        .to_owned();
    plan(&id, "Replacement pending", false);
    error_data(
        from_uri(&id, &plan_uri(&id, seal)).expect_err("replaced plan"),
        "stale_anchor",
    );
    error_data(
        from_uri(&id, &pinned_catalog).expect_err("catalog gap"),
        "cursor_gap",
    );
    assert!(read(&id, &format!("df://session/{other}/plans"), "plans").is_err());
}

#[test]
fn both_whole_fortress_grants_are_checked_before_plan_existence() {
    for capability in [Capability::Observe, Capability::Query] {
        for narrowed in [false, true] {
            let id = open();
            plan(&id, "Private plan", false);
            let session = resolve_session(Some(id.clone())).expect("session");
            {
                let mut guard = session.lock().expect("lock");
                if narrowed {
                    for grant in &mut guard.grants {
                        if grant.capability == capability {
                            grant.scope.entity_ids.insert(EntityId::new(1));
                        }
                    }
                } else {
                    guard.grants.retain(|grant| grant.capability != capability);
                }
            }
            let denied = error_data(
                view(&id, "plans").expect_err("grant denied"),
                "capability_denied",
            );
            assert_eq!(denied["agent_turn"]["anchor"], Value::Null);
            let unknown = format!("plan-{}~{}", "a".repeat(64), "b".repeat(64));
            let denied = error_data(
                view(&id, &unknown).expect_err("existence denied"),
                "capability_denied",
            );
            assert_eq!(denied["agent_turn"]["anchor"], Value::Null);
        }
    }
}

#[test]
fn shared_world_expiry_is_checked_at_current_tick_for_old_plan_bytes() {
    fn shared() -> String {
        let raw = crate::server::open_session_in_scenario(
            Some(true),
            Some("391011".to_owned()),
            None,
            None,
            None,
            None,
            None,
            Some(16_384),
            None,
            Some("empty".to_owned()),
            Some(true),
            Some(false),
        );
        let value: Value = serde_json::from_str(&raw).expect("session");
        assert_eq!(value["ok"], true, "{value}");
        value["session_id"].as_str().expect("id").to_owned()
    }
    let reader = shared();
    let controller = shared();
    let prepared = plan(&reader, "Shared original plan", false);
    let expected = original(&reader, prepared["plan_digest"].as_str().expect("digest"));
    let (_, seal) = plan_archive::encode(&expected, ArchiveLimits::default()).expect("archive");
    let session = resolve_session(Some(reader.clone())).expect("reader");
    for grant in &mut session.lock().expect("lock").grants {
        if matches!(grant.capability, Capability::Observe | Capability::Query) {
            grant.expires_at_tick = Some(GameTick(2));
        }
    }
    assert!(from_uri(&reader, &plan_uri(&reader, seal)).is_ok());
    let controller_session = resolve_session(Some(controller)).expect("controller");
    with_session(
        &controller_session,
        || panic!("controller unavailable"),
        |guard| {
            guard
                .adapter
                .advance_ticks(10)
                .expect("advance shared world")
        },
    );
    let denied = error_data(
        from_uri(&reader, &archive_uri(&reader, seal, 0)).expect_err("expired"),
        "capability_denied",
    );
    assert_eq!(denied["agent_turn"]["anchor"], Value::Null);
}

#[test]
fn full_source_and_publication_budgets_cannot_be_bypassed_with_a_small_page() {
    let id = open();
    let prepared = plan(&id, "Bounded original plan", false);
    let expected = original(&id, prepared["plan_digest"].as_str().expect("digest"));
    let (bytes, seal) = plan_archive::encode(&expected, ArchiveLimits::default()).expect("archive");
    let session = resolve_session(Some(id.clone())).expect("session");
    let budget = session.lock().expect("lock").budget;
    session.lock().expect("lock").budget.max_bytes = bytes.len() as u64 - 1;
    error_data(
        from_uri(&id, &archive_uri(&id, seal, bytes.len() - 1))
            .expect_err("complete source budget"),
        "budget_exceeded",
    );
    session.lock().expect("lock").budget = budget;
    session.lock().expect("lock").budget.max_entities = 1;
    error_data(
        view(&id, "plans").expect_err("record plus step budget"),
        "budget_exceeded",
    );
    session.lock().expect("lock").budget = budget;
    session.lock().expect("lock").budget.max_output_tokens = 1;
    error_data(
        from_uri(&id, &plan_uri(&id, seal)).expect_err("metadata"),
        "budget_exceeded",
    );
    session.lock().expect("lock").budget = budget;
    let before = fingerprint(&id);
    let mut checks = 0;
    let error = read_with_elapsed(&id, &format!("df://session/{id}/plans"), "plans", || {
        checks += 1;
        if checks > 3 {
            std::time::Duration::from_millis(budget.max_wall_millis)
        } else {
            std::time::Duration::ZERO
        }
    })
    .expect_err("deadline during checked archive encoding");
    error_data(error, "budget_exceeded");
    assert!(checks > 3);
    assert_eq!(fingerprint(&id), before);
}

#[test]
fn catalog_pages_are_bounded_and_reject_membership_or_world_changes() {
    let id = open();
    for index in 0..26 {
        let prepared = plan(&id, &format!("retained original {index}"), index % 2 != 0);
        commit(&id, &prepared);
    }
    let first = payload(view(&id, "plans").expect("catalog"));
    let returned = first["returned"].as_u64().expect("returned");
    assert!((1..=25).contains(&returned));
    assert_eq!(first["total"], 26);
    assert_eq!(first["complete_catalog"], false);
    let next = first["next"].as_str().expect("continuation").to_owned();
    let second = payload(from_uri(&id, &next).expect("next page"));
    assert_eq!(second["offset"], returned);
    assert_eq!(second["catalog_digest"], first["catalog_digest"]);
    assert_eq!(second["complete_catalog"], false);
    assert_eq!(second["coverage"]["absence_proof_scope"], json!([]));

    let prepared = plan(&id, "New pending changes membership", false);
    error_data(
        from_uri(&id, &next).expect_err("membership gap"),
        "cursor_gap",
    );
    let refreshed = payload(view(&id, "plans").expect("fresh catalog"));
    let pinned = refreshed["canonical_uri"]
        .as_str()
        .expect("pinned")
        .to_owned();
    commit(&id, &prepared);
    error_data(
        from_uri(&id, &pinned).expect_err("world and retention gap"),
        "cursor_gap",
    );
}
