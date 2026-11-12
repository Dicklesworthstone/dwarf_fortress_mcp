//! One coherent selected watch set through the real laboratory wait owner.
//! Owning implementation bead: df-lab-structured-query-f7q.3.
//! These scenarios distinguish sampled outcomes, accepted publications and time.

use super::*;
use crate::agent_facade as facade;
use serde_json::Value;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Owner {
    id: String,
}

impl Drop for Owner {
    fn drop(&mut self) {
        if let Ok(id) = u128::from_str_radix(&self.id, 16) {
            // LabSession's query-resource guard releases every retained watch
            // and baseline when the final session owner is dropped.
            sessions().remove(&SessionId::new(id));
        }
    }
}

fn parsed(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn open(scenario: &str, paused: bool) -> TestResult<Owner> {
    let opened = parsed(&facade::fortress_open_session(
        Some(paused),
        None,
        Some(
            [
                ("observe", "read_only"),
                ("query", "read_only"),
                ("plan", "reversible"),
                ("configure_production", "reversible"),
                ("control_clock", "reversible"),
            ]
            .into_iter()
            .map(|(capability, risk)| (capability.to_owned(), risk.to_owned()))
            .collect(),
        ),
        Some(60_000),
        Some(2_000),
        Some(8_192),
        Some(4 * 1024 * 1024),
        Some(16_384),
        Some(64),
        Some(scenario.to_owned()),
        Some(false),
        Some(false),
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    Ok(Owner {
        id: opened["session_id"].as_str().ok_or("session ID")?.to_owned(),
    })
}

fn with_owner<T>(
    owner: &Owner,
    body: impl FnOnce(&mut LabSession) -> TestResult<T>,
) -> TestResult<T> {
    let session = resolve_session(Some(owner.id.clone()))?;
    with_session(&session, || Err("session lock unavailable".into()), body)
}

fn read_owner<T>(
    owner: &Owner,
    body: impl FnOnce(&LabSession) -> TestResult<T>,
) -> TestResult<T> {
    let session = resolve_session(Some(owner.id.clone()))?;
    with_session_read(&session, |error| Err(error.into()), body)
}

fn snapshot(owner: &Owner) -> TestResult<WorldSnapshot> {
    read_owner(owner, |session| Ok(session.adapter.snapshot().clone()))
}

fn retained(owner: &Owner, handles: &[String]) -> TestResult<Vec<Value>> {
    read_owner(owner, |session| {
        let context = read_context(session);
        handles
            .iter()
            .map(|handle| Ok(semantic_query::laboratory_watch_record(&context, handle)?))
            .collect()
    })
}

fn mutate(owner: &Owner, edit: impl FnOnce(&mut WorldSnapshot) -> TestResult) -> TestResult {
    with_owner(owner, |session| {
        let mut source = session.adapter.snapshot().clone();
        edit(&mut source)?;
        source.cursor = source.cursor.checked_next().ok_or("cursor overflow")?;
        source.refresh_hash();
        session.adapter.inject_snapshot(source)?;
        Ok(())
    })
}

fn query(owner: &Owner, query: Value) -> TestResult<Value> {
    parsed(&facade::fortress_query(
        Some(owner.id.clone()),
        Some(
            json!({
                "mode":"temporal",
                "request":{"schema":"dfmcp.query/1", "query":query}
            })
            .to_string(),
        ),
    ))
}

fn register(
    owner: &Owner,
    key: &str,
    condition: Value,
    failure: Option<Value>,
    cadence: u64,
    stable: u32,
    deadline: u64,
) -> TestResult<Value> {
    let mut request = json!({
        "kind":"watch", "key":key, "condition":condition,
        "deadline_tick":deadline, "poll_interval_ticks":cadence,
        "stable_observations":stable
    });
    if let Some(failure) = failure {
        request["failure_condition"] = failure;
    }
    let result = query(owner, request)?;
    assert_eq!(result["ok"], true, "{result}");
    Ok(result["record"].clone())
}

fn handle(record: &Value) -> TestResult<String> {
    Ok(record["watch"].as_str().ok_or("watch handle")?.to_owned())
}

fn record<'a>(selected: &'a Value, handle: &str) -> TestResult<&'a Value> {
    selected["records"]
        .as_array()
        .and_then(|records| records.iter().find(|record| record["watch"] == handle))
        .ok_or_else(|| "selected detailed watch record".into())
}

fn wait(owner: &Owner, handles: &[String], policy: Option<&str>, ticks: u64) -> TestResult<Value> {
    parsed(&facade::fortress_wait_for_watches(
        Some(owner.id.clone()),
        Some(ticks),
        handles.to_vec(),
        policy.map(str::to_owned),
    ))
}

fn successful_wait(
    owner: &Owner,
    handles: &[String],
    policy: Option<&str>,
    ticks: u64,
) -> TestResult<Value> {
    let result = wait(owner, handles, policy, ticks)?;
    assert_eq!(result["ok"], true, "{result}");
    let selected = &result["selected_watch_set_wait"];
    assert_eq!(selected["schema"], "dfmcp.selected-watch-set-wait/1");
    assert_eq!(selected["watch_policy"], policy.unwrap_or("all_satisfied"));
    let mut sorted = handles.to_vec();
    sorted.sort();
    assert_eq!(selected["watch_ids"], json!(sorted));
    assert_eq!(selected["selected"].as_u64(), Some(handles.len() as u64));
    assert_eq!(selected["records"].as_array().ok_or("full selected records")?.len(), handles.len());
    assert_eq!(
        json!(selected["records"].as_array().ok_or("ordered selected records")?
            .iter().map(|record| record["watch"].clone()).collect::<Vec<_>>()),
        selected["watch_ids"],
    );
    for handle in handles {
        let row = record(selected, handle)?;
        assert!(row["definition"].is_object(), "{row}");
        assert!(row["created_at"].is_object(), "{row}");
        assert!(row["evaluation"].is_object(), "{row}");
    }
    assert_eq!(selected["original_definitions_unchanged"], true);
    assert_eq!(selected["evaluation_complete"], true);
    assert_eq!(selected["game_effect_success_proven"], false);
    assert_eq!(result["agent_turn"]["anchor"], result["observed_anchor"]);
    assert!(result.get("selected_watch_wait").is_none());
    Ok(result)
}

fn refusal(result: &Value, code: &str) {
    assert_eq!(result["ok"], false, "{result}");
    assert_eq!(result["error"]["code"], code, "{result}");
}

fn tick(value: u64) -> Value {
    json!({"op":"tick_at_least", "value":value})
}

fn stock(minimum: u64) -> Value {
    json!({
        "op":"field",
        "entity_id":crate::lab_world::starter::STOCK_LEDGER.to_string(),
        "generation":1,
        "field":dfmcp_intent::effects::STOCK_DRINK_FIELD,
        "comparison":"ge", "value":{"type":"u64", "value":minimum}
    })
}

fn interleaved(owner: &Owner) -> TestResult<(u64, Value, Value, Vec<String>)> {
    let start = snapshot(owner)?.tick.0;
    let slow = register(owner, "cadence-seven", tick(start + 10), None, 7, 3, start + 100)?;
    let fast = register(owner, "cadence-five", tick(start + 8), None, 5, 2, start + 100)?;
    let handles = vec![handle(&fast)?, handle(&slow)?];
    Ok((start, slow, fast, handles))
}

#[test]
fn interleaved_cadences_preserve_each_members_stability_and_publish_full_records() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("empty", false)?;
    let (start, slow, fast, handles) = interleaved(&owner)?;
    let waited = successful_wait(&owner, &handles, None, 500)?;
    let selected = &waited["selected_watch_set_wait"];
    assert_eq!(selected["stop_reason"], "all_satisfied", "{waited}");
    assert_eq!(selected["group_satisfied"], true);
    assert_eq!(selected["all_satisfied"], true);
    assert_eq!(selected["all_terminal"], true);
    assert_eq!(selected["satisfied"], 2);
    assert_eq!(selected["samples_added"], 7);
    assert_eq!(waited["advanced_game_ticks"], 28);
    assert_eq!(waited["wait_progress"]["remaining_game_ticks"], 472);
    assert_eq!(snapshot(&owner)?.tick.0, start + 28);
    let slow_now = record(selected, &handle(&slow)?)?;
    let fast_now = record(selected, &handle(&fast)?)?;
    assert_eq!(slow_now["definition"], slow["definition"]);
    assert_eq!(fast_now["definition"], fast["definition"]);
    assert_eq!(slow_now["sample_count"], 5);
    assert_eq!(fast_now["sample_count"], 4);
    assert_eq!(slow_now["stable_observations"], 3);
    assert_eq!(fast_now["stable_observations"], 2);
    assert_eq!(slow_now["evaluation"]["skipped_observations_reset_streak"], false);
    assert_eq!(fast_now["evaluation"]["skipped_observations_reset_streak"], false);
    assert_eq!(slow_now["last_evaluated_anchor"]["game_tick"], start + 28);
    assert_eq!(fast_now["last_evaluated_anchor"]["game_tick"], start + 15);
    assert_eq!(slow_now["evaluation_current"], true);
    assert_eq!(fast_now["evaluation_current"], false);
    for retained in retained(&owner, &handles)? {
        assert_eq!(&retained, record(selected, &handle(&retained)?)?);
    }
    Ok(())
}

#[test]
fn any_satisfied_stops_early_and_all_satisfied_can_continue_the_same_original_set() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("empty", false)?;
    let (start, slow, fast, handles) = interleaved(&owner)?;
    let first = successful_wait(&owner, &handles, Some("any_satisfied"), 500)?;
    let selected = &first["selected_watch_set_wait"];
    assert_eq!(selected["stop_reason"], "any_satisfied");
    assert_eq!(selected["group_satisfied"], true);
    assert_eq!(selected["all_satisfied"], false);
    assert_eq!(selected["all_terminal"], false);
    assert_eq!(selected["satisfied"], 1);
    assert_eq!(first["advanced_game_ticks"], 15);
    assert_eq!(record(selected, &handle(&slow)?)?["stable_observations"], 1);
    assert_eq!(record(selected, &handle(&slow)?)?["sample_count"], 3);
    assert_eq!(selected["triggered_watch_ids"], json!([handle(&fast)?]));
    let fast_before = record(selected, &handle(&fast)?)?.clone();
    let bundle = replay_bundle_for(&owner.id)?;
    let call = bundle["calls"].as_array()
        .and_then(|calls| calls.last()).ok_or("last group replay call")?;
    assert_eq!(call["tool"], "fortress.wait");
    assert_eq!(call["arguments"]["watch_ids"], json!(handles));
    assert_eq!(call["arguments"]["watch_policy"], "any_satisfied");
    assert_eq!(call["arguments"]["max_game_ticks"], 500);
    assert_eq!(bundle["replayable"], false, "{bundle}");
    assert!(bundle["not_replayable_reason"].as_str()
        .is_some_and(|reason| reason.contains("watch")), "{bundle}");
    let before_replay = snapshot(&owner)?.anchor();
    let refused = crate::replay::replay_bundle(&bundle);
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["replayed"], 0);
    assert_eq!(snapshot(&owner)?.anchor(), before_replay);

    let continued = successful_wait(&owner, &handles, Some("all_satisfied"), 500)?;
    let completed = &continued["selected_watch_set_wait"];
    assert_eq!(completed["stop_reason"], "all_satisfied");
    assert_eq!(continued["advanced_game_ticks"], 13);
    assert_eq!(snapshot(&owner)?.tick.0, start + 28);
    assert_eq!(record(completed, &handle(&slow)?)?["stable_observations"], 3);
    let fast_after = record(completed, &handle(&fast)?)?;
    for field in ["definition", "created_at", "sample_count", "evaluation", "evidence_digest",
        "last_evaluated_anchor"] {
        assert_eq!(fast_after[field], fast_before[field], "{field}");
    }
    assert_eq!(fast_after["evaluation_current"], false);
    let replayed = successful_wait(&owner, &handles, None, 500)?;
    assert_eq!(replayed["advanced_game_ticks"], 0);
    assert_eq!(replayed["selected_watch_set_wait"]["samples_added"], 0);
    assert_eq!(replayed["selected_watch_set_wait"]["records"], completed["records"]);
    Ok(())
}

#[test]
fn terminal_members_keep_historical_evidence_when_their_current_predicate_reverses() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("empty", true)?;
    let start = snapshot(&owner)?.tick.0;
    let was_paused = register(&owner, "historical-pause", json!({"op":"paused", "value":true}),
        None, 7, 1, start + 100)?;
    assert_eq!(was_paused["status"], "satisfied");
    let later = register(&owner, "later-clock", tick(start + 7), None, 7, 1, start + 100)?;
    let handles = vec![handle(&later)?, handle(&was_paused)?];
    mutate(&owner, |source| {
        source.paused = false;
        Ok(())
    })?;
    let waited = successful_wait(&owner, &handles, None, 50)?;
    let selected = &waited["selected_watch_set_wait"];
    assert_eq!(selected["stop_reason"], "all_satisfied", "{waited}");
    assert_eq!(selected["group_satisfied"], true);
    assert_eq!(waited["paused"], false);
    assert_eq!(waited["advanced_game_ticks"], 7);
    let historical = record(selected, &handle(&was_paused)?)?;
    for field in ["status", "definition", "created_at", "sample_count", "evaluation",
        "evidence_digest", "last_evaluated_anchor"] {
        assert_eq!(historical[field], was_paused[field], "{field}");
    }
    assert_eq!(historical["evaluation"]["condition"], "true");
    assert_eq!(historical["evaluation_current"], false);
    assert_eq!(historical["needs_poll"], false);
    assert_eq!(historical["last_evaluated_anchor"]["game_tick"], start);
    assert_eq!(record(selected, &handle(&later)?)?["evaluation_current"], true);
    Ok(())
}

#[test]
fn known_failure_and_unknown_guards_precede_any_members_satisfaction() -> TestResult {
    let _serial = crate::test_serial();
    for unknown in [false, true] {
        let owner = open("empty", false)?;
        let start = snapshot(&owner)?.tick.0;
        let good = register(&owner, "already-true", tick(start), None, 3, 1, start + 100)?;
        let guarded = register(&owner, "guarded", tick(start),
            Some(if unknown { stock(1) } else { tick(start) }), 3, 1, start + 100)?;
        let handles = vec![handle(&good)?, handle(&guarded)?];
        let before = snapshot(&owner)?.anchor();
        let waited = successful_wait(&owner, &handles, Some("any_satisfied"), 50)?;
        let selected = &waited["selected_watch_set_wait"];
        assert_eq!(selected["stop_reason"], if unknown { "blocked_unknown" } else { "failed" });
        assert_eq!(selected["group_satisfied"], false);
        assert_eq!(selected["satisfied"], 1);
        assert_eq!(selected["triggered_watch_ids"], json!([handle(&guarded)?]));
        assert_eq!(waited["advanced_game_ticks"], 0);
        assert_eq!(snapshot(&owner)?.anchor(), before);
        assert_eq!(record(selected, &handle(&guarded)?)?["stable_observations"], 0);
    }

    // Evaluate both members at the same due frontier: success in one member
    // cannot hide the other's newly observed failure.
    let owner = open("empty", false)?;
    let start = snapshot(&owner)?.tick.0;
    let good = register(&owner, "future-good", tick(start + 6), None, 3, 1, start + 100)?;
    let bad = register(&owner, "future-failure", tick(start + 6), Some(tick(start + 6)),
        3, 1, start + 100)?;
    let waited = successful_wait(&owner, &[handle(&good)?, handle(&bad)?],
        Some("any_satisfied"), 50)?;
    assert_eq!(waited["advanced_game_ticks"], 6);
    assert_eq!(waited["selected_watch_set_wait"]["stop_reason"], "failed");
    assert_eq!(waited["selected_watch_set_wait"]["group_satisfied"], false);
    assert_eq!(waited["selected_watch_set_wait"]["satisfied"], 1);
    let unknown = register(&owner, "also-unknown", stock(1), None, 3, 1, start + 100)?;
    let combined = successful_wait(&owner,
        &[handle(&good)?, handle(&unknown)?, handle(&bad)?], Some("any_satisfied"), 0)?;
    assert_eq!(combined["selected_watch_set_wait"]["stop_reason"], "failed");
    assert_eq!(combined["selected_watch_set_wait"]["triggered_watch_ids"], json!([handle(&bad)?]));
    assert_eq!(combined["advanced_game_ticks"], 0);
    Ok(())
}

#[test]
fn cancellation_and_identity_invalidation_cannot_be_hidden_by_an_already_satisfied_member() -> TestResult {
    let _serial = crate::test_serial();
    for cancelled in [false, true] {
        let owner = open("starter_fortress", false)?;
        let now = snapshot(&owner)?.tick.0;
        let satisfied = register(&owner, "already-satisfied", tick(now), None, 7, 1, now + 100)?;
        let guarded = register(&owner, "identity-bound", stock(1), None, 7, 2, now + 100)?;
        let guarded_id = handle(&guarded)?;
        assert_eq!(guarded["stable_observations"], 1);
        if cancelled {
            let result = query(&owner, json!({"kind":"cancel_watch", "watch":guarded_id}))?;
            assert_eq!(result["ok"], true, "{result}");
            assert_eq!(result["record"]["status"], "cancelled");
        } else {
            mutate(&owner, |source| {
                let ledger = source.graph.entities
                    .get_mut(&crate::lab_world::starter::STOCK_LEDGER)
                    .ok_or("stock ledger")?;
                ledger.generation = ledger.generation.checked_add(1).ok_or("generation overflow")?;
                Ok(())
            })?;
        }
        let before = snapshot(&owner)?.anchor();
        let waited = successful_wait(&owner, &[handle(&satisfied)?, guarded_id.clone()],
            Some("any_satisfied"), 50)?;
        let selected = &waited["selected_watch_set_wait"];
        assert_eq!(selected["stop_reason"], if cancelled { "cancelled" } else { "invalidated" });
        assert_eq!(selected["group_satisfied"], false);
        assert_eq!(selected["satisfied"], 1);
        assert_eq!(selected["triggered_watch_ids"], json!([guarded_id]));
        assert_eq!(waited["advanced_game_ticks"], 0);
        assert_eq!(snapshot(&owner)?.anchor(), before);
        assert_eq!(record(selected, &handle(&satisfied)?)?["evidence_digest"], satisfied["evidence_digest"]);
    }
    Ok(())
}

#[test]
fn the_earliest_original_deadline_clips_the_group_and_exact_deadline_truth_can_finish() -> TestResult {
    let _serial = crate::test_serial();
    for deadline in [19, 21] {
        let owner = open("empty", false)?;
        let start = snapshot(&owner)?.tick.0;
        let first = register(&owner, "due-seven", tick(start + 21), None, 7, 1, start + 100)?;
        let second = register(&owner, "deadline-three", tick(start + 21), None, 3, 1,
            start + deadline)?;
        let handles = vec![handle(&first)?, handle(&second)?];
        let waited = successful_wait(&owner, &handles, None, 100)?;
        let selected = &waited["selected_watch_set_wait"];
        assert_eq!(waited["advanced_game_ticks"], deadline, "{waited}");
        assert_eq!(snapshot(&owner)?.tick.0, start + deadline);
        if deadline == 21 {
            assert_eq!(selected["stop_reason"], "all_satisfied");
            assert_eq!(selected["group_satisfied"], true);
        } else {
            assert_eq!(selected["stop_reason"], "expired");
            assert_eq!(selected["group_satisfied"], false);
            assert_eq!(selected["triggered_watch_ids"], json!([handle(&second)?]));
            let expired = record(selected, &handle(&second)?)?.clone();
            let later = parsed(&facade::fortress_wait(Some(owner.id.clone()), Some(10)))?;
            assert_eq!(later["ok"], true, "{later}");
            let replayed = successful_wait(&owner, &handles, None, 100)?;
            assert_eq!(replayed["selected_watch_set_wait"]["stop_reason"], "expired");
            assert_eq!(replayed["advanced_game_ticks"], 0);
            let historical = record(&replayed["selected_watch_set_wait"], &handle(&second)?)?;
            for field in ["status", "evidence_digest", "last_evaluated_anchor", "sample_count"] {
                assert_eq!(historical[field], expired[field], "{field}");
            }
            assert_eq!(historical["evaluation_current"], false);
        }
    }
    Ok(())
}

#[test]
fn selection_shape_ownership_and_selector_conflicts_refuse_before_any_effect() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("empty", false)?;
    let foreign = open("empty", false)?;
    let start = snapshot(&owner)?.tick.0;
    let first = register(&owner, "first-owned", tick(start + 7), None, 7, 2, start + 100)?;
    let second = register(&owner, "second-owned", tick(start + 5), None, 5, 2, start + 100)?;
    let handles = vec![handle(&first)?, handle(&second)?];
    let foreign_watch = register(&foreign, "other-owner", tick(start + 5), None, 5, 1,
        start + 100)?;
    // Both owned members have a real, due positive sample waiting. A faulty
    // sequential implementation must not publish that valid prefix before
    // discovering an invalid or foreign member later in the requested set.
    mutate(&owner, |source| {
        source.tick = GameTick(start + 7);
        Ok(())
    })?;
    let before = snapshot(&owner)?.anchor();
    let originals = retained(&owner, &handles)?;
    let invalid_sets = vec![
        Vec::new(),
        vec![handles[0].clone(), handles[0].clone()],
        vec![handles[0].clone(), "malformed".to_owned()],
        vec![handles[0].clone(), format!("watch:{}", "ab".repeat(32))],
        vec![handles[0].clone(), handle(&foreign_watch)?],
        (0..9).map(|index| format!("watch:{index:064x}")).collect(),
    ];
    for invalid in invalid_sets {
        let refused = wait(&owner, &invalid, None, 50)?;
        refusal(&refused, "invalid_request");
        assert_eq!(snapshot(&owner)?.anchor(), before);
        assert_eq!(retained(&owner, &handles)?, originals);
    }
    let invalid_policy = wait(&owner, &handles, Some("first"), 50)?;
    refusal(&invalid_policy, "invalid_request");
    for (plan, single, set, policy) in [
        (Some("ab".repeat(32)), None, Some(handles.clone()), None),
        (None, Some(handles[0].clone()), Some(handles.clone()), None),
        (None, None, None, Some("any_satisfied".to_owned())),
        (None, Some(handles[0].clone()), None, Some("all_satisfied".to_owned())),
    ] {
        let refused = parsed(&facade::fortress_wait_for_targets(
            Some(owner.id.clone()), Some(50), plan, single, set, policy,
        ))?;
        refusal(&refused, "invalid_request");
        assert_eq!(snapshot(&owner)?.anchor(), before);
        assert_eq!(retained(&owner, &handles)?, originals);
    }
    Ok(())
}

#[test]
fn singleton_and_eight_member_sets_are_bounded_and_existing_single_wait_stays_compatible() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("empty", false)?;
    let now = snapshot(&owner)?.tick.0;
    let mut handles = Vec::new();
    for index in 0..8 {
        handles.push(handle(&register(&owner, &format!("member-{index}"), tick(now),
            None, 1, 1, now + 100)?)?);
    }
    let all = successful_wait(&owner, &handles, None, 0)?;
    assert_eq!(all["selected_watch_set_wait"]["selected"], 8);
    assert_eq!(all["selected_watch_set_wait"]["stop_reason"], "all_satisfied");
    assert_eq!(all["advanced_game_ticks"], 0);
    let one = successful_wait(&owner, &handles[..1], None, 0)?;
    assert_eq!(one["selected_watch_set_wait"]["selected"], 1);
    let scalar = parsed(&facade::fortress_wait_for_watch(
        Some(owner.id.clone()), Some(0), handles[0].clone(),
    ))?;
    assert_eq!(scalar["ok"], true, "{scalar}");
    assert_eq!(scalar["selected_watch_wait"]["watch_id"], handles[0]);
    assert_eq!(scalar["selected_watch_wait"]["stop_reason"], "satisfied");
    assert!(scalar.get("selected_watch_set_wait").is_none());
    assert_eq!(scalar["advanced_game_ticks"], 0);
    Ok(())
}

#[test]
fn zero_tick_and_paused_waits_do_not_manufacture_group_stability() -> TestResult {
    let _serial = crate::test_serial();
    for paused in [false, true] {
        let owner = open("empty", paused)?;
        let now = snapshot(&owner)?.tick.0;
        let first = register(&owner, "zero-first", tick(now), None, 3, 2, now + 100)?;
        let second = register(&owner, "zero-second", tick(now + 5), None, 5, 2, now + 100)?;
        let handles = vec![handle(&first)?, handle(&second)?];
        let before = snapshot(&owner)?.anchor();
        if !paused {
            with_owner(&owner, |session| {
                session.grants.retain(|grant| grant.capability != Capability::ControlClock);
                Ok(())
            })?;
        }
        let waited = successful_wait(&owner, &handles, None, if paused { 50 } else { 0 })?;
        let selected = &waited["selected_watch_set_wait"];
        assert_eq!(selected["stop_reason"], if paused { "paused" } else { "tick_limit" });
        assert_eq!(selected["group_satisfied"], false);
        assert_eq!(selected["samples_added"], 0);
        assert_eq!(waited["advanced_game_ticks"], 0);
        assert_eq!(snapshot(&owner)?.anchor(), before);
        assert_eq!(record(selected, &handles[0])?["sample_count"], first["sample_count"]);
        assert_eq!(record(selected, &handles[1])?["sample_count"], second["sample_count"]);
        assert_eq!(record(selected, &handles[0])?["stable_observations"], 1);
    }
    Ok(())
}

#[test]
fn current_authority_horizon_and_complete_output_admission_are_atomic_for_all_members() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("starter_fortress", false)?;
    let start = snapshot(&owner)?.tick.0;
    let first = register(&owner, "authority-first", tick(start + 7), None, 7, 1, start + 100)?;
    let second = register(&owner, "authority-second", stock(50), None, 5, 2, start + 100)?;
    let handles = vec![handle(&first)?, handle(&second)?];
    let originals = retained(&owner, &handles)?;
    let before = snapshot(&owner)?.anchor();
    let (grants, budget) = read_owner(&owner, |session| Ok((session.grants.clone(), session.budget)))?;
    for defect in 0..8 {
        with_owner(&owner, |session| {
            session.grants = grants.clone();
            session.budget = budget;
            match defect {
                0 => session.grants.retain(|grant| grant.capability != Capability::Query),
                1 => session.grants.retain(|grant| grant.capability != Capability::Observe),
                2 => session.grants.retain(|grant| grant.capability != Capability::ControlClock),
                3 => {
                    for grant in &mut session.grants {
                        if grant.capability == Capability::Query {
                            grant.scope.entity_ids.insert(EntityId::new(1001));
                        }
                    }
                }
                4 => {
                    for grant in &mut session.grants {
                        if grant.capability == Capability::Query {
                            grant.expires_at_tick = Some(GameTick(start + 8));
                        }
                    }
                }
                5 => session.budget.max_entities = 1,
                6 => session.budget.max_bytes = 1_024,
                _ => session.budget.max_output_tokens = 128,
            }
            Ok(())
        })?;
        let refused = wait(&owner, &handles, None, 50)?;
        refusal(&refused, if defect < 5 { "capability_denied" } else { "budget_exceeded" });
        if defect == 6 {
            assert!(refused.to_string().len() <= 1_024, "{refused}");
        }
        if defect == 7 {
            assert!(refused.to_string().len() <= 128 * 4, "{refused}");
            assert_eq!(refused["output_budget"]["max_output_tokens"], 128);
        }
        assert_eq!(snapshot(&owner)?.anchor(), before, "{refused}");
        with_owner(&owner, |session| {
            session.grants = grants.clone();
            session.budget = budget;
            Ok(())
        })?;
        assert_eq!(retained(&owner, &handles)?, originals);
    }
    Ok(())
}

#[test]
fn selected_set_progresses_production_and_preserves_unselected_work_and_baselines() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("starter_fortress", false)?;
    let planned = parsed(&facade::fortress_plan(
        Some(owner.id.clone()), Some("brew for a group of observed conditions".to_owned()), None,
        Some(json!([{"action":{"kind":"create_work_order", "name":"group-brew",
            "job_token":"BREW_DRINK", "amount":20}}]).to_string()), None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let committed = parsed(&facade::fortress_commit(
        Some(owner.id.clone()),
        planned["plan_digest"].as_str().ok_or("production digest")?.to_owned(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let action = read_owner(&owner, |session| Ok(session.last_action))?;
    let now = snapshot(&owner)?.tick.0;
    let captured = query(&owner, json!({
        "kind":"capture", "key":"retained-roster", "max_game_ticks":2_000,
        "select":{"kind":"entities", "kinds":["unit"], "fields":["alive"]}
    }))?;
    assert_eq!(captured["ok"], true, "{captured}");
    let baseline = captured["captured"]["baseline"].clone();
    let drink = register(&owner, "drink-reserve", stock(50), None, 10, 2, now + 1_000)?;
    let clock = register(&owner, "clock-minimum", tick(now + 15), None, 7, 2, now + 1_000)?;
    let unrelated = register(&owner, "unselected-condition", tick(now + 1_500), None,
        5, 1, now + 1_900)?;
    let unrelated_id = handle(&unrelated)?;
    let unrelated_before = retained(&owner, &[unrelated_id.clone()])?;
    let pending = parsed(&facade::fortress_plan(
        Some(owner.id.clone()), Some("independent retained review".to_owned()),
        Some(true), None, None,
    ))?;
    assert_eq!(pending["ok"], true, "{pending}");
    let waited = successful_wait(&owner, &[handle(&clock)?, handle(&drink)?], None, 1_000)?;
    let selected = &waited["selected_watch_set_wait"];
    assert_eq!(selected["stop_reason"], "all_satisfied", "{waited}");
    assert_eq!(selected["group_satisfied"], true);
    assert_eq!(record(selected, &handle(&drink)?)?["definition"], drink["definition"]);
    assert_eq!(record(selected, &handle(&clock)?)?["definition"], clock["definition"]);
    let advanced = waited["advanced_game_ticks"].as_u64().ok_or("elapsed ticks")?;
    assert!(advanced > 0 && advanced < 1_000, "{waited}");
    let work = &waited["agent_turn"]["active_work"];
    assert!(work["query_baselines"].as_array().ok_or("baseline catalog")?
        .iter().any(|entry| entry["baseline"] == baseline));
    assert!(work["pending_plans"].as_array().ok_or("pending plans")?
        .iter().any(|entry| entry["plan_digest"] == pending["plan_digest"]));
    let unselected = retained(&owner, &[unrelated_id.clone()])?;
    for field in ["definition", "created_at", "sample_count", "evaluation",
        "evidence_digest", "last_evaluated_anchor"] {
        assert_eq!(unselected[0][field], unrelated_before[0][field], "{field}");
    }
    assert!(work["condition_watches"].as_array().ok_or("active watch catalog")?
        .iter().any(|entry| entry["watch"] == unrelated_id));
    for chosen in [handle(&drink)?, handle(&clock)?] {
        assert!(work["query_watches"].as_array().ok_or("retained watch catalog")?
            .iter().any(|entry| entry["watch"] == chosen));
    }
    read_owner(&owner, |session| {
        assert_eq!(session.last_action, action);
        assert_eq!(session.pending.as_ref().map(|plan| plan.digest.as_str()), pending["plan_digest"].as_str());
        let receipt = session.adapter.action_receipt(action.ok_or("original action")?)
            .ok_or("original receipt")?;
        assert_ne!(receipt.state, CommitState::Cancelled);
        Ok(())
    })?;
    Ok(())
}

#[test]
fn refused_complete_batch_publication_preserves_every_root_after_real_elapsed_time() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("empty", false)?;
    let start = snapshot(&owner)?.tick.0;
    let first = register(&owner, "late-first", tick(start + 7), None, 7, 1, start + 100)?;
    let second = register(&owner, "late-second", tick(start + 7), None, 7, 1, start + 100)?;
    let handles = vec![handle(&second)?, handle(&first)?];
    with_owner(&owner, |session| {
        let started = std::time::Instant::now();
        let mut selected = selected_watch_set_wait::SelectedWatchSetWait::new(
            session, handles.clone(), None, 50, started,
        )?;
        let context = read_context(session);
        let mut progress = lab_progress::ForegroundProgress::new_with_start(
            &session.adapter, 50, &context, started,
        )?;
        let base = wait_payload(session, None, &[], &BTreeMap::new(), &progress, true)?;
        selected.sample(session, base)?;
        let initial = parsed(&selected.take_published().ok_or("initial group packet")?)?;
        assert_eq!(initial["selected_watch_set_wait"]["satisfied"], 0);
        let prior = handles.iter().map(|handle| {
            semantic_query::laboratory_watch_record(&read_context(session), handle)
        }).collect::<Result<Vec<_>>>()?;

        // Use the real foreground engine, publishing each accepted world slice.
        // The first slice polls at zero ticks; the next reaches both due samples.
        for _ in 0..2 {
            let context = read_context(session);
            let slice = progress.step(&mut session.adapter, &[], &context, Some(7))?
                .ok_or("foreground slice")?;
            assert!(!slice.finished);
            persist_durable_head(session);
            remember_version(session);
        }
        assert_eq!(progress.advanced_game_ticks(), 7);
        let current = session.adapter.snapshot().anchor();
        assert_eq!(current.tick.0, start + 7);
        let base = wait_payload(session, None, &[], &BTreeMap::new(), &progress, true)?;
        let error = match selected.refuse_sample_publication(session, base) {
            Ok(()) => return Err("injected complete group publication unexpectedly succeeded".into()),
            Err(error) => error,
        };
        assert!(selected.take_published().is_none());

        let mut selection = WaitWatchSelection::Set(selected);
        let refused = parsed(&wait_selection_error(
            session, &error, &progress, &BTreeMap::new(), None, Some(&selection),
        ))?;
        assert_eq!(refused["ok"], false);
        assert_eq!(refused["partial_progress"], true);
        assert_eq!(refused["advanced_game_ticks"], 7);
        assert_eq!(refused["wait_progress"]["advanced_game_ticks"], 7);
        assert_eq!(refused["wait_progress"]["remaining_game_ticks"], 43);
        assert_eq!(refused["observed_anchor"], anchor_json(&current));
        let outcome = &refused["selected_watch_set_wait"];
        assert_eq!(outcome["evaluation_complete"], false);
        assert_eq!(outcome["group_satisfied"], false);
        assert_eq!(outcome["satisfied"], 0);
        assert_eq!(outcome["samples_added"], 0);

        // Formatting an error may omit optional presentation, but every fitting
        // tier must retain the complete selected records and real progress.
        let mut bulky = refused.clone();
        bulky["diagnostic_detail"] = json!("x".repeat(12_000));
        bulky["polled_actions"] = json!((0..30).map(|id| {
            json!({"action_id":id, "message":"x".repeat(500)})
        }).collect::<Vec<_>>());
        let mut fitted_tiers = BTreeSet::new();
        for tokens in [20_000, 8_192, 4_096, 3_000, 2_048, 1_024, 512, 256, 128] {
            let encoded = crate::output_budget::fit(&bulky.to_string(), tokens);
            let fitted = parsed(&encoded)?;
            if fitted["output_budget"]["tier"] == "refused" {
                assert_eq!(fitted["ok"], false);
                assert_eq!(fitted["error"]["code"], "budget_exceeded");
                continue;
            }
            assert!(encoded.len() as u64 <= tokens * 4, "{fitted}");
            assert_eq!(fitted["selected_watch_set_wait"], *outcome, "{fitted}");
            assert_eq!(fitted["wait_progress"], refused["wait_progress"]);
            assert_eq!(fitted["observed_anchor"], anchor_json(&current));
            assert_eq!(fitted["advanced_game_ticks"], 7);
            assert_eq!(fitted["partial_progress"], true);
            if let Some(tier) = fitted["output_budget"]["tier"].as_str() {
                fitted_tiers.insert(tier.to_owned());
            }
        }
        assert!(fitted_tiers.len() >= 2, "no fitting compact error tier was exercised");
        for original in &prior {
            let retained = semantic_query::laboratory_watch_record(
                &read_context(session), &handle(original)?,
            )?;
            let row = record(outcome, &handle(original)?)?;
            assert_eq!(row["evaluation_current"], false);
            assert_eq!(row["needs_poll"], true);
            for field in ["definition", "created_at", "sample_count", "evaluation",
                "evidence_digest", "last_evaluated_anchor", "status"] {
                assert_eq!(row[field], original[field], "error record {field}");
                assert_eq!(retained[field], original[field], "retained root {field}");
            }
        }
        assert_eq!(session.adapter.snapshot().anchor(), current);

        // The refused candidate supplied no samples. A retry publishes both
        // detailed records at this same frontier, without another clock effect.
        let WaitWatchSelection::Set(selected) = &mut selection else {
            return Err("group selection changed type".into());
        };
        let base = wait_payload(session, None, &[], &BTreeMap::new(), &progress, true)?;
        selected.sample(session, base)?;
        let retried = parsed(&selected.take_published().ok_or("retry group packet")?)?;
        let accepted = &retried["selected_watch_set_wait"];
        assert_eq!(accepted["stop_reason"], "all_satisfied");
        assert_eq!(accepted["group_satisfied"], true);
        assert_eq!(accepted["samples_added"], 2);
        for handle in &handles {
            assert_eq!(record(accepted, handle)?["sample_count"], 2);
            assert_eq!(record(accepted, handle)?["status"], "satisfied");
            assert_eq!(record(accepted, handle)?["last_evaluated_anchor"], anchor_json(&current));
        }
        assert_eq!(session.adapter.snapshot().anchor(), current);
        Ok(())
    })?;
    Ok(())
}

#[test]
fn wire_keeps_one_wait_tool_with_optional_set_and_policy_selectors() {
    let definition = facade::FortressWait.definition();
    assert_eq!(definition.name, "fortress_wait");
    let properties = &definition.input_schema["properties"];
    for field in ["session_id", "max_game_ticks", "plan_digest", "watch_id",
        "watch_ids", "watch_policy"] {
        assert!(properties.get(field).is_some(), "{field}");
    }
    for selector in ["plan_digest", "watch_id", "watch_ids", "watch_policy"] {
        assert!(!definition.input_schema["required"].as_array()
            .is_some_and(|fields| fields.iter().any(|field| field == selector)));
    }
}
