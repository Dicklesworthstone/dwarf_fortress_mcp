//! Optional order-run/1.14 resources remain separate from insertion authority.
//! The original-goal owner selects and seals the order; this shell binds the
//! native endpoint/incarnation and rechecks operator clock authority at effects.
use super::{
    Arc, CLOCK_TOKEN, Config, Deserialize, Digest32, MIB, Mode, OperationContext, Result, Work,
    denied, digest, invalid, nonce, token,
};
use dfmcp_adapter::order_run::journal::{OrderRunBinding, OrderRunJournal, OrderRunMode};
use dfmcp_adapter::order_run::private_file::{
    PrivateOrderRunFile, open_private_order_journal_exact,
};
use dfmcp_adapter::order_run::rpc::{
    OrderRunManifest, OrderRunRpc, OrderRunSource, OrderRunTcp, authorize,
};
use dfmcp_adapter::order_run::{FortressIdentity, OrderCapture, OrderRunPlan, OrderRunRecord};
use std::net::SocketAddr;
use std::time::Duration;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ClockInput {
    generation: u64,
    #[serde(default, deserialize_with = "journal_identity")]
    journal_id: Option<String>,
}
fn journal_identity<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error> {
    String::deserialize(deserializer).map(Some)
}
#[derive(Clone)]
pub(super) struct ClockConfig {
    generation: u64,
    journal_id: Option<Digest32>,
}
impl ClockConfig {
    pub(super) fn journal_id(&self) -> Option<Digest32> {
        self.journal_id
    }
}
impl ClockInput {
    pub(super) fn validate(self, initialize: bool) -> Result<ClockConfig> {
        if self.generation == 0
            || self.generation == u64::MAX
            || initialize == self.journal_id.is_some()
        {
            return Err(invalid(
                "clock requires an exact incarnation and an external journal identity only when reopening",
            ));
        }
        Ok(ClockConfig {
            generation: self.generation,
            journal_id: self.journal_id.as_deref().map(digest).transpose()?,
        })
    }
}

fn identity(
    config: &Config,
    manifest: &OrderRunManifest,
    endpoint: Option<SocketAddr>,
    fortress: &FortressIdentity,
) -> Result<()> {
    let clock = config
        .clock
        .as_ref()
        .ok_or_else(|| denied("operator clock source is not configured"))?;
    if manifest.generation != clock.generation
        || manifest.df_version != config.sources.df_version
        || manifest.dfhack_version != config.sources.dfhack_version
        || endpoint != Some(config.endpoint)
        || fortress.folder() != config.sources.world_folder
        || fortress.site() != config.sources.site_id
    {
        return Err(denied(
            "clock source differs from the exact operator endpoint, fortress or incarnation",
        ));
    }
    Ok(())
}

/// Every effect is checked again after the durable clock coordinator's marker.
/// A transport or operator failure never permits another unpause attempt.
pub struct PinnedClock {
    inner: OrderRunRpc<OrderRunTcp>,
    config: Arc<Config>,
}
impl PinnedClock {
    fn check(&mut self, context: &OperationContext, control: bool) -> Result<()> {
        let result = (|| {
            self.config.current()?;
            identity(
                &self.config,
                self.inner.manifest(),
                self.inner.endpoint(),
                self.inner.fortress(),
            )?;
            authorize(context, self.inner.fortress(), control)?;
            if control && !Config::clock_enabled() {
                return Err(denied("operator clock authority was revoked"));
            }
            Ok(())
        })();
        if result.is_err() {
            self.inner.fence();
        }
        result
    }
    fn capture(&mut self, value: &OrderCapture) -> Result<()> {
        if value.generation() != self.inner.manifest().generation
            || value.fortress() != self.inner.fortress()
        {
            self.inner.fence();
            return Err(denied(
                "clock capture differs from its exact configured source",
            ));
        }
        Ok(())
    }
    fn plan(&mut self, plan: &OrderRunPlan) -> Result<()> {
        self.capture(plan.before())
    }
}
impl OrderRunSource for PinnedClock {
    fn manifest(&self) -> &OrderRunManifest {
        self.inner.manifest()
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        self.inner.endpoint()
    }
    fn fortress(&self) -> &FortressIdentity {
        self.inner.fortress()
    }
    fn fence(&mut self) {
        self.inner.fence();
    }
    fn observe(
        &mut self,
        id: u32,
        c: &OperationContext,
        timeout: Duration,
    ) -> Result<OrderCapture> {
        self.check(c, false)?;
        let result = self.inner.observe(id, c, timeout)?;
        self.check(c, false)?;
        self.capture(&result)?;
        Ok(result)
    }
    fn prepare(
        &mut self,
        plan: &OrderRunPlan,
        c: &OperationContext,
        timeout: Duration,
    ) -> Result<OrderRunRecord> {
        self.check(c, true)?;
        self.plan(plan)?;
        let result = self.inner.prepare(plan, c, timeout)?;
        self.check(c, false)?;
        Ok(result)
    }
    fn commit(
        &mut self,
        plan: &OrderRunPlan,
        c: &OperationContext,
        timeout: Duration,
    ) -> Result<OrderRunRecord> {
        self.check(c, true)?;
        self.plan(plan)?;
        let result = self.inner.commit(plan, c, timeout)?;
        self.check(c, false)?;
        Ok(result)
    }
    fn query(
        &mut self,
        plan: &OrderRunPlan,
        c: &OperationContext,
        timeout: Duration,
    ) -> Result<Option<OrderRunRecord>> {
        self.check(c, false)?;
        self.plan(plan)?;
        let result = self.inner.query(plan, c, timeout)?;
        self.check(c, false)?;
        Ok(result)
    }
    fn cancel(
        &mut self,
        plan: &OrderRunPlan,
        c: &OperationContext,
        timeout: Duration,
    ) -> Result<OrderRunRecord> {
        self.check(c, true)?;
        self.plan(plan)?;
        let result = self.inner.cancel(plan, c, timeout)?;
        self.check(c, false)?;
        Ok(result)
    }
}

pub(super) fn connect(config: &Arc<Config>, c: &OperationContext) -> Result<PinnedClock> {
    config.current()?;
    if config.clock.is_none() {
        return Err(denied("operator clock source is not configured"));
    }
    let fortress = FortressIdentity::new(&config.sources.world_folder, config.sources.site_id)?;
    let inner = OrderRunRpc::connect(
        config.endpoint,
        token(CLOCK_TOKEN)?,
        nonce(c, b"order-clock"),
        fortress,
        c,
    )?;
    let mut result = PinnedClock {
        inner,
        config: Arc::clone(config),
    };
    result.check(c, false)?;
    Ok(result)
}

pub(super) fn open(
    config: &Arc<Config>,
    mode: Mode,
    c: &OperationContext,
) -> Result<Option<OrderRunJournal<PrivateOrderRunFile>>> {
    config.current()?;
    let fortress = FortressIdentity::new(&config.sources.world_folder, config.sources.site_id)?;
    authorize(c, &fortress, false)?;
    let mut work = Work::new(c)?;
    let Some(clock) = config.clock.as_ref() else {
        // Omission is compatible with old, clock-free custody only. An existing
        // clock history may retain an unresolved unpause and cannot disappear
        // from the handoff merely because the operator omitted its config.
        work.reserve(4096)?;
        match std::fs::symlink_metadata(config.path("clock.journal")) {
            Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {}
            _ => {
                return Err(denied(
                    "existing or unverifiable clock custody requires its exact configured recovery identity",
                ));
            }
        }
        config.current()?;
        work.current()?;
        return Ok(None);
    };
    // A read recovery owner remains read-only with respect to clock control even
    // if the operator subsequently enables the gate. Reopen to acquire control.
    let mode = match mode {
        Mode::Offline => OrderRunMode::Offline,
        Mode::Reconcile => OrderRunMode::Recover,
        Mode::Control if Config::clock_enabled() => OrderRunMode::Control,
        Mode::Control => OrderRunMode::Recover,
    };
    if config.initialize && mode != OrderRunMode::Control {
        return Err(denied(
            "initial clock custody requires explicit clock control",
        ));
    }
    let view_context = work.reserve(3 * MIB)?;
    let binding = if config.initialize {
        let source = connect(config, &work.reserve(3 * MIB)?)?;
        Some(OrderRunBinding::from_source(&source)?)
    } else {
        // Existing source identity is checked against operator pins below.
        // Opening offline or recovering custody requires no credential or I/O
        // to a native source, including when the clock gate has been revoked.
        None
    };
    config.current()?;
    if config.initialize && !Config::clock_enabled() {
        return Err(denied(
            "operator clock authority was revoked during initialization",
        ));
    }
    let mut journal = open_private_order_journal_exact(
        &config.path("clock.journal"),
        &fortress,
        &work.rest()?,
        mode,
        binding,
        config.initialize,
    )?;
    let view = journal.view(&work.narrow(&view_context)?)?;
    identity(
        config,
        view.binding.manifest(),
        Some(view.binding.endpoint()),
        view.binding.fortress(),
    )?;
    if clock.journal_id.is_some_and(|expected| expected != view.id) {
        return Err(denied(
            "clock journal differs from the externally retained identity",
        ));
    }
    config.current()?;
    work.current()?;
    Ok(Some(journal))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dfmcp_core::{
        Capability, CapabilityGrant, CapabilityScope, GameTick, ObservationCursor, RequestId,
        RiskTier, SessionId, StateAnchor, WorkBudget,
    };
    use serde_json::{Value, json};

    fn config() -> Value {
        json!({
            "schema":"dfmcp.semantic-work-order-development/1",
            "directory":"/tmp/semantic-order-clock-config",
            "endpoint":"127.0.0.1:5000", "initialize":true,
            "sources":{
                "kind":"operator-development-source-acceptance/1",
                "world_folder":"region1", "site_id":7,
                "df_version":"df", "dfhack_version":"dfhack",
                "citizen_bridge_version":"citizens", "citizen_generation":10,
                "work_order_generation":20, "progress_generation":30,
                "operations_generation":40
            }
        })
    }
    #[test]
    fn optional_clock_configuration_is_closed_and_preserves_existing_configs() -> Result<()> {
        let mut raw = config();
        assert!(Config::parse(&raw.to_string())?.clock.is_none());
        raw["clock"] = json!({"generation":50});
        assert!(Config::parse(&raw.to_string())?.clock.is_some());
        for generation in [0, u64::MAX] {
            raw["clock"]["generation"] = json!(generation);
            assert!(Config::parse(&raw.to_string()).is_err());
        }
        raw["clock"] = json!({"generation":50, "order_id":9});
        assert!(Config::parse(&raw.to_string()).is_err());
        raw["clock"] = json!({"generation":50, "journal_id":"ab".repeat(32)});
        assert!(Config::parse(&raw.to_string()).is_err());
        raw["clock"] = json!({"generation":50, "journal_id":null});
        assert!(Config::parse(&raw.to_string()).is_err());
        Ok(())
    }
    #[test]
    fn clock_recovery_requires_the_external_exact_nonzero_journal_identity() -> Result<()> {
        let mut raw = config();
        raw["initialize"] = json!(false);
        raw["clock"] = json!({"generation":50});
        assert!(Config::parse(&raw.to_string()).is_err());
        for id in ["00".repeat(32), "AB".repeat(32), "ab".repeat(31)] {
            raw["clock"]["journal_id"] = json!(id);
            assert!(Config::parse(&raw.to_string()).is_err());
        }
        raw["clock"]["journal_id"] = json!("ab".repeat(32));
        let parsed = Config::parse(&raw.to_string())?;
        assert_eq!(
            parsed.clock.as_ref().and_then(|c| c.journal_id),
            Some(digest(&"ab".repeat(32))?)
        );
        raw["recovery"] = json!({
            "plan_digest":"cd".repeat(32), "object_digest":"ef".repeat(32),
            "anchor":{
                "fortress_id":parsed.sources.fortress().get().to_string(),
                "epoch":0,"sequence":1,"tick":100,"state_hash":"12".repeat(32)
            },
            "history_present":false
        });
        assert!(Config::parse(&raw.to_string()).is_err());
        raw["recovery"]["clock_journal_id"] = json!("cd".repeat(32));
        assert!(Config::parse(&raw.to_string()).is_err());
        raw["recovery"]["clock_journal_id"] = json!("ab".repeat(32));
        assert!(Config::parse(&raw.to_string()).is_ok());
        raw["clock"] = Value::Null;
        assert!(Config::parse(&raw.to_string()).is_err());
        raw["recovery"]["clock_journal_id"] = Value::Null;
        assert!(Config::parse(&raw.to_string()).is_ok());
        Ok(())
    }
    #[test]
    fn clock_pins_bind_the_independent_incarnation_and_exact_source() -> Result<()> {
        let mut raw = config();
        raw["clock"] = json!({"generation":50});
        let config = Config::parse(&raw.to_string())?;
        let fortress = FortressIdentity::new("region1", 7)?;
        let endpoint = Some(SocketAddr::from(([127, 0, 0, 1], 5000)));
        let manifest = OrderRunManifest {
            generation: 50,
            df_version: "df".into(),
            dfhack_version: "dfhack".into(),
        };
        identity(&config, &manifest, endpoint, &fortress)?;
        for generation in [10, 20, 30, 40, 49, 51] {
            let changed = OrderRunManifest {
                generation,
                ..manifest.clone()
            };
            assert!(identity(&config, &changed, endpoint, &fortress).is_err());
        }
        for changed in [
            OrderRunManifest {
                df_version: "other".into(),
                ..manifest.clone()
            },
            OrderRunManifest {
                dfhack_version: "other".into(),
                ..manifest.clone()
            },
        ] {
            assert!(identity(&config, &changed, endpoint, &fortress).is_err());
        }
        assert!(identity(&config, &manifest, None, &fortress).is_err());
        assert!(
            identity(
                &config,
                &manifest,
                Some(SocketAddr::from(([127, 0, 0, 1], 5001))),
                &fortress,
            )
            .is_err()
        );
        for fortress in [
            FortressIdentity::new("region2", 7)?,
            FortressIdentity::new("region1", 8)?,
        ] {
            assert!(identity(&config, &manifest, endpoint, &fortress).is_err());
        }
        Ok(())
    }

    fn context(config: &Config, control: bool) -> OperationContext {
        let fortress = config.sources.fortress();
        let capabilities = if control {
            vec![Capability::Query, Capability::ControlClock]
        } else {
            vec![Capability::Query]
        };
        OperationContext {
            session_id: SessionId::new(1201),
            request_id: RequestId::new(1202),
            anchor: StateAnchor {
                fortress_id: fortress,
                cursor: ObservationCursor::ORIGIN,
                tick: GameTick(0),
                state_hash: Digest32::ZERO,
            },
            budget: WorkBudget {
                max_wall_millis: 5000,
                max_bytes: 16 * MIB,
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
                        fortress_id: Some(fortress),
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
    struct BindingOnly {
        manifest: OrderRunManifest,
        fortress: FortressIdentity,
        endpoint: SocketAddr,
    }
    impl OrderRunSource for BindingOnly {
        fn manifest(&self) -> &OrderRunManifest {
            &self.manifest
        }
        fn endpoint(&self) -> Option<SocketAddr> {
            Some(self.endpoint)
        }
        fn fortress(&self) -> &FortressIdentity {
            &self.fortress
        }
        fn fence(&mut self) {}
        fn observe(&mut self, _: u32, _: &OperationContext, _: Duration) -> Result<OrderCapture> {
            Err(denied("binding fixture has no native transport"))
        }
        fn prepare(
            &mut self,
            _: &OrderRunPlan,
            _: &OperationContext,
            _: Duration,
        ) -> Result<OrderRunRecord> {
            Err(denied("binding fixture has no native transport"))
        }
        fn commit(
            &mut self,
            _: &OrderRunPlan,
            _: &OperationContext,
            _: Duration,
        ) -> Result<OrderRunRecord> {
            Err(denied("binding fixture has no native transport"))
        }
        fn query(
            &mut self,
            _: &OrderRunPlan,
            _: &OperationContext,
            _: Duration,
        ) -> Result<Option<OrderRunRecord>> {
            Err(denied("binding fixture has no native transport"))
        }
        fn cancel(
            &mut self,
            _: &OrderRunPlan,
            _: &OperationContext,
            _: Duration,
        ) -> Result<OrderRunRecord> {
            Err(denied("binding fixture has no native transport"))
        }
    }

    #[test]
    #[cfg(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))]
    fn offline_clock_custody_requires_exact_handoff_without_native_credentials() -> Result<()> {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        const CHILD: &str = "SEMANTIC_ORDER_CLOCK_TEST_CASE";
        const CHECKED: &str = "semantic-clock-recovery-child-checked";
        if let Ok(case) = std::env::var(CHILD) {
            assert!(std::env::var(super::super::CLOCK_TOKEN).is_err());
            let config = Arc::new(Config::from_environment()?);
            let c = context(&config, false);
            let mode = match case.as_str() {
                "offline" | "wrong_id" | "omitted" | "missing" => Mode::Offline,
                "reconcile" => Mode::Reconcile,
                "revoked_control" | "ungranted_control" => Mode::Control,
                _ => return Err(invalid("unexpected clock recovery fixture")),
            };
            let result = open(&config, mode, &c);
            if matches!(
                case.as_str(),
                "wrong_id" | "omitted" | "missing" | "ungranted_control"
            ) {
                assert!(result.is_err());
            } else {
                let mut journal = result?.ok_or_else(|| invalid("clock fixture not opened"))?;
                assert_eq!(
                    journal.mode(),
                    if mode == Mode::Offline {
                        OrderRunMode::Offline
                    } else {
                        OrderRunMode::Recover
                    }
                );
                let view = journal.view(&c)?;
                assert_eq!(
                    Some(view.id),
                    config.clock.as_ref().and_then(|clock| clock.journal_id)
                );
                assert!(view.entries.is_empty());
            }
            println!("{CHECKED}");
            return Ok(());
        }

        let storage_error = |_| denied("private clock fixture storage failed");
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| invalid("fixture timestamp"))?
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "dfmcp-semantic-clock-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir(&directory).map_err(storage_error)?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
            .map_err(storage_error)?;
        let directory = directory.canonicalize().map_err(storage_error)?;
        let mut raw = config();
        raw["directory"] = json!(directory);
        raw["clock"] = json!({"generation":50});
        let parsed = Config::parse(&raw.to_string())?;
        let c = context(&parsed, true);
        let source = BindingOnly {
            manifest: OrderRunManifest {
                generation: 50,
                df_version: "df".into(),
                dfhack_version: "dfhack".into(),
            },
            fortress: FortressIdentity::new("region1", 7)?,
            endpoint: parsed.endpoint,
        };
        let path = directory.join("clock.journal");
        let mut journal = open_private_order_journal_exact(
            &path,
            &source.fortress,
            &c,
            OrderRunMode::Control,
            Some(OrderRunBinding::from_source(&source)?),
            true,
        )?;
        let id = journal.view(&c)?.id;
        drop(journal);
        let original = fs::read(&path).map_err(storage_error)?;
        raw["initialize"] = json!(false);
        raw["clock"]["journal_id"] = json!(id.to_hex());
        let full_name = format!(
            "{}::offline_clock_custody_requires_exact_handoff_without_native_credentials",
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
                input["clock"]["journal_id"] =
                    json!(Digest32::of_bytes(b"another-clock-journal").to_hex());
            } else if case == "omitted" {
                input
                    .as_object_mut()
                    .ok_or_else(|| invalid("fixture configuration"))?
                    .remove("clock");
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
                command.env(super::super::CLOCK_AUTHORITY, "1");
            }
            let output = command.output().map_err(storage_error)?;
            assert!(
                output.status.success(),
                "clock recovery case {case} failed: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(String::from_utf8_lossy(&output.stdout).contains(CHECKED));
            assert_eq!(fs::read(&path).map_err(storage_error)?, original);
        }
        assert!(!directory.join("missing/clock.journal").exists());
        Ok(())
    }
}
