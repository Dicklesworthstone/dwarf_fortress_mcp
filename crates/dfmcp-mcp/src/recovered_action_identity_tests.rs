//! Recovered terminal proof retains the original generation and input custody.
use super::*;
use dfmcp_core::EntityId;
use dfmcp_intent::{FurnitureKind, effects};
use dfmcp_world::WorldSnapshot;
use serde_json::{Value, json};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct StateDir(std::path::PathBuf);

impl StateDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-recovered-action-identity-{label}-{}",
            std::process::id(),
        ));
        let _ = std::fs::remove_dir_all(&path);
        simulate_durable_restart(Some(path.clone()));
        Self(path)
    }

    fn restart(&self) {
        simulate_durable_restart(Some(self.0.clone()));
    }
}

impl Drop for StateDir {
    fn drop(&mut self) {
        simulate_durable_restart(None);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn parsed(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn identity(value: &Value, field: &str) -> TestResult<String> {
    Ok(value[field]
        .as_str()
        .ok_or_else(|| format!("missing {field}: {value}"))?
        .to_owned())
}

fn open(selector: &str, scenario: &str, can_dispatch: bool) -> TestResult<String> {
    let mut capabilities = vec![
        ("observe", "read_only"),
        ("query", "read_only"),
        ("control_clock", "reversible"),
        ("checkpoint", "guarded"),
    ];
    if can_dispatch {
        capabilities.extend([
            ("plan", "guarded"),
            ("construct", "guarded"),
            ("configure_production", "reversible"),
        ]);
    }
    let opened = parsed(&open_session_in_scenario(
        Some(false),
        Some(selector.to_owned()),
        Some(
            capabilities
                .into_iter()
                .map(|(capability, risk)| (capability.to_owned(), risk.to_owned()))
                .collect(),
        ),
        Some(60_000),
        Some(2_000),
        Some(8_192),
        None,
        Some(16_384),
        Some(64),
        Some(scenario.to_owned()),
        Some(false),
        Some(true),
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    identity(&opened, "session_id")
}

fn commit(session: &str, actions: Value) -> TestResult<Digest32> {
    let planned = parsed(&plan_request(
        Some(session.to_owned()),
        Some("retain original action identity through recovery".to_owned()),
        None,
        Some(actions.to_string()),
        None,
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let digest = identity(&planned, "plan_digest")?;
    let committed = parsed(&fortress_commit(Some(session.to_owned()), digest.clone()))?;
    assert_eq!(committed["ok"], true, "{committed}");
    Ok(Digest32::from_hex(&digest).ok_or("invalid plan digest")?)
}

#[derive(Clone, Copy, Debug)]
enum Work {
    Construction,
    Provisions,
    Furniture,
}

impl Work {
    fn ticks(self) -> u64 {
        match self {
            Self::Construction | Self::Furniture => effects::BUILD_TICKS,
            Self::Provisions => effects::WORK_ORDER_TICKS_PER_UNIT,
        }
    }
}

struct Fixture {
    session: String,
    digest: Digest32,
    _directory: StateDir,
}

fn fixture(label: &str, work: Work) -> TestResult<Fixture> {
    let directory = StateDir::new(label);
    let scenario = match work {
        Work::Provisions => "starter_fortress_provisions",
        Work::Construction | Work::Furniture => "starter_fortress_economy",
    };
    let session = open("7855901", scenario, true)?;
    let action = match work {
        Work::Construction => json!({
            "kind":"build", "building":"workshop:Still", "location":[0,0,10],
            "min":[0,0,10], "max":[0,0,10]
        }),
        Work::Provisions => json!({
            "kind":"create_work_order", "name":"original provision input",
            "job_token":"BREW_DRINK", "amount":1
        }),
        Work::Furniture => {
            commit(
                &session,
                json!([{"action":{
                    "kind":"create_work_order", "name":"original furniture item",
                    "job_token":"CONSTRUCT_BED", "amount":1
                }}]),
            )?;
            let waited = parsed(&wait_with_ticks(
                Some(session.clone()),
                Some(effects::WORK_ORDER_TICKS_PER_UNIT),
            ))?;
            assert_eq!(waited["ok"], true, "{waited}");
            let handle = lookup_session_str(&session)?;
            let guard = handle.lock().map_err(|_| "test session lock poisoned")?;
            let items =
                effects::loose_furniture_items(guard.adapter.snapshot(), FurnitureKind::Bed)?;
            assert_eq!(items.len(), 1);
            json!({
                "kind":"place_furniture", "furniture":"bed",
                "item":items[0].get().to_string(), "location":[0,0,10]
            })
        }
    };
    let digest = commit(&session, json!([{"action":action}]))?;
    directory.restart();
    // Only read and clock authority returns; no original dispatch handle does.
    let session = open("7855901", scenario, false)?;
    {
        let handle = lookup_session_str(&session)?;
        let mut guard = handle.lock().map_err(|_| "test session lock poisoned")?;
        assert!(guard.carried.iter().any(|row| {
            row.plan_digest == digest && row.state == "dispatched" && row.monitor.is_some()
        }));
        // Finish the physical effect without first publishing its proof.
        guard.adapter.advance_ticks(work.ticks())?;
    }
    Ok(Fixture {
        session,
        digest,
        _directory: directory,
    })
}

#[derive(Clone, Copy, Debug)]
enum Damage {
    WorkGeneration,
    LedgerGeneration,
    AmbiguousLedger,
    FurnitureItemGeneration,
    DuplicateFurnitureAttachment,
}

fn damage(
    snapshot: &mut WorldSnapshot,
    step: &dfmcp_intent::PlanStep,
    damage: Damage,
) -> TestResult {
    let work = effects::created_entity_id(&step.idempotency_key, 0);
    match damage {
        Damage::WorkGeneration => {
            snapshot
                .graph
                .entities
                .get_mut(&work)
                .ok_or("work missing")?
                .generation += 1;
        }
        Damage::LedgerGeneration | Damage::AmbiguousLedger => {
            let ledger = effects::stock_ledger(snapshot).ok_or("stock ledger missing")?;
            let record = snapshot
                .graph
                .entities
                .get_mut(&ledger)
                .ok_or("stock ledger missing")?;
            if matches!(damage, Damage::LedgerGeneration) {
                record.generation += 1;
            } else {
                let mut duplicate = record.clone();
                duplicate.id = EntityId::new(9_999);
                snapshot.graph.entities.insert(duplicate.id, duplicate);
            }
        }
        Damage::FurnitureItemGeneration => {
            let Action::PlaceFurniture { item, .. } = &step.action else {
                return Err("placement action missing".into());
            };
            snapshot
                .graph
                .entities
                .get_mut(item)
                .ok_or("furniture item missing")?
                .generation += 1;
        }
        Damage::DuplicateFurnitureAttachment => {
            let mut duplicate = snapshot
                .graph
                .entities
                .get(&work)
                .ok_or("building missing")?
                .clone();
            duplicate.id = EntityId::new(9_999);
            snapshot.graph.entities.insert(duplicate.id, duplicate);
        }
    }
    snapshot.cursor.sequence += 1;
    snapshot.refresh_hash();
    Ok(())
}

#[test]
fn recovered_completion_refuses_replaced_identity_and_contradictory_input_custody() -> TestResult {
    let _serial = serialized_durable_tests();
    for (work, damage_kind) in [
        (Work::Construction, Damage::WorkGeneration),
        (Work::Construction, Damage::LedgerGeneration),
        (Work::Construction, Damage::AmbiguousLedger),
        (Work::Provisions, Damage::WorkGeneration),
        (Work::Provisions, Damage::LedgerGeneration),
        (Work::Provisions, Damage::AmbiguousLedger),
        (Work::Furniture, Damage::WorkGeneration),
        (Work::Furniture, Damage::FurnitureItemGeneration),
        (Work::Furniture, Damage::DuplicateFurnitureAttachment),
    ] {
        let label = format!("{work:?}-{damage_kind:?}");
        let fixture = fixture(&label, work)?;
        let handle = lookup_session_str(&fixture.session)?;
        let mut guard = handle.lock().map_err(|_| "test session lock poisoned")?;
        let before = guard
            .carried
            .iter()
            .find(|row| row.plan_digest == fixture.digest)
            .ok_or("carried original missing")?
            .clone();
        let step = before
            .work_step
            .as_ref()
            .ok_or("original work step missing")?;
        let mut changed = guard.adapter.snapshot().clone();
        damage(&mut changed, step, damage_kind)?;
        // Ordinary terminal predicates still hold: the additional identity
        // proof is the only thing preventing a false immutable receipt.
        assert!(
            PredicateEvidence::laboratory(&changed)?.establishes(&before.proof)?,
            "{label}"
        );
        guard.adapter.inject_snapshot(changed)?;
        assert!(observe_carried(&mut guard).is_err(), "{label}");
        let retained = guard
            .carried
            .iter()
            .find(|row| row.plan_digest == fixture.digest)
            .ok_or("carried original vanished")?;
        assert_eq!(retained.state, "dispatched", "{label}");
        assert_eq!(retained.proof_anchor, before.proof_anchor, "{label}");
        assert!(retained.observation_error.is_some(), "{label}");
        assert!(
            !matches!(
                retained
                    .monitor
                    .as_ref()
                    .and_then(RecoveredObligation::status),
                Some(ObligationStatus::Fulfilled { .. })
            ),
            "{label}"
        );

        // World persistence must not smuggle the rejected proof into the
        // durable frontier or the immutable retired-commit archive.
        persist_durable_head(&mut guard);
        with_durable_store(|store| {
            let commit = store
                .commit(guard.fortress_id, fixture.digest)
                .ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::InternalInvariantViolation,
                        "rejected proof retired its commit",
                    )
                })?;
            assert_eq!(
                commit.steps.get(&step.id.get()).map(String::as_str),
                Some("dispatched"),
                "{label}"
            );
            assert!(
                store
                    .retired_commit(guard.fortress_id, fixture.digest)
                    .is_none(),
                "{label}"
            );
            Ok(())
        })?;
    }
    Ok(())
}

#[test]
fn recovered_completion_publishes_and_archives_valid_original_custody() -> TestResult {
    let _serial = serialized_durable_tests();
    for work in [Work::Construction, Work::Provisions, Work::Furniture] {
        let fixture = fixture(&format!("valid-{work:?}"), work)?;
        let handle = lookup_session_str(&fixture.session)?;
        let mut guard = handle.lock().map_err(|_| "test session lock poisoned")?;
        observe_carried(&mut guard)?;
        let retained = guard
            .carried
            .iter()
            .find(|row| row.plan_digest == fixture.digest)
            .ok_or("carried original missing")?;
        assert_eq!(retained.state, "verified", "{work:?}");
        assert!(retained.is_final(), "{work:?}");
        assert_eq!(
            retained.proof_anchor,
            Some(guard.adapter.snapshot().anchor())
        );
        let step = retained.step.ok_or("original step missing")?;
        persist_durable_head(&mut guard);
        with_durable_store(|store| {
            assert!(store.commit(guard.fortress_id, fixture.digest).is_none());
            let retired = store
                .retired_commit(guard.fortress_id, fixture.digest)
                .ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::InternalInvariantViolation,
                        "valid original proof was not archived",
                    )
                })?;
            assert_eq!(
                retired.steps.get(&step.get()).map(String::as_str),
                Some("verified")
            );
            assert_eq!(
                retired.step_anchors.get(&step.get()),
                Some(&guard.adapter.snapshot().anchor())
            );
            Ok(())
        })?;
    }
    Ok(())
}
