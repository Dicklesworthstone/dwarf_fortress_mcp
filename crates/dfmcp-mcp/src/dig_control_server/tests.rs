use super::*;
use dfmcp_adapter::dig_control_policy::DigCheckpointPolicy;
use dfmcp_adapter::dig_designation::journal::{DigGuard, DigJournal, DigStage};
use dfmcp_adapter::dig_designation::{
    DigEffect,
    rpc::{DigManifest, DigPreparation},
};
use dfmcp_core::{MapCoord, MapCuboid};
use std::cell::RefCell;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::rc::Rc;

fn fixture(name: &str) -> Result<Vec<u8>> {
    let raw = match name {
        "observation" => {
            include_str!("../../../../tests/native/dig_designation/vectors/observation.hex")
        }
        "prepared" => include_str!("../../../../tests/native/dig_designation/vectors/prepared.hex"),
        "designated" => {
            include_str!("../../../../tests/native/dig_designation/vectors/designated.hex")
        }
        "cancelled" => {
            include_str!("../../../../tests/native/dig_designation/vectors/cancelled.hex")
        }
        _ => return Err(error(ErrorCode::InvalidRequest, "unknown fixture")),
    }
    .trim();
    (0..raw.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&raw[i..i + 2], 16)
                .map_err(|_| error(ErrorCode::InvalidRequest, "bad fixture"))
        })
        .collect()
}
#[derive(Clone, Default)]
struct Memory {
    data: Rc<RefCell<Cursor<Vec<u8>>>>,
    fail_terminal: Rc<RefCell<bool>>,
}
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.data.borrow_mut().read(out)
    }
}
impl Write for Memory {
    fn write(&mut self, out: &[u8]) -> io::Result<usize> {
        self.data.borrow_mut().write(out)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Seek for Memory {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.data.borrow_mut().seek(pos)
    }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        if *self.fail_terminal.borrow() && last_state(self.data.borrow().get_ref()) == Some(5) {
            Err(io::Error::other("terminal sync failed"))
        } else {
            Ok(())
        }
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("no repair"))
    }
}
fn last_state(bytes: &[u8]) -> Option<u8> {
    let n = u16::from_be_bytes(bytes.get(8..10)?.try_into().ok()?) as usize;
    let mut offset = 74 + n;
    let mut state = None;
    while offset < bytes.len() {
        let n = u32::from_be_bytes(bytes.get(offset + 8..offset + 12)?.try_into().ok()?) as usize;
        state = bytes.get(offset + 52).copied();
        offset += 92 + n;
    }
    state
}
struct Game {
    calls: Vec<DigStage>,
    record: Option<DigEffect>,
    lost: bool,
    factories: usize,
}
struct Source {
    game: Rc<RefCell<Game>>,
    binding: DigBinding,
    before: DigObservation,
    permit: bool,
}
impl DigSource for Source {
    fn manifest(&self) -> &DigManifest {
        self.binding.manifest()
    }
    fn endpoint(&self) -> Option<std::net::SocketAddr> {
        Some(self.binding.endpoint())
    }
    fn observe(&mut self, r: DigRegion, _: &OperationContext) -> Result<DigObservation> {
        assert_eq!(r, self.before.region());
        self.game.borrow_mut().calls.push(DigStage::Observe);
        Ok(self.before.clone())
    }
    fn prepare(&mut self, p: &DigPlan, _: &OperationContext) -> Result<DigPreparation> {
        let mut g = self.game.borrow_mut();
        g.calls.push(DigStage::Prepare);
        self.permit = true;
        let value = DigPreparation::decode(&fixture("prepared")?, false, p)?;
        g.record = Some(value.effect().clone());
        Ok(value)
    }
    fn commit(&mut self, p: &DigPlan, _: &OperationContext) -> Result<DigEffect> {
        assert!(self.permit);
        self.permit = false;
        let mut g = self.game.borrow_mut();
        g.calls.push(DigStage::Commit);
        let effect = DigEffect::decode(&fixture("designated")?, p)?;
        g.record = Some(effect.clone());
        if g.lost {
            Err(error(ErrorCode::AdapterRejected, "lost reply"))
        } else {
            Ok(effect)
        }
    }
    fn query(&mut self, _: &DigPlan, _: &OperationContext) -> Result<Option<DigEffect>> {
        let mut g = self.game.borrow_mut();
        g.calls.push(DigStage::Query);
        Ok(g.record.clone())
    }
    fn cancel(&mut self, p: &DigPlan, _: &OperationContext) -> Result<DigEffect> {
        let mut g = self.game.borrow_mut();
        g.calls.push(DigStage::Cancel);
        let effect = DigEffect::decode(&fixture("cancelled")?, p)?;
        g.record = Some(effect.clone());
        Ok(effect)
    }
}
struct Runtime {
    memory: Memory,
    revoke_commit: bool,
}
impl DigGuard for Runtime {
    fn check(&mut self, stage: DigStage, _: &DigPlan, _: &OperationContext) -> Result<()> {
        if stage == DigStage::Commit && self.revoke_commit {
            assert_eq!(last_state(self.memory.data.borrow().get_ref()), Some(2));
            return Err(error(
                ErrorCode::CapabilityDenied,
                "revoked after dispatch sync",
            ));
        }
        Ok(())
    }
}
impl DigSessionGuard for Runtime {
    fn connect(&mut self, _: &DigBinding, _: DigRegion, _: &OperationContext) -> Result<()> {
        Ok(())
    }
    fn observe(&mut self, _: &DigBinding, _: DigRegion, _: &OperationContext) -> Result<()> {
        Ok(())
    }
}
struct Harness {
    state: State<Memory, Source>,
    game: Rc<RefCell<Game>>,
    runtime: Runtime,
    c: OperationContext,
    before: DigObservation,
}
impl Harness {
    fn new(checkpoint: DigCheckpointPolicy) -> Result<Self> {
        let before = DigObservation::decode(&fixture("observation")?)?;
        let config = Config {
            path: "/private/dig/journal".into(),
            folder: before.folder().into(),
            site: before.site(),
            scope: MapCuboid {
                min: MapCoord::new(0, 0, 0),
                max: MapCoord::new(63, 63, 7),
            },
            endpoint: "127.0.0.1:5000".parse().map_err(|_| exhausted())?,
            protected: vec![],
            checkpoint,
        };
        let binding = DigBinding::new(
            config.endpoint,
            DigManifest {
                generation: 7,
                df_version: "test-df".into(),
                dfhack_version: "test-dfhack".into(),
            },
            &before,
            config.scope,
        )?;
        let c = context(
            SessionId::new(1),
            RequestId::new(1),
            &binding,
            before.tick(),
            WorkBudget {
                max_wall_millis: 60_000,
                max_game_ticks: 0,
                max_entities: 300,
                max_bytes: MAX_WORK_BYTES,
                max_output_tokens: 8192,
                max_actions: 1,
            },
            true,
        );
        let memory = Memory::default();
        let journal = DigJournal::open(
            memory.clone(),
            &c,
            DigMode::Control,
            Some(binding),
            Some([7; 32]),
        )?;
        let session = DigSession::new(journal, &c)?;
        Ok(Self {
            state: State::new(session, &c, &config)?,
            game: Rc::new(RefCell::new(Game {
                calls: vec![],
                record: None,
                lost: false,
                factories: 0,
            })),
            runtime: Runtime {
                memory,
                revoke_commit: false,
            },
            c,
            before,
        })
    }
    fn run(&mut self, op: &str, action: Result<Action>) -> Result<Value> {
        let game = self.game.clone();
        let before = self.before.clone();
        let raw = run_action(
            &mut self.state,
            self.c.clone(),
            op,
            action,
            Instant::now(),
            &mut self.runtime,
            move |b, r, _| {
                assert_eq!(r, before.region());
                game.borrow_mut().factories += 1;
                Ok(Source {
                    game,
                    binding: b.clone(),
                    before,
                    permit: false,
                })
            },
        );
        assert!(raw.len() <= OUTPUT_BYTES as usize);
        serde_json::from_str(&raw)
            .map_err(|_| error(ErrorCode::InternalInvariantViolation, "invalid packet"))
    }
    fn observe(&mut self) -> Result<Value> {
        self.run(
            "fortress.observe",
            Ok(Action::Observe(self.before.region())),
        )
    }
    fn prepare(&mut self) -> Result<Value> {
        self.run(
            "fortress.plan",
            Ok(Action::Plan {
                key: "dig-001".into(),
                witness: self.before.witness(),
                hidden: false,
            }),
        )
    }
    fn commit(&mut self, seal: Digest32) -> Result<Value> {
        let p = DigPlan::new("dig-001", false, self.before.clone())?;
        self.run(
            "fortress.commit",
            Ok(Action::Commit {
                key: p.key().into(),
                plan: p.digest(),
                seal,
            }),
        )
    }
}

#[test]
fn actual_handler_lifecycle_retains_one_connection_and_commits_once() -> Result<()> {
    let mut h = Harness::new(DigCheckpointPolicy::DisposableFortress)?;
    assert_eq!(h.observe()?["result"]["ok"], true);
    let prepared = h.prepare()?;
    assert_eq!(prepared["result"]["plan"]["state"], "prepared");
    let seal = digest(
        prepared["result"]["review_seal"]
            .as_str()
            .ok_or_else(exhausted)?,
    )?;
    let done = h.commit(seal)?;
    assert_eq!(done["result"]["effect"]["native"]["phase"], "designated");
    assert_eq!(h.game.borrow().factories, 1);
    assert_eq!(
        h.game.borrow().calls,
        vec![
            DigStage::Observe,
            DigStage::Observe,
            DigStage::Prepare,
            DigStage::Observe,
            DigStage::Commit
        ]
    );
    assert!(h.state.review.is_none());
    assert!(!h.state.control.has_preparation_connection());
    assert_eq!(h.commit(seal)?["result"]["native_calls"], 0);
    assert_eq!(
        h.game
            .borrow()
            .calls
            .iter()
            .filter(|s| **s == DigStage::Commit)
            .count(),
        1
    );
    Ok(())
}
#[test]
fn default_policy_blocks_native_prepare_without_creating_an_obligation() -> Result<()> {
    let mut h = Harness::new(DigCheckpointPolicy::Required)?;
    h.observe()?;
    let denied = h.prepare()?;
    assert_eq!(denied["result"]["error"]["code"], "checkpoint_required");
    assert_eq!(h.game.borrow().calls, vec![DigStage::Observe]);
    assert_eq!(h.state.control.view(&h.c)?.total_records, 0);
    Ok(())
}
#[test]
fn lost_commit_reply_is_recovered_only_by_query_and_original_key() -> Result<()> {
    let mut h = Harness::new(DigCheckpointPolicy::DisposableFortress)?;
    h.observe()?;
    h.prepare()?;
    let seal = h.state.seal().ok_or_else(exhausted)?;
    h.game.borrow_mut().lost = true;
    let lost = h.commit(seal)?;
    assert_eq!(lost["result"]["ok"], false);
    assert_eq!(
        lost["agent_turn"]["active_work"]["pending_plans"][0]["state"],
        "dispatch_started"
    );
    assert_eq!(h.commit(seal)?["result"]["ok"], false);
    let p = DigPlan::new("dig-001", false, h.before.clone())?;
    let recovered = h.run(
        "fortress.wait",
        Ok(Action::Wait("dig-001".into(), p.digest())),
    )?;
    assert_eq!(
        recovered["result"]["effect"]["native"]["phase"],
        "designated"
    );
    assert_eq!(
        h.game
            .borrow()
            .calls
            .iter()
            .filter(|s| **s == DigStage::Commit)
            .count(),
        1
    );
    assert_eq!(h.game.borrow().calls.last(), Some(&DigStage::Query));
    Ok(())
}
#[test]
fn wrong_confirmation_never_reaches_commit_and_revokes_ephemeral_permission() -> Result<()> {
    let mut h = Harness::new(DigCheckpointPolicy::DisposableFortress)?;
    h.observe()?;
    h.prepare()?;
    assert_eq!(h.commit(Digest32::ZERO)?["result"]["ok"], false);
    assert!(h.state.review.is_none());
    assert!(!h.game.borrow().calls.contains(&DigStage::Commit));
    assert_eq!(
        h.state
            .control
            .view(&h.c)?
            .pending
            .as_ref()
            .map(|p| p.state),
        Some(DigState::Prepared)
    );
    Ok(())
}
#[test]
fn expired_lease_blocks_commit_but_does_not_erase_the_preparation() -> Result<()> {
    let mut h = Harness::new(DigCheckpointPolicy::DisposableFortress)?;
    h.observe()?;
    h.prepare()?;
    let seal = h.state.seal().ok_or_else(exhausted)?;
    h.c.anchor.tick = GameTick(h.c.anchor.tick.get() + 1200);
    assert_eq!(h.commit(seal)?["result"]["ok"], false);
    assert!(!h.game.borrow().calls.contains(&DigStage::Commit));
    assert!(h.state.control.view(&h.c)?.pending.is_some());
    Ok(())
}
#[test]
fn runtime_revocation_after_dispatch_sync_preserves_uncertainty_without_native_commit() -> Result<()>
{
    let mut h = Harness::new(DigCheckpointPolicy::DisposableFortress)?;
    h.observe()?;
    h.prepare()?;
    let seal = h.state.seal().ok_or_else(exhausted)?;
    h.runtime.revoke_commit = true;
    let denied = h.commit(seal)?;
    assert_eq!(denied["result"]["ok"], false);
    assert!(!h.game.borrow().calls.contains(&DigStage::Commit));
    assert_eq!(
        last_state(h.runtime.memory.data.borrow().get_ref()),
        Some(2)
    );
    assert_eq!(
        denied["agent_turn"]["active_work"]["pending_plans"][0]["state"],
        "dispatch_started"
    );
    Ok(())
}
#[test]
fn failed_terminal_sync_never_acknowledges_a_known_durable_outcome() -> Result<()> {
    let mut h = Harness::new(DigCheckpointPolicy::DisposableFortress)?;
    h.observe()?;
    h.prepare()?;
    let seal = h.state.seal().ok_or_else(exhausted)?;
    *h.runtime.memory.fail_terminal.borrow_mut() = true;
    let failed = h.commit(seal)?;
    assert_eq!(failed["result"]["ok"], false);
    assert_eq!(
        failed["agent_turn"]["active_work"]["inventory_verified"],
        false
    );
    assert_eq!(
        failed["agent_turn"]["active_work"]["pending_absence_proven"],
        false
    );
    assert_eq!(
        failed["result"]["historical_pending_before_request"]["idempotency_key"],
        "dig-001"
    );
    assert_eq!(
        h.game
            .borrow()
            .calls
            .iter()
            .filter(|s| **s == DigStage::Commit)
            .count(),
        1
    );
    Ok(())
}
#[test]
fn output_budget_refusal_precedes_source_creation() -> Result<()> {
    let mut h = Harness::new(DigCheckpointPolicy::DisposableFortress)?;
    h.c.budget.max_output_tokens = 8191;
    assert_eq!(h.observe()?["result"]["ok"], false);
    assert_eq!(h.game.borrow().factories, 0);
    let c = context(
        h.c.session_id,
        h.c.request_id,
        &h.state.binding,
        h.before.tick(),
        WorkBudget {
            max_output_tokens: 8192,
            ..h.c.budget
        },
        true,
    );
    assert_eq!(h.state.control.view(&c)?.total_records, 0);
    Ok(())
}
#[test]
fn malformed_refresh_abandons_selection_without_forgetting_pending_work() -> Result<()> {
    let mut h = Harness::new(DigCheckpointPolicy::DisposableFortress)?;
    h.observe()?;
    h.prepare()?;
    let denied = h.run(
        "fortress.observe",
        Err(error(ErrorCode::InvalidRequest, "bad region")),
    )?;
    assert_eq!(denied["result"]["ok"], false);
    assert!(h.state.review.is_none());
    assert_eq!(
        denied["agent_turn"]["active_work"]["pending_plans"][0]["idempotency_key"],
        "dig-001"
    );
    Ok(())
}
#[test]
fn exact_retained_tiles_are_local_and_do_not_reobserve_native_state() -> Result<()> {
    let mut h = Harness::new(DigCheckpointPolicy::DisposableFortress)?;
    h.observe()?;
    h.prepare()?;
    let p = DigPlan::new("dig-001", false, h.before.clone())?;
    let n = h.game.borrow().calls.len();
    let value = h.run(
        "fortress.query",
        Ok(Action::Query(Query::PlanTiles {
            idempotency_key: "dig-001".into(),
            plan_digest: p.digest().to_string(),
            offset: 0,
            limit: Some(16),
        })),
    )?;
    assert_eq!(
        value["result"]["page"]["tiles"].as_array().map(Vec::len),
        Some(16)
    );
    assert_eq!(h.game.borrow().calls.len(), n);
    assert_eq!(value["result"]["native_calls"], 0);
    assert!(Query::parse(&value["result"]["page"]["next_query"].to_string()).is_ok());
    Ok(())
}
#[test]
fn closed_queries_reject_wrong_shapes_and_limits() {
    for raw in [
        "{\"mode\":\"schema\"}",
        "{\"mode\":\"records\",\"limit\":8}",
    ] {
        assert!(Query::parse(raw).is_ok());
    }
    for raw in [
        "{\"mode\":\"commit\"}",
        "{\"mode\":\"schema\",\"command\":\"x\"}",
        "{\"mode\":\"records\",\"limit\":9}",
        "{\"mode\":\"records\",\"limit\":true}",
        "{\"mode\":\"records\",\"limit\":null}",
        "{\"mode\":\"records\",\"limit\":1.0}",
        "{\"mode\":\"schema\",\"mode\":\"records\"}",
    ] {
        assert!(Query::parse(raw).is_err());
    }
    for raw in ["[1,1,1,9,1]", "[0,1,1,1,1]", "[1,1,1,1.0,1]", "[1,1,1,1]"] {
        assert!(parse_region(raw).is_err());
    }
}
#[test]
fn cancelling_preparation_is_native_retirement_not_undo() -> Result<()> {
    let mut h = Harness::new(DigCheckpointPolicy::DisposableFortress)?;
    h.observe()?;
    h.prepare()?;
    let p = DigPlan::new("dig-001", false, h.before.clone())?;
    let cancelled = h.run(
        "fortress.cancel",
        Ok(Action::Cancel("dig-001".into(), p.digest())),
    )?;
    assert_eq!(
        cancelled["result"]["effect"]["native"]["reason"],
        "cancelled_before_dispatch"
    );
    assert_eq!(cancelled["result"]["designation_undone"], false);
    assert!(!h.game.borrow().calls.contains(&DigStage::Commit));
    assert!(h.state.review.is_none());
    Ok(())
}

#[test]
fn protected_block_outside_target_is_rejected_before_native_preparation() -> Result<()> {
    let mut h = Harness::new(DigCheckpointPolicy::DisposableFortress)?;
    let view = h.state.control.view(&h.c)?;
    h.state.policy = DigControlPolicy::new(
        h.state.binding.clone(),
        view.journal_id,
        h.c.session_id,
        h.state.policy.lease_id(),
        vec![MapCuboid {
            min: MapCoord::new(0, 0, 2),
            max: MapCoord::new(0, 0, 2),
        }],
        DigCheckpointPolicy::DisposableFortress,
    )?;
    h.observe()?;
    assert_eq!(h.prepare()?["result"]["ok"], false);
    assert_eq!(h.game.borrow().calls, vec![DigStage::Observe]);
    assert_eq!(h.state.control.view(&h.c)?.total_records, 0);
    Ok(())
}
#[test]
fn reopened_prepared_journal_cannot_reconstruct_review_or_native_permission() -> Result<()> {
    let mut h = Harness::new(DigCheckpointPolicy::DisposableFortress)?;
    h.observe()?;
    h.prepare()?;
    let seal = h.state.seal().ok_or_else(exhausted)?;
    h.state.abandon();
    let config = Config {
        path: "/private/dig/journal".into(),
        folder: h.before.folder().into(),
        site: h.before.site(),
        scope: h.state.binding.scope(),
        endpoint: h.state.binding.endpoint(),
        protected: vec![],
        checkpoint: DigCheckpointPolicy::DisposableFortress,
    };
    let journal = DigJournal::open(
        h.runtime.memory.clone(),
        &h.c,
        DigMode::Control,
        Some(h.state.binding.clone()),
        None,
    )?;
    h.state = State::new(DigSession::new(journal, &h.c)?, &h.c, &config)?;
    let attempts = h.game.borrow().factories;
    assert_eq!(h.commit(seal)?["result"]["ok"], false);
    assert_eq!(h.game.borrow().factories, attempts);
    assert!(!h.game.borrow().calls.contains(&DigStage::Commit));
    assert!(h.state.control.view(&h.c)?.pending.is_some());
    Ok(())
}
#[test]
fn native_unknown_remains_pending_without_polling_or_new_commit_authority() -> Result<()> {
    let mut h = Harness::new(DigCheckpointPolicy::DisposableFortress)?;
    h.observe()?;
    h.prepare()?;
    let seal = h.state.seal().ok_or_else(exhausted)?;
    h.game.borrow_mut().lost = true;
    h.commit(seal)?;
    let p = DigPlan::new("dig-001", false, h.before.clone())?;
    let mut raw = fixture("prepared")?;
    raw[133] = 1;
    h.game.borrow_mut().record = Some(DigEffect::decode(&raw, &p)?);
    let first = h.run(
        "fortress.wait",
        Ok(Action::Wait("dig-001".into(), p.digest())),
    )?;
    assert_eq!(first["result"]["effect"]["permanent_unknown"], true);
    let calls = h.game.borrow().calls.len();
    let factories = h.game.borrow().factories;
    let second = h.run(
        "fortress.wait",
        Ok(Action::Wait("dig-001".into(), p.digest())),
    )?;
    assert_eq!(second["result"]["effect"]["permanent_unknown"], true);
    assert_eq!(h.game.borrow().calls.len(), calls);
    assert_eq!(h.game.borrow().factories, factories);
    assert_eq!(
        second["agent_turn"]["active_work"]["pending_absence_proven"],
        false
    );
    Ok(())
}
#[test]
fn changing_current_grants_cannot_be_overridden_by_a_review_seal() -> Result<()> {
    let mut h = Harness::new(DigCheckpointPolicy::DisposableFortress)?;
    h.observe()?;
    h.prepare()?;
    let seal = h.state.seal().ok_or_else(exhausted)?;
    h.c.grants.retain(|g| g.capability != Capability::Designate);
    assert_eq!(h.commit(seal)?["result"]["ok"], false);
    assert!(!h.game.borrow().calls.contains(&DigStage::Commit));
    assert_eq!(
        last_state(h.runtime.memory.data.borrow().get_ref()),
        Some(1)
    );
    Ok(())
}
#[test]
fn complete_worst_case_tile_response_and_pending_policy_fit_reservation() -> Result<()> {
    let mut raw = b"DFMDG016".to_vec();
    for n in [u64::MAX - 1, u64::MAX - 2, MAX_NATIVE_TICK] {
        raw.extend_from_slice(&n.to_be_bytes());
    }
    for n in [
        i32::MAX as u32,
        32768,
        32768,
        32768,
        32759,
        32759,
        32766,
        8,
        8,
    ] {
        raw.extend_from_slice(&n.to_be_bytes());
    }
    raw.push(1);
    raw.extend_from_slice(&512u16.to_be_bytes());
    raw.extend_from_slice(&[b'"'; 512]);
    raw.extend_from_slice(&300u16.to_be_bytes());
    for _ in 0..300 {
        raw.push(2);
        for n in [u32::MAX, u32::MAX, u32::MAX, 7000, u32::MAX, u32::MAX] {
            raw.extend_from_slice(&n.to_be_bytes());
        }
        raw.extend_from_slice(&[255, 255, 255, 255, 0, 0, 1]);
    }
    let captured = DigObservation::decode(&raw)?;
    let mut h = Harness::new(DigCheckpointPolicy::DisposableFortress)?;
    h.observe()?;
    h.prepare()?;
    let mut view = h.state.control.view(&h.c)?;
    if let Some(pending) = &mut view.pending {
        pending.key = "k".repeat(128);
    }
    let protected = (0..32)
        .map(|x| MapCuboid {
            min: MapCoord::new(x, 0, 0),
            max: MapCoord::new(x, 32767, 32767),
        })
        .collect();
    let policy = DigControlPolicy::new(
        h.state.binding.clone(),
        view.journal_id,
        h.c.session_id,
        h.state.policy.lease_id(),
        protected,
        DigCheckpointPolicy::DisposableFortress,
    )?;
    let result = json!({"ok":true,"page":presentation::tile_page(&captured,0,16)?,"observation":presentation::observation(&captured)});
    let text = packet(
        "fortress.query",
        result,
        Some(&h.c),
        Some(&h.state.binding),
        Some(&view),
        Some(&policy),
        h.state.seal(),
    );
    assert!(text.len() <= OUTPUT_BYTES as usize);
    let parsed: Value = serde_json::from_str(&text).map_err(|_| exhausted())?;
    assert_eq!(
        parsed["agent_turn"]["briefing"]["excavation_completion_proven"],
        false
    );
    assert_eq!(
        parsed["agent_turn"]["active_work"]["pending_plans"][0]["idempotency_key"],
        "k".repeat(128)
    );
    Ok(())
}
