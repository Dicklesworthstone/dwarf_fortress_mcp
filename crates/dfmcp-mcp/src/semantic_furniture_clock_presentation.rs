//! Bounded clock identities and actionable offers for the original construction goal.
use super::{anchor, digest, obligation};
use crate::semantic_furniture_runtime::clock::MAX_RUNS;
use dfmcp_core::WorkBudget;
use serde_json::{Value, json};

fn state(value: &Value) -> Option<&str> {
    value.as_str().filter(|s| {
        matches!(
            *s,
            "intent"
                | "prepared"
                | "dispatch_started"
                | "tracking"
                | "terminal"
                | "cancel_requested"
                | "cancelled_before_dispatch"
        )
    })
}
fn run(value: &Value) -> Option<Value> {
    let key = value["key"].as_str().filter(|s| {
        !s.is_empty()
            && s.len() <= 128
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    })?;
    let native = &value["native"];
    Some(
        json!({"review_seal":digest(&value["review_seal"])?,"key":key,
        "plan_digest":digest(&value["plan_digest"])?,"state":state(&value["state"]),
        "before_tick":value["before_tick"].as_u64(),
        "source_generation":value["source_generation"].as_u64(),
        "source_sequence":value["source_sequence"].as_u64(),
        "game_ticks":value["game_ticks"].as_u64().filter(|n|(1..=1200).contains(n)),
        "wall_millis":value["wall_millis"].as_u64().filter(|n|(1..=60000).contains(n)),
        "pending":value["pending"].as_bool(),"unresolved":value["unresolved"].as_bool(),
        "native":{"phase":native["phase"].as_str().filter(|s|matches!(*s,
            "prepared"|"running"|"stopping"|"stopped"|"refused"|"source_lost")),
            "reason":native["reason"].as_str().filter(|s|matches!(*s,
                "none"|"tick_limit"|"wall_limit"|"cancelled"|"external_pause"|"native_failure"
                |"clock_regression"|"source_changed"|"shutdown"|"stale")),
            "receipt":digest(&native["receipt"]),
            "unpause_attempted":native["unpause_attempted"].as_bool(),
            "historical_pause_verified":native["historical_pause_verified"].as_bool(),
            "observed_tick":native["observed_tick"].as_u64(),
            "observed_tick_overshoot":native["observed_tick_overshoot"].as_u64()},
        "current_pause_proven":false,"original_goal_proven":false}),
    )
}
pub(super) fn identity(view: &Value) -> Option<Value> {
    let clock = &view["clock"];
    Some(
        json!({"journal_id":digest(&clock["journal_id"])?,"head":digest(&clock["head"])?,
        "mode":clock["mode"].as_str().filter(|s|matches!(*s,"control"|"reconcile"|"offline")),
        "target_digest":digest(&clock["target_digest"]),
        "original_paired_seal":digest(&clock["original_paired_seal"]),
        "run_count":clock["run_count"].as_u64().filter(|n|*n<=MAX_RUNS as u64),
        "pending":clock["pending"].as_bool(),"unresolved":clock["unresolved"].as_bool(),
        "may_be_running":clock["may_be_running"].as_bool(),
        "needs_reconciliation":clock["needs_reconciliation"].as_bool(),
        "high_tick":clock["high_tick"].as_u64(),
        "high_source_sequence":clock["high_source_sequence"].as_u64(),
        "active_run":run(&clock["active_run"]),
        "source_identity_semantics":"source_incarnation_only_not_named_fortress_lineage",
        "current_pause_proven":false,"original_goal_proven":false}),
    )
}
pub(super) fn uncertain(clock: Option<&Value>, result: &Value, view: &Value) -> bool {
    result["clock_effect_may_have_started"] == true
        || view["clock_effect_may_have_started"] == true
        || clock.is_some_and(|c| {
            c["unresolved"] == true
                || c["may_be_running"] == true
                || matches!(
                    state(&c["active_run"]["state"]),
                    Some("dispatch_started" | "tracking" | "cancel_requested")
                )
        })
}

pub(super) struct Context<'a> {
    pub view: &'a Value,
    pub clock: Option<&'a Value>,
    pub goal: Option<&'a Value>,
    pub mode: Option<&'a str>,
    pub budget: Option<WorkBudget>,
    pub verified: bool,
    pub closed: bool,
    pub placed: bool,
    pub complete: bool,
    pub uncertain: bool,
}
pub(super) struct Offers {
    pub authority: bool,
    pub propose: bool,
    pub commit: bool,
    pub cancel: bool,
    pub allowed_ticks: Option<u64>,
    pub next_ticks: Option<u64>,
}
pub(super) fn offers(c: Context<'_>) -> Offers {
    let view = c.view;
    let active = c
        .clock
        .and_then(|v| v.get("active_run"))
        .filter(|v| v.is_object());
    let authority = c.verified
        && !c.closed
        && c.mode == Some("control")
        && view["development_clock_enabled"] == true
        && c.clock.is_some_and(|v| v["mode"] == "control");
    let linked = c.clock.is_some_and(|v| {
        digest(&v["target_digest"]).is_some()
            && digest(&v["original_paired_seal"]).is_some()
            && v["original_paired_seal"] == view["review"]["paired_seal"]
            && v["journal_id"] == view["recovery"]["clock_journal_id"]
    });
    let deadline = obligation(view).and_then(|v| v["deadline_tick"].as_u64());
    let observed_tick = view["anchor"]["tick"].as_u64();
    let known_tick = observed_tick.map(|tick| {
        tick.max(
            c.clock
                .and_then(|v| v["high_tick"].as_u64())
                .unwrap_or(tick),
        )
    });
    let allowed_ticks = deadline
        .zip(known_tick)
        .and_then(|(end, tick)| end.checked_sub(tick))
        .zip(c.budget)
        .map(|(remaining, b)| remaining.min(b.max_game_ticks).min(1200))
        .filter(|n| *n > 0);
    let next_ticks = allowed_ticks
        .zip(obligation(view).and_then(|v| v["poll_interval_ticks"].as_u64()))
        .map(|(limit, cadence)| limit.min(cadence))
        .filter(|n| *n > 0);
    // Construction may legitimately be incomplete. Only a qualified, viable
    // receipt-linked sample permits proposing time; unknown or blocked work does not.
    let viable = c.goal.is_some_and(|g| {
        g["receipt_linked"] == true
            && matches!(
                g["construction_condition"].as_str(),
                Some("pending" | "condition_met")
            )
            && matches!(g["current_truth"].as_str(), Some("true" | "false"))
            && g["failure_predicate"] != "true"
            && g["semantic_completion_proven"] == false
            && anchor(&g["anchor"]).is_some()
            && g["anchor"] == view["anchor"]
            && observed_tick == known_tick
    });
    let eligible = authority && linked && c.placed && viable && !c.complete && !c.uncertain;
    let propose = eligible
        && next_ticks.is_some()
        && c.clock.is_some_and(|v| {
            v["pending"] == false && v["run_count"].as_u64().is_some_and(|n| n < MAX_RUNS as u64)
        });
    let commit = eligible
        && active.is_some_and(|r| {
            r["state"] == "prepared"
                && r["before_tick"].as_u64() == observed_tick
                && r["before_tick"].as_u64() == known_tick
                && r["source_sequence"]
                    .as_u64()
                    .zip(c.clock.and_then(|v| v["high_source_sequence"].as_u64()))
                    .is_some_and(|(sequence, floor)| sequence >= floor)
                && r["game_ticks"]
                    .as_u64()
                    .zip(c.budget)
                    .is_some_and(|(ticks, b)| ticks <= b.max_game_ticks)
                && r["before_tick"]
                    .as_u64()
                    .zip(r["game_ticks"].as_u64())
                    .and_then(|(tick, ticks)| tick.checked_add(ticks))
                    .zip(deadline)
                    .is_some_and(|(end, deadline)| end <= deadline)
        });
    Offers {
        authority,
        propose,
        commit,
        cancel: authority && linked && active.is_some(),
        allowed_ticks,
        next_ticks,
    }
}
