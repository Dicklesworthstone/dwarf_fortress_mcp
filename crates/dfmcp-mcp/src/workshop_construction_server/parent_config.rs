//! Configuration seam for one fixed parent owner. The parent checks the exact
//! union of profile state; the Workshop still checks its own pins and authorities.
use super::{Config, WorkshopBinding, denied, invalid};
use dfmcp_core::Result;
use serde::Deserialize;
use std::path::Path;
use std::sync::Arc;

impl Config {
    /// Pure decoding for the parent to validate every source and custody path
    /// before opening either native owner.
    pub(crate) fn parse_for_parent(raw: &str) -> Result<Self> {
        let config = Self::parse(raw)?;
        #[derive(Deserialize)]
        struct ClockMode {
            initialize: bool,
        }
        #[derive(Deserialize)]
        struct Modes {
            initialize: bool,
            clock: Option<ClockMode>,
        }
        // Full closed decoding above already rejects duplicate and unknown
        // fields. The staged parent cannot initialize a new clock on recovery.
        let modes: Modes = serde_json::from_str(raw)
            .map_err(|_| invalid("invalid staged Workshop initialization"))?;
        if modes
            .clock
            .is_some_and(|clock| clock.initialize != modes.initialize)
        {
            return Err(denied(
                "staged Workshop and its clock must share initialization or retained recovery",
            ));
        }
        Ok(config)
    }

    pub(crate) fn from_parent_environment(
        guard: Arc<dyn Fn() -> Result<()> + Send + Sync>,
    ) -> Result<Self> {
        guard()?;
        let raw = std::env::var(super::CONFIG)
            .map_err(|_| denied("operator Workshop configuration is missing"))?;
        let mut config = Self::parse_for_parent(&raw)?;
        config.parent_guard = Some(guard);
        config.material_environment()?;
        config.clock_environment()?;
        config.current()?;
        Ok(config)
    }

    pub(crate) fn source_binding(&self) -> Result<WorkshopBinding> {
        WorkshopBinding::from_source_pins(
            self.endpoint,
            self.identity.clone(),
            self.input.generation,
            self.input.map_dimensions,
            &self.input.df_version,
            &self.input.dfhack_version,
        )
    }

    pub(crate) fn custody_directory(&self) -> &Path {
        &self.input.directory
    }

    pub(crate) fn tree_custody_directory(&self) -> Option<&Path> {
        self.tree_material.as_ref().map(|tree| tree.directory())
    }

    pub(crate) fn initializing(&self) -> bool {
        self.input.initialize
    }

    pub(crate) fn clock_configured(&self) -> bool {
        self.clock.is_some()
    }
}
