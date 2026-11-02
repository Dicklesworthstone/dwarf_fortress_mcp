//! Real descriptor-pinned files plus bounded fault/delay instrumentation.
use super::*;
use crate::tree_harvest::tests::{capture, plan, progress, record};
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, ObservationCursor, RequestId, StateAnchor, WorkBudget,
};
use std::io::{self, Read, Seek, Write};
use std::os::unix::fs::DirBuilderExt;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::thread;
static NEXT: AtomicU64 = AtomicU64::new(1);
pub(crate) fn context() -> Result<OperationContext> {
    let cap = capture()?;
    let effect = cap.effect_scope()?;
    let read = MapCuboid::new(
        dfmcp_core::MapCoord::new(0, 0, 0),
        dfmcp_core::MapCoord::new(63, 63, 7),
    )?;
    Ok(OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: StateAnchor {
            fortress_id: cap.source.fortress_id(),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(cap.tick),
            state_hash: cap.witness(),
        },
        budget: WorkBudget {
            max_wall_millis: 5000,
            max_bytes: 512 * 1024 * 1024,
            ..WorkBudget::CONSERVATIVE_DEFAULT
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
                fortress_id: Some(cap.source.fortress_id()),
                map_area: Some(
                    if matches!(capability, Capability::Query | Capability::Observe) {
                        read
                    } else {
                        effect
                    },
                ),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::Irreversible,
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
        cancellation_requested: false,
    })
}
pub(crate) fn binding(endpoint: std::net::SocketAddr) -> Result<Binding> {
    let c = capture()?;
    Ok(Binding {
        endpoint,
        source: c.source.clone(),
        generation: c.generation,
        dimensions: c.dimensions,
        scope: c.effect_scope()?,
        df_version: "test-df".into(),
        dfhack_version: "test-dfhack".into(),
    })
}
pub(crate) struct Directory(pub std::path::PathBuf);
impl Directory {
    pub(crate) fn new() -> Result<Self> {
        let p = std::env::temp_dir().join(format!(
            "dfmcp-tree-rust-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&p)
            .map_err(|_| corrupt())?;
        Ok(Self(p))
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[derive(Default)]
struct Faults {
    reads: AtomicU64,
    writes: AtomicU64,
    validations: AtomicU64,
    fail_sync: AtomicBool,
    read_delay_ms: AtomicU64,
    sync_delay_ms: AtomicU64,
}
struct Instrumented {
    inner: PrivateBuildFile,
    faults: Arc<Faults>,
}
impl Read for Instrumented {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        self.faults.reads.fetch_add(1, Ordering::SeqCst);
        thread::sleep(Duration::from_millis(
            self.faults.read_delay_ms.load(Ordering::SeqCst),
        ));
        self.inner.read(b)
    }
}
impl Write for Instrumented {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.faults.writes.fetch_add(1, Ordering::SeqCst);
        self.inner.write(b)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
impl Seek for Instrumented {
    fn seek(&mut self, p: SeekFrom) -> io::Result<u64> {
        self.inner.seek(p)
    }
}
impl EffectJournalStorage for Instrumented {
    fn sync(&mut self) -> io::Result<()> {
        thread::sleep(Duration::from_millis(
            self.faults.sync_delay_ms.load(Ordering::SeqCst),
        ));
        if self.faults.fail_sync.swap(false, Ordering::SeqCst) {
            return Err(io::Error::other("injected sync failure"));
        }
        self.inner.sync()
    }
    fn truncate(&mut self, n: u64) -> io::Result<()> {
        self.inner.truncate(n)
    }
    fn validate_identity(&self) -> io::Result<()> {
        self.faults.validations.fetch_add(1, Ordering::SeqCst);
        self.inner.validate_identity()
    }
}
fn setup() -> Result<(
    Directory,
    Journal<Instrumented>,
    WorkshopAcknowledgements<PrivateBuildFile>,
    Arc<Faults>,
    OperationContext,
    Binding,
)> {
    let dir = Directory::new()?;
    let c = context()?;
    let b = binding("127.0.0.1:5000".parse().map_err(|_| corrupt())?)?;
    let ack = open_private_tree_acknowledgements(
        &dir.0.join("tree-harvest.ack"),
        &c,
        Mode::Control,
        true,
        None,
        b.read_scope()?,
    )?;
    let (file, created, current) = open_private_storage_scoped(
        &dir.0.join("tree-harvest.journal"),
        &c,
        crate::build_placement::journal::BuildMode::Control,
        true,
        MAX_BYTES,
        Some(b.read_scope()?),
    )?;
    assert!(created);
    let faults = Arc::new(Faults::default());
    let store = Instrumented {
        inner: file,
        faults: faults.clone(),
    };
    let j = Journal::open(
        store,
        &current,
        Mode::Control,
        &b,
        [15, 15, 2],
        Some([9; 32]),
        None,
    )?;
    let mut ack = ack;
    ack.acknowledge(j.pin(), &c)?;
    Ok((dir, j, ack, faults, c, b))
}
fn prepare(
    j: &mut Journal<Instrumented>,
    a: &mut WorkshopAcknowledgements<PrivateBuildFile>,
    c: &OperationContext,
) -> Result<Original> {
    let o = Original::new(plan()?, capture()?.tick + 12000)?;
    j.intent(o.clone(), c, a)?;
    j.prepared(record("prepared")?, c, a)?;
    Ok(o)
}
fn phase(
    j: &mut Journal<Instrumented>,
    a: &mut WorkshopAcknowledgements<PrivateBuildFile>,
    c: &OperationContext,
    name: &str,
) -> Result<()> {
    let o = prepare(j, a, c)?;
    let mut guard = |_: &OperationContext| Ok(());
    j.dispatch(o.seal(), c, a, &mut guard, |_, _| record(name))?;
    Ok(())
}
fn counters(f: &Faults) -> [u64; 3] {
    [
        f.reads.load(Ordering::SeqCst),
        f.writes.load(Ordering::SeqCst),
        f.validations.load(Ordering::SeqCst),
    ]
}
#[test]
fn tree_journal_header_and_private_open_charge_one_total_budget() -> Result<()> {
    let c = context()?;
    let b = binding("127.0.0.1:5000".parse().map_err(|_| corrupt())?)?;
    let header = Journal::<PrivateBuildFile>::header(&b, [15, 15, 2], [9; 32]).len() + 32;
    for sufficient in [false, true] {
        let directory = Directory::new()?;
        let (inner, _, _) = open_private_storage_scoped(
            &directory.0.join("empty"),
            &c,
            crate::build_placement::journal::BuildMode::Control,
            true,
            MAX_BYTES,
            Some(b.read_scope()?),
        )?;
        let faults = Arc::new(Faults::default());
        let storage = Instrumented {
            inner,
            faults: faults.clone(),
        };
        let mut limited = c.clone();
        limited.budget.max_bytes = header as u64 * if sufficient { 2 } else { 1 };
        let result = Journal::open(
            storage,
            &limited,
            Mode::Control,
            &b,
            [15, 15, 2],
            Some([9; 32]),
            None,
        );
        assert_eq!(result.is_ok(), sufficient);
        assert_eq!(faults.writes.load(Ordering::SeqCst) > 0, sufficient);
        if !sufficient {
            assert_eq!(
                result.err().map(|e| e.code),
                Some(ErrorCode::BudgetExceeded)
            );
        }
    }
    // Every individual stage fits this allowance. Their aggregate, including
    // the final independent acknowledgement, is one byte too large.
    let total =
        2 * crate::workshop_construction::acknowledgement::WORK_BYTES + 8192 + 2 * header as u64;
    for sufficient in [false, true] {
        let directory = Directory::new()?;
        let mut limited = c.clone();
        limited.budget.max_bytes = total - u64::from(!sufficient);
        let result = open_private(
            &directory.0,
            &limited,
            Mode::Control,
            &b,
            [15, 15, 2],
            true,
            None,
            [9; 32],
        );
        assert_eq!(result.is_ok(), sufficient);
        if !sufficient {
            assert_eq!(
                result.err().map(|e| e.code),
                Some(ErrorCode::BudgetExceeded)
            );
        }
        assert_eq!(
            std::fs::metadata(directory.0.join("tree-harvest.ack"))
                .map_err(|_| corrupt())?
                .len()
                > 0,
            sufficient
        );
    }
    // Real prepared history adds replay bytes; reopening must not reset the
    // allowance between ACK open, descriptor open, replay and acknowledgement.
    let (directory, mut j, mut a, _f, c, b) = setup()?;
    prepare(&mut j, &mut a, &c)?;
    let pin = a.latest().ok_or_else(corrupt)?;
    let mut r = Reader(&j.raw[header..]);
    let mut replay = j.raw.len() as u64;
    while !r.0.is_empty() {
        r.take(8)?;
        let n = r.u32()? as usize;
        r.take(36)?;
        r.take(n)?;
        r.take(40)?;
        replay += 8 * n as u64;
    }
    drop(j);
    drop(a);
    let mut limited = c.clone();
    limited.budget.max_bytes =
        2 * crate::workshop_construction::acknowledgement::WORK_BYTES + 8192 + replay - 1;
    assert_eq!(
        open_private(
            &directory.0,
            &limited,
            Mode::Recover,
            &b,
            [15, 15, 2],
            false,
            Some(pin),
            [0; 32]
        )
        .err()
        .map(|e| e.code),
        Some(ErrorCode::BudgetExceeded)
    );
    limited.budget.max_bytes += 1;
    let (j, _a) = open_private(
        &directory.0,
        &limited,
        Mode::Recover,
        &b,
        [15, 15, 2],
        false,
        Some(pin),
        [0; 32],
    )?;
    assert!(!j.dispatchable());
    Ok(())
}
#[test]
fn tree_journal_cancelled_revoked_and_invalid_dispatch_withdraw_permit() -> Result<()> {
    for variant in 0..5 {
        let (_dir, mut j, mut a, f, c, _) = setup()?;
        let o = prepare(&mut j, &mut a, &c)?;
        let mut denied = c.clone();
        match variant {
            0 => denied.cancellation_requested = true,
            1 => denied.grants.retain(|g| g.capability != Capability::Query),
            2 => denied.grants.retain(|g| g.capability != Capability::Plan),
            3 => denied
                .grants
                .retain(|g| g.capability != Capability::Designate),
            _ => denied.budget.max_wall_millis = 0,
        }
        let before = counters(&f);
        let mut writers = 0;
        let guards = std::cell::Cell::new(0);
        let mut guard = |_: &OperationContext| {
            guards.set(guards.get() + 1);
            Ok(())
        };
        assert!(
            j.dispatch(o.seal(), &denied, &mut a, &mut guard, |_, _| {
                writers += 1;
                record("designated")
            })
            .is_err()
        );
        assert_eq!(counters(&f), before);
        assert_eq!(guards.get(), 0);
        assert_eq!(writers, 0);
        assert!(
            j.dispatch(o.seal(), &c, &mut a, &mut guard, |_, _| {
                writers += 1;
                record("designated")
            })
            .is_err()
        );
        assert_eq!(writers, 0);
    }
    Ok(())
}
#[test]
fn tree_journal_query_authority_and_cancellation_precede_storage() -> Result<()> {
    let (_dir, mut j, _a, f, c, _) = setup()?;
    for cancelled in [false, true] {
        let mut denied = c.clone();
        if cancelled {
            denied.cancellation_requested = true;
        } else {
            denied.grants.clear();
        }
        let before = counters(&f);
        assert!(j.verify(&denied).is_err());
        assert_eq!(counters(&f), before);
    }
    j.verify(&c)?;
    assert!(f.reads.load(Ordering::SeqCst) > 0);
    Ok(())
}
#[test]
fn tree_journal_one_deadline_covers_guard_storage_and_late_read() -> Result<()> {
    let (_dir, mut j, mut a, f, c, _) = setup()?;
    let o = prepare(&mut j, &mut a, &c)?;
    let mut limited = c.clone();
    limited.budget.max_wall_millis = 50;
    f.read_delay_ms.store(80, Ordering::SeqCst);
    assert!(j.verify(&limited).is_err());
    f.read_delay_ms.store(0, Ordering::SeqCst);
    let mut writers = 0;
    let mut guard = |_: &OperationContext| {
        thread::sleep(Duration::from_millis(40));
        Ok(())
    };
    assert!(
        j.dispatch(o.seal(), &limited, &mut a, &mut guard, |_, _| {
            writers += 1;
            record("designated")
        })
        .is_err()
    );
    assert_eq!(writers, 0);
    assert!(
        j.dispatch(o.seal(), &c, &mut a, &mut guard, |_, _| {
            writers += 1;
            record("designated")
        })
        .is_err()
    );
    assert_eq!(writers, 0);
    Ok(())
}
#[test]
fn tree_journal_indeterminate_native_successors_cannot_turn_negative() -> Result<()> {
    for negative in ["refused", "cancelled"] {
        let (dir, mut j, mut a, _f, c, b) = setup()?;
        phase(&mut j, &mut a, &c, "indeterminate")?;
        assert_eq!(j.state, State::Tracking);
        let before = j.raw.clone();
        assert!(j.retain(record(negative)?, &c, &mut a).is_err());
        assert_eq!(j.raw, before);
        assert_eq!(j.native.as_ref().map(|r| r.phase), Some(1));
        // Replay uses the same semantic transition check, even for a byte-valid
        // frame with independently acknowledged head and a valid native codec.
        let original = j.original.clone().ok_or_else(corrupt)?;
        let negative = record(negative)?;
        let mut body = vec![State::Terminal as u8];
        blob(&mut body, &original.encode());
        blob(&mut body, negative.bytes());
        blob(&mut body, &[]);
        body.extend([0, 0]);
        let mut frame = Journal::<Instrumented>::frame_prefix(j.frames + 1, j.head, &body);
        let head = hash(b"dfmcp-tree-frame/1", &frame);
        frame.extend_from_slice(head.as_bytes());
        frame.extend_from_slice(b"DFMTEND1");
        assert!(j.apply(&body).is_err());
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(dir.0.join("tree-harvest.journal"))
            .map_err(|_| corrupt())?;
        file.write_all(&frame)
            .and_then(|_| file.sync_all())
            .map_err(|_| corrupt())?;
        drop(file);
        let pin = WorkshopPin {
            journal_id: j.id,
            head,
            frames: j.frames + 1,
        };
        a.acknowledge(pin, &c)?;
        drop(j);
        drop(a);
        assert!(
            open_private(
                &dir.0,
                &c,
                Mode::Recover,
                &b,
                [15, 15, 2],
                false,
                Some(pin),
                [0; 32]
            )
            .is_err()
        );
    }
    let (_dir, mut j, mut a, _f, c, _) = setup()?;
    phase(&mut j, &mut a, &c, "indeterminate")?;
    j.retain(record("designated")?, &c, &mut a)?;
    assert_eq!(j.state, State::Terminal);
    let (_dir, mut j, mut a, _f, c, _) = setup()?;
    prepare(&mut j, &mut a, &c)?;
    j.retain(record("cancelled")?, &c, &mut a)?;
    assert_eq!(j.native.as_ref().map(|r| r.phase), Some(4));
    Ok(())
}
#[test]
fn tree_journal_prewrite_ack_rejects_valid_prefix_rollback_and_new_key() -> Result<()> {
    let (dir, mut j, mut a, _f, c, b) = setup()?;
    let prefix = j.raw.clone();
    let old_pin = j.pin();
    let o = prepare(&mut j, &mut a, &c)?;
    let mut writers = 0;
    let mut guard = |_: &OperationContext| Ok(());
    let e = j.dispatch(o.seal(), &c, &mut a, &mut guard, |_, _| {
        writers += 1;
        Err(uncertain())
    });
    assert!(e.is_err());
    assert_eq!(writers, 1);
    assert!(a.latest().is_some_and(|p| p.frames >= 3));
    drop(j);
    drop(a);
    std::fs::write(dir.0.join("tree-harvest.journal"), prefix).map_err(|_| corrupt())?;
    let reopened = open_private(
        &dir.0,
        &c,
        Mode::Control,
        &b,
        [15, 15, 2],
        false,
        Some(old_pin),
        [0; 32],
    );
    assert!(reopened.is_err());
    assert_eq!(writers, 1);
    Ok(())
}
#[test]
fn tree_journal_postwriter_sync_failure_is_indeterminate_and_not_retried() -> Result<()> {
    let (dir, mut j, mut a, f, c, b) = setup()?;
    let o = prepare(&mut j, &mut a, &c)?;
    let mut writers = 0;
    let mut guard = |_: &OperationContext| Ok(());
    let result = j.dispatch(o.seal(), &c, &mut a, &mut guard, |_, _| {
        writers += 1;
        f.fail_sync.store(true, Ordering::SeqCst);
        record("designated")
    });
    let e = result.err().ok_or_else(corrupt)?;
    assert_eq!(e.code, ErrorCode::EffectIndeterminate);
    assert!(e.details.iter().any(|(k, v)| k == "key" && v == "tree-001"));
    assert_eq!(writers, 1);
    let pin = a.latest().ok_or_else(corrupt)?;
    drop(j);
    drop(a);
    let (mut recovered, mut a) = open_private(
        &dir.0,
        &c,
        Mode::Recover,
        &b,
        [15, 15, 2],
        false,
        Some(pin),
        [0; 32],
    )?;
    assert!(
        recovered
            .dispatch(o.seal(), &c, &mut a, &mut guard, |_, _| {
                writers += 1;
                record("designated")
            })
            .is_err()
    );
    assert_eq!(writers, 1);
    Ok(())
}
#[test]
fn tree_journal_progress_history_epoch_regression_and_lost_publication() -> Result<()> {
    let (_dir, mut j, mut a, _f, c, _) = setup()?;
    phase(&mut j, &mut a, &c, "designated")?;
    j.begin_sample(&c, &mut a)?;
    j.sample(progress(12355, 8)?, &c, &mut a)?;
    assert!(j.continuity_lost && j.achieved_tick.is_none());
    j.begin_sample(&c, &mut a)?;
    assert!(j.sample(progress(12360, 7)?, &c, &mut a).is_err());
    assert!(j.achieved_tick.is_none());
    let (dir, mut j, mut a, _f, c, b) = setup()?;
    phase(&mut j, &mut a, &c, "designated")?;
    j.begin_sample(&c, &mut a)?;
    let pin = a.latest().ok_or_else(corrupt)?;
    drop(j);
    drop(a);
    let (mut j, mut a) = open_private(
        &dir.0,
        &c,
        Mode::Recover,
        &b,
        [15, 15, 2],
        false,
        Some(pin),
        [0; 32],
    )?;
    assert!(j.sample_pending);
    assert!(j.begin_sample(&c, &mut a).is_err());
    assert!(j.achieved_tick.is_none());
    let (_dir, mut j, mut a, _f, c, _) = setup()?;
    phase(&mut j, &mut a, &c, "designated")?;
    j.begin_sample(&c, &mut a)?;
    j.sample(progress(12355, 7)?, &c, &mut a)?;
    assert_eq!(j.achieved_tick, Some(12355));
    j.begin_sample(&c, &mut a)?;
    assert!(j.sample(progress(12354, 7)?, &c, &mut a).is_err());
    Ok(())
}
