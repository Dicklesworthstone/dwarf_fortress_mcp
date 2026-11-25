//! Regression for df-lab-session-output-b5r and df-fastmcp-conformance-5pj.3.
//! Exercise the real facade and owned wake registrations without installing a
//! process-global Task service that would change other session tests.

use super::*;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::{Wake, Waker};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

thread_local! {
    static SIGNAL: RefCell<Option<Arc<ProgressSignal>>> = const { RefCell::new(None) };
}

pub(super) fn current_signal() -> Option<Arc<ProgressSignal>> {
    SIGNAL.with(|signal| signal.borrow().clone())
}

struct SignalScope(Option<Arc<ProgressSignal>>);

impl SignalScope {
    fn enter(signal: Arc<ProgressSignal>) -> Self {
        Self(SIGNAL.with(|current| current.replace(Some(signal))))
    }
}

impl Drop for SignalScope {
    fn drop(&mut self) {
        let _ = SIGNAL.with(|current| current.replace(self.0.take()));
    }
}

struct PublicationWake {
    session: String,
    expected_tokens: u32,
    calls: AtomicUsize,
    saw_unpublished_budget: AtomicBool,
}

impl PublicationWake {
    fn new(session: &str, tokens: u32) -> Arc<Self> {
        Arc::new(Self {
            session: session.to_owned(),
            expected_tokens: tokens,
            calls: AtomicUsize::new(0),
            saw_unpublished_budget: AtomicBool::new(false),
        })
    }
}

impl Wake for PublicationWake {
    fn wake(self: Arc<Self>) {
        let tokens = crate::server::session_output_budget::current_budget(&self.session)
            .ok()
            .map(|budget| budget.max_output_tokens);
        if tokens != Some(self.expected_tokens) {
            self.saw_unpublished_budget.store(true, Ordering::SeqCst);
        }
        self.calls.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn accepted_output_renegotiation_wakes_monitors_and_recovery_after_publication() -> TestResult {
    let _serial = crate::test_serial();
    let opened: Value = serde_json::from_str(&crate::agent_facade::fortress_open_session(
        Some(true),
        Some("7855304".to_owned()),
        None,
        None,
        None,
        None,
        None,
        Some(4_096),
        None,
        None,
        None,
        None,
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    let session = opened["session_id"].as_str().ok_or("session missing")?;
    let planned: Value = serde_json::from_str(&crate::agent_facade::fortress_plan(
        Some(session.to_owned()),
        Some("preserve reviewed work during output recovery".to_owned()),
        Some(false),
        None,
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");

    let signal = Arc::new(ProgressSignal::new(1));
    let monitor = signal.monitor("output-budget-monitor")?;
    let recovery = signal.recovery()?;
    let ordinary = PublicationWake::new(session, 32_768);
    let recovering = PublicationWake::new(session, 32_768);
    monitor.register(&Waker::from(Arc::clone(&ordinary)))?;
    recovery.register(&Waker::from(Arc::clone(&recovering)))?;
    let _scope = SignalScope::enter(Arc::clone(&signal));

    // An accepted retry with changed=false must also reawaken a retained
    // handoff. Neither request is an authorization to progress original work.
    for (index, changed) in [true, false].into_iter().enumerate() {
        let generation = signal.generation();
        let accepted: Value = serde_json::from_str(
            &crate::agent_facade::fortress_open_session_with_session(
                Some(session.to_owned()),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(32_768),
                None,
                None,
                None,
                None,
            ),
        )?;
        assert_eq!(accepted["ok"], true, "{accepted}");
        assert_eq!(accepted["kind"], "output_budget_renegotiation");
        assert_eq!(accepted["changed"], changed);
        assert_eq!(accepted["budget"]["max_output_tokens"], 32_768);
        assert_eq!(accepted["game_effect"], "none");
        assert_eq!(accepted["anchor"], planned["agent_turn"]["anchor"]);
        assert_eq!(
            accepted["agent_turn"]["active_work"]["pending_plans"][0]["plan_digest"],
            planned["plan_digest"]
        );
        assert_eq!(
            signal.generation(),
            generation + 1,
            "successful output publication must wake the existing recovery owner"
        );
        for receiver in [&ordinary, &recovering] {
            assert_eq!(receiver.calls.load(Ordering::SeqCst), index + 1);
            assert!(
                !receiver.saw_unpublished_budget.load(Ordering::SeqCst),
                "the current allowance must be installed and its session lock released before wake"
            );
        }
    }
    Ok(())
}
