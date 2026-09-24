use super::*;
use crate::live_map::tile_entity_id;
use crate::live_operations::item_entity_id;
use dfmcp_world::map_region::Cell;
use dfmcp_world::{EntityKind, FactPresence};

fn hex(text: &str) -> Result<Vec<u8>> {
    let text = text.trim();
    (0..text.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&text[i..i + 2], 16).map_err(|_| invalid("bad test fixture hex"))
        })
        .collect()
}
fn observation() -> Result<LiveSpatialObservation> {
    let mut op = hex(include_str!("../tests/fixtures/operations_v1_3.hex"))?;
    op[..8].copy_from_slice(b"DFMO1400");
    let map = hex(include_str!("../tests/fixtures/map_v1_5.hex"))?;
    let mut bytes = b"DFMS1600".to_vec();
    for part in [&op, &map] {
        bytes.extend_from_slice(&(part.len() as u32).to_be_bytes());
        bytes.extend_from_slice(part);
    }
    LiveSpatialObservation::decode_payload(&bytes, 7, "df".to_owned(), "dfhack".to_owned())
}
fn anchor(state: &LiveSpatialState) -> Result<dfmcp_core::StateAnchor> {
    Ok(state
        .snapshot()
        .ok_or_else(|| invalid("test snapshot absent"))?
        .anchor())
}
#[test]
fn roundtrip_and_every_truncated_prefix() -> Result<()> {
    let value = observation()?;
    let bytes = value.encode_payload()?;
    assert_eq!(
        LiveSpatialObservation::decode_payload(&bytes, 7, "df".to_owned(), "dfhack".to_owned())?,
        value
    );
    for n in 0..bytes.len() {
        assert!(
            LiveSpatialObservation::decode_payload(
                &bytes[..n],
                7,
                "df".to_owned(),
                "dfhack".to_owned()
            )
            .is_err()
        );
    }
    let mut extra = bytes;
    extra.push(0);
    assert!(
        LiveSpatialObservation::decode_payload(&extra, 7, "df".to_owned(), "dfhack".to_owned())
            .is_err()
    );
    assert!(
        LiveSpatialObservation::decode_payload(
            &value.terrain.encode_payload()?,
            7,
            "df".to_owned(),
            "dfhack".to_owned()
        )
        .is_err()
    );
    Ok(())
}
#[test]
fn all_cross_domain_capture_identities_must_agree() -> Result<()> {
    let original = observation()?;
    for change in 0..8 {
        let mut value = original.clone();
        match change {
            0 => value.terrain.year_tick += 1,
            1 => value.terrain.paused = !value.terrain.paused,
            2 => value.terrain.site_id += 1,
            3 => value.terrain.world_folder.push('x'),
            4 => value.terrain.bridge_generation += 1,
            5 => value.terrain.df_version.push('x'),
            6 => value.terrain.dfhack_version.push('x'),
            _ => value.terrain.year += 1,
        }
        assert!(value.validate().is_err());
    }
    Ok(())
}
#[test]
fn every_entity_and_relationship_has_one_anchor_and_source() -> Result<()> {
    let value = observation()?;
    let digest = value.source_digest()?;
    let mut state = LiveSpatialState::default();
    state.publish(value.clone())?;
    let snapshot = state
        .snapshot()
        .ok_or_else(|| invalid("test snapshot absent"))?;
    assert!(snapshot.hash_is_valid());
    for kind in [
        EntityKind::Job,
        EntityKind::Building,
        EntityKind::Item,
        EntityKind::TileFeature,
    ] {
        assert!(snapshot.graph.entities.values().any(|e| e.kind == kind));
    }
    for fact in snapshot
        .graph
        .entities
        .values()
        .flat_map(|e| e.fields.values())
        .chain(
            snapshot
                .graph
                .edges
                .values()
                .flat_map(|e| e.fields.values()),
        )
    {
        assert_eq!(fact.source_digest, digest);
        assert_eq!(fact.observed_at, snapshot.tick);
        assert!(matches!(&fact.source,FactSource::DfhackField(s) if s.starts_with("spatial/1.6.")));
    }
    assert_ne!(digest, value.terrain.source_digest()?);
    assert_ne!(
        digest,
        value
            .operations
            .source_digest_profile(OperationsProfile::PagedV1_4)?
    );
    assert!(
        snapshot
            .graph
            .entities
            .values()
            .flat_map(|e| e.fields.values())
            .any(|f| matches!(&f.presence, Some(FactPresence::Redacted(_))))
    );
    Ok(())
}
#[test]
fn invalid_component_preserves_every_published_domain() -> Result<()> {
    let value = observation()?;
    let mut state = LiveSpatialState::default();
    state.publish(value.clone())?;
    let prior = state.snapshot().cloned();
    let mut bad = value;
    bad.terrain.map.cells.pop();
    assert!(state.publish(bad).is_err());
    assert_eq!(state.snapshot(), prior.as_ref());
    Ok(())
}
#[test]
fn map_only_changes_advance_the_shared_revision_without_false_retirement() -> Result<()> {
    let mut value = observation()?;
    let mut state = LiveSpatialState::default();
    state.publish(value.clone())?;
    let first = anchor(&state)?;
    assert_eq!(state.publish(value.clone())?, JobPublication::Heartbeat);
    if let Some(Cell::Visible(tile)) = value
        .terrain
        .map
        .cells
        .iter_mut()
        .find(|c| matches!(c, Cell::Visible(_)))
    {
        tile.traffic ^= 1;
    }
    assert_eq!(state.publish(value)?, JobPublication::Advanced);
    let snapshot = state
        .snapshot()
        .ok_or_else(|| invalid("test snapshot absent"))?;
    assert_eq!(snapshot.cursor.sequence, first.cursor.sequence + 1);
    assert_eq!(snapshot.graph.entities[&EntityId::new(9)].generation, 1);
    assert_eq!(snapshot.graph.entities[&EntityId::new(9)].revision, 2);
    assert_ne!(snapshot.state_hash, first.state_hash);
    Ok(())
}
#[test]
fn map_resets_and_item_retirement_share_a_generation_universe() -> Result<()> {
    let original = observation()?;
    let mut state = LiveSpatialState::default();
    state.publish(original.clone())?;
    let mut next = original.clone();
    next.terrain.map_dimensions[0] += 1;
    assert_eq!(state.publish(next.clone())?, JobPublication::Reset);
    let p = next.terrain.map.region.origin;
    let snapshot = state
        .snapshot()
        .ok_or_else(|| invalid("test snapshot absent"))?;
    assert_eq!(snapshot.graph.entities[&EntityId::new(9)].generation, 2);
    assert_eq!(snapshot.graph.entities[&tile_entity_id(p)?].generation, 2);
    let mut retired = next.clone();
    retired.operations.jobs.jobs.clear();
    retired.operations.attachments.clear();
    retired.operations.items.retain(|item| item.native_id != 30);
    state.publish(retired)?;
    state.publish(next)?;
    let snapshot = state
        .snapshot()
        .ok_or_else(|| invalid("test snapshot absent"))?;
    assert_eq!(snapshot.graph.entities[&item_entity_id(30)].generation, 3);
    assert_eq!(snapshot.graph.entities[&tile_entity_id(p)?].generation, 2);
    assert_eq!(snapshot.cursor.epoch, 1);
    Ok(())
}
