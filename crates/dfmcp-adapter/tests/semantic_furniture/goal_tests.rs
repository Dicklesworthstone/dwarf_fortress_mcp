//! End-to-end original-goal proof uses real receipt assessment, predicate
//! evaluation, obligation runtime and durable history. Only I/O is injected.

use super::fixtures::*;
use dfmcp_adapter::build_placement::journal::BuildMode;
use dfmcp_adapter::build_placement::{BuildBinding, BuildItem, BuildKind, BuildRecord};
use dfmcp_adapter::construction_plan::{LinkedSample, Manifest};
use dfmcp_adapter::live_jobs::{LiveJob, LiveJobObservation};
use dfmcp_adapter::live_operations::{
    LiveBuilding, LiveItem, LiveOperationsObservation, OperationsProfile,
};
use dfmcp_adapter::semantic_furniture::{
    DurableFurnitureGoalMonitor, FurnitureGoalMonitor, FurnitureGoalProjection,
    FurnitureGoalResult, FurnitureGoalSource, GoalHistoryStore, SemanticFurnitureReview,
};
use dfmcp_core::{
    Capability, DfmcpError, Digest32, EntityId, ErrorCode, GameTick, MapCoord, ObservationCursor,
    OperationContext, Result, RiskTier,
};
use dfmcp_intent::{ObligationStatus, PreparedPlan};
use dfmcp_world::{EntityKind, EvidenceCoverage, EvidencePolicy, Predicate, PredicateTruth};
use std::cell::RefCell;
use std::rc::Rc;

fn placed(
    stable: u32,
    update: impl FnOnce(&mut PreparedPlan),
) -> Result<(Fixture, SemanticFurnitureReview, BuildRecord)> {
    let mut fixture = Fixture::new()?;
    fixture.original = reseal(&fixture.original, |plan| {
        if let Some(spec) = &mut plan.steps[0].obligation {
            spec.stable_for_observations = stable;
        }
        update(plan);
    });
    let review = fixture.prepared()?;
    let result = fixture.session.commit(
        review.seal(),
        true,
        &mut fixture.evidence,
        &fixture.context,
        &mut fixture.guard,
    )?;
    let record = result
        .native_record()
        .native()
        .cloned()
        .ok_or_else(|| invalid("placed goal fixture receipt"))?;
    fixture
        .context
        .grants
        .retain(|grant| matches!(grant.capability, Capability::Query | Capability::Observe));
    fixture.context.budget.max_bytes = 1024 * 1024 * 1024;
    fixture.context.budget.max_entities = 8192;
    Ok((fixture, review, record))
}

fn cursor(fixture: &Fixture, index: u64) -> ObservationCursor {
    ObservationCursor {
        epoch: fixture.original.anchor.cursor.epoch,
        sequence: fixture.original.anchor.cursor.sequence + index,
    }
}

fn sample(
    record: &BuildRecord,
    tick: u64,
    mutate: impl FnOnce(&mut LiveOperationsObservation),
) -> Result<LinkedSample> {
    let insertion = record
        .insertion()
        .ok_or_else(|| invalid("fixture insertion"))?;
    let BuildItem::Visible(expected) = record.plan().before().item() else {
        return Err(invalid("fixture selected item must be visible"));
    };
    let after = record
        .after()
        .ok_or_else(|| invalid("fixture receipt after capture"))?;
    let [x, y, z] = insertion.position().map(|value| value as i32);
    let (building_kind, item_kind) = match insertion.kind() {
        BuildKind::Bed => ("Bed", "BED"),
        BuildKind::Chair => ("Chair", "CHAIR"),
        BuildKind::Table => ("Table", "TABLE"),
    };
    let mut observed = LiveOperationsObservation {
        jobs: LiveJobObservation {
            bridge_generation: 902,
            df_version: "df".to_owned(),
            dfhack_version: "dfhack".to_owned(),
            year: (tick / 403_200) as u32,
            year_tick: (tick % 403_200) as u32,
            paused: true,
            site_id: record.plan().before().site() as i32,
            world_folder: record.plan().before().folder().to_owned(),
            next_job_id: after.next_job_id(),
            jobs: Vec::new(),
        },
        next_building_id: after.next_building_id(),
        next_item_id: insertion.item_id() + 1,
        buildings: vec![LiveBuilding {
            native_id: insertion.building_id(),
            building_type: insertion.kind() as i32,
            type_key: building_kind.to_owned(),
            x1: x,
            y1: y,
            x2: x,
            y2: y,
            z,
            build_stage: insertion.max_stage() as i32,
            max_build_stage: insertion.max_stage() as i32,
        }],
        items: vec![LiveItem {
            native_id: insertion.item_id(),
            item_type: expected.native_type() as i32,
            type_key: item_kind.to_owned(),
            subtype: expected.subtype(),
            material_type: expected.material(),
            material_index: expected.material_index(),
            stack_size: 1,
            raw_position: MapCoord::new(x, y, z),
            flags: 1 << 8,
            container_native_id: None,
            holder_building_native_id: Some(insertion.building_id()),
        }],
        attachments: Vec::new(),
    };
    mutate(&mut observed);
    let native = Manifest {
        generation: record.plan().before().generation(),
        df_version: "df".to_owned(),
        dfhack_version: "dfhack".to_owned(),
    };
    let encoded = record.canonical_bytes().to_vec();
    Ok(LinkedSample {
        before: native.clone(),
        before_records: vec![encoded.clone()],
        operations: Manifest {
            generation: observed.jobs.bridge_generation,
            df_version: observed.jobs.df_version.clone(),
            dfhack_version: observed.jobs.dfhack_version.clone(),
        },
        capture: observed.encode_profile(OperationsProfile::PagedV1_4)?,
        after: native,
        after_records: vec![encoded],
    })
}

struct SourceState {
    receipt: BuildRecord,
    seal: Digest32,
    sample: LinkedSample,
    opened: usize,
    active: usize,
    reads: usize,
    policies: usize,
    permitted: bool,
    grant_sources: bool,
    fail_read: bool,
}
type Source = Rc<RefCell<SourceState>>;

fn source(record: &BuildRecord, review: &SemanticFurnitureReview, tick: u64) -> Result<Source> {
    Ok(Rc::new(RefCell::new(SourceState {
        receipt: record.clone(),
        seal: review.seal(),
        sample: sample(record, tick, |_| {})?,
        opened: 0,
        active: 0,
        reads: 0,
        policies: 0,
        permitted: true,
        grant_sources: true,
        fail_read: false,
    })))
}

fn read_authority(context: &OperationContext) -> Result<()> {
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)
}
struct NativeGoal(Source);
impl FurnitureGoalSource for NativeGoal {
    fn read_construction(
        &mut self,
        original: &BuildRecord,
        context: &OperationContext,
    ) -> Result<LinkedSample> {
        read_authority(context)?;
        let mut state = self.0.borrow_mut();
        assert_eq!(original, &state.receipt);
        state.reads += 1;
        if state.fail_read {
            return Err(DfmcpError::new(
                ErrorCode::AdapterUnavailable,
                "injected interrupted original construction read",
            ));
        }
        Ok(state.sample.clone())
    }
}
impl Drop for NativeGoal {
    fn drop(&mut self) {
        let mut state = self.0.borrow_mut();
        state.active = state.active.saturating_sub(1);
    }
}
fn factory(source: Source) -> impl FnOnce(&BuildBinding, &OperationContext) -> Result<NativeGoal> {
    move |binding, context| {
        read_authority(context)?;
        let mut state = source.borrow_mut();
        assert!(binding.capture_matches(state.receipt.plan().before()));
        state.opened += 1;
        state.active += 1;
        drop(state);
        Ok(NativeGoal(source))
    }
}
fn policy(
    source: Source,
) -> impl FnMut(&FurnitureGoalProjection, &OperationContext) -> Result<EvidencePolicy> {
    move |projection, context| {
        read_authority(context)?;
        let mut state = source.borrow_mut();
        assert_eq!(
            state.active, 0,
            "native construction source must close before policy"
        );
        assert_eq!(projection.review_seal(), state.seal);
        assert_eq!(projection.assessment().receipt, state.receipt.receipt());
        state.policies += 1;
        if !state.permitted {
            return Err(DfmcpError::new(
                ErrorCode::CapabilityDenied,
                "independent construction policy revoked",
            ));
        }
        let mut granted = EvidencePolicy::at(projection.anchor());
        granted.paused = true;
        granted
            .entity_kinds
            .insert(EntityKind::Building, EvidenceCoverage::Observed);
        if state.grant_sources {
            granted.sources = projection.evidence_sources();
        }
        Ok(granted)
    }
}
fn poll(
    fixture: &mut Fixture,
    monitor: &mut FurnitureGoalMonitor,
    index: u64,
    source: &Source,
) -> Result<FurnitureGoalResult> {
    let cursor = cursor(fixture, index);
    fixture.session.poll_original_goal(
        monitor,
        cursor,
        &fixture.context,
        factory(source.clone()),
        policy(source.clone()),
    )
}
fn durable_poll(
    fixture: &mut Fixture,
    monitor: &mut DurableFurnitureGoalMonitor<Memory>,
    index: u64,
    source: &Source,
) -> Result<FurnitureGoalResult> {
    let cursor = cursor(fixture, index);
    fixture.session.poll_original_goal_durable(
        monitor,
        cursor,
        &fixture.context,
        factory(source.clone()),
        policy(source.clone()),
    )
}
fn history(
    fixture: &mut Fixture,
    review: &SemanticFurnitureReview,
    memory: Memory,
    initialize: bool,
    read_only: bool,
) -> Result<GoalHistoryStore<Memory>> {
    let native = fixture.session.inventory(&fixture.context)?.journal_id;
    GoalHistoryStore::open(
        memory,
        native,
        review,
        initialize,
        read_only,
        &fixture.context,
    )
}
fn stable_count(progress: &FurnitureGoalResult) -> Option<u32> {
    match progress.progress().obligation {
        Some(ObligationStatus::Active {
            consecutive_stable_observations,
            ..
        }) => Some(consecutive_stable_observations),
        _ => None,
    }
}

#[test]
fn exact_original_construction_reaches_stability_then_current_contradiction_preserves_only_history()
-> Result<()> {
    let (mut fixture, review, record) = placed(2, |_| {})?;
    let base = fixture.capture.tick();
    let original = fixture.original.clone();
    let mut monitor = fixture
        .session
        .begin_goal_monitor(review.seal(), &fixture.context)?;
    let source = source(&record, &review, base + 10)?;
    source.borrow_mut().sample =
        sample(&record, base + 10, |raw| raw.buildings[0].build_stage = 0)?;
    let pending = poll(&mut fixture, &mut monitor, 1, &source)?;
    assert_eq!(
        pending.progress().construction_condition,
        "no_construction_job"
    );
    assert_eq!(pending.progress().current_goal, PredicateTruth::False);
    assert!(!pending.semantic_completion_proven());
    source.borrow_mut().sample = sample(&record, base + 20, |_| {})?;
    let candidate = poll(&mut fixture, &mut monitor, 2, &source)?;
    assert_eq!(candidate.progress().current_goal, PredicateTruth::True);
    assert!(candidate.progress().construction_proven);
    assert_eq!(stable_count(&candidate), Some(1));
    assert!(!candidate.original_goal_proven());
    source.borrow_mut().sample = sample(&record, base + 30, |_| {})?;
    let completed = poll(&mut fixture, &mut monitor, 3, &source)?;
    assert!(completed.semantic_completion_proven());
    let achieved = completed.progress().anchor;
    assert_eq!(monitor.first_satisfied_anchor(), Some(achieved));
    assert_eq!(monitor.original_plan(), &original);
    source.borrow_mut().sample = sample(&record, base + 40, |raw| {
        let building = &raw.buildings[0];
        let id = raw.jobs.next_job_id;
        raw.jobs.next_job_id += 1;
        raw.jobs.jobs.push(LiveJob {
            native_id: id,
            job_type: 2,
            type_key: "DestroyBuilding".to_owned(),
            reaction: String::new(),
            suspended: false,
            repeating: false,
            position: MapCoord::new(building.x1, building.y1, building.z),
            worker_native_id: None,
            holder_native_id: Some(building.native_id),
            completion_timer: -1,
            attached_item_count: 0,
            required_item_filter_count: 0,
        });
    })?;
    let contradicted = poll(&mut fixture, &mut monitor, 4, &source)?;
    assert_eq!(
        contradicted.progress().construction_condition,
        "removal_pending"
    );
    assert!(!contradicted.semantic_completion_proven());
    assert_eq!(
        contradicted.progress().first_satisfied_anchor,
        Some(achieved)
    );
    assert!(matches!(
        contradicted.progress().obligation,
        Some(ObligationStatus::Fulfilled { .. })
    ));
    assert_eq!(fixture.native.borrow().commits, 1);
    assert_eq!(source.borrow().active, 0);
    assert_eq!(source.borrow().policies, 8);
    Ok(())
}

#[test]
fn construction_readiness_does_not_replace_original_terminal_failure_or_unknown_predicates()
-> Result<()> {
    for case in 0..3 {
        let (mut fixture, review, record) = placed(1, |plan| match case {
            0 => plan.terminal_condition = Predicate::Paused(false),
            1 => {
                if let Some(spec) = &mut plan.steps[0].obligation {
                    spec.failure = Some(Predicate::Paused(true));
                }
            }
            _ => {
                plan.steps[0].postconditions.push(Predicate::EntityKind {
                    entity_id: EntityId::new(999_999),
                    kind: EntityKind::Unit,
                });
                plan.steps[0]
                    .postconditions
                    .sort_by_key(Predicate::canonical_bytes);
            }
        })?;
        let mut monitor = fixture
            .session
            .begin_goal_monitor(review.seal(), &fixture.context)?;
        let source = source(&record, &review, fixture.capture.tick() + 10)?;
        let result = poll(&mut fixture, &mut monitor, 1, &source)?;
        assert!(result.progress().construction_proven);
        assert!(!result.original_goal_proven());
        assert!(!result.semantic_completion_proven());
        assert_eq!(monitor.first_satisfied_anchor(), None);
        match case {
            0 => assert_eq!(result.progress().plan_terminal, PredicateTruth::False),
            1 => {
                assert_eq!(
                    result.progress().failure_predicate,
                    Some(PredicateTruth::True)
                );
                assert!(matches!(
                    result.progress().obligation,
                    Some(ObligationStatus::Failed { .. })
                ));
            }
            _ => assert_eq!(result.progress().postconditions, PredicateTruth::Unknown),
        }
        assert_eq!(fixture.native.borrow().commits, 1);
    }
    Ok(())
}

#[test]
fn fresh_raw_capture_without_independent_derivation_authority_remains_unknown() -> Result<()> {
    let (mut fixture, review, record) = placed(1, |_| {})?;
    let base = fixture.capture.tick();
    let source = source(&record, &review, base + 10)?;
    source.borrow_mut().grant_sources = false;
    let mut monitor = fixture
        .session
        .begin_goal_monitor(review.seal(), &fixture.context)?;
    let unknown = poll(&mut fixture, &mut monitor, 1, &source)?;
    assert_eq!(unknown.progress().construction_condition, "condition_met");
    assert_eq!(unknown.progress().current_goal, PredicateTruth::Unknown);
    assert!(!unknown.progress().construction_proven);
    assert!(!unknown.semantic_completion_proven());
    source.borrow_mut().grant_sources = true;
    source.borrow_mut().sample = sample(&record, base + 20, |_| {})?;
    assert!(poll(&mut fixture, &mut monitor, 2, &source)?.semantic_completion_proven());
    assert_eq!(fixture.native.borrow().commits, 1);
    Ok(())
}

#[test]
fn duplicate_tick_and_observation_gap_never_renew_original_stability_or_deadline() -> Result<()> {
    let (mut fixture, review, record) = placed(2, |plan| {
        let deadline = GameTick(plan.anchor.tick.get() + 25);
        if let Some(spec) = &mut plan.steps[0].obligation {
            spec.deadline_tick = deadline;
        }
        plan.expires_at_tick = deadline;
    })?;
    let base = fixture.capture.tick();
    let source = source(&record, &review, base + 10)?;
    let mut monitor = fixture
        .session
        .begin_goal_monitor(review.seal(), &fixture.context)?;
    assert_eq!(
        stable_count(&poll(&mut fixture, &mut monitor, 1, &source)?),
        Some(1)
    );
    let duplicate_tick = poll(&mut fixture, &mut monitor, 2, &source)?;
    assert_eq!(stable_count(&duplicate_tick), Some(1));
    assert!(!duplicate_tick.original_goal_proven());
    monitor.observation_interrupted()?;
    assert!(monitor.latest().is_none());
    source.borrow_mut().sample = sample(&record, base + 20, |_| {})?;
    let after_gap = poll(&mut fixture, &mut monitor, 3, &source)?;
    assert_eq!(stable_count(&after_gap), Some(1));
    assert!(!after_gap.original_goal_proven());
    source.borrow_mut().sample = sample(&record, base + 30, |_| {})?;
    let late = poll(&mut fixture, &mut monitor, 4, &source)?;
    assert!(matches!(
        late.progress().obligation,
        Some(ObligationStatus::Failed { .. })
    ));
    assert!(!late.semantic_completion_proven());
    assert_eq!(monitor.original_plan().expires_at_tick, GameTick(base + 25));
    let calls = source.borrow().opened;
    assert!(poll(&mut fixture, &mut monitor, 4, &source).is_err());
    assert_eq!(source.borrow().opened, calls);
    let mut replacement = fixture
        .session
        .begin_goal_monitor(review.seal(), &fixture.context)?;
    source.borrow_mut().sample = sample(&record, base + 40, |_| {})?;
    assert!(!poll(&mut fixture, &mut replacement, 5, &source)?.semantic_completion_proven());
    assert_eq!(fixture.native.borrow().commits, 1);
    Ok(())
}

#[test]
fn rejected_policy_still_consumes_source_clock_identity_and_stage_floors() -> Result<()> {
    for case in 0..4 {
        let (mut fixture, review, record) = placed(1, |_| {})?;
        let base = fixture.capture.tick();
        let source = source(&record, &review, base + 20)?;
        if case == 1 {
            source.borrow_mut().sample = sample(&record, base + 20, |raw| {
                raw.buildings.clear();
                raw.items[0].holder_building_native_id = None;
                raw.items[0].flags = 1 << 6;
            })?;
        }
        source.borrow_mut().permitted = false;
        let mut monitor = fixture
            .session
            .begin_goal_monitor(review.seal(), &fixture.context)?;
        assert!(poll(&mut fixture, &mut monitor, 1, &source).is_err());
        assert!(monitor.latest().is_none());
        source.borrow_mut().permitted = true;
        let mut replacement = fixture
            .session
            .begin_goal_monitor(review.seal(), &fixture.context)?;
        source.borrow_mut().sample = sample(
            &record,
            if case == 0 { base + 10 } else { base + 30 },
            |raw| {
                if case == 2 {
                    raw.jobs.bridge_generation += 1;
                }
                if case == 3 {
                    raw.buildings[0].build_stage = 0;
                }
            },
        )?;
        assert!(poll(&mut fixture, &mut replacement, 2, &source).is_err());
        assert!(replacement.is_fenced());
        assert!(
            fixture
                .session
                .begin_goal_monitor(review.seal(), &fixture.context)
                .is_err()
        );
        assert_eq!(fixture.native.borrow().commits, 1);
        assert_eq!(source.borrow().active, 0);
    }
    Ok(())
}

#[test]
fn durable_restart_resets_unfinished_stability_and_recovers_achievement_without_current_truth()
-> Result<()> {
    let (mut fixture, review, record) = placed(2, |_| {})?;
    let base = fixture.capture.tick();
    let source = source(&record, &review, base + 10)?;
    let disk = Memory::default();
    let store = history(&mut fixture, &review, disk.clone(), true, false)?;
    let mut monitor = fixture.session.begin_goal_monitor_durable(
        review.seal(),
        store,
        &fixture.context,
        policy(source.clone()),
    )?;
    assert_eq!(
        stable_count(&durable_poll(&mut fixture, &mut monitor, 1, &source)?),
        Some(1)
    );
    assert_eq!(monitor.history_event_count(), 3);
    drop(monitor);
    drop(fixture.session);
    fixture.session = reopen(
        fixture.native_disk.clone(),
        fixture.associations.clone(),
        &fixture.binding,
        BuildMode::Recover,
        false,
        &fixture.context,
    )?;
    fixture
        .session
        .reattach(fixture.original.clone(), &fixture.context)?;
    let store = history(&mut fixture, &review, disk.clone(), false, false)?;
    let mut resumed = fixture.session.begin_goal_monitor_durable(
        review.seal(),
        store,
        &fixture.context,
        policy(source.clone()),
    )?;
    assert!(resumed.latest().is_none());
    assert_eq!(resumed.first_satisfied_anchor(), None);
    source.borrow_mut().sample = sample(&record, base + 20, |_| {})?;
    let next = durable_poll(&mut fixture, &mut resumed, 2, &source)?;
    assert_eq!(stable_count(&next), Some(1));
    assert!(!next.semantic_completion_proven());
    source.borrow_mut().sample = sample(&record, base + 30, |_| {})?;
    let complete = durable_poll(&mut fixture, &mut resumed, 3, &source)?;
    assert!(complete.semantic_completion_proven());
    let achieved = complete.progress().anchor;
    let bytes = disk.bytes();
    let calls = source.borrow().opened;
    drop(resumed);
    drop(fixture.session);
    fixture.session = reopen(
        fixture.native_disk.clone(),
        fixture.associations.clone(),
        &fixture.binding,
        BuildMode::Offline,
        true,
        &fixture.context,
    )?;
    fixture
        .session
        .reattach(fixture.original.clone(), &fixture.context)?;
    let store = history(&mut fixture, &review, disk.clone(), false, true)?;
    let mut historical = fixture.session.begin_goal_monitor_durable(
        review.seal(),
        store,
        &fixture.context,
        policy(source.clone()),
    )?;
    assert!(historical.latest().is_none());
    assert_eq!(historical.first_satisfied_anchor(), Some(achieved));
    assert!(matches!(
        historical.historical_obligation_status(),
        Some(ObligationStatus::Fulfilled { .. })
    ));
    assert!(durable_poll(&mut fixture, &mut historical, 4, &source).is_err());
    assert_eq!(source.borrow().opened, calls);
    assert_eq!(disk.bytes(), bytes);
    assert_eq!(fixture.native.borrow().commits, 1);
    Ok(())
}

#[test]
fn durable_history_requires_current_independent_policy_and_exact_storage_custody() -> Result<()> {
    let (mut fixture, review, record) = placed(1, |_| {})?;
    let source = source(&record, &review, fixture.capture.tick() + 10)?;
    let disk = Memory::default();
    let store = history(&mut fixture, &review, disk.clone(), true, false)?;
    let mut monitor = fixture.session.begin_goal_monitor_durable(
        review.seal(),
        store,
        &fixture.context,
        policy(source.clone()),
    )?;
    assert!(durable_poll(&mut fixture, &mut monitor, 1, &source)?.semantic_completion_proven());
    drop(monitor);
    // Recovery cannot revive authority by returning to the original plan tick.
    let mut expired = fixture.context.clone();
    for grant in &mut expired.grants {
        if grant.capability == Capability::Query {
            grant.expires_at_tick = Some(GameTick(fixture.capture.tick() + 5));
        }
    }
    let policies = source.borrow().policies;
    let store = history(&mut fixture, &review, disk.clone(), false, false)?;
    assert!(
        fixture
            .session
            .begin_goal_monitor_durable(review.seal(), store, &expired, policy(source.clone()))
            .is_err()
    );
    assert_eq!(source.borrow().policies, policies);
    let bytes = disk.bytes();
    let calls = source.borrow().opened;
    source.borrow_mut().permitted = false;
    let store = history(&mut fixture, &review, disk.clone(), false, false)?;
    assert!(
        fixture
            .session
            .begin_goal_monitor_durable(
                review.seal(),
                store,
                &fixture.context,
                policy(source.clone())
            )
            .is_err()
    );
    assert_eq!(source.borrow().opened, calls);
    assert_eq!(disk.bytes(), bytes);
    source.borrow_mut().permitted = true;
    let store = history(&mut fixture, &review, disk.clone(), false, false)?;
    let mut recovered = fixture.session.begin_goal_monitor_durable(
        review.seal(),
        store,
        &fixture.context,
        policy(source.clone()),
    )?;
    assert!(recovered.first_satisfied_anchor().is_some());
    disk.corrupt();
    assert!(durable_poll(&mut fixture, &mut recovered, 2, &source).is_err());
    assert!(recovered.latest().is_none());
    assert!(recovered.is_fenced());
    assert_eq!(source.borrow().opened, calls);
    assert_eq!(fixture.native.borrow().commits, 1);
    Ok(())
}

#[test]
fn failed_publication_sync_never_exposes_success_or_recovers_a_dispatch_permit() -> Result<()> {
    let (mut fixture, review, record) = placed(1, |_| {})?;
    let source = source(&record, &review, fixture.capture.tick() + 10)?;
    let disk = Memory::default();
    let store = history(&mut fixture, &review, disk.clone(), true, false)?;
    let mut monitor = fixture.session.begin_goal_monitor_durable(
        review.seal(),
        store,
        &fixture.context,
        policy(source.clone()),
    )?;
    let captured = disk.clone();
    disk.after_next_sync(move || {
        let published = captured.clone();
        captured.after_next_sync(move || published.fail_next_sync());
    });
    assert!(durable_poll(&mut fixture, &mut monitor, 1, &source).is_err());
    assert!(monitor.latest().is_none());
    assert!(monitor.is_fenced());
    assert_eq!(monitor.first_satisfied_anchor(), None);
    assert_eq!(source.borrow().reads, 1);
    assert_eq!(source.borrow().policies, 2);
    let survived = Memory::from_bytes(disk.durable());
    drop(monitor);
    drop(fixture.session);
    fixture.session = reopen(
        fixture.native_disk.clone(),
        fixture.associations.clone(),
        &fixture.binding,
        BuildMode::Recover,
        false,
        &fixture.context,
    )?;
    fixture
        .session
        .reattach(fixture.original.clone(), &fixture.context)?;
    let store = history(&mut fixture, &review, survived, false, false)?;
    let mut recovered = fixture.session.begin_goal_monitor_durable(
        review.seal(),
        store,
        &fixture.context,
        policy(source.clone()),
    )?;
    assert!(recovered.is_fenced());
    assert!(recovered.latest().is_none());
    assert_eq!(recovered.first_satisfied_anchor(), None);
    let calls = source.borrow().opened;
    assert!(durable_poll(&mut fixture, &mut recovered, 2, &source).is_err());
    assert_eq!(source.borrow().opened, calls);
    assert_eq!(fixture.native.borrow().commits, 1);
    Ok(())
}
