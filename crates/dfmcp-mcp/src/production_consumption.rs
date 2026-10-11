//! Fixed consumption-aware quota compiler for the reference laboratory.
//!
//! The source records the compiler generation. Historical requests without it
//! retain their exact stock-only program. New candidates reserve enough output
//! for known population consumption through their complete sealed horizon,
//! then recompute service and dependency time until that reserve is stable.
//! These are bounded predictions, never observed stock or completion evidence.

use super::*;

const MAX_CONSUMPTION_ROUNDS: u32 = 64;

struct ConsumptionModel {
    living: u64,
    metabolism_ticks: u64,
}

fn unavailable(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::PreconditionsFailed, message)
}

fn exhausted(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

impl ConsumptionModel {
    fn capture(snapshot: &WorldSnapshot) -> Result<Self> {
        if snapshot.graph.entities.len() > MAX_PRODUCTION_SETUP_ENTITIES {
            return Err(exhausted(
                "consumption planning exceeds the 65,536-entity observation bound",
            ));
        }
        let mut living = 0u64;
        for unit in snapshot.graph.entities.values() {
            if unit.kind != EntityKind::Unit {
                continue;
            }
            match production_fact(unit, "alive", snapshot.tick) {
                Some(Value::Bool(true)) => living += 1,
                Some(Value::Bool(false)) => {}
                _ => {
                    return Err(unavailable(format!(
                        "consumption planning requires source-qualified alive status for unit {}; an unknown population cannot establish reserve demand",
                        unit.id.get(),
                    )));
                }
            }
        }
        let ledger = effects::stock_ledger(snapshot)
            .and_then(|id| snapshot.graph.entities.get(&id))
            .ok_or_else(|| {
                unavailable("consumption planning requires the observed stock ledger")
            })?;
        let Some(Value::U64(metabolism_ticks)) =
            production_fact(ledger, effects::METABOLISM_TICKS_FIELD, snapshot.tick)
        else {
            return Err(unavailable(
                "consumption planning requires source-qualified metabolism_ticks; world tick alone does not establish the next drink or meal boundary",
            ));
        };
        Ok(Self {
            living,
            metabolism_ticks: *metabolism_ticks,
        })
    }

    fn allowance(&self, item: &str, horizon: u64) -> Result<u64> {
        let interval = match item {
            "DRINK" => effects::DRINK_INTERVAL_TICKS,
            "FOOD" => effects::FOOD_INTERVAL_TICKS,
            _ => {
                return Err(invalid(
                    "consumption planning escaped its registered recipes",
                ));
            }
        };
        let after = self.metabolism_ticks.checked_add(horizon).ok_or_else(|| {
            exhausted("consumption planning would overflow the observed metabolism clock")
        })?;
        // Reference effects settle same-tick production before metabolism.
        // Counting that boundary conservatively reserves its entire demand.
        let rounds = after / interval - self.metabolism_ticks / interval;
        rounds.checked_mul(self.living).ok_or_else(|| {
            exhausted("consumption planning would overflow the bounded population demand")
        })
    }
}

pub(super) fn compile(
    request: &ProductionRequest,
    snapshot: &WorldSnapshot,
) -> Result<ProductionCompilation> {
    let mut targets = request.quotas.clone();
    // This first candidate preserves the ordinary "nothing to produce" result
    // when every original quota is already met. There is no implicit perpetual
    // maintenance objective, background work or newly invented horizon.
    let mut candidate = request.compile_targets(snapshot, &targets, true)?;
    let model = ConsumptionModel::capture(snapshot)?;
    for round in 1..=MAX_CONSUMPTION_ROUNDS {
        let horizon = candidate.workload.horizon_ticks(&candidate.actions)?;
        let mut next = targets.clone();
        for (item, minimum) in &request.quotas {
            let target = model
                .allowance(item, horizon)?
                .checked_add(u64::from(*minimum))
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| {
                    exhausted("consumption reserve exceeds the compiler's exact u32 quantity bound")
                })?;
            // Retain an earlier conservative target if adding a second job
            // changes the selected staffing/dependency program. Never cycle
            // between programs or silently reduce an already budgeted reserve.
            let previous = next
                .get_mut(item)
                .ok_or_else(|| invalid("consumption planning lost an original quota"))?;
            *previous = (*previous).max(target);
        }
        if next == targets {
            candidate.consumption_horizon = Some(horizon);
            annotate(
                request,
                snapshot,
                &model,
                &targets,
                horizon,
                round,
                &mut candidate,
            )?;
            return Ok(candidate);
        }
        targets = next;
        if round < MAX_CONSUMPTION_ROUNDS {
            candidate = request.compile_targets(snapshot, &targets, true)?;
        }
    }
    Err(exhausted(
        "consumption-aware planning did not establish a stable reserve within 64 bounded rounds; this conservative model cannot establish this plan, not that every possible production strategy is infeasible",
    ))
}

fn annotate(
    request: &ProductionRequest,
    snapshot: &WorldSnapshot,
    model: &ConsumptionModel,
    targets: &BTreeMap<String, u32>,
    horizon: u64,
    rounds: u32,
    candidate: &mut ProductionCompilation,
) -> Result<()> {
    let rows = candidate.analysis["requirements"]
        .as_array_mut()
        .ok_or_else(|| invalid("consumption planning lost its resource report"))?;
    for row in rows {
        let item = row["item"]
            .as_str()
            .ok_or_else(|| invalid("consumption planning lost its resource identity"))?
            .to_owned();
        let minimum = request
            .quotas
            .get(&item)
            .ok_or_else(|| invalid("consumption report contains an unrequested resource"))?;
        let target = targets
            .get(&item)
            .ok_or_else(|| invalid("consumption report lost its planning stock target"))?;
        row["minimum_stock"] = json!(minimum);
        row["planning_stock_target"] = json!(target);
        row["consumption_allowance"] = json!(target - minimum);
        row["predicted_consumption_through_horizon"] = json!(model.allowance(&item, horizon)?);
    }
    let end = snapshot
        .tick
        .checked_add(horizon)
        .ok_or_else(|| exhausted("consumption planning would overflow the game-tick horizon"))?;
    candidate.analysis["consumption"] = json!({
        "epistemic_state": "predicted",
        "planner": "consumption_aware_v1",
        "method": "monotone_reserve_through_complete_sealed_obligation_horizon",
        "source_state_hash": snapshot.state_hash.to_hex(),
        "source_tick": snapshot.tick.0,
        "observed_living_population": model.living,
        "observed_metabolism_ticks": model.metabolism_ticks,
        "drink_interval_ticks": effects::DRINK_INTERVAL_TICKS,
        "food_interval_ticks": effects::FOOD_INTERVAL_TICKS,
        "planning_horizon_ticks": horizon,
        "planning_horizon_tick": end.0,
        "fixed_point_rounds": rounds,
        "maximum_rounds": MAX_CONSUMPTION_ROUNDS,
        "future_output_counted_as_stock": false,
        "original_terminal_quotas_preserved": true,
        "completion_guaranteed": false,
        "assumptions": "reference recipe yields and observed population/capacity; queued service and prerequisite deadlines are included; current workers/workshops remain available, foreground observations release dependencies and verify work by their sealed deadlines, and no new competing work or population arrives; scheduled changes and interference can invalidate this prediction",
    });
    Ok(())
}
