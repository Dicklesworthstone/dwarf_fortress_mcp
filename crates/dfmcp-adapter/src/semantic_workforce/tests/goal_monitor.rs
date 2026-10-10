use super::*;

use crate::semantic_workforce::{
    WorkforceGoalMonitor, WorkforceGoalProjection, WorkforceGoalResult,
};
use crate::semantic_workforce::projection::LABOR_SOURCE;
use dfmcp_intent::ObligationStatus;
use dfmcp_world::PredicateTruth;
use std::cell::Cell;

fn labor_goal() -> Predicate {
    Predicate::FieldCompare {
        entity_id: EntityId::new(43),
        field: "labor.MINE".to_owned(),
        op: CompareOp::Eq,
        value: Value::Bool(true),
    }
}

fn goal_fixture(obligation: bool) -> Result<Fixture> {
    let mut f = fixture()?;
    let mut observe = f.context.grants[0].clone();
    observe.capability = Capability::Observe;
    f.context.grants.push(observe);
    f.plan.terminal_condition = labor_goal();
    f.plan.steps[0].postconditions = vec![labor_goal()];
    f.plan.steps[0].obligation = obligation.then(|| ObligationSpec {
        terminal: labor_goal(),
        failure: None,
        deadline_tick: GameTick(150),
        poll_interval_ticks: 5,
        stable_for_observations: 2,
    });
    reseal(&mut f.plan);
    Ok(f)
}

fn read_context(f: &Fixture) -> OperationContext {
    let mut c = f.context.clone();
    c.grants.retain(|grant| matches!(grant.capability, Capability::Query | Capability::Observe));
    // WorkBudget requires every dimension, including the unused action bound,
    // to stay nonzero. Lack of write grants is the actual no-mutation authority.
    c
}

fn commit_goal(f: &mut Fixture) -> Result<SemanticWorkforceReview> {
    let reviewed = prepared(f)?;
    let result = f.owner.commit(reviewed.seal(), true, &mut f.observer, &f.context, |binding, c| {
        f.native.connect(binding, c)
    })?;
    assert!(!result.original_goal_proven());
    // The fixture's native receipt has advanced the dispatch fence. Its fresh
    // source is independent: leave actual labor False until explicitly changed.
    f.native.data.borrow_mut().fixture.sequence += 1;
    Ok(reviewed)
}

fn begin(f: &mut Fixture, reviewed: &SemanticWorkforceReview) -> Result<WorkforceGoalMonitor> {
    let c = read_context(f);
    f.owner.begin_goal_monitor(reviewed.seal(), &f.observer.routing_evidence()?, &c)
}

fn native_sample(f: &Fixture, tick: u64, enabled: bool) {
    let mut data = f.native.data.borrow_mut();
    data.fixture.tick = tick;
    data.fixture.citizens[0].labors[0] = u8::from(enabled);
}

fn current_policy(projection: &WorkforceGoalProjection, c: &OperationContext) -> Result<EvidencePolicy> {
    assert_eq!(c.anchor, projection.anchor());
    assert_eq!(c.budget.max_bytes, 1024 * 1024);
    assert!(c.budget.max_wall_millis > 0 && c.budget.max_wall_millis <= 10000);
    Ok(policy(projection.snapshot()))
}

fn poll(f: &mut Fixture, monitor: &mut WorkforceGoalMonitor, sequence: u64) -> Result<WorkforceGoalResult> {
    let c = read_context(f);
    f.owner.poll_original_goal(monitor, ObservationCursor { epoch: 0, sequence }, &c,
        |binding, c| f.native.connect(binding, c), current_policy)
}

fn stable(result: &WorkforceGoalResult) -> u32 {
    match result.progress().obligation.as_ref() {
        Some(ObligationStatus::Active { consecutive_stable_observations, .. }) =>
            *consecutive_stable_observations,
        _ => 0,
    }
}

#[test]
fn applied_history_needs_fresh_true_and_retains_later_false() -> Result<()> {
    let mut f = goal_fixture(false)?;
    let review = commit_goal(&mut f)?;
    let mut monitor = begin(&mut f, &review)?;
    let calls = f.native.data.borrow().calls;
    let first = poll(&mut f, &mut monitor, 1)?;
    assert!(matches!(first.action_result(), SingleLaborResult::Verified { .. }));
    assert_eq!(first.progress().current_goal, PredicateTruth::False);
    assert!(!first.original_goal_proven());
    assert!(!first.semantic_completion_proven());
    native_sample(&f, 125, true);
    let achieved = poll(&mut f, &mut monitor, 2)?;
    assert!(achieved.original_goal_proven());
    assert!(achieved.semantic_completion_proven());
    let first_anchor = achieved.progress().first_satisfied_anchor;
    native_sample(&f, 130, false);
    let lost = poll(&mut f, &mut monitor, 3)?;
    assert_eq!(lost.progress().current_goal, PredicateTruth::False);
    assert!(!lost.original_goal_proven());
    assert_eq!(lost.progress().first_satisfied_anchor, first_anchor);
    assert_eq!(monitor.first_satisfied_anchor(), first_anchor);
    let after = f.native.data.borrow().calls;
    assert_eq!(after[0], calls[0] + 3);
    assert_eq!(&after[1..], &calls[1..]);
    Ok(())
}

#[test]
fn fresh_goal_true_cannot_resolve_unknown_native_action() -> Result<()> {
    let mut f = goal_fixture(false)?;
    f.native.data.borrow_mut().unknown = true;
    let review = commit_goal(&mut f)?;
    let mut monitor = begin(&mut f, &review)?;
    native_sample(&f, 125, true);
    let before = f.native.data.borrow().calls;
    let result = poll(&mut f, &mut monitor, 1)?;
    assert!(result.original_goal_proven());
    assert!(!result.semantic_completion_proven());
    assert_eq!(result.native_phase(), Some(AssignmentPhase::Unknown));
    assert_eq!(result.action_result(), &SingleLaborResult::Unverified);
    assert_eq!(&f.native.data.borrow().calls[1..], &before[1..]);
    Ok(())
}

#[test]
fn every_credited_sample_requires_all_original_goal_parts() -> Result<()> {
    for changed in ["postcondition", "plan_terminal", "obligation"] {
        let mut f = goal_fixture(true)?;
        f.plan.terminal_condition = Predicate::Paused(true);
        let spec = f.plan.steps[0].obligation.as_mut().ok_or_else(|| invalid("missing spec"))?;
        spec.terminal = Predicate::FieldCompare {
            entity_id: EntityId::new(43), field: "workforce_eligible".to_owned(),
            op: CompareOp::Eq, value: Value::Bool(true),
        };
        reseal(&mut f.plan);
        let review = commit_goal(&mut f)?;
        let mut monitor = begin(&mut f, &review)?;
        native_sample(&f, 125, true);
        assert_eq!(stable(&poll(&mut f, &mut monitor, 1)?), 1);
        {
            let mut data = f.native.data.borrow_mut();
            data.fixture.tick = 127;
            match changed {
                "postcondition" => data.fixture.citizens[0].labors[0] = 0,
                "plan_terminal" => data.fixture.paused = false,
                _ => data.fixture.citizens[0].eligible = false,
            }
        }
        let contradicted = poll(&mut f, &mut monitor, 2)?;
        assert_eq!(contradicted.progress().current_goal, PredicateTruth::False);
        assert_eq!(stable(&contradicted), 0);
        {
            let mut data = f.native.data.borrow_mut();
            data.fixture.tick = 130;
            data.fixture.citizens[0].labors[0] = 1;
            data.fixture.paused = true;
            data.fixture.citizens[0].eligible = true;
        }
        assert_eq!(stable(&poll(&mut f, &mut monitor, 3)?), 1);
        native_sample(&f, 135, true);
        assert!(poll(&mut f, &mut monitor, 4)?.original_goal_proven());
    }
    Ok(())
}

#[test]
fn absent_labor_authority_is_unknown_and_resets_stability() -> Result<()> {
    let mut f = goal_fixture(true)?;
    let review = commit_goal(&mut f)?;
    let mut monitor = begin(&mut f, &review)?;
    native_sample(&f, 125, true);
    assert_eq!(stable(&poll(&mut f, &mut monitor, 1)?), 1);
    native_sample(&f, 130, true);
    let c = read_context(&f);
    let unknown = f.owner.poll_original_goal(&mut monitor, ObservationCursor { epoch: 0, sequence: 2 },
        &c, |binding, c| f.native.connect(binding, c), |projection, c| {
            let mut out = current_policy(projection, c)?;
            out.sources.retain(|source| !matches!(source,
                EvidenceSource::Observed { field, .. } if field == LABOR_SOURCE));
            Ok(out)
        })?;
    assert_eq!(unknown.progress().current_goal, PredicateTruth::Unknown);
    assert_eq!(stable(&unknown), 0);
    assert!(!unknown.original_goal_proven());
    native_sample(&f, 135, true);
    assert_eq!(stable(&poll(&mut f, &mut monitor, 3)?), 1);
    native_sample(&f, 140, true);
    assert!(poll(&mut f, &mut monitor, 4)?.original_goal_proven());
    Ok(())
}

#[test]
fn failed_acquisition_and_final_policy_denial_erase_only_unfinished_streaks() -> Result<()> {
    for final_policy_failure in [false, true] {
        let mut f = goal_fixture(true)?;
        let review = commit_goal(&mut f)?;
        let mut monitor = begin(&mut f, &review)?;
        native_sample(&f, 125, true);
        assert_eq!(stable(&poll(&mut f, &mut monitor, 1)?), 1);
        native_sample(&f, 130, true);
        let c = read_context(&f);
        let policies = Cell::new(0usize);
        if !final_policy_failure { f.native.data.borrow_mut().fail_connect = true; }
        let failed = f.owner.poll_original_goal(&mut monitor,
            ObservationCursor { epoch: 0, sequence: 2 }, &c,
            |binding, c| f.native.connect(binding, c), |projection, c| {
                policies.set(policies.get() + 1);
                if final_policy_failure && policies.get() == 2 {
                    Err(error(ErrorCode::CapabilityDenied, "revoked before publication"))
                } else { current_policy(projection, c) }
            });
        assert!(failed.is_err());
        assert!(monitor.latest().is_none());
        assert!(monitor.first_satisfied_anchor().is_none());
        f.native.data.borrow_mut().fail_connect = false;
        native_sample(&f, 135, true);
        assert_eq!(stable(&poll(&mut f, &mut monitor, 2)?), 1);
        native_sample(&f, 140, true);
        assert!(poll(&mut f, &mut monitor, 3)?.original_goal_proven());
    }
    Ok(())
}

#[test]
fn duplicate_ticks_and_duplicate_anchors_never_add_stability() -> Result<()> {
    let mut f = goal_fixture(true)?;
    let review = commit_goal(&mut f)?;
    let mut monitor = begin(&mut f, &review)?;
    native_sample(&f, 125, true);
    assert_eq!(stable(&poll(&mut f, &mut monitor, 1)?), 1);
    assert_eq!(stable(&poll(&mut f, &mut monitor, 2)?), 1);
    assert_eq!(stable(&poll(&mut f, &mut monitor, 2)?), 1);
    native_sample(&f, 130, true);
    assert!(poll(&mut f, &mut monitor, 3)?.original_goal_proven());
    native_sample(&f, 135, false);
    let result = poll(&mut f, &mut monitor, 4)?;
    assert_eq!(result.progress().current_goal, PredicateTruth::False);
    assert!(matches!(result.progress().obligation, Some(ObligationStatus::Fulfilled { .. })));
    assert!(!result.original_goal_proven());
    assert!(result.progress().first_satisfied_anchor.is_some());
    Ok(())
}

#[test]
fn canonical_gaps_and_forks_cannot_bridge_temporal_proof() -> Result<()> {
    let mut f = goal_fixture(true)?;
    let review = commit_goal(&mut f)?;
    let mut monitor = begin(&mut f, &review)?;
    native_sample(&f, 125, true);
    assert_eq!(stable(&poll(&mut f, &mut monitor, 1)?), 1);
    native_sample(&f, 130, true);
    let gap = poll(&mut f, &mut monitor, 3)?;
    assert!(!gap.progress().continuous);
    assert_eq!(stable(&gap), 1);
    native_sample(&f, 130, false);
    assert!(poll(&mut f, &mut monitor, 3).is_err());
    assert!(monitor.latest().is_none());
    native_sample(&f, 135, true);
    assert_eq!(stable(&poll(&mut f, &mut monitor, 4)?), 1);
    native_sample(&f, 140, true);
    assert!(poll(&mut f, &mut monitor, 5)?.original_goal_proven());
    Ok(())
}

struct CaptureSource {
    native: Native,
    replacement: Option<WorkforceCapture>,
    closed: Rc<Cell<usize>>,
}
impl Drop for CaptureSource {
    fn drop(&mut self) { self.closed.set(self.closed.get() + 1); }
}
impl WorkforceSource for CaptureSource {
    fn manifest(&self) -> &WorkforceManifest { self.native.manifest() }
    fn endpoint(&self) -> Option<SocketAddr> { self.native.endpoint() }
    fn observe(&mut self, ids: &[u32], c: &OperationContext) -> Result<WorkforceCapture> {
        let capture = self.native.observe(ids, c)?;
        Ok(self.replacement.clone().unwrap_or(capture))
    }
    fn prepare(&mut self, _: &AssignmentPlan, _: &OperationContext) -> Result<AssignmentEffect> {
        Err(invalid("goal monitor attempted prepare"))
    }
    fn commit(&mut self, _: &AssignmentPlan, _: &OperationContext) -> Result<AssignmentEffect> {
        Err(invalid("goal monitor attempted commit"))
    }
    fn query(&mut self, _: &AssignmentPlan, _: &OperationContext) -> Result<Option<AssignmentEffect>> {
        Err(invalid("goal monitor attempted receipt reconciliation"))
    }
    fn cancel(&mut self, _: &AssignmentPlan, _: &OperationContext) -> Result<AssignmentEffect> {
        Err(invalid("goal monitor attempted cancellation"))
    }
}

#[test]
fn fresh_source_closes_before_policy_and_facts_keep_original_identity() -> Result<()> {
    let mut f = goal_fixture(false)?;
    let review = commit_goal(&mut f)?;
    let mut monitor = begin(&mut f, &review)?;
    native_sample(&f, 125, true);
    let c = read_context(&f);
    let closed = Rc::new(Cell::new(0usize));
    let issued = Cell::new(0usize);
    let result = f.owner.poll_original_goal(&mut monitor,
        ObservationCursor { epoch: 0, sequence: 1 }, &c,
        |binding, c| Ok(CaptureSource {
            native: f.native.connect(binding, c)?, replacement: None, closed: Rc::clone(&closed),
        }), |projection, c| {
            assert_eq!(closed.get(), 1);
            issued.set(issued.get() + 1);
            let unit = projection.snapshot().graph.entities.get(&EntityId::new(43))
                .ok_or_else(|| invalid("lost canonical unit"))?;
            assert_eq!(unit.generation, 1);
            assert_eq!(unit.fields["raw_unit_id"].known_value(), Some(&Value::I64(42)));
            assert_eq!(unit.fields["historical_figure_id"].known_value(), Some(&Value::U64(142)));
            assert_eq!(unit.fields["labor.MINE"].known_value(), Some(&Value::Bool(true)));
            assert_eq!(unit.fields["labor.MINE"].source_digest, projection.capture().witness());
            assert_eq!(projection.original_anchor(), f.plan.anchor);
            assert_eq!(projection.review_seal(), review.seal());
            assert_eq!(projection.snapshot().graph.entities.len(), 1);
            let evidence = projection.evidence(current_policy(projection, c)?)?;
            assert_eq!(evidence.evaluate(&Predicate::Not(Box::new(Predicate::EntityExists(EntityId::new(999)))))?,
                PredicateTruth::Unknown);
            current_policy(projection, c)
        })?;
    assert!(result.original_goal_proven());
    assert_eq!(issued.get(), 2);
    assert_eq!(closed.get(), 1);
    Ok(())
}

#[test]
fn historical_figure_replacement_fences_original_identity() -> Result<()> {
    let mut f = goal_fixture(false)?;
    let review = commit_goal(&mut f)?;
    let mut monitor = begin(&mut f, &review)?;
    native_sample(&f, 125, true);
    let mut bytes = f.native.data.borrow().fixture.capture()?.canonical_bytes().to_vec();
    let count = bytes.len();
    // Independent fixed selected-citizen record: id u32, historical id u32,
    // eligible byte, and the fixture's two labor bytes.
    bytes[count - 7..count - 3].copy_from_slice(&143u32.to_be_bytes());
    let replacement = WorkforceCapture::decode(&bytes)?;
    let c = read_context(&f);
    let closed = Rc::new(Cell::new(0));
    assert!(f.owner.poll_original_goal(&mut monitor,
        ObservationCursor { epoch: 0, sequence: 1 }, &c,
        |binding, c| Ok(CaptureSource {
            native: f.native.connect(binding, c)?, replacement: Some(replacement), closed: Rc::clone(&closed),
        }), current_policy).is_err());
    assert!(monitor.is_fenced());
    assert!(monitor.latest().is_none());
    let calls = f.native.data.borrow().calls;
    assert!(poll(&mut f, &mut monitor, 2).is_err());
    assert_eq!(f.native.data.borrow().calls, calls);
    Ok(())
}

#[test]
fn changed_epoch_source_and_native_sequence_are_refused() -> Result<()> {
    let mut f = goal_fixture(false)?;
    let review = commit_goal(&mut f)?;
    let mut monitor = begin(&mut f, &review)?;
    let c = read_context(&f);
    let before = f.native.data.borrow().calls;
    assert!(f.owner.poll_original_goal(&mut monitor,
        ObservationCursor { epoch: 1, sequence: 1 }, &c,
        |binding, c| f.native.connect(binding, c), current_policy).is_err());
    assert_eq!(f.native.data.borrow().calls, before);
    f.native.data.borrow_mut().fixture.generation += 1;
    assert!(poll(&mut f, &mut monitor, 1).is_err());
    f.native.data.borrow_mut().fixture.generation -= 1;
    // An Applied receipt established a sequence floor one above the original
    // capture. It does not supply labor evidence, but cannot be silently rewound.
    f.native.data.borrow_mut().fixture.sequence -= 1;
    assert!(poll(&mut f, &mut monitor, 1).is_err());
    assert!(monitor.latest().is_none());
    Ok(())
}

#[test]
fn selected_capture_rejects_complete_domains_and_wrong_source_policies() -> Result<()> {
    for mutation in 0..4 {
        let mut f = goal_fixture(false)?;
        let review = commit_goal(&mut f)?;
        let mut monitor = begin(&mut f, &review)?;
        native_sample(&f, 125, true);
        let c = read_context(&f);
        assert!(f.owner.poll_original_goal(&mut monitor,
            ObservationCursor { epoch: 0, sequence: 1 }, &c,
            |binding, c| f.native.connect(binding, c), |projection, c| {
                let mut out = current_policy(projection, c)?;
                match mutation {
                    0 => out.all_entities = EvidenceCoverage::Complete,
                    1 => out.all_edges = EvidenceCoverage::Observed,
                    2 => { out.sources.insert(EvidenceSource::Observed {
                        field: LABOR_SOURCE.to_owned(), source_digest: Digest32::of_bytes(b"wrong source"),
                    }); }
                    _ => out.anchor = Some(f.plan.anchor),
                }
                Ok(out)
            }).is_err());
        assert!(monitor.latest().is_none());
        assert!(monitor.first_satisfied_anchor().is_none());
    }
    Ok(())
}

#[test]
fn observe_revocation_at_fresh_tick_cannot_publish_or_renew_scope() -> Result<()> {
    let mut f = goal_fixture(true)?;
    let review = commit_goal(&mut f)?;
    let mut monitor = begin(&mut f, &review)?;
    native_sample(&f, 125, true);
    assert_eq!(stable(&poll(&mut f, &mut monitor, 1)?), 1);
    for grant in &mut f.context.grants {
        if grant.capability == Capability::Observe { grant.expires_at_tick = Some(GameTick(130)); }
    }
    native_sample(&f, 131, true);
    let calls = f.native.data.borrow().calls;
    assert!(poll(&mut f, &mut monitor, 2).is_err());
    assert_eq!(f.native.data.borrow().calls[0], calls[0] + 1);
    assert!(monitor.latest().is_none());
    // The caller's old tick must not let a second read bypass the observed floor.
    let calls = f.native.data.borrow().calls;
    assert!(poll(&mut f, &mut monitor, 2).is_err());
    assert_eq!(f.native.data.borrow().calls, calls);
    for grant in &mut f.context.grants {
        if grant.capability == Capability::Observe { grant.expires_at_tick = None; }
    }
    native_sample(&f, 135, true);
    assert_eq!(stable(&poll(&mut f, &mut monitor, 2)?), 1);
    Ok(())
}

#[test]
fn fixed_deadline_exact_endpoint_and_preparation_expiry_stay_distinct() -> Result<()> {
    let mut f = goal_fixture(true)?;
    let review = commit_goal(&mut f)?;
    let mut monitor = begin(&mut f, &review)?;
    native_sample(&f, 145, true);
    assert_eq!(stable(&poll(&mut f, &mut monitor, 1)?), 1);
    native_sample(&f, 150, true);
    assert!(poll(&mut f, &mut monitor, 2)?.original_goal_proven());

    let mut late = goal_fixture(true)?;
    let review = commit_goal(&mut late)?;
    let mut monitor = begin(&mut late, &review)?;
    native_sample(&late, 151, true);
    let failed = poll(&mut late, &mut monitor, 1)?;
    assert_eq!(failed.progress().current_goal, PredicateTruth::True);
    assert!(matches!(failed.progress().obligation, Some(ObligationStatus::Failed { .. })));
    assert!(!failed.original_goal_proven());

    let mut timeless = goal_fixture(false)?;
    let review = commit_goal(&mut timeless)?;
    let mut monitor = begin(&mut timeless, &review)?;
    native_sample(&timeless, timeless.plan.expires_at_tick.get() + 100, true);
    assert!(poll(&mut timeless, &mut monitor, 1)?.original_goal_proven());
    Ok(())
}

#[test]
fn budget_refusal_precedes_contact_and_resets_unfinished_stability() -> Result<()> {
    let mut f = goal_fixture(true)?;
    let review = commit_goal(&mut f)?;
    let mut monitor = begin(&mut f, &review)?;
    native_sample(&f, 125, true);
    assert_eq!(stable(&poll(&mut f, &mut monitor, 1)?), 1);
    let mut c = read_context(&f);
    c.budget.max_output_tokens = 1;
    let before = f.native.data.borrow().calls;
    assert!(f.owner.poll_original_goal(&mut monitor,
        ObservationCursor { epoch: 0, sequence: 2 }, &c,
        |binding, c| f.native.connect(binding, c), current_policy).is_err());
    assert_eq!(f.native.data.borrow().calls, before);
    assert!(monitor.latest().is_none());
    native_sample(&f, 130, true);
    assert_eq!(stable(&poll(&mut f, &mut monitor, 2)?), 1);
    Ok(())
}

#[test]
fn query_observe_only_recovery_starts_new_stability_without_renewing_deadline() -> Result<()> {
    let mut f = goal_fixture(true)?;
    let review = commit_goal(&mut f)?;
    let mut monitor = begin(&mut f, &review)?;
    native_sample(&f, 125, true);
    poll(&mut f, &mut monitor, 1)?;
    native_sample(&f, 130, true);
    let achieved = poll(&mut f, &mut monitor, 2)?;
    assert!(achieved.original_goal_proven());
    let mut c = read_context(&f);
    c.anchor = achieved.progress().anchor;
    let journal = WorkforceJournal::open(f.native.journal.clone(), &c, WorkforceMode::Recover, None)?;
    let (session, view) = WorkforceSession::new(journal, &c)?;
    let associations = AssociationStore::open(
        f.native.associations.clone(), view.id, false, true, &c,
    )?;
    let mut recovered = SemanticWorkforceSession::new(session, associations, &c)?;
    let reattached = recovered.reattach(f.plan.clone(), &c)?;
    let mut new_monitor = recovered.begin_goal_monitor(
        reattached.seal(), &f.observer.routing_evidence()?, &c,
    )?;
    assert!(new_monitor.latest().is_none());
    assert!(new_monitor.first_satisfied_anchor().is_none());
    assert_eq!(new_monitor.original_plan().steps[0].obligation.as_ref().map(|s| s.deadline_tick),
        Some(GameTick(150)));
    native_sample(&f, 140, true);
    let calls = f.native.data.borrow().calls;
    // Restart can reset history but cannot reuse an already allocated canonical
    // cursor, even when the new context is later replaced by an old one.
    assert!(recovered.poll_original_goal(&mut new_monitor,
        ObservationCursor { epoch: 0, sequence: 2 }, &c,
        |binding, c| f.native.connect(binding, c), current_policy).is_err());
    let old_context = read_context(&f);
    assert!(recovered.poll_original_goal(&mut new_monitor,
        ObservationCursor { epoch: 0, sequence: 1 }, &old_context,
        |binding, c| f.native.connect(binding, c), current_policy).is_err());
    assert_eq!(f.native.data.borrow().calls, calls);
    let result = recovered.poll_original_goal(&mut new_monitor,
        ObservationCursor { epoch: 0, sequence: 3 }, &c,
        |binding, c| f.native.connect(binding, c), current_policy)?;
    assert_eq!(stable(&result), 1);
    assert!(!result.original_goal_proven());
    assert_eq!(&f.native.data.borrow().calls[1..], &calls[1..]);
    native_sample(&f, 151, true);
    let expired = recovered.poll_original_goal(&mut new_monitor,
        ObservationCursor { epoch: 0, sequence: 4 }, &c,
        |binding, c| f.native.connect(binding, c), current_policy)?;
    assert!(matches!(expired.progress().obligation, Some(ObligationStatus::Failed { .. })));
    assert!(!expired.original_goal_proven());
    Ok(())
}


#[test]
fn current_failure_after_historical_fulfillment_blocks_current_proof() -> Result<()> {
    let mut f = goal_fixture(true)?;
    let spec = f.plan.steps[0].obligation.as_mut().ok_or_else(|| invalid("missing spec"))?;
    spec.failure = Some(Predicate::FieldCompare {
        entity_id: EntityId::new(43), field: "workforce_eligible".to_owned(),
        op: CompareOp::Eq, value: Value::Bool(false),
    });
    reseal(&mut f.plan);
    let review = commit_goal(&mut f)?;
    let mut monitor = begin(&mut f, &review)?;
    native_sample(&f, 125, true);
    poll(&mut f, &mut monitor, 1)?;
    native_sample(&f, 130, true);
    let achieved = poll(&mut f, &mut monitor, 2)?;
    assert!(achieved.original_goal_proven());
    native_sample(&f, 135, true);
    f.native.data.borrow_mut().fixture.citizens[0].eligible = false;
    let contradicted = poll(&mut f, &mut monitor, 3)?;
    assert_eq!(contradicted.progress().current_goal, PredicateTruth::True);
    assert_eq!(contradicted.progress().failure_predicate, Some(PredicateTruth::True));
    assert!(matches!(contradicted.progress().obligation, Some(ObligationStatus::Fulfilled { .. })));
    assert_eq!(contradicted.progress().first_satisfied_anchor,
        achieved.progress().first_satisfied_anchor);
    assert!(!contradicted.original_goal_proven());
    assert!(!contradicted.semantic_completion_proven());
    Ok(())
}

#[test]
fn explicit_128_mib_foreground_budget_executes_real_observation_path() -> Result<()> {
    let mut f = goal_fixture(false)?;
    let review = commit_goal(&mut f)?;
    let mut c = read_context(&f);
    c.budget.max_bytes = 128 * 1024 * 1024;
    let mut monitor = f.owner.begin_goal_monitor(
        review.seal(), &f.observer.routing_evidence()?, &c,
    )?;
    native_sample(&f, 125, true);
    let result = f.owner.poll_original_goal(&mut monitor,
        ObservationCursor { epoch: 0, sequence: 1 }, &c,
        |binding, c| f.native.connect(binding, c), current_policy)?;
    assert!(result.semantic_completion_proven());
    Ok(())
}
