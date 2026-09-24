use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, GameTick, ObservationCursor, RequestId, SessionId,
    StateAnchor, WorkBudget,
};
use std::cell::{Cell, RefCell};
use std::io::{Cursor, Read, Seek, Write};
use std::rc::Rc;

pub(super) type Trace = Rc<RefCell<Vec<&'static str>>>;
pub(super) struct Memory {
    pub bytes: Cursor<Vec<u8>>,
    pub trace: Trace,
    pub syncs: usize,
    pub fail_sync: Option<usize>,
    pub write_limit: Option<usize>,
    pub custody: Rc<Cell<bool>>,
}
impl Memory {
    pub fn new(bytes: Vec<u8>, trace: Trace) -> Self {
        Self {
            bytes: Cursor::new(bytes),
            trace,
            syncs: 0,
            fail_sync: None,
            write_limit: None,
            custody: Rc::new(Cell::new(true)),
        }
    }
}
impl Read for Memory {
    fn read(&mut self, data: &mut [u8]) -> io::Result<usize> {
        self.bytes.read(data)
    }
}
impl Seek for Memory {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.bytes.seek(position)
    }
}
impl Write for Memory {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let available = self.write_limit.map_or(data.len(), |limit| {
            limit
                .saturating_sub(self.bytes.position() as usize)
                .min(data.len())
        });
        if available == 0 {
            return Err(io::Error::other("injected torn write"));
        }
        self.trace.borrow_mut().push("write");
        self.bytes.write(&data[..available])
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        self.syncs += 1;
        if self.fail_sync == Some(self.syncs) {
            return Err(io::Error::other("injected sync failure"));
        }
        self.trace.borrow_mut().push("sync");
        Ok(())
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("no repair in tests"))
    }
    fn validate_identity(&self) -> io::Result<()> {
        if self.custody.get() {
            Ok(())
        } else {
            Err(io::Error::other("injected lost custody"))
        }
    }
}
pub(super) fn unhex(text: &str) -> Result<Vec<u8>> {
    let text = text.trim();
    if text.len() % 2 != 0 {
        return Err(exhausted());
    }
    text.as_bytes()
        .chunks_exact(2)
        .map(|p| {
            let a = char::from(p[0]).to_digit(16).ok_or_else(exhausted)?;
            let b = char::from(p[1]).to_digit(16).ok_or_else(exhausted)?;
            Ok((a * 16 + b) as u8)
        })
        .collect()
}
pub(super) fn plan(key: &str) -> Result<WorkOrderPlan> {
    let bytes = unhex(include_str!(
        "../../tests/fixtures/work_order_observation_v1_10.hex"
    ))?;
    WorkOrderPlan::new(
        WorkOrderObservation::decode(&bytes)?,
        key,
        WorkOrderSpec::new(WorkOrderRecipe::WoodenBed, 5)?,
    )
}
pub(super) fn context() -> Result<OperationContext> {
    let p = plan("order-001")?;
    let o = p.observation();
    Ok(OperationContext {
        session_id: SessionId::new(11),
        request_id: RequestId::new(12),
        anchor: StateAnchor {
            fortress_id: o.fortress_id(),
            tick: GameTick(o.tick()),
            cursor: ObservationCursor::ORIGIN,
            state_hash: o.witness(),
        },
        budget: WorkBudget {
            max_bytes: 65 * 1024 * 1024,
            max_entities: 4096,
            max_wall_millis: 60_000,
            ..WorkBudget::default()
        },
        cancellation_requested: false,
        grants: [Capability::Query, Capability::ConfigureProduction]
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope::default(),
                max_risk: RiskTier::Reversible,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect(),
    })
}
fn text(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&(value.len() as u16).to_be_bytes());
    out.extend_from_slice(value.as_bytes());
}
pub(super) fn effect(p: &WorkOrderPlan, state: WorkOrderState) -> Result<WorkOrderEffect> {
    let o = p.observation();
    let mut bytes = b"DFMWOE10".to_vec();
    for n in [o.generation(), o.sequence(), o.tick()] {
        bytes.extend_from_slice(&n.to_be_bytes());
    }
    bytes.extend_from_slice(&o.next_order_id().to_be_bytes());
    bytes.push(p.spec().recipe() as u8);
    bytes.extend_from_slice(&p.spec().amount().to_be_bytes());
    bytes.extend_from_slice(o.witness().as_bytes());
    bytes.extend_from_slice(p.digest().as_bytes());
    bytes.extend_from_slice(p.prepare_token());
    let known = state == WorkOrderState::Created;
    bytes.extend_from_slice(&[state as u8, u8::from(known)]);
    bytes.extend_from_slice(&(if known { o.tick() } else { 0 }).to_be_bytes());
    let (after, config) = if known {
        let mut after = o.canonical_bytes().to_vec();
        after[16..24].copy_from_slice(&(o.sequence() + 1).to_be_bytes());
        after[32..36].copy_from_slice(&(o.next_order_id() + 1).to_be_bytes());
        let offset = 43 + o.world_folder().len();
        after[offset..offset + 4]
            .copy_from_slice(&((o.order_ids().len() + 1) as u32).to_be_bytes());
        after.extend_from_slice(&o.next_order_id().to_be_bytes());
        let mut config = b"DFMWOC10".to_vec();
        config.extend_from_slice(&o.next_order_id().to_be_bytes());
        config.push(p.spec().recipe() as u8);
        config.extend_from_slice(&p.spec().amount().to_be_bytes());
        (Digest32::of_bytes(&after), Digest32::of_bytes(&config))
    } else {
        (Digest32::ZERO, Digest32::ZERO)
    };
    bytes.extend_from_slice(after.as_bytes());
    bytes.extend_from_slice(config.as_bytes());
    let receipt = if state.terminal() {
        let mut proof = b"dfmcp-work-order-receipt/1\0".to_vec();
        proof.extend_from_slice(&o.generation().to_be_bytes());
        text(&mut proof, p.key());
        proof.extend_from_slice(&bytes[73..195]);
        Digest32::of_bytes(&proof)
    } else {
        Digest32::ZERO
    };
    bytes.extend_from_slice(receipt.as_bytes());
    text(&mut bytes, p.key());
    WorkOrderEffect::decode(&bytes, p)
}
pub(super) struct Source {
    pub manifest: WorkOrderManifest,
    pub trace: Trace,
    pub reads: usize,
    pub prepares: usize,
    pub commits: usize,
    pub queries: usize,
    pub fenced: bool,
    pub read_fails: bool,
    pub commit_state: Option<WorkOrderState>,
    pub query_state: Option<WorkOrderState>,
}
impl Source {
    pub fn new(trace: Trace) -> Self {
        Self {
            manifest: WorkOrderManifest {
                generation: 7,
                df_version: "df".into(),
                dfhack_version: "dfhack".into(),
            },
            trace,
            reads: 0,
            prepares: 0,
            commits: 0,
            queries: 0,
            fenced: false,
            read_fails: false,
            commit_state: Some(WorkOrderState::Created),
            query_state: Some(WorkOrderState::Created),
        }
    }
}
impl WorkOrderQuerySource for Source {
    fn manifest(&self) -> &WorkOrderManifest {
        &self.manifest
    }
    fn fence(&mut self) {
        self.fenced = true;
    }
    fn read_orders(&mut self, _: Duration) -> Result<WorkOrderObservation> {
        self.reads += 1;
        if self.read_fails {
            return Err(error(
                ErrorCode::AdapterUnavailable,
                "injected read failure",
            ));
        }
        Ok(plan("order-001")?.observation().clone())
    }
    fn query(&mut self, p: &WorkOrderPlan, _: Duration) -> Result<Option<WorkOrderEffect>> {
        self.queries += 1;
        self.trace.borrow_mut().push("query");
        self.query_state.map(|state| effect(p, state)).transpose()
    }
}
impl WorkOrderSource for Source {
    fn prepare(&mut self, p: &WorkOrderPlan, _: Duration) -> Result<WorkOrderEffect> {
        self.prepares += 1;
        self.trace.borrow_mut().push("prepare");
        effect(p, WorkOrderState::Prepared)
    }
    fn commit(
        &mut self,
        p: &WorkOrderPlan,
        _: &WorkOrderEffect,
        _: Duration,
    ) -> Result<WorkOrderEffect> {
        self.commits += 1;
        self.trace.borrow_mut().push("commit");
        self.commit_state
            .map(|state| effect(p, state))
            .unwrap_or_else(|| Err(error(ErrorCode::AdapterUnavailable, "lost insertion reply")))
    }
}
pub(super) fn setup() -> Result<(WorkOrderJournal<Memory>, Source)> {
    let trace = Trace::default();
    Ok((
        WorkOrderJournal::open(
            Memory::new(Vec::new(), trace.clone()),
            &context()?,
            JournalMode::Control,
            true,
        )?,
        Source::new(trace),
    ))
}
fn reopen(bytes: Vec<u8>, mode: JournalMode) -> Result<WorkOrderJournal<Memory>> {
    WorkOrderJournal::open(
        Memory::new(bytes, Trace::default()),
        &context()?,
        mode,
        false,
    )
}

#[test]
fn native_vectors_and_durable_roundtrip_replay_without_duplicate_insertion() -> Result<()> {
    let p = plan("order-001")?;
    let c = context()?;
    let (mut j, mut s) = setup()?;
    assert_eq!(
        effect(&p, WorkOrderState::Created)?.canonical_bytes(),
        unhex(include_str!(
            "../../tests/fixtures/work_order_created_v1_10.hex"
        ))?
    );
    let prepared = j.prepare(&mut s, &p, &c)?;
    assert_eq!(j.prepare(&mut s, &p, &c)?, prepared);
    assert_eq!(s.prepares, 1);
    s.trace.borrow_mut().clear();
    let created = j.commit(&mut s, &p, &c)?;
    assert_eq!(created.state(), CreationState::Created);
    assert_eq!(
        &*s.trace.borrow(),
        &["write", "sync", "commit", "write", "sync"]
    );
    let mut restored = reopen(j.storage.bytes.into_inner(), JournalMode::Control)?;
    assert_eq!(restored.commit(&mut s, &p, &c)?, created);
    assert_eq!(s.commits, 1);
    Ok(())
}

#[test]
fn failed_dispatch_sync_fences_without_insertion_and_cannot_redispatch_after_restart() -> Result<()>
{
    let (mut j, mut s) = setup()?;
    let p = plan("order-001")?;
    let c = context()?;
    j.prepare(&mut s, &p, &c)?;
    j.storage.fail_sync = Some(3);
    assert!(matches!(j.commit(&mut s, &p, &c), Err(e) if e.code == ErrorCode::EffectIndeterminate));
    assert_eq!(s.commits, 0);
    assert!(j.fenced());
    let mut restored = reopen(j.storage.bytes.into_inner(), JournalMode::Control)?;
    assert!(restored.commit(&mut s, &p, &c).is_err());
    assert_eq!(s.commits, 0);
    assert_eq!(restored.summary(&c)?.unresolved, 1);
    Ok(())
}

#[test]
fn lost_terminal_sync_recovers_both_complete_and_missing_terminal_frames() -> Result<()> {
    let (mut j, mut s) = setup()?;
    let p = plan("order-001")?;
    let c = context()?;
    let prepared = j.prepare(&mut s, &p, &c)?;
    let started_end = j.length as usize + PREFIX_BYTES + encode_record(&prepared)?.len() + 40;
    j.storage.fail_sync = Some(4);
    assert!(j.commit(&mut s, &p, &c).is_err());
    assert_eq!(s.commits, 1);
    let bytes = j.storage.bytes.into_inner();
    for retained in [bytes.clone(), bytes[..started_end].to_vec()] {
        let mut restored = reopen(retained, JournalMode::Reconcile)?;
        assert_eq!(
            restored.reconcile(&mut s, &p, &c)?.state(),
            CreationState::Created
        );
    }
    assert_eq!(s.commits, 1);
    Ok(())
}

#[test]
fn unknown_other_key_blocks_preparation_and_dispatch_at_the_coordinator_boundary() -> Result<()> {
    let (mut j, mut s) = setup()?;
    let c = context()?;
    let first = plan("first")?;
    let second = plan("second")?;
    let third = plan("third")?;
    j.prepare(&mut s, &first, &c)?;
    j.prepare(&mut s, &second, &c)?;
    s.commit_state = None;
    assert_eq!(
        j.commit(&mut s, &first, &c)?.state(),
        CreationState::Indeterminate
    );
    assert!(j.prepare(&mut s, &third, &c).is_err());
    assert!(j.commit(&mut s, &second, &c).is_err());
    assert_eq!(s.prepares, 2);
    assert_eq!(s.commits, 1);
    j.cancel(&second, &c)?; // Local retirement is still safe; not an undo.
    assert_eq!(
        j.reconcile(&mut s, &first, &c)?.state(),
        CreationState::Created
    );
    assert!(j.prepare(&mut s, &third, &c).is_ok());
    Ok(())
}

#[test]
fn query_only_recovery_has_no_creation_edge_even_with_a_later_injected_grant() -> Result<()> {
    struct QueryOnly(Source);
    impl WorkOrderQuerySource for QueryOnly {
        fn manifest(&self) -> &WorkOrderManifest {
            &self.0.manifest
        }
        fn fence(&mut self) {
            self.0.fence();
        }
        fn read_orders(&mut self, timeout: Duration) -> Result<WorkOrderObservation> {
            self.0.read_orders(timeout)
        }
        fn query(
            &mut self,
            p: &WorkOrderPlan,
            timeout: Duration,
        ) -> Result<Option<WorkOrderEffect>> {
            self.0.query(p, timeout)
        }
    }
    let (mut j, mut s) = setup()?;
    let p = plan("order-001")?;
    let mut c = context()?;
    j.prepare(&mut s, &p, &c)?;
    s.commit_state = None;
    j.commit(&mut s, &p, &c)?;
    c.grants.retain(|g| g.capability == Capability::Query);
    let mut restored = WorkOrderJournal::open(
        Memory::new(j.storage.bytes.into_inner(), Trace::default()),
        &c,
        JournalMode::Reconcile,
        false,
    )?;
    assert_eq!(
        restored
            .reconcile(&mut QueryOnly(Source::new(Trace::default())), &p, &c)?
            .state(),
        CreationState::Created
    );
    assert!(
        restored
            .prepare(&mut s, &plan("new")?, &context()?)
            .is_err()
    );
    assert!(restored.cancel(&p, &context()?).is_err());
    Ok(())
}

#[test]
fn absent_prepared_or_changed_generation_never_clears_ambiguous_creation() -> Result<()> {
    for query in [None, Some(WorkOrderState::Prepared)] {
        let (mut j, mut s) = setup()?;
        let p = plan("order-001")?;
        let c = context()?;
        j.prepare(&mut s, &p, &c)?;
        s.commit_state = None;
        j.commit(&mut s, &p, &c)?;
        s.query_state = query;
        let first = j.reconcile(&mut s, &p, &c)?;
        let head = j.head;
        assert_eq!(first.state(), CreationState::Indeterminate);
        assert_eq!(j.reconcile(&mut s, &p, &c)?, first);
        assert_eq!(j.head, head);
        s.manifest.generation += 1;
        let calls = s.queries;
        j.reconcile(&mut s, &p, &c)?;
        assert_eq!(s.queries, calls);
        assert!(j.commit(&mut s, &p, &c).is_err());
        assert_eq!(s.commits, 1);
    }
    Ok(())
}

#[test]
fn native_unknown_is_immutable_and_cancelled_keys_are_permanently_retired() -> Result<()> {
    let (mut j, mut s) = setup()?;
    let p = plan("unknown")?;
    let c = context()?;
    j.prepare(&mut s, &p, &c)?;
    s.commit_state = Some(WorkOrderState::Unknown);
    j.commit(&mut s, &p, &c)?;
    assert!(j.reconcile(&mut s, &p, &c).is_err());
    assert_eq!(j.summary(&c)?.unresolved, 1);
    let (mut j, mut s) = setup()?;
    let p = plan("cancel")?;
    j.prepare(&mut s, &p, &c)?;
    let cancelled = j.cancel(&p, &c)?;
    assert_eq!(j.commit(&mut s, &p, &c)?, cancelled);
    assert_eq!(j.prepare(&mut s, &p, &c)?, cancelled);
    assert_eq!(s.commits, 0);
    assert_eq!(s.prepares, 1);
    let mut restored = reopen(j.storage.bytes.into_inner(), JournalMode::Control)?;
    assert_eq!(restored.cancel(&p, &c)?, cancelled);
    Ok(())
}

#[test]
fn authority_scope_expiry_cancellation_and_budget_fail_before_native_work() -> Result<()> {
    let p = plan("order-001")?;
    for kind in 0..8 {
        let (mut j, mut s) = setup()?;
        let mut c = context()?;
        match kind {
            0 => c.grants.retain(|g| g.capability == Capability::Query),
            1 => c.cancellation_requested = true,
            2 => c.anchor.fortress_id = FortressId::NIL,
            3 => c.anchor.state_hash = Digest32::ZERO,
            4 => c.budget.max_bytes = RPC_RESERVE_BYTES,
            5 => {
                for g in &mut c.grants {
                    g.remaining_uses = Some(1);
                }
            }
            6 => {
                for g in &mut c.grants {
                    g.expires_at_tick = Some(GameTick(0));
                }
            }
            _ => {
                for g in &mut c.grants {
                    g.scope.entity_ids.insert(dfmcp_core::EntityId::new(7));
                }
            }
        }
        assert!(j.prepare(&mut s, &p, &c).is_err());
        assert_eq!(s.prepares, 0);
        assert_eq!(s.commits, 0);
    }
    Ok(())
}

#[test]
fn complete_pages_include_terminal_records_and_require_exact_head_and_custody() -> Result<()> {
    let (mut j, mut s) = setup()?;
    let c = context()?;
    for key in ["a", "b", "c"] {
        let p = plan(key)?;
        j.prepare(&mut s, &p, &c)?;
        j.cancel(&p, &c)?;
    }
    let head = j.summary(&c)?.head;
    let first = j.records_page(head, None, 2, &c)?;
    assert_eq!(
        first
            .records
            .iter()
            .map(|r| r.plan().key())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert_eq!(first.next_after.as_deref(), Some("b"));
    assert_eq!(
        j.records_page(head, Some("b"), 2, &c)?.records[0]
            .plan()
            .key(),
        "c"
    );
    j.prepare(&mut s, &plan("d")?, &c)?;
    assert!(j.records_page(head, Some("b"), 2, &c).is_err());
    j.storage.custody.set(false);
    assert!(j.summary(&c).is_err());
    assert!(j.lookup("a", &c).is_err());
    assert!(j.commit(&mut s, &plan("a")?, &c).is_err());
    Ok(())
}

#[test]
fn corruptions_and_torn_tails_are_rejected_without_repair() -> Result<()> {
    let (mut j, mut s) = setup()?;
    let c = context()?;
    let p = plan("order-001")?;
    j.prepare(&mut s, &p, &c)?;
    let prepared_end = j.length as usize;
    j.commit(&mut s, &p, &c)?;
    let bytes = j.storage.bytes.into_inner();
    for i in 0..bytes.len() {
        let mut bad = bytes.clone();
        bad[i] ^= 1;
        assert!(reopen(bad, JournalMode::Offline).is_err(), "byte {i}");
    }
    let frame_length = (bytes.len() - prepared_end) / 2;
    for end in 0..bytes.len() {
        if [HEADER_BYTES, prepared_end, prepared_end + frame_length].contains(&end) {
            continue;
        }
        assert!(
            reopen(bytes[..end].to_vec(), JournalMode::Offline).is_err(),
            "prefix {end}"
        );
    }
    Ok(())
}

#[test]
fn rehashed_illegal_histories_cannot_reset_dispatch_or_rewrite_terminal_evidence() -> Result<()> {
    let (mut j, mut s) = setup()?;
    let c = context()?;
    let p = plan("order-001")?;
    let prepared = j.prepare(&mut s, &p, &c)?;
    j.commit(&mut s, &p, &c)?;
    let bytes = j.storage.bytes.clone().into_inner();
    let mut cancelled = prepared.clone();
    cancelled.state = CreationState::CancelledBeforeDispatch;
    let mut started = prepared.clone();
    started.state = CreationState::DispatchStarted;
    for next in [prepared, cancelled, started] {
        let body = encode_record(&next)?;
        let mut prefix = FRAME.to_vec();
        prefix.extend_from_slice(&(body.len() as u32).to_be_bytes());
        prefix.extend_from_slice(&(j.transitions + 1).to_be_bytes());
        prefix.extend_from_slice(j.head.as_bytes());
        let hash = frame_hash(j.id, &prefix, &body);
        let mut forged = bytes.clone();
        forged.extend_from_slice(&prefix);
        forged.extend_from_slice(&body);
        forged.extend_from_slice(hash.as_bytes());
        forged.extend_from_slice(FOOTER);
        assert!(reopen(forged, JournalMode::Control).is_err());
    }
    Ok(())
}

#[test]
fn record_transition_and_byte_capacity_are_reserved_before_prepare_or_commit() -> Result<()> {
    let (mut j, mut s) = setup()?;
    let p = plan("order-001")?;
    let c = context()?;
    j.transitions = MAX_TRANSITIONS - 2;
    assert!(j.prepare(&mut s, &p, &c).is_err());
    assert_eq!(s.prepares, 0);
    let (mut j, mut s) = setup()?;
    j.prepare(&mut s, &p, &c)?;
    j.length = MAX_JOURNAL_BYTES - MAX_FRAME_BYTES as u64;
    assert!(j.commit(&mut s, &p, &c).is_err());
    assert_eq!(s.commits, 0);
    Ok(())
}

#[test]
fn partial_dispatch_write_fences_and_does_not_call_insertion() -> Result<()> {
    let (mut j, mut s) = setup()?;
    let p = plan("order-001")?;
    let c = context()?;
    j.prepare(&mut s, &p, &c)?;
    j.storage.write_limit = Some(j.length as usize + 17);
    assert!(j.commit(&mut s, &p, &c).is_err());
    assert_eq!(s.commits, 0);
    assert!(j.fenced());
    assert!(reopen(j.storage.bytes.into_inner(), JournalMode::Reconcile).is_err());
    Ok(())
}
