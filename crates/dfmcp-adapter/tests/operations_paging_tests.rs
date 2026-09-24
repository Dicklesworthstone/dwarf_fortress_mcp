#![forbid(unsafe_code)]
use dfmcp_adapter::live_jobs::LiveJobObservation;
use dfmcp_adapter::live_operations::{
    LiveItem, LiveOperationsObservation, LiveOperationsState, OperationsProfile,
};
use dfmcp_core::{MapCoord, Result};
use dfmcp_world::FactSource;

fn observation(count: u32) -> LiveOperationsObservation {
    LiveOperationsObservation {
        jobs: LiveJobObservation {
            bridge_generation: 7,
            df_version: "df".to_owned(),
            dfhack_version: "dfhack".to_owned(),
            year: 105,
            year_tick: 3,
            paused: true,
            site_id: 1,
            world_folder: "region1".to_owned(),
            next_job_id: 0,
            jobs: Vec::new(),
        },
        next_building_id: 0,
        next_item_id: count,
        buildings: Vec::new(),
        attachments: Vec::new(),
        items: (0..count)
            .map(|id| LiveItem {
                native_id: id,
                item_type: 1,
                type_key: "item_type_1".to_owned(),
                subtype: -1,
                material_type: 0,
                material_index: 1,
                stack_size: 1,
                raw_position: MapCoord::new(0, 0, 0),
                flags: 64,
                container_native_id: None,
                holder_building_native_id: None,
            })
            .collect(),
    }
}

#[test]
fn large_roster_requires_the_explicit_new_profile() -> Result<()> {
    let source = observation(40_000);
    assert!(source.validate().is_err());
    assert!(source.encode_payload().is_err());
    let bytes = source.encode_profile(OperationsProfile::PagedV1_4)?;
    assert!(bytes.len() > 2 * 1024 * 1024);
    assert!(
        LiveOperationsObservation::decode_payload(&bytes, 7, "df".to_owned(), "dfhack".to_owned())
            .is_err()
    );
    assert_eq!(
        LiveOperationsObservation::decode_profile(
            &bytes,
            7,
            "df".to_owned(),
            "dfhack".to_owned(),
            OperationsProfile::PagedV1_4
        )?,
        source
    );
    assert!(
        observation(65_537)
            .validate_profile(OperationsProfile::PagedV1_4)
            .is_err()
    );
    Ok(())
}

#[test]
fn profile_identity_does_not_alias_even_for_small_equal_rosters() -> Result<()> {
    let source = observation(3);
    let old = source.encode_payload()?;
    assert!(
        LiveOperationsObservation::decode_profile(
            &old,
            7,
            "df".to_owned(),
            "dfhack".to_owned(),
            OperationsProfile::PagedV1_4
        )
        .is_err()
    );
    let mut old_state = LiveOperationsState::default();
    old_state.publish(source.clone())?;
    let mut state = LiveOperationsState::with_profile(OperationsProfile::PagedV1_4);
    state.publish(source.clone())?;
    assert_ne!(old_state.source_digest()?, state.source_digest()?);
    assert_ne!(old_state.snapshot(), state.snapshot());
    assert_eq!(
        state.source_digest()?,
        source.source_digest_profile(OperationsProfile::PagedV1_4)?
    );
    let snapshot = state.snapshot().ok_or_else(|| {
        dfmcp_core::DfmcpError::new(
            dfmcp_core::ErrorCode::InternalInvariantViolation,
            "test snapshot absent",
        )
    })?;
    for entity in snapshot.graph.entities.values() {
        for fact in entity.fields.values() {
            assert_eq!(fact.source_digest, state.source_digest()?);
            assert!(
                matches!(&fact.source, FactSource::DfhackField(path) if path.starts_with("operations/1.4."))
            );
        }
    }
    Ok(())
}

#[test]
fn malformed_extended_roster_preserves_the_old_published_anchor() -> Result<()> {
    let source = observation(3);
    let mut state = LiveOperationsState::with_profile(OperationsProfile::PagedV1_4);
    state.publish(source.clone())?;
    let prior = state.snapshot().cloned();
    let mut broken = source;
    broken.items[0].container_native_id = Some(99);
    assert!(state.publish(broken).is_err());
    assert_eq!(state.snapshot(), prior.as_ref());
    Ok(())
}
