//! Read-only operator pins for one independently acknowledged Workshop original.
use crate::semantic_workforce_runtime::{denied, digest, invalid};
use crate::{semantic_work_order_runtime as orders, semantic_workforce_runtime as workforce};
use dfmcp_adapter::workshop_construction::journal::{MAX_FRAMES, WorkshopPin};
use dfmcp_adapter::workshop_construction::{FortressIdentity, WorkshopBinding};
use dfmcp_core::{Digest32, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub(super) const TOKEN: &str = "DFMCP_COMBINED_PRODUCTION_WORKSHOP_TOKEN";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    directory: PathBuf,
    endpoint: String,
    world_folder: String,
    site_id: u32,
    df_version: String,
    dfhack_version: String,
    generation: u64,
    map_dimensions: [u32; 3],
    acknowledged: PinInput,
    original_key: String,
    original_plan_digest: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PinInput {
    journal_id: String,
    head: String,
    frames: u32,
}

#[derive(Clone, Debug)]
pub(super) struct WorkshopConfig {
    pub(super) directory: PathBuf,
    pub(super) binding: WorkshopBinding,
    pub(super) acknowledged: WorkshopPin,
    pub(super) original_key: String,
    pub(super) original_plan_digest: Digest32,
}

impl Input {
    pub(super) fn validate(self, parent: &Path) -> Result<WorkshopConfig> {
        let path = self
            .directory
            .to_str()
            .ok_or_else(|| invalid("workshop donor directory must be UTF-8"))?;
        if !self.directory.is_absolute()
            || !(2..=3800).contains(&path.len())
            || path[1..].split('/').any(|part| {
                part.is_empty() || part == "." || part == ".." || part.chars().any(char::is_control)
            })
            || self.directory.starts_with(parent)
            || parent.starts_with(&self.directory)
            || !(1..=MAX_FRAMES).contains(&self.acknowledged.frames)
            || [&self.world_folder, &self.df_version, &self.dfhack_version]
                .iter()
                .any(|value| value.is_empty() || value.chars().any(char::is_control))
            || self.world_folder.len() > 512
            || self.df_version.len() > 128
            || self.dfhack_version.len() > 128
        {
            return Err(invalid(
                "invalid workshop donor directory, source pins or external acknowledgement",
            ));
        }
        let endpoint = dfmcp_adapter::parse_loopback_endpoint(&self.endpoint)?;
        if !endpoint.is_ipv4() || endpoint.port() == 0 || endpoint.to_string() != self.endpoint {
            return Err(invalid(
                "workshop donor requires a canonical numeric IPv4 loopback endpoint",
            ));
        }
        dfmcp_adapter::bounded_run::validate_key(&self.original_key)?;
        let binding = WorkshopBinding::from_source_pins(
            endpoint,
            FortressIdentity::new(&self.world_folder, self.site_id)?,
            self.generation,
            self.map_dimensions,
            &self.df_version,
            &self.dfhack_version,
        )?;
        Ok(WorkshopConfig {
            directory: self.directory,
            binding,
            acknowledged: WorkshopPin {
                journal_id: digest(&self.acknowledged.journal_id)?,
                head: digest(&self.acknowledged.head)?,
                frames: self.acknowledged.frames,
            },
            original_key: self.original_key,
            original_plan_digest: digest(&self.original_plan_digest)?,
        })
    }
}

impl WorkshopConfig {
    /// Native family incarnation numbers are independent. Only their common
    /// fortress and software facts are joined here; the donor retains its own
    /// endpoint, exact generation and map dimensions for native qualification.
    pub(super) fn matching_sources(
        &self,
        workforce: &workforce::policy::SourceAcceptance,
        orders: &orders::policy::SourceAcceptance,
    ) -> bool {
        let fortress = self.binding.fortress();
        fortress.folder() == workforce.world_folder
            && fortress.folder() == orders.world_folder
            && fortress.site() == workforce.site_id
            && fortress.site() == orders.site_id
            && self.binding.df_version() == workforce.df_version
            && self.binding.df_version() == orders.df_version
            && self.binding.dfhack_version() == workforce.dfhack_version
            && self.binding.dfhack_version() == orders.dfhack_version
    }

    /// Offline custody verification does not call this accessor. The native
    /// reader must pin and recheck the returned credential for its connection.
    pub(super) fn token(&self) -> Result<Vec<u8>> {
        let value = std::env::var(TOKEN)
            .map_err(|_| denied("required read-only workshop credential is absent or not UTF-8"))?;
        Self::validate_token(&value)?;
        Ok(value.into_bytes())
    }

    pub(super) fn validate_token(value: &str) -> Result<()> {
        if !(32..=256).contains(&value.len()) || value.contains('\0') {
            return Err(invalid(
                "read-only workshop credential exceeds its fixed bound",
            ));
        }
        Ok(())
    }
}
