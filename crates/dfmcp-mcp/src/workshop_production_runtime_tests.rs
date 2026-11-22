//! Actual staged owner over private Workshop, ACK, workforce and order custody.
//! Source vectors seed a completed original; no credential, native endpoint or
//! game supplies the staged handoff assertions. Environment runs are isolated.
#![cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]

use super::{Mode, Runtime};
use crate::workshop_construction_server::runtime::production_handoff_tests::{
    Fixture as WorkshopFixture, completed_original_fixture,
};
use crate::workshop_production_config::{Config, Stage};
use dfmcp_adapter::semantic_work_orders::open_private_association_store as order_associations;
use dfmcp_adapter::semantic_workforce::open_private_association_store as workforce_associations;
use dfmcp_adapter::work_order_control::{JournalMode, open_private_work_orders_exact};
use dfmcp_adapter::workforce_control::{
    journal::{WorkforceBinding, WorkforceMode},
    private_file::open_private_workforce_exact,
    rpc::WorkforceManifest,
};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, DfmcpError, Digest32, ErrorCode, GameTick,
    ObservationCursor, OperationContext, RequestId, Result, RiskTier, SessionId, StateAnchor,
    WorkBudget,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::net::SocketAddr;
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

const CHILD: &str = "STAGED_WORKSHOP_TEST_CHILD";
const CASE: &str = "STAGED_WORKSHOP_TEST_CASE";
const OUTPUT: &str = "STAGED_WORKSHOP_RESULT=";
const PARENT_CONFIG: &str = "DFMCP_WORKSHOP_PRODUCTION_CONFIG";
const WORKSHOP_CONFIG: &str = "DFMCP_WORKSHOP_MCP_CONFIG";
const PRODUCTION_CONFIG: &str = "DFMCP_COMBINED_PRODUCTION_CONFIG";
const WORKFORCE_CONFIG: &str = "DFMCP_SEMANTIC_WORKFORCE_CONFIG";
const ORDER_CONFIG: &str = "DFMCP_SEMANTIC_WORK_ORDER_CONFIG";
fn session() -> SessionId {
    SessionId::new(1902)
}

fn fixture_error() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::InternalInvariantViolation,
        "staged custody fixture failure",
    )
}

fn budget() -> WorkBudget {
    WorkBudget {
        max_wall_millis: 60000,
        max_bytes: 8 * 1024 * 1024 * 1024,
        max_entities: 8192,
        max_output_tokens: 32768,
        max_actions: 5,
        max_game_ticks: 0,
    }
}

fn runtime_io() -> Result<()> {
    let cx = asupersync::Cx::current().ok_or_else(fixture_error)?;
    cx.checkpoint().map_err(|_| fixture_error())?;
    if !cx.capabilities().io {
        return Err(fixture_error());
    }
    Ok(())
}

fn open() -> Result<Runtime> {
    Runtime::open(
        Config::from_environment(runtime_io)?,
        Mode::Offline,
        session(),
        budget(),
    )
}

fn child(test: &str) -> bool {
    std::env::var(CHILD).ok().as_deref() == Some(test)
}

fn in_runtime(check: impl FnOnce() -> Result<Value>) -> Result<()> {
    let value =
        crate::run_with_runtime_cx(|_| async move { check() }).map_err(|_| fixture_error())??;
    println!("{OUTPUT}{value}");
    Ok(())
}

fn read_configuration(key: &str) -> Result<Value> {
    serde_json::from_str(&std::env::var(key).map_err(|_| fixture_error())?)
        .map_err(|_| fixture_error())
}

fn directory(key: &str) -> Result<PathBuf> {
    read_configuration(key)?["directory"]
        .as_str()
        .map(PathBuf::from)
        .ok_or_else(fixture_error)
}

fn custody() -> Result<BTreeMap<PathBuf, Vec<u8>>> {
    let mut result = BTreeMap::new();
    for key in [
        WORKSHOP_CONFIG,
        PRODUCTION_CONFIG,
        WORKFORCE_CONFIG,
        ORDER_CONFIG,
    ] {
        for entry in fs::read_dir(directory(key)?).map_err(|_| fixture_error())? {
            let entry = entry.map_err(|_| fixture_error())?;
            let kind = entry.file_type().map_err(|_| fixture_error())?;
            if kind.is_file() {
                result.insert(
                    entry.path(),
                    fs::read(entry.path()).map_err(|_| fixture_error())?,
                );
            } else {
                return Err(fixture_error());
            }
        }
    }
    Ok(result)
}

struct Fixture {
    workshop: WorkshopFixture,
    directory: PathBuf,
    values: BTreeMap<String, String>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

impl Fixture {
    fn new() -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let directory = std::env::temp_dir().join(format!(
            "dfmcp-staged-workshop-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .map_err(|_| fixture_error())?;
        let workshop = completed_original_fixture()?;
        let mut fixture = Self {
            workshop,
            directory,
            values: BTreeMap::new(),
        };
        for name in ["production", "workforce", "orders"] {
            fs::DirBuilder::new()
                .mode(0o700)
                .create(fixture.directory.join(name))
                .map_err(|_| fixture_error())?;
        }
        let workshop = fixture.workshop.configuration();
        let folder = workshop["world_folder"]
            .as_str()
            .ok_or_else(fixture_error)?
            .to_owned();
        let site = u32::try_from(workshop["site_id"].as_u64().ok_or_else(fixture_error)?)
            .map_err(|_| fixture_error())?;
        let binding = WorkforceBinding::new(
            SocketAddr::from(([127, 0, 0, 1], 5000)),
            WorkforceManifest {
                generation: 20,
                df_version: "test-df".to_owned(),
                dfhack_version: "test-dfhack".to_owned(),
            },
            folder.clone(),
            site,
        )?;
        let c = OperationContext {
            session_id: SessionId::new(1901),
            request_id: RequestId::new(1),
            anchor: StateAnchor {
                fortress_id: binding.fortress(),
                cursor: ObservationCursor::ORIGIN,
                tick: GameTick(0),
                state_hash: Digest32::ZERO,
            },
            budget: budget(),
            grants: [
                Capability::Query,
                Capability::Plan,
                Capability::ConfigureLabor,
                Capability::ConfigureProduction,
            ]
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(binding.fortress()),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::Guarded,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect(),
            cancellation_requested: false,
        };
        let mut workforce = open_private_workforce_exact(
            &fixture.directory.join("workforce/workforce.journal"),
            &c,
            WorkforceMode::Control,
            Some(binding),
            true,
        )?;
        let workforce_id = workforce.view(&c)?.id;
        let wa = workforce_associations(
            &fixture.directory.join("workforce/semantic.associations"),
            workforce_id,
            true,
            false,
            &c,
        )?;
        let orders = open_private_work_orders_exact(
            &fixture.directory.join("orders/creation.journal"),
            &c,
            JournalMode::Control,
            true,
        )?;
        let order_id = orders.summary(&c)?.journal_id;
        let oa = order_associations(
            &fixture.directory.join("orders/semantic.associations"),
            order_id,
            true,
            false,
            &c,
        )?;
        drop((workforce, wa, orders, oa));
        for key in [
            "DFMCP_ALLOW_UNADMITTED_WORKSHOP_PRODUCTION",
            "DFMCP_ALLOW_UNADMITTED_WORKSHOP_V1_20",
            "DFMCP_ALLOW_UNADMITTED_COMBINED_PRODUCTION",
            "DFMCP_ALLOW_UNADMITTED_SEMANTIC_WORKFORCE",
            "DFMCP_ALLOW_UNADMITTED_SEMANTIC_WORK_ORDERS",
        ] {
            fixture.values.insert(key.to_owned(), "1".to_owned());
        }
        fixture.set(
            PARENT_CONFIG,
            json!({
                "schema":"dfmcp.workshop-production-development/1",
                "recovery":{"stage":"workshop","authority_tick_floor":0}
            }),
        );
        fixture.set(WORKSHOP_CONFIG, workshop);
        fixture.set(
            PRODUCTION_CONFIG,
            json!({
                "schema":"dfmcp.combined-production-development/1",
                "directory":fixture.directory.join("production"),"initialize":false
            }),
        );
        fixture.set(WORKFORCE_CONFIG, json!({
            "schema":"dfmcp.semantic-workforce-development/1",
            "directory":fixture.directory.join("workforce"),"endpoint":"127.0.0.1:5000",
            "initialize":false,"journal_id":workforce_id.to_hex(),
            "sources":{"kind":"operator-development-source-acceptance/1",
                "world_folder":folder,"site_id":site,"df_version":"test-df","dfhack_version":"test-dfhack",
                "citizen_bridge_version":"bridge","citizen_generation":10,"workforce_generation":20}
        }));
        fixture.set(ORDER_CONFIG, json!({
            "schema":"dfmcp.semantic-work-order-development/1",
            "directory":fixture.directory.join("orders"),"endpoint":"127.0.0.1:5000",
            "initialize":false,
            "sources":{"kind":"operator-development-source-acceptance/1",
                "world_folder":folder,"site_id":site,"df_version":"test-df","dfhack_version":"test-dfhack",
                "citizen_bridge_version":"bridge","citizen_generation":10,
                "work_order_generation":21,"progress_generation":22,"operations_generation":23}
        }));
        Ok(fixture)
    }

    fn set(&mut self, key: &str, value: Value) {
        self.values.insert(key.to_owned(), value.to_string());
    }

    fn run(&self, test: &str, case: &str) -> Result<Value> {
        let mut command = Command::new(std::env::current_exe().map_err(|_| fixture_error())?);
        command
            .arg("--exact")
            .arg(format!("workshop_production_runtime::tests::{test}"))
            .arg("--nocapture");
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("DFMCP_") {
                command.env_remove(key);
            }
        }
        command.envs(&self.values).env(CHILD, test).env(CASE, case);
        let output = command.output().map_err(|_| fixture_error())?;
        assert!(
            output.status.success(),
            "{case}: stdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("1 passed"),
            "isolated actual-source test was not selected"
        );
        let result = stdout
            .lines()
            .find_map(|line| line.split_once(OUTPUT).map(|(_, value)| value))
            .ok_or_else(fixture_error)?;
        serde_json::from_str(result).map_err(|_| fixture_error())
    }
}

#[test]
fn staged_handoff_imports_one_exact_original_without_rewriting_custody() -> Result<()> {
    let test = "staged_handoff_imports_one_exact_original_without_rewriting_custody";
    if child(test) {
        return in_runtime(|| {
            let before = custody()?;
            let mut runtime = open()?;
            let first = runtime.context()?;
            assert_eq!(runtime.session_id(), session());
            assert_eq!(first.session_id, session());
            assert_eq!(runtime.mode(), Mode::Offline);
            assert_eq!(runtime.budget(), budget());
            assert!(runtime.canonical_anchor().is_none());
            let initial = runtime.recovery();
            let native = initial["production"]["native_custody"].clone();
            let original = initial["workshop"]["original"].clone();
            let pin = initial["workshop"]["acknowledged"].clone();
            let value = runtime.begin_production(&first)?;
            assert_eq!(runtime.stage(), Stage::Production);
            assert!(
                runtime.workshop.is_none(),
                "the construction writer must be dropped"
            );
            assert!(runtime.attached);
            let recovery = runtime.recovery();
            let handoff = &recovery["production_handoff"];
            let prerequisite = &value["workshop_prerequisite"];
            assert_eq!(recovery["production"]["native_custody"], native);
            assert_eq!(handoff["workshop_prerequisite"]["acknowledged"], pin);
            assert_eq!(
                handoff["workshop_prerequisite"]["original_key"],
                original["key"]
            );
            assert_eq!(
                handoff["workshop_prerequisite"]["original_plan_digest"],
                original["plan_digest"]
            );
            assert_eq!(prerequisite["workshop_key"], original["key"]);
            assert_eq!(
                prerequisite["workshop_plan_digest"],
                original["plan_digest"]
            );
            assert_eq!(
                prerequisite["workshop_receipt"],
                handoff["original_receipt"]
            );
            assert_eq!(prerequisite["building_id"], 70);
            assert_eq!(prerequisite["mutation_authority"], false);
            assert_eq!(recovery["current_workshop_completion_proven"], false);
            assert_eq!(recovery["complete_cross_stage_goal_proven"], false);
            assert_eq!(recovery["automatic_redispatch"], false);
            assert_eq!(handoff["native_calls"], 0);
            assert_eq!(handoff["retained_custody_verified"], false);
            assert_eq!(handoff["verified_at_export"], true);
            assert!(!runtime.effect_uncertain());
            assert!(
                runtime.canonical_anchor().is_none(),
                "handoff cannot manufacture a citizen snapshot"
            );
            let next = runtime.context()?;
            assert!(next.request_id.get() > first.request_id.get());
            assert_eq!(next.session_id, first.session_id);
            assert_eq!(
                next.anchor.tick,
                GameTick(0),
                "production canonical anchor remains unchanged"
            );
            assert!(
                recovery["parent"]["authority_tick_floor"]
                    .as_u64()
                    .ok_or_else(fixture_error)?
                    >= 13212
            );
            assert!(next.grants.iter().all(|grant| !matches!(
                grant.capability,
                Capability::Construct
                    | Capability::ConfigureLabor
                    | Capability::ConfigureProduction
            )));
            let repeated = runtime.begin_production(&next)?;
            assert_eq!(
                repeated["workshop_prerequisite"],
                value["workshop_prerequisite"]
            );
            assert_eq!(
                runtime.recovery()["parent"]["handoff_digest"],
                recovery["parent"]["handoff_digest"]
            );
            assert_eq!(
                custody()?,
                before,
                "Offline handoff and repeat must leave every journal byte unchanged"
            );
            Ok(recovery)
        });
    }
    let mut fixture = Fixture::new()?;
    let first = fixture.run(test, "handoff")?;
    fixture.set(
        PARENT_CONFIG,
        json!({
            "schema":"dfmcp.workshop-production-development/1", "recovery":first["parent"]
        }),
    );
    let second = fixture.run(test, "recovered-production")?;
    assert_eq!(second["parent"]["stage"], "production");
    assert_eq!(
        second["parent"]["handoff_digest"],
        first["parent"]["handoff_digest"]
    );
    assert_eq!(
        second["production"]["native_custody"],
        first["production"]["native_custody"]
    );
    assert_eq!(
        second["workshop"]["acknowledged"],
        first["workshop"]["acknowledged"]
    );
    Ok(())
}

#[test]
fn failed_attachment_retains_transition_and_retries_only_the_exact_read_import() -> Result<()> {
    let test = "failed_attachment_retains_transition_and_retries_only_the_exact_read_import";
    if child(test) {
        return in_runtime(|| {
            let mut runtime = open()?;
            let before = custody()?;
            let c = runtime.context()?;
            let native = runtime.recovery()["production"]["native_custody"].clone();
            let unexpected = directory(PRODUCTION_CONFIG)?.join("original.workshop");
            fs::write(&unexpected, b"unacknowledged foreign original")
                .map_err(|_| fixture_error())?;
            assert!(runtime.begin_production(&c).is_err());
            assert_eq!(runtime.stage(), Stage::Transition);
            assert!(
                runtime.workshop.is_none(),
                "failure must not restore the construction writer"
            );
            assert!(!runtime.attached);
            let refused = runtime.recovery();
            assert_eq!(refused["parent"]["stage"], "transition");
            assert!(refused["parent"]["handoff_digest"].as_str().is_some());
            assert_eq!(refused["production"]["native_custody"], native);
            assert_eq!(
                refused["production_handoff"]["historical_completion_proven"],
                true
            );
            assert_eq!(
                refused["production_handoff"]["current_completion_proven"],
                false
            );
            let stale = runtime
                .begin_production(&c)
                .err()
                .ok_or_else(fixture_error)?;
            assert_eq!(
                stale.code,
                ErrorCode::CapabilityDenied,
                "old Workshop authority cannot enter a later stage"
            );
            let current = runtime.context()?;
            assert!(
                runtime
                    .workshop_action(
                        crate::workshop_construction_server::runtime::Action::Inspect,
                        &current
                    )
                    .is_err()
            );
            let current = runtime.context()?;
            assert!(
                runtime
                    .production_action(crate::live_production_server::Action::Inspect, &current)
                    .is_err(),
                "production actions wait for successful attachment"
            );
            fs::remove_file(&unexpected).map_err(|_| fixture_error())?;
            let current = runtime.context()?;
            runtime.begin_production(&current)?;
            assert_eq!(runtime.stage(), Stage::Production);
            assert!(runtime.workshop.is_none());
            assert!(runtime.attached);
            let resumed = runtime.recovery();
            assert_eq!(
                resumed["parent"]["handoff_digest"],
                refused["parent"]["handoff_digest"]
            );
            assert_eq!(resumed["production"]["native_custody"], native);
            assert_eq!(
                custody()?,
                before,
                "retry may import, never reinitialize a native owner"
            );
            Ok(resumed)
        });
    }
    Fixture::new()?.run(test, "refuse-then-import")?;
    Ok(())
}

#[test]
fn staged_owner_rejects_stale_authority_and_prepays_transition_before_releasing_workshop()
-> Result<()> {
    let test =
        "staged_owner_rejects_stale_authority_and_prepays_transition_before_releasing_workshop";
    if child(test) {
        return in_runtime(|| {
            let mut runtime = open()?;
            let before = custody()?;
            let first = runtime.context()?;
            let issued = runtime.context()?;
            assert!(issued.request_id.get() > first.request_id.get());
            assert_eq!(
                runtime
                    .begin_production(&first)
                    .err()
                    .ok_or_else(fixture_error)?
                    .code,
                ErrorCode::CapabilityDenied
            );
            for kind in ["owner", "fortress", "grants", "cancel", "bytes"] {
                let mut c = runtime.context()?;
                match kind {
                    "owner" => c.session_id = SessionId::new(9999),
                    "fortress" => c.anchor.fortress_id = dfmcp_core::FortressId::NIL,
                    "grants" => c.grants.clear(),
                    "cancel" => c.cancellation_requested = true,
                    "bytes" => c.budget.max_bytes = 1,
                    _ => return Err(fixture_error()),
                }
                assert!(runtime.begin_production(&c).is_err(), "{kind}");
                assert_eq!(
                    runtime.stage(),
                    Stage::Workshop,
                    "{kind} must fail before ownership transfer"
                );
                assert!(runtime.workshop.is_some());
                assert!(runtime.handoff.is_none());
                assert!(!runtime.attached);
                assert_eq!(custody()?, before);
            }
            let recovery = runtime.recovery();
            assert!(
                recovery["parent"].get("handoff_digest").is_none(),
                "the exact Workshop recovery object must round-trip through the strict parser"
            );
            let c = runtime.context()?;
            runtime.inspect(&c)?;
            assert_eq!(custody()?, before);
            Ok(recovery)
        });
    }
    let mut fixture = Fixture::new()?;
    let first = fixture.run(test, "initial")?;
    fixture.set(
        PARENT_CONFIG,
        json!({
            "schema":"dfmcp.workshop-production-development/1", "recovery":first["parent"]
        }),
    );
    let recovered = fixture.run(test, "recovered-workshop")?;
    assert_eq!(recovered["parent"]["stage"], "workshop");
    assert_eq!(
        recovered["production"]["native_custody"],
        first["production"]["native_custody"]
    );
    Ok(())
}

#[test]
fn staged_recovery_preserves_authority_floor_and_rejects_a_different_handoff() -> Result<()> {
    let test = "staged_recovery_preserves_authority_floor_and_rejects_a_different_handoff";
    if child(test) {
        return in_runtime(|| {
            let before = custody()?;
            if std::env::var(CASE).ok().as_deref() == Some("wrong-handoff") {
                let cause = open().err().ok_or_else(fixture_error)?;
                assert_eq!(cause.code, ErrorCode::CapabilityDenied);
                assert_eq!(custody()?, before);
                return Ok(Value::Null);
            }
            let mut runtime = open()?;
            let c = runtime.context()?;
            runtime.begin_production(&c)?;
            let recovery = runtime.recovery();
            let floor = recovery["parent"]["authority_tick_floor"]
                .as_u64()
                .ok_or_else(fixture_error)?;
            assert!(
                floor >= 17000,
                "the externally retained floor survives old Workshop and production anchors"
            );
            let canonical = runtime.context()?;
            assert_eq!(canonical.anchor.tick, GameTick(0));
            assert!(runtime.canonical_anchor().is_none());
            let mut cancelled = canonical;
            cancelled.cancellation_requested = true;
            assert!(runtime.inspect(&cancelled).is_err());
            assert!(
                runtime.recovery()["parent"]["authority_tick_floor"]
                    .as_u64()
                    .ok_or_else(fixture_error)?
                    >= floor
            );
            assert_eq!(custody()?, before);
            Ok(recovery)
        });
    }
    let mut fixture = Fixture::new()?;
    fixture.set(
        PARENT_CONFIG,
        json!({
            "schema":"dfmcp.workshop-production-development/1",
            "recovery":{"stage":"workshop","authority_tick_floor":17000}
        }),
    );
    let initial = fixture.run(test, "floor-workshop")?;
    let mut recovered = initial["parent"].clone();
    recovered["stage"] = json!("transition");
    fixture.set(
        PARENT_CONFIG,
        json!({
            "schema":"dfmcp.workshop-production-development/1", "recovery":recovered
        }),
    );
    let transition = fixture.run(test, "floor-transition")?;
    assert_eq!(
        transition["parent"]["handoff_digest"],
        initial["parent"]["handoff_digest"]
    );
    let mut wrong = transition["parent"].clone();
    wrong["handoff_digest"] = json!(Digest32::of_bytes(b"another original handoff").to_hex());
    fixture.set(
        PARENT_CONFIG,
        json!({
            "schema":"dfmcp.workshop-production-development/1", "recovery":wrong
        }),
    );
    fixture.run(test, "wrong-handoff")?;
    Ok(())
}
