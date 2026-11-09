//! Retained resource goals through real publication, effects and durable recovery.

use super::*;
use crate::agent_facade as facade;
use dfmcp_intent::effects;
use dfmcp_world::{Fact, FactSource, Value as WorldValue, laboratory_fact_value};
use serde_json::Value as Json;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn parsed(raw: &str) -> TestResult<Json> {
    Ok(serde_json::from_str(raw)?)
}

fn text(value: &Json, field: &str) -> TestResult<String> {
    Ok(value[field]
        .as_str()
        .ok_or_else(|| format!("missing {field}: {value}"))?
        .to_owned())
}

fn fixture(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InternalInvariantViolation, message)
}

fn owned<T>(session: &str, body: impl FnOnce(&mut LabSession) -> Result<T>) -> TestResult<T> {
    let handle = resolve_session(Some(session.to_owned()))?;
    Ok(with_session(
        &handle,
        || Err(fixture("resource session unavailable")),
        body,
    )?)
}

fn open(selector: &str, shared: bool, durable: bool, tokens: Option<u32>) -> TestResult<String> {
    let result = parsed(&facade::fortress_open_session(
        Some(false),
        Some(selector.to_owned()),
        Some(
            [
                ("observe", "read_only"),
                ("query", "read_only"),
                ("plan", "read_only"),
                ("designate", "guarded"),
                ("checkpoint", "guarded"),
                ("restore", "guarded"),
                ("configure_labor", "reversible"),
                ("configure_production", "reversible"),
                ("control_clock", "reversible"),
            ]
            .into_iter()
            .map(|(capability, risk)| (capability.to_owned(), risk.to_owned()))
            .collect(),
        ),
        Some(60_000),
        Some(12_000),
        Some(8_192),
        Some(4 * 1024 * 1024),
        tokens,
        Some(64),
        Some("starter_fortress_resources".to_owned()),
        Some(shared),
        Some(durable),
    ))?;
    assert_eq!(result["ok"], true, "{result}");
    text(&result, "session_id")
}

fn request() -> Json {
    json!({"template":"resource_acquisition", "assign_labor":true,
        "quotas":[{"resource":"WOOD","minimum":26},{"resource":"PLANT","minimum":20}]})
}

fn plan(session: &str, request: Json, summary: Option<&str>) -> TestResult<Json> {
    parsed(&facade::fortress_plan(
        Some(session.to_owned()),
        summary.map(str::to_owned),
        None,
        None,
        Some(request.to_string()),
    ))
}

fn continuation(session: &str, digest: &str, summary: Option<&str>) -> TestResult<Json> {
    plan(
        session,
        json!({"template":"continue_goal", "plan_digest":digest}),
        summary,
    )
}

fn commit(session: &str, proposal: &Json) -> TestResult<Json> {
    parsed(&facade::fortress_commit(
        Some(session.to_owned()),
        text(proposal, "plan_digest")?,
    ))
}

fn commit_ok(session: &str, proposal: &Json) -> TestResult {
    let result = commit(session, proposal)?;
    assert_eq!(result["ok"], true, "{result}");
    Ok(())
}

fn wait(session: &str, ticks: u64) -> TestResult<Json> {
    let result = parsed(&facade::fortress_wait(
        Some(session.to_owned()),
        Some(ticks),
    ))?;
    assert_eq!(result["ok"], true, "{result}");
    Ok(result)
}

fn goal(session: &str, digest: &str) -> TestResult<Json> {
    owned(session, |session| {
        original_goal_observation(session, digest).map(|(_, goal)| goal)
    })
}

fn pending(session: &str) -> TestResult<Option<String>> {
    owned(session, |session| {
        Ok(session
            .pending
            .as_ref()
            .map(|pending| pending.digest.clone()))
    })
}

fn stock(session: &str, field: &str) -> TestResult<u64> {
    owned(session, |session| {
        let snapshot = session.adapter.snapshot();
        let ledger = effects::stock_ledger(snapshot)
            .and_then(|id| snapshot.graph.entities.get(&id))
            .ok_or_else(|| fixture("resource stock ledger missing"))?;
        match ledger
            .fields
            .get(field)
            .and_then(|fact| laboratory_fact_value(fact, snapshot.tick))
        {
            Some(WorldValue::U64(value)) => Ok(*value),
            _ => Err(fixture("resource stock unavailable")),
        }
    })
}

#[derive(Debug, PartialEq)]
struct Effects {
    snapshot: WorldSnapshot,
    receipts: BTreeMap<String, String>,
    objectives: Vec<Digest32>,
    open_actions: Vec<ActionId>,
    leases: usize,
}

fn effects_before(session: &str) -> TestResult<Effects> {
    owned(session, |session| {
        Ok(Effects {
            snapshot: session.adapter.snapshot().clone(),
            receipts: session.commit_receipts.clone(),
            objectives: session
                .objectives
                .iter()
                .map(|goal| goal.plan_digest)
                .collect(),
            open_actions: session.open_actions.clone(),
            leases: session.leases.by_action.len(),
        })
    })
}

fn unchanged_history(before: &Json, after: &Json) {
    for field in [
        "plan_digest",
        "original_source",
        "owner_session_id",
        "sealed_anchor",
        "terminal_condition",
        "first_satisfied_anchor",
        "historical_achievement",
        "restore_abandoned_anchor",
    ] {
        assert_eq!(before[field], after[field], "historical {field}");
    }
}

fn partial(session: &str) -> TestResult<(String, Json)> {
    let original = plan(
        session,
        request(),
        Some("retain the complete resource reserve"),
    )?;
    assert_eq!(original["ok"], true, "{original}");
    let digest = text(&original, "plan_digest")?;
    commit_ok(session, &original)?;
    let active = continuation(session, &digest, None)?;
    assert_eq!(active["ok"], false, "{active}");
    assert_eq!(active["error"]["code"], "preconditions_failed");
    wait(session, 100)?;
    assert_eq!(stock(session, effects::STOCK_WOOD_FIELD)?, 22);
    let stopped = parsed(&facade::fortress_cancel(
        Some(session.to_owned()),
        Some("stop_future_steps".to_owned()),
        Some("plan".to_owned()),
    ))?;
    assert_eq!(stopped["ok"], true, "{stopped}");
    let history = goal(session, &digest)?;
    assert_eq!(history["predicate_truth"], "false", "{history}");
    assert_eq!(history["physical_quiescent"], true, "{history}");
    Ok((digest, history))
}

fn complete_reference(proposal: &Json) {
    let pending = &proposal["agent_turn"]["active_work"]["pending_plans"][0];
    let reference = &pending["steps_reference"];
    assert_eq!(reference["scope"], "same_response");
    assert_eq!(reference["json_pointer"], "/steps");
    assert_eq!(reference["complete"], true);
    assert_eq!(reference["plan_digest"], proposal["plan_digest"]);
    assert_eq!(
        pending["step_count"].as_u64(),
        proposal["steps"].as_array().map(|steps| steps.len() as u64)
    );
    assert_eq!(proposal.pointer("/steps"), Some(&proposal["steps"]));
    let analysis = &proposal["resource_acquisition"];
    assert_eq!(
        analysis["source_digest"],
        analysis["continuation"]["continuation_source_digest"]
    );
    assert!(
        proposal.get("continuation").is_none(),
        "lineage is published once inside its resource analysis"
    );
}

#[test]
fn cancelled_resource_goal_continues_only_residual_stock_and_preserves_exact_history() -> TestResult
{
    let _serial = serialized_durable_tests();
    let session = open("9642001", false, false, None)?;
    let (root, history) = partial(&session)?;
    let before = effects_before(&session)?;
    let proposed = continuation(&session, &root, None)?;
    assert_eq!(proposed["ok"], true, "{proposed}");
    assert!(proposed.to_string().len() <= 4 * 4_096);
    println!(
        "resource_continuation_default_bytes={}; complete_steps_bytes={}; complete_lineage_bytes={}",
        proposed.to_string().len(),
        proposed["steps"].to_string().len(),
        proposed["resource_acquisition"]["continuation"]
            .to_string()
            .len(),
    );
    assert_eq!(proposed["kind"], "resource_acquisition_continuation");
    assert_eq!(proposed["production"], Json::Null);
    assert_eq!(
        proposed["resource_acquisition"]["continuation"]["parent_plan_digest"],
        root
    );
    assert_eq!(
        proposed["resource_acquisition"]["continuation"]["root_plan_digest"],
        root
    );
    assert_eq!(
        proposed["resource_acquisition"]["original_request"],
        history["original_source"]["request"]
    );
    assert_eq!(
        proposed["resource_acquisition"]["requirements"][0]["deficit"],
        4
    );
    assert_eq!(
        proposed["resource_acquisition"]["requirements"][1]["minimum"],
        20
    );
    assert_eq!(
        proposed["resource_acquisition"]["original_request"]["assign_labor"],
        true
    );
    complete_reference(&proposed);
    assert_eq!(effects_before(&session)?, before);
    let child = text(&proposed, "plan_digest")?;
    commit_ok(&session, &proposed)?;
    wait(&session, 500)?;
    assert_eq!(stock(&session, effects::STOCK_WOOD_FIELD)?, 26);
    assert_eq!(goal(&session, &child)?["predicate_truth"], "true");
    let original = goal(&session, &root)?;
    assert_eq!(original["predicate_truth"], "false");
    unchanged_history(&history, &original);
    assert!(original["continuation_request"].is_null());
    let refused = continuation(&session, &root, None)?;
    assert_eq!(refused["error"]["code"], "invalid_intent", "{refused}");
    assert!(
        refused["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains(&child))
    );
    Ok(())
}

#[test]
fn independently_satisfied_resource_minima_do_not_repeat_a_cancelled_goal() -> TestResult {
    let _serial = serialized_durable_tests();
    let session = open("9642002", false, false, Some(16_384))?;
    let (root, history) = partial(&session)?;
    let independent = plan(
        &session,
        request(),
        Some("separate reviewed resource objective"),
    )?;
    assert_eq!(independent["ok"], true, "{independent}");
    commit_ok(&session, &independent)?;
    wait(&session, 500)?;
    let before = effects_before(&session)?;
    let refused = continuation(&session, &root, None)?;
    assert_eq!(refused["error"]["code"], "invalid_intent", "{refused}");
    assert!(
        refused["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("already hold"))
    );
    assert_eq!(effects_before(&session)?, before);
    let original = goal(&session, &root)?;
    assert!(original["continuation_request"].is_null());
    assert_eq!(original["predicate_truth"], "false");
    unchanged_history(&history, &original);
    Ok(())
}

#[test]
fn resource_continuation_authority_source_and_output_refusals_preserve_pending_and_effects()
-> TestResult {
    let _serial = serialized_durable_tests();
    let session = open("9642003", false, false, Some(16_384))?;
    let (root, _) = partial(&session)?;
    let proposed = continuation(&session, &root, Some("reviewed residual"))?;
    assert_eq!(proposed["ok"], true, "{proposed}");
    let retained = pending(&session)?;
    for capability in [Capability::Observe, Capability::Query, Capability::Plan] {
        owned(&session, |session| {
            for grant in &mut session.grants {
                if grant.capability == capability {
                    grant.expires_at_tick = Some(GameTick(0));
                }
            }
            Ok(())
        })?;
        let before = effects_before(&session)?;
        let refused = continuation(&session, &root, None)?;
        assert_eq!(refused["error"]["code"], "capability_denied", "{refused}");
        let refused = commit(&session, &proposed)?;
        assert_eq!(refused["error"]["code"], "capability_denied", "{refused}");
        assert_eq!(pending(&session)?, retained);
        assert_eq!(effects_before(&session)?, before);
        owned(&session, |session| {
            for grant in &mut session.grants {
                if grant.capability == capability {
                    grant.expires_at_tick = None;
                }
            }
            Ok(())
        })?;
    }
    let original_budget = owned(&session, |session| Ok(session.budget))?;
    for source_bound in [true, false] {
        owned(&session, |session| {
            if source_bound {
                session.budget.max_entities = 1;
            } else {
                session.budget.max_output_tokens = 600;
            }
            Ok(())
        })?;
        let before = effects_before(&session)?;
        let refused = continuation(&session, &root, None)?;
        assert_eq!(refused["error"]["code"], "budget_exceeded", "{refused}");
        assert_eq!(pending(&session)?, retained);
        assert_eq!(effects_before(&session)?, before);
        owned(&session, |session| {
            session.budget = original_budget;
            Ok(())
        })?;
    }
    commit_ok(&session, &proposed)?;
    Ok(())
}

#[test]
fn shared_resource_continuations_refuse_active_and_unknown_same_root_work() -> TestResult {
    let _serial = serialized_durable_tests();
    let first = open("9642004", true, false, Some(16_384))?;
    let (root, _) = partial(&first)?;
    let second = open("9642004", true, false, Some(16_384))?;
    let chosen = continuation(&first, &root, Some("first pursuit"))?;
    let peer = continuation(&second, &root, Some("second pursuit"))?;
    assert_eq!(chosen["ok"], true, "{chosen}");
    assert_eq!(peer["ok"], true, "{peer}");
    let step = owned(&first, |session| {
        session
            .pending
            .as_ref()
            .and_then(|pending| {
                pending
                    .plan
                    .steps
                    .iter()
                    .find(|step| matches!(step.action, Action::HarvestResource { .. }))
            })
            .cloned()
            .ok_or_else(|| fixture("harvest child missing"))
    })?;
    commit_ok(&first, &chosen)?;
    for unknown in [false, true] {
        if unknown {
            owned(&first, |session| {
                let mut snapshot = session.adapter.snapshot().clone();
                let task = effects::created_entity_id(&step.idempotency_key, 0);
                let record = snapshot
                    .graph
                    .entities
                    .get_mut(&task)
                    .ok_or_else(|| fixture("harvest task missing"))?;
                record.fields.insert(
                    effects::STATUS_FIELD.to_owned(),
                    Fact::known(
                        WorldValue::Text(effects::STATUS_CANCELLED.to_owned()),
                        snapshot.tick,
                        FactSource::AgentAssertion("unproved sibling stop".to_owned()),
                        Digest32::ZERO,
                    ),
                );
                record.revision += 1;
                snapshot.cursor = snapshot
                    .cursor
                    .checked_next()
                    .ok_or_else(|| fixture("cursor exhausted"))?;
                snapshot.refresh_hash();
                session.adapter.inject_snapshot(snapshot)
            })?;
        }
        let before = effects_before(&second)?;
        for refused in [continuation(&second, &root, None)?, commit(&second, &peer)?] {
            assert_eq!(refused["ok"], false, "{refused}");
            assert_eq!(
                refused["error"]["code"], "preconditions_failed",
                "{refused}"
            );
            assert!(refused["rebased_plan"].is_null());
        }
        assert_eq!(pending(&second)?.as_deref(), peer["plan_digest"].as_str());
        assert_eq!(effects_before(&second)?, before);
    }
    Ok(())
}

#[test]
fn stale_resource_child_recovers_with_public_parent_request_and_exact_summary() -> TestResult {
    let _serial = serialized_durable_tests();
    let session = open("9642005", false, false, Some(16_384))?;
    let (root, _) = partial(&session)?;
    let proposed = continuation(&session, &root, Some("exact retained child summary"))?;
    assert_eq!(proposed["ok"], true, "{proposed}");
    wait(&session, 1)?;
    let before = effects_before(&session)?;
    let refused = commit(&session, &proposed)?;
    assert_eq!(refused["error"]["code"], "stale_anchor", "{refused}");
    let next = &refused["next_step"];
    assert_eq!(next["tool"], "fortress.plan");
    assert_eq!(next["arguments"]["summary"], "exact retained child summary");
    let request = parsed(&text(&next["arguments"], "blueprint")?)?;
    assert_eq!(
        request,
        json!({"template":"continue_goal","plan_digest":root})
    );
    assert_eq!(
        refused["error"]["recovery"]["minimum_safe_next_step"],
        *next
    );
    assert_eq!(
        pending(&session)?.as_deref(),
        proposed["plan_digest"].as_str()
    );
    assert_eq!(effects_before(&session)?, before);
    let observed = parsed(&facade::fortress_observe(Some(session.clone())))?;
    let pending = &observed["agent_turn"]["active_work"]["pending_plans"][0];
    assert_eq!(pending["state"], "review_required");
    assert_eq!(pending["minimum_safe_next_step"], *next);
    let reviewed = plan(&session, request, Some("exact retained child summary"))?;
    assert_eq!(reviewed["ok"], true, "{reviewed}");
    assert_ne!(reviewed["plan_digest"], proposed["plan_digest"]);
    assert_eq!(
        reviewed["resource_acquisition"]["continuation"]["parent_plan_digest"],
        root
    );
    commit_ok(&session, &reviewed)?;
    Ok(())
}

#[test]
fn resource_continuation_private_source_replays_exactly_and_cannot_be_imported_publicly()
-> TestResult {
    let _serial = serialized_durable_tests();
    let session = open("9642006", false, false, Some(16_384))?;
    let (root, _) = partial(&session)?;
    let proposed = continuation(&session, &root, None)?;
    assert_eq!(proposed["ok"], true, "{proposed}");
    let (source, prepared, snapshot, context) = owned(&session, |session| {
        let pending = session
            .pending
            .as_ref()
            .ok_or_else(|| fixture("continuation missing"))?;
        Ok((
            pending.source.clone(),
            pending.plan.clone(),
            session.adapter.snapshot().clone(),
            read_context(session),
        ))
    })?;
    let durable = source.durable();
    assert!(matches!(
        durable,
        dfmcp_lab::durable::DurablePlanSource::ResourceAcquisitionContinuation { .. }
    ));
    let restored = PlanSource::from_durable(&durable);
    let (intent, _) = restored.compile(prepared.intent_id, &snapshot)?;
    let replayed = StaticPlanner::default().prepare_laboratory(&snapshot, &intent, &context)?;
    assert_eq!(replayed, prepared);
    let PlanSource::ResourceAcquisitionContinuation { raw, .. } = source else {
        return Err("private resource source missing".into());
    };
    let before = effects_before(&session)?;
    for value in [
        parsed(&raw)?,
        json!({"template":"continue_goal","plan_digest":root,"assign_labor":false}),
    ] {
        let refused = plan(&session, value, None)?;
        assert_eq!(refused["ok"], false, "{refused}");
        assert_eq!(
            pending(&session)?.as_deref(),
            proposed["plan_digest"].as_str()
        );
    }
    let comparison = plan(
        &session,
        json!({"template":"compare_candidates","version":1,"horizon_ticks":100,
            "candidates":[{"key":"a","blueprint":{"template":"continue_goal","plan_digest":root}},
                {"key":"b","blueprint":{"template":"continue_goal","plan_digest":root}}]}),
        None,
    )?;
    assert_eq!(comparison["ok"], true, "{comparison}");
    let rows = comparison["candidates"]
        .as_array()
        .ok_or("comparison rows missing")?;
    assert_eq!(rows.len(), 2);
    for row in rows {
        assert_eq!(row["status"], "unavailable", "{row}");
        assert_eq!(row["reason"]["code"], "invalid_request", "{row}");
        assert_eq!(row["selection_available"], false, "{row}");
        assert!(row["next_step"].is_null(), "{row}");
    }
    assert!(
        comparison["recommended_candidate"].is_null(),
        "{comparison}"
    );
    assert_eq!(comparison["recommendation"]["evidence_complete"], false);
    assert_eq!(
        pending(&session)?.as_deref(),
        proposed["plan_digest"].as_str()
    );
    assert_eq!(effects_before(&session)?, before);
    Ok(())
}

struct StateDir(std::path::PathBuf);
impl StateDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-resource-continuation-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        simulate_durable_restart(Some(path.clone()));
        Self(path)
    }
}
impl Drop for StateDir {
    fn drop(&mut self) {
        simulate_durable_restart(None);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn recovered_resource_continuation_keeps_original_policy_lineage_and_exact_history() -> TestResult {
    let _serial = serialized_durable_tests();
    let directory = StateDir::new("reopen");
    let first = open("9642007", false, true, Some(16_384))?;
    let (root, history) = partial(&first)?;
    with_durable_store(|store| store.compact())?;
    simulate_durable_restart(Some(directory.0.clone()));
    let second = open("9642007", false, true, Some(16_384))?;
    let recovered = goal(&second, &root)?;
    unchanged_history(&history, &recovered);
    assert_ne!(
        recovered["sealed_anchor"]["epoch"],
        recovered["observed_anchor"]["epoch"]
    );
    let proposal = continuation(&second, &root, None)?;
    assert_eq!(proposal["ok"], true, "{proposal}");
    let child = text(&proposal, "plan_digest")?;
    commit_ok(&second, &proposal)?;
    wait(&second, 500)?;
    let child_history = goal(&second, &child)?;
    assert_eq!(child_history["predicate_truth"], "true", "{child_history}");
    assert_eq!(
        child_history["original_source"]["request"]["resource_acquisition"],
        history["original_source"]["request"]
    );
    with_durable_store(|store| store.compact())?;
    simulate_durable_restart(Some(directory.0.clone()));
    let third = open("9642007", false, true, Some(16_384))?;
    unchanged_history(&history, &goal(&third, &root)?);
    unchanged_history(&child_history, &goal(&third, &child)?);
    let refused = continuation(&third, &root, None)?;
    assert_eq!(refused["error"]["code"], "invalid_intent", "{refused}");
    let consumed = plan(
        &third,
        json!({"template":"production","quotas":[{"item":"BED","minimum":2}]}),
        None,
    )?;
    assert_eq!(consumed["ok"], true, "{consumed}");
    commit_ok(&third, &consumed)?;
    wait(&third, 500)?;
    assert_eq!(stock(&third, effects::STOCK_WOOD_FIELD)?, 24);
    let grandchild = continuation(
        &third,
        &child,
        Some("restore the original stock after consumption"),
    )?;
    assert_eq!(grandchild["ok"], true, "{grandchild}");
    assert_eq!(
        grandchild["resource_acquisition"]["continuation"]["parent_plan_digest"],
        child
    );
    assert_eq!(
        grandchild["resource_acquisition"]["continuation"]["root_plan_digest"],
        root
    );
    assert_eq!(
        grandchild["resource_acquisition"]["original_request"],
        history["original_source"]["request"]
    );
    assert_eq!(
        grandchild["resource_acquisition"]["requirements"][0]["deficit"],
        2
    );
    commit_ok(&third, &grandchild)?;
    wait(&third, 500)?;
    assert_eq!(
        goal(&third, &text(&grandchild, "plan_digest")?)?["predicate_truth"],
        "true"
    );
    unchanged_history(&history, &goal(&third, &root)?);
    unchanged_history(&child_history, &goal(&third, &child)?);
    Ok(())
}

#[test]
fn restored_resource_goal_stays_abandoned_after_reopen() -> TestResult {
    let _serial = serialized_durable_tests();
    let directory = StateDir::new("restore");
    let first = open("9642008", false, true, Some(16_384))?;
    let checkpoint = parsed(&facade::fortress_checkpoint(
        Some(first.clone()),
        Some("before original harvest".to_owned()),
    ))?;
    assert_eq!(checkpoint["ok"], true, "{checkpoint}");
    let (root, history) = partial(&first)?;
    let restored = parsed(&facade::fortress_restore(
        Some(first),
        text(&checkpoint, "checkpoint_id")?,
    ))?;
    assert_eq!(restored["ok"], true, "{restored}");
    simulate_durable_restart(Some(directory.0.clone()));
    let reopened = open("9642008", false, true, Some(16_384))?;
    let abandoned = goal(&reopened, &root)?;
    assert_eq!(abandoned["status"], "abandoned");
    assert_eq!(abandoned["predicate_truth"], "unknown");
    assert_eq!(abandoned["original_source"], history["original_source"]);
    assert!(!abandoned["restore_abandoned_anchor"].is_null());
    let before = effects_before(&reopened)?;
    let refused = continuation(&reopened, &root, None)?;
    assert_eq!(
        refused["error"]["code"], "preconditions_failed",
        "{refused}"
    );
    assert_eq!(effects_before(&reopened)?, before);
    Ok(())
}
