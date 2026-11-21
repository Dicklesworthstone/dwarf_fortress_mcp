//! Planner-to-adapter inverse setters, including actual cancellation effects.

use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, EntityId, IntentId, RequestId, SessionId, WorkBudget,
};
use dfmcp_intent::{Constraint, Intent, ObligationSpec, RequestedAction, StaticPlanner};
use dfmcp_world::{EntityKind, EntityRecord, Fact, FactPresence, FactSource, Value};

const FIRST: EntityId = EntityId::new(11);
const SECOND: EntityId = EntityId::new(12);
const BURROW: EntityId = EntityId::new(21);

fn fact(value: bool) -> Fact {
    Fact::known(
        Value::Bool(value),
        GameTick(1),
        FactSource::Derived("dfmcp.lab-scenario/1".into()),
        Digest32::ZERO,
    )
}

fn world() -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    for (id, kind, fields) in [
        (
            FIRST,
            EntityKind::Unit,
            BTreeMap::from([
                ("labor.MINE".into(), fact(true)),
                ("labor.BREW".into(), fact(false)),
                ("burrow.21".into(), fact(true)),
            ]),
        ),
        (
            SECOND,
            EntityKind::Unit,
            BTreeMap::from([
                ("labor.MINE".into(), fact(false)),
                ("labor.BREW".into(), fact(false)),
                ("burrow.21".into(), fact(false)),
            ]),
        ),
        (BURROW, EntityKind::Burrow, BTreeMap::new()),
    ] {
        graph.entities.insert(
            id,
            EntityRecord {
                id,
                generation: 1,
                revision: 1,
                kind,
                label: "fixture".into(),
                fields,
            },
        );
    }
    WorldSnapshot::new(
        FortressId::new(71),
        GameTick(100),
        ObservationCursor::ORIGIN,
        true,
        graph,
    )
}

fn context(adapter: &MemoryAdapter) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: adapter.snapshot().anchor(),
        budget: WorkBudget::default(),
        cancellation_requested: false,
        grants: [
            Capability::Observe,
            Capability::Plan,
            Capability::ConfigureLabor,
            Capability::ConfigureLogistics,
            Capability::ControlClock,
        ]
        .into_iter()
        .map(|capability| CapabilityGrant {
            capability,
            max_risk: RiskTier::Reversible,
            scope: CapabilityScope {
                fortress_id: Some(adapter.snapshot().fortress_id),
                ..CapabilityScope::default()
            },
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
    }
}

fn labor(units: Vec<EntityId>, enabled: bool) -> Action {
    Action::SetLabor {
        units,
        labor: "MINE".into(),
        enabled,
    }
}

fn request(action: Action, delayed: bool) -> RequestedAction {
    RequestedAction {
        obligation: delayed.then(|| ObligationSpec {
            terminal: Predicate::All(effects::default_postconditions(
                &action,
                "",
                FortressId::new(71),
            )),
            failure: None,
            deadline_tick: GameTick(1_100),
            poll_interval_ticks: 1,
            stable_for_observations: 2,
        }),
        action,
        preconditions: Vec::new(),
        postconditions: Vec::new(),
        compensation: None,
        depends_on: Vec::new(),
    }
}

fn prepare(
    adapter: &MemoryAdapter,
    id: u128,
    actions: Vec<RequestedAction>,
    legacy: bool,
) -> Result<PreparedPlan> {
    let intent = Intent {
        id: IntentId::new(id),
        anchor: adapter.snapshot().anchor(),
        summary: "source-correct inverse setters".into(),
        terminal_condition: Predicate::All(
            actions
                .iter()
                .flat_map(|requested| {
                    effects::default_postconditions(
                        &requested.action,
                        "",
                        adapter.snapshot().fortress_id,
                    )
                })
                .collect(),
        ),
        constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions: actions,
    };
    let planner = StaticPlanner::default();
    if legacy {
        planner.prepare_laboratory_legacy_compensation(
            adapter.snapshot(),
            &intent,
            &context(adapter),
        )
    } else {
        planner.prepare_laboratory(adapter.snapshot(), &intent, &context(adapter))
    }
}

fn commit(adapter: &mut MemoryAdapter, plan: &PreparedPlan) -> Result<Vec<ActionId>> {
    let prepared = adapter.prepare(plan, &context(adapter))?;
    let committed = adapter.commit(plan, &prepared, &context(adapter))?;
    Ok(committed
        .actions
        .iter()
        .map(|receipt| receipt.action_id)
        .collect())
}

fn value(adapter: &MemoryAdapter, id: EntityId, field: &str) -> Option<Value> {
    adapter
        .snapshot()
        .graph
        .entities
        .get(&id)
        .and_then(|entity| entity.fields.get(field))
        .and_then(Fact::known_value)
        .cloned()
}

fn cancel(adapter: &mut MemoryAdapter, id: ActionId) -> Result<CancelReceipt> {
    adapter.request_cancel(id, CancelMode::CompensateReversible, &context(adapter))?;
    adapter.finalize_cancel(id, &context(adapter))
}

#[test]
fn mixed_labor_and_burrow_cancellation_preserve_preexisting_configuration() -> Result<()> {
    for (action, target) in [
        (labor(vec![FIRST, SECOND], true), true),
        (labor(vec![FIRST, SECOND], false), false),
        (Action::SetBurrowMembership {
            units: vec![FIRST, SECOND], burrow: BURROW, assigned: true,
        }, true),
        (Action::SetBurrowMembership {
            units: vec![FIRST, SECOND], burrow: BURROW, assigned: false,
        }, false),
    ] {
        let field = if matches!(&action, Action::SetLabor { .. }) {
            "labor.MINE"
        } else {
            "burrow.21"
        };
        let mut adapter = MemoryAdapter::new(world());
        let changed = if target { SECOND } else { FIRST };
        let unchanged = if target { FIRST } else { SECOND };
        let unchanged_before = adapter.snapshot().graph.entities[&unchanged].clone();
        let plan = prepare(&adapter, 1, vec![request(action, true)], false)?;
        let inverse = plan.steps[0].compensation.as_ref().ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "fixture inverse missing",
            )
        })?;
        match inverse {
            Action::SetLabor { units, enabled, .. } => {
                assert_eq!(units, &[changed]);
                assert_eq!(*enabled, !target);
            }
            Action::SetBurrowMembership {
                units, assigned, ..
            } => {
                assert_eq!(units, &[changed]);
                assert_eq!(*assigned, !target);
            }
            _ => {
                return Err(DfmcpError::new(
                    ErrorCode::InternalInvariantViolation,
                    "wrong inverse family",
                ));
            }
        }
        assert!(plan.digest_is_valid());
        let ids = commit(&mut adapter, &plan)?;
        assert_eq!(
            adapter.action_receipt(ids[0]).map(|row| row.state),
            Some(CommitState::AppliedAwaitingVerification)
        );
        assert_eq!(
            cancel(&mut adapter, ids[0])?.state,
            CommitState::Compensated
        );
        assert_eq!(value(&adapter, FIRST, field), Some(Value::Bool(true)));
        assert_eq!(value(&adapter, SECOND, field), Some(Value::Bool(false)));
        assert_eq!(adapter.snapshot().graph.entities[&unchanged], unchanged_before);
    }
    Ok(())
}

#[test]
fn no_op_setter_has_no_inverse_even_inside_an_unfinished_goal() -> Result<()> {
    for no_op in [
        labor(vec![FIRST], true),
        Action::SetBurrowMembership { units: vec![FIRST], burrow: BURROW, assigned: true },
        Action::Pause { paused: true },
    ] {
    let mut adapter = MemoryAdapter::new(world());
    let plan = prepare(
        &adapter,
        2,
        vec![
            request(no_op, true),
            request(labor(vec![SECOND], true), true),
        ],
        false,
    )?;
    assert!(plan.steps[0].compensation.is_none());
    assert!(plan.steps[1].compensation.is_some());
    let ids = commit(&mut adapter, &plan)?;
    let before = adapter.snapshot().clone();
    assert_eq!(cancel(&mut adapter, ids[0])?.state, CommitState::Cancelled);
    assert_eq!(&before, adapter.snapshot());
    assert_eq!(
        value(&adapter, FIRST, "labor.MINE"),
        Some(Value::Bool(true))
    );
    }
    Ok(())
}

#[test]
fn unknown_missing_future_and_untrusted_prior_values_never_become_false_inverses() -> Result<()> {
    for variation in 0..6 {
        let mut snapshot = world();
        let unit = snapshot.graph.entities.get_mut(&FIRST).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "fixture unit missing",
            )
        })?;
        let mut unavailable = fact(false);
        match variation {
            0 => {
                unit.fields.remove("labor.MINE");
            }
            1 => {
                unavailable.presence = Some(FactPresence::Unknown("unobserved".into()));
                unit.fields.insert("labor.MINE".into(), unavailable);
            }
            2 => {
                unavailable.source = FactSource::AgentAssertion("caller".into());
                unit.fields.insert("labor.MINE".into(), unavailable);
            }
            3 => {
                unavailable.observed_at = GameTick(101);
                unit.fields.insert("labor.MINE".into(), unavailable);
            }
            4 => {
                unavailable.presence = Some(FactPresence::Absent);
                unit.fields.insert("labor.MINE".into(), unavailable);
            }
            _ => {
                unavailable.source_digest = Digest32::of_bytes(b"unadmitted");
                unit.fields.insert("labor.MINE".into(), unavailable);
            }
        }
        snapshot.refresh_hash();
        let mut adapter = MemoryAdapter::new(snapshot);
        let plan = prepare(
            &adapter,
            10 + variation,
            vec![request(labor(vec![FIRST, SECOND], true), true)],
            false,
        )?;
        assert!(
            plan.steps[0].compensation.is_none(),
            "variation {variation}"
        );
        let ids = commit(&mut adapter, &plan)?;
        let before = adapter.snapshot().clone();
        assert_eq!(cancel(&mut adapter, ids[0])?.state, CommitState::Cancelled);
        assert_eq!(&before, adapter.snapshot());
        assert_eq!(
            value(&adapter, FIRST, "labor.MINE"),
            Some(Value::Bool(true))
        );
    }
    let snapshot = world();
    assert!(
        effects::default_compensation_with_evidence(
            &labor(vec![SECOND], true),
            &PredicateEvidence::untrusted(&snapshot)?,
        )?
        .is_none()
    );
    Ok(())
}

#[test]
fn legacy_inverse_is_reconstructible_but_cannot_overwrite_its_actual_preimage() -> Result<()> {
    let mut adapter = MemoryAdapter::new(world());
    let requests = vec![request(labor(vec![FIRST, SECOND], true), true)];
    let current = prepare(&adapter, 20, requests.clone(), false)?;
    let legacy = prepare(&adapter, 20, requests, true)?;
    assert_ne!(current.digest, legacy.digest);
    assert!(legacy.digest_is_valid());
    assert_eq!(
        legacy.steps[0].compensation,
        Some(labor(vec![FIRST, SECOND], false))
    );
    let ids = commit(&mut adapter, &legacy)?;
    let before = adapter.snapshot().clone();
    assert!(
        matches!(adapter.request_cancel(ids[0], CancelMode::CompensateReversible, &context(&adapter)),
        Err(error) if error.code == ErrorCode::CancellationIncomplete)
    );
    assert_eq!(&before, adapter.snapshot());
    assert_eq!(
        adapter.action_receipt(ids[0]).map(|row| row.state),
        Some(CommitState::AppliedAwaitingVerification)
    );
    adapter.request_cancel(ids[0], CancelMode::StopFutureSteps, &context(&adapter))?;
    assert_eq!(
        adapter.finalize_cancel(ids[0], &context(&adapter))?.state,
        CommitState::Cancelled
    );
    assert_eq!(
        value(&adapter, FIRST, "labor.MINE"),
        Some(Value::Bool(true))
    );
    Ok(())
}

#[test]
fn cancellation_rechecks_drift_and_preserves_the_pending_receipt_on_refusal() -> Result<()> {
    for variation in 0..5 {
        let mut adapter = MemoryAdapter::new(world());
        let plan = prepare(
            &adapter,
            30 + variation,
            vec![request(labor(vec![SECOND], true), true)],
            false,
        )?;
        let ids = commit(&mut adapter, &plan)?;
        adapter.request_cancel(ids[0], CancelMode::CompensateReversible, &context(&adapter))?;
        let record = adapter
            .snapshot
            .graph
            .entities
            .get_mut(&SECOND)
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::InternalInvariantViolation,
                    "fixture unit missing",
                )
            })?;
        match variation {
            0 => {
                record.fields.insert("labor.MINE".into(), fact(false));
                record.revision += 1;
            }
            1 => {
                record.generation += 1;
            }
            2 => {
                let mut unknown = fact(true);
                unknown.source = FactSource::Replay;
                record.fields.insert("labor.MINE".into(), unknown);
            }
            3 => {
                record.revision += 2;
            } // Same-value ABA remains a different writer history.
            _ => {
                record.fields.insert("labor.BREW".into(), fact(true));
                record.revision += 1;
            }
        }
        adapter.snapshot.cursor = adapter.snapshot.cursor.checked_next().ok_or_else(|| {
            DfmcpError::new(ErrorCode::BudgetExceeded, "fixture cursor exhausted")
        })?;
        adapter.snapshot.refresh_hash();
        let before = adapter.snapshot().clone();
        let pending = adapter.action_receipt(ids[0]).cloned();
        assert!(
            matches!(adapter.finalize_cancel(ids[0], &context(&adapter)),
            Err(error) if error.code == ErrorCode::CancellationIncomplete),
            "variation {variation}"
        );
        assert_eq!(&before, adapter.snapshot());
        assert_eq!(adapter.action_receipt(ids[0]), pending.as_ref());
        adapter.request_cancel(ids[0], CancelMode::StopFutureSteps, &context(&adapter))?;
        assert_eq!(
            adapter.finalize_cancel(ids[0], &context(&adapter))?.state,
            CommitState::Cancelled
        );
        assert_eq!(&before, adapter.snapshot());
    }
    Ok(())
}

#[test]
fn deferred_setter_rechecks_its_sealed_source_before_effect() -> Result<()> {
    let mut adapter = MemoryAdapter::new(world());
    let mut child = request(labor(vec![SECOND], true), true);
    child.depends_on = vec![0];
    let plan = prepare(
        &adapter,
        40,
        vec![request(Action::Pause { paused: false }, true), child],
        false,
    )?;
    let ids = commit(&mut adapter, &plan)?;
    assert_eq!(
        adapter.action_receipt(ids[1]).map(|row| row.state),
        Some(CommitState::Prepared)
    );
    let changed = prepare(
        &adapter,
        41,
        vec![request(labor(vec![SECOND], true), false)],
        false,
    )?;
    commit(&mut adapter, &changed)?;
    for _ in 0..2 {
        adapter.advance_ticks(1)?;
        adapter.poll_action(ids[0], &context(&adapter))?;
    }
    let before = adapter.snapshot().clone();
    assert_eq!(
        adapter.poll_action(ids[1], &context(&adapter))?.state,
        CommitState::Failed
    );
    assert_eq!(&before, adapter.snapshot());
    Ok(())
}

#[test]
fn later_overlapping_setter_does_not_reuse_the_initial_source_inverse() -> Result<()> {
    let mut adapter = MemoryAdapter::new(world());
    let mut second = request(labor(vec![SECOND], false), true);
    second.depends_on = vec![0];
    let plan = prepare(
        &adapter,
        50,
        vec![request(labor(vec![SECOND], true), true), second],
        false,
    )?;
    assert!(plan.steps[0].compensation.is_some());
    assert!(plan.steps[1].compensation.is_none());
    let ids = commit(&mut adapter, &plan)?;
    for _ in 0..2 {
        adapter.advance_ticks(1)?;
        adapter.poll_action(ids[0], &context(&adapter))?;
    }
    assert_eq!(
        adapter.poll_action(ids[1], &context(&adapter))?.state,
        CommitState::AppliedAwaitingVerification
    );
    assert_eq!(
        value(&adapter, SECOND, "labor.MINE"),
        Some(Value::Bool(false))
    );
    assert_eq!(cancel(&mut adapter, ids[1])?.state, CommitState::Cancelled);
    Ok(())
}

#[test]
fn pause_compensation_allows_time_but_rejects_another_clock_writer() -> Result<()> {
    for superseded in [false, true] {
        let mut snapshot = world();
        snapshot.paused = false;
        snapshot.refresh_hash();
        let mut adapter = MemoryAdapter::new(snapshot);
        let plan = prepare(
            &adapter,
            60,
            vec![request(Action::Pause { paused: true }, true)],
            false,
        )?;
        let ids = commit(&mut adapter, &plan)?;
        adapter.advance_ticks(1)?;
        if superseded {
            // Even a same-value emergency reassertion is a separately owned
            // clock decision; an earlier action must not unpause it on cancel.
            adapter.pause_for_emergency_drain(&context(&adapter))?;
            let before = adapter.snapshot().clone();
            assert!(matches!(cancel(&mut adapter, ids[0]),
                Err(error) if error.code == ErrorCode::CancellationIncomplete));
            assert_eq!(&before, adapter.snapshot());
            assert!(adapter.snapshot().paused);
        } else {
            assert_eq!(
                cancel(&mut adapter, ids[0])?.state,
                CommitState::Compensated
            );
            assert!(!adapter.snapshot().paused);
        }
    }
    Ok(())
}

#[test]
fn same_value_labor_and_burrow_writers_supersede_earlier_inverse_custody() -> Result<()> {
    for action in [
        labor(vec![SECOND], true),
        Action::SetBurrowMembership {
            units: vec![SECOND],
            burrow: BURROW,
            assigned: true,
        },
    ] {
        let mut adapter = MemoryAdapter::new(world());
        let original = prepare(&adapter, 70, vec![request(action.clone(), true)], false)?;
        let original_ids = commit(&mut adapter, &original)?;
        let before_reassertion = adapter.snapshot().graph.entities[&SECOND].clone();
        // The clock goal is still unsatisfied, so this is an ordinary admitted
        // plan even though its explicit membership/labor reassertion is a no-op.
        let reassertion = prepare(
            &adapter,
            71,
            vec![
                request(action, false),
                request(Action::Pause { paused: false }, false),
            ],
            false,
        )?;
        commit(&mut adapter, &reassertion)?;
        assert_eq!(
            adapter.snapshot().graph.entities[&SECOND],
            before_reassertion
        );
        let before_cancel = adapter.snapshot().clone();
        assert!(matches!(cancel(&mut adapter, original_ids[0]),
            Err(error) if error.code == ErrorCode::CancellationIncomplete));
        assert_eq!(&before_cancel, adapter.snapshot());
    }
    Ok(())
}

#[test]
fn direct_emergency_cancel_and_terminal_drain_supersede_pause_inverses() -> Result<()> {
    for terminal_drain in [false, true] {
        let mut snapshot = world();
        snapshot.paused = false;
        snapshot.refresh_hash();
        let mut adapter = MemoryAdapter::new(snapshot);
        let pause = prepare(
            &adapter,
            80,
            vec![request(Action::Pause { paused: true }, true)],
            false,
        )?;
        let pause_ids = commit(&mut adapter, &pause)?;
        let other = prepare(
            &adapter,
            81,
            vec![request(labor(vec![SECOND], true), !terminal_drain)],
            false,
        )?;
        let other_ids = commit(&mut adapter, &other)?;
        assert!(adapter.snapshot().paused);
        if terminal_drain {
            adapter.drain_action_work_in_mode(
                other_ids[0],
                CancelMode::EmergencyPauseAndDrain,
                &context(&adapter),
            )?;
        } else {
            adapter.request_cancel(
                other_ids[0],
                CancelMode::EmergencyPauseAndDrain,
                &context(&adapter),
            )?;
            adapter.finalize_cancel(other_ids[0], &context(&adapter))?;
        }
        let before = adapter.snapshot().clone();
        assert!(matches!(cancel(&mut adapter, pause_ids[0]),
            Err(error) if error.code == ErrorCode::CancellationIncomplete));
        assert_eq!(&before, adapter.snapshot());
        assert!(adapter.snapshot().paused);
    }
    Ok(())
}
