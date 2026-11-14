//! Public shared-session coverage; helpers are the ordinary eleven-tool facade.
use super::*;

const MINE: &str =
    r#"[{"action":{"kind":"set_labor","units":["1001"],"labor":"MINE","enabled":true}}]"#;
const BREW: &str =
    r#"[{"action":{"kind":"set_labor","units":["1001"],"labor":"BREW","enabled":true}}]"#;
const BURROW: &str = r#"[{"action":{"kind":"set_burrow_membership","units":["1001"],"burrow":"3001","assigned":true}}]"#;

fn pair(selector: &str) -> std::result::Result<(String, String), Box<dyn std::error::Error>> {
    let a = open_shared(selector, Some("starter_fortress"), &ALL_EFFECTS)?;
    let b = open_shared(selector, None, &ALL_EFFECTS)?;
    Ok((
        a["session_id"].as_str().ok_or("a")?.to_owned(),
        b["session_id"].as_str().ok_or("b")?.to_owned(),
    ))
}

fn seal(session: &str, actions: &str) -> std::result::Result<String, Box<dyn std::error::Error>> {
    let planned = parsed(&fortress_plan(
        Some(session.to_owned()),
        None,
        None,
        Some(actions.to_owned()),
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    Ok(planned["plan_digest"].as_str().ok_or("digest")?.to_owned())
}

fn unit_fields(session: &str) -> std::result::Result<Value, Box<dyn std::error::Error>> {
    let queried = parsed(&fortress_query(
        Some(session.to_owned()),
        Some(r#"{"mode":"entities","kind":"unit"}"#.to_owned()),
    ))?;
    assert_eq!(queried["ok"], true, "{queried}");
    let unit = queried["rows"]
        .as_array()
        .ok_or("rows")?
        .iter()
        .find(|row| row["entity_id"] == "1001")
        .ok_or("unit")?;
    Ok(unit["fields"].clone())
}

#[test]
fn independent_same_unit_setters_commit_with_source_bound_field_certificate() -> TestResult {
    for (selector, actions, field) in [
        ("73811", MINE, "labor.MINE"),
        ("73812", BURROW, "burrow.3001"),
    ] {
        let (a, b) = pair(selector)?;
        let sealed = seal(&a, actions)?;
        let other = plan_and_commit(&b, BREW)?;
        assert_eq!(other["ok"], true, "{other}");
        let committed = parsed(&fortress_commit(Some(a.clone()), sealed.clone()))?;
        assert_eq!(committed["ok"], true, "{committed}");
        assert_eq!(committed["actions"][0]["state"], "Verified");
        let rebase = &committed["witness_rebase"];
        let certificate = &rebase["certificate"];
        assert_eq!(certificate["schema"], "dfmcp.field-witness-rebase/1");
        assert_eq!(certificate["from_digest"], sealed);
        assert_ne!(certificate["to_digest"], sealed);
        assert_ne!(
            certificate["base_state_hash"],
            certificate["current_state_hash"]
        );
        assert_eq!(
            certificate["coarse_conflict"],
            json!({"read":"entity","entity_id":"1001"})
        );
        assert_eq!(certificate["witness"]["fields"][0]["field"], field);
        assert_eq!(certificate["witness"]["fields"][0]["presence"], "missing");
        assert_eq!(
            rebase["certificate_digest"],
            dfmcp_core::Digest32::of_bytes(certificate.to_string().as_bytes()).to_hex()
        );
        let observed = unit_fields(&a)?;
        assert_eq!(observed[field], true);
        assert_eq!(observed["labor.BREW"], true);
        assert!(committed["agent_turn"]["anchor"].is_object());
        let retry = parsed(&fortress_commit(Some(a), sealed))?;
        assert_eq!(retry["ok"], true, "{retry}");
        assert_eq!(retry["actions"], committed["actions"]);
    }
    Ok(())
}

#[test]
fn field_refinement_does_not_recover_revoked_effect_authority() -> TestResult {
    let (a, b) = pair("73813")?;
    let sealed = seal(&a, MINE)?;
    assert_eq!(plan_and_commit(&b, BREW)?["ok"], true);
    let session = crate::server::resolve_session(Some(a.clone()))?;
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        guard
            .grants
            .retain(|grant| grant.capability != dfmcp_core::Capability::ConfigureLabor);
    }
    let refused = parsed(&fortress_commit(Some(a.clone()), sealed))?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "capability_denied");
    let observed = unit_fields(&a)?;
    assert!(observed["labor.MINE"].is_null());
    assert_eq!(observed["labor.BREW"], true);
    Ok(())
}

#[test]
fn incomplete_field_census_returns_a_reviewed_replay_without_dispatch() -> TestResult {
    let (a, b) = pair("73814")?;
    let sealed = seal(&a, MINE)?;
    assert_eq!(plan_and_commit(&b, BREW)?["ok"], true);
    let session = crate::server::resolve_session(Some(a.clone()))?;
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        guard.budget.max_entities = 1;
    }
    let refused = parsed(&fortress_commit(Some(a.clone()), sealed))?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "stale_anchor");
    assert_eq!(refused["witness_check"]["accepted"], false);
    assert!(
        refused["witness_check"]["refinement_refusal"]
            .as_str()
            .is_some_and(|reason| reason.contains("census"))
    );
    assert!(refused["rebased_plan"]["plan_digest"].is_string());
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        guard.budget.max_entities = 2_000;
    }
    let observed = unit_fields(&a)?;
    assert!(observed["labor.MINE"].is_null());
    assert_eq!(observed["labor.BREW"], true);
    Ok(())
}

fn alive_guard() -> Value {
    json!({
        "kind":"entity_quantity", "entity_kind":"unit", "filter_field":"alive",
        "filter_value":{"type":"bool","value":true}, "comparison":"eq", "value":7,
    })
}

fn guarded_mine(guard: Value) -> String {
    json!([{
        "action":{"kind":"set_labor","units":["1001"],"labor":"MINE","enabled":true},
        "preconditions":[guard],
    }])
    .to_string()
}

/// Simulate an independently observed, same-tick world version. Only fixture
/// construction uses injection; requests, sealed guards and commits use the facade.
fn inject_guard_world(
    session: &str,
    change: impl FnOnce(&mut dfmcp_world::WorldSnapshot),
) -> TestResult {
    let state = crate::server::resolve_session(Some(session.to_owned()))?;
    let mut guard = state.lock().map_err(|_| "session poisoned")?;
    let mut snapshot = guard.adapter.snapshot().clone();
    change(&mut snapshot);
    snapshot.cursor.sequence += 1;
    snapshot.refresh_hash();
    guard.adapter.inject_snapshot(snapshot)?;
    Ok(())
}

fn guard_relation(snapshot: &mut dfmcp_world::WorldSnapshot) {
    let id = dfmcp_core::EdgeId::new(30);
    snapshot.graph.edges.insert(
        id,
        dfmcp_world::EdgeRecord {
            id,
            revision: 1,
            kind: dfmcp_world::EdgeKind::AssignedTo,
            from: dfmcp_core::EntityId::new(1001),
            to: dfmcp_core::EntityId::new(3001),
            fields: Default::default(),
        },
    );
}

#[test]
fn caller_quantity_and_negative_relation_guards_survive_independent_shared_setters() -> TestResult {
    let (a, b) = pair("73820")?;
    let actions = guarded_mine(json!({"kind":"all","args":[
        alive_guard(),
        {"kind":"not","arg":{"kind":"edge_exists","edge_id":dfmcp_core::EdgeId::new(30).to_string()}},
    ]}));
    let planned = parsed(&fortress_plan(
        Some(a.clone()),
        None,
        None,
        Some(actions),
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    assert!(planned["steps"][0]["preconditions"].is_array());
    let sealed = planned["plan_digest"].as_str().ok_or("digest")?.to_owned();
    assert_eq!(plan_and_commit(&b, BREW)?["ok"], true);
    let committed = parsed(&fortress_commit(Some(a.clone()), sealed.clone()))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let witness = &committed["witness_rebase"]["certificate"]["witness"];
    assert_eq!(witness["aggregate_domains"][0]["total"], 7);
    assert_eq!(
        witness["aggregate_domains"][0]["members"]
            .as_array()
            .map(Vec::len),
        Some(7)
    );
    assert_eq!(witness["edges"][0]["presence"], "missing");
    let observed = unit_fields(&a)?;
    assert_eq!(observed["labor.MINE"], true);
    assert_eq!(observed["labor.BREW"], true);
    let retry = parsed(&fortress_commit(Some(a), sealed))?;
    assert_eq!(retry["actions"], committed["actions"]);
    Ok(())
}

#[test]
fn caller_guards_refuse_false_unknown_and_negated_unknown_evidence_before_effects() -> TestResult {
    let session = open("73821", true, &ALL_EFFECTS)?;
    let mut false_count = alive_guard();
    false_count["value"] = json!(8);
    let mut unknown_count = alive_guard();
    unknown_count["filter_field"] = json!("not_observed");
    for guard in [
        false_count,
        unknown_count.clone(),
        json!({"kind":"not","arg":unknown_count}),
        json!({"kind":"edge_exists","edge_id":dfmcp_core::EdgeId::new(30).to_string()}),
    ] {
        let planned = parsed(&fortress_plan(
            Some(session.clone()),
            None,
            None,
            Some(guarded_mine(guard)),
            None,
        ))?;
        assert_eq!(planned["ok"], false, "{planned}");
        assert_eq!(planned["error"]["code"], "preconditions_failed");
    }
    assert!(unit_fields(&session)?["labor.MINE"].is_null());
    Ok(())
}

#[test]
fn guarded_relation_rebase_rejects_endpoint_reuse_and_negative_read_phantoms() -> TestResult {
    let session = open("73822", true, &ALL_EFFECTS)?;
    inject_guard_world(&session, guard_relation)?;
    let positive = guarded_mine(json!({
        "kind":"edge_exists", "edge_id":dfmcp_core::EdgeId::new(30).to_string(),
        "edge_kind":"assigned_to",
    }));
    let sealed = seal(&session, &positive)?;
    inject_guard_world(&session, |snapshot| {
        if let Some(endpoint) = snapshot
            .graph
            .entities
            .get_mut(&dfmcp_core::EntityId::new(3001))
        {
            endpoint.generation += 1;
        }
    })?;
    let refused = parsed(&fortress_commit(Some(session.clone()), sealed))?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "stale_anchor");
    assert_eq!(refused["witness_check"]["change"]["read"], "edge_endpoint");
    assert!(refused["rebased_plan"]["plan_digest"].is_string());
    assert!(unit_fields(&session)?["labor.MINE"].is_null());

    let session = open("73823", true, &ALL_EFFECTS)?;
    let negative = guarded_mine(json!({
        "kind":"not", "arg":{"kind":"edge_exists","edge_id":dfmcp_core::EdgeId::new(30).to_string()},
    }));
    let sealed = seal(&session, &negative)?;
    inject_guard_world(&session, guard_relation)?;
    let refused = parsed(&fortress_commit(Some(session.clone()), sealed))?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "stale_anchor");
    assert_eq!(refused["rebase"]["refused"]["code"], "preconditions_failed");
    assert!(unit_fields(&session)?["labor.MINE"].is_null());
    Ok(())
}

fn quantity_inventory(snapshot: &mut dfmcp_world::WorldSnapshot) {
    use dfmcp_world::{EntityKind, EntityRecord, Fact, FactSource, Value as WorldValue};
    for (id, amount) in [(7001, 2), (7002, 3)] {
        let id = dfmcp_core::EntityId::new(id);
        let fact = |value| {
            Fact::known(
                value,
                snapshot.tick,
                FactSource::Derived("dfmcp.lab-scenario/1".into()),
                dfmcp_core::Digest32::ZERO,
            )
        };
        snapshot.graph.entities.insert(
            id,
            EntityRecord {
                id,
                generation: 1,
                revision: 1,
                kind: EntityKind::Item,
                label: "guarded inventory".into(),
                fields: std::collections::BTreeMap::from([
                    ("selected".into(), fact(WorldValue::Bool(true))),
                    ("amount".into(), fact(WorldValue::U64(amount))),
                ]),
            },
        );
    }
}

#[test]
fn guarded_quantity_rebase_requires_review_for_equal_totals_and_nonmatching_phantoms() -> TestResult
{
    use dfmcp_world::{EntityKind, Fact, FactSource, Value as WorldValue};
    for (selector, change) in [
        ("73824", "equal_total"),
        ("73825", "nonmatching_phantom"),
        ("73826", "canonical_alias_phantom"),
    ] {
        let session = open(selector, true, &ALL_EFFECTS)?;
        inject_guard_world(&session, quantity_inventory)?;
        let actions = guarded_mine(json!({
            "kind":"entity_quantity", "entity_kind":"item",
            "filter_field":"selected", "filter_value":{"type":"bool","value":true},
            "quantity_field":"amount", "comparison":"eq", "value":5,
        }));
        let sealed = seal(&session, &actions)?;
        inject_guard_world(&session, |snapshot| {
            let fact = |value| {
                Fact::known(
                    value,
                    snapshot.tick,
                    FactSource::Derived("dfmcp.lab-scenario/1".into()),
                    dfmcp_core::Digest32::ZERO,
                )
            };
            if change == "equal_total" {
                for (id, amount) in [(7001, 1), (7002, 4)] {
                    if let Some(record) = snapshot
                        .graph
                        .entities
                        .get_mut(&dfmcp_core::EntityId::new(id))
                    {
                        record.revision += 1;
                        record
                            .fields
                            .insert("amount".into(), fact(WorldValue::U64(amount)));
                    }
                }
            } else {
                let mut record = snapshot.graph.entities[&dfmcp_core::EntityId::new(7001)].clone();
                record.id = dfmcp_core::EntityId::new(7003);
                record
                    .fields
                    .insert("selected".into(), fact(WorldValue::Bool(false)));
                if change == "canonical_alias_phantom" {
                    record.kind = EntityKind::Other("item".into());
                }
                snapshot.graph.entities.insert(record.id, record);
            }
        })?;
        let refused = parsed(&fortress_commit(Some(session.clone()), sealed))?;
        assert_eq!(refused["ok"], false, "{change}: {refused}");
        assert_eq!(refused["error"]["code"], "stale_anchor");
        assert_eq!(refused["witness_check"]["accepted"], false);
        assert!(refused["rebased_plan"]["plan_digest"].is_string());
        assert!(unit_fields(&session)?["labor.MINE"].is_null());
    }
    Ok(())
}

#[test]
fn guarded_field_refinement_preserves_current_effect_authority() -> TestResult {
    let (a, b) = pair("73827")?;
    let sealed = seal(&a, &guarded_mine(alive_guard()))?;
    assert_eq!(plan_and_commit(&b, BREW)?["ok"], true);
    let state = crate::server::resolve_session(Some(a.clone()))?;
    state
        .lock()
        .map_err(|_| "session poisoned")?
        .grants
        .retain(|grant| grant.capability != dfmcp_core::Capability::ConfigureLabor);
    let refused = parsed(&fortress_commit(Some(a.clone()), sealed))?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "capability_denied");
    assert!(unit_fields(&a)?["labor.MINE"].is_null());
    Ok(())
}

#[test]
fn caller_positive_relation_guard_rebases_after_unrelated_endpoint_field_change() -> TestResult {
    let session = open("73828", true, &ALL_EFFECTS)?;
    inject_guard_world(&session, guard_relation)?;
    let sealed = seal(
        &session,
        &guarded_mine(json!({
            "kind":"edge_exists", "edge_id":dfmcp_core::EdgeId::new(30).to_string(),
            "edge_kind":"assigned_to",
        })),
    )?;
    inject_guard_world(&session, |snapshot| {
        let value = dfmcp_world::Fact::known(
            dfmcp_world::Value::Bool(true),
            snapshot.tick,
            dfmcp_world::FactSource::Derived("dfmcp.reference-effects/1".into()),
            dfmcp_core::Digest32::ZERO,
        );
        if let Some(unit) = snapshot
            .graph
            .entities
            .get_mut(&dfmcp_core::EntityId::new(1001))
        {
            unit.revision += 1;
            unit.fields.insert("labor.BREW".into(), value);
        }
    })?;
    let committed = parsed(&fortress_commit(Some(session.clone()), sealed))?;
    assert_eq!(committed["ok"], true, "{committed}");
    assert_eq!(
        committed["witness_rebase"]["certificate"]["witness"]["edges"][0]["presence"],
        "present"
    );
    assert_eq!(unit_fields(&session)?["labor.MINE"], true);
    assert_eq!(unit_fields(&session)?["labor.BREW"], true);
    Ok(())
}

#[test]
fn empty_quantity_domain_cannot_hide_a_new_nonmatching_member() -> TestResult {
    let session = open("73829", true, &ALL_EFFECTS)?;
    let actions = guarded_mine(json!({
        "kind":"entity_quantity","entity_kind":"item","filter_field":"selected",
        "filter_value":{"type":"bool","value":true},"comparison":"eq","value":0,
    }));
    let sealed = seal(&session, &actions)?;
    inject_guard_world(&session, |snapshot| {
        quantity_inventory(snapshot);
        let fact = dfmcp_world::Fact::known(
            dfmcp_world::Value::Bool(false),
            snapshot.tick,
            dfmcp_world::FactSource::Derived("dfmcp.lab-scenario/1".into()),
            dfmcp_core::Digest32::ZERO,
        );
        for record in snapshot.graph.entities.values_mut() {
            if record.kind == dfmcp_world::EntityKind::Item {
                record.fields.insert("selected".into(), fact.clone());
            }
        }
    })?;
    let refused = parsed(&fortress_commit(Some(session.clone()), sealed))?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "stale_anchor");
    assert!(refused["rebased_plan"]["plan_digest"].is_string());
    assert!(unit_fields(&session)?["labor.MINE"].is_null());
    Ok(())
}

#[test]
fn a_true_relation_branch_does_not_erase_unknown_quantity_reads_from_rebase() -> TestResult {
    let session = open("73830", true, &ALL_EFFECTS)?;
    inject_guard_world(&session, guard_relation)?;
    let mut unknown = alive_guard();
    unknown["filter_field"] = json!("not_observed");
    let sealed = seal(
        &session,
        &guarded_mine(json!({"kind":"any","args":[
            {"kind":"edge_exists","edge_id":dfmcp_core::EdgeId::new(30).to_string()},
            unknown,
        ]})),
    )?;
    inject_guard_world(&session, |snapshot| {
        if let Some(unit) = snapshot
            .graph
            .entities
            .get_mut(&dfmcp_core::EntityId::new(1001))
        {
            unit.revision += 1;
            unit.label.push_str(" independently observed");
        }
    })?;
    let refused = parsed(&fortress_commit(Some(session.clone()), sealed))?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "stale_anchor");
    assert_eq!(refused["witness_check"]["accepted"], false);
    assert!(refused["rebased_plan"]["plan_digest"].is_string());
    assert!(unit_fields(&session)?["labor.MINE"].is_null());
    Ok(())
}
