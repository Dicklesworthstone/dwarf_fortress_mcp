use super::*;
use super::super::{DurableJobRecord, JobSuspensionSource};
use crate::job_suspension::{JobObservation, SuspensionEffect, SuspensionPlan};
use crate::job_suspension::rpc::JobControlManifest;
use dfmcp_core::{Capability, CapabilityGrant, CapabilityScope, ErrorCode, GameTick,
    ObservationCursor, RequestId, RiskTier, SessionId, StateAnchor, WorkBudget};
use std::cell::Cell;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::time::Duration;

#[derive(Default)]
struct Memory { bytes: Cursor<Vec<u8>>, invalid: Cell<bool> }
impl Read for Memory { fn read(&mut self, out: &mut [u8]) -> io::Result<usize> { self.bytes.read(out) } }
impl Write for Memory {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> { self.bytes.write(bytes) }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
impl Seek for Memory { fn seek(&mut self, from: SeekFrom) -> io::Result<u64> { self.bytes.seek(from) } }
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> { Ok(()) }
    fn truncate(&mut self, _: u64) -> io::Result<()> { Err(io::Error::other("no repair")) }
    fn validate_identity(&self) -> io::Result<()> {
        if self.invalid.get() { Err(io::Error::other("custody lost")) } else { Ok(()) }
    }
}
fn observation() -> Result<JobObservation> {
    let raw = include_str!("../../tests/fixtures/job_suspension_observation_v1_9.hex").trim();
    let mut bytes = Vec::new();
    for offset in (0..raw.len()).step_by(2) {
        bytes.push(u8::from_str_radix(&raw[offset..offset + 2], 16)
            .map_err(|_| corrupt("fixture hex"))?);
    }
    JobObservation::decode(&bytes)
}
fn context() -> Result<OperationContext> {
    let o = observation()?;
    Ok(OperationContext { session_id: SessionId::new(41), request_id: RequestId::new(1),
        anchor: StateAnchor { fortress_id: super::super::job_fortress_id(&o),
            cursor: ObservationCursor { epoch: o.generation(), sequence: o.sequence() },
            tick: GameTick(o.tick()), state_hash: o.witness() },
        grants: [Capability::Query, Capability::ConfigureProduction].into_iter().map(|capability|
            CapabilityGrant { capability, scope: CapabilityScope::default(), max_risk: RiskTier::Reversible,
                expires_at_tick: None, remaining_uses: None }).collect(),
        budget: WorkBudget { max_wall_millis: 10_000, ..WorkBudget::default() }, cancellation_requested: false })
}
fn record(key: &str) -> Result<DurableJobRecord> {
    let plan = SuspensionPlan::new(observation()?, key, true)?;
    let o = plan.observation();
    let manifest = JobControlManifest { generation: o.generation(), df_version: "df".into(), dfhack_version: "dfhack".into() };
    let mut bytes = b"DFMJSE19".to_vec();
    for n in [o.generation(), o.sequence(), o.tick()] { bytes.extend_from_slice(&n.to_be_bytes()); }
    bytes.extend_from_slice(&o.job_id().to_be_bytes()); bytes.push(1);
    bytes.extend_from_slice(o.witness().as_bytes()); bytes.extend_from_slice(plan.digest().as_bytes());
    bytes.extend_from_slice(plan.prepare_token()); bytes.extend_from_slice(&[0; 75]);
    bytes.extend_from_slice(&(key.len() as u16).to_be_bytes()); bytes.extend_from_slice(key.as_bytes());
    let effect = SuspensionEffect::decode(&bytes, &plan)?;
    Ok(DurableJobRecord { plan, effect, manifest, state: DurableJobState::Prepared })
}
fn populated() -> Result<(JobControlJournal<Memory>, OperationContext)> {
    let c = context()?;
    let mut j = JobControlJournal::open(Memory::default(), &c, true)?;
    for key in ["c", "a", "b"] { j.append(record(key)?, &mut Budget::new(&c)?)?; }
    Ok((j, c))
}

#[test]
fn complete_keyset_pages_include_cancelled_terminal_work() -> Result<()> {
    let (mut j, c) = populated()?;
    let plan = record("b")?.plan;
    j.cancel_before_dispatch(&plan, &c)?;
    let s = j.summary(&c)?;
    assert_eq!((s.records, s.prepared, s.unresolved, s.terminal), (3, 2, 0, 1));
    let first = j.records_page(s.head, None, 2, &c)?;
    assert_eq!(first.records.iter().map(|r| r.plan().key()).collect::<Vec<_>>(), ["a", "b"]);
    assert_eq!(first.next_after.as_deref(), Some("b"));
    assert_eq!(first.records[1].state(), DurableJobState::CancelledBeforeDispatch);
    let last = j.records_page(s.head, first.next_after.as_deref(), 2, &c)?;
    assert_eq!(last.records[0].plan().key(), "c"); assert!(last.next_after.is_none());
    Ok(())
}
#[test]
fn restart_preserves_page_generation_and_all_records() -> Result<()> {
    let (j, c) = populated()?;
    let summary = j.summary(&c)?;
    let expected = j.records_page(summary.head, None, 64, &c)?;
    let restored = JobControlJournal::open_read_only(j.storage, &c)?;
    assert_eq!(restored.records_page(summary.head, None, 64, &c)?, expected);
    assert!(restored.summary(&c)?.read_only); Ok(())
}
#[test]
fn changed_head_unknown_key_and_page_budget_fail_closed() -> Result<()> {
    let (mut j, c) = populated()?; let head = j.summary(&c)?.head;
    j.cancel_before_dispatch(&record("a")?.plan, &c)?;
    assert!(matches!(j.records_page(head, Some("a"), 1, &c), Err(e) if e.code == ErrorCode::StaleAnchor));
    let head = j.summary(&c)?.head;
    assert!(j.records_page(head, Some("absent"), 1, &c).is_err());
    for limit in [0, 65] { assert!(j.records_page(head, None, limit, &c).is_err()); }
    let mut tiny = c.clone(); tiny.budget.max_bytes = 1;
    assert!(j.records_page(head, None, 1, &tiny).is_err());
    tiny = c.clone(); tiny.budget.max_entities = 1;
    assert!(j.records_page(head, None, 2, &tiny).is_err()); Ok(())
}
#[test]
fn cached_discovery_and_terminal_replays_recheck_custody() -> Result<()> {
    let (mut j, c) = populated()?; let head = j.summary(&c)?.head; let plan = record("a")?.plan;
    j.cancel_before_dispatch(&plan, &c)?;
    j.storage.invalid.set(true);
    assert!(j.summary(&c).is_err()); assert!(j.lookup("a", &c).is_err());
    assert!(j.unresolved(&c).is_err()); assert!(j.records_page(head, None, 1, &c).is_err());
    assert!(j.cancel_before_dispatch(&plan, &c).is_err()); Ok(())
}
struct Native { manifest: JobControlManifest, writes: usize, queries: usize }
impl JobSuspensionSource for Native {
    fn manifest(&self) -> &JobControlManifest { &self.manifest }
    fn fence(&mut self) {}
    fn prepare(&mut self, _: &SuspensionPlan, _: Duration) -> Result<SuspensionEffect> {
        self.writes += 1; Err(corrupt("unexpected prepare"))
    }
    fn commit(&mut self, _: &SuspensionPlan, _: &SuspensionEffect, _: Duration) -> Result<SuspensionEffect> {
        self.writes += 1; Err(corrupt("unexpected commit"))
    }
    fn query(&mut self, _: &SuspensionPlan, _: Duration) -> Result<Option<SuspensionEffect>> { self.queries += 1; Ok(None) }
}
#[test]
fn query_only_reopen_can_reconcile_but_cannot_dispatch_or_create() -> Result<()> {
    let (j, mut c) = populated()?;
    c.grants.retain(|g| g.capability == Capability::Query);
    assert!(JobControlJournal::open_for_reconciliation(Memory::default(), &c).is_err());
    let mut j = JobControlJournal::open_for_reconciliation(j.storage, &c)?;
    let r = record("a")?;
    let mut n = Native { manifest: r.manifest, writes: 0, queries: 0 };
    assert!(j.prepare(&mut n, &r.plan, &c).is_err()); assert!(j.commit(&mut n, &r.plan, &c).is_err());
    assert!(j.cancel_before_dispatch(&r.plan, &c).is_err());
    assert_eq!(j.reconcile(&mut n, &r.plan, &c)?.state(), DurableJobState::Indeterminate);
    assert_eq!((n.writes, n.queries), (0, 1));
    assert_eq!(j.summary(&c)?.unresolved, 1); Ok(())
}
#[test]
fn current_query_authority_required_for_each_cached_result() -> Result<()> {
    let (j, mut c) = populated()?; let head = j.summary(&c)?.head;
    c.cancellation_requested = true;
    assert!(j.summary(&c).is_err()); assert!(j.records_page(head, None, 1, &c).is_err());
    c.cancellation_requested = false; c.grants.clear();
    assert!(j.summary(&c).is_err()); assert!(j.lookup("a", &c).is_err()); Ok(())
}
