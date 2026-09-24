//! Actual dispatcher tests with injected native/storage edges; no live-game claim.
use super::*;
use dfmcp_adapter::workforce_control::journal::{AssignmentState, WorkforceJournal};
use dfmcp_adapter::workforce_control::rpc::WorkforceManifest;
use dfmcp_adapter::workforce_control::{AssignmentEffect, AssignmentPhase, AssignmentPlan};
use std::cell::RefCell;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::rc::Rc;

fn unhex(raw: &str) -> Result<Vec<u8>> {
    raw.trim()
        .as_bytes()
        .chunks_exact(2)
        .map(|b| {
            let s = std::str::from_utf8(b).map_err(|_| exhausted())?;
            u8::from_str_radix(s, 16).map_err(|_| exhausted())
        })
        .collect()
}
fn capture() -> Result<WorkforceCapture> {
    WorkforceCapture::decode(&unhex(include_str!(
        "../../../../tests/native/workforce/vectors/capture.hex"
    ))?)
}
#[derive(Default)]
struct FileState {
    bytes: Cursor<Vec<u8>>,
    syncs: usize,
    fail_sync: Option<usize>,
}
#[derive(Clone, Default)]
struct Memory(Rc<RefCell<FileState>>);
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.0.borrow_mut().bytes.read(out)
    }
}
impl Seek for Memory {
    fn seek(&mut self, p: SeekFrom) -> io::Result<u64> {
        self.0.borrow_mut().bytes.seek(p)
    }
}
impl Write for Memory {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().bytes.write(b)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        let mut file = self.0.borrow_mut();
        file.syncs += 1;
        if file.fail_sync == Some(file.syncs) {
            return Err(io::Error::other("injected uncertain sync"));
        }
        Ok(())
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("no repair"))
    }
}
#[derive(Default)]
struct Calls {
    connects: usize,
    observes: usize,
    prepares: usize,
    commits: usize,
    queries: usize,
    cancels: usize,
    lose_prepare: bool,
    lose_commit: bool,
    fail_observe: bool,
    missing: bool,
    unknown: bool,
    native: Option<AssignmentEffect>,
}
fn binding() -> Result<WorkforceBinding> {
    WorkforceBinding::new(
        SocketAddr::from(([127, 0, 0, 1], 5000)),
        WorkforceManifest {
            generation: 42,
            df_version: "fake-df".into(),
            dfhack_version: "fake-dfhack".into(),
        },
        "region1".into(),
        7,
    )
}
fn nonapplied(p: &AssignmentPlan, phase: AssignmentPhase) -> Result<AssignmentEffect> {
    let mut out = b"DFMWE017".to_vec();
    out.extend_from_slice(&(p.key().len() as u16).to_be_bytes());
    out.extend_from_slice(p.key().as_bytes());
    out.extend_from_slice(p.digest().as_bytes());
    out.extend_from_slice(p.token());
    out.extend_from_slice(&p.spec().detail().to_be_bytes());
    out.push(u8::from(p.spec().assigned()));
    out.extend_from_slice(p.before().witness().as_bytes());
    for n in [
        p.before().generation(),
        p.before().sequence(),
        p.before().tick(),
    ] {
        out.extend_from_slice(&n.to_be_bytes());
    }
    out.push(phase as u8);
    out.extend_from_slice(&[0; 32]);
    out.extend_from_slice(&(p.before().labor_keys().len() as u16).to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    let mut hashed = b"dfmcp-workforce-receipt/1\0".to_vec();
    hashed.extend_from_slice(&out);
    out.extend_from_slice(Digest32::of_bytes(&hashed).as_bytes());
    AssignmentEffect::decode(&out, p)
}
struct Source {
    binding: WorkforceBinding,
    calls: Rc<RefCell<Calls>>,
    memory: Memory,
}
impl Source {
    fn marker(&self, p: &AssignmentPlan) -> Result<AssignmentState> {
        let c = context(1, WorkforceMode::Control)?;
        let mut j = WorkforceJournal::open(self.memory.clone(), &c, WorkforceMode::Offline, None)?;
        Ok(j.get(p.key(), p.digest(), &c)?.state())
    }
}
impl WorkforceSource for Source {
    fn manifest(&self) -> &WorkforceManifest {
        self.binding.manifest()
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        Some(self.binding.endpoint())
    }
    fn observe(&mut self, ids: &[u32], _: &OperationContext) -> Result<WorkforceCapture> {
        let mut calls = self.calls.borrow_mut();
        calls.observes += 1;
        if calls.fail_observe {
            return Err(error(ErrorCode::AdapterRejected, "lost observation"));
        }
        let c = capture()?;
        assert_eq!(ids, c.ids().as_slice());
        Ok(c)
    }
    fn prepare(&mut self, p: &AssignmentPlan, _: &OperationContext) -> Result<AssignmentEffect> {
        assert_eq!(self.marker(p)?, AssignmentState::Intent);
        let mut calls = self.calls.borrow_mut();
        calls.prepares += 1;
        let effect = nonapplied(p, AssignmentPhase::Prepared)?;
        calls.native = Some(effect.clone());
        if calls.lose_prepare {
            return Err(error(ErrorCode::AdapterRejected, "lost prepare"));
        }
        Ok(effect)
    }
    fn commit(&mut self, p: &AssignmentPlan, _: &OperationContext) -> Result<AssignmentEffect> {
        assert_eq!(self.marker(p)?, AssignmentState::DispatchStarted);
        let mut calls = self.calls.borrow_mut();
        calls.commits += 1;
        let effect = if calls.unknown {
            nonapplied(p, AssignmentPhase::Unknown)?
        } else {
            AssignmentEffect::decode(
                &unhex(include_str!(
                    "../../../../tests/native/workforce/vectors/applied.hex"
                ))?,
                p,
            )?
        };
        calls.native = Some(effect.clone());
        if calls.lose_commit {
            return Err(error(ErrorCode::AdapterRejected, "lost commit"));
        }
        Ok(effect)
    }
    fn query(
        &mut self,
        _: &AssignmentPlan,
        _: &OperationContext,
    ) -> Result<Option<AssignmentEffect>> {
        let mut calls = self.calls.borrow_mut();
        calls.queries += 1;
        Ok(if calls.missing {
            None
        } else {
            calls.native.clone()
        })
    }
    fn cancel(&mut self, p: &AssignmentPlan, _: &OperationContext) -> Result<AssignmentEffect> {
        assert_eq!(self.marker(p)?, AssignmentState::CancelRequested);
        let mut calls = self.calls.borrow_mut();
        calls.cancels += 1;
        let effect = nonapplied(p, AssignmentPhase::Cancelled)?;
        calls.native = Some(effect.clone());
        Ok(effect)
    }
}
fn context(id: u128, mode: WorkforceMode) -> Result<OperationContext> {
    let cap = capture()?;
    Ok(OperationContext {
        session_id: SessionId::new(id),
        request_id: RequestId::new(1),
        anchor: StateAnchor {
            fortress_id: cap.fortress_id(),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(cap.tick()),
            state_hash: cap.witness(),
        },
        budget: WorkBudget {
            max_wall_millis: 60_000,
            max_bytes: MAX_WORK_BYTES,
            max_output_tokens: 65_536,
            max_entities: 4192,
            max_actions: 1,
            max_game_ticks: 0,
        },
        grants: grants(mode, cap.fortress_id(), true)?,
        cancellation_requested: false,
    })
}
fn setup(memory: Memory, mode: WorkforceMode, id: u128, fresh: bool) -> Result<State<Memory>> {
    let c = context(id, mode)?;
    let journal = WorkforceJournal::open(
        memory,
        &c,
        mode,
        if fresh {
            Some((binding()?, [1; 32]))
        } else {
            None
        },
    )?;
    let (control, _) = WorkforceSession::new(journal, &c)?;
    Ok(State {
        id: c.session_id,
        request: 1,
        budget: c.budget,
        grants: c.grants,
        binding: binding()?,
        control,
        selected: None,
        cursors: Cursors::default(),
    })
}
fn execute(
    state: &mut State<Memory>,
    memory: &Memory,
    calls: &Rc<RefCell<Calls>>,
    op: &str,
    action: Action,
    output: u64,
) -> Result<Value> {
    let c = state.context(true, false)?;
    let calls = calls.clone();
    let memory = memory.clone();
    let raw = run_action(
        state,
        c,
        Dispatch {
            operation: op,
            limits: Limits::default(),
            output,
            action: Ok(action),
        },
        |_| {
            calls.borrow_mut().connects += 1;
            Ok(Source {
                binding: binding()?,
                calls,
                memory,
            })
        },
    );
    serde_json::from_str(&raw).map_err(|_| exhausted())
}
fn prepare(
    state: &mut State<Memory>,
    memory: &Memory,
    calls: &Rc<RefCell<Calls>>,
) -> Result<Digest32> {
    let cap = capture()?;
    let observed = execute(
        state,
        memory,
        calls,
        "fortress.observe",
        Action::Observe(Selection {
            unit_ids: cap.ids(),
        }),
        COMPACT_OUTPUT,
    )?;
    assert_eq!(observed["result"]["ok"], true);
    let plan = AssignmentPlan::new("assign", AssignmentSpec::new(0, true)?, cap.clone())?;
    let out = execute(
        state,
        memory,
        calls,
        "fortress.plan",
        Action::Plan {
            key: "assign".into(),
            spec: plan.spec(),
            witness: cap.witness(),
        },
        DETAIL_OUTPUT,
    )?;
    if !calls.borrow().lose_prepare {
        assert_eq!(out["result"]["ok"], true);
    }
    Ok(plan.digest())
}
#[test]
fn full_dispatcher_lifecycle_and_offline_reopen() -> Result<()> {
    let memory = Memory::default();
    let calls = Rc::new(RefCell::new(Calls::default()));
    let mut s = setup(memory.clone(), WorkforceMode::Control, 1, true)?;
    let plan = prepare(&mut s, &memory, &calls)?;
    let done = execute(
        &mut s,
        &memory,
        &calls,
        "fortress.commit",
        Action::Commit {
            key: "assign".into(),
            plan,
            confirm: true,
        },
        DETAIL_OUTPUT,
    )?;
    assert_eq!(done["result"]["effect"]["native"]["phase"], "applied");
    assert_eq!(calls.borrow().commits, 1);
    let connections = calls.borrow().connects;
    execute(
        &mut s,
        &memory,
        &calls,
        "fortress.commit",
        Action::Commit {
            key: "assign".into(),
            plan,
            confirm: true,
        },
        DETAIL_OUTPUT,
    )?;
    let mut offline = setup(memory.clone(), WorkforceMode::Offline, 2, false)?;
    let historical = execute(
        &mut offline,
        &memory,
        &calls,
        "fortress.wait",
        Action::Wait {
            key: "assign".into(),
            plan,
        },
        DETAIL_OUTPUT,
    )?;
    assert_eq!(historical["result"]["effect"], done["result"]["effect"]);
    assert_eq!(calls.borrow().connects, connections);
    Ok(())
}
#[test]
fn review_confirmation_and_conflicting_key_refuse_before_connect() -> Result<()> {
    let m = Memory::default();
    let calls = Rc::new(RefCell::new(Calls::default()));
    let mut s = setup(m.clone(), WorkforceMode::Control, 1, true)?;
    let plan = prepare(&mut s, &m, &calls)?;
    let before = calls.borrow().connects;
    for confirm in [false, true] {
        let digest = if confirm { Digest32::ZERO } else { plan };
        let out = execute(
            &mut s,
            &m,
            &calls,
            "fortress.commit",
            Action::Commit {
                key: "assign".into(),
                plan: digest,
                confirm,
            },
            DETAIL_OUTPUT,
        )?;
        assert_eq!(out["result"]["ok"], false);
    }
    let out = execute(
        &mut s,
        &m,
        &calls,
        "fortress.plan",
        Action::Plan {
            key: "assign".into(),
            spec: AssignmentSpec::new(0, false)?,
            witness: capture()?.witness(),
        },
        DETAIL_OUTPUT,
    )?;
    assert_eq!(out["result"]["ok"], false);
    assert_eq!(calls.borrow().connects, before);
    Ok(())
}
#[test]
fn lost_commit_and_absence_never_restore_dispatch() -> Result<()> {
    let m = Memory::default();
    let calls = Rc::new(RefCell::new(Calls::default()));
    let mut s = setup(m.clone(), WorkforceMode::Control, 1, true)?;
    let plan = prepare(&mut s, &m, &calls)?;
    calls.borrow_mut().lose_commit = true;
    let out = execute(
        &mut s,
        &m,
        &calls,
        "fortress.commit",
        Action::Commit {
            key: "assign".into(),
            plan,
            confirm: true,
        },
        DETAIL_OUTPUT,
    )?;
    assert_eq!(out["result"]["ok"], false);
    let before = calls.borrow().connects;
    execute(
        &mut s,
        &m,
        &calls,
        "fortress.commit",
        Action::Commit {
            key: "assign".into(),
            plan,
            confirm: true,
        },
        DETAIL_OUTPUT,
    )?;
    assert_eq!(calls.borrow().connects, before);
    let mut recover = setup(m.clone(), WorkforceMode::Recover, 2, false)?;
    calls.borrow_mut().missing = true;
    let absent = execute(
        &mut recover,
        &m,
        &calls,
        "fortress.wait",
        Action::Wait {
            key: "assign".into(),
            plan,
        },
        DETAIL_OUTPUT,
    )?;
    assert_eq!(absent["result"]["ok"], false);
    calls.borrow_mut().missing = false;
    let done = execute(
        &mut recover,
        &m,
        &calls,
        "fortress.wait",
        Action::Wait {
            key: "assign".into(),
            plan,
        },
        DETAIL_OUTPUT,
    )?;
    assert_eq!(done["result"]["effect"]["native"]["phase"], "applied");
    assert_eq!(calls.borrow().commits, 1);
    assert_eq!(calls.borrow().queries, 2);
    Ok(())
}
#[test]
fn lost_prepare_recovers_without_preparation_renewal() -> Result<()> {
    let m = Memory::default();
    let calls = Rc::new(RefCell::new(Calls {
        lose_prepare: true,
        ..Calls::default()
    }));
    let mut s = setup(m.clone(), WorkforceMode::Control, 1, true)?;
    let plan = prepare(&mut s, &m, &calls)?;
    let out = execute(
        &mut s,
        &m,
        &calls,
        "fortress.wait",
        Action::Wait {
            key: "assign".into(),
            plan,
        },
        DETAIL_OUTPUT,
    )?;
    assert_eq!(out["result"]["effect"]["record"]["state"], "prepared");
    assert_eq!(calls.borrow().prepares, 1);
    assert_eq!(calls.borrow().queries, 1);
    Ok(())
}
#[test]
fn permanent_unknown_remains_visible_without_native_retry() -> Result<()> {
    let m = Memory::default();
    let calls = Rc::new(RefCell::new(Calls {
        unknown: true,
        ..Calls::default()
    }));
    let mut s = setup(m.clone(), WorkforceMode::Control, 1, true)?;
    let plan = prepare(&mut s, &m, &calls)?;
    execute(
        &mut s,
        &m,
        &calls,
        "fortress.commit",
        Action::Commit {
            key: "assign".into(),
            plan,
            confirm: true,
        },
        DETAIL_OUTPUT,
    )?;
    let before = calls.borrow().connects;
    for action in [
        Action::Wait {
            key: "assign".into(),
            plan,
        },
        Action::Cancel {
            key: "assign".into(),
            plan,
        },
    ] {
        let out = execute(&mut s, &m, &calls, "fortress.wait", action, DETAIL_OUTPUT)?;
        assert_eq!(out["result"]["effect"]["native"]["phase"], "unknown");
        assert_eq!(
            out["agent_turn"]["active_work"]["counts"]["permanent_unknown"],
            1
        );
    }
    assert_eq!(calls.borrow().connects, before);
    assert_eq!(calls.borrow().commits, 1);
    Ok(())
}
#[test]
fn cancellation_before_dispatch_is_local_and_fixed_modes_do_not_promote() -> Result<()> {
    let m = Memory::default();
    let calls = Rc::new(RefCell::new(Calls::default()));
    let mut s = setup(m.clone(), WorkforceMode::Control, 1, true)?;
    let plan = prepare(&mut s, &m, &calls)?;
    let before = calls.borrow().connects;
    for mode in [WorkforceMode::Offline, WorkforceMode::Recover] {
        let mut recovered = setup(m.clone(), mode, 2, false)?;
        recovered.grants = grants(WorkforceMode::Control, recovered.binding.fortress(), true)?;
        let out = execute(
            &mut recovered,
            &m,
            &calls,
            "fortress.cancel",
            Action::Cancel {
                key: "assign".into(),
                plan,
            },
            DETAIL_OUTPUT,
        )?;
        assert_eq!(out["result"]["ok"], false);
    }
    let out = execute(
        &mut s,
        &m,
        &calls,
        "fortress.cancel",
        Action::Cancel {
            key: "assign".into(),
            plan,
        },
        DETAIL_OUTPUT,
    )?;
    assert_eq!(
        out["result"]["effect"]["record"]["state"],
        "cancelled_before_dispatch"
    );
    assert_eq!(calls.borrow().connects, before);
    Ok(())
}
#[test]
fn failed_refresh_clears_selection_but_preserves_local_history() -> Result<()> {
    let m = Memory::default();
    let calls = Rc::new(RefCell::new(Calls::default()));
    let mut s = setup(m.clone(), WorkforceMode::Control, 1, true)?;
    execute(
        &mut s,
        &m,
        &calls,
        "fortress.observe",
        Action::Observe(Selection {
            unit_ids: capture()?.ids(),
        }),
        COMPACT_OUTPUT,
    )?;
    assert!(s.selected.is_some());
    calls.borrow_mut().fail_observe = true;
    let out = execute(
        &mut s,
        &m,
        &calls,
        "fortress.observe",
        Action::Observe(Selection {
            unit_ids: capture()?.ids(),
        }),
        COMPACT_OUTPUT,
    )?;
    assert_eq!(out["result"]["ok"], false);
    assert!(s.selected.is_none());
    let before = calls.borrow().connects;
    let out = execute(
        &mut s,
        &m,
        &calls,
        "fortress.doctor",
        Action::Doctor,
        COMPACT_OUTPUT,
    )?;
    assert_eq!(out["result"]["ok"], true);
    assert_eq!(calls.borrow().connects, before);
    Ok(())
}
#[test]
fn output_reservation_refuses_before_native_work_and_invalidates_refresh() -> Result<()> {
    let m = Memory::default();
    let mut s = setup(m, WorkforceMode::Control, 1, true)?;
    s.selected = Some(capture()?);
    let c = s.context(true, false)?;
    let connected = std::cell::Cell::new(false);
    let result = run_action::<_, Source, _>(
        &mut s,
        c,
        Dispatch {
            operation: "fortress.observe",
            limits: Limits {
                tokens: Some(1),
                ..Limits::default()
            },
            output: COMPACT_OUTPUT,
            action: Ok(Action::Observe(Selection {
                unit_ids: capture()?.ids(),
            })),
        },
        |_| {
            connected.set(true);
            Err(error(
                ErrorCode::InternalInvariantViolation,
                "budget-refused operation connected",
            ))
        },
    );
    let out: Value = serde_json::from_str(&result).map_err(|_| exhausted())?;
    assert_eq!(out["result"]["ok"], false);
    assert!(s.selected.is_none());
    assert!(!connected.get());
    Ok(())
}
#[test]
fn per_effect_revocation_after_durable_dispatch_blocks_native_assignment() -> Result<()> {
    let m = Memory::default();
    let calls = Rc::new(RefCell::new(Calls::default()));
    let mut s = setup(m.clone(), WorkforceMode::Control, 1, true)?;
    let plan = prepare(&mut s, &m, &calls)?;
    let c = s.context(true, false)?;
    let encoded = run_action(
        &mut s,
        c,
        Dispatch {
            operation: "fortress.commit",
            limits: Limits::default(),
            output: DETAIL_OUTPUT,
            action: Ok(Action::Commit {
                key: "assign".into(),
                plan,
                confirm: true,
            }),
        },
        |_| {
            Ok(CheckedSource {
                source: Source {
                    binding: binding()?,
                    calls: calls.clone(),
                    memory: m.clone(),
                },
                check: |write| {
                    if write {
                        Err(error(ErrorCode::CapabilityDenied, "revoked"))
                    } else {
                        Ok(())
                    }
                },
            })
        },
    );
    let out: Value = serde_json::from_str(&encoded).map_err(|_| exhausted())?;
    assert_eq!(out["result"]["ok"], false);
    assert_eq!(calls.borrow().commits, 0);
    let c = s.context(true, false)?;
    assert_eq!(
        s.control.view(&c)?.records[0].state(),
        AssignmentState::DispatchStarted
    );
    Ok(())
}
#[test]
fn uncertain_dispatch_sync_is_not_a_retryable_assignment() -> Result<()> {
    let m = Memory::default();
    let calls = Rc::new(RefCell::new(Calls::default()));
    let mut s = setup(m.clone(), WorkforceMode::Control, 1, true)?;
    let plan = prepare(&mut s, &m, &calls)?;
    let next = m.0.borrow().syncs + 1;
    m.0.borrow_mut().fail_sync = Some(next);
    let out = execute(
        &mut s,
        &m,
        &calls,
        "fortress.commit",
        Action::Commit {
            key: "assign".into(),
            plan,
            confirm: true,
        },
        DETAIL_OUTPUT,
    )?;
    assert_eq!(out["result"]["ok"], false);
    assert_eq!(calls.borrow().commits, 0);
    m.0.borrow_mut().fail_sync = None;
    let mut recovered = setup(m.clone(), WorkforceMode::Control, 2, false)?;
    execute(
        &mut recovered,
        &m,
        &calls,
        "fortress.wait",
        Action::Wait {
            key: "assign".into(),
            plan,
        },
        DETAIL_OUTPUT,
    )?;
    let out = execute(
        &mut recovered,
        &m,
        &calls,
        "fortress.commit",
        Action::Commit {
            key: "assign".into(),
            plan,
            confirm: true,
        },
        DETAIL_OUTPUT,
    )?;
    assert_eq!(out["result"]["ok"], false);
    assert_eq!(calls.borrow().commits, 0);
    Ok(())
}
#[test]
fn detail_pages_are_exact_selection_pinned_without_another_capture() -> Result<()> {
    let m = Memory::default();
    let calls = Rc::new(RefCell::new(Calls::default()));
    let mut s = setup(m.clone(), WorkforceMode::Control, 1, true)?;
    execute(
        &mut s,
        &m,
        &calls,
        "fortress.observe",
        Action::Observe(Selection {
            unit_ids: capture()?.ids(),
        }),
        COMPACT_OUTPUT,
    )?;
    let before = calls.borrow().connects;
    let out = execute(
        &mut s,
        &m,
        &calls,
        "fortress.query",
        Action::Query(Query::Details {
            witness: capture()?.witness().to_string(),
            offset: 0,
            limit: Some(1),
        }),
        DETAIL_OUTPUT,
    )?;
    assert_eq!(out["result"]["ok"], true);
    assert_eq!(
        out["result"]["selection"]["details"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    let bad = execute(
        &mut s,
        &m,
        &calls,
        "fortress.query",
        Action::Query(Query::Details {
            witness: Digest32::ZERO.to_string(),
            offset: 0,
            limit: Some(1),
        }),
        DETAIL_OUTPUT,
    )?;
    assert_eq!(bad["result"]["ok"], false);
    assert_eq!(calls.borrow().connects, before);
    Ok(())
}
#[test]
fn continuation_context_and_cache_eviction_are_enforced() -> Result<()> {
    let m = Memory::default();
    let mut s = setup(m, WorkforceMode::Control, 1, true)?;
    let c = s.context(true, false)?;
    let view = s.control.view(&c)?;
    let token = s.cursors.issue(s.id, &view, Filter::All, 4, 4)?;
    assert_eq!(s.cursors.resolve(&token, s.id, &view, Filter::All, 4)?, 4);
    assert!(
        s.cursors
            .resolve(&token, SessionId::new(99), &view, Filter::All, 4)
            .is_err()
    );
    assert!(
        s.cursors
            .resolve(&token, s.id, &view, Filter::Pending, 4)
            .is_err()
    );
    assert!(
        s.cursors
            .resolve(&token, s.id, &view, Filter::All, 8)
            .is_err()
    );
    let mut drift = view.clone();
    drift.head = Digest32::ZERO;
    assert!(
        s.cursors
            .resolve(&token, s.id, &drift, Filter::All, 4)
            .is_err()
    );
    for i in 0..64 {
        s.cursors.issue(s.id, &view, Filter::All, 4, i)?;
    }
    assert!(
        s.cursors
            .resolve(&token, s.id, &view, Filter::All, 4)
            .is_err()
    );
    Ok(())
}
#[test]
fn strict_selection_query_and_digest_grammars() -> Result<()> {
    Selection::parse(r#"{"unit_ids":[2,5]}"#)?;
    Query::parse(r#"{"mode":"records","limit":8}"#)?;
    for raw in [
        r#"{"unit_ids":[]}"#,
        r#"{"unit_ids":[2,2]}"#,
        r#"{"unit_ids":[5,2]}"#,
        r#"{"unit_ids":[true]}"#,
        r#"{"unit_ids":[2],"unit_ids":[5]}"#,
        r#"{"unit_ids":[2],"shell":"x"}"#,
    ] {
        assert!(Selection::parse(raw).is_err());
    }
    for raw in [
        r#"{"mode":"shell"}"#,
        r#"{"mode":"records","limit":0}"#,
        r#"{"mode":"records","limit":9}"#,
        r#"{"mode":"records","state":"unknown"}"#,
        r#"{"mode":"records","state":"all","state":"pending"}"#,
        r#"{"mode":"details","witness":"0","offset":0}"#,
    ] {
        assert!(Query::parse(raw).is_err());
    }
    assert!(digest(&"A".repeat(64)).is_err());
    assert!(Selection::parse(&" ".repeat(2049)).is_err());
    Ok(())
}
#[test]
fn current_query_authority_and_cancellation_cannot_be_replayed() -> Result<()> {
    let m = Memory::default();
    let mut s = setup(m, WorkforceMode::Control, 1, true)?;
    for which in 0..3 {
        let mut c = s.context(true, false)?;
        if which == 0 {
            c.grants.clear();
        } else if which == 1 {
            c.cancellation_requested = true;
        } else {
            for g in &mut c.grants {
                g.remaining_uses = Some(1);
            }
        }
        let out = run_action::<_, Source, _>(
            &mut s,
            c,
            Dispatch {
                operation: "fortress.doctor",
                limits: Limits::default(),
                output: COMPACT_OUTPUT,
                action: Ok(Action::Doctor),
            },
            |_| Err(exhausted()),
        );
        let parsed: Value = serde_json::from_str(&out).map_err(|_| exhausted())?;
        assert_eq!(parsed["result"]["ok"], false);
    }
    Ok(())
}
#[test]
fn close_never_confuses_recovery_release_with_quiescence() -> Result<()> {
    let m = Memory::default();
    let calls = Rc::new(RefCell::new(Calls::default()));
    let mut s = setup(m.clone(), WorkforceMode::Control, 1, true)?;
    let c = s.context(true, false)?;
    assert!(close_allowed(Some(&s.control.view(&c)?), false));
    prepare(&mut s, &m, &calls)?;
    let c = s.context(true, false)?;
    let view = s.control.view(&c)?;
    assert!(!close_allowed(Some(&view), false));
    assert!(close_allowed(Some(&view), true));
    assert!(close_allowed(None, true));
    assert!(!close_allowed(None, false));
    Ok(())
}
#[test]
fn environment_and_packet_do_not_inherit_admission_or_invent_absence() -> Result<()> {
    environment_contract(Some("1"), None, &[], false)?;
    for (opt, labor, admitted) in [
        (None, None, false),
        (Some("0"), None, false),
        (Some("1"), Some("0"), false),
        (Some("1"), None, true),
    ] {
        assert!(environment_contract(opt, labor, &[], admitted).is_err());
    }
    assert!(
        environment_contract(
            Some("1"),
            None,
            &["DFMCP_ADMITTED_BRIDGE_PROTOCOL".into()],
            false
        )
        .is_err()
    );
    assert!(runtime_io().is_err());
    let value: Value =
        serde_json::from_str(&unbound("fortress.query", &exhausted())).map_err(|_| exhausted())?;
    assert_eq!(
        value["agent_turn"]["active_work"]["pending_absence_proven"],
        false
    );
    assert!(value["agent_turn"]["briefing"]["admission"].is_null());
    assert!(value["agent_turn"]["anchor"].is_null());
    Ok(())
}
#[test]
fn adapter_owned_clock_floor_rejects_regressed_capture() -> Result<()> {
    let memory = Memory::default();
    let calls = Rc::new(RefCell::new(Calls::default()));
    let mut c = context(1, WorkforceMode::Control)?;
    c.anchor.tick = GameTick(c.anchor.tick.get() + 1);
    let journal = WorkforceJournal::open(
        memory.clone(),
        &c,
        WorkforceMode::Control,
        Some((binding()?, [9; 32])),
    )?;
    let (control, _) = WorkforceSession::new(journal, &c)?;
    let mut state = State {
        id: c.session_id,
        request: 1,
        budget: c.budget,
        grants: c.grants,
        binding: binding()?,
        control,
        selected: None,
        cursors: Cursors::default(),
    };
    let result = execute(
        &mut state,
        &memory,
        &calls,
        "fortress.observe",
        Action::Observe(Selection {
            unit_ids: capture()?.ids(),
        }),
        COMPACT_OUTPUT,
    )?;
    assert_eq!(result["result"]["ok"], false);
    assert!(state.selected.is_none());
    assert_eq!(calls.borrow().commits, 0);
    Ok(())
}
#[test]
fn generated_definitions_use_exactly_eleven_dotted_tools() {
    use fastmcp_rust::__private::server::ToolHandler;
    let defs = [
        FortressOpenSession.definition(),
        FortressObserve.definition(),
        FortressQuery.definition(),
        FortressPlan.definition(),
        FortressCommit.definition(),
        FortressWait.definition(),
        FortressCancel.definition(),
        FortressCheckpoint.definition(),
        FortressRestore.definition(),
        FortressExplain.definition(),
        FortressDoctor.definition(),
    ];
    assert_eq!(
        defs.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
        vec![
            "fortress.open_session",
            "fortress.observe",
            "fortress.query",
            "fortress.plan",
            "fortress.commit",
            "fortress.wait",
            "fortress.cancel",
            "fortress.checkpoint",
            "fortress.restore",
            "fortress.explain",
            "fortress.doctor"
        ]
    );
}
