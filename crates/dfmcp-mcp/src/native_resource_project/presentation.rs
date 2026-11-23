//! Complete bounded Agent Turns. Evidence chunks are data, never authority.
use super::runtime::RESPONSE_BYTES;
use crate::agent_turn::{AgentPhase, AgentTurnBuilder, ContinuityStatus, recommendation};
use crate::live_production_presentation::stock_watch;
use dfmcp_core::{Digest32, RequestId, SessionId};
use fastmcp_rust::{
    CompleteResult, ContentBlock, CoreResult, FinalCallToolResult, FinalCoreResult, ResultMeta,
};
use serde_json::{Value, json};

#[path = "manager_presentation.rs"]
mod manager;
#[path = "tree_presentation.rs"]
mod tree;

const CHUNK_BYTES: usize = 24_000;
const FIRST_TEXT_BYTES: usize = 60_000;
// The modern codec permits a 4096-byte numeric request ID. Reserve its full
// outer envelope independently of the typed tools/call result.
const ENVELOPE_BYTES: usize = 4608;

fn phase(operation: &str) -> AgentPhase {
    match operation {
        "fortress.open_session" => AgentPhase::Bootstrap,
        "fortress.observe" => AgentPhase::Orient,
        "fortress.plan" => AgentPhase::Propose,
        "fortress.commit" => AgentPhase::Commit,
        "fortress.wait" => AgentPhase::Verify,
        "fortress.cancel" => AgentPhase::Reconcile,
        _ => AgentPhase::Inspect,
    }
}

fn publication_pending(reference: &Value) -> bool {
    let checkpoint = &reference["retained_stage"]["checkpoint"];
    checkpoint["production_intent"].is_object() && checkpoint["production"]["recovery"].is_null()
}

fn review(result: &Value) -> Option<(&str, Value)> {
    if result["ok"] != true
        || result["project_cancelled"] == true
        || result["session_closed"] == true
    {
        return None;
    }
    if manager::active(result) {
        return manager::review(result);
    }
    if tree::active(result) {
        return tree::review(result);
    }
    let child = &result["child"];
    // Clock confirmation requires the actual child's current review flag,
    // never a historical seal present in a recovery descriptor.
    for (scope, clock) in [
        ("clock", &child["clock"]),
        ("inventory_clock", &child["inventory_clock"]),
    ] {
        if (clock["review_available"] == true || clock["commit_available"] == true)
            && clock["active_run"]["review_seal"].is_string()
        {
            return Some((scope, clock["active_run"]["review_seal"].clone()));
        }
    }
    if child["original"]["prepared_in_this_connection"] == true
        && child["original"]["review_seal"].is_string()
    {
        return Some(("goal", child["original"]["review_seal"].clone()));
    }
    if child["review"].is_object() && child["frontier"]["review"]["paired_seal"].is_string() {
        return Some(("goal", child["frontier"]["review"]["paired_seal"].clone()));
    }
    None
}

/// Build only from actual child offers. The child's ordinary planner still
/// revalidates the selected evidence and all native permissions.
fn observed_plan(result: &Value) -> Option<Value> {
    if result["mode"] != "control" {
        return None;
    }
    if manager::active(result) {
        return manager::observed_plan(result);
    }
    if tree::active(result) {
        return tree::observed_plan(result);
    }
    let child = &result["child"];
    let observation = &child["observation"];
    if result["stage"] == "workshop"
        && child["original"].is_null()
        && observation["eligible_at_anchor"] == true
    {
        if child["material_policy"] == "operator_pinned" {
            return Some(json!({"kind":"carpenters",
                "expected_observation":observation["witness"].as_str()?,
                "material":observation["material_handle"].as_str()?}));
        }
        if child["material_policy"] == "tree_supply" {
            let source = &child["tree_material"];
            let offer = source["offers"].as_array()?.iter().find(|offer| {
                offer["host_selection_allowed"] == true
                    && offer["item_id"] == observation["selection"]["item_id"]
            })?;
            return Some(json!({"kind":"carpenters_from_tree_supply",
                "expected_observation":source["witness"].as_str()?,
                "material":offer["material_handle"].as_str()?}));
        }
    }
    if result["stage"] == "production" && child["original_plan"].is_null() {
        let goal = &result["original"]["goal"];
        let assigning = goal["assign_labor"].as_bool()?;
        let choices = &observation["workforce_choices"];
        let (options, candidates) = if assigning {
            ("options", "enable_candidates")
        } else {
            ("existing_labor_options", "eligible_enabled_ids")
        };
        let option = choices[options]
            .as_array()?
            .iter()
            .find(|option| option["labor"] == "CARPENTER")?;
        let units = option[candidates]
            .as_array()
            .filter(|ids| !ids.is_empty() && ids.len() <= 32 && ids.iter().all(Value::is_string))?;
        let mut request = json!({
            "expected_observation":observation["seal"].as_str()?,
            "units":units,"labor":"CARPENTER","deadline_tick":goal["deadline_tick"],
            "poll_interval_ticks":goal["poll_interval_ticks"],"stable_samples":goal["stable_samples"],
            "assign_labor":assigning,
        });
        if let Some(targets) = goal.get("targets") {
            let rows = targets.as_array()?;
            if rows.is_empty() || rows.len() > 4 {
                return None;
            }
            request["kind"] = json!("stock_targets");
            request["targets"] = targets.clone();
        } else {
            request["kind"] = json!("stock_target");
            request["recipe"] = goal["recipe"].clone();
            request["minimum_stack_units"] = goal["minimum_stack_units"].clone();
            request["max_order_amount"] = goal["max_order_amount"].clone();
        }
        return Some(request);
    }
    None
}

/// The stock watch must retain every root minimum, including original rows
/// that needed no order. The assessment independently binds each amount limit
/// and the fixed labor mode; the watch binds the shared original timing.
fn no_work_matches_root(raw: &Value, child: &Value, original: &Value) -> bool {
    let timing = [
        ("deadline_tick", "deadline_tick"),
        ("poll_interval_ticks", "poll_interval_ticks"),
        ("stable_samples_required", "stable_samples"),
    ]
    .into_iter()
    .all(|(watch, goal)| !raw[watch].is_null() && raw[watch] == original[goal]);
    if !timing {
        return false;
    }
    if let Some(requested) = original.get("targets") {
        let Some(requested) = requested.as_array() else {
            return false;
        };
        let Some(watched) = raw["targets"].as_array() else {
            return false;
        };
        let assessment = &child["stock_targets"];
        let Some(assessed) = assessment["targets"].as_array() else {
            return false;
        };
        raw["schema"] == "dfmcp.stock-targets-watch/1"
            && (1..=4).contains(&requested.len())
            && watched.len() == requested.len()
            && assessed.len() == requested.len()
            && original["assign_labor"].is_boolean()
            && assessment["labor_assignment_requested"] == original["assign_labor"]
            && requested
                .iter()
                .zip(watched)
                .zip(assessed)
                .all(|((goal, watch), assessed)| {
                    goal["recipe"].is_string()
                        && goal["minimum_stack_units"].is_u64()
                        && goal["max_order_amount"].is_u64()
                        && watch["recipe"] == goal["recipe"]
                        && watch["target_stack_units"] == goal["minimum_stack_units"]
                        && assessed["recipe"] == goal["recipe"]
                        && assessed["target_stack_units"] == goal["minimum_stack_units"]
                        && assessed["max_order_amount"] == goal["max_order_amount"]
                        && assessed["labor_assignment_requested"] == original["assign_labor"]
                })
    } else {
        !raw["recipe"].is_null()
            && raw["recipe"] == original["recipe"]
            && !raw["target_stack_units"].is_null()
            && raw["target_stack_units"] == original["minimum_stack_units"]
    }
}

/// The project and child must name the same original minima and current
/// sample. A retained stage checkpoint never supplies temporal evidence.
fn no_work_stock_goal(result: &Value, reference: &Value) -> Value {
    let child = &result["child"];
    let raw = &child["no_work_stock_goal"];
    let parent = &result["current_stock_goal"];
    let original = &result["original"]["goal"];
    let matches_original = no_work_matches_root(raw, child, original);
    let proven = raw["original_goal_proven"] == true;
    let current = result["ok"] == true
        && result["stage"] == "production"
        && result["mode"] != "offline"
        && result["project_cancelled"] != true
        && result["session_closed"] != true
        && reference["controller_fenced"] != true
        && matches_original
        && parent["no_work_stock_goal"] == *raw
        && parent["original_goal_proven"] == proven
        && result["original_goal_proven"] == proven
        && (!proven || parent["current_goal_anchor"] == raw["sample"]["anchor"]);
    stock_watch::summary(child, Some(&child["anchor"]), current)
}

fn withdraw_retained_stock_goals(reference: &mut Value) {
    for pointer in [
        "/child_reference",
        "/retained_stage/checkpoint/production",
        "/retained_stage/checkpoint/production_intent/recovery_reference",
    ] {
        if let Some(child) = reference.pointer_mut(pointer) {
            stock_watch::withdraw(child);
        }
    }
}

fn next_step(
    result: &Value,
    reference: &Value,
    session: SessionId,
    stock_watch_owned: bool,
) -> Vec<Value> {
    if result["session_closed"] == true {
        return Vec::new();
    }
    if let Some(next) = manager::next_step(result, reference, session) {
        return next;
    }
    if !stock_watch_owned
        && !publication_pending(reference)
        && let Some(next) = tree::next_step(result, reference, session)
    {
        return next;
    }
    let mut arguments = json!({"session_id":session.to_string()});
    let mut confirmation = false;
    let (tool, reason) = if result["ok"] != true || reference["controller_fenced"] == true {
        (
            "fortress_query",
            "Inspect retained parent and child custody before any recovery or further action.",
        )
    } else if result["project_cancelled"] == true {
        arguments["scope"] = json!("goal");
        (
            "fortress_wait",
            "Reconcile the existing child. The durable project stop forbids any next stage; active clocks require their explicit cancellation scope.",
        )
    } else if publication_pending(reference) {
        (
            "fortress_query",
            "Inspect the acknowledged production intent and exact existing child files. Reopen in recovery to acknowledge a complete matching original; never initialize, resize or replace an interrupted publication.",
        )
    } else if stock_watch_owned && !result["current_stock_goal"]["no_work_stock_goal"].is_object() {
        (
            "fortress_explain",
            "Inspect the retained no-work goal whose current identity could not be verified. Preserve its original target and deadline before further planning.",
        )
    } else if let Some((scope, seal)) = review(result) {
        confirmation = true;
        arguments["scope"] = json!(scope);
        arguments["review_seal"] = seal;
        arguments["confirmed"] = json!(true);
        (
            "fortress_commit",
            "Review the complete child evidence and confirm only this current exact native review.",
        )
    } else if result["mode"] == "control"
        && (result["stage"] == "transition"
            || (result["stage"] == "workshop"
                && result["child"]["original"]["historical_goal"] == "complete"))
    {
        arguments["request"] = json!(
            json!({
                "kind":"advance_to_production",
                "expected_head":reference["parent_acknowledged"]["head"],
            })
            .to_string()
        );
        (
            "fortress_plan",
            "Freshly qualify the completed original Workshop and enter production without dispatching a new native effect.",
        )
    } else if result["current_stock_goal"]["no_work_stock_goal"].is_object()
        && result["current_stock_goal"]["no_work_stock_goal"]["cancelled"] != true
    {
        let watch = &result["current_stock_goal"]["no_work_stock_goal"];
        if watch["original_goal_proven"] == true
            || watch["source_fenced"] == true
            || watch["sample"]["expired"] == true
            || result["mode"] == "offline"
        {
            (
                "fortress_explain",
                "Inspect the unchanged no-work goal and its exact current evidence. Historical counts and a missed deadline cannot supply a new stable proof.",
            )
        } else {
            arguments["scope"] = json!("goal");
            (
                "fortress_wait",
                "Take one fresh inventory sample for every original no-work minimum together. Later game ticks at the fixed cadence are required; wait does not create an order or advance time.",
            )
        }
    } else if result["original_goal_proven"] == true
        || result["current_stock_goal"]["already_satisfied"].is_object()
    {
        (
            "fortress_explain",
            "Inspect the exact terminal evidence and its observation anchor; a retained census does not prove a fresh or stable count.",
        )
    } else if result["stage"] == "production"
        && result["mode"] == "control"
        && result["child"]["frontier"]["next_ready"].is_number()
        && result["child"]["original_plan"]["plan_digest"].is_string()
    {
        arguments["request"] = json!(
            json!({
                "kind":"step",
                "original_plan_digest":result["child"]["original_plan"]["plan_digest"],
            })
            .to_string()
        );
        (
            "fortress_plan",
            "Prepare the next dependency-ready step of the exact original stock-target plan for its own review.",
        )
    } else if result["stage"] == "production" && result["child"]["original_plan"].is_object() {
        arguments["scope"] = json!("goal");
        (
            "fortress_wait",
            "Reconcile the exact original steps or obtain a foreground labor and inventory goal sample.",
        )
    } else if result["stage"] == "workshop" && result["child"]["clock"]["pending"] == true {
        arguments["scope"] = json!("clock");
        (
            "fortress_wait",
            "Reconcile the retained Workshop clock before asking for a construction sample.",
        )
    } else if result["stage"] == "workshop" && result["child"]["original"].is_object() {
        arguments["scope"] = json!("goal");
        (
            "fortress_wait",
            "Reconcile or sample the original Workshop; a placement receipt alone does not prove construction complete.",
        )
    } else if result["mode"] == "offline" {
        (
            "fortress_query",
            "Inspect local custody; Offline mode cannot acquire new native evidence or effects.",
        )
    } else if let Some(plan) = observed_plan(result) {
        arguments["request"] = json!(plan.to_string());
        (
            "fortress_plan",
            "Plan the fixed project goal using these published eligible child offers; planning still revalidates the original source and authority.",
        )
    } else if result["child"]["observation"].is_object() {
        let choices = &result["child"]["observation"]["workforce_choices"];
        let field = if result["original"]["goal"]["assign_labor"] == true {
            "next_observation_request"
        } else {
            "next_existing_labor_observation_request"
        };
        if choices[field].is_object() {
            arguments["request"] = json!(choices[field].to_string());
            (
                "fortress_observe",
                "Read the published next workforce selection page before choosing the original production workers.",
            )
        } else {
            (
                "fortress_explain",
                "Inspect the current eligibility blockers; no qualified next plan is available from this observation.",
            )
        }
    } else {
        (
            "fortress_observe",
            "Obtain the current child's bounded source evidence before planning the fixed project goal.",
        )
    };
    let mut next = recommendation(
        "continue_original_resource_project",
        tool,
        reason,
        "high",
        "high",
        if confirmation { "guarded" } else { "read_only" },
        if confirmation {
            "native effect may be irreversible"
        } else {
            "not_applicable"
        },
        confirmation,
        arguments,
    );
    next["grants_authority"] = json!(false);
    next["confidence"]["evidence"] =
        json!([{"parent_acknowledged":reference["parent_acknowledged"]}]);
    next["invalidating_conditions"] = json!([
        "original custody or policy changes",
        "native source or epoch changes",
        "original deadline or authority expires",
        "prepared connection, lease or review is lost",
    ]);
    vec![next]
}

pub(super) fn packet(
    operation: &str,
    session: Option<SessionId>,
    request: Option<RequestId>,
    mut result: Value,
    mut reference: Value,
) -> Value {
    let no_work_stock_goal = no_work_stock_goal(&result, &reference);
    let has_stock_watch = !result["child"]["no_work_stock_goal"].is_null()
        || !result["current_stock_goal"]["no_work_stock_goal"].is_null();
    if has_stock_watch {
        let proven = no_work_stock_goal["original_goal_proven"] == true;
        if result["child"].is_object() {
            result["child"]["no_work_stock_goal"] = no_work_stock_goal.clone();
            result["child"]["original_goal_proven"] = json!(proven);
            result["child"]["current_state_proven"] = json!(false);
            // A no-work observation owns no native preparation to confirm.
            result["child"]["review"] = Value::Null;
            if result["child"]["frontier"].is_object() {
                result["child"]["frontier"]["review"] = Value::Null;
            }
            for clock in ["clock", "inventory_clock"] {
                if result["child"][clock].is_object() {
                    result["child"][clock]["review_available"] = json!(false);
                    result["child"][clock]["commit_available"] = json!(false);
                }
            }
        }
        if !result["current_stock_goal"].is_object() {
            result["current_stock_goal"] = json!({});
        }
        result["current_stock_goal"]["no_work_stock_goal"] = no_work_stock_goal.clone();
        result["current_stock_goal"]["original_goal_proven"] = json!(proven);
        result["current_stock_goal"]["current_goal_anchor"] = if proven {
            no_work_stock_goal["sample"]["anchor"].clone()
        } else {
            Value::Null
        };
        result["original_goal_proven"] = json!(proven);
        result["current_state_proven"] = json!(false);
    }
    withdraw_retained_stock_goals(&mut reference);
    let mut turn = AgentTurnBuilder::new(operation, phase(operation)).continuity(
        ContinuityStatus::Indeterminate,
        None,
        Some(json!({"canonical_cross_family_anchor_available":false,
            "native_family_anchors_are_independent":true})),
        None,
    );
    if let Some(session) = session {
        turn = turn.session_id(session.to_string());
    }
    if let Some(request) = request {
        turn = turn.request_id(request.to_string());
    }
    let publication_pending = publication_pending(&reference);
    let pending = (!publication_pending && reference["controller_fenced"] != true)
        .then(|| review(&result))
        .flatten()
        .map(|(scope, seal)| {
            json!({
                "scope":scope, "review_seal":seal, "requires_confirmation":true,
                "complete_evidence_pointer":"/result/child", "restored_from_recovery":false,
            })
        });
    let original = &reference["retained_stage"]["checkpoint"];
    let goal = result.get("original").unwrap_or(&reference["original"]);
    let stage = result
        .get("stage")
        .unwrap_or(&reference["retained_stage"]["stage"]);
    let cancelled = *stage == "cancelled";
    let child = result
        .get("child")
        .filter(|value| value.is_object())
        .unwrap_or(&reference["child_reference"]);
    let child_path = if result.get("child").is_some_and(Value::is_object) {
        "/result/child"
    } else {
        "/retained_reference/child_reference"
    };
    let tree_active = tree::active(&result)
        || matches!(
            stage.as_str(),
            Some("tree_opening" | "tree" | "tree_transition")
        )
        || (cancelled
            && matches!(
                reference["retained_stage"]["cancelled_from"].as_str(),
                Some("tree_opening" | "tree" | "tree_transition")
            ));
    let tree_visible = tree_active || original["tree"].is_object();
    let tree_state = if tree_active {
        child
    } else {
        &original["tree"]
    };
    let tree_path = if tree_active {
        child_path
    } else {
        "/retained_reference/retained_stage/checkpoint/tree"
    };
    let manager_active = manager::in_stage(&result, &reference);
    let manager_visible = manager_active || original["manager"].is_object();
    let manager_child = manager_active
        && (child["source_profile"] == "manager/1.23"
            || manager::original(child).is_some()
            || child["project_source_frontier"].is_object());
    let manager_state = if manager_child {
        child
    } else {
        &original["manager"]
    };
    let manager_path = if manager_child {
        child_path
    } else {
        "/retained_reference/retained_stage/checkpoint/manager"
    };
    let manager_original = manager::original(manager_state);
    let manager_verified =
        manager_active && manager::verified(&result) && reference["controller_fenced"] != true;
    let mut actions = ["clock", "inventory_clock"].into_iter().filter_map(|scope| {
        let clock = &child[scope];
        clock.is_object().then(|| json!({
            "scope":scope, "pending":clock["pending"], "unresolved":clock["unresolved"],
            "may_be_running":clock["may_be_running"], "advancement_fenced":clock["advancement_fenced"],
            "target_id":clock["target_id"], "original_key":clock["active_run"]["key"],
            "native_unpause_attempted":clock["active_run"]["native_unpause_attempted"],
            "current_pause_proven":false, "complete_custody_pointer":"/retained_reference",
            "complete_evidence_pointer":format!("{child_path}/{scope}"),
        }))
    }).collect::<Vec<_>>();
    if let Some(original) = &manager_original {
        actions.push(json!({
            "scope":"manager", "original":original,
            "current_production_readiness_proven":false,
            "complete_custody_pointer":"/retained_reference",
            "complete_evidence_pointer":manager_path,
            "automatic_redispatch":false,
        }));
    }
    let mut indeterminate = if result["ok"] != true && reference.is_object() {
        vec![json!({"custody_pointer":"/retained_reference","automatic_redispatch":false})]
    } else {
        Vec::new()
    };
    if tree_active && child["clock"]["unresolved"] == true {
        indeterminate.push(json!({"scope":"clock", "current_pause_proven":false,
            "complete_custody_pointer":"/retained_reference",
            "complete_evidence_pointer":format!("{child_path}/clock"),
            "automatic_redispatch":false}));
    }
    if manager_visible && manager::uncertain(manager_state) {
        indeterminate.push(json!({"scope":"manager",
            "current_production_readiness_proven":false,
            "complete_custody_pointer":"/retained_reference",
            "complete_evidence_pointer":manager_path,
            "automatic_redispatch":false}));
    }
    let obligation = reference.is_object().then(|| {
        json!({
            "stage":reference["retained_stage"]["stage"],
            "project_goal":goal["goal"],
            "manager_original":manager::original(&original["manager"]),
            "manager_handoff_pointer":original["manager_handoff"].is_object()
                .then_some("/retained_reference/retained_stage/checkpoint/manager_handoff"),
            "tree_original":original["tree"]["original"],
            "tree_handoff_pointer":original["tree_handoff"].is_object()
                .then_some("/retained_reference/retained_stage/checkpoint/tree_handoff"),
            "workshop_original":original["workshop"]["original"],
            "production_original":original["production"]["recovery"],
            "project_cancelled":reference["retained_stage"]["stage"] == "cancelled",
            "exact_custody_pointer":"/retained_reference",
        })
    });
    let publications = original["production_intent"].is_object().then(|| json!({
        "kind":"original_production_plan",
        "status":if publication_pending { "pending_child_checkpoint" } else { "retained" },
        "plan_digest":original["production_intent"]["recovery_reference"]["recovery"]["plan_digest"],
        "intent_pointer":"/retained_reference/retained_stage/checkpoint/production_intent",
        "recovery_requires_existing_files":true,
        "replacement_original_allowed":false,
        "native_effect_dispatched_by_publication":false,
        "preparation_restored":false,
    })).into_iter().collect::<Vec<_>>();
    let mut uncertainty = vec![
        json!({"uncertainty_id":"native_project_scope","epistemic_state":"unknown",
            "statement":"This explicitly unadmitted development owner can retain an optional original Manager appointment, an optional original Tree designation and supply handoff, original Workshop construction and one fixed furniture stock goal or bounded portfolio. Every original minimum remains required; each native effect and configured clock interval has its own review."}),
        json!({"uncertainty_id":"receipt_and_goal","epistemic_state":"unknown",
            "statement":"Placement, order insertion and clock receipts are historical custody. Production with an order requires its current labor and stock monitor. An originally satisfied no-work target instead requires fresh qualified inventory samples at its fixed cadence and deadline; its planning census contributes no stability. Neither path proves causal output attribution."}),
        json!({"uncertainty_id":"stop_and_recovery","epistemic_state":"indeterminate",
            "statement":"A project stop prevents new stages and new reviews. It does not remove an appointed Manager, undo construction, cancel an already inserted order or prove a clock paused. Preserve all recovery references; independently reconcile or cancel the active clock. Opening markers never grant replayed initialization."}),
    ];
    if goal["goal"]["targets"].is_array() {
        uncertainty.push(json!({"uncertainty_id":"portfolio_time_control","epistemic_state":"observed",
            "statement":"This fixed stock portfolio has no production or inventory clock. Foreground goal samples require separately managed game time and jointly preserve every original minimum, cadence and deadline. Optional Tree and Workshop prefix clocks keep their independent authority."}));
    }
    if manager_visible {
        uncertainty.push(json!({"uncertainty_id":"manager_appointment_and_readiness","epistemic_state":"unknown",
            "statement":"A verified original Manager/1.23 receipt covers its historical assignment, HF link, history event and responsibility-cache transaction. A retained handoff cannot establish current Manager readiness, office sufficiency, recipe feasibility or final stock. Production independently reopens and freshly qualifies the exact donor. Native Manager projection anchors are not canonical world anchors; capsule commitments name retained raw bytes, not inline evidence chunks."}));
    }
    if tree_visible {
        uncertainty.push(json!({"uncertainty_id":"tree_supply_and_capture","epistemic_state":"unknown",
            "statement":"Tree completion observes a coordinate predicate and eligible ground wood; no stable plant identity, causal wood provenance or final stock proof follows. Workshop independently qualifies current material. The bounded Tree review exposes semantic fields and a capsule commitment; raw capsule bytes remain with their indicated owner or custody and are not delivered in this packet."}));
    }
    if publication_pending {
        uncertainty.push(json!({
            "uncertainty_id":"production_original_publication",
            "epistemic_state":"indeterminate",
            "statement":"The parent acknowledged the exact compiled production original before child publication. Existing complete matching custody may be recovered; the intent alone does not prove the child files are complete or restore a native review.",
            "intent_pointer":"/retained_reference/retained_stage/checkpoint/production_intent",
        }));
    }
    let references = if reference.is_object() {
        vec![
            json!({"parent_acknowledged":reference["parent_acknowledged"],
            "complete_recovery_pointer":"/retained_reference","current_native_proof":false}),
        ]
    } else {
        Vec::new()
    };
    let mut partial_domains = vec![
        "original_tree",
        "original_workshop",
        "selected_production_goal",
    ];
    let mut omitted_domains = vec!["canonical_cross_family_world"];
    if manager_visible {
        partial_domains.insert(0, "original_manager");
    } else {
        omitted_domains.push("manager_appointment");
    }
    let manager_offer = if reference["controller_fenced"] == true {
        None
    } else {
        manager::observed_plan(&result)
    };
    let mut built = turn
        .briefing(json!({
            "profile":"native-resource-project",
            "runtime_admitted":false, "mutation_admissible":false,
            "stage":stage, "project_cancelled":cancelled,
            "original_goal":goal["goal"],
            "no_work_stock_goal":no_work_stock_goal,
            "original_goal_proven":result["ok"] == true && *stage == "production"
                && !cancelled && result["original_goal_proven"] == true,
            "current_goal_anchor":if result["ok"] == true && *stage == "production" && !cancelled {
                result["current_stock_goal"]["current_goal_anchor"].clone()
            } else { Value::Null },
            "manager_appointment":if manager_visible { json!({
                "original":manager_original,
                "historical_appointment_proved":manager_verified
                    && manager_state["original"]["appointment_proved"] == true,
                "historical_cache_transaction_proved":manager_verified
                    && manager_state["original"]["production_manager_cache_proved"] == true,
                "handoff_retained":original["manager_handoff"].is_object(),
                "current_production_readiness_proven":false,
                "project_goal_proven":false,
                "source_fenced":manager::source_fenced(manager_state),
                "native_projection_anchor":manager_state.get("native_anchor")
                    .unwrap_or(&manager_state["custody"]["native_anchor"]),
                "canonical_world_anchor":false,
                "complete_evidence_pointer":manager_path,
            }) } else { Value::Null },
            "tree_supply":if tree_visible { json!({
                "historical_supply_observed_tick":tree_state["original"]["historical_supply_observed_tick"],
                "fresh_supply_observed_this_call":tree_active && result["ok"] == true
                    && tree_state["retained_state_currently_verified"] == true
                    && tree_state["original"]["fresh_supply_observed"] == true,
                "project_goal_proven":false,"causal_item_provenance":"unknown",
                "complete_evidence_pointer":tree_path,
            }) } else { Value::Null },
            "canonical_cross_family_anchor_available":false,
            "development_review_available":pending.is_some(),
            "production_publication_pending":publication_pending,
        }))
        .active_work(json!({
            "pending_plans":pending.clone().into_iter().collect::<Vec<_>>(),
            "actions":actions,
            "obligations":obligation.into_iter().collect::<Vec<_>>(),
            "no_work_stock_goal":no_work_stock_goal,
            "cancellation_drains":if cancelled {
                vec![json!({"scope":"original_child","native_effects_cancelled":false,
                    "custody_pointer":"/retained_reference"})]
            } else { Vec::new() },
            "indeterminate_effects":indeterminate,
            "publications":publications,
            "confirmations":pending.into_iter().collect::<Vec<_>>(),
        }))
        .affordances(manager_offer.map(|request| json!({
            "intent":"appoint_vacant_manager", "request":request,
            "unit_id":result["child"]["observation"]["unit_id"],
            "histfig_id":result["child"]["observation"]["histfig_id"],
            "capability":"configure_production", "risk":"irreversible",
            "confirmation_required":true, "grants_authority":false,
            "complete_evidence_pointer":"/result/child/observation",
        })).into_iter().collect())
        .recommendations(session.map_or_else(Vec::new, |id| {
            next_step(&result, &reference, id, has_stock_watch)
        }))
        .uncertainty(uncertainty)
        .coverage(json!({
            "status":"partial",
            "complete_domains":if result["project_custody_currently_verified"] == true {
                vec!["parent_stage_custody"]
            } else { Vec::<&str>::new() },
            "partial_domains":partial_domains,
            "omitted_domains":omitted_domains,
            "parent_custody_verified_this_call":result["project_custody_currently_verified"] == true,
            "recovery_references_are_current_proof":false,
        }))
        .budget(result.get("budget").cloned().unwrap_or(json!({"max_output_bytes":RESPONSE_BYTES})))
        .references(references)
        .build();
    if let Some(briefing) = built["briefing"].as_object_mut() {
        briefing.remove("admission");
    }
    json!({"agent_turn":built, "result":result, "retained_reference":reference,
        "retained_reference_is_current_native_proof":false})
}

pub(super) fn failure(
    operation: &str,
    session: Option<SessionId>,
    request: Option<RequestId>,
    cause: dfmcp_core::DfmcpError,
    reference: Value,
) -> Value {
    packet(
        operation,
        session,
        request,
        json!({
            "ok":false, "error":{"code":cause.code.as_str(),"message":cause.message,"retryable":false},
            "original_goal_proven":false, "current_goal_available":false,
            "project_custody_currently_verified":false, "automatic_redispatch":false,
            "effect_may_have_occurred":reference.is_object(), "runtime_admitted":false,
        }),
        reference,
    )
}

fn complete(texts: Vec<String>, is_error: bool) -> CompleteResult<FinalCallToolResult> {
    CompleteResult::new(
        FinalCallToolResult {
            content: texts.into_iter().map(ContentBlock::text).collect(),
            is_error,
            structured_content: None,
        },
        ResultMeta::empty(),
    )
}
fn fits(result: &CompleteResult<FinalCallToolResult>) -> bool {
    CoreResult::Final(FinalCoreResult::ToolsCall {
        result: result.clone(),
        diagnostic: None,
    })
    .encode()
    .is_ok_and(|encoded| encoded.len() + ENVELOPE_BYTES <= RESPONSE_BYTES as usize)
}
fn decoded_hex(raw: &str) -> Option<Vec<u8>> {
    if raw.is_empty() || raw.len() > 2 * RESPONSE_BYTES as usize || !raw.len().is_multiple_of(2) {
        return None;
    }
    let nibble = |b| match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        _ => None,
    };
    raw.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| Some(nibble(p[0])? * 16 + nibble(p[1])?))
        .collect()
}
fn hex_blocks(raw: &str, path: &str, texts: &mut Vec<String>) -> Option<Value> {
    let bytes = decoded_hex(raw)?;
    let mut blocks = Vec::new();
    for (index, chunk) in bytes.chunks(CHUNK_BYTES).enumerate() {
        let offset = index * CHUNK_BYTES;
        blocks.push(json!({"content_index":texts.len(),"offset_bytes":offset,
            "byte_length":chunk.len(),"sha256":Digest32::of_bytes(chunk).to_hex()}));
        texts.push(raw[offset * 2..(offset + chunk.len()) * 2].to_owned());
    }
    Some(
        json!({"schema":"dfmcp.native-resource-evidence/1","field_path":path,
        "encoding":"hex","byte_length":bytes.len(),"sha256":Digest32::of_bytes(&bytes).to_hex(),
        "blocks":blocks,"grants_authority":false}),
    )
}
fn detach_hex(value: &mut Value, path: &str, texts: &mut Vec<String>, depth: usize) -> Option<()> {
    if depth > 32 {
        return None;
    }
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let path = format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"));
                if key.ends_with("_hex") && child.as_str().is_some_and(|s| s.len() > 4096) {
                    *child = hex_blocks(child.as_str()?, &path, texts)?;
                } else {
                    detach_hex(child, &path, texts, depth + 1)?;
                }
            }
        }
        Value::Array(array) => {
            for (index, child) in array.iter_mut().enumerate() {
                detach_hex(child, &format!("{path}/{index}"), texts, depth + 1)?;
            }
        }
        _ => {}
    }
    Some(())
}

fn json_blocks(value: &Value, path: &str, texts: &mut Vec<String>) -> Value {
    let raw = value.to_string();
    let mut blocks = Vec::new();
    let mut offset = 0;
    while offset < raw.len() {
        let mut end = (offset + CHUNK_BYTES).min(raw.len());
        while !raw.is_char_boundary(end) {
            end -= 1;
        }
        let chunk = &raw[offset..end];
        blocks.push(json!({"content_index":texts.len(),"offset_bytes":offset,
            "byte_length":chunk.len(),"sha256":Digest32::of_bytes(chunk.as_bytes()).to_hex()}));
        texts.push(chunk.to_owned());
        offset = end;
    }
    json!({"schema":"dfmcp.native-resource-evidence/1","field_path":path,
        "encoding":"utf-8-json","byte_length":raw.len(),"sha256":Digest32::of_bytes(raw.as_bytes()).to_hex(),
        "blocks":blocks,"grants_authority":false})
}

fn texts(mut packet: Value) -> Option<Vec<String>> {
    let mut texts = vec![String::new()];
    detach_hex(&mut packet, "", &mut texts, 0)?;
    if packet.to_string().len() > FIRST_TEXT_BYTES {
        let reference = packet["retained_reference"].clone();
        let descriptor = json_blocks(&reference, "/retained_reference", &mut texts);
        packet["retained_reference"] = json!({
            "parent_acknowledged":reference["parent_acknowledged"],
            "authority_tick_floor":reference["authority_tick_floor"],
            "controller_fenced":reference["controller_fenced"],
            "complete_recovery":descriptor,
            "current_goal_available":false,
        });
    }
    if packet.to_string().len() > FIRST_TEXT_BYTES {
        let child = packet["result"]["child"].clone();
        packet["result"]["child"] = json_blocks(&child, "/result/child", &mut texts);
    }
    texts[0] = packet.to_string();
    (texts[0].len() <= FIRST_TEXT_BYTES).then_some(texts)
}

/// Called while the owner lock is held. None requires withdrawing every review
/// before emitting a smaller complete recovery packet.
pub(super) fn render(packet: Value) -> Option<CompleteResult<FinalCallToolResult>> {
    let is_error = packet["result"]["ok"] != true;
    let result = complete(texts(packet)?, is_error);
    fits(&result).then_some(result)
}

#[cfg(test)]
#[path = "portfolio_presentation_tests.rs"]
mod portfolio_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn reassemble(
        descriptor: &Value,
        parts: &[String],
    ) -> std::result::Result<String, Box<dyn std::error::Error>> {
        let mut raw = String::new();
        for part in descriptor["blocks"].as_array().ok_or("missing blocks")? {
            let index = usize::try_from(
                part["content_index"]
                    .as_u64()
                    .ok_or("missing block index")?,
            )?;
            raw.push_str(parts.get(index).ok_or("missing content block")?);
        }
        Ok(raw)
    }

    #[test]
    fn large_native_evidence_is_complete_and_codec_bounded()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let original = "ab".repeat(40_000);
        let packet = packet(
            "fortress.query",
            None,
            None,
            json!({"ok":true,"child":{"native_capture_hex":original}}),
            Value::Null,
        );
        let parts = texts(packet.clone()).ok_or("packet did not fit")?;
        let first: Value = serde_json::from_str(&parts[0])?;
        let descriptor = &first["result"]["child"]["native_capture_hex"];
        assert_eq!(reassemble(descriptor, &parts)?, original);
        assert_eq!(descriptor["byte_length"], 40_000);
        assert!(render(packet).is_some());
        assert!(parts.iter().all(|text| text.len() < 65_536));
        Ok(())
    }

    #[test]
    fn oversized_review_requires_withdrawal_instead_of_partial_success() {
        let packet = packet(
            "fortress.plan",
            None,
            None,
            json!({"ok":true,"child":{"canonical_capture_hex":"aa".repeat(180_000),
                "original":{"prepared_in_this_connection":true,"review_seal":"s"}}}),
            json!({"parent_acknowledged":{"head":"h"}}),
        );
        assert!(render(packet).is_none());
    }

    #[test]
    fn metadata_chunks_preserve_unicode_and_exact_recovery_pin()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let recovery = json!({"parent_acknowledged":{"journal_id":"j","head":"h","frames":3},
            "notes":vec!["工房 and stock".repeat(40); 180]});
        let packet = packet(
            "fortress.query",
            None,
            None,
            json!({"ok":true}),
            recovery.clone(),
        );
        let parts = texts(packet).ok_or("metadata packet did not fit")?;
        let first: Value = serde_json::from_str(&parts[0])?;
        let raw = reassemble(&first["retained_reference"]["complete_recovery"], &parts)?;
        assert_eq!(serde_json::from_str::<Value>(&raw)?, recovery);
        assert_eq!(
            first["retained_reference"]["parent_acknowledged"]["head"],
            "h"
        );
        Ok(())
    }

    #[test]
    fn historical_no_work_and_errors_never_claim_project_completion() {
        let output = packet(
            "fortress.query",
            None,
            None,
            json!({"ok":true,"current_stock_goal":{"already_satisfied":{
                "satisfied_at_anchor":true,"current_inventory_proven":false,
                "stability_proven":false}}}),
            Value::Null,
        );
        assert_eq!(
            output["agent_turn"]["briefing"]["original_goal_proven"],
            false
        );
        assert_eq!(output["agent_turn"]["anchor"], Value::Null);
        assert_eq!(
            output["agent_turn"]["continuity"]["status"],
            "indeterminate"
        );
        let error = packet(
            "fortress.wait",
            None,
            None,
            json!({"ok":false}),
            json!({"original":{"goal":{"deadline_tick":800}},"retained_stage":{"stage":"cancelled"}}),
        );
        assert_eq!(
            error["agent_turn"]["briefing"]["original_goal"]["deadline_tick"],
            800
        );
        assert_eq!(error["agent_turn"]["briefing"]["project_cancelled"], true);
        assert_eq!(
            error["agent_turn"]["coverage"]["complete_domains"],
            json!([])
        );
    }

    #[test]
    fn pending_original_publication_withholds_reviews_and_replacement_plans() {
        let reference = json!({
            "parent_acknowledged":{"head":"parent-intent-head"},
            "retained_stage":{"stage":"production","checkpoint":{
                "production":{"recovery":null},
                "production_intent":{
                    "schema":"dfmcp.project-production-intent/1",
                    "recovery_reference":{"recovery":{"plan_digest":"recorded-original"}}
                }
            }}
        });
        let result = json!({"ok":true,"mode":"control","stage":"production","child":{
            "original_plan":null,
            "review":{"retained":true},
            "frontier":{"review":{"paired_seal":"stale-review"}}
        }});
        let output = packet(
            "fortress.query",
            Some(SessionId::new(1)),
            None,
            result,
            reference.clone(),
        );
        let turn = &output["agent_turn"];
        assert_eq!(turn["briefing"]["production_publication_pending"], true);
        assert_eq!(turn["briefing"]["development_review_available"], false);
        assert_eq!(turn["active_work"]["confirmations"], json!([]));
        assert_eq!(turn["active_work"]["pending_plans"], json!([]));
        assert_eq!(
            turn["active_work"]["publications"][0]["plan_digest"],
            "recorded-original"
        );
        assert_eq!(
            turn["active_work"]["publications"][0]["status"],
            "pending_child_checkpoint"
        );
        assert_eq!(
            turn["active_work"]["publications"][0]["replacement_original_allowed"],
            false
        );
        assert_eq!(turn["recommendations"][0]["tool"], "fortress_query");
        assert_eq!(output["retained_reference"], reference);

        let failed = failure(
            "fortress.plan",
            Some(SessionId::new(1)),
            None,
            super::super::error(
                dfmcp_core::ErrorCode::CorruptLedger,
                "child publication incomplete",
            ),
            reference,
        );
        assert_eq!(
            failed["agent_turn"]["briefing"]["original_goal_proven"],
            false
        );
        assert_eq!(
            failed["agent_turn"]["active_work"]["confirmations"],
            json!([])
        );
        assert_eq!(
            failed["agent_turn"]["active_work"]["publications"][0]["status"],
            "pending_child_checkpoint"
        );
    }

    #[test]
    fn acknowledged_child_keeps_its_intent_and_resumes_only_the_original_step() {
        let reference = json!({"retained_stage":{"stage":"production","checkpoint":{
            "production":{"recovery":{"plan_digest":"recorded-original"}},
            "production_intent":{"recovery_reference":{"recovery":{"plan_digest":"recorded-original"}}}
        }}});
        let result = json!({"ok":true,"mode":"control","stage":"production","child":{
            "original_plan":{"plan_digest":"recorded-original"},"frontier":{"next_ready":0}
        }});
        let output = packet(
            "fortress.query",
            Some(SessionId::new(1)),
            None,
            result,
            reference,
        );
        let turn = &output["agent_turn"];
        assert_eq!(turn["briefing"]["production_publication_pending"], false);
        assert_eq!(turn["active_work"]["publications"][0]["status"], "retained");
        assert_eq!(turn["recommendations"][0]["tool"], "fortress_plan");
        assert!(
            turn["recommendations"][0]["arguments"]["request"]
                .as_str()
                .is_some_and(|raw| raw.contains("recorded-original") && raw.contains("step"))
        );
        assert_eq!(turn["active_work"]["confirmations"], json!([]));
    }

    #[test]
    fn next_step_prepares_pending_work_and_keeps_clock_confirmation_separate()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let session = SessionId::new(1);
        let result = json!({"ok":true,"mode":"control","stage":"production","child":{
            "original_plan":{"plan_digest":"original"},"frontier":{"next_ready":2}}});
        let next = next_step(&result, &Value::Null, session, false);
        assert_eq!(next[0]["tool"], "fortress_plan");
        let raw = next[0]["arguments"]["request"]
            .as_str()
            .ok_or("missing step request")?;
        assert_eq!(serde_json::from_str::<Value>(raw)?["kind"], "step");
        let result = json!({"ok":true,"stage":"production","child":{
            "inventory_clock":{"review_available":true,"active_run":{"review_seal":"clock"}}}});
        let next = next_step(&result, &Value::Null, session, false);
        assert_eq!(next[0]["tool"], "fortress_commit");
        assert_eq!(next[0]["arguments"]["scope"], "inventory_clock");
        assert_eq!(next[0]["arguments"]["review_seal"], "clock");
        let historical = json!({"ok":true,"child":{"clock":{
            "review_available":false,"active_run":{"review_seal":"historical"}}}});
        assert!(review(&historical).is_none());
        Ok(())
    }
    #[test]
    fn observed_stock_plan_preserves_every_original_goal_term()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let goal = json!({"recipe":"wooden_bed","minimum_stack_units":5,"max_order_amount":20,
            "deadline_tick":900,"poll_interval_ticks":5,"stable_samples":2,"assign_labor":false});
        let result = json!({"ok":true,"mode":"control","stage":"production","original":{"goal":goal},
            "child":{"original_plan":null,"observation":{"seal":"observation",
                "workforce_choices":{"existing_labor_options":[{"labor":"CARPENTER",
                    "eligible_enabled_ids":["selected-worker"]}]}}}});
        let request = observed_plan(&result).ok_or("qualified observed plan missing")?;
        for key in [
            "recipe",
            "minimum_stack_units",
            "max_order_amount",
            "deadline_tick",
            "poll_interval_ticks",
            "stable_samples",
            "assign_labor",
        ] {
            assert_eq!(request[key], goal[key]);
        }
        assert_eq!(request["units"], json!(["selected-worker"]));
        assert_eq!(request["expected_observation"], "observation");
        assert_eq!(request["labor"], "CARPENTER");
        let mut blocked = result;
        blocked["child"]["observation"]["workforce_choices"]["existing_labor_options"] = json!([]);
        assert!(observed_plan(&blocked).is_none());
        Ok(())
    }

    fn no_work_project_fixture() -> Value {
        let (_, child) = stock_watch::tests::fixture();
        json!({
            "ok":true,"mode":"control","stage":"production",
            "project_cancelled":false,"original_goal_proven":true,
            "original":{"goal":{
                "recipe":"wooden_bed","minimum_stack_units":10,
                "max_order_amount":1,"deadline_tick":200,
                "poll_interval_ticks":10,"stable_samples":2,"assign_labor":true,
            }},
            "current_stock_goal":{
                "original_goal_proven":true,
                "current_goal_anchor":child["anchor"],
                "no_work_stock_goal":child["no_work_stock_goal"],
                "already_satisfied":{"satisfied_at_anchor":true,"stability_proven":false},
            },
            "child":child,
        })
    }

    fn refresh_no_work_child(result: &mut Value) {
        result["current_stock_goal"]["no_work_stock_goal"] =
            result["child"]["no_work_stock_goal"].clone();
        let proven = result["child"]["original_goal_proven"] == true;
        result["current_stock_goal"]["original_goal_proven"] = json!(proven);
        result["current_stock_goal"]["current_goal_anchor"] = if proven {
            result["child"]["anchor"].clone()
        } else {
            Value::Null
        };
        result["original_goal_proven"] = json!(proven);
    }

    #[test]
    fn no_work_project_proof_and_wait_follow_the_fixed_inventory_goal() {
        let result = no_work_project_fixture();
        let complete = packet(
            "fortress.wait",
            Some(SessionId::new(1)),
            None,
            result.clone(),
            Value::Null,
        );
        assert_eq!(
            complete["agent_turn"]["briefing"]["original_goal_proven"],
            true
        );
        assert_eq!(complete["result"]["original_goal_proven"], true);
        assert_eq!(
            complete["agent_turn"]["briefing"]["current_goal_anchor"],
            result["child"]["anchor"]
        );
        assert_eq!(
            complete["agent_turn"]["active_work"]["confirmations"],
            json!([])
        );
        assert_eq!(
            complete["agent_turn"]["recommendations"][0]["tool"],
            "fortress_explain"
        );

        let mut pending = result;
        stock_watch::tests::pending(&mut pending["child"]);
        refresh_no_work_child(&mut pending);
        let waiting = packet(
            "fortress.plan",
            Some(SessionId::new(1)),
            None,
            pending,
            Value::Null,
        );
        assert_eq!(waiting["result"]["original_goal_proven"], false);
        assert!(waiting["agent_turn"]["briefing"]["current_goal_anchor"].is_null());
        assert_eq!(
            waiting["agent_turn"]["recommendations"][0]["tool"],
            "fortress_wait"
        );
        assert_eq!(
            waiting["agent_turn"]["recommendations"][0]["arguments"]["scope"],
            "goal"
        );
        assert_eq!(
            waiting["agent_turn"]["active_work"]["confirmations"],
            json!([])
        );
    }

    #[test]
    fn no_work_project_rejects_forged_parent_links_and_stale_current_claims() {
        for cause in 0..8 {
            let mut result = no_work_project_fixture();
            match cause {
                0 => result["current_stock_goal"]["current_goal_anchor"]["tick"] = json!(141),
                1 => result["original"]["goal"]["minimum_stack_units"] = json!(11),
                2 => result["original"]["goal"]["deadline_tick"] = json!(210),
                3 => {
                    result["current_stock_goal"]["no_work_stock_goal"]["goal_witness"] =
                        json!("f".repeat(64))
                }
                4 => {
                    result["child"]["no_work_stock_goal"]["sample"]["stable_samples_observed"] =
                        json!(1)
                }
                5 => result["mode"] = json!("offline"),
                6 => result["project_cancelled"] = json!(true),
                _ => result["ok"] = json!(false),
            }
            let output = packet(
                "fortress.query",
                Some(SessionId::new(1)),
                None,
                result,
                Value::Null,
            );
            assert_eq!(output["result"]["original_goal_proven"], false, "{cause}");
            assert_eq!(
                output["agent_turn"]["briefing"]["original_goal_proven"], false,
                "{cause}"
            );
            assert!(
                output["result"]["child"]["no_work_stock_goal"]["sample"].is_null(),
                "{cause}"
            );
            assert!(
                output["result"]["current_stock_goal"]["current_goal_anchor"].is_null(),
                "{cause}"
            );
            assert_eq!(
                output["agent_turn"]["active_work"]["confirmations"],
                json!([]),
                "{cause}"
            );
        }
    }

    #[test]
    fn no_work_project_later_stock_loss_keeps_read_only_wait_after_achievement() {
        let mut result = no_work_project_fixture();
        let child = &mut result["child"];
        child["anchor"]["tick"] = json!(210);
        child["anchor"]["sequence"] = json!(10);
        child["observation_tick_floor"] = json!(210);
        child["no_work_stock_goal"]["consumed_tick_floor"] = json!(210);
        child["no_work_stock_goal"]["sample"]["anchor"] = child["anchor"].clone();
        child["no_work_stock_goal"]["sample"]["observed_stack_units"] = json!(5);
        child["no_work_stock_goal"]["sample"]["current_truth"] = json!("false");
        child["no_work_stock_goal"]["sample"]["deadline_reached"] = json!(true);
        child["no_work_stock_goal"]["original_goal_proven"] = json!(false);
        child["no_work_stock_goal"]["current_inventory_proven"] = json!(false);
        child["original_goal_proven"] = json!(false);
        refresh_no_work_child(&mut result);
        let output = packet(
            "fortress.wait",
            Some(SessionId::new(1)),
            None,
            result,
            Value::Null,
        );
        assert_eq!(output["result"]["original_goal_proven"], false);
        let sample = &output["result"]["current_stock_goal"]["no_work_stock_goal"]["sample"];
        assert_eq!(sample["achieved_at_tick"], 140);
        assert_eq!(sample["current_truth"], "false");
        assert_eq!(sample["expired"], false);
        assert!(output["agent_turn"]["briefing"]["current_goal_anchor"].is_null());
        assert_eq!(
            output["agent_turn"]["recommendations"][0]["tool"],
            "fortress_wait"
        );
        assert_eq!(
            output["agent_turn"]["active_work"]["confirmations"],
            json!([])
        );
    }

    #[test]
    fn no_work_project_recovery_references_strip_samples_and_proof() {
        let result = no_work_project_fixture();
        let reference = json!({
            "child_reference":result["child"],
            "retained_stage":{"stage":"production","checkpoint":{
                "production":result["child"],
                "production_intent":{"recovery_reference":result["child"]},
            }},
        });
        let output = packet(
            "fortress.query",
            Some(SessionId::new(1)),
            None,
            json!({"ok":false,"original_goal_proven":false}),
            reference,
        );
        for pointer in [
            "/retained_reference/child_reference",
            "/retained_reference/retained_stage/checkpoint/production",
            "/retained_reference/retained_stage/checkpoint/production_intent/recovery_reference",
        ] {
            let child = output.pointer(pointer).expect("retained child");
            assert!(child["no_work_stock_goal"]["sample"].is_null());
            assert_eq!(child["no_work_stock_goal"]["original_goal_proven"], false);
            assert_eq!(child["original_goal_proven"], false);
            assert_eq!(
                child["no_work_stock_goal"]["goal_witness"],
                result["child"]["no_work_stock_goal"]["goal_witness"]
            );
        }
        assert_eq!(
            output["agent_turn"]["briefing"]["original_goal_proven"],
            false
        );
        assert!(output["agent_turn"]["briefing"]["no_work_stock_goal"].is_null());
    }

    #[test]
    fn invalid_no_work_identity_keeps_ownership_without_a_satisfied_census_hint() {
        for with_choices in [true, false] {
            let mut result = no_work_project_fixture();
            result["current_stock_goal"]["already_satisfied"] = Value::Null;
            result["child"]["no_work_stock_goal"]["goal_witness"] = json!("invalid");
            result["current_stock_goal"]["no_work_stock_goal"] =
                result["child"]["no_work_stock_goal"].clone();
            if with_choices {
                result["child"]["observation"] = json!({
                    "seal":"f".repeat(64),
                    "workforce_choices":{"options":[{
                        "labor":"CARPENTER","enable_candidates":["7"]
                    }]}
                });
                assert!(observed_plan(&result).is_some());
            } else {
                result["child"]["observation"] = Value::Null;
            }
            let output = packet(
                "fortress.query",
                Some(SessionId::new(1)),
                None,
                result,
                Value::Null,
            );
            assert!(output["result"]["current_stock_goal"]["no_work_stock_goal"].is_null());
            assert_eq!(output["result"]["original_goal_proven"], false);
            assert_eq!(
                output["agent_turn"]["active_work"]["confirmations"],
                json!([])
            );
            let recommendations = &output["agent_turn"]["recommendations"];
            assert_eq!(recommendations.as_array().map(Vec::len), Some(1));
            assert_eq!(recommendations[0]["tool"], "fortress_explain");
        }
    }
}
