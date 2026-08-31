#![forbid(unsafe_code)]

//! Stable semantic identity for a live fortress lineage.
//!
//! Transport endpoint, bridge process generation, pagination, observation
//! cursor, names projection, and software versions are deliberately excluded.
//! A bridge restart or upgrade must not rename the fortress. The identity is
//! derived from the save-world folder plus the current site ID after the source
//! capsule has passed its complete integrity checks.

use dfmcp_core::{Digest32, FortressId, Result};

use crate::LiveObservationCapsule;

const LIVE_FORTRESS_ID_DOMAIN: &[u8] = b"dfmcp-live-fortress-id-v1\0";

pub fn derive_live_fortress_id(capsule: &LiveObservationCapsule) -> Result<FortressId> {
    capsule.validate()?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(LIVE_FORTRESS_ID_DOMAIN);
    bytes.extend_from_slice(capsule.world_folder.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(&capsule.site_id.to_be_bytes());
    let digest = Digest32::of_bytes(&bytes);
    let source = digest.as_bytes();
    let raw = u64::from_be_bytes([
        source[0], source[1], source[2], source[3], source[4], source[5], source[6], source[7],
    ]) | 1;
    Ok(FortressId::new(raw))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::{
        BridgeManifest, CitizenRecord, ObservationAssembler, ObservationPage,
    };

    fn manifest() -> BridgeManifest {
        BridgeManifest {
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
        }
    }

    fn citizen(unit_id: i32, name: &str) -> CitizenRecord {
        CitizenRecord {
            unit_id,
            name: name.to_owned(),
            race: "dwarf".to_owned(),
            profession: 4,
            x: unit_id,
            y: 2,
            z: 3,
            alive: true,
            sane: true,
            active: true,
            visible: true,
            citizen: true,
            resident: false,
            baby: false,
            child: false,
            adult: true,
        }
    }

    fn capsule(
        world_folder: &str,
        site_id: i32,
        names_included: bool,
    ) -> Result<LiveObservationCapsule> {
        let page = ObservationPage {
            bridge_generation: 42,
            world_loaded: true,
            fortress_mode: true,
            paused: true,
            current_year: 105,
            current_year_tick: 12_345,
            world_name: "The Balanced Realm".to_owned(),
            world_folder: world_folder.to_owned(),
            site_id,
            citizen_count_total: 1,
            citizen_offset: 0,
            complete: true,
            citizens: vec![citizen(7, if names_included { "Urist" } else { "" })],
        };
        let mut assembler = ObservationAssembler::with_names(manifest(), names_included);
        assembler.push_page(page)?;
        assembler.finalize()
    }

    #[test]
    fn identity_ignores_projection_and_transport_details() -> Result<()> {
        let named = capsule("region1", 7, true)?;
        let unnamed = capsule("region1", 7, false)?;
        assert_ne!(named.content_digest, unnamed.content_digest);
        assert_eq!(
            derive_live_fortress_id(&named)?,
            derive_live_fortress_id(&unnamed)?
        );
        Ok(())
    }

    #[test]
    fn identity_changes_with_world_or_site() -> Result<()> {
        let baseline = derive_live_fortress_id(&capsule("region1", 7, true)?)?;
        let other_world = derive_live_fortress_id(&capsule("region2", 7, true)?)?;
        let other_site = derive_live_fortress_id(&capsule("region1", 8, true)?)?;
        assert_ne!(baseline, other_world);
        assert_ne!(baseline, other_site);
        assert_ne!(baseline, FortressId::NIL);
        Ok(())
    }
}
