//! Room continuation crosses real plan, cancellation, commit and recovery paths.

use super::*;
use crate::agent_facade as facade;
use dfmcp_intent::effects;
use dfmcp_world::{Value as WorldValue, laboratory_fact_value};
use serde_json::Value as Json;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn parsed(raw: &str) -> TestResult<Json> {
    Ok(serde_json::from_str(raw)?)
}

fn identity(value: &Json, field: &str) -> TestResult<String> {
    Ok(value[field]
        .as_str()
        .ok_or_else(|| format!("missing {field}: {value}"))?
        .to_owned())
}

fn fixture_error(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InternalInvariantViolation, message)
}

fn in_session<T>(session: &str, body: impl FnOnce(&mut LabSession) -> Result<T>) -> TestResult<T> {
    let handle = resolve_session(Some(session.to_owned()))?;
    Ok(with_session(
        &handle,
        || Err(fixture_error("room session unavailable")),
        body,
    )?)
}

fn open(selector: &str, shared: bool, durable: bool) -> TestResult<String> {
    let opened = parsed(&facade::fortress_open_session(
        Some(false),
        Some(selector.to_owned()),
        Some(
            [
                ("observe", "read_only"),
                ("query", "read_only"),
                ("plan", "read_only"),
                ("control_clock", "reversible"),
                ("configure_production", "reversible"),
                ("configure_labor", "reversible"),
                ("designate", "guarded"),
                ("construct", "guarded"),
                ("checkpoint", "guarded"),
                ("restore", "guarded"),
            ]
            .into_iter()
            .map(|(capability, risk)| (capability.to_owned(), risk.to_owned()))
            .collect(),
        ),
        None,
        Some(2_000),
        None,
        None,
        Some(8_192),
        Some(64),
        Some("starter_fortress".to_owned()),
        Some(shared),
        Some(durable),
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    identity(&opened, "session_id")
}

fn request() -> Json {
    json!({
        "template":"provisioned_room",
        "layout":{"template":"bedroom_cluster", "origin":[2,6,10], "rooms":2, "room_size":[3,3]},
        "prerequisites":{"assign_labor":true,"workshops":[{
            "building":"workshop:Carpenters", "location":[1,0,10],
        }]},
    })
}

fn continuation(session: &str, parent: &str) -> TestResult<Json> {
    parsed(&facade::fortress_plan(
        Some(session.to_owned()),
        Some("finish the retained complete room".to_owned()),
        None,
        None,
        Some(json!({"template":"continue_goal", "plan_digest":parent}).to_string()),
    ))
}

fn commit(session: &str, plan: &Json) -> TestResult<Json> {
    parsed(&facade::fortress_commit(
        Some(session.to_owned()),
        identity(plan, "plan_digest")?,
    ))
}

fn commit_ok(session: &str, plan: &Json) -> TestResult<()> {
    let result = commit(session, plan)?;
    assert_eq!(result["ok"], true, "{result}");
    Ok(())
}

fn wait(session: &str, ticks: u64) -> TestResult<()> {
    let mut remaining = ticks;
    for _ in 0..128 {
        let waited = parsed(&wait_with_ticks(Some(session.to_owned()), Some(remaining)))?;
        if waited["ok"] == true {
            return Ok(());
        }
        assert_eq!(waited["error"]["code"], "budget_exceeded", "{waited}");
        let advanced = waited["advanced_game_ticks"]
            .as_u64()
            .ok_or("bounded wait refused without a progress frontier")?;
        let next = waited["wait_progress"]["remaining_game_ticks"]
            .as_u64()
            .ok_or("bounded wait refused without remaining ticks")?;
        assert!(advanced > 0 && next < remaining, "{waited}");
        assert_eq!(advanced + next, remaining);
        // This is a new explicit foreground request for only the unconsumed
        // ticks; no call's wall-time or game-time allowance is increased.
        remaining = next;
    }
    Err("wait exceeded 128 bounded continuation requests".into())
}

fn goal(session: &str, digest: &str) -> TestResult<Json> {
    in_session(session, |guard| {
        original_goal_observation(guard, digest).map(|(_, value)| value)
    })
}

#[derive(Debug, PartialEq)]
struct Effects {
    snapshot: WorldSnapshot,
    receipts: BTreeMap<String, String>,
    objectives: Vec<Digest32>,
    actions: Vec<ActionId>,
    leases: usize,
}

fn effects_before(session: &str) -> TestResult<Effects> {
    in_session(session, |guard| {
        Ok(Effects {
            snapshot: guard.adapter.snapshot().clone(),
            receipts: guard.commit_receipts.clone(),
            objectives: guard
                .objectives
                .iter()
                .map(|goal| goal.plan_digest)
                .collect(),
            actions: guard.open_actions.clone(),
            leases: guard.leases.by_action.len(),
        })
    })
}

fn assert_history_unchanged(before: &Json, after: &Json) {
    for field in [
        "plan_digest",
        "original_source",
        "sealed_anchor",
        "terminal_condition",
        "first_satisfied_anchor",
        "historical_achievement",
        "restore_abandoned_anchor",
    ] {
        assert_eq!(before[field], after[field], "history field {field}");
    }
}

/// Manufacture two actual beds, cancel only the last allocated placement, and
/// let the other complete. No fabricated item, building or receipt is injected.
fn partial_room(session: &str) -> TestResult<(Json, PreparedPlan, Json, dfmcp_core::EntityId)> {
    let planned = parsed(&plan_request(
        Some(session.to_owned()),
        Some("two complete original bedrooms".to_owned()),
        None,
        None,
        Some(request().to_string()),
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let original = in_session(session, |guard| {
        guard
            .pending
            .as_ref()
            .map(|pending| pending.plan.clone())
            .ok_or_else(|| fixture_error("room plan missing"))
    })?;
    let (last_step, last_item) = original
        .steps
        .iter()
        .rev()
        .find_map(|step| match step.action {
            Action::PlaceFurniture { item, .. } => Some((step.id, item)),
            _ => None,
        })
        .ok_or("room placement missing")?;
    commit_ok(session, &planned)?;
    let root = identity(&planned, "plan_digest")?;
    let active = continuation(session, &root)?;
    assert_eq!(
        active["ok"], false,
        "active room cannot start a second pursuit: {active}"
    );
    let mut allocated = false;
    for _ in 0..200 {
        allocated = in_session(session, |guard| {
            let snapshot = guard.adapter.snapshot();
            let allocated = snapshot
                .graph
                .entities
                .get(&last_item)
                .and_then(|item| item.fields.get("inventory_state"))
                .and_then(|fact| laboratory_fact_value(fact, snapshot.tick))
                == Some(&WorldValue::Text("allocated".to_owned()));
            if allocated {
                let receipt = guard
                    .adapter
                    .step_receipt(original.id, last_step)
                    .ok_or_else(|| fixture_error("allocated room receipt missing"))?;
                assert_eq!(guard.last_action, Some(receipt.action_id));
            }
            Ok(allocated)
        })?;
        if allocated {
            break;
        }
        wait(session, 10)?;
    }
    assert!(
        allocated,
        "last furnishing did not reach its real allocation phase"
    );
    let cancelled = parsed(&cancel_in_scope(
        Some(session.to_owned()),
        Some("stop_future_steps".to_owned()),
        None,
    ))?;
    assert_eq!(cancelled["ok"], true, "{cancelled}");
    wait(session, 1_000)?;
    let history = goal(session, &root)?;
    assert_eq!(history["predicate_truth"], "false", "{history}");
    assert_eq!(history["physical_quiescent"], true, "{history}");
    let completed = in_session(session, |guard| {
        let mut items = Vec::new();
        for step in &original.steps {
            if let Action::PlaceFurniture {
                kind,
                item,
                location,
            } = step.action
            {
                if effects::validate_furniture_completion(
                    guard.adapter.snapshot(),
                    effects::created_entity_id(&step.idempotency_key, 0),
                    kind,
                    item,
                    location,
                )
                .is_ok()
                {
                    items.push(item);
                }
            }
        }
        Ok(items)
    })?;
    assert_eq!(completed.len(), 1);
    Ok((planned, original, history, completed[0]))
}

#[test]
fn cancelled_room_continues_only_missing_furniture_without_rewriting_old_exact_goal() -> TestResult
{
    let _serial = serialized_durable_tests();
    let session = open("963301", false, false)?;
    let (original, _, history, retained_item) = partial_room(&session)?;
    let root = identity(&original, "plan_digest")?;
    let before = effects_before(&session)?;
    let proposal = continuation(&session, &root)?;
    assert_eq!(proposal["ok"], true, "{proposal}");
    assert_eq!(effects_before(&session)?, before);
    let child = identity(&proposal, "plan_digest")?;
    assert_ne!(child, root);
    assert_eq!(proposal["continuation"]["parent_plan_digest"], root);
    assert_eq!(proposal["continuation"]["root_plan_digest"], root);
    assert_eq!(
        proposal["continuation"]["historical_exact_goal_unchanged"],
        true
    );
    in_session(&session, |guard| {
        let pending = guard
            .pending
            .as_ref()
            .ok_or_else(|| fixture_error("continuation missing"))?;
        assert!(matches!(
            pending.source,
            PlanSource::RoomContinuation { .. }
        ));
        let placements: Vec<_> = pending
            .plan
            .steps
            .iter()
            .filter_map(|step| match step.action {
                Action::PlaceFurniture { item, .. } => Some(item),
                _ => None,
            })
            .collect();
        assert_eq!(placements.len(), 1);
        assert!(!placements.contains(&retained_item));
        assert!(
            !pending
                .plan
                .steps
                .iter()
                .any(|step| matches!(step.action, Action::CreateWorkOrder { .. }))
        );
        Ok(())
    })?;
    commit_ok(&session, &proposal)?;
    wait(&session, 1_000)?;
    let child_goal = goal(&session, &child)?;
    assert_eq!(child_goal["predicate_truth"], "true", "{child_goal}");
    assert_eq!(child_goal["physical_quiescent"], true);
    let original_now = goal(&session, &root)?;
    assert_eq!(original_now["predicate_truth"], "false");
    assert_history_unchanged(&history, &original_now);
    assert!(original_now["continuation_request"].is_null());
    let before = effects_before(&session)?;
    let duplicate = continuation(&session, &root)?;
    assert_eq!(duplicate["ok"], false, "{duplicate}");
    assert_eq!(duplicate["error"]["code"], "invalid_intent");
    assert!(
        duplicate["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains(&child))
    );
    assert_eq!(effects_before(&session)?, before);
    Ok(())
}

#[test]
fn room_continuation_rechecks_observe_and_plan_authority_before_commit() -> TestResult {
    let _serial = serialized_durable_tests();
    for (index, capability) in [Capability::Observe, Capability::Plan]
        .into_iter()
        .enumerate()
    {
        let session = open(&format!("96331{index}"), false, false)?;
        let (original, _, _, _) = partial_room(&session)?;
        let root = identity(&original, "plan_digest")?;
        let proposed = continuation(&session, &root)?;
        assert_eq!(proposed["ok"], true, "{proposed}");
        in_session(&session, |guard| {
            for grant in &mut guard.grants {
                if grant.capability == capability {
                    grant.expires_at_tick = Some(GameTick(0));
                }
            }
            Ok(())
        })?;
        let before = effects_before(&session)?;
        for denied in [continuation(&session, &root)?, commit(&session, &proposed)?] {
            assert_eq!(denied["ok"], false, "{denied}");
            assert_eq!(denied["error"]["code"], "capability_denied");
        }
        assert_eq!(effects_before(&session)?, before);
    }
    Ok(())
}

#[test]
fn shared_room_pursuits_cannot_dispatch_around_an_active_same_root_sibling() -> TestResult {
    let _serial = serialized_durable_tests();
    let first = open("963320", true, false)?;
    let (original, _, _, _) = partial_room(&first)?;
    let root = identity(&original, "plan_digest")?;
    let second = open("963320", true, false)?;
    let chosen = continuation(&first, &root)?;
    let competing = continuation(&second, &root)?;
    assert_eq!(chosen["ok"], true, "{chosen}");
    assert_eq!(competing["ok"], true, "{competing}");
    commit_ok(&first, &chosen)?;
    let before = effects_before(&second)?;
    for denied in [commit(&second, &competing)?, continuation(&second, &root)?] {
        assert_eq!(denied["ok"], false, "{denied}");
        assert_eq!(denied["error"]["code"], "preconditions_failed");
        assert!(denied["rebased_plan"].is_null());
    }
    assert_eq!(effects_before(&second)?, before);
    Ok(())
}

#[test]
fn changed_retained_furniture_identity_cannot_commit_a_stale_room_proposal() -> TestResult {
    let _serial = serialized_durable_tests();
    let session = open("963330", false, false)?;
    let (original, _, _, retained_item) = partial_room(&session)?;
    let proposed = continuation(&session, &identity(&original, "plan_digest")?)?;
    assert_eq!(proposed["ok"], true, "{proposed}");
    in_session(&session, |guard| {
        let mut snapshot = guard.adapter.snapshot().clone();
        snapshot
            .graph
            .entities
            .get_mut(&retained_item)
            .ok_or_else(|| fixture_error("retained furniture missing"))?
            .generation += 1;
        snapshot.cursor = snapshot
            .cursor
            .checked_next()
            .ok_or_else(|| fixture_error("cursor exhausted"))?;
        snapshot.refresh_hash();
        guard.adapter.inject_snapshot(snapshot)
    })?;
    let before = effects_before(&session)?;
    let refused = commit(&session, &proposed)?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(effects_before(&session)?, before);
    Ok(())
}

#[test]
fn private_room_envelope_roundtrips_but_public_intake_cannot_import_lineage() -> TestResult {
    let _serial = serialized_durable_tests();
    let session = open("963340", false, false)?;
    let room = room_provisioning::ProvisionedRoomRequest::parse_current(&request().to_string())?;
    let root = Digest32::of_bytes(b"private room envelope");
    let source = PlanSource::RoomContinuation {
        summary: "retained room".to_owned(),
        raw: room_continuation::RoomContinuation::new(root, root, room)?.canonical_json(),
    };
    let snapshot = in_session(&session, |guard| Ok(guard.adapter.snapshot().clone()))?;
    let (intent, analysis) = source.compile(IntentId::new(89), &snapshot)?;
    let durable = source.durable();
    assert!(matches!(
        durable,
        dfmcp_lab::durable::DurablePlanSource::RoomContinuation { .. }
    ));
    let restored = PlanSource::from_durable(&durable);
    assert_eq!(
        restored.compile(IntentId::new(89), &snapshot)?,
        (intent, analysis)
    );
    let PlanSource::RoomContinuation { raw, .. } = source else {
        return Err("room source missing".into());
    };
    let before = effects_before(&session)?;
    for input in [
        raw,
        json!({"template":"continue_goal","plan_digest":root.to_hex(),"room":request()})
            .to_string(),
    ] {
        let refused = parsed(&plan_request(
            Some(session.clone()),
            None,
            None,
            None,
            Some(input),
            None,
        ))?;
        assert_eq!(refused["ok"], false, "{refused}");
    }
    assert_eq!(effects_before(&session)?, before);
    Ok(())
}

struct StateDir(std::path::PathBuf);
impl StateDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-room-continuation-reopen-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        simulate_durable_restart(Some(path.clone()));
        Self(path)
    }
}
impl Drop for StateDir {
    fn drop(&mut self) {
        simulate_durable_restart(None);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn durable_room_reopen_preserves_original_history_and_new_pursuit_proof_separately() -> TestResult {
    let _serial = serialized_durable_tests();
    let directory = StateDir::new();
    let first = open("963350", false, true)?;
    let (original, _, history, _) = partial_room(&first)?;
    let root = identity(&original, "plan_digest")?;
    with_durable_store(|store| store.compact())?;
    simulate_durable_restart(Some(directory.0.clone()));
    let second = open("963350", false, true)?;
    let recovered = goal(&second, &root)?;
    assert_history_unchanged(&history, &recovered);
    assert_ne!(
        recovered["sealed_anchor"]["epoch"],
        recovered["observed_anchor"]["epoch"]
    );
    let proposed = continuation(&second, &root)?;
    assert_eq!(proposed["ok"], true, "{proposed}");
    let child = identity(&proposed, "plan_digest")?;
    commit_ok(&second, &proposed)?;
    wait(&second, 1_000)?;
    let child_history = goal(&second, &child)?;
    assert_eq!(child_history["predicate_truth"], "true");
    assert_eq!(
        child_history["original_source"]["request"]["room"],
        history["original_source"]["request"]
    );
    assert_history_unchanged(&history, &goal(&second, &root)?);
    with_durable_store(|store| store.compact())?;
    simulate_durable_restart(Some(directory.0.clone()));
    let third = open("963350", false, true)?;
    let recovered_child = goal(&third, &child)?;
    assert_eq!(recovered_child["predicate_truth"], "true");
    assert_history_unchanged(&child_history, &recovered_child);
    assert_eq!(goal(&third, &root)?["predicate_truth"], "false");
    assert_history_unchanged(&history, &goal(&third, &root)?);
    let duplicate = continuation(&third, &root)?;
    assert_eq!(duplicate["ok"], false, "{duplicate}");
    assert_eq!(duplicate["error"]["code"], "invalid_intent");
    Ok(())
}
