//! Borrow the operations component and canonical identity of ONE published state.
//! Never project an embedded operations capture into a new generation universe.
use dfmcp_core::{DfmcpError, Digest32, ErrorCode, Result};
use dfmcp_world::WorldSnapshot;
use crate::live_operations::{LiveOperationsObservation, LiveOperationsState};
use crate::live_spatial::{LiveSpatialState, SpatialStateView};
use crate::live_spatial::citizens::LiveSpatialCitizenState;

mod sealed {
    pub trait Sealed {}
    impl Sealed for super::LiveOperationsState {}
    impl Sealed for super::LiveSpatialState {}
    impl Sealed for super::LiveSpatialCitizenState {}
}

/// Only canonical, privately published adapter states can implement this view.
/// The full source digest and snapshot are those of the enclosing capture, not
/// a newly synthesized operations-only snapshot with reset entity generations.
pub trait OperationsStateView: sealed::Sealed {
    fn operations_observation(&self) -> Option<&LiveOperationsObservation>;
    fn operations_snapshot(&self) -> Option<&WorldSnapshot>;
    fn operations_source_digest(&self) -> Result<Digest32>;
}

impl OperationsStateView for LiveOperationsState {
    fn operations_observation(&self) -> Option<&LiveOperationsObservation> { self.observation() }
    fn operations_snapshot(&self) -> Option<&WorldSnapshot> { self.snapshot() }
    fn operations_source_digest(&self) -> Result<Digest32> { self.source_digest() }
}

impl OperationsStateView for LiveSpatialState {
    fn operations_observation(&self) -> Option<&LiveOperationsObservation> {
        self.spatial_observation().map(|value| value.operations())
    }
    fn operations_snapshot(&self) -> Option<&WorldSnapshot> { SpatialStateView::snapshot(self) }
    fn operations_source_digest(&self) -> Result<Digest32> { SpatialStateView::source_digest(self) }
}

impl OperationsStateView for LiveSpatialCitizenState {
    fn operations_observation(&self) -> Option<&LiveOperationsObservation> {
        self.observation_full().map(|value| value.spatial().operations())
    }
    fn operations_snapshot(&self) -> Option<&WorldSnapshot> { SpatialStateView::snapshot(self) }
    fn operations_source_digest(&self) -> Result<Digest32> {
        self.observation_full().ok_or_else(||DfmcpError::new(ErrorCode::InvalidRequest,
            "coherent citizen/spatial observation absent"))?.source_digest()
    }
}
