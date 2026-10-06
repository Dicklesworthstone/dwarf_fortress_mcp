//! Deterministic agent-policy evaluation in the laboratory.
//!
//! A policy plays a scenario for a bounded amount of game time through the
//! same eleven tools an agent uses, and the outcome is scored from canonical
//! observation: dwarves alive, dwarf-rounds spent thirsty or hungry, hostiles
//! slain, objectives achieved, plans committed and tool calls spent. The
//! laboratory is deterministic, so the same scenario, policy and horizon
//! always produce the same report, which makes policies comparable and turns
//! the recommendation system itself into something that can regress.
//!
//! Scores measure laboratory calibration, not Dwarf Fortress play.

use serde_json::{Value, json};

use crate::agent_facade as f;

/// Policies the harness can run.
pub const POLICIES: [&str; 2] = ["idle", "follow_recommendations"];
/// Game ticks advanced per policy step.
pub const STEP_TICKS: u64 = 100;
/// Longest horizon one evaluation may run.
pub const MAX_HORIZON_TICKS: u64 = 200_000;

const CAPABILITIES: [(&str, &str); 11] = [
    ("observe", "read_only"),
    ("query", "read_only"),
    ("plan", "reversible"),
    ("control_clock", "reversible"),
    ("checkpoint", "guarded"),
    ("doctor", "read_only"),
    ("designate", "guarded"),
    ("construct", "guarded"),
    ("configure_labor", "reversible"),
    ("configure_production", "reversible"),
    ("configure_military", "guarded"),
];

fn parsed(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or(Value::Null)
}

struct Tally {
    calls: u64,
    plans_committed: u64,
    plans_refused: u64,
    deprived_dwarf_steps: u64,
}

fn units(session: &str, tally: &mut Tally) -> Vec<Value> {
    tally.calls += 1;
    parsed(&f::fortress_query(
        Some(session.to_owned()),
        Some(r#"{"mode":"entities","kind":"unit","limit":100}"#.to_owned()),
    ))["rows"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// Act on the turn's top recommendation when it is a concrete plan or
/// commit; anything else (observe, wait, doctor) is what the loop does anyway.
fn follow(session: &str, turn: &Value, tally: &mut Tally) {
    let Some(top) = turn["agent_turn"]["recommendations"].get(0) else {
        return;
    };
    let digest = match top["tool"].as_str() {
        Some("fortress.plan") => {
            let Some(actions) = top["arguments"]["actions"].as_str() else {
                return;
            };
            tally.calls += 1;
            let planned = parsed(&f::fortress_plan(
                Some(session.to_owned()),
                None,
                None,
                Some(actions.to_owned()),
                None,
            ));
            match planned["plan_digest"].as_str() {
                Some(digest) => digest.to_owned(),
                None => {
                    tally.plans_refused += 1;
                    return;
                }
            }
        }
        Some("fortress.commit") => match top["arguments"]["plan_digest"].as_str() {
            Some(digest) => digest.to_owned(),
            None => return,
        },
        _ => return,
    };
    tally.calls += 1;
    let committed = parsed(&f::fortress_commit(Some(session.to_owned()), digest));
    if committed["ok"] == true {
        tally.plans_committed += 1;
    } else {
        tally.plans_refused += 1;
    }
}

/// Run `policy` on `scenario` for `horizon_ticks` of game time and score it.
#[must_use]
pub fn evaluate(scenario: &str, policy: &str, horizon_ticks: u64) -> Value {
    if !POLICIES.contains(&policy) {
        return json!({"ok": false, "error": format!("unknown policy {policy:?}; expected one of {POLICIES:?}")});
    }
    if horizon_ticks == 0 || horizon_ticks > MAX_HORIZON_TICKS {
        return json!({"ok": false, "error": format!("horizon must be 1..={MAX_HORIZON_TICKS} game ticks")});
    }
    let opened = parsed(&f::fortress_open_session(
        Some(false),
        Some("1".to_owned()),
        Some(
            CAPABILITIES
                .iter()
                .map(|(c, r)| ((*c).to_owned(), (*r).to_owned()))
                .collect(),
        ),
        None,
        Some(MAX_HORIZON_TICKS),
        None,
        None,
        Some(16_384),
        Some(1_024),
        Some(scenario.to_owned()),
        None,
        None,
    ));
    let Some(session) = opened["session_id"].as_str().map(str::to_owned) else {
        return json!({"ok": false, "error": "scenario could not be opened", "detail": opened});
    };
    let mut tally = Tally {
        calls: 1,
        plans_committed: 0,
        plans_refused: 0,
        deprived_dwarf_steps: 0,
    };
    let mut elapsed = 0u64;
    let mut last = Value::Null;
    while elapsed < horizon_ticks {
        let step = STEP_TICKS.min(horizon_ticks - elapsed);
        tally.calls += 1;
        last = parsed(&f::fortress_wait(Some(session.clone()), Some(step)));
        elapsed += step;
        if policy == "follow_recommendations" {
            follow(&session, &last, &mut tally);
        }
        tally.deprived_dwarf_steps += units(&session, &mut tally)
            .iter()
            .filter(|u| {
                u["fields"]["alive"] != false
                    && (u["fields"]["need.drink"] == "thirsty"
                        || u["fields"]["need.food"] == "hungry")
            })
            .count() as u64;
    }
    let roster = units(&session, &mut tally);
    let alive = roster
        .iter()
        .filter(|u| u["fields"]["alive"] != false)
        .count();
    tally.calls += 1;
    let creatures = parsed(&f::fortress_query(
        Some(session.clone()),
        Some(r#"{"mode":"entities","kind":"creature","limit":100}"#.to_owned()),
    ));
    let hostiles: Vec<&Value> = creatures["rows"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter(|c| c["fields"]["hostile"] == true)
                .collect()
        })
        .unwrap_or_default();
    let slain = hostiles
        .iter()
        .filter(|c| c["fields"]["threat_status"] == "slain")
        .count();
    let objectives = last["agent_turn"]["briefing"]["objective_status"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let achieved = objectives
        .iter()
        .filter(|o| o["status"] == "achieved")
        .count();
    json!({
        "ok": true,
        "schema": "dfmcp.lab-evaluation/1",
        "scenario": scenario,
        "policy": policy,
        "horizon_ticks": horizon_ticks,
        "final_anchor": last["agent_turn"]["anchor"],
        "score": {
            "dwarves_alive": alive,
            "dwarves_total": roster.len(),
            "deprived_dwarf_steps": tally.deprived_dwarf_steps,
            "hostiles_slain": slain,
            "hostiles_total": hostiles.len(),
            "objectives_achieved": achieved,
            "objectives_total": objectives.len(),
        },
        "cost": {
            "tool_calls": tally.calls,
            "plans_committed": tally.plans_committed,
            "plans_refused": tally.plans_refused,
            "step_ticks": STEP_TICKS,
        },
        "note": "laboratory calibration; scores compare policies, not Dwarf Fortress play",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn following_recommendations_beats_idling_and_is_deterministic() {
        let idle = evaluate("besieged_fortress", "idle", 12_000);
        let follow = evaluate("besieged_fortress", "follow_recommendations", 12_000);
        assert_eq!(idle["ok"], true, "{idle}");
        assert_eq!(follow["ok"], true, "{follow}");
        let alive = |r: &Value| r["score"]["dwarves_alive"].as_u64().unwrap_or(0);
        assert!(alive(&follow) > alive(&idle), "{idle}\n{follow}");
        assert_eq!(follow["score"]["hostiles_slain"], 1);
        assert_eq!(follow["score"]["dwarves_alive"], 7, "{follow}");
        assert!(
            follow["score"]["deprived_dwarf_steps"].as_u64()
                <= idle["score"]["deprived_dwarf_steps"].as_u64(),
            "{idle}\n{follow}"
        );
        assert!(follow["score"]["objectives_achieved"].as_u64() >= Some(2));
        // In a peaceful fortress the difference is supply: idling runs dry.
        let idle = evaluate("starter_fortress", "idle", 12_000);
        let follow = evaluate("starter_fortress", "follow_recommendations", 12_000);
        assert!(
            idle["score"]["deprived_dwarf_steps"].as_u64() > Some(0),
            "{idle}"
        );
        assert_eq!(follow["score"]["deprived_dwarf_steps"], 0, "{follow}");
        // Same scenario, policy and horizon: the same report, anchor included.
        let again = evaluate("starter_fortress", "follow_recommendations", 12_000);
        assert_eq!(
            again["final_anchor"]["state_hash"],
            follow["final_anchor"]["state_hash"]
        );
        assert_eq!(again["score"], follow["score"]);
        assert_eq!(evaluate("starter_fortress", "nonsense", 100)["ok"], false);
    }
}
