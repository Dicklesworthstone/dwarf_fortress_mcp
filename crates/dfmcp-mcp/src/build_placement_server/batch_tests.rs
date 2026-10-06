#![cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]

use super::*;
use dfmcp_adapter::build_placement::journal::BuildState;
use std::cell::Cell;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);
struct Directory(PathBuf);
impl Directory {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-batch-mcp-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).map_err(storage_error)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).map_err(storage_error)?;
        Ok(Self(path))
    }
    fn parent(&self) -> PathBuf {
        self.0.join("batch.journal")
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn storage_error(_: io::Error) -> dfmcp_core::DfmcpError {
    error(ErrorCode::CorruptLedger, "batch test storage failed")
}

/// Keep the existing Memory implementation and record only synchronized bytes.
/// Fault guards inspect this independent snapshot, never a partially written
/// frame or the active journal's seek cursor.
#[derive(Clone, Default)]
struct TrackedMemory {
    memory: Memory,
    synced: Rc<RefCell<Vec<u8>>>,
}
impl Read for TrackedMemory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.memory.read(out)
    }
}
impl Write for TrackedMemory {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.memory.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.memory.flush()
    }
}
impl Seek for TrackedMemory {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.memory.seek(position)
    }
}
impl EffectJournalStorage for TrackedMemory {
    fn sync(&mut self) -> io::Result<()> {
        self.memory.sync()?;
        *self.synced.borrow_mut() = self.memory.bytes.borrow().get_ref().clone();
        Ok(())
    }
    fn truncate(&mut self, length: u64) -> io::Result<()> {
        self.memory.truncate(length)
    }
    fn validate_identity(&self) -> io::Result<()> {
        self.memory.validate_identity()
    }
}

fn rekey_record(plan: &BuildPlan, name: &str) -> Result<BuildRecord> {
    // Tests exercise the exact first captured selection. Preserve the native
    // corpus's complete post-state/insertion rather than inventing game effects.
    assert_eq!(plan.before(), &capture()?);
    let original = fixture(name)?;
    let suffix = 8 + 2 + "golden".len() + 2 + capture()?.canonical_bytes().len() + 32 + 16;
    let mut bytes = b"DFMBR019".to_vec();
    for field in [plan.key().as_bytes(), plan.before().canonical_bytes()] {
        bytes.extend_from_slice(&(field.len() as u16).to_be_bytes());
        bytes.extend_from_slice(field);
    }
    bytes.extend_from_slice(plan.digest().as_bytes());
    bytes.extend_from_slice(plan.token());
    bytes.extend_from_slice(
        original
            .get(suffix..original.len() - 32)
            .ok_or_else(exhausted)?,
    );
    let mut digest_bytes = b"dfmcp-build-receipt/1\0".to_vec();
    digest_bytes.extend_from_slice(&bytes);
    bytes.extend_from_slice(Digest32::of_bytes(&digest_bytes).as_bytes());
    BuildRecord::decode(&bytes)
}

struct BatchNative {
    native: Native,
    parent: PathBuf,
    lose_parent_after_commit: Rc<Cell<bool>>,
}
impl BuildSource for BatchNative {
    fn binding(&self) -> &BuildBinding {
        self.native.binding()
    }
    fn native_summary(&self) -> BuildNativeSummary {
        self.native.native_summary()
    }
    fn fence(&mut self) {
        self.native.fence();
    }
    fn observe(
        &mut self,
        selection: BuildSelection,
        c: &OperationContext,
        d: Duration,
    ) -> Result<BuildCapture> {
        self.native.observe(selection, c, d)
    }
    fn prepare(
        &mut self,
        plan: &BuildPlan,
        _: &OperationContext,
        _: Duration,
    ) -> Result<BuildPreparation> {
        assert!(!self.native.fenced);
        let record = rekey_record(plan, "prepared")?;
        self.native.permit = true;
        let mut game = self.native.game.borrow_mut();
        game.calls.push("prepare");
        game.record = Some(record.clone());
        self.native.summary = game_summary(&game)?;
        BuildPreparation::new(record, false)
    }
    fn commit(
        &mut self,
        dispatch: BuildDispatch<'_>,
        _: &OperationContext,
        _: Duration,
    ) -> Result<BuildRecord> {
        assert!(!self.native.fenced && self.native.permit);
        assert_ne!(dispatch.journal_head(), Digest32::ZERO);
        self.native.permit = false;
        let record = rekey_record(
            dispatch.plan(),
            if self.native.game.borrow().indeterminate {
                "indeterminate"
            } else {
                "placed"
            },
        )?;
        let mut game = self.native.game.borrow_mut();
        game.calls.push("commit");
        game.record = Some(record.clone());
        self.native.summary = game_summary(&game)?;
        if self.lose_parent_after_commit.get() {
            fs::remove_file(&self.parent).map_err(storage_error)?;
        }
        if game.lost_commit {
            Err(error(
                ErrorCode::AdapterUnavailable,
                "lost batch commit reply",
            ))
        } else {
            Ok(record)
        }
    }
    fn query(
        &mut self,
        plan: &BuildPlan,
        c: &OperationContext,
        d: Duration,
    ) -> Result<Option<BuildRecord>> {
        self.native.query(plan, c, d)
    }
    fn cancel(
        &mut self,
        plan: &BuildPlan,
        _: &OperationContext,
        _: Duration,
    ) -> Result<BuildRecord> {
        assert!(!self.native.fenced);
        self.native.permit = false;
        let record = rekey_record(plan, "cancelled")?;
        let mut game = self.native.game.borrow_mut();
        game.calls.push("cancel");
        game.record = Some(record.clone());
        self.native.summary = game_summary(&game)?;
        Ok(record)
    }
}

type BatchState = State<TrackedMemory, BatchNative>;
struct Harness {
    state: BatchState,
    memory: TrackedMemory,
    game: Rc<RefCell<Game>>,
    lose_parent_after_commit: Rc<Cell<bool>>,
    directory: Directory,
}
fn parent_plan(second_x: u32) -> Result<FurniturePlan> {
    FurniturePlan::decode(
        json!({"schema":"dfmcp.furniture-plan/1","steps":[
            {"name":"bed","kind":"bed","item":42,"target":[15,15,2]},
            {"name":"chair","kind":"chair","item":43,"target":[second_x,15,2],"after":["bed"]}
        ]})
        .to_string()
        .as_bytes(),
    )
}
fn batch_setup() -> Result<Harness> {
    let directory = Directory::new()?;
    let mut config = config()?;
    config.batch_path = Some(directory.parent());
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
    let memory = TrackedMemory::default();
    let journal = BuildJournal::open(
        memory.clone(),
        &c,
        BuildMode::Control,
        Some(binding.clone()),
        Some([7; 32]),
    )?;
    let mut state = State::new(BuildSession::new(journal, &c)?, &c, &config)?;
    let view = state.control.inventory(&c)?;
    let definition = BatchDefinition::new(parent_plan(18)?, binding, view.journal_id)?;
    state.batch = Some(open_private_batch(
        &directory.parent(),
        &c,
        BuildMode::Control,
        Some(definition),
    )?);
    state.verify_batch(&c, &view)?;
    Ok(Harness {
        state,
        memory,
        game: Rc::new(RefCell::new(Game::default())),
        lose_parent_after_commit: Rc::new(Cell::new(false)),
        directory,
    })
}
fn batch_call<G: BuildGuard>(
    h: &mut Harness,
    op: &str,
    action: Action,
    guard: &mut G,
) -> Result<Value> {
    let c = h.state.context(true, None)?;
    let game = h.game.clone();
    let parent = h.directory.parent();
    let lose_parent_after_commit = h.lose_parent_after_commit.clone();
    let raw = run_action(
        &mut h.state,
        c,
        op,
        Ok(action),
        Instant::now(),
        guard,
        move |_, _, binding, _, _| {
            game.borrow_mut().factories += 1;
            let summary = game_summary(&game.borrow())?;
            Ok(BatchNative {
                native: Native {
                    binding: binding.clone(),
                    game,
                    fenced: false,
                    permit: false,
                    summary,
                },
                parent,
                lose_parent_after_commit,
            })
        },
    );
    assert!(raw.len() as u64 <= OUTPUT_BYTES);
    serde_json::from_str(&raw)
        .map_err(|_| error(ErrorCode::InvalidRequest, "invalid batch MCP result"))
}
fn original_plan(h: &Harness) -> Result<BuildPlan> {
    let parent = h.state.batch.as_ref().ok_or_else(exhausted)?;
    let step = parent
        .definition()
        .plan()
        .step("bed")
        .ok_or_else(exhausted)?;
    BuildPlan::new(&parent.definition().key(step), capture()?)
}
fn batch_prepare(h: &mut Harness) -> Result<Digest32> {
    let observed = batch_call(
        h,
        "fortress.observe",
        Action::ObserveNext,
        &mut Guard::default(),
    )?;
    assert_eq!(observed["result"]["ok"], true, "{observed}");
    assert_eq!(observed["result"]["batch"]["total"], 2);
    let plan = original_plan(h)?;
    let prepared = batch_call(
        h,
        "fortress.plan",
        Action::Plan {
            key: plan.key().to_owned(),
            witness: plan.before().witness(),
        },
        &mut Guard::default(),
    )?;
    assert_eq!(prepared["result"]["ok"], true, "{prepared}");
    assert_eq!(prepared["result"]["batch"]["pending_step"], "bed");
    digest(
        prepared["result"]["review_seal"]
            .as_str()
            .ok_or_else(exhausted)?,
    )
}
fn batch_commit<G: BuildGuard>(h: &mut Harness, seal: Digest32, guard: &mut G) -> Result<Value> {
    let plan = original_plan(h)?;
    batch_call(
        h,
        "fortress.commit",
        Action::Commit {
            key: plan.key().to_owned(),
            plan: plan.digest(),
            seal,
        },
        guard,
    )
}
fn pending_key(value: &Value) -> &Value {
    &value["agent_turn"]["active_work"]["pending_plans"][0]["idempotency_key"]
}

#[test]
fn batch_native_rekeying_preserves_every_unchanged_reference_record() -> Result<()> {
    for name in [
        "prepared",
        "placed",
        "expired",
        "cancelled",
        "indeterminate",
    ] {
        assert_eq!(
            rekey_record(&plan()?, name)?.canonical_bytes(),
            fixture(name)?
        );
    }
    Ok(())
}

#[test]
fn batch_review_binds_complete_parent_and_exact_prepared_journal_head() -> Result<()> {
    let mut h = batch_setup()?;
    let seal = batch_prepare(&mut h)?;
    let plan = original_plan(&h)?;
    let c = h.state.context(true, None)?;
    let view = h.state.control.inventory(&c)?;
    let parent = h.state.batch.as_ref().ok_or_else(exhausted)?;
    let policy = h.state.policy.seal(&plan);
    assert_eq!(seal, batch::seal(parent, &plan, view.head, policy));
    assert_ne!(
        seal,
        batch::seal(
            parent,
            &plan,
            Digest32::of_bytes(b"another prepared head"),
            policy
        )
    );
    let changed = BatchDefinition::new(parent_plan(19)?, h.state.binding.clone(), view.journal_id)?;
    let other_directory = Directory::new()?;
    let other_parent = open_private_batch(
        &other_directory.parent(),
        &c,
        BuildMode::Control,
        Some(changed),
    )?;
    assert_ne!(seal, batch::seal(&other_parent, &plan, view.head, policy));
    let review = h.state.review.as_mut().ok_or_else(exhausted)?;
    review.head = Digest32::of_bytes(b"substituted prepared head");
    let denied = batch_commit(&mut h, seal, &mut Guard::default())?;
    assert_eq!(denied["result"]["ok"], false);
    assert_eq!(
        denied["result"]["error"]["code"],
        ErrorCode::StaleAnchor.as_str()
    );
    assert!(!h.game.borrow().calls.contains(&"commit"));
    assert_eq!(pending_key(&denied), plan.key());
    Ok(())
}

#[test]
fn batch_substituted_selection_or_step_key_never_prepares_native_work() -> Result<()> {
    let mut h = batch_setup()?;
    let wrong = BuildSelection::new(BuildKind::Chair, 43, [18, 15, 2])?;
    let denied = batch_call(
        &mut h,
        "fortress.observe",
        Action::Observe(wrong),
        &mut Guard::default(),
    )?;
    assert_eq!(denied["result"]["ok"], false);
    assert_eq!(h.game.borrow().factories, 0);
    let wrong_keys = {
        let parent = h.state.batch.as_ref().ok_or_else(exhausted)?;
        let later = parent
            .definition()
            .plan()
            .step("chair")
            .ok_or_else(exhausted)?;
        ["unrelated-key".to_owned(), parent.definition().key(later)]
    };
    for key in wrong_keys {
        let observed = batch_call(
            &mut h,
            "fortress.observe",
            Action::ObserveNext,
            &mut Guard::default(),
        )?;
        assert_eq!(observed["result"]["ok"], true);
        let denied = batch_call(
            &mut h,
            "fortress.plan",
            Action::Plan {
                key,
                witness: capture()?.witness(),
            },
            &mut Guard::default(),
        )?;
        assert_eq!(denied["result"]["ok"], false);
        assert_eq!(denied["result"]["batch"]["inventory_verified"], true);
        assert_eq!(
            denied["result"]["batch"]["steps"].as_array().map(Vec::len),
            Some(2)
        );
        assert_eq!(denied["result"]["batch"]["pending_step"], Value::Null);
        assert!(!h.game.borrow().calls.contains(&"prepare"));
    }
    let c = h.state.context(false, None)?;
    assert_eq!(h.state.control.inventory(&c)?.total_records(), 0);
    Ok(())
}

#[test]
fn permanent_batch_stop_blocks_observation_and_owned_preparation_but_keeps_query_retirement()
-> Result<()> {
    let mut ready = batch_setup()?;
    let stopped = batch_call(
        &mut ready,
        "fortress.cancel",
        Action::StopBatch,
        &mut Guard::default(),
    )?;
    assert_eq!(stopped["result"]["ok"], true);
    assert_eq!(stopped["result"]["batch"]["stopped"], true);
    assert_eq!(stopped["result"]["batch"]["next"], Value::Null);
    let denied = batch_call(
        &mut ready,
        "fortress.observe",
        Action::ObserveNext,
        &mut Guard::default(),
    )?;
    assert_eq!(denied["result"]["ok"], false);
    assert_eq!(ready.game.borrow().factories, 0);

    let mut prepared = batch_setup()?;
    let seal = batch_prepare(&mut prepared)?;
    let plan = original_plan(&prepared)?;
    let c = prepared.state.context(false, None)?;
    prepared
        .state
        .batch
        .as_mut()
        .ok_or_else(exhausted)?
        .stop(&c)?;
    // Preserve the original local connection/review deliberately: the durable
    // parent stop itself must prevent dispatch, independent of local abandon.
    assert_eq!(prepared.state.seal(), Some(seal));
    let denied = batch_commit(&mut prepared, seal, &mut Guard::default())?;
    assert_eq!(denied["result"]["ok"], false);
    assert!(!prepared.game.borrow().calls.contains(&"commit"));
    prepared
        .state
        .grants
        .retain(|grant| grant.capability == Capability::Query);
    let retired = batch_call(
        &mut prepared,
        "fortress.cancel",
        Action::Recover(plan.key().to_owned(), plan.digest(), true),
        &mut Guard::default(),
    )?;
    assert_eq!(retired["result"]["ok"], true, "{retired}");
    assert_eq!(
        retired["result"]["effect"]["summary"]["native_phase"],
        "cancelled"
    );
    assert_eq!(retired["result"]["batch"]["stopped"], true);
    assert!(!prepared.game.borrow().calls.contains(&"commit"));
    Ok(())
}

struct LoseParentAfterDispatchSync {
    path: PathBuf,
    synced: Rc<RefCell<Vec<u8>>>,
    replace: bool,
    observed_dispatch_sync: bool,
}
impl BuildGuard for LoseParentAfterDispatchSync {
    fn check(
        &mut self,
        stage: BuildStage,
        binding: &BuildBinding,
        plan: Option<&BuildPlan>,
        _: BuildSelection,
        context: &OperationContext,
    ) -> Result<()> {
        if stage != BuildStage::Commit || self.observed_dispatch_sync {
            return Ok(());
        }
        let plan = plan.ok_or_else(exhausted)?;
        let memory = Memory {
            bytes: Rc::new(RefCell::new(Cursor::new(self.synced.borrow().clone()))),
            corrupt: Rc::new(RefCell::new(false)),
        };
        let mut journal = BuildJournal::open(
            memory,
            context,
            BuildMode::Offline,
            Some(binding.clone()),
            None,
        )?;
        let entry = journal.get(plan.key(), plan.digest(), context)?;
        assert_eq!(entry.state(), BuildState::DispatchStarted);
        assert!(entry.dispatch_started());
        self.observed_dispatch_sync = true;
        if self.replace {
            let bytes = fs::read(&self.path).map_err(storage_error)?;
            fs::rename(&self.path, self.path.with_extension("original")).map_err(storage_error)?;
            fs::write(&self.path, bytes).map_err(storage_error)?;
            fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600))
                .map_err(storage_error)?;
        } else {
            fs::remove_file(&self.path).map_err(storage_error)?;
        }
        Ok(())
    }
}

#[test]
fn parent_loss_at_final_synced_dispatch_guard_prevents_native_writer_and_preserves_original_key()
-> Result<()> {
    for replace in [false, true] {
        let mut h = batch_setup()?;
        let seal = batch_prepare(&mut h)?;
        let plan = original_plan(&h)?;
        let mut guard = LoseParentAfterDispatchSync {
            path: h.directory.parent(),
            synced: h.memory.synced.clone(),
            replace,
            observed_dispatch_sync: false,
        };
        let denied = batch_commit(&mut h, seal, &mut guard)?;
        assert!(guard.observed_dispatch_sync);
        assert_eq!(denied["result"]["ok"], false);
        assert_eq!(denied["result"]["batch_inventory_unverified"], true);
        assert_eq!(
            denied["agent_turn"]["active_work"]["inventory_verified"],
            false
        );
        assert_eq!(
            denied["agent_turn"]["active_work"]["pending_absence_proven"],
            false
        );
        assert_eq!(denied["result"]["batch"]["inventory_verified"], false);
        assert_eq!(denied["result"]["batch"]["next"], Value::Null);
        assert_eq!(pending_key(&denied), plan.key());
        assert!(!h.game.borrow().calls.contains(&"commit"));
        let c = h.state.context(false, None)?;
        let entry = h.state.control.get(plan.key(), plan.digest(), &c)?;
        assert!(entry.dispatch_started());
        assert_eq!(entry.plan(), &plan);
        assert!(h.state.seal().is_none());
        h.state
            .grants
            .retain(|grant| grant.capability == Capability::Query);
        let retired = batch_call(
            &mut h,
            "fortress.cancel",
            Action::Recover(plan.key().to_owned(), plan.digest(), true),
            &mut Guard::default(),
        )?;
        assert_eq!(retired["result"]["ok"], true, "{retired}");
        assert_eq!(
            retired["result"]["effect"]["summary"]["native_phase"],
            "cancelled"
        );
        assert_eq!(retired["result"]["batch"]["inventory_verified"], false);
        assert!(!h.game.borrow().calls.contains(&"commit"));
    }
    Ok(())
}

#[test]
fn parent_loss_after_native_writer_keeps_effect_indeterminate_until_original_query_recovers()
-> Result<()> {
    let mut h = batch_setup()?;
    let seal = batch_prepare(&mut h)?;
    let plan = original_plan(&h)?;
    h.lose_parent_after_commit.set(true);
    let uncertain = batch_commit(&mut h, seal, &mut Guard::default())?;
    assert_eq!(uncertain["result"]["ok"], false);
    assert_eq!(uncertain["result"]["effect_may_have_occurred"], true);
    assert_eq!(uncertain["result"]["batch_inventory_unverified"], true);
    assert_eq!(
        uncertain["agent_turn"]["active_work"]["inventory_verified"],
        false
    );
    assert_eq!(
        uncertain["agent_turn"]["active_work"]["pending_absence_proven"],
        false
    );
    assert_eq!(pending_key(&uncertain), plan.key());
    assert_eq!(
        h.game
            .borrow()
            .calls
            .iter()
            .filter(|&&call| call == "commit")
            .count(),
        1
    );
    assert!(h.state.seal().is_none());
    let denied = batch_commit(&mut h, seal, &mut Guard::default())?;
    assert_eq!(denied["result"]["ok"], false);
    h.state
        .grants
        .retain(|grant| grant.capability == Capability::Query);
    let recovered = batch_call(
        &mut h,
        "fortress.wait",
        Action::Recover(plan.key().to_owned(), plan.digest(), false),
        &mut Guard::default(),
    )?;
    assert_eq!(recovered["result"]["ok"], true, "{recovered}");
    assert_eq!(
        recovered["result"]["effect"]["summary"]["native_phase"],
        "placed"
    );
    assert_eq!(recovered["result"]["batch"]["inventory_verified"], false);
    assert_eq!(
        recovered["agent_turn"]["active_work"]["inventory_verified"],
        false
    );
    assert_eq!(
        recovered["agent_turn"]["active_work"]["pending_absence_proven"],
        false
    );
    assert_eq!(
        recovered["agent_turn"]["active_work"]["original_build_journal_inventory_verified"],
        true
    );
    assert_eq!(
        recovered["result"]["batch"]["construction_completion_proven"],
        false
    );
    assert_eq!(recovered["result"]["batch"]["next"], Value::Null);
    assert_eq!(
        h.game
            .borrow()
            .calls
            .iter()
            .filter(|&&call| call == "commit")
            .count(),
        1
    );
    let c = h.state.context(false, None)?;
    let retained = h.state.control.get(plan.key(), plan.digest(), &c)?;
    assert_eq!(
        retained.native().map(|record| record.canonical_bytes()),
        Some(rekey_record(&plan, "placed")?.canonical_bytes())
    );
    Ok(())
}

#[test]
fn dense_complete_plan_reserves_future_rows_and_refuses_oversized_response() -> Result<()> {
    let names = (0..32)
        .map(|i| format!("s{i:02}{}", "x".repeat(45)))
        .collect::<Vec<_>>();
    let mut rows = names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            json!({
                "name":name,"kind":"bed","item":2_147_483_600u32 + i as u32,
                "target":[1 + i % 16, 1 + i / 16, 2],"after":[]
            })
        })
        .collect::<Vec<_>>();
    let mut document = json!({"schema":"dfmcp.furniture-plan/1","steps":rows}).to_string();
    'edges: for index in 1..rows.len() {
        for predecessor in names.iter().take(index) {
            rows[index]["after"]
                .as_array_mut()
                .ok_or_else(exhausted)?
                .push(json!(predecessor));
            let candidate = json!({"schema":"dfmcp.furniture-plan/1","steps":rows}).to_string();
            if candidate.len() > dfmcp_adapter::furniture_batch::MAX_PLAN_BYTES {
                break 'edges;
            }
            document = candidate;
        }
    }
    let plan = FurniturePlan::decode(document.as_bytes())?;
    assert_eq!(plan.steps().len(), 32);
    assert!(plan.canonical_bytes().len() > dfmcp_adapter::furniture_batch::MAX_PLAN_BYTES - 64);
    assert_eq!(
        batch::reserve_output(&plan).err().map(|error| error.code),
        Some(ErrorCode::BudgetExceeded)
    );
    assert!(batch::reserve_output(&parent_plan(18)?).is_ok());
    Ok(())
}
