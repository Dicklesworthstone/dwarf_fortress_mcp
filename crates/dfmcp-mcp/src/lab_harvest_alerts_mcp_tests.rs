//! Public agent recommendations acquire finite inputs before reserve production.

use crate::agent_facade::{
    fortress_commit, fortress_observe, fortress_open_session, fortress_plan, fortress_wait,
};
use crate::lab_world::{starter, world_alerts};
use dfmcp_adapter::GameAdapter;
use dfmcp_core::{Capability, Digest32, EntityId, GameTick, RiskTier};
use dfmcp_intent::effects;
use dfmcp_world::{Fact, FactSource, Value, WorldSnapshot};
use serde_json::Value as Json;

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
            ("plan".to_owned(), "read_only".to_owned()),
            ("control_clock".to_owned(), "reversible".to_owned()),
            ("designate".to_owned(), "guarded".to_owned()),
            ("checkpoint".to_owned(), "guarded".to_owned()),
            ("configure_production".to_owned(), "reversible".to_owned()),
        ]),
        None,
        Some(2_000),
        None,
        None,
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
fn public_harvest_recommendation_closes_the_depleted_input_to_drink_production_loop() -> TestResult
{
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
    let planned = parsed(&fortress_plan(
        Some(session.clone()),
        None,
        None,
        Some(
            next["arguments"]["actions"]
                .as_str()
                .ok_or("actions missing")?
                .to_owned(),
        ),
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let committed = parsed(&fortress_commit(
        Some(session.clone()),
        planned["plan_digest"]
            .as_str()
            .ok_or("digest missing")?
            .to_owned(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let pending = parsed(&fortress_observe(Some(session.clone())))?;
    assert!(public_drink(&pending)?["remedy"].is_null());
    assert_eq!(
        public_drink(&pending)?["ingredient_readiness"]["available_plants"],
        0
    );
    let progressed = parsed(&fortress_wait(Some(session.clone()), Some(300)))?;
    assert_eq!(progressed["ok"], true, "{progressed}");
    let ready = parsed(&fortress_observe(Some(session.clone())))?;
    let remedy = &public_drink(&ready)?["remedy"];
    assert_eq!(remedy["available"], true, "{ready}");
    assert_eq!(remedy["requires"], "configure_production");
    let production = parsed(&fortress_plan(
        Some(session.clone()),
        None,
        None,
        None,
        Some(
            remedy["arguments"]["blueprint"]
                .as_str()
                .ok_or("production source missing")?
                .to_owned(),
        ),
    ))?;
    assert_eq!(production["ok"], true, "{production}");
    let committed = parsed(&fortress_commit(
        Some(session.clone()),
        production["plan_digest"]
            .as_str()
            .ok_or("production digest missing")?
            .to_owned(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let progressed = parsed(&fortress_wait(Some(session.clone()), Some(300)))?;
    assert_eq!(progressed["ok"], true, "{progressed}");
    session_change(&session, |session| {
        let snapshot = session.adapter.snapshot();
        let provisions = effects::provisions_report(snapshot)?.ok_or("provisions missing")?;
        assert_eq!(provisions.available_plants, 0);
        assert_eq!(provisions.consumed_plants, 6);
        let harvest = effects::harvest_report(snapshot)?.ok_or("harvest missing")?;
        let plants = harvest
            .sources
            .iter()
            .find(|source| source.source == PLANTS)
            .ok_or("plant source missing")?;
        assert_eq!(plants.remaining, 34);
        assert_eq!(plants.harvested, 6);
        assert_eq!(harvest.tasks.len(), 1);
        assert_eq!(harvest.tasks[0].status, effects::STATUS_COMPLETE);
        assert_eq!(
            snapshot.graph.entities[&starter::STOCK_LEDGER].fields[effects::STOCK_DRINK_FIELD]
                .known_value(),
            Some(&Value::U64(30))
        );
        assert!(
            world_alerts(snapshot)
                .iter()
                .all(|alert| alert["alert"] != "drink_supply")
        );
        Ok(())
    })?;
    Ok(())
}
