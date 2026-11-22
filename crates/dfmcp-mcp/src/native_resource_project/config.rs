//! Closed operator policy for one Workshop -> stock-target development project.
use super::{error, journal};
use crate::live_production_runtime::{Config as ProductionConfig, ProjectStockGoal};
use crate::workshop_construction_server::{ProjectWorkshopConfig, WorkshopProjectHandoff};
use dfmcp_core::{Digest32, ErrorCode, Result};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(super) const OPT_IN: &str = "DFMCP_ALLOW_UNADMITTED_NATIVE_RESOURCE_PROJECT";
pub(super) const CONFIG: &str = "DFMCP_NATIVE_RESOURCE_PROJECT_CONFIG";
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
    cx.checkpoint()
        .map_err(|_| error(ErrorCode::CancellationRequested, "resource project cancelled"))?;
    if !cx.capabilities().io {
        return Err(denied("inherited context denies resource project I/O"));
    }
    Ok(())
}

fn read(name: &str) -> Result<String> {
    let raw = std::env::var(name)
        .map_err(|_| denied("required resource project operator configuration is missing"))?;
    let maximum = if name == CONFIG { 96 * 1024 } else { 16 * 1024 };
    if raw.is_empty() || raw.len() > maximum {
        return Err(invalid("resource project operator configuration exceeds its bound"));
    }
    Ok(raw)
}

#[derive(Clone, Copy)]
struct Features {
    tree_material: bool,
    workshop_clock: bool,
    production_clock: bool,
    inventory_clock: bool,
    manager: bool,
}
impl Features {
    fn from_raw(workshop: &str, production: &str) -> Result<Self> {
        let w: Value = serde_json::from_str(workshop)
            .map_err(|_| invalid("invalid Workshop configuration"))?;
        let p: Value = serde_json::from_str(production)
            .map_err(|_| invalid("invalid production configuration"))?;
        Ok(Self {
            tree_material: !w["tree_material_source"].is_null(),
            workshop_clock: !w["clock"].is_null(),
            production_clock: !p["clock"].is_null(),
            inventory_clock: !p["inventory_clock"].is_null(),
            manager: !p["manager_prerequisite"].is_null(),
        })
    }
    fn allows(self, key: &str) -> bool {
        match key {
            "DFMCP_ALLOW_UNADMITTED_WORKSHOP_TREE_MATERIAL" | "DFMCP_TREE_HARVEST_TOKEN" => {
                self.tree_material
            }
            "DFMCP_ALLOW_UNADMITTED_WORKSHOP_CLOCK"
            | "DFMCP_RUN_TOKEN"
            | "DFMCP_RUN_ALLOW_CLOCK" => self.workshop_clock,
            "DFMCP_SEMANTIC_WORK_ORDER_ALLOW_CLOCK"
            | "DFMCP_SEMANTIC_WORK_ORDER_CLOCK_TOKEN" => self.production_clock,
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
        return Err(denied("resource project requires its development opt-in and refuses production admission"));
    }
    let features = Features::from_raw(&read(CONFIGURATIONS[1])?, &read(CONFIGURATIONS[2])?)?;
    for (key, _) in std::env::vars_os() {
        if !features.allows(&key.to_string_lossy()) {
            return Err(denied("resource project refuses other DFMCP profiles and unconfigured credentials"));
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
}

pub(super) struct Config {
    pub(super) directory: PathBuf,
    pub(super) initialize: bool,
    pub(super) acknowledged: Option<journal::Pin>,
    pub(super) goal: ProjectStockGoal,
    pub(super) original: Value,
    pub(super) handoff: Option<WorkshopProjectHandoff>,
    guard: Arc<dyn Fn() -> Result<()> + Send + Sync>,
}

fn directory(path: &Path) -> Result<()> {
    let raw = path.to_str().ok_or_else(|| invalid("project directory must be UTF-8"))?;
    if !path.is_absolute()
        || raw.len() < 2
        || raw.len() > 3800
        || raw[1..].split('/').any(|part| {
            part.is_empty() || part == "." || part == ".." || part.chars().any(char::is_control)
        })
    {
        return Err(invalid("project requires a canonical absolute custody directory"));
    }
    Ok(())
}

fn disjoint(paths: &[&Path]) -> Result<()> {
    for (index, path) in paths.iter().enumerate() {
        directory(path)?;
        if paths[..index].iter().any(|other| path.starts_with(other) || other.starts_with(path)) {
            return Err(invalid("parent, Workshop, production, workforce and order custody directories must be disjoint"));
        }
    }
    Ok(())
}

impl Config {
    pub(super) fn load() -> Result<(Self, ProjectWorkshopConfig)> {
        runtime_io()?;
        environment()?;
        let raw = CONFIGURATIONS.map(read).into_iter().collect::<Result<Vec<_>>>()?;
        let input: Input = serde_json::from_str(&raw[0])
            .map_err(|_| invalid("invalid closed resource project configuration"))?;
        input.goal.validate()?;
        directory(&input.directory)?;
        if input.schema != "dfmcp.native-resource-project-development/1"
            || input.initialize == input.acknowledged.is_some()
            || (input.initialize && input.handoff.is_some())
        {
            return Err(invalid("invalid project schema, initialization or retained acknowledgement"));
        }
        let hashes: Vec<Digest32> = raw.iter().map(|value| Digest32::of_bytes(value.as_bytes())).collect();
        let guard: Arc<dyn Fn() -> Result<()> + Send + Sync> = Arc::new(move || {
            runtime_io()?;
            environment()?;
            for (name, expected) in CONFIGURATIONS.iter().zip(&hashes) {
                if Digest32::of_bytes(read(name)?.as_bytes()) != *expected {
                    return Err(denied("resource project configuration changed; preserve original custody and close"));
                }
            }
            Ok(())
        });
        let handoff = input.handoff.as_ref().map(WorkshopProjectHandoff::from_json).transpose()?;
        let workshop = ProjectWorkshopConfig::from_resource_project_environment(guard.clone())?
            .with_project_deadline(input.goal.deadline_tick)?;
        let production = ProductionConfig::validate_project_environment(
            guard.clone(), runtime_io, handoff.as_ref(),
        )?;
        production.matches_workshop(&workshop.project_binding()?)?;
        disjoint(&[
            &input.directory,
            workshop.project_directory(),
            production.directory(),
            production.workforce_directory(),
            production.order_directory(),
        ])?;
        if input.initialize && (!workshop.project_initializing() || !production.initializing()) {
            return Err(invalid("a new resource project requires new Workshop and production custodians"));
        }
        let original = json!({
            "schema":"dfmcp.native-resource-project-original/1",
            "goal":input.goal,
            "directory":input.directory,
            "workshop_policy":workshop.project_identity().to_hex(),
            "production_policy":production.identity().to_hex(),
        });
        Ok((Self {
            directory: input.directory,
            initialize: input.initialize,
            acknowledged: input.acknowledged,
            goal: input.goal,
            original,
            handoff,
            guard,
        }, workshop))
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
    fn custody_directories_cannot_alias_or_nest() {
        assert!(disjoint(&[Path::new("/custody/project"), Path::new("/custody/workshop")]).is_ok());
        for other in ["/custody/project", "/custody", "/custody/project/orders", "/custody/../orders"] {
            assert!(disjoint(&[Path::new("/custody/project"), Path::new(other)]).is_err());
        }
    }

    #[test]
    fn credentials_follow_configured_child_features() {
        let features = Features {
            tree_material: false, workshop_clock: false, production_clock: false,
            inventory_clock: false, manager: false,
        };
        assert!(features.allows(CONFIG));
        assert!(features.allows("DFMCP_COMBINED_PRODUCTION_WORKSHOP_TOKEN"));
        for key in ["DFMCP_RUN_TOKEN", "DFMCP_TREE_HARVEST_TOKEN",
            "DFMCP_SEMANTIC_WORK_ORDER_CLOCK_TOKEN",
            "DFMCP_SEMANTIC_WORK_ORDER_INVENTORY_CLOCK_TOKEN",
            "DFMCP_COMBINED_PRODUCTION_MANAGER_TOKEN", "DFMCP_ADMITTED_BRIDGE_PROTOCOL"] {
            assert!(!features.allows(key));
        }
    }
}
