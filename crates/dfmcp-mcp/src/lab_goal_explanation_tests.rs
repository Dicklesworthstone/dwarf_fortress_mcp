//! Real public-facade inspection without proof, clock or custody mutation.

use super::*;
use crate::agent_facade as facade;
use crate::server::{read_context, resolve_session, resource_replay_bundle, with_session_read};
use dfmcp_core::{EntityId, GameTick, SessionId};
use dfmcp_world::{CompareOp, Fact, FactSource, Value as WorldValue};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Owner(String);

impl Drop for Owner {
    fn drop(&mut self) {
        if let Ok(id) = u128::from_str_radix(&self.0, 16) {
            let _ = crate::server::sessions().remove(&SessionId::new(id));
        }
    }
}

fn parsed(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn text(value: &Value, key: &str) -> TestResult<String> {
    Ok(value[key]
        .as_str()
        .ok_or_else(|| format!("missing {key}: {value}"))?
        .to_owned())
}

fn open(scenario: &str, paused: bool) -> TestResult<Owner> {
    let value = parsed(&facade::fortress_open_session(
        Some(paused),
        None,
        Some(
            [
                ("observe", "read_only"),
                ("query", "read_only"),
                ("plan", "read_only"),
                ("control_clock", "reversible"),
                ("configure_production", "reversible"),
                ("configure_labor", "reversible"),
                ("checkpoint", "guarded"),
                ("restore", "guarded"),
                ("designate", "guarded"),
                ("construct", "guarded"),
            ]
            .into_iter()
            .map(|(cap, risk)| (cap.to_owned(), risk.to_owned()))
            .collect(),
        ),
        Some(60_000),
        Some(12_000),
        Some(8_192),
        Some(4 * 1024 * 1024),
        Some(4_096),
        Some(64),
        Some(scenario.to_owned()),
        Some(false),
        Some(false),
    ))?;
    assert_eq!(value["ok"], true, "{value}");
    Ok(Owner(text(&value, "session_id")?))
}

fn explain(owner: &Owner, selector: &str) -> TestResult<Value> {
    let output = facade::fortress_explain(Some(owner.0.clone()), Some(selector.to_owned()));
    let value = parsed(&output)?;
    if value["ok"] == true {
        assert!(output.len() <= 4_096 * 4, "{value}");
    }
    Ok(value)
}

fn follow(owner: &Owner, call: &Value) -> TestResult<Value> {
    assert_eq!(call["tool"], "fortress.explain");
    assert_eq!(call["arguments"]["session_id"], owner.0);
    explain(owner, &text(&call["arguments"], "entity_id")?)
}

fn prepare_pause(owner: &Owner, paused: bool) -> TestResult<String> {
    let value = parsed(&facade::fortress_plan(
        Some(owner.0.clone()),
        Some("inspect original pause intention".to_owned()),
        Some(paused),
        None,
        None,
    ))?;
    assert_eq!(value["ok"], true, "{value}");
    text(&value, "plan_digest")
}

fn fingerprint(owner: &Owner) -> TestResult<Value> {
    let handle = resolve_session(Some(owner.0.clone()))?;
    with_session_read(
        &handle,
        |error| Err(error.into()),
        |session| {
            Ok(json!({
                "anchor":anchor_json(&session.adapter.snapshot().anchor()),
                "request_id":read_context(session).request_id.to_string(),
                "objectives":format!("{:?}",session.objectives),
                "pending":session.pending.as_ref().map(|pending|(&pending.digest,format!("{:?}",pending.plan))),
                "transcript":format!("{:?}",session.adapter.transcript()),
                "replay":resource_replay_bundle(session),
                "commits":session.commit_receipts,
                "open_actions":session.open_actions.iter().map(ToString::to_string).collect::<Vec<_>>(),
            }))
        },
    )
}

fn refused(value: &Value, code: &str) {
    assert_eq!(value["ok"], false, "{value}");
    assert_eq!(value["error"]["code"], code, "{value}");
    assert_eq!(value["error"]["effect_may_have_occurred"], false, "{value}");
    assert!(value.get("goal").is_none(), "{value}");
    assert!(value["agent_turn"]["anchor"].is_null(), "{value}");
}

#[test]
fn typed_selector_grammar_is_closed_and_preserves_legacy_routing() -> TestResult {
    let digest = "1".repeat(64);
    for prefix in ["plan", "goal"] {
        assert!(subject(&format!("{prefix}:{digest}")).is_ok());
        assert!(subject(&format!("{prefix}:{digest}~{digest}")).is_ok());
        assert!(subject(&format!("{prefix}:{digest}~{digest}~{digest}~steps~0")).is_ok());
        for raw in [
            format!("{prefix}:"),
            format!("{prefix}:{}", "A".repeat(64)),
            format!("{prefix}:{digest}~"),
            format!("{prefix}:{digest}~{digest}~{digest}"),
            format!("{prefix}:{digest}~{digest}~{digest}~steps~01"),
            format!("{prefix}:{digest}~{digest}~{digest}~source~0"),
            format!("{prefix}:{digest}~{digest}~{digest}~steps~0~extra"),
        ] {
            assert!(is_subject(&raw));
            assert!(subject(&raw).is_err(), "{raw}");
        }
    }
    assert!(!is_subject("1001"));
    let owner = open("empty", true)?;
    let before = fingerprint(&owner)?;
    refused(&explain(&owner, "goal:bad")?, "invalid_request");
    assert_eq!(before, fingerprint(&owner)?);
    let legacy = parsed(&facade::fortress_explain(Some(owner.0.clone()), None))?;
    assert_eq!(legacy["ok"], true);
    assert!(legacy["recent_events"].is_array());
    Ok(())
}

#[test]
fn pending_then_committed_original_explains_truth_receipts_without_changing_state() -> TestResult {
    let owner = open("empty", true)?;
    let digest = prepare_pause(&owner, false)?;
    let before = fingerprint(&owner)?;
    let pending = explain(&owner, &format!("plan:{digest}"))?;
    assert_eq!(pending["ok"], true, "{pending}");
    assert_eq!(pending["conclusion"], "prepared_plan_not_committed");
    assert_eq!(pending["requirements_summary"]["false"], 1);
    assert_eq!(pending["goal"]["historical_achievement"], "not_committed");
    let steps = follow(&owner, &pending["original_steps"])?;
    assert_eq!(
        steps["rows"][0]["dispatch_status"],
        "prepared_not_dispatched"
    );
    refused(&explain(&owner, &format!("goal:{digest}"))?, "stale_anchor");
    assert_eq!(before, fingerprint(&owner)?);
    let committed = parsed(&facade::fortress_commit(
        Some(owner.0.clone()),
        digest.clone(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let before = fingerprint(&owner)?;
    let goal = explain(&owner, &format!("goal:{digest}"))?;
    assert_eq!(goal["ok"], true, "{goal}");
    assert_eq!(goal["goal"]["predicate_truth"], "true");
    assert_eq!(goal["goal"]["historical_achievement"], "verified");
    assert_eq!(goal["goal"]["physical_quiescent"], true);
    let steps = follow(&owner, &goal["original_steps"])?;
    assert_eq!(steps["rows"][0]["receipt"]["state"], "Verified");
    assert_eq!(steps["rows"][0]["physical_work"]["quiescent"], true);
    assert_eq!(steps["rows"][0]["obligation"], Value::Null);
    assert_eq!(before, fingerprint(&owner)?);
    Ok(())
}

#[test]
fn current_authority_source_output_and_deadline_refusals_are_side_effect_free() -> TestResult {
    let owner = open("empty", false)?;
    let advanced = parsed(&facade::fortress_wait(Some(owner.0.clone()), Some(1)))?;
    assert_eq!(advanced["ok"], true, "{advanced}");
    let digest = prepare_pause(&owner, true)?;
    let selector = format!("plan:{digest}");
    let handle = resolve_session(Some(owner.0.clone()))?;
    let (grants, budget) = {
        let guard = handle.lock().map_err(|_| "session poisoned")?;
        (guard.grants.clone(), guard.budget)
    };
    for capability in [Capability::Observe, Capability::Query] {
        {
            handle
                .lock()
                .map_err(|_| "session poisoned")?
                .grants
                .retain(|grant| grant.capability != capability);
        }
        let before = fingerprint(&owner)?;
        refused(&explain(&owner, &selector)?, "capability_denied");
        refused(
            &explain(&owner, &format!("goal:{}", "f".repeat(64)))?,
            "capability_denied",
        );
        assert_eq!(before, fingerprint(&owner)?);
        handle.lock().map_err(|_| "session poisoned")?.grants = grants.clone();
    }
    for narrow in ["scope", "expiry", "uses", "records", "bytes", "output"] {
        {
            let mut guard = handle.lock().map_err(|_| "session poisoned")?;
            guard.grants = grants.clone();
            guard.budget = budget;
            match narrow {
                "scope" => {
                    for grant in &mut guard.grants {
                        if grant.capability == Capability::Query {
                            grant.scope.entity_ids.insert(EntityId::new(1));
                        }
                    }
                }
                "expiry" => {
                    for grant in &mut guard.grants {
                        if grant.capability == Capability::Observe {
                            grant.expires_at_tick = Some(GameTick(0));
                        }
                    }
                }
                "uses" => {
                    for grant in &mut guard.grants {
                        if grant.capability == Capability::Observe {
                            grant.remaining_uses = Some(1);
                        }
                    }
                }
                "records" => guard.budget.max_entities = 1,
                "bytes" => guard.budget.max_bytes = 64,
                _ => guard.budget.max_output_tokens = 600,
            }
        }
        let before = fingerprint(&owner)?;
        let result = explain(&owner, &selector)?;
        assert_eq!(result["ok"], false, "{narrow}: {result}");
        assert_eq!(before, fingerprint(&owner)?);
    }
    {
        let mut guard = handle.lock().map_err(|_| "session poisoned")?;
        guard.grants = grants;
        guard.budget = budget;
        let late = Instant::now()
            .checked_sub(Duration::from_secs(61))
            .ok_or("test clock")?;
        assert_eq!(
            inspect(&guard, &selector, late)
                .err()
                .ok_or("deadline should refuse")?
                .code,
            ErrorCode::BudgetExceeded
        );
    }
    Ok(())
}

#[test]
fn requirements_pages_pin_the_full_anchor_and_exact_typed_object() -> TestResult {
    let owner = open("empty", true)?;
    prepare_pause(&owner, false)?;
    let handle = resolve_session(Some(owner.0.clone()))?;
    let digest = {
        let mut session = handle.lock().map_err(|_| "session poisoned")?;
        let pending = session.pending.as_mut().ok_or("missing pending")?;
        pending.plan.terminal_condition = Predicate::All(
            (10..30)
                .map(|id| Predicate::EntityExists(EntityId::new(id)))
                .collect(),
        )
        .normalized();
        pending.plan.digest = pending.plan.compute_digest();
        pending.plan.id = pending.plan.expected_id();
        pending.digest = pending.plan.digest.to_hex();
        pending.digest.clone()
    };
    let first = explain(&owner, &format!("plan:{digest}"))?;
    assert_eq!(first["ok"], true, "{first}");
    assert_eq!(first["total"], 20);
    assert_eq!(first["complete"], false);
    let mut rows = first["rows"].as_array().ok_or("missing rows")?.clone();
    let mut next = first["next"].clone();
    let before = fingerprint(&owner)?;
    while !next.is_null() {
        let page = follow(&owner, &next)?;
        assert_eq!(page["ok"], true, "{page}");
        assert_eq!(page["object_digest"], first["object_digest"]);
        assert_eq!(page["observation_pin"], first["observation_pin"]);
        assert_eq!(page["offset"].as_u64(), Some(rows.len() as u64));
        rows.extend(page["rows"].as_array().ok_or("missing rows")?.clone());
        next = page["next"].clone();
    }
    assert_eq!(rows.len(), 20);
    assert_eq!(
        rows.iter()
            .map(|row| row["path"].as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        20
    );
    assert_eq!(before, fingerprint(&owner)?);
    refused(
        &explain(&owner, &format!("plan:{digest}~{}", "f".repeat(64)))?,
        "stale_anchor",
    );
    {
        let mut session = handle.lock().map_err(|_| "session poisoned")?;
        let mut snapshot = session.adapter.snapshot().clone();
        // Same semantic root, but another observation cursor is a different source.
        snapshot.cursor = snapshot.cursor.checked_next().ok_or("cursor exhausted")?;
        snapshot.refresh_hash();
        session.adapter.inject_snapshot(snapshot)?;
    }
    refused(&follow(&owner, &first["next"])?, "cursor_gap");
    let fresh = explain(&owner, &format!("plan:{digest}"))?;
    assert_eq!(fresh["ok"], true, "{fresh}");
    assert_ne!(fresh["observation_pin"], first["observation_pin"]);
    Ok(())
}

#[test]
fn same_anchor_historical_proof_changes_invalidate_original_goal_pages() -> TestResult {
    let owner = open("empty", true)?;
    prepare_pause(&owner, true)?;
    let handle = resolve_session(Some(owner.0.clone()))?;
    let digest = {
        let mut session = handle.lock().map_err(|_| "session poisoned")?;
        let pending = session.pending.take().ok_or("missing pending")?;
        let mut plan = pending.plan;
        // Model the owner's retained original before its progress-publication
        // postlude: every clause holds, but no first-achievement proof exists.
        // Twenty distinct clauses force a real continuation through the facade.
        plan.terminal_condition = Predicate::All(
            (10..30)
                .map(|id| {
                    Predicate::Any(vec![
                        Predicate::Paused(true),
                        Predicate::EntityExists(EntityId::new(id)),
                    ])
                })
                .collect(),
        )
        .normalized();
        plan.digest = plan.compute_digest();
        plan.id = plan.expected_id();
        let digest = plan.digest;
        crate::server::objectives::install_objective(
            &mut session,
            &plan,
            pending.source,
            &[],
            None,
        );
        digest
    };
    let selector = format!("goal:{digest}");
    let before = fingerprint(&owner)?;
    let first = explain(&owner, &selector)?;
    assert_eq!(first["ok"], true, "{first}");
    assert_eq!(first["total"], 20);
    assert_eq!(first["complete"], false);
    assert_eq!(first["goal"]["predicate_truth"], "true");
    assert_eq!(first["goal"]["historical_achievement"], "not_recorded");
    let unchanged_page = follow(&owner, &first["next"])?;
    assert_eq!(unchanged_page["ok"], true, "{unchanged_page}");
    assert_eq!(unchanged_page["observation_pin"], first["observation_pin"]);
    assert_eq!(before, fingerprint(&owner)?);
    {
        let mut session = handle.lock().map_err(|_| "session poisoned")?;
        let anchor = session.adapter.snapshot().anchor();
        let satisfied = crate::server::objectives::newly_satisfied_objectives(&session);
        assert!(satisfied.contains(&digest));
        crate::server::objectives::record_objective_progress(&mut session, anchor, &satisfied, &[]);
        assert_eq!(anchor, session.adapter.snapshot().anchor());
    }
    let before = fingerprint(&owner)?;
    refused(&follow(&owner, &first["next"])?, "cursor_gap");
    refused(&follow(&owner, &first["original_steps"])?, "cursor_gap");
    let refreshed = explain(&owner, &selector)?;
    assert_eq!(refreshed["ok"], true, "{refreshed}");
    assert_eq!(refreshed["current_anchor"], first["current_anchor"]);
    assert_eq!(refreshed["object_digest"], first["object_digest"]);
    assert_eq!(refreshed["goal"]["predicate_truth"], "true");
    assert_eq!(refreshed["goal"]["historical_achievement"], "verified");
    assert_ne!(refreshed["observation_pin"], first["observation_pin"]);
    assert_eq!(before, fingerprint(&owner)?);
    Ok(())
}

#[test]
fn both_reserve_planners_keep_future_work_and_expiry_distinct_from_current_stock() {
    for field in ["resource_reserve", "production_reserve"] {
        for (status, expected) in [
            ("work_required", "future_reserve_requires_work"),
            ("expired", "original_reserve_horizon_expired"),
            ("unknown", "original_project_truth_is_unknown"),
            (
                "funded_from_current_stock",
                "original_stage_holds_at_current_anchor",
            ),
        ] {
            let mut goal = json!({"predicate_truth":"true","historical_achievement":"verified",
                "physical_quiescent":true,"semantic_output_goal":{"predicate_truth":"true"}});
            goal[field] = json!({"status":status,"continuous_maintenance_proven":false});
            assert_eq!(conclusion(&goal, false, false), expected, "{goal}");
        }
    }
}

#[test]
fn boolean_diagnostics_preserve_alternatives_and_unqualified_facts_remain_unknown() -> TestResult {
    let owner = open("starter_fortress", true)?;
    prepare_pause(&owner, false)?;
    let handle = resolve_session(Some(owner.0.clone()))?;
    let session = handle.lock().map_err(|_| "session poisoned")?;
    let mut snapshot = session.adapter.snapshot().clone();
    let entity = snapshot
        .graph
        .entities
        .values_mut()
        .next()
        .ok_or("entity missing")?;
    let entity_id = entity.id;
    entity.fields.insert(
        "observed_for_test".into(),
        Fact::known(
            WorldValue::Bool(true),
            snapshot.tick,
            FactSource::AgentAssertion("not source-qualified".into()),
            Digest32::ZERO,
        ),
    );
    snapshot.refresh_hash();
    let unknown = Predicate::FieldCompare {
        entity_id,
        field: "observed_for_test".into(),
        op: CompareOp::Eq,
        value: WorldValue::Bool(true),
    };
    let mut plan = session.pending.as_ref().ok_or("plan missing")?.plan.clone();
    plan.terminal_condition = Predicate::All(vec![
        Predicate::Any(vec![Predicate::Paused(true), unknown.clone()]),
        Predicate::Not(Box::new(Predicate::Paused(false))),
        unknown,
    ]);
    let evidence = PredicateEvidence::laboratory(&snapshot)?;
    let rows = requirements(&plan, &evidence, &session, Instant::now())?;
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["truth"], "true");
    assert!(rows[0]["condition"]["any"].is_array());
    assert_eq!(rows[1]["truth"], "true");
    assert_eq!(rows[2]["truth"], "unknown");
    Ok(())
}

#[test]
fn finished_harvest_explains_unfinished_original_production_and_reviewed_continuation() -> TestResult
{
    let owner = open("starter_fortress_resources", false)?;
    let source = json!({"template":"production","quotas":[{"item":"BED","minimum":22}],
        "acquire_resources":{"assign_labor":false}});
    let proposal = parsed(&facade::fortress_plan(
        Some(owner.0.clone()),
        Some("original 22 beds".into()),
        None,
        None,
        Some(source.to_string()),
    ))?;
    assert_eq!(proposal["ok"], true, "{proposal}");
    let digest = text(&proposal, "plan_digest")?;
    let committed = parsed(&facade::fortress_commit(
        Some(owner.0.clone()),
        digest.clone(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let waited = parsed(&facade::fortress_wait(Some(owner.0.clone()), Some(400)))?;
    assert_eq!(waited["ok"], true, "{waited}");
    let before = fingerprint(&owner)?;
    let report = explain(&owner, &format!("goal:{digest}"))?;
    assert_eq!(report["ok"], true, "{report}");
    // The resource-aware production compiler preserves the complete original
    // output quota in the terminal condition even for the harvest stage.
    assert_eq!(report["goal"]["predicate_truth"], "false", "{report}");
    assert_eq!(
        report["goal"]["semantic_output_goal"]["predicate_truth"], "false",
        "{report}"
    );
    assert_eq!(report["goal"]["historical_achievement"], "not_recorded");
    assert_eq!(report["goal"]["physical_quiescent"], true);
    assert_eq!(
        report["conclusion"],
        "original_goal_requires_reviewed_continuation"
    );
    assert_eq!(report["next_step"], report["goal"]["continuation_request"]);
    assert_eq!(report["blind_retry_allowed"], false);
    assert_eq!(before, fingerprint(&owner)?);
    Ok(())
}

#[test]
fn restored_originals_keep_historical_proof_separate_from_abandoned_pursuit() -> TestResult {
    let owner = open("empty", true)?;
    let checkpoint = parsed(&facade::fortress_checkpoint(
        Some(owner.0.clone()),
        Some("before original".into()),
    ))?;
    assert_eq!(checkpoint["ok"], true, "{checkpoint}");
    let digest = prepare_pause(&owner, false)?;
    let committed = parsed(&facade::fortress_commit(
        Some(owner.0.clone()),
        digest.clone(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let achieved = explain(&owner, &format!("goal:{digest}"))?;
    let restored = parsed(&facade::fortress_restore(
        Some(owner.0.clone()),
        text(&checkpoint, "checkpoint_id")?,
    ))?;
    assert_eq!(restored["ok"], true, "{restored}");
    let before = fingerprint(&owner)?;
    let report = explain(&owner, &format!("goal:{digest}"))?;
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["conclusion"], "original_pursuit_abandoned");
    assert_eq!(report["goal"]["predicate_truth"], "unknown");
    assert_eq!(
        report["goal"]["first_satisfied_anchor"],
        achieved["goal"]["first_satisfied_anchor"]
    );
    assert!(report.get("next_step").is_none());
    assert_eq!(before, fingerprint(&owner)?);
    Ok(())
}
