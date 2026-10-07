#![forbid(unsafe_code)]

//! Deferred-action regressions for df-cx-authority-budget-threading-lmu and
//! df-cancellation-progress-certificates-vvw. Every state is reached through
//! the public planner/adapter APIs; game time is explicit and deterministic.

use std::collections::BTreeMap;

use dfmcp_adapter::{ActionReceipt, CancelMode, GameAdapter};
use dfmcp_core::{
    ActionId, Capability, CapabilityGrant, CapabilityScope, CommitState, Digest32, EntityId,
    ErrorCode, FortressId, GameTick, IntentId, MapCoord, MapCuboid, ObservationCursor,
    OperationContext, RequestId, Result, RiskTier, SessionId, WorkBudget,
};
use dfmcp_intent::{
    Action, Constraint, DigMode, Intent, ObligationSpec, RequestedAction, StaticPlanner,
};
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::{
    ChunkCoord, CompareOp, EntityKind, EntityRecord, Fact, FactSource, Predicate, Value,
    WorldGraph, WorldSnapshot, terrain::uniform_chunk, tile_codes,
};

const DWARF: EntityId = EntityId::new(101);

fn world(paused: bool) -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    let coord = ChunkCoord { x: 0, y: 0, z: 10 };
    graph
        .chunks
        .insert(coord, uniform_chunk(coord, tile_codes::SOLID_WALL));
    graph.entities.insert(
        DWARF,
        EntityRecord {
            id: DWARF,
            generation: 1,
            revision: 1,
            kind: EntityKind::Unit,
            label: "Urist".to_owned(),
            fields: ["MINE", "WOODCUT"]
                .into_iter()
                .map(|labor| {
                    (
                        format!("labor.{labor}"),
                        Fact::known(
                            Value::Bool(false),
                            GameTick(100),
                            FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                            Digest32::ZERO,
                        ),
                    )
                })
                .collect::<BTreeMap<_, _>>(),
        },
    );
    WorldSnapshot::new(
        FortressId::new(71),
        GameTick(100),
        ObservationCursor::ORIGIN,
        paused,
        graph,
    )
}

fn context(adapter: &MemoryAdapter) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: adapter.snapshot().anchor(),
        budget: WorkBudget::default(),
        grants: [
            (Capability::Observe, RiskTier::ReadOnly),
            (Capability::Plan, RiskTier::ReadOnly),
            (Capability::Designate, RiskTier::Guarded),
            (Capability::ConfigureLabor, RiskTier::Reversible),
            (Capability::ControlClock, RiskTier::Reversible),
            (Capability::Checkpoint, RiskTier::Guarded),
        ]
        .into_iter()
        .map(|(capability, max_risk)| CapabilityGrant {
            capability,
            scope: CapabilityScope {
                fortress_id: Some(adapter.snapshot().fortress_id),
                ..CapabilityScope::default()
            },
            max_risk,
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
        cancellation_requested: false,
    }
}

fn labor_predicate(labor: &str, enabled: bool) -> Predicate {
    Predicate::FieldCompare {
        entity_id: DWARF,
        field: format!("labor.{labor}"),
        op: CompareOp::Eq,
        value: Value::Bool(enabled),
    }
}

fn labor_request(labor: &str, depends_on: Vec<u32>) -> RequestedAction {
    RequestedAction {
        action: Action::SetLabor {
            units: vec![DWARF],
            labor: labor.to_owned(),
            enabled: true,
        },
        preconditions: Vec::new(),
        postconditions: Vec::new(),
        compensation: None,
        obligation: None,
        depends_on,
    }
}

fn bounded_child(deadline: u64) -> RequestedAction {
    let mut child = labor_request("WOODCUT", vec![0]);
    child.obligation = Some(ObligationSpec {
        terminal: labor_predicate("WOODCUT", true),
        failure: None,
        deadline_tick: GameTick(deadline),
        poll_interval_ticks: 1,
        stable_for_observations: 1,
    });
    child
}

fn commit_actions(
    adapter: &mut MemoryAdapter,
    intent_id: u128,
    actions: Vec<RequestedAction>,
) -> Result<Vec<ActionId>> {
    let intent = Intent {
        id: IntentId::new(intent_id),
        anchor: adapter.snapshot().anchor(),
        summary: "deferred action lifecycle".to_owned(),
        terminal_condition: labor_predicate("WOODCUT", true),
        constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
        requested_actions: actions,
    };
    let plan = StaticPlanner::default().prepare_laboratory(adapter.snapshot(), &intent, &context(adapter))?;
    let prepared = adapter.prepare(&plan, &context(adapter))?;
    let receipt = adapter.commit(&plan, &prepared, &context(adapter))?;
    Ok(receipt.actions.into_iter().map(|a| a.action_id).collect())
}

fn dig_request(deadline: u64, stability: u32) -> Result<RequestedAction> {
    let area = MapCuboid::new(MapCoord::new(2, 2, 10), MapCoord::new(2, 2, 10))?;
    Ok(RequestedAction {
        action: Action::DesignateDig {
            area,
            mode: DigMode::Mine,
        },
        preconditions: Vec::new(),
        postconditions: Vec::new(),
        compensation: None,
        obligation: Some(ObligationSpec {
            terminal: Predicate::RegionTerrain {
                area,
                tile_code: tile_codes::FLOOR,
            },
            failure: None,
            deadline_tick: GameTick(deadline),
            poll_interval_ticks: 1,
            stable_for_observations: stability,
        }),
        depends_on: Vec::new(),
    })
}

fn dig_chain(
    paused: bool,
    parent_deadline: u64,
    children: Vec<RequestedAction>,
) -> Result<(MemoryAdapter, Vec<ActionId>)> {
    let mut adapter = MemoryAdapter::new(world(paused));
    let mut actions = vec![dig_request(parent_deadline, 2)?];
    actions.extend(children);
    let ids = commit_actions(&mut adapter, 501, actions)?;
    assert_eq!(
        adapter.action_receipt(ids[1]).map(|a| a.state),
        Some(CommitState::Prepared)
    );
    Ok((adapter, ids))
}

fn poll(adapter: &mut MemoryAdapter, action: ActionId) -> Result<ActionReceipt> {
    adapter.poll_action(action, &context(adapter))
}

fn complete_parent(adapter: &mut MemoryAdapter, parent: ActionId) -> Result<ActionReceipt> {
    adapter.advance_ticks(10)?;
    assert_eq!(
        poll(adapter, parent)?.state,
        CommitState::AppliedAwaitingVerification
    );
    adapter.advance_ticks(1)?;
    let receipt = poll(adapter, parent)?;
    assert_eq!(receipt.state, CommitState::Verified);
    Ok(receipt)
}

fn assert_terminal_stays_exact(adapter: &mut MemoryAdapter, receipt: &ActionReceipt) -> Result<()> {
    assert!(receipt.state.is_terminal());
    assert!(!receipt.evidence.is_empty());
    for ticks in [1, 7] {
        adapter.advance_ticks(ticks)?;
        let before_poll = adapter.snapshot().clone();
        assert_ne!(before_poll.anchor(), receipt.observed_anchor);
        assert_eq!(poll(adapter, receipt.action_id)?, *receipt);
        assert_eq!(adapter.snapshot(), &before_poll);
    }
    Ok(())
}

#[test]
fn ready_child_needs_current_mutation_grant_before_its_first_effect() -> Result<()> {
    let (mut adapter, actions) = dig_chain(false, 500, vec![labor_request("WOODCUT", vec![0])])?;
    complete_parent(&mut adapter, actions[0])?;
    let before = adapter.snapshot().clone();
    let prior_receipt = adapter.action_receipt(actions[1]).cloned();

    for denial in ["observe_only", "expired", "wrong_entity"] {
        let mut ctx = context(&adapter);
        if denial == "observe_only" {
            ctx.grants
                .retain(|grant| grant.capability == Capability::Observe);
        } else {
            for grant in &mut ctx.grants {
                if grant.capability == Capability::ConfigureLabor {
                    if denial == "expired" {
                        grant.expires_at_tick = Some(GameTick(110));
                    } else {
                        grant.scope.entity_ids.insert(EntityId::new(999));
                    }
                }
            }
        }
        let result = adapter.poll_action(actions[1], &ctx);
        assert!(
            matches!(result, Err(ref error) if error.code == ErrorCode::CapabilityDenied),
            "{denial}: {result:?}"
        );
        assert_eq!(adapter.snapshot(), &before, "{denial}");
        assert_eq!(adapter.action_receipt(actions[1]), prior_receipt.as_ref());
    }

    let permitted = poll(&mut adapter, actions[1])?;
    assert_eq!(permitted.state, CommitState::Verified);
    assert!(dfmcp_world::evaluate(
        adapter.snapshot(),
        &labor_predicate("WOODCUT", true)
    ));
    assert_terminal_stays_exact(&mut adapter, &permitted)
}

#[test]
fn changed_precondition_fails_ready_child_without_dispatch() -> Result<()> {
    let mut child = labor_request("WOODCUT", vec![0]);
    child.preconditions.push(labor_predicate("MINE", false));
    let (mut adapter, actions) = dig_chain(false, 500, vec![child])?;
    complete_parent(&mut adapter, actions[0])?;
    // Independent, authorized work invalidates the sealed child's read.
    commit_actions(&mut adapter, 502, vec![labor_request("MINE", Vec::new())])?;
    let before = adapter.snapshot().clone();
    let failed = poll(&mut adapter, actions[1])?;
    assert_eq!(failed.state, CommitState::Failed);
    assert_eq!(adapter.snapshot(), &before);
    assert!(!dfmcp_world::evaluate(
        adapter.snapshot(),
        &labor_predicate("WOODCUT", true)
    ));
    assert_terminal_stays_exact(&mut adapter, &failed)
}

#[test]
fn same_commit_revalidates_each_dispatch_and_rolls_back_prior_effects() -> Result<()> {
    let mut adapter = MemoryAdapter::new(world(true));
    let unpause = RequestedAction {
        action: Action::Pause { paused: false },
        preconditions: vec![Predicate::Paused(true)],
        postconditions: Vec::new(),
        compensation: None,
        obligation: None,
        depends_on: Vec::new(),
    };
    let intent = Intent {
        id: IntentId::new(506),
        anchor: adapter.snapshot().anchor(),
        summary: "an earlier effect invalidates the next dispatch".to_owned(),
        terminal_condition: Predicate::Paused(false),
        constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions: vec![unpause.clone(), unpause],
    };
    let plan = StaticPlanner::default().prepare_laboratory(adapter.snapshot(), &intent, &context(&adapter))?;
    let prepared = adapter.prepare(&plan, &context(&adapter))?;
    let before = adapter.snapshot().clone();
    let prior_transcript = adapter.transcript().clone();

    // Both preconditions hold at prepare and commit entry. The first effect
    // invalidates the second, so the laboratory transaction must roll back
    // the first effect, its receipt, and its journal entry together.
    for _ in 0..2 {
        let result = adapter.commit(&plan, &prepared, &context(&adapter));
        assert!(
            matches!(result, Err(ref error) if error.code == ErrorCode::PreconditionsFailed),
            "dispatch-time precondition must reject the whole commit: {result:?}"
        );
        assert_eq!(adapter.snapshot(), &before);
        assert_eq!(adapter.transcript(), &prior_transcript);
        for step in &plan.steps {
            assert!(adapter.step_receipt(plan.id, step.id).is_none());
        }
    }
    Ok(())
}

#[test]
fn waiting_child_deadline_expires_even_before_parent_verifies() -> Result<()> {
    let (mut adapter, actions) = dig_chain(true, 500, vec![bounded_child(115)])?;
    adapter.advance_ticks(15)?;
    assert_eq!(
        poll(&mut adapter, actions[0])?.state,
        CommitState::AppliedAwaitingVerification
    );
    let before = adapter.snapshot().clone();
    let failed = poll(&mut adapter, actions[1])?;
    assert_eq!(failed.state, CommitState::Failed);
    assert_eq!(adapter.snapshot(), &before);
    assert_terminal_stays_exact(&mut adapter, &failed)
}

#[test]
fn already_expired_child_never_dispatches_when_parent_finally_verifies() -> Result<()> {
    let (mut adapter, actions) = dig_chain(false, 500, vec![bounded_child(110)])?;
    complete_parent(&mut adapter, actions[0])?;
    let before = adapter.snapshot().clone();
    let failed = poll(&mut adapter, actions[1])?;
    assert_eq!(failed.state, CommitState::Failed);
    assert_eq!(adapter.snapshot(), &before);
    assert_terminal_stays_exact(&mut adapter, &failed)
}

#[test]
fn failed_predecessor_closes_the_whole_deferred_dependency_chain() -> Result<()> {
    let (mut adapter, actions) = dig_chain(
        true,
        101,
        vec![
            labor_request("WOODCUT", vec![0]),
            labor_request("MINE", vec![1]),
        ],
    )?;
    adapter.advance_ticks(1)?;
    let parent = poll(&mut adapter, actions[0])?;
    assert_eq!(parent.state, CommitState::Failed);
    let before = adapter.snapshot().clone();
    for action in &actions[1..] {
        let child = poll(&mut adapter, *action)?;
        assert_eq!(child.state, CommitState::Failed);
        assert_eq!(adapter.snapshot(), &before);
        assert!(!child.evidence.is_empty());
    }
    assert_terminal_stays_exact(&mut adapter, &parent)
}

#[test]
fn cancellation_blocks_child_then_terminal_cancellation_fails_it() -> Result<()> {
    let (mut adapter, actions) = dig_chain(true, 500, vec![labor_request("WOODCUT", vec![0])])?;
    adapter.request_cancel(actions[0], CancelMode::StopFutureSteps, &context(&adapter))?;
    let before_drain = adapter.snapshot().clone();
    assert_eq!(poll(&mut adapter, actions[1])?.state, CommitState::Prepared);
    assert_eq!(adapter.snapshot(), &before_drain);
    let cancelled = adapter.finalize_cancel(actions[0], &context(&adapter))?;
    assert_eq!(cancelled.state, CommitState::Cancelled);
    let before = adapter.snapshot().clone();
    let child = poll(&mut adapter, actions[1])?;
    assert_eq!(child.state, CommitState::Failed);
    assert_eq!(adapter.snapshot(), &before);
    let parent = poll(&mut adapter, actions[0])?;
    assert_eq!(parent.state, CommitState::Cancelled);
    assert_eq!(parent.observed_anchor, cancelled.observed_anchor);
    assert_terminal_stays_exact(&mut adapter, &parent)
}

#[test]
fn compensated_predecessor_cannot_release_deferred_work() -> Result<()> {
    let mut adapter = MemoryAdapter::new(world(false));
    let actions = commit_actions(
        &mut adapter,
        503,
        vec![
            RequestedAction {
                action: Action::Pause { paused: true },
                preconditions: Vec::new(),
                postconditions: Vec::new(),
                compensation: None,
                obligation: Some(ObligationSpec {
                    terminal: Predicate::Paused(true),
                    failure: None,
                    deadline_tick: GameTick(500),
                    poll_interval_ticks: 1,
                    stable_for_observations: 2,
                }),
                depends_on: Vec::new(),
            },
            labor_request("WOODCUT", vec![0]),
        ],
    )?;
    adapter.request_cancel(
        actions[0],
        CancelMode::CompensateReversible,
        &context(&adapter),
    )?;
    let compensated = adapter.finalize_cancel(actions[0], &context(&adapter))?;
    assert_eq!(compensated.state, CommitState::Compensated);
    assert!(!adapter.snapshot().paused);
    let before = adapter.snapshot().clone();
    let child = poll(&mut adapter, actions[1])?;
    assert_eq!(child.state, CommitState::Failed);
    assert_eq!(adapter.snapshot(), &before);
    let parent = poll(&mut adapter, actions[0])?;
    assert_eq!(parent.state, CommitState::Compensated);
    assert_eq!(parent.observed_anchor, compensated.observed_anchor);
    assert_terminal_stays_exact(&mut adapter, &parent)
}

#[test]
fn terminal_verification_keeps_its_original_anchor_and_evidence() -> Result<()> {
    let (mut adapter, actions) = dig_chain(false, 500, vec![labor_request("WOODCUT", vec![0])])?;
    let verified = complete_parent(&mut adapter, actions[0])?;
    assert_terminal_stays_exact(&mut adapter, &verified)
}

#[test]
fn completion_first_observed_after_deadline_cannot_certify_on_time_success() -> Result<()> {
    let mut adapter = MemoryAdapter::new(world(false));
    let actions = commit_actions(&mut adapter, 504, vec![dig_request(110, 1)?])?;
    adapter.advance_ticks(11)?;
    assert_eq!(
        adapter.snapshot().tile_code_at(MapCoord::new(2, 2, 10)),
        Some(tile_codes::FLOOR)
    );
    let before = adapter.snapshot().clone();
    let late = poll(&mut adapter, actions[0])?;
    assert_eq!(late.state, CommitState::Failed);
    assert_eq!(adapter.snapshot(), &before);
    assert_terminal_stays_exact(&mut adapter, &late)
}

#[test]
fn completion_proven_at_exact_deadline_is_still_eligible() -> Result<()> {
    let mut adapter = MemoryAdapter::new(world(false));
    let actions = commit_actions(&mut adapter, 505, vec![dig_request(110, 1)?])?;
    adapter.advance_ticks(10)?;
    assert_eq!(adapter.snapshot().tick, GameTick(110));
    let exact = poll(&mut adapter, actions[0])?;
    assert_eq!(exact.state, CommitState::Verified);
    assert_terminal_stays_exact(&mut adapter, &exact)
}
