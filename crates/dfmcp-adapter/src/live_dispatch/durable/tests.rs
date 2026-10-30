use super::*;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Read, Seek, Write};
use std::rc::Rc;

use dfmcp_core::{
    CapabilityScope, EntityId, FortressId, IntentId, ObservationCursor, RequestId, StateAnchor,
    WorkBudget,
};
use dfmcp_intent::Action;
use dfmcp_world::Predicate;

#[derive(Default)]
struct Disk {
    bytes: Vec<u8>,
    writes: usize,
    syncs: usize,
    fail_sync: bool,
    partial: Option<usize>,
    invalid: bool,
    delay: u64,
}
#[derive(Clone, Default)]
struct Memory {
    disk: Rc<RefCell<Disk>>,
    position: usize,
}
impl Memory {
    fn stored_bytes(&self) -> Vec<u8> {
        self.disk.borrow().bytes.clone()
    }
    fn copied(raw: Vec<u8>) -> Self {
        Self {
            disk: Rc::new(RefCell::new(Disk {
                bytes: raw,
                ..Disk::default()
            })),
            position: 0,
        }
    }
}
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let disk = self.disk.borrow();
        let count = out
            .len()
            .min(disk.bytes.len().saturating_sub(self.position));
        out[..count].copy_from_slice(&disk.bytes[self.position..self.position + count]);
        self.position += count;
        Ok(count)
    }
}
impl Seek for Memory {
    fn seek(&mut self, at: SeekFrom) -> io::Result<u64> {
        let next = match at {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::End(n) => self.disk.borrow().bytes.len() as i128 + i128::from(n),
            SeekFrom::Current(n) => self.position as i128 + i128::from(n),
        };
        self.position = usize::try_from(next).map_err(|_| io::Error::other("invalid seek"))?;
        Ok(self.position as u64)
    }
}
impl Write for Memory {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut disk = self.disk.borrow_mut();
        if self.position != disk.bytes.len() {
            return Err(io::Error::other("nonappend write"));
        }
        let count = disk
            .partial
            .map_or(bytes.len(), |remaining| remaining.min(bytes.len()));
        if count == 0 {
            return Err(io::Error::other("torn append"));
        }
        disk.bytes.extend_from_slice(&bytes[..count]);
        disk.writes += 1;
        disk.partial = disk.partial.map(|remaining| remaining - count);
        self.position += count;
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl EffectJournalStorage for Memory {
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("repair forbidden"))
    }
    fn sync(&mut self) -> io::Result<()> {
        let delay = self.disk.borrow().delay;
        if delay > 0 {
            std::thread::sleep(Duration::from_millis(delay));
        }
        let mut disk = self.disk.borrow_mut();
        disk.syncs += 1;
        if disk.fail_sync {
            Err(io::Error::other("lost sync acknowledgment"))
        } else {
            Ok(())
        }
    }
    fn validate_identity(&self) -> io::Result<()> {
        if self.disk.borrow().invalid {
            Err(io::Error::other("substituted file"))
        } else {
            Ok(())
        }
    }
}

fn anchor() -> StateAnchor {
    StateAnchor {
        fortress_id: FortressId::new(9),
        cursor: ObservationCursor {
            epoch: 2,
            sequence: 1,
        },
        tick: GameTick(100),
        state_hash: Digest32::of_bytes(b"paused-original"),
    }
}
fn plan(count: usize) -> PreparedPlan {
    let mut required = BTreeSet::new();
    let steps: Vec<_> = (0..count)
        .map(|index| {
            let id = StepId::new(index as u32);
            let action = if index % 2 == 0 {
                Action::Pause { paused: true }
            } else {
                Action::SetLabor {
                    units: vec![EntityId::new(17)],
                    labor: "MINE".into(),
                    enabled: true,
                }
            };
            required.insert(action.capability());
            let key =
                dfmcp_intent::derive_step_idempotency_key(IntentId::new(77), anchor(), id, &action);
            PlanStep {
                id,
                required_capability: action.capability(),
                risk: action.risk(),
                action,
                preconditions: vec![Predicate::Paused(true)],
                postconditions: vec![Predicate::Paused(true)],
                compensation: None,
                obligation: None,
                depends_on: if index == 0 {
                    Vec::new()
                } else {
                    vec![StepId::new(index as u32 - 1)]
                },
                idempotency_key: key,
            }
        })
        .collect();
    let max_risk = steps
        .iter()
        .map(|step| step.risk)
        .max()
        .map_or(RiskTier::ReadOnly, |risk| risk);
    PreparedPlan::builder(
        IntentId::new(77),
        anchor(),
        "original mixed plan",
        Predicate::Paused(true),
    )
    .steps(steps)
    .max_risk(max_risk)
    .required_capabilities(required)
    .expires_at_tick(GameTick(500))
    .build()
}
fn context() -> OperationContext {
    OperationContext {
        session_id: SessionId::new(18),
        request_id: RequestId::new(1),
        anchor: anchor(),
        budget: WorkBudget {
            max_wall_millis: 60_000,
            max_game_ticks: 0,
            max_bytes: 64 * 1024 * 1024,
            max_entities: 1024,
            max_output_tokens: 4096,
            max_actions: 16,
        },
        grants: [
            Capability::Query,
            Capability::Plan,
            Capability::ControlClock,
            Capability::ConfigureLabor,
        ]
        .into_iter()
        .map(|capability| CapabilityGrant {
            capability,
            max_risk: RiskTier::Guarded,
            scope: CapabilityScope {
                fortress_id: Some(anchor().fortress_id),
                ..CapabilityScope::default()
            },
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
        cancellation_requested: false,
    }
}
fn created(count: usize) -> Result<(Coordinator<Memory>, Memory, OperationContext, Fake)> {
    let c = context();
    let plan = plan(count);
    plan.validate_structure()?;
    let pin = initial_recovery(&plan, &c)?;
    let disk = Memory::default();
    let owner = Coordinator::open(disk.clone(), &c, BuildMode::Control, Some(plan), pin, true)?;
    let fake = Fake {
        disk: disk.clone(),
        pin,
        ..Fake::default()
    };
    Ok((owner, disk, c, fake))
}
#[derive(Default)]
struct Fake {
    disk: Memory,
    pin: Recovery,
    prepare: usize,
    commit: usize,
    recover: usize,
    cancel: usize,
    outcomes: BTreeMap<StepId, RecoveredFamily>,
    fail_prepare: bool,
    lose_commit: bool,
    fail_result_sync: bool,
    recover_override: Option<RecoveredFamily>,
    floor: u64,
    commit_floor: Option<u64>,
    prepare_floor: Option<u64>,
    abandon: usize,
    local_permit: bool,
    corrupt_recovery: bool,
    walls: Vec<u64>,
}
impl Default for Recovery {
    fn default() -> Self {
        Self {
            journal_id: Digest32::ZERO,
            head: Digest32::ZERO,
            authority_tick_floor: 0,
        }
    }
}
impl Fake {
    fn durable(&self, step: StepId, phase: Phase, c: &OperationContext) -> Result<()> {
        let mut owner = Coordinator::open(
            self.disk.clone(),
            c,
            BuildMode::Offline,
            None,
            self.pin,
            false,
        )?;
        let view = owner.inspect(c)?;
        assert_eq!(view.entries[entry_index(&view.entries, step)?].phase, phase);
        Ok(())
    }
    fn receipt(route: &RoutedStep, step: &PlanStep) -> Result<FamilyReceipt> {
        FamilyReceipt::new(
            route.family,
            Digest32::of_bytes(format!("receipt:{}", step.idempotency_key).as_bytes()),
        )
    }
}
impl LiveFamilyExecutor for Fake {
    fn prepare(
        &mut self,
        _: &PreparedPlan,
        step: &PlanStep,
        route: &RoutedStep,
        c: &OperationContext,
    ) -> Result<FamilyPreparation> {
        self.prepare += 1;
        self.walls.push(c.budget.max_wall_millis);
        self.durable(step.id, Phase::PrepareStarted, c)?;
        let value = FamilyPreparation::new(
            route.family,
            Digest32::of_bytes(step.idempotency_key.as_bytes()),
        )?;
        self.outcomes
            .insert(step.id, RecoveredFamily::Prepared(value));
        self.local_permit = true;
        if let Some(floor) = self.prepare_floor {
            self.floor = floor;
        }
        if self.fail_prepare {
            return Err(error(
                ErrorCode::AdapterUnavailable,
                "lost family prepare response",
            ));
        }
        if self.fail_result_sync {
            self.disk.disk.borrow_mut().fail_sync = true;
        }
        Ok(value)
    }
    fn commit(
        &mut self,
        _: &PreparedPlan,
        step: &PlanStep,
        route: &RoutedStep,
        _: FamilyPreparation,
        c: &OperationContext,
    ) -> Result<FamilyReceipt> {
        self.commit += 1;
        self.local_permit = false;
        self.walls.push(c.budget.max_wall_millis);
        self.durable(step.id, Phase::DispatchStarted, c)?;
        let value = Self::receipt(route, step)?;
        self.outcomes
            .insert(step.id, RecoveredFamily::Applied(value));
        if let Some(floor) = self.commit_floor {
            self.floor = floor;
        }
        if self.lose_commit {
            return Err(error(
                ErrorCode::EffectIndeterminate,
                "lost family commit response",
            ));
        }
        if self.fail_result_sync {
            self.disk.disk.borrow_mut().fail_sync = true;
        }
        Ok(value)
    }
    fn reconcile(
        &mut self,
        _: &PreparedPlan,
        _: &PlanStep,
        _: &RoutedStep,
        _: &OperationContext,
    ) -> Result<super::super::ReconcileOutcome> {
        Err(error(
            ErrorCode::InternalInvariantViolation,
            "durable owner must use exact reconstruction seam",
        ))
    }
    fn cancel_prepared(
        &mut self,
        _: &PreparedPlan,
        step: &PlanStep,
        _: &RoutedStep,
        _: FamilyPreparation,
        c: &OperationContext,
    ) -> Result<Option<FamilyReceipt>> {
        self.cancel += 1;
        self.walls.push(c.budget.max_wall_millis);
        self.durable(step.id, Phase::CancelStarted, c)?;
        self.outcomes
            .insert(step.id, RecoveredFamily::Cancelled(None));
        Ok(None)
    }
}
impl DurableFamilyExecutor for Fake {
    fn recover_step(
        &mut self,
        _: &PreparedPlan,
        step: &PlanStep,
        _: &RoutedStep,
        c: &OperationContext,
    ) -> Result<RecoveredFamily> {
        self.recover += 1;
        if self.corrupt_recovery {
            return Err(corrupt());
        }
        self.walls.push(c.budget.max_wall_millis);
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        Ok(self
            .recover_override
            .or_else(|| self.outcomes.get(&step.id).copied())
            .map_or(RecoveredFamily::Indeterminate, |value| value))
    }
    fn observed_tick_floor(&self) -> u64 {
        self.floor
    }
    fn abandon_preparation(&mut self) {
        self.abandon += 1;
        self.local_permit = false;
    }
}

#[test]
fn full_plan_survives_all_16_steps_and_each_native_boundary_follows_durable_intent() -> Result<()> {
    let (mut owner, disk, c, mut fake) = created(16)?;
    let original = plan(16);
    for index in 0..16 {
        let review = owner.prepare_next(&mut fake, &c)?;
        assert_eq!(review.step, StepId::new(index));
        let view = owner.commit(review.seal, true, &mut fake, &c)?;
        assert_eq!(view.entries[index as usize].phase, Phase::Applied);
        assert!(!view.original_goal_proven());
    }
    let view = owner.inspect(&c)?;
    assert!(view.all_applied());
    assert!(view.next_ready.is_none());
    let pin = owner.recovery();
    drop(owner);
    let mut offline = Coordinator::open(
        disk,
        &c,
        BuildMode::Offline,
        Some(original.clone()),
        pin,
        false,
    )?;
    assert_eq!(offline.original_plan(), &original);
    assert!(offline.inspect(&c)?.all_applied());
    assert_eq!((fake.prepare, fake.commit), (16, 16));
    Ok(())
}

#[test]
fn lost_prepare_reopens_without_review_and_only_explicit_query_or_cancel_remains() -> Result<()> {
    let (mut owner, disk, c, mut fake) = created(2)?;
    fake.fail_prepare = true;
    assert!(owner.prepare_next(&mut fake, &c).is_err());
    assert!(owner.inspect(&c)?.needs_reconciliation());
    assert!(owner.prepare_next(&mut fake, &c).is_err());
    assert_eq!(fake.prepare, 1);
    let pin = owner.recovery();
    drop(owner);
    let mut recovered = Coordinator::open(disk, &c, BuildMode::Recover, None, pin, false)?;
    let view = recovered.reconcile(StepId::ZERO, &mut fake, &c)?;
    assert_eq!(view.entries[0].phase, Phase::Prepared);
    assert!(view.review.is_none());
    assert!(
        recovered
            .commit(Digest32::of_bytes(b"invented review"), true, &mut fake, &c)
            .is_err()
    );
    assert!(recovered.prepare_next(&mut fake, &c).is_err());
    let view = recovered.cancel_prepared(StepId::ZERO, &mut fake, &c)?;
    assert_eq!(view.entries[0].phase, Phase::Cancelled);
    assert!(view.next_ready.is_none());
    assert_eq!((fake.prepare, fake.commit, fake.cancel), (1, 0, 1));
    Ok(())
}

#[test]
fn lost_commit_reconciles_under_query_after_expiry_without_redispatch() -> Result<()> {
    let (mut owner, disk, c, mut fake) = created(2)?;
    let review = owner.prepare_next(&mut fake, &c)?;
    fake.lose_commit = true;
    assert!(owner.commit(review.seal, true, &mut fake, &c).is_err());
    assert!(owner.commit(review.seal, true, &mut fake, &c).is_err());
    assert!(owner.prepare_next(&mut fake, &c).is_err());
    let pin = owner.recovery();
    drop(owner);
    let mut query = c.clone();
    query.session_id = SessionId::new(19);
    query.anchor.tick = GameTick(500);
    query
        .grants
        .retain(|grant| grant.capability == Capability::Query);
    query.budget.max_actions = 1;
    let mut recovered = Coordinator::open(disk, &query, BuildMode::Recover, None, pin, false)?;
    let view = recovered.reconcile(StepId::ZERO, &mut fake, &query)?;
    assert_eq!(view.entries[0].phase, Phase::Applied);
    assert!(view.entries[0].dispatch_attempted);
    assert_eq!((fake.prepare, fake.commit, fake.recover), (1, 1, 1));
    assert!(!view.original_goal_proven());
    Ok(())
}

#[test]
fn contradictory_terminal_recovery_fences_before_any_successor_prepare() -> Result<()> {
    for variant in 0..4 {
        let (mut owner, _, c, mut fake) = created(2)?;
        let review = owner.prepare_next(&mut fake, &c)?;
        owner.commit(review.seal, true, &mut fake, &c)?;
        fake.recover_override = Some(match variant {
            0 => RecoveredFamily::Indeterminate,
            1 => RecoveredFamily::AbsentBeforeDispatch,
            2 => RecoveredFamily::Prepared(review.preparation),
            _ => RecoveredFamily::Applied(FamilyReceipt::new(
                review.family,
                Digest32::of_bytes(b"substituted receipt"),
            )?),
        });
        assert!(owner.reconcile(StepId::ZERO, &mut fake, &c).is_err());
        assert!(owner.is_fenced());
        assert!(owner.prepare_next(&mut fake, &c).is_err());
        assert_eq!(fake.prepare, 1);
    }
    Ok(())
}

#[test]
fn complete_result_with_lost_sync_ack_recovers_descendant_and_torn_tail_is_never_repaired()
-> Result<()> {
    let (mut owner, disk, c, mut fake) = created(2)?;
    let pin = owner.recovery();
    fake.fail_result_sync = true;
    assert!(owner.prepare_next(&mut fake, &c).is_err());
    assert!(owner.is_fenced());
    disk.disk.borrow_mut().fail_sync = false;
    drop(owner);
    let mut recovered = Coordinator::open(disk.clone(), &c, BuildMode::Recover, None, pin, false)?;
    let view = recovered.inspect(&c)?;
    assert_eq!(view.entries[0].phase, Phase::Prepared);
    assert!(view.review.is_none());
    recovered.verify_recovery(&c, pin)?;
    let bytes = disk.stored_bytes();
    for cut in [bytes.len() - 1, bytes.len() - 31] {
        let torn = Memory::copied(bytes[..cut].to_vec());
        let before = torn.stored_bytes();
        assert!(Coordinator::open(torn.clone(), &c, BuildMode::Recover, None, pin, false).is_err());
        assert_eq!(torn.stored_bytes(), before);
    }
    Ok(())
}

#[test]
fn last_sync_delay_shrinks_prepaid_family_wall_allowance_before_contact() -> Result<()> {
    let (mut owner, disk, mut c, mut fake) = created(2)?;
    c.budget.max_wall_millis = 1000;
    disk.disk.borrow_mut().delay = 60;
    let review = owner.prepare_next(&mut fake, &c)?;
    assert!(
        fake.walls[0] <= 940,
        "family allowance did not shrink after synchronized intent"
    );
    owner.commit(review.seal, true, &mut fake, &c)?;
    assert!(fake.walls[1] <= 940);
    Ok(())
}

#[test]
fn retained_grants_expiry_foreign_context_and_action_budget_refuse_before_family_contact()
-> Result<()> {
    let (mut owner, _, c, mut fake) = created(2)?;
    for variant in 0..5 {
        let mut denied = c.clone();
        match variant {
            0 => denied.session_id = SessionId::new(999),
            1 => denied.anchor.fortress_id = FortressId::new(999),
            2 => denied
                .grants
                .retain(|grant| grant.capability != Capability::ControlClock),
            3 => denied.budget.max_actions = 0,
            _ => denied.anchor.tick = GameTick(500),
        }
        assert!(owner.prepare_next(&mut fake, &denied).is_err());
    }
    assert_eq!(fake.prepare, 0);
    let (mut opened, disk, c, _) = created(2)?;
    let pin = opened.recovery();
    opened.abandon_review();
    drop(opened);
    let mut query = c.clone();
    query
        .grants
        .retain(|grant| grant.capability == Capability::Query);
    let mut query_owner = Coordinator::open(disk, &query, BuildMode::Control, None, pin, false)?;
    assert!(query_owner.prepare_next(&mut fake, &c).is_err());
    assert_eq!(fake.prepare, 0);
    Ok(())
}

#[test]
fn consumed_future_tick_prevents_receipt_publication_under_expired_query() -> Result<()> {
    let (mut owner, _, mut c, mut fake) = created(2)?;
    for grant in &mut c.grants {
        if grant.capability == Capability::Query {
            grant.expires_at_tick = Some(GameTick(200));
        }
    }
    let review = owner.prepare_next(&mut fake, &c)?;
    fake.commit_floor = Some(300);
    assert!(owner.commit(review.seal, true, &mut fake, &c).is_err());
    assert!(owner.is_fenced());
    assert_eq!(owner.high_tick(), 300);
    assert_eq!(fake.commit, 1);
    assert!(owner.inspect(&c).is_err());
    Ok(())
}

#[test]
fn rollback_foreign_pin_and_source_substitution_refuse_without_native_calls() -> Result<()> {
    let (mut owner, disk, c, mut fake) = created(2)?;
    let earlier = disk.stored_bytes();
    let first = owner.recovery();
    owner.prepare_next(&mut fake, &c)?;
    let later = owner.recovery();
    assert!(
        Coordinator::open(
            Memory::copied(earlier),
            &c,
            BuildMode::Offline,
            None,
            later,
            false
        )
        .is_err()
    );
    let foreign = Recovery {
        journal_id: Digest32::of_bytes(b"foreign"),
        ..first
    };
    assert!(Coordinator::open(disk.clone(), &c, BuildMode::Offline, None, foreign, false).is_err());
    disk.disk.borrow_mut().bytes[30] ^= 1;
    assert!(owner.inspect(&c).is_err());
    assert!(owner.is_fenced());
    assert!(
        owner
            .commit(Digest32::of_bytes(b"old"), true, &mut fake, &c)
            .is_err()
    );
    assert_eq!(fake.commit, 0);
    Ok(())
}

#[test]
fn failed_future_tick_handoff_survives_reopen_and_next_authorized_frame() -> Result<()> {
    let (mut owner, disk, c, mut fake) = created(2)?;
    let review = owner.prepare_next(&mut fake, &c)?;
    fake.lose_commit = true;
    fake.commit_floor = Some(300);
    assert!(owner.commit(review.seal, true, &mut fake, &c).is_err());
    let pin = owner.recovery();
    assert_eq!(pin.authority_tick_floor, 300);
    assert!(owner.stored_tick < pin.authority_tick_floor);
    assert!(!fake.local_permit);
    drop(owner);
    let mut expired = c.clone();
    expired
        .grants
        .retain(|grant| grant.capability == Capability::Query);
    for grant in &mut expired.grants {
        grant.expires_at_tick = Some(GameTick(200));
    }
    let before = disk.stored_bytes();
    assert!(
        Coordinator::open(disk.clone(), &expired, BuildMode::Recover, None, pin, false).is_err()
    );
    assert_eq!(disk.stored_bytes(), before);
    let mut query = c.clone();
    query
        .grants
        .retain(|grant| grant.capability == Capability::Query);
    query.budget.max_actions = 1;
    let mut recovered =
        Coordinator::open(disk.clone(), &query, BuildMode::Recover, None, pin, false)?;
    assert_eq!(recovered.high_tick(), 300);
    let lower = Recovery {
        authority_tick_floor: 100,
        ..pin
    };
    assert_eq!(recovered.verify_recovery(&query, lower)?.high_tick, 300);
    let view = recovered.reconcile(StepId::ZERO, &mut fake, &query)?;
    assert_eq!(view.entries[0].phase, Phase::Applied);
    assert_eq!(recovered.stored_tick, 300);
    let retained = recovered.recovery();
    drop(recovered);
    let mut offline = Coordinator::open(disk, &query, BuildMode::Offline, None, retained, false)?;
    assert_eq!(offline.inspect(&query)?.high_tick, 300);
    assert_eq!(fake.commit, 1);
    Ok(())
}

#[test]
fn expired_prepare_output_has_no_local_review_or_family_permit() -> Result<()> {
    for expire_plan in [false, true] {
        let (mut owner, disk, mut c, mut fake) = created(2)?;
        fake.prepare_floor = Some(if expire_plan { 500 } else { 300 });
        if !expire_plan {
            for grant in &mut c.grants {
                if grant.capability == Capability::Plan {
                    grant.expires_at_tick = Some(GameTick(200));
                }
            }
        }
        assert!(owner.prepare_next(&mut fake, &c).is_err());
        assert!(owner.is_fenced());
        assert!(owner.review.is_none());
        assert!(!fake.local_permit);
        assert_eq!(fake.abandon, 1);
        let pin = owner.recovery();
        drop(owner);
        let mut query = context();
        query
            .grants
            .retain(|grant| grant.capability == Capability::Query);
        let mut recovered = Coordinator::open(disk, &query, BuildMode::Recover, None, pin, false)?;
        assert_eq!(recovered.inspect(&query)?.entries[0].phase, Phase::Prepared);
        assert_eq!(fake.commit, 0);
    }
    Ok(())
}

#[test]
fn retention_preserves_a_complete_prepare_and_commit_lifecycle_before_contact() -> Result<()> {
    for spare in [1, 3, 4] {
        let (owner, disk, c, mut fake) = created(2)?;
        let pin = owner.recovery();
        let mut bytes = disk.stored_bytes();
        let mut head = pin.head;
        for sequence in 0..MAX_FRAMES - spare {
            let frame = wire::frame(sequence, head, 100, &Event::Floor)?;
            head = wire::tail(&frame)?;
            bytes.extend_from_slice(&frame);
        }
        drop(owner);
        disk.disk.borrow_mut().bytes = bytes;
        let mut owner = Coordinator::open(disk.clone(), &c, BuildMode::Control, None, pin, false)?;
        if spare < 4 {
            let before = disk.stored_bytes();
            assert!(owner.prepare_next(&mut fake, &c).is_err());
            assert_eq!(disk.stored_bytes(), before);
            assert_eq!(fake.prepare, 0);
        } else {
            let review = owner.prepare_next(&mut fake, &c)?;
            let view = owner.commit(review.seal, true, &mut fake, &c)?;
            assert_eq!(view.entries[0].phase, Phase::Applied);
            assert_eq!(owner.frames, MAX_FRAMES);
            assert_eq!(fake.prepare, 1);
            assert_eq!(fake.commit, 1);
        }
    }
    Ok(())
}

#[test]
fn terminal_custody_loss_and_wrong_family_proof_fence_successors() -> Result<()> {
    for custody_loss in [false, true] {
        let (mut owner, _, c, mut fake) = created(2)?;
        let review = owner.prepare_next(&mut fake, &c)?;
        owner.commit(review.seal, true, &mut fake, &c)?;
        if custody_loss {
            fake.corrupt_recovery = true;
        } else {
            fake.recover_override = Some(RecoveredFamily::Applied(FamilyReceipt::new(
                LiveFamily::WorkforceV1_17,
                Digest32::of_bytes(b"foreign family receipt"),
            )?));
        }
        assert!(owner.reconcile(StepId::ZERO, &mut fake, &c).is_err());
        assert!(owner.is_fenced());
        assert!(owner.prepare_next(&mut fake, &c).is_err());
        assert_eq!(fake.prepare, 1);
    }
    Ok(())
}

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
#[path = "private_tests.rs"]
mod private_tests;

#[path = "authority_tests.rs"]
mod authority_tests;
