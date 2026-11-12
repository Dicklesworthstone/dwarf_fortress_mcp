//! Read-only operator pins for one independently acknowledged Manager 1.23 original.
use crate::semantic_workforce_runtime::{denied, digest, invalid};
use crate::{semantic_work_order_runtime as orders, semantic_workforce_runtime as workforce};
use dfmcp_adapter::manager_appointment::{
    FortressIdentity, ManagerProtocol, Selection,
    production_origin::{MAX_ORIGINAL_FRAMES, ManagerProductionFloor},
    rpc::Binding,
};
use dfmcp_adapter::workshop_construction::journal::WorkshopPin;
use dfmcp_core::{Digest32, MapCoord, MapCuboid, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub(super) const TOKEN: &str = "DFMCP_COMBINED_PRODUCTION_MANAGER_TOKEN";

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
    unit_id: u32,
    histfig_id: u32,
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
pub(super) struct ManagerConfig {
    pub(super) directory: PathBuf,
    pub(super) binding: Binding,
    pub(super) acknowledged: WorkshopPin,
    pub(super) original_key: String,
    pub(super) original_plan_digest: Digest32,
}

impl Input {
    pub(super) fn validate(self, parent: &Path) -> Result<ManagerConfig> {
        let path = self
            .directory
            .to_str()
            .ok_or_else(|| invalid("manager donor directory must be UTF-8"))?;
        if !self.directory.is_absolute()
            || !(2..=3800).contains(&path.len())
            || path[1..].split('/').any(|part| {
                part.is_empty() || part == "." || part == ".." || part.chars().any(char::is_control)
            })
            || self.directory.starts_with(parent)
            || parent.starts_with(&self.directory)
            || !(1..=MAX_ORIGINAL_FRAMES).contains(&self.acknowledged.frames)
            || self
                .map_dimensions
                .iter()
                .any(|dimension| !(1..=32768).contains(dimension))
            || [&self.world_folder, &self.df_version, &self.dfhack_version]
                .iter()
                .any(|value| value.is_empty() || value.chars().any(char::is_control))
            || self.world_folder.len() > 512
            || self.df_version.len() > 128
            || self.dfhack_version.len() > 128
        {
            return Err(invalid(
                "invalid manager donor directory, source pins or external acknowledgement",
            ));
        }
        let endpoint = dfmcp_adapter::parse_loopback_endpoint(&self.endpoint)?;
        if !endpoint.is_ipv4() || endpoint.port() == 0 || endpoint.to_string() != self.endpoint {
            return Err(invalid(
                "manager donor requires a canonical numeric IPv4 loopback endpoint",
            ));
        }
        dfmcp_adapter::bounded_run::validate_key(&self.original_key)?;
        let binding = Binding {
            protocol: ManagerProtocol::V1_23,
            endpoint,
            source: FortressIdentity::new(&self.world_folder, self.site_id)?,
            generation: self.generation,
            dimensions: self.map_dimensions,
            scope: MapCuboid::new(
                MapCoord::new(0, 0, 0),
                MapCoord::new(
                    (self.map_dimensions[0] - 1) as i32,
                    (self.map_dimensions[1] - 1) as i32,
                    (self.map_dimensions[2] - 1) as i32,
                ),
            )?,
            selection: Selection::new(self.unit_id, self.histfig_id)?,
            df_version: self.df_version,
            dfhack_version: self.dfhack_version,
        };
        binding.validate()?;
        Ok(ManagerConfig {
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

impl ManagerConfig {
    pub(super) fn matching_sources(
        &self,
        workforce: &workforce::policy::SourceAcceptance,
        orders: &orders::policy::SourceAcceptance,
    ) -> bool {
        self.binding.source.folder() == workforce.world_folder
            && self.binding.source.folder() == orders.world_folder
            && self.binding.source.site() == workforce.site_id
            && self.binding.source.site() == orders.site_id
            && self.binding.df_version == workforce.df_version
            && self.binding.df_version == orders.df_version
            && self.binding.dfhack_version == workforce.dfhack_version
            && self.binding.dfhack_version == orders.dfhack_version
    }

    /// Offline custody verification never obtains a credential. Each fresh
    /// source reader pins this independent token for its own connection.
    pub(super) fn token(&self) -> Result<Vec<u8>> {
        let value = std::env::var(TOKEN)
            .map_err(|_| denied("required read-only manager credential is absent or not UTF-8"))?;
        Self::validate_token(&value)?;
        Ok(value.into_bytes())
    }

    pub(super) fn validate_token(value: &str) -> Result<()> {
        if !(32..=256).contains(&value.len()) || value.contains('\0') {
            return Err(invalid(
                "read-only manager credential exceeds its fixed bound",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct ManagerRecoveryFloor {
    pub(super) tick: u64,
    pub(super) generation: u64,
    pub(super) sequence: u64,
    pub(super) next_event: u32,
    pub(super) history_count: u32,
    pub(super) group_next_position: u32,
    pub(super) group_next_assignment: u32,
    pub(super) civ_next_position: u32,
    pub(super) civ_next_assignment: u32,
    pub(super) fenced: bool,
}

impl From<ManagerRecoveryFloor> for ManagerProductionFloor {
    fn from(floor: ManagerRecoveryFloor) -> Self {
        Self {
            tick: floor.tick,
            generation: floor.generation,
            sequence: floor.sequence,
            next_event: floor.next_event,
            history_count: floor.history_count,
            group_next_position: floor.group_next_position,
            group_next_assignment: floor.group_next_assignment,
            civ_next_position: floor.civ_next_position,
            civ_next_assignment: floor.civ_next_assignment,
            fenced: floor.fenced,
        }
    }
}

impl ManagerRecoveryFloor {
    pub(super) fn validate(
        self,
        original_generation: u64,
        authority_tick_floor: u64,
    ) -> Result<()> {
        ManagerProductionFloor::from(self).validate()?;
        if self.generation < original_generation
            || (!self.fenced && self.generation != original_generation)
            || authority_tick_floor < self.tick
        {
            return Err(denied(
                "manager recovery must retain consumed source floors and sticky continuity loss",
            ));
        }
        Ok(())
    }
}

pub(super) fn recovery_floor<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<ManagerRecoveryFloor>, D::Error> {
    ManagerRecoveryFloor::deserialize(deserializer).map(Some)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ManagerPrerequisiteCheckpoint {
    pub(super) origin_witness: Digest32,
    pub(super) floor: ManagerRecoveryFloor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckpointInput {
    origin_witness: String,
    tick: u64,
    generation: u64,
    sequence: u64,
    next_event: u32,
    history_count: u32,
    group_next_position: u32,
    group_next_assignment: u32,
    civ_next_position: u32,
    civ_next_assignment: u32,
    fenced: bool,
}

pub(super) fn checkpoint<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<ManagerPrerequisiteCheckpoint>, D::Error> {
    let input = CheckpointInput::deserialize(deserializer)?;
    Ok(Some(ManagerPrerequisiteCheckpoint {
        origin_witness: digest(&input.origin_witness).map_err(serde::de::Error::custom)?,
        floor: ManagerRecoveryFloor {
            tick: input.tick,
            generation: input.generation,
            sequence: input.sequence,
            next_event: input.next_event,
            history_count: input.history_count,
            group_next_position: input.group_next_position,
            group_next_assignment: input.group_next_assignment,
            civ_next_position: input.civ_next_position,
            civ_next_assignment: input.civ_next_assignment,
            fenced: input.fenced,
        },
    }))
}
