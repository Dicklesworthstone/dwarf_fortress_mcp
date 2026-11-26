//! Bounded, source-qualified laboratory needs and threat triage.
//!
//! An alert is a derivation from the reference world, never a new observation.
//! Unknown inputs remain explicit and cannot authorize a guessed remedy.

use super::{
    MAX_PRODUCTION_SETUP_ENTITIES, civilian_alert, laboratory_provisions_view, production_fact,
    ready_production_workshop,
};
use dfmcp_intent::{HarvestResource, effects};
use dfmcp_world::{EntityKind, EntityRecord, Value, WorldSnapshot};
use serde_json::{Value as Json, json};

const MAX_THREAT_ALERTS: usize = 16;

struct SupplyGoal {
    alert_index: usize,
    item: &'static str,
    minimum: u64,
    minimum_batches: u64,
    assign_labor: Option<bool>,
}

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

/// Elapsed game ticks until stock is empty and until at least one dwarf goes
/// without. The simulator consumes on the ledger's periodic boundaries. An
/// exact last full round empties the ledger without leaving anyone unserved.
fn supply_timing(
    held: Option<u64>,
    living: u64,
    metabolism: Option<u64>,
    interval: u64,
    world_tick: u64,
) -> (Option<u64>, Option<u64>) {
    let Some(held) = held.filter(|_| living > 0) else {
        return (None, None);
    };
    let ticks_to_round = |round: u64| -> Option<u64> {
        if round == 0 {
            return Some(0);
        }
        let clock = metabolism?;
        let next = interval - clock % interval;
        let elapsed = (round - 1).checked_mul(interval)?.checked_add(next)?;
        clock.checked_add(elapsed)?;
        world_tick.checked_add(elapsed)?;
        Some(elapsed)
    };
    (
        ticks_to_round(held.div_ceil(living)),
        (held / living).checked_add(1).and_then(ticks_to_round),
    )
}

fn supply_alerts(snapshot: &WorldSnapshot, population: &Population<'_>) -> Vec<Json> {
    let ledger = effects::stock_ledger(snapshot).and_then(|id| snapshot.graph.entities.get(&id));
    let metabolism = ledger.and_then(|ledger| {
        match production_fact(ledger, effects::METABOLISM_TICKS_FIELD, snapshot.tick) {
            Some(Value::U64(ticks)) => Some(*ticks),
            _ => None,
        }
    });
    if population.living.is_empty() && population.complete() {
        return Vec::new();
    }
    let living = population.living.len() as u64;
    let provisions = effects::provisions_report(snapshot);
    let mut alerts = Vec::new();
    let mut goals = Vec::new();
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
        let held = match ledger.and_then(|ledger| production_fact(ledger, stock, snapshot.tick)) {
            Some(Value::U64(held)) => Some(*held),
            _ => None,
        };
        let mut known_deprived = 0usize;
        let mut unknown_needs = population.unknown;
        let mut unavailable_reports = 0usize;
        for unit in &population.living {
            match production_fact(unit, need, snapshot.tick) {
                Some(Value::Text(state)) if state == deprived => known_deprived += 1,
                Some(Value::Text(state)) if state == "satisfied" => {}
                _ => {
                    unknown_needs += 1;
                    unavailable_reports += usize::from(unit.fields.contains_key(need));
                }
            }
        }
        let target = population.complete().then_some(living.saturating_mul(4));
        // Worlds without a metabolism produce no need reports. Preserve any
        // actual citizen report even if its stock ledger has disappeared.
        if ledger.is_none() && known_deprived == 0 && unavailable_reports == 0 {
            continue;
        }
        let (ticks_left, first_unserved) = if population.complete() {
            supply_timing(held, living, metabolism, interval, snapshot.tick.0)
        } else {
            (None, None)
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
        } else if unavailable_reports > 0 {
            alerts.push(json!({
                "alert":format!("{noun}_need_coverage"), "severity":"medium",
                "epistemic_state":"unknown",
                "finding":format!("{unavailable_reports} living dwarves have unavailable {noun} need reports"),
                "unknown_need_units":unknown_needs, "unavailable_need_reports":unavailable_reports,
                "known_deprived_units":known_deprived, "stock":held, "remedy":null,
            }));
            continue;
        } else {
            continue;
        };
        let finding = match first_unserved {
            Some(ticks) => format!(
                "{finding}. With the current population and no replenishment, some dwarves would go without {noun} in {ticks} game ticks."
            ),
            None => finding,
        };
        let (blocker, assign_labor) = if shortfall.is_some() {
            production_capacity(snapshot, population, job)
        } else {
            (None, None)
        };
        let alert = json!({
            "alert":format!("{noun}_supply"), "severity":severity,
            "epistemic_state":epistemic, "finding":finding, "stock":held,
            "ticks_until_exhausted":ticks_left,
            "ticks_until_first_unserved_round":first_unserved,
            "estimate_basis":"prediction using the observed metabolism phase with constant population and no stock changes except consumption",
            "living_units":population.complete().then_some(living),
            "known_living_units":living, "unknown_living_units":population.unknown,
            "deprived_units":(unknown_needs == 0).then_some(known_deprived),
            "known_deprived_units":known_deprived, "unknown_need_units":unknown_needs,
            "production_blocked_by":blocker, "remedy":null,
        });
        if let Some((held, target)) = shortfall {
            goals.push(SupplyGoal {
                alert_index: alerts.len(),
                item: token,
                minimum: target,
                minimum_batches: (target - held).div_ceil(5),
                assign_labor,
            });
        }
        alerts.push(alert);
    }
    supply_recovery(snapshot, population, &provisions, &goals, &mut alerts);
    alerts
}

/// Establish whether an existing workshop and qualified labor can support a
/// production proposal. A disabled worker permits a reviewed enable; missing
/// labor is not interchangeable with false and supplies no compensation.
fn production_capacity(
    snapshot: &WorldSnapshot,
    population: &Population<'_>,
    job: &str,
) -> (Option<String>, Option<bool>) {
    let Some((workshop, labor)) = effects::work_order_requirements(job) else {
        return (
            Some("production requirements are unavailable".to_owned()),
            None,
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
    let assign_labor = if ready && has_worker {
        Some(false)
    } else if ready
        && population
            .living
            .iter()
            .any(|unit| production_fact(unit, &field, snapshot.tick) == Some(&Value::Bool(false)))
    {
        Some(true)
    } else {
        None
    };
    (blocker, assign_labor)
}

/// Keep every known deficient output in one source. Independent food and drink
/// advice can double-spend the same ingredients, while a raw harvest or labor
/// action discards the goal that made that prerequisite useful.
fn supply_recovery(
    snapshot: &WorldSnapshot,
    population: &Population<'_>,
    provisions: &dfmcp_core::Result<Option<effects::ProvisionsReport>>,
    goals: &[SupplyGoal],
    alerts: &mut [Json],
) {
    let Some(first) = goals.first() else {
        return;
    };
    // The complete census bounds each quota to 4 * 65,536 and there are only
    // two registered supplies. This necessary screen remains distinct from
    // the compiler's full gathering/production consumption-reserve budget.
    let minimum_batches = goals.iter().map(|goal| goal.minimum_batches).sum();
    let input =
        laboratory_provisions_view::alert_input_blocker(snapshot, provisions, minimum_batches);
    if let Some(input) = &input {
        for goal in goals {
            let alert = &mut alerts[goal.alert_index];
            let finding = alert["finding"].as_str().unwrap_or_default();
            alert["finding"] = json!(format!("{finding}; {}", input.reason));
            alert["capacity_blocked_by"] = alert["production_blocked_by"].clone();
            alert["production_blocked_by"] = json!(input.reason);
            alert["ingredient_readiness"] = input.readiness.clone();
        }
    }
    // There is no evidenced workshop site to synthesize here. A joint source
    // must be able to pursue both outputs, rather than drop the blocked one.
    if goals.iter().any(|goal| goal.assign_labor.is_none()) {
        return;
    }
    let acquisition = if input.is_some() {
        let Some(assign_labor) =
            acquisition_capacity(snapshot, population, provisions, minimum_batches)
        else {
            return;
        };
        Some(assign_labor)
    } else {
        None
    };
    let mut blueprint = json!({
        "template":"production",
        "quotas":goals.iter().map(|goal| json!({
            "item":goal.item, "minimum":goal.minimum,
        })).collect::<Vec<_>>(),
    });
    if goals.iter().any(|goal| goal.assign_labor == Some(true)) {
        // Existing workshops need recipe staffing only. Do not import the
        // shared-workforce policy into legacy worlds or invent setup sites.
        blueprint["prerequisites"] = json!({"assign_labor":true});
    }
    if let Some(assign_labor) = acquisition {
        blueprint["acquire_resources"] = json!({"assign_labor":assign_labor});
    }
    let shared_with = alerts[first.alert_index]["alert"].clone();
    let goal_alerts: Vec<_> = goals
        .iter()
        .map(|goal| alerts[goal.alert_index]["alert"].clone())
        .collect();
    alerts[first.alert_index]["remedy"] = json!({
        "tool":"fortress.plan", "requires":"configure_production",
        "arguments":{"blueprint":blueprint.to_string()},
        "goal_scope":"joint_supply_recovery", "goal_alerts":goal_alerts,
        "separate_stage_review_required":acquisition.is_some(),
        "then":if acquisition.is_some() {
            "observe the committed stage until its work is quiet, then review continue_goal for the retained production plan; harvested inputs alone do not complete the original output quotas"
        } else {
            "observe the committed production and the unchanged original output quotas"
        },
    });
    for goal in goals.iter().skip(1) {
        alerts[goal.alert_index]["remedy_shared_with"] = shared_with.clone();
    }
}

/// Necessary acquisition eligibility, without choosing or dispatching actions.
/// The normal resource-aware compiler still establishes complete finite supply,
/// worker selection, action bounds and reserves before advice is available.
fn acquisition_capacity(
    snapshot: &WorldSnapshot,
    population: &Population<'_>,
    provisions: &dfmcp_core::Result<Option<effects::ProvisionsReport>>,
    minimum_batches: u64,
) -> Option<bool> {
    let provisions = provisions.as_ref().ok()?.as_ref()?;
    let harvest = effects::harvest_report(snapshot).ok()??;
    if harvest.ledger != provisions.ledger
        || harvest.ledger_generation != provisions.ledger_generation
        || provisions.remaining_demand_plants != 0
        || harvest.tasks.iter().any(|task| {
            task.resource == HarvestResource::Plant && task.status == effects::STATUS_ACTIVE
        })
    {
        return None;
    }
    let shortage = minimum_batches.checked_sub(provisions.available_plants)?;
    let available = harvest
        .sources
        .iter()
        .filter(|source| source.resource == HarvestResource::Plant)
        .try_fold(0u64, |total, source| {
            total.checked_add(source.remaining.checked_sub(source.active_demand)?)
        })?;
    if shortage == 0 || available < shortage {
        return None;
    }
    let field = format!(
        "{}{}",
        effects::LABOR_FIELD_PREFIX,
        HarvestResource::Plant.labor()
    );
    if population
        .living
        .iter()
        .any(|unit| production_fact(unit, &field, snapshot.tick) == Some(&Value::Bool(true)))
    {
        Some(false)
    } else if population
        .living
        .iter()
        .any(|unit| production_fact(unit, &field, snapshot.tick) == Some(&Value::Bool(false)))
    {
        Some(true)
    } else {
        None
    }
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
