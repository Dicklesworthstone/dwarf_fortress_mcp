//! Whole handler tests with independent native and parent custody. The mock
//! tracks exact sparse targets, shared blocks and native sequence across regions.
use super::*;
use dfmcp_adapter::dig_blueprint::BlueprintPart;
use dfmcp_adapter::dig_control_policy::DigCheckpointPolicy;
use dfmcp_adapter::dig_designation::journal::{DigGuard, DigJournal, DigStage};
use dfmcp_adapter::dig_designation::rpc::{DigManifest, DigPreparation};
use dfmcp_adapter::dig_designation::{DigEffect, DigPhase};
use dfmcp_core::{MapCoord, MapCuboid};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::rc::Rc;

const TICK: u64 = 12345;

fn field(value: &Value) -> Result<&str> {
    value.as_str().ok_or_else(|| {
        error(
            ErrorCode::InternalInvariantViolation,
            "missing test result field",
        )
    })
}
fn success(value: &Value) {
    assert_eq!(value["result"]["ok"], true, "{value}");
}
fn text(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&(value.len() as u16).to_be_bytes());
    out.extend_from_slice(value.as_bytes());
}
fn hash(domain: &[u8], bytes: &[u8]) -> Digest32 {
    let mut raw = domain.to_vec();
    raw.push(0);
    raw.extend_from_slice(bytes);
    Digest32::of_bytes(&raw)
}
fn sparse() -> Result<Layout> {
    Layout::new(vec![
        BlueprintPart {
            origin: [2, 2, 2],
            size: [9, 1, 1],
        },
        BlueprintPart {
            origin: [2, 4, 2],
            size: [2, 1, 1],
        },
    ])
}

#[derive(Clone, Default)]
struct Memory {
    data: Rc<RefCell<Cursor<Vec<u8>>>>,
    writes: Rc<RefCell<usize>>,
    syncs: Rc<RefCell<usize>>,
}
impl Memory {
    fn snapshot_bytes(&self) -> Vec<u8> {
        self.data.borrow().get_ref().clone()
    }
}
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.data.borrow_mut().read(out)
    }
}
impl Write for Memory {
    fn write(&mut self, out: &[u8]) -> io::Result<usize> {
        *self.writes.borrow_mut() += 1;
        self.data.borrow_mut().write(out)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Seek for Memory {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.data.borrow_mut().seek(position)
    }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        *self.syncs.borrow_mut() += 1;
        Ok(())
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("test must never repair retained evidence"))
    }
}
fn last_state(raw: &[u8]) -> Option<u8> {
    let length = u16::from_be_bytes(raw.get(8..10)?.try_into().ok()?) as usize;
    let mut offset = 74 + length;
    let mut state = None;
    while offset < raw.len() {
        let length =
            u32::from_be_bytes(raw.get(offset + 8..offset + 12)?.try_into().ok()?) as usize;
        state = raw.get(offset + 52).copied();
        offset += 92 + length;
    }
    state
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Reply {
    #[default]
    Designated,
    Lost,
    Unknown,
}
struct Game {
    tick: u64,
    sequence: u64,
    targets: BTreeSet<[u32; 3]>,
    records: BTreeMap<String, DigEffect>,
    calls: Vec<DigStage>,
    factories: usize,
    reply: Reply,
}
impl Default for Game {
    fn default() -> Self {
        Self {
            tick: TICK,
            sequence: 0,
            targets: BTreeSet::new(),
            records: BTreeMap::new(),
            calls: vec![],
            factories: 0,
            reply: Reply::Designated,
        }
    }
}
impl Game {
    fn count(&self, stage: DigStage) -> usize {
        self.calls.iter().filter(|call| **call == stage).count()
    }
    fn apply(&mut self, region: DigRegion) {
        let [x, y, z, width, height] = region.coordinates();
        for py in y..y + height {
            for px in x..x + width {
                assert!(
                    self.targets.insert([px, py, z]),
                    "duplicate native designation"
                );
            }
        }
        self.sequence += 1;
    }
    fn capture(&self, region: DigRegion) -> Result<DigObservation> {
        let mut raw = b"DFMDG016".to_vec();
        for value in [7u64, self.sequence, self.tick] {
            raw.extend_from_slice(&value.to_be_bytes());
        }
        for value in [1u32, 128, 128, 8] {
            raw.extend_from_slice(&value.to_be_bytes());
        }
        let [x, y, z, width, height] = region.coordinates();
        for value in region.coordinates() {
            raw.extend_from_slice(&value.to_be_bytes());
        }
        raw.push(1);
        text(&mut raw, "region1");
        raw.extend_from_slice(&(region.halo_count() as u16).to_be_bytes());
        for pz in z - 1..=z + 1 {
            for py in y - 1..=y + height {
                for px in x - 1..=x + width {
                    let designated = self.targets.contains(&[px, py, pz]);
                    let dirty = self
                        .targets
                        .iter()
                        .any(|q| q[2] == pz && q[0] / 16 == px / 16 && q[1] / 16 == py / 16);
                    raw.push(2);
                    for value in [200u32, 0, 0, if designated { 4000 } else { 0 }, 0, 0] {
                        raw.extend_from_slice(&value.to_be_bytes());
                    }
                    raw.extend_from_slice(&10000u16.to_be_bytes());
                    raw.extend_from_slice(&10000u16.to_be_bytes());
                    raw.extend_from_slice(&[
                        u8::from(designated),
                        0,
                        1 | if dirty { 16 } else { 0 },
                    ]);
                }
            }
        }
        DigObservation::decode(&raw)
    }
}
fn native(plan: &DigPlan, phase: DigPhase) -> Result<DigEffect> {
    let before = plan.before();
    let mut raw = b"DFMDGE16".to_vec();
    for value in [before.generation(), before.sequence(), before.tick()] {
        raw.extend_from_slice(&value.to_be_bytes());
    }
    for value in before.region().coordinates() {
        raw.extend_from_slice(&value.to_be_bytes());
    }
    raw.push(u8::from(plan.allow_hidden_neighbors()));
    raw.extend_from_slice(before.witness().as_bytes());
    raw.extend_from_slice(plan.digest().as_bytes());
    raw.extend_from_slice(plan.token());
    raw.push(match phase {
        DigPhase::Prepared => 0,
        DigPhase::Unknown => 1,
        DigPhase::Designated => 2,
        DigPhase::Refused => 4,
    });
    raw.push(if phase == DigPhase::Refused { 2 } else { 0 });
    raw.push(u8::from(phase == DigPhase::Designated));
    raw.extend_from_slice(
        &(if phase == DigPhase::Designated {
            before.region().target_count()
        } else {
            0
        })
        .to_be_bytes(),
    );
    let after = if phase == DigPhase::Designated {
        let mut capture = before.canonical_bytes().to_vec();
        capture[16..24].copy_from_slice(&(before.sequence() + 1).to_be_bytes());
        let start = capture.len() - before.region().halo_count() * 32;
        let [x, y, z, width, height] = before.region().coordinates();
        for (index, (point, _)) in before.tiles().enumerate() {
            let at = start + 32 * index;
            if point[2] == z
                && (x..x + width).contains(&point[0])
                && (y..y + height).contains(&point[1])
            {
                capture[at + 13..at + 17].copy_from_slice(&4000u32.to_be_bytes());
                capture[at + 29] = 1;
            }
            if before.region().write_area().contains(MapCoord::new(
                point[0] as i32,
                point[1] as i32,
                point[2] as i32,
            )) {
                capture[at + 17..at + 21].fill(0);
                capture[at + 31] |= 16;
            }
        }
        Digest32::of_bytes(&capture)
    } else {
        Digest32::ZERO
    };
    raw.extend_from_slice(after.as_bytes());
    let mut proof = before.generation().to_be_bytes().to_vec();
    text(&mut proof, plan.key());
    proof.extend_from_slice(plan.digest().as_bytes());
    proof.extend_from_slice(plan.token());
    proof.extend_from_slice(&raw[133..172]);
    raw.extend_from_slice(
        if phase.terminal() {
            hash(b"dfmcp-dig-designation-receipt/1", &proof)
        } else {
            Digest32::ZERO
        }
        .as_bytes(),
    );
    text(&mut raw, plan.key());
    DigEffect::decode(&raw, plan)
}
struct Source {
    game: Rc<RefCell<Game>>,
    binding: DigBinding,
    region: DigRegion,
    permit: Option<String>,
}
impl DigSource for Source {
    fn manifest(&self) -> &DigManifest {
        self.binding.manifest()
    }
    fn endpoint(&self) -> Option<std::net::SocketAddr> {
        Some(self.binding.endpoint())
    }
    fn observe(&mut self, region: DigRegion, _: &OperationContext) -> Result<DigObservation> {
        assert_eq!(region, self.region);
        let mut game = self.game.borrow_mut();
        game.calls.push(DigStage::Observe);
        game.capture(region)
    }
    fn prepare(&mut self, plan: &DigPlan, _: &OperationContext) -> Result<DigPreparation> {
        assert_eq!(plan.before().region(), self.region);
        let mut game = self.game.borrow_mut();
        game.calls.push(DigStage::Prepare);
        assert_eq!(*plan.before(), game.capture(self.region)?);
        assert!(
            !game.records.contains_key(plan.key()),
            "native key was re-prepared"
        );
        let effect = native(plan, DigPhase::Prepared)?;
        let prepared = DigPreparation::decode(effect.canonical_bytes(), false, plan)?;
        game.records.insert(plan.key().to_owned(), effect);
        self.permit = Some(plan.key().to_owned());
        Ok(prepared)
    }
    fn commit(&mut self, plan: &DigPlan, _: &OperationContext) -> Result<DigEffect> {
        assert_eq!(self.permit.take().as_deref(), Some(plan.key()));
        let mut game = self.game.borrow_mut();
        game.calls.push(DigStage::Commit);
        assert_eq!(*plan.before(), game.capture(self.region)?);
        game.apply(plan.before().region());
        let phase = if game.reply == Reply::Unknown {
            DigPhase::Unknown
        } else {
            DigPhase::Designated
        };
        let effect = native(plan, phase)?;
        if phase == DigPhase::Designated {
            assert_eq!(
                effect.after_witness(),
                Some(game.capture(self.region)?.witness())
            );
        }
        game.records.insert(plan.key().to_owned(), effect.clone());
        if game.reply == Reply::Lost {
            Err(error(
                ErrorCode::AdapterRejected,
                "native reply lost after effect",
            ))
        } else {
            Ok(effect)
        }
    }
    fn query(&mut self, plan: &DigPlan, _: &OperationContext) -> Result<Option<DigEffect>> {
        let mut game = self.game.borrow_mut();
        game.calls.push(DigStage::Query);
        Ok(game.records.get(plan.key()).cloned())
    }
    fn cancel(&mut self, plan: &DigPlan, _: &OperationContext) -> Result<DigEffect> {
        let mut game = self.game.borrow_mut();
        game.calls.push(DigStage::Cancel);
        if let Some(effect) = game.records.get(plan.key())
            && (effect.phase().terminal() || effect.phase() == DigPhase::Unknown)
        {
            return Ok(effect.clone());
        }

        let effect = native(plan, DigPhase::Refused)?;
        game.records.insert(plan.key().to_owned(), effect.clone());
        Ok(effect)
    }
}
struct Runtime {
    native: Memory,
    parent: Memory,
    corrupt_after_dispatch: bool,
}
impl DigGuard for Runtime {
    fn check(&mut self, stage: DigStage, _: &DigPlan, _: &OperationContext) -> Result<()> {
        if stage == DigStage::Commit && self.corrupt_after_dispatch {
            assert_eq!(
                last_state(&self.native.snapshot_bytes()),
                Some(DigState::DispatchStarted as u8)
            );
            self.parent.data.borrow_mut().get_mut()[0] ^= 1;
            self.corrupt_after_dispatch = false;
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
    config: Config,
    layout: Layout,
}
impl Harness {
    fn new(layout: Layout, checkpoint: DigCheckpointPolicy) -> Result<Self> {
        let game = Rc::new(RefCell::new(Game::default()));
        let before = game.borrow().capture(layout.regions()[0])?;
        let config = Config {
            path: "/private/dig/journal".into(),
            folder: before.folder().to_owned(),
            site: before.site(),
            scope: MapCuboid::new(MapCoord::new(0, 0, 0), MapCoord::new(127, 127, 7))?,
            endpoint: "127.0.0.1:5000".parse().map_err(|_| exhausted())?,
            protected: vec![],
            checkpoint,
            monitor: None,
            clock: None,
            blueprint: Some(batch::Config {
                path: "/private/dig/blueprint".into(),
                initialize: true,
                recovery: None,
            }),
        };
        let binding = DigBinding::new(
            config.endpoint,
            DigManifest {
                generation: 7,
                df_version: "test-df".to_owned(),
                dfhack_version: "test-dfhack".to_owned(),
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
        assert!(
            c.grants
                .iter()
                .all(|g| g.scope.map_area == Some(config.scope))
        );
        let native = Memory::default();
        let parent = Memory::default();
        let journal = DigJournal::open(
            native.clone(),
            &c,
            DigMode::Control,
            Some(binding),
            Some([7; 32]),
        )?;
        let session = DigSession::new(journal, &c)?;
        let mut state = State::new(session, &c, &config)?;
        state.blueprint_storage = Some(parent.clone());
        Ok(Self {
            state,
            game,
            c,
            config,
            layout,
            runtime: Runtime {
                native,
                parent,
                corrupt_after_dispatch: false,
            },
        })
    }
    fn run(&mut self, op: &str, action: Result<Action>) -> Result<Value> {
        self.c.request_id = RequestId::new(self.c.request_id.get() + 1);
        self.c.anchor.tick = GameTick(self.c.anchor.tick.get().max(self.state.control.high_tick()));
        let game = self.game.clone();
        let raw = run_action(
            &mut self.state,
            self.c.clone(),
            op,
            action,
            Instant::now(),
            &mut self.runtime,
            move |binding, region, _| {
                game.borrow_mut().factories += 1;
                Ok(Source {
                    game,
                    binding: binding.clone(),
                    region,
                    permit: None,
                })
            },
        );
        assert!(
            raw.len() <= OUTPUT_BYTES as usize,
            "response exceeded complete output reservation"
        );
        serde_json::from_str(&raw).map_err(|_| {
            error(
                ErrorCode::InternalInvariantViolation,
                "invalid handler packet",
            )
        })
    }
    fn publish(&mut self) -> Result<Value> {
        self.run("fortress.plan", Ok(Action::Blueprint(self.layout.clone())))
    }
    fn definition(&self) -> Result<&BatchDefinition> {
        self.state
            .blueprint
            .as_ref()
            .map(BatchStore::definition)
            .ok_or_else(|| {
                error(
                    ErrorCode::InternalInvariantViolation,
                    "missing fixture parent",
                )
            })
    }
    fn observe(&mut self, index: usize) -> Result<Value> {
        self.run(
            "fortress.observe",
            Ok(Action::Observe(self.layout.regions()[index])),
        )
    }
    fn prepare(&mut self, index: usize) -> Result<(DigPlan, Digest32, Value)> {
        if self
            .state
            .control
            .selected()
            .is_none_or(|v| v.region() != self.layout.regions()[index])
        {
            success(&self.observe(index)?);
        }
        let before = self.state.control.selected().cloned().ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "missing fixture selection",
            )
        })?;
        let plan = DigPlan::new(&self.definition()?.key(index)?, false, before)?;
        let result = self.run(
            "fortress.plan",
            Ok(Action::Plan {
                key: plan.key().to_owned(),
                witness: plan.before().witness(),
                hidden: false,
            }),
        )?;
        success(&result);
        assert_eq!(
            result["result"]["plan"]["plan_digest"],
            plan.digest().to_string()
        );
        let seal = digest(field(&result["result"]["review_seal"])?)?;
        Ok((plan, seal, result))
    }
    fn commit(&mut self, plan: &DigPlan, seal: Digest32) -> Result<Value> {
        self.run(
            "fortress.commit",
            Ok(Action::Commit {
                key: plan.key().to_owned(),
                plan: plan.digest(),
                seal,
            }),
        )
    }
    fn query(&mut self, offset: usize, limit: usize) -> Result<Value> {
        self.run(
            "fortress.query",
            Ok(Action::Query(Query::Blueprint {
                blueprint_id: self.definition()?.id().to_string(),
                expected_head: None,
                expected_parent_head: None,
                offset: Some(offset),
                limit: Some(limit),
            })),
        )
    }
    fn reopen(&mut self, mode: DigMode) -> Result<()> {
        self.state.abandon();
        let recovery = self
            .state
            .blueprint
            .as_ref()
            .ok_or_else(|| {
                error(
                    ErrorCode::InternalInvariantViolation,
                    "missing recovery fixture",
                )
            })?
            .recovery();
        let mut config = self.config.clone();
        config.blueprint = Some(batch::Config {
            path: "/private/dig/blueprint".into(),
            initialize: false,
            recovery: Some(recovery),
        });
        // An empty native journal does not retain the original bootstrap clock.
        // Use the actual production floor helper before constructing its owner
        // and lease; bytes never recreate lost grants or a preparation permit.
        let mut c = context(
            SessionId::new(self.c.session_id.get() + 1),
            RequestId::new(1),
            &self.state.binding,
            0,
            self.c.budget,
            mode == DigMode::Control,
        );
        if mode == DigMode::Offline {
            c.grants.retain(|g| g.capability == Capability::Query);
        }
        let mut parent = BatchStore::open(
            self.runtime.parent.clone(),
            &c,
            mode,
            None,
            false,
            Some(recovery),
        )?;
        c = context_with_blueprint_floor(&c, Some(&parent));
        assert_eq!(c.anchor.tick, GameTick(TICK));
        let journal = DigJournal::open(
            self.runtime.native.clone(),
            &c,
            mode,
            Some(self.state.binding.clone()),
            None,
        )?;
        let mut session = DigSession::new(journal, &c)?;
        c.anchor.tick = c.anchor.tick.max(GameTick(session.high_tick()));
        let (view, records) = session.complete_inventory(&c)?;
        batch::verify(&mut parent, &view, &records, &c)?;
        if mode == DigMode::Control {
            let region = self.layout.regions()[0];
            self.game.borrow_mut().factories += 1;
            let mut source = Source {
                game: self.game.clone(),
                binding: self.state.binding.clone(),
                region,
                permit: None,
            };
            let capture = source.observe(region, &c)?;
            c = current_blueprint_context(
                &c,
                parent.definition(),
                &records,
                &source,
                region,
                &capture,
            )?;
            session.retain_read_floor(&capture, &c)?;
            // The current clock is retained, never this source or a selection.
            drop(source);
        }
        let mut state = State::new(session, &c, &config)?;
        state.blueprint_recovery = Some(parent.recovery());
        state.blueprint = Some(parent);
        assert!(state.review.is_none());
        assert!(!state.control.has_preparation_connection());
        self.state = state;
        self.c = c;
        self.config = config;
        Ok(())
    }
}

#[test]
fn blueprint_scoped_authority_and_default_checkpoint_refuse_before_contact() -> Result<()> {
    let mut h = Harness::new(sparse()?, DigCheckpointPolicy::Required)?;
    let refused = h.publish()?;
    assert_eq!(refused["result"]["error"]["code"], "checkpoint_required");
    assert_eq!(h.game.borrow().factories, 0);
    assert!(h.runtime.parent.snapshot_bytes().is_empty());
    let mut h = Harness::new(sparse()?, DigCheckpointPolicy::DisposableFortress)?;
    h.c.grants.retain(|g| g.capability != Capability::Plan);
    assert_eq!(h.publish()?["result"]["ok"], false);
    assert_eq!(h.game.borrow().factories, 0);
    assert!(h.runtime.parent.snapshot_bytes().is_empty());

    // A protected shared block in a later child must stop the complete project
    // before observing or preparing an otherwise admissible first child.
    let layout = Layout::new(vec![
        BlueprintPart {
            origin: [2, 2, 2],
            size: [1, 1, 1],
        },
        BlueprintPart {
            origin: [17, 2, 2],
            size: [1, 1, 1],
        },
    ])?;
    let mut h = Harness::new(layout, DigCheckpointPolicy::DisposableFortress)?;
    h.config.protected = vec![MapCuboid::new(
        MapCoord::new(16, 0, 2),
        MapCoord::new(31, 15, 2),
    )?];
    h.state.policy = DigControlPolicy::new(
        h.state.binding.clone(),
        h.state.policy.journal_id(),
        h.state.id,
        h.state.policy.lease_id(),
        h.config.protected.clone(),
        h.config.checkpoint,
    )?;
    assert_eq!(h.publish()?["result"]["ok"], false);
    assert_eq!(h.game.borrow().factories, 0);
    assert!(h.runtime.parent.snapshot_bytes().is_empty());
    Ok(())
}

#[test]
fn sparse_blueprint_handler_designates_exact_union_one_review_per_region() -> Result<()> {
    let mut h = Harness::new(sparse()?, DigCheckpointPolicy::DisposableFortress)?;
    let published = h.publish()?;
    success(&published);
    assert_eq!(published["result"]["native_preparation_performed"], false);
    assert!(
        published["agent_turn"]["recommendations"]
            .to_string()
            .contains("fortress.plan")
    );
    assert_eq!(h.game.borrow().count(DigStage::Prepare), 0);
    let original_id = h.definition()?.id();
    let original_bytes = h.runtime.parent.snapshot_bytes();
    assert_eq!(h.layout.regions().len(), 3);
    for index in 0..h.layout.regions().len() {
        let (plan, seal, prepared) = h.prepare(index)?;
        assert_eq!(prepared["result"]["blueprint"]["pending_step"], index);
        assert!(
            prepared["agent_turn"]["recommendations"]
                .to_string()
                .contains("fortress.commit")
        );
        let handoff =
            presentation::retained_handoff(&prepared.to_string()).ok_or_else(exhausted)?;
        assert_eq!(handoff["record"]["idempotency_key"], plan.key());
        assert_eq!(handoff["record"]["plan_digest"], plan.digest().to_string());
        assert_eq!(handoff["pending_plans"][0]["idempotency_key"], plan.key());
        assert_eq!(handoff["inventory_verified"], false);
        assert_eq!(handoff["dispatch_authorized"], false);
        assert_eq!(handoff["retry_commit_permitted"], false);
        assert!(!handoff.to_string().contains("review_seal"));
        assert!(!handoff.to_string().contains(&seal.to_string()));
        let calls = h.game.borrow().calls.len();
        let inspected = h.query(0, 8)?;
        success(&inspected);
        assert_eq!(h.game.borrow().calls.len(), calls);
        assert_eq!(h.state.seal(), Some(seal));
        let committed = h.commit(&plan, seal)?;
        success(&committed);
        let handoff =
            presentation::retained_handoff(&committed.to_string()).ok_or_else(exhausted)?;
        assert_eq!(handoff["record"]["idempotency_key"], plan.key());
        assert_eq!(handoff["record"]["plan_digest"], plan.digest().to_string());
        assert_eq!(handoff["record"]["native"]["phase"], "designated");
        assert_eq!(handoff["historical_evidence_only"], true);
        assert_eq!(handoff["dispatch_authorized"], false);
        assert!(!handoff.to_string().contains("review_seal"));
        assert_eq!(
            committed["result"]["effect"]["native"]["phase"],
            "designated"
        );
        assert_eq!(committed["result"]["blueprint"]["designated"], index + 1);
        assert_eq!(h.game.borrow().count(DigStage::Commit), index + 1);
        assert_eq!(h.game.borrow().factories, index + 1);
        assert_eq!(h.definition()?.id(), original_id);
        assert_eq!(h.runtime.parent.snapshot_bytes(), original_bytes);
        let replay = h.commit(&plan, Digest32::ZERO)?;
        success(&replay);
        assert_eq!(replay["result"]["native_calls"], 0);
        assert_eq!(h.game.borrow().count(DigStage::Commit), index + 1);
    }
    let result = h.query(0, 8)?;
    assert_eq!(result["result"]["blueprint"]["status"], "all_designated");
    assert_eq!(result["result"]["blueprint"]["next"], Value::Null);
    assert_eq!(
        result["result"]["blueprint"]["excavation_completion_proven"],
        false
    );
    assert_eq!(
        h.game.borrow().targets,
        h.layout.targets().iter().copied().collect()
    );
    assert!(!h.game.borrow().targets.contains(&[2, 3, 2]));
    Ok(())
}

#[test]
fn blueprint_rejects_substituted_layout_child_region_hidden_policy_and_seal() -> Result<()> {
    let mut h = Harness::new(sparse()?, DigCheckpointPolicy::DisposableFortress)?;
    success(&h.publish()?);
    let original = h.definition()?.id();
    let factories = h.game.borrow().factories;
    assert_eq!(h.observe(1)?["result"]["ok"], false);
    assert_eq!(h.game.borrow().factories, factories);
    for (wrong_key, hidden) in [(true, false), (false, true)] {
        success(&h.observe(0)?);
        let capture = h.state.control.selected().cloned().ok_or_else(exhausted)?;
        let calls = h.game.borrow().calls.len();
        let key = if wrong_key {
            "substitute-original".into()
        } else {
            h.definition()?.key(0)?
        };
        let denied = h.run(
            "fortress.plan",
            Ok(Action::Plan {
                key,
                witness: capture.witness(),
                hidden,
            }),
        )?;
        assert_eq!(denied["result"]["ok"], false);
        assert_eq!(h.game.borrow().calls.len(), calls);
        assert_eq!(
            denied["result"]["blueprint"]["blueprint_id"],
            original.to_string()
        );
    }
    let different = Layout::new(vec![BlueprintPart {
        origin: [2, 2, 2],
        size: [1, 1, 1],
    }])?;
    let denied = h.run("fortress.plan", Ok(Action::Blueprint(different)))?;
    assert_eq!(denied["result"]["ok"], false);
    assert_eq!(h.definition()?.id(), original);
    let (plan, seal, _) = h.prepare(0)?;
    assert_eq!(h.commit(&plan, Digest32::ZERO)?["result"]["ok"], false);
    assert!(h.state.seal().is_none());
    assert_eq!(h.commit(&plan, seal)?["result"]["ok"], false);
    assert_eq!(h.game.borrow().count(DigStage::Commit), 0);
    assert_eq!(h.game.borrow().count(DigStage::Prepare), 1);
    Ok(())
}

#[test]
fn blueprint_lost_reply_blocks_advancement_until_original_key_reconciliation() -> Result<()> {
    let mut h = Harness::new(sparse()?, DigCheckpointPolicy::DisposableFortress)?;
    success(&h.publish()?);
    let (plan, seal, _) = h.prepare(0)?;
    h.game.borrow_mut().reply = Reply::Lost;
    let lost = h.commit(&plan, seal)?;
    assert_eq!(lost["result"]["ok"], false);
    assert_eq!(lost["result"]["effect_may_have_occurred"], true);
    assert_eq!(lost["result"]["blueprint"]["pending_step"], 0);
    assert_eq!(lost["result"]["blueprint"]["next"], Value::Null);
    assert!(
        lost["agent_turn"]["recommendations"]
            .to_string()
            .contains("fortress.explain")
    );
    assert!(
        !lost["agent_turn"]["recommendations"]
            .to_string()
            .contains("fortress.commit")
    );
    let contacts = h.game.borrow().calls.len();
    assert_eq!(h.observe(1)?["result"]["ok"], false);
    assert_eq!(h.commit(&plan, seal)?["result"]["ok"], false);
    assert_eq!(h.game.borrow().calls.len(), contacts);
    let recovered = h.run(
        "fortress.wait",
        Ok(Action::Wait(plan.key().into(), plan.digest())),
    )?;
    success(&recovered);
    assert_eq!(
        recovered["result"]["effect"]["native"]["phase"],
        "designated"
    );
    assert_eq!(recovered["result"]["blueprint"]["next"]["index"], 1);
    assert_eq!(h.game.borrow().count(DigStage::Commit), 1);
    assert_eq!(h.game.borrow().count(DigStage::Query), 1);
    h.game.borrow_mut().reply = Reply::Designated;
    let (next, seal, _) = h.prepare(1)?;
    success(&h.commit(&next, seal)?);
    assert_eq!(h.game.borrow().count(DigStage::Commit), 2);
    Ok(())
}

#[test]
fn blueprint_native_unknown_is_permanent_and_never_unlocks_another_child() -> Result<()> {
    let mut h = Harness::new(sparse()?, DigCheckpointPolicy::DisposableFortress)?;
    success(&h.publish()?);
    let (plan, seal, _) = h.prepare(0)?;
    h.game.borrow_mut().reply = Reply::Unknown;
    let unknown = h.commit(&plan, seal)?;
    success(&unknown);
    assert_eq!(unknown["result"]["blueprint"]["status"], "halted_unknown");
    assert_eq!(unknown["result"]["effect"]["permanent_unknown"], true);
    let calls = h.game.borrow().calls.len();
    for action in [
        Action::Wait(plan.key().into(), plan.digest()),
        Action::Cancel(plan.key().into(), plan.digest()),
    ] {
        success(&h.run("fortress.wait", Ok(action))?);
    }
    assert_eq!(h.game.borrow().calls.len(), calls);
    h.reopen(DigMode::Control)?;
    assert_eq!(h.game.borrow().calls.len(), calls + 1);
    assert_eq!(h.observe(1)?["result"]["ok"], false);
    assert_eq!(h.game.borrow().calls.len(), calls + 1);
    assert_eq!(
        h.query(0, 8)?["result"]["blueprint"]["status"],
        "halted_unknown"
    );
    Ok(())
}

#[test]
fn blueprint_reopening_never_recovers_preparation_permission() -> Result<()> {
    let mut h = Harness::new(sparse()?, DigCheckpointPolicy::DisposableFortress)?;
    success(&h.publish()?);
    let (plan, seal, _) = h.prepare(0)?;
    let before_native = h.runtime.native.snapshot_bytes();
    let before_parent = h.runtime.parent.snapshot_bytes();
    let contacts = h.game.borrow().calls.len();
    let syncs = (
        *h.runtime.native.syncs.borrow(),
        *h.runtime.parent.syncs.borrow(),
    );
    h.reopen(DigMode::Offline)?;
    success(&h.query(0, 8)?);
    assert_eq!(h.commit(&plan, seal)?["result"]["ok"], false);
    assert_eq!(
        h.run(
            "fortress.wait",
            Ok(Action::Wait(plan.key().into(), plan.digest()))
        )?["result"]["ok"],
        false
    );
    assert_eq!(h.game.borrow().calls.len(), contacts);
    assert_eq!(h.runtime.native.snapshot_bytes(), before_native);
    assert_eq!(h.runtime.parent.snapshot_bytes(), before_parent);
    assert_eq!(
        (
            *h.runtime.native.syncs.borrow(),
            *h.runtime.parent.syncs.borrow()
        ),
        syncs
    );
    h.reopen(DigMode::Control)?;
    assert_eq!(h.game.borrow().calls.len(), contacts + 1);
    assert_eq!(h.commit(&plan, seal)?["result"]["ok"], false);
    assert_eq!(h.game.borrow().calls.len(), contacts + 1);
    let cancelled = h.run(
        "fortress.cancel",
        Ok(Action::Cancel(plan.key().into(), plan.digest())),
    )?;
    success(&cancelled);
    assert_eq!(
        cancelled["result"]["blueprint"]["status"],
        "halted_cancelled"
    );
    assert_eq!(h.game.borrow().count(DigStage::Prepare), 1);
    assert_eq!(h.game.borrow().count(DigStage::Commit), 0);
    assert_eq!(h.observe(1)?["result"]["ok"], false);
    Ok(())
}

#[test]
fn blueprint_empty_reopen_restores_clock_floor_and_settled_prefix_allows_new_review() -> Result<()>
{
    let mut h = Harness::new(sparse()?, DigCheckpointPolicy::DisposableFortress)?;
    success(&h.publish()?);
    let parent = h.state.blueprint.as_ref().ok_or_else(exhausted)?;
    let mut low = h.c.clone();
    low.anchor.tick = GameTick(0);
    low.grants
        .iter_mut()
        .for_each(|g| g.expires_at_tick = Some(GameTick(TICK - 1)));
    let floored = context_with_blueprint_floor(&low, Some(parent));
    assert_eq!(floored.grants, low.grants);
    assert!(
        floored
            .authorize(
                Capability::Plan,
                RiskTier::Guarded,
                &[],
                Some(h.config.scope)
            )
            .is_err()
    );
    // The fortress advanced while this owner was closed. The fresh control
    // lease must begin at the verified current clock, not the old bootstrap.
    h.game.borrow_mut().tick = TICK + 2000;
    h.game.borrow_mut().sequence = 10;
    h.reopen(DigMode::Control)?;
    assert_eq!(h.c.anchor.tick, GameTick(TICK + 2000));
    assert_eq!(
        h.state.context(true, None)?.anchor.tick,
        GameTick(TICK + 2000)
    );
    assert!(h.state.control.selected().is_none());
    let region = h.layout.regions()[0];
    let capture = h.game.borrow().capture(region)?;
    let source = Source {
        game: h.game.clone(),
        binding: h.state.binding.clone(),
        region,
        permit: None,
    };
    let mut expired = h.c.clone();
    expired.anchor.tick = GameTick(0);
    expired
        .grants
        .iter_mut()
        .for_each(|g| g.expires_at_tick = Some(GameTick(TICK + 1999)));
    assert!(
        current_blueprint_context(&expired, h.definition()?, &[], &source, region, &capture)
            .is_err()
    );
    let regressed = Game::default().capture(region)?;
    assert!(
        current_blueprint_context(&h.c, h.definition()?, &[], &source, region, &regressed).is_err()
    );
    let mut changed = capture.canonical_bytes().to_vec();
    changed[36..40].copy_from_slice(&96u32.to_be_bytes());
    let changed = DigObservation::decode(&changed)?;
    assert!(
        current_blueprint_context(&h.c, h.definition()?, &[], &source, region, &changed).is_err()
    );
    // A fresh read also advances the session's native sequence floor. Later
    // observations at the same game tick cannot silently rewind that sequence.
    h.game.borrow_mut().sequence = 4;
    assert_eq!(h.observe(0)?["result"]["ok"], false);
    let regressed_sequence = h.game.borrow().capture(region)?;
    let denied = h.run(
        "fortress.plan",
        Ok(Action::Plan {
            key: h.definition()?.key(0)?,
            witness: regressed_sequence.witness(),
            hidden: false,
        }),
    )?;
    assert_eq!(denied["result"]["ok"], false);
    assert_eq!(h.game.borrow().count(DigStage::Prepare), 0);
    assert!(h.state.control.selected().is_none());
    h.game.borrow_mut().sequence = 10;
    let (plan, seal, _) = h.prepare(0)?;
    h.game.borrow_mut().reply = Reply::Lost;
    assert_eq!(h.commit(&plan, seal)?["result"]["ok"], false);
    h.reopen(DigMode::Recover)?;
    let contacts = h.game.borrow().calls.len();
    assert_eq!(h.commit(&plan, seal)?["result"]["ok"], false);
    assert_eq!(h.game.borrow().calls.len(), contacts);
    let recovered = h.run(
        "fortress.wait",
        Ok(Action::Wait(plan.key().into(), plan.digest())),
    )?;
    success(&recovered);
    assert_eq!(recovered["result"]["blueprint"]["next"]["index"], 1);
    assert_eq!(h.observe(1)?["result"]["ok"], false);
    h.reopen(DigMode::Control)?;
    h.game.borrow_mut().reply = Reply::Designated;
    let (next, seal, _) = h.prepare(1)?;
    success(&h.commit(&next, seal)?);
    assert_eq!(h.game.borrow().count(DigStage::Prepare), 2);
    assert_eq!(h.game.borrow().count(DigStage::Commit), 2);
    Ok(())
}

#[test]
fn blueprint_stop_is_durable_and_preserves_exact_native_recovery() -> Result<()> {
    let mut h = Harness::new(sparse()?, DigCheckpointPolicy::DisposableFortress)?;
    success(&h.publish()?);
    let (plan, seal, _) = h.prepare(0)?;
    let id = h.definition()?.id();
    let native_before = h.runtime.native.snapshot_bytes();
    let head_before = h.state.blueprint.as_ref().ok_or_else(exhausted)?.head();
    // Stopping is local Query authority, never a fresh designation grant.
    let grants = h.c.grants.clone();
    h.c.grants.retain(|g| g.capability == Capability::Query);
    let stopped = h.run("fortress.cancel", Ok(Action::StopBlueprint(id)))?;
    success(&stopped);
    assert_eq!(stopped["result"]["native_effect_cancelled"], false);
    assert_eq!(h.runtime.native.snapshot_bytes(), native_before);
    assert_ne!(
        h.state.blueprint.as_ref().ok_or_else(exhausted)?.head(),
        head_before
    );
    let recovery = stopped["result"]["blueprint"]["recovery"].clone();
    h.c.grants = grants;
    assert_eq!(h.commit(&plan, seal)?["result"]["ok"], false);
    let queried = h.run(
        "fortress.wait",
        Ok(Action::Wait(plan.key().into(), plan.digest())),
    )?;
    success(&queried);
    assert_eq!(queried["result"]["effect"]["native"]["phase"], "prepared");
    let cancelled = h.run(
        "fortress.cancel",
        Ok(Action::Cancel(plan.key().into(), plan.digest())),
    )?;
    success(&cancelled);
    assert_eq!(
        cancelled["result"]["effect"]["native"]["reason"],
        "cancelled_before_dispatch"
    );
    assert_eq!(cancelled["result"]["blueprint"]["recovery"], recovery);
    h.reopen(DigMode::Control)?;
    assert!(h.state.blueprint.as_ref().ok_or_else(exhausted)?.stopped());
    assert_eq!(h.observe(1)?["result"]["ok"], false);
    assert_eq!(h.game.borrow().count(DigStage::Commit), 0);
    assert_eq!(h.game.borrow().count(DigStage::Query), 1);
    assert_eq!(h.game.borrow().count(DigStage::Cancel), 1);
    Ok(())
}

#[test]
fn blueprint_parent_corruption_after_dispatch_marker_prevents_native_commit() -> Result<()> {
    let mut h = Harness::new(sparse()?, DigCheckpointPolicy::DisposableFortress)?;
    success(&h.publish()?);
    let (plan, seal, _) = h.prepare(0)?;
    let recovery = h.state.blueprint.as_ref().ok_or_else(exhausted)?.recovery();
    h.runtime.corrupt_after_dispatch = true;
    let failed = h.commit(&plan, seal)?;
    assert_eq!(failed["result"]["ok"], false);
    assert_eq!(h.game.borrow().count(DigStage::Commit), 0);
    assert_eq!(
        last_state(&h.runtime.native.snapshot_bytes()),
        Some(DigState::DispatchStarted as u8)
    );
    assert_eq!(
        failed["result"]["blueprint"]["recovery"],
        batch::recovery(recovery)
    );
    assert_eq!(failed["result"]["blueprint"]["advancement_fenced"], true);
    assert_eq!(
        failed["agent_turn"]["active_work"]["pending_plans"][0]["idempotency_key"],
        plan.key()
    );
    // Parent damage cannot prevent retirement of the original native token.
    let cancelled = h.run(
        "fortress.cancel",
        Ok(Action::Cancel(plan.key().into(), plan.digest())),
    )?;
    success(&cancelled);
    assert_eq!(cancelled["result"]["effect"]["native"]["phase"], "refused");
    assert_eq!(cancelled["result"]["blueprint_inventory_verified"], false);
    assert!(cancelled["result"]["blueprint_error"].is_object());
    assert_eq!(h.game.borrow().count(DigStage::Cancel), 1);
    let native = h.state.control.get(plan.key(), plan.digest(), &h.c)?;
    assert_eq!(
        native.effect().map(DigEffect::phase),
        Some(DigPhase::Refused)
    );
    assert_eq!(h.game.borrow().count(DigStage::Commit), 0);
    Ok(())
}

fn append_record(
    raw: &mut Vec<u8>,
    head: &mut Digest32,
    event: &mut u64,
    plan: &DigPlan,
    state: DigState,
    phase: Option<DigPhase>,
) -> Result<()> {
    let bytes = plan.canonical_bytes();
    let effect = phase.map(|phase| native(plan, phase)).transpose()?;
    let mut body = vec![state as u8];
    body.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    body.extend_from_slice(&bytes);
    let bytes = effect.as_ref().map_or(&[][..], DigEffect::canonical_bytes);
    body.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    body.extend_from_slice(bytes);
    *event += 1;
    let mut frame = b"DFMDJR01".to_vec();
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(&event.to_be_bytes());
    frame.extend_from_slice(head.as_bytes());
    frame.extend_from_slice(&body);
    *head = hash(b"dfmcp-dig-journal-frame/1", &frame);
    frame.extend_from_slice(head.as_bytes());
    frame.extend_from_slice(b"DFMDJEND");
    raw.extend_from_slice(&frame);
    Ok(())
}

#[test]
fn blueprint_full_128_child_audit_and_pages_retain_last_pending_obligation() -> Result<()> {
    let layout = Layout::new(
        (0..32)
            .map(|index| BlueprintPart {
                origin: [1 + 2 * index, 1, 1],
                size: [1, 1, 4],
            })
            .collect(),
    )?;
    assert_eq!(layout.regions().len(), 128);
    let mut h = Harness::new(layout, DigCheckpointPolicy::DisposableFortress)?;
    success(&h.publish()?);
    let definition = h.definition()?.clone();
    let mut raw = h.runtime.native.snapshot_bytes();
    let mut head = definition.journal_id();
    let mut event = 0;
    let mut game = Game::default();
    for (index, region) in h.layout.regions().iter().copied().enumerate() {
        let plan = DigPlan::new(&definition.key(index)?, false, game.capture(region)?)?;
        append_record(
            &mut raw,
            &mut head,
            &mut event,
            &plan,
            DigState::Intent,
            None,
        )?;
        append_record(
            &mut raw,
            &mut head,
            &mut event,
            &plan,
            DigState::Prepared,
            Some(DigPhase::Prepared),
        )?;
        if index < 127 {
            append_record(
                &mut raw,
                &mut head,
                &mut event,
                &plan,
                DigState::DispatchStarted,
                Some(DigPhase::Prepared),
            )?;
            append_record(
                &mut raw,
                &mut head,
                &mut event,
                &plan,
                DigState::Terminal,
                Some(DigPhase::Designated),
            )?;
            game.apply(region);
        }
    }
    h.runtime
        .native
        .data
        .borrow_mut()
        .get_mut()
        .clone_from(&raw);
    {
        let mut live = h.game.borrow_mut();
        live.sequence = game.sequence;
        live.targets = game.targets;
    }
    h.reopen(DigMode::Control)?;
    let mut result = h.query(0, 16)?;
    let mut rows = vec![];
    let calls = h.game.borrow().calls.len();
    loop {
        success(&result);
        let project = &result["result"]["blueprint"];
        assert_eq!(project["designated"], 127);
        assert_eq!(project["pending_step"], 127);
        assert_eq!(project["next"], Value::Null);
        assert_eq!(
            result["agent_turn"]["active_work"]["pending_plans"][0]["idempotency_key"],
            definition.key(127)?
        );
        for row in project["steps"].as_array().ok_or_else(exhausted)? {
            rows.push(row["index"].as_u64().ok_or_else(exhausted)?);
        }
        if project["next_query"].is_null() {
            break;
        }
        let action = Query::parse(&project["next_query"].to_string()).map(Action::Query);
        result = h.run("fortress.query", action)?;
    }
    assert_eq!(rows, (0..128).collect::<Vec<_>>());
    assert_eq!(h.game.borrow().calls.len(), calls);
    assert_eq!(h.observe(0)?["result"]["ok"], false);
    assert_eq!(h.game.borrow().calls.len(), calls);
    Ok(())
}

#[test]
fn blueprint_query_pins_closed_inputs_and_error_release_handoffs_are_exact() -> Result<()> {
    let layout = sparse()?;
    let blueprint =
        String::from_utf8(layout.canonical_bytes().to_vec()).map_err(|_| exhausted())?;
    assert!(plan_action(None, None, None, Some(blueprint.clone())).is_ok());
    assert!(plan_action(Some("child".into()), None, None, Some(blueprint.clone())).is_err());
    assert!(plan_action(None, None, Some(false), Some(blueprint.clone())).is_err());
    assert!(
        plan_action(
            Some("child".into()),
            Some(Digest32::ZERO.to_string()),
            None,
            None
        )
        .is_err()
    );
    assert!(batch::parse(&blueprint.replace("\"floor\"", "\"ramp\"")).is_err());
    assert!(batch::parse(&blueprint.replacen("{", "{\"session_id\":\"forged\",", 1)).is_err());
    let mut h = Harness::new(layout, DigCheckpointPolicy::DisposableFortress)?;
    success(&h.publish()?);
    let id = h.definition()?.id();
    let before = h.query(0, 1)?;
    let continuation = before["result"]["blueprint"]["next_query"].to_string();
    assert!(Query::parse(&continuation).is_ok());
    for raw in [
        json!({"mode":"blueprint","blueprint_id":id.to_string(),"limit":17}).to_string(),
        json!({"mode":"blueprint","blueprint_id":id.to_string(),"offset":128}).to_string(),
        json!({"mode":"blueprint","blueprint_id":id.to_string(),"expected_head":null}).to_string(),
        json!({"mode":"blueprint","blueprint_id":id.to_string(),"unknown":1}).to_string(),
        format!("{{\"mode\":\"blueprint\",\"blueprint_id\":\"{id}\",\"blueprint_id\":\"{id}\"}}"),
    ] {
        assert!(Query::parse(&raw).is_err());
    }
    for (identity, native_head, parent_head) in [
        (Digest32::ZERO.to_string(), None, None),
        (id.to_string(), Some(Digest32::ZERO.to_string()), None),
        (id.to_string(), None, Some(Digest32::ZERO.to_string())),
    ] {
        let denied = h.run(
            "fortress.query",
            Ok(Action::Query(Query::Blueprint {
                blueprint_id: identity,
                expected_head: native_head,
                expected_parent_head: parent_head,
                offset: None,
                limit: None,
            })),
        )?;
        assert_eq!(denied["result"]["ok"], false);
        assert_eq!(
            denied["result"]["blueprint"]["blueprint_id"],
            id.to_string()
        );
    }
    success(&h.run("fortress.cancel", Ok(Action::StopBlueprint(id)))?);
    let stale = h.run(
        "fortress.query",
        Query::parse(&continuation).map(Action::Query),
    )?;
    assert_eq!(stale["result"]["error"]["code"], "stale_anchor");
    let recovery = h.state.blueprint.as_ref().ok_or_else(exhausted)?.recovery();
    let released = retain_blueprint(
        &h.state,
        json!({"ok":true,"scope":"session","release_for_recovery":true}),
    );
    assert_eq!(released["blueprint"]["recovery"], batch::recovery(recovery));
    let failed = h.run(
        "fortress.plan",
        Err(error(ErrorCode::InvalidRequest, "bad request")),
    )?;
    assert_eq!(
        failed["result"]["blueprint"]["recovery"],
        batch::recovery(recovery)
    );
    let mut tiny = h.c.clone();
    tiny.budget.max_bytes = 0;
    let game = h.game.clone();
    let raw = run_action(
        &mut h.state,
        tiny,
        "fortress.query",
        Ok(Action::Inventory),
        Instant::now(),
        &mut h.runtime,
        move |binding, region, _| {
            Ok(Source {
                game,
                binding: binding.clone(),
                region,
                permit: None,
            })
        },
    );
    let failed: Value = serde_json::from_str(&raw).map_err(|_| exhausted())?;
    assert_eq!(failed["result"]["ok"], false);
    assert_eq!(
        failed["result"]["blueprint"]["recovery"],
        batch::recovery(recovery)
    );
    Ok(())
}

#[path = "batch_tests/monitor_tests.rs"]
mod monitor_tests;
