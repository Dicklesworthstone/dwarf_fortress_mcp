#![forbid(unsafe_code)]
//! MCP presentation plane for the Dwarf Fortress semantic control plane.
//!
//! `dfmcp-mcp` binds the frozen 11-tool `fortress.*` narrow waist to the owned
//! [`fastmcp_rust`](https://github.com/Dicklesworthstone/fastmcp_rust) sibling
//! (the `fastmcp-rust` facade) and runs it as a modern-only MCP 2026-07-28
//! server. The plane is deliberately thin: every semantic decision is
//! delegated to `dfmcp-core`, `dfmcp-world`, `dfmcp-intent`, `dfmcp-adapter`,
//! and `dfmcp-lab`. This crate adds transport, session framing, and the
//! authority-free canonical Agent Turn Packet projection. It must never become
//! an authority: no tool here may bypass plan sealing, commit-time
//! revalidation, idempotency, or evidence checks (ADR-013,
//! `docs/FASTMCP_INTEGRATION.md`).

pub mod admission;
pub mod agent_facade;
pub mod agent_turn;
pub mod doctor;
pub mod ee_memory;
pub mod http_transport;
pub mod live_jobs_server;
mod live_server;
mod live_server_v1_1;
pub mod server;
pub mod tasks;

pub use admission::{AdmissionProvenance, current_admission_provenance, run_live_stdio};
pub use agent_facade::run_stdio;
pub use agent_turn::{
    AGENT_TURN_SCHEMA, AgentPhase, AgentTurnBuilder, ContinuityStatus, ObservationProfile,
    RecoveryClass, empty_active_work, empty_budget, empty_coverage, recommendation,
    recovery_guidance, uncertainty,
};
pub use doctor::{DoctorDiagnosticReport, DoctorInspector};
pub use ee_memory::{EeMemoryBatch, EeMemoryItem};
pub use http_transport::{
    HttpSessionResumeToken, HttpTransportSessionManager, MAX_HTTP_MESSAGE_BYTES,
    MAX_HTTP_SESSION_BUFFER_BYTES, MAX_HTTP_SESSIONS, MAX_HTTP_TOTAL_BUFFER_BYTES,
    MAX_RESUMPTION_BUFFER_SIZE,
};
pub use server::validate_localhost_bind;
pub use tasks::{McpTaskProjection, McpTaskStatus, cancel_action_task, project_action_task};

/// Own the runtime while polling a presentation-plane operation. An inherited
/// context keeps its original drivers, cancellation, budget, and runtime mask.
fn run_with_runtime_cx<F: std::future::Future>(
    operation: impl FnOnce(fastmcp_rust::asupersync::Cx) -> F,
) -> Result<F::Output, Box<dyn std::error::Error>> {
    use fastmcp_rust::asupersync::{Budget, Cx, runtime::RuntimeBuilder};

    let inherited = Cx::current();
    let reactor = fastmcp_rust::asupersync::runtime::reactor::create_reactor()?;
    let runtime = RuntimeBuilder::new()
        .worker_threads(1)
        .with_reactor(reactor)
        // The stdio receive pump must not occupy the scheduler worker. Match
        // the framework bridge's bounded, on-demand blocking capacity.
        .blocking_threads(0, 16)
        .build()?;
    let cx = match inherited {
        Some(cx) => cx,
        None => runtime.request_cx_with_budget(Budget::INFINITE),
    };
    Ok(runtime.block_on(async move {
        let _guard = Cx::set_current(Some(cx.clone()));
        operation(cx).await
    }))
}

fn run_modern_stdio(server: fastmcp_rust::modern::Server) {
    if let Err(error) = run_with_runtime_cx(|cx| async move {
        server.run_stdio_with_cx(&cx).await;
    }) {
        eprintln!("MCP runtime startup failed: {error}");
        std::process::exit(1);
    }
}

/// Run the explicitly unadmitted protocol-1.1 development server.
///
/// The public seam rejects the production protocol marker before entering the
/// private runtime so external callers cannot accidentally combine development
/// execution with a production-looking admission environment.
pub fn run_live_v1_1_development_stdio() {
    const ADMITTED_PROTOCOL_ENVIRONMENT: &str = "DFMCP_ADMITTED_BRIDGE_PROTOCOL";
    if std::env::var_os(ADMITTED_PROTOCOL_ENVIRONMENT).is_some() {
        eprintln!(
            "unadmitted protocol-1.1 development runtime refuses production admission environment: {ADMITTED_PROTOCOL_ENVIRONMENT}"
        );
        std::process::exit(1);
    }
    live_server_v1_1::run_live_v1_1_development_stdio();
}

#[cfg(test)]
mod runtime_entry_tests {
    use super::run_with_runtime_cx;
    use fastmcp_rust::asupersync::{Cx, cx::cap, runtime::SpawnError};
    use std::error::Error;
    use std::task::Poll;
    use std::time::Duration;

    #[test]
    fn runtime_entry_runs_owned_blocking_work() -> Result<(), Box<dyn Error>> {
        assert!(Cx::current().is_none());
        run_with_runtime_cx(|cx| async move {
            assert!(cx.io().is_some());
            assert!(cx.timer_driver().is_some());
            let caller = std::thread::current().id();
            let mut child = cx.spawn_blocking(move |child_cx| {
                assert!(child_cx.checkpoint().is_ok());
                (42, std::thread::current().id())
            })?;
            let (answer, worker) = fastmcp_rust::asupersync::time::timeout(
                cx.now(),
                Duration::from_secs(5),
                child.join(&cx),
            )
            .await??;
            assert_eq!(answer, 42);
            assert_ne!(worker, caller);
            Ok::<_, Box<dyn Error>>(())
        })??;
        assert!(Cx::current().is_none());
        Ok(())
    }

    #[test]
    fn runtime_entry_preserves_inherited_restrictions_and_restores_parent()
    -> Result<(), Box<dyn Error>> {
        run_with_runtime_cx(|parent| async move {
            let parent_caps = parent.capabilities();
            assert!(parent.timer_driver().is_some());
            let task = parent.task_id();
            let region = parent.region_id();
            {
                let _restricted = parent.restrict::<cap::None>().set_current_restricted();
                run_with_runtime_cx(|cx| async move {
                    let mut polls = 0;
                    std::future::poll_fn(|context| {
                        polls += 1;
                        assert_eq!(cx.task_id(), task);
                        assert_eq!(cx.region_id(), region);
                        assert!(cx.io().is_none());
                        assert!(cx.timer_driver().is_none());
                        assert!(!cx.capabilities().spawn);
                        assert!(matches!(
                            cx.spawn_blocking(|_| 42),
                            Err(SpawnError::RuntimeUnavailable)
                        ));
                        let ambient = Cx::current().ok_or("runtime entry lost its context");
                        match ambient {
                            Ok(ambient) => {
                                assert_eq!(ambient.task_id(), task);
                                assert_eq!(ambient.capabilities(), cx.capabilities());
                            }
                            Err(error) => return Poll::Ready(Err(error)),
                        }
                        if polls == 1 {
                            context.waker().wake_by_ref();
                            Poll::Pending
                        } else {
                            Poll::Ready(Ok(()))
                        }
                    })
                    .await
                })??;
                assert!(
                    Cx::current()
                        .ok_or("restriction lost")?
                        .timer_driver()
                        .is_none()
                );
            }
            let restored = Cx::current().ok_or("parent context lost")?;
            assert_eq!(restored.task_id(), task);
            assert_eq!(restored.capabilities(), parent_caps);
            assert!(restored.timer_driver().is_some());
            parent.cancel_with(
                fastmcp_rust::asupersync::types::CancelKind::User,
                Some("inherited entry cancellation"),
            );
            run_with_runtime_cx(|cx| async move {
                assert_eq!(cx.task_id(), task);
                assert!(cx.checkpoint().is_err());
                assert!(cx.cancelled_by(fastmcp_rust::asupersync::types::CancelKind::User));
            })?;
            Ok::<_, Box<dyn Error>>(())
        })??;
        assert!(Cx::current().is_none());
        Ok(())
    }
}
