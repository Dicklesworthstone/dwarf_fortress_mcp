//! Closed observation codecs for the shared journal. A file has one profile for
//! its entire lifetime; selecting another profile is never an archive migration.

use dfmcp_core::{Digest32, FortressId, Result};
use dfmcp_world::WorldSnapshot;
use crate::live_jobs::{JobPublication, LiveJobObservation};
use crate::live_operations::{LiveOperationsObservation, LiveOperationsState, OperationsProfile};
use crate::live_spatial::{LiveSpatialObservation, LiveSpatialState, MAX_SPATIAL_BYTES};

mod sealed {
    pub trait Sealed {}
}

/// Sealed so caller-defined codecs cannot reinterpret a trusted journal file.
/// Implementations validate native data through the existing canonical codecs.
pub trait JournalProfile: sealed::Sealed {
    type Observation: Clone;
    type State: Clone;
    const MAGIC: &'static [u8; 8];
    const NAME: &'static str;
    const MAX_PAYLOAD: usize;
    const IDENTITY_DOMAIN: &'static [u8];
    fn empty() -> Self::State;
    fn publish(state: &mut Self::State, value: Self::Observation) -> Result<JobPublication>;
    fn snapshot(state: &Self::State) -> Option<&WorldSnapshot>;
    fn encode(value: &Self::Observation) -> Result<Vec<u8>>;
    fn decode(bytes: &[u8], generation: u64, df: String, dfhack: String) -> Result<Self::Observation>;
    fn source_digest(value: &Self::Observation) -> Result<Digest32>;
    fn jobs(value: &Self::Observation) -> &LiveJobObservation;
    fn entity_count(value: &Self::Observation) -> usize;
    fn fortress(value: &Self::Observation) -> Result<FortressId> {
        Self::jobs(value).fortress_id()
    }
}

pub struct Operations13;
pub struct Operations14;
pub struct Spatial16;
impl sealed::Sealed for Operations13 {}
impl sealed::Sealed for Operations14 {}
impl sealed::Sealed for Spatial16 {}

macro_rules! operations_profile {
    ($name:ident, $profile:expr, $magic:expr, $text:expr, $domain:expr) => {
        impl JournalProfile for $name {
            type Observation = LiveOperationsObservation;
            type State = LiveOperationsState;
            const MAGIC: &'static [u8; 8] = $magic;
            const NAME: &'static str = $text;
            const MAX_PAYLOAD: usize = $profile.maximum_bytes();
            const IDENTITY_DOMAIN: &'static [u8] = $domain;
            fn empty() -> Self::State { LiveOperationsState::with_profile($profile) }
            fn publish(state: &mut Self::State, value: Self::Observation) -> Result<JobPublication> { state.publish(value) }
            fn snapshot(state: &Self::State) -> Option<&WorldSnapshot> { state.snapshot() }
            fn encode(value: &Self::Observation) -> Result<Vec<u8>> { value.encode_profile($profile) }
            fn decode(bytes: &[u8], generation: u64, df: String, dfhack: String) -> Result<Self::Observation> {
                LiveOperationsObservation::decode_profile(bytes, generation, df, dfhack, $profile)
            }
            fn source_digest(value: &Self::Observation) -> Result<Digest32> { value.source_digest_profile($profile) }
            fn jobs(value: &Self::Observation) -> &LiveJobObservation { &value.jobs }
            fn entity_count(value: &Self::Observation) -> usize {
                1usize.saturating_add(value.jobs.jobs.len()).saturating_add(value.buildings.len()).saturating_add(value.items.len())
            }
        }
    };
}

// The 1.3 magic, identity derivation, framing and payload bytes are unchanged.
operations_profile!(Operations13, OperationsProfile::V1_3, b"DFMOJ001", "operations/1.3",
    b"dfmcp-operations-journal-incarnation/1\0");
operations_profile!(Operations14, OperationsProfile::PagedV1_4, b"DFMPJ001", "operations/1.4",
    b"dfmcp-paged-operations-journal-incarnation/1\0");

impl JournalProfile for Spatial16 {
    type Observation = LiveSpatialObservation;
    type State = LiveSpatialState;
    const MAGIC: &'static [u8; 8] = b"DFMSJ001";
    const NAME: &'static str = "spatial/1.6";
    const MAX_PAYLOAD: usize = MAX_SPATIAL_BYTES;
    const IDENTITY_DOMAIN: &'static [u8] = b"dfmcp-spatial-journal-incarnation/1\0";
    fn empty() -> Self::State { LiveSpatialState::default() }
    fn publish(state: &mut Self::State, value: Self::Observation) -> Result<JobPublication> { state.publish(value) }
    fn snapshot(state: &Self::State) -> Option<&WorldSnapshot> { state.snapshot() }
    fn encode(value: &Self::Observation) -> Result<Vec<u8>> { value.encode_payload() }
    fn decode(bytes: &[u8], generation: u64, df: String, dfhack: String) -> Result<Self::Observation> {
        LiveSpatialObservation::decode_payload(bytes, generation, df, dfhack)
    }
    fn source_digest(value: &Self::Observation) -> Result<Digest32> { value.source_digest() }
    fn jobs(value: &Self::Observation) -> &LiveJobObservation { &value.operations().jobs }
    fn entity_count(value: &Self::Observation) -> usize {
        Operations14::entity_count(value.operations()).saturating_add(value.terrain().map.cells.len())
    }
}

#[cfg(test)]
#[path = "journal_profile_tests.rs"]
mod tests;
