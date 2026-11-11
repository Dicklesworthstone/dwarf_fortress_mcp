//! Selected condition waiting through the actual public facade and owner.
//! Owning implementation bead: df-lab-structured-query-f7q.3.
//! These tests keep simulation progress, retained evidence and game work distinct.

use super::*;
use crate::agent_facade as facade;
use dfmcp_world::{FactPresence, Value as WorldValue};
use serde_json::Value;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Owner {
    id: String,
}

impl Drop for Owner {
    fn drop(&mut self) {
        if let Ok(id) = u128::from_str_radix(&self.id, 16) {
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
                ("checkpoint", "guarded"),
                ("restore", "guarded"),
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

fn snapshot(owner: &Owner) -> TestResult<WorldSnapshot> {
    with_owner(owner, |session| Ok(session.adapter.snapshot().clone()))
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
    Ok(result)
}

fn handle(created: &Value) -> TestResult<String> {
    Ok(created["record"]["watch"]
        .as_str()
        .ok_or("watch handle")?
        .to_owned())
}

fn detail(owner: &Owner, watch: &str) -> TestResult<Value> {
    let result = query(owner, json!({"kind":"poll_watch", "watch":watch}))?;
    assert_eq!(result["ok"], true, "{result}");
    Ok(result["record"].clone())
}

fn wait(owner: &Owner, watch: &str, ticks: u64) -> TestResult<Value> {
    parsed(&facade::fortress_wait_for_watch(
        Some(owner.id.clone()),
        Some(ticks),
        watch.to_owned(),
    ))
}

fn successful_wait(owner: &Owner, watch: &str, ticks: u64) -> TestResult<Value> {
    let result = wait(owner, watch, ticks)?;
    assert_eq!(result["ok"], true, "{result}");
    let selected = &result["selected_watch_wait"];
    assert_eq!(selected["schema"], "dfmcp.selected-watch-wait/1");
    assert_eq!(selected["watch_id"], watch);
    assert_eq!(selected["record"]["watch"], watch);
    assert_eq!(selected["original_definition_unchanged"], true);
    assert_eq!(selected["game_effect_success_proven"], false);
    assert_eq!(result["agent_turn"]["anchor"], result["observed_anchor"]);
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

#[test]
fn selected_watch_samples_every_frontier_and_stops_at_its_due_stable_tick() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("empty", false)?;
    let start = snapshot(&owner)?.tick.0;
    let created = register(&owner, "three-samples", tick(start + 10), None, 7, 3, start + 100)?;
    let id = handle(&created)?;
    let waited = successful_wait(&owner, &id, 500)?;
    let selected = &waited["selected_watch_wait"];
    assert_eq!(selected["stop_reason"], "satisfied", "{waited}");
    assert_eq!(selected["watch_satisfied"], true);
    assert_eq!(selected["record"]["stable_observations"], 3);
    assert_eq!(selected["record"]["sample_count"], 5);
    assert_eq!(selected["record"]["evaluation"]["skipped_observations_reset_streak"], false);
    assert_eq!(selected["record"]["definition"], created["record"]["definition"]);
    assert_eq!(waited["advanced_game_ticks"], 28);
    assert_eq!(waited["wait_progress"]["remaining_game_ticks"], 472);
    assert_eq!(snapshot(&owner)?.tick.0, start + 28);
    assert_eq!(detail(&owner, &id)?, selected["record"]);
    Ok(())
}

#[test]
fn watch_wait_retains_current_progress_across_bounded_calls_and_terminal_replay() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("empty", false)?;
    let start = snapshot(&owner)?.tick.0;
    let id = handle(&register(&owner, "bounded", tick(start + 10), None, 7, 2, start + 100)?)?;
    let first = successful_wait(&owner, &id, 14)?;
    assert_eq!(first["selected_watch_wait"]["stop_reason"], "tick_limit");
    assert_eq!(first["selected_watch_wait"]["record"]["stable_observations"], 1);
    assert_eq!(first["advanced_game_ticks"], 14);
    let second = successful_wait(&owner, &id, 500)?;
    assert_eq!(second["selected_watch_wait"]["stop_reason"], "satisfied");
    assert_eq!(second["advanced_game_ticks"], 7);
    assert_eq!(snapshot(&owner)?.tick.0, start + 21);
    let replayed = successful_wait(&owner, &id, 500)?;
    assert_eq!(replayed["advanced_game_ticks"], 0);
    assert_eq!(replayed["selected_watch_wait"]["samples_added"], 0);
    assert_eq!(replayed["selected_watch_wait"]["record"], second["selected_watch_wait"]["record"]);
    Ok(())
}

#[test]
fn selected_watch_stops_at_the_exact_deadline_and_never_credits_late_truth() -> TestResult {
    let _serial = crate::test_serial();
    for (deadline_offset, truth_offset, expected) in [(19, 20, "expired"), (21, 21, "satisfied")] {
        let owner = open("empty", false)?;
        let start = snapshot(&owner)?.tick.0;
        let id = handle(&register(
            &owner, "deadline", tick(start + truth_offset), None, 7, 1, start + deadline_offset,
        )?)?;
        let waited = successful_wait(&owner, &id, 500)?;
        let record = &waited["selected_watch_wait"]["record"];
        assert_eq!(waited["selected_watch_wait"]["stop_reason"], expected, "{waited}");
        assert_eq!(waited["advanced_game_ticks"], deadline_offset);
        assert_eq!(snapshot(&owner)?.tick.0, start + deadline_offset);
        let later = parsed(&facade::fortress_wait(Some(owner.id.clone()), Some(20)))?;
        assert_eq!(later["ok"], true, "{later}");
        let retained = detail(&owner, &id)?;
        assert_eq!(retained["status"], expected);
        assert_eq!(retained["evidence_digest"], record["evidence_digest"]);
        assert_eq!(retained["evaluation"], record["evaluation"]);
        assert_eq!(retained["sample_count"], record["sample_count"]);
    }
    Ok(())
}

#[test]
fn failure_conditions_win_and_unknown_guards_stop_without_claiming_success() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("empty", false)?;
    let start = snapshot(&owner)?.tick.0;
    let id = handle(&register(
        &owner, "failed-first", tick(start + 5), Some(tick(start + 5)), 3, 1, start + 100,
    )?)?;
    let failed = successful_wait(&owner, &id, 100)?;
    assert_eq!(failed["selected_watch_wait"]["stop_reason"], "failed");
    assert_eq!(failed["selected_watch_wait"]["watch_satisfied"], false);
    assert_eq!(failed["selected_watch_wait"]["record"]["evaluation"]["condition"], "true");
    assert_eq!(failed["selected_watch_wait"]["record"]["evaluation"]["failure_condition"], "true");
    assert_eq!(failed["advanced_game_ticks"], 6);
    let now = snapshot(&owner)?.tick.0;
    let guarded = handle(&register(&owner, "unknown-failure", tick(now), Some(stock(1)), 3, 1, now + 100)?)?;
    let before = snapshot(&owner)?.anchor();
    let unknown = successful_wait(&owner, &guarded, 100)?;
    assert_eq!(unknown["selected_watch_wait"]["stop_reason"], "blocked_unknown");
    assert_eq!(unknown["selected_watch_wait"]["watch_satisfied"], false);
    assert_eq!(unknown["selected_watch_wait"]["record"]["stable_observations"], 0);
    assert_eq!(unknown["advanced_game_ticks"], 0);
    assert_eq!(snapshot(&owner)?.anchor(), before);
    Ok(())
}

#[test]
fn a_paused_world_does_not_advance_or_manufacture_stable_samples() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("empty", true)?;
    let start = snapshot(&owner)?.tick.0;
    let created = register(&owner, "paused", tick(start + 5), None, 3, 2, start + 100)?;
    let id = handle(&created)?;
    let before = snapshot(&owner)?.anchor();
    let waited = successful_wait(&owner, &id, 100)?;
    assert_eq!(waited["selected_watch_wait"]["stop_reason"], "paused");
    assert_eq!(waited["advanced_game_ticks"], 0);
    assert_eq!(waited["selected_watch_wait"]["record"]["sample_count"], created["record"]["sample_count"]);
    assert_eq!(waited["selected_watch_wait"]["samples_added"], 0);
    assert_eq!(snapshot(&owner)?.anchor(), before);
    Ok(())
}

#[test]
fn terminal_watches_need_no_committed_action_or_clock_budget() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("empty", false)?;
    let now = snapshot(&owner)?.tick.0;
    let satisfied = handle(&register(&owner, "already-met", tick(now), None, 1, 1, now + 100)?)?;
    let cancelled = handle(&register(&owner, "cancelled", tick(now + 50), None, 1, 1, now + 100)?)?;
    let cancellation = query(&owner, json!({"kind":"cancel_watch", "watch":cancelled}))?;
    assert_eq!(cancellation["record"]["status"], "cancelled");
    with_owner(&owner, |session| {
        assert!(session.last_action.is_none());
        session.grants.retain(|grant| grant.capability != Capability::ControlClock);
        Ok(())
    })?;
    let before = snapshot(&owner)?.anchor();
    for (id, status) in [(satisfied, "satisfied"), (cancelled, "cancelled")] {
        let waited = successful_wait(&owner, &id, 0)?;
        assert_eq!(waited["selected_watch_wait"]["stop_reason"], status);
        assert_eq!(waited["advanced_game_ticks"], 0);
        assert_eq!(waited["selected_watch_wait"]["samples_added"], 0);
        assert_eq!(snapshot(&owner)?.anchor(), before);
    }
    Ok(())
}

#[test]
fn selected_wait_cannot_hide_an_observation_gap_or_generation_reuse() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("empty", false)?;
    let start = snapshot(&owner)?.tick.0;
    let created = register(&owner, "gap", tick(start), None, 7, 3, start + 100)?;
    assert_eq!(created["record"]["stable_observations"], 1);
    let id = handle(&created)?;
    mutate(&owner, |source| {
        source.cursor = source.cursor.checked_next().ok_or("extra cursor")?;
        Ok(())
    })?;
    let waited = successful_wait(&owner, &id, 500)?;
    assert_eq!(waited["selected_watch_wait"]["stop_reason"], "satisfied");
    assert_eq!(waited["advanced_game_ticks"], 21);
    assert_eq!(waited["selected_watch_wait"]["record"]["stable_observations"], 3);

    let reused = open("starter_fortress", false)?;
    let now = snapshot(&reused)?.tick.0;
    let condition = json!({"op":"any", "args":[tick(now), stock(1)]});
    let id = handle(&register(&reused, "reused-generation", condition, None, 7, 2, now + 100)?)?;
    mutate(&reused, |source| {
        let entity = source.graph.entities
            .get_mut(&crate::lab_world::starter::STOCK_LEDGER)
            .ok_or("stock ledger")?;
        entity.generation = entity.generation.checked_add(1).ok_or("generation overflow")?;
        Ok(())
    })?;
    let before = snapshot(&reused)?.anchor();
    let waited = successful_wait(&reused, &id, 500)?;
    assert_eq!(waited["selected_watch_wait"]["stop_reason"], "invalidated", "{waited}");
    assert_eq!(waited["selected_watch_wait"]["watch_satisfied"], false);
    assert_eq!(waited["advanced_game_ticks"], 0);
    assert_eq!(snapshot(&reused)?.anchor(), before);
    Ok(())
}

#[test]
fn current_authority_and_source_output_budgets_refuse_before_time_or_watch_publication() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("starter_fortress", false)?;
    let start = snapshot(&owner)?.tick.0;
    let created = register(&owner, "authority", tick(start + 10), None, 7, 2, start + 100)?;
    let id = handle(&created)?;
    let (grants, budget) = with_owner(&owner, |session| Ok((session.grants.clone(), session.budget)))?;
    let before = snapshot(&owner)?.anchor();
    for defect in 0..7 {
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
                4 => session.budget.max_entities = 1,
                5 => session.budget.max_bytes = 1_024,
                _ => session.budget.max_output_tokens = 128,
            }
            Ok(())
        })?;
        let refused = wait(&owner, &id, 50)?;
        refusal(&refused, if defect < 4 { "capability_denied" } else { "budget_exceeded" });
        if defect == 5 {
            assert!(refused.to_string().len() <= 1_024, "{refused}");
        }
        if defect == 6 {
            assert!(refused.to_string().len() <= 128 * 4, "{refused}");
            assert_eq!(refused["output_budget"]["max_output_tokens"], 128);
        }
        assert_eq!(snapshot(&owner)?.anchor(), before, "{refused}");
        with_owner(&owner, |session| {
            session.grants = grants.clone();
            session.budget = budget;
            Ok(())
        })?;
        assert_eq!(detail(&owner, &id)?, created["record"]);
    }
    Ok(())
}

#[test]
fn expired_query_authority_cannot_advance_into_an_unobservable_selected_frontier() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("empty", false)?;
    let start = snapshot(&owner)?.tick.0;
    let created = register(&owner, "expiry", tick(start + 14), None, 7, 1, start + 100)?;
    let id = handle(&created)?;
    let before = snapshot(&owner)?.anchor();
    let grants = with_owner(&owner, |session| {
        let original = session.grants.clone();
        for grant in &mut session.grants {
            if grant.capability == Capability::Query {
                grant.expires_at_tick = Some(GameTick(start + 8));
            }
        }
        Ok(original)
    })?;
    let refused = wait(&owner, &id, 50)?;
    refusal(&refused, "capability_denied");
    // The requested positive horizon is authorized before any clock effect.
    assert_eq!(snapshot(&owner)?.anchor(), before);
    with_owner(&owner, |session| {
        session.grants = grants;
        Ok(())
    })?;
    assert_eq!(detail(&owner, &id)?, created["record"]);
    Ok(())
}

#[test]
fn invalid_foreign_and_conflicting_selectors_preserve_both_world_and_watch() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("empty", false)?;
    let foreign = open("empty", false)?;
    let now = snapshot(&owner)?.tick.0;
    let created = register(&owner, "owned", tick(now + 10), None, 7, 1, now + 100)?;
    let id = handle(&created)?;
    let foreign_id = handle(&register(&foreign, "foreign", tick(now + 10), None, 7, 1, now + 100)?)?;
    let before = snapshot(&owner)?.anchor();
    for invalid in ["malformed".to_owned(), format!("watch:{}", "ab".repeat(32)), foreign_id] {
        let refused = wait(&owner, &invalid, 50)?;
        assert_eq!(refused["ok"], false, "{refused}");
        assert_eq!(snapshot(&owner)?.anchor(), before);
        assert_eq!(detail(&owner, &id)?, created["record"]);
    }
    let conflict = parsed(&facade::fortress_wait_for_selection(
        Some(owner.id.clone()), Some(50), Some("ab".repeat(32)), Some(id.clone()),
    ))?;
    refusal(&conflict, "invalid_request");
    assert_eq!(snapshot(&owner)?.anchor(), before);
    assert_eq!(detail(&owner, &id)?, created["record"]);
    Ok(())
}

#[test]
fn selected_watch_progresses_real_production_without_replacing_other_work_or_catalogs() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("starter_fortress", false)?;
    let planned = parsed(&facade::fortress_plan(
        Some(owner.id.clone()), Some("brew for a retained stock condition".to_owned()), None,
        Some(json!([{"action":{"kind":"create_work_order", "name":"watch-brew",
            "job_token":"BREW_DRINK", "amount":20}}]).to_string()), None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let committed = parsed(&facade::fortress_commit(
        Some(owner.id.clone()),
        planned["plan_digest"].as_str().ok_or("production digest")?.to_owned(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let original_action = with_owner(&owner, |session| Ok(session.last_action))?;
    let now = snapshot(&owner)?.tick.0;
    let captured = query(&owner, json!({
        "kind":"capture", "key":"roster", "max_game_ticks":2_000,
        "select":{"kind":"entities", "kinds":["unit"], "fields":["alive"]}
    }))?;
    assert_eq!(captured["ok"], true, "{captured}");
    let baseline = captured["captured"]["baseline"].clone();
    let chosen = register(&owner, "drink-reserve", stock(50), None, 10, 2, now + 1_000)?;
    let id = handle(&chosen)?;
    let unrelated = register(&owner, "other-condition", tick(now + 1_500), None, 5, 1, now + 1_900)?;
    let unrelated_id = handle(&unrelated)?;
    let pending = parsed(&facade::fortress_plan(
        Some(owner.id.clone()), Some("retained independent review".to_owned()), Some(true), None, None,
    ))?;
    assert_eq!(pending["ok"], true, "{pending}");
    let waited = successful_wait(&owner, &id, 1_000)?;
    assert_eq!(waited["selected_watch_wait"]["stop_reason"], "satisfied", "{waited}");
    assert_eq!(waited["selected_watch_wait"]["record"]["definition"], chosen["record"]["definition"]);
    let advanced = waited["advanced_game_ticks"].as_u64().ok_or("elapsed ticks")?;
    assert!(advanced > 0 && advanced < 1_000, "{waited}");
    let work = &waited["agent_turn"]["active_work"];
    assert!(work["query_baselines"].as_array().ok_or("baseline catalog")?
        .iter().any(|entry| entry["baseline"] == baseline));
    assert!(work["pending_plans"].as_array().ok_or("pending plans")?
        .iter().any(|entry| entry["plan_digest"] == pending["plan_digest"]));
    let retained = work["query_watches"].as_array().ok_or("watch catalog")?
        .iter().find(|entry| entry["watch"] == unrelated_id).ok_or("unrelated watch")?;
    assert_eq!(retained["evidence_digest"], unrelated["record"]["evidence_digest"]);
    assert_eq!(retained["last_evaluated_anchor"], unrelated["record"]["last_evaluated_anchor"]);
    assert!(work["query_watches"].as_array().ok_or("watch catalog")?
        .iter().any(|entry| entry["watch"] == id));
    with_owner(&owner, |session| {
        assert_eq!(session.last_action, original_action);
        assert_eq!(session.pending.as_ref().map(|plan| plan.digest.as_str()), pending["plan_digest"].as_str());
        let receipt = session.adapter.action_receipt(original_action.ok_or("original action")?)
            .ok_or("original receipt")?;
        assert_ne!(receipt.state, CommitState::Cancelled);
        Ok(())
    })?;
    Ok(())
}

#[test]
fn unknown_source_and_restore_never_reuse_old_stability_as_current_truth() -> TestResult {
    let _serial = crate::test_serial();
    for restore_epoch in [false, true] {
        let owner = open("starter_fortress", false)?;
        let now = snapshot(&owner)?.tick.0;
        let created = register(&owner, "source-bound", stock(1), None, 7, 2, now + 100)?;
        assert_eq!(created["record"]["stable_observations"], 1);
        let id = handle(&created)?;
        mutate(&owner, |source| {
            if restore_epoch {
                source.cursor = source.cursor.checked_reset_epoch().ok_or("epoch overflow")?;
            } else {
                let fact = source.graph.entities
                    .get_mut(&crate::lab_world::starter::STOCK_LEDGER)
                    .and_then(|record| record.fields.get_mut(dfmcp_intent::effects::STOCK_DRINK_FIELD))
                    .ok_or("stock fact")?;
                fact.value = WorldValue::U64(1_000);
                fact.presence = Some(FactPresence::Redacted("private stock".to_owned()));
            }
            Ok(())
        })?;
        let before = snapshot(&owner)?.anchor();
        let waited = successful_wait(&owner, &id, 50)?;
        assert_eq!(waited["selected_watch_wait"]["stop_reason"],
            if restore_epoch { "invalidated" } else { "blocked_unknown" }, "{waited}");
        assert_eq!(waited["selected_watch_wait"]["watch_satisfied"], false);
        assert_eq!(waited["selected_watch_wait"]["record"]["stable_observations"], 0);
        assert_eq!(waited["advanced_game_ticks"], 0);
        assert_eq!(snapshot(&owner)?.anchor(), before);
    }
    Ok(())
}

#[test]
fn wait_wire_keeps_one_name_with_both_optional_target_selectors() {
    let definition = facade::FortressWait.definition();
    assert_eq!(definition.name, "fortress_wait");
    let properties = &definition.input_schema["properties"];
    for field in ["session_id", "max_game_ticks", "plan_digest", "watch_id"] {
        assert!(properties.get(field).is_some(), "{field}");
    }
    for selector in ["plan_digest", "watch_id"] {
        assert!(!definition.input_schema["required"].as_array()
            .is_some_and(|fields| fields.iter().any(|field| field == selector)));
    }
}

#[test]
fn late_watch_publication_refusal_keeps_real_elapsed_time_and_the_prior_evidence() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("empty", false)?;
    let start = snapshot(&owner)?.tick.0;
    let created = register(&owner, "late-publication", tick(start + 7), None, 7, 1, start + 100)?;
    let id = handle(&created)?;
    with_owner(&owner, |session| {
        let started = std::time::Instant::now();
        let mut selected = selected_watch_wait::SelectedWatchWait::new(
            session, id.clone(), 50, started,
        )?;
        let context = read_context(session);
        let mut progress = lab_progress::ForegroundProgress::new_with_start(
            &session.adapter, 50, &context, started,
        )?;
        let base = |session: &LabSession, progress: &lab_progress::ForegroundProgress| {
            let source = session.adapter.snapshot();
            json!({
                "ok":true, "session_id":session.session_id.to_string(),
                "observed_anchor":anchor_json(&source.anchor()),
                "game_tick":source.tick.0, "paused":source.paused,
                "advanced_game_ticks":progress.advanced_game_ticks(),
                "wait_progress":wait_progress_json(
                    50, progress.advanced_game_ticks(), progress.slices()
                )
            })
        };
        selected.sample(session, base(session, &progress))?;
        let initial = parsed(&selected.take_published().ok_or("initial packet")?)?;
        assert_eq!(initial["selected_watch_wait"]["record"]["status"], "waiting");
        let prior = semantic_query::laboratory_watch_record(&read_context(session), &id)?;

        // This is the actual foreground clock engine: its first slice polls at
        // zero ticks, and the next publishes exactly the requested due frontier.
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

        // Reject only after the complete candidate packet has rendered. The
        // sampled truth is now positive, but that candidate has not published.
        let error = match selected.refuse_sample_publication(session, base(session, &progress)) {
            Ok(()) => return Err("injected final publication unexpectedly succeeded".into()),
            Err(error) => error,
        };
        let refused = parsed(&wait_progress_error(
            session, &error, &progress, &BTreeMap::new(), None, Some(&selected),
        ))?;
        assert_eq!(refused["ok"], false);
        assert_eq!(refused["partial_progress"], true);
        assert_eq!(refused["advanced_game_ticks"], 7);
        assert_eq!(refused["wait_progress"]["advanced_game_ticks"], 7);
        assert_eq!(refused["wait_progress"]["remaining_game_ticks"], 43);
        assert_eq!(refused["observed_anchor"], anchor_json(&current));
        let outcome = &refused["selected_watch_wait"];
        assert_eq!(outcome["evaluation_complete"], false);
        assert_eq!(outcome["watch_satisfied"], false);
        assert_eq!(outcome["record"]["evaluation_current"], false);
        assert_eq!(outcome["record"]["needs_poll"], true);
        assert_eq!(outcome["record"]["last_evaluated_anchor"], prior["last_evaluated_anchor"]);
        assert_eq!(outcome["record"]["evidence_digest"], prior["evidence_digest"]);
        assert_eq!(outcome["record"]["sample_count"], prior["sample_count"]);
        assert_eq!(outcome["record"]["status"], "waiting");
        assert!(selected.take_published().is_none());

        let retained = semantic_query::laboratory_watch_record(&read_context(session), &id)?;
        assert_eq!(retained["evidence_digest"], prior["evidence_digest"]);
        assert_eq!(retained["last_evaluated_anchor"], prior["last_evaluated_anchor"]);
        assert_eq!(retained["sample_count"], prior["sample_count"]);
        assert_eq!(session.adapter.snapshot().anchor(), current);

        // An explicit retry may publish the same frontier once; it must neither
        // advance time again nor count the refused candidate as an extra sample.
        selected.sample(session, base(session, &progress))?;
        let retried = parsed(&selected.take_published().ok_or("retry packet")?)?;
        assert_eq!(retried["selected_watch_wait"]["stop_reason"], "satisfied");
        assert_eq!(retried["selected_watch_wait"]["samples_added"], 1);
        assert_eq!(retried["selected_watch_wait"]["record"]["sample_count"], 2);
        assert_eq!(session.adapter.snapshot().anchor(), current);
        Ok(())
    })?;
    Ok(())
}

#[test]
fn replay_records_the_exact_watch_selector_but_refuses_cross_session_handle_reuse() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("empty", false)?;
    let start = snapshot(&owner)?.tick.0;
    let id = handle(&register(&owner, "replay-watch", tick(start + 7), None, 7, 1, start + 100)?)?;
    let waited = successful_wait(&owner, &id, 50)?;
    assert_eq!(waited["selected_watch_wait"]["stop_reason"], "satisfied");
    let before = snapshot(&owner)?.anchor();
    let bundle = replay_bundle_for(&owner.id)?;
    let call = bundle["calls"].as_array()
        .and_then(|calls| calls.last()).ok_or("last replay call")?;
    assert_eq!(call["tool"], "fortress.wait");
    assert_eq!(call["arguments"]["watch_id"], id);
    assert_eq!(call["arguments"]["max_game_ticks"], 50);
    assert_eq!(bundle["replayable"], false, "{bundle}");
    assert!(bundle["not_replayable_reason"].as_str()
        .is_some_and(|reason| reason.contains("watch")), "{bundle}");
    let refused = crate::replay::replay_bundle(&bundle);
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["replayed"], 0);
    assert_eq!(snapshot(&owner)?.anchor(), before);
    Ok(())
}

#[test]
fn partial_watch_evidence_and_elapsed_progress_survive_every_fitting_output_tier() -> TestResult {
    let prior = json!({"fortress_id":"1", "epoch":0, "sequence":0,
        "game_tick":1, "state_hash":"11".repeat(32)});
    let current = json!({"fortress_id":"1", "epoch":0, "sequence":1,
        "game_tick":8, "state_hash":"22".repeat(32)});
    let selected = json!({
        "schema":"dfmcp.selected-watch-wait/1", "watch_id":format!("watch:{}", "ab".repeat(32)),
        "record":{
            "watch":format!("watch:{}", "ab".repeat(32)), "status":"candidate", "terminal":false,
            "last_evaluated_anchor":prior, "evaluation_current":false, "needs_poll":true,
            "evidence_digest":"33".repeat(32), "sample_count":1,
            "stable_observations":1, "required_stable_observations":2,
            "definition":{
                "key":"exact-condition",
                "condition":{"op":"all", "args":[tick(1), tick(1)]},
                "deadline_tick":100, "poll_interval_ticks":7, "stable_observations":2
            },
            "evaluation":{
                "condition":"true", "failure_condition":"false",
                "facts":[{"field":"a", "truth":"true"}, {"field":"b", "truth":"true"}]
            }
        },
        "evaluation_complete":false, "watch_satisfied":false,
        "original_definition_unchanged":true, "game_effect_success_proven":false
    });
    let progress = json!({
        "requested_game_ticks":50, "advanced_game_ticks":7, "remaining_game_ticks":43,
        "action_polls":0, "deferred_actions_dispatched":0
    });
    let original = json!({
        "ok":false, "session_id":"s",
        "error":{"operation":"fortress.wait", "code":"budget_exceeded",
            "message":"later watch publication was refused"},
        "selected_watch_wait":selected, "wait_progress":progress,
        "observed_anchor":current, "advanced_game_ticks":7, "game_tick":8,
        "paused":false, "partial_progress":true, "durability_pending":false,
        "diagnostic_detail":"x".repeat(12_000),
        "polled_actions":(0..30).map(|id| {
            json!({"action_id":id, "message":"x".repeat(500)})
        }).collect::<Vec<_>>(),
        "agent_turn":{
            "schema":"dfmcp.agent_turn/1", "operation":"fortress.wait", "anchor":current,
            "continuity":{"status":"partial"}, "active_work":{}, "recommendations":[], "coverage":{}
        }
    });
    let mut minimal_seen = false;
    for tokens in [20_000, 4_096, 3_000, 2_048, 1_024, 512, 256, 128, 50] {
        let encoded = crate::output_budget::fit(&original.to_string(), tokens);
        let fitted = parsed(&encoded)?;
        assert_eq!(fitted["ok"], false);
        if fitted["output_budget"]["tier"] == "refused" {
            assert_eq!(fitted["error"]["code"], "budget_exceeded");
            continue;
        }
        assert!(encoded.len() as u64 <= tokens * 4, "{fitted}");
        assert_eq!(fitted["selected_watch_wait"], selected, "{fitted}");
        assert_eq!(fitted["wait_progress"], progress, "{fitted}");
        assert_eq!(fitted["observed_anchor"], current);
        assert_eq!(fitted["advanced_game_ticks"], 7);
        assert_eq!(fitted["partial_progress"], true);
        minimal_seen |= fitted["output_budget"]["tier"] == "minimal";
    }
    assert!(minimal_seen, "minimal selected-watch error envelope was not exercised");
    Ok(())
}
