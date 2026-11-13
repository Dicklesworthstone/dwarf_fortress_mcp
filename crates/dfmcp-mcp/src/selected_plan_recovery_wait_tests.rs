//! Archived per-step proof cannot be reconstructed from an achieved goal.

use super::*;
use crate::server::{self, PlanSource};
use dfmcp_adapter::GameAdapter;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, CommitState, FortressId, IntentId,
    OperationContext, RequestId, RiskTier, SessionId, WorkBudget,
};
use dfmcp_intent::StaticPlanner;
use dfmcp_lab::{MemoryAdapter, durable::DurableStepUpdate};

struct StateDir(std::path::PathBuf);

impl StateDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-selected-recovery-{label}-{}",
            std::process::id(),
        ));
        let _ = std::fs::remove_dir_all(&path);
        Self(path)
    }
}

impl Drop for StateDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(759),
        request_id: RequestId::new(759),
        anchor: snapshot.anchor(),
        budget: WorkBudget {
            max_game_ticks: 2_000,
            max_wall_millis: 60_000,
            max_actions: 64,
            ..WorkBudget::default()
        },
        cancellation_requested: false,
        grants: [
            (Capability::Observe, RiskTier::ReadOnly),
            (Capability::Plan, RiskTier::ReadOnly),
            (Capability::ControlClock, RiskTier::Reversible),
            (Capability::Checkpoint, RiskTier::Guarded),
            (Capability::Designate, RiskTier::Guarded),
        ]
        .into_iter()
        .map(|(capability, max_risk)| CapabilityGrant {
            capability,
            max_risk,
            scope: CapabilityScope {
                fortress_id: Some(snapshot.fortress_id),
                ..CapabilityScope::default()
            },
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
    }
}

#[derive(Clone, Copy)]
enum Publication {
    Complete,
    MissingStep,
    Unanchored,
    FalseTerminal,
}

struct Fixture {
    store: DurableLabStore,
    _directory: StateDir,
    plan: PreparedPlan,
    retained: DurableObjective,
    head: WorldSnapshot,
    recovered: WorldSnapshot,
    lineage: RecoveredGoalLineage,
}

fn fixture(label: &str, publication: Publication) -> Result<Fixture> {
    let directory = StateDir::new(label);
    let sealed =
        crate::lab_world::scenario_snapshot("starter_fortress", FortressId::new(759), false)?;
    let source = PlanSource::Actions {
        summary: "recover exact original actions".to_owned(),
        raw: r#"[
            {"action":{"kind":"pause","paused":false}},
            {"action":{"kind":"designate_dig","min":[0,3,10],"max":[7,5,10],"mode":"mine"}}
        ]"#
        .to_owned(),
    };
    let intent = source.intent(IntentId::new(759), &sealed)?;
    let plan = StaticPlanner::default().prepare_laboratory(&sealed, &intent, &context(&sealed))?;
    assert_eq!(plan.steps.len(), 2);
    let mut store = DurableLabStore::open(&directory.0)?;
    store.persist_head("starter_fortress", &sealed)?;
    store.persist_objective_commit(
        &sealed,
        plan.digest,
        plan.intent_id.get(),
        source.durable(),
        SessionId::new(759),
        &[],
    )?;
    let mut adapter = MemoryAdapter::new(sealed);
    let prepared = adapter.prepare(&plan, &context(adapter.snapshot()))?;
    adapter.commit(&plan, &prepared, &context(adapter.snapshot()))?;
    let mut updates = Vec::new();
    for step in &plan.steps {
        let pause = matches!(step.action, Action::Pause { .. });
        assert_eq!(
            adapter
                .step_receipt(plan.id, step.id)
                .map(|receipt| receipt.state),
            Some(if pause {
                CommitState::Verified
            } else {
                CommitState::AppliedAwaitingVerification
            }),
        );
        if matches!(publication, Publication::MissingStep) && !pause {
            continue;
        }
        let state = if pause || matches!(publication, Publication::FalseTerminal) {
            "verified"
        } else {
            "dispatched"
        };
        if matches!(publication, Publication::Unanchored) {
            store.persist_step(plan.anchor.fortress_id, plan.digest, step.id.get(), state)?;
        } else {
            updates.push(DurableStepUpdate {
                plan_digest: plan.digest,
                step: step.id.get(),
                state: state.to_owned(),
            });
        }
    }
    store.persist_progress("starter_fortress", adapter.snapshot(), &updates, &[])?;
    let head = adapter.snapshot().clone();
    let recovered = MemoryAdapter::recovered(head.clone())?.snapshot().clone();
    let lineage = RecoveredGoalLineage::verify(&head, head.anchor(), recovered.anchor())?;
    let retained = store
        .objective(head.fortress_id, plan.digest)
        .cloned()
        .ok_or_else(|| corrupt("fixture lost its original goal"))?;
    Ok(Fixture {
        store,
        _directory: directory,
        plan,
        retained,
        head,
        recovered,
        lineage,
    })
}

fn capture(fixture: &Fixture) -> RecoveredPlanCustody {
    RecoveredPlanCustody::capture(
        &fixture.store,
        &fixture.retained,
        &fixture.plan,
        &fixture.lineage,
    )
}

fn complete(fixture: &mut Fixture, anchored: bool) -> Result<()> {
    let mut adapter = MemoryAdapter::recovered(fixture.head.clone())?;
    adapter.advance_ticks(300)?;
    let dig = fixture
        .plan
        .steps
        .iter()
        .find(|step| matches!(step.action, Action::DesignateDig { .. }))
        .ok_or_else(|| corrupt("fixture lacks dig"))?;
    verify_terminal(dig, adapter.snapshot())?;
    let updates = if anchored {
        vec![DurableStepUpdate {
            plan_digest: fixture.plan.digest,
            step: dig.id.get(),
            state: "verified".to_owned(),
        }]
    } else {
        // A legacy unanchored transition may record a correct final state;
        // it still cannot establish the exact historical action proof.
        fixture.store.persist_step(
            fixture.head.fortress_id,
            fixture.plan.digest,
            dig.id.get(),
            "verified",
        )?;
        Vec::new()
    };
    fixture
        .store
        .persist_progress("starter_fortress", adapter.snapshot(), &updates, &[])?;
    fixture.head = adapter.snapshot().clone();
    fixture.recovered = MemoryAdapter::recovered(fixture.head.clone())?
        .snapshot()
        .clone();
    fixture.lineage = RecoveredGoalLineage::verify(
        &fixture.head,
        fixture.head.anchor(),
        fixture.recovered.anchor(),
    )?;
    Ok(())
}

#[test]
fn mixed_recovery_retains_original_terminal_map_across_retirement() -> Result<()> {
    let mut fixture = fixture("mixed", Publication::Complete)?;
    let custody = capture(&fixture);
    let checked = custody.checked()?;
    assert_eq!(checked.receipts.len(), fixture.plan.steps.len());
    assert_eq!(
        checked
            .receipts
            .iter()
            .filter(|receipt| receipt.state == State::Verified)
            .count(),
        1,
    );
    assert_eq!(
        checked
            .receipts
            .iter()
            .filter(|receipt| receipt.state == State::Dispatched)
            .count(),
        1,
    );
    assert!(checked.archived_sources > 0);
    assert!(checked.archived_bytes > 0);
    custody.validate(
        &fixture.plan,
        fixture.lineage.active_anchor(),
        fixture.recovered.anchor(),
    )?;

    // A complete current transition must precede retirement. The immutable
    // first capture remains valid for its own process; a later recovery reads
    // the actual final map archived at retirement, never inferred goal truth.
    let active = fixture.lineage.active_anchor();
    complete(&mut fixture, true)?;
    fixture
        .store
        .retire_commit(fixture.head.fortress_id, fixture.plan.digest)?;
    custody.validate(&fixture.plan, active, fixture.head.anchor())?;
    fixture.store.compact()?;
    let recovered = capture(&fixture);
    assert!(recovered.checked()?.archived_after_retirement);
    assert!(
        recovered
            .checked()?
            .receipts
            .iter()
            .all(|receipt| receipt.state == State::Verified)
    );
    Ok(())
}

#[test]
fn achieved_goal_cannot_replace_a_legacy_unanchored_retired_action() -> Result<()> {
    let mut fixture = fixture("legacy-retired", Publication::Complete)?;
    complete(&mut fixture, false)?;
    assert_eq!(
        crate::lab_world::plan_goal_truth(&fixture.head, &fixture.plan)?,
        dfmcp_world::PredicateTruth::True,
    );
    fixture
        .store
        .retire_commit(fixture.head.fortress_id, fixture.plan.digest)?;
    let missing = capture(&fixture).checked().err().ok_or_else(|| {
        corrupt("retired action history unexpectedly reconstructed a receipt map")
    })?;
    assert_eq!(missing.code, ErrorCode::Conflict);
    assert!(
        missing
            .details
            .iter()
            .any(|(key, value)| { key == "reason" && value == "action_evidence_unavailable" })
    );
    Ok(())
}

#[test]
fn missing_unanchored_and_false_terminal_receipts_refuse_independently_of_goal_truth() -> Result<()>
{
    for (label, publication) in [
        ("missing", Publication::MissingStep),
        ("unanchored", Publication::Unanchored),
        ("false-terminal", Publication::FalseTerminal),
    ] {
        let fixture = fixture(label, publication)?;
        assert!(capture(&fixture).checked().is_err(), "{label}");
    }
    Ok(())
}

#[test]
fn action_receipts_cannot_come_from_a_future_frontier_or_cross_another_reset() -> Result<()> {
    let mut fixture = fixture("future", Publication::Complete)?;
    let custody = capture(&fixture);
    custody.checked()?;
    let mut future = fixture.head.clone();
    future.tick.0 += 1;
    future.cursor.sequence += 1;
    future.refresh_hash();
    let dig = fixture
        .plan
        .steps
        .iter()
        .find(|step| matches!(step.action, Action::DesignateDig { .. }))
        .ok_or_else(|| corrupt("fixture lacks dig"))?;
    fixture.store.persist_progress(
        "starter_fortress",
        &future,
        &[DurableStepUpdate {
            plan_digest: fixture.plan.digest,
            step: dig.id.get(),
            state: "failed".to_owned(),
        }],
        &[],
    )?;
    assert!(capture(&fixture).checked().is_err());
    let reset = MemoryAdapter::recovered(fixture.recovered.clone())?;
    assert!(
        custody
            .validate(
                &fixture.plan,
                fixture.lineage.active_anchor(),
                reset.snapshot().anchor(),
            )
            .is_err()
    );
    Ok(())
}

#[test]
fn carried_monitor_must_bind_the_exact_original_step_and_recovery() -> Result<()> {
    let mut fixture = fixture("monitor", Publication::Complete)?;
    let custody = capture(&fixture);
    let commit = fixture
        .store
        .commit(fixture.head.fortress_id, fixture.plan.digest)
        .cloned()
        .ok_or_else(|| corrupt("fixture lacks original commit"))?;
    let mut carried = Vec::new();
    server::recover_commit(
        &mut fixture.store,
        &commit,
        &mut carried,
        &fixture.recovered,
    )?;
    assert_eq!(
        carried.len(),
        1,
        "the prior verified pause has no unfinished monitor"
    );
    let row = &carried[0];
    let step = fixture
        .plan
        .steps
        .iter()
        .find(|step| Some(step.id) == row.step)
        .ok_or_else(|| corrupt("carried step missing from plan"))?;
    let receipt = custody
        .checked()?
        .receipts
        .iter()
        .find(|receipt| receipt.step == step.id)
        .ok_or_else(|| corrupt("carried step missing from custody"))?;
    let check = |row: &CarriedStep| {
        inspect_carried(
            row,
            receipt,
            step,
            fixture.lineage.active_anchor(),
            fixture.recovered.anchor(),
            &mut Actions::default(),
        )
    };
    check(row)?;
    let mut changed = row.clone();
    changed.proof = Predicate::True;
    assert!(check(&changed).is_err());
    let mut changed = row.clone();
    changed.monitor = None;
    assert!(check(&changed).is_err());
    let mut changed = row.clone();
    changed.recorded_state = "verified".to_owned();
    assert!(check(&changed).is_err());
    let mut changed = row.clone();
    changed.state = "verified".to_owned();
    changed.proof_anchor = Some(fixture.recovered.anchor());
    assert!(
        check(&changed).is_err(),
        "current goal matching cannot replace the monitor's unfinished stability",
    );
    Ok(())
}

#[test]
fn archived_verified_proof_rejects_simultaneous_original_failure() -> Result<()> {
    let fixture = fixture("failure", Publication::Complete)?;
    let mut step = fixture
        .plan
        .steps
        .iter()
        .find(|step| matches!(step.action, Action::DesignateDig { .. }))
        .cloned()
        .ok_or_else(|| corrupt("fixture lacks dig"))?;
    // Isolate the terminal/failure rule from physical completion: the original
    // temporal specification must refuse before accepting positive evidence.
    step.postconditions = vec![Predicate::True];
    let spec = step
        .obligation
        .as_mut()
        .ok_or_else(|| corrupt("fixture dig lacks temporal obligation"))?;
    spec.terminal = Predicate::True;
    spec.failure = Some(Predicate::True);
    let error = verify_terminal(&step, &fixture.head)
        .err()
        .ok_or_else(|| corrupt("contradictory terminal proof was accepted"))?;
    assert_eq!(error.code, ErrorCode::CorruptLedger);
    assert!(error.message.contains("failure"));
    Ok(())
}

#[test]
fn archived_verified_proof_cannot_turn_unknown_failure_into_false() -> Result<()> {
    let fixture = fixture("unknown-failure", Publication::Complete)?;
    let mut step = fixture
        .plan
        .steps
        .iter()
        .find(|step| matches!(step.action, Action::DesignateDig { .. }))
        .cloned()
        .ok_or_else(|| corrupt("fixture lacks dig"))?;
    let unknown = Predicate::FieldCompare {
        entity_id: effects::created_entity_id(&step.idempotency_key, 0),
        field: "unobserved_original_failure".to_owned(),
        op: dfmcp_world::CompareOp::Eq,
        value: dfmcp_world::Value::Bool(true),
    };
    assert_eq!(
        PredicateEvidence::laboratory(&fixture.head)?.evaluate(&unknown)?,
        PredicateTruth::Unknown,
    );
    step.postconditions = vec![Predicate::True];
    let spec = step
        .obligation
        .as_mut()
        .ok_or_else(|| corrupt("fixture dig lacks temporal obligation"))?;
    spec.terminal = Predicate::True;
    spec.failure = Some(unknown);
    let error = verify_terminal(&step, &fixture.head)
        .err()
        .ok_or_else(|| corrupt("unknown failure was treated as established false"))?;
    assert_eq!(error.code, ErrorCode::CorruptLedger);
    assert!(error.message.contains("failure"));
    Ok(())
}
