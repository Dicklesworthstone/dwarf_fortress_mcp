//! Optional run/1.13 source authority is issued independently of order insertion
//! and the separate order-run/1.14 owner.
//! This profile does not encode a fortress lineage. Never reinterpret a named
//! fortress grant as native clock authority or use a clock receipt as goal proof.
use super::{
    Arc, Config, Deserialize, INVENTORY_CLOCK_TOKEN, MIB, Mode, Work, denied, digest, invalid,
    nonce, token,
};
use dfmcp_adapter::bounded_run::journal::{RunBinding, RunJournal, RunMode};
use dfmcp_adapter::bounded_run::private_file::{PrivateRunFile, open_private_run_journal_exact};
use dfmcp_adapter::bounded_run::rpc::{RunManifest, RunRpcClient, RunSource, RunTcpStream};
use dfmcp_adapter::bounded_run::{RunObservation, RunPlan, RunRecord};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, Digest32, FortressId, ObservationCursor,
    OperationContext, Result, RiskTier, StateAnchor,
};
use std::net::SocketAddr;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct InventoryClockInput {
    generation: u64,
    #[serde(default, deserialize_with = "journal_identity")]
    journal_id: Option<String>,
}
fn journal_identity<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<String>, D::Error> {
    String::deserialize(d).map(Some)
}
#[derive(Clone)]
pub(super) struct InventoryClockConfig {
    generation: u64,
    journal_id: Option<Digest32>,
}
impl InventoryClockConfig {
    pub(super) fn journal_id(&self) -> Option<Digest32> {
        self.journal_id
    }
}
impl InventoryClockInput {
    pub(super) fn validate(self, initialize: bool) -> Result<InventoryClockConfig> {
        if self.generation == 0
            || self.generation == u64::MAX
            || initialize == self.journal_id.is_some()
        {
            return Err(invalid(
                "inventory clock requires an exact source incarnation and an external journal identity only on reopening",
            ));
        }
        Ok(InventoryClockConfig {
            generation: self.generation,
            journal_id: self.journal_id.as_deref().map(digest).transpose()?,
        })
    }
}

/// This is an independent operator-authority issuer, not a grant conversion.
/// The named-fortress request still needs its own Plan/ControlClock permission;
/// the native owner receives only source-scoped grants under this separate gate.
pub(super) fn context(
    config: &Config,
    canonical: &OperationContext,
    control: bool,
) -> Result<OperationContext> {
    config.current()?;
    if config.inventory_clock.is_none() || canonical.anchor.fortress_id != config.sources.fortress()
    {
        return Err(denied(
            "source clock authority requires the configured original fortress context",
        ));
    }
    canonical.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if control {
        if !Config::inventory_clock_enabled() {
            return Err(denied(
                "independent operator inventory clock authority is disabled",
            ));
        }
        canonical.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
    }
    issue_context(canonical, control)
}
fn issue_context(canonical: &OperationContext, control: bool) -> Result<OperationContext> {
    canonical.budget.validate()?;
    let mut capabilities = vec![Capability::Query];
    if control {
        capabilities.extend([Capability::Plan, Capability::ControlClock]);
    }
    Ok(OperationContext {
        session_id: canonical.session_id,
        request_id: canonical.request_id,
        anchor: StateAnchor {
            fortress_id: FortressId::NIL,
            cursor: ObservationCursor::ORIGIN,
            tick: canonical.anchor.tick,
            state_hash: Digest32::ZERO,
        },
        budget: canonical.budget,
        grants: capabilities
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(FortressId::NIL),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::Guarded,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect(),
        cancellation_requested: canonical.cancellation_requested,
    })
}
fn identity(config: &Config, manifest: &RunManifest, endpoint: Option<SocketAddr>) -> Result<()> {
    let clock = config
        .inventory_clock
        .as_ref()
        .ok_or_else(|| denied("operator inventory clock source is not configured"))?;
    if manifest.generation != clock.generation
        || manifest.df_version != config.sources.df_version
        || manifest.dfhack_version != config.sources.dfhack_version
        || endpoint != Some(config.endpoint)
    {
        return Err(denied(
            "clock source differs from its exact operator endpoint, versions or incarnation",
        ));
    }
    Ok(())
}
pub struct PinnedInventoryClock {
    inner: RunRpcClient<RunTcpStream>,
    config: Arc<Config>,
    pinned_credential: Vec<u8>,
    fenced: bool,
}
impl PinnedInventoryClock {
    fn check(&mut self, c: &OperationContext, control: bool) -> Result<()> {
        if self.fenced {
            return Err(denied(
                "clock source is fenced after an earlier boundary failure",
            ));
        }
        let result = (|| {
            self.config.current()?;
            if token(INVENTORY_CLOCK_TOKEN)? != self.pinned_credential {
                return Err(denied(
                    "operator inventory clock credential changed; close this source",
                ));
            }
            identity(&self.config, self.inner.manifest(), self.inner.endpoint())?;
            if c.anchor.fortress_id != FortressId::NIL {
                return Err(denied("native clock requires independent source authority"));
            }
            c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
            if control {
                if !Config::inventory_clock_enabled() {
                    return Err(denied("operator inventory clock authority was revoked"));
                }
                c.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
            }
            self.inner.restrict_deadline(c)
        })();
        if result.is_err() {
            self.fenced = true;
        }
        result
    }
}
impl RunSource for PinnedInventoryClock {
    fn manifest(&self) -> &RunManifest {
        self.inner.manifest()
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        self.inner.endpoint()
    }
    fn observe(&mut self, c: &OperationContext) -> Result<RunObservation> {
        self.check(c, false)?;
        let value = self.inner.observe(c)?;
        self.check(c, false)?;
        Ok(value)
    }
    fn prepare(&mut self, p: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        self.check(c, true)?;
        let value = self.inner.prepare(p, c)?;
        self.check(c, false)?;
        Ok(value)
    }
    fn commit(&mut self, p: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        self.check(c, true)?;
        let value = self.inner.commit(p, c)?;
        self.check(c, false)?;
        Ok(value)
    }
    fn query(&mut self, p: &RunPlan, c: &OperationContext) -> Result<Option<RunRecord>> {
        self.check(c, false)?;
        let value = self.inner.query(p, c)?;
        self.check(c, false)?;
        Ok(value)
    }
    fn cancel(&mut self, p: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        self.check(c, true)?;
        let value = self.inner.cancel(p, c)?;
        self.check(c, false)?;
        Ok(value)
    }
}
pub(super) fn connect(config: &Arc<Config>, c: &OperationContext) -> Result<PinnedInventoryClock> {
    config.current()?;
    if config.inventory_clock.is_none() {
        return Err(denied("operator inventory clock source is not configured"));
    }
    let work = Work::new(c)?;
    let current = work.current()?;
    let connection_nonce = nonce(&current, b"inventory-clock");
    let pinned_credential = token(INVENTORY_CLOCK_TOKEN)?;
    config.current()?;
    let inner = RunRpcClient::connect(
        config.endpoint,
        pinned_credential.clone(),
        connection_nonce,
        &work.narrow(&current)?,
    )?;
    let mut source = PinnedInventoryClock {
        inner,
        config: Arc::clone(config),
        pinned_credential,
        fenced: false,
    };
    source.check(&work.narrow(&current)?, false)?;
    Ok(source)
}
pub(super) fn open(
    config: &Arc<Config>,
    mode: Mode,
    canonical: &OperationContext,
) -> Result<Option<RunJournal<PrivateRunFile>>> {
    config.current()?;
    let Some(clock) = &config.inventory_clock else {
        match std::fs::symlink_metadata(config.path("inventory-clock.journal")) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            _ => {
                return Err(denied(
                    "existing or unverifiable inventory clock custody requires its exact configuration and recovery identity",
                ));
            }
        }
    };
    let mode = match mode {
        Mode::Offline => RunMode::Offline,
        Mode::Control if Config::inventory_clock_enabled() => RunMode::Control,
        _ => RunMode::Recover,
    };
    if config.initialize && mode != RunMode::Control {
        return Err(denied(
            "inventory clock initialization requires independent operator inventory clock control",
        ));
    }
    let native = context(config, canonical, mode == RunMode::Control)?;
    let mut work = Work::new(&native)?;
    let binding = RunBinding::new(
        config.endpoint,
        RunManifest {
            generation: clock.generation,
            df_version: config.sources.df_version.clone(),
            dfhack_version: config.sources.dfhack_version.clone(),
        },
    )?;
    let inspect = work.reserve(8 * MIB)?;
    let mut journal = open_private_run_journal_exact(
        &config.path("inventory-clock.journal"),
        &work.rest()?,
        mode,
        Some(binding.clone()),
        config.initialize,
    )?;
    journal.records(&work.narrow(&inspect)?)?;
    if journal.binding() != &binding || clock.journal_id.is_some_and(|id| id != journal.id()) {
        return Err(denied(
            "inventory clock custody differs from its exact source or externally retained journal identity",
        ));
    }
    config.current()?;
    work.current()?;
    Ok(Some(journal))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dfmcp_core::{GameTick, RequestId, SessionId, WorkBudget};
    use serde_json::{Value, json};

    fn config() -> Value {
        json!({"schema":"dfmcp.semantic-work-order-development/1",
            "directory":"/tmp/semantic-inventory-clock-config","endpoint":"127.0.0.1:5000","initialize":true,
            "sources":{"kind":"operator-development-source-acceptance/1","world_folder":"region1","site_id":7,
                "df_version":"df","dfhack_version":"dfhack","citizen_bridge_version":"citizens","citizen_generation":10,
                "work_order_generation":20,"progress_generation":30,"operations_generation":40}})
    }
    #[test]
    fn optional_inventory_clock_is_closed_separate_and_requires_exact_recovery_identity()
    -> Result<()> {
        let mut raw = config();
        assert!(Config::parse(&raw.to_string())?.inventory_clock.is_none());
        raw["inventory_clock"] = json!({"generation":60});
        let parsed = Config::parse(&raw.to_string())?;
        assert!(parsed.clock.is_none());
        assert!(parsed.inventory_clock.is_some());
        assert!(parsed.validate_combined_custody().is_err());
        for value in [
            json!({"generation":0}),
            json!({"generation":u64::MAX}),
            json!({"generation":60,"native_order_id":7}),
            json!({"generation":60,"journal_id":null}),
            json!({"generation":60,"journal_id":"ab".repeat(32)}),
        ] {
            raw["inventory_clock"] = value;
            assert!(Config::parse(&raw.to_string()).is_err());
        }
        raw["initialize"] = json!(false);
        raw["inventory_clock"] = json!({"generation":60});
        assert!(Config::parse(&raw.to_string()).is_err());
        raw["inventory_clock"]["journal_id"] = json!("ab".repeat(32));
        Config::parse(&raw.to_string())?;
        raw["recovery"] = json!({"plan_digest":"cd".repeat(32),"object_digest":"ef".repeat(32),"history_present":false,
            "anchor":{"fortress_id":parsed.sources.fortress().get().to_string(),"epoch":0,"sequence":1,"tick":100,"state_hash":"12".repeat(32)}});
        assert!(Config::parse(&raw.to_string()).is_err());
        raw["recovery"]["inventory_clock_journal_id"] = json!("cd".repeat(32));
        assert!(Config::parse(&raw.to_string()).is_err());
        raw["recovery"]["inventory_clock_journal_id"] = json!("ab".repeat(32));
        Config::parse(&raw.to_string())?;
        raw["inventory_clock"] = Value::Null;
        assert!(Config::parse(&raw.to_string()).is_err());
        Ok(())
    }
    #[test]
    fn inventory_source_pins_are_independent_of_legacy_clock_generation() -> Result<()> {
        let mut raw = config();
        raw["clock"] = json!({"generation":50});
        raw["inventory_clock"] = json!({"generation":60});
        let parsed = Config::parse(&raw.to_string())?;
        let manifest = RunManifest {
            generation: 60,
            df_version: "df".into(),
            dfhack_version: "dfhack".into(),
        };
        let endpoint = Some(SocketAddr::from(([127, 0, 0, 1], 5000)));
        identity(&parsed, &manifest, endpoint)?;
        for generation in [10, 20, 30, 40, 50, 59, 61] {
            assert!(
                identity(
                    &parsed,
                    &RunManifest {
                        generation,
                        ..manifest.clone()
                    },
                    endpoint
                )
                .is_err()
            );
        }
        assert!(
            identity(
                &parsed,
                &RunManifest {
                    df_version: "changed".into(),
                    ..manifest.clone()
                },
                endpoint
            )
            .is_err()
        );
        assert!(
            identity(
                &parsed,
                &RunManifest {
                    dfhack_version: "changed".into(),
                    ..manifest.clone()
                },
                endpoint
            )
            .is_err()
        );
        assert!(identity(&parsed, &manifest, None).is_err());
        assert!(
            identity(
                &parsed,
                &manifest,
                Some(SocketAddr::from(([127, 0, 0, 1], 5001)))
            )
            .is_err()
        );
        Ok(())
    }
    #[test]
    fn independent_inventory_source_issuer_never_rewrites_named_fortress_grants() -> Result<()> {
        let canonical = OperationContext {
            session_id: SessionId::new(1),
            request_id: RequestId::new(2),
            anchor: StateAnchor {
                fortress_id: FortressId::new(7),
                cursor: ObservationCursor::ORIGIN,
                tick: GameTick(100),
                state_hash: Digest32::ZERO,
            },
            budget: WorkBudget {
                max_bytes: 16 * MIB,
                max_wall_millis: 5000,
                max_entities: 4096,
                max_actions: 1,
                max_output_tokens: 32768,
                max_game_ticks: 1200,
            },
            grants: vec![CapabilityGrant {
                capability: Capability::Query,
                scope: CapabilityScope {
                    fortress_id: Some(FortressId::new(7)),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: Some(GameTick(105)),
                remaining_uses: Some(1),
            }],
            cancellation_requested: false,
        };
        let source = issue_context(&canonical, false)?;
        assert_eq!(source.anchor.fortress_id, FortressId::NIL);
        assert_eq!(source.session_id, canonical.session_id);
        assert_eq!(source.request_id, canonical.request_id);
        assert_eq!(source.budget, canonical.budget);
        assert_eq!(source.grants.len(), 1);
        assert_eq!(source.grants[0].scope.fortress_id, Some(FortressId::NIL));
        assert_eq!(
            canonical.grants[0].scope.fortress_id,
            Some(FortressId::new(7))
        );
        assert!(
            source
                .authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)
                .is_err()
        );
        Ok(())
    }

    fn custody_context(config: &Config, control: bool) -> OperationContext {
        let capabilities = if control {
            vec![Capability::Query, Capability::ControlClock]
        } else {
            vec![Capability::Query]
        };
        OperationContext {
            session_id: SessionId::new(1301),
            request_id: RequestId::new(1302),
            anchor: StateAnchor {
                fortress_id: config.sources.fortress(),
                cursor: ObservationCursor::ORIGIN,
                tick: GameTick(0),
                state_hash: Digest32::ZERO,
            },
            budget: WorkBudget {
                max_bytes: 16 * MIB,
                max_wall_millis: 5000,
                max_entities: 4096,
                max_actions: 1,
                max_output_tokens: 32768,
                max_game_ticks: 1200,
            },
            grants: capabilities
                .into_iter()
                .map(|capability| CapabilityGrant {
                    capability,
                    scope: CapabilityScope {
                        fortress_id: Some(config.sources.fortress()),
                        ..CapabilityScope::default()
                    },
                    max_risk: RiskTier::Guarded,
                    expires_at_tick: None,
                    remaining_uses: None,
                })
                .collect(),
            cancellation_requested: false,
        }
    }

    #[test]
    #[cfg(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))]
    fn inventory_private_custody_cannot_be_omitted_or_recreated_and_needs_no_offline_token()
    -> Result<()> {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        const CHILD: &str = "SEMANTIC_INVENTORY_CLOCK_TEST_CASE";
        const CHECKED: &str = "semantic-inventory-clock-custody-checked";
        if let Ok(case) = std::env::var(CHILD) {
            assert!(std::env::var(INVENTORY_CLOCK_TOKEN).is_err());
            let config = Arc::new(Config::from_environment()?);
            let c = custody_context(&config, false);
            let mode = match case.as_str() {
                "reconcile" => Mode::Reconcile,
                "revoked_control" | "ungranted_control" => Mode::Control,
                _ => Mode::Offline,
            };
            let result = open(&config, mode, &c);
            if matches!(
                case.as_str(),
                "wrong_id" | "omitted" | "missing" | "ungranted_control"
            ) {
                assert!(result.is_err());
            } else {
                let mut journal =
                    result?.ok_or_else(|| invalid("inventory clock fixture not opened"))?;
                assert_eq!(
                    journal.mode(),
                    if mode == Mode::Offline {
                        RunMode::Offline
                    } else {
                        RunMode::Recover
                    }
                );
                let native = context(&config, &c, false)?;
                assert!(journal.records(&native)?.is_empty());
                assert_eq!(
                    Some(journal.id()),
                    config
                        .inventory_clock
                        .as_ref()
                        .and_then(InventoryClockConfig::journal_id)
                );
            }
            println!("{CHECKED}");
            return Ok(());
        }
        let storage_error = |_| denied("private inventory clock fixture storage failed");
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| invalid("fixture timestamp"))?
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "dfmcp-inventory-clock-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir(&directory).map_err(storage_error)?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
            .map_err(storage_error)?;
        let directory = directory.canonicalize().map_err(storage_error)?;
        let mut raw = config();
        raw["directory"] = json!(directory);
        raw["inventory_clock"] = json!({"generation":60});
        let parsed = Config::parse(&raw.to_string())?;
        let native = issue_context(&custody_context(&parsed, true), true)?;
        let binding = RunBinding::new(
            parsed.endpoint,
            RunManifest {
                generation: 60,
                df_version: "df".into(),
                dfhack_version: "dfhack".into(),
            },
        )?;
        let path = directory.join("inventory-clock.journal");
        let journal =
            open_private_run_journal_exact(&path, &native, RunMode::Control, Some(binding), true)?;
        let id = journal.id();
        drop(journal);
        let original = fs::read(&path).map_err(storage_error)?;
        raw["initialize"] = json!(false);
        raw["inventory_clock"]["journal_id"] = json!(id.to_hex());
        let full_name = format!(
            "{}::inventory_private_custody_cannot_be_omitted_or_recreated_and_needs_no_offline_token",
            module_path!()
        );
        let (_, test_name) = full_name
            .split_once("::")
            .ok_or_else(|| invalid("fixture test name"))?;
        for case in [
            "offline",
            "reconcile",
            "revoked_control",
            "ungranted_control",
            "wrong_id",
            "omitted",
            "missing",
        ] {
            let mut input = raw.clone();
            if case == "wrong_id" {
                input["inventory_clock"]["journal_id"] =
                    json!(Digest32::of_bytes(b"other-inventory-clock").to_hex());
            } else if case == "omitted" {
                input
                    .as_object_mut()
                    .ok_or_else(|| invalid("fixture configuration"))?
                    .remove("inventory_clock");
            } else if case == "missing" {
                let missing = directory.join("missing");
                fs::create_dir(&missing).map_err(storage_error)?;
                fs::set_permissions(&missing, fs::Permissions::from_mode(0o700))
                    .map_err(storage_error)?;
                input["directory"] = json!(missing);
            }
            let mut command =
                std::process::Command::new(std::env::current_exe().map_err(storage_error)?);
            command
                .args(["--exact", test_name, "--nocapture"])
                .env_clear()
                .env(super::super::OPT_IN, "1")
                .env(super::super::CONFIG, input.to_string())
                .env(CHILD, case);
            if case == "ungranted_control" {
                command.env(super::super::INVENTORY_CLOCK_AUTHORITY, "1");
            }
            let output = command.output().map_err(storage_error)?;
            assert!(
                output.status.success(),
                "inventory clock custody case {case}: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(String::from_utf8_lossy(&output.stdout).contains(CHECKED));
            assert_eq!(fs::read(&path).map_err(storage_error)?, original);
        }
        assert!(!directory.join("missing/inventory-clock.journal").exists());
        Ok(())
    }
}
