use super::*;
use dfmcp_adapter::build_placement::journal::{BuildDispatch, BuildJournal, BuildStage};
use dfmcp_adapter::build_placement::{
    BuildCapture, BuildNativeSummary, BuildPhase, BuildPreparation, BuildRecord,
};
use dfmcp_core::{MapCoord, MapCuboid};
use std::cell::RefCell;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::rc::Rc;
fn fixture(name: &str) -> Result<Vec<u8>> {
    let value: Value = serde_json::from_str(include_str!(
        "../../../../bridge/common/tests/fixtures/build_placement_v1_19.json"
    ))
    .map_err(|_| error(ErrorCode::InvalidRequest, "bad native fixture"))?;
    let raw = value[name]
        .as_str()
        .ok_or_else(|| error(ErrorCode::InvalidRequest, "fixture missing"))?;
    raw.as_bytes()
        .chunks_exact(2)
        .map(|v| {
            let s = std::str::from_utf8(v).map_err(|_| exhausted())?;
            u8::from_str_radix(s, 16).map_err(|_| exhausted())
        })
        .collect()
}
fn capture() -> Result<BuildCapture> {
    BuildCapture::decode(&fixture("capture")?)
}
#[derive(Clone, Default)]
struct Memory {
    bytes: Rc<RefCell<Cursor<Vec<u8>>>>,
    corrupt: Rc<RefCell<bool>>,
}
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.bytes.borrow_mut().read(out)
    }
}
impl Write for Memory {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.bytes.borrow_mut().write(data)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Seek for Memory {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.bytes.borrow_mut().seek(pos)
    }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("no repair"))
    }
    fn validate_identity(&self) -> io::Result<()> {
        if *self.corrupt.borrow() {
            Err(io::Error::other("custody changed"))
        } else {
            Ok(())
        }
    }
}
#[derive(Default)]
struct Game {
    calls: Vec<&'static str>,
    record: Option<BuildRecord>,
    lost_commit: bool,
    indeterminate: bool,
    other_unresolved: bool,
    later_tick: Option<u64>,
    factories: usize,
}
fn game_summary(game: &Game) -> Result<BuildNativeSummary> {
    BuildNativeSummary::new(
        game.other_unresolved
            || game
                .record
                .as_ref()
                .is_some_and(|r| r.phase() == BuildPhase::Indeterminate),
        u16::from(game.record.is_some()) + u16::from(game.other_unresolved),
    )
}
struct Native {
    binding: BuildBinding,
    game: Rc<RefCell<Game>>,
    fenced: bool,
    permit: bool,
    summary: BuildNativeSummary,
}
impl BuildSource for Native {
    fn native_summary(&self) -> BuildNativeSummary {
        self.summary
    }
    fn binding(&self) -> &BuildBinding {
        &self.binding
    }
    fn fence(&mut self) {
        self.fenced = true;
        self.permit = false;
    }
    fn observe(
        &mut self,
        s: BuildSelection,
        _: &OperationContext,
        _: Duration,
    ) -> Result<BuildCapture> {
        assert!(!self.fenced);
        let before = if let Some(tick) = self.game.borrow().later_tick {
            let mut bytes = fixture("capture")?;
            bytes[24..32].copy_from_slice(&tick.to_be_bytes());
            BuildCapture::decode(&bytes)?
        } else {
            capture()?
        };
        assert_eq!(s, before.selection());
        self.game.borrow_mut().calls.push("observe");
        Ok(before)
    }
    fn prepare(
        &mut self,
        p: &BuildPlan,
        _: &OperationContext,
        _: Duration,
    ) -> Result<BuildPreparation> {
        assert!(!self.fenced);
        let record = BuildRecord::decode(&fixture("prepared")?)?;
        record.verify_plan(p)?;
        self.permit = true;
        let mut game = self.game.borrow_mut();
        game.calls.push("prepare");
        game.record = Some(record.clone());
        self.summary = game_summary(&game)?;
        BuildPreparation::new(record, false)
    }
    fn commit(
        &mut self,
        d: BuildDispatch<'_>,
        _: &OperationContext,
        _: Duration,
    ) -> Result<BuildRecord> {
        assert!(!self.fenced && self.permit);
        self.permit = false;
        assert_ne!(d.journal_head(), Digest32::ZERO);
        let record = BuildRecord::decode(&fixture(if self.game.borrow().indeterminate {
            "indeterminate"
        } else {
            "placed"
        })?)?;
        record.verify_plan(d.plan())?;
        let mut game = self.game.borrow_mut();
        game.calls.push("commit");
        game.record = Some(record.clone());
        self.summary = game_summary(&game)?;
        if game.lost_commit {
            Err(error(ErrorCode::AdapterUnavailable, "lost commit reply"))
        } else {
            Ok(record)
        }
    }
    fn query(
        &mut self,
        p: &BuildPlan,
        _: &OperationContext,
        _: Duration,
    ) -> Result<Option<BuildRecord>> {
        assert!(!self.fenced);
        self.permit = false;
        let mut game = self.game.borrow_mut();
        game.calls.push("query");
        if let Some(r) = &game.record {
            r.verify_plan(p)?;
        }
        self.summary = game_summary(&game)?;
        Ok(game.record.clone())
    }
    fn cancel(&mut self, p: &BuildPlan, _: &OperationContext, _: Duration) -> Result<BuildRecord> {
        assert!(!self.fenced);
        self.permit = false;
        let record = BuildRecord::decode(&fixture("cancelled")?)?;
        record.verify_plan(p)?;
        let mut game = self.game.borrow_mut();
        game.calls.push("cancel");
        game.record = Some(record.clone());
        self.summary = game_summary(&game)?;
        Ok(record)
    }
}
#[derive(Default)]
struct Guard {
    deny_commit: bool,
}
impl BuildGuard for Guard {
    fn check(
        &mut self,
        stage: BuildStage,
        _: &BuildBinding,
        _: Option<&BuildPlan>,
        _: BuildSelection,
        _: &OperationContext,
    ) -> Result<()> {
        if self.deny_commit && stage == BuildStage::Commit {
            Err(error(ErrorCode::CapabilityDenied, "revoked"))
        } else {
            Ok(())
        }
    }
}
fn config() -> Result<Config> {
    Ok(Config {
        path: "/private/build/journal".into(),
        scope: MapCuboid::new(MapCoord::new(0, 0, 0), MapCoord::new(63, 63, 7))?,
        fortress: capture()?.fortress().clone(),
        endpoint: "127.0.0.1:5000".parse().map_err(|_| exhausted())?,
        protected: vec![],
        checkpoint: runtime::CheckpointPolicy::DisposableFortress,
        mode: BuildMode::Control,
    })
}
fn budget() -> WorkBudget {
    WorkBudget {
        max_wall_millis: 60000,
        max_bytes: MAX_WORK_BYTES,
        max_output_tokens: 16384,
        max_entities: 65536,
        max_actions: 1,
        max_game_ticks: 0,
    }
}
type TestState = State<Memory, Native>;
fn setup() -> Result<(TestState, Memory, Rc<RefCell<Game>>)> {
    let config = config()?;
    let before = capture()?;
    let binding = BuildBinding::new(config.endpoint, "fake-df", "fake-dfhack", &before)?;
    let c = context(
        SessionId::new(71),
        RequestId::new(1),
        config.fortress.fortress_id(),
        before.tick(),
        budget(),
        BuildMode::Control,
        true,
    );
    let memory = Memory::default();
    let journal = BuildJournal::open(
        memory.clone(),
        &c,
        BuildMode::Control,
        Some(binding),
        Some([7; 32]),
    )?;
    let session = BuildSession::new(journal, &c)?;
    let state = State::new(session, &c, &config)?;
    Ok((state, memory, Rc::new(RefCell::new(Game::default()))))
}
fn call(
    state: &mut TestState,
    game: &Rc<RefCell<Game>>,
    op: &str,
    action: Result<Action>,
    guard: &mut Guard,
) -> Result<Value> {
    let c = state.context(true, None)?;
    let game = game.clone();
    let raw = run_action(
        state,
        c,
        op,
        action,
        Instant::now(),
        guard,
        move |_, _, b, _, _| {
            game.borrow_mut().factories += 1;
            let summary = {
                let native = game.borrow();
                game_summary(&native)?
            };
            Ok(Native {
                binding: b.clone(),
                game,
                fenced: false,
                permit: false,
                summary,
            })
        },
    );
    assert!(raw.len() as u64 <= OUTPUT_BYTES);
    serde_json::from_str(&raw).map_err(|_| error(ErrorCode::InvalidRequest, "bad handler output"))
}
fn observe(state: &mut TestState, game: &Rc<RefCell<Game>>) -> Result<Value> {
    call(
        state,
        game,
        "fortress.observe",
        Ok(Action::Observe(capture()?.selection())),
        &mut Guard::default(),
    )
}
fn prepare(state: &mut TestState, game: &Rc<RefCell<Game>>) -> Result<Value> {
    let observed = observe(state, game)?;
    assert_eq!(observed["result"]["ok"], true);
    call(
        state,
        game,
        "fortress.plan",
        Ok(Action::Plan {
            key: "golden".into(),
            witness: capture()?.witness(),
        }),
        &mut Guard::default(),
    )
}
fn plan() -> Result<BuildPlan> {
    BuildPlan::new("golden", capture()?)
}
fn commit(
    state: &mut TestState,
    game: &Rc<RefCell<Game>>,
    seal: Digest32,
    guard: &mut Guard,
) -> Result<Value> {
    call(
        state,
        game,
        "fortress.commit",
        Ok(Action::Commit {
            key: "golden".into(),
            plan: plan()?.digest(),
            seal,
        }),
        guard,
    )
}
#[test]
fn actual_handler_lifecycle_uses_original_source_and_one_writer() -> Result<()> {
    let (mut state, _, game) = setup()?;
    let review = prepare(&mut state, &game)?;
    assert_eq!(review["result"]["ok"], true);
    assert_eq!(review["result"]["plan"]["native"]["insertion"], Value::Null);
    assert_eq!(review["agent_turn"]["anchor"], Value::Null);
    assert_eq!(
        review["agent_turn"]["active_work"]["pending_plans"][0]["idempotency_key"],
        "golden"
    );
    let seal = digest(
        review["result"]["review_seal"]
            .as_str()
            .ok_or_else(exhausted)?,
    )?;
    let result = commit(&mut state, &game, seal, &mut Guard::default())?;
    assert_eq!(
        result["result"]["effect"]["summary"]["native_phase"],
        "placed"
    );
    assert_eq!(
        result["agent_turn"]["briefing"]["building_completion_proven"],
        false
    );
    assert_eq!(game.borrow().factories, 1);
    let insertion = json!({
        "building_id":70,"job_id":90,"item_id":42,"kind":"bed","position":[15,15,2],
        "material":419,"material_index":-1,"stage":0,"max_stage":1,"linked":true,
        "construct_job":true,"exact_item_link":true,"suspended":false,"historical_evidence_only":true
    });
    assert_eq!(result["result"]["effect"]["native"]["insertion"], insertion);
    let replay = commit(&mut state, &game, Digest32::ZERO, &mut Guard::default())?;
    assert_eq!(replay["result"]["historical_replay"], true);
    assert_eq!(replay["result"]["effect"]["native"]["insertion"], insertion);
    assert_eq!(
        game.borrow()
            .calls
            .iter()
            .filter(|&&c| c == "commit")
            .count(),
        1
    );
    Ok(())
}
#[test]
fn changed_review_refuses_effect_and_keeps_pending_identity() -> Result<()> {
    let (mut state, _, game) = setup()?;
    prepare(&mut state, &game)?;
    let result = commit(&mut state, &game, Digest32::ZERO, &mut Guard::default())?;
    assert_eq!(result["result"]["ok"], false);
    assert_eq!(
        game.borrow()
            .calls
            .iter()
            .filter(|&&s| s == "commit")
            .count(),
        0
    );
    assert_eq!(
        result["agent_turn"]["active_work"]["pending_plans"][0]["idempotency_key"],
        "golden"
    );
    assert!(state.seal().is_none());
    Ok(())
}
#[test]
fn lost_commit_reply_requires_query_and_never_dispatches_twice() -> Result<()> {
    let (mut state, _, game) = setup()?;
    let review = prepare(&mut state, &game)?;
    game.borrow_mut().lost_commit = true;
    let seal = digest(
        review["result"]["review_seal"]
            .as_str()
            .ok_or_else(exhausted)?,
    )?;
    let lost = commit(&mut state, &game, seal, &mut Guard::default())?;
    assert_eq!(lost["result"]["ok"], false);
    assert_eq!(
        lost["agent_turn"]["active_work"]["pending_plans"][0]["dispatch_started"],
        true
    );
    let retry = commit(&mut state, &game, seal, &mut Guard::default())?;
    assert_eq!(retry["result"]["ok"], false);
    let recovered = call(
        &mut state,
        &game,
        "fortress.wait",
        Ok(Action::Recover("golden".into(), plan()?.digest(), false)),
        &mut Guard::default(),
    )?;
    assert_eq!(
        recovered["result"]["effect"]["summary"]["native_phase"],
        "placed"
    );
    assert_eq!(
        game.borrow()
            .calls
            .iter()
            .filter(|&&s| s == "commit")
            .count(),
        1
    );
    assert_eq!(game.borrow().factories, 2);
    Ok(())
}
#[test]
fn reopened_preparation_has_no_review_or_commit_connection() -> Result<()> {
    let (mut state, memory, game) = setup()?;
    let review = prepare(&mut state, &game)?;
    let oldseal = digest(
        review["result"]["review_seal"]
            .as_str()
            .ok_or_else(exhausted)?,
    )?;
    let binding = state.binding.clone();
    drop(state);
    let config = config()?;
    let c = context(
        SessionId::new(72),
        RequestId::new(1),
        config.fortress.fortress_id(),
        capture()?.tick(),
        budget(),
        BuildMode::Control,
        true,
    );
    let journal = BuildJournal::open(memory, &c, BuildMode::Control, Some(binding), None)?;
    let mut state = State::new(BuildSession::new(journal, &c)?, &c, &config)?;
    let denied = commit(&mut state, &game, oldseal, &mut Guard::default())?;
    assert_eq!(denied["result"]["ok"], false);
    assert_eq!(game.borrow().factories, 1);
    assert_eq!(
        game.borrow()
            .calls
            .iter()
            .filter(|&&s| s == "commit")
            .count(),
        0
    );
    Ok(())
}
#[test]
fn cancellation_remains_available_with_only_query_grant() -> Result<()> {
    let (mut state, _, game) = setup()?;
    prepare(&mut state, &game)?;
    state.grants.retain(|g| g.capability == Capability::Query);
    let value = call(
        &mut state,
        &game,
        "fortress.cancel",
        Ok(Action::Recover("golden".into(), plan()?.digest(), true)),
        &mut Guard::default(),
    )?;
    assert_eq!(value["result"]["ok"], true);
    assert_eq!(
        value["result"]["effect"]["native"]["insertion"],
        Value::Null
    );
    assert_eq!(
        value["result"]["effect"]["summary"]["native_phase"],
        "cancelled"
    );
    assert_eq!(
        game.borrow()
            .calls
            .iter()
            .filter(|&&s| s == "commit")
            .count(),
        0
    );
    Ok(())
}
#[test]
fn strict_checkpoint_protected_item_and_target_refuse_before_intent() -> Result<()> {
    for case in 0..3 {
        let (mut state, _, game) = setup()?;
        if case == 0 {
            state.policy.config.checkpoint = runtime::CheckpointPolicy::Required;
        } else {
            let p = if case == 1 { [10, 11, 2] } else { [15, 15, 2] };
            state.policy.config.protected = vec![MapCuboid::new(
                MapCoord::new(p[0], p[1], p[2]),
                MapCoord::new(p[0], p[1], p[2]),
            )?];
        }
        let value = prepare(&mut state, &game)?;
        assert_eq!(value["result"]["ok"], false);
        assert_eq!(
            value["agent_turn"]["active_work"]["pending_absence_proven"],
            true
        );
        assert!(!game.borrow().calls.contains(&"prepare"));
    }
    Ok(())
}
#[test]
fn released_lease_and_current_runtime_revocation_refuse_commit() -> Result<()> {
    for release in [true, false] {
        let (mut state, _, game) = setup()?;
        let review = prepare(&mut state, &game)?;
        let seal = digest(
            review["result"]["review_seal"]
                .as_str()
                .ok_or_else(exhausted)?,
        )?;
        if release {
            state.leases.release_lease(state.policy.lease, state.id)?;
        }
        let result = commit(
            &mut state,
            &game,
            seal,
            &mut Guard {
                deny_commit: !release,
            },
        )?;
        assert_eq!(result["result"]["ok"], false);
        assert!(!game.borrow().calls.contains(&"commit"));
    }
    Ok(())
}
#[test]
fn insufficient_response_reservation_fails_before_work_and_retains_pending() -> Result<()> {
    let (mut state, _, game) = setup()?;
    prepare(&mut state, &game)?;
    let before = game.borrow().calls.len();
    state.budget.max_output_tokens = 1;
    let value = call(
        &mut state,
        &game,
        "fortress.plan",
        Ok(Action::Plan {
            key: "golden".into(),
            witness: capture()?.witness(),
        }),
        &mut Guard::default(),
    )?;
    assert_eq!(value["result"]["ok"], false);
    assert_eq!(game.borrow().calls.len(), before);
    assert_eq!(
        value["agent_turn"]["active_work"]["pending_plans"][0]["idempotency_key"],
        "golden"
    );
    assert_eq!(
        value["agent_turn"]["active_work"]["inventory_verified"],
        false
    );
    Ok(())
}
#[test]
fn custody_failure_withdraws_certainty_and_preserves_historical_pending() -> Result<()> {
    let (mut state, memory, game) = setup()?;
    prepare(&mut state, &game)?;
    *memory.corrupt.borrow_mut() = true;
    let value = call(
        &mut state,
        &game,
        "fortress.doctor",
        Ok(Action::Inventory),
        &mut Guard::default(),
    )?;
    assert_eq!(value["result"]["ok"], false);
    assert_eq!(
        value["agent_turn"]["active_work"]["inventory_verified"],
        false
    );
    assert_eq!(
        value["agent_turn"]["active_work"]["pending_plans"][0]["idempotency_key"],
        "golden"
    );
    assert!(state.seal().is_none());
    Ok(())
}
#[test]
fn closed_query_and_selection_parser_reject_ambiguous_authority() {
    for raw in [
        "{}",
        "{\"mode\":\"schema\",\"path\":\"/tmp\"}",
        "{\"mode\":\"schema\",\"mode\":\"schema\"}",
        "{\"mode\":\"records\",\"limit\":null}",
        "{\"mode\":\"records\",\"limit\":1,\"limit\":2}",
        "{\"mode\":\"records\",\"path\":\"/tmp\"}",
        "{\"mode\":\"records\",\"offset\":1}",
        "{\"mode\":\"records\",\"limit\":9}",
        "{\"mode\":\"records\",\"limit\":1.0}",
    ] {
        assert!(Query::parse(raw).is_err(), "{raw}");
    }
    for raw in [
        "[\"bed\",42,15,15,2]",
        "[\"chair\",0,1,1,0]",
        "[\"table\",42,15,15,2]",
    ] {
        assert!(parse_selection(raw).is_ok());
    }
    for raw in [
        "[\"workshop\",42,15,15,2]",
        "[\"bed\",42,15,15,2,0]",
        "[\"bed\",42,-1,15,2]",
        "[\"bed\",42,15.0,15,2]",
    ] {
        assert!(parse_selection(raw).is_err());
    }
}
#[test]
fn historical_query_is_source_free_and_checkpoint_refusal_keeps_agent_turn() -> Result<()> {
    let (mut state, _, game) = setup()?;
    prepare(&mut state, &game)?;
    let before = game.borrow().calls.len();
    let value = call(
        &mut state,
        &game,
        "fortress.query",
        Ok(Action::Query(Query::Records {
            limit: Some(1),
            offset: None,
            head: None,
        })),
        &mut Guard::default(),
    )?;
    assert_eq!(value["result"]["records"][0]["idempotency_key"], "golden");
    assert_eq!(value["agent_turn"]["anchor"], Value::Null);
    let refused = call(
        &mut state,
        &game,
        "fortress.checkpoint",
        Ok(Action::Denied),
        &mut Guard::default(),
    )?;
    assert_eq!(refused["result"]["ok"], false);
    assert_eq!(refused["agent_turn"]["schema"], "dfmcp.agent_turn/1");
    assert_eq!(game.borrow().calls.len(), before);
    Ok(())
}

#[test]
fn local_review_replay_rechecks_current_construction_grant() -> Result<()> {
    let (mut state, _, game) = setup()?;
    prepare(&mut state, &game)?;
    let before = game.borrow().calls.len();
    state
        .grants
        .retain(|g| g.capability != Capability::Construct);
    let value = call(
        &mut state,
        &game,
        "fortress.plan",
        Ok(Action::Plan {
            key: "golden".into(),
            witness: capture()?.witness(),
        }),
        &mut Guard::default(),
    )?;
    assert_eq!(value["result"]["ok"], false);
    assert_eq!(game.borrow().calls.len(), before);
    assert!(state.seal().is_none());
    assert_eq!(
        value["agent_turn"]["active_work"]["pending_plans"][0]["idempotency_key"],
        "golden"
    );
    Ok(())
}

#[test]
fn immutable_uncertain_receipt_remains_indeterminate_in_agent_state() -> Result<()> {
    let (mut state, _, game) = setup()?;
    let review = prepare(&mut state, &game)?;
    game.borrow_mut().indeterminate = true;
    let seal = digest(
        review["result"]["review_seal"]
            .as_str()
            .ok_or_else(exhausted)?,
    )?;
    let value = commit(&mut state, &game, seal, &mut Guard::default())?;
    assert_eq!(value["result"]["ok"], true);
    assert_eq!(
        value["result"]["effect"]["summary"]["state"],
        "indeterminate"
    );
    assert_eq!(
        value["result"]["effect"]["summary"]["coordinator_state"],
        "terminal"
    );
    assert_eq!(
        value["agent_turn"]["active_work"]["pending_plans"][0]["state"],
        "indeterminate"
    );
    assert_eq!(
        value["agent_turn"]["active_work"]["pending_absence_proven"],
        false
    );
    let before = game.borrow().calls.len();
    let recovered = call(
        &mut state,
        &game,
        "fortress.wait",
        Ok(Action::Recover("golden".into(), plan()?.digest(), false)),
        &mut Guard::default(),
    )?;
    assert_eq!(
        recovered["result"]["effect"]["summary"]["state"],
        "indeterminate"
    );
    assert_eq!(game.borrow().calls.len(), before);
    Ok(())
}

fn assert_no_retained_disclosure(value: &Value) -> Result<()> {
    assert_eq!(value["agent_turn"]["session_id"], Value::Null);
    assert_eq!(value["agent_turn"]["request_id"], Value::Null);
    assert_eq!(value["agent_turn"]["briefing"]["source"], Value::Null);
    assert_eq!(
        value["agent_turn"]["briefing"]["development_policy"],
        Value::Null
    );
    assert_eq!(value["agent_turn"]["references"], json!([]));
    assert_eq!(
        value["agent_turn"]["active_work"]["pending_plans"],
        json!([])
    );
    assert_eq!(
        value["agent_turn"]["active_work"]["indeterminate_effects"],
        json!([])
    );
    assert_eq!(value["agent_turn"]["active_work"]["counts"], Value::Null);
    assert_eq!(value["agent_turn"]["active_work"]["recovery"], Value::Null);
    assert_eq!(
        value["agent_turn"]["active_work"]["unverified_operation_identity"],
        Value::Null
    );
    assert_eq!(
        value["agent_turn"]["active_work"]["pending_absence_proven"],
        false
    );
    assert_eq!(value["result"]["pending_identity_hint"], Value::Null);
    let raw = value.to_string();
    assert!(!raw.contains("golden"));
    assert!(!raw.contains(&plan()?.digest().to_string()));
    assert!(!raw.contains(capture()?.fortress().folder()));
    Ok(())
}

#[test]
fn denied_query_never_discloses_cached_history_or_error_hints() -> Result<()> {
    for case in 0..3 {
        let (mut state, memory, game) = setup()?;
        prepare(&mut state, &game)?;
        let original_grants = state.grants.clone();
        state.pending_hint =
            Some(json!({"idempotency_key":"golden","plan_digest":plan()?.digest().to_string()}));
        match case {
            0 => state.grants.retain(|g| g.capability != Capability::Query),
            1 => {
                for grant in &mut state.grants {
                    if grant.capability == Capability::Query {
                        grant.expires_at_tick = Some(GameTick(capture()?.tick() - 1));
                    }
                }
            }
            _ => {
                for grant in &mut state.grants {
                    if grant.capability == Capability::Query {
                        grant.remaining_uses = Some(1);
                    }
                }
            }
        }
        let before_calls = game.borrow().calls.len();
        let before_bytes = memory.bytes.borrow().get_ref().clone();
        let value = call(
            &mut state,
            &game,
            "fortress.query",
            Ok(Action::Inventory),
            &mut Guard::default(),
        )?;
        assert_eq!(
            value["result"]["error"]["code"],
            ErrorCode::CapabilityDenied.as_str()
        );
        assert_no_retained_disclosure(&value)?;
        assert_eq!(game.borrow().calls.len(), before_calls);
        assert_eq!(*memory.bytes.borrow().get_ref(), before_bytes);
        assert!(state.seal().is_none());

        // Redaction changes presentation only. A newly authorized read still
        // exposes the original durable obligation without contacting native.
        state.grants = original_grants;
        let authorized = call(
            &mut state,
            &game,
            "fortress.query",
            Ok(Action::Inventory),
            &mut Guard::default(),
        )?;
        assert_eq!(authorized["result"]["ok"], true);
        assert_eq!(
            authorized["agent_turn"]["active_work"]["pending_plans"][0]["idempotency_key"],
            "golden"
        );
        assert_eq!(game.borrow().calls.len(), before_calls);
    }
    Ok(())
}

#[test]
fn newly_observed_expiry_redacts_response_and_old_context_error_fallback() -> Result<()> {
    let (mut state, memory, game) = setup()?;
    prepare(&mut state, &game)?;
    let later = capture()?.tick() + 2;
    for grant in &mut state.grants {
        if grant.capability == Capability::Query {
            grant.expires_at_tick = Some(GameTick(later - 1));
        }
    }
    let old_context = state.context(true, None)?;
    game.borrow_mut().later_tick = Some(later);
    let before_bytes = memory.bytes.borrow().get_ref().clone();
    let value = observe(&mut state, &game)?;
    assert_eq!(
        value["result"]["error"]["code"],
        ErrorCode::CapabilityDenied.as_str()
    );
    assert_eq!(state.control.high_tick(), later);
    assert!(state.control.selected().is_none());
    assert_no_retained_disclosure(&value)?;

    // The post-call runtime guard receives the original request context. It
    // must not render cached identities at that older, still-authorized tick.
    let fallback: Value = serde_json::from_str(&state.failed(
        "fortress.observe",
        Some(&old_context),
        &error(ErrorCode::CorruptLedger, "late custody failure"),
    ))
    .map_err(|_| exhausted())?;
    assert_no_retained_disclosure(&fallback)?;
    assert_eq!(*memory.bytes.borrow().get_ref(), before_bytes);
    assert!(!game.borrow().calls.contains(&"commit"));
    assert!(
        state
            .historical
            .as_ref()
            .is_some_and(|v| v.pending().is_some())
    );
    Ok(())
}

#[test]
fn revoked_query_recovery_release_redacts_history_and_preserves_journal() -> Result<()> {
    let (mut state, memory, game) = setup()?;
    prepare(&mut state, &game)?;
    let authorized = state.context(true, None)?;
    let before_calls = game.borrow().calls.len();
    let before_bytes = memory.bytes.borrow().get_ref().clone();
    state.grants.retain(|g| g.capability != Capability::Query);
    let revoked = state.context(false, None)?;
    let value: Value =
        serde_json::from_str(&state.close_packet(&revoked, None, true)).map_err(|_| exhausted())?;
    assert_eq!(value["result"]["ok"], true);
    assert_eq!(value["result"]["closed"], true);
    assert_eq!(value["result"]["release_for_recovery"], true);
    assert_no_retained_disclosure(&value)?;
    drop(state);
    assert_eq!(game.borrow().calls.len(), before_calls);
    assert_eq!(*memory.bytes.borrow().get_ref(), before_bytes);
    let journal = BuildJournal::open(memory, &authorized, BuildMode::Offline, None, None)?;
    let mut recovered = BuildSession::<_, Native>::new(journal, &authorized)?;
    let view = recovered.inventory(&authorized)?;
    assert_eq!(view.pending().map(|e| e.plan().key()), Some("golden"));
    assert!(!recovered.has_preparation_connection());
    Ok(())
}

#[test]
fn missing_or_wrong_owner_context_cannot_disclose_error_history() -> Result<()> {
    let (mut state, _, game) = setup()?;
    prepare(&mut state, &game)?;
    let mut wrong_owner = state.context(true, None)?;
    wrong_owner.session_id = SessionId::new(999);
    for context in [None, Some(&wrong_owner)] {
        let value: Value = serde_json::from_str(&state.failed(
            "fortress.doctor",
            context,
            &error(ErrorCode::BudgetExceeded, "response unavailable"),
        ))
        .map_err(|_| exhausted())?;
        assert_no_retained_disclosure(&value)?;
    }
    Ok(())
}

#[test]
fn empty_local_journal_does_not_hide_native_global_preparation_fence() -> Result<()> {
    let (mut state, memory, game) = setup()?;
    game.borrow_mut().other_unresolved = true;
    let observed = observe(&mut state, &game)?;
    assert_eq!(observed["result"]["ok"], true);
    assert_eq!(observed["result"]["observation"]["eligible"], true);
    assert_eq!(
        observed["result"]["observation"]["eligibility_scope"],
        "captured_item_and_target_only"
    );
    assert_eq!(observed["result"]["source_summary"]["unresolved"], true);
    assert_eq!(
        observed["result"]["source_summary"]["prepare_available"],
        false
    );
    assert_eq!(
        observed["agent_turn"]["briefing"]["native_source_summary"]["preparation_blockers"],
        json!(["native_unresolved_effect"])
    );
    assert_eq!(
        observed["agent_turn"]["active_work"]["pending_absence_proven"],
        true
    );
    let before = memory.bytes.borrow().get_ref().clone();
    let refused = call(
        &mut state,
        &game,
        "fortress.plan",
        Ok(Action::Plan {
            key: "golden".into(),
            witness: capture()?.witness(),
        }),
        &mut Guard::default(),
    )?;
    assert_eq!(refused["result"]["ok"], false);
    assert_eq!(refused["result"]["new_local_obligation_created"], false);
    assert_eq!(
        refused["result"]["recovery_target"],
        "journal_owning_unresolved_native_history"
    );
    assert_eq!(
        refused["result"]["source_summary"]["prepare_available"],
        false
    );
    assert_eq!(
        refused["agent_turn"]["active_work"]["pending_absence_proven"],
        true
    );
    assert_eq!(*memory.bytes.borrow().get_ref(), before);
    assert!(!game.borrow().calls.contains(&"prepare"));
    Ok(())
}
