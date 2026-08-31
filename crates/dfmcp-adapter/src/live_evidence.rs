#![forbid(unsafe_code)]

//! Independently verifiable receipt linking one authenticated live capsule to
//! one canonical world snapshot.
//!
//! The receipt is small enough to persist or transfer independently. It binds
//! fortress lineage, cursor, game tick, capsule digest and size, bridge
//! generation, site identity, complete citizen coverage, projected entity
//! count, snapshot root, and a digest of all projected fact provenance.

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, FortressId, GameTick, ObservationCursor, Result};
use dfmcp_world::WorldSnapshot;

use crate::LiveObservationCapsule;

const RECEIPT_DOMAIN: &[u8] = b"dfmcp-live-observation-receipt-v1\0";

fn error(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

fn put_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_be_bytes());
}

fn put_i32(output: &mut Vec<u8>, value: i32) {
    output.extend_from_slice(&value.to_be_bytes());
}

fn put_u64(output: &mut Vec<u8>, value: u64) {
    output.extend_from_slice(&value.to_be_bytes());
}

fn put_digest(output: &mut Vec<u8>, value: Digest32) {
    output.extend_from_slice(value.as_bytes());
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveObservationReceipt {
    pub fortress_id: FortressId,
    pub cursor: ObservationCursor,
    pub game_tick: GameTick,
    pub capsule_digest: Digest32,
    pub capsule_bytes: u64,
    pub bridge_generation: u64,
    pub site_id: i32,
    pub citizen_count: u32,
    pub citizen_coverage_complete: bool,
    pub projected_entity_count: u64,
    pub fact_count: u64,
    pub fact_provenance_digest: Digest32,
    pub snapshot_root: Digest32,
    pub receipt_digest: Digest32,
}

impl LiveObservationReceipt {
    pub fn issue(
        capsule: &LiveObservationCapsule,
        snapshot: &WorldSnapshot,
    ) -> Result<Self> {
        capsule.validate()?;
        if !snapshot.hash_is_valid() {
            return Err(error(
                ErrorCode::ChecksumMismatch,
                "cannot issue a live receipt for a snapshot with an invalid root",
            ));
        }
        if snapshot.paused != capsule.paused {
            return Err(error(
                ErrorCode::InternalInvariantViolation,
                "live capsule and projected snapshot disagree about pause state",
            ));
        }
        let expected_entities = u64::from(capsule.citizen_coverage.total)
            .checked_add(1)
            .ok_or_else(|| {
                error(
                    ErrorCode::BudgetExceeded,
                    "live projected entity count overflows u64",
                )
            })?;
        let projected_entity_count = u64::try_from(snapshot.graph.entities.len()).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "projected entity count does not fit u64",
            )
        })?;
        if projected_entity_count != expected_entities {
            return Err(error(
                ErrorCode::InternalInvariantViolation,
                format!(
                    "live projection contains {projected_entity_count} entities; expected {expected_entities}"
                ),
            ));
        }

        let (fact_count, fact_provenance_digest) =
            fact_provenance(snapshot, capsule.content_digest)?;
        let capsule_bytes = u64::try_from(capsule.canonical_bytes.len()).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "live capsule byte count does not fit u64",
            )
        })?;
        let mut receipt = Self {
            fortress_id: snapshot.fortress_id,
            cursor: snapshot.cursor,
            game_tick: snapshot.tick,
            capsule_digest: capsule.content_digest,
            capsule_bytes,
            bridge_generation: capsule.bridge.bridge_generation,
            site_id: capsule.site_id,
            citizen_count: capsule.citizen_coverage.total,
            citizen_coverage_complete: capsule.citizen_coverage.proves_complete_roster(),
            projected_entity_count,
            fact_count,
            fact_provenance_digest,
            snapshot_root: snapshot.state_hash,
            receipt_digest: Digest32::ZERO,
        };
        receipt.receipt_digest = receipt.compute_digest();
        Ok(receipt)
    }

    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut output = Vec::new();
        output.extend_from_slice(RECEIPT_DOMAIN);
        put_u64(&mut output, self.fortress_id.get());
        put_u64(&mut output, self.cursor.epoch);
        put_u64(&mut output, self.cursor.sequence);
        put_u64(&mut output, self.game_tick.0);
        put_digest(&mut output, self.capsule_digest);
        put_u64(&mut output, self.capsule_bytes);
        put_u64(&mut output, self.bridge_generation);
        put_i32(&mut output, self.site_id);
        put_u32(&mut output, self.citizen_count);
        output.push(u8::from(self.citizen_coverage_complete));
        put_u64(&mut output, self.projected_entity_count);
        put_u64(&mut output, self.fact_count);
        put_digest(&mut output, self.fact_provenance_digest);
        put_digest(&mut output, self.snapshot_root);
        output
    }

    #[must_use]
    pub fn compute_digest(&self) -> Digest32 {
        Digest32::of_bytes(&self.canonical_bytes())
    }

    pub fn verify(
        &self,
        capsule: &LiveObservationCapsule,
        snapshot: &WorldSnapshot,
    ) -> Result<()> {
        if self.receipt_digest != self.compute_digest() {
            return Err(error(
                ErrorCode::ChecksumMismatch,
                "live observation receipt digest is invalid",
            ));
        }
        let expected = Self::issue(capsule, snapshot)?;
        if &expected != self {
            return Err(error(
                ErrorCode::ChecksumMismatch,
                "live observation receipt does not match the supplied capsule and snapshot",
            ));
        }
        Ok(())
    }
}

fn fact_provenance(
    snapshot: &WorldSnapshot,
    expected_source: Digest32,
) -> Result<(u64, Digest32)> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"dfmcp-live-fact-provenance-v1\0");
    let mut count = 0u64;
    for (entity_id, entity) in &snapshot.graph.entities {
        for (field, fact) in &entity.fields {
            if fact.source_digest != expected_source {
                return Err(error(
                    ErrorCode::ChecksumMismatch,
                    format!(
                        "projected fact {}.{} cites a different source digest",
                        entity_id, field
                    ),
                ));
            }
            count = count.checked_add(1).ok_or_else(|| {
                error(
                    ErrorCode::BudgetExceeded,
                    "projected fact count overflows u64",
                )
            })?;
            put_u64(&mut bytes, entity_id.get());
            let field_len = u32::try_from(field.len()).map_err(|_| {
                error(
                    ErrorCode::BudgetExceeded,
                    "projected field name length does not fit u32",
                )
            })?;
            put_u32(&mut bytes, field_len);
            bytes.extend_from_slice(field.as_bytes());
            put_digest(&mut bytes, fact.source_digest);
        }
    }
    Ok((count, Digest32::of_bytes(&bytes)))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use dfmcp_core::{FortressId, ObservationCursor};

    use super::*;
    use crate::{
        BridgeManifest, CitizenRecord, ObservationAssembler, ObservationPage,
        project_live_capsule,
    };

    fn fixture() -> Result<(LiveObservationCapsule, WorldSnapshot)> {
        let bridge = BridgeManifest {
            bridge_version: "0.1.0".to_owned(),
            dfhack_version: "0.51.11-r1".to_owned(),
            df_version: "0.51.11".to_owned(),
            world_loaded: true,
            fortress_mode: true,
            bridge_generation: 42,
            supported_methods: BTreeSet::from([
                "Handshake".to_owned(),
                "ReadObservation".to_owned(),
            ]),
        };
        let mut assembler = ObservationAssembler::new(bridge);
        assembler.push_page(ObservationPage {
            bridge_generation: 42,
            world_loaded: true,
            fortress_mode: true,
            paused: true,
            current_year: 105,
            current_year_tick: 12345,
            world_name: "The Balanced Realm".to_owned(),
            world_folder: "region1".to_owned(),
            site_id: 7,
            citizen_count_total: 1,
            citizen_offset: 0,
            complete: true,
            citizens: vec![CitizenRecord {
                unit_id: 11,
                name: "Urist Example".to_owned(),
                race: "dwarf".to_owned(),
                profession: 4,
                x: 10,
                y: 20,
                z: 30,
                alive: true,
                sane: true,
                active: true,
                visible: true,
                citizen: true,
                resident: false,
                baby: false,
                child: false,
                adult: true,
            }],
        })?;
        let capsule = assembler.finalize()?;
        let snapshot = project_live_capsule(
            &capsule,
            FortressId::new(77),
            ObservationCursor {
                epoch: 3,
                sequence: 9,
            },
        )?
        .snapshot;
        Ok((capsule, snapshot))
    }

    #[test]
    fn receipt_round_trip_binds_capsule_snapshot_and_provenance() -> Result<()> {
        let (capsule, snapshot) = fixture()?;
        let receipt = LiveObservationReceipt::issue(&capsule, &snapshot)?;
        receipt.verify(&capsule, &snapshot)?;
        assert_eq!(receipt.projected_entity_count, 2);
        assert!(receipt.fact_count > 0);
        assert!(receipt.citizen_coverage_complete);
        Ok(())
    }

    #[test]
    fn receipt_rejects_snapshot_tampering() -> Result<()> {
        let (capsule, mut snapshot) = fixture()?;
        let receipt = LiveObservationReceipt::issue(&capsule, &snapshot)?;
        snapshot.paused = false;
        snapshot.refresh_hash();
        assert!(receipt.verify(&capsule, &snapshot).is_err());
        Ok(())
    }

    #[test]
    fn receipt_rejects_fact_source_substitution() -> Result<()> {
        let (capsule, mut snapshot) = fixture()?;
        let entity = snapshot.graph.entities.values_mut().next().ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "fixture has no projected entity",
            )
        })?;
        let fact = entity.fields.values_mut().next().ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "fixture entity has no projected fact",
            )
        })?;
        fact.source_digest = Digest32::of_bytes(b"substituted-source");
        snapshot.refresh_hash();
        assert!(LiveObservationReceipt::issue(&capsule, &snapshot).is_err());
        Ok(())
    }
}
