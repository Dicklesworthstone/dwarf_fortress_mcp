//! Source-referencing tests of the full original furniture runtime lifecycle.
use super::*;
#[allow(dead_code)]
#[path = "../../../dfmcp-adapter/tests/semantic_furniture/fixtures.rs"]
mod fixtures;
use dfmcp_adapter::build_placement::journal::BuildJournal;
use dfmcp_adapter::build_placement::{BuildItem, BuildKind, BuildRecord};
use dfmcp_adapter::construction_plan::{LinkedSample, Manifest};
use dfmcp_adapter::live_jobs::LiveJobObservation;
use dfmcp_adapter::live_operations::{
    LiveBuilding, LiveItem, LiveOperationsObservation, OperationsProfile,
};
use dfmcp_core::MapCoord;
use dfmcp_world::{EntityKind, EvidenceCoverage};
use fixtures::{Evidence, EvidenceState, Memory, Native, SourceState};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
#[path = "runtime_clock_tests.rs"]
mod runtime_clock_tests;

#[derive(Default)]
struct GoalState {
    tick: u64,
    reads: usize,
    active: usize,
    policies: usize,
    denied: bool,
    corrupt_archive: bool,
    paused: bool,
    condition: &'static str,
}
struct GoalSource {
    state: Rc<RefCell<GoalState>>,
}
impl FurnitureGoalSource for GoalSource {
    fn read_construction(
        &mut self,
        original: &BuildRecord,
        c: &OperationContext,
    ) -> Result<LinkedSample> {
        c.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        let mut state = self.state.borrow_mut();
        state.reads += 1;
        complete_sample(original, state.tick, |observed| {
            runtime_clock_tests::change_construction(original, observed, &state);
        })
    }
}
impl Drop for GoalSource {
    fn drop(&mut self) {
        let mut state = self.state.borrow_mut();
        state.active = state.active.saturating_sub(1);
    }
}
#[derive(Clone)]
struct Host {
    binding: BuildBinding,
    native_disk: Memory,
    associations: Memory,
    archive: Memory,
    history: Memory,
    native: Rc<RefCell<SourceState>>,
    evidence: Rc<RefCell<EvidenceState>>,
    goal: Rc<RefCell<GoalState>>,
    enabled: Rc<Cell<bool>>,
    exception: Rc<Cell<bool>>,
    original_archive_ack_lost: Rc<Cell<bool>>,
    recovery: Option<Recovery>,
    policy: Rc<Cell<bool>>,
    open_calls: Rc<Cell<usize>>,
    clock: Option<runtime_clock_tests::ClockResources>,
}
impl Host {
    fn new() -> Result<Self> {
        let f = fixtures::Fixture::new()?;
        Ok(Self {
            binding: f.binding.clone(),
            native_disk: f.native_disk.clone(),
            associations: f.associations.clone(),
            archive: Memory::default(),
            history: Memory::default(),
            native: f.native.clone(),
            evidence: f.evidence_state.clone(),
            goal: Rc::new(RefCell::new(GoalState {
                tick: f.capture.tick(),
                paused: true,
                ..GoalState::default()
            })),
            enabled: Rc::new(Cell::new(true)),
            exception: Rc::new(Cell::new(true)),
            original_archive_ack_lost: Rc::new(Cell::new(false)),
            recovery: None,
            policy: Rc::new(Cell::new(true)),
            open_calls: Rc::new(Cell::new(0)),
            clock: None,
        })
    }
    fn runtime(&self, mode: Mode) -> Result<Runtime<Self>> {
        Runtime::from_shell(
            self.clone(),
            mode,
            SessionId::new(771),
            WorkBudget {
                max_wall_millis: 60_000,
                max_bytes: DEFAULT_BYTES,
                max_entities: 8192,
                max_actions: 1,
                max_output_tokens: MAX_OUTPUT_TOKENS,
                max_game_ticks: if self.clock.is_some() {
                    MAX_GAME_TICKS
                } else {
                    0
                },
            },
        )
    }
    fn recovered(&self, runtime: &Runtime<Self>, history_present: bool) -> Result<Self> {
        let mut out = self.clone();
        out.recovery = Some(Recovery {
            journal_id: runtime.journal,
            seal: runtime
                .archive_seal
                .ok_or_else(|| invalid("test original archive seal"))?,
            anchor: runtime
                .original
                .as_ref()
                .ok_or_else(|| invalid("test original"))?
                .anchor,
            history_present,
        });
        if let Some(clock) = &self.clock {
            out.clock = Some(
                clock.recovered(
                    runtime
                        .opening_clock
                        .as_ref()
                        .ok_or_else(|| invalid("test retained clock"))?
                        .journal_id,
                ),
            );
        }
        Ok(out)
    }
}
impl Shell for Host {
    type Journal = Memory;
    type Association = Memory;
    type Native = Native;
    type Archive = Memory;
    type History = Memory;
    type Evidence = Evidence;
    type Guard = fixtures::Guard;
    type GoalSource = GoalSource;
    type ClockJournal = Memory;
    type ClockSource = runtime_clock_tests::ClockNative;
    fn clock_enabled(&self) -> bool {
        self.clock.as_ref().is_some_and(|clock| clock.enabled())
    }
    fn open_clock(&self, mode: Mode, c: &OperationContext) -> Result<Option<RunJournal<Memory>>> {
        self.clock
            .as_ref()
            .map(|clock| clock.open(mode, c))
            .transpose()
    }
    fn clock_context(&self, c: &OperationContext, control: bool) -> Result<OperationContext> {
        self.clock
            .as_ref()
            .ok_or_else(|| denied("test clock not configured"))?
            .context(c, control)
    }
    fn clock_source(&self, c: &OperationContext) -> Result<Self::ClockSource> {
        self.clock
            .as_ref()
            .ok_or_else(|| denied("test clock not configured"))?
            .source(c)
    }
    fn fortress(&self) -> FortressId {
        self.binding.fortress().fortress_id()
    }
    fn recovery(&self) -> Option<Recovery> {
        self.recovery
    }
    fn check_environment(&self) -> Result<()> {
        Ok(())
    }
    fn placement_enabled(&self) -> bool {
        self.enabled.get()
    }
    fn checkpoint_exception(&self) -> bool {
        self.exception.get()
    }
    fn open_control(
        &self,
        mode: Mode,
        c: &OperationContext,
    ) -> Result<(
        BuildSession<Memory, Native>,
        AssociationStore<Memory>,
        Digest32,
    )> {
        self.open_calls.set(self.open_calls.get() + 1);
        let mut journal = BuildJournal::open(
            self.native_disk.clone(),
            c,
            mode.journal(),
            Some(self.binding.clone()),
            None,
        )?;
        let id = journal.inventory(c)?.journal_id;
        Ok((
            BuildSession::new(journal, c)?,
            AssociationStore::open(
                self.associations.clone(),
                id,
                false,
                mode == Mode::Offline,
                c,
            )?,
            id,
        ))
    }
    fn native(
        &self,
        binding: &BuildBinding,
        selection: BuildSelection,
        c: &OperationContext,
        timeout: Duration,
    ) -> Result<Native> {
        assert_eq!(selection, self.native.borrow().capture.selection());
        fixtures::factory(self.native.clone(), self.native_disk.clone())(binding, c, timeout)
    }
    fn guard(&self, _: Digest32, _: &OperationContext) -> Result<Self::Guard> {
        Ok(fixtures::Guard {
            permitted: self.policy.clone(),
            ..fixtures::Guard::default()
        })
    }
    fn acquire(&self, c: &OperationContext) -> Result<LiveSpatialCitizenObservation> {
        c.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        Ok(self.evidence.borrow().observation.clone())
    }
    fn spatial_policy(
        &self,
        state: &LiveSpatialCitizenState,
        _: &OperationContext,
    ) -> Result<EvidencePolicy> {
        if !self.evidence.borrow().permitted {
            return Err(denied("test source policy revoked"));
        }
        Ok(fixtures::policy(
            state
                .snapshot()
                .ok_or_else(|| invalid("test spatial state"))?,
        ))
    }
    fn evidence(&self, state: LiveSpatialCitizenState, c: &OperationContext) -> Result<Evidence> {
        fixtures::evidence_owner(state, self.evidence.clone(), c)
    }
    fn create_archive(
        &self,
        plan: &PreparedPlan,
        c: &OperationContext,
    ) -> Result<PlanArchiveStore<Memory>> {
        let archive = PlanArchiveStore::create(self.archive.clone(), plan, archive_limits(), c)?;
        if self.original_archive_ack_lost.get() {
            return Err(DfmcpError::new(
                ErrorCode::AdapterUnavailable,
                "archive persisted; acknowledgement lost",
            ));
        }
        Ok(archive)
    }
    fn recover_archive(
        &self,
        recovery: Recovery,
        c: &OperationContext,
    ) -> Result<PlanArchiveStore<Memory>> {
        PlanArchiveStore::open_at_authority_floor(
            self.archive.clone(),
            recovery.seal,
            archive_limits(),
            c,
        )
    }
    fn history(
        &self,
        journal: Digest32,
        review: &SemanticFurnitureReview,
        initialize: bool,
        read_only: bool,
        c: &OperationContext,
    ) -> Result<GoalHistoryStore<Memory>> {
        GoalHistoryStore::open(
            self.history.clone(),
            journal,
            review,
            initialize,
            read_only,
            c,
        )
    }
    fn goal_source(&self, binding: &BuildBinding, c: &OperationContext) -> Result<GoalSource> {
        c.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        assert_eq!(binding, &self.binding);
        self.goal.borrow_mut().active += 1;
        Ok(GoalSource {
            state: self.goal.clone(),
        })
    }
    fn goal_policy(
        &self,
        p: &FurnitureGoalProjection,
        c: &OperationContext,
    ) -> Result<EvidencePolicy> {
        c.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        let mut state = self.goal.borrow_mut();
        assert_eq!(
            state.active, 0,
            "construction source must close before independent policy"
        );
        state.policies += 1;
        if state.denied {
            return Err(denied("test construction authority revoked"));
        }
        if state.corrupt_archive {
            self.archive.corrupt();
        }
        let mut policy = EvidencePolicy::at(p.anchor());
        policy.paused = true;
        policy
            .entity_kinds
            .insert(EntityKind::Building, EvidenceCoverage::Observed);
        policy.sources = p.evidence_sources();
        Ok(policy)
    }
}
fn observe_plan(runtime: &mut Runtime<Host>, stable: u32) -> Result<View> {
    let c = runtime.context(false)?;
    let observed = runtime.observe(&c)?;
    let seal = observed
        .observation
        .as_ref()
        .and_then(|v| v["seal"].as_str())
        .ok_or_else(|| invalid("test observation seal"))?
        .to_owned();
    let capture = runtime.shell.native.borrow().capture.clone();
    let request = Request {
        expected_observation: seal,
        kind: "bed".into(),
        item: item_entity_id(capture.selection().item_id())
            .get()
            .to_string(),
        target: capture.selection().target(),
        deadline_tick: capture.tick() + 1000,
        poll_interval_ticks: 5,
        stable_samples: stable,
    };
    let c = runtime.context(false)?;
    runtime.plan(request, &c)
}
fn commit(runtime: &mut Runtime<Host>) -> Result<View> {
    let seal = runtime
        .review
        .as_ref()
        .ok_or_else(|| invalid("test paired review"))?
        .seal();
    let c = runtime.context(false)?;
    runtime.commit(seal, true, &c)
}
fn wait(runtime: &mut Runtime<Host>) -> Result<View> {
    let c = runtime.context(false)?;
    runtime.wait(&c)
}
#[test]
fn insufficient_complete_opening_budget_refuses_before_any_custody_io() -> Result<()> {
    let host = Host::new()?;
    let native_before = host.native_disk.durable();
    let associations_before = host.associations.durable();
    let result = Runtime::from_shell(
        host.clone(),
        Mode::Control,
        SessionId::new(771),
        WorkBudget {
            max_wall_millis: 60_000,
            max_bytes: 576 * MIB - 1,
            max_entities: 8192,
            max_actions: 1,
            max_output_tokens: MAX_OUTPUT_TOKENS,
            max_game_ticks: 0,
        },
    );
    assert!(matches!(result, Err(cause) if cause.code == ErrorCode::BudgetExceeded));
    assert_eq!(host.open_calls.get(), 0);
    assert_eq!(host.native.borrow().connections, 0);
    assert_eq!(host.native_disk.durable(), native_before);
    assert_eq!(host.associations.durable(), associations_before);
    assert!(host.archive.bytes().is_empty());
    Ok(())
}
#[test]
fn original_plan_places_once_and_two_cadenced_samples_prove_original_goal() -> Result<()> {
    let host = Host::new()?;
    let mut runtime = host.runtime(Mode::Control)?;
    let prepared = observe_plan(&mut runtime, 2)?;
    assert!(prepared.prepared_in_opening);
    assert_eq!(
        prepared.native.as_ref().map(|n| n.state()),
        Some(BuildState::Prepared)
    );
    assert_eq!(prepared.to_json()["effect_uncertain"], false);
    let original = prepared
        .original
        .clone()
        .ok_or_else(|| invalid("original plan absent"))?;
    assert!(!original.requires_checkpoint);
    assert!(matches!(
        &original.steps[0].action,
        dfmcp_intent::Action::Build { .. }
    ));
    assert_eq!(
        original.steps[0].postconditions,
        dfmcp_intent::effects::default_postconditions(
            &original.steps[0].action,
            &original.steps[0].idempotency_key,
            original.anchor.fortress_id
        )
    );
    let placed = commit(&mut runtime)?;
    assert_eq!(host.native.borrow().commits, 1);
    assert!(placed.goal.is_none());
    assert_eq!(
        placed.to_json()["placement"]["construction_complete"],
        false
    );
    let initialized = wait(&mut runtime)?;
    assert!(initialized.history_present);
    assert!(initialized.goal.is_none());
    assert_eq!(host.goal.borrow().reads, 0);
    let early = wait(&mut runtime)?;
    let early = early.goal.as_ref().ok_or_else(|| invalid("early sample"))?;
    assert!(early.construction_proven);
    assert!(!early.current_goal_proven());
    assert!(matches!(
        early.obligation,
        Some(ObligationStatus::Active {
            consecutive_stable_observations: 0,
            ..
        })
    ));
    host.goal.borrow_mut().tick += 5;
    let first = wait(&mut runtime)?;
    assert!(
        !first
            .goal
            .as_ref()
            .ok_or_else(|| invalid("first construction sample"))?
            .current_goal_proven()
    );
    host.goal.borrow_mut().tick += 5;
    let second = wait(&mut runtime)?;
    assert!(
        second
            .goal
            .as_ref()
            .ok_or_else(|| invalid("second construction sample"))?
            .current_goal_proven()
    );
    assert_eq!(
        second.to_json()["current_goal"]["semantic_completion_proven"],
        true
    );
    assert_eq!(second.original, Some(original));
    assert_eq!(host.native.borrow().commits, 1);
    assert_eq!(host.goal.borrow().reads, 3);
    Ok(())
}
#[test]
fn lost_commit_reply_recovers_original_once_and_does_not_reacquire_dispatch_permission()
-> Result<()> {
    let host = Host::new()?;
    let mut runtime = host.runtime(Mode::Control)?;
    observe_plan(&mut runtime, 1)?;
    host.native.borrow_mut().lost_commit_reply = true;
    assert!(commit(&mut runtime).is_err());
    assert_eq!(host.native.borrow().commits, 1);
    assert!(commit(&mut runtime).is_err());
    let recovered = host.recovered(&runtime, false)?;
    drop(runtime);
    recovered.enabled.set(false);
    let mut runtime = recovered.runtime(Mode::Reconcile)?;
    let at_open = recovered.native.borrow().connections;
    assert!(!runtime.prepared_in_opening);
    assert!(runtime.evidence.is_none());
    let view = wait(&mut runtime)?;
    assert!(matches!(
        view.placement,
        Some(NativeFurniturePlacement::Placed { .. })
    ));
    assert_eq!(recovered.native.borrow().commits, 1);
    assert_eq!(recovered.native.borrow().connections, at_open + 1);
    Ok(())
}
#[test]
fn required_checkpoint_cannot_be_disabled_by_client_request() -> Result<()> {
    let host = Host::new()?;
    host.exception.set(false);
    let mut runtime = host.runtime(Mode::Control)?;
    assert!(observe_plan(&mut runtime, 1).is_err());
    assert!(host.archive.bytes().is_empty());
    assert_eq!(host.native.borrow().prepares, 0);
    Ok(())
}
#[test]
fn archive_changes_after_dispatch_marker_refuse_native_writer() -> Result<()> {
    let host = Host::new()?;
    let mut runtime = host.runtime(Mode::Control)?;
    observe_plan(&mut runtime, 1)?;
    let archive = host.archive.clone();
    host.native_disk.after_next_sync(move || archive.corrupt());
    assert!(commit(&mut runtime).is_err());
    assert_eq!(host.native.borrow().commits, 0);
    assert!(!runtime.prepared_in_opening);
    Ok(())
}
#[test]
fn lost_archive_acknowledgement_retains_original_and_reopens_without_a_native_permit() -> Result<()>
{
    let host = Host::new()?;
    host.original_archive_ack_lost.set(true);
    let mut runtime = host.runtime(Mode::Control)?;
    assert!(observe_plan(&mut runtime, 1).is_err());
    assert!(runtime.original.is_some());
    assert!(runtime.archive.is_none());
    let hint = runtime
        .recovery_hint()
        .ok_or_else(|| invalid("partial archive hint"))?;
    assert_eq!(hint["archive_publication_unknown"], true);
    assert!(!host.archive.durable().is_empty());
    assert_eq!(host.native.borrow().connections, 0);
    let recovered = host.recovered(&runtime, false)?;
    drop(runtime);
    let mut runtime = recovered.runtime(Mode::Offline)?;
    let c = runtime.context(false)?;
    let view = runtime.inspect(&c)?;
    assert!(view.original.is_some());
    assert!(view.review.is_none());
    assert!(view.native.is_none());
    assert!(!view.prepared_in_opening);
    assert_eq!(recovered.native.borrow().connections, 0);
    Ok(())
}
#[test]
fn durable_goal_reopen_preserves_achievement_but_requires_a_new_current_sample() -> Result<()> {
    let host = Host::new()?;
    let mut runtime = host.runtime(Mode::Control)?;
    observe_plan(&mut runtime, 1)?;
    commit(&mut runtime)?;
    wait(&mut runtime)?;
    host.goal.borrow_mut().tick += 5;
    let fulfilled = wait(&mut runtime)?;
    let first = fulfilled
        .first_satisfied_anchor
        .ok_or_else(|| invalid("first achievement"))?;
    let recovered = host.recovered(&runtime, true)?;
    let calls = host.native.borrow().connections;
    drop(runtime);
    recovered.enabled.set(false);
    let mut runtime = recovered.runtime(Mode::Reconcile)?;
    let c = runtime.context(false)?;
    let reopened = runtime.inspect(&c)?;
    assert!(reopened.goal.is_none());
    assert_eq!(reopened.first_satisfied_anchor, Some(first));
    assert!(!reopened.prepared_in_opening);
    assert_eq!(host.native.borrow().connections, calls);
    host.goal.borrow_mut().tick += 5;
    let current = wait(&mut runtime)?;
    assert!(
        current
            .goal
            .as_ref()
            .ok_or_else(|| invalid("fresh resumed goal"))?
            .current_goal_proven()
    );
    assert_eq!(host.native.borrow().commits, 1);
    Ok(())
}
#[test]
fn original_archive_is_rechecked_after_goal_source_closes_before_publication() -> Result<()> {
    let host = Host::new()?;
    let mut runtime = host.runtime(Mode::Control)?;
    observe_plan(&mut runtime, 1)?;
    commit(&mut runtime)?;
    wait(&mut runtime)?;
    host.goal.borrow_mut().corrupt_archive = true;
    assert!(wait(&mut runtime).is_err());
    assert_eq!(host.goal.borrow().reads, 1);
    assert_eq!(host.goal.borrow().active, 0);
    assert!(
        runtime
            .monitor
            .as_ref()
            .is_none_or(|m| m.latest().is_none())
    );
    assert_eq!(host.native.borrow().commits, 1);
    Ok(())
}

fn complete_sample(
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
