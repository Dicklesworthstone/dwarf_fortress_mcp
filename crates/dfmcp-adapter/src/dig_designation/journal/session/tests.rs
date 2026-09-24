use super::super::DigStage;
use super::*;
use crate::dig_designation::rpc::{DigManifest, DigPreparation};
use crate::dig_designation::tests::{fixture, plan};
use crate::dig_designation::{DigEffect, DigPhase};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, GameTick, MapCoord, MapCuboid, ObservationCursor,
    RequestId, RiskTier, SessionId, StateAnchor, WorkBudget,
};
use std::cell::RefCell;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::net::SocketAddr;
use std::rc::Rc;

#[derive(Clone, Default)]
struct Memory(Rc<RefCell<Cursor<Vec<u8>>>>);
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.0.borrow_mut().read(out)
    }
}
impl Write for Memory {
    fn write(&mut self, raw: &[u8]) -> io::Result<usize> {
        let mut disk = self.0.borrow_mut();
        if disk.position() != disk.get_ref().len() as u64 {
            return Err(io::Error::other("non-append"));
        }
        disk.write(raw)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Seek for Memory {
    fn seek(&mut self, at: SeekFrom) -> io::Result<u64> {
        self.0.borrow_mut().seek(at)
    }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("repair forbidden"))
    }
}
fn scope() -> MapCuboid {
    MapCuboid {
        min: MapCoord::new(0, 0, 0),
        max: MapCoord::new(63, 63, 7),
    }
}
fn binding() -> Result<DigBinding> {
    DigBinding::new(
        "127.0.0.1:5000".parse().map_err(|_| exhausted())?,
        DigManifest {
            generation: 7,
            df_version: "test-df".into(),
            dfhack_version: "test-dfhack".into(),
        },
        plan()?.before(),
        scope(),
    )
}
fn context() -> Result<OperationContext> {
    let p = plan()?;
    Ok(OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: StateAnchor {
            fortress_id: p.before().fortress_id(),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(p.before().tick()),
            state_hash: p.before().witness(),
        },
        budget: WorkBudget {
            max_wall_millis: 60_000,
            max_game_ticks: 0,
            max_entities: 1000,
            max_bytes: 1024 * 1024 * 1024,
            max_output_tokens: 65_536,
            max_actions: 1,
        },
        grants: [
            Capability::Query,
            Capability::Observe,
            Capability::Plan,
            Capability::Designate,
        ]
        .into_iter()
        .map(|capability| CapabilityGrant {
            capability,
            scope: CapabilityScope {
                fortress_id: Some(p.before().fortress_id()),
                map_area: Some(scope()),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::Guarded,
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
        cancellation_requested: false,
    })
}
fn journal(memory: Memory, mode: DigMode, create: bool) -> Result<DigJournal<Memory>> {
    DigJournal::open(
        memory,
        &context()?,
        mode,
        Some(binding()?),
        create.then_some([9; 32]),
    )
}
fn native(p: &DigPlan, phase: DigPhase) -> Result<DigEffect> {
    let mut out = fixture(if phase == DigPhase::Designated {
        "designated"
    } else if phase == DigPhase::Refused {
        "cancelled"
    } else {
        "prepared"
    })?;
    out[133] = match phase {
        DigPhase::Prepared => 0,
        DigPhase::Unknown => 1,
        DigPhase::Designated => 2,
        DigPhase::Refused => 4,
    };
    // Session scenarios deliberately use the independently checked native plan.
    DigEffect::decode(&out, p)
}
struct Game {
    capture: DigObservation,
    effect: Option<DigEffect>,
    calls: Vec<(usize, DigStage)>,
    opened: usize,
    dropped: usize,
    lose_commit: bool,
    replayed: bool,
    unknown_commit: bool,
}
impl Game {
    fn new() -> Result<Rc<RefCell<Self>>> {
        Ok(Rc::new(RefCell::new(Self {
            capture: plan()?.before().clone(),
            effect: None,
            calls: Vec::new(),
            opened: 0,
            dropped: 0,
            lose_commit: false,
            replayed: false,
            unknown_commit: false,
        })))
    }
}
struct Source {
    id: usize,
    game: Rc<RefCell<Game>>,
    binding: DigBinding,
    permit: bool,
}
impl Drop for Source {
    fn drop(&mut self) {
        self.game.borrow_mut().dropped += 1;
    }
}
fn factory(
    game: Rc<RefCell<Game>>,
) -> impl FnOnce(&DigBinding, DigRegion, &OperationContext) -> Result<Source> {
    move |binding, region, context| {
        assert_eq!(context.budget.max_bytes, SOURCE_RESERVATION_BYTES);
        assert!(context.budget.max_wall_millis > 0 && context.budget.max_wall_millis <= 60_000);
        assert_eq!(region, game.borrow().capture.region());
        let id = {
            let mut g = game.borrow_mut();
            g.opened += 1;
            g.opened
        };
        Ok(Source {
            id,
            game,
            binding: binding.clone(),
            permit: false,
        })
    }
}
impl DigSource for Source {
    fn manifest(&self) -> &DigManifest {
        self.binding.manifest()
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        Some(self.binding.endpoint())
    }
    fn observe(&mut self, region: DigRegion, _: &OperationContext) -> Result<DigObservation> {
        let mut g = self.game.borrow_mut();
        g.calls.push((self.id, DigStage::Observe));
        assert_eq!(region, g.capture.region());
        Ok(g.capture.clone())
    }
    fn prepare(&mut self, p: &DigPlan, _: &OperationContext) -> Result<DigPreparation> {
        let mut g = self.game.borrow_mut();
        g.calls.push((self.id, DigStage::Prepare));
        let value = match &g.effect {
            Some(e) => e.clone(),
            None => native(p, DigPhase::Prepared)?,
        };
        g.effect = Some(value.clone());
        self.permit = !g.replayed;
        DigPreparation::decode(value.canonical_bytes(), g.replayed, p)
    }
    fn commit(&mut self, p: &DigPlan, _: &OperationContext) -> Result<DigEffect> {
        assert!(
            self.permit,
            "session lost the original native preparation connection"
        );
        self.permit = false;
        let mut g = self.game.borrow_mut();
        g.calls.push((self.id, DigStage::Commit));
        let value = native(
            p,
            if g.unknown_commit {
                DigPhase::Unknown
            } else {
                DigPhase::Designated
            },
        )?;
        g.effect = Some(value.clone());
        if g.lose_commit {
            Err(unknown(p.key()))
        } else {
            Ok(value)
        }
    }
    fn query(&mut self, _: &DigPlan, _: &OperationContext) -> Result<Option<DigEffect>> {
        let mut g = self.game.borrow_mut();
        g.calls.push((self.id, DigStage::Query));
        Ok(g.effect.clone())
    }
    fn cancel(&mut self, p: &DigPlan, _: &OperationContext) -> Result<DigEffect> {
        let mut g = self.game.borrow_mut();
        g.calls.push((self.id, DigStage::Cancel));
        let value = match &g.effect {
            Some(e) if e.phase().terminal() || e.phase() == DigPhase::Unknown => e.clone(),
            Some(_) => native(p, DigPhase::Refused)?,
            None => return Err(unknown(p.key())),
        };
        g.effect = Some(value.clone());
        self.permit = false;
        Ok(value)
    }
}
#[derive(Default)]
struct Guard {
    deny: Option<DigStage>,
    deny_connect: bool,
    observe_calls: usize,
    deny_observe_at: Option<usize>,
}
impl DigGuard for Guard {
    fn check(&mut self, stage: DigStage, _: &DigPlan, _: &OperationContext) -> Result<()> {
        if self.deny == Some(stage) {
            Err(error(ErrorCode::CapabilityDenied, "revoked test runtime"))
        } else {
            Ok(())
        }
    }
}
impl DigSessionGuard for Guard {
    fn connect(&mut self, _: &DigBinding, _: DigRegion, _: &OperationContext) -> Result<()> {
        if self.deny_connect {
            Err(error(
                ErrorCode::CapabilityDenied,
                "revoked test connection",
            ))
        } else {
            Ok(())
        }
    }
    fn observe(&mut self, _: &DigBinding, _: DigRegion, _: &OperationContext) -> Result<()> {
        self.observe_calls += 1;
        if self.deny_observe_at == Some(self.observe_calls) {
            Err(error(
                ErrorCode::CapabilityDenied,
                "revoked test observation",
            ))
        } else {
            Ok(())
        }
    }
}
fn session(memory: Memory, mode: DigMode, create: bool) -> Result<DigSession<Memory, Source>> {
    DigSession::new(journal(memory, mode, create)?, &context()?)
}
fn prepared(
    session: &mut DigSession<Memory, Source>,
    game: Rc<RefCell<Game>>,
    guard: &mut Guard,
) -> Result<DigRecord> {
    let p = plan()?;
    session.observe(p.before().region(), &context()?, factory(game), guard)?;
    session.prepare(p.key(), false, p.before().witness(), &context()?, guard)
}
fn never_connect(_: &DigBinding, _: DigRegion, _: &OperationContext) -> Result<Source> {
    Err(error(
        ErrorCode::InternalInvariantViolation,
        "unexpected connection factory",
    ))
}

#[test]
fn observation_prepare_and_commit_share_one_owned_connection() -> Result<()> {
    let mut s = session(Memory::default(), DigMode::Control, true)?;
    let game = Game::new()?;
    let mut guard = Guard::default();
    let p = prepared(&mut s, game.clone(), &mut guard)?;
    assert!(s.has_preparation_connection());
    assert!(s.selected().is_none());
    let result = s.commit(p.plan().key(), p.plan().digest(), &context()?, &mut guard)?;
    assert_eq!(result.state(), DigState::Terminal);
    assert!(!s.has_preparation_connection());
    let g = game.borrow();
    assert_eq!(g.opened, 1);
    assert_eq!(g.dropped, 1);
    assert_eq!(
        g.calls,
        [
            DigStage::Observe,
            DigStage::Observe,
            DigStage::Prepare,
            DigStage::Observe,
            DigStage::Commit
        ]
        .map(|stage| (1, stage))
    );
    Ok(())
}
#[test]
fn no_observation_or_wrong_witness_cannot_prepare_native_work() -> Result<()> {
    let mut s = session(Memory::default(), DigMode::Control, true)?;
    let p = plan()?;
    let mut guard = Guard::default();
    let game = Game::new()?;
    assert!(
        s.prepare(
            p.key(),
            false,
            p.before().witness(),
            &context()?,
            &mut guard
        )
        .is_err()
    );
    s.observe(
        p.before().region(),
        &context()?,
        factory(game.clone()),
        &mut guard,
    )?;
    assert!(
        s.prepare(p.key(), false, Digest32::ZERO, &context()?, &mut guard)
            .is_err()
    );
    assert_eq!(game.borrow().calls, [(1, DigStage::Observe)]);
    Ok(())
}
#[test]
fn repeated_preparation_returns_history_without_repreparing() -> Result<()> {
    let mut s = session(Memory::default(), DigMode::Control, true)?;
    let game = Game::new()?;
    let mut guard = Guard::default();
    let first = prepared(&mut s, game.clone(), &mut guard)?;
    let second = s.prepare(
        first.plan().key(),
        false,
        first.plan().before().witness(),
        &context()?,
        &mut guard,
    )?;
    assert_eq!(first, second);
    assert_eq!(game.borrow().calls.len(), 3);
    assert!(s.has_preparation_connection());
    assert!(
        s.prepare(
            first.plan().key(),
            true,
            first.plan().before().witness(),
            &context()?,
            &mut guard
        )
        .is_err()
    );
    Ok(())
}
#[test]
fn replayed_prepared_receipt_never_retains_a_dispatch_connection() -> Result<()> {
    let mut s = session(Memory::default(), DigMode::Control, true)?;
    let game = Game::new()?;
    game.borrow_mut().replayed = true;
    let mut guard = Guard::default();
    let p = prepared(&mut s, game.clone(), &mut guard)?;
    assert_eq!(p.state(), DigState::Tracking);
    assert!(!s.has_preparation_connection());
    assert!(
        s.commit(p.plan().key(), p.plan().digest(), &context()?, &mut guard)
            .is_err()
    );
    assert_eq!(game.borrow().dropped, 1);
    assert!(
        !game
            .borrow()
            .calls
            .iter()
            .any(|(_, stage)| *stage == DigStage::Commit)
    );
    Ok(())
}
#[test]
fn lost_commit_reply_reconnects_only_for_query_and_keeps_the_original_intent() -> Result<()> {
    let memory = Memory::default();
    let game = Game::new()?;
    let mut s = session(memory.clone(), DigMode::Control, true)?;
    let mut guard = Guard::default();
    let p = prepared(&mut s, game.clone(), &mut guard)?;
    game.borrow_mut().lose_commit = true;
    assert!(
        s.commit(p.plan().key(), p.plan().digest(), &context()?, &mut guard)
            .is_err()
    );
    assert!(!s.has_preparation_connection());
    assert_eq!(
        s.get(p.plan().key(), p.plan().digest(), &context()?)?
            .state(),
        DigState::DispatchStarted
    );
    drop(s);
    let mut recovery = session(memory, DigMode::Recover, false)?;
    let mut query = context()?;
    query.grants.retain(|g| g.capability == Capability::Query);
    let result = recovery.reconcile(
        p.plan().key(),
        p.plan().digest(),
        &query,
        factory(game.clone()),
        &mut guard,
    )?;
    assert_eq!(result.state(), DigState::Terminal);
    assert_eq!(result.plan(), p.plan());
    assert_eq!(game.borrow().opened, 2);
    assert_eq!(game.borrow().dropped, 2);
    assert_eq!(
        game.borrow()
            .calls
            .iter()
            .filter(|(_, s)| *s == DigStage::Commit)
            .count(),
        1
    );
    assert_eq!(game.borrow().calls.last(), Some(&(2, DigStage::Query)));
    Ok(())
}
#[test]
fn reopen_prepared_never_restores_a_permit_and_can_retire_by_cancellation() -> Result<()> {
    let memory = Memory::default();
    let game = Game::new()?;
    let mut guard = Guard::default();
    let mut s = session(memory.clone(), DigMode::Control, true)?;
    let p = prepared(&mut s, game.clone(), &mut guard)?;
    drop(s);
    let mut reopened = session(memory, DigMode::Control, false)?;
    assert!(!reopened.list(&context()?, 8, None)?.records[0].dispatchable);
    assert!(
        reopened
            .commit(p.plan().key(), p.plan().digest(), &context()?, &mut guard)
            .is_err()
    );
    let cancelled = reopened.cancel(
        p.plan().key(),
        p.plan().digest(),
        &context()?,
        factory(game.clone()),
        &mut guard,
    )?;
    assert_eq!(
        cancelled.effect().map(DigEffect::phase),
        Some(DigPhase::Refused)
    );
    assert!(
        !game
            .borrow()
            .calls
            .iter()
            .any(|(_, s)| *s == DigStage::Commit)
    );
    Ok(())
}
#[test]
fn known_terminal_and_absorbing_unknown_skip_all_native_recovery() -> Result<()> {
    for unknown_commit in [false, true] {
        let mut s = session(Memory::default(), DigMode::Control, true)?;
        let game = Game::new()?;
        game.borrow_mut().unknown_commit = unknown_commit;
        let mut guard = Guard::default();
        let p = prepared(&mut s, game.clone(), &mut guard)?;
        let result = s.commit(p.plan().key(), p.plan().digest(), &context()?, &mut guard)?;
        assert_eq!(
            s.reconcile(
                p.plan().key(),
                p.plan().digest(),
                &context()?,
                never_connect,
                &mut guard
            )?,
            result
        );
        assert_eq!(
            s.cancel(
                p.plan().key(),
                p.plan().digest(),
                &context()?,
                never_connect,
                &mut guard
            )?,
            result
        );
        assert_eq!(game.borrow().opened, 1);
    }
    Ok(())
}
#[test]
fn changing_observation_revokes_the_old_permit_without_forgetting_the_obligation() -> Result<()> {
    let mut s = session(Memory::default(), DigMode::Control, true)?;
    let game = Game::new()?;
    let mut guard = Guard::default();
    let p = prepared(&mut s, game.clone(), &mut guard)?;
    s.observe(
        p.plan().before().region(),
        &context()?,
        factory(game.clone()),
        &mut guard,
    )?;
    assert!(!s.has_preparation_connection());
    assert_eq!(s.list(&context()?, 8, None)?.unsettled_records, 1);
    assert!(
        s.commit(p.plan().key(), p.plan().digest(), &context()?, &mut guard)
            .is_err()
    );
    assert!(
        s.prepare(
            "different-key",
            false,
            p.plan().before().witness(),
            &context()?,
            &mut guard
        )
        .is_err()
    );
    assert!(
        !game
            .borrow()
            .calls
            .iter()
            .any(|(_, s)| *s == DigStage::Commit)
    );
    Ok(())
}
#[test]
fn wrong_confirmation_never_dispatches_and_correct_confirmation_still_works() -> Result<()> {
    let mut s = session(Memory::default(), DigMode::Control, true)?;
    let game = Game::new()?;
    let mut guard = Guard::default();
    let p = prepared(&mut s, game.clone(), &mut guard)?;
    assert!(
        s.commit(p.plan().key(), Digest32::ZERO, &context()?, &mut guard)
            .is_err()
    );
    assert!(s.has_preparation_connection());
    assert_eq!(game.borrow().calls.len(), 3);
    assert_eq!(
        s.commit(p.plan().key(), p.plan().digest(), &context()?, &mut guard)?
            .state(),
        DigState::Terminal
    );
    Ok(())
}
#[test]
fn runtime_revocation_before_commit_keeps_durable_uncertainty_and_drops_the_source() -> Result<()> {
    let mut s = session(Memory::default(), DigMode::Control, true)?;
    let game = Game::new()?;
    let mut guard = Guard::default();
    let p = prepared(&mut s, game.clone(), &mut guard)?;
    guard.deny = Some(DigStage::Commit);
    assert!(
        s.commit(p.plan().key(), p.plan().digest(), &context()?, &mut guard)
            .is_err()
    );
    assert!(!s.has_preparation_connection());
    assert_eq!(
        s.get(p.plan().key(), p.plan().digest(), &context()?)?
            .state(),
        DigState::DispatchStarted
    );
    assert!(
        !game
            .borrow()
            .calls
            .iter()
            .any(|(_, s)| *s == DigStage::Commit)
    );
    Ok(())
}
#[test]
fn insufficient_budget_scope_or_cancelled_context_prevents_connection() -> Result<()> {
    let mut s = session(Memory::default(), DigMode::Control, true)?;
    let p = plan()?;
    let mut guard = Guard::default();
    for change in 0..3 {
        let mut c = context()?;
        match change {
            0 => c.budget.max_bytes = 1,
            1 => c.cancellation_requested = true,
            _ => c.grants.retain(|g| g.capability == Capability::Query),
        }
        let e = s
            .observe(p.before().region(), &c, never_connect, &mut guard)
            .err()
            .ok_or_else(exhausted)?;
        assert_ne!(e.code, ErrorCode::InternalInvariantViolation);
    }
    let outside = DigRegion::new(64, 15, 2, 2, 2)?;
    assert_eq!(
        s.observe(outside, &context()?, never_connect, &mut guard)
            .err()
            .ok_or_else(exhausted)?
            .code,
        ErrorCode::CapabilityDenied
    );
    Ok(())
}
#[test]
fn missing_native_record_preserves_the_recovery_barrier() -> Result<()> {
    let mut s = session(Memory::default(), DigMode::Control, true)?;
    let game = Game::new()?;
    let mut guard = Guard::default();
    let p = prepared(&mut s, game.clone(), &mut guard)?;
    game.borrow_mut().effect = None;
    assert!(
        s.reconcile(
            p.plan().key(),
            p.plan().digest(),
            &context()?,
            factory(game.clone()),
            &mut guard
        )
        .is_err()
    );
    assert_eq!(s.list(&context()?, 8, None)?.unsettled_records, 1);
    assert!(!s.has_preparation_connection());
    assert!(
        s.commit(p.plan().key(), p.plan().digest(), &context()?, &mut guard)
            .is_err()
    );
    Ok(())
}
#[test]
fn offline_mode_exposes_history_but_cannot_connect_or_promote_to_control() -> Result<()> {
    let memory = Memory::default();
    let game = Game::new()?;
    let mut guard = Guard::default();
    let mut s = session(memory.clone(), DigMode::Control, true)?;
    let p = prepared(&mut s, game, &mut guard)?;
    drop(s);
    let mut offline = session(memory, DigMode::Offline, false)?;
    assert_eq!(
        offline
            .get(p.plan().key(), p.plan().digest(), &context()?)?
            .state(),
        DigState::Prepared
    );
    assert!(
        offline
            .observe(
                p.plan().before().region(),
                &context()?,
                never_connect,
                &mut guard
            )
            .is_err()
    );
    assert!(
        offline
            .reconcile(
                p.plan().key(),
                p.plan().digest(),
                &context()?,
                never_connect,
                &mut guard
            )
            .is_err()
    );
    assert!(
        offline
            .commit(p.plan().key(), p.plan().digest(), &context()?, &mut guard)
            .is_err()
    );
    Ok(())
}
#[test]
fn observed_clock_floor_prevents_expired_grants_from_being_revived() -> Result<()> {
    let mut s = session(Memory::default(), DigMode::Control, true)?;
    let game = Game::new()?;
    let mut raw = fixture("observation")?;
    raw[24..32].copy_from_slice(&(plan()?.before().tick() + 1).to_be_bytes());
    game.borrow_mut().capture = DigObservation::decode(&raw)?;
    let mut c = context()?;
    for grant in &mut c.grants {
        grant.expires_at_tick = Some(c.anchor.tick);
    }
    let mut guard = Guard::default();
    assert!(
        s.observe(
            plan()?.before().region(),
            &c,
            factory(game.clone()),
            &mut guard
        )
        .is_err()
    );
    assert!(s.selected().is_none());
    assert_eq!(game.borrow().dropped, 1);
    assert_eq!(s.high_tick(), c.anchor.tick.get() + 1);
    assert_eq!(
        s.observe(plan()?.before().region(), &c, never_connect, &mut guard)
            .err()
            .ok_or_else(exhausted)?
            .code,
        ErrorCode::CapabilityDenied
    );
    Ok(())
}
#[test]
fn connection_and_post_observation_guards_are_not_optional() -> Result<()> {
    let mut s = session(Memory::default(), DigMode::Control, true)?;
    let mut guard = Guard {
        deny_connect: true,
        ..Guard::default()
    };
    let e = s
        .observe(
            plan()?.before().region(),
            &context()?,
            never_connect,
            &mut guard,
        )
        .err()
        .ok_or_else(exhausted)?;
    assert_eq!(e.code, ErrorCode::CapabilityDenied);
    let game = Game::new()?;
    guard = Guard {
        deny_observe_at: Some(3),
        ..Guard::default()
    };
    assert!(
        s.observe(
            plan()?.before().region(),
            &context()?,
            factory(game.clone()),
            &mut guard
        )
        .is_err()
    );
    assert!(s.selected().is_none());
    assert_eq!(game.borrow().dropped, 1);
    Ok(())
}

// This scenario composes the actual RPC codec/permit implementation with the
// actual session and journal. The stream is fragmented, not a native game/SDK.
struct Wire {
    input: Cursor<Vec<u8>>,
    output: Rc<RefCell<Vec<u8>>>,
}
impl Read for Wire {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let n = out.len().min(7);
        self.input.read(&mut out[..n])
    }
}
impl Write for Wire {
    fn write(&mut self, raw: &[u8]) -> io::Result<usize> {
        let n = raw.len().min(7);
        self.output.borrow_mut().extend_from_slice(&raw[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl crate::dig_designation::rpc::DigStream for Wire {
    fn narrow_deadline(&mut self, _: std::time::Duration) -> Result<()> {
        Ok(())
    }
}
struct WireSource(crate::dig_designation::rpc::DigRpcClient<Wire>, SocketAddr);
impl DigSource for WireSource {
    fn manifest(&self) -> &DigManifest {
        self.0.manifest()
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        Some(self.1)
    }
    fn observe(&mut self, r: DigRegion, c: &OperationContext) -> Result<DigObservation> {
        self.0.observe(r, c)
    }
    fn prepare(&mut self, p: &DigPlan, c: &OperationContext) -> Result<DigPreparation> {
        self.0.prepare(p, c)
    }
    fn commit(&mut self, p: &DigPlan, c: &OperationContext) -> Result<DigEffect> {
        self.0.commit(p, c)
    }
    fn query(&mut self, p: &DigPlan, c: &OperationContext) -> Result<Option<DigEffect>> {
        self.0.query(p, c)
    }
    fn cancel(&mut self, p: &DigPlan, c: &OperationContext) -> Result<DigEffect> {
        self.0.cancel(p, c)
    }
}
fn varint(out: &mut Vec<u8>, mut n: u64) {
    while n >= 128 {
        out.push((n as u8 & 127) | 128);
        n >>= 7;
    }
    out.push(n as u8);
}
fn num(out: &mut Vec<u8>, tag: u64, n: u64) {
    varint(out, tag * 8);
    varint(out, n);
}
fn data(out: &mut Vec<u8>, tag: u64, raw: &[u8]) {
    varint(out, tag * 8 + 2);
    varint(out, raw.len() as u64);
    out.extend_from_slice(raw);
}
fn framed(raw: &[u8]) -> Vec<u8> {
    let mut out = (-1i16).to_le_bytes().to_vec();
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&(raw.len() as i32).to_le_bytes());
    out.extend_from_slice(raw);
    out
}
fn reply(capture: Option<&[u8]>, effect: Option<&[u8]>, replay: Option<bool>) -> Vec<u8> {
    let mut raw = Vec::new();
    num(&mut raw, 1, 1);
    num(&mut raw, 2, 0);
    data(&mut raw, 3, &[8; 32]);
    num(&mut raw, 4, 1);
    num(&mut raw, 5, 16);
    num(&mut raw, 6, 7);
    data(&mut raw, 7, b"test-df");
    data(&mut raw, 8, b"test-dfhack");
    if let Some(capture) = capture {
        data(&mut raw, 9, capture);
    }
    if let Some(effect) = effect {
        data(&mut raw, 10, effect);
    }
    if let Some(replay) = replay {
        num(&mut raw, 11, u64::from(replay));
    }
    framed(&raw)
}
#[test]
fn actual_fragmented_rpc_permit_survives_across_separate_session_operations() -> Result<()> {
    let mut input = b"DFHack!\n\x01\0\0\0".to_vec();
    for method in 2..8 {
        let mut raw = Vec::new();
        num(&mut raw, 1, method);
        input.extend(framed(&raw));
    }
    input.extend(reply(None, None, None));
    let capture = fixture("observation")?;
    input.extend(reply(Some(&capture), None, None)); // session observation
    input.extend(reply(Some(&capture), None, None)); // journal prepare revalidation
    input.extend(reply(None, Some(&fixture("prepared")?), Some(false)));
    input.extend(reply(Some(&capture), None, None)); // journal commit revalidation
    input.extend(reply(None, Some(&fixture("designated")?), None));
    let output = Rc::new(RefCell::new(Vec::new()));
    let wire = Wire {
        input: Cursor::new(input),
        output: output.clone(),
    };
    let j = journal(Memory::default(), DigMode::Control, true)?;
    let mut s: DigSession<_, WireSource> = DigSession::new(j, &context()?)?;
    let mut guard = Guard::default();
    let p = plan()?;
    let factory = |b: &DigBinding, r, c: &OperationContext| {
        let client = crate::dig_designation::rpc::DigRpcClient::negotiate(
            wire,
            vec![7; 32],
            vec![8; 32],
            r,
            c,
        )?;
        Ok(WireSource(client, b.endpoint()))
    };
    s.observe(p.before().region(), &context()?, factory, &mut guard)?;
    s.prepare(
        p.key(),
        false,
        p.before().witness(),
        &context()?,
        &mut guard,
    )?;
    assert_eq!(
        s.commit(p.key(), p.digest(), &context()?, &mut guard)?
            .state(),
        DigState::Terminal
    );
    let raw = output.borrow();
    assert_eq!(&raw[..12], b"DFHack?\n\x01\0\0\0");
    let mut at = 12;
    let mut methods = Vec::new();
    while at < raw.len() {
        assert!(raw.len() - at >= 8);
        methods.push(i16::from_le_bytes([raw[at], raw[at + 1]]));
        let width =
            u32::from_le_bytes([raw[at + 4], raw[at + 5], raw[at + 6], raw[at + 7]]) as usize;
        at += 8 + width;
        assert!(at <= raw.len());
    }
    assert_eq!(methods, [0, 0, 0, 0, 0, 0, 2, 3, 3, 4, 3, 5]);
    Ok(())
}

#[test]
fn session_view_keeps_pending_identity_after_abandoning_the_connection() -> Result<()> {
    let mut s = session(Memory::default(), DigMode::Control, true)?;
    let game = Game::new()?;
    let mut guard = Guard::default();
    let p = prepared(&mut s, game, &mut guard)?;
    let before = s.view(&context()?)?;
    assert_eq!(before.total_records, 1);
    assert!(before.pending.as_ref().is_some_and(|r| r.dispatchable));
    s.abandon_preparation();
    let after = s.view(&context()?)?;
    assert_eq!(before.head, after.head);
    assert!(
        after
            .pending
            .as_ref()
            .is_some_and(|r| r.key == p.plan().key() && !r.dispatchable)
    );
    Ok(())
}
#[test]
fn session_view_rechecks_current_authority_and_persistent_custody() -> Result<()> {
    let memory = Memory::default();
    let mut s = session(memory.clone(), DigMode::Control, true)?;
    let mut denied = context()?;
    denied.grants.clear();
    assert!(s.view(&denied).is_err());
    memory.0.borrow_mut().get_mut()[0] ^= 1;
    assert!(s.view(&context()?).is_err());
    Ok(())
}
