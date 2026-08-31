#![forbid(unsafe_code)]

pub use dfmcp_adapter::{CitizenRecord, LiveObservationCapsule};

#[path = "../src/live_projection.rs"]
mod live_projection;

use dfmcp_core::{FortressId, GameTick, ObservationCursor, Result};
use live_projection::{
    LiveProjectionContext, fortress_entity_id, project_live_observation, unit_entity_id,
};

#[test]
fn public_projection_contract_uses_stable_disjoint_identity_domains() -> Result<()> {
    let fortress = fortress_entity_id(FortressId::new(7)).ok_or_else(|| {
        dfmcp_core::DfmcpError::new(
            dfmcp_core::ErrorCode::InternalInvariantViolation,
            "test fortress ID was not representable",
        )
    })?;
    let unit = unit_entity_id(7).ok_or_else(|| {
        dfmcp_core::DfmcpError::new(
            dfmcp_core::ErrorCode::InternalInvariantViolation,
            "test unit ID was not representable",
        )
    })?;
    assert_ne!(fortress, unit);
    Ok(())
}

#[test]
fn projection_context_rejects_zero_fortress_lineage() {
    let context = LiveProjectionContext {
        fortress_id: FortressId::NIL,
        cursor: ObservationCursor::ORIGIN,
        observed_at: GameTick::new(1),
        expected_site_id: None,
    };
    assert!(context.validate().is_err());
}

#[test]
fn projection_function_is_linked_into_the_adapter_test_graph() {
    let _projection = project_live_observation;
}
