//! Closed operator policy for optional Manager and Tree stages before Workshop and stock.
use super::{error, journal};
use crate::live_production_runtime::{Config as ProductionConfig, ProjectStockGoal};
use crate::manager_appointment_server::{ManagerProjectHandoff, ProjectManagerConfig};
use crate::tree_harvest_server::{ProjectTreeConfig, TreeProjectHandoff};
use crate::workshop_construction_server::{ProjectWorkshopTemplate, WorkshopProjectHandoff};
use dfmcp_core::{Digest32, ErrorCode, Result};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(super) const OPT_IN: &str = "DFMCP_ALLOW_UNADMITTED_NATIVE_RESOURCE_PROJECT";
pub(super) const CONFIG: &str = "DFMCP_NATIVE_RESOURCE_PROJECT_CONFIG";
const TREE_CONFIG: &str = "DFMCP_TREE_HARVEST_MCP_CONFIG";
const MANAGER_CONFIG: &str = "DFMCP_MANAGER_V1_23_MCP_CONFIG";
const PARENT_CONFIG_BYTES: usize = 128 * 1024;
const CONFIGURATIONS: [&str; 5] = [
    CONFIG,
    "DFMCP_WORKSHOP_MCP_CONFIG",
    "DFMCP_COMBINED_PRODUCTION_CONFIG",
    "DFMCP_SEMANTIC_WORKFORCE_CONFIG",
    "DFMCP_SEMANTIC_WORK_ORDER_CONFIG",
];
const ALLOWED: &[&str] = &[
    OPT_IN,
    CONFIG,
    "DFMCP_ALLOW_UNADMITTED_WORKSHOP_V1_20",
    "DFMCP_WORKSHOP_MCP_CONFIG",
    "DFMCP_WORKSHOP_TOKEN",
    "DFMCP_WORKSHOP_ALLOW_BUILD",
    "DFMCP_ALLOW_UNADMITTED_COMBINED_PRODUCTION",
    "DFMCP_COMBINED_PRODUCTION_CONFIG",
    "DFMCP_COMBINED_PRODUCTION_WORKSHOP_TOKEN",
    "DFMCP_ALLOW_UNADMITTED_SEMANTIC_WORKFORCE",
    "DFMCP_SEMANTIC_WORKFORCE_CONFIG",
    "DFMCP_SEMANTIC_WORKFORCE_ALLOW_LABOR",
    "DFMCP_SEMANTIC_WORKFORCE_CITIZEN_TOKEN",
    "DFMCP_SEMANTIC_WORKFORCE_TOKEN",
    "DFMCP_ALLOW_UNADMITTED_SEMANTIC_WORK_ORDERS",
    "DFMCP_SEMANTIC_WORK_ORDER_CONFIG",
    "DFMCP_SEMANTIC_WORK_ORDER_ALLOW_PRODUCTION",
    "DFMCP_SEMANTIC_WORK_ORDER_CITIZEN_TOKEN",
    "DFMCP_SEMANTIC_WORK_ORDER_ORDER_TOKEN",
    "DFMCP_SEMANTIC_WORK_ORDER_PROGRESS_TOKEN",
    "DFMCP_SEMANTIC_WORK_ORDER_OPERATIONS_TOKEN",
];

fn denied(message: &str) -> dfmcp_core::DfmcpError {
    error(ErrorCode::CapabilityDenied, message)
}
fn invalid(message: &str) -> dfmcp_core::DfmcpError {
    error(ErrorCode::InvalidRequest, message)
}

pub(super) fn runtime_io() -> Result<()> {
    let cx = asupersync::Cx::current()
        .ok_or_else(|| denied("resource project requires an owned runtime context"))?;
    cx.checkpoint().map_err(|_| {
        error(
            ErrorCode::CancellationRequested,
            "resource project cancelled",
        )
    })?;
    if !cx.capabilities().io {
        return Err(denied("inherited context denies resource project I/O"));
    }
    Ok(())
}

fn read(name: &str) -> Result<String> {
    let raw = std::env::var(name)
        .map_err(|_| denied("required resource project operator configuration is missing"))?;
    let maximum = if name == CONFIG {
        PARENT_CONFIG_BYTES
    } else {
        16 * 1024
    };
    if raw.is_empty() || raw.len() > maximum {
        return Err(invalid(
            "resource project operator configuration exceeds its bound",
        ));
    }
    Ok(raw)
}

#[derive(Clone, Copy)]
struct Features {
    appoint_manager: bool,
    harvest_tree: bool,
    tree_material: bool,
    tree_clock: bool,
    workshop_clock: bool,
    production_clock: bool,
    inventory_clock: bool,
    manager: bool,
}
impl Features {
    fn from_raw(
        workshop: &str,
        production: &str,
        parent: Option<&str>,
        tree: Option<&str>,
    ) -> Result<Self> {
        let w: Value = serde_json::from_str(workshop)
            .map_err(|_| invalid("invalid Workshop configuration"))?;
        let p: Value = serde_json::from_str(production)
            .map_err(|_| invalid("invalid production configuration"))?;
        let parent: Option<Value> = parent
            .map(serde_json::from_str)
            .transpose()
            .map_err(|_| invalid("invalid resource project feature configuration"))?;
        let harvest_tree = match parent.as_ref().and_then(|value| value.get("harvest_tree")) {
            None => false,
            Some(Value::Bool(value)) => *value,
            Some(_) => return Err(invalid("harvest_tree must be a boolean")),
        };
        let appoint_manager = match parent
            .as_ref()
            .and_then(|value| value.get("appoint_manager"))
        {
            None => false,
            Some(Value::Bool(value)) => *value,
            Some(_) => return Err(invalid("appoint_manager must be a boolean")),
        };
        let tree_clock = if harvest_tree {
            let tree: Value = serde_json::from_str(tree.ok_or_else(|| {
                denied("a harvesting project requires its exact Tree configuration")
            })?)
            .map_err(|_| invalid("invalid project Tree configuration"))?;
            !tree["clock"].is_null()
        } else {
            false
        };
        Ok(Self {
            appoint_manager,
            harvest_tree,
            tree_material: harvest_tree || !w["tree_material_source"].is_null(),
            tree_clock,
            workshop_clock: !w["clock"].is_null(),
            production_clock: !p["clock"].is_null(),
            inventory_clock: !p["inventory_clock"].is_null(),
            manager: appoint_manager || !p["manager_prerequisite"].is_null(),
        })
    }
    fn allows(self, key: &str) -> bool {
        match key {
            MANAGER_CONFIG
            | "DFMCP_ALLOW_UNADMITTED_MANAGER_V1_23"
            | "DFMCP_MANAGER_V1_23_TOKEN"
            | "DFMCP_MANAGER_V1_23_ALLOW_APPOINT" => self.appoint_manager,
            TREE_CONFIG
            | "DFMCP_ALLOW_UNADMITTED_TREE_HARVEST_V1_21"
            | "DFMCP_TREE_HARVEST_ALLOW_DESIGNATE" => self.harvest_tree,
            "DFMCP_ALLOW_UNADMITTED_TREE_CLOCK" => self.tree_clock,
            "DFMCP_ALLOW_UNADMITTED_WORKSHOP_TREE_MATERIAL" | "DFMCP_TREE_HARVEST_TOKEN" => {
                self.tree_material
            }
            "DFMCP_ALLOW_UNADMITTED_WORKSHOP_CLOCK" => self.workshop_clock,
            "DFMCP_RUN_TOKEN" | "DFMCP_RUN_ALLOW_CLOCK" => self.tree_clock || self.workshop_clock,
            "DFMCP_SEMANTIC_WORK_ORDER_ALLOW_CLOCK" | "DFMCP_SEMANTIC_WORK_ORDER_CLOCK_TOKEN" => {
                self.production_clock
            }
            "DFMCP_SEMANTIC_WORK_ORDER_ALLOW_INVENTORY_CLOCK"
            | "DFMCP_SEMANTIC_WORK_ORDER_INVENTORY_CLOCK_TOKEN" => self.inventory_clock,
            "DFMCP_COMBINED_PRODUCTION_MANAGER_TOKEN" => self.manager,
            _ => !key.starts_with("DFMCP_") || ALLOWED.contains(&key),
        }
    }
}

pub(super) fn environment() -> Result<()> {
    if std::env::var(OPT_IN).ok().as_deref() != Some("1")
        || std::env::var_os("DFMCP_ADMITTED_BRIDGE_PROTOCOL").is_some()
        || crate::current_admission_provenance().is_some()
    {
        return Err(denied(
            "resource project requires its development opt-in and refuses production admission",
        ));
    }
    // Discovery can precede complete operator configuration. Optional feature
    // selectors still determine the closed environment union before startup;
    // opening later validates every required configuration and identity.
    let parent = std::env::var(CONFIG).ok();
    let tree = std::env::var(TREE_CONFIG).ok();
    if parent
        .as_ref()
        .is_some_and(|raw| raw.len() > PARENT_CONFIG_BYTES)
        || tree.as_ref().is_some_and(|raw| raw.len() > 16 * 1024)
    {
        return Err(invalid(
            "resource project feature configuration exceeds its bound",
        ));
    }
    let features = Features::from_raw(
        &read(CONFIGURATIONS[1])?,
        &read(CONFIGURATIONS[2])?,
        parent.as_deref(),
        tree.as_deref(),
    )?;
    for (key, _) in std::env::vars_os() {
        if !features.allows(&key.to_string_lossy()) {
            return Err(denied(
                "resource project refuses other DFMCP profiles and unconfigured credentials",
            ));
        }
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    schema: String,
    directory: PathBuf,
    initialize: bool,
    acknowledged: Option<journal::Pin>,
    goal: ProjectStockGoal,
    handoff: Option<Value>,
    #[serde(default)]
    harvest_tree: bool,
    tree_handoff: Option<Value>,
    #[serde(default)]
    appoint_manager: bool,
    manager_handoff: Option<Value>,
    #[serde(default)]
    stock_watch_clock: bool,
    stock_clock_recovery: Option<Value>,
}

pub(super) struct Config {
    pub(super) directory: PathBuf,
    pub(super) initialize: bool,
    pub(super) acknowledged: Option<journal::Pin>,
    pub(super) goal: ProjectStockGoal,
    pub(super) original: Value,
    pub(super) handoff: Option<WorkshopProjectHandoff>,
    pub(super) tree: Option<ProjectTreeConfig>,
    pub(super) tree_handoff: Option<TreeProjectHandoff>,
    pub(super) manager: Option<ProjectManagerConfig>,
    pub(super) manager_handoff: Option<ManagerProjectHandoff>,
    pub(super) stock_watch_clock: bool,
    pub(super) stock_clock_recovery: Option<Value>,
    guard: Arc<dyn Fn() -> Result<()> + Send + Sync>,
}

fn directory(path: &Path) -> Result<()> {
    let raw = path
        .to_str()
        .ok_or_else(|| invalid("project directory must be UTF-8"))?;
    if !path.is_absolute()
        || raw.len() < 2
        || raw.len() > 3800
        || raw[1..].split('/').any(|part| {
            part.is_empty() || part == "." || part == ".." || part.chars().any(char::is_control)
        })
    {
        return Err(invalid(
            "project requires a canonical absolute custody directory",
        ));
    }
    Ok(())
}

fn disjoint(paths: &[&Path]) -> Result<()> {
    for (index, path) in paths.iter().enumerate() {
        directory(path)?;
        if paths[..index]
            .iter()
            .any(|other| path.starts_with(other) || other.starts_with(path))
        {
            return Err(invalid(
                "parent and all native child custody directories must be disjoint",
            ));
        }
    }
    Ok(())
}


impl Config {
    pub(super) fn load() -> Result<(Self, ProjectWorkshopTemplate)> {
        runtime_io()?;
        environment()?;
        let mut raw = CONFIGURATIONS
            .map(read)
            .into_iter()
            .collect::<Result<Vec<_>>>()?;
        let mut input: Input = serde_json::from_str(&raw[0])
            .map_err(|_| invalid("invalid closed resource project configuration"))?;
        input.goal.canonicalize()?;
        directory(&input.directory)?;
        if let Some(recovery) = &input.stock_clock_recovery {
            if input.initialize || !input.stock_watch_clock {
                return Err(invalid(
                    "stock-clock recovery only narrows an existing opted-in project",
                ));
            }
            super::runtime::stock_clock::ClockCheckpoint::from_json(recovery)?;
        }
        if input.stock_watch_clock {
            let production: Value = serde_json::from_str(&raw[2])
                .map_err(|_| invalid("invalid production configuration"))?;
            if !production["inventory_clock"].is_object() {
                return Err(invalid(
                    "stock_watch_clock requires an independently configured production inventory clock",
                ));
            }
        }
        if input.schema != "dfmcp.native-resource-project-development/1"
            || input.initialize == input.acknowledged.is_some()
            || (input.initialize
                && (input.handoff.is_some()
                    || input.tree_handoff.is_some()
                    || input.manager_handoff.is_some()))
            || (!input.harvest_tree && input.tree_handoff.is_some())
            || (!input.appoint_manager && input.manager_handoff.is_some())
        {
            return Err(invalid(
                "invalid project schema, initialization or retained acknowledgement",
            ));
        }
        let mut names = CONFIGURATIONS.to_vec();
        if input.harvest_tree {
            names.push(TREE_CONFIG);
            raw.push(read(TREE_CONFIG)?);
        }
        if input.appoint_manager {
            names.push(MANAGER_CONFIG);
            raw.push(read(MANAGER_CONFIG)?);
        }
        let hashes: Vec<Digest32> = raw
            .iter()
            .map(|value| Digest32::of_bytes(value.as_bytes()))
            .collect();
        let guard: Arc<dyn Fn() -> Result<()> + Send + Sync> = Arc::new(move || {
            runtime_io()?;
            environment()?;
            for (name, expected) in names.iter().zip(&hashes) {
                if Digest32::of_bytes(read(name)?.as_bytes()) != *expected {
                    return Err(denied(
                        "resource project configuration changed; preserve original custody and close",
                    ));
                }
            }
            Ok(())
        });
        let handoff = input
            .handoff
            .as_ref()
            .map(WorkshopProjectHandoff::from_json)
            .transpose()?;
        let tree_handoff = input
            .tree_handoff
            .as_ref()
            .map(TreeProjectHandoff::from_json)
            .transpose()?;
        let manager_handoff = input
            .manager_handoff
            .as_ref()
            .map(ManagerProjectHandoff::from_json)
            .transpose()?;
        let manager = if input.appoint_manager {
            Some(
                ProjectManagerConfig::from_resource_project_environment(guard.clone())?
                    .with_project_deadline(input.goal.deadline_tick())?,
            )
        } else {
            None
        };
        if let (Some(manager), Some(handoff)) = (&manager, &manager_handoff) {
            manager.validate_project_handoff(handoff)?;
        }
        let tree = if input.harvest_tree {
            Some(
                ProjectTreeConfig::from_resource_project_environment(guard.clone())?
                    .with_project_deadline(input.goal.deadline_tick())?,
            )
        } else {
            None
        };
        let workshop = ProjectWorkshopTemplate::from_resource_project_environment(
            guard.clone(),
            tree.as_ref(),
        )?
        .with_project_deadline(input.goal.deadline_tick())?;
        let production = ProductionConfig::validate_resource_project_environment(
            guard.clone(),
            runtime_io,
            handoff.as_ref(),
            manager.as_ref(),
            manager_handoff.as_ref(),
        )?;
        production.matches_workshop(&workshop.project_binding()?)?;
        if let Some(manager) = &manager {
            production.matches_manager(manager.project_binding())?;
            let m = manager.project_binding();
            let w = workshop.project_binding()?;
            if m.source.folder() != w.fortress().folder()
                || m.source.site() != w.fortress().site()
                || m.endpoint != w.endpoint()
                || m.dimensions != w.dimensions()
                || m.df_version != w.df_version()
                || m.dfhack_version != w.dfhack_version()
            {
                return Err(denied(
                    "Manager and Workshop project source policies differ",
                ));
            }
        }
        let mut directories = vec![
            input.directory.as_path(),
            workshop.project_directory(),
            production.directory(),
            production.workforce_directory(),
            production.order_directory(),
        ];
        if let Some(tree) = &tree {
            directories.push(tree.project_directory());
        }
        if let Some(manager) = &manager {
            directories.push(manager.project_directory());
        }
        disjoint(&directories)?;
        if input.initialize
            && (!workshop.project_initializing()
                || !production.initializing()
                || tree
                    .as_ref()
                    .is_some_and(|tree| !tree.project_initializing())
                || manager
                    .as_ref()
                    .is_some_and(|manager| !manager.project_initializing()))
        {
            return Err(invalid(
                "a new resource project requires new custody for every native child",
            ));
        }
        let mut original = json!({
            "schema":"dfmcp.native-resource-project-original/1",
            "goal":input.goal,
            "directory":input.directory,
            "workshop_policy":workshop.project_identity().to_hex(),
            "production_policy":production.identity().to_hex(),
        });
        // Preserve the exact original bytes/identity of existing two-stage
        // projects. A Tree prefix is an explicit additional immutable policy.
        if let Some(tree) = &tree {
            original["tree_policy"] = json!(tree.project_identity().to_hex());
        }
        if let Some(manager) = &manager {
            original["manager_policy"] = json!(manager.project_identity().to_hex());
        }
        if input.stock_watch_clock {
            original["stock_watch_clock"] = json!(true);
        }
        Ok((
            Self {
                directory: input.directory,
                initialize: input.initialize,
                acknowledged: input.acknowledged,
                goal: input.goal,
                original,
                handoff,
                tree,
                tree_handoff,
                manager,
                manager_handoff,
                stock_watch_clock: input.stock_watch_clock,
                stock_clock_recovery: input.stock_clock_recovery,
                guard,
            },
            workshop,
        ))
    }

    pub(super) fn current(&self) -> Result<()> {
        (self.guard)()
    }

    pub(super) fn guard(&self) -> Arc<dyn Fn() -> Result<()> + Send + Sync> {
        self.guard.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portfolio_clock_credentials_follow_only_the_configured_original_families()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let parent = json!({
            "goal":{
                "targets":[
                    {"recipe":"wooden_door","minimum_stack_units":2,"max_order_amount":3},
                    {"recipe":"wooden_bed","minimum_stack_units":5,"max_order_amount":8}
                ],
                "deadline_tick":900,"poll_interval_ticks":5,"stable_samples":2,"assign_labor":false
            },
            "harvest_tree":true,
        }).to_string();
        for (order, inventory) in [(false, false), (true, false), (false, true), (true, true)] {
            let mut production = json!({});
            if order {
                production["clock"] = json!({"generation":1});
            }
            if inventory {
                production["inventory_clock"] = json!({"generation":2});
            }
            let features = Features::from_raw(
                r#"{"clock":{}}"#,
                &production.to_string(),
                Some(&parent),
                Some(r#"{"clock":{}}"#),
            )?;
            for key in [
                "DFMCP_ALLOW_UNADMITTED_TREE_CLOCK",
                "DFMCP_ALLOW_UNADMITTED_WORKSHOP_CLOCK",
                "DFMCP_RUN_TOKEN",
                "DFMCP_RUN_ALLOW_CLOCK",
            ] {
                assert!(features.allows(key), "{key}");
            }
            for key in [
                "DFMCP_SEMANTIC_WORK_ORDER_ALLOW_CLOCK",
                "DFMCP_SEMANTIC_WORK_ORDER_CLOCK_TOKEN",
            ] {
                assert_eq!(features.allows(key), order, "{key}");
            }
            for key in [
                "DFMCP_SEMANTIC_WORK_ORDER_ALLOW_INVENTORY_CLOCK",
                "DFMCP_SEMANTIC_WORK_ORDER_INVENTORY_CLOCK_TOKEN",
            ] {
                assert_eq!(features.allows(key), inventory, "{key}");
            }
        }
        Ok(())
    }

    #[test]
    fn custody_directories_cannot_alias_or_nest() {
        assert!(
            disjoint(&[
                Path::new("/custody/project"),
                Path::new("/custody/workshop")
            ])
            .is_ok()
        );
        for other in [
            "/custody/project",
            "/custody",
            "/custody/project/orders",
            "/custody/../orders",
        ] {
            assert!(disjoint(&[Path::new("/custody/project"), Path::new(other)]).is_err());
        }
    }

    #[test]
    fn credentials_follow_configured_child_features() {
        let features = Features {
            appoint_manager: false,
            harvest_tree: false,
            tree_material: false,
            tree_clock: false,
            workshop_clock: false,
            production_clock: false,
            inventory_clock: false,
            manager: false,
        };
        assert!(features.allows(CONFIG));
        assert!(features.allows("DFMCP_COMBINED_PRODUCTION_WORKSHOP_TOKEN"));
        for key in [
            "DFMCP_RUN_TOKEN",
            "DFMCP_TREE_HARVEST_TOKEN",
            "DFMCP_SEMANTIC_WORK_ORDER_CLOCK_TOKEN",
            "DFMCP_SEMANTIC_WORK_ORDER_INVENTORY_CLOCK_TOKEN",
            "DFMCP_COMBINED_PRODUCTION_MANAGER_TOKEN",
            "DFMCP_ADMITTED_BRIDGE_PROTOCOL",
        ] {
            assert!(!features.allows(key));
        }
    }

    #[test]
    fn tree_owner_and_shared_clock_credentials_require_the_configured_stage() -> Result<()> {
        let parent = r#"{"harvest_tree":true}"#;
        let features = Features::from_raw("{}", "{}", Some(parent), Some(r#"{"clock":{}}"#))?;
        for key in [
            TREE_CONFIG,
            "DFMCP_ALLOW_UNADMITTED_TREE_HARVEST_V1_21",
            "DFMCP_TREE_HARVEST_ALLOW_DESIGNATE",
            "DFMCP_TREE_HARVEST_TOKEN",
            "DFMCP_ALLOW_UNADMITTED_TREE_CLOCK",
            "DFMCP_RUN_TOKEN",
            "DFMCP_RUN_ALLOW_CLOCK",
            "DFMCP_ALLOW_UNADMITTED_WORKSHOP_TREE_MATERIAL",
        ] {
            assert!(features.allows(key), "{key}");
        }
        assert!(!features.allows("DFMCP_ALLOW_UNADMITTED_WORKSHOP_CLOCK"));
        let no_clock = Features::from_raw("{}", "{}", Some(parent), Some("{}"))?;
        assert!(!no_clock.allows("DFMCP_RUN_TOKEN"));
        assert!(!no_clock.allows("DFMCP_ALLOW_UNADMITTED_TREE_CLOCK"));
        let external_donor =
            Features::from_raw(r#"{"tree_material_source":{}}"#, "{}", None, None)?;
        assert!(external_donor.allows("DFMCP_TREE_HARVEST_TOKEN"));
        assert!(!external_donor.allows(TREE_CONFIG));
        assert!(!external_donor.allows("DFMCP_TREE_HARVEST_ALLOW_DESIGNATE"));
        assert!(Features::from_raw("{}", "{}", Some(parent), None).is_err());
        assert!(
            Features::from_raw("{}", "{}", Some(r#"{"harvest_tree":"true"}"#), Some("{}")).is_err()
        );
        Ok(())
    }

    #[test]
    fn manager_owner_requires_its_explicit_generation_and_read_only_production_guard() -> Result<()>
    {
        let features = Features::from_raw("{}", "{}", Some(r#"{"appoint_manager":true}"#), None)?;
        for key in [
            MANAGER_CONFIG,
            "DFMCP_ALLOW_UNADMITTED_MANAGER_V1_23",
            "DFMCP_MANAGER_V1_23_TOKEN",
            "DFMCP_MANAGER_V1_23_ALLOW_APPOINT",
            "DFMCP_COMBINED_PRODUCTION_MANAGER_TOKEN",
        ] {
            assert!(features.allows(key), "{key}");
        }
        for key in [
            "DFMCP_MANAGER_MCP_CONFIG",
            "DFMCP_ALLOW_UNADMITTED_MANAGER_V1_22",
            "DFMCP_MANAGER_TOKEN",
            "DFMCP_MANAGER_ALLOW_APPOINT",
            TREE_CONFIG,
            "DFMCP_RUN_TOKEN",
            "DFMCP_ADMITTED_BRIDGE_PROTOCOL",
        ] {
            assert!(!features.allows(key), "{key}");
        }
        let donor = Features::from_raw("{}", r#"{"manager_prerequisite":{}}"#, None, None)?;
        assert!(donor.allows("DFMCP_COMBINED_PRODUCTION_MANAGER_TOKEN"));
        assert!(!donor.allows(MANAGER_CONFIG));
        assert!(!donor.allows("DFMCP_MANAGER_V1_23_ALLOW_APPOINT"));
        assert!(Features::from_raw("{}", "{}", Some(r#"{"appoint_manager":1}"#), None).is_err());
        Ok(())
    }

    #[test]
    fn manager_and_tree_opt_ins_compose_without_granting_unconfigured_clocks() -> Result<()> {
        let features = Features::from_raw(
            "{}",
            "{}",
            Some(r#"{"appoint_manager":true,"harvest_tree":true}"#),
            Some("{}"),
        )?;
        assert!(features.allows(MANAGER_CONFIG));
        assert!(features.allows(TREE_CONFIG));
        assert!(features.allows("DFMCP_TREE_HARVEST_TOKEN"));
        assert!(!features.allows("DFMCP_RUN_TOKEN"));
        assert!(!features.allows("DFMCP_SEMANTIC_WORK_ORDER_ALLOW_CLOCK"));
        assert!(!features.allows("DFMCP_SEMANTIC_WORK_ORDER_ALLOW_INVENTORY_CLOCK"));
        Ok(())
    }
}

#[cfg(test)]
#[path = "stock_clock_config_tests.rs"]
mod stock_clock_tests;
