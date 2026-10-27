#![allow(dead_code, unused_imports)]
//! Joined owner tests execute the real coordinator, archive codec, goal monitor,
//! source projection and strict native codecs with injected effect/storage edges.
include!("fixtures.rs");
use super::identity::{CapsuleStore, decode};
use super::policy::SourceAcceptance;
use super::{Mode, Recovery, Request, Runtime, Shell, View};
use dfmcp_adapter::plan_archive::PlanArchiveStore;
use dfmcp_adapter::semantic_workforce::GoalHistoryStore;
use std::cell::Cell;

#[path = "runtime_clock_tests.rs"]
mod runtime_clock_tests;
use runtime_clock_tests::{ClockNative, ClockResources};

#[derive(Clone)]
struct MockShell {
    native: Native,
    capsule: LiveObservationCapsule,
    acceptance: SourceAcceptance,
    archive: Storage,
    identity: Storage,
    history: Storage,
    recovery: Option<Recovery>,
    enabled: Rc<Cell<bool>>,
    acquisitions: Rc<Cell<usize>>,
    archive_failure: Rc<Cell<bool>>,
    clock: Option<ClockResources>,
}
impl Shell for MockShell {
    type Journal = Storage;
    type Association = Storage;
    type Native = Native;
    type Archive = Storage;
    type Identity = Storage;
    type History = Storage;
    type Evidence = Observer;
    type ClockJournal = Storage;
    type ClockSource = ClockNative;
    fn clock_enabled(&self) -> bool {
        self.clock.as_ref().is_some_and(ClockResources::enabled)
    }
    fn open_clock(
        &self,
        mode: Mode,
        c: &OperationContext,
    ) -> Result<Option<dfmcp_adapter::bounded_run::journal::RunJournal<Storage>>> {
        self.clock
            .as_ref()
            .map(|clock| clock.open(mode, c))
            .transpose()
    }
    fn clock_context(&self, c: &OperationContext, control: bool) -> Result<OperationContext> {
        self.clock
            .as_ref()
            .ok_or_else(|| invalid("clock fixture absent"))?
            .context(c, control)
    }
    fn clock_source(&self, c: &OperationContext) -> Result<ClockNative> {
        self.clock
            .as_ref()
            .ok_or_else(|| invalid("clock fixture absent"))?
            .source(c)
    }
    fn acceptance(&self) -> &SourceAcceptance {
        &self.acceptance
    }
    fn recovery(&self) -> Option<Recovery> {
        self.recovery
    }
    fn check_environment(&self) -> Result<()> {
        Ok(())
    }
    fn labor_enabled(&self) -> bool {
        self.enabled.get()
    }
    fn open_control(
        &self,
        mode: Mode,
        c: &OperationContext,
    ) -> Result<(
        WorkforceSession<Storage>,
        AssociationStore<Storage>,
        Digest32,
    )> {
        let mode = if mode == Mode::Control && !self.enabled.get() {
            WorkforceMode::Recover
        } else {
            mode.journal()
        };
        let journal = WorkforceJournal::open(self.native.journal.clone(), c, mode, None)?;
        let (native, view) = WorkforceSession::new(journal, c)?;
        let store = AssociationStore::open(
            self.native.associations.clone(),
            view.id,
            false,
            mode != WorkforceMode::Control,
            c,
        )?;
        Ok((native, store, view.id))
    }
    fn native(&self, binding: &WorkforceBinding, c: &OperationContext) -> Result<Native> {
        self.native.connect(binding, c)
    }
    fn acquire(&self, _: &OperationContext) -> Result<LiveObservationCapsule> {
        self.acquisitions.set(self.acquisitions.get() + 1);
        Ok(self.capsule.clone())
    }
    fn evidence(&self, capsule: LiveObservationCapsule, c: &OperationContext) -> Result<Observer> {
        let projection = project_live_capsule(&capsule, c.anchor.fortress_id, c.anchor.cursor)?;
        Ok(Observer {
            capsule,
            projection,
            refreshes: 0,
            fail_at: None,
            deny: false,
            corrupt_at: None,
            delay_at: None,
        })
    }
    fn create_archive(
        &self,
        p: &PreparedPlan,
        c: &OperationContext,
    ) -> Result<PlanArchiveStore<Storage>> {
        if self.archive_failure.get() {
            return Err(error(
                ErrorCode::CorruptLedger,
                "injected archive publication failure",
            ));
        }
        PlanArchiveStore::create(self.archive.clone(), p, super::archive_limits(), c)
    }
    fn recover_archive(
        &self,
        r: Recovery,
        c: &OperationContext,
    ) -> Result<PlanArchiveStore<Storage>> {
        PlanArchiveStore::open(self.archive.clone(), r.seal, super::archive_limits(), c)
    }
    fn create_identity(
        &self,
        p: &LiveObservationCapsule,
        c: &OperationContext,
    ) -> Result<CapsuleStore<Storage>> {
        CapsuleStore::create(self.identity.clone(), p, c)
    }
    fn recover_identity(
        &self,
        seal: Digest32,
        c: &OperationContext,
    ) -> Result<CapsuleStore<Storage>> {
        CapsuleStore::open(self.identity.clone(), seal, c)
    }
    fn history(
        &self,
        journal: Digest32,
        r: &SemanticWorkforceReview,
        initialize: bool,
        read_only: bool,
        c: &OperationContext,
    ) -> Result<GoalHistoryStore<Storage>> {
        GoalHistoryStore::open(self.history.clone(), journal, r, initialize, read_only, c)
    }
}
fn runtime_fixture() -> Result<(Runtime<MockShell>, MockShell)> {
    let f = fixture()?;
    let shell = MockShell {
        native: f.native,
        capsule: f.observer.capsule.clone(),
        acceptance: SourceAcceptance {
            kind: super::policy::ACCEPTANCE_KIND.into(),
            world_folder: "region1".into(),
            site_id: 4,
            df_version: "0.51.11".into(),
            dfhack_version: "0.51.11-r1".into(),
            citizen_bridge_version: "0.1.0".into(),
            citizen_generation: CITIZEN_GENERATION,
            workforce_generation: 7,
        },
        archive: Storage::default(),
        identity: Storage::default(),
        history: Storage::default(),
        recovery: None,
        enabled: Rc::new(Cell::new(true)),
        acquisitions: Rc::new(Cell::new(0)),
        archive_failure: Rc::new(Cell::new(false)),
        clock: None,
    };
    let mut budget = f.context.budget;
    budget.max_bytes = super::DEFAULT_BYTES;
    budget.max_output_tokens = super::MAX_OUTPUT_TOKENS;
    budget.max_entities = 8192;
    let runtime = Runtime::from_shell(shell.clone(), Mode::Control, SessionId::new(11), budget)?;
    Ok((runtime, shell))
}
fn request_for(view: &View) -> Result<Request> {
    let value = view.to_json();
    Request::parse(&serde_json::json!({"expected_observation":value["observation"]["seal"],"units":["43"],"labor":"MINE","enabled":true,"deadline_tick":220,"poll_interval_ticks":5,"stable_samples":2}).to_string())
}
fn prepare_runtime(r: &mut Runtime<MockShell>) -> Result<View> {
    let c = r.context(false)?;
    let observed = r.observe(&c)?;
    let request = request_for(&observed)?;
    let c = r.context(false)?;
    r.plan(request, &c)
}
fn capture_after(shell: &MockShell, tick: u64, enabled: bool) {
    let mut native = shell.native.data.borrow_mut();
    native.fixture.sequence = 4;
    native.fixture.tick = tick;
    native.fixture.citizens[0].labors = vec![u8::from(enabled), 0];
    native.fixture.details[0].members = if enabled { vec![42] } else { Vec::new() };
}
fn recovery(view: &View) -> Result<Recovery> {
    Ok(Recovery {
        seal: view.archive.ok_or_else(|| invalid("archive absent"))?,
        anchor: view
            .original
            .as_ref()
            .ok_or_else(|| invalid("original absent"))?
            .anchor,
        identity_digest: view
            .identity_digest
            .ok_or_else(|| invalid("identity absent"))?,
        history_present: view.history_present,
    })
}
#[test]
fn original_labor_goal_runs_once_then_requires_distinct_stable_current_samples() -> Result<()> {
    let (mut runtime, shell) = runtime_fixture()?;
    let prepared = prepare_runtime(&mut runtime)?;
    let seal = prepared
        .review
        .as_ref()
        .ok_or_else(|| invalid("review absent"))?
        .seal();
    let c = runtime.context(false)?;
    assert_eq!(
        runtime.commit(seal, false, &c).err().map(|e| e.code),
        Some(ErrorCode::CapabilityDenied)
    );
    let committed = runtime.commit(seal, true, &c)?;
    assert!(matches!(
        committed.action,
        Some(SingleLaborResult::Verified { .. })
    ));
    assert!(committed.goal.is_none());
    assert!(committed.observation.is_none());
    assert_eq!(shell.native.data.borrow().calls[2], 1);
    capture_after(&shell, 125, true);
    let c = runtime.context(false)?;
    let first = runtime.wait(&c)?;
    assert!(!first.goal.as_ref().is_some_and(|p| p.current_goal_proven()));
    let c = runtime.context(false)?;
    let same = runtime.wait(&c)?;
    assert!(!same.goal.as_ref().is_some_and(|p| p.current_goal_proven()));
    capture_after(&shell, 130, true);
    let c = runtime.context(false)?;
    let final_view = runtime.wait(&c)?;
    assert_eq!(
        final_view.to_json()["current_goal"]["semantic_completion_proven"],
        true
    );
    assert_eq!(shell.native.data.borrow().calls[2], 1);
    assert_eq!(
        final_view
            .original
            .as_ref()
            .and_then(|p| p.steps[0].obligation.as_ref())
            .map(|o| o.deadline_tick),
        Some(GameTick(220))
    );
    capture_after(&shell, 135, false);
    let c = runtime.context(false)?;
    let changed = runtime.wait(&c)?;
    assert_eq!(
        changed.to_json()["current_goal"]["original_goal_proven"],
        false
    );
    assert!(changed.first_satisfied_anchor.is_some());
    Ok(())
}
#[test]
fn durable_recovery_reauthorizes_original_identity_without_native_contact() -> Result<()> {
    let (mut runtime, mut shell) = runtime_fixture()?;
    let prepared = prepare_runtime(&mut runtime)?;
    let seal = prepared
        .review
        .as_ref()
        .ok_or_else(|| invalid("review absent"))?
        .seal();
    let c = runtime.context(false)?;
    runtime.commit(seal, true, &c)?;
    capture_after(&shell, 125, true);
    let c = runtime.context(false)?;
    runtime.wait(&c)?;
    capture_after(&shell, 130, true);
    let c = runtime.context(false)?;
    let done = runtime.wait(&c)?;
    let before = shell.native.data.borrow().calls;
    let reads = shell.acquisitions.get();
    let budget = runtime.budget();
    shell.recovery = Some(recovery(&done)?);
    drop(runtime);
    let mut offline =
        Runtime::from_shell(shell.clone(), Mode::Offline, SessionId::new(12), budget)?;
    let c = offline.context(false)?;
    let recovered = offline.wait(&c)?;
    assert!(recovered.goal.is_none());
    assert!(recovered.first_satisfied_anchor.is_some());
    assert!(matches!(
        recovered.historical_obligation,
        Some(dfmcp_intent::ObligationStatus::Fulfilled { .. })
    ));
    assert_eq!(shell.native.data.borrow().calls, before);
    assert_eq!(shell.acquisitions.get(), reads);
    assert!(offline.observe(&c).is_err());
    assert!(offline.commit(seal, true, &c).is_err());
    drop(offline);
    let mut query =
        Runtime::from_shell(shell.clone(), Mode::Reconcile, SessionId::new(13), budget)?;
    capture_after(&shell, 135, false);
    let c = query.context(false)?;
    let current = query.wait(&c)?;
    assert_eq!(
        current.to_json()["current_goal"]["original_goal_proven"],
        false
    );
    assert!(current.first_satisfied_anchor.is_some());
    assert!(!query.labor_control_enabled());
    Ok(())
}
#[test]
fn lost_assignment_reply_never_redispatches_and_unknown_cannot_be_goal_completion() -> Result<()> {
    let (mut runtime, shell) = runtime_fixture()?;
    let prepared = prepare_runtime(&mut runtime)?;
    let seal = prepared
        .review
        .as_ref()
        .ok_or_else(|| invalid("review absent"))?
        .seal();
    shell.native.data.borrow_mut().lose_commit = true;
    shell.native.data.borrow_mut().unknown = true;
    let c = runtime.context(false)?;
    assert!(runtime.commit(seal, true, &c).is_err());
    assert!(runtime.commit(seal, true, &c).is_err());
    assert_eq!(shell.native.data.borrow().calls[2], 1);
    capture_after(&shell, 125, true);
    let c = runtime.context(false)?;
    let first = runtime.wait(&c)?;
    assert_eq!(first.to_json()["native_phase"], "unknown");
    assert_eq!(first.to_json()["effect_uncertain"], true);
    let queries = shell.native.data.borrow().calls[3];
    capture_after(&shell, 130, true);
    let c = runtime.context(false)?;
    let goal = runtime.wait(&c)?;
    assert_eq!(goal.to_json()["current_goal"]["original_goal_proven"], true);
    assert_eq!(
        goal.to_json()["current_goal"]["semantic_completion_proven"],
        false
    );
    assert_eq!(shell.native.data.borrow().calls[3], queries);
    assert_eq!(shell.native.data.borrow().calls[2], 1);
    Ok(())
}
#[test]
fn exact_request_retries_reuse_native_preparation_and_cannot_replace_goal() -> Result<()> {
    let (mut runtime, shell) = runtime_fixture()?;
    let c = runtime.context(false)?;
    let observed = runtime.observe(&c)?;
    let request = request_for(&observed)?;
    let c = runtime.context(false)?;
    let first = runtime.plan(request.clone(), &c)?;
    let calls = shell.native.data.borrow().calls;
    let c = runtime.context(false)?;
    let repeated = runtime.plan(request.clone(), &c)?;
    assert_eq!(
        first.review.as_ref().map(|r| r.seal()),
        repeated.review.as_ref().map(|r| r.seal())
    );
    assert_eq!(calls, shell.native.data.borrow().calls);
    let mut changed = request;
    changed.deadline_tick += 10;
    assert_eq!(
        runtime.plan(changed, &c).err().map(|e| e.code),
        Some(ErrorCode::Conflict)
    );
    Ok(())
}
#[test]
fn immutable_control_modes_and_current_revocation_are_separate() -> Result<()> {
    let (runtime, shell) = runtime_fixture()?;
    let budget = runtime.budget();
    drop(runtime);
    shell.enabled.set(false);
    let mut owner = Runtime::from_shell(shell.clone(), Mode::Control, SessionId::new(15), budget)?;
    shell.enabled.set(true);
    assert!(!owner.labor_control_enabled());
    let c = owner.context(false)?;
    let observed = owner.observe(&c)?;
    let request = request_for(&observed)?;
    let c = owner.context(false)?;
    assert!(owner.plan(request, &c).is_err());
    drop(owner);
    let mut controlled =
        Runtime::from_shell(shell.clone(), Mode::Control, SessionId::new(16), budget)?;
    let prepared = prepare_runtime(&mut controlled)?;
    shell.enabled.set(false);
    let c = controlled.context(false)?;
    assert!(
        controlled
            .commit(
                prepared
                    .review
                    .as_ref()
                    .ok_or_else(|| invalid("review absent"))?
                    .seal(),
                true,
                &c
            )
            .is_err()
    );
    assert_eq!(shell.native.data.borrow().calls[2], 0);
    Ok(())
}
#[test]
fn identity_archive_is_exact_bounded_and_fences_changed_bytes() -> Result<()> {
    let (runtime, shell) = runtime_fixture()?;
    let mut c = shell.native.context.clone();
    c.budget = runtime.budget();
    assert_eq!(
        decode(&shell.capsule.canonical_bytes, shell.capsule.content_digest)?,
        shell.capsule
    );
    let mut trailing = shell.capsule.canonical_bytes.clone();
    trailing.push(0);
    assert!(decode(&trailing, Digest32::of_bytes(&trailing)).is_err());
    assert!(decode(&shell.capsule.canonical_bytes, Digest32::ZERO).is_err());
    let store = Storage::default();
    let mut archive = CapsuleStore::create(store.clone(), &shell.capsule, &c)?;
    assert_eq!(archive.recover(&c)?, shell.capsule);
    store.0.borrow_mut().file.get_mut()[0] ^= 1;
    assert!(archive.recover(&c).is_err());
    assert!(archive.is_fenced());
    store.0.borrow_mut().file.get_mut()[0] ^= 1;
    assert!(archive.recover(&c).is_err());
    Ok(())
}
#[test]
fn partial_publication_can_retry_only_original_identity_before_any_native_work() -> Result<()> {
    let (mut runtime, shell) = runtime_fixture()?;
    let c = runtime.context(false)?;
    let observed = runtime.observe(&c)?;
    let request = request_for(&observed)?;
    shell.archive_failure.set(true);
    let c = runtime.context(false)?;
    assert!(runtime.plan(request.clone(), &c).is_err());
    assert!(!shell.identity.0.borrow().file.get_ref().is_empty());
    assert!(shell.archive.0.borrow().file.get_ref().is_empty());
    assert_eq!(shell.native.data.borrow().calls, [0; 5]);
    shell.archive_failure.set(false);
    runtime.plan(request, &c)?;
    assert_eq!(shell.native.data.borrow().calls[1], 1);
    Ok(())
}
#[test]
fn request_and_retained_roster_pagination_are_closed_and_anchor_bound() -> Result<()> {
    let (mut runtime, shell) = runtime_fixture()?;
    let c = runtime.context(false)?;
    let observed = runtime.observe(&c)?;
    let request = request_for(&observed)?;
    let mut raw = serde_json::json!({"expected_observation":request.expected_observation,"units":["43"],"labor":"MINE","enabled":true,"deadline_tick":220});
    raw["detail"] = serde_json::json!(0);
    assert!(Request::parse(&raw.to_string()).is_err());
    raw.as_object_mut()
        .ok_or_else(|| invalid("object"))?
        .remove("detail");
    raw["units"] = serde_json::json!(["43", "43"]);
    assert!(Request::parse(&raw.to_string()).is_err());
    let seal = observed
        .observation
        .as_ref()
        .and_then(|v| v["seal"].as_str())
        .ok_or_else(|| invalid("seal"))?;
    let c = runtime.context(false)?;
    let before = shell.acquisitions.get();
    let page = runtime.citizens_page(super::digest(seal)?, 0, 1, &c)?;
    assert_eq!(page.to_json()["observation"]["units"][0]["entity_id"], "43");
    assert_eq!(shell.acquisitions.get(), before);
    assert!(
        runtime
            .citizens_page(Digest32::of_bytes(b"changed"), 0, 1, &c)
            .is_err()
    );
    assert!(
        runtime
            .citizens_page(super::digest(seal)?, 0, 33, &c)
            .is_err()
    );
    Ok(())
}
#[test]
fn archive_loss_after_dispatch_sync_blocks_assignment_and_preserves_native_uncertainty()
-> Result<()> {
    let (mut runtime, shell) = runtime_fixture()?;
    let prepared = prepare_runtime(&mut runtime)?;
    let seal = prepared
        .review
        .as_ref()
        .ok_or_else(|| invalid("review absent"))?
        .seal();
    let archive = shell.archive.clone();
    let next = shell.native.journal.0.borrow().syncs + 1;
    shell.native.journal.0.borrow_mut().on_sync = Some((
        next,
        Rc::new(move || {
            archive.0.borrow_mut().file.get_mut()[0] ^= 1;
        }),
    ));
    let c = runtime.context(false)?;
    assert!(runtime.commit(seal, true, &c).is_err());
    assert_eq!(shell.native.data.borrow().calls[2], 0);
    let mut journal = WorkforceJournal::open(
        Storage::from_bytes(shell.native.journal.0.borrow().synced.clone()),
        &c,
        WorkforceMode::Offline,
        None,
    )?;
    assert_eq!(
        journal.view(&c)?.records[0].state(),
        AssignmentState::DispatchStarted
    );
    assert!(runtime.commit(seal, true, &c).is_err());
    assert_eq!(shell.native.data.borrow().calls[2], 0);
    Ok(())
}
#[test]
fn authority_revocation_after_dispatch_sync_cannot_reach_native_assignment() -> Result<()> {
    let (mut runtime, shell) = runtime_fixture()?;
    let prepared = prepare_runtime(&mut runtime)?;
    let seal = prepared
        .review
        .as_ref()
        .ok_or_else(|| invalid("review absent"))?
        .seal();
    let enabled = Rc::clone(&shell.enabled);
    let next = shell.native.journal.0.borrow().syncs + 1;
    shell.native.journal.0.borrow_mut().on_sync = Some((next, Rc::new(move || enabled.set(false))));
    let c = runtime.context(false)?;
    assert_eq!(
        runtime.commit(seal, true, &c).err().map(|e| e.code),
        Some(ErrorCode::CapabilityDenied)
    );
    assert_eq!(shell.native.data.borrow().calls[2], 0);
    let c = runtime.context(false)?;
    let view = runtime.inspect(&c)?;
    assert_eq!(view.to_json()["native_state"], "dispatch_started");
    assert_eq!(view.to_json()["effect_uncertain"], true);
    assert!(!runtime.labor_control_enabled());
    Ok(())
}

#[test]
fn thirty_two_citizen_goal_uses_exact_paged_128_citizen_source_and_normalized_review() -> Result<()>
{
    let (runtime, mut shell) = runtime_fixture()?;
    let budget = runtime.budget();
    drop(runtime);
    let ids: Vec<i32> = (0..128).collect();
    shell.capsule = observation(&ids)?.0;
    shell.native.data.borrow_mut().fixture.citizens = (0..32)
        .map(|id| CitizenFixture {
            id,
            eligible: true,
            labors: vec![0, 0],
        })
        .collect();
    let mut runtime =
        Runtime::from_shell(shell.clone(), Mode::Control, SessionId::new(31), budget)?;
    let c = runtime.context(false)?;
    let observed = runtime.observe(&c)?;
    let source = observed.to_json();
    assert_eq!(source["observation"]["citizens"], 128);
    assert_eq!(
        source["observation"]["units"].as_array().map(Vec::len),
        Some(32)
    );
    assert_eq!(source["observation"]["complete"], false);
    let observation_seal = source["observation"]["seal"]
        .as_str()
        .ok_or_else(|| invalid("observation seal"))?;
    let seal = super::digest(observation_seal)?;
    let acquisitions = shell.acquisitions.get();
    let mut canonical_ids = Vec::new();
    for offset in [0, 32, 64, 96] {
        let c = runtime.context(false)?;
        let page = runtime.citizens_page(seal, offset, 32, &c)?.to_json();
        assert_eq!(page["anchor"], source["anchor"]);
        let page = &page["observation"];
        assert_eq!(page["seal"], observation_seal);
        assert_eq!(page["returned"], 32);
        assert_eq!(page["complete"], offset == 96);
        let rows = page["units"]
            .as_array()
            .ok_or_else(|| invalid("citizen rows"))?;
        for row in rows {
            canonical_ids.push(
                row["entity_id"]
                    .as_str()
                    .ok_or_else(|| invalid("canonical ID"))?
                    .to_owned(),
            );
        }
        if offset < 96 {
            assert_eq!(page["next_query"]["offset"], offset + 32);
            assert_eq!(page["next_query"]["expected_observation"], observation_seal);
        } else {
            assert!(page["next_query"].is_null());
        }
    }
    assert_eq!(
        canonical_ids,
        (1..=128).map(|id| id.to_string()).collect::<Vec<_>>()
    );
    assert_eq!(shell.acquisitions.get(), acquisitions);
    assert_eq!(shell.native.data.borrow().calls, [0; 5]);
    let mut request = Request {
        expected_observation: observation_seal.to_owned(),
        units: canonical_ids[..32].iter().rev().cloned().collect(),
        labor: "MINE".into(),
        enabled: true,
        deadline_tick: 220,
        poll_interval_ticks: 5,
        stable_samples: 1,
    };
    let c = runtime.context(false)?;
    let prepared = runtime.plan(request.clone(), &c)?;
    let paired = prepared
        .review
        .as_ref()
        .ok_or_else(|| invalid("review"))?
        .seal();
    assert_eq!(
        prepared
            .review
            .as_ref()
            .map(|r| r.native_plan().before().citizens().len()),
        Some(32)
    );
    let original = prepared
        .original
        .as_ref()
        .ok_or_else(|| invalid("original"))?;
    assert!(original.steps[0].compensation.is_none());
    let Action::SetLabor { units, .. } = &original.steps[0].action else {
        return Err(invalid("SetLabor expected"));
    };
    assert_eq!(units, &(1..=32).map(EntityId::new).collect::<Vec<_>>());
    let before = shell.native.data.borrow().calls;
    request.units.reverse();
    let c = runtime.context(false)?;
    let reordered = runtime.plan(request, &c)?;
    assert_eq!(reordered.review.as_ref().map(|r| r.seal()), Some(paired));
    assert_eq!(shell.native.data.borrow().calls, before);
    let c = runtime.context(false)?;
    let assigned = runtime.commit(paired, true, &c)?;
    assert_eq!(assigned.to_json()["action_result"]["verified"], true);
    {
        let mut native = shell.native.data.borrow_mut();
        native.fixture.sequence = 4;
        native.fixture.tick = 125;
        for citizen in &mut native.fixture.citizens {
            citizen.labors = vec![1, 0];
        }
        native.fixture.details[0].members = (0..32).collect();
    }
    let c = runtime.context(false)?;
    let done = runtime.wait(&c)?;
    assert_eq!(
        done.to_json()["current_goal"]["semantic_completion_proven"],
        true
    );
    assert_eq!(shell.native.data.borrow().calls[2], 1);
    Ok(())
}

#[test]
fn restart_discards_unfinished_stability_and_cannot_renew_original_deadline() -> Result<()> {
    let (mut runtime, mut shell) = runtime_fixture()?;
    let prepared = prepare_runtime(&mut runtime)?;
    let paired = prepared
        .review
        .as_ref()
        .ok_or_else(|| invalid("review"))?
        .seal();
    let original = prepared
        .original
        .clone()
        .ok_or_else(|| invalid("original"))?;
    let c = runtime.context(false)?;
    runtime.commit(paired, true, &c)?;
    capture_after(&shell, 125, true);
    let c = runtime.context(false)?;
    let interrupted = runtime.wait(&c)?;
    assert_eq!(
        interrupted.to_json()["current_goal"]["current_truth"],
        "true"
    );
    assert_eq!(
        interrupted.to_json()["current_goal"]["original_goal_proven"],
        false
    );
    assert!(interrupted.first_satisfied_anchor.is_none());
    let before = shell.native.data.borrow().calls;
    let reads = shell.acquisitions.get();
    let budget = runtime.budget();
    shell.recovery = Some(recovery(&interrupted)?);
    drop(runtime);
    let mut recovered =
        Runtime::from_shell(shell.clone(), Mode::Reconcile, SessionId::new(32), budget)?;
    assert_eq!(shell.native.data.borrow().calls, before);
    assert_eq!(shell.acquisitions.get(), reads);
    let c = recovered.context(false)?;
    let opening = recovered.inspect(&c)?;
    assert!(opening.goal.is_none());
    assert!(opening.first_satisfied_anchor.is_none());
    assert_eq!(opening.original, Some(original.clone()));
    capture_after(&shell, 130, true);
    let c = recovered.context(false)?;
    let first_after_restart = recovered.wait(&c)?;
    assert_eq!(
        first_after_restart.to_json()["current_goal"]["current_truth"],
        "true"
    );
    assert_eq!(
        first_after_restart.to_json()["current_goal"]["original_goal_proven"],
        false
    );
    let c = recovered.context(false)?;
    let duplicate = recovered.wait(&c)?;
    assert_eq!(
        duplicate.to_json()["current_goal"]["original_goal_proven"],
        false
    );
    capture_after(&shell, 221, true);
    let c = recovered.context(false)?;
    let expired = recovered.wait(&c)?;
    assert_eq!(expired.original, Some(original));
    assert_eq!(expired.to_json()["current_goal"]["current_truth"], "true");
    assert_eq!(
        expired.to_json()["current_goal"]["semantic_completion_proven"],
        false
    );
    assert!(matches!(
        expired.historical_obligation,
        Some(dfmcp_intent::ObligationStatus::Failed { .. })
    ));
    assert!(expired.first_satisfied_anchor.is_none());
    assert_eq!(
        expired
            .original
            .as_ref()
            .and_then(|p| p.steps[0].obligation.as_ref())
            .map(|o| o.deadline_tick),
        Some(GameTick(220))
    );
    assert_eq!(shell.native.data.borrow().calls[2], 1);
    Ok(())
}

#[test]
fn full_sixty_four_sample_goal_recovers_with_explicit_allowance_without_native_replay() -> Result<()>
{
    let (mut runtime, mut shell) = runtime_fixture()?;
    assert_eq!(runtime.budget().max_bytes, super::DEFAULT_BYTES);
    let c = runtime.context(false)?;
    let observed = runtime.observe(&c)?;
    let mut request = request_for(&observed)?;
    request.deadline_tick = 600;
    request.stable_samples = 64;
    let c = runtime.context(false)?;
    let prepared = runtime.plan(request, &c)?;
    let paired = prepared
        .review
        .as_ref()
        .ok_or_else(|| invalid("review"))?
        .seal();
    let c = runtime.context(false)?;
    runtime.commit(paired, true, &c)?;
    let mut completed = None;
    for sample in 1..=64 {
        capture_after(&shell, 120 + sample * 5, true);
        let c = runtime.context(false)?;
        let view = runtime.wait(&c)?;
        assert_eq!(
            view.to_json()["current_goal"]["original_goal_proven"],
            sample == 64
        );
        completed = Some(view);
    }
    let completed = completed.ok_or_else(|| invalid("completed goal"))?;
    assert_eq!(completed.history_events, 64 * 3);
    assert_eq!(
        completed.first_satisfied_anchor.map(|a| a.tick),
        Some(GameTick(440))
    );
    let original = completed.original.clone();
    shell.recovery = Some(recovery(&completed)?);
    let budget = runtime.budget();
    let calls = shell.native.data.borrow().calls;
    let acquisitions = shell.acquisitions.get();
    let history = shell.history.0.borrow().file.get_ref().clone();
    drop(runtime);
    let refusal = Runtime::from_shell(shell.clone(), Mode::Offline, SessionId::new(41), budget);
    assert_eq!(
        refusal.err().map(|e| e.code),
        Some(ErrorCode::BudgetExceeded)
    );
    assert_eq!(shell.native.data.borrow().calls, calls);
    assert_eq!(shell.acquisitions.get(), acquisitions);
    assert_eq!(shell.history.0.borrow().file.get_ref(), &history);
    let mut explicit = budget;
    explicit.max_bytes = 8 * 1024 * 1024 * 1024;
    let mut recovered =
        Runtime::from_shell(shell.clone(), Mode::Offline, SessionId::new(42), explicit)?;
    let c = recovered.context(false)?;
    let view = recovered.inspect(&c)?;
    assert_eq!(view.original, original);
    assert_eq!(view.history_events, completed.history_events);
    assert_eq!(
        view.first_satisfied_anchor,
        completed.first_satisfied_anchor
    );
    assert!(matches!(
        view.historical_obligation,
        Some(dfmcp_intent::ObligationStatus::Fulfilled { .. })
    ));
    assert!(view.goal.is_none());
    assert_eq!(view.to_json()["current_goal_available"], false);
    assert_eq!(shell.native.data.borrow().calls, calls);
    assert_eq!(shell.acquisitions.get(), acquisitions);
    assert_eq!(shell.history.0.borrow().file.get_ref(), &history);
    assert_eq!(calls[2], 1);
    Ok(())
}
