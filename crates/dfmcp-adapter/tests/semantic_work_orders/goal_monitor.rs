use super::*;
use std::cell::Cell;
use std::io::Cursor;
use std::time::Instant;

use dfmcp_adapter::live_jobs::{LiveJob, LiveJobObservation};
use dfmcp_adapter::live_jobs_rpc::operations::{OperationsLimits, OperationsRpcClient};
use dfmcp_adapter::live_operations::{LiveItem, LiveOperationsObservation};
use dfmcp_adapter::semantic_work_orders::{
    OperationsGoalSource, WorkOrderGoalMonitor, WorkOrderGoalProjection, WorkOrderGoalResult,
    WorkOrderPhysicalState,
};
use dfmcp_adapter::work_order_progress::rpc::{ProgressRpcClient, ProgressStream};
use dfmcp_adapter::work_order_progress::{ProgressManifest, ProgressObservation, ProgressSource};
use dfmcp_core::MapCoord;
use dfmcp_intent::ObligationStatus;
use dfmcp_world::{CompareOp, EntityKind, PredicateTruth, Value};

fn observed_goal() -> Predicate {
    Predicate::FieldCompare {
        entity_id: EntityId::new(1),
        field: "item_count".into(),
        op: CompareOp::Ge,
        value: Value::U64(1),
    }
}
fn goal_fixture() -> Result<(
    Fixture,
    Session,
    dfmcp_adapter::semantic_work_orders::SemanticWorkOrderReview,
)> {
    goal_fixture_with_state(WorkOrderState::Created)
}
fn goal_fixture_with_state(
    state: WorkOrderState,
) -> Result<(
    Fixture,
    Session,
    dfmcp_adapter::semantic_work_orders::SemanticWorkOrderReview,
)> {
    goal_fixture_with_config(state, |_| {})
}
fn goal_fixture_with_config(
    state: WorkOrderState,
    edit: impl FnOnce(&mut PreparedPlan),
) -> Result<(
    Fixture,
    Session,
    dfmcp_adapter::semantic_work_orders::SemanticWorkOrderReview,
)> {
    let mut f = Fixture::new()?;
    f.native.borrow_mut().commit_state = state;
    f.context.budget.max_bytes = 256 * 1024 * 1024;
    let mut observe = f.context.grants[0].clone();
    observe.capability = Capability::Observe;
    f.context.grants.push(observe);
    let mut p = f.original.clone();
    p.terminal_condition = observed_goal();
    p.steps[0].postconditions = vec![observed_goal()];
    p.steps[0]
        .obligation
        .as_mut()
        .ok_or_else(rejected)?
        .terminal = observed_goal();
    edit(&mut p);
    f.original = reseal(p);
    let mut session = f.session(true, JournalMode::Control, false)?;
    let review = session.observe(f.original.clone(), &mut f.observer, &f.context)?;
    session.prepare(review.seal(), &mut f.observer, &f.context)?;
    session.commit(review.seal(), true, &mut f.observer, &f.context)?;
    Ok((f, session, review))
}

#[test]
fn every_original_postcondition_plan_terminal_and_obligation_terminal_participates_in_proof()
-> Result<()> {
    for term in 0..3 {
        let (f, mut session, review) =
            goal_fixture_with_config(WorkOrderState::Created, |p| match term {
                0 => {
                    p.steps[0].postconditions = vec![Predicate::FieldCompare {
                        entity_id: EntityId::new(1),
                        field: "item_count".into(),
                        op: CompareOp::Ge,
                        value: Value::U64(2),
                    }]
                }
                1 => {
                    p.terminal_condition = Predicate::FieldCompare {
                        entity_id: EntityId::new(1),
                        field: "item_count".into(),
                        op: CompareOp::Ge,
                        value: Value::U64(2),
                    }
                }
                _ => {
                    if let Some(spec) = &mut p.steps[0].obligation {
                        spec.terminal = Predicate::FieldCompare {
                            entity_id: EntityId::new(1),
                            field: "item_count".into(),
                            op: CompareOp::Ge,
                            value: Value::U64(2),
                        };
                    }
                }
            })?;
        let mut monitor = session.begin_goal_monitor(review.seal(), &f.context)?;
        for n in 1..=2 {
            let result = poll(
                &f,
                &mut session,
                &mut monitor,
                n,
                10 * n,
                0,
                true,
                false,
                |p| Ok(grant(p)),
            )?;
            assert_eq!(
                result.progress().current_goal,
                PredicateTruth::False,
                "term {term}"
            );
            assert!(!result.original_goal_proven());
            assert!(!result.semantic_completion_proven());
        }
    }
    Ok(())
}
fn operations(queue: &WorkOrderObservation, items: bool, jobs: bool) -> LiveOperationsObservation {
    LiveOperationsObservation {
        jobs: LiveJobObservation {
            bridge_generation: 601,
            df_version: "0.51.11".into(),
            dfhack_version: "0.51.11-r1".into(),
            year: (queue.tick() / 403_200) as u32,
            year_tick: (queue.tick() % 403_200) as u32,
            paused: true,
            site_id: queue.site_id() as i32,
            world_folder: queue.world_folder().into(),
            next_job_id: 100,
            jobs: if jobs {
                vec![LiveJob {
                    native_id: 3,
                    job_type: 77,
                    type_key: "ConstructBed".into(),
                    reaction: String::new(),
                    suspended: false,
                    repeating: false,
                    position: MapCoord::new(1, 1, 1),
                    worker_native_id: None,
                    holder_native_id: None,
                    completion_timer: 20,
                    attached_item_count: 0,
                    required_item_filter_count: 0,
                }]
            } else {
                Vec::new()
            },
        },
        next_building_id: 100,
        next_item_id: 100,
        buildings: Vec::new(),
        items: if items {
            vec![LiveItem {
                native_id: 2,
                item_type: 5,
                type_key: "BED".into(),
                subtype: -1,
                material_type: 420,
                material_index: 0,
                stack_size: 1,
                raw_position: MapCoord::new(1, 1, 1),
                flags: 64,
                container_native_id: None,
                holder_building_native_id: None,
            }]
        } else {
            Vec::new()
        },
        attachments: Vec::new(),
    }
}
fn progress(
    queue: &WorkOrderObservation,
    id: u32,
    sequence: u64,
    remaining: i32,
    recipe: u8,
    status: u32,
) -> Result<ProgressObservation> {
    let mut bytes = b"DFMWP012".to_vec();
    for n in [501u64, sequence, queue.tick()] {
        bytes.extend_from_slice(&n.to_be_bytes());
    }
    for n in [
        queue.site_id(),
        queue.next_order_id(),
        queue.order_ids().len() as u32,
    ] {
        bytes.extend_from_slice(&n.to_be_bytes());
    }
    bytes.push(1);
    text(&mut bytes, queue.world_folder());
    bytes.extend_from_slice(&1u32.to_be_bytes());
    bytes.extend_from_slice(&id.to_be_bytes());
    let present = queue.order_ids().contains(&id);
    bytes.push(u8::from(present));
    if present {
        bytes.extend_from_slice(&77i32.to_be_bytes());
        bytes.push(recipe);
        for n in [remaining, 5] {
            bytes.extend_from_slice(&n.to_be_bytes());
        }
        bytes.extend_from_slice(&status.to_be_bytes());
        for n in [0i32, -1, 1, 0, 0] {
            bytes.extend_from_slice(&n.to_be_bytes());
        }
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(&0u32.to_be_bytes());
        text(
            &mut bytes,
            match recipe {
                2 => "ConstructDoor",
                3 => "ConstructTable",
                4 => "ConstructThrone",
                _ => "ConstructBed",
            },
        );
        text(&mut bytes, "");
    }
    ProgressObservation::decode(&bytes, &[id])
}
fn grant(projection: &WorkOrderGoalProjection) -> EvidencePolicy {
    let mut policy = EvidencePolicy::at(projection.anchor());
    policy.paused = true;
    policy.sources = projection.observed_sources();
    for kind in [
        EntityKind::Fortress,
        EntityKind::Job,
        EntityKind::Item,
        EntityKind::Building,
    ] {
        policy.entity_kinds.insert(kind, EvidenceCoverage::Complete);
    }
    policy
        .entity_kinds
        .insert(EntityKind::WorkOrder, EvidenceCoverage::Observed);
    policy
}
struct ProgressFixture {
    manifest: ProgressManifest,
    captures: Vec<ProgressObservation>,
    closed: Rc<Cell<usize>>,
}
impl ProgressSource for ProgressFixture {
    fn manifest(&self) -> &ProgressManifest {
        &self.manifest
    }
    fn fence(&mut self) {}
    fn read(&mut self, ids: &[u32], timeout: Duration) -> Result<ProgressObservation> {
        assert!(timeout <= Duration::from_secs(60));
        if self.captures.is_empty() {
            return Err(rejected());
        }
        let raw = self.captures.remove(0);
        ProgressObservation::decode(raw.canonical_bytes(), ids)
    }
}
impl Drop for ProgressFixture {
    fn drop(&mut self) {
        self.closed.set(self.closed.get() + 1);
    }
}
struct OperationsFixture {
    raw: LiveOperationsObservation,
    closed: Rc<Cell<usize>>,
}
impl OperationsGoalSource for OperationsFixture {
    fn read_operations(&mut self, c: &OperationContext) -> Result<LiveOperationsObservation> {
        assert!(c.budget.max_bytes <= 16 * 1024 * 1024);
        let raw = self.raw.encode_payload()?;
        LiveOperationsObservation::decode_payload(
            &raw,
            self.raw.jobs.bridge_generation,
            self.raw.jobs.df_version.clone(),
            self.raw.jobs.dfhack_version.clone(),
        )
    }
}
impl Drop for OperationsFixture {
    fn drop(&mut self) {
        self.closed.set(self.closed.get() + 1);
    }
}
fn native_progress(captures: Vec<ProgressObservation>, closed: Rc<Cell<usize>>) -> ProgressFixture {
    ProgressFixture {
        manifest: ProgressManifest {
            generation: 501,
            df_version: "0.51.11".into(),
            dfhack_version: "0.51.11-r1".into(),
        },
        captures,
        closed,
    }
}
fn advance(f: &Fixture, relative_tick: u64) -> Result<WorkOrderObservation> {
    let old = f.native.borrow().queue.clone();
    let queue = changed_clock(
        &old,
        old.generation(),
        old.sequence(),
        f.original.anchor.tick.get() + relative_tick,
    )?;
    f.native.borrow_mut().queue = queue.clone();
    Ok(queue)
}
#[allow(clippy::too_many_arguments)]
fn poll<A>(
    f: &Fixture,
    session: &mut Session,
    monitor: &mut WorkOrderGoalMonitor,
    n: u64,
    tick: u64,
    remaining: i32,
    has_items: bool,
    has_jobs: bool,
    mut policy: A,
) -> Result<WorkOrderGoalResult>
where
    A: FnMut(&WorkOrderGoalProjection) -> Result<EvidencePolicy>,
{
    let queue = advance(f, tick)?;
    // The fixture always creates the original next-ID exactly once.
    let expected = queue.next_order_id() - 1;
    let captures = vec![
        progress(
            &queue,
            expected,
            2 * n - 1,
            remaining,
            1,
            if remaining == 0 { 1 } else { 3 },
        )?,
        progress(
            &queue,
            expected,
            2 * n,
            remaining,
            1,
            if remaining == 0 { 1 } else { 3 },
        )?,
    ];
    let closed = Rc::new(Cell::new(0));
    let progress_source = native_progress(captures, Rc::clone(&closed));
    let operations_source = OperationsFixture {
        raw: operations(&queue, has_items, has_jobs),
        closed: Rc::clone(&closed),
    };
    session.poll_original_goal(
        monitor,
        ObservationCursor {
            epoch: f.context.anchor.cursor.epoch,
            sequence: f.context.anchor.cursor.sequence + n,
        },
        &f.context,
        |_| Ok(progress_source),
        |_| Ok(operations_source),
        |projection, c| {
            assert_eq!(
                closed.get(),
                2,
                "fresh sources must close before source policy is issued"
            );
            assert_eq!(c.anchor, projection.anchor());
            policy(projection)
        },
    )
}

#[test]
fn independently_true_original_goal_never_resolves_unknown_native_creation() -> Result<()> {
    let (f, mut session, review) = goal_fixture_with_state(WorkOrderState::Unknown)?;
    let mut monitor = session.begin_goal_monitor(review.seal(), &f.context)?;
    for n in 1..=2 {
        let queue = advance(&f, 10 * n)?;
        let id = review.native_plan().observation().next_order_id();
        let captures = vec![
            progress(&queue, id, 2 * n - 1, 0, 1, 1)?,
            progress(&queue, id, 2 * n, 0, 1, 1)?,
        ];
        let closed = Rc::new(Cell::new(0));
        let source = native_progress(captures, Rc::clone(&closed));
        let result = session.poll_original_goal(
            &mut monitor,
            ObservationCursor {
                epoch: f.context.anchor.cursor.epoch,
                sequence: f.context.anchor.cursor.sequence + n,
            },
            &f.context,
            |_| Ok(source),
            |_| {
                Ok(OperationsFixture {
                    raw: operations(&queue, true, false),
                    closed,
                })
            },
            |p, _| Ok(grant(p)),
        )?;
        assert_eq!(result.original_goal_proven(), n == 2);
        assert!(!result.semantic_completion_proven());
        assert_eq!(result.creation_result(), &NativeOrderCreation::Unverified);
        assert_eq!(result.native_state(), CreationState::Indeterminate);
        assert!(!result.progress().receipt_linked);
    }
    assert_eq!(f.native.borrow().commits, 1);
    assert_eq!(f.native.borrow().queries, 0);
    Ok(())
}

#[test]
fn disappearance_is_unknown_and_reappearance_cannot_reinherit_original_receipt_identity()
-> Result<()> {
    let (f, mut session, review) = goal_fixture()?;
    let mut monitor = session.begin_goal_monitor(review.seal(), &f.context)?;
    poll(&f, &mut session, &mut monitor, 1, 10, 0, true, false, |p| {
        Ok(grant(p))
    })?;
    let old = advance(&f, 20)?;
    let mut bytes = old.canonical_bytes().to_vec();
    let count = 43 + old.world_folder().len();
    bytes[count..count + 4].copy_from_slice(&((old.order_ids().len() - 1) as u32).to_be_bytes());
    bytes.truncate(bytes.len() - 4);
    f.native.borrow_mut().queue = WorkOrderObservation::decode(&bytes)?;
    let gone = poll(&f, &mut session, &mut monitor, 2, 20, 0, true, false, |p| {
        Ok(grant(p))
    })?;
    assert!(gone.original_goal_proven());
    assert!(!gone.semantic_completion_proven());
    assert!(!gone.progress().receipt_linked);
    f.native.borrow_mut().queue = old;
    assert!(
        poll(&f, &mut session, &mut monitor, 3, 30, 0, true, false, |p| {
            Ok(grant(p))
        })
        .is_err()
    );
    assert!(monitor.is_fenced());
    Ok(())
}

#[test]
fn original_goal_reaches_proof_only_after_real_items_stability_and_complete_physical_quiet()
-> Result<()> {
    let (f, mut session, review) = goal_fixture()?;
    let mut monitor = session.begin_goal_monitor(review.seal(), &f.context)?;
    let initial = poll(&f, &mut session, &mut monitor, 1, 10, 3, true, true, |p| {
        Ok(grant(p))
    })?;
    assert_eq!(initial.progress().current_goal, PredicateTruth::True);
    assert!(!initial.original_goal_proven());
    assert!(matches!(
        initial.progress().physical_work,
        WorkOrderPhysicalState::Outstanding { remaining: 3, .. }
    ));
    let done = poll(&f, &mut session, &mut monitor, 2, 20, 0, true, false, |p| {
        Ok(grant(p))
    })?;
    assert!(done.original_goal_proven());
    assert!(done.semantic_completion_proven());
    assert_eq!(monitor.original_plan(), &f.original);
    assert_eq!(
        done.progress().first_satisfied_anchor,
        Some(done.progress().anchor)
    );
    assert_eq!(f.native.borrow().commits, 1);
    assert_eq!(f.native.borrow().prepares, 1);
    assert_eq!(f.native.borrow().queries, 0);
    Ok(())
}

#[test]
fn zero_order_does_not_prove_original_goods_or_nonempty_job_quiescence() -> Result<()> {
    let (f, mut session, review) = goal_fixture()?;
    let mut monitor = session.begin_goal_monitor(review.seal(), &f.context)?;
    let zero = poll(
        &f,
        &mut session,
        &mut monitor,
        1,
        10,
        0,
        false,
        false,
        |p| Ok(grant(p)),
    )?;
    assert_eq!(zero.progress().current_goal, PredicateTruth::False);
    assert!(zero.progress().physical_work.is_quiescent());
    assert!(!zero.semantic_completion_proven());
    poll(&f, &mut session, &mut monitor, 2, 20, 0, true, true, |p| {
        Ok(grant(p))
    })?;
    let waiting = poll(&f, &mut session, &mut monitor, 3, 30, 0, true, true, |p| {
        Ok(grant(p))
    })?;
    assert!(waiting.original_goal_proven());
    assert!(!waiting.semantic_completion_proven());
    assert!(matches!(
        waiting.progress().physical_work,
        WorkOrderPhysicalState::Unknown { .. }
    ));
    Ok(())
}

#[test]
fn later_false_and_unknown_goals_preserve_first_achievement_without_claiming_current_success()
-> Result<()> {
    let (f, mut session, review) = goal_fixture()?;
    let mut monitor = session.begin_goal_monitor(review.seal(), &f.context)?;
    poll(&f, &mut session, &mut monitor, 1, 10, 0, true, false, |p| {
        Ok(grant(p))
    })?;
    let done = poll(&f, &mut session, &mut monitor, 2, 20, 0, true, false, |p| {
        Ok(grant(p))
    })?;
    let proof = done.progress().first_satisfied_anchor;
    let consumed = poll(
        &f,
        &mut session,
        &mut monitor,
        3,
        30,
        0,
        false,
        false,
        |p| Ok(grant(p)),
    )?;
    assert_eq!(consumed.progress().current_goal, PredicateTruth::False);
    assert!(!consumed.semantic_completion_proven());
    assert!(matches!(
        consumed.progress().obligation,
        Some(ObligationStatus::Fulfilled { .. })
    ));
    let unknown = poll(&f, &mut session, &mut monitor, 4, 40, 0, true, false, |p| {
        Ok(EvidencePolicy::at(p.anchor()))
    })?;
    assert_eq!(unknown.progress().current_goal, PredicateTruth::Unknown);
    assert!(!unknown.original_goal_proven());
    assert_eq!(unknown.progress().first_satisfied_anchor, proof);
    Ok(())
}

#[test]
fn missing_counter_status_or_complete_job_authority_cannot_certify_physical_completion()
-> Result<()> {
    for denied in ["counters", "status", "jobs"] {
        let (f, mut session, review) = goal_fixture()?;
        let mut monitor = session.begin_goal_monitor(review.seal(), &f.context)?;
        poll(&f, &mut session, &mut monitor, 1, 10, 0, true, false, |p| {
            Ok(grant(p))
        })?;
        let result = poll(&f, &mut session, &mut monitor, 2, 20, 0, true, false, |p| {
            let mut policy = grant(p);
            if denied == "jobs" {
                policy
                    .entity_kinds
                    .insert(EntityKind::Job, EvidenceCoverage::Observed);
            } else {
                policy.sources.retain(|source| !matches!(source, EvidenceSource::Observed { field, .. } if field.ends_with(denied)));
            }
            Ok(policy)
        })?;
        assert!(result.original_goal_proven());
        assert!(!result.semantic_completion_proven(), "{denied}");
    }
    Ok(())
}

#[test]
fn failed_final_policy_erases_pending_stability_and_does_not_publish_first_achievement()
-> Result<()> {
    let (f, mut session, review) = goal_fixture()?;
    let mut monitor = session.begin_goal_monitor(review.seal(), &f.context)?;
    poll(&f, &mut session, &mut monitor, 1, 10, 0, true, false, |p| {
        Ok(grant(p))
    })?;
    let mut checks = 0;
    assert!(
        poll(&f, &mut session, &mut monitor, 2, 20, 0, true, false, |p| {
            checks += 1;
            if checks == 2 {
                Err(rejected())
            } else {
                Ok(grant(p))
            }
        })
        .is_err()
    );
    assert!(monitor.latest().is_none());
    assert!(monitor.first_satisfied_anchor().is_none());
    let first = poll(&f, &mut session, &mut monitor, 3, 30, 0, true, false, |p| {
        Ok(grant(p))
    })?;
    assert!(!first.original_goal_proven());
    assert!(!first.progress().continuous);
    let next = poll(&f, &mut session, &mut monitor, 4, 40, 0, true, false, |p| {
        Ok(grant(p))
    })?;
    assert!(next.semantic_completion_proven());
    Ok(())
}

#[test]
fn same_tick_and_cursor_gaps_cannot_manufacture_stability() -> Result<()> {
    let (f, mut session, review) = goal_fixture()?;
    let mut monitor = session.begin_goal_monitor(review.seal(), &f.context)?;
    poll(&f, &mut session, &mut monitor, 1, 10, 0, true, false, |p| {
        Ok(grant(p))
    })?;
    let repeated = poll(&f, &mut session, &mut monitor, 2, 10, 0, true, false, |p| {
        Ok(grant(p))
    })?;
    assert!(!repeated.original_goal_proven());
    let gap = poll(&f, &mut session, &mut monitor, 4, 20, 0, true, false, |p| {
        Ok(grant(p))
    })?;
    assert!(!gap.original_goal_proven());
    assert!(!gap.progress().continuous);
    let after = poll(&f, &mut session, &mut monitor, 5, 30, 0, true, false, |p| {
        Ok(grant(p))
    })?;
    assert!(after.semantic_completion_proven());
    Ok(())
}

#[test]
fn replacement_and_reopened_owners_cannot_branch_or_import_an_older_monitor_streak() -> Result<()> {
    let (f, mut session, review) = goal_fixture()?;
    let mut first = session.begin_goal_monitor(review.seal(), &f.context)?;
    poll(&f, &mut session, &mut first, 1, 10, 0, true, false, |p| {
        Ok(grant(p))
    })?;
    let mut second = session.begin_goal_monitor(review.seal(), &f.context)?;
    let reads = f.native.borrow().reads;
    assert!(
        poll(&f, &mut session, &mut first, 2, 20, 0, true, false, |p| Ok(
            grant(p)
        ))
        .is_err()
    );
    assert_eq!(f.native.borrow().reads, reads);
    assert!(
        !poll(
            &f,
            &mut session,
            &mut second,
            2,
            20,
            0,
            true,
            false,
            |p| Ok(grant(p))
        )?
        .original_goal_proven()
    );
    assert!(
        poll(
            &f,
            &mut session,
            &mut second,
            3,
            30,
            0,
            true,
            false,
            |p| Ok(grant(p))
        )?
        .semantic_completion_proven()
    );
    drop(session);
    let mut reopened = f.session(false, JournalMode::Reconcile, true)?;
    let review = reopened.reattach(f.original.clone(), &f.context)?;
    let mut fresh = reopened.begin_goal_monitor(review.seal(), &f.context)?;
    let reads = f.native.borrow().reads;
    assert!(
        poll(&f, &mut reopened, &mut second, 4, 40, 0, true, false, |p| {
            Ok(grant(p))
        })
        .is_err()
    );
    assert_eq!(f.native.borrow().reads, reads);
    assert!(fresh.first_satisfied_anchor().is_none());
    assert!(
        !poll(
            &f,
            &mut reopened,
            &mut fresh,
            4,
            40,
            0,
            true,
            false,
            |p| Ok(grant(p))
        )?
        .original_goal_proven()
    );
    assert!(
        poll(
            &f,
            &mut reopened,
            &mut fresh,
            5,
            50,
            0,
            true,
            false,
            |p| Ok(grant(p))
        )?
        .semantic_completion_proven()
    );
    Ok(())
}

#[test]
fn original_deadline_is_not_renewed_by_starting_or_reattaching_a_monitor() -> Result<()> {
    let (f, mut session, review) = goal_fixture()?;
    let mut monitor = session.begin_goal_monitor(review.seal(), &f.context)?;
    let expired = poll(
        &f,
        &mut session,
        &mut monitor,
        1,
        101,
        0,
        true,
        false,
        |p| Ok(grant(p)),
    )?;
    assert!(!expired.original_goal_proven());
    assert!(matches!(
        expired.progress().obligation,
        Some(ObligationStatus::Failed { .. })
    ));
    drop(session);
    let mut session = f.session(false, JournalMode::Reconcile, true)?;
    let review = session.reattach(f.original.clone(), &f.context)?;
    let mut monitor = session.begin_goal_monitor(review.seal(), &f.context)?;
    let restarted = poll(
        &f,
        &mut session,
        &mut monitor,
        2,
        110,
        0,
        true,
        false,
        |p| Ok(grant(p)),
    )?;
    assert!(!restarted.semantic_completion_proven());
    assert_eq!(
        monitor.original_plan().steps[0].obligation,
        f.original.steps[0].obligation
    );
    Ok(())
}

#[test]
fn read_only_exact_recovery_can_prove_current_goal_without_restoring_control_authority()
-> Result<()> {
    let (mut f, session, _) = goal_fixture()?;
    drop(session);
    f.context
        .grants
        .retain(|g| matches!(g.capability, Capability::Query | Capability::Observe));
    let mut session = f.session(false, JournalMode::Reconcile, true)?;
    let review = session.reattach(f.original.clone(), &f.context)?;
    let mut monitor = session.begin_goal_monitor(review.seal(), &f.context)?;
    assert!(
        session
            .commit(review.seal(), true, &mut f.observer, &f.context)
            .is_err()
    );
    poll(&f, &mut session, &mut monitor, 1, 10, 0, true, false, |p| {
        Ok(grant(p))
    })?;
    let done = poll(&f, &mut session, &mut monitor, 2, 20, 0, true, false, |p| {
        Ok(grant(p))
    })?;
    assert!(done.semantic_completion_proven());
    assert_eq!(f.native.borrow().commits, 1);
    Ok(())
}

#[test]
fn budgets_and_revoked_observe_refuse_before_any_capture_or_new_source() -> Result<()> {
    let (f, mut session, review) = goal_fixture()?;
    let mut monitor = session.begin_goal_monitor(review.seal(), &f.context)?;
    for mode in 0..4 {
        let mut context = f.context.clone();
        match mode {
            0 => context.budget.max_bytes = 4 * 1024 * 1024,
            1 => context.budget.max_entities = 4095,
            2 => context.budget.max_output_tokens = 1,
            _ => context
                .grants
                .retain(|g| g.capability != Capability::Observe),
        }
        let reads = f.native.borrow().reads;
        let progress_contacts = Cell::new(0_u32);
        let operations_contacts = Cell::new(0_u32);
        let result = session.poll_original_goal::<ProgressFixture, OperationsFixture, _, _, _>(
            &mut monitor,
            ObservationCursor {
                epoch: context.anchor.cursor.epoch,
                sequence: context.anchor.cursor.sequence + 1,
            },
            &context,
            |_| {
                progress_contacts.set(progress_contacts.get() + 1);
                Err::<ProgressFixture, _>(rejected())
            },
            |_| {
                operations_contacts.set(operations_contacts.get() + 1);
                Err::<OperationsFixture, _>(rejected())
            },
            |_, _| Err(rejected()),
        );
        assert!(result.is_err());
        assert_eq!(f.native.borrow().reads, reads);
        assert_eq!(progress_contacts.get(), 0);
        assert_eq!(operations_contacts.get(), 0);
    }
    Ok(())
}

#[test]
fn fresh_tick_enforces_observe_expiry_and_prior_observations_prevent_clock_rollback() -> Result<()>
{
    let (mut f, mut session, review) = goal_fixture()?;
    let mut monitor = session.begin_goal_monitor(review.seal(), &f.context)?;
    let expiry = GameTick(f.original.anchor.tick.get() + 15);
    for grant in &mut f.context.grants {
        if grant.capability == Capability::Observe {
            grant.expires_at_tick = Some(expiry);
        }
    }
    poll(&f, &mut session, &mut monitor, 1, 10, 0, true, false, |p| {
        Ok(grant(p))
    })?;
    assert!(
        poll(&f, &mut session, &mut monitor, 2, 20, 0, true, false, |p| {
            Ok(grant(p))
        })
        .is_err()
    );
    assert!(monitor.latest().is_none());
    let reads = f.native.borrow().reads;
    assert!(
        poll(&f, &mut session, &mut monitor, 3, 10, 0, true, false, |p| {
            Ok(grant(p))
        })
        .is_err()
    );
    assert_eq!(f.native.borrow().reads, reads);
    Ok(())
}

#[test]
fn source_reset_order_reconfiguration_and_counter_increase_fence_original_monitor() -> Result<()> {
    for change in 0..4 {
        let (f, mut session, review) = goal_fixture()?;
        let mut monitor = session.begin_goal_monitor(review.seal(), &f.context)?;
        poll(&f, &mut session, &mut monitor, 1, 10, 2, true, false, |p| {
            Ok(grant(p))
        })?;
        let queue = advance(&f, 20)?;
        let id = queue.next_order_id() - 1;
        let remaining = if change == 1 { 3 } else { 1 };
        let recipe = if change == 2 { 2 } else { 1 };
        let mut captures = vec![
            progress(&queue, id, 3, remaining, recipe, 3)?,
            progress(&queue, id, 4, remaining, recipe, 3)?,
        ];
        let mut raw = operations(&queue, true, false);
        let closed = Rc::new(Cell::new(0));
        let mut source = native_progress(captures.clone(), Rc::clone(&closed));
        if change == 0 {
            source.manifest.generation = 502;
            for capture in &mut captures {
                let mut bytes = capture.canonical_bytes().to_vec();
                bytes[8..16].copy_from_slice(&502u64.to_be_bytes());
                *capture = ProgressObservation::decode(&bytes, &[id])?;
            }
            source.captures = captures;
        }
        if change == 3 {
            raw.jobs.bridge_generation = 602;
        }
        let result = session.poll_original_goal(
            &mut monitor,
            ObservationCursor {
                epoch: f.context.anchor.cursor.epoch,
                sequence: f.context.anchor.cursor.sequence + 2,
            },
            &f.context,
            |_| Ok(source),
            |_| Ok(OperationsFixture { raw, closed }),
            |p, _| Ok(grant(p)),
        );
        assert!(result.is_err(), "change {change}");
        assert!(monitor.is_fenced(), "change {change}");
        assert!(monitor.first_satisfied_anchor().is_none());
    }
    Ok(())
}

#[test]
fn cross_capture_changes_and_overbroad_policy_never_publish_goal_proof() -> Result<()> {
    for change in 0..5 {
        let (f, mut session, review) = goal_fixture()?;
        let mut monitor = session.begin_goal_monitor(review.seal(), &f.context)?;
        let queue = advance(&f, 10)?;
        let id = queue.next_order_id() - 1;
        let before = progress(&queue, id, 1, if change == 0 { 1 } else { 0 }, 1, 1)?;
        let after = progress(&queue, id, 2, 0, 1, 1)?;
        let mut raw = operations(&queue, true, false);
        if change == 1 {
            raw.jobs.year_tick += 1;
        }
        if change == 2 {
            raw.jobs.paused = false;
        }
        let closed = Rc::new(Cell::new(0));
        let source = native_progress(vec![before, after], Rc::clone(&closed));
        let result = session.poll_original_goal(
            &mut monitor,
            ObservationCursor {
                epoch: f.context.anchor.cursor.epoch,
                sequence: f.context.anchor.cursor.sequence + 1,
            },
            &f.context,
            |_| Ok(source),
            |_| Ok(OperationsFixture { raw, closed }),
            |p, _| {
                let mut policy = grant(p);
                if change == 3 {
                    policy
                        .entity_kinds
                        .insert(EntityKind::WorkOrder, EvidenceCoverage::Complete);
                }
                if change == 4 {
                    policy.all_entities = EvidenceCoverage::Complete;
                }
                Ok(policy)
            },
        );
        assert!(result.is_err(), "change {change}");
        assert!(monitor.latest().is_none());
    }
    Ok(())
}

fn varint(out: &mut Vec<u8>, mut n: u64) {
    while n >= 128 {
        out.push((n as u8 & 127) | 128);
        n >>= 7;
    }
    out.push(n as u8);
}
fn number(out: &mut Vec<u8>, field: u64, value: u64) {
    varint(out, field << 3);
    varint(out, value);
}
fn bytes(out: &mut Vec<u8>, field: u64, value: &[u8]) {
    varint(out, (field << 3) | 2);
    varint(out, value.len() as u64);
    out.extend_from_slice(value);
}
fn reply(minor: u64, generation: u64, observation: Option<&[u8]>) -> Vec<u8> {
    let mut out = Vec::new();
    for (f, n) in [(1, 1), (2, 0), (4, 1), (5, minor), (6, generation)] {
        number(&mut out, f, n);
    }
    bytes(&mut out, 3, &[b'n'; 16]);
    bytes(&mut out, 7, b"0.51.11");
    bytes(&mut out, 8, b"0.51.11-r1");
    if let Some(value) = observation {
        bytes(&mut out, 9, value);
    }
    out
}
fn frame(value: &[u8]) -> Vec<u8> {
    let mut out = (-1i16).to_le_bytes().to_vec();
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&(value.len() as i32).to_le_bytes());
    out.extend_from_slice(value);
    out
}
struct Script {
    input: Cursor<Vec<u8>>,
    writes: Rc<RefCell<Vec<u8>>>,
    closed: Rc<Cell<usize>>,
}
impl Read for Script {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let n = out.len().min(3);
        self.input.read(&mut out[..n])
    }
}
impl Write for Script {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        self.writes.borrow_mut().extend_from_slice(input);
        Ok(input.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl ProgressStream for Script {
    fn set_deadline(&mut self, _: Instant) -> io::Result<()> {
        Ok(())
    }
}
impl Drop for Script {
    fn drop(&mut self) {
        self.closed.set(self.closed.get() + 1);
    }
}
fn script(
    minor: u64,
    generation: u64,
    observations: Vec<Vec<u8>>,
    writes: Rc<RefCell<Vec<u8>>>,
    closed: Rc<Cell<usize>>,
) -> Script {
    let mut input = b"DFHack!\n".to_vec();
    input.extend_from_slice(&1i32.to_le_bytes());
    input.extend(frame(&[8, 2]));
    input.extend(frame(&[8, 3]));
    input.extend(frame(&reply(minor, generation, None)));
    for observation in observations {
        input.extend(frame(&reply(minor, generation, Some(&observation))));
    }
    Script {
        input: Cursor::new(input),
        writes,
        closed,
    }
}
struct WireOperations(OperationsRpcClient<Script>);
impl OperationsGoalSource for WireOperations {
    fn read_operations(&mut self, _: &OperationContext) -> Result<LiveOperationsObservation> {
        self.0.read_observation()
    }
}

#[test]
fn actual_closed_rpc_clients_drive_the_original_goal_to_completion_and_only_send_reads()
-> Result<()> {
    let (f, mut session, review) = goal_fixture()?;
    let mut monitor = session.begin_goal_monitor(review.seal(), &f.context)?;
    for n in 1..=2 {
        let queue = advance(&f, 10 * n)?;
        let id = queue.next_order_id() - 1;
        let closed = Rc::new(Cell::new(0));
        let writes = Rc::new(RefCell::new(Vec::new()));
        let source = script(
            12,
            501,
            vec![
                progress(&queue, id, 2 * n - 1, 0, 1, 1)?
                    .canonical_bytes()
                    .to_vec(),
                progress(&queue, id, 2 * n, 0, 1, 1)?
                    .canonical_bytes()
                    .to_vec(),
            ],
            Rc::clone(&writes),
            Rc::clone(&closed),
        );
        let raw = operations(&queue, true, false).encode_payload()?;
        let ops = script(3, 601, vec![raw], Rc::clone(&writes), Rc::clone(&closed));
        let result = session.poll_original_goal(
            &mut monitor,
            ObservationCursor {
                epoch: f.context.anchor.cursor.epoch,
                sequence: f.context.anchor.cursor.sequence + n,
            },
            &f.context,
            |c| {
                ProgressRpcClient::negotiate(
                    source,
                    vec![b't'; 32],
                    vec![b'n'; 16],
                    Duration::from_millis(c.budget.max_wall_millis),
                )
            },
            |_| {
                Ok(WireOperations(OperationsRpcClient::negotiate(
                    ops,
                    vec![b't'; 32],
                    vec![b'n'; 16],
                    OperationsLimits::default(),
                )?))
            },
            |p, _| {
                assert_eq!(closed.get(), 2);
                Ok(grant(p))
            },
        )?;
        assert_eq!(result.semantic_completion_proven(), n == 2);
        let data = writes.borrow();
        let text = String::from_utf8_lossy(&data);
        assert!(
            text.contains("ReadObservation")
                && text.contains("dfmcp_operations_v1_3")
                && text.contains("dfmcp_work_order_progress_v1_12")
        );
        assert!(!text.contains("Commit") && !text.contains("Prepare") && !text.contains("Create"));
    }
    Ok(())
}

#[test]
fn progress_before_a_mismatched_queue_frontier_still_fences_later_counter_increase() -> Result<()> {
    let (f, mut session, review) = goal_fixture()?;
    let mut monitor = session.begin_goal_monitor(review.seal(), &f.context)?;
    poll(&f, &mut session, &mut monitor, 1, 10, 4, true, false, |p| {
        Ok(grant(p))
    })?;
    let queue = advance(&f, 20)?;
    let later = changed_clock(
        &queue,
        queue.generation(),
        queue.sequence(),
        queue.tick() + 1,
    )?;
    let id = review.native_plan().observation().next_order_id();
    let closed = Rc::new(Cell::new(0));
    let source = native_progress(vec![progress(&later, id, 3, 2, 1, 3)?], Rc::clone(&closed));
    let operations_contacts = Cell::new(0);
    let refused = session.poll_original_goal(
        &mut monitor,
        ObservationCursor {
            epoch: f.context.anchor.cursor.epoch,
            sequence: f.context.anchor.cursor.sequence + 2,
        },
        &f.context,
        |_| Ok(source),
        |_| {
            operations_contacts.set(operations_contacts.get() + 1);
            Err::<OperationsFixture, _>(rejected())
        },
        |p, _| Ok(grant(p)),
    );
    assert_eq!(
        refused.err().ok_or_else(rejected)?.code,
        ErrorCode::StaleAnchor
    );
    assert_eq!(closed.get(), 1);
    assert_eq!(operations_contacts.get(), 0);
    assert!(monitor.latest().is_none());
    assert!(
        !monitor.is_fenced(),
        "a coherent but differently timed endpoint may be sampled again"
    );
    let increase = poll(&f, &mut session, &mut monitor, 3, 30, 3, true, false, |p| {
        Ok(grant(p))
    });
    assert_eq!(
        increase.err().ok_or_else(rejected)?.code,
        ErrorCode::StaleAnchor
    );
    assert!(
        monitor.is_fenced(),
        "the consumed remaining=2 cannot be rolled back to 3"
    );
    assert!(monitor.first_satisfied_anchor().is_none());
    Ok(())
}

#[test]
fn changed_second_progress_endpoint_still_fences_later_counter_increase() -> Result<()> {
    let (f, mut session, review) = goal_fixture()?;
    let mut monitor = session.begin_goal_monitor(review.seal(), &f.context)?;
    poll(&f, &mut session, &mut monitor, 1, 10, 4, true, false, |p| {
        Ok(grant(p))
    })?;
    let queue = advance(&f, 20)?;
    let id = review.native_plan().observation().next_order_id();
    let closed = Rc::new(Cell::new(0));
    let source = native_progress(
        vec![
            progress(&queue, id, 3, 3, 1, 3)?,
            progress(&queue, id, 4, 2, 1, 3)?,
        ],
        Rc::clone(&closed),
    );
    let raw = operations(&queue, true, false);
    let refused = session.poll_original_goal(
        &mut monitor,
        ObservationCursor {
            epoch: f.context.anchor.cursor.epoch,
            sequence: f.context.anchor.cursor.sequence + 2,
        },
        &f.context,
        |_| Ok(source),
        |_| {
            Ok(OperationsFixture {
                raw,
                closed: Rc::clone(&closed),
            })
        },
        |p, _| Ok(grant(p)),
    );
    assert_eq!(
        refused.err().ok_or_else(rejected)?.code,
        ErrorCode::StaleAnchor
    );
    assert_eq!(closed.get(), 2);
    assert!(monitor.latest().is_none());
    assert!(
        !monitor.is_fenced(),
        "a decreasing endpoint alone does not change the finite order"
    );
    let increase = poll(&f, &mut session, &mut monitor, 3, 30, 3, true, false, |p| {
        Ok(grant(p))
    });
    assert_eq!(
        increase.err().ok_or_else(rejected)?.code,
        ErrorCode::StaleAnchor
    );
    assert!(
        monitor.is_fenced(),
        "failed bracket cannot erase its second endpoint"
    );
    assert!(monitor.first_satisfied_anchor().is_none());
    Ok(())
}
