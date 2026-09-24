use super::*;
use crate::order_progress::ProgressState;
use crate::order_progress::tests::sample;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, ObservationCursor, RequestId, StateAnchor, WorkBudget,
};
use std::collections::VecDeque;

struct Source {
    queue: VecDeque<Result<OrderProgress>>,
    manifest: ProgressManifest,
    reads: usize,
    fenced: bool,
}
impl OrderProgressSource for Source {
    fn manifest(&self) -> &ProgressManifest {
        &self.manifest
    }
    fn fence(&mut self) {
        self.fenced = true;
    }
    fn read_order(&mut self, _: u32, _: Duration) -> Result<OrderProgress> {
        self.reads += 1;
        self.queue.pop_front().unwrap()
    }
}
fn context() -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(2),
        anchor: StateAnchor {
            fortress_id: sample(1, 10, 5).fortress_id(),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(0),
            state_hash: Digest32::ZERO,
        },
        budget: WorkBudget {
            max_entities: 4096,
            max_game_ticks: 1000,
            ..WorkBudget::default()
        },
        cancellation_requested: false,
        grants: vec![CapabilityGrant {
            capability: Capability::Query,
            scope: CapabilityScope::default(),
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        }],
    }
}
fn setup(values: Vec<Result<OrderProgress>>) -> ProgressSession<Source> {
    ProgressSession::new(
        Source {
            queue: values.into(),
            manifest: ProgressManifest {
                generation: 7,
                df_version: "test-df".into(),
                dfhack_version: "test-dfhack".into(),
            },
            reads: 0,
            fenced: false,
        },
        &context(),
    )
    .unwrap()
}
#[test]
fn observe_register_poll_and_terminal_replay_never_add_native_calls() {
    let c = context();
    let mut s = setup(vec![
        Ok(sample(1, 10, 5)),
        Ok(sample(2, 12, 0)),
        Ok(sample(3, 14, 0)),
    ]);
    let o = s.observe(10, &c).unwrap();
    s.register("beds", o.witness(), 100, 2, 2, &c).unwrap();
    assert_eq!(s.poll("beds", &c).unwrap().zero_samples(), 1);
    let terminal = s.poll("beds", &c).unwrap();
    assert_eq!(terminal.state(), ProgressState::CounterZeroStable);
    assert_eq!(s.poll("beds", &c).unwrap(), terminal);
    assert_eq!(s.source.reads, 3);
    assert_eq!(
        s.register("beds", o.witness(), 100, 2, 2, &c).unwrap(),
        terminal
    );
    assert!(s.register("beds", o.witness(), 101, 2, 2, &c).is_err());
    let mut small = c.clone();
    small.budget.max_bytes = WATCH_RESERVE_BYTES - 1;
    assert!(s.register("beds", o.witness(), 100, 2, 2, &small).is_err());
}
#[test]
fn failed_refresh_invalidates_selection_and_fences_without_forgetting_watches() {
    let c = context();
    let mut s = setup(vec![
        Ok(sample(1, 10, 5)),
        Err(error(ErrorCode::AdapterUnavailable, "lost read")),
    ]);
    let o = s.observe(10, &c).unwrap();
    s.register("beds", o.witness(), 100, 1, 2, &c).unwrap();
    assert!(s.poll("beds", &c).is_err());
    assert!(s.selected(&c).unwrap().is_none());
    assert!(s.poisoned());
    assert!(s.poll("beds", &c).is_err());
    assert_eq!(s.source.reads, 2);
    assert_eq!(s.watches(&c).unwrap().len(), 1);
    assert_eq!(
        s.cancel("beds", &c).unwrap().state(),
        ProgressState::Cancelled
    );
}
#[test]
fn authority_expiry_uses_observed_tick_and_does_not_revert_to_old_anchor() {
    let mut c = context();
    c.grants[0].expires_at_tick = Some(GameTick(11));
    let mut s = setup(vec![Ok(sample(1, 12, 5))]);
    assert!(s.observe(10, &c).is_err());
    assert!(s.watches(&c).is_err());
    assert!(s.poisoned());
    assert_eq!(s.source.reads, 1);
    let mut denied = context();
    denied.grants.clear();
    assert!(s.selected(&denied).is_err());
}
#[test]
fn bounds_cancellation_and_session_scope_fail_before_native_work() {
    let c = context();
    let mut s = setup(vec![]);
    let mut denied = c.clone();
    denied.budget.max_entities = 4095;
    assert!(s.observe(10, &denied).is_err());
    denied = c.clone();
    denied.budget.max_bytes = RPC_RESERVE_BYTES;
    assert!(s.observe(10, &denied).is_err());
    denied = c.clone();
    denied.session_id = SessionId::new(9);
    assert!(s.observe(10, &denied).is_err());
    denied = c.clone();
    denied.cancellation_requested = true;
    assert!(s.observe(10, &denied).is_err());
    assert!(s.observe(u32::MAX, &c).is_err());
    assert_eq!(s.source.reads, 0);
}
#[test]
fn bounded_discovery_is_sorted_and_retired_keys_do_not_renew_deadlines() {
    let c = context();
    let mut s = setup(vec![Ok(sample(1, 10, 5))]);
    let o = s.observe(10, &c).unwrap();
    for i in (0..MAX_WATCHES).rev() {
        s.register(&format!("w{i}"), o.witness(), 100, 1, 2, &c)
            .unwrap();
    }
    assert!(s.register("overflow", o.witness(), 100, 1, 2, &c).is_err());
    let listed = s.watches(&c).unwrap();
    assert_eq!(listed[0].key(), "w0");
    assert_eq!(listed[7].key(), "w7");
    s.cancel("w0", &c).unwrap();
    assert_eq!(
        s.register("w0", o.witness(), 100, 1, 2, &c)
            .unwrap()
            .state(),
        ProgressState::Cancelled
    );
    assert_eq!(s.source.reads, 1);
}
