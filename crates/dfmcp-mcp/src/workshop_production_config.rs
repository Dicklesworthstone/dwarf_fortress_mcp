//! Closed operator configuration for a single staged Workshop/production owner.
//! Child configurations and credentials remain operator state, never tool input.
use crate::live_production_runtime::Config as ProductionConfig;
use crate::semantic_workforce_runtime::{denied, digest, invalid};
use crate::workshop_construction_server::runtime::Config as WorkshopConfig;
use crate::{semantic_work_order_runtime as orders, semantic_workforce_runtime as workforce};
use dfmcp_adapter::bounded_run::MAX_NATIVE_TICK;
use dfmcp_core::{Digest32, Result};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

pub(crate) const OPT_IN: &str = "DFMCP_ALLOW_UNADMITTED_WORKSHOP_PRODUCTION";
pub(crate) const CONFIG: &str = "DFMCP_WORKSHOP_PRODUCTION_CONFIG";
const WORKSHOP_CONFIG: &str = "DFMCP_WORKSHOP_MCP_CONFIG";
const PRODUCTION_CONFIG: &str = "DFMCP_COMBINED_PRODUCTION_CONFIG";
const WORKFORCE_CONFIG: &str = "DFMCP_SEMANTIC_WORKFORCE_CONFIG";
const ORDER_CONFIG: &str = "DFMCP_SEMANTIC_WORK_ORDER_CONFIG";
const WORKSHOP_OPT_IN: &str = "DFMCP_ALLOW_UNADMITTED_WORKSHOP_V1_20";
const TREE_OPT_IN: &str = "DFMCP_ALLOW_UNADMITTED_WORKSHOP_TREE_MATERIAL";
const WORKSHOP_CLOCK_OPT_IN: &str = "DFMCP_ALLOW_UNADMITTED_WORKSHOP_CLOCK";
const CONFIGURATIONS: [&str; 5] = [
    CONFIG,
    WORKSHOP_CONFIG,
    PRODUCTION_CONFIG,
    WORKFORCE_CONFIG,
    ORDER_CONFIG,
];
const REQUIRED_OPT_INS: [&str; 5] = [
    OPT_IN,
    WORKSHOP_OPT_IN,
    "DFMCP_ALLOW_UNADMITTED_COMBINED_PRODUCTION",
    "DFMCP_ALLOW_UNADMITTED_SEMANTIC_WORKFORCE",
    "DFMCP_ALLOW_UNADMITTED_SEMANTIC_WORK_ORDERS",
];
const PERMISSIONS: [&str; 6] = [
    "DFMCP_WORKSHOP_ALLOW_BUILD",
    "DFMCP_RUN_ALLOW_CLOCK",
    "DFMCP_SEMANTIC_WORKFORCE_ALLOW_LABOR",
    "DFMCP_SEMANTIC_WORK_ORDER_ALLOW_PRODUCTION",
    "DFMCP_SEMANTIC_WORK_ORDER_ALLOW_CLOCK",
    "DFMCP_SEMANTIC_WORK_ORDER_ALLOW_INVENTORY_CLOCK",
];
const MAX_CONFIG_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Stage {
    Workshop,
    Transition,
    Production,
}

impl Stage {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Workshop => "workshop",
            Self::Transition => "transition",
            Self::Production => "production",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Recovery {
    pub(crate) stage: Stage,
    pub(crate) handoff_digest: Option<Digest32>,
    pub(crate) authority_tick_floor: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryInput {
    stage: Stage,
    #[serde(default, deserialize_with = "optional_digest")]
    handoff_digest: Option<Digest32>,
    authority_tick_floor: u64,
}

fn optional_digest<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Digest32>, D::Error> {
    let text = String::deserialize(deserializer)?;
    digest(&text).map(Some).map_err(serde::de::Error::custom)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    schema: String,
    #[serde(default, deserialize_with = "recovery_input")]
    recovery: Option<RecoveryInput>,
}

fn recovery_input<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<RecoveryInput>, D::Error> {
    RecoveryInput::deserialize(deserializer).map(Some)
}

fn parse(raw: &str) -> Result<Option<Recovery>> {
    if raw.is_empty() || raw.len() > MAX_CONFIG_BYTES {
        return Err(invalid(
            "staged Workshop/production configuration exceeds 16 KiB",
        ));
    }
    let input: Input = serde_json::from_str(raw)
        .map_err(|_| invalid("invalid closed staged Workshop/production configuration"))?;
    if input.schema != "dfmcp.workshop-production-development/1" {
        return Err(invalid("unsupported staged Workshop/production schema"));
    }
    input
        .recovery
        .map(|recovery| {
            if recovery.authority_tick_floor > MAX_NATIVE_TICK
                || (recovery.stage == Stage::Workshop) == recovery.handoff_digest.is_some()
            {
                return Err(invalid(
                    "staged recovery requires a legal authority floor and the exact handoff after Workshop",
                ));
            }
            Ok(Recovery {
                stage: recovery.stage,
                handoff_digest: recovery.handoff_digest,
                authority_tick_floor: recovery.authority_tick_floor,
            })
        })
        .transpose()
}

/// A bounded snapshot of only the fixed DFMCP namespace. Its digest also pins
/// credentials and optional permission presence, without exposing their values.
struct Environment {
    values: BTreeMap<String, String>,
}

impl Environment {
    fn read() -> Result<Self> {
        let mut values = BTreeMap::new();
        for (key, value) in std::env::vars_os() {
            if !key.to_string_lossy().starts_with("DFMCP_") {
                continue;
            }
            let key = key
                .into_string()
                .map_err(|_| denied("staged profile requires exact UTF-8 configuration names"))?;
            let value = value
                .into_string()
                .map_err(|_| denied("staged profile requires UTF-8 operator configuration"))?;
            if key.len() > 128 || value.len() > MAX_CONFIG_BYTES || values.len() >= 64 {
                return Err(invalid(
                    "staged profile operator environment exceeds its fixed bound",
                ));
            }
            values.insert(key, value);
        }
        Ok(Self { values })
    }

    fn required(&self, key: &str) -> Result<&str> {
        self.values
            .get(key)
            .map(String::as_str)
            .filter(|value| !value.is_empty() && value.len() <= MAX_CONFIG_BYTES)
            .ok_or_else(|| denied("required staged operator configuration is absent or oversized"))
    }

    fn digest(&self) -> Digest32 {
        let mut bytes = b"dfmcp.workshop-production-operator-environment/1".to_vec();
        for (key, value) in &self.values {
            bytes.extend_from_slice(&(key.len() as u64).to_be_bytes());
            bytes.extend_from_slice(key.as_bytes());
            bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
            bytes.extend_from_slice(value.as_bytes());
        }
        Digest32::of_bytes(&bytes)
    }

    fn validate(&self) -> Result<Option<Recovery>> {
        for key in REQUIRED_OPT_INS {
            if self.values.get(key).map(String::as_str) != Some("1") {
                return Err(denied(
                    "staged parent and every native family require their exact opt-in",
                ));
            }
        }
        for key in PERMISSIONS {
            if self.values.get(key).is_some_and(|value| value != "1") {
                return Err(denied(
                    "staged native permission must be absent or exactly 1",
                ));
            }
        }
        for key in CONFIGURATIONS {
            self.required(key)?;
        }
        let recovery = parse(self.required(CONFIG)?)?;
        let workshop = WorkshopConfig::parse_for_parent(self.required(WORKSHOP_CONFIG)?)?;
        let production = ProductionConfig::parent_metadata(self.required(PRODUCTION_CONFIG)?)?;
        let workforce = workforce::native::Config::parse(self.required(WORKFORCE_CONFIG)?)?;
        let orders = orders::native::Config::parse(self.required(ORDER_CONFIG)?)?;
        let tree = workshop.tree_custody_directory().is_some();
        let workshop_clock = workshop.clock_configured();
        for (key, configured) in [(TREE_OPT_IN, tree), (WORKSHOP_CLOCK_OPT_IN, workshop_clock)] {
            if self.values.get(key).map(String::as_str) != configured.then_some("1") {
                return Err(denied(
                    "staged Workshop donor and clock opt-ins must exactly match their configuration",
                ));
            }
        }
        for key in self.values.keys() {
            if !allowed_key(
                key,
                tree,
                workshop_clock,
                production.clock,
                production.inventory_clock,
                production.manager_directory.is_some(),
            ) {
                return Err(denied(
                    "staged Workshop/production rejects unrelated DFMCP state and unconfigured credentials",
                ));
            }
        }
        let initialize = recovery.is_none();
        let stage = recovery.map_or(Stage::Workshop, |r| r.stage);
        if workshop.initializing() != initialize
            || production.initialize != initialize
            || workforce.initializing() != initialize
            || orders.initializing() != initialize
            || workforce.endpoint() != orders.endpoint()
            || (stage != Stage::Production && production.has_original)
            || (stage == Stage::Workshop && production.workshop_checkpoint)
            || recovery.map_or(0, |r| r.authority_tick_floor) < production.authority_tick_floor
        {
            return Err(denied(
                "staged children must retain one initialization mode, stage and complete authority floor",
            ));
        }
        let mut directories = vec![
            workshop.custody_directory(),
            production.directory.as_path(),
            workforce.custody_directory(),
            orders.custody_directory(),
        ];
        if let Some(directory) = workshop.tree_custody_directory() {
            directories.push(directory);
        }
        if let Some(directory) = production.manager_directory.as_deref() {
            directories.push(directory);
        }
        if !disjoint_directories(&directories) {
            return Err(denied(
                "staged native owners and optional donors require disjoint nonnested custody",
            ));
        }
        #[derive(Deserialize)]
        struct WorkforceSources {
            sources: workforce::policy::SourceAcceptance,
        }
        #[derive(Deserialize)]
        struct OrderSources {
            sources: orders::policy::SourceAcceptance,
        }
        // The native Config parsers above already checked these complete closed
        // objects, including duplicate fields and independent family generations.
        let wa: WorkforceSources = serde_json::from_str(self.required(WORKFORCE_CONFIG)?)
            .map_err(|_| invalid("invalid staged workforce sources"))?;
        let oa: OrderSources = serde_json::from_str(self.required(ORDER_CONFIG)?)
            .map_err(|_| invalid("invalid staged order sources"))?;
        let binding = workshop.source_binding()?;
        if binding.fortress().folder() != wa.sources.world_folder
            || binding.fortress().folder() != oa.sources.world_folder
            || binding.fortress().site() != wa.sources.site_id
            || binding.fortress().site() != oa.sources.site_id
            || binding.df_version() != wa.sources.df_version
            || binding.df_version() != oa.sources.df_version
            || binding.dfhack_version() != wa.sources.dfhack_version
            || binding.dfhack_version() != oa.sources.dfhack_version
            || wa.sources.citizen_bridge_version != oa.sources.citizen_bridge_version
            || wa.sources.citizen_generation != oa.sources.citizen_generation
        {
            return Err(denied(
                "staged Workshop and production require the same pinned fortress and software",
            ));
        }
        Ok(recovery)
    }
}

fn allowed_key(
    key: &str,
    tree: bool,
    workshop_clock: bool,
    order_clock: bool,
    inventory_clock: bool,
    manager: bool,
) -> bool {
    match key {
        OPT_IN
        | CONFIG
        | WORKSHOP_OPT_IN
        | WORKSHOP_CONFIG
        | "DFMCP_WORKSHOP_TOKEN"
        | "DFMCP_WORKSHOP_ALLOW_BUILD" => true,
        TREE_OPT_IN | "DFMCP_TREE_HARVEST_TOKEN" => tree,
        WORKSHOP_CLOCK_OPT_IN | "DFMCP_RUN_TOKEN" | "DFMCP_RUN_ALLOW_CLOCK" => workshop_clock,
        _ => ProductionConfig::parent_allowed_key(key, order_clock, inventory_clock, manager),
    }
}

fn disjoint_directories(directories: &[&Path]) -> bool {
    (0..directories.len()).all(|i| {
        ((i + 1)..directories.len()).all(|j| {
            !directories[i].starts_with(directories[j])
                && !directories[j].starts_with(directories[i])
        })
    })
}

pub(crate) struct Config {
    pub(crate) recovery: Option<Recovery>,
    workshop: WorkshopConfig,
    guard: Arc<dyn Fn() -> Result<()> + Send + Sync>,
    runtime_check: fn() -> Result<()>,
}

impl Config {
    pub(crate) fn from_environment(runtime_check: fn() -> Result<()>) -> Result<Self> {
        runtime_check()?;
        if crate::current_admission_provenance().is_some() {
            return Err(denied(
                "staged development profile refuses production provenance",
            ));
        }
        let environment = Environment::read()?;
        let recovery = environment.validate()?;
        let pinned = environment.digest();
        let guard: Arc<dyn Fn() -> Result<()> + Send + Sync> = Arc::new(move || {
            runtime_check()?;
            if crate::current_admission_provenance().is_some() {
                return Err(denied(
                    "staged development profile refuses production provenance",
                ));
            }
            let current = Environment::read()?;
            current.validate()?;
            if current.digest() != pinned {
                return Err(denied(
                    "staged operator configuration or authority changed; retain custody and close",
                ));
            }
            Ok(())
        });
        let workshop = WorkshopConfig::from_parent_environment(Arc::clone(&guard))?;
        let config = Self {
            recovery,
            workshop,
            guard,
            runtime_check,
        };
        config.current()?;
        Ok(config)
    }

    pub(crate) fn workshop_config(&self) -> WorkshopConfig {
        self.workshop.clone()
    }

    pub(crate) fn production_config(
        &self,
        verified_workshop: Option<&Value>,
    ) -> Result<ProductionConfig> {
        self.current()?;
        if self.recovery.is_some_and(|r| r.stage != Stage::Workshop) && verified_workshop.is_none()
        {
            return Err(denied(
                "recovered post-Workshop stage requires its independently verified exact handoff",
            ));
        }
        let production = ProductionConfig::from_parent_environment(
            Arc::clone(&self.guard),
            self.runtime_check,
            verified_workshop,
        )?;
        if verified_workshop.is_some() {
            production.require_parent_workshop(
                &self.workshop.source_binding()?,
                self.workshop.custody_directory(),
            )?;
        }
        self.current()?;
        Ok(production)
    }

    pub(crate) fn current(&self) -> Result<()> {
        (self.guard)()
    }
}

#[cfg(test)]
#[path = "workshop_production_config_tests.rs"]
mod tests;
