use super::super::tests::{fixture, plan};
use super::super::{
    DigReason, DigRegion, append_text,
    rpc::{DigManifest, DigPreparation},
};
use super::*;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, GameTick, MapCoord, MapCuboid, ObservationCursor,
    RequestId, RiskTier, StateAnchor, WorkBudget,
};
use std::cell::RefCell;
use std::io::{Read, Seek, Write};
use std::rc::Rc;

#[derive(Default)]
struct Disk {
    bytes: Vec<u8>,
    syncs: usize,
    fail_sync: Option<usize>,
    write_left: Option<usize>,
    revoke_at_dispatch: bool,
    revoked: bool,
}
#[derive(Clone)]
struct Memory {
    disk: Rc<RefCell<Disk>>,
    position: usize,
}
impl Memory {
    fn new() -> Self {
        Self {
            disk: Rc::new(RefCell::new(Disk::default())),
            position: 0,
        }
    }
}
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let disk = self.disk.borrow();
        let n = out
            .len()
            .min(disk.bytes.len().saturating_sub(self.position));
        if n != 0 {
            out[..n].copy_from_slice(&disk.bytes[self.position..self.position + n]);
            self.position += n;
        }
        Ok(n)
    }
}
impl Seek for Memory {
    fn seek(&mut self, value: SeekFrom) -> io::Result<u64> {
        let n = match value {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::End(n) => self.disk.borrow().bytes.len() as i128 + i128::from(n),
            SeekFrom::Current(n) => self.position as i128 + i128::from(n),
        };
        self.position = usize::try_from(n).map_err(|_| io::Error::other("invalid seek"))?;
        Ok(self.position as u64)
    }
}
impl Write for Memory {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut disk = self.disk.borrow_mut();
        if self.position != disk.bytes.len() || disk.write_left == Some(0) {
            return Err(io::Error::other("injected write failure"));
        }
        let n = disk
            .write_left
            .map_or(bytes.len(), |left| left.min(bytes.len()));
        if let Some(left) = &mut disk.write_left {
            *left -= n;
        }
        disk.bytes.extend_from_slice(&bytes[..n]);
        self.position += n;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        let mut disk = self.disk.borrow_mut();
        disk.syncs += 1;
        if disk.revoke_at_dispatch
            && last_state(&disk.bytes) == Some(DigState::DispatchStarted as u8)
        {
            disk.revoked = true;
        }
        if disk.fail_sync == Some(disk.syncs) {
            Err(io::Error::other("injected sync failure"))
        } else {
            Ok(())
        }
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("repair forbidden"))
    }
}
fn last_state(bytes: &[u8]) -> Option<u8> {
    let n = u16::from_be_bytes(bytes.get(8..10)?.try_into().ok()?) as usize;
    let mut at = 74 + n;
    let mut last = None;
    while at < bytes.len() {
        let n = u32::from_be_bytes(bytes.get(at + 8..at + 12)?.try_into().ok()?) as usize;
        last = bytes.get(at + 52).copied();
        at += 92 + n;
    }
    last
}
fn scope() -> MapCuboid {
    MapCuboid {
        min: MapCoord::new(0, 0, 0),
        max: MapCoord::new(63, 63, 7),
    }
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
fn open(memory: Memory, mode: DigMode, create: bool) -> Result<DigJournal<Memory>> {
    DigJournal::open(
        memory,
        &context()?,
        mode,
        Some(binding()?),
        create.then_some([7; 32]),
    )
}
fn effect(p: &DigPlan, phase: DigPhase) -> Result<DigEffect> {
    let mut out = b"DFMDGE16".to_vec();
    for n in [
        p.before().generation(),
        p.before().sequence(),
        p.before().tick(),
    ] {
        out.extend_from_slice(&n.to_be_bytes());
    }
    p.before().region().append(&mut out);
    out.push(u8::from(p.allow_hidden_neighbors()));
    out.extend_from_slice(p.before().witness().as_bytes());
    out.extend_from_slice(p.digest().as_bytes());
    out.extend_from_slice(p.token());
    out.push(match phase {
        DigPhase::Prepared => 0,
        DigPhase::Unknown => 1,
        DigPhase::Designated => 2,
        DigPhase::Refused => 4,
    });
    out.push(if phase == DigPhase::Refused { 2 } else { 0 });
    out.push(u8::from(phase == DigPhase::Designated));
    let count = if phase == DigPhase::Designated {
        p.before().region().target_count()
    } else {
        0
    };
    out.extend_from_slice(&count.to_be_bytes());
    let witness = if phase == DigPhase::Designated {
        p.before().expected_witness()?
    } else {
        Digest32::ZERO
    };
    out.extend_from_slice(witness.as_bytes());
    let mut proof = p.before().generation().to_be_bytes().to_vec();
    append_text(&mut proof, p.key());
    proof.extend_from_slice(p.digest().as_bytes());
    proof.extend_from_slice(p.token());
    proof.extend_from_slice(&out[133..172]);
    let receipt = if phase.terminal() {
        hash(b"dfmcp-dig-designation-receipt/1", &proof)
    } else {
        Digest32::ZERO
    };
    out.extend_from_slice(receipt.as_bytes());
    append_text(&mut out, p.key());
    DigEffect::decode(&out, p)
}
struct Source {
    binding: DigBinding,
    before: DigObservation,
    record: Option<DigEffect>,
    calls: Vec<DigStage>,
    permit: bool,
    replayed: bool,
    fail: Option<DigStage>,
    commits: usize,
    disk: Rc<RefCell<Disk>>,
}
impl Source {
    fn new(memory: &Memory) -> Result<Self> {
        Ok(Self {
            binding: binding()?,
            before: plan()?.before().clone(),
            record: None,
            calls: Vec::new(),
            permit: false,
            replayed: false,
            fail: None,
            commits: 0,
            disk: memory.disk.clone(),
        })
    }
    fn finish(&self, stage: DigStage) -> Result<()> {
        if self.fail == Some(stage) {
            Err(unknown("native"))
        } else {
            Ok(())
        }
    }
}
impl DigSource for Source {
    fn manifest(&self) -> &DigManifest {
        self.binding.manifest()
    }
    fn endpoint(&self) -> Option<std::net::SocketAddr> {
        Some(self.binding.endpoint())
    }
    fn observe(&mut self, region: DigRegion, _: &OperationContext) -> Result<DigObservation> {
        self.calls.push(DigStage::Observe);
        assert_eq!(region, self.before.region());
        self.finish(DigStage::Observe)?;
        Ok(self.before.clone())
    }
    fn prepare(&mut self, p: &DigPlan, _: &OperationContext) -> Result<DigPreparation> {
        self.calls.push(DigStage::Prepare);
        assert_eq!(
            last_state(&self.disk.borrow().bytes),
            Some(DigState::Intent as u8)
        );
        let value = match &self.record {
            Some(r) => r.clone(),
            None => effect(p, DigPhase::Prepared)?,
        };
        self.record = Some(value.clone());
        self.permit = !self.replayed;
        self.finish(DigStage::Prepare)?;
        DigPreparation::decode(value.canonical_bytes(), self.replayed, p)
    }
    fn commit(&mut self, p: &DigPlan, _: &OperationContext) -> Result<DigEffect> {
        self.calls.push(DigStage::Commit);
        assert_eq!(
            last_state(&self.disk.borrow().bytes),
            Some(DigState::DispatchStarted as u8)
        );
        if !self.permit {
            return Err(unknown(p.key()));
        }
        self.permit = false;
        self.commits += 1;
        let value = effect(p, DigPhase::Designated)?;
        self.record = Some(value.clone());
        self.finish(DigStage::Commit)?;
        Ok(value)
    }
    fn query(&mut self, _: &DigPlan, _: &OperationContext) -> Result<Option<DigEffect>> {
        self.calls.push(DigStage::Query);
        self.finish(DigStage::Query)?;
        Ok(self.record.clone())
    }
    fn cancel(&mut self, p: &DigPlan, _: &OperationContext) -> Result<DigEffect> {
        self.calls.push(DigStage::Cancel);
        assert_eq!(
            last_state(&self.disk.borrow().bytes),
            Some(DigState::CancelRequested as u8)
        );
        let value = match &self.record {
            Some(r) if r.phase().terminal() || r.phase() == DigPhase::Unknown => r.clone(),
            Some(_) => effect(p, DigPhase::Refused)?,
            None => return Err(unknown(p.key())),
        };
        self.permit = false;
        self.record = Some(value.clone());
        self.finish(DigStage::Cancel)?;
        Ok(value)
    }
}
struct Guard {
    disk: Rc<RefCell<Disk>>,
    calls: Vec<DigStage>,
}
impl Guard {
    fn new(m: &Memory) -> Self {
        Self {
            disk: m.disk.clone(),
            calls: Vec::new(),
        }
    }
}
impl DigGuard for Guard {
    fn check(&mut self, stage: DigStage, _: &DigPlan, _: &OperationContext) -> Result<()> {
        self.calls.push(stage);
        if self.disk.borrow().revoked {
            Err(error(ErrorCode::CapabilityDenied, "runtime revoked"))
        } else {
            Ok(())
        }
    }
}

fn hex32(value: &str) -> Result<Digest32> {
    check(value.len() == 64)?;
    let mut out = [0; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[2 * i..2 * i + 2], 16).map_err(|_| exhausted())?;
    }
    Ok(Digest32::from_bytes(out))
}

#[test]
fn intent_dispatch_and_terminal_sync_order_is_exercised() -> Result<()> {
    let m = Memory::new();
    let mut j = open(m.clone(), DigMode::Control, true)?;
    let mut n = Source::new(&m)?;
    let mut g = Guard::new(&m);
    let p = plan()?;
    let c = context()?;
    assert_eq!(
        j.id(),
        hex32("b0b5bf8cb76333f1c02ed8dd37555e75a894fcfb0a36a8235cf1d8a4fa8296c3")?
    );
    assert_eq!(
        j.prepare(&mut n, &p, &c, &mut g)?.state(),
        DigState::Prepared
    );
    assert_eq!(
        j.head(),
        hex32("1fdaa6967cefd71e0ea924af26482cdf37948ee0c964c1ecd47f8306e09a3830")?
    );
    assert!(j.list(&c, 8, None)?.records[0].dispatchable);
    let terminal = j.commit(&mut n, p.key(), p.digest(), &c, &mut g)?;
    assert_eq!(terminal.state(), DigState::Terminal);
    assert_eq!(
        j.head(),
        hex32("56b6382b026c680a84599ba28f4df38f6acc3e16c9c0838d6fdb134b4d383303")?
    );
    assert_eq!(
        terminal.effect().map(DigEffect::phase),
        Some(DigPhase::Designated)
    );
    assert_eq!(j.commit(&mut n, p.key(), p.digest(), &c, &mut g)?, terminal);
    assert_eq!(n.commits, 1);
    assert_eq!(j.list(&c, 8, None)?.unsettled_records, 0);
    let mut offline = open(m, DigMode::Offline, false)?;
    assert_eq!(offline.get(p.key(), p.digest(), &c)?, terminal);
    Ok(())
}
#[test]
fn recovered_and_queried_preparations_never_become_committable() -> Result<()> {
    let m = Memory::new();
    let p = plan()?;
    let c = context()?;
    let mut n = Source::new(&m)?;
    let mut g = Guard::new(&m);
    let mut j = open(m.clone(), DigMode::Control, true)?;
    j.prepare(&mut n, &p, &c, &mut g)?;
    for mode in [DigMode::Recover, DigMode::Control] {
        let mut recovered = open(m.clone(), mode, false)?;
        assert!(!recovered.list(&c, 8, None)?.records[0].dispatchable);
        assert!(
            recovered
                .commit(&mut n, p.key(), p.digest(), &c, &mut g)
                .is_err()
        );
    }
    assert_eq!(
        j.reconcile(&mut n, p.key(), p.digest(), &c, &mut g)?
            .state(),
        DigState::Tracking
    );
    assert!(j.commit(&mut n, p.key(), p.digest(), &c, &mut g).is_err());
    assert_eq!(n.commits, 0);
    Ok(())
}
#[test]
fn lost_commit_recovers_by_query_only_and_terminal_lookup_is_offline() -> Result<()> {
    let m = Memory::new();
    let p = plan()?;
    let c = context()?;
    let mut j = open(m.clone(), DigMode::Control, true)?;
    let mut n = Source::new(&m)?;
    let mut g = Guard::new(&m);
    j.prepare(&mut n, &p, &c, &mut g)?;
    n.fail = Some(DigStage::Commit);
    assert!(j.commit(&mut n, p.key(), p.digest(), &c, &mut g).is_err());
    assert_eq!(
        j.get(p.key(), p.digest(), &c)?.state(),
        DigState::DispatchStarted
    );
    let mut recovered = open(m.clone(), DigMode::Recover, false)?;
    n.fail = None;
    n.permit = false;
    assert_eq!(
        recovered
            .reconcile(&mut n, p.key(), p.digest(), &c, &mut g)?
            .state(),
        DigState::Terminal
    );
    let calls = n.calls.len();
    let mut offline = open(m, DigMode::Offline, false)?;
    offline.reconcile(&mut n, p.key(), p.digest(), &c, &mut g)?;
    offline.cancel(&mut n, p.key(), p.digest(), &c, &mut g)?;
    assert_eq!(n.calls.len(), calls);
    assert_eq!(n.commits, 1);
    Ok(())
}
#[test]
fn lost_prepare_and_replayed_preparation_can_only_be_recovered_or_cancelled() -> Result<()> {
    for replayed in [false, true] {
        let m = Memory::new();
        let p = plan()?;
        let c = context()?;
        let mut j = open(m.clone(), DigMode::Control, true)?;
        let mut n = Source::new(&m)?;
        let mut g = Guard::new(&m);
        n.replayed = replayed;
        if !replayed {
            n.fail = Some(DigStage::Prepare);
        }
        let prepared = j.prepare(&mut n, &p, &c, &mut g);
        assert_eq!(prepared.is_ok(), replayed);
        n.fail = None;
        let mut recovered = open(m, DigMode::Recover, false)?;
        recovered.reconcile(&mut n, p.key(), p.digest(), &c, &mut g)?;
        assert!(
            recovered
                .commit(&mut n, p.key(), p.digest(), &c, &mut g)
                .is_err()
        );
        let cancelled = recovered.cancel(&mut n, p.key(), p.digest(), &c, &mut g)?;
        assert_eq!(
            cancelled.effect().map(DigEffect::reason),
            Some(DigReason::CancelledBeforeDispatch)
        );
        assert_eq!(n.commits, 0);
    }
    Ok(())
}
#[test]
fn unresolved_work_fences_new_keys_and_native_unknown_is_absorbing() -> Result<()> {
    let m = Memory::new();
    let p = plan()?;
    let c = context()?;
    let mut j = open(m.clone(), DigMode::Control, true)?;
    let mut n = Source::new(&m)?;
    let mut g = Guard::new(&m);
    j.prepare(&mut n, &p, &c, &mut g)?;
    let other = DigPlan::new("another", false, p.before().clone())?;
    assert!(j.prepare(&mut n, &other, &c, &mut g).is_err());
    n.record = Some(effect(&p, DigPhase::Unknown)?);
    assert!(
        j.reconcile(&mut n, p.key(), p.digest(), &c, &mut g)?
            .permanent_unknown()
    );
    let calls = n.calls.len();
    n.record = Some(effect(&p, DigPhase::Designated)?);
    j.reconcile(&mut n, p.key(), p.digest(), &c, &mut g)?;
    j.cancel(&mut n, p.key(), p.digest(), &c, &mut g)?;
    assert_eq!(n.calls.len(), calls);
    assert!(j.prepare(&mut n, &other, &c, &mut g).is_err());
    let mut reopened = open(m, DigMode::Control, false)?;
    assert!(reopened.prepare(&mut n, &other, &c, &mut g).is_err());
    Ok(())
}
#[test]
fn cancellation_settles_native_preparation_and_allows_a_distinct_region() -> Result<()> {
    let m = Memory::new();
    let p = plan()?;
    let c = context()?;
    let mut j = open(m.clone(), DigMode::Control, true)?;
    let mut n = Source::new(&m)?;
    let mut g = Guard::new(&m);
    j.prepare(&mut n, &p, &c, &mut g)?;
    j.cancel(&mut n, p.key(), p.digest(), &c, &mut g)?;
    let mut raw = fixture("observation")?;
    raw[48..52].copy_from_slice(&32u32.to_be_bytes());
    let next = DigPlan::new("next-region", false, DigObservation::decode(&raw)?)?;
    n.before = next.before().clone();
    n.record = None;
    assert_eq!(
        j.prepare(&mut n, &next, &c, &mut g)?.state(),
        DigState::Prepared
    );
    assert_eq!(j.list(&c, 8, None)?.total_records, 2);
    assert!(j.get(p.key(), p.digest(), &c)?.state().terminal());
    Ok(())
}
#[test]
fn intent_sync_failure_prevents_native_prepare_and_fences_storage() -> Result<()> {
    let m = Memory::new();
    let p = plan()?;
    let c = context()?;
    let mut j = open(m.clone(), DigMode::Control, true)?;
    let mut n = Source::new(&m)?;
    let mut g = Guard::new(&m);
    {
        let mut d = m.disk.borrow_mut();
        d.fail_sync = Some(d.syncs + 1);
    }
    assert!(j.prepare(&mut n, &p, &c, &mut g).is_err());
    assert!(j.fenced());
    assert!(!n.calls.contains(&DigStage::Prepare));
    assert_eq!(n.commits, 0);
    m.disk.borrow_mut().fail_sync = None;
    let mut reopened = open(m, DigMode::Recover, false)?;
    assert_eq!(
        reopened.get(p.key(), p.digest(), &c)?.state(),
        DigState::Intent
    );
    Ok(())
}
#[test]
fn dispatch_sync_failure_and_post_sync_policy_revocation_prevent_native_commit() -> Result<()> {
    for revoke in [false, true] {
        let m = Memory::new();
        let p = plan()?;
        let c = context()?;
        let mut j = open(m.clone(), DigMode::Control, true)?;
        let mut n = Source::new(&m)?;
        let mut g = Guard::new(&m);
        j.prepare(&mut n, &p, &c, &mut g)?;
        {
            let mut d = m.disk.borrow_mut();
            if revoke {
                d.revoke_at_dispatch = true;
            } else {
                d.fail_sync = Some(d.syncs + 1);
            }
        }
        assert!(j.commit(&mut n, p.key(), p.digest(), &c, &mut g).is_err());
        assert!(!n.calls.contains(&DigStage::Commit));
        assert_eq!(n.commits, 0);
        {
            let mut d = m.disk.borrow_mut();
            d.fail_sync = None;
            d.revoked = false;
            d.revoke_at_dispatch = false;
        }
        let mut recovered = open(m, DigMode::Control, false)?;
        assert_eq!(
            recovered.get(p.key(), p.digest(), &c)?.state(),
            DigState::DispatchStarted
        );
        assert!(
            recovered
                .commit(&mut n, p.key(), p.digest(), &c, &mut g)
                .is_err()
        );
    }
    Ok(())
}
#[test]
fn terminal_sync_failure_is_unknown_to_caller_but_complete_frame_recovers() -> Result<()> {
    let m = Memory::new();
    let p = plan()?;
    let c = context()?;
    let mut j = open(m.clone(), DigMode::Control, true)?;
    let mut n = Source::new(&m)?;
    let mut g = Guard::new(&m);
    j.prepare(&mut n, &p, &c, &mut g)?;
    {
        let mut d = m.disk.borrow_mut();
        d.fail_sync = Some(d.syncs + 2);
    }
    assert!(j.commit(&mut n, p.key(), p.digest(), &c, &mut g).is_err());
    assert!(j.fenced());
    assert_eq!(n.commits, 1);
    m.disk.borrow_mut().fail_sync = None;
    let mut recovered = open(m, DigMode::Recover, false)?;
    assert_eq!(
        recovered.get(p.key(), p.digest(), &c)?.state(),
        DigState::Terminal
    );
    let calls = n.calls.len();
    recovered.reconcile(&mut n, p.key(), p.digest(), &c, &mut g)?;
    assert_eq!(calls, n.calls.len());
    Ok(())
}
#[test]
fn torn_appends_and_changed_bytes_are_never_repaired() -> Result<()> {
    let m = Memory::new();
    let p = plan()?;
    let c = context()?;
    let mut j = open(m.clone(), DigMode::Control, true)?;
    let mut n = Source::new(&m)?;
    let mut g = Guard::new(&m);
    m.disk.borrow_mut().write_left = Some(23);
    assert!(j.prepare(&mut n, &p, &c, &mut g).is_err());
    let raw = m.disk.borrow().bytes.clone();
    m.disk.borrow_mut().write_left = None;
    assert!(open(m.clone(), DigMode::Recover, false).is_err());
    assert_eq!(m.disk.borrow().bytes, raw);
    assert!(!n.calls.contains(&DigStage::Prepare));
    let fresh = Memory::new();
    let mut other = open(fresh.clone(), DigMode::Control, true)?;
    fresh.disk.borrow_mut().bytes[12] ^= 1;
    assert!(other.list(&c, 8, None).is_err());
    assert!(other.fenced());
    Ok(())
}
#[test]
fn missing_native_retention_never_proves_nonapplication() -> Result<()> {
    let m = Memory::new();
    let p = plan()?;
    let c = context()?;
    let mut j = open(m.clone(), DigMode::Control, true)?;
    let mut n = Source::new(&m)?;
    let mut g = Guard::new(&m);
    j.prepare(&mut n, &p, &c, &mut g)?;
    n.record = None;
    let head = j.head();
    assert!(
        j.reconcile(&mut n, p.key(), p.digest(), &c, &mut g)
            .is_err()
    );
    assert_eq!(j.head(), head);
    assert!(!j.list(&c, 8, None)?.records[0].dispatchable);
    assert!(j.cancel(&mut n, p.key(), p.digest(), &c, &mut g).is_err());
    assert_eq!(j.list(&c, 8, None)?.unsettled_records, 1);
    assert_eq!(n.commits, 0);
    Ok(())
}
#[test]
fn current_authority_budget_scope_and_confirmation_are_required() -> Result<()> {
    let m = Memory::new();
    let p = plan()?;
    let c = context()?;
    let mut j = open(m.clone(), DigMode::Control, true)?;
    let mut n = Source::new(&m)?;
    let mut g = Guard::new(&m);
    for fault in 0..6 {
        let mut denied = c.clone();
        match fault {
            0 => denied.cancellation_requested = true,
            1 => denied
                .grants
                .retain(|g| g.capability != Capability::Designate),
            2 => {
                for grant in &mut denied.grants {
                    grant.remaining_uses = Some(1);
                }
            }
            3 => {
                for grant in &mut denied.grants {
                    grant.expires_at_tick = Some(GameTick(0));
                }
            }
            4 => denied.budget.max_bytes = RPC_BYTES,
            _ => denied.session_id = SessionId::new(99),
        }
        assert!(j.prepare(&mut n, &p, &denied, &mut g).is_err());
    }
    assert!(n.calls.is_empty());
    j.prepare(&mut n, &p, &c, &mut g)?;
    let calls = n.calls.len();
    assert!(
        j.commit(&mut n, p.key(), Digest32::ZERO, &c, &mut g)
            .is_err()
    );
    assert_eq!(n.calls.len(), calls);
    let mut narrow = c.clone();
    for grant in &mut narrow.grants {
        grant.scope.map_area = Some(p.before().region().halo());
    }
    assert!(j.list(&narrow, 8, None).is_err());
    Ok(())
}
#[test]
fn source_drift_and_out_of_scope_plans_never_touch_native_state() -> Result<()> {
    let m = Memory::new();
    let p = plan()?;
    let c = context()?;
    let mut j = open(m.clone(), DigMode::Control, true)?;
    let mut n = Source::new(&m)?;
    let mut g = Guard::new(&m);
    let mut outside = fixture("observation")?;
    outside[36..40].copy_from_slice(&128u32.to_be_bytes());
    outside[48..52].copy_from_slice(&64u32.to_be_bytes());
    let outside = DigPlan::new("outside", false, DigObservation::decode(&outside)?)?;
    assert!(j.prepare(&mut n, &outside, &c, &mut g).is_err());
    assert!(n.calls.is_empty());
    let mut raw = fixture("observation")?;
    raw[8..16].copy_from_slice(&8u64.to_be_bytes());
    n.before = DigObservation::decode(&raw)?;
    n.binding = DigBinding::new(
        n.binding.endpoint(),
        DigManifest {
            generation: 8,
            ..n.binding.manifest().clone()
        },
        &n.before,
        scope(),
    )?;
    assert!(j.prepare(&mut n, &p, &c, &mut g).is_err());
    assert!(n.calls.is_empty());
    let expected = DigBinding::new(
        "127.0.0.1:5001".parse().map_err(|_| exhausted())?,
        binding()?.manifest().clone(),
        p.before(),
        scope(),
    )?;
    assert!(DigJournal::open(m, &c, DigMode::Recover, Some(expected), None).is_err());
    Ok(())
}
#[test]
fn discovery_cursors_bind_session_and_exact_head() -> Result<()> {
    let m = Memory::new();
    let p = plan()?;
    let c = context()?;
    let mut j = open(m.clone(), DigMode::Control, true)?;
    let mut n = Source::new(&m)?;
    let mut g = Guard::new(&m);
    j.prepare(&mut n, &p, &c, &mut g)?;
    j.cancel(&mut n, p.key(), p.digest(), &c, &mut g)?;
    let next = DigPlan::new("next", false, p.before().clone())?;
    n.record = None;
    j.prepare(&mut n, &next, &c, &mut g)?;
    let page = j.list(&c, 1, None)?;
    let cursor = page.continuation.ok_or_else(exhausted)?;
    assert_eq!(j.list(&c, 1, Some(&cursor))?.records.len(), 1);
    assert!(j.list(&c, 2, Some(&cursor)).is_err());
    j.cancel(&mut n, next.key(), next.digest(), &c, &mut g)?;
    assert!(j.list(&c, 1, Some(&cursor)).is_err());
    let mut other = c.clone();
    other.session_id = SessionId::new(2);
    let mut reopened = DigJournal::open(m, &other, DigMode::Offline, None, None)?;
    assert!(reopened.list(&other, 1, Some(&cursor)).is_err());
    Ok(())
}
#[test]
fn recovery_modes_never_create_or_prepare_and_offline_never_syncs() -> Result<()> {
    for mode in [DigMode::Offline, DigMode::Recover] {
        assert!(open(Memory::new(), mode, true).is_err());
    }
    let m = Memory::new();
    let c = context()?;
    let p = plan()?;
    let _j = open(m.clone(), DigMode::Control, true)?;
    let syncs = m.disk.borrow().syncs;
    let mut offline = open(m.clone(), DigMode::Offline, false)?;
    offline.list(&c, 8, None)?;
    assert_eq!(m.disk.borrow().syncs, syncs);
    let mut n = Source::new(&m)?;
    let mut g = Guard::new(&m);
    assert!(offline.prepare(&mut n, &p, &c, &mut g).is_err());
    let mut recovered = open(m, DigMode::Recover, false)?;
    assert!(recovered.prepare(&mut n, &p, &c, &mut g).is_err());
    assert!(n.calls.is_empty());
    Ok(())
}
#[test]
fn reservation_keeps_room_for_recovery_before_any_native_work() -> Result<()> {
    let m = Memory::new();
    let p = plan()?;
    let c = context()?;
    let mut j = open(m.clone(), DigMode::Control, true)?;
    let mut n = Source::new(&m)?;
    let mut g = Guard::new(&m);
    j.events = MAX_EVENTS - 3;
    assert!(j.prepare(&mut n, &p, &c, &mut g).is_err());
    assert!(n.calls.is_empty());
    Ok(())
}
