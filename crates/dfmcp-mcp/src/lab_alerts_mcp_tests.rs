//! Exercise alert evidence through the real public observation and Agent Turn.

use super::*;
use dfmcp_core::{Digest32, EntityId};
use dfmcp_world::{Fact, FactPresence, FactSource, Value as WorldValue, WorldSnapshot};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn parsed(raw: &str) -> std::result::Result<Value, Box<dyn std::error::Error>> {
    Ok(serde_json::from_str(raw)?)
}

fn open(
    selector: &str,
    scenario: &str,
    plan: bool,
) -> std::result::Result<String, Box<dyn std::error::Error>> {
    open_with_military(selector, scenario, plan, "reversible", false)
}

fn open_with_military(
    selector: &str,
    scenario: &str,
    plan: bool,
    military_risk: &str,
    checkpoint: bool,
) -> std::result::Result<String, Box<dyn std::error::Error>> {
    let mut capabilities = vec![
        ("observe".to_owned(), "read_only".to_owned()),
        ("control_clock".to_owned(), "reversible".to_owned()),
        ("configure_production".to_owned(), "reversible".to_owned()),
        ("configure_labor".to_owned(), "reversible".to_owned()),
        ("configure_military".to_owned(), military_risk.to_owned()),
    ];
    if plan {
        capabilities.push(("plan".to_owned(), "reversible".to_owned()));
    }
    if checkpoint {
        capabilities.push(("checkpoint".to_owned(), "guarded".to_owned()));
    }
    let opened = parsed(&fortress_open_session(
        Some(true),
        Some(selector.to_owned()),
        Some(capabilities),
        None,
        Some(2_000),
        None,
        None,
        Some(8_192),
        None,
        Some(scenario.to_owned()),
        None,
        None,
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    Ok(opened["session_id"]
        .as_str()
        .ok_or("session missing")?
        .to_owned())
}

fn edit(session: &str, change: impl FnOnce(&mut WorldSnapshot) -> TestResult) -> TestResult {
    let handle = crate::server::lookup_session_str(session)?;
    crate::server::with_session(
        &handle,
        || Err("session unavailable".into()),
        |session| {
            let mut snapshot = session.adapter.snapshot().clone();
            change(&mut snapshot)?;
            snapshot.cursor = snapshot.cursor.checked_next().ok_or("cursor exhausted")?;
            snapshot.refresh_hash();
            session.adapter.inject_snapshot(snapshot)?;
            Ok(())
        },
    )
}

fn write(snapshot: &mut WorldSnapshot, id: EntityId, field: &str, value: WorldValue) -> TestResult {
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
                snapshot.tick,
                FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                Digest32::ZERO,
            ),
        );
    Ok(())
}

#[test]
fn public_observation_keeps_unqualified_stock_unknown() -> TestResult {
    for (index, redacted) in [false, true].into_iter().enumerate() {
        let session = open(&(98_701 + index).to_string(), "starter_fortress", true)?;
        edit(&session, |snapshot| {
            let ledger = snapshot
                .graph
                .entities
                .get_mut(&EntityId::new(5001))
                .ok_or("ledger missing")?;
            let fact = if redacted {
                Fact::with_presence(
                    FactPresence::Redacted("withheld".to_owned()),
                    snapshot.tick,
                    FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                    Digest32::ZERO,
                )
            } else {
                Fact::known(
                    WorldValue::U64(0),
                    snapshot.tick,
                    FactSource::AgentAssertion("empty-stock claim".to_owned()),
                    Digest32::ZERO,
                )
            };
            ledger.fields.insert("stock.drink".to_owned(), fact);
            Ok(())
        })?;
        let observed = parsed(&fortress_observe(Some(session)))?;
        assert_eq!(observed["ok"], true, "{observed}");
        let alert = observed["world_alerts"]
            .as_array()
            .ok_or("alerts missing")?
            .iter()
            .find(|alert| alert["alert"] == "drink_supply")
            .ok_or("drink alert missing")?;
        assert!(alert["stock"].is_null(), "{alert}");
        assert!(alert["remedy"].is_null());
        let attention = observed["agent_turn"]["attention"]
            .as_array()
            .ok_or("attention missing")?
            .iter()
            .find(|item| item["attention_id"] == "world-drink_supply")
            .ok_or("need attention missing")?;
        assert_eq!(attention["confidence"]["epistemic_state"], "unknown");
        assert!(attention["confidence"]["value"].is_null());
        assert_eq!(
            attention["evidence"][0]["source_anchor"]["state_hash"],
            observed["anchor"]["state_hash"]
        );
        assert!(
            !observed["agent_turn"]["recommendations"]
                .as_array()
                .ok_or("recommendations missing")?
                .iter()
                .any(|recommendation| recommendation["recommendation_id"] == "remedy-drink_supply")
        );
    }
    Ok(())
}

#[test]
fn public_alert_does_not_offer_labor_for_an_unready_workshop() -> TestResult {
    let session = open("98703", "starter_fortress", true)?;
    edit(&session, |snapshot| {
        write(
            snapshot,
            EntityId::new(5001),
            "stock.drink",
            WorldValue::U64(0),
        )?;
        write(
            snapshot,
            EntityId::new(7001),
            "construction_stage",
            WorldValue::Text("planned".to_owned()),
        )
    })?;
    let observed = parsed(&fortress_observe(Some(session)))?;
    assert_eq!(observed["ok"], true, "{observed}");
    let alert = observed["world_alerts"]
        .as_array()
        .ok_or("alerts missing")?
        .iter()
        .find(|alert| alert["alert"] == "drink_supply")
        .ok_or("drink alert missing")?;
    assert_eq!(alert["severity"], "critical");
    assert!(
        alert["production_blocked_by"]
            .as_str()
            .is_some_and(|why| why.contains("workshop:Still"))
    );
    assert!(alert["remedy"].is_null());
    let attention = observed["agent_turn"]["attention"]
        .as_array()
        .ok_or("attention missing")?
        .iter()
        .find(|item| item["attention_id"] == "world-drink_supply")
        .ok_or("need attention missing")?;
    assert_eq!(
        attention["confidence"]["epistemic_state"],
        "certified_derived"
    );
    assert!(attention["confidence"]["value"].is_null());
    Ok(())
}

#[test]
fn public_stockless_hostiles_have_distinct_certified_selection_identities() -> TestResult {
    let session = open("98704", "besieged_fortress", true)?;
    edit(&session, |snapshot| {
        snapshot.graph.entities.remove(&EntityId::new(5001));
        let mut raider = snapshot
            .graph
            .entities
            .get(&EntityId::new(6001))
            .ok_or("raider missing")?
            .clone();
        raider.id = EntityId::new(6002);
        snapshot.graph.entities.insert(raider.id, raider);
        let mut burrow = snapshot
            .graph
            .entities
            .get(&EntityId::new(3001))
            .ok_or("burrow missing")?
            .clone();
        burrow.id = EntityId::new(3002);
        snapshot.graph.entities.insert(burrow.id, burrow);
        Ok(())
    })?;
    let observed = parsed(&fortress_observe(Some(session)))?;
    assert_eq!(observed["ok"], true, "{observed}");
    let attention = observed["agent_turn"]["attention"]
        .as_array()
        .ok_or("attention missing")?;
    for id in [
        "world-hostile-6001",
        "world-hostile-6002",
        "world-civilian_lockdown-3001",
        "world-civilian_lockdown-3002",
    ] {
        let item = attention
            .iter()
            .find(|item| item["attention_id"] == id)
            .ok_or("hostile hidden")?;
        assert_eq!(item["confidence"]["epistemic_state"], "certified_derived");
        assert!(item["confidence"]["value"].is_null());
        assert_eq!(
            item["evidence"][0]["source_anchor"]["state_hash"],
            observed["anchor"]["state_hash"]
        );
    }
    assert_eq!(
        observed["agent_turn"]["coverage"]["attention_selection"]["certified"],
        true
    );
    Ok(())
}

#[test]
fn public_remedy_recommendation_requires_plan_authority() -> TestResult {
    let session = open("98705", "starter_fortress", false)?;
    edit(&session, |snapshot| {
        write(
            snapshot,
            EntityId::new(5001),
            "stock.drink",
            WorldValue::U64(0),
        )
    })?;
    let observed = parsed(&fortress_observe(Some(session)))?;
    assert_eq!(observed["ok"], true, "{observed}");
    let alert = observed["world_alerts"]
        .as_array()
        .ok_or("alerts missing")?
        .iter()
        .find(|alert| alert["alert"] == "drink_supply")
        .ok_or("drink alert missing")?;
    assert_eq!(alert["remedy"]["requires"], "configure_production");
    assert_eq!(alert["remedy"]["available"], false);
    assert!(
        !observed["agent_turn"]["recommendations"]
            .as_array()
            .ok_or("recommendations missing")?
            .iter()
            .any(|recommendation| recommendation["recommendation_id"] == "remedy-drink_supply")
    );
    Ok(())
}

#[test]
fn public_military_remedies_require_the_actual_guarded_risk_and_checkpoint() -> TestResult {
    for (index, risk, checkpoint) in [(0, "reversible", true), (1, "guarded", false)] {
        let session = open_with_military(
            &(98_706 + index).to_string(),
            "besieged_fortress",
            true,
            risk,
            checkpoint,
        )?;
        let observed = parsed(&fortress_observe(Some(session)))?;
        assert_eq!(observed["ok"], true, "{observed}");
        let alert = observed["world_alerts"]
            .as_array()
            .ok_or("alerts missing")?
            .iter()
            .find(|alert| alert["alert"] == "hostile")
            .ok_or("hostile missing")?;
        assert_eq!(alert["remedy"]["available"], false, "{alert}");
        assert_eq!(alert["remedy"]["risk"], "guarded");
        assert_eq!(alert["remedy"]["reversibility"], "not_compensable");
        assert_eq!(alert["remedy"]["requires_checkpoint"], true);
        assert!(alert["remedy"]["unavailable_reason"].is_string());
        assert!(
            !observed["agent_turn"]["recommendations"]
                .as_array()
                .ok_or("recommendations missing")?
                .iter()
                .any(|recommendation| recommendation["recommendation_id"] == "remedy-hostile")
        );
    }
    Ok(())
}

#[test]
fn public_qualified_military_remedy_plans_and_commits_with_its_checkpoint() -> TestResult {
    let session = open_with_military("98708", "besieged_fortress", true, "guarded", true)?;
    let observed = parsed(&fortress_observe(Some(session.clone())))?;
    assert_eq!(observed["ok"], true, "{observed}");
    let next = observed["agent_turn"]["recommendations"]
        .as_array()
        .ok_or("recommendations missing")?
        .iter()
        .find(|next| next["recommendation_id"] == "remedy-hostile")
        .ok_or("qualified military remedy not recommended")?;
    assert_eq!(next["risk"], "guarded");
    assert_eq!(next["reversibility"], "not_compensable");
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
    assert_eq!(planned["requires_checkpoint"], true);
    let committed = parsed(&fortress_commit(
        Some(session.clone()),
        planned["plan_digest"]
            .as_str()
            .ok_or("digest missing")?
            .to_owned(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let observed = parsed(&fortress_observe(Some(session)))?;
    let alert = observed["world_alerts"]
        .as_array()
        .ok_or("alerts missing")?
        .iter()
        .find(|alert| alert["alert"] == "hostile")
        .ok_or("hostile missing")?;
    assert_eq!(alert["soldiers"], 4);
    assert!(alert["remedy"].is_null());
    Ok(())
}

#[test]
fn public_wait_pulse_retains_unknown_alert_evidence() -> TestResult {
    let session = open("98709", "starter_fortress", true)?;
    edit(&session, |snapshot| {
        let stock = snapshot
            .graph
            .entities
            .get_mut(&EntityId::new(5001))
            .ok_or("ledger missing")?
            .fields
            .get_mut("stock.drink")
            .ok_or("stock missing")?;
        stock.source = FactSource::AgentAssertion("unqualified stock".to_owned());
        Ok(())
    })?;
    let waited = parsed(&fortress_wait(Some(session), Some(1)))?;
    assert_eq!(waited["ok"], true, "{waited}");
    assert_eq!(waited["agent_turn"]["profile"], "pulse");
    let item = waited["agent_turn"]["attention"]
        .as_array()
        .ok_or("attention missing")?
        .iter()
        .find(|item| item["attention_id"] == "world-drink_supply")
        .ok_or("drink attention missing")?;
    assert_eq!(item["confidence"]["epistemic_state"], "unknown");
    assert!(item["confidence"]["value"].is_null());
    assert_eq!(item["evidence"][0]["scope"], "laboratory_reference_world");
    assert!(item["evidence"][0]["source_anchor"]["state_hash"].is_string());
    Ok(())
}

#[path = "lab_harvest_alerts_mcp_tests.rs"]
mod harvesting;
