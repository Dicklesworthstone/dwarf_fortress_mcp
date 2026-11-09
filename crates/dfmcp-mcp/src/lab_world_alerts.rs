//! Bounded, source-qualified laboratory needs and threat triage.
//!
//! An alert is a derivation from the reference world, never a new observation.
//! Unknown inputs remain explicit and cannot authorize a guessed remedy.

use super::{
    MAX_PRODUCTION_SETUP_ENTITIES, civilian_alert, laboratory_provisions_view, production_fact,
    production_military_rank, ready_production_workshop,
};
use dfmcp_intent::effects;
use dfmcp_world::{EntityKind, EntityRecord, Value, WorldSnapshot};
use serde_json::{Value as Json, json};

const MAX_THREAT_ALERTS: usize = 16;

struct Population<'a> {
    living: Vec<&'a EntityRecord>,
    unknown: usize,
}

impl<'a> Population<'a> {
    fn census(snapshot: &'a WorldSnapshot) -> Self {
        let mut population = Self {
            living: Vec::new(),
            unknown: 0,
        };
        for unit in snapshot
            .graph
            .entities
            .values()
            .filter(|entity| entity.kind.as_str() == EntityKind::Unit.as_str())
        {
            if unit.kind != EntityKind::Unit {
                // A textual alias is not a participant in the registered
                // reference effects and cannot certify a population quota.
                population.unknown += 1;
                continue;
            }
            match production_fact(unit, "alive", snapshot.tick) {
                Some(Value::Bool(true)) => population.living.push(unit),
                Some(Value::Bool(false)) => {}
                _ => population.unknown += 1,
            }
        }
        population
    }

    fn complete(&self) -> bool {
        self.unknown == 0
    }
}

pub(crate) fn world_alerts(snapshot: &WorldSnapshot) -> Vec<Json> {
    if snapshot.graph.entities.len() > MAX_PRODUCTION_SETUP_ENTITIES {
        return vec![qualified(
            snapshot,
            json!({
                "alert":"world_alert_coverage", "severity":"high", "epistemic_state":"unknown",
                "finding":"needs and threats cannot be established within the complete entity census bound",
                "entity_limit":MAX_PRODUCTION_SETUP_ENTITIES, "remedy":null,
            }),
        )];
    }
    let population = Population::census(snapshot);
    let mut alerts = supply_alerts(snapshot, &population);
    // Threats and civilian safety remain meaningful without a stock ledger or
    // a complete living roster. Economy availability must never hide them.
    alerts.extend(threat_alerts(snapshot, &population));
    alerts.extend(civilian_alert(snapshot, &population.living));
    alerts
        .into_iter()
        .map(|alert| qualified(snapshot, alert))
        .collect()
}

fn qualified(snapshot: &WorldSnapshot, mut alert: Json) -> Json {
    if alert.get("epistemic_state").is_none() {
        alert["epistemic_state"] = json!("certified_derived");
    }
    alert["evidence_scope"] = json!("laboratory_reference_world");
    alert["source_anchor"] = crate::observation_projection::anchor_json(&snapshot.anchor());
    alert
}

fn supply_alerts(snapshot: &WorldSnapshot, population: &Population<'_>) -> Vec<Json> {
    let Some(ledger) =
        effects::stock_ledger(snapshot).and_then(|id| snapshot.graph.entities.get(&id))
    else {
        return Vec::new();
    };
    if population.living.is_empty() && population.complete() {
        return Vec::new();
    }
    let living = population.living.len() as u64;
    let provisions = effects::provisions_report(snapshot);
    let mut alerts = Vec::new();
    for (stock, interval, need, deprived, noun, token, job) in [
        (
            effects::STOCK_DRINK_FIELD,
            effects::DRINK_INTERVAL_TICKS,
            effects::NEED_DRINK_FIELD,
            "thirsty",
            "drink",
            "DRINK",
            "BREW_DRINK",
        ),
        (
            effects::STOCK_FOOD_FIELD,
            effects::FOOD_INTERVAL_TICKS,
            effects::NEED_FOOD_FIELD,
            "hungry",
            "food",
            "FOOD",
            "PREPARE_MEAL",
        ),
    ] {
        let held = match production_fact(ledger, stock, snapshot.tick) {
            Some(Value::U64(held)) => Some(*held),
            _ => None,
        };
        let mut known_deprived = 0usize;
        let mut unknown_needs = population.unknown;
        for unit in &population.living {
            match production_fact(unit, need, snapshot.tick) {
                Some(Value::Text(state)) if state == deprived => known_deprived += 1,
                Some(Value::Text(state)) if state == "satisfied" => {}
                _ => unknown_needs += 1,
            }
        }
        let target = population.complete().then_some(living.saturating_mul(4));
        let ticks_left = if population.complete() && living > 0 {
            held.and_then(|held| (held / living).checked_mul(interval))
        } else {
            None
        };
        let shortfall = held.zip(target).filter(|(held, target)| held < target);
        let (severity, epistemic, finding) = if known_deprived > 0 {
            (
                "critical",
                "certified_derived",
                format!(
                    "at least {known_deprived} dwarves are {deprived}; {}",
                    held.map_or_else(
                        || format!("current {noun} stock is unknown"),
                        |held| format!("current {noun} stock is {held}")
                    ),
                ),
            )
        } else if held == Some(0) {
            (
                "critical",
                "certified_derived",
                format!("the fortress has no {noun} left"),
            )
        } else if let Some((held, _)) = shortfall {
            (
                "high",
                "certified_derived",
                format!(
                    "{noun} has fewer than four consumption rounds remaining ({held} units for {living} dwarves)",
                ),
            )
        } else if held.is_none() || !population.complete() {
            (
                "high",
                "unknown",
                format!(
                    "{noun} sufficiency is unknown: current stock or the complete living roster is not established"
                ),
            )
        } else {
            continue;
        };
        let (blocker, remedy) = if let Some((_, target)) = shortfall {
            production_remedy(snapshot, population, token, job, target)
        } else {
            (None, Json::Null)
        };
        let mut alert = json!({
            "alert":format!("{noun}_supply"), "severity":severity,
            "epistemic_state":epistemic, "finding":finding, "stock":held,
            "ticks_until_exhausted":ticks_left,
            "estimate_basis":"whole consumption rounds at the laboratory calibration",
            "living_units":population.complete().then_some(living),
            "known_living_units":living, "unknown_living_units":population.unknown,
            "deprived_units":(unknown_needs == 0).then_some(known_deprived),
            "known_deprived_units":known_deprived, "unknown_need_units":unknown_needs,
            "production_blocked_by":blocker, "remedy":remedy,
        });
        if let Some((held, target)) = shortfall {
            // Necessary ingredient screening does not replace quota planning,
            // which still establishes the complete reserve and capacity horizon.
            let minimum_batches = (target - held).div_ceil(5);
            if let Some((reason, readiness)) = laboratory_provisions_view::alert_input_blocker(
                snapshot,
                &provisions,
                minimum_batches,
            ) {
                alert["finding"] = json!(format!("{finding}; {reason}"));
                alert["capacity_blocked_by"] = json!(blocker);
                alert["production_blocked_by"] = json!(reason);
                alert["ingredient_readiness"] = readiness;
                alert["remedy"] = Json::Null;
            }
        }
        alerts.push(alert);
    }
    alerts
}

fn production_remedy(
    snapshot: &WorldSnapshot,
    population: &Population<'_>,
    token: &str,
    job: &str,
    target: u64,
) -> (Option<String>, Json) {
    let Some((workshop, labor)) = effects::work_order_requirements(job) else {
        return (
            Some("production requirements are unavailable".to_owned()),
            Json::Null,
        );
    };
    let ready = ready_production_workshop(snapshot, workshop).is_some();
    let field = format!("{}{labor}", effects::LABOR_FIELD_PREFIX);
    let has_worker = population
        .living
        .iter()
        .any(|unit| production_fact(unit, &field, snapshot.tick) == Some(&Value::Bool(true)));
    let blocker = match (ready, has_worker) {
        (true, true) => None,
        (false, _) => Some(format!("completed {workshop} is not established")),
        (true, false) => Some(format!(
            "living unit with the {labor} labor enabled is not established"
        )),
    };
    let remedy = if ready && has_worker {
        json!({
            "tool":"fortress.plan", "requires":"configure_production",
            "arguments":{"blueprint":json!({
                "template":"production", "quotas":[{"item":token,"minimum":target}],
            }).to_string()},
        })
    } else if ready {
        // A reversible labor write requires the actual prior boolean. Missing
        // or unqualified fields are not interchangeable with disabled labor.
        match population
            .living
            .iter()
            .filter(|unit| {
                production_fact(unit, &field, snapshot.tick) == Some(&Value::Bool(false))
            })
            .min_by_key(|unit| (production_military_rank(unit, snapshot.tick), unit.id))
        {
            Some(worker) => json!({
                "tool":"fortress.plan", "requires":"configure_labor",
                "arguments":{"actions":json!([{"action":{
                    "kind":"set_labor", "units":[worker.id.to_string()],
                    "labor":labor, "enabled":true,
                }}]).to_string()},
                "then":"plan production once the labor is assigned",
            }),
            None => Json::Null,
        }
    } else {
        Json::Null
    };
    (blocker, remedy)
}

fn threat_alerts(snapshot: &WorldSnapshot, population: &Population<'_>) -> Vec<Json> {
    let squad = snapshot
        .graph
        .entities
        .values()
        .find(|entity| entity.kind == EntityKind::Squad)
        .map(|entity| entity.id);
    let mut soldiers = 0usize;
    let mut unknown_defenders = population.unknown;
    let mut recruits = Vec::new();
    for unit in &population.living {
        match production_fact(unit, effects::SQUAD_FIELD, snapshot.tick) {
            Some(Value::Entity(id))
                if snapshot
                    .graph
                    .entities
                    .get(id)
                    .is_some_and(|entity| entity.kind == EntityKind::Squad) =>
            {
                soldiers += 1
            }
            Some(Value::Null) => recruits.push(unit.id.to_string()),
            None if !unit.fields.contains_key(effects::SQUAD_FIELD) => {
                // The registered legacy laboratory model represents an
                // unassigned squad by field absence; present unknown differs.
                recruits.push(unit.id.to_string());
            }
            _ => unknown_defenders += 1,
        }
    }
    recruits.truncate(4usize.saturating_sub(soldiers));
    let remedy = squad
        .filter(|_| unknown_defenders == 0 && !recruits.is_empty())
        .map(|squad| {
            json!({
                "tool":"fortress.plan", "requires":"configure_military",
                "arguments":{"actions":json!([{"action":{
                    "kind":"assign_squad", "units":recruits, "squad":squad.to_string(),
                }}]).to_string()},
            })
        });
    let defense = if unknown_defenders == 0 {
        format!("{soldiers} dwarves are in a squad")
    } else {
        format!("at least {soldiers} dwarves are in a squad; complete defense strength is unknown")
    };
    let mut selected = std::collections::BTreeMap::new();
    let mut active = 0usize;
    let mut unknown_threats = 0usize;
    let mut known_hostiles_with_unknown_status = 0usize;
    for hostile in snapshot
        .graph
        .entities
        .values()
        .filter(|entity| entity.kind.as_str() == EntityKind::Creature.as_str())
    {
        if hostile.kind != EntityKind::Creature {
            unknown_threats += 1;
            continue;
        }
        match production_fact(hostile, effects::HOSTILE_FIELD, snapshot.tick) {
            Some(Value::Bool(false)) => continue,
            Some(Value::Bool(true)) => {}
            _ => {
                unknown_threats += 1;
                continue;
            }
        }
        let attacking = match production_fact(hostile, effects::THREAT_STATUS_FIELD, snapshot.tick)
        {
            Some(Value::Text(status)) if status == effects::THREAT_SLAIN => continue,
            Some(Value::Text(status)) if status == effects::THREAT_ATTACKING => true,
            Some(Value::Text(status)) if status == effects::THREAT_APPROACHING => false,
            _ => {
                unknown_threats += 1;
                known_hostiles_with_unknown_status += 1;
                continue;
            }
        };
        let arrives = match production_fact(hostile, effects::ARRIVES_AT_FIELD, snapshot.tick) {
            Some(Value::U64(tick)) => Some(*tick),
            _ => None,
        };
        let threat = if attacking {
            format!("{} is attacking", hostile.label)
        } else if let Some(arrives) = arrives {
            format!("{} arrives at tick {arrives}", hostile.label)
        } else {
            format!("{} is approaching; arrival time is unknown", hostile.label)
        };
        active += 1;
        selected.insert(
            (!attacking, hostile.id),
            json!({
                "alert":"hostile", "severity":if attacking { "critical" } else { "high" },
                "finding":format!("{threat}; {defense}"), "subject":hostile.id.to_string(),
                "arrives_at_tick":arrives,
                "soldiers":(unknown_defenders == 0).then_some(soldiers),
                "known_soldiers":soldiers, "unknown_defender_units":unknown_defenders,
                "remedy":remedy,
            }),
        );
        if selected.len() > MAX_THREAT_ALERTS {
            selected.pop_last();
        }
    }
    let omitted = active.saturating_sub(selected.len());
    let mut alerts: Vec<Json> = selected.into_values().collect();
    if omitted > 0 || unknown_threats > 0 {
        alerts.push(json!({
            "alert":"threat_coverage", "severity":"high", "epistemic_state":"unknown",
            "finding":format!(
                "threat coverage is incomplete: {unknown_threats} creature threat states are unknown ({known_hostiles_with_unknown_status} are known hostile); {omitted} additional active threats are omitted"
            ),
            "unknown_threats":unknown_threats,
            "known_hostiles_with_unknown_status":known_hostiles_with_unknown_status,
            "omitted_active_threats":omitted, "remedy":null,
        }));
    }
    alerts
}

#[cfg(test)]
#[path = "lab_world_alerts_tests.rs"]
mod tests;
