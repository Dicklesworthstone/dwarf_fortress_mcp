//! Public stock-watch time control stays attached to one acknowledged parent.
//! Advice prepares a review; historical clock custody never proves current stock.
use super::super::runtime::stock_clock::{
    ClockCheckpoint, WatchClockTarget, original_digest, production_source_digest,
};
use crate::agent_turn::recommendation;
use crate::live_production_runtime::ProjectStockGoal;
use dfmcp_core::{Digest32, SessionId};
use serde_json::{Value, json};

const VIEW_SCHEMA: &str = "dfmcp.native-resource-stock-clock-view/1";
const GUIDANCE_SCHEMA: &str = "dfmcp.resource-stock-clock-guidance/1";

fn digest(value: &Value) -> Option<Digest32> {
    let raw = value.as_str()?;
    Digest32::from_hex(raw).filter(|value| *value != Digest32::ZERO && value.to_hex() == raw)
}

struct Binding<'a> {
    clock: &'a Value,
    target: WatchClockTarget,
    floor: u64,
}

/// Match the complete immutable parent and production custody, including every
/// portfolio term. A hash copied from a guidance row is not this binding.
fn binding<'a>(result: &'a Value, reference: &Value) -> Option<Binding<'a>> {
    let production = result["stage"] == "production"
        || (result["stage"] == "cancelled"
            && reference["retained_stage"]["cancelled_from"] == "production");
    if result["ok"] != true
        || !production
        || result["stage"] != reference["retained_stage"]["stage"]
        || !matches!(result["mode"].as_str(), Some("control" | "recover"))
        || result["project_custody_currently_verified"] != true
        || result["session_closed"] == true
        || reference["controller_fenced"] != false
        || super::publication_pending(reference)
        || result["original"] != reference["original"]
        || result["original"]["stock_watch_clock"] != true
    {
        return None;
    }
    let clock = &result["stock_clock"];
    let target = WatchClockTarget::from_json(&clock["target"]).ok()?;
    let checkpoint = &reference["retained_stage"]["checkpoint"];
    let retained = ClockCheckpoint::from_json(&checkpoint["stock_clock"]).ok()?;
    let original = original_digest(&reference["original"]).ok()?;
    let production = production_source_digest(&checkpoint["production"]).ok()?;
    let goal: ProjectStockGoal = serde_json::from_value(result["original"]["goal"].clone()).ok()?;
    target
        .matches_original(
            digest(&reference["parent_acknowledged"]["journal_id"])?,
            original,
            &goal,
            production,
        )
        .ok()?;
    let floor = clock["high_tick"]
        .as_u64()?
        .max(reference["parent_acknowledged"]["authority_tick_floor"].as_u64()?)
        .max(reference["authority_tick_floor"].as_u64().unwrap_or(0))
        .max(
            result["child"]["observation_tick_floor"]
                .as_u64()
                .unwrap_or(0),
        );
    let valid = clock["schema"] == VIEW_SCHEMA
        && clock["target_scope"] == "original_resource_project_stock_goal"
        && clock["source"] == "run/1.13"
        && clock["target_digest"] == target.digest().to_hex()
        && clock["original_paired_seal"] == target.original_paired_seal().to_hex()
        && clock["target"] == retained.target().to_json()
        && clock["journal_id"] == retained.journal_id().to_hex()
        && clock["head"] == retained.head().to_hex()
        && clock["high_tick"]
            .as_u64()
            .is_some_and(|tick| tick >= retained.high_tick())
        && clock["high_source_sequence"]
            .as_u64()
            .is_some_and(|sequence| sequence >= retained.high_sequence())
        && clock["sample_history_durable"] == false
        && clock["recovery_restores_current_proof"] == false;
    valid.then_some(Binding {
        clock,
        target,
        floor,
    })
}

fn active_watch(result: &Value) -> Option<Value> {
    let child = &result["child"];
    let raw = &child["no_work_stock_goal"];
    if result["mode"] != "control"
        || result["stage"] != "production"
        || result["project_cancelled"] == true
        || child["ok"] != true
        || child["mode"] != "control"
        || child["local_parent_only"] == true
        || !child["original_plan"].is_null()
        || !super::no_work_matches_root(raw, child, &result["original"]["goal"])
    {
        return None;
    }
    let watch = super::stock_watch::summary(child, Some(&child["anchor"]), true);
    (watch.is_object() && watch["cancelled"] == false && watch["source_fenced"] == false)
        .then_some(watch)
}

fn unsettled(clock: &Value) -> bool {
    clock["schema"] != VIEW_SCHEMA
        || clock["inspection_required"] != false
        || clock["pending"] != false
        || clock["unresolved"] != false
        || clock["may_be_running"] != false
        || clock["advancement_fenced"] == true
}

/// A pending preparation is still unsettled time-control work. It may expose
/// its own current confirmation, but cannot lend a fresh goal sample.
pub(super) fn has_custody(result: &Value, reference: &Value) -> bool {
    !result["stock_clock"].is_null()
        || !reference["stock_clock"].is_null()
        || !reference["retained_stage"]["checkpoint"]["stock_clock"].is_null()
}

pub(super) fn permits_goal_sample(result: &Value, reference: &Value) -> bool {
    !has_custody(result, reference)
        || binding(result, reference).is_some_and(|bound| !unsettled(bound.clock))
}

pub(super) fn review(result: &Value, reference: &Value) -> Option<Value> {
    let bound = binding(result, reference)?;
    active_watch(result)?;
    let clock = bound.clock;
    let active = &clock["active_run"];
    let before = active["before_tick"].as_u64()?;
    let ticks = active["game_ticks"].as_u64()?;
    let wall = active["wall_millis"].as_u64()?;
    let seal = digest(&active["review_seal"])?;
    let valid = clock["mode"] == "control"
        && clock["review_available"] == true
        && clock["inspection_required"] == false
        && clock["pending"] == true
        && clock["unresolved"] == false
        && clock["may_be_running"] == false
        && clock["advancement_fenced"] != true
        && active["state"] == "prepared"
        && active["unresolved"] != true
        && digest(&active["plan_digest"]).is_some()
        && active["key"]
            .as_str()
            .is_some_and(|key| key.starts_with("stock-watch-clock-") && key.len() == 82)
        && (1..=1200).contains(&ticks)
        && (1..=60_000).contains(&wall)
        && before >= bound.target.original_tick()
        && before >= bound.floor
        && before
            .checked_add(ticks)
            .is_some_and(|end| end <= bound.target.deadline_tick());
    valid.then(|| json!(seal.to_hex()))
}

struct Guidance<'a> {
    raw: &'a Value,
    watch: Value,
    bound: Binding<'a>,
    source_tick: u64,
    sample_withheld: bool,
}

fn guidance<'a>(result: &'a Value, reference: &Value) -> Option<Guidance<'a>> {
    let bound = binding(result, reference)?;
    let watch = active_watch(result)?;
    let raw = &result["stock_clock_guidance"];
    let anchor = &result["child"]["anchor"];
    let source_tick = anchor["tick"].as_u64()?;
    let current_witness =
        raw["goal_witness"] == watch["goal_witness"] && digest(&raw["goal_witness"]).is_some();
    let blocker_without_sample = raw["goal_witness"].is_null()
        && raw["plan_request"].is_null()
        && raw["blocker"].is_object();
    let valid = raw.as_object().is_some_and(|fields| fields.len() == 6)
        && raw["schema"] == GUIDANCE_SCHEMA
        && raw["target_digest"] == bound.target.digest().to_hex()
        && raw["original_goal_digest"] == bound.clock["target"]["original_goal_digest"]
        && (current_witness || blocker_without_sample)
        && (raw["plan_request"].is_null() || raw["blocker"].is_null())
        && anchor["fortress_id"] == bound.target.fortress().get().to_string()
        && anchor["epoch"].as_u64() == Some(bound.target.epoch())
        && anchor["sequence"].is_u64()
        && digest(&anchor["state_hash"]).is_some()
        && source_tick >= bound.target.original_tick();
    valid.then_some(Guidance {
        raw,
        watch,
        bound,
        source_tick,
        sample_withheld: result["child"]["no_work_stock_goal"]
            .get("sample")
            .is_some_and(Value::is_null),
    })
}

fn retained_review(guide: &Guidance<'_>) -> bool {
    let clock = guide.bound.clock;
    let active = &clock["active_run"];
    let request = &guide.raw["plan_request"];
    let sample = &guide.watch["sample"];
    clock["inspection_required"] == false
        && clock["pending"] == true
        && clock["unresolved"] == false
        && clock["may_be_running"] == false
        && clock["advancement_fenced"] != true
        && clock["review_available"] == false
        && active["state"] == "prepared"
        && active["unresolved"] == false
        && active["before_tick"].as_u64() == Some(guide.source_tick)
        && guide.source_tick >= guide.bound.floor
        && request["game_ticks"] == active["game_ticks"]
        && request["wall_millis"] == active["wall_millis"]
        && digest(&active["plan_digest"]).is_some()
        && active["key"]
            .as_str()
            .is_some_and(|key| key.starts_with("stock-watch-clock-") && key.len() == 82)
        && guide.watch["original_goal_proven"] == false
        && guide.watch["consumed_tick_floor"]
            .as_u64()
            .is_some_and(|floor| floor <= guide.source_tick)
        && (guide.sample_withheld
            || (sample["current_truth"] == "true"
                && sample["achieved_at_tick"].is_null()
                && sample["expired"] == false
                && sample["deadline_reached"] == false))
}

fn plan_request(guide: &Guidance<'_>) -> Option<Value> {
    let clock = guide.bound.clock;
    let watch = &guide.watch;
    let sample = &watch["sample"];
    let request = &guide.raw["plan_request"];
    let ticks = request["game_ticks"].as_u64()?;
    let wall = request["wall_millis"].as_u64()?;
    let reviewing = retained_review(guide);
    let at = if reviewing {
        guide.source_tick
    } else {
        sample["anchor"]["tick"].as_u64()?
    };
    let valid = request.as_object().is_some_and(|fields| fields.len() == 4)
        && request["kind"] == "stock_watch_run"
        && request["original_paired_seal"] == clock["original_paired_seal"]
        && guide.raw["blocker"].is_null()
        && (reviewing || !unsettled(clock))
        && clock["mode"] == "control"
        && clock["review_available"] == false
        && (reviewing
            || (sample["current_truth"] == "true"
                && sample["achieved_at_tick"].is_null()
                && sample["expired"] == false
                && sample["deadline_reached"] == false))
        && at >= guide.bound.floor
        && (1..=1200).contains(&ticks)
        && (1..=60_000).contains(&wall)
        && at
            .checked_add(ticks)
            .is_some_and(|end| end <= guide.bound.target.deadline_tick());
    valid.then(|| request.clone())
}

/// Validate proposed re-review advice before the parent intentionally withholds
/// pending goal samples. Otherwise that withdrawal would conceal a malformed,
/// False or Unknown source sample as if it had already been withheld.
pub(super) fn qualify_retained_guidance(result: &mut Value, reference: &Value) {
    if result["stock_clock"]["pending"] != true
        || result["stock_clock"]["review_available"] != false
        || result["stock_clock_guidance"]["plan_request"].is_null()
    {
        return;
    }
    let qualified = guidance(result, reference)
        .is_some_and(|guide| retained_review(&guide) && plan_request(&guide).is_some());
    if !qualified {
        result["stock_clock_guidance"]["plan_request"] = Value::Null;
    }
}

fn blocker<'a>(guide: &'a Guidance<'_>) -> Option<&'a Value> {
    let blocker = &guide.raw["blocker"];
    let valid = guide.raw["plan_request"].is_null()
        && blocker.as_object().is_some_and(|fields| fields.len() == 3)
        && blocker["code"]
            .as_str()
            .is_some_and(|code| !code.is_empty() && code.len() <= 128 && !code.contains('\0'))
        && blocker["message"].as_str().is_some_and(|message| {
            !message.is_empty() && message.len() <= 1024 && !message.contains('\0')
        })
        && matches!(
            blocker["next_action"].as_str(),
            Some("wait" | "inspect" | "operator")
        );
    valid.then_some(blocker)
}

fn evidence() -> Value {
    json!([
        {"complete_evidence_pointer":"/result/stock_clock"},
        {"complete_evidence_pointer":"/result/stock_clock_guidance"},
        {"complete_evidence_pointer":"/result/current_stock_goal/no_work_stock_goal"},
    ])
}

pub(super) fn pending_next(
    result: &Value,
    reference: &Value,
    session: SessionId,
) -> Option<Vec<Value>> {
    let bound = binding(result, reference)?;
    if !unsettled(bound.clock) || review(result, reference).is_some() {
        return None;
    }
    if bound.clock["inspection_required"] == false
        && bound.clock["active_run"]["state"] == "prepared"
        && bound.clock["unresolved"] == false
        && bound.clock["may_be_running"] == false
    {
        if guidance(result, reference)
            .is_some_and(|guide| retained_review(&guide) && plan_request(&guide).is_some())
        {
            return next(result, reference, session);
        }
        let missing_watch = result["child"]["no_work_stock_goal"].is_null()
            && result["current_stock_goal"]["no_work_stock_goal"].is_null();
        let control = result["mode"] == "control" && bound.clock["mode"] == "control";
        let can_retire = missing_watch && control;
        let mut arguments = json!({"session_id":session.to_string()});
        if can_retire {
            arguments["scope"] = json!("stock_clock");
        }
        let mut next = recommendation(
            "resolve_retained_stock_clock_preparation",
            if can_retire {
                "fortress_cancel"
            } else {
                "fortress_explain"
            },
            if can_retire {
                "Explicitly retire the retained undispatched stock-clock preparation, then recreate the unchanged original stock watch and obtain fresh evidence before another reviewed interval. Recovery restores no review or stability history."
            } else if !control {
                "Reopen the same existing parent and stock-clock custody in Control with independent current clock authority before requesting a fresh review or retiring this preparation. Recovery still permits reconciliation; it restores no permission to cancel or unpause."
            } else {
                "Inspect the retained undispatched stock-clock preparation before requesting a fresh review of its exact interval or explicitly cancelling it. Waiting alone cannot restore a lost confirmation."
            },
            "high",
            "high",
            if can_retire { "guarded" } else { "read_only" },
            if can_retire {
                "retires an unexecuted preparation without dispatching native work"
            } else {
                "not_applicable"
            },
            false,
            arguments,
        );
        next["clock_scope"] = json!("stock_clock");
        next["blocker_code"] = json!("stock_clock_prepared_requires_review");
        next["grants_authority"] = json!(false);
        next["confidence"]["evidence"] =
            json!([{"complete_evidence_pointer":"/result/stock_clock"}]);
        return Some(vec![next]);
    }
    let mut next = recommendation(
        "reconcile_original_stock_clock",
        "fortress_wait",
        "Reconcile the exact retained stock-watch clock before sampling the original goal or continuing production. No clock or stock completion is inferred from retained history.",
        "high",
        "high",
        "read_only",
        "not_applicable",
        false,
        json!({"session_id":session.to_string(),"scope":"stock_clock"}),
    );
    next["clock_scope"] = json!("stock_clock");
    next["grants_authority"] = json!(false);
    next["confidence"]["evidence"] = json!([{"complete_evidence_pointer":"/result/stock_clock"}]);
    Some(vec![next])
}

pub(super) fn next(result: &Value, reference: &Value, session: SessionId) -> Option<Vec<Value>> {
    let guide = guidance(result, reference)?;
    if let Some(request) = plan_request(&guide) {
        let reviewing = retained_review(&guide);
        let mut next = recommendation(
            if reviewing {
                "review_retained_stock_clock"
            } else {
                "advance_original_stock_stability"
            },
            "fortress_plan",
            if reviewing {
                "Obtain a fresh review of the exact retained interval after current stock and authority checks. This reuses its original key without another native preparation; confirm the newly published review separately."
            } else {
                "Review one bounded clock interval for the unchanged original stock watch. Planning advances no game time; the clock needs its own explicit confirmation and later stock evidence."
            },
            "high",
            "high",
            "guarded",
            "preparation can be cancelled; a committed interval advances native game time",
            false,
            json!({"session_id":session.to_string(),"request":request.to_string()}),
        );
        next["clock_scope"] = json!("stock_clock");
        next["prepares_review_only"] = json!(true);
        next["reuses_retained_interval"] = json!(reviewing);
        next["separate_commit_confirmation_required"] = json!(true);
        next["grants_authority"] = json!(false);
        next["estimated_cost"]["game_ticks_after_separate_commit"] = request["game_ticks"].clone();
        next["estimated_cost"]["clock_wall_millis"] = request["wall_millis"].clone();
        next["estimated_cost"]["native_time_advanced_by_planning"] = json!(0);
        next["confidence"]["evidence"] = evidence();
        next["invalidating_conditions"] = json!([
            "original parent, target or production custody changes",
            "independent clock authority or request allowance expires",
            "fresh source, current stock truth or original deadline no longer qualifies",
            "another clock becomes pending or current preparation is lost",
        ]);
        return Some(vec![next]);
    }
    let blocked = blocker(&guide)?;
    let mut arguments = json!({"session_id":session.to_string()});
    let tool = if blocked["next_action"] == "wait" {
        arguments["scope"] = json!("goal");
        "fortress_wait"
    } else {
        "fortress_explain"
    };
    let mut next = recommendation(
        "inspect_original_stock_clock",
        tool,
        blocked["message"].as_str()?,
        "high",
        "high",
        "read_only",
        "not_applicable",
        false,
        arguments,
    );
    next["clock_scope"] = json!("stock_clock");
    next["blocker_code"] = blocked["code"].clone();
    next["operator_action_required"] = json!(blocked["next_action"] == "operator");
    next["grants_authority"] = json!(false);
    next["confidence"]["evidence"] = evidence();
    Some(vec![next])
}

pub(super) fn affordances(result: &Value, reference: &Value) -> Vec<Value> {
    if review(result, reference).is_some() {
        return Vec::new();
    }
    let Some(guide) = guidance(result, reference) else {
        return Vec::new();
    };
    let Some(request) = plan_request(&guide) else {
        return Vec::new();
    };
    vec![json!({
        "intent":"review_original_stock_watch_clock", "clock_scope":"stock_clock",
        "request":request, "risk":"guarded", "prepares_review_only":true,
        "native_time_advanced_by_planning":0,
        "separate_commit_confirmation_required":true, "grants_authority":false,
        "reuses_retained_interval":retained_review(&guide),
        "evidence":evidence(),
        "complete_evidence_pointer":"/result/stock_clock_guidance",
    })]
}

pub(super) fn summary(result: &Value, reference: &Value) -> Value {
    let Some(guide) = guidance(result, reference) else {
        return Value::Null;
    };
    let offer = plan_request(&guide);
    let blocker = blocker(&guide);
    if offer.is_none() && blocker.is_none() {
        return Value::Null;
    }
    json!({
        "target_digest":guide.raw["target_digest"],
        "original_goal_digest":guide.raw["original_goal_digest"],
        "goal_witness":guide.raw["goal_witness"],
        "plan_available":offer.is_some(),
        "reuses_retained_interval":offer.is_some() && retained_review(&guide),
        "blocker":blocker,
        "original_deadline_tick":guide.bound.target.deadline_tick(),
        "grants_authority":false, "restored_from_recovery":false,
        "complete_evidence_pointer":"/result/stock_clock_guidance",
    })
}

/// Retained full views describe custody only; compact checkpoint bytes stay
/// untouched and never gain a current-review field.
pub(super) fn withdraw_retained(reference: &mut Value) {
    if reference["stock_clock"].is_object() {
        reference["stock_clock"]["review_available"] = json!(false);
        reference["stock_clock"]["commit_available"] = json!(false);
    }
    if reference.get("stock_clock_guidance").is_some() {
        reference["stock_clock_guidance"] = Value::Null;
    }
}
