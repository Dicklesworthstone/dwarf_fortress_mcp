use super::*;
use crate::build_placement::journal::BuildMode;
use crate::build_placement::{BuildBinding, BuildCapture};
use crate::control_effect_journal::EffectJournalStorage;
use crate::furniture_allocation::{Kind, Request, Slot};
use crate::furniture_batch::store::{BatchStore, MAX_STORE_BYTES};
use crate::furniture_batch::{BatchDefinition, MAX_DEFINITION_BYTES};
use crate::furniture_handoff::{Handoff, MAX_HANDOFF_BYTES};
use crate::furniture_supply;
use crate::live_jobs::LiveJobObservation;
use crate::live_operations::{
    LiveItem, LiveOperationsObservation, LiveOperationsState, OperationsProfile,
};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, MapCoord, OperationContext, RequestId, RiskTier,
    SessionId, WorkBudget,
};
use std::cell::RefCell;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::net::SocketAddr;
use std::rc::Rc;

fn missing() -> DfmcpError {
    invalid("room test fixture is incomplete")
}
fn request(count: usize) -> Result<FurnitureRequest> {
    FurnitureRequest::new(
        "region1".into(),
        2,
        Request {
            slots: (0..count)
                .map(|i| Slot {
                    name: format!("s{i:02}"),
                    kind: Kind::Bed,
                    target: [15 + i as u32, 15, 2],
                    after: Vec::new(),
                    material: None,
                    subtype: None,
                    max_distance: 65532,
                })
                .collect(),
            excluded_items: Vec::new(),
        },
    )
}
fn terrain(count: usize) -> Result<TerrainRequirements> {
    TerrainRequirements::new(
        Region {
            origin: [0, 0, 2],
            size: [64, 64, 1],
        },
        (0..count).map(|i| [15 + i as u32, 15, 2]).collect(),
        vec![[0, 0, 2]],
    )
}
fn endpoint() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 5000))
}
fn binding() -> Result<BuildBinding> {
    let text = include_str!("../../../../bridge/common/tests/fixtures/build_placement_v1_19.json");
    let source = text
        .split("\"capture\": \"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .ok_or_else(missing)?;
    let raw = source
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            u8::from_str_radix(std::str::from_utf8(pair).map_err(|_| missing())?, 16)
                .map_err(|_| missing())
        })
        .collect::<Result<Vec<_>>>()?;
    BuildBinding::new(endpoint(), "df", "dfhack", &BuildCapture::decode(&raw)?)
}
fn observed(count: usize) -> Result<LiveOperationsState> {
    let observed = LiveOperationsObservation {
        jobs: LiveJobObservation {
            bridge_generation: 901,
            df_version: "df".into(),
            dfhack_version: "dfhack".into(),
            year: 2,
            year_tick: 100,
            paused: true,
            site_id: 2,
            world_folder: "region1".into(),
            next_job_id: 90,
            jobs: Vec::new(),
        },
        next_building_id: 70,
        next_item_id: 42 + count as u32,
        buildings: Vec::new(),
        attachments: Vec::new(),
        items: (0..count)
            .map(|i| LiveItem {
                native_id: 42 + i as u32,
                item_type: 101,
                type_key: "BED".into(),
                subtype: -1,
                material_type: 419,
                material_index: -1,
                stack_size: 1,
                raw_position: MapCoord::new(10 + i as i32, 11, 2),
                flags: 64,
                container_native_id: None,
                holder_building_native_id: None,
            })
            .collect(),
    };
    let mut state = LiveOperationsState::with_profile(OperationsProfile::PagedV1_4);
    state.publish(observed)?;
    Ok(state)
}
fn context(state: &LiveOperationsState) -> Result<OperationContext> {
    Ok(OperationContext {
        session_id: SessionId::new(41),
        request_id: RequestId::new(1),
        anchor: state.snapshot().ok_or_else(missing)?.anchor(),
        budget: WorkBudget {
            max_wall_millis: 60_000,
            max_bytes: 64 * 1024 * 1024,
            max_entities: 80_000,
            ..WorkBudget::CONSERVATIVE_DEFAULT
        },
        grants: [Capability::Query, Capability::Plan]
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope::default(),
                max_risk: RiskTier::Guarded,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect(),
        cancellation_requested: false,
    })
}
fn allocate(request: &FurnitureRequest) -> Result<Handoff> {
    let source = observed(request.request().slots.len())?;
    Handoff::allocate(
        &source,
        &context(&source)?,
        endpoint(),
        request,
        furniture_supply::MAX_WORK,
    )?
    .handoff
    .ok_or_else(missing)
}
fn digest(domain: &[u8], raw: &[u8]) -> Digest32 {
    let mut value = domain.to_vec();
    value.push(0);
    value.extend_from_slice(raw);
    Digest32::of_bytes(&value)
}
fn independent_definition(
    magic: &[u8; 8],
    handoff: &Handoff,
    binding: &BuildBinding,
    journal: Digest32,
    room: Option<&RoomIntent>,
) -> Vec<u8> {
    let mut raw = magic.to_vec();
    field(&mut raw, handoff.plan().canonical_bytes());
    field(&mut raw, &binding.encode());
    raw.extend_from_slice(journal.as_bytes());
    if magic != b"DFMFBD01" {
        field(&mut raw, handoff.canonical_bytes());
    }
    if let Some(room) = room {
        field(&mut raw, room.terrain().canonical_bytes());
    }
    raw
}

#[test]
fn terrain_masks_have_one_independent_row_major_encoding() -> Result<()> {
    let region = Region {
        origin: [10, 20, 2],
        size: [3, 3, 1],
    };
    let first = TerrainRequirements::new(
        region,
        vec![[12, 22, 2], [10, 20, 2], [11, 21, 2]],
        vec![[11, 20, 2]],
    )?;
    let ordered = TerrainRequirements::new(
        region,
        vec![[10, 20, 2], [11, 21, 2], [12, 22, 2]],
        vec![[11, 20, 2]],
    )?;
    assert_eq!(first, ordered);
    assert_eq!(first.floors(), &[[10, 20, 2], [11, 21, 2], [12, 22, 2]]);
    assert_eq!(first.floor_mask(), &[0x11, 0x01]);
    assert_eq!(first.wall_mask(), &[0x02, 0]);
    let mut independent = b"DFMRTN01".to_vec();
    for value in [10u32, 20, 2, 3, 3, 1] {
        independent.extend_from_slice(&value.to_be_bytes());
    }
    independent.extend_from_slice(&[0, 2, 0x11, 1, 2, 0]);
    assert_eq!(first.canonical_bytes(), independent);
    assert_eq!(TerrainRequirements::decode(&independent)?, first);
    Ok(())
}

#[test]
fn terrain_requires_a_bounded_single_level_and_unique_disjoint_complete_selections() -> Result<()> {
    for region in [
        Region {
            origin: [0, 0, 0],
            size: [0, 1, 1],
        },
        Region {
            origin: [0, 0, 0],
            size: [1, 1, 2],
        },
        Region {
            origin: [0, 0, 0],
            size: [129, 1, 1],
        },
        Region {
            origin: [0, 0, 0],
            size: [65, 64, 1],
        },
        Region {
            origin: [32768, 0, 0],
            size: [1, 1, 1],
        },
        Region {
            origin: [u32::MAX, 0, 0],
            size: [2, 1, 1],
        },
    ] {
        assert!(TerrainRequirements::new(region, vec![region.origin], Vec::new()).is_err());
    }
    let region = Region {
        origin: [10, 10, 2],
        size: [2, 2, 1],
    };
    for (floors, walls) in [
        (vec![], vec![]),
        (vec![[10, 10, 2], [10, 10, 2]], vec![]),
        (vec![[10, 10, 2]], vec![[11, 10, 2], [11, 10, 2]]),
        (vec![[10, 10, 2]], vec![[10, 10, 2]]),
        (vec![[12, 10, 2]], vec![]),
        (vec![[10, 10, 3]], vec![]),
        (vec![[10, 10, 2]], vec![[9, 10, 2]]),
        (vec![[10, 10, 2]; 5], vec![]),
    ] {
        assert!(TerrainRequirements::new(region, floors, walls).is_err());
    }
    for (floors, walls) in [
        (vec![], vec![0]),
        (vec![1, 0], vec![0]),
        (vec![0], vec![0]),
        (vec![1], vec![1]),
        (vec![0x11], vec![0]),
        (vec![1], vec![0x10]),
    ] {
        assert!(TerrainRequirements::from_masks(region, floors, walls).is_err());
    }
    assert_eq!(
        TerrainRequirements::from_masks(region, vec![1], vec![2])?.floors(),
        &[[10, 10, 2]]
    );
    Ok(())
}

#[test]
fn terrain_codec_refuses_every_truncated_prefix_trailing_data_unknown_versions_and_bad_padding()
-> Result<()> {
    let original = TerrainRequirements::new(
        Region {
            origin: [0, 0, 2],
            size: [3, 3, 1],
        },
        vec![[0, 0, 2]],
        vec![[1, 0, 2]],
    )?;
    let raw = original.canonical_bytes();
    for length in 0..raw.len() {
        assert!(TerrainRequirements::decode(&raw[..length]).is_err());
    }
    for (index, byte) in [(7, b'2'), (32, 1), (33, 1), (35, 2), (37, 2)] {
        let mut changed = raw.to_vec();
        changed[index] = byte;
        assert!(
            TerrainRequirements::decode(&changed).is_err(),
            "accepted alteration at {index}"
        );
    }
    let mut trailing = raw.to_vec();
    trailing.push(0);
    assert!(TerrainRequirements::decode(&trailing).is_err());
    assert!(TerrainRequirements::decode(&vec![0; MAX_TERRAIN_BYTES + 1]).is_err());
    Ok(())
}

#[test]
fn room_identity_retains_all_32_targets_constraints_and_room_only_geometry() -> Result<()> {
    for count in [1, 32] {
        let request = request(count)?;
        let room = RoomIntent::new(request.clone(), terrain(count)?)?;
        assert_eq!(room.furniture(), &request);
        assert_eq!(room.terrain().floors().len(), count);
        assert_eq!(RoomIntent::decode(room.canonical_bytes())?, room);
        assert_eq!(
            room.digest(),
            digest(b"dfmcp-room-intent-rust/1", room.canonical_bytes())
        );
        let mut floors = room.terrain().floors().to_vec();
        floors.push([1, 0, 2]);
        let more_floor = RoomIntent::new(
            request.clone(),
            TerrainRequirements::new(
                room.terrain().region(),
                floors,
                room.terrain().walls().to_vec(),
            )?,
        )?;
        let more_wall = RoomIntent::new(
            request.clone(),
            TerrainRequirements::new(
                room.terrain().region(),
                room.terrain().floors().to_vec(),
                vec![[0, 0, 2], [1, 0, 2]],
            )?,
        )?;
        let region = RoomIntent::new(
            request.clone(),
            TerrainRequirements::new(
                Region {
                    origin: [0, 0, 2],
                    size: [64, 63, 1],
                },
                room.terrain().floors().to_vec(),
                room.terrain().walls().to_vec(),
            )?,
        )?;
        for changed in [&more_floor, &more_wall, &region] {
            assert_eq!(changed.furniture(), room.furniture());
            assert_ne!(changed.digest(), room.digest());
        }
        let mut model = request.request().clone();
        model.slots[0].max_distance -= 1;
        let changed = RoomIntent::new(
            FurnitureRequest::new(request.folder().into(), request.site(), model)?,
            room.terrain().clone(),
        )?;
        assert_ne!(changed.digest(), room.digest());
    }
    Ok(())
}

#[test]
fn room_requires_every_furnishing_floor_and_rejects_noncanonical_nested_requests() -> Result<()> {
    let request = request(32)?;
    let original = terrain(32)?;
    let mut missing = original.floors().to_vec();
    missing.pop();
    assert!(
        RoomIntent::new(
            request.clone(),
            TerrainRequirements::new(
                original.region(),
                missing.clone(),
                original.walls().to_vec(),
            )?
        )
        .is_err()
    );
    assert!(
        RoomIntent::new(
            request.clone(),
            TerrainRequirements::new(original.region(), missing, vec![[46, 15, 2]],)?
        )
        .is_err()
    );
    let room = RoomIntent::new(request.clone(), original)?;
    let raw = room.canonical_bytes();
    for length in 0..raw.len() {
        assert!(RoomIntent::decode(&raw[..length]).is_err());
    }
    let mut alias = b"DFMRIN01".to_vec();
    let mut noncanonical = vec![b' '];
    noncanonical.extend_from_slice(request.canonical_bytes());
    field(&mut alias, &noncanonical);
    field(&mut alias, room.terrain().canonical_bytes());
    assert!(RoomIntent::decode(&alias).is_err());
    for (index, byte) in [(7, b'2'), (8, 0xff)] {
        let mut changed = raw.to_vec();
        changed[index] = byte;
        assert!(RoomIntent::decode(&changed).is_err());
    }
    let mut trailing = raw.to_vec();
    trailing.push(0);
    assert!(RoomIntent::decode(&trailing).is_err());
    assert!(RoomIntent::decode(&vec![0; MAX_ROOM_INTENT_BYTES + 1]).is_err());
    Ok(())
}

#[test]
fn original_room_is_sealed_into_every_batch_key_without_changing_the_placed_selections()
-> Result<()> {
    for count in [1, 32] {
        let request = request(count)?;
        let handoff = allocate(&request)?;
        let binding = binding()?;
        let journal = Digest32::of_bytes(b"original placements");
        let room = RoomIntent::new(request, terrain(count)?)?;
        let batch = BatchDefinition::from_room_handoff(
            room.clone(),
            handoff.clone(),
            binding.clone(),
            journal,
        )?;
        let independent =
            independent_definition(b"DFMFBD03", &handoff, &binding, journal, Some(&room));
        assert_eq!(batch.canonical_bytes(), independent);
        assert_eq!(
            batch.id(),
            digest(b"dfmcp-furniture-batch-rust/3", &independent)
        );
        assert_eq!(batch.room(), Some(&room));
        assert_eq!(batch.handoff(), Some(&handoff));
        assert_eq!(BatchDefinition::decode(&independent)?, batch);
        let changed = RoomIntent::new(
            room.furniture().clone(),
            TerrainRequirements::new(
                room.terrain().region(),
                room.terrain().floors().to_vec(),
                vec![[1, 0, 2]],
            )?,
        )?;
        let other = BatchDefinition::from_room_handoff(changed, handoff, binding, journal)?;
        assert_eq!(other.plan(), batch.plan());
        for step in batch.plan().steps() {
            assert_ne!(batch.key(step), other.key(step));
        }
    }
    Ok(())
}

#[test]
fn existing_batch_generations_keep_exact_bytes_digests_and_keys() -> Result<()> {
    let handoff = allocate(&request(1)?)?;
    let binding = binding()?;
    let journal = Digest32::of_bytes(b"original placements");
    for (version, batch) in [
        (
            1,
            BatchDefinition::new(handoff.plan().clone(), binding.clone(), journal)?,
        ),
        (
            2,
            BatchDefinition::from_handoff(handoff.clone(), binding.clone(), journal)?,
        ),
    ] {
        let magic = if version == 1 {
            b"DFMFBD01"
        } else {
            b"DFMFBD02"
        };
        let raw = independent_definition(magic, &handoff, &binding, journal, None);
        let expected = digest(
            format!("dfmcp-furniture-batch-rust/{version}").as_bytes(),
            &raw,
        );
        assert_eq!(batch.canonical_bytes(), raw);
        assert_eq!(batch.id(), expected);
        assert!(batch.room().is_none());
        assert_eq!(BatchDefinition::decode(&raw)?, batch);
        for step in batch.plan().steps() {
            assert_eq!(batch.key(step), format!("fb-{expected}-{}", step.name));
        }
    }
    Ok(())
}

#[test]
fn room_batch_refuses_equal_targets_with_different_original_constraints_or_native_extent()
-> Result<()> {
    let original = request(1)?;
    let handoff = allocate(&original)?;
    let binding = binding()?;
    let journal = Digest32::of_bytes(b"original placements");
    let mut changed = original.request().clone();
    changed.slots[0].max_distance -= 1;
    let substituted = RoomIntent::new(
        FurnitureRequest::new(original.folder().into(), original.site(), changed)?,
        terrain(1)?,
    )?;
    assert!(
        BatchDefinition::from_room_handoff(
            substituted.clone(),
            handoff.clone(),
            binding.clone(),
            journal
        )
        .is_err()
    );
    let altered =
        independent_definition(b"DFMFBD03", &handoff, &binding, journal, Some(&substituted));
    // V3 stores the complete request only once, in the handoff. There is no
    // second request field through which a contradictory room can be smuggled.
    let replayed = BatchDefinition::decode(&altered)?;
    assert_eq!(replayed.room().ok_or_else(missing)?.furniture(), &original);
    assert_ne!(replayed.room(), Some(&substituted));

    let width = binding.dimensions()[0];
    assert!(
        width < 128,
        "fixture must leave a structurally valid out-of-native-map region"
    );
    let outside = RoomIntent::new(
        original,
        TerrainRequirements::new(
            Region {
                origin: [0, 0, 2],
                size: [width + 1, 16, 1],
            },
            vec![[15, 15, 2]],
            Vec::new(),
        )?,
    )?;
    assert!(BatchDefinition::from_room_handoff(outside, handoff, binding, journal).is_err());
    Ok(())
}

#[test]
fn room_batch_codec_rejects_truncation_missing_room_trailing_bytes_and_generation_substitution()
-> Result<()> {
    let request = request(1)?;
    let handoff = allocate(&request)?;
    let room = RoomIntent::new(request, terrain(1)?)?;
    let batch = BatchDefinition::from_room_handoff(
        room,
        handoff,
        binding()?,
        Digest32::of_bytes(b"journal"),
    )?;
    let raw = batch.canonical_bytes();
    for length in 0..raw.len() {
        assert!(BatchDefinition::decode(&raw[..length]).is_err());
    }
    for version in *b"0124" {
        let mut changed = raw.clone();
        changed[7] = version;
        assert!(BatchDefinition::decode(&changed).is_err());
    }
    let mut trailing = raw;
    trailing.push(0);
    assert!(BatchDefinition::decode(&trailing).is_err());
    assert!(BatchDefinition::decode(&vec![0; MAX_DEFINITION_BYTES + 1]).is_err());
    Ok(())
}

#[derive(Clone, Default)]
struct Memory(Rc<RefCell<Cursor<Vec<u8>>>>);
impl Read for Memory {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.0.borrow_mut().read(bytes)
    }
}
impl Write for Memory {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Seek for Memory {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.0.borrow_mut().seek(position)
    }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn truncate(&mut self, length: u64) -> io::Result<()> {
        self.0.borrow_mut().get_mut().truncate(length as usize);
        Ok(())
    }
}

#[test]
fn maximum_request_and_4096_cell_room_survive_existing_parent_custody_stop_and_reopen() -> Result<()>
{
    let mut model = request(32)?.request().clone();
    for (i, slot) in model.slots.iter_mut().enumerate() {
        slot.name = format!("{i:02}{}", "x".repeat(46));
        slot.after = (0..i.min(3))
            .map(|j| format!("{j:02}{}", "x".repeat(46)))
            .collect();
    }
    let mut largest = FurnitureRequest::new("region1".into(), 2, model.clone())?;
    for item in 10000..14096 {
        model.excluded_items.push(item);
        match FurnitureRequest::new("region1".into(), 2, model.clone()) {
            Ok(next) => largest = next,
            Err(_) => break,
        }
    }
    assert!(largest.canonical_bytes().len() > MAX_REQUEST_BYTES - 10);
    let handoff = allocate(&largest)?;
    assert_eq!(handoff.items().len(), 32);
    assert!(handoff.canonical_bytes().len() <= MAX_HANDOFF_BYTES);
    let region = Region {
        origin: [0, 0, 2],
        size: [64, 64, 1],
    };
    let floors = (1..MAX_ROOM_CELLS)
        .map(|i| region.position(i).ok_or_else(missing))
        .collect::<Result<Vec<_>>>()?;
    let terrain = TerrainRequirements::new(region, floors, vec![[0, 0, 2]])?;
    assert_eq!(terrain.canonical_bytes().len(), MAX_TERRAIN_BYTES);
    let room = RoomIntent::new(largest, terrain)?;
    assert!(room.canonical_bytes().len() > MAX_ROOM_INTENT_BYTES - 10);
    let definition = BatchDefinition::from_room_handoff(
        room.clone(),
        handoff,
        binding()?,
        Digest32::of_bytes(b"journal"),
    )?;
    assert!(definition.canonical_bytes().len() <= MAX_DEFINITION_BYTES);
    const _: () = assert!(MAX_DEFINITION_BYTES + 256 < MAX_STORE_BYTES);
    const _: () = assert!(
        MAX_DEFINITION_BYTES + 32 * crate::build_placement::MAX_RECORD_BYTES + 20_000
            < crate::construction_plan::origin::MAX_ORIGIN_BYTES
    );
    let c = context(&observed(32)?)?;
    let memory = Memory::default();
    let mut owner = BatchStore::open(
        memory.clone(),
        &c,
        BuildMode::Control,
        Some(definition.clone()),
        true,
    )?;
    owner.verify(&c)?;
    assert_eq!(owner.definition().room(), Some(&room));
    owner.stop(&c)?;
    assert!(owner.durable_stopped());
    drop(owner);
    let mut reopened = BatchStore::open(
        memory.clone(),
        &c,
        BuildMode::Offline,
        Some(definition.clone()),
        false,
    )?;
    reopened.verify(&c)?;
    assert_eq!(reopened.definition(), &definition);
    assert_eq!(reopened.definition().room(), Some(&room));
    assert!(reopened.durable_stopped());
    assert!(reopened.stop(&c).is_err());
    assert!(memory.0.borrow().get_ref().len() < MAX_STORE_BYTES);
    Ok(())
}

#[test]
fn an_existing_parent_cannot_rebind_its_room_only_intent_on_reopen() -> Result<()> {
    let request = request(1)?;
    let handoff = allocate(&request)?;
    let room = RoomIntent::new(request.clone(), terrain(1)?)?;
    let journal = Digest32::of_bytes(b"journal");
    let original =
        BatchDefinition::from_room_handoff(room.clone(), handoff.clone(), binding()?, journal)?;
    let altered_room = RoomIntent::new(
        request,
        TerrainRequirements::new(
            room.terrain().region(),
            room.terrain().floors().to_vec(),
            vec![[1, 0, 2]],
        )?,
    )?;
    let altered = BatchDefinition::from_room_handoff(altered_room, handoff, binding()?, journal)?;
    let c = context(&observed(1)?)?;
    let memory = Memory::default();
    let owner = BatchStore::open(
        memory.clone(),
        &c,
        BuildMode::Control,
        Some(original.clone()),
        true,
    )?;
    drop(owner);
    let before = memory.0.borrow().get_ref().clone();
    assert!(
        BatchStore::open(memory.clone(), &c, BuildMode::Recover, Some(altered), false).is_err()
    );
    assert_eq!(*memory.0.borrow().get_ref(), before);
    let mut reopened = BatchStore::open(memory, &c, BuildMode::Recover, Some(original), false)?;
    reopened.verify(&c)?;
    assert_eq!(reopened.definition().room(), Some(&room));
    Ok(())
}
