//! Public agent recommendations acquire finite inputs before reserve production.

use crate::agent_facade::{
    fortress_commit, fortress_observe, fortress_open_session, fortress_plan, fortress_wait_for_plan,
};
use crate::lab_world::{starter, world_alerts};
use dfmcp_adapter::GameAdapter;
use dfmcp_core::{Capability, Digest32, EntityId, GameTick, RiskTier};
use dfmcp_intent::effects;
use dfmcp_world::{Fact, FactSource, Value, WorldSnapshot};
use serde_json::{Value as Json, json};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
const PLANTS: EntityId = EntityId::new(8_002);

fn write(snapshot: &mut WorldSnapshot, id: EntityId, field: &str, value: Value) -> TestResult {
    let tick = snapshot.tick;
    snapshot
        .graph
        .entities
        .get_mut(&id)
        .ok_or("fixture entity missing")?
        .fields
        .insert(
            field.to_owned(),
            Fact::known(
                value,
                tick,
                FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                Digest32::ZERO,
            ),
        );
    snapshot.refresh_hash();
    Ok(())
}

fn parsed(raw: &str) -> TestResult<Json> {
    Ok(serde_json::from_str(raw)?)
}

fn session_change(
    session: &str,
    change: impl FnOnce(&mut crate::server::LabSession) -> TestResult,
) -> TestResult {
    let handle = crate::server::lookup_session_str(session)?;
    crate::server::with_session(&handle, || Err("session unavailable".into()), change)
}

fn open(selector: u64) -> TestResult<String> {
    let opened = parsed(&fortress_open_session(
        Some(false),
        Some(selector.to_string()),
        Some(vec![
            ("observe".to_owned(), "read_only".to_owned()),
            ("query".to_owned(), "read_only".to_owned()),
            ("plan".to_owned(), "read_only".to_owned()),
            ("control_clock".to_owned(), "reversible".to_owned()),
            ("designate".to_owned(), "guarded".to_owned()),
            ("checkpoint".to_owned(), "guarded".to_owned()),
            ("configure_production".to_owned(), "reversible".to_owned()),
            ("configure_labor".to_owned(), "reversible".to_owned()),
        ]),
        Some(60_000),
        Some(2_000),
        Some(8_192),
        Some(4 * 1024 * 1024),
        Some(16_384),
        None,
        Some("starter_fortress_resources".to_owned()),
        None,
        None,
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    let session = opened["session_id"]
        .as_str()
        .ok_or("session missing")?
        .to_owned();
    session_change(&session, |session| {
        let mut snapshot = session.adapter.snapshot().clone();
        write(
            &mut snapshot,
            starter::STOCK_LEDGER,
            effects::STOCK_PLANT_FIELD,
            Value::U64(0),
        )?;
        write(
            &mut snapshot,
            starter::STOCK_LEDGER,
            effects::STOCK_DRINK_FIELD,
            Value::U64(0),
        )?;
        snapshot.cursor = snapshot.cursor.checked_next().ok_or("cursor exhausted")?;
        snapshot.refresh_hash();
        session.adapter.inject_snapshot(snapshot)?;
        Ok(())
    })?;
    Ok(session)
}

fn public_drink(observed: &Json) -> TestResult<&Json> {
    observed["world_alerts"]
        .as_array()
        .ok_or("world alerts missing")?
        .iter()
        .find(|alert| alert["alert"] == "drink_supply")
        .ok_or_else(|| "drink alert missing".into())
}

#[test]
fn public_harvest_advice_uses_real_plan_risk_scope_checkpoint_and_budget_authority() -> TestResult {
    for case in 0..6 {
        let session = open(98_790 + case)?;
        session_change(&session, |session| {
            match case {
                0 => session
                    .grants
                    .retain(|grant| grant.capability != Capability::Plan),
                1 => {
                    session
                        .grants
                        .iter_mut()
                        .find(|grant| grant.capability == Capability::Designate)
                        .ok_or("designate missing")?
                        .max_risk = RiskTier::Reversible
                }
                2 => session
                    .grants
                    .retain(|grant| grant.capability != Capability::Checkpoint),
                3 => {
                    session
                        .grants
                        .iter_mut()
                        .find(|grant| grant.capability == Capability::Designate)
                        .ok_or("designate missing")?
                        .scope
                        .entity_ids = [EntityId::new(8_001)].into_iter().collect()
                }
                4 => {
                    session
                        .grants
                        .iter_mut()
                        .find(|grant| grant.capability == Capability::Designate)
                        .ok_or("designate missing")?
                        .expires_at_tick = Some(GameTick(0))
                }
                _ => session.budget.max_actions = 0,
            }
            Ok(())
        })?;
        let observed = parsed(&fortress_observe(Some(session)))?;
        assert_eq!(observed["ok"], true, "case {case}: {observed}");
        let alert = public_drink(&observed)?;
        assert_eq!(alert["remedy"]["available"], false, "case {case}: {alert}");
        assert_eq!(alert["remedy"]["risk"], "guarded");
        assert_eq!(alert["remedy"]["requires_checkpoint"], true);
        assert_eq!(alert["remedy"]["reversibility"], "not_compensable");
        assert!(
            observed["agent_turn"]["recommendations"]
                .as_array()
                .ok_or("recommendations missing")?
                .iter()
                .all(|next| next["recommendation_id"] != "remedy-drink_supply")
        );
    }
    Ok(())
}

#[test]
fn public_supply_recovery_preserves_the_original_goal_through_harvest_and_production() -> TestResult
{
    let _serial = crate::test_serial();
    let session = open(98_799)?;
    let observed = parsed(&fortress_observe(Some(session.clone())))?;
    let next = observed["agent_turn"]["recommendations"]
        .as_array()
        .ok_or("recommendations missing")?
        .iter()
        .find(|next| next["recommendation_id"] == "remedy-drink_supply")
        .ok_or("harvest recommendation missing")?;
    assert_eq!(next["risk"], "guarded");
    assert_eq!(next["requires_checkpoint"], true);
    assert_eq!(next["goal_scope"], "joint_supply_recovery");
    assert_eq!(next["availability_scope"], "current_reviewed_stage_only");
    assert_eq!(next["separate_stage_review_required"], true);
    let source: Json = parsed(
        next["arguments"]["blueprint"]
            .as_str()
            .ok_or("original production source missing")?,
    )?;
    assert_eq!(source["template"], "production");
    assert_eq!(source["quotas"], json!([{"item":"DRINK","minimum":28}]));
    let planned = parsed(&fortress_plan(
        Some(session.clone()),
        None,
        None,
        None,
        Some(source.to_string()),
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    assert_eq!(
        planned["production"]["resource_workflow"]["stage"],
        "acquire_resources"
    );
    let original = planned["plan_digest"]
        .as_str()
        .ok_or("digest missing")?
        .to_owned();
    let committed = parsed(&fortress_commit(Some(session.clone()), original.clone()))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let pending = parsed(&fortress_observe(Some(session.clone())))?;
    assert!(public_drink(&pending)?["remedy"].is_null());
    assert_eq!(
        public_drink(&pending)?["ingredient_readiness"]["available_plants"],
        0
    );
    let progressed = parsed(&fortress_wait_for_plan(
        Some(session.clone()),
        Some(2_000),
        Some(original.clone()),
    ))?;
    assert_eq!(progressed["ok"], true, "{progressed}");
    // Harvesting does not replace or satisfy the retained output-stock goal.
    assert!(
        progressed["objectives"]
            .as_array()
            .is_some_and(|goals| goals
                .iter()
                .any(|goal| goal["plan_digest"] == original && goal["predicate_truth"] == "false")),
        "{progressed}"
    );
    let production = parsed(&fortress_plan(
        Some(session.clone()),
        None,
        None,
        None,
        Some(json!({"template":"continue_goal", "plan_digest":original.clone()}).to_string()),
    ))?;
    assert_eq!(production["ok"], true, "{production}");
    assert_eq!(
        production["production"]["resource_workflow"]["stage"],
        "production"
    );
    assert_eq!(
        production["production"]["continuation"]["parent_plan_digest"],
        original
    );
    assert_eq!(
        production["production"]["resource_workflow"]["original_request"]["quotas"],
        source["quotas"]
    );
    let child = production["plan_digest"]
        .as_str()
        .ok_or("production digest missing")?
        .to_owned();
    let committed = parsed(&fortress_commit(Some(session.clone()), child.clone()))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let progressed = parsed(&fortress_wait_for_plan(
        Some(session.clone()),
        Some(2_000),
        Some(child.clone()),
    ))?;
    assert_eq!(progressed["ok"], true, "{progressed}");
    assert!(
        progressed["objectives"]
            .as_array()
            .is_some_and(|goals| goals.iter().any(|goal| goal["plan_digest"] == child
                && goal["predicate_truth"] == "true"
                && goal["status"] == "achieved")),
        "{progressed}"
    );
    session_change(&session, |session| {
        let snapshot = session.adapter.snapshot();
        let provisions = effects::provisions_report(snapshot)?.ok_or("provisions missing")?;
        let harvest = effects::harvest_report(snapshot)?.ok_or("harvest missing")?;
        let plants = harvest
            .sources
            .iter()
            .find(|source| source.source == PLANTS)
            .ok_or("plant source missing")?;
        assert_eq!(plants.initial, 40);
        assert_eq!(plants.remaining + plants.harvested, plants.initial);
        assert_eq!(
            plants.harvested,
            provisions.consumed_plants + provisions.available_plants
        );
        assert!(provisions.consumed_plants >= 6);
        assert_eq!(harvest.tasks.len(), 1);
        assert_eq!(harvest.tasks[0].status, effects::STATUS_COMPLETE);
        assert!(matches!(
            snapshot.graph.entities[&starter::STOCK_LEDGER].fields[effects::STOCK_DRINK_FIELD]
                .known_value(),
            Some(Value::U64(count)) if *count >= 28
        ));
        assert!(
            world_alerts(snapshot)
                .iter()
                .all(|alert| alert["alert"] != "drink_supply")
        );
        Ok(())
    })?;
    Ok(())
}

#[test]
fn public_joint_supply_recovery_finishes_both_original_quotas_from_one_input_project() -> TestResult
{
    let _serial = crate::test_serial();
    let session = open(98_800)?;
    session_change(&session, |session| {
        let mut snapshot = session.adapter.snapshot().clone();
        write(
            &mut snapshot,
            starter::STOCK_LEDGER,
            effects::STOCK_FOOD_FIELD,
            Value::U64(0),
        )?;
        snapshot.cursor = snapshot.cursor.checked_next().ok_or("cursor exhausted")?;
        snapshot.refresh_hash();
        session.adapter.inject_snapshot(snapshot)?;
        Ok(())
    })?;
    let before = session_change_snapshot(&session)?;
    let observed = parsed(&fortress_observe(Some(session.clone())))?;
    assert_eq!(observed["ok"], true, "{observed}");
    let alerts = observed["world_alerts"]
        .as_array()
        .ok_or("alerts missing")?;
    let food = alerts
        .iter()
        .find(|alert| alert["alert"] == "food_supply")
        .ok_or("food alert missing")?;
    assert_eq!(food["remedy_shared_with"], "drink_supply");
    assert!(food["remedy"].is_null());
    let recommendations = observed["agent_turn"]["recommendations"]
        .as_array()
        .ok_or("recommendations missing")?;
    assert_eq!(
        recommendations
            .iter()
            .filter(|next| {
                matches!(
                    next["recommendation_id"].as_str(),
                    Some("remedy-drink_supply" | "remedy-food_supply")
                )
            })
            .count(),
        1,
        "{observed}"
    );
    let next = recommendations
        .iter()
        .find(|next| next["recommendation_id"] == "remedy-drink_supply")
        .ok_or("joint recovery recommendation missing")?;
    let original: Json = parsed(
        next["arguments"]["blueprint"]
            .as_str()
            .ok_or("joint production source missing")?,
    )?;
    let quotas = json!([{"item":"DRINK","minimum":28},{"item":"FOOD","minimum":28}]);
    assert_eq!(original["quotas"], quotas);
    assert_eq!(session_change_snapshot(&session)?, before);

    let mut request = original;
    let mut parent: Option<String> = None;
    for expected_stage in ["acquire_resources", "production"] {
        let planned = parsed(&fortress_plan(
            Some(session.clone()),
            None,
            None,
            None,
            Some(request.to_string()),
        ))?;
        assert_eq!(planned["ok"], true, "{planned}");
        assert_eq!(
            planned["production"]["resource_workflow"]["stage"],
            expected_stage
        );
        assert_eq!(
            planned["production"]["resource_workflow"]["original_request"]["quotas"],
            quotas
        );
        if let Some(parent) = &parent {
            assert_eq!(
                planned["production"]["continuation"]["parent_plan_digest"],
                *parent
            );
        }
        let digest = planned["plan_digest"]
            .as_str()
            .ok_or("plan digest missing")?
            .to_owned();
        let committed = parsed(&fortress_commit(Some(session.clone()), digest.clone()))?;
        assert_eq!(committed["ok"], true, "{committed}");
        let waited = parsed(&fortress_wait_for_plan(
            Some(session.clone()),
            Some(2_000),
            Some(digest.clone()),
        ))?;
        assert_eq!(waited["ok"], true, "{waited}");
        if expected_stage == "production" {
            let goal = waited["objectives"]
                .as_array()
                .and_then(|goals| goals.iter().find(|goal| goal["plan_digest"] == digest))
                .ok_or("production objective missing")?;
            assert_eq!(goal["predicate_truth"], "true", "{goal}");
            assert_eq!(goal["status"], "achieved", "{goal}");
        }
        request = json!({"template":"continue_goal", "plan_digest":digest.clone()});
        parent = Some(digest);
    }
    let final_world = session_change_snapshot(&session)?;
    for field in [effects::STOCK_DRINK_FIELD, effects::STOCK_FOOD_FIELD] {
        assert!(matches!(
            final_world.graph.entities[&starter::STOCK_LEDGER].fields[field].known_value(),
            Some(Value::U64(count)) if *count >= 28
        ));
    }
    let harvest = effects::harvest_report(&final_world)?.ok_or("harvest missing")?;
    assert_eq!(
        harvest.tasks.len(),
        1,
        "one joint input acquisition must fund both outputs"
    );
    assert!(world_alerts(&final_world).iter().all(|alert| {
        !matches!(
            alert["alert"].as_str(),
            Some("drink_supply" | "food_supply")
        )
    }));
    Ok(())
}

fn session_change_snapshot(session: &str) -> TestResult<WorldSnapshot> {
    let handle = crate::server::lookup_session_str(session)?;
    crate::server::with_session_read(
        &handle,
        |error| Err(error.into()),
        |session| Ok(session.adapter.snapshot().clone()),
    )
}

#[test]
fn public_resource_recovery_advice_requires_query_and_the_complete_source_budget() -> TestResult {
    let _serial = crate::test_serial();
    for case in 0..3 {
        let session = open(98_801 + case)?;
        let before = session_change_snapshot(&session)?;
        session_change(&session, |session| {
            match case {
                0 => session
                    .grants
                    .retain(|grant| grant.capability != Capability::Query),
                1 => session.budget.max_bytes = 1,
                _ => session.budget.max_entities = 1,
            }
            Ok(())
        })?;
        let observed = parsed(&fortress_observe(Some(session.clone())))?;
        assert_eq!(observed["ok"], true, "case {case}: {observed}");
        let remedy = &public_drink(&observed)?["remedy"];
        assert_eq!(remedy["available"], false, "case {case}: {observed}");
        assert!(remedy["unavailable_reason"].is_string());
        assert!(
            observed["agent_turn"]["recommendations"]
                .as_array()
                .ok_or("recommendations missing")?
                .iter()
                .all(|next| next["recommendation_id"] != "remedy-drink_supply")
        );
        assert_eq!(session_change_snapshot(&session)?, before);
    }
    Ok(())
}
