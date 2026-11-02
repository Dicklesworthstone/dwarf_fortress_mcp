//! Real private files and ACK recovery. Component bytes establish no live proof.
use super::super::tests::{native_capture as capture, native_plan as plan, vector};
use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, ObservationCursor, RequestId, StateAnchor, WorkBudget,
};
use std::os::unix::fs::DirBuilderExt;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(1);
pub(crate) fn context() -> Result<OperationContext> {
    let c = capture()?;
    Ok(OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: StateAnchor {
            fortress_id: c.source.fortress_id(),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(c.tick),
            state_hash: c.witness(),
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
            Capability::ConfigureProduction,
        ]
        .into_iter()
        .map(|capability| CapabilityGrant {
            capability,
            scope: CapabilityScope {
                fortress_id: Some(c.source.fortress_id()),
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
        protocol: super::super::ManagerProtocol::V1_22,
        endpoint,
        source: c.source.clone(),
        generation: c.generation,
        dimensions: c.dimensions,
        selection: c.selection,
        scope: dfmcp_core::MapCuboid::new(
            dfmcp_core::MapCoord::new(0, 0, 0),
            dfmcp_core::MapCoord::new(
                (c.dimensions[0] - 1) as i32,
                (c.dimensions[1] - 1) as i32,
                (c.dimensions[2] - 1) as i32,
            ),
        )?,
        df_version: "test-df".into(),
        dfhack_version: "test-dfhack".into(),
    })
}
pub(crate) struct Directory(pub std::path::PathBuf);
impl Directory {
    pub(crate) fn new() -> Result<Self> {
        let p = std::env::temp_dir().join(format!(
            "dfmcp-manager-test-{}-{}",
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
type JournalFixture = (
    Directory,
    Journal<PrivateBuildFile>,
    WorkshopAcknowledgements<PrivateBuildFile>,
    OperationContext,
    Binding,
);
fn setup() -> Result<JournalFixture> {
    let d = Directory::new()?;
    let c = context()?;
    let b = binding("127.0.0.1:5000".parse().map_err(|_| corrupt())?)?;
    let (j, a) = open_private(
        &d.0,
        &c,
        Mode::Control,
        &b,
        b.selection,
        true,
        None,
        [9; 32],
    )?;
    Ok((d, j, a, c, b))
}
fn prepared(
    j: &mut Journal<PrivateBuildFile>,
    a: &mut WorkshopAcknowledgements<PrivateBuildFile>,
    c: &OperationContext,
) -> Result<Original> {
    let p = plan()?;
    let o = Original::new(p.clone(), p.before.tick + 12000)?;
    j.intent(o.clone(), c, a)?;
    j.prepared(Record::decode(&vector("prepared")?, &p)?, c, a)?;
    Ok(o)
}
#[test]
fn acknowledged_positive_dispatch_once_and_reopen_never_reauthorizes() -> Result<()> {
    let (d, mut j, mut a, c, b) = setup()?;
    let o = prepared(&mut j, &mut a, &c)?;
    let mut calls = 0;
    let r = j.dispatch(o.seal(), &c, &mut a, &mut |_| Ok(()), |permit, _| {
        calls += 1;
        assert_eq!(permit.plan(), &o.plan);
        assert_ne!(permit.journal_head(), Digest32::ZERO);
        Record::decode(&vector("assigned")?, &o.plan)
    })?;
    assert!(r.complete());
    assert_eq!(calls, 1);
    assert_eq!(a.latest(), Some(j.pin()));
    assert!(!j.dispatchable());
    assert!(
        j.dispatch(o.seal(), &c, &mut a, &mut |_| Ok(()), |_, _| {
            calls += 1;
            Err(uncertain())
        })
        .is_err()
    );
    assert_eq!(calls, 1);
    let pin = j.pin();
    drop(j);
    drop(a);
    let (j, a) = open_private(
        &d.0,
        &c,
        Mode::Recover,
        &b,
        b.selection,
        false,
        Some(pin),
        [0; 32],
    )?;
    assert_eq!(j.original, Some(o));
    assert!(j.native.as_ref().is_some_and(Record::complete));
    assert!(!j.dispatchable());
    assert_eq!(a.latest(), Some(pin));
    Ok(())
}
#[test]
fn lost_reply_and_prepared_reopen_never_dispatch_and_keep_original() -> Result<()> {
    for lost in [false, true] {
        let (d, mut j, mut a, c, b) = setup()?;
        let o = prepared(&mut j, &mut a, &c)?;
        if lost {
            let e = j.dispatch(o.seal(), &c, &mut a, &mut |_| Ok(()), |_, _| {
                Err(uncertain())
            });
            assert!(e.is_err());
            assert!(j.effect_uncertain);
        }
        let pin = j.pin();
        drop(j);
        drop(a);
        let (mut j, mut a) = open_private(
            &d.0,
            &c,
            Mode::Recover,
            &b,
            b.selection,
            false,
            Some(pin),
            [0; 32],
        )?;
        assert_eq!(j.original, Some(o.clone()));
        let mut calls = 0;
        assert!(
            j.dispatch(o.seal(), &c, &mut a, &mut |_| Ok(()), |_, _| {
                calls += 1;
                Err(uncertain())
            })
            .is_err()
        );
        assert_eq!(calls, 0);
        if lost {
            j.retain(Record::decode(&vector("assigned")?, &o.plan)?, &c, &mut a)?;
            assert!(j.native.as_ref().is_some_and(Record::complete));
            assert!(!j.effect_uncertain);
        }
    }
    Ok(())
}
#[test]
fn missing_changed_or_rolled_back_ack_and_journal_fail_closed() -> Result<()> {
    for which in [0, 1, 2] {
        let (d, mut j, mut a, c, b) = setup()?;
        let ack0 = std::fs::read(d.0.join("manager-appointment.ack")).map_err(|_| corrupt())?;
        prepared(&mut j, &mut a, &c)?;
        let pin = j.pin();
        drop(j);
        drop(a);
        let p = d.0.join(if which == 0 {
            "manager-appointment.journal"
        } else {
            "manager-appointment.ack"
        });
        if which == 2 {
            std::fs::write(&p, &ack0).map_err(|_| corrupt())?;
        } else {
            std::fs::remove_file(&p).map_err(|_| corrupt())?;
        }
        assert!(
            open_private(
                &d.0,
                &c,
                Mode::Recover,
                &b,
                b.selection,
                false,
                Some(pin),
                [0; 32]
            )
            .is_err()
        );
    }
    Ok(())
}
#[test]
fn expiry_revocation_and_wrong_seal_consume_permission_before_writer() -> Result<()> {
    for which in 0..4 {
        let (_d, mut j, mut a, c, _) = setup()?;
        let o = prepared(&mut j, &mut a, &c)?;
        let mut bad = c.clone();
        if which == 0 {
            bad.cancellation_requested = true;
        }
        if which == 1 {
            bad.grants
                .retain(|g| g.capability != Capability::ConfigureProduction);
        }
        if which == 2 {
            bad.budget.max_wall_millis = 0;
        }
        let seal = if which == 3 { Digest32::ZERO } else { o.seal() };
        let mut calls = 0;
        assert!(
            j.dispatch(seal, &bad, &mut a, &mut |_| Ok(()), |_, _| {
                calls += 1;
                Err(uncertain())
            })
            .is_err()
        );
        assert_eq!(calls, 0);
        assert!(!j.dispatchable());
    }
    Ok(())
}
