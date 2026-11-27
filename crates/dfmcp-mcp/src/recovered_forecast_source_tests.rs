//! Exact typed-source custody and unavailable historical forecast metadata.

use super::*;
use super::super::super as server;
use crate::agent_facade as facade;
use dfmcp_core::{FortressId, GameTick, ObservationCursor, RequestId, SessionId, StepId};
use dfmcp_intent::{Action, PlanStep};
use dfmcp_world::{EntityKind, Predicate, WorldGraph, WorldSnapshot};
use serde_json::json;

fn fixture() -> (WorldSnapshot, PreparedPlan, OperationContext) {
    let snapshot = WorldSnapshot::new(
        FortressId::new(816_630), GameTick(1), ObservationCursor::ORIGIN, false,
        WorldGraph::default(),
    );
    let intent = dfmcp_core::IntentId::new(816_630);
    let action = Action::Pause { paused: true };
    let plan = PreparedPlan::builder(
        intent, snapshot.anchor(), "typed source witness",
        Predicate::EntityKind { entity_id: dfmcp_core::EntityId::new(1), kind: EntityKind::Unit },
    )
    .steps(vec![PlanStep {
        id: StepId::new(0), action: action.clone(), preconditions: Vec::new(),
        postconditions: vec![Predicate::Paused(true)], compensation: None, obligation: None,
        depends_on: Vec::new(), risk: RiskTier::Reversible,
        required_capability: Capability::ControlClock,
        idempotency_key: dfmcp_intent::derive_step_idempotency_key(
            intent, snapshot.anchor(), StepId::new(0), &action,
        ),
    }])
    .max_risk(RiskTier::Reversible)
    .required_capabilities(std::collections::BTreeSet::from([Capability::ControlClock]))
    .expires_at_tick(GameTick(100))
    .build();
    let context = OperationContext {
        session_id: SessionId::new(816_630), request_id: RequestId::new(1),
        anchor: snapshot.anchor(), budget: MAX_LAB_BUDGET, grants: Vec::new(),
        cancellation_requested: false,
    };
    (snapshot, plan, context)
}

#[test]
fn raw_source_witness_needs_no_ledger_but_binds_the_exact_typed_plan() -> Result<()> {
    let (snapshot, plan, context) = fixture();
    plan.validate_structure()?;
    assert!(crate::lab_world::ProductionGoalLedger::capture(&snapshot).is_err());
    let source = RecoveredOriginalSource::capture(&snapshot, &plan, &context)?;
    source.validate_plan(&plan)?;
    assert_eq!(source.anchor, plan.anchor);
    assert_eq!(source.bytes, snapshot.canonical_bytes().len() as u64);
    assert!(RecoveredProjectSource::capture(
        &snapshot, &plan, source, &context, std::time::Instant::now(),
    ).is_err());

    let mut alias = plan.clone();
    alias.terminal_condition = Predicate::EntityKind {
        entity_id: dfmcp_core::EntityId::new(1),
        kind: EntityKind::Other("unit".to_owned()),
    };
    assert_ne!(alias, plan);
    assert_eq!(alias.compute_digest(), plan.digest);
    alias.validate_structure()?;
    assert!(matches!(source.validate_plan(&alias),
        Err(failure) if failure.code == ErrorCode::CorruptLedger));
    let mut changed = snapshot.clone();
    changed.paused = true;
    changed.refresh_hash();
    assert!(matches!(RecoveredOriginalSource::capture(&changed, &plan, &context),
        Err(failure) if failure.code == ErrorCode::CorruptLedger));
    Ok(())
}

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct DurableFixture(std::path::PathBuf);
impl DurableFixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-current-stage-source-witness-{}", std::process::id(),
        ));
        let _ = std::fs::remove_dir_all(&path);
        server::simulate_durable_restart(Some(path.clone()));
        Self(path)
    }
    fn restart(&self) {
        server::simulate_durable_restart(Some(self.0.clone()));
    }
}
impl Drop for DurableFixture {
    fn drop(&mut self) {
        server::simulate_durable_restart(None);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn open() -> TestResult<String> {
    let result: Value = serde_json::from_str(&facade::fortress_open_session(
        Some(false), Some("8161531".to_owned()),
        Some([
            ("observe", "read_only"), ("query", "read_only"), ("plan", "guarded"),
            ("control_clock", "reversible"), ("checkpoint", "guarded"),
        ].into_iter().map(|(capability, risk)| (capability.to_owned(), risk.to_owned())).collect()),
        Some(60_000), Some(4_000), Some(32_768), Some(4 * 1024 * 1024),
        Some(32_768), Some(64), Some("empty".to_owned()), Some(false), Some(true),
    ))?;
    assert_eq!(result["ok"], true, "{result}");
    Ok(result["session_id"].as_str().ok_or("session id missing")?.to_owned())
}

fn query(session: &str, digest: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(&facade::fortress_query(
        Some(session.to_owned()), Some(json!({
            "mode":"forecast_originals", "plan_digests":[digest], "horizon_ticks":0,
        }).to_string()),
    ))?)
}

#[test]
fn fully_historical_original_cannot_forecast_without_its_captured_source_witness() -> TestResult {
    let _serial = crate::test_serial();
    let _durable = server::serialized_durable_tests();
    let directory = DurableFixture::new();
    let first = open()?;
    let planned: Value = serde_json::from_str(&facade::fortress_plan(
        Some(first.clone()), None, None,
        Some(json!([{"action":{"kind":"pause","paused":true}}]).to_string()), None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let digest = planned["plan_digest"].as_str().ok_or("plan digest missing")?.to_owned();
    let committed: Value = serde_json::from_str(&facade::fortress_commit(Some(first), digest.clone()))?;
    assert_eq!(committed["ok"], true, "{committed}");
    directory.restart();
    let current = open()?;
    let valid = query(&current, &digest)?;
    assert_eq!(valid["ok"], true, "{valid}");
    assert_eq!(valid["schema"], "dfmcp.lab-original-forecast/5");
    assert_eq!(valid["prediction"]["all_complete"], true);

    let handle = server::resolve_session(Some(current.clone()))?;
    let original = Digest32::from_hex(&digest).ok_or("invalid original digest")?;
    let (source, before) = {
        let mut session = handle.lock().map_err(|_| "fixture session poisoned")?;
        assert!(session.carried.is_empty());
        assert!(session.open_actions.is_empty());
        let goal = session.objectives.iter_mut().find(|goal| goal.plan_digest == original)
            .ok_or("historical original missing")?;
        let source = goal.recovered_forecast_source.take().ok_or("captured source missing")?;
        (source, format!("{:?}|{:?}", session.adapter, session.objectives))
    };
    let refused = query(&current, &digest)?;
    assert_eq!(refused["ok"], false, "{refused}");
    {
        let mut session = handle.lock().map_err(|_| "fixture session poisoned")?;
        assert_eq!(format!("{:?}|{:?}", session.adapter, session.objectives), before);
        session.objectives.iter_mut().find(|goal| goal.plan_digest == original)
            .ok_or("historical original missing")?.recovered_forecast_source = Some(source);
    }
    let restored = query(&current, &digest)?;
    assert_eq!(restored["ok"], true, "{restored}");
    assert_eq!(restored["prediction"]["all_complete"], true);
    Ok(())
}
