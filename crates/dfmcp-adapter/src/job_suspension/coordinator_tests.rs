use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, GameTick, ObservationCursor, RequestId, SessionId,
    StateAnchor, WorkBudget,
};
use std::cell::RefCell;
use std::io::{Cursor, Read, Seek, Write};
use std::rc::Rc;

const OBSERVATION: &str = include_str!("../../tests/fixtures/job_suspension_observation_v1_9.hex");
const EFFECT: &str = include_str!("../../tests/fixtures/job_suspension_effect_v1_9.hex");
fn hex(text: &str) -> Result<Vec<u8>> {
    text.trim()
        .as_bytes()
        .chunks_exact(2)
        .map(|p| {
            let a = char::from(p[0]).to_digit(16).ok_or_else(exhausted)?;
            let b = char::from(p[1]).to_digit(16).ok_or_else(exhausted)?;
            Ok((a * 16 + b) as u8)
        })
        .collect()
}
fn plan() -> Result<SuspensionPlan> {
    SuspensionPlan::new(JobObservation::decode(&hex(OBSERVATION)?)?, "job-001", true)
}
fn context() -> Result<OperationContext> {
    let p = plan()?;
    Ok(OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(2),
        anchor: StateAnchor {
            fortress_id: job_fortress_id(p.observation()),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(p.observation().tick()),
            state_hash: p.observation().witness(),
        },
        budget: WorkBudget::default(),
        cancellation_requested: false,
        grants: [Capability::ConfigureProduction, Capability::Query]
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
fn native(p: &SuspensionPlan, state: SuspensionState) -> Result<SuspensionEffect> {
    let o = p.observation();
    let mut bytes = b"DFMJSE19".to_vec();
    for value in [o.generation(), o.sequence(), o.tick()] {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    bytes.extend_from_slice(&o.job_id().to_be_bytes());
    bytes.push(u8::from(p.desired()));
    bytes.extend_from_slice(o.witness().as_bytes());
    bytes.extend_from_slice(p.digest().as_bytes());
    bytes.extend_from_slice(p.prepare_token());
    let known = matches!(
        state,
        SuspensionState::Applied | SuspensionState::NotApplied
    );
    let after = known
        && if state == SuspensionState::Applied {
            p.desired()
        } else {
            !p.desired()
        };
    bytes.extend_from_slice(&[state as u8, u8::from(known), u8::from(after)]);
    bytes.extend_from_slice(&(if known { o.tick() } else { 0 }).to_be_bytes());
    let after_witness = if known {
        o.expected_after_witness(after)?
    } else {
        Digest32::ZERO
    };
    bytes.extend_from_slice(after_witness.as_bytes());
    let receipt = if state.terminal() {
        let mut proof = b"dfmcp-job-suspension-receipt/1\0".to_vec();
        proof.extend_from_slice(&o.generation().to_be_bytes());
        put_text(&mut proof, p.key());
        proof.extend_from_slice(&bytes[69..160]);
        Digest32::of_bytes(&proof)
    } else {
        Digest32::ZERO
    };
    bytes.extend_from_slice(receipt.as_bytes());
    put_text(&mut bytes, p.key());
    SuspensionEffect::decode(&bytes, p)
}

type Trace = Rc<RefCell<Vec<&'static str>>>;
struct Memory {
    bytes: Cursor<Vec<u8>>,
    trace: Trace,
    syncs: usize,
    fail_sync: Option<usize>,
    write_limit: Option<usize>,
}
impl Memory {
    fn new(bytes: Vec<u8>, trace: Trace) -> Self {
        Self {
            bytes: Cursor::new(bytes),
            trace,
            syncs: 0,
            fail_sync: None,
            write_limit: None,
        }
    }
}
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.bytes.read(out)
    }
}
impl Seek for Memory {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.bytes.seek(from)
    }
}
impl Write for Memory {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let available = self.write_limit.map_or(data.len(), |end| {
            end.saturating_sub(self.bytes.position() as usize)
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
            self.trace.borrow_mut().push("sync-failed");
            return Err(io::Error::other("injected sync"));
        }
        self.trace.borrow_mut().push("sync");
        Ok(())
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("test forbids truncation"))
    }
}
struct Source {
    manifest: JobControlManifest,
    trace: Trace,
    prepares: usize,
    commits: usize,
    queries: usize,
    fenced: bool,
    commit_state: Option<SuspensionState>,
    query_state: Option<SuspensionState>,
}
impl Source {
    fn new(trace: Trace) -> Self {
        Self {
            manifest: JobControlManifest {
                generation: 7,
                df_version: "df".into(),
                dfhack_version: "dfhack".into(),
            },
            trace,
            prepares: 0,
            commits: 0,
            queries: 0,
            fenced: false,
            commit_state: Some(SuspensionState::Applied),
            query_state: Some(SuspensionState::Applied),
        }
    }
}
impl JobSuspensionSource for Source {
    fn manifest(&self) -> &JobControlManifest {
        &self.manifest
    }
    fn fence(&mut self) {
        self.fenced = true;
    }
    fn prepare(&mut self, p: &SuspensionPlan, _: Duration) -> Result<SuspensionEffect> {
        self.prepares += 1;
        self.trace.borrow_mut().push("prepare");
        native(p, SuspensionState::Prepared)
    }
    fn commit(
        &mut self,
        p: &SuspensionPlan,
        _: &SuspensionEffect,
        _: Duration,
    ) -> Result<SuspensionEffect> {
        self.commits += 1;
        self.trace.borrow_mut().push("commit");
        self.commit_state
            .map(|s| native(p, s))
            .unwrap_or_else(|| Err(fail(ErrorCode::AdapterUnavailable, "lost reply")))
    }
    fn query(&mut self, p: &SuspensionPlan, _: Duration) -> Result<Option<SuspensionEffect>> {
        self.queries += 1;
        self.trace.borrow_mut().push("query");
        self.query_state.map(|s| native(p, s)).transpose()
    }
}
fn setup() -> Result<(JobControlJournal<Memory>, Source)> {
    let trace = Trace::default();
    let journal =
        JobControlJournal::open(Memory::new(Vec::new(), trace.clone()), &context()?, true)?;
    Ok((journal, Source::new(trace)))
}
fn reopen(bytes: Vec<u8>) -> Result<JobControlJournal<Memory>> {
    JobControlJournal::open(Memory::new(bytes, Trace::default()), &context()?, false)
}

#[test]
fn actual_native_fixture_matches_the_source_double_and_journal_round_trip() -> Result<()> {
    let p = plan()?;
    assert_eq!(
        native(&p, SuspensionState::Applied)?.canonical_bytes(),
        hex(EFFECT)?
    );
    let (mut j, mut source) = setup()?;
    let c = context()?;
    let first = j.prepare(&mut source, &p, &c)?;
    assert_eq!(first.state(), DurableJobState::Prepared);
    let replay = j.prepare(&mut source, &p, &c)?;
    assert_eq!(replay, first);
    assert_eq!(source.prepares, 1);
    let done = j.commit(&mut source, &p, &c)?;
    assert_eq!(done.state(), DurableJobState::Applied);
    assert_eq!(j.commit(&mut source, &p, &c)?, done);
    assert_eq!(source.commits, 1);
    let opened = reopen(j.storage.bytes.into_inner())?;
    assert_eq!(opened.lookup(p.key(), &c)?, Some(&done));
    assert!(opened.unresolved(&c)?.is_empty());
    Ok(())
}

#[test]
fn sync_precedes_dispatch_and_terminal_sync_precedes_acknowledgement() -> Result<()> {
    let (mut j, mut source) = setup()?;
    let p = plan()?;
    let c = context()?;
    j.prepare(&mut source, &p, &c)?;
    source.trace.borrow_mut().clear();
    j.commit(&mut source, &p, &c)?;
    assert_eq!(
        &*source.trace.borrow(),
        &["write", "sync", "commit", "write", "sync"]
    );
    Ok(())
}

#[test]
fn failed_dispatch_sync_never_reaches_native_setter_and_reopens_nonretryable() -> Result<()> {
    let (mut j, mut source) = setup()?;
    let p = plan()?;
    let c = context()?;
    j.prepare(&mut source, &p, &c)?;
    j.storage.fail_sync = Some(3);
    assert!(j.commit(&mut source, &p, &c).is_err());
    assert_eq!(source.commits, 0);
    assert!(j.poisoned());
    let mut opened = reopen(j.storage.bytes.into_inner())?;
    assert_eq!(
        opened.lookup(p.key(), &c)?.map(|r| r.state()),
        Some(DurableJobState::DispatchStarted)
    );
    assert!(
        matches!(opened.commit(&mut source, &p, &c), Err(e) if e.code == ErrorCode::EffectIndeterminate)
    );
    assert_eq!(source.commits, 0);
    Ok(())
}

#[test]
fn terminal_sync_failure_recovers_from_either_complete_or_missing_terminal_frame() -> Result<()> {
    let (mut j, mut source) = setup()?;
    let p = plan()?;
    let c = context()?;
    let prepared = j.prepare(&mut source, &p, &c)?;
    let started_end = j.length as usize + FRAME_PREFIX + encode_record(&prepared)?.len() + 40;
    j.storage.fail_sync = Some(4);
    assert!(j.commit(&mut source, &p, &c).is_err());
    assert_eq!(source.commits, 1);
    assert!(j.poisoned());
    let bytes = j.storage.bytes.into_inner();
    for persisted in [bytes.clone(), bytes[..started_end].to_vec()] {
        let mut opened = reopen(persisted)?;
        let before = source.commits;
        let state = opened.lookup(p.key(), &c)?.ok_or_else(exhausted)?.state();
        if state == DurableJobState::DispatchStarted {
            assert!(opened.commit(&mut source, &p, &c).is_err());
        }
        let done = opened.reconcile(&mut source, &p, &c)?;
        assert_eq!(done.state(), DurableJobState::Applied);
        assert_eq!(source.commits, before);
    }
    Ok(())
}

#[test]
fn lost_reply_recovery_is_query_only_and_retains_complete_prepared_identity() -> Result<()> {
    let (mut j, mut source) = setup()?;
    let p = plan()?;
    let c = context()?;
    j.prepare(&mut source, &p, &c)?;
    source.commit_state = None;
    let uncertain = j.commit(&mut source, &p, &c)?;
    assert_eq!(uncertain.state(), DurableJobState::Indeterminate);
    assert!(source.fenced);
    let mut opened = reopen(j.storage.bytes.into_inner())?;
    assert_eq!(opened.unresolved(&c)?, vec![uncertain]);
    let mut recovery = Source::new(Trace::default());
    let mut query_only = c.clone();
    query_only
        .grants
        .retain(|g| g.capability == Capability::Query);
    let result = opened.reconcile(&mut recovery, &p, &query_only)?;
    assert_eq!(result.state(), DurableJobState::Applied);
    assert_eq!(
        (recovery.prepares, recovery.commits, recovery.queries),
        (0, 0, 1)
    );
    Ok(())
}

#[test]
fn missing_or_prepared_query_after_dispatch_never_reenables_commit() -> Result<()> {
    for query in [
        None,
        Some(SuspensionState::Prepared),
        Some(SuspensionState::Unknown),
    ] {
        let (mut j, mut source) = setup()?;
        let p = plan()?;
        let c = context()?;
        j.prepare(&mut source, &p, &c)?;
        source.commit_state = None;
        j.commit(&mut source, &p, &c)?;
        source.query_state = query;
        assert_eq!(
            j.reconcile(&mut source, &p, &c)?.state(),
            DurableJobState::Indeterminate
        );
        assert!(j.commit(&mut source, &p, &c).is_err());
        assert_eq!(source.commits, 1);
        let mut opened = reopen(j.storage.bytes.into_inner())?;
        assert!(opened.commit(&mut source, &p, &c).is_err());
        assert_eq!(source.commits, 1);
    }
    Ok(())
}

#[test]
fn changed_incarnation_or_software_does_not_query_or_commit_again() -> Result<()> {
    for software in [false, true] {
        let (mut j, mut source) = setup()?;
        let p = plan()?;
        let c = context()?;
        j.prepare(&mut source, &p, &c)?;
        if software {
            source.manifest.df_version = "different".into();
        } else {
            source.manifest.generation = 8;
        }
        assert!(j.commit(&mut source, &p, &c).is_err());
        assert_eq!(source.commits, 0);
        assert_eq!(
            j.reconcile(&mut source, &p, &c)?.state(),
            DurableJobState::Indeterminate
        );
        assert_eq!(source.queries, 0);
        assert!(j.commit(&mut source, &p, &c).is_err());
    }
    Ok(())
}

#[test]
fn native_unknown_is_immutable_even_if_a_later_terminal_hash_is_self_consistent() -> Result<()> {
    let (mut j, mut source) = setup()?;
    let p = plan()?;
    let c = context()?;
    j.prepare(&mut source, &p, &c)?;
    source.commit_state = Some(SuspensionState::Unknown);
    let unknown = j.commit(&mut source, &p, &c)?;
    source.query_state = Some(SuspensionState::Applied);
    assert!(j.reconcile(&mut source, &p, &c).is_err());
    assert!(source.fenced);
    assert_eq!(j.lookup(p.key(), &c)?, Some(&unknown));
    assert!(j.commit(&mut source, &p, &c).is_err());
    Ok(())
}

#[test]
fn refusal_and_observed_not_applied_are_distinct_durable_terminal_outcomes() -> Result<()> {
    for (native_state, state) in [
        (SuspensionState::Refused, DurableJobState::Refused),
        (SuspensionState::NotApplied, DurableJobState::NotApplied),
    ] {
        let (mut j, mut source) = setup()?;
        let p = plan()?;
        let c = context()?;
        j.prepare(&mut source, &p, &c)?;
        source.commit_state = Some(native_state);
        let record = j.commit(&mut source, &p, &c)?;
        assert_eq!(record.state(), state);
        assert!(record.effect().receipt().is_some());
        let opened = reopen(j.storage.bytes.into_inner())?;
        assert_eq!(opened.lookup(p.key(), &c)?, Some(&record));
    }
    Ok(())
}

#[test]
fn cancellation_is_local_durable_and_impossible_after_dispatch_starts() -> Result<()> {
    let (mut j, mut source) = setup()?;
    let p = plan()?;
    let c = context()?;
    j.prepare(&mut source, &p, &c)?;
    let cancelled = j.cancel_before_dispatch(&p, &c)?;
    assert_eq!(cancelled.state(), DurableJobState::CancelledBeforeDispatch);
    assert_eq!(j.commit(&mut source, &p, &c)?, cancelled);
    assert_eq!(source.commits, 0);
    let mut opened = reopen(j.storage.bytes.into_inner())?;
    assert_eq!(opened.commit(&mut source, &p, &c)?, cancelled);
    assert_eq!(source.commits, 0);
    let (mut j, mut source) = setup()?;
    j.prepare(&mut source, &p, &c)?;
    source.commit_state = None;
    j.commit(&mut source, &p, &c)?;
    assert!(j.cancel_before_dispatch(&p, &c).is_err());
    Ok(())
}

#[test]
fn authority_lineage_tick_budget_and_key_conflicts_fail_before_native_work() -> Result<()> {
    let (mut j, mut source) = setup()?;
    let p = plan()?;
    let c = context()?;
    for case in 0..7 {
        let mut bad = c.clone();
        match case {
            0 => bad.grants.clear(),
            1 => bad.cancellation_requested = true,
            2 => bad.anchor.fortress_id = FortressId::new(4),
            3 => bad.anchor.tick = GameTick(0),
            4 => bad.budget.max_bytes = 1,
            5 => bad.grants[0].remaining_uses = Some(1),
            _ => bad.grants[0].expires_at_tick = Some(GameTick(0)),
        }
        assert!(j.prepare(&mut source, &p, &bad).is_err());
    }
    assert_eq!(source.prepares, 0);
    j.prepare(&mut source, &p, &c)?;
    let other = SuspensionPlan::new(p.observation().clone(), p.key(), false)?;
    assert!(j.prepare(&mut source, &other, &c).is_err());
    assert!(j.commit(&mut source, &other, &c).is_err());
    assert_eq!((source.prepares, source.commits), (1, 0));
    Ok(())
}

#[test]
fn torn_dispatch_frame_is_refused_without_truncation_or_native_dispatch() -> Result<()> {
    let (mut j, mut source) = setup()?;
    let p = plan()?;
    let c = context()?;
    j.prepare(&mut source, &p, &c)?;
    let before = j.length as usize;
    j.storage.write_limit = Some(before + 19);
    assert!(j.commit(&mut source, &p, &c).is_err());
    assert!(j.poisoned());
    assert_eq!(source.commits, 0);
    let bytes = j.storage.bytes.into_inner();
    assert_eq!(bytes.len(), before + 19);
    assert!(reopen(bytes).is_err());
    Ok(())
}

#[test]
fn every_byte_corruption_and_every_incomplete_frame_prefix_fails_replay() -> Result<()> {
    let (mut j, mut source) = setup()?;
    let p = plan()?;
    let c = context()?;
    j.prepare(&mut source, &p, &c)?;
    j.commit(&mut source, &p, &c)?;
    let bytes = j.storage.bytes.into_inner();
    for offset in 0..bytes.len() {
        let mut corrupt = bytes.clone();
        corrupt[offset] ^= 1;
        assert!(reopen(corrupt).is_err(), "offset {offset}");
    }
    let mut boundaries = vec![HEADER_BYTES];
    let mut offset = HEADER_BYTES;
    while offset < bytes.len() {
        let length = u32::from_be_bytes(
            bytes[offset + 8..offset + 12]
                .try_into()
                .map_err(|_| exhausted())?,
        ) as usize;
        offset += FRAME_PREFIX + length + 40;
        boundaries.push(offset);
    }
    for length in 0..bytes.len() {
        if !boundaries.contains(&length) {
            assert!(reopen(bytes[..length].to_vec()).is_err(), "prefix {length}");
        }
    }
    Ok(())
}

#[test]
fn complete_history_is_credential_free_and_read_only_recovery_cannot_dispatch() -> Result<()> {
    let (mut j, mut source) = setup()?;
    let p = plan()?;
    let c = context()?;
    j.prepare(&mut source, &p, &c)?;
    let bytes = j.storage.bytes.into_inner();
    assert!(!bytes.windows(32).any(|v| v == &[b't'; 32]));
    assert!(!bytes.windows(16).any(|v| v == &[b'n'; 16]));
    let mut read_context = c.clone();
    read_context
        .grants
        .retain(|g| g.capability == Capability::Query);
    let mut read =
        JobControlJournal::open_read_only(Memory::new(bytes, Trace::default()), &read_context)?;
    assert_eq!(read.unresolved(&read_context)?.len(), 1);
    assert!(read.commit(&mut source, &p, &c).is_err());
    assert!(read.reconcile(&mut source, &p, &read_context).is_err());
    assert_eq!((source.commits, source.queries), (0, 0));
    Ok(())
}

#[test]
fn capacity_is_reserved_before_native_prepare_or_dispatch() -> Result<()> {
    let (mut j, mut source) = setup()?;
    let p = plan()?;
    let c = context()?;
    j.transitions = MAX_TRANSITIONS - 2;
    assert!(j.prepare(&mut source, &p, &c).is_err());
    assert_eq!(source.prepares, 0);
    j.transitions = 0;
    j.prepare(&mut source, &p, &c)?;
    j.transitions = MAX_TRANSITIONS - 1;
    assert!(j.commit(&mut source, &p, &c).is_err());
    assert_eq!(source.commits, 0);
    Ok(())
}

#[test]
fn rehashed_rollback_and_identity_substitution_frames_are_rejected() -> Result<()> {
    let (mut j, mut source) = setup()?;
    let p = plan()?;
    let c = context()?;
    let prepared = j.prepare(&mut source, &p, &c)?;
    let mut started = prepared.clone();
    started.state = DurableJobState::DispatchStarted;
    let mut unknown = prepared.clone();
    unknown.state = DurableJobState::Indeterminate;
    unknown.effect = native(&p, SuspensionState::Unknown)?;
    let mut applied = prepared.clone();
    applied.state = DurableJobState::Applied;
    applied.effect = native(&p, SuspensionState::Applied)?;
    let mut cancelled = prepared.clone();
    cancelled.state = DurableJobState::CancelledBeforeDispatch;
    let mut software = started.clone();
    software.manifest.df_version = "substituted".into();
    for records in [
        vec![prepared.clone(), started.clone(), prepared.clone()],
        vec![prepared.clone(), started.clone(), cancelled.clone()],
        vec![
            prepared.clone(),
            started.clone(),
            unknown.clone(),
            applied.clone(),
        ],
        vec![prepared.clone(), cancelled, started.clone()],
        vec![prepared.clone(), started, applied, prepared.clone()],
        vec![prepared, software],
    ] {
        let mut bytes = j.storage.bytes.get_ref()[..HEADER_BYTES].to_vec();
        let mut previous = Digest32::of_bytes(&bytes[..48]);
        for (index, record) in records.iter().enumerate() {
            let body = encode_record(record)?;
            let mut prefix = FRAME.to_vec();
            prefix.extend_from_slice(&(body.len() as u32).to_be_bytes());
            prefix.extend_from_slice(&(index as u64 + 1).to_be_bytes());
            prefix.extend_from_slice(previous.as_bytes());
            previous = frame_hash(j.id, &prefix, &body);
            bytes.extend(prefix);
            bytes.extend(body);
            bytes.extend_from_slice(previous.as_bytes());
            bytes.extend_from_slice(FOOTER);
        }
        assert!(reopen(bytes).is_err());
    }
    Ok(())
}
