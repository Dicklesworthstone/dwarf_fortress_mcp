//! Closed opt-in input policy and externally retained rejection checkpoints.
use super::{denied, digest, invalid};
use dfmcp_core::{Digest32, Result};
use serde::Deserialize;

pub(super) const SCHEMA: &str = "dfmcp.furniture-input-supply/1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct InputConfig {
    pub(super) reserve_wood_stack_units: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    schema: String,
    #[serde(default)]
    reserve_wood_stack_units: u64,
}

pub(super) fn configuration<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<InputConfig>, D::Error> {
    let input = Input::deserialize(deserializer)?;
    if input.schema != SCHEMA || input.reserve_wood_stack_units > 1_000_000 {
        return Err(serde::de::Error::custom(invalid(
            "input supply requires its exact schema and at most 1000000 reserved WOOD stack units",
        )));
    }
    Ok(Some(InputConfig {
        reserve_wood_stack_units: input.reserve_wood_stack_units,
    }))
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct InputFloor {
    pub(super) tick: u64,
    pub(super) generation: u64,
    pub(super) next_job_id: u32,
    pub(super) next_building_id: u32,
    pub(super) next_item_id: u32,
    pub(super) fenced: bool,
}

impl InputFloor {
    pub(super) fn validate(self, expected_generation: u64, authority_floor: u64) -> Result<()> {
        if self.tick > dfmcp_adapter::bounded_run::MAX_NATIVE_TICK
            || self.generation == 0
            || self.generation == u64::MAX
            || (expected_generation != 0
                && (self.generation < expected_generation
                    || (!self.fenced && self.generation != expected_generation)))
            || [self.next_job_id, self.next_building_id, self.next_item_id]
                .into_iter()
                .any(|id| id > i32::MAX as u32)
            || authority_floor < self.tick
        {
            return Err(denied(
                "input supply recovery lost valid source floors or its sticky fence",
            ));
        }
        Ok(())
    }

    pub(super) fn counters(self) -> [u32; 3] {
        [self.next_job_id, self.next_building_id, self.next_item_id]
    }
}

pub(super) fn recovery_floor<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<InputFloor>, D::Error> {
    InputFloor::deserialize(deserializer).map(Some)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct InputCheckpoint {
    pub(super) origin_witness: Digest32,
    pub(super) floor: InputFloor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckpointInput {
    origin_witness: String,
    tick: u64,
    generation: u64,
    next_job_id: u32,
    next_building_id: u32,
    next_item_id: u32,
    fenced: bool,
}

pub(super) fn checkpoint<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<InputCheckpoint>, D::Error> {
    let input = CheckpointInput::deserialize(deserializer)?;
    Ok(Some(InputCheckpoint {
        origin_witness: digest(&input.origin_witness).map_err(serde::de::Error::custom)?,
        floor: InputFloor {
            tick: input.tick,
            generation: input.generation,
            next_job_id: input.next_job_id,
            next_building_id: input.next_building_id,
            next_item_id: input.next_item_id,
            fenced: input.fenced,
        },
    }))
}
