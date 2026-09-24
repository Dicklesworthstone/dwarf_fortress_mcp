use dfmcp_adapter::live_jobs::JobPublication;
use dfmcp_adapter::live_operations::{
    LiveOperationsObservation, LiveOperationsState, building_entity_id, item_entity_id,
};
use dfmcp_core::{DfmcpError, EntityId, ErrorCode, Result};
use dfmcp_world::{EdgeKind, EntityKind, Value};

fn fixture() -> Result<LiveOperationsObservation> {
    let hex = include_str!("fixtures/operations_v1_3.hex").trim();
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16)
                .map_err(|_| DfmcpError::new(ErrorCode::InvalidRequest, "invalid golden hex"))
        })
        .collect::<Result<Vec<_>>>()?;
    LiveOperationsObservation::decode_payload(&bytes, 7, "df".to_owned(), "dfhack".to_owned())
}
fn snapshot(state: &LiveOperationsState) -> Result<&dfmcp_world::WorldSnapshot> {
    state.snapshot().ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "missing fixture snapshot",
        )
    })
}
#[test]
fn native_golden_roundtrips_with_distinct_entity_namespaces() -> Result<()> {
    let observation = fixture()?;
    let bytes = observation.encode_payload()?;
    assert_eq!(bytes.len(), 423);
    assert_eq!(
        LiveOperationsObservation::decode_payload(&bytes, 7, "df".to_owned(), "dfhack".to_owned())?,
        observation
    );
    let mut state = LiveOperationsState::default();
    assert_eq!(state.publish(observation)?, JobPublication::Bootstrap);
    let snapshot = snapshot(&state)?;
    assert!(snapshot.hash_is_valid());
    assert_eq!(snapshot.graph.entities.len(), 7);
    assert_eq!(
        snapshot.graph.entities[&EntityId::new(9)].kind,
        EntityKind::Job
    );
    assert_eq!(
        snapshot.graph.entities[&building_entity_id(20)].kind,
        EntityKind::Building
    );
    assert_eq!(
        snapshot.graph.entities[&item_entity_id(30)].kind,
        EntityKind::Item
    );
    assert_eq!(
        snapshot.graph.entities[&item_entity_id(30)].fields["stack_size"].value,
        Value::U64(5)
    );
    for edge in snapshot.graph.edges.values() {
        assert!(snapshot.graph.entities.contains_key(&edge.from));
        assert!(snapshot.graph.entities.contains_key(&edge.to));
    }
    assert!(
        snapshot
            .graph
            .edges
            .values()
            .any(|edge| edge.kind == EdgeKind::Uses
                && edge.from == EntityId::new(9)
                && edge.to == item_entity_id(30))
    );
    Ok(())
}
#[test]
fn every_truncated_prefix_and_trailing_data_are_rejected() -> Result<()> {
    let bytes = fixture()?.encode_payload()?;
    for length in 0..bytes.len() {
        assert!(
            LiveOperationsObservation::decode_payload(
                &bytes[..length],
                7,
                "df".to_owned(),
                "dfhack".to_owned()
            )
            .is_err(),
            "prefix {length}"
        );
    }
    let mut trailing = bytes;
    trailing.push(0);
    assert!(
        LiveOperationsObservation::decode_payload(
            &trailing,
            7,
            "df".to_owned(),
            "dfhack".to_owned()
        )
        .is_err()
    );
    Ok(())
}
#[test]
fn dangling_relations_cycles_count_lies_and_unknown_flags_fail() -> Result<()> {
    for mutation in 0..8 {
        let mut value = fixture()?;
        match mutation {
            0 => value.items[0].container_native_id = Some(99),
            1 => value.items[1].container_native_id = Some(30),
            2 => value.jobs.jobs[0].holder_native_id = Some(99),
            3 => value.attachments[0].item_native_id = 99,
            4 => value.attachments.clear(),
            5 => value.items[0].flags = 1 << 9,
            6 => value.buildings[0].build_stage = 4,
            _ => value.attachments[0].filter_index = 1,
        }
        assert!(value.validate().is_err(), "mutation {mutation}");
    }
    Ok(())
}
#[test]
fn invalid_combined_publication_preserves_every_prior_domain() -> Result<()> {
    let mut state = LiveOperationsState::default();
    state.publish(fixture()?)?;
    let before = snapshot(&state)?.clone();
    let mut value = fixture()?;
    value.jobs.year_tick += 1;
    value.items[0].stack_size = 999;
    value.buildings[0].native_id = 99;
    assert!(state.publish(value).is_err());
    assert_eq!(snapshot(&state)?, &before);
    Ok(())
}
#[test]
fn inventory_only_change_advances_combined_anchor_and_stable_edges() -> Result<()> {
    let mut state = LiveOperationsState::default();
    let value = fixture()?;
    state.publish(value.clone())?;
    let before = snapshot(&state)?.clone();
    assert_eq!(state.publish(value.clone())?, JobPublication::Heartbeat);
    let mut changed = value;
    changed.items[0].stack_size = 6;
    assert_eq!(state.publish(changed)?, JobPublication::Advanced);
    let after = snapshot(&state)?;
    assert_ne!(before.state_hash, after.state_hash);
    assert_eq!(after.cursor.sequence, 1);
    assert_eq!(
        before.graph.edges.keys().collect::<Vec<_>>(),
        after.graph.edges.keys().collect::<Vec<_>>()
    );
    assert!(after.graph.edges.values().all(|edge| edge.revision == 2));
    Ok(())
}
#[test]
fn observed_item_retirement_and_return_advance_generation() -> Result<()> {
    let mut state = LiveOperationsState::default();
    state.publish(fixture()?)?;
    let mut fewer = fixture()?;
    fewer.items.pop();
    fewer.jobs.year_tick += 1;
    state.publish(fewer)?;
    assert!(
        !snapshot(&state)?
            .graph
            .entities
            .contains_key(&item_entity_id(32))
    );
    let mut returned = fixture()?;
    returned.jobs.year_tick += 2;
    state.publish(returned)?;
    assert_eq!(
        snapshot(&state)?.graph.entities[&item_entity_id(32)].generation,
        2
    );
    assert_eq!(
        snapshot(&state)?.graph.entities[&item_entity_id(30)].generation,
        1
    );
    Ok(())
}
#[test]
fn any_identity_horizon_regression_resets_all_domain_handles() -> Result<()> {
    for domain in 0..3 {
        let mut state = LiveOperationsState::default();
        let mut before = fixture()?;
        before.jobs.next_job_id += 1;
        before.next_building_id += 1;
        before.next_item_id += 1;
        state.publish(before.clone())?;
        let mut after = before;
        match domain {
            0 => after.jobs.next_job_id -= 1,
            1 => after.next_building_id -= 1,
            _ => after.next_item_id -= 1,
        }
        assert_eq!(state.publish(after)?, JobPublication::Reset);
        let target = snapshot(&state)?;
        assert_eq!(target.cursor.epoch, 1);
        assert!(target.graph.entities.values().all(|e| e.generation == 2));
    }
    Ok(())
}
#[test]
fn source_digest_covers_building_inventory_and_relation_changes() -> Result<()> {
    let original = fixture()?;
    let digest = original.source_digest()?;
    let mut item = original.clone();
    item.items[0].flags |= 1;
    assert_ne!(item.source_digest()?, digest);
    let mut building = original.clone();
    building.buildings[0].build_stage = 2;
    assert_ne!(building.source_digest()?, digest);
    let mut relation = original;
    relation.attachments[0].role = 2;
    assert_ne!(relation.source_digest()?, digest);
    Ok(())
}
#[test]
fn version_or_world_switch_cannot_replace_existing_state() -> Result<()> {
    let mut state = LiveOperationsState::default();
    state.publish(fixture()?)?;
    let anchor = snapshot(&state)?.anchor();
    for field in 0..3 {
        let mut value = fixture()?;
        match field {
            0 => value.jobs.df_version = "other".to_owned(),
            1 => value.jobs.world_folder = "other".to_owned(),
            _ => value.jobs.site_id = 2,
        }
        assert!(state.publish(value).is_err());
        assert_eq!(snapshot(&state)?.anchor(), anchor);
    }
    Ok(())
}
