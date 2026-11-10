//! Original production goals across separately reviewed resource and output stages.

use super::*;
use crate::agent_facade as facade;
use dfmcp_intent::{HarvestResource, effects};
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
        || Err(fixture("resource production session unavailable")),
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

fn request(quotas: Json) -> Json {
    json!({"template":"production", "quotas":quotas,
        "acquire_resources":{"assign_labor":false}})
}

fn beds(minimum: u32) -> Json {
    request(json!([{"item":"BED","minimum":minimum}]))
}

fn raw_plan(session: &str, request: &str, summary: Option<&str>) -> TestResult<Json> {
    parsed(&facade::fortress_plan(
        Some(session.to_owned()),
        summary.map(str::to_owned),
        None,
        None,
        Some(request.to_owned()),
    ))
}

fn plan(session: &str, request: Json, summary: Option<&str>) -> TestResult<Json> {
    raw_plan(session, &request.to_string(), summary)
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

fn prepared(session: &str) -> TestResult<PreparedPlan> {
    owned(session, |session| {
        session
            .pending
            .as_ref()
            .map(|pending| pending.plan.clone())
            .ok_or_else(|| fixture("resource production plan missing"))
    })
}

fn stock(session: &str, field: &str) -> TestResult<u64> {
    owned(session, |session| {
        let snapshot = session.adapter.snapshot();
        let ledger = effects::stock_ledger(snapshot)
            .and_then(|id| snapshot.graph.entities.get(&id))
            .ok_or_else(|| fixture("production stock ledger missing"))?;
        match ledger
            .fields
            .get(field)
            .and_then(|fact| laboratory_fact_value(fact, snapshot.tick))
        {
            Some(WorldValue::U64(value)) => Ok(*value),
            _ => Err(fixture("production stock unavailable")),
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
    assert!(pending.get("steps").is_none());
}

fn assert_stage(proposal: &Json, stage: &str, child: bool) {
    assert_eq!(proposal["ok"], true, "{proposal}");
    assert_eq!(proposal["schema"], "dfmcp.resource-production/1");
    assert_eq!(
        proposal["kind"],
        if child {
            "resource_production_continuation"
        } else {
            "resource_production"
        }
    );
    let workflow = &proposal["production"]["resource_workflow"];
    assert_eq!(workflow["stage"], stage, "{proposal}");
    assert_eq!(workflow["planner"], "resource_aware_v1");
    assert_eq!(workflow["original_request"]["planner"], "resource_aware_v1");
    assert_eq!(
        workflow["original_request"]["acquire_resources"]["assign_labor"],
        false
    );
    complete_reference(proposal);
}

fn assert_lineage(proposal: &Json, parent: &str, root: &str) {
    let lineage = &proposal["production"]["continuation"];
    assert_eq!(lineage["parent_plan_digest"], parent);
    assert_eq!(lineage["root_plan_digest"], root);
    assert_eq!(lineage["source_schema"], "dfmcp.production-continuation/4");
    assert_eq!(lineage["requires_explicit_commit"], true);
    assert_eq!(lineage["original_resource_policy_preserved"], true);
    assert_eq!(lineage["requires_observed_inputs_for_production"], true);
    assert_eq!(lineage["next_stage_dispatched"], false);
}

fn only_harvest(session: &str, resource: HarvestResource, amount: u32) -> TestResult {
    let plan = prepared(session)?;
    assert_eq!(plan.steps.len(), 1, "{plan:?}");
    assert!(
        matches!(plan.steps[0].action, Action::HarvestResource { resource: actual, amount: units, .. }
            if actual == resource && units == amount),
        "{plan:?}"
    );
    Ok(())
}

fn partial(session: &str) -> TestResult<(String, Json)> {
    let original = plan(session, beds(26), Some("retain twenty-six beds"))?;
    assert_stage(&original, "acquire_resources", false);
    only_harvest(session, HarvestResource::Wood, 6)?;
    let digest = text(&original, "plan_digest")?;
    commit_ok(session, &original)?;
    let active = continuation(session, &digest, None)?;
    assert_eq!(active["error"]["code"], "preconditions_failed", "{active}");
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

#[test]
fn resource_production_keeps_bed_goal_and_requires_observed_wood_before_output() -> TestResult {
    let _serial = serialized_durable_tests();
    let session = open("9643001", false, false, None)?;
    let ordinary = json!({"template":"production", "quotas":[{"item":"BED","minimum":22}]});
    let blocked = plan(&session, ordinary.clone(), None)?;
    assert_eq!(blocked["ok"], false, "{blocked}");
    assert!(pending(&session)?.is_none());
    let before = effects_before(&session)?;
    let acquisition = plan(&session, beds(22), Some("make twenty-two beds"))?;
    assert_stage(&acquisition, "acquire_resources", false);
    assert!(acquisition.to_string().len() <= 4 * 4_096);
    assert_eq!(effects_before(&session)?, before);
    only_harvest(&session, HarvestResource::Wood, 2)?;
    let terminal = prepared(&session)?.terminal_condition;
    let root = text(&acquisition, "plan_digest")?;
    assert_eq!(stock(&session, effects::STOCK_WOOD_FIELD)?, 20);
    assert_eq!(stock(&session, effects::STOCK_BED_FIELD)?, 0);

    // A successfully published forecast is not input stock or an order.
    let blocked = plan(&session, ordinary.clone(), None)?;
    assert_eq!(blocked["ok"], false, "{blocked}");
    assert_eq!(pending(&session)?.as_deref(), Some(root.as_str()));
    commit_ok(&session, &acquisition)?;
    let blocked = plan(&session, ordinary, None)?;
    assert_eq!(blocked["ok"], false, "{blocked}");
    wait(&session, 100)?;
    assert_eq!(stock(&session, effects::STOCK_WOOD_FIELD)?, 22);
    assert_eq!(stock(&session, effects::STOCK_BED_FIELD)?, 0);
    let acquired = goal(&session, &root)?;
    assert_eq!(acquired["predicate_truth"], "false");
    assert_eq!(acquired["physical_quiescent"], true);
    assert_eq!(acquired["historical_achievement"], "not_recorded");

    let before = effects_before(&session)?;
    let production = continuation(&session, &root, None)?;
    assert_stage(&production, "production", true);
    assert_lineage(&production, &root, &root);
    assert!(production.to_string().len() <= 4 * 4_096);
    assert_eq!(effects_before(&session)?, before);
    let prepared = prepared(&session)?;
    assert_eq!(prepared.terminal_condition, terminal);
    assert!(
        prepared
            .steps
            .iter()
            .all(|step| matches!(step.action, Action::CreateWorkOrder { .. }))
    );
    let child = text(&production, "plan_digest")?;
    commit_ok(&session, &production)?;
    wait(&session, 3_000)?;
    assert_eq!(stock(&session, effects::STOCK_WOOD_FIELD)?, 0);
    assert_eq!(stock(&session, effects::STOCK_BED_FIELD)?, 22);
    assert_eq!(goal(&session, &child)?["predicate_truth"], "true");
    assert_eq!(goal(&session, &root)?["predicate_truth"], "true");
    let satisfied = continuation(&session, &root, None)?;
    assert_eq!(satisfied["error"]["code"], "invalid_intent", "{satisfied}");
    Ok(())
}

#[test]
fn resource_production_recomputes_consumption_and_plant_inputs_after_harvest() -> TestResult {
    let _serial = serialized_durable_tests();
    let session = open("9643002", false, false, Some(16_384))?;
    wait(&session, 1_100)?;
    let quotas = json!([{"item":"DRINK","minimum":140},{"item":"FOOD","minimum":65}]);
    let acquisition = plan(&session, request(quotas.clone()), None)?;
    assert_stage(&acquisition, "acquire_resources", false);
    // The sealed shared-worker deadlines conservatively budget service and
    // observation delay, so the combined horizon needs more than the raw
    // recipe deficit of one plant at this source.
    only_harvest(&session, HarvestResource::Plant, 10)?;
    let root = text(&acquisition, "plan_digest")?;
    let terminal = prepared(&session)?.terminal_condition;
    commit_ok(&session, &acquisition)?;
    wait(&session, 500)?;
    assert_eq!(stock(&session, effects::STOCK_PLANT_FIELD)?, 30);
    assert_eq!(stock(&session, effects::STOCK_DRINK_FIELD)?, 33);
    assert_eq!(stock(&session, effects::STOCK_FOOD_FIELD)?, 60);
    assert_eq!(goal(&session, &root)?["predicate_truth"], "false");

    let proposal = continuation(&session, &root, None)?;
    assert_stage(&proposal, "production", true);
    assert_eq!(prepared(&session)?.terminal_condition, terminal);
    let requirement = proposal["production"]["requirements"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["item"] == "DRINK"))
        .ok_or("current drink requirement missing")?;
    assert_eq!(requirement["stock"], 33);
    assert_eq!(requirement["minimum_stock"], 140);
    let child = text(&proposal, "plan_digest")?;
    commit_ok(&session, &proposal)?;
    wait(&session, 1_500)?;
    assert_eq!(stock(&session, effects::STOCK_PLANT_FIELD)?, 2);
    assert!(stock(&session, effects::STOCK_DRINK_FIELD)? >= 140);
    assert!(stock(&session, effects::STOCK_FOOD_FIELD)? >= 65);
    assert_eq!(goal(&session, &child)?["predicate_truth"], "true");
    Ok(())
}

#[test]
fn cancelled_resource_production_keeps_exact_history_and_continues_residual_then_output()
-> TestResult {
    let _serial = serialized_durable_tests();
    let session = open("9643003", false, false, Some(16_384))?;
    let (root, history) = partial(&session)?;
    let residual = continuation(&session, &root, None)?;
    assert_stage(&residual, "acquire_resources", true);
    assert_lineage(&residual, &root, &root);
    only_harvest(&session, HarvestResource::Wood, 4)?;
    let residual_digest = text(&residual, "plan_digest")?;
    commit_ok(&session, &residual)?;
    wait(&session, 200)?;
    assert_eq!(stock(&session, effects::STOCK_WOOD_FIELD)?, 26);
    assert_eq!(stock(&session, effects::STOCK_BED_FIELD)?, 0);
    let proposal = continuation(&session, &residual_digest, None)?;
    assert_stage(&proposal, "production", true);
    assert_lineage(&proposal, &residual_digest, &root);
    let output_digest = text(&proposal, "plan_digest")?;
    commit_ok(&session, &proposal)?;
    wait(&session, 3_000)?;
    assert_eq!(stock(&session, effects::STOCK_BED_FIELD)?, 26);
    assert_eq!(stock(&session, effects::STOCK_WOOD_FIELD)?, 0);
    assert_eq!(goal(&session, &output_digest)?["predicate_truth"], "true");
    assert_eq!(goal(&session, &residual_digest)?["predicate_truth"], "true");
    let original = goal(&session, &root)?;
    assert_eq!(original["predicate_truth"], "false");
    assert_eq!(original["semantic_output_goal"]["predicate_truth"], "true");
    assert_eq!(
        original["semantic_output_goal"]["historical_action_proof_rewritten"],
        false
    );
    assert_eq!(original["needs_replan"], false);
    assert!(original["continuation_request"].is_null());
    unchanged_history(&history, &original);
    let before = effects_before(&session)?;
    let refused = continuation(&session, &root, None)?;
    assert_eq!(refused["error"]["code"], "invalid_intent", "{refused}");
    assert_eq!(effects_before(&session)?, before);
    Ok(())
}

#[test]
fn resource_production_authority_and_publication_refusals_preserve_reviewed_pending() -> TestResult
{
    let _serial = serialized_durable_tests();
    let session = open("9643004", false, false, Some(16_384))?;
    let (root, _) = partial(&session)?;
    let proposed = continuation(&session, &root, Some("reviewed residual ingredients"))?;
    assert_stage(&proposed, "acquire_resources", true);
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
    let budget = owned(&session, |session| Ok(session.budget))?;
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
            session.budget = budget;
            Ok(())
        })?;
    }
    owned(&session, |session| {
        for grant in &mut session.grants {
            if grant.capability == Capability::Designate {
                grant.expires_at_tick = Some(GameTick(0));
            }
        }
        Ok(())
    })?;
    let before = effects_before(&session)?;
    let denied = commit(&session, &proposed)?;
    assert_eq!(denied["error"]["code"], "capability_denied", "{denied}");
    assert_eq!(pending(&session)?, retained);
    assert_eq!(effects_before(&session)?, before);
    Ok(())
}

#[test]
fn shared_resource_production_rechecks_active_and_unknown_siblings_at_commit() -> TestResult {
    let _serial = serialized_durable_tests();
    let first = open("9643005", true, false, Some(16_384))?;
    let (root, _) = partial(&first)?;
    let second = open("9643005", true, false, Some(16_384))?;
    let chosen = continuation(&first, &root, Some("first retained production pursuit"))?;
    let peer = continuation(
        &second,
        &root,
        Some("competing retained production pursuit"),
    )?;
    assert_stage(&chosen, "acquire_resources", true);
    assert_stage(&peer, "acquire_resources", true);
    let step = prepared(&first)?
        .steps
        .into_iter()
        .find(|step| matches!(step.action, Action::HarvestResource { .. }))
        .ok_or("resource child missing")?;
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
fn stale_resource_production_requires_explicit_original_request_review_at_both_stages() -> TestResult
{
    let _serial = serialized_durable_tests();
    let session = open("9643006", false, false, Some(16_384))?;
    let acquisition = plan(
        &session,
        beds(22),
        Some("exact original production summary"),
    )?;
    assert_stage(&acquisition, "acquire_resources", false);
    wait(&session, 1)?;
    let before = effects_before(&session)?;
    let stale = commit(&session, &acquisition)?;
    assert_eq!(stale["error"]["code"], "stale_anchor", "{stale}");
    assert_eq!(
        stale["rebase"]["method"],
        "fresh_resource_production_required"
    );
    assert!(stale["rebased_plan"].is_null());
    assert_eq!(effects_before(&session)?, before);
    assert_eq!(
        pending(&session)?.as_deref(),
        acquisition["plan_digest"].as_str()
    );
    let next = &stale["next_step"];
    assert_eq!(next["tool"], "fortress.plan");
    assert_eq!(
        next["arguments"]["summary"],
        "exact original production summary"
    );
    let original_request = parsed(&text(&next["arguments"], "blueprint")?)?;
    assert_eq!(original_request["quotas"], beds(22)["quotas"]);
    assert_eq!(original_request["planner"], "resource_aware_v1");
    let reviewed = plan(
        &session,
        original_request,
        Some("exact original production summary"),
    )?;
    assert_stage(&reviewed, "acquire_resources", false);
    let root = text(&reviewed, "plan_digest")?;
    commit_ok(&session, &reviewed)?;
    wait(&session, 100)?;
    let production = continuation(&session, &root, Some("exact production child summary"))?;
    assert_stage(&production, "production", true);
    wait(&session, 1)?;
    let before = effects_before(&session)?;
    let stale = commit(&session, &production)?;
    assert_eq!(stale["error"]["code"], "stale_anchor", "{stale}");
    assert_eq!(
        stale["rebase"]["method"],
        "fresh_resource_production_required"
    );
    assert!(stale["rebased_plan"].is_null());
    assert_eq!(effects_before(&session)?, before);
    assert_eq!(
        pending(&session)?.as_deref(),
        production["plan_digest"].as_str()
    );
    let next = &stale["next_step"];
    assert_eq!(
        next["arguments"]["summary"],
        "exact production child summary"
    );
    let source = parsed(&text(&next["arguments"], "blueprint")?)?;
    assert_eq!(
        source,
        json!({"template":"continue_goal","plan_digest":root})
    );
    assert_eq!(stale["error"]["recovery"]["minimum_safe_next_step"], *next);
    let reviewed = plan(&session, source, Some("exact production child summary"))?;
    assert_stage(&reviewed, "production", true);
    assert_ne!(reviewed["plan_digest"], production["plan_digest"]);
    commit_ok(&session, &reviewed)?;
    Ok(())
}

#[test]
fn resource_production_refuses_unknown_inputs_invalid_policy_and_satisfied_original_goal()
-> TestResult {
    let _serial = serialized_durable_tests();
    let session = open("9643007", false, false, Some(16_384))?;
    let ordinary_stage = plan(&session, beds(2), None)?;
    assert_stage(&ordinary_stage, "production", false);
    assert!(
        prepared(&session)?
            .steps
            .iter()
            .all(|step| matches!(step.action, Action::CreateWorkOrder { .. }))
    );
    let retained = pending(&session)?;
    let before = effects_before(&session)?;
    let mut reserve = beds(22);
    reserve["reserve_until_tick"] = json!(2_401);
    let mut unknown_policy = beds(22);
    unknown_policy["acquire_resources"]["automatic_commit"] = json!(true);
    for invalid in [
        reserve,
        unknown_policy,
        request(json!([{"item":"DRINK","minimum":40}])),
    ] {
        let refused = plan(&session, invalid, None)?;
        assert_eq!(refused["ok"], false, "{refused}");
        assert_eq!(pending(&session)?, retained);
        assert_eq!(effects_before(&session)?, before);
    }
    let duplicate = r#"{"template":"production","quotas":[{"item":"BED","minimum":22}],"acquire_resources":{"assign_labor":false,"assign_labor":true}}"#;
    let refused = raw_plan(&session, duplicate, None)?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(pending(&session)?, retained);

    owned(&session, |session| {
        let mut snapshot = session.adapter.snapshot().clone();
        let ledger =
            effects::stock_ledger(&snapshot).ok_or_else(|| fixture("stock ledger missing"))?;
        let record = snapshot
            .graph
            .entities
            .get_mut(&ledger)
            .ok_or_else(|| fixture("stock ledger missing"))?;
        record.fields.insert(
            effects::STOCK_WOOD_FIELD.to_owned(),
            Fact::known(
                WorldValue::U64(20),
                snapshot.tick,
                FactSource::AgentAssertion("unobserved finite wood".to_owned()),
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
    let before = effects_before(&session)?;
    let refused = plan(&session, beds(22), None)?;
    assert_eq!(
        refused["error"]["code"], "preconditions_failed",
        "{refused}"
    );
    assert_eq!(pending(&session)?, retained);
    assert_eq!(effects_before(&session)?, before);
    Ok(())
}

#[test]
fn compared_resource_production_selects_original_source_without_forecasting_next_stage()
-> TestResult {
    let _serial = serialized_durable_tests();
    let session = open("9643010", false, false, Some(16_384))?;
    let mut staffing = beds(22);
    staffing["acquire_resources"]["assign_labor"] = json!(true);
    let before = effects_before(&session)?;
    let report = plan(
        &session,
        json!({"template":"compare_candidates", "version":1, "horizon_ticks":500,
            "candidates":[{"key":"existing_workers","blueprint":beds(22)},
                {"key":"assign_workers","blueprint":staffing}]}),
        None,
    )?;
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["kind"], "compare_candidates");
    assert_eq!(report["effect_evidence"], false);
    assert_eq!(report["forecast_work"]["actual_game_ticks_advanced"], 0);
    assert_eq!(report["pending_plan_changed"], false);
    assert_eq!(effects_before(&session)?, before);
    assert!(pending(&session)?.is_none());
    let rows = report["candidates"]
        .as_array()
        .ok_or("comparison rows missing")?;
    assert_eq!(rows.len(), 2);
    for row in rows {
        assert_eq!(row["status"], "evaluated", "{row}");
        assert_eq!(row["prediction"]["predicted_complete"], true, "{row}");
        assert_eq!(row["prediction"]["predicted_goal_truth"], "false", "{row}");
        assert!(row.get("plan_digest").is_none());
    }
    let chosen = rows
        .iter()
        .find(|row| row["key"] == "existing_workers")
        .ok_or("selected candidate missing")?;
    assert_eq!(chosen["next_step"]["tool"], "fortress.plan");
    let source = text(&chosen["next_step"]["arguments"], "blueprint")?;
    let selected = raw_plan(&session, &source, None)?;
    assert_eq!(selected["ok"], true, "{selected}");
    assert_eq!(selected["kind"], "select_candidate");
    assert_eq!(
        selected["production"]["resource_workflow"]["stage"],
        "acquire_resources"
    );
    complete_reference(&selected);
    only_harvest(&session, HarvestResource::Wood, 2)?;
    assert_eq!(effects_before(&session)?, before);
    owned(&session, |session| {
        let source = &session
            .pending
            .as_ref()
            .ok_or_else(|| fixture("selected source missing"))?
            .source;
        let PlanSource::Production { raw, .. } = source else {
            return Err(fixture(
                "comparison replaced the original production source",
            ));
        };
        let source: Json = serde_json::from_str(raw)
            .map_err(|_| fixture("selected production source is invalid"))?;
        assert_eq!(source["planner"], "resource_aware_v1");
        assert_eq!(source["quotas"], beds(22)["quotas"]);
        assert_eq!(source["acquire_resources"]["assign_labor"], false);
        Ok(())
    })?;
    let root = text(&selected, "plan_digest")?;
    commit_ok(&session, &selected)?;
    wait(&session, 100)?;
    assert_eq!(stock(&session, effects::STOCK_BED_FIELD)?, 0);
    assert_eq!(stock(&session, effects::STOCK_WOOD_FIELD)?, 22);
    assert_eq!(goal(&session, &root)?["predicate_truth"], "false");
    let next = continuation(&session, &root, None)?;
    assert_stage(&next, "production", true);
    assert_lineage(&next, &root, &root);
    commit_ok(&session, &next)?;
    wait(&session, 3_000)?;
    assert_eq!(stock(&session, effects::STOCK_BED_FIELD)?, 22);
    Ok(())
}

#[test]
fn replaced_ledger_cannot_satisfy_a_retained_production_goal_or_replace_pending_review()
-> TestResult {
    let _serial = serialized_durable_tests();
    let session = open("9643011", false, false, Some(16_384))?;
    let (root, history) = partial(&session)?;
    let residual = continuation(&session, &root, Some("retain the original ledger"))?;
    assert_stage(&residual, "acquire_resources", true);
    let retained = pending(&session)?;
    owned(&session, |session| {
        let mut snapshot = session.adapter.snapshot().clone();
        let ledger =
            effects::stock_ledger(&snapshot).ok_or_else(|| fixture("original ledger missing"))?;
        let record = snapshot
            .graph
            .entities
            .get_mut(&ledger)
            .ok_or_else(|| fixture("original ledger missing"))?;
        record.generation = record
            .generation
            .checked_add(1)
            .ok_or_else(|| fixture("fixture generation exhausted"))?;
        record.fields.insert(
            effects::STOCK_BED_FIELD.to_owned(),
            Fact::known(
                WorldValue::U64(26),
                snapshot.tick,
                FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
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
    // This new ledger has qualified sufficient stock. Its generation still
    // cannot establish the output goal bound to the original ledger.
    assert_eq!(stock(&session, effects::STOCK_BED_FIELD)?, 26);
    let before = effects_before(&session)?;
    let observed = parsed(&facade::fortress_observe(Some(session.clone())))?;
    assert_eq!(observed["ok"], true, "{observed}");
    let original = observed["objectives"]
        .as_array()
        .and_then(|rows| rows.iter().find(|goal| goal["plan_digest"] == root))
        .ok_or("original production objective missing from public observation")?;
    assert_eq!(
        original["semantic_output_goal"]["predicate_truth"], "unknown",
        "{original}"
    );
    assert_eq!(
        original["semantic_output_goal"]["historical_action_proof_rewritten"],
        false
    );
    assert!(original["continuation_request"].is_null());
    unchanged_history(&history, original);
    for refused in [
        continuation(&session, &root, None)?,
        commit(&session, &residual)?,
    ] {
        assert_eq!(
            refused["error"]["code"], "preconditions_failed",
            "{refused}"
        );
        assert!(refused["rebased_plan"].is_null());
    }
    assert_eq!(pending(&session)?, retained);
    assert_eq!(effects_before(&session)?, before);
    Ok(())
}

#[test]
fn replacement_stock_cannot_record_achievement_for_a_funded_production_root() -> TestResult {
    let _serial = serialized_durable_tests();
    let session = open("9643012", false, false, Some(16_384))?;
    let original = plan(
        &session,
        beds(2),
        Some("produce from existing original stock"),
    )?;
    assert_stage(&original, "production", false);
    assert!(
        prepared(&session)?
            .steps
            .iter()
            .all(|step| matches!(step.action, Action::CreateWorkOrder { .. }))
    );
    let root = text(&original, "plan_digest")?;
    commit_ok(&session, &original)?;
    let stopped = parsed(&facade::fortress_cancel(
        Some(session.clone()),
        Some("stop_future_steps".to_owned()),
        Some("plan".to_owned()),
    ))?;
    assert_eq!(stopped["ok"], true, "{stopped}");
    assert_eq!(stock(&session, effects::STOCK_BED_FIELD)?, 0);
    let history = goal(&session, &root)?;
    assert_eq!(history["predicate_truth"], "false");
    assert_eq!(history["historical_achievement"], "not_recorded");
    let unrelated = plan(
        &session,
        beds(22),
        Some("keep this separate reviewed proposal"),
    )?;
    assert_stage(&unrelated, "acquire_resources", false);
    let retained = pending(&session)?;
    owned(&session, |session| {
        let mut snapshot = session.adapter.snapshot().clone();
        let ledger =
            effects::stock_ledger(&snapshot).ok_or_else(|| fixture("original ledger missing"))?;
        let record = snapshot
            .graph
            .entities
            .get_mut(&ledger)
            .ok_or_else(|| fixture("original ledger missing"))?;
        record.generation = record
            .generation
            .checked_add(1)
            .ok_or_else(|| fixture("fixture generation exhausted"))?;
        record.fields.insert(
            effects::STOCK_BED_FIELD.to_owned(),
            Fact::known(
                WorldValue::U64(2),
                snapshot.tick,
                FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
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
    assert_eq!(stock(&session, effects::STOCK_BED_FIELD)?, 2);
    let before = effects_before(&session)?;
    let observed = parsed(&facade::fortress_observe(Some(session.clone())))?;
    assert_eq!(observed["ok"], true, "{observed}");
    let original = observed["objectives"]
        .as_array()
        .and_then(|rows| rows.iter().find(|goal| goal["plan_digest"] == root))
        .ok_or("funded original objective missing from public observation")?;
    assert_eq!(original["predicate_truth"], "unknown", "{original}");
    assert_eq!(
        original["semantic_output_goal"]["predicate_truth"], "unknown",
        "{original}"
    );
    assert_eq!(original["historical_achievement"], "not_recorded");
    assert!(original["first_satisfied_anchor"].is_null());
    assert!(original["continuation_request"].is_null());
    unchanged_history(&history, original);
    let refused = continuation(&session, &root, None)?;
    assert_eq!(
        refused["error"]["code"], "preconditions_failed",
        "{refused}"
    );
    assert_eq!(pending(&session)?, retained);
    assert_eq!(effects_before(&session)?, before);
    Ok(())
}

struct StateDir(std::path::PathBuf);

impl StateDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-production-resources-{label}-{}",
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
fn resource_production_reopens_between_stages_with_same_original_source_and_goal() -> TestResult {
    let _serial = serialized_durable_tests();
    let directory = StateDir::new("reopen");
    let first = open("9643008", false, true, Some(16_384))?;
    let acquisition = plan(&first, beds(22), Some("persist the production objective"))?;
    assert_stage(&acquisition, "acquire_resources", false);
    let root = text(&acquisition, "plan_digest")?;
    commit_ok(&first, &acquisition)?;
    wait(&first, 100)?;
    let history = goal(&first, &root)?;
    assert_eq!(history["predicate_truth"], "false");
    with_durable_store(|store| store.compact())?;
    simulate_durable_restart(Some(directory.0.clone()));
    let second = open("9643008", false, true, Some(16_384))?;
    let reopened = goal(&second, &root)?;
    unchanged_history(&history, &reopened);
    assert_ne!(
        reopened["sealed_anchor"]["epoch"],
        reopened["observed_anchor"]["epoch"]
    );
    assert_eq!(stock(&second, effects::STOCK_WOOD_FIELD)?, 22);
    assert_eq!(stock(&second, effects::STOCK_BED_FIELD)?, 0);
    let proposal = continuation(&second, &root, None)?;
    assert_stage(&proposal, "production", true);
    assert_lineage(&proposal, &root, &root);
    let (source, expected, snapshot, context) = owned(&second, |session| {
        let pending = session
            .pending
            .as_ref()
            .ok_or_else(|| fixture("pending source missing"))?;
        Ok((
            pending.source.clone(),
            pending.plan.clone(),
            session.adapter.snapshot().clone(),
            read_context(session),
        ))
    })?;
    let restored = PlanSource::from_durable(&source.durable());
    let (intent, _) = restored.compile(expected.intent_id, &snapshot)?;
    let replayed = StaticPlanner::default().prepare_laboratory(&snapshot, &intent, &context)?;
    assert_eq!(replayed, expected);
    let PlanSource::ProductionContinuation { raw, .. } = source else {
        return Err("private production continuation missing".into());
    };
    let archived = parsed(&raw)?;
    assert_eq!(archived["schema"], "dfmcp.production-continuation/4");
    assert_eq!(
        archived["production"],
        history["original_source"]["request"]
    );
    let before = effects_before(&second)?;
    let refused = plan(&second, archived, None)?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(effects_before(&second)?, before);
    let child = text(&proposal, "plan_digest")?;
    commit_ok(&second, &proposal)?;
    wait(&second, 3_000)?;
    assert_eq!(stock(&second, effects::STOCK_BED_FIELD)?, 22);
    let completed = goal(&second, &child)?;
    assert_eq!(completed["predicate_truth"], "true");
    with_durable_store(|store| store.compact())?;
    simulate_durable_restart(Some(directory.0.clone()));
    let third = open("9643008", false, true, Some(16_384))?;
    unchanged_history(&completed, &goal(&third, &child)?);
    let refused = continuation(&third, &root, None)?;
    assert_eq!(refused["error"]["code"], "invalid_intent", "{refused}");
    Ok(())
}

#[test]
fn restored_resource_production_goal_cannot_be_continued_after_durable_reopen() -> TestResult {
    let _serial = serialized_durable_tests();
    let directory = StateDir::new("restore");
    let first = open("9643009", false, true, Some(16_384))?;
    let checkpoint = parsed(&facade::fortress_checkpoint(
        Some(first.clone()),
        Some("before production procurement".to_owned()),
    ))?;
    assert_eq!(checkpoint["ok"], true, "{checkpoint}");
    let (root, history) = partial(&first)?;
    let restored = parsed(&facade::fortress_restore(
        Some(first),
        text(&checkpoint, "checkpoint_id")?,
    ))?;
    assert_eq!(restored["ok"], true, "{restored}");
    simulate_durable_restart(Some(directory.0.clone()));
    let second = open("9643009", false, true, Some(16_384))?;
    let abandoned = goal(&second, &root)?;
    assert_eq!(abandoned["status"], "abandoned");
    assert_eq!(abandoned["predicate_truth"], "unknown");
    assert_eq!(abandoned["original_source"], history["original_source"]);
    assert!(!abandoned["restore_abandoned_anchor"].is_null());
    let before = effects_before(&second)?;
    let refused = continuation(&second, &root, None)?;
    assert_eq!(
        refused["error"]["code"], "preconditions_failed",
        "{refused}"
    );
    assert_eq!(effects_before(&second)?, before);
    Ok(())
}
