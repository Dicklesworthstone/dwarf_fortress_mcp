use super::*;
use crate::live_jobs::LiveJobObservation;
use crate::live_operations::{LiveItem, LiveOperationsObservation};
use crate::live_production::{SelectedWorkOrderGoalMonitor, SelectedWorkOrderGoalResult};
use crate::semantic_work_orders::{
    GoalHistoryStore, NativeOrderCreation, ObservedInventoryObjective, OperationsGoalSource,
    WorkOrderGoalProjection, WorkOrderPhysicalState,
};
use crate::work_order_progress::{ProgressManifest, ProgressObservation, ProgressSource};
use dfmcp_core::MapCoord;
use dfmcp_intent::ObligationStatus;
use dfmcp_world::PredicateTruth;

fn inventory_fixture(count: usize) -> Result<ProductionFixture> {
    let mut f = ProductionFixture::new(count)?;
    f.c.grants.push(CapabilityGrant {
        capability: Capability::Observe,
        scope: CapabilityScope {
            fortress_id: Some(f.c.anchor.fortress_id),
            ..CapabilityScope::default()
        },
        max_risk: RiskTier::ReadOnly,
        expires_at_tick: None,
        remaining_uses: None,
    });
    f.c.budget.max_bytes = 4 * 1024 * 1024 * 1024;
    let goal = Predicate::EntityQuantity {
        kind: EntityKind::Item,
        filter_field: "type_key".into(),
        filter_value: Value::Text("BED".into()),
        quantity_field: Some("stack_size".into()),
        op: CompareOp::Ge,
        value: 2,
    };
    for step in &mut f.plan.steps[1..] {
        step.postconditions = vec![goal.clone()];
        if let Some(spec) = &mut step.obligation {
            spec.terminal = goal.clone();
        }
    }
    f.plan.terminal_condition =
        Predicate::All(vec![f.plan.steps[0].postconditions[0].clone(), goal]).normalized();
    reseal(&mut f.plan);
    let Executor {
        workforce,
        orders,
        evidence,
        factory,
        ..
    } = f.executor;
    f.executor = Executor::new(f.plan.clone(), workforce, orders, evidence, factory, &f.c)?;
    Ok(f)
}

fn grant(p: &WorkOrderGoalProjection) -> EvidencePolicy {
    let mut policy = EvidencePolicy::at(p.anchor());
    policy.paused = true;
    policy.entity_kinds = BTreeMap::from([
        (EntityKind::Fortress, EvidenceCoverage::Observed),
        (EntityKind::Job, EvidenceCoverage::Complete),
        (EntityKind::Item, EvidenceCoverage::Complete),
        (EntityKind::WorkOrder, EvidenceCoverage::Observed),
    ]);
    policy.sources = p
        .observed_sources()
        .into_iter()
        .filter(|s| {
            matches!(s,
        EvidenceSource::Observed { field, .. } if matches!(field.as_str(),
            "operations/1.3.job_count" | "operations/1.3.item.type_key" |
            "operations/1.3.item.getStackSize" | "work-order-progress/1.12.manager_order.counters" |
            "work-order-progress/1.12.manager_order.status"))
        })
        .collect();
    policy
}

fn open(
    f: &mut ProductionFixture,
    step: u32,
    storage: Storage,
    initialize: bool,
) -> Result<SelectedWorkOrderGoalMonitor<Storage>> {
    let mut opening = f.c.clone();
    opening.budget.max_bytes = 2 * 1024 * 1024 * 1024;
    f.executor.begin_selected_order_goal_monitor(
        StepId::new(step),
        storage,
        initialize,
        false,
        &opening,
        |p, _| Ok(grant(p)),
    )
}

struct Progress {
    manifest: ProgressManifest,
    captures: Vec<ProgressObservation>,
}
impl ProgressSource for Progress {
    fn manifest(&self) -> &ProgressManifest {
        &self.manifest
    }
    fn fence(&mut self) {}
    fn read(&mut self, ids: &[u32], _: Duration) -> Result<ProgressObservation> {
        if self.captures.is_empty() {
            return Err(invalid("missing selected progress capture"));
        }
        ProgressObservation::decode(self.captures.remove(0).canonical_bytes(), ids)
    }
}
struct Operations(LiveOperationsObservation);
impl OperationsGoalSource for Operations {
    fn read_operations(&mut self, _: &OperationContext) -> Result<LiveOperationsObservation> {
        LiveOperationsObservation::decode_payload(
            &self.0.encode_payload()?,
            self.0.jobs.bridge_generation,
            self.0.jobs.df_version.clone(),
            self.0.jobs.dfhack_version.clone(),
        )
    }
}

fn progress(queue: &WorkOrderObservation, id: u32, sequence: u64) -> Result<ProgressObservation> {
    let mut raw = b"DFMWP012".to_vec();
    for value in [501u64, sequence, queue.tick()] {
        raw.extend_from_slice(&value.to_be_bytes());
    }
    for value in [
        queue.site_id(),
        queue.next_order_id(),
        queue.order_ids().len() as u32,
    ] {
        raw.extend_from_slice(&value.to_be_bytes());
    }
    raw.push(1);
    text(&mut raw, queue.world_folder());
    raw.extend_from_slice(&1u32.to_be_bytes());
    raw.extend_from_slice(&id.to_be_bytes());
    let present = queue.order_ids().contains(&id);
    raw.push(u8::from(present));
    if present {
        raw.extend_from_slice(&77i32.to_be_bytes());
        raw.push(1);
        for value in [2i32, 2] {
            raw.extend_from_slice(&value.to_be_bytes());
        }
        raw.extend_from_slice(&3u32.to_be_bytes());
        for value in [0i32, -1, 1, 0, 0] {
            raw.extend_from_slice(&value.to_be_bytes());
        }
        raw.extend_from_slice(&0u32.to_be_bytes());
        raw.extend_from_slice(&0u32.to_be_bytes());
        text(&mut raw, "ConstructBed");
        text(&mut raw, "");
    }
    ProgressObservation::decode(&raw, &[id])
}

fn operations(queue: &WorkOrderObservation, units: u32) -> LiveOperationsObservation {
    LiveOperationsObservation {
        jobs: LiveJobObservation {
            bridge_generation: 601,
            df_version: "0.51.11".into(),
            dfhack_version: "0.51.11-r1".into(),
            year: 0,
            year_tick: queue.tick() as u32,
            paused: true,
            site_id: queue.site_id() as i32,
            world_folder: queue.world_folder().into(),
            next_job_id: 100,
            jobs: Vec::new(),
        },
        next_building_id: 100,
        next_item_id: 100,
        buildings: Vec::new(),
        items: if units > 0 {
            vec![LiveItem {
                native_id: 2,
                item_type: 5,
                type_key: "BED".into(),
                subtype: -1,
                material_type: 420,
                material_index: 0,
                stack_size: units,
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

fn retire(f: &ProductionFixture, id: u32) -> Result<()> {
    let old = f.orders.borrow().queue.clone();
    let ids: Vec<_> = old
        .order_ids()
        .iter()
        .copied()
        .filter(|value| *value != id)
        .collect();
    let at = 43 + old.world_folder().len();
    let mut bytes = old.canonical_bytes()[..at].to_vec();
    bytes.extend_from_slice(&(ids.len() as u32).to_be_bytes());
    for value in ids {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    f.orders.borrow_mut().queue = WorkOrderObservation::decode(&bytes)?;
    Ok(())
}

fn sample<A>(
    f: &mut ProductionFixture,
    monitor: &mut SelectedWorkOrderGoalMonitor<Storage>,
    sequence: u64,
    tick: u64,
    units: u32,
    policy: A,
) -> Result<SelectedWorkOrderGoalResult>
where
    A: FnMut(&WorkOrderGoalProjection, &OperationContext) -> Result<EvidencePolicy>,
{
    let old = f.orders.borrow().queue.clone();
    let mut raw = old.canonical_bytes().to_vec();
    raw[16..24].copy_from_slice(&(old.sequence() + 1).to_be_bytes());
    raw[24..32].copy_from_slice(&tick.to_be_bytes());
    let queue = WorkOrderObservation::decode(&raw)?;
    f.orders.borrow_mut().queue = queue.clone();
    let id = 99 + monitor.step_id().get();
    let before = progress(&queue, id, 2 * sequence)?;
    let after = progress(&queue, id, 2 * sequence + 1)?;
    let operations = operations(&queue, units);
    f.executor.poll_selected_order_goal(
        monitor,
        ObservationCursor {
            epoch: f.c.anchor.cursor.epoch,
            sequence,
        },
        &f.c,
        |_| {
            Ok(Progress {
                manifest: ProgressManifest {
                    generation: 501,
                    df_version: "0.51.11".into(),
                    dfhack_version: "0.51.11-r1".into(),
                },
                captures: vec![before, after],
            })
        },
        |_| Ok(Operations(operations)),
        policy,
    )
}

#[test]
fn selected_inventory_retains_nonzero_key_whole_parent_and_proves_after_retirement() -> Result<()> {
    let mut f = inventory_fixture(2)?;
    for index in 0..=2 {
        f.applied(index)?;
    }
    let mut monitor = open(&mut f, 2, Storage::default(), true)?;
    assert_eq!(monitor.step_id(), StepId::new(2));
    assert_eq!(monitor.original_plan(), &f.plan);
    assert_eq!(monitor.archive_seal(), validate_plan(&f.plan)?);
    assert!(ObservedInventoryObjective::from_plan(&f.plan).is_none());
    let objective = ObservedInventoryObjective::from_selected_plan(&f.plan, StepId::new(2))
        .ok_or_else(|| invalid("missing selected inventory objective"))?;
    assert_eq!(objective.item_type, "BED");
    assert_eq!(objective.min_stack_units, 2);
    assert!(ObservedInventoryObjective::from_selected_plan(&f.plan, StepId::ZERO).is_none());
    let expected = dfmcp_intent::effects::created_entity_id(&f.plan.steps[2].idempotency_key, 0);
    let first = sample(&mut f, &mut monitor, 1, 125, 2, |p, _| {
        assert_eq!(p.original_order_entity(), expected);
        Ok(grant(p))
    })?;
    assert_eq!(first.progress().current_goal, PredicateTruth::True);
    assert_eq!(first.progress().plan_terminal, PredicateTruth::Unknown);
    assert!(!first.selected_goal_proven());
    assert!(matches!(
        first.progress().physical_work,
        WorkOrderPhysicalState::Outstanding {
            native_order_id: 101,
            remaining: 2
        }
    ));
    retire(&f, 101)?;
    let retired = sample(&mut f, &mut monitor, 2, 130, 2, |p, _| Ok(grant(p)))?;
    assert!(retired.selected_goal_proven());
    assert!(!retired.original_goal_proven());
    assert!(!retired.progress().receipt_linked);
    assert!(matches!(
        retired.creation_result(),
        NativeOrderCreation::Inserted {
            native_order_id: 101,
            ..
        }
    ));
    assert!(matches!(
        retired.progress().physical_work,
        WorkOrderPhysicalState::Unknown { .. }
    ));
    assert_eq!(f.orders.borrow().prepares, 2);
    assert_eq!(f.orders.borrow().commits, 2);
    assert_eq!(f.orders.borrow().queries, 0);
    Ok(())
}

#[test]
fn selected_history_reopens_exact_achievement_without_current_truth_or_new_effects() -> Result<()> {
    let mut f = inventory_fixture(1)?;
    f.applied(0)?;
    f.applied(1)?;
    let history = Storage::default();
    let mut monitor = open(&mut f, 1, history.clone(), true)?;
    sample(&mut f, &mut monitor, 1, 125, 2, |p, _| Ok(grant(p)))?;
    retire(&f, 100)?;
    let proven = sample(&mut f, &mut monitor, 2, 130, 2, |p, _| Ok(grant(p)))?;
    assert!(proven.selected_goal_proven());
    let first = monitor.first_satisfied_anchor();
    let retained_bytes = monitor.history_byte_len();
    drop(monitor);
    let mut f = f.reopen(false)?;
    let mut monitor = open(&mut f, 1, history, false)?;
    assert!(monitor.latest().is_none());
    assert_eq!(monitor.first_satisfied_anchor(), first);
    assert_eq!(monitor.history_byte_len(), retained_bytes);
    assert_eq!(monitor.high_cursor().sequence, 2);
    assert!(monitor.high_tick() >= GameTick(130));
    assert!(matches!(
        monitor.historical_obligation_status(),
        Some(ObligationStatus::Fulfilled { .. })
    ));
    let lost = sample(&mut f, &mut monitor, 3, 135, 0, |p, _| Ok(grant(p)))?;
    assert!(!lost.selected_goal_proven());
    assert_eq!(lost.progress().current_goal, PredicateTruth::False);
    assert_eq!(lost.progress().first_satisfied_anchor, first);
    assert_eq!(f.orders.borrow().commits, 1);
    assert_eq!(f.workforce.data.borrow().calls[2], 1);
    Ok(())
}

#[test]
fn denied_selected_publication_breaks_stability_and_reopen_cannot_join_the_old_streak() -> Result<()>
{
    let mut f = inventory_fixture(1)?;
    f.applied(0)?;
    f.applied(1)?;
    let history = Storage::default();
    let mut monitor = open(&mut f, 1, history.clone(), true)?;
    sample(&mut f, &mut monitor, 1, 125, 2, |p, _| Ok(grant(p)))?;
    let mut calls = 0;
    assert!(
        sample(&mut f, &mut monitor, 2, 130, 2, |p, _| {
            calls += 1;
            if calls == 2 {
                Err(invalid("denied selected final policy"))
            } else {
                Ok(grant(p))
            }
        })
        .is_err()
    );
    assert_eq!(calls, 2);
    assert!(monitor.latest().is_none());
    assert!(monitor.first_satisfied_anchor().is_none());
    assert!(f.executor.high_tick() >= 130);
    drop(monitor);
    let mut f = f.reopen(false)?;
    let mut monitor = open(&mut f, 1, history, false)?;
    assert!(monitor.first_satisfied_anchor().is_none());
    let first = sample(&mut f, &mut monitor, 3, 135, 2, |p, _| Ok(grant(p)))?;
    assert!(!first.selected_goal_proven());
    let second = sample(&mut f, &mut monitor, 4, 140, 2, |p, _| Ok(grant(p)))?;
    assert!(second.selected_goal_proven());
    Ok(())
}

#[test]
fn selected_recovery_breaks_unfinished_stability_and_keeps_the_original_deadline() -> Result<()> {
    let mut f = inventory_fixture(1)?;
    f.applied(0)?;
    f.applied(1)?;
    let history = Storage::default();
    let mut monitor = open(&mut f, 1, history.clone(), true)?;
    sample(&mut f, &mut monitor, 1, 215, 2, |p, _| Ok(grant(p)))?;
    drop(monitor);
    let mut f = f.reopen(false)?;
    let mut monitor = open(&mut f, 1, history, false)?;
    assert!(monitor.latest().is_none());
    assert!(monitor.first_satisfied_anchor().is_none());
    let late = sample(&mut f, &mut monitor, 2, 225, 2, |p, _| Ok(grant(p)))?;
    assert_eq!(late.progress().current_goal, PredicateTruth::True);
    assert!(!late.selected_goal_proven());
    assert!(matches!(
        late.progress().obligation,
        Some(ObligationStatus::Failed { .. })
    ));
    assert_eq!(
        monitor.original_plan().steps[1]
            .obligation
            .as_ref()
            .map(|spec| spec.deadline_tick),
        Some(GameTick(220))
    );
    Ok(())
}

#[test]
fn selected_history_cannot_move_to_another_key_review_or_whole_parent() -> Result<()> {
    let mut f = inventory_fixture(2)?;
    for index in 0..=2 {
        f.applied(index)?;
    }
    let history = Storage::default();
    let mut old = open(&mut f, 1, history.clone(), true)?;
    assert!(open(&mut f, 2, history.clone(), false).is_err());
    let replacement = open(&mut f, 1, history.clone(), false)?;
    let reads = f.orders.borrow().reads;
    assert!(sample(&mut f, &mut old, 1, 125, 2, |p, _| Ok(grant(p))).is_err());
    assert_eq!(f.orders.borrow().reads, reads);
    drop((old, replacement));
    let Some(StepEvidence::WorkOrder(result)) = f.executor.latest_step_evidence() else {
        return Err(invalid("missing selected review"));
    };
    let review = result.review().clone();
    let selected = f.executor.selection(review.step_id())?;
    f.executor.orders.reattach_selected(selected, &f.c)?;
    assert!(
        f.executor
            .orders
            .begin_goal_monitor(review.seal(), &f.c)
            .is_err()
    );
    let journal = WorkOrderJournal::open(
        Storage::from_bytes(f.order_journal.0.borrow().synced.clone()),
        &f.c,
        JournalMode::Offline,
        false,
    )?
    .summary(&f.c)?
    .journal_id;
    assert!(
        GoalHistoryStore::open(Storage::default(), journal, &review, true, false, &f.c).is_err()
    );
    let selected_store =
        GoalHistoryStore::open_selected(Storage::default(), journal, &review, true, false, &f.c)?;
    assert!(
        f.executor
            .orders
            .begin_durable_goal_monitor(selected_store, review.seal(), &f.c, |p, _| Ok(grant(p)))
            .is_err()
    );
    // Even an internal selected monitor cannot enter a legacy public polling
    // API; rejection precedes source acquisition and history publication.
    let mut selected_monitor = f.executor.orders.begin_selected_goal_history(
        review.seal(),
        &f.c,
        |native, review, c| {
            GoalHistoryStore::open_selected(Storage::default(), native, review, true, false, c)
        },
        |p, _| Ok(grant(p)),
    )?;
    let events = selected_monitor.history_event_count();
    let reads = f.orders.borrow().reads;
    assert!(
        f.executor
            .orders
            .poll_original_goal_durable(
                &mut selected_monitor,
                ObservationCursor {
                    epoch: f.c.anchor.cursor.epoch,
                    sequence: 1
                },
                &f.c,
                |_| Err::<Progress, _>(invalid("legacy source must not open")),
                |_| Err::<Operations, _>(invalid("legacy source must not open")),
                |p, _| Ok(grant(p)),
            )
            .is_err()
    );
    assert_eq!(f.orders.borrow().reads, reads);
    assert_eq!(selected_monitor.history_event_count(), events);
    let mut changed = f.plan.clone();
    changed.terminal_condition = Predicate::True;
    reseal(&mut changed);
    let Executor {
        workforce,
        orders,
        evidence,
        factory,
        ..
    } = f.executor;
    assert!(Executor::new(changed, workforce, orders, evidence, factory, &f.c).is_err());
    Ok(())
}

#[test]
fn uncertain_insertion_cannot_open_selected_inventory_history_or_dispatch_again() -> Result<()> {
    let mut f = inventory_fixture(1)?;
    f.applied(0)?;
    let prepared = f.prepare(1)?;
    f.orders.borrow_mut().lose_reply = true;
    assert!(f.commit(1, prepared).is_err());
    let history = Storage::default();
    assert!(open(&mut f, 1, history.clone(), true).is_err());
    assert!(history.0.borrow().file.get_ref().is_empty());
    assert_eq!(f.orders.borrow().prepares, 1);
    assert_eq!(f.orders.borrow().commits, 1);
    assert_eq!(f.orders.borrow().queries, 0);
    Ok(())
}

#[test]
fn selected_poll_checks_predecessor_custody_after_policy_and_withholds_publication() -> Result<()> {
    let mut f = inventory_fixture(1)?;
    f.applied(0)?;
    f.applied(1)?;
    let mut monitor = open(&mut f, 1, Storage::default(), true)?;
    let storage = f.workforce.associations.clone();
    assert!(
        sample(&mut f, &mut monitor, 1, 125, 2, |p, _| {
            storage.0.borrow_mut().file.get_mut()[0] ^= 1;
            Ok(grant(p))
        })
        .is_err()
    );
    assert!(monitor.latest().is_none());
    assert!(monitor.first_satisfied_anchor().is_none());
    assert!(f.executor.high_tick() >= 125);
    assert_eq!(f.orders.borrow().commits, 1);
    Ok(())
}

#[test]
fn selected_replay_rechecks_predecessor_custody_after_historical_policy() -> Result<()> {
    let mut f = inventory_fixture(1)?;
    f.applied(0)?;
    f.applied(1)?;
    let history = Storage::default();
    let mut monitor = open(&mut f, 1, history.clone(), true)?;
    sample(&mut f, &mut monitor, 1, 125, 2, |p, _| Ok(grant(p)))?;
    drop(monitor);
    let storage = f.workforce.journal.clone();
    let mut altered = false;
    assert!(
        f.executor
            .begin_selected_order_goal_monitor(
                StepId::new(1),
                history,
                false,
                false,
                &f.c,
                |p, _| {
                    if !altered {
                        storage.0.borrow_mut().file.get_mut()[0] ^= 1;
                        altered = true;
                    }
                    Ok(grant(p))
                }
            )
            .is_err()
    );
    assert!(altered);
    assert!(f.executor.high_tick() >= 125);
    assert_eq!(f.orders.borrow().commits, 1);
    Ok(())
}
