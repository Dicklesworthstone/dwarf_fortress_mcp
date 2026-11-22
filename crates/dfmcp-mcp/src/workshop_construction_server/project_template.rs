//! Deferred material policy for the optional parent-owned Tree stage. Parsing
//! validates every Workshop source/scope/clock pin without inventing an item or
//! acknowledgement for a Tree original which does not yet exist.
use super::*;
use crate::tree_harvest_server::{ProjectTreeConfig, TreeProjectHandoff};
use dfmcp_adapter::tree_harvest::rpc::Binding as TreeBinding;
use std::path::Path;

pub(super) const SCHEMA: &str = "dfmcp.workshop-project-tree-material-mcp-development/1";

#[derive(Clone)]
struct ExpectedTree {
    directory: PathBuf,
    binding: TreeBinding,
    root: [u32; 3],
    deadline: u64,
}
impl ExpectedTree {
    fn new(
        config: &Config,
        directory: &Path,
        binding: &TreeBinding,
        root: [u32; 3],
        deadline: u64,
    ) -> Result<Self> {
        if !config.project_tree_pending
            || binding.source != config.identity
            || binding.dimensions != config.input.map_dimensions
            || binding.df_version != config.input.df_version
            || binding.dfhack_version != config.input.dfhack_version
            || directory == config.project_directory()
        {
            return Err(invalid(
                "Tree and Workshop project source/software or custody policy differs",
            ));
        }
        binding.validate()?;
        Ok(Self {
            directory: directory.to_owned(),
            binding: binding.clone(),
            root,
            deadline,
        })
    }
}

#[derive(Clone)]
pub(crate) struct ProjectWorkshopTemplate {
    config: Config,
    tree: Option<ExpectedTree>,
}

impl ProjectWorkshopTemplate {
    pub(crate) fn from_resource_project_environment(
        parent_check: Arc<dyn Fn() -> Result<()> + Send + Sync>,
        tree: Option<&ProjectTreeConfig>,
    ) -> Result<Self> {
        runtime_io()?;
        parent_check()?;
        validate_family_environment()?;
        let raw = std::env::var(CONFIG)
            .map_err(|_| denied("operator Workshop configuration is missing"))?;
        let result = Self::parse(&raw, tree, parent_check)?;
        result.config.current()?;
        // Retain the parsed object; no Tree files, native source or credential
        // is accessed until the parent persists WorkshopOpening and activates.
        Ok(result)
    }

    fn parse(
        raw: &str,
        tree: Option<&ProjectTreeConfig>,
        parent_check: Arc<dyn Fn() -> Result<()> + Send + Sync>,
    ) -> Result<Self> {
        let mut config = Config::parse_project_template(raw, tree.is_some())?;
        config.parent_check = Some(parent_check);
        if config.clock.is_some()
            && config.policy["clock"]["initialize"] != json!(config.input.initialize)
        {
            return Err(invalid(
                "project Workshop and clock must share initialization or retained recovery",
            ));
        }
        let expected = tree
            .map(|tree| -> Result<ExpectedTree> {
                if !config.project_tree_pending {
                    return Err(invalid(
                        "automatic Tree harvest requires the deferred Workshop material schema",
                    ));
                }
                ExpectedTree::new(
                    &config,
                    tree.project_directory(),
                    tree.project_binding(),
                    tree.project_root(),
                    tree.project_deadline_tick().ok_or_else(|| {
                        invalid("Tree project deadline must be fixed before Workshop preflight")
                    })?,
                )
            })
            .transpose()?;
        Ok(Self {
            config,
            tree: expected,
        })
    }

    pub(crate) fn with_project_deadline(mut self, deadline: u64) -> Result<Self> {
        if self
            .tree
            .as_ref()
            .is_some_and(|tree| tree.deadline != deadline)
        {
            return Err(invalid(
                "Tree and Workshop must retain the same original project deadline",
            ));
        }
        self.config = self.config.with_project_deadline(deadline)?;
        Ok(self)
    }
    pub(crate) fn project_identity(&self) -> Digest32 {
        self.config.project_identity()
    }
    pub(crate) fn project_binding(&self) -> Result<WorkshopBinding> {
        self.config.project_binding()
    }
    pub(crate) fn project_directory(&self) -> &Path {
        self.config.project_directory()
    }
    pub(crate) fn project_initializing(&self) -> bool {
        self.config.project_initializing()
    }
    pub(crate) fn project_clock_configured(&self) -> bool {
        self.config.project_clock_configured()
    }
    pub(crate) fn project_context(
        &self,
        mode: WorkshopMode,
        session: SessionId,
        request: RequestId,
        budget: WorkBudget,
    ) -> Result<OperationContext> {
        self.config.project_context(mode, session, request, budget)
    }

    pub(crate) fn activate(&self, handoff: Option<&TreeProjectHandoff>) -> Result<Config> {
        self.config.current()?;
        self.activate_verified_policy(handoff)
    }

    fn activate_verified_policy(&self, handoff: Option<&TreeProjectHandoff>) -> Result<Config> {
        let mut config = self.config.clone();
        match (&self.tree, handoff) {
            (None, None) if !config.project_tree_pending => Ok(config),
            (Some(expected), Some(handoff)) => {
                if handoff.directory() != expected.directory
                    || handoff.binding() != &expected.binding
                    || handoff.root() != expected.root
                    || handoff.project_deadline_tick() != expected.deadline
                    || Some(expected.deadline) != config.project_deadline
                {
                    return Err(denied(
                        "completed Tree handoff differs from the preflighted project policy",
                    ));
                }
                config.tree_material = Some(tree_material::Config::from_project_handoff(
                    handoff,
                    &config.input,
                )?);
                config.project_tree_pending = false;
                Ok(config)
            }
            _ => Err(denied(
                "project Workshop material requires exactly its original Tree handoff",
            )),
        }
    }
}

#[cfg(test)]
#[path = "project_template_tests.rs"]
mod tests;
